//! Type, effect and structure checker (counterpart of check.py). Fills in `ty` for every expression.
//! Function structure, check order, code, stage, message and fix match Python character for character.
//!
//! G3 A1: types are `GType` inside the checker; the AST carries them as key strings (gtype.rs `to_key`/`from_key`),
//! which are the plain legacy names for every non-generic program. Type parameters and written type arguments are
//! real AST fields (ast.rs); the checker records the instantiation (`inst`) and `enum_ty` on the nodes (Python check.py).

use crate::ast::*;
use crate::diagnostic::PruvError;
use crate::foreign::{
    catalog_fn, check_foreign_decl, check_ownership, is_catalog, is_catalog_contract_only, is_file_observer, is_net_observer, is_async_observer, is_async_op, is_proc_observer, CatFn,
    Takes,
};
use crate::gtype::{
    bind_args, check_arity, collection_arity, decl_owner, dependencies, fn_owner, format_type, is_element, is_value,
    key_is_collection, key_is_vec_slice_arg, mk_array, mk_nominal, mk_option, mk_result, substitute, GType, Mapping, ParamId,
};
use crate::typeinfo::{program_uses_inout, TypeTable, BUILTIN_TYPE_NAMES};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

type Res<T> = Result<T, PruvError>;

fn i64t() -> GType {
    GType::i64()
}
fn boolt() -> GType {
    GType::bool()
}
fn unitt() -> GType {
    GType::unit()
}
fn strt() -> GType {
    GType::str()
}

/// What a contract-only built-in returns.
pub(crate) enum CRes {
    Bool,
    I64,
    /// The Option/Result payload type.
    Payload,
}

/// P2: pure (i64, i64) -> i64 bit builtins (check.py BIT_BUILTINS); later batches add names here.
pub(crate) const BIT_BUILTINS: [&str; 7] = ["band", "bor", "bxor", "wrapping_shl", "ushr", "wrapping_add", "wrapping_mul"];

pub(crate) fn is_bit_builtin(n: &str) -> bool {
    BIT_BUILTINS.contains(&n)
}

/// P2 batch 4: byte-word access on [u8] (check.py WORD_BUILTINS).
pub(crate) const WORD_BUILTINS: [&str; 2] = ["load_be", "store_be"];

pub(crate) fn is_word_builtin(n: &str) -> bool {
    WORD_BUILTINS.contains(&n)
}

/// P2 batch 4: the value of an integer literal width argument n of load_be / store_be, 1..=8 (None otherwise).
pub(crate) fn word_width(e: &Expr) -> Option<i64> {
    match e {
        Expr::IntLit(l) => l.value.parse::<i64>().ok().filter(|v| (1..=8).contains(v)),
        _ => None,
    }
}

/// Built-ins used only in contracts: name -> (argument kind, result).
pub(crate) fn contract_only(n: &str) -> Option<(&'static str, CRes)> {
    match n {
        "is_some" => Some(("option", CRes::Bool)),
        "some_val" => Some(("option", CRes::Payload)),
        "is_ok" => Some(("result", CRes::Bool)),
        "ok_val" => Some(("result", CRes::Payload)),
        "err_val" => Some(("result", CRes::I64)),
        "size" => Some(("size", CRes::I64)), // G9 A0: structural size of a non-generic nominal value
        _ => None,
    }
}

/// G9 A0: strongly connected components of the graph on nodes 0..n-1 (adj[i]: successors of i), iterative Tarjan.
/// -> member lists (each ascending), ordered by their smallest member.
fn sccs(n: usize, adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut index_of: Vec<i64> = vec![-1; n];
    let mut low: Vec<i64> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut counter: i64 = 0;
    for root in 0..n {
        if index_of[root] != -1 {
            continue;
        }
        index_of[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&(v, i)) = work.last() {
            if i < adj[v].len() {
                let last = work.len() - 1;
                work[last] = (v, i + 1);
                let w = adj[v][i];
                if index_of[w] == -1 {
                    index_of[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index_of[w]);
                }
            } else {
                work.pop();
                if let Some(&(p, _)) = work.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index_of[v] {
                    let mut comp: Vec<usize> = Vec::new();
                    loop {
                        let w = stack.pop().unwrap_or(v);
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comp.sort();
                    out.push(comp);
                }
            }
        }
    }
    out.sort_by_key(|c| c[0]);
    out
}

/// G9 A0: shortest cycle start -> ... -> start inside `members` (breadth first, successors in adjacency order);
/// -> nodes beginning and ending with start.
fn cycle_through(start: usize, adj: &[Vec<usize>], members: &[usize]) -> Vec<usize> {
    let members: HashSet<usize> = members.iter().cloned().collect();
    let mut prev: HashMap<usize, Option<usize>> = HashMap::new();
    prev.insert(start, None);
    let mut queue: Vec<usize> = vec![start];
    let mut qi = 0usize;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for &w in &adj[v] {
            if !members.contains(&w) {
                continue;
            }
            if w == start {
                let mut chain: Vec<usize> = Vec::new();
                let mut x: Option<usize> = Some(v);
                while let Some(xv) = x {
                    chain.push(xv);
                    x = prev.get(&xv).cloned().unwrap_or(None);
                }
                chain.reverse(); // start ... v
                chain.push(start);
                return chain;
            }
            if !prev.contains_key(&w) {
                prev.insert(w, Some(v));
                queue.push(w);
            }
        }
    }
    vec![start, start]
}

/// G9 A0: [(variant name or None, field)] of a declaration in variant/field source order.
fn decl_fields(d: &TypeDecl) -> Vec<(Option<&str>, &FieldDecl)> {
    match d {
        TypeDecl::Struct(s) => s.fields.iter().map(|f| (None, f)).collect(),
        TypeDecl::Enum(e) => e
            .variants
            .iter()
            .flat_map(|v| v.fields.iter().map(move |f| (Some(v.name.as_str()), f)))
            .collect(),
    }
}

fn field_label(d: &str, vname: Option<&str>, field: &str) -> String {
    match vname {
        None => format!("{}.{}", d, field),
        Some(v) => format!("{}::{}.{}", d, v, field),
    }
}

/// The checked type key of an expression (None before the checker has seen it).
fn expr_ty(e: &Expr) -> &Ty {
    match e {
        Expr::InoutArg(x) => &x.ty,
        Expr::Move(x) => &x.ty,
        Expr::Spent(x) => &x.ty,
        Expr::IntLit(x) => &x.ty,
        Expr::BoolLit(x) => &x.ty,
        Expr::StrLit(x) => &x.ty,
        Expr::Var(x) => &x.ty,
        Expr::Unary(x) => &x.ty,
        Expr::Binary(x) => &x.ty,
        Expr::Call(x) => &x.ty,
        Expr::Index(x) => &x.ty,
        Expr::Ctor(x) => &x.ty,
        Expr::Quant(x) => &x.ty,
        Expr::StructLit(x) => &x.ty,
        Expr::EnumCtor(x) => &x.ty,
        Expr::FieldAccess(x) => &x.ty,
        Expr::IsVariant(x) => &x.ty,
        Expr::VariantField(x) => &x.ty,
    }
}

pub(crate) const RESERVED_TYPE_NAMES: [&str; 6] = ["Option", "Result", "Some", "None", "Ok", "Err"];

fn is_tail(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// ^[A-Z][A-Za-z0-9_]*$ (ASCII)
pub(crate) fn type_name_ok(s: &str) -> bool {
    match s.chars().next() {
        Some(c) if c.is_ascii_uppercase() => is_tail(&s[1..]),
        _ => false,
    }
}

/// ^[a-z][A-Za-z0-9_]*$ (ASCII)
fn field_name_ok(s: &str) -> bool {
    match s.chars().next() {
        Some(c) if c.is_ascii_lowercase() => is_tail(&s[1..]),
        _ => false,
    }
}

/// Converts only the first character to ASCII case (names are ASCII identifiers, never empty).
pub(crate) fn cap_first(s: &str, upper: bool) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => {
            let c = if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
            format!("{}{}", c, it.as_str())
        }
        None => String::new(),
    }
}

fn terr(line: i64, col: i64, code: &str, msg: impl Into<String>, fix: impl Into<String>) -> PruvError {
    PruvError::new("type", line, col, code, msg, fix)
}

/// The type name as source spelling (Python `type_name`).
fn tn(t: &GType) -> String {
    format_type(t)
}

/// Whether the integer literal (sign plus decimal string) is in the i64 range: -2^63 <= v < 2^63.
fn int_in_range(v: &str) -> bool {
    let (neg, rest) = if let Some(r) = v.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = v.strip_prefix('+') {
        (false, r)
    } else {
        (false, v)
    };
    let d = rest.trim_start_matches('0');
    let limit = if neg { "9223372036854775808" } else { "9223372036854775807" };
    d.len() < limit.len() || (d.len() == limit.len() && d <= limit)
}

#[derive(Clone)]
struct Entry {
    ty: GType,
    mutable: bool,
    kind: &'static str, // "param" | "inout" (G5) | "result" | "local" | "bound"
}
type Scope = HashMap<String, Entry>;

#[derive(Clone)]
struct FuncInfo {
    src_name: String,
    line: i64,
    col: i64,
    params: Vec<GType>,
    /// G5 A0: parameter names (position of `old(a)`, messages)
    pnames: Vec<String>,
    /// G5 A0: parameter is `inout`
    inout: Vec<bool>,
    ret: GType,
    effects: Vec<String>,
    tparams: Vec<TParam>,
    /// G6: a `trusted rust fn` (cannot be called in a contract)
    foreign: bool,
    /// P13o: an `opaque fn` (a contract call stays an uninterpreted application until `reveal`)
    opaque: bool,
    pred: bool,
}

struct Checker {
    funcs: HashMap<String, Rc<FuncInfo>>,
    decls: HashMap<String, TypeDecl>, // first declaration
    tt: TypeTable,
    cur_name: String,
    cur_ret: GType,
    used_effects: BTreeSet<String>,
    /// Expected array type, turned on only when the right-hand expression of a let/reassignment/return is itself an alloc call (alloc takes it right away).
    alloc_want: Option<GType>,
    // G3: type-parameter context of the declaration being checked
    ctx_names: HashSet<String>,
    ctx_generic: bool,
    elem_params: Vec<ParamId>,
    /// type name -> number of type parameters (for gtype::check_arity / is_value)
    arg_decls: HashMap<String, usize>,
    /// G6: crate names given with --rust-extern
    externs: Vec<String>,
    /// G6: function name -> `take` flags of its parameters (for the ownership checker)
    takes: Takes,
    /// Location of the field move allowed in the current field assignment.
    field_move: Option<(i64, i64)>,
    collection_records: std::cell::RefCell<HashSet<String>>,
    nested_records: HashSet<String>,
    holder_records: HashSet<String>,
    read_context: Option<usize>,
    statement_call: Option<usize>,
    borrowed_text: Option<usize>,
    map_roots: Vec<(String, Vec<String>, Vec<String>)>,
    map_iterations: Vec<(String, Vec<String>)>,
    // ---- G9 A0 metadata ----
    /// recursive type SCCs (names in declaration order), SCC-number order
    type_sccs: Vec<Vec<String>>,
    /// "Decl.field" / "Enum::Variant.field" of every intra-SCC (boxed) field
    boxed: Vec<String>,
    /// (declaration name, variant name or None) holding at least one boxed field
    boxed_decls: HashSet<(String, Option<String>)>,
    /// names of extended declarations
    extended: HashSet<String>,
    /// declarations transitively containing a buffer (array/str) field
    buffered: HashSet<String>,
    /// recursive function SCCs (names in function order)
    fn_sccs: Vec<Vec<String>>,
    /// (caller name, call line, call col, call targs, callee name) of every checked user call
    call_log: Vec<(String, i64, i64, Option<Vec<GType>>, String)>,
    /// Roots of enclosing extended matches, with a flag for map_get scrutinees.
    active_roots: Vec<(String, bool)>,
    /// Places held by map_get Some arms, checked for overlap when taking an entry.
    map_read_paths: Vec<(String, Vec<String>)>,
    /// R-6e-8: (root, field path) of enclosing matches on extended places of inout roots
    match_paths: Vec<(String, Vec<String>)>,
    cur_cost: bool,
    s5_errors: Vec<(String, PruvError)>,
    par_groups: Vec<Vec<Expr>>,
    /// G4 A0: explicit visibility mode (the declaration names are resolved identities)
    explicit: bool,
    /// G4 A0: per function (same index as Program.functions): a host entry
    host_entry: Vec<bool>,
    /// G5 A0: enables the contextual builtin `old`
    uses_inout: bool,
    /// G5: kind of the expression being checked: requires/ensures/decreases/invariant/assert/code/par
    ckind: &'static str,
    // ---- P06: contract expansion of user functions (check.py x_*) ----
    /// function name -> why a contract cannot expand it, or a raw copy of its body
    x_raw: HashMap<String, Result<Vec<Stmt>, String>>,
    /// function name -> its checked template (an expression over its parameters)
    x_memo: HashMap<String, Expr>,
    /// templates being built (recursion)
    x_busy: HashSet<String>,
    /// set by `call` on a contract call: the expansion that replaces the Call node
    x_pending: Option<Expr>,
    contract_vec_slice: Option<(i64, i64)>,
    /// G5: identity of the call allowed at the statement boundary of the statement being checked
    boundary: Option<usize>,
    /// G5 A0: parameter names of the function being checked
    cur_params: Vec<String>,
}

/// G4 A0: `check_with_externs` for an explicit-mode program: resolves the names of the whole program to identities first
/// (modules::resolve), then runs the same checker plus the host-entry signature rules.
pub fn check_explicit(
    prog: &mut Program,
    g4: &mut crate::modules::G4,
    externs: &[String],
) -> Result<TypeTable, PruvError> {
    crate::modules::resolve(prog, g4)?;
    let src_names: Vec<String> = g4.fn_meta.iter().map(|m| m.src_name.clone()).collect();
    let mut c = Checker::new(prog, externs, &src_names)?;
    prog.uses_inout = Some(c.uses_inout); // G5 A0: lets the staging guard skip its walk for inout-free programs
    c.explicit = true;
    c.host_entry = g4.fn_meta.iter().map(|m| m.host_entry).collect();
    c.run(prog)?;
    prog.contract_vec_slice = c.contract_vec_slice;
    g4.remove_preds(&prog.functions);
    prog.functions.retain(|f| !f.pred);
    Ok(c.tt)
}

/// Checks the program, fills in ty for every expression, and returns the type table.
pub fn check(prog: &mut Program) -> Result<TypeTable, PruvError> {
    check_with_externs(prog, &[])
}

/// `check` with the crate names given with --rust-extern (G6: crates a trusted rust fn path may start with besides std).
pub fn check_with_externs(prog: &mut Program, externs: &[String]) -> Result<TypeTable, PruvError> {
    let mut c = Checker::new(prog, externs, &[])?;
    prog.uses_inout = Some(c.uses_inout); // G5 A0: lets the staging guard skip its walk for inout-free programs
    c.run(prog)?;
    prog.contract_vec_slice = c.contract_vec_slice;
    prog.functions.retain(|f| !f.pred);
    Ok(c.tt)
}

fn ends_in_return(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        None => false,
        Some(Stmt::Return(_)) => true,
        Some(Stmt::If(i)) => ends_in_return(&i.then) && ends_in_return(&i.els),
        Some(Stmt::Match(m)) => m.arms.iter().all(|a| ends_in_return(&a.body)),
        Some(_) => false,
    }
}

fn set_ty(e: &mut Expr, t: &GType) {
    let v = Some(t.to_key());
    match e {
        Expr::Spent(x) => x.ty = v,
        Expr::IntLit(x) => x.ty = v,
        Expr::BoolLit(x) => x.ty = v,
        Expr::StrLit(x) => x.ty = v,
        Expr::Var(x) => x.ty = v,
        Expr::Unary(x) => x.ty = v,
        Expr::Binary(x) => x.ty = v,
        Expr::Call(x) => x.ty = v,
        Expr::Index(x) => x.ty = v,
        Expr::Ctor(x) => x.ty = v,
        Expr::Quant(x) => x.ty = v,
        Expr::StructLit(x) => x.ty = v,
        Expr::EnumCtor(x) => x.ty = v,
        Expr::FieldAccess(x) => x.ty = v,
        Expr::IsVariant(x) => x.ty = v,
        Expr::VariantField(x) => x.ty = v,
        Expr::Move(x) => x.ty = v,
        Expr::InoutArg(x) => x.ty = v,
    }
}

/// G5: does the full syntax of e (every operand, reachable or not) mention the binding `name`?
fn collection_catalog(name: &str) -> bool {
    matches!(name, "vec_new" | "vec_push" | "vec_get" | "map_new" | "map_insert"
        | "map_get" | "map_get_mut" | "map_remove" | "map_take" | "vec_at_mut" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice")
}

pub(crate) fn place_root(e: &Expr) -> Option<&str> {
    match e {
        Expr::Var(v) => Some(&v.name),
        Expr::FieldAccess(f) => place_root(&f.base),
        _ => None,
    }
}

pub(crate) fn field_place(mut e: &Expr) -> (&Expr, Vec<String>) {
    let mut path = Vec::new();
    while let Expr::FieldAccess(f) = e {
        path.push(f.field.clone());
        e = &f.base;
    }
    path.reverse();
    (e, path)
}

pub(crate) fn mentions(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Var(x) if x.name == name => return true,
        Expr::Move(x) if x.name == name => return true,
        _ => {}
    }
    crate::refine::kids(e).into_iter().any(|k| mentions(k, name))
}

fn paths_overlap(a: &[String], b: &[String]) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Collect each mentioned place once, using the full field path for a field read.
fn place_mentions<'a>(e: &'a Expr, out: &mut Vec<(&'a str, Vec<String>)>) {
    if let Expr::Move(m) = e {
        out.push((&m.name, m.fields.clone()));
        return;
    }
    let (root, path) = field_place(e);
    if let Expr::Var(v) = root {
        out.push((&v.name, path));
        return;
    }
    for child in crate::refine::kids(e) {
        place_mentions(child, out);
    }
}

fn first_field_move(e: &Expr) -> Option<&Move> {
    if let Expr::Move(m) = e {
        if !m.fields.is_empty() {
            return Some(m);
        }
    }
    crate::refine::kids(e).into_iter().find_map(first_field_move)
}

fn field_move_error(m: &Move) -> PruvError {
    terr(
        m.line,
        m.col,
        "foreign_move",
        "a field moved out must be reassigned by the next statement",
        "restore the field immediately, or pass the move to a take call assigned back to the same field",
    )
}

/// G5: the identity of a call node, for "the call allowed at the statement boundary" (Python `e is s.boundary`).
fn boundary_of(e: &Expr) -> Option<usize> {
    match e {
        Expr::Call(c) => Some(c as *const Call as usize),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum InoutContext<'a> {
    Value,
    Assign(&'a str),
    WhileCondition,
    PlaceAssignment(&'a str, &'a [String]),
}

/// Visit calls in evaluation order (arguments first).
fn inout_statement_calls<'a>(
    e: &'a Expr,
    conditional: bool,
    calls: &mut Vec<(&'a Call, bool)>,
) {
    if let Expr::Binary(b) = e {
        if b.op == "&&" || b.op == "||" {
            inout_statement_calls(&b.left, conditional, calls);
            inout_statement_calls(&b.right, true, calls);
            return;
        }
    }
    for child in crate::refine::kids(e) {
        inout_statement_calls(child, conditional, calls);
    }
    if let Expr::Call(c) = e {
        calls.push((c, conditional));
    }
}

/// The instantiation recorded on a node: declaration identity, type arguments, and the substitution ordered by index.
fn inst_of(decl: String, targs: &[GType], m: &Mapping) -> Inst {
    let mut subst: Vec<(String, usize, GType)> = m.iter().map(|((o, i), t)| (o.clone(), *i, t.clone())).collect();
    subst.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    Inst { decl, args: targs.to_vec(), subst, src_args: None }
}

/// Type of an element read from / stored into the array type `at`: T for [T]; bool for [bool]; else i64.
fn elem_type(at: &GType) -> GType {
    if at.name == "array" && !at.args.is_empty() {
        return at.args[0].clone();
    }
    if *at == GType::array_bool() {
        boolt()
    } else {
        i64t()
    }
}

impl Checker {
    /// src_names: explicit mode only, the name of each function as written (Python `Function.src_name`); empty otherwise.
    fn new(prog: &Program, externs: &[String], src_names: &[String]) -> Res<Checker> {
        let mut funcs: HashMap<String, Rc<FuncInfo>> = HashMap::new();
        let mut takes: Takes = HashMap::new();
        let uses_inout = program_uses_inout(prog); // G5 A0: enables the contextual builtin `old`
        for (fi, f) in prog.functions.iter().enumerate() {
            let name = f.name.clone();
            if funcs.contains_key(&name) {
                return Err(terr(f.line, f.col, "dup_fn", format!("function {} defined twice", name), ""));
            }
            if matches!(name.as_str(), "len" | "alloc" | "copy" | "print" | "is_char_boundary" | "substr" | "truncate" | "eq_range" | "resize" | "copy_range" | "copy_within")
                || is_bit_builtin(&name)
                || is_word_builtin(&name)
                || contract_only(&name).is_some()
                || (is_catalog(&name) && !is_async_op(&name))
            {
                return Err(terr(f.line, f.col, "reserved", format!("{} is a builtin name", name), "rename the function"));
            }
            let written = src_names.get(fi).map(|s| s.as_str()).unwrap_or(name.as_str());
            if uses_inout && written == "old" {
                return Err(terr(
                    f.line,
                    f.col,
                    "reserved",
                    "old is a builtin name in a program that uses inout",
                    "rename the function",
                ));
            }
            takes.insert(name.clone(), f.params.iter().map(|p| p.take).collect());
            funcs.insert(
                name.clone(),
                Rc::new(FuncInfo {
                    src_name: written.to_string(),
                    line: f.line,
                    col: f.col,
                    params: f.params.iter().map(|p| GType::from_key(&p.ty)).collect(),
                    pnames: f.params.iter().map(|p| p.name.clone()).collect(),
                    inout: f.params.iter().map(|p| p.mode_pos.is_some()).collect(),
                    ret: GType::from_key(&f.ret),
                    effects: f.effects.clone(),
                    tparams: f.tparams.clone(),
                    foreign: f.foreign.is_some(),
                    opaque: f.opaque,
                    pred: f.pred,
                }),
            );
        }
        Ok(Checker {
            funcs,
            decls: HashMap::new(),
            tt: TypeTable::default(),
            cur_name: String::new(),
            cur_ret: unitt(),
            used_effects: BTreeSet::new(),
            alloc_want: None,
            ctx_names: HashSet::new(),
            ctx_generic: false,
            elem_params: Vec::new(),
            arg_decls: HashMap::new(),
            externs: externs.to_vec(),
            takes,
            field_move: None,
            collection_records: Default::default(),
            nested_records: HashSet::new(),
            holder_records: HashSet::new(),
            read_context: None,
            statement_call: None,
            borrowed_text: None,
            map_roots: Vec::new(),
            map_iterations: Vec::new(),
            type_sccs: Vec::new(),
            boxed: Vec::new(),
            boxed_decls: HashSet::new(),
            extended: HashSet::new(),
            buffered: HashSet::new(),
            fn_sccs: Vec::new(),
            call_log: Vec::new(),
            active_roots: Vec::new(),
            map_read_paths: Vec::new(),
            match_paths: Vec::new(),
            cur_cost: false,
            s5_errors: Vec::new(),
            par_groups: Vec::new(),
            explicit: false,
            host_entry: Vec::new(),
            uses_inout,
            ckind: "code",
            boundary: None,
            cur_params: Vec::new(),
            x_raw: HashMap::new(),
            x_memo: HashMap::new(),
            x_busy: HashSet::new(),
            x_pending: None,
            contract_vec_slice: None,
        })
    }

    /// If the value is itself an alloc call and the expected type is an array, sets alloc's result type to that array type.
    fn want_alloc(&mut self, value: &Expr, ty: &GType) {
        if let Expr::Call(c) = value {
            if c.name == "alloc" && ty.is_array() {
                self.alloc_want = Some(ty.clone());
            }
        }
    }

    fn run(&mut self, prog: &mut Program) -> Res<()> {
        crate::refine::expand(prog)?; // STAGE1 A: validate refinement annotations, then expand them into contracts/asserts
        self.declare_types(&prog.types)?;
        self.normalize_types(prog); // G6: Vec/HashMap/String become catalog types unless the program declares them
        // `decls` holds clones taken before normalization (Python shares the nodes): refresh them
        for d in &prog.types {
            self.decls.insert(d.name().to_string(), d.clone());
        }
        // likewise the signatures in `funcs` were taken before normalization (Python reads the shared nodes)
        for f in &prog.functions {
            if let Some(info) = self.funcs.get_mut(&f.name) {
                let mut ni = (**info).clone();
                ni.params = f.params.iter().map(|p| GType::from_key(&p.ty)).collect();
                ni.ret = GType::from_key(&f.ret);
                *info = Rc::new(ni);
            }
        }
        self.arg_decls = self.decls.iter().map(|(n, d)| (n.clone(), d.tparams().len())).collect();
        self.check_tparams(prog)?; // G3: type-parameter lists (type_parameter, generic_bound, generic_main)
        self.compute_holder_records();
        self.resolve_types(prog)?;
        self.type_graph(&prog.types)?; // G9 A0: type SCCs, generic/recursive field rules
        self.inhabitance(&prog.types)?; // G9 A0: every declaration needs a finite value
        self.compute_extended(&prog.types);
        self.check_extended_uses(prog)?;
        self.check_modes(prog)?; // G5 A0: parameter modes, before host entries and bodies
        if self.explicit {
            self.check_entries(prog)?; // G4 A0: host-entry signatures
        }
        self.tt = TypeTable::new(&prog.types);
        self.x_prepare(prog)?; // P06: which functions a contract may expand, and their raw bodies
        for f in prog.functions.iter_mut() {
            self.fn_(f)?;
        }
        self.check_fn_sccs(&prog.functions)?; // G9 A0: measures and stable type parameters of recursive call SCCs
        self.check_par_eligibility(&prog.functions)?; // G7 A0: effects, foreign calls, then nested groups
        self.check_costs(prog)?;
        self.record_g9(prog);
        self.x_auto(prog)?; // P07: code calls of an expandable function know its body (result == expansion)
        Ok(())
    }

    /// P07 (docs/L3_RESTRUCTURE_SPEC.md appendix P07, check.py `x_auto`): every function whose body expands as a
    /// contract keeps its template as `x_auto`, so the verifier can assume result == expansion at each code call. A
    /// function whose template cannot be built gets None (never an error); so does an i64 function whose template uses a
    /// bit builtin (its value would put a bit-vector term into every later query of the caller).
    fn x_auto(&mut self, prog: &mut Program) -> Res<()> {
        for f in prog.functions.iter_mut() {
            f.x_auto = None;
            if !matches!(self.x_raw.get(&f.name), Some(Ok(_))) {
                continue;
            }
            let fi = match self.funcs.get(&f.name) {
                Some(fi) => fi.clone(),
                None => continue,
            };
            self.cur_name = f.name.clone();
            self.enter(&fn_owner(&f.name), &f.tparams);
            self.ckind = "ensures";
            match self.x_template(f.line, f.col, &f.name, &fi) {
                Ok(xa) => {
                    if !f.pred && (fi.ret == boolt() || !x_has_bit(&xa))
                        && (f.opaque || !(x_has_quant(&xa) || fi.params.iter().any(|p| x_record(p, &self.decls))))
                    {
                        f.x_auto = Some(xa);
                    }
                }
                Err(err) => {
                    self.x_busy.clear();
                    if f.opaque {
                        // P13o: an opaque body is its definition, so it must build
                        return Err(err);
                    }
                }
            }
            self.ckind = "code";
        }
        Ok(())
    }

    fn check_costs(&mut self, prog: &mut Program) -> Res<()> {
        for f in &mut prog.functions {
            if let Some(cost) = &mut f.cost {
                self.cur_name = f.name.clone();
                self.cur_ret = GType::from_key(&f.ret);
                self.cur_cost = true;
                self.cur_params = f.params.iter().map(|p| p.name.clone()).collect();
                self.enter(&fn_owner(&f.name), &f.tparams);
                self.ckind = "cost";
                let scope: Scope = f.params.iter().map(|p| (p.name.clone(), Entry {
                    ty: GType::from_key(&p.ty), mutable: false,
                    kind: if p.mode_pos.is_some() { "inout" } else { "param" },
                })).collect();
                let before = self.s5_errors.len();
                let ty = self.expr(cost, &scope, true)?;
                if ty != i64t() && self.s5_errors.len() == before {
                    self.s5_errors.push((f.name.clone(), terr(cost.line(), cost.col(), "cost_type", "cost expression must have type i64", "")));
                }
            }
        }
        if let Some((_, e)) = self.s5_errors.iter().min_by_key(|(name, e)| (
            prog.functions.iter().position(|f| &f.name == name).unwrap(), e.line, e.col)) {
            return Err(e.clone());
        }
        for f in &prog.functions {
            if f.cost.is_none() { continue; }
            let mut calls = Vec::new();
            cost_calls(&f.body, &mut calls);
            calls.sort_by_key(|c| (c.line, c.col));
            for c in calls {
                let callee = prog.functions.iter().find(|f| f.name == c.name);
                if is_catalog(&c.name) || callee.map_or(false, |f| f.foreign.is_some()) {
                    return Err(PruvError::new("effect", c.line, c.col, "cost_unsupported", "cost functions cannot call foreign, catalog or file operations", ""));
                }
                if callee.map_or(false, |f| f.cost.is_none()) {
                    let name = self.funcs.get(&c.name).map(|f| f.src_name.as_str()).filter(|n| !n.is_empty()).unwrap_or(&c.name);
                    return Err(PruvError::new("effect", c.line, c.col, "cost_callee", format!("`{}` has no cost clause, so its work is unbounded here", name), ""));
                }
            }
        }
        Ok(())
    }

    // ---- user type declarations ----
    fn declare_types(&mut self, types: &[TypeDecl]) -> Res<()> {
        for d in types {
            let (name, line, col) = (d.name(), d.line(), d.col());
            // G4: explicit mode checked the names as written (modules::resolve); the identities are not type-name shaped
            if !self.explicit && !type_name_ok(name) {
                return Err(terr(
                    line,
                    col,
                    "type_name",
                    format!("type name {} must start with an upper-case letter", name),
                    format!("rename it, e.g. {}", cap_first(name, true)),
                ));
            }
            if !self.explicit && RESERVED_TYPE_NAMES.contains(&name) {
                return Err(terr(line, col, "reserved", format!("{} is a builtin name", name), "rename the type"));
            }
            if self.decls.contains_key(name) {
                return Err(terr(line, col, "dup_type", format!("type {} defined twice", name), ""));
            }
            if self.funcs.contains_key(name) {
                return Err(terr(
                    line,
                    col,
                    "name_collision",
                    format!("{} is both a type and a function", name),
                    "rename the type or the function",
                ));
            }
            self.decls.insert(name.to_string(), d.clone());
            match d {
                TypeDecl::Struct(s) => self.check_field_names(&s.fields, &format!("struct {}", name))?,
                TypeDecl::Enum(e) => {
                    if e.variants.is_empty() {
                        return Err(terr(line, col, "empty_enum", format!("enum {} has no variants", name), "add at least one variant"));
                    }
                    // Duplicate checking uses a set like a Python set (audit: a Vec is O(n^2)).
                    let mut seen: HashSet<&str> = HashSet::new();
                    for v in &e.variants {
                        if !type_name_ok(&v.name) {
                            return Err(terr(
                                v.line,
                                v.col,
                                "variant_name",
                                format!("variant name {} must start with an upper-case letter", v.name),
                                format!("rename it, e.g. {}", cap_first(&v.name, true)),
                            ));
                        }
                        if seen.contains(v.name.as_str()) {
                            return Err(terr(
                                v.line,
                                v.col,
                                "dup_variant",
                                format!("variant {} defined twice in enum {}", v.name, name),
                                "",
                            ));
                        }
                        seen.insert(&v.name);
                        self.check_field_names(&v.fields, &format!("variant {}::{}", name, v.name))?;
                    }
                }
            }
        }
        Ok(())
    }

    // ---- G6 A0: collection spellings ----
    /// The parser yields nominal Vec/HashMap/String; map them to the catalog types unless a user type of that name
    /// is declared (an existing user type keeps its meaning).
    fn norm_type(&self, ty: &GType) -> GType {
        if ty.owner.is_some() {
            return ty.clone();
        }
        let args: Vec<GType> = ty.args.iter().map(|a| self.norm_type(a)).collect();
        let mut name = ty.name.clone();
        if !self.decls.contains_key(&name) {
            name = match name.as_str() {
                "Vec" => "vec".to_string(),
                "HashMap" => "hashmap".to_string(),
                "String" => "string".to_string(),
                "File" => "file".to_string(), // G10 A0
                "Process" => "process".to_string(), // G10 proc A0
                "ProcessArgs" => "process_args".to_string(),
                "Socket" => "socket".to_string(), // G10 net A0
                "Listener" => "listener".to_string(),
                "Loop" => "loop".to_string(),
                _ => name.clone(),
            };
        }
        if matches!(name.as_str(), "vec" | "hashmap") {
            if let Some(payload) = args.last().filter(|t| t.owner.is_none() && t.args.is_empty()) {
                self.collection_records.borrow_mut().insert(payload.name.clone());
            }
        }
        if name == ty.name && args == ty.args {
            return ty.clone();
        }
        if args.len() == 1 {
            if name == "array" {
                return mk_array(args[0].clone());
            }
            if name == "option" {
                return mk_option(args[0].clone());
            }
            if name == "result" {
                return mk_result(args[0].clone());
            }
        }
        GType::new(&name, args)
    }

    /// `norm_type` on an AST type key; a key without any of the three spellings is returned unchanged.
    fn norm_key(&self, key: &str) -> String {
        if !(key.contains("Vec") || key.contains("HashMap") || key.contains("String") || key.contains("File") || key.contains("Process")
            || key.contains("Socket") || key.contains("Listener") || key.contains("Loop"))
        {
            return key.to_string();
        }
        self.norm_type(&GType::from_key(key)).to_key()
    }

    fn normalize_types(&self, prog: &mut Program) {
        for d in prog.types.iter_mut() {
            match d {
                TypeDecl::Struct(s) => {
                    for fd in s.fields.iter_mut() {
                        fd.ty = self.norm_key(&fd.ty);
                    }
                }
                TypeDecl::Enum(e) => {
                    for v in e.variants.iter_mut() {
                        for fd in v.fields.iter_mut() {
                            fd.ty = self.norm_key(&fd.ty);
                        }
                    }
                }
            }
        }
        for f in prog.functions.iter_mut() {
            for p in f.params.iter_mut() {
                p.ty = self.norm_key(&p.ty);
            }
            f.ret = self.norm_key(&f.ret);
            for e in f.requires.iter_mut() {
                self.norm_expr(e);
            }
            for e in f.ensures.iter_mut() {
                self.norm_expr(e);
            }
            if let Some(d) = f.cost.as_mut() { self.norm_expr(d); }
            if let Some(d) = f.decreases.as_mut() {
                self.norm_expr(d); // G9 A0
            }
            self.norm_stmts(&mut f.body);
        }
    }

    fn norm_targs(&self, targs: &mut Option<Vec<GType>>) {
        if let Some(v) = targs {
            *v = v.iter().map(|t| self.norm_type(t)).collect();
        }
    }

    fn norm_stmts(&self, stmts: &mut [Stmt]) {
        for st in stmts.iter_mut() {
            match st {
                Stmt::Let(s) => {
                    s.ty = self.norm_key(&s.ty);
                    self.norm_expr(&mut s.value);
                }
                Stmt::Assign(s) => self.norm_expr(&mut s.value),
                Stmt::FieldAssign(s) => self.norm_expr(&mut s.value),
                Stmt::Return(s) => {
                    if let Some(v) = s.value.as_mut() {
                        self.norm_expr(v);
                    }
                }
                Stmt::IndexAssign(s) => {
                    self.norm_expr(&mut s.index);
                    self.norm_expr(&mut s.value);
                }
                Stmt::If(s) => {
                    self.norm_expr(&mut s.cond);
                    self.norm_stmts(&mut s.then);
                    self.norm_stmts(&mut s.els);
                }
                Stmt::While(s) => {
                    self.norm_expr(&mut s.cond);
                    for i in s.invariants.iter_mut() {
                        self.norm_expr(i);
                    }
                    self.norm_expr(&mut s.decreases);
                    self.norm_stmts(&mut s.body);
                }
                Stmt::ForMap(s) => {
                    self.norm_expr(&mut s.place);
                    for inv in &mut s.invariants {
                        self.norm_expr(inv);
                    }
                    self.norm_stmts(&mut s.body);
                }
                Stmt::Assert(s) => self.norm_expr(&mut s.cond),
                Stmt::Par(s) => {
                    for c in &mut s.calls {
                        self.norm_expr(c);
                    }
                }
                Stmt::ExprStmt(s) => self.norm_expr(&mut s.expr),
                Stmt::Match(s) => {
                    self.norm_expr(&mut s.scrutinee);
                    for a in s.arms.iter_mut() {
                        self.norm_stmts(&mut a.body);
                    }
                }
            }
        }
    }

    /// Explicit type arguments (`f::<..>`, `S::<..> {..}`, `E::<..>::V(..)`, `Some::<..>(..)`) of an expression tree.
    fn norm_expr(&self, e: &mut Expr) {
        match e {
            Expr::Call(x) => {
                self.norm_targs(&mut x.targs);
                if (x.name.starts_with("vec_") || x.name.starts_with("map_"))
                    && is_catalog(&x.name) && !self.funcs.contains_key(&x.name)
                {
                    if let Some(payload) = x.targs.as_ref().and_then(|args| args.last())
                        .filter(|t| t.owner.is_none() && t.args.is_empty())
                    {
                        self.collection_records.borrow_mut().insert(payload.name.clone());
                    }
                }
                for a in x.args.iter_mut() {
                    self.norm_expr(a);
                }
            }
            Expr::Ctor(x) => {
                self.norm_targs(&mut x.targs);
                if let Some(a) = x.arg.as_mut() {
                    self.norm_expr(a);
                }
            }
            Expr::StructLit(x) => {
                self.norm_targs(&mut x.targs);
                for f in x.fields.iter_mut() {
                    self.norm_expr(&mut f.value);
                }
            }
            Expr::EnumCtor(x) => {
                self.norm_targs(&mut x.targs);
                for a in x.args.iter_mut() {
                    self.norm_expr(a);
                }
            }
            Expr::Unary(x) => self.norm_expr(&mut x.operand),
            Expr::Binary(x) => {
                self.norm_expr(&mut x.left);
                self.norm_expr(&mut x.right);
            }
            Expr::Index(x) => {
                self.norm_expr(&mut x.array);
                self.norm_expr(&mut x.index);
            }
            Expr::Quant(x) => {
                self.norm_expr(&mut x.lo);
                self.norm_expr(&mut x.hi);
                self.norm_expr(&mut x.body);
            }
            Expr::FieldAccess(x) => self.norm_expr(&mut x.base),
            Expr::IsVariant(x) => self.norm_expr(&mut x.value),
            Expr::VariantField(x) => self.norm_expr(&mut x.value),
            Expr::InoutArg(x) => self.norm_expr(&mut x.expr), // G5 A0
            Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) | Expr::Move(_) => {}
        }
    }

    fn check_field_names(&self, fields: &[FieldDecl], wher: &str) -> Res<()> {
        let mut seen: HashSet<&str> = HashSet::new();
        for f in fields {
            if !field_name_ok(&f.name) {
                return Err(terr(
                    f.line,
                    f.col,
                    "field_name",
                    format!("field name {} must start with a lower-case letter", f.name),
                    format!("rename it, e.g. {}", cap_first(&f.name, false)),
                ));
            }
            if seen.contains(f.name.as_str()) {
                return Err(terr(f.line, f.col, "dup_field", format!("field {} defined twice in {}", f.name, wher), ""));
            }
            seen.insert(&f.name);
        }
        Ok(())
    }

    // ---- G3: type parameters and type applications ----
    /// Makes the type-parameter list of the declaration being checked the current context.
    fn enter(&mut self, owner: &str, tparams: &[TParam]) {
        self.ctx_names = tparams.iter().map(|t| t.name.clone()).collect();
        self.ctx_generic = !tparams.is_empty();
        self.elem_params = tparams
            .iter()
            .enumerate()
            .filter(|(_, t)| t.bound.as_deref() == Some("Element"))
            .map(|(i, _)| (owner.to_string(), i))
            .collect();
    }

    fn check_tparams(&self, prog: &Program) -> Res<()> {
        for d in &prog.types {
            self.tparam_list(&d.tparams())?;
        }
        for f in &prog.functions {
            let tps = &f.tparams;
            if f.name == "main" && !tps.is_empty() {
                return Err(terr(
                    f.line,
                    f.col,
                    "generic_main",
                    "main cannot declare type parameters",
                    "make main a plain function; call generic functions from it with explicit type arguments",
                ));
            }
            self.tparam_list(tps)?;
        }
        Ok(())
    }

    fn tparam_list(&self, tparams: &[TParam]) -> Res<()> {
        let mut seen: HashSet<&str> = HashSet::new();
        for tp in tparams {
            if !type_name_ok(&tp.name) {
                return Err(terr(
                    tp.line,
                    tp.col,
                    "type_parameter",
                    format!("type parameter {} must start with an upper-case letter", tp.name),
                    format!("rename it, e.g. {}", cap_first(&tp.name, true)),
                ));
            }
            if RESERVED_TYPE_NAMES.contains(&tp.name.as_str()) || self.decls.contains_key(&tp.name) {
                return Err(terr(
                    tp.line,
                    tp.col,
                    "type_parameter",
                    format!("type parameter {} shadows a declared or built-in type", tp.name),
                    "choose a name that is not a type, e.g. T",
                ));
            }
            if seen.contains(tp.name.as_str()) {
                return Err(terr(
                    tp.line,
                    tp.col,
                    "type_parameter",
                    format!("type parameter {} is declared twice", tp.name),
                    "give every parameter of a declaration its own name",
                ));
            }
            seen.insert(&tp.name);
            if let Some(b) = &tp.bound {
                if b != "Element" {
                    return Err(terr(
                        tp.line,
                        tp.col,
                        "generic_bound",
                        format!("unknown bound {} on type parameter {}", b, tp.name),
                        format!("the only bound is Element: <{}: Element>", tp.name),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Every type reference: arity, known names, type-argument and bound rules (G3_SPEC 3.1, 3.2, 3.4).
    fn check_type(&self, line: i64, col: i64, ty: &GType) -> Res<()> {
        let ad = &self.arg_decls;
        let look = |n: &str| ad.get(n).map(|k| (*k, Vec::<GType>::new()));
        if let Err(ex) = check_arity(ty, &look) {
            return Err(terr(
                line,
                col,
                "type_arity",
                ex.to_string(),
                "give the exact number of type arguments, e.g. Option<i64>, Pair<i64, bool>",
            ));
        }
        self.check_type_rec(line, col, ty)
    }

    fn check_type_rec(&self, line: i64, col: i64, ty: &GType) -> Res<()> {
        if ty.owner.is_some() {
            return Ok(());
        }
        if ty.name == "option" || ty.name == "result" {
            for a in &ty.args {
                self.check_arg(line, col, a, None)?;
            }
            return Ok(());
        }
        if collection_arity(&ty.name).is_some() {
            // G6: Vec<T>, HashMap<K, V>, String
            for a in &ty.args {
                self.check_type_rec(line, col, a)?;
            }
            if ty.name == "vec" && ty.args.len() == 1 {
                if ty.args[0].to_key() != "string" {
                    self.collection_payload(line, col, &ty.args[0], "a Vec element")?;
                }
            } else if ty.name == "hashmap" && ty.args.len() == 2 {
                let k = &ty.args[0];
                let text_key = k.to_key() == "string";
                if !text_key && !is_element(k, &self.elem_params) {
                    return Err(terr(
                        line,
                        col,
                        "foreign_owned_type",
                        format!("{} cannot be a HashMap key: only i64, bool, String and Element type parameters can", tn(k)),
                        "use i64, bool or String keys, or declare the parameter `: Element`",
                    ));
                }
                let v = &ty.args[1];
                self.collection_payload(line, col, v, "a HashMap value")?;
                if text_key && *v != i64t() && *v != boolt() && !self.owned_aggregate(v) {
                    return Err(terr(line, col, "foreign_owned_type", "String-keyed maps require i64, bool or owned aggregate values", ""));
                }
                if !text_key && *k != i64t() && v.owner.is_none() && v.args.is_empty() && self.holder_records.contains(&v.name) {
                    return Err(terr(line, col, "foreign_owned_type", "aggregate HashMap values with owned fields require i64 or String keys", "")); // D90
                }
                if !text_key && *k != i64t() && v.owner.is_none() {
                    if let Some(TypeDecl::Struct(d)) = self.decls.get(&v.name) {
                        if d.tparams.is_empty()
                            && d.fields.iter().any(|f| crate::gtype::key_is_collection(&f.ty) || f.ty == "array_u8")
                            && d.fields.iter().all(|f| {
                                let ft = GType::from_key(&f.ty);
                                crate::gtype::key_is_owned_field(&f.ty)
                                    || f.ty == "array_u8"
                                    || self.record_map_field(&ft)
                                    || is_value(&ft, &|n| self.value_decl(n))
                            })
                        {
                            return Err(terr(line, col, "foreign_owned_type", "aggregate HashMap values with owned fields require i64 or String keys", ""));
                        }
                    }
                }
            }
            return Ok(());
        }
        if ty.name == "array" {
            if !ty.args.is_empty() {
                // [X] for X other than i64/u8/bool
                let el = &ty.args[0];
                self.check_type_rec(line, col, el)?;
                if el.contains_collection() {
                    return Err(terr(
                        line,
                        col,
                        "foreign_owned_type",
                        format!("an array cannot hold the collection type {}", tn(el)),
                        "arrays hold i64, bool or an Element type parameter",
                    ));
                }
                if let Some(o) = &el.owner {
                    if !self.elem_params.contains(&(o.clone(), el.index)) {
                        return Err(terr(
                            line,
                            col,
                            "generic_bound",
                            format!("[{}] requires {}: Element", el.name, el.name),
                            format!("declare the parameter as <{}: Element>", el.name),
                        ));
                    }
                } else {
                    return Err(terr(
                        line,
                        col,
                        "type_argument",
                        format!("{} cannot be an array element", tn(el)),
                        "arrays hold i64, bool or an Element type parameter",
                    ));
                }
            }
            return Ok(());
        }
        if BUILTIN_TYPE_NAMES.contains(&ty.name.as_str()) {
            return Ok(());
        }
        if self.ctx_names.contains(&ty.name) {
            return Err(terr(
                line,
                col,
                "type_parameter",
                format!("type parameter {} is not a type constructor", ty.name),
                format!("write {} without type arguments", ty.name),
            ));
        }
        let d = match self.decls.get(&ty.name) {
            Some(d) => d,
            None => {
                if self.ctx_generic {
                    return Err(terr(
                        line,
                        col,
                        "type_parameter",
                        format!("unknown type {}: it is neither declared nor a type parameter of this declaration", ty.name),
                        format!("declare it in the type-parameter list, e.g. <{}>, or declare the type", ty.name),
                    ));
                }
                return Err(terr(
                    line,
                    col,
                    "unknown_type",
                    format!("unknown type {}", ty.name),
                    format!("declare it with `struct {} {{ ... }}` or `enum {} {{ ... }}`", ty.name, ty.name),
                ));
            }
        };
        let tps = d.tparams();
        for (a, tp) in ty.args.iter().zip(tps.iter()) {
            self.check_arg(line, col, a, Some(tp))?;
        }
        Ok(())
    }

    /// G6: a collection payload is a plain G3 value (never another collection).
    fn value_decl(&self, name: &str) -> Option<(usize, Vec<GType>)> {
        self.decls.get(name).map(|decl| (
            decl.tparams().len(),
            decl_fields(decl).into_iter().map(|(_, f)| GType::from_key(&f.ty)).collect(),
        ))
    }

    fn collection_payload(&self, line: i64, col: i64, a: &GType, what: &str) -> Res<()> {
        if a.contains_collection() {
            return Err(terr(
                line,
                col,
                "foreign_owned_type",
                format!("{} cannot be the collection type {}", what, tn(a)),
                "collection payloads are plain values; keep every collection in its own binding",
            ));
        }
        let ad = &self.arg_decls;
        let look = |n: &str| ad.get(n).map(|k| (*k, Vec::<GType>::new()));
        if !is_value(a, &look) {
            return Err(terr(
                line,
                col,
                "foreign_owned_type",
                format!(
                    "{} cannot have type {}: only i64, bool, Option, Result, structs, enums and type parameters can",
                    what,
                    tn(a)
                ),
                "arrays, str and () cannot be collection payloads",
            ));
        }
        Ok(())
    }

    /// a: a type argument; tp: the TypeParam it instantiates (None for Option/Result).
    fn check_arg(&self, line: i64, col: i64, a: &GType, tp: Option<&TParam>) -> Res<()> {
        self.check_type_rec(line, col, a)?;
        if a.contains_collection() || self.depends_on_nested_record(a) {
            // G6: collections are never type arguments of Option/Result/user declarations
            return Err(terr(
                line,
                col,
                "foreign_owned_type",
                format!("{} cannot be a type argument", tn(a)),
                "keep collections in their own bindings; Vec<T> and HashMap<K, V> are the only places they appear",
            ));
        }
        let ad = &self.arg_decls;
        let look = |n: &str| ad.get(n).map(|k| (*k, Vec::<GType>::new()));
        if !is_value(a, &look) {
            return Err(terr(
                line,
                col,
                "type_argument",
                format!(
                    "{} is not a valid type argument: only i64, bool, Option, Result, structs, enums and type parameters are",
                    tn(a)
                ),
                "arrays, str and () cannot be type arguments",
            ));
        }
        if let Some(tp) = tp {
            if tp.bound.as_deref() == Some("Element") && !is_element(a, &self.elem_params) {
                return Err(terr(
                    line,
                    col,
                    "generic_bound",
                    format!("{} does not satisfy the bound Element of {}", tn(a), tp.name),
                    "only i64, bool and type parameters declared `: Element` satisfy Element",
                ));
            }
        }
        Ok(())
    }

    /// Steps 1-4 of G3_SPEC 3.4 for a call / literal / constructor written with `targs` (None = no type application)
    /// -> substitution, or None for a non-generic declaration.
    #[allow(clippy::too_many_arguments)]
    fn instantiate(
        &self,
        line: i64,
        col: i64,
        what: &str,
        owner: &str,
        tparams: &[TParam],
        targs: &Option<Vec<GType>>,
    ) -> Res<Option<Mapping>> {
        let targs = match targs {
            None => {
                if !tparams.is_empty() {
                    let last = what.split_whitespace().last().unwrap_or("");
                    return Err(terr(
                        line,
                        col,
                        "type_arity",
                        format!("{} expects {} type argument(s)", what, tparams.len()),
                        format!("write the type arguments explicitly, e.g. {}::<T>; they are never inferred", last),
                    ));
                }
                return Ok(None);
            }
            Some(t) => t,
        };
        if tparams.is_empty() {
            return Err(terr(line, col, "type_arity", format!("{} takes no type arguments", what), "remove the `::<...>`"));
        }
        if targs.len() != tparams.len() {
            return Err(terr(
                line,
                col,
                "type_arity",
                format!("{} expects {} type argument(s), got {}", what, tparams.len(), targs.len()),
                "give exactly one type argument per type parameter",
            ));
        }
        let catalog_collection = what.strip_prefix("function ").filter(|n|
            !self.funcs.contains_key(*n) && (collection_catalog(n)
                || *n == "map_lookup" && targs[0].to_key() == "string"));
        if let Some(name) = catalog_collection {
            if name == "vec_extend_from_slice" && targs[0] != i64t() && targs[0] != boolt() {
                return Err(terr(line, col, "foreign_owned_type", "vec_extend_from_slice requires i64 or bool elements", ""));
            }
            let ty = if name.starts_with("vec_") { "vec" } else { "hashmap" };
            self.check_type(line, col, &GType::new(ty, targs.clone()))?;
        }
        for (a, tp) in targs.iter().zip(tparams.iter()) {
            self.check_type(line, col, a)?;
            if catalog_collection.is_none() {
                self.check_arg(line, col, a, Some(tp))?;
            }
            let catalog_collection = catalog_collection.is_some();
            self.ext_misuse(line, col, a, Some(if catalog_collection { "coll" } else { "arg" }))?;
        }
        match bind_args(owner, tparams.len(), targs) {
            Ok(m) => Ok(Some(m)),
            Err(e) => Err(terr(line, col, "internal", e.to_string(), "")),
        }
    }

    fn resolve_types(&mut self, prog: &Program) -> Res<()> {
        for d in &prog.types {
            self.enter(&decl_owner(d.name()), &d.tparams());
            let groups: Vec<&Vec<FieldDecl>> = match d {
                TypeDecl::Struct(s) => vec![&s.fields],
                TypeDecl::Enum(e) => e.variants.iter().map(|v| &v.fields).collect(),
            };
            for fields in groups {
                for f in fields {
                    let ft = GType::from_key(&f.ty);
                    if !d.tparams().is_empty() && (ft.is_array() || ft == strt()) {
                        // G9 A0
                        return Err(terr(
                            f.line,
                            f.col,
                            "field_type",
                            format!("generic type {} cannot have the array/str field {}", d.name(), f.name),
                            "use a dedicated non-generic declaration",
                        ));
                    }
                    if ft == unitt()
                        || (ft.is_array() && ft != GType::array() && ft != GType::array_u8() && ft != GType::array_bool())
                    {
                        return Err(terr(
                            f.line,
                            f.col,
                            "field_type",
                            format!("field {} cannot have type {}", f.name, tn(&ft)),
                            "fields are i64, bool, Option<i64>, Result<i64> or a user struct/enum",
                        ));
                    }
                    self.check_type(f.line, f.col, &ft)?;
                    let holder_field = self.holder_records.contains(d.name())
                        && !self.holder_records.contains(&ft.name) && self.nested_child_type(&ft);
                    if !self.state_collection(&ft) && !holder_field && self.depends_on_nested_record(&ft) {
                        return Err(terr(f.line, f.col, "foreign_owned_type", "a record with nested collection fields cannot be a direct field", ""));
                    }
                    let nested_field = matches!(d, TypeDecl::Struct(_))
                        && crate::gtype::key_is_nested_collection_field(&f.ty)
                        && (!matches!(f.ty.as_str(), "hashmap<i64,i64>" | "hashmap<string,i64>")
                            || self.collection_records.borrow().contains(d.name()));
                    if ft.contains_collection()
                        && !(d.tparams().is_empty() && ((crate::gtype::key_is_owned_field(&f.ty)
                            && !crate::gtype::key_is_nested_collection_field(&f.ty) || nested_field)
                            || matches!(d, TypeDecl::Struct(_)) && self.state_collection(&ft)))
                    {
                        if matches!(ft.name.as_str(), "vec" | "hashmap") {
                            if let Some(payload) = ft.args.last().filter(|ty| is_value(ty, &|n| self.value_decl(n))) {
                                // The type graph has not run yet; is_value rules out recursive payloads.
                                self.ext_misuse(f.line, f.col, payload, Some("coll"))?;
                            }
                        }
                        return Err(terr(
                            f.line,
                            f.col,
                            "foreign_owned_type",
                            format!("field {} cannot have the collection type {}", f.name, tn(&ft)),
                            "only non-generic declarations may have Vec<i64>, Vec<bool> or String fields",
                        ));
                    }
                }
            }
        }
        for f in &prog.functions {
            self.enter(&fn_owner(&f.name), &f.tparams);
            for p in &f.params {
                self.check_type(f.line, f.col, &GType::from_key(&p.ty))?;
            }
            self.check_type(f.line, f.col, &GType::from_key(&f.ret))?;
        }
        Ok(())
    }

    // ---- G9 A0: type graph, inhabitance, extended types ----
    /// A declared user type written without type arguments.
    fn is_plain_nominal(&self, ty: &GType) -> bool {
        ty.owner.is_none()
            && ty.args.is_empty()
            && self.decls.contains_key(&ty.name)
            && !BUILTIN_TYPE_NAMES.contains(&ty.name.as_str())
    }

    fn type_graph(&mut self, types: &[TypeDecl]) -> Res<()> {
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (i, d) in types.iter().enumerate() {
            index.entry(d.name()).or_insert(i);
        }
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); types.len()];
        for (i, d) in types.iter().enumerate() {
            for (_, fd) in decl_fields(d) {
                // G3: nested type arguments count
                for n in dependencies(&GType::from_key(&fd.ty)) {
                    if let Some(&j) = index.get(n.as_str()) {
                        if !adj[i].contains(&j) {
                            adj[i].push(j);
                        }
                    }
                }
            }
        }
        for comp in sccs(types.len(), &adj) {
            if !(comp.len() > 1 || adj[comp[0]].contains(&comp[0])) {
                continue;
            }
            let names: Vec<String> = comp.iter().map(|&i| types[i].name().to_string()).collect();
            let members: HashSet<&str> = names.iter().map(|n| n.as_str()).collect();
            self.type_sccs.push(names.clone());
            // (a) generic declarations cannot be recursive
            for &i in &comp {
                let d = &types[i];
                if !d.tparams().is_empty() {
                    let cyc: Vec<&str> = cycle_through(i, &adj, &comp).iter().map(|&k| types[k].name()).collect();
                    return Err(terr(
                        d.line(),
                        d.col(),
                        "recursive_generic_type",
                        format!("generic type {} cannot be recursive (cycle: {})", d.name(), cyc.join(" -> ")),
                        "recursive types must be non-generic; write a dedicated enum for the element type",
                    ));
                }
            }
            // (b) intra-SCC fields must be direct nominal fields (these are the boxed edges)
            for &i in &comp {
                let d = &types[i];
                for (vname, fd) in decl_fields(d) {
                    let ft = GType::from_key(&fd.ty);
                    if !dependencies(&ft).iter().any(|n| members.contains(n.as_str())) {
                        continue;
                    }
                    if !(self.is_plain_nominal(&ft) && members.contains(ft.name.as_str())) {
                        return Err(terr(
                            fd.line,
                            fd.col,
                            "recursive_type",
                            format!("recursive field {} must have the type itself, not hide it inside {}", fd.name, tn(&ft)),
                            "use a direct field of a dedicated non-generic type",
                        ));
                    }
                    self.boxed.push(field_label(d.name(), vname, &fd.name));
                    self.boxed_decls.insert((d.name().to_string(), vname.map(|v| v.to_string())));
                }
            }
        }
        Ok(())
    }

    fn inhabitance(&self, types: &[TypeDecl]) -> Res<()> {
        let mut marked: HashSet<String> = HashSet::new();
        // worklist: per struct / per variant count of fields whose type is not yet inhabited
        let mut counts: Vec<Vec<usize>> = Vec::with_capacity(types.len());
        let mut rev: HashMap<String, Vec<(usize, usize)>> = HashMap::new(); // type name -> (decl, unit), one per dependent field
        let mut queue: Vec<usize> = Vec::new();
        for (i, d) in types.iter().enumerate() {
            let units: Vec<&Vec<FieldDecl>> = match d {
                TypeDecl::Struct(s) => vec![&s.fields],
                TypeDecl::Enum(e) => e.variants.iter().map(|v| &v.fields).collect(),
            };
            let mut cs: Vec<usize> = Vec::with_capacity(units.len());
            for (u, fields) in units.iter().enumerate() {
                let mut c = 0usize;
                for fd in fields.iter() {
                    let t = GType::from_key(&fd.ty);
                    if self.is_plain_nominal(&t) {
                        c += 1;
                        rev.entry(t.name.clone()).or_default().push((i, u));
                    }
                }
                cs.push(c);
            }
            if cs.iter().any(|&c| c == 0) {
                queue.push(i);
            }
            counts.push(cs);
        }
        while let Some(i) = queue.pop() {
            let name = types[i].name();
            if marked.contains(name) {
                continue;
            }
            marked.insert(name.to_string());
            if let Some(deps) = rev.get(name) {
                for &(j, u) in deps {
                    counts[j][u] -= 1;
                    if counts[j][u] == 0 && !marked.contains(types[j].name()) {
                        queue.push(j);
                    }
                }
            }
        }
        for d in types {
            if marked.contains(d.name()) {
                continue;
            }
            let mut path: Vec<String> = vec![d.name().to_string()];
            let mut cur: &TypeDecl = d;
            loop {
                let mut nxt: Option<String> = None;
                for (_, fd) in decl_fields(cur) {
                    let t = GType::from_key(&fd.ty);
                    if self.is_plain_nominal(&t) && !marked.contains(&t.name) {
                        nxt = Some(t.name.clone());
                        break;
                    }
                }
                let nxt = match nxt {
                    Some(n) => n,
                    None => break,
                };
                path.push(nxt.clone());
                if path.iter().position(|p| *p == nxt) != Some(path.len() - 1) {
                    break;
                }
                cur = match self.decls.get(&nxt) {
                    Some(c) => c,
                    None => break,
                };
            }
            return Err(terr(
                d.line(),
                d.col(),
                "recursive_type",
                format!("recursive type has no finite value: {}", path.join(" -> ")),
                "add a variant without recursive fields (a base case such as `Empty`)",
            ));
        }
        Ok(())
    }

    /// extended = recursive, or with a buffer field, or with a field mentioning an extended declaration;
    /// buffered = with a buffer field directly or through nested declarations.
    fn compute_extended(&mut self, types: &[TypeDecl]) {
        let rec: HashSet<String> = self.type_sccs.iter().flat_map(|names| names.iter().cloned()).collect();
        let mut direct: HashSet<String> = HashSet::new();
        for d in types {
            if decl_fields(d).iter().any(|(_, fd)| {
                let t = GType::from_key(&fd.ty);
                t.is_array() || t == strt() || t.is_collection()
            }) {
                direct.insert(d.name().to_string());
            }
        }
        let mut ext: HashSet<String> = rec.union(&direct).cloned().collect();
        let mut buf: HashSet<String> = direct.clone();
        // reverse dependency edges built once: mentioned name -> declaration names mentioning it
        let mut rev: HashMap<String, Vec<String>> = HashMap::new();
        for d in types {
            let mut mentioned: HashSet<String> = HashSet::new();
            for (_, fd) in decl_fields(d) {
                mentioned.extend(dependencies(&GType::from_key(&fd.ty)));
            }
            for n in mentioned {
                rev.entry(n).or_default().push(d.name().to_string());
            }
        }
        for marked in [&mut ext, &mut buf] {
            let mut queue: Vec<String> = marked.iter().cloned().collect();
            while let Some(n) = queue.pop() {
                if let Some(ms) = rev.get(&n) {
                    for m in ms {
                        if marked.insert(m.clone()) {
                            queue.push(m.clone());
                        }
                    }
                }
            }
        }
        self.extended = ext;
        self.buffered = buf;
    }

    fn is_extended(&self, ty: &GType) -> bool {
        ty.owner.is_none() && ty.args.is_empty() && self.extended.contains(&ty.name)
    }

    /// An extended type may not be a type argument (type_argument) or a G6 collection payload (foreign_owned_type).
    fn owned_aggregate(&self, ty: &GType) -> bool {
        ty.owner.is_none() && ty.args.is_empty()
            && (self.holder_records.contains(&ty.name)
                || matches!(self.decls.get(&ty.name), Some(TypeDecl::Struct(d))
                if d.tparams.is_empty() && !self.type_sccs.iter().any(|s| s.contains(&ty.name))
                    && d.fields.iter().all(|f| {
                        let t = GType::from_key(&f.ty);
                        crate::gtype::key_is_owned_field(&f.ty)
                            || f.ty == "array_u8"
                            || self.collection_records.borrow().contains(&ty.name) && self.record_map_field(&t)
                            || (!t.contains_collection() && !t.is_array() && t != strt()
                                && dependencies(&t).iter().all(|n| !self.extended.contains(n)))
                    })))
    }

    fn record_map_field(&self, ty: &GType) -> bool {
        let Some(value) = ty.string_record_map_value() else { return false };
        matches!(self.decls.get(&value.name), Some(TypeDecl::Struct(d))
            if d.tparams.is_empty() && d.fields.iter().all(|f| {
                f.ty == "string" || is_value(&GType::from_key(&f.ty), &|n| self.value_decl(n))
            }))
    }

    fn compute_holder_records(&mut self) {
        self.nested_records = self.decls.iter().filter_map(|(name, decl)| {
            let TypeDecl::Struct(d) = decl else { return None };
            d.fields.iter().any(|f| crate::gtype::key_is_nested_collection_field(&f.ty)
                || self.collection_records.borrow().contains(name)
                    && self.record_map_field(&GType::from_key(&f.ty)))
                .then(|| name.clone())
        }).collect();
        self.holder_records = self.decls.iter().filter_map(|(name, decl)| {
            let TypeDecl::Struct(d) = decl else { return None };
            (d.tparams.is_empty() && !self.nested_records.contains(name)
                && d.fields.iter().any(|f| self.nested_child_type(&GType::from_key(&f.ty)))
                && d.fields.iter().all(|f| {
                    let ty = GType::from_key(&f.ty);
                    self.nested_child_type(&ty)
                        || crate::gtype::key_is_owned_field(&f.ty)
                        || f.ty == "array_u8"
                        || is_value(&ty, &|n| self.value_decl(n))
                })).then(|| name.clone())
        }).collect();
        self.nested_records.extend(self.holder_records.iter().cloned());
    }

    fn nested_child_type(&self, ty: &GType) -> bool {
        ty.owner.is_none() && ty.args.is_empty() && self.nested_records.contains(&ty.name)
            && matches!(self.decls.get(&ty.name), Some(TypeDecl::Struct(d))
                if d.tparams.is_empty() && d.fields.iter().all(|f| {
                    let ft = GType::from_key(&f.ty);
                    crate::gtype::key_is_owned_field(&f.ty)
                        || f.ty == "array_u8"
                        || self.collection_records.borrow().contains(&ty.name) && self.record_map_field(&ft)
                        || is_value(&ft, &|n| self.value_decl(n))
                }))
    }

    fn depends_on_nested_record(&self, ty: &GType) -> bool {
        dependencies(ty).iter().any(|name| self.nested_records.contains(name))
    }

    fn state_collection(&self, ty: &GType) -> bool {
        match (ty.name.as_str(), ty.args.as_slice()) {
            ("vec", [v]) => self.owned_aggregate(v),
            ("hashmap", [k, v]) => (*k == i64t() || k.to_key() == "string") && self.owned_aggregate(v),
            _ => false,
        }
    }

    fn is_zero_sized_struct(&self, ty: &GType) -> bool {
        ty.owner.is_none()
            && matches!(self.decls.get(&ty.name), Some(TypeDecl::Struct(d))
                if !self.type_sccs.iter().any(|s| s.contains(&ty.name))
                    && self.bind_decl_args(ty).is_ok_and(|m| d.fields.iter().all(|f| {
                        self.is_zero_sized_struct(&substitute(&GType::from_key(&f.ty), &m))
                    })))
    }

    fn ext_misuse(&self, line: i64, col: i64, ty: &GType, inside: Option<&str>) -> Res<()> {
        if ty.owner.is_some() {
            return Ok(());
        }
        if ty.is_collection() {
            for a in &ty.args {
                self.ext_misuse(line, col, a, Some("coll"))?;
            }
            return Ok(());
        }
        if inside == Some("coll") {
            if self.is_zero_sized_struct(ty) {
                let name = tn(ty);
                return Err(terr(
                    line, col, "foreign_owned_type",
                    format!("{name} has no data and cannot be a Vec or HashMap element"),
                    format!("give {name} a field"),
                ));
            }
            if self.owned_aggregate(ty) {
                return Ok(());
            }
        }
        if self.extended.contains(&ty.name) && ty.args.is_empty() && self.decls.contains_key(&ty.name) {
            if matches!(inside, Some("coll" | "coll_nested")) {
                return Err(terr(
                    line,
                    col,
                    "foreign_owned_type",
                    format!("{} cannot be a Vec or HashMap element: it is recursive or holds an array/str", tn(ty)),
                    "keep extended values in their own bindings",
                ));
            }
            if inside == Some("arg") {
                return Err(terr(
                    line,
                    col,
                    "type_argument",
                    format!(
                        "{} cannot be a type argument or an Option/Result payload: it is recursive or holds an array/str",
                        tn(ty)
                    ),
                    "use a dedicated non-generic enum instead",
                ));
            }
            return Ok(());
        }
        let next = if matches!(inside, Some("coll" | "coll_nested")) { "coll_nested" } else { "arg" };
        for a in &ty.args {
            self.ext_misuse(line, col, a, Some(next))?;
        }
        Ok(())
    }

    fn check_extended_uses(&self, prog: &Program) -> Res<()> {
        for d in &prog.types {
            for (_, fd) in decl_fields(d) {
                let ft = GType::from_key(&fd.ty);
                if !d.tparams().is_empty() {
                    let hit: Vec<String> = dependencies(&ft).into_iter().filter(|n| self.extended.contains(n)).collect();
                    if let Some(first) = hit.first() {
                        return Err(terr(
                            fd.line,
                            fd.col,
                            "field_type",
                            format!(
                                "generic type {} cannot have the field {} of the extended type {}",
                                d.name(),
                                fd.name,
                                first
                            ),
                            "use a dedicated non-generic declaration",
                        ));
                    }
                }
                if matches!(d, TypeDecl::Struct(_)) && ft.is_collection()
                    && !crate::gtype::key_is_owned_field(&fd.ty) && !self.state_collection(&ft)
                {
                    return Err(terr(fd.line, fd.col, "foreign_owned_type", format!("field {} cannot have the collection type {}", fd.name, tn(&ft)), ""));
                }
                self.ext_misuse(fd.line, fd.col, &ft, None)?;
            }
        }
        for f in &prog.functions {
            for p in &f.params {
                self.ext_misuse(f.line, f.col, &GType::from_key(&p.ty), None)?;
            }
            self.ext_misuse(f.line, f.col, &GType::from_key(&f.ret), None)?;
        }
        Ok(())
    }

    /// Inout parameters are scalars, arrays, non-generic structs or supported collections.
    fn check_modes(&self, prog: &Program) -> Res<()> {
        for f in &prog.functions {
            for p in &f.params {
                if let Some((ml, mc)) = p.mode_pos {
                    let pt = GType::from_key(&p.ty);
                    let aggregate_collection = pt.owner.is_none() && pt.name == "hashmap"
                        && pt.args.last().is_some_and(|payload| self.owned_aggregate(payload));
                    if pt != i64t() && pt != boolt() && !pt.is_array()
                        && !(pt.owner.is_none() && pt.name == "vec") && !aggregate_collection
                        && !(pt.args.is_empty() && matches!(self.decls.get(&pt.name), Some(TypeDecl::Struct(_)))) {
                        return Err(terr(
                            ml,
                            mc,
                            "inout_type",
                            format!(
                                "inout parameter {} cannot have the type {}",
                                p.name,
                                tn(&pt)
                            ),
                            "use a scalar, an array, a non-generic struct, or a vector or a map of aggregate records, or remove inout",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn is_result_record(&self, ty: &GType) -> bool {
        let Some(TypeDecl::Struct(d)) = self.decls.get(&ty.name) else { return false };
        if ty.owner.is_some() || !ty.args.is_empty() || !d.tparams.is_empty() {
            return false;
        }
        d.fields.iter().any(|f| GType::from_key(&f.ty).is_collection())
            && d.fields.iter().all(|f| {
                if crate::gtype::key_is_scalar_vec(&f.ty) || f.ty == "vec<string>" {
                    return true;
                }
                let ft = GType::from_key(&f.ty);
                if let ("vec", [element]) = (ft.name.as_str(), ft.args.as_slice()) {
                    if ft.owner.is_none() {
                        return self.owned_aggregate(element)
                            && matches!(self.decls.get(&element.name), Some(TypeDecl::Struct(d))
                                if d.fields.iter().all(|f| {
                                    !crate::gtype::key_is_nested_collection_field(&f.ty)
                                        && (crate::gtype::key_is_owned_field(&f.ty)
                                            || is_value(&GType::from_key(&f.ty), &|n| self.value_decl(n)))
                                }));
                    }
                }
                !ft.contains_collection() && !ft.is_array() && ft != strt()
                    && dependencies(&ft).iter().all(|n| !self.extended.contains(n))
            })
    }

    fn is_result_enum(&self, ty: &GType) -> bool {
        let Some(TypeDecl::Enum(d)) = self.decls.get(&ty.name) else { return false };
        if ty.owner.is_some() || !ty.args.is_empty() || !d.tparams.is_empty() {
            return false;
        }
        let fields = d.variants.iter().flat_map(|v| &v.fields);
        let owned = |key: &str, ft: &GType| {
            crate::gtype::key_is_scalar_vec(key) || key == "vec<string>" || self.is_result_record(ft)
        };
        fields.clone().any(|f| owned(&f.ty, &GType::from_key(&f.ty)))
            && fields.into_iter().all(|f| {
                let ft = GType::from_key(&f.ty);
                owned(&f.ty, &ft)
                    || (!ft.contains_collection() && !ft.is_array() && ft != strt()
                        && dependencies(&ft).iter().all(|n| !self.extended.contains(n)))
            })
    }

    /// Host entries allow plain signatures, scalar vectors and eligible record and enum results.
    fn check_entries(&self, prog: &Program) -> Res<()> {
        fn new_collection(ty: &GType) -> bool {
            matches!(ty.to_key().as_str(), "vec<string>" | "hashmap<i64,bool>" | "hashmap<i64,i64>")
                || ty.name == "hashmap" && ty.args.first().is_some_and(|k| k.to_key() == "string")
                || ty.args.iter().any(new_collection)
        }
        let mut rust_records = HashSet::new();
        loop {
            let mut changed = false;
            for d in &prog.types {
                if decl_fields(d).iter().any(|(_, f)| {
                    let ty = GType::from_key(&f.ty);
                    new_collection(&ty) || dependencies(&ty).iter().any(|name| rust_records.contains(name))
                }) {
                    changed |= rust_records.insert(d.name().to_string());
                }
            }
            if !changed { break; }
        }
        for (i, f) in prog.functions.iter().enumerate() {
            if !self.host_entry.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let Some(p) = f.params.iter().find(|p| p.mode_pos.is_some() && matches!(p.ty.as_str(), "i64" | "bool")) {
                return Err(terr(f.line, f.col, "entry_signature",
                    format!("host entry {} cannot take scalar inout parameter {}", f.name, p.name), ""));
            }
            let rust_state = f.params.first().is_some_and(|p| p.mode_pos.is_some() && rust_records.contains(&p.ty));
            let mut sig: Vec<(String, &str)> = f.params.iter().enumerate().filter(|(i, p)| {
                !(*i == 0 && self.explicit && p.mode_pos.is_some()
                    && matches!(self.decls.get(&p.ty), Some(TypeDecl::Struct(_)))
                    || p.take && p.ty == "string"
                    || !p.take && p.mode_pos.is_none() && (rust_records.contains(&p.ty) || rust_state && p.ty == "string")
                    || p.mode_pos.is_none() && self.owned_aggregate(&GType::from_key(&p.ty)))
            }).map(|(_, p)| (format!("parameter {}", p.name), p.ty.as_str())).collect();
            if let Some(p) = f.params.iter().skip(if self.explicit { 1 } else { 0 })
                .find(|p| p.mode_pos.is_some() && matches!(self.decls.get(&p.ty), Some(TypeDecl::Struct(_))))
            {
                return Err(terr(f.line, f.col, "entry_signature", format!("host entry {} must take inout state {} first", f.name, p.name), ""));
            }
            let ret = GType::from_key(&f.ret);
            if !crate::gtype::key_is_scalar_vec(&f.ret) && !self.is_result_record(&ret) && !self.is_result_enum(&ret) {
                sig.push(("the result".to_string(), f.ret.as_str()));
            }
            for (what, key) in sig {
                let ty = GType::from_key(key);
                if ty.contains_collection() || dependencies(&ty).iter().any(|n| self.extended.contains(n)) {
                    return Err(terr(
                        f.line,
                        f.col,
                        "entry_signature",
                        format!("host entry {} cannot have the type {} in {}", f.name, tn(&ty), what),
                        "keep it Pruv-callable (child `pub` or private) and expose a scalar/value wrapper at the root",
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_fn_sccs(&mut self, functions: &[Function]) -> Res<()> {
        let mut found: Vec<Vec<String>> = Vec::new();
        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (i, n) in names.iter().enumerate() {
            index.insert(*n, i);
        }
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
        for (caller, _, _, _, callee) in &self.call_log {
            if let (Some(&i), Some(&j)) = (index.get(caller.as_str()), index.get(callee.as_str())) {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
        let comps: Vec<Vec<usize>> =
            sccs(names.len(), &adj).into_iter().filter(|comp| comp.len() > 1 || adj[comp[0]].contains(&comp[0])).collect();
        // bucket the call log once per recursive SCC, keeping the original relative order
        let mut scc_of: HashMap<usize, usize> = HashMap::new();
        for (k, comp) in comps.iter().enumerate() {
            for &i in comp {
                scc_of.insert(i, k);
            }
        }
        type Edge<'a> = (usize, i64, i64, &'a Option<Vec<GType>>, usize);
        let mut buckets: Vec<Vec<Edge>> = vec![Vec::new(); comps.len()];
        for (caller, line, col, targs, callee) in &self.call_log {
            if let (Some(&ci), Some(&ce)) = (index.get(caller.as_str()), index.get(callee.as_str())) {
                if let (Some(&a), Some(&b)) = (scc_of.get(&ci), scc_of.get(&ce)) {
                    if a == b {
                        buckets[a].push((ci, *line, *col, targs, ce));
                    }
                }
            }
        }
        for (comp, bucket) in comps.into_iter().zip(buckets.into_iter()) {
            found.push(comp.iter().map(|&i| names[i].to_string()).collect());
            for &i in &comp {
                let f = &functions[i];
                if f.decreases.is_none() {
                    let cyc: Vec<&str> = cycle_through(i, &adj, &comp).iter().map(|&k| names[k]).collect();
                    return Err(terr(
                        f.line,
                        f.col,
                        "recursive_decreases",
                        format!("recursive function {} needs a decreases clause (cycle: {})", f.name, cyc.join(" -> ")),
                        "add `decreases <expr>` after requires/ensures, e.g. `decreases n`",
                    ));
                }
            }
            // (caller index, call line, call col, call targs, callee index), stable-sorted by (caller, line, col)
            let mut edges = bucket;
            edges.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
            for (ci, line, col, targs, ce) in edges {
                let caller = &functions[ci];
                let callee = &functions[ce];
                let mut ok = caller.tparams.len() == callee.tparams.len()
                    && caller.tparams.iter().zip(callee.tparams.iter()).all(|(a, b)| a.bound == b.bound);
                if ok {
                    let empty: Vec<GType> = Vec::new();
                    let ta: &Vec<GType> = targs.as_ref().unwrap_or(&empty);
                    let owner = fn_owner(&caller.name);
                    ok = ta.len() == caller.tparams.len()
                        && ta
                            .iter()
                            .enumerate()
                            .all(|(k, t)| t.owner.as_deref() == Some(owner.as_str()) && t.index == k && t.args.is_empty());
                }
                if !ok {
                    let tp_names: Vec<&str> = caller.tparams.iter().map(|tp| tp.name.as_str()).collect();
                    return Err(terr(
                        line,
                        col,
                        "recursive_polymorphism",
                        format!(
                            "recursive call to {} must pass the type parameters <{}> unchanged",
                            callee.name,
                            tp_names.join(", ")
                        ),
                        "call with ::<T, ...> in the same order",
                    ));
                }
            }
        }
        self.fn_sccs = found;
        Ok(())
    }

    /// G9 A0: the metadata on the program and the type table, only when a G9 feature is present.
    fn record_g9(&mut self, prog: &mut Program) {
        if self.type_sccs.is_empty()
            && self.extended.is_empty()
            && self.fn_sccs.is_empty()
            && !prog.functions.iter().any(|f| f.decreases.is_some())
        {
            return;
        }
        let meta = G9Meta {
            type_sccs: self.type_sccs.clone(),
            boxed: self.boxed.clone(),
            extended: prog.types.iter().filter(|d| self.extended.contains(d.name())).map(|d| d.name().to_string()).collect(),
            fn_sccs: self.fn_sccs.clone(),
        };
        prog.g9 = Some(meta.clone());
        self.tt.g9 = Some(meta);
    }

    // ---- functions ----
    fn fn_(&mut self, f: &mut Function) -> Res<()> {
        let fname = f.name.clone();
        let ret = GType::from_key(&f.ret);
        self.enter(&fn_owner(&fname), &f.tparams);
        self.cur_name = fname.clone();
        self.cur_ret = ret.clone();
        self.cur_cost = f.cost.is_some();
        self.cur_params = f.params.iter().map(|p| p.name.clone()).collect();
        if f.foreign.is_some() {
            check_foreign_decl(f, &self.externs)?; // G6: signature, then path of a trusted rust fn
            if let Some(d) = &f.decreases {
                // G9 A0: foreign termination is trusted
                return Err(terr(
                    d.line(),
                    d.col(),
                    "recursive_decreases",
                    "a trusted rust fn cannot have a decreases clause",
                    "remove it; foreign termination is trusted",
                ));
            }
        } else if fname == "main" && (ret.is_collection() || f.params.iter().any(|p| key_is_collection(&p.ty))) {
            return Err(terr(
                f.line,
                f.col,
                "foreign_export",
                "main cannot have a collection parameter or result",
                "give main plain value parameters and result; use collections inside it",
            ));
        }
        let mut scope: Scope = HashMap::new();
        for p in &f.params {
            if scope.contains_key(&p.name) {
                return Err(terr(f.line, f.col, "dup_param", format!("duplicate parameter {}", p.name), ""));
            }
            let pt = GType::from_key(&p.ty);
            if pt == unitt() {
                return Err(terr(f.line, f.col, "unit_param", format!("parameter {} cannot have type ()", p.name), ""));
            }
            if p.take && !pt.is_array() && !pt.is_collection() && !self.owned_aggregate(&pt) {
                return Err(terr(
                    f.line,
                    f.col,
                    "foreign_move",
                    "take is only for array, collection or aggregate parameters",
                    format!("remove `take` from {}; scalars and str are passed by value or borrowed", p.name),
                ));
            }
            // G5 A0: an inout parameter allows element stores but, like every parameter, never whole reassignment
            let kind = if p.mode_pos.is_some() { "inout" } else if p.take { "take" } else { "param" };
            scope.insert(p.name.clone(), Entry { ty: pt, mutable: false, kind });
        }
        if f.pred {
            if ret == boolt() && f.effects.is_empty() && f.requires.is_empty() && f.ensures.is_empty()
                && f.decreases.is_none() && f.tparams.is_empty() && f.cost.is_none()
            {
                if let [Stmt::Return(r)] = f.body.as_mut_slice() {
                    if let Some(value) = &mut r.value {
                        self.ckind = "ensures";
                        self.expect(value, &boolt(), &scope, true, None)?;
                        self.ckind = "code";
                        return Ok(());
                    }
                }
            }
            return Err(terr(f.line, f.col, "pred_signature",
                format!("pred {} must return bool with no effects, contracts or type parameters and a body of one `return <expr>;`", f.name),
                format!("write `pred fn {}(..) -> bool {{ return <contract expression>; }}`", f.name)));
        }
        self.used_effects = BTreeSet::new();
        self.ckind = "requires";
        for r in f.requires.iter_mut() {
            self.expect(r, &boolt(), &scope, true, None)?;
        }
        let mut escope = scope.clone();
        if ret != unitt() {
            escope.insert("result".to_string(), Entry { ty: ret.clone(), mutable: false, kind: "result" });
        }
        self.ckind = "ensures";
        for e in f.ensures.iter_mut() {
            self.expect(e, &boolt(), &escope, true, None)?;
        }
        self.ckind = "decreases";
        if let Some(d) = f.decreases.as_mut() {
            // G9 A0: scalar measure over the parameters only (no result)
            self.expect(d, &i64t(), &scope, true, None)?;
        }
        self.ckind = "code";
        if f.foreign.is_some() {
            if ret.is_collection() && !f.effects.iter().any(|e| e == "alloc") {
                // G6b: an owned result allocates
                let mut all: BTreeSet<String> = f.effects.iter().cloned().collect();
                all.insert("alloc".to_string());
                let joined = all.into_iter().collect::<Vec<_>>().join(", ");
                return Err(PruvError::new(
                    "effect",
                    f.line,
                    f.col,
                    "missing_effect",
                    format!("trusted rust fn {} returns {} and must declare `alloc`", fname, format_type(&ret)),
                    format!("add `! {}` after the return type", joined),
                ));
            }
            return Ok(()); // a trusted rust fn has no body to check
        }
        let mut body_scope = scope.clone();
        self.block(&mut f.body, &mut body_scope)?;
        if ret != unitt() && !ends_in_return(&f.body) {
            return Err(terr(
                f.line,
                f.col,
                "missing_return",
                format!("function {} may reach its end without returning", fname),
                "end every path with `return ...;`",
            ));
        }
        for e in &self.used_effects {
            if !f.effects.contains(e) {
                let mut all: BTreeSet<String> = f.effects.iter().cloned().collect();
                all.extend(self.used_effects.iter().cloned());
                let joined = all.into_iter().collect::<Vec<_>>().join(", ");
                return Err(PruvError::new(
                    "effect",
                    f.line,
                    f.col,
                    "missing_effect",
                    format!("function {} uses effect `{}` but does not declare it", fname, e),
                    format!("add `! {}` after the return type", joined),
                ));
            }
        }
        let aggregates = self.decls.keys().filter(|n| self.owned_aggregate(&GType::from_key(n))).cloned().collect();
        check_ownership(f, &self.takes, &aggregates)?; // Check ownership and liveness after typing.
        Ok(())
    }

    // ---- statements ----
    fn block(&mut self, stmts: &mut [Stmt], scope: &mut Scope) -> Res<()> {
        let mut rest = stmts;
        while let Some((st, tail)) = rest.split_first_mut() {
            self.stmt(st, scope, tail.first())?;
            rest = tail;
        }
        Ok(())
    }

    fn field_move_out(
        &self,
        value: &Expr,
        ty: &GType,
        target: Option<(&str, &[String])>,
        scope: &Scope,
        next: Option<&Stmt>,
    ) -> Res<Option<(i64, i64)>> {
        let Expr::Move(m) = value else { return Ok(None) };
        if m.fields.is_empty() {
            return Ok(None);
        }
        self.check_borrow(m.line, m.col, &m.name, &m.fields)?;
        let source = self.lookup(m.line, m.col, &m.name, scope, &m.fields)?;
        if !(source.mutable || matches!(source.kind, "inout" | "map_inout"))
            || !matches!(ty.to_key().as_str(), "string" | "vec<i64>" | "vec<bool>")
            || target.is_some_and(|(name, fields)| name != m.name || paths_overlap(fields, &m.fields))
        {
            return Err(field_move_error(m));
        }
        let Some(Stmt::FieldAssign(restore)) = next else { return Err(field_move_error(m)) };
        if restore.name != m.name || restore.fields != m.fields {
            return Err(field_move_error(m));
        }
        let chained = matches!(&restore.value, Expr::Move(other)
            if other.name == m.name && !other.fields.is_empty());
        if !matches!(&restore.value, Expr::Call(_) | Expr::Move(_))
            || (!chained && mentions(&restore.value, &m.name))
        {
            return Err(field_move_error(m));
        }
        Ok(Some((m.line, m.col)))
    }

    fn lookup(&self, line: i64, col: i64, name: &str, scope: &Scope, path: &[String]) -> Res<Entry> {
        if self.map_roots.iter().any(|(root, held, _)| {
            root == name && paths_overlap(path, held)
        }) {
            return Err(terr(line, col, "recursive_borrow", format!("cannot use {name} inside a mutable collection Some arm"), ""));
        }
        match scope.get(name) {
            Some(v) => Ok(v.clone()),
            None => Err(terr(line, col, "unknown_name", format!("unknown variable {}", name), "")),
        }
    }

    fn immutable_fix(kind: &str) -> &'static str {
        if kind == "local" {
            "declare it with `let mut`"
        } else {
            "parameters are immutable; copy into a `let mut` local"
        }
    }

    /// Whether `t` names a declared struct (a type parameter never does).
    fn is_struct(&self, t: &GType) -> bool {
        t.owner.is_none() && self.tt.is_struct(&t.name)
    }

    fn is_enum(&self, t: &GType) -> bool {
        t.owner.is_none() && self.tt.is_enum(&t.name)
    }

    /// R-6g-1: the ordinary argument checks have already handled aliases within each call.
    fn check_inout_context(&self, exprs: &[&Expr], context: InoutContext<'_>) -> Res<()> {
        if !self.uses_inout {
            return Ok(());
        }
        let mut mentions = Vec::new();
        let mut calls = Vec::new();
        match context {
            InoutContext::Assign(name) => mentions.push((name, Vec::new())),
            InoutContext::PlaceAssignment(name, path) => mentions.push((name, path.to_vec())),
            _ => {}
        }
        for expr in exprs {
            place_mentions(expr, &mut mentions);
            inout_statement_calls(expr, matches!(context, InoutContext::WhileCondition), &mut calls);
        }
        let mut effect_before = false;
        for (call, conditional) in calls {
            let f = self.funcs.get(&call.name);
            if f.is_some() || matches!(call.name.as_str(), "map_remove" | "map_take" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice") {
                let marked: Vec<&crate::ast::InoutArg> = call
                    .args
                    .iter()
                    .filter_map(|arg| if let Expr::InoutArg(a) = arg { Some(a) } else { None })
                    .collect();
                if !marked.is_empty() {
                    for arg in &marked {
                        let (root, path) = field_place(&arg.expr);
                        let Expr::Var(root) = root else { continue };
                        let name = &root.name;
                        if mentions.iter().filter(|(other, p)| *other == name && paths_overlap(&path, p)).count() > 1 {
                            return Err(terr(
                                arg.line,
                                arg.col,
                                "inout_context",
                                format!("{} is passed inout and also mentioned elsewhere in this statement", name),
                                "use a separate statement for the inout call",
                            ));
                        }
                    }
                    if conditional {
                        return Err(terr(
                            call.line,
                            call.col,
                            "inout_context",
                            "the inout call may be skipped or repeated",
                            "use a separate statement where the call must run once",
                        ));
                    }
                    if matches!(context, InoutContext::PlaceAssignment(..)) {
                        return Err(terr(
                            call.line,
                            call.col,
                            "inout_context",
                            "an inout call is not allowed in an element or field assignment",
                            "compute the call's result before the assignment",
                        ));
                    }
                    if effect_before {
                        return Err(terr(
                            call.line,
                            call.col,
                            "inout_context",
                            "an inout call is not allowed here because a call with effects runs before it",
                            "evaluate the earlier call in a separate statement",
                        ));
                    }
                }
                effect_before |= f.is_some_and(|f| f.effects.iter().any(|effect| effect != "alloc"));
            } else if !effect_before {
                effect_before = call.name == "print"
                    || catalog_fn(&call.name).is_some_and(|f| f.effects.iter().any(|effect| effect != "alloc"));
            }
        }
        Ok(())
    }

    fn stmt(&mut self, st: &mut Stmt, scope: &mut Scope, next: Option<&Stmt>) -> Res<()> {
        self.ckind = "code";
        self.boundary = None; // G5: only the statements below that allow a mutating call set it
        match st {
            Stmt::Let(s) => {
                if scope.contains_key(&s.name) {
                    return Err(terr(
                        s.line,
                        s.col,
                        "shadow",
                        format!("{} is already defined; shadowing is not allowed", s.name),
                        "use a new name",
                    ));
                }
                let sty = GType::from_key(&s.ty);
                if sty == unitt() {
                    return Err(terr(s.line, s.col, "unit_let", "cannot bind a () value", ""));
                }
                self.check_type(s.line, s.col, &sty)?;
                self.ext_misuse(s.line, s.col, &sty, None)?; // G9 A0
                self.boundary = boundary_of(&s.value);
                self.want_alloc(&s.value, &sty);
                self.field_move = self.field_move_out(&s.value, &sty, None, scope, next)?;
                let checked = self.expect(&mut s.value, &sty, scope, false, None);
                self.field_move = None;
                checked?;
                self.check_inout_context(&[&s.value], InoutContext::Value)?;
                if sty.is_array() || sty == strt() {
                    self.array_source(s.line, s.col, &s.value, scope, false, &sty)?;
                }
                self.own(&s.value, scope)?;
                scope.insert(s.name.clone(), Entry { ty: sty, mutable: s.mutable, kind: "local" });
            }
            Stmt::Assign(s) => {
                let v = self.lookup(s.line, s.col, &s.name, scope, &[])?;
                self.check_borrow(s.line, s.col, &s.name, &[])?; // G9 A0
                let scalar_binder = v.kind == "map_inout" && (v.ty == i64t() || v.ty == boolt());
                let record_binder = v.kind == "map_inout" && self.owned_aggregate(&v.ty);
                let scalar_inout = v.kind == "inout" && (v.ty == i64t() || v.ty == boolt());
                if s.deref && !(scalar_binder || record_binder) {
                    return Err(terr(s.line, s.col, "type_mismatch",
                        "dereference requires a mutable scalar or record collection binder", ""));
                }
                if !(v.mutable || scalar_binder || record_binder || scalar_inout) {
                    if matches!(v.kind, "inout" | "map_inout") {
                        // G5 A0: element stores are allowed, replacing the binding is not
                        return Err(terr(
                            s.line,
                            s.col,
                            "immutable",
                            format!("inout parameter {} cannot be reassigned as a whole", s.name),
                            format!("store elements with {}[i] = v, or copy into a `let mut` local", s.name),
                        ));
                    }
                    return Err(terr(s.line, s.col, "immutable", format!("{} is not mutable", s.name), Self::immutable_fix(v.kind)));
                }
                if record_binder && (mentions(&s.value, &s.name)
                    || self.map_roots.iter().any(|(root, _, binders)| {
                        binders.contains(&s.name) && mentions(&s.value, root)
                    }))
                {
                    return Err(terr(s.value.line(), s.value.col(), "recursive_borrow",
                        "a record replacement cannot mention its binder or a borrowed root", ""));
                }
                self.boundary = boundary_of(&s.value);
                self.want_alloc(&s.value, &v.ty);
                self.expect(&mut s.value, &v.ty, scope, false, None)?;
                if record_binder && !matches!(&s.value, Expr::StructLit(_) | Expr::Call(_))
                    && !matches!(&s.value, Expr::Move(m) if m.fields.is_empty()
                        && scope.get(&m.name).is_some_and(|entry| matches!(entry.kind, "local" | "take")))
                {
                    return Err(terr(s.value.line(), s.value.col(), "foreign_move",
                        "a record replacement needs a constructor, call result or move", ""));
                }
                self.check_inout_context(&[&s.value], InoutContext::Assign(&s.name))?;
                if v.ty.is_array() || v.ty == strt() {
                    self.array_source(s.line, s.col, &s.value, scope, false, &v.ty)?;
                }
                self.own(&s.value, scope)?;
            }
            Stmt::IndexAssign(s) => {
                self.check_iteration_borrow(s.line, s.col, &s.name, &[])?;
                let v = self.lookup(s.line, s.col, &s.name, scope, &[])?;
                if v.ty == strt() {
                    return Err(terr(
                        s.line,
                        s.col,
                        "immutable_string",
                        format!("{} is a str; its bytes cannot be changed", s.name),
                        "build a new str with substr() or copy() instead",
                    ));
                }
                if !v.ty.is_array() {
                    return Err(terr(s.line, s.col, "not_array", format!("{} is not an array", s.name), ""));
                }
                if !v.mutable && v.kind != "inout" {
                    // G5 A0: an inout parameter permits element stores
                    return Err(terr(
                        s.line,
                        s.col,
                        "immutable",
                        format!("array {} is read-only", s.name),
                        format!("use `let mut b: {} = copy({});` (needs alloc)", tn(&v.ty), s.name),
                    ));
                }
                self.expect(&mut s.index, &i64t(), scope, false, None)?;
                                // G2 A5: stored value is bool for a bool array; G3: exactly T for a [T]
                let want_val = elem_type(&v.ty);
                self.expect(&mut s.value, &want_val, scope, false, None)?;
                if Self::is_iteration_binder(&s.value, scope) {
                    self.own(&s.value, scope)?;
                }
                self.check_inout_context(&[&s.index, &s.value], InoutContext::PlaceAssignment(&s.name, &[]))?;
            }
            Stmt::FieldAssign(s) => {
                let v = self.lookup(s.line, s.col, &s.name, scope, &s.fields)?;
                self.check_borrow(s.line, s.col, &s.name, &s.fields)?; // G9 A0
                let mut cur = v.ty.clone();
                for fname in &s.fields {
                    if cur.owner.is_some() {
                        return Err(terr(
                            s.line,
                            s.col,
                            "generic_operation",
                            format!("cannot assign field {}: {} is an abstract type parameter", fname, tn(&cur)),
                            "a type parameter has no fields",
                        ));
                    }
                    if !self.is_struct(&cur) {
                        return Err(terr(
                            s.line,
                            s.col,
                            "not_struct",
                            format!("cannot assign field {}: {} is not a struct", fname, tn(&cur)),
                            "field assignment paths run through structs only; assign the whole enum value instead",
                        ));
                    }
                    cur = self.field_type(s.line, s.col, &cur, fname)?;
                }
                let moved = self.field_move_out(&s.value, &cur, Some((&s.name, &s.fields)), scope, next)?;
                if !v.mutable && !matches!(v.kind, "inout" | "map_inout") {
                    if let Some(m) = first_field_move(&s.value) {
                        self.check_borrow(m.line, m.col, &m.name, &m.fields)?;
                        self.lookup(m.line, m.col, &m.name, scope, &m.fields)?;
                        return Err(field_move_error(m));
                    }
                    return Err(terr(s.line, s.col, "immutable", format!("{} is not mutable", s.name), Self::immutable_fix(v.kind)));
                }
                let saved = std::mem::replace(&mut self.field_move, moved);
                if matches!(v.kind, "local" | "inout" | "map_inout")
                    && (crate::gtype::key_is_owned_field(&cur.to_key()) || self.state_collection(&cur))
                {
                    if let Expr::Call(call) = &s.value {
                        self.field_move = call.args.iter().enumerate().find_map(|(i, arg)| {
                            let Expr::Move(m) = arg else { return None };
                            if m.name != s.name || m.fields != s.fields {
                                return None;
                            }
                            let take = if let Some(takes) = self.takes.get(&call.name) {
                                takes.get(i).copied().unwrap_or(false)
                            } else {
                                catalog_fn(&call.name).and_then(|f| f.params.get(i).map(|p| p.take)).unwrap_or(false)
                            };
                            (take && !call.args.iter().enumerate().any(|(j, arg)| j != i && mentions(arg, &s.name)))
                                .then_some((m.line, m.col))
                        });
                    }
                }
                let checked = self.expect(&mut s.value, &cur, scope, false, None);
                self.field_move = saved;
                checked?;
                self.check_inout_context(&[&s.value], InoutContext::PlaceAssignment(&s.name, &s.fields))?;
                if cur.is_array() || cur == strt() {
                    // G9 A0: G2 explicit ownership for a buffer field
                    self.array_source(s.line, s.col, &s.value, scope, false, &cur)?;
                }
                self.own(&s.value, scope)?;
            }
            Stmt::If(s) => {
                self.expect(&mut s.cond, &boolt(), scope, false, Some("a condition"))?;
                self.check_inout_context(&[&s.cond], InoutContext::Value)?;
                let mut sc = scope.clone();
                self.block(&mut s.then, &mut sc)?;
                let mut sc = scope.clone();
                self.block(&mut s.els, &mut sc)?;
            }
            Stmt::While(s) => {
                self.expect(&mut s.cond, &boolt(), scope, false, Some("a condition"))?;
                self.check_inout_context(&[&s.cond], InoutContext::WhileCondition)?;
                self.ckind = "invariant";
                for i in s.invariants.iter_mut() {
                    self.expect(i, &boolt(), scope, true, None)?;
                }
                self.ckind = "decreases";
                self.expect(&mut s.decreases, &i64t(), scope, true, None)?;
                self.ckind = "code";
                let mut sc = scope.clone();
                self.block(&mut s.body, &mut sc)?;
            }
            Stmt::ForMap(s) => self.for_map(s, scope)?,
            Stmt::Return(s) => {
                if let Some(m) = s.value.as_ref().and_then(first_field_move) {
                    self.check_borrow(m.line, m.col, &m.name, &m.fields)?;
                    self.lookup(m.line, m.col, &m.name, scope, &m.fields)?;
                    return Err(field_move_error(m));
                }
                if !self.map_iterations.is_empty() {
                    return Err(terr(s.line, s.col, "foreign_contract", "cannot return inside a map iteration", ""));
                }
                if !self.map_roots.is_empty() {
                    return Err(terr(s.line, s.col, "foreign_contract", "cannot return inside a mutable collection Some arm", ""));
                }
                let ret = self.cur_ret.clone();
                match &mut s.value {
                    None => {
                        if ret != unitt() {
                            return Err(terr(
                                s.line,
                                s.col,
                                "return_type",
                                format!("function returns {}; `return;` is not enough", tn(&ret)),
                                "",
                            ));
                        }
                    }
                    Some(v) => {
                        if ret == unitt() {
                            return Err(terr(s.line, s.col, "return_type", "function returns (); use `return;`", ""));
                        }
                        self.boundary = boundary_of(v);
                        self.want_alloc(v, &ret);
                        self.expect(v, &ret, scope, false, None)?;
                        self.check_inout_context(&[&*v], InoutContext::Value)?;
                        if ret.is_array() || ret == strt() {
                            self.array_source(s.line, s.col, v, scope, true, &ret)?;
                        }
                        self.own(v, scope)?;
                    }
                }
            }
            Stmt::Assert(s) if s.reveal.is_some() => {
                // P13o: `reveal F;` names an opaque function
                let (rn, rl, rc) = s.reveal.clone().unwrap();
                let why = match self.funcs.get(&rn) {
                    Some(rf) if rf.opaque => None,
                    Some(_) => Some("the function is not opaque"),
                    None => Some("unknown function"),
                };
                if let Some(why) = why {
                    return Err(terr(
                        rl,
                        rc,
                        "reveal_target",
                        format!("reveal {}: {}", rn, why),
                        "reveal names an `opaque fn`; a function without `opaque` is always transparent",
                    ));
                }
            }
            Stmt::Assert(s) => {
                self.ckind = "assert";
                self.expect(&mut s.cond, &boolt(), scope, true, None)?;
                self.ckind = "code";
            }
            Stmt::ExprStmt(s) => {
                if !matches!(s.expr, Expr::Call(_)) {
                    if let Some(m) = first_field_move(&s.expr) {
                        return Err(field_move_error(m));
                    }
                    return Err(terr(
                        s.line,
                        s.col,
                        "useless_expr",
                        "expression statement must be a call",
                        "bind it with let or remove it",
                    ));
                }
                self.boundary = boundary_of(&s.expr);
                if let Expr::Call(call) = &s.expr {
                    self.statement_call = Some(call as *const Call as usize);
                }
                let result = self.expr(&mut s.expr, scope, false);
                self.statement_call = None;
                let t = result?;
                self.check_inout_context(&[&s.expr], InoutContext::Value)?;
                if t.owner.is_none() && t.name == "process" {
                    // G10 proc: a discarded Process result keeps its duty
                    return Err(terr(
                        s.line,
                        s.col,
                        "proc_unconsumed",
                        "temporary Process is not consumed",
                        "consume it with proc_wait(move p) or proc_kill(move p), or return / transfer ownership",
                    ));
                }
                if t.owner.is_none() && t.name == "loop" {
                    return Err(terr(s.line, s.col, "async_unconsumed", "temporary Loop is not consumed", "consume it with ev_close(move l), or return / transfer ownership"));
                }
                if t.owner.is_none() && (t.name == "socket" || t.name == "listener") {
                    // G10 net A0: a discarded Socket / Listener keeps its duty
                    let what = if t.name == "socket" { "Socket" } else { "Listener" };
                    return Err(terr(
                        s.line,
                        s.col,
                        "net_unconsumed",
                        format!("temporary {} is not consumed", what),
                        "consume it with net_close(move s) or net_listener_close(move l), or return / transfer ownership",
                    ));
                }
                if t != unitt() {
                    return Err(terr(
                        s.line,
                        s.col,
                        "unused_value",
                        format!("value of type {} is dropped", tn(&t)),
                        "bind it with `let` or `match` it",
                    ));
                }
            }
            Stmt::Par(p) => self.par(p, scope)?,
            Stmt::Match(m) => self.match_stmt(m, scope)?,
        }
        Ok(())
    }

    // G7: preserve Python's validation order and source-order alias witnesses.
    fn par(&mut self, p: &mut Par, scope: &Scope) -> Res<()> {
        if p.calls.len() < 2 {
            return Err(terr(p.line, p.col, "parallel_arity",
                format!("a par group needs at least two branch calls, found {}", p.calls.len()),
                "write a single ordinary call instead of a par group"));
        }
        for e in &p.calls {
            self.par_target(e)?;
        }
        for e in &mut p.calls {
            self.ckind = "par";
            self.boundary = boundary_of(e);
            self.expr(e, scope, false)?;
        }
        self.ckind = "code";
        self.boundary = None;
        for e in &p.calls {
            self.par_args(e)?;
        }
        Self::par_overlap(&p.calls)?;
        self.par_groups.push(p.calls.clone());
        Ok(())
    }

    fn par_target(&self, e: &Expr) -> Res<()> {
        let c = match e {
            Expr::Call(c) => c,
            _ => return Err(terr(e.line(), e.col(), "parallel_call",
                "a par branch must be a call to a Pruv function",
                "call a unit Pruv helper that takes the arrays as inout parameters")),
        };
        let n = &c.name;
        if let Some(f) = self.funcs.get(n) {
            if f.foreign {
                return Err(terr(c.line, c.col, "parallel_call",
                    format!("{} is a trusted rust fn: it cannot be a par branch", n),
                    "call a unit Pruv helper, and keep trusted rust fns outside the par group"));
            }
            if f.ret != unitt() {
                return Err(terr(c.line, c.col, "parallel_call",
                    format!("{} returns {}: a par branch must return ()", n, tn(&f.ret)),
                    "call a unit Pruv helper and pass results back through whole inout arrays"));
            }
        } else if ["len", "alloc", "copy", "print", "is_char_boundary", "substr", "truncate", "eq_range", "resize", "copy_range", "copy_within"].contains(&n.as_str())
            || is_bit_builtin(n)
            || is_word_builtin(n)
            || contract_only(n).is_some() || is_catalog(n) || (n == "old" && self.uses_inout)
        {
            return Err(terr(c.line, c.col, "parallel_call",
                format!("{} is a builtin or catalog operation: it cannot be a par branch", n),
                "call a unit Pruv helper that performs the operation"));
        }
        Ok(())
    }

    fn par_type_ok(&self, t: &GType) -> bool {
        is_element(t, &self.elem_params)
            || matches!(t.to_key().as_str(), "array" | "array_u8" | "array_bool")
            || (t.name == "array" && t.args.len() == 1
                && t.args[0].owner.is_some() && is_element(&t.args[0], &self.elem_params))
    }

    fn par_args(&self, e: &Expr) -> Res<()> {
        if let Expr::Call(c) = e {
            for arg in &c.args {
                if !matches!(arg, Expr::InoutArg(_) | Expr::Var(_) | Expr::IntLit(_) | Expr::BoolLit(_))
                    || matches!(arg, Expr::InoutArg(a) if !matches!(&*a.expr, Expr::Var(_)))
                {
                    return Err(terr(arg.line(), arg.col(), "parallel_argument",
                        "a par branch argument must be a binding name, an integer or bool literal, or `inout name`",
                        "bind the value before the group, e.g. `let n: i64 = len(a);`, and pass n"));
                }
                let t = expr_ty(arg).as_ref().map(|k| GType::from_key(k));
                if !t.as_ref().map_or(false, |t| self.par_type_ok(t)) {
                    return Err(terr(arg.line(), arg.col(), "parallel_argument",
                        format!("an argument of type {} cannot cross a par branch boundary",
                            t.as_ref().map(tn).unwrap_or("?".into())),
                        "bind a scalar first, or pass whole [i64], [u8] or [bool] arrays"));
                }
            }
        }
        Ok(())
    }

    fn par_overlap(calls: &[Expr]) -> Res<()> {
        // Vectors preserve Python dict insertion order and the first operand at each binding.
        let sets: Vec<_> = calls.iter().map(|e| {
            let mut w: Vec<(&str, &Expr)> = Vec::new();
            let mut r: Vec<(&str, &Expr)> = Vec::new();
            if let Expr::Call(c) = e {
                for a in &c.args {
                    match a {
                        Expr::InoutArg(x) => if let Expr::Var(v) = x.expr.as_ref() {
                            if !w.iter().any(|(n, _)| *n == v.name) {
                                w.push((&v.name, a));
                            }
                        },
                        Expr::Var(v) => {
                            if !r.iter().any(|(n, _)| *n == v.name) {
                                r.push((&v.name, a));
                            }
                        }
                        _ => {}
                    }
                }
            }
            (w, r)
        }).collect();
        for j in 1..sets.len() {
            for i in 0..j {
                for (name, node) in &sets[j].0 {
                    for (set, theirs) in [(&sets[i].0, "also writes"), (&sets[i].1, "reads")] {
                        if set.iter().any(|(n, _)| n == name) {
                            return Err(Self::par_alias(node, name, j, i, "writes", theirs));
                        }
                    }
                }
                for (name, node) in &sets[j].1 {
                    if sets[i].0.iter().any(|(n, _)| n == name) {
                        return Err(Self::par_alias(node, name, j, i, "reads", "writes"));
                    }
                }
            }
        }
        Ok(())
    }

    fn par_alias(node: &Expr, name: &str, j: usize, i: usize, mine: &str, theirs: &str) -> PruvError {
        terr(node.line(), node.col(), "parallel_alias",
            format!("branch {} {} `{}`, which branch {} {}", j + 1, mine, name, i + 1, theirs),
            "use separate bindings for the two branches, or call the helpers one after another")
    }

    fn check_par_eligibility(&self, functions: &[Function]) -> Res<()> {
        if self.par_groups.is_empty() {
            return Ok(());
        }
        let infos: HashMap<_, _> = functions.iter().map(|f| (f.name.clone(), par_info(f, &self.funcs))).collect();
        for calls in &self.par_groups {
            for e in calls {
                if let Expr::Call(c) = e {
                    // At each branch, effect witnesses take precedence over foreign and nesting witnesses.
                    for kind in 0..3 {
                        if let Some(path) = par_witness(&c.name, kind, &infos) {
                            let last = path.last().unwrap();
                            let info = &infos[last];
                            let (stage, code, msg, fix) = match kind {
                                0 => {
                                    let eff = info.effects[0].as_str();
                                    ("effect", "parallel_effect",
                                        format!("par branch {} reaches {}, which declares the `{}` effect ({})",
                                            c.name, last, eff, path.join(" -> ")),
                                        format!("move the {} work outside the par group", if eff == "io" { "I/O" } else if eff == "fs" { "file" } else if eff == "async" { "async" } else if eff == "net" { "network" } else { "process" }))
                                }
                                1 => {
                                    let leaf = info.foreign.as_ref().unwrap();
                                    let mut chain = path.clone();
                                    if leaf != last { chain.push(leaf.clone()); }
                                    ("type", "parallel_foreign",
                                        format!("par branch {} reaches the trusted rust fn or catalog operation {} ({})",
                                            c.name, leaf, chain.join(" -> ")),
                                        "use Pruv computation below the par group, and keep foreign calls outside it".into())
                                }
                                _ => ("type", "parallel_nested",
                                    format!("par branch {} reaches {}, which contains a par group ({})",
                                        c.name, last, path.join(" -> ")),
                                    "keep only the outer par group; call the inner helpers one after another".into()),
                            };
                            return Err(PruvError::new(stage, c.line, c.col, code, msg, fix));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn match_stmt(&mut self, m: &mut Match, scope: &mut Scope) -> Res<()> {
        let (line, col) = (m.line, m.col);
        let t = self.read_expr(&mut m.scrutinee, scope, false, &["map_get", "map_get_mut", "map_take", "vec_at_mut"])?;
        let map_call = match &m.scrutinee {
            Expr::Call(call) if !self.funcs.contains_key(&call.name) => Some(call),
            _ => None,
        };
        let map_mut = map_call.is_some_and(|call| matches!(call.name.as_str(), "map_get_mut" | "vec_at_mut"));
        let map_take = map_call.is_some_and(|call| call.name == "map_take");
        let map_read = map_call.filter(|call| call.name == "map_get");
        let map_read_path = map_read.and_then(|call| {
            let (root, path) = field_place(&call.args[0]);
            let Expr::Var(v) = root else { return None };
            Some((v.name.clone(), path))
        });
        let nested_map_path = map_read.and_then(|call| {
            GType::from_key(expr_ty(&call.args[0]).as_deref()?).string_record_map_value()?;
            let (root, path) = field_place(&call.args[0]);
            let Expr::Var(v) = root else { return None };
            (!path.is_empty() && scope.get(&v.name).is_some_and(|entry| entry.kind == "map_inout"))
                .then(|| (v.name.clone(), path))
        });
        let map_borrow = map_read.is_some()
            && (t.args.first().is_some_and(|ty| self.is_extended(ty)) || nested_map_path.is_some());
        self.check_inout_context(&[&m.scrutinee], InoutContext::Value)?;
        let user = self.is_enum(&t);
        let mutable_place = map_call.filter(|_| map_mut).and_then(|call| {
            let Expr::InoutArg(arg) = &call.args[0] else { return None };
            let (root, path) = field_place(&arg.expr);
            let Expr::Var(v) = root else { return None };
            Some((v.name.clone(), path))
        });
        let mpath = self.match_inout_path(&m.scrutinee, &t, scope);
        let root = if let Some(call) = map_call.filter(|_| map_borrow && nested_map_path.is_none()) {
            place_root(&call.args[0]).map(str::to_string)
        } else if mpath.is_none() {
            self.match_root(&m.scrutinee, &t, scope)
        } else {
            None
        }; // G9 A0: root variable of an extended scrutinee path
        if t.owner.is_some() {
            return Err(terr(
                line,
                col,
                "generic_operation",
                format!("cannot match a value of the abstract type {}", t.name),
                "a type parameter has no variants; match on an Option, Result or enum that contains it",
            ));
        }
        let need: Vec<String> = if t.name == "option" {
            vec!["Some".to_string(), "None".to_string()]
        } else if t.name == "result" {
            vec!["Ok".to_string(), "Err".to_string()]
        } else if user {
            match self.tt.decl(&t.name) {
                Some(TypeDecl::Enum(e)) => e.variants.iter().map(|v| v.name.clone()).collect(),
                _ => Vec::new(),
            }
        } else {
            return Err(terr(line, col, "match_type", "can only match Option<i64>, Result<i64> or a user enum", ""));
        };
        if !user {
            let got: Vec<&String> = m.arms.iter().map(|a| &a.ctor).collect();
            let got_set: BTreeSet<&String> = got.iter().cloned().collect();
            let need_set: BTreeSet<&String> = need.iter().collect();
            if got_set != need_set || got.len() != 2 || m.arms.iter().any(|a| a.enum_name.is_some()) {
                let sorted: Vec<&String> = need_set.into_iter().collect();
                let joined = sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" and ");
                return Err(terr(line, col, if map_mut { "foreign_contract" } else { "match_arms" }, format!("match needs exactly arms {}", joined), ""));
            }
        } else {
            let mut seen: Vec<&str> = Vec::new();
            for a in &m.arms {
                if a.enum_name.as_deref() != Some(t.name.as_str()) {
                    let what = match &a.enum_name {
                        Some(en) if !en.is_empty() => format!("{}::{}", en, a.ctor),
                        _ => a.ctor.clone(),
                    };
                    return Err(terr(
                        a.line,
                        a.col,
                        "match_arms",
                        format!("arm {} does not belong to enum {}", what, t.name),
                        format!("match arms are written {}::Variant", t.name),
                    ));
                }
                if !need.contains(&a.ctor) {
                    return Err(terr(a.line, a.col, "unknown_variant", format!("enum {} has no variant {}", t.name, a.ctor), ""));
                }
                if seen.contains(&a.ctor.as_str()) {
                    return Err(terr(a.line, a.col, "match_arms", format!("arm {}::{} appears twice", t.name, a.ctor), ""));
                }
                seen.push(&a.ctor);
            }
            let missing: Vec<String> = need.iter().filter(|n| !seen.contains(&n.as_str())).map(|n| format!("{}::{}", t.name, n)).collect();
            if !missing.is_empty() {
                return Err(terr(line, col, "match_arms", format!("match is missing arm(s): {}", missing.join(", ")), ""));
            }
        }
        for a in m.arms.iter_mut() {
            let mut sc = scope.clone();
            let (nl, nc) = if user { (a.line, a.col) } else { (line, col) };
            let ftypes: Vec<GType> = if user {
                let v: Vec<GType> = self.fields_of(&t, Some(a.ctor.as_str()))?.into_iter().map(|(_, ft)| ft).collect();
                if a.binders.len() != v.len() {
                    return Err(terr(
                        a.line,
                        a.col,
                        "match_arity",
                        format!("{}::{} has {} field(s), the pattern binds {}", t.name, a.ctor, v.len(), a.binders.len()),
                        "bind every field, in declaration order",
                    ));
                }
                v
            } else {
                // G3: the Some/Ok binder has the payload type of Option<X> / Result<X>; Err binds the i64 code
                let payload = if !t.args.is_empty() && a.ctor != "Err" { t.args[0].clone() } else { i64t() };
                vec![payload; a.binders.len()]
            };
            for (b, bt) in a.binders.iter().zip(ftypes.iter()) {
                if sc.contains_key(b) {
                    return Err(terr(nl, nc, "shadow", format!("{} is already defined", b), "use a new name"));
                }
                // G9 A0: a buffer payload is a read view (like a parameter): returning it needs copy()
                let kind = if map_mut { "map_inout" } else if map_borrow { "map_view" }
                    else if bt.is_array() || *bt == strt() { "param" } else { "local" };
                sc.insert(b.clone(), Entry { ty: bt.clone(), mutable: map_take, kind });
            }
            let active = root.as_ref().filter(|_| !map_borrow || a.ctor == "Some");
            let borrowed = mutable_place.as_ref().filter(|_| a.ctor == "Some");
            let arm_path = mpath.as_ref().or(nested_map_path.as_ref().filter(|_| a.ctor == "Some"));
            if let Some(r) = active {
                self.active_roots.push((r.clone(), map_read_path.is_some()));
            }
            if let Some((root, path)) = borrowed {
                self.map_roots.push((root.clone(), path.clone(), a.binders.clone()));
            }
            if let Some(mp) = arm_path {
                self.match_paths.push(mp.clone());
            }
            let read_path = map_read_path.as_ref().filter(|_| a.ctor == "Some");
            if let Some(place) = read_path {
                self.map_read_paths.push(place.clone());
            }
            self.block(&mut a.body, &mut sc)?;
            if read_path.is_some() {
                self.map_read_paths.pop();
            }
            if arm_path.is_some() {
                self.match_paths.pop();
            }
            if borrowed.is_some() {
                self.map_roots.pop();
            }
            if active.is_some() {
                self.active_roots.pop();
            }
        }
        Ok(())
    }

    // ---- G9 A0: ownership of extended values, active-match borrows ----
    /// `value` (already typed) is moved into an owning position (let, assignment, constructor field, return):
    /// a non-fresh extended value is cloned, which needs `alloc`; constructors, literals and calls are fresh.
    fn own(&mut self, value: &Expr, scope: &Scope) -> Res<()> {
        if Self::is_iteration_binder(value, scope) {
            return Err(terr(value.line(), value.col(), "foreign_move",
                "a map iteration binder cannot be used as an owned value", ""));
        }
        if matches!(value, Expr::Var(v) if scope.get(&v.name).is_some_and(|e| e.kind == "map_inout" && e.ty != i64t() && e.ty != boolt())) {
            return Err(terr(value.line(), value.col(), "foreign_move",
                "a mutable collection binder cannot be used as an owned value", ""));
        }
        if let Some(k) = expr_ty(value) {
            let t = GType::from_key(k);
            if self.is_extended(&t) && !matches!(value, Expr::EnumCtor(_) | Expr::StructLit(_) | Expr::Call(_) | Expr::Move(_)) {
                self.used_effects.insert("alloc".to_string());
            }
        }
        Ok(())
    }

    /// A constructor / struct-literal field argument: owning position, G2 explicit ownership for buffers.
    fn own_field(&mut self, value: &Expr, ftype: &GType, scope: &Scope, c: bool) -> Res<()> {
        if c {
            return Ok(());
        }
        if ftype.is_array() || *ftype == strt() {
            self.array_source(value.line(), value.col(), value, scope, false, ftype)?;
        }
        self.own(value, scope)
    }

    /// A constructor that initializes a boxed field allocates.
    fn ctor_allocs(&mut self, decl_name: &str, variant: Option<&str>, c: bool) {
        if !c && self.boxed_decls.contains(&(decl_name.to_string(), variant.map(|v| v.to_string()))) {
            self.used_effects.insert("alloc".to_string());
        }
    }

    /// Root variable of a match scrutinee that is a variable / field path over an extended type, else None.
    fn match_root(&self, scrut: &Expr, t: &GType, scope: &Scope) -> Option<String> {
        if !self.is_extended(t) {
            return None;
        }
        let mut e: &Expr = scrut;
        while let Expr::FieldAccess(x) = e {
            e = &x.base;
        }
        if let Expr::Var(v) = e {
            if scope.get(&v.name).map(|x| x.kind == "local" || x.kind == "param").unwrap_or(false) {
                return Some(v.name.clone());
            }
        }
        None
    }

    /// R-6e-8: the held path of an extended match on an inout, element binding or mutable local field.
    fn match_inout_path(&self, scrut: &Expr, t: &GType, scope: &Scope) -> Option<(String, Vec<String>)> {
        if !self.is_extended(t) {
            return None;
        }
        let (e, fields) = field_place(scrut);
        if let Expr::Var(v) = e {
            if scope.get(&v.name).is_some_and(|x| {
                matches!(x.kind, "inout" | "map_inout") || !fields.is_empty() && x.kind == "local" && x.mutable
            }) {
                return Some((v.name.clone(), fields));
            }
        }
        None
    }

    fn for_map(&mut self, st: &mut ForMap, scope: &Scope) -> Res<()> {
        if self.cur_cost {
            return Err(terr(st.line, st.col, "foreign_contract",
                "map iteration is not allowed in a function with a cost clause", ""));
        }
        let (root, path) = field_place(&st.place);
        let Expr::Var(root) = root else {
            return Err(terr(st.place.line(), st.place.col(), "foreign_contract",
                "map iteration requires a binding or field path", ""));
        };
        let root = root.name.clone();
        let ty = self.expr(&mut st.place, scope, false)?;
        let val = match (ty.owner.is_none(), ty.name.as_str(), ty.args.as_slice()) {
            (true, "hashmap", [k, v]) if k.to_key() == "string"
                && (*v == i64t() || *v == boolt() || self.owned_aggregate(v)) => v.clone(),
            _ => return Err(terr(st.place.line(), st.place.col(), "foreign_owned_type",
                "map iteration requires HashMap<String, V> with scalar or owned aggregate values", "")),
        };
        self.ckind = "invariant";
        for inv in &mut st.invariants {
            self.expect(inv, &boolt(), scope, true, None)?;
        }
        self.ckind = "code";
        let mut inner = scope.clone();
        for (name, ty) in [(&st.key, GType::from_key("string")), (&st.val, val)] {
            if inner.contains_key(name) {
                return Err(terr(st.line, st.col, "shadow", format!("{} is already defined", name), "use a new name"));
            }
            inner.insert(name.clone(), Entry { ty, mutable: false, kind: "map_iter" });
        }
        self.map_iterations.push((root, path));
        let result = self.block(&mut st.body, &mut inner);
        self.map_iterations.pop();
        result
    }

    fn is_iteration_binder(e: &Expr, scope: &Scope) -> bool {
        matches!(e, Expr::Var(v) if scope.get(&v.name).is_some_and(|entry| entry.kind == "map_iter"))
    }

    fn expect_str_arg(&mut self, e: &mut Expr, scope: &Scope, contract: bool) -> Res<()> {
        let got = self.text_expr(e, scope, contract)?;
        if got.to_key() == "string" && Self::is_iteration_binder(e, scope) {
            return Ok(());
        }
        self.check_expected_type(e, &strt(), None, got)?;
        Ok(())
    }

    fn expect_user_arg(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, c: bool, borrow_vector: bool) -> Res<()> {
        if *ty == strt() {
            return self.expect_str_arg(e, scope, c);
        }
        let got = self.expr(e, scope, c)?;
        if borrow_vector && place_root(e).is_some() && key_is_vec_slice_arg(&got.to_key(), &ty.to_key()) {
            return Ok(());
        }
        self.check_expected_type(e, ty, None, got)?;
        Ok(())
    }

    fn match_path_held(&self, name: &str, path: &[String]) -> bool {
        self.match_paths.iter().any(|(root, held)| root == name && paths_overlap(path, held))
    }

    fn check_iteration_borrow(&self, line: i64, col: i64, name: &str, path: &[String]) -> Res<()> {
        if self.map_iterations.iter().any(|(root, held)|
            root == name && (path.starts_with(held) || held.starts_with(path))) {
            let place = std::iter::once(name).chain(path.iter().map(String::as_str)).collect::<Vec<_>>().join(".");
            return Err(terr(line, col, "recursive_borrow",
                format!("cannot mutate or move {} during map iteration", place), ""));
        }
        Ok(())
    }

    fn check_borrow(&self, line: i64, col: i64, name: &str, fields: &[String]) -> Res<()> {
        self.check_iteration_borrow(line, col, name, fields)?;
        if self.active_roots.iter().any(|(root, _)| root == name) || self.match_path_held(name, fields) {
            let mut parts: Vec<&str> = vec![name];
            parts.extend(fields.iter().map(|f| f.as_str()));
            return Err(terr(
                line,
                col,
                "recursive_borrow",
                format!("cannot assign to {} while a match on it is active", parts.join(".")),
                "copy the needed payload into a local, finish the match, then assign",
            ));
        }
        Ok(())
    }

    /// Arrays are moved or borrowed explicitly: a new array comes only from alloc/copy/a call.
    fn array_source(&self, line: i64, col: i64, value: &Expr, scope: &Scope, allow_local_var: bool, ty: &GType) -> Res<()> {
        if matches!(value, Expr::Call(_) | Expr::StrLit(_)) || matches!(value, Expr::Move(_)) && ty.is_array() {
            return Ok(());
        }
        if let Expr::Var(v) = value {
            if scope.get(&v.name).map(|x| x.kind == "inout").unwrap_or(false) {
                // G5 A0
                return Err(terr(
                    line,
                    col,
                    "array_alias",
                    format!("inout parameter {} is a borrowed array: it cannot be stored or returned", v.name),
                    format!("write copy({}) to make an independent array (needs `! alloc`)", v.name),
                ));
            }
        }
        if allow_local_var {
            if let Expr::Var(v) = value {
                if scope.get(&v.name).map(|x| x.kind == "local").unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        Err(terr(
            line,
            col,
            "array_alias",
            if *ty == strt() {
                "a str value here must come from a string literal, copy(), substr() or a function call"
            } else {
                "an array value here must come from move, alloc(), copy() or a function call"
            },
            "write copy(x) to make an explicit copy (needs `! alloc`)",
        ))
    }

    /// The argument must be an array of any element type (G2 B0: or a str). coll (G6): a Vec/HashMap/String is accepted too.
    fn expect_array(&mut self, e: &mut Expr, scope: &Scope, c: bool, coll: bool) -> Res<GType> {
        let t = self.expr(e, scope, c)?;
        self.check_array_type(e, coll, t)
    }

    fn check_array_type(&self, e: &Expr, coll: bool, t: GType) -> Res<GType> {
        if t.owner.is_some() {
            return Err(terr(
                e.line(),
                e.col(),
                "generic_operation",
                format!("{} is an abstract type parameter, not an array", tn(&t)),
                "only [T] with T: Element can be indexed, measured or copied",
            ));
        }
        if !t.is_array() && t != strt() && !(coll && t.is_collection()) {
            return Err(terr(
                e.line(),
                e.col(),
                "type_mismatch",
                format!("expected {}, found {}", tn(&GType::array()), tn(&t)),
                "",
            ));
        }
        Ok(t)
    }

    // ---- expressions ----
    /// op: what the operand is used for (arithmetic, comparison, ...) when the operation needs a concrete type:
    /// an abstract type parameter there is a generic_operation error rather than a plain mismatch.
    fn expect(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, contract: bool, op: Option<&str>) -> Res<GType> {
        let t = self.expr(e, scope, contract)?;
        self.check_expected_type(e, ty, op, t)
    }

    fn check_expected_type(&self, e: &Expr, ty: &GType, op: Option<&str>, t: GType) -> Res<GType> {
        if t != *ty {
            if let Some(op) = op {
                if t.owner.is_some() {
                    return Err(terr(
                        e.line(),
                        e.col(),
                        "generic_operation",
                        format!("{} is not allowed on the abstract type {}", op, t.name),
                        "a type parameter can be copied, passed, returned and stored; == works on it in contracts only",
                    ));
                }
            }
            return Err(terr(
                e.line(),
                e.col(),
                "type_mismatch",
                format!("expected {}, found {}", tn(ty), tn(&t)),
                "",
            ));
        }
        Ok(t)
    }

    fn read_expr(&mut self, e: &mut Expr, scope: &Scope, c: bool, allowed: &[&str]) -> Res<GType> {
        let saved = self.read_context.take();
        if let Expr::Call(call) = e {
            if allowed.contains(&call.name.as_str()) {
                self.read_context = Some(call as *const Call as usize);
            }
        }
        let result = self.expr(e, scope, c);
        self.read_context = saved;
        result
    }

    fn text_expr(&mut self, e: &mut Expr, scope: &Scope, contract: bool) -> Res<GType> {
        let saved = self.borrowed_text.take();
        if let Expr::Call(call) = e {
            self.borrowed_text = Some(call as *const Call as usize);
        }
        let result = self.expr(e, scope, contract);
        self.borrowed_text = saved;
        result
    }

    fn expr(&mut self, e: &mut Expr, scope: &Scope, contract: bool) -> Res<GType> {
        self.expr_with_path(e, scope, contract, &[])
    }

    fn expr_with_path(&mut self, e: &mut Expr, scope: &Scope, contract: bool, path: &[String]) -> Res<GType> {
        let t = self.expr_inner(e, scope, contract, path)?;
        set_ty(e, &t);
        Ok(t)
    }

    fn expr_inner(&mut self, e: &mut Expr, scope: &Scope, c: bool, path: &[String]) -> Res<GType> {
        if self.ckind == "cost" && (matches!(e, Expr::Spent(_))
            || matches!(e, Expr::Var(v) if v.name == "result")
            || matches!(e, Expr::Call(v) if v.name == "old")) {
            self.s5_errors.push((self.cur_name.clone(), terr(e.line(), e.col(), "cost_expr", "cost cannot contain spent, result or old", "")));
            if let Expr::Call(call) = e {
                let mut types = Vec::new();
                for a in &mut call.args { types.push(self.expr(a, scope, c)?); }
                return Ok(types.into_iter().next().unwrap_or_else(i64t));
            }
            return Ok(if matches!(e, Expr::Var(_)) && self.cur_ret != unitt() { self.cur_ret.clone() } else { i64t() });
        }
        if let Expr::Spent(_) = e {
            if !self.cur_cost || !matches!(self.ckind, "invariant" | "assert") {
                self.s5_errors.push((self.cur_name.clone(), terr(e.line(), e.col(), "spent_context", "spent is only allowed in invariant/assert of a cost function", "")));
            }
            return Ok(i64t());
        }
        match e {
            Expr::Spent(_) => unreachable!("spent handled above"),
            Expr::IntLit(x) => {
                if !int_in_range(&x.value) {
                    return Err(terr(x.line, x.col, "int_range", "integer literal out of i64 range", ""));
                }
                Ok(i64t())
            }
            Expr::BoolLit(_) => Ok(boolt()),
            Expr::StrLit(_) => {
                if !c {
                    self.used_effects.insert("alloc".to_string());
                }
                Ok(strt())
            }
            Expr::Var(x) => {
                if x.name == "result" && !scope.contains_key("result") {
                    return Err(terr(x.line, x.col, "result_scope", "`result` is only allowed in ensures", ""));
                }
                Ok(self.lookup(x.line, x.col, &x.name, scope, path)?.ty)
            }
            Expr::Unary(x) => {
                if x.op == "*" {
                    let error = || terr(x.line, x.col, "type_mismatch",
                        "dereference requires a mutable scalar collection binder", "");
                    let Expr::Var(var) = x.operand.as_ref() else { return Err(error()); };
                    let binding = self.lookup(x.line, x.col, &var.name, scope, &[])?;
                    if binding.kind != "map_inout" || (binding.ty != i64t() && binding.ty != boolt()) {
                        return Err(error());
                    }
                    *e = Expr::Var(var.clone());
                    return Ok(binding.ty);
                }
                if x.op == "-" {
                    self.expect(&mut x.operand, &i64t(), scope, c, Some("unary minus"))?;
                    Ok(i64t())
                } else {
                    self.expect(&mut x.operand, &boolt(), scope, c, Some("a boolean operation"))?;
                    Ok(boolt())
                }
            }
            Expr::Binary(x) => {
                let op = x.op.clone();
                match op.as_str() {
                    "+" | "-" | "*" | "/" | "%" => {
                        self.expect(&mut x.left, &i64t(), scope, c, Some("arithmetic"))?;
                        self.expect(&mut x.right, &i64t(), scope, c, Some("arithmetic"))?;
                        Ok(i64t())
                    }
                    "<" | "<=" | ">" | ">=" => {
                        self.expect(&mut x.left, &i64t(), scope, c, Some("ordering"))?;
                        self.expect(&mut x.right, &i64t(), scope, c, Some("ordering"))?;
                        Ok(boolt())
                    }
                    "==" | "!=" => {
                        let lt = self.expr(&mut x.left, scope, c)?;
                        if lt.is_collection() {
                            // G6
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                "collections cannot be compared with ==",
                                "compare lengths and elements: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]",
                            ));
                        }
                        if lt.is_array() {
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                "arrays cannot be compared with ==",
                                "compare lengths and elements: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]",
                            ));
                        }
                        if lt == strt() {
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                "str values cannot be compared with ==",
                                "compare lengths and bytes: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]",
                            ));
                        }
                        if lt.owner.is_none() && dependencies(&lt).iter().any(|n| self.buffered.contains(n)) {
                            // G9 A0
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                format!("values of {} contain arrays or str and cannot be compared with ==", tn(&lt)),
                                "compare tags, lengths and indexed contents instead",
                            ));
                        }
                        if lt.owner.is_some() && !c {
                            return Err(terr(
                                x.line,
                                x.col,
                                "generic_operation",
                                format!("== is not allowed on the abstract type {} in code", lt.name),
                                "runtime equality only compares i64 or bool; == on a type parameter works in contracts",
                            ));
                        }
                        if lt != i64t() && lt != boolt() && !c {
                            return Err(terr(x.line, x.col, "eq_type", "== only compares i64 or bool in code", "use match"));
                        }
                        self.expect(&mut x.right, &lt, scope, c, None)?;
                        Ok(boolt())
                    }
                    "&&" | "||" => {
                        self.expect(&mut x.left, &boolt(), scope, c, Some("a boolean operation"))?;
                        self.expect(&mut x.right, &boolt(), scope, c, Some("a boolean operation"))?;
                        Ok(boolt())
                    }
                    "==>" => {
                        if !c {
                            return Err(terr(x.line, x.col, "contract_only", "==> is only allowed in contracts", ""));
                        }
                        self.expect(&mut x.left, &boolt(), scope, c, Some("a boolean operation"))?;
                        self.expect(&mut x.right, &boolt(), scope, c, Some("a boolean operation"))?;
                        Ok(boolt())
                    }
                    _ => Err(terr(x.line, x.col, "internal", "unknown expression", "")),
                }
            }
            Expr::Move(x) => {
                self.check_borrow(x.line, x.col, &x.name, &x.fields)?;
                if scope.get(&x.name).is_some_and(|entry| entry.kind == "map_iter") {
                    return Err(terr(x.line, x.col, "foreign_move", "a map iteration binder cannot be moved", ""));
                }
                if !x.fields.is_empty() {
                    let mut ty = self.lookup(x.line, x.col, &x.name, scope, &x.fields)?.ty;
                    for field in &x.fields {
                        if !self.is_struct(&ty) {
                            return Err(field_move_error(x));
                        }
                        ty = self.field_type(x.line, x.col, &ty, field)?;
                    }
                    if ty.is_array() {
                        return Err(terr(x.line, x.col, "array_alias", "an array field cannot be moved",
                            "use copy() to make an independent array"));
                    }
                    if self.field_move != Some((x.line, x.col)) {
                        return Err(field_move_error(x));
                    }
                    return Ok(ty);
                }
                // G6: liveness and position rules for binding moves are checked by foreign::check_ownership.
                if c {
                    return Err(terr(
                        x.line,
                        x.col,
                        "foreign_move",
                        "move only where ownership is consumed",
                        "contracts read parameters and locals directly; remove `move`",
                    ));
                }
                let entry = self.lookup(x.line, x.col, &x.name, scope, &[])?;
                if entry.ty.is_array() && !matches!(entry.kind, "local" | "take") {
                    if matches!(entry.kind, "param" | "inout") {
                        return Err(terr(x.line, x.col, "array_alias",
                            format!("parameter {} is a borrowed array and cannot be moved", x.name),
                            format!("declare `take {}: {}` or use copy({})", x.name, tn(&entry.ty), x.name)));
                    }
                    return Err(terr(x.line, x.col, "array_alias", "only an owned array binding can be moved",
                        "use copy() to make an independent array"));
                }
                Ok(entry.ty)
            }
            Expr::Index(x) => {
                let at = self.expect_array(&mut x.array, scope, c, true)?;
                if matches!(&*x.array, Expr::Move(_)) && at.is_array() {
                    return Err(terr(x.array.line(), x.array.col(), "array_alias", "an array element cannot be moved",
                        "read the element without move"));
                }
                if at.name == "hashmap" && at.owner.is_none() {
                    return Err(terr(
                        x.array.line(),
                        x.array.col(),
                        "type_mismatch",
                        format!("expected {}, found {}", tn(&GType::array()), tn(&at)),
                        "",
                    ));
                }
                self.expect(&mut x.index, &i64t(), scope, c, None)?;
                if at.is_collection() {
                    // G6: contract-only observation of a Vec / String
                    if !c {
                        return Err(terr(
                            x.line,
                            x.col,
                            "contract_only",
                            format!("indexing a {} is only allowed in contracts", tn(&at)),
                            if at.name == "vec" { "use vec_get in code" } else { "use string_byte in code" },
                        ));
                    }
                    if at.name == "vec" && at.args.first().is_some_and(|a| a.to_key() == "string") {
                        return Err(terr(x.line, x.col, "foreign_owned_type", "Vec<String> elements cannot be read", ""));
                    }
                    return Ok(if at.name == "vec" { at.args.first().cloned().unwrap_or_else(i64t) } else { i64t() });
                }
                // G2 A5: a bool array reads bool; G3: [T] reads T
                Ok(elem_type(&at))
            }
            Expr::Ctor(x) => self.ctor(x, scope, c),
            Expr::Quant(x) => {
                let agg = matches!(x.kind.as_str(), "sum" | "count" | "max" | "min");
                if !c {
                    if agg {
                        return Err(terr(
                            x.line,
                            x.col,
                            "contract_only",
                            format!("{} is only allowed in contracts", x.kind),
                            "compute it with a while loop and state it in ensures, e.g. ensures result == sum(k in 0..len(a): a[k])",
                        ));
                    }
                    return Err(terr(x.line, x.col, "contract_only", format!("{} is only allowed in contracts", x.kind), ""));
                }
                self.expect(&mut x.lo, &i64t(), scope, c, None)?;
                self.expect(&mut x.hi, &i64t(), scope, c, None)?;
                if scope.contains_key(&x.var) {
                    return Err(terr(x.line, x.col, "shadow", format!("{} is already defined", x.var), "use a new name"));
                }
                let mut sc = scope.clone();
                sc.insert(x.var.clone(), Entry { ty: i64t(), mutable: false, kind: "bound" });
                let boolean_body = matches!(x.kind.as_str(), "forall" | "exists" | "count");
                let body_ty = if boolean_body { boolt() } else { i64t() };
                let what = if boolean_body { "a quantifier body".to_string() } else { format!("an aggregate ({})", x.kind) };
                self.expect(&mut x.body, &body_ty, &sc, c, Some(what.as_str()))?;
                Ok(if agg { i64t() } else { boolt() })
            }
            Expr::StructLit(x) => self.struct_lit(x, scope, c),
            Expr::EnumCtor(x) => {
                let name = x.enum_name.clone();
                let targs = x.targs.clone();
                let mut ety = self.enum_type(x.line, x.col, &name)?;
                let tps = self.decls.get(&name).map(|d| d.tparams()).unwrap_or_default();
                let m = self.instantiate(x.line, x.col, &format!("enum {}", name), &decl_owner(&name), tps, &targs)?;
                if let Some(m) = &m {
                    let targs = targs.clone().unwrap_or_default();
                    x.inst = Some(inst_of(decl_owner(&name), &targs, m));
                    ety = mk_nominal(&name, targs);
                }
                self.variant_decl(x.line, x.col, &ety, &x.variant)?;
                let fields = self.fields_of(&ety, Some(x.variant.as_str()))?;
                if x.args.len() != fields.len() {
                    return Err(terr(
                        x.line,
                        x.col,
                        "arity",
                        format!("{}::{} takes {} argument(s), got {}", name, x.variant, fields.len(), x.args.len()),
                        "",
                    ));
                }
                for (arg, fd) in x.args.iter_mut().zip(fields.iter()) {
                    self.expect(arg, &fd.1, scope, c, None)?;
                    self.own_field(arg, &fd.1, scope, c)?; // G9 A0
                }
                self.ctor_allocs(&name, Some(x.variant.as_str()), c); // G9 A0
                Ok(ety)
            }
            Expr::FieldAccess(x) => {
                let bt = if !path.is_empty() {
                    self.expr_with_path(&mut x.base, scope, c, path)?
                } else {
                    let (root, mut fields) = field_place(&x.base);
                    if matches!(root, Expr::Var(_)) {
                        fields.push(x.field.clone());
                        self.expr_with_path(&mut x.base, scope, c, &fields)?
                    } else {
                        self.read_expr(&mut x.base, scope, c, &["vec_get"])?
                    }
                };
                if bt.owner.is_some() {
                    return Err(terr(
                        x.line,
                        x.col,
                        "generic_operation",
                        format!("cannot read field {}: {} is an abstract type parameter", x.field, tn(&bt)),
                        "a type parameter has no fields",
                    ));
                }
                if !self.is_struct(&bt) {
                    return Err(terr(
                        x.line,
                        x.col,
                        "not_struct",
                        format!("cannot read field {}: {} is not a struct", x.field, tn(&bt)),
                        if self.is_enum(&bt) { "enum payloads are read with match" } else { "" },
                    ));
                }
                self.field_type(x.line, x.col, &bt, &x.field)
            }
            Expr::IsVariant(x) => {
                if !c {
                    return Err(terr(x.line, x.col, "contract_only", "is_variant is only allowed in contracts", "use match in code"));
                }
                x.enum_ty = Some(self.variant_target(x.line, x.col, &mut x.value, &x.enum_name, &x.variant, scope, c)?);
                Ok(boolt())
            }
            Expr::VariantField(x) => {
                if !c {
                    return Err(terr(x.line, x.col, "contract_only", "variant_field is only allowed in contracts", "use match in code"));
                }
                let ety = self.variant_target(x.line, x.col, &mut x.value, &x.enum_name, &x.variant, scope, c)?;
                x.enum_ty = Some(ety.clone());
                for (fname, ft) in self.fields_of(&ety, Some(x.variant.as_str()))? {
                    if fname == x.field {
                        return Ok(ft);
                    }
                }
                Err(terr(
                    x.line,
                    x.col,
                    "unknown_field",
                    format!("variant {}::{} has no field {}", x.enum_name, x.variant, x.field),
                    "",
                ))
            }
            Expr::InoutArg(x) => {
                // G5 A0: valid only as a direct argument of a call (see inout_args)
                Err(terr(
                    x.line,
                    x.col,
                    "inout_argument",
                    "the `inout` marker is only valid on an argument of a function whose parameter is inout",
                    "remove `inout`",
                ))
            }
            Expr::Call(x) => {
                let t = self.call(x, scope, c)?;
                if let Some(xe) = self.x_pending.take() {
                    *e = xe; // P06: a contract call to a user function becomes its expansion, in place
                }
                Ok(t)
            }
        }
    }

    // ---- user type helpers ----
    /// [(field name, field type)] of the struct / enum variant `ty` names, with the type arguments of `ty` substituted
    /// into the declared field types (G3_SPEC 3.4).
    fn fields_of(&self, ty: &GType, variant: Option<&str>) -> Res<Vec<(String, GType)>> {
        let m = self.bind_decl_args(ty)?;
        let fs = self.tt.fields(&ty.name, variant).unwrap_or(&[]);
        Ok(fs.iter().map(|fd| (fd.name.clone(), substitute(&GType::from_key(&fd.ty), &m))).collect())
    }

    /// The substitution of a nominal application's type arguments for its declaration's parameters.
    fn bind_decl_args(&self, ty: &GType) -> Res<Mapping> {
        let n = self.decls.get(&ty.name).map(|d| d.tparams().len()).unwrap_or(0);
        match bind_args(&decl_owner(&ty.name), n, &ty.args) {
            Ok(m) => Ok(m),
            Err(e) => Err(terr(0, 0, "internal", e.to_string(), "")),
        }
    }

    /// The type of a struct field: find the field first, then substitute only its type (substituting every field made
    /// `x.f` O(fields)); a struct type without type arguments yields the declared type as is.
    fn field_type(&self, line: i64, col: i64, sty: &GType, name: &str) -> Res<GType> {
        let fs = self.tt.fields(&sty.name, None).unwrap_or(&[]);
        for fd in fs {
            if fd.name == name {
                let ft = GType::from_key(&fd.ty);
                if sty.args.is_empty() {
                    return Ok(ft);
                }
                return Ok(substitute(&ft, &self.bind_decl_args(sty)?));
            }
        }
        Err(terr(line, col, "unknown_field", format!("struct {} has no field {}", sty.name, name), ""))
    }

    /// The type of a named enum; unknown_type if missing, not_enum if it is not an enum.
    fn enum_type(&self, line: i64, col: i64, name: &str) -> Res<GType> {
        if !self.decls.contains_key(name) {
            return Err(terr(line, col, "unknown_type", format!("unknown type {}", name), ""));
        }
        if !self.tt.is_enum(name) {
            return Err(terr(line, col, "not_enum", format!("{} is not an enum", name), ""));
        }
        Ok(GType::new(name, Vec::new()))
    }

    fn variant_decl(&self, line: i64, col: i64, ety: &GType, variant: &str) -> Res<()> {
        match self.tt.variant(&ety.name, variant) {
            Some(_) => Ok(()),
            None => Err(terr(line, col, "unknown_variant", format!("enum {} has no variant {}", ety.name, variant), "")),
        }
    }

    /// Shared by is_variant / variant_field: checks the value, enum and variant.
    #[allow(clippy::too_many_arguments)]
    fn variant_target(
        &mut self,
        line: i64,
        col: i64,
        value: &mut Expr,
        enum_name: &str,
        variant: &str,
        scope: &Scope,
        c: bool,
    ) -> Res<GType> {
        let vt = self.expr(value, scope, c)?;
        let ety = self.enum_type(line, col, enum_name)?;
        self.variant_decl(line, col, &ety, variant)?;
        // G3: no explicit type arguments here; they come from the checked scrutinee, which keeps its full type
        if vt.owner.is_some() || vt.name != ety.name {
            return Err(terr(
                value.line(),
                value.col(),
                "type_mismatch",
                format!("expected {}, found {}", tn(&ety), tn(&vt)),
                "",
            ));
        }
        Ok(vt)
    }

    fn struct_lit(&mut self, e: &mut StructLit, scope: &Scope, c: bool) -> Res<GType> {
        let name = e.name.clone();
        let targs = e.targs.clone();
        if !self.decls.contains_key(&name) {
            return Err(terr(e.line, e.col, "unknown_type", format!("unknown type {}", name), ""));
        }
        if !self.tt.is_struct(&name) {
            return Err(terr(
                e.line,
                e.col,
                "not_struct",
                format!("{} is not a struct", name),
                format!("enum values are written {}::Variant", name),
            ));
        }
        let mut sty = GType::new(&name, Vec::new());
        let tps = self.decls.get(&name).map(|d| d.tparams()).unwrap_or_default();
        let m = self.instantiate(e.line, e.col, &format!("struct {}", name), &decl_owner(&name), tps, &targs)?;
        if let Some(m) = &m {
            let targs = targs.clone().unwrap_or_default();
            e.inst = Some(inst_of(decl_owner(&name), &targs, m));
            sty = mk_nominal(&name, targs);
        }
        let fields: Vec<(String, GType)> = self.fields_of(&sty, None)?;
        let declared: Vec<&str> = fields.iter().map(|f| f.0.as_str()).collect();
        for fi in &e.fields {
            if !declared.contains(&fi.name.as_str()) {
                return Err(terr(fi.line, fi.col, "unknown_field", format!("struct {} has no field {}", name, fi.name), ""));
            }
        }
        let given: Vec<&str> = e.fields.iter().map(|f| f.name.as_str()).collect();
        if given != declared {
            let uniq: BTreeSet<&str> = given.iter().cloned().collect();
            let why = if uniq.len() != given.len() {
                let dup = given.iter().find(|n| given.iter().filter(|m| m == n).count() > 1).cloned().unwrap_or("");
                format!("field {} is given twice", dup)
            } else if uniq != declared.iter().cloned().collect::<BTreeSet<&str>>() {
                let miss: Vec<&str> = declared.iter().filter(|n| !given.contains(*n)).cloned().collect();
                format!("missing field(s): {}", miss.join(", "))
            } else {
                format!("fields must be written in declaration order: {}", declared.join(", "))
            };
            return Err(terr(
                e.line,
                e.col,
                "struct_fields",
                format!("{} literal: {}", name, why),
                "write every field exactly once, in declaration order",
            ));
        }
        for (fi, fd) in e.fields.iter_mut().zip(fields.iter()) {
            self.expect(&mut fi.value, &fd.1, scope, c, None)?;
            self.own_field(&fi.value, &fd.1, scope, c)?; // G9 A0
        }
        self.ctor_allocs(&name, None, c); // G9 A0
        Ok(sty)
    }

    /// Some / None / Ok / Err. Without type arguments they keep the legacy i64 meaning (G3_SPEC 0.0 item 8);
    /// `Some::<X>(x)`, `None::<X>`, `Ok::<X>(x)`, `Err::<X>(code)` build Option<X> / Result<X> (3.5).
    fn ctor(&mut self, e: &mut Ctor, scope: &Scope, c: bool) -> Res<GType> {
        let name = e.name.clone();
        let targs = match e.targs.clone() {
            None => {
                if name == "None" {
                    return Ok(GType::option());
                }
                if let Some(a) = e.arg.as_mut() {
                    self.expect(a, &i64t(), scope, c, None)?;
                    if !c {
                        self.own(a, scope)?;
                    }
                }
                return Ok(if name == "Some" { GType::option() } else { GType::result() });
            }
            Some(t) => t,
        };
        if targs.len() != 1 {
            return Err(terr(
                e.line,
                e.col,
                "type_arity",
                format!("{}::<..> takes exactly 1 type argument, got {}", name, targs.len()),
                format!("write {}::<T>", name),
            ));
        }
        let x = targs[0].clone();
        self.check_type(e.line, e.col, &x)?;
        self.check_arg(e.line, e.col, &x, None)?;
        self.ext_misuse(e.line, e.col, &x, Some("arg"))?; // G9 A0
        let owner = format!("builtin:{}", name);
        let mut m = Mapping::new();
        m.insert((owner.clone(), 0), x.clone());
        e.inst = Some(inst_of(owner, &targs, &m));
        if name == "None" {
            return Ok(mk_option(x));
        }
        let want = if name == "Err" { i64t() } else { x.clone() };
        if let Some(a) = e.arg.as_mut() {
            self.expect(a, &want, scope, c, None)?;
            if !c {
                self.own(a, scope)?;
            }
        }
        Ok(if name == "Some" { mk_option(x) } else { mk_result(x) })
    }

    /// map_lookup / map_updated arguments: a key whose type differs from a map argument's key type is a
    /// foreign_contract error; any other mismatch is the ordinary type_mismatch.
    fn check_text_key(&mut self, arg: &mut Expr, scope: &Scope, c: bool, actual: Option<GType>) -> Res<()> {
        if let Expr::StrLit(lit) = arg {
            lit.ty = Some("str".into());
            return Ok(());
        }
        let actual = match actual {
            Some(ty) => ty,
            None => self.text_expr(arg, scope, c)?,
        };
        if actual == strt() || actual.to_key() == "string" && matches!(arg, Expr::Var(_) | Expr::FieldAccess(_) | Expr::VariantField(_)) {
            return Ok(());
        }
        Err(terr(arg.line(), arg.col(), "type_mismatch",
            format!("expected str or a borrowed String read, found {}", tn(&actual)), ""))
    }

    fn observer_args(&mut self, e: &mut Call, cf: &CatFn, m: &Mapping, scope: &Scope, c: bool) -> Res<()> {
        let mut got: Vec<GType> = Vec::new();
        for arg in e.args.iter_mut() {
            got.push(self.expr(arg, scope, c)?);
        }
        let kidx = cf.params.iter().position(|p| p.name == "k").unwrap_or(0);
        for g in &got {
            if g.name == "hashmap" && g.owner.is_none() {
                if let (Some(k0), Some(kt)) = (g.args.first(), got.get(kidx)) {
                    if k0 != kt && !(e.name == "map_lookup" && k0.to_key() == "string") {
                        return Err(terr(
                            e.args[kidx].line(),
                            e.args[kidx].col(),
                            "foreign_contract",
                            format!("key of type {} does not match the map key type {}", tn(kt), tn(k0)),
                            "pass a key of the map's key type",
                        ));
                    }
                }
            }
        }
        for ((arg, g), prm) in e.args.iter_mut().zip(got.iter()).zip(cf.params.iter()) {
            let want = substitute(&prm.ty, m);
            if e.name == "map_lookup" && prm.name == "k" && want.to_key() == "string" {
                self.check_text_key(arg, scope, c, Some(g.clone()))?;
            } else if *g != want {
                return Err(terr(
                    arg.line(),
                    arg.col(),
                    "type_mismatch",
                    format!("expected {}, found {}", tn(&want), tn(g)),
                    "",
                ));
            }
        }
        Ok(())
    }

    // ---- G5 A0: inout call arguments and old(..) ----
    /// Arguments of a resolved call to a user function whose signature has an inout parameter (or that carries a marker):
    /// per argument marker/place validity and the instantiated type, then exclusive use of every marked binding across the
    /// whole argument list.
    fn inout_args(&mut self, e: &mut Call, f: &FuncInfo, m: &Mapping, scope: &Scope, c: bool) -> Res<()> {
        let n = e.name.clone();
        let user_call = self.funcs.contains_key(&n);
        let collection_place = !user_call && matches!(n.as_str(), "map_get_mut" | "vec_at_mut" | "map_remove" | "map_take" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice");
        let mut marked = Vec::new();
        for i in 0..e.args.len().min(f.params.len()) {
            let want = substitute(&f.params[i], m);
            let pname = f.pnames[i].clone();
            if !f.inout[i] {
                if let Expr::InoutArg(a) = &e.args[i] {
                    return Err(terr(
                        a.line,
                        a.col,
                        "inout_argument",
                        format!("parameter {} of {} is not inout: the `inout` marker is not allowed here", pname, n),
                        format!("remove `inout`, or declare the parameter `inout {}: ...`", pname),
                    ));
                }
                if collection_place && want.to_key() == "string" {
                    self.check_text_key(&mut e.args[i], scope, c, None)?;
                } else if user_call {
                    self.expect_user_arg(&mut e.args[i], &want, scope, c, !f.foreign && !self.takes[&n][i])?;
                } else {
                    self.expect(&mut e.args[i], &want, scope, c, None)?;
                }
                continue;
            }
            let (aline, acol) = (e.args[i].line(), e.args[i].col());
            let arg: &mut InoutArg = match &mut e.args[i] {
                Expr::InoutArg(a) => a,
                _ => {
                    return Err(terr(
                        aline,
                        acol,
                        "inout_argument",
                        format!("parameter {} of {} is inout: the argument needs the `inout` marker", pname, n),
                        "write `inout name` with a `let mut` local array or an inout parameter",
                    ))
                }
            };
            let (place, path) = field_place(&arg.expr);
            if let Expr::Var(op) = place {
                self.check_iteration_borrow(aline, acol, &op.name, &path)?;
            }
            let what: Option<&str> = match &*arg.expr {
                Expr::Var(_) => None,
                Expr::FieldAccess(_) if (user_call || collection_place) && place_root(&arg.expr).is_some() => None,
                Expr::FieldAccess(_) => Some("a field"),
                Expr::Index(_) => Some("an element"),
                Expr::Call(_) => Some("a call"),
                _ => Some("an expression"),
            };
            if let Some(what) = what {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!("an inout argument must be the whole name of a local array or an inout parameter, not {}", what),
                    "name a `let mut` local array or an inout parameter of this function",
                ));
            }
            let Expr::Var(op) = place else { return Err(terr(aline, acol, "internal", "unknown expression", "")) };
            let name = op.name.clone();
            let v = self.lookup(op.line, op.col, &name, scope, &path)?;
            if v.kind == "local" && !v.mutable {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!("{} is not mutable: it cannot be passed inout", op.name),
                    "declare it with `let mut`",
                ));
            }
            if !matches!(v.kind, "inout" | "local" | "map_inout") {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!(
                        "{} is a read-only {}: it cannot be passed inout",
                        op.name,
                        if v.kind == "param" { "parameter" } else { "binding" }
                    ),
                    format!("declare the parameter `inout {}: ...`, or copy into a `let mut` local", op.name),
                ));
            }
            let actual = if matches!(&*arg.expr, Expr::Var(_)) { v.ty } else { self.expr(&mut arg.expr, scope, c)? };
            if actual != want {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!(
                        "inout argument {} has type {} but parameter {} of {} expects {}",
                        name,
                        tn(&actual),
                        pname,
                        n,
                        tn(&want)
                    ),
                    "pass an array of exactly the parameter's element type",
                ));
            }
            let take = !user_call && n == "map_take";
            let root_held = self.active_roots.iter().any(|(root, map_read)| {
                *root == name && (!take || !map_read)
            });
            let read_held = take && self.map_read_paths.iter().any(|(root, held)| {
                *root == name && paths_overlap(&path, held)
            });
            if root_held || read_held || self.match_path_held(&name, &path) {
                // G9: mutable access while a match view of the root is active
                return Err(terr(
                    aline,
                    acol,
                    "recursive_borrow",
                    format!("cannot pass {} inout while a match on it is active", name),
                    "finish the match before the call",
                ));
            }
            if let Expr::Var(op) = &mut *arg.expr { op.ty = Some(actual.to_key()); }
            arg.ty = Some(actual.to_key());
            marked.push((i, name, if user_call { path } else { Vec::new() }));
        }
        for (i, name, path) in &marked {
            let mut others = Vec::new();
            for (j, arg) in e.args.iter().enumerate() {
                if j != *i {
                    place_mentions(arg, &mut others);
                }
            }
            if others.iter().any(|(other, p)| *other == name && paths_overlap(path, p)) {
                return Err(terr(
                    e.args[*i].line(),
                    e.args[*i].col(),
                    "inout_alias",
                    format!("{} is passed inout and also used elsewhere in the arguments of this call", name),
                    format!("compute the other value first, e.g. `let n: i64 = len({});`, then pass n", name),
                ));
            }
        }
        if !c && self.ckind == "par" && self.boundary != Some(&*e as *const Call as usize) {
            return Err(terr(
                e.line,
                e.col,
                "inout_context",
                "a par branch cannot contain a nested inout call",
                "evaluate the call in a separate statement before the par group",
            ));
        }
        Ok(())
    }

    /// P2 batch 4: load_be(a: [u8], i, n) -> i64 (code and contracts) and the statement-only store_be(inout a, i, w, n);
    /// n is an integer literal 1..=8 (check.py word_call).
    fn word_call(&mut self, e: &mut Call, scope: &Scope, c: bool) -> Res<GType> {
        let n = e.name.clone();
        let (line, col) = (e.line, e.col);
        let store = n == "store_be";
        let k = if store { 4 } else { 3 };
        if e.args.len() != k {
            return Err(terr(line, col, "arity", format!("{} takes {} argument(s), got {}", n, k, e.args.len()), ""));
        }
        if store {
            if c {
                return Err(terr(line, col, "effect_in_contract", "store_be not allowed in contracts", ""));
            }
            let (aline, acol) = (e.args[0].line(), e.args[0].col());
            let name: String;
            {
                let arg: &mut InoutArg = match &mut e.args[0] {
                    Expr::InoutArg(a) => a,
                    _ => {
                        return Err(terr(
                            aline,
                            acol,
                            "inout_argument",
                            "the array of store_be needs the `inout` marker",
                            "write `store_be(inout name, i, w, n)` with a `let mut` local array or an inout parameter",
                        ))
                    }
                };
                let op: &mut Var = match &mut *arg.expr {
                    Expr::Var(v) => v,
                    _ => {
                        return Err(terr(
                            aline,
                            acol,
                            "inout_argument",
                            "an inout argument must be the whole name of a local array or an inout parameter",
                            "name a `let mut` local array or an inout parameter of this function",
                        ))
                    }
                };
                let v = self.lookup(op.line, op.col, &op.name, scope, &[])?;
                if v.kind == "local" && !v.mutable {
                    return Err(terr(
                        aline,
                        acol,
                        "inout_argument",
                        format!("{} is not mutable: it cannot be passed inout", op.name),
                        "declare it with `let mut`",
                    ));
                }
                if v.kind != "inout" && v.kind != "local" {
                    return Err(terr(
                        aline,
                        acol,
                        "inout_argument",
                        format!(
                            "{} is a read-only {}: it cannot be passed inout",
                            op.name,
                            if v.kind == "param" { "parameter" } else { "binding" }
                        ),
                        format!("declare the parameter `inout {}: ...`, or copy into a `let mut` local", op.name),
                    ));
                }
                if v.ty != GType::array_u8() {
                    return Err(terr(
                        aline,
                        acol,
                        "type_mismatch",
                        format!("expected {}, found {}", tn(&GType::array_u8()), tn(&v.ty)),
                        "",
                    ));
                }
                if self.active_roots.iter().any(|(root, _)| *root == op.name) || self.match_path_held(&op.name, &[]) {
                    return Err(terr(
                        aline,
                        acol,
                        "recursive_borrow",
                        format!("cannot pass {} inout while a match on it is active", op.name),
                        "finish the match before the call",
                    ));
                }
                op.ty = Some(v.ty.to_key());
                arg.ty = Some(v.ty.to_key());
                name = op.name.clone();
            }
            self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
            self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
            if e.args[1..].iter().any(|x| mentions(x, &name)) {
                return Err(terr(
                    aline,
                    acol,
                    "inout_alias",
                    format!("{} is passed inout and also used elsewhere in the arguments of this call", name),
                    format!("compute the other value first, e.g. `let n: i64 = len({});`, then pass n", name),
                ));
            }
            if self.boundary != Some(&*e as *const Call as usize) {
                return Err(terr(
                    line,
                    col,
                    "inout_context",
                    "store_be must be a whole expression statement",
                    "write `store_be(inout a, i, w, n);` as a statement",
                ));
            }
        } else {
            self.expect(&mut e.args[0], &GType::array_u8(), scope, c, None)?;
            self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
        }
        let wi = e.args.len() - 1;
        if word_width(&e.args[wi]).is_none() {
            let (wl, wc) = (e.args[wi].line(), e.args[wi].col());
            return Err(terr(
                wl,
                wc,
                "type_mismatch",
                format!("the width argument of {} must be an integer literal 1..=8", n),
                "write the byte count as a literal such as 8",
            ));
        }
        if let Expr::IntLit(l) = &mut e.args[wi] {
            l.ty = Some(i64t().to_key());
        }
        Ok(if store { unitt() } else { i64t() })
    }

    /// R-5-1 and R-5-2: range copy statements with an inout integer array destination.
    fn copy_range_call(&mut self, e: &mut Call, scope: &Scope, c: bool) -> Res<GType> {
        let (line, col) = (e.line, e.col);
        let within = e.name == "copy_within";
        let builtin = e.name.as_str();
        let arity = if within { 4 } else { 5 };
        if c {
            return Err(terr(line, col, "effect_in_contract", format!("{builtin} is not allowed in contracts"), ""));
        }
        if e.args.len() != arity {
            return Err(terr(line, col, "arity", format!("{builtin} takes {arity} argument(s), got {}", e.args.len()), ""));
        }
        let (aline, acol) = (e.args[0].line(), e.args[0].col());
        let name: String;
        let dty: GType;
        {
            let arg: &mut InoutArg = match &mut e.args[0] {
                Expr::InoutArg(a) => a,
                _ => {
                    return Err(terr(
                        aline,
                        acol,
                        "inout_argument",
                        format!("the destination array of {builtin} needs the `inout` marker"),
                        if within { "write `copy_within(inout name, dst, src, n)` with a `let mut` local array or an inout parameter" }
                        else { "write `copy_range(inout name, i, src, j, n)` with a `let mut` local array or an inout parameter" },
                    ))
                }
            };
            let op: &mut Var = match &mut *arg.expr {
                Expr::Var(v) => v,
                _ => {
                    return Err(terr(
                        aline,
                        acol,
                        "inout_argument",
                        "an inout argument must be the whole name of a local array or an inout parameter",
                        "name a `let mut` local array or an inout parameter of this function",
                    ))
                }
            };
            let v = self.lookup(op.line, op.col, &op.name, scope, &[])?;
            if v.kind == "local" && !v.mutable {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!("{} is not mutable: it cannot be passed inout", op.name),
                    "declare it with `let mut`",
                ));
            }
            if v.kind != "inout" && v.kind != "local" {
                return Err(terr(
                    aline,
                    acol,
                    "inout_argument",
                    format!(
                        "{} is a read-only {}: it cannot be passed inout",
                        op.name,
                        if v.kind == "param" { "parameter" } else { "binding" }
                    ),
                    format!("declare the parameter `inout {}: ...`, or copy into a `let mut` local", op.name),
                ));
            }
            if v.ty != GType::array() && v.ty != GType::array_u8() {
                return Err(terr(
                    aline,
                    acol,
                    "type_mismatch",
                    format!("expected {} or {}, found {}", tn(&GType::array()), tn(&GType::array_u8()), tn(&v.ty)),
                    "",
                ));
            }
            if self.active_roots.iter().any(|(root, _)| *root == op.name) || self.match_path_held(&op.name, &[]) {
                return Err(terr(
                    aline,
                    acol,
                    "recursive_borrow",
                    format!("cannot pass {} inout while a match on it is active", op.name),
                    "finish the match before the call",
                ));
            }
            op.ty = Some(v.ty.to_key());
            arg.ty = Some(v.ty.to_key());
            name = op.name.clone();
            dty = v.ty.clone();
        }
        self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
        self.expect(&mut e.args[2], &if within { i64t() } else { dty }, scope, c, None)?;
        self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
        if !within {
            self.expect(&mut e.args[4], &i64t(), scope, c, None)?;
        }
        if e.args[1..].iter().any(|x| mentions(x, &name)) {
            return Err(terr(
                aline,
                acol,
                "inout_alias",
                format!("{} is passed inout and also used elsewhere in the arguments of this call", name),
                if within { "compute the index arguments before the call".to_string() }
                else { format!("copy the source first with `let mut t: [..] = copy({});` (needs alloc), then pass t", name) },
            ));
        }
        if self.boundary != Some(&*e as *const Call as usize) {
            return Err(terr(
                line,
                col,
                "inout_context",
                format!("{builtin} must be a whole expression statement"),
                if within { "write `copy_within(inout a, dst, src, n);` as a statement" }
                else { "write `copy_range(inout dst, i, src, j, n);` as a statement" },
            ));
        }
        Ok(unitt())
    }

    /// The contextual builtin old(a): context first, then arity and operand.
    fn old_call(&mut self, e: &mut Call, scope: &Scope) -> Res<GType> {
        let (line, col) = (e.line, e.col);
        if !matches!(self.ckind, "ensures" | "invariant" | "assert") {
            return Err(terr(
                line,
                col,
                "old_context",
                "old(..) is only allowed in ensures, loop invariants and asserts",
                "state the property in a contract clause; in code use copy(a) (needs alloc) if the entry value is needed",
            ));
        }
        if e.args.len() != 1 {
            return Err(terr(
                line,
                col,
                "old_argument",
                format!("old takes exactly 1 argument, got {}", e.args.len()),
                "write old(a) with the name of an inout parameter of this function",
            ));
        }
        let entry: Option<Entry> = match &e.args[0] {
            Expr::Var(v) => scope.get(&v.name).filter(|x| x.kind == "inout" && x.ty.is_array()).cloned(),
            _ => None,
        };
        let entry = match entry {
            Some(x) => x,
            None => {
                return Err(terr(
                    line,
                    col,
                    "old_argument",
                    "the operand of old must be the name of an inout parameter of this function",
                    "write old(a) where `inout a: [..]` is a parameter of the enclosing function",
                ))
            }
        };
        let mut name = String::new();
        if let Expr::Var(v) = &mut e.args[0] {
            v.ty = Some(entry.ty.to_key());
            name = v.name.clone();
        }
        let index = self.cur_params.iter().position(|p| *p == name).unwrap_or(0);
        e.old = Some((name, index));
        Ok(entry.ty)
    }

    // ---- P06: expanding user functions in contracts (docs/L3_RESTRUCTURE_SPEC.md appendix P06) ----
    /// The reason each function cannot be expanded in a contract, or a raw copy of its body when its signature and
    /// statements allow it (checked before any body is typed, so every template starts from source).
    fn x_prepare(&mut self, prog: &Program) -> Res<()> {
        for f in &prog.functions {
            let r = x_sig_reason(f, &self.decls).or_else(|| x_scan_block(f, &f.body, &self.funcs));
            if f.pred {
                if let Some(reason) = &r {
                    return Err(terr(f.line, f.col, "pred_signature",
                        format!("pred {} must expand in a contract: {}", f.name, reason), X_FIX));
                }
            }
            let not_bool = GType::from_key(&f.ret) != boolt();
            if f.opaque && (r.is_some() || not_bool) {
                // P13o: an opaque function is an expandable predicate
                let why = match &r {
                    None => "must return bool".to_string(),
                    Some(r) => format!("must expand in a contract: {}", r),
                };
                return Err(terr(
                    f.line,
                    f.col,
                    "opaque_signature",
                    format!("opaque function {} {}", f.name, why),
                    format!("keep `opaque` for a bool predicate that a contract may expand ({})", X_FIX),
                ));
            }
            self.x_raw.insert(f.name.clone(), match r {
                Some(r) => Err(r),
                None => Ok(f.body.clone()),
            });
        }
        Ok(())
    }

    fn x_template(&mut self, line: i64, col: i64, n: &str, f: &FuncInfo) -> Res<Expr> {
        if let Some(t) = self.x_memo.get(n) {
            return Ok(t.clone());
        }
        let mut raw = match self.x_raw.get(n) {
            Some(Ok(b)) => b.clone(),
            Some(Err(r)) => return Err(terr(line, col, "call_in_contract", format!("function {} cannot be expanded in a contract: {}", n, r), X_FIX)),
            None => return Err(terr(line, col, "call_in_contract", format!("function {} cannot be expanded in a contract: it has no body", n), X_FIX)),
        };
        if self.x_busy.contains(n) {
            return Err(terr(line, col, "call_in_contract", format!("function {} cannot be expanded in a contract: it is recursive", n), X_FIX));
        }
        self.x_busy.insert(n.to_string());
        let scope: Scope = f.pnames.iter().zip(f.params.iter())
            .map(|(p, t)| (p.clone(), Entry { ty: t.clone(), mutable: false, kind: "param" })).collect();
        let t = self.x_walk(line, col, n, f, &mut raw, scope, HashMap::new())?;
        self.x_busy.remove(n);
        self.x_memo.insert(n.to_string(), t.clone());
        Ok(t)
    }

    /// Type the statements in contract mode and fold them into one expression over the parameters.
    #[allow(clippy::too_many_arguments)]
    fn x_walk(&mut self, sl: i64, sc: i64, n: &str, f: &FuncInfo, stmts: &mut [Stmt], scope: Scope,
              env: HashMap<String, Expr>) -> Res<Expr> {
        let (mut scope, mut env) = (scope, env);
        let mut i = 0;
        loop {
            let (head, rest) = stmts.split_at_mut(i + 1);
            match &mut head[i] {
                Stmt::Let(st) => {
                    if scope.contains_key(&st.name) {
                        return Err(terr(st.line, st.col, "shadow",
                            format!("{} is already defined; shadowing is not allowed", st.name), "use a new name"));
                    }
                    let ty = GType::from_key(&st.ty);
                    self.expect(&mut st.value, &ty, &scope, true, None)?;
                    let v = x_subst(&st.value, &env);
                    x_sized(sl, sc, n, &v)?;
                    env.insert(st.name.clone(), v);
                    scope.insert(st.name.clone(), Entry { ty, mutable: false, kind: "local" });
                }
                Stmt::Return(st) => {
                    let v = st.value.as_mut().expect("x_scan_block: a return has a value");
                    self.expect(v, &f.ret, &scope, true, None)?;
                    let r = x_subst(v, &env);
                    x_sized(sl, sc, n, &r)?;
                    return Ok(r);
                }
                Stmt::If(st) => {
                    self.expect(&mut st.cond, &boolt(), &scope, true, Some("a condition"))?;
                    let cond = x_subst(&st.cond, &env);
                    x_sized(sl, sc, n, &cond)?;
                    let t = self.x_walk(sl, sc, n, f, &mut st.then, scope.clone(), env.clone())?;
                    let el = if !st.els.is_empty() {
                        self.x_walk(sl, sc, n, f, &mut st.els, scope.clone(), env.clone())?
                    } else {
                        self.x_walk(sl, sc, n, f, rest, scope.clone(), env.clone())?
                    };
                    let j = x_join(st.line, st.col, cond, t, el);
                    x_sized(sl, sc, n, &j)?;
                    return Ok(j);
                }
                _ => unreachable!("x_scan_block admits only let, if and return"),
            }
            i += 1;
        }
    }

    fn call(&mut self, e: &mut Call, scope: &Scope, c: bool) -> Res<GType> {
        let n = e.name.clone();
        let targs = e.targs.clone();
        let (line, col) = (e.line, e.col);
        let alen = e.args.len();
        let arity = |k: usize| -> Res<()> {
            if alen != k {
                return Err(terr(line, col, "arity", format!("{} takes {} argument(s), got {}", n, k, alen), ""));
            }
            Ok(())
        };
        let is_builtin = matches!(n.as_str(), "len" | "alloc" | "copy" | "print" | "is_char_boundary" | "substr" | "truncate" | "eq_range" | "resize" | "copy_range" | "copy_within")
            || is_bit_builtin(&n)
            || is_word_builtin(&n);
        let is_old = n == "old" && self.uses_inout && !self.funcs.contains_key(&n); // G5 A0: contextual builtin
        if targs.is_some() && !self.funcs.contains_key(&n) && (is_builtin || contract_only(&n).is_some() || is_old) {
            return Err(terr(line, col, "type_arity", format!("builtin {} takes no type arguments", n), "remove the `::<...>`"));
        }
        if is_old {
            return self.old_call(e, scope);
        }
        match n.as_str() {
            "len" => {
                arity(1)?;
                let ty = self.text_expr(&mut e.args[0], scope, c)?;
                self.check_array_type(&e.args[0], true, ty)?;
                return Ok(i64t());
            }
            "alloc" => {
                let want = self.alloc_want.take();
                if c {
                    return Err(terr(line, col, "effect_in_contract", "alloc not allowed in contracts", ""));
                }
                arity(2)?;
                self.expect(&mut e.args[0], &i64t(), scope, c, None)?;
                // G2 A5: the fill value is bool when the hinted type is [bool]; G3: T when it is [T]
                let vwant = match &want {
                    Some(h) => elem_type(h),
                    None => i64t(),
                };
                self.expect(&mut e.args[1], &vwant, scope, c, None)?;
                self.used_effects.insert("alloc".to_string());
                return Ok(want.unwrap_or_else(GType::array));
            }
            "copy" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "copy not allowed in contracts", ""));
                }
                arity(1)?;
                let at = self.expect_array(&mut e.args[0], scope, c, false)?;
                self.used_effects.insert("alloc".to_string());
                return Ok(at);
            }
            // the first m elements of an array as a new array (moves the array at its last use)
            "truncate" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "truncate not allowed in contracts", ""));
                }
                arity(2)?;
                let at = self.expect_array(&mut e.args[0], scope, c, false)?;
                if at == strt() {
                    let (l0, c0) = (e.args[0].line(), e.args[0].col());
                    return Err(terr(l0, c0, "type_mismatch", "truncate takes an array, found str", "use substr(s, 0, m) for a str"));
                }
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.used_effects.insert("alloc".to_string());
                return Ok(at);
            }
            // P05G: a copy of a grown to length n, the new tail set to v (no prefix loop, one allocation)
            "resize" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "resize not allowed in contracts", ""));
                }
                arity(3)?;
                let at = self.expect_array(&mut e.args[0], scope, c, false)?;
                let (l0, c0) = (e.args[0].line(), e.args[0].col());
                if at == strt() {
                    return Err(terr(l0, c0, "type_mismatch", "resize takes an array, found str", "build a String with push for a str"));
                }
                if !at.args.is_empty() {
                    return Err(terr(l0, c0, "type_mismatch",
                        format!("resize takes [i64], [u8] or [bool], found {}", tn(&at)),
                        "use alloc and a copy loop for other element types"));
                }
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &elem_type(&at), scope, c, None)?;
                self.used_effects.insert("alloc".to_string());
                return Ok(at);
            }
            "is_char_boundary" => {
                arity(2)?;
                self.expect(&mut e.args[0], &strt(), scope, c, None)?;
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                return Ok(boolt());
            }
            // P2: pure i64 bit operation, allowed in code and contracts
            _ if is_bit_builtin(&n) => {
                arity(2)?;
                self.expect(&mut e.args[0], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                return Ok(i64t());
            }
            // P2 batch 4: load_be / store_be
            _ if is_word_builtin(&n) => return self.word_call(e, scope, c),
            // L-copy-range: copy_range(inout dst: [T], i, src: [T], j, n), a statement, T an integer element type
            "copy_range" | "copy_within" => return self.copy_range_call(e, scope, c),
            // L3-10: eq_range(a: [T], i, b: [T], j, n) -> bool, code only, T an integer element type
            "eq_range" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "eq_range is not allowed in contracts", ""));
                }
                arity(5)?;
                let at = self.expect_array(&mut e.args[0], scope, c, false)?;
                // check.py: a must be [i64] or [u8]; b must have exactly a's type
                if !(at.is_array() && matches!(at.array_elem(), Some(el) if el == i64t() || el == GType::u8())) {
                    let (l0, c0) = (e.args[0].line(), e.args[0].col());
                    return Err(terr(l0, c0, "type_mismatch", format!("expected [i64] or [u8], found {}", tn(&at)), ""));
                }
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &at, scope, c, None)?;
                self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[4], &i64t(), scope, c, None)?;
                return Ok(boolt());
            }
            "substr" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "substr not allowed in contracts", ""));
                }
                arity(3)?;
                self.expect(&mut e.args[0], &strt(), scope, c, None)?;
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
                self.used_effects.insert("alloc".to_string());
                return Ok(strt());
            }
            "print" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "print not allowed in contracts", ""));
                }
                arity(1)?;
                let t = self.expr(&mut e.args[0], scope, c)?;
                if t.owner.is_some() {
                    return Err(terr(
                        line,
                        col,
                        "generic_operation",
                        format!("print cannot take the abstract type {}", t.name),
                        "print takes a concrete i64 or bool",
                    ));
                }
                if t != i64t() && t != boolt() {
                    return Err(terr(line, col, "print_type", "print takes i64 or bool", ""));
                }
                self.used_effects.insert("io".to_string());
                return Ok(unitt());
            }
            _ => {}
        }
        if let Some((kind, rt)) = contract_only(&n) {
            if !c {
                return Err(terr(
                    line,
                    col,
                    "contract_only",
                    format!("{} is only allowed in contracts", n),
                    if n == "size" { "size is a contract measure; use len or a counter in code" } else { "use match in code" },
                ));
            }
            arity(1)?;
            let t = self.expr(&mut e.args[0], scope, c)?;
            if n == "size" {
                // G9 A0: a non-generic user struct/enum value
                let generic = self.decls.get(&t.name).map(|d| !d.tparams().is_empty()).unwrap_or(false);
                if !(self.is_plain_nominal(&t) && !generic) {
                    return Err(terr(
                        line,
                        col,
                        "size_type",
                        format!("size needs a non-generic struct or enum value, got {}", tn(&t)),
                        "size counts constructors; use len for arrays and strings",
                    ));
                }
                return Ok(i64t());
            }
            if t.owner.is_some() || t.name != kind {
                let want = if kind == "option" { GType::option() } else { GType::result() };
                return Err(terr(
                    e.args[0].line(),
                    e.args[0].col(),
                    "type_mismatch",
                    format!("expected {}, found {}", tn(&want), tn(&t)),
                    "",
                ));
            }
            return Ok(match rt {
                CRes::Bool => boolt(),
                CRes::I64 => i64t(),
                // G3: the payload type of Option<X> / Result<X>
                CRes::Payload => {
                    if t.args.is_empty() {
                        i64t()
                    } else {
                        t.args[0].clone()
                    }
                }
            });
        }
        // G6: fixed standard catalog (G6_SPEC 10.3)
        let cat: Option<CatFn> = if self.funcs.contains_key(&n) { None } else { catalog_fn(&n) };
        let call_site = Some(e as *const Call as usize);
        if cat.is_some() && n == "map_take" && self.read_context != call_site {
            return Err(terr(line, col, "foreign_contract", "map_take is only allowed as a direct match scrutinee", ""));
        }
        if cat.is_some() && matches!(n.as_str(), "vec_clear" | "vec_reserve" | "vec_extend_from_slice") && !c && self.statement_call != call_site {
            return Err(terr(line, col, "inout_context", format!("{n} must be a whole expression statement"), ""));
        }
        let f: Rc<FuncInfo> = if let Some(cf) = &cat {
            if is_catalog_contract_only(&n) {
                if !c {
                    return Err(terr(
                        line,
                        col,
                        "foreign_contract",
                        format!("{} is only allowed in contracts", n),
                        if n == "map_lookup" { "use map_get in code" } else { "state the update in ensures" },
                    ));
                }
            } else if c && !is_file_observer(&n) && !is_proc_observer(&n) && !is_net_observer(&n) && !is_async_observer(&n) {
                return Err(terr(
                    line,
                    col,
                    "foreign_contract",
                    format!("{} cannot be called in a contract", n),
                    "contracts observe collections with len(x), x[i], map_lookup and map_updated",
                ));
            }
            Rc::new(FuncInfo {
                src_name: n.clone(),
                line: 0,
                col: 0,
                params: cf.params.iter().map(|p| p.ty.clone()).collect(),
                pnames: cf.params.iter().map(|p| p.name.to_string()).collect(),
                inout: (0..cf.params.len()).map(|i| i == 0 && matches!(n.as_str(), "map_get_mut" | "vec_at_mut" | "map_remove" | "map_take" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice")).collect(),
                ret: cf.ret.clone(),
                effects: cf.effects.clone(),
                tparams: cf.tparams.clone(),
                foreign: false,
                opaque: false,
                pred: false,
            })
        } else {
            match self.funcs.get(&n) {
                Some(f) => {
                    if c && f.foreign {
                        return Err(terr(
                            line,
                            col,
                            "foreign_contract",
                            format!("trusted rust fn {} cannot be called in a contract", n),
                            "state the property with the arguments' own contents instead",
                        ));
                    }
                    Rc::clone(f)
                }
                None => {
                    let fix = if matches!(n.as_str(), "sum" | "count" | "max" | "min") {
                        "aggregates use the form sum(k in 0..len(a): a[k])"
                    } else {
                        ""
                    };
                    return Err(terr(line, col, "unknown_fn", format!("unknown function {}", n), fix));
                }
            }
        };
        // G3_SPEC 3.4: resolve + arity + admissibility + bounds, substitute the signature, check the arguments
        if cat.is_some() && n == "map_take" {
            if let Some([key, value]) = targs.as_deref() {
                if *key != i64t() && key.to_key() != "string" {
                    return Err(terr(line, col, "foreign_owned_type", "map_take requires i64 or String keys", ""));
                }
                if *value != i64t() && *value != boolt() && !self.owned_aggregate(value) {
                    return Err(terr(line, col, "foreign_owned_type", "map_take requires i64, bool or owned aggregate values", ""));
                }
            }
        }
        let m = self.instantiate(line, col, &format!("function {}", n), &fn_owner(&n), &f.tparams, &targs)?;
        if let Some(m) = &m {
            e.inst = Some(inst_of(fn_owner(&n), targs.as_deref().unwrap_or(&[]), m));
        }
        let m = m.unwrap_or_default();
        arity(f.params.len())?;
        let direct = self.read_context == call_site;
        let string_element = cat.is_some() && n == "vec_get"
            && targs.as_ref().unwrap()[0].to_key() == "string";
        if cat.is_some() && matches!(n.as_str(), "map_get_mut" | "vec_at_mut") && !direct {
            return Err(terr(line, col, "foreign_contract", format!("{n} is only allowed as a direct match scrutinee"), ""));
        }
        if cat.is_some() && !direct && (matches!(n.as_str(), "vec_get" | "map_get")
            || n == "map_lookup" && targs.as_ref().unwrap()[0].to_key() != "string") {
            let payload = targs.as_ref().and_then(|a| a.last()).unwrap();
            if !string_element && self.is_extended(payload) {
                return Err(terr(line, col, "foreign_owned_type", format!("{} cannot copy {}", n, tn(payload)), ""));
            }
        }
        match &cat {
            Some(_) if n == "string_eq" => {
                for arg in &mut e.args {
                    if let Expr::StrLit(lit) = arg {
                        lit.ty = Some("str".into());
                        continue;
                    }
                    let got = self.text_expr(arg, scope, c)?;
                    if !matches!(got.to_key().as_str(), "string" | "str" | "array_u8") || matches!(arg, Expr::Move(_)) {
                        return Err(terr(arg.line(), arg.col(), "type_mismatch",
                            format!("expected String, str or [u8], found {}", tn(&got)), ""));
                    }
                }
            }
            Some(cf) if is_catalog_contract_only(&n) => self.observer_args(e, cf, &m, scope, c)?,
            // G5 A0: marker, place, type and same-call exclusivity
            _ if f.inout.iter().any(|b| *b) || cat.is_none() && e.args.iter().any(|x| matches!(x, Expr::InoutArg(_))) => {
                self.inout_args(e, &f, &m, scope, c)?
            }
            _ => {
                for (i, (arg, p)) in e.args.iter_mut().zip(f.params.iter()).enumerate() {
                    let want = substitute(p, &m);
                    if cat.is_some() && n == "map_get" && want.to_key() == "string" {
                        self.check_text_key(arg, scope, c, None)?;
                    } else if cat.is_none() {
                        self.expect_user_arg(arg, &want, scope, c, !f.foreign && !self.takes[&n][i])?;
                    } else {
                        self.expect(arg, &want, scope, c, None)?;
                    }
                }
            }
        }
        if cat.is_some() && n == "vec_at_mut" && !self.owned_aggregate(&targs.as_ref().unwrap()[0]) {
            return Err(terr(line, col, "foreign_owned_type", "vec_at_mut requires an owned aggregate element", ""));
        }
        if cat.is_some() && n == "map_remove" {
            let key = &targs.as_ref().unwrap()[0];
            if *key != i64t() && key.to_key() != "string" {
                return Err(terr(line, col, "foreign_owned_type", "map_remove requires i64 or String keys", ""));
            }
        }
        if string_element {
            if self.borrowed_text != call_site {
                return Err(terr(line, col, "foreign_owned_type",
                    "a Vec<String> element must be a direct borrowed text argument", ""));
            }
            let vector = &e.args[0];
            if place_root(vector).is_none() {
                return Err(terr(vector.line(), vector.col(), "foreign_borrow",
                    "a borrowed Vec<String> element needs a vector place", ""));
            }
            return Ok(strt());
        }
        if cat.is_some() && matches!(n.as_str(), "vec_push" | "map_insert") {
            if targs.as_ref().unwrap()[0].to_key() == "string" {
                let arg = &e.args[1];
                if !matches!(arg, Expr::Call(_)) && !matches!(arg, Expr::Move(v) if v.fields.is_empty()) {
                    return Err(terr(arg.line(), arg.col(), "foreign_move", "an owned String argument needs a call result or move", ""));
                }
            }
            let arg = e.args.last().unwrap();
            if Self::is_iteration_binder(arg, scope) {
                self.own(arg, scope)?;
            }
            if matches!(arg, Expr::Var(v) if scope.get(&v.name).is_some_and(|entry| entry.kind == "map_inout" && entry.ty != i64t() && entry.ty != boolt())) {
                return Err(terr(arg.line(), arg.col(), "foreign_move",
                    "an aggregate collection payload needs a constructor, call result or move", ""));
            }
            let payload = targs.as_ref().and_then(|a| a.last()).unwrap();
            if self.owned_aggregate(payload) && !matches!(arg, Expr::StructLit(_) | Expr::Call(_) | Expr::Move(_)) {
                return Err(terr(arg.line(), arg.col(), "foreign_move", "an aggregate collection payload needs a constructor, call result or move", ""));
            }
        }
        if cat.is_some() {
            self.used_effects.extend(f.effects.iter().cloned());
            return Ok(substitute(&f.ret, &m));
        }
        if let Some(takes) = self.takes.get(&n) {
            for ((arg, param), take) in e.args.iter().zip(&f.params).zip(takes) {
                let pt = substitute(param, &m);
                if *take && Self::is_iteration_binder(arg, scope) {
                    return Err(terr(arg.line(), arg.col(), "foreign_move",
                        "a map iteration binder cannot be used as an owned value", ""));
                }
                if *take && pt.is_array() && !matches!(arg, Expr::Move(_)) {
                    let fix = match arg {
                        Expr::Var(v) => format!("write move {}", v.name),
                        _ => "bind the array to a local, then move it".into(),
                    };
                    return Err(terr(arg.line(), arg.col(), "foreign_move", "missing move in a consuming argument", fix));
                }
                if !*take || !self.is_struct(&pt) {
                    continue;
                }
                let valid = match arg {
                    Expr::StructLit(_) | Expr::Call(_) => true,
                    Expr::Move(v) if v.fields.is_empty() => {
                        scope.get(&v.name).is_some_and(|entry| entry.kind == "local")
                            || self.funcs.get(&self.cur_name).zip(self.takes.get(&self.cur_name))
                                .is_some_and(|(sig, flags)| sig.pnames.iter().zip(flags)
                                    .any(|(name, take)| name == &v.name && *take))
                    }
                    _ => false,
                };
                if !valid {
                    return Err(terr(arg.line(), arg.col(), "foreign_move",
                        "a take struct argument needs a constructor, call result or move", ""));
                }
            }
        }
        if !c && f.pred {
            return Err(terr(line, col, "pred_in_code", format!("pred {} is only allowed in contracts", n),
                "use it in requires, ensures, invariant or assert, or write a fn for code"));
        }
        if c && !f.effects.is_empty() {
            return Err(terr(
                line,
                col,
                "effect_in_contract",
                format!("function {} has effects; not allowed in contracts", n),
                "",
            ));
        }
        if c {
            // Contract expansion can erase the call before backend validation.
            if self.contract_vec_slice.is_none() && e.args.iter().zip(&f.params).any(|(arg, p)| {
                expr_ty(arg).as_deref().is_some_and(|ty| key_is_vec_slice_arg(ty, &substitute(p, &m).to_key()))
            }) {
                self.contract_vec_slice = Some((line, col));
            }
            if self.ckind == "cost" {
                return Err(terr(
                    line,
                    col,
                    "call_in_contract",
                    "calling functions in contracts is not supported in v0.1",
                    "inline the property",
                ));
            }
            if f.opaque {
                // P13o: an opaque call stays a call; the verifier applies its uninterpreted symbol (code generation
                // inlines it: opaque_inline)
                e.opaque_c = true;
                return Ok(substitute(&f.ret, &m));
            }
            // P06: expand the call (docs/L3_RESTRUCTURE_SPEC.md appendix P06)
            let tpl = self.x_template(line, col, &n, &f)?;
            if x_has_quant(&tpl) {
                let mut names = HashSet::new();
                for arg in &e.args {
                    x_names(arg, &mut names);
                }
                if let Some(q) = x_captured_quant(&tpl, &names) {
                    return Err(terr(line, col, "call_in_contract",
                        format!("function {} cannot be expanded in a contract: an argument uses the name {} that its body binds in {}", n, q.var, q.kind),
                        format!("rename {} in the argument or in the body of {}", q.var, n)));
                }
            }
            let x = x_instantiate(&tpl, &f.pnames, &e.args, line, col);
            x_sized(line, col, &n, &x)?;
            self.x_pending = Some(x);
            return Ok(substitute(&f.ret, &m));
        }
        self.used_effects.extend(f.effects.iter().cloned());
        if !f.foreign {
            // G9 A0: foreign calls are leaves of the call graph
            self.call_log.push((self.cur_name.clone(), line, col, targs.clone(), n.clone()));
        }
        Ok(substitute(&f.ret, &m))
    }
}

// ---- P06: contract expansion of user functions (check.py X_* / x_*) ----
/// P07: does the expression call a bit builtin anywhere (check.py `x_has_bit`)
fn x_has_bit(e: &Expr) -> bool {
    if let Expr::Call(c) = e {
        if BIT_BUILTINS.contains(&c.name.as_str()) {
            return true;
        }
    }
    crate::refine::kids(e).into_iter().any(x_has_bit)
}
const X_FIX: &str = "keep it loop-free with no requires (immutable lets, if/return over i64, bool and array parameters), or inline the property";
/// nodes in one template or one expansion
const X_MAX: usize = 4096;
const X_R_STMT: &str = "its body has a statement other than an immutable i64 or bool let, if and return";
const X_R_EXPR: &str = "its body has an expression other than literals, names, operators, indexing, field reads, quantifiers, aggregates and calls";

fn x_has_quant(e: &Expr) -> bool {
    matches!(e, Expr::Quant(_)) || crate::refine::kids(e).into_iter().any(x_has_quant)
}

fn x_names(e: &Expr, names: &mut HashSet<String>) {
    if let Expr::Var(v) = e {
        names.insert(v.name.clone());
    }
    for child in crate::refine::kids(e) {
        x_names(child, names);
    }
}

fn x_captured_quant<'a>(e: &'a Expr, names: &HashSet<String>) -> Option<&'a Quant> {
    if let Expr::Quant(q) = e {
        if names.contains(&q.var) {
            return Some(q);
        }
    }
    crate::refine::kids(e).into_iter().find_map(|child| x_captured_quant(child, names))
}

fn x_record(ty: &GType, decls: &HashMap<String, TypeDecl>) -> bool {
    ty.owner.is_none() && ty.args.is_empty() && matches!(decls.get(&ty.name), Some(TypeDecl::Struct(_)))
}

fn x_sig_reason(f: &Function, decls: &HashMap<String, TypeDecl>) -> Option<String> {
    if !f.tparams.is_empty() {
        return Some("it is generic".into());
    }
    if f.params.iter().any(|p| p.mode_pos.is_some() || p.take) {
        return Some("it has an inout or take parameter".into());
    }
    let ok = [i64t(), boolt(), GType::array(), GType::array_u8(), GType::array_bool()];
    if f.params.iter().any(|p| {
        let ty = GType::from_key(&p.ty);
        !ok.contains(&ty) && !x_record(&ty, decls)
    }) {
        return Some("its parameters are not all i64, bool, arrays of i64, u8 or bool, or structs".into());
    }
    let ret = GType::from_key(&f.ret);
    if ret != i64t() && ret != boolt() {
        return Some("it does not return bool or i64".into());
    }
    if !f.requires.is_empty() {
        return Some("it has requires".into());
    }
    if f.decreases.is_some() {
        return Some("it has decreases".into());
    }
    if f.cost.is_some() {
        return Some("it has a cost clause".into());
    }
    None
}

fn x_scan_block(f: &Function, stmts: &[Stmt], funcs: &HashMap<String, Rc<FuncInfo>>) -> Option<String> {
    for (i, st) in stmts.iter().enumerate() {
        match st {
            Stmt::Let(l) => {
                let t = GType::from_key(&l.ty);
                if l.mutable || l.refine.is_some() || (t != i64t() && t != boolt()) {
                    return Some(X_R_STMT.into());
                }
                if let Some(r) = x_scan_expr(&l.value, funcs) {
                    return Some(r);
                }
            }
            Stmt::Return(r) => {
                return match &r.value {
                    None => Some(X_R_STMT.into()),
                    Some(v) => x_scan_expr(v, funcs),
                };
            }
            Stmt::If(x) => {
                if GType::from_key(&f.ret) != boolt() {
                    return Some("it returns i64 through an if (only bool functions may branch)".into());
                }
                if let Some(r) = x_scan_expr(&x.cond, funcs).or_else(|| x_scan_block(f, &x.then, funcs)) {
                    return Some(r);
                }
                return if !x.els.is_empty() { x_scan_block(f, &x.els, funcs) } else { x_scan_block(f, &stmts[i + 1..], funcs) };
            }
            _ => return Some(X_R_STMT.into()),
        }
    }
    Some("an if branch does not end in return".into())
}

fn x_scan_expr(e: &Expr, funcs: &HashMap<String, Rc<FuncInfo>>) -> Option<String> {
    match e {
        Expr::IntLit(_) | Expr::BoolLit(_) | Expr::Var(_) => None,
        Expr::Unary(u) => x_scan_expr(&u.operand, funcs),
        Expr::Binary(b) => x_scan_expr(&b.left, funcs).or_else(|| x_scan_expr(&b.right, funcs)),
        Expr::Index(x) => x_scan_expr(&x.array, funcs).or_else(|| x_scan_expr(&x.index, funcs)),
        Expr::FieldAccess(_) | Expr::Quant(_) => {
            crate::refine::kids(e).into_iter().find_map(|child| x_scan_expr(child, funcs))
        }
        Expr::Call(c) => {
            if funcs.contains_key(&c.name) || c.name == "len" || is_bit_builtin(&c.name) || c.name == "load_be" {
                c.args.iter().find_map(|a| x_scan_expr(a, funcs))
            } else {
                Some(format!("it calls {}", c.name))
            }
        }
        _ => Some(X_R_EXPR.into()),
    }
}

fn x_size(e: &Expr) -> usize {
    match e {
        Expr::Unary(u) => 1 + x_size(&u.operand),
        Expr::Binary(b) => 1 + x_size(&b.left) + x_size(&b.right),
        Expr::Index(x) => 1 + x_size(&x.array) + x_size(&x.index),
        Expr::Call(c) => 1 + c.args.iter().map(x_size).sum::<usize>(),
        Expr::FieldAccess(x) => 1 + x_size(&x.base),
        Expr::Quant(x) => 1 + x_size(&x.lo) + x_size(&x.hi) + x_size(&x.body),
        _ => 1,
    }
}

fn x_sized(line: i64, col: i64, n: &str, e: &Expr) -> Res<()> {
    if x_size(e) > X_MAX {
        return Err(terr(line, col, "call_in_contract",
            format!("function {} cannot be expanded in a contract: its expansion is too large", n), X_FIX));
    }
    Ok(())
}

/// e with every let name replaced by its folded value
fn x_subst(e: &Expr, env: &HashMap<String, Expr>) -> Expr {
    match e {
        Expr::Var(v) if env.contains_key(&v.name) => env[&v.name].clone(),
        Expr::Unary(u) => Expr::Unary(Unary {
            line: u.line, col: u.col, op: u.op.clone(), operand: Box::new(x_subst(&u.operand, env)), ty: u.ty.clone(),
        }),
        Expr::Binary(b) => Expr::Binary(Binary {
            line: b.line, col: b.col, op: b.op.clone(),
            left: Box::new(x_subst(&b.left, env)), right: Box::new(x_subst(&b.right, env)), ty: b.ty.clone(),
        }),
        Expr::Index(x) => Expr::Index(Index {
            line: x.line, col: x.col, array: Box::new(x_subst(&x.array, env)), index: Box::new(x_subst(&x.index, env)),
            ty: x.ty.clone(),
        }),
        Expr::FieldAccess(x) => Expr::FieldAccess(FieldAccess {
            base: Box::new(x_subst(&x.base, env)), ..x.clone()
        }),
        Expr::Quant(x) => Expr::Quant(Quant {
            lo: Box::new(x_subst(&x.lo, env)),
            hi: Box::new(x_subst(&x.hi, env)),
            // R03L: a name bound by the quantifier is never replaced inside its body
            body: Box::new(if env.contains_key(&x.var) {
                let mut inner = env.clone();
                inner.remove(&x.var);
                x_subst(&x.body, &inner)
            } else {
                x_subst(&x.body, env)
            }), ..x.clone()
        }),
        Expr::Call(c) => Expr::Call(Call {
            line: c.line, col: c.col, name: c.name.clone(), args: c.args.iter().map(|a| x_subst(a, env)).collect(),
            ty: c.ty.clone(), targs: c.targs.clone(), inst: c.inst.clone(), old: c.old.clone(), opaque_c: c.opaque_c,
        }),
        _ => e.clone(),
    }
}

/// if c { t } else { el } over bool values, short-circuit form (nodes at the if's position)
fn x_join(line: i64, col: i64, c: Expr, t: Expr, el: Expr) -> Expr {
    let b = Some(boolt().to_key());
    let un = |x: Expr| Expr::Unary(Unary { line, col, op: "!".into(), operand: Box::new(x), ty: b.clone() });
    let bin = |op: &str, l: Expr, r: Expr| Expr::Binary(Binary {
        line, col, op: op.into(), left: Box::new(l), right: Box::new(r), ty: b.clone(),
    });
    match (&t, &el) {
        (Expr::BoolLit(x), _) if !x.value => bin("&&", un(c), el),
        (Expr::BoolLit(x), _) if x.value => bin("||", c, el),
        (_, Expr::BoolLit(x)) if !x.value => bin("&&", c, t),
        (_, Expr::BoolLit(x)) if x.value => bin("||", un(c), t),
        _ => {
            let c2 = c.clone();
            bin("||", bin("&&", c, t), bin("&&", un(c2), el))
        }
    }
}

/// The template with each parameter replaced by a copy of the checked argument; every template node takes the call
/// site's position (the arguments keep theirs).
fn x_instantiate(tpl: &Expr, pnames: &[String], args: &[Expr], line: i64, col: i64) -> Expr {
    match tpl {
        Expr::Var(v) => match pnames.iter().position(|p| *p == v.name) {
            Some(k) => args[k].clone(),
            None => Expr::Var(Var { line, col, name: v.name.clone(), ty: v.ty.clone() }),
        },
        Expr::IntLit(x) => Expr::IntLit(IntLit { line, col, value: x.value.clone(), ty: x.ty.clone() }),
        Expr::BoolLit(x) => Expr::BoolLit(BoolLit { line, col, value: x.value, ty: x.ty.clone() }),
        Expr::Unary(u) => Expr::Unary(Unary {
            line, col, op: u.op.clone(), operand: Box::new(x_instantiate(&u.operand, pnames, args, line, col)), ty: u.ty.clone(),
        }),
        Expr::Binary(b) => Expr::Binary(Binary {
            line, col, op: b.op.clone(),
            left: Box::new(x_instantiate(&b.left, pnames, args, line, col)),
            right: Box::new(x_instantiate(&b.right, pnames, args, line, col)), ty: b.ty.clone(),
        }),
        Expr::Index(x) => Expr::Index(Index {
            line, col, array: Box::new(x_instantiate(&x.array, pnames, args, line, col)),
            index: Box::new(x_instantiate(&x.index, pnames, args, line, col)), ty: x.ty.clone(),
        }),
        Expr::FieldAccess(x) => Expr::FieldAccess(FieldAccess {
            line, col, base: Box::new(x_instantiate(&x.base, pnames, args, line, col)), ..x.clone()
        }),
        Expr::Quant(x) => Expr::Quant(Quant {
            line, col,
            lo: Box::new(x_instantiate(&x.lo, pnames, args, line, col)),
            hi: Box::new(x_instantiate(&x.hi, pnames, args, line, col)),
            // R03L: a name bound by the quantifier is never replaced inside its body
            body: Box::new(if pnames.contains(&x.var) {
                let inner: Vec<String> = pnames.iter().map(|p| if *p == x.var { String::new() } else { p.clone() }).collect();
                x_instantiate(&x.body, &inner, args, line, col)
            } else {
                x_instantiate(&x.body, pnames, args, line, col)
            }), ..x.clone()
        }),
        Expr::Call(c) => Expr::Call(Call {
            line, col, name: c.name.clone(), args: c.args.iter().map(|a| x_instantiate(a, pnames, args, line, col)).collect(),
            ty: c.ty.clone(), targs: c.targs.clone(), inst: c.inst.clone(), old: c.old.clone(), opaque_c: c.opaque_c,
        }),
        _ => tpl.clone(),
    }
}

/// P13o (docs/L3_RESTRUCTURE_SPEC.md appendix P13o.1, check.py `opaque_inline`): after verification, every contract
/// application of an opaque function becomes its expansion, so the code generators see what P06 would have produced
/// (a run-time contract check evaluates the body). Idempotent: an inlined program has no opaque contract call left.
pub fn opaque_inline(prog: &mut Program) {
    if !prog.functions.iter().any(|f| f.opaque) {
        return;
    }
    let defs: HashMap<String, (Vec<String>, Option<Expr>)> = prog
        .functions
        .iter()
        .filter(|f| f.opaque)
        .map(|f| (f.name.clone(), (f.params.iter().map(|p| p.name.clone()).collect(), f.x_auto.clone())))
        .collect();
    fn ex(e: &mut Expr, d: &HashMap<String, (Vec<String>, Option<Expr>)>) {
        if let Expr::Call(c) = e {
            if c.opaque_c {
                if let Some((pn, Some(tpl))) = d.get(&c.name) {
                    for a in c.args.iter_mut() {
                        ex(a, d);
                    }
                    let env: HashMap<String, Expr> = pn.iter().cloned().zip(c.args.iter().cloned()).collect();
                    let mut n = x_subst(tpl, &env);
                    ex(&mut n, d);
                    *e = n;
                    return;
                }
            }
        }
        match e {
            Expr::InoutArg(x) => {
                ex(&mut x.expr, d);
                for a in x.post_refine.iter_mut() {
                    ex(&mut a.cond, d);
                }
            }
            Expr::Unary(x) => ex(&mut x.operand, d),
            Expr::Binary(x) => {
                ex(&mut x.left, d);
                ex(&mut x.right, d);
            }
            Expr::Call(x) => x.args.iter_mut().for_each(|a| ex(a, d)),
            Expr::Index(x) => {
                ex(&mut x.array, d);
                ex(&mut x.index, d);
            }
            Expr::Ctor(x) => {
                if let Some(a) = x.arg.as_mut() {
                    ex(a, d);
                }
            }
            Expr::Quant(x) => {
                ex(&mut x.lo, d);
                ex(&mut x.hi, d);
                ex(&mut x.body, d);
            }
            Expr::StructLit(x) => x.fields.iter_mut().for_each(|f| ex(&mut f.value, d)),
            Expr::EnumCtor(x) => x.args.iter_mut().for_each(|a| ex(a, d)),
            Expr::FieldAccess(x) => ex(&mut x.base, d),
            Expr::IsVariant(x) => ex(&mut x.value, d),
            Expr::VariantField(x) => ex(&mut x.value, d),
            Expr::Spent(_) | Expr::Move(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) => {}
        }
    }
    fn st(ss: &mut [Stmt], d: &HashMap<String, (Vec<String>, Option<Expr>)>) {
        for s in ss.iter_mut() {
            match s {
                Stmt::Par(x) => x.calls.iter_mut().for_each(|a| ex(a, d)),
                Stmt::Let(x) => ex(&mut x.value, d),
                Stmt::Assign(x) => ex(&mut x.value, d),
                Stmt::IndexAssign(x) => {
                    ex(&mut x.index, d);
                    ex(&mut x.value, d);
                }
                Stmt::If(x) => {
                    ex(&mut x.cond, d);
                    st(&mut x.then, d);
                    st(&mut x.els, d);
                }
                Stmt::While(x) => {
                    ex(&mut x.cond, d);
                    x.invariants.iter_mut().for_each(|a| ex(a, d));
                    ex(&mut x.decreases, d);
                    st(&mut x.body, d);
                }
                Stmt::ForMap(x) => {
                    ex(&mut x.place, d);
                    x.invariants.iter_mut().for_each(|a| ex(a, d));
                    st(&mut x.body, d);
                }
                Stmt::Return(x) => {
                    if let Some(v) = x.value.as_mut() {
                        ex(v, d);
                    }
                }
                Stmt::Assert(x) => ex(&mut x.cond, d),
                Stmt::ExprStmt(x) => ex(&mut x.expr, d),
                Stmt::FieldAssign(x) => ex(&mut x.value, d),
                Stmt::Match(x) => {
                    ex(&mut x.scrutinee, d);
                    for a in x.arms.iter_mut() {
                        st(&mut a.body, d);
                    }
                }
            }
        }
    }
    for f in prog.functions.iter_mut() {
        f.requires.iter_mut().for_each(|e| ex(e, &defs));
        f.ensures.iter_mut().for_each(|e| ex(e, &defs));
        if let Some(e) = f.decreases.as_mut() {
            ex(e, &defs);
        }
        if let Some(e) = f.cost.as_mut() {
            ex(e, &defs);
        }
        st(&mut f.body, &defs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Result<TypeTable, PruvError> {
        let mut p = crate::parse(src).expect("parse");
        check(&mut p)
    }

    #[test]
    fn int_range_edges() {
        assert!(int_in_range("9223372036854775807"));
        assert!(!int_in_range("9223372036854775808"));
        assert!(int_in_range("-9223372036854775808"));
        assert!(!int_in_range("-9223372036854775809"));
        assert!(int_in_range("0"));
        assert!(!int_in_range("100000000000000000000000000000000000000000"));
    }

    #[test]
    fn recursive_type_message() {
        let e = run("struct A { b: B }\nstruct B { a: A }\n").unwrap_err();
        assert_eq!(e.code, "recursive_type");
        // G9 A0: a cycle without a base case has no finite value
        assert_eq!(e.message, "recursive type has no finite value: A -> B -> A");
        assert_eq!(e.fix, "add a variant without recursive fields (a base case such as `Empty`)");
    }

    #[test]
    fn missing_effect_fix() {
        let e = run("fn f(x: i64) -> () { print(x); return; }").unwrap_err();
        assert_eq!(e.stage, "effect");
        assert_eq!(e.code, "missing_effect");
        assert_eq!(e.message, "function f uses effect `io` but does not declare it");
        assert_eq!(e.fix, "add `! io` after the return type");
    }

    // L3-10: eq_range
    #[test]
    fn eq_range_checks_in_code() {
        assert!(run("fn f(a: [u8], b: [u8]) -> bool { return eq_range(a, 0, b, 1, 2); }").is_ok());
        assert!(run("fn f(a: [i64], b: [i64]) -> bool { return eq_range(a, 0, b, 1, 2); }").is_ok());
    }

    // L3-10: eq_range
    #[test]
    fn eq_range_rejected_in_contracts() {
        let e = run("fn f(a: [u8], b: [u8]) -> bool requires eq_range(a, 0, b, 0, 1) { return true; }").unwrap_err();
        assert_eq!(e.message, "eq_range is not allowed in contracts");
        let e = run("fn f(a: [u8], b: [u8]) -> bool ensures eq_range(a, 0, b, 0, 1) { return true; }").unwrap_err();
        assert_eq!(e.message, "eq_range is not allowed in contracts");
    }

    // L3-10: eq_range
    #[test]
    fn eq_range_element_type_and_arity_errors() {
        let e = run("fn f(a: [u8], b: [i64]) -> bool { return eq_range(a, 0, b, 0, 1); }").unwrap_err();
        assert_eq!(e.code, "type_mismatch");
        let e = run("fn f(a: [u8], b: [u8]) -> bool { return eq_range(a, 0, b, 0); }").unwrap_err();
        assert_eq!(e.code, "arity");
        let e = run("fn f(a: [bool], b: [bool]) -> bool { return eq_range(a, 0, b, 0, 1); }").unwrap_err();
        assert_eq!(e.code, "type_mismatch");
    }

    // L-copy-range: accepted forms
    #[test]
    fn copy_within_checks() {
        assert!(run("fn f(inout a: [u8]) -> () { copy_within(inout a, 1, 0, 2); }").is_ok());
        assert!(run("fn f(inout a: [i64]) -> () { copy_within(inout a, 1, 0, 2); }").is_ok());
        let cases = [
            ("fn f(inout a: [u8]) -> () { copy_within(inout a, 1, 0); }", "arity"),
            ("fn f(a: [u8]) -> () { copy_within(a, 1, 0, 2); }", "inout_argument"),
            ("fn f(inout a: [bool]) -> () { copy_within(inout a, 1, 0, 2); }", "type_mismatch"),
            ("fn f(inout a: [u8]) -> () { copy_within(inout a, 1, len(a), 2); }", "inout_alias"),
            ("fn f(inout a: [u8]) -> i64 { return 1 + copy_within(inout a, 1, 0, 2); }", "inout_context"),
            ("fn f(inout a: [u8]) -> () requires copy_within(a, 1, 0, 2) == 0 { return; }", "effect_in_contract"),
        ];
        for (src, code) in cases {
            assert_eq!(run(src).unwrap_err().code, code, "{src}");
        }
    }

    #[test]
    fn copy_range_checks_as_a_statement() {
        assert!(run("fn f(inout d: [u8], s: [u8]) -> () { copy_range(inout d, 0, s, 1, 2); }").is_ok());
        assert!(run("fn f(inout d: [i64], s: [i64]) -> () { copy_range(inout d, 0, s, 1, 2); }").is_ok());
    }

    // L-copy-range: every rejection of the statement
    #[test]
    fn copy_range_rejections() {
        let e = run("fn f(inout d: [u8], s: [i64]) -> () { copy_range(inout d, 0, s, 0, 1); }").unwrap_err();
        assert_eq!(e.code, "type_mismatch");
        let e = run("fn f(inout d: [bool], s: [bool]) -> () { copy_range(inout d, 0, s, 0, 1); }").unwrap_err();
        assert_eq!(e.code, "type_mismatch");
        let e = run("fn f(inout d: [u8], s: [u8]) -> () { copy_range(inout d, 0, s, 0); }").unwrap_err();
        assert_eq!(e.code, "arity");
        let e = run("fn f(d: [u8], s: [u8]) -> () { copy_range(d, 0, s, 0, 1); }").unwrap_err();
        assert_eq!(e.code, "inout_argument");
        let e = run("fn f(inout a: [u8]) -> () { copy_range(inout a, 0, a, 4, 2); }").unwrap_err();
        assert_eq!(e.code, "inout_alias");
        let e = run("fn f(inout a: [i64], s: [i64]) -> () { copy_range(inout a, len(a) - 2, s, 0, 2); }").unwrap_err();
        assert_eq!(e.code, "inout_alias");
        let e = run("fn f(inout d: [u8], s: [u8]) -> i64 { let x: i64 = 1 + copy_range(inout d, 0, s, 0, 1); return x; }")
            .unwrap_err();
        assert_eq!(e.code, "inout_context");
        let e = run("fn f(inout d: [u8], s: [u8]) -> () requires copy_range(d, 0, s, 0, 1) == 0 { return; }").unwrap_err();
        assert_eq!(e.code, "effect_in_contract");
        assert_eq!(e.message, "copy_range is not allowed in contracts");
        let e = run("fn copy_range(a: i64) -> i64 { return a; }").unwrap_err();
        assert_eq!(e.code, "reserved");
    }

    #[test]
    fn type_name_fix_capitalizes() {
        let e = run("struct point { x: i64 }").unwrap_err();
        assert_eq!(e.code, "type_name");
        assert_eq!(e.fix, "rename it, e.g. Point");
    }

    #[test]
    fn int_literal_out_of_range() {
        let e = run("fn f() -> i64 { return 9223372036854775808; }").unwrap_err();
        assert_eq!(e.code, "int_range");
    }
}

/// Executable calls only: contracts do not contribute to branch eligibility.
struct ParInfo {
    effects: Vec<String>,
    foreign: Option<String>,
    par: bool,
    callees: Vec<String>,
}

fn par_info(f: &Function, funcs: &HashMap<String, Rc<FuncInfo>>) -> ParInfo {
    let mut info = ParInfo {
        effects: ["io", "fs", "proc", "net", "async"].iter().filter(|e| f.effects.iter().any(|x| x == **e)).map(|e| e.to_string()).collect(),
        foreign: f.foreign.as_ref().map(|_| f.name.clone()),
        par: false,
        callees: Vec::new(),
    };
    fn ex(e: &Expr, info: &mut ParInfo, funcs: &HashMap<String, Rc<FuncInfo>>) {
        if let Expr::Call(c) = e {
            if funcs.contains_key(&c.name) {
                if !info.callees.contains(&c.name) { info.callees.push(c.name.clone()); }
            } else if is_catalog(&c.name) && info.foreign.is_none() {
                info.foreign = Some(c.name.clone());
            }
        }
        for k in crate::refine::kids(e) { ex(k, info, funcs); }
    }
    fn walk(stmts: &[Stmt], info: &mut ParInfo, funcs: &HashMap<String, Rc<FuncInfo>>) {
        for st in stmts {
            match st {
                Stmt::Let(x) => ex(&x.value, info, funcs),
                Stmt::Assign(x) => ex(&x.value, info, funcs),
                Stmt::FieldAssign(x) => ex(&x.value, info, funcs),
                Stmt::IndexAssign(x) => {
                    ex(&x.index, info, funcs);
                    ex(&x.value, info, funcs);
                }
                Stmt::If(x) => {
                    ex(&x.cond, info, funcs);
                    walk(&x.then, info, funcs);
                    walk(&x.els, info, funcs);
                }
                Stmt::While(x) => {
                    ex(&x.cond, info, funcs);
                    walk(&x.body, info, funcs);
                }
                Stmt::ForMap(x) => {
                    ex(&x.place, info, funcs);
                    walk(&x.body, info, funcs);
                }
                Stmt::Return(x) => if let Some(v) = &x.value { ex(v, info, funcs); },
                Stmt::ExprStmt(x) => ex(&x.expr, info, funcs),
                Stmt::Par(x) => {
                    info.par = true;
                    for c in &x.calls { ex(c, info, funcs); }
                }
                Stmt::Match(x) => {
                    ex(&x.scrutinee, info, funcs);
                    for a in &x.arms { walk(&a.body, info, funcs); }
                }
                Stmt::Assert(_) => {},
            }
        }
    }
    walk(&f.body, &mut info, funcs);
    info
}

/// Source-order DFS, including the starting function, with cycles visited only once.
fn par_witness(start: &str, kind: usize, infos: &HashMap<String, ParInfo>) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let mut stack = vec![(start.to_string(), vec![start.to_string()])];
    while let Some((n, path)) = stack.pop() {
        if !seen.insert(n.clone()) { continue; }
        let info = &infos[&n];
        let found = match kind {
            0 => !info.effects.is_empty(),
            1 => info.foreign.is_some(),
            _ => info.par,
        };
        if found { return Some(path); }
        for c in info.callees.iter().rev() {
            if !seen.contains(c) {
                let mut p = path.clone();
                p.push(c.clone());
                stack.push((c.clone(), p));
            }
        }
    }
    None
}


// Executable calls only, like Python par_items: assertions and loop contracts do not spend work.
fn cost_calls(stmts: &[Stmt], out: &mut Vec<Call>) {
    fn ex(e: &Expr, out: &mut Vec<Call>) {
        if let Expr::Call(c) = e { out.push(c.clone()); }
        for k in crate::refine::kids(e) { ex(k, out); }
    }
    for st in stmts {
        match st {
            Stmt::Let(x) => ex(&x.value, out),
            Stmt::Assign(x) => ex(&x.value, out),
            Stmt::FieldAssign(x) => ex(&x.value, out),
            Stmt::IndexAssign(x) => { ex(&x.index, out); ex(&x.value, out); }
            Stmt::If(x) => { ex(&x.cond, out); cost_calls(&x.then, out); cost_calls(&x.els, out); }
            Stmt::While(x) => { ex(&x.cond, out); cost_calls(&x.body, out); }
            Stmt::ForMap(x) => cost_calls(&x.body, out),
            Stmt::Return(x) => if let Some(v) = &x.value { ex(v, out); },
            Stmt::ExprStmt(x) => ex(&x.expr, out),
            Stmt::Par(x) => for c in &x.calls { ex(c, out); },
            Stmt::Match(x) => {
                ex(&x.scrutinee, out);
                for a in &x.arms { cost_calls(&a.body, out); }
            }
            Stmt::Assert(_) => {},
        }
    }
}

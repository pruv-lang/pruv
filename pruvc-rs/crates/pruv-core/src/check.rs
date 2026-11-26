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
    key_is_collection_owner, key_is_vec_slice_arg, mk_array, mk_nominal, mk_option, mk_result_with_error, substitute, GType, Mapping, ParamId,
};
use crate::intwidth::{self, IntWidth};
use crate::typeinfo::{program_uses_inout, TypeTable};
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
    ErrorPayload,
}

/// P2: pure (i64, i64) -> i64 bit builtins (check.py BIT_BUILTINS); later batches add names here.
pub(crate) const BIT_BUILTINS: [&str; 7] = ["band", "bor", "bxor", "wrapping_shl", "ushr", "wrapping_add", "wrapping_mul"];

pub(crate) fn is_bit_builtin(n: &str) -> bool {
    BIT_BUILTINS.contains(&n)
}

/// P2 batch 4: byte-word access on [u8] (check.py WORD_BUILTINS).
pub(crate) const WORD_BUILTINS: [&str; 4] = ["load_be", "store_be", "load_le", "store_le"];

pub(crate) fn is_word_builtin(n: &str) -> bool {
    WORD_BUILTINS.contains(&n)
}

/// P2 batch 4: the value of an integer literal width argument n of load_be / store_be, 1..=8 (None otherwise).
pub(crate) fn word_width(e: &Expr) -> Option<i64> {
    match e {
        Expr::IntLit(l) if l.kind == crate::ast::LitKind::Int => l.value.parse::<i64>().ok().filter(|v| (1..=8).contains(v)),
        _ => None,
    }
}

pub(crate) use crate::ast::is_array_lit;

/// P7: an integer literal without a type of its own (char/byte literals and constant uses carry theirs).
fn bare_lit(e: &Expr) -> bool {
    matches!(e, Expr::IntLit(l) if l.kind == crate::ast::LitKind::Int)
}

/// Built-ins used only in contracts: name -> (argument kind, result).
pub(crate) fn contract_only(n: &str) -> Option<(&'static str, CRes)> {
    match n {
        "is_some" => Some(("option", CRes::Bool)),
        "some_val" => Some(("option", CRes::Payload)),
        "is_ok" => Some(("result", CRes::Bool)),
        "ok_val" => Some(("result", CRes::Payload)),
        "err_val" => Some(("result", CRes::ErrorPayload)),
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
pub(crate) fn expr_ty(e: &Expr) -> &Ty {
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

/// Whether the integer literal (sign plus decimal string) fits the given scalar type.
fn int_in_range(v: &str, width: &IntWidth) -> bool {
    v.parse::<i128>().is_ok_and(|n| width.contains(n))
}

fn check_int_literal(lit: &IntLit, width: &IntWidth) -> Res<GType> {
    if !int_in_range(&lit.value, width) {
        return Err(terr(lit.line, lit.col, "int_range", format!("integer literal out of {} range", width.name), ""));
    }
    Ok(GType::new(width.name, Vec::new()))
}

fn is_narrow(ty: &GType) -> bool {
    ty.int_width().is_some_and(|width| intwidth::is_narrow(width.name))
}

fn unsupported_small_int_arg(ty: &GType) -> bool {
    is_narrow(ty) || ty.name == "array_u16"
}

fn contains_unsupported_small_int_arg(ty: &GType) -> bool {
    unsupported_small_int_arg(ty) || ty.args.iter().any(contains_unsupported_small_int_arg)
}

fn small_int_unsupported(line: i64, col: i64, msg: impl Into<String>, fix: &str) -> PruvError {
    terr(line, col, "small_int_unsupported", msg, fix)
}

fn small_int_op(line: i64, col: i64, op: &str) -> PruvError {
    terr(line, col, "small_int_op", format!("operator `{}` is unsupported on integer types other than i64", op), "use i64(x) to widen the operand")
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
    /// P7-1 B: the result may borrow from the arguments (`from`), so no argument is a call-duration `[T; N]` borrow
    holds_args: bool,
}

struct Checker {
    funcs: HashMap<String, Rc<FuncInfo>>,
    decls: HashMap<String, TypeDecl>, // first declaration
    tt: TypeTable,
    cur_name: String,
    cur_ret: GType,
    borrowed_result: bool,
    place_functions: crate::borrowed::PlaceFunctions,
    try_count: usize,
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
    array_replace: Option<String>,
    /// G-DB16-1: array binders of the enclosing match arms that may be moved (consuming match on an owned local)
    match_movable: HashSet<String>,
    /// G-DB16-1: every binder of an enclosing match arm (a view, never an owner)
    arm_binders: HashSet<String>,
    /// P9a-5: per open value `match`: the arm type (fixed by `want` or the first arm that is not a bare literal),
    /// the arms whose bare literal waits for it, and the number of arms checked so far.
    value_arms: Vec<(Option<GType>, Vec<usize>, usize)>,
    /// P9a-6: counter for the fresh field names of a lowered pattern match
    pat_tmp: usize,
    collection_records: std::cell::RefCell<HashSet<String>>,
    nested_records: HashSet<String>,
    holder_records: HashSet<String>,
    read_context: Option<usize>,
    statement_call: Option<usize>,
    borrowed_text: Option<usize>,
    /// R-6g9-3: the R-6g8-3 static text locals of the function being checked (they need no `alloc`)
    static_lets: HashSet<String>,
    /// R-6g9-3: positions of the `[]` literals a view position accepted; any other `[]` is rejected
    empty_views: HashSet<(i64, i64)>,
    /// R-6g9-3: the string literal checked next is a static text local's value
    static_lit: bool,
    map_roots: Vec<(String, Vec<String>, Vec<String>)>,
    map_iterations: Vec<(String, Vec<String>)>,
    /// P4: collections iterated by an enclosing `for x in v` (read-only while the loop runs).
    for_holds: Vec<(String, Vec<String>)>,
    /// P4: enclosing range `for` loops: (position of the synthesized step, loop variable).
    for_steps: Vec<(i64, i64, String)>,
    /// P5: enclosing while/for loops: (label, map_roots depth, map_iterations depth) at loop entry.
    loop_frames: Vec<(Option<String>, usize, usize, Option<(i64, i64, String)>)>,
    // ---- G9 A0 metadata ----
    /// recursive type SCCs (names in declaration order), SCC-number order
    type_sccs: Vec<Vec<String>>,
    recursive_vec_elements: HashSet<String>,
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
    /// P7-1 R1: type parameters (owner, index) compared with `==` in a contract
    eq_tparams: HashSet<(String, usize)>,
    /// P7-1 R1: (call line, call col, call targs, callee name) of every user call with type arguments, contracts included
    targ_log: Vec<(i64, i64, Vec<GType>, String)>,
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
    /// LG6: checking the operands of a subview(...) argument
    in_view: bool,
    /// P7-1 B: the next user_arg_slices is a direct internal call argument, which may borrow a [T; N] as [T]
    fixed_borrow: bool,
}

/// Reject [u16] and fixed-length arrays at explicit boundaries even when a struct or enum hides them.
fn check_u16_boundary(prog: &Program, public: impl Fn(usize) -> bool) -> Res<()> {
    fn contains(ty: &GType, decls: &HashMap<&str, &TypeDecl>, seen: &mut HashSet<String>, hit: fn(&GType) -> bool) -> bool {
        hit(ty)
            || ty.args.iter().any(|arg| contains(arg, decls, seen, hit))
            || (ty.is_user_type() && seen.insert(ty.name.clone())
                && decls.get(ty.name.as_str()).is_some_and(|decl| {
                    decl_fields(decl).iter().any(|(_, field)| contains(&GType::from_key(&field.ty), decls, seen, hit))
                }))
    }

    let decls = prog.types.iter().map(|d| (d.name(), d)).collect();
    for (i, f) in prog.functions.iter().enumerate() {
        if f.foreign.is_none() && !public(i) {
            continue;
        }
        let sig = || std::iter::once(&f.ret).chain(f.params.iter().map(|p| &p.ty));
        let mut seen = HashSet::new();
        if sig().any(|ty| contains(&GType::from_key(ty), &decls, &mut seen, |t| t.name == "array_u16")) {
            return Err(small_int_unsupported(
                f.line, f.col, "[u16] at the host boundary is bundle P12", "use a host-supported type",
            ));
        }
        let mut seen = HashSet::new();
        if sig().any(|ty| contains(&GType::from_key(ty), &decls, &mut seen, GType::is_fixed)) {
            // P7-1: no host or foreign ABI carries [T; N] (directly or inside a struct field)
            return Err(terr(f.line, f.col, "fixed_array_position",
                format!("{} has a fixed-length array in its signature, which a pub, host entry or extern function cannot have", f.name),
                "keep the fixed-length array inside the Pruv code: pass a buffer ([T]) or its elements at the boundary"));
        }
    }
    Ok(())
}

/// G4 A0: `check_with_externs` for an explicit-mode program: resolves the names of the whole program to identities first
/// (modules::resolve), then runs the same checker plus the host-entry signature rules.
pub fn check_explicit(
    prog: &mut Program,
    g4: &mut crate::modules::G4,
    externs: &[String],
) -> Result<TypeTable, PruvError> {
    crate::modules::resolve(prog, g4)?;
    crate::methods::resolve(prog)?; // P9b: method calls become calls of the impl functions
    crate::modules::contract_methods(prog, g4)?;
    expand_match_forms(prog)?;
    // P9b-2: a method is never a host entry (R-6x-5), so its signature is not a host boundary
    let methods: HashSet<&str> = prog.methods.iter().map(|m| m.func.as_str()).collect();
    check_u16_boundary(prog, |i| {
        g4.fn_meta.get(i).is_some_and(|m| m.is_pub || m.host_entry) && !methods.contains(prog.functions[i].name.as_str())
    })?;
    let src_names: Vec<String> = g4.fn_meta.iter().map(|m| m.src_name.clone()).collect();
    let hosts: Vec<bool> = g4.fn_meta.iter().map(|m| m.host_entry).collect();
    crate::borrowed::prepare(prog, &hosts)?;
    let mut c = Checker::new(prog, externs, &src_names)?;
    prog.uses_inout = Some(c.uses_inout); // G5 A0: lets the staging guard skip its walk for inout-free programs
    c.explicit = true;
    c.host_entry = g4.fn_meta.iter().map(|m| m.host_entry).collect();
    c.run(prog)?;
    crate::borrowed::check(prog, &c.place_functions)?;
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
    crate::methods::resolve(prog)?; // P9b: method calls become calls of the impl functions
    let methods: HashSet<&str> = prog.methods.iter().map(|m| m.func.as_str()).collect();
    check_u16_boundary(prog, |i| {
        prog.explicit.as_ref().and_then(|e| e.fns.get(i)).is_some_and(|m| m.is_pub || m.host_entry)
            && !methods.contains(prog.functions[i].name.as_str())
    })?;
    expand_match_forms(prog)?;
    crate::borrowed::prepare(prog, &[])?;
    let mut c = Checker::new(prog, externs, &[])?;
    prog.uses_inout = Some(c.uses_inout); // G5 A0: lets the staging guard skip its walk for inout-free programs
    c.run(prog)?;
    crate::borrowed::check(prog, &c.place_functions)?;
    prog.contract_vec_slice = c.contract_vec_slice;
    prog.functions.retain(|f| !f.pred);
    Ok(c.tt)
}

/// P8: completes every `if let` and `let ... else` into a full match before any pass reads the program: arm 0 is the
/// written pattern, and every other variant of the scrutinee type gets an arm whose body is a copy of `els` (the
/// else block) and whose binders are fresh unused names. `els` is emptied, so later passes see an ordinary match and
/// the form only steers how the Rust backend prints it. A `let ... else` block must diverge
/// (type/let_else_fallthrough). The binder count of a synthesized `Ok` arm is fixed by `match_arms` once the payload
/// type is known (`Ok(())` binds nothing).
pub(crate) fn expand_match_forms(prog: &mut Program) -> Result<(), PruvError> {
    let variants: HashMap<String, Vec<(String, usize)>> = prog.types.iter().filter_map(|d| match d {
        TypeDecl::Enum(e) => Some((e.name.clone(), e.variants.iter().map(|v| (v.name.clone(), v.fields.len())).collect())),
        TypeDecl::Struct(_) => None,
    }).collect();
    let mut n = 0usize;
    for f in prog.functions.iter_mut() {
        expand_forms_block(&mut f.body, &variants, &mut n)?;
    }
    Ok(())
}

/// P8: the lets that destructure the arm temporary `tmp` of tuple type `g` by the patterns `ps`, in source order.
/// Components are scalars (integers, `bool`, `char`) or tuples of them: an owned or struct component would need a
/// move out of the matched value (type/tuple_pattern_owned).
fn lower_arm_pat(tmp: &str, ps: &[TuplePat], g: &GType, line: i64, col: i64, out: &mut Vec<Stmt>) -> Result<(), PruvError> {
    let comps = if g.args.is_empty() { crate::gtype::tuple_components(&g.name) } else { None };
    let Some(comps) = comps else {
        return Err(terr(line, col, "tuple_pattern_type", format!("a tuple pattern needs a tuple payload, found {}", tn(g)),
            "bind the payload with a name"));
    };
    if comps.len() != ps.len() {
        return Err(terr(line, col, "tuple_pattern_type",
            format!("the pattern has {} components but {} has {}", ps.len(), tn(g), comps.len()),
            "write one binder or `_` per component"));
    }
    for (i, (p, k)) in ps.iter().zip(comps.iter()).enumerate() {
        let read = |l: i64, c: i64| Expr::FieldAccess(FieldAccess {
            line: l, col: c, ty: None, field: i.to_string(),
            base: Box::new(Expr::Var(Var { line: l, col: c, name: tmp.to_string(), ty: None })),
        });
        let kg = GType::from_key(k);
        let scalar = kg.args.is_empty() && (crate::intwidth::of(&kg.name).is_some() || kg.name == "bool");
        match p {
            TuplePat::Wild => {}
            TuplePat::Bind(l, c, name, mutable) => {
                if !scalar {
                    return Err(terr(*l, *c, "tuple_pattern_owned",
                        format!("the tuple pattern component {} has type {}; a match pattern binds only integer, bool \
                                 and char components", name, tn(&kg)),
                        "bind the whole tuple and destructure it with a let"));
                }
                out.push(Stmt::Let(Let { borrowed: false, line: *l, col: *c, name: name.clone(), mutable: *mutable,
                    ty: k.clone(), value: read(*l, *c), refine: None }));
            }
            TuplePat::Tuple(l, c, inner) => {
                let t = format!("{}_{}", tmp, i);
                out.push(Stmt::Let(Let { borrowed: false, line: *l, col: *c, name: t.clone(), mutable: false,
                    ty: k.clone(), value: read(*l, *c), refine: None }));
                lower_arm_pat(&t, inner, &kg, *l, *c, out)?;
            }
        }
    }
    Ok(())
}

/// P8b: the shape of a slice pattern arm (parser `slice_arm`): `None` for a constructor arm, `Some(None)` for an
/// exact length `[a, b]`, `Some(Some(""))` for an unnamed rest `[a, ..]` and `Some(Some(name))` for `[a, name @ ..]`.
pub(crate) fn slice_arm_shape(ctor: &str) -> Option<Option<&str>> {
    match ctor {
        "[]" => Some(None),
        _ => ctor.strip_prefix("[..").and_then(|r| r.strip_suffix(']')).map(Some),
    }
}

/// Whether control never leaves the end of the block (P8 `let ... else`).
fn diverges(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        Some(Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_)) => true,
        Some(Stmt::If(i)) => diverges(&i.then) && diverges(&i.els),
        // an expanded `if let`/`let else` holds its else block in the complement arms and has an empty els
        Some(Stmt::Match(m)) => m.arms.iter().all(|a| diverges(&a.body))
            && (m.form == MatchForm::Match || diverges(&m.els) || (m.els.is_empty() && m.arms.len() > 1)),
        _ => false,
    }
}

fn expand_forms_block(
    stmts: &mut [Stmt],
    variants: &HashMap<String, Vec<(String, usize)>>,
    n: &mut usize,
) -> Result<(), PruvError> {
    for st in stmts.iter_mut() {
        match st {
            Stmt::If(s) => {
                expand_forms_block(&mut s.then, variants, n)?;
                expand_forms_block(&mut s.els, variants, n)?;
            }
            Stmt::While(s) => expand_forms_block(&mut s.body, variants, n)?,
            Stmt::ForMap(s) => expand_forms_block(&mut s.body, variants, n)?,
            Stmt::Match(m) => {
                for a in m.arms.iter_mut() {
                    expand_forms_block(&mut a.body, variants, n)?;
                }
                expand_forms_block(&mut m.els, variants, n)?;
                if m.form == MatchForm::Match || m.arms.len() != 1 {
                    continue;
                }
                if m.form == MatchForm::LetElse && !diverges(&m.els) {
                    let (l, c) = m.els.last().map(Stmt::position).unwrap_or((m.line, m.col));
                    return Err(terr(l, c, "let_else_fallthrough",
                        "the else block of `let ... else` must not reach its end",
                        "end the else block with return, break or continue"));
                }
                let a0 = &m.arms[0];
                let others: Vec<(String, usize)> = match (&a0.enum_name, a0.ctor.as_str()) {
                    (None, "Some") => vec![("None".into(), 0)],
                    (None, "None") => vec![("Some".into(), 1)],
                    (None, "Ok") => vec![("Err".into(), 1)],
                    (None, "Err") => vec![("Ok".into(), 1)],
                    (Some(en), ctor) => match variants.get(en) {
                        Some(vs) if vs.iter().any(|(v, _)| v == ctor) => {
                            vs.iter().filter(|(v, _)| v != ctor).cloned().collect()
                        }
                        _ => continue, // match_arms reports the unknown enum or variant
                    },
                    _ => continue,
                };
                if others.is_empty() && !m.els.is_empty() {
                    let (l, c) = m.els[0].position();
                    if m.form == MatchForm::WhileLet {
                        return Err(terr(l, c, "irrefutable_pattern",
                            "the `while let` pattern matches every value, so the loop never ends by a failed match",
                            "use a `while` loop and bind the payload with `match` or a `let`"));
                    }
                    return Err(terr(l, c, "irrefutable_pattern",
                        "the pattern matches every value, so the else block never runs",
                        "bind the payload with `match` or drop the else block"));
                }
                let els = std::mem::take(&mut m.els);
                let (line, col, enum_name) = (a0.line, a0.col, a0.enum_name.clone());
                for (ctor, k) in others {
                    *n += 1;
                    let binders = (0..k).map(|i| format!("pruv_else_{}_{}", n, i)).collect();
                    m.arms.push(MatchArm { line, col, enum_name: enum_name.clone(), ctor, binders,
                        body: els.clone(), pats: Vec::new(), pat_lets: 0 });
                }
            }
            _ => {}
        }
    }
    Ok(())
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

/// LG13/LG14: the kid-index path (crate::refine::kids order) of the first `?` of e in evaluation order (operands left
/// to right, then the node itself) with the paths of the operands to bind to fresh temporaries first, or None when e has
/// no `?` that can be lowered. An operand evaluated before the `?` that is not pure (a call, a constructor, a move, an
/// index, arithmetic) is bound to a temporary when the whole operand holds no `inout` argument and no quantifier and
/// moves no binding mentioned elsewhere in e (LG14, G-LG13-1); its type is checked by Checker::try_temps. A `?` only
/// in the right operand of && / || gives the path of that operator node instead (LG15, G-LG13-2; lowered by
/// Checker::try_short) unless the statement passes `inout` or the node holds a quantifier or a subview or moves a name
/// mentioned outside it (or mentions one moved outside it). A `?` inside a quantifier, after an operand that cannot be
/// bound, or whose operand passes `inout` while the statement reads a binding outside that operand stays for
/// check_try_position. Python `find_try`.
fn find_try(e: &Expr) -> Option<(Vec<usize>, Vec<Vec<usize>>)> {
    fn is_try(e: &Expr) -> bool {
        matches!(e, Expr::Unary(u) if u.op == "?")
    }
    fn any(e: &Expr, f: &dyn Fn(&Expr) -> bool) -> bool {
        f(e) || crate::refine::kids(e).into_iter().any(|k| any(k, f))
    }
    fn reads(e: &Expr) -> usize {
        usize::from(matches!(e, Expr::Var(_) | Expr::Move(_))) + crate::refine::kids(e).into_iter().map(reads).sum::<usize>()
    }
    fn mentions(e: &Expr, name: &str) -> usize {
        let own = match e {
            Expr::Var(v) => v.name == name,
            Expr::Move(m) => m.name == name,
            _ => false,
        };
        usize::from(own) + crate::refine::kids(e).into_iter().map(|k| mentions(k, name)).sum::<usize>()
    }
    fn moves_of(e: &Expr, name: &str) -> usize {
        usize::from(matches!(e, Expr::Move(m) if m.name == name))
            + crate::refine::kids(e).into_iter().map(|k| moves_of(k, name)).sum::<usize>()
    }
    // A node that may run after a later `?` without changing what happens (Python `_try_pure`).
    fn pure(e: &Expr) -> bool {
        match e {
            Expr::Var(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::FieldAccess(_) => true,
            Expr::Unary(u) => u.op == "!",
            Expr::Binary(b) => matches!(b.op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" | "==>"),
            _ => false,
        }
    }
    fn note(e: &Expr, seen: &mut bool) {
        if !pure(e) {
            *seen = true;
        }
    }
    // Err(()) = no lowerable `?`; Ok(false) = none here, keep going; Ok(true) = found.
    fn visit(e: &Expr, root: &Expr, seen: &mut bool, path: &mut Vec<usize>) -> Result<bool, ()> {
        if let Expr::Quant(_) = e {
            if any(e, &is_try) {
                return Err(());
            }
            *seen = true; // G-LG13-7: a quantifier is not code that may move after a `?`
            return Ok(false);
        }
        if let Expr::Binary(b) = e {
            if matches!(b.op.as_str(), "&&" | "||" | "==>") {
                let before = *seen;
                path.push(0);
                if visit(&b.left, root, seen, path)? {
                    return Ok(true);
                }
                path.pop();
                if any(&b.right, &is_try) {
                    if b.op == "==>" {
                        // ==> is contract-only; lowering it would hide type/contract_only
                        return Err(());
                    }
                    // LG15 (G-LG13-2): the whole operator is lowered to a nested if (Checker::try_short), so it runs
                    // before the rest of the statement; it must not pass inout, hold a quantifier or a subview, or
                    // move a name the rest of the statement mentions (or mention one it moves)
                    if any(root, &|x| matches!(x, Expr::InoutArg(_)))
                        || any(e, &|x| matches!(x, Expr::Quant(_)) || matches!(x, Expr::Call(c) if c.name == "subview")) {
                        return Err(());
                    }
                    fn bad(m: &Expr, e: &Expr, root: &Expr) -> bool {
                        let hit = match m {
                            Expr::Move(x) => mentions(root, &x.name) > mentions(e, &x.name)
                                || moves_of(root, &x.name) > moves_of(e, &x.name),
                            Expr::Var(x) => moves_of(root, &x.name) > moves_of(e, &x.name),
                            _ => false,
                        };
                        hit || crate::refine::kids(m).into_iter().any(|k| bad(k, e, root))
                    }
                    if bad(e, e, root) {
                        return Err(());
                    }
                    *seen = before;
                    return Ok(true);
                }
                fn all(e: &Expr, seen: &mut bool) {
                    note(e, seen);
                    for k in crate::refine::kids(e) {
                        all(k, seen);
                    }
                }
                all(&b.right, seen);
                note(e, seen);
                return Ok(false);
            }
        }
        if let Expr::Unary(u) = e {
            if u.op == "?" {
                let before = *seen;
                path.push(0);
                if visit(&u.operand, root, seen, path)? {
                    return Ok(true);
                }
                path.pop();
                if any(&u.operand, &|x| matches!(x, Expr::InoutArg(_))) && reads(root) > reads(&u.operand) {
                    return Err(());
                }
                *seen = before;
                return Ok(true);
            }
        }
        for (i, k) in crate::refine::kids(e).into_iter().enumerate() {
            path.push(i);
            if visit(k, root, seen, path)? {
                return Ok(true);
            }
            path.pop();
        }
        note(e, seen);
        Ok(false)
    }
    // The operands evaluated before the `?` at path that are not wholly pure, in evaluation order.
    fn temps(root: &Expr, path: &[usize]) -> Result<Vec<Vec<usize>>, ()> {
        fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
            out.push(e);
            for k in crate::refine::kids(e) {
                walk(k, out);
            }
        }
        // LG18: some call of root holds both nodes a and b (by identity): their arguments share one borrow track
        fn shared_call(root: &Expr, a: &Expr, b: &Expr) -> bool {
            let mut all = Vec::new();
            walk(root, &mut all);
            all.into_iter().any(|x| {
                if !matches!(x, Expr::Call(_)) {
                    return false;
                }
                let mut inner = Vec::new();
                walk(x, &mut inner);
                inner.iter().any(|y| std::ptr::eq(*y, a)) && inner.iter().any(|y| std::ptr::eq(*y, b))
            })
        }
        let mut out = Vec::new();
        let mut all = Vec::new();
        walk(root, &mut all);
        let mut inout: Vec<String> = Vec::new();
        for x in &all {
            if let Expr::InoutArg(a) = x {
                match field_place(&a.expr).0 {
                    Expr::Var(v) => inout.push(v.name.clone()),
                    _ => return Err(()),
                }
            }
        }
        let touches = |e: &Expr| inout.iter().any(|n| mentions(e, n) > 0);
        // LG18 (G-LG14-2 a): with inout in the statement, neither the temporaries nor any `?` operand may hold an
        // inout argument or mention an inout root (that would be inout_context unlowered); Checker::try_temps rejects
        // a temporary or `?` operand with an effect call (effect_before)
        if !inout.is_empty() && all.iter().any(|x| matches!(x, Expr::Unary(u) if u.op == "?" && touches(&u.operand))) {
            return Err(());
        }
        let mut node = root;
        for (depth, &i) in path.iter().enumerate() {
            let kids = crate::refine::kids(node);
            for (j, k) in kids.iter().enumerate().take(i) {
                fn all_pure(e: &Expr) -> bool {
                    pure(e) && crate::refine::kids(e).into_iter().all(all_pure)
                }
                if all_pure(k) || matches!(k, Expr::Move(m) if m.name.starts_with(TRY_TEMP)) {
                    continue; // LG18: `move __pruv_try_tN` is an already bound temporary passed to take
                }
                if any(k, &|x| matches!(x, Expr::Quant(_)) || matches!(x, Expr::Call(c) if c.name == "subview")) {
                    return Err(());
                }
                if !inout.is_empty() && (any(k, &|x| matches!(x, Expr::InoutArg(_))) || touches(k)) {
                    return Err(());
                }
                fn names<'a>(e: &'a Expr, out: &mut Vec<(&'a str, bool)>) {
                    match e {
                        Expr::Move(m) => out.push((&m.name, true)),
                        Expr::Var(v) => out.push((&v.name, false)),
                        _ => {}
                    }
                    for k in crate::refine::kids(e) {
                        names(k, out);
                    }
                }
                let mut named = Vec::new();
                names(k, &mut named);
                for &(m, moved) in &named {
                    if moved && (mentions(root, m) > mentions(k, m) || moves_of(root, m) > moves_of(k, m)) {
                        return Err(()); // a moved name used again
                    }
                    // LG18 (G-LG14-2 b): a borrow bound early keeps its order before the later move; only a move in
                    // the same call (one borrow track, foreign_borrow unlowered) must stay rejected
                    if !moved && moves_of(root, m) > moves_of(k, m)
                        && all.iter().any(|x| matches!(x, Expr::Move(y) if y.name == m) && shared_call(root, k, x)) {
                        return Err(());
                    }
                }
                let mut p = path[..depth].to_vec();
                p.push(j);
                out.push(p);
            }
            node = kids[i];
        }
        Ok(out)
    }
    let mut path = Vec::new();
    let mut seen = false;
    match visit(e, e, &mut seen, &mut path) {
        Ok(true) => {
            // seen now holds whether a node evaluated before the `?` is not pure (restored in the `?` branch)
            if seen {
                return Some((path.clone(), temps(e, &path).ok()?));
            }
            // LG21 (G-LG18-1): with nothing evaluated before the `?` nothing is bound, but a `?` operand still may not
            // mention the root of an inout argument outside it (inout_context unlowered); an inout argument inside the
            // operand itself (`set(inout x)?`) stays as before. Python `find_try`.
            fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
                out.push(e);
                for k in crate::refine::kids(e) {
                    walk(k, out);
                }
            }
            let mut all = Vec::new();
            walk(e, &mut all);
            for t in &all {
                let Expr::Unary(u) = t else { continue };
                if u.op != "?" {
                    continue;
                }
                let mut inner = Vec::new();
                walk(&u.operand, &mut inner);
                let mut outs: Vec<&str> = Vec::new();
                for x in &all {
                    if let Expr::InoutArg(a) = x {
                        if inner.iter().any(|y| std::ptr::eq(*y, *x)) {
                            continue;
                        }
                        match field_place(&a.expr).0 {
                            Expr::Var(v) => outs.push(v.name.as_str()),
                            _ => return None,
                        }
                    }
                }
                if outs.iter().any(|n| mentions(&u.operand, n) > 0) {
                    return None;
                }
            }
            Some((path, Vec::new()))
        }
        _ => None,
    }
}

const TRY_TEMP: &str = "__pruv_try_t";

fn has_try(e: &Expr) -> bool {
    matches!(e, Expr::Unary(u) if u.op == "?") || crate::refine::kids(e).into_iter().any(has_try)
}

/// LG13: kid i of e, in crate::refine::kids order (no quantifier: find_try never enters one).
fn kid_mut(e: &mut Expr, i: usize) -> &mut Expr {
    match e {
        Expr::InoutArg(x) => &mut x.expr,
        Expr::Unary(x) => &mut x.operand,
        Expr::Binary(x) => if i == 0 { &mut x.left } else { &mut x.right },
        Expr::Index(x) => if i == 0 { &mut x.array } else { &mut x.index },
        Expr::Ctor(x) => x.arg.as_mut().unwrap(),
        Expr::Call(x) => &mut x.args[i],
        Expr::StructLit(x) => &mut x.fields[i].value,
        Expr::EnumCtor(x) => &mut x.args[i],
        Expr::FieldAccess(x) => &mut x.base,
        Expr::IsVariant(x) => &mut x.value,
        Expr::VariantField(x) => &mut x.value,
        _ => unreachable!(),
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

/// Exact place identity, independent of source positions and type annotations.
pub(crate) fn same_place(a: &Expr, b: &Expr) -> bool {
    let (ar, ap) = field_place(a);
    let (br, bp) = field_place(b);
    matches!((ar, br), (Expr::Var(a), Expr::Var(b)) if a.name == b.name) && ap == bp
}

pub fn walk_assignments(stmts: &[Stmt], visit: &mut impl FnMut(&str, &[String])) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(a) => visit(&a.name, &[]),
            Stmt::FieldAssign(a) => visit(&a.name, &a.fields),
            Stmt::If(s) => {
                walk_assignments(&s.then, visit);
                walk_assignments(&s.els, visit);
            }
            Stmt::While(s) => walk_assignments(&s.body, visit),
            Stmt::ForMap(s) => walk_assignments(&s.body, visit),
            Stmt::Match(s) => {
                for arm in &s.arms {
                    walk_assignments(&arm.body, visit);
                }
            }
            _ => {}
        }
    }
}

pub fn vec_resize_builtin(name: &str) -> bool {
    matches!(name, "vec_push" | "vec_pop" | "vec_clear" | "vec_extend_from_slice")
}

pub fn array_grow_builtin(name: &str) -> bool {
    matches!(name, "arr_reserve" | "arr_push" | "arr_append")
}

/// LB6 SX (R-6g5-5): the most values one arr_push appends.
pub const ARR_PUSH_MAX: usize = 16;

/// Array and vector inout parameters whose storage may change, including through forwarding.
pub fn resize_params(prog: &Program) -> HashSet<(String, usize)> {
    resizing_params(prog, false)
}

/// Capacity-only changes do not affect length framing.
pub fn length_changing_params(prog: &Program) -> HashSet<(String, usize)> {
    resizing_params(prog, true)
}

// Resizing through a whole-place binding changes the storage of its root.
fn walk_resize_uses<'a>(
    body: &'a [Stmt],
    aliases: &HashMap<&'a str, Option<&'a str>>,
    visit: &mut impl FnMut(&str, Option<&str>, Option<(&Call, usize)>),
) {
    fn root(e: &Expr) -> Option<&str> {
        match e {
            Expr::Var(v) => Some(&v.name),
            Expr::Call(c) if PlaceIf::matches(c) => {
                let left = root(&c.args[1])?;
                (root(&c.args[2]) == Some(left)).then_some(left)
            }
            _ => None,
        }
    }
    let mut aliases = aliases.clone();
    for st in body {
        let canonical = |name: &'a str| aliases.get(name).copied().unwrap_or(Some(name));
        if let Stmt::Assign(a) = st {
            visit(&a.name, canonical(&a.name), None);
        }
        let mut expressions = Vec::new();
        let mut blocks = Vec::new();
        crate::borrowed::walk_statement(st, &mut |e| expressions.push(e),
            &mut |_, _, _, _| {}, &mut |block| blocks.push(block));
        for e in expressions {
            crate::foreign_shim::walk_expr(e, &mut |node| {
                let crate::foreign_shim::Node::Expr(Expr::Call(call)) = node else { return; };
                for (i, arg) in call.args.iter().enumerate() {
                    let Expr::InoutArg(arg) = arg else { continue; };
                    let Expr::Var(v) = &*arg.expr else { continue; };
                    visit(&v.name, aliases.get(v.name.as_str()).copied().unwrap_or(Some(&v.name)), Some((call, i)));
                }
            });
        }
        for block in blocks {
            walk_resize_uses(block, &aliases, visit);
        }
        if let Stmt::Let(l) = st {
            if let Expr::InoutArg(arg) = &l.value {
                aliases.insert(&l.name, root(&arg.expr).and_then(canonical));
            }
        }
    }
}

fn resizing_params(prog: &Program, length_only: bool) -> HashSet<(String, usize)> {
    let mut resizes = HashSet::new();
    let mut forwarded = HashMap::<_, Vec<_>>::new();
    for f in &prog.functions {
        let params: HashMap<_, _> = f.params.iter().enumerate()
            .filter(|(_, p)| p.mode_pos.is_some() && (GType::from_key(&p.ty).is_array() || GType::from_key(&p.ty).name == "vec"))
            .map(|(i, p)| (p.name.as_str(), i))
            .collect();
        if params.is_empty() { continue; }
        walk_resize_uses(&f.body, &HashMap::new(), &mut |_, root, call| {
            let Some(index) = root.and_then(|name| params.get(name)) else { return; };
            let source = (f.name.clone(), *index);
            match call {
                None => { resizes.insert(source); }
                Some((call, i)) => {
                    if vec_resize_builtin(&call.name) || (array_grow_builtin(&call.name) && (call.name != "arr_reserve" || !length_only)) {
                        resizes.insert(source);
                    } else {
                        forwarded.entry((call.name.clone(), i)).or_default().push(source);
                    }
                }
            }
        });
    }
    let mut pending: Vec<_> = resizes.iter().cloned().collect();
    while let Some(target) = pending.pop() {
        for source in forwarded.get(&target).into_iter().flatten() {
            if resizes.insert(source.clone()) {
                pending.push(source.clone());
            }
        }
    }
    resizes
}

/// Bare array arguments to resizing functions need owned, addressable storage.
pub(crate) fn resize_slot_names(f: &Function, resizes: &HashSet<(String, usize)>) -> HashSet<String> {
    let mut resize_slots = HashSet::new();
    walk_resize_uses(&f.body, &HashMap::new(), &mut |name, root, call| {
        let Some((call, i)) = call else { return; };
        let Expr::InoutArg(arg) = &call.args[i] else { unreachable!() };
        if arg.ty.as_deref().is_some_and(crate::typeinfo::is_array_name)
            && (array_grow_builtin(&call.name) || resizes.contains(&(call.name.clone(), i)))
        {
            resize_slots.insert(name.to_owned());
            resize_slots.extend(root.map(str::to_owned));
        }
    });
    resize_slots
}

pub(crate) fn mentions(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Var(x) if x.name == name => return true,
        Expr::Move(x) if x.name == name => return true,
        _ => {}
    }
    crate::refine::kids(e).into_iter().any(|k| mentions(k, name))
}

pub(crate) fn paths_overlap(a: &[String], b: &[String]) -> bool {
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
    if let Expr::Call(c) = e {
        if crate::ast::ValueIf::matches(c) {
            // P9a-3: only one arm runs
            inout_statement_calls(&c.args[0], conditional, calls);
            inout_statement_calls(&c.args[1], true, calls);
            inout_statement_calls(&c.args[2], true, calls);
            return;
        }
        if crate::ast::ValueMatch::matches(c) {
            // P9a-5: only one arm runs
            inout_statement_calls(&c.args[0], conditional, calls);
            for arm in &c.args[1..] {
                inout_statement_calls(arm, true, calls);
            }
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

fn is_concrete_int_buffer(ty: &GType) -> bool {
    ty.args.is_empty() && crate::typeinfo::array_elem_name(&ty.name).and_then(intwidth::of).is_some()
}

/// Type of an element read from / stored into the array type `at`: T for [T]; bool for [bool]; else i64.
/// P4: the lowered `for` loop inside a parser `if true { ... }` wrapper (parser `for_loop`).
pub(crate) fn for_loop_of(s: &If) -> Option<&While> {
    match (s.then.last(), &s.cond, s.els.is_empty()) {
        (Some(Stmt::While(w)), Expr::BoolLit(b), true) if b.value && w.for_head.is_some() => Some(w),
        _ => None,
    }
}

/// P4: the iterated place of a lowered slice `for`: its first body statement reads `v[K]` or `vec_get(v, K)`.
pub(crate) fn for_element_read(body: &[Stmt]) -> (&Expr, &Expr) {
    match body.first() {
        Some(Stmt::Let(l)) => match &l.value {
            Expr::Index(i) => (&i.array, &i.index),
            Expr::Call(c) if c.args.len() == 2 => (&c.args[0], &c.args[1]),
            _ => unreachable!("lowered for element read"),
        },
        _ => unreachable!("lowered for element binder"),
    }
}

fn for_collection(body: &[Stmt]) -> (String, Vec<String>) {
    let (place, _) = for_element_read(body);
    let (root, path) = field_place(place);
    let Expr::Var(v) = root else { unreachable!("parser checks the for place") };
    (v.name.clone(), path)
}

/// An untyped integer literal (optionally negated). Char, byte and constant uses carry their own type (P7).
fn is_int_literal(e: &Expr) -> bool {
    match e {
        Expr::IntLit(_) => bare_lit(e),
        Expr::Unary(u) => u.op == "-" && bare_lit(&u.operand),
        _ => false,
    }
}

fn elem_type(at: &GType) -> GType {
    if let Some((e, _)) = at.fixed_parts() {
        return e.clone(); // P7-1: [T; N] reads exactly T
    }
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
            if matches!(name.as_str(), "len" | "alloc" | "copy" | "print" | "is_char_boundary" | "substr" | "utf8_valid" | "utf8_valid_span" | "str_from_utf8" | "truncate" | "eq_range" | "find_byte" | "resize" | "copy_range" | "copy_within" | "arr_reserve" | "arr_push" | "arr_append" | "subview")
                || intwidth::of(src_names.get(fi).map_or(name.as_str(), String::as_str)).is_some()
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
                    holds_args: f.borrow_from.is_some() || f.borrowed_result,
                }),
            );
        }
        Ok(Checker {
            funcs,
            decls: HashMap::new(),
            tt: TypeTable::default(),
            cur_name: String::new(),
            cur_ret: unitt(),
            borrowed_result: false,
            place_functions: HashMap::new(),
            try_count: 0,
            used_effects: BTreeSet::new(),
            alloc_want: None,
            ctx_names: HashSet::new(),
            ctx_generic: false,
            elem_params: Vec::new(),
            arg_decls: HashMap::new(),
            externs: externs.to_vec(),
            takes,
            field_move: None,
            array_replace: None,
            match_movable: HashSet::new(),
            arm_binders: HashSet::new(),
            value_arms: Vec::new(),
            pat_tmp: 0,
            collection_records: Default::default(),
            nested_records: HashSet::new(),
            holder_records: HashSet::new(),
            read_context: None,
            statement_call: None,
            borrowed_text: None,
            static_lets: HashSet::new(),
            empty_views: HashSet::new(),
            static_lit: false,
            map_roots: Vec::new(),
            map_iterations: Vec::new(),
            for_holds: Vec::new(),
            for_steps: Vec::new(),
            loop_frames: Vec::new(),
            type_sccs: Vec::new(),
            recursive_vec_elements: HashSet::new(),
            boxed: Vec::new(),
            boxed_decls: HashSet::new(),
            extended: HashSet::new(),
            buffered: HashSet::new(),
            fn_sccs: Vec::new(),
            call_log: Vec::new(),
            eq_tparams: HashSet::new(),
            targ_log: Vec::new(),
            active_roots: Vec::new(),
            map_read_paths: Vec::new(),
            match_paths: Vec::new(),
            cur_cost: false,
            s5_errors: Vec::new(),
            par_groups: Vec::new(),
            in_view: false,
            fixed_borrow: false,
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
        for f in &prog.functions {
            for e in f.requires.iter().chain(&f.ensures).chain(&f.decreases).chain(&f.cost) {
                Self::check_try_position(e, e.line(), e.col())?;
            }
        }
        crate::refine::expand(prog)?; // STAGE1 A: validate refinement annotations, then expand them into contracts/asserts
        self.declare_types(&prog.types)?;
        self.normalize_types(prog); // G6: Vec/HashMap/String become catalog types unless the program declares them
        // `decls` holds clones taken before normalization (Python shares the nodes): refresh them
        for d in &prog.types {
            self.decls.insert(d.name().to_string(), d.clone());
        }
        self.resolve_places(prog)?;
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
        let graph = self.type_graph(&prog.types); // also identifies Vec edges for field validation
        if let Err(e) = &graph {
            if e.code == "recursive_generic_type" {
                return graph;
            }
        }
        self.resolve_types(prog)?;
        graph?;
        self.inhabitance(&prog.types)?; // G9 A0: every declaration needs a finite value
        self.compute_extended(&prog.types);
        self.check_extended_uses(prog)?;
        self.check_modes(prog)?; // G5 A0: parameter modes, before host entries and bodies
        if self.explicit {
            self.check_entries(prog)?; // G4 A0: host-entry signatures
        }
        self.tt = TypeTable::new(&prog.types);
        self.x_prepare(prog)?; // P06: which functions a contract may expand, and their raw bodies
        self.check_invariants(prog)?; // LG16: struct invariant declarations
        // R-6g9-3: the R-6g8-3 locals Rust emits as `&'static str` need no alloc (check.py static_lets)
        let statics: Vec<HashSet<String>> = {
            let mut fmap: HashMap<String, &Function> = HashMap::new();
            for f in &prog.functions {
                fmap.entry(f.name.clone()).or_insert(f);
            }
            prog.functions.iter().map(|f| crate::codegen::static_str_locals(f, &fmap)).collect()
        };
        for (f, statics) in prog.functions.iter_mut().zip(statics) {
            self.static_lets = statics;
            self.fn_(f)?;
            self.reject_stray_empty_views(f)?;
        }
        self.check_fn_sccs(&prog.functions)?; // G9 A0: measures and stable type parameters of recursive call SCCs
        self.check_par_eligibility(&prog.functions)?; // G7 A0: effects, foreign calls, then nested groups
        self.check_costs(prog)?;
        self.check_eq_targs()?; // P7-1 R1: after cost clauses, which may compare type parameters too
        self.record_g9(prog);
        self.x_auto(prog)?; // P07: code calls of an expandable function know its body (result == expansion)
        Ok(())
    }

    /// P07 (docs/L3_RESTRUCTURE_SPEC.md appendix P07, check.py `x_auto`): every function whose body expands as a
    /// contract keeps its template as `x_auto`, so the verifier can assume result == expansion at each code call. A
    /// function whose template cannot be built gets None (never an error); so does an i64 function whose template uses a
    /// bit builtin (its value would put a bit-vector term into every later query of the caller).
    fn x_auto(&mut self, prog: &mut Program) -> Res<()> {
        // check.py `allow_str`: read observers get a template only when the program uses borrow syntax
        let borrows = crate::borrowed::first_borrow(prog).is_some();
        for f in prog.functions.iter_mut() {
            f.x_auto = None;
            if !matches!(self.x_raw.get(&f.name), Some(Ok(_))) {
                // A read observer of a slice can supply its exact scalar result after the caller
                // has proved its requires. This template is for the verifier only.
                if borrows && f.ret == "i64" && f.tparams.is_empty() && f.effects.is_empty()
                    && f.decreases.is_none() && f.params.iter().all(|p| p.mode_pos.is_none() && !p.take)
                    && f.params.iter().any(|p| p.ty == "str"
                        || self.decls.get(&p.ty).is_some_and(crate::borrowed::borrowing_struct))
                {
                    if let [Stmt::Return(Return { value: Some(value), .. })] = f.body.as_slice() {
                        let observed = match value {
                            Expr::Call(c) if c.name == "len" => c.args.first(),
                            Expr::Index(x) if matches!(&*x.index, Expr::IntLit(_) | Expr::Var(_)) => Some(&*x.array),
                            _ => None,
                        };
                        if observed.and_then(place_root).is_some_and(|name| f.params.iter().any(|p| p.name == name)) {
                            f.x_auto = Some(value.clone());
                        }
                    }
                }
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

    /// LG16 (G-DB26-1): declaration checks of struct invariants. Each clause is checked once, in the environment of the
    /// struct's fields, as a contract expression (like a `requires`); the annotated clause stays on the declaration, and
    /// the verifier rewrites it per parameter (pruv-proof `inv_clauses`). Nothing else sees the clauses.
    fn check_invariants(&mut self, prog: &mut Program) -> Res<()> {
        fn uses_old_or_result(e: &Expr) -> bool {
            match e {
                Expr::Var(v) if v.name == "result" => true,
                Expr::Call(c) if c.name == "old" => true,
                _ => crate::refine::kids(e).into_iter().any(uses_old_or_result),
            }
        }
        for d in prog.types.iter_mut() {
            let TypeDecl::Struct(s) = d else { continue };
            if s.invariants.is_empty() {
                continue;
            }
            let sname = s.name.clone();
            if !s.tparams.is_empty() {
                return Err(terr(s.line, s.col, "invariant_decl",
                    format!("struct {} with type parameters cannot have an invariant", sname), ""));
            }
            let scope: Scope = s.fields.iter().map(|fd| (fd.name.clone(), Entry {
                ty: GType::from_key(&fd.ty), mutable: false, kind: "param",
            })).collect();
            self.cur_name = format!("\u{1}invariant:{}", sname);
            self.cur_ret = unitt();
            self.cur_cost = false;
            self.cur_params = s.fields.iter().map(|fd| fd.name.clone()).collect();
            self.enter(&decl_owner(&sname), &[]);
            for e in s.invariants.iter_mut() {
                if uses_old_or_result(e) {
                    return Err(terr(e.line(), e.col(), "invariant_decl",
                        format!("invariant of struct {} cannot use old() or result", sname), ""));
                }
                self.ckind = "requires";
                let ty = self.expr(e, &scope, true)?;
                self.ckind = "code";
                if ty != boolt() {
                    return Err(terr(e.line(), e.col(), "invariant_decl",
                        format!("invariant of struct {} must be bool", sname), ""));
                }
            }
        }
        for f in &prog.functions {
            if let Some(owner) = f.params.iter().filter(|p| p.name == "result")
                .find_map(|p| self.invariant_owner(&p.ty).or_else(|| {
                    // LG27: a top-level Vec element reaching an invariant struct counts too (check.py vec_elem)
                    let g = GType::from_key(&p.ty);
                    match (g.owner.is_none(), g.name.as_str(), g.args.as_slice()) {
                        (true, "vec", [e]) => self.invariant_owner(&e.to_key()),
                        _ => None,
                    }
                }))
            {
                return Err(terr(f.line, f.col, "invariant_decl",
                    format!("parameter result of type {} cannot be used: {} has an invariant", owner.name, owner.name), ""));
            }
        }
        // Legacy mode exposes all ordinary functions to host callers.
        if !self.explicit {
            for f in prog.functions.iter().filter(|f| !f.pred && f.foreign.is_none()) {
                if let Some((p, owner)) = f.params.iter()
                    .find_map(|p| self.invariant_owner(&p.ty).map(|owner| (p, owner)))
                {
                    return Err(terr(f.line, f.col, "entry_signature",
                        format!("host entry {} cannot take parameter {} of struct {} with an invariant", f.name, p.name, owner.name), ""));
                }
            }
        }
        Ok(())
    }

    fn plain_struct(&self, ty: &GType) -> Option<&StructDecl> {
        plain_struct(ty, &|name| self.struct_decl(name))
    }

    fn struct_decl(&self, name: &str) -> Option<&StructDecl> {
        match self.decls.get(name) {
            Some(TypeDecl::Struct(s)) => Some(s),
            _ => None,
        }
    }

    fn invariant_owner(&self, ty: &str) -> Option<&StructDecl> {
        reaches(&GType::from_key(ty), &|name| self.decls.get(name), &mut HashMap::new())
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
            if !self.explicit && name != "subview" && !type_name_ok(name) {
                return Err(terr(
                    line,
                    col,
                    "type_name",
                    format!("type name {} must start with an upper-case letter", name),
                    format!("rename it, e.g. {}", cap_first(name, true)),
                ));
            }
            if !self.explicit && (name == "subview" || RESERVED_TYPE_NAMES.contains(&name)) {
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
        if let Some(cs) = crate::gtype::tuple_components(&ty.name) {
            // P6: a tuple name spells its component keys; normalize them so `(Vec<i64>, i64)` names the catalog vec
            let cs: Vec<GType> = cs.iter().map(|k| self.norm_type(&GType::from_key(k))).collect();
            return GType::new(&crate::gtype::tuple_name(&cs), Vec::new());
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
        if name == "result" {
            match args.as_slice() {
                [x] => return mk_result_with_error(x.clone(), i64t()),
                [x, e] => return mk_result_with_error(x.clone(), e.clone()),
                _ => {}
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
        }
        GType::new(&name, args)
    }

    /// `norm_type` on an AST type key that may need normalization.
    fn norm_key(&self, key: &str) -> String {
        if !(key.contains("Vec") || key.contains("HashMap") || key.contains("String") || key.contains("File") || key.contains("Process")
            || key.contains("Socket") || key.contains("Listener") || key.contains("Loop") || key.contains("result<")
            || key.contains(crate::gtype::TUPLE_PREFIX))
        {
            return key.to_string();
        }
        self.norm_type(&GType::from_key(key)).to_key()
    }

    fn normalize_types(&self, prog: &mut Program) {
        for d in prog.types.iter_mut() {
            match d {
                TypeDecl::Struct(s) => {
                    if crate::gtype::is_tuple_name(&s.name) {
                        s.name = self.norm_key(&s.name);
                    }
                    for fd in s.fields.iter_mut() {
                        fd.ty = self.norm_key(&fd.ty);
                    }
                    for e in s.invariants.iter_mut() {
                        self.norm_expr(e); // LG16
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

    fn resolve_places(&mut self, prog: &mut Program) -> Res<()> {
        for f in &mut prog.functions {
            if !f.has_places() { continue; }
            let mut roots = f.params.iter().filter(|p| p.mode_pos.is_some())
                .map(|p| (p.name.clone(), p.ty.clone())).collect();
            let mut bindings = HashMap::new();
            self.resolve_place_block(&mut f.body, &mut roots, &mut bindings)?;
            self.place_functions.insert(f.name.clone(), bindings);
        }
        Ok(())
    }

    fn resolve_place_block(
        &self, body: &mut [Stmt], roots: &mut HashMap<String, String>, bindings: &mut crate::borrowed::PlaceRoots,
    ) -> Res<()> {
        for st in body {
            match st {
                Stmt::Let(s) if s.is_place() => {
                    if s.mutable {
                        return Err(terr(s.line, s.col, "borrow_signature", "a place binding cannot be `let mut`",
                            format!("write `let {} = inout ...;`; the binding itself is writable", s.name)));
                    }
                    let Expr::InoutArg(arg) = &mut s.value else { unreachable!() };
                    let (root, path, ty) = self.resolve_place(&mut arg.expr, roots)?;
                    arg.ty = Some(ty.clone());
                    if !s.ty.is_empty() && s.ty != ty {
                        return Err(terr(s.line, s.col, "type_mismatch", format!("{} is declared {} but the place has type {}",
                            s.name, tn(&GType::from_key(&s.ty)), tn(&GType::from_key(&ty))), ""));
                    }
                    s.ty = ty;
                    bindings.insert((s.line, s.col), (root, path));
                }
                Stmt::Let(s) if s.mutable => { roots.insert(s.name.clone(), s.ty.clone()); }
                Stmt::If(s) => {
                    self.resolve_place_block(&mut s.then, roots, bindings)?;
                    self.resolve_place_block(&mut s.els, roots, bindings)?;
                }
                Stmt::While(s) => self.resolve_place_block(&mut s.body, roots, bindings)?,
                Stmt::ForMap(s) => self.resolve_place_block(&mut s.body, roots, bindings)?,
                Stmt::Match(s) => for arm in &mut s.arms { self.resolve_place_block(&mut arm.body, roots, bindings)?; },
                _ => {}
            }
        }
        Ok(())
    }

    fn bad_place(line: i64, col: i64, why: impl std::fmt::Display) -> PruvError {
        terr(line, col, "place_binding", format!("a place binding needs a place: {why}"),
            "bind `x`, `x.f`, `x[i]` or `if c { x.a } else { x.b }` of a `let mut` local or an inout parameter")
    }

    fn resolve_place(&self, e: &mut Expr, roots: &HashMap<String, String>) -> Res<(String, Vec<String>, String)> {
        match e {
            Expr::Var(v) => {
                let ty = roots.get(&v.name).ok_or_else(|| Self::bad_place(v.line, v.col,
                    format!("{} is not a `let mut` local or an inout parameter", v.name)))?;
                Ok((v.name.clone(), Vec::new(), ty.clone()))
            }
            Expr::FieldAccess(f) => {
                let (root, mut path, ty) = self.resolve_place(&mut f.base, roots)?;
                let ty = GType::from_key(&ty);
                let field = self.field_type(f.line, f.col, &ty, &f.field)
                    .map_err(|_| Self::bad_place(f.line, f.col,
                        format!("{} has no field {}", tn(&ty), f.field)))?;
                path.push(f.field.clone());
                Ok((root, path, field.to_key()))
            }
            Expr::Index(i) => {
                let (root, path, ty) = self.resolve_place(&mut i.array, roots)?;
                let elem = match ty.as_str() {
                    "array" => "i64",
                    "array_bool" => "bool",
                    _ => return Err(Self::bad_place(i.line, i.col, format!(
                        "an element of {} (only [i64] and [bool] elements are places)", tn(&GType::from_key(&ty))))),
                };
                Ok((root, path, elem.into()))
            }
            Expr::Call(c) if PlaceIf::matches(c) => {
                let [_, then, els] = c.args.as_mut_slice() else { unreachable!() };
                let (root, mut path, ty) = self.resolve_place(then, roots)?;
                let (other, other_path, other_ty) = self.resolve_place(els, roots)?;
                if root != other {
                    return Err(Self::bad_place(c.line, c.col, format!("the arms name different roots {root} and {other}")));
                }
                if ty != other_ty {
                    return Err(Self::bad_place(c.line, c.col, format!("the arms have different types {} and {}",
                        tn(&GType::from_key(&ty)), tn(&GType::from_key(&other_ty)))));
                }
                path.truncate(path.iter().zip(&other_path).take_while(|(a, b)| a == b).count());
                c.ty = Some(ty.clone());
                Ok((root, path, ty))
            }
            _ => Err(Self::bad_place(e.line(), e.col(), "this expression is not a place")),
        }
    }

    fn check_place(&mut self, e: &mut Expr, scope: &Scope) -> Res<bool> {
        if let Expr::Call(c) = e {
            if PlaceIf::matches(c) {
                let [cond, then, els] = c.args.as_mut_slice() else { unreachable!() };
                self.expect(cond, &boolt(), scope, false, None)?;
                let then_ref = self.check_place(then, scope)?;
                let else_ref = self.check_place(els, scope)?;
                return Ok(then_ref || else_ref);
            }
        }
        let (mut root, mut path) = field_place(e);
        while let Expr::Index(i) = root {
            (root, path) = field_place(&i.array);
        }
        let Expr::Var(v) = root else { unreachable!("resolved place root") };
        let entry = self.lookup(v.line, v.col, &v.name, scope, &[])?;
        if !(entry.kind == "inout" || entry.kind == "local" && entry.mutable) {
            return Err(Self::bad_place(v.line, v.col, format!("{} is not a `let mut` local or an inout parameter", v.name)));
        }
        // R-6q-2: a place binding (`let d = inout P[..]`) writes P through `d`, so it is held like a direct write
        self.check_iteration_borrow(v.line, v.col, &v.name, &path)?;
        self.expr(e, scope, false)?;
        Ok(matches!(e, Expr::FieldAccess(f) if self.is_borrowed_field(&f.base, &f.field)))
    }

    fn norm_stmts(&self, stmts: &mut [Stmt]) {
        for st in stmts.iter_mut() {
            match st {
                Stmt::Let(s) => {
                    if !s.ty.is_empty() { s.ty = self.norm_key(&s.ty); }
                    self.norm_expr(&mut s.value);
                }
                Stmt::Assign(s) => self.norm_expr(&mut s.value),
                Stmt::FieldAssign(s) => self.norm_expr(&mut s.value),
                Stmt::Break(_) | Stmt::Continue(_) => {}
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
                for t in x.triggers.iter_mut() {
                    self.norm_expr(t);
                }
            }
            Expr::FieldAccess(x) => self.norm_expr(&mut x.base),
            Expr::IsVariant(x) => self.norm_expr(&mut x.value),
            Expr::VariantField(x) => self.norm_expr(&mut x.value),
            Expr::InoutArg(x) => self.norm_expr(&mut x.expr),
            Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) | Expr::Move(_) => {}
        }
    }

    fn check_field_names(&self, fields: &[FieldDecl], wher: &str) -> Res<()> {
        let mut seen: HashSet<&str> = HashSet::new();
        for f in fields {
            // P6: tuple fields are the indexes 0, 1, ..
            if !field_name_ok(&f.name) && !(f.name.bytes().all(|b| b.is_ascii_digit()) && !f.name.is_empty()) {
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
        if ty.is_fixed() {
            return Ok(()); // P7-1: the parser admitted the element and the size
        }
        if ty.view_base().is_none() && ty.args.iter().any(contains_unsupported_small_int_arg) {
            return Err(small_int_unsupported(line, col, "non-i64 integer and [u16] type arguments require P11", "use i64"));
        }
        if ty.view_payload().is_some() {
            return Ok(());
        }
        if ty.name == "@ref" || ty.args.iter().any(|t| self.borrowed_type_argument(t)) {
            return Err(terr(line, col, "borrow_signature", "borrowed types cannot be generic arguments", ""));
        }
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
            if let Some(a) = ty.args.first() {
                // LB8 R-6g7-1: Result<(), E>
                if !(ty.name == "result" && *a == unitt()) {
                    self.check_arg(line, col, a, None)?;
                }
            }
            if let Some(e) = ty.result_error() {
                self.check_result_error(line, col, e)?;
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
                            && d.fields.iter().any(|f| crate::gtype::key_is_collection(&f.ty) || matches!(f.ty.as_str(), "array_u8" | "array_u16"))
                            && d.fields.iter().all(|f| {
                                let ft = GType::from_key(&f.ty);
                                crate::gtype::key_is_owned_field(&f.ty)
                                    || matches!(f.ty.as_str(), "array_u8" | "array_u16")
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
        if ty.int_width().is_some() || crate::typeinfo::is_builtin_type_name(&ty.name) {
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

    fn check_result_error(&self, line: i64, col: i64, e: &GType) -> Res<()> {
        if unsupported_small_int_arg(e) {
            return Err(small_int_unsupported(line, col, "non-i64 integer and [u16] Result error types require P11", "use i64"));
        }
        if self.borrowed_type_argument(e) {
            return Err(terr(line, col, "borrow_signature", "borrowed types cannot be generic arguments", ""));
        }
        if e.owner.is_none() && e.name == "string" {
            return self.check_type(line, col, e);
        }
        if e.owner.is_some()
            || matches!(e.name.as_str(), "option" | "result")
            || !is_value(e, &|n| self.value_decl(n))
        {
            return Err(terr(
                line,
                col,
                "result_error_type",
                format!("{} is not supported as a Result error type", tn(e)),
                "use i64, bool, String or a value struct or enum",
            ));
        }
        self.check_type(line, col, e)
    }

    fn borrowed_type_argument(&self, ty: &GType) -> bool {
        ty.name == "@ref" || self.decls.get(&ty.name).is_some_and(crate::borrowed::borrowing_struct)
            || ty.args.iter().any(|t| self.borrowed_type_argument(t))
    }

    /// a: a type argument; tp: the TypeParam it instantiates (None for Option/Result).
    fn check_arg(&self, line: i64, col: i64, a: &GType, tp: Option<&TParam>) -> Res<()> {
        if unsupported_small_int_arg(a) {
            return Err(small_int_unsupported(line, col, "non-i64 integer and [u16] type arguments require P11", "use i64"));
        }
        if self.borrowed_type_argument(a) {
            return Err(terr(line, col, "borrow_signature", "borrowed types cannot be generic arguments", ""));
        }
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
                        // P9b-3: a method's leading type arguments belong to its impl block and are written
                        // after the type (`Box2::<T>::f`, R-6x-1); `Box2::f::<T>` is a parse error there
                        match crate::diagnostic::method_text(last).rsplit_once("::").filter(|_| crate::diagnostic::method_text(last) != last) {
                            Some((ty, f)) => format!(
                                "write the type arguments explicitly where Rust takes them: the impl block's after the type ({}::<T>::{}), the method's own after its name; they are never inferred",
                                ty, f
                            ),
                            _ => format!("write the type arguments explicitly, e.g. {}::<T>; they are never inferred", last),
                        },
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
            if !catalog_collection.is_some_and(|n| n.starts_with("vec_")) || !self.recursive_vec_element(a) {
                let inside = if catalog_collection.is_some() { "coll" } else { "arg" };
                self.ext_misuse(line, col, a, Some(inside))?;
            }
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
                    if matches!(d, TypeDecl::Enum(_)) && is_narrow(&ft) {
                        return Err(small_int_unsupported(f.line, f.col, "non-i64 integer enum payloads require P11", "use i64"));
                    }
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
                        || (ft.is_array() && !ft.args.is_empty())
                        || (ft.owner.is_none() && ft.name == "result" && ft.args.len() <= 2 && ft.args.first() == Some(&unitt()))
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
                            || matches!(d, TypeDecl::Struct(_)) && self.state_collection(&ft)
                            || self.vec_scc_edge(d.name(), &ft)))
                    {
                        if matches!(ft.name.as_str(), "vec" | "hashmap") {
                            if let Some(payload) = ft.args.last().filter(|ty| is_value(ty, &|n| self.value_decl(n))) {
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
            if f.foreign.is_some() && std::iter::once(&f.ret).chain(f.params.iter().map(|p| &p.ty)).any(|ty| intwidth::is_narrow(ty)) {
                return Err(small_int_unsupported(f.line, f.col, "non-i64 integers in foreign signatures require P12", "use i64"));
            }
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
            && !crate::typeinfo::is_builtin_type_name(&ty.name)
    }

    /// R-6e-20: `e` is a catalog `vec_get` of a recursive vector element (a borrowed read).
    fn rec_vec_get<'e>(&self, e: &'e Expr) -> Option<&'e Call> {
        match e {
            Expr::Call(call) if call.name == "vec_get" && !self.funcs.contains_key(&call.name)
                && call.ty.as_deref().is_some_and(|ty| self.recursive_vec_element(&GType::from_key(ty))) => Some(call),
            _ => None,
        }
    }

    fn recursive_vec_element(&self, ty: &GType) -> bool {
        self.is_plain_nominal(ty) && self.recursive_vec_elements.contains(&ty.name)
    }

    fn vec_scc_edge(&self, owner: &str, ty: &GType) -> bool {
        ty.name == "vec" && ty.args.first().is_some_and(|elem| {
            self.is_plain_nominal(elem) && self.type_sccs.iter().any(|scc| {
                scc.iter().any(|n| n == owner) && scc.contains(&elem.name)
            })
        })
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
            // (b) intra-SCC fields are direct boxed fields or unboxed Vec edges
            for &i in &comp {
                let d = &types[i];
                for (vname, fd) in decl_fields(d) {
                    let ft = GType::from_key(&fd.ty);
                    if !dependencies(&ft).iter().any(|n| members.contains(n.as_str())) {
                        continue;
                    }
                    if self.vec_scc_edge(d.name(), &ft) {
                        self.recursive_vec_elements.insert(ft.args[0].name.clone());
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
        // P7-1 B: an inline `[T; N]` field makes `==` unsound in proofs (Buf compares the whole SMT array), so a
        // struct holding one is buffered for `==` without becoming extended
        let mut buf: HashSet<String> = direct.clone();
        for d in types {
            if decl_fields(d).iter().any(|(_, fd)| GType::from_key(&fd.ty).is_fixed()) {
                buf.insert(d.name().to_string());
            }
        }
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
                            || matches!(f.ty.as_str(), "array_u8" | "array_u16")
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
                        || matches!(f.ty.as_str(), "array_u8" | "array_u16")
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
                        || matches!(f.ty.as_str(), "array_u8" | "array_u16")
                        || self.collection_records.borrow().contains(&ty.name) && self.record_map_field(&ft)
                        || is_value(&ft, &|n| self.value_decl(n))
                }))
    }

    fn depends_on_nested_record(&self, ty: &GType) -> bool {
        dependencies(ty).iter().any(|name| self.nested_records.contains(name))
    }

    fn state_collection(&self, ty: &GType) -> bool {
        match (ty.name.as_str(), ty.args.as_slice()) {
            ("vec", [v]) => self.owned_aggregate(v) || self.recursive_vec_element(v),
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
                if ty.name != "vec" || !self.recursive_vec_element(a) {
                    self.ext_misuse(line, col, a, Some("coll"))?;
                }
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
                    if pt.int_width().is_none() && pt != boolt() && !pt.is_array() && !pt.is_fixed()
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
        let resizes = resize_params(prog);
        for (i, f) in prog.functions.iter().enumerate() {
            if !self.host_entry.get(i).copied().unwrap_or(false) {
                continue;
            }
            // LG16: a host entry cannot take a struct with an invariant (its callers are not proved)
            if let Some((p, owner)) = f.params.iter()
                .find_map(|p| self.invariant_owner(&p.ty).map(|owner| (p, owner)))
            {
                return Err(terr(f.line, f.col, "entry_signature",
                    format!("host entry {} cannot take parameter {} of struct {} with an invariant", f.name, p.name, owner.name), ""));
            }
            if let Some(p) = f.params.iter().find(|p| p.mode_pos.is_some() && matches!(p.ty.as_str(), "i64" | "bool")) {
                return Err(terr(f.line, f.col, "entry_signature",
                    format!("host entry {} cannot take scalar inout parameter {}", f.name, p.name), ""));
            }
            for (j, p) in f.params.iter().enumerate() {
                if GType::from_key(&p.ty).is_array() && resizes.contains(&(f.name.clone(), j)) {
                    let (line, col) = p.mode_pos.unwrap_or((f.line, f.col));
                    return Err(terr(line, col, "entry_signature",
                        format!("host entry {} cannot reassign inout array parameter {}", f.name, p.name), ""));
                }
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

    /// P7-1 R1: `==` on a type parameter in a contract compares whole SMT values, and a `[T; N]` (alone or inside a
    /// struct) is an SMT array whose cells past N are free, so the proof could tell equal values apart. A call that
    /// binds such a parameter, directly or through the caller's own parameters, to a type holding a fixed array is
    /// `type/eq_type`.
    fn check_eq_targs(&mut self) -> Res<()> {
        let mut changed = true;
        while changed {
            changed = false;
            for (_, _, targs, callee) in &self.targ_log {
                let owner = fn_owner(callee);
                for (k, t) in targs.iter().enumerate() {
                    if self.eq_tparams.contains(&(owner.clone(), k)) {
                        let mut found = HashSet::new();
                        collect_tparams(t, &mut found);
                        for tp in found {
                            changed |= self.eq_tparams.insert(tp);
                        }
                    }
                }
            }
        }
        for (line, col, targs, callee) in &self.targ_log {
            let owner = fn_owner(callee);
            for (k, t) in targs.iter().enumerate() {
                if self.eq_tparams.contains(&(owner.clone(), k)) && self.holds_fixed(t) {
                    return Err(terr(*line, *col, "eq_type",
                        format!("{} compares its type parameter with == in a contract (directly or through a call), and {} holds a fixed-length array",
                            callee, tn(t)),
                        "compare the elements in a dedicated non-generic function"));
                }
            }
        }
        Ok(())
    }

    /// a `[T; N]` itself, or a declaration that holds one (buffered), anywhere in `t`
    fn holds_fixed(&self, t: &GType) -> bool {
        t.is_fixed() || (t.owner.is_none() && self.buffered.contains(&t.name)) || t.args.iter().any(|a| self.holds_fixed(a))
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
        self.borrowed_result = f.borrowed_result;
        self.try_count = 0;
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
        } else if fname == "main" && (ret.is_collection_owner() || f.params.iter().any(|p| key_is_collection_owner(&p.ty))) {
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
            if p.take && !pt.is_array() && !pt.is_collection_owner() && !self.owned_aggregate(&pt) {
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
            for st in &f.body {
                if let Stmt::Return(r) = st {
                    if let Some(v) = &r.value {
                        Self::check_try_position(v, r.line, r.col)?;
                    }
                }
            }
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
            self.expect_measure(d, &scope, true)?;
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
        let aggregates = self.decls.keys().filter(|n| {
            let ty = GType::from_key(n);
            self.owned_aggregate(&ty) || self.recursive_vec_element(&ty)
        }).cloned().collect();
        // G-DB16-1 / G-R09-4: non-generic user enums and structs, tracked as owned locals so a binder move or a
        // consuming field move can consume them
        let enums: HashSet<String> = self
            .decls
            .iter()
            .filter(|(_, d)| match d {
                TypeDecl::Enum(e) => e.tparams.is_empty(),
                TypeDecl::Struct(s) => s.tparams.is_empty(),
            })
            .map(|(n, _)| n.clone())
            .collect();
        let variants: HashMap<(String, String), Vec<String>> = self
            .decls
            .iter()
            .filter_map(|(n, d)| match d {
                TypeDecl::Enum(e) if e.tparams.is_empty() => Some((n, e)),
                _ => None,
            })
            .flat_map(|(n, e)| e.variants.iter()
                .map(move |v| ((n.clone(), v.name.clone()), v.fields.iter().map(|f| f.ty.clone()).collect())))
            .collect();
        // For ownership analysis a replaced array starts live, like a take
        // parameter. Only the RHS that restores it may consume it.
        let mut moved = HashSet::new();
        if f.params.iter().any(|p| p.mode_pos.is_some() && GType::from_key(&p.ty).is_array()) {
            crate::foreign_shim::walk_stmts(&f.body, &mut |node| {
                if let crate::foreign_shim::Node::Expr(Expr::Move(m)) = node {
                    if m.fields.is_empty() {
                        moved.insert(m.name.clone());
                    }
                }
            });
        }
        if f.params.iter().any(|p| p.mode_pos.is_some() && GType::from_key(&p.ty).is_array() && moved.contains(&p.name)) {
            let mut owned = f.clone();
            for p in &mut owned.params {
                p.take |= p.mode_pos.is_some() && GType::from_key(&p.ty).is_array() && moved.contains(&p.name);
            }
            check_ownership(&owned, &self.takes, &aggregates, &enums, &variants)?;
        } else {
            check_ownership(f, &self.takes, &aggregates, &enums, &variants)?;
        }
        Ok(())
    }

    // ---- statements ----
    fn block(&mut self, stmts: &mut Vec<Stmt>, scope: &mut Scope) -> Res<()> {
        let mut i = 0;
        while i < stmts.len() {
            if let Stmt::Match(m) = &stmts[i] {
                if m.arms.iter().any(|a| slice_arm_shape(&a.ctor).is_some()) {
                    let one = m.arms.len() == 1;
                    let st = self.slice_match(m, scope)?;
                    if one && i + 1 == stmts.len() {
                        // a lone `[..]`/`[rest @ ..]` arm as the last statement is spliced in place, as tuple_match is
                        let Stmt::If(w) = st else { unreachable!("slice_match builds an if") };
                        stmts.pop();
                        stmts.extend(w.then);
                    } else {
                        stmts[i] = st;
                    }
                    continue;
                }
                if Self::has_ext_pattern(m.arms.iter().map(|a| (a.ctor.as_str(), a.binders.as_slice()))) {
                    let st = self.pattern_match(m, scope)?;
                    stmts[i] = st;
                    continue;
                }
                if m.arms.iter().any(|a| a.ctor == "()") {
                    let st = self.tuple_match(m, scope)?;
                    if i + 1 == stmts.len() {
                        // the last statement: its lets end with the block anyway, so they are spliced in place
                        let Stmt::If(w) = st else { unreachable!("tuple_match builds an if") };
                        stmts.pop();
                        stmts.extend(w.then);
                    } else {
                        stmts[i] = st;
                    }
                    continue;
                }
            }
            if let Stmt::Assign(s) = &stmts[i] {
                if !s.deref && has_try(&s.value) {
                    // G-LG13-6: the target passed inout inside a statement with a `?`
                    fn inout_of(e: &Expr, name: &str) -> Option<(i64, i64)> {
                        if let Expr::InoutArg(a) = e {
                            if matches!(field_place(&a.expr).0, Expr::Var(v) if v.name == name) {
                                return Some((a.line, a.col));
                            }
                        }
                        crate::refine::kids(e).into_iter().find_map(|k| inout_of(k, name))
                    }
                    if let Some((line, col)) = inout_of(&s.value, &s.name) {
                        return Err(terr(line, col, "inout_context",
                            format!("{} is passed inout and also mentioned elsewhere in this statement", s.name),
                            "use a separate statement for the inout call"));
                    }
                }
            }
            let site = match &stmts[i] {
                Stmt::Let(s) => Some(&s.value),
                Stmt::Assign(s) if !s.deref && scope.get(&s.name).is_none_or(|entry| entry.kind == "local") =>
                    Some(&s.value),
                Stmt::ExprStmt(s) => Some(&s.expr),
                Stmt::Return(s) => s.value.as_ref(),
                Stmt::If(s) => Some(&s.cond),
                _ => None,
            };
            if let Some((path, temps)) = site.and_then(find_try) {
                let short = {
                    let (_, _, mut node) = Self::try_site_mut(&mut stmts[i]);
                    for &k in &path {
                        node = kid_mut(node, k);
                    }
                    matches!(node, Expr::Binary(_))
                };
                if short {
                    // LG15: checked on the statement before its temporaries
                    self.try_short_check(&mut stmts[i], &path, scope)?;
                }
                // LG14 (G-LG13-1): bind the earlier operands first; LG21 (G-LG20-3): try_temps also runs its split
                // and effect checks on a statement with nothing to bind
                let lets = self.try_temps(&mut stmts[i], temps, scope)?;
                if !lets.is_empty() {
                    stmts.splice(i..i, lets);
                    continue;
                }
                if short {
                    // LG15 (G-LG13-2): the operator becomes a nested if
                    let lowered = self.try_short(&mut stmts[i], &path, scope)?;
                    stmts.splice(i..i, lowered);
                    continue;
                }
                let rest = stmts.split_off(i + 1);
                let stmt = stmts.pop().unwrap();
                stmts.push(Stmt::Match(self.lower_try(stmt, rest, path, scope)?));
                break;
            }
            let (prefix, tail) = stmts.split_at_mut(i + 1);
            self.stmt(&mut prefix[i], scope, tail.first())?;
            i += 1;
        }
        Ok(())
    }

    fn try_position_error(line: i64, col: i64) -> PruvError {
        terr(line, col, "try_position",
            "`?` is only allowed in the value of a let, a local assignment or a return, an expression statement or an if \
             condition, and not in a quantifier, in the right operand of ==>, in the right operand of && / || when \
             the statement passes inout or the operator holds a quantifier or a subview or moves a name used outside it \
             (or uses one moved outside it, or a binding of a type that is not a value used again), or after an operand evaluated before it that cannot be bound to a temporary (one with inout, \
             a quantifier, a move of a name used again or a type that is not a value)",
            "bind the value with `let x: T = e?;` first (for a Result<(), E>, write `e?;` as a statement)")
    }

    fn check_try_position(e: &Expr, line: i64, col: i64) -> Res<()> {
        if matches!(e, Expr::Unary(u) if u.op == "?") {
            return Err(Self::try_position_error(line, col));
        }
        for child in crate::refine::kids(e) {
            Self::check_try_position(child, line, col)?;
        }
        Ok(())
    }

    /// The line, column and lowered expression of a statement find_try looks at.
    fn try_site_mut(stmt: &mut Stmt) -> (i64, i64, &mut Expr) {
        match stmt {
            Stmt::Let(s) => (s.line, s.col, &mut s.value),
            Stmt::Assign(s) => (s.line, s.col, &mut s.value),
            Stmt::ExprStmt(s) => (s.line, s.col, &mut s.expr),
            Stmt::Return(s) => (s.line, s.col, s.value.as_mut().unwrap()),
            Stmt::If(s) => (s.line, s.col, &mut s.cond),
            _ => unreachable!(),
        }
    }

    /// LG14 (G-LG13-1): `let __pruv_try_tN: T = k;` for each operand k evaluated before the statement's first `?`, in
    /// evaluation order, with k replaced by the fresh local in stmt; T is a value type or, per LG18 (G-LG14-1), an owned call result or move
    /// that find_try admitted as a direct user-call argument. Python `try_temps`.
    fn try_temps(&mut self, stmt: &mut Stmt, temps: Vec<Vec<usize>>, scope: &Scope) -> Res<Vec<Stmt>> {
        fn has_inout(e: &Expr) -> bool {
            matches!(e, Expr::InoutArg(_)) || crate::refine::kids(e).into_iter().any(has_inout)
        }
        fn calls<'a>(e: &'a Expr, out: &mut Vec<&'a Call>) {
            if let Expr::Call(c) = e {
                out.push(c);
            }
            for k in crate::refine::kids(e) {
                calls(k, out);
            }
        }
        let inout = has_inout(Self::try_site_mut(stmt).2);
        let (line0, col0, _) = Self::try_site_mut(stmt);
        let effect = |me: &Self, e: &Expr| -> bool {
            let mut cs = Vec::new();
            calls(e, &mut cs);
            cs.into_iter().any(|c| {
                c.name == "print"
                    || match me.funcs.get(&c.name) {
                        Some(f) => f.effects.iter().any(|x| x != "alloc"),
                        None => catalog_fn(&c.name).is_some_and(|f| f.effects.iter().any(|x| x != "alloc")),
                    }
            })
        };
        {
            let root: &Expr = Self::try_site_mut(stmt).2;
            fn walk<'a>(e: &'a Expr, parent: Option<(&'a Expr, usize)>, out: &mut Vec<(&'a Expr, Option<(&'a Expr, usize)>)>) {
                out.push((e, parent));
                for (i, k) in crate::refine::kids(e).into_iter().enumerate() {
                    walk(k, Some((e, i)), out);
                }
            }
            let mut all = Vec::new();
            walk(root, None, &mut all);
            let mut parts: Vec<&Expr> = temps
                .iter()
                .map(|path| path.iter().fold(root, |n, &i| crate::refine::kids(n)[i]))
                .collect();
            let tries: Vec<&Expr> = all
                .iter()
                .filter_map(|(x, _)| match x {
                    Expr::Unary(u) if u.op == "?" => Some(&*u.operand),
                    _ => None,
                })
                .collect();
            if inout && tries.iter().any(|q| effect(self, q)) {
                return Err(Self::try_position_error(line0, col0));
            }
            parts.extend(tries);
            // LG18 (G-LG14-2 b): a part (temporary or `?` operand) mentioning a binding of a type that is not a value
            // type, mentioned again outside the part in a call holding the part, is type/try_position unless that
            // mention is a direct argument of a borrowed parameter of a user function: nested calls share the
            // outermost call's borrow track and lowering moves the part out of it. Python `try_split_check`.
            for part in parts {
                let mut inside = Vec::new();
                walk(part, None, &mut inside);
                let has = |x: &Expr| inside.iter().any(|(y, _)| std::ptr::eq(*y, x));
                fn named(x: &Expr) -> Option<&str> {
                    match x {
                        Expr::Var(v) => Some(v.name.as_str()),
                        Expr::Move(m) => Some(m.name.as_str()),
                        _ => None,
                    }
                }
                let names: Vec<&str> = inside
                    .iter()
                    .filter_map(|(x, _)| named(x))
                    .filter(|n| scope.get(*n).is_some_and(|e| !is_value(&e.ty, &|d| self.value_decl(d))))
                    .collect();
                for (c, _) in &all {
                    if !matches!(c, Expr::Call(_)) {
                        continue;
                    }
                    let mut sub = Vec::new();
                    walk(c, None, &mut sub);
                    if !sub.iter().any(|(y, _)| std::ptr::eq(*y, part)) {
                        continue;
                    }
                    for (x, _) in &sub {
                        let Some(n) = named(x) else { continue };
                        if has(x) || !names.contains(&n) {
                            continue;
                        }
                        let parent = all.iter().find(|(y, _)| std::ptr::eq(*y, *x)).and_then(|(_, p)| *p);
                        let ok = match (x, parent) {
                            (Expr::Var(_), Some((Expr::Call(p), i))) => match (self.funcs.get(&p.name), self.takes.get(&p.name)) {
                                (Some(f), Some(takes)) => !f.foreign && i < f.params.len() && i < takes.len() && !takes[i],
                                // LG21 (G-LG20-3): with nothing bound, a builtin's borrowed argument (`len(s)`) also
                                // only reads the name after the `?` operand ran; string_to_str consumes a String
                                // error binder and is not a read
                                (None, _) if temps.is_empty() => {
                                    p.name != "string_to_str"
                                        && catalog_fn(&p.name).is_none_or(|f| i < f.params.len() && !f.params[i].take)
                                }
                                _ => false,
                            },
                            _ => false,
                        };
                        if !ok {
                            return Err(Self::try_position_error(line0, col0));
                        }
                    }
                }
            }
        }
        let mut lets = Vec::new();
        for path in temps {
            let parent_name = {
                let (_, _, mut p) = Self::try_site_mut(stmt);
                for &i in &path[..path.len() - 1] {
                    p = kid_mut(p, i);
                }
                match p {
                    Expr::Call(c) => Some(c.name.clone()),
                    _ => None,
                }
            };
            let j = *path.last().unwrap();
            let (line, col, mut rhs) = Self::try_site_mut(stmt);
            for i in path {
                rhs = kid_mut(rhs, i);
            }
            let mut probe = rhs.clone();
            self.ckind = "code";
            self.boundary = None;
            let t = self.expr(&mut probe, scope, false)?;
            if t == unitt() {
                return Err(Self::try_position_error(line, col));
            }
            // LG18 (G-LG14-2 a): a call with effects before the inout call is inout_context unlowered
            if inout && effect(self, rhs) {
                return Err(Self::try_position_error(line, col));
            }
            let mut take = false;
            if !is_value(&t, &|n| self.value_decl(n)) {
                // LG18 (G-LG14-1): an operand of an owned type (Vec, HashMap, String) may be bound only when it is a
                // call result (not subview) or a plain move passed directly as an argument of a non-foreign user
                // function to a take parameter of exactly its type: the temporary is moved into the same call, so it
                // is consumed where the value was (a borrowed parameter would drop it at the end of the block, and a
                // coercion such as Vec to [T] applies to places only). Python `try_owned_temp`.
                let f = parent_name.as_ref().and_then(|n| self.funcs.get(n).map(|f| (f.clone(), self.takes.get(n).cloned())));
                let ok = match &f {
                    Some((f, Some(takes))) if !f.foreign && j < f.params.len() && j < takes.len() && takes[j]
                        && f.params[j] == t && t.owner.is_none() && matches!(t.name.as_str(), "vec" | "hashmap" | "string") => {
                        take = true;
                        match &*rhs {
                            Expr::Call(c) => c.name != "subview",
                            Expr::Move(m) => m.fields.is_empty(),
                            _ => false,
                        }
                    }
                    _ => false,
                };
                // LG19 (G-LG18-4): a lifted call result is bound only when its own arguments create no owned
                // temporary: the `let` would drop a value the call borrowed at its end, before the `?` operand runs,
                // instead of at the end of the statement. Python `try_owned_temp`.
                let plain = !ok
                    || match &probe {
                        Expr::Call(c) => c.args.iter().all(|a| self.try_plain_arg(a)),
                        _ => true,
                    };
                if !ok || !plain {
                    return Err(Self::try_position_error(line, col));
                }
            } else if !crate::refine::kids(&probe).into_iter().all(|a| self.try_plain_arg(a)) {
                // LG20 (G-LG19-1): a value temporary follows the same rule: `lend(mk(2), 0)` is an i64, but the
                // String `mk(2)` that `lend` borrows would be dropped at the end of the temporary's `let`, before
                // the `?` operand runs. Python `try_temps`.
                return Err(Self::try_position_error(line, col));
            }
            let name = format!("{TRY_TEMP}{}", self.try_count);
            self.try_count += 1;
            let new = if take {
                Expr::Move(Move { line, col, name: name.clone(), fields: Vec::new(), ty: None })
            } else {
                Expr::Var(Var { line, col, name: name.clone(), ty: None })
            };
            let k = std::mem::replace(rhs, new);
            lets.push(Stmt::Let(Let { borrowed: false, line, col, name, mutable: false, ty: t.to_key(), value: k, refine: None }));
        }
        Ok(lets)
    }

    /// LG19 (G-LG18-4): a is a move, an int / bool literal, a place (a binding or a field of one), or an operator or
    /// index whose operands are all plain in this sense, so evaluating it creates no temporary that needs a drop.
    /// Syntactic, without a standalone type check (an argument may need its parameter's borrow context); a string
    /// literal is not plain (the generated code may build a String from it). LG20 (G-LG19-3): any other node (a call)
    /// whose type, as the typed copy of the temporary already recorded it, is a value type and whose operands are all
    /// plain is plain too: its result needs no drop and it creates no other value. Python `try_plain_arg`.
    fn try_plain_arg(&self, a: &Expr) -> bool {
        if matches!(a, Expr::Move(_) | Expr::IntLit(_) | Expr::BoolLit(_)) || matches!(field_place(a).0, Expr::Var(_)) {
            return true;
        }
        let kids_plain = || crate::refine::kids(a).into_iter().all(|k| self.try_plain_arg(k));
        match a {
            Expr::Unary(u) if u.op == "?" => false,
            Expr::Unary(_) | Expr::Binary(_) | Expr::Index(_) => kids_plain(),
            _ => match expr_ty(a) {
                Some(k) => {
                    GType::try_from_key(k).is_ok_and(|t| is_value(&t, &|d| self.value_decl(d))) && kids_plain()
                }
                None => false,
            },
        }
    }

    /// LG15 (G-LG13-2): `let mut __pruv_try_tN: bool = D; if C { __pruv_try_tN = R; }` for the operator node `L op R`
    /// at path whose right operand R holds the statement's first `?`, with the node replaced by the fresh local in
    /// stmt: D, C = false, L for &&; true, !L for ||. The assignment is a lowered position, so the `?` in R is lowered
    /// there and runs only when R runs, as in the hand-written nested if. A binding of a type that is not a value type
    /// mentioned in the operator and mentioned again in the statement is type/try_position: splitting the statement
    /// would hide a borrow or an implicit move between the two mentions. Python `try_short`.
    /// LG15: type/try_position when the operator at path mentions a binding whose declared type is not a value type
    /// and the statement mentions it again. Called on the statement before its LG14 temporaries are bound (a
    /// temporary would hide a borrow in an earlier operand) and again before the lowering.
    fn try_short_check(&mut self, stmt: &mut Stmt, path: &[usize], scope: &Scope) -> Res<()> {
        let (line, col, mut node) = Self::try_site_mut(stmt);
        let root = node.clone();
        for &k in path {
            node = kid_mut(node, k);
        }
        fn names(e: &Expr, out: &mut Vec<String>) {
            match e {
                Expr::Var(x) => out.push(x.name.clone()),
                Expr::Move(x) => out.push(x.name.clone()),
                _ => {}
            }
            for k in crate::refine::kids(e) {
                names(k, out);
            }
        }
        let mut used = Vec::new();
        names(node, &mut used);
        let mut all = Vec::new();
        names(&root, &mut all);
        for n in used {
            if let Some(entry) = scope.get(&n) {
                if all.iter().filter(|x| **x == n).count() > 1 && !is_value(&entry.ty, &|d| self.value_decl(d)) {
                    return Err(Self::try_position_error(line, col));
                }
            }
        }
        Ok(())
    }

    fn try_short(&mut self, stmt: &mut Stmt, path: &[usize], scope: &Scope) -> Res<Vec<Stmt>> {
        self.try_short_check(stmt, path, scope)?;
        let (line, col, mut node) = Self::try_site_mut(stmt);
        for &k in path {
            node = kid_mut(node, k);
        }
        let name = format!("{TRY_TEMP}{}", self.try_count);
        self.try_count += 1;
        let Expr::Binary(b) = std::mem::replace(node, Expr::Var(Var { line, col, name: name.clone(), ty: None }))
        else { unreachable!() };
        let cond = if b.op == "||" {
            Expr::Unary(Unary { line, col, op: "!".into(), operand: b.left, ty: None })
        } else {
            *b.left
        };
        let init = Expr::BoolLit(BoolLit { line, col, value: b.op != "&&", ty: None });
        Ok(vec![
            Stmt::Let(Let { borrowed: false, line, col, name: name.clone(), mutable: true, ty: boolt().to_key(), value: init, refine: None }),
            Stmt::If(If { line, col, cond, then: vec![Stmt::Assign(Assign { line, col, name, value: *b.right, deref: false })],
                els: Vec::new() }),
        ])
    }

    fn lower_try(&mut self, mut stmt: Stmt, mut rest: Vec<Stmt>, path: Vec<usize>, scope: &mut Scope)
        -> Res<Match> {
        let whole = path.is_empty();
        let (line, col, mut rhs) = Self::try_site_mut(&mut stmt);
        for i in path {
            rhs = kid_mut(rhs, i);
        }
        let n = self.try_count;
        self.try_count += 1;
        let ok = format!("__pruv_try_v{n}");
        let err = format!("__pruv_try_e{n}");
        let var = |name: &str| Expr::Var(Var { line, col, name: name.into(), ty: None });
        let Expr::Unary(u) = std::mem::replace(rhs, var(&ok)) else { unreachable!() };
        let mut scrutinee = *u.operand;
        Self::check_try_position(&scrutinee, line, col)?;
        self.ckind = "code";
        self.boundary = None;
        let ty = self.match_scrutinee(&mut scrutinee, scope)?;
        if ty.owner.is_some() || ty.name != "result" {
            return Err(terr(line, col, "try_operand", format!("`?` needs a Result operand, got {}", tn(&ty)),
                "use `?` only on a Result value"));
        }
        if self.cur_ret.owner.is_some() || self.cur_ret.name != "result" {
            return Err(terr(line, col, "try_return", "`?` needs the enclosing function to return a Result",
                "return a Result or use an explicit match"));
        }
        if ty.result_error() != self.cur_ret.result_error() {
            let actual = ty.result_error().cloned().unwrap_or_else(i64t);
            let expected = self.cur_ret.result_error().cloned().unwrap_or_else(i64t);
            return Err(terr(line, col, "try_error_type",
                format!("`?` error type {} does not match the function error type {}", tn(&actual), tn(&expected)),
                "use an explicit match and convert the error"));
        }
        let bare = whole && matches!(stmt, Stmt::ExprStmt(_));
        let unit = ty.args.first() == Some(&unitt());
        if unit && !bare {
            // LB8 R-6g7-1: the unit value of `e?` is only discarded
            return Err(Self::try_position_error(line, col));
        }
        if !bare {
            rest.insert(0, stmt);
        }
        let mut targs = self.cur_ret.args.clone();
        if targs.is_empty() {
            targs.push(i64t());
        }
        let ret = Expr::Ctor(Ctor {
            line, col, name: "Err".into(), arg: Some(Box::new(var(&err))), ty: None, targs: Some(targs), inst: None,
        });
        let mut lowered = Match { line, col, scrutinee, form: MatchForm::Match, els: Vec::new(), arms: vec![
            MatchArm { line, col, enum_name: None, ctor: "Ok".into(), binders: if unit { vec![] } else { vec![ok] }, body: rest,
                pats: Vec::new(), pat_lets: 0 },
            MatchArm { line, col, enum_name: None, ctor: "Err".into(), binders: vec![err],
                body: vec![Stmt::Return(Return { line, col, value: Some(ret) })], pats: Vec::new(), pat_lets: 0 },
        ] };
        // The operand was already checked in match context.
        self.match_arms(&mut lowered, scope, ty)?;
        Ok(lowered)
    }

    /// LG9 (G-LG8-1): `f(move d.a)` as a let initializer, call statement or return value, the field move a direct
    /// argument of a take parameter, consumes d like `let t = move d.a;` (no restore follows; no other argument
    /// mentions d). Python `field_move_arg`.
    fn field_move_arg(&self, call: &Call, scope: &Scope, next: Option<&Stmt>) -> Res<Option<(i64, i64)>> {
        fn count(e: &Expr, n: &mut usize) {
            if let Expr::Move(m) = e {
                if !m.fields.is_empty() {
                    *n += 1;
                }
            }
            for k in crate::refine::kids(e) {
                count(k, n);
            }
        }
        if !call.args.iter().any(|arg| matches!(arg, Expr::Move(m) if !m.fields.is_empty())) {
            return Ok(None);
        }
        let mut n = 0;
        for arg in &call.args {
            count(arg, &mut n);
        }
        if n != 1 {
            return Ok(None);
        }
        let found = call.args.iter().enumerate().find_map(|(i, arg)| match arg {
            Expr::Move(m) if !m.fields.is_empty() => Some((i, m)),
            _ => None,
        });
        let Some((i, m)) = found else { return Ok(None) };
        let take = if let Some(takes) = self.takes.get(&call.name) {
            takes.get(i).copied().unwrap_or(false)
        } else {
            catalog_fn(&call.name).and_then(|f| f.params.get(i).map(|p| p.take)).unwrap_or(false)
        };
        if !take || call.args.iter().enumerate().any(|(j, arg)| j != i && mentions(arg, &m.name)) {
            return Ok(None);
        }
        if matches!(next, Some(Stmt::FieldAssign(r)) if r.name == m.name && r.fields == m.fields) {
            return Ok(None);
        }
        self.check_borrow(m.line, m.col, &m.name, &m.fields)?;
        let source = self.lookup(m.line, m.col, &m.name, scope, &m.fields)?;
        if !((source.kind == "local" && source.mutable) || source.kind == "take") || self.arm_binders.contains(&m.name) {
            return Ok(None);
        }
        let mut ty = source.ty.clone();
        for field in &m.fields {
            if !self.is_struct(&ty) {
                return Ok(None);
            }
            ty = self.field_type(m.line, m.col, &ty, field)?;
        }
        let typed = matches!(ty.to_key().as_str(), "string" | "vec<i64>" | "vec<bool>")
            || (ty.args.is_empty() && crate::typeinfo::is_array_name(&ty.name))
            || crate::gtype::is_tuple_name(&ty.name); // P6: a tuple component moves like a buffer field
        Ok(typed.then_some((m.line, m.col)))
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
        if self.ref_path(&source.ty, &m.fields) {
            return Err(terr(m.line, m.col, "array_alias", "a borrowed view cannot be moved", ""));
        }
        let typed = matches!(ty.to_key().as_str(), "string" | "vec<i64>" | "vec<bool>")
            || (ty.args.is_empty() && crate::typeinfo::is_array_name(&ty.name))
            || crate::gtype::is_tuple_name(&ty.name); // P6: a tuple component moves like a buffer field
        let restored = matches!(next, Some(Stmt::FieldAssign(r)) if r.name == m.name && r.fields == m.fields);
        if typed
            && !restored
            && target.is_none()
            && ((((source.kind == "local" && source.mutable) || source.kind == "take")
                && !self.arm_binders.contains(&m.name))
                // LB9 (R-6g8-4): or a struct binder of a consuming match (the arm owns the payload; see Match)
                || (source.kind == "local" && self.match_movable.contains(&m.name)))
        {
            // G-R09-4: `let t = move d.a;` of a `let mut` local record (LG9: or a take record parameter, which the
            // callee owns; LG9 G-LG8-1: any owned buffer field type, any field path) consumes d; foreign.rs marks d
            // moved
            return Ok(Some((m.line, m.col)));
        }
        if !(source.mutable || matches!(source.kind, "inout" | "map_inout"))
            || !typed
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
            if f.is_some() || array_grow_builtin(&call.name)
                || matches!(call.name.as_str(), "map_remove" | "map_take" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice" | "ev_read_into" | "ev_proc_read_into") {
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

    /// P4/P5: checks a loop body (a `for` loop keeps its binder read-only and its collection held).
    fn loop_body(&mut self, s: &mut While, scope: &mut Scope) -> Res<()> {
        let mut sc = scope.clone();
        match s.for_head.clone() {
            None => self.block(&mut s.body, &mut sc)?,
            Some(head) if head.slice => {
                let (root, path) = for_collection(&s.body);
                self.for_holds.push((root, path));
                let r = self.block(&mut s.body, &mut sc);
                self.for_holds.pop();
                r?;
            }
            Some(head) => {
                if let Some(e) = sc.get_mut(&head.var) {
                    e.mutable = false;
                }
                self.for_steps.push((s.line, s.col, head.var));
                let r = self.block(&mut s.body, &mut sc);
                self.for_steps.pop();
                r?;
            }
        }
        Ok(())
    }

    /// P5: a `break`/`continue` needs a target loop it may leave, and ends its block.
    fn loop_jump(&self, j: &Jump, what: &str, next: Option<&Stmt>) -> Res<()> {
        let frame = match &j.label {
            None => self.loop_frames.last(),
            Some(l) => self.loop_frames.iter().rev().find(|(n, _, _, _)| n.as_ref() == Some(l)),
        };
        let Some(&(_, roots, iters, _)) = frame else {
            return Err(match &j.label {
                None => terr(j.line, j.col, "loop_jump_outside", format!("`{}` outside a while or for loop", what),
                    "use `break`/`continue` only inside a while or for loop body"),
                Some(l) => terr(j.line, j.col, "loop_label_unknown",
                    format!("no enclosing loop has the label '{}", l),
                    "label an enclosing loop with `'name: while` or `'name: for`"),
            });
        };
        if self.map_iterations.len() > iters {
            return Err(terr(j.line, j.col, "loop_jump_map",
                format!("`{}` cannot leave a map iteration `for (k, v) in m`", what),
                "map iterations always visit every entry; use a flag in the invariant instead"));
        }
        if self.map_roots.len() > roots {
            return Err(terr(j.line, j.col, "loop_jump_map",
                format!("`{}` cannot leave a mutable collection Some arm", what),
                "end the arm first, then break or continue after the match"));
        }
        // the synthesized step of the innermost `for` follows a jump at the end of its body
        let step_next = match (next, self.loop_frames.last()) {
            (Some(Stmt::Assign(a)), Some((_, _, _, Some((l, c, n))))) => a.line == *l && a.col == *c && a.name == *n,
            _ => false,
        };
        if next.is_some() && !step_next {
            return Err(terr(j.line, j.col, "loop_jump_unreachable",
                format!("statements after `{}` in the same block never run", what),
                "remove them or move them before the jump"));
        }
        Ok(())
    }

    fn stmt(&mut self, st: &mut Stmt, scope: &mut Scope, next: Option<&Stmt>) -> Res<()> {
        self.ckind = "code";
        self.boundary = None; // G5: only the statements below that allow a mutating call set it
        let position = Self::check_try_position;
        match &*st {
            Stmt::Let(s) => position(&s.value, s.line, s.col)?,
            Stmt::Assign(s) => position(&s.value, s.line, s.col)?,
            Stmt::IndexAssign(s) => {
                position(&s.index, s.line, s.col)?;
                position(&s.value, s.line, s.col)?;
            }
            Stmt::FieldAssign(s) => position(&s.value, s.line, s.col)?,
            Stmt::If(s) => position(&s.cond, s.line, s.col)?,
            Stmt::While(s) => {
                for e in std::iter::once(&s.cond).chain(&s.invariants).chain([&s.decreases]) {
                    position(e, s.line, s.col)?;
                }
            }
            Stmt::ForMap(s) => {
                for e in std::iter::once(&s.place).chain(&s.invariants) {
                    position(e, s.line, s.col)?;
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(s) => {
                if let Some(v) = &s.value {
                    position(v, s.line, s.col)?;
                }
            }
            Stmt::Assert(s) => position(&s.cond, s.line, s.col)?,
            Stmt::ExprStmt(s) => position(&s.expr, s.line, s.col)?,
            Stmt::Par(s) => {
                for call in &s.calls {
                    position(call, call.line(), call.col())?;
                }
            }
            Stmt::Match(s) => position(&s.scrutinee, s.line, s.col)?,
        }
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
                if s.is_place() {
                    let Expr::InoutArg(arg) = &mut s.value else { unreachable!() };
                    // R-6g9-6: a place alias would bypass the target's provenance check.
                    if self.check_place(&mut arg.expr, scope)? || self.borrowed_type_argument(&sty) {
                        return Err(terr(s.line, s.col, "place_binding",
                            "a place binding cannot hold a borrowed view or a borrowing struct",
                            "bind an owned field of it, or write the struct directly"));
                    }
                    scope.insert(s.name.clone(), Entry { ty: sty, mutable: true, kind: "inout" });
                    return Ok(());
                }
                if sty == unitt() {
                    return Err(terr(s.line, s.col, "unit_let", "cannot bind a () value", ""));
                }
                self.check_type(s.line, s.col, &sty)?;
                self.ext_misuse(s.line, s.col, &sty, None)?; // G9 A0
                self.boundary = boundary_of(&s.value);
                if !s.borrowed { self.want_alloc(&s.value, &sty); }
                self.field_move = match &s.value {
                    Expr::Call(call) => self.field_move_arg(call, scope, next)?, // LG9 (G-LG8-1)
                    v => self.field_move_out(v, &sty, None, scope, next)?,
                };
                let checked = if s.borrowed && sty.view_payload().is_none() {
                    self.view_value(&mut s.value, &sty, scope, false)
                } else {
                    // R-6g9-3: a static text local is the literal's read-only global
                    self.static_lit = matches!(s.value, Expr::StrLit(_)) && self.static_lets.contains(&s.name);
                    let r = self.expect(&mut s.value, &sty, scope, false, None).map(|_| ());
                    self.static_lit = false;
                    r
                };
                self.field_move = None;
                checked?;
                self.check_inout_context(&[&s.value], InoutContext::Value)?;
                if !s.borrowed && (sty.is_array() || sty == strt()) {
                    self.array_source(s.line, s.col, &s.value, scope, false, &sty)?;
                }
                if !s.borrowed {
                    self.own(&s.value, scope)?;
                }
                scope.insert(s.name.clone(), Entry { ty: sty, mutable: s.mutable, kind: if s.borrowed { "ref_local" } else { "local" } });
            }
            Stmt::Assign(s) => {
                let v = self.lookup(s.line, s.col, &s.name, scope, &[])?;
                self.check_borrow(s.line, s.col, &s.name, &[])?; // G9 A0
                let scalar_binder = v.kind == "map_inout" && (v.ty == i64t() || v.ty == boolt());
                let record_binder = v.kind == "map_inout"
                    && (self.owned_aggregate(&v.ty) || self.recursive_vec_element(&v.ty));
                let scalar_inout = v.kind == "inout" && (v.ty.int_width().is_some() || v.ty == boolt());
                if s.deref && !(scalar_binder || record_binder) {
                    return Err(terr(s.line, s.col, "type_mismatch",
                        "dereference requires a mutable scalar or record collection binder", ""));
                }
                let array_inout = v.kind == "inout" && v.ty.is_array();
                let for_step = self.for_steps.iter().any(|(l, c, n)| (*l, *c) == (s.line, s.col) && *n == s.name);
                if !(v.mutable || scalar_binder || record_binder || scalar_inout || array_inout || for_step) {
                    if self.for_steps.iter().any(|(_, _, n)| *n == s.name) {
                        return Err(terr(s.line, s.col, "immutable",
                            format!("the for loop variable {} cannot be assigned in the loop body", s.name),
                            "copy it into a `let mut` local, or use a while loop"));
                    }
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
                self.array_replace = array_inout.then(|| s.name.clone());
                let checked = self.expect(&mut s.value, &v.ty, scope, false, None);
                self.array_replace = None;
                checked?;
                if record_binder && !matches!(&s.value, Expr::StructLit(_) | Expr::EnumCtor(_) | Expr::Call(_))
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
            Stmt::IndexAssign(s) if !s.fields.is_empty() => {
                // LG4: `p.a.b[i] = e` -- the place is the field path of a mutable local / inout parameter
                let v = self.lookup(s.line, s.col, &s.name, scope, &s.fields)?;
                self.check_borrow(s.line, s.col, &s.name, &s.fields)?;
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
                    // R-6g9-6: a mutable struct can replace its views, but cannot write their elements.
                    if self.ref_field(&cur.name, fname) {
                        return Err(terr(s.line, s.col, "immutable", "a borrowed view is read-only", ""));
                    }
                    cur = self.field_type(s.line, s.col, &cur, fname)?;
                }
                let place = format!("{}.{}", s.name, s.fields.join("."));
                if cur == strt() {
                    return Err(terr(
                        s.line,
                        s.col,
                        "immutable_string",
                        format!("{} is a str; its bytes cannot be changed", place),
                        "build a new str with substr() or copy() instead",
                    ));
                }
                if !(cur.args.is_empty() && crate::typeinfo::is_array_name(&cur.name)) && !cur.is_fixed() {
                    return Err(terr(s.line, s.col, "not_array", format!("{} is not an array", place), ""));
                }
                if !v.mutable && !matches!(v.kind, "inout" | "map_inout") {
                    return Err(terr(s.line, s.col, "immutable", format!("{} is not mutable", s.name), Self::immutable_fix(v.kind)));
                }
                self.expect_index(&mut s.index, scope, false)?;
                let want_val = elem_type(&cur);
                self.expect(&mut s.value, &want_val, scope, false, None)?;
                if Self::is_iteration_binder(&s.value, scope) {
                    self.own(&s.value, scope)?;
                }
                self.check_inout_context(&[&s.index, &s.value], InoutContext::PlaceAssignment(&s.name, &s.fields))?;
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
                if !v.ty.is_array() && !v.ty.is_fixed() {
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
                self.expect_index(&mut s.index, scope, false)?;
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
                let mut ref_target = false;
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
                    ref_target = self.ref_field(&cur.name, fname);
                    cur = self.field_type(s.line, s.col, &cur, fname)?;
                }
                // R-6g9-6: a ref field takes a view and the borrow pass checks its provenance.
                s.ref_store = ref_target || self.decls.get(&cur.name).is_some_and(crate::borrowed::borrowing_struct);
                let moved = if ref_target {
                    None
                } else {
                    self.field_move_out(&s.value, &cur, Some((&s.name, &s.fields)), scope, next)?
                };
                if !v.mutable && !matches!(v.kind, "inout" | "map_inout") {
                    if let Some(m) = first_field_move(&s.value) {
                        self.check_borrow(m.line, m.col, &m.name, &m.fields)?;
                        self.lookup(m.line, m.col, &m.name, scope, &m.fields)?;
                        return Err(field_move_error(m));
                    }
                    return Err(terr(s.line, s.col, "immutable", format!("{} is not mutable", s.name), Self::immutable_fix(v.kind)));
                }
                if ref_target {
                    self.view_value(&mut s.value, &cur, scope, false)?;
                    return self.check_inout_context(&[&s.value], InoutContext::PlaceAssignment(&s.name, &s.fields));
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
                self.want_alloc(&s.value, &cur);
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
                if for_loop_of(s).is_some() {
                    self.for_types(s, scope)?;
                }
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
                self.expect_measure(&mut s.decreases, scope, true)?;
                self.ckind = "code";
                if let Some(l) = &s.label {
                    if self.loop_frames.iter().any(|(n, _, _, _)| n.as_ref() == Some(l)) {
                        return Err(terr(s.line, s.col, "loop_label_duplicate",
                            format!("loop label '{} is already used by an enclosing loop", l),
                            "give the inner loop a different label"));
                    }
                }
                let step = s.for_head.as_ref().map(|h| (s.line, s.col, h.counter.clone()));
                self.loop_frames.push((s.label.clone(), self.map_roots.len(), self.map_iterations.len(), step));
                let r = self.loop_body(s, scope);
                self.loop_frames.pop();
                r?;
            }
            Stmt::Break(j) => self.loop_jump(j, "break", next)?,
            Stmt::Continue(j) => self.loop_jump(j, "continue", next)?,
            Stmt::ForMap(s) => self.for_map(s, scope)?,
            Stmt::Return(s) => {
                // LG9 (G-LG8-1): `return f(move d.a);` with a take parameter
                self.field_move = match &s.value {
                    Some(Expr::Call(call)) => self.field_move_arg(call, scope, None)?,
                    _ => None,
                };
                if let Some(m) = s.value.as_ref().and_then(first_field_move).filter(|_| self.field_move.is_none()) {
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
                        let checked = if self.borrowed_result && ret.view_payload().is_none() {
                            self.view_value(v, &ret, scope, false)
                        } else {
                            self.want_alloc(v, &ret);
                            self.expect(v, &ret, scope, false, None).map(|_| ())
                        };
                        self.field_move = None;
                        checked?;
                        self.check_inout_context(&[&*v], InoutContext::Value)?;
                        // LG9 (G-DB15-2): a str binder of a consuming match is returned like an owned local
                        let binder_ret = ret == strt() && matches!(&*v, Expr::Var(x)
                            if self.match_movable.contains(&x.name)
                                && scope.get(&x.name).is_some_and(|entry| entry.ty == strt()));
                        if !self.borrowed_result && (ret.is_array() || ret == strt()) && !binder_ret {
                            self.array_source(s.line, s.col, v, scope, true, &ret)?;
                        }
                        if binder_ret {
                            self.used_effects.insert("alloc".to_string());
                        }
                        self.own(v, scope)?;
                        if binder_ret {
                            // it means `return copy(x)`: rewrite it so both backends and the cost proof see the copy
                            let (line, col) = match &*v { Expr::Var(x) => (x.line, x.col), _ => unreachable!() };
                            *v = Expr::Call(Call {
                                line, col, name: "copy".to_string(), args: vec![v.clone()],
                                ty: Some("str".to_string()), targs: None, inst: None, old: None, opaque_c: false,
                            });
                        }
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
            Stmt::ExprStmt(s) if matches!(&s.expr, Expr::Call(c) if crate::ast::ValueMatch::is_yield(c)) => {
                // P9a-5: an arm of a value `match`, checked in the arm's scope (binders in place)
                let Expr::Call(c) = &mut s.expr else { unreachable!() };
                self.value_match_arm(&mut c.args[0], scope)?;
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
                self.field_move = match &s.expr {
                    Expr::Call(call) => self.field_move_arg(call, scope, next)?, // LG9 (G-LG8-1): `f(move d.a);`
                    _ => None,
                };
                let result = self.expr(&mut s.expr, scope, false);
                self.field_move = None;
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
            if let Expr::Call(c) = e {
                for a in &c.args {
                    let value = match a {
                        Expr::InoutArg(arg) => &*arg.expr,
                        _ => a,
                    };
                    let place_binding = place_root(value).and_then(|name| scope.get(name))
                        .is_some_and(|entry| entry.kind == "inout" && entry.mutable);
                    if place_binding
                        || matches!(a, Expr::Var(x) if scope.get(&x.name).is_some_and(|e| e.kind == "ref_local"))
                        || matches!(a, Expr::FieldAccess(x) if self.is_borrowed_field(&x.base, &x.field))
                    {
                        return Err(terr(a.line(), a.col(), "borrow_signature", "borrowed values cannot cross a par branch boundary", ""));
                    }
                }
            }
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
        } else if ["len", "alloc", "copy", "print", "is_char_boundary", "substr", "utf8_valid", "utf8_valid_span", "str_from_utf8", "truncate", "eq_range", "find_byte", "resize", "copy_range", "copy_within", "arr_reserve", "arr_push", "arr_append", "subview"].contains(&n.as_str())
            || is_bit_builtin(n)
            || is_word_builtin(n)
            || crate::intwidth::of(n).is_some()
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
            || (t.args.is_empty() && t.is_array())
            || (t.name == "array" && t.args.len() == 1
                && t.args[0].owner.is_some() && is_element(&t.args[0], &self.elem_params))
    }

    fn is_borrowed_field(&self, base: &Expr, field: &str) -> bool {
        expr_ty(base).as_deref().is_some_and(|ty| self.ref_field(ty, field))
    }

    /// R-6g9-6: whether a field path runs through a ref field, a shared view its record does not own.
    fn ref_path(&self, ty: &GType, fields: &[String]) -> bool {
        let Some((first, rest)) = fields.split_first() else { return false };
        self.ref_field(&ty.name, first)
            || self.field_type(0, 0, ty, first).is_ok_and(|next| self.ref_path(&next, rest))
    }

    fn ref_field(&self, ty: &str, field: &str) -> bool {
        matches!(self.decls.get(ty),
            Some(TypeDecl::Struct(s)) if s.fields.iter().any(|f| f.name == field && f.borrowed))
    }

    fn check_inout_view(&self, arg: &InoutArg) -> Res<()> {
        // R-6g9-6: a mutable record still holds shared views into their sources.
        if matches!(&*arg.expr, Expr::FieldAccess(f) if self.is_borrowed_field(&f.base, &f.field)) {
            return Err(terr(arg.line, arg.col, "inout_argument", "a borrowed view cannot be passed inout", ""));
        }
        Ok(())
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
                if t.as_ref().and_then(|t| self.decls.get(&t.name)).is_some_and(crate::borrowed::borrowing_struct) {
                    return Err(terr(arg.line(), arg.col(), "borrow_signature", "borrowed values cannot cross a par branch boundary", ""));
                }
                if !t.as_ref().map_or(false, |t| self.par_type_ok(t)) {
                    return Err(terr(arg.line(), arg.col(), "parallel_argument",
                        format!("an argument of type {} cannot cross a par branch boundary",
                            t.as_ref().map(tn).unwrap_or("?".into())),
                        "bind a scalar first, or pass whole [i64], [u8], [u16] or [bool] arrays"));
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

    /// P9a-6: whether a match needs `lower_patterns` (a `_` arm, or a `_`, literal or nested sub-pattern).
    fn has_ext_pattern<'a>(mut arms: impl Iterator<Item = (&'a str, &'a [String])>) -> bool {
        arms.any(|(ctor, bs)| ctor == "_" || !bs.iter().all(|b| crate::ast::SubPat::is_plain(b)))
    }

    /// P9a-6: the constructors of a matched type with their payload types: `Some`/`None`, `Ok`/`Err` (a unit `Ok`
    /// binds nothing) or the variants of a user enum. Anything else is type/match_type.
    fn pat_ctors(&self, t: &GType, line: i64, col: i64) -> Res<Vec<(Option<String>, String, Vec<GType>)>> {
        if t.owner.is_some() {
            return Err(terr(line, col, "generic_operation", format!("cannot match a value of the abstract type {}", t.name),
                "a type parameter has no variants; match on an Option, Result or enum that contains it"));
        }
        if t.name == "option" {
            let p = t.view_payload().or_else(|| t.args.first()).cloned().unwrap_or_else(i64t);
            return Ok(vec![(None, "Some".into(), vec![p]), (None, "None".into(), Vec::new())]);
        }
        if t.name == "result" {
            let p = t.view_payload().or_else(|| t.args.first()).cloned().unwrap_or_else(i64t);
            let e = t.result_error().cloned().unwrap_or_else(i64t);
            let ok = if p == unitt() { Vec::new() } else { vec![p] };
            return Ok(vec![(None, "Ok".into(), ok), (None, "Err".into(), vec![e])]);
        }
        if self.is_enum(t) {
            if let Some(TypeDecl::Enum(e)) = self.tt.decl(&t.name) {
                let names: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
                let mut out = Vec::new();
                for v in names {
                    let fs = self.fields_of(t, Some(v.as_str()))?.into_iter().map(|(_, ft)| ft).collect();
                    out.push((Some(t.name.clone()), v, fs));
                }
                return Ok(out);
            }
        }
        Err(terr(line, col, "match_type", format!("a variant pattern cannot match a value of type {}", tn(t)), ""))
    }

    /// P9a-6: lowers a match whose arms use `_`, literal or nested patterns to a decision tree of plain matches and
    /// `if`s, tried in arm order (the first arm that matches wins, as in Rust). Every value must reach an arm
    /// (type/match_arms names a missing case) and every arm must be reachable (type/unreachable_arm). `arms` are the
    /// arm patterns (ctor `_` for a wildcard arm), `tuple` the P8 tuple patterns of plain binders; `scrut` the
    /// scrutinee and `t` its type. Returns the tree; leaves name their arm and the lets that bind its names.
    fn lower_patterns(&mut self, arms: &[(i64, i64, Option<String>, String, Vec<String>)], uses: &[HashSet<String>],
        split_lone: bool, t: &GType, line: i64, col: i64) -> Res<PNode>
    {
        use crate::ast::SubPat;
        // an arm's own pattern names count as mentioned (two fields of one case never share a kept name)
        let uses: Vec<HashSet<String>> = uses.iter().zip(arms).map(|(u, a)| {
            let mut u = u.clone();
            a.4.iter().for_each(|b| pat_slot_names(b, &mut u));
            u
        }).collect();
        let uses = &uses[..];
        let mut rows = Vec::new();
        for (k, (_, _, en, ctor, bs)) in arms.iter().enumerate() {
            let top = if ctor == "_" { SubPat::Wild } else {
                SubPat::Variant(en.clone(), ctor.clone(), bs.iter().map(|b| SubPat::decode(b)).collect())
            };
            let mut row = PRow { tests: Vec::new(), binds: Vec::new(), arm: k };
            row.add(String::new(), t.clone(), top);
            rows.push(row);
        }
        let mut used = vec![false; arms.len()];
        let mut budget = 4096usize;
        let mut node = self.pat_compile(rows, &mut used, &mut budget, &mut Vec::new(), arms, uses, line, col)?;
        if let Some(k) = used.iter().position(|u| !u) {
            let (l, c) = (arms[k].0, arms[k].1);
            return Err(terr(l, c, "unreachable_arm", "this arm can never match: the arms above take every value it matches",
                "remove the arm or move it above the arm that covers it"));
        }
        if let (true, PNode::Leaf(k, binds)) = (split_lone, &node) {
            // a lone `_` arm on an Option/Result/enum still matches the scrutinee (evaluated once, as in Rust): one
            // case per constructor, payloads bound to fresh names (a place scrutinee has nothing to evaluate, so the
            // statement form keeps the bare arm and the place stays assignable)
            if let Ok(ctors) = self.pat_ctors(t, line, col) {
                let mut out = Vec::new();
                for (cen, ctor, fs) in ctors {
                    let names = fs.iter().map(|_| { self.pat_tmp += 1; format!("pruv_pat{}", self.pat_tmp) }).collect();
                    out.push((cen, ctor, names, PNode::Leaf(*k, binds.clone())));
                }
                node = PNode::Match(String::new(), out);
            }
        }
        Ok(node)
    }

    #[allow(clippy::too_many_arguments)]
    fn pat_compile(&mut self, rows: Vec<PRow>, used: &mut [bool], budget: &mut usize, path: &mut Vec<String>,
        arms: &[(i64, i64, Option<String>, String, Vec<String>)], uses: &[HashSet<String>], line: i64, col: i64)
        -> Res<PNode>
    {
        use crate::ast::SubPat;
        if *budget == 0 {
            return Err(terr(line, col, "match_arms", "the patterns of this match need too many cases",
                "split the match into nested matches"));
        }
        *budget -= 1;
        let Some(r0) = rows.first() else {
            let at = if path.is_empty() { String::new() } else { format!(" ({})", path.join(", ")) };
            return Err(terr(line, col, "match_arms", format!("match does not cover every value{}", at),
                "add the missing case or a `_` arm"));
        };
        let Some((v, ty, pat)) = r0.tests.first().cloned() else {
            used[r0.arm] = true;
            return Ok(PNode::Leaf(r0.arm, r0.binds.clone()));
        };
        let test_of = |r: &PRow| r.tests.iter().position(|(x, _, _)| *x == v);
        let (al, ac) = (arms[r0.arm].0, arms[r0.arm].1);
        let label = |v: &str| if v.is_empty() { "the value".to_string() } else { "a payload".to_string() };
        match pat {
            SubPat::Variant(..) => {
                let ctors = self.pat_ctors(&ty, al, ac)?;
                // every pattern on `v` names one of these constructors with its arity
                for r in &rows {
                    let Some(i) = test_of(r) else { continue };
                    let (l, c) = (arms[r.arm].0, arms[r.arm].1);
                    match &r.tests[i].2 {
                        SubPat::Variant(en, ctor, subs) => {
                            let Some((cen, _, fs)) = ctors.iter().find(|(_, n, _)| n == ctor) else {
                                return Err(terr(l, c, if ctors[0].0.is_some() { "unknown_variant" } else { "match_arms" },
                                    format!("{} has no variant {}", tn(&ty), ctor), ""));
                            };
                            if en != cen {
                                let what = match en { Some(e) => format!("{}::{}", e, ctor), None => ctor.clone() };
                                return Err(terr(l, c, "match_arms", format!("arm {} does not belong to {}", what, tn(&ty)),
                                    match cen { Some(e) => format!("write {}::Variant", e), None => String::new() }));
                            }
                            if subs.len() != fs.len() {
                                return Err(terr(l, c, "match_arity",
                                    format!("{} has {} field(s), the pattern binds {}", ctor, fs.len(), subs.len()),
                                    "bind every field, in declaration order (`Ok(())` for a unit result)"));
                            }
                        }
                        _ => return Err(terr(l, c, "type_mismatch",
                            format!("a literal pattern cannot match {} of type {}", label(&v), tn(&ty)), "")),
                    }
                }
                let mut out = Vec::new();
                for (cen, ctor, fs) in ctors {
                    // a field keeps its source name when the rows that reach this case and bind it all use that
                    // name and no row that reaches it without binding the field refers to the name (so no other
                    // arm body sees it); otherwise it gets a fresh name and the arms bind their own names with lets
                    let mut names = Vec::new();
                    for j in 0..fs.len() {
                        let mut named: Option<String> = None;
                        let mut ok = true;
                        let mut others = Vec::new();
                        for r in &rows {
                            match test_of(r).map(|i| &r.tests[i].2) {
                                Some(SubPat::Variant(_, c, _)) if *c != ctor => {}
                                Some(SubPat::Variant(_, _, subs)) => match (&subs[j], &named) {
                                    (SubPat::Bind(n), None) => named = Some(n.clone()),
                                    (SubPat::Bind(n), Some(m)) if n == m => {}
                                    (SubPat::Bind(_), Some(_)) => ok = false,
                                    _ => others.push(r.arm),
                                },
                                _ => others.push(r.arm),
                            }
                        }
                        if let Some(n) = &named {
                            ok &= others.iter().all(|k| !uses[*k].contains(n));
                        }
                        names.push(match named {
                            Some(n) if ok => n,
                            _ => {
                                self.pat_tmp += 1;
                                format!("pruv_pat{}", self.pat_tmp)
                            }
                        });
                    }
                    let mut spec = Vec::new();
                    for r in &rows {
                        match test_of(r) {
                            None => spec.push(r.clone()),
                            Some(i) => {
                                let SubPat::Variant(_, c, subs) = &r.tests[i].2 else { unreachable!("checked above") };
                                if *c != ctor {
                                    continue;
                                }
                                let mut r2 = r.clone();
                                r2.tests.remove(i);
                                for (j, s) in subs.iter().enumerate() {
                                    r2.add(names[j].clone(), fs[j].clone(), s.clone());
                                }
                                spec.push(r2);
                            }
                        }
                    }
                    path.push(match &cen { Some(e) => format!("{}::{}", e, ctor), None => ctor.clone() });
                    let child = self.pat_compile(spec, used, budget, path, arms, uses, line, col)?;
                    path.pop();
                    out.push((cen, ctor, names, child));
                }
                Ok(PNode::Match(v, out))
            }
            SubPat::Bool(_) | SubPat::Int(_) => {
                let is_bool = ty == boolt();
                let int = ty.int_width().is_some();
                if !is_bool && !int {
                    return Err(terr(al, ac, "type_mismatch",
                        format!("a literal pattern cannot match {} of type {}", label(&v), tn(&ty)), ""));
                }
                // the literal of the first row decides the split; a pattern of the other kind is a type error
                let key = pat.clone();
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for r in &rows {
                    let Some(i) = test_of(r) else {
                        yes.push(r.clone());
                        no.push(r.clone());
                        continue;
                    };
                    let p = &r.tests[i].2;
                    let ok_kind = if is_bool { matches!(p, SubPat::Bool(_)) } else { matches!(p, SubPat::Int(_)) };
                    if !ok_kind {
                        let (l, c) = (arms[r.arm].0, arms[r.arm].1);
                        // a constructor on a scalar is a match on a non-enum (as for the first arm); a literal of the
                        // other kind is a type mismatch
                        let code = if matches!(p, SubPat::Variant(..)) { "match_type" } else { "type_mismatch" };
                        return Err(terr(l, c, code,
                            format!("this pattern cannot match {} of type {}", label(&v), tn(&ty)), ""));
                    }
                    let same = match (p, &key) {
                        (SubPat::Int(a), SubPat::Int(b)) => a.parse::<i128>().ok() == b.parse::<i128>().ok(),
                        _ => *p == key,
                    };
                    let mut r2 = r.clone();
                    r2.tests.remove(i);
                    if same {
                        yes.push(r2);
                    } else if is_bool {
                        no.push(r2); // the other bool literal
                    } else {
                        no.push(r.clone());
                    }
                }
                let lit = match &key { SubPat::Int(x) => x.clone(), SubPat::Bool(b) => b.to_string(), _ => unreachable!() };
                path.push(format!("{} == {}", label(&v), lit));
                let a = self.pat_compile(yes, used, budget, path, arms, uses, line, col)?;
                path.pop();
                path.push(format!("{} != {}", label(&v), lit));
                let b = self.pat_compile(no, used, budget, path, arms, uses, line, col)?;
                path.pop();
                Ok(PNode::Test(v, key, Box::new(a), Box::new(b)))
            }
            SubPat::Bind(_) | SubPat::Wild => unreachable!("PRow::add keeps no binder tests"),
        }
    }

    /// P9a-6: a `match` statement with `_`, literal or nested patterns, lowered by `lower_patterns`. A tree that is a
    /// single leaf (a lone `_` arm) is wrapped in `if true { .. }` so its lets stay in the arm.
    fn pattern_match(&mut self, m: &Match, scope: &Scope) -> Res<Stmt> {
        let mut probe = m.scrutinee.clone();
        let t = self.match_scrutinee(&mut probe, scope)?;
        let arms: Vec<_> = m.arms.iter().map(|a| (a.line, a.col, a.enum_name.clone(), a.ctor.clone(), a.binders.clone())).collect();
        let uses: Vec<HashSet<String>> = m.arms.iter().map(|a| {
            let mut u = HashSet::new();
            pat_names_stmts(&a.body, &mut u);
            a.pats.iter().flatten().for_each(|p| pat_tuple_names(p, &mut u));
            u
        }).collect();
        let node = self.lower_patterns(&arms, &uses, place_root(&m.scrutinee).is_none(), &t, m.line, m.col)?;
        let bodies: Vec<Vec<Stmt>> = m.arms.iter().map(|a| a.body.clone()).collect();
        let tuples: Vec<Vec<(String, TuplePat)>> = m.arms.iter().map(|a| a.binders.iter().zip(a.pats.iter())
            .filter_map(|(b, p)| p.clone().map(|p| (b.clone(), p))).collect()).collect();
        let mut out = pat_stmts(&node, &m.scrutinee, &bodies, &tuples, m.line, m.col)?;
        if let PNode::Leaf(..) = node {
            if place_root(&m.scrutinee).is_none() {
                // the scrutinee is still evaluated, as in Rust
                self.pat_tmp += 1;
                out.insert(0, Stmt::Let(Let { borrowed: false, line: m.line, col: m.col, name: format!("pruv_pat{}", self.pat_tmp),
                    mutable: false, ty: t.to_key(), value: m.scrutinee.clone(), refine: None }));
            }
            return Ok(Stmt::If(If { line: m.line, col: m.col,
                cond: Expr::BoolLit(BoolLit { line: m.line, col: m.col, value: true, ty: None }), then: out, els: Vec::new() }));
        }
        let Some(st) = out.pop() else { unreachable!("a lowered match") };
        Ok(st)
    }

    fn match_scrutinee(&mut self, e: &mut Expr, scope: &Scope) -> Res<GType> {
        self.read_expr(e, scope, false, &["map_get", "map_get_mut", "map_take", "vec_at_mut", "vec_get"])
    }

    /// P8: `match t { (a, b) => { B } }` is the destructuring `if true { let tmp: T = t; let a = tmp.0; ..; B }`
    /// (one shape, one arm; the components follow the match-pattern rule of `lower_arm_pat`).
    fn tuple_match(&mut self, m: &Match, scope: &Scope) -> Res<Stmt> {
        if m.arms.len() != 1 {
            return Err(terr(m.line, m.col, "match_arms", "a tuple has one shape, so its match has exactly one arm",
                "write one `(a, b) =>` arm"));
        }
        let a = &m.arms[0];
        let mut probe = m.scrutinee.clone();
        let t = self.match_scrutinee(&mut probe, scope)?;
        let tmp = a.binders[0].clone();
        let Some(Some(TuplePat::Tuple(l, c, ps))) = a.pats.first() else {
            return Err(terr(a.line, a.col, "match_arms", "a tuple arm is written (a, b) =>", ""));
        };
        let mut then = vec![Stmt::Let(Let { borrowed: false, line: a.line, col: a.col, name: tmp.clone(), mutable: false,
            ty: t.to_key(), value: m.scrutinee.clone(), refine: None })];
        lower_arm_pat(&tmp, ps, &t, *l, *c, &mut then)?;
        then.extend(a.body.iter().cloned());
        Ok(Stmt::If(If { line: m.line, col: m.col,
            cond: Expr::BoolLit(BoolLit { line: m.line, col: m.col, value: true, ty: None }), then, els: Vec::new() }))
    }

    /// P8b: a match of slice patterns over an array place `s` is the length test chain
    /// `if len(s) == k { let a: T = s[0]; .. A } else if len(s) >= k { .. let tail: ref [T] = subview(s, k, len(s)); B }
    /// else { .. }`, tested in source order; the last arm takes what the earlier arms leave and has no test. Lengths
    /// must be covered and every arm must match some length (type/match_arms).
    fn slice_match(&mut self, m: &Match, scope: &Scope) -> Res<Stmt> {
        let (line, col) = (m.line, m.col);
        if let Some(a) = m.arms.iter().find(|a| slice_arm_shape(&a.ctor).is_none()) {
            return Err(terr(a.line, a.col, "match_type", "a match mixes slice patterns with other patterns",
                "write every arm as a slice pattern"));
        }
        if !matches!(m.scrutinee, Expr::Var(_) | Expr::FieldAccess(_)) || place_root(&m.scrutinee).is_none() {
            return Err(terr(line, col, "match_type",
                "a slice pattern matches a local, a parameter or a field path rooted at one",
                "bind the value with a let first"));
        }
        let mut probe = m.scrutinee.clone();
        let t = self.match_scrutinee(&mut probe, scope)?;
        if let Some((el, n)) = t.fixed_parts() {
            return self.fixed_slice_match(m, el.clone(), n);
        }
        let elem = t.array_elem().filter(|_| t.args.is_empty() && t.is_array());
        let Some(elem) = elem else {
            return Err(terr(line, col, "match_type",
                format!("a slice pattern matches an array ([i64], [u8], [u16] or [bool]), found {}", tn(&t)),
                "match a slice place, or use an if on len(..)"));
        };
        let (mut exact, mut min_rest): (BTreeSet<usize>, Option<usize>) = (BTreeSet::new(), None);
        for a in &m.arms {
            let k = a.binders.len();
            let rest = slice_arm_shape(&a.ctor).is_some_and(|r| r.is_some());
            let shadowed = if rest {
                min_rest.is_some_and(|r| r <= k || (k..r).all(|n| exact.contains(&n)))
            } else {
                exact.contains(&k) || min_rest.is_some_and(|r| r <= k)
            };
            if shadowed {
                return Err(terr(a.line, a.col, "match_arms", "this slice pattern only matches lengths an earlier arm takes",
                    "remove the arm or move it before the arm that covers it"));
            }
            if rest {
                min_rest = Some(min_rest.map_or(k, |r| r.min(k)));
            } else {
                exact.insert(k);
            }
        }
        let missing = match min_rest {
            None => (0..).find(|n| !exact.contains(n)),
            Some(r) => (0..r).find(|n| !exact.contains(n)),
        };
        if let Some(n) = missing {
            return Err(terr(line, col, "match_arms", format!("no arm matches a slice of length {}", n),
                "add an arm such as `[..]` or `[a, rest @ ..]` for the remaining lengths"));
        }
        // An element read widens integer elements to i64 (as `s[i]` does), so prefix binders are i64 or bool.
        let elem = if elem.name == "bool" { elem } else { GType::i64() };
        let s = &m.scrutinee;
        let int = |v: usize| Expr::IntLit(IntLit { line, col, value: v.to_string(), ty: None, kind: LitKind::Int });
        let call = |name: &str, args: Vec<Expr>| Expr::Call(Call { line, col, name: name.into(), args, ty: None,
            targs: None, inst: None, old: None, opaque_c: false });
        let len = || call("len", vec![s.clone()]);
        let arm_block = |a: &MatchArm| {
            let mut out = Vec::new();
            for (i, b) in a.binders.iter().enumerate() {
                if b != "_" {
                    out.push(Stmt::Let(Let { borrowed: false, line: a.line, col: a.col, name: b.clone(), mutable: false,
                        ty: elem.to_key(), refine: None,
                        value: Expr::Index(Index { line: a.line, col: a.col, array: Box::new(s.clone()),
                            index: Box::new(int(i)), ty: None }) }));
                }
            }
            if let Some(Some(r)) = slice_arm_shape(&a.ctor).filter(|r| r.is_some_and(|n| !n.is_empty())) {
                out.push(Stmt::Let(Let { borrowed: true, line: a.line, col: a.col, name: r.to_string(), mutable: false,
                    ty: t.to_key(), refine: None, value: call("subview", vec![s.clone(), int(a.binders.len()), len()]) }));
            }
            out.extend(a.body.iter().cloned());
            out
        };
        let last = m.arms.last().expect("a match has an arm");
        let mut els = arm_block(last);
        for a in m.arms[..m.arms.len() - 1].iter().rev() {
            let k = a.binders.len();
            let op = if slice_arm_shape(&a.ctor).is_some_and(|r| r.is_some()) { ">=" } else { "==" };
            let cond = Expr::Binary(Binary { line: a.line, col: a.col, op: op.into(), left: Box::new(len()),
                right: Box::new(int(k)), ty: None });
            els = vec![Stmt::If(If { line: a.line, col: a.col, cond, then: arm_block(a), els })];
        }
        if m.arms.len() == 1 {
            return Ok(Stmt::If(If { line, col, cond: Expr::BoolLit(BoolLit { line, col, value: true, ty: None }),
                then: els, els: Vec::new() }));
        }
        Ok(els.pop().expect("the chain is one if"))
    }

    /// P7-1 B: a slice pattern on a `[T; N]` place. Every arm must match length `N` (Rust rejects the others), so
    /// the first arm is the only reachable one; it binds element reads `a[i]` of the element type. A named rest
    /// (`rest @ ..`) would view the array beyond a call-duration lend and stays `type/fixed_array_operation`.
    fn fixed_slice_match(&mut self, m: &Match, el: GType, n: u64) -> Res<Stmt> {
        let (line, col) = (m.line, m.col);
        for (k, a) in m.arms.iter().enumerate() {
            let rest = slice_arm_shape(&a.ctor).flatten();
            let width = a.binders.len() as u64;
            if rest.is_some_and(|r| !r.is_empty()) {
                return Err(terr(a.line, a.col, "fixed_array_operation",
                    format!("a rest binding would view the fixed-length array [{}; {}] as a slice", tn(&el), n),
                    "use `..` without a name, or index the array directly"));
            }
            let fits = if rest.is_some() { width <= n } else { width == n };
            if !fits {
                return Err(terr(a.line, a.col, "match_arms",
                    format!("this slice pattern never matches a fixed-length array of length {}", n),
                    format!("write {} element patterns, or fewer with `..`", n)));
            }
            if k > 0 {
                return Err(terr(a.line, a.col, "match_arms", "this slice pattern only matches lengths an earlier arm takes",
                    "remove the arm or move it before the arm that covers it"));
            }
        }
        let s = &m.scrutinee;
        let a = m.arms.first().expect("a match has an arm");
        let mut out = Vec::new();
        for (i, b) in a.binders.iter().enumerate() {
            if b != "_" {
                let index = Expr::IntLit(IntLit { line, col, value: i.to_string(), ty: None, kind: LitKind::Int });
                out.push(Stmt::Let(Let { borrowed: false, line: a.line, col: a.col, name: b.clone(), mutable: false,
                    ty: el.to_key(), refine: None,
                    value: Expr::Index(Index { line: a.line, col: a.col, array: Box::new(s.clone()),
                        index: Box::new(index), ty: None }) }));
            }
        }
        out.extend(a.body.iter().cloned());
        Ok(Stmt::If(If { line, col, cond: Expr::BoolLit(BoolLit { line, col, value: true, ty: None }),
            then: out, els: Vec::new() }))
    }

    fn match_stmt(&mut self, m: &mut Match, scope: &mut Scope) -> Res<()> {
        let t = self.match_scrutinee(&mut m.scrutinee, scope)?;
        self.match_arms(m, scope, t)
    }

    fn match_arms(&mut self, m: &mut Match, scope: &mut Scope, t: GType) -> Res<()> {
        let (line, col) = (m.line, m.col);
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
        let vec_read = self.rec_vec_get(field_place(&m.scrutinee).0);
        let root = if let Some(mut call) = vec_read {
            // a nested borrowed element read borrows the outermost vector
            while let Some(inner) = self.rec_vec_get(field_place(&call.args[0]).0) {
                call = inner;
            }
            place_root(&call.args[0]).map(str::to_string)
        } else if let Some(call) = map_call.filter(|_| map_borrow && nested_map_path.is_none()) {
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
        // G-DB16-1: an array binder may be moved when the scrutinee is a plain owned local enum value that
        // no arm names again; the move then consumes the scrutinee (foreign.rs marks it moved)
        // LG36 (G-LG35-1): not while an enclosing match arm still views the same owner (active_roots): the outer
        // binder reads the payload the move would hand over
        let mut movable_scrut = false;
        if user && t.args.is_empty() {
            if let Expr::Var(v) = &m.scrutinee {
                if !self.arm_binders.contains(&v.name)
                    && !self.active_roots.iter().any(|(r, _)| *r == v.name)
                    && scope.get(&v.name).is_some_and(|e| e.kind == "local")
                {
                    let mut named: HashSet<String> = HashSet::new();
                    for a in &m.arms {
                        crate::refine::stmt_names(&a.body, &mut named);
                    }
                    movable_scrut = !named.contains(&v.name);
                }
            }
        }
        for a in m.arms.iter_mut() {
            let mut sc = scope.clone();
            let (nl, nc) = if user { (a.line, a.col) } else { (line, col) };
            let saved_movable = self.match_movable.clone();
            let saved_binders = self.arm_binders.clone();
            self.arm_binders.extend(a.binders.iter().cloned());
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
                let payload = if a.ctor == "Err" { t.result_error() } else { t.view_payload().or_else(|| t.args.first()) };
                let payload = payload.cloned().unwrap_or_else(i64t);
                if a.ctor == "Ok" && payload == unitt() && a.binders.len() == 1 && a.binders[0].starts_with("pruv_else_") {
                    a.binders.clear(); // P8: the synthesized complement of `Err(e)` for Result<(), E>
                }
                if a.ctor == "Ok" && (payload == unitt()) != a.binders.is_empty() {
                    // LB8 R-6g7-1
                    let what = if payload == unitt() { "binds no value, write `Ok(())`" }
                        else { "carries a value, the pattern must bind it" };
                    return Err(terr(a.line, a.col, "match_arity", format!("Ok of {} {}", tn(&t), what),
                        "write `Ok(())` for Result<(), E> and `Ok(v)` otherwise"));
                }
                vec![payload; a.binders.len()]
            };
            if a.pat_lets == 0 && !a.pats.is_empty() {
                // P8: `Some((a, b))` binds the temporary a.binders[k]; its components become leading lets
                let mut lets = Vec::new();
                for (k, pat) in a.pats.iter().enumerate() {
                    if let (Some(TuplePat::Tuple(l, c, ps)), Some(bt)) = (pat, ftypes.get(k)) {
                        lower_arm_pat(&a.binders[k], ps, bt, *l, *c, &mut lets)?;
                    }
                }
                a.pat_lets = lets.len();
                lets.append(&mut a.body);
                a.body = lets;
            }
            for (b, bt) in a.binders.iter().zip(ftypes.iter()) {
                if sc.contains_key(b) {
                    return Err(terr(nl, nc, "shadow", format!("{} is already defined", b), "use a new name"));
                }
                // G9 A0: a buffer payload is a read view (like a parameter): returning it needs copy()
                let kind = if a.ctor != "Err" && t.view_payload().is_some() { "ref_local" }
                    else if map_mut { "map_inout" } else if map_borrow { "map_view" }
                    else if bt.is_array() || *bt == strt() { "param" } else { "local" };
                sc.insert(b.clone(), Entry { ty: bt.clone(), mutable: map_take, kind });
            }
            if movable_scrut {
                for (b, bt) in a.binders.iter().zip(ftypes.iter()) {
                    // LG9 (G-DB15-2): a str binder too, for `return x` (see Return)
                    // LB9 (R-6g8-4): a struct binder too, for `let t = move b.f;` (see field_move_out)
                    if bt.is_array() || *bt == strt() || self.is_struct(bt) {
                        self.match_movable.insert(b.clone());
                    }
                }
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
            self.match_movable = saved_movable;
            self.arm_binders = saved_binders;
        }
        Ok(())
    }

    // ---- G9 A0: ownership of extended values, active-match borrows ----
    fn place_escape(&self, name: &str, line: i64, col: i64, scope: &Scope) -> Res<()> {
        // Place bindings are mutable inout entries; inout parameters are not rebindable.
        if let Some(entry) = scope.get(name).filter(|e| e.kind == "inout" && e.mutable) {
            let ty = &entry.ty;
            if self.is_extended(ty) || ty.is_collection() || ty.is_array() || *ty == strt()
                || crate::llvmgen::is_owned_result(&ty.to_key())
            {
                return Err(terr(line, col, "borrow_escape",
                    format!("place binding {name} cannot be returned, stored or moved"),
                    "copy(...) the value, or return / store it from the place's root after the binding's last use"));
            }
        }
        Ok(())
    }

    /// `value` (already typed) is moved into an owning position (let, assignment, constructor field, return):
    /// a non-fresh extended value is cloned, which needs `alloc`; constructors, literals and calls are fresh.
    fn own(&mut self, value: &Expr, scope: &Scope) -> Res<()> {
        if let Expr::Var(v) = value {
            self.place_escape(&v.name, v.line, v.col, scope)?;
        }
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
            // R-6e-20: a field read through a borrowed vector element is never owned
            if matches!(value, Expr::FieldAccess(_)) && self.rec_vec_get(field_place(value).0).is_some()
                && (self.is_extended(&t) || t.is_collection() || crate::llvmgen::is_owned_result(k) || t.is_array() || t == strt()) {
                return Err(terr(value.line(), value.col(), "foreign_owned_type", format!("vec_get cannot copy {}", tn(&t)), ""));
            }
            if self.is_extended(&t) && !matches!(value, Expr::EnumCtor(_) | Expr::StructLit(_) | Expr::Call(_) | Expr::Move(_)) {
                let copies_only_refs = self.decls.get(&t.name).is_some_and(|decl| {
                    crate::borrowed::borrowing_struct(decl) && decl.all_fields().iter().all(|field| {
                        let ty = GType::from_key(&field.ty);
                        field.borrowed || !(ty.is_array() || ty == strt() || ty.is_collection()
                            || ty.has_string_error() || self.is_extended(&ty))
                    })
                });
                if !copies_only_refs {
                    self.used_effects.insert("alloc".to_string());
                }
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
            _ if ty.is_array() || ty.view_base().is_some() || ty.owner.is_none() && ty.name == "vec" =>
                return Err(terr(st.line, st.col, "for_enumerate",
                    format!("`for (i, x) in` over {} is not available yet (bundle P15)", tn(&ty)),
                    "write `for i in 0..len(v)` and read the element by index")),
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

    /// P4: resolves the types the parser left as `i64` in a lowered `for` (see parser `for_loop`). A range takes the
    /// width of its non-literal bound (the end bound first); a slice binder takes the element type, and a `Vec<T>`
    /// element read becomes `vec_get::<T>(v, K)`.
    fn for_types(&mut self, s: &mut If, scope: &Scope) -> Res<()> {
        let slice = for_loop_of(s).is_some_and(|w| w.for_head.as_ref().is_some_and(|h| h.slice));
        if !slice {
            let mut ty = None;
            for k in [1, 0] {
                let Stmt::Let(l) = &s.then[k] else { unreachable!() };
                if ty.is_none() && !is_int_literal(&l.value) {
                    ty = Some(self.expr(&mut l.value.clone(), scope, false)?);
                }
            }
            let ty = ty.unwrap_or_else(i64t);
            if ty.int_width().is_none() {
                let Stmt::Let(l) = &s.then[1] else { unreachable!() };
                return Err(terr(l.value.line(), l.value.col(), "for_iterable",
                    format!("a for range needs integer bounds, found {}", tn(&ty)), ""));
            }
            for k in 0..3 {
                let Stmt::Let(l) = &mut s.then[k] else { unreachable!() };
                l.ty = ty.to_key();
            }
            return Ok(());
        }
        let Some(Stmt::While(w)) = s.then.last_mut() else { unreachable!() };
        let (place, _) = for_element_read(&w.body);
        let (line, col) = (place.line(), place.col());
        let ct = self.expr(&mut place.clone(), scope, false)?;
        let base = ct.view_base().cloned().unwrap_or_else(|| ct.clone());
        let elem = if base.is_array() {
            elem_type(&base)
        } else if let Some((el, _)) = base.fixed_parts() {
            // P7-1 B: a fixed array iterates its N elements like a slice
            el.clone()
        } else if base.owner.is_none() && base.name == "vec" && base.args.len() == 1
            && (base.args[0].int_width().is_some() || base.args[0] == boolt()) {
            let elem = base.args[0].clone();
            let Some(Stmt::Let(l)) = w.body.first_mut() else { unreachable!() };
            let Expr::Index(i) = std::mem::replace(&mut l.value, Expr::BoolLit(BoolLit { line, col, value: true, ty: None }))
                else { unreachable!() };
            l.value = Expr::Call(Call { line: i.line, col: i.col, name: "vec_get".into(), args: vec![*i.array, *i.index],
                ty: None, targs: Some(vec![elem.clone()]), inst: None, old: None, opaque_c: false });
            elem
        } else if base.name == "vec" {
            return Err(terr(line, col, "for_iterable",
                format!("a for loop over {} is not available: only Vec elements of integer or bool type", tn(&ct)),
                "iterate the indexes with `for i in 0..len(v)` and read with vec_get (owned elements: bundle P15/P16)"));
        } else {
            return Err(terr(line, col, "for_iterable",
                format!("a for loop iterates a half-open range or a slice/Vec, found {}", tn(&ct)),
                "write `for i in a..b`; map iteration is `for (k, v) in m`"));
        };
        let Some(Stmt::Let(l)) = w.body.first_mut() else { unreachable!() };
        l.ty = elem.to_key();
        Ok(())
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

    fn view_context(e: &Call) -> PruvError {
        terr(e.line, e.col, "view_context",
            "subview(...) is only allowed as a direct argument to a non-inout array parameter of a function call",
            "pass it straight to a function, or use copy_range to make an owned array")
    }

    /// LG8: subview(a, lo, hi) as the only direct argument of the builtin copy; the array type is the source's
    fn copy_subview_arg(&mut self, x: &mut Call, scope: &Scope, c: bool) -> Res<GType> {
        if x.targs.is_some() {
            return Err(terr(x.line, x.col, "type_arity", "builtin subview takes no type arguments", "remove the `::<...>`"));
        }
        if x.args.len() != 3 {
            return Err(terr(x.line, x.col, "arity", format!("subview takes 3 argument(s), got {}", x.args.len()), ""));
        }
        let rooted = place_root(&x.args[0]).is_some();
        self.in_view = true;
        let r = (|| -> Res<GType> {
            let at = self.expect_array(&mut x.args[0], scope, c, false)?; // a view nested in the source is reported where it is
            if !rooted {
                self.in_view = false;
                return Err(Self::view_context(x));
            }
            if !at.is_array() {
                return Err(terr(x.args[0].line(), x.args[0].col(), "type_mismatch",
                    format!("expected {}, found {}", tn(&GType::array()), tn(&at)), ""));
            }
            self.expect(&mut x.args[1], &i64t(), scope, c, None)?;
            self.expect(&mut x.args[2], &i64t(), scope, c, None)?;
            Ok(at)
        })();
        self.in_view = false;
        let at = r?;
        x.ty = Some(at.to_key());
        Ok(at)
    }

    /// `fixed`: the call's result cannot hold a borrow of its arguments, so a `[T; N]` may be lent as `[T]` (P7-1 B).
    fn expect_call_arg(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, c: bool, borrow: bool, fixed: bool) -> Res<()> {
        if borrow && *ty == GType::array_u8() {
            if let Expr::StrLit(lit) = e {
                lit.ty = Some(ty.to_key());
                return Ok(());
            }
        }
        // P7-1 B: set right before user_arg_slices, which takes it on entry (a set flag must never outlive this call)
        self.fixed_borrow = borrow && fixed && !c;
        self.expect_user_arg(e, ty, scope, c, borrow)
    }

    /// a value in a view position: a `ref` let, the result of a `from` function or a `ref` field (check.py view_value)
    fn view_value(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, c: bool) -> Res<()> {
        if let Expr::StrLit(lit) = e {
            if lit.empty {
                // R-6g9-3: `[]` is the static empty view of an array, never a str
                if !ty.is_array() {
                    return Err(empty_view_err(lit.line, lit.col));
                }
                lit.ty = Some(ty.to_key());
                self.empty_views.insert((lit.line, lit.col));
                return Ok(());
            }
        }
        // A view position holds a borrow of a place, not an owned `Vec` (check.py `view_value`).
        if self.user_arg_slices(e, ty, scope, c, true)? {
            return Err(terr(e.line(), e.col(), "borrow_signature",
                "a borrowed view needs a place, subview, borrowed result or string literal", ""));
        }
        Ok(())
    }

    /// R-6g9-3: a `[]` that no view position accepted (a call argument, an operand) is an error
    /// (several string-literal branches accept a StrLit without typing it)
    fn reject_stray_empty_views(&mut self, f: &Function) -> Res<()> {
        let mut stray: Option<(i64, i64)> = None;
        let seen = std::mem::take(&mut self.empty_views);
        let mut visit = |n: crate::foreign_shim::Node<'_>| {
            if let crate::foreign_shim::Node::Expr(Expr::StrLit(lit)) = n {
                if lit.empty && !seen.contains(&(lit.line, lit.col)) && stray.is_none_or(|p| (lit.line, lit.col) < p) {
                    stray = Some((lit.line, lit.col)); // the first in source order (check.py)
                }
            }
        };
        for e in f.requires.iter().chain(f.ensures.iter()).chain(f.decreases.iter()) {
            crate::foreign_shim::walk_expr(e, &mut visit);
        }
        crate::foreign_shim::walk_stmts(&f.body, &mut visit);
        match stray {
            Some((line, col)) => Err(empty_view_err(line, col)),
            None => Ok(()),
        }
    }

    fn expect_user_arg(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, c: bool, borrow_vector: bool) -> Res<()> {
        self.user_arg_slices(e, ty, scope, c, borrow_vector).map(|_| ())
    }

    /// `expect_user_arg`, answering whether a `Vec` place was taken as an array slice argument.
    fn user_arg_slices(&mut self, e: &mut Expr, ty: &GType, scope: &Scope, c: bool, borrow_vector: bool) -> Res<bool> {
        let fixed_borrow = std::mem::take(&mut self.fixed_borrow);
        if let Expr::Call(x) = e {
            if x.name == "subview" {
                let string_view = *ty == strt();
                if c || !borrow_vector || !(ty.is_array() || string_view) || self.in_view {
                    return Err(Self::view_context(x));
                }
                if x.targs.is_some() {
                    return Err(terr(x.line, x.col, "type_arity", "builtin subview takes no type arguments", "remove the `::<...>`"));
                }
                if x.args.len() != 3 {
                    return Err(terr(x.line, x.col, "arity", format!("subview takes 3 argument(s), got {}", x.args.len()), ""));
                }
                let source_is_place = place_root(&x.args[0]).is_some();
                self.in_view = true;
                let r = (|| -> Res<()> {
                    // Check the source first so a nested view is reported where it is.
                    if string_view {
                        let got = self.expr(&mut x.args[0], scope, c)?;
                        if !source_is_place {
                            return Err(Self::view_context(x));
                        }
                        if got != GType::array_u8() {
                            self.check_expected_type(&x.args[0], ty, None, got)?;
                        }
                    } else {
                        self.expect(&mut x.args[0], ty, scope, c, None)?;
                    }
                    if !source_is_place {
                        return Err(Self::view_context(x));
                    }
                    self.expect(&mut x.args[1], &i64t(), scope, c, None)?;
                    self.expect(&mut x.args[2], &i64t(), scope, c, None).map(|_| ())
                })();
                self.in_view = false;
                r?;
                x.ty = Some(ty.to_key());
                return Ok(false);
            }
        }
        if *ty == strt() {
            if borrow_vector {
                if let Expr::StrLit(lit) = e {
                    lit.ty = Some(ty.to_key());
                    return Ok(false);
                }
            }
            return self.expect_str_arg(e, scope, c).map(|_| false);
        }
        if is_narrow(ty) && matches!(e, Expr::IntLit(_)) {
            return self.expect(e, ty, scope, c, None).map(|_| false);
        }
        // R-6w-2: the parameter type is the expected type of each value `if` arm (a narrow literal, a tuple literal)
        if (is_narrow(ty) || crate::gtype::is_tuple_name(&ty.name)) && matches!(e, Expr::Call(x) if ValueIf::matches(x) || crate::ast::ValueMatch::matches(x)) {
            return self.expect(e, ty, scope, c, None).map(|_| false);
        }
        let got = if borrow_vector && self.recursive_vec_element(ty) {
            self.read_expr(e, scope, c, &["vec_get"])?
        } else {
            self.expr(e, scope, c)?
        };
        if borrow_vector && place_root(e).is_some() && key_is_vec_slice_arg(&got.to_key(), &ty.to_key()) {
            return Ok(true);
        }
        if fixed_borrow && crate::gtype::key_is_fixed_borrow(&got.to_key(), &ty.to_key()) {
            // P7-1 B: a [T; N] argument is borrowed as a read-only [T] for this internal call only
            return Ok(false);
        }
        self.check_expected_type(e, ty, None, got)?;
        Ok(false)
    }

    fn match_path_held(&self, name: &str, path: &[String]) -> bool {
        self.match_paths.iter().any(|(root, held)| root == name && paths_overlap(path, held))
    }

    fn check_iteration_borrow(&self, line: i64, col: i64, name: &str, path: &[String]) -> Res<()> {
        if self.for_holds.iter().any(|(root, held)|
            root == name && (path.starts_with(held) || held.starts_with(path))) {
            let place = std::iter::once(name).chain(path.iter().map(String::as_str)).collect::<Vec<_>>().join(".");
            return Err(terr(line, col, "recursive_borrow",
                format!("cannot mutate or move {} while a for loop iterates it", place),
                "collect the changes and apply them after the loop, or use a while loop over indexes"));
        }
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
        if let Expr::Var(v) = value {
            self.place_escape(&v.name, v.line, v.col, scope)?;
        }
        if matches!(value, Expr::Call(_) | Expr::StrLit(_)) || matches!(value, Expr::Move(_)) && (ty.is_array() || *ty == strt()) {
            return Ok(()); // LG17 (G-DB28-2): `move t` of an owned str binding also hands over its bytes
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
        if matches!(e, Expr::Call(call) if crate::ast::ValueIf::matches(call)) {
            // P9a-3: the expected type types every arm (a narrow literal arm takes it, not the first arm's i64)
            let t = self.value_if(e, Some(ty), scope, contract)?;
            return self.check_expected_type(e, ty, op, t);
        }
        if matches!(e, Expr::Call(call) if crate::ast::ValueMatch::matches(call)) {
            // P9a-5: the expected type types every arm
            let t = self.value_match(e, Some(ty), scope, contract)?;
            return self.check_expected_type(e, ty, op, t);
        }
        if let (Expr::Call(call), Some((el, _))) = (&mut *e, ty.fixed_parts()) {
            if is_array_lit(&call.name) {
                // P7-1: the expected [T; N] types the literal's elements
                let t = self.array_lit(call, Some(el), scope, contract)?;
                set_ty(e, &t);
                return self.check_expected_type(e, ty, op, t);
            }
        }
        if let (Expr::IntLit(lit), Some(width)) = (&*e, ty.int_width()) {
            // P7: char/byte literals and constant uses have a fixed type and are compared below
            if lit.kind == crate::ast::LitKind::Int {
                if width.is_char() {
                    return Err(terr(lit.line, lit.col, "type_mismatch", "expected char, found an integer literal",
                        "write a char literal such as 'a', or char(x) with x: u32"));
                }
                let t = check_int_literal(lit, width)?;
                set_ty(e, &t);
                return Ok(t);
            }
        }
        resolve_tuple_lit(e, ty);
        let t = self.expr(e, scope, contract)?;
        self.check_expected_type(e, ty, op, t)
    }

    /// P7-1: `[a, b, c]` / `[x; N]`. With `want` every element is checked against it; without it the first
    /// element that is not a bare integer literal fixes T (all bare literals: i64).
    fn array_lit(&mut self, call: &mut Call, want: Option<&GType>, scope: &Scope, c: bool) -> Res<GType> {
        let n = match crate::ast::array_repeat_len(&call.name) {
            Some(n) => n,
            None => call.args.len() as u64,
        };
        let el = match want {
            Some(w) => w.clone(),
            None => match call.args.iter_mut().find(|a| !bare_lit(a)) {
                Some(a) => self.expr(a, scope, c)?,
                None => i64t(),
            },
        };
        if el != boolt() && el.int_width().is_none() {
            return Err(terr(call.line, call.col, "fixed_array_element",
                format!("a fixed-length array holds scalar integers, char or bool, found {}", tn(&el)),
                "use a buffer ([T]) or a Vec for other elements"));
        }
        for a in call.args.iter_mut() {
            self.expect(a, &el, scope, c, None)?;
        }
        crate::parser::fixed_size(call.line, call.col, &el, n)?;
        Ok(GType::fixed(el, n))
    }

    /// P3: an index is i64 or any unsigned scalar integer (`u8`..`usize`); a literal index stays i64.
    fn expect_index(&mut self, e: &mut Expr, scope: &Scope, contract: bool) -> Res<GType> {
        if bare_lit(e) {
            return self.expect(e, &i64t(), scope, contract, Some("index"));
        }
        let t = self.expr(e, scope, contract)?;
        if t.int_width().is_some_and(|w| !w.signed && !w.is_char()) {
            return Ok(t);
        }
        self.check_expected_type(e, &i64t(), Some("index"), t)
    }

    /// P3: a `decreases` measure is any scalar integer type (proved >= 0 and decreasing as an Int).
    fn expect_measure(&mut self, e: &mut Expr, scope: &Scope, contract: bool) -> Res<GType> {
        if bare_lit(e) {
            return self.expect(e, &i64t(), scope, contract, Some("decreases measure"));
        }
        let t = self.expr(e, scope, contract)?;
        // P7: `char` is an integer width in the table but has no arithmetic, so it is not a measure (R-6p-5)
        if t.int_width().is_some_and(|w| !w.is_char()) {
            return Ok(t);
        }
        self.check_expected_type(e, &i64t(), Some("decreases measure"), t)
    }

    fn check_expected_type(&self, e: &Expr, ty: &GType, op: Option<&str>, t: GType) -> Res<GType> {
        if t != *ty {
            if let Some(op) = op {
                if is_narrow(&t) {
                    match op {
                        "unary minus" | "~" | "!" | "&" | "|" | "^" | "<<" | ">>" => {
                            return Err(small_int_op(e.line(), e.col(), op));
                        }
                        "index" => {
                            return Err(small_int_unsupported(e.line(), e.col(),
                                format!("index must be i64 or an unsigned integer type, found {}", t.name), "write i64(x) or usize(x)"));
                        }
                        "range bound" => {
                            return Err(small_int_unsupported(e.line(), e.col(),
                                format!("{} must be i64; {} here requires P4", op, t.name), "write i64(x)"));
                        }
                        _ => {}
                    }
                }
                if t.owner.is_some() && !matches!(op, "~" | "&" | "|" | "^" | "<<" | ">>" | "index" | "decreases measure" | "range bound") {
                    return Err(terr(
                        e.line(),
                        e.col(),
                        "generic_operation",
                        format!("{} is not allowed on the abstract type {}", if op == "!" { "a boolean operation" } else { op }, t.name),
                        "a type parameter can be copied, passed, returned and stored; == works on it in contracts only",
                    ));
                }
            }
            if crate::gtype::key_is_fixed_borrow(&t.to_key(), &ty.to_key()) {
                // P7-1 B: the only [T; N] -> [T] conversion is a read argument of an internal call (user_arg_slices)
                return Err(terr(e.line(), e.col(), "fixed_array_operation",
                    format!("a fixed-length array {} is lent as {} only as a read argument of a Pruv function call", tn(&t), tn(ty)),
                    "pass it directly to a function with a plain [T] parameter (not take, inout, extern, or one whose result borrows it), or copy the elements into a buffer"));
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

    // The right operand supplies a literal on the left with its width.
    // Return its type as well when it has already been checked.
    fn scalar_left(&mut self, left: &mut Expr, right: &mut Expr, scope: &Scope, c: bool) -> Res<(GType, Option<GType>)> {
        if bare_lit(left) && !bare_lit(right) {
            let rt = self.expr(right, scope, c)?;
            let lt = if is_narrow(&rt) { self.expect(left, &rt, scope, c, None)? } else { self.expr(left, scope, c)? };
            return Ok((lt, Some(rt)));
        }
        Ok((self.expr(left, scope, c)?, None))
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
            Expr::IntLit(x) if x.kind != crate::ast::LitKind::Int => Ok(GType::from_key(x.ty.as_deref().unwrap_or("i64"))),
            Expr::IntLit(x) => check_int_literal(x, &intwidth::I64),
            Expr::BoolLit(_) => Ok(boolt()),
            Expr::StrLit(x) => {
                if x.empty {
                    return Err(empty_view_err(x.line, x.col));
                }
                if !c && !std::mem::take(&mut self.static_lit) {
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
                    if !bare_lit(&x.operand) {
                        let t = self.expr(&mut x.operand, scope, c)?;
                        if t.int_width().is_some_and(|w| w.is_char()) {
                            return Err(terr(x.line, x.col, "char_op", "operator `unary minus` is unsupported on char",
                                "convert first: u32(c)"));
                        }
                        self.check_expected_type(&x.operand, &i64t(), Some("unary minus"), t)?;
                        return Ok(i64t());
                    }
                    self.expect(&mut x.operand, &i64t(), scope, c, Some("unary minus"))?;
                    Ok(i64t())
                } else if x.op == "~" {
                    self.expect(&mut x.operand, &i64t(), scope, c, Some("~"))?;
                    Ok(i64t())
                } else {
                    self.expect(&mut x.operand, &boolt(), scope, c, Some("!"))?;
                    Ok(boolt())
                }
            }
            Expr::Binary(x) => {
                let op = x.op.clone();
                match op.as_str() {
                    "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
                        let ordering = matches!(op.as_str(), "<" | "<=" | ">" | ">=");
                        let operation = Some(if ordering { "ordering" } else { "arithmetic" });
                        let (lt, rt) = self.scalar_left(&mut x.left, &mut x.right, scope, c)?;
                        let lt = if lt.int_width().is_some() {
                            lt
                        } else {
                            self.check_expected_type(&x.left, &i64t(), operation, lt)?
                        };
                        if !ordering && lt.int_width().is_some_and(|w| w.is_char()) {
                            // P7 (R-6t-1): a char is not a number
                            return Err(terr(x.line, x.col, "char_op", format!("operator `{}` is not defined on char", op),
                                "convert to a number first: u32(c), and back with char(x)"));
                        }
                        if let Some(rt) = rt {
                            self.check_expected_type(&x.right, &lt, operation, rt)?;
                        } else {
                            self.expect(&mut x.right, &lt, scope, c, operation)?;
                        }
                        Ok(if ordering { boolt() } else { lt })
                    }
                    "&" | "|" | "^" | "<<" | ">>" => {
                        self.expect(&mut x.left, &i64t(), scope, c, Some(&op))?;
                        self.expect(&mut x.right, &i64t(), scope, c, Some(&op))?;
                        Ok(i64t())
                    }
                    "==" | "!=" => {
                        let (lt, rt) = self.scalar_left(&mut x.left, &mut x.right, scope, c)?;
                        if lt.is_collection_owner() {
                            // G6
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                "collections cannot be compared with ==",
                                "compare lengths and elements: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]",
                            ));
                        }
                        if lt.is_fixed() || rt.as_ref().is_some_and(|t| t.is_fixed()) {
                            // P7-1: z3 array equality would also compare the elements past N
                            return Err(terr(x.line, x.col, "fixed_array_operation",
                                "fixed-length arrays cannot be compared with == yet",
                                "compare the elements: forall k in 0..N: a[k] == b[k]"));
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
                        if lt.view_payload().is_some()
                            || (lt.owner.is_none() && dependencies(&lt).iter().any(|n| self.buffered.contains(n))) {
                            // G9 A0
                            return Err(terr(
                                x.line,
                                x.col,
                                "eq_type",
                                format!("values of {} contain arrays or str and cannot be compared with ==", tn(&lt)),
                                "compare tags, lengths and indexed contents instead",
                            ));
                        }
                        if c {
                            // P7-1 R1: an instantiation holding a [T; N] makes this == unsound (check_eq_targs)
                            collect_tparams(&lt, &mut self.eq_tparams);
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
                        if lt.int_width().is_none() && lt != boolt() && !c {
                            return Err(terr(x.line, x.col, "eq_type", "== only compares i64 or bool in code", "use match"));
                        }
                        if let Some(rt) = rt {
                            self.check_expected_type(&x.right, &lt, None, rt)?;
                        } else {
                            self.expect(&mut x.right, &lt, scope, c, None)?;
                        }
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
                self.place_escape(&x.name, x.line, x.col, scope)?;
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
                        // R-6g9-6: ref fields never own the value they store.
                        if self.ref_field(&ty.name, field) {
                            return Err(terr(x.line, x.col, "array_alias", "a borrowed view cannot be moved", ""));
                        }
                        ty = self.field_type(x.line, x.col, &ty, field)?;
                    }
                    if ty.is_array() && self.field_move != Some((x.line, x.col)) {
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
                if (entry.ty.is_array() || entry.ty == strt()) && !matches!(entry.kind, "local" | "take")
                    && self.array_replace.as_deref() != Some(x.name.as_str())
                    && !self.match_movable.contains(&x.name) // G-DB16-1: array binder of a consuming match
                {
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
                let at = self.expr(&mut x.array, scope, c)?;
                if let Some((el, _)) = at.fixed_parts() {
                    // P7-1: a[i] on [T; N] is a T; the proof asks 0 <= i < N
                    let el = el.clone();
                    self.expect_index(&mut x.index, scope, c)?;
                    return Ok(el);
                }
                let at = self.check_array_type(&x.array, true, at)?;
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
                let it = self.expect_index(&mut x.index, scope, c)?;
                if is_narrow(&it) && !at.is_array() {
                    return Err(small_int_unsupported(x.index.line(), x.index.col(),
                        format!("a {} index is allowed on an array only; here it must be i64", it.name), "write i64(x)"));
                }
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
                self.expect(&mut x.lo, &i64t(), scope, c, Some("range bound"))?;
                self.expect(&mut x.hi, &i64t(), scope, c, Some("range bound"))?;
                if scope.contains_key(&x.var) {
                    return Err(terr(x.line, x.col, "shadow", format!("{} is already defined", x.var), "use a new name"));
                }
                let mut sc = scope.clone();
                sc.insert(x.var.clone(), Entry { ty: i64t(), mutable: false, kind: "bound" });
                let boolean_body = matches!(x.kind.as_str(), "forall" | "exists" | "count");
                let body_ty = if boolean_body { boolt() } else { i64t() };
                let what = if boolean_body { "a quantifier body".to_string() } else { format!("an aggregate ({})", x.kind) };
                // R-6-3: trigger shape, then each trigger type checks like the body (binder in scope);
                // triggers are checked before the body, as in pruvc/check.py
                for (ti, tg) in x.triggers.iter_mut().enumerate() {
                    let why = if agg {
                        Some(format!("{} takes no trigger", x.kind))
                    } else if !matches!(tg, Expr::Index(_)) {
                        Some("a trigger must be an array read a[e]".to_string())
                    } else if !mentions(tg, &x.var) {
                        Some(format!("a trigger must mention {}", x.var))
                    } else {
                        None
                    };
                    if let Some(why) = why {
                        return Err(terr(
                            x.line,
                            x.col,
                            "bad_trigger",
                            format!("trigger {} of quantifier over {}: {}", ti + 1, x.var, why),
                            format!("write trigger a[...] with an array read that uses {}", x.var),
                        ));
                    }
                    self.expr(tg, &sc, c)?;
                    // a contract call inside the trigger is expanded in place; it must still use the binder
                    if !mentions(tg, &x.var) {
                        return Err(terr(
                            x.line,
                            x.col,
                            "bad_trigger",
                            format!("trigger {} of quantifier over {}: after expanding calls it does not mention {}", ti + 1, x.var, x.var),
                            format!("write trigger a[...] with an array read that uses {}", x.var),
                        ));
                    }
                }
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
                    fixed_contract_field(arg, &fd.1, c)?;
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
            Expr::Call(x) if crate::ast::ValueIf::matches(x) => self.value_if(e, None, scope, c),
            Expr::Call(x) if crate::ast::ValueMatch::matches(x) => self.value_match(e, None, scope, c),
            Expr::Call(x) if crate::ast::ValueMatch::is_yield(x) => Err(terr(x.line, x.col, "syntax",
                "an arm value outside a value `match`", "")),
            Expr::Call(x) => {
                let t = self.call(x, scope, c)?;
                if let Some(xe) = self.x_pending.take() {
                    *e = xe; // P06: a contract call to a user function becomes its expansion, in place
                }
                Ok(t)
            }
        }
    }

    /// P9a-3: a value `if`. The condition is bool; with `want` every arm is checked against it, otherwise the first
    /// arm that is not a bare literal fixes the type. A result type that owns storage takes only fresh arms (a call,
    /// a literal, a `move` or a constructor), so the value is new on either path, like a call result.
    fn value_if(&mut self, e: &mut Expr, want: Option<&GType>, scope: &Scope, c: bool) -> Res<GType> {
        let Expr::Call(call) = e else { unreachable!("value if") };
        let (line, col) = (call.line, call.col);
        if c {
            return Err(terr(line, col, "value_branch_context", "a value `if` is not allowed in a contract",
                "write the cases with `==>`, or `&&` and `||`"));
        }
        self.static_lit = false;
        let [cond, then, els] = call.args.as_mut_slice() else { unreachable!("value if arity") };
        self.expect(cond, &boolt(), scope, c, None)?;
        let t = match want {
            Some(w) => w.clone(),
            None if bare_lit(then) && !bare_lit(els) => self.expr(els, scope, c)?,
            None => self.expr(then, scope, c)?,
        };
        for arm in [&mut *then, &mut *els] {
            self.static_lit = false;
            self.want_alloc(arm, &t); // `alloc` takes the arm type, as in a `let`
            self.expect(arm, &t, scope, c, None)?;
        }
        let owned = self.value_branch_kind(&t).ok_or_else(|| terr(line, col, "value_branch_type",
            format!("a value `if` of type {} is not supported yet", tn(&t)),
            "use an `if` statement that assigns a `let mut` local"))?;
        if owned {
            for arm in [&*then, &*els] {
                if !self.fresh_arm(arm) {
                    return Err(terr(arm.line(), arm.col(), "value_branch_owned",
                        format!("an arm of a value `if` of type {} must be a new value", tn(&t)),
                        "write `move x` or `copy(x)`, or call a function"));
                }
            }
        }
        call.ty = Some(t.to_key());
        Ok(t)
    }

    /// P9a-5: a value `match`. The scrutinee is a copied enum, `Option` or `Result` (no owned payload); the arms are
    /// checked through the statement `match` (exhaustiveness, duplicates, binders scoped to the arm), each arm body
    /// being the marker `@arm_value(e)`. With `want` every arm is checked against it, otherwise the first arm that is
    /// not a bare literal fixes the type and the bare literal arms are checked against it afterwards.
    fn value_match(&mut self, e: &mut Expr, want: Option<&GType>, scope: &Scope, c: bool) -> Res<GType> {
        let Expr::Call(call) = e else { unreachable!("value match") };
        let (line, col) = (call.line, call.col);
        if c {
            return Err(terr(line, col, "value_branch_context", "a value `match` is not allowed in a contract",
                "write the cases with `is` tests and `==>`"));
        }
        let varms = crate::ast::ValueMatch::arms(call);
        if varms.is_empty() {
            // P9a-6: a value match needs an arm to produce its value
            return Err(terr(line, col, "match_arms", "a value `match` has no arms", "write one arm for every case"));
        }
        if Self::has_ext_pattern(varms.iter().map(|a| (a.ctor.as_str(), a.binders.as_slice()))) {
            // P9a-6: lowered to nested value matches and value ifs, which are then checked as usual
            let mut probe = call.args[0].clone();
            let t = self.match_scrutinee(&mut probe, scope)?;
            let arms: Vec<_> = varms.iter().zip(call.args[1..].iter())
                .map(|(a, v)| (v.line(), v.col(), a.enum_name.clone(), a.ctor.clone(), a.binders.clone())).collect();
            let uses: Vec<HashSet<String>> = call.args[1..].iter().map(|v| { let mut u = HashSet::new(); pat_names_expr(v, &mut u); u }).collect();
            // binders are renamed into the arm values below, so check them against the scope here, as a plain match does
            for (l, c, _, _, bs) in &arms {
                let mut names = HashSet::new();
                bs.iter().for_each(|b| pat_slot_names(b, &mut names));
                if let Some(b) = names.iter().find(|b| scope.contains_key(*b)) {
                    return Err(terr(*l, *c, "shadow", format!("{} is already defined", b), "use a new name"));
                }
            }
            let node = self.lower_patterns(&arms, &uses, true, &t, line, col)?;
            if let PNode::Leaf(..) = node {
                // a lone `_` arm over a scalar: there is no case to test, and the scrutinee must still be evaluated
                return Err(terr(line, col, "match_type", format!("a value `match` needs an Option, Result or enum, got {}", tn(&t)),
                    "use the value directly"));
            }
            *e = pat_value(&node, &call.args[0], &call.args[1..], line, col);
            return match e {
                Expr::Call(x) if crate::ast::ValueMatch::matches(x) => self.value_match(e, want, scope, c),
                Expr::Call(x) if ValueIf::matches(x) => self.value_if(e, want, scope, c),
                _ => match want {
                    Some(w) => {
                        let w = w.clone();
                        self.static_lit = false;
                        self.want_alloc(e, &w);
                        self.expect(e, &w, scope, c, None)?;
                        Ok(w)
                    }
                    None => self.expr(e, scope, c),
                },
            };
        }
        self.static_lit = false;
        let mut m = crate::ast::ValueMatch::as_match(call);
        if let Expr::Call(sc) = &m.scrutinee {
            if !self.funcs.contains_key(&sc.name) && matches!(sc.name.as_str(), "map_get" | "map_get_mut" | "map_take" | "vec_at_mut" | "vec_get") {
                return Err(terr(sc.line, sc.col, "value_branch_type",
                    format!("a value `match` on `{}` is not supported yet", sc.name),
                    "bind the result with `let` first, or use a `match` statement"));
            }
        }
        let st = self.match_scrutinee(&mut m.scrutinee, scope)?;
        // a unit payload (`Ok(())`) is copied like a scalar
        let copied = if matches!(st.name.as_str(), "option" | "result") {
            st.args.iter().all(|a| *a == unitt() || self.value_branch_kind(a) == Some(false))
        } else {
            self.value_branch_kind(&st) == Some(false)
        };
        if !copied || self.is_extended(&st) {
            return Err(terr(line, col, "value_branch_type",
                format!("a value `match` on {} is not supported yet (a payload is owned, borrowed or a type parameter)", tn(&st)),
                "use a `match` statement that assigns a `let mut` local"));
        }
        self.value_arms.push((want.cloned(), Vec::new(), 0));
        let mut sc = scope.clone();
        let r = self.match_arms(&mut m, &mut sc, st);
        let (t, mut deferred, _) = self.value_arms.pop().expect("value arm frame");
        r?;
        let Expr::Call(call) = e else { unreachable!("value match") };
        call.args[0] = m.scrutinee;
        for (k, a) in m.arms.into_iter().enumerate() {
            let Some(Stmt::ExprStmt(ExprStmt { expr: Expr::Call(mut y), .. })) = a.body.into_iter().last() else {
                unreachable!("value match arm body")
            };
            call.args[k + 1] = y.args.pop().expect("arm value");
        }
        let t = match t {
            Some(t) => t,
            None => {
                // every arm is a bare literal: the first fixes the type
                deferred.retain(|&k| k != 0);
                self.expr(&mut call.args[1], scope, c)?
            }
        };
        for k in deferred {
            self.static_lit = false;
            self.expect(&mut call.args[k + 1], &t, scope, c, None)?;
        }
        let owned = self.value_branch_kind(&t).ok_or_else(|| terr(line, col, "value_branch_type",
            format!("a value `match` of type {} is not supported yet", tn(&t)),
            "use a `match` statement that assigns a `let mut` local"))?;
        if owned {
            for arm in &call.args[1..] {
                if !self.fresh_arm(arm) {
                    return Err(terr(arm.line(), arm.col(), "value_branch_owned",
                        format!("an arm of a value `match` of type {} must be a new value", tn(&t)),
                        "write `move x` or `copy(x)`, or call a function"));
                }
            }
        }
        call.ty = Some(t.to_key());
        Ok(t)
    }

    /// P9a-5: one arm value of the innermost value `match`, in the arm's scope.
    fn value_match_arm(&mut self, e: &mut Expr, scope: &Scope) -> Res<()> {
        let frame = self.value_arms.last_mut().expect("value arm frame");
        let k = frame.2;
        frame.2 += 1;
        let want = frame.0.clone();
        self.static_lit = false;
        match want {
            Some(t) => {
                self.want_alloc(e, &t);
                self.expect(e, &t, scope, false, None)?;
            }
            None if bare_lit(e) => {
                self.value_arms.last_mut().expect("value arm frame").1.push(k);
            }
            None => {
                let t = self.expr(e, scope, false)?;
                self.value_arms.last_mut().expect("value arm frame").0 = Some(t);
            }
        }
        Ok(())
    }

    /// P9a-3/P9a-5: an arm of an owned value branch must be a new value: a call (not one whose result borrows an
    /// argument through `from` or a `ref` result), a literal, a constructor or a bare `move x`.
    fn fresh_arm(&self, arm: &Expr) -> bool {
        match arm {
            Expr::Call(x) => !PlaceIf::matches(x) && !self.funcs.get(&x.name).is_some_and(|f| f.holds_args),
            Expr::Move(m) => m.fields.is_empty(),
            Expr::StrLit(_) | Expr::StructLit(_) | Expr::EnumCtor(_) | Expr::Ctor(_) => true,
            _ => false,
        }
    }

    /// P9a-3: Some(true) when a value of `t` owns storage, Some(false) when it is copied, None when a value branch of
    /// this type is not supported (type parameters, views, borrowing structs, fixed arrays, handles). A tuple is owned
    /// when a component is.
    fn value_branch_kind(&self, t: &GType) -> Option<bool> {
        if t.owner.is_some() || crate::llvmgen::is_owned_result(&t.to_key()) || t.is_fixed() || t.view_base().is_some() || t.view_payload().is_some() {
            return None;
        }
        if t.int_width().is_some() || *t == boolt() {
            return Some(false);
        }
        if t.is_array() || *t == strt() || t.is_collection() || self.is_extended(t) {
            return Some(true);
        }
        if matches!(t.name.as_str(), "option" | "result") {
            return t.args.iter().all(|a| self.value_branch_kind(a) == Some(false)).then_some(false);
        }
        match self.decls.get(&t.name) {
            Some(d) if !crate::borrowed::borrowing_struct(d)
                && t.args.iter().all(|a| self.value_branch_kind(a) == Some(false)) => Some(false),
            _ => None,
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
        let fields = match self.decls.get(&sty.name) {
            Some(TypeDecl::Struct(s)) if sty.owner.is_none() => s.fields.as_slice(),
            _ => &[],
        };
        for fd in fields {
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
        if e.name == crate::gtype::TUPLE_PENDING {
            return Err(terr(e.line, e.col, "tuple_needs_type",
                "the type of this tuple literal is not known here",
                "bind it first with a typed let: `let t: (A, B) = (a, b);`"));
        }
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
        let borrowed_fields = match self.decls.get(&name) {
            Some(TypeDecl::Struct(s)) => s.fields.iter().map(|f| f.borrowed).collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        for ((fi, fd), borrowed) in e.fields.iter_mut().zip(&fields).zip(borrowed_fields) {
            if borrowed {
                self.view_value(&mut fi.value, &fd.1, scope, c)?;
            } else {
                fixed_contract_field(&fi.value, &fd.1, c)?;
                self.expect(&mut fi.value, &fd.1, scope, c, None)?;
                self.own_field(&fi.value, &fd.1, scope, c)?;
            }
        }
        self.ctor_allocs(&name, None, c); // G9 A0
        Ok(sty)
    }

    /// Some / None / Ok / Err. Without type arguments they keep the legacy i64 meaning (G3_SPEC 0.0 item 8);
    /// `Some::<X>(x)`, `None::<X>`, `Ok::<X>(x)`, `Err::<X>(code)` build Option<X> / Result<X> (3.5).
    fn ctor(&mut self, e: &mut Ctor, scope: &Scope, c: bool) -> Res<GType> {
        let name = e.name.clone();
        let mut targs = match e.targs.clone() {
            None => {
                if name == "None" {
                    return Ok(GType::option());
                }
                if e.arg.is_none() {
                    // LB8 R-6g7-1: unqualified Ok keeps Result<i64>
                    return Err(Self::unit_payload(e, &i64t()));
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
        let result = matches!(name.as_str(), "Ok" | "Err");
        if targs.len() != 1 && !(result && targs.len() == 2) {
            let (arity, fix) = if result {
                ("1 or 2 type arguments", format!("write {name}::<T> or {name}::<T, E>"))
            } else {
                ("exactly 1 type argument", format!("write {name}::<T>"))
            };
            return Err(terr(
                e.line,
                e.col,
                "type_arity",
                format!("{name}::<..> takes {arity}, got {}", targs.len()),
                fix,
            ));
        }
        let x = targs[0].clone();
        let view = x.view_base();
        if view.is_some() && targs.get(1).is_some_and(|err| *err != i64t()) {
            return Err(terr(e.line, e.col, "borrow_signature", "view payloads require an i64 error type", ""));
        }
        if view.is_none() {
            self.check_type(e.line, e.col, &x)?;
        }
        if view.is_none() && !(result && x == unitt()) {
            // LB8 R-6g7-1: Ok::<(), E>(()), Err::<(), E>(e)
            self.check_arg(e.line, e.col, &x, None)?;
            self.ext_misuse(e.line, e.col, &x, Some("arg"))?; // G9 A0
        }
        let err = targs.get(1).cloned().unwrap_or_else(i64t);
        if targs.len() == 2 {
            let ad = &self.arg_decls;
            let look = |n: &str| ad.get(n).map(|k| (*k, Vec::<GType>::new()));
            if let Err(ex) = check_arity(&err, &look) {
                return Err(terr(
                    e.line,
                    e.col,
                    "type_arity",
                    ex.to_string(),
                    "give the exact number of type arguments, e.g. Option<i64>, Pair<i64, bool>",
                ));
            }
            self.check_result_error(e.line, e.col, &err)?;
            if err == i64t() {
                targs.pop();
                e.targs = Some(targs.clone());
            }
        }
        let owner = format!("builtin:{}", name);
        let mut m = Mapping::new();
        m.insert((owner.clone(), 0), x.clone());
        if targs.len() == 2 {
            m.insert((owner.clone(), 1), err.clone());
        }
        e.inst = Some(inst_of(owner, &targs, &m));
        if name == "None" {
            return Ok(mk_option(x));
        }
        if name == "Ok" && (x == unitt()) != e.arg.is_none() {
            return Err(Self::unit_payload(e, &x));
        }
        if e.arg.is_none() {
            return Ok(mk_result_with_error(x, err));
        }
        let want = if name == "Err" { err.clone() } else { x.clone() };
        if let Some(a) = e.arg.as_mut() {
            if let Some(base) = view.filter(|_| name != "Err") {
                self.view_value(a, base, scope, c)?;
            } else {
                self.expect(a, &want, scope, c, None)?;
                if !c {
                    self.own(a, scope)?;
                }
            }
        }
        Ok(if name == "Some" { mk_option(x) } else { mk_result_with_error(x, err) })
    }

    /// LB8 R-6g7-1: the unit value `()` is written only as `Ok::<(), E>(())`, and that Ok takes nothing else
    fn unit_payload(e: &Ctor, x: &GType) -> PruvError {
        match e.arg.as_ref() {
            None => terr(e.line, e.col, "type_mismatch", format!("expected {}, found ()", tn(x)),
                "`Ok(())` builds a Result<(), E>: write Ok::<(), E>(())"),
            Some(a) => terr(a.line(), a.col(), "type_mismatch", "expected (), found a value",
                "the Ok payload of a Result<(), E> is written Ok::<(), E>(())"),
        }
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
                    self.expect_call_arg(&mut e.args[i], &want, scope, c, !f.foreign && !self.takes[&n][i], !f.holds_args)?;
                } else {
                    self.expect(&mut e.args[i], &want, scope, c, None)?;
                }
                continue;
            }
            if let Expr::Call(x) = &e.args[i] {
                if x.name == "subview" {
                    return Err(Self::view_context(x));
                }
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
            self.check_inout_view(arg)?;
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
        let store = matches!(n.as_str(), "store_be" | "store_le");
        let k = if store { 4 } else { 3 };
        if e.args.len() != k {
            return Err(terr(line, col, "arity", format!("{} takes {} argument(s), got {}", n, k, e.args.len()), ""));
        }
        if store {
            if c {
                return Err(terr(line, col, "effect_in_contract", format!("{} not allowed in contracts", n), ""));
            }
            let (aline, acol) = (e.args[0].line(), e.args[0].col());
            let name: String;
            let path: Vec<String>;
            {
                let arg: &mut InoutArg = match &mut e.args[0] {
                    Expr::InoutArg(a) => a,
                    _ => {
                        return Err(terr(
                            aline,
                            acol,
                            "inout_argument",
                            format!("the array of {} needs the `inout` marker", n),
                            format!("write `{}(inout name, i, w, n)` with a `let mut` local array or an inout parameter", n),
                        ))
                    }
                };
                let (root, fields) = field_place(&arg.expr);
                let op = match root {
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
                name = op.name.clone();
                path = fields;
                if !path.is_empty() || self.for_holds.iter().any(|(root, _)| *root == name) {
                    self.check_iteration_borrow(aline, acol, &name, &path)?;
                }
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
                if !(matches!(v.kind, "inout" | "local") || v.kind == "map_inout" && !path.is_empty()) {
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
                let ty = if path.is_empty() { v.ty } else { self.expr(&mut arg.expr, scope, c)? };
                self.check_inout_view(arg)?;
                if ty != GType::array_u8() {
                    return Err(terr(
                        aline,
                        acol,
                        "type_mismatch",
                        format!("expected {}, found {}", tn(&GType::array_u8()), tn(&ty)),
                        "",
                    ));
                }
                if self.active_roots.iter().any(|(root, _)| *root == name) || self.match_path_held(&name, &path) {
                    return Err(terr(
                        aline,
                        acol,
                        "recursive_borrow",
                        format!("cannot pass {} inout while a match on it is active", name),
                        "finish the match before the call",
                    ));
                }
                set_ty(&mut arg.expr, &ty);
                arg.ty = Some(ty.to_key());
            }
            self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
            self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
            let alias_args = if path.is_empty() { &e.args[1..] } else { &e.args[1..3] };
            if alias_args.iter().any(|x| {
                if path.is_empty() {
                    return mentions(x, &name);
                }
                let mut places = Vec::new();
                place_mentions(x, &mut places);
                places.iter().any(|(root, other)| *root == name && paths_overlap(&path, other))
            }) {
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
                    format!("{} must be a whole expression statement", n),
                    format!("write `{}(inout a, i, w, n);` as a statement", n),
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

    /// Array statements with an exclusive destination place.
    fn array_stmt_call(&mut self, e: &mut Call, scope: &Scope, c: bool) -> Res<GType> {
        let (line, col) = (e.line, e.col);
        let within = e.name == "copy_within";
        let builtin = e.name.as_str();
        let grow = array_grow_builtin(builtin);
        let append = builtin == "arr_append";
        // LB6 SX (R-6g5-5): arr_push(inout a, v1, .., vk) appends k values in one step
        let arity = if within || append { 4 } else if builtin == "arr_push" && (3..=ARR_PUSH_MAX + 1).contains(&e.args.len()) {
            e.args.len()
        } else if grow { 2 } else { 5 };
        if c {
            return Err(terr(line, col, "effect_in_contract", format!("{builtin} is not allowed in contracts"), ""));
        }
        if grow && self.statement_call != Some(&*e as *const Call as usize) {
            return Err(terr(line, col, "inout_context", format!("{builtin} must be a whole expression statement"), ""));
        }
        if e.args.len() != arity {
            let want = if builtin == "arr_push" { format!("2 to {}", ARR_PUSH_MAX + 1) } else { arity.to_string() };
            return Err(terr(line, col, "arity", format!("{builtin} takes {want} argument(s), got {}", e.args.len()), ""));
        }
        let (aline, acol) = (e.args[0].line(), e.args[0].col());
        let name: String;
        let path: Vec<String>;
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
                        if grow { "name a `let mut` local array or an inout parameter" }
                        else if within { "write `copy_within(inout name, dst, src, n)` with a `let mut` local array or an inout parameter" }
                        else { "write `copy_range(inout name, i, src, j, n)` with a `let mut` local array or an inout parameter" },
                    ))
                }
            };
            let (root, fields) = field_place(&arg.expr);
            let op = match root {
                // LB6 IF (R-6g5-2): arr_reserve / arr_push take a field place like arr_append
                Expr::Var(v) if !within || fields.is_empty() => v,
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
            name = op.name.clone();
            path = fields;
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
            if !(matches!(v.kind, "inout" | "local") || v.kind == "map_inout" && !path.is_empty()) {
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
            dty = if path.is_empty() { v.ty } else { self.expr(&mut arg.expr, scope, c)? };
            self.check_inout_view(arg)?;
            if !(is_concrete_int_buffer(&dty) || grow && !append && dty == GType::array_bool()) {
                return Err(terr(
                    aline,
                    acol,
                    "type_mismatch",
                    if grow && !append { format!("{builtin} takes [i64], [u8], [u16] or [bool], found {}", tn(&dty)) }
                    else { format!("expected [i64], [u8] or [u16], found {}", tn(&dty)) },
                    "",
                ));
            }
            if self.active_roots.iter().any(|(root, _)| *root == name) || self.match_path_held(&name, &path) {
                return Err(terr(
                    aline,
                    acol,
                    "recursive_borrow",
                    format!("cannot pass {} inout while a match on it is active", name),
                    "finish the match before the call",
                ));
            }
            if !path.is_empty() || self.for_holds.iter().any(|(root, _)| *root == name) {
                // after the type and match checks, in the order of check.py
                self.check_iteration_borrow(aline, acol, &name, &path)?;
            }
            set_ty(&mut arg.expr, &dty);
            arg.ty = Some(dty.to_key());
        }
        // LB6 SB (R-6g5-1): a str (a literal lent statically) is the byte source of a [u8] copy
        let byte_src = |this: &mut Self, x: &mut Expr| -> Res<GType> {
            if dty == GType::array_u8() {
                if let Expr::StrLit(lit) = x {
                    lit.ty = Some(GType::array_u8().to_key());
                    return Ok(GType::array_u8());
                }
            }
            let got = this.expr(x, scope, c)?;
            Ok(if dty == GType::array_u8() && got == strt() { GType::array_u8() } else { got })
        };
        if append {
            let src = byte_src(self, &mut e.args[1])?;
            if src != dty && !(dty == GType::array() && src == GType::array_u8()) {
                return Err(terr(e.args[1].line(), e.args[1].col(), "type_mismatch",
                    format!("arr_append cannot append {} to {}", tn(&src), tn(&dty)), ""));
            }
            self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
            self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
        } else {
            let value_ty = if builtin == "arr_push" { elem_type(&dty) } else { i64t() };
            for i in 1..if builtin == "arr_push" { e.args.len() } else { 2 } {
                self.expect(&mut e.args[i], &value_ty, scope, c, None)?;
            }
        }
        if !grow {
            if within {
                self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
            } else {
                let got = byte_src(self, &mut e.args[2])?;
                if got != dty {
                    return Err(terr(e.args[2].line(), e.args[2].col(), "type_mismatch",
                        format!("expected {}, found {}", tn(&dty), tn(&got)), ""));
                }
            }
            self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
            if !within {
                self.expect(&mut e.args[4], &i64t(), scope, c, None)?;
            }
        }
        let Expr::InoutArg(dst) = &e.args[0] else { unreachable!() };
        if e.args.iter().enumerate().skip(1).any(|(i, arg)| {
            if !grow && !within && i == 2 && same_place(&dst.expr, arg) {
                return false;
            }
            if path.is_empty() {
                return mentions(arg, &name);
            }
            let mut places = Vec::new();
            place_mentions(arg, &mut places);
            places.iter().any(|(root, other)| *root == name && paths_overlap(&path, other))
        }) {
            return Err(terr(
                aline,
                acol,
                "inout_alias",
                format!("{} is passed inout and also used elsewhere in the arguments of this call", name),
                if grow { "compute the value before the call".to_string() }
                else if within { "compute the index arguments before the call".to_string() }
                else { format!("copy the source first with `let mut t: [..] = copy({});` (needs alloc), then pass t", name) },
            ));
        }
        if !grow && self.boundary != Some(&*e as *const Call as usize) {
            return Err(terr(
                line,
                col,
                "inout_context",
                format!("{builtin} must be a whole expression statement"),
                if within { "write `copy_within(inout a, dst, src, n);` as a statement" }
                else { "write `copy_range(inout dst, i, src, j, n);` as a statement" },
            ));
        }
        if grow { self.used_effects.insert("alloc".to_string()); }
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
        fn reads_inout_or_old(e: &Expr, scope: &Scope) -> bool {
            match e {
                Expr::Var(v) if scope.get(&v.name).is_some_and(|p| p.kind == "inout") => true,
                Expr::Move(m) if scope.get(&m.name).is_some_and(|p| p.kind == "inout") => true,
                Expr::Call(c) if c.name == "old" => true,
                _ => crate::refine::kids(e).into_iter().any(|child| reads_inout_or_old(child, scope)),
            }
        }
        let bad_operand = || terr(
            line,
            col,
            "old_argument",
            "the operand of old must be the name of an inout parameter of this function",
            "write old(a) where `inout a: [..]` is a parameter of the enclosing function",
        );
        let mut lowered = e.args[0].clone();
        let len = matches!(&lowered, Expr::Call(c) if c.name == "len" && c.args.len() == 1 && c.targs.is_none());
        let mut base = match &mut lowered {
            Expr::Call(c) if len => &mut c.args[0],
            _ => &mut lowered,
        };
        let mut whole = !len;
        let mut bad_index = false;
        let mut root_field = false;
        loop {
            match base {
                Expr::FieldAccess(field) => {
                    root_field = true;
                    base = &mut field.base;
                }
                Expr::Index(index) => {
                    root_field = false;
                    bad_index |= reads_inout_or_old(&index.index, scope);
                    base = &mut index.array;
                }
                _ => break,
            }
            whole = false;
        }
        let Expr::Var(root) = base else { return Err(bad_operand()) };
        let entry = scope.get(&root.name).filter(|x| {
            // LB6 OS (R-6g5-4): a whole scalar inout also has an entry value
            x.kind == "inout" && (!root_field && (x.ty.is_array() || x.ty.is_fixed()) || self.plain_struct(&x.ty).is_some()
                || whole && (x.ty.int_width().is_some() || x.ty == boolt()))
        }).ok_or_else(bad_operand)?;
        if bad_index {
            return Err(terr(line, col, "old_argument",
                "the index of an old operand cannot read an inout parameter; write old(a)[old(b)[0]] explicitly", ""));
        }
        let ty = entry.ty.clone();
        root.ty = Some(ty.to_key());
        let index = self.cur_params.iter().position(|p| *p == root.name).unwrap_or(0);
        e.old = Some((root.name.clone(), index));
        if whole {
            e.args[0] = Expr::Var(root.clone());
            return Ok(ty);
        }
        let mut call = e.clone();
        call.args[0] = Expr::Var(root.clone());
        call.ty = Some(ty.to_key());
        *base = Expr::Call(call);
        let checked = self.expr(&mut lowered, scope, true);
        // Ordinary len also accepts strings and collections, which are excluded here.
        if let Expr::Call(c) = &lowered {
            if len && expr_ty(&c.args[0]).as_deref().is_some_and(|ty| { let t = GType::from_key(ty); !t.is_array() && !t.is_fixed() }) {
                return Err(bad_operand());
            }
        }
        let ty = checked?;
        self.x_pending = Some(lowered);
        Ok(ty)
    }

    // ---- P06: expanding user functions in contracts (docs/L3_RESTRUCTURE_SPEC.md appendix P06) ----
    /// The reason each function cannot be expanded in a contract, or a raw copy of its body when its signature and
    /// statements allow it (checked before any body is typed, so every template starts from source).
    fn x_prepare(&mut self, prog: &Program) -> Res<()> {
        for f in &prog.functions {
            let r = x_sig_reason(f, &self.decls).or_else(|| x_scan_block(f, &f.body, self));
            if f.pred {
                // G-LG13-5: a `?` in a pred return is try_position, as in Python (checked before pred_signature)
                for st in &f.body {
                    if let Stmt::Return(ret) = st {
                        if let Some(v) = &ret.value {
                            Self::check_try_position(v, ret.line, ret.col)?;
                        }
                    }
                }
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
        if is_array_lit(&e.name) {
            return self.array_lit(e, None, scope, c);
        }
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
        if intwidth::PENDING_INTS.contains(&n.as_str()) && !self.funcs.contains_key(&n) {
            return Err(small_int_unsupported(line, col, format!("{} casts are not available yet", n), "use a scalar integer type"));
        }
        if let Some(target) = intwidth::of(&n) {
            arity(1)?;
            if targs.is_some() {
                return Err(terr(line, col, "type_arity", "integer casts take no type arguments", ""));
            }
            let arg = &mut e.args[0];
            let source = self.expr(arg, scope, c)?;
            let width = source.int_width().ok_or_else(|| terr(arg.line(), arg.col(), if target.is_char() { "char_cast" } else { "type_mismatch" },
                format!("expected a scalar integer, found {}", tn(&source)), ""))?;
            // P7 (R-6t-2): `char` converts to the widths that hold every char and from `u8`/`u32` only
            if (target.is_char() && !width.is_char() && !matches!(width.name, "u8" | "u32"))
                || (width.is_char() && !target.is_char() && !width.fits_in(target))
            {
                return Err(terr(line, col, "char_cast", format!("no conversion from {} to {}", width.name, target.name),
                    "convert a char through u32: u32(c) and char(x) with x: u32 (or u8)"));
            }
            // P3: 64-bit pairs (i64/isize/u64/usize) have no common wider type, so a contract may convert between them;
            // contracts compute on unbounded integers, so the cast keeps the value (SPEC R-6p-5).
            if c && !width.fits_in(target) && !(width.bits == 64 && target.bits == 64) {
                return Err(terr(line, col, "cast_range", format!("narrowing from {} to {} is unsupported in contracts", width.name, target.name),
                    "widen the other value or do the conversion before the contract expression"));
            }
            return Ok(GType::new(target.name, Vec::new()));
        }
        if n == "subview" {
            return Err(Self::view_context(e));
        }
        for arg in &e.args {
            let arg = match arg {
                Expr::InoutArg(a) => a.expr.as_ref(),
                _ if self.funcs.contains_key(&n) => continue,
                _ => arg,
            };
            if let Expr::Call(x) = arg {
                if x.name == "subview" {
                    // LG8: copy(subview(a, lo, hi))
                    if n == "copy" && alen == 1 && !c && !self.in_view && !self.funcs.contains_key(&n) {
                        continue;
                    }
                    // LB7 (R-6g6-3): string_push_str(move s, subview(t, lo, hi)), string_from(subview(t, lo, hi))
                    if is_text_view_op(&n) && std::ptr::eq(arg, e.args.last().unwrap()) && !c && !self.in_view && !self.funcs.contains_key(&n) {
                        continue;
                    }
                    return Err(Self::view_context(x));
                }
            }
        }
        let is_builtin = matches!(n.as_str(), "len" | "alloc" | "copy" | "print" | "is_char_boundary" | "substr" | "utf8_valid" | "utf8_valid_span" | "str_from_utf8" | "truncate" | "eq_range" | "find_byte" | "resize" | "copy_range" | "copy_within" | "arr_reserve" | "arr_push" | "arr_append" | "subview")
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
                if ty.is_fixed() {
                    return Ok(i64t()); // P7-1: len(a) is N
                }
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
                let at = if let Expr::Call(x) = &mut e.args[0] {
                    if x.name == "subview" {
                        self.copy_subview_arg(x, scope, c)? // LG8
                    } else {
                        self.expect_array(&mut e.args[0], scope, c, false)?
                    }
                } else {
                    self.expect_array(&mut e.args[0], scope, c, false)?
                };
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
                        format!("resize takes [i64], [u8], [u16] or [bool], found {}", tn(&at)),
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
            "copy_range" | "copy_within" | "arr_reserve" | "arr_push" | "arr_append" => return self.array_stmt_call(e, scope, c),
            // L3-10: eq_range(a: [T], i, b: [T], j, n) -> bool, code only, T an integer element type
            "eq_range" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "eq_range is not allowed in contracts", ""));
                }
                arity(5)?;
                let at = self.expect_array(&mut e.args[0], scope, c, false)?;
                // Both operands must have the same concrete integer buffer type.
                if !is_concrete_int_buffer(&at) {
                    let (l0, c0) = (e.args[0].line(), e.args[0].col());
                    return Err(terr(l0, c0, "type_mismatch", format!("expected [i64], [u8] or [u16], found {}", tn(&at)), ""));
                }
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &at, scope, c, None)?;
                self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[4], &i64t(), scope, c, None)?;
                return Ok(boolt());
            }
            // LB6 MC (R-6g5-3): find_byte(a: [u8] or str, b, lo, hi) -> i64, code only
            "find_byte" => {
                if c {
                    return Err(terr(line, col, "effect_in_contract", "find_byte is not allowed in contracts", ""));
                }
                arity(4)?;
                // a literal is borrowed as its static bytes, no alloc (check.py)
                let at = if let Expr::StrLit(lit) = &mut e.args[0] {
                    lit.ty = Some(GType::array_u8().to_key());
                    GType::array_u8()
                } else {
                    self.expr(&mut e.args[0], scope, c)?
                };
                if at != GType::array_u8() && at != strt() {
                    let (l0, c0) = (e.args[0].line(), e.args[0].col());
                    return Err(terr(l0, c0, "type_mismatch", format!("expected [u8] or str, found {}", tn(&at)), ""));
                }
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[3], &i64t(), scope, c, None)?;
                return Ok(i64t());
            }
            // LG26 (R-6b-3, G-LG25-1): this predicate is evaluated only by the verifier.
            "utf8_valid_span" => {
                if !c || self.ckind == "cost" {
                    return Err(terr(line, col, "contract_only", "utf8_valid_span is only allowed in contracts", ""));
                }
                arity(3)?;
                self.expect(&mut e.args[0], &GType::array_u8(), scope, c, None)?;
                self.expect(&mut e.args[1], &i64t(), scope, c, None)?;
                self.expect(&mut e.args[2], &i64t(), scope, c, None)?;
                return Ok(boolt());
            }
            "utf8_valid" | "str_from_utf8" => {
                if c && n == "str_from_utf8" {
                    return Err(terr(line, col, "effect_in_contract", "str_from_utf8 not allowed in contracts", ""));
                }
                arity(1)?;
                self.expect(&mut e.args[0], &GType::array_u8(), scope, c, None)?;
                if n == "utf8_valid" {
                    return Ok(boolt());
                }
                self.used_effects.insert("alloc".to_string());
                return Ok(strt());
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
                if t.int_width().is_none() && t != boolt() {
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
                CRes::ErrorPayload => t.result_error().cloned().unwrap_or_else(i64t),
                // G3: the payload type of Option<X> / Result<X>
                CRes::Payload => {
                    if t.args.first() == Some(&unitt()) {
                        // LB8 R-6g7-1: the unit payload carries nothing to read
                        return Err(terr(e.line, e.col, "type_mismatch", format!("ok_val of {} has no value", tn(&t)), "use is_ok"));
                    }
                    if t.args.is_empty() {
                        i64t()
                    } else {
                        t.view_payload().unwrap_or(&t.args[0]).clone()
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
                inout: cf.params.iter().enumerate().map(|(i, p)| p.inout || i == 0 && matches!(n.as_str(), "map_get_mut" | "vec_at_mut" | "map_remove" | "map_take" | "vec_clear" | "vec_reserve" | "vec_extend_from_slice")).collect(),
                ret: cf.ret.clone(),
                effects: cf.effects.clone(),
                tparams: cf.tparams.clone(),
                foreign: false,
                opaque: false,
                pred: false,
                holds_args: true,
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
        if cat.is_none() {
            if let Some(ta) = &targs {
                self.targ_log.push((line, col, ta.clone(), n.clone()));
            }
        }
        let m = m.unwrap_or_default();
        arity(f.params.len())?;
        for (arg, p) in e.args.iter_mut().zip(f.params.iter()) {
            resolve_tuple_lit(arg, &substitute(p, &m)); // P6
        }
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
                    } else if cat.is_some() && is_text_view_op(&n) && want == strt() && matches!(arg, Expr::Call(x) if x.name == "subview") {
                        self.expect_user_arg(arg, &want, scope, c, true)?; // LB7 (R-6g6-3): a borrowed str range, no copy
                    } else if cat.is_none() {
                        self.expect_call_arg(arg, &want, scope, c, !f.foreign && !self.takes[&n][i], !f.holds_args)?;
                    } else {
                        self.expect(arg, &want, scope, c, None)?;
                    }
                }
            }
        }
        if cat.is_some() && n == "vec_at_mut" {
            let elem = &targs.as_ref().unwrap()[0];
            if !self.owned_aggregate(elem) && !self.recursive_vec_element(elem) {
                return Err(terr(line, col, "foreign_owned_type", "vec_at_mut requires an owned aggregate element", ""));
            }
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
            if (self.owned_aggregate(payload) || n == "vec_push" && self.recursive_vec_element(payload))
                && !matches!(arg, Expr::StructLit(_) | Expr::EnumCtor(_) | Expr::Call(_) | Expr::Move(_))
            {
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

fn plain_struct<'a>(ty: &GType, lookup: &impl Fn(&str) -> Option<&'a StructDecl>) -> Option<&'a StructDecl> {
    if ty.owner.is_some() || !ty.args.is_empty() {
        return None;
    }
    lookup(&ty.name).filter(|s| s.tparams.is_empty())
}

#[derive(Clone)]
pub enum InvariantStep {
    Field(String, GType),
    Some(GType),
    Ok(GType),
    Err(GType),
    VariantField(GType, String, String, GType),
}

impl InvariantStep {
    fn ty(&self) -> &GType {
        match self {
            Self::Field(_, ty) | Self::Some(ty) | Self::Ok(ty) | Self::Err(ty)
            | Self::VariantField(_, _, _, ty) => ty,
        }
    }
}

fn invariant_positions<'a>(
    ty: &GType,
    lookup: &impl Fn(&str) -> Option<&'a TypeDecl>,
) -> (Option<&'a StructDecl>, Vec<InvariantStep>) {
    if ty.owner.is_some() {
        return (None, Vec::new());
    }
    match (ty.name.as_str(), ty.args.as_slice()) {
        ("option", [payload]) => return (None, vec![InvariantStep::Some(payload.clone())]),
        ("result", [ok, err]) => return (None, vec![InvariantStep::Ok(ok.clone()), InvariantStep::Err(err.clone())]),
        _ => {}
    }
    match lookup(&ty.name) {
        Some(TypeDecl::Struct(s)) => {
            let Ok(mapping) = bind_args(&decl_owner(&s.name), s.tparams.len(), &ty.args) else {
                return (None, Vec::new());
            };
            let fields = s.fields.iter().map(|f| {
                InvariantStep::Field(f.name.clone(), substitute(&GType::from_key(&f.ty), &mapping))
            }).collect();
            (Some(s).filter(|s| !s.invariants.is_empty() && s.tparams.is_empty()), fields)
        }
        Some(TypeDecl::Enum(e)) => {
            let Ok(mapping) = bind_args(&decl_owner(&e.name), e.tparams.len(), &ty.args) else {
                return (None, Vec::new());
            };
            let mapping = &mapping;
            let fields = e.variants.iter().flat_map(|v| v.fields.iter().map(|f| {
                InvariantStep::VariantField(
                    ty.clone(), v.name.clone(), f.name.clone(), substitute(&GType::from_key(&f.ty), mapping),
                )
            })).collect();
            (None, fields)
        }
        _ => (None, Vec::new()),
    }
}

fn reaches<'a>(
    ty: &GType,
    lookup: &impl Fn(&str) -> Option<&'a TypeDecl>,
    memo: &mut HashMap<GType, Option<&'a StructDecl>>,
) -> Option<&'a StructDecl> {
    if let Some(&found) = memo.get(ty) {
        return found;
    }
    let mut seen = HashSet::new();
    let mut todo = vec![ty.clone()];
    let mut found = None;
    while let Some(cur) = todo.pop() {
        if !seen.insert(cur.clone()) {
            continue;
        }
        if let Some(&cached) = memo.get(&cur) {
            if cached.is_some() {
                found = cached;
                break;
            }
            continue;
        }
        let (owner, steps) = invariant_positions(&cur, lookup);
        if owner.is_some() {
            found = owner;
            break;
        }
        todo.extend(steps.iter().rev().map(|step| step.ty().clone()));
    }
    memo.insert(ty.clone(), found);
    found
}

/// Structs owning effective clauses, in preorder, with their paths from the root.
pub fn effective_invariant_structs<'a>(
    ty: &GType,
    lookup: &impl Fn(&str) -> Option<&'a TypeDecl>,
) -> Vec<(&'a StructDecl, Vec<InvariantStep>)> {
    fn visit<'a>(
        ty: &GType,
        lookup: &impl Fn(&str) -> Option<&'a TypeDecl>,
        seen: &mut HashSet<GType>,
        memo: &mut HashMap<GType, Option<&'a StructDecl>>,
        path: &mut Vec<InvariantStep>,
        out: &mut Vec<(&'a StructDecl, Vec<InvariantStep>)>,
    ) {
        if seen.contains(ty) || reaches(ty, lookup, memo).is_none() {
            return;
        }
        seen.insert(ty.clone());
        let (owner, steps) = invariant_positions(ty, lookup);
        if let Some(s) = owner {
            out.push((s, path.clone()));
        }
        for step in steps {
            let next = step.ty().clone();
            path.push(step);
            visit(&next, lookup, seen, memo, path, out);
            path.pop();
        }
        seen.remove(ty);
    }
    let mut out = Vec::new();
    visit(ty, lookup, &mut HashSet::new(), &mut HashMap::new(), &mut Vec::new(), &mut out);
    out
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
    let ok = [i64t(), boolt(), GType::array(), GType::array_u8(), GType::array_u16(), GType::array_bool()];
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

fn x_scan_block(f: &Function, stmts: &[Stmt], checker: &Checker) -> Option<String> {
    for (i, st) in stmts.iter().enumerate() {
        match st {
            Stmt::Let(l) => {
                let t = GType::from_key(&l.ty);
                if l.is_place() || l.mutable || l.refine.is_some() || (t != i64t() && t != boolt()) {
                    return Some(X_R_STMT.into());
                }
                if let Some(r) = x_scan_expr(&l.value, checker) {
                    return Some(r);
                }
            }
            Stmt::Return(r) => {
                return match &r.value {
                    None => Some(X_R_STMT.into()),
                    Some(v) => x_scan_expr(v, checker),
                };
            }
            Stmt::If(x) => {
                if GType::from_key(&f.ret) != boolt() {
                    return Some("it returns i64 through an if (only bool functions may branch)".into());
                }
                if let Some(r) = x_scan_expr(&x.cond, checker).or_else(|| x_scan_block(f, &x.then, checker)) {
                    return Some(r);
                }
                return if !x.els.is_empty() { x_scan_block(f, &x.els, checker) } else { x_scan_block(f, &stmts[i + 1..], checker) };
            }
            _ => return Some(X_R_STMT.into()),
        }
    }
    Some("an if branch does not end in return".into())
}

fn x_scan_expr(e: &Expr, checker: &Checker) -> Option<String> {
    match e {
        Expr::IntLit(_) | Expr::BoolLit(_) | Expr::Var(_) => None,
        Expr::Unary(u) if u.op != "?" => x_scan_expr(&u.operand, checker),
        Expr::Binary(b) => x_scan_expr(&b.left, checker).or_else(|| x_scan_expr(&b.right, checker)),
        Expr::Index(x) => x_scan_expr(&x.array, checker).or_else(|| x_scan_expr(&x.index, checker)),
        Expr::FieldAccess(_) | Expr::Quant(_) => {
            crate::refine::kids(e).into_iter().find_map(|child| x_scan_expr(child, checker))
        }
        // R-6w-2: a value `if` is not a contract expression, so a body using one is not expanded
        Expr::Call(c) if ValueIf::matches(c) => Some("it uses a value `if`".into()),
        Expr::Call(c) if crate::ast::ValueMatch::matches(c) => Some("it uses a value `match`".into()),
        Expr::Call(c) => {
            if checker.funcs.contains_key(&c.name) || c.name == "len" || is_bit_builtin(&c.name) || matches!(c.name.as_str(), "load_be" | "load_le" | "utf8_valid" | "utf8_valid_span") {
                c.args.iter().enumerate().find_map(|(i, a)| {
                    let literal_bytes = matches!(a, Expr::StrLit(_)) && checker.funcs.get(&c.name).is_some_and(|f| {
                        !f.foreign && f.params.get(i) == Some(&GType::array_u8()) && !f.inout[i] && !checker.takes[&c.name][i]
                    });
                    if literal_bytes { None } else { x_scan_expr(a, checker) }
                })
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
        Expr::Quant(x) => 1 + x_size(&x.lo) + x_size(&x.hi) + x_size(&x.body) + x.triggers.iter().map(x_size).sum::<usize>(),
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
            }),
            // R-6-3: the triggers are substituted like the body
            triggers: x.triggers.iter().map(|t| {
                if env.contains_key(&x.var) {
                    let mut inner = env.clone();
                    inner.remove(&x.var);
                    x_subst(t, &inner)
                } else {
                    x_subst(t, env)
                }
            }).collect(),
            ..x.clone()
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
        Expr::IntLit(x) => Expr::IntLit(IntLit { line, col, value: x.value.clone(), ty: x.ty.clone(), kind: x.kind.clone() }),
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
            }),
            // R-6-3: the triggers are substituted like the body (the binder is never replaced)
            triggers: x.triggers.iter().map(|t| {
                if pnames.contains(&x.var) {
                    let inner: Vec<String> = pnames.iter().map(|p| if *p == x.var { String::new() } else { p.clone() }).collect();
                    x_instantiate(t, &inner, args, line, col)
                } else {
                    x_instantiate(t, pnames, args, line, col)
                }
            }).collect(),
            ..x.clone()
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
                x.triggers.iter_mut().for_each(|t| ex(t, d));
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
                Stmt::Break(_) | Stmt::Continue(_) => {}
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

/// P6: a tuple literal takes its tuple type from the expected type (same arity); nested literals too.
fn resolve_tuple_lit(e: &mut Expr, ty: &GType) {
    let Expr::StructLit(lit) = e else { return };
    if lit.name != crate::gtype::TUPLE_PENDING || !ty.args.is_empty() {
        return;
    }
    let Some(comps) = crate::gtype::tuple_components(&ty.name) else { return };
    if comps.len() != lit.fields.len() {
        return;
    }
    lit.name = ty.name.clone();
    for (f, k) in lit.fields.iter_mut().zip(comps.iter()) {
        resolve_tuple_lit(&mut f.value, &GType::from_key(k));
    }
}

/// P9a-6: one arm while a pattern match is lowered: the tests still open on named values (`""` is the scrutinee)
/// and the names it binds (`(name, value, type)`).
#[derive(Clone)]
struct PRow {
    tests: Vec<(String, GType, crate::ast::SubPat)>,
    binds: Vec<(String, String, GType)>,
    arm: usize,
}

impl PRow {
    fn add(&mut self, var: String, ty: GType, p: crate::ast::SubPat) {
        use crate::ast::SubPat;
        match p {
            SubPat::Wild => {}
            SubPat::Bind(n) => self.binds.push((n, var, ty)),
            p => self.tests.push((var, ty, p)),
        }
    }
}

/// P9a-6: the decision tree of a lowered pattern match.
enum PNode {
    /// arm index and its bindings `(name, value, type)`
    Leaf(usize, Vec<(String, String, GType)>),
    /// a plain match on a named value (`""`: the scrutinee) with every constructor once: (enum, ctor, binders, tree)
    Match(String, Vec<(Option<String>, String, Vec<String>, PNode)>),
    /// `if v == lit { a } else { b }` (`if v { a } else { b }` for `bool`; `#false` swaps the branches)
    Test(String, crate::ast::SubPat, Box<PNode>, Box<PNode>),
}

fn pat_var(line: i64, col: i64, name: &str) -> Expr {
    Expr::Var(Var { line, col, name: name.to_string(), ty: None })
}

/// P9a-6: every name an arm body reads, moves, assigns or declares (a field keeps its source name in a decision-tree
/// case only when no arm that reaches the case without binding the field mentions that name). Over-approximating is
/// safe: a mentioned name only makes the field fresh.
fn pat_names_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    crate::foreign_shim::walk_stmts(stmts, &mut |n| pat_names_node(n, out));
    pat_targets(stmts, out);
}

fn pat_names_expr(e: &Expr, out: &mut HashSet<String>) {
    crate::foreign_shim::walk_expr(e, &mut |n| pat_names_node(n, out));
}

fn pat_names_node(n: crate::foreign_shim::Node<'_>, out: &mut HashSet<String>) {
    use crate::foreign_shim::Node;
    match n {
        Node::Expr(Expr::Var(x)) => { out.insert(x.name.clone()); }
        Node::Expr(Expr::Move(x)) => { out.insert(x.name.clone()); }
        Node::Expr(Expr::Quant(x)) => { out.insert(x.var.clone()); }
        // a value match spells its arm patterns (binder names) in the call name
        Node::Expr(Expr::Call(c)) if ValueMatch::matches(c) => {
            out.extend(c.name.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).filter(|s| !s.is_empty()).map(String::from));
        }
        Node::Let(l) => { out.insert(l.name.clone()); }
        Node::ForMap(f) => { out.insert(f.key.clone()); out.insert(f.val.clone()); }
        _ => {}
    }
}

/// The names a match sub-pattern slot binds (nested variants included).
fn pat_slot_names(slot: &str, out: &mut HashSet<String>) {
    fn go(p: &crate::ast::SubPat, out: &mut HashSet<String>) {
        match p {
            crate::ast::SubPat::Bind(n) => { out.insert(n.clone()); }
            crate::ast::SubPat::Variant(_, _, subs) => subs.iter().for_each(|s| go(s, out)),
            _ => {}
        }
    }
    go(&crate::ast::SubPat::decode(slot), out);
}

fn pat_tuple_names(p: &TuplePat, out: &mut HashSet<String>) {
    match p {
        TuplePat::Bind(_, _, n, _) => { out.insert(n.clone()); }
        TuplePat::Tuple(_, _, ps) => ps.iter().for_each(|q| pat_tuple_names(q, out)),
        TuplePat::Wild => {}
    }
}

/// Assignment targets and the names nested matches declare (binders, tuple patterns) or use in an `else` block,
/// which the shared walker does not report.
fn pat_targets(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Assign(a) => { out.insert(a.name.clone()); }
            Stmt::FieldAssign(a) => { out.insert(a.name.clone()); }
            Stmt::IndexAssign(a) => { out.insert(a.name.clone()); }
            Stmt::If(x) => { pat_targets(&x.then, out); pat_targets(&x.els, out); }
            Stmt::While(x) => pat_targets(&x.body, out),
            Stmt::ForMap(x) => pat_targets(&x.body, out),
            Stmt::Match(m) => {
                for a in &m.arms {
                    a.binders.iter().for_each(|b| pat_slot_names(b, out));
                    a.pats.iter().flatten().for_each(|p| pat_tuple_names(p, out));
                    pat_targets(&a.body, out);
                }
                pat_names_stmts(&m.els, out);
            }
            _ => {}
        }
    }
}

fn pat_cond(line: i64, col: i64, v: &str, lit: &crate::ast::SubPat) -> (Expr, bool) {
    use crate::ast::SubPat;
    match lit {
        SubPat::Bool(b) => (pat_var(line, col, v), !*b),
        SubPat::Int(x) => {
            // a signed literal, as the parser folds `-N` (so `-9223372036854775808` stays in range)
            let lit = Expr::IntLit(IntLit { line, col, value: x.clone(), ty: None, kind: Default::default() });
            (Expr::Binary(Binary { line, col, op: "==".into(), left: Box::new(pat_var(line, col, v)), right: Box::new(lit), ty: None }), false)
        }
        _ => unreachable!("a literal test"),
    }
}

/// P9a-6: the statements of a lowered pattern match; `bodies[k]` is arm k's body (its P8 tuple lets come from
/// `tuples[k]`).
fn pat_stmts(node: &PNode, scrut: &Expr, bodies: &[Vec<Stmt>], tuples: &[Vec<(String, TuplePat)>], line: i64, col: i64)
    -> Result<Vec<Stmt>, PruvError>
{
    Ok(match node {
        PNode::Leaf(k, binds) => {
            let mut out = Vec::new();
            for (n, v, ty) in binds {
                if n != v {
                    out.push(Stmt::Let(Let { borrowed: false, line, col, name: n.clone(), mutable: false, ty: ty.to_key(),
                        value: pat_var(line, col, v), refine: None }));
                }
                if let Some((_, TuplePat::Tuple(l, c, ps))) = tuples[*k].iter().find(|(t, _)| t == n) {
                    lower_arm_pat(n, ps, ty, *l, *c, &mut out)?;
                }
            }
            out.extend(bodies[*k].iter().cloned());
            out
        }
        PNode::Match(v, arms) => {
            let mut ms = Vec::new();
            for (en, ctor, names, child) in arms {
                ms.push(MatchArm { line, col, enum_name: en.clone(), ctor: ctor.clone(), binders: names.clone(),
                    body: pat_stmts(child, scrut, bodies, tuples, line, col)?, pats: Vec::new(), pat_lets: 0 });
            }
            let scrutinee = if v.is_empty() { scrut.clone() } else { pat_var(line, col, v) };
            vec![Stmt::Match(Match { line, col, scrutinee, arms: ms, form: MatchForm::Match, els: Vec::new() })]
        }
        PNode::Test(v, lit, a, b) => {
            let (cond, swap) = pat_cond(line, col, v, lit);
            let (a, b) = (pat_stmts(a, scrut, bodies, tuples, line, col)?, pat_stmts(b, scrut, bodies, tuples, line, col)?);
            let (then, els) = if swap { (b, a) } else { (a, b) };
            vec![Stmt::If(If { line, col, cond, then, els })]
        }
    })
}

/// P9a-6: the value of a lowered value `match`: nested value matches and value `if`s; a leaf is its arm value
/// with each renamed binder replaced by the value it names.
fn pat_value(node: &PNode, scrut: &Expr, vals: &[Expr], line: i64, col: i64) -> Expr {
    match node {
        PNode::Leaf(k, binds) => {
            let mut e = vals[*k].clone();
            for (n, v, _) in binds {
                if n != v {
                    rename_var(&mut e, n, v);
                }
            }
            e
        }
        PNode::Match(v, arms) => {
            let scrutinee = if v.is_empty() { scrut.clone() } else { pat_var(line, col, v) };
            let arms = arms.iter().map(|(en, ctor, names, child)| (crate::ast::ValueArm {
                enum_name: en.clone(), ctor: ctor.clone(), binders: names.clone() }, pat_value(child, scrut, vals, line, col))).collect();
            crate::ast::ValueMatch::expr(line, col, scrutinee, arms)
        }
        PNode::Test(v, lit, a, b) => {
            let (cond, swap) = pat_cond(line, col, v, lit);
            let (a, b) = (pat_value(a, scrut, vals, line, col), pat_value(b, scrut, vals, line, col));
            let (then, els) = if swap { (b, a) } else { (a, b) };
            ValueIf::expr(line, col, cond, then, els)
        }
    }
}

/// P9a-6: replaces the variable `from` by `to` in an expression (binder names are never shadowed: type/shadow).
fn rename_var(e: &mut Expr, from: &str, to: &str) {
    match e {
        Expr::Var(x) if x.name == from => x.name = to.to_string(),
        Expr::Move(x) if x.name == from => x.name = to.to_string(),
        _ => {}
    }
    match e {
        Expr::InoutArg(x) => rename_var(&mut x.expr, from, to),
        Expr::Unary(x) => rename_var(&mut x.operand, from, to),
        Expr::Binary(x) => {
            rename_var(&mut x.left, from, to);
            rename_var(&mut x.right, from, to);
        }
        Expr::Index(x) => {
            rename_var(&mut x.array, from, to);
            rename_var(&mut x.index, from, to);
        }
        Expr::Ctor(x) => x.arg.iter_mut().for_each(|a| rename_var(a, from, to)),
        Expr::Quant(x) => {
            for k in [&mut x.lo, &mut x.hi, &mut x.body] {
                rename_var(k, from, to);
            }
            x.triggers.iter_mut().for_each(|a| rename_var(a, from, to));
        }
        Expr::Call(x) => x.args.iter_mut().for_each(|a| rename_var(a, from, to)),
        Expr::StructLit(x) => x.fields.iter_mut().for_each(|f| rename_var(&mut f.value, from, to)),
        Expr::EnumCtor(x) => x.args.iter_mut().for_each(|a| rename_var(a, from, to)),
        Expr::FieldAccess(x) => rename_var(&mut x.base, from, to),
        Expr::IsVariant(x) => rename_var(&mut x.value, from, to),
        Expr::VariantField(x) => rename_var(&mut x.value, from, to),
        _ => {}
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
        assert!(int_in_range("9223372036854775807", &intwidth::I64));
        assert!(!int_in_range("9223372036854775808", &intwidth::I64));
        assert!(int_in_range("-9223372036854775808", &intwidth::I64));
        assert!(!int_in_range("-9223372036854775809", &intwidth::I64));
        assert!(int_in_range("0", &intwidth::I64));
        assert!(!int_in_range("100000000000000000000000000000000000000000", &intwidth::I64));
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
        assert!(run("fn f(a: [u16], b: [u16]) -> bool { return eq_range(a, 0, b, 1, 2); }").is_ok());
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
        assert!(run("fn f(inout a: [u16]) -> () { copy_within(inout a, 1, 0, 2); }").is_ok());
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
        assert!(run("fn f(inout d: [u16], s: [u16]) -> () { copy_range(inout d, 0, s, 1, 2); }").is_ok());
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
        assert!(run("fn f(inout a: [u8]) -> () { copy_range(inout a, 0, a, 4, 2); }").is_ok());
        let e = run("fn f(inout a: [u8]) -> () ! alloc { copy_range(inout a, 0, copy(a), 4, 2); }").unwrap_err();
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
                Stmt::Break(_) | Stmt::Continue(_) => {}
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
            Stmt::Break(_) | Stmt::Continue(_) => {}
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

/// LB7 (R-6g6-3), LB9 (R-6g8-1): catalog text operations whose text argument may be a str subview.
fn is_text_view_op(n: &str) -> bool {
    matches!(n, "string_push_str" | "string_from" | "proc_args_push")
}

/// R-6g9-3: `[]` outside a `ref [T]` view position
fn empty_view_err(line: i64, col: i64) -> PruvError {
    terr(line, col, "type_mismatch", "`[]` is the static empty view; it is only allowed in a `ref [T]` position",
        "use it as a `ref [T]` let, a `ref` field or the result of a `from` function, or alloc(0, v) for an owned array")
}

/// P7-1 R1: the type parameters (owner, index) mentioned anywhere in `t`
fn collect_tparams(t: &GType, out: &mut HashSet<(String, usize)>) {
    if let Some(o) = &t.owner {
        out.insert((o.clone(), t.index));
    }
    for a in &t.args {
        collect_tparams(a, out);
    }
}

/// P7-1 B: inside a contract a `[T; N]` struct or variant field takes a place (a variable or field path); a literal
/// or call there has no contract storage yet (parity row P7-8).
fn fixed_contract_field(arg: &Expr, fty: &GType, c: bool) -> Res<()> {
    if c && fty.is_fixed() && matches!(arg, Expr::Call(_)) {
        return Err(terr(arg.line(), arg.col(), "fixed_array_operation",
            "inside a contract a fixed-length array field of a struct literal or enum constructor must be a variable or field path, not a literal or call",
            "bind the array with a let before the function, or compare its elements directly"));
    }
    Ok(())
}

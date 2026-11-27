//! LB2 ownership restrictions. The checker and verifier use the underlying value types.
use crate::ast::*;
use crate::check::{expr_ty, field_place, mentions, paths_overlap, place_root};
use crate::diagnostic::PruvError;
use crate::foreign_shim::{walk_expr, Node};
use crate::gtype::{bind_args, decl_owner, substitute, GType};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};

type Res<T> = Result<T, PruvError>;
pub(crate) type PlaceRoots = HashMap<(i64, i64), (String, Vec<String>)>;
pub(crate) type PlaceFunctions = HashMap<String, PlaceRoots>;

fn error(line: i64, col: i64, code: &str, message: &str) -> PruvError {
    PruvError::new("type", line, col, code, message, "")
}

fn signature(line: i64, col: i64) -> PruvError {
    error(line, col, "borrow_signature", "borrowed type or from-list is not allowed here")
}

pub(crate) fn borrowing_struct(decl: &TypeDecl) -> bool {
    matches!(decl, TypeDecl::Struct(s) if s.fields.iter().any(|f| f.borrowed))
}

fn contains_borrow(ty: &GType, structs: &HashSet<String>) -> bool {
    ty.name == "@ref" || structs.contains(&ty.name)
        || ty.args.iter().any(|t| contains_borrow(t, structs))
}

pub(crate) fn is_view_base(ty: &str) -> bool {
    crate::typeinfo::is_array_name(ty) || ty == "str"
}

/// R-6g9-10: the element base of a view list, `str` or (L3-VB) `array_u8`.
fn view_list_base(t: &GType) -> Option<&'static str> {
    match t.view_base()?.name.as_str() {
        "str" => Some("str"),
        "array_u8" => Some("array_u8"),
        _ => None,
    }
}

/// R-6g9-10: the declared element base of a view list type `Vec<ref str>` / `Vec<ref [u8]>`.
fn view_list_type_base(ty: &str) -> Option<&'static str> {
    let parsed = GType::from_key(ty);
    if parsed.owner.is_none() && matches!(parsed.name.as_str(), "vec" | "Vec") && parsed.args.len() == 1 {
        view_list_base(&parsed.args[0])
    } else {
        None
    }
}

/// R-6g9-10: the element base (`str` or `array_u8`) of a view-list operation's type argument `ref str` / `ref [u8]`.
pub(crate) fn view_list_targ(targs: &Option<Vec<GType>>) -> Option<&'static str> {
    match targs.as_deref() {
        Some([t]) => view_list_base(t),
        _ => None,
    }
}

/// R-6g9-10: the element type argument `ref str` or `ref [u8]` of a view-list operation.
pub(crate) fn is_view_list_targ(targs: &Option<Vec<GType>>) -> bool {
    view_list_targ(targs).is_some()
}

/// R-6g9-10: a view list after `prepare` (a borrowed `let` of type `vec<string>`).
pub(crate) fn is_view_list(s: &Let) -> bool {
    s.borrowed && s.ty == "vec<string>"
}

/// L3-VB: a view list of `[u8]` views. Its `vec_new::<ref [u8]>()` keeps the element type argument (the checker
/// models both lists as `vec<string>`, whose element bytes carry no UTF-8 fact).
pub(crate) fn is_byte_view_list(s: &Let) -> bool {
    is_view_list(s) && matches!(&s.value, Expr::Call(c) if view_list_targ(&c.targs) == Some("array_u8"))
}

pub(crate) fn is_view_wrapper(ty: &str) -> bool {
    (ty.starts_with("option<") || ty.starts_with("result<"))
        && GType::from_key(ty).view_payload().is_some()
}

pub(crate) fn uses_view_wrappers(f: &Function) -> bool {
    let mut found = is_view_wrapper(&f.ret);
    crate::foreign_shim::walk_stmts(&f.body, &mut |node| {
        if let Node::Expr(e) = node {
            found |= expr_ty(e).as_deref().is_some_and(is_view_wrapper);
        }
    });
    found
}

fn annotation(ty: &mut String, line: i64, col: i64, permitted: bool, wrapper: bool, structs: &HashSet<String>) -> Res<bool> {
    let parsed = GType::from_key(ty);
    if permitted {
        if let Some(base) = parsed.view_base() {
            *ty = base.to_key();
            return Ok(true);
        }
        if wrapper && parsed.view_payload().is_some() {
            return Ok(true);
        }
    }
    if contains_borrow(&parsed, structs) {
        Err(signature(line, col))
    } else {
        Ok(false)
    }
}

fn prepare_block(body: &mut [Stmt], permitted: bool, structs: &HashSet<String>) -> Res<()> {
    for st in body {
        match st {
            // R-6g9-10: a local `Vec<ref str>` is a view list: `vec<string>` to the checker and the verifier (they see the
            // text of its elements), `Vec<&str>` in Rust; `borrowed` marks it.
            Stmt::Let(s) if permitted && !s.is_place() && view_list_type_base(&s.ty).is_some() => {
                // L3-VB: the `vec_new` type argument names the same element as the declaration
                let base = view_list_type_base(&s.ty);
                if let Expr::Call(c) = &s.value {
                    if c.name == "vec_new" && view_list_targ(&c.targs).is_some_and(|b| Some(b) != base) {
                        return Err(signature(c.line, c.col));
                    }
                }
                s.ty = "vec<string>".into();
                s.borrowed = true;
            }
            Stmt::Let(s) if !s.is_place() => {
                // R-6g9-6: mutability is allowed for a borrowing struct, but not for a ref local.
                if !(permitted && structs.contains(&s.ty)) {
                    s.borrowed = annotation(&mut s.ty, s.line, s.col, permitted && !s.mutable, true, structs)?;
                }
            }
            Stmt::If(s) => {
                prepare_block(&mut s.then, permitted, structs)?;
                prepare_block(&mut s.els, permitted, structs)?;
            }
            Stmt::While(s) => prepare_block(&mut s.body, permitted, structs)?,
            Stmt::ForMap(s) => prepare_block(&mut s.body, permitted, structs)?,
            Stmt::Match(s) => for a in &mut s.arms { prepare_block(&mut a.body, permitted, structs)?; },
            _ => {}
        }
    }
    Ok(())
}

fn owned_buffer_path(
    ty: &str, path: &str, declarations: &HashMap<&str, &StructDecl>, borrowing: &HashSet<String>,
) -> Option<()> {
    let mut ty = GType::from_key(ty);
    for (i, name) in path.split('.').enumerate() {
        if ty.owner.is_some() || (i > 0 && borrowing.contains(&ty.name)) {
            return None;
        }
        let decl = declarations.get(ty.name.as_str())?;
        let field = decl.fields.iter().find(|f| f.name == name && !f.borrowed)?;
        let args = bind_args(&decl_owner(&ty.name), decl.tparams.len(), &ty.args).ok()?;
        ty = substitute(&GType::from_key(&field.ty), &args);
    }
    is_view_base(&ty.to_key()).then_some(())
}

pub(crate) fn prepare(prog: &mut Program, hosts: &[bool]) -> Res<()> {
    let structs: HashSet<String> = prog.types.iter().filter_map(|d| match d {
        TypeDecl::Struct(s) if s.fields.iter().any(|f| GType::from_key(&f.ty).name == "@ref") => Some(s.name.clone()),
        _ => None,
    }).collect();
    for d in &mut prog.types {
        match d {
            TypeDecl::Struct(s) => {
                for f in &mut s.fields {
                    if !(s.tparams.is_empty() && structs.contains(&s.name) && structs.contains(&f.ty)) {
                        f.borrowed = annotation(&mut f.ty, f.line, f.col, s.tparams.is_empty(), false, &structs)?;
                    }
                }
            }
            TypeDecl::Enum(e) => {
                for v in &mut e.variants {
                    for f in &mut v.fields {
                        annotation(&mut f.ty, f.line, f.col, false, false, &structs)?;
                    }
                }
            }
        }
    }
    let declarations: HashMap<_, _> = prog.types.iter().filter_map(|d| match d {
        TypeDecl::Struct(s) => Some((s.name.as_str(), s)),
        _ => None,
    }).collect();
    for (i, f) in prog.functions.iter_mut().enumerate() {
        let permitted = f.tparams.is_empty() && f.foreign.is_none() && !hosts.get(i).copied().unwrap_or(false);
        for p in &mut f.params {
            // A read-only ref [u16] parameter has the same lifetime as a plain read parameter.
            let read_u16_view = permitted && !p.take && p.mode_pos.is_none() && p.ty == "@ref<array_u16>";
            // R-6g9-6: borrowing structs may be inout parameters.
            if !(permitted && structs.contains(&p.ty) && !p.take) {
                annotation(&mut p.ty, f.line, f.col, read_u16_view, false, &structs)?;
            }
        }
        if !(permitted && structs.contains(&f.ret)) {
            f.borrowed_result = annotation(&mut f.ret, f.line, f.col, permitted, true, &structs)?;
        }
        let borrowed_result = f.borrowed_result || structs.contains(&f.ret);
        match &f.borrow_from {
            Some(names) if borrowed_result && !names.is_empty() => {
                for (i, name) in names.iter().enumerate() {
                    if names[..i].contains(name) {
                        return Err(signature(f.line, f.col));
                    }
                    let (root, path) = name.split_once('.').map_or((name.as_str(), None), |(root, path)| (root, Some(path)));
                    let Some(p) = f.params.iter().find(|p| p.name == root) else {
                        return Err(signature(f.line, f.col));
                    };
                    if let Some(path) = path {
                        if p.take || p.mode_pos.is_some() || names.contains(&p.name)
                            || owned_buffer_path(&p.ty, path, &declarations, &structs).is_none()
                        {
                            return Err(signature(f.line, f.col));
                        }
                        continue;
                    }
                    if p.take || !(is_view_base(&p.ty) || (structs.contains(&p.ty) && p.mode_pos.is_none())) {
                        return Err(signature(f.line, f.col));
                    }
                }
                let inouts: Vec<&String> = names.iter()
                    .filter(|n| f.params.iter().any(|p| p.name == **n && p.mode_pos.is_some())).collect();
                if let Some(e) = f.ensures.iter().find(|e| inouts.iter().any(|n| mentions(e, n))) {
                    return Err(signature(e.line(), e.col()));
                }
            }
            None if !borrowed_result => {}
            _ => return Err(signature(f.line, f.col)),
        }
        prepare_block(&mut f.body, f.tparams.is_empty(), &structs)?;
    }
    Ok(())
}

pub(crate) fn first_borrow(prog: &Program) -> Option<(i64, i64)> {
    for d in &prog.types {
        if let TypeDecl::Struct(s) = d {
            if borrowing_struct(d) {
                return Some((s.line, s.col));
            }
        }
    }
    for f in &prog.functions {
        if f.borrow_from.is_some() {
            return Some((f.line, f.col));
        }
        let mut position = None;
        crate::foreign_shim::walk_stmts(&f.body, &mut |node| {
            match node {
                Node::Let(s) if s.borrowed => {
                    position.get_or_insert((s.line, s.col));
                }
                Node::Expr(e @ Expr::Ctor(_)) if expr_ty(e).as_deref().is_some_and(is_view_wrapper) => {
                    position.get_or_insert((e.line(), e.col()));
                }
                _ => {}
            }
        });
        if position.is_some() {
            return position;
        }
    }
    None
}

pub(crate) fn static_str_argument(arg: &Expr, p: &Param) -> bool {
    p.ty == "str" && !p.take && p.mode_pos.is_none() && matches!(arg, Expr::StrLit(_))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Access { Read, Write }

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Source {
    root: String,
    binding: Option<usize>,
}

type Roots = BTreeMap<Source, Access>;

fn merge(roots: &mut Roots, source: Source, access: Access) {
    let held = roots.entry(source).or_insert(access);
    *held = (*held).max(access);
}

impl Source {
    fn direct(root: &str) -> Self {
        Self { root: root.to_owned(), binding: None }
    }
}

pub(crate) fn walk_statement<'a>(
    st: &'a Stmt,
    expr: &mut impl FnMut(&'a Expr),
    write: &mut impl FnMut(&'a str, &'a [String], i64, i64),
    block: &mut impl FnMut(&'a [Stmt]),
) {
    match st {
        Stmt::Let(s) if s.is_place() => place_operands(&s.value, expr),
        Stmt::Let(s) => expr(&s.value),
        Stmt::Assign(s) => { write(&s.name, &[], s.line, s.col); expr(&s.value); }
        Stmt::IndexAssign(s) => { write(&s.name, &s.fields, s.line, s.col); expr(&s.index); expr(&s.value); }
        Stmt::FieldAssign(s) => { write(&s.name, &s.fields, s.line, s.col); expr(&s.value); }
        Stmt::If(s) => { expr(&s.cond); block(&s.then); block(&s.els); }
        Stmt::While(s) => {
            expr(&s.cond);
            s.invariants.iter().for_each(&mut *expr);
            expr(&s.decreases);
            block(&s.body);
        }
        Stmt::ForMap(s) => {
            expr(&s.place);
            s.invariants.iter().for_each(&mut *expr);
            block(&s.body);
        }
        Stmt::Match(s) => { expr(&s.scrutinee); for a in &s.arms { block(&a.body); } }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Return(s) => s.value.iter().for_each(expr),
        Stmt::Assert(s) => expr(&s.cond),
        Stmt::ExprStmt(s) => expr(&s.expr),
        Stmt::Par(s) => s.calls.iter().for_each(expr),
    }
}

/// The root accesses of `st` itself: a compound statement contributes its head (condition, contracts, place,
/// scrutinee); its nested blocks are their own statements in the liveness graph.
/// The first `move` in the value of a `let` or whole rebind that moves a root the new view borrows
/// (`Doc { src: x, scratch: move x }`): it conflicts with the view itself, reported only where reachable.
fn moved_root(st: &Stmt, roots: &Roots) -> Option<(i64, i64)> {
    let (Stmt::Let(Let { value, .. }) | Stmt::Assign(Assign { value, .. })) = st else { return None };
    let mut found = None;
    walk_expr(value, &mut |node| {
        if let Node::Expr(Expr::Move(m)) = node {
            if found.is_none() && roots.keys().any(|source| source.root == m.name) {
                found = Some((m.line, m.col));
            }
        }
    });
    found
}

/// `writes_only`: the operands have their own earlier point (P10 read-first stores).
fn own_accesses(st: &Stmt, writes_only: bool, visit: &mut impl FnMut(&str, &[String], Access)) {
    let mut uses = Vec::new();
    let mut writes = Vec::new();
    walk_statement(st, &mut |e| uses.push(e), &mut |n, path, _, _| writes.push((n, path)), &mut |_| {});
    for (name, path) in writes {
        visit(name, path, Access::Write);
    }
    for e in uses {
        if !writes_only {
            expr_accesses(e, visit);
        }
    }
}

/// A value whose store copies it: an integer, `bool` or `char` (no view or owned buffer survives the store).
fn is_copy(e: &Expr) -> bool {
    expr_ty(e).as_deref().is_some_and(|t| matches!(t, "i64" | "bool") || crate::intwidth::of(t).is_some())
}

fn expr_accesses(e: &Expr, visit: &mut impl FnMut(&str, &[String], Access)) {
    match e {
        Expr::Move(m) => visit(&m.name, &m.fields, Access::Write),
        Expr::InoutArg(arg) => {
            let (root, path) = field_place(&arg.expr);
            if let Expr::Var(v) = root {
                visit(&v.name, &path, Access::Write);
            }
            // L-gaps 8 (P10-M1): `inout v[i]` (swap/replace of an element) writes the array v
            if let Expr::Index(ix) = &*arg.expr {
                if let Expr::Var(v) = &*ix.array {
                    visit(&v.name, &[], Access::Write);
                }
            }
            expr_accesses(&arg.expr, visit);
        }
        _ => {
            let (root, path) = field_place(e);
            if let Expr::Var(v) = root {
                visit(&v.name, &path, Access::Read);
                return;
            }
            for child in crate::refine::kids(e) {
                expr_accesses(child, visit);
            }
        }
    }
}

fn place_operands<'a>(e: &'a Expr, visit: &mut impl FnMut(&'a Expr)) {
    match e {
        Expr::InoutArg(arg) => place_operands(&arg.expr, visit),
        Expr::Call(c) if PlaceIf::matches(c) => {
            let [cond, then, els] = c.args.as_slice() else { unreachable!() };
            visit(cond);
            place_operands(then, visit);
            place_operands(els, visit);
        }
        Expr::FieldAccess(f) => place_operands(&f.base, visit),
        Expr::Index(i) => {
            place_operands(&i.array, visit);
            visit(&i.index);
        }
        _ => {}
    }
}

/// A fixed-width bit set over the borrow variables of one function (R-6g9-4).
#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(width: usize) -> Bits { Bits(vec![0; width.div_ceil(64)]) }
    fn set(&mut self, bit: usize) { self.0[bit / 64] |= 1 << (bit % 64); }
    fn contains(&self, bit: usize) -> bool { self.0[bit / 64] & (1 << (bit % 64)) != 0 }
    fn clear(&mut self, bit: usize) { self.0[bit / 64] &= !(1 << (bit % 64)); }
    fn union(&mut self, other: &Bits) { self.0.iter_mut().zip(&other.0).for_each(|(a, b)| *a |= b); }
    fn meets(&self, other: &Bits) -> bool { self.0.iter().zip(&other.0).any(|(a, b)| a & b != 0) }
}

/// One statement of the function body, numbered in pre-order. `gen` / `kill` are borrow variables (bits).
struct Point {
    succ: Vec<usize>,
    gen: Vec<usize>,
    kill: Option<usize>,
    uses: Vec<(String, Vec<String>, Access)>,
    site: (i64, i64),
    /// A borrow `let` whose value moves a root of its own provenance: the position of that move.
    own_move: Option<(i64, i64)>,
}

/// The statement graph of one function, the provenance of each borrow variable, and the first error that does not
/// depend on liveness with the point where it was found (a conflict at that point or before it is reported first).
#[derive(Default)]
struct Graph {
    points: Vec<Point>,
    borrows: Vec<Roots>,
    places: HashMap<usize, (String, String, Vec<String>)>,
    failure: Option<(usize, PruvError)>,
    /// P5: enclosing while loops during `build`: (label, head point, break points).
    loops: Vec<(Option<String>, usize, Vec<usize>)>,
}

impl Graph {
    /// Backward liveness by repeated sweeps over the points from the last to the first:
    /// live[n] = gen[n] | (union of live[succ] minus kill[n]), stored in place, until a whole sweep changes nothing.
    /// Points are in pre-order and only loop back edges go to lower indices, so this takes loop depth + 2 sweeps,
    /// i.e. O((d+2)·S·B/64). Returns the live-in set of each point.
    fn liveness(&self) -> Vec<Bits> {
        let width = self.borrows.len();
        let count = self.points.len();
        let mut live = vec![Bits::new(width); count];
        let mut changed = true;
        while changed {
            changed = false;
            for n in (0..count).rev() {
                let point = &self.points[n];
                let mut set = Bits::new(width);
                for &s in &point.succ {
                    set.union(&live[s]);
                }
                if let Some(k) = point.kill {
                    set.clear(k);
                }
                for &g in &point.gen {
                    set.set(g);
                }
                if set != live[n] {
                    live[n] = set;
                    changed = true;
                }
            }
        }
        live
    }

    /// The points reachable from the entry, by one forward pass in index order (the only edges to lower indices are
    /// loop back edges to a head that is already reachable through its forward predecessor).
    fn reachable(&self) -> Vec<bool> {
        let mut reach = vec![false; self.points.len()];
        if let Some(first) = reach.first_mut() {
            *first = true;
        }
        for n in 0..self.points.len() {
            if reach[n] {
                for &s in &self.points[n].succ {
                    reach[s] = true;
                }
            }
        }
        reach
    }

    /// One synthetic join point after the branches of an `if` / `match` (no uses, no gen/kill): every exit flows into
    /// it. Returns the single-point exit list, or empty when no branch falls through.
    fn join(&mut self, exits: Vec<usize>) -> Vec<usize> {
        if exits.is_empty() {
            return exits;
        }
        let join = self.points.len();
        self.points.push(Point { succ: Vec::new(), gen: Vec::new(), kill: None, uses: Vec::new(), site: (0, 0), own_move: None });
        for p in exits {
            self.points[p].succ.push(join);
        }
        vec![join]
    }

    /// R-6g9-4: a use conflicts with the borrows live at its point. Root masks: per root, the borrows that hold it
    /// shared and those that hold it exclusive; a read meets the exclusive mask, a write either. Only points
    /// reachable from the entry are checked; a structural failure is raised regardless of reachability.
    /// Cost: O((d+2)·S·B/64 + M·B/64 + Σ|provenance|).
    fn conflicts(self) -> Res<()> {
        let width = self.borrows.len();
        let mut masks: HashMap<&str, [Bits; 2]> = HashMap::new();
        let mut places: HashMap<&str, (Bits, Vec<(usize, &[String])>)> = HashMap::new();
        let mut add_place = |bit, root, footprint| {
            let (mask, paths) = places.entry(root).or_insert_with(|| (Bits::new(width), Vec::new()));
            mask.set(bit);
            paths.push((bit, footprint));
        };
        for (&bit, (_, root, footprint)) in &self.places {
            add_place(bit, root.as_str(), footprint.as_slice());
        }
        for (bit, roots) in self.borrows.iter().enumerate() {
            for (source, access) in roots {
                let name = if let Some(binding) = source.binding {
                    // The view holds the place's footprint and borrows its alias with the view's own mode.
                    let (alias, _, footprint) = &self.places[&binding];
                    add_place(bit, source.root.as_str(), footprint.as_slice());
                    alias.as_str()
                } else {
                    source.root.as_str()
                };
                let mask = masks.entry(name).or_insert_with(|| [Bits::new(width), Bits::new(width)]);
                mask[(*access == Access::Write) as usize].set(bit);
            }
        }
        let live = self.liveness();
        let reach = self.reachable();
        let last = self.failure.as_ref().map_or(self.points.len(), |(n, _)| n + 1);
        for ((point, live), &reached) in self.points[..last].iter().zip(&live).zip(&reach) {
            if !reached { continue; }
            if let Some((line, col)) = point.own_move {
                return Err(error(line, col, "borrow_conflict", "a root moved while the new view borrows it"));
            }
            for (name, path, access) in &point.uses {
                if let Some((mask, paths)) = places.get(name.as_str()) {
                    if live.meets(mask) && paths.iter().any(|(bit, foot)| live.contains(*bit) && paths_overlap(path, foot)) {
                        let place = std::iter::once(name.as_str()).chain(path.iter().map(String::as_str))
                            .collect::<Vec<_>>().join(".");
                        return Err(PruvError::new("type", point.site.0, point.site.1, "borrow_conflict",
                            format!("{place} overlaps the place bound by a live place binding"),
                            "use the binding instead, or move this after its last use"));
                    }
                }
                let Some([shared, exclusive]) = masks.get(name.as_str()) else { continue };
                if live.meets(exclusive) || (*access == Access::Write && live.meets(shared)) {
                    return Err(PruvError::new("type", point.site.0, point.site.1, "borrow_conflict",
                        format!("{name} is still borrowed by a live view"), ""));
                }
            }
        }
        match self.failure {
            Some((_, e)) => Err(e),
            None => Ok(()),
        }
    }
}

enum Origin {
    View(Roots),
    // A struct parameter can carry its caller's shared provenance.
    Param,
    Place(Source),
}

struct Analysis<'a> {
    /// R-6g9-10: the provenance of each view list of the current function (the union over its pushes).
    lists: RefCell<HashMap<String, Roots>>,
    /// R-6g9-10: the names visible where each view list is declared (parameters and earlier locals of enclosing blocks).
    list_scope: RefCell<HashMap<String, HashSet<String>>>,
    visible: RefCell<Vec<String>>,
    /// R-6g9-10: the fixpoint passes only grow `lists`; the final pass checks.
    collecting: Cell<bool>,
    place_functions: &'a PlaceFunctions,
    functions: HashMap<&'a str, &'a Function>,
    records: HashMap<&'a str, &'a StructDecl>,
    structs: HashSet<&'a str>,
}

impl Analysis<'_> {
    fn ref_field(&self, base: &Expr, name: &str) -> bool {
        expr_ty(base).as_deref().and_then(|ty| self.records.get(ty))
            .is_some_and(|s| s.fields.iter().any(|f| f.name == name && (f.borrowed || self.records.contains_key(f.ty.as_str()))))
    }

    fn list_name<'e>(&self, e: &'e Expr) -> Option<&'e str> {
        let e = match e { Expr::InoutArg(x) => &*x.expr, e => e };
        match e {
            Expr::Var(v) if self.lists.borrow().contains_key(&v.name) => Some(&v.name),
            _ => None,
        }
    }

    /// R-6g9-10: `vec_get(l, i)` of a view list `l`.
    fn list_read(&self, e: &Expr) -> bool {
        matches!(e, Expr::Call(x) if x.name == "vec_get" && !self.functions.contains_key("vec_get")
            && x.args.first().is_some_and(|a| !matches!(a, Expr::InoutArg(_)) && self.list_name(a).is_some()))
    }

    /// R-6g9-10: a push adds the element's provenance to the list; every root must be visible where the list is
    /// declared, so the list never outlives a local it borrows.
    fn list_push(&self, list: &str, value: &Expr, locals: &HashMap<String, Origin>) -> Res<()> {
        let roots = self.provenance(value, locals)?;
        let scope = self.list_scope.borrow();
        for source in roots.keys() {
            let base = source.root.split('.').next().unwrap_or(&source.root);
            if scope.get(list).is_some_and(|names| !names.contains(base)) {
                return Err(PruvError::new("type", value.line(), value.col(), "borrow_escape",
                    format!("{list} would outlive {base}, declared after the list or in an inner block"),
                    "declare the list after the data it borrows"));
            }
        }
        let mut lists = self.lists.borrow_mut();
        let held = lists.get_mut(list).expect("view list");
        if self.collecting.get() {
            for (source, access) in roots {
                merge(held, source, access);
            }
        } else if roots.iter().any(|(source, access)| held.get(source).is_none_or(|prev| access > prev)) {
            return Err(error(value.line(), value.col(), "borrow_escape", "pushed view borrows outside the list"));
        }
        Ok(())
    }

    fn is_view(&self, e: &Expr, locals: &HashMap<String, Origin>) -> bool {
        if self.list_read(e) {
            return true;
        }
        match e {
            Expr::Var(x) => matches!(locals.get(&x.name), Some(Origin::View(_)))
                && expr_ty(e).as_deref().is_some_and(is_view_base),
            Expr::Call(x) => self.functions.get(x.name.as_str()).is_some_and(|f| f.borrowed_result),
            Expr::FieldAccess(x) => self.ref_field(&x.base, &x.field)
                && expr_ty(e).as_deref().is_some_and(is_view_base),
            _ => false,
        }
    }

    fn provenance(&self, e: &Expr, locals: &HashMap<String, Origin>) -> Res<Roots> {
        if let Expr::Call(x) = e {
            if self.list_read(e) {
                return self.provenance(&x.args[0], locals);
            }
        }
        let mut roots = Roots::new();
        if let Some(root) = place_root(e) {
            match locals.get(root) {
                Some(Origin::Place(source)) => merge(&mut roots, source.clone(), Access::Read),
                Some(Origin::View(origin)) => {
                    // An owned field lives in the record's storage and carries nothing the record borrows.
                    if matches!(e, Expr::FieldAccess(x) if !self.ref_field(&x.base, &x.field)) {
                        merge(&mut roots, Source::direct(root), Access::Read);
                    } else {
                        roots = origin.clone();
                    }
                }
                origin => {
                    merge(&mut roots, Source::direct(root), Access::Read);
                    // Owned field storage needs its own from entry.
                    if matches!(e, Expr::FieldAccess(x) if !self.ref_field(&x.base, &x.field))
                        && matches!(origin, Some(Origin::Param))
                    {
                        let (_, path) = field_place(e);
                        merge(&mut roots, Source::direct(&format!("{root}.{}", path.join("."))), Access::Read);
                    }
                }
            }
        } else {
            match e {
                Expr::StrLit(_) => {}
                Expr::Ctor(c) if expr_ty(e).as_deref().is_some_and(is_view_wrapper) => {
                    if matches!(c.name.as_str(), "Some" | "Ok") {
                        return self.provenance(c.arg.as_deref().unwrap(), locals);
                    }
                }
                Expr::Call(c) if c.name == "subview" => return self.provenance(&c.args[0], locals),
                Expr::Call(c) => {
                    let Some(f) = self.functions.get(c.name.as_str()) else {
                        return Err(signature(e.line(), e.col()));
                    };
                    let Some(from) = &f.borrow_from else {
                        return Err(signature(e.line(), e.col()));
                    };
                    for (p, a) in f.params.iter().zip(&c.args) {
                        let mut fields = f.lent_fields(&p.name).peekable();
                        if fields.peek().is_some() {
                            let (base, prefix) = field_place(a);
                            let Expr::Var(v) = base else { return Err(signature(a.line(), a.col())); };
                            let origin = locals.get(&v.name);
                            let source = match origin {
                                Some(Origin::Place(source)) => source.clone(),
                                _ => Source::direct(&v.name),
                            };
                            merge(&mut roots, source, Access::Read);
                            // A shared reference into a borrowing root keeps that root's loans live too:
                            // `&'a Doc<'_>` requires the borrowed fields to outlive `'a`.
                            if let Some(Origin::View(held)) = origin {
                                for (source, access) in held {
                                    merge(&mut roots, source.clone(), *access);
                                }
                            }
                            if matches!(origin, Some(Origin::Param)) {
                                let prefix = std::iter::once(v.name.as_str())
                                    .chain(prefix.iter().map(String::as_str)).collect::<Vec<_>>().join(".");
                                for path in fields {
                                    merge(&mut roots, Source::direct(&format!("{prefix}.{path}")), Access::Read);
                                }
                            }
                        }
                        if from.contains(&p.name) {
                            let (value, mode) = match a {
                                Expr::InoutArg(x) => (&*x.expr, Access::Write),
                                _ => (a, Access::Read),
                            };
                            for (source, access) in self.provenance(value, locals)? {
                                merge(&mut roots, source, access.max(mode));
                            }
                        }
                    }
                }
                Expr::StructLit(s) => for f in &s.fields {
                    if self.ref_field(e, &f.name) {
                        for (source, access) in self.provenance(&f.value, locals)? {
                            merge(&mut roots, source, access);
                        }
                    }
                },
                Expr::FieldAccess(f) if self.ref_field(&f.base, &f.field) => return self.provenance(&f.base, locals),
                _ => return Err(signature(e.line(), e.col())),
            }
        }
        Ok(roots)
    }

    fn owned(&self, e: &Expr, locals: &HashMap<String, Origin>) -> Res<()> {
        if self.is_view(e, locals) {
            return Err(error(e.line(), e.col(), "array_alias", "use copy to obtain an owned value from a borrowed view"));
        }
        Ok(())
    }

    /// The checks of one statement that do not depend on liveness; a borrow `let` records its provenance in `locals`
    /// and logs the previous entry in `undo`.
    fn statement(
        &self,
        f: &Function,
        st: &Stmt,
        locals: &mut HashMap<String, Origin>,
        undo: &mut Vec<(String, Option<Origin>)>,
    ) -> Res<Option<Roots>> {
        let mut expressions = Vec::new();
        walk_statement(st, &mut |e| expressions.push(e), &mut |_, _, _, _| {}, &mut |_| {});
        for e in expressions {
            let mut failure = None;
            walk_expr(e, &mut |node| {
                let Node::Expr(e) = node else { return };
                if failure.is_some() { return; }
                let result = match e {
                    Expr::Call(c) if c.name == "vec_push" && !self.functions.contains_key("vec_push")
                        && c.args.len() == 2 && self.list_name(&c.args[0]).is_some() =>
                    {
                        self.list_push(self.list_name(&c.args[0]).unwrap(), &c.args[1], locals)
                    }
                    Expr::Call(c) => {
                        if let Some(callee) = self.functions.get(c.name.as_str()) {
                            if callee.borrow_from.is_some() {
                                if let Err(error) = self.provenance(e, locals) {
                                    failure = Some(error);
                                    return;
                                }
                            }
                            c.args.iter().zip(&callee.params)
                                .filter(|(_, p)| p.take)
                                .try_for_each(|(a, _)| self.owned(a, locals))
                        } else {
                            Ok(())
                        }
                    }
                    Expr::StructLit(s) => s.fields.iter().try_for_each(|field| {
                        if self.ref_field(e, &field.name) { self.provenance(&field.value, locals).map(|_| ()) }
                        else { self.owned(&field.value, locals) }
                    }),
                    // Only the explicit builtin view wrapper can carry a borrowed payload.
                    Expr::EnumCtor(x) => x.args.iter().try_for_each(|a| self.owned(a, locals)),
                    Expr::Ctor(_) if expr_ty(e).as_deref().is_some_and(is_view_wrapper) =>
                        self.provenance(e, locals).map(|_| ()),
                    Expr::Ctor(x) => x.arg.iter().try_for_each(|a| self.owned(a, locals)),
                    _ => Ok(()),
                };
                failure = result.err();
                if matches!(st, Stmt::Par(_)) && (self.is_view(e, locals)
                    || expr_ty(e).as_deref().is_some_and(|ty| self.records.contains_key(ty))) {
                    failure = Some(signature(e.line(), e.col()));
                }
            });
            if let Some(e) = failure {
                return Err(e);
            }
        }
        match st {
            Stmt::Let(s) if s.is_place() => {}
            Stmt::Let(s) if is_view_list(s) => {
                let names = self.visible.borrow().iter().cloned().collect();
                self.list_scope.borrow_mut().insert(s.name.clone(), names);
                let roots = self.lists.borrow().get(&s.name).cloned().unwrap_or_default();
                undo.push((s.name.clone(), locals.insert(s.name.clone(), Origin::View(roots))));
            }
            Stmt::Let(s) if self.borrow_let(s) => {
                let roots = self.provenance(&s.value, locals)?;
                undo.push((s.name.clone(), locals.insert(s.name.clone(), Origin::View(roots))));
            }
            Stmt::Let(s) => self.owned(&s.value, locals)?,
            Stmt::Assign(s) => return self.check_store(&s.name, &s.value, true, st.position(), locals),
            Stmt::FieldAssign(s) => { self.check_store(&s.name, &s.value, s.ref_store, st.position(), locals)?; }
            Stmt::Return(s) => if let Some(value) = &s.value {
                if let Some(from) = &f.borrow_from {
                    let roots = self.provenance(value, locals)?;
                    // A bare parameter root is permitted only beside one of its lent field paths.
                    let listed = |root: &str| from.iter().any(|entry| entry == root);
                    if roots.keys().any(|source| !listed(&source.root) && !roots.keys().any(|other| {
                        listed(&other.root)
                            && other.root.strip_prefix(source.root.as_str()).is_some_and(|rest| rest.starts_with('.'))
                    })) {
                        return Err(error(s.line, s.col, "borrow_escape", "returned value borrows outside its from-list"));
                    }
                } else {
                    self.owned(value, locals)?;
                }
            },
            _ => {}
        }
        Ok(None)
    }

    // R-6g9-6: stores may only reuse the provenance already held by the target. Returns the provenance of a
    // borrowing store.
    fn check_store(
        &self, name: &str, value: &Expr, borrow_store: bool, (line, col): (i64, i64),
        locals: &HashMap<String, Origin>,
    ) -> Res<Option<Roots>> {
        if borrow_store {
            let held = locals.get(name);
            if matches!(held, Some(Origin::View(_) | Origin::Param)) {
                let roots = self.provenance(value, locals)?;
                if roots.iter().any(|(source, access)| match held {
                    Some(Origin::View(held)) => held.get(source).is_none_or(|prev| access > prev),
                    _ => source.root != name || source.binding.is_some() || *access != Access::Read,
                }) {
                    return Err(PruvError::new("type", line, col, "borrow_escape",
                        format!("{name} can only hold data it already borrows"),
                        "build it from a view of the same source, or bind a new borrowing local"));
                }
                return Ok(Some(roots));
            }
        }
        self.owned(value, locals).map(|_| None)
    }

    fn borrow_let(&self, s: &Let) -> bool {
        s.borrowed || self.records.contains_key(s.ty.as_str())
    }

    /// Adds the statements of `body` to `graph` in pre-order; `incoming` are the points that flow into the block.
    /// Returns the points that flow out of it, at most one: every `if` / `match` ends in one synthetic join point.
    /// `locals` (provenance) and `scope` (visible borrow variable -> bit) are one shared mutable map each; the block
    /// logs every insert in an undo log and restores both in reverse order before returning, so the caller sees
    /// them unchanged (a shadowing `let v = subview(v, ..)` still reads the old `v` while computing provenance).
    fn build<'a>(
        &self,
        f: &Function,
        body: &'a [Stmt],
        mut incoming: Vec<usize>,
        locals: &mut HashMap<String, Origin>,
        scope: &mut HashMap<&'a str, usize>,
        graph: &mut Graph,
    ) -> Vec<usize> {
        let mut locals_undo: Vec<(String, Option<Origin>)> = Vec::new();
        let mut scope_undo: Vec<(&'a str, Option<usize>)> = Vec::new();
        let visible = self.visible.borrow().len();
        for st in body {
            // P10 (G-B4-Q1): a store of a copied scalar evaluates its operands (index, then value) into locals
            // before it writes the target, so those operands are their own point: a view whose last read is in
            // them is dead at the write, as under NLL.
            let read_first = match st {
                Stmt::IndexAssign(s) => is_copy(&s.value),
                Stmt::FieldAssign(s) => !s.ref_store && is_copy(&s.value),
                _ => false,
            };
            if read_first {
                let n = graph.points.len();
                for p in incoming {
                    graph.points[p].succ.push(n);
                }
                let mut point = Point { succ: Vec::new(), gen: Vec::new(), kill: None, uses: Vec::new(), site: st.position(), own_move: None };
                let mut operands = Vec::new();
                walk_statement(st, &mut |e| operands.push(e), &mut |_, _, _, _| {}, &mut |_| {});
                for e in operands {
                    expr_accesses(e, &mut |name, path, access| {
                        point.gen.extend(scope.get(name).copied());
                        point.uses.push((name.to_owned(), path.to_vec(), access));
                    });
                }
                graph.points.push(point);
                incoming = vec![n];
            }
            let n = graph.points.len();
            for p in incoming {
                graph.points[p].succ.push(n);
            }
            let mut point = Point { succ: Vec::new(), gen: Vec::new(), kill: None, uses: Vec::new(), site: st.position(), own_move: None };
            // R-6g9-6: a whole rebind kills the old loan. Its target still counts as a write.
            if let Stmt::Assign(s) = st {
                if matches!(locals.get(&s.name), Some(Origin::View(_))) {
                    point.kill = scope.get(s.name.as_str()).copied();
                }
            }
            own_accesses(st, read_first, &mut |name, path, access| {
                if !(point.kill.is_some() && point.uses.is_empty()) {
                    point.gen.extend(scope.get(name).copied());
                }
                point.uses.push((name.to_owned(), path.to_vec(), access));
            });
            if graph.failure.is_none() {
                match self.statement(f, st, locals, &mut locals_undo) {
                    Err(e) => graph.failure = Some((n, e)),
                    // R-6g9-6: a whole rebind that moves a root its new value borrows conflicts like a `let`.
                    Ok(Some(roots)) if point.kill.is_some() => point.own_move = moved_root(st, &roots),
                    Ok(_) => {}
                }
            }
            if let Stmt::Let(s) = st {
                if s.is_place() {
                    let (root, footprint) = &self.place_functions[&f.name][&(s.line, s.col)];
                    point.uses.insert(0, (root.clone(), footprint.clone(), Access::Write));
                    point.gen.extend(scope.get(root.as_str()).copied());
                    let bit = graph.borrows.len();
                    let source = Source { root: root.clone(), binding: Some(bit) };
                    locals_undo.push((s.name.clone(), locals.insert(s.name.clone(), Origin::Place(source))));
                    point.kill = Some(bit);
                    scope_undo.push((&s.name, scope.insert(&s.name, bit)));
                    graph.borrows.push(Roots::new());
                    graph.places.insert(bit, (s.name.clone(), root.clone(), footprint.clone()));
                } else if self.borrow_let(s) {
                    point.kill = Some(graph.borrows.len());
                    scope_undo.push((&s.name, scope.insert(&s.name, graph.borrows.len())));
                    let roots = match locals.get(&s.name) {
                        Some(Origin::View(roots)) => roots.clone(),
                        _ => Roots::new(),
                    };
                    graph.borrows.push(roots);
                    point.own_move = moved_root(st, graph.borrows.last().unwrap());
                }
            }
            graph.points.push(point);
            if let Stmt::Let(s) = st {
                self.visible.borrow_mut().push(s.name.clone());
            }
            incoming = match st {
                Stmt::If(s) => {
                    let mut out = self.build(f, &s.then, vec![n], locals, scope, graph);
                    out.extend(self.build(f, &s.els, vec![n], locals, scope, graph));
                    graph.join(out)
                }
                Stmt::While(While { body, label, .. }) => {
                    // P5: `continue` is a back edge to the head, `break` an exit edge after the loop.
                    graph.loops.push((label.clone(), n, Vec::new()));
                    for p in self.build(f, body, vec![n], locals, scope, graph) {
                        graph.points[p].succ.push(n);
                    }
                    let mut out = graph.loops.pop().map(|l| l.2).unwrap_or_default();
                    out.insert(0, n);
                    out
                }
                Stmt::ForMap(ForMap { body, .. }) => {
                    for p in self.build(f, body, vec![n], locals, scope, graph) {
                        graph.points[p].succ.push(n);
                    }
                    vec![n]
                }
                Stmt::Break(j) | Stmt::Continue(j) => {
                    let brk = matches!(st, Stmt::Break(_));
                    if let Some(l) = graph.loops.iter_mut().rev().find(|l| j.label.is_none() || l.0 == j.label) {
                        if brk { l.2.push(n) } else { let head = l.1; graph.points[n].succ.push(head) }
                    }
                    Vec::new()
                }
                Stmt::Match(s) => {
                    let mut out = Vec::new();
                    let wrapper = expr_ty(&s.scrutinee).as_deref().is_some_and(is_view_wrapper);
                    for a in &s.arms {
                        if let Some(binder) = a.binders.first().filter(|_| wrapper && matches!(a.ctor.as_str(), "Some" | "Ok")) {
                            let roots = match self.provenance(&s.scrutinee, locals) {
                                Ok(roots) => roots,
                                Err(e) => {
                                    graph.failure.get_or_insert((n, e));
                                    Roots::new()
                                }
                            };
                            let bit = graph.borrows.len();
                            graph.borrows.push(roots.clone());
                            let old_origin = locals.insert(binder.clone(), Origin::View(roots));
                            let old_bit = scope.insert(binder, bit);
                            // The arm defines a fresh loan, including when this match is in a loop.
                            let entry = graph.points.len();
                            graph.points[n].succ.push(entry);
                            graph.points.push(Point {
                                succ: Vec::new(), gen: Vec::new(), kill: Some(bit),
                                uses: Vec::new(), site: st.position(), own_move: None,
                            });
                            out.extend(self.build(f, &a.body, vec![entry], locals, scope, graph));
                            if let Some(old) = old_origin { locals.insert(binder.clone(), old); }
                            else { locals.remove(binder); }
                            if let Some(old) = old_bit { scope.insert(binder, old); }
                            else { scope.remove(binder.as_str()); }
                        } else {
                            out.extend(self.build(f, &a.body, vec![n], locals, scope, graph));
                        }
                    }
                    graph.join(out)
                }
                Stmt::Return(_) => Vec::new(),
                _ => vec![n],
            };
        }
        self.visible.borrow_mut().truncate(visible);
        for (name, old) in scope_undo.into_iter().rev() {
            match old {
                Some(bit) => { scope.insert(name, bit); }
                None => { scope.remove(name); }
            }
        }
        for (name, old) in locals_undo.into_iter().rev() {
            match old {
                Some(roots) => { locals.insert(name, roots); }
                None => { locals.remove(&name); }
            }
        }
        incoming
    }
}

pub(crate) fn check(prog: &Program, place_functions: &PlaceFunctions) -> Res<()> {
    let has_views = first_borrow(prog).is_some();
    if !has_views && place_functions.is_empty() { return Ok(()); }
    let analysis = Analysis {
        lists: RefCell::default(),
        list_scope: RefCell::default(),
        visible: RefCell::default(),
        collecting: Cell::new(false),
        place_functions,
        functions: prog.functions.iter().map(|f| (f.name.as_str(), f)).collect(),
        records: prog.types.iter().filter_map(|d| match d {
            TypeDecl::Struct(s) if borrowing_struct(d) => Some((s.name.as_str(), s)),
            _ => None,
        }).collect(),
        structs: prog.types.iter().filter_map(|d| match d {
            TypeDecl::Struct(s) => Some(s.name.as_str()),
            _ => None,
        }).collect(),
    };
    for f in &prog.functions {
        if !has_views && !place_functions.contains_key(&f.name) { continue; }
        let mut graph = Graph::default();
        // R-6g9-6: parameters hold provenance, but have no local liveness bit.
        let mut locals: HashMap<String, Origin> = f.params.iter().filter_map(|p| {
            let read = !p.take && p.mode_pos.is_none();
            let ty = GType::from_key(&p.ty);
            (analysis.records.contains_key(ty.name.as_str()) || (read && analysis.structs.contains(ty.name.as_str())))
                .then(|| (p.name.clone(), Origin::Param))
        }).collect();
        let lists = view_lists(f, analysis.functions.contains_key("vec_join"))?;
        *analysis.lists.borrow_mut() = lists.iter().map(|name| (name.clone(), Roots::new())).collect();
        analysis.list_scope.borrow_mut().clear();
        let params = || f.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        if !lists.is_empty() {
            // R-6g9-10: an element read carries the list's provenance, which a later push can grow: repeat until the
            // union of the pushes is stable (each pass only adds sources; at most one pass per list plus one).
            analysis.collecting.set(true);
            for _ in 0..=lists.len() {
                let before = analysis.lists.borrow().clone();
                *analysis.visible.borrow_mut() = params();
                let mut scratch_locals = locals.iter().map(|(k, _)| (k.clone(), Origin::Param)).collect();
                analysis.build(f, &f.body, Vec::new(), &mut scratch_locals, &mut HashMap::new(), &mut Graph::default());
                if *analysis.lists.borrow() == before { break; }
            }
            analysis.collecting.set(false);
        }
        *analysis.visible.borrow_mut() = params();
        analysis.build(f, &f.body, Vec::new(), &mut locals, &mut HashMap::new(), &mut graph);
        graph.conflicts()?;
    }
    Ok(())
}

/// R-6g9-10: the view lists of `f`. A list is a local: it may only be pushed to (`vec_push(inout l, v)`), read
/// (`vec_get(l, i)`, `len(l)`), cleared or reserved; any other use of its name (a move, an argument, a return, a
/// store) would let it escape. Each list name is bound once in the function.
fn view_lists(f: &Function, user_join: bool) -> Res<Vec<String>> {
    let mut lists = Vec::new();
    let mut names: Vec<(String, i64, i64)> = Vec::new();
    crate::foreign_shim::walk_stmts(&f.body, &mut |node| {
        if let Node::Let(s) = node {
            if is_view_list(s) { lists.push(s.name.clone()); }
            names.push((s.name.clone(), s.line, s.col));
        }
    });
    if lists.is_empty() { return Ok(lists); }
    for list in &lists {
        if let Some((_, line, col)) = names.iter().filter(|(n, _, _)| n == list).nth(1) {
            return Err(signature(*line, *col));
        }
        if f.params.iter().any(|p| &p.name == list) {
            return Err(signature(f.line, f.col));
        }
    }
    let set: HashSet<&str> = lists.iter().map(String::as_str).collect();
    // a program's own `vec_join` keeps the name (R-6g9-16), so passing the list to it is an escape
    fn uses(e: &Expr, set: &HashSet<&str>, uj: bool) -> Res<()> {
        let escape = |line, col| error(line, col, "borrow_escape", "a Vec<ref str> list cannot leave its function's operations");
        let list = |a: &Expr, inout: bool| match (a, inout) {
            (Expr::InoutArg(x), true) => matches!(&*x.expr, Expr::Var(v) if set.contains(v.name.as_str())),
            (Expr::Var(v), false) => set.contains(v.name.as_str()),
            _ => false,
        };
        match e {
            Expr::Call(c) if matches!(c.name.as_str(), "vec_push" | "vec_clear" | "vec_reserve" | "vec_pop")
                && c.args.first().is_some_and(|a| list(a, true)) => c.args[1..].iter().try_for_each(|a| uses(a, set, uj)),
            Expr::Call(c) if (matches!(c.name.as_str(), "vec_get" | "len" | "vec_views_len") || c.name == "vec_join" && !uj)
                && c.args.first().is_some_and(|a| list(a, false)) => c.args[1..].iter().try_for_each(|a| uses(a, set, uj)),
            Expr::Var(v) if set.contains(v.name.as_str()) => Err(escape(v.line, v.col)),
            Expr::Move(m) if set.contains(m.name.as_str()) => Err(escape(m.line, m.col)),
            _ => crate::refine::kids(e).into_iter().try_for_each(|k| uses(k, set, uj)),
        }
    }
    fn block(body: &[Stmt], set: &HashSet<&str>, uj: bool) -> Res<()> {
        for st in body {
            let (mut exprs, mut writes, mut blocks) = (Vec::new(), Vec::new(), Vec::new());
            walk_statement(st, &mut |e| exprs.push(e), &mut |name, _, line, col| writes.push((name, line, col)),
                &mut |b| blocks.push(b));
            if let Some((_, line, col)) = writes.into_iter().find(|(name, _, _)| set.contains(name)) {
                return Err(error(line, col, "borrow_escape", "a Vec<ref str> list cannot be reassigned"));
            }
            exprs.into_iter().try_for_each(|e| uses(e, set, uj))?;
            blocks.into_iter().try_for_each(|b| block(b, set, uj))?;
        }
        Ok(())
    }
    block(&f.body, &set, user_join)?;
    Ok(lists)
}

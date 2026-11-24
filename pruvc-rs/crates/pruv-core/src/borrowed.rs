//! LB2 ownership restrictions. The checker and verifier use the underlying value types.
use crate::ast::*;
use crate::check::{expr_ty, field_place, mentions, paths_overlap, place_root};
use crate::diagnostic::PruvError;
use crate::foreign_shim::{walk_expr, Node};
use crate::gtype::GType;
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

fn is_view_base(ty: &str) -> bool {
    matches!(ty, "str" | "array_u8")
}

fn annotation(ty: &mut String, line: i64, col: i64, permitted: bool, structs: &HashSet<String>) -> Res<bool> {
    let parsed = GType::from_key(ty);
    if parsed.name == "@ref" && permitted && is_view_base(&parsed.args[0].to_key()) {
        *ty = parsed.args[0].to_key();
        Ok(true)
    } else if contains_borrow(&parsed, structs) {
        Err(signature(line, col))
    } else {
        Ok(false)
    }
}

fn prepare_block(body: &mut [Stmt], permitted: bool, structs: &HashSet<String>) -> Res<()> {
    for st in body {
        match st {
            Stmt::Let(s) if !s.is_place() => {
                let allow = permitted && !s.mutable;
                if !(allow && structs.contains(&s.ty)) {
                    s.borrowed = annotation(&mut s.ty, s.line, s.col, allow, structs)?;
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
                        f.borrowed = annotation(&mut f.ty, f.line, f.col, s.tparams.is_empty(), &structs)?;
                    }
                }
            }
            TypeDecl::Enum(e) => {
                for v in &mut e.variants {
                    for f in &mut v.fields {
                        annotation(&mut f.ty, f.line, f.col, false, &structs)?;
                    }
                }
            }
        }
    }
    for (i, f) in prog.functions.iter_mut().enumerate() {
        let permitted = f.tparams.is_empty() && f.foreign.is_none() && !hosts.get(i).copied().unwrap_or(false);
        for p in &mut f.params {
            if !(permitted && structs.contains(&p.ty) && !p.take && p.mode_pos.is_none()) {
                annotation(&mut p.ty, f.line, f.col, false, &structs)?;
            }
        }
        if !(permitted && structs.contains(&f.ret)) {
            f.borrowed_result = annotation(&mut f.ret, f.line, f.col, permitted, &structs)?;
        }
        let borrowed_result = f.borrowed_result || structs.contains(&f.ret);
        match &f.borrow_from {
            Some(names) if borrowed_result && !names.is_empty() => {
                for (i, name) in names.iter().enumerate() {
                    if names[..i].contains(name) {
                        return Err(signature(f.line, f.col));
                    }
                    let Some(p) = f.params.iter().find(|p| p.name == *name) else {
                        return Err(signature(f.line, f.col));
                    };
                    if p.take || !(is_view_base(&p.ty) || (structs.contains(&p.ty) && p.mode_pos.is_none()))
                        || (p.mode_pos.is_some() && f.ensures.iter().any(|e| mentions(e, name)))
                    {
                        return Err(signature(f.line, f.col));
                    }
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
            if let Node::Let(s) = node {
                if s.borrowed {
                    position.get_or_insert((s.line, s.col));
                }
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
        Stmt::Return(s) => s.value.iter().for_each(expr),
        Stmt::Assert(s) => expr(&s.cond),
        Stmt::ExprStmt(s) => expr(&s.expr),
        Stmt::Par(s) => s.calls.iter().for_each(expr),
    }
}

/// The root accesses of `st` itself: a compound statement contributes its head (condition, contracts, place,
/// scrutinee); its nested blocks are their own statements in the liveness graph.
fn own_accesses(st: &Stmt, visit: &mut impl FnMut(&str, &[String], Access, i64, i64)) {
    let mut uses = Vec::new();
    let mut writes = Vec::new();
    walk_statement(st, &mut |e| uses.push(e), &mut |n, path, l, c| writes.push((n, path, l, c)), &mut |_| {});
    for (name, path, line, col) in writes {
        visit(name, path, Access::Write, line, col);
    }
    let place_binding = matches!(st, Stmt::Let(s) if s.is_place());
    for e in uses {
        expr_accesses(e, &mut |name, path, access, line, col| {
            let access = if place_binding { Access::Read } else { access };
            visit(name, path, access, line, col);
        });
    }
}

fn expr_accesses(e: &Expr, visit: &mut impl FnMut(&str, &[String], Access, i64, i64)) {
    match e {
        Expr::Move(m) => visit(&m.name, &m.fields, Access::Write, m.line, m.col),
        Expr::InoutArg(arg) => {
            let (root, path) = field_place(&arg.expr);
            if let Expr::Var(v) = root {
                visit(&v.name, &path, Access::Write, arg.line, arg.col);
            }
            expr_accesses(&arg.expr, visit);
        }
        _ => {
            let (root, path) = field_place(e);
            if let Expr::Var(v) = root {
                visit(&v.name, &path, Access::Read, v.line, v.col);
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
    uses: Vec<(String, Vec<String>, Access, i64, i64)>,
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
            for (name, path, access, line, col) in &point.uses {
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
                    return Err(error(*line, *col, "borrow_conflict", "use conflicts with a live borrowed view"));
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
    Place(Source),
}

struct Analysis<'a> {
    place_functions: &'a PlaceFunctions,
    functions: HashMap<&'a str, &'a Function>,
    records: HashMap<&'a str, &'a StructDecl>,
}

impl Analysis<'_> {
    fn ref_field(&self, base: &Expr, name: &str) -> bool {
        expr_ty(base).as_deref().and_then(|ty| self.records.get(ty))
            .is_some_and(|s| s.fields.iter().any(|f| f.name == name && (f.borrowed || self.records.contains_key(f.ty.as_str()))))
    }

    fn is_view(&self, e: &Expr, locals: &HashMap<String, Origin>) -> bool {
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
                None => {
                    merge(&mut roots, Source::direct(root), Access::Read);
                    // An owned field of a borrowing-struct parameter lives in the caller's record, which the
                    // Rust signature `&Doc<'a>` does not lend to the result.
                    if matches!(e, Expr::FieldAccess(x) if !self.ref_field(&x.base, &x.field))
                        && expr_ty(field_place(e).0).as_deref().is_some_and(|ty| self.records.contains_key(ty))
                    {
                        merge(&mut roots, Source::direct(&format!("{root}.storage")), Access::Read);
                    }
                }
            }
        } else {
            match e {
                Expr::StrLit(_) => {}
                Expr::Call(c) if c.name == "subview" => return self.provenance(&c.args[0], locals),
                Expr::Call(c) => {
                    let Some(f) = self.functions.get(c.name.as_str()) else {
                        return Err(signature(e.line(), e.col()));
                    };
                    let Some(from) = &f.borrow_from else {
                        return Err(signature(e.line(), e.col()));
                    };
                    for (p, a) in f.params.iter().zip(&c.args) {
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
    ) -> Res<()> {
        let mut expressions = Vec::new();
        walk_statement(st, &mut |e| expressions.push(e), &mut |_, _, _, _| {}, &mut |_| {});
        for e in expressions {
            let mut failure = None;
            walk_expr(e, &mut |node| {
                let Node::Expr(e) = node else { return };
                if failure.is_some() { return; }
                let result = match e {
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
                    // R-6g9-1: an enum (or Option / Result) payload owns its value, so it cannot hold a view
                    Expr::EnumCtor(x) => x.args.iter().try_for_each(|a| self.owned(a, locals)),
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
            Stmt::Let(s) if self.borrow_let(s) => {
                let roots = self.provenance(&s.value, locals)?;
                undo.push((s.name.clone(), locals.insert(s.name.clone(), Origin::View(roots))));
            }
            Stmt::Let(s) => self.owned(&s.value, locals)?,
            Stmt::Assign(s) => self.owned(&s.value, locals)?,
            Stmt::FieldAssign(s) => self.owned(&s.value, locals)?,
            Stmt::Return(s) => if let Some(value) = &s.value {
                if let Some(from) = &f.borrow_from {
                    let roots = self.provenance(value, locals)?;
                    if roots.keys().any(|source| !from.contains(&source.root)) {
                        return Err(error(s.line, s.col, "borrow_escape", "returned value borrows outside its from-list"));
                    }
                } else {
                    self.owned(value, locals)?;
                }
            },
            _ => {}
        }
        Ok(())
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
        for st in body {
            let n = graph.points.len();
            for p in incoming {
                graph.points[p].succ.push(n);
            }
            let mut point = Point { succ: Vec::new(), gen: Vec::new(), kill: None, uses: Vec::new(), site: st.position(), own_move: None };
            own_accesses(st, &mut |name, path, access, line, col| {
                if let Some(&b) = scope.get(name) {
                    point.gen.push(b);
                }
                point.uses.push((name.to_owned(), path.to_vec(), access, line, col));
            });
            if graph.failure.is_none() {
                if let Err(e) = self.statement(f, st, locals, &mut locals_undo) {
                    graph.failure = Some((n, e));
                }
            }
            if let Stmt::Let(s) = st {
                if s.is_place() {
                    let (root, footprint) = &self.place_functions[&f.name][&(s.line, s.col)];
                    point.uses.insert(0, (root.clone(), footprint.clone(), Access::Write, s.line, s.col));
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
                    // A value that moves a root the new view borrows (`Doc { src: x, scratch: move x }`) conflicts
                    // with the view itself; like the other conflicts it is reported only where reachable.
                    let roots = graph.borrows.last().unwrap();
                    walk_expr(&s.value, &mut |node| {
                        if let Node::Expr(Expr::Move(m)) = node {
                            if point.own_move.is_none() && roots.keys().any(|source| source.root == m.name) {
                                point.own_move = Some((m.line, m.col));
                            }
                        }
                    });
                }
            }
            graph.points.push(point);
            incoming = match st {
                Stmt::If(s) => {
                    let mut out = self.build(f, &s.then, vec![n], locals, scope, graph);
                    out.extend(self.build(f, &s.els, vec![n], locals, scope, graph));
                    graph.join(out)
                }
                Stmt::While(While { body, .. }) | Stmt::ForMap(ForMap { body, .. }) => {
                    for p in self.build(f, body, vec![n], locals, scope, graph) {
                        graph.points[p].succ.push(n);
                    }
                    vec![n]
                }
                Stmt::Match(s) => {
                    let mut out = Vec::new();
                    for a in &s.arms {
                        out.extend(self.build(f, &a.body, vec![n], locals, scope, graph));
                    }
                    graph.join(out)
                }
                Stmt::Return(_) => Vec::new(),
                _ => vec![n],
            };
        }
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
        place_functions,
        functions: prog.functions.iter().map(|f| (f.name.as_str(), f)).collect(),
        records: prog.types.iter().filter_map(|d| match d {
            TypeDecl::Struct(s) if borrowing_struct(d) => Some((s.name.as_str(), s)),
            _ => None,
        }).collect(),
    };
    for f in &prog.functions {
        if !has_views && !place_functions.contains_key(&f.name) { continue; }
        let mut graph = Graph::default();
        analysis.build(f, &f.body, Vec::new(), &mut HashMap::new(), &mut HashMap::new(), &mut graph);
        graph.conflicts()?;
    }
    Ok(())
}

//! LB2 ownership restrictions. The checker and verifier use the underlying value types.
use crate::ast::*;
use crate::check::{expr_ty, field_place, mentions, place_root};
use crate::diagnostic::PruvError;
use crate::foreign_shim::{walk_expr, Node};
use crate::gtype::GType;
use std::collections::{BTreeMap, HashMap, HashSet};

type Res<T> = Result<T, PruvError>;

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
            Stmt::Let(s) => {
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

type Roots = BTreeMap<String, Access>;

fn merge(roots: &mut Roots, root: &str, access: Access) {
    let held = roots.entry(root.to_owned()).or_insert(access);
    *held = (*held).max(access);
}

fn walk_statement<'a>(
    st: &'a Stmt,
    expr: &mut impl FnMut(&'a Expr),
    write: &mut impl FnMut(&'a str, i64, i64),
    block: &mut impl FnMut(&'a [Stmt]),
) {
    match st {
        Stmt::Let(s) => expr(&s.value),
        Stmt::Assign(s) => { write(&s.name, s.line, s.col); expr(&s.value); }
        Stmt::IndexAssign(s) => { write(&s.name, s.line, s.col); expr(&s.index); expr(&s.value); }
        Stmt::FieldAssign(s) => { write(&s.name, s.line, s.col); expr(&s.value); }
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

fn accesses(st: &Stmt, visit: &mut impl FnMut(&str, Access, i64, i64)) {
    let mut uses = Vec::new();
    let mut writes = Vec::new();
    let mut blocks = Vec::new();
    walk_statement(st, &mut |e| uses.push(e), &mut |n, l, c| writes.push((n, l, c)), &mut |b| blocks.push(b));
    for (name, line, col) in writes {
        visit(name, Access::Write, line, col);
    }
    for e in uses {
        walk_expr(e, &mut |n| {
            let Node::Expr(e) = n else { return };
            let access = match e {
                Expr::Var(v) => Some((v.name.as_str(), Access::Read)),
                Expr::Move(m) => Some((m.name.as_str(), Access::Write)),
                Expr::InoutArg(a) => place_root(&a.expr).map(|name| (name, Access::Write)),
                _ => None,
            };
            if let Some((name, mode)) = access {
                visit(name, mode, e.line(), e.col());
            }
        });
    }
    for body in blocks {
        for st in body {
            accesses(st, visit);
        }
    }
}

struct Analysis<'a> {
    functions: HashMap<&'a str, &'a Function>,
    records: HashMap<&'a str, &'a StructDecl>,
}

impl Analysis<'_> {
    fn ref_field(&self, base: &Expr, name: &str) -> bool {
        expr_ty(base).as_deref().and_then(|ty| self.records.get(ty))
            .is_some_and(|s| s.fields.iter().any(|f| f.name == name && (f.borrowed || self.records.contains_key(f.ty.as_str()))))
    }

    fn is_view(&self, e: &Expr, locals: &HashMap<String, Roots>) -> bool {
        match e {
            Expr::Var(x) => locals.contains_key(&x.name)
                && expr_ty(e).as_deref().is_some_and(is_view_base),
            Expr::Call(x) => self.functions.get(x.name.as_str()).is_some_and(|f| f.borrowed_result),
            Expr::FieldAccess(x) => self.ref_field(&x.base, &x.field)
                && expr_ty(e).as_deref().is_some_and(is_view_base),
            _ => false,
        }
    }

    fn provenance(&self, e: &Expr, locals: &HashMap<String, Roots>) -> Res<Roots> {
        let mut roots = Roots::new();
        if let Some(root) = place_root(e) {
            if let Some(origin) = locals.get(root) {
                // An owned field (not `ref`, not a borrowing struct: `ref_field` covers those) lives in the
                // record's storage and carries nothing the record borrows.
                if matches!(e, Expr::FieldAccess(x) if !self.ref_field(&x.base, &x.field)) {
                    merge(&mut roots, root, Access::Read);
                } else {
                    roots = origin.clone();
                }
            } else {
                merge(&mut roots, root, Access::Read);
                // An owned field of a borrowing-struct parameter lives in the caller's record, which the
                // Rust signature `&Doc<'a>` does not lend to the result.
                if matches!(e, Expr::FieldAccess(x) if !self.ref_field(&x.base, &x.field))
                    && expr_ty(field_place(e).0).as_deref().is_some_and(|ty| self.records.contains_key(ty))
                {
                    merge(&mut roots, &format!("{root}.storage"), Access::Read);
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
                            for (name, access) in self.provenance(value, locals)? {
                                merge(&mut roots, &name, access.max(mode));
                            }
                        }
                    }
                }
                Expr::StructLit(s) => for f in &s.fields {
                    if self.ref_field(e, &f.name) {
                        for (name, access) in self.provenance(&f.value, locals)? {
                            merge(&mut roots, &name, access);
                        }
                    }
                },
                Expr::FieldAccess(f) if self.ref_field(&f.base, &f.field) => return self.provenance(&f.base, locals),
                _ => return Err(signature(e.line(), e.col())),
            }
        }
        Ok(roots)
    }

    fn owned(&self, e: &Expr, locals: &HashMap<String, Roots>) -> Res<()> {
        if self.is_view(e, locals) {
            return Err(error(e.line(), e.col(), "array_alias", "use copy to obtain an owned value from a borrowed view"));
        }
        Ok(())
    }

    fn block(&self, f: &Function, body: &[Stmt], mut locals: HashMap<String, Roots>) -> Res<()> {
        let mut live = Vec::<(usize, Roots)>::new();
        for (index, st) in body.iter().enumerate() {
            live.retain(|(end, _)| *end >= index);
            let mut conflict = None;
            if !live.is_empty() {
                accesses(st, &mut |name, access, line, col| {
                    if live.iter().any(|(_, roots)| roots.get(name).is_some_and(|held| {
                        *held == Access::Write || access == Access::Write
                    })) {
                        conflict.get_or_insert(error(line, col, "borrow_conflict", "use conflicts with a live borrowed view"));
                    }
                });
            }
            if let Some(e) = conflict {
                return Err(e);
            }

            let mut expressions = Vec::new();
            let mut blocks = Vec::new();
            walk_statement(st, &mut |e| expressions.push(e), &mut |_, _, _| {}, &mut |b| blocks.push(b));
            for e in expressions {
                let mut failure = None;
                walk_expr(e, &mut |node| {
                    let Node::Expr(e) = node else { return };
                    if failure.is_some() { return; }
                    let result = match e {
                        Expr::Call(c) => {
                            if let Some(callee) = self.functions.get(c.name.as_str()) {
                                if callee.borrow_from.is_some() {
                                    if let Err(error) = self.provenance(e, &locals) {
                                        failure = Some(error);
                                        return;
                                    }
                                }
                                c.args.iter().zip(&callee.params)
                                    .filter(|(_, p)| p.take)
                                    .try_for_each(|(a, _)| self.owned(a, &locals))
                            } else {
                                Ok(())
                            }
                        }
                        Expr::StructLit(s) => s.fields.iter().try_for_each(|field| {
                            if self.ref_field(e, &field.name) { self.provenance(&field.value, &locals).map(|_| ()) }
                            else { self.owned(&field.value, &locals) }
                        }),
                        // R-6g9-1: an enum (or Option / Result) payload owns its value, so it cannot hold a view
                        Expr::EnumCtor(x) => x.args.iter().try_for_each(|a| self.owned(a, &locals)),
                        Expr::Ctor(x) => x.arg.iter().try_for_each(|a| self.owned(a, &locals)),
                        _ => Ok(()),
                    };
                    failure = result.err();
                    if matches!(st, Stmt::Par(_)) && (self.is_view(e, &locals)
                        || expr_ty(e).as_deref().is_some_and(|ty| self.records.contains_key(ty))) {
                        failure = Some(signature(e.line(), e.col()));
                    }
                });
                if let Some(e) = failure {
                    return Err(e);
                }
            }

            match st {
                Stmt::Let(s) if s.borrowed || self.records.contains_key(s.ty.as_str()) => {
                    let roots = self.provenance(&s.value, &locals)?;
                    if !roots.is_empty() {
                        let end = body[index + 1..].iter().rposition(|st| {
                            let mut used = false;
                            accesses(st, &mut |name, _, _, _| used |= name == s.name);
                            used
                        });
                        if let Some(end) = end {
                            live.push((index + 1 + end, roots.clone()));
                        }
                    }
                    locals.insert(s.name.clone(), roots);
                }
                Stmt::Let(s) => self.owned(&s.value, &locals)?,
                Stmt::Assign(s) => self.owned(&s.value, &locals)?,
                Stmt::FieldAssign(s) => self.owned(&s.value, &locals)?,
                Stmt::Return(s) => if let Some(value) = &s.value {
                    if let Some(from) = &f.borrow_from {
                        let roots = self.provenance(value, &locals)?;
                        if roots.keys().any(|name| !from.contains(name)) {
                            return Err(error(s.line, s.col, "borrow_escape", "returned value borrows outside its from-list"));
                        }
                    } else {
                        self.owned(value, &locals)?;
                    }
                },
                _ => {}
            }
            for nested in blocks {
                self.block(f, nested, locals.clone())?;
            }
        }
        Ok(())
    }
}

pub(crate) fn check(prog: &Program) -> Res<()> {
    if first_borrow(prog).is_none() {
        return Ok(());
    }
    let analysis = Analysis {
        functions: prog.functions.iter().map(|f| (f.name.as_str(), f)).collect(),
        records: prog.types.iter().filter_map(|d| match d {
            TypeDecl::Struct(s) if borrowing_struct(d) => Some((s.name.as_str(), s)),
            _ => None,
        }).collect(),
    };
    for f in &prog.functions {
        analysis.block(f, &f.body, HashMap::new())?;
    }
    Ok(())
}

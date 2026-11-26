//! P9b (SPEC 6x): impl blocks are parsed as free functions `Owner__name`; this pass validates them and resolves every
//! method call `recv.m(args)` (the parser's `@method|m(recv, args)`) statically from the declared type of the
//! receiver, before any other check reads the program. The call becomes `Owner__m(recv', args)` with recv' = `recv`
//! (a read receiver), `inout recv` (inout) or `move recv` (take), so every later rule (argument modes, aliasing,
//! moves, effects, contracts and the proof) is the free-function rule.

use crate::ast::*;
use crate::diagnostic::PruvError;
use crate::gtype::GType;
use crate::pyrepr::repr_str;
use std::collections::HashMap;

fn terr(line: i64, col: i64, code: &str, msg: String, fix: &str) -> PruvError {
    PruvError::new("type", line, col, code, msg, fix)
}

/// A binding's declared type key, or None when the pass does not know it (match binders, `for` element binders,
/// quantifier variables): a method call on such a receiver is rejected (type/method_receiver).
type Scopes = Vec<HashMap<String, Option<String>>>;

struct Resolver<'a> {
    methods: HashMap<(String, String), (String, Receiver)>,
    /// struct name -> field name -> field type key (non-generic structs only)
    fields: HashMap<String, HashMap<String, String>>,
    types: &'a HashMap<String, bool>,
}

pub fn resolve(prog: &mut Program) -> Result<(), PruvError> {
    // owner name -> generic?
    let mut types: HashMap<String, bool> = HashMap::new();
    let mut fields: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut variants: HashMap<String, Vec<String>> = HashMap::new();
    for d in &prog.types {
        match d {
            TypeDecl::Struct(s) => {
                types.insert(s.name.clone(), !s.tparams.is_empty());
                if s.tparams.is_empty() {
                    fields.insert(s.name.clone(), s.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect());
                }
            }
            TypeDecl::Enum(e) => {
                types.insert(e.name.clone(), !e.tparams.is_empty());
                variants.insert(e.name.clone(), e.variants.iter().map(|v| v.name.clone()).collect());
            }
        }
    }
    for (owner, line, col) in &prog.impls {
        match types.get(owner) {
            None => {
                return Err(terr(*line, *col, "impl_type",
                    format!("impl of {}, which is not a struct or enum of this program", repr_str(owner)),
                    "write the impl block for a struct or enum declared in this file"));
            }
            Some(true) => {
                return Err(terr(*line, *col, "impl_type",
                    format!("impl blocks of the generic type {} are not available yet (parity row P9b-2)", repr_str(owner)),
                    "write free generic functions for this type"));
            }
            Some(false) => {}
        }
    }
    let mut methods: HashMap<(String, String), (String, Receiver)> = HashMap::new();
    for m in &prog.methods {
        if variants.get(&m.owner).is_some_and(|vs| vs.contains(&m.name)) {
            return Err(terr(m.line, m.col, "method_name_clash",
                format!("{}::{} is both a variant and an impl function", m.owner, m.name),
                "rename the function"));
        }
        if methods.insert((m.owner.clone(), m.name.clone()), (m.func.clone(), m.recv)).is_some() {
            return Err(terr(m.line, m.col, "dup_method",
                format!("{} has two impl functions named {}", repr_str(&m.owner), repr_str(&m.name)),
                "rename one of them"));
        }
    }
    let r = Resolver { methods, fields, types: &types };
    for f in prog.functions.iter_mut() {
        let mut scopes: Scopes = vec![f.params.iter().map(|p| (p.name.clone(), Some(p.ty.clone()))).collect()];
        for e in f.requires.iter_mut().chain(f.decreases.iter_mut()).chain(f.cost.iter_mut()) {
            r.expr(e, &mut scopes)?;
        }
        scopes.push(HashMap::from([("result".to_string(), Some(f.ret.clone()))]));
        for e in f.ensures.iter_mut() {
            r.expr(e, &mut scopes)?;
        }
        scopes.pop();
        r.block(&mut f.body, &mut scopes)?;
    }
    Ok(())
}

impl Resolver<'_> {
    fn block(&self, stmts: &mut [Stmt], scopes: &mut Scopes) -> Result<(), PruvError> {
        scopes.push(HashMap::new());
        for s in stmts.iter_mut() {
            self.stmt(s, scopes)?;
        }
        scopes.pop();
        Ok(())
    }

    fn stmt(&self, s: &mut Stmt, scopes: &mut Scopes) -> Result<(), PruvError> {
        match s {
            Stmt::Let(l) => {
                self.expr(&mut l.value, scopes)?;
                let known = !(l.ty.is_empty() || l.ty == "_");
                scopes.last_mut().unwrap().insert(l.name.clone(), known.then(|| l.ty.clone()));
            }
            Stmt::Assign(a) => self.expr(&mut a.value, scopes)?,
            Stmt::IndexAssign(a) => {
                self.expr(&mut a.index, scopes)?;
                self.expr(&mut a.value, scopes)?;
            }
            Stmt::FieldAssign(a) => self.expr(&mut a.value, scopes)?,
            Stmt::If(i) => {
                self.expr(&mut i.cond, scopes)?;
                self.block(&mut i.then, scopes)?;
                self.block(&mut i.els, scopes)?;
            }
            Stmt::While(w) => {
                self.expr(&mut w.cond, scopes)?;
                for e in w.invariants.iter_mut() {
                    self.expr(e, scopes)?;
                }
                self.expr(&mut w.decreases, scopes)?;
                self.block(&mut w.body, scopes)?;
            }
            Stmt::ForMap(m) => {
                self.expr(&mut m.place, scopes)?;
                scopes.push(HashMap::from([(m.key.clone(), None), (m.val.clone(), None)]));
                for e in m.invariants.iter_mut() {
                    self.expr(e, scopes)?;
                }
                self.block(&mut m.body, scopes)?;
                scopes.pop();
            }
            Stmt::Return(r) => {
                if let Some(v) = r.value.as_mut() {
                    self.expr(v, scopes)?;
                }
            }
            Stmt::Assert(a) => self.expr(&mut a.cond, scopes)?,
            Stmt::ExprStmt(e) => self.expr(&mut e.expr, scopes)?,
            Stmt::Match(m) => {
                self.expr(&mut m.scrutinee, scopes)?;
                for arm in m.arms.iter_mut() {
                    scopes.push(arm.binders.iter().map(|b| (b.clone(), None)).collect());
                    self.block(&mut arm.body, scopes)?;
                    scopes.pop();
                }
                self.block(&mut m.els, scopes)?;
            }
            Stmt::Par(p) => {
                for e in p.calls.iter_mut() {
                    self.expr(e, scopes)?;
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
        }
        Ok(())
    }

    fn lookup(scopes: &Scopes, name: &str) -> Option<Option<String>> {
        scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }

    /// The static type key of a receiver place `v.f.g`, or the error for a non-place / unknown receiver.
    fn place_type(&self, e: &Expr, scopes: &Scopes, line: i64, col: i64, m: &str) -> Result<String, PruvError> {
        let bad = || terr(line, col, "method_receiver",
            format!("the receiver of .{}(..) must be a variable, a parameter, `self` or a field of one, declared with its type", m),
            "bind the value with `let x: T = ..;` first (parity row P9b-2)");
        match e {
            Expr::Var(v) => match Self::lookup(scopes, &v.name) {
                Some(Some(t)) => Ok(t),
                _ => Err(bad()),
            },
            Expr::FieldAccess(fa) => {
                let base = self.place_type(&fa.base, scopes, line, col, m)?;
                self.fields.get(&base).and_then(|fs| fs.get(&fa.field)).cloned().ok_or_else(bad)
            }
            _ => Err(bad()),
        }
    }

    fn expr(&self, e: &mut Expr, scopes: &mut Scopes) -> Result<(), PruvError> {
        match e {
            Expr::Call(c) => {
                for a in c.args.iter_mut() {
                    self.expr(a, scopes)?;
                }
                if let Some(m) = c.name.strip_prefix(METHOD_CALL) {
                    let m = m.to_string();
                    let ty = self.place_type(&c.args[0], scopes, c.line, c.col, &m)?;
                    let owner = GType::from_key(&ty);
                    let found = if owner.args.is_empty() && self.types.contains_key(&owner.name) {
                        self.methods.get(&(owner.name.clone(), m.clone()))
                    } else {
                        None
                    };
                    let Some((func, recv)) = found else {
                        return Err(terr(c.line, c.col, "unknown_method",
                            format!("type {} has no method {}", repr_str(&ty), repr_str(&m)),
                            "declare it in an `impl` block of the type, or call a free function"));
                    };
                    if *recv == Receiver::None {
                        return Err(terr(c.line, c.col, "unknown_method",
                            format!("{}::{} has no receiver, so it is not a method", owner.name, m),
                            "call it as an associated function: Owner::f(..)"));
                    }
                    let r = c.args[0].clone();
                    let (line, col) = (r.line(), r.col());
                    c.args[0] = match recv {
                        Receiver::Inout => Expr::InoutArg(InoutArg { line, col, expr: Box::new(r), ty: None, post_refine: Vec::new() }),
                        Receiver::Take => {
                            let mut path = Vec::new();
                            let mut cur = &r;
                            while let Expr::FieldAccess(fa) = cur {
                                path.push(fa.field.clone());
                                cur = &fa.base;
                            }
                            let Expr::Var(v) = cur else { unreachable!("place_type accepted the receiver") };
                            path.reverse();
                            Expr::Move(Move { line, col, name: v.name.clone(), fields: path, ty: None })
                        }
                        _ => r,
                    };
                    c.name = func.clone();
                }
            }
            Expr::Quant(q) => {
                self.expr(&mut q.lo, scopes)?;
                self.expr(&mut q.hi, scopes)?;
                scopes.push(HashMap::from([(q.var.clone(), None)]));
                self.expr(&mut q.body, scopes)?;
                for t in q.triggers.iter_mut() {
                    self.expr(t, scopes)?;
                }
                scopes.pop();
            }
            Expr::InoutArg(x) => self.expr(&mut x.expr, scopes)?,
            Expr::Unary(x) => self.expr(&mut x.operand, scopes)?,
            Expr::Binary(x) => {
                self.expr(&mut x.left, scopes)?;
                self.expr(&mut x.right, scopes)?;
            }
            Expr::Index(x) => {
                self.expr(&mut x.array, scopes)?;
                self.expr(&mut x.index, scopes)?;
            }
            Expr::Ctor(x) => {
                if let Some(a) = x.arg.as_mut() {
                    self.expr(a, scopes)?;
                }
            }
            Expr::StructLit(x) => {
                for f in x.fields.iter_mut() {
                    self.expr(&mut f.value, scopes)?;
                }
            }
            Expr::EnumCtor(x) => {
                for a in x.args.iter_mut() {
                    self.expr(a, scopes)?;
                }
            }
            Expr::FieldAccess(x) => self.expr(&mut x.base, scopes)?,
            Expr::IsVariant(x) => self.expr(&mut x.value, scopes)?,
            Expr::VariantField(x) => self.expr(&mut x.value, scopes)?,
            Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) | Expr::Move(_) => {}
        }
        Ok(())
    }
}

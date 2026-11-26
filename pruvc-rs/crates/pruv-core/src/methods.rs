//! P9b (SPEC 6x): impl blocks are parsed as free functions `Owner__name`; this pass validates them and resolves every
//! method call `recv.m(args)` (the parser's `@method|m(recv, args)`) statically from the declared type of the
//! receiver, before any other check reads the program. The call becomes `Owner__m(recv', args)` with recv' = `recv`
//! (a read receiver), `inout recv` (inout) or `move recv` (take), so every later rule (argument modes, aliasing,
//! moves, effects, contracts and the proof) is the free-function rule.

use crate::ast::*;
use crate::diagnostic::PruvError;
use crate::gtype::{bind_args, decl_owner, fn_owner, substitute, tuple_components, GType};
use crate::pyrepr::repr_str;
use std::collections::HashMap;

fn terr(line: i64, col: i64, code: &str, msg: String, fix: &str) -> PruvError {
    PruvError::new("type", line, col, code, msg, fix)
}

/// A binding's type key, or None when the pass does not know it (quantifier variables, `for` binders over a `Vec`, a `let`
/// without a type whose value has no static type here): a method call on such a receiver is rejected
/// (type/method_receiver).
type Scopes = Vec<HashMap<String, Option<String>>>;

struct Resolver<'a> {
    /// (owner, method name) -> (function, receiver, `pub`)
    methods: HashMap<(String, String), (String, Receiver, bool)>,
    /// struct name -> field name -> field type key (P9b-3: also of generic structs, written with the declaration's
    /// parameters and substituted by the receiver's type arguments)
    fields: HashMap<String, HashMap<String, String>>,
    /// P9b-3: number of type parameters of each struct or enum
    arity: HashMap<String, usize>,
    /// P9b-3: (number of type parameters, result type key) of each function
    types: &'a HashMap<String, bool>,
    fn_ret: HashMap<String, (usize, String)>,
    /// P9b-2: (enum, variant) -> payload field type keys (types match binders; P9b-3: also of generic enums)
    variant_fields: HashMap<(String, String), Vec<String>>,
    /// P9b-2: module of the function being resolved (`crate::a` of `crate::a::f`; empty in a single-file program)
    cur_mod: String,
}

/// `crate::a` of the identity `crate::a::f` (empty for a single-file name)
fn module_of(name: &str) -> &str {
    name.rfind("::").map_or("", |k| &name[..k])
}

pub fn resolve(prog: &mut Program) -> Result<(), PruvError> {
    crate::diagnostic::set_method_names(
        prog.methods.iter().map(|m| (m.func.clone(), format!("{}::{}", m.owner, m.name))).collect(),
    );
    // owner name -> generic?
    let mut types: HashMap<String, bool> = HashMap::new();
    let mut fields: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut variants: HashMap<String, Vec<String>> = HashMap::new();
    let mut variant_fields: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut arity: HashMap<String, usize> = HashMap::new();
    for d in &prog.types {
        match d {
            TypeDecl::Struct(s) => {
                types.insert(s.name.clone(), !s.tparams.is_empty());
                arity.insert(s.name.clone(), s.tparams.len());
                fields.insert(s.name.clone(), s.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect());
            }
            TypeDecl::Enum(e) => {
                types.insert(e.name.clone(), !e.tparams.is_empty());
                arity.insert(e.name.clone(), e.tparams.len());
                variants.insert(e.name.clone(), e.variants.iter().map(|v| v.name.clone()).collect());
                for v in &e.variants {
                    variant_fields.insert((e.name.clone(), v.name.clone()), v.fields.iter().map(|f| f.ty.clone()).collect());
                }
            }
        }
    }
    for (owner, line, col, n) in &prog.impls {
        if !types.contains_key(owner) {
            return Err(terr(*line, *col, "impl_type",
                format!("impl of {}, which is not a struct or enum of this program", repr_str(owner)),
                "write the impl block for a struct or enum declared in this file"));
        }
        // P9b-3: a generic impl block names every parameter of its type (`impl<T> Pair<T>` of `Pair<A, B>` is
        // wrong), also when the block is empty
        let want = arity.get(owner).copied().unwrap_or(0);
        if *n != want {
            return Err(terr(*line, *col, "impl_type",
                format!("the impl block of {} must name its {} type parameter(s)", repr_str(owner), want),
                "write `impl<T, ..> Name<T, ..> { .. }` with one parameter per type parameter of Name"));
        }
    }
    let mut methods: HashMap<(String, String), (String, Receiver, bool)> = HashMap::new();
    for m in &prog.methods {
        if variants.get(&m.owner).is_some_and(|vs| vs.contains(&m.name)) {
            return Err(terr(m.line, m.col, "method_name_clash",
                format!("{}::{} is both a variant and an impl function", m.owner, m.name),
                "rename the function"));
        }
        if methods.insert((m.owner.clone(), m.name.clone()), (m.func.clone(), m.recv, m.is_pub)).is_some() {
            return Err(terr(m.line, m.col, "dup_method",
                format!("{} has two impl functions named {}", repr_str(&m.owner), repr_str(&m.name)),
                "rename one of them"));
        }
    }
    let fn_ret: HashMap<String, (usize, String)> =
        prog.functions.iter().map(|f| (f.name.clone(), (f.tparams.len(), f.ret.clone()))).collect();
    let mut r = Resolver { methods, fields, arity, types: &types, fn_ret, variant_fields, cur_mod: String::new() };
    for f in prog.functions.iter_mut() {
        r.cur_mod = module_of(&f.name).to_string();
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
                // P9b-2: `let x = ..;` takes the static type of its value when this pass knows it
                let ty = if known { Some(l.ty.clone()) } else { self.value_type(&l.value, scopes) };
                scopes.last_mut().unwrap().insert(l.name.clone(), ty);
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
                if w.for_head.as_ref().is_some_and(|h| h.slice) {
                    // P9b-2: the parser binds the element of `for x in v` with the placeholder type `i64` (the
                    // checker sets the real one), so the binder takes the element type of `v[i]` here
                    scopes.push(HashMap::new());
                    for (k, s) in w.body.iter_mut().enumerate() {
                        match s {
                            Stmt::Let(l) if k == 0 => {
                                self.expr(&mut l.value, scopes)?;
                                let ty = self.value_type(&l.value, scopes);
                                scopes.last_mut().unwrap().insert(l.name.clone(), ty);
                            }
                            _ => self.stmt(s, scopes)?,
                        }
                    }
                    scopes.pop();
                } else {
                    self.block(&mut w.body, scopes)?;
                }
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
                let scrut = self.value_type(&m.scrutinee, scopes);
                for arm in m.arms.iter_mut() {
                    // P9b-2: a binder of a variant has the payload field's type; P9b-3: also of a generic enum, of
                    // `Option`/`Result`, and the names of nested and tuple patterns
                    let mut sc = HashMap::new();
                    self.bind_arm(scrut.as_deref(), &arm.ctor, &arm.binders, &arm.pats, &mut sc);
                    scopes.push(sc);
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

    /// P9b-3: the payload field type keys of variant `ctor` of a value of type `scrut` (a user enum, also generic,
    /// `Option` or `Result`).
    fn payload(&self, scrut: &str, ctor: &str) -> Option<Vec<String>> {
        let g = GType::from_key(scrut);
        match (g.name.as_str(), ctor) {
            ("option", "Some") => return Some(vec![g.args.first().map_or_else(|| "i64".to_string(), GType::to_key)]),
            ("result", "Ok") => return Some(vec![g.args.first().map_or_else(|| "i64".to_string(), GType::to_key)]),
            ("result", "Err") => return Some(vec![g.args.get(1).map_or_else(|| "i64".to_string(), GType::to_key)]),
            ("option" | "result", _) => return None,
            _ => {}
        }
        let fs = self.variant_fields.get(&(g.name.clone(), ctor.to_string()))?;
        Some(fs.iter().map(|f| self.inst(&g, f)).collect())
    }

    /// P9b-3: the type key `k`, written with the parameters of declaration `g.name`, at the arguments of `g`.
    fn inst(&self, g: &GType, k: &str) -> String {
        if g.args.is_empty() {
            return k.to_string();
        }
        match bind_args(&decl_owner(&g.name), self.arity.get(&g.name).copied().unwrap_or(0), &g.args) {
            Ok(m) => substitute(&GType::from_key(k), &m).to_key(),
            Err(_) => k.to_string(),
        }
    }

    /// P9b-3: types the binder slots of one arm (`ctor` of a value of type `scrut`): plain names, nested variant
    /// patterns (`SubPat` slots) and tuple patterns (`pats`, bound through their `pruv_tup_d<N>` temporary).
    fn bind_arm(&self, scrut: Option<&str>, ctor: &str, slots: &[String], pats: &[Option<TuplePat>], sc: &mut HashMap<String, Option<String>>) {
        let payload = scrut.and_then(|t| self.payload(t, ctor)).filter(|p| p.len() == slots.len());
        for (k, slot) in slots.iter().enumerate() {
            let ty = payload.as_ref().map(|p| p[k].clone());
            self.bind_sub(&SubPat::decode(slot), ty.clone(), sc);
            if let Some(Some(TuplePat::Tuple(_, _, ps))) = pats.get(k) {
                self.bind_tuple(ps, ty, sc);
            }
        }
    }

    fn bind_sub(&self, p: &SubPat, ty: Option<String>, sc: &mut HashMap<String, Option<String>>) {
        match p {
            SubPat::Bind(n) => {
                sc.insert(n.clone(), ty);
            }
            SubPat::Variant(_, ctor, subs) => {
                let payload = ty.and_then(|t| self.payload(&t, ctor)).filter(|p| p.len() == subs.len());
                for (k, q) in subs.iter().enumerate() {
                    self.bind_sub(q, payload.as_ref().map(|p| p[k].clone()), sc);
                }
            }
            SubPat::At(n, q) => {
                sc.insert(n.clone(), ty.clone());
                self.bind_sub(q, ty, sc);
            }
            SubPat::Wild | SubPat::Int(_) | SubPat::Bool(_) => {}
        }
    }

    fn bind_tuple(&self, ps: &[TuplePat], ty: Option<String>, sc: &mut HashMap<String, Option<String>>) {
        let comps = ty.and_then(|t| tuple_components(&GType::from_key(&t).name)).filter(|c| c.len() == ps.len());
        for (k, q) in ps.iter().enumerate() {
            let t = comps.as_ref().map(|c| c[k].clone());
            match q {
                TuplePat::Wild => {}
                TuplePat::Bind(_, _, n, _) => {
                    sc.insert(n.clone(), t);
                }
                TuplePat::Tuple(_, _, inner) => self.bind_tuple(inner, t, sc),
            }
        }
    }

    fn lookup(scopes: &Scopes, name: &str) -> Option<Option<String>> {
        scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }

    /// P9b-2: the static type key of a value as far as this pass knows it (before the checker): a binding, a field of
    /// a typed value, an element of a typed array, the result of a non-generic function (method calls inside are
    /// already resolved, so chains work), a struct literal or a variant of a non-generic type.
    fn value_type(&self, e: &Expr, scopes: &Scopes) -> Option<String> {
        match e {
            Expr::Var(v) => Self::lookup(scopes, &v.name).flatten(),
            Expr::FieldAccess(fa) => {
                let base = GType::from_key(&self.value_type(&fa.base, scopes)?);
                let f = self.fields.get(&base.name).and_then(|fs| fs.get(&fa.field))?;
                Some(self.inst(&base, f))
            }
            Expr::Index(x) => {
                // P9b-3: also an element of a `Vec<T>` (and so the binder of `for x in v` over a Vec)
                let a = GType::from_key(&self.value_type(&x.array, scopes)?);
                if a.name == "vec" || a.name == "Vec" { a.args.first().map(GType::to_key) } else { a.array_elem().map(|t| t.to_key()) }
            }
            // P9b-3: an element read `vec_get::<T>(v, i)` (the builtin, not a user function of that name)
            Expr::Call(c) if c.name == "vec_get" && !self.fn_ret.contains_key("vec_get") => {
                c.targs.as_ref().and_then(|t| t.first()).map(GType::to_key)
            }
            Expr::Call(c) if !c.name.starts_with('@') => {
                // P9b-3: also the result of a generic function at its written type arguments
                let (n, ret) = self.fn_ret.get(&c.name)?;
                if *n == 0 {
                    return Some(ret.clone());
                }
                let m = bind_args(&fn_owner(&c.name), *n, c.targs.as_deref()?).ok()?;
                Some(substitute(&GType::from_key(ret), &m).to_key())
            }
            Expr::StructLit(x) if self.types.contains_key(&x.name) => match (&x.targs, self.types.get(&x.name) == Some(&true)) {
                (None, false) => Some(x.name.clone()),
                (Some(a), true) => Some(GType::new(&x.name, a.clone()).to_key()),
                _ => None,
            },
            Expr::EnumCtor(x) if self.types.contains_key(&x.enum_name) => match (&x.targs, self.types.get(&x.enum_name) == Some(&true)) {
                (None, false) => Some(x.enum_name.clone()),
                (Some(a), true) => Some(GType::new(&x.enum_name, a.clone()).to_key()),
                _ => None,
            },
            _ => None,
        }
    }

    /// A place `v.f.g` (an `inout self` or `take self` receiver must be one).
    fn is_place(e: &Expr) -> bool {
        match e {
            Expr::Var(_) => true,
            Expr::FieldAccess(fa) => Self::is_place(&fa.base),
            _ => false,
        }
    }

    fn expr(&self, e: &mut Expr, scopes: &mut Scopes) -> Result<(), PruvError> {
        match e {
            Expr::Call(c) => {
                let value_match = crate::ast::ValueMatch::matches(c) && !c.args.is_empty();
                let n = if value_match { 1 } else { c.args.len() };
                for a in c.args[..n].iter_mut() {
                    self.expr(a, scopes)?;
                }
                if let Some(m) = c.name.strip_prefix(METHOD_CALL) {
                    let m = m.to_string();
                    let Some(ty) = self.value_type(&c.args[0], scopes) else {
                        return Err(terr(c.line, c.col, "method_receiver",
                            format!("the type of the receiver of .{}(..) is not known before type checking", m),
                            "bind the value with `let x: T = ..;` first"));
                    };
                    let owner = GType::from_key(&ty);
                    let found = if owner.owner.is_none() && self.types.contains_key(&owner.name) {
                        self.methods.get(&(owner.name.clone(), m.clone()))
                    } else {
                        None
                    };
                    let Some((func, recv, is_pub)) = found else {
                        return Err(terr(c.line, c.col, "unknown_method",
                            format!("type {} has no method {}", repr_str(&ty), repr_str(&m)),
                            "declare it in an `impl` block of the type, or call a free function"));
                    };
                    if *recv == Receiver::None {
                        return Err(terr(c.line, c.col, "unknown_method",
                            format!("{}::{} has no receiver, so it is not a method", owner.name, m),
                            "call it as an associated function: Owner::f(..)"));
                    }
                    if !*is_pub && module_of(func) != self.cur_mod {
                        // P9b-2: as for a path call (modules.rs `access`)
                        return Err(terr(c.line, c.col, "private_item",
                            format!("{}::{} is private to module {}", owner.name, m, module_of(func)),
                            "mark it `pub fn` in its impl block, or call it through a public item of that module"));
                    }
                    let r = c.args[0].clone();
                    let (line, col) = (r.line(), r.col());
                    if *recv == Receiver::Inout && !Self::is_place(&r) {
                        return Err(terr(c.line, c.col, "method_receiver",
                            format!("{}::{} takes `inout self`, so its receiver must be a variable or a field of one", owner.name, m),
                            "bind the value with `let mut x: T = ..;` and call the method on x"));
                    }
                    c.args[0] = match recv {
                        Receiver::Inout => Expr::InoutArg(InoutArg { line, col, expr: Box::new(r), ty: None, post_refine: Vec::new() }),
                        Receiver::Take if Self::is_place(&r) => {
                            let mut path = Vec::new();
                            let mut cur = &r;
                            while let Expr::FieldAccess(fa) = cur {
                                path.push(fa.field.clone());
                                cur = &fa.base;
                            }
                            let Expr::Var(v) = cur else { unreachable!("is_place accepted the receiver") };
                            path.reverse();
                            Expr::Move(Move { line, col, name: v.name.clone(), fields: path, ty: None })
                        }
                        _ => r,
                    };
                    c.name = func.clone();
                    // P9b-3: a method of a generic type takes the receiver's type arguments first, then those written
                    // after its name (`s.map::<U>(..)`)
                    if !owner.args.is_empty() {
                        let mut t = owner.args.clone();
                        t.extend(c.targs.take().unwrap_or_default());
                        c.targs = Some(t);
                    }
                } else if value_match {
                    // P9b-3: the binders of a value `match` arm are in scope in the arm's value
                    let scrut = self.value_type(&c.args[0], scopes);
                    let arms = crate::ast::ValueMatch::arms(c);
                    for (a, e) in arms.iter().zip(c.args[1..].iter_mut()) {
                        let mut sc = HashMap::new();
                        self.bind_arm(scrut.as_deref(), &a.ctor, &a.binders, &[], &mut sc);
                        scopes.push(sc);
                        self.expr(e, scopes)?;
                        scopes.pop();
                    }
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

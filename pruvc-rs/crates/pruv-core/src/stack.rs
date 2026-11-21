//! P3-S1: deterministic post-mono analysis choosing local arrays that live on the stack. A rule-for-rule port of
//! pruvc/stack.py (same function order, same statement ordinal preorder, same exclusions, no host-entry exclusion).
//! Keyed by node address of the analysed AST (like storage.rs): the plan only answers for the unmoved `Let` nodes of the
//! program it was built from.
use crate::ast::*;
use crate::storage::{elem_size, StoragePlan};
use std::collections::{HashMap, HashSet};

pub const MAX_ARRAY_BYTES: usize = 16384;
pub const MAX_TOTAL_BYTES: usize = 65536;

#[derive(Debug, Default)]
pub struct StackPlan {
    /// (function, statement ordinal, name, length, element bytes)
    pub entries: Vec<(String, usize, String, usize, usize)>,
    ids: HashMap<usize, usize>,
    lits: HashSet<usize>,
}

impl StackPlan {
    pub fn lit(&self, node: &Let) -> bool {
        self.lits.contains(&(node as *const Let as usize))
    }
    pub fn contains(&self, node: &Let) -> bool {
        self.ids.contains_key(&(node as *const Let as usize))
    }
    pub fn length(&self, node: &Let) -> usize {
        self.ids[&(node as *const Let as usize)]
    }
}

// ---------- recursion ----------
fn calls_expr(e: &Expr, out: &mut HashSet<String>) {
    if let Expr::Call(c) = e {
        out.insert(c.name.clone());
    }
    for k in crate::refine::kids(e) {
        calls_expr(k, out);
    }
}

fn calls_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for st in stmts {
        match st {
            Stmt::Let(x) => calls_expr(&x.value, out),
            Stmt::Assign(x) => calls_expr(&x.value, out),
            Stmt::IndexAssign(x) => { calls_expr(&x.index, out); calls_expr(&x.value, out); }
            Stmt::FieldAssign(x) => calls_expr(&x.value, out),
            Stmt::Return(x) => { if let Some(v) = &x.value { calls_expr(v, out); } }
            Stmt::If(x) => { calls_expr(&x.cond, out); calls_stmts(&x.then, out); calls_stmts(&x.els, out); }
            Stmt::While(x) => {
                calls_expr(&x.cond, out);
                for inv in &x.invariants { calls_expr(inv, out); }
                calls_expr(&x.decreases, out);
                calls_stmts(&x.body, out);
            }
            Stmt::ForMap(x) => {
                calls_expr(&x.place, out);
                for inv in &x.invariants { calls_expr(inv, out); }
                calls_stmts(&x.body, out);
            }
            Stmt::Assert(x) => calls_expr(&x.cond, out),
            Stmt::ExprStmt(x) => calls_expr(&x.expr, out),
            Stmt::Par(x) => { for c in &x.calls { calls_expr(c, out); } }
            Stmt::Match(x) => {
                calls_expr(&x.scrutinee, out);
                for arm in &x.arms { calls_stmts(&arm.body, out); }
            }
        }
    }
}

/// names of functions on a call cycle (self-recursion included): f is recursive iff f is reachable from a callee
pub fn recursive_functions(prog: &Program) -> HashSet<String> {
    let fmap: HashSet<&str> = prog.functions.iter().map(|f| f.name.as_str()).collect();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for f in &prog.functions {
        let mut names = HashSet::new();
        calls_stmts(&f.body, &mut names);
        for e in &f.requires { calls_expr(e, &mut names); }
        for e in &f.ensures { calls_expr(e, &mut names); }
        if let Some(e) = &f.cost { calls_expr(e, &mut names); }
        if let Some(e) = &f.decreases { calls_expr(e, &mut names); }
        let mut v: Vec<String> = names.into_iter().filter(|n| fmap.contains(n.as_str())).collect();
        v.sort();
        edges.insert(f.name.clone(), v);
    }
    let mut rec = HashSet::new();
    for f in &prog.functions {
        let mut seen: HashSet<String> = HashSet::new();
        let mut todo: Vec<String> = edges[&f.name].clone();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.clone()) { continue; }
            todo.extend(edges[&n].iter().cloned());
        }
        if seen.contains(&f.name) { rec.insert(f.name.clone()); }
    }
    rec
}

// ---------- declarations ----------
/// element bytes of the declaration, or None when its element type does not qualify
fn elem_bytes(x: &Let, storage: &StoragePlan) -> Option<usize> {
    match x.ty.as_str() {
        "array_u8" | "array_bool" => Some(1),
        "array" => Some(elem_size(storage.layout_of(x))),
        _ => None,
    }
}

/// (length, elem bytes) when the declaration shape qualifies, else None (the total cap is checked by the caller)
fn eligible_decl(x: &Let, storage: &StoragePlan) -> Option<(usize, usize)> {
    let c = match &x.value {
        Expr::Call(c) if c.name == "alloc" && c.args.len() == 2 => c,
        _ => return None,
    };
    let n = match &c.args[0] {
        Expr::IntLit(n) if !n.value.starts_with('-') => n,
        _ => return None,
    };
    let eb = elem_bytes(x, storage)?;
    // a value that does not fit a usize is far over the cap
    let len: usize = n.value.parse().ok()?;
    if len.checked_mul(eb)? > MAX_ARRAY_BYTES { return None; }
    Some((len, eb))
}

/// true for `let x: str = "literal"`; such a binding may alias the read-only global when only read
fn eligible_lit(x: &Let) -> bool {
    x.ty == "str" && matches!(&x.value, Expr::StrLit(_))
}

// ---------- uses ----------
/// binding name -> address of the candidate `Let` (None: not a candidate, or shadowed)
type Scope = HashMap<String, Option<usize>>;

struct Uses<'a> {
    fmap: HashMap<&'a str, &'a Function>,
    storage: &'a StoragePlan,
    bad: HashSet<usize>,
    ordinal: usize,
    decls: Vec<(usize, &'a Let, usize, usize)>,
    lits: HashSet<usize>,
}

impl<'a> Uses<'a> {
    /// true when the scope entry is a literal candidate (Python: isinstance(d.value, StrLit))
    fn is_lit(&self, d: usize) -> bool {
        self.lits.contains(&d)
    }

    fn kill(&mut self, scope: &Scope, name: &str) {
        if let Some(Some(d)) = scope.get(name) { self.bad.insert(*d); }
    }

    fn expr(&mut self, e: &'a Expr, scope: &Scope, deny: bool) {
        match e {
            Expr::Var(x) => self.kill(scope, &x.name),
            Expr::Move(x) => self.kill(scope, &x.name),
            Expr::InoutArg(x) => self.expr(&x.expr, scope, true),
            Expr::Index(x) => {
                if matches!(&*x.array, Expr::Var(_)) && !deny {
                    // a read of x[i]
                } else {
                    self.expr(&x.array, scope, deny);
                }
                self.expr(&x.index, scope, deny);
            }
            Expr::Quant(x) => {
                self.expr(&x.lo, scope, deny);
                self.expr(&x.hi, scope, deny);
                let mut inner = scope.clone();
                inner.insert(x.var.clone(), None);
                self.expr(&x.body, &inner, deny);
                for t in &x.triggers {
                    self.expr(t, &inner, deny);
                }
            }
            Expr::Call(x) => self.call(x, scope, deny),
            _ => {
                for k in crate::refine::kids(e) { self.expr(k, scope, deny); }
            }
        }
    }

    fn call(&mut self, e: &'a Call, scope: &Scope, deny: bool) {
        if e.name == "len" && e.args.len() == 1 && matches!(e.args[0], Expr::Var(_)) && !deny { return; }
        let callee = self.fmap.get(e.name.as_str()).copied();
        for (j, a) in e.args.iter().enumerate() {
            let mut ok = false;
            if !deny {
                if let Some(c) = callee {
                    if c.foreign.is_none() && j < c.params.len()
                        && !matches!(e.name.as_str(), "len" | "alloc" | "copy" | "truncate" | "resize" | "print") {
                        let p = &c.params[j];
                        let array = matches!(p.ty.as_str(), "array" | "array_u8" | "array_bool" | "str");
                        if array && !p.take {
                            match a {
                                Expr::Var(_) if p.mode_pos.is_none() => ok = true,
                                Expr::InoutArg(ia) if matches!(&*ia.expr, Expr::Var(_)) && p.mode_pos.is_some() => {
                                    ok = true;
                                    // a literal candidate must not be mutated through inout
                                    if let Expr::Var(v) = &*ia.expr {
                                        if let Some(Some(d)) = scope.get(&v.name) {
                                            if self.is_lit(*d) { self.bad.insert(*d); }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            if ok { continue; }
            self.expr(a, scope, deny);
        }
    }

    fn block(&mut self, stmts: &'a [Stmt], outer: &Scope) {
        let mut scope = outer.clone();
        for st in stmts { self.stmt(st, &mut scope); }
    }

    fn stmt(&mut self, st: &'a Stmt, scope: &mut Scope) {
        self.ordinal += 1;
        let number = self.ordinal - 1;
        match st {
            Stmt::Let(x) => {
                self.expr(&x.value, scope, false);
                scope.insert(x.name.clone(), None);
                if let Some((len, eb)) = eligible_decl(x, self.storage) {
                    self.decls.push((number, x, len, eb));
                    scope.insert(x.name.clone(), Some(x as *const Let as usize));
                } else if eligible_lit(x) {
                    self.lits.insert(x as *const Let as usize);
                    scope.insert(x.name.clone(), Some(x as *const Let as usize));
                }
            }
            Stmt::Assign(x) => {
                self.kill(scope, &x.name);
                self.expr(&x.value, scope, false);
            }
            Stmt::IndexAssign(x) => {
                if let Some(Some(d)) = scope.get(&x.name) {
                    if self.is_lit(*d) { self.bad.insert(*d); }
                }
                self.expr(&x.index, scope, false);
                self.expr(&x.value, scope, false);
            }
            Stmt::FieldAssign(x) => {
                self.kill(scope, &x.name);
                self.expr(&x.value, scope, false);
            }
            Stmt::Return(x) => { if let Some(v) = &x.value { self.expr(v, scope, false); } }
            Stmt::If(x) => {
                self.expr(&x.cond, scope, false);
                self.block(&x.then, scope);
                self.block(&x.els, scope);
            }
            Stmt::While(x) => {
                self.expr(&x.cond, scope, false);
                for inv in &x.invariants { self.expr(inv, scope, false); }
                self.expr(&x.decreases, scope, false);
                self.block(&x.body, scope);
            }
            Stmt::ForMap(x) => {
                self.expr(&x.place, scope, false);
                for inv in &x.invariants { self.expr(inv, scope, false); }
                self.block(&x.body, scope);
            }
            Stmt::Assert(x) => self.expr(&x.cond, scope, false),
            Stmt::ExprStmt(x) => self.expr(&x.expr, scope, false),
            Stmt::Par(x) => { for c in &x.calls { self.expr(c, scope, true); } }
            Stmt::Match(x) => {
                self.expr(&x.scrutinee, scope, false);
                for arm in &x.arms {
                    let mut inner = scope.clone();
                    for name in &arm.binders { inner.insert(name.clone(), None); }
                    self.block(&arm.body, &inner);
                }
            }
        }
    }
}

pub fn plan_stack(prog: &Program, storage: &StoragePlan, verified: bool) -> StackPlan {
    let mut out = StackPlan::default();
    // Python: dict comprehension, the last declaration of a name wins
    let mut fmap: HashMap<&str, &Function> = HashMap::new();
    for f in &prog.functions { fmap.insert(f.name.as_str(), f); }
    let rec = recursive_functions(prog);
    for f in &prog.functions {
        if f.foreign.is_some() || (!verified && f.cost.is_some()) { continue; }
        let mut w = Uses { fmap: fmap.clone(), storage, bad: HashSet::new(), ordinal: 0, decls: Vec::new(), lits: HashSet::new() };
        let scope: Scope = f.params.iter().map(|p| (p.name.clone(), None)).collect();
        w.block(&f.body, &scope);
        for &id in &w.lits {
            if !w.bad.contains(&id) { out.lits.insert(id); }
        }
        if rec.contains(&f.name) { continue; } // only literals are planned in recursive functions
        let mut total = 0usize;
        for &(number, st, length, eb) in &w.decls {
            if w.bad.contains(&(st as *const Let as usize)) { continue; }
            let size = length * eb;
            if total + size > MAX_TOTAL_BYTES { continue; }
            total += size;
            out.ids.insert(st as *const Let as usize, length);
            out.entries.push((f.name.clone(), number, st.name.clone(), length, eb));
        }
    }
    out
}

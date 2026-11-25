//! KD-M6-3: small array results on the stack (docs/SMALL_RESULT_SPEC.md). A rule-for-rule port of pruvc/small.py.
//! `declared_bound` reads the contract bound K of `ensures len(result) <= K`; `plan_small` decides which functions get a
//! `__pruv_small` variant and which call sites are rewritten to use a caller-owned stack buffer. The plan is keyed by node
//! address (like storage.rs / stack.rs): it only answers for the unmoved `Let` nodes of the program it was built from.
use crate::ast::*;
use crate::refine::kids;
use crate::stack::StackPlan;
use crate::storage::StoragePlan;
use std::collections::{HashMap, HashSet};

pub const MAX_BOUND: i64 = 64;
/// bytes of rewritten buffers per function
pub const STACK_CAP: i64 = 4096;

/// result type -> integer buffer element; small results need the unnarrowed storage layout ("i64" in
/// StoragePlan terms)
pub fn elem_of(ret: &str) -> Option<&'static str> {
    crate::typeinfo::array_elem_name(ret).filter(|elem| crate::intwidth::of(elem).is_some())
}

fn elem_size(elem: &str) -> i64 {
    i64::from(crate::intwidth::of(elem).expect("integer buffer element").bits / 8)
}

#[derive(Debug, Default)]
pub struct SmallPlan {
    /// function name -> (K, the local `o` that is returned, buffer element)
    pub fns: HashMap<String, (i64, String, &'static str)>,
    /// address of a rewritten `Let` -> (callee, K, buffer element)
    pub sites: HashMap<usize, (String, i64, &'static str)>,
}

type Fmap<'a> = HashMap<String, &'a Function>;

fn conjuncts<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::Binary(b) = e {
        if b.op == "&&" {
            conjuncts(&b.left, out);
            conjuncts(&b.right, out);
            return;
        }
    }
    out.push(e);
}

/// K of a top-level `len(result) <= K` conjunct (literal, 1..=64) of a non-generic, non-foreign integer buffer
/// function
pub fn declared_bound(f: &Function) -> Option<i64> {
    if !f.tparams.is_empty() || f.foreign.is_some() || elem_of(&f.ret).is_none() {
        return None;
    }
    let mut cs: Vec<&Expr> = Vec::new();
    for e in &f.ensures {
        conjuncts(e, &mut cs);
    }
    let mut best: Option<i64> = None;
    for c in cs {
        if let Expr::Binary(b) = c {
            if b.op != "<=" {
                continue;
            }
            if let (Expr::Call(l), Expr::IntLit(r)) = (&*b.left, &*b.right) {
                if l.name == "len" && l.args.len() == 1 && is_var(&l.args[0], "result") {
                    if let Ok(k) = r.value.parse::<i64>() {
                        if (1..=MAX_BOUND).contains(&k) {
                            best = Some(best.map_or(k, |b| b.min(k)));
                        }
                    }
                }
            }
        }
    }
    best
}

fn is_var(e: &Expr, name: &str) -> bool {
    matches!(e, Expr::Var(v) if v.name == name)
}

/// every occurrence of `name` in e is len(name), name[i], or a direct argument of a user function
fn expr_ok(e: &Expr, name: &str, write_ok: bool, fmap: &Fmap) -> bool {
    match e {
        Expr::Var(v) => v.name != name,
        Expr::Move(m) => m.name != name,
        Expr::Call(c) => {
            if c.name == "len" && c.args.len() == 1 && is_var(&c.args[0], name) {
                return true;
            }
            let user = fmap.get(&c.name).map_or(false, |f| f.foreign.is_none());
            for a in &c.args {
                match a {
                    Expr::InoutArg(ia) if is_var(&ia.expr, name) => {
                        if !(write_ok && user) {
                            return false;
                        }
                    }
                    Expr::Var(v) if v.name == name => {
                        if !user {
                            return false;
                        }
                    }
                    _ => {
                        if !expr_ok(a, name, write_ok, fmap) {
                            return false;
                        }
                    }
                }
            }
            true
        }
        Expr::Index(ix) if is_var(&ix.array, name) => expr_ok(&ix.index, name, write_ok, fmap),
        _ => kids(e).into_iter().all(|k| expr_ok(k, name, write_ok, fmap)),
    }
}

fn mentions(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Var(v) => v.name == name,
        Expr::Move(m) => m.name == name,
        _ => kids(e).into_iter().any(|k| mentions(k, name)),
    }
}

fn stmts_ok(stmts: &[Stmt], name: &str, write_ok: bool, fmap: &Fmap) -> bool {
    for st in stmts {
        let ok = match st {
            Stmt::Let(x) => x.name != name && expr_ok(&x.value, name, write_ok, fmap),
            Stmt::Assign(x) => x.name != name && expr_ok(&x.value, name, write_ok, fmap),
            Stmt::FieldAssign(x) => x.name != name && expr_ok(&x.value, name, write_ok, fmap),
            Stmt::IndexAssign(x) => {
                !(x.name == name && !write_ok)
                    && expr_ok(&x.index, name, write_ok, fmap)
                    && expr_ok(&x.value, name, write_ok, fmap)
            }
            Stmt::If(x) => {
                expr_ok(&x.cond, name, write_ok, fmap)
                    && stmts_ok(&x.then, name, write_ok, fmap)
                    && stmts_ok(&x.els, name, write_ok, fmap)
            }
            Stmt::While(x) => expr_ok(&x.cond, name, write_ok, fmap) && stmts_ok(&x.body, name, write_ok, fmap),
            Stmt::ForMap(x) => expr_ok(&x.place, name, write_ok, fmap) && stmts_ok(&x.body, name, write_ok, fmap),
            Stmt::Return(x) => x.value.as_ref().map_or(true, |v| expr_ok(v, name, write_ok, fmap)),
            Stmt::ExprStmt(x) => expr_ok(&x.expr, name, write_ok, fmap),
            Stmt::Par(x) => !x.calls.iter().any(|c| mentions(c, name)),
            Stmt::Match(x) => {
                expr_ok(&x.scrutinee, name, write_ok, fmap)
                    && x.arms.iter().all(|a| !a.binders.iter().any(|b| b == name) && stmts_ok(&a.body, name, write_ok, fmap))
            }
            Stmt::Assert(_) => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// names called in executable positions (contracts are erased and do not count)
fn code_calls(stmts: &[Stmt], out: &mut HashSet<String>) {
    fn ex(e: &Expr, out: &mut HashSet<String>) {
        if let Expr::Call(c) = e {
            out.insert(c.name.clone());
        }
        for k in kids(e) {
            ex(k, out);
        }
    }
    for st in stmts {
        match st {
            Stmt::Let(x) => ex(&x.value, out),
            Stmt::Assign(x) => ex(&x.value, out),
            Stmt::FieldAssign(x) => ex(&x.value, out),
            Stmt::IndexAssign(x) => {
                ex(&x.index, out);
                ex(&x.value, out);
            }
            Stmt::If(x) => {
                ex(&x.cond, out);
                code_calls(&x.then, out);
                code_calls(&x.els, out);
            }
            Stmt::While(x) => {
                ex(&x.cond, out);
                code_calls(&x.body, out);
            }
            Stmt::ForMap(x) => {
                ex(&x.place, out);
                code_calls(&x.body, out);
            }
            Stmt::Return(x) => {
                if let Some(v) = &x.value {
                    ex(v, out);
                }
            }
            Stmt::ExprStmt(x) => ex(&x.expr, out),
            Stmt::Par(x) => {
                for c in &x.calls {
                    ex(c, out);
                }
            }
            Stmt::Match(x) => {
                ex(&x.scrutinee, out);
                for a in &x.arms {
                    code_calls(&a.body, out);
                }
            }
            Stmt::Assert(_) => {}
        }
    }
}

fn children(st: &Stmt) -> Vec<&[Stmt]> {
    match st {
        Stmt::If(x) => vec![x.then.as_slice(), x.els.as_slice()],
        Stmt::While(x) => vec![x.body.as_slice()],
        Stmt::ForMap(x) => vec![x.body.as_slice()],
        Stmt::Match(x) => x.arms.iter().map(|a| a.body.as_slice()).collect(),
        _ => Vec::new(),
    }
}

fn count(stmts: &[Stmt], pred: &dyn Fn(&Stmt) -> bool) -> usize {
    let mut n = 0;
    for st in stmts {
        if pred(st) {
            n += 1;
        }
        for body in children(st) {
            n += count(body, pred);
        }
    }
    n
}

struct Sites<'a, 'b> {
    fns: &'b HashMap<String, (i64, String, &'static str)>,
    fmap: &'b Fmap<'a>,
    storage: &'b StoragePlan,
    used: i64,
    full: bool,
    sites: &'b mut HashMap<usize, (String, i64, &'static str)>,
}

impl<'a, 'b> Sites<'a, 'b> {
    fn walk(&mut self, stmts: &[Stmt]) {
        for (i, st) in stmts.iter().enumerate() {
            if let Stmt::Let(x) = st {
                if !x.mutable && !self.full && self.storage.layout_of(x) == "i64" {
                    if let Expr::Call(c) = &x.value {
                        if let Some((k, _, elem)) = self.fns.get(&c.name) {
                            if elem_of(&x.ty) == Some(*elem) && stmts_ok(&stmts[i + 1..], &x.name, false, self.fmap) {
                                let bytes = elem_size(elem) * k;
                                if self.used + bytes <= STACK_CAP {
                                    self.used += bytes;
                                    self.sites.insert(x as *const Let as usize, (c.name.clone(), *k, *elem));
                                } else {
                                    self.full = true;
                                }
                            }
                        }
                    }
                }
            }
            for body in children(st) {
                self.walk(body);
            }
        }
    }
}

/// Names on a call cycle (iterative Tarjan, linear in the call graph).
pub(crate) fn cyclic(calls: &HashMap<String, HashSet<String>>) -> HashSet<String> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut low: HashMap<&str, usize> = HashMap::new();
    let mut on: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut out: HashSet<String> = HashSet::new();
    let succ = |v: &str| -> Vec<&str> { calls.get(v).map_or(Vec::new(), |s| s.iter().map(|x| x.as_str()).collect()) };
    for root in calls.keys() {
        let root = root.as_str();
        if index.contains_key(root) {
            continue;
        }
        let i = index.len();
        index.insert(root, i);
        low.insert(root, i);
        stack.push(root);
        on.insert(root);
        let mut work: Vec<(&str, Vec<&str>, usize)> = vec![(root, succ(root), 0)];
        while let Some(top) = work.last_mut() {
            let v = top.0;
            if top.2 < top.1.len() {
                let w = top.1[top.2];
                top.2 += 1;
                if !index.contains_key(w) {
                    let i = index.len();
                    index.insert(w, i);
                    low.insert(w, i);
                    stack.push(w);
                    on.insert(w);
                    work.push((w, succ(w), 0));
                } else if on.contains(w) {
                    let m = low[v].min(index[w]);
                    low.insert(v, m);
                }
                continue;
            }
            work.pop();
            if let Some(parent) = work.last() {
                let u = parent.0;
                let m = low[u].min(low[v]);
                low.insert(u, m);
            }
            if low[v] == index[v] {
                let mut comp: Vec<&str> = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on.remove(w);
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                if comp.len() > 1 || calls.get(v).map_or(false, |s| s.contains(v)) {
                    out.extend(comp.into_iter().map(|s| s.to_string()));
                }
            }
        }
    }
    out
}

pub fn plan_small(
    prog: &Program, fmap: &Fmap, storage: &StoragePlan, stack: &StackPlan, resizes: &HashSet<(String, usize)>,
) -> SmallPlan {
    let mut calls: HashMap<String, HashSet<String>> = HashMap::new();
    for f in fmap.values() {
        let mut out = HashSet::new();
        if f.foreign.is_none() {
            code_calls(&f.body, &mut out);
        }
        calls.insert(f.name.clone(), out.into_iter().filter(|n| fmap.contains_key(n)).collect());
    }
    let mut callers: HashMap<&str, Vec<&str>> = HashMap::new();
    for (n, cs) in &calls {
        for m in cs {
            callers.entry(m.as_str()).or_default().push(n.as_str());
        }
    }
    let mut tainted: HashSet<String> = fmap.iter().filter(|(_, f)| f.foreign.is_some()).map(|(n, _)| n.clone()).collect();
    let mut todo: Vec<String> = tainted.iter().cloned().collect();
    while let Some(n) = todo.pop() {
        for c in callers.get(n.as_str()).map_or(&[][..], |v| &v[..]) {
            if tainted.insert((*c).to_string()) {
                todo.push((*c).to_string());
            }
        }
    }
    let cyclic = cyclic(&calls);
    let in_cycle = |n: &str| -> bool { cyclic.contains(n) };

    let mut plan = SmallPlan::default();
    for f in &prog.functions {
        if f.foreign.is_some()
            || plan.fns.contains_key(&f.name)
            || !fmap.get(&f.name).map_or(false, |g| std::ptr::eq(*g, f))
            || elem_of(&f.ret).is_none()
            || tainted.contains(&f.name)
        {
            continue;
        }
        let k = match f.small_bound.or_else(|| declared_bound(f)) {
            Some(k) if (1..=MAX_BOUND).contains(&k) => k,
            _ => continue,
        };
        let o = match f.body.last() {
            Some(Stmt::Return(Return { value: Some(Expr::Var(v)), .. })) => v.name.clone(),
            _ => continue,
        };
        let idx: Vec<usize> = f
            .body
            .iter()
            .enumerate()
            .filter(|(_, st)| matches!(st, Stmt::Let(x) if x.name == o))
            .map(|(i, _)| i)
            .collect();
        if idx.len() != 1 || count(&f.body, &|st| matches!(st, Stmt::Let(x) if x.name == o)) != 1 {
            continue;
        }
        let decl = match &f.body[idx[0]] {
            Stmt::Let(x) => x,
            _ => continue,
        };
        let alloc_ok = matches!(&decl.value, Expr::Call(c) if c.name == "alloc" && c.args.len() == 2);
        let elem = elem_of(&f.ret).unwrap_or("i64");
        if !(decl.mutable && decl.ty == f.ret && alloc_ok) {
            continue;
        }
        if count(&f.body, &|st| matches!(st, Stmt::Return(_))) != 1 {
            continue;
        }
        if storage.layout_of(f) != "i64" || storage.layout_of(decl) != "i64" || stack.contains(decl) {
            continue;
        }
        if !stmts_ok(&f.body[idx[0] + 1..f.body.len() - 1], &o, true, fmap) {
            continue;
        }
        plan.fns.insert(f.name.clone(), (k, o, elem));
    }

    // The resizable-slot filter runs before the site walk so ineligible candidates take no budget.
    // The slice-backed result cannot be passed to a helper that needs a Vec slot.
    plan.fns.retain(|name, (_, buffer, _)| {
        !crate::check::resize_slot_names(fmap[name], resizes).contains(buffer)
    });
    if plan.fns.is_empty() {
        return plan;
    }
    let mut sites: HashMap<usize, (String, i64, &'static str)> = HashMap::new();
    for f in &prog.functions {
        if f.foreign.is_some() || !fmap.get(&f.name).map_or(false, |g| std::ptr::eq(*g, f)) || in_cycle(&f.name) {
            continue;
        }
        let mut w = Sites { fns: &plan.fns, fmap, storage, used: 0, full: false, sites: &mut sites };
        w.walk(&f.body);
    }
    plan.sites = sites;
    plan
}

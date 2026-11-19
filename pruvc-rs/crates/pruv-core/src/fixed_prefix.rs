//! P18d: by-value fixed-prefix twin for `inout o: [i64]` (--proven-entries). A rule-for-rule port of pruvc/fixed_prefix.py.
//! `plan_fp` decides which internal functions whose LAST parameter is `inout o: [i64]` only ever touch `o` through a
//! statically fixed prefix of K words: literal indices < K, `len(o)` compared with a literal (the comparison has the same
//! truth value for every runtime length >= K), and `inout o` passed as the last argument to another planned function.
//! `cands` are the functions the code generator already filtered (concrete specializations only, non-foreign, not host
//! entries, not body_named, not in the small plan, no cost, last parameter emitted as `&mut [i64]`), and only in a
//! verified program without G10 runtime or par groups (the code generator checks that); `may_unwind` names the
//! functions that reach a foreign function, a G6 catalog operation or print, which get no twin.
use crate::ast::*;
use crate::foreign::is_catalog;
use crate::refine::kids;
use std::collections::{HashMap, HashSet};

pub const MAX_K: i64 = 16;

fn call_names(e: &Expr, acc: &mut HashSet<String>) {
    if let Expr::Call(c) = e {
        acc.insert(c.name.clone());
    }
    for k in kids(e) {
        call_names(k, acc);
    }
}

fn stmt_call_names(stmts: &[Stmt], acc: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Let(l) => call_names(&l.value, acc),
            Stmt::Assign(a) => call_names(&a.value, acc),
            Stmt::IndexAssign(a) => {
                call_names(&a.index, acc);
                call_names(&a.value, acc);
            }
            Stmt::If(i) => {
                call_names(&i.cond, acc);
                stmt_call_names(&i.then, acc);
                stmt_call_names(&i.els, acc);
            }
            Stmt::While(w) => {
                call_names(&w.cond, acc);
                stmt_call_names(&w.body, acc);
            }
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    call_names(v, acc);
                }
            }
            Stmt::Assert(a) => call_names(&a.cond, acc),
            Stmt::Par(p) => p.calls.iter().for_each(|c| call_names(c, acc)),
            Stmt::ExprStmt(e) => call_names(&e.expr, acc),
            Stmt::FieldAssign(f) => call_names(&f.value, acc),
            Stmt::Match(m) => {
                call_names(&m.scrutinee, acc);
                m.arms.iter().for_each(|a| stmt_call_names(&a.body, acc));
            }
        }
    }
}

/// Names of the functions that can unwind through a call: a foreign function, a call to print (println! panics when
/// stdout fails), or a call to a function in this set. A twin's adapter copies the prefix back only when the core
/// returns, so these never get one. (Verified bodies have no other panic; the code generator turns the plan off for
/// G10 runtime and par programs.) Same rule as pruvc/fixed_prefix.py may_unwind.
pub fn may_unwind(prog: &Program, fmap: &Fmap) -> HashSet<String> {
    let mut calls: Vec<(String, HashSet<String>)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut bad: HashSet<String> = HashSet::new();
    for f in &prog.functions {
        if !seen.insert(f.name.as_str()) {
            continue;
        }
        let mut acc = HashSet::new();
        stmt_call_names(&f.body, &mut acc);
        let g_foreign = fmap.get(&f.name).map_or(false, |g| g.foreign.is_some());
        // a G6 catalog operation's boundary helper panics on a failed check or an exhausted check budget
        let catalog = acc.iter().any(|c| is_catalog(c) && !fmap.contains_key(c));
        if f.foreign.is_some() || g_foreign || acc.contains("print") || catalog {
            bad.insert(f.name.clone());
        }
        calls.push((f.name.clone(), acc));
    }
    bad.extend(fmap.iter().filter(|(_, g)| g.foreign.is_some()).map(|(n, _)| n.clone()));
    let mut changed = true;
    while changed {
        changed = false;
        for (n, cs) in &calls {
            if !bad.contains(n) && cs.iter().any(|c| bad.contains(c)) {
                bad.insert(n.clone());
                changed = true;
            }
        }
    }
    bad
}

type Fmap<'a> = HashMap<String, &'a Function>;

fn flip(op: &str) -> Option<&'static str> {
    match op {
        "<" => Some(">"),
        "<=" => Some(">="),
        ">" => Some("<"),
        ">=" => Some("<="),
        "==" => Some("=="),
        "!=" => Some("!="),
        _ => None,
    }
}

fn is_len(e: &Expr, o: &str) -> bool {
    match e {
        Expr::Call(c) => c.name == "len" && c.args.len() == 1 && matches!(&c.args[0], Expr::Var(v) if v.name == o),
        _ => false,
    }
}

/// the value of a non-negative integer literal (None: not a literal, negative, or too large for i64)
fn lit(e: &Expr) -> Option<i64> {
    match e {
        Expr::IntLit(l) => l.value.parse::<i64>().ok().filter(|v| *v >= 0),
        _ => None,
    }
}

/// K needed so that `len(o) OP c` has the same truth value for every length >= K
fn len_need(op: &str, c: i64) -> i64 {
    if op == "<" || op == ">=" { c } else { c.saturating_add(1) }
}

struct Acc {
    need: i64,
    callees: Vec<String>,
}

/// every occurrence of o in e is a U1-U4 use
fn expr_ok(e: &Expr, o: &str, fmap: &Fmap, acc: &mut Acc) -> bool {
    match e {
        Expr::Var(v) => return v.name != o,
        Expr::Move(m) => return m.name != o,
        Expr::Binary(b) => {
            if let Some(fl) = flip(&b.op) {
                let sides: [(&Expr, &Expr, &str); 2] = [(&*b.left, &*b.right, b.op.as_str()), (&*b.right, &*b.left, fl)];
                for (a, c, op) in sides {
                    if is_len(a, o) {
                        if let Some(v) = lit(c) {
                            acc.need = acc.need.max(len_need(op, v));
                            return true;
                        }
                    }
                }
            }
        }
        Expr::Index(ix) => {
            if matches!(&*ix.array, Expr::Var(v) if v.name == o) {
                return match lit(&ix.index) {
                    Some(c) => {
                        acc.need = acc.need.max(c.saturating_add(1));
                        true
                    }
                    None => false,
                };
            }
        }
        Expr::Quant(q) => {
            if q.var == o {
                return false;
            }
        }
        Expr::Call(c) => {
            let n = c.args.len();
            for (i, a) in c.args.iter().enumerate() {
                let inout_o = matches!(a, Expr::InoutArg(ia) if matches!(&*ia.expr, Expr::Var(v) if v.name == o));
                if inout_o {
                    let good = match fmap.get(&c.name) {
                        Some(g) => {
                            i == n - 1
                                && g.foreign.is_none()
                                && g.params.len() == n
                                && g.params.last().map_or(false, |p| p.mode_pos.is_some())
                        }
                        None => false,
                    };
                    if !good {
                        return false;
                    }
                    acc.callees.push(c.name.clone());
                } else if !expr_ok(a, o, fmap, acc) {
                    return false;
                }
            }
            return true;
        }
        _ => {}
    }
    kids(e).into_iter().all(|k| expr_ok(k, o, fmap, acc))
}

fn mentions(e: &Expr, o: &str) -> bool {
    match e {
        Expr::Var(v) => v.name == o,
        Expr::Move(m) => m.name == o,
        _ => kids(e).into_iter().any(|k| mentions(k, o)),
    }
}

fn stmts_ok(stmts: &[Stmt], o: &str, fmap: &Fmap, acc: &mut Acc) -> bool {
    for st in stmts {
        let ok = match st {
            Stmt::Let(x) => x.name != o && expr_ok(&x.value, o, fmap, acc),
            Stmt::Assign(x) => x.name != o && expr_ok(&x.value, o, fmap, acc),
            Stmt::FieldAssign(x) => x.name != o && expr_ok(&x.value, o, fmap, acc),
            Stmt::IndexAssign(x) => {
                let idx = if x.name == o {
                    match lit(&x.index) {
                        Some(c) => {
                            acc.need = acc.need.max(c.saturating_add(1));
                            true
                        }
                        None => false,
                    }
                } else {
                    expr_ok(&x.index, o, fmap, acc)
                };
                idx && expr_ok(&x.value, o, fmap, acc)
            }
            Stmt::If(x) => {
                expr_ok(&x.cond, o, fmap, acc) && stmts_ok(&x.then, o, fmap, acc) && stmts_ok(&x.els, o, fmap, acc)
            }
            Stmt::While(x) => expr_ok(&x.cond, o, fmap, acc) && stmts_ok(&x.body, o, fmap, acc),
            Stmt::Return(x) => match &x.value {
                Some(v) => expr_ok(v, o, fmap, acc),
                None => true,
            },
            Stmt::Assert(x) => expr_ok(&x.cond, o, fmap, acc),
            Stmt::ExprStmt(x) => expr_ok(&x.expr, o, fmap, acc),
            Stmt::Par(x) => !x.calls.iter().any(|c| mentions(c, o)),
            Stmt::Match(x) => {
                let mut ok = expr_ok(&x.scrutinee, o, fmap, acc);
                for a in &x.arms {
                    ok = ok && !a.binders.iter().any(|b| b == o) && stmts_ok(&a.body, o, fmap, acc);
                }
                ok
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

/// {function name: (K, o)} of the functions that get the by-value twin
/// `unwind`: may_unwind's names, which get no twin.
pub fn plan_fp(cands: &[&Function], fmap: &Fmap, unwind: &HashSet<String>) -> HashMap<String, (i64, String)> {
    // name -> (need, callee names) of every candidate that passed the shape rules; `order` keeps declaration order
    let mut own: HashMap<String, (i64, Vec<String>)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for f in cands {
        if own.contains_key(&f.name) || unwind.contains(&f.name) || !fmap.get(&f.name).map_or(false, |g| std::ptr::eq(*g, *f)) {
            continue;
        }
        let o = match f.params.last() {
            Some(p) => p.name.as_str(),
            None => continue,
        };
        let mut acc = Acc { need: 1, callees: Vec::new() };
        if stmts_ok(&f.body, o, fmap, &mut acc) {
            own.insert(f.name.clone(), (acc.need, acc.callees));
            order.push(f.name.clone());
        }
    }
    let mut calls: HashMap<String, HashSet<String>> = HashMap::new();
    for (n, (_, cs)) in &own {
        calls.insert(n.clone(), cs.iter().filter(|c| own.contains_key(*c)).cloned().collect());
    }
    let mut rej: HashSet<String> = crate::small::cyclic(&calls);
    let mut ks: HashMap<String, i64> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for n in &order {
            if rej.contains(n) || ks.contains_key(n) {
                continue;
            }
            let cs = &own[n].1;
            let need = own[n].0;
            if cs.iter().any(|c| !own.contains_key(c) || rej.contains(c)) {
                rej.insert(n.clone());
                changed = true;
            } else if cs.iter().all(|c| ks.contains_key(c)) {
                let k = cs.iter().map(|c| ks[c]).fold(need, i64::max);
                if k > MAX_K {
                    rej.insert(n.clone());
                } else {
                    ks.insert(n.clone(), k);
                }
                changed = true;
            }
        }
    }
    ks.into_iter()
        .map(|(n, k)| {
            let o = fmap[&n].params.last().map(|p| p.name.clone()).unwrap_or_default();
            (n, (k, o))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the plan of every function of `src` whose last parameter is an inout `[i64]`, as {name: K}
    fn plan(src: &str) -> HashMap<String, i64> {
        let prog = crate::parse(src).expect("parse");
        let mut fmap: Fmap = HashMap::new();
        for f in &prog.functions {
            fmap.entry(f.name.clone()).or_insert(f);
        }
        let cands: Vec<&Function> = prog
            .functions
            .iter()
            .filter(|f| f.params.last().map_or(false, |p| p.mode_pos.is_some() && p.ty == "array"))
            .collect();
        plan_fp(&cands, &fmap, &HashSet::new()).into_iter().map(|(n, (k, _))| (n, k)).collect()
    }

    #[test]
    fn accepts_literal_prefix_and_chain() {
        let got = plan(
            "fn g(x: i64, inout o: [i64]) -> i64 {\n  if len(o) < 1 { return 3; }\n  o[0] = x;\n  return 0;\n}\n\
             fn f(x: i64, inout o: [i64]) -> i64 {\n  let r: i64 = g(x, inout o);\n  if 2 <= len(o) { o[1] = r; }\n  return r;\n}\n",
        );
        assert_eq!(got.get("g"), Some(&1));
        assert_eq!(got.get("f"), Some(&2));
    }

    #[test]
    fn len_comparison_needs() {
        let got = plan("fn a(inout o: [i64]) -> i64 {\n  if len(o) <= 3 { return 1; }\n  return 0;\n}\n");
        assert_eq!(got.get("a"), Some(&4));
        let got = plan("fn a(inout o: [i64]) -> i64 {\n  if len(o) >= 3 { return 1; }\n  return 0;\n}\n");
        assert_eq!(got.get("a"), Some(&3));
    }

    #[test]
    fn rejects() {
        // variable index
        assert!(plan("fn a(i: i64, inout o: [i64]) -> i64 {\n  o[i] = 1;\n  return 0;\n}\n").is_empty());
        // callee with a variable index (not planned), so the caller is rejected too
        let got = plan(
            "fn wr(inout a: [i64], i: i64, v: i64) -> i64 {\n  a[i] = v;\n  return 0;\n}\n\
             fn f(v: i64, inout o: [i64]) -> i64 {\n  let r: i64 = wr(inout o, 0, v);\n  return r;\n}\n",
        );
        assert!(got.is_empty());
        // len(o) in arithmetic
        assert!(plan("fn a(inout o: [i64]) -> i64 {\n  return len(o) + 1;\n}\n").is_empty());
        // recursion
        assert!(plan("fn a(n: i64, inout o: [i64]) -> i64 {\n  o[0] = n;\n  let r: i64 = a(n, inout o);\n  return r;\n}\n").is_empty());
        // K = 17
        assert!(plan("fn a(inout o: [i64]) -> i64 {\n  o[16] = 1;\n  return 0;\n}\n").is_empty());
        assert_eq!(plan("fn a(inout o: [i64]) -> i64 {\n  o[15] = 1;\n  return 0;\n}\n").get("a"), Some(&16));
    }
}

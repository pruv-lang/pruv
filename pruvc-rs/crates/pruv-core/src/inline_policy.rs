// Inline policy (SPEC "Inline policy"; Python `inline_policy`): a pass over the finished Rust text, so the Python and Rust
// compilers decide the same way by construction. A non-recursive `#[inline]` fn (one that cannot reach itself through
// calls) gets `#[inline(always)]` when its inclusive size (own body lines plus, once per call, the inclusive size of
// each callee that gets always) is at most INLINE_SMALL, when it has exactly one call site, or when it has two or more and
// the copies it adds beyond one, incl * (call sites - 1), fit INLINE_BUDGET and, if its own body has a loop (a line
// starting with `while `, `for ` or `loop {`), incl is at most INLINE_LOOP. A fn with zero call sites (a host entry
// called only from the harness closures) is not promoted by the call-site rules, so it gets always only at
// incl <= INLINE_SMALL. The rest keep `#[inline]` and rustc's own heuristics.
use crate::shared_tables::inline_policy::{INLINE_BUDGET, INLINE_LOOP, INLINE_SMALL}; // shared with pruvc (pruvc/data/inline_policy.json, tools/gen_shared.py)
use std::collections::{BTreeMap, HashMap};

// Private codegen markers consumed before the Rust text is returned.
pub(crate) const PIN_INLINE: &str = "#[inline] // pruv: explicit";
pub(crate) const PIN_ALWAYS: &str = "#[inline(always)] // pruv: explicit";

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// fn index -> number of call tokens in text: an identifier of names followed by '(', not after '.' or 'fn '
fn inline_calls(text: &str, index: &HashMap<&str, usize>) -> BTreeMap<usize, u64> {
    let b = text.as_bytes();
    let mut got = BTreeMap::new();
    let (mut i, n) = (0, b.len());
    while i < n {
        if !ident_byte(b[i]) {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < n && ident_byte(b[e]) {
            e += 1;
        }
        if e < n && b[e] == b'(' && (i == 0 || b[i - 1] != b'.') && !(i >= 3 && &b[i - 3..i] == b"fn ") {
            if let Some(&k) = std::str::from_utf8(&b[i..e]).ok().and_then(|w| index.get(w)) {
                *got.entry(k).or_insert(0) += 1;
            }
        }
        i = e;
    }
    got
}

/// True when a body line starts (after spaces) with `while `, `for ` or `loop {`: the loops codegen emits.
fn has_loop(body: &str) -> bool {
    body.split('\n').any(|ln| {
        let t = ln.trim_start_matches(' ');
        t.starts_with("while ") || t.starts_with("for ") || t.starts_with("loop {")
    })
}

pub(crate) fn inline_policy(text: String) -> String {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // (attribute line, name, size, body text, pinned always)
    let mut fns: Vec<(usize, &str, u64, String, bool)> = Vec::new();
    for i in 0..lines.len().saturating_sub(1) {
        if lines[i] == PIN_INLINE {
            lines[i] = "#[inline]";
            continue;
        }
        let pinned = lines[i] == PIN_ALWAYS;
        if pinned {
            lines[i] = "#[inline(always)]";
        } else if lines[i] != "#[inline]" {
            continue;
        }
        let h = lines[i + 1];
        let p = if h.starts_with("pub fn ") {
            4
        } else if h.starts_with("pub(crate) fn ") {
            11
        } else {
            0
        };
        if !h[p..].starts_with("fn ") {
            continue;
        }
        let hb = h.as_bytes();
        let mut e = p + 3;
        while e < hb.len() && ident_byte(hb[e]) {
            e += 1;
        }
        if e == p + 3 || e >= hb.len() || hb[e] != b'(' {
            continue;
        }
        let name = &h[p + 3..e];
        if h.ends_with('}') && h.matches('{').count() == h.matches('}').count() {
            fns.push((i, name, 1, h.to_string(), pinned));
            continue;
        }
        let mut end = i + 2;
        while end < lines.len() && lines[end] != "}" {
            end += 1;
        }
        if end == lines.len() {
            continue;
        }
        fns.push((i, name, std::cmp::max(end - i - 2, 1) as u64, lines[i + 2..end].join("\n"), pinned));
    }
    if fns.is_empty() {
        return lines.join("\n");
    }
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (k, f) in fns.iter().enumerate() {
        index.entry(f.1).or_insert(k);
    }
    let sites = inline_calls(&text, &index);
    let mut edges: Vec<Vec<(usize, u64)>> = fns.iter().map(|f| inline_calls(&f.3, &index).into_iter().collect()).collect();
    let nf = fns.len();
    // Recursive automatic candidates add nothing. Explicit always still contributes its finite size.
    let mut rec = vec![false; nf];
    for v in 0..nf {
        let mut seen = vec![false; nf];
        let mut todo: Vec<usize> = edges[v].iter().map(|e| e.0).collect();
        while let Some(w) = todo.pop() {
            if w == v {
                rec[v] = true;
                break;
            }
            if !seen[w] {
                seen[w] = true;
                todo.extend(edges[w].iter().map(|e| e.0));
            }
        }
    }
    // Cut edges between recursive functions so pinned recursion has a finite inclusive size.
    for v in 0..nf {
        if rec[v] {
            edges[v].retain(|&(w, _)| !rec[w]);
        }
    }
    // Evaluate the remaining DAG in post-order (iterative, call chains can be deep).
    let mut done: Vec<bool> = (0..nf).map(|v| rec[v] && !fns[v].4).collect();
    let mut incl = vec![0u64; nf];
    let mut always = vec![false; nf];
    for root in 0..nf {
        if done[root] {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&(v, k)) = stack.last() {
            if k < edges[v].len() {
                stack.last_mut().unwrap().1 = k + 1;
                let w = edges[v][k].0;
                if !done[w] {
                    stack.push((w, 0));
                }
                continue;
            }
            stack.pop();
            done[v] = true;
            let mut tot = fns[v].2;
            for &(w, c) in &edges[v] {
                if always[w] {
                    tot = tot.saturating_add(c.saturating_mul(incl[w]));
                }
            }
            incl[v] = tot;
            if fns[v].4 {
                always[v] = true;
            } else {
                // P18d: the by-value twin has one textual call site (its adapter); count the adapter's callers.
                // Only a generated triple (base, base__pruv_fp, base__pruv_fpb) counts, never a user name ending so.
                let nm = fns[v].1;
                let nm = match nm.strip_suffix("__pruv_fp") {
                    Some(b) if index.contains_key(b) && index.contains_key(format!("{}b", nm).as_str()) => b,
                    _ => nm,
                };
                let calls = index.get(nm).and_then(|k| sites.get(k)).copied().unwrap_or(0);
                // LI1: a loop amortizes the call, so copying a larger loop body to several sites only adds code
                // pages (bench/INLINE_POLICY_LI1.md); rustc keeps such fns out of line in the hand Rust as well.
                always[v] = tot <= INLINE_SMALL
                    || calls == 1
                    || (calls >= 2 && tot.saturating_mul(calls - 1) <= INLINE_BUDGET && (tot <= INLINE_LOOP || !has_loop(&fns[v].3)));
            }
        }
    }
    for (v, f) in fns.iter().enumerate() {
        if always[v] {
            lines[f.0] = "#[inline(always)]";
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_policy_pub_crate_upgrade() {
        let a = inline_policy("#[inline]\npub fn f(x: i64) -> i64 {\n    x\n}\n".to_string());
        let b = inline_policy("#[inline]\npub(crate) fn f(x: i64) -> i64 {\n    x\n}\n".to_string());
        assert_eq!(a, "#[inline(always)]\npub fn f(x: i64) -> i64 {\n    x\n}\n");
        assert_eq!(b, "#[inline(always)]\npub(crate) fn f(x: i64) -> i64 {\n    x\n}\n");
    }

    /// One `#[inline]` fn of `lines` body lines, called `sites` times from a plain fn; true when it gets always.
    fn inline_case(lines: usize, sites: usize) -> bool {
        let mut t = String::from("#[inline]\nfn f(x: i64) -> i64 {\n");
        for i in 0..lines - 1 {
            t.push_str(&format!("    let _ = x + {};\n", i));
        }
        t.push_str("    x\n}\nfn main() {\n");
        for _ in 0..sites {
            t.push_str("    f(0);\n");
        }
        t.push_str("}\n");
        inline_policy(t).starts_with("#[inline(always)]\n")
    }

    /// Like `inline_case`, but `lines` body lines include one `while` line when `looped`.
    fn inline_loop_case(lines: usize, sites: usize, looped: bool) -> bool {
        let mut t = String::from("#[inline]\nfn f(x: i64) -> i64 {\n");
        for i in 0..lines - if looped { 2 } else { 1 } {
            t.push_str(&format!("    let _ = x + {};\n", i));
        }
        if looped {
            t.push_str("    while x < 0 { }\n");
        }
        t.push_str("    x\n}\nfn main() {\n");
        for _ in 0..sites {
            t.push_str("    f(0);\n");
        }
        t.push_str("}\n");
        inline_policy(t).starts_with("#[inline(always)]\n")
    }

    /// LI1 loop condition, spec values (tests/check_generated.py has the same cases for the Python compiler).
    #[test]
    fn inline_policy_loop_condition() {
        assert!(!inline_loop_case(65, 3, true), "loop fn, 3 sites, incl 65 keeps #[inline]");
        assert!(inline_loop_case(64, 3, true), "loop fn, 3 sites, incl 64 gets always");
        assert!(inline_loop_case(65, 3, false), "loop-free fn, 3 sites, incl 65 still gets always");
        assert!(inline_loop_case(200, 1, true), "loop fn, 1 site, incl 200 still gets always");
    }

    /// P18d: a `__pruv_fp` twin is counted by the call sites of its base name, not by its single adapter call.
    #[test]
    fn inline_policy_fp_uses_base_sites() {
        let mut t = String::from(
            "#[inline(always)] // pruv: explicit\nfn f(x: i64) -> i64 {\n    f__pruv_fp(x)\n}\n#[inline]\nfn f__pruv_fp(x: i64) -> i64 {\n",
        );
        for i in 0..20 {
            t.push_str(&format!("    let _ = x + {};\n", i));
        }
        t.push_str("    x\n}\n#[inline(always)] // pruv: explicit\nfn f__pruv_fpb(x: i64) -> i64 {\n    x\n}\nfn main() {\n");
        for _ in 0..300 {
            t.push_str("    f(0);\n");
        }
        t.push_str("}\n");
        let few = t.replace("    f(0);\n", "").replace("fn main() {\n", "fn main() {\n    f(0);\n");
        assert!(inline_policy(t.clone()).contains("#[inline]\nfn f__pruv_fp("), "many base sites keep #[inline]");
        assert!(inline_policy(few).contains("#[inline(always)]\nfn f__pruv_fp("), "one base site forces always");
        // without the generated __pruv_fpb body the name is an ordinary function with its own single call site
        let lone = t.replace("fn f__pruv_fpb(", "fn g__other(");
        assert!(inline_policy(lone).contains("#[inline(always)]\nfn f__pruv_fp("), "no triple, own sites");
    }

    /// Spec thresholds, not the JSON values, so a wrong pruvc/data/inline_policy.json is caught
    /// (tests/check_generated.py has the same pairs for the Python compiler).
    #[test]
    fn inline_policy_thresholds() {
        assert!(inline_case(8, 300), "INLINE_SMALL 8 accepts");
        assert!(!inline_case(9, 300), "9 lines reject");
        assert!(inline_case(1500, 2), "INLINE_BUDGET 1500 accepts");
        assert!(!inline_case(1501, 2), "1501 rejects");
        assert!(inline_case(8, 0), "0 sites, 8 lines accepts (INLINE_SMALL)");
        assert!(!inline_case(9, 0), "0 sites, 9 lines rejects (host entry)");
        assert!(inline_case(9, 1), "1 site still accepts");
    }
}

// Inline policy (SPEC "Inline policy"; Python `inline_policy`): a pass over the finished Rust text, so the Python and Rust
// compilers decide the same way by construction. A non-recursive `#[inline]` fn (one that cannot reach itself through
// calls) gets `#[inline(always)]` when its inclusive size (own body lines plus, once per call, the inclusive size of
// each callee that gets always) is at most INLINE_SMALL, when it has at most one call site, or when the copies it adds
// beyond one, incl * (call sites - 1), fit INLINE_BUDGET. The rest keep `#[inline]` and rustc's own heuristics.
use crate::shared_tables::inline_policy::{INLINE_BUDGET, INLINE_SMALL}; // shared with pruvc (pruvc/data/inline_policy.json, tools/gen_shared.py)
use std::collections::{BTreeMap, HashMap};

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

pub(crate) fn inline_policy(text: String) -> String {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // (attribute line, name, size, body text)
    let mut fns: Vec<(usize, &str, u64, String)> = Vec::new();
    for i in 0..lines.len().saturating_sub(1) {
        if lines[i] != "#[inline]" {
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
            fns.push((i, name, 1, h.to_string()));
            continue;
        }
        let mut end = i + 2;
        while end < lines.len() && lines[end] != "}" {
            end += 1;
        }
        if end == lines.len() {
            continue;
        }
        fns.push((i, name, std::cmp::max(end - i - 2, 1) as u64, lines[i + 2..end].join("\n")));
    }
    if fns.is_empty() {
        return text;
    }
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (k, f) in fns.iter().enumerate() {
        index.entry(f.1).or_insert(k);
    }
    let sites = inline_calls(&text, &index);
    let edges: Vec<Vec<(usize, u64)>> = fns.iter().map(|f| inline_calls(&f.3, &index).into_iter().collect()).collect();
    let nf = fns.len();
    // recursive: the fn reaches itself through its calls (direct or mutual); it never gets always and adds nothing
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
    // the non-recursive fns form a DAG: incl / always in post-order (iterative, call chains can be deep)
    let mut done = vec![false; nf];
    let mut incl = vec![0u64; nf];
    let mut always = vec![false; nf];
    for root in 0..nf {
        if rec[root] || done[root] {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&(v, k)) = stack.last() {
            if k < edges[v].len() {
                stack.last_mut().unwrap().1 = k + 1;
                let w = edges[v][k].0;
                if !rec[w] && !done[w] {
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
            let calls = sites.get(&v).copied().unwrap_or(0);
            always[v] = tot <= INLINE_SMALL || calls <= 1 || tot.saturating_mul(calls - 1) <= INLINE_BUDGET;
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
}

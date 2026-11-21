//! Apply the shared inline policy to generated LLVM function bodies.
use crate::shared_tables::inline_policy::{INLINE_BUDGET, INLINE_SMALL};

struct Candidate {
    header: usize,
    token: String,
    size: u64,
}

pub(crate) fn llvm_inline(text: String) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut fns = Vec::new();
    let mut bodies = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("define internal ") || !line.ends_with(" {") || line.contains("alwaysinline") {
            continue;
        }
        let Some((_, symbol)) = line.split_once('@') else { continue };
        let Some((name, _)) = symbol.split_once('(') else { continue };
        if !name.ends_with("__pruv_body") {
            continue;
        }
        let Some(size) = lines[i + 1..].iter().position(|line| *line == "}") else { continue };
        fns.push(Candidate { header: i, token: format!("@{name}("), size: size.max(1) as u64 });
        bodies.push(&lines[i + 1..i + 1 + size]);
    }
    if fns.is_empty() {
        return text;
    }

    let sites: Vec<u64> = fns.iter().map(|f| {
        text.matches(&f.token).count() as u64 - lines[f.header].matches(&f.token).count() as u64
    }).collect();
    let mut edges: Vec<Vec<(usize, u64)>> = bodies.iter().map(|body| {
        fns.iter().enumerate().filter_map(|(i, f)| {
            let count = body.iter().map(|line| line.matches(&f.token).count() as u64).sum();
            (count != 0).then_some((i, count))
        }).collect()
    }).collect();

    let nf = fns.len();
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
    for v in 0..nf {
        if rec[v] {
            edges[v].retain(|&(w, _)| !rec[w]);
        }
    }
    // Evaluate the remaining DAG in post-order (iterative, call chains can be deep).
    let mut done = rec;
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
            let mut tot = fns[v].size;
            for &(w, c) in &edges[v] {
                if always[w] {
                    tot = tot.saturating_add(c.saturating_mul(incl[w]));
                }
            }
            incl[v] = tot;
            let calls = sites[v];
            always[v] = tot <= INLINE_SMALL
                || calls == 1
                || (calls >= 2 && tot.saturating_mul(calls - 1) <= INLINE_BUDGET);
        }
    }

    let mut output: Vec<_> = lines.into_iter().map(str::to_owned).collect();
    for (v, f) in fns.iter().enumerate() {
        if always[v] {
            let header = &mut output[f.header];
            header.truncate(header.len() - 2);
            header.push_str(" alwaysinline {");
        }
    }
    output.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llvm_inline_small_and_recursive() {
        let ir = "define internal void @small__pruv_body() {\n  ret void\n}\n\
                  define internal void @recursive__pruv_body() {\n  call void @recursive__pruv_body()\n  ret void\n}\n";
        let actual = llvm_inline(ir.into());
        assert_eq!(actual, ir.replace("@small__pruv_body() {", "@small__pruv_body() alwaysinline {"));
    }

    #[test]
    fn llvm_inline_counts_calls_and_callee_size() {
        let large = "  ret void\n".repeat(INLINE_BUDGET as usize);
        let ir = format!(
            "define internal void @callee__pruv_body() {{\n{large}}}\n\
             define internal void @caller__pruv_body() {{\n  call void @callee__pruv_body()\n}}\n\
             define void @entry() {{\n  call void @caller__pruv_body()\n  call void @caller__pruv_body()\n}}\n"
        );
        assert_eq!(llvm_inline(ir.clone()), ir.replace("@callee__pruv_body() {", "@callee__pruv_body() alwaysinline {"));
    }
}

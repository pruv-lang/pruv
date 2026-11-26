//! P9a-6 (bench/l3/lib/RESHAPE.md row VP): literal arms over an `i64` scrutinee lower to one `if`/`else if` chain, so
//! the verifier follows N + 1 paths, not the 2^N of N independent `if` statements. The program is the row's minimal
//! reproducer (a recursive walk whose loop body matches the read byte after the recursive call); with sixteen
//! independent `if`s it took 147 s, with an `else if` chain about 1 s. These tests pin that the statement and the
//! value `match` with 24 arms verify, that the obligation count grows linearly with the arms, and (with a generous
//! wall-clock bound) that the verification stays fast.

use pruv_proof::obligation::Trace;
use pruv_proof::verify_program;
use std::time::{Duration, Instant};

fn walk_src(arms: usize, value: bool) -> String {
    let mut m = String::new();
    if value {
        m.push_str("    let w: i64 = match k {\n");
        for i in 1..=arms {
            m.push_str(&format!("      {i} => v + {i},\n"));
        }
        m.push_str("      _ => v,\n    };\n    c.f0 = w;\n");
    } else {
        m.push_str("    match k {\n");
        for i in 1..=arms {
            m.push_str(&format!("      {i} => {{ c.f{} = v + {i}; }}\n", i % 4));
        }
        m.push_str("      _ => {}\n    }\n");
    }
    format!(
        "struct C {{ f0: i64, f1: i64, f2: i64, f3: i64 }}

fn walk(b: [u8], p: i64, d: i64, inout c: C) -> i64
  requires 0 <= p && p <= len(b) && 0 <= d
  ensures result == -1 || (p <= result && result <= len(b))
  decreases d
{{
  if d == 0 {{ return p; }}
  let mut q: i64 = p;
  while q < len(b)
    invariant p <= q && q <= len(b)
    decreases len(b) - q
  {{
    let k: i64 = b[q];
    let v: i64 = walk(b, q + 1, d - 1, inout c);
    if v < 0 {{ return -1; }}
{m}    if v <= q {{ return -1; }}
    q = v;
  }}
  return q;
}}
"
    )
}

/// (obligation count, elapsed) of verifying the program; every obligation must be proved.
fn verify(src: &str) -> (usize, Duration) {
    let mut prog = pruv_core::parse(src).expect("parse");
    let tt = pruv_core::check::check(&mut prog).expect("check");
    let mut trace = Trace::adapter();
    let t0 = Instant::now();
    let errs = verify_program(&prog, &tt, Some(&mut trace)).expect("verify");
    let dt = t0.elapsed();
    assert!(errs.is_empty(), "verification failed: {:?}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>());
    (trace.obligations.len(), dt)
}

fn check_linear(value: bool) {
    let (small, _) = verify(&walk_src(8, value));
    let (big, dt) = verify(&walk_src(24, value));
    // 16 more arms add at most a fixed number of obligations per arm (an exponential path count would add thousands)
    assert!(big <= small + 16 * 8, "obligations grow faster than the arms: 8 arms {small}, 24 arms {big}");
    // the 16-arm chain of `if`s took 147 s; one chain of 24 arms takes well under a second on a laptop
    assert!(dt < Duration::from_secs(60), "24 literal arms took {dt:?} to verify");
}

#[test]
fn statement_match_24_arms_is_one_chain() {
    check_linear(false);
}

#[test]
fn value_match_24_arms_is_one_chain() {
    check_linear(true);
}

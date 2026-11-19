#![allow(dead_code, unused_imports, unused_variables)]
// M6 harness. run_m6.py replaces @SOL@ with the path of the compiled root (sol.rs) and builds this with rustc -O.
// Input: one line per case, `op w1 w2 ...` with decimal i64 words; a string is `len u1 .. ulen` (UTF-16 units).
// Output: one line per case with the result words (decimal), or `PANIC <message>` if the Pruv entry panicked
// (e.g. a `requires` check at the host entry). Perf ops print `ns <elapsed ns of the Pruv call> <checksum>`.
// Ops prefixed `hand_` run the same perf op on the hand Rust baseline (hand.rs, linked into this binary so both lanes
// share one process image and their peak RSS compares the helpers, not two executables).
// Calling convention (SPEC.md section 2): [i64] params are &[i64], [i64] results are Vec<i64>, bool is bool, i64 is i64.
#[path = "@SOL@"] mod sol;
#[path = "@HAND@"] mod hand;
use std::fmt::Display;
use std::io::{self, BufRead};
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

fn words(v: &[impl Display]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")
}

// the string starting at word p: returns its units
fn take_str(w: &[i64], p: usize) -> Vec<i64> {
    let n = w[p] as usize;
    w[p + 1..p + 1 + n].to_vec()
}

fn timed(f: impl FnOnce() -> i64) -> String {
    let t = Instant::now();
    let c = f();
    let ns = t.elapsed().as_nanos();
    format!("ns {} {}", ns, c)
}

fn run(op: &str, w: &[i64]) -> String {
    match op {
        "cls" => words(&sol::t_class(w[0])),
        "oki" => sol::t_okint(w[0]).to_string(),
        "sfe" => sol::t_issafe(w[0]).to_string(),
        "ari" => words(&sol::t_arith(w[0], w[1], w[2])),
        "fdv" => sol::t_fdiv(w[0], w[1]).to_string(),
        "min" => sol::t_minutes(w[0]).to_string(),
        "int" => words(&sol::t_int(w[0])),
        "iso" => words(&sol::t_iso(w[0])),
        "dok" => sol::t_dok(w[0]).to_string(),
        "ref" => words(&sol::t_refusal(w[0], w[1], w[2], w[3], w[4])),
        "con" => words(&sol::t_continue(w[0], w[1], w[2], w[3], w[4])),
        "amt" => words(&sol::t_amount(w[0], w[1], w[2], w[3], w[4])),
        "lmt" => words(&sol::t_limit(w[0], w[1], w[2])),
        "wir" => {
            let mut args = [0; 2];
            let key = sol::t_wire(w[0], w[1], w[2], &mut args[..]);
            format!("{} {}", words(&key), words(&args))
        }
        "cmi" => words(&sol::t_card_missing(w[0])),
        "cer" => words(&sol::t_card_error(w[0], w[1])),
        "cnt" => words(&sol::t_counter(w[0], w[1])),
        "nol" => words(&sol::t_nolimit(w[0])),
        "lng" => {
            let s = take_str(w, 1);
            sol::t_lang(w[0], &s).to_string()
        }
        "lex" => {
            let s = take_str(w, 0);
            sol::t_lang_excl(&s).to_string()
        }
        "len" => {
            let s = take_str(w, 0);
            words(&sol::t_len(&s))
        }
        "p_iso" => timed(|| sol::p_iso(w[0], w[1])),
        "p_ref" => timed(|| sol::p_refusal(w[0], w[1], w[2], w[3], w[4], w[5])),
        "p_amount" => timed(|| sol::p_amount(w[0], w[1], w[2], w[3], w[4], w[5])),
        "p_card_error" => timed(|| sol::p_card_error(w[0], w[1], w[2])),
        "p_int" => timed(|| sol::p_int(w[0], w[1])),
        _ if op.starts_with("hand_") => timed(|| hand::run(&op[5..], w)),
        _ => panic!("unknown op {}", op),
    }
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut it = line.split_whitespace();
        let op = match it.next() {
            Some(o) => o.to_string(),
            None => continue,
        };
        let w: Vec<i64> = it.map(|x| x.parse().unwrap()).collect();
        match panic::catch_unwind(AssertUnwindSafe(|| run(&op, &w))) {
            Ok(s) => println!("{}", s),
            Err(e) => {
                let m = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "?".to_string()
                };
                println!("PANIC {}", m.replace('\n', " "));
            }
        }
    }
}

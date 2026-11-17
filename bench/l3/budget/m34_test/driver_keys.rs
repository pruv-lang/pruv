// Keys-lane driver (appended to a main file that already declares `mod sol;`, see run.py).
//
// Reads one command per line from stdin and prints one result line per command. All numbers are decimal i64.
// Tokens are whitespace separated; `|` separates buffers. The driver makes no decisions: it parses, calls sol::kc_*,
// prints. Empty lines are skipped (and print nothing). Keep this protocol identical to gen_keys.py.
//
//   rec P W...                  buffer = W...                      -> rec_end(buffer, P)
//   ok KS N W...                buffer = W...                      -> 1 / 0
//   unit KS N J W...            buffer = W...                      -> unit
//   hash SEED KS N W...         buffer = W...                      -> hash
//   hash_units SEED S N U...    buffer = units U...                -> packed-equivalent hash
//   eq SA SB N | A... | B...    buffers A and B                    -> 1 / 0
//   cmp SA NA SB NB | A... | B...                                  -> -1 / 0 / 1
//   pack D S N DLEN | SRC...    dst = DLEN words of 7, src = SRC   -> `STATUS dst...`
//   unpack D KS N DLEN | K...   dst = DLEN words of 7, k = K       -> `STATUS dst...`
use std::io::{self, BufRead, Write};

const FILLER: i64 = 7;

fn num(t: &str) -> i64 {
    t.parse::<i64>().unwrap_or_else(|_| {
        eprintln!("bad number {:?}", t);
        std::process::exit(2);
    })
}

// Groups of numbers separated by `|` tokens (group 0 is everything before the first bar).
fn groups(toks: &[&str]) -> Vec<Vec<i64>> {
    let mut g: Vec<Vec<i64>> = vec![Vec::new()];
    for t in toks {
        if *t == "|" {
            g.push(Vec::new());
        } else {
            g.last_mut().unwrap().push(num(t));
        }
    }
    g
}

fn need(g: &[Vec<i64>], heads: usize, bufs: usize, cmd: &str) {
    if g.len() != 1 + bufs || g[0].len() < heads {
        eprintln!("bad arity for {}", cmd);
        std::process::exit(2);
    }
}

fn with_dst(status: i64, dst: &[i64]) -> String {
    let mut s = status.to_string();
    for x in dst {
        s.push(' ');
        s.push_str(&x.to_string());
    }
    s
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.is_empty() {
            continue;
        }
        let g = groups(&toks[1..]);
        match toks[0] {
            "words" => {
                need(&g, 1, 0, "words");
                writeln!(w, "{}", sol::kc_words(g[0][0])).unwrap();
            }
            "rec" => {
                need(&g, 1, 0, "rec");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_rec_end(&h[1..], h[0])).unwrap();
            }
            "ok" => {
                need(&g, 2, 0, "ok");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_slice_ok(&h[2..], h[0], h[1])).unwrap();
            }
            "unit" => {
                need(&g, 3, 0, "unit");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_unit(&h[3..], h[0], h[1], h[2])).unwrap();
            }
            "hash" => {
                need(&g, 3, 0, "hash");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_hash(h[0], &h[3..], h[1], h[2])).unwrap();
            }
            "hash_units" => {
                need(&g, 3, 0, "hash_units");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_hash_units(h[0], &h[3..], h[1], h[2])).unwrap();
            }
            "eq" => {
                need(&g, 3, 2, "eq");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_eq(&g[1], h[0], &g[2], h[1], h[2])).unwrap();
            }
            "cmp" => {
                need(&g, 4, 2, "cmp");
                let h = &g[0];
                writeln!(w, "{}", sol::kc_cmp(&g[1], h[0], h[1], &g[2], h[2], h[3])).unwrap();
            }
            "pack" => {
                need(&g, 4, 1, "pack");
                let h = &g[0];
                let mut dst: Vec<i64> = vec![FILLER; h[3].max(0) as usize];
                let st = sol::kc_pack(&mut dst, h[0], &g[1], h[1], h[2]);
                writeln!(w, "{}", with_dst(st, &dst)).unwrap();
            }
            "unpack" => {
                need(&g, 4, 1, "unpack");
                let h = &g[0];
                let mut dst: Vec<i64> = vec![FILLER; h[3].max(0) as usize];
                let st = sol::kc_unpack(&mut dst, h[0], &g[1], h[1], h[2]);
                writeln!(w, "{}", with_dst(st, &dst)).unwrap();
            }
            other => {
                eprintln!("unknown command {}", other);
                std::process::exit(2);
            }
        }
    }
    w.flush().unwrap();
}

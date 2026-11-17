#![allow(dead_code, unused_imports, unused_variables)]
// M11 harness. run_m11.py replaces @SOL@ / @HAND@ with paths and builds this with rustc -O.
// The host only moves words: it owns the store buffer st (never reads its words), grows it on status 1 and retries the
// same call, and prints result words. Every judgment is in the Pruv entries (root.pruv / bud_card.pruv).
//
// Input lines (decimal words; `|` separates arguments; an empty argument = absent):
//   init                               fresh world
//   load <key> | <budget V>            session with that budget (empty V = no budget); key = packed `n, words(n)`
//   set <now> | <ref> | <sid> | <card> | <lang> | <by>   sid = packed key of ref.sessionId (empty = none)
//   q <op> | <ref> | <sid>             op 1 current, 2 isHalted, 3 hasCard
// Keys are packed u16 (bud_keys): word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3], tail lanes zero; the driver
// (run_m11.py pack) packs each key once, the host never packs or unpacks inside a call.
//   dump
//   perfA <U> <reps> | perfB <n> <reps> | perfC <n> <reps> | handA / handB / handC (same args)   ->  `ns <ns per call> <checksum>`
// Output: `ok <words>` (the entry's out[1..1+out[0]]), `err <status>`, or `PANIC <msg>`.
#[path = "@SOL@"] mod sol;
#[path = "@HAND@"] mod hand;
use std::io::{self, BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;
use std::hint::black_box;

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
}

impl World {
    fn new() -> World {
        let mut st = vec![0i64; 4096];
        let r = sol::m11_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            assert_eq!(sol::m11_init(&mut st), 0);
        }
        World { st, out: vec![0; 4096] }
    }
    fn grow(&mut self) -> bool {
        let n = self.out[0].max(0) as usize;
        let mut fresh = vec![0i64; n.max(2 * self.st.len())];
        if sol::m11_grow(&self.st, &mut fresh) != 0 {
            return false;
        }
        self.st = fresh;
        true
    }
    // runs f with retries on Grow (1) and out too small (3)
    fn call(&mut self, f: &dyn Fn(&mut Vec<i64>, &mut Vec<i64>) -> i64) -> Result<&[i64], i64> {
        for _ in 0..6 {
            let s = f(&mut self.st, &mut self.out);
            match s {
                0 => {
                    let k = (self.out[0].max(0) as usize).min(self.out.len() - 1);
                    return Ok(&self.out[1..1 + k]);
                }
                1 => {
                    if !self.grow() {
                        return Err(1);
                    }
                }
                3 => {
                    let n = 2 * self.out.len();
                    self.out.resize(n, 0);
                }
                s => return Err(s),
            }
        }
        Err(9)
    }
}

fn words(v: &[i64]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")
}

fn res(r: Result<&[i64], i64>) -> String {
    match r {
        Ok(w) => format!("ok {}", words(w)),
        Err(s) => format!("err {}", s),
    }
}

fn args(rest: &str) -> Vec<Vec<i64>> {
    rest.split('|').map(|p| p.split_whitespace().map(|x| x.parse::<i64>().unwrap()).collect()).collect()
}

// ---- perf inputs (host builds input words; no judgment) ----

// packed-u16 words of units (same layout as run_m11.py pack / driver_m8.rs pack_u16)
fn pack_u16(units: &[u16]) -> Vec<i64> {
    units
        .chunks(4)
        .map(|c| {
            let mut x: u64 = 0;
            for l in 0..4 {
                x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
            }
            x as i64
        })
        .collect()
}
// framed packed key `n, words(n)` of a string
fn pkey(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    v.extend(pack_u16(&u));
    v
}

fn key(o: &mut Vec<i64>, k: &str) {
    o.push(k.len() as i64);
    o.extend(k.bytes().map(|b| b as i64));
}
fn sv(o: &mut Vec<i64>, s: &str) {
    o.push(6);
    o.push(s.encode_utf16().count() as i64);
    o.extend(s.encode_utf16().map(|u| u as i64));
}
fn perf_card(u: usize) -> Vec<i64> {
    let line = format!("  {}  ", "a".repeat(u));
    let mut o = vec![9, 2];
    key(&mut o, "deliverable");
    sv(&mut o, &line);
    key(&mut o, "done");
    o.extend_from_slice(&[8, 5]);
    for _ in 0..5 {
        sv(&mut o, &line);
    }
    o
}
fn perf_ref() -> Vec<i64> {
    let mut o = vec![9, 2];
    key(&mut o, "sessionId");
    sv(&mut o, "s1");
    key(&mut o, "budgetId");
    sv(&mut o, "b1");
    o
}
fn perf_budget() -> Vec<i64> {
    let mut o = vec![9, 3];
    key(&mut o, "id");
    sv(&mut o, "b1");
    key(&mut o, "state");
    sv(&mut o, "running");
    key(&mut o, "turnCount");
    o.extend_from_slice(&[5, 3]);
    o
}
const NOW0: i64 = 1_700_000_000_000;

// a card with every optional field, for the hand-vs-Pruv word equality check
fn full_card() -> Vec<i64> {
    let mut o = perf_card(5);
    o[1] = 6;
    key(&mut o, "targets");
    o.extend_from_slice(&[9, 4]);
    key(&mut o, "confirmed");
    o.extend_from_slice(&[5, 1]);
    key(&mut o, "candidate");
    o.extend_from_slice(&[5, 2]);
    key(&mut o, "unknown");
    o.extend_from_slice(&[5, 0]);
    key(&mut o, "note");
    sv(&mut o, " n ");
    key(&mut o, "verify");
    o.extend_from_slice(&[8, 1, 9, 2]);
    key(&mut o, "label");
    sv(&mut o, " x ");
    key(&mut o, "status");
    sv(&mut o, "done");
    key(&mut o, "conflicts");
    o.extend_from_slice(&[8, 1]);
    sv(&mut o, " c ");
    key(&mut o, "state");
    sv(&mut o, "working");
    o
}

// hand.rs must produce the same result words as the Pruv entry (same work); prints `same` or `differ ...`
fn hand_check() -> String {
    let rf = perf_ref();
    let sid = pkey("s1");
    let mut lines = Vec::new();
    for card in [perf_card(20), full_card()] {
        let mut w = World::new();
        let k: Vec<i64> = pkey("s1");
        let b = perf_budget();
        w.call(&|st, out| sol::m11_load(st, &k, &b, out)).unwrap();
        let mut s = hand::Store { sessions: std::collections::HashMap::new() };
        s.sessions.insert(pkey("s1"), perf_budget());
        let mut ho: Vec<i64> = Vec::new();
        for i in 0..3 {
            let p = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, &card, &[], &[], NOW0 + i, out)).unwrap().to_vec();
            let hk = s.set_card(&rf, &sid, &card, NOW0 + i, &mut ho);
            if hk != -1 || ho.len() != p.len() + 1 || ho[0] as usize != p.len() || ho[1..] != p[..] {
                lines.push(format!("differ card={} call={} pruv={} hand={}", card.len(), i, words(&p), words(&ho)));
            }
        }
    }
    if lines.is_empty() { "same".to_string() } else { lines.join("\n") }
}

fn perf(op: &str, a: usize, reps: usize) -> String {
    match op {
        "perfA" | "handA" => {
            let card = perf_card(a);
            let t = Instant::now();
            let mut sum = 0i64;
            for i in 0..reps {
                let c = if op == "perfA" {
                    sol::m11_perf_card(black_box(&card), NOW0 + i as i64)
                } else {
                    let k = hand::validate(black_box(&card), 0);
                    if k != 0 { -k } else {
                        let w = hand::build(&card, 0, None, NOW0 + i as i64);
                        let n = w.len();
                        n as i64 + (w[n - 1] & 1023) + (w[n / 2] & 1023)
                    }
                };
                sum = sum.wrapping_add(black_box(c));
            }
            format!("ns {:.1} {}", t.elapsed().as_nanos() as f64 / reps as f64, sum)
        }
        "perfB" | "handB" => {
            // reps rounds of n successive setCard calls on one budget; ns per call over all rounds
            let rf = perf_ref();
            let sid = pkey("s1");
            let card = perf_card(20);
            let mut total = 0u128;
            let mut sum = 0i64;
            for _ in 0..reps {
                if op == "perfB" {
                    let mut w = World::new();
                    let key: Vec<i64> = pkey("s1");
                    let b = perf_budget();
                    w.call(&|st, out| sol::m11_load(st, &key, &b, out)).unwrap();
                    let t = Instant::now();
                    for i in 0..a {
                        let now = NOW0 + i as i64;
                        let r = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, &card, &[], &[], now, out)).unwrap();
                        sum = sum.wrapping_add(black_box(r.len() as i64 + r[2]));
                    }
                    total += t.elapsed().as_nanos();
                } else {
                    let mut s = hand::Store { sessions: std::collections::HashMap::new() };
                    s.sessions.insert(pkey("s1"), perf_budget());
                    let mut out: Vec<i64> = Vec::with_capacity(4096);
                    let t = Instant::now();
                    for i in 0..a {
                        let k = s.set_card(black_box(&rf), &sid, &card, NOW0 + i as i64, &mut out);
                        sum = sum.wrapping_add(black_box(k + out.len() as i64 + out[2]));
                    }
                    total += t.elapsed().as_nanos();
                }
            }
            format!("ns {:.1} {}", total as f64 / (a * reps) as f64, sum)
        }
        "perfC" | "handC" => {
            // diagnostic split of B: n successive setCard judgments on one in-memory budget (no store, no out words)
            let card = perf_card(20);
            let mut total = 0u128;
            let mut sum = 0i64;
            for _ in 0..reps {
                let b = perf_budget();
                let t = Instant::now();
                if op == "perfC" {
                    sum = sum.wrapping_add(black_box(sol::m11_perf_chain(black_box(&b), &card, a as i64, NOW0)));
                } else {
                    let mut cur = b;
                    for i in 0..a {
                        if hand::validate(black_box(&card), 0) != 0 { break; }
                        let c = hand::build(&card, 0, None, NOW0 + i as i64);
                        cur = hand::put_card(&cur, &c);
                    }
                    sum = sum.wrapping_add(black_box(cur.len() as i64));
                }
                total += t.elapsed().as_nanos();
            }
            format!("ns {:.1} {}", total as f64 / (a * reps) as f64, sum)
        }
        _ => "bad".to_string(),
    }
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut world: Option<World> = None;
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (cmd, rest) = match line.split_once(' ') {
            Some((c, r)) => (c.to_string(), r.to_string()),
            None => (line.trim().to_string(), String::new()),
        };
        if cmd.is_empty() {
            continue;
        }
        let r = panic::catch_unwind(AssertUnwindSafe(|| -> String {
            match cmd.as_str() {
                "init" => {
                    world = Some(World::new());
                    "ok".to_string()
                }
                "load" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    res(w.call(&|st, out| sol::m11_load(st, &a[0], &a[1], out)))
                }
                "set" => {
                    let a = args(&rest);
                    let now = a[0][0];
                    let w = world.as_mut().unwrap();
                    res(w.call(&|st, out| sol::m11_set_card(st, &a[1], &a[2], &a[3], &a[4], &a[5], now, out)))
                }
                "q" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    let op = a[0][0];
                    res(w.call(&|st, out| sol::m11_query(st, op, &a[1], &a[2], out)))
                }
                "dump" => {
                    let w = world.as_mut().unwrap();
                    res(w.call(&|st, out| sol::m11_dump(st, out)))
                }
                "handcheck" => hand_check(),
                p => {
                    let a = args(&rest);
                    perf(p, a[0][0] as usize, a[0][1] as usize)
                }
            }
        }));
        match r {
            Ok(s) => writeln!(o, "{}", s).unwrap(),
            Err(e) => {
                let m = if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
                    else if let Some(s) = e.downcast_ref::<String>() { s.clone() } else { "?".to_string() };
                writeln!(o, "PANIC {}", m.replace('\n', " ")).unwrap();
            }
        }
        o.flush().unwrap();
    }
}

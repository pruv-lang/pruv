#![allow(dead_code, unused_imports, unused_variables)]
// M11 harness. run_m11.py replaces @SOL@ / @HAND@ with paths and builds this with rustc -O.
// The host only moves words: it owns the store buffer st (never reads its words), grows it on status 1 and retries the
// same call, and prints result words. Every judgment is in the Pruv entries (root.pruv / bud_card.pruv).
//
// Input lines (decimal words; `|` separates arguments; an empty argument = absent):
//   init                               fresh world
//   load <key> | <budget V>            session with that budget (empty V = no budget); key = packed `n, words(n)`
//   set <now> | <ref> | <sid> | <card> | <lang> | <by>   sid = packed key of ref.sessionId (empty = none);
//       card / lang / by = JB byte packets as decimal words 0..255 (hand.rs header; empty = absent)
//   q <op> | <ref> | <sid>             op 1 current, 2 isHalted, 3 hasCard
// Keys are packed u16 (bud_keys): word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3], tail lanes zero; the driver
// (run_m11.py pack) packs each key once, the host never packs or unpacks inside a call.
//   dump
//   perfA <U> <reps> | perfB/C/R/H/I <n> <reps> | handA/B/C/R/H/I (same args) -> `ns <ns per call> <checksum>`
//   R alternates card sizes, H queries hasCard, I pairs setCard with current.
//   handcheck | hdiff <card> | <lang> | <by> [| <scen 0..4>]  compare hand and Pruv result words
// Output: `ok <words>` (the entry's out[1..1+out[0]]), `err <status>`, or `PANIC <msg>`.
#[cfg(not(m11_typed))]
#[path = "@SOL@"] mod sol;
#[cfg(m11_typed)]
#[path = "@SOL@"] mod generated;
#[cfg(m11_typed)]
#[path = "@TYPED@"] mod sol;
#[path = "@HAND@"] mod hand;
use std::io::{self, BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;
use std::hint::black_box;

#[cfg(not(m11_typed))]
type State = Vec<i64>;
#[cfg(m11_typed)]
type State = generated::St;

struct World {
    st: State,
    out: Vec<i64>,
}

impl World {
    #[cfg(not(m11_typed))]
    fn new() -> World {
        let mut st = vec![0i64; 4096];
        let r = sol::m11_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            assert_eq!(sol::m11_init(&mut st), 0);
        }
        World { st, out: vec![0; 4096] }
    }
    #[cfg(m11_typed)]
    fn new() -> World {
        World { st: sol::new_state(), out: vec![0; 4096] }
    }
    #[cfg(not(m11_typed))]
    fn grow(&mut self) -> bool {
        let n = self.out[0].max(0) as usize;
        let mut fresh = vec![0i64; n]; // N from Pruv (bud_store::need, N >= 2*len), L3 P13b
        if sol::m11_grow(&self.st, &mut fresh) != 0 {
            return false;
        }
        self.st = fresh;
        true
    }
    // runs f with retries on Grow (1) and out too small (3)
    fn call(&mut self, f: &dyn Fn(&mut State, &mut Vec<i64>) -> i64) -> Result<&[i64], i64> {
        for _ in 0..6 {
            let s = f(&mut self.st, &mut self.out);
            match s {
                0 => {
                    let k = (self.out[0].max(0) as usize).min(self.out.len() - 1);
                    return Ok(&self.out[1..1 + k]);
                }
                1 => {
                    #[cfg(m11_typed)]
                    return Err(1);
                    #[cfg(not(m11_typed))]
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
// ---- JB byte packets (big-endian; see hand.rs header): 6 str, 7 f64 bits, 8 array, 9 object ----
fn bkey(o: &mut Vec<u8>, k: &str) {
    o.extend_from_slice(&(k.len() as u32).to_be_bytes());
    o.extend_from_slice(k.as_bytes());
}
fn bstr(o: &mut Vec<u8>, s: &str) {
    o.push(6);
    bkey(o, s);
}
fn bnum(o: &mut Vec<u8>, x: f64) {
    o.push(7);
    o.extend_from_slice(&x.to_be_bytes());
}
fn bhead(o: &mut Vec<u8>, tag: u8, n: u32) {
    o.push(tag);
    o.extend_from_slice(&n.to_be_bytes());
}
// the harness input words of a byte argument: each word must be 0..255
fn bytes_of(w: &[i64]) -> Vec<u8> {
    w.iter().map(|&x| u8::try_from(x).expect("byte word outside 0..255")).collect()
}
fn perf_card(u: usize) -> Vec<u8> {
    let line = format!("  {}  ", "a".repeat(u));
    let mut o = Vec::new();
    bhead(&mut o, 9, 2);
    bkey(&mut o, "deliverable");
    bstr(&mut o, &line);
    bkey(&mut o, "done");
    bhead(&mut o, 8, 5);
    for _ in 0..5 {
        bstr(&mut o, &line);
    }
    o
}
fn minimal_card() -> Vec<u8> {
    let mut o = Vec::new();
    bhead(&mut o, 9, 2);
    bkey(&mut o, "deliverable");
    bstr(&mut o, "a");
    bkey(&mut o, "done");
    bhead(&mut o, 8, 1);
    bstr(&mut o, "a");
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
fn full_card() -> Vec<u8> {
    let line = format!("  {}  ", "a".repeat(5));
    let mut o = Vec::new();
    bhead(&mut o, 9, 6);
    bkey(&mut o, "deliverable");
    bstr(&mut o, &line);
    bkey(&mut o, "done");
    bhead(&mut o, 8, 5);
    for _ in 0..5 {
        bstr(&mut o, &line);
    }
    bkey(&mut o, "targets");
    bhead(&mut o, 9, 4);
    bkey(&mut o, "confirmed");
    bnum(&mut o, 1.0);
    bkey(&mut o, "candidate");
    bnum(&mut o, 2.0);
    bkey(&mut o, "unknown");
    bnum(&mut o, 0.0);
    bkey(&mut o, "note");
    bstr(&mut o, " n ");
    bkey(&mut o, "verify");
    bhead(&mut o, 8, 1);
    bhead(&mut o, 9, 2);
    bkey(&mut o, "label");
    bstr(&mut o, " x ");
    bkey(&mut o, "status");
    bstr(&mut o, "done");
    bkey(&mut o, "conflicts");
    bhead(&mut o, 8, 1);
    bstr(&mut o, " c ");
    bkey(&mut o, "state");
    bstr(&mut o, "working");
    o
}

// a `by` object {sessionId: "s1", turnId: 7}
fn perf_by() -> Vec<u8> {
    let mut o = Vec::new();
    bhead(&mut o, 9, 2);
    bkey(&mut o, "sessionId");
    bstr(&mut o, "s1");
    bkey(&mut o, "turnId");
    bnum(&mut o, 7.0);
    o
}

// hand.rs must produce the same result words as the Pruv entry (same work); prints `same` or `differ ...`
fn hand_check() -> String {
    let rf = perf_ref();
    let sid = pkey("s1");
    let mut lines = Vec::new();
    let none: Vec<u8> = Vec::new();
    let en: Vec<u8> = { let mut o = Vec::new(); bstr(&mut o, "en"); o };
    for (card, lang, by) in [(perf_card(20), none.clone(), none.clone()), (full_card(), none.clone(), none.clone()), (full_card(), en, perf_by())] {
        let mut w = World::new();
        let k: Vec<i64> = pkey("s1");
        let b = perf_budget();
        w.call(&|st, out| sol::m11_load(st, &k, &b, out)).unwrap();
        let mut s = hand::Store { sessions: std::collections::HashMap::new() };
        s.sessions.insert(pkey("s1"), perf_budget());
        let mut ho: Vec<i64> = Vec::new();
        for i in 0..3 {
            let p = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, &card, &lang, &by, NOW0 + i, out)).unwrap().to_vec();
            let hk = s.set_card(&rf, &sid, &card, &lang, &by, NOW0 + i, &mut ho);
            if hk != -1 || ho.len() != p.len() + 1 || ho[0] as usize != p.len() || ho[1..] != p[..] {
                lines.push(format!("differ card={} call={} pruv={} hand={}", card.len(), i, words(&p), words(&ho)));
            }
        }
    }
    if lines.is_empty() { "same".to_string() } else { lines.join("\n") }
}

fn perf_resident(op: &str, n: usize, reps: usize) -> String {
    let hand = op.starts_with("hand");
    let has = op.ends_with('H');
    let read_back = op.ends_with('I');
    let rf = perf_ref();
    let sid = pkey("s1");
    let mut card = perf_card(20);
    let mut short = None;
    if op.ends_with('R') {
        // Give the long card an optional field so the short card removes it.
        card[1..5].copy_from_slice(&3u32.to_be_bytes());
        bkey(&mut card, "state");
        bstr(&mut card, "working");
        short = Some(minimal_card());
    }
    let mut total = 0u128;
    let mut sum = 0i64;
    for _ in 0..reps {
        if hand {
            let mut s = hand::Store { sessions: std::collections::HashMap::new() };
            s.sessions.insert(sid.clone(), perf_budget());
            let mut out = Vec::with_capacity(4096);
            if has {
                assert_eq!(s.set_card(&rf, &sid, &card, &[], &[], NOW0, &mut out), -1);
            }
            let t = Instant::now();
            for i in 0..n {
                if has {
                    let found = s.has_card(black_box(&rf), &sid).unwrap();
                    sum = sum.wrapping_add(black_box(3 + i64::from(found)));
                } else {
                    let input = if i % 2 == 0 { &card } else { short.as_ref().unwrap_or(&card) };
                    let kind = s.set_card(black_box(&rf), &sid, input, &[], &[], NOW0 + i as i64, &mut out);
                    assert_eq!(kind, -1);
                    sum = sum.wrapping_add(black_box(out[0] + out[3]));
                    if read_back {
                        assert!(s.current(black_box(&rf), &sid, &mut out));
                        sum = sum.wrapping_add(black_box(out[0] + out[out.len() - 1]));
                    }
                }
            }
            total += t.elapsed().as_nanos();
        } else {
            let mut w = World::new();
            let b = perf_budget();
            w.call(&|st, out| sol::m11_load(st, &sid, &b, out)).unwrap();
            if has {
                let r = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, &card, &[], &[], NOW0, out)).unwrap();
                assert_eq!(r[1], 1);
            }
            let t = Instant::now();
            for i in 0..n {
                if has {
                    let r = w.call(&|st, out| sol::m11_query(st, 3, &rf, &sid, out)).unwrap();
                    sum = sum.wrapping_add(black_box(r[1]));
                } else {
                    let input = if i % 2 == 0 { &card } else { short.as_ref().unwrap_or(&card) };
                    let now = NOW0 + i as i64;
                    let r = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, input, &[], &[], now, out)).unwrap();
                    assert_eq!(r[1], 1);
                    sum = sum.wrapping_add(black_box(r.len() as i64 + r[2]));
                    if read_back {
                        let r = w.call(&|st, out| sol::m11_query(st, 1, &rf, &sid, out)).unwrap();
                        sum = sum.wrapping_add(black_box(r.len() as i64 + r[r.len() - 1]));
                    }
                }
            }
            total += t.elapsed().as_nanos();
        }
    }
    let calls = (n * reps * if read_back { 2 } else { 1 }) as f64;
    format!("ns {:.1} {}", total as f64 / calls, sum)
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
                    let k = if hand::admit(black_box(&card)) { hand::validate(&card) } else { 2 };
                    if k != 0 { -k } else {
                        let w = hand::build(&card, &[], NOW0 + i as i64);
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
                        let k = s.set_card(black_box(&rf), &sid, &card, &[], &[], NOW0 + i as i64, &mut out);
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
                // admitted once outside the timer on both sides: the Pruv chain is handed an admitted card
                if op == "handC" && !hand::admit(black_box(&card)) { return "bad".to_string(); }
                let t = Instant::now();
                if op == "perfC" {
                    sum = sum.wrapping_add(black_box(sol::m11_perf_chain(black_box(&b), &card, a as i64, NOW0)));
                } else {
                    let mut cur = b;
                    for i in 0..a {
                        if hand::validate(black_box(&card)) != 0 { break; }
                        let c = hand::build(&card, &[], NOW0 + i as i64);
                        cur = hand::put_card(&cur, &c);
                    }
                    sum = sum.wrapping_add(black_box(cur.len() as i64));
                }
                total += t.elapsed().as_nanos();
            }
            format!("ns {:.1} {}", total as f64 / (a * reps) as f64, sum)
        }
        "perfR" | "handR" | "perfH" | "handH" | "perfI" | "handI" => perf_resident(op, a, reps),
        _ => "bad".to_string(),
    }
}

// hand-vs-Pruv on one card: the admission / validation kind (or the stored-card checksum) of m11_perf_card against
// hand admit + validate + build, then the whole setCard result words with the given lang and by.
//
// scen picks the stored budget / ref / session key (0 = open budget s1/b1; every other scenario expects error kind 0):
//   1 stored budget is "closed"; 2 ref names session s2 (not stored); 3 ref budgetId b2 (mismatch);
//   4 ref without sessionId and an empty session key.
// Result line: "same err <kind> <en> <used_now>" | "same ok" | "same refused" | "differ ...".
fn diff_ref(scen: i64) -> Vec<i64> {
    let (sess, bid) = match scen { 2 => (Some("s2"), "b1"), 3 => (Some("s1"), "b2"), 4 => (None, "b1"), _ => (Some("s1"), "b1") };
    let mut o = vec![9, if sess.is_some() { 2 } else { 1 }];
    if let Some(s) = sess {
        key(&mut o, "sessionId");
        sv(&mut o, s);
    }
    key(&mut o, "budgetId");
    sv(&mut o, bid);
    o
}
fn diff_sid(scen: i64) -> Vec<i64> {
    match scen { 2 => pkey("s2"), 4 => Vec::new(), _ => pkey("s1") }
}
fn diff_budget(scen: i64) -> Vec<i64> {
    let mut o = vec![9, 3];
    key(&mut o, "id");
    sv(&mut o, "b1");
    key(&mut o, "state");
    sv(&mut o, if scen == 1 { "closed" } else { "running" });
    key(&mut o, "turnCount");
    o.extend_from_slice(&[5, 3]);
    o
}
// the string units of an error V, which must be exactly {ok:false, error:<string>}
fn error_units(v: &[i64]) -> Option<&[i64]> {
    let mut pre = vec![9, 2];
    key(&mut pre, "ok");
    pre.push(3);
    key(&mut pre, "error");
    pre.push(6);
    let rest = v.strip_prefix(pre.as_slice())?;
    let n = usize::try_from(*rest.first()?).ok()?;
    if rest.len() != n + 1 { return None; }
    Some(&rest[1..])
}
fn hand_diff(card: &[u8], lang: &[u8], by: &[u8], scen: i64) -> String {
    let pk = sol::m11_perf_card(card, NOW0);
    let hk = if !hand::admit(card) { -102 } else {
        let k = hand::validate(card);
        if k != 0 { -k } else {
            let w = hand::build(card, &[], NOW0);
            let n = w.len();
            n as i64 + (w[n - 1] & 1023) + (w[n / 2] & 1023)
        }
    };
    if pk != hk { return format!("differ kind pruv={} hand={}", pk, hk); }
    let kind = if scen != 0 { 0 } else if card.is_empty() { 1 } else if pk < 0 { -pk } else { -1 };
    let rf = diff_ref(scen);
    let sid = diff_sid(scen);
    let mut w = World::new();
    let b = diff_budget(scen);
    w.call(&|st, out| sol::m11_load(st, &pkey("s1"), &b, out)).unwrap();
    let mut s = hand::Store { sessions: std::collections::HashMap::new() };
    s.sessions.insert(pkey("s1"), diff_budget(scen));
    let mut ho: Vec<i64> = Vec::new();
    let mut line = String::new();
    // twice: the second call reads the budget the first one stored (a stored card must stay readable)
    for i in 0..2 {
        let p = w.call(&|st, out| sol::m11_set_card(st, &rf, &sid, card, lang, by, NOW0 + i, out)).map(|x| x.to_vec());
        // World::call clamps the length prefix to the buffer; the raw prefix must equal the words returned
        if let Ok(ref q) = p {
            if w.out[0] != q.len() as i64 { return format!("differ prefix call={} pruv={} words={}", i, w.out[0], q.len()); }
        }
        let hk = s.set_card(&rf, &sid, card, lang, by, NOW0 + i, &mut ho);
        match p {
            Err(2) if hk == -2 => return "same refused".to_string(),
            Ok(p) if hk == kind && ho.len() == p.len() + 1 && ho[0] as usize == p.len() && ho[1..] == p[..] => {
                if kind < 0 {
                    line = "same ok".to_string();
                    continue;
                }
                // an error result: counters and the {ok:false, error} V checked explicitly
                if p.len() < 4 || p[0] != ho[1] || p[0] != (hk >= 3) as i64 { return format!("differ used_now pruv={} hand={}", p.first().copied().unwrap_or(-1), ho[1]); }
                if p[1] != 0 || p[2] != 0 || ho[2] != 0 || ho[3] != 0 {
                    return format!("differ saved/changed pruv={},{} hand={},{}", p[1], p[2], ho[2], ho[3]);
                }
                match (error_units(&p[4..]), error_units(&ho[5..])) {
                    (Some(a), Some(c)) if a == c => {}
                    _ => return format!("differ error value call={} pruv={} hand={}", i, words(&p[4..]), words(&ho[5..])),
                }
                line = format!("same err {} {} {}", hk, hand::lang_code(lang), p[0]);
            }
            p => return format!("differ words call={} pruv={:?} hand={} {}", i, p.map(|x| words(&x)), hk, words(&ho)),
        }
    }
    line
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
                #[cfg(m11_typed)]
                "malformed-load" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    format!("err {}", sol::malformed_load(&mut w.st, &a[0], &a[1]))
                }
                #[cfg(m11_typed)]
                "native-id" => {
                    let w = world.as_mut().unwrap();
                    let (id, sid) = rest.split_once(' ').unwrap();
                    let mut out = Vec::new();
                    assert_eq!(sol::query_text(&w.st, 1, Some(id), Some(sid), &mut out), 0);
                    assert_ne!(out, [2, 1, 1]);
                    assert_eq!(sol::set_text(&mut w.st, Some(id), Some(sid),
                        &full_card(), &[], &perf_by(), NOW0, &mut out), 0);
                    assert_eq!(out[3], 1);
                    "same".to_string()
                }
                "set" => {
                    let a = args(&rest);
                    let now = a[0][0];
                    let w = world.as_mut().unwrap();
                    let (card, lang, by) = (bytes_of(&a[3]), bytes_of(&a[4]), bytes_of(&a[5]));
                    res(w.call(&|st, out| sol::m11_set_card(st, &a[1], &a[2], &card, &lang, &by, now, out)))
                }
                "q" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    let op = a[0][0];
                    res(w.call(&|st, out| sol::m11_query(st, op, &a[1], &a[2], out)))
                }
                #[cfg(m11_typed)]
                "set-id" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    let id = &a[1];
                    let (card, lang, by) = (bytes_of(&a[3]), bytes_of(&a[4]), bytes_of(&a[5]));
                    res(w.call(&|st, out| sol::set_id(st, id, &a[2], &card, &lang, &by, a[0][0], out)))
                }
                #[cfg(m11_typed)]
                "q-id" => {
                    let a = args(&rest);
                    let w = world.as_mut().unwrap();
                    let id = &a[1];
                    res(w.call(&|st, out| sol::query_id(st, a[0][0], id, &a[2], out)))
                }
                "dump" => {
                    let w = world.as_mut().unwrap();
                    res(w.call(&|st, out| sol::m11_dump(st, out)))
                }
                "handcheck" => hand_check(),
                "hdiff" => {
                    let a = args(&rest);
                    let scen = a.get(3).and_then(|p| p.first()).copied().unwrap_or(0);
                    hand_diff(&bytes_of(&a[0]), &bytes_of(&a[1]), &bytes_of(&a[2]), scen)
                }
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

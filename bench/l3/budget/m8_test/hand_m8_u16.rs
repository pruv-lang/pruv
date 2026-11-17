// Hand-written safe idiomatic Rust parity baseline for M8 with the SAME semantics as the Pruv implementation
// (bud_charge.pruv + bud_tools.pruv + root_m8.pruv behind driver_m8.rs), replacing hand_m8.rs as the parity bar
// (hand_m8.rs stays as an informational row: UTF-8 String keys, no safe-integer refusal).
//
//   hand_m8_u16 perf <N> <reps> [item]   prints `item size ns_per_call`   (same items/shapes as driver_m8.rs run_perf)
//   hand_m8_u16 q                        stdin program as driver_m8.rs run_q, byte-identical output lines
//
// Differences from hand_m8.rs:
//  * keys (sessionId, budgetId, turnId, activityId) are UTF-16 code units: Box<[u16]> in HashMaps with the std default
//    hasher, looked up by &[u16]. Lone surrogates are plain units; no UTF-8 conversion anywhere.
//  * safe-integer domain: operands and results of every counter addition must lie in +-(2^53 - 1), else the call is
//    refused with ResourceBound (status 5) and nothing is written (atomic), as in bud_charge.pruv `sum`.
//  * sessions hold the typed Budget decoded once at put time from the PLACEHOLDER record words (root_m8.pruv header).
//
// Order observation: Pruv observes sessions, turns and activities in insertion order. Sessions are never deleted, so a
// Vec of keys beside the ordinary HashMap is enough (a put over an existing key keeps its position). Turns and
// activities get an insertion sequence number stored in the map value; the observation sorts by it (turn forget +
// re-add moves a turn to the end, as in Pruv). The hot paths therefore stay plain HashMap lookups / inserts.
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// ---- safe-integer arithmetic (bud_charge.pruv `sum`) ----

const MAX_SAFE: i64 = 9_007_199_254_740_991; // 2^53 - 1

fn is_safe(x: i64) -> bool {
    (-MAX_SAFE..=MAX_SAFE).contains(&x)
}

// u + a when both operands and the sum are safe integers, else None (the ResourceBound signal).
fn safe_add(u: i64, a: i64) -> Option<i64> {
    if !is_safe(u) || !is_safe(a) {
        return None;
    }
    let r = u.checked_add(a)?;
    is_safe(r).then_some(r)
}

// ---- records ----

#[derive(PartialEq, Clone, Copy)]
enum State {
    Open,
    Halted,
    Closed,
}

struct Budget {
    id: Box<[u16]>,
    state: State,
    unlimited: bool,
    halt_reason: Option<u8>, // 0 time, 1 delegations, 2 commands, 3 reviews, 4 checks
    limits: [i64; 4],
    used: [i64; 4],
    tools: i64,
    failures: i64,
    retries: i64,
}

// A session's record: a typed budget, or the words of a record that is not a current-capable budget (state -1 =
// "no budget", or malformed). Pruv stores any blob opaquely and its CURRENT never matches such a record, so it is
// kept as words and printed back unchanged.
enum Slot {
    Budget(Budget),
    Opaque(Box<[i64]>),
}

// Decodes PLACEHOLDER record words [state, unlimited, haltReason, limits x4, used x4, tools, failures, retries,
// idLen, id units...] into the typed form; anything that load_cv (root_m8.pruv) would not treat as a well-formed
// current-capable record stays Opaque.
fn decode_record(w: &[i64]) -> Slot {
    let typed = (|| {
        if w.len() < 15 {
            return None;
        }
        let state = match w[0] {
            0 => State::Open,
            1 => State::Halted,
            2 => State::Closed,
            _ => return None,
        };
        let unlimited = match w[1] {
            0 => false,
            1 => true,
            _ => return None,
        };
        let halt_reason = match w[2] {
            -1 => None,
            r @ 0..=4 => Some(r as u8),
            _ => return None,
        };
        if w[14] != (w.len() - 15) as i64 {
            return None;
        }
        let id: Option<Box<[u16]>> = w[15..].iter().map(|&u| u16::try_from(u).ok()).collect();
        Some(Budget {
            id: id?,
            state,
            unlimited,
            halt_reason,
            limits: [w[3], w[4], w[5], w[6]],
            used: [w[7], w[8], w[9], w[10]],
            tools: w[11],
            failures: w[12],
            retries: w[13],
        })
    })();
    match typed {
        Some(b) => Slot::Budget(b),
        None => Slot::Opaque(w.into()),
    }
}

fn record_words(s: &Slot) -> Vec<i64> {
    match s {
        Slot::Opaque(w) => w.to_vec(),
        Slot::Budget(b) => {
            let st = match b.state {
                State::Open => 0,
                State::Halted => 1,
                State::Closed => 2,
            };
            let mut v = vec![st, b.unlimited as i64, b.halt_reason.map_or(-1, i64::from)];
            v.extend_from_slice(&b.limits);
            v.extend_from_slice(&b.used);
            v.extend_from_slice(&[b.tools, b.failures, b.retries, b.id.len() as i64]);
            v.extend(b.id.iter().map(|&u| u as i64));
            v
        }
    }
}

// `what` of charge: `what[key] ?? 0` is applied at use (None = missing or null).
#[derive(Default)]
struct What {
    counts: [Option<i64>; 4],
    retries: Option<i64>,
}

enum Charge {
    Ok,                   // 0: amounts added
    Nothing,              // 1: no current budget, or a closed one
    Halted(usize),        // 16 + r: halted budget (an absent halt reports 1)
    OverLimit(usize),     // 32 + k (k = 1..4): over limit, nothing charged (caller dispatches halt)
    ResourceBound,        // 5: a counter would leave the safe-integer range, nothing written
}

struct ResourceBound;

// Per-turn activity set: activity -> insertion index inside the turn; `seq` = insertion index of the turn.
struct Turn {
    seq: u64,
    acts: HashMap<Box<[u16]>, u32>,
}

struct World {
    sessions: HashMap<Box<[u16]>, Slot>,
    session_order: Vec<Box<[u16]>>,
    turns: HashMap<Box<[u16]>, Turn>,
    next_turn: u64,
}

impl World {
    fn new() -> World {
        World { sessions: HashMap::new(), session_order: Vec::new(), turns: HashMap::new(), next_turn: 0 }
    }

    // session put: a new key is appended, an existing key keeps its position.
    fn put(&mut self, key: &[u16], slot: Slot) {
        match self.sessions.get_mut(key) {
            Some(s) => *s = slot,
            None => {
                self.session_order.push(key.into());
                self.sessions.insert(key.into(), slot);
            }
        }
    }

    // CURRENT(ref): the session exists, holds a budget, and its budget id equals the ref's.
    fn current(&mut self, session: &[u16], budget: &[u16]) -> Option<&mut Budget> {
        match self.sessions.get_mut(session) {
            Some(Slot::Budget(b)) if *b.id == *budget => Some(b),
            _ => None,
        }
    }

    // bud_charge.pruv charge, step by step.
    fn charge(&mut self, session: &[u16], budget: &[u16], what: &What) -> Charge {
        // 1. no current budget -> nothing charged (status 1); closed -> 1 (L56-58)
        let b = match self.current(session, budget) {
            Some(b) if b.state != State::Closed => b,
            _ => return Charge::Nothing,
        };
        // 2. halted -> refused with the halt reason (L59-64)
        if b.state == State::Halted {
            return Charge::Halted(b.halt_reason.map_or(1, usize::from));
        }
        let a: [i64; 4] = std::array::from_fn(|k| what.counts[k].unwrap_or(0));
        let ar = what.retries.unwrap_or(0);
        // 3. limit checks in key order (L74-79): a positive amount needs a safe operand and sum (else ResourceBound),
        //    then sum > limit -> over limit on key k
        if !b.unlimited {
            for k in 0..4 {
                if a[k] > 0 {
                    match safe_add(b.used[k], a[k]) {
                        None => return Charge::ResourceBound,
                        Some(t) if t > b.limits[k] => return Charge::OverLimit(1 + k),
                        Some(_) => {}
                    }
                }
            }
        }
        // 4. all sums first (also for unlimited budgets and non-positive amounts), then the writes (L80-92): atomic
        let mut nv = [0i64; 4];
        for k in 0..4 {
            match safe_add(b.used[k], a[k]) {
                Some(v) => nv[k] = v,
                None => return Charge::ResourceBound,
            }
        }
        let Some(nr) = safe_add(b.retries, ar) else {
            return Charge::ResourceBound;
        };
        b.used = nv;
        b.retries = nr;
        Charge::Ok
    }

    // noteFailure after CURRENT: failures += 1 (any state); ResourceBound leaves the record untouched.
    fn note_failure(&mut self, session: &[u16], budget: &[u16]) -> Result<(), ResourceBound> {
        if let Some(b) = self.current(session, budget) {
            b.failures = safe_add(b.failures, 1).ok_or(ResourceBound)?;
        }
        Ok(())
    }

    // bud_tools.pruv note_tool + bud_charge add_tool: the activity key is consumed first (even with a missing or stale
    // budget), then CURRENT, then tools += 1. A ResourceBound there leaves the dedup key inserted, as in Pruv.
    fn note_tool(&mut self, session: &[u16], budget: &[u16], turn: &[u16], act: &[u16]) -> Result<(), ResourceBound> {
        let inserted = match self.turns.get_mut(turn) {
            Some(t) => t.add(act),
            None => {
                let mut t = Turn { seq: self.next_turn, acts: HashMap::new() };
                self.next_turn += 1;
                t.add(act);
                self.turns.insert(turn.into(), t);
                true
            }
        };
        if !inserted {
            return Ok(());
        }
        if let Some(b) = self.current(session, budget) {
            b.tools = safe_add(b.tools, 1).ok_or(ResourceBound)?;
        }
        Ok(())
    }

    fn forget_turn(&mut self, turn: &[u16]) {
        self.turns.remove(turn);
    }

    // Whole-state observation words (bud_world.pruv observe): sessions (key, record), clocks (always 0 here),
    // turns (key, count, activity keys), all in insertion order.
    fn observe(&self) -> Vec<i64> {
        fn push_key(out: &mut Vec<i64>, k: &[u16]) {
            out.push(k.len() as i64);
            out.extend(k.iter().map(|&u| u as i64));
        }
        let mut out = vec![self.session_order.len() as i64];
        for k in &self.session_order {
            push_key(&mut out, k);
            let rec = record_words(&self.sessions[k]);
            out.push(rec.len() as i64);
            out.extend(rec);
        }
        out.push(0);
        let mut turns: Vec<(&Box<[u16]>, &Turn)> = self.turns.iter().collect();
        turns.sort_by_key(|(_, t)| t.seq);
        out.push(turns.len() as i64);
        for (k, t) in turns {
            push_key(&mut out, k);
            let mut acts: Vec<(&Box<[u16]>, u32)> = t.acts.iter().map(|(a, &i)| (a, i)).collect();
            acts.sort_by_key(|&(_, i)| i);
            out.push(acts.len() as i64);
            for (a, _) in acts {
                push_key(&mut out, a);
            }
        }
        out
    }
}

impl Turn {
    // true when the activity was new
    fn add(&mut self, act: &[u16]) -> bool {
        if self.acts.contains_key(act) {
            return false;
        }
        let ord = self.acts.len() as u32;
        self.acts.insert(act.into(), ord);
        true
    }
}

// ---- q mode: the driver_m8.rs program protocol ----

const FX_LEN: usize = 256; // driver_m8.rs World::new: fx = 256 words

enum Op {
    Charge { frame: Vec<i64>, session: Vec<u16>, budget: Vec<u16>, what: What },
    Failure { session: Vec<u16>, budget: Vec<u16> },
    Tool { session: Vec<u16>, budget: Vec<u16>, turn: Vec<u16>, act: Vec<u16> },
    Forget { turn: Vec<u16> },
}

// root_m8.pruv key_end: the end of the key `len, units...` at p
fn key_end(op: &[i64], p: usize) -> Option<usize> {
    let n = op.len();
    if p >= n {
        return None;
    }
    let kn = op[p];
    if kn < 0 || kn as u64 > (n - p - 1) as u64 {
        return None;
    }
    Some(p + 1 + kn as usize)
}

fn units(w: &[i64]) -> Result<Vec<u16>, i64> {
    w.iter().map(|&u| u16::try_from(u).map_err(|_| 2)).collect()
}

fn what_of(w: &[i64]) -> What {
    let amount = |k: usize| if w[2 * k] == 2 { Some(w[2 * k + 1]) } else { None };
    What { counts: [amount(0), amount(1), amount(2), amount(3)], retries: amount(4) }
}

// t_call frame checks (root_m8.pruv t_call); Err = status 2
fn decode(op: &[i64]) -> Result<Op, i64> {
    if op.len() < 2 {
        return Err(2);
    }
    let code = op[0];
    let e1 = key_end(op, 1).ok_or(2)?;
    if code == 4 {
        if e1 != op.len() {
            return Err(2);
        }
        return Ok(Op::Forget { turn: units(&op[2..e1])? });
    }
    let e2 = key_end(op, e1).ok_or(2)?;
    let session = units(&op[2..e1])?;
    let budget = units(&op[e1 + 1..e2])?;
    match code {
        1 => {
            if e2 + 10 != op.len() {
                return Err(2);
            }
            Ok(Op::Charge { frame: op[1..e2].to_vec(), session, budget, what: what_of(&op[e2..e2 + 10]) })
        }
        2 => {
            if e2 != op.len() {
                return Err(2);
            }
            Ok(Op::Failure { session, budget })
        }
        3 => {
            let e3 = key_end(op, e2).ok_or(2)?;
            if key_end(op, e3) != Some(op.len()) {
                return Err(2);
            }
            Ok(Op::Tool { session, budget, turn: units(&op[e2 + 1..e3])?, act: units(&op[e3 + 1..])? })
        }
        _ => Err(2),
    }
}

// What the driver prints for one call.
fn run_call(w: &mut World, words: &[i64], o: &mut impl Write) -> bool {
    // returns true when the case stops
    let op = match decode(words) {
        Ok(op) => op,
        Err(s) => {
            writeln!(o, "X {}", s).unwrap();
            return true;
        }
    };
    match op {
        Op::Charge { frame, session, budget, what } => match w.charge(&session, &budget, &what) {
            Charge::Ok => writeln!(o, "R 0").unwrap(),
            Charge::Nothing => writeln!(o, "R 1").unwrap(),
            Charge::Halted(r) => writeln!(o, "R {}", 16 + r).unwrap(),
            Charge::ResourceBound => {
                writeln!(o, "X 5").unwrap();
                return true;
            }
            Charge::OverLimit(k) => {
                // halt frame [20, k, sessionId key, budgetId key] must fit fx (status 3 otherwise)
                if FX_LEN < frame.len() + 1 + 3 {
                    writeln!(o, "X 3").unwrap();
                } else {
                    let f: Vec<String> = [20, k as i64].iter().chain(frame.iter()).map(|x| x.to_string()).collect();
                    writeln!(o, "H {}", f.join(" ")).unwrap();
                }
                return true;
            }
        },
        Op::Failure { session, budget } => {
            if w.note_failure(&session, &budget).is_err() {
                writeln!(o, "X 5").unwrap();
                return true;
            }
            writeln!(o, "R -").unwrap();
        }
        Op::Tool { session, budget, turn, act } => {
            if w.note_tool(&session, &budget, &turn, &act).is_err() {
                writeln!(o, "X 5").unwrap();
                return true;
            }
            writeln!(o, "R -").unwrap();
        }
        Op::Forget { turn } => {
            w.forget_turn(&turn);
            writeln!(o, "R -").unwrap();
        }
    }
    false
}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

// session put words: [sessionId key, n, record words...] (root_m8.pruv t_put); malformed framing = status 2, which
// the Pruv driver treats as fatal.
fn put_words(w: &mut World, op: &[i64]) {
    let framed = (|| {
        let e1 = key_end(op, 0)?;
        if e1 >= op.len() {
            return None;
        }
        let n = op[e1];
        if n < 0 || n as usize != op.len() - e1 - 1 {
            return None;
        }
        let key = units(&op[1..e1]).ok()?;
        Some((key, e1 + 1))
    })();
    let Some((key, rs)) = framed else {
        eprintln!("t_put status 2");
        std::process::exit(2);
    };
    w.put(&key, decode_record(&op[rs..]));
}

fn run_q() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut w = World::new();
    let mut stopped = false;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                w = World::new();
                stopped = false;
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "put" => put_words(&mut w, &nums(&p[1..])),
            "step" => {
                if !stopped {
                    writeln!(o, "@ {}", p[1]).unwrap();
                }
            }
            "call" if !stopped => {
                let op = nums(&p[1..]);
                stopped = run_call(&mut w, &op, &mut o);
            }
            "call" => {}
            "end" => {
                let words: Vec<String> = w.observe().iter().map(|x| x.to_string()).collect();
                writeln!(o, "O {}", words.join(" ")).unwrap();
                writeln!(o, "end").unwrap();
            }
            other => {
                eprintln!("bad line {}", other);
                std::process::exit(2);
            }
        }
    }
}

// ---- perf mode: same items and shapes as driver_m8.rs run_perf ----

fn u16s(s: &str) -> Box<[u16]> {
    s.encode_utf16().collect()
}

fn bid_of(i: usize) -> String {
    format!("99b568f7-0000-4000-8000-{:012}", i)
}

// Retained UTF-16 keys, encoded once at setup. The Pruv driver also prepares its ops (key words) once at setup and
// does not re-encode per call; per call it decodes the frame, which the hand version has no counterpart of.
struct Ref {
    session: Box<[u16]>,
    budget: Box<[u16]>,
}

fn refk(i: usize) -> Ref {
    Ref { session: u16s(&format!("session-{:05}", i)), budget: u16s(&bid_of(i)) }
}

// perf <N> <reps> [item], prints `item size ns_per_call`:
//   charge   : charge(ref_i, {commands: 1, retries: 1}) round-robin over N sessions
//   failure  : noteFailure(ref_i) round-robin over N sessions
//   tooldup  : noteTool(ref_0, t1, a_j) on a turn that already holds N activities (dedup hit)
//   toolgrow : noteTool(ref_0, t2, a_j) for N new activities, then forgetTurn(t2); ns per noteTool (forget included)
fn run_perf(args: &[String]) {
    let n: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut w = World::new();
    for i in 0..n {
        let r = refk(i);
        w.put(
            &u16s(&format!("session-{:05}", i)),
            Slot::Budget(Budget {
                id: r.budget,
                state: State::Open,
                unlimited: false,
                halt_reason: None,
                limits: [1_000_000_000_000; 4],
                used: [0; 4],
                tools: 0,
                failures: 0,
                retries: 0,
            }),
        );
    }
    let refs: Vec<Ref> = (0..n).map(refk).collect();
    let what = What { counts: [None, Some(1), None, None], retries: Some(1) };
    for i in 0..1000 {
        let r = &refs[i % n];
        w.charge(&r.session, &r.budget, &what);
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            let r = black_box(&refs[i % n]);
            black_box(w.charge(&r.session, &r.budget, black_box(&what)));
        }
        println!("charge {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            let r = black_box(&refs[i % n]);
            let _ = black_box(w.note_failure(&r.session, &r.budget));
        }
        println!("failure {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    let turn1 = u16s("turn-1");
    let turn2 = u16s("turn-2");
    let acts: Vec<Box<[u16]>> = (0..n).map(|j| u16s(&format!("activity-{:05}", j))).collect();
    let r0 = &refs[0];
    for a in &acts {
        let _ = w.note_tool(&r0.session, &r0.budget, &turn1, a);
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            let a = black_box(&acts[i % n]);
            let _ = black_box(w.note_tool(&r0.session, &r0.budget, black_box(&turn1), a));
        }
        println!("tooldup {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    for a in &acts {
        let _ = w.note_tool(&r0.session, &r0.budget, &turn2, a);
    }
    w.forget_turn(&turn2);
    if on("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let t = Instant::now();
        for _ in 0..rounds {
            for a in &acts {
                let _ = black_box(w.note_tool(&r0.session, &r0.budget, black_box(&turn2), black_box(a)));
            }
            w.forget_turn(black_box(&turn2));
        }
        println!("toolgrow {} {:.1}", n, t.elapsed().as_nanos() as f64 / (rounds * n) as f64);
    }
    black_box(&w.sessions);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

// Hand-written safe idiomatic Rust parity baseline for M8 with the SAME semantics as the Pruv implementation
// (bud_charge.pruv + bud_tools.pruv + root_m8.pruv behind driver_m8.rs), replacing hand_m8.rs as the parity bar
// (hand_m8.rs stays as an informational row: UTF-8 String keys, no safe-integer refusal).
//
//   hand_m8_u16 perf  <N> <reps> [item]  prints `item size ns_per_call` and, after each item, `chk item size value`
//                                        (same items/shapes as driver_m8.rs run_perf; chk = full observed state folded
//                                        as s = s*31 + w, equal to driver_m8's). UNMATCHED boundary: keys are decoded
//                                        to Box<[u16]> once outside the timed loop and the timed loop calls
//                                        World::charge / note_failure / note_tool / forget_turn directly.
//   hand_m8_u16 perfw <N> <reps> [item]  MATCHED wire boundary: the timed loop takes exactly the i64 op words that
//                                        driver_m8.rs run_perf builds and does what a careful hand host must: frame
//                                        checks (key spans, O(1) canonical packed-key check: n range, words inside the
//                                        op, zero tail lanes; status 2 on failure), then unpacks the packed-u16 words
//                                        (4 lanes per i64 word) into a reused scratch Vec<u16> (no allocation per call
//                                        after the first), then the same World methods. Same chk checksums.
//   hand_m8_u16 q                        stdin program as driver_m8.rs run_q, byte-identical output lines (a rejected
//                                        put prints `P X 2`, state unchanged, the case continues). q mode runs through
//                                        the same call_wire as perfw, so the malformed-frame controls test it.
//
// Hasher option (cargo-free): `rustc --cfg fxmix ...` swaps the std SipHash for an Fx-style multiply-rotate hasher
// with a murmur3 fmix64 finalizer (plain Rust, no crates). Same semantics, same code otherwise (built as hand_m8_fx).
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

#[cfg(fxmix)]
mod hs {
    use std::hash::{BuildHasherDefault, Hasher};

    const K: u64 = 0x517c_c1b7_2722_0a95; // FxHash multiplier

    #[derive(Default)]
    pub struct FxMix {
        h: u64,
    }

    impl FxMix {
        #[inline(always)]
        fn add(&mut self, w: u64) {
            self.h = (self.h.rotate_left(5) ^ w).wrapping_mul(K);
        }
    }

    // murmur3 fmix64
    #[inline(always)]
    fn fmix64(mut k: u64) -> u64 {
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^ (k >> 33)
    }

    impl Hasher for FxMix {
        #[inline]
        fn write(&mut self, bytes: &[u8]) {
            let mut c = bytes.chunks_exact(8);
            for ch in &mut c {
                let mut b = [0u8; 8];
                b.copy_from_slice(ch);
                self.add(u64::from_le_bytes(b));
            }
            let r = c.remainder();
            if !r.is_empty() {
                let mut b = [0u8; 8];
                b[..r.len()].copy_from_slice(r);
                self.add(u64::from_le_bytes(b));
            }
        }
        #[inline]
        fn write_usize(&mut self, i: usize) {
            self.add(i as u64);
        }
        #[inline]
        fn finish(&self) -> u64 {
            fmix64(self.h)
        }
    }

    pub type Hs = BuildHasherDefault<FxMix>;
}

#[cfg(not(fxmix))]
mod hs {
    pub type Hs = std::collections::hash_map::RandomState;
}

type Map<K, V> = HashMap<K, V, hs::Hs>;

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

// A session's record: a typed budget, or the words of a record with state -1 ("no budget"), kept as words and printed
// back unchanged (Pruv's CURRENT never matches it). Malformed records never get here: put_words rejects them.
enum Slot {
    Budget(Budget),
    Opaque(Box<[i64]>),
}

// ---- packed-u16 key codec (bud_keys.pruv) ----
// n UTF-16 units (0..=2^30) are m = ceil(n/4) i64 words; word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3]
// (two's complement); lanes past n are zero. Any word is 4 valid u16 lanes, so there is no per-unit range check.

const MAX_KEY_UNITS: i64 = 1 << 30;

// m = ceil(n/4) for n in 0..=2^30, else None.
#[inline]
fn words_of(n: i64) -> Option<usize> {
    if (0..=MAX_KEY_UNITS).contains(&n) {
        Some((n as usize + 3) / 4)
    } else {
        None
    }
}

// The unused tail lanes of the last word of an n-unit key (n % 4 != 0) must be zero (canonical form).
#[inline]
fn tail_ok(n: i64, last: i64) -> bool {
    let r = (n & 3) as u32;
    r == 0 || (last as u64) & ((1u64 << (16 * (4 - r))) - 1) == 0
}

// O(1) canonical check of a packed key of n units held in exactly `w`.
#[inline]
fn packed_ok(n: i64, w: &[i64]) -> bool {
    words_of(n) == Some(w.len()) && (w.is_empty() || tail_ok(n, w[w.len() - 1]))
}

// Unpacks the n units of packed words `w` (the words already passed the canonical check) onto the end of `sc`.
#[inline]
fn unpack_units(sc: &mut Vec<u16>, n: usize, w: &[i64]) {
    for &x in w {
        let u = x as u64;
        sc.extend_from_slice(&[(u >> 48) as u16, (u >> 32) as u16, (u >> 16) as u16, u as u16]);
    }
    sc.truncate(sc.len() - (4 * w.len() - n));
}

// Inverse of unpack_units (used to re-emit packed words, e.g. a record's budget id in record_words).
fn pack_u16(u: &[u16]) -> Vec<i64> {
    u.chunks(4)
        .map(|c| {
            let mut x: u64 = 0;
            for l in 0..4 {
                x = (x << 16) | u64::from(c.get(l).copied().unwrap_or(0));
            }
            x as i64
        })
        .collect()
}

// The ingress check of root_m8.pruv t_put on record words [state, unlimited, haltReason, limits x4, used x4, tools,
// failures, retries, bn, packed id words(bn)...]: n >= 15; state in -1..2; unlimited in 0..1; reason in -1..4;
// len == 15 + ceil(bn/4) with bn in 0..=2^30 and a canonical id (zero tail lanes). Counter words are not validated.
fn valid_record(w: &[i64]) -> bool {
    w.len() >= 15
        && (-1..=2).contains(&w[0])
        && (0..=1).contains(&w[1])
        && (-1..=4).contains(&w[2])
        && packed_ok(w[14], &w[15..])
}

// Decodes a record that passed valid_record: state -1 -> Opaque (no budget), else the typed Budget.
fn decode_record(w: &[i64]) -> Slot {
    debug_assert!(valid_record(w));
    if w[0] == -1 {
        return Slot::Opaque(w.into());
    }
    let mut id = Vec::with_capacity(w[14] as usize);
    unpack_units(&mut id, w[14] as usize, &w[15..]);
    Slot::Budget(Budget {
        id: id.into(),
        state: match w[0] {
            0 => State::Open,
            1 => State::Halted,
            _ => State::Closed,
        },
        unlimited: w[1] == 1,
        halt_reason: if w[2] < 0 { None } else { Some(w[2] as u8) },
        limits: [w[3], w[4], w[5], w[6]],
        used: [w[7], w[8], w[9], w[10]],
        tools: w[11],
        failures: w[12],
        retries: w[13],
    })
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
            v.extend(pack_u16(&b.id)); // the id is re-emitted as packed words, like the record that was put
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
    acts: Map<Box<[u16]>, u32>,
}

struct World {
    sessions: Map<Box<[u16]>, Slot>,
    session_order: Vec<Box<[u16]>>,
    turns: Map<Box<[u16]>, Turn>,
    next_turn: u64,
}

impl World {
    fn new() -> World {
        World { sessions: Default::default(), session_order: Vec::new(), turns: Default::default(), next_turn: 0 }
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

    // bud_tools.pruv note_tool + bud_charge tool_room / add_tool: a seen activity returns at once (before CURRENT); a
    // new one is recorded even with a missing or stale budget, unless the current budget's used.tools is at the
    // safe-integer bound: that ResourceBound refuses before anything is written (KD-M8-1).
    fn note_tool(&mut self, session: &[u16], budget: &[u16], turn: &[u16], act: &[u16]) -> Result<(), ResourceBound> {
        if self.turns.get(turn).map_or(false, |t| t.acts.contains_key(act)) {
            return Ok(());
        }
        if let Some(b) = self.current(session, budget) {
            safe_add(b.tools, 1).ok_or(ResourceBound)?;
        }
        match self.turns.get_mut(turn) {
            Some(t) => {
                t.add(act);
            }
            None => {
                let mut t = Turn { seq: self.next_turn, acts: Default::default() };
                self.next_turn += 1;
                t.add(act);
                self.turns.insert(turn.into(), t);
            }
        }
        if let Some(b) = self.current(session, budget) {
            b.tools += 1;
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

impl World {
    // Folds the full observed state (observe() words, the same words q mode prints after `O` and driver_m8.rs
    // World::checksum folds) as s = s*31 + w (wrapping u64). Called after a timed loop, outside the timing.
    fn checksum(&self) -> u64 {
        self.observe().iter().fold(0u64, |s, &x| s.wrapping_mul(31).wrapping_add(x as u64))
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

// ---- wire boundary: the driver_m8.rs / root_m8.pruv t_call frame protocol ----

const FX_LEN: usize = 256; // driver_m8.rs World::new: fx = 256 words

// What t_call leaves in fx: `r` = the charge status (fx[1]); for status 6 `k` = the limit key and `e2` = the end of the
// ref frame (the halt frame is [20, k, op[1..e2]]).
struct Fr {
    r: i64,
    k: i64,
    e2: usize,
}

// root_m8.pruv key_span: the end of the packed key `n, packed words(n)` at p: O(1) canonical check (n in 0..=2^30,
// the ceil(n/4) words fit inside op, unused tail lanes of the last word zero). No per-unit range check exists.
#[inline]
fn key_span(op: &[i64], p: usize) -> Option<usize> {
    if p >= op.len() {
        return None;
    }
    let kn = op[p];
    let m = words_of(kn)?;
    if m > op.len() - p - 1 {
        return None;
    }
    let end = p + 1 + m;
    if m > 0 && !tail_ok(kn, op[end - 1]) {
        return None;
    }
    Some(end)
}

// Unpacks the key at p (already validated by key_span, end e) to u16 units appended to the scratch buffer.
#[inline]
fn push_key(sc: &mut Vec<u16>, op: &[i64], p: usize, e: usize) {
    unpack_units(sc, op[p] as usize, &op[p + 1..e]);
}

fn what_of(w: &[i64]) -> What {
    let amount = |k: usize| if w[2 * k] == 2 { Some(w[2 * k + 1]) } else { None };
    What { counts: [amount(0), amount(1), amount(2), amount(3)], retries: amount(4) }
}

impl World {
    // One call on the wire words, status as Pruv's t_call returns it: 0 done, 2 invalid input, 3 halt frame does not
    // fit fx, 5 ResourceBound, 6 halt dispatch (over-limit charge). `sc` is the reused scratch buffer of converted
    // units (unpacked from the packed wire keys): it is cleared, never shrunk, so a call allocates only while the buffer is still growing.
    fn call_wire(&mut self, op: &[i64], sc: &mut Vec<u16>, fr: &mut Fr) -> i64 {
        if op.len() < 2 {
            return 2;
        }
        sc.clear();
        let code = op[0];
        let Some(e1) = key_span(op, 1) else {
            return 2;
        };
        if code == 4 {
            if e1 != op.len() {
                return 2;
            }
            push_key(sc, op, 1, e1);
            self.forget_turn(sc);
            return 0;
        }
        let Some(e2) = key_span(op, e1) else {
            return 2;
        };
        push_key(sc, op, 1, e1);
        push_key(sc, op, e1, e2);
        let sn = op[1] as usize; // session unit count
        match code {
            1 => {
                if e2 + 10 != op.len() {
                    return 2;
                }
                let what = what_of(&op[e2..e2 + 10]);
                let (s, b) = sc.split_at(sn);
                match self.charge(s, b, &what) {
                    Charge::Ok => {
                        fr.r = 0;
                        0
                    }
                    Charge::Nothing => {
                        fr.r = 1;
                        0
                    }
                    Charge::Halted(r) => {
                        fr.r = 16 + r as i64;
                        0
                    }
                    Charge::ResourceBound => 5,
                    Charge::OverLimit(k) => {
                        fr.k = k as i64;
                        fr.e2 = e2;
                        if FX_LEN < e2 + 3 {
                            3
                        } else {
                            6
                        }
                    }
                }
            }
            2 => {
                if e2 != op.len() {
                    return 2;
                }
                let (s, b) = sc.split_at(sn);
                if self.note_failure(s, b).is_err() {
                    return 5;
                }
                0
            }
            3 => {
                let Some(e3) = key_span(op, e2) else {
                    return 2;
                };
                if key_span(op, e3) != Some(op.len()) {
                    return 2;
                }
                push_key(sc, op, e2, e3);
                push_key(sc, op, e3, op.len());
                let bn = op[e1] as usize;
                let tn = op[e2] as usize;
                let (s, rest) = sc.split_at(sn);
                let (b, rest) = rest.split_at(bn);
                let (t, a) = rest.split_at(tn);
                if self.note_tool(s, b, t, a).is_err() {
                    return 5;
                }
                0
            }
            _ => 2,
        }
    }
}

// ---- q mode: the driver_m8.rs program protocol ----

// root_m8.pruv key_end: the end of the packed key `n, packed words(n)` at p (same O(1) canonical check as key_span);
// used by the put framing only (calls go through call_wire).
fn key_end(op: &[i64], p: usize) -> Option<usize> {
    key_span(op, p)
}

// The units of the (validated) packed key at p..e.
fn units(op: &[i64], p: usize, e: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(op[p] as usize);
    push_key(&mut v, op, p, e);
    v
}

// What the driver prints for one call.
fn run_call(w: &mut World, words: &[i64], sc: &mut Vec<u16>, o: &mut impl Write) -> bool {
    // returns true when the case stops
    let mut fr = Fr { r: 0, k: 0, e2: 0 };
    match w.call_wire(words, sc, &mut fr) {
        0 => {
            if words[0] == 1 {
                writeln!(o, "R {}", fr.r).unwrap();
            } else {
                writeln!(o, "R -").unwrap();
            }
            false
        }
        6 => {
            let f: Vec<String> = [20, fr.k].iter().chain(words[1..fr.e2].iter()).map(|x| x.to_string()).collect();
            writeln!(o, "H {}", f.join(" ")).unwrap();
            true
        }
        s => {
            writeln!(o, "X {}", s).unwrap();
            true
        }
    }
}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

// session put words: [sessionId key, n, record words...] (root_m8.pruv t_put); returns the status: 0 stored, 2 for
// malformed framing / key units / record words (state unchanged, as the Pruv driver prints `P X 2` and continues).
fn put_words(w: &mut World, op: &[i64]) -> i64 {
    let framed = (|| {
        let e1 = key_end(op, 0)?;
        if e1 >= op.len() {
            return None;
        }
        let n = op[e1];
        if n < 0 || n as usize != op.len() - e1 - 1 {
            return None;
        }
        let key = units(op, 0, e1);
        Some((key, e1 + 1))
    })();
    let Some((key, rs)) = framed else {
        return 2;
    };
    if !valid_record(&op[rs..]) {
        return 2;
    }
    w.put(&key, decode_record(&op[rs..]));
    0
}

fn run_q() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut w = World::new();
    let mut sc: Vec<u16> = Vec::new();
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
            "put" => {
                let r = put_words(&mut w, &nums(&p[1..]));
                if r != 0 {
                    writeln!(o, "P X {}", r).unwrap();
                }
            }
            "step" => {
                if !stopped {
                    writeln!(o, "@ {}", p[1]).unwrap();
                }
            }
            "call" if !stopped => {
                let op = nums(&p[1..]);
                stopped = run_call(&mut w, &op, &mut sc, &mut o);
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

// Retained UTF-16 keys, encoded once at setup (the UNMATCHED boundary: the Pruv driver also prepares its op words once
// at setup, but per call it parses and validates the frame, which `perf` has no counterpart of; see perfw).
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
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("charge {} {:.1}", n, ns);
        println!("chk charge {} {}", n, w.checksum()); // outside the timing
    }
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            let r = black_box(&refs[i % n]);
            let _ = black_box(w.note_failure(&r.session, &r.budget));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("failure {} {:.1}", n, ns);
        println!("chk failure {} {}", n, w.checksum());
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
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("tooldup {} {:.1}", n, ns);
        println!("chk tooldup {} {}", n, w.checksum());
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
        let ns = t.elapsed().as_nanos() as f64 / (rounds * n) as f64;
        println!("toolgrow {} {:.1}", n, ns);
        println!("chk toolgrow {} {}", n, w.checksum());
    }
    black_box(&w.sessions);
}

// ---- perfw: matched wire boundary (same op words as driver_m8.rs run_perf) ----

// Wire key `n, packed words(n)` (built at setup, outside the timed loops).
fn key_words(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    v.extend(pack_u16(&u));
    v
}

fn ref_words(i: usize) -> Vec<i64> {
    let mut v = key_words(&format!("session-{:05}", i));
    v.extend(key_words(&bid_of(i)));
    v
}

// N sessions put through the same ingress words as driver_m8.rs world_n (open budget, limits 10^12).
fn world_wire(n: usize) -> World {
    let mut w = World::new();
    for i in 0..n {
        let mut put = key_words(&format!("session-{:05}", i));
        let mut rec = vec![0i64, 0, -1];
        rec.extend_from_slice(&[1_000_000_000_000i64; 4]);
        rec.extend_from_slice(&[0i64; 7]);
        rec.extend(key_words(&bid_of(i)));
        put.push(rec.len() as i64);
        put.extend(rec);
        if put_words(&mut w, &put) != 0 {
            eprintln!("put rejected");
            std::process::exit(2);
        }
    }
    w
}

fn run_perfw(args: &[String]) {
    let n: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut w = world_wire(n);
    let mut sc: Vec<u16> = Vec::with_capacity(64);
    let mut fr = Fr { r: 0, k: 0, e2: 0 };
    let ops: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            let mut op = vec![1];
            op.extend(ref_words(i));
            op.extend_from_slice(&[0, 0, 2, 1, 0, 0, 0, 0, 2, 1]);
            op
        })
        .collect();
    for i in 0..1000 {
        w.call_wire(&ops[i % n], &mut sc, &mut fr);
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call_wire(black_box(&ops[i % n]), &mut sc, &mut fr));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("charge {} {:.1}", n, ns);
        println!("chk charge {} {}", n, w.checksum());
    }
    let fo: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            let mut op = vec![2];
            op.extend(ref_words(i));
            op
        })
        .collect();
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call_wire(black_box(&fo[i % n]), &mut sc, &mut fr));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("failure {} {:.1}", n, ns);
        println!("chk failure {} {}", n, w.checksum());
    }
    let tool = |turn: &str, j: usize| -> Vec<i64> {
        let mut op = vec![3];
        op.extend(ref_words(0));
        op.extend(key_words(turn));
        op.extend(key_words(&format!("activity-{:05}", j)));
        op
    };
    let dup: Vec<Vec<i64>> = (0..n).map(|j| tool("turn-1", j)).collect();
    for d in &dup {
        w.call_wire(d, &mut sc, &mut fr);
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call_wire(black_box(&dup[i % n]), &mut sc, &mut fr));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("tooldup {} {:.1}", n, ns);
        println!("chk tooldup {} {}", n, w.checksum());
    }
    let grow: Vec<Vec<i64>> = (0..n).map(|j| tool("turn-2", j)).collect();
    let mut forget = vec![4];
    forget.extend(key_words("turn-2"));
    for d in &grow {
        w.call_wire(d, &mut sc, &mut fr);
    }
    w.call_wire(&forget, &mut sc, &mut fr);
    if on("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let t = Instant::now();
        for _ in 0..rounds {
            for d in &grow {
                black_box(w.call_wire(black_box(d), &mut sc, &mut fr));
            }
            black_box(w.call_wire(black_box(&forget), &mut sc, &mut fr));
        }
        let ns = t.elapsed().as_nanos() as f64 / (rounds * n) as f64;
        println!("toolgrow {} {:.1}", n, ns);
        println!("chk toolgrow {} {}", n, w.checksum());
    }
    black_box(&w.sessions);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else if args.len() > 1 && args[1] == "perfw" {
        run_perfw(&args[2..]);
    } else {
        run_q();
    }
}

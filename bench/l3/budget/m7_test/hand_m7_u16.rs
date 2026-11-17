// Hand-written, typed, safe idiomatic Rust parity baseline for M7 with the SAME semantics as the Pruv implementation
// (bud_lifecycle.pruv + root.pruv behind driver_m7.rs). hand_m7.rs (UTF-8 String keys) stays as an informational row.
//
//   hand_m7_u16 q                        stdin program as driver_m7.rs run_q, byte-identical output lines
//   hand_m7_u16 perf <S> <reps> [item]   prints `item size ns_per_call` for ishalted, current, attach, closereopen,
//                                        replace (same items / shapes as driver_m7.rs run_perf; dispatch and rawcall
//                                        are Pruv-only probes) and, after each item, `chk item size value`
//
// chk (hand only): the full observed final state (the words q mode prints after `O`) folded as s = s*31 + w (wrapping
// u64), computed outside the timing. driver_m7 has no checksum, so Pruv/hand state equality is checked in q mode
// (diff_hand_m7.py compares the `O` observation byte for byte); chk only shows that the perf runs are deterministic.
//
// Typing: keys (sessionId, budgetId, follows, uuids) are UTF-16 code units, Box<[u16]> in std HashMaps, looked up by
// &[u16]; the session records are decoded once at put time into a typed Budget (enums for state / origin / how, the
// instants as Stamp / At variants). encode() writes the identical record words back (header comment of
// bud_lifecycle.pruv), which q mode prints for `V 4 ...` and `O`.
//
// Domain of this baseline (differences from the Pruv words machine, all fatal, exit status 3, never silent):
//  * a stored key or uuid with a unit outside 0..65535 (the Pruv store keeps the raw word) is not representable;
//    a call key with such a unit can never match a stored key and is answered as a miss.
//  * a session record with state 0..2 and >= 30 words that is not canonical (unused header words nonzero, string
//    lengths not adding up to the word count, unit out of range, header enum out of range) is not representable.
//    Records with state outside 0..2 or fewer than 30 words are kept as opaque words (Pruv's CURRENT never matches
//    them; attach / replace open over them, or answer 2 when the word count is below 30 and state >= 0).
//  * clocks: the M9 clock map exists (HashMap, empty: the q protocol has no way to fill it), consulted where Pruv
//    consults it, behind an is_empty() test like Pruv's clock_in.
#![allow(dead_code)]
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

const MAX_SAFE: i64 = 9_007_199_254_740_991; // 2^53 - 1 (clamp of unit limits, bud_lifecycle fresh)
const OUT_LEN: usize = 4096; // driver_m7.rs World::out during calls: results that do not fit answer status 3

type Key = Box<[u16]>;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Open,
    Halted,
    Closed,
}

#[derive(Clone, Copy, PartialEq)]
enum Origin {
    User,
    Schedule,
    Direct,
    Release,
}

#[derive(Clone, Copy, PartialEq)]
enum How {
    Quiet,
    UserEnd,
    UserMessage,
}

#[derive(Clone, Copy, PartialEq)]
enum Size {
    Absent,
    Unlimited,
    Times(i64),
}

#[derive(Clone, Copy, PartialEq)]
enum Unlimited {
    Absent,
    True,
    Other, // key present with a non-true value, kept in `rest` by the record's producer
}

// An instant property: absent, text as stored, or a number of ms written by this module.
#[derive(Clone)]
enum Stamp {
    Absent,
    Text(Key),
    Ms(i64),
}

#[derive(Clone)]
enum At {
    Text(Key),
    Ms(i64),
}

#[derive(Clone)]
enum Follows {
    Absent,
    Id(Key),
    Other,
}

#[derive(Clone)]
struct Budget {
    id: Key,
    state: State,
    origin: Origin,
    unlimited: Unlimited,
    limits: [i64; 5],
    used: [i64; 8],
    active_since: Stamp,
    ended: Option<(How, At)>,
    opened_at: Stamp,
    follows: Follows,
    rest: Key, // the other budget properties (halt, card, ...) as one opaque unit string
}

// A session's record: the typed budget, or opaque words (no budget [-1], a blob too short to be a record, a state
// word outside 0..2): kept and printed back unchanged.
enum Slot {
    Budget(Budget),
    Opaque(Box<[i64]>),
}

fn unsupported(msg: &str) -> ! {
    eprintln!("hand_m7_u16: unsupported input: {}", msg);
    std::process::exit(3)
}

fn key_of(w: &[i64]) -> Option<Key> {
    w.iter().map(|&u| u16::try_from(u).ok()).collect()
}

fn u16s(s: &str) -> Key {
    s.encode_utf16().collect()
}

// ---- record words <-> typed Budget ----

fn push_str(v: &mut Vec<i64>, s: &[u16]) {
    v.push(s.len() as i64);
    v.extend(s.iter().map(|&u| u as i64));
}

fn encode(b: &Budget) -> Vec<i64> {
    let empty: &[u16] = &[];
    let mut h = [0i64; 24];
    h[0] = match b.state {
        State::Open => 0,
        State::Halted => 1,
        State::Closed => 2,
    };
    h[1] = match b.origin {
        Origin::User => 0,
        Origin::Schedule => 1,
        Origin::Direct => 2,
        Origin::Release => 3,
    };
    h[2] = match b.unlimited {
        Unlimited::Absent => 0,
        Unlimited::True => 1,
        Unlimited::Other => 2,
    };
    h[3..8].copy_from_slice(&b.limits);
    h[8..16].copy_from_slice(&b.used);
    let (sk, sm, st): (i64, i64, &[u16]) = match &b.active_since {
        Stamp::Absent => (0, 0, empty),
        Stamp::Text(t) => (1, 0, &t[..]),
        Stamp::Ms(m) => (2, *m, empty),
    };
    h[16] = sk;
    h[17] = sm;
    let (eh, ek, em, et): (i64, i64, i64, &[u16]) = match &b.ended {
        None => (0, 0, 0, empty),
        Some((how, At::Text(t))) => (how_n(*how), 1, 0, &t[..]),
        Some((how, At::Ms(m))) => (how_n(*how), 2, *m, empty),
    };
    h[18] = eh;
    h[19] = ek;
    h[22] = em;
    let (ok, om, ot): (i64, i64, &[u16]) = match &b.opened_at {
        Stamp::Absent => (0, 0, empty),
        Stamp::Text(t) => (1, 0, &t[..]),
        Stamp::Ms(m) => (2, *m, empty),
    };
    h[20] = ok;
    h[21] = om;
    let (fk, ft): (i64, &[u16]) = match &b.follows {
        Follows::Absent => (0, empty),
        Follows::Id(t) => (1, &t[..]),
        Follows::Other => (2, empty),
    };
    h[23] = fk;
    let mut v: Vec<i64> = Vec::with_capacity(30 + b.id.len() + b.rest.len() + st.len() + et.len() + ot.len() + ft.len());
    v.extend_from_slice(&h);
    push_str(&mut v, &b.id);
    push_str(&mut v, ot);
    push_str(&mut v, st);
    push_str(&mut v, et);
    push_str(&mut v, ft);
    push_str(&mut v, &b.rest);
    v
}

fn how_n(h: How) -> i64 {
    match h {
        How::Quiet => 1,
        How::UserEnd => 2,
        How::UserMessage => 3,
    }
}

// Typed view of a record with state 0..2 and >= 30 words, or None when the words are not canonical (see header).
fn canonical(w: &[i64]) -> Option<Budget> {
    let empty_w: &[i64] = &[];
    let mut strs = [empty_w; 6];
    let mut p = 24usize;
    for s in strs.iter_mut() {
        let len = *w.get(p)?;
        if len < 0 {
            return None;
        }
        let end = p.checked_add(1)?.checked_add(usize::try_from(len).ok()?)?;
        if end > w.len() {
            return None;
        }
        *s = &w[p + 1..end];
        p = end;
    }
    if p != w.len() {
        return None;
    }
    let mut t: Vec<Key> = Vec::with_capacity(6);
    for s in &strs {
        t.push(key_of(s)?);
    }
    let empty = |i: usize| strs[i].is_empty();
    let state = match w[0] {
        0 => State::Open,
        1 => State::Halted,
        _ => State::Closed,
    };
    let origin = match w[1] {
        0 => Origin::User,
        1 => Origin::Schedule,
        2 => Origin::Direct,
        3 => Origin::Release,
        _ => return None,
    };
    let unlimited = match w[2] {
        0 => Unlimited::Absent,
        1 => Unlimited::True,
        2 => Unlimited::Other,
        _ => return None,
    };
    let active_since = match (w[16], w[17]) {
        (0, 0) if empty(2) => Stamp::Absent,
        (1, 0) => Stamp::Text(t[2].clone()),
        (2, m) if empty(2) => Stamp::Ms(m),
        _ => return None,
    };
    let how = match w[18] {
        0 => None,
        1 => Some(How::Quiet),
        2 => Some(How::UserEnd),
        3 => Some(How::UserMessage),
        _ => return None,
    };
    let ended = match (how, w[19], w[22]) {
        (None, 0, 0) if empty(3) => None,
        (Some(h), 1, 0) => Some((h, At::Text(t[3].clone()))),
        (Some(h), 2, m) if empty(3) => Some((h, At::Ms(m))),
        _ => return None,
    };
    let opened_at = match (w[20], w[21]) {
        (0, 0) if empty(1) => Stamp::Absent,
        (1, 0) => Stamp::Text(t[1].clone()),
        (2, m) if empty(1) => Stamp::Ms(m),
        _ => return None,
    };
    let follows = match w[23] {
        0 if empty(4) => Follows::Absent,
        1 => Follows::Id(t[4].clone()),
        2 if empty(4) => Follows::Other,
        _ => return None,
    };
    let mut limits = [0i64; 5];
    limits.copy_from_slice(&w[3..8]);
    let mut used = [0i64; 8];
    used.copy_from_slice(&w[8..16]);
    Some(Budget {
        id: t[0].clone(),
        state,
        origin,
        unlimited,
        limits,
        used,
        active_since,
        ended,
        opened_at,
        follows,
        rest: t[5].clone(),
    })
}

fn decode(w: &[i64]) -> Slot {
    if w.len() < 30 || !(0..=2).contains(&w[0]) {
        return Slot::Opaque(w.into());
    }
    match canonical(w) {
        Some(b) => Slot::Budget(b),
        None => unsupported("budget record (state 0..2, >= 30 words) is not canonical"),
    }
}

// ---- the lifecycle ----

enum Effect {
    TimerClear, // fx [13]
    Changed,    // [20]
    CarryHeld,  // [21]
    Uuid,       // [22] one id consumed
    Quiet,      // [23] isQuiet consulted
}

// A call's result, borrowing the stored data (nothing is copied; q mode renders the words from it).
enum Res<'a> {
    Undef,
    Bool(bool),
    Ref(&'a [u16], Option<&'a [u16]>), // budgetId, carried old id (effect CarryHeld)
    View(&'a Budget),
}

struct World {
    sessions: HashMap<Key, Slot>,
    // keys in insertion order (Pruv observes the session map in insertion order; a put over a key keeps its place, a
    // delete + put moves it to the end). Hot paths do not touch it.
    order: Vec<Key>,
    // budget id -> runs on its clock (the M9 clock map; empty here)
    clocks: HashMap<Key, usize>,
    fx: Vec<Effect>,
}

// bud_lifecycle fresh: the record of a new budget.
fn fresh(now: i64, origin: Origin, size: Size, unit: &[i64; 6], id: &[u16], follows: Follows) -> Budget {
    let chosen = match size {
        Size::Absent => {
            if unit[5] == 1 {
                0
            } else {
                1
            }
        }
        Size::Unlimited => 0,
        Size::Times(n) => n,
    };
    let times = if chosen == 0 { 1 } else { chosen.min(8) };
    let mut limits = [0i64; 5];
    for i in 0..5 {
        limits[i] = unit[i].clamp(0, MAX_SAFE) * times;
    }
    Budget {
        id: id.into(),
        state: State::Open,
        origin,
        unlimited: if chosen == 0 { Unlimited::True } else { Unlimited::Absent },
        limits,
        used: [0; 8],
        active_since: Stamp::Absent,
        ended: None,
        opened_at: Stamp::Ms(now),
        follows,
        rest: Vec::new().into_boxed_slice(),
    }
}

// attach on the session's slot (B:181-185): the session's budget when it is not closed, else OPEN.
fn attach_slot<'a>(
    slot: &'a mut Slot,
    fx: &mut Vec<Effect>,
    clocks: &mut HashMap<Key, usize>,
    origin: Origin,
    size: Size,
    unit: &[i64; 6],
    uuid: Option<&[u16]>,
    now: i64,
) -> Result<Res<'a>, i64> {
    if let Slot::Opaque(w) = &*slot {
        if w.is_empty() || (w[0] >= 0 && w.len() < 30) {
            return Err(2);
        }
    }
    let live = matches!(&*slot, Slot::Budget(b) if b.state != State::Closed);
    if live {
        let Slot::Budget(b) = slot else { unreachable!() };
        if 2 + b.id.len() > OUT_LEN {
            return Err(3);
        }
        return Ok(Res::Ref(&b.id, None));
    }
    let Some(uuid) = uuid else { unsupported("uuid key unit outside 0..65535") };
    if 2 + uuid.len() > OUT_LEN {
        return Err(3);
    }
    if !clocks.is_empty() {
        if let Slot::Budget(old) = &*slot {
            if clocks.remove(&*old.id).is_some() {
                fx.push(Effect::TimerClear);
            }
        }
    }
    fx.push(Effect::Uuid);
    *slot = Slot::Budget(fresh(now, origin, size, unit, uuid, Follows::Absent));
    fx.push(Effect::Changed);
    match slot {
        Slot::Budget(b) => Ok(Res::Ref(&b.id, None)),
        Slot::Opaque(_) => unreachable!(),
    }
}

impl World {
    fn new() -> World {
        World { sessions: HashMap::new(), order: Vec::new(), clocks: HashMap::new(), fx: Vec::with_capacity(16) }
    }

    // session put: a new key is appended, an existing key keeps its position.
    fn put(&mut self, key: &[u16], slot: Slot) {
        match self.sessions.get_mut(key) {
            Some(s) => *s = slot,
            None => {
                self.order.push(key.into());
                self.sessions.insert(key.into(), slot);
            }
        }
    }

    fn del(&mut self, key: &[u16]) {
        if self.sessions.remove(key).is_some() {
            if let Some(i) = self.order.iter().position(|k| &**k == key) {
                self.order.remove(i);
            }
        }
    }

    // CURRENT(ref): the session has a budget whose id is the ref's.
    fn current(&self, sid: &[u16], bid: &[u16]) -> Option<&Budget> {
        match self.sessions.get(sid) {
            Some(Slot::Budget(b)) if *b.id == *bid => Some(b),
            _ => None,
        }
    }

    fn is_halted(&self, sid: &[u16], bid: &[u16]) -> bool {
        self.current(sid, bid).map_or(false, |b| b.state == State::Halted)
    }

    fn attach(
        &mut self,
        sid: &[u16],
        origin: Origin,
        size: Size,
        unit: &[i64; 6],
        uuid: Option<&[u16]>,
        now: i64,
    ) -> Result<Res<'_>, i64> {
        let slot = match self.sessions.get_mut(sid) {
            Some(s) => s,
            None => return Err(2),
        };
        attach_slot(slot, &mut self.fx, &mut self.clocks, origin, size, unit, uuid, now)
    }

    // replaceForUserMessage (B:192-200): a halted budget is closed ('user-message') and a new one opened; unless the
    // origin is schedule the new budget follows the old id and the old id's held items are carried. Otherwise attach.
    // The closed record is replaced in the same step, so it is never built (as in Pruv); the old id moves into follows.
    fn replace(
        &mut self,
        sid: &[u16],
        origin: Origin,
        size: Size,
        unit: &[i64; 6],
        uuid: Option<&[u16]>,
        now: i64,
    ) -> Result<Res<'_>, i64> {
        let slot = match self.sessions.get_mut(sid) {
            Some(s) => s,
            None => return Err(2),
        };
        if !matches!(&*slot, Slot::Budget(b) if b.state == State::Halted) {
            return attach_slot(slot, &mut self.fx, &mut self.clocks, origin, size, unit, uuid, now);
        }
        let Slot::Budget(old) = slot else { unreachable!() };
        let Some(uuid) = uuid else { unsupported("uuid key unit outside 0..65535") };
        if 3 + uuid.len() + old.id.len() > OUT_LEN {
            return Err(3);
        }
        // CLOSE's clock part (SETTLE adds nothing without runs; DROP removes the clock)
        if !self.clocks.is_empty() && self.clocks.remove(&*old.id).is_some() {
            self.fx.push(Effect::TimerClear);
        }
        let carry = origin != Origin::Schedule;
        let follows = if carry { Follows::Id(std::mem::take(&mut old.id)) } else { Follows::Absent };
        self.fx.push(Effect::Uuid);
        *old = fresh(now, origin, size, unit, uuid, follows);
        self.fx.push(Effect::Changed);
        if carry {
            self.fx.push(Effect::CarryHeld);
        }
        let carried = match &old.follows {
            Follows::Id(f) => Some(&f[..]),
            _ => None,
        };
        Ok(Res::Ref(&old.id, carried))
    }

    // reopen (B:224-231): CURRENT(ref) closed with ended.how quiet -> open, ended deleted, changed.
    fn reopen(&mut self, sid: &[u16], bid: &[u16]) -> Result<Res<'_>, i64> {
        let b = match self.sessions.get_mut(sid) {
            Some(Slot::Budget(b)) if *b.id == *bid => b,
            _ => return Ok(Res::Bool(false)),
        };
        if b.state != State::Closed || !matches!(b.ended, Some((How::Quiet, _))) {
            return Ok(Res::Bool(false));
        }
        b.state = State::Open;
        b.ended = None;
        self.fx.push(Effect::Changed);
        Ok(Res::Bool(true))
    }

    // maybeClose (B:399-408): an open budget with no runs on its clock closes when the host says the session is quiet
    // (quiet: 0 no, otherwise yes, -1 not supplied -> status 5). Consulting the answer pushes Quiet first.
    fn maybe_close(&mut self, sid: &[u16], quiet: i64, now: i64) -> Result<Res<'_>, i64> {
        let b = match self.sessions.get_mut(sid) {
            None => return Err(2),
            Some(Slot::Budget(b)) if b.state == State::Open => b,
            Some(_) => return Ok(Res::Bool(false)),
        };
        if !self.clocks.is_empty() && self.clocks.get(&*b.id).copied().unwrap_or(0) > 0 {
            return Ok(Res::Bool(false));
        }
        if quiet < 0 {
            return Err(5);
        }
        self.fx.push(Effect::Quiet);
        if quiet == 0 {
            return Ok(Res::Bool(false));
        }
        // CLOSE('quiet'): clock part, state closed, ended = {how, at: now}
        if !self.clocks.is_empty() && self.clocks.remove(&*b.id).is_some() {
            self.fx.push(Effect::TimerClear);
        }
        b.state = State::Closed;
        b.ended = Some((How::Quiet, At::Ms(now)));
        self.fx.push(Effect::Changed);
        Ok(Res::Bool(true))
    }

    // Whole-state observation words (bud_world.pruv observe): sessions (key, record) in insertion order, then the
    // clock count and the turn count (both 0 in M7 scenarios).
    fn observe(&self) -> Vec<i64> {
        let mut v = vec![self.order.len() as i64];
        for k in &self.order {
            push_str(&mut v, k);
            let rec = match &self.sessions[k] {
                Slot::Budget(b) => encode(b),
                Slot::Opaque(w) => w.to_vec(),
            };
            v.push(rec.len() as i64);
            v.extend(rec);
        }
        v.push(0);
        v.push(0);
        v
    }

    // Folds the observed state as s = s*31 + w (wrapping u64). Called after a timed loop, outside the timing.
    fn checksum(&self) -> u64 {
        self.observe().iter().fold(0u64, |s, &x| s.wrapping_mul(31).wrapping_add(x as u64))
    }
}

// ---- q mode: the driver_m7.rs program protocol ----

// root.pruv key_end: the end of the key `len, units...` at p (length within op; units are not checked here).
fn key_end(op: &[i64], p: usize) -> Option<usize> {
    let n = op.len();
    if p >= n {
        return None;
    }
    let kn = op[p];
    if kn < 0 || kn > (n - p - 1) as i64 {
        return None;
    }
    Some(p + 1 + kn as usize)
}

// t_call's op: [code, now, quiet, origin, size, unit x6, sessionId key, budgetId key, uuid key]
struct Call {
    code: i64,
    now: i64,
    quiet: i64,
    origin: i64,
    size: i64,
    unit: [i64; 6],
    sk: Option<Key>,
    bk: Option<Key>,
    uk: Option<Key>,
}

// root.pruv t_call's frame checks; Err = status 2. A key with a unit outside 0..65535 decodes to None (never matches).
fn decode_call(op: &[i64]) -> Result<Call, i64> {
    let e1 = key_end(op, 11).ok_or(2)?;
    let e2 = key_end(op, e1).ok_or(2)?;
    let e3 = key_end(op, e2).ok_or(2)?;
    if e3 != op.len() {
        return Err(2);
    }
    let mut unit = [0i64; 6];
    unit.copy_from_slice(&op[5..11]);
    Ok(Call {
        code: op[0],
        now: op[1],
        quiet: op[2],
        origin: op[3],
        size: op[4],
        unit,
        sk: key_of(&op[12..e1]),
        bk: key_of(&op[e1 + 1..e2]),
        uk: key_of(&op[e2 + 1..e3]),
    })
}

fn origin_of(o: i64) -> Option<Origin> {
    match o {
        0 => Some(Origin::User),
        1 => Some(Origin::Schedule),
        2 => Some(Origin::Direct),
        3 => Some(Origin::Release),
        _ => None,
    }
}

fn size_of(z: i64) -> Option<Size> {
    match z {
        -1 => Some(Size::Absent),
        0 => Some(Size::Unlimited),
        1 | 2 | 4 | 8 => Some(Size::Times(z)),
        _ => None,
    }
}

fn dispatch<'a>(w: &'a mut World, c: &Call) -> Result<Res<'a>, i64> {
    let sk = c.sk.as_deref();
    let bk = c.bk.as_deref();
    match c.code {
        1 => Ok(match (sk, bk) {
            (Some(s), Some(b)) => match w.current(s, b) {
                Some(x) => Res::View(x),
                None => Res::Undef,
            },
            _ => Res::Undef,
        }),
        2 => Ok(Res::Bool(match (sk, bk) {
            (Some(s), Some(b)) => w.is_halted(s, b),
            _ => false,
        })),
        5 => match (sk, bk) {
            (Some(s), Some(b)) => w.reopen(s, b),
            _ => Ok(Res::Bool(false)),
        },
        6 => match sk {
            Some(s) => w.maybe_close(s, c.quiet, c.now),
            None => Err(2),
        },
        3 => {
            let origin = origin_of(c.origin).ok_or(2)?;
            let size = size_of(c.size).ok_or(2)?;
            let s = sk.ok_or(2)?;
            w.attach(s, origin, size, &c.unit, c.uk.as_deref(), c.now)
        }
        4 => {
            let origin = origin_of(if c.origin < 0 { 0 } else { c.origin }).ok_or(2)?;
            let size = size_of(c.size).ok_or(2)?;
            let s = sk.ok_or(2)?;
            w.replace(s, origin, size, &c.unit, c.uk.as_deref(), c.now)
        }
        _ => Err(2),
    }
}

// The result words the driver prints after `V `: [0] undefined, [1] false, [2] true, [3, len, units..(, oldLen,
// units..)] ref (the old id when carried), [4, n, record words..] budget view.
fn res_words(r: &Res) -> Vec<i64> {
    match r {
        Res::Undef => vec![0],
        Res::Bool(false) => vec![1],
        Res::Bool(true) => vec![2],
        Res::Ref(id, carry) => {
            let mut v = vec![3];
            push_str(&mut v, id);
            if let Some(c) = carry {
                push_str(&mut v, c);
            }
            v
        }
        Res::View(b) => {
            let r = encode(b);
            let mut v = vec![4, r.len() as i64];
            v.extend(r);
            v
        }
    }
}

struct Host {
    w: World,
    now: i64,
    uuids: Vec<Vec<i64>>, // keys `len, units...`
    next_uuid: usize,
    quiet: Vec<i64>,
    next_quiet: usize,
    log: Vec<String>,
}

impl Host {
    fn new() -> Host {
        Host { w: World::new(), now: 1700000000000, uuids: Vec::new(), next_uuid: 0, quiet: Vec::new(), next_quiet: 0, log: Vec::new() }
    }

    // t_put: [sessionId key, n, record words...]; a malformed frame stops the driver (status 2), like driver_m7.
    fn put(&mut self, op: &[i64]) {
        let framed = match key_end(op, 0) {
            Some(e1) if e1 < op.len() => {
                let n = op[e1];
                if n >= 0 && n == (op.len() - e1 - 1) as i64 {
                    Some(e1)
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(e1) = framed else {
            eprintln!("t_put status 2");
            std::process::exit(2);
        };
        let Some(key) = key_of(&op[1..e1]) else { unsupported("session key unit outside 0..65535") };
        let slot = decode(&op[e1 + 1..]);
        self.w.put(&key, slot);
    }

    // t_del: [sessionId key]; a malformed key frame logs `X del 2` and the case goes on.
    fn del(&mut self, op: &[i64]) {
        match key_end(op, 0) {
            None => self.log.push("X del 2".to_string()),
            Some(e1) => {
                if let Some(k) = key_of(&op[1..e1]) {
                    self.w.del(&k);
                }
            }
        }
    }

    // One public call: body = [code, origin, size, unit x6, sessionId key, budgetId key]; the host inserts now, the next
    // isQuiet answer and the next UUID key (an empty key when none is left), exactly as driver_m7.rs World::take_op.
    fn call(&mut self, body: &[i64]) {
        let quiet = self.quiet.get(self.next_quiet).copied().unwrap_or(0);
        let mut op: Vec<i64> = Vec::with_capacity(body.len() + 48);
        op.push(body[0]);
        op.push(self.now);
        op.push(quiet);
        op.extend_from_slice(&body[1..]);
        match self.uuids.get(self.next_uuid) {
            Some(u) => op.extend_from_slice(u),
            None => op.push(0),
        }
        self.w.fx.clear();
        let (status, words) = match decode_call(&op) {
            Err(s) => (s, Vec::new()),
            Ok(c) => match dispatch(&mut self.w, &c) {
                Ok(r) => (0, res_words(&r)),
                Err(s) => (s, Vec::new()),
            },
        };
        if status != 0 {
            // A finished call that consulted isQuiet consumed the answer even when it failed afterwards.
            if self.w.fx.iter().any(|e| matches!(e, Effect::Quiet)) {
                self.next_quiet += 1;
                self.log.push(format!("E quiet {}", quiet));
            }
            self.log.push(format!("X {}", status));
            return;
        }
        for e in &self.w.fx {
            match e {
                Effect::Uuid => self.next_uuid += 1,
                Effect::Quiet => self.next_quiet += 1,
                _ => {}
            }
            self.log.push(match e {
                Effect::Changed => "E changed".to_string(),
                Effect::CarryHeld => "E carry".to_string(),
                Effect::Uuid => "E uuid".to_string(),
                Effect::Quiet => format!("E quiet {}", quiet),
                Effect::TimerClear => "E clear".to_string(),
            });
        }
        let ws: Vec<String> = words.iter().map(|x| x.to_string()).collect();
        self.log.push(format!("V {}", ws.join(" ")));
    }

    fn observe_line(&self) -> String {
        let mut s = String::from("O");
        for x in self.w.observe() {
            s.push(' ');
            s.push_str(&x.to_string());
        }
        s
    }
}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

// A run of keys `len, units...` into separate keys (as driver_m7.rs keys).
fn keys(w: &[i64]) -> Vec<Vec<i64>> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < w.len() {
        let n = w[p] as usize;
        v.push(w[p..p + 1 + n].to_vec());
        p += 1 + n;
    }
    v
}

fn run_q() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut h = Host::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                h = Host::new();
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "now" => h.now = p[1].parse().unwrap(),
            "uuids" => h.uuids = keys(&nums(&p[1..])),
            "quiet" => h.quiet = nums(&p[1..]),
            "put" => h.put(&nums(&p[1..])),
            "del" => h.del(&nums(&p[1..])),
            "adv" => h.now += p[1].parse::<i64>().unwrap(),
            "call" => {
                h.log.push(format!("@ {}", p[1]));
                h.call(&nums(&p[2..]));
            }
            "end" => {
                for l in h.log.drain(..) {
                    writeln!(o, "{}", l).unwrap();
                }
                writeln!(o, "{}", h.observe_line()).unwrap();
                writeln!(o, "end").unwrap();
            }
            other => {
                eprintln!("bad line {}", other);
                std::process::exit(2);
            }
        }
    }
}

// ---- perf mode: same items and shapes as driver_m7.rs run_perf ----
//
// S sessions s0000..s<S-1>, each with an open budget (rest text of a typical card). Items (ns per call):
//   ishalted    : isHalted(ref) of the last session
//   current     : current(ref) of the last session (hit; the borrowed budget, nothing copied)
//   attach      : attach on an open budget of the last session (ref result)
//   closereopen : maybeClose(quiet) then reopen on the last session (2 calls per rep)
//   replace     : replaceForUserMessage on a halted budget of the last session (CLOSE + OPEN + carryHeld), then the
//                 halted record is put back (timed together, both sides)
// Per call the hosts reset the UUID / isQuiet streams (here: the constants passed) and clear the effect list, as
// driver_m7 resets next_uuid / next_quiet / fx[0]. Keys are encoded once at setup (the Pruv driver prepares its ops
// once). The put-back of `replace` clones a prepared typed record (3 allocations), where Pruv copies words into the
// state buffer; that is what restoring a typed record costs.

const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn rec(id: &str, state: State) -> Budget {
    Budget {
        id: u16s(id),
        state,
        origin: Origin::User,
        unlimited: Unlimited::Absent,
        limits: [120, 24, 3, 2, 1],
        used: [0; 8],
        active_since: Stamp::Absent,
        ended: None,
        opened_at: Stamp::Text(u16s("2023-11-14T22:13:20.000Z")),
        follows: Follows::Absent,
        rest: u16s(PERF_REST),
    }
}

fn run_perf(args: &[String]) {
    let size: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let sids: Vec<Key> = (0..size).map(|i| u16s(&format!("s{:04}", i))).collect();
    let bid = |i: usize| format!("{:08x}-0000-4000-8000-000000000001", i);
    let unit = [120i64, 24, 3, 2, 1, 0];
    let uuid_key = u16s("00000000-0000-4000-8000-0000000000aa");
    let uuid: Option<&[u16]> = Some(&uuid_key);
    let now = 1700000000000i64;
    let setup = |state: State| -> World {
        let mut w = World::new();
        for (i, s) in sids.iter().enumerate() {
            w.put(s, Slot::Budget(rec(&bid(i), state)));
        }
        w
    };
    let last = size - 1;
    let sid: &[u16] = &sids[last];
    let lbid = u16s(&bid(last));
    let lbid: &[u16] = &lbid;

    if on("ishalted") {
        let w = setup(State::Open);
        for _ in 0..1000 {
            black_box(w.is_halted(black_box(sid), black_box(lbid)));
        }
        let t = Instant::now();
        for _ in 0..reps {
            black_box(w.is_halted(black_box(sid), black_box(lbid)));
        }
        println!("ishalted {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
        println!("chk ishalted {} {}", size, w.checksum());
    }

    if on("current") {
        let w = setup(State::Open);
        for _ in 0..1000 {
            black_box(w.current(black_box(sid), black_box(lbid)));
        }
        let t = Instant::now();
        for _ in 0..reps {
            black_box(w.current(black_box(sid), black_box(lbid)));
        }
        println!("current {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
        println!("chk current {} {}", size, w.checksum());
    }

    if on("attach") {
        let mut w = setup(State::Open);
        let mut go = |w: &mut World| {
            w.fx.clear();
            black_box(w.attach(black_box(sid), Origin::User, Size::Absent, &unit, uuid, now).is_ok());
        };
        for _ in 0..1000 {
            go(&mut w);
        }
        let t = Instant::now();
        for _ in 0..reps {
            go(&mut w);
        }
        println!("attach {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
        println!("chk attach {} {}", size, w.checksum());
    }

    if on("closereopen") {
        let mut w = setup(State::Open);
        let mut go = |w: &mut World| {
            w.fx.clear();
            black_box(w.maybe_close(black_box(sid), black_box(1), now).is_ok());
            w.fx.clear();
            black_box(w.reopen(black_box(sid), black_box(lbid)).is_ok());
        };
        for _ in 0..1000 {
            go(&mut w);
        }
        let t = Instant::now();
        for _ in 0..reps {
            go(&mut w);
        }
        println!("closereopen {} {:.1}", size, t.elapsed().as_nanos() as f64 / (2 * reps) as f64);
        println!("chk closereopen {} {}", size, w.checksum());
    }

    if on("replace") {
        let mut w = setup(State::Halted);
        let halted = rec(&bid(last), State::Halted);
        let mut go = |w: &mut World| {
            w.fx.clear();
            black_box(w.replace(black_box(sid), Origin::User, Size::Times(2), &unit, uuid, now).is_ok());
            w.put(black_box(sid), Slot::Budget(halted.clone()));
        };
        for _ in 0..1000 {
            go(&mut w);
        }
        let t = Instant::now();
        for _ in 0..reps {
            go(&mut w);
        }
        println!("replace {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
        println!("chk replace {} {}", size, w.checksum());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

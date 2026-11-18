// Optimized hand baseline for M8 (same structure as the Pruv store, hand-tuned, typed SAFE Rust, no unsafe):
//   * open addressing over packed records: a power-of-two slot table of u32 indices with linear probing
//     (slot = (hash + i) & mask), keys in flat arenas of u16 units, never one allocation per key;
//   * sessions: a packed struct array (SRec) in insertion order + one slot table; sessions are never deleted, so the
//     array order IS the observation order;
//   * turns: a second open-addressing table (with tombstones) over a TRec array with a free list; every turn carries
//     a sequence number (observation order) and a generation;
//   * turn -> activity sets: a third open-addressing table keyed by (turn index, turn generation, activity key). A
//     forgetTurn is O(1): it bumps the turn's generation, so the turn's activity records become stale (never match,
//     dropped and their key units compacted at the next activity-table rehash).
//   * hash: the same function as Pruv's bud_map hash_key (gen.rs): h0 = mix(1469598103934665603, owner + ogen*2^32),
//     mix(h, v) = (h ^ v) * -7046029254386353131 (wrapping), 4 units packed by pack4 (tail zero padded), fin(h ^ n)
//     with fin = (h ^ h>>33) * -49064778989728563, then ^ >>33. The owner / generation words differ from Pruv's entry
//     numbers (sessions owner 0, turns owner 2, generation 0; activities owner = turn index, generation = turn
//     generation): hash values are not observable.
//   * semantics identical to hand_m8_u16.rs (safe-integer refusal, atomic charge and noteTool refusal,
//     malformed ingress = status 2). Table growth is internal: no status -N / Grow retry exists here, which is not
//     observable in q output.
//
//   hand_m8_oa perf  <N> <reps> [item]   pre-decoded keys (UNMATCHED boundary), prints `item size ns` + `chk item size v`
//   hand_m8_oa perfw <N> <reps> [item]   MATCHED wire boundary: the i64 op words of driver_m8.rs run_perf, frame checks,
//                                        O(1) canonical packed-key checks, packed-u16 words unpacked (4 lanes per word)
//                                        into a reused scratch Vec<u16>, then the same methods
//   hand_m8_oa q                         driver_m8.rs q protocol, byte-identical output
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// ---- safe-integer arithmetic (bud_charge.pruv `sum`) ----

const MAX_SAFE: i64 = 9_007_199_254_740_991; // 2^53 - 1

#[inline(always)]
fn is_safe(x: i64) -> bool {
    (-MAX_SAFE..=MAX_SAFE).contains(&x)
}

#[inline(always)]
fn safe_add(u: i64, a: i64) -> Option<i64> {
    if !is_safe(u) || !is_safe(a) {
        return None;
    }
    let r = u.checked_add(a)?;
    is_safe(r).then_some(r)
}

// ---- hash: mirror of gen.rs bud_map mix / fin / pack4 / hash_key ----

#[inline(always)]
fn mix(h: i64, v: i64) -> i64 {
    (h ^ v).wrapping_mul(-7046029254386353131i64)
}

#[inline(always)]
fn fin(h: i64) -> i64 {
    let a = h ^ (((h as u64) >> 33) as i64);
    let b = a.wrapping_mul(-49064778989728563i64);
    b ^ (((b as u64) >> 33) as i64)
}

#[inline(always)]
fn pack4(u0: i64, u1: i64, u2: i64, u3: i64) -> i64 {
    u0.wrapping_add(u1.wrapping_mul(65536)).wrapping_add(u2.wrapping_mul(4294967296).wrapping_add(u3.wrapping_mul(281474976710656)))
}

#[inline]
fn hash_key(owner: i64, ogen: i64, key: &[u16]) -> i64 {
    let mut h = mix(1469598103934665603i64, owner.wrapping_add(ogen.wrapping_mul(4294967296)));
    let mut c = key.chunks_exact(4);
    for q in &mut c {
        h = mix(h, pack4(q[0] as i64, q[1] as i64, q[2] as i64, q[3] as i64));
    }
    let r = c.remainder();
    if !r.is_empty() {
        let u1 = r.get(1).map_or(0, |&u| u as i64);
        let u2 = r.get(2).map_or(0, |&u| u as i64);
        h = mix(h, pack4(r[0] as i64, u1, u2, 0));
    }
    fin(h ^ (key.len() as i64))
}

// ---- sessions ----

const NONE: u32 = u32::MAX;
const ST_NOBUDGET: u8 = 3; // record with state -1: kept as words, CURRENT never matches

#[derive(Clone, Copy)]
struct SRec {
    hash: i64,
    key_off: u32,
    key_len: u32,
    bid_off: u32,
    bid_len: u32,
    opaque: u32, // index into Sessions::opaque, NONE for a typed budget
    state: u8,   // 0 open, 1 halted, 2 closed, 3 no budget
    unlimited: bool,
    halt: i8, // -1 none, 0 time, 1 delegations, 2 commands, 3 reviews, 4 checks
    limits: [i64; 4],
    used: [i64; 4],
    tools: i64,
    failures: i64,
    retries: i64,
}

struct Sessions {
    recs: Vec<SRec>,
    slots: Vec<u32>, // 0 empty, else record index + 1
    keys: Vec<u16>,  // arena: session keys and budget ids
    opaque: Vec<Box<[i64]>>,
}

fn idx32(x: usize) -> u32 {
    u32::try_from(x).expect("arena exceeds u32 offsets")
}

// ---- packed-u16 key codec (bud_keys.pruv; same helpers as hand_m8_u16.rs) ----
// n UTF-16 units (0..=2^30) are m = ceil(n/4) i64 words; word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3]
// (two's complement); lanes past n are zero. Any word is 4 valid u16 lanes, so there is no per-unit range check.

const MAX_KEY_UNITS: i64 = 1 << 30;

#[inline]
fn words_of(n: i64) -> Option<usize> {
    if (0..=MAX_KEY_UNITS).contains(&n) {
        Some((n as usize + 3) / 4)
    } else {
        None
    }
}

// the unused tail lanes of the last word of an n-unit key (n % 4 != 0) must be zero (canonical form)
#[inline]
fn tail_ok(n: i64, last: i64) -> bool {
    let r = (n & 3) as u32;
    r == 0 || (last as u64) & ((1u64 << (16 * (4 - r))) - 1) == 0
}

// O(1) canonical check of a packed key of n units held in exactly `w`
#[inline]
fn packed_ok(n: i64, w: &[i64]) -> bool {
    words_of(n) == Some(w.len()) && (w.is_empty() || tail_ok(n, w[w.len() - 1]))
}

// unpacks the n units of (canonical) packed words `w` onto the end of `sc`
#[inline]
fn unpack_units(sc: &mut Vec<u16>, n: usize, w: &[i64]) {
    for &x in w {
        let u = x as u64;
        sc.extend_from_slice(&[(u >> 48) as u16, (u >> 32) as u16, (u >> 16) as u16, u as u16]);
    }
    sc.truncate(sc.len() - (4 * w.len() - n));
}

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

// The ingress check of root_m8.pruv t_put on record words (see hand_m8_u16.rs valid_record): len == 15 + ceil(bn/4),
// w[14] == bn in 0..=2^30, canonical packed id.
fn valid_record(w: &[i64]) -> bool {
    w.len() >= 15
        && (-1..=2).contains(&w[0])
        && (0..=1).contains(&w[1])
        && (-1..=4).contains(&w[2])
        && packed_ok(w[14], &w[15..])
}

impl Sessions {
    fn new() -> Sessions {
        Sessions { recs: Vec::new(), slots: vec![0; 64], keys: Vec::new(), opaque: Vec::new() }
    }

    #[inline(always)]
    fn key_of(&self, r: &SRec) -> &[u16] {
        &self.keys[r.key_off as usize..(r.key_off + r.key_len) as usize]
    }

    #[inline]
    fn find(&self, h: i64, key: &[u16]) -> Option<usize> {
        let mask = self.slots.len() - 1;
        let mut i = (h as usize) & mask;
        loop {
            let s = self.slots[i];
            if s == 0 {
                return None;
            }
            let r = &self.recs[(s - 1) as usize];
            if r.hash == h && self.key_of(r) == key {
                return Some((s - 1) as usize);
            }
            i = (i + 1) & mask;
        }
    }

    fn insert_slot(&mut self, h: i64, idx: usize) {
        let mask = self.slots.len() - 1;
        let mut i = (h as usize) & mask;
        while self.slots[i] != 0 {
            i = (i + 1) & mask;
        }
        self.slots[i] = idx32(idx + 1);
    }

    fn grow_slots(&mut self) {
        let cap = self.slots.len() * 2;
        self.slots = vec![0; cap];
        for k in 0..self.recs.len() {
            let h = self.recs[k].hash;
            self.insert_slot(h, k);
        }
    }

    // session put of a record that passed valid_record: a new key is appended (insertion order), an existing key keeps
    // its position and key units; state -1 records are kept as words.
    fn put_rec(&mut self, key: &[u16], w: &[i64]) {
        let h = hash_key(0, 0, key);
        let at = self.find(h, key);
        let (key_off, key_len) = match at {
            Some(i) => (self.recs[i].key_off, self.recs[i].key_len),
            None => {
                let off = idx32(self.keys.len());
                self.keys.extend_from_slice(key);
                (off, idx32(key.len()))
            }
        };
        let mut rec = SRec {
            hash: h,
            key_off,
            key_len,
            bid_off: 0,
            bid_len: 0,
            opaque: NONE,
            state: ST_NOBUDGET,
            unlimited: false,
            halt: -1,
            limits: [0; 4],
            used: [0; 4],
            tools: 0,
            failures: 0,
            retries: 0,
        };
        let old_opaque = at.map_or(NONE, |i| self.recs[i].opaque);
        if w[0] == -1 {
            let boxed: Box<[i64]> = w.into();
            if old_opaque != NONE {
                self.opaque[old_opaque as usize] = boxed;
                rec.opaque = old_opaque;
            } else {
                rec.opaque = idx32(self.opaque.len());
                self.opaque.push(boxed);
            }
        } else {
            // the budget id arrives as packed words (bn = w[14] units): unpacked here (put path, not timed)
            let mut ids: Vec<u16> = Vec::with_capacity(w[14] as usize);
            unpack_units(&mut ids, w[14] as usize, &w[15..]);
            let (old_off, old_len) = at.map_or((0u32, 0u32), |i| (self.recs[i].bid_off, self.recs[i].bid_len));
            if ids.len() <= old_len as usize {
                self.keys[old_off as usize..old_off as usize + ids.len()].copy_from_slice(&ids);
                rec.bid_off = old_off;
            } else {
                rec.bid_off = idx32(self.keys.len());
                self.keys.extend_from_slice(&ids);
            }
            rec.bid_len = idx32(ids.len());
            rec.state = w[0] as u8;
            rec.unlimited = w[1] == 1;
            rec.halt = w[2] as i8;
            rec.limits = [w[3], w[4], w[5], w[6]];
            rec.used = [w[7], w[8], w[9], w[10]];
            rec.tools = w[11];
            rec.failures = w[12];
            rec.retries = w[13];
        }
        match at {
            Some(i) => self.recs[i] = rec,
            None => {
                if (self.recs.len() + 1) * 2 > self.slots.len() {
                    self.grow_slots();
                }
                self.recs.push(rec);
                let last = self.recs.len() - 1;
                self.insert_slot(h, last);
            }
        }
    }

    // CURRENT(ref): the session exists, holds a budget, and its budget id equals the ref's.
    #[inline]
    fn current(&self, session: &[u16], budget: &[u16]) -> Option<usize> {
        let h = hash_key(0, 0, session);
        let i = self.find(h, session)?;
        let r = &self.recs[i];
        if r.state != ST_NOBUDGET && &self.keys[r.bid_off as usize..(r.bid_off + r.bid_len) as usize] == budget {
            Some(i)
        } else {
            None
        }
    }

    fn record_words(&self, r: &SRec) -> Vec<i64> {
        if r.state == ST_NOBUDGET {
            return self.opaque[r.opaque as usize].to_vec();
        }
        let mut v = vec![r.state as i64, r.unlimited as i64, r.halt as i64];
        v.extend_from_slice(&r.limits);
        v.extend_from_slice(&r.used);
        v.extend_from_slice(&[r.tools, r.failures, r.retries, r.bid_len as i64]);
        // the id is re-emitted as packed words, like the record that was put
        v.extend(pack_u16(&self.keys[r.bid_off as usize..(r.bid_off + r.bid_len) as usize]));
        v
    }
}

// ---- turns and activities ----

#[derive(Clone, Copy)]
struct TRec {
    hash: i64,
    key_off: u32,
    key_len: u32,
    seq: u64,
    gen: u32,
    alive: bool,
}

#[derive(Clone, Copy)]
struct ARec {
    hash: i64,
    key_off: u32,
    key_len: u32,
    ti: u32,
    gen: u32,
}

struct Turns {
    recs: Vec<TRec>,
    free: Vec<u32>,
    slots: Vec<u32>, // 0 empty, 1 tombstone, else turn index + 2
    used: usize,     // live + tombstones
    keys: Vec<u16>,
    dead_units: usize,
    next_seq: u64,
    acts: Vec<ARec>,
    aslots: Vec<u32>, // 0 empty, else activity index + 1 (no tombstones: stale records stay until the rehash)
    akeys: Vec<u16>,
}

impl Turns {
    fn new() -> Turns {
        Turns {
            recs: Vec::new(),
            free: Vec::new(),
            slots: vec![0; 16],
            used: 0,
            keys: Vec::new(),
            dead_units: 0,
            next_seq: 0,
            acts: Vec::new(),
            aslots: vec![0; 16],
            akeys: Vec::new(),
        }
    }

    // (slot, turn index)
    #[inline]
    fn find_turn(&self, h: i64, key: &[u16]) -> Option<(usize, usize)> {
        let mask = self.slots.len() - 1;
        let mut i = (h as usize) & mask;
        loop {
            let s = self.slots[i];
            if s == 0 {
                return None;
            }
            if s >= 2 {
                let r = &self.recs[(s - 2) as usize];
                if r.hash == h && &self.keys[r.key_off as usize..(r.key_off + r.key_len) as usize] == key {
                    return Some((i, (s - 2) as usize));
                }
            }
            i = (i + 1) & mask;
        }
    }

    // drops tombstones and dead key units: rebuilds the slot table (load <= 1/4) and the key arena
    fn rehash_turns(&mut self) {
        let live = self.recs.iter().filter(|r| r.alive).count();
        let mut cap = 16usize;
        while cap < live * 4 + 4 {
            cap *= 2;
        }
        let mask = cap - 1;
        self.slots = vec![0; cap];
        let mut nk: Vec<u16> = Vec::with_capacity(self.keys.len() - self.dead_units.min(self.keys.len()));
        for idx in 0..self.recs.len() {
            let r = self.recs[idx];
            if !r.alive {
                continue;
            }
            let off = idx32(nk.len());
            nk.extend_from_slice(&self.keys[r.key_off as usize..(r.key_off + r.key_len) as usize]);
            self.recs[idx].key_off = off;
            let mut i = (r.hash as usize) & mask;
            while self.slots[i] != 0 {
                i = (i + 1) & mask;
            }
            self.slots[i] = idx32(idx + 2);
        }
        self.keys = nk;
        self.used = live;
        self.dead_units = 0;
    }

    // the key is absent: new turn with the next sequence number
    fn add_turn(&mut self, h: i64, key: &[u16]) -> usize {
        if (self.used + 1) * 2 > self.slots.len() || (self.dead_units > 1024 && self.dead_units * 2 > self.keys.len()) {
            self.rehash_turns();
        }
        let off = idx32(self.keys.len());
        self.keys.extend_from_slice(key);
        let rec = TRec { hash: h, key_off: off, key_len: idx32(key.len()), seq: self.next_seq, gen: 0, alive: true };
        self.next_seq += 1;
        let idx = match self.free.pop() {
            Some(f) => {
                let g = self.recs[f as usize].gen; // already bumped by forget
                self.recs[f as usize] = TRec { gen: g, ..rec };
                f as usize
            }
            None => {
                self.recs.push(rec);
                self.recs.len() - 1
            }
        };
        let mask = self.slots.len() - 1;
        let mut i = (h as usize) & mask;
        while self.slots[i] >= 2 {
            i = (i + 1) & mask;
        }
        if self.slots[i] == 0 {
            self.used += 1;
        }
        self.slots[i] = idx32(idx + 2);
        idx
    }

    fn forget(&mut self, h: i64, key: &[u16]) {
        if let Some((slot, idx)) = self.find_turn(h, key) {
            self.slots[slot] = 1;
            let r = &mut self.recs[idx];
            r.alive = false;
            r.gen = r.gen.wrapping_add(1);
            self.dead_units += r.key_len as usize;
            self.free.push(idx32(idx));
        }
    }

    #[inline]
    fn find_act(&self, h: i64, ti: u32, gen: u32, key: &[u16]) -> bool {
        let mask = self.aslots.len() - 1;
        let mut i = (h as usize) & mask;
        loop {
            let s = self.aslots[i];
            if s == 0 {
                return false;
            }
            let a = &self.acts[(s - 1) as usize];
            if a.hash == h
                && a.ti == ti
                && a.gen == gen
                && &self.akeys[a.key_off as usize..(a.key_off + a.key_len) as usize] == key
            {
                return true;
            }
            i = (i + 1) & mask;
        }
    }

    // keeps the activities of live turns (generation equals the turn's), compacts their key units, rebuilds the table
    fn rehash_acts(&mut self) {
        let old = std::mem::take(&mut self.acts);
        let oldk = std::mem::take(&mut self.akeys);
        let live = old.iter().filter(|a| a.gen == self.recs[a.ti as usize].gen).count();
        let mut cap = 16usize;
        while cap < live * 4 + 4 {
            cap *= 2;
        }
        let mask = cap - 1;
        let mut slots = vec![0u32; cap];
        let mut acts: Vec<ARec> = Vec::with_capacity(live);
        let mut keys: Vec<u16> = Vec::new();
        for a in old {
            if a.gen != self.recs[a.ti as usize].gen {
                continue;
            }
            let off = idx32(keys.len());
            keys.extend_from_slice(&oldk[a.key_off as usize..(a.key_off + a.key_len) as usize]);
            acts.push(ARec { key_off: off, ..a });
            let mut i = (a.hash as usize) & mask;
            while slots[i] != 0 {
                i = (i + 1) & mask;
            }
            slots[i] = idx32(acts.len());
        }
        self.acts = acts;
        self.akeys = keys;
        self.aslots = slots;
    }

    // true when the activity was new for turn `ti`
    #[inline]
    fn add_act(&mut self, ti: usize, key: &[u16]) -> bool {
        let gen = self.recs[ti].gen;
        let h = hash_key(ti as i64, gen as i64, key);
        if self.find_act(h, idx32(ti), gen, key) {
            return false;
        }
        if (self.acts.len() + 1) * 2 > self.aslots.len() {
            self.rehash_acts();
        }
        let off = idx32(self.akeys.len());
        self.akeys.extend_from_slice(key);
        self.acts.push(ARec { hash: h, key_off: off, key_len: idx32(key.len()), ti: idx32(ti), gen });
        let mask = self.aslots.len() - 1;
        let mut i = (h as usize) & mask;
        while self.aslots[i] != 0 {
            i = (i + 1) & mask;
        }
        self.aslots[i] = idx32(self.acts.len());
        true
    }
}

// ---- world ----

// `what` of charge: `what[key] ?? 0` is applied at use (None = missing or null).
#[derive(Default)]
struct What {
    counts: [Option<i64>; 4],
    retries: Option<i64>,
}

enum Charge {
    Ok,
    Nothing,
    Halted(usize),
    OverLimit(usize),
    ResourceBound,
}

struct ResourceBound;

struct World {
    s: Sessions,
    t: Turns,
}

impl World {
    fn new() -> World {
        World { s: Sessions::new(), t: Turns::new() }
    }

    fn charge(&mut self, session: &[u16], budget: &[u16], what: &What) -> Charge {
        let i = match self.s.current(session, budget) {
            Some(i) if self.s.recs[i].state != 2 => i,
            _ => return Charge::Nothing,
        };
        let b = &mut self.s.recs[i];
        if b.state == 1 {
            return Charge::Halted(if b.halt < 0 { 1 } else { b.halt as usize });
        }
        let a: [i64; 4] = std::array::from_fn(|k| what.counts[k].unwrap_or(0));
        let ar = what.retries.unwrap_or(0);
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

    fn note_failure(&mut self, session: &[u16], budget: &[u16]) -> Result<(), ResourceBound> {
        if let Some(i) = self.s.current(session, budget) {
            let b = &mut self.s.recs[i];
            b.failures = safe_add(b.failures, 1).ok_or(ResourceBound)?;
        }
        Ok(())
    }

    // a seen activity returns at once (before CURRENT); a new one is recorded even with a missing or stale budget,
    // unless the current budget's used.tools is at the safe-integer bound: that ResourceBound refuses before anything
    // is written (KD-M8-1), as hand_m8_u16.rs / root_m8.pruv
    fn note_tool(&mut self, session: &[u16], budget: &[u16], turn: &[u16], act: &[u16]) -> Result<(), ResourceBound> {
        let th = hash_key(2, 0, turn);
        let found = self.t.find_turn(th, turn).map(|(_, i)| i);
        if let Some(ti) = found {
            let gen = self.t.recs[ti].gen;
            if self.t.find_act(hash_key(ti as i64, gen as i64, act), idx32(ti), gen, act) {
                return Ok(());
            }
        }
        let cur = self.s.current(session, budget);
        if let Some(i) = cur {
            safe_add(self.s.recs[i].tools, 1).ok_or(ResourceBound)?;
        }
        let ti = match found {
            Some(i) => i,
            None => self.t.add_turn(th, turn),
        };
        self.t.add_act(ti, act);
        if let Some(i) = cur {
            self.s.recs[i].tools += 1;
        }
        Ok(())
    }

    fn forget_turn(&mut self, turn: &[u16]) {
        let th = hash_key(2, 0, turn);
        self.t.forget(th, turn);
    }

    // Whole-state observation words (same layout as hand_m8_u16.rs / bud_world.pruv observe): sessions in array
    // order, clocks 0, live turns by sequence number, each with its live activities in pool (= insertion) order.
    fn observe(&self) -> Vec<i64> {
        fn push_key(out: &mut Vec<i64>, k: &[u16]) {
            out.push(k.len() as i64);
            out.extend(k.iter().map(|&u| u as i64));
        }
        let mut out = vec![self.s.recs.len() as i64];
        for r in &self.s.recs {
            push_key(&mut out, self.s.key_of(r));
            let rec = self.s.record_words(r);
            out.push(rec.len() as i64);
            out.extend(rec);
        }
        out.push(0);
        let t = &self.t;
        let mut order: Vec<usize> = (0..t.recs.len()).filter(|&i| t.recs[i].alive).collect();
        order.sort_by_key(|&i| t.recs[i].seq);
        let mut by: Vec<Vec<usize>> = vec![Vec::new(); t.recs.len()];
        for (k, a) in t.acts.iter().enumerate() {
            let ti = a.ti as usize;
            if a.gen == t.recs[ti].gen && t.recs[ti].alive {
                by[ti].push(k);
            }
        }
        out.push(order.len() as i64);
        for ti in order {
            let r = &t.recs[ti];
            push_key(&mut out, &t.keys[r.key_off as usize..(r.key_off + r.key_len) as usize]);
            out.push(by[ti].len() as i64);
            for &k in &by[ti] {
                let a = &t.acts[k];
                push_key(&mut out, &t.akeys[a.key_off as usize..(a.key_off + a.key_len) as usize]);
            }
        }
        out
    }

    fn checksum(&self) -> u64 {
        self.observe().iter().fold(0u64, |s, &x| s.wrapping_mul(31).wrapping_add(x as u64))
    }
}

// ---- wire boundary (same frame protocol as hand_m8_u16.rs call_wire) ----

const FX_LEN: usize = 256; // driver_m8.rs World::new: fx = 256 words

struct Fr {
    r: i64,
    k: i64,
    e2: usize,
}

// the end of the packed key `n, packed words(n)` at p: O(1) canonical check (n in 0..=2^30, the ceil(n/4) words fit
// inside op, unused tail lanes of the last word zero). No per-unit range check exists.
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

// unpacks the key at p (validated by key_span, end e) to u16 units appended to the scratch buffer
#[inline]
fn push_key(sc: &mut Vec<u16>, op: &[i64], p: usize, e: usize) {
    unpack_units(sc, op[p] as usize, &op[p + 1..e]);
}

fn what_of(w: &[i64]) -> What {
    let amount = |k: usize| if w[2 * k] == 2 { Some(w[2 * k + 1]) } else { None };
    What { counts: [amount(0), amount(1), amount(2), amount(3)], retries: amount(4) }
}

impl World {
    // status as Pruv's t_call: 0 done, 2 invalid input, 3 halt frame does not fit fx, 5 ResourceBound, 6 halt dispatch
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

// ---- q mode ----

// root_m8.pruv key_end: the end of the packed key at p (same O(1) canonical check as key_span; put framing only)
fn key_end(op: &[i64], p: usize) -> Option<usize> {
    key_span(op, p)
}

// the units of the (validated) packed key at p..e
fn units(op: &[i64], p: usize, e: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(op[p] as usize);
    push_key(&mut v, op, p, e);
    v
}

fn run_call(w: &mut World, words: &[i64], sc: &mut Vec<u16>, o: &mut impl Write) -> bool {
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

// session put words: [sessionId key, n, record words...]; 0 stored, 2 malformed (state unchanged)
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
    w.s.put_rec(&key, &op[rs..]);
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

// ---- perf ----

fn u16s(s: &str) -> Box<[u16]> {
    s.encode_utf16().collect()
}

fn bid_of(i: usize) -> String {
    format!("99b568f7-0000-4000-8000-{:012}", i)
}

// wire key `n, packed words(n)` (built at setup, outside the timed loops)
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

// N sessions through the same ingress words as driver_m8.rs world_n
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

struct Ref {
    session: Box<[u16]>,
    budget: Box<[u16]>,
}

fn refk(i: usize) -> Ref {
    Ref { session: u16s(&format!("session-{:05}", i)), budget: u16s(&bid_of(i)) }
}

// perf <N> <reps> [item]: items as driver_m8.rs run_perf (charge / failure / tooldup / toolgrow), keys pre-decoded
fn run_perf(args: &[String]) {
    let n: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut w = world_wire(n);
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
        println!("chk charge {} {}", n, w.checksum());
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
    // charge and failure are timed before any tool setup runs, so a run that names one of them stops here; the tool
    // setup below would only add its own state to that item's peak RSS (in every lane alike).
    if matches!(only, Some("charge") | Some("failure")) {
        return;
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
    black_box(&w.s.recs);
}

// perfw <N> <reps> [item]: the same items, the timed loop takes the driver's i64 op words (matched boundary)
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
    // charge and failure are timed before any tool setup runs, so a run that names one of them stops here; the tool
    // setup below would only add its own state to that item's peak RSS (in every lane alike).
    if matches!(only, Some("charge") | Some("failure")) {
        return;
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
    black_box(&w.s.recs);
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

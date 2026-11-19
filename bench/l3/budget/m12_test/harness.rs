// M12 line protocol is documented in ABI.md.
#[path = "@SOL@"] mod sol;
#[path = "@HAND@"] mod hand;
#[path = "@PERF_LIFE@"] mod perf_life;
use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};
use std::time::Instant;
use std::hint::black_box;

#[derive(Clone)]
struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    grows: i64,
}

impl World {
    fn new() -> World {
        let mut st = vec![0i64; 4096];
        let r = sol::m12_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            assert_eq!(sol::m12_init(&mut st), 0);
        }
        World { st, out: vec![0; 4096], grows: 0 }
    }
    fn grow(&mut self) -> bool {
        let n = self.out[0].max(0) as usize;
        let mut fresh = vec![0i64; n]; // N from Pruv (bud_store::need, N >= 2*len), L3 P13b
        if sol::m12_grow(&self.st, &mut fresh) != 0 {
            return false;
        }
        self.st = fresh;
        self.grows += 1;
        true
    }
    // runs f with retries on Grow (1) and out too small (3); each retry at least doubles a buffer,
    // so 64 rounds cover every size the ABI allows
    fn call(&mut self, f: &dyn Fn(&mut Vec<i64>, &mut Vec<i64>) -> i64) -> Result<&[i64], i64> {
        for _ in 0..64 {
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
fn perf_ref() -> Vec<i64> {
    let mut o = vec![9, 2];
    key(&mut o, "sessionId");
    sv(&mut o, "s1");
    key(&mut o, "budgetId");
    sv(&mut o, "b1");
    o
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, layout, n)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) { System.dealloc(p, layout); }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[derive(Clone)]
enum Lane {
    Pruv(World),
    Hand { store: hand::Store, order: Vec<Vec<i64>>, out: Vec<i64> },
    HandReplay { sessions: hand::life::Sessions, out: Vec<i64> },
}

impl Lane {
    fn new(hand: bool) -> Self {
        if hand {
            Self::Hand { store: hand::Store::new(), order: Vec::new(), out: Vec::with_capacity(4096) }
        } else {
            Self::Pruv(World::new())
        }
    }

    fn load(&mut self, key: &[i64], budget: &[i64]) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_load(st, key, budget, out)),
            Self::Hand { store, order, out } => {
                let fresh = store.load(key, budget);
                if fresh == 1 { order.push(key.to_vec()); }
                out.clear();
                out.extend([fresh]);
                Ok(out)
            }
            Self::HandReplay { sessions, out } => {
                out.clear();
                out.push(sessions.load(key, budget));
                Ok(out)
            }
        }
    }

    fn auto(&mut self, op: i64, rf: &[i64], sid: &[i64], caller: &[i64], packet: &[u8]) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_auto(st, op, rf, sid, caller, packet, out)),
            Self::Hand { store, out, .. } => hand_result(store.auto(op, rf, sid, caller, packet, out), out),
            Self::HandReplay { .. } => Err(2),
        }
    }

    fn set(&mut self, a: &[Vec<i64>], now: i64) -> Result<&[i64], i64> {
        let (card, lang, by) = (bytes_of(&a[3]), bytes_of(&a[4]), bytes_of(&a[5]));
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_set_card(st, &a[1], &a[2], &card, &lang, &by, now, out)),
            Self::Hand { store, out, .. } => {
                hand_result(store.set_status(&a[1], &a[2], &card, &lang, &by, now, out), out)
            }
            Self::HandReplay { .. } => Err(2),
        }
    }

    fn query(&mut self, a: &[Vec<i64>]) -> Result<&[i64], i64> {
        let op = a[0][0];
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_query(st, op, &a[1], &a[2], out)),
            Self::Hand { store, out, .. } => hand_result(store.query(op, &a[1], &a[2], out), out),
            Self::HandReplay { .. } => Err(2),
        }
    }

    fn dump(&mut self) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_dump(st, out)),
            Self::Hand { store, order, out } => {
                store.dump(order, out);
                Ok(&out[1..])
            }
            Self::HandReplay { sessions, out } => {
                sessions.dump(out);
                Ok(out)
            }
        }
    }
}

fn hand_result(s: i64, out: &[i64]) -> Result<&[i64], i64> {
    if s == 0 { Ok(&out[1..]) } else { Err(s) }
}

fn perf_budget(item: &str, card_text: &str) -> Vec<i64> {
    let mut v = vec![9, if item == "C" || item == "T" { 4 } else { 3 }];
    key(&mut v, "id");
    sv(&mut v, "b1");
    key(&mut v, "state");
    sv(&mut v, "open");
    // Keep the large card after autoContinue to exercise the clear tail move.
    if item == "C" || item == "T" {
        key(&mut v, "autoContinue");
        v.extend([9, 2]);
        key(&mut v, "sessionId");
        sv(&mut v, "s1");
        key(&mut v, "stopped");
        v.push(3);
    }
    key(&mut v, "card");
    v.extend([9, 3]);
    key(&mut v, "state");
    sv(&mut v, "working");
    key(&mut v, "by");
    v.extend([9, 1]);
    key(&mut v, "sessionId");
    sv(&mut v, "s1");
    key(&mut v, "deliverable");
    sv(&mut v, card_text);
    v
}

fn perf(is_hand: bool, item: &str, size: usize, reps: usize) -> String {
    let text = "x".repeat(size);
    let sid = pkey("s1");
    let rf = perf_ref();
    let mut record = Vec::new();
    bhead(&mut record, 9, 4);
    bkey(&mut record, "sessionId");
    bstr(&mut record, "s1");
    bkey(&mut record, "userTurnId");
    bstr(&mut record, &text);
    bkey(&mut record, "count");
    bnum(&mut record, 99.0);
    bkey(&mut record, "stopped");
    record.push(3);
    let (op, reference, ref_sid, packet) = match item {
        "R" => (2, rf.as_slice(), sid.as_slice(), record.as_slice()),
        "C" => (1, rf.as_slice(), sid.as_slice(), &[][..]),
        "T" | "S" => (3, &[1][..], &[][..], &[][..]),
        _ => panic!("unknown perf item"),
    };
    let initial = perf_budget(item, if item == "R" { "" } else { &text });
    // Preallocate room using a large initial card. Then load the actual input
    // and reuse the same store buffers between measured batches.
    let capacity = perf_budget(item, &"x".repeat(size + 128));
    let width = reps.min(64);
    let mut lanes: Vec<_> = (0..width).map(|_| {
        let mut lane = Lane::new(is_hand);
        lane.load(&sid, &capacity).unwrap();
        lane
    }).collect();
    let mut time = std::time::Duration::ZERO;
    let mut allocs = 0;
    let mut checksum = 0;
    let mut remaining = reps;
    while remaining > 0 {
        let batch = &mut lanes[..remaining.min(width)];
        for lane in &mut *batch { lane.load(&sid, &initial).unwrap(); }
        let before = ALLOCS.load(Ordering::Relaxed);
        let start = Instant::now();
        for lane in &mut *batch {
            let out = black_box(lane.auto(op, reference, ref_sid, &sid, packet).unwrap());
            checksum += out[3];
        }
        time += start.elapsed();
        allocs += ALLOCS.load(Ordering::Relaxed) - before;
        remaining -= batch.len();
    }
    format!("ns {:.1} {} alloc {}", time.as_nanos() as f64 / reps as f64, checksum, allocs)
}

fn dispatch(lane: &mut Lane, cmd: &str, rest: &str) -> String {
    let a = args(rest);
    match cmd {
        "load" => res(lane.load(&a[0], &a[1])),
        "auto" => res(lane.auto(a[0][0], &a[1], &a[2], &a[3], &bytes_of(&a[4]))),
        "set" => res(lane.set(&a, a[0][0])),
        "q" => res(lane.query(&a)),
        "dump" => res(lane.dump()),
        // test probe, Pruv lane only: Grow count so far and free arena words (min(len, word 11) - word 10)
        "room" => match lane {
            Lane::Pruv(w) => format!("ok {} {}", w.grows, (w.st.len() as i64).min(w.st[11]) - w.st[10]),
            Lane::Hand { .. } | Lane::HandReplay { .. } => String::from("ok 0 0"),
        },
        _ => panic!("unknown input command"),
    }
}

type Address = (usize, Vec<i64>); // instance + framed packed budget ID
type Deadline = (i64, i64); // due time + globally increasing token

#[derive(Clone)]
struct Timer {
    address: Address,
    sid: Vec<i64>,
    budget_first: bool,
    delay: i64,
}

#[derive(Clone, Default)]
struct Scheduler {
    now: i64,
    seq: i64,
    unref: bool,
    active: BTreeMap<Deadline, Timer>,
    named: HashMap<Address, Deadline>,
}

// Output trace packet: code, instance (-1 for an advance), length, payload.
fn trace(out: &mut Vec<i64>, code: i64, instance: i64, payload: &[i64]) {
    out.extend([code, instance, payload.len() as i64]);
    out.extend(payload);
}

fn effects(mut packet: &[i64]) -> impl Iterator<Item = (usize, i64, i64, i64, &[i64])> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        if packet.is_empty() { return None; }
        let (code, n, delay, effective) = (packet[0], packet[1], packet[2], packet[3]);
        let (payload, rest) = packet[4..].split_at(n as usize);
        let at = offset;
        offset += 4 + n as usize;
        packet = rest;
        Some((at, code, delay, effective, payload))
    })
}

fn traced(value: &[i64], log: &[i64]) -> String {
    format!("ok {} | {}", words(value), words(log))
}

fn session_words(id: &[i64], budget: &[i64], exists: bool) -> Vec<i64> {
    if !exists { return vec![1]; }
    let mut out = vec![9, if budget.is_empty() { 1 } else { 2 }];
    key(&mut out, "id");
    out.extend(id);
    if !budget.is_empty() {
        key(&mut out, "budget");
        out.extend(budget);
    }
    out
}

fn observe_session(lane: &Lane, sid: &[i64]) -> Vec<i64> {
    if sid.is_empty() { return Vec::new(); }
    match lane {
        Lane::Pruv(w) => sol::m12_session(&w.st, sid),
        Lane::HandReplay { sessions, .. } => sessions.observe_session(sid),
        _ => unreachable!(),
    }
}

// The ordered effect packet refers to session keys. A find reads the input
// snapshot unless the core recorded an intermediate timer transition there.
fn observe_effects(lane: &Lane, packet: &[i64], before: &[(&[i64], Vec<i64>)], log: &mut Vec<i64>) {
    let overrides = m12_facts::take_finds();
    let mut finds = effects(packet).filter(|&(_, code, _, _, _)| code == 1);
    let original = std::mem::take(log);
    let mut rest = original.as_slice();
    while !rest.is_empty() {
        let (code, instance, n) = (rest[0], rest[1], rest[2] as usize);
        let (data, next) = rest[3..].split_at(n);
        rest = next;
        if !matches!(code, 1 | 3 | 8 | 15 | 16) {
            trace(log, code, instance, data);
            continue;
        }
        let id = if code == 15 { &data[2..4 + data[3] as usize] } else { data };
        let find_offset = (code == 1).then(|| finds.next().unwrap().0);
        let snapshot = if id.first() != Some(&6) {
            vec![1]
        } else {
            let mut sid = vec![id[1]];
            sid.extend(pack_u16(&id[2..].iter().map(|&unit| unit as u16).collect::<Vec<_>>()));
            if let Some(offset) = find_offset {
                if let Some((_, budget)) = overrides.iter().find(|(at, _)| *at == offset) {
                    session_words(id, budget, true)
                } else {
                    let words = before.iter().find(|(key, _)| *key == sid).map_or(&[][..], |(_, words)| words);
                    session_words(id, words.get(1..).unwrap_or_default(), !words.is_empty())
                }
            } else {
                let words = observe_session(lane, &sid);
                session_words(id, words.get(1..).unwrap_or_default(), !words.is_empty())
            }
        };
        let mut payload = vec![8, 2];
        payload.extend(data);
        payload.extend(snapshot);
        trace(log, code, instance, &payload);
    }
}

impl Scheduler {
    fn apply(&mut self, i: usize, sid: &[i64], packet: &[i64], log: &mut Vec<i64>) {
        let mut reads = m12_facts::take_reads().into_iter().peekable();
        for (offset, code, delay, effective, data) in effects(packet) {
            if reads.peek().is_some_and(|read| read.offset == offset) {
                let read = reads.next().unwrap();
                assert_eq!(code, read.code);
                if code == 7 {
                    assert_eq!(&data[2..2 + read.reference.len()], read.reference);
                }
            } else {
                assert!(!matches!(code, 5..=7), "effect without a synchronous fact read");
            }
            match code {
                0 => {}
                12 => {
                    let (&budget_first, bid) = data.split_last().expect("captured reference");
                    self.seq += 1;
                    let deadline = (self.now + effective, self.seq);
                    let address = (i, bid.to_vec());
                    self.named.insert(address.clone(), deadline);
                    self.active.insert(deadline, Timer { address, sid: sid.to_vec(), budget_first: budget_first == 1, delay });
                    trace(log, 12, i as i64, &[self.seq, delay, deadline.0]);
                    if self.unref { trace(log, 17, i as i64, &[self.seq]); }
                }
                13 => {
                    let deadline = self.named.remove(&(i, data.to_vec())).expect("clear named timer");
                    self.active.remove(&deadline);
                    trace(log, 13, i as i64, &[deadline.1]);
                }
                14 => {
                    self.named.remove(&(i, data.to_vec())).expect("forget named timer");
                }
                _ => trace(log, code, i as i64, data),
            }
        }
        assert!(reads.next().is_none(), "unreported synchronous fact");
    }

    fn dump(&self) -> String {
        let mut out = vec![self.active.len() as i64];
        for (&(due, token), timer) in &self.active {
            out.extend([token, timer.delay, due, timer.address.0 as i64]);
        }
        out.push(self.named.len() as i64);
        let mut named: Vec<_> = self.named.iter().collect();
        named.sort_unstable_by_key(|&(_, &(_, token))| token);
        for ((instance, bid), &(_, token)) in named {
            out.extend([*instance as i64, bid.len() as i64]);
            out.extend(bid);
            out.push(token);
        }
        traced(&out, &[])
    }
}

// Pruv reserves space before changing either store and marks clean Grow replies.
fn lifecycle(
    lane: &mut Lane, clocks: &mut World,
    f: impl Fn(&mut [i64], &mut [i64]) -> Vec<i64>,
) -> Result<Vec<i64>, i64> {
    let Lane::Pruv(sessions) = lane else { return Err(2); };
    let checkpoint = m12_facts::checkpoint();
    for _ in 0..64 {
        let out = f(&mut sessions.st, &mut clocks.st);
        let (offset, code, status, session_grow, result) = effects(&out).last().expect("lifecycle result");
        assert_eq!(code, 0);
        if status == 0 {
            return Ok(out);
        }
        if status > 0 { return Err(status); }
        m12_facts::rewind(checkpoint);
        debug_assert!(offset == 0 && result.is_empty(), "Grow must precede store changes");
        let store = if session_grow == 1 { &mut *sessions } else { &mut *clocks };
        store.out[0] = -status;
        if !store.grow() { return Err(1); }
    }
    Err(9)
}

#[derive(Clone)]
enum Clocks {
    Pruv(World),
    Hand(hand::life::Clocks),
}

impl Clocks {
    fn observe(&mut self) -> Result<Vec<i64>, i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_observe(st, out)).map(<[_]>::to_vec),
            Self::Hand(w) => Ok(w.observe()),
        }
    }

    fn seed_tool(&mut self, turn: &[i64], activity: &[i64]) -> Result<i64, i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_seed_tool(st, turn, activity, out)).map(|v| v[0]),
            Self::Hand(w) => Ok(i64::from(w.seed_tool(turn, activity))),
        }
    }

    fn call(&mut self, lane: &mut Lane, op: i64, rf: &[i64], sid: &[i64], arg: &[i64], now: i64)
        -> Result<Vec<i64>, i64>
    {
        match (self, lane) {
            (Self::Pruv(clocks), lane) => lifecycle(lane, clocks, |st, cl| {
                sol::m12_life(st, cl, op, rf, sid, arg, now)
            }),
            (Self::Hand(clocks), Lane::HandReplay { sessions, .. }) => {
                clocks.call_words(sessions, op, rf, sid, arg, now)
            }
            _ => unreachable!(),
        }
    }

    fn fire(&mut self, lane: &mut Lane, sid: &[i64], bid: &[i64], budget_first: bool, now: i64)
        -> Result<Vec<i64>, i64>
    {
        match (self, lane) {
            (Self::Pruv(clocks), lane) => lifecycle(lane, clocks, |st, cl| {
                sol::m12_fire(st, cl, sid, bid, budget_first, now)
            }),
            (Self::Hand(clocks), Lane::HandReplay { sessions, .. }) => {
                clocks.fire_words(sessions, sid, bid, budget_first, now)
            }
            _ => unreachable!(),
        }
    }
}

fn replay_command(
    lane: &mut Lane, clocks: &mut Vec<Clocks>, scheduler: &mut Scheduler, cmd: &str, rest: &str,
) -> String {
    if !matches!(cmd, "world" | "life" | "seed-tool" | "advance" | "observe" | "timers" | "trace-auto" | "trace-set" | "trace-q") {
        return dispatch(lane, cmd, rest);
    }
    let a = args(rest);
    match cmd {
        "world" => {
            let [now, instances, unref, scripted_uuids] = a[0][..] else { panic!("world header"); };
            let n = a[2][0] as usize;
            *scheduler = Scheduler { now, unref: unref == 1, ..Scheduler::default() };
            *clocks = if matches!(lane, Lane::Hand { .. }) {
                *lane = Lane::HandReplay { sessions: hand::life::Sessions::default(), out: Vec::new() };
                (0..instances).map(|_| Clocks::Hand(hand::life::Clocks::default())).collect()
            } else {
                (0..instances).map(|_| Clocks::Pruv(World::new())).collect()
            };
            let uuids = (scripted_uuids == 1).then(|| a[3..3 + n].to_vec());
            m12_facts::reset(a[1].clone(), uuids, a[3 + n..].to_vec());
            m12_facts::observe();
            "ok".to_string()
        }
        "life" => {
            let (i, op) = (a[0][0] as usize, a[0][1]);
            let before = [(&a[2][..], observe_session(lane, &a[2]))];
            match clocks[i].call(lane, op, &a[1], &a[2], &a[3], scheduler.now) {
                Ok(packet) => {
                    let mut log = Vec::new();
                    scheduler.apply(i, &a[2], &packet, &mut log);
                    observe_effects(lane, &packet, &before, &mut log);
                    traced(effects(&packet).last().unwrap().4, &log)
                }
                Err(status) => {
                    m12_facts::take_reads();
                    m12_facts::take_finds();
                    format!("err {status}")
                }
            }
        }
        "seed-tool" => match clocks[a[0][0] as usize].seed_tool(&a[1], &a[2]) {
            Ok(fresh) => format!("ok {fresh}"),
            Err(status) => format!("err {status}"),
        },
        "advance" => {
            let ms = a[0][0];
            let target = scheduler.now + ms;
            let mut log = Vec::new();
            trace(&mut log, 18, -1, &[ms]);
            while let Some((&(due, token), _)) = scheduler.active.first_key_value() {
                if due > target { break; }
                let timer = scheduler.active.pop_first().unwrap().1;
                let (i, bid) = timer.address;
                scheduler.now = due;
                trace(&mut log, 19, i as i64, &[token]);
                let before = [(&timer.sid[..], observe_session(lane, &timer.sid))];
                match clocks[i].fire(lane, &timer.sid, &bid, timer.budget_first, due) {
                    Ok(packet) => {
                        let mut fired = Vec::new();
                        scheduler.apply(i, &timer.sid, &packet, &mut fired);
                        observe_effects(lane, &packet, &before, &mut fired);
                        log.extend(fired);
                    }
                    Err(status) => return format!("err {status}"),
                }
            }
            scheduler.now = target;
            traced(&[], &log)
        }
        "observe" => res(clocks[a[0][0] as usize].observe().as_deref().map_err(|&status| status)),
        "timers" => scheduler.dump(),
        "trace-auto" | "trace-set" | "trace-q" => {
            let i = a.last().unwrap()[0];
            let mut before = vec![(&a[2][..], observe_session(lane, &a[2]))];
            if cmd == "trace-auto" && a[0][0] == 3 {
                before.push((&a[3], observe_session(lane, &a[3])));
            }
            if let Lane::HandReplay { sessions, .. } = lane {
                let (kind, op) = match cmd {
                    "trace-auto" => (1, a[0][0]),
                    "trace-set" => (2, 0),
                    _ => (3, a[0][0]),
                };
                return match sessions.legacy(kind, op, &a, scheduler.now) {
                    Ok((value, packet)) => {
                        let mut log = Vec::new();
                        scheduler.apply(i as usize, &a[2], &packet, &mut log);
                        observe_effects(lane, &packet, &before, &mut log);
                        traced(&value, &log)
                    }
                    Err(status) => format!("err {status}"),
                };
            }
            let (kind, op, caller, response) = match cmd {
                "trace-auto" => (1, a[0][0], a[3].as_slice(),
                    lane.auto(a[0][0], &a[1], &a[2], &a[3], &bytes_of(&a[4]))),
                "trace-set" => (2, 0, &[][..], lane.set(&a, scheduler.now)),
                _ => (3, a[0][0], &[][..], lane.query(&a)),
            };
            match response {
                Ok(w) => {
                    let w = w.to_vec();
                    let Lane::Pruv(sessions) = lane else { return "err 2".into(); };
                    let packet = sol::m12_legacy_effects(&sessions.st, kind, op, &a[1], &a[2], caller, scheduler.now, &w);
                    let mut log = Vec::new();
                    for (_, code, _, _, data) in effects(&packet) { trace(&mut log, code, i, data); }
                    observe_effects(lane, &packet, &before, &mut log);
                    traced(&w, &log)
                }
                Err(status) => format!("err {status}"),
            }
        }
        _ => unreachable!(),
    }
}

fn main() {
    let is_hand = std::env::args().nth(1).as_deref() == Some("--hand");
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut lane = None;
    let mut clocks = Vec::new();
    let mut scheduler = Scheduler::default();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let (cmd, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let answer = if cmd == "init" {
            lane = Some(Lane::new(is_hand));
            String::from("ok")
        } else if let Some(item) = cmd.strip_prefix("perf").or_else(|| cmd.strip_prefix("hand")) {
            let a = args(rest);
            let (size, reps) = (a[0][0] as usize, a[0][1] as usize);
            let hand = cmd.starts_with("hand");
            if matches!(item, "F" | "D" | "O" | "B" | "A" | "H" | "U") {
                perf_life::measure(hand, item, size, reps)
            } else {
                perf(hand, item, size, reps)
            }
        } else {
            replay_command(lane.as_mut().expect("init required"), &mut clocks, &mut scheduler, cmd, rest)
        };
        writeln!(o, "{}", answer).unwrap();
    }
}

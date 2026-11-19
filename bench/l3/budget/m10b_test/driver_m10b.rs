// M10b driver (appended to a main file that already declares `mod sol;`, see run_m10b.py).
//
//   driver_m10b q        program on stdin, one case after another:
//                         case <id> | now <ms> | uuids <key>... | quiet <0/1>... | rdi <n> <key>... (one line per
//                         queued runningDelegationIds answer, in order) | unit <status and values> | put <words...> | <text bytes...> | adv <ms>
//                         | call <req> <body words> | end
//   driver_m10b perf <path> <D> <reps>   perf program, see run_m10b.py (`perf`)
//
// Boundary rule (M7 + M9): the host owns the state buffer and the scratch frames, supplies the inputs of a call (now)
// and the synchronous host facts' answers, and performs the effects in order; it never reads state words and decides
// nothing. Body = [code, what x10, flag, sessionId key, budgetId key, runKey key]; the host inserts now and a reserved
// zero word after the code.
// runningDelegationIds (P24): halt asks it synchronously after SETTLE (bud_facts::running_ids, answers queued by the
// `rdi` lines and taken one per query). The query is logged as `E rdi s:<sessionId units> b:<budgetId units>` at the effect
// position it was asked at (the effect count the host fact recorded), so the trace shows it after SETTLE's timerClear.
// Effects: [31] cancelRecovery, [32] stopMain,
// [33, i] stopDelegation of the i-th id of that query's answer, [34] stopAttachedTurns, [35] holdQueued, [36] save,
// [20] changed, [37] announce, [21] carryHeld, [22] uuid, [12, req, eff] timerSet, [13] timerClear, [14] timerForget.
// Timers (M9): timerSet registers token t<n> due at now + eff with the call's ref (the callback's captured ref) and
// makes it the budget's timer; timerClear cancels the budget's timer; timerForget drops the budget's handle. adv fires
// due timers in (due, token) order through FIRE (code 11) with now = due.
// isQuiet takes the next answer synchronously (default false).
// Grow -N: buffer of exactly N (Pruv sizes it, N >= 2*len), t_grow, retry.
// Keys on the wire (op keys, the runningDelegationIds answers `n, (len, words(len))*`) are packed u16 keys
// `n, words(n) packed words` (bud_keys layout), packed once by the encoder (run_m10b.py; key() below for perf). The
// scheduler keeps a timer's captured ref as those framed packed keys (passed back verbatim to FIRE); the log and the
// observation print ids in units (unpacked here).
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// sid / bid: framed packed keys `n, words(n)` of the captured ref
struct Timer {
    token: u64,
    due: i64,
    sid: Vec<i64>,
    bid: Vec<i64>,
}

// Host-side counts for M10 replay controls (printed to stderr at the end of q): grows (all, puts included), public
// t_call attempts, public calls, and grown calls: public calls that hit Grow at least once, then finished with status
// 0 and carried effects or consumed query answers (the calls a replay would duplicate).
static GROWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ATTEMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GROWN_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

use record_text::{observation, parse_put, perf_put, unit_record, Put};

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    fx: Vec<i64>,
    op: Vec<i64>,
    now: i64,
    rdi: Vec<Vec<i64>>,
    seq: u64,
    active: Vec<Timer>,
    by_bid: HashMap<Vec<i64>, u64>,
    log: Vec<String>,
    silent: bool,
    // perf: the answer the host gives (wire form `n, keys`) and the id starts of the current call's answer
    host_ids: Vec<i64>,
    pos: Vec<usize>,
    audit: bool,
    tx: Vec<u8>,
}

// Data words of a packed key of n units: ceil(n / 4).
fn words(n: i64) -> usize {
    (n.max(0) as usize + 3) / 4
}

// End of the framed packed key record `n, words(n)` at op[p].
fn rec_end(op: &[i64], p: usize) -> usize {
    p + 1 + words(op[p])
}

// The units of the framed packed key k = `n, words(n)...`, comma separated.
fn key_units_str(k: &[i64]) -> String {
    let n = k[0].max(0) as usize;
    let v: Vec<String> = (0..n).map(|j| ((((k[1 + j / 4] as u64) >> (48 - 16 * (j % 4))) & 0xffff) as i64).to_string()).collect();
    v.join(",")
}

impl World {
    fn new() -> World {
        // M10_INIT_LEN: start length of the state buffer (M10 replay controls runs the same program from a small and a
        // big start: Grow retries must not change any effect, answer or observation, L3 P13b)
        let init: usize = std::env::var("M10_INIT_LEN").ok().and_then(|s| s.parse().ok()).unwrap_or(256);
        let mut st = vec![0i64; init];
        let r = sol::t_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            if sol::t_init(&mut st) != 0 {
                eprintln!("t_init failed");
                std::process::exit(2);
            }
        }
        // The first nonempty put sizes the arena from its actual text payload.
        let mut tx = vec![0; 8];
        assert_eq!(sol::t_tinit(&mut tx), 0);
        World {
            st,
            tx,
            out: vec![0; 4096],
            bv: vec![0; 8],
            fx: vec![0; 4096],
            op: Vec::with_capacity(256),
            now: 1700000000000,
            rdi: Vec::new(),
            seq: 0,
            active: Vec::new(),
            by_bid: HashMap::new(),
            log: Vec::new(),
            silent: false,
            host_ids: vec![0],
            pos: Vec::new(),
            audit: false,
        }
    }

    fn grow(&mut self, n: usize) {
        let want = n; // N from Pruv (bud_store::need, N >= 2*len): the host makes no sizing choice (L3 P13b)
        GROWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut fresh = vec![0i64; want];
        if sol::t_grow(&self.st, &mut fresh) != 0 {
            eprintln!("t_grow failed");
            std::process::exit(2);
        }
        self.st = fresh;
    }

    fn put(&mut self, put: &Put) {
        for _ in 0..4 {
            let r = sol::t_put(&mut self.st, &mut self.tx, &put.words, &put.text, &mut self.out);
            if r == 0 {
                return;
            }
            if r == 1 {
                let n = self.out[0].max(0) as usize;
                self.grow(n);
                continue;
            }
            if r == 4 {
                let mut next = vec![0; self.out[0] as usize];
                assert_eq!(sol::t_tgrow(&mut self.st, &self.tx, &mut next), 0);
                self.tx = next;
                continue;
            }
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
        panic!("t_put exhausted Grow retries");
    }

    fn note(&mut self, s: String) {
        if !self.silent {
            self.log.push(s);
        }
    }

    // One public call (or FIRE); returns the status after the Grow retries, then performs the effects.
    fn call(&mut self, body: &[i64]) -> i64 {
        let mut r = 0;
        let mut op = std::mem::take(&mut self.op);
        op.clear();
        op.push(body[0]);
        op.push(self.now);
        op.push(0);
        op.extend_from_slice(&body[1..]);
        CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut grew = false;
        let before = bud_facts::calls();
        let rbefore = bud_facts::running_calls();
        for _ in 0..8 {
            self.fx[0] = 0;
            ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let snapshot = self.audit.then(|| (self.st.clone(), self.fx.clone(), self.out.clone(),
                bud_facts::calls() + bud_facts::running_calls()));
            r = sol::t_call(&mut self.st, &op, &mut self.bv, &mut self.fx, &mut self.out);
            if let Some((st, fx, out, before)) = snapshot {
                self.log.push(format!("A {} {} {} {} {} {} {:?}",
                    r, bud_facts::calls() + bud_facts::running_calls() - before, self.st == st, self.fx == fx,
                    self.out == out, self.out[0], &self.fx[1..1 + self.fx[0] as usize]));
            }
            if r < 0 {
                assert!(self.fx[0] == 0, "Grow with effects");
                assert!(bud_facts::take_unit_log().is_empty(), "Grow after unit");
                self.grow((-r) as usize);
                grew = true;
                continue;
            }
            break;
        }
        let unit_log = bud_facts::take_unit_log();
        let mut units = unit_log.into_iter().peekable();
        let after = bud_facts::calls();
        let rafter = bud_facts::running_calls();
        if !self.silent {
            for i in before..after {
                self.log.push(bud_facts::quiet_log(i));
            }
        }
        // runningDelegationIds queries of this call: (effect position, log line, wire answer `n, keys`)
        let mut rq: Vec<(usize, String, Vec<i64>)> = Vec::new();
        if !self.silent {
            for i in rbefore..rafter {
                let q = bud_facts::running_query(i);
                // the answer the query actually returned, in the wire form `n, keys` (an answer without one: no ids)
                let a = if q.answer.len() >= 2 { q.answer[1..].to_vec() } else { vec![0] };
                let line = format!("E rdi s:{} b:{}", key_units_str(&q.sid), key_units_str(&q.bid));
                rq.push((q.fxn.max(0) as usize, line, a));
            }
        }
        // the answer the stops name: the logged one in q mode, the host's own copy in a silent (perf) run
        let mut ids: Vec<i64> = if self.silent { std::mem::take(&mut self.host_ids) } else { vec![0] };
        let mut pos = std::mem::take(&mut self.pos);
        pos.clear();
        let mut nq = 0;
        if grew && r == 0 && self.fx[0] > 0 {
            GROWN_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // fx[1..fx[0]] is valid on every status >= 0: a rejection before any write leaves it empty, and halt's
        // terminal 2 / 6 after SETTLE (bud_halt.pruv header) carries the timerClear SETTLE applied, which the host
        // must apply too. A Grow never carries effects (every reserve precedes the first write).
        if r < 0 {
            self.op = op;
            if self.silent {
                self.host_ids = ids;
            }
            self.pos = pos;
            for (_, line, _) in rq.drain(..) {
                self.note(line);
            }
            self.note(format!("X {}", r));
            return r;
        }
        // the call's ref (framed packed keys): scheduler key and captured ref
        let se = rec_end(&op, 14);
        let be = rec_end(&op, se);
        // borrowed: the perf lane's effects (31..37) touch neither, so only a timer set copies them
        let sid = &op[14..se];
        let bid = &op[se..be];
        let used = self.fx[0].max(0) as usize;
        let mut i = 1;
        while i <= used + 1 {
            while units.peek().is_some_and(|(position, _)| *position < i as i64) {
                let (position, answer) = units.next().unwrap();
                assert_eq!(position, i as i64 - 1);
                let words: Vec<_> = answer.iter().map(i64::to_string).collect();
                self.note(format!("E unit {}", words.join(" ")));
            }
            while nq < rq.len() && rq[nq].0 + 1 <= i {
                let line = rq[nq].1.clone();
                ids = rq[nq].2.clone();
                pos.clear();
                // the host performs fx[1..fxn] before it answers (timerClear included): a timer of this budget still
                // armed when the query is answered means the query came before SETTLE's clear. The effects are replayed
                // after the call up to the query's fxn, so this checks the host-visible order (fx[1..fxn) vs the query),
                // not the order of Pruv's store writes inside the call, which the host cannot observe
                if self.by_bid.contains_key(bid) {
                    self.note("X armed".to_string());
                }
                self.note(line);
                nq += 1;
            }
            if i > used { break; }
            let e = self.fx[i];
            match e {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    self.seq += 1;
                    self.active.push(Timer { token: self.seq, due: self.now + eff, sid: sid.to_vec(), bid: bid.to_vec() });
                    self.by_bid.insert(bid.to_vec(), self.seq);
                    if !self.silent { self.note(format!("E set t{} {} {}", self.seq, req, eff)); }
                    i += 3;
                    continue;
                }
                13 => {
                    let s = match self.by_bid.remove(bid) {
                        Some(tok) => {
                            self.active.retain(|t| t.token != tok);
                            if self.silent { String::new() } else { format!("E clear t{}", tok) }
                        }
                        None => "E clear -".to_string(),
                    };
                    self.note(s);
                }
                14 => {
                    self.by_bid.remove(bid);
                }
                33 => {
                    // the k-th id of the answer (`n, (len, words(len))*`): the id starts are found in one pass on the
                    // first stop (O(id words) per call); q mode prints it in units, a silent (perf) run names it
                    // (black_box) as the hand baseline does
                    if pos.is_empty() {
                        let n = ids[0].max(0) as usize;
                        let mut p = 1;
                        pos.push(p);
                        for _ in 0..n {
                            p = rec_end(&ids, p);
                            pos.push(p);
                        }
                    }
                    let k = self.fx[i + 1].max(0) as usize;
                    let id = &ids[pos[k]..pos[k + 1]];
                    if self.silent {
                        black_box(id);
                    } else {
                        let s = format!("E stopDel {}", key_units_str(id));
                        self.note(s);
                    }
                    i += 2;
                    continue;
                }
                _ if self.silent => {}
                _ => {
                    let s = match e {
                        20 => "E changed".to_string(),
                        21 => "E carry".to_string(),
                        22 => "E uuid".to_string(),
                        31 => "E cancel".to_string(),
                        32 => "E stopMain".to_string(),
                        34 => "E stopTurns".to_string(),
                        35 => "E hold".to_string(),
                        36 => "E save".to_string(),
                        37 => "E announce".to_string(),
                        x => format!("X fx{}", x),
                    };
                    self.note(s);
                }
            }
            i += 1;
        }
        assert!(units.next().is_none(), "unit position outside effect stream");
        while nq < rq.len() {
            let line = rq[nq].1.clone();
            if self.by_bid.contains_key(bid) {
                self.note("X armed".to_string());
            }
            self.note(line);
            nq += 1;
        }
        self.op = op;
        if self.silent {
            self.host_ids = ids;
        }
        self.pos = pos;
        if r != 0 {
            self.note(format!("X {}", r));
            return r;
        }
        if !self.silent {
            let w: Vec<String> = match self.out[0] {
                4 => {
                    // budget view: the record is st[off..off+n)
                    let n = self.out[1].max(0) as usize;
                    let off = self.out[2].max(0) as usize;
                    let units = unit_record(&self.tx, &self.st[off..off + n]);
                    let mut v = vec!["4".to_string(), units.len().to_string()];
                    v.extend(units.iter().map(|x| x.to_string()));
                    v
                }
                5 => self.out[..3].iter().map(|x| x.to_string()).collect(),
                _ => vec![self.out[0].to_string()],
            };
            self.log.push(format!("V {}", w.join(" ")));
        }
        0
    }

    // The timer callback `() => this.fire(ref)` with its captured ref (FIRE, code 11).
    fn fire(&mut self, idx: usize) -> i64 {
        let t = self.active.remove(idx);
        self.note(format!("F t{}", t.token));
        let mut body = vec![11i64];
        body.extend(std::iter::repeat(0).take(11));
        body.extend_from_slice(&t.sid);
        body.extend_from_slice(&t.bid);
        body.push(0);
        let silent = self.silent;
        // the fire's own result word is not a request value
        let r = self.call(&body);
        if !silent {
            if let Some(l) = self.log.last() {
                if l.starts_with("V ") {
                    self.log.pop();
                }
            }
        }
        r
    }

    fn advance(&mut self, ms: i64) {
        let target = self.now + ms;
        loop {
            let mut best: Option<usize> = None;
            for (i, t) in self.active.iter().enumerate() {
                if t.due <= target {
                    match best {
                        None => best = Some(i),
                        Some(b) => {
                            let bt = &self.active[b];
                            if t.due < bt.due || (t.due == bt.due && t.token < bt.token) {
                                best = Some(i)
                            }
                        }
                    }
                }
            }
            match best {
                None => break,
                Some(i) => {
                    self.now = self.active[i].due;
                    self.fire(i);
                }
            }
        }
        self.now = target;
    }

    fn observe(&mut self) -> String {
        let need = std::cmp::max(4096, 2 * self.st.len());
        if self.out.len() < need {
            self.out.resize(need, 0);
        }
        let r = sol::t_observe(&self.st, &mut self.out);
        if r != 0 {
            return format!("O X {}", r);
        }
        let n = self.out[0].max(0) as usize;
        let mut s = observation(&self.tx, &self.out[1..1 + n]);
        // active timers: t<token>:<due>:<budgetId units>:<1 when it is the budget's timer>
        s.push_str(" | T");
        for t in &self.active {
            let own = self.by_bid.get(&t.bid) == Some(&t.token);
            s.push_str(&format!(" t{}:{}:{}:{}", t.token, t.due, key_units_str(&t.bid), own as i32));
        }
        s
    }
}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

fn run_q() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut w = World::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                w = World::new();
                bud_facts::record_unit();
                bud_facts::set_unit(&[]);
                bud_facts::record_queries();
                bud_facts::set_answers(&[], 0);
                bud_facts::set_calls(0);
                bud_facts::record_running();
                bud_facts::set_running(Vec::new());
                bud_facts::set_running_calls(0);
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "now" => w.now = p[1].parse().unwrap(),
            "audit" => {
                w.audit = true;
                w.out.fill(-99);
                w.fx[1..].fill(-99);
            }
            // Test-only: consume the remaining arena with unreferenced padding.
            "fill_arena" => w.st[10] = w.st[11],
            "quiet" => bud_facts::set_answers(&nums(&p[1..]), 0),
            "unit" => bud_facts::set_unit(&nums(&p[1..])),
            "rdi" => {
                let a = nums(&p[1..]);
                let mut v = vec![0i64];
                v.extend_from_slice(&a);
                bud_facts::push_running(v);
                w.rdi.push(a);
            }
            "put" => w.put(&parse_put(&p[1..])),
            "adv" => {
                w.log.push("@ -".to_string());
                w.advance(p[1].parse::<i64>().unwrap());
            }
            "call" => {
                w.log.push(format!("@ {}", p[1]));
                w.call(&nums(&p[2..]));
            }
            "end" => {
                for l in w.log.drain(..) {
                    writeln!(o, "{}", l).unwrap();
                }
                let ob = w.observe();
                writeln!(o, "{}", ob).unwrap();
                writeln!(o, "end").unwrap();
            }
            other => {
                eprintln!("bad line {}", other);
                std::process::exit(2);
            }
        }
    }
    drop(o);
    eprintln!(
        "m10 host grows {} attempts {} calls {} grown_calls {}",
        GROWS.load(std::sync::atomic::Ordering::Relaxed),
        ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed),
        CALLS.load(std::sync::atomic::Ordering::Relaxed),
        GROWN_CALLS.load(std::sync::atomic::Ordering::Relaxed)
    );
}

// The performance workload keeps D clock runs. Each sample times one raise and its host effects.
// Wire key: `n, packed words(n)` (bud_keys layout: four big-endian lanes per word, tail lanes zero).
fn key(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    for c in u.chunks(4) {
        let mut x: u64 = 0;
        for l in 0..4 {
            x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
        }
        v.push(x as i64);
    }
    v
}

pub const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

mod benchmark {
    include!("hand_m10b.rs");
}

use benchmark::{allocations, Setup, SID, BID};

fn body(code: i64, flag: i64, run: &str) -> Vec<i64> {
    let mut op = vec![0; 12];
    op[0] = code;
    op[11] = flag;
    op.extend(key(SID));
    op.extend(key(BID));
    op.extend(key(run));
    op
}

// The measured raise with the hand lane's host work: one Grow-retrying public call and a single timer slot.
// q mode and the setup calls keep the M10 scheduler (World::call), whose logging, audit and per-budget timer index
// the hand lane has no counterpart for.
fn perf_raise(world: &mut World, op: &mut Vec<i64>, timer: &mut Option<i64>) -> i64 {
    // The frame is built once outside the sample (as the hand lane's prepared keys); only the time word changes.
    op[1] = world.now;
    let mut r;
    loop {
        world.fx[0] = 0;
        r = sol::t_call(&mut world.st, op, &mut world.bv, &mut world.fx, &mut world.out);
        if r >= 0 { break; }
        world.grow((-r) as usize);
    }
    let used = world.fx[0].max(0) as usize;
    let mut i = 1;
    while i <= used {
        match world.fx[i] {
            12 => {
                *timer = Some(world.now + world.fx[i + 2]);
                i += 3;
                continue;
            }
            13 => *timer = None,
            e => { black_box(e); }
        }
        i += 1;
    }
    r
}

fn run_perf(args: &[String]) {
    let setup = Setup::parse(args);
    let mut world = World::new();
    world.silent = true;
    let put = perf_put(key(SID), key(BID), 0, PERF_REST);
    world.put(&put);
    for i in 1..setup.d {
        let run = format!("d{i:04}");
        assert_eq!(world.call(&body(8, 0, &run)), 0);
        assert_eq!(world.call(&body(9, 1, &run)), 0);
    }
    assert_eq!(world.call(&body(8, 0, "d0000")), 0);
    let pause = body(9, 1, "d0000");
    let resume = body(9, 0, "d0000");
    let raise = body(13, setup.size, "");
    // Match M10's staged running-delegation answer. Ordinary raises do not ask for it.
    let mut ids = vec![0, setup.d as i64];
    for i in 0..setup.d {
        ids.extend(key(&format!("d{i:04}")));
    }
    world.host_ids = ids[1..].to_vec();
    bud_facts::set_running(vec![ids]);
    bud_facts::set_unit(&setup.unit);
    let mut op = Vec::with_capacity(raise.len() + 2);
    op.push(raise[0]);
    op.push(0);
    op.push(0);
    op.extend_from_slice(&raise[1..]);
    let mut timer = None;
    benchmark::measure(setup.reps, || {
        // Put back the original budget and re-enter its active interval outside the measurement.
        assert_eq!(world.call(&pause), 0);
        world.put(&put);
        assert_eq!(world.call(&resume), 0);
        world.now += 1;
        // The resume armed the budget's timer; the slot holds the same expiry as the hand lane's (120-minute limit).
        timer = Some(world.now - 1 + 120 * 60_000);

        let before = allocations();
        let start = Instant::now();
        let status = black_box(perf_raise(&mut world, black_box(&mut op), &mut timer));
        black_box(timer);
        let elapsed = start.elapsed();
        let allocs = allocations() - before;
        // The raise's timer effects went to the single slot. Drop the scheduler's copy of the resume timer so the
        // next pause does not see a stale entry and timers do not accumulate across samples.
        world.by_bid.clear();
        world.active.clear();
        assert_eq!(status, 0);
        assert_eq!(world.out[0], 2);
        assert_eq!(world.fx[0], setup.effect_words);
        (elapsed, allocs)
    });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "perf") {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

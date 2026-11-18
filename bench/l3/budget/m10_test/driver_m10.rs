// M10 driver (appended to a main file that already declares `mod sol;`, see run_m10.py).
//
//   driver_m10 q        program on stdin, one case after another:
//                         case <id> | now <ms> | uuids <key>... | quiet <0/1>... | rdi <n> <key>... (one line per
//                         queued runningDelegationIds answer, in order) | put <words...> | adv <ms>
//                         | call <req> <body words> | end
//   driver_m10 perf <D> <reps>   perf program, see run_m10.py (cmd_perf)
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
// `n, words(n) packed words` (bud_keys layout), packed once by the encoder (run_m10.py; key() below for perf). The
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

// Host-side counts for run_m10.py replay (printed to stderr at the end of q): grows (all, puts included), public
// t_call attempts, public calls, and grown calls: public calls that hit Grow at least once, then finished with status
// 0 and carried effects or consumed query answers (the calls a replay would duplicate).
static GROWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ATTEMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GROWN_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    audit: bool,
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
        // M10_INIT_LEN: start length of the state buffer (run_m10.py replay runs the same program from a small and a
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
        World {
            st,
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

    fn put(&mut self, op: &[i64]) {
        for _ in 0..4 {
            let r = sol::t_put(&mut self.st, op, &mut self.out);
            if r == 0 {
                return;
            }
            if r == 1 {
                let n = self.out[0].max(0) as usize;
                self.grow(n);
                continue;
            }
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
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
                self.grow((-r) as usize);
                grew = true;
                continue;
            }
            break;
        }
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
                let a = self.rdi.get(i).cloned().unwrap_or_else(|| vec![0]);
                let line = format!("E rdi s:{} b:{}", key_units_str(&q.sid), key_units_str(&q.bid));
                rq.push((q.fxn.max(0) as usize, line, a));
            }
        }
        let mut ids: Vec<i64> = vec![0];
        let mut nq = 0;
        if grew && r == 0 && self.fx[0] > 0 {
            GROWN_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // fx[1..fx[0]] is valid on every status >= 0: a rejection before any write leaves it empty, and halt's
        // terminal 2 / 6 after SETTLE (bud_halt.pruv header) carries the timerClear SETTLE applied, which the host
        // must apply too. A Grow never carries effects (every reserve precedes the first write).
        if r < 0 {
            self.op = op;
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
        while i <= used {
            while nq < rq.len() && rq[nq].0 + 1 <= i {
                let line = rq[nq].1.clone();
                ids = rq[nq].2.clone();
                self.note(line);
                nq += 1;
            }
            let e = self.fx[i];
            match e {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    self.seq += 1;
                    self.active.push(Timer { token: self.seq, due: self.now + eff, sid: sid.to_vec(), bid: bid.to_vec() });
                    self.by_bid.insert(bid.to_vec(), self.seq);
                    let s = format!("E set t{} {} {}", self.seq, req, eff);
                    self.note(s);
                    i += 3;
                    continue;
                }
                13 => {
                    let s = match self.by_bid.remove(bid) {
                        Some(tok) => {
                            self.active.retain(|t| t.token != tok);
                            format!("E clear t{}", tok)
                        }
                        None => "E clear -".to_string(),
                    };
                    self.note(s);
                }
                14 => {
                    self.by_bid.remove(bid);
                }
                33 => {
                    // the k-th id of the last answer taken (`n, (len, words(len))*`), printed in units; the log is
                    // the only consumer, so a silent (perf) run skips the walk and the formatting
                    if !self.silent {
                        let k = self.fx[i + 1].max(0) as usize;
                        let mut p = 1;
                        for _ in 0..k {
                            p = rec_end(&ids, p);
                        }
                        let s = format!("E stopDel {}", key_units_str(&ids[p..rec_end(&ids, p)]));
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
        while nq < rq.len() {
            let line = rq[nq].1.clone();
            self.note(line);
            nq += 1;
        }
        self.op = op;
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
                    let mut v = vec!["4".to_string(), n.to_string()];
                    v.extend(self.st[off..off + n].iter().map(|x| x.to_string()));
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
        let mut s = String::from("O");
        for x in &self.out[1..1 + n] {
            s.push(' ');
            s.push_str(&x.to_string());
        }
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
            "rdi" => {
                let a = nums(&p[1..]);
                let mut v = vec![0i64];
                v.extend_from_slice(&a);
                bud_facts::push_running(v);
                w.rdi.push(a);
            }
            "put" => w.put(&nums(&p[1..])),
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

// perf <D> <reps>: one session with an open budget; halt(ref, 'delegations') with D running delegation ids, then the
// record is put back open (both inside the timed loop; `putback` times the put alone). prints `item D ns_per_call`.
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

// A unit string `len, units...`.
fn ustr(s: &str) -> Vec<i64> {
    let mut v = vec![s.encode_utf16().count() as i64];
    v.extend(s.encode_utf16().map(|u| u as i64));
    v
}

// Record words (bud_lifecycle.pruv header): strings 0 (id) and 4 (follows, empty) packed, the others units.
fn rec(id: &str, rest: &str) -> Vec<i64> {
    let mut h = vec![0i64; 24];
    h[3] = 120;
    h[4] = 24;
    h[5] = 3;
    h[6] = 2;
    h[7] = 1;
    h[20] = 1;
    let mut v = h;
    v.extend(key(id));
    for t in ["2023-11-14T22:13:20.000Z", "", ""] {
        v.extend(ustr(t));
    }
    v.extend(key(""));
    v.extend(ustr(rest));
    v
}

pub const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn run_perf(args: &[String]) {
    bud_facts::set_answers(&[], 1);
    bud_facts::set_calls(0);
    let d: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let bid = "99b568f7-0000-4000-8000-000000000001";
    let mut put = key("session-perf");
    let r = rec(bid, PERF_REST);
    put.push(r.len() as i64);
    put.extend(r);
    let mut ids = vec![0i64, d as i64];
    for i in 0..d {
        ids.extend(key(&format!("d{:04}", i)));
    }
    let mut body = vec![12i64];
    body.extend(std::iter::repeat(0).take(10));
    body.push(1);
    body.extend(key("session-perf"));
    body.extend(key(bid));
    body.push(0);
    let mut w = World::new();
    w.silent = true;
    w.put(&put);
    let mut go = |w: &mut World| {
        // the host builds its answer for the one query of the call (the hand baseline does the same)
        bud_facts::set_running(vec![ids.clone()]);
        bud_facts::set_running_calls(0);
        black_box(w.call(black_box(&body)));
        w.put(black_box(&put));
    };
    for _ in 0..1000 {
        go(&mut w);
    }
    // the halt ran: one query, D stops (2D + 7 effect words, no timerClear: the perf budget has no clock)
    bud_facts::set_running(vec![ids.clone()]);
    bud_facts::set_running_calls(0);
    assert!(w.call(&body) == 0 && w.fx[0] == 2 * d as i64 + 7 && bud_facts::running_calls() == 1);
    w.put(&put);
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w);
    }
    println!("halt {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
    let t = Instant::now();
    for _ in 0..reps {
        w.put(black_box(&put));
    }
    println!("putback {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

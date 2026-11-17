// M10 driver (appended to a main file that already declares `mod sol;`, see run_m10.py).
//
//   driver_m10 q        program on stdin, one case after another:
//                         case <id> | now <ms> | uuids <key>... | quiet <0/1>... | rdi <n> <key>... (one line per
//                         queued runningDelegationIds answer, in order) | put <words...> | adv <ms>
//                         | call <req> <body words> | end
//   driver_m10 perf <D> <reps>   perf program, see run_m10.py (cmd_perf)
//
// Boundary rule (M7 + M9): the host owns the state buffer and the scratch frames, supplies the inputs of a call (now,
// the isQuiet answer, the next runningDelegationIds answer) and performs the effects in order; it never reads state
// words and decides nothing. Body = [code, what x10, flag, sessionId key, budgetId key, runKey key]; the host inserts
// now and quiet after the code and appends the head of its runningDelegationIds queue (`n, keys`, n = 0 when empty).
// Effects: [30] runningDelegationIds consumed (the host pops its queue), [31] cancelRecovery, [32] stopMain,
// [33, i] stopDelegation of the i-th id of the answer passed in, [34] stopAttachedTurns, [35] holdQueued, [36] save,
// [20] changed, [37] announce, [21] carryHeld, [22] uuid, [12, req, eff] timerSet, [13] timerClear, [14] timerForget.
// Timers (M9): timerSet registers token t<n> due at now + eff with the call's ref (the callback's captured ref) and
// makes it the budget's timer; timerClear cancels the budget's timer; timerForget drops the budget's handle. adv fires
// due timers in (due, token) order through FIRE (code 11) with now = due. isQuiet: the next answer (default false) is prefetched in op[2]
// and consumed when the effects hold [23].
// Grow -N: buffer of max(N, 2*len), t_grow, retry.
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

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    fx: Vec<i64>,
    op: Vec<i64>,
    now: i64,
    quiet: Vec<i64>,
    next_quiet: usize,
    rdi: Vec<Vec<i64>>,
    next_rdi: usize,
    seq: u64,
    active: Vec<Timer>,
    by_bid: HashMap<Vec<i64>, u64>,
    log: Vec<String>,
    silent: bool,
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
        let mut st = vec![0i64; 256];
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
            quiet: Vec::new(),
            next_quiet: 0,
            rdi: Vec::new(),
            next_rdi: 0,
            seq: 0,
            active: Vec::new(),
            by_bid: HashMap::new(),
            log: Vec::new(),
            silent: false,
        }
    }

    fn grow(&mut self, n: usize) {
        let want = std::cmp::max(n, 2 * self.st.len());
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
        let quiet = self.quiet.get(self.next_quiet).copied().unwrap_or(0);
        let mut r = 0;
        let empty = vec![0i64];
        let ids: Vec<i64> = self.rdi.get(self.next_rdi).cloned().unwrap_or(empty);
        let mut op = std::mem::take(&mut self.op);
        op.clear();
        op.push(body[0]);
        op.push(self.now);
        op.push(quiet);
        op.extend_from_slice(&body[1..]);
        op.extend_from_slice(&ids);
        for _ in 0..8 {
            self.fx[0] = 0;
            r = sol::t_call(&mut self.st, &op, &mut self.bv, &mut self.fx, &mut self.out);
            if r < 0 {
                self.grow((-r) as usize);
                continue;
            }
            break;
        }
        if r != 0 {
            // a finished call that consulted isQuiet ([23]) consumed the answer even when it failed afterwards
            let used = self.fx[0].max(0) as usize;
            if self.fx[1..=used].contains(&23) {
                self.next_quiet += 1;
                self.note(format!("E quiet {}", quiet));
            }
            self.op = op;
            self.note(format!("X {}", r));
            return r;
        }
        // the call's ref (framed packed keys): scheduler key and captured ref
        let se = rec_end(&op, 14);
        let be = rec_end(&op, se);
        let sid = op[14..se].to_vec();
        let bid = op[se..be].to_vec();
        let used = self.fx[0].max(0) as usize;
        let mut i = 1;
        while i <= used {
            let e = self.fx[i];
            match e {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    self.seq += 1;
                    self.active.push(Timer { token: self.seq, due: self.now + eff, sid: sid.clone(), bid: bid.clone() });
                    self.by_bid.insert(bid.clone(), self.seq);
                    let s = format!("E set t{} {} {}", self.seq, req, eff);
                    self.note(s);
                    i += 3;
                    continue;
                }
                13 => {
                    let s = match self.by_bid.remove(&bid) {
                        Some(tok) => {
                            self.active.retain(|t| t.token != tok);
                            format!("E clear t{}", tok)
                        }
                        None => "E clear -".to_string(),
                    };
                    self.note(s);
                }
                14 => {
                    self.by_bid.remove(&bid);
                }
                23 => {
                    self.next_quiet += 1;
                    self.note(format!("E quiet {}", quiet));
                }
                30 => {
                    self.next_rdi += 1;
                    self.note("E rdi".to_string());
                }
                33 => {
                    // the k-th id of the answer passed in (`n, (len, words(len))*`), printed in units; the log is
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
        self.op = op;
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
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "now" => w.now = p[1].parse().unwrap(),
            "quiet" => w.quiet = nums(&p[1..]),
            "rdi" => w.rdi.push(nums(&p[1..])),
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
    let d: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let bid = "99b568f7-0000-4000-8000-000000000001";
    let mut put = key("session-perf");
    let r = rec(bid, PERF_REST);
    put.push(r.len() as i64);
    put.extend(r);
    let mut ids = vec![d as i64];
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
        w.rdi.clear();
        w.rdi.push(ids.clone());
        w.next_rdi = 0;
        black_box(w.call(black_box(&body)));
        w.put(black_box(&put));
    };
    for _ in 0..1000 {
        go(&mut w);
    }
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

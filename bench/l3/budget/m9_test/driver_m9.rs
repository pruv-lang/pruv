// M9 driver (appended to a main file that already declares `mod sol;`, see run_m9.py).
//
//   driver_m9 q        program on stdin, one case after another:
//                        case <id> | put <words...> | now <ms> | step <i> | call <op words...> | adv <ms> | settime <ms>
//                        | deliver <token> <captured 0/1> | end
//   driver_m9 perf     perf program, see run_m9.py (cmd_perf)
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words. It is the
// timer scheduler only, like setTimeout / clearTimeout: a timerSet effect registers token t<n> (n counts
// registrations) due at now + effective delay, with the ref (sessionId, budgetId) of the call that armed it (the
// callback's captured ref), and makes it the budget's timer; timerClear cancels the budget's timer; timerForget makes
// the budget own no timer without cancelling it (FIRE dropped its handle). An elapsed timer leaves the schedule and
// calls FIRE (op code 4) with its captured ref and now = due, in (due, registration) order. deliver runs an active
// token the same way; an inactive one (cancelled or already run) runs its captured callback only with captured = 1,
// otherwise the script is invalid (X deliver). Grow: status -N -> buffer of max(N, 2*len), t_grow, retry.
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// Scheduler keys hold the op's key units in the byte form hand_m9.rs keys its Strings by (each unit as its UTF-8
// style 1-3 byte sequence, an injective encoding; ASCII keys are one byte per unit, the same bytes as the String).
// The host never interprets them. A key is encoded into a reused scratch buffer and lookups borrow it; a key is
// copied only when a timer captures its ref or a budget gets its first map slot, the same allocations as hand_m9.rs
// (which clones the ref's two Strings and inserts the budget key the same way).
struct Timer {
    token: u64,
    due: i64,
    sid: Vec<u8>,
    bid: Vec<u8>,
}

fn enc(units: &[i64], out: &mut Vec<u8>) {
    out.clear();
    if units.iter().all(|&x| (x as u64) < 0x80) {
        out.extend(units.iter().map(|&x| x as u8));
        return;
    }
    for &x in units {
        let u = (x as u32) & 0xffff;
        if u < 0x80 {
            out.push(u as u8);
        } else if u < 0x800 {
            out.push(0xc0 | (u >> 6) as u8);
            out.push(0x80 | (u & 0x3f) as u8);
        } else {
            out.push(0xe0 | (u >> 12) as u8);
            out.push(0x80 | ((u >> 6) & 0x3f) as u8);
            out.push(0x80 | (u & 0x3f) as u8);
        }
    }
}

fn dec(b: &[u8], out: &mut Vec<i64>) {
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as i64;
        if c < 0x80 {
            out.push(c);
            i += 1;
        } else if c < 0xe0 {
            out.push(((c & 0x1f) << 6) | (b[i + 1] as i64 & 0x3f));
            i += 2;
        } else {
            out.push(((c & 0x0f) << 12) | ((b[i + 1] as i64 & 0x3f) << 6) | (b[i + 2] as i64 & 0x3f));
            i += 3;
        }
    }
}

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    tmp: Vec<i64>,
    fx: Vec<i64>,
    now: i64,
    seq: u64,
    active: Vec<Timer>,
    by_bid: HashMap<Vec<u8>, u64>,
    known: HashMap<u64, (Vec<u8>, Vec<u8>)>,
    cb: Vec<i64>,
    kb: Vec<u8>,
    ks: Vec<u8>,
    log: Vec<String>,
    quiet: bool,
}

fn key_at(op: &[i64], p: usize) -> (usize, usize) {
    let n = op[p] as usize;
    (p + 1, p + 1 + n)
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
            tmp: vec![0; 256],
            fx: vec![0; 16],
            now: 1700000000000,
            seq: 0,
            active: Vec::new(),
            by_bid: HashMap::new(),
            known: HashMap::new(),
            cb: Vec::with_capacity(64),
            kb: Vec::with_capacity(64),
            ks: Vec::with_capacity(64),
            log: Vec::new(),
            quiet: false,
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

    // One Pruv call; returns the status after Grow retries, then runs the effects.
    fn call(&mut self, op: &[i64]) -> i64 {
        let mut r = 0;
        for _ in 0..4 {
            self.fx[0] = 0;
            r = sol::t_call(&mut self.st, op, self.now, &mut self.bv, &mut self.tmp, &mut self.fx);
            if r < 0 {
                self.grow((-r) as usize);
                continue;
            }
            break;
        }
        if r < 0 || r == 2 || r == 3 {
            if !self.quiet {
                self.log.push(format!("X {}", r));
            }
            return r;
        }
        let used = self.fx[0].max(0) as usize;
        if used == 0 {
            return self.halt_log(op, r);
        }
        // ref of the call (scheduler key and captured ref), encoded into the scratch keys
        let (ss, se) = key_at(op, 1);
        let (bs, be) = key_at(op, 2 + op[1] as usize);
        let mut kb = std::mem::take(&mut self.kb);
        let mut ks = std::mem::take(&mut self.ks);
        enc(&op[bs..be], &mut kb);
        let bid = &kb[..];
        let mut i = 1;
        while i <= used {
            match self.fx[i] {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    // like `clock.timer = setTimeout(..)`: a previous handle is overwritten, not cancelled
                    self.seq += 1;
                    enc(&op[ss..se], &mut ks);
                    let sid = &ks[..];
                    let t = Timer { token: self.seq, due: self.now + eff, sid: sid.to_vec(), bid: bid.to_vec() };
                    match self.by_bid.get_mut(bid) {
                        Some(v) => *v = self.seq,
                        None => {
                            self.by_bid.insert(bid.to_vec(), self.seq);
                        }
                    }
                    if !self.quiet {
                        self.known.insert(self.seq, (sid.to_vec(), bid.to_vec()));
                        self.log.push(format!("S t{} {}", self.seq, req));
                    }
                    self.active.push(t);
                    i += 3;
                }
                13 => {
                    self.clear_bid(bid, true);
                    i += 1;
                }
                14 => {
                    self.by_bid.remove(bid);
                    i += 1;
                }
                _ => {
                    self.log.push(format!("X fx{}", self.fx[i]));
                    break;
                }
            }
        }
        self.kb = kb;
        self.ks = ks;
        self.halt_log(op, r)
    }

    fn halt_log(&mut self, op: &[i64], r: i64) -> i64 {
        if r == 4 && !self.quiet {
            // HALT_TIME: when and for which session (the caller's halt(ref, 'time') is M10)
            let (ss, se) = key_at(op, 1);
            let u: Vec<String> = op[ss..se].iter().map(|x| x.to_string()).collect();
            self.log.push(format!("H {} {}", self.now, u.join(",")));
        }
        r
    }

    fn clear_bid(&mut self, bid: &[u8], logit: bool) {
        if let Some(tok) = self.by_bid.remove(bid) {
            self.active.retain(|t| t.token != tok);
            if logit && !self.quiet {
                self.log.push(format!("C t{}", tok));
            }
        } else if logit && !self.quiet {
            self.log.push("C -".to_string());
        }
    }

    fn fire_token(&mut self, idx: usize) -> i64 {
        let t = self.active.remove(idx);
        self.run_callback(t.token, &t.sid, &t.bid)
    }

    // The timer callback `() => this.fire(ref)` with its captured ref.
    // The op is built in a reused scratch buffer (no allocation per delivery).
    fn run_callback(&mut self, token: u64, sid: &[u8], bid: &[u8]) -> i64 {
        if !self.quiet {
            self.log.push(format!("F t{}", token));
        }
        let mut op = std::mem::take(&mut self.cb);
        op.clear();
        op.push(4i64);
        op.push(0);
        dec(sid, &mut op);
        op[1] = (op.len() - 2) as i64;
        let p = op.len();
        op.push(0);
        dec(bid, &mut op);
        op[p] = (op.len() - p - 1) as i64;
        op.push(0);
        op.push(0);
        let r = self.call(&op);
        self.cb = op;
        r
    }

    fn advance(&mut self, ms: i64) -> bool {
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
                    if self.fire_token(i) == 4 {
                        return true;
                    }
                }
            }
        }
        self.now = target;
        false
    }

    fn deliver(&mut self, tok: u64, captured: bool) -> i64 {
        if let Some(i) = self.active.iter().position(|t| t.token == tok) {
            return self.fire_token(i);
        }
        match self.known.get(&tok).cloned() {
            Some((sid, bid)) if captured => self.run_callback(tok, &sid, &bid),
            _ => {
                self.log.push(format!("X deliver t{}", tok));
                0
            }
        }
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
        s.push_str(" | T");
        // t<token>:<due>:<budgetId units>:<1 when it is the budget's timer>
        for t in &self.active {
            let mut units = Vec::new();
            dec(&t.bid, &mut units);
            let b: Vec<String> = units.iter().map(|u| u.to_string()).collect();
            let own = self.by_bid.get(&t.bid) == Some(&t.token);
            s.push_str(&format!(" t{}:{}:{}:{}", t.token, t.due, b.join(","), own as i32));
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
    let mut halted = false;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                w = World::new();
                halted = false;
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "put" => w.put(&nums(&p[1..])),
            "now" => w.now = p[1].parse().unwrap(),
            "step" => {
                if !halted {
                    w.log.push(format!("@ {}", p[1]));
                }
            }
            _ if halted => {
                if p[0] == "end" {
                    for l in w.log.drain(..) {
                        writeln!(o, "{}", l).unwrap();
                    }
                    let ob = w.observe();
                    writeln!(o, "{}", ob).unwrap();
                    writeln!(o, "end").unwrap();
                }
            }
            "call" => {
                if w.call(&nums(&p[1..])) == 4 {
                    halted = true;
                }
            }
            "adv" => {
                if w.advance(p[1].parse().unwrap()) {
                    halted = true;
                }
            }
            "settime" => w.now = p[1].parse().unwrap(),
            "deliver" => {
                if w.deliver(p[1].trim_start_matches('t').parse().unwrap(), p.get(2) == Some(&"1")) == 4 {
                    halted = true;
                }
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

// perf <R> <reps>: one session/budget with R runs (all running). Items:
//   refresh_R  : runWaiting(k_last, true) then runWaiting(k_last, false) on R runs where the others wait, so each
//                call reaches REFRESH (O(1) via the active-run count) and a SETTLE / BEGIN+ARM transition (2 calls per rep)
//   cycle      : timer cycle with one run: runStarted (BEGIN+ARM, timerSet), runEnded (SETTLE, timerClear) (2 calls)
//   fire       : early delivery: ARM then FIRE with time left (SETTLE + BEGIN + ARM re-arm) (1 fire per rep)
// prints `item size ns_per_call`.
fn run_perf(args: &[String]) {
    let rsz: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let key = |s: &str| -> Vec<i64> {
        let mut v = vec![s.len() as i64];
        v.extend(s.encode_utf16().map(|u| u as i64));
        v
    };
    let sid = key("session-perf");
    let bid = key("99b568f7-0000-4000-8000-000000000001");
    let mut put = sid.clone();
    let mut rec = vec![0i64, 0, 1000000, 0, 0, 0, (bid.len() - 1) as i64];
    rec.extend_from_slice(&bid[1..]);
    put.push(rec.len() as i64);
    put.extend(rec);
    let mk = |code: i64, rk: &Vec<i64>, flag: i64| -> Vec<i64> {
        let mut op = vec![code];
        op.extend_from_slice(&sid);
        op.extend_from_slice(&bid);
        op.extend_from_slice(rk);
        op.push(flag);
        op
    };
    // refresh_R
    let mut w = World::new();
    w.quiet = true;
    w.put(&put);
    let keys: Vec<Vec<i64>> = (0..rsz).map(|i| key(&format!("run-{:04}", i))).collect();
    for k in &keys {
        w.call(&mk(1, k, 0));
    }
    for k in &keys[..rsz - 1] {
        w.call(&mk(2, k, 1));
    }
    let last = &keys[rsz - 1];
    let wt = mk(2, last, 1);
    let wf = mk(2, last, 0);
    for _ in 0..1000 {
        w.call(&wt);
        w.call(&wf);
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.call(black_box(&wt)));
        black_box(w.call(black_box(&wf)));
    }
    let ns = t.elapsed().as_nanos() as f64 / (2 * reps) as f64;
    println!("refresh {} {:.1}", rsz, ns);
    // cycle
    let mut w = World::new();
    w.quiet = true;
    w.put(&put);
    let k0 = key("run-0000");
    let st = mk(1, &k0, 0);
    let en = mk(3, &k0, 0);
    for _ in 0..1000 {
        w.call(&st);
        w.call(&en);
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.call(black_box(&st)));
        black_box(w.call(black_box(&en)));
    }
    let ns = t.elapsed().as_nanos() as f64 / (2 * reps) as f64;
    println!("cycle {} {:.1}", 1, ns);
    // fire (early delivery re-arm)
    let mut w = World::new();
    w.quiet = true;
    w.put(&put);
    w.call(&st);
    let t = Instant::now();
    for _ in 0..reps {
        w.now += 1;
        let i = w.active.len() - 1;
        black_box(w.fire_token(i));
    }
    let ns = t.elapsed().as_nanos() as f64 / reps as f64;
    println!("fire {} {:.1}", 1, ns);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

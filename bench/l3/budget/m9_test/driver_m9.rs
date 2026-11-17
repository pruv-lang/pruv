// M9 driver (appended to a main file that already declares `mod sol;`, see run_m9.py).
//
//   driver_m9 q        program on stdin, one case after another:
//                        case <id> | put <words...> | now <ms> | step <i> | call <op words...> | adv <ms> | settime <ms>
//                        | deliver <token> | end
//   driver_m9 perf     perf program, see run_m9.py (cmd_perf)
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words. It is the
// timer scheduler only: a timerSet effect registers token t<n> (n counts registrations) due at now + effective delay
// for that budget, timerClear removes the budget's active token, and an elapsed timer calls FIRE (op code 4) with
// now = due, in (due, registration) order. A delivery of a token that is no longer active is dropped (the host
// cleared it; a real scheduler cannot run a cleared callback). Grow: status -N -> buffer of max(N, 2*len), t_grow,
// retry.
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

struct Timer {
    token: u64,
    due: i64,
    bid: Vec<u16>,
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
    by_bid: HashMap<Vec<u16>, u64>,
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
        // budget id of the ref (scheduler key), built only when there is an effect to apply
        let bid: Vec<u16> = if used > 0 {
            let (bs, be) = key_at(op, 2 + op[1] as usize);
            op[bs..be].iter().map(|&u| u as u16).collect()
        } else {
            Vec::new()
        };
        let mut i = 1;
        while i <= used {
            match self.fx[i] {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    self.clear_bid(&bid, false);
                    self.seq += 1;
                    let t = Timer { token: self.seq, due: self.now + eff, bid: bid.clone() };
                    self.by_bid.insert(bid.clone(), self.seq);
                    if !self.quiet {
                        self.log.push(format!("S t{} {}", self.seq, req));
                    }
                    self.active.push(t);
                    i += 3;
                }
                13 => {
                    self.clear_bid(&bid, true);
                    i += 1;
                }
                _ => {
                    self.log.push(format!("X fx{}", self.fx[i]));
                    break;
                }
            }
        }
        if r == 4 && !self.quiet {
            self.log.push("H".to_string());
        }
        r
    }

    fn clear_bid(&mut self, bid: &Vec<u16>, logit: bool) {
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
        self.by_bid.remove(&t.bid);
        if !self.quiet {
            self.log.push(format!("F t{}", t.token));
        }
        let mut op = vec![4i64, 0];
        op.push(t.bid.len() as i64);
        op.extend(t.bid.iter().map(|&u| u as i64));
        op.push(0);
        op.push(0);
        self.call(&op)
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

    fn deliver(&mut self, tok: u64) -> i64 {
        match self.active.iter().position(|t| t.token == tok) {
            Some(i) => self.fire_token(i),
            None => {
                self.log.push(format!("D t{}", tok));
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
        for t in &self.active {
            let b: Vec<String> = t.bid.iter().map(|u| u.to_string()).collect();
            s.push_str(&format!(" t{}:{}:{}", t.token, t.due, b.join(",")));
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
                if w.deliver(p[1].trim_start_matches('t').parse().unwrap()) == 4 {
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
//                call reaches REFRESH with a full O(R) scan and a SETTLE / BEGIN+ARM transition (2 calls per rep)
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

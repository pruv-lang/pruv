// M9 driver (appended to a main file that already declares `mod sol;`, see run_m9.py).
//
//   driver_m9 q        program on stdin, one case after another:
//                        case <id> | put <words...> | now <ms> | step <i> | call <op words...> | adv <ms> | settime <ms>
//                        | deliver <token> <captured 0/1> | inspect | end
//   driver_m9 perf     perf program, see run_m9.py (cmd_perf)
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words. It is the
// timer scheduler only, like setTimeout / clearTimeout: a timerSet effect registers token t<n> (n counts
// registrations) due at now + effective delay, with the ref (sessionId, budgetId) of the call that armed it (the
// callback's captured ref), and makes it the budget's timer; timerClear cancels the budget's timer; timerForget makes
// the budget own no timer without cancelling it (FIRE dropped its handle). An elapsed timer leaves the schedule and
// calls FIRE (op code 4) with its captured ref and now = due, in (due, registration) order. deliver runs an active
// token the same way; an inactive one (cancelled or already run) runs its captured callback only with captured = 1,
// otherwise the script is invalid (X deliver). Grow: status -N -> buffer of exactly N (Pruv sizes it, N >= 2*len), t_grow, retry.
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// Op keys are packed u16 keys `n, words(n) packed words` (bud_keys layout), packed once by the encoder (run_m9.py, or
// pack_u16 below for perf and the timer callback).
// Scheduler keys hold the op's key units in the byte form hand_m9.rs keys its Strings by (each unit as its UTF-8
// style 1-3 byte sequence, an injective encoding; ASCII keys are one byte per unit, the same bytes as the String):
// the host unpacks the op key's units (reused scratch) and byte-encodes them; packed words never reach the scheduler.
// The host never interprets them. A key is encoded into a reused scratch buffer and lookups borrow it; a key is
// copied only when a timer captures its ref or a budget gets its first map slot, the same allocations as hand_m9.rs
// (which clones the ref's two Strings and inserts the budget key the same way).
struct Timer {
    token: u64,
    due: i64,
    bid: Vec<u8>,
    // the captured ref as the FIRE op it delivers: `4, sessionId record, budgetId record, 0, 0` (packed words, the
    // form t_call takes, like the hand side keeps the ref's Strings)
    cap: Vec<i64>,
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

struct World<const CHECK_I2: bool> {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    tmp: Vec<i64>,
    fx: Vec<i64>,
    now: i64,
    seq: u64,
    active: Vec<Timer>,
    by_bid: HashMap<Vec<u8>, u64>,
    known: HashMap<u64, (Vec<i64>, Vec<u8>)>,
    kb: Vec<u8>,
    log: Vec<String>,
    quiet: bool,
    i2_checks: usize,
    i2_bad: usize,
}

// Data words of a packed key of n units: ceil(n / 4).
fn words(n: i64) -> usize {
    (n.max(0) as usize + 3) / 4
}

// The packed key record at op[p]: (unit count, first word, end).
fn key_at(op: &[i64], p: usize) -> (usize, usize, usize) {
    let n = op[p].max(0) as usize;
    (n, p + 1, p + 1 + words(op[p]))
}

// Appends the n units of packed words `w` to out.
fn unpack_into(n: usize, w: &[i64], out: &mut Vec<i64>) {
    for j in 0..n {
        out.push(((w[j / 4] as u64) >> (48 - 16 * (j % 4))) as i64 & 0xffff);
    }
}

// Packed-u16 words (bud_keys layout) of units, appended to out.
fn pack_into(units: &[i64], out: &mut Vec<i64>) {
    for c in units.chunks(4) {
        let mut x: u64 = 0;
        for l in 0..4 {
            x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64 & 0xffff);
        }
        out.push(x as i64);
    }
}

// Byte form of the packed key at op[p]: the bytes enc() gives for its units, read straight from the packed words (no
// unit scratch). When every lane is below 0x80 (the zero tail lanes too) each unit is one byte, four per word.
fn enc_key(op: &[i64], p: usize, out: &mut Vec<u8>) {
    let (n, s, e) = key_at(op, p);
    let w = &op[s..e];
    out.clear();
    if w.iter().all(|&x| (x as u64) & 0xff80_ff80_ff80_ff80 == 0) {
        for &x in w {
            let x = x as u64;
            out.extend_from_slice(&[(x >> 48) as u8, (x >> 32) as u8, (x >> 16) as u8, x as u8]);
        }
        out.truncate(n);
        return;
    }
    for j in 0..n {
        let u = (((w[j / 4] as u64) >> (48 - 16 * (j % 4))) & 0xffff) as u32;
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

impl<const CHECK_I2: bool> World<CHECK_I2> {
    fn new() -> Self {
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
            kb: Vec::with_capacity(64),
            log: Vec::new(),
            quiet: false,
            i2_checks: 0,
            i2_bad: 0,
        }
    }

    fn grow(&mut self, n: usize) {
        let want = n; // N from Pruv (bud_store::need, N >= 2*len): the host makes no sizing choice (L3 P13b)
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
            if CHECK_I2 {
                self.check_i2();
            }
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
        eprintln!("t_put still Grow after 4 tries");
        std::process::exit(2);
    }

    fn check_i2(&mut self) {
        self.i2_checks += 1;
        let bad = sol::t_i2(&self.st);
        if bad != 0 {
            self.i2_bad += 1;
            self.log.push(format!("I2 {}", bad));
        }
    }

    // One Pruv call; returns the status after Grow retries, then runs the effects.
    fn call(&mut self, op: &[i64]) -> i64 {
        self.call_k(op, None)
    }

    // `key`: the budgetId bytes of op when the host already has them (a timer callback keeps its captured ref's
    // scheduler key); otherwise they are encoded from the op's packed record.
    fn call_k(&mut self, op: &[i64], key: Option<&[u8]>) -> i64 {
        let mut r = 0;
        for _ in 0..4 {
            self.fx[0] = 0;
            r = sol::t_call(&mut self.st, op, self.now, &mut self.bv, &mut self.tmp, &mut self.fx);
            if CHECK_I2 {
                self.check_i2();
            }
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
        // ref of the call: the scheduler key (budgetId bytes, encoded into a scratch buffer) and the captured ref (the
        // op's sessionId and budgetId records, op[1..be])
        let bp = 2 + words(op[1]);
        let be = key_at(op, bp).2;
        let mut kb = std::mem::take(&mut self.kb);
        let bid = match key {
            Some(k) => k,
            None => {
                enc_key(op, bp, &mut kb);
                &kb[..]
            }
        };
        let mut i = 1;
        while i <= used {
            match self.fx[i] {
                12 => {
                    let req = self.fx[i + 1];
                    let eff = self.fx[i + 2];
                    // like `clock.timer = setTimeout(..)`: a previous handle is overwritten, not cancelled
                    self.seq += 1;
                    let mut cap = Vec::with_capacity(be + 2);
                    cap.push(4i64);
                    cap.extend_from_slice(&op[1..be]);
                    cap.push(0);
                    cap.push(0);
                    match self.by_bid.get_mut(bid) {
                        Some(v) => *v = self.seq,
                        None => {
                            self.by_bid.insert(bid.to_vec(), self.seq);
                        }
                    }
                    if !self.quiet {
                        self.known.insert(self.seq, (cap.clone(), bid.to_vec()));
                        self.log.push(format!("S t{} {}", self.seq, req));
                    }
                    self.active.push(Timer { token: self.seq, due: self.now + eff, bid: bid.to_vec(), cap });
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
        self.halt_log(op, r)
    }

    fn halt_log(&mut self, op: &[i64], r: i64) -> i64 {
        if r == 4 && !self.quiet {
            // HALT_TIME: when and for which session (the caller's halt(ref, 'time') is M10), in units
            let (n, s, e) = key_at(op, 1);
            let mut units = Vec::new();
            unpack_into(n, &op[s..e], &mut units);
            let u: Vec<String> = units.iter().map(|x| x.to_string()).collect();
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
        self.run_callback(t.token, &t.cap, &t.bid)
    }

    // The timer callback `() => this.fire(ref)` with its captured ref, kept as the FIRE op since the timer was set.
    fn run_callback(&mut self, token: u64, cap: &[i64], bid: &[u8]) -> i64 {
        if !self.quiet {
            self.log.push(format!("F t{}", token));
        }
        self.call_k(cap, Some(bid))
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
            Some((cap, bid)) if captured => self.run_callback(tok, &cap, &bid),
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
    let mut w = World::<true>::new();
    let mut i2_checks = 0;
    let mut i2_bad = 0;
    let mut halted = false;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                i2_checks += w.i2_checks;
                i2_bad += w.i2_bad;
                w = World::new();
                halted = false;
                writeln!(o, "case {}", p[1]).unwrap();
            }
            // After a halt the script stops (the observation is the one at the halt), so later puts are skipped.
            "put" if !halted => w.put(&nums(&p[1..])),
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
            "inspect" => {
                w.log.push(format!("L {} {}", sol::t_live(&w.st), w.st.len()));
                let snapshot = w.observe();
                w.log.push(format!("V {}", snapshot));
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
    i2_checks += w.i2_checks;
    i2_bad += w.i2_bad;
    o.flush().unwrap();
    eprintln!("i2_checks={} i2_bad={}", i2_checks, i2_bad);
    if i2_bad > 0 {
        std::process::exit(3);
    }
}

// perf <R> <reps>: one session/budget with R runs (all running). Items:
//   refresh_R  : runWaiting(k_last, true) then runWaiting(k_last, false) on R runs where the others wait, so each
//                call reaches REFRESH (O(1) via the active-run count) and a SETTLE / BEGIN+ARM transition (2 calls per rep)
//   cycle      : timer cycle with one run: runStarted (BEGIN+ARM, timerSet), runEnded (SETTLE, timerClear) (2 calls)
//   fire       : early delivery: ARM then FIRE with time left (SETTLE + BEGIN + ARM re-arm) (1 fire per rep)
//   drop/close : build R runs, then DROP or advance 1 ms and CLOSE (ns per repetition)
//   stale/u    : captured FIRE after a budget id change or an unlimited budget replacement
// prints `item size ns`.
fn run_perf(args: &[String]) {
    let rsz: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    // wire key `n, packed words(n)`
    let key = |s: &str| -> Vec<i64> {
        let u: Vec<i64> = s.encode_utf16().map(|u| u as i64).collect();
        let mut v = vec![u.len() as i64];
        pack_into(&u, &mut v);
        v
    };
    let sid = key("session-perf");
    let bid = key("99b568f7-0000-4000-8000-000000000001");
    let mut put = sid.clone();
    // record word 6 = bn, then the packed id words
    // timeMinutes 60 keeps the timer delay (3,600,000 ms) under the 2^31-1 ms host cap, so advance(1) before CLOSE
    // does not deliver the timer and CLOSE settles the 1 ms itself (P23 audit)
    let mut rec = vec![0i64, 0, 60, 0, 0, 0];
    rec.extend_from_slice(&bid);
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
    let mut w = World::<false>::new();
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
    let mut w = World::<false>::new();
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
    // fire (early delivery re-arm). Each FIRE spends 1 ms of budget, so a 60-minute budget would run out after
    // 3,600,000 reps and leave no timer to fire (P23 confirm audit). timeMinutes 1000000 gives a delay over the host
    // cap (capped to 1 ms) and a budget no rep count reaches.
    let mut put_fire = put.clone();
    put_fire[sid.len() + 1 + 2] = 1000000;
    let mut w = World::<false>::new();
    w.quiet = true;
    w.put(&put_fire);
    w.call(&st);
    let t = Instant::now();
    for _ in 0..reps {
        w.now += 1;
        let i = w.active.len() - 1;
        black_box(w.fire_token(i));
    }
    let ns = t.elapsed().as_nanos() as f64 / reps as f64;
    println!("fire {} {:.1}", 1, ns);

    // Measure DROP/CLOSE of a clock with R runs. The R runStarted calls, the 1 ms advance before CLOSE and the host
    // budget reset are outside the timed interval; only the DROP or CLOSE call is timed (P23 audit).
    let starts: Vec<_> = keys.iter().map(|k| mk(1, k, 0)).collect();
    for (item, code) in [("drop", 5), ("close", 6)] {
        let op = mk(code, &vec![0], 0);
        let mut w = World::<false>::new();
        w.quiet = true;
        w.put(&put);
        let mut elapsed = std::time::Duration::ZERO;
        for _ in 0..reps {
            for start in &starts {
                black_box(w.call(black_box(start)));
            }
            if code == 6 {
                w.advance(1);
            }
            // the armed timer t1 is alive (not delivered by the advance) when DROP/CLOSE runs
            assert!(w.active.len() == 1 && w.active[0].due > w.now, "{}: timer not alive", item);
            let t = Instant::now();
            black_box(w.call(black_box(&op)));
            elapsed += t.elapsed();
            assert!(w.active.is_empty(), "{}: timer not cleared", item);
            if code == 6 {
                w.put(&put);
            }
        }
        println!("{} {} {:.1}", item, rsz, elapsed.as_nanos() as f64 / reps as f64);
    }

    // Prepare an armed clock and capture its callback before changing the session budget.
    if rsz == 10 {
        for (item, unlimited) in [("stale", false), ("staleu", true)] {
            let mut changed = put.clone();
            let record = sid.len() + 1;
            if unlimited {
                changed[record + 1] = 1;
            } else {
                changed[record + 6..].copy_from_slice(&key("99b568f7-0000-4000-8000-000000000002"));
            }
            let drop = mk(5, &vec![0], 0);
            let mut w = World::<false>::new();
            w.quiet = true;
            let mut elapsed = std::time::Duration::ZERO;
            for _ in 0..reps {
                w.put(&put);
                w.call(&st);
                w.put(&changed);
                let captured = w.active.pop().unwrap();
                let t = Instant::now();
                black_box(w.run_callback(captured.token, &captured.cap, &captured.bid));
                elapsed += t.elapsed();
                w.call(&drop);
            }
            println!("{} 1 {:.1}", item, elapsed.as_nanos() as f64 / reps as f64);
        }
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

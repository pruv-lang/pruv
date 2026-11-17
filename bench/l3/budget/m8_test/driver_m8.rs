// M8 driver (appended to a main file that already declares `mod sol;`, see run_m8.py).
//
//   driver_m8 q              program on stdin, one case after another:
//                              case <id> | put <words...> | step <i> | call <op words...> | end
//                              | poke <k> <p1> ... <pk> <delta> <value>
//                              poke corrupts the resident buffer for tests (see corrupt_resident.py): the first index i
//                              with st[i..i+k] == [p1..pk] gets st[i+delta] = value (when in range); prints `Z <i>`, or
//                              `Z -` when the pattern is absent. It calls no sol:: function.
//   driver_m8 perf <N> <reps> perf items, see run_perf below
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words and decides
// nothing. Per call it prints what Pruv returned: `R <charge status>` (fx[1]) or `R -` for the void methods. On status 6
// (a HALT dispatch the root does not implement yet, M10) it prints `H <halt frame words>` and stops the case: the halt
// sub-operation would run next and nothing after it is executed or filled. Any other non-zero status prints
// `X <status>` and stops the case. A put the root rejects (malformed input, status 2) prints `P X <status>`, leaves the
// state unchanged and the case continues (perf-mode puts stay fatal). perf also prints `chk <item> <size> <value>`
// after each item's timed loop: the t_observe words folded as s = s*31 + w (wrapping u64). Grow: status -N -> buffer of max(N, 2*len), t_grow, retry.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    cv: Vec<i64>,
    fx: Vec<i64>,
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
        World { st, out: vec![0; 4096], cv: vec![0; 16], fx: vec![0; 256] }
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

    // One session put; returns the status after Grow retries (0 = stored, 2 = rejected malformed input with the
    // state unchanged). The caller decides what a non-zero status means.
    fn put(&mut self, op: &[i64]) -> i64 {
        let mut r = 0;
        for _ in 0..4 {
            r = sol::t_put(&mut self.st, op, &mut self.out);
            if r == 1 {
                let n = self.out[0].max(0) as usize;
                self.grow(n);
                continue;
            }
            return r;
        }
        r
    }

    // Folds the full observed final state (the t_observe words, same words as q mode prints after `O`) into a u64.
    // Used by perf after a timed loop, outside the timing.
    fn checksum(&mut self) -> u64 {
        let need = std::cmp::max(4096, 2 * self.st.len());
        if self.out.len() < need {
            self.out.resize(need, 0);
        }
        let r = sol::t_observe(&self.st, &mut self.out);
        if r != 0 {
            eprintln!("t_observe status {}", r);
            std::process::exit(2);
        }
        let n = self.out[0].max(0) as usize;
        let mut s: u64 = 0;
        for &x in &self.out[1..1 + n] {
            s = s.wrapping_mul(31).wrapping_add(x as u64);
        }
        s
    }

    // One Pruv call; returns the status after Grow retries.
    #[inline]
    fn call(&mut self, op: &[i64]) -> i64 {
        loop {
            let r = sol::t_call(&mut self.st, op, &mut self.cv, &mut self.fx);
            if r < 0 {
                self.grow((-r) as usize);
                continue;
            }
            return r;
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
                // a rejected put (malformed input) leaves the state unchanged and the case continues
                let r = w.put(&nums(&p[1..]));
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
                let r = w.call(&op);
                if r == 0 {
                    if op[0] == 1 {
                        writeln!(o, "R {}", w.fx[1]).unwrap();
                    } else {
                        writeln!(o, "R -").unwrap();
                    }
                } else if r == 6 {
                    let n = w.fx[0] as usize;
                    let f: Vec<String> = w.fx[2..1 + n].iter().map(|x| x.to_string()).collect();
                    writeln!(o, "H {}", f.join(" ")).unwrap();
                    stopped = true;
                } else {
                    writeln!(o, "X {}", r).unwrap();
                    stopped = true;
                }
            }
            "call" => {}
            "poke" => {
                // poke <k> <p1> ... <pk> <delta> <value>: test-only corruption of the resident buffer (calls no sol::
                // function). Finds the first i (scanning st from 0) with st[i..i+k] == [p1..pk]; if found, sets
                // st[i+delta] = value (only when that index is in range) and prints `Z <i>`, else prints `Z -`.
                let k: usize = p[1].parse().unwrap();
                let pat = nums(&p[2..2 + k]);
                let delta: i64 = p[2 + k].parse().unwrap();
                let value: i64 = p[3 + k].parse().unwrap();
                let found = if k == 0 || w.st.len() < k {
                    None
                } else {
                    (0..=w.st.len() - k).find(|&i| w.st[i..i + k] == pat[..])
                };
                match found {
                    Some(i) => {
                        let t = i as i64 + delta;
                        if t >= 0 && (t as usize) < w.st.len() {
                            w.st[t as usize] = value;
                        }
                        writeln!(o, "Z {}", i).unwrap();
                    }
                    None => writeln!(o, "Z -").unwrap(),
                }
            }
            "end" => {
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

// Packed-u16 key codec (bud_keys.pruv): word i = (u[4i] << 48) | (u[4i+1] << 32) | (u[4i+2] << 16) | u[4i+3] as a
// two's-complement i64; lanes past n are zero. A pure ingress encoding, no validation.
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

// Wire key: `n, packed words(n)` (1 + ceil(n/4) words).
fn key(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    v.extend(pack_u16(&u));
    v
}

fn bid_of(i: usize) -> String {
    format!("99b568f7-0000-4000-8000-{:012}", i)
}

// N sessions `session-<i>` with an open budget (limits 10^12, so charges never reach a limit).
fn world_n(n: usize) -> World {
    let mut w = World::new();
    for i in 0..n {
        let mut put = key(&format!("session-{:05}", i));
        let bid = key(&bid_of(i));
        let mut rec = vec![0i64, 0, -1];
        rec.extend_from_slice(&[1_000_000_000_000i64; 4]);
        rec.extend_from_slice(&[0i64; 7]);
        rec.extend_from_slice(&bid); // [bn] + packed budgetId words, so rec.len() == 14 + 1 + ceil(bn/4)
        put.push(rec.len() as i64);
        put.extend(rec);
        let r = w.put(&put);
        if r != 0 {
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
    }
    w
}

fn refk(i: usize) -> Vec<i64> {
    let mut v = key(&format!("session-{:05}", i));
    v.extend(key(&bid_of(i)));
    v
}

// perf <N> <reps>, prints `item size ns_per_call`:
//   charge   : charge(ref_i, {commands: 1, retries: 1}) round-robin over N sessions (successful charge)
//   failure  : noteFailure(ref_i) round-robin over N sessions
//   tooldup  : noteTool(ref_0, t1, a_j) on a turn that already holds N activities (dedup hit, no CURRENT)
//   toolgrow : noteTool(ref_0, t2, a_j) for N new activities, then forgetTurn(t2); ns per noteTool (forget included)
// An optional third argument names one item: only that item's timed loop runs (the setup of every item still runs),
// so `/usr/bin/time -l` instruction counts at two rep counts give the instructions per call of that item alone.
fn run_perf(args: &[String]) {
    let n: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut w = world_n(n);
    let ops: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            let mut op = vec![1];
            op.extend(refk(i));
            op.extend_from_slice(&[0, 0, 2, 1, 0, 0, 0, 0, 2, 1]);
            op
        })
        .collect();
    for i in 0..1000 {
        w.call(&ops[i % n]);
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(&ops[i % n])));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("charge {} {:.1}", n, ns);
        println!("chk charge {} {}", n, w.checksum()); // outside the timing
    }
    let fo: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            let mut op = vec![2];
            op.extend(refk(i));
            op
        })
        .collect();
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(&fo[i % n])));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("failure {} {:.1}", n, ns);
        println!("chk failure {} {}", n, w.checksum());
    }
    let tool = |turn: &str, j: usize| -> Vec<i64> {
        let mut op = vec![3];
        op.extend(refk(0));
        op.extend(key(turn));
        op.extend(key(&format!("activity-{:05}", j)));
        op
    };
    let dup: Vec<Vec<i64>> = (0..n).map(|j| tool("turn-1", j)).collect();
    for d in &dup {
        w.call(d);
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(&dup[i % n])));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("tooldup {} {:.1}", n, ns);
        println!("chk tooldup {} {}", n, w.checksum());
    }
    let grow: Vec<Vec<i64>> = (0..n).map(|j| tool("turn-2", j)).collect();
    let mut forget = vec![4];
    forget.extend(key("turn-2"));
    for d in &grow {
        w.call(d);
    }
    w.call(&forget);
    if on("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let t = Instant::now();
        for _ in 0..rounds {
            for d in &grow {
                black_box(w.call(black_box(d)));
            }
            black_box(w.call(black_box(&forget)));
        }
        let ns = t.elapsed().as_nanos() as f64 / (rounds * n) as f64;
        println!("toolgrow {} {:.1}", n, ns);
        println!("chk toolgrow {} {}", n, w.checksum());
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

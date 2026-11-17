// M8 driver (appended to a main file that already declares `mod sol;`, see run_m8.py).
//
//   driver_m8 q              program on stdin, one case after another:
//                              case <id> | put <words...> | step <i> | call <op words...> | end
//   driver_m8 perf <N> <reps> perf items, see run_perf below
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words and decides
// nothing. Per call it prints what Pruv returned: `R <charge status>` (fx[1]) or `R -` for the void methods. On status 6
// (a HALT dispatch the root does not implement yet, M10) it prints `H <halt frame words>` and stops the case: the halt
// sub-operation would run next and nothing after it is executed or filled. Any other non-zero status prints
// `X <status>` and stops the case. Grow: status -N -> buffer of max(N, 2*len), t_grow, retry.
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
            "put" => w.put(&nums(&p[1..])),
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

fn key(s: &str) -> Vec<i64> {
    let mut v = vec![s.encode_utf16().count() as i64];
    v.extend(s.encode_utf16().map(|u| u as i64));
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
        rec.extend_from_slice(&bid);
        put.push(rec.len() as i64);
        put.extend(rec);
        w.put(&put);
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
        println!("charge {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
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
        println!("failure {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
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
        println!("tooldup {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
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
        println!("toolgrow {} {:.1}", n, t.elapsed().as_nanos() as f64 / (rounds * n) as f64);
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

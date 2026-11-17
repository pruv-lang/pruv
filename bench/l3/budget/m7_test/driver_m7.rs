// M7 driver (appended to a main file that already declares `mod sol;`, see run_m7.py).
//
//   driver_m7 q        program on stdin, one case after another:
//                        case <id> | now <ms> | uuids <key>... | quiet <0/1>... | put <words...> | del <key words...>
//                        | adv <ms> | call <req> <op words without now/quiet/uuid> | end
//   driver_m7 perf <S> <reps>   perf program, see run_m7.py (cmd_perf)
//
// Boundary rule: the host owns the state buffer and the scratch frames and supplies the inputs of a call (now, the
// next randomUUID value, the isQuiet answer); it never reads state words and decides nothing. It reads the result
// tag in out only to print the words (a budget result is a view [4, n, off]: the host prints st[off..off+n), as a
// caller reads through the returned reference). Effects: [20] changed, [21] carryHeld, [22] uuid consumed (the host
// advances its UUID stream), [23] isQuiet consulted (the host advances its isQuiet answers; the answer, next queued
// or default false, is supplied up front in the op like the next UUID), [13] timerClear (no timers exist in M7
// scenarios; logged). Need 5 only answers a call made without an isQuiet input; this host always supplies one.
// Grow -N: buffer of max(N, 2*len), t_grow, retry.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    fx: Vec<i64>,
    op: Vec<i64>,
    now: i64,
    uuids: Vec<Vec<i64>>,
    next_uuid: usize,
    quiet: Vec<i64>,
    next_quiet: usize,
    log: Vec<String>,
    silent: bool,
    // perf only (silent): the ops built so far, keyed by (body address, body length, quiet, uuid index). The perf
    // bodies are fixed vectors, so a host that keeps its prepared arguments does not rebuild them on every call.
    ops: Vec<((usize, usize, i64, usize), Vec<i64>)>,
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
            fx: vec![0; 16],
            op: Vec::with_capacity(256),
            now: 1700000000000,
            uuids: Vec::new(),
            next_uuid: 0,
            quiet: Vec::new(),
            next_quiet: 0,
            log: Vec::new(),
            silent: false,
            ops: Vec::new(),
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

    fn del(&mut self, op: &[i64]) {
        let r = sol::t_del(&mut self.st, op, &mut self.out);
        if r != 0 && !self.silent {
            self.log.push(format!("X del {}", r));
        }
    }

    // One public call: body = [code, origin, size, unit x6, sessionId key, budgetId key]; the host inserts now, the
    // next isQuiet answer and the next UUID key.
    fn call(&mut self, body: &[i64]) -> i64 {
        let quiet = self.quiet.get(self.next_quiet).copied().unwrap_or(0);
        let k = (body.as_ptr() as usize, body.len(), quiet, self.next_uuid);
        let mut slot = usize::MAX;
        if self.silent {
            slot = match self.ops.iter().position(|(key, _)| *key == k) {
                Some(i) => i,
                None => {
                    self.ops.push((k, Vec::new()));
                    self.ops.len() - 1
                }
            };
        }
        let mut op = if slot == usize::MAX { std::mem::take(&mut self.op) } else { std::mem::take(&mut self.ops[slot].1) };
        if op.is_empty() || slot == usize::MAX {
            op.clear();
            op.push(body[0]);
            op.push(self.now);
            op.push(quiet);
            op.extend_from_slice(&body[1..]);
            match self.uuids.get(self.next_uuid) {
                Some(u) => op.extend_from_slice(u),
                None => op.push(0),
            }
        }
        op[1] = self.now;
        let mut r = 0;
        for _ in 0..8 {
            self.fx[0] = 0;
            r = sol::t_call(&mut self.st, &op, &mut self.bv, &mut self.fx, &mut self.out);
            if r < 0 {
                self.grow((-r) as usize);
                continue;
            }
            break;
        }
        if slot == usize::MAX {
            self.op = op;
        } else {
            self.ops[slot].1 = op;
        }
        let used = self.fx[0].max(0) as usize;
        if r != 0 {
            // A finished call that consulted isQuiet consumed the answer even when it failed afterwards.
            if self.fx[1..=used].contains(&23) {
                self.next_quiet += 1;
                if !self.silent {
                    self.log.push(format!("E quiet {}", quiet));
                }
            }
            if !self.silent {
                self.log.push(format!("X {}", r));
            }
            return r;
        }
        for i in 1..=used {
            let e = self.fx[i];
            if e == 22 {
                self.next_uuid += 1;
            }
            if e == 23 {
                self.next_quiet += 1;
            }
            if !self.silent {
                self.log.push(match e {
                    20 => "E changed".to_string(),
                    21 => "E carry".to_string(),
                    22 => "E uuid".to_string(),
                    23 => format!("E quiet {}", quiet),
                    13 => "E clear".to_string(),
                    x => format!("X fx{}", x),
                });
            }
        }
        if !self.silent {
            let w: Vec<String> = match self.out[0] {
                3 => {
                    let l = 2 + self.out[1].max(0) as usize;
                    let k = if self.fx[1..=used].contains(&21) { l + 1 + self.out[l].max(0) as usize } else { l };
                    self.out[..k].iter().map(|x| x.to_string()).collect()
                }
                4 => {
                    // budget view: the record is st[off..off+n)
                    let n = self.out[1].max(0) as usize;
                    let off = self.out[2].max(0) as usize;
                    let mut v = vec!["4".to_string(), n.to_string()];
                    v.extend(self.st[off..off + n].iter().map(|x| x.to_string()));
                    v
                }
                x => vec![x.to_string()],
            };
            self.log.push(format!("V {}", w.join(" ")));
        }
        0
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

// A run of keys `len, units...` into separate keys.
fn keys(w: &[i64]) -> Vec<Vec<i64>> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < w.len() {
        let n = w[p] as usize;
        v.push(w[p..p + 1 + n].to_vec());
        p += 1 + n;
    }
    v
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
            "uuids" => w.uuids = keys(&nums(&p[1..])),
            "quiet" => w.quiet = nums(&p[1..]),
            "put" => w.put(&nums(&p[1..])),
            "del" => w.del(&nums(&p[1..])),
            "adv" => w.now += p[1].parse::<i64>().unwrap(),
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

// perf <S> <reps>: S sessions s0000..s<S-1>, each with an open budget (rest text of a typical card). Items:
//   current_S : current(ref) of the last session (hit)
//   replace_S : replaceForUserMessage on a halted budget of the last session (CLOSE + OPEN + carryHeld), then the
//               record is put back halted (the put is outside the timed call... see below: timed together, both sides)
//   reopen_S  : maybeClose(quiet) then reopen on the last session (2 calls per rep)
//   attach_S  : attach on an open budget of the last session (ref result)
// prints `item size ns_per_call`.
fn rec(id: &str, state: i64, rest: &str) -> Vec<i64> {
    let mut h = vec![0i64; 24];
    h[0] = state;
    h[3] = 120;
    h[4] = 24;
    h[5] = 3;
    h[6] = 2;
    h[7] = 1;
    h[20] = 1;
    let mut v = h;
    let mut s = |t: &str, v: &mut Vec<i64>| {
        let u: Vec<i64> = t.encode_utf16().map(|x| x as i64).collect();
        v.push(u.len() as i64);
        v.extend(u);
    };
    s(id, &mut v);
    s("2023-11-14T22:13:20.000Z", &mut v);
    s("", &mut v);
    s("", &mut v);
    s("", &mut v);
    s(rest, &mut v);
    v
}

fn key(s: &str) -> Vec<i64> {
    let mut v = vec![s.encode_utf16().count() as i64];
    v.extend(s.encode_utf16().map(|u| u as i64));
    v
}

pub const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn run_perf(args: &[String]) {
    let size: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let sids: Vec<String> = (0..size).map(|i| format!("s{:04}", i)).collect();
    let bid = |i: usize| format!("{:08x}-0000-4000-8000-000000000001", i);
    let unit = [120i64, 24, 3, 2, 1, 0];
    let setup = |state: i64| -> World {
        let mut w = World::new();
        w.silent = true;
        for (i, s) in sids.iter().enumerate() {
            let mut op = key(s);
            let r = rec(&bid(i), state, PERF_REST);
            op.push(r.len() as i64);
            op.extend(r);
            w.put(&op);
        }
        w.uuids = vec![key("00000000-0000-4000-8000-0000000000aa")];
        w
    };
    let last = size - 1;
    let body = |code: i64, origin: i64, sz: i64, b: &str| -> Vec<i64> {
        let mut v = vec![code, origin, sz];
        v.extend_from_slice(&unit);
        v.extend(key(&sids[last]));
        v.extend(key(b));
        v
    };
    let time = |w: &mut World, ops: &[Vec<i64>], per: usize| -> f64 {
        for _ in 0..1000 {
            for op in ops {
                w.next_uuid = 0;
                w.next_quiet = 0;
                w.call(op);
            }
        }
        let t = Instant::now();
        for _ in 0..reps {
            for op in ops {
                w.next_uuid = 0;
                w.next_quiet = 0;
                black_box(w.call(black_box(op)));
            }
        }
        t.elapsed().as_nanos() as f64 / (reps * per) as f64
    };
    // dispatch probe: the host op build + t_call's checks (valid, 3 key scans) with an unknown code (status 2)
    let mut w = setup(0);
    let op = body(0, -1, -1, &bid(last));
    println!("dispatch {} {:.1}", size, time(&mut w, &[op], 1));
    // raw t_call with a prepared op (no World::call): what the dispatch probe costs inside t_call itself
    {
        let mut w = setup(0);
        let mut op = vec![0i64, w.now, -1];
        op.extend_from_slice(&body(0, -1, -1, &bid(last))[1..]);
        op.push(0);
        let t = Instant::now();
        for _ in 0..reps {
            black_box(sol::t_call(&mut w.st, black_box(&op), &mut w.bv, &mut w.fx, &mut w.out));
        }
        println!("rawcall {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    // isHalted (no record copy)
    let mut w = setup(0);
    let op = body(2, -1, -1, &bid(last));
    println!("ishalted {} {:.1}", size, time(&mut w, &[op], 1));
    // current
    let mut w = setup(0);
    let op = body(1, -1, -1, &bid(last));
    println!("current {} {:.1}", size, time(&mut w, &[op], 1));
    // attach (open budget: ref)
    let mut w = setup(0);
    let op = body(3, 0, -1, "");
    println!("attach {} {:.1}", size, time(&mut w, &[op], 1));
    // close + reopen cycle: maybeClose (quiet) then reopen
    let mut w = setup(0);
    w.quiet = vec![1];
    let mc = body(6, -1, -1, "");
    let ro = body(5, -1, -1, &bid(last));
    println!("closereopen {} {:.1}", size, time(&mut w, &[mc, ro], 2));
    // replace on a halted budget (CLOSE + OPEN + follows + carryHeld), then put the halted record back
    let mut w = setup(1);
    let rp = body(4, 0, 2, "");
    let mut put = key(&sids[last]);
    let r = rec(&bid(last), 1, PERF_REST);
    put.push(r.len() as i64);
    put.extend(r);
    for _ in 0..1000 {
        w.next_uuid = 0;
        w.call(&rp);
        w.put(&put);
    }
    let t = Instant::now();
    for _ in 0..reps {
        w.next_uuid = 0;
        black_box(w.call(black_box(&rp)));
        w.put(black_box(&put));
    }
    println!("replace {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

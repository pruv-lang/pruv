// M4 driver (appended to a main file that already declares `mod sol;`, see run.py).
//
//   driver_m4 q                      program on stdin: `init <L>` | `op <w...>` | `observe` (op keys already packed)
//   driver_m4 perf <axis> <n> <reps> axis = sessions | tools | clocks
//
// Boundary rule: the host owns the state buffer `st` but never reads or writes its words. It allocates, calls the
// entries, and on status 1 (Grow, N = out[0]) allocates a zeroed buffer of max(N, 2*len), calls m4_grow, swaps the
// buffers and retries the same op (at most 3 retries).
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

enum R {
    Ok(usize),
    Invalid,
    GrowFailed,
    OutSmall,
}

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    grows: u64,
    grow_ns: u128,
}

impl World {
    fn new(l: usize) -> World {
        let mut st = vec![0i64; l];
        let r = sol::m4_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            let r2 = sol::m4_init(&mut st);
            if r2 != 0 {
                eprintln!("m4_init failed twice (status {})", r2);
                std::process::exit(2);
            }
        } else if r < 0 {
            eprintln!("m4_init returned {}", r);
            std::process::exit(2);
        }
        World { st, out: Vec::new(), grows: 0, grow_ns: 0 }
    }

    fn exec(&mut self, op: &[i64]) -> R {
        for attempt in 0..4 {
            let need = std::cmp::max(4096, 4 * self.st.len());
            if self.out.len() < need {
                self.out.resize(need, 0);
            }
            let s = sol::m4_op(&mut self.st, op, &mut self.out);
            match s {
                0 => return R::Ok(self.out[0].max(0) as usize),
                1 => {
                    if attempt == 3 {
                        return R::GrowFailed;
                    }
                    let n = self.out[0].max(0) as usize;
                    let t = Instant::now();
                    let want = std::cmp::max(n, 2 * self.st.len());
                    let mut fresh = vec![0i64; want];
                    let g = sol::m4_grow(&self.st, &mut fresh);
                    if g != 0 {
                        return R::GrowFailed;
                    }
                    self.st = fresh;
                    self.grows += 1;
                    self.grow_ns += t.elapsed().as_nanos();
                }
                2 => return R::Invalid,
                _ => return R::OutSmall,
            }
        }
        R::GrowFailed
    }

    fn result(&self, k: usize) -> &[i64] {
        let hi = (1 + k).min(self.out.len());
        &self.out[1..hi]
    }
}

fn words_line(prefix: &str, ws: &[i64]) -> String {
    let mut s = String::from(prefix);
    for x in ws {
        s.push(' ');
        s.push_str(&x.to_string());
    }
    s
}

fn q_mode() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());
    let mut world: Option<World> = None;
    let mut total_grows = 0u64;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut it = line.split_whitespace();
        let cmd = match it.next() {
            Some(c) => c,
            None => continue,
        };
        match cmd {
            "init" => {
                let l: usize = it.next().expect("init needs L").parse().unwrap();
                if let Some(old) = &world {
                    total_grows += old.grows;
                }
                world = Some(World::new(l));
                writeln!(w, "init ok").unwrap();
            }
            "op" => {
                let op: Vec<i64> = it.map(|t| t.parse::<i64>().expect("bad word")).collect();
                let wd = world.as_mut().expect("op before init");
                match wd.exec(&op) {
                    R::Ok(k) => {
                        let res: Vec<i64> = wd.result(k).to_vec();
                        if !op.is_empty() && op[0] == 18 {
                            // stats: [live entries, used slots, live arena words, len(st)]; the host knows len(st).
                            if res.len() == 4 && res[3] == wd.st.len() as i64 {
                                writeln!(w, "{}", words_line("stats", &res)).unwrap();
                            } else {
                                writeln!(w, "{}", words_line("stats-bad", &res)).unwrap();
                            }
                        } else {
                            writeln!(w, "{}", words_line("ok", &res)).unwrap();
                        }
                    }
                    R::Invalid => writeln!(w, "invalid").unwrap(),
                    R::GrowFailed => writeln!(w, "grow-failed").unwrap(),
                    R::OutSmall => writeln!(w, "out-small").unwrap(),
                }
            }
            "observe" => {
                let wd = world.as_ref().expect("observe before init");
                let mut out = vec![0i64; std::cmp::max(4096, 4 * wd.st.len())];
                let s = sol::m4_observe(&wd.st, &mut out);
                if s != 0 {
                    writeln!(w, "observe-fail {}", s).unwrap();
                } else {
                    let n = (out[0].max(0) as usize).min(out.len() - 1);
                    let prefix = format!("observe {}", n);
                    writeln!(w, "{}", words_line(&prefix, &out[1..1 + n])).unwrap();
                }
            }
            _ => {
                eprintln!("unknown command {}", cmd);
                std::process::exit(2);
            }
        }
    }
    if let Some(wd) = &world {
        total_grows += wd.grows;
        writeln!(w, "final_len {}", wd.st.len()).unwrap();
    }
    writeln!(w, "grows {}", total_grows).unwrap();
    w.flush().unwrap();
}

// ---- perf mode -------------------------------------------------------------------------------------------------

// Packed-u16 key codec (bud_keys.pruv): word i = (u[4i] << 48) | (u[4i+1] << 32) | (u[4i+2] << 16) | u[4i+3] as a
// two's-complement i64; lanes past n are zero. A pure ingress encoding, no validation.
// Wire key `n, packed words(n)`, packed once at ingress straight from the UTF-16 units (no unit or word scratch).
// (q mode reads op lines that gen_m4.py already packed.)
fn key(v: &mut Vec<i64>, s: &str) {
    let np = v.len();
    v.push(0);
    let (mut x, mut k, mut n) = (0u64, 0u32, 0i64);
    for c in s.encode_utf16() {
        x = (x << 16) | c as u64;
        k += 1;
        n += 1;
        if k == 4 {
            v.push(x as i64);
            x = 0;
            k = 0;
        }
    }
    if k > 0 {
        v.push((x << (16 * (4 - k))) as i64);
    }
    v[np] = n;
}

fn op_session_put(k: &str, blob: &[i64]) -> Vec<i64> {
    let mut v = vec![1];
    key(&mut v, k);
    v.push(blob.len() as i64);
    v.extend_from_slice(blob);
    v
}

fn op_k(code: i64, k: &str) -> Vec<i64> {
    let mut v = vec![code];
    key(&mut v, k);
    v
}

fn op_kk(code: i64, a: &str, b: &str) -> Vec<i64> {
    let mut v = vec![code];
    key(&mut v, a);
    key(&mut v, b);
    v
}

fn run_checked(w: &mut World, op: &[i64], what: &str) {
    match w.exec(op) {
        R::Ok(_) => {}
        R::Invalid => {
            eprintln!("perf: {} rejected as invalid", what);
            std::process::exit(1);
        }
        R::GrowFailed => {
            eprintln!("perf: {} grow failed", what);
            std::process::exit(1);
        }
        R::OutSmall => {
            eprintln!("perf: {} out too small", what);
            std::process::exit(1);
        }
    }
}

fn perf(axis: &str, n: usize, reps: usize) {
    let mut w = World::new(4096);
    let blob: Vec<i64> = vec![11, 22, 33, 44, 55, 66, 77, 88];

    // Prefill (not timed, not counted).
    match axis {
        "sessions" => {
            for i in 0..n {
                let op = op_session_put(&format!("s{}", i), &blob);
                run_checked(&mut w, &op, "prefill session_put");
            }
        }
        "tools" => {
            for i in 0..n {
                let op = op_kk(13, "t0", &format!("a{}", i));
                run_checked(&mut w, &op, "prefill tool_note");
            }
        }
        "clocks" => {
            for i in 0..n {
                let op = op_kk(5, &format!("c{}", i), "s0");
                run_checked(&mut w, &op, "prefill clock_put");
            }
        }
        _ => {
            eprintln!("unknown axis {}", axis);
            std::process::exit(2);
        }
    }
    w.grows = 0;
    w.grow_ns = 0;

    // Build every op vector first; this loop is the "encode" time.
    let t_enc = Instant::now();
    let mut ops: Vec<Vec<i64>> = Vec::with_capacity(reps * 3);
    match axis {
        "sessions" => {
            let k = format!("s{}", n / 2);
            for _ in 0..reps {
                ops.push(op_k(2, &k));
                ops.push(op_session_put(&k, &blob));
            }
        }
        "tools" => {
            // Per rep: note an already-seen activity, note a new activity (fresh turn "x"), forget that turn.
            // The prefilled turn keeps exactly n activities throughout.
            for r in 0..reps {
                ops.push(op_kk(13, "t0", &format!("a{}", r % n.max(1))));
                ops.push(op_kk(13, "x", "y"));
                ops.push(op_k(14, "x"));
            }
        }
        _ => {
            // clocks: run_set / run_del on one clock.
            let c = format!("c{}", n / 2);
            for r in 0..reps {
                let rk = format!("r{}", r % 4);
                let mut set = vec![9];
                key(&mut set, &c);
                key(&mut set, &rk);
                set.push(1);
                ops.push(set);
                ops.push(op_kk(10, &c, &rk));
            }
        }
    }
    let enc_ns = t_enc.elapsed().as_nanos();
    let nops = ops.len().max(1);

    // The op sequence repeats with period `per` reps (tools: r % n, clocks: r % 4, sessions: 1), so the timed loop
    // replays the first period's op vectors in the same order. Like the TS loop, which reuses a few prebuilt key
    // strings, it then reads ops from cache instead of streaming the whole encoded list from memory.
    let per = match axis {
        "tools" => n.max(1),
        "clocks" => 4,
        _ => 1,
    }
    .min(reps.max(1));
    let width = ops.len() / reps.max(1);
    ops.truncate(per * width);
    let cycle = ops;

    let t0 = Instant::now();
    let mut sink = 0i64;
    let mut j = 0usize;
    for _ in 0..nops {
        let op = &cycle[j];
        j += 1;
        if j == cycle.len() {
            j = 0;
        }
        match w.exec(black_box(op)) {
            R::Ok(k) => sink = sink.wrapping_add(black_box(w.out[0] + k as i64)),
            _ => {
                eprintln!("perf: op failed in timed loop");
                std::process::exit(1);
            }
        }
    }
    let total = t0.elapsed().as_nanos();
    black_box(sink);
    let core = total.saturating_sub(w.grow_ns);
    println!(
        "axis={} n={} encode_ns_per_op={:.1} core_ns_per_op={:.1} grow_ns_total={} grows={} ops={}",
        axis,
        n,
        enc_ns as f64 / nops as f64,
        core as f64 / nops as f64,
        w.grow_ns,
        w.grows,
        nops
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("q") => q_mode(),
        Some("perf") => {
            if args.len() < 5 {
                eprintln!("usage: driver_m4 perf <axis> <n> <reps>");
                std::process::exit(2);
            }
            let n: usize = args[3].parse().unwrap();
            let reps: usize = args[4].parse().unwrap();
            perf(&args[2], n, reps);
        }
        _ => {
            eprintln!("usage: driver_m4 q | perf <axis> <n> <reps>");
            std::process::exit(2);
        }
    }
}

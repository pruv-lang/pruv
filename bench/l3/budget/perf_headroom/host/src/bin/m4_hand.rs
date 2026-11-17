// Hand-written idiomatic Rust baseline for the M4 perf axes.
//
// Mirrors: `exec()` in bench/l3/budget/m34_test/perf_ts.mjs (the spec for validation and result words) and the timed
// loop of `perf()` in bench/l3/budget/m34_test/driver_m4.rs (cyclic replay of one op period, black_box on op and sink).
//
// Measured scope: decode + validate the op words, build the keys from the units (into reusable scratch buffers, no
// caching between calls), apply the map/set operation, write the result words into `out`. Prefill and op encoding
// are untimed.
//
//   m4_hand <sessions|tools|clocks> ... is invoked as:  m4_hand perf <axis> <n> <reps>
//   m4_hand selftest
// Prints one line: axis=.. n=.. hand_ns_per_op=..

use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::time::Instant;

const OUT_WORDS: usize = 4096;

type Key = Box<[u16]>;

#[allow(dead_code)]
struct Clock {
    sid: Key,
    since_present: i64,
    since: i64,
    armed: i64,
    deadline: i64,
    runs: HashMap<Key, i64>,
}

#[derive(Default)]
struct State {
    sessions: HashMap<Key, Vec<i64>>,
    clocks: HashMap<Key, Clock>,
    tool_seen: HashMap<Key, HashSet<Key>>,
    // Scratch buffers, one per key position.
    k1: Vec<u16>,
    k2: Vec<u16>,
}

/// Decodes the key `len, units...` at `p` into `buf`. Returns the end position or None if invalid.
fn decode_key(op: &[i64], p: usize, buf: &mut Vec<u16>) -> Option<usize> {
    let n = op.len();
    if p >= n {
        return None;
    }
    let kn = op[p];
    if kn < 0 || kn > (n - p - 1) as i64 {
        return None;
    }
    let kn = kn as usize;
    buf.clear();
    for &u in &op[p + 1..p + 1 + kn] {
        if !(0..=65535).contains(&u) {
            return None;
        }
        buf.push(u as u16);
    }
    Some(p + 1 + kn)
}

fn boxed(k: &[u16]) -> Key {
    k.to_vec().into_boxed_slice()
}

impl State {
    /// Returns 0 ok, 2 invalid, 3 out too small.
    fn exec(&mut self, op: &[i64], out: &mut [i64]) -> i32 {
        let Some(&code) = op.first() else { return 2 };
        match code {
            1 => {
                let Some(e1) = decode_key(op, 1, &mut self.k1) else { return 2 };
                if e1 >= op.len() {
                    return 2;
                }
                let cnt = op[e1];
                if !(0..=65535).contains(&cnt) || cnt != (op.len() - e1 - 1) as i64 {
                    return 2;
                }
                let blob = &op[e1 + 1..];
                let existed = if let Some(v) = self.sessions.get_mut(&self.k1[..]) {
                    v.clear();
                    v.extend_from_slice(blob);
                    true
                } else {
                    self.sessions.insert(boxed(&self.k1), blob.to_vec());
                    false
                };
                out[0] = 1;
                out[1] = if existed { 0 } else { 1 };
                0
            }
            2 | 14 => {
                let Some(e1) = decode_key(op, 1, &mut self.k1) else { return 2 };
                if e1 != op.len() {
                    return 2;
                }
                if code == 2 {
                    match self.sessions.get(&self.k1[..]) {
                        None => {
                            out[0] = 1;
                            out[1] = 0;
                        }
                        Some(b) => {
                            let cnt = b.len();
                            if 3 + cnt > out.len() {
                                return 3;
                            }
                            out[0] = 2 + cnt as i64;
                            out[1] = 1;
                            out[2] = cnt as i64;
                            out[3..3 + cnt].copy_from_slice(b);
                        }
                    }
                } else {
                    out[0] = 1;
                    out[1] = self.tool_seen.remove(&self.k1[..]).is_some() as i64;
                }
                0
            }
            5 | 9 | 10 | 13 => {
                let Some(e1) = decode_key(op, 1, &mut self.k1) else { return 2 };
                let Some(e2) = decode_key(op, e1, &mut self.k2) else { return 2 };
                if code == 9 {
                    if op.len() - e2 != 1 {
                        return 2;
                    }
                    let w = op[e2];
                    if w != 0 && w != 1 {
                        return 2;
                    }
                    out[0] = 1;
                    match self.clocks.get_mut(&self.k1[..]) {
                        None => out[1] = 2,
                        Some(c) => {
                            if let Some(slot) = c.runs.get_mut(&self.k2[..]) {
                                *slot = w;
                                out[1] = 0;
                            } else {
                                c.runs.insert(boxed(&self.k2), w);
                                out[1] = 1;
                            }
                        }
                    }
                    return 0;
                }
                if e2 != op.len() {
                    return 2;
                }
                out[0] = 1;
                match code {
                    5 => match self.clocks.get_mut(&self.k1[..]) {
                        None => {
                            let c = Clock {
                                sid: boxed(&self.k2),
                                since_present: 0,
                                since: 0,
                                armed: 0,
                                deadline: 0,
                                runs: HashMap::new(),
                            };
                            self.clocks.insert(boxed(&self.k1), c);
                            out[1] = 1;
                        }
                        Some(c) => {
                            if c.sid[..] != self.k2[..] {
                                c.sid = boxed(&self.k2);
                            }
                            out[1] = 0;
                        }
                    },
                    10 => {
                        out[1] = match self.clocks.get_mut(&self.k1[..]) {
                            Some(c) => c.runs.remove(&self.k2[..]).is_some() as i64,
                            None => 0,
                        };
                    }
                    _ => {
                        if !self.tool_seen.contains_key(&self.k1[..]) {
                            self.tool_seen.insert(boxed(&self.k1), HashSet::new());
                        }
                        let s = self.tool_seen.get_mut(&self.k1[..]).unwrap();
                        if s.contains(&self.k2[..]) {
                            out[1] = 0;
                        } else {
                            s.insert(boxed(&self.k2));
                            out[1] = 1;
                        }
                    }
                }
                0
            }
            _ => 2,
        }
    }
}

// ---- op encoding (untimed) ----------------------------------------------------------------------------------------

fn key(v: &mut Vec<i64>, s: &str) {
    v.push(s.len() as i64);
    v.extend(s.bytes().map(|b| b as i64));
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

fn op_run_set(c: &str, rk: &str, w: i64) -> Vec<i64> {
    let mut v = vec![9];
    key(&mut v, c);
    key(&mut v, rk);
    v.push(w);
    v
}

// ---- perf ---------------------------------------------------------------------------------------------------------

fn run_check(st: &mut State, op: &[i64], out: &mut [i64], what: &str) {
    let s = st.exec(op, out);
    if s != 0 {
        eprintln!("perf: {} failed with status {}", what, s);
        std::process::exit(1);
    }
}

/// One full run on fresh state. Returns (elapsed ns, ops, sink).
fn run_once(axis: &str, n: usize, reps: usize) -> (u128, usize, i64) {
    let mut st = State::default();
    let mut out: Vec<i64> = vec![0; OUT_WORDS];
    let blob: Vec<i64> = vec![11, 22, 33, 44, 55, 66, 77, 88];

    // Prefill (not timed).
    match axis {
        "sessions" => {
            for i in 0..n {
                run_check(&mut st, &op_session_put(&format!("s{}", i), &blob), &mut out, "prefill session_put");
            }
        }
        "tools" => {
            for i in 0..n {
                run_check(&mut st, &op_kk(13, "t0", &format!("a{}", i)), &mut out, "prefill tool_note");
            }
        }
        "clocks" => {
            for i in 0..n {
                run_check(&mut st, &op_kk(5, &format!("c{}", i), "s0"), &mut out, "prefill clock_put");
            }
        }
        _ => {
            eprintln!("unknown axis {}", axis);
            std::process::exit(2);
        }
    }

    // One period of the op cycle (not timed).
    let (per, width) = match axis {
        "sessions" => (1, 2),
        "tools" => (n, 3),
        _ => (4, 2),
    };
    let per = per.min(reps);
    let mut cycle: Vec<Vec<i64>> = Vec::with_capacity(per * width);
    match axis {
        "sessions" => {
            let k = format!("s{}", n / 2);
            for _ in 0..per {
                cycle.push(op_k(2, &k));
                cycle.push(op_session_put(&k, &blob));
            }
        }
        "tools" => {
            for r in 0..per {
                cycle.push(op_kk(13, "t0", &format!("a{}", r % n)));
                cycle.push(op_kk(13, "x", "y"));
                cycle.push(op_k(14, "x"));
            }
        }
        _ => {
            let c = format!("c{}", n / 2);
            for r in 0..per {
                let rk = format!("r{}", r % 4);
                cycle.push(op_run_set(&c, &rk, 1));
                cycle.push(op_kk(10, &c, &rk));
            }
        }
    }
    let nops = reps * width;
    let clen = cycle.len();

    let t0 = Instant::now();
    let mut sink = 0i64;
    let mut j = 0usize;
    for _ in 0..nops {
        let op = &cycle[j];
        j += 1;
        if j == clen {
            j = 0;
        }
        // Same per-op shape as driver_m4 World::exec: an output capacity check, the call, a black_box on the result.
        if out.len() < OUT_WORDS {
            out.resize(OUT_WORDS, 0);
        }
        let s = st.exec(black_box(op), &mut out);
        if s != 0 {
            eprintln!("perf: op failed in timed loop (status {})", s);
            std::process::exit(1);
        }
        sink = sink.wrapping_add(black_box(out[0] + out[1]));
    }
    let ns = t0.elapsed().as_nanos();
    black_box(sink);
    (ns, nops, sink)
}

// ---- selftest -----------------------------------------------------------------------------------------------------

fn selftest() {
    let mut st = State::default();
    let mut out: Vec<i64> = vec![0; OUT_WORDS];
    let steps: Vec<(&str, Vec<i64>, i32, Option<Vec<i64>>)> = vec![
        ("session_put new", op_session_put("ab", &[5, 6, 7]), 0, Some(vec![1])),
        ("session_get", op_k(2, "ab"), 0, Some(vec![1, 3, 5, 6, 7])),
        ("session_put replace", op_session_put("ab", &[9]), 0, Some(vec![0])),
        ("session_get after replace", op_k(2, "ab"), 0, Some(vec![1, 1, 9])),
        ("session_get absent", op_k(2, "zz"), 0, Some(vec![0])),
        ("session_put empty blob", op_session_put("e", &[]), 0, Some(vec![1])),
        ("session_get empty blob", op_k(2, "e"), 0, Some(vec![1, 0])),
        ("run_set without clock", op_run_set("c1", "r1", 1), 0, Some(vec![2])),
        ("clock_put new", op_kk(5, "c1", "s1"), 0, Some(vec![1])),
        ("clock_put existing", op_kk(5, "c1", "s2"), 0, Some(vec![0])),
        ("run_set new", op_run_set("c1", "r1", 1), 0, Some(vec![1])),
        ("run_set again", op_run_set("c1", "r1", 0), 0, Some(vec![0])),
        ("run_del present", op_kk(10, "c1", "r1"), 0, Some(vec![1])),
        ("run_del absent", op_kk(10, "c1", "r1"), 0, Some(vec![0])),
        ("run_del no clock", op_kk(10, "c9", "r1"), 0, Some(vec![0])),
        ("tool_note new", op_kk(13, "t", "a"), 0, Some(vec![1])),
        ("tool_note seen", op_kk(13, "t", "a"), 0, Some(vec![0])),
        ("tool_note second activity", op_kk(13, "t", "b"), 0, Some(vec![1])),
        ("tool_forget present", op_k(14, "t"), 0, Some(vec![1])),
        ("tool_forget absent", op_k(14, "t"), 0, Some(vec![0])),
        ("tool_note after forget", op_kk(13, "t", "a"), 0, Some(vec![1])),
        ("key unit 70000", vec![2, 1, 70000], 2, None),
        ("key unit -1", vec![2, 1, -1], 2, None),
        ("key length beyond op", vec![2, 1, 97, 5], 2, None),
        ("wrong length: trailing word", vec![2, 1, 97, 7], 2, None),
        ("wrong length: tool_note trailing", vec![13, 1, 97, 1, 98, 7], 2, None),
        ("wrong length: tool_note missing key", vec![13, 1, 97], 2, None),
        ("run_set waiting 2", vec![9, 1, 97, 1, 98, 2], 2, None),
        ("run_set missing waiting", vec![9, 1, 97, 1, 98], 2, None),
        ("session_put n mismatch", vec![1, 1, 97, 3, 1], 2, None),
        ("session_put missing n", vec![1, 1, 97], 2, None),
        ("unknown code", vec![99], 2, None),
        ("empty op", vec![], 2, None),
    ];
    for (what, op, want_status, want_res) in &steps {
        out.fill(0);
        let s = st.exec(op, &mut out);
        if s != *want_status {
            println!("selftest mismatch: {}: status {}, expected {}", what, s, want_status);
            std::process::exit(1);
        }
        if let Some(want) = want_res {
            let cnt = out[0];
            let got: Vec<i64> = if cnt >= 0 && (cnt as usize) < out.len() {
                out[1..1 + cnt as usize].to_vec()
            } else {
                Vec::new()
            };
            if cnt != want.len() as i64 || &got != want {
                println!(
                    "selftest mismatch: {}: result {:?} (out[0]={}), expected {:?}",
                    what, got, cnt, want
                );
                std::process::exit(1);
            }
        }
    }
    println!("selftest ok");
}

fn usage() -> ! {
    eprintln!("usage: m4_hand perf <sessions|tools|clocks> <n> <reps> | selftest");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("selftest") if args.len() == 2 => selftest(),
        Some("perf") if args.len() == 5 => {
            let axis = args[2].as_str();
            if !matches!(axis, "sessions" | "tools" | "clocks") {
                usage();
            }
            let n: usize = args[3].parse().unwrap_or(0);
            let reps: usize = args[4].parse().unwrap_or(0);
            if n == 0 || reps == 0 {
                usage();
            }
            // One measured run on fresh state, no separate warm-up (driver_m4 perf has none either).
            let (ns, ops, sink) = run_once(axis, n, reps);
            eprintln!("sink={}", sink);
            println!("axis={} n={} hand_ns_per_op={:.1}", axis, n, ns as f64 / ops as f64);
        }
        _ => usage(),
    }
}

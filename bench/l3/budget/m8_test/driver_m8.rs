// M8 driver (appended to a main file that already declares `mod sol;`, see run_m8.py).
//
//   driver_m8 q              program on stdin, one case after another:
//                              case <id> | put <words...> | step <i> | call <op words...> | end
//                              | poke <k> <p1> ... <pk> <delta> <value>
//                              poke corrupts the resident buffer for tests (see corrupt_resident.py): the first index i
//                              with st[i..i+k] == [p1..pk] gets st[i+delta] = value (when in range); prints `Z <i>`, or
//                              `Z -` when the pattern is absent. It calls no sol:: function.
//                              | rekey <k> <p1> ... <pk> <delta> <value>
//                              rekey is poke on a STORED MAP KEY word that keeps the index consistent, so a lookup with
//                              the new key really takes the hit path: it finds the live entry whose packed key words
//                              cover st[i+delta], checks that the stored hash (field 6) equals the bud_keys::hash mirror
//                              below for the OLD words, writes the word, stores the hash of the NEW words in field 6 and
//                              forces one grow (t_grow rebuilds the slot table from field 6). Prints `Z <i>`, or `Z -`
//                              when the pattern is absent, no live entry covers the word or the hash mirror disagrees.
//   driver_m8 perf <N> <reps> perf items, see run_perf below
//
// Boundary rule: the host owns the state buffer and the scratch frames but never reads state words and decides
// nothing. Per call it prints what Pruv returned: `R <charge status>` (fx[1]) or `R -` for the void methods. On status 6
// (a HALT dispatch the root does not implement yet, M10) it prints `H <halt frame words>` and stops the case: the halt
// sub-operation would run next and nothing after it is executed or filled. Any other non-zero status prints
// `X <status>` and stops the case. A put the root rejects (malformed input, status 2) prints `P X <status>`, leaves the
// state unchanged and the case continues (perf-mode puts stay fatal). perf also prints `chk <item> <size> <value>`
// after each item's timed loop: the t_observe words folded as s = s*31 + w (wrapping u64). Grow: status -N -> buffer of exactly N (Pruv sizes it, N >= 2*len), t_grow, retry.
// The state buffer is the product host's StateBuf (bench/l3/budget/state_buf.rs, included by run_m8.py in front of
// this file): large buffers are mapped pages, returned at once on drop (L3 P13c).
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

struct World {
    st: StateBuf,
    out: Vec<i64>,
    cv: Vec<i64>,
    fx: Vec<i64>,
}

impl World {
    fn new() -> World {
        let mut st = StateBuf::zeroed_or_abort(256);
        let r = sol::t_init(&mut st);
        if r > 0 {
            st = StateBuf::zeroed_or_abort(r as usize);
            if sol::t_init(&mut st) != 0 {
                eprintln!("t_init failed");
                std::process::exit(2);
            }
        }
        World { st, out: vec![0; 256], cv: vec![0; 16], fx: vec![0; 256] }
    }

    // Test-only (rekey line): st[t] = value where t is a packed key word of a live map entry, keeping the stored hash
    // and the slot table consistent with the new key. Reads the bud_store layout (header word 5 entry base, 6 entry
    // cap, 7 top; entry = 20 words: 0 live, 1 owner, 2 owner generation, 4 key offset, 5 unit count, 6 hash), which
    // is the same in every revision corrupt_resident.py compares. None when no live entry covers t or the stored hash
    // differs from the mirror (the mirror would then be wrong, a control error).
    fn rekey(&mut self, t: i64, value: i64) -> Option<()> {
        let st = &self.st;
        if st.len() < 16 || t < 0 || t as usize >= st.len() {
            return None;
        }
        let base = st[5];
        let top = st[7].min(st[6]);
        for e in 0..top.max(0) {
            let b = base + 20 * e;
            if b < 16 || (b + 20) as usize > st.len() {
                return None;
            }
            let b = b as usize;
            if st[b] != 1 {
                continue;
            }
            let (owner, ogen, koff, n) = (st[b + 1], st[b + 2], st[b + 4], st[b + 5]);
            let m = (n.clamp(0, 1 << 30) + 3) >> 2;
            if t < koff || t >= koff + m {
                continue;
            }
            let seed = key_mix(1469598103934665603, owner.wrapping_add(ogen.wrapping_mul(4294967296)));
            if key_hash(seed, st, koff, n) != st[b + 6] {
                return None;
            }
            self.st[t as usize] = value;
            self.st[b + 6] = key_hash(seed, &self.st, koff, n);
            let l = self.st.len();
            self.grow(l);
            return Some(());
        }
        None
    }

    fn grow(&mut self, n: usize) {
        let want = n; // N from Pruv (bud_store::need, N >= 2*len): the host makes no sizing choice (L3 P13b)
        let mut fresh = StateBuf::zeroed_or_abort(want);
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

    // t_observe into `out`; on status 3 (out too short) the buffer doubles and observe runs again, like the growth of
    // the Vec a hand observe builds (no host size guess from the state length).
    fn observe_words(&mut self) -> i64 {
        loop {
            let r = sol::t_observe(&self.st, &mut self.out);
            if r != 3 {
                return r;
            }
            let l = self.out.len();
            self.out.resize(2 * l, 0);
        }
    }

    // Folds the full observed final state (the t_observe words, same words as q mode prints after `O`) into a u64.
    // Used by perf after a timed loop, outside the timing.
    fn checksum(&mut self) -> u64 {
        let r = self.observe_words();
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
        let r = self.observe_words();
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
            "rekey" => {
                let k: usize = p[1].parse().unwrap();
                let pat = nums(&p[2..2 + k]);
                let delta: i64 = p[2 + k].parse().unwrap();
                let value: i64 = p[3 + k].parse().unwrap();
                let found = if k == 0 || w.st.len() < k {
                    None
                } else {
                    (0..=w.st.len() - k).find(|&i| w.st[i..i + k] == pat[..])
                };
                match found.and_then(|i| w.rekey(i as i64 + delta, value).map(|_| i)) {
                    Some(i) => writeln!(o, "Z {}", i).unwrap(),
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

// Test-only mirror of bud_keys::hash and bud_map::hash_words (rekey line only): seed = mix(FNV basis,
// owner + ogen * 2^32); mix each of the ceil(n/4) packed words (0 past the state end); fin(h ^ n).
fn key_mix(h: i64, v: i64) -> i64 {
    (h ^ v).wrapping_mul(-7046029254386353131)
}

fn key_fin(h: i64) -> i64 {
    let a = h ^ ((h as u64) >> 33) as i64;
    let b = a.wrapping_mul(-49064778989728563);
    b ^ ((b as u64) >> 33) as i64
}

fn key_hash(seed: i64, st: &[i64], ks: i64, n: i64) -> i64 {
    let nc = n.clamp(0, 1 << 30);
    let mut h = seed;
    for i in 0..(nc + 3) >> 2 {
        let a = ks + i;
        h = key_mix(h, if a >= 0 && (a as usize) < st.len() { st[a as usize] } else { 0 });
    }
    key_fin(h ^ nc)
}

// Appends `s` as a wire key `n, packed words(n)` (1 + ceil(n/4) words; bud_keys.pruv codec: word i = (u[4i] << 48) |
// (u[4i+1] << 32) | (u[4i+2] << 16) | u[4i+3] as a two's-complement i64, lanes past n zero), with no intermediate
// buffer.
fn push_key(v: &mut Vec<i64>, s: &str) {
    let at = v.len();
    v.push(0);
    let (mut n, mut x, mut l) = (0i64, 0u64, 0u32);
    for u in s.encode_utf16() {
        x = (x << 16) | u as u64;
        n += 1;
        l += 1;
        if l == 4 {
            v.push(x as i64);
            x = 0;
            l = 0;
        }
    }
    if l > 0 {
        v.push((x << (16 * (4 - l))) as i64);
    }
    v[at] = n;
}

// Perf text keys, formatted into one reused String.
struct Text(String);

impl Text {
    fn session(&mut self, i: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "session-{:05}", i);
        &self.0
    }
    fn bid(&mut self, i: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "99b568f7-0000-4000-8000-{:012}", i);
        &self.0
    }
    fn activity(&mut self, j: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "activity-{:05}", j);
        &self.0
    }
}

// Perf op frames, all in one word buffer: op i is w[at[i]..at[i + 1]].
struct Ops {
    w: Vec<i64>,
    at: Vec<usize>,
}

impl Ops {
    fn new(n: usize, words: usize) -> Ops {
        let mut at = Vec::with_capacity(n + 1);
        at.push(0);
        Ops { w: Vec::with_capacity(n * words), at }
    }
    fn end(&mut self) {
        self.at.push(self.w.len());
    }
    #[inline]
    fn get(&self, i: usize) -> &[i64] {
        &self.w[self.at[i]..self.at[i + 1]]
    }
    fn len(&self) -> usize {
        self.at.len() - 1
    }
}

fn push_ref(v: &mut Vec<i64>, t: &mut Text, i: usize) {
    push_key(v, t.session(i));
    push_key(v, t.bid(i));
}

// N sessions `session-<i>` with an open budget (limits 10^12, so charges never reach a limit).
fn world_n(n: usize, t: &mut Text) -> World {
    let mut w = World::new();
    let mut put: Vec<i64> = Vec::with_capacity(64);
    for i in 0..n {
        put.clear();
        push_key(&mut put, t.session(i));
        let rl = put.len();
        put.push(0);
        put.extend_from_slice(&[0i64, 0, -1]);
        put.extend_from_slice(&[1_000_000_000_000i64; 4]);
        put.extend_from_slice(&[0i64; 7]);
        push_key(&mut put, t.bid(i)); // [bn] + packed budgetId words, so the record is 14 + 1 + ceil(bn/4) words
        put[rl] = (put.len() - rl - 1) as i64;
        let r = w.put(&put);
        if r != 0 {
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
    }
    w
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
    let mut t = Text(String::with_capacity(64));
    let mut w = world_n(n, &mut t);
    let mut ops = Ops::new(n, 32);
    for i in 0..n {
        ops.w.push(1);
        push_ref(&mut ops.w, &mut t, i);
        ops.w.extend_from_slice(&[0, 0, 2, 1, 0, 0, 0, 0, 2, 1]);
        ops.end();
    }
    for i in 0..1000 {
        w.call(ops.get(i % n));
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(ops.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("charge {} {:.1}", n, ns);
        println!("chk charge {} {}", n, w.checksum()); // outside the timing
    }
    let mut fo = Ops::new(n, 20);
    for i in 0..n {
        fo.w.push(2);
        push_ref(&mut fo.w, &mut t, i);
        fo.end();
    }
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(fo.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("failure {} {:.1}", n, ns);
        println!("chk failure {} {}", n, w.checksum());
    }
    let tools = |turn: &str, t: &mut Text| -> Ops {
        let mut o = Ops::new(n, 32);
        for j in 0..n {
            o.w.push(3);
            push_ref(&mut o.w, t, 0);
            push_key(&mut o.w, turn);
            push_key(&mut o.w, t.activity(j));
            o.end();
        }
        o
    };
    let dup = tools("turn-1", &mut t);
    for j in 0..dup.len() {
        w.call(dup.get(j));
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(dup.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("tooldup {} {:.1}", n, ns);
        println!("chk tooldup {} {}", n, w.checksum());
    }
    let grow = tools("turn-2", &mut t);
    let mut forget = vec![4];
    push_key(&mut forget, "turn-2");
    for j in 0..grow.len() {
        w.call(grow.get(j));
    }
    w.call(&forget);
    if on("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let t = Instant::now();
        for _ in 0..rounds {
            for j in 0..grow.len() {
                black_box(w.call(black_box(grow.get(j))));
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

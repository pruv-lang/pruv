// M4 driver (appended to a main file that already declares `mod sol;`, see run.py).
//
//   driver_m4 q                      program on stdin: `init <L>` | `op <w...>` | `observe` (op keys already packed)
//   driver_m4 perf <axis> <n> <reps> axis = sessions | tools | clocks
//   driver_m4 bound state|words      length-bound probe (see bound_probe)
//
// Boundary rule: the host owns the state buffer `st` but never reads or writes its words. It allocates, calls the
// entries, and on status 1 (Grow, N = out[0]) allocates a zeroed buffer of exactly N (sized in Pruv, N >= 2*len), calls m4_grow, swaps the
// buffers and retries the same op (at most 3 retries). State buffers are the product host's StateBuf (bench/l3/budget/
// state_buf.rs, included by run.py in front of this file): large ones are mapped pages, returned at once on drop (L3 P13c).
//
// Length bound (L3 P13f): every slice this driver hands an entry is at most 2^40 words (STATE_MAX_WORDS), the bound
// each checked entry tests and the host owes a raw_entries = "crate" build (SPEC "Length bound at host boundaries").
// The state is a StateBuf (zeroed refuses longer), ops and the out buffer are Words (below, every constructor and resize
// checks), and the q-mode observe / import buffers are capped. The bound is paid where a buffer is made or resized, so the
// entry's own test on a held buffer folds away and the timed loop pays no length test.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// Allocation count (L3 P13b memory gate): every alloc / alloc_zeroed / realloc of the whole process plus every mapped
// state buffer (STATE_MAPS, one per buffer like an allocator call), printed as allocs=N on the perf line; perf_headroom/host/run.py compares it with the other side's count (same accounting).
struct CountingAlloc;
static ALLOC_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        std::alloc::System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;

// An owned op or output buffer of at most STATE_MAX_WORDS words (L3 P13f). new / zeroed / resize check the bound
// before allocating, and nothing else changes the length, so Deref / DerefMut can tell the optimizer: an entry's length
// test on a Words argument folds away. In perf mode every cycle op is made once, before the timed loop; in q mode once
// per input line. Over the bound the process exits like a generated entry (status 101, same message).
struct Words(Vec<i64>);

#[cold]
#[inline(never)]
fn words_too_long(n: usize) -> ! {
    eprintln!("pruv: invalid representation: {} (array length)", n);
    std::process::exit(101);
}

impl Words {
    fn new(v: Vec<i64>) -> Words {
        if v.len() > STATE_MAX_WORDS {
            words_too_long(v.len());
        }
        Words(v)
    }
    fn zeroed(n: usize) -> Words {
        if n > STATE_MAX_WORDS {
            words_too_long(n);
        }
        Words(vec![0; n])
    }
    fn resize(&mut self, n: usize, v: i64) {
        if n > STATE_MAX_WORDS {
            words_too_long(n);
        }
        self.0.resize(n, v);
    }
}

impl std::ops::Deref for Words {
    type Target = [i64];
    #[inline(always)]
    fn deref(&self) -> &[i64] {
        // SAFETY: new / zeroed / resize keep len <= STATE_MAX_WORDS and are the only ways to set it.
        unsafe { std::hint::assert_unchecked(self.0.len() <= STATE_MAX_WORDS) };
        &self.0
    }
}

impl std::ops::DerefMut for Words {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [i64] {
        // SAFETY: as in deref.
        unsafe { std::hint::assert_unchecked(self.0.len() <= STATE_MAX_WORDS) };
        &mut self.0
    }
}

// LANE-WORLD-BEGIN (run.py replaces everything up to LANE-WORLD-END with driver_m4c.rs for the m4c lane)
// Every call on the resident state goes through a store with the PruvStore API (`new`, `with`). run.py puts one line
// in front of this file, `type Store<T> = ...;`: `Plain<T>` (below, no poison flag; default build "m4") or
// `sol::pruv_raw::PruvStore<T>` (manifest raw_entries = "crate", tag "m4raw"). The driver code is the same for both.
#[derive(Debug)]
struct PlainNever {}

struct Plain<T> {
    data: T,
}

impl<T> Plain<T> {
    #[inline(always)]
    fn new(data: T) -> Self {
        Plain { data }
    }
    #[inline(always)]
    #[allow(dead_code)]
    fn is_poisoned(&self) -> bool {
        false
    }
    #[inline(always)]
    fn with<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> Result<R, PlainNever> {
        Ok(f(&mut self.data))
    }
}

type St = Store<StateBuf>;

// One state call. A poisoned store cannot happen in this driver (no panic is caught); exit loudly if it does.
#[inline(always)]
fn call<R>(st: &mut St, f: impl FnOnce(&mut StateBuf) -> R) -> R {
    match st.with(f) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("resident state poisoned");
            std::process::exit(3);
        }
    }
}

enum R {
    Ok(usize),
    Invalid,
    GrowFailed,
    OutSmall,
}

struct World {
    st: St,
    len: usize, // len(st), kept by the host (the host never reads the words)
    out: Words, // len <= STATE_MAX_WORDS (Words)
    grows: u64,
    grow_ns: u128,
}

impl World {
    fn new(l: usize) -> World {
        let mut st: St = Store::new(StateBuf::zeroed_or_abort(l));
        let mut len = l;
        let r = call(&mut st, |s| sol::m4_init(s));
        if r > 0 {
            len = r as usize;
            let r2 = call(&mut st, |s| {
                *s = StateBuf::zeroed_or_abort(len);
                sol::m4_init(s)
            });
            if r2 != 0 {
                eprintln!("m4_init failed twice (status {})", r2);
                std::process::exit(2);
            }
        } else if r < 0 {
            eprintln!("m4_init returned {}", r);
            std::process::exit(2);
        }
        World { st, len, out: Words::zeroed(0), grows: 0, grow_ns: 0 }
    }

    fn observe(&mut self, out: &mut Vec<i64>) -> i64 {
        call(&mut self.st, |s| sol::m4_observe(s, out))
    }

    // Import admission (m4_import, L3 P13a.10) as an adapter would load a host buffer: the host copies the state into a
    // fresh store, admits it, and only on 0 serves from it. Returns (status of the admitted copy, entries visited,
    // status of a corrupt copy). The admitted copy is then swapped in as the live store, so every later op and the
    // final observation run on imported state (q mode imports right before the program's observe); before the swap
    // its observation must match the old store's. The
    // corrupt copy (entry 0's state word, at st[5], set to 7) is never served from.
    fn import_check(&mut self) -> (i64, i64, String) {
        let buf = call(&mut self.st, |s| s.clone());
        let mut bad_buf = buf.clone();
        let b = bad_buf[5] as usize;
        bad_buf[b] = 7;
        let mut fresh: St = Store::new(buf);
        let mut cnt = vec![0i64; 3];
        let r = call(&mut fresh, |s| sol::m4_import(s, &mut cnt));
        if r == 0 {
            let mut o1 = vec![0i64; std::cmp::max(4096, 4 * self.len).min(STATE_MAX_WORDS)];
            let mut o2 = o1.clone();
            let r1 = self.observe(&mut o1);
            let r2 = call(&mut fresh, |s| sol::m4_observe(s, &mut o2));
            if r1 != r2 || o1 != o2 {
                eprintln!("imported copy observes differently: {} vs {}", r1, r2);
                std::process::exit(2);
            }
            self.st = fresh;
        }
        let mut cs: St = Store::new(bad_buf);
        let mut c2 = vec![0i64; 3];
        let bad = call(&mut cs, |s| sol::m4_import(s, &mut c2));
        (r, cnt[0], bad.to_string())
    }

    // The out buffer starts at 4096 words and doubles on status 3 up to 4·len (L3 P13b; before, it was sized 4·len up
    // front, which made it the largest resident buffer). The retry replays nothing: every bud_world mutator checks
    // len(out) >= 2 before its first write, so with >= 4096 words only the read-only ops (gets, key and run lists)
    // can return 3, and they take st by value. The doubling stops at STATE_MAX_WORDS (L3 P13f).
    fn exec(&mut self, op: &Words) -> R {
        // The op and out slices are taken here, outside the state-call closure: Words::deref states the length bound
        // in exec itself, so the closure (and the entry inlined into it) inlines into exec as before (P13f measured a
        // separate closure function, +5..+20 instructions per op, when the derefs sat inside it).
        let op: &[i64] = op;
        if self.out.len() < 4096 {
            self.out.resize(4096, 0);
        }
        let mut attempt = 0;
        while attempt < 4 {
            let out: &mut [i64] = &mut self.out;
            let s = call(&mut self.st, |st| sol::m4_op(st, op, out));
            match s {
                0 => return R::Ok(self.out[0].max(0) as usize),
                3 if self.out.len() < (4 * self.len).min(STATE_MAX_WORDS) => {
                    let n = (2 * self.out.len()).min(4 * self.len).min(STATE_MAX_WORDS);
                    self.out.resize(n, 0);
                    continue;
                }
                1 => {
                    if attempt == 3 {
                        return R::GrowFailed;
                    }
                    attempt += 1;
                    // An N that does not fit usize (32-bit targets) is refused like an allocation failure.
                    let n = match usize::try_from(self.out[0].max(0)) {
                        Ok(v) => v,
                        Err(_) => return R::GrowFailed,
                    };
                    let t = Instant::now();
                    let want = n; // N from Pruv (bud_store::need, N >= 2*len): the host makes no sizing choice (L3 P13b)
                    // An N that does not grow the buffer (need at its 2^40 cap) or an allocator refusal fails the op
                    // with the old state kept; the allocation is fallible so it cannot abort mid-op.
                    if want <= self.len {
                        return R::GrowFailed;
                    }
                    let mut fresh = match StateBuf::zeroed(want) {
                        Some(v) => v,
                        None => return R::GrowFailed,
                    };
                    let g = call(&mut self.st, |st| sol::m4_grow(st, &mut fresh));
                    if g != 0 {
                        return R::GrowFailed;
                    }
                    call(&mut self.st, |st| *st = fresh);
                    self.len = want;
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
// LANE-WORLD-END

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
    let mut imported: Option<(i64, i64, String)> = None;
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
                imported = None; // a new world is imported afresh before its own observe
                writeln!(w, "init ok").unwrap();
            }
            "op" => {
                let op = Words::new(it.map(|t| t.parse::<i64>().expect("bad word")).collect());
                let wd = world.as_mut().expect("op before init");
                match wd.exec(&op) {
                    R::Ok(k) => {
                        let res: Vec<i64> = wd.result(k).to_vec();
                        if !op.is_empty() && op[0] == 18 {
                            // stats: [live entries, used slots, live arena words, len(st)]; the host knows len(st).
                            if res.len() == 4 && res[3] == wd.len as i64 {
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
                let wd = world.as_mut().expect("observe before init");
                if imported.is_none() {
                    imported = Some(wd.import_check());
                }
                let mut out = vec![0i64; std::cmp::max(4096, 4 * wd.len).min(STATE_MAX_WORDS)];
                let s = wd.observe(&mut out);
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
    if let Some(wd) = &mut world {
        let (r, entries, bad) = match imported.take() {
            Some(t) => t,
            None => wd.import_check(),
        };
        writeln!(w, "import {} {} {}", r, entries, bad).unwrap();
    }
    if let Some(wd) = &world {
        total_grows += wd.grows;
        writeln!(w, "final_len {}", wd.len).unwrap();
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

fn run_checked(w: &mut World, op: &Words, what: &str) {
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
                let op = Words::new(op_session_put(&format!("s{}", i), &blob));
                run_checked(&mut w, &op, "prefill session_put");
            }
        }
        "tools" => {
            for i in 0..n {
                let op = Words::new(op_kk(13, "t0", &format!("a{}", i)));
                run_checked(&mut w, &op, "prefill tool_note");
            }
        }
        "clocks" => {
            for i in 0..n {
                let op = Words::new(op_kk(5, &format!("c{}", i), "s0"));
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

    // The op sequence repeats with period `per` reps (tools: r % n, clocks: r % 4, sessions: 1), so only the first
    // period's op vectors are built (this loop is the "encode" time) and the timed loop replays them in order, like
    // m4_hand's cycle and the TS loop's few prebuilt key strings. Building all reps first and truncating (before
    // L3 P13b) cost ~1M allocations and ~35 MiB of peak RSS that the hand baseline does not pay.
    let per = match axis {
        "tools" => n.max(1),
        "clocks" => 4,
        _ => 1,
    }
    .min(reps.max(1));
    let t_enc = Instant::now();
    // Each cycle op is a Words: its length bound is checked here, once, outside the timed loop (L3 P13f).
    let mut ops: Vec<Words> = Vec::with_capacity(per * 3);
    let mut width = 0usize;
    match axis {
        "sessions" => {
            let k = format!("s{}", n / 2);
            for _ in 0..per {
                ops.push(Words::new(op_k(2, &k)));
                ops.push(Words::new(op_session_put(&k, &blob)));
            }
            width = 2;
        }
        "tools" => {
            // Per rep: note an already-seen activity, note a new activity (fresh turn "x"), forget that turn.
            // The prefilled turn keeps exactly n activities throughout.
            for r in 0..per {
                ops.push(Words::new(op_kk(13, "t0", &format!("a{}", r % n.max(1)))));
                ops.push(Words::new(op_kk(13, "x", "y")));
                ops.push(Words::new(op_k(14, "x")));
            }
            width = 3;
        }
        _ => {
            // clocks: run_set / run_del on one clock.
            let c = format!("c{}", n / 2);
            for r in 0..per {
                let rk = format!("r{}", r % 4);
                let mut set = vec![9];
                key(&mut set, &c);
                key(&mut set, &rk);
                set.push(1);
                ops.push(Words::new(set));
                ops.push(Words::new(op_kk(10, &c, &rk)));
            }
            width = 2;
        }
    }
    let enc_ns = t_enc.elapsed().as_nanos();
    let nops = (reps * width).max(1);
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
            // Same sink as m4_hand: out[0] + out[1] (out always has >= 4096 words).
            R::Ok(_) => sink = sink.wrapping_add(black_box(w.out[0] + w.out[1])),
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
        "axis={} n={} encode_ns_per_op={:.1} core_ns_per_op={:.1} total_ns_per_op={:.1} grow_ns_total={} grows={} ops={} allocs={}",
        axis,
        n,
        enc_ns as f64 / nops as f64,
        core as f64 / nops as f64,
        total as f64 / nops as f64,
        w.grow_ns,
        w.grows,
        nops,
        ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed) + STATE_MAPS.load(std::sync::atomic::Ordering::Relaxed)
    );
}

// Length-bound probe (L3 P13f; run.py m4 runs it on every lane): `bound state` checks that a state buffer one word over
// STATE_MAX_WORDS, and one whose byte size overflows, is refused without allocating (no mapping, no allocator call);
// `bound words` makes an op / out buffer one word over the bound, which must exit 101 before allocating.
fn bound_probe(what: &str) {
    let before = ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed) + STATE_MAPS.load(std::sync::atomic::Ordering::Relaxed);
    match what {
        "state" => {
            let over = StateBuf::zeroed(STATE_MAX_WORDS + 1).is_none() && StateBuf::zeroed(usize::MAX / 8 + 1).is_none();
            let after = ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed) + STATE_MAPS.load(std::sync::atomic::Ordering::Relaxed);
            println!("bound state refused={} allocs={}", over, after - before);
        }
        "words" => {
            let w = Words::zeroed(STATE_MAX_WORDS + 1);
            println!("bound words accepted {}", w.len());
        }
        _ => {
            eprintln!("usage: driver_m4 bound state|words");
            std::process::exit(2);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("q") => q_mode(),
        Some("bound") => bound_probe(args.get(2).map(|s| s.as_str()).unwrap_or("")),
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
            eprintln!("usage: driver_m4 q | perf <axis> <n> <reps> | bound state|words");
            std::process::exit(2);
        }
    }
}

// Hand-written idiomatic Rust baseline for the M4 perf axes.
//
// Mirrors: `exec()` in bench/l3/budget/m34_test/perf_ts.mjs (the spec for validation and result words) and the timed
// loop of `perf()` in bench/l3/budget/m34_test/driver_m4.rs (cyclic replay of one op period, black_box on op and sink).
//
// Measured scope: decode + validate the op words (keys are packed u16 on the wire: O(1) canonical check, then a lane
// unpack), build the keys from the units (into reusable scratch buffers, no caching between calls), apply the map/set operation, write the result words into `out`. Prefill and op encoding
// are untimed.
//
//   m4_hand perf  <axis> <n> <reps>   in-process calls
//   m4_hand perfc <axis> <n> <reps>   same loop, State behind an opaque extern "C" handle (hand_h_*), called through
//                                     black_box'd function pointers: the matched baseline for the m4c lane
//   m4_hand perfcb <axis> <n> <reps>  as perfc, but on the sessions axis each rep's get + put is ONE call of the batch
//                                     entry hand_hb_exec (one catch_unwind, one store access, same per-item checks):
//                                     the matched baseline for the m4cb lane; the other axes are identical to perfc
//   m4_hand selftest                  (also runs every step through the C API, single and batch)
// Prints one line: axis=.. n=.. hand_ns_per_op=..

use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::time::Instant;

// Allocation count (L3 P13b memory gate): every alloc / alloc_zeroed / realloc of the whole process, printed as
// allocs=N on the perf line; perf_headroom/host/run.py compares it with the other side's count (same accounting).
struct CountingAlloc;
static ALLOC_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
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

/// Decodes the packed key `n, ceil(n/4) words` at `p` (packed-u16 wire, bud_keys.pruv: big-endian 16-bit lanes, four
/// per i64 word, zero tail lanes) into the unit scratch `buf`. Returns the end position or None if invalid: n outside
/// 0..=2^30, the words past the end of the op, or a nonzero lane past n in the last word. Every word is 4 valid u16
/// lanes, so there is no per-unit range check.
fn decode_key(op: &[i64], p: usize, buf: &mut Vec<u16>) -> Option<usize> {
    let len = op.len();
    if p >= len {
        return None;
    }
    let kn = op[p];
    if !(0..=(1i64 << 30)).contains(&kn) {
        return None;
    }
    let kn = kn as usize;
    let m = (kn + 3) / 4;
    if m > len - p - 1 {
        return None;
    }
    let words = &op[p + 1..p + 1 + m];
    let r = (kn & 3) as u32;
    if r != 0 && (words[m - 1] as u64) & ((1u64 << (16 * (4 - r))) - 1) != 0 {
        return None;
    }
    buf.clear();
    for &x in words {
        let u = x as u64;
        buf.extend_from_slice(&[(u >> 48) as u16, (u >> 32) as u16, (u >> 16) as u16, u as u16]);
    }
    buf.truncate(kn);
    Some(p + 1 + m)
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

// Wire key `n, packed words(n)`: word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3], zero tail lanes.
fn key(v: &mut Vec<i64>, s: &str) {
    let u: Vec<u16> = s.encode_utf16().collect();
    v.push(u.len() as i64);
    v.extend(u.chunks(4).map(|c| {
        let mut x: u64 = 0;
        for l in 0..4 {
            x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
        }
        x as i64
    }));
}

// A one-unit ASCII key as a packed word (selftest helper).
fn pk1(c: u8) -> i64 {
    (c as i64) << 48
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

// ---- extern "C" handle (the matched baseline for the m4c lane) ----------------------------------------------------
//
// The hand State behind an opaque handle, doing per call the same work as the generated pruv_h_m4_op: handle check,
// poison check, ret check, overlap check, slice conversion, poison set/clear around the call, `*ret = result`.

pub struct HandHandle {
    st: State,
    poisoned: bool,
}

type NewF = extern "C" fn() -> *mut HandHandle;
type FreeF = unsafe extern "C" fn(*mut HandHandle);
type ExecF = unsafe extern "C" fn(*mut HandHandle, *const i64, i64, *mut i64, i64, *mut i64) -> i32;
// batch: (h, n, op table, op lens, out table, out lens, ret[n], done)
type BatchF = unsafe extern "C" fn(
    *mut HandHandle,
    i64,
    *const *const i64,
    *const i64,
    *const *mut i64,
    *const i64,
    *mut i64,
    *mut i64,
) -> i32;

#[derive(Clone, Copy)]
struct CFns {
    new: NewF,
    free: FreeF,
    exec: ExecF,
    batch: BatchF,
}

#[no_mangle]
pub extern "C" fn hand_h_new() -> *mut HandHandle {
    Box::into_raw(Box::new(HandHandle { st: State::default(), poisoned: false }))
}

/// # Safety
/// `h` must be null or a pointer returned by `hand_h_new` that has not been freed.
#[no_mangle]
pub unsafe extern "C" fn hand_h_free(h: *mut HandHandle) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

/// Byte span [p, p + 8*len) with saturating arithmetic (an invalid length is rejected by ro/mt afterwards anyway).
fn span(p: usize, len: i64) -> (usize, usize) {
    (p, p.saturating_add((len.max(0) as usize).saturating_mul(8)))
}

/// Empty spans never overlap.
fn spans_overlap(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1
}

unsafe fn ro<'a>(p: *const i64, len: i64) -> Option<&'a [i64]> {
    if len == 0 {
        return Some(&[]);
    }
    if len < 0
        || len as u64 > (isize::MAX as u64) / 8
        || p.is_null()
        || (p as usize) % std::mem::align_of::<i64>() != 0
        || (p as usize).checked_add(len as usize * 8).is_none()
    {
        return None;
    }
    Some(std::slice::from_raw_parts(p, len as usize))
}

unsafe fn mt<'a>(p: *mut i64, len: i64) -> Option<&'a mut [i64]> {
    if len == 0 {
        return Some(&mut []);
    }
    if len < 0
        || len as u64 > (isize::MAX as u64) / 8
        || p.is_null()
        || (p as usize) % std::mem::align_of::<i64>() != 0
        || (p as usize).checked_add(len as usize * 8).is_none()
    {
        return None;
    }
    Some(std::slice::from_raw_parts_mut(p, len as usize))
}

/// # Safety
/// Pointers must be null or valid for the stated lengths, as for any C entry point.
#[no_mangle]
pub unsafe extern "C" fn hand_h_exec(
    h: *mut HandHandle,
    op: *const i64,
    op_len: i64,
    out: *mut i64,
    out_len: i64,
    ret: *mut i64,
) -> i32 {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> i32 {
        if h.is_null() {
            return 2;
        }
        if (*h).poisoned {
            return 1;
        }
        if ret.is_null()
            || (ret as usize) % std::mem::align_of::<i64>() != 0
            || (ret as usize).checked_add(std::mem::size_of::<i64>()).is_none()
        {
            return 2;
        }
        let s_op = span(op as usize, op_len);
        let s_out = span(out as usize, out_len);
        let s_ret = span(ret as usize, 1);
        let s_h = (h as usize, (h as usize).saturating_add(std::mem::size_of::<HandHandle>()));
        // Pairwise disjoint where at least one side is writable: op is read-only, the other three are written.
        if spans_overlap(s_op, s_out)
            || spans_overlap(s_op, s_ret)
            || spans_overlap(s_op, s_h)
            || spans_overlap(s_out, s_ret)
            || spans_overlap(s_out, s_h)
            || spans_overlap(s_ret, s_h)
        {
            return 2;
        }
        let Some(op_s) = ro(op, op_len) else { return 2 };
        let Some(out_s) = mt(out, out_len) else { return 2 };
        (*h).poisoned = true;
        let status = (*h).st.exec(op_s, out_s);
        (*h).poisoned = false;
        *ret = status as i64;
        0
    }));
    match r {
        Ok(s) => s,
        Err(payload) => {
            std::mem::forget(payload);
            3
        }
    }
}

/// True if `p` is a usable table of `n` elements of T: non-null, aligned, end address does not overflow.
fn table_ok<T>(p: *const T, n: usize) -> bool {
    !p.is_null()
        && (p as usize) % std::mem::align_of::<T>() == 0
        && n.checked_mul(std::mem::size_of::<T>()).and_then(|b| (p as usize).checked_add(b)).is_some()
}

/// The batch entry (mirror of the generated <pkg>__pruv_hb_NAME): runs items 0..n in order inside ONE catch_unwind and
/// ONE store access (one poison set/clear) and stops after the first item whose status is nonzero. `ret[i]` is item i's
/// status (as hand_h_exec's *ret); `*done` = items that ran. The call returns 0 OK, 1 poisoned, 2 bad args (up-front for
/// bad tables / n < 0 / ret, done, h overlap; per item i with `*done = i`), 3 panicked.
///
/// # Safety
/// Pointers must be null or valid for the stated lengths, as for any C entry point.
#[no_mangle]
pub unsafe extern "C" fn hand_hb_exec(
    h: *mut HandHandle,
    n: i64,
    ops: *const *const i64,
    op_lens: *const i64,
    outs: *const *mut i64,
    out_lens: *const i64,
    ret: *mut i64,
    done: *mut i64,
) -> i32 {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> i32 {
        if h.is_null() {
            return 2;
        }
        if (*h).poisoned {
            return 1;
        }
        if n < 0 || (n as u64) > (isize::MAX as u64) / 64 {
            return 2;
        }
        let nu = n as usize;
        if !table_ok(done as *const i64, 1)
            || (nu > 0
                && !(table_ok(ops, nu)
                    && table_ok(op_lens, nu)
                    && table_ok(outs, nu)
                    && table_ok(out_lens, nu)
                    && table_ok(ret as *const i64, nu)))
        {
            return 2;
        }
        let s_ret = span(ret as usize, n);
        let s_done = span(done as usize, 1);
        let s_h = (h as usize, (h as usize).saturating_add(std::mem::size_of::<HandHandle>()));
        if spans_overlap(s_ret, s_done) || spans_overlap(s_ret, s_h) || spans_overlap(s_done, s_h) {
            return 2;
        }
        (*h).poisoned = true;
        for i in 0..nu {
            let op = *ops.add(i);
            let op_len = *op_lens.add(i);
            let out = *outs.add(i);
            let out_len = *out_lens.add(i);
            // Same per-item checks as hand_h_exec: op is read-only, out/ret slot/done/h are written.
            let s_op = span(op as usize, op_len);
            let s_out = span(out as usize, out_len);
            let bad = spans_overlap(s_op, s_out)
                || spans_overlap(s_op, s_ret)
                || spans_overlap(s_op, s_done)
                || spans_overlap(s_op, s_h)
                || spans_overlap(s_out, s_ret)
                || spans_overlap(s_out, s_done)
                || spans_overlap(s_out, s_h);
            let (Some(op_s), Some(out_s)) = (if bad { None } else { ro(op, op_len) }, if bad { None } else { mt(out, out_len) })
            else {
                *done = i as i64;
                (*h).poisoned = false;
                return 2;
            };
            let status = (*h).st.exec(op_s, out_s);
            *ret.add(i) = status as i64;
            if status != 0 {
                *done = (i + 1) as i64;
                (*h).poisoned = false;
                return 0;
            }
        }
        *done = n;
        (*h).poisoned = false;
        0
    }));
    match r {
        Ok(s) => s,
        Err(payload) => {
            std::mem::forget(payload);
            3
        }
    }
}

// ---- perf ---------------------------------------------------------------------------------------------------------

fn run_check(exec: &mut dyn FnMut(&[i64], &mut [i64]) -> i32, op: &[i64], out: &mut [i64], what: &str) {
    let s = exec(op, out);
    if s != 0 {
        eprintln!("perf: {} failed with status {}", what, s);
        std::process::exit(1);
    }
}

/// One full run on fresh state. Returns (elapsed ns, ops, sink). `c`: the State sits behind the opaque extern "C"
/// handle (`perfc`), reached through black_box'd function pointers; otherwise it is called in-process (`perf`).
/// `cb` (`perfcb`, implies `c`): on the sessions axis each rep's [get, put] pair is one hand_hb_exec call.
fn run_once(axis: &str, n: usize, reps: usize, c: bool, cb: bool) -> (u128, usize, i64) {
    let c = c || cb;
    let mut st = State::default();
    let f = CFns {
        new: black_box::<NewF>(hand_h_new),
        free: black_box::<FreeF>(hand_h_free),
        exec: black_box::<ExecF>(hand_h_exec),
        batch: black_box::<BatchF>(hand_hb_exec),
    };
    let h: *mut HandHandle = if c { (f.new)() } else { std::ptr::null_mut() };
    let mut out: Vec<i64> = vec![0; OUT_WORDS];
    let blob: Vec<i64> = vec![11, 22, 33, 44, 55, 66, 77, 88];

    // Prefill (not timed). The closure returns the op status (C mode: handle status, else the status in *ret).
    let mut pre = |op: &[i64], o: &mut [i64]| -> i32 {
        if c {
            let mut ret = 0i64;
            let s = unsafe { (f.exec)(h, op.as_ptr(), op.len() as i64, o.as_mut_ptr(), o.len() as i64, &mut ret) };
            if s != 0 {
                100 + s
            } else {
                ret as i32
            }
        } else {
            st.exec(op, o)
        }
    };
    match axis {
        "sessions" => {
            for i in 0..n {
                run_check(&mut pre, &op_session_put(&format!("s{}", i), &blob), &mut out, "prefill session_put");
            }
        }
        "tools" => {
            for i in 0..n {
                run_check(&mut pre, &op_kk(13, "t0", &format!("a{}", i)), &mut out, "prefill tool_note");
            }
        }
        "clocks" => {
            for i in 0..n {
                run_check(&mut pre, &op_kk(5, &format!("c{}", i), "s0"), &mut out, "prefill clock_put");
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
    // perfcb, sessions axis: one batch call per rep (item 0 -> out, item 1 -> out2), same sink as two single ops.
    let batched = cb && axis == "sessions" && clen == 2;
    if batched {
        let mut out2: Vec<i64> = vec![0; OUT_WORDS];
        for _ in 0..nops / 2 {
            if out.len() < OUT_WORDS {
                out.resize(OUT_WORDS, 0);
            }
            if out2.len() < OUT_WORDS {
                out2.resize(OUT_WORDS, 0);
            }
            let (a, b) = (black_box(&cycle[0]), black_box(&cycle[1]));
            let op_ptrs: [*const i64; 2] = [a.as_ptr(), b.as_ptr()];
            let op_lens: [i64; 2] = [a.len() as i64, b.len() as i64];
            let out_ptrs: [*mut i64; 2] = [out.as_mut_ptr(), out2.as_mut_ptr()];
            let out_lens: [i64; 2] = [out.len() as i64, out2.len() as i64];
            let mut ret = [0i64; 2];
            let mut done = 0i64;
            let s = unsafe {
                (f.batch)(
                    h,
                    2,
                    op_ptrs.as_ptr(),
                    op_lens.as_ptr(),
                    out_ptrs.as_ptr(),
                    out_lens.as_ptr(),
                    ret.as_mut_ptr(),
                    &mut done,
                )
            };
            if s != 0 || done != 2 || ret[0] != 0 || ret[1] != 0 {
                eprintln!("perfcb: batch failed in timed loop (status {}, done {}, ret {:?})", s, done, ret);
                std::process::exit(1);
            }
            sink = sink.wrapping_add(black_box(out[0] + out[1]));
            sink = sink.wrapping_add(black_box(out2[0] + out2[1]));
        }
    }
    for _ in 0..(if batched { 0 } else { nops }) {
        let op = &cycle[j];
        j += 1;
        if j == clen {
            j = 0;
        }
        // Same per-op shape as driver_m4 World::exec: an output capacity check, the call, a black_box on the result.
        if out.len() < OUT_WORDS {
            out.resize(OUT_WORDS, 0);
        }
        if c {
            let op = black_box(op);
            let mut ret = 0i64;
            let s = unsafe { (f.exec)(h, op.as_ptr(), op.len() as i64, out.as_mut_ptr(), out.len() as i64, &mut ret) };
            if s != 0 || ret != 0 {
                eprintln!("perfc: op failed in timed loop (status {}, ret {})", s, ret);
                std::process::exit(1);
            }
        } else {
            let s = st.exec(black_box(op), &mut out);
            if s != 0 {
                eprintln!("perf: op failed in timed loop (status {})", s);
                std::process::exit(1);
            }
        }
        sink = sink.wrapping_add(black_box(out[0] + out[1]));
    }
    let ns = t0.elapsed().as_nanos();
    black_box(sink);
    if c {
        unsafe { (f.free)(h) };
    }
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
        ("bad tail lane", vec![2, 1, pk1(b'a') | (1i64 << 32)], 2, None),
        ("key n negative", vec![2, -1], 2, None),
        ("key n beyond op words", vec![2, 5, pk1(b'a')], 2, None),
        ("key n above 2^30", vec![2, (1i64 << 30) + 1, pk1(b'a')], 2, None),
        ("wrong length: trailing word", vec![2, 1, pk1(b'a'), 7], 2, None),
        ("wrong length: tool_note trailing", vec![13, 1, pk1(b'a'), 1, pk1(b'b'), 7], 2, None),
        ("wrong length: tool_note missing key", vec![13, 1, pk1(b'a')], 2, None),
        ("run_set waiting 2", vec![9, 1, pk1(b'a'), 1, pk1(b'b'), 2], 2, None),
        ("run_set missing waiting", vec![9, 1, pk1(b'a'), 1, pk1(b'b')], 2, None),
        ("session_put n mismatch", vec![1, 1, pk1(b'a'), 3, 1], 2, None),
        ("session_put missing n", vec![1, 1, pk1(b'a')], 2, None),
        ("unknown code", vec![99], 2, None),
        ("empty op", vec![], 2, None),
    ];
    // Checks status and result words; `via` names the path (in-process or C API) in the message.
    let check = |via: &str, what: &str, s: i32, out: &[i64], want_status: i32, want_res: &Option<Vec<i64>>| {
        if s != want_status {
            println!("selftest mismatch ({}): {}: status {}, expected {}", via, what, s, want_status);
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
                    "selftest mismatch ({}): {}: result {:?} (out[0]={}), expected {:?}",
                    via, what, got, cnt, want
                );
                std::process::exit(1);
            }
        }
    };
    for (what, op, want_status, want_res) in &steps {
        out.fill(0);
        let s = st.exec(op, &mut out);
        check("in-process", what, s, &out, *want_status, want_res);
    }

    // The same steps through the extern "C" handle: the status is in *ret, the call itself returns 0.
    let h = hand_h_new();
    let mut cout: Vec<i64> = vec![0; OUT_WORDS];
    for (what, op, want_status, want_res) in &steps {
        cout.fill(0);
        let mut ret = -1i64;
        let s = unsafe { hand_h_exec(h, op.as_ptr(), op.len() as i64, cout.as_mut_ptr(), cout.len() as i64, &mut ret) };
        if s != 0 {
            println!("selftest mismatch (C API): {}: call status {}, expected 0", what, s);
            std::process::exit(1);
        }
        check("C API", what, ret as i32, &cout, *want_status, want_res);
    }

    // C API argument checks.
    let fail = |what: &str, s: i32| {
        if s != 2 {
            println!("selftest mismatch (C API): {}: status {}, expected 2", what, s);
            std::process::exit(1);
        }
    };
    let op = op_k(2, "ab");
    let mut ret = -1i64;
    unsafe {
        fail(
            "null handle",
            hand_h_exec(std::ptr::null_mut(), op.as_ptr(), op.len() as i64, cout.as_mut_ptr(), cout.len() as i64, &mut ret),
        );
        fail(
            "null ret",
            hand_h_exec(h, op.as_ptr(), op.len() as i64, cout.as_mut_ptr(), cout.len() as i64, std::ptr::null_mut()),
        );
        // out overlapping op: both inside one buffer.
        let mut buf: Vec<i64> = vec![0; OUT_WORDS];
        buf[..op.len()].copy_from_slice(&op);
        let p = buf.as_mut_ptr();
        fail("out overlaps op", hand_h_exec(h, p as *const i64, op.len() as i64, p, OUT_WORDS as i64, &mut ret));
        // ret inside the handle object.
        fail(
            "ret inside handle",
            hand_h_exec(h, op.as_ptr(), op.len() as i64, cout.as_mut_ptr(), cout.len() as i64, h as *mut i64),
        );
        // The handle must still work (no poison left behind by the rejected calls).
        cout.fill(0);
        let s = hand_h_exec(h, op.as_ptr(), op.len() as i64, cout.as_mut_ptr(), cout.len() as i64, &mut ret);
        if s != 0 || ret != 0 {
            println!("selftest mismatch (C API): call after rejected calls: status {}, ret {}", s, ret);
            std::process::exit(1);
        }
        hand_h_free(h);

        // Batch entry on a fresh handle: [session_put, session_get] in one call.
        let hb = hand_h_new();
        let put = op_session_put("ab", &[5, 6]);
        let get = op_k(2, "ab");
        let bad_op: Vec<i64> = vec![99];
        let mut o1: Vec<i64> = vec![0; OUT_WORDS];
        let mut o2: Vec<i64> = vec![0; OUT_WORDS];
        let mut rt = [-1i64; 2];
        let mut done = -1i64;
        let bfail = |what: &str, s: i32, want: i32| {
            if s != want {
                println!("selftest mismatch (batch): {}: status {}, expected {}", what, s, want);
                std::process::exit(1);
            }
        };
        {
            let ops: [*const i64; 2] = [put.as_ptr(), get.as_ptr()];
            let lens: [i64; 2] = [put.len() as i64, get.len() as i64];
            let outs: [*mut i64; 2] = [o1.as_mut_ptr(), o2.as_mut_ptr()];
            let olens: [i64; 2] = [OUT_WORDS as i64, OUT_WORDS as i64];
            bfail(
                "put+get",
                hand_hb_exec(hb, 2, ops.as_ptr(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done),
                0,
            );
            if done != 2 || rt != [0, 0] || o1[..2] != [1, 1] || o2[..5] != [4, 1, 2, 5, 6] {
                println!("selftest mismatch (batch): put+get: done {}, ret {:?}, out {:?} / {:?}", done, rt, &o1[..2], &o2[..5]);
                std::process::exit(1);
            }
            // n = 0: done = 0, nothing else touched.
            done = -1;
            bfail("n=0", hand_hb_exec(hb, 0, ops.as_ptr(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done), 0);
            if done != 0 {
                println!("selftest mismatch (batch): n=0: done {}", done);
                std::process::exit(1);
            }
            // Up-front refusals: n < 0, NULL table, ret overlapping done.
            bfail("n<0", hand_hb_exec(hb, -1, ops.as_ptr(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done), 2);
            bfail("null table", hand_hb_exec(hb, 2, std::ptr::null(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done), 2);
            let rp = rt.as_mut_ptr();
            bfail("ret overlaps done", hand_hb_exec(hb, 2, ops.as_ptr(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rp, rp.add(1)), 2);
        }
        {
            // Stop rule: the first item fails (status 2), the second must not run.
            let ops: [*const i64; 2] = [bad_op.as_ptr(), get.as_ptr()];
            let lens: [i64; 2] = [bad_op.len() as i64, get.len() as i64];
            let outs: [*mut i64; 2] = [o1.as_mut_ptr(), o2.as_mut_ptr()];
            let olens: [i64; 2] = [OUT_WORDS as i64, OUT_WORDS as i64];
            rt = [-1; 2];
            o2.fill(0);
            bfail(
                "stop",
                hand_hb_exec(hb, 2, ops.as_ptr(), lens.as_ptr(), outs.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done),
                0,
            );
            if done != 1 || rt != [2, -1] || o2[0] != 0 {
                println!("selftest mismatch (batch): stop rule: done {}, ret {:?}, out2[0] {}", done, rt, o2[0]);
                std::process::exit(1);
            }
            // Bad item: item 1's out buffer is the ret table: status 2, done = 1, item 0 (a get) ran.
            let outs2: [*mut i64; 2] = [o1.as_mut_ptr(), rt.as_mut_ptr()];
            let ops2: [*const i64; 2] = [get.as_ptr(), get.as_ptr()];
            let lens2: [i64; 2] = [get.len() as i64, get.len() as i64];
            rt = [-1; 2];
            bfail(
                "bad item",
                hand_hb_exec(hb, 2, ops2.as_ptr(), lens2.as_ptr(), outs2.as_ptr(), olens.as_ptr(), rt.as_mut_ptr(), &mut done),
                2,
            );
            if done != 1 || rt != [0, -1] {
                println!("selftest mismatch (batch): bad item: done {}, ret {:?}", done, rt);
                std::process::exit(1);
            }
        }
        hand_h_free(hb);
    }
    println!("selftest ok");
}

fn usage() -> ! {
    eprintln!("usage: m4_hand perf|perfc|perfcb <sessions|tools|clocks> <n> <reps> | selftest");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("selftest") if args.len() == 2 => selftest(),
        Some(mode @ ("perf" | "perfc" | "perfcb")) if args.len() == 5 => {
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
            let (ns, ops, sink) = run_once(axis, n, reps, mode == "perfc", mode == "perfcb");
            eprintln!("sink={}", sink);
            println!("axis={} n={} hand_ns_per_op={:.1} allocs={}", axis, n, ns as f64 / ops as f64,
                     ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed));
        }
        _ => usage(),
    }
}

// m4cb lane: driver_m4c.rs (World over the manifest raw_entries = "c" adapter, sol::pruv_c extern "C" symbols) plus the
// batch entry m4cb__pruv_hb_m4_op. run.py splices this file in place of driver_m4.rs's store/World section (between
// LANE-WORLD-BEGIN and LANE-WORLD-END) and, for this lane only, rewrites the sessions-axis timed loop to call
// World::exec2 (one hb call per rep = the get + the put) instead of two exec calls. Prefill, q_mode and the other
// axes use the single-call exec exactly as in m4c.
// The host holds only an opaque PruvHandle: no state buffer, no Rust slices; every call is (pointer, length) pairs
// plus a status code and a `ret` out-pointer, as from C. The symbols are reached through std::hint::black_box'd
// function pointers, so the optimizer cannot inline the Rust bodies into the driver.
// Every global symbol of the generated pruv_c module carries the manifest package name as a prefix; this lane's
// manifest name is the lane tag, so the symbols are m4cb__pruv_*.
use sol::pruv_c::PruvHandle;

type NewF = extern "C" fn(i64) -> *mut PruvHandle;
type FreeF = unsafe extern "C" fn(*mut PruvHandle);
type InitF = unsafe extern "C" fn(*mut PruvHandle, *mut i64) -> i32;
type OpF = unsafe extern "C" fn(*mut PruvHandle, *const i64, i64, *mut i64, i64, *mut i64) -> i32;
// hb: (h, n, op table, op lens, out table, out lens, ret[n], done)
type BatchF = unsafe extern "C" fn(
    *mut PruvHandle,
    i64,
    *const *const i64,
    *const i64,
    *const *mut i64,
    *const i64,
    *mut i64,
    *mut i64,
) -> i32;
type ObsF = unsafe extern "C" fn(*mut PruvHandle, *mut i64, i64, *mut i64) -> i32;
type GrowF = unsafe extern "C" fn(*mut PruvHandle, i64, *mut i64) -> i32;

#[derive(Clone, Copy)]
struct Fns {
    new: NewF,
    free: FreeF,
    init: InitF,
    op: OpF,
    batch: BatchF,
    obs: ObsF,
    imp: ObsF,
    grow: GrowF,
}

// Any status other than PRUV_OK (0) cannot happen in this driver (valid arguments, no panic); exit loudly.
fn chk(s: i32, what: &str) {
    if s != 0 {
        eprintln!("pruv_c {} returned status {} (1 poisoned, 2 bad args, 3 panicked)", what, s);
        std::process::exit(3);
    }
}

enum R {
    Ok(usize),
    Invalid,
    GrowFailed,
    OutSmall,
}

struct World {
    h: *mut PruvHandle,
    f: Fns,
    len: usize, // len of the state, kept by the host (the host never sees the words)
    out: Vec<i64>,
    out2: Vec<i64>, // second output buffer, item 1 of exec2 (separate from `out`: the two ops' results must not overlap)
    grows: u64,
    grow_ns: u128,
}

impl World {
    fn handle(f: &Fns, len: usize) -> *mut PruvHandle {
        let h = (f.new)(len as i64);
        if h.is_null() {
            eprintln!("pruv_handle_new({}) returned null", len);
            std::process::exit(2);
        }
        h
    }

    fn init_call(f: &Fns, h: *mut PruvHandle) -> i64 {
        let mut ret = 0i64;
        chk(unsafe { (f.init)(h, &mut ret) }, "pruv_h_m4_init");
        ret
    }

    fn new(l: usize) -> World {
        let f = Fns {
            new: black_box::<NewF>(sol::pruv_c::m4cb__pruv_handle_new),
            free: black_box::<FreeF>(sol::pruv_c::m4cb__pruv_handle_free),
            init: black_box::<InitF>(sol::pruv_c::m4cb__pruv_h_m4_init),
            op: black_box::<OpF>(sol::pruv_c::m4cb__pruv_h_m4_op),
            batch: black_box::<BatchF>(sol::pruv_c::m4cb__pruv_hb_m4_op),
            obs: black_box::<ObsF>(sol::pruv_c::m4cb__pruv_h_m4_observe),
            imp: black_box::<ObsF>(sol::pruv_c::m4cb__pruv_h_m4_import),
            grow: black_box::<GrowF>(sol::pruv_c::m4cb__pruv_hr_m4_grow),
        };
        let mut h = World::handle(&f, l);
        let mut len = l;
        let r = World::init_call(&f, h);
        if r > 0 {
            // too small: a fresh handle of exactly N zero words, then init again
            unsafe { (f.free)(h) };
            len = r as usize;
            h = World::handle(&f, len);
            let r2 = World::init_call(&f, h);
            if r2 != 0 {
                eprintln!("m4_init failed twice (status {})", r2);
                std::process::exit(2);
            }
        } else if r < 0 {
            eprintln!("m4_init returned {}", r);
            std::process::exit(2);
        }
        World { h, f, len, out: Vec::new(), out2: Vec::new(), grows: 0, grow_ns: 0 }
    }

    fn observe(&mut self, out: &mut Vec<i64>) -> i64 {
        let mut ret = 0i64;
        chk(unsafe { (self.f.obs)(self.h, out.as_mut_ptr(), out.len() as i64, &mut ret) }, "pruv_h_m4_observe");
        ret
    }

    // Import admission (m4_import, L3 P13a.10) on the handle's state: (status, entries visited, "-"). The host never
    // sees the words here, so there is no corrupted copy (driver_m4.rs checks that one).
    fn import_check(&mut self) -> (i64, i64, String) {
        let mut cnt = vec![0i64; 3];
        let mut ret = 0i64;
        chk(unsafe { (self.f.imp)(self.h, cnt.as_mut_ptr(), cnt.len() as i64, &mut ret) }, "pruv_h_m4_import");
        (ret, cnt[0], "-".to_string())
    }

    // Grows the state to hold `n` words (at least twice the current length), as the single-call path does.
    fn grow_to(&mut self, n: usize) -> bool {
        let t = Instant::now();
        let want = std::cmp::max(n, 2 * self.len);
        let mut g = 0i64;
        chk(unsafe { (self.f.grow)(self.h, want as i64, &mut g) }, "pruv_hr_m4_grow");
        if g != 0 {
            return false;
        }
        self.len = want;
        self.grows += 1;
        self.grow_ns += t.elapsed().as_nanos();
        true
    }

    fn exec(&mut self, op: &[i64]) -> R {
        for attempt in 0..4 {
            let need = std::cmp::max(4096, 4 * self.len);
            if self.out.len() < need {
                self.out.resize(need, 0);
            }
            let mut ret = 0i64;
            let st = unsafe {
                (self.f.op)(self.h, op.as_ptr(), op.len() as i64, self.out.as_mut_ptr(), self.out.len() as i64, &mut ret)
            };
            chk(st, "pruv_h_m4_op");
            match ret {
                0 => return R::Ok(self.out[0].max(0) as usize),
                1 => {
                    if attempt == 3 {
                        return R::GrowFailed;
                    }
                    let n = self.out[0].max(0) as usize;
                    if !self.grow_to(n) {
                        return R::GrowFailed;
                    }
                }
                2 => return R::Invalid,
                _ => return R::OutSmall,
            }
        }
        R::GrowFailed
    }

    // Two ops in ONE hb call (item 0 -> self.out, item 1 -> self.out2). The batch stops after the first item whose
    // result is nonzero; on a grow request (result 1) the state is grown and the remaining items (the one that asked
    // for the grow and everything after it) are retried, exactly like exec retries one op.
    fn exec2(&mut self, a: &[i64], b: &[i64]) -> R {
        let ops: [&[i64]; 2] = [a, b];
        let mut start = 0usize;
        for attempt in 0..4 {
            let need = std::cmp::max(4096, 4 * self.len);
            if self.out.len() < need {
                self.out.resize(need, 0);
            }
            if self.out2.len() < need {
                self.out2.resize(need, 0);
            }
            let op_ptrs: [*const i64; 2] = [ops[0].as_ptr(), ops[1].as_ptr()];
            let op_lens: [i64; 2] = [ops[0].len() as i64, ops[1].len() as i64];
            let out_ptrs: [*mut i64; 2] = [self.out.as_mut_ptr(), self.out2.as_mut_ptr()];
            let out_lens: [i64; 2] = [self.out.len() as i64, self.out2.len() as i64];
            let mut ret = [0i64; 2];
            let mut done = 0i64;
            let st = unsafe {
                (self.f.batch)(
                    self.h,
                    (2 - start) as i64,
                    op_ptrs.as_ptr().add(start),
                    op_lens.as_ptr().add(start),
                    out_ptrs.as_ptr().add(start),
                    out_lens.as_ptr().add(start),
                    ret.as_mut_ptr().add(start),
                    &mut done,
                )
            };
            chk(st, "pruv_hb_m4_op");
            if done < 1 || done as usize > 2 - start {
                eprintln!("pruv_hb_m4_op: done = {} for {} items", done, 2 - start);
                std::process::exit(3);
            }
            let last = start + done as usize - 1; // the item that stopped the batch (or the last one)
            match ret[last] {
                0 => return R::Ok(self.out[0].max(0) as usize),
                1 => {
                    if attempt == 3 {
                        return R::GrowFailed;
                    }
                    let want_words = if last == 0 { self.out[0] } else { self.out2[0] };
                    let n = want_words.max(0) as usize;
                    if !self.grow_to(n) {
                        return R::GrowFailed;
                    }
                    start = last; // retry the item that asked for the grow and the ones after it
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

impl Drop for World {
    fn drop(&mut self) {
        unsafe { (self.f.free)(self.h) };
    }
}

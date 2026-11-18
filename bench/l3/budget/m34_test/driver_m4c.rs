// m4c lane: the World of driver_m4.rs over the manifest raw_entries = "c" adapter (sol::pruv_c, extern "C" symbols).
// run.py splices this file in place of driver_m4.rs's store/World section (between LANE-WORLD-BEGIN and
// LANE-WORLD-END); q_mode and perf are unchanged. The host holds only an opaque PruvHandle: no state buffer, no
// Rust slices; every call is (pointer, length) pairs plus a status code and a `ret` out-pointer, as from C.
// Every global symbol of the generated pruv_c module carries the manifest package name as a prefix; the m4c lane's
// manifest name is the lane tag, so the symbols are m4c__pruv_*.
// The symbols are reached through std::hint::black_box'd function pointers, so the optimizer cannot inline the
// Rust bodies into the driver and the calls cost what a call from a separate C translation unit costs.
use sol::pruv_c::PruvHandle;

type NewF = extern "C" fn(i64) -> *mut PruvHandle;
type FreeF = unsafe extern "C" fn(*mut PruvHandle);
type InitF = unsafe extern "C" fn(*mut PruvHandle, *mut i64) -> i32;
type OpF = unsafe extern "C" fn(*mut PruvHandle, *const i64, i64, *mut i64, i64, *mut i64) -> i32;
type ObsF = unsafe extern "C" fn(*mut PruvHandle, *mut i64, i64, *mut i64) -> i32;
type GrowF = unsafe extern "C" fn(*mut PruvHandle, i64, *mut i64) -> i32;

#[derive(Clone, Copy)]
struct Fns {
    new: NewF,
    free: FreeF,
    init: InitF,
    op: OpF,
    obs: ObsF,
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
            new: black_box::<NewF>(sol::pruv_c::m4c__pruv_handle_new),
            free: black_box::<FreeF>(sol::pruv_c::m4c__pruv_handle_free),
            init: black_box::<InitF>(sol::pruv_c::m4c__pruv_h_m4_init),
            op: black_box::<OpF>(sol::pruv_c::m4c__pruv_h_m4_op),
            obs: black_box::<ObsF>(sol::pruv_c::m4c__pruv_h_m4_observe),
            grow: black_box::<GrowF>(sol::pruv_c::m4c__pruv_hr_m4_grow),
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
        World { h, f, len, out: Vec::new(), grows: 0, grow_ns: 0 }
    }

    fn observe(&mut self, out: &mut Vec<i64>) -> i64 {
        let mut ret = 0i64;
        chk(unsafe { (self.f.obs)(self.h, out.as_mut_ptr(), out.len() as i64, &mut ret) }, "pruv_h_m4_observe");
        ret
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
                    let t = Instant::now();
                    let want = std::cmp::max(n, 2 * self.len);
                    let mut g = 0i64;
                    chk(unsafe { (self.f.grow)(self.h, want as i64, &mut g) }, "pruv_hr_m4_grow");
                    if g != 0 {
                        return R::GrowFailed;
                    }
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

impl Drop for World {
    fn drop(&mut self) {
        unsafe { (self.f.free)(self.h) };
    }
}

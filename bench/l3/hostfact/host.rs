//! Host side of the P10H bundle (crate `hostfact_host`): synchronous G6 host facts for budget and req.
//! The host only answers or performs side effects; every decision stays in Pruv.
//! Test setup (`reset`, `set_*`, `take_trace`) is thread-local state; the facts append to a trace unless
//! `set_trace(false)` (perf lanes: a plain call with no bookkeeping beyond one flag read).
use std::cell::{Cell, RefCell};

/// One trace entry: (tag, a, b).
pub type Event = (i64, i64, i64);

pub const T_RUNNING: i64 = 1; // running_ids(session = a, settled = b)
pub const T_REENTRY: i64 = 2; // result of the reenter callback (a)
pub const T_QUIET: i64 = 3; // is_quiet(session = a)
pub const T_RELEASE: i64 = 4; // release_held(session = a)
pub const T_TITLE: i64 = 5; // title_obs(s = a)
pub const T_NEXT: i64 = 6; // next_item(s = a, m = b)
pub const T_TEMP: i64 = 7; // is_temporary(item = a)

thread_local! {
    static TRACE_ON: Cell<bool> = const { Cell::new(true) };
    static TRACE: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    static RUNNING: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static QUIET: Cell<i64> = const { Cell::new(0) };
    static RELEASE: Cell<i64> = const { Cell::new(0) };
    // req grid: the item of (s, m) answers ANSWER from is_temporary, every other item answers 0
    static TARGET: Cell<i64> = const { Cell::new(-1) };
    static ANSWER: Cell<i64> = const { Cell::new(0) };
    static REENTER: Cell<Option<fn() -> i64>> = const { Cell::new(None) };
}

#[inline(always)]
fn log(tag: i64, a: i64, b: i64) {
    if TRACE_ON.with(|t| t.get()) {
        TRACE.with(|t| t.borrow_mut().push((tag, a, b)));
    }
}

/// The item id `next_item(s, m)` returns (fresh observation per item; m < 16).
pub fn item_id(s: i64, m: i64) -> i64 {
    s * 16 + m + 1
}

// ---------------------------------------------------------------- test setup
pub fn reset() {
    TRACE_ON.with(|t| t.set(true));
    TRACE.with(|t| t.borrow_mut().clear());
    RUNNING.with(|t| *t.borrow_mut() = vec![0, 0]);
    QUIET.with(|t| t.set(0));
    RELEASE.with(|t| t.set(0));
    TARGET.with(|t| t.set(-1));
    ANSWER.with(|t| t.set(0));
    REENTER.with(|t| t.set(None));
}
/// The raw answer of running_ids: `[status, n, ids...]` (not validated here).
pub fn set_running(answer: Vec<i64>) {
    RUNNING.with(|t| *t.borrow_mut() = answer);
}
pub fn set_quiet(v: i64) {
    QUIET.with(|t| t.set(v));
}
pub fn set_release(v: i64) {
    RELEASE.with(|t| t.set(v));
}
/// The item of (s, m) answers `temp_pattern` from is_temporary (0 = no match anywhere, 1 = temporary,
/// anything else = an invalid answer); all other items answer 0.
pub fn set_req_grid(s: i64, m: i64, temp_pattern: i64) {
    TARGET.with(|t| t.set(item_id(s, m)));
    ANSWER.with(|t| t.set(temp_pattern));
}
/// Callback run (once, then cleared) inside running_ids; its return value is traced with T_REENTRY.
pub fn set_reenter(cb: Option<fn() -> i64>) {
    REENTER.with(|t| t.set(cb));
}
pub fn set_trace(on: bool) {
    TRACE_ON.with(|t| t.set(on));
}
pub fn take_trace() -> Vec<Event> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

// ---------------------------------------------------------------- host facts
#[inline(never)]
pub fn running_ids(session: i64, settled: i64) -> Vec<i64> {
    log(T_RUNNING, session, settled);
    if let Some(cb) = REENTER.with(|t| t.replace(None)) {
        let r = cb();
        log(T_REENTRY, r, 0);
    }
    RUNNING.with(|t| t.borrow().clone())
}

#[inline(never)]
pub fn is_quiet(session: i64) -> i64 {
    log(T_QUIET, session, 0);
    QUIET.with(|t| t.get())
}

#[inline(never)]
pub fn release_held(session: i64) -> i64 {
    log(T_RELEASE, session, 0);
    RELEASE.with(|t| t.get())
}

#[inline(never)]
pub fn next_item(s: i64, m: i64) -> i64 {
    log(T_NEXT, s, m);
    item_id(s, m)
}

#[inline(never)]
pub fn is_temporary(item: i64) -> i64 {
    log(T_TEMP, item, 0);
    if item == TARGET.with(|t| t.get()) {
        ANSWER.with(|t| t.get())
    } else {
        0
    }
}

#[inline(never)]
pub fn title_obs(s: i64) -> i64 {
    log(T_TITLE, s, 0);
    s + 1
}

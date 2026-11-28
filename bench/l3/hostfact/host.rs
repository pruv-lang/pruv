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

// ---------------------------------------------------------------- req list/open grid (P10H)
// World of the synchronous req facts: the packed session list (see root.pruv for the layout), the packed item list,
// the host's isTemporary answers (by session index of the current list), and the recorded effects. A fresh
// observation is a fresh copy of the current list; `set_rg_switch(k, list)` replaces the list after the k-th
// observation (fresh-query gate). Counters always run; the trace only when on.
pub const T_SL: i64 = 8; // rg_sessions (a = observation number, from 1)
pub const T_RGTEMP: i64 = 9; // rg_is_temporary(i = a)
pub const T_ITEMS: i64 = 10; // rg_items
pub const T_ROW: i64 = 11; // rg_row(j = a, b = 1 bound / 0 none)
pub const T_READ: i64 = 12; // rg_read
pub const T_CANCEL: i64 = 13; // rg_cancelled
pub const T_EXIST: i64 = 14; // rg_existing
pub const T_CREATE: i64 = 15; // rg_create(turns = a)

#[derive(Default)]
pub struct RgWorld {
    pub sessions: String,
    pub switch_after: Option<(u64, String)>,
    pub items: String,
    pub temp: Vec<bool>,
    pub read_turns: i64,
    pub cancelled: bool,
    pub sl_calls: u64,
    pub temp_calls: u64,
    /// (item index, bound session id, title)
    pub rows: Vec<(i64, Option<String>, String)>,
    pub existing: Option<String>,
    pub created: Option<(String, i64)>,
}

thread_local! {
    static RG: RefCell<RgWorld> = RefCell::new(RgWorld::default());
    static RG_REENTER: Cell<Option<fn() -> i64>> = const { Cell::new(None) };
}

/// Appends a field (two length bytes, each < 128, then the bytes).
pub fn pack_field(out: &mut String, s: &str) {
    pack_count(out, s.len());
    out.push_str(s);
}
/// Appends a count in the two-byte field-length encoding.
pub fn pack_count(out: &mut String, n: usize) {
    assert!(n < 16384, "packed field too long");
    out.push(((n >> 7) as u8) as char);
    out.push(((n & 127) as u8) as char);
}

pub fn rg_reset(sessions: String, items: String, temp: Vec<bool>, read_turns: i64) {
    RG.with(|w| {
        *w.borrow_mut() = RgWorld { sessions, items, temp, read_turns, ..RgWorld::default() };
    });
    RG_REENTER.with(|t| t.set(None));
}
pub fn set_rg_switch(after: u64, sessions: String) {
    RG.with(|w| w.borrow_mut().switch_after = Some((after, sessions)));
}
pub fn set_rg_cancelled(v: bool) {
    RG.with(|w| w.borrow_mut().cancelled = v);
}
/// Callback run (once, then cleared) inside the next rg_sessions; its return value is traced with T_REENTRY.
pub fn set_rg_reenter(cb: Option<fn() -> i64>) {
    RG_REENTER.with(|t| t.set(cb));
}
pub fn rg_take() -> RgWorld {
    RG.with(|w| std::mem::take(&mut *w.borrow_mut()))
}
pub fn rg_counts() -> (u64, u64) {
    RG.with(|w| {
        let w = w.borrow();
        (w.sl_calls, w.temp_calls)
    })
}
/// Clears the recorded effects and counters, keeping the world (perf rounds).
pub fn rg_clear_effects() {
    RG.with(|w| {
        let mut w = w.borrow_mut();
        w.rows.clear();
        w.existing = None;
        w.created = None;
        w.sl_calls = 0;
        w.temp_calls = 0;
    });
}

#[inline(never)]
pub fn rg_sessions() -> String {
    let n = RG.with(|w| {
        let mut w = w.borrow_mut();
        w.sl_calls += 1;
        w.sl_calls
    });
    log(T_SL, n as i64, 0);
    if let Some(cb) = RG_REENTER.with(|t| t.replace(None)) {
        let r = cb();
        log(T_REENTRY, r, 0);
    }
    RG.with(|w| {
        let mut w = w.borrow_mut();
        let out = w.sessions.clone();
        if let Some((k, _)) = &w.switch_after {
            if *k == n {
                let (_, next) = w.switch_after.take().unwrap();
                w.sessions = next;
            }
        }
        out
    })
}

#[inline(never)]
pub fn rg_is_temporary(i: i64) -> bool {
    log(T_RGTEMP, i, 0);
    RG.with(|w| {
        let mut w = w.borrow_mut();
        w.temp_calls += 1;
        w.temp.get(i as usize).copied().unwrap_or(false)
    })
}

#[inline(never)]
pub fn rg_items() -> String {
    log(T_ITEMS, 0, 0);
    RG.with(|w| w.borrow().items.clone())
}

#[inline(never)]
pub fn rg_row(j: i64, sid: &str, so: i64, sl: i64, ttl: &str, to: i64, tl: i64) {
    log(T_ROW, j, (sl >= 0) as i64);
    let id = if sl < 0 { None } else { Some(sid[so as usize..(so + sl) as usize].to_string()) };
    let title = ttl[to as usize..(to + tl) as usize].to_string();
    RG.with(|w| w.borrow_mut().rows.push((j, id, title)));
}

#[inline(never)]
pub fn rg_read(_thread: &str) -> i64 {
    log(T_READ, 0, 0);
    RG.with(|w| w.borrow().read_turns)
}

#[inline(never)]
pub fn rg_cancelled() -> bool {
    log(T_CANCEL, 0, 0);
    RG.with(|w| w.borrow().cancelled)
}

#[inline(never)]
pub fn rg_existing(sid: &str, so: i64, sl: i64) {
    log(T_EXIST, 0, 0);
    let id = sid[so as usize..(so + sl) as usize].to_string();
    RG.with(|w| w.borrow_mut().existing = Some(id));
}

#[inline(never)]
pub fn rg_create(thread: &str, turns: i64) {
    log(T_CREATE, turns, 0);
    RG.with(|w| w.borrow_mut().created = Some((thread.to_string(), turns)));
}

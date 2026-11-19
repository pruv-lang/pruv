//! Host side of the budget lifecycle's synchronous host facts (crate `bud_facts`, P19; P10H pattern of
//! bench/l3/hostfact/host.rs). bud_lifecycle::maybe_close calls `is_quiet` at the point the spec consults
//! isQuiet(session) (B:399-408: after the open-state and running-runs checks, before CLOSE); the host only answers.
//! Pruv validates the answer (0 no, 1 yes, anything else: status 2 with nothing written).
//!
//! Test setup is thread-local: an answer stream (`set_answers`, the case's `quiet` line) with a default for answers
//! past its end (0 in q mode: isQuiet defaults to false; perf lanes use an empty stream with default 1), and a call
//! counter (`calls` / `set_calls`). Q records the packed session key and effect count with each answer, in call
//! order. The hand baseline (hand_m7_u16.rs) calls the same function at the same point.
use std::cell::{Cell, RefCell};

thread_local! {
    static ANSWERS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static DEFAULT: Cell<i64> = const { Cell::new(0) };
    static QUERIES: RefCell<Option<Vec<Query>>> = const { RefCell::new(None) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

/// Replaces the answer stream; answer i is taken by the i-th call since `set_calls(0)`, `default` past the end.
pub fn set_answers(v: &[i64], default: i64) {
    ANSWERS.with(|a| {
        let mut a = a.borrow_mut();
        a.clear();
        a.extend_from_slice(v);
    });
    DEFAULT.with(|d| d.set(default));
}

/// Number of answers taken (the index of the next answer).
pub fn calls() -> usize {
    CALLS.with(|c| c.get())
}

pub fn set_calls(n: usize) {
    CALLS.with(|c| c.set(n));
    QUERIES.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            q.truncate(n);
        }
    });
}

/// The answer the i-th call takes (for the drivers' `E quiet` log line).
#[inline(always)]
pub fn answer(i: usize) -> i64 {
    ANSWERS.with(|a| a.borrow().get(i).copied()).unwrap_or_else(|| DEFAULT.with(|d| d.get()))
}

#[derive(Clone)]
pub struct Query {
    pub index: usize,
    pub answer: i64,
    pub kn: i64,
    pub key: Vec<i64>,
    pub fxn: i64,
}

pub fn record_queries() {
    QUERIES.with(|q| *q.borrow_mut() = Some(Vec::new()));
}

pub fn query(index: usize) -> Query {
    QUERIES.with(|q| q.borrow().as_ref().unwrap()[index].clone())
}

pub fn quiet_log(index: usize) -> String {
    use std::fmt::Write;
    let q = query(index);
    let mut line = format!("E quiet {} {}", q.answer, q.kn);
    for word in q.key {
        write!(line, " {}", word).unwrap();
    }
    write!(line, " @{}", q.fxn).unwrap();
    line
}

/// Answer the next query and record its arguments in q mode.
#[inline(always)]
pub fn is_quiet(k: &[i64], ks: i64, kn: i64, fxn: i64) -> i64 {
    let index = calls();
    CALLS.with(|c| c.set(index + 1));
    let answer = answer(index);
    QUERIES.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            let start = ks as usize;
            let end = start + (kn as usize).div_ceil(4);
            q.push(Query { index, answer, kn, key: k[start..end].to_vec(), fxn });
        }
    });
    answer
}

// ---- runningDelegationIds (P24, bud_halt::halt) ----
// A separate answer stream: answer i is the packed list `[0, n, (len, words(len) packed words)*]` (status 0, then the
// bud_keys records of the running delegation ids) taken by the i-th `running_ids` call since `set_running_calls(0)`;
// `[0, 0]` (no running delegations) past the end. Bad answers (status != 0, bad framing) are for G fixtures; Pruv
// rejects them after SETTLE with status 2. Q records the ref (framed packed sessionId and budgetId keys) and the effect
// count with each call, so the driver places the query among the call's effects.

#[derive(Clone)]
pub struct RunningQuery {
    pub sid: Vec<i64>,
    pub bid: Vec<i64>,
    pub fxn: i64,
    /// The answer this call returned (`[status, n, keys...]`), as the host gave it.
    pub answer: Vec<i64>,
}

thread_local! {
    static RUNNING: RefCell<Vec<Vec<i64>>> = const { RefCell::new(Vec::new()) };
    static RQUERIES: RefCell<Option<Vec<RunningQuery>>> = const { RefCell::new(None) };
    static RCALLS: Cell<usize> = const { Cell::new(0) };
}

/// Replaces the runningDelegationIds answer stream.
pub fn set_running(v: Vec<Vec<i64>>) {
    RUNNING.with(|a| *a.borrow_mut() = v);
}

/// Appends one answer to the stream.
pub fn push_running(v: Vec<i64>) {
    RUNNING.with(|a| a.borrow_mut().push(v));
}

pub fn running_calls() -> usize {
    RCALLS.with(|c| c.get())
}

pub fn set_running_calls(n: usize) {
    RCALLS.with(|c| c.set(n));
    RQUERIES.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            q.truncate(n);
        }
    });
}

pub fn record_running() {
    RQUERIES.with(|q| *q.borrow_mut() = Some(Vec::new()));
}

/// The answer the i-th call takes.
pub fn running_answer(i: usize) -> Vec<i64> {
    RUNNING.with(|a| a.borrow().get(i).cloned()).unwrap_or_else(|| vec![0, 0])
}

/// The i-th recorded call (q mode).
pub fn running_query(i: usize) -> RunningQuery {
    RQUERIES.with(|q| q.borrow().as_ref().unwrap()[i].clone())
}

fn framed(k: &[i64], ks: i64, kn: i64) -> Vec<i64> {
    let start = ks as usize;
    let mut v = vec![kn];
    v.extend_from_slice(&k[start..start + (kn as usize).div_ceil(4)]);
    v
}

/// Answer the next runningDelegationIds query and record its ref and the answer it returns in q mode. The answer is
/// moved out of the stream (each answer is taken once), so the host cost is the call and the move, no copy (q mode
/// also clones it into the record).
#[inline(always)]
pub fn running_ids(k: &[i64], ks: i64, kn: i64, bk: &[i64], bs: i64, bn: i64, fxn: i64) -> Vec<i64> {
    let index = running_calls();
    RCALLS.with(|c| c.set(index + 1));
    let a = RUNNING.with(|a| a.borrow_mut().get_mut(index).map(std::mem::take)).unwrap_or_else(|| vec![0, 0]);
    RQUERIES.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            q.push(RunningQuery { sid: framed(k, ks, kn), bid: framed(bk, bs, bn), fxn, answer: a.clone() });
        }
    });
    a
}

// P30 synchronous unit fact. The comparison lane enables logging.
thread_local! {
    static UNIT: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static UNIT_LOG: RefCell<Option<Vec<(i64, Vec<i64>)>>> = const { RefCell::new(None) };
}

pub fn set_unit(answer: &[i64]) {
    UNIT.with(|u| {
        let mut u = u.borrow_mut();
        u.clear();
        u.extend_from_slice(answer);
    });
}

pub fn record_unit() {
    UNIT_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
}

pub fn take_unit_log() -> Vec<(i64, Vec<i64>)> {
    UNIT_LOG.with(|log| log.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
}

pub fn unit(fxn: i64) -> Vec<i64> {
    let answer = UNIT.with(|u| u.borrow().clone());
    UNIT_LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.push((fxn, answer.clone()));
        }
    });
    answer
}

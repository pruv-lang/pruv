//! Host side of the budget lifecycle's synchronous host facts (crate `bud_facts`, P19; P10H pattern of
//! bench/l3/hostfact/host.rs). bud_lifecycle::maybe_close calls `is_quiet` at the point the spec consults
//! isQuiet(session) (B:399-408: after the open-state and running-runs checks, before CLOSE); the host only answers.
//! Pruv validates the answer (0 no, 1 yes, anything else: status 2 with nothing written).
//!
//! Test setup is thread-local: an answer stream (`set_answers`, the case's `quiet` line) with a default for answers
//! past its end (0 in q mode: isQuiet defaults to false; perf lanes use an empty stream with default 1), and a call
//! counter (`calls` / `set_calls`) that the drivers read before and after each call to log `E quiet <answer>` for each
//! answer actually taken, in call order. The hand baseline (hand_m7_u16.rs) calls the same function at the same point,
//! so both sides pay the same host work.
use std::cell::{Cell, RefCell};

thread_local! {
    static ANSWERS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static DEFAULT: Cell<i64> = const { Cell::new(0) };
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
}

/// The answer the i-th call takes (for the drivers' `E quiet` log line).
#[inline(always)]
pub fn answer(i: usize) -> i64 {
    ANSWERS.with(|a| a.borrow().get(i).copied()).unwrap_or_else(|| DEFAULT.with(|d| d.get()))
}

/// isQuiet(session): the session is the packed key k[ks .. ks + words(kn)) (bud_keys layout). The test host's answer
/// does not depend on it; a real host would look the session up. Returns the next answer of the stream.
#[inline(always)]
pub fn is_quiet(_k: &[i64], _ks: i64, _kn: i64, _fxn: i64) -> i64 {
    let i = CALLS.with(|c| {
        let i = c.get();
        c.set(i + 1);
        i
    });
    answer(i)
}

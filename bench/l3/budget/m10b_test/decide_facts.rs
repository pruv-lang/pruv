// P31 synchronous releaseHeld fact for decide/extend. The host answers whether a held continuation was released
// (0 = none, 1 = released); every decision about it is made in bud_tdecide.pruv. The comparison lane queues the
// oracle's answers and records each query (effect position, session ref, budget id, answer).

use std::cell::{Cell, RefCell};

pub struct Release {
    pub fxn: i64,
    pub sid: Vec<i64>,
    pub bid: Vec<i64>,
    pub answer: i64,
    pub text: Option<(String, String)>,
}

thread_local! {
    static ANSWERS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<usize> = const { Cell::new(0) };
    static LOG: RefCell<Option<Vec<Release>>> = const { RefCell::new(None) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

pub fn reset() {
    ANSWERS.with(|a| a.borrow_mut().clear());
    NEXT.with(|n| n.set(0));
    CALLS.with(|c| c.set(0));
    LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
}

pub fn push(v: &[i64]) {
    ANSWERS.with(|a| a.borrow_mut().extend_from_slice(v));
}

pub fn calls() -> usize {
    CALLS.with(|c| c.get())
}

pub fn take_log() -> Vec<Release> {
    LOG.with(|l| l.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
}

fn framed(k: &[i64], ks: i64, kn: i64) -> Vec<i64> {
    let start = ks as usize;
    let mut v = vec![kn];
    v.extend_from_slice(&k[start..start + (kn as usize).div_ceil(4)]);
    v
}

/// Answer the next releaseHeld query (default 0 = nothing released when the queue is empty).
#[inline(always)]
pub fn release_held(k: &[i64], ks: i64, kn: i64, bk: &[i64], bs: i64, bn: i64, fxn: i64) -> i64 {
    CALLS.with(|c| c.set(c.get() + 1));
    let i = NEXT.with(|n| {
        let i = n.get();
        n.set(i + 1);
        i
    });
    let answer = ANSWERS.with(|a| a.borrow().get(i).copied().unwrap_or(0));
    LOG.with(|l| {
        if let Some(l) = l.borrow_mut().as_mut() {
            l.push(Release { fxn, sid: framed(k, ks, kn), bid: framed(bk, bs, bn), answer, text: None });
        }
    });
    answer
}

pub fn release_held_text(sid: &str, bid: &str, position: i64) -> i64 {
    let answer = release_held(&[], 0, 0, &[], 0, 0, position);
    LOG.with(|l| {
        if let Some(last) = l.borrow_mut().as_mut().and_then(|log| log.last_mut()) {
            last.text = Some((sid.to_owned(), bid.to_owned()));
        }
    });
    answer
}

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
    static RUNNING_POSITION: Cell<i64> = const { Cell::new(0) };
    static CURRENT_WORDS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
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

// Encode JavaScript UTF-16 losslessly inside a Rust String. Escape the escape
// character itself so isolated surrogates and ordinary text remain distinct.
pub fn encode_js_text(units: impl IntoIterator<Item = u16>) -> String {
    let units = units.into_iter();
    let mut text = String::with_capacity(units.size_hint().0);
    for ch in char::decode_utf16(units) {
        match ch {
            Ok('\u{e000}') => text.push_str("\u{e000}\u{e000}"),
            Ok(ch) => text.push(ch),
            Err(err) => {
                text.push('\u{e000}');
                text.push(char::from_u32(0xe100 + u32::from(err.unpaired_surrogate() - 0xd800)).unwrap());
            }
        }
    }
    text
}

pub fn decode_js_text(text: &str) -> Vec<u16> {
    let mut chars = text.chars();
    let mut units = Vec::new();
    while let Some(ch) = chars.next() {
        if ch == '\u{e000}' {
            let escaped = chars.next().expect("missing JavaScript text escape");
            units.push(if escaped == '\u{e000}' { 0xe000 } else { 0xd800 + (escaped as u16 - 0xe100) });
        } else {
            units.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
        }
    }
    units
}

fn packed_key(key: &str) -> Vec<i64> {
    let mut words = Vec::new();
    let mut count = 0;
    for unit in decode_js_text(key) {
        if count % 4 == 0 {
            words.push(0);
        }
        *words.last_mut().unwrap() |= i64::from(unit) << (48 - 16 * (count % 4));
        count += 1;
    }
    words.insert(0, count);
    words
}

/// Decode the UTF-16 units of a frame of the current answer already validated by Pruv (`pos` = first packed word).
pub fn unit_text(pos: i64, units: i64) -> String {
    let (pos, units) = (pos as usize, units as usize);
    CURRENT_WORDS.with(|w| {
        let words = &w.borrow()[pos..];
        encode_js_text((0..units).map(|i| (words[i / 4] >> (48 - 16 * (i % 4))) as u16))
    })
}

/// Word `i` of the current answer (0 past the end; Pruv checks bounds first).
pub fn running_word(i: i64) -> i64 {
    CURRENT_WORDS.with(|w| usize::try_from(i).ok().and_then(|i| w.borrow().get(i).copied()).unwrap_or(0))
}

/// Set the effect position recorded by the next typed running-delegations query.
pub fn running_position(position: i64) {
    RUNNING_POSITION.with(|p| p.set(position));
}

/// Consume one host answer, keep its wire words for `running_word` / `unit_text` and return their count.
pub fn running_fetch(sid: &str, bid: &str) -> i64 {
    let index = running_calls();
    RCALLS.with(|c| c.set(index + 1));
    let answer = RUNNING.with(|r| r.borrow_mut().get_mut(index).map(std::mem::take))
        .unwrap_or_else(|| vec![0, 0]);
    RQUERIES.with(|q| {
        if let Some(log) = q.borrow_mut().as_mut() {
            log.push(RunningQuery {
                sid: packed_key(sid),
                bid: packed_key(bid),
                fxn: RUNNING_POSITION.with(Cell::get),
                answer: answer.clone(),
            });
        }
    });
    let n = answer.len() as i64;
    CURRENT_WORDS.with(|w| *w.borrow_mut() = answer);
    n
}

// Packs the session key on the stack, as hand_m7_u16 does (pack_u16_into); keys past 64 words take the heap path.
pub fn is_quiet_text(sid: &str, position: i64) -> i64 {
    let mut kbuf = [0i64; 65];
    let mut count: usize = 0;
    let mut long = false;
    let mut chars = sid.chars();
    while let Some(ch) = chars.next() {
        let mut pair = [0u16; 2];
        let units: &[u16] = if ch == '\u{e000}' {
            let escaped = chars.next().expect("missing JavaScript text escape");
            pair[0] = if escaped == '\u{e000}' { 0xe000 } else { 0xd800 + (escaped as u16 - 0xe100) };
            &pair[..1]
        } else {
            ch.encode_utf16(&mut pair)
        };
        for &unit in units {
            let word = 1 + count / 4;
            if word >= kbuf.len() {
                long = true;
                break;
            }
            kbuf[word] |= i64::from(unit) << (48 - 16 * (count % 4));
            count += 1;
        }
        if long {
            break;
        }
    }
    if long {
        let key = packed_key(sid);
        return is_quiet(&key, 1, key[0], position);
    }
    kbuf[0] = count as i64;
    is_quiet(&kbuf, 1, count as i64, position)
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

// Reading status records the complete answer once, even if it is malformed.
pub fn unit_at(fxn: i64, i: i64) -> i64 {
    UNIT.with(|u| {
        let answer = u.borrow();
        if i == 0 {
            UNIT_LOG.with(|log| {
                if let Some(log) = log.borrow_mut().as_mut() {
                    log.push((fxn, answer.clone()));
                }
            });
        }
        if answer.len() != 6 {
            return -1;
        }
        answer.get(i as usize).copied().unwrap_or(-1)
    })
}

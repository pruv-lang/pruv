// Hand Rust baseline for `bench/l3/lib/ent_bound.pruv` (Rust-shape reset step 3.5).
// Ops 7 session_threads, 8 excluded_from_bound, 9 bound_session_index. Ops 7 and 8 are replay steps over the shared
// context (`fn item(cx: &mut Ctx) -> Result<(), Halt>`); op 9 is two host round trips driven by `entry_api`
// (`index_collect` / `index_request` build the request, `index_complete` builds the done packet from the answer).
// Session record (30 words, slots of 3 at position p): id p, temporary p+3, engine p+6, externalThread p+9 (tag 11 object:
// engine p+12, thread p+15, forks p+18), terminal p+21 (tag 11: cliSession p+24), engineThreads p+27 (rows of 9: key,
// bound, threadId).
// Check: rustc --edition 2021 --test ent_bound.rs -o work/b && work/b

#![allow(dead_code)]

#[cfg(not(shared_ctx))]
#[path = "ent_state.rs"]
pub mod ent_state;
#[cfg(shared_ctx)]
pub use crate::ent_state;

use ent_state::ent_packet::{boundary, mk_packet, put_vs, sec_start, section_starts, start_in, MAX_WORDS};
use ent_state::ent_value::{self, key, units, Pool, PoolBuf, Value};
use ent_state::{blob_pool, Ctx, Halt, Output, Request};
use std::collections::HashMap;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

fn slot(w: &[i64], p: usize) -> [i64; 3] {
    [rd(w, p), rd(w, p + 1), rd(w, p + 2)]
}

fn value(w: &[i64], p: usize) -> Value {
    Value::parse(slot(w, p))
}

fn clampu(x: i64, hi: i64) -> usize {
    x.clamp(0, hi) as usize
}

// ---- session pairs ----

/// Where the engine of a pair comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eng {
    /// the literal "codex" (terminal session with a nullish engine)
    Codex,
    /// the value slot at this position, whole
    Slot(usize),
    /// the first `n` bytes of the string slot at this position (an engineThreads key cut at its first ':')
    Cut(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pair {
    eng: Eng,
    /// position of the thread slot
    thread: usize,
}

/// Bytes of the string slot at p before its first ':' (the whole string when there is none).
/// The ASCII colon is itself a valid byte boundary for the sub-slot.
fn colon_at(w: &[i64], p: usize, pool: Pool) -> usize {
    let Value::Str(s) = value(w, p) else { return 0 };
    let n = s.len.min(pool.nbytes().saturating_sub(s.off));
    (0..n).position(|i| pool.byte(s.off + i) == b':').unwrap_or(n)
}

/// The (engine, thread) pairs of the session at p, in order, appended to `out`.
/// starts = section_starts(w, head): sessions and their sections are many.
fn ses_pairs(w: &[i64], starts: &[usize], p: usize, out: &mut Vec<Pair>) {
    if rd(w, p + 9) == 11 {
        out.push(Pair { eng: Eng::Slot(p + 12), thread: p + 15 });
        if rd(w, p + 18) == 7 {
            let base = start_in(starts, clampu(rd(w, p + 19), 1000000));
            let cnt = clampu(rd(w, p + 20), 16777216);
            out.extend((0..cnt).map(|i| Pair { eng: Eng::Slot(p + 12), thread: base + 3 * i }));
        }
    }
    if rd(w, p + 21) == 11 && value(w, p + 24).truthy() {
        // engine ?? "codex" (undefined, null or absent only)
        let eng = if value(w, p + 6).nullish() { Eng::Codex } else { Eng::Slot(p + 6) };
        out.push(Pair { eng, thread: p + 24 });
    }
    if rd(w, p + 27) == 8 {
        let base = start_in(starts, clampu(rd(w, p + 28), 1000000));
        let cnt = clampu(rd(w, p + 29), 16777216);
        let pool = Pool::at(w, start_in(starts, 0));
        for q in (0..cnt).map(|i| base + 9 * i) {
            if rd(w, q + 3) == 11 && value(w, q + 6).truthy() {
                out.push(Pair { eng: Eng::Cut(q, colon_at(w, q, pool)), thread: q + 6 });
            }
        }
    }
}

// ---- helpers ----

/// The done record of a thrown answer: [1, E] with E the thrown value slot at p.
fn thrown(answer: &[i64], p: usize) -> Vec<i64> {
    let mut rec = vec![1];
    rec.extend(slot(answer, p));
    rec
}

/// The id slot at p as a request table row (an absent id is sent as undefined).
fn put_id(tab: &mut Vec<i64>, w: &[i64], p: usize) {
    let [t, a, b] = slot(w, p);
    tab.extend([if t == 10 { 0 } else { t }, a, b]);
}

/// The temporary slot is exactly true.
fn temp_true(w: &[i64], p: usize) -> bool {
    rd(w, p + 3) == 3
}

// ---- op 7 ----
pub fn session_threads(cx: &mut Ctx) -> Result<(), Halt> {
    let w = &cx.blob;
    let starts = section_starts(w, 0);
    let mut pairs = Vec::new();
    ses_pairs(w, &starts, sec_start(w, 1), &mut pairs);
    let n = pairs.len();
    if n > 2796202 {
        return cx.too_big();
    }
    let mut pool = PoolBuf::default();
    let src = Pool(blob_pool(w));
    pool.append_span(src, 0, src.nbytes(), rd(src.0, 0).max(0) as usize);
    let codex: Vec<u16> = "codex".encode_utf16().collect();
    let codex_off = pool.append_units(&codex);
    let mut tab = Vec::with_capacity(6 * n);
    for pr in &pairs {
        match pr.eng {
            Eng::Codex => tab.extend([6, codex_off as i64, codex.len() as i64]),
            Eng::Cut(q, l) => tab.extend([6, rd(w, q + 1), l as i64]),
            Eng::Slot(e) => tab.extend(slot(w, e)),
        }
        tab.extend(slot(w, pr.thread));
    }
    let mut rec = vec![0; 4];
    put_vs(&mut rec, 1, 7, 4, n as i64);
    cx.set_done(Output { pool: pool.view_words(), rec, tab: Some(tab) });
    Ok(())
}

// ---- op 8 ----
pub fn excluded_from_bound(cx: &mut Ctx) -> Result<(), Halt> {
    let r0 = sec_start(&cx.blob, 1);
    if temp_true(&cx.blob, r0) {
        let mut rec = vec![0; 4];
        put_vs(&mut rec, 1, 3, 0, 0);
        cx.set_done(Output { pool: Vec::new(), rec, tab: None });
        return Ok(());
    }
    let mut tab = Vec::with_capacity(3);
    put_id(&mut tab, &cx.blob, r0);
    cx.obs(index_req(blob_pool(&cx.blob).to_vec(), 1, tab))?;
    let e = cx.ent();
    let rp = sec_start(e, 1);
    if rd(e, rp + 4) == 1 {
        let rec = thrown(e, rp + 5);
        cx.set_done(Output { pool: Vec::new(), rec, tab: None });
        return Ok(());
    }
    let base = sec_start(e, clampu(rd(e, rp + 2), 1000000));
    let mut rec = vec![0];
    rec.extend(slot(e, base));
    let pool = blob_pool(e).to_vec();
    cx.set_done(Output { pool, rec, tab: None });
    Ok(())
}

// ---- op 9 ----

/// The units of the engine part of the key.
fn eng_units(w: &[i64], pool: Pool, eng: Eng) -> Option<Vec<u16>> {
    match eng {
        Eng::Codex => Some("codex".encode_utf16().collect()),
        Eng::Cut(q, l) => Some(units(pool, rd(w, q + 1) as usize, l).collect()),
        Eng::Slot(e) => key(pool, value(w, e)),
    }
}

/// One id row per session that is not temporary, in session order; progress[0] = count, progress[1 + k] = the
/// logical position of row k's session.
pub fn index_collect(w: &[i64], base: usize, ns: usize, progress: &mut [i64]) -> Vec<i64> {
    let delta = rd(w, sec_start(w, 0)).clamp(0, MAX_WORDS) - ent_state::ent_packet::sec_len(w, 0) as i64;
    let mut ids = Vec::with_capacity(3 * ns);
    let mut nt = 0;
    for p in (0..ns).map(|i| base + 30 * i) {
        if !temp_true(w, p) {
            put_id(&mut ids, w, p);
            progress[nt + 1] = p as i64 + delta;
            nt += 1;
        }
    }
    progress[0] = nt as i64;
    ids
}

/// The positions recorded by index_collect still name non-temporary sessions of the packet at head, in order.
pub fn index_progress_ok(w: &[i64], head: usize, lb: i64, base: i64, progress: &[i64], nt: usize) -> bool {
    let nsec = rd(w, head + 6).clamp(0, 1000000) as usize;
    let pool = 7 + nsec;
    let words = rd(w, head + 7).clamp(0, MAX_WORDS);
    let units = rd(w, head + pool).clamp(0, MAX_WORDS);
    let delta = units - words;
    let rec = pool as i64 + units;
    if rec + 3 > lb || base < rec + 3 || base > lb {
        return false;
    }
    let ns = rd(w, head + pool + words as usize + 2);
    if ns < nt as i64 || ns > (lb - base) / 30 {
        return false;
    }
    let end = base + 30 * ns;
    let mut last = base - 1;
    for &p in &progress[1..=nt] {
        if p <= last || p < base || p >= end || (p - base) % 30 != 0 || temp_true(w, (head as i64 + p - delta) as usize) {
            return false;
        }
        last = p;
    }
    true
}

/// The excluded_from_bound request [8, array of nt ids]; the ids are the table section.
fn index_req(pool: Vec<i64>, nt: i64, tab: Vec<i64>) -> Request {
    Request { code: 8, aux: 1, pool, rec: index_request(nt), tab: Some(tab) }
}

pub fn index_request(nt: i64) -> Vec<i64> {
    let mut rq = vec![8, 0, 0, 0];
    put_vs(&mut rq, 1, 7, 4, nt);
    rq
}

/// The done packet of op 9: the map "engine:thread" -> session id of the sessions the host did not exclude; the first
/// write of a key wins.
pub fn index_complete(w: &[i64], head: usize, id: i64, seq: i64, answer: &[i64], progress: &[i64]) -> Vec<i64> {
    let nt = clampu(rd(progress, 0), 559240);
    let mut rbase = 0;
    if nt > 0 {
        let rp = sec_start(answer, 1);
        if rd(answer, rp + 4) == 1 {
            return mk_packet(3, id, seq, 9, &[], &thrown(answer, rp + 5), &[], &[], &[], 0);
        }
        rbase = sec_start(answer, clampu(rd(answer, rp + 2), 1000000));
    }
    let starts = section_starts(w, head);
    let ps = start_in(&starts, 0);
    let pool = Pool::at(w, ps);
    let pl = rd(w, ps).clamp(0, MAX_WORDS);
    let delta = pl - rd(w, head + 7).clamp(0, MAX_WORDS);
    // the candidate pairs of the sessions that are not excluded, each with its session
    let mut pairs: Vec<(Pair, usize)> = Vec::new();
    let mut ses = Vec::new();
    for a in 0..nt {
        if !value(answer, rbase + 3 * a).truthy() {
            let p = (head as i64 + rd(progress, a + 1).clamp(0, MAX_WORDS) - delta) as usize;
            ses.clear();
            ses_pairs(w, &starts, p, &mut ses);
            pairs.extend(ses.iter().map(|&pr| (pr, p)));
        }
    }
    if pairs.len() > 2796202 {
        return boundary(9, id, 1);
    }
    // first write wins: (key, session id slot) in first-occurrence order
    let mut index: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut kept: Vec<(Vec<u16>, usize)> = Vec::new();
    let mut sum = 0;
    for &(pr, p) in &pairs {
        let (Some(mut k), Some(t)) = (eng_units(w, pool, pr.eng), key(pool, value(w, pr.thread))) else {
            return boundary(9, id, 0);
        };
        k.push(b':' as u16);
        k.extend(t);
        sum += k.len();
        if sum > 16000000 {
            return boundary(9, id, 1);
        }
        // MUTANT M16: a later write overwrites the stored session (last write wins)
        index.entry(k.clone()).or_insert_with(|| {
            kept.push((k, p));
            kept.len() - 1
        });
    }
    let m = kept.len();
    if 16 + pl as usize + sum + 6 * m > MAX_WORDS as usize {
        return boundary(9, id, 1);
    }
    // the session id strings go to a compact pool when their units fit in the input pool's units, else the whole input
    // pool is kept and the id slots point into it
    let mut vsum = 0;
    for &(_, vp) in &kept {
        if let Value::Str(s) = value(w, vp) {
            if !ent_value::span_ok(pool, s.off as i64, s.len as i64) {
                return boundary(9, id, 0);
            }
            vsum += ent_value::units_in(pool, s.off, s.len);
        }
    }
    let full = vsum as i64 > pl;
    let mut values = PoolBuf::default();
    if full {
        values.append_span(pool, 0, pool.nbytes(), pl as usize);
    }
    let mut key_pool = PoolBuf::default();
    let mut tab = Vec::with_capacity(6 * m);
    let mut kslots = Vec::with_capacity(m);
    for (k, vp) in &kept {
        let at = key_pool.append_units(k);
        kslots.push((at, key_pool.nbytes - at));
        match value(w, *vp) {
            Value::Str(s) if !full => {
                let at = values.append_span(pool, s.off, s.len, ent_value::units_in(pool, s.off, s.len));
                tab.extend([0, 0, 0, 6, at as i64, s.len as i64]);
            }
            _ => {
                tab.extend([0, 0, 0]);
                tab.extend(slot(w, *vp));
            }
        }
    }
    let mut out = values.clone();
    let base = out.append_span(Pool(&key_pool.view_words()), 0, key_pool.nbytes, key_pool.units as usize);
    for (h, &(at, nb)) in kslots.iter().enumerate() {
        put_vs(&mut tab, 6 * h, 6, (base + at) as i64, nb as i64);
    }
    let mut rec = vec![0; 4];
    put_vs(&mut rec, 1, 7, 4, m as i64);
    mk_packet(3, id, seq, 9, &out.view_words(), &rec, &[], &[], &tab, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ent_state::ent_packet::mk_packet;
    use ent_value::encode_pool;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// A blob: pool, a 30-word session record, then the extra sections.
    fn blob(pool: &str, rec: &[i64], extra: &[i64]) -> Vec<i64> {
        let p = encode_pool(&u(pool)).view_words();
        // sections: pool, record, state, rows (extra rows as section 3)
        mk_packet(0, 7, 1, 7, &p, rec, &[], extra, &[], 0)
    }

    #[test]
    fn colon_cut() {
        let p = encode_pool(&u("claude:abc")).view_words();
        let w = [6, 0, 10];
        assert_eq!(colon_at(&w, 0, Pool(&p)), 6);
        let p2 = encode_pool(&u("plain")).view_words();
        assert_eq!(colon_at(&[6, 0, 5], 0, Pool(&p2)), 5);
    }

    #[test]
    fn pairs_in_order() {
        // session: externalThread {engine "x", thread "t", forks: none}, terminal {cliSession "s"}, engine null
        let mut rec = vec![0; 30];
        rec[6] = 1; // engine null
        rec[9] = 11;
        rec[12..15].copy_from_slice(&[6, 0, 1]); // "x"
        rec[15..18].copy_from_slice(&[6, 1, 1]); // "t"
        rec[21] = 11;
        rec[24..27].copy_from_slice(&[6, 2, 1]); // "s"
        let w = blob("xts", &rec, &[]);
        let starts = section_starts(&w, 0);
        let r0 = sec_start(&w, 1);
        let mut out = Vec::new();
        ses_pairs(&w, &starts, r0, &mut out);
        assert_eq!(out, vec![Pair { eng: Eng::Slot(r0 + 12), thread: r0 + 15 }, Pair { eng: Eng::Codex, thread: r0 + 24 }]);

        let mut cx = Ctx::new(7, 3, w, Vec::new(), &[0; 8]);
        session_threads(&mut cx).unwrap();
        let o = cx.out.as_ref().unwrap();
        assert_eq!(o.rec, vec![0, 7, 4, 2]);
        let tab = o.tab.as_ref().unwrap();
        assert_eq!(&tab[..6], &[6, 0, 1, 6, 1, 1]);
        assert_eq!(&tab[6..], &[6, 3, 5, 6, 2, 1]);
    }

    #[test]
    fn temporary_is_not_excluded() {
        let mut rec = vec![0; 30];
        rec[3] = 3;
        let mut cx = Ctx::new(8, 3, blob("", &rec, &[]), Vec::new(), &[0; 8]);
        excluded_from_bound(&mut cx).unwrap();
        assert_eq!(cx.out.as_ref().unwrap().rec, vec![0, 3, 0, 0]);
    }

    #[test]
    fn excluded_stalls_on_request() {
        let mut rec = vec![0; 30];
        rec[0] = 10; // absent id is sent as undefined
        let mut cx = Ctx::new(8, 3, blob("", &rec, &[]), Vec::new(), &[0; 8]);
        assert_eq!(excluded_from_bound(&mut cx), Err(Halt::Stalled));
        let rq = cx.req.as_ref().unwrap();
        assert_eq!((rq.code, rq.aux), (8, 1));
        assert_eq!(rq.rec, vec![8, 7, 4, 1]);
        assert_eq!(rq.tab.as_deref(), Some(&[0, 0, 0][..]));
    }

    #[test]
    fn collect_skips_temporary() {
        let mut w = vec![0; 90];
        w[3] = 3; // session 0 temporary
        w[30..33].copy_from_slice(&[5, 1, 0]);
        w[60..63].copy_from_slice(&[10, 0, 0]);
        let mut progress = vec![0; 4];
        let ids = index_collect(&w, 0, 3, &mut progress);
        assert_eq!(ids, vec![5, 1, 0, 0, 0, 0]);
        assert_eq!(progress[0], 2);
    }

    #[test]
    fn first_write_wins() {
        // two sessions with the same terminal pair (engine "e", cliSession "s"); ids "A" and "B"
        let pool = encode_pool(&u("esAB")).view_words();
        let ses = |idoff: i64| {
            let mut r = vec![0; 30];
            r[0..3].copy_from_slice(&[6, idoff, 1]);
            r[6..9].copy_from_slice(&[6, 0, 1]);
            r[21] = 11;
            r[24..27].copy_from_slice(&[6, 1, 1]);
            r
        };
        let mut rows = ses(2);
        rows.extend(ses(3));
        let w = mk_packet(0, 9, 1, 9, &pool, &[0, 0, 0], &[], &rows, &[], 0);
        // progress holds logical positions (pool counted at its logical size)
        let base = sec_start(&w, 3) as i64 + rd(&w, sec_start(&w, 0)) - ent_state::ent_packet::sec_len(&w, 0) as i64;
        let progress = [2, base, base + 30];
        // answer: [0, rows section 3] with two falsy (not excluded) slots
        let answer = mk_packet(2, 9, 1, 9, &[], &[0, 0, 3, 0, 0, 0, 0, 0], &[], &[2, 0, 0, 2, 0, 0], &[], 0);
        let out = index_complete(&w, 0, 9, 2, &answer, &progress);
        let tab = &out[sec_start(&out, 4)..];
        assert_eq!(tab.len(), 6);
        // value: "A" in the compact pool at 0; key "e:s" after it
        assert_eq!(&tab[3..], &[6, 0, 1]);
        assert_eq!(&tab[..3], &[6, 1, 3]);
    }
}

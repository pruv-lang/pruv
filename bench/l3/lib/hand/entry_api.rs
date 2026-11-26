// Hand Rust baseline for `bench/l3/lib/entry_api.pruv` (Rust-shape reset step 3.7).
// The packet layer of the entry: begin / step of ops 2..8 and the two round trips of op 9. A begin checks the
// envelope and the record, repacks the input pool to the referenced prefix and runs the item on a fresh `Ctx`; a step
// checks prev and the completion, parses prev's state words once (`State::parse`, `replay_ok`), appends the completion
// to the log and re-runs the item. Every item is `fn(&mut Ctx) -> Result<(), Halt>`; the packet comes from
// `Ctx::finish`. Ops 1 and 6 have their own begin / step (ent_sources, ent_canon); op 5 never awaits.
// The crate root of the shared build is `entry_root.rs` (step 3.8): the item modules use its single `ent_state` (cfg
// `shared_ctx`), so one `Ctx` type runs every item. Each item module still checks alone without the cfg.
// Check (from entry_root.rs): rustc --edition 2021 --cfg shared_ctx --test entry_root.rs -o /tmp/entry_root_hand

#![allow(dead_code)]

use crate::{ent_accounts, ent_bound, ent_canon, ent_dispatch, ent_sources, ent_state};

use ent_bound::{excluded_from_bound, index_collect, index_complete, index_progress_ok, index_request, session_threads};
use ent_accounts::account_home_begin;
use ent_canon::{codex_folder_begin, codex_folder_step};
use ent_dispatch::{find_source, list_external, read_external};
use ent_sources::{import_begin, import_step};
use ent_state::ent_packet::ent_wire::same_packet;
use ent_state::ent_packet::{boundary, comp_check, env_check, env_check_wire, input_check, mk_packet, put_vs, sec_len, sec_start, Reject};
use ent_state::ent_value::{encode_pool, span_ok, units_in, Pool, PoolBuf, NB_MAX};
use ent_state::{blob_pool, ent_value_pkt_size as pkt_size, log_append, log_words, repack, Ctx, Halt, State, MAGIC};

const MAX_WORDS: usize = 1 << 24;
/// op 9: at most this many sessions (3 id words each must fit the packet)
const MAX_SESSIONS: i64 = 559240;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

/// The boundary for packet w: an envelope InputTooLarge is boundary code 1, anything else InvalidPacket (0).
fn bad_pkt(w: &[i64], code: Reject) -> Vec<i64> {
    let c = if code == Reject::InputTooLarge { 1 } else { 0 };
    boundary(rd(w, 5), rd(w, 2), c)
}

/// The pool units the packet's string slots count (the logical pool size).
fn pool_units(w: &[i64]) -> usize {
    rd(w, sec_start(w, 0)).clamp(0, MAX_WORDS as i64) as usize
}

/// The units of the first `cut` bytes of w's pool (all of them when the cut is the whole pool).
fn prefix_units(w: &[i64], cut: i64) -> usize {
    let s0 = sec_start(w, 0);
    let pool = Pool::at(w, s0);
    let nb = pool.nbytes();
    let cut = cut.clamp(0, nb as i64) as usize;
    if cut == nb {
        return rd(w, s0).clamp(0, nb as i64) as usize;
    }
    units_in(pool, 0, cut)
}

fn run_item(cx: &mut Ctx) -> Result<(), Halt> {
    match cx.op {
        2 => {
            find_source(cx);
            Ok(())
        }
        3 => list_external(cx),
        4 => read_external(cx),
        7 => session_threads(cx),
        8 => excluded_from_bound(cx),
        _ => cx.fail(),
    }
}

pub fn entry_begin(input: &[i64]) -> Vec<i64> {
    entry_begin_with_env(input, env_check(input))
}

pub fn entry_begin_with_env(input: &[i64], ec: Result<(), Reject>) -> Vec<i64> {
    if let Err(e) = ec {
        return bad_pkt(input, e);
    }
    let (op, id) = (rd(input, 5), rd(input, 2));
    if rd(input, 1) != 0 || rd(input, 3) != 0 || op == 9 {
        return bad_pkt(input, Reject::InvalidPacket);
    }
    match op {
        1 => return import_begin(input),
        5 => return account_home_begin(input),
        6 => return codex_folder_begin(input),
        _ => {}
    }
    let Some(hi) = input_check(input) else { return bad_pkt(input, Reject::InvalidPacket) };
    let mut cx = Ctx::new(op, id, repack(input, hi), Vec::new(), &[0; 8]);
    let _ = run_item(&mut cx);
    cx.finish(1, 0)
}

pub fn entry_step(prev: &[i64], completion: &[i64]) -> Vec<i64> {
    entry_step_at(prev, completion, false)
}

pub fn entry_step_at(prev: &[i64], completion: &[i64], wire: bool) -> Vec<i64> {
    let e1 = if wire { env_check_wire(prev) } else { env_check(prev) };
    if let Err(e) = e1 {
        return bad_pkt(prev, e);
    }
    let e2 = if wire { env_check_wire(completion) } else { env_check(completion) };
    if let Err(e) = e2 {
        return bad_pkt(prev, e);
    }
    let (op, id, seq) = (rd(prev, 5), rd(prev, 2), rd(prev, 3));
    if rd(prev, 1) != 1 || rd(completion, 1) != 2 {
        return bad_pkt(prev, Reject::InvalidPacket);
    }
    if rd(completion, 2) != id || rd(completion, 3) != seq || rd(completion, 5) != op || seq >= 1 << 53 {
        return bad_pkt(prev, Reject::InvalidPacket);
    }
    match op {
        1 => return import_step(prev, completion),
        6 => return codex_folder_step(prev, completion),
        2 | 5 | 7 | 9 => return bad_pkt(prev, Reject::InvalidPacket),
        _ => {}
    }
    let s2 = sec_start(prev, 2).min(prev.len());
    let st = &prev[s2..(s2 + sec_len(prev, 2)).min(prev.len())];
    let Some(s) = State::parse(st, op, id).filter(|s| s.replay_ok(op, seq)) else { return bad_pkt(prev, Reject::InvalidPacket) };
    let Some(hi) = comp_check(completion, s.code, s.aux) else { return bad_pkt(prev, Reject::InvalidPacket) };
    let lg = log_append(s.log, s.code, s.aux, completion, hi);
    if log_words(&lg) > MAX_WORDS {
        return bad_pkt(prev, Reject::InputTooLarge);
    }
    let mut cx = Ctx::new(op, id, s.blob.to_vec(), lg, s.ck);
    cx.with_prev(prev, true);
    let _ = run_item(&mut cx);
    cx.finish(seq + 1, seq)
}

// ---- op 9: one await (the excluded_from_bound request); the state section keeps the v1 wire format ----

/// The caller keeps the input after the call and passes it to index_step.
pub fn index_begin(progress: &mut [i64], input: &[i64]) -> Vec<i64> {
    if let Some(p) = progress.first_mut() {
        *p = 0;
    }
    if let Err(e) = env_check_wire(input) {
        return bad_pkt(input, e);
    }
    if rd(input, 1) != 0 || rd(input, 3) != 0 || rd(input, 5) != 9 {
        return bad_pkt(input, Reject::InvalidPacket);
    }
    let Some(hi) = input_check(input) else { return bad_pkt(input, Reject::InvalidPacket) };
    let raw = pkt_size(input);
    if hi != Pool::at(input, sec_start(input, 0)).nbytes() as i64 {
        return index_begin_checked(progress, &repack(input, hi), raw);
    }
    index_begin_checked(progress, input, raw)
}

/// The await state stores the logical pool size and the input size: [magic, 9, 8, nt, base, 0 x 7, pl, raw].
fn index_begin_checked(progress: &mut [i64], blob: &[i64], raw: usize) -> Vec<i64> {
    let rec = sec_start(blob, 1);
    let sec = rd(blob, rec + 1).clamp(0, 1_000_000) as usize;
    let ns = rd(blob, rec + 2).clamp(0, MAX_SESSIONS) as usize;
    if progress.len() <= ns {
        return bad_pkt(blob, Reject::InvalidPacket);
    }
    let base = sec_start(blob, sec);
    let mut tab = index_collect(blob, base, ns, progress);
    let nt = progress[0];
    let id = rd(blob, 2);
    if nt == 0 {
        return index_complete(blob, 0, id, 0, &[], progress);
    }
    let pool = Pool::at(blob, sec_start(blob, 0));
    let pl = pool_units(blob);
    let (lb, lt) = (pkt_size(blob), tab.len());
    if lb > MAX_WORDS || lt > MAX_WORDS || 30 + pl + lb + lt > MAX_WORDS {
        return bad_pkt(blob, Reject::InputTooLarge);
    }
    // the compact / full choice counts UTF-16 units
    let mut units = 0;
    for slot in tab.chunks_exact(3) {
        if units > pl {
            break;
        }
        if slot[0] == 6 {
            if !span_ok(pool, slot[1], slot[2]) {
                return bad_pkt(blob, Reject::InvalidPacket);
            }
            let u = units_in(pool, slot[1] as usize, slot[2] as usize);
            if u > pl {
                return bad_pkt(blob, Reject::InvalidPacket);
            }
            units += u;
        }
    }
    // the id strings go to a compact pool (the slots are rewritten in place), else the whole input pool is kept
    let strings = if units <= pl {
        let mut compact = PoolBuf { words: Vec::with_capacity((4 * units).div_ceil(7)), ..PoolBuf::default() };
        for j in (0..tab.len() / 3).map(|k| 3 * k) {
            if tab[j] == 6 {
                let off = tab[j + 1].clamp(0, NB_MAX as i64) as usize;
                let nb = (tab[j + 2].clamp(0, NB_MAX as i64) as usize).min(pool.nbytes().saturating_sub(off));
                let at = compact.append_span(pool, off, nb, units_in(pool, off, nb));
                put_vs(&mut tab, j, 6, at as i64, nb as i64);
            }
        }
        compact.view_words()
    } else {
        blob_pool(blob).to_vec()
    };
    let delta = pl as i64 - sec_len(blob, 0) as i64;
    let st = vec![MAGIC, 9, 8, nt, base as i64 + delta, 0, 0, 0, 0, 0, 0, 0, pl as i64, raw as i64];
    mk_packet(1, id, 1, 9, &strings, &index_request(nt), &st, &[], &tab, 1)
}

/// The caller keeps the emitted packet and the input given to index_begin; the input is read in place, or its repack
/// when begin repacked it. prev is passed separately so a change outside the state is rejected too.
pub fn index_step(progress: &mut [i64], input: &[i64], prev: &[i64], emitted: &[i64], completion: &[i64]) -> Vec<i64> {
    let nt = rd(progress, 0);
    let n = emitted.len();
    if nt <= 0 || nt > MAX_SESSIONS || nt as usize >= progress.len() || !same_packet(prev, emitted) {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    if n < 30 || pkt_size(emitted) > MAX_WORDS || emitted[6] != 5 {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    let (st, ls) = (sec_start(emitted, 2), sec_len(emitted, 2));
    if ls != 14 || st > MAX_WORDS || st + ls > n {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    let raw = pkt_size(input);
    if raw > MAX_WORDS || emitted[st + 13] != raw as i64 {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    if emitted[8] != 4 || rd(emitted, st - 1) != nt || emitted[11] != 3 * nt {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    let hi = emitted[st + 12];
    if hi != pool_units(input) as i64 {
        let Some(cut) = input_check(input) else { return bad_pkt(emitted, Reject::InvalidPacket) };
        let blob = repack(input, cut);
        if hi != pool_units(&blob) as i64 {
            return bad_pkt(emitted, Reject::InvalidPacket);
        }
        return index_step_on(progress, &blob, emitted, st, nt as usize, completion);
    }
    index_step_on(progress, input, emitted, st, nt as usize, completion)
}

fn index_step_on(progress: &mut [i64], blob: &[i64], emitted: &[i64], st: usize, nt: usize, completion: &[i64]) -> Vec<i64> {
    let lb = pkt_size(blob);
    if lb > MAX_WORDS {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    let base = rd(emitted, st + 4);
    if !index_progress_ok(blob, 0, lb as i64, base, progress, nt) {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    if let Err(e) = env_check_wire(completion) {
        return bad_pkt(emitted, e);
    }
    let id = rd(emitted, 2);
    if rd(completion, 1) != 2 || rd(completion, 2) != id || rd(completion, 3) != 1 || rd(completion, 5) != 9 {
        return bad_pkt(emitted, Reject::InvalidPacket);
    }
    let Some(hi0) = comp_check(completion, 8, nt as i64) else { return bad_pkt(emitted, Reject::InvalidPacket) };
    let hi = prefix_units(completion, hi0);
    let l = pkt_size(completion).min(MAX_WORDS);
    if l as i64 - pool_units(completion) as i64 + hi as i64 + 3 > MAX_WORDS as i64 {
        return bad_pkt(emitted, Reject::InputTooLarge);
    }
    let done = index_complete(blob, 0, id, 1, completion, progress);
    progress[0] = 0;
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn pool(s: &str) -> Vec<i64> {
        encode_pool(&u(s)).view_words()
    }

    /// A packet with exactly these sections (pool, record, tables): every table section must be claimed by a slot.
    fn pkt(kind: i64, id: i64, seq: i64, op: i64, secs: &[&[i64]]) -> Vec<i64> {
        let body: usize = secs.iter().map(|s| s.len()).sum();
        let mut w = vec![1, kind, id, seq, (2 + secs.len() + body) as i64, op, secs.len() as i64];
        w.extend(secs.iter().map(|s| s.len() as i64));
        secs.iter().for_each(|s| w.extend_from_slice(s));
        w
    }

    fn kind(w: &[i64]) -> i64 {
        rd(w, 1)
    }

    /// The record of a packet (section 1).
    fn rec(w: &[i64]) -> &[i64] {
        let s = sec_start(w, 1);
        &w[s..s + sec_len(w, 1)]
    }

    #[test]
    fn find_runs_to_done() {
        // query "a"; rows "b", "a": the first equal row is 1
        let input = pkt(0, 7, 0, 2, &[&pool("ab"), &[6, 0, 1, 7, 2, 2], &[6, 1, 1, 6, 0, 1]]);
        let out = entry_begin(&input);
        assert_eq!((kind(&out), rd(&out, 2), rd(&out, 3), rd(&out, 5)), (3, 7, 0, 2));
        assert_eq!(rec(&out), &[0, 4, 1, 0]);
    }

    #[test]
    fn begin_rejects() {
        let input = pkt(0, 7, 0, 2, &[&pool("ab"), &[6, 0, 1, 7, 2, 2], &[6, 1, 1, 6, 0, 1]]);
        // op 9 only begins through index_begin; a kind or seq other than 0 is not a begin
        let mut op9 = input.clone();
        op9[5] = 9;
        assert_eq!(entry_begin(&op9), boundary(9, 7, 0));
        let mut k = input.clone();
        k[1] = 1;
        assert_eq!(entry_begin(&k), boundary(2, 7, 0));
        assert_eq!(entry_begin_with_env(&input, Err(Reject::InputTooLarge)), boundary(2, 7, 1));
        // a record of the wrong length
        let short = pkt(0, 7, 0, 2, &[&pool("ab"), &[6, 0, 1]]);
        assert_eq!(entry_begin(&short), boundary(2, 7, 0));
    }

    #[test]
    fn account_home_done() {
        let id = "acc_0123456789abcdef";
        let p = pool(&format!("/dclaude{id}"));
        let input = pkt(0, 4, 0, 5, &[&p, &[6, 0, 2, 6, 2, 6, 6, 8, 20]]);
        let out = entry_begin(&input);
        assert_eq!(kind(&out), 3);
        let r = rec(&out);
        assert_eq!(&r[..2], &[0, 6]);
        let s0 = sec_start(&out, 0);
        let home: Vec<u16> = ent_state::ent_value::decode_span(Pool::at(&out, s0), r[2] as usize, r[3] as usize);
        assert_eq!(String::from_utf16(&home).unwrap(), format!("/d/engine-homes/claude/{id}"));
        let bad = pkt(0, 4, 0, 5, &[&p, &[6, 0, 2, 6, 2, 6, 6, 8, 19]]);
        assert_eq!(rec(&entry_begin(&bad)), &[3, 0]);
    }

    /// An op 3 begin: engine "codex", folder ref, options undefined, signal ref.
    fn list_input() -> Vec<i64> {
        pkt(0, 5, 0, 3, &[&pool("codex"), &[6, 0, 5, 9, 1, 0, 0, 0, 0, 9, 2, 0]])
    }

    #[test]
    fn list_stalls_then_steps() {
        let a = entry_begin(&list_input());
        assert_eq!((kind(&a), rd(&a, 3)), (1, 1));
        assert_eq!(&rec(&a)[..3], &[9, 1, 0]);
        // the answer: no rows
        let comp = pkt(2, 5, 1, 3, &[&[0, 0], &[0, 7, 2, 0], &[]]);
        let d = entry_step(&a, &comp);
        assert_eq!((kind(&d), rd(&d, 3)), (3, 1));
        assert_eq!(rec(&d), &[0, 7, 4, 0]);
        // the same completion against a changed state is rejected
        let mut bad = a.clone();
        let s2 = sec_start(&a, 2);
        bad[s2 + 4] = 7;
        assert_eq!(entry_step(&bad, &comp), boundary(3, 5, 0));
        // a completion for another sequence number is rejected
        let late = pkt(2, 5, 2, 3, &[&[0, 0], &[0, 7, 2, 0], &[]]);
        assert_eq!(entry_step(&a, &late), boundary(3, 5, 0));
    }

    #[test]
    fn step_rejects_ops_without_await() {
        let mut a = entry_begin(&list_input());
        let comp = pkt(2, 5, 1, 3, &[&[0, 0], &[0, 7, 2, 0], &[]]);
        for op in [2, 5, 7, 9] {
            a[5] = op;
            let mut c = comp.clone();
            c[5] = op;
            assert_eq!(entry_step(&a, &c), boundary(op, 5, 0));
        }
    }

    /// Session record: id {6} at `idoff` (1 unit), terminal {cliSession "s"}, engine "e".
    fn session(idoff: i64) -> Vec<i64> {
        let mut r = vec![0; 30];
        r[0..3].copy_from_slice(&[6, idoff, 1]);
        r[3..6].copy_from_slice(&[0, 0, 0]);
        r[6..9].copy_from_slice(&[6, 0, 1]);
        r[9] = 10;
        r[21] = 11;
        r[24..27].copy_from_slice(&[6, 1, 1]);
        r[27] = 10;
        r
    }

    #[test]
    fn index_round_trip() {
        let mut rows = session(2);
        rows.extend(session(3));
        let input = pkt(0, 9, 0, 9, &[&pool("esAB"), &[7, 2, 2], &rows]);
        let mut progress = vec![0; 3];
        let a = index_begin(&mut progress, &input);
        assert_eq!((kind(&a), rd(&a, 3), progress[0]), (1, 1, 2));
        assert_eq!(rec(&a), &[8, 7, 4, 2]);
        // compact ids: "A" at 0, "B" at 1
        let tab = &a[sec_start(&a, 4)..];
        assert_eq!(tab, &[6, 0, 1, 6, 1, 1]);
        // the answer: session 0 excluded (true), session 1 not (false)
        let mut crec = vec![0, 7, 2, 2, 0];
        crec.resize(23, 0);
        let comp = pkt(2, 9, 1, 9, &[&[0, 0], &crec, &[3, 0, 0, 2, 0, 0]]);
        let d = index_step(&mut progress, &input, &a, &a, &comp);
        assert_eq!((kind(&d), progress[0]), (3, 0));
        assert_eq!(rec(&d), &[0, 7, 4, 1]);
        // a changed emitted packet (not the one prev names) is rejected
        let mut p2 = vec![0; 3];
        index_begin(&mut p2, &input);
        let mut e2 = a.clone();
        e2[sec_start(&a, 2) + 4] += 30;
        assert_eq!(index_step(&mut p2, &input, &a, &e2, &comp), boundary(9, 9, 0));
    }
}

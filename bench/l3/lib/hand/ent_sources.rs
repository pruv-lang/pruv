// Hand Rust baseline for `bench/l3/lib/ent_sources.pruv` (Rust-shape reset step 3.6).
// Op 1 import_sources (reference/backend-engine-import.ts importSources). The work is a chain of items (ck0 phase 0..9,
// ck1 account index, ck2 account count). An item re-runs from scratch against the log of its own answers; every item
// takes the replay context `&mut Ctx` plus the rows of this step `&mut Rows` and returns `Result<(), Halt>`: `Ok` means
// the item is complete (its row, if any, is in `rows`, ck names the next item), `Err` stops the step (a stall, a
// protocol error, or `Done`: the import result is set). A completed item is followed by the next one in the same Ctx
// (`next_item`: the log is dropped). Rows are appended straight to one pool; their string offsets are placed after the
// request / output pool once, when the packet is built.
// Check: rustc --edition 2021 --test ent_sources.rs -o /tmp/ent_sources_hand && /tmp/ent_sources_hand

#![allow(dead_code)]

#[cfg(not(shared_ctx))]
#[path = "ent_canon.rs"]
pub mod ent_canon;
#[cfg(shared_ctx)]
pub use crate::ent_canon;

use ent_canon::ent_path::join;
use ent_canon::ent_state::ent_packet::{boundary, comp_check, mk_packet, put_vs, sec_len, sec_start};
use ent_canon::ent_state::ent_value::{Pool, PoolBuf};
use ent_canon::ent_state::{ck_phase_only, log_append, log_words, repack, s0_of, Ctx, Fault, Halt, Output};
use ent_canon::ent_accounts::{account_home, io_check, valid_account_id};
use ent_canon::{codex_home, home_dir, home_join, is_codex_home, lit, rethrow_answer, state_ok_ie, str_at};

const SLASH: u16 = b'/' as u16;
const MAX_WORDS: usize = 1 << 24;
const MAX_ITEMS: usize = 64;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

/// The rows completed in this step: one pool and the 18-word records.
#[derive(Default)]
pub struct Rows {
    pub pool: PoolBuf,
    pub rec: Vec<i64>,
}

/// The account name: a string or any other value slot (tag, a, b).
pub enum Name {
    Str(Vec<u16>),
    Raw(i64, i64, i64),
}

pub struct Account {
    pub id: Vec<u16>,
    pub name: Name,
}

/// pruv `ent_value::pool_put`: append s to the pool and put its string slot at rec[at].
fn pool_put(pool: &mut PoolBuf, rec: &mut [i64], at: usize, s: &[u16]) {
    let off = pool.append_units(s);
    put_vs(rec, at, 6, off as i64, (pool.nbytes - off) as i64);
}

/// The item is complete: ck names the next one (the climb words ck3..7 belong to one item and start cleared).
fn advance(cx: &mut Ctx, ph: i64, idx: i64, cnt: i64) {
    cx.ck = [ph, idx, cnt, 0, 0, 0, 0, 0];
}

/// ck word i as a loop bound
fn ck_word(cx: &Ctx, i: usize) -> i64 {
    cx.ck[i].clamp(0, 1 << 24)
}

fn rec0(cx: &Ctx) -> usize {
    sec_start(&cx.blob, 1)
}

/// trimmed env slot (empty for a non-string slot); slot k is at record offset 3k
fn env_trim(cx: &Ctx, k: usize) -> Vec<u16> {
    let p = rec0(cx) + 3 * k.min(16);
    if rd(&cx.blob, p) == 6 {
        return ent_canon::ent_u16::trim(&str_at(&cx.blob, p));
    }
    Vec::new()
}

/// the start of a row: source | engine | kind
fn row_head(rows: &mut Rows, rec: &mut [i64], src: &[u16], eng: &str, kind: &str) {
    pool_put(&mut rows.pool, rec, 0, src);
    pool_put(&mut rows.pool, rec, 3, &lit(eng));
    pool_put(&mut rows.pool, rec, 6, &lit(kind));
}

/// rows.rec ++ rec (pruv guards the arr_append bound 2^40; row LB)
fn push_row(rows: &mut Rows, rec: &[i64]) {
    rows.rec.extend_from_slice(rec);
}

/// a home row: no account and no name
fn home_row(rows: &mut Rows, src: &str, eng: &str, home: &[u16]) {
    let mut rec = vec![0; 18];
    row_head(rows, &mut rec, &lit(src), eng, "home");
    put_vs(&mut rec, 9, 10, 0, 0);
    put_vs(&mut rec, 12, 10, 0, 0);
    pool_put(&mut rows.pool, &mut rec, 15, home);
    push_row(rows, &rec);
}

fn account_row(rows: &mut Rows, src: &[u16], eng: &str, a: &Account, home: &[u16]) {
    let mut rec = vec![0; 18];
    row_head(rows, &mut rec, src, eng, "account");
    pool_put(&mut rows.pool, &mut rec, 9, &a.id);
    match &a.name {
        Name::Str(u) => pool_put(&mut rows.pool, &mut rec, 12, u),
        Name::Raw(t, x, y) => put_vs(&mut rec, 12, *t, *x, *y),
    }
    pool_put(&mut rows.pool, &mut rec, 15, home);
    push_row(rows, &rec);
}

/// trimmed env slot, else join(homedir(), seg)
fn env_or_home(cx: &mut Ctx, slot: usize, seg: &str) -> Result<Vec<u16>, Halt> {
    let t = env_trim(cx, slot);
    if !t.is_empty() {
        return Ok(t);
    }
    home_join(cx, seg)
}

fn home_item(cx: &mut Ctx, rows: &mut Rows, slot: usize, seg: &str, src: &str, eng: &str, next: i64) -> Result<(), Halt> {
    let home = env_or_home(cx, slot, seg)?;
    home_row(rows, src, eng, &home);
    advance(cx, next, 0, 0);
    Ok(())
}

/// accounts(eng): the count goes to the next phase; a throw is rethrown
fn accounts_item(cx: &mut Ctx, eng: i64, next: i64) -> Result<(), Halt> {
    cx.accounts(eng)?;
    if cx.ent_r(0) == 1 {
        return Err(rethrow_answer(cx));
    }
    let cnt = cx.ent_r(1).clamp(0, 1 << 24);
    advance(cx, next, 0, cnt);
    Ok(())
}

/// the account row of the last account_at answer: a nullish row or an invalid id is skipped
fn account_ok(cx: &Ctx) -> bool {
    let e = cx.ent();
    cx.ent_r(1) != 0 && valid_account_id(e, sec_start(e, 1) + 2)
}

/// id and name of the last account_at answer
fn account_of(cx: &Ctx) -> Account {
    let e = cx.ent();
    let rp = sec_start(e, 1);
    let name = if rd(e, rp + 5) == 6 { Name::Str(str_at(e, rp + 5)) } else { Name::Raw(rd(e, rp + 5), rd(e, rp + 6), rd(e, rp + 7)) };
    Account { id: str_at(e, rp + 2), name }
}

// ---- Claude accounts: one account_at per item ----
fn claude_acct(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    let (idx, cnt) = (ck_word(cx, 1), ck_word(cx, 2));
    if idx >= cnt {
        advance(cx, 3, 0, 0);
        return Ok(());
    }
    cx.account_at(0, idx)?;
    // MUTANT M05: an invalid id (or nullish row) aborts the import (throws) instead of being skipped
    if !account_ok(cx) {
        advance(cx, 2, idx + 1, cnt);
        return Ok(());
    }
    let a = account_of(cx);
    let home = account_home(&str_at(&cx.blob, rec0(cx)), &lit("claude"), &a.id);
    account_row(rows, &[&lit("claude:")[..], &a.id].concat(), "claude", &a, &home);
    advance(cx, 2, idx + 1, cnt);
    Ok(())
}

// ---- Codex accounts: the body (codexAccountFolder) may observe several times; any throw in it skips the account ----
fn codex_acct(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    let (idx, cnt) = (ck_word(cx, 1), ck_word(cx, 2));
    if idx >= cnt {
        advance(cx, 6, 0, 0);
        return Ok(());
    }
    cx.account_at(1, idx)?;
    if !account_ok(cx) {
        advance(cx, 5, idx + 1, cnt);
        return Ok(());
    }
    // everything needed from the account_at answer is taken before the body observes again
    let a = account_of(cx);
    let expected = account_home(&str_at(&cx.blob, rec0(cx)), &lit("codex"), &a.id);
    match is_codex_home(cx, &expected) {
        Ok(false) => {}
        // a host home, or a throw in the body (rethrown as a result that this item drops)
        Ok(true) | Err(Halt::Done) => {
            advance(cx, 5, idx + 1, cnt);
            return Ok(());
        }
        Err(h) => return Err(h),
    }
    // MUTANT M04: a Codex row whose id equals the previously emitted Codex id is skipped (deduplicated; id hash in ck6)
    account_row(rows, &[&lit("codex:")[..], &a.id].concat(), "codex", &a, &expected);
    advance(cx, 5, idx + 1, cnt);
    Ok(())
}

// ---- Codex home: hostCodexHome() reads the AMBIENT CODEX_HOME ----
fn codex_home_item(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    // MUTANT M02: reads the input env slot CODEX_HOME (record offset 18) instead of the ambient request
    let home = codex_home(cx)?;
    home_row(rows, "codex:home", "codex", &home);
    advance(cx, 4, 0, 0);
    Ok(())
}

fn opencode_item(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    let xdg = env_trim(cx, 3);
    // MUTANT M03: the starts-with '/' check is dropped (a relative XDG_DATA_HOME is used)
    if !xdg.is_empty() && xdg[0] == SLASH {
        home_row(rows, "opencode:home", "opencode", &join(&[&xdg, &lit("opencode")]));
    } else {
        let home = home_dir(cx)?;
        home_row(rows, "opencode:home", "opencode", &join(&[&home, &lit(".local"), &lit("share"), &lit("opencode")]));
    }
    advance(cx, 8, 0, 0);
    Ok(())
}

/// Qwen: homedir() is evaluated first (eager argument), even when QWEN_RUNTIME_DIR is set
fn qwen_item(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    let home = home_dir(cx)?;
    let rt = env_trim(cx, 4);
    let qh = env_trim(cx, 5);
    if !rt.is_empty() {
        home_row(rows, "qwen:home", "qwen", &rt);
    } else if !qh.is_empty() {
        home_row(rows, "qwen:home", "qwen", &qh);
    } else {
        home_row(rows, "qwen:home", "qwen", &join(&[&home, &lit(".qwen")]));
    }
    advance(cx, 9, 0, 0);
    Ok(())
}

fn phase_item(cx: &mut Ctx, rows: &mut Rows) -> Result<(), Halt> {
    match cx.ck[0] {
        0 => home_item(cx, rows, 1, ".claude", "claude:home", "claude", 1),
        1 => accounts_item(cx, 0, 2),
        2 => claude_acct(cx, rows),
        3 => codex_home_item(cx, rows),
        4 => accounts_item(cx, 1, 5),
        5 => codex_acct(cx, rows),
        6 => home_item(cx, rows, 2, ".grok", "grok:home", "grok", 7),
        7 => opencode_item(cx, rows),
        8 => qwen_item(cx, rows),
        9 => {
            cx.set_done(Output { pool: Vec::new(), rec: vec![0], tab: None });
            Err(Halt::Done)
        }
        _ => cx.fail(),
    }
}

/// add to the pool offset of every string slot of the rows (3-word slots)
fn rebase(rec: &mut [i64], add: i64) {
    for s in rec.chunks_exact_mut(3) {
        if s[0] == 6 {
            s[1] = s[1].clamp(0, 1 << 30) + add.clamp(0, 1 << 30);
        }
    }
}

/// front ++ the rows' pool; the row records are rebased to their place in it
fn place_rows(front: &[i64], rows: &mut Rows) -> Vec<i64> {
    let f = Pool(front);
    let mut pool = PoolBuf::default();
    pool.append_span(f, 0, f.nbytes(), rd(front, 0).max(0) as usize);
    let rp = rows.pool.view_words();
    let r = Pool(&rp);
    let base = pool.append_span(r, 0, r.nbytes(), rows.pool.units.max(0) as usize);
    rebase(&mut rows.rec, base as i64);
    pool.view_words()
}

/// s0 = the seq of the first request of the current item (R1: the await state keeps it in ck0 as phase + 16 * s0, so
/// that the continuity of start seq + dropped entries + kept log + current request can be checked). An item that
/// completes in this step is followed by one whose first request is the one this step emits (aseq).
fn drive(cx: &mut Ctx, aseq: i64, dseq: i64, s0: i64) -> Vec<i64> {
    let mut s0 = s0;
    let mut rows = Rows::default();
    for _ in 0..MAX_ITEMS {
        let r = phase_item(cx, &mut rows);
        // R1(c): an item that ends without a stall or a fault has consumed the whole log
        if cx.req.is_none() && cx.fault.is_none() && cx.pos != cx.lg.len() {
            let _ = cx.fail();
            break;
        }
        if r.is_err() {
            break;
        }
        cx.next_item();
        s0 = aseq;
    }
    match cx.fault {
        Some(Fault::TooBig) => return boundary(1, cx.op_id, 1),
        Some(Fault::Protocol) => return boundary(1, cx.op_id, 0),
        None => {}
    }
    if let Some(req) = cx.req.take() {
        if aseq >= 1 << 28 || s0 >= 1 << 28 {
            return boundary(1, cx.op_id, 1);
        }
        cx.ck[0] = cx.ck[0].clamp(0, 15) + 16 * s0.clamp(0, (1 << 28) - 1);
        let pool = place_rows(&req.pool, &mut rows);
        return cx.replay_await(&req, aseq, &pool, &rows.rec);
    }
    if let Some(o) = &cx.out {
        let pool = place_rows(&o.pool, &mut rows);
        let (t, nt) = match &o.tab {
            Some(t) => (t.as_slice(), 1),
            None => (&[][..], 0),
        };
        return mk_packet(3, cx.op_id, dseq, 1, &pool, &o.rec, &[], &rows.rec, t, nt);
    }
    boundary(1, cx.op_id, 2)
}

pub fn import_begin(input: &[i64]) -> Vec<i64> {
    let id = rd(input, 2);
    let Some(hi) = io_check(input) else { return boundary(1, id, 0) };
    let mut cx = Ctx::new(1, id, repack(input, hi), Vec::new(), &[0; 8]);
    drive(&mut cx, 1, 0, 1)
}

pub fn import_step(prev: &[i64], completion: &[i64]) -> Vec<i64> {
    let (id, seq) = (rd(prev, 2), rd(prev, 3));
    if !(0..1i64 << 53).contains(&seq) {
        return boundary(1, id, 0);
    }
    let s2 = sec_start(prev, 2).min(prev.len());
    let st = &prev[s2..(s2 + sec_len(prev, 2)).min(prev.len())];
    let Some(s) = state_ok_ie(st, 1, id).filter(|s| s.replay_ok(1, seq)) else { return boundary(1, id, 0) };
    let Some(hi) = comp_check(completion, s.code, s.aux) else { return boundary(1, id, 0) };
    let lg = log_append(s.log, s.code, s.aux, completion, hi);
    if log_words(&lg) > MAX_WORDS {
        return boundary(1, id, 1);
    }
    let mut cx = Ctx::new(1, id, s.blob.to_vec(), lg, &ck_phase_only(s.ck));
    cx.with_prev(prev, false);
    drive(&mut cx, seq + 1, seq, s0_of(1, s.ck))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ent_canon::ent_state::ent_value::{decode_span, encode_pool};

    fn u(s: &str) -> Vec<u16> {
        lit(s)
    }

    /// an op 1 input: dataDir and the env slots 1..7 (string or undefined)
    fn input(data_dir: &str, env: &[(usize, &str)]) -> Vec<i64> {
        let mut pool = PoolBuf::default();
        let mut rec = vec![0; 24];
        pool_put(&mut pool, &mut rec, 0, &u(data_dir));
        for &(k, s) in env {
            pool_put(&mut pool, &mut rec, 3 * k, &u(s));
        }
        // an input has only the pool and record sections (io_check wants every later section claimed)
        let pw = pool.view_words();
        let (lp, lr) = (pw.len() as i64, rec.len() as i64);
        let mut w = vec![1, 0, 7, 0, 2 + 2 + lp + lr, 1, 2, lp, lr];
        w.extend_from_slice(&pw);
        w.extend_from_slice(&rec);
        w
    }

    /// an answer packet (kind 2): record words and string slots
    fn answer(words: &[(usize, i64)], strs: &[(usize, &str)], reclen: usize) -> Vec<i64> {
        let mut pool = PoolBuf::default();
        let mut rec = vec![0; reclen];
        for &(at, w) in words {
            rec[at] = w;
        }
        for &(at, s) in strs {
            pool_put(&mut pool, &mut rec, at, &u(s));
        }
        mk_packet(2, 7, 1, 1, &pool.view_words(), &rec, &[], &[], &[], 0)
    }

    fn log_of(entries: &[(i64, Vec<i64>)]) -> Vec<i64> {
        entries.iter().fold(Vec::new(), |lg, (code, pk)| {
            let hi = Pool::at(pk, sec_start(pk, 0)).nbytes() as i64;
            log_append(&lg, *code, 0, pk, hi)
        })
    }

    fn ctx(blob: &[i64], lg: Vec<i64>, ck: &[i64]) -> Ctx {
        let hi = io_check(blob).expect("input");
        Ctx::new(1, 7, repack(blob, hi), lg, ck)
    }

    /// the string at the pool offset of row slot at
    fn row_str(rows: &Rows, at: usize) -> Vec<u16> {
        let w = rows.pool.view_words();
        decode_span(Pool(&w), rows.rec[at + 1] as usize, rows.rec[at + 2] as usize)
    }

    #[test]
    fn begin_emits_home_then_stalls_on_accounts() {
        let pk = import_begin(&input("/d", &[(1, " /c ")]));
        assert_eq!(pk[1], 1);
        assert_eq!(sec_len(&pk, 3), 18);
        assert_eq!(rd(&pk, sec_start(&pk, 1)), 1);
    }

    #[test]
    fn accounts_then_next_item_in_one_step() {
        let blob = input("/d", &[]);
        let mut cx = ctx(&blob, log_of(&[(1, answer(&[(1, 2)], &[], 4))]), &[1, 0, 0, 0, 0, 0, 0, 0]);
        let pk = drive(&mut cx, 3, 2, 2);
        // the count goes to ck2 and the next item (Claude account 0) stalls on account_at with s0 = aseq
        assert_eq!(pk[1], 1);
        assert_eq!(cx.ck, [2 + 16 * 3, 0, 2, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn claude_account_row() {
        let blob = input("/d", &[]);
        let ans = answer(&[(1, 1)], &[(2, "acc_0123456789abcdef"), (5, "Main")], 8);
        let mut cx = ctx(&blob, log_of(&[(2, ans)]), &[2, 0, 1, 0, 0, 0, 0, 0]);
        let mut rows = Rows::default();
        assert_eq!(claude_acct(&mut cx, &mut rows), Ok(()));
        assert_eq!(cx.ck, [2, 1, 1, 0, 0, 0, 0, 0]);
        assert_eq!(rows.rec.len(), 18);
        assert_eq!(row_str(&rows, 0), u("claude:acc_0123456789abcdef"));
        assert_eq!(row_str(&rows, 12), u("Main"));
        assert_eq!(row_str(&rows, 15), u("/d/engine-homes/claude/acc_0123456789abcdef"));
        // an invalid id is skipped
        let ans = answer(&[(1, 1)], &[(2, "acc_x")], 8);
        let mut cx = ctx(&blob, log_of(&[(2, ans)]), &[2, 0, 1, 0, 0, 0, 0, 0]);
        let mut rows = Rows::default();
        assert_eq!(claude_acct(&mut cx, &mut rows), Ok(()));
        assert_eq!((cx.ck[1], rows.rec.len()), (1, 0));
    }

    #[test]
    fn rows_are_placed_after_the_front_pool() {
        let mut rows = Rows::default();
        home_row(&mut rows, "grok:home", "grok", &u("/h/.grok"));
        let front = encode_pool(&u("abc")).view_words();
        let pool = place_rows(&front, &mut rows);
        assert_eq!(decode_span(Pool(&pool), rows.rec[1] as usize, rows.rec[2] as usize), u("grok:home"));
        assert_eq!(decode_span(Pool(&pool), rows.rec[16] as usize, rows.rec[17] as usize), u("/h/.grok"));
        assert_eq!((rows.rec[9], rows.rec[12]), (10, 10));
    }

    #[test]
    fn last_phase_is_done() {
        let blob = input("/d", &[]);
        let mut cx = ctx(&blob, Vec::new(), &[9, 0, 0, 0, 0, 0, 0, 0]);
        let pk = drive(&mut cx, 1, 0, 1);
        assert_eq!((pk[1], rd(&pk, sec_start(&pk, 1))), (3, 0));
    }

    #[test]
    fn rebase_keeps_offsets_past_2_pow_24() {
        // a pool of 16,777,216 words holds about 117 MB, so a string can start past byte 2^24
        let mut rec = vec![6, 20_000_000, 4, 10, 0, 0, 6, 3, 2];
        rebase(&mut rec, 30_000_000);
        assert_eq!(rec, vec![6, 50_000_000, 4, 10, 0, 0, 6, 30_000_003, 2]);
    }
}

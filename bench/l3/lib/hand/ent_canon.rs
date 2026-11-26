// Hand Rust baseline for `bench/l3/lib/ent_canon.pruv` (Rust-shape reset steps 3.5, 3.6).
// canonicalPath / isHostCodexHome / codexAccountFolder (reference/backend-codex-home.ts) and op 6. Every step takes the
// replay context `&mut Ctx` and returns `Result<T, Halt>`: an observation that stalls or breaks the replay returns
// through `?`, and a thrown homedir / cwd answer is rethrown as the item result (`Err(Halt::Done)`, output set).
// String results are returned as `Vec<u16>`, 0/1 results as `bool`.
// The account helpers (`valid_account_id`, `account_home`, `io_check`) come from `ent_accounts` (step 3.8).
// Check: rustc --edition 2021 --test ent_canon.rs -o /tmp/ent_canon_hand && /tmp/ent_canon_hand

#![allow(dead_code)]

#[cfg(not(shared_ctx))]
#[path = "ent_state.rs"]
pub mod ent_state;
#[cfg(shared_ctx)]
pub use crate::ent_state;
#[cfg(not(shared_ctx))]
#[path = "ent_path.rs"]
pub mod ent_path;
#[cfg(shared_ctx)]
pub use crate::ent_path;
#[cfg(not(shared_ctx))]
#[path = "ent_u16.rs"]
pub mod ent_u16;
#[cfg(shared_ctx)]
pub use crate::ent_u16;
#[cfg(not(shared_ctx))]
#[path = "ent_accounts.rs"]
pub mod ent_accounts;
#[cfg(shared_ctx)]
pub use crate::ent_accounts;

use ent_path::{basename, dirname, join, resolve_finish, resolve_need};
use ent_accounts::{account_home, io_check, valid_account_id};
use ent_state::ent_packet::{boundary, comp_check, put_vs, sec_len, sec_start};
use ent_state::ent_value::{decode_span, encode_pool, str_eq_lit, Pool, Value};
use ent_state::{log_append, log_ok, log_words, repack, Ctx, Halt, Output, State, MAGIC};

const SLASH: u16 = b'/' as u16;
const MAX_WORDS: usize = 1 << 24;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

pub fn lit(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn value(w: &[i64], p: usize) -> Value {
    Value::parse([rd(w, p), rd(w, p + 1), rd(w, p + 2)])
}

/// units of the string slot at word p of packet w (empty for any other value)
pub fn str_at(w: &[i64], p: usize) -> Vec<u16> {
    match value(w, p) {
        Value::Str(s) => decode_span(Pool::at(w, sec_start(w, 0)), s.off, s.len),
        _ => Vec::new(),
    }
}

/// units of the string slot at record offset k of the last answer
pub fn ent_str(cx: &Ctx, k: usize) -> Vec<u16> {
    let e = cx.ent();
    str_at(e, sec_start(e, 1) + k)
}

/// A protocol error as the result of any step.
fn fail<T>(cx: &mut Ctx) -> Result<T, Halt> {
    Err(cx.fail().unwrap_err())
}

/// The thrown value of the last (throw) answer is rethrown as it is: the item is done.
pub fn rethrow_answer(cx: &mut Ctx) -> Halt {
    let e = cx.ent();
    let rp = sec_start(e, 1);
    let rec = vec![1, rd(e, rp + 1), rd(e, rp + 2), rd(e, rp + 3)];
    cx.set_done(Output { pool: Vec::new(), rec, tab: None });
    Halt::Done
}

/// wire error: key 0 invalidId, 1 codexHostExecution
fn wire_done(cx: &mut Ctx, key: i64) {
    cx.set_done(Output { pool: Vec::new(), rec: vec![3, key], tab: None });
}

fn plain_done(cx: &mut Ctx) {
    cx.set_done(Output { pool: Vec::new(), rec: vec![4, 0], tab: None });
}

/// homedir(); a throw is rethrown
pub fn home_dir(cx: &mut Ctx) -> Result<Vec<u16>, Halt> {
    cx.homedir()?;
    if cx.ent_r(0) == 1 {
        return Err(rethrow_answer(cx));
    }
    Ok(ent_str(cx, 1))
}

/// join(homedir(), seg)
pub fn home_join(cx: &mut Ctx, seg: &str) -> Result<Vec<u16>, Halt> {
    let home = home_dir(cx)?;
    Ok(join(&[&home, &lit(seg)]))
}

/// path.resolve(p): process.cwd() is requested as the path module asks for it
fn resolve_cwd(cx: &mut Ctx, p: &[u16]) -> Result<Vec<u16>, Halt> {
    let mut ncwd = 0;
    let (mut c1, mut c2) = (Vec::new(), Vec::new());
    while ncwd < 2 && resolve_need(p, ncwd, &c1) {
        cx.cwd()?;
        if cx.ent_r(0) == 1 {
            return Err(rethrow_answer(cx));
        }
        let s = ent_str(cx, 1);
        if ncwd == 0 {
            c1 = s;
        } else {
            c2 = s;
        }
        ncwd += 1;
    }
    Ok(resolve_finish(p, ncwd, &c1, &c2))
}

// R3 helpers. r is a resolved path; a "normalized absolute" path starts with '/', has no "//" and no trailing '/'
// (except "/"). A normalized relative path is nonempty with no leading '/', no "//" and no trailing '/': its climb levels
// are the path with its last d segments cut off for d = 1..k (k = number of '/'), then "." (d = k + 1).
fn has_double_slash(r: &[u16]) -> bool {
    r.windows(2).any(|w| w == [SLASH, SLASH])
}

fn norm_abs(r: &[u16]) -> bool {
    r.first() == Some(&SLASH) && (r.len() == 1 || (r.last() != Some(&SLASH) && !has_double_slash(r)))
}

fn norm_rel(r: &[u16]) -> bool {
    !r.is_empty() && r[0] != SLASH && r[r.len() - 1] != SLASH && !has_double_slash(r)
}

fn count_slash(r: &[u16]) -> usize {
    r.iter().filter(|&&u| u == SLASH).count()
}

/// index of the d-th '/' counted from the end of r (d >= 1)
fn slash_from_end(r: &[u16], d: usize) -> Option<usize> {
    r.iter().enumerate().rev().filter(|&(_, &u)| u == SLASH).nth(d - 1).map(|(i, _)| i)
}

/// canonicalPath(path). Realpath and stat answers are observations. Climb checkpoint (R3): a level that fails and
/// moves on to its parent is dropped from the log and counted in ck[3 + call] (levels) and ck7 (entries), for the
/// first 3 calls of an item and only when the resolved path is normalized; a resumed step starts at that level.
pub fn canonical(cx: &mut Ctx, path: &[u16]) -> Result<Vec<u16>, Halt> {
    let ci = cx.ncan.clamp(0, 1000) as usize;
    cx.ncan = ci as i64 + 1;
    let r = resolve_cwd(cx, path)?;
    let slot = 3 + ci;
    let isabs = norm_abs(&r);
    let cmp = ci < 3 && (isabs || norm_rel(&r));
    let d = if ci < 3 { cx.ck[slot].clamp(0, 1 << 25) as usize } else { 0 };
    if d > 0 && (!cmp || r.len() < 2 || d > r.len() + 1) {
        return fail(cx);
    }
    let mut current = r.clone();
    let mut raw = Vec::new();
    let mut jn = Vec::new();
    if d > 0 {
        if isabs {
            let Some(q) = slash_from_end(&r, d) else { return fail(cx) };
            current = r[..q.max(1)].to_vec();
            raw = r[q + 1..].to_vec();
            jn = raw.clone();
        } else {
            // relative: levels 1..k cut at a slash, level k + 1 is "." with every segment still missing
            let k = count_slash(&r);
            if d > k + 1 {
                return fail(cx);
            }
            if d == k + 1 {
                current = lit(".");
                raw = r.clone();
                jn = r.clone();
            } else {
                let Some(q) = slash_from_end(&r, d) else { return fail(cx) };
                current = r[..q].to_vec();
                raw = r[q + 1..].to_vec();
                jn = raw.clone();
            }
        }
    }
    let mut nmiss = d;
    for _ in d..r.len() + 3 {
        let a0 = cx.pos;
        let mut ne = 1;
        cx.realpath(&current)?;
        if cx.ent_r(0) == 0 {
            let real = ent_str(cx, 1);
            return Ok(if nmiss == 0 { real } else { join(&[&real, &jn]) });
        }
        let e = cx.ent();
        let pool = Pool::at(e, sec_start(e, 0));
        let enoent = matches!(value(e, sec_start(e, 1) + 16), Value::Str(s) if str_eq_lit(pool, s, "ENOENT"));
        if !enoent {
            cx.stat(&current)?;
            ne = 2;
            if cx.ent_r(0) == 0 {
                let mut id = lit("dev:");
                id.extend(ent_str(cx, 1));
                id.extend(lit(":ino:"));
                id.extend(ent_str(cx, 4));
                if nmiss > 0 {
                    id.push(SLASH);
                    id.extend(&raw);
                }
                return Ok(id);
            }
        }
        let parent = dirname(&current);
        if parent == current {
            return resolve_cwd(cx, path);
        }
        // this level failed and the climb moves on: its entries are dropped and counted (R3)
        if cmp {
            cx.drop_level(a0, slot, ne);
        }
        let base = basename(&current);
        if nmiss == 0 {
            raw = base.clone();
            jn = base;
        } else {
            raw = [&base[..], &[SLASH], &raw].concat();
            if !base.is_empty() {
                jn = if jn.is_empty() { base } else { [&base[..], &[SLASH], &jn].concat() };
            }
        }
        nmiss += 1;
        current = parent;
    }
    fail(cx)
}

/// sameFile(left, right): both stat and the dev / ino strings are equal; any throw gives false
fn same_file(cx: &mut Ctx, left: &[u16], right: &[u16]) -> Result<bool, Halt> {
    cx.stat(left)?;
    if cx.ent_r(0) == 1 {
        return Ok(false);
    }
    let (dev, ino) = (ent_str(cx, 1), ent_str(cx, 4));
    cx.stat(right)?;
    if cx.ent_r(0) == 1 {
        return Ok(false);
    }
    Ok(dev == ent_str(cx, 1) && ino == ent_str(cx, 4))
}

/// hostCodexHome(): the ambient CODEX_HOME (trimmed) or join(homedir(), '.codex')
pub fn codex_home(cx: &mut Ctx) -> Result<Vec<u16>, Halt> {
    cx.ambient()?;
    let t = ent_u16::trim(&ent_str(cx, 1));
    if !t.is_empty() {
        return Ok(t);
    }
    home_join(cx, ".codex")
}

/// isHostCodexHome(home): every call asks the host again; nothing is cached
pub fn is_codex_home(cx: &mut Ctx, home: &[u16]) -> Result<bool, Halt> {
    let target = canonical(cx, home)?;
    let host = codex_home(cx)?;
    let host = canonical(cx, &host)?;
    let dot = home_join(cx, ".codex")?;
    let dot = canonical(cx, &dot)?;
    if target == host || target == dot {
        return Ok(true);
    }
    let host = codex_home(cx)?;
    if same_file(cx, home, &host)? {
        return Ok(true);
    }
    let dot = home_join(cx, ".codex")?;
    same_file(cx, home, &dot)
}

// ---- op 6 state check (State::parse covers only ops 3, 4, 8, 9) ----

/// request code / aux of an await emitted for op 1 (codes 1..7) or op 6 (codes 3..7); aux is always 0
fn pair_ie(op: i64, code: i64, aux: i64) -> bool {
    aux == 0
        && match op {
            1 => (1..=7).contains(&code),
            6 => (3..=7).contains(&code),
            _ => false,
        }
}

/// The state of an op 1 / op 6 await, parsed once, or None (an InvalidPacket for the caller).
pub fn state_ok_ie(st: &[i64], op: i64, op_id: i64) -> Option<State<'_>> {
    let n = st.len();
    if !(14..=MAX_WORDS).contains(&n) || st[0] != MAGIC || st[1] != op || !pair_ie(op, st[2], st[3]) {
        return None;
    }
    if st[4..12].iter().any(|&c| !(0..=1i64 << 32).contains(&c)) {
        return None;
    }
    let (l, g) = (st[12], st[13]);
    if l < 13 || g < 0 || l as usize > n || g as usize > n || 14 + (l + g) as usize != n {
        return None;
    }
    let s = State::view(st);
    let blob = s.blob;
    if ent_state::ent_packet::env_check(blob).is_err() || rd(blob, 1) != 0 || rd(blob, 3) != 0 {
        return None;
    }
    if rd(blob, 5) != op || rd(blob, 2) != op_id {
        return None;
    }
    if io_check(blob) != Some(Pool::at(blob, sec_start(blob, 0)).nbytes() as i64) || !log_ok(s.log, op_id) {
        return None;
    }
    Some(s)
}

// ---- op 6: codexAccountFolder(dataDir, { id, home }) ----
fn folder_item(cx: &mut Ctx) -> Result<(), Halt> {
    let b = &cx.blob;
    let r0 = sec_start(b, 1);
    if !valid_account_id(b, r0 + 3) {
        wire_done(cx, 0);
        return Ok(());
    }
    let expected = account_home(&str_at(b, r0), &lit("codex"), &str_at(b, r0 + 3));
    if !matches!(value(b, r0 + 6), Value::Str(_)) || str_at(b, r0 + 6) != expected {
        plain_done(cx);
        return Ok(());
    }
    if is_codex_home(cx, &expected)? {
        wire_done(cx, 1);
        return Ok(());
    }
    let pool = encode_pool(&expected);
    let mut rec = vec![0; 4];
    put_vs(&mut rec, 1, 6, 0, pool.nbytes as i64);
    cx.set_done(Output { pool: pool.view_words(), rec, tab: None });
    Ok(())
}

pub fn codex_folder_begin(input: &[i64]) -> Vec<i64> {
    let id = rd(input, 2);
    let Some(hi) = io_check(input) else { return boundary(6, id, 0) };
    let mut cx = Ctx::new(6, id, repack(input, hi), Vec::new(), &[0; 8]);
    let _ = folder_item(&mut cx);
    cx.finish(1, 0)
}

pub fn codex_folder_step(prev: &[i64], completion: &[i64]) -> Vec<i64> {
    let (id, seq) = (rd(prev, 2), rd(prev, 3));
    if !(0..1i64 << 53).contains(&seq) {
        return boundary(6, id, 0);
    }
    let s2 = sec_start(prev, 2).min(prev.len());
    let st = &prev[s2..(s2 + sec_len(prev, 2)).min(prev.len())];
    let Some(s) = state_ok_ie(st, 6, id).filter(|s| s.replay_ok(6, seq)) else { return boundary(6, id, 0) };
    let Some(hi) = comp_check(completion, s.code, s.aux) else { return boundary(6, id, 0) };
    let lg = log_append(s.log, s.code, s.aux, completion, hi);
    if log_words(&lg) > MAX_WORDS {
        return boundary(6, id, 1);
    }
    let mut cx = Ctx::new(6, id, s.blob.to_vec(), lg, s.ck);
    cx.with_prev(prev, true);
    let _ = folder_item(&mut cx);
    cx.finish(seq + 1, seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ent_state::ent_packet::mk_packet;

    fn u(s: &str) -> Vec<u16> {
        lit(s)
    }

    /// an answer packet (kind 2) whose record holds the given strings as slots after word 0
    fn answer(word0: i64, strs: &[(usize, &str)], reclen: usize) -> Vec<i64> {
        let mut pool = ent_state::ent_value::PoolBuf::default();
        let mut rec = vec![0; reclen];
        rec[0] = word0;
        for &(at, s) in strs {
            let off = pool.append_units(&u(s));
            put_vs(&mut rec, at, 6, off as i64, (pool.nbytes - off) as i64);
        }
        mk_packet(2, 7, 1, 6, &pool.view_words(), &rec, &[], &[], &[], 0)
    }

    fn log_of(entries: &[(i64, Vec<i64>)]) -> Vec<i64> {
        entries.iter().fold(Vec::new(), |lg, (code, pk)| {
            let hi = Pool::at(pk, sec_start(pk, 0)).nbytes() as i64;
            log_append(&lg, *code, 0, pk, hi)
        })
    }

    fn ctx(lg: Vec<i64>, ck: &[i64]) -> Ctx {
        Ctx::new(6, 7, Vec::new(), lg, ck)
    }

    #[test]
    fn helpers() {
        assert!(norm_abs(&u("/")) && norm_abs(&u("/a/b")) && !norm_abs(&u("/a/")) && !norm_abs(&u("/a//b")));
        assert!(norm_rel(&u("a/b")) && !norm_rel(&u("/a")) && !norm_rel(&u("a/")) && !norm_rel(&u("")));
        assert_eq!(count_slash(&u("/a/b")), 2);
        assert_eq!(slash_from_end(&u("/a/b"), 1), Some(2));
        assert_eq!(slash_from_end(&u("/a/b"), 2), Some(0));
        assert_eq!(slash_from_end(&u("/a/b"), 3), None);
        assert!(pair_ie(6, 3, 0) && !pair_ie(6, 2, 0) && pair_ie(1, 1, 0) && !pair_ie(1, 1, 1));
    }

    #[test]
    fn home_stalls_then_throws() {
        let mut cx = ctx(Vec::new(), &[0; 8]);
        assert_eq!(home_dir(&mut cx), Err(Halt::Stalled));
        assert_eq!(cx.req.as_ref().map(|r| r.code), Some(4));
        let mut cx = ctx(log_of(&[(4, answer(1, &[], 4))]), &[0; 8]);
        assert_eq!(home_join(&mut cx, ".codex"), Err(Halt::Done));
        assert_eq!(cx.out.as_ref().map(|o| o.rec[0]), Some(1));
        let mut cx = ctx(log_of(&[(4, answer(0, &[(1, "/home/u")], 4))]), &[0; 8]);
        assert_eq!(home_join(&mut cx, ".codex"), Ok(u("/home/u/.codex")));
    }

    #[test]
    fn canonical_climbs_and_drops() {
        // realpath(/a/b) throws ENOENT, realpath(/a) gives /x: /x/b, and the failed level is dropped (ck3 = 1, ck7 = 1)
        let lg = log_of(&[(6, answer(1, &[(16, "ENOENT")], 19)), (6, answer(0, &[(1, "/x")], 4))]);
        let mut cx = ctx(lg, &[0; 8]);
        assert_eq!(canonical(&mut cx, &u("/a/b")), Ok(u("/x/b")));
        assert_eq!((cx.ck[3], cx.ck[7], cx.pos, cx.pos == cx.lg.len()), (1, 1, cx.lg.len(), true));
        // the resumed step starts at level 1 (/a) from the checkpoint
        let lg = log_of(&[(6, answer(0, &[(1, "/x")], 4))]);
        let mut cy = ctx(lg, &[0, 0, 0, 1, 0, 0, 0, 1]);
        assert_eq!(canonical(&mut cy, &u("/a/b")), Ok(u("/x/b")));
        // a checkpoint past the path is a protocol error
        let mut cz = ctx(Vec::new(), &[0, 0, 0, 9, 0, 0, 0, 0]);
        assert_eq!(canonical(&mut cz, &u("/a/b")), Err(Halt::Bad));
    }

    #[test]
    fn canonical_stat_id() {
        // realpath(/a) throws EACCES: stat gives dev 1 ino 2
        let lg = log_of(&[(6, answer(1, &[(16, "EACCES")], 19)), (7, answer(0, &[(1, "1"), (4, "2")], 7))]);
        let mut cx = ctx(lg, &[0; 8]);
        assert_eq!(canonical(&mut cx, &u("/a")), Ok(u("dev:1:ino:2")));
    }

    #[test]
    fn same_file_compares_dev_ino() {
        let st = || answer(0, &[(1, "1"), (4, "2")], 7);
        let mut cx = ctx(log_of(&[(7, st()), (7, st())]), &[0; 8]);
        assert_eq!(same_file(&mut cx, &u("/a"), &u("/b")), Ok(true));
        let mut cx = ctx(log_of(&[(7, st()), (7, answer(1, &[], 4))]), &[0; 8]);
        assert_eq!(same_file(&mut cx, &u("/a"), &u("/b")), Ok(false));
    }
}

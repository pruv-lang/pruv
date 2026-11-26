// Hand Rust baseline for `bench/l3/lib/ent_dispatch.pruv` (Rust-shape reset step 3.4).
// Ops 2 find_source, 3 list_external, 4 read_external and the rethrow classification. Each item is a replay step
// over the shared context: `fn item(cx: &mut Ctx) -> Result<(), Halt>`; an observation that stalls or breaks the replay
// returns through `?`, so the code after it only sees an answer.
// Check: rustc --edition 2021 --test ent_dispatch.rs -o /tmp/ent_dispatch_hand && /tmp/ent_dispatch_hand

#![allow(dead_code)]

#[path = "ent_state.rs"]
pub mod ent_state;

use ent_state::ent_packet::{put_vs, sec_start, MAX_WORDS};
use ent_state::ent_value::{self, prefix_in, str_eq_lit, strict_eq, units, units_in, Pool, PoolBuf, Value};
use ent_state::{blob_pool, Ctx, Halt, Output, Request};

const LIMIT: i64 = MAX_WORDS;
/// list_external cuts titles to 200 UTF-16 units
const CUT: usize = 200;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

fn slot(w: &[i64], p: usize) -> [i64; 3] {
    [rd(w, p), rd(w, p + 1), rd(w, p + 2)]
}

fn value(w: &[i64], p: usize) -> Value {
    Value::parse(slot(w, p))
}

fn pool_of(w: &[i64]) -> Pool<'_> {
    Pool::at(w, sec_start(w, 0))
}

// ---- op 2: first row whose source strictly equals the query ----
pub fn find_source(cx: &mut Ctx) {
    let w = &cx.blob;
    let r0 = sec_start(w, 1);
    let base = sec_start(w, rd(w, r0 + 4).clamp(0, 8) as usize);
    let cnt = rd(w, r0 + 5).clamp(0, LIMIT) as usize;
    let (pool, query) = (pool_of(w), value(w, r0));
    let found = (0..cnt).position(|i| strict_eq(pool, query, pool, value(w, base + 3 * i)));
    let mut rec = vec![0; 4];
    if let Some(i) = found {
        put_vs(&mut rec, 1, 4, i as i64, 0);
    }
    cx.set_done(Output { pool: Vec::new(), rec, tab: None });
}

// ---- route: 0 claude, 1 codex, 2 opencode, 3 qwen, else grok (4); the check order is fixed ----
pub fn route(pool: Pool, engine: Value) -> i64 {
    let Value::Str(s) = engine else { return 4 };
    ["claude", "codex", "opencode", "qwen"].iter().position(|lit| str_eq_lit(pool, s, lit)).unwrap_or(4) as i64
}

/// invoke_engine request: record [9, route, opKind, source, folder, thread, options]; no thread is tag 10.
fn invoke(blob: &[i64], read: bool) -> Request {
    let r0 = sec_start(blob, 1);
    let mut rec = vec![9, route(pool_of(blob), value(blob, r0)), read as i64];
    rec.extend(slot(blob, r0 + 3));
    rec.extend(slot(blob, r0 + 6));
    if read {
        rec.extend(slot(blob, r0 + 9));
        rec.extend(slot(blob, r0 + 12));
    } else {
        rec.extend([10, 0, 0]);
        rec.extend(slot(blob, r0 + 9));
    }
    Request { code: 9, aux: read as i64, pool: blob_pool(blob).to_vec(), rec, tab: None }
}

// ---- rethrow: classification of the thrown value E (record offset 1 of a throw answer) ----
pub fn rethrow(cx: &mut Ctx) {
    let e = cx.ent();
    let rp = sec_start(e, 1);
    let thrown = slot(e, rp + 1);
    let is_eie = rd(e, rp + 4) == 3;
    let is_err = rd(e, rp + 7) == 3;
    let pool = pool_of(e);
    let is_abort = matches!(value(e, rp + 10), Value::Str(s) if str_eq_lit(pool, s, "AbortError"));
    let out = if is_eie || is_abort {
        let mut rec = vec![1];
        rec.extend(thrown);
        Output { pool: Vec::new(), rec, tab: None }
    } else {
        let mut o = PoolBuf::default();
        let mut rec = vec![2];
        let lit: Vec<u16> = "read-failed".encode_utf16().collect();
        rec.extend([6, o.append_units(&lit) as i64, o.nbytes as i64]);
        if is_err {
            let msg: Vec<u16> = match value(e, rp + 13) {
                Value::Str(s) => units(pool, s.off, s.len).collect(),
                _ => Vec::new(),
            };
            let at = o.append_units(&msg);
            rec.extend([6, at as i64, (o.nbytes - at) as i64]);
        } else {
            rec.extend([0, 0, 0]);
        }
        rec.extend(thrown);
        Output { pool: o.view_words(), rec, tab: None }
    };
    cx.set_done(out);
}

// ---- op 3: list_external ----

/// A title cut to CUT units: the bytes kept, and whether the cut falls inside a surrogate pair.
struct Clip {
    nb: usize,
    split: bool,
}

fn title_clip(pool: Pool, off: usize, n: usize) -> Option<Clip> {
    let n = n.min(pool.nbytes().saturating_sub(off));
    if units_in(pool, off, n) <= CUT {
        return None;
    }
    let nb = prefix_in(pool, off, n, CUT);
    Some(Clip { nb, split: units_in(pool, off, nb) < CUT })
}

/// Rows [clipped, rowRef {3}, title {3}]: an unclipped row keeps the source title (title slot zero).
fn list_recipe(cx: &mut Ctx) -> Result<(), Halt> {
    let e = cx.ent();
    let rp = sec_start(e, 1);
    let base = sec_start(e, rd(e, rp + 2).clamp(0, 8) as usize);
    let cnt = rd(e, rp + 3).clamp(0, LIMIT) as usize;
    if cnt > 2396745 || 16 + 7 * cnt > MAX_WORDS as usize {
        return cx.too_big();
    }
    let src = pool_of(e);
    let clips: Vec<Option<Clip>> = (0..cnt)
        .map(|i| match value(e, base + 6 * i + 3) {
            Value::Str(s) => title_clip(src, s.off, s.len),
            _ => None,
        })
        .collect();
    let copies = clips.iter().flatten().count();
    if copies > 83886 || 16 + 7 * cnt + 200 * copies > MAX_WORDS as usize {
        return cx.too_big();
    }
    // the source pool is kept when it fits: whole-form cuts then point into it
    let nu = rd(e, sec_start(e, 0)).clamp(0, LIMIT) as usize;
    let reuse = copies > 0 && 16 + 7 * cnt + nu + 201 * copies <= MAX_WORDS as usize;
    let mut pool = PoolBuf::default();
    if reuse {
        pool.append_span(src, 0, src.nbytes(), nu);
    }
    let mut tab = Vec::with_capacity(7 * cnt);
    for (i, clip) in clips.iter().enumerate() {
        let rb = base + 6 * i;
        tab.push(clip.is_some() as i64);
        tab.extend(slot(e, rb));
        let Some(c) = clip else {
            tab.extend([0, 0, 0]);
            continue;
        };
        let off = rd(e, rb + 4) as usize;
        if reuse && !c.split {
            tab.extend([6, off as i64, c.nb as i64]);
            continue;
        }
        let mut title: Vec<u16> = units(src, off, c.nb).collect();
        if c.split {
            // the cut keeps the high surrogate of the next pair
            title.push(units(src, off + c.nb, 4).next().unwrap_or(0));
        }
        let at = pool.append_units(&title);
        tab.extend([6, at as i64, (pool.nbytes - at) as i64]);
    }
    let rec = vec![0, 7, 4, cnt as i64];
    cx.set_done(Output { pool: pool.view_words(), rec, tab: Some(tab) });
    Ok(())
}

pub fn list_external(cx: &mut Ctx) -> Result<(), Halt> {
    cx.obs(invoke(&cx.blob, false))?;
    if cx.ent_r(0) == 1 {
        rethrow(cx);
        return Ok(());
    }
    list_recipe(cx)
}

// ---- op 4: read_external ----
pub fn read_external(cx: &mut Ctx) -> Result<(), Halt> {
    cx.obs(invoke(&cx.blob, true))?;
    if cx.ent_r(0) == 1 {
        rethrow(cx);
        return Ok(());
    }
    let e = cx.ent();
    let mut rec = vec![0];
    rec.extend(slot(e, sec_start(e, 1) + 1));
    cx.set_done(Output { pool: Vec::new(), rec, tab: None });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ent_state::ent_packet::mk_packet;
    use ent_value::encode_pool;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn routes() {
        let p = encode_pool(&u("codexqwenx")).view_words();
        let s = |off, len| Value::Str(ent_value::Span { off, len });
        assert_eq!(route(Pool(&p), s(0, 5)), 1);
        assert_eq!(route(Pool(&p), s(5, 4)), 3);
        assert_eq!(route(Pool(&p), s(5, 5)), 4);
        assert_eq!(route(Pool(&p), Value::Null), 4);
    }

    #[test]
    fn list_cuts_titles() {
        // titles: "ab", 201 x 'x' (whole-form cut), 199 x 'y' + U+1F600 (the cut splits the pair)
        let mut t = u("ab");
        t.extend(u(&"x".repeat(201)));
        t.extend(u(&"y".repeat(199)));
        t.extend(u("\u{1F600}"));
        let pool = encode_pool(&t).view_words();
        let rows = [9, 1, 0, 6, 0, 2, 9, 2, 0, 6, 2, 201, 9, 3, 0, 6, 203, 203];
        let ent = mk_packet(2, 7, 1, 3, &pool, &[0, 7, 3, 3], &[], &rows, &[], 0);
        let mut cx = Ctx::new(3, 7, Vec::new(), Vec::new(), &[0; 8]);
        cx.lg = vec![0, 0, 0];
        cx.lg.extend(&ent);
        cx.ent = 3..cx.lg.len();
        list_recipe(&mut cx).unwrap();
        let o = cx.out.as_ref().unwrap();
        let tab = o.tab.as_ref().unwrap();
        assert_eq!(&tab[..7], &[0, 9, 1, 0, 0, 0, 0]);
        assert_eq!(&tab[7..14], &[1, 9, 2, 0, 6, 2, 200]);
        // the split cut is appended after the retained pool: 199 'y' and a lone high surrogate (3 bytes)
        let n0 = Pool(&pool).nbytes() as i64;
        assert_eq!(&tab[14..21], &[1, 9, 3, 0, 6, n0, 202]);
        assert_eq!(o.rec, vec![0, 7, 4, 3]);
    }

    #[test]
    fn rethrow_classifies() {
        // throw answer [1, E {3}, eie {3}, isErr {3}, name {3}, message {3}, ...] with isErr true, message "boom"
        let pool = encode_pool(&u("boom")).view_words();
        let mut rec = vec![1, 4, 5, 0, 2, 0, 0, 3, 0, 0, 0, 0, 0, 6, 0, 4];
        rec.resize(19, 0);
        let ent = mk_packet(2, 7, 1, 4, &pool, &rec, &[], &[], &[], 0);
        let mut cx = Ctx::new(4, 7, Vec::new(), ent.clone(), &[0; 8]);
        cx.ent = 0..ent.len();
        rethrow(&mut cx);
        let o = cx.out.as_ref().unwrap();
        assert_eq!(o.rec, vec![2, 6, 0, 11, 6, 11, 4, 4, 5, 0]);
    }
}

// M11 perf baseline: idiomatic hand-written Rust for setCard. The stored budget and the result words are flat V words
// (tags 1..9, see ABI.md); the card / lang / by inputs are "JB" byte packets (big-endian):
//   01 undefined, 02 null, 03 false, 04 true, 07 + f64 bits (every number), 06 + u32 len + WTF-8,
//   08 + u32 count + elements, 09 + u32 count + (u32 key len + key WTF-8 + value) pairs; depth <= 32, one value, no tail.
// Used only by the harness perf ops; it never feeds the Q comparison.
#![allow(dead_code)]
use std::collections::HashMap;

const MAX_DEPTH: u32 = 32;
const SAFE: f64 = 9007199254740991.0;

// ---- V words (budget side) ----

pub fn v_end(v: &[i64], at: usize) -> Option<usize> {
    let t = *v.get(at)?;
    match t {
        1..=4 => Some(at + 1),
        5 | 7 => if at + 2 <= v.len() { Some(at + 2) } else { None },
        6 => {
            let n = *v.get(at + 1)? as usize;
            if at + 2 + n <= v.len() { Some(at + 2 + n) } else { None }
        }
        8 | 9 => {
            let cnt = *v.get(at + 1)?;
            let mut p = at + 2;
            for _ in 0..cnt {
                if t == 9 {
                    p += 1 + *v.get(p)? as usize;
                }
                p = v_end(v, p)?;
            }
            Some(p)
        }
        _ => None,
    }
}

fn key_eq(v: &[i64], p: usize, k: &str) -> bool {
    let n = v[p] as usize;
    n == k.len() && k.bytes().enumerate().all(|(j, b)| v[p + 1 + j] == b as i64)
}

pub fn obj_get(v: &[i64], at: usize, k: &str) -> Option<usize> {
    if v.get(at) != Some(&9) {
        return None;
    }
    let mut p = at + 2;
    let mut found = None;
    for _ in 0..v[at + 1] {
        let vs = p + 1 + v[p] as usize;
        if key_eq(v, p, k) {
            found = Some(vs);
        }
        p = v_end(v, vs)?;
    }
    found
}

fn str_is_v(v: &[i64], p: Option<usize>, s: &str) -> bool {
    matches!(p, Some(i) if v[i] == 6 && key_eq(v, i + 1, s))
}

// ---- JB admission: structure + canonical WTF-8 + depth <= 32 ----

fn get32(b: &[u8], p: usize) -> Option<usize> {
    let s = b.get(p..p + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Canonical WTF-8: no overlongs, <= U+10FFFF, lone surrogates (ED A0..BF xx) allowed, but a lead surrogate directly
/// followed by a trail surrogate is rejected (it must be one 4-byte sequence).
fn wtf8_ok(s: &[u8]) -> bool {
    if s.is_ascii() {
        return true;
    }
    let n = s.len();
    let cont = |i: usize| i < n && s[i] & 0xC0 == 0x80;
    let mut i = 0;
    while i < n {
        let c = s[i];
        if c < 0x80 {
            i += 1;
            continue;
        }
        let l = match c {
            0xC2..=0xDF if cont(i + 1) => 2,
            0xE0 if i + 2 < n && s[i + 1] >= 0xA0 && s[i + 1] <= 0xBF && cont(i + 2) => 3,
            0xED if cont(i + 1) && cont(i + 2) => {
                if s[i + 1] <= 0xAF && s[i + 1] >= 0xA0 && i + 4 < n && s[i + 3] == 0xED && s[i + 4] >= 0xB0 && s[i + 4] <= 0xBF {
                    return false;
                }
                3
            }
            0xE1..=0xEC | 0xEE | 0xEF if cont(i + 1) && cont(i + 2) => 3,
            0xF0 if i + 3 < n && s[i + 1] >= 0x90 && s[i + 1] <= 0xBF && cont(i + 2) && cont(i + 3) => 4,
            0xF1..=0xF3 if cont(i + 1) && cont(i + 2) && cont(i + 3) => 4,
            0xF4 if i + 3 < n && s[i + 1] >= 0x80 && s[i + 1] <= 0x8F && cont(i + 2) && cont(i + 3) => 4,
            _ => return false,
        };
        i += l;
    }
    true
}

fn skip(b: &[u8], at: usize, depth: u32) -> Option<usize> {
    if depth > MAX_DEPTH {
        return None;
    }
    match *b.get(at)? {
        1..=4 => Some(at + 1),
        7 => if at + 9 <= b.len() { Some(at + 9) } else { None },
        6 => {
            let e = at + 5 + get32(b, at + 1)?;
            if wtf8_ok(b.get(at + 5..e)?) { Some(e) } else { None }
        }
        8 => {
            let cnt = get32(b, at + 1)?;
            let mut p = at + 5;
            for _ in 0..cnt {
                p = skip(b, p, depth + 1)?;
            }
            Some(p)
        }
        9 => {
            let cnt = get32(b, at + 1)?;
            let mut p = at + 5;
            for _ in 0..cnt {
                let ke = p + 4 + get32(b, p)?;
                if !wtf8_ok(b.get(p + 4..ke)?) {
                    return None;
                }
                p = skip(b, ke, depth + 1)?;
            }
            Some(p)
        }
        _ => None,
    }
}

/// One pass: exactly one well-formed value covering the whole packet (a non-empty packet).
pub fn admit(b: &[u8]) -> bool {
    skip(b, 0, 1) == Some(b.len())
}

// ---- JB access (all of it assumes an admitted packet) ----

fn end(b: &[u8], at: usize) -> usize {
    match b[at] {
        7 => at + 9,
        6 => at + 5 + get32(b, at + 1).unwrap(),
        8 => {
            let mut p = at + 5;
            for _ in 0..get32(b, at + 1).unwrap() {
                p = end(b, p);
            }
            p
        }
        9 => {
            let mut p = at + 5;
            for _ in 0..get32(b, at + 1).unwrap() {
                p = end(b, p + 4 + get32(b, p).unwrap());
            }
            p
        }
        _ => at + 1,
    }
}

fn count(b: &[u8], at: usize) -> usize {
    get32(b, at + 1).unwrap()
}

/// Element start positions of an array at `at`.
fn elems<'a>(b: &'a [u8], at: usize) -> impl Iterator<Item = usize> + 'a {
    let mut p = at + 5;
    (0..count(b, at)).map(move |_| {
        let s = p;
        p = end(b, s);
        s
    })
}

/// One pass over an object's pairs: the value position of the LAST pair for each key (None if absent / not an object).
fn fields<const N: usize>(b: &[u8], at: usize, keys: [&str; N]) -> [Option<usize>; N] {
    let mut r = [None; N];
    if b[at] != 9 {
        return r;
    }
    let mut p = at + 5;
    for _ in 0..count(b, at) {
        let kn = get32(b, p).unwrap();
        let k = &b[p + 4..p + 4 + kn];
        let vs = p + 4 + kn;
        for j in 0..N {
            if keys[j].as_bytes() == k {
                r[j] = Some(vs);
            }
        }
        p = end(b, vs);
    }
    r
}

/// present = key exists and the value is not undefined.
fn present(b: &[u8], p: Option<usize>) -> Option<usize> {
    p.filter(|&i| b[i] != 1)
}

fn str_bytes(b: &[u8], at: usize) -> &[u8] {
    &b[at + 5..at + 5 + count(b, at)]
}

fn str_is(b: &[u8], p: Option<usize>, s: &str) -> bool {
    matches!(p, Some(i) if b[i] == 6 && str_bytes(b, i) == s.as_bytes())
}

fn num_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64::from_be_bytes(b[at + 1..at + 9].try_into().unwrap()))
}

// ---- strings: ECMAScript trim on WTF-8 edges, UTF-16 length ----

fn lead_trim(s: &[u8]) -> usize {
    match s {
        [9..=13 | 32, ..] => 1,
        [0xC2, 0xA0, ..] => 2,
        [0xE1, 0x9A, 0x80, ..]
        | [0xE2, 0x80, 0x80..=0x8A | 0xA8 | 0xA9 | 0xAF, ..]
        | [0xE2, 0x81, 0x9F, ..]
        | [0xE3, 0x80, 0x80, ..]
        | [0xEF, 0xBB, 0xBF, ..] => 3,
        _ => 0,
    }
}

fn trail_trim(s: &[u8]) -> usize {
    match s {
        [.., 9..=13 | 32] => 1,
        [.., 0xC2, 0xA0] => 2,
        [.., 0xE1, 0x9A, 0x80]
        | [.., 0xE2, 0x80, 0x80..=0x8A | 0xA8 | 0xA9 | 0xAF]
        | [.., 0xE2, 0x81, 0x9F]
        | [.., 0xE3, 0x80, 0x80]
        | [.., 0xEF, 0xBB, 0xBF] => 3,
        _ => 0,
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    loop {
        let k = lead_trim(s);
        if k == 0 { break; }
        s = &s[k..];
    }
    loop {
        let k = trail_trim(s);
        if k == 0 { break; }
        s = &s[..s.len() - k];
    }
    s
}

/// UTF-16 length of canonical WTF-8, stopping once it exceeds `cap` (the returned value is then > cap, not exact).
fn units_upto(s: &[u8], cap: usize) -> usize {
    let mut n = 0;
    for &c in s {
        if c & 0xC0 != 0x80 {
            n += 1 + (c >= 0xF0) as usize;
            if n > cap {
                return n;
            }
        }
    }
    n
}

fn units_len(s: &[u8]) -> usize {
    units_upto(s, usize::MAX)
}

// ---- validation (kind 0..6) ----

fn text_ok(b: &[u8], p: Option<usize>) -> bool {
    match p {
        Some(i) if b[i] == 6 => (1..=300).contains(&units_upto(trim(str_bytes(b, i)), 300)),
        _ => false,
    }
}

fn lines_ok(b: &[u8], p: Option<usize>, min: usize, max: usize) -> bool {
    match p {
        Some(i) if b[i] == 8 && (min..=max).contains(&count(b, i)) => elems(b, i).all(|e| text_ok(b, Some(e))),
        _ => false,
    }
}

fn count_ok(b: &[u8], p: Option<usize>) -> bool {
    match p {
        Some(i) if b[i] == 7 => {
            let f = num_at(b, i);
            f.is_finite() && f.fract() == 0.0 && f >= 0.0 // -0 accepted
        }
        _ => false,
    }
}

/// `card` must be admitted. 0 = valid, 1..6 = the kind of the first failing field (same order as bud_card).
pub fn validate(b: &[u8]) -> i64 {
    let [dl, dn, tg, vf, cf, st] =
        fields(b, 0, ["deliverable", "done", "targets", "verify", "conflicts", "state"]);
    if !text_ok(b, dl) { return 1; }
    if !lines_ok(b, dn, 1, 5) { return 2; }
    if let Some(t) = present(b, tg) {
        let [c, d, u, n] = fields(b, t, ["confirmed", "candidate", "unknown", "note"]);
        let ok = b[t] == 9
            && count_ok(b, c)
            && count_ok(b, d)
            && count_ok(b, u)
            && match present(b, n) {
                None => true,
                Some(n) => b[n] == 6 && units_upto(str_bytes(b, n), 300) <= 300,
            };
        if !ok { return 3; }
    }
    if let Some(f) = present(b, vf) {
        let ok = b[f] == 8 && count(b, f) <= 5 && elems(b, f).all(|e| {
            let [label, status] = fields(b, e, ["label", "status"]);
            b[e] == 9 && text_ok(b, label) && ["planned", "done", "skipped"].iter().any(|s| str_is(b, status, s))
        });
        if !ok { return 4; }
    }
    if let Some(c) = present(b, cf) {
        if !lines_ok(b, Some(c), 0, 5) { return 5; }
    }
    if let Some(s) = present(b, st) {
        if !["working", "done", "waiting-user", "blocked"].iter().any(|x| str_is(b, Some(s), x)) { return 6; }
    }
    0
}

// ---- projection to V words ----

fn key(o: &mut Vec<i64>, k: &str) {
    o.push(k.len() as i64);
    o.extend(k.bytes().map(|b| b as i64));
}

/// WTF-8 -> UTF-16 unit words (no header).
fn put_units(o: &mut Vec<i64>, s: &[u8]) {
    if s.is_ascii() {
        o.extend(s.iter().map(|&c| c as i64));
        return;
    }
    let mut i = 0;
    while i < s.len() {
        let c = s[i] as u32;
        if c < 0x80 {
            o.push(c as i64);
            i += 1;
        } else if c < 0xE0 {
            o.push((((c & 0x1F) << 6) | (s[i + 1] as u32 & 0x3F)) as i64);
            i += 2;
        } else if c < 0xF0 {
            o.push((((c & 0x0F) << 12) | ((s[i + 1] as u32 & 0x3F) << 6) | (s[i + 2] as u32 & 0x3F)) as i64);
            i += 3;
        } else {
            let cp = ((c & 0x07) << 18) | ((s[i + 1] as u32 & 0x3F) << 12) | ((s[i + 2] as u32 & 0x3F) << 6) | (s[i + 3] as u32 & 0x3F);
            let x = cp - 0x10000;
            o.push((0xD800 + (x >> 10)) as i64);
            o.push((0xDC00 + (x & 0x3FF)) as i64);
            i += 4;
        }
    }
}

fn put_str(o: &mut Vec<i64>, s: &[u8]) {
    o.push(6);
    o.push(units_len(s) as i64);
    put_units(o, s);
}

fn put_key(o: &mut Vec<i64>, s: &[u8]) {
    o.push(units_len(s) as i64);
    put_units(o, s);
}

fn put_num(o: &mut Vec<i64>, b: &[u8], at: usize) {
    let bits = u64::from_be_bytes(b[at + 1..at + 9].try_into().unwrap());
    let f = f64::from_bits(bits);
    if f.is_finite() && f.fract() == 0.0 && f.abs() <= SAFE && bits != 0x8000_0000_0000_0000 {
        o.extend_from_slice(&[5, f as i64]);
    } else {
        o.extend_from_slice(&[7, bits as i64]);
    }
}

/// Generic projection of the JB value at `at` (None = absent -> undefined).
fn put_val(o: &mut Vec<i64>, b: &[u8], at: Option<usize>) {
    let at = match at {
        Some(a) => a,
        None => return o.push(1),
    };
    match b[at] {
        t @ 1..=4 => o.push(t as i64),
        7 => put_num(o, b, at),
        6 => put_str(o, str_bytes(b, at)),
        8 => {
            o.extend_from_slice(&[8, count(b, at) as i64]);
            for e in elems(b, at) {
                put_val(o, b, Some(e));
            }
        }
        _ => {
            o.extend_from_slice(&[9, count(b, at) as i64]);
            let mut p = at + 5;
            for _ in 0..count(b, at) {
                let kn = get32(b, p).unwrap();
                put_key(o, &b[p + 4..p + 4 + kn]);
                p += 4 + kn;
                put_val(o, b, Some(p));
                p = end(b, p);
            }
        }
    }
}

fn put_lines(o: &mut Vec<i64>, b: &[u8], at: usize) {
    o.extend_from_slice(&[8, count(b, at) as i64]);
    for e in elems(b, at) {
        put_str(o, trim(str_bytes(b, e)));
    }
}

fn truthy(b: &[u8]) -> bool {
    match b.first() {
        Some(3) | Some(1) | Some(2) | None => false,
        Some(7) => {
            let f = num_at(b, 0);
            f != 0.0 && !f.is_nan()
        }
        Some(6) => count(b, 0) > 0,
        _ => true,
    }
}

/// ISO-8601 of epoch ms (years 0..9999 suffice for the perf inputs).
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

/// The stored card V words. `card` must be admitted and valid; `by` is an admitted packet or empty (absent);
/// it is stored only when JS-truthy.
pub fn build(b: &[u8], by: &[u8], now_ms: i64) -> Vec<i64> {
    let [dl, dn, tg, vf, cf, st] =
        fields(b, 0, ["deliverable", "done", "targets", "verify", "conflicts", "state"]);
    let [tp, vp, cp, sp] = [tg, vf, cf, st].map(|p| present(b, p));
    let by_on = truthy(by);
    let k = 3 + [tp, vp, cp, sp].iter().filter(|x| x.is_some()).count() + by_on as usize;
    let mut o = Vec::with_capacity(b.len() + by.len() + 96);
    o.extend_from_slice(&[9, k as i64]);
    key(&mut o, "deliverable");
    put_str(&mut o, trim(str_bytes(b, dl.unwrap())));
    key(&mut o, "done");
    put_lines(&mut o, b, dn.unwrap());
    key(&mut o, "updatedAt");
    put_str(&mut o, iso(now_ms).as_bytes());
    if let Some(t) = tp {
        let [c, d, u, n] = fields(b, t, ["confirmed", "candidate", "unknown", "note"]);
        let note = n.filter(|&n| b[n] == 6 && count(b, n) > 0);
        key(&mut o, "targets");
        o.extend_from_slice(&[9, if note.is_some() { 4 } else { 3 }]);
        for (kk, p) in [("confirmed", c), ("candidate", d), ("unknown", u)] {
            key(&mut o, kk);
            put_val(&mut o, b, p);
        }
        if note.is_some() {
            key(&mut o, "note");
            put_val(&mut o, b, note);
        }
    }
    if let Some(f) = vp {
        key(&mut o, "verify");
        o.extend_from_slice(&[8, count(b, f) as i64]);
        for e in elems(b, f) {
            let [label, status] = fields(b, e, ["label", "status"]);
            o.extend_from_slice(&[9, 2]);
            key(&mut o, "label");
            put_str(&mut o, trim(str_bytes(b, label.unwrap())));
            key(&mut o, "status");
            put_val(&mut o, b, status);
        }
    }
    if let Some(c) = cp {
        key(&mut o, "conflicts");
        put_lines(&mut o, b, c);
    }
    if let Some(s) = sp {
        key(&mut o, "state");
        put_val(&mut o, b, Some(s));
    }
    if by_on {
        let [sid, tid] = fields(by, 0, ["sessionId", "turnId"]);
        key(&mut o, "by");
        o.extend_from_slice(&[9, 2]);
        key(&mut o, "sessionId");
        put_val(&mut o, by, sid);
        key(&mut o, "turnId");
        put_val(&mut o, by, tid);
    }
    o
}

/// lang code: 1 iff lang is the string "en", else 0 (Korean).
pub fn lang_code(lang: &[u8]) -> i64 {
    (lang == [6, 0, 0, 0, 2, b'e', b'n']) as i64
}

// ---- budget (V words) ----

pub fn put_card(budget: &[i64], card: &[i64]) -> Vec<i64> {
    let be = v_end(budget, 0).unwrap();
    match obj_get(budget, 0, "card") {
        Some(cp) => {
            let ce = v_end(budget, cp).unwrap();
            let mut o = Vec::with_capacity(budget.len() + card.len());
            o.extend_from_slice(&budget[..cp]);
            o.extend_from_slice(card);
            o.extend_from_slice(&budget[ce..be]);
            o
        }
        None => {
            let mut o = budget[..be].to_vec();
            o[1] += 1;
            key(&mut o, "card");
            o.extend_from_slice(card);
            o
        }
    }
}

/// Sessions keyed by the framed packed session-id key `n, words(n)` (packed u16 lanes, see bud_keys); value = budget V words.
pub struct Store {
    pub sessions: HashMap<Vec<i64>, Vec<i64>>,
}

fn units(v: &[i64], p: usize) -> &[i64] {
    &v[p + 2..p + 2 + v[p + 1] as usize]
}

/// The framed packed key `k` = [n, words(n) words] is canonical: n in 0..=2^30, len == 1 + ceil(n/4), zero tail lanes
/// (the same test as bud_keys::rec_end over the whole slice).
fn packed_framed_ok(k: &[i64]) -> bool {
    let n = match k.first() { Some(&n) if (0..=1 << 30).contains(&n) => n as usize, _ => return false };
    let m = (n + 3) / 4;
    if k.len() != 1 + m { return false; }
    let r = n & 3;
    r == 0 || m == 0 || (k[m] as u64) & (u64::MAX >> (16 * r)) == 0
}

impl Store {
    /// setCard on the stored budget; returns -1 ok (budget stored, result words in `out`), 0 = input shape / session
    /// check failed, 1..=6 = validation kind, -2 = a JB packet is malformed (the Pruv entry's status 2).
    /// `sid` = the framed packed key of ref.sessionId (packed once by the caller; empty = the ref has none).
    /// Same work as root.pruv m11_set_card: admission of card / lang / by, shape checks, then on success the result words
    /// [k, used_now, saved, changed, findSession, {ok: true, budget}] written into `out`.
    pub fn set_card(&mut self, rf: &[i64], sid: &[i64], card: &[u8], lang: &[u8], by: &[u8], now_ms: i64, out: &mut Vec<i64>) -> i64 {
        if v_end(rf, 0) != Some(rf.len()) || card.is_empty() { return 0; }
        if !admit(card) || (!lang.is_empty() && !admit(lang)) || (!by.is_empty() && !admit(by)) { return -2; }
        if !sid.is_empty() && !packed_framed_ok(sid) { return 0; }
        let budget = match self.sessions.get(sid) { Some(b) if !b.is_empty() => b, _ => return 0 };
        let same = match (obj_get(budget, 0, "id"), obj_get(rf, 0, "budgetId")) {
            (Some(a), Some(b)) if budget[a] == 6 && rf[b] == 6 => units(budget, a) == units(rf, b),
            _ => false,
        };
        if !same || str_is_v(budget, obj_get(budget, 0, "state"), "closed") { return 0; }
        let k = validate(card);
        if k != 0 {
            let _lang = lang_code(lang); // selects the error text; the baseline only returns the kind
            return k;
        }
        let c = build(card, by, now_ms);
        let nb = put_card(budget, &c);
        out.clear();
        out.push((4 + 13 + nb.len()) as i64);
        out.extend_from_slice(&[1, 1, 1, 2, 9, 2]);
        key(out, "ok");
        out.push(4);
        key(out, "budget");
        out.extend_from_slice(&nb);
        self.sessions.insert(sid.to_vec(), nb); // sid is non-empty here: the lookup above found it
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(o: &mut Vec<u8>, x: &str) {
        o.push(6);
        o.extend((x.len() as u32).to_be_bytes());
        o.extend(x.as_bytes());
    }
    fn k(o: &mut Vec<u8>, x: &str) {
        o.extend((x.len() as u32).to_be_bytes());
        o.extend(x.as_bytes());
    }
    fn n(o: &mut Vec<u8>, x: f64) {
        o.push(7);
        o.extend(x.to_be_bytes());
    }
    fn card(deliv: &str) -> Vec<u8> {
        let mut o = vec![9, 0, 0, 0, 3];
        k(&mut o, "deliverable");
        s(&mut o, deliv);
        k(&mut o, "done");
        o.extend([8, 0, 0, 0, 1]);
        s(&mut o, " c ");
        k(&mut o, "state");
        s(&mut o, "working");
        o
    }

    #[test]
    fn admission() {
        assert!(admit(&card(" ab ")));
        assert!(admit(&[1]) && admit(&[7, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!admit(&[1, 1]), "trailing byte");
        assert!(!admit(&[]), "empty");
        assert!(!admit(&[6, 0, 0, 0, 2, 0xC0, 0x80]), "overlong");
        assert!(!admit(&[6, 0, 0, 0, 4, 0xF4, 0x90, 0x80, 0x80]), "above 10FFFF");
        assert!(admit(&[6, 0, 0, 0, 3, 0xED, 0xA0, 0x80]), "lone lead surrogate");
        assert!(admit(&[6, 0, 0, 0, 3, 0xED, 0xB0, 0x80]), "lone trail surrogate");
        assert!(!admit(&[6, 0, 0, 0, 6, 0xED, 0xA0, 0x80, 0xED, 0xB0, 0x80]), "surrogate pair as 3+3");
        assert!(admit(&[6, 0, 0, 0, 4, 0xF0, 0x9F, 0x98, 0x80]));
        let mut deep = vec![8, 0, 0, 0, 1].repeat(32);
        deep.push(1);
        assert!(!admit(&deep), "33 levels (32 arrays + scalar)");
        let mut ok = vec![8, 0, 0, 0, 1].repeat(31);
        ok.push(1);
        assert!(admit(&ok));
    }

    #[test]
    fn validate_and_build() {
        let c = card("\u{3000} ab\u{feff}");
        assert!(admit(&c));
        assert_eq!(validate(&c), 0);
        let o = build(&c, &[], 0);
        assert_eq!(&o[..2], &[9, 4]);
        let p = 2 + 1 + 11; // key "deliverable"
        assert_eq!(&o[p..p + 4], &[6, 2, 'a' as i64, 'b' as i64]);
        assert_eq!(validate(&card(" \t ")), 1);
        assert_eq!(validate(&card(&"a".repeat(301))), 1);
        assert_eq!(validate(&card(&"a".repeat(300))), 0);
        // 150 astral chars = 300 units ok, 151 = 302 units rejected
        assert_eq!(validate(&card(&"\u{1F600}".repeat(150))), 0);
        assert_eq!(validate(&card(&"\u{1F600}".repeat(151))), 1);
        // by projection
        let mut by = vec![9, 0, 0, 0, 1];
        k(&mut by, "turnId");
        n(&mut by, 7.0);
        let o = build(&c, &by, 0);
        assert_eq!(&o[..2], &[9, 5]);
        assert_eq!(&o[o.len() - 2..], &[5, 7]);
        assert!(!truthy(&[7, 0x80, 0, 0, 0, 0, 0, 0, 0]), "-0 is falsy");
        assert!(!truthy(&[6, 0, 0, 0, 0]));
        assert!(truthy(&[4]));
    }
}

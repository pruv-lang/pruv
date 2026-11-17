// M11 perf baseline: idiomatic hand-written Rust for setCard on the same flat V words as the Pruv code
// (tags 1..9, see ABI.md). Used only by the harness perf ops; it never feeds the Q comparison.
#![allow(dead_code)]
use std::collections::HashMap;

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

fn present(v: &[i64], p: Option<usize>) -> Option<usize> {
    p.filter(|&i| v[i] != 1)
}

fn is_trim(u: i64) -> bool {
    matches!(u, 9..=13 | 32 | 160 | 5760 | 65279 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
}

fn trimmed(v: &[i64], at: usize) -> &[i64] {
    let n = v[at + 1] as usize;
    let s = &v[at + 2..at + 2 + n];
    let a = s.iter().position(|&u| !is_trim(u)).unwrap_or(n);
    let b = s.iter().rposition(|&u| !is_trim(u)).map(|i| i + 1).unwrap_or(a);
    &s[a..b.max(a)]
}

fn text_ok(v: &[i64], p: Option<usize>) -> bool {
    match p {
        Some(i) if v[i] == 6 => (1..=300).contains(&trimmed(v, i).len()),
        _ => false,
    }
}

fn elems(v: &[i64], at: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(v[at + 1] as usize);
    let mut p = at + 2;
    for _ in 0..v[at + 1] {
        out.push(p);
        p = v_end(v, p).unwrap();
    }
    out
}

fn lines_ok(v: &[i64], p: Option<usize>, min: i64, max: i64) -> bool {
    match p {
        Some(i) if v[i] == 8 && (min..=max).contains(&v[i + 1]) => elems(v, i).into_iter().all(|e| text_ok(v, Some(e))),
        _ => false,
    }
}

fn str_is(v: &[i64], p: Option<usize>, s: &str) -> bool {
    matches!(p, Some(i) if v[i] == 6 && key_eq(v, i + 1, s))
}

fn count_ok(v: &[i64], p: Option<usize>) -> bool {
    match p {
        Some(i) if v[i] == 5 => v[i + 1] >= 0,
        Some(i) if v[i] == 7 => {
            let f = f64::from_bits(v[i + 1] as u64);
            f.is_finite() && f.fract() == 0.0 && f >= 0.0
        }
        _ => false,
    }
}

pub fn validate(v: &[i64], at: usize) -> i64 {
    if !text_ok(v, obj_get(v, at, "deliverable")) { return 1; }
    if !lines_ok(v, obj_get(v, at, "done"), 1, 5) { return 2; }
    if let Some(t) = present(v, obj_get(v, at, "targets")) {
        let ok = v[t] == 9
            && ["confirmed", "candidate", "unknown"].iter().all(|k| count_ok(v, obj_get(v, t, k)))
            && match present(v, obj_get(v, t, "note")) {
                None => true,
                Some(n) => v[n] == 6 && v[n + 1] <= 300,
            };
        if !ok { return 3; }
    }
    if let Some(f) = present(v, obj_get(v, at, "verify")) {
        let ok = v[f] == 8 && v[f + 1] <= 5 && elems(v, f).into_iter().all(|e| {
            v[e] == 9 && text_ok(v, obj_get(v, e, "label"))
                && ["planned", "done", "skipped"].iter().any(|s| str_is(v, obj_get(v, e, "status"), s))
        });
        if !ok { return 4; }
    }
    if let Some(c) = present(v, obj_get(v, at, "conflicts")) {
        if !lines_ok(v, Some(c), 0, 5) { return 5; }
    }
    if let Some(s) = present(v, obj_get(v, at, "state")) {
        if !["working", "done", "waiting-user", "blocked"].iter().any(|x| str_is(v, Some(s), x)) { return 6; }
    }
    0
}

fn key(o: &mut Vec<i64>, k: &str) {
    o.push(k.len() as i64);
    o.extend(k.bytes().map(|b| b as i64));
}

fn put_str(o: &mut Vec<i64>, s: &[i64]) {
    o.push(6);
    o.push(s.len() as i64);
    o.extend_from_slice(s);
}

fn put_val(o: &mut Vec<i64>, v: &[i64], p: Option<usize>) {
    match p {
        Some(i) => o.extend_from_slice(&v[i..v_end(v, i).unwrap()]),
        None => o.push(1),
    }
}

fn put_lines(o: &mut Vec<i64>, v: &[i64], at: usize) {
    o.push(8);
    o.push(v[at + 1]);
    for e in elems(v, at) {
        put_str(o, trimmed(v, e));
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

pub fn build(v: &[i64], at: usize, by: Option<&[i64]>, now_ms: i64) -> Vec<i64> {
    let tp = present(v, obj_get(v, at, "targets"));
    let vp = present(v, obj_get(v, at, "verify"));
    let cp = present(v, obj_get(v, at, "conflicts"));
    let sp = present(v, obj_get(v, at, "state"));
    let k = 3 + [tp, vp, cp, sp].iter().filter(|x| x.is_some()).count() + by.is_some() as usize;
    let mut o = Vec::with_capacity(v.len() + 64);
    o.push(9);
    o.push(k as i64);
    key(&mut o, "deliverable");
    put_str(&mut o, trimmed(v, obj_get(v, at, "deliverable").unwrap()));
    key(&mut o, "done");
    put_lines(&mut o, v, obj_get(v, at, "done").unwrap());
    key(&mut o, "updatedAt");
    let d: Vec<i64> = iso(now_ms).bytes().map(|b| b as i64).collect();
    put_str(&mut o, &d);
    if let Some(t) = tp {
        let note = obj_get(v, t, "note").filter(|&n| v[n] == 6 && v[n + 1] > 0);
        key(&mut o, "targets");
        o.push(9);
        o.push(if note.is_some() { 4 } else { 3 });
        for kk in ["confirmed", "candidate", "unknown"] {
            key(&mut o, kk);
            put_val(&mut o, v, obj_get(v, t, kk));
        }
        if note.is_some() {
            key(&mut o, "note");
            put_val(&mut o, v, note);
        }
    }
    if let Some(f) = vp {
        key(&mut o, "verify");
        o.push(8);
        o.push(v[f + 1]);
        for e in elems(v, f) {
            o.extend_from_slice(&[9, 2]);
            key(&mut o, "label");
            put_str(&mut o, trimmed(v, obj_get(v, e, "label").unwrap()));
            key(&mut o, "status");
            put_val(&mut o, v, obj_get(v, e, "status"));
        }
    }
    if let Some(c) = cp {
        key(&mut o, "conflicts");
        put_lines(&mut o, v, c);
    }
    if let Some(s) = sp {
        key(&mut o, "state");
        put_val(&mut o, v, Some(s));
    }
    if let Some(b) = by {
        key(&mut o, "by");
        o.extend_from_slice(&[9, 2]);
        key(&mut o, "sessionId");
        put_val(&mut o, b, obj_get(b, 0, "sessionId"));
        key(&mut o, "turnId");
        put_val(&mut o, b, obj_get(b, 0, "turnId"));
    }
    o
}

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

/// Sessions keyed by session-id units; value = budget V words.
pub struct Store {
    pub sessions: HashMap<Vec<i64>, Vec<i64>>,
}

fn units(v: &[i64], p: usize) -> &[i64] {
    &v[p + 2..p + 2 + v[p + 1] as usize]
}

impl Store {
    /// setCard on the stored budget; returns the error kind (-1 ok) and stores the new budget on success.
    /// Same work as root.pruv m11_set_card: input shape checks, and on success the result words
    /// [k, used_now, saved, changed, findSession, {ok: true, budget}] written into `out`.
    pub fn set_card(&mut self, rf: &[i64], card: &[i64], now_ms: i64, out: &mut Vec<i64>) -> i64 {
        if v_end(rf, 0) != Some(rf.len()) || v_end(card, 0) != Some(card.len()) { return 0; }
        let sp = match obj_get(rf, 0, "sessionId") { Some(p) if rf[p] == 6 => p, _ => return 0 };
        let sid = units(rf, sp);
        let budget = match self.sessions.get(sid) { Some(b) if !b.is_empty() => b, _ => return 0 };
        let same = match (obj_get(budget, 0, "id"), obj_get(rf, 0, "budgetId")) {
            (Some(a), Some(b)) if budget[a] == 6 && rf[b] == 6 => units(budget, a) == units(rf, b),
            _ => false,
        };
        if !same || str_is(budget, obj_get(budget, 0, "state"), "closed") { return 0; }
        let k = validate(card, 0);
        if k != 0 { return k; }
        let c = build(card, 0, None, now_ms);
        let nb = put_card(budget, &c);
        out.clear();
        out.push((4 + 13 + nb.len()) as i64);
        out.extend_from_slice(&[1, 1, 1, 2, 9, 2]);
        key(out, "ok");
        out.push(4);
        key(out, "budget");
        out.extend_from_slice(&nb);
        self.sessions.insert(sid.to_vec(), nb);
        -1
    }
}

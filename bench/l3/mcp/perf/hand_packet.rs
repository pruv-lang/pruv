// P05M hand-written Rust baseline of the MCP "packet core" (included into perf_driver.rs, see run_perf.py).
//
// Same input boundary as the Pruv rows: the source bytes and the 6-word tape of one parsed tools/list page, the current
// packet and the running (count, objects); same output: the new packet. Semantics follow bench/l3/mcp/packet.pruv at
// the pre-rewrite commit (old_packet.pruv: `tool`, `usable`, `cut`) and the root tools loop:
//   * tools = field(field(root,"result"),"tools"); field lookup = LAST matching key (keys compared after JSON decoding),
//     a non-object parent has no fields;
//   * loop over the array's elements while objects < 500; every object counts; an object is accepted when its name is a
//     string with 1..200 UTF-16 units, each >= 32, not 127..159, not 8232/8233, and count < 500;
//   * slot layout at base = 10 + 10*count: name/title/description as (offset, len) or -1 for "absent" (the len slot is
//     left as it was), title must be nonempty, caps 16000000 / 16000000 / 65536 with a pair-safe cut, guard
//     `len(p) <= 100000000 - n`; base+6 = 1 for readOnlyHint true; base+7 = destructiveHint 0/1 else -1; base+8/9 =
//     (offset, len) of the widened raw JSON bytes of the tool object (len <= 16000000 and the same guard);
//   * strings decode like jstr::step: \uXXXX, \b \f \n \r \t, any other escaped byte stands for itself, UTF-8 scalars,
//     astral scalars become two units.
// Written as a single pass: one reserved output Vec, strings decoded straight into it and cut back with truncate, no
// per-field temporaries, safe Rust with the usual bounds checks. The decode of a field stops at cap+1 units (all the
// cut needs), a lazy cutoff a competent programmer would add; the output is identical.

const MAX_WORDS: usize = 100_000_000;

#[derive(Clone, Copy)]
struct Span {
    s: usize,
    e: usize,
}

// access::raw_span validity (status 0): 0 <= s <= e <= len(source) && e <= 16777216.
#[inline]
fn span(src: &[u8], t: &[i64], id: usize) -> Option<Span> {
    let s = t[6 * id + 1];
    let e = t[6 * id + 2];
    if s < 0 || e < s || e as usize > src.len() || e > 16_777_216 {
        return None;
    }
    Some(Span { s: s as usize, e: e as usize })
}

#[inline]
fn hex(c: u8) -> i64 {
    match c {
        b'0'..=b'9' => (c - b'0') as i64,
        b'A'..=b'F' => (c - b'A') as i64 + 10,
        b'a'..=b'f' => (c - b'a') as i64 + 10,
        _ => 0,
    }
}

// Decode the string token (quotes included) to UTF-16 units appended to `out`; stops once `limit` units were pushed
// (a step may overshoot by one unit).
fn decode(src: &[u8], sp: Span, out: &mut Vec<i64>, limit: usize) {
    let (mut p, stop) = if sp.e - sp.s >= 2 { (sp.s + 1, sp.e - 1) } else { (sp.s, sp.s) };
    let base = out.len();
    while p < stop && out.len() - base < limit {
        let c = src[p];
        if c < 128 && c != b'\\' {
            out.push(c as i64);
            p += 1;
            continue;
        }
        if c == b'\\' && stop - p >= 2 {
            let e = src[p + 1];
            if e == b'u' && stop - p >= 6 {
                let v = 4096 * hex(src[p + 2]) + 256 * hex(src[p + 3]) + 16 * hex(src[p + 4]) + hex(src[p + 5]);
                out.push(v);
                p += 6;
                continue;
            }
            out.push(match e {
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                _ => e as i64,
            });
            p += 2;
            continue;
        }
        let (n, mut v): (usize, i64) = match c {
            194..=223 => (2, c as i64 - 192),
            224..=239 => (3, c as i64 - 224),
            240..=244 => (4, c as i64 - 240),
            _ => (1, c as i64),
        };
        if stop - p < n {
            out.push(65533);
            p += 1;
            continue;
        }
        let mut bad = false;
        for k in 1..n {
            v = v * 64 + (src[p + k] % 64) as i64;
            if v > 16_777_215 {
                bad = true;
                break;
            }
        }
        p += n;
        if bad || v > 1_114_111 {
            v = 65533;
        }
        if n == 1 {
            v = c as i64;
        }
        if v <= 65535 {
            out.push(v);
        } else {
            out.push(55296 + (v - 65536) / 1024);
            out.push(56320 + (v - 65536) % 1024);
        }
    }
}

// ascii_equal: the decoded string token equals the ASCII `key`.
fn key_eq(src: &[u8], sp: Span, key: &[u8]) -> bool {
    let inner = if sp.e - sp.s >= 2 { &src[sp.s + 1..sp.e - 1] } else { &src[sp.s..sp.s] };
    if inner.iter().all(|&c| c != b'\\' && c < 128) {
        return inner == key;
    }
    let mut u: Vec<i64> = Vec::with_capacity(key.len() + 1);
    decode(src, sp, &mut u, key.len() + 1);
    u.len() == key.len() && u.iter().zip(key).all(|(a, b)| *a == *b as i64)
}

// Last member named `key` of object `obj`; None when `obj` is not an object or has no such member.
fn field(src: &[u8], t: &[i64], obj: usize, key: &[u8]) -> Option<usize> {
    if t[6 * obj] != 6 {
        return None;
    }
    let stop = t[6 * obj + 3] as usize;
    let mut j = obj + 1;
    let mut found = None;
    while j < stop {
        if let Some(sp) = span(src, t, j) {
            if key_eq(src, sp, key) {
                found = Some(j + 1);
            }
        }
        j = t[6 * (j + 1) + 3] as usize;
    }
    found
}

#[inline]
fn is_unit_ok(u: i64) -> bool {
    !(u < 32 || (127..=159).contains(&u) || u == 8232 || u == 8233)
}

fn str_span(src: &[u8], t: &[i64], id: Option<usize>) -> Option<Span> {
    let id = id?;
    if t[6 * id] != 4 {
        return None;
    }
    span(src, t, id)
}

// One string field: slots (slot, slot+1) get (-1, untouched) or (cursor, n); decodes in place at the end of `out`.
fn put_field(src: &[u8], sp: Option<Span>, cap: usize, nonempty: bool, out: &mut Vec<i64>, slot: usize) {
    out[slot] = -1;
    let Some(sp) = sp else { return };
    let at = out.len();
    decode(src, sp, out, cap + 1);
    let m = out.len() - at;
    let mut n = m.min(cap);
    if n > 0 && n < m && (55296..=56319).contains(&out[at + n - 1]) && (56320..=57343).contains(&out[at + n]) {
        n -= 1;
    }
    if (!nonempty || m > 0) && at <= MAX_WORDS - n {
        out.truncate(at + n);
        out[slot] = at as i64;
        out[slot + 1] = n as i64;
    } else {
        out.truncate(at);
    }
}

// Returns (new packet, new count, new objects).
pub fn core_hand(src: &[u8], t: &[i64], p0: &[i64], count0: i64, objects0: i64) -> (Vec<i64>, i64, i64) {
    let mut count = count0;
    let mut objects = objects0;
    let tools = field(src, t, 0, b"result").and_then(|r| field(src, t, r, b"tools"));
    // Every unit comes from a token byte and every raw copy is one byte per word, so 2 * (array bytes) is an upper
    // bound of the appended words: one allocation, no regrowth.
    let mut reserve = 0usize;
    let mut arr: Option<(usize, usize)> = None;
    if let Some(a) = tools {
        if t[6 * a] == 5 {
            let stop = t[6 * a + 3] as usize;
            if stop > a && stop <= t.len() / 6 {
                arr = Some((a + 1, stop));
                if let Some(sp) = span(src, t, a) {
                    reserve = 2 * (sp.e - sp.s);
                }
            }
        }
    }
    let mut out: Vec<i64> = Vec::with_capacity(p0.len() + reserve);
    out.extend_from_slice(p0);
    let Some((mut j, stop)) = arr else { return (out, count, objects) };
    let mut seen = 0usize;
    while j < stop && seen < 1_048_576 && objects < 500 {
        let cur = j;
        j = t[6 * j + 3] as usize;
        seen += 1;
        if t[6 * cur] != 6 {
            continue;
        }
        objects += 1;
        if count >= 500 {
            continue;
        }
        // name: decode tentatively at the end of `out`; usable() decides.
        let at = out.len();
        let mut ok = false;
        if let Some(sp) = str_span(src, t, field(src, t, cur, b"name")) {
            decode(src, sp, &mut out, 201);
            let u = &out[at..];
            ok = !u.is_empty() && u.len() <= 200 && u.iter().all(|&x| is_unit_ok(x));
        }
        if !ok {
            out.truncate(at);
            continue;
        }
        let base = 10 + 10 * count as usize;
        out[base] = at as i64;
        out[base + 1] = (out.len() - at) as i64;
        put_field(src, str_span(src, t, field(src, t, cur, b"title")), 16_000_000, true, &mut out, base + 2);
        put_field(src, str_span(src, t, field(src, t, cur, b"description")), 65_536, false, &mut out, base + 4);
        let a = field(src, t, cur, b"annotations");
        if let Some(h) = a.and_then(|a| field(src, t, a, b"readOnlyHint")) {
            if t[6 * h] == 2 {
                out[base + 6] = 1;
            }
        }
        out[base + 7] = -1;
        if let Some(h) = a.and_then(|a| field(src, t, a, b"destructiveHint")) {
            match t[6 * h] {
                1 => out[base + 7] = 0,
                2 => out[base + 7] = 1,
                _ => {}
            }
        }
        // raw JSON of the tool object, widened one byte per word; an invalid span reads as empty.
        let raw: &[u8] = match span(src, t, cur) {
            Some(sp) => &src[sp.s..sp.e],
            None => &[],
        };
        if raw.len() <= 16_000_000 && out.len() <= MAX_WORDS - raw.len() {
            out[base + 8] = out.len() as i64;
            out[base + 9] = raw.len() as i64;
            out.extend(raw.iter().map(|&b| b as i64));
        }
        count += 1;
    }
    (out, count, objects)
}

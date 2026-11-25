// Hand Rust baseline for `bench/l3/lib/ent_value.pruv` (Rust-shape reset step 3.3).
// Same meaning per operation, written the way a Rust programmer writes JS value semantics over the entry packet.
// The packet is the external input format (host.rs is the layout authority). A value slot is three words
// `[tag, a, b]`; the wire check (ent_wire) and the packet check (ent_packet) ran first, so every string slot spans
// complete forms inside the pool (`span_ok`). The slot is parsed once into `Value`; the pool is one borrowed view
// `Pool<'a>` over `&w[s..]` and every reader takes that view, never `(w, start)` pairs re-clamped per byte.
// Section 0 is the packed WTF-8 pool `[units, nbytes, W_1..W_m]`, seven bytes per word, high byte first. Byte 255
// separates a lone high and a lone low surrogate and counts no unit.
// Check: rustc --edition 2021 --test ent_value.rs -o /tmp/ent_value_hand && /tmp/ent_value_hand

use std::cmp::Ordering;

pub const NB_MAX: usize = (1 << 30) - 1;

/// One borrowed pool view. Words past the view read as zero (the writer zero-fills the last word).
#[derive(Clone, Copy)]
pub struct Pool<'a>(pub &'a [i64]);

impl<'a> Pool<'a> {
    /// The pool that starts at word `s` of `w` (hand of pruv `pool_at`).
    pub fn at(w: &'a [i64], s: usize) -> Self {
        Pool(&w[s.min(w.len())..])
    }
    pub fn nbytes(self) -> usize {
        self.0.get(1).copied().unwrap_or(0).clamp(0, NB_MAX as i64) as usize
    }
    pub fn byte(self, k: usize) -> u8 {
        let word = self.0.get(2 + k / 7).copied().unwrap_or(0);
        ((word >> (8 * (6 - k % 7))) & 255) as u8
    }
    fn bytes(self, off: usize, n: usize) -> impl Iterator<Item = u8> + 'a {
        let p = self;
        (off..off + n).map(move |k| p.byte(k))
    }
}

fn cont(b: u8) -> bool {
    (128..192).contains(&b)
}

fn form_width(b: u8) -> usize {
    match b {
        255 => 1,
        0..=191 => 1,
        192..=223 => 2,
        224..=239 => 3,
        _ => 4,
    }
}

/// A string span `off..off+n` of the pool: inside the stream, on form boundaries, not covering the separator.
/// On success the caller knows `off + n <= nbytes <= 2^30 - 1` (the pruv `ensures`; `vs` then adds `a + b`).
pub fn span_ok(pool: Pool, off: i64, n: i64) -> bool {
    let nb = pool.nbytes() as i64;
    if off < 0 || n < 0 || off > nb || n > nb - off {
        return false;
    }
    let (off, end) = (off as usize, (off + n) as usize);
    let nb = nb as usize;
    if (off != nb && cont(pool.byte(off))) || (end != nb && cont(pool.byte(end))) {
        return false;
    }
    pool.bytes(off, end - off).all(|b| b != 255)
}

/// The code point of the form of width `width` at byte `k`.
fn form_code(pool: Pool, k: usize, width: usize) -> u32 {
    let b = pool.byte(k) as u32;
    let t = |i: usize| (pool.byte(k + i) % 64) as u32;
    match width {
        1 => b,
        2 => b % 32 * 64 + t(1),
        3 => b % 16 * 4096 + t(1) * 64 + t(2),
        _ => b % 8 * 262144 + t(1) * 4096 + t(2) * 64 + t(3),
    }
}

/// Walk the complete forms of `off..off+n` (cut to the stream): (byte, width) per form.
fn forms<'a>(pool: Pool<'a>, off: usize, n: usize) -> impl Iterator<Item = (usize, usize)> + 'a {
    let n = n.min(pool.nbytes().saturating_sub(off));
    let mut k = 0;
    std::iter::from_fn(move || {
        if k >= n {
            return None;
        }
        let width = form_width(pool.byte(off + k));
        if width > n - k {
            return None;
        }
        let at = off + k;
        k += width;
        Some((at, width))
    })
}

fn form_units(pool: Pool, at: usize, width: usize) -> usize {
    match (width, pool.byte(at)) {
        (4, _) => 2,
        (_, 255) => 0,
        _ => 1,
    }
}

/// UTF-16 units of the complete forms in `off..off+n`.
pub fn units_in(pool: Pool, off: usize, n: usize) -> usize {
    forms(pool, off, n).map(|(at, w)| form_units(pool, at, w)).sum()
}

/// Bytes of the longest complete prefix of `off..off+n` with at most `limit` units.
pub fn prefix_in(pool: Pool, off: usize, n: usize, limit: usize) -> usize {
    let (mut k, mut u) = (0, 0);
    for (at, w) in forms(pool, off, n) {
        let du = form_units(pool, at, w);
        if u >= limit || u + du > limit {
            break;
        }
        u += du;
        k += w;
    }
    k
}

/// The UTF-16 units of `off..off+n`, lazily.
pub fn units<'a>(pool: Pool<'a>, off: usize, n: usize) -> impl Iterator<Item = u16> + 'a {
    forms(pool, off, n).flat_map(move |(at, w)| {
        let cp = form_code(pool, at, w);
        let (a, b) = match (w, pool.byte(at)) {
            (_, 255) => (None, None),
            (4, _) => (Some(0xD800 + (cp - 0x10000) / 1024), Some(0xDC00 + (cp - 0x10000) % 1024)),
            _ => (Some(cp), None),
        };
        a.into_iter().chain(b).map(|c| c as u16)
    })
}

/// Decode `off..off+n` into UTF-16 units at the end of `o`.
pub fn decode_into(pool: Pool, off: usize, n: usize, o: &mut Vec<u16>) {
    o.extend(units(pool, off, n));
}

pub fn decode_span(pool: Pool, off: usize, n: usize) -> Vec<u16> {
    let mut o = Vec::with_capacity(units_in(pool, off, n));
    decode_into(pool, off, n, &mut o);
    o
}

// ---- values ----

/// A string slot after the packet check: a checked span of the packet pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub off: usize,
    pub len: usize,
}

/// One value slot `[tag, a, b]`, parsed once. Tag 10 (absent own property) folds into `Undefined` for `===` and
/// `${}` but stays distinct for the `??` test, so it keeps its own arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    /// tag 4: a reference compared by id
    Ref(i64),
    /// tag 5: a Number as its IEEE bits (lo, hi 32 bits on the wire)
    Num(f64),
    Str(Span),
    /// tag 9: a function or symbol compared by id; truthy
    Id(i64),
    Absent,
    /// tag 11: an array/object, truthy, never `===` another slot
    Object,
    /// tags 7, 8 and anything else the packet check let through: falsy, never `===`
    Other(i64),
}

impl Value {
    pub fn parse(slot: [i64; 3]) -> Value {
        let [t, a, b] = slot;
        match t {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Bool(false),
            3 => Value::Bool(true),
            4 => Value::Ref(a),
            5 => Value::Num(f64::from_bits(((b as u64 & 0xFFFF_FFFF) << 32) | (a as u64 & 0xFFFF_FFFF))),
            6 => Value::Str(Span { off: a as usize, len: b as usize }),
            9 => Value::Id(a),
            10 => Value::Absent,
            11 => Value::Object,
            t => Value::Other(t),
        }
    }

    pub fn nullish(self) -> bool {
        matches!(self, Value::Undefined | Value::Null | Value::Absent)
    }

    pub fn truthy(self) -> bool {
        match self {
            Value::Bool(b) => b,
            Value::Id(_) | Value::Object => true,
            Value::Num(x) => x != 0.0 && !x.is_nan(),
            Value::Str(s) => s.len > 0,
            _ => false,
        }
    }

    fn folded(self) -> Value {
        if self == Value::Absent { Value::Undefined } else { self }
    }
}

pub fn str_eq(pa: Pool, a: Span, pb: Pool, b: Span) -> bool {
    a.len == b.len && pa.bytes(a.off, a.len).eq(pb.bytes(b.off, b.len))
}

/// `===` of two slots, each with its own packet pool.
pub fn strict_eq(pa: Pool, a: Value, pb: Pool, b: Value) -> bool {
    match (a.folded(), b.folded()) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Ref(x), Value::Ref(y)) | (Value::Id(x), Value::Id(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => str_eq(pa, x, pb, y),
        _ => false,
    }
}

pub fn str_eq_lit(pool: Pool, s: Span, lit: &str) -> bool {
    s.len == lit.len() && pool.bytes(s.off, s.len).eq(lit.bytes())
}

/// A Number that is a safe integer (-0 gives 0).
pub fn num_int(x: f64) -> Option<i64> {
    const SAFE: f64 = 9007199254740991.0;
    (x.fract() == 0.0 && x.abs() <= SAFE).then(|| x as i64)
}

/// The template-literal string `${v}` of a slot, or None outside the supported domain
/// (refs, non-safe-integer Numbers, non-primitive values). `key_len` is `key(..).map(|k| k.len())`.
pub fn key(pool: Pool, v: Value) -> Option<Vec<u16>> {
    let lit = |s: &str| Some(s.encode_utf16().collect());
    match v.folded() {
        Value::Undefined => lit("undefined"),
        Value::Null => lit("null"),
        Value::Bool(b) => lit(if b { "true" } else { "false" }),
        Value::Str(s) => Some(decode_span(pool, s.off, s.len)),
        Value::Num(x) => num_int(x).map(|n| n.to_string().encode_utf16().collect()),
        _ => None,
    }
}

// ---- writing pools ----

/// A standalone growable pool `[units, nbytes, words..]`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PoolBuf {
    pub units: i64,
    pub nbytes: usize,
    pub words: Vec<i64>,
}

impl PoolBuf {
    pub fn view_words(&self) -> Vec<i64> {
        let mut w = vec![self.units, self.nbytes as i64];
        w.extend_from_slice(&self.words);
        w
    }
    fn byte(&self, k: usize) -> u8 {
        ((self.words.get(k / 7).copied().unwrap_or(0) >> (8 * (6 - k % 7))) & 255) as u8
    }
    fn push_byte(&mut self, b: u8) {
        let k = self.nbytes;
        if k / 7 >= self.words.len() {
            self.words.push(0);
        }
        self.words[k / 7] |= (b as i64) << (8 * (6 - k % 7));
        self.nbytes += 1;
    }
    fn ends_high(&self) -> bool {
        let e = self.nbytes;
        e >= 3 && self.byte(e - 3) == 237 && (160..=175).contains(&self.byte(e - 2))
    }
    /// The byte offset where the next append lands: after a separator if a lone high meets a lone low.
    fn join(&mut self, starts_low: bool) -> usize {
        if self.ends_high() && starts_low {
            self.push_byte(255);
        }
        self.nbytes
    }

    /// Append complete forms `off..off+n` of another pool; returns where they start.
    pub fn append_span(&mut self, src: Pool, off: usize, n: usize, units: usize) -> usize {
        let n = n.min(src.nbytes().saturating_sub(off));
        let low = n >= 3 && src.byte(off) == 237 && (176..=191).contains(&src.byte(off + 1));
        let base = self.join(low);
        src.bytes(off, n).for_each(|b| self.push_byte(b));
        self.units += units.min(n) as i64;
        base
    }

    /// Encode UTF-16 units (lone surrogates as three-byte forms); returns where they start.
    pub fn append_units(&mut self, s: &[u16]) -> usize {
        let low = s.first().is_some_and(|&c| (0xDC00..=0xDFFF).contains(&c));
        let base = self.join(low);
        for r in char::decode_utf16(s.iter().copied()) {
            let mut buf = [0u8; 4];
            let bytes: &[u8] = match r {
                Ok(c) => c.encode_utf8(&mut buf).as_bytes(),
                Err(e) => {
                    let c = e.unpaired_surrogate() as u32;
                    buf[..3].copy_from_slice(&[0xE0 | (c >> 12) as u8, 0x80 | ((c >> 6) & 63) as u8, 0x80 | (c & 63) as u8]);
                    &buf[..3]
                }
            };
            bytes.iter().for_each(|&b| self.push_byte(b));
        }
        self.units += s.len() as i64;
        base
    }

    /// `pool_cat`: this pool followed by `b`.
    pub fn cat(&self, b: Pool) -> PoolBuf {
        let mut o = self.clone();
        o.append_span(b, 0, b.nbytes(), b.0.first().copied().unwrap_or(0).max(0) as usize);
        o
    }
}

pub fn encode_pool(s: &[u16]) -> PoolBuf {
    let mut o = PoolBuf::default();
    o.append_units(s);
    o
}

// ---- ordering (UTF-16 code-unit order over the WTF-8 bytes) ----

/// Compare two spans of one pool by UTF-16 code units. Bytes order like code points except where a
/// four-byte form (a pair, units D800..DBFF DC00..DFFF) meets a three-byte surrogate or a BMP unit above DFFF;
/// comparing the lazy unit streams says that directly (pruv `pstr_cmp` walks the bytes and fixes up those cases).
pub fn str_cmp(pool: Pool, a: Span, b: Span) -> Ordering {
    units(pool, a.off, a.len).cmp(units(pool, b.off, b.len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_of(s: &[u16]) -> Vec<i64> {
        encode_pool(s).view_words()
    }
    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn round_trip_and_units() {
        let s = u("aé€😀");
        let w = pool_of(&s);
        let p = Pool(&w);
        assert_eq!(p.nbytes(), 1 + 2 + 3 + 4);
        assert_eq!(units_in(p, 0, p.nbytes()), 5);
        assert_eq!(decode_span(p, 0, p.nbytes()), s);
        assert_eq!(prefix_in(p, 0, p.nbytes(), 4), 6);
        assert_eq!(prefix_in(p, 0, p.nbytes(), 5), 10);
    }

    #[test]
    fn lone_surrogates_get_a_separator() {
        let mut o = encode_pool(&[0xD800]);
        let base = o.append_units(&[0xDC00]);
        assert_eq!(base, 4);
        let w = o.view_words();
        let p = Pool(&w);
        assert_eq!(p.byte(3), 255);
        assert_eq!(units_in(p, 0, p.nbytes()), 2);
        assert_eq!(decode_span(p, 0, p.nbytes()), vec![0xD800, 0xDC00]);
        assert!(!span_ok(p, 0, 7));
        assert!(span_ok(p, 0, 3) && span_ok(p, 4, 3));
        assert!(!span_ok(p, 1, 2));
    }

    #[test]
    fn values() {
        let w = pool_of(&u("ab"));
        let p = Pool(&w);
        let s = Value::parse([6, 0, 2]);
        assert!(s.truthy() && str_eq_lit(p, Span { off: 0, len: 2 }, "ab"));
        assert!(strict_eq(p, s, p, s));
        assert!(strict_eq(p, Value::parse([10, 0, 0]), p, Value::Undefined));
        let nan = Value::Num(f64::NAN);
        assert!(!nan.truthy() && !strict_eq(p, nan, p, nan));
        assert!(strict_eq(p, Value::Num(0.0), p, Value::Num(-0.0)));
        assert_eq!(key(p, Value::Num(-12.0)), Some(u("-12")));
        assert_eq!(key(p, Value::Num(0.5)), None);
        assert_eq!(key(p, s), Some(u("ab")));
        assert!(Value::Absent.nullish() && !Value::Bool(false).nullish());
    }

    #[test]
    fn order_is_utf16() {
        let mut o = encode_pool(&u("\u{FFFF}"));
        o.append_units(&u("😀"));
        let w = o.view_words();
        let p = Pool(&w);
        let a = Span { off: 0, len: 3 };
        let b = Span { off: 3, len: 4 };
        assert_eq!(str_cmp(p, a, b), Ordering::Greater);
    }
}

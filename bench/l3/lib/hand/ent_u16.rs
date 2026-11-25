// Hand Rust baseline for `bench/l3/lib/ent_u16.pruv` (Rust-shape reset step 3.1).
// Same meaning per operation, written the way a Rust programmer writes it over UTF-16 units:
// `&[u16]` borrowed, `Vec<u16>` owned, `usize` indexes. The Pruv module keeps one unit per `i64` word
// and `i64` indexes (gap U, bench/l3/lib/RESHAPE.md); every other line is meant to match.
// Check: rustc --edition 2021 --test ent_u16.rs -o /tmp/ent_u16_hand && /tmp/ent_u16_hand

pub const MAX_ALLOC: usize = 1 << 40;

/// Total read: an index out of range reads 0 (JS `charCodeAt` past the end is checked by the caller this way).
pub fn rd(a: &[u16], i: usize) -> u16 {
    a.get(i).copied().unwrap_or(0)
}

/// Total write: an index out of range is dropped.
pub fn wr(a: &mut [u16], i: usize, v: u16) {
    if let Some(x) = a.get_mut(i) {
        *x = v;
    }
}

pub fn zeros(n: i64) -> Vec<u16> {
    let k = if n < 0 || n as usize > MAX_ALLOC { 0 } else { n as usize };
    vec![0; k]
}

pub fn empty() -> Vec<u16> {
    Vec::new()
}

/// `a[s..e]` with both ends clamped, owned.
pub fn slice(a: &[u16], s: i64, e: i64) -> Vec<u16> {
    let lo = s.max(0) as usize;
    let hi = (e.max(0) as usize).min(a.len());
    if hi <= lo {
        return Vec::new();
    }
    a[lo..hi].to_vec()
}

pub fn cat(a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut o = Vec::with_capacity(a.len() + b.len());
    o.extend_from_slice(a);
    o.extend_from_slice(b);
    o
}

/// ASCII literal as units.
pub fn lit(s: &str) -> Vec<u16> {
    s.bytes().map(u16::from).collect()
}

pub fn one(u: u16) -> Vec<u16> {
    vec![u]
}

pub fn eq(a: &[u16], b: &[u16]) -> bool {
    a == b
}

pub fn eq_lit(a: &[u16], s: &str) -> bool {
    a.len() == s.len() && a.iter().zip(s.bytes()).all(|(&x, y)| x == u16::from(y))
}

/// First index >= from with a[i] == u.
pub fn index_of(a: &[u16], u: u16, from: i64) -> Option<usize> {
    let from = (from.max(0) as usize).min(a.len());
    a[from..].iter().position(|&x| x == u).map(|p| p + from)
}

/// ECMAScript WhiteSpace + LineTerminator of `String.prototype.trim`.
pub fn is_trim_unit(u: u16) -> bool {
    matches!(u, 9..=13 | 32 | 160 | 5760 | 65279 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
}

/// Owned trimmed copy (callers store it).
pub fn trim(a: &[u16]) -> Vec<u16> {
    let mut s = 0;
    while s < a.len() && is_trim_unit(a[s]) {
        s += 1;
    }
    let mut e = a.len();
    while e > s && is_trim_unit(a[e - 1]) {
        e -= 1;
    }
    a[s..e].to_vec()
}

pub fn clamp(x: i64, hi: i64) -> i64 {
    x.clamp(0, hi)
}

/// Copy `m` units of `src` from `s` into `o` at `at`. Positions past the end of `o` are dropped and positions past
/// the end of `src` write 0. Returns `at + m` (m clamped into 0..=2^30).
pub fn put_range(o: &mut [u16], at: usize, src: &[u16], s: usize, m: i64) -> usize {
    let m = m.clamp(0, 1 << 30) as usize;
    let room = m.min(o.len().saturating_sub(at));
    if room > 0 {
        let k = room.min(src.len().saturating_sub(s));
        if k > 0 {
            o[at..at + k].copy_from_slice(&src[s..s + k]);
        }
        o[at + k..at + room].fill(0);
    }
    at + m
}

pub fn put_all(o: &mut [u16], at: usize, src: &[u16]) -> usize {
    put_range(o, at, src, 0, src.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let a = lit("  ab c ");
        assert_eq!(trim(&a), lit("ab c"));
        assert_eq!(slice(&a, -3, 4), lit("  ab"));
        assert_eq!(slice(&a, 5, 2), empty());
        assert_eq!(cat(&lit("x"), &lit("yz")), lit("xyz"));
        assert!(eq_lit(&lit("abc"), "abc"));
        assert_eq!(index_of(&lit("a/b/c"), 47, 2), Some(3));
        let mut o = zeros(6);
        assert_eq!(put_range(&mut o, 4, &lit("pqr"), 1, 3), 7);
        assert_eq!(o, vec![0, 0, 0, 0, 113, 114]);
        let mut o = vec![9u16; 4];
        assert_eq!(put_range(&mut o, 1, &lit("ab"), 1, 3), 4);
        assert_eq!(o, vec![9, 98, 0, 0]);
        assert_eq!(put_range(&mut o, 9, &lit("ab"), 7, 2), 11);
        assert_eq!(put_range(&mut o, 0, &lit("ab"), 7, 2), 2);
        assert_eq!(o, vec![0, 0, 0, 0]);
    }
}

// Hand Rust baseline for `bench/l3/lib/ent_wire.pruv` (Rust-shape reset step 3.2).
// Same meaning per operation, written the way a Rust programmer writes the entry wire check.
// The packet is the external input format (host.rs is the layout authority): a borrowed `&[i64]` of words,
//   packet = [1, kind, op_id, seq, len, op, nsec, len_0..len_{nsec-1}, section data...]
// and section 0 is the packed WTF-8 pool `[nu, nb, W_1..W_m]`, seven bytes per word, high byte first.
// The wire check runs once, on the borrowed words, before anything else reads the packet; it never copies the
// input. `poison` is the only allocation: the packet with its pool replaced by `[-1]`, so the packet layer
// classifies the rejection the same way as before (InputTooLarge before InvalidPacket).
// Check: rustc --edition 2021 --test ent_wire.rs -o /tmp/ent_wire_hand && /tmp/ent_wire_hand

pub const MAX_WORDS: usize = 1 << 24;
/// 2^56 - 1: a pool word holds seven bytes.
const WORD_MAX: i64 = (1 << 56) - 1;
/// 0x80 in every packed byte.
const HIGH_BITS: i64 = 0x0080_8080_8080_8080;

/// Byte `k` of the packed stream (the pool words were range checked first).
fn byte(pool: &[i64], k: usize) -> i64 {
    (pool[2 + k / 7] >> (8 * (6 - k % 7))) & 255
}

fn cont(b: i64) -> bool {
    (128..=191).contains(&b)
}

/// The section table: 2 <= nsec, every length >= 0, the lengths sum to the words after the table.
/// Returns nsec.
fn sections(w: &[i64]) -> Option<usize> {
    let n = w.len();
    if n > MAX_WORDS || n < 8 {
        return None;
    }
    let ns = w[6];
    if ns < 2 || ns > (n - 7) as i64 {
        return None;
    }
    let ns = ns as usize;
    let mut rest = n - 7 - ns;
    for &l in &w[7..7 + ns] {
        if l < 0 || l as usize > rest {
            return None;
        }
        rest -= l as usize;
    }
    if rest != 0 {
        return None;
    }
    Some(ns)
}

/// The packet with section 0 replaced by the one-word pool `[-1]` (env_check rejects it). A packet whose table
/// does not hold section 0 is copied as is, with the same total writes.
pub fn poison(w: &[i64]) -> Vec<i64> {
    let n = w.len();
    let ns = w.get(6).copied().unwrap_or(0);
    let mut o = match pool_bounds(w) {
        Some((p0, l0)) => {
            let m = n - l0 + 1;
            let mut o = Vec::with_capacity(m);
            o.extend_from_slice(&w[..p0]);
            o.push(-1);
            o.extend_from_slice(&w[p0 + l0..]);
            o[4] = m as i64 - 5;
            o[7] = 1;
            o
        }
        None => {
            let mut o = w.to_vec();
            if let Some(x) = o.get_mut(7 + ns.clamp(0, MAX_WORDS as i64) as usize) {
                *x = -1;
            }
            o
        }
    };
    // env_check looks for the pool at 7 + min(nsec, 1000000); above that cap also break the version so it still rejects
    if ns > 1_000_000 {
        if let Some(x) = o.get_mut(0) {
            *x = 0;
        }
    }
    o
}

/// Section 0 as (start, length) when the header and its length fit in the words (the rest of the table is not checked).
fn pool_bounds(w: &[i64]) -> Option<(usize, usize)> {
    let n = w.len();
    if n < 8 || n > MAX_WORDS {
        return None;
    }
    let ns = w[6];
    if ns < 2 || ns > (n - 7) as i64 {
        return None;
    }
    let p0 = 7 + ns as usize;
    let l0 = w[7];
    if l0 < 0 || l0 as usize > n - p0 {
        return None;
    }
    Some((p0, l0 as usize))
}

/// false only for a well-formed envelope whose pool is not valid packed WTF-8. Malformed envelopes go to the
/// packet layer for the existing error classification.
pub fn wire_in(w: &[i64]) -> bool {
    let Some(ns) = sections(w) else { return true };
    if w[4] != w.len() as i64 - 5 {
        return true;
    }
    let p0 = 7 + ns;
    pool_ok(w, p0, w[7])
}

/// The pool at `w[p0..p0 + l0]`, if it lies inside `w`.
pub fn pool_ok(w: &[i64], p0: usize, l0: i64) -> bool {
    if p0 > 1 << 25 || l0 < 2 || l0 as usize > MAX_WORDS {
        return false;
    }
    match w.get(p0..p0 + l0 as usize) {
        Some(pool) => pool_valid(pool),
        None => false,
    }
}

/// A packed WTF-8 pool: the exact word count, every word in seven bytes, valid forms (a lone high and a lone low
/// surrogate may be split by the 0xFF separator, which counts no unit), zero padding.
pub fn pool_valid(pool: &[i64]) -> bool {
    let l0 = pool.len();
    if l0 < 2 || l0 > MAX_WORDS {
        return false;
    }
    let (nu, nb) = (pool[0], pool[1]);
    let cap = 7 * (l0 as i64 - 2);
    if nu < 0 || nu > MAX_WORDS as i64 || nb < nu || nb > cap || nb > 3 * nu + nu / 2 {
        return false;
    }
    // No trailing zero words and no missing word: the pool is exactly as long as its bytes need.
    if l0 as i64 != 2 + (nb + 6) / 7 {
        return false;
    }
    // Equal counts imply an ASCII-only stream: check all seven high bits in each word with the word range.
    let ascii = nb == nu;
    if !pool[2..].iter().all(|&x| (0..=WORD_MAX).contains(&x) && !(ascii && x & HIGH_BITS != 0)) {
        return false;
    }
    let nb = nb as usize;
    if ascii {
        return padding_ok(pool, nb);
    }
    let at = |k: usize| if k < nb { byte(pool, k) } else { -1 };
    let (mut u, mut k, mut hi3) = (0usize, 0usize, false);
    while k < nb {
        let b0 = byte(pool, k);
        if b0 < 128 {
            k += 1;
            u += 1;
            hi3 = false;
        } else if b0 == 255 {
            // The separator byte: only between a lone high and a lone low.
            if !hi3 || !(k + 3 < nb && at(k + 1) == 237 && (176..=191).contains(&at(k + 2))) {
                return false;
            }
            k += 1;
            hi3 = false;
        } else if b0 < 194 {
            return false;
        } else if b0 < 224 {
            if !cont(at(k + 1)) {
                return false;
            }
            k += 2;
            u += 1;
            hi3 = false;
        } else if b0 < 240 {
            let (b1, b2) = (at(k + 1), at(k + 2));
            if !cont(b1) || !cont(b2) || (b0 == 224 && b1 < 160) {
                return false;
            }
            let v = (b0 - 224) * 4096 + (b1 - 128) * 64 + (b2 - 128);
            if hi3 && (0xDC00..=0xDFFF).contains(&v) {
                return false;
            }
            hi3 = (0xD800..=0xDBFF).contains(&v);
            k += 3;
            u += 1;
        } else if b0 < 245 {
            let (b1, b2, b3) = (at(k + 1), at(k + 2), at(k + 3));
            if !cont(b1) || !cont(b2) || !cont(b3) || (b0 == 240 && b1 < 144) || (b0 == 244 && b1 > 143) {
                return false;
            }
            k += 4;
            u += 2;
            hi3 = false;
        } else {
            return false;
        }
    }
    k == nb && u == nu as usize && padding_ok(pool, nb)
}

/// The bytes after the stream up to the end of its last word are zero.
fn padding_ok(pool: &[i64], nb: usize) -> bool {
    let end = nb.div_ceil(7) * 7;
    (nb..end).all(|k| byte(pool, k) == 0)
}

/// Pools have one canonical length (pool_valid checks it exactly), so equal awaits are equal word for word.
pub fn same_packet(a: &[i64], b: &[i64]) -> bool {
    a == b && wire_in(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack WTF-8 bytes into a pool.
    fn pool(bytes: &[u8], units: i64) -> Vec<i64> {
        let mut p = vec![units, bytes.len() as i64];
        for c in bytes.chunks(7) {
            let mut x = 0i64;
            for i in 0..7 {
                x = (x << 8) | c.get(i).copied().unwrap_or(0) as i64;
            }
            p.push(x);
        }
        p
    }

    fn packet(pool: &[i64], rec: &[i64]) -> Vec<i64> {
        let mut w = vec![1, 0, 1, 0, 0, 2, 2, pool.len() as i64, rec.len() as i64];
        w.extend_from_slice(pool);
        w.extend_from_slice(rec);
        w[4] = w.len() as i64 - 5;
        w
    }

    #[test]
    fn pools() {
        assert!(pool_valid(&pool(b"abc", 3)));
        assert!(pool_valid(&pool("é€😀".as_bytes(), 4)));
        assert!(!pool_valid(&pool(b"\xc0\x80", 1)));
        // lone high, separator, lone low
        assert!(pool_valid(&pool(b"\xed\xa0\x80\xff\xed\xb0\x80", 2)));
        assert!(!pool_valid(&pool(b"\xed\xa0\x80\xed\xb0\x80", 2)));
        let mut extra = pool(b"abc", 3);
        extra.push(0);
        assert!(!pool_valid(&extra));
        let mut pad = pool(b"abc", 3);
        pad[2] |= 1;
        assert!(!pool_valid(&pad));
    }

    #[test]
    fn packets() {
        let w = packet(&pool(b"hi", 2), &[0, 0, 0]);
        assert!(wire_in(&w));
        assert!(same_packet(&w, &w.clone()));
        let bad = packet(&pool(b"\xff", 1), &[0, 0, 0]);
        assert!(!wire_in(&bad));
        let p = poison(&bad);
        assert_eq!(p.len(), bad.len() - 3 + 1);
        assert_eq!((p[7], p[9]), (1, -1));
        assert_eq!(p[4], p.len() as i64 - 5);
        assert!(wire_in(&[1, 2, 3]));
    }
}

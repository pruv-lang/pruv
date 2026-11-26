// Hand Rust baseline for `bench/l3/lib/ent_packet.pruv` (Rust-shape reset step 3.2).
// Same meaning per operation, written the way a Rust programmer writes the entry packet layer.
// The packet is the external input and output format (host.rs is the layout authority), so it stays `&[i64]` in
// and `Vec<i64>` out:
//   packet  = [1, kind, op_id, seq, len, op, nsec, len_0..len_{nsec-1}, section data...]   total words = 5 + len
//   section 0 = packed WTF-8 pool, section 1 = record, sections >= 2 = tables or state/rows/request tables.
//   VS (value slot) = [tag, a, b], 3 words.
// A record check borrows the packet once in a `Check` (section starts built once, claimed sections, pool
// high-water, a bad flag) and walks the record slots; nothing is copied out of the packet. Reads of record
// words are total (`rd`): the envelope was checked first, so they are defence in depth.
// `logical_size` and `span_ok` belong to ent_value (step 3.3); they are written here so the file stands alone.
// Check: rustc --edition 2021 --test ent_packet.rs -o /tmp/ent_packet_hand && /tmp/ent_packet_hand

#[path = "ent_wire.rs"]
pub mod ent_wire;

pub const MAX_WORDS: i64 = 1 << 24;
const MAX_SECTIONS: i64 = 1_000_000;

// tag masks: bit t set = tag t allowed
pub const ANY: i64 = 1647; // 0 1 2 3 5 6 9 10
pub const THREAD: i64 = 3075; // 0 1 10 11
pub const FORKS: i64 = 1155; // 0 1 10 7
pub const ETS: i64 = 1283; // 0 1 10 8
pub const BOUND: i64 = 3183; // 0 1 2 3 5 6 10 11
pub const BOOL: i64 = 12; // 2 3
pub const STR: i64 = 64; // 6
pub const ARR: i64 = 128; // 7
pub const REF: i64 = 512; // 9
pub const UNDEF: i64 = 1; // 0
pub const USTR: i64 = 65; // 0 6

/// Envelope rejection: `env_check` codes 1 and 2 (boundary done-packet codes 0 and 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    InvalidPacket = 1,
    InputTooLarge = 2,
}

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

fn wr(o: &mut [i64], i: usize, v: i64) {
    if let Some(x) = o.get_mut(i) {
        *x = v;
    }
}

fn clampu(x: i64, hi: i64) -> usize {
    x.clamp(0, hi) as usize
}

// ---- sections (total over any words, used on packets this process built as well) ----
pub fn nsec(w: &[i64]) -> usize {
    clampu(rd(w, 6), MAX_SECTIONS)
}

pub fn sec_len(w: &[i64], i: usize) -> usize {
    if i >= nsec(w) {
        return 0;
    }
    clampu(rd(w, 7 + i), MAX_WORDS)
}

pub fn sec_start(w: &[i64], i: usize) -> usize {
    let ns = nsec(w);
    let at = 7 + ns + (0..i.min(ns)).map(|j| clampu(rd(w, 7 + j), MAX_WORDS)).sum::<usize>();
    at.min(1 << 25)
}

/// starts[j] = absolute start of section j of the packet at `head`, for 0 <= j <= nsec.
pub fn section_starts(w: &[i64], head: usize) -> Vec<usize> {
    let ns = clampu(rd(w, head + 6), MAX_SECTIONS);
    let mut starts = Vec::with_capacity(ns + 1);
    let mut at = head + 7 + ns;
    for j in 0..=ns {
        starts.push(at.min(1 << 25));
        if j < ns {
            at += clampu(rd(w, head + 7 + j), MAX_WORDS);
        }
    }
    starts
}

pub fn start_in(starts: &[usize], i: usize) -> usize {
    starts[i.min(starts.len() - 1)]
}

// ---- envelope ----
fn logical_size(w: &[i64]) -> usize {
    let l0 = sec_len(w, 0);
    if l0 < 2 || l0 > w.len() {
        return w.len();
    }
    let rest = w.len() - l0;
    if rest > MAX_WORDS as usize {
        return rest;
    }
    rest + clampu(rd(w, sec_start(w, 0)), MAX_WORDS)
}

/// Structure only: version, kind, ids, section header, packed pool validation.
pub fn env_check(w: &[i64]) -> Result<(), Reject> {
    env_check_at(w, false)
}

/// Preserve wire rejection before the logical size check, including the poisoned envelope's size.
pub fn env_check_wire(w: &[i64]) -> Result<(), Reject> {
    if !ent_wire::wire_in(w) {
        return env_check_at(&ent_wire::poison(w), false);
    }
    env_check_at(w, w.len() <= MAX_WORDS as usize)
}

fn env_check_at(w: &[i64], pool_checked: bool) -> Result<(), Reject> {
    use Reject::*;
    if logical_size(w) > MAX_WORDS as usize {
        return Err(InputTooLarge);
    }
    let n = w.len();
    if n < 7 || w[0] != 1 || !(0..=3).contains(&w[1]) || w[2] < 1 || w[3] < 0 || w[4] != n as i64 - 5 {
        return Err(InvalidPacket);
    }
    if !(1..=9).contains(&w[5]) {
        return Err(InvalidPacket);
    }
    let ns = w[6];
    if ns < 2 || ns > n as i64 - 7 {
        return Err(InvalidPacket);
    }
    let ns = ns as usize;
    let mut rest = n - 7 - ns;
    for &l in &w[7..7 + ns] {
        if l < 0 || l as usize > rest {
            return Err(InvalidPacket);
        }
        rest -= l as usize;
    }
    if rest != 0 || ns > MAX_SECTIONS as usize || (!pool_checked && !ent_wire::pool_ok(w, 7 + ns, w[7])) {
        return Err(InvalidPacket);
    }
    Ok(())
}

// ---- value slots ----
pub fn in_mask(mask: i64, tag: i64) -> bool {
    (0..=11).contains(&tag) && (mask >> tag) & 1 == 1
}

/// n words from p are zero (words past the end read as zero).
pub fn zero_run(w: &[i64], p: usize, n: usize) -> bool {
    (p..p + n).all(|i| rd(w, i) == 0)
}

fn cont(b: i64) -> bool {
    (128..192).contains(&b)
}

fn pool_byte(w: &[i64], pool: usize, k: usize) -> i64 {
    (rd(w, pool + 2 + k / 7) >> (8 * (6 - k % 7))) & 255
}

/// A string span of the pool at `pool`: inside the stream, on form boundaries, not covering the 0xFF separator.
fn span_ok(w: &[i64], pool: usize, off: i64, n: i64) -> bool {
    let nb = rd(w, pool + 1).clamp(0, (1 << 30) - 1);
    if off < 0 || n < 0 || off > nb || n > nb - off {
        return false;
    }
    let (off, end, nb) = (off as usize, (off + n) as usize, nb as usize);
    (off == nb || !cont(pool_byte(w, pool, off)))
        && (end == nb || !cont(pool_byte(w, pool, end)))
        && (off..end).all(|k| pool_byte(w, pool, k) != 255)
}

/// One record check over a borrowed packet.
pub struct Check<'a> {
    w: &'a [i64],
    starts: Vec<usize>,
    claimed: Vec<bool>,
    pool_hi: i64,
    bad: bool,
}

impl<'a> Check<'a> {
    pub fn new(w: &'a [i64]) -> Self {
        let starts = section_starts(w, 0);
        let claimed = vec![false; starts.len() - 1];
        Check { w, starts, claimed, pool_hi: 0, bad: false }
    }

    pub fn start(&self, i: usize) -> usize {
        start_in(&self.starts, i)
    }

    /// Check the slot at p against a tag mask. width > 0: a table slot (tag 7/8) whose section must hold exactly
    /// count * width words; the section is claimed. width == 0: table tags are rejected. A failed check only sets
    /// the flag; the callers go on and the flag is read once at the end.
    pub fn slot(&mut self, p: usize, mask: i64, width: i64) {
        let w = self.w;
        let (tag, a, b) = (rd(w, p), rd(w, p + 1), rd(w, p + 2));
        let ok = in_mask(mask, tag)
            && match tag {
                0..=3 | 10 | 11 => a == 0 && b == 0,
                4 => b == 0,
                5 => (0..=u32::MAX as i64).contains(&a) && (0..=u32::MAX as i64).contains(&b),
                6 => {
                    let ok = span_ok(w, self.starts[0], a, b);
                    if ok {
                        self.pool_hi = self.pool_hi.max(a + b);
                    }
                    ok
                }
                9 => a >= 1 && b == 0,
                _ => {
                    // tags 7 and 8
                    let ok = width > 0
                        && a >= 2
                        && (a as usize) < self.claimed.len()
                        && (0..=MAX_WORDS).contains(&b)
                        && rd(w, 7 + a as usize) == b * width.min(64)
                        && !self.claimed[a as usize];
                    if ok {
                        self.claimed[a as usize] = true;
                    }
                    ok
                }
            };
        if !ok {
            self.bad = true;
        }
    }

    /// The pool high-water, or None when a slot failed or a table section was not claimed.
    pub fn finish(&self) -> Option<i64> {
        if self.bad || self.claimed.iter().skip(2).any(|&c| !c) {
            return None;
        }
        Some(self.pool_hi)
    }

    /// rows of width 3 (one VS each, tags in mask) in the table slot at p; the slot itself was already checked
    fn rows1(&mut self, p: usize, mask: i64) {
        if rd(self.w, p) != 7 {
            return;
        }
        let base = self.start(clampu(rd(self.w, p + 1), MAX_SECTIONS));
        let cnt = clampu(rd(self.w, p + 2), MAX_WORDS);
        for i in 0..cnt {
            self.slot(base + 3 * i, mask, 0);
        }
    }

    /// engineThreads entry rows: key {6}, bound, threadId (any if bound is 11, else a zero slot)
    fn ets(&mut self, p: usize) {
        if rd(self.w, p) != 8 {
            return;
        }
        let base = self.start(clampu(rd(self.w, p + 1), MAX_SECTIONS));
        let cnt = clampu(rd(self.w, p + 2), MAX_WORDS);
        for i in 0..cnt {
            let q = base + 9 * i;
            self.slot(q, STR, 0);
            self.slot(q + 3, BOUND, 0);
            if rd(self.w, q + 3) == 11 {
                self.slot(q + 6, ANY, 0);
            } else if !zero_run(self.w, q + 6, 3) {
                self.bad = true;
            }
        }
    }

    /// one session record (10 slots = 30 words) at p
    fn session(&mut self, p: usize) {
        self.slot(p, ANY, 0);
        self.slot(p + 3, ANY, 0);
        self.slot(p + 6, ANY, 0);
        self.slot(p + 9, THREAD, 0);
        if rd(self.w, p + 9) == 11 {
            self.slot(p + 12, ANY, 0);
            self.slot(p + 15, ANY, 0);
            self.slot(p + 18, FORKS, 3);
            self.rows1(p + 18, ANY);
        } else if !zero_run(self.w, p + 12, 9) {
            self.bad = true;
        }
        self.slot(p + 21, THREAD, 0);
        if rd(self.w, p + 21) == 11 {
            self.slot(p + 24, ANY, 0);
        } else if !zero_run(self.w, p + 24, 3) {
            self.bad = true;
        }
        self.slot(p + 27, ETS, 9);
        self.ets(p + 27);
    }

    /// error observation E (6 slots = 18 words) at p
    fn err(&mut self, p: usize) {
        self.slot(p, REF, 0);
        self.slot(p + 3, BOOL, 0);
        self.slot(p + 6, BOOL, 0);
        self.slot(p + 9, ANY, 0);
        let message = if rd(self.w, p + 6) == 3 { STR } else { UNDEF };
        self.slot(p + 12, message, 0);
        self.slot(p + 15, ANY, 0);
    }
}

// ---- record checks ----

/// Input record check for ops 2,3,4,7,8,9 (kind 0, envelope already ok): the pool high-water, or None.
pub fn input_check(w: &[i64]) -> Option<i64> {
    let mut c = Check::new(w);
    let r0 = c.start(1);
    let rl = sec_len(w, 1);
    match rd(w, 5) {
        2 if rl == 6 => {
            c.slot(r0, ANY, 0);
            c.slot(r0 + 3, ARR, 3);
            c.rows1(r0 + 3, ANY);
        }
        3 if rl == 12 => {
            for (k, m) in [ANY, REF, ANY, REF].into_iter().enumerate() {
                c.slot(r0 + 3 * k, m, 0);
            }
        }
        4 if rl == 15 => {
            for (k, m) in [ANY, REF, ANY, ANY, REF].into_iter().enumerate() {
                c.slot(r0 + 3 * k, m, 0);
            }
        }
        7 | 8 if rl == 30 => c.session(r0),
        9 if rl == 3 => {
            c.slot(r0, ARR, 30);
            let cnt = clampu(rd(w, r0 + 2), MAX_WORDS);
            let base = c.start(clampu(rd(w, r0 + 1), MAX_SECTIONS));
            for i in 0..cnt {
                c.session(base + 30 * i);
            }
        }
        _ => return None,
    }
    c.finish()
}

/// Completion record check for the outstanding request `rcode` (1..9). aux: for 8 the number of ids sent, for 9
/// the opKind (0 list / 1 read). Envelope already ok.
pub fn comp_check(w: &[i64], rcode: i64, aux: i64) -> Option<i64> {
    let mut c = Check::new(w);
    let r0 = c.start(1);
    let rl = sec_len(w, 1);
    if rl < 1 {
        return None;
    }
    let st = rd(w, r0);
    if rcode != 8 && st != 0 && st != 1 {
        return None;
    }
    let failed = st == 1 && matches!(rcode, 1 | 4 | 5 | 6 | 7 | 9);
    if failed {
        if rl != 19 {
            return None;
        }
        c.err(r0 + 1);
        return c.finish();
    }
    match rcode {
        1 => {
            let cnt = rd(w, r0 + 1);
            if rl != 2 || !(0..=MAX_WORDS).contains(&cnt) {
                return None;
            }
        }
        2 => {
            let kind = rd(w, r0 + 1);
            if st != 0 || rl != 8 || (kind != 0 && kind != 1) {
                return None;
            }
            c.slot(r0 + 2, ANY, 0);
            c.slot(r0 + 5, ANY, 0);
        }
        3 => {
            if st != 0 || rl != 4 {
                return None;
            }
            c.slot(r0 + 1, USTR, 0);
        }
        4 | 5 | 6 => {
            if rl != 4 {
                return None;
            }
            c.slot(r0 + 1, STR, 0);
        }
        7 => {
            if rl != 7 {
                return None;
            }
            c.slot(r0 + 1, STR, 0);
            c.slot(r0 + 4, STR, 0);
        }
        8 => {
            // [0, results {7} rows of one any, thrown 0/1, E (18 words)]
            if rl != 23 || st != 0 {
                return None;
            }
            c.slot(r0 + 1, ARR, 3);
            c.rows1(r0 + 1, ANY);
            let (n, thrown) = (rd(w, r0 + 3), rd(w, r0 + 4));
            match thrown {
                0 if n == aux && zero_run(w, r0 + 5, 18) => {}
                1 if n < aux => c.err(r0 + 5),
                _ => return None,
            }
        }
        9 => {
            if rl != 4 {
                return None;
            }
            if aux == 0 {
                c.slot(r0 + 1, ARR, 6);
                // rows [rowRef {9}, title {6}]
                let base = c.start(clampu(rd(w, r0 + 2), MAX_SECTIONS));
                let cnt = clampu(rd(w, r0 + 3), MAX_WORDS);
                for i in 0..cnt {
                    c.slot(base + 6 * i, REF, 0);
                    c.slot(base + 6 * i + 3, STR, 0);
                }
            } else {
                c.slot(r0 + 1, REF, 0);
            }
        }
        _ => return None,
    }
    c.finish()
}

// ---- builders ----
pub fn put_vs(o: &mut [i64], at: usize, tag: i64, a: i64, b: i64) {
    wr(o, at, tag);
    wr(o, at + 1, a);
    wr(o, at + 2, b);
}

/// A boundary done packet: [5, code] with an empty pool; sections pool, record, state, rows.
/// code 0 InvalidPacket, 1 InputTooLarge, 2 InternalInvariant. op 0 when unknown.
pub fn boundary(op: i64, op_id: i64, code: i64) -> Vec<i64> {
    let id = op_id.max(1);
    let c = if (0..=2).contains(&code) { code } else { 2 };
    let p = if (0..=9).contains(&op) { op } else { 0 };
    vec![1, 3, id, 0, 10, p, 4, 2, 2, 0, 0, 0, 0, 5, c]
}

/// Assemble a packet. Sections: pool, rec, state, rows, then one table when ntab != 0 (else tab is ignored).
/// An output that would exceed 2^24 words becomes a done packet with InputTooLarge, never a truncation.
#[allow(clippy::too_many_arguments)]
pub fn mk_packet(kind: i64, op_id: i64, seq: i64, op: i64, pool: &[i64], rec: &[i64], state: &[i64], rows: &[i64], tab: &[i64], ntab: i64) -> Vec<i64> {
    let lp = if pool.is_empty() { 2 } else { pool.len() as i64 };
    let (lr, ls, lw) = (rec.len() as i64, state.len() as i64, rows.len() as i64);
    let (lt, ns) = if ntab == 0 { (0, 4) } else { (tab.len() as i64, 5) };
    if [lp, lr, ls, lw, lt].iter().any(|&l| l > MAX_WORDS) {
        return boundary(op, op_id, 1);
    }
    let total = 5 + 2 + ns + lp + lr + ls + lw + lt;
    if total > MAX_WORDS || total - lp + rd(pool, 0).clamp(0, MAX_WORDS) > MAX_WORDS {
        return boundary(op, op_id, 1);
    }
    let mut o = Vec::with_capacity(total as usize);
    o.extend_from_slice(&[1, kind, op_id, seq, total - 5, op, ns, lp, lr, ls, lw]);
    if ns == 5 {
        o.push(lt);
    }
    if pool.is_empty() {
        o.extend_from_slice(&[0, 0]);
    } else {
        o.extend_from_slice(pool);
    }
    o.extend_from_slice(rec);
    o.extend_from_slice(state);
    o.extend_from_slice(rows);
    if ns == 5 {
        o.extend_from_slice(tab);
    }
    o
}

pub fn empty_state() -> Vec<i64> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_check() {
        // pool "ab", record [6, 0, 2, 7, 2, 1], one table section of one row [6, 0, 1]
        let pool = [2, 2, (97 << 48) | (98 << 40)];
        let mut w = vec![1, 0, 1, 0, 0, 2, 3, 3, 6, 3];
        w.extend_from_slice(&pool);
        w.extend_from_slice(&[6, 0, 2, 7, 2, 1, 6, 0, 1]);
        w[4] = w.len() as i64 - 5;
        assert_eq!(env_check(&w), Ok(()));
        assert_eq!(env_check_wire(&w), Ok(()));
        assert_eq!(input_check(&w), Some(2));
        let mut bad = w.clone();
        bad[10 + 3 + 6] = 9; // the row slot tag
        assert_eq!(input_check(&bad), None);
        // a fourth section nobody claims
        let built = mk_packet(0, 1, 0, 2, &pool, &[6, 0, 2, 7, 2, 1], &[6, 0, 1], &[], &[], 0);
        assert_eq!(env_check(&built), Ok(()));
        assert_eq!(input_check(&built), None);
        let b = boundary(12, 0, 7);
        assert_eq!(b, vec![1, 3, 1, 0, 10, 0, 4, 2, 2, 0, 0, 0, 0, 5, 2]);
        assert_eq!(env_check(&b[..6]), Err(Reject::InvalidPacket));
        assert_eq!(sec_start(&w, 2), section_starts(&w, 0)[2]);
        let big = mk_packet(0, 1, 0, 2, &[MAX_WORDS, 0, 0], &[], &[], &[], &[], 0);
        assert_eq!(big[13], 5);
    }
}

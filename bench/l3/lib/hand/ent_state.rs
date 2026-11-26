// Hand Rust baseline for `bench/l3/lib/ent_state.pruv` (Rust-shape reset step 3.4).
// Same meaning per operation, written the way a Rust programmer writes the entry replay context.
// An await packet carries (section 2) the state words `[magic, op, code, aux, ck0..7, L, G, blob, log]`: the
// validated input packet (the blob) and the log of the answers received so far. A step appends the new completion to
// the log and re-runs the item; an observation takes the next log entry, or records the request to emit and stops.
// The state words are external input: they are parsed once into a borrowed `State` (no re-slicing per field). The
// replay context is one `Ctx` that every item takes as `&mut Ctx`; an observation returns `Result<(), Halt>`, so an
// item writes `cx.obs(req)?;` instead of testing flags after every call. The outstanding request and the request
// recorded in prev are `Option<Request>` / `Option<Prev>`, the result is `Option<Output>`, the fault `Option<Fault>`.
// Log entries are `[n, rcode, aux, completion packet]` (n the entry length); the answer is a range into the log.
// Check: rustc --edition 2021 --test ent_state.rs -o /tmp/ent_state_hand && /tmp/ent_state_hand

#![allow(dead_code)]

#[path = "ent_packet.rs"]
pub mod ent_packet;
#[path = "ent_value.rs"]
pub mod ent_value;

use ent_packet::{boundary, comp_check, env_check, input_check, mk_packet, nsec, put_vs, sec_len, sec_start};
use ent_value::Pool;
use std::ops::Range;

pub const MAGIC: i64 = 1162761298;
const MAX_WORDS: usize = 1 << 24;
const CK_MAX: i64 = 1 << 32;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

fn clampu(x: i64, hi: i64) -> usize {
    x.clamp(0, hi) as usize
}

fn pool_capacity(nb: usize) -> usize {
    2 + nb.div_ceil(7)
}

/// The packed pool at word `s`: its words up to the referenced byte count.
fn pool_words(w: &[i64], s: usize) -> &[i64] {
    let p = Pool::at(w, s);
    let end = (s + pool_capacity(p.nbytes())).min(s + sec_len(w, 0)).min(w.len());
    &w[s.min(end)..end]
}

/// The input pool prefix that the request's string slots refer to (pruv `ent_value::blob_pool`).
pub fn blob_pool(blob: &[i64]) -> &[i64] {
    pool_words(blob, sec_start(blob, 0))
}

/// `prefix` (a pool) is a byte prefix of `pool`.
fn pool_starts_with(pool: &[i64], prefix: &[i64]) -> bool {
    let (a, b) = (Pool(pool), Pool(prefix));
    let n = b.nbytes();
    n <= a.nbytes() && (0..n).all(|k| a.byte(k) == b.byte(k))
}

// ---- repack: a copy of packet w whose pool is truncated to its first hi bytes ----
pub fn repack(w: &[i64], hi: i64) -> Vec<i64> {
    let s0 = sec_start(w, 0);
    let s1 = sec_start(w, 1).min(w.len());
    let src = Pool::at(w, s0);
    let cut = clampu(hi, src.nbytes() as i64);
    let units = if cut == src.nbytes() { clampu(rd(w, s0), cut as i64) } else { ent_value::units_in(src, 0, cut) };
    let l = pool_capacity(cut);
    let mut o = Vec::with_capacity(w.len() - sec_len(w, 0) + l);
    o.extend_from_slice(&w[..s0.min(w.len())]);
    o.push(units as i64);
    o.push(cut as i64);
    let full = cut / 7;
    o.extend_from_slice(&src.0[2..2 + full]);
    if cut % 7 != 0 {
        let shift = 8 * (7 - cut % 7);
        o.push(rd(src.0, 2 + full) & !((1i64 << shift) - 1));
    }
    o.extend_from_slice(&w[s1..]);
    o[4] = o.len() as i64 - 5;
    o[7] = l as i64;
    o
}

// ---- log ----

/// One log entry: the request it answers and the completion packet.
pub struct Entry<'a> {
    pub len: usize,
    pub code: i64,
    pub aux: i64,
    pub packet: &'a [i64],
}

/// The entry at word `pos` of the log, if its length word fits.
fn entry_at(lg: &[i64], pos: usize) -> Option<Entry<'_>> {
    let n = rd(lg, pos);
    if n < 12 || n as usize > lg.len() - pos {
        return None;
    }
    let n = n as usize;
    Some(Entry { len: n, code: lg[pos + 1], aux: lg[pos + 2], packet: &lg[pos + 3..pos + n] })
}

/// The log entries in order; stops at the first entry whose length word does not fit.
fn entries(lg: &[i64]) -> impl Iterator<Item = Option<Entry<'_>>> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos >= lg.len() {
            return None;
        }
        let e = entry_at(lg, pos);
        pos = match &e {
            Some(e) => pos + e.len,
            None => lg.len(),
        };
        Some(e)
    })
}

/// Every entry of the log is a well-formed, canonical completion for op_id.
pub fn log_ok(lg: &[i64], op_id: i64) -> bool {
    entries(lg).all(|e| {
        let Some(e) = e else { return false };
        if !(1..=9).contains(&e.code) || !(0..=MAX_WORDS as i64).contains(&e.aux) || (e.code == 9 && e.aux > 1) {
            return false;
        }
        let pk = e.packet;
        env_check(pk).is_ok()
            && rd(pk, 1) == 2
            && rd(pk, 2) == op_id
            && comp_check(pk, e.code, e.aux) == Some(Pool::at(pk, sec_start(pk, 0)).nbytes() as i64)
    })
}

/// Expanded words of the log (the replay budget counts the logical pool size, not the packed words).
pub fn log_words(lg: &[i64]) -> usize {
    if lg.len() > MAX_WORDS {
        return lg.len();
    }
    let mut words = 0;
    for e in entries(lg) {
        if words > MAX_WORDS {
            break;
        }
        let Some(e) = e else { return MAX_WORDS + 1 };
        let n = e.len as i64;
        let ns = rd(e.packet, 6);
        let lp = rd(e.packet, 7);
        if ns < 2 || ns > n - 10 || lp < 2 || lp > n - 10 - ns {
            return MAX_WORDS + 1;
        }
        words += (n - lp) as usize + clampu(rd(e.packet, 7 + ns as usize), MAX_WORDS as i64);
    }
    words
}

/// The log plus one entry for the completion `comp` (answer to rc/ax); hi is its comp_check result.
pub fn log_append(lg: &[i64], rc: i64, ax: i64, comp: &[i64], hi: i64) -> Vec<i64> {
    let pk = repack(comp, hi);
    if lg.len() > MAX_WORDS || pk.len() > MAX_WORDS {
        return vec![0; MAX_WORDS + 1];
    }
    let mut o = Vec::with_capacity(lg.len() + 3 + pk.len());
    o.extend_from_slice(lg);
    o.extend_from_slice(&[pk.len() as i64 + 3, rc, ax]);
    o.extend_from_slice(&pk);
    o
}

// ---- state words ----

pub fn encode_state(op: i64, code: i64, aux: i64, ck: &[i64; 8], blob: &[i64], lg: &[i64]) -> Vec<i64> {
    if blob.len() > MAX_WORDS || lg.len() > MAX_WORDS || 14 + blob.len() + lg.len() > MAX_WORDS {
        return vec![0; MAX_WORDS + 1];
    }
    let mut o = Vec::with_capacity(14 + blob.len() + lg.len());
    o.extend_from_slice(&[MAGIC, op, code, aux]);
    o.extend_from_slice(ck);
    o.extend_from_slice(&[blob.len() as i64, lg.len() as i64]);
    o.extend_from_slice(blob);
    o.extend_from_slice(lg);
    o
}

/// request code / aux valid for an await emitted by this core for op
fn pair_ok(op: i64, code: i64, aux: i64) -> bool {
    match op {
        3 => code == 9 && aux == 0,
        4 => code == 9 && aux == 1,
        8 | 9 => code == 8 && (1..=MAX_WORDS as i64).contains(&aux),
        _ => false,
    }
}

/// The state words of an await, parsed once (borrowing the state section).
pub struct State<'a> {
    pub code: i64,
    pub aux: i64,
    pub ck: &'a [i64],
    pub blob: &'a [i64],
    pub log: &'a [i64],
}

impl<'a> State<'a> {
    /// The fields of any state words (total: missing words read as zero, ranges clamped).
    pub fn view(st: &'a [i64]) -> Self {
        let n = st.len();
        let l = clampu(rd(st, 12), MAX_WORDS as i64);
        let g = clampu(rd(st, 13), MAX_WORDS as i64);
        let (a, b) = ((14 + l).min(n), (14 + l + g).min(n));
        State { code: rd(st, 2), aux: rd(st, 3), ck: &st[4.min(n)..12.min(n)], blob: &st[14.min(a)..a], log: &st[a..b] }
    }

    /// The state of an await for (op, op_id), or None (an InvalidPacket for the caller).
    pub fn parse(st: &'a [i64], op: i64, op_id: i64) -> Option<Self> {
        let n = st.len();
        if !(14..=MAX_WORDS).contains(&n) || st[0] != MAGIC || st[1] != op || !pair_ok(op, st[2], st[3]) {
            return None;
        }
        if st[4..12].iter().any(|&c| !(0..=CK_MAX).contains(&c)) {
            return None;
        }
        let (l, g) = (st[12], st[13]);
        if l < 13 || g < 0 || l as usize > n || g as usize > n || 14 + (l + g) as usize != n {
            return None;
        }
        let s = State::view(st);
        let blob = s.blob;
        if env_check(blob).is_err() || rd(blob, 1) != 0 || rd(blob, 3) != 0 || rd(blob, 5) != op || rd(blob, 2) != op_id {
            return None;
        }
        if input_check(blob) != Some(Pool::at(blob, sec_start(blob, 0)).nbytes() as i64) || !log_ok(s.log, op_id) {
            return None;
        }
        Some(s)
    }

    /// Everything R1 needs from prev's state: checkpoint words and log continuity (seq of the awaiting packet).
    pub fn replay_ok(&self, op: i64, seq: i64) -> bool {
        ck_ok(op, self.code, self.ck) && log_cont(self.log, op, seq, rd(self.ck, 7), s0_of(op, self.ck))
    }
}

// ---- R1: replay validation of prev ----

/// R1(b): continuity of prev's log (see the pruv comment: s0 + d + kept entries == seq).
pub fn log_cont(lg: &[i64], op: i64, seq: i64, d: i64, s0: i64) -> bool {
    if !(1..=1 << 53).contains(&seq) || s0 < 1 || s0 > seq {
        return false;
    }
    let mut cnt = 0;
    let mut ps = s0 - 1;
    for e in entries(lg) {
        let Some(e) = e else { return false };
        let s = rd(e.packet, 3);
        if rd(e.packet, 5) != op || s <= ps || s >= seq {
            return false;
        }
        ps = s;
        cnt += 1;
    }
    d == seq - s0 - cnt
}

/// R3 climb words: ck3..ck5 levels dropped for canonical call 0..2, ck6 reserved 0, ck7 entries dropped.
fn canon_ck_ok(ck: &[i64]) -> bool {
    let w = |i| rd(ck, i).clamp(0, CK_MAX);
    let (a, b, c, d) = (w(3), w(4), w(5), w(7));
    if rd(ck, 6) != 0 || a > 1 << 25 || b > 1 << 25 || c > 1 << 25 {
        return false;
    }
    let s = a + b + c;
    s <= d && d <= 2 * s
}

fn ck_zero_from(ck: &[i64], from: usize) -> bool {
    (from..8).all(|i| rd(ck, i) == 0)
}

/// R1(d): the checkpoint words of an await for op with outstanding request code are consistent.
pub fn ck_ok(op: i64, code: i64, ck: &[i64]) -> bool {
    if ck.len() != 8 {
        return false;
    }
    if op == 6 {
        return ck[..3] == [0, 0, 0] && canon_ck_ok(ck);
    }
    if op != 1 {
        return ck_zero_from(ck, 0);
    }
    let ph = ck[0].clamp(0, CK_MAX) % 16;
    let (idx, cnt) = (ck[1], ck[2]);
    let codes_ok = match ph {
        0 | 6 | 7 | 8 => code == 4,
        1 | 4 => code == 1,
        2 => code == 2,
        3 => code == 3 || code == 4,
        5 => (2..=7).contains(&code),
        _ => return false,
    };
    if !codes_ok {
        return false;
    }
    let loop_ok = if ph == 2 || ph == 5 { idx < cnt && cnt <= MAX_WORDS as i64 } else { idx == 0 && cnt == 0 };
    loop_ok && if ph == 5 { canon_ck_ok(ck) } else { ck_zero_from(ck, 3) }
}

/// The seq of the item's first request: 1 for every op but 1; op 1 packs it above the phase (ck0 = phase + 16 * s0).
pub fn s0_of(op: i64, ck: &[i64]) -> i64 {
    if op != 1 { 1 } else { rd(ck, 0).clamp(0, CK_MAX) / 16 }
}

/// ck with word 0 reduced to the phase.
pub fn ck_phase_only(ck: &[i64]) -> [i64; 8] {
    let mut k = [0; 8];
    k.iter_mut().zip(ck).for_each(|(d, &s)| *d = s);
    k[0] = rd(ck, 0).clamp(0, CK_MAX) % 16;
    k
}

// ---- replay context ----

/// A request to the host: what the await packet carries in its pool, record and (optional) table sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub code: i64,
    pub aux: i64,
    pub pool: Vec<i64>,
    pub rec: Vec<i64>,
    pub tab: Option<Vec<i64>>,
}

/// The request recorded in prev (R1(a)); `exact`: the pool must be equal, else only start with the regenerated pool.
pub struct Prev {
    pub exact: bool,
    pub ns: usize,
    pub pool: Vec<i64>,
    pub rec: Vec<i64>,
    pub tab: Vec<i64>,
}

/// The finished item: pool, record and an optional table section.
pub struct Output {
    pub pool: Vec<i64>,
    pub rec: Vec<i64>,
    pub tab: Option<Vec<i64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// InvalidPacket (a protocol error)
    Protocol,
    /// InputTooLarge (the output cannot fit the packet limit)
    TooBig,
}

/// Why a step gave no answer: the context stalled on a new request, the replay is broken, or the item is already
/// finished (a nested step set the output, e.g. a thrown answer rethrown as the result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    Stalled,
    Bad,
    Done,
}

pub struct Ctx {
    pub op: i64,
    pub op_id: i64,
    pub pos: usize,
    pub blob: Vec<i64>,
    pub lg: Vec<i64>,
    pub ck: [i64; 8],
    /// the answer of the last observation, a completion packet inside `lg`
    pub ent: Range<usize>,
    pub fault: Option<Fault>,
    pub req: Option<Request>,
    pub prev: Option<Prev>,
    pub out: Option<Output>,
    pub ncan: i64,
}

impl Ctx {
    pub fn new(op: i64, op_id: i64, blob: Vec<i64>, lg: Vec<i64>, ck: &[i64]) -> Ctx {
        let mut k = [0; 8];
        k.iter_mut().zip(ck).for_each(|(d, &s)| *d = s);
        Ctx { op, op_id, pos: 0, blob, lg, ck: k, ent: 0..0, fault: None, req: None, prev: None, out: None, ncan: 0 }
    }

    pub fn ent(&self) -> &[i64] {
        &self.lg[self.ent.clone()]
    }

    /// Observation: the next log entry if present (it must answer exactly this request), else stall on it.
    pub fn obs(&mut self, req: Request) -> Result<(), Halt> {
        if self.req.is_some() {
            return Err(Halt::Stalled);
        }
        if self.fault.is_some() {
            return Err(Halt::Bad);
        }
        if self.pos >= self.lg.len() {
            self.req = Some(req);
            return Err(Halt::Stalled);
        }
        let pos = self.pos;
        let ok = match entry_at(&self.lg, pos) {
            // R1(a): the last entry answers the request recorded in prev; the regenerated request must equal it
            Some(e) => {
                e.code == req.code && e.aux == req.aux && (pos + e.len != self.lg.len() || self.prev.as_ref().is_none_or(|p| p.same(&req)))
            }
            None => false,
        };
        if !ok {
            return self.fail();
        }
        let n = rd(&self.lg, pos) as usize;
        self.ent = pos + 3..pos + n;
        self.pos = pos + n;
        Ok(())
    }

    /// The next item of the same step: an empty log and no answer, prev request or output (blob and ck stay).
    pub fn next_item(&mut self) {
        self.pos = 0;
        self.lg = Vec::new();
        self.ent = 0..0;
        self.prev = None;
        self.out = None;
        self.ncan = 0;
    }

    /// Remember the request recorded in prev (an await packet).
    pub fn with_prev(&mut self, prev: &[i64], exact: bool) {
        let ns = nsec(prev);
        let sec = |i: usize| -> Vec<i64> {
            let s = sec_start(prev, i).min(prev.len());
            prev[s..(s + sec_len(prev, i)).min(prev.len())].to_vec()
        };
        let tab = if ns >= 5 { sec(4) } else { Vec::new() };
        self.prev = Some(Prev { exact, ns, pool: blob_pool(prev).to_vec(), rec: sec(1), tab });
    }

    /// R3: remove the log entries [a, pos), put the cursor at a, count the level in ck[slot] and the entries in ck7.
    pub fn drop_level(&mut self, a: usize, slot: usize, ne: i64) {
        let a = a.min(self.pos);
        self.lg.drain(a..self.pos);
        self.pos = a;
        self.ck[slot] = self.ck[slot].clamp(0, CK_MAX) + 1;
        self.ck[7] = self.ck[7].clamp(0, CK_MAX) + ne.clamp(0, 2);
    }

    pub fn accounts(&mut self, eng: i64) -> Result<(), Halt> {
        self.obs(Request { code: 1, aux: 0, pool: Vec::new(), rec: vec![1, eng], tab: None })
    }
    pub fn account_at(&mut self, eng: i64, idx: i64) -> Result<(), Halt> {
        self.obs(Request { code: 2, aux: 0, pool: Vec::new(), rec: vec![2, eng, idx], tab: None })
    }
    fn nullary(&mut self, code: i64) -> Result<(), Halt> {
        self.obs(Request { code, aux: 0, pool: Vec::new(), rec: vec![code], tab: None })
    }
    pub fn ambient(&mut self) -> Result<(), Halt> {
        self.nullary(3)
    }
    pub fn homedir(&mut self) -> Result<(), Halt> {
        self.nullary(4)
    }
    pub fn cwd(&mut self) -> Result<(), Halt> {
        self.nullary(5)
    }
    fn path(&mut self, code: i64, path: &[u16]) -> Result<(), Halt> {
        let pool = ent_value::encode_pool(path);
        let mut rec = vec![code, 0, 0, 0];
        put_vs(&mut rec, 1, 6, 0, pool.nbytes as i64);
        self.obs(Request { code, aux: 0, pool: pool.view_words(), rec, tab: None })
    }
    pub fn realpath(&mut self, path: &[u16]) -> Result<(), Halt> {
        self.path(6, path)
    }
    pub fn stat(&mut self, path: &[u16]) -> Result<(), Halt> {
        self.path(7, path)
    }

    /// Answer addressing: record word k, physical pool word k, word k of section s.
    pub fn ent_r(&self, k: usize) -> i64 {
        let e = self.ent();
        rd(e, sec_start(e, 1) + k)
    }
    pub fn ent_p(&self, k: usize) -> i64 {
        let e = self.ent();
        rd(e, sec_start(e, 0) + k)
    }
    pub fn ent_s(&self, s: usize, k: usize) -> i64 {
        let e = self.ent();
        rd(e, sec_start(e, s) + k)
    }

    pub fn set_done(&mut self, out: Output) {
        self.out = Some(out);
    }
    pub fn too_big(&mut self) -> Result<(), Halt> {
        self.fault = Some(Fault::TooBig);
        Err(Halt::Bad)
    }
    pub fn fail(&mut self) -> Result<(), Halt> {
        self.fault = Some(Fault::Protocol);
        Err(Halt::Bad)
    }

    /// The await packet for the stalled request; the blob and log count towards the same output limit.
    pub fn replay_await(&self, req: &Request, seq: i64, pool: &[i64], rows: &[i64]) -> Vec<i64> {
        let blob = ent_value_pkt_size(&self.blob);
        let log = log_words(&self.lg);
        let nu = clampu(rd(pool, 0), MAX_WORDS as i64);
        let tab = req.tab.as_ref().map_or(0, |t| 1 + t.len());
        let parts = [blob, log, req.rec.len(), rows.len(), tab];
        if parts.iter().any(|&p| p > MAX_WORDS) || 25 + nu + parts.iter().sum::<usize>() > MAX_WORDS {
            return boundary(self.op, self.op_id, 1);
        }
        let st = encode_state(self.op, req.code, req.aux, &self.ck, &self.blob, &self.lg);
        let (t, nt) = match &req.tab {
            Some(t) => (t.as_slice(), 1),
            None => (&[][..], 0),
        };
        mk_packet(1, self.op_id, seq, self.op, pool, &req.rec, &st, rows, t, nt)
    }

    /// The packet to return to the host: await (aseq), done (dseq) or a boundary.
    pub fn finish(&self, aseq: i64, dseq: i64) -> Vec<i64> {
        match self.fault {
            Some(Fault::TooBig) => return boundary(self.op, self.op_id, 1),
            Some(Fault::Protocol) => return boundary(self.op, self.op_id, 0),
            None => {}
        }
        // R1(c): the replay must have consumed the whole log
        if self.pos != self.lg.len() {
            return boundary(self.op, self.op_id, 0);
        }
        if let Some(req) = &self.req {
            return self.replay_await(req, aseq, &req.pool, &[]);
        }
        match &self.out {
            Some(o) => {
                let (t, nt) = match &o.tab {
                    Some(t) => (t.as_slice(), 1),
                    None => (&[][..], 0),
                };
                mk_packet(3, self.op_id, dseq, self.op, &o.pool, &o.rec, &[], &[], t, nt)
            }
            None => boundary(self.op, self.op_id, 2),
        }
    }
}

impl Prev {
    /// The regenerated request equals the one recorded in prev.
    fn same(&self, req: &Request) -> bool {
        if self.rec != req.rec {
            return false;
        }
        let (lr, lq) = (Pool(&req.pool).nbytes(), Pool(&self.pool).nbytes());
        if lr > lq || (self.exact && lr != lq) || (lr != 0 && !pool_starts_with(&self.pool, &req.pool)) {
            return false;
        }
        match &req.tab {
            Some(t) => self.ns == 5 && self.tab == *t,
            None => self.ns == 4,
        }
    }
}

/// pruv `ent_value::pkt_size`: words with the pool counted by its units.
fn ent_value_pkt_size(w: &[i64]) -> usize {
    let l0 = sec_len(w, 0);
    if l0 < 2 || l0 > w.len() {
        return w.len();
    }
    let rest = w.len() - l0;
    if rest > MAX_WORDS {
        return rest;
    }
    rest + clampu(rd(w, sec_start(w, 0)), MAX_WORDS as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Vec<i64> {
        // op 3 input: sections pool (empty) and record [undefined, ref 1, undefined, ref 1]
        vec![1, 0, 7, 0, 18, 3, 2, 2, 12, 0, 0, 0, 0, 0, 9, 1, 0, 0, 0, 0, 9, 1, 0]
    }

    #[test]
    fn stall_then_replay() {
        let blob = input();
        let req = || Request { code: 9, aux: 0, pool: blob_pool(&input()).to_vec(), rec: vec![9, 1, 0], tab: None };
        let mut cx = Ctx::new(3, 7, blob.clone(), Vec::new(), &[0; 8]);
        assert_eq!(cx.obs(req()), Err(Halt::Stalled));
        assert_eq!(cx.obs(req()), Err(Halt::Stalled));
        let aw = cx.finish(1, 0);
        assert_eq!(aw[1], 1);
        // the answer: a kind 2 completion with an empty list: sections pool, record, rows
        let comp = vec![1, 2, 7, 1, 11, 3, 3, 2, 4, 0, 0, 0, 0, 7, 2, 0];
        let lg = log_append(&[], 9, 0, &comp, 0);
        assert!(log_ok(&lg, 7));
        let st = encode_state(3, 9, 0, &[0; 8], &blob, &lg);
        let s = State::parse(&st, 3, 7).expect("state");
        assert!(s.replay_ok(3, 2));
        let mut cy = Ctx::new(3, 7, s.blob.to_vec(), s.log.to_vec(), s.ck);
        cy.with_prev(&aw, true);
        assert_eq!(cy.obs(req()), Ok(()));
        assert_eq!(cy.ent_r(0), 0);
        cy.set_done(Output { pool: Vec::new(), rec: vec![0], tab: None });
        assert_eq!(cy.finish(2, 1)[1], 3);
        // a different regenerated request is a protocol error
        let mut cz = Ctx::new(3, 7, s.blob.to_vec(), s.log.to_vec(), s.ck);
        cz.with_prev(&aw, true);
        let mut other = req();
        other.rec[1] = 2;
        assert_eq!(cz.obs(other), Err(Halt::Bad));
        assert_eq!(cz.finish(2, 1)[14], 0);
    }

    #[test]
    fn checkpoint_words() {
        assert!(ck_ok(3, 9, &[0; 8]));
        assert!(!ck_ok(3, 9, &[0, 0, 0, 0, 0, 0, 0, 1]));
        assert!(ck_ok(6, 9, &[0, 0, 0, 1, 0, 0, 0, 2]));
        assert!(!ck_ok(6, 9, &[0, 0, 0, 1, 0, 0, 0, 3]));
        assert!(ck_ok(1, 2, &[2 + 16, 0, 3, 0, 0, 0, 0, 0]));
        assert!(!ck_ok(1, 2, &[2, 3, 3, 0, 0, 0, 0, 0]));
        assert_eq!(s0_of(1, &[5 + 32, 0, 0, 0, 0, 0, 0, 0]), 2);
        assert_eq!(ck_phase_only(&[5 + 32, 1, 2, 0, 0, 0, 0, 0])[0], 5);
        let mut cx = Ctx::new(6, 1, Vec::new(), vec![12, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0; 8]);
        cx.pos = 24;
        cx.drop_level(0, 3, 2);
        assert_eq!((cx.pos, cx.lg.len(), cx.ck[3], cx.ck[7]), (0, 0, 1, 2));
    }
}

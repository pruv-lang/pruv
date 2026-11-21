// L3-8 engine-import common entry host (DESIGN.md sections 2, 3, 5, 8). Included by host/src/lib.rs as crate `entry_host`.
// Only plain `//` comments here (include! forbids inner doc comments and inner attributes).
//
// The host makes no import decisions. It encodes a case into the input packet, drives entry_begin/entry_step, serves
// every request from the case script (queue semantics), keeps opaque reference tables, accumulates emitted source rows,
// reconstructs the result, and prints one JSON result line per case.
//
// Stdin: one case JSON per line (DESIGN 5.1). Stdout: one result JSON line per case:
//   {"id":..,"terminal":..,"value":..,"trace":[..],"driver":null|"InvalidPacket"[,"note":".."]}
// or {"id":..,"error":"case: .."} for a case the host cannot encode or serve. The host never panics on bad input.
//
// Packet layout decisions made here (DESIGN 2/3 leave them implicit):
//   * words written as a bare name/number in DESIGN 3 (request code, engine word, status, count, rowKind, thrown,
//     route, opKind, recipe action, done first word, enum words) are single raw words; every {set} slot is a 3-word VS.
//   * a table section is added in order of first reference; a VS of tag 7/8 names its section index.
//   * an input/completion packet carries no state section: sections are pool, record, tables... (section index of
//     table n is 2 + n).

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use serde_json::Value;

const MAX_WORDS: usize = 16_777_216;
const MAX_SAFE: i64 = 9_007_199_254_740_991;
const MSG_INVALID_ID: &str = "계정 번호 모양이 틀려.";
const MSG_HOST_EXEC: &str = "기본 Codex 계정 폴더에서는 추가 계정으로 실행하지 않아.";
const MSG_PLAIN: &str = "Codex 계정 폴더를 확인하지 못해 쓰지 않았어.";
const KEY_INVALID_ID: &str = "wireEngine.account.invalidId";
const KEY_HOST_EXEC: &str = "wireEngine.account.folder.codexHostExecution";

// The generated module as seen by the host; harness.rs implements it with the `sol` functions.
pub trait Sol {
    // The core may keep the input (op 9 keeps it for the step instead of copying it into the await).
    fn begin(&self, input: Vec<i64>) -> Rc<Vec<i64>>;
    fn step(&self, prev: &[i64], completion: &[i64]) -> Vec<i64>;
}

pub fn run<S: Sol>(sol: &S) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let out = run_one(sol, &line);
        let mut o = stdout.lock();
        let _ = writeln!(o, "{}", out);
        let _ = o.flush();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// errors

#[derive(Debug)]
enum HostErr {
    Case(String),
    Core(String),
}
type R<T> = Result<T, HostErr>;

fn ce<T>(s: impl Into<String>) -> R<T> {
    Err(HostErr::Case(s.into()))
}
fn ie<T>(s: impl Into<String>) -> R<T> {
    Err(HostErr::Core(s.into()))
}

// ---------------------------------------------------------------------------------------------------------------
// values

#[derive(Clone, Debug, PartialEq)]
enum Jv {
    Undef,
    Null,
    Bool(bool),
    Num(u64),
    Str(Vec<u16>),
    Ref(String),
    Absent,
}

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn is_special_key(k: &str) -> bool {
    matches!(k, "u16" | "undef" | "num" | "ref" | "absent" | "arr" | "obj")
}

fn is_jv_special(m: &serde_json::Map<String, Value>) -> bool {
    m.len() == 1 && m.keys().next().map(|k| is_special_key(k)).unwrap_or(false)
}

fn parse_jv(v: &Value) -> R<Jv> {
    match v {
        Value::String(s) => Ok(Jv::Str(u16s(s))),
        Value::Null => Ok(Jv::Null),
        Value::Bool(b) => Ok(Jv::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i.unsigned_abs() <= MAX_SAFE as u64 {
                    return Ok(Jv::Num((i as f64).to_bits()));
                }
                return ce(format!("integer {} is not a safe integer", i));
            }
            ce(format!("number {} is not a safe JSON integer (use {{\"num\":..}})", n))
        }
        Value::Object(m) => {
            if m.len() != 1 {
                return ce("object is not a JV value");
            }
            let (k, x) = m.iter().next().unwrap();
            match k.as_str() {
                "u16" => {
                    let a = match x.as_array() {
                        Some(a) => a,
                        None => return ce("u16 needs an array"),
                    };
                    let mut out = Vec::with_capacity(a.len());
                    for e in a {
                        match e.as_u64() {
                            Some(n) if n <= 65535 => out.push(n as u16),
                            _ => return ce("u16 unit out of range"),
                        }
                    }
                    Ok(Jv::Str(out))
                }
                "undef" => Ok(Jv::Undef),
                "absent" => Ok(Jv::Absent),
                "num" => {
                    let s = match x.as_str() {
                        Some(s) if s.len() == 16 => s,
                        _ => return ce("num needs 16 hex digits"),
                    };
                    match u64::from_str_radix(s, 16) {
                        Ok(b) => Ok(Jv::Num(b)),
                        Err(_) => ce("num needs 16 hex digits"),
                    }
                }
                "ref" => match x.as_str() {
                    Some(s) => Ok(Jv::Ref(s.to_string())),
                    None => ce("ref needs a name"),
                },
                "arr" | "obj" => ce("composite value is not allowed in this position"),
                _ => ce("object is not a JV value"),
            }
        }
        Value::Array(_) => ce("array is not a JV value (use {\"arr\":..} where an array is allowed)"),
    }
}

fn jv_opt(v: Option<&Value>) -> R<Jv> {
    match v {
        None => Ok(Jv::Absent),
        Some(x) => parse_jv(x),
    }
}

fn jv_str(v: &Value, what: &str) -> R<Vec<u16>> {
    match parse_jv(v)? {
        Jv::Str(s) => Ok(s),
        _ => ce(format!("{} must be a string", what)),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// reference table

#[derive(Clone, Copy, Debug, PartialEq)]
enum Class {
    Error,
    TypeError,
    Eie,
    Plain,
    Primitive,
}

#[derive(Clone)]
struct ErrSpec {
    class: Class,
    name_ref: String,
    code: Option<Jv>,
    name: Option<Jv>,
    message: Option<Jv>,
    errcode: Option<Jv>,
    value: Option<Jv>,
    rename: Option<Jv>,
}

enum RefVal {
    Opaque,
    #[allow(dead_code)]
    Row(Value),
    Record,
    #[allow(dead_code)]
    Thrown(ErrSpec),
}

struct Refs {
    ids: HashMap<String, i64>,
    names: Vec<String>,
    vals: Vec<RefVal>,
}

impl Refs {
    fn new() -> Refs {
        Refs { ids: HashMap::new(), names: Vec::new(), vals: Vec::new() }
    }
    // Ids are assigned by first occurrence, starting at 1.
    fn id(&mut self, name: &str) -> i64 {
        if let Some(i) = self.ids.get(name) {
            return *i;
        }
        self.names.push(name.to_string());
        self.vals.push(RefVal::Opaque);
        let i = self.names.len() as i64;
        self.ids.insert(name.to_string(), i);
        i
    }
    fn set(&mut self, id: i64, v: RefVal) {
        self.vals[(id - 1) as usize] = v;
    }
    fn name(&self, id: i64) -> Option<&str> {
        if id >= 1 && (id as usize) <= self.names.len() {
            Some(&self.names[(id - 1) as usize])
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// packet encoding

struct Enc {
    // The string pool as WTF-8 bytes (DB8 wire format) plus the unit count; slots stay in UTF-16 unit offsets.
    bytes: Vec<u8>,
    nu: usize,
    rec: Vec<i64>,
    tables: Vec<Vec<i64>>,
}

// Pool mark for truncation: a later low surrogate may rewrite the last 3 bytes (join into a 4-byte form).
struct PoolMark {
    len: usize,
    nu: usize,
    tail: [u8; 3],
}

impl Enc {
    fn new() -> Enc {
        Enc { bytes: Vec::new(), nu: 0, rec: Vec::new(), tables: Vec::new() }
    }
    fn pool_mark(&self) -> PoolMark {
        let len = self.bytes.len();
        let mut tail = [0u8; 3];
        if len >= 3 {
            tail.copy_from_slice(&self.bytes[len - 3..]);
        }
        PoolMark { len, nu: self.nu, tail }
    }
    fn pool_restore(&mut self, m: PoolMark) {
        if m.len >= 3 {
            self.bytes.truncate(m.len - 3);
            self.bytes.extend_from_slice(&m.tail);
        } else {
            self.bytes.truncate(m.len);
        }
        self.nu = m.nu;
    }
    // Appends UTF-16 units as WTF-8, joining a pair split across pushes into one 4-byte scalar.
    fn push_units(&mut self, u: &[u16]) {
        self.bytes.reserve(u.len());
        let mut i = 0;
        while i < u.len() {
            let x = u[i] as u32;
            if x < 0x80 {
                self.bytes.push(x as u8);
            } else if x < 0x800 {
                self.bytes.push(0xC0 | (x >> 6) as u8);
                self.bytes.push(0x80 | (x & 0x3F) as u8);
            } else if (0xD800..0xDC00).contains(&x) && i + 1 < u.len() && (0xDC00..0xE000).contains(&(u[i + 1] as u32)) {
                let c = 0x10000 + ((x - 0xD800) << 10) + (u[i + 1] as u32 - 0xDC00);
                self.push4(c);
                i += 1;
            } else if (0xDC00..0xE000).contains(&x) && self.ends_lone_high() {
                let n = self.bytes.len();
                let hi = 0xD000 | (((self.bytes[n - 2] & 0x3F) as u32) << 6) | (self.bytes[n - 1] & 0x3F) as u32;
                self.bytes.truncate(n - 3);
                self.push4(0x10000 + ((hi - 0xD800) << 10) + (x - 0xDC00));
            } else {
                self.bytes.push(0xE0 | (x >> 12) as u8);
                self.bytes.push(0x80 | ((x >> 6) & 0x3F) as u8);
                self.bytes.push(0x80 | (x & 0x3F) as u8);
            }
            i += 1;
        }
        self.nu += u.len();
    }
    fn push4(&mut self, c: u32) {
        self.bytes.push(0xF0 | (c >> 18) as u8);
        self.bytes.push(0x80 | ((c >> 12) & 0x3F) as u8);
        self.bytes.push(0x80 | ((c >> 6) & 0x3F) as u8);
        self.bytes.push(0x80 | (c & 0x3F) as u8);
    }
    fn ends_lone_high(&self) -> bool {
        let n = self.bytes.len();
        n >= 3 && self.bytes[n - 3] == 0xED && (0xA0..=0xAF).contains(&self.bytes[n - 2])
    }
    // Old-format pool: one i64 per UTF-16 unit (REQ keeps this form).
    fn units(&self) -> Vec<i64> {
        wtf8_units(&self.bytes).expect("encoder produced invalid WTF-8").into_iter().map(i64::from).collect()
    }
    fn str_vs(&mut self, u: &[u16]) -> [i64; 3] {
        let off = self.nu as i64;
        self.push_units(u);
        [6, off, u.len() as i64]
    }
    fn text_vs(&mut self, s: &str) -> [i64; 3] {
        let off = self.nu;
        self.bytes.extend_from_slice(s.as_bytes());
        self.nu += s.encode_utf16().count();
        [6, off as i64, (self.nu - off) as i64]
    }
    fn units_vs(&mut self, v: &Value) -> R<[i64; 3]> {
        let units = v.as_array().ok_or_else(|| HostErr::Case("u16 needs an array".into()))?;
        let mut u: Vec<u16> = Vec::with_capacity(units.len());
        for unit in units {
            match unit.as_u64() {
                Some(n) if n <= 65535 => u.push(n as u16),
                _ => return ce("u16 unit out of range"),
            }
        }
        Ok(self.str_vs(&u))
    }
    fn value_vs(&mut self, refs: &mut Refs, v: Option<&Value>) -> R<[i64; 3]> {
        match v {
            None => Ok([10, 0, 0]),
            Some(Value::String(s)) => Ok(self.text_vs(s)),
            Some(Value::Object(m)) if m.len() == 1 && m.contains_key("u16") => self.units_vs(&m["u16"]),
            Some(Value::Object(m)) if m.len() == 1 && m.contains_key("ref") => {
                let name = m["ref"].as_str().ok_or_else(|| HostErr::Case("ref needs a name".into()))?;
                Ok([9, refs.id(name), 0])
            }
            Some(v) => Ok(self.jv_vs(refs, &parse_jv(v)?)),
        }
    }
    fn key_vs(&mut self, v: &Value) -> R<[i64; 3]> {
        match v {
            Value::String(s) => Ok(self.text_vs(s)),
            Value::Object(m) if m.len() == 1 && m.contains_key("u16") => self.units_vs(&m["u16"]),
            _ => Ok(self.str_vs(&jv_str(v, "engineThreads key")?)),
        }
    }
    fn jv_vs(&mut self, refs: &mut Refs, j: &Jv) -> [i64; 3] {
        match j {
            Jv::Undef => [0, 0, 0],
            Jv::Null => [1, 0, 0],
            Jv::Bool(false) => [2, 0, 0],
            Jv::Bool(true) => [3, 0, 0],
            Jv::Num(b) => [5, (*b & 0xffff_ffff) as i64, (*b >> 32) as i64],
            Jv::Str(s) => self.str_vs(s),
            Jv::Ref(n) => [9, refs.id(n), 0],
            Jv::Absent => [10, 0, 0],
        }
    }
    // Reserves a table section and returns its section index (2 + position).
    fn reserve(&mut self) -> usize {
        self.tables.push(Vec::new());
        self.tables.len() + 1
    }
    fn fill(&mut self, sec: usize, rows: Vec<i64>) {
        self.tables[sec - 2] = rows;
    }
}

fn put(out: &mut Vec<i64>, vs: [i64; 3]) {
    out.extend_from_slice(&vs);
}

// Section 0 on the wire: [nu, W_1..W_m], 7 bytes per word big-endian, zero padded.
fn pool_words(bytes: &[u8], nu: usize) -> usize {
    1 + (bytes.len() + 6) / 7
}

fn push_pool_words(w: &mut Vec<i64>, bytes: &[u8], nu: usize) {
    w.push(nu as i64);
    for ch in bytes.chunks(7) {
        let mut x: u64 = 0;
        for j in 0..7 {
            x = (x << 8) | *ch.get(j).unwrap_or(&0) as u64;
        }
        w.push(x as i64);
    }
}

fn packet_header(kind: i64, op_id: i64, seq: i64, op: i64, lens0: usize, e_rec: usize, tables: &[Vec<i64>]) -> Vec<i64> {
    let mut lens: Vec<usize> = vec![lens0, e_rec];
    for t in tables {
        lens.push(t.len());
    }
    let nsec = lens.len();
    let sum: usize = lens.iter().sum();
    let len = 2 + nsec + sum;
    let mut w: Vec<i64> = Vec::with_capacity(5 + len);
    w.extend_from_slice(&[1, kind, op_id, seq, len as i64, op, nsec as i64]);
    for l in &lens {
        w.push(*l as i64);
    }
    w
}

fn mk_packet(kind: i64, op_id: i64, seq: i64, op: i64, e: &Enc) -> Vec<i64> {
    let mut w = packet_header(kind, op_id, seq, op, pool_words(&e.bytes, e.nu), e.rec.len(), &e.tables);
    push_pool_words(&mut w, &e.bytes, e.nu);
    w.extend_from_slice(&e.rec);
    for t in &e.tables {
        w.extend_from_slice(t);
    }
    w
}

// Old-format packet (section 0 = one word per unit); REQ's entry packets keep this form.
fn mk_packet_units(kind: i64, op_id: i64, seq: i64, op: i64, e: &Enc) -> Vec<i64> {
    let pool = e.units();
    let mut w = packet_header(kind, op_id, seq, op, pool.len(), e.rec.len(), &e.tables);
    w.extend_from_slice(&pool);
    w.extend_from_slice(&e.rec);
    for t in &e.tables {
        w.extend_from_slice(t);
    }
    w
}

// D-DB4-2: the byte pool is dropped as soon as it is packed, then rec and each table are dropped as they are copied.
fn mk_packet_owned(kind: i64, op_id: i64, seq: i64, op: i64, enc: Enc) -> Vec<i64> {
    let Enc { bytes, nu, rec, tables } = enc;
    let mut w = packet_header(kind, op_id, seq, op, pool_words(&bytes, nu), rec.len(), &tables);
    push_pool_words(&mut w, &bytes, nu);
    drop(bytes);
    w.extend_from_slice(&rec);
    drop(rec);
    for table in tables {
        w.extend_from_slice(&table);
    }
    w
}

// Strict WTF-8 -> UTF-16 units. None on bad UTF-8, overlong forms, > U+10FFFF, or a split pair written as two 3-byte forms.
fn wtf8_units(b: &[u8]) -> Option<Vec<u16>> {
    let mut out: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    let cont = |k: usize| -> Option<u32> {
        let c = *b.get(k)?;
        if c & 0xC0 == 0x80 { Some((c & 0x3F) as u32) } else { None }
    };
    while i < b.len() {
        let c = b[i] as u32;
        if c < 0x80 {
            out.push(c as u16);
            i += 1;
        } else if (0xC2..0xE0).contains(&c) {
            out.push((((c & 0x1F) << 6) | cont(i + 1)?) as u16);
            i += 2;
        } else if (0xE0..0xF0).contains(&c) {
            let x = ((c & 0x0F) << 12) | (cont(i + 1)? << 6) | cont(i + 2)?;
            if x < 0x800 {
                return None;
            }
            if (0xDC00..0xE000).contains(&x) && matches!(out.last(), Some(&h) if (0xD800..0xDC00).contains(&h)) {
                return None;
            }
            out.push(x as u16);
            i += 3;
        } else if (0xF0..0xF5).contains(&c) {
            let x = ((c & 0x07) << 18) | (cont(i + 1)? << 12) | (cont(i + 2)? << 6) | cont(i + 3)?;
            if !(0x10000..=0x10FFFF).contains(&x) {
                return None;
            }
            let y = x - 0x10000;
            out.push(0xD800 + (y >> 10) as u16);
            out.push(0xDC00 + (y & 0x3FF) as u16);
            i += 4;
        } else {
            return None;
        }
    }
    Some(out)
}

// Decodes wire section 0 back to units (words of Pruv output; any violation is an internal error).
fn decode_pool(sec: &[i64]) -> R<Vec<u16>> {
    if sec.is_empty() {
        return ie("pool section is empty");
    }
    let nu = sec[0];
    let words = &sec[1..];
    if nu < 0 {
        return ie("pool unit count is negative");
    }
    let mut bytes: Vec<u8> = Vec::with_capacity(words.len() * 7);
    for &x in words {
        if !(0..(1i64 << 56)).contains(&x) {
            return ie("pool word out of range");
        }
        for j in 0..7 {
            bytes.push(((x >> (8 * (6 - j))) & 0xff) as u8);
        }
    }
    // padding: the true byte length is the one that decodes to nu units with only zero bytes after it
    let units = match wtf8_prefix(&bytes, nu as usize) {
        Some((u, used)) => {
            if bytes[used..].iter().any(|&c| c != 0) || bytes.len() - used >= 7 {
                return ie("pool padding is not zero or has the wrong word count");
            }
            u
        }
        None => return ie("pool is not valid WTF-8 of the stated unit count"),
    };
    Ok(units)
}

// Decodes exactly nu units from the start of b; returns (units, bytes used).
fn wtf8_prefix(b: &[u8], nu: usize) -> Option<(Vec<u16>, usize)> {
    // find the end: scan scalars until nu units are produced
    let mut units = 0usize;
    let mut i = 0usize;
    while units < nu {
        let c = *b.get(i)?;
        let (n, u) = if c < 0x80 { (1, 1) } else if c < 0xE0 { (2, 1) } else if c < 0xF0 { (3, 1) } else { (4, 2) };
        i += n;
        units += u;
        if i > b.len() {
            return None;
        }
    }
    if units != nu {
        return None;
    }
    let u = wtf8_units(&b[..i])?;
    if u.len() != nu {
        return None;
    }
    Some((u, i))
}

// ---------------------------------------------------------------------------------------------------------------
// packet decoding

struct Pkt<'a> {
    kind: i64,
    op_id: i64,
    seq: i64,
    op: i64,
    secs: Vec<&'a [i64]>,
    // Section 0 as UTF-16 units: decoded from the wire bytes into u16, or borrowed words for old-format packets.
    pool: Pool<'a>,
}

enum Pool<'a> {
    Words(&'a [i64]),
    Units(Vec<u16>),
}

fn parse_pkt(w: &[i64]) -> R<Pkt<'_>> {
    let mut p = parse_pkt_units(w)?;
    p.pool = Pool::Units(decode_pool(p.secs[0])?);
    Ok(p)
}

// Old format: section 0 is one word per unit (REQ entry packets).
fn parse_pkt_units(w: &[i64]) -> R<Pkt<'_>> {
    if w.len() < 7 {
        return ie(format!("core packet too short ({} words)", w.len()));
    }
    if w[0] != 1 {
        return ie("core packet version is not 1");
    }
    if !(0..=3).contains(&w[1]) {
        return ie("core packet kind out of range");
    }
    if w[4] < 2 || (w.len() - 5) as i64 != w[4] {
        return ie("core packet length field does not match the array length");
    }
    let payload = &w[5..];
    let nsec = payload[1];
    if nsec < 2 || nsec > payload.len() as i64 {
        return ie("core packet section count is invalid");
    }
    let nsec = nsec as usize;
    if payload.len() < 2 + nsec {
        return ie("core packet header is truncated");
    }
    let mut rest = payload.len() - 2 - nsec;
    let mut secs = Vec::with_capacity(nsec);
    let mut at = 2 + nsec;
    for i in 0..nsec {
        let l = payload[2 + i];
        if l < 0 || (l as u64) > rest as u64 {
            return ie("core packet section length is invalid");
        }
        rest -= l as usize;
        let end = at + l as usize;
        secs.push(&payload[at..end]);
        at = end;
    }
    if rest != 0 {
        return ie("core packet section lengths do not cover the payload");
    }
    let pool = Pool::Words(secs[0]);
    Ok(Pkt { kind: w[1], op_id: w[2], seq: w[3], op: payload[0], secs, pool })
}

#[derive(Clone, Debug, PartialEq)]
enum V {
    Undef,
    Null,
    Bool(bool),
    Proto(i64),
    Num(u64),
    Str(Vec<u16>),
    Table(usize, usize),
    Ref(i64),
    Absent,
    Insp,
}

impl Pkt<'_> {
    fn word(&self, sec: usize, off: usize) -> R<i64> {
        match self.secs.get(sec).and_then(|s| s.get(off)) {
            Some(x) => Ok(*x),
            None => ie(format!("core packet word ({},{}) is out of range", sec, off)),
        }
    }
    fn val(&self, sec: usize, off: usize) -> R<V> {
        let tag = self.word(sec, off)?;
        let a = self.word(sec, off + 1)?;
        let b = self.word(sec, off + 2)?;
        match tag {
            0 => Ok(V::Undef),
            1 => Ok(V::Null),
            2 => Ok(V::Bool(false)),
            3 => Ok(V::Bool(true)),
            4 => Ok(V::Proto(a)),
            5 => {
                if !(0..=0xffff_ffff).contains(&a) || !(0..=0xffff_ffff).contains(&b) {
                    return ie("number slot words out of range");
                }
                Ok(V::Num(((b as u64) << 32) | a as u64))
            }
            6 => {
                let units = self.string_units(a, b)?;
                let mut out = Vec::with_capacity(units.len());
                for x in units.iter() {
                    out.push(*x);
                }
                Ok(V::Str(out))
            }
            7 | 8 => {
                if a < 2 || (a as usize) >= self.secs.len() || b < 0 {
                    return ie("table slot names an invalid section");
                }
                Ok(V::Table(a as usize, b as usize))
            }
            9 => {
                if a < 1 {
                    return ie("reference id below 1");
                }
                Ok(V::Ref(a))
            }
            10 => Ok(V::Absent),
            11 => Ok(V::Insp),
            _ => ie(format!("unknown value tag {}", tag)),
        }
    }
    fn string_units(&self, start: i64, len: i64) -> R<std::borrow::Cow<'_, [u16]>> {
        let n = match &self.pool {
            Pool::Words(w) => w.len(),
            Pool::Units(u) => u.len(),
        };
        if start < 0 || len < 0 || (start as u64) + (len as u64) > n as u64 {
            return ie("string slot outside the pool");
        }
        let r = start as usize..(start + len) as usize;
        match &self.pool {
            Pool::Units(u) => Ok(std::borrow::Cow::Borrowed(&u[r])),
            Pool::Words(w) => {
                let mut out = Vec::with_capacity(r.len());
                for x in &w[r] {
                    if !(0..=65535).contains(x) {
                        return ie("pool word is not a UTF-16 unit");
                    }
                    out.push(*x as u16);
                }
                Ok(std::borrow::Cow::Owned(out))
            }
        }
    }

    fn render_at(&self, refs: &Refs, sec: usize, off: usize, out: &mut String) -> R<()> {
        if self.word(sec, off)? == 6 {
            let units = self.string_units(self.word(sec, off + 1)?, self.word(sec, off + 2)?)?;
            if units.iter().all(|u| (32..=126).contains(u) && *u != 34 && *u != 92) {
                out.reserve(units.len() + 2);
                out.push('"');
                out.extend(units.iter().map(|&u| char::from(u as u8)));
                out.push('"');
                return Ok(());
            }
        }
        out.push_str(&render_v(refs, &self.val(sec, off)?)?);
        Ok(())
    }
    // Rows of a table slot: validates section length = count * width.
    fn table(&self, v: &V, width: usize) -> R<(usize, usize)> {
        match v {
            V::Table(s, n) => {
                if self.secs[*s].len() as u64 != (*n as u64) * (width as u64) {
                    return ie("table section length does not match count * width");
                }
                Ok((*s, *n))
            }
            _ => ie("expected a table slot"),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// rendering

fn jstr(u: &[u16]) -> String {
    match String::from_utf16(u) {
        Ok(s) => serde_json::to_string(&s).unwrap_or_else(|_| "\"\"".to_string()),
        Err(_) => {
            let parts: Vec<String> = u.iter().map(|x| x.to_string()).collect();
            format!("{{\"u16\":[{}]}}", parts.join(","))
        }
    }
}

fn jstr_s(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn render_num(bits: u64) -> String {
    let f = f64::from_bits(bits);
    let neg_zero = bits == 0x8000_0000_0000_0000;
    if f.is_finite() && f.fract() == 0.0 && f.abs() <= MAX_SAFE as f64 && !neg_zero {
        format!("{}", f as i64)
    } else {
        format!("{{\"num\":\"{:016x}\"}}", bits)
    }
}

fn render_ref(refs: &Refs, id: i64) -> R<String> {
    match refs.name(id) {
        Some(n) => Ok(format!("{{\"ref\":{}}}", jstr_s(n))),
        None => ie(format!("core returned unknown reference id {}", id)),
    }
}

fn render_v(refs: &Refs, v: &V) -> R<String> {
    match v {
        V::Undef | V::Absent => Ok("{\"undef\":1}".to_string()),
        V::Null => Ok("null".to_string()),
        V::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        V::Proto(i) => Ok(i.to_string()),
        V::Num(b) => Ok(render_num(*b)),
        V::Str(s) => Ok(jstr(s)),
        V::Ref(id) => render_ref(refs, *id),
        V::Table(_, _) | V::Insp => ie("cannot render a table or inspected slot as a value"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// JS own-property order (DESIGN 5.3)

fn index_key(k: &[u16]) -> Option<u64> {
    if k.is_empty() || k.len() > 10 {
        return None;
    }
    if k.len() > 1 && k[0] == '0' as u16 {
        return None;
    }
    let mut n: u64 = 0;
    for &c in k {
        if !(('0' as u16)..=('9' as u16)).contains(&c) {
            return None;
        }
        n = n * 10 + (c - '0' as u16) as u64;
    }
    if n < 4_294_967_295 {
        Some(n)
    } else {
        None
    }
}

// Assignment semantics: a repeated key keeps its first position and takes the last value; integer-index keys come
// first in ascending numeric order, then the other keys in first-assignment order.
fn order_entries<T>(entries: Vec<(Vec<u16>, T)>) -> Vec<(Vec<u16>, T)> {
    let mut pos: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut list: Vec<(Vec<u16>, T)> = Vec::new();
    for (k, v) in entries {
        if let Some(&i) = pos.get(&k) {
            list[i].1 = v;
        } else {
            pos.insert(k.clone(), list.len());
            list.push((k, v));
        }
    }
    let mut idx: Vec<(u64, (Vec<u16>, T))> = Vec::new();
    let mut rest: Vec<(Vec<u16>, T)> = Vec::new();
    for e in list {
        match index_key(&e.0) {
            Some(n) => idx.push((n, e)),
            None => rest.push(e),
        }
    }
    idx.sort_by_key(|x| x.0);
    let mut out: Vec<(Vec<u16>, T)> = idx.into_iter().map(|x| x.1).collect();
    out.extend(rest);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// script

#[derive(Clone)]
enum FsRes<T> {
    Ok(T),
    Throw(ErrSpec),
    Errno(String, Option<String>, Option<String>), // code, ref name, message override
}

struct Queue<T> {
    items: Vec<T>,
    pos: usize,
}

impl<T: Clone> Queue<T> {
    fn new(items: Vec<T>) -> Queue<T> {
        Queue { items, pos: 0 }
    }
    // Consumed in call order; the last entry repeats. None when the queue is empty.
    fn next(&mut self) -> Option<T> {
        if self.items.is_empty() {
            return None;
        }
        let i = self.pos.min(self.items.len() - 1);
        self.pos += 1;
        Some(self.items[i].clone())
    }
}

enum AccRes {
    Ok(Vec<AccRow>),
    Throw(ErrSpec),
}

#[derive(Clone)]
enum AccRow {
    Nullish,
    Prim,
    Obj(Jv, Jv),
}

enum Eng {
    SyncThrow(ErrSpec),
    Reject(ErrSpec),
    Rows(Vec<(String, Value, Jv)>),
    Record(String),
}

struct Script {
    homedir: Queue<FsRes<Vec<u16>>>,
    cwd: Queue<FsRes<Vec<u16>>>,
    ambient: Queue<Jv>,
    realpath: HashMap<Vec<u16>, Queue<FsRes<Vec<u16>>>>,
    stat: HashMap<Vec<u16>, Queue<FsRes<(Vec<u16>, Vec<u16>)>>>,
    accounts: [Option<AccRes>; 2],
    is_temp: Queue<FsRes<Jv>>,
    engine: Option<Eng>,
}

fn items_of(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        x => vec![x],
    }
}

fn str_of(v: &Value, what: &str) -> R<String> {
    match v.as_str() {
        Some(s) => Ok(s.to_string()),
        None => ce(format!("{} must be a string", what)),
    }
}

fn parse_err(v: &Value) -> R<ErrSpec> {
    let m = match v.as_object() {
        Some(m) => m,
        None => return ce("ERR must be an object"),
    };
    let class = match m.get("class").and_then(|c| c.as_str()).unwrap_or("Error") {
        "Error" => Class::Error,
        "TypeError" => Class::TypeError,
        "EngineImportError" => Class::Eie,
        "plain" => Class::Plain,
        "primitive" => Class::Primitive,
        other => return ce(format!("unknown ERR class {}", other)),
    };
    let name_ref = match m.get("ref") {
        Some(r) => match r.as_str() {
            Some(s) => s.to_string(),
            None => return ce("ERR ref must be a string"),
        },
        None => return ce("ERR needs a ref"),
    };
    let opt = |k: &str| -> R<Option<Jv>> {
        match m.get(k) {
            None => Ok(None),
            Some(x) => Ok(Some(parse_jv(x)?)),
        }
    };
    Ok(ErrSpec {
        class,
        name_ref,
        code: opt("code")?,
        name: opt("name")?,
        message: opt("message")?,
        errcode: opt("errcode")?,
        value: opt("value")?,
        rename: opt("rename")?,
    })
}

fn parse_fs_res<T>(v: &Value, ok: &dyn Fn(&Value) -> R<T>) -> R<FsRes<T>> {
    let m = match v.as_object() {
        Some(m) => m,
        None => return ce("script entry must be an object"),
    };
    if let Some(x) = m.get("ok") {
        return Ok(FsRes::Ok(ok(x)?));
    }
    if let Some(x) = m.get("throw") {
        return Ok(FsRes::Throw(parse_err(x)?));
    }
    if let Some(x) = m.get("errno") {
        let code = str_of(x, "errno")?;
        let r = match m.get("ref") {
            Some(r) => Some(str_of(r, "errno ref")?),
            None => None,
        };
        let msg = match m.get("message") {
            Some(r) => Some(str_of(r, "errno message")?),
            None => None,
        };
        return Ok(FsRes::Errno(code, r, msg));
    }
    ce("script entry needs ok, throw or errno")
}

fn parse_str_res(v: &Value) -> R<FsRes<Vec<u16>>> {
    parse_fs_res(v, &|x| jv_str(x, "scripted path"))
}

/// A scripted stat dev/ino: a decimal digit string or a non-negative integer. The text is canonicalized (leading zeros
/// stripped, "0" kept) the way the oracle's BigInt renders it, so "01" and "1" are the same identity (DESIGN 5.1).
/// Signs, spaces, radix prefixes, empty text and non-integers are refused (the oracle refuses them too).
fn dec_text(v: &Value, what: &str) -> R<Vec<u16>> {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return ce(format!("{} must be a decimal string", what)),
    };
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return ce(format!("{} must be a decimal digit string", what));
    }
    let trimmed = text.trim_start_matches('0');
    Ok(u16s(if trimmed.is_empty() { "0" } else { trimmed }))
}

fn parse_acc_row(v: &Value) -> R<AccRow> {
    match v {
        Value::Null => Ok(AccRow::Nullish),
        Value::Object(m) => {
            if m.len() == 1 && (m.contains_key("undef") || m.contains_key("absent")) {
                return Ok(AccRow::Nullish);
            }
            Ok(AccRow::Obj(jv_opt(m.get("id"))?, jv_opt(m.get("name"))?))
        }
        _ => Ok(AccRow::Prim),
    }
}

fn parse_engine(v: &Value) -> R<Eng> {
    let m = match v.as_object() {
        Some(m) => m,
        None => return ce("engine script must be an object"),
    };
    if let Some(x) = m.get("sync_throw") {
        return Ok(Eng::SyncThrow(parse_err(x)?));
    }
    if let Some(x) = m.get("reject") {
        return Ok(Eng::Reject(parse_err(x)?));
    }
    if let Some(x) = m.get("rows") {
        let a = match x.as_array() {
            Some(a) => a,
            None => return ce("engine rows must be an array"),
        };
        let mut rows = Vec::new();
        for r in a {
            let ro = match r.as_object() {
                Some(o) => o,
                None => return ce("engine row must be an object"),
            };
            let name = match ro.get("ref") {
                Some(n) => str_of(n, "row ref")?,
                None => return ce("engine row needs a ref"),
            };
            rows.push((name, r.clone(), jv_opt(ro.get("title"))?));
        }
        return Ok(Eng::Rows(rows));
    }
    if let Some(x) = m.get("record") {
        let name = match x {
            Value::String(s) => s.clone(),
            Value::Object(o) => match o.get("ref").and_then(|r| r.as_str()) {
                Some(s) => s.to_string(),
                None => return ce("record needs a ref name"),
            },
            _ => return ce("record needs a ref name"),
        };
        return Ok(Eng::Record(name));
    }
    ce("engine script needs sync_throw, reject, rows or record")
}

fn parse_script(script: Option<&Value>, forward: Option<&Value>) -> R<Script> {
    let empty = serde_json::Map::new();
    let m = match script {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        _ => return ce("script must be an object"),
    };
    let queue = |k: &str, def: &str| -> R<Queue<FsRes<Vec<u16>>>> {
        let mut items = Vec::new();
        if let Some(x) = m.get(k) {
            for e in items_of(x) {
                items.push(parse_str_res(e)?);
            }
        }
        if items.is_empty() {
            items.push(FsRes::Ok(u16s(def)));
        }
        Ok(Queue::new(items))
    };
    let homedir = queue("homedir", "/home/test")?;
    let cwd = queue("cwd", "/work")?;
    let mut amb = Vec::new();
    if let Some(x) = m.get("ambient") {
        for e in items_of(x) {
            let j = match e {
                Value::Object(o) if o.contains_key("ok") => parse_jv(&o["ok"])?,
                other => parse_jv(other)?,
            };
            match j {
                Jv::Undef | Jv::Str(_) => amb.push(j),
                _ => return ce("ambient must be a string or undefined"),
            }
        }
    }
    if amb.is_empty() {
        amb.push(Jv::Undef);
    }
    let mut realpath = HashMap::new();
    if let Some(x) = m.get("realpath") {
        let o = match x.as_object() {
            Some(o) => o,
            None => return ce("realpath script must be an object keyed by path"),
        };
        for (k, v) in o {
            let mut items = Vec::new();
            for e in items_of(v) {
                items.push(parse_str_res(e)?);
            }
            realpath.insert(u16s(k), Queue::new(items));
        }
    }
    let mut stat = HashMap::new();
    if let Some(x) = m.get("stat") {
        let o = match x.as_object() {
            Some(o) => o,
            None => return ce("stat script must be an object keyed by path"),
        };
        for (k, v) in o {
            let mut items = Vec::new();
            for e in items_of(v) {
                items.push(parse_fs_res(e, &|x| {
                    let o = match x.as_object() {
                        Some(o) => o,
                        None => return ce("stat ok needs {dev, ino}"),
                    };
                    let dev = dec_text(o.get("dev").unwrap_or(&Value::Null), "dev")?;
                    let ino = dec_text(o.get("ino").unwrap_or(&Value::Null), "ino")?;
                    Ok((dev, ino))
                })?);
            }
            stat.insert(u16s(k), Queue::new(items));
        }
    }
    let mut accounts: [Option<AccRes>; 2] = [None, None];
    if let Some(x) = m.get("accounts") {
        let o = match x.as_object() {
            Some(o) => o,
            None => return ce("accounts script must be an object"),
        };
        for (i, k) in ["claude", "codex"].iter().enumerate() {
            if let Some(e) = o.get(*k) {
                let eo = match e.as_object() {
                    Some(o) => o,
                    None => return ce("accounts entry must be an object"),
                };
                if let Some(t) = eo.get("throw") {
                    accounts[i] = Some(AccRes::Throw(parse_err(t)?));
                } else if let Some(rows) = eo.get("ok") {
                    let a = match rows.as_array() {
                        Some(a) => a,
                        None => return ce("accounts ok must be an array"),
                    };
                    let mut out = Vec::new();
                    for r in a {
                        out.push(parse_acc_row(r)?);
                    }
                    accounts[i] = Some(AccRes::Ok(out));
                } else {
                    return ce("accounts entry needs ok or throw");
                }
            }
        }
    }
    let mut temps = Vec::new();
    if let Some(x) = m.get("isTemporary") {
        for e in items_of(x) {
            temps.push(parse_fs_res(e, &|x| parse_jv(x))?);
        }
    }
    if temps.is_empty() {
        temps.push(FsRes::Ok(Jv::Bool(false)));
    }
    // Forward mode (DESIGN 8) is served exactly like a scripted engine result.
    let engine = match (forward, m.get("engine")) {
        (Some(f), _) if !f.is_null() => Some(parse_engine(f)?),
        (_, Some(e)) => Some(parse_engine(e)?),
        _ => None,
    };
    Ok(Script {
        homedir,
        cwd,
        ambient: Queue::new(amb),
        realpath,
        stat,
        accounts,
        is_temp: Queue::new(temps),
        engine,
    })
}

struct Probe {
    at: usize,
    corrupt: String,
}

fn parse_probe(v: Option<&Value>) -> R<Option<Probe>> {
    let v = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let at = match v.get("at").and_then(|a| a.as_u64()) {
        Some(a) => a as usize,
        None => return ce("probe needs an integer at"),
    };
    let c = match v.get("corrupt").and_then(|c| c.as_str()) {
        Some(c) => c.to_string(),
        None => return ce("probe needs corrupt"),
    };
    match c.as_str() {
        "version" | "length" | "tag" | "unit" | "op_id" | "seq" | "duplicate" | "reentry" | "oversize" | "exact_limit" => {}
        _ => return ce(format!("unknown probe corruption {}", c)),
    }
    Ok(Some(Probe { at, corrupt: c }))
}

// ---------------------------------------------------------------------------------------------------------------
// input encoding (DESIGN 3.1)

fn bound_thread_id(entries: &[Value]) -> R<Jv> {
    let mut tid = Jv::Absent;
    for e in entries {
        let pair = e.as_array().filter(|p| p.len() == 2)
            .ok_or_else(|| HostErr::Case("obj entry must be [key, value]".into()))?;
        let k = jv_str(&pair[0], "obj key")?;
        let val = parse_jv(&pair[1])?;
        if k.iter().copied().eq("threadId".encode_utf16()) && val != Jv::Absent {
            tid = val;
        }
    }
    Ok(tid)
}

fn enc_bound(enc: &mut Enc, refs: &mut Refs, b: &Value, rows: &mut Vec<i64>) -> R<()> {
    match b {
        Value::Object(o) if !is_jv_special(o) => {
            rows.extend_from_slice(&[11, 0, 0]);
            put(rows, enc.value_vs(refs, o.get("threadId"))?);
        }
        Value::Array(_) => rows.extend_from_slice(&[11, 0, 0, 10, 0, 0]),
        Value::Object(o) if o.len() == 1 && o.contains_key("arr") => {
            rows.extend_from_slice(&[11, 0, 0, 10, 0, 0]);
        }
        Value::Object(o) if o.len() == 1 && o.get("obj").and_then(Value::as_array).is_some() => {
            let tid = bound_thread_id(o["obj"].as_array().unwrap())?;
            rows.extend_from_slice(&[11, 0, 0]);
            put(rows, enc.jv_vs(refs, &tid));
        }
        _ => {
            put(rows, enc.value_vs(refs, Some(b))?);
            rows.extend_from_slice(&[0, 0, 0]);
        }
    }
    Ok(())
}

enum TL<'a> {
    Absent,
    Undef,
    Null,
    Obj(&'a serde_json::Map<String, Value>),
}

fn thread_like(v: Option<&Value>) -> R<TL<'_>> {
    match v {
        None => Ok(TL::Absent),
        Some(Value::Null) => Ok(TL::Null),
        Some(Value::Object(m)) => {
            if m.len() == 1 && m.contains_key("absent") {
                Ok(TL::Absent)
            } else if m.len() == 1 && m.contains_key("undef") {
                Ok(TL::Undef)
            } else if is_jv_special(m) {
                ce("object slot must be null, undefined, absent or an object")
            } else {
                Ok(TL::Obj(m))
            }
        }
        _ => ce("object slot must be null, undefined, absent or an object"),
    }
}

fn enc_forks(enc: &mut Enc, refs: &mut Refs, v: Option<&Value>) -> R<[i64; 3]> {
    let list: Option<&Vec<Value>> = match v {
        None => return Ok([10, 0, 0]),
        Some(Value::Null) => return Ok([1, 0, 0]),
        Some(Value::Array(a)) => Some(a),
        Some(Value::Object(m)) => {
            if m.len() == 1 && m.contains_key("absent") {
                return Ok([10, 0, 0]);
            }
            if m.len() == 1 && m.contains_key("undef") {
                return Ok([0, 0, 0]);
            }
            match m.get("arr") {
                Some(Value::Array(a)) if m.len() == 1 => Some(a),
                _ => return ce("forks must be an array, null, undefined or absent"),
            }
        }
        _ => return ce("forks must be an array, null, undefined or absent"),
    };
    let a = list.unwrap();
    let sec = enc.reserve();
    let mut rows = Vec::with_capacity(3 * a.len());
    for e in a {
        put(&mut rows, enc.value_vs(refs, Some(e))?);
    }
    enc.fill(sec, rows);
    Ok([7, sec as i64, a.len() as i64])
}

fn enc_session(enc: &mut Enc, refs: &mut Refs, v: &Value, out: &mut Vec<i64>) -> R<()> {
    let m = match v.as_object() {
        Some(m) => m,
        None => return ce("session must be an object"),
    };
    for k in ["id", "temporary", "engine"] {
        put(out, enc.value_vs(refs, m.get(k))?);
    }
    // externalThread: slots 3..6
    match thread_like(m.get("externalThread"))? {
        TL::Absent => out.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        TL::Undef => out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        TL::Null => out.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        TL::Obj(o) => {
            out.extend_from_slice(&[11, 0, 0]);
            for k in ["engine", "thread"] {
                put(out, enc.value_vs(refs, o.get(k))?);
            }
            let f = enc_forks(enc, refs, o.get("forks"))?;
            put(out, f);
        }
    }
    // terminal: slots 7..8
    match thread_like(m.get("terminal"))? {
        TL::Absent => out.extend_from_slice(&[10, 0, 0, 0, 0, 0]),
        TL::Undef => out.extend_from_slice(&[0, 0, 0, 0, 0, 0]),
        TL::Null => out.extend_from_slice(&[1, 0, 0, 0, 0, 0]),
        TL::Obj(o) => {
            out.extend_from_slice(&[11, 0, 0]);
            put(out, enc.value_vs(refs, o.get("cliSession"))?);
        }
    }
    // engineThreads: slot 9
    let et = m.get("engineThreads");
    match et {
        None => out.extend_from_slice(&[10, 0, 0]),
        Some(Value::Null) => out.extend_from_slice(&[1, 0, 0]),
        Some(Value::Object(o)) if o.len() == 1 && o.contains_key("absent") => out.extend_from_slice(&[10, 0, 0]),
        Some(Value::Object(o)) if o.len() == 1 && o.contains_key("undef") => out.extend_from_slice(&[0, 0, 0]),
        Some(Value::Object(o)) if o.len() == 1 && o.contains_key("obj") => {
            let a = match o["obj"].as_array() {
                Some(a) => a,
                None => return ce("engineThreads obj needs an array"),
            };
            let sec = enc.reserve();
            let mut rows = Vec::with_capacity(9 * a.len());
            if let [entry] = a.as_slice() {
                let pair = entry.as_array().filter(|p| p.len() == 2)
                    .ok_or_else(|| HostErr::Case("engineThreads entry must be [key, bound]".into()))?;
                let pool_mark = enc.pool_mark();
                let kv = enc.key_vs(&pair[0])?;
                if matches!(&pair[1], Value::Object(b) if b.len() == 1 && b.contains_key("absent")) {
                    enc.pool_restore(pool_mark);
                } else {
                    put(&mut rows, kv);
                    enc_bound(enc, refs, &pair[1], &mut rows)?;
                }
            } else {
                let mut entries = Vec::with_capacity(a.len());
                for e in a {
                    let pair = e.as_array().filter(|p| p.len() == 2)
                        .ok_or_else(|| HostErr::Case("engineThreads entry must be [key, bound]".into()))?;
                    let k = jv_str(&pair[0], "engineThreads key")?;
                    if matches!(&pair[1], Value::Object(b) if b.len() == 1 && b.contains_key("absent")) {
                        continue;
                    }
                    entries.push((k, &pair[1]));
                }
                for (k, b) in order_entries(entries) {
                    put(&mut rows, enc.str_vs(&k));
                    enc_bound(enc, refs, b, &mut rows)?;
                }
            }
            let count = rows.len() / 9;
            enc.fill(sec, rows);
            out.extend_from_slice(&[8, sec as i64, count as i64]);
        }
        Some(_) => return ce("engineThreads must be {\"obj\":..}, null, undefined or absent"),
    }
    Ok(())
}

const ENV_NAMES: [&str; 7] = [
    "CLAUDE_CONFIG_DIR",
    "GROK_HOME",
    "XDG_DATA_HOME",
    "QWEN_RUNTIME_DIR",
    "QWEN_HOME",
    "CODEX_HOME",
    "HOME",
];

fn encode_input(refs: &mut Refs, op: i64, args: &Value) -> R<Enc> {
    let empty = serde_json::Map::new();
    let a = match args {
        Value::Object(m) => m,
        Value::Null => &empty,
        _ => return ce("args must be an object"),
    };
    let mut enc = Enc::new();
    let mut rec: Vec<i64> = Vec::new();
    macro_rules! slot {
        ($j:expr) => {{
            let j = $j;
            let vs = enc.jv_vs(refs, &j);
            put(&mut rec, vs);
        }};
    }
    match op {
        1 => {
            slot!(jv_opt(a.get("dataDir"))?);
            let env = match a.get("env") {
                None | Some(Value::Null) => None,
                Some(Value::Object(m)) => Some(m),
                _ => return ce("env must be an object"),
            };
            for n in ENV_NAMES {
                let j = match env.and_then(|e| e.get(n)) {
                    None => Jv::Absent,
                    Some(x) => parse_jv(x)?,
                };
                slot!(j);
            }
        }
        2 => {
            slot!(jv_opt(a.get("query"))?);
            let list = match a.get("sources") {
                Some(Value::Array(l)) => l,
                _ => return ce("sources must be an array"),
            };
            let sec = enc.reserve();
            let mut rows = Vec::new();
            for e in list {
                let j = match e {
                    Value::Object(m) if m.len() == 1 && m.contains_key("absent") => Jv::Absent,
                    Value::Object(m) => jv_opt(m.get("source"))?,
                    _ => return ce("source entry must be an object"),
                };
                let vs = enc.jv_vs(refs, &j);
                put(&mut rows, vs);
            }
            enc.fill(sec, rows);
            put(&mut rec, [7, sec as i64, list.len() as i64]);
        }
        3 | 4 => {
            let src = match a.get("source").and_then(|s| s.as_object()) {
                Some(s) => s,
                None => return ce("source must be an object with a ref"),
            };
            let name = match src.get("ref").and_then(|r| r.as_str()) {
                Some(n) => n.to_string(),
                None => return ce("source needs a ref"),
            };
            slot!(jv_opt(src.get("engine"))?);
            slot!(Jv::Ref(name));
            slot!(jv_opt(a.get("folder"))?);
            if op == 4 {
                slot!(jv_opt(a.get("thread"))?);
            }
            let opt = match a.get("options") {
                Some(Value::Object(o)) => match o.get("ref").and_then(|r| r.as_str()) {
                    Some(n) => n.to_string(),
                    None => return ce("options needs a ref"),
                },
                _ => return ce("options must be {\"ref\":..}"),
            };
            slot!(Jv::Ref(opt));
        }
        5 => {
            slot!(jv_opt(a.get("dataDir"))?);
            slot!(jv_opt(a.get("engine"))?);
            slot!(jv_opt(a.get("id"))?);
        }
        6 => {
            let acc = match a.get("account").and_then(|x| x.as_object()) {
                Some(x) => x,
                None => return ce("account must be an object"),
            };
            slot!(jv_opt(a.get("dataDir"))?);
            slot!(jv_opt(acc.get("id"))?);
            slot!(jv_opt(acc.get("home"))?);
        }
        7 | 8 => {
            let s = match a.get("session") {
                Some(s) => s,
                None => return ce("session is required"),
            };
            enc_session(&mut enc, refs, s, &mut rec)?;
        }
        _ => return ce("unsupported op"),
    }
    enc.rec = rec;
    Ok(enc)
}

fn encode_index(refs: &mut Refs, args: Value) -> R<Enc> {
    let sessions = match args {
        Value::Object(mut a) => match a.remove("sessions") {
            Some(Value::Array(list)) => list,
            _ => return ce("sessions must be an array"),
        },
        Value::Null => return ce("sessions must be an array"),
        _ => return ce("args must be an object"),
    };
    let count = sessions.len();
    let mut enc = Enc::new();
    enc.bytes.reserve(24 * count);
    let sec = enc.reserve();
    let mut rows = Vec::with_capacity(30 * count);
    for session in sessions {
        enc_session(&mut enc, refs, &session, &mut rows)?;
    }
    enc.fill(sec, rows);
    put(&mut enc.rec, [7, sec as i64, count as i64]);
    Ok(enc)
}

// Same encoding as encode_index, from the session texts one at a time (lean_split).
fn encode_index_raw(refs: &mut Refs, sessions: Vec<&str>) -> R<Enc> {
    let count = sessions.len();
    let mut enc = Enc::new();
    enc.bytes.reserve(24 * count);
    let sec = enc.reserve();
    let mut rows = Vec::with_capacity(30 * count);
    for r in sessions {
        let session: Value = match serde_json::from_str(r) {
            Ok(v) => v,
            Err(e) => return Err(HostErr::Core(format!("session text: {}", e))),
        };
        enc_session(&mut enc, refs, &session, &mut rows)?;
    }
    enc.fill(sec, rows);
    put(&mut enc.rec, [7, sec as i64, count as i64]);
    Ok(enc)
}

// ---------------------------------------------------------------------------------------------------------------
// host state: serving requests (DESIGN 3.2)

struct Host {
    refs: Refs,
    script: Script,
    trace: Vec<String>,
    acct: [Vec<AccRow>; 2],
    rows: Vec<[V; 6]>,
    seen: HashSet<i64>,
    errno_n: usize,
}

fn errno_desc(code: &str) -> &'static str {
    match code {
        "ENOENT" => "no such file or directory",
        "EACCES" => "permission denied",
        "ELOOP" => "too many symbolic links encountered",
        "ENOTDIR" => "not a directory",
        "EPERM" => "operation not permitted",
        "EIO" => "i/o error",
        "ENAMETOOLONG" => "name too long",
        "EBUSY" => "resource busy or locked",
        "EMFILE" => "too many open files",
        "ENOMEM" => "not enough memory",
        "EINVAL" => "invalid argument",
        "EISDIR" => "illegal operation on a directory",
        "ENXIO" => "no such device or address",
        _ => "unknown error",
    }
}

impl Host {
    fn err_spec_for_errno(&mut self, code: &str, name: &Option<String>, msg: &Option<String>, syscall: &str, path: &[u16]) -> ErrSpec {
        let name_ref = match name {
            Some(n) => n.clone(),
            None => {
                self.errno_n += 1;
                format!("$errno{}", self.errno_n)
            }
        };
        let message = match msg {
            Some(m) => u16s(m),
            None => {
                let mut m = u16s(&format!("{}: {}, {} '", code, errno_desc(code), syscall));
                m.extend_from_slice(path);
                m.push('\'' as u16);
                m
            }
        };
        ErrSpec {
            class: Class::Error,
            name_ref,
            code: None,
            name: None,
            message: Some(Jv::Str(message)),
            errcode: Some(Jv::Str(u16s(code))),
            value: None,
            rename: None,
        }
    }

    // The 6-VS error observation E (DESIGN 3.2). Every thrown value gets a ref, primitives included.
    fn err_obs(&mut self, enc: &mut Enc, s: &ErrSpec) -> R<Vec<i64>> {
        let id = self.refs.id(&s.name_ref);
        // A ref names one object: once a thrown value exists for it, later script entries naming the same
        // ref observe that first value (the oracle caches it the same way), not a freshly built one.
        let first = match &self.refs.vals[(id - 1) as usize] {
            RefVal::Thrown(prev) => prev.clone(),
            _ => s.clone(),
        };
        let s = &first;
        self.refs.set(id, RefVal::Thrown(s.clone()));
        self.seen.insert(id);
        let is_error = matches!(s.class, Class::Error | Class::TypeError | Class::Eie);
        let name = if let Some(r) = &s.rename {
            r.clone()
        } else if let Some(n) = &s.name {
            n.clone()
        } else {
            match s.class {
                Class::Error => Jv::Str(u16s("Error")),
                Class::TypeError => Jv::Str(u16s("TypeError")),
                Class::Eie => Jv::Str(u16s("EngineImportError")),
                _ => Jv::Undef,
            }
        };
        let code = if let Some(c) = &s.errcode {
            c.clone()
        } else if s.class == Class::Eie {
            s.code.clone().unwrap_or(Jv::Undef)
        } else {
            Jv::Undef
        };
        let message = if is_error {
            match &s.message {
                Some(Jv::Str(m)) => Jv::Str(m.clone()),
                Some(_) => return ce("error message must be a string"),
                None => match (&s.class, &s.code) {
                    (Class::Eie, Some(Jv::Str(c))) => Jv::Str(c.clone()),
                    _ => Jv::Str(Vec::new()),
                },
            }
        } else {
            Jv::Undef
        };
        let mut out = Vec::new();
        put(&mut out, [9, id, 0]);
        put(&mut out, [if s.class == Class::Eie { 3 } else { 2 }, 0, 0]);
        put(&mut out, [if is_error { 3 } else { 2 }, 0, 0]);
        let vs = enc.jv_vs(&mut self.refs, &name);
        put(&mut out, vs);
        let vs = enc.jv_vs(&mut self.refs, &message);
        put(&mut out, vs);
        let vs = enc.jv_vs(&mut self.refs, &code);
        put(&mut out, vs);
        if s.class == Class::Primitive {
            // a primitive thrown value: name/code of a primitive are undefined; `value` only documents the case
            let _ = &s.value;
        }
        Ok(out)
    }

    fn throw_rec(&mut self, enc: &mut Enc, s: &ErrSpec) -> R<()> {
        let e = self.err_obs(enc, s)?;
        enc.rec.push(1);
        enc.rec.extend(e);
        Ok(())
    }

    fn ok_str(enc: &mut Enc, s: &[u16]) {
        enc.rec.push(0);
        let vs = enc.str_vs(s);
        put(&mut enc.rec, vs);
    }

    fn serve_str(&mut self, enc: &mut Enc, res: FsRes<Vec<u16>>, syscall: &str, path: &[u16]) -> R<()> {
        match res {
            FsRes::Ok(s) => {
                Host::ok_str(enc, &s);
                Ok(())
            }
            FsRes::Throw(e) => self.throw_rec(enc, &e),
            FsRes::Errno(c, n, m) => {
                let e = self.err_spec_for_errno(&c, &n, &m, syscall, path);
                self.throw_rec(enc, &e)
            }
        }
    }

    fn serve(&mut self, p: &Pkt) -> R<Enc> {
        let mut enc = Enc::new();
        // request strings live in the await pool; decode them into the completion-side values as needed
        let code = p.word(1, 0)?;
        match code {
            1 => {
                let eng = p.word(1, 1)?;
                if eng != 0 && eng != 1 {
                    return ie("accounts request with an engine word outside 0..1");
                }
                self.trace.push(format!("[\"accounts\",{}]", if eng == 0 { "\"claude\"" } else { "\"codex\"" }));
                let r = match self.script.accounts[eng as usize].take() {
                    Some(r) => r,
                    None => AccRes::Ok(Vec::new()),
                };
                match &r {
                    AccRes::Ok(rows) => {
                        self.acct[eng as usize] = rows.clone();
                        enc.rec.extend_from_slice(&[0, rows.len() as i64]);
                    }
                    AccRes::Throw(e) => {
                        let e = e.clone();
                        self.throw_rec(&mut enc, &e)?;
                    }
                }
                // keep the script entry for repeated calls
                self.script.accounts[eng as usize] = Some(r);
            }
            2 => {
                let eng = p.word(1, 1)?;
                let idx = p.word(1, 2)?;
                if !(0..=1).contains(&eng) || idx < 0 || (idx as usize) >= self.acct[eng as usize].len() {
                    return ie("account_at request outside the snapshotted array");
                }
                let row = self.acct[eng as usize][idx as usize].clone();
                enc.rec.push(0);
                match row {
                    AccRow::Nullish => {
                        enc.rec.push(0);
                        put(&mut enc.rec, [0, 0, 0]);
                        put(&mut enc.rec, [0, 0, 0]);
                    }
                    AccRow::Prim => {
                        enc.rec.push(1);
                        put(&mut enc.rec, [0, 0, 0]);
                        put(&mut enc.rec, [0, 0, 0]);
                    }
                    AccRow::Obj(id, name) => {
                        enc.rec.push(1);
                        let a = enc.jv_vs(&mut self.refs, &id);
                        put(&mut enc.rec, a);
                        let b = enc.jv_vs(&mut self.refs, &name);
                        put(&mut enc.rec, b);
                    }
                }
            }
            3 => {
                self.trace.push("[\"ambient\",\"CODEX_HOME\"]".to_string());
                let j = self.script.ambient.next().unwrap_or(Jv::Undef);
                enc.rec.push(0);
                let vs = enc.jv_vs(&mut self.refs, &j);
                put(&mut enc.rec, vs);
            }
            4 => {
                self.trace.push("[\"homedir\"]".to_string());
                let r = self.script.homedir.next().unwrap_or(FsRes::Ok(u16s("/home/test")));
                self.serve_str(&mut enc, r, "homedir", &[])?;
            }
            5 => {
                self.trace.push("[\"cwd\"]".to_string());
                let r = self.script.cwd.next().unwrap_or(FsRes::Ok(u16s("/work")));
                self.serve_str(&mut enc, r, "cwd", &[])?;
            }
            6 | 7 => {
                let path = match p.val(1, 1)? {
                    V::Str(s) => s,
                    _ => return ie("realpath/stat request path is not a string"),
                };
                let ptxt = jstr(&path);
                if code == 6 {
                    self.trace.push(format!("[\"realpath\",{}]", ptxt));
                    let r = self.script.realpath.get_mut(&path).and_then(|q| q.next());
                    let r = r.unwrap_or(FsRes::Errno("ENOENT".to_string(), None, None));
                    self.serve_str(&mut enc, r, "realpath", &path)?;
                } else {
                    self.trace.push(format!("[\"stat\",{}]", ptxt));
                    let r = self.script.stat.get_mut(&path).and_then(|q| q.next());
                    let r = r.unwrap_or(FsRes::Errno("ENOENT".to_string(), None, None));
                    match r {
                        FsRes::Ok((d, i)) => {
                            enc.rec.push(0);
                            let a = enc.str_vs(&d);
                            put(&mut enc.rec, a);
                            let b = enc.str_vs(&i);
                            put(&mut enc.rec, b);
                        }
                        FsRes::Throw(e) => self.throw_rec(&mut enc, &e)?,
                        FsRes::Errno(c, n, m) => {
                            let e = self.err_spec_for_errno(&c, &n, &m, "stat", &path);
                            self.throw_rec(&mut enc, &e)?;
                        }
                    }
                }
            }
            8 => {
                let v = p.val(1, 1)?;
                let (sec, count) = p.table(&v, 3)?;
                let mut results = Vec::with_capacity(3 * count);
                let mut thrown: Option<ErrSpec> = None;
                let mut n = 0usize;
                for i in 0..count {
                    let mut trace = String::from("[\"isTemporary\",");
                    p.render_at(&self.refs, sec, i * 3, &mut trace)?;
                    trace.push(']');
                    self.trace.push(trace);
                    let r = self.script.is_temp.next().unwrap_or(FsRes::Ok(Jv::Bool(false)));
                    match r {
                        FsRes::Ok(j) => {
                            let vs = enc.jv_vs(&mut self.refs, &j);
                            put(&mut results, vs);
                            n += 1;
                        }
                        FsRes::Throw(e) => {
                            thrown = Some(e);
                            break;
                        }
                        FsRes::Errno(c, nm, m) => {
                            thrown = Some(self.err_spec_for_errno(&c, &nm, &m, "isTemporary", &[]));
                            break;
                        }
                    }
                }
                let tsec = enc.reserve();
                enc.fill(tsec, results);
                enc.rec.push(0);
                put(&mut enc.rec, [7, tsec as i64, n as i64]);
                match thrown {
                    None => {
                        enc.rec.push(0);
                        for _ in 0..6 {
                            put(&mut enc.rec, [0, 0, 0]);
                        }
                    }
                    Some(e) => {
                        enc.rec.push(1);
                        let o = self.err_obs(&mut enc, &e)?;
                        enc.rec.extend(o);
                    }
                }
            }
            9 => self.serve_engine(p, &mut enc)?,
            _ => return ie(format!("core emitted unknown request code {}", code)),
        }
        Ok(enc)
    }

    fn serve_engine(&mut self, p: &Pkt, enc: &mut Enc) -> R<()> {
        let route = p.word(1, 1)?;
        let opk = p.word(1, 2)?;
        if !(0..=4).contains(&route) || !(0..=1).contains(&opk) {
            return ie("invoke_engine request with route/opKind out of range");
        }
        let source = p.val(1, 3)?;
        let folder = p.val(1, 6)?;
        let thread = p.val(1, 9)?;
        let options = p.val(1, 12)?;
        for (v, what) in [(&source, "source"), (&options, "options")] {
            if !matches!(v, V::Ref(_)) {
                return ie(format!("invoke_engine {} is not a reference", what));
            }
        }
        let route_name = ["claude", "codex", "opencode", "qwen", "grok"][route as usize];
        let mut t = format!(
            "[\"engine\",{},{},{{\"source\":{},\"folder\":{}",
            jstr_s(route_name),
            jstr_s(if opk == 0 { "list" } else { "read" }),
            render_v(&self.refs, &source)?,
            render_v(&self.refs, &folder)?
        );
        if thread != V::Absent {
            t.push_str(&format!(",\"thread\":{}", render_v(&self.refs, &thread)?));
        }
        t.push_str(&format!(",\"options\":{}}}]", render_v(&self.refs, &options)?));
        self.trace.push(t);
        let eng = match self.script.engine.take() {
            Some(e) => e,
            None => return ce("no engine script for an invoke_engine request"),
        };
        let r = match &eng {
            Eng::SyncThrow(e) | Eng::Reject(e) => {
                let e = e.clone();
                self.throw_rec(enc, &e)
            }
            Eng::Rows(rows) => {
                if opk != 0 {
                    Err(HostErr::Case("engine rows scripted for a read".to_string()))
                } else {
                    let sec = enc.reserve();
                    let mut t = Vec::new();
                    let mut bad = None;
                    for (name, row, title) in rows {
                        let id = self.refs.id(name);
                        self.refs.set(id, RefVal::Row(row.clone()));
                        self.seen.insert(id);
                        put(&mut t, [9, id, 0]);
                        match title {
                            Jv::Str(s) => {
                                let vs = enc.str_vs(s);
                                put(&mut t, vs);
                            }
                            _ => bad = Some("engine row title must be a string"),
                        }
                    }
                    match bad {
                        Some(b) => Err(HostErr::Case(b.to_string())),
                        None => {
                            enc.fill(sec, t);
                            enc.rec.push(0);
                            put(&mut enc.rec, [7, sec as i64, rows.len() as i64]);
                            Ok(())
                        }
                    }
                }
            }
            Eng::Record(name) => {
                if opk != 1 {
                    Err(HostErr::Case("engine record scripted for a list".to_string()))
                } else {
                    let id = self.refs.id(name);
                    self.refs.set(id, RefVal::Record);
                    self.seen.insert(id);
                    enc.rec.push(0);
                    put(&mut enc.rec, [9, id, 0]);
                    Ok(())
                }
            }
        };
        self.script.engine = Some(eng);
        r
    }
}

// ---------------------------------------------------------------------------------------------------------------
// terminal decoding (DESIGN 3.3)

struct Outcome {
    terminal: &'static str,
    value: String,
}

fn read_rows(h: &mut Host, p: &Pkt) -> R<()> {
    let s3 = match p.secs.get(3) {
        Some(s) => s,
        None => return ie("packet has no emitted-rows section"),
    };
    if s3.len() % 18 != 0 {
        return ie("emitted rows section is not a multiple of 18 words");
    }
    for i in 0..s3.len() / 18 {
        let o = i * 18;
        let row = [p.val(3, o)?, p.val(3, o + 3)?, p.val(3, o + 6)?, p.val(3, o + 9)?, p.val(3, o + 12)?, p.val(3, o + 15)?];
        h.rows.push(row);
    }
    Ok(())
}

fn render_sources(h: &Host) -> R<String> {
    let mut out: Vec<String> = Vec::new();
    for r in &h.rows {
        for k in [0usize, 1, 2, 5] {
            if !matches!(r[k], V::Str(_)) {
                return ie("emitted row has a non-string source/engine/kind/home");
            }
        }
        let mut s = format!(
            "{{\"source\":{},\"engine\":{},\"kind\":{}",
            render_v(&h.refs, &r[0])?,
            render_v(&h.refs, &r[1])?,
            render_v(&h.refs, &r[2])?
        );
        if r[3] != V::Absent {
            s.push_str(&format!(
                ",\"account\":{},\"accountName\":{}",
                render_v(&h.refs, &r[3])?,
                render_v(&h.refs, &r[4])?
            ));
        }
        s.push_str(&format!(",\"home\":{}}}", render_v(&h.refs, &r[5])?));
        out.push(s);
    }
    Ok(format!("[{}]", out.join(",")))
}

fn pair_table(h: &Host, p: &Pkt, v: &V) -> R<String> {
    let (sec, n) = p.table(v, 6)?;
    let mut out = String::with_capacity(16 * n + 2);
    out.push('[');
    for i in 0..n {
        let a = p.val(sec, i * 6)?;
        let b = p.val(sec, i * 6 + 3)?;
        if i != 0 {
            out.push(',');
        }
        out.push('[');
        out.push_str(&render_v(&h.refs, &a)?);
        out.push(',');
        out.push_str(&render_v(&h.refs, &b)?);
        out.push(']');
    }
    out.push(']');
    Ok(out)
}

fn decode_done(h: &mut Host, p: &Pkt, op: i64) -> R<Outcome> {
    let first = p.word(1, 0)?;
    match first {
        0 => {
            let value = match op {
                1 => render_sources(h)?,
                2 => {
                    let v = p.val(1, 1)?;
                    match v {
                        V::Proto(_) | V::Undef => render_v(&h.refs, &v)?,
                        _ => return ie("find_source result is neither an index nor undefined"),
                    }
                }
                3 => {
                    let v = p.val(1, 1)?;
                    let (sec, n) = p.table(&v, 7)?;
                    let mut out = Vec::new();
                    for i in 0..n {
                        let act = p.word(sec, i * 7)?;
                        let r = p.val(sec, i * 7 + 1)?;
                        let title = p.val(sec, i * 7 + 4)?;
                        let rid = match r {
                            V::Ref(id) if h.seen.contains(&id) => id,
                            _ => return ie("recipe row reference was not delivered by the engine"),
                        };
                        let rr = render_ref(&h.refs, rid)?;
                        match (act, &title) {
                            (0, V::Undef) => out.push(format!("[\"keep\",{}]", rr)),
                            (1, V::Str(s)) => out.push(format!("[\"copy_title\",{},{}]", rr, jstr(s))),
                            _ => return ie("recipe row has an invalid action/title combination"),
                        }
                    }
                    format!("[{}]", out.join(","))
                }
                4 => match p.val(1, 1)? {
                    V::Ref(id) if h.seen.contains(&id) => render_ref(&h.refs, id)?,
                    _ => return ie("read result is not a reference delivered by the engine"),
                },
                5 | 6 => match p.val(1, 1)? {
                    V::Str(s) => jstr(&s),
                    _ => return ie("path result is not a string"),
                },
                7 | 9 => {
                    let v = p.val(1, 1)?;
                    pair_table(h, p, &v)?
                }
                8 => {
                    let v = p.val(1, 1)?;
                    render_v(&h.refs, &v)?
                }
                _ => return ie("done packet for an unknown op"),
            };
            Ok(Outcome { terminal: "ok", value })
        }
        1 => match p.val(1, 1)? {
            V::Ref(id) if h.seen.contains(&id) => {
                let r = render_ref(&h.refs, id)?;
                Ok(Outcome { terminal: "throw_existing", value: r })
            }
            _ => ie("throw_existing reference was not delivered in this operation"),
        },
        2 => {
            let code = match p.val(1, 1)? {
                V::Str(s) => s,
                _ => return ie("throw_import code is not a string"),
            };
            let detail = p.val(1, 4)?;
            let cause = match p.val(1, 7)? {
                V::Ref(id) if h.seen.contains(&id) => id,
                _ => return ie("throw_import cause was not delivered in this operation"),
            };
            let mut message = u16s("read-failed");
            match &detail {
                V::Undef | V::Absent => {}
                V::Str(d) => {
                    if !d.is_empty() {
                        message.extend(u16s(": "));
                        message.extend_from_slice(d);
                    }
                }
                _ => return ie("throw_import detail is neither undefined nor a string"),
            }
            let value = format!(
                "{{\"code\":{},\"message\":{},\"cause\":{},\"hasCause\":true}}",
                jstr(&code),
                jstr(&message),
                render_ref(&h.refs, cause)?
            );
            Ok(Outcome { terminal: "throw_import", value })
        }
        3 => {
            let (k, m) = match p.word(1, 1)? {
                0 => (KEY_INVALID_ID, MSG_INVALID_ID),
                1 => (KEY_HOST_EXEC, MSG_HOST_EXEC),
                _ => return ie("unknown account wire enum"),
            };
            Ok(Outcome { terminal: "throw_account_wire", value: format!("{{\"key\":{},\"message\":{}}}", jstr_s(k), jstr_s(m)) })
        }
        4 => {
            if p.word(1, 1)? != 0 {
                return ie("unknown plain error enum");
            }
            Ok(Outcome { terminal: "throw_plain_error", value: format!("{{\"message\":{}}}", jstr_s(MSG_PLAIN)) })
        }
        5 => {
            let c = p.word(1, 1)?;
            Ok(Outcome { terminal: "boundary_error", value: format!("{{\"code\":{}}}", jstr_s(boundary_name(c)?)) })
        }
        _ => ie("unknown terminal word"),
    }
}

fn boundary_name(c: i64) -> R<&'static str> {
    match c {
        0 => Ok("InvalidPacket"),
        1 => Ok("InputTooLarge"),
        2 => Ok("InternalInvariant"),
        _ => ie("unknown boundary code"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// probes (DESIGN 5.4)

// Header-word offsets of a host-built packet.
fn sec_start(w: &[i64], sec: usize) -> usize {
    let nsec = w[6] as usize;
    let mut at = 7 + nsec;
    for i in 0..sec {
        at += w[7 + i] as usize;
    }
    at
}

fn corrupt(w: &mut Vec<i64>, how: &str) {
    let is_input = w[1] == 0;
    match how {
        "version" => w[0] = 2,
        "length" => w[4] += 1,
        "tag" => {
            let at = sec_start(w, 1);
            if at < w.len() {
                w[at] = 99;
            }
        }
        "unit" => {
            let at = sec_start(w, 0);
            if w[7] > 0 {
                w[at] = -1;
            } else {
                w.insert(at, -1);
                w[7] = 1;
                w[4] += 1;
            }
        }
        "op_id" => w[2] = if is_input { 0 } else { w[2] + 1 },
        "seq" => w[3] = if is_input { 7 } else { w[3] + 1 },
        _ => {}
    }
}

fn pad_to(w: &mut Vec<i64>, total: usize) {
    if w.len() >= total {
        return;
    }
    let extra = total - w.len();
    let at = sec_start(w, 1); // end of the pool
    let tail: Vec<i64> = w.split_off(at);
    w.resize(at + extra, 0);
    w.extend(tail);
    w[7] += extra as i64;
    w[4] += extra as i64;
}

// ---------------------------------------------------------------------------------------------------------------
// driver (DESIGN 5.2)

struct Run {
    outcome: Outcome,
    driver: Option<&'static str>,
    note: Option<String>,
}

fn drive<S: Sol>(sol: &S, h: &mut Host, op: i64, op_id: i64, input: Vec<i64>, probe: &Option<Probe>) -> R<Run> {
    let mut driver: Option<&'static str> = None;
    let mut note: Option<String> = None;
    let needs_input = matches!(probe, Some(p) if p.corrupt == "reentry");
    let mut last_completion: Vec<i64> = if needs_input { input.clone() } else { Vec::new() };
    let mut first = input;
    if let Some(p) = probe {
        if p.at == 0 {
            match p.corrupt.as_str() {
                "oversize" => pad_to(&mut first, MAX_WORDS + 1),
                "exact_limit" => pad_to(&mut first, MAX_WORDS),
                "duplicate" | "reentry" => {}
                c => corrupt(&mut first, c),
            }
        }
    }
    let mut out = sol.begin(first);
    let mut calls = 0usize;
    let mut last_seq: i64 = 0;
    loop {
        let p = parse_pkt(&out)?;
        if p.op_id != op_id && p.kind != 3 {
            return ie("core packet op_id does not echo the input");
        }
        if p.op != op && !(p.kind == 3 && p.op == 0) {
            return ie("core packet op does not echo the input op");
        }
        match p.kind {
            3 => {
                if p.secs.len() < 4 {
                    return ie("done packet lacks the emitted-rows section");
                }
                if p.seq != last_seq && p.seq != 0 {
                    return ie("done packet seq is neither the last await seq nor 0");
                }
                if let Some(pr) = probe {
                    if pr.corrupt == "reentry" {
                        let resp = sol.step(&out, &last_completion);
                        match parse_pkt(&resp) {
                            Ok(rp) if rp.kind == 3 && rp.secs.len() >= 2 && rp.word(1, 0).ok() == Some(5) => {
                                let o = decode_done(h, &rp, rp.op)?;
                                return Ok(Run { outcome: o, driver, note });
                            }
                            _ => {
                                note = Some("reentry: core did not answer a step on a done packet with a boundary result".to_string());
                            }
                        }
                    }
                }
                let first_word = p.word(1, 0)?;
                if first_word == 0 {
                    read_rows(h, &p)?;
                }
                let o = decode_done(h, &p, op)?;
                return Ok(Run { outcome: o, driver, note });
            }
            1 => {
                if p.seq != last_seq + 1 {
                    return ie("await seq is not previous + 1");
                }
                if p.seq > (1i64 << 53) {
                    return ie("await seq exceeds 2^53");
                }
                if p.secs.len() < 4 {
                    return ie("await packet lacks the state/emitted-rows sections");
                }
                read_rows(h, &p)?;
                let served = h.serve(&p);
                let enc = match served {
                    Ok(e) => e,
                    Err(e) => return Err(e),
                };
                let mut comp = mk_packet(2, p.op_id, p.seq, p.op, &enc);
                calls += 1;
                drop(enc);
                if let Some(pr) = probe {
                    if pr.at == calls {
                        match pr.corrupt.as_str() {
                            "oversize" => pad_to(&mut comp, MAX_WORDS + 1),
                            "exact_limit" => pad_to(&mut comp, MAX_WORDS),
                            "duplicate" | "reentry" => {}
                            c => corrupt(&mut comp, c),
                        }
                    }
                }
                let next = sol.step(&out, &comp);
                last_seq = p.seq;
                let mut next = next;
                if let Some(pr) = probe {
                    if pr.corrupt == "duplicate" && pr.at == calls {
                        // the driver refuses to replay a stepped await; the core must reject the stale pairing
                        driver = Some("InvalidPacket");
                        if let Ok(np) = parse_pkt(&next) {
                            if np.kind == 1 {
                                let dup = sol.step(&next, &comp);
                                match parse_pkt(&dup) {
                                    Ok(dp) if dp.kind == 3 && dp.secs.len() >= 2 && dp.word(1, 0).ok() == Some(5) => next = dup,
                                    _ => note = Some("duplicate: core accepted a completion of the previous seq".to_string()),
                                }
                            }
                        }
                    }
                }
                if needs_input {
                    last_completion = std::mem::take(&mut comp);
                }
                out = Rc::new(next);
            }
            _ => return ie("core returned a packet that is neither await nor done"),
        }
    }
}

fn run_case<S: Sol>(sol: &S, mut v: Value, raw_sessions: Option<Vec<&str>>) -> Result<String, String> {
    let obj = match v.as_object_mut() {
        Some(o) => o,
        None => return Err("case: line is not a JSON object".to_string()),
    };
    let opname = match obj.get("op").and_then(|o| o.as_str()) {
        Some(o) => o,
        None => return Err("case: missing op".to_string()),
    };
    let op = match opname {
        "import_sources" => 1,
        "find_source" => 2,
        "list_external" => 3,
        "read_external" => 4,
        "account_home" => 5,
        "codex_account_folder" => 6,
        "session_threads" => 7,
        "excluded_from_bound" => 8,
        "bound_session_index" => 9,
        other => return Err(format!("case: unknown op {}", other)),
    };
    let mk = |e: HostErr| match e {
        HostErr::Case(s) => format!("case: {}", s),
        HostErr::Core(s) => format!("case: internal {}", s),
    };
    let script = parse_script(obj.get("script"), obj.get("forward")).map_err(mk)?;
    let probe = parse_probe(obj.get("probe")).map_err(mk)?;
    let mut refs = Refs::new();
    let enc = if let Some(list) = raw_sessions {
        encode_index_raw(&mut refs, list)
    } else if op == 9 {
        let args = obj.remove("args").unwrap_or(Value::Null);
        encode_index(&mut refs, args)
    } else {
        encode_input(&mut refs, op, obj.get("args").unwrap_or(&Value::Null))
    }.map_err(mk)?;
    drop(v);
    let op_id: i64 = 1;
    let input = mk_packet_owned(0, op_id, 0, op, enc);
    let mut h = Host {
        refs,
        script,
        trace: Vec::new(),
        acct: [Vec::new(), Vec::new()],
        rows: Vec::new(),
        seen: HashSet::new(),
        errno_n: 0,
    };
    let res = drive(sol, &mut h, op, op_id, input, &probe);
    let (terminal, value, driver, note) = match res {
        Ok(r) => (r.outcome.terminal, r.outcome.value, r.driver, r.note),
        Err(HostErr::Case(s)) => return Err(format!("case: {}", s)),
        Err(HostErr::Core(s)) => (
            "boundary_error",
            "{\"code\":\"InternalInvariant\"}".to_string(),
            None,
            Some(s),
        ),
    };
    let mut s = format!(
        "\"terminal\":{},\"value\":{},\"trace\":[{}],\"driver\":{}",
        jstr_s(terminal),
        value,
        h.trace.join(","),
        match driver {
            Some(d) => jstr_s(d),
            None => "null".to_string(),
        }
    );
    if let Some(n) = note {
        s.push_str(&format!(",\"note\":{}", jstr_s(&n)));
    }
    Ok(s)
}

// A bound_session_index line keeps its args as text, so the host never holds the whole session list as a JSON tree
// (at L that tree alone was 18 MB, more than the hand run). One byte scan finds the value spans; serde_json then
// parses every span, so each piece is checked as before. A line this split does not fit takes the plain path, so
// errors read as before.
fn ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

// End of the JSON value starting at b[i] (i is not whitespace). Only finds the boundary; serde checks the span.
fn skip_value(b: &[u8], i: usize) -> Option<usize> {
    fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
        i += 1;
        loop {
            match *b.get(i)? {
                b'\\' => i += 2,
                b'"' => return Some(i + 1),
                _ => i += 1,
            }
        }
    }
    match *b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = i;
            loop {
                match *b.get(i)? {
                    b'"' => {
                        i = string_end(b, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        _ => {
            let mut i = i;
            while i < b.len() && !matches!(b[i], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            Some(i)
        }
    }
}

// The members of the object at b[i..]: (key, value text) in order, and the index after the closing brace.
fn members(t: &str, i: usize) -> Option<(Vec<(String, &str)>, usize)> {
    let b = t.as_bytes();
    if b.get(i) != Some(&b'{') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = ws(b, i + 1);
    if b.get(i) == Some(&b'}') {
        return Some((out, i + 1));
    }
    loop {
        if b.get(i) != Some(&b'"') {
            return None;
        }
        let ke = skip_value(b, i)?;
        let key: String = serde_json::from_str(&t[i..ke]).ok()?;
        i = ws(b, ke);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = ws(b, i + 1);
        let ve = skip_value(b, i)?;
        out.push((key, &t[i..ve]));
        i = ws(b, ve);
        match b.get(i) {
            Some(b',') => i = ws(b, i + 1),
            Some(b'}') => return Some((out, i + 1)),
            _ => return None,
        }
    }
}

// The element texts of the array t (None if t is not an array of this shape).
fn elements(t: &str) -> Option<Vec<&str>> {
    let b = t.as_bytes();
    if b.first() != Some(&b'[') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = ws(b, 1);
    if b.get(i) == Some(&b']') {
        return if ws(b, i + 1) == b.len() { Some(out) } else { None };
    }
    loop {
        let e = skip_value(b, i)?;
        out.push(&t[i..e]);
        i = ws(b, e);
        match b.get(i) {
            Some(b',') => i = ws(b, i + 1),
            Some(b']') => return if ws(b, i + 1) == b.len() { Some(out) } else { None },
            _ => return None,
        }
    }
}

fn lean_split(line: &str) -> Option<(Value, Vec<&str>)> {
    let b = line.as_bytes();
    let (top, end) = members(line, ws(b, 0))?;
    if ws(b, end) != b.len() {
        return None;
    }
    let mut obj = serde_json::Map::new();
    let mut args = None;
    for (k, t) in top {
        if k == "args" {
            args = Some(t);
        } else {
            obj.insert(k, serde_json::from_str(t).ok()?);
        }
    }
    if obj.get("op").and_then(|o| o.as_str()) != Some("bound_session_index") {
        return None;
    }
    let (am, ae) = members(args?, 0)?;
    if ae != args?.len() {
        return None;
    }
    let mut sessions = None;
    for (k, t) in am {
        if k == "sessions" {
            sessions = Some(t);
        } else {
            serde_json::from_str::<Value>(t).ok()?;
        }
    }
    Some((Value::Object(obj), elements(sessions?)?))
}

fn run_one<S: Sol>(sol: &S, line: &str) -> String {
    if let Some((v, sessions)) = lean_split(line) {
        let id = serde_json::to_string(v.get("id").unwrap_or(&Value::Null)).unwrap_or_else(|_| "null".to_string());
        // Any error takes the plain path again, which reports it exactly as before (a bad session text first).
        if let Ok(Ok(body)) = catch_unwind(AssertUnwindSafe(|| run_case(sol, v, Some(sessions)))) {
            return format!("{{\"id\":{},{}}}", id, body);
        }
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return format!("{{\"id\":null,\"error\":{}}}", jstr_s(&format!("case: bad JSON: {}", e))),
    };
    let id = match v.get("id") {
        Some(x) => serde_json::to_string(x).unwrap_or_else(|_| "null".to_string()),
        None => "null".to_string(),
    };
    match catch_unwind(AssertUnwindSafe(|| run_case(sol, v, None))) {
        Ok(Ok(body)) => format!("{{\"id\":{},{}}}", id, body),
        Ok(Err(msg)) => format!("{{\"id\":{},\"error\":{}}}", id, jstr_s(&msg)),
        Err(_) => format!("{{\"id\":{},\"error\":\"case: host panic\"}}", id),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// unit tests (JV codec, key order, packet encode/decode, driver smoke)

#[cfg(test)]
mod tests {
    use super::*;

    fn jv(s: &str) -> Jv {
        match parse_jv(&serde_json::from_str::<Value>(s).unwrap()) {
            Ok(j) => j,
            Err(HostErr::Case(m)) | Err(HostErr::Core(m)) => panic!("{}", m),
        }
    }
    fn jv_err(s: &str) -> bool {
        parse_jv(&serde_json::from_str::<Value>(s).unwrap()).is_err()
    }

    #[test]
    fn jv_codec() {
        assert_eq!(jv("\"a\""), Jv::Str(vec![97]));
        assert_eq!(jv("{\"u16\":[55357,97]}"), Jv::Str(vec![55357, 97]));
        assert_eq!(jv("null"), Jv::Null);
        assert_eq!(jv("{\"undef\":1}"), Jv::Undef);
        assert_eq!(jv("true"), Jv::Bool(true));
        assert_eq!(jv("{\"num\":\"8000000000000000\"}"), Jv::Num(0x8000_0000_0000_0000));
        assert_eq!(jv("-3"), Jv::Num((-3f64).to_bits()));
        assert_eq!(jv("{\"ref\":\"r\"}"), Jv::Ref("r".to_string()));
        assert_eq!(jv("{\"absent\":1}"), Jv::Absent);
        assert!(jv_err("9007199254740992"));
        assert!(jv_err("1.5"));
        assert!(jv_err("{\"arr\":[]}"));
        assert!(jv_err("{\"num\":\"12\"}"));
        assert!(jv_err("{\"u16\":[65536]}"));
    }

    #[test]
    fn rendering() {
        assert_eq!(jstr(&[0xD800]), "{\"u16\":[55296]}");
        assert_eq!(jstr(&u16s("한")), "\"한\"");
        assert_eq!(render_num((5f64).to_bits()), "5");
        assert_eq!(render_num(0x8000_0000_0000_0000), "{\"num\":\"8000000000000000\"}");
        assert_eq!(render_num(0), "0");
        assert_eq!(render_num((1.5f64).to_bits()), "{\"num\":\"3ff8000000000000\"}");
    }

    #[test]
    fn key_order() {
        let e: Vec<(Vec<u16>, i32)> = vec![("b", 1), ("2", 2), ("a", 3), ("1", 4), ("10", 5), ("2", 6), ("01", 7), ("4294967295", 8)]
            .into_iter()
            .map(|(k, v)| (u16s(k), v))
            .collect();
        let o = order_entries(e);
        let keys: Vec<String> = o.iter().map(|x| String::from_utf16(&x.0).unwrap()).collect();
        assert_eq!(keys, vec!["1", "2", "10", "b", "a", "01", "4294967295"]);
        assert_eq!(o[1].1, 6);
    }

    #[test]
    fn packet_roundtrip() {
        let mut refs = Refs::new();
        let args: Value = serde_json::from_str("{\"dataDir\":\"/d\",\"env\":{\"HOME\":{\"u16\":[55357]},\"GROK_HOME\":null}}").unwrap();
        let enc = encode_input(&mut refs, 1, &args).unwrap();
        let w = mk_packet(0, 1, 0, 1, &enc);
        assert_eq!(w[4] as usize, w.len() - 5);
        let p = parse_pkt(&w).unwrap();
        assert_eq!((p.kind, p.op_id, p.seq, p.op, p.secs.len()), (0, 1, 0, 1, 2));
        assert_eq!(p.val(1, 0).unwrap(), V::Str(u16s("/d")));
        assert_eq!(p.val(1, 3).unwrap(), V::Absent); // CLAUDE_CONFIG_DIR
        assert_eq!(p.val(1, 6).unwrap(), V::Null); // GROK_HOME
        assert_eq!(p.val(1, 21).unwrap(), V::Str(vec![55357])); // HOME
    }

    #[test]
    fn session_encoding() {
        let mut refs = Refs::new();
        let args: Value = serde_json::from_str(
            "{\"session\":{\"id\":\"s\",\"temporary\":true,\"engineThreads\":{\"obj\":[[\"b:x\",{\"threadId\":\"t\"}],[\"7\",\"p\"],[\"3\",null]]},\"externalThread\":{\"engine\":\"claude\",\"thread\":\"o\",\"forks\":{\"arr\":[\"f\"]}}}}",
        )
        .unwrap();
        let enc = encode_input(&mut refs, 7, &args).unwrap();
        assert_eq!(enc.rec.len(), 30);
        let w = mk_packet(0, 1, 0, 7, &enc);
        let p = parse_pkt(&w).unwrap();
        assert_eq!(p.secs.len(), 4);
        assert_eq!(p.val(1, 3).unwrap(), V::Bool(true));
        assert_eq!(p.val(1, 6).unwrap(), V::Absent); // engine absent
        assert_eq!(p.val(1, 9).unwrap(), V::Insp);
        let et = p.val(1, 27).unwrap();
        let (sec, n) = p.table(&et, 9).unwrap();
        assert_eq!(n, 3);
        // numeric keys first: "3" then "7" then "b:x"
        assert_eq!(p.val(sec, 0).unwrap(), V::Str(u16s("3")));
        assert_eq!(p.val(sec, 3).unwrap(), V::Null);
        assert_eq!(p.val(sec, 9).unwrap(), V::Str(u16s("7")));
        assert_eq!(p.val(sec, 18).unwrap(), V::Str(u16s("b:x")));
        assert_eq!(p.val(sec, 21).unwrap(), V::Insp);
        assert_eq!(p.val(sec, 24).unwrap(), V::Str(u16s("t")));
    }

    #[test]
    fn corrupt_and_pad() {
        let mut refs = Refs::new();
        let args: Value = serde_json::from_str("{\"dataDir\":\"/d\"}").unwrap();
        let enc = encode_input(&mut refs, 1, &args).unwrap();
        let base = mk_packet(0, 1, 0, 1, &enc);
        let mut w = base.clone();
        pad_to(&mut w, 5000);
        assert_eq!(w.len(), 5000);
        // the host output decoder is strict: padding is accepted only on the Pruv input path (wire_in)
        assert!(parse_pkt(&w).is_err());
        let p = parse_pkt(&base).unwrap();
        assert_eq!(p.val(1, 0).unwrap(), V::Str(u16s("/d")));
        let mut w = base.clone();
        corrupt(&mut w, "unit");
        assert!(parse_pkt(&w).is_err());
        let mut w = base.clone();
        corrupt(&mut w, "length");
        assert!(parse_pkt(&w).is_err());
    }

    // Mock core: op 5 answers immediately with a done/ok path "/x"; used to smoke the driver and JSON line.
    struct Mock;
    impl Sol for Mock {
        fn begin(&self, input: Vec<i64>) -> Rc<Vec<i64>> {
            let mut e = Enc::new();
            e.rec.push(0);
            let vs = e.str_vs(&u16s("/x"));
            put(&mut e.rec, vs);
            e.tables.push(Vec::new());
            e.tables.pop();
            let mut w = mk_done(&e, input[2], 5);
            w[1] = 3;
            Rc::new(w)
        }
        fn step(&self, _p: &[i64], _c: &[i64]) -> Vec<i64> {
            vec![]
        }
    }
    fn mk_done(e: &Enc, op_id: i64, op: i64) -> Vec<i64> {
        // done: pool, record, empty state, empty rows
        let mut e2 = Enc { bytes: e.bytes.clone(), nu: e.nu, rec: e.rec.clone(), tables: vec![Vec::new(), Vec::new()] };
        e2.tables[0] = Vec::new();
        mk_packet(3, op_id, 0, op, &e2)
    }

    #[test]
    fn driver_smoke_and_case_errors() {
        let out = run_one(&Mock, "{\"id\":\"a\",\"op\":\"account_home\",\"args\":{\"dataDir\":\"/d\",\"engine\":\"claude\",\"id\":\"i\"}}");
        assert_eq!(out, "{\"id\":\"a\",\"terminal\":\"ok\",\"value\":\"/x\",\"trace\":[],\"driver\":null}");
        let bad = run_one(&Mock, "{nope");
        assert!(bad.starts_with("{\"id\":null,\"error\":\"case: bad JSON"));
        let bad = run_one(&Mock, "{\"id\":3,\"op\":\"nope\"}");
        assert_eq!(bad, "{\"id\":3,\"error\":\"case: unknown op nope\"}");
        let bad = run_one(&Mock, "{\"id\":4,\"op\":\"account_home\",\"args\":{\"id\":1.5}}");
        assert!(bad.starts_with("{\"id\":4,\"error\":\"case: "));
    }
}

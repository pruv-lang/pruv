// Leaf of the L3-10 budget host (crate `budget_host`): an order-preserving JSON value (J), a JSON parser/writer,
// the internal JV value (V), and the JV codec that mirrors oracle-world.mjs d() / enc().
// Included into src/lib.rs after host.rs: only plain `//` comments, no inner attributes, no `use` lines
// (host.rs owns the imports; this file names std paths in full).
//
// Why an own JSON type: serde_json without `preserve_order` sorts object keys, but the observation must keep the key
// order of the program JV (copied verbatim) and of the trace records. J keeps insertion order and the original
// number text, so Raw program JV is written back byte for byte (modulo whitespace).

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(Vec<u16>),
    Arr(Vec<J>),
    Obj(Vec<(Vec<u16>, J)>),
    // JSON text written by the JV encoder (`Heap::enc_into`, `enc_str`, `enc_num`): output only, never read back.
    Raw(String),
}

pub static JNULL: J = J::Null;

pub fn u16s(s: &str) -> Vec<u16> {
    if s.is_ascii() {
        // One unit per byte; bytes() knows its length, so this is one exact allocation.
        return s.bytes().map(u16::from).collect();
    }
    s.encode_utf16().collect()
}

pub fn ku(s: &str) -> Vec<u16> {
    u16s(s)
}

fn k_eq(k: &[u16], s: &str) -> bool {
    if s.is_ascii() {
        // One UTF-16 unit per byte: compare lengths first, then units (the keys here are ASCII literals).
        return k.len() == s.len() && k.iter().zip(s.bytes()).all(|(&u, b)| u == b as u16);
    }
    k.iter().copied().eq(s.encode_utf16())
}

impl J {
    pub fn s(x: &str) -> J {
        J::Str(u16s(x))
    }
    pub fn int(n: i64) -> J {
        J::Num(n.to_string())
    }
    // Last matching key wins (JS duplicate-key semantics).
    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(es) => es.iter().rev().find(|e| k_eq(&e.0, key)).map(|e| &e.1),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<String> {
        match self {
            J::Str(u) => Some(String::from_utf16_lossy(u)),
            _ => None,
        }
    }
}

// Option<&J> with JSON null treated as absent (JS `??`).
pub fn nn(o: Option<&J>) -> Option<&J> {
    o.filter(|x| **x != J::Null)
}

// ---- JSON writer -------------------------------------------------------------------------------------------------

fn esc_char(c: char, out: &mut String) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        c if (c as u32) < 0x20 => push_u4(c as u32, out),
        c => out.push(c),
    }
}

// "\\u" and four lowercase hex digits.
fn push_u4(x: u32, out: &mut String) {
    out.push_str("\\u");
    for sh in [12, 8, 4, 0] {
        out.push(char::from_digit((x >> sh) & 15, 16).unwrap_or('0'));
    }
}

// A unit that is written as itself: printable ASCII other than '"' and '\\'.
fn plain_unit(c: u16) -> bool {
    (0x20..0x80).contains(&c) && c != 0x22 && c != 0x5c
}

fn push_ascii(u: &[u16], out: &mut String) {
    out.reserve(u.len() + 2);
    out.push('"');
    for &c in u {
        out.push(c as u8 as char);
    }
    out.push('"');
}

fn write_str(u: &[u16], out: &mut String) {
    if u.iter().all(|&c| plain_unit(c)) {
        return push_ascii(u, out);
    }
    out.push('"');
    for r in char::decode_utf16(u.iter().copied()) {
        match r {
            Ok(c) => esc_char(c, out),
            Err(e) => push_u4(e.unpaired_surrogate() as u32, out),
        }
    }
    out.push('"');
}

// write_str of the UTF-16 units of `s`.
fn write_s(s: &str, out: &mut String) {
    if s.bytes().all(|b| plain_unit(b as u16)) {
        out.reserve(s.len() + 2);
        out.push('"');
        out.push_str(s);
        out.push('"');
        return;
    }
    out.push('"');
    for c in s.chars() {
        esc_char(c, out);
    }
    out.push('"');
}

pub fn write_json(j: &J, out: &mut String) {
    match j {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Num(t) => out.push_str(t),
        J::Str(u) => write_str(u, out),
        J::Raw(t) => out.push_str(t),
        J::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(x, out);
            }
            out.push(']');
        }
        J::Obj(es) => {
            out.push('{');
            for (i, e) in es.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(&e.0, out);
                out.push(':');
                write_json(&e.1, out);
            }
            out.push('}');
        }
    }
}

pub fn json_text(j: &J) -> String {
    let mut s = String::new();
    write_json(j, &mut s);
    s
}

// Plain text of a string J, JSON text otherwise.
pub fn plain(j: &J) -> String {
    match j.text() {
        Some(t) => t,
        None => json_text(j),
    }
}

// ---- JSON parser -------------------------------------------------------------------------------------------------

struct P<'a> {
    s: &'a str,
    b: &'a [u8],
    p: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.p < self.b.len() && matches!(self.b[self.p], b' ' | b'\n' | b'\r' | b'\t') {
            self.p += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.p).copied()
    }
    fn string(&mut self) -> Result<Vec<u16>, String> {
        self.p += 1;
        let mut out: Vec<u16> = Vec::new();
        loop {
            if self.p >= self.b.len() {
                return Err("unterminated string".to_string());
            }
            let c = self.b[self.p];
            if c == b'"' {
                self.p += 1;
                return Ok(out);
            } else if c == b'\\' {
                self.p += 1;
                let e = match self.b.get(self.p) {
                    Some(e) => *e,
                    None => return Err("bad escape".to_string()),
                };
                self.p += 1;
                match e {
                    b'"' => out.push(34),
                    b'\\' => out.push(92),
                    b'/' => out.push(47),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(10),
                    b'r' => out.push(13),
                    b't' => out.push(9),
                    b'u' => {
                        if self.p + 4 > self.b.len() {
                            return Err("bad \\u escape".to_string());
                        }
                        let h = match std::str::from_utf8(&self.b[self.p..self.p + 4]) {
                            Ok(h) => h,
                            Err(_) => return Err("bad \\u escape".to_string()),
                        };
                        let u = match u16::from_str_radix(h, 16) {
                            Ok(u) => u,
                            Err(_) => return Err("bad \\u escape".to_string()),
                        };
                        out.push(u);
                        self.p += 4;
                    }
                    _ => return Err("bad escape".to_string()),
                }
            } else {
                let ch = match self.s[self.p..].chars().next() {
                    Some(ch) => ch,
                    None => return Err("unterminated string".to_string()),
                };
                let mut buf = [0u16; 2];
                let enc = ch.encode_utf16(&mut buf);
                out.extend_from_slice(enc);
                self.p += ch.len_utf8();
            }
        }
    }
    fn val(&mut self, depth: usize) -> Result<J, String> {
        if depth > 1000 {
            return Err("json nested too deep".to_string());
        }
        self.ws();
        let c = match self.peek() {
            Some(c) => c,
            None => return Err("unexpected end of json".to_string()),
        };
        match c {
            b'{' => {
                self.p += 1;
                let mut es: Vec<(Vec<u16>, J)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.p += 1;
                    return Ok(J::Obj(es));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return Err("expected object key".to_string());
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err("expected ':'".to_string());
                    }
                    self.p += 1;
                    let v = self.val(depth + 1)?;
                    // JSON.parse: a duplicate key keeps its first position, the later value wins.
                    let pos = es.iter().position(|e| e.0 == k);
                    match pos {
                        Some(i) => es[i].1 = v,
                        None => es.push((k, v)),
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.p += 1,
                        Some(b'}') => {
                            self.p += 1;
                            break;
                        }
                        _ => return Err("expected ',' or '}'".to_string()),
                    }
                }
                Ok(J::Obj(es))
            }
            b'[' => {
                self.p += 1;
                let mut items: Vec<J> = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.p += 1;
                    return Ok(J::Arr(items));
                }
                loop {
                    items.push(self.val(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.p += 1,
                        Some(b']') => {
                            self.p += 1;
                            break;
                        }
                        _ => return Err("expected ',' or ']'".to_string()),
                    }
                }
                Ok(J::Arr(items))
            }
            b'"' => Ok(J::Str(self.string()?)),
            b't' if self.b[self.p..].starts_with(b"true") => {
                self.p += 4;
                Ok(J::Bool(true))
            }
            b'f' if self.b[self.p..].starts_with(b"false") => {
                self.p += 5;
                Ok(J::Bool(false))
            }
            b'n' if self.b[self.p..].starts_with(b"null") => {
                self.p += 4;
                Ok(J::Null)
            }
            _ => {
                let start = self.p;
                while self.p < self.b.len() && matches!(self.b[self.p], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    self.p += 1;
                }
                let t = &self.s[start..self.p];
                if t.is_empty() || t.parse::<f64>().is_err() {
                    return Err(format!("unexpected character at {}", start));
                }
                Ok(J::Num(t.to_string()))
            }
        }
    }
}

pub fn parse_json(text: &str) -> Result<J, String> {
    let mut p = P { s: text, b: text.as_bytes(), p: 0 };
    let v = p.val(0)?;
    p.ws();
    if p.p != p.b.len() {
        return Err("trailing characters after json".to_string());
    }
    Ok(v)
}

// ---- JV values ---------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum V {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(Vec<u16>),
    Arr(Vec<V>),
    Obj(Vec<(Vec<u16>, V)>),
    // handle into Heap.objs (a session-like object with identity)
    Sess(usize),
}

pub fn field<'a>(v: &'a V, key: &str) -> Option<&'a V> {
    match v {
        V::Obj(es) => es.iter().find(|e| k_eq(&e.0, key)).map(|e| &e.1),
        _ => None,
    }
}

// JS strict equality (objects by identity: only equal handles are equal).
pub fn strict_eq(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Undef, V::Undef) => true,
        (V::Null, V::Null) => true,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Num(x), V::Num(y)) => x == y,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Sess(x), V::Sess(y)) => x == y,
        _ => false,
    }
}

// Canonical array index keys ("0".."4294967294", no leading zeros).
fn index_key(k: &[u16]) -> Option<u32> {
    if k.is_empty() || k.len() > 10 || (k.len() > 1 && k[0] == 48) {
        return None;
    }
    let mut n: u64 = 0;
    for &c in k {
        if !(48..=57).contains(&c) {
            return None;
        }
        n = n * 10 + (c - 48) as u64;
    }
    if n <= 4294967294 {
        Some(n as u32)
    } else {
        None
    }
}

// Object.fromEntries semantics: a duplicate key replaces the value and keeps the first position; then JS key order
// (index keys ascending first, others in insertion order).
pub fn build_obj(entries: Vec<(Vec<u16>, V)>) -> Vec<(Vec<u16>, V)> {
    let mut out: Vec<(Vec<u16>, V)>;
    if entries.len() <= 8 {
        // Small objects dedup in place: a repeated key keeps its first position and takes the later value.
        out = entries;
        let mut i = 1;
        while i < out.len() {
            match out[..i].iter().position(|e| e.0 == out[i].0) {
                Some(j) => {
                    let (_, v) = out.remove(i);
                    out[j].1 = v;
                }
                None => i += 1,
            }
        }
    } else {
        out = Vec::with_capacity(entries.len());
        let mut seen: std::collections::HashMap<Vec<u16>, usize> = std::collections::HashMap::new();
        for (k, v) in entries {
            match seen.get(&k) {
                Some(&i) => out[i].1 = v,
                None => {
                    seen.insert(k.clone(), out.len());
                    out.push((k, v));
                }
            }
        }
    }
    if out.iter().any(|e| index_key(&e.0).is_some()) {
        let (mut idx, rest): (Vec<(Vec<u16>, V)>, Vec<(Vec<u16>, V)>) = out.into_iter().partition(|e| index_key(&e.0).is_some());
        idx.sort_by_key(|e| index_key(&e.0).unwrap_or(0));
        idx.extend(rest);
        return idx;
    }
    out
}

fn js_num(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() };
    }
    if x == 0.0 {
        return "0".to_string();
    }
    let a = x.abs();
    if a >= 1e21 || a < 1e-6 {
        let s = format!("{:e}", x);
        return match s.split_once('e') {
            Some((m, e)) => {
                if e.starts_with('-') {
                    format!("{}e{}", m, e)
                } else {
                    format!("{}e+{}", m, e)
                }
            }
            None => s,
        };
    }
    format!("{}", x)
}

// JS ToString for property keys.
pub fn js_to_string(v: &V) -> Vec<u16> {
    match v {
        V::Undef => u16s("undefined"),
        V::Null => u16s("null"),
        V::Bool(b) => u16s(if *b { "true" } else { "false" }),
        V::Num(x) => u16s(&js_num(*x)),
        V::Str(s) => s.clone(),
        V::Arr(a) => {
            let mut out: Vec<u16> = Vec::new();
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(44);
                }
                match x {
                    V::Undef | V::Null => {}
                    other => out.extend(js_to_string(other)),
                }
            }
            out
        }
        V::Obj(_) | V::Sess(_) => u16s("[object Object]"),
    }
}

// str::parse::<f64> of JSON number text. Up to 15 ASCII digits is an integer below 2^53, so the fold is exact and
// gives the same value; anything else goes through the std parser.
fn num_of(t: &str) -> Option<f64> {
    let b = t.as_bytes();
    if !b.is_empty() && b.len() <= 15 && b.iter().all(|c| c.is_ascii_digit()) {
        return Some(b.iter().fold(0u64, |n, &c| n * 10 + (c - b'0') as u64) as f64);
    }
    t.parse::<f64>().ok()
}

fn to_u16(x: f64) -> u16 {
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(65536.0) as u16
}

pub fn no_sess(_id: &[u16]) -> Option<usize> {
    None
}

// d(): input JV -> V. `sess` resolves {sess:"id"} to the handle of the currently stored session with that id.
pub fn decode(j: &J, sess: &dyn Fn(&[u16]) -> Option<usize>) -> Result<V, String> {
    match j {
        J::Null => Ok(V::Null),
        J::Bool(b) => Ok(V::Bool(*b)),
        J::Num(t) => match num_of(t) {
            Some(x) => Ok(V::Num(x)),
            None => Err(format!("bad number {}", t)),
        },
        J::Str(u) => Ok(V::Str(u.clone())),
        J::Raw(_) => Err("encoded text is not input".to_string()),
        J::Arr(a) => {
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                out.push(decode(x, sess)?);
            }
            Ok(V::Arr(out))
        }
        J::Obj(es) => {
            if es.len() == 1 {
                let key = &es[0].0;
                let v = &es[0].1;
                if k_eq(key, "undef") {
                    return Ok(V::Undef);
                }
                if k_eq(key, "u16") {
                    return match v {
                        J::Arr(a) => Ok(V::Str(
                            a.iter()
                                .map(|x| match x {
                                    J::Num(t) => to_u16(num_of(t).unwrap_or(0.0)),
                                    _ => 0,
                                })
                                .collect(),
                        )),
                        _ => Err("bad {u16}".to_string()),
                    };
                }
                if k_eq(key, "num") {
                    if let J::Str(u) = v {
                        let t = String::from_utf16_lossy(u);
                        if t.len() == 16 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
                            if let Ok(bits) = u64::from_str_radix(&t, 16) {
                                return Ok(V::Num(f64::from_bits(bits)));
                            }
                        }
                    }
                    return Err("bad {num}".to_string());
                }
                if k_eq(key, "arr") {
                    return match v {
                        J::Arr(a) => {
                            let mut out = Vec::with_capacity(a.len());
                            for x in a {
                                out.push(decode(x, sess)?);
                            }
                            Ok(V::Arr(out))
                        }
                        _ => Err("bad {arr}".to_string()),
                    };
                }
                if k_eq(key, "obj") {
                    let pairs = match v {
                        J::Arr(a) => a,
                        _ => return Err("bad {obj}".to_string()),
                    };
                    let mut ents: Vec<(Vec<u16>, V)> = Vec::with_capacity(pairs.len());
                    for p in pairs {
                        match p {
                            J::Arr(kv) if kv.len() == 2 => {
                                // A plain string key is its own String(key); anything else is decoded and converted.
                                let k = match &kv[0] {
                                    J::Str(u) => u.clone(),
                                    kj => js_to_string(&decode(kj, sess)?),
                                };
                                let vd = decode(&kv[1], sess)?;
                                ents.push((k, vd));
                            }
                            _ => return Err("bad {obj} pair".to_string()),
                        }
                    }
                    return Ok(V::Obj(build_obj(ents)));
                }
                if k_eq(key, "sess") {
                    if let J::Str(u) = v {
                        if let Some(h) = sess(u) {
                            return Ok(V::Sess(h));
                        }
                    }
                    return Err(format!("{{sess:{}}} names no stored session", json_text(v)));
                }
            }
            let mut ents: Vec<(Vec<u16>, V)> = Vec::with_capacity(es.len());
            for e in es {
                ents.push((e.0.clone(), decode(&e.1, sess)?));
            }
            Ok(V::Obj(build_obj(ents)))
        }
    }
}

// ---- encoding ----------------------------------------------------------------------------------------------------
//
// enc() writes JSON text straight from the host values (one encoder; the J results of `enc`/`enc_str`/`enc_num` are
// J::Raw text). Output is the same text the earlier J-tree encoder wrote through write_json.

// A JS string: as is when well formed, {"u16":[units]} when it holds a lone surrogate.
pub fn enc_str_into(u: &[u16], out: &mut String) {
    if u.iter().all(|&c| plain_unit(c)) {
        return push_ascii(u, out);
    }
    if char::decode_utf16(u.iter().copied()).all(|r| r.is_ok()) {
        return write_str(u, out);
    }
    out.push_str("{\"u16\":[");
    for (i, &c) in u.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_int(c as i64, out);
    }
    out.push_str("]}");
}

fn push_int(n: i64, out: &mut String) {
    if n < 0 {
        out.push('-');
    }
    push_uint(n.unsigned_abs(), out);
}

fn push_uint(mut u: u64, out: &mut String) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    out.push_str(std::str::from_utf8(&buf[i..]).unwrap_or_default());
}

// A JS number: a safe integer (not -0) as decimal text, anything else as {"num":"<16 hex digits of the bits>"}.
pub fn enc_num_into(x: f64, out: &mut String) {
    if x.fract() == 0.0 && x.abs() <= 9007199254740991.0 && !(x == 0.0 && x.is_sign_negative()) {
        push_int(x as i64, out);
    } else {
        let _ = std::fmt::Write::write_fmt(out, format_args!("{{\"num\":\"{:016x}\"}}", x.to_bits()));
    }
}

pub fn enc_str(u: &[u16]) -> J {
    let mut s = String::new();
    enc_str_into(u, &mut s);
    J::Raw(s)
}

pub fn enc_num(x: f64) -> J {
    let mut s = String::new();
    enc_num_into(x, &mut s);
    J::Raw(s)
}

// enc() of a plain JS object given already-encoded values: {"obj":[[k,v],...]}.
pub fn enc_obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(vec![(ku("obj"), J::Arr(pairs.into_iter().map(|(k, v)| J::Arr(vec![J::s(k), v])).collect()))])
}

// An object with a stable identity: a session (stored or not) whose label is assigned on first encode.
pub struct SObj {
    pub val: V,
    pub label: Option<Label>,
    // label of the budget object held under the key "budget"
    pub bud: Option<Label>,
}

// A session ('s') or budget ('b') label. The text "<p><n>" with n in canonical decimal (every label the encoder
// assigns) is kept as n; any other label text as is (from String::from_utf16_lossy, as before).
#[derive(Clone)]
pub enum Label {
    N(u64),
    T(String),
}

// n when `u` is p followed by the canonical decimal text of a u64 (no sign, no leading zero, no overflow).
fn canon_num(u: &[u16], p: char) -> Option<u64> {
    let d = match u.split_first() {
        Some((&c, d)) if c as u32 == p as u32 && !d.is_empty() => d,
        _ => return None,
    };
    if d.len() > 1 && d[0] == b'0' as u16 {
        return None;
    }
    let mut n: u64 = 0;
    for &c in d {
        let x = c.wrapping_sub(b'0' as u16);
        if x > 9 {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(x as u64)?;
    }
    Some(n)
}

fn label_of(u: &[u16], p: char) -> Label {
    match canon_num(u, p) {
        Some(n) => Label::N(n),
        None => Label::T(String::from_utf16_lossy(u)),
    }
}

fn push_label(l: &Label, p: char, out: &mut String) {
    match l {
        Label::N(n) => {
            out.push('"');
            out.push(p);
            push_uint(*n, out);
            out.push('"');
        }
        Label::T(t) => write_s(t, out),
    }
}

fn label_text(l: &Label, p: char) -> String {
    match l {
        Label::N(n) => format!("{}{}", p, n),
        Label::T(t) => t.clone(),
    }
}

pub struct Heap {
    pub objs: Vec<SObj>,
    pub s_seq: u64,
    pub b_seq: u64,
    // session label -> handle: canonical labels by number, others by text
    by_num: std::collections::HashMap<u64, usize>,
    by_text: std::collections::HashMap<String, usize>,
}

// `p` then a u64 as str::parse::<u64> reads it (optional '+', ASCII digits, no overflow), straight from the UTF-16 units.
fn label_num(u: &[u16], p: char) -> Option<u64> {
    let rest = match u.split_first() {
        Some((&c, rest)) if c as u32 == p as u32 => rest,
        _ => return None,
    };
    let digits = match rest.split_first() {
        Some((&c, d)) if c == b'+' as u16 => d,
        _ => rest,
    };
    if digits.is_empty() {
        return None;
    }
    let mut n: u64 = 0;
    for &c in digits {
        let d = c.wrapping_sub(b'0' as u16);
        if d > 9 {
            return None;
        }
        n = n.checked_mul(10)?.checked_add(d as u64)?;
    }
    Some(n)
}

// The value of the "budget" pair inside an encoded {"obj":[...]} session, if any.
fn budget_pair(v: &J) -> Option<&J> {
    if let J::Obj(es) = v {
        if es.len() == 1 && k_eq(&es[0].0, "obj") {
            if let J::Arr(pairs) = &es[0].1 {
                for p in pairs {
                    if let J::Arr(kv) = p {
                        if kv.len() == 2 {
                            if let J::Str(kk) = &kv[0] {
                                if k_eq(kk, "budget") {
                                    return Some(&kv[1]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// Replace {"budget": {"bref"":"bJ","v":W}} inside an encoded {"obj":[...]} session by "budget": W and report bJ.
fn strip_budget(v: &J, bud: &mut Option<Label>) -> J {
    if let J::Obj(es) = v {
        if es.len() == 1 && k_eq(&es[0].0, "obj") {
            if let J::Arr(pairs) = &es[0].1 {
                let mut np: Vec<J> = Vec::with_capacity(pairs.len());
                for p in pairs {
                    let mut done = false;
                    if let J::Arr(kv) = p {
                        if kv.len() == 2 {
                            if let (J::Str(kk), J::Obj(be)) = (&kv[0], &kv[1]) {
                                if k_eq(kk, "budget") {
                                    let lab = be.iter().find(|e| k_eq(&e.0, "bref")).and_then(|e| bref_label(&e.1));
                                    let inner = be.iter().find(|e| k_eq(&e.0, "v")).map(|e| e.1.clone());
                                    if let (Some(l), Some(i)) = (lab, inner) {
                                        *bud = Some(l);
                                        np.push(J::Arr(vec![kv[0].clone(), i]));
                                        done = true;
                                    }
                                }
                            }
                        }
                    }
                    if !done {
                        np.push(p.clone());
                    }
                }
                return J::Obj(vec![(ku("obj"), J::Arr(np))]);
            }
        }
    }
    v.clone()
}

fn bref_label(j: &J) -> Option<Label> {
    match j {
        J::Str(u) => Some(label_of(u, 'b')),
        _ => None,
    }
}

impl Heap {
    pub fn new() -> Heap {
        Heap { objs: Vec::new(), s_seq: 0, b_seq: 0, by_num: std::collections::HashMap::new(), by_text: std::collections::HashMap::new() }
    }

    fn find_label(&self, l: &Label) -> Option<usize> {
        match l {
            Label::N(n) => self.by_num.get(n).copied(),
            Label::T(t) => self.by_text.get(t).copied(),
        }
    }

    fn put_label(&mut self, l: Label, h: usize) {
        match l {
            Label::N(n) => self.by_num.insert(n, h),
            Label::T(t) => self.by_text.insert(t, h),
        };
    }

    pub fn alloc(&mut self, val: V) -> usize {
        self.objs.push(SObj { val, label: None, bud: None });
        self.objs.len() - 1
    }

    pub fn enc(&mut self, v: &V) -> J {
        let mut s = String::new();
        self.enc_into(v, &mut s);
        J::Raw(s)
    }

    // enc() of `v` appended to `out` as JSON text. Labels are assigned on first encode, in the order they are written.
    pub fn enc_into(&mut self, v: &V, out: &mut String) {
        match v {
            V::Undef => out.push_str("{\"undef\":1}"),
            V::Null => out.push_str("null"),
            V::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            V::Str(u) => enc_str_into(u, out),
            V::Num(x) => enc_num_into(*x, out),
            V::Arr(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    self.enc_into(x, out);
                }
                out.push(']');
            }
            V::Obj(es) => {
                out.push_str("{\"obj\":[");
                for (i, e) in es.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('[');
                    enc_str_into(&e.0, out);
                    out.push(',');
                    self.enc_into(&e.1, out);
                    out.push(']');
                }
                out.push_str("]}");
            }
            V::Sess(h) => self.enc_session(*h, out),
        }
    }

    fn enc_session(&mut self, h: usize, out: &mut String) {
        if self.objs[h].label.is_none() {
            self.s_seq += 1;
            self.by_num.insert(self.s_seq, h);
            self.objs[h].label = Some(Label::N(self.s_seq));
        }
        out.push_str("{\"sref\":");
        if let Some(l) = &self.objs[h].label {
            push_label(l, 's', out);
        }
        out.push_str(",\"v\":{\"obj\":[");
        // The value is taken out while encoding so that nested handles can borrow the heap mutably.
        let val = std::mem::replace(&mut self.objs[h].val, V::Undef);
        if let V::Obj(es) = &val {
            for (i, e) in es.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                enc_str_into(&e.0, out);
                out.push(',');
                if k_eq(&e.0, "budget") && matches!(e.1, V::Obj(_)) {
                    self.enc_budget(h, &e.1, out);
                } else {
                    self.enc_into(&e.1, out);
                }
                out.push(']');
            }
        }
        self.objs[h].val = val;
        out.push_str("]}}");
    }

    fn enc_budget(&mut self, h: usize, budget: &V, out: &mut String) {
        if self.objs[h].bud.is_none() {
            self.b_seq += 1;
            self.objs[h].bud = Some(Label::N(self.b_seq));
        }
        out.push_str("{\"bref\":");
        if let Some(l) = &self.objs[h].bud {
            push_label(l, 'b', out);
        }
        out.push_str(",\"v\":");
        self.enc_into(budget, out);
        out.push('}');
    }

    // Raise the label counters over every sref / bref label found in program JV.
    pub fn note_raw(&mut self, j: &J) {
        match j {
            J::Arr(a) => {
                for x in a {
                    self.note_raw(x);
                }
            }
            J::Obj(es) => {
                for e in es {
                    if let J::Str(u) = &e.1 {
                        if k_eq(&e.0, "sref") {
                            if let Some(n) = label_num(u, 's') {
                                if n > self.s_seq {
                                    self.s_seq = n;
                                }
                            }
                        } else if k_eq(&e.0, "bref") {
                            if let Some(n) = label_num(u, 'b') {
                                if n > self.b_seq {
                                    self.b_seq = n;
                                }
                            }
                        }
                    }
                    self.note_raw(&e.1);
                }
            }
            _ => {}
        }
    }

    // Models TS in-place mutation of a shared session: every {"sref":"sK","v":V} in program JV sets the budget attachment
    // of the host object labelled sK (other fields stay host-owned); an unknown sK is created, unstored, from V.
    pub fn write_through(&mut self, j: &J) -> Result<(), String> {
        match j {
            J::Arr(a) => {
                for x in a {
                    self.write_through(x)?;
                }
                Ok(())
            }
            J::Obj(es) => {
                let sref = es.iter().rev().find(|e| k_eq(&e.0, "sref")).map(|e| &e.1);
                let vv = es.iter().rev().find(|e| k_eq(&e.0, "v")).map(|e| &e.1);
                if let (Some(J::Str(lab)), Some(v)) = (sref, vv) {
                    return self.apply_sref(lab, v);
                }
                for e in es {
                    self.write_through(&e.1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // Program item bset: the core's budget attachment of the session labeled `label`. Some(b) sets the key "budget"
    // (in place, or appended as a new key, as in JS), None deletes it.
    pub fn set_budget(&mut self, label: &str, b: Option<&J>) -> Result<(), String> {
        let h = match self.find_label(&label_of(&u16s(label), 's')) {
            Some(h) => h,
            None => return Err(format!("program: bset on unknown session {}", label)),
        };
        self.set_budget_h(h, b)
    }

    fn set_budget_h(&mut self, h: usize, b: Option<&J>) -> Result<(), String> {
        let es = match &mut self.objs[h].val {
            V::Obj(es) => es,
            _ => {
                let l = self.objs[h].label.as_ref().map(|l| label_text(l, 's')).unwrap_or_default();
                return Err(format!("program: bset on non-object session {}", l));
            }
        };
        let pos = es.iter().position(|e| k_eq(&e.0, "budget"));
        match b {
            None => {
                if let Some(p) = pos {
                    es.remove(p);
                }
                self.objs[h].bud = None;
            }
            Some(bj) => {
                // Decode the budget value directly: {"bref":L,"v":W} attaches label L and decodes W, anything else
                // decodes as is (same result as decoding the one-pair object {"obj":[["budget", bj]]} after strip_budget).
                let (bud, val): (Option<Label>, V) = match bj {
                    J::Obj(be) => {
                        let lab = be.iter().find(|e| k_eq(&e.0, "bref")).and_then(|e| bref_label(&e.1));
                        let inner = be.iter().find(|e| k_eq(&e.0, "v"));
                        match (lab, inner) {
                            (Some(l), Some(i)) => (Some(l), decode(&i.1, &no_sess)?),
                            _ => (None, decode(bj, &no_sess)?),
                        }
                    }
                    _ => (None, decode(bj, &no_sess)?),
                };
                match pos {
                    Some(p) => es[p].1 = val,
                    None => es.push((ku("budget"), val)),
                }
                self.objs[h].bud = bud;
            }
        }
        Ok(())
    }

    // Program JV as the host sees it: every {"sref":"sK","v":..} of a known session is re-encoded from the host object.
    pub fn host_view(&mut self, j: &J) -> J {
        match j {
            J::Arr(a) => J::Arr(a.iter().map(|x| self.host_view(x)).collect()),
            J::Obj(es) => {
                let sref = es.iter().rev().find(|e| k_eq(&e.0, "sref")).map(|e| &e.1);
                if let Some(J::Str(u)) = sref {
                    if let Some(h) = self.find_label(&label_of(u, 's')) {
                        return self.enc(&V::Sess(h));
                    }
                    return j.clone();
                }
                J::Obj(es.iter().map(|e| (e.0.clone(), self.host_view(&e.1))).collect())
            }
            _ => j.clone(),
        }
    }

    fn apply_sref(&mut self, label: &[u16], v: &J) -> Result<(), String> {
        let lab = label_of(label, 's');
        if let Some(h) = self.find_label(&lab) {
            return self.set_budget_h(h, budget_pair(v));
        }
        let mut bud: Option<Label> = None;
        let vj = strip_budget(v, &mut bud);
        let val = decode(&vj, &no_sess)?;
        self.objs.push(SObj { val, label: Some(lab.clone()), bud });
        let h = self.objs.len() - 1;
        self.put_label(lab, h);
        Ok(())
    }
}

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
}

pub static JNULL: J = J::Null;

pub fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn ku(s: &str) -> Vec<u16> {
    u16s(s)
}

fn k_eq(k: &[u16], s: &str) -> bool {
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

fn write_str(u: &[u16], out: &mut String) {
    out.push('"');
    for r in char::decode_utf16(u.iter().copied()) {
        match r {
            Ok(c) => match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            },
            Err(e) => out.push_str(&format!("\\u{:04x}", e.unpaired_surrogate())),
        }
    }
    out.push('"');
}

pub fn write_json(j: &J, out: &mut String) {
    match j {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Num(t) => out.push_str(t),
        J::Str(u) => write_str(u, out),
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
    let mut out: Vec<(Vec<u16>, V)> = Vec::with_capacity(entries.len());
    if entries.len() > 8 {
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
    } else {
        for (k, v) in entries {
            let pos = out.iter().position(|e| e.0 == k);
            match pos {
                Some(i) => out[i].1 = v,
                None => out.push((k, v)),
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
        J::Num(t) => match t.parse::<f64>() {
            Ok(x) => Ok(V::Num(x)),
            Err(_) => Err(format!("bad number {}", t)),
        },
        J::Str(u) => Ok(V::Str(u.clone())),
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
                                    J::Num(t) => to_u16(t.parse::<f64>().unwrap_or(0.0)),
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
                                let kd = decode(&kv[0], sess)?;
                                let vd = decode(&kv[1], sess)?;
                                ents.push((js_to_string(&kd), vd));
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

pub fn enc_str(u: &[u16]) -> J {
    if char::decode_utf16(u.iter().copied()).all(|r| r.is_ok()) {
        J::Str(u.to_vec())
    } else {
        J::Obj(vec![(ku("u16"), J::Arr(u.iter().map(|x| J::int(*x as i64)).collect()))])
    }
}

pub fn enc_num(x: f64) -> J {
    if x.fract() == 0.0 && x.abs() <= 9007199254740991.0 && !(x == 0.0 && x.is_sign_negative()) {
        J::Num(format!("{}", x as i64))
    } else {
        J::Obj(vec![(ku("num"), J::s(&format!("{:016x}", x.to_bits())))])
    }
}

// enc() of a plain JS object given already-encoded values: {"obj":[[k,v],...]}.
pub fn enc_obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(vec![(ku("obj"), J::Arr(pairs.into_iter().map(|(k, v)| J::Arr(vec![J::s(k), v])).collect()))])
}

// An object with a stable identity: a session (stored or not) whose label is assigned on first encode.
pub struct SObj {
    pub val: V,
    pub label: Option<String>,
    // label of the budget object held under the key "budget"
    pub bud: Option<String>,
}

pub struct Heap {
    pub objs: Vec<SObj>,
    pub s_seq: u64,
    pub b_seq: u64,
    by_label: std::collections::HashMap<String, usize>,
}

fn label_num(u: &[u16], p: char) -> Option<u64> {
    let s = String::from_utf16_lossy(u);
    let rest = s.strip_prefix(p)?;
    rest.parse::<u64>().ok()
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
fn strip_budget(v: &J, bud: &mut Option<String>) -> J {
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
                                    let lab = be.iter().find(|e| k_eq(&e.0, "bref")).and_then(|e| e.1.text());
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

impl Heap {
    pub fn new() -> Heap {
        Heap { objs: Vec::new(), s_seq: 0, b_seq: 0, by_label: std::collections::HashMap::new() }
    }

    pub fn alloc(&mut self, val: V) -> usize {
        self.objs.push(SObj { val, label: None, bud: None });
        self.objs.len() - 1
    }

    pub fn enc(&mut self, v: &V) -> J {
        match v {
            V::Undef => J::Obj(vec![(ku("undef"), J::int(1))]),
            V::Null => J::Null,
            V::Bool(b) => J::Bool(*b),
            V::Str(u) => enc_str(u),
            V::Num(x) => enc_num(*x),
            V::Arr(a) => {
                let mut out: Vec<J> = Vec::with_capacity(a.len());
                for x in a {
                    out.push(self.enc(x));
                }
                J::Arr(out)
            }
            V::Obj(es) => self.enc_plain(es),
            V::Sess(h) => self.enc_session(*h),
        }
    }

    fn enc_plain(&mut self, es: &[(Vec<u16>, V)]) -> J {
        let mut pairs: Vec<J> = Vec::with_capacity(es.len());
        for e in es {
            let k = enc_str(&e.0);
            let x = self.enc(&e.1);
            pairs.push(J::Arr(vec![k, x]));
        }
        J::Obj(vec![(ku("obj"), J::Arr(pairs))])
    }

    fn enc_session(&mut self, h: usize) -> J {
        if self.objs[h].label.is_none() {
            self.s_seq += 1;
            let l = format!("s{}", self.s_seq);
            self.by_label.insert(l.clone(), h);
            self.objs[h].label = Some(l);
        }
        let label = self.objs[h].label.clone().unwrap_or_default();
        // The value is taken out while encoding so that nested handles can borrow the heap mutably.
        let val = std::mem::replace(&mut self.objs[h].val, V::Undef);
        let mut pairs: Vec<J> = Vec::new();
        if let V::Obj(es) = &val {
            for e in es {
                let ev = if k_eq(&e.0, "budget") && matches!(e.1, V::Obj(_)) { self.enc_budget(h, &e.1) } else { self.enc(&e.1) };
                pairs.push(J::Arr(vec![enc_str(&e.0), ev]));
            }
        }
        self.objs[h].val = val;
        J::Obj(vec![(ku("sref"), J::s(&label)), (ku("v"), J::Obj(vec![(ku("obj"), J::Arr(pairs))]))])
    }

    fn enc_budget(&mut self, h: usize, budget: &V) -> J {
        if self.objs[h].bud.is_none() {
            self.b_seq += 1;
            self.objs[h].bud = Some(format!("b{}", self.b_seq));
        }
        let label = self.objs[h].bud.clone().unwrap_or_default();
        let body = self.enc(budget);
        J::Obj(vec![(ku("bref"), J::s(&label)), (ku("v"), body)])
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
        let h = match self.by_label.get(label) {
            Some(&h) => h,
            None => return Err(format!("program: bset on unknown session {}", label)),
        };
        let es = match &mut self.objs[h].val {
            V::Obj(es) => es,
            _ => return Err(format!("program: bset on non-object session {}", label)),
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
                let (bud, val): (Option<String>, V) = match bj {
                    J::Obj(be) => {
                        let lab = be.iter().find(|e| k_eq(&e.0, "bref")).and_then(|e| e.1.text());
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
                let sref = es.iter().rev().find(|e| k_eq(&e.0, "sref")).and_then(|e| e.1.text());
                if let Some(l) = sref {
                    if let Some(&h) = self.by_label.get(&l) {
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
        let lab = String::from_utf16_lossy(label);
        if self.by_label.contains_key(&lab) {
            let b = budget_pair(v);
            return self.set_budget(&lab, b);
        }
        let mut bud: Option<String> = None;
        let vj = strip_budget(v, &mut bud);
        let val = decode(&vj, &no_sess)?;
        self.objs.push(SObj { val, label: Some(lab.clone()), bud });
        let h = self.objs.len() - 1;
        self.by_label.insert(lab, h);
        Ok(())
    }
}

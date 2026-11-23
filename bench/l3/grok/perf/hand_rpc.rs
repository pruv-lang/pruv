// Hand-written safe Rust counterpart of grok_rpc_lines (perf_root.pruv): the per-line work of the ACP rpc pump (rpc.pruv).
// Included by run_perf.py as `mod hand_rpc { include!("hand_rpc.rs"); }` (plain comments only: no inner doc comments).
//
// One pass per line: trim (ECMAScript whitespace as UTF-8, same steps as rpc::trim_bounds), then ONE validating recursive
// descent scanner over the trimmed bytes of the window itself (no JSON tape, no per-line copy). While scanning, the scanner
// remembers only the spans (offset range, kind, "has no escapes") of the few members the pump needs: root id, method, params ->
// update -> content -> text, error -> message / code. Duplicate keys: the last occurrence wins (a new occurrence of params /
// update / content / error resets what was captured below it). Strings are decoded afterwards, only the ones needed, into one
// reused buffer (and not at all when they hold no escape: the raw bytes are folded directly). The error detail is folded
// straight from the source (no buffer).
//
// Scope limits (the bench inputs stay inside them): object keys are compared by raw spelling (a key written with escapes never
// matches); the raw error.code is the source token of the value, which equals util::json_value for JSON integer codes.
//
// Result: 15 words, see perf_root.pruv: 0 lines, 1 failed parses, 2 kind 0, 3 kind 1, 4 kind 2, 5/6 text bytes/fold,
// 7/8 not_handled bytes/fold, 9/10 detail bytes/fold, 11/12 code bytes/fold, 13/14 method bytes/fold.

const P: u64 = 1_000_000_007;
const MAX_DEPTH: usize = 128; // open containers
const MAX_RECORDS: u32 = 1_048_576; // keys and values together, as the Pruv tape counts them
const MAX_INPUT: usize = 16_777_216;

// value kinds
const ABSENT: u8 = 0;
const STR: u8 = 1;
const NUM: u8 = 2;
const OBJ: u8 = 3;
const OTHER: u8 = 4; // null, true, false, array

// which members of which object are tracked
const T_NONE: u8 = 0;
const T_ROOT: u8 = 1;
const T_PARAMS: u8 = 2;
const T_UPDATE: u8 = 3;
const T_CONTENT: u8 = 4;
const T_ERROR: u8 = 5;

// capture slots
const S_NONE: u8 = 0;
const S_ID: u8 = 1;
const S_METHOD: u8 = 2;
const S_TEXT: u8 = 3;
const S_MSG: u8 = 4;
const S_CODE: u8 = 5;
const S_ERROR: u8 = 6;

// A scanned value: token span in the trimmed line (strings include their quotes), kind, and for strings "no backslash inside".
#[derive(Clone, Copy)]
struct Val {
    s: usize,
    e: usize,
    kind: u8,
    plain: bool,
}

const NOVAL: Val = Val { s: 0, e: 0, kind: ABSENT, plain: false };

#[derive(Clone, Copy)]
struct Cap {
    id: Val,
    method: Val,
    text: Val,
    msg: Val,
    code: Val,
    err_obj: bool,
}

fn space(b: &[u8], i: usize) -> usize {
    let c = b[i];
    if c == 32 || (9..=13).contains(&c) {
        return 1;
    }
    if c < 194 {
        return 0;
    }
    if i + 1 < b.len() && c == 194 && b[i + 1] == 160 {
        return 2;
    }
    if i + 2 < b.len() {
        let (c1, c2) = (b[i + 1], b[i + 2]);
        if c == 225 && c1 == 154 && c2 == 128 {
            return 3;
        }
        if c == 226 && c1 == 128 && ((128..=138).contains(&c2) || c2 == 168 || c2 == 169 || c2 == 175) {
            return 3;
        }
        if c == 226 && c1 == 129 && c2 == 159 {
            return 3;
        }
        if c == 227 && c1 == 128 && c2 == 128 {
            return 3;
        }
        if c == 239 && c1 == 187 && c2 == 191 {
            return 3;
        }
    }
    0
}

// rpc::trim_bounds: left to right over `space` steps; lo is the end of the leading run, hi the position after the last
// non-space byte step (hi = lo when everything is whitespace).
fn trim_bounds(b: &[u8]) -> (usize, usize) {
    let n = b.len();
    let mut lo = 0;
    while lo < n {
        let k = space(b, lo);
        if k == 0 {
            break;
        }
        lo += k;
    }
    if lo >= n {
        return (n, n);
    }
    // an ASCII non-space last byte can never be part of a multi-byte space step, so the walk would end at n
    let last = b[n - 1];
    if last < 128 && last != 32 && !(9..=13).contains(&last) {
        return (lo, n);
    }
    let mut hi = 0;
    let mut i = lo;
    while i < n {
        let k = space(b, i);
        if k > 0 {
            i += k;
        } else {
            i += 1;
            hi = i;
        }
    }
    if hi < lo {
        hi = lo;
    }
    (lo, hi)
}

// width of a valid UTF-8 sequence whose lead byte (>= 128) is at i, 0 if invalid (same rules as parse::utf8_width)
fn utf8_width(b: &[u8], i: usize) -> usize {
    let c = b[i];
    let n = if (194..=223).contains(&c) {
        2
    } else if (224..=239).contains(&c) {
        3
    } else if (240..=244).contains(&c) {
        4
    } else {
        return 0;
    };
    if b.len() - i < n {
        return 0;
    }
    let c1 = b[i + 1];
    if !(128..=191).contains(&c1) || (c == 224 && c1 < 160) || (c == 237 && c1 >= 160) || (c == 240 && c1 < 144) || (c == 244 && c1 > 143) {
        return 0;
    }
    for j in 2..n {
        if !(128..=191).contains(&b[i + j]) {
            return 0;
        }
    }
    n
}

struct Sc<'a> {
    b: &'a [u8],
    p: usize,
    rec: u32,
    cap: Cap,
}

impl<'a> Sc<'a> {
    fn ws(&mut self) {
        while self.p < self.b.len() && matches!(self.b[self.p], b' ' | b'\t' | b'\n' | b'\r') {
            self.p += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.p).copied()
    }

    // p at the opening quote; on success p is after the closing quote; returns "no escapes"
    fn string(&mut self) -> Option<bool> {
        let b = self.b;
        let n = b.len();
        let mut i = self.p + 1;
        let mut plain = true;
        loop {
            while i < n {
                let c = b[i];
                if c == b'"' || c == b'\\' || c < 32 || c >= 128 {
                    break;
                }
                i += 1;
            }
            if i >= n {
                return None;
            }
            match b[i] {
                b'"' => {
                    self.p = i + 1;
                    return Some(plain);
                }
                b'\\' => {
                    plain = false;
                    i += 1;
                    if i >= n {
                        return None;
                    }
                    match b[i] {
                        b'u' => {
                            if i + 5 > n || !b[i + 1..i + 5].iter().all(|c| c.is_ascii_hexdigit()) {
                                return None;
                            }
                            i += 5;
                        }
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 1,
                        _ => return None,
                    }
                }
                c if c < 32 => return None,
                _ => {
                    let w = utf8_width(b, i);
                    if w == 0 {
                        return None;
                    }
                    i += w;
                }
            }
        }
    }

    // RFC 8259 number: -? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?
    fn number(&mut self) -> Option<()> {
        let b = self.b;
        let n = b.len();
        let mut i = self.p;
        if b[i] == b'-' {
            i += 1;
        }
        match b.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => {
                while i < n && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            _ => return None,
        }
        if i < n && b[i] == b'.' {
            i += 1;
            let d = i;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == d {
                return None;
            }
        }
        if i < n && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < n && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            let d = i;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == d {
                return None;
            }
        }
        self.p = i;
        Some(())
    }

    fn lit(&mut self, w: &[u8]) -> Option<()> {
        if self.b[self.p..].starts_with(w) {
            self.p += w.len();
            Some(())
        } else {
            None
        }
    }

    // depth = number of open containers around this value; track says which tracked object this value is, if it is an object
    fn value(&mut self, depth: usize, track: u8) -> Option<Val> {
        self.ws();
        let b = self.b;
        if self.p >= b.len() || self.rec >= MAX_RECORDS {
            return None;
        }
        self.rec += 1;
        let s = self.p;
        let (kind, plain) = match b[s] {
            b'"' => (STR, self.string()?),
            b'{' => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                self.p += 1;
                self.object(depth + 1, track)?;
                (OBJ, false)
            }
            b'[' => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                self.p += 1;
                self.array(depth + 1)?;
                (OTHER, false)
            }
            b't' => {
                self.lit(b"true")?;
                (OTHER, false)
            }
            b'f' => {
                self.lit(b"false")?;
                (OTHER, false)
            }
            b'n' => {
                self.lit(b"null")?;
                (OTHER, false)
            }
            b'-' | b'0'..=b'9' => {
                self.number()?;
                (NUM, false)
            }
            _ => return None,
        };
        Some(Val { s, e: self.p, kind, plain })
    }

    fn array(&mut self, depth: usize) -> Option<()> {
        self.ws();
        if self.peek() == Some(b']') {
            self.p += 1;
            return Some(());
        }
        loop {
            self.value(depth, T_NONE)?;
            self.ws();
            match self.peek()? {
                b',' => self.p += 1,
                b']' => {
                    self.p += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    // which slot a member fills and which tracked object its value is; a new occurrence resets what hangs below it
    fn member(&mut self, track: u8, key: &[u8]) -> (u8, u8) {
        match (track, key) {
            (T_ROOT, b"id") => (S_ID, T_NONE),
            (T_ROOT, b"method") => (S_METHOD, T_NONE),
            (T_ROOT, b"params") => {
                self.cap.text = NOVAL;
                (S_NONE, T_PARAMS)
            }
            (T_ROOT, b"error") => {
                self.cap.msg = NOVAL;
                self.cap.code = NOVAL;
                self.cap.err_obj = false;
                (S_ERROR, T_ERROR)
            }
            (T_PARAMS, b"update") => {
                self.cap.text = NOVAL;
                (S_NONE, T_UPDATE)
            }
            (T_UPDATE, b"content") => {
                self.cap.text = NOVAL;
                (S_NONE, T_CONTENT)
            }
            (T_CONTENT, b"text") => (S_TEXT, T_NONE),
            (T_ERROR, b"message") => (S_MSG, T_NONE),
            (T_ERROR, b"code") => (S_CODE, T_NONE),
            _ => (S_NONE, T_NONE),
        }
    }

    // p is after the '{'
    fn object(&mut self, depth: usize, track: u8) -> Option<()> {
        self.ws();
        if self.peek() == Some(b'}') {
            self.p += 1;
            return Some(());
        }
        let b = self.b;
        loop {
            self.ws();
            if self.peek()? != b'"' || self.rec >= MAX_RECORDS {
                return None;
            }
            self.rec += 1;
            let ks = self.p;
            let plain = self.string()?;
            let ke = self.p;
            self.ws();
            if self.peek()? != b':' {
                return None;
            }
            self.p += 1;
            let key: &[u8] = if plain && track != T_NONE { &b[ks + 1..ke - 1] } else { &[] };
            let (slot, child) = self.member(track, key);
            let v = self.value(depth, child)?;
            match slot {
                S_ID => self.cap.id = v,
                S_METHOD => self.cap.method = v,
                S_TEXT => self.cap.text = v,
                S_MSG => self.cap.msg = v,
                S_CODE => self.cap.code = v,
                S_ERROR => self.cap.err_obj = v.kind == OBJ,
                _ => {}
            }
            self.ws();
            match self.peek()? {
                b',' => self.p += 1,
                b'}' => {
                    self.p += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }
}

fn fold(mut s: u64, bytes: &[u8]) -> u64 {
    for &c in bytes {
        s = (s * 31 + c as u64) % P;
    }
    s
}

fn bump(a: u64, n: usize) -> u64 {
    (a + n as u64 % P) % P
}

fn hex4(h: &[u8]) -> u32 {
    let mut v = 0u32;
    for &c in &h[..4] {
        v = v * 16 + (c as char).to_digit(16).unwrap_or(0);
    }
    v
}

// JSON unescape of the (already validated) inside of a string into UTF-8; a surrogate pair combines, a lone one is U+FFFD
fn unescape(inner: &[u8], out: &mut Vec<u8>) {
    let n = inner.len();
    let mut i = 0;
    while i < n {
        let j = inner[i..].iter().position(|&c| c == b'\\').map_or(n, |k| i + k);
        out.extend_from_slice(&inner[i..j]);
        i = j;
        if i >= n {
            break;
        }
        let e = inner[i + 1];
        if e == b'u' {
            let u = hex4(&inner[i + 2..]);
            i += 6;
            let cp = if (0xD800..0xDC00).contains(&u) {
                if i + 6 <= n && inner[i] == b'\\' && inner[i + 1] == b'u' {
                    let lo = hex4(&inner[i + 2..]);
                    if (0xDC00..0xE000).contains(&lo) {
                        i += 6;
                        0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)
                    } else {
                        0xFFFD
                    }
                } else {
                    0xFFFD
                }
            } else if (0xDC00..0xE000).contains(&u) {
                0xFFFD
            } else {
                u
            };
            let mut tmp = [0u8; 4];
            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
            out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
        } else {
            out.push(match e {
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                other => other, // '"', '\\', '/'
            });
            i += 2;
        }
    }
}

// decoded UTF-8 of a string value (empty for anything else); borrows the source when there is nothing to unescape
fn text_of<'a>(src: &'a [u8], v: &Val, buf: &'a mut Vec<u8>) -> &'a [u8] {
    if v.kind != STR {
        return &[];
    }
    let inner = &src[v.s + 1..v.e - 1];
    if v.plain {
        return inner;
    }
    buf.clear();
    unescape(inner, buf);
    &buf[..]
}

// the error detail folded straight from the source: first 300 UTF-16 units, 32..=126 as themselves, the rest '?'
struct Detail {
    s: u64,
    n: usize,
}

impl Detail {
    fn emit(&mut self, unit: u32) {
        if self.n < 300 {
            let c = if (32..=126).contains(&unit) { unit as u8 } else { b'?' };
            self.s = (self.s * 31 + c as u64) % P;
            self.n += 1;
        }
    }
}

fn detail(src: &[u8], v: &Val, s0: u64) -> (usize, u64) {
    let mut d = Detail { s: s0, n: 0 };
    if v.kind != STR {
        let r = b"rejected";
        return (r.len(), fold(s0, r));
    }
    let inner = &src[v.s + 1..v.e - 1];
    let n = inner.len();
    let mut i = 0;
    while i < n && d.n < 300 {
        let c = inner[i];
        if c == b'\\' {
            let e = inner[i + 1];
            if e == b'u' {
                d.emit(hex4(&inner[i + 2..]));
                i += 6;
            } else {
                d.emit(match e {
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    other => other as u32,
                });
                i += 2;
            }
        } else if c < 128 {
            d.emit(c as u32);
            i += 1;
        } else {
            // one UTF-16 unit for 2 and 3 byte sequences, two for 4 byte ones; none of them is printable ASCII
            let w = if c >= 0xF0 { 4 } else if c >= 0xE0 { 3 } else { 2 };
            d.emit(0x80);
            if w == 4 {
                d.emit(0x80);
            }
            i += w;
        }
    }
    (d.n, d.s)
}

const NH_HEAD: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":";
const NH_TAIL: &[u8] = b",\"error\":{\"code\":-32601,\"message\":\"not handled\"}}\n";

fn line(raw: &[u8], acc: &mut [u64; 15], buf: &mut Vec<u8>) {
    acc[0] = bump(acc[0], 1);
    let (lo, hi) = trim_bounds(raw);
    let t = &raw[lo..hi];
    if t.is_empty() || t.len() > MAX_INPUT {
        acc[1] = bump(acc[1], 1);
        return;
    }
    let mut sc = Sc { b: t, p: 0, rec: 0, cap: Cap { id: NOVAL, method: NOVAL, text: NOVAL, msg: NOVAL, code: NOVAL, err_obj: false } };
    let ok = sc.value(0, T_ROOT).is_some() && {
        sc.ws();
        sc.p == t.len()
    };
    if !ok {
        acc[1] = bump(acc[1], 1);
        return;
    }
    let cap = sc.cap; // stays empty unless the root is an object
    let kind = if cap.id.kind != ABSENT && cap.method.kind == STR {
        1
    } else if cap.id.kind == NUM && &t[cap.id.s..cap.id.e] == b"2" {
        2
    } else {
        0
    };
    if kind == 1 {
        let idr = &t[cap.id.s..cap.id.e];
        acc[3] = bump(acc[3], 1);
        acc[7] = bump(acc[7], NH_HEAD.len() + idr.len() + NH_TAIL.len());
        acc[8] = fold(fold(fold(acc[8], NH_HEAD), idr), NH_TAIL);
    } else if kind == 2 {
        acc[4] = bump(acc[4], 1);
        if cap.err_obj {
            let (n, s) = detail(t, &cap.msg, acc[10]);
            acc[9] = bump(acc[9], n);
            acc[10] = s;
            if cap.code.kind != ABSENT {
                let code = &t[cap.code.s..cap.code.e];
                acc[11] = bump(acc[11], code.len());
                acc[12] = fold(acc[12], code);
            }
        }
    } else {
        acc[2] = bump(acc[2], 1);
        let m = text_of(t, &cap.method, buf);
        acc[13] = bump(acc[13], m.len());
        acc[14] = fold(acc[14], m);
        let x = text_of(t, &cap.text, buf);
        acc[5] = bump(acc[5], x.len());
        acc[6] = fold(acc[6], x);
    }
}

pub fn rpc_lines(win: &[u8]) -> Vec<i64> {
    let mut acc = [0u64; 15];
    let mut buf: Vec<u8> = Vec::new();
    let mut start = 0;
    while let Some(k) = win[start..].iter().position(|&c| c == 10) {
        line(&win[start..start + k], &mut acc, &mut buf);
        start += k + 1;
    }
    if start < win.len() {
        line(&win[start..], &mut acc, &mut buf);
    }
    acc.iter().map(|&w| w as i64).collect()
}

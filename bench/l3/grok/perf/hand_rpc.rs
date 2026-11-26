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
// Reset 4.3: tracked keys are compared after unescaping (key_is, as access::field does), error.code is the compact
// util::json_value form (compact), and result -> agentCapabilities -> loadSession is tracked for the load check. load_check at the
// end is the hand counterpart of rpc::load_check (process, reader thread, one overall deadline); the perf driver does not run it.
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
const T_RESULT: u8 = 6;
const T_CAPS: u8 = 7;

// capture slots
const S_NONE: u8 = 0;
const S_ID: u8 = 1;
const S_METHOD: u8 = 2;
const S_TEXT: u8 = 3;
const S_MSG: u8 = 4;
const S_CODE: u8 = 5;
const S_ERROR: u8 = 6;
const S_RESULT: u8 = 7;
const S_CAPS: u8 = 8;
const S_LOAD: u8 = 9;

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
    res_obj: bool,
    caps_obj: bool,
    load_true: bool,
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
    fn member(&mut self, track: u8, key: &[u8], plain: bool) -> (u8, u8) {
        let is = |lit: &[u8]| key_is(key, plain, lit);
        match track {
            T_ROOT if is(b"id") => (S_ID, T_NONE),
            T_ROOT if is(b"method") => (S_METHOD, T_NONE),
            T_ROOT if is(b"params") => {
                self.cap.text = NOVAL;
                (S_NONE, T_PARAMS)
            }
            T_ROOT if is(b"error") => {
                self.cap.msg = NOVAL;
                self.cap.code = NOVAL;
                self.cap.err_obj = false;
                (S_ERROR, T_ERROR)
            }
            T_ROOT if is(b"result") => {
                self.cap.caps_obj = false;
                self.cap.load_true = false;
                (S_RESULT, T_RESULT)
            }
            T_PARAMS if is(b"update") => {
                self.cap.text = NOVAL;
                (S_NONE, T_UPDATE)
            }
            T_UPDATE if is(b"content") => {
                self.cap.text = NOVAL;
                (S_NONE, T_CONTENT)
            }
            T_CONTENT if is(b"text") => (S_TEXT, T_NONE),
            T_ERROR if is(b"message") => (S_MSG, T_NONE),
            T_ERROR if is(b"code") => (S_CODE, T_NONE),
            T_RESULT if is(b"agentCapabilities") => {
                self.cap.load_true = false;
                (S_CAPS, T_CAPS)
            }
            T_CAPS if is(b"loadSession") => (S_LOAD, T_NONE),
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
            let (slot, child) = if track != T_NONE { self.member(track, &b[ks + 1..ke - 1], plain) } else { (S_NONE, T_NONE) };
            let v = self.value(depth, child)?;
            match slot {
                S_ID => self.cap.id = v,
                S_METHOD => self.cap.method = v,
                S_TEXT => self.cap.text = v,
                S_MSG => self.cap.msg = v,
                S_CODE => self.cap.code = v,
                S_ERROR => self.cap.err_obj = v.kind == OBJ,
                S_RESULT => self.cap.res_obj = v.kind == OBJ,
                S_CAPS => self.cap.caps_obj = v.kind == OBJ,
                S_LOAD => self.cap.load_true = v.kind == OTHER && b[v.s] == b't',
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

// the key body (inside the quotes) equals the ASCII literal after JSON unescaping; a \u escape is one code unit
fn key_is(key: &[u8], plain: bool, lit: &[u8]) -> bool {
    if plain {
        return key == lit;
    }
    let mut i = 0;
    let mut k = 0;
    while i < key.len() {
        if k >= lit.len() {
            return false;
        }
        let (u, n) = if key[i] == b'\\' {
            match key[i + 1] {
                b'u' => (hex4(&key[i + 2..]), 6),
                b'b' => (8, 2),
                b'f' => (12, 2),
                b'n' => (10, 2),
                b'r' => (13, 2),
                b't' => (9, 2),
                other => (other as u32, 2),
            }
        } else {
            (key[i] as u32, 1)
        };
        if u != lit[k] as u32 {
            return false;
        }
        k += 1;
        i += n;
    }
    k == lit.len()
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

// the error detail from the source span into out (cleared first): first 300 UTF-16 units, 32..=126 as themselves, the rest '?',
// "rejected" when the message is missing or not a string
fn emit(out: &mut Vec<u8>, unit: u32) {
    if out.len() < 300 {
        out.push(if (32..=126).contains(&unit) { unit as u8 } else { b'?' });
    }
}

fn detail(src: &[u8], v: &Val, out: &mut Vec<u8>) {
    out.clear();
    if v.kind != STR {
        out.extend_from_slice(b"rejected");
        return;
    }
    let inner = &src[v.s + 1..v.e - 1];
    let n = inner.len();
    let mut i = 0;
    while i < n && out.len() < 300 {
        let c = inner[i];
        if c == b'\\' {
            let e = inner[i + 1];
            if e == b'u' {
                emit(out, hex4(&inner[i + 2..]));
                i += 6;
            } else {
                emit(out, match e {
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
            emit(out, c as u32);
            i += 1;
        } else {
            // one UTF-16 unit for 2 and 3 byte sequences, two for 4 byte ones; none of them is printable ASCII
            let w = if c >= 0xF0 { 4 } else if c >= 0xE0 { 3 } else { 2 };
            emit(out, 0x80);
            if w == 4 {
                emit(out, 0x80);
            }
            i += w;
        }
    }
}

// util::json_value of a scanned value: whitespace dropped, numbers, literals and keys (raw spelling with quotes) copied, string
// values unescaped and re-escaped like util::escaped (quote, backslash, C0 as \u00xx); a plain string is its own re-escaping
fn compact(src: &[u8], v: &Val, out: &mut Vec<u8>, buf: &mut Vec<u8>) {
    out.clear();
    let t = &src[..v.e];
    let mut q = v.s;
    while q < v.e {
        let c = t[q];
        if c != b'"' {
            if !matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                out.push(c);
            }
            q += 1;
            continue;
        }
        // the closing quote: the first '"' not preceded by an escaping backslash
        let mut e = q + 1;
        let mut plain = true;
        while t[e] != b'"' {
            if t[e] == b'\\' {
                plain = false;
                e += 1;
            }
            e += 1;
        }
        let mut w = e + 1;
        while w < v.e && matches!(t[w], b' ' | b'\t' | b'\n' | b'\r') {
            w += 1;
        }
        let key = w < v.e && t[w] == b':';
        if key || plain {
            out.extend_from_slice(&t[q..=e]);
        } else {
            buf.clear();
            unescape(&t[q + 1..e], buf);
            out.push(b'"');
            for &y in buf.iter() {
                match y {
                    b'"' => out.extend_from_slice(b"\\\""),
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    0..=31 => out.extend_from_slice(&[b'\\', b'u', b'0', b'0', HEX[(y / 16) as usize], HEX[(y % 16) as usize]]),
                    _ => out.push(y),
                }
            }
            out.push(b'"');
        }
        q = e + 1;
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

const NH_HEAD: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":";
const NH_TAIL: &[u8] = b",\"error\":{\"code\":-32601,\"message\":\"not handled\"}}\n";

// one validating scan of the trimmed line t; the captured members (empty unless the root is an object)
fn scan(t: &[u8]) -> Option<Cap> {
    let empty = Cap { id: NOVAL, method: NOVAL, text: NOVAL, msg: NOVAL, code: NOVAL, err_obj: false, res_obj: false, caps_obj: false, load_true: false };
    let mut sc = Sc { b: t, p: 0, rec: 0, cap: empty };
    sc.value(0, T_ROOT)?;
    sc.ws();
    if sc.p != t.len() {
        return None;
    }
    Some(sc.cap)
}

fn line(raw: &[u8], acc: &mut [u64; 15], buf: &mut Vec<u8>, out: &mut Vec<u8>) {
    acc[0] = bump(acc[0], 1);
    let (lo, hi) = trim_bounds(raw);
    let t = &raw[lo..hi];
    if t.is_empty() || t.len() > MAX_INPUT {
        acc[1] = bump(acc[1], 1);
        return;
    }
    let Some(cap) = scan(t) else {
        acc[1] = bump(acc[1], 1);
        return;
    };
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
        // DB65 (RUST_SHAPE_RESET §7: the hand computes the full result): the reply the pump sends is built, one exact Vec
        let mut nh = Vec::with_capacity(NH_HEAD.len() + idr.len() + NH_TAIL.len());
        nh.extend_from_slice(NH_HEAD);
        nh.extend_from_slice(idr);
        nh.extend_from_slice(NH_TAIL);
        let nh = std::hint::black_box(nh);
        acc[7] = bump(acc[7], nh.len());
        acc[8] = fold(acc[8], &nh);
    } else if kind == 2 {
        acc[4] = bump(acc[4], 1);
        if cap.err_obj {
            detail(t, &cap.msg, out);
            acc[9] = bump(acc[9], out.len());
            acc[10] = fold(acc[10], out);
            if cap.code.kind != ABSENT {
                compact(t, &cap.code, out, buf);
                acc[11] = bump(acc[11], out.len());
                acc[12] = fold(acc[12], out);
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
    let mut out: Vec<u8> = Vec::new();
    let mut start = 0;
    while let Some(k) = win[start..].iter().position(|&c| c == 10) {
        line(&win[start..start + k], &mut acc, &mut buf, &mut out);
        start += k + 1;
    }
    if start < win.len() {
        line(&win[start..], &mut acc, &mut buf, &mut out);
    }
    acc.iter().map(|&w| w as i64).collect()
}

// ---- the load check (rpc::load_check) ----
// The ACP check of a copied session: spawn `binary agent --no-leader stdio` (cwd folder, GROK_HOME=home), initialize (id 1),
// require result.agentCapabilities.loadSession === true, then session/load (id 2). One overall deadline; a reader thread hands
// stdout chunks over a channel (the Pruv side has the G10 event loop instead: gap row TH). Server requests get the not-handled
// reply. Err carries what rpc::failure puts into the result packet (stage, class, code, raw error.code, name, detail).

#[allow(dead_code)]
pub struct LoadError {
    pub stage: u8,
    pub class: u8,
    pub code: u8,
    pub raw: Vec<u8>,
    pub name: &'static str,
    pub detail: String,
}

// rpc::Check.fail
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum Fail {
    Internal,
    Rpc,
    NoLoad,
    Timeout,
    ReadFailed,
    Missing,
}

#[allow(dead_code)]
struct Check {
    fail: Option<Fail>,
    stage: u8,
    detail: Vec<u8>,
    raw: Vec<u8>,
}

#[allow(dead_code)]
const REQ_INIT: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{\"fs\":{\"readTextFile\":false,\"writeTextFile\":false},\"terminal\":false},\"clientInfo\":{\"name\":\"lair\",\"title\":\"Lair\",\"version\":\"0.1.0\"}}}\n";

#[allow(dead_code)]
fn push_escaped(out: &mut Vec<u8>, s: &str) {
    for &c in s.as_bytes() {
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0..=31 => out.extend_from_slice(&[b'\\', b'u', b'0', b'0', HEX[(c / 16) as usize], HEX[(c % 16) as usize]]),
            _ => out.push(c),
        }
    }
}

#[allow(dead_code)]
fn req_load(thread: &str, folder: &str) -> Vec<u8> {
    let a: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/load\",\"params\":{\"sessionId\":\"";
    let b: &[u8] = b"\",\"cwd\":\"";
    let z: &[u8] = b"\",\"mcpServers\":[]}}\n";
    let mut out = Vec::with_capacity(a.len() + b.len() + z.len() + 6 * thread.len() + 6 * folder.len());
    out.extend_from_slice(a);
    push_escaped(&mut out, thread);
    out.extend_from_slice(b);
    push_escaped(&mut out, folder);
    out.extend_from_slice(z);
    out
}

// the child, its stdin (None once a write failed: half-closed), the chunk being read and the line being assembled
#[allow(dead_code)]
struct Rpc {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    deadline: std::time::Instant,
    chunk: Vec<u8>,
    at: usize,
    line: Vec<u8>,
}

const LINE_CAP: usize = 8_388_608;

#[allow(dead_code)]
impl Rpc {
    fn send(&mut self, data: &[u8]) {
        use std::io::Write;
        if let Some(w) = self.stdin.as_mut() {
            if w.write_all(data).and_then(|_| w.flush()).is_err() {
                self.stdin = None;
            }
        }
    }

    // rpc::step: the next complete line (in self.line), reading a new chunk only when `wait` and the current one is used up
    fn step(&mut self, ck: &mut Check, wait: bool) -> bool {
        loop {
            if self.at < self.chunk.len() {
                let rest = &self.chunk[self.at..];
                if let Some(k) = rest.iter().position(|&c| c == b'\n') {
                    // the line buffer never holds more than LINE_CAP bytes (rpc::step, transport::receive_grow)
                    if self.line.len() + k > LINE_CAP {
                        ck.fail = Some(Fail::ReadFailed);
                        return false;
                    }
                    self.line.extend_from_slice(&rest[..k]);
                    self.at += k + 1;
                    return true;
                }
                if self.line.len() + rest.len() > LINE_CAP {
                    ck.fail = Some(Fail::ReadFailed);
                    return false;
                }
                self.line.extend_from_slice(rest);
                self.at = self.chunk.len();
            }
            if !wait {
                return false;
            }
            let left = self.deadline.saturating_duration_since(std::time::Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(c) => {
                    self.chunk = c;
                    self.at = 0;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    ck.fail = Some(Fail::Timeout);
                    return false;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    ck.fail = Some(Fail::ReadFailed);
                    return false;
                }
            }
        }
    }

    // one line waiting for `id` (99: nothing outstanding): true when it was the response to `id`
    fn on_line(&mut self, ck: &mut Check, id: u8, buf: &mut Vec<u8>) -> bool {
        let line = std::mem::take(&mut self.line);
        let (lo, hi) = trim_bounds(&line);
        let t = &line[lo..hi];
        let mut done = false;
        if !t.is_empty() && t.len() <= MAX_INPUT {
            if let Some(cap) = scan(t) {
                let digit = [b'0' + id];
                if cap.id.kind != ABSENT && cap.method.kind == STR {
                    let idr = &t[cap.id.s..cap.id.e];
                    let mut nh = Vec::with_capacity(NH_HEAD.len() + idr.len() + NH_TAIL.len());
                    nh.extend_from_slice(NH_HEAD);
                    nh.extend_from_slice(idr);
                    nh.extend_from_slice(NH_TAIL);
                    self.send(&nh);
                } else if id != 99 && cap.id.kind == NUM && t[cap.id.s..cap.id.e] == digit {
                    done = true;
                    on_reply(t, &cap, id, ck, buf);
                }
            }
        }
        self.line = line;
        self.line.clear();
        done
    }

    fn pump(&mut self, ck: &mut Check, id: u8, buf: &mut Vec<u8>) {
        let mut done = false;
        let mut events = 0;
        while ck.fail.is_none() && !done && events < 1_000_000 {
            events += 1;
            if self.step(ck, true) {
                done = self.on_line(ck, id, buf);
            }
        }
        if ck.fail.is_none() && !done {
            ck.fail = Some(Fail::ReadFailed);
        }
    }

    // rpc::before_send: a child whose stdin is already closed (an earlier write failed) is a timeout when the deadline has
    // passed, else read-failed; no request is written to it
    fn before_send(&mut self, ck: &mut Check) {
        if self.stdin.is_none() {
            ck.fail = Some(if std::time::Instant::now() >= self.deadline { Fail::Timeout } else { Fail::ReadFailed });
        }
    }

    // rpc::drain: the remaining complete lines of the current chunk, nothing outstanding
    fn drain(&mut self, ck: &mut Check, buf: &mut Vec<u8>) {
        while ck.fail.is_none() && self.at < self.chunk.len() {
            if self.step(ck, false) {
                self.on_line(ck, 99, buf);
            }
        }
    }
}

#[allow(dead_code)]
fn on_reply(t: &[u8], cap: &Cap, id: u8, ck: &mut Check, buf: &mut Vec<u8>) {
    if cap.err_obj {
        ck.fail = Some(Fail::Rpc);
        detail(t, &cap.msg, &mut ck.detail);
        if cap.code.kind != ABSENT {
            compact(t, &cap.code, &mut ck.raw, buf);
            if ck.raw.is_empty() || ck.raw.len() > LINE_CAP {
                ck.fail = Some(Fail::Internal);
            }
        }
    } else if id == 1 && !(cap.res_obj && cap.caps_obj && cap.load_true) {
        ck.fail = Some(Fail::NoLoad);
    }
}

#[allow(dead_code)]
pub fn load_check(binary: &str, home: &str, folder: &str, thread: &str, timeout_ms: u64) -> Result<(), LoadError> {
    use std::process::{Command, Stdio};
    let mut ck = Check { fail: None, stage: 24, detail: Vec::new(), raw: Vec::new() };
    let ms = if timeout_ms == 0 { 20_000 } else { timeout_ms.min(86_400_000) };
    let spawned = Command::new(binary)
        .args(["agent", "--no-leader", "stdio"])
        .current_dir(folder)
        .env("GROK_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(_) => return Err(failure(Fail::Missing, 24, Vec::new(), Vec::new())),
    };
    let stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut chunk = vec![0u8; 65536];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(chunk[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut rpc = Rpc { child, stdin, rx, deadline, chunk: Vec::new(), at: 0, line: Vec::with_capacity(4096) };
    let mut buf: Vec<u8> = Vec::new();
    ck.stage = 25;
    rpc.before_send(&mut ck);
    rpc.send(REQ_INIT);
    rpc.pump(&mut ck, 1, &mut buf);
    if ck.fail.is_none() {
        rpc.drain(&mut ck, &mut buf);
    }
    if ck.fail.is_none() {
        ck.stage = 26;
        rpc.before_send(&mut ck);
        rpc.send(&req_load(thread, folder));
        rpc.pump(&mut ck, 2, &mut buf);
    }
    // a settling response leaves the rest of its chunk: server requests in it are still answered before the child is closed
    let settled = ck.fail;
    if matches!(settled, None | Some(Fail::Internal) | Some(Fail::Rpc) | Some(Fail::NoLoad)) {
        ck.fail = None;
        rpc.drain(&mut ck, &mut buf);
        ck.fail = settled;
    }
    drop(rpc.stdin.take());
    let _ = rpc.child.kill();
    let _ = rpc.child.wait();
    drop(rpc);
    let _ = reader.join();
    match ck.fail {
        None => Ok(()),
        Some(f) => Err(failure(f, ck.stage, ck.raw, ck.detail)),
    }
}

// rpc::failure: the error the result packet carries
#[allow(dead_code)]
fn failure(f: Fail, stage: u8, raw: Vec<u8>, detail: Vec<u8>) -> LoadError {
    let e = |stage, class, code, name| LoadError { stage, class, code, raw: Vec::new(), name, detail: String::new() };
    match f {
        Fail::Rpc => LoadError { stage, class: 3, code: 0, raw, name: "RpcResponseError", detail: String::from_utf8(detail).unwrap_or_default() },
        Fail::Missing => e(24, 1, 8, "EngineImportError"),
        Fail::ReadFailed => e(stage, 1, 7, "EngineImportError"),
        Fail::Timeout => e(stage, 1, 6, "EngineImportError"),
        Fail::NoLoad => e(25, 1, 5, "EngineImportError"),
        Fail::Internal => e(30, 6, 2, ""),
    }
}

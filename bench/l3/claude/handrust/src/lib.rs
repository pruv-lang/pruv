//! Safe hand-written Claude JSONL packet baseline for head, tail, and window.
//! Each function is marked `// Pruv: module::fn` with its Pruv counterpart (RUST_SHAPE_RESET §6 step 4.3h).
//! No hand counterpart: boundary::check_output (packet self-check), boundary::copy_span and the a0-a4 probe
//! roots, import::last_max / first_present / replaces (A2 selection probes).

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::str;

const EDGE: usize = 65_536;
const WINDOW: usize = 8_388_608;
const LINE: usize = 1_048_576;
const DROP: usize = 8192;
const LIM: usize = 32768;
const TOTAL: usize = 2_097_152;
const MAXTURNS: usize = 200;
const SYNTHETIC: &str = "<synthetic>";
const SAFE: i64 = 9_007_199_254_740_991;
const MIN_MS: i64 = -62_167_219_200_000;
const MAX_MS: i64 = 253_402_300_799_999;

type Stamp = Option<[u8; 24]>;

// ---------------- ECMAScript whitespace and text helpers ----------------
// Pruv: text::space_len
fn is_es_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

// Pruv: text::trim_span, text::trim
fn es_trim(s: &str) -> &str {
    s.trim_matches(is_es_ws)
}

/// Whitespace runs -> one space, trim, then cut to `mx` UTF-16 units on a scalar boundary.
// Pruv: text::one_line
fn one_line(s: &str, mx: usize) -> String {
    let mut out = String::new();
    let mut pending = false;
    let mut u = 0usize;
    for c in s.chars() {
        if is_es_ws(c) {
            pending = true;
            continue;
        }
        if pending && !out.is_empty() {
            if u + 1 > mx {
                break;
            }
            out.push(' ');
            u += 1;
        }
        pending = false;
        let w = c.len_utf16();
        if u + w > mx {
            break;
        }
        out.push(c);
        u += w;
    }
    out
}

/// Longest prefix length <= mx that ends on a scalar boundary of valid UTF-8 `raw`.
// Pruv: text::clip_len (text::clip_bytes is the A1 probe)
fn clip_len(raw: &[u8], mx: usize) -> usize {
    if raw.len() <= mx {
        return raw.len();
    }
    let mut k = mx;
    while k > 0 && (raw[k] & 0xC0) == 0x80 {
        k -= 1;
    }
    k
}

// ---------------- timestamps ----------------
// Pruv: time (civil date helpers)
fn is_leap(y: u32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn mdays(y: u32, m: u32) -> u32 {
    match m {
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Pruv: time::canon
fn canon(s: &[u8]) -> bool {
    if s.len() != 24 {
        return false;
    }
    for (i, &c) in s.iter().enumerate() {
        let ok = match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 | 16 => c == b':',
            19 => c == b'.',
            23 => c == b'Z',
            _ => c.is_ascii_digit(),
        };
        if !ok {
            return false;
        }
    }
    let n = |a: usize, l: usize| -> u32 { s[a..a + l].iter().fold(0u32, |x, &c| x * 10 + (c - b'0') as u32) };
    let (y, mo, d, h, mi, sec) = (n(0, 4), n(5, 2), n(8, 2), n(11, 2), n(14, 2), n(17, 2));
    (1..=12).contains(&mo) && d >= 1 && d <= mdays(y, mo) && h <= 23 && mi <= 59 && sec <= 59
}

fn put_digits(o: &mut [u8; 24], at: usize, n: usize, mut v: i64) {
    for k in (0..n).rev() {
        o[at + k] = b'0' + (v % 10) as u8;
        v /= 10;
    }
}

/// `ms` within [MIN_MS, MAX_MS] -> "YYYY-MM-DDTHH:MM:SS.mmmZ".
// Pruv: time::format_iso
fn format_iso(ms: i64) -> [u8; 24] {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let secs = rem / 1000;
    let mut o = *b"0000-00-00T00:00:00.000Z";
    put_digits(&mut o, 0, 4, y);
    put_digits(&mut o, 5, 2, m);
    put_digits(&mut o, 8, 2, d);
    put_digits(&mut o, 11, 2, secs / 3600);
    put_digits(&mut o, 14, 2, secs % 3600 / 60);
    put_digits(&mut o, 17, 2, secs % 60);
    put_digits(&mut o, 20, 3, rem % 1000);
    o
}

// Pruv: time::iso_of_int
fn iso_of_int(v: i64) -> Stamp {
    let ms = if v < 100_000_000_000 { v * 1000 } else { v };
    if (MIN_MS..=MAX_MS).contains(&ms) {
        Some(format_iso(ms))
    } else {
        None
    }
}

// Pruv: time::iso_of (number branch)
fn iso_number(text: &str) -> Stamp {
    if text.bytes().any(|c| matches!(c, b'.' | b'e' | b'E')) {
        return None;
    }
    if text.trim_start_matches('-').len() > 17 {
        return None;
    }
    let n: i64 = text.parse().ok()?;
    if n.abs() > SAFE {
        return None;
    }
    iso_of_int(n)
}

// Pruv: time::iso_of (string branch)
fn iso_str(s: &str) -> Stamp {
    let t = es_trim(s);
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        let digits = t.trim_start_matches('0');
        if digits.len() > 16 {
            return None;
        }
        let n: i64 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
        if n > SAFE {
            return None;
        }
        return iso_of_int(n);
    }
    if s.len() == 24 && canon(s.as_bytes()) {
        let mut o = [0u8; 24];
        o.copy_from_slice(s.as_bytes());
        return Some(o);
    }
    None
}

// Pruv: time::iso_of; import::stamp_from adds the jstr adapter check
fn iso(v: Option<&Value<'_>>) -> Stamp {
    match v? {
        Value::Number(n) => iso_number(n),
        Value::String(s) => iso_str(s),
        _ => None,
    }
}

// ---------------- oversized-line probe (the oracle's o_dropped regexes) ----------------
/// Byte length of the ECMAScript whitespace character at `i` (0 if none).
// Pruv: text::ws_at
fn ws_len(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(0x09..=0x0D) | Some(0x20) => 1,
        Some(0xC2) => (b.get(i + 1) == Some(&0xA0)) as usize * 2,
        Some(0xE1) => (b.get(i + 1) == Some(&0x9A) && b.get(i + 2) == Some(&0x80)) as usize * 3,
        Some(0xE2) => match (b.get(i + 1), b.get(i + 2)) {
            (Some(0x80), Some(0x80..=0x8A)) | (Some(0x80), Some(0xA8)) | (Some(0x80), Some(0xA9)) => 3,
            (Some(0x80), Some(0xAF)) | (Some(0x81), Some(0x9F)) => 3,
            _ => 0,
        },
        Some(0xE3) => (b.get(i + 1) == Some(&0x80) && b.get(i + 2) == Some(&0x80)) as usize * 3,
        Some(0xEF) => (b.get(i + 1) == Some(&0xBB) && b.get(i + 2) == Some(&0xBF)) as usize * 3,
        _ => 0,
    }
}

// Pruv: text::skip_ws
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        let n = ws_len(b, i);
        if n == 0 {
            return i;
        }
        i += n;
    }
}

// Pruv: text::dropped_user (one forward scan; row 4.3h-D)
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    let first = needle[0];
    let mut i = from;
    while i + needle.len() <= hay.len() {
        match hay[i..hay.len() - needle.len() + 1].iter().position(|&c| c == first) {
            None => return None,
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    return Some(i);
                }
                i += 1;
            }
        }
    }
    None
}

/// key WS* ':' WS* val, anywhere.
// Pruv: text::kv, text::colon_value
fn key_colon_val(b: &[u8], key: &[u8], val: &[u8]) -> bool {
    let mut from = 0;
    while let Some(p) = find(b, key, from) {
        let mut i = skip_ws(b, p + key.len());
        if b.get(i) == Some(&b':') {
            i = skip_ws(b, i + 1);
            if b[i..].starts_with(val) {
                return true;
            }
        }
        from = p + 1;
    }
    false
}

/// "content" WS* : WS* [ WS* { [^{}]* "type" WS* : WS* "tool_result"
// Pruv: text::content_hit, text::tool_result_after
fn has_tool_result(b: &[u8]) -> bool {
    let key = b"\"content\"";
    let mut from = 0;
    while let Some(p) = find(b, key, from) {
        from = p + 1;
        let mut i = skip_ws(b, p + key.len());
        if b.get(i) != Some(&b':') {
            continue;
        }
        i = skip_ws(b, i + 1);
        if b.get(i) != Some(&b'[') {
            continue;
        }
        i = skip_ws(b, i + 1);
        if b.get(i) != Some(&b'{') {
            continue;
        }
        i += 1;
        let end = b[i..].iter().position(|&c| c == b'{' || c == b'}').map_or(b.len(), |x| i + x);
        if key_colon_val(&b[i..end], b"\"type\"", b"\"tool_result\"") {
            return true;
        }
    }
    false
}

// Pruv: text::dropped_user
fn dropped_user(head: &[u8]) -> bool {
    let s = String::from_utf8_lossy(head);
    let b = s.as_bytes();
    key_colon_val(b, b"\"type\"", b"\"user\"")
        && !(has_tool_result(b)
            || key_colon_val(b, b"\"isMeta\"", b"true")
            || key_colon_val(b, b"\"isSidechain\"", b"true"))
}

// Objects and raw number tokens are distinct even for serde_json's private
// number key. Keys use WTF-8 for lone surrogates; values use U+FFFD.
type Object<'a> = BTreeMap<Cow<'a, [u8]>, Value<'a>>;

enum Value<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    String(Cow<'a, str>),
    Array(Vec<Value<'a>>),
    Object(Object<'a>),
}

// Pruv: pkgs parse::parse / lex (JSON tape, not a Value tree; accepted representation)
struct Parser<'a>(&'a str);

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.0.as_bytes().first().copied()
    }

    fn eat(&mut self, token: &str) -> bool {
        if let Some(rest) = self.0.strip_prefix(token) {
            self.0 = rest;
            true
        } else {
            false
        }
    }

    fn ws(&mut self) {
        self.0 = self.0.trim_start_matches([' ', '\t', '\r', '\n']);
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.0.get(..4)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.0 = &self.0[4..];
        u32::from_str_radix(digits, 16).ok()
    }

    fn escape(&mut self, out: &mut Vec<u8>, key: bool) -> Option<()> {
        let b = self.peek()?;
        if !b.is_ascii() {
            return None;
        }
        self.0 = &self.0[1..];
        match b {
            b'"' | b'\\' | b'/' => out.push(b),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => {
                let mut cp = self.hex4()?;
                if (0xD800..0xDC00).contains(&cp) {
                    let rest = self.0;
                    if self.eat("\\u") {
                        let low = self.hex4()?;
                        if (0xDC00..0xE000).contains(&low) {
                            cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                        } else {
                            self.0 = rest;
                        }
                    }
                }
                if let Some(c) = char::from_u32(cp) {
                    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                } else if key {
                    out.extend_from_slice(&[
                        0xED, (0x80 | ((cp >> 6) & 0x3F)) as u8, (0x80 | (cp & 0x3F)) as u8,
                    ]);
                } else {
                    out.extend_from_slice("\u{FFFD}".as_bytes());
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn string(&mut self, key: bool) -> Option<Cow<'a, [u8]>> {
        if !self.eat("\"") {
            return None;
        }
        let mut decoded = None::<Vec<u8>>;
        loop {
            let end = self.0.bytes().position(|b| matches!(b, b'"' | b'\\' | 0..=31))?;
            let chunk = &self.0.as_bytes()[..end];
            let mark = self.0.as_bytes()[end];
            self.0 = &self.0[end + 1..];
            match mark {
                b'"' => return Some(match decoded {
                    Some(mut buf) => {
                        buf.extend_from_slice(chunk);
                        Cow::Owned(buf)
                    }
                    None => Cow::Borrowed(chunk),
                }),
                b'\\' => {
                    let buf = decoded.get_or_insert_with(Vec::new);
                    buf.extend_from_slice(chunk);
                    self.escape(buf, key)?;
                }
                _ => return None,
            }
        }
    }

    fn digits(&mut self) -> bool {
        let n = self.0.bytes().take_while(u8::is_ascii_digit).count();
        self.0 = &self.0[n..];
        n != 0
    }

    fn number(&mut self) -> Option<&'a str> {
        let start = self.0;
        self.eat("-");
        if !self.eat("0") && !self.digits() {
            return None;
        }
        if self.eat(".") && !self.digits() {
            return None;
        }
        if self.eat("e") || self.eat("E") {
            if !self.eat("+") {
                self.eat("-");
            }
            if !self.digits() {
                return None;
            }
        }
        Some(&start[..start.len() - self.0.len()])
    }

    fn items(&mut self, end: &str, mut item: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        self.ws();
        if self.eat(end) {
            return Some(());
        }
        loop {
            item(self)?;
            self.ws();
            if self.eat(end) {
                return Some(());
            }
            if !self.eat(",") {
                return None;
            }
            self.ws();
        }
    }

    fn object(&mut self, depth: usize) -> Option<Object<'a>> {
        if depth > 128 || !self.eat("{") {
            return None;
        }
        let mut obj = BTreeMap::new();
        self.items("}", |p| {
            let key = p.string(true)?;
            p.ws();
            if !p.eat(":") {
                return None;
            }
            obj.insert(key, p.value(depth + 1)?);
            Some(())
        })?;
        Some(obj)
    }

    fn value(&mut self, depth: usize) -> Option<Value<'a>> {
        self.ws();
        Some(match self.peek()? {
            b'{' => Value::Object(self.object(depth)?),
            b'[' if depth <= 128 => {
                self.eat("[");
                let mut items = Vec::new();
                self.items("]", |p| {
                    items.push(p.value(depth + 1)?);
                    Some(())
                })?;
                Value::Array(items)
            }
            b'"' => Value::String(match self.string(false)? {
                Cow::Borrowed(bytes) => Cow::Borrowed(str::from_utf8(bytes).ok()?),
                Cow::Owned(bytes) => Cow::Owned(String::from_utf8(bytes).ok()?),
            }),
            b'-' | b'0'..=b'9' => Value::Number(self.number()?),
            b't' if self.eat("true") => Value::Bool(true),
            b'f' if self.eat("false") => Value::Bool(false),
            b'n' if self.eat("null") => Value::Null,
            _ => return None,
        })
    }
}

// Pruv: entry::parse_line
fn parse_line(line: &[u8]) -> Option<Object<'_>> {
    let mut parser = Parser(str::from_utf8(line).ok()?);
    parser.ws();
    let object = parser.object(1)?;
    parser.ws();
    parser.0.is_empty().then_some(object)
}

// ---------------- entry classification ----------------
// Pruv: entry::string_field, entry::string_id, access::string_is
fn get_str<'a>(m: &'a Object<'_>, k: &str) -> Option<&'a str> {
    match m.get(k.as_bytes()) {
        Some(Value::String(s)) => Some(s.as_ref()),
        _ => None,
    }
}

// Pruv: entry::is_true, entry::true_of
fn is_true(m: &Object<'_>, k: &str) -> bool {
    matches!(m.get(k.as_bytes()), Some(Value::Bool(true)))
}

// Pruv: entry::task_origin, entry::task_origin_of
fn task_origin(e: &Object<'_>) -> bool {
    matches!(e.get(b"origin".as_slice()), Some(Value::Object(o)) if get_str(o, "kind") == Some("task-notification"))
}

// Pruv: entry::string_content, text::engine_line
fn is_human_text(e: &Object<'_>, text: &str) -> bool {
    let t = es_trim(text);
    if t.is_empty() {
        return false;
    }
    if t == "[Request interrupted by user]" || t == "[Request interrupted by user for tool use]" {
        return false;
    }
    !(t.starts_with("<task-notification>")
        && t.ends_with("</task-notification>")
        && t.chars().count() >= 39
        && task_origin(e))
}

// Pruv: entry::human_text, entry::user_text, entry::user_text_of, entry::array_content, entry::block_of
fn human_text<'a>(e: &'a Object<'_>) -> Option<Cow<'a, str>> {
    if get_str(e, "type") != Some("user") || is_true(e, "isMeta") || is_true(e, "isSidechain") {
        return None;
    }
    let content = match e.get(b"message".as_slice()) {
        Some(Value::Object(m)) => m.get(b"content".as_slice()),
        _ => None,
    };
    match content {
        Some(Value::String(s)) => {
            if is_human_text(e, s) {
                Some(Cow::Borrowed(s.as_ref()))
            } else {
                None
            }
        }
        Some(Value::Array(blocks)) => {
            let mut first: Option<&str> = None;
            let mut joined = String::new();
            let mut count = 0usize;
            for b in blocks {
                let Value::Object(o) = b else { continue };
                match get_str(o, "type") {
                    Some("tool_result") => return None,
                    Some("text") => {
                        if let Some(Value::String(t)) = o.get(b"text".as_slice()) {
                            match count {
                                0 => first = Some(t.as_ref()),
                                1 => {
                                    joined.push_str(first.unwrap_or(""));
                                    joined.push_str("\n\n");
                                    joined.push_str(t);
                                }
                                _ => {
                                    joined.push_str("\n\n");
                                    joined.push_str(t);
                                }
                            }
                            count += 1;
                        }
                    }
                    _ => {}
                }
            }
            let text = match count {
                0 => return None,
                1 => Cow::Borrowed(first.unwrap_or("")),
                _ => Cow::Owned(joined),
            };
            if is_human_text(e, &text) {
                Some(text)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// First of `keys` holding a string that is not blank.
// Pruv: entry::pick_title, entry::title_key
fn pick<'a>(e: &'a Object<'_>, k1: &str, k2: &str) -> Option<&'a str> {
    for k in [k1, k2] {
        if let Some(s) = get_str(e, k) {
            if !es_trim(s).is_empty() {
                return Some(s);
            }
        }
    }
    None
}

// ---------------- turn collector ----------------
// Pruv: turns slot table sl[8*s .. 8*s+7] + arena buf (row VL)
struct Draft {
    prompt: Vec<u8>,
    reply: Vec<u8>,
    used: usize,
    tools: i64,
    created: Stamp,
    completed: Stamp,
    clipped: bool,
}

// Pruv: turns::new_state / new_slots / new_buffer (st, sl, buf; row VL)
struct Collector {
    drafts: VecDeque<Draft>, // only the newest MAXTURNS can ever be kept
    begun: usize,
    active: bool,
    lost: bool,
}

impl Collector {
    fn new() -> Self {
        Collector { drafts: VecDeque::new(), begun: 0, active: false, lost: false }
    }

    // Pruv: turns::begin, turns::write_begin
    fn begin(&mut self, prompt: &str, created: Stamp) {
        let raw = prompt.as_bytes();
        let mut d = if self.drafts.len() >= MAXTURNS {
            let mut d = self.drafts.pop_front().unwrap();
            d.prompt.clear();
            d.reply.clear();
            d
        } else {
            Draft {
                prompt: Vec::new(),
                reply: Vec::new(),
                used: 0,
                tools: 0,
                created: None,
                completed: None,
                clipped: false,
            }
        };
        d.prompt.extend_from_slice(&raw[..clip_len(raw, LIM)]);
        d.used = 0;
        d.tools = 0;
        d.created = created;
        d.completed = None;
        d.clipped = raw.len() > LIM || self.lost;
        self.lost = false;
        self.active = true;
        self.begun += 1;
        self.drafts.push_back(d);
    }

    // Pruv: turns::skip_user
    fn skip_user(&mut self) {
        self.active = false;
        self.lost = true;
    }

    // Pruv: turns::answer, turns::write_answer, turns::write_piece
    fn answer(&mut self, text: &str, at: Stamp) {
        if !self.active {
            return;
        }
        let Some(d) = self.drafts.back_mut() else { return };
        if at.is_some() {
            d.completed = at;
        }
        let raw = text.as_bytes();
        if raw.is_empty() || d.used >= LIM {
            return;
        }
        let sep = if d.reply.is_empty() { 0 } else { 2 };
        let room = LIM - d.used;
        if sep + raw.len() <= room {
            d.reply.extend_from_slice(&b"\n\n"[..sep]);
            d.reply.extend_from_slice(raw);
            d.used += sep + raw.len();
        } else {
            if room <= sep {
                d.reply.extend_from_slice(&b"\n\n"[..room]);
            } else {
                let k = clip_len(raw, room - sep);
                d.reply.extend_from_slice(&b"\n\n"[..sep]);
                d.reply.extend_from_slice(&raw[..k]);
            }
            d.used = LIM;
            d.clipped = true;
        }
    }

    // Pruv: turns::tool
    fn tool(&mut self) {
        if self.active {
            if let Some(d) = self.drafts.back_mut() {
                d.tools += 1;
            }
        }
    }

    /// Number of newest drafts kept (<= MAXTURNS, <= TOTAL bytes).
    // Pruv: turns::plan, turns::plan_end
    fn kept(&self) -> usize {
        let (mut n, mut total) = (0usize, 0usize);
        for d in self.drafts.iter().rev() {
            if n >= MAXTURNS {
                break;
            }
            let size = d.prompt.len() + d.reply.len();
            if total + size > TOTAL {
                break;
            }
            total += size;
            n += 1;
        }
        n
    }
}

#[derive(Default)]
// Pruv: import best / best_rank locals
struct RankedTitle {
    rank: u8,
    text: String,
}

impl RankedTitle {
    // Pruv: entry::title_of, entry::title_for_type, entry::ranked (rank check before pick)
    fn update(&mut self, e: &Object<'_>) {
        let (rank, key) = match get_str(e, "type") {
            Some("custom-title") => (3, "customTitle"),
            Some("ai-title") => (2, "aiTitle"),
            Some("summary") => (1, "summary"),
            _ => return,
        };
        if rank >= self.rank {
            if let Some(t) = pick(e, key, "title") {
                self.rank = rank;
                self.text.clear();
                self.text.push_str(t);
            }
        }
    }

    // Pruv: text::one_line(best, 120) in import::tail_info / read_window
    fn finish(&self) -> Option<String> {
        (self.rank > 0).then(|| one_line(&self.text, 120))
    }
}

#[derive(Default)]
struct State {
    title: RankedTitle,
    model: Vec<u8>,
}

// Pruv: import::consume_entry, import::answer_blocks
fn consume(e: &Object<'_>, col: &mut Collector, st: &mut State) {
    let ty = get_str(e, "type");
    st.title.update(e);
    if let Some(prompt) = human_text(e) {
        col.begin(&prompt, iso(e.get(b"timestamp".as_slice())));
        return;
    }
    if ty != Some("assistant") || is_true(e, "isSidechain") {
        return;
    }
    let msg = match e.get(b"message".as_slice()) {
        Some(Value::Object(m)) => Some(m),
        _ => None,
    };
    let model = msg.and_then(|m| get_str(m, "model"));
    if model == Some(SYNTHETIC) && !is_true(e, "isApiErrorMessage") {
        return;
    }
    if let Some(m) = model {
        if !m.is_empty() && m != SYNTHETIC {
            st.model.clear();
            st.model.extend_from_slice(m.as_bytes());
        }
    }
    match msg.and_then(|m| m.get(b"content".as_slice())) {
        Some(Value::String(s)) => col.answer(s, iso(e.get(b"timestamp".as_slice()))),
        Some(Value::Array(blocks)) => {
            let at = iso(e.get(b"timestamp".as_slice()));
            for b in blocks {
                let Value::Object(o) = b else { continue };
                match get_str(o, "type") {
                    Some("text") => {
                        if let Some(Value::String(t)) = o.get(b"text".as_slice()) {
                            col.answer(t, at);
                        }
                    }
                    Some("tool_use") => col.tool(),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

// ---------------- packet ----------------
// Pruv: boundary::write_bytes, import::put_str
fn put(text: &mut Vec<u8>, b: &[u8]) -> [i64; 2] {
    let start = text.len() as i64;
    text.extend_from_slice(b);
    [start, b.len() as i64]
}

fn put_optional(text: &mut Vec<u8>, value: Option<&[u8]>) -> [i64; 2] {
    value.map_or([-1, 0], |b| put(text, b))
}

// Pruv: import::head_result, import::tail_result, boundary::new_packet
fn edge_packet(mode: i64, fields: [Option<&[u8]>; 4]) -> Vec<i64> {
    let mut text = Vec::new();
    let refs = fields.map(|field| put_optional(&mut text, field));
    let mut out = Vec::with_capacity(18 + text.len());
    out.extend_from_slice(&[0, 14, 0, text.len() as i64, 1, mode, 0, 0]);
    out.extend(refs.into_iter().flatten());
    out.extend_from_slice(&[-1, 0]);
    out.extend(text.into_iter().map(i64::from));
    out
}

// Pruv: framing::edge_next (pkgs lines::jsonl_next)
fn edge_lines(data: &[u8], include_final: bool) -> impl Iterator<Item = Object<'_>> {
    data.split_inclusive(|&b| b == b'\n').filter_map(move |line| {
        if let Some(raw) = line.strip_suffix(b"\n") {
            parse_line(raw.strip_suffix(b"\r").unwrap_or(raw))
        } else if include_final {
            parse_line(line)
        } else {
            None
        }
    })
}

// Pruv: framing::skip_first
fn skip_first(data: &[u8], cut_before: bool) -> &[u8] {
    if cut_before {
        data.iter().position(|&b| b == b'\n').map_or(&[], |i| &data[i + 1..])
    } else {
        data
    }
}

// Pruv: import::read_head
pub fn head_packet(data: &[u8], cut_after: i64) -> Vec<i64> {
    if data.len() > EDGE {
        return vec![1, 0];
    }
    let (mut title, mut cwd, mut entrypoint) = (None, None, None);
    for e in edge_lines(data, cut_after == 0) {
        if cwd.is_none() {
            cwd = get_str(&e, "cwd").map(str::to_owned);
        }
        if entrypoint.is_none() {
            entrypoint = get_str(&e, "entrypoint").map(str::to_owned);
        }
        if title.is_none() {
            title = human_text(&e).map(|t| one_line(&t, 120));
        }
        if title.is_some() && cwd.is_some() && entrypoint.is_some() {
            break;
        }
    }
    edge_packet(0, [title, cwd, entrypoint, None].each_ref().map(|s| s.as_deref().map(str::as_bytes)))
}

// Pruv: import::tail_info
pub fn tail_packet(data: &[u8], cut_before: i64) -> Vec<i64> {
    if data.len() > EDGE {
        return vec![1, 0];
    }
    let mut title = RankedTitle::default();
    let mut last = None;
    for e in edge_lines(skip_first(data, cut_before != 0), true) {
        if let Some(at) = iso(e.get(b"timestamp".as_slice())) {
            last = Some(at);
        }
        title.update(&e);
    }
    let title = title.finish();
    edge_packet(1, [title.as_deref().map(str::as_bytes), None, None, last.as_ref().map(|s| s.as_slice())])
}

// Pruv: import::read_window, framing::window_next, turns::write_turns
pub fn window_packet(data: &[u8], cut_before: i64) -> Vec<i64> {
    if data.len() > WINDOW {
        return vec![1, 0];
    }
    let cut = cut_before != 0;
    let mut col = Collector::new();
    let mut st = State::default();

    let data = skip_first(data, cut);
    let mut pos = 0;
    while pos < data.len() {
        let rest = &data[pos..];
        match rest.iter().position(|&b| b == b'\n') {
            None => {
                if rest.len() > LINE && dropped_user(&rest[..DROP]) {
                    col.skip_user();
                }
                break;
            }
            Some(i) => {
                let raw = &rest[..i];
                if raw.len() > LINE {
                    if dropped_user(&raw[..DROP]) {
                        col.skip_user();
                    }
                } else {
                    let line = if raw.last() == Some(&b'\r') { &raw[..raw.len() - 1] } else { raw };
                    if let Some(e) = parse_line(line) {
                        consume(&e, &mut col, &mut st);
                    }
                }
                pos += i + 1;
            }
        }
    }

    let n = col.kept();
    let trimmed = n < col.begun;
    let title = st.title.finish();

    let mut text: Vec<u8> = Vec::new();
    let title_ref = put_optional(&mut text, title.as_deref().map(str::as_bytes));
    let model_ref = if st.model.is_empty() { [-1, 0] } else { put(&mut text, &st.model) };
    let mut turns: Vec<i64> = Vec::with_capacity(n * 10);
    for d in col.drafts.iter().skip(col.drafts.len() - n) {
        turns.extend_from_slice(&put(&mut text, &d.prompt));
        turns.extend_from_slice(&put(&mut text, &d.reply));
        for s in [&d.created, &d.completed] {
            match s {
                Some(b) => turns.extend_from_slice(&put(&mut text, b)),
                None => turns.extend_from_slice(&[-1, 0]),
            }
        }
        turns.push(d.tools);
        turns.push(d.clipped as i64);
    }

    let mut out: Vec<i64> = Vec::with_capacity(18 + turns.len() + text.len());
    out.extend_from_slice(&[0, 14, turns.len() as i64, text.len() as i64]);
    out.extend_from_slice(&[1, 2, (cut || trimmed) as i64, n as i64]);
    out.extend_from_slice(&title_ref);
    out.extend_from_slice(&[-1, 0, -1, 0, -1, 0]);
    out.extend_from_slice(&model_ref);
    out.extend_from_slice(&turns);
    out.extend(text.iter().map(|&b| b as i64));
    out
}

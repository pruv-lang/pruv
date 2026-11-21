#![forbid(unsafe_code)]

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

pub const WINDOW_BYTES: usize = 8 * 1024 * 1024;
const LINE_BYTES: usize = 1024 * 1024;
const DROP_HEAD_BYTES: usize = 8192;
const TEXT_BYTES: usize = 32768;
const TURN_LIMIT: usize = 200;
const TOTAL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Window {
    pub title: Option<String>,
    pub model: Option<String>,
    pub omitted_before: bool,
    pub turns: Vec<Turn>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Turn {
    pub prompt: String,
    pub reply: String,
    pub created_at: Option<String>,
    pub completed_at: Option<String>,
    pub tools: i64,
    pub clipped: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    InputTooLarge,
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window exceeds {WINDOW_BYTES} bytes")
    }
}

impl std::error::Error for Fail {}

type Object = Map<String, Value>;

struct Entry {
    uuid: Option<String>,
    parent: Option<String>,
    action: Action,
}

enum Action {
    User { parts: Vec<Value>, timestamp: Value },
    Assistant { parts: Vec<Value>, timestamp: Value, model: Option<String> },
    RoleWithoutContent,
    Other,
}

enum Slot {
    Entry(Entry),
    DroppedUser,
}

fn take_string(object: &mut Object, key: &str) -> Option<String> {
    match object.remove(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

fn take_parts(object: &mut Object) -> Vec<Value> {
    if let Some(Value::Object(mut message)) = object.remove("message") {
        if let Some(Value::Array(parts)) = message.remove("parts") {
            return parts;
        }
    }
    Vec::new()
}

fn visible_parts(parts: &[Value]) -> impl Iterator<Item = &Object> {
    parts.iter().filter_map(Value::as_object)
        .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
}

impl Entry {
    fn new(mut object: Object, title: &mut Option<String>) -> Self {
        let uuid = take_string(&mut object, "uuid");
        let parent = take_string(&mut object, "parentUuid");
        let role = take_string(&mut object, "type");
        let action = match role.as_deref() {
            Some("user")
                if object.get("provenance").is_none_or(|v| v.as_str() == Some("real_user")) =>
            {
                Action::User {
                    parts: take_parts(&mut object),
                    timestamp: object.remove("timestamp").unwrap_or(Value::Null),
                }
            }
            Some("assistant") => Action::Assistant {
                parts: take_parts(&mut object),
                timestamp: object.remove("timestamp").unwrap_or(Value::Null),
                model: take_string(&mut object, "model"),
            },
            Some("user" | "tool_result") => Action::RoleWithoutContent,
            _ => {
                if role.as_deref() == Some("system")
                    && object.get("subtype").and_then(Value::as_str) == Some("custom_title")
                {
                    if let Some(text) = object.get("systemPayload")
                        .and_then(|payload| payload.get("customTitle"))
                        .and_then(Value::as_str)
                        .filter(|text| !es_trim(text).is_empty())
                    {
                        *title = Some(one_line(text, 120));
                    }
                }
                Action::Other
            }
        };
        Self { uuid, parent, action }
    }
}

fn current_chain(slots: &[Slot]) -> (Option<HashSet<&str>>, bool) {
    let mut latest = HashMap::new();
    let (mut last_role, mut last_entry) = (None, None);
    for slot in slots {
        if let Slot::Entry(entry) = slot {
            if let Some(uuid) = entry.uuid.as_deref() {
                latest.insert(uuid, entry);
                last_entry = Some(entry);
                if !matches!(entry.action, Action::Other) {
                    last_role = Some(entry);
                }
            }
        }
    }

    let Some(mut entry) = last_role.or(last_entry) else {
        return (None, false);
    };
    let mut selected = HashSet::new();
    // Use the actual leaf's parent on the first hop and the newest entry per UUID thereafter.
    while selected.insert(entry.uuid.as_deref().expect("indexed UUID")) {
        let Some(parent) = entry.parent.as_deref() else {
            break;
        };
        let Some(next) = latest.get(parent) else {
            return (Some(selected), true);
        };
        entry = next;
    }
    (Some(selected), false)
}

#[derive(Default)]
struct Collector {
    turns: VecDeque<Turn>,
    attached: bool,
    next_clipped: bool,
    reply_full: bool,
    trimmed: bool,
}

impl Collector {
    fn begin(&mut self, mut prompt: String, created_at: Option<String>) {
        let clipped = prompt.len() > TEXT_BYTES || self.next_clipped;
        prompt.truncate(prefix_len(&prompt, TEXT_BYTES));
        if self.turns.len() == TURN_LIMIT {
            self.turns.pop_front();
            self.trimmed = true;
        }
        self.turns.push_back(Turn {
            prompt,
            reply: String::new(),
            created_at,
            completed_at: None,
            tools: 0,
            clipped,
        });
        self.attached = true;
        self.next_clipped = false;
        self.reply_full = false;
    }

    fn skip_user(&mut self) {
        self.attached = false;
        self.next_clipped = true;
    }

    fn answer(&mut self, text: &str, at: Option<&str>) {
        let turn = self.turns.back_mut().expect("attached turn");
        if let Some(at) = at {
            turn.completed_at = Some(at.to_owned());
        }
        if self.reply_full || text.is_empty() {
            return;
        }
        let mut room = TEXT_BYTES - turn.reply.len();
        if room == 0 {
            return;
        }
        if !turn.reply.is_empty() {
            let separator = &"\n\n"[..room.min(2)];
            turn.reply.push_str(separator);
            room -= separator.len();
        }
        turn.reply.push_str(&text[..prefix_len(text, room)]);
        if text.len() > room {
            turn.clipped = true;
            self.reply_full = true;
        }
    }

    fn finish(mut self) -> (Vec<Turn>, bool) {
        let mut bytes = 0;
        let keep = self.turns.iter().rev().take_while(|turn| {
            bytes += turn.prompt.len() + turn.reply.len();
            bytes <= TOTAL_BYTES
        }).count();
        let remove = self.turns.len() - keep;
        self.turns.drain(..remove);
        (self.turns.into(), self.trimmed || remove > 0)
    }
}

pub fn window_import(data: &[u8], cut_before: bool) -> Result<Window, Fail> {
    if data.len() > WINDOW_BYTES {
        return Err(Fail::InputTooLarge);
    }
    let mut lines = data.split_inclusive(|&byte| byte == b'\n');
    if cut_before {
        lines.next();
    }

    let mut slots = Vec::new();
    let mut title = None;
    let mut scratch = Vec::new();
    for raw in lines {
        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
        if line.len() > LINE_BYTES {
            if dropped_user(&line[..DROP_HEAD_BYTES]) {
                slots.push(Slot::DroppedUser);
            }
        } else if raw.last() == Some(&b'\n') {
            if let Some(object) = parse_line(line, &mut scratch) {
                slots.push(Slot::Entry(Entry::new(object, &mut title)));
            }
        }
    }

    let (chain, chain_cut) = current_chain(&slots);
    let mut collector = Collector::default();
    let (mut started, mut pending_drop) = (false, false);
    let mut model = None;
    for slot in &slots {
        let Slot::Entry(entry) = slot else {
            pending_drop = true;
            if started {
                collector.skip_user();
            }
            continue;
        };
        let selected = chain.as_ref().is_none_or(|uuids| {
            entry.uuid.as_deref().is_some_and(|uuid| uuids.contains(uuid))
        });
        if selected {
            if !started && pending_drop {
                collector.skip_user();
            }
            started = true;
            match &entry.action {
                Action::User { parts, timestamp } => {
                    let prompt = visible_parts(parts)
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    if !es_trim(&prompt).is_empty() {
                        collector.begin(prompt, iso(timestamp));
                    }
                }
                Action::Assistant { parts, timestamp, model: id } => {
                    if let Some(id) = id {
                        model = Some(id.clone());
                    }
                    if collector.attached {
                        let at = iso(timestamp);
                        for part in visible_parts(parts) {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                collector.answer(text, at.as_deref());
                            } else if part.get("functionCall").is_some_and(Value::is_object) {
                                collector.turns.back_mut().expect("attached turn").tools += 1;
                            }
                        }
                    }
                }
                Action::RoleWithoutContent | Action::Other => {}
            }
        }
        pending_drop = false;
    }
    let (turns, trimmed) = collector.finish();
    Ok(Window {
        title,
        model,
        omitted_before: cut_before || trimmed || chain_cut,
        turns,
    })
}

fn is_es_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

fn es_trim(s: &str) -> &str {
    s.trim_matches(is_es_ws)
}

/// Whitespace runs -> one space, trim, then cut to `mx` UTF-16 units on a scalar boundary.
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

fn prefix_len(s: &str, max: usize) -> usize {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

const SAFE: i64 = 9_007_199_254_740_991;
const MIN_MS: i64 = -62_167_219_200_000;
const MAX_MS: i64 = 253_402_300_799_999;

type Stamp = Option<[u8; 24]>;

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

fn iso_of_int(v: i64) -> Stamp {
    let ms = if v < 100_000_000_000 { v * 1000 } else { v };
    if (MIN_MS..=MAX_MS).contains(&ms) {
        Some(format_iso(ms))
    } else {
        None
    }
}

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

fn iso(value: &Value) -> Option<String> {
    let stamp = match value {
        Value::Number(n) => iso_number(&n.to_string()),
        Value::String(s) => iso_str(s),
        _ => None,
    }?;
    Some(String::from_utf8(stamp.to_vec()).expect("ASCII timestamp"))
}

/// Byte length of the ECMAScript whitespace character at `i` (0 if none).
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

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        let n = ws_len(b, i);
        if n == 0 {
            return i;
        }
        i += n;
    }
}

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

fn dropped_user(head: &[u8]) -> bool {
    key_colon_val(head, b"\"type\"", b"\"user\"")
        && key_colon_val(head, b"\"provenance\"", b"\"real_user\"")
}

fn hex4(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    let mut n = 0u32;
    for &c in &b[..4] {
        n = n * 16 + (c as char).to_digit(16)?;
    }
    Some(n)
}

/// Copies `src` into `out`, turning lone surrogate escapes into "�". Returns whether anything changed.
fn rewrite_lone_surrogates(src: &[u8], out: &mut Vec<u8>) -> bool {
    let mut changed = false;
    let mut in_str = false;
    let mut i = 0;
    out.reserve(src.len());
    while i < src.len() {
        let c = src[i];
        if !in_str {
            if c == b'"' {
                in_str = true;
            }
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = false;
                out.push(c);
                i += 1;
            }
            b'\\' => {
                if src.get(i + 1) == Some(&b'u') {
                    if let Some(n) = hex4(&src[i + 2..]) {
                        if (0xD800..0xDC00).contains(&n) {
                            let pair = src.get(i + 6) == Some(&b'\\')
                                && src.get(i + 7) == Some(&b'u')
                                && matches!(src.get(i + 8..).and_then(hex4), Some(m) if (0xDC00..0xE000).contains(&m));
                            if pair {
                                out.extend_from_slice(&src[i..i + 12]);
                                i += 12;
                            } else {
                                out.extend_from_slice(b"\\ufffd");
                                changed = true;
                                i += 6;
                            }
                        } else if (0xDC00..0xE000).contains(&n) {
                            out.extend_from_slice(b"\\ufffd");
                            changed = true;
                            i += 6;
                        } else {
                            out.extend_from_slice(&src[i..i + 6]);
                            i += 6;
                        }
                        continue;
                    }
                }
                out.push(c);
                if let Some(&d) = src.get(i + 1) {
                    out.push(d);
                }
                i += 2;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    changed
}

/// Container nesting deeper than `limit` (string aware; exact for valid JSON).
fn deeper_than(src: &[u8], limit: usize) -> bool {
    let (mut d, mut in_str, mut esc) = (0usize, false, false);
    for &c in src {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' | b'[' => {
                    d += 1;
                    if d > limit {
                        return true;
                    }
                }
                b'}' | b']' => d = d.saturating_sub(1),
                _ => {}
            }
        }
    }
    false
}

/// serde's recursion limit rejects 128 nested containers; the oracle allows exactly 128.
fn parse_deep(src: &[u8], err: &serde_json::Error) -> Option<Value> {
    if !err.to_string().contains("recursion limit") || deeper_than(src, 128) {
        return None;
    }
    let mut de = serde_json::Deserializer::from_slice(src);
    de.disable_recursion_limit();
    let mut it = de.into_iter::<Value>();
    let v = it.next()?.ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(v)
}

fn parse_slow(line: &[u8], first_err: serde_json::Error, scratch: &mut Vec<u8>) -> Option<Value> {
    scratch.clear();
    if line.windows(2).any(|w| w == b"\\u") && rewrite_lone_surrogates(line, scratch) {
        let src = &scratch[..];
        return match serde_json::from_slice::<Value>(src) {
            Ok(v) => Some(v),
            Err(e) => parse_deep(src, &e),
        };
    }
    parse_deep(line, &first_err)
}

fn parse_line(line: &[u8], scratch: &mut Vec<u8>) -> Option<Object> {
    match line.iter().find(|&&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
        Some(&b'{') => {}
        _ => return None,
    }
    // arbitrary_precision turns an object keyed by this literal into a number. Rename the key
    // (same length) so it stays an object; no lookup reads that key. A \u-escaped key is not covered.
    const NUMBER_KEY: &[u8] = b"\"$serde_json::private::Number\"";
    let renamed;
    let line = if line.windows(NUMBER_KEY.len()).any(|w| w == NUMBER_KEY) {
        let mut copy = line.to_vec();
        for i in 0..=copy.len() - NUMBER_KEY.len() {
            let rest = &copy[i + NUMBER_KEY.len()..];
            let key = rest.iter().find(|&&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r')) == Some(&b':');
            if key && copy[i..i + NUMBER_KEY.len()] == *NUMBER_KEY {
                copy[i + 1] = b'_';
            }
        }
        renamed = copy;
        &renamed[..]
    } else {
        line
    };
    let v = match serde_json::from_slice::<Value>(line) {
        Ok(v) => v,
        Err(e) => parse_slow(line, e, scratch)?,
    };
    match v {
        Value::Object(m) => Some(m),
        _ => None,
    }
}


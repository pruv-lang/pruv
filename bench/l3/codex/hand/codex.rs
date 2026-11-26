#![forbid(unsafe_code)]
// Hand baseline for the Rust-shape reset step 4.3f: the whole Codex import (list and read), written as an experienced
// Rust programmer writes it, std + serde_json only. One JSON request per stdin line, one JSON result line per request.
// Every function carries a `// Pruv: module::fn` line. Pruv helpers with no hand function of their own:
//   util::field/raw/to_bytes/elements/str_at/byte_buffer/grow_bytes/id_is/push_int -> serde_json::Value, Vec, kind_of
//   rpc::receive (line assembly) -> read_lines; folder::lone -> serde_json rejects lone surrogates before we see a string
//   read::stamp_ok/feed/span_ok/replay -> finish; item::stamp -> iso
//   plan::next/take_segment, root::codex_read_wire -> codex_plan, codex_read

use serde_json::Value;
use std::borrow::Cow;
use std::io::{self, BufRead, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

const MAX_PAGES: usize = 40;
const MAX_TURNS: usize = 200;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FIELD: usize = 32 * 1024;
const MAX_LIST: usize = 50;
const LINE_CAP: usize = 8 * 1024 * 1024;
const PLAN_ITEMS: usize = 30;
const PLAN_BLOB: usize = 18_000;
const PLAN_UNITS: usize = 200;
const HEX: &[u8; 16] = b"0123456789abcdef";
const REQ_INIT: &[u8] = b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"lair\",\"title\":\"Lair\",\"version\":\"0.0.1\"},\"capabilities\":{\"experimentalApi\":true,\"requestAttestation\":false}}}\n";
const NOTIFY_INITIALIZED: &[u8] = b"{\"method\":\"initialized\",\"params\":{}}\n";
const NH_HEAD: &[u8] = b"{\"id\":";
const NH_TAIL: &[u8] = b",\"error\":{\"code\":-32601,\"message\":\"not handled\"}}\n";

/// The error code a failed request prints: not-found, codex-unsupported, timeout, read-failed, engine-missing, internal.
type Fail = &'static str;

#[derive(Default)]
struct Turn {
    prompt: String,
    reply: String,
    replies: usize,
    tools: usize,
    created: Option<String>,
    completed: Option<String>,
    clipped: bool,
}

struct Plan {
    items: Vec<String>,
    more: usize,
    at: Option<String>,
}

/// Newest-first held turns, their admission cost, and the first (newest) plan met.
#[derive(Default)]
struct Held {
    turns: Vec<Turn>,
    bytes: usize,
    plan: Option<Plan>,
    plan_seen: bool,
}

struct Row<'a> {
    thread: &'a str,
    title: String,
    updated: Option<String>,
    path: Option<&'a str>,
    origin: Option<&'static str>,
}

// ---- text helpers ----

// Pruv: text::trim (ECMAScript whitespace)
fn is_space(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

// Pruv: text::trim
fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

// Pruv: text::is_blank
fn is_blank(s: &str) -> bool {
    s.chars().all(is_space)
}

// Pruv: text::clip_bytes (longest whole-scalar prefix of at most max bytes)
fn clip_bytes(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// Pruv: plan::clip_units (longest whole-scalar prefix of at most max UTF-16 units)
fn clip_units(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &s[..i];
        }
    }
    s
}

// Pruv: text::one_line
fn one_line(s: &str, max: usize) -> String {
    clip_units(&s.split(is_space).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "), max).to_owned()
}

// Pruv: util::units16
fn units16(s: &str) -> usize {
    s.encode_utf16().count()
}

// Pruv: util::space_in
fn space_at(b: &[u8]) -> usize {
    match b {
        [b' ' | 9..=13, ..] => 1,
        [0xC2, 0xA0, ..] => 2,
        [0xE1, 0x9A, 0x80, ..] | [0xE2, 0x80, 0x80..=0x8A | 0xA8 | 0xA9 | 0xAF, ..] | [0xE2, 0x81, 0x9F, ..] | [0xE3, 0x80, 0x80, ..] | [0xEF, 0xBB, 0xBF, ..] => 3,
        _ => 0,
    }
}

// Pruv: util::trim_span
fn trim_bytes(b: &[u8]) -> &[u8] {
    let (mut lo, mut hi, mut i) = (0, 0, 0);
    while i < b.len() {
        let k = space_at(&b[i..]);
        if k > 0 {
            if lo == i {
                lo += k;
            }
            i += k;
        } else {
            i += 1;
            hi = i;
        }
    }
    &b[lo..hi.max(lo)]
}

// Pruv: util::push_escaped (quote, backslash and C0 controls)
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

/// `prefix` then `s` as a JSON string.
fn put(out: &mut Vec<u8>, prefix: &str, s: &str) -> io::Result<()> {
    out.extend_from_slice(prefix.as_bytes());
    Ok(serde_json::to_writer(&mut *out, s)?)
}

// ---- time ----

// Pruv: time::canonical_iso
fn canonical_iso(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 24 || b[23] != b'Z' {
        return false;
    }
    for (i, &c) in b[..23].iter().enumerate() {
        let want = match i { 4 | 7 => b'-', 10 => b'T', 13 | 16 => b':', 19 => b'.', _ => 0 };
        if want != 0 { if c != want { return false; } } else if !c.is_ascii_digit() { return false; }
    }
    let num = |a: usize, n: usize| b[a..a + n].iter().fold(0u32, |v, &c| v * 10 + u32::from(c - b'0'));
    let (year, month, day) = (num(0, 4), num(5, 2), num(8, 2));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month { 2 if leap => 29, 2 => 28, 4 | 6 | 9 | 11 => 30, _ => 31 };
    (1..=12).contains(&month) && (1..=days).contains(&day) && num(11, 2) < 24 && num(14, 2) < 60 && num(17, 2) < 60
}

// Pruv: time::format_iso (civil-from-days)
fn format_iso(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

// Pruv: time::iso_of (integer seconds or milliseconds, digit strings, canonical ISO strings; all else absent)
fn iso(value: &Value) -> Option<String> {
    const SAFE: i64 = 9_007_199_254_740_991;
    let from_integer = |n: i64| {
        let ms = if n < 100_000_000_000 { n * 1000 } else { n };
        (-62_167_219_200_000..=253_402_300_799_999).contains(&ms).then(|| format_iso(ms))
    };
    match value {
        Value::Number(n) => n.as_i64().filter(|n| n.abs() <= SAFE).and_then(from_integer),
        Value::String(s) => {
            let t = trim(s);
            if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
                return t.parse::<i64>().ok().filter(|&n| n <= SAFE).and_then(from_integer);
            }
            canonical_iso(s).then(|| s.clone())
        }
        _ => None,
    }
}

// ---- rpc ----

// Pruv: rpc::child_args
fn child_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    ["app-server", "--stdio"].into_iter().chain(args.iter().copied()).collect()
}

struct Rpc {
    stdin: Option<ChildStdin>,
    rx: Receiver<Vec<u8>>,
    deadline: Instant,
    line: Vec<u8>,
}

enum Kind {
    Ignore,
    Server,
    Response,
}

// Pruv: rpc::receive (64 KiB reads, lines split at '\n', a line over the cap ends the stream: read-failed)
fn read_lines(mut stdout: ChildStdout, tx: SyncSender<Vec<u8>>) {
    let (mut chunk, mut pending) = (vec![0u8; 65_536], Vec::new());
    loop {
        let n = match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let mut rest = &chunk[..n];
        while let Some(k) = rest.iter().position(|&c| c == b'\n') {
            pending.extend_from_slice(&rest[..k]);
            if pending.len() > LINE_CAP || tx.send(std::mem::take(&mut pending)).is_err() {
                return;
            }
            rest = &rest[k + 1..];
        }
        pending.extend_from_slice(rest);
        if pending.len() > LINE_CAP {
            return;
        }
    }
}

// Pruv: rpc::start (spawn failure is engine-missing; one overall deadline replaces the timer slot)
fn start(binary: &str, args: &[&str], cwd: &str, ms: u64) -> Result<(Child, Rpc), Fail> {
    let mut command = Command::new(binary);
    command.args(child_args(args)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if !cwd.is_empty() {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|_| "engine-missing")?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("internal");
    };
    let (tx, rx) = sync_channel(16);
    std::thread::spawn(move || read_lines(stdout, tx));
    let deadline = Instant::now() + Duration::from_millis(ms);
    Ok((child, Rpc { stdin: Some(stdin), rx, deadline, line: Vec::new() }))
}

// Pruv: rpc::parse_line (invalid UTF-8 or bad JSON: the line is skipped)
fn parse_line(line: &[u8]) -> Option<Value> {
    serde_json::from_slice(trim_bytes(line)).ok()
}

// Pruv: rpc::kind_of (a server request has a string `method` and an `id` member; a response has the id token of `id`)
fn kind_of(v: &Value, id: u64) -> Kind {
    let Some(object) = v.as_object() else { return Kind::Ignore };
    match object.get("id") {
        Some(_) if object.get("method").map_or(false, Value::is_string) => Kind::Server,
        Some(Value::Number(n)) if n.as_u64() == Some(id) => Kind::Response,
        _ => Kind::Ignore,
    }
}

// Pruv: rpc::not_handled
fn not_handled(v: &Value) -> Vec<u8> {
    let id = v["id"].to_string();
    let mut b = Vec::with_capacity(NH_HEAD.len() + id.len() + NH_TAIL.len());
    b.extend_from_slice(NH_HEAD);
    b.extend_from_slice(id.as_bytes());
    b.extend_from_slice(NH_TAIL);
    b
}

impl Rpc {
    // Pruv: transport::send (a child whose stdin closed earlier is a timeout past the deadline, else read-failed)
    fn send(&mut self, data: &[u8]) -> Result<(), Fail> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(if Instant::now() >= self.deadline { "timeout" } else { "read-failed" });
        };
        if stdin.write_all(data).and_then(|()| stdin.flush()).is_err() {
            self.stdin = None;
        }
        Ok(())
    }

    // Pruv: rpc::step (one line into self.line, or timeout / closed)
    fn step(&mut self) -> Result<(), Fail> {
        match self.rx.recv_timeout(self.deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                self.line = line;
                Ok(())
            }
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
            Err(RecvTimeoutError::Disconnected) => Err("read-failed"),
        }
    }

    // Pruv: the pump loops of root::codex_list / codex_read_wire and rpc::open (at most 1,000,000 events)
    fn call(&mut self, id: u64, request: &[u8]) -> Result<Value, Fail> {
        self.send(request)?;
        for _ in 0..1_000_000 {
            self.step()?;
            let Some(v) = parse_line(&self.line) else { continue };
            match kind_of(&v, id) {
                Kind::Server => {
                    let _ = self.send(&not_handled(&v));
                }
                Kind::Response => return Ok(v),
                Kind::Ignore => {}
            }
        }
        Err("read-failed")
    }
}

// Pruv: rpc::open (initialize id 1, then the initialized notification)
fn open(rpc: &mut Rpc) -> Result<(), Fail> {
    if rpc.call(1, REQ_INIT)?["error"].is_object() {
        return Err("read-failed");
    }
    rpc.send(NOTIFY_INITIALIZED)
}

// Pruv: rpc::req_list
fn req_list(id: u64, folder: &str) -> Vec<u8> {
    const A: &[u8] = b",\"method\":\"thread/list\",\"params\":{\"cwd\":[\"";
    const Z: &[u8] = b"\"],\"limit\":50,\"sortKey\":\"updated_at\",\"useStateDbOnly\":true,\"sourceKinds\":[\"cli\",\"vscode\",\"exec\",\"appServer\"]}}\n";
    let mut b = Vec::with_capacity(8 + A.len() + 6 * folder.len() + Z.len());
    b.extend_from_slice(b"{\"id\":");
    b.extend_from_slice(id.to_string().as_bytes());
    b.extend_from_slice(A);
    push_escaped(&mut b, folder);
    b.extend_from_slice(Z);
    b
}

// Pruv: rpc::req_read
fn req_read(id: u64, thread: &str) -> Vec<u8> {
    const A: &[u8] = b",\"method\":\"thread/read\",\"params\":{\"threadId\":\"";
    const Z: &[u8] = b"\",\"includeTurns\":false}}\n";
    let mut b = Vec::with_capacity(8 + A.len() + 6 * thread.len() + Z.len());
    b.extend_from_slice(b"{\"id\":");
    b.extend_from_slice(id.to_string().as_bytes());
    b.extend_from_slice(A);
    push_escaped(&mut b, thread);
    b.extend_from_slice(Z);
    b
}

// Pruv: rpc::req_turns (the cursor is sent again as a JSON string)
fn req_turns(id: u64, thread: &str, cursor: &str) -> Vec<u8> {
    const A: &[u8] = b",\"method\":\"thread/turns/list\",\"params\":{\"threadId\":\"";
    const M: &[u8] = b"\",\"limit\":20,\"sortDirection\":\"desc\",\"itemsView\":\"summary\"";
    let mut b = Vec::with_capacity(8 + A.len() + 6 * thread.len() + M.len() + 14 + 6 * cursor.len() + 4);
    b.extend_from_slice(b"{\"id\":");
    b.extend_from_slice(id.to_string().as_bytes());
    b.extend_from_slice(A);
    push_escaped(&mut b, thread);
    b.extend_from_slice(M);
    if !cursor.is_empty() {
        b.extend_from_slice(b",\"cursor\":\"");
        push_escaped(&mut b, cursor);
        b.push(b'"');
    }
    b.extend_from_slice(b"}}\n");
    b
}

// Pruv: rpc::is_minus_32601 (JS `===` on the parsed binary64 number; Pruv decides it on the token digits)
fn is_minus_32601(code: &Value) -> bool {
    code.as_f64() == Some(-32601.0)
}

// Pruv: rpc::has_phrase (/not found|no rollout|unknown thread/i; a non-ASCII unit never matches an ASCII letter)
fn has_phrase(message: &str) -> bool {
    let hay: Vec<u8> = message.encode_utf16().take(300).map(|u| if u < 128 { (u as u8).to_ascii_lowercase() } else { 0xFF }).collect();
    ["not found", "no rollout", "unknown thread"].iter().any(|p| hay.windows(p.len()).any(|w| w == p.as_bytes()))
}

// Pruv: rpc::error_class (1 codex-unsupported, 2 not-found, 3 read-failed)
fn error_class(error: &Value) -> u8 {
    if is_minus_32601(&error["code"]) {
        1
    } else if error["message"].as_str().map_or(false, has_phrase) {
        2
    } else {
        3
    }
}

// Pruv: rpc::error_detail (first 300 units, non-ASCII shown as '?', "rejected" without a string message)
fn error_detail(error: &Value) -> String {
    match error["message"].as_str() {
        None => "rejected".to_owned(),
        Some(m) => m.encode_utf16().take(300).map(|u| if (32..127).contains(&u) { u as u8 as char } else { '?' }).collect(),
    }
}

/// The failure `code`, with the response's error detail on stderr (never part of the result).
fn fail_from(response: &Value, code: Fail) -> Fail {
    eprintln!("{code}: {}", error_detail(&response["error"]));
    code
}

// ---- names, folder, plan ----

// Pruv: names::valid_thread
fn valid_thread(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=128).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b[1..].iter().all(|c| c.is_ascii_alphanumeric() || matches!(*c, b'_' | b'-'))
}

// Pruv: folder::key_len (strip every trailing '/', an all-slash string stays)
fn folder_key(s: &str) -> &str {
    let t = s.trim_end_matches('/');
    if t.is_empty() { s } else { t }
}

// Pruv: folder::same (a cwd that is not a string, as one with a lone surrogate, is never the folder)
fn same_folder(cwd: &Value, folder: &str) -> bool {
    cwd.as_str().map_or(false, |c| folder_key(c) == folder_key(folder))
}

// Pruv: plan::marker_end (byte length of a leading '-', '*' or digits+'.' followed by whitespace, else 0)
fn marker_end(t: &str) -> usize {
    let b = t.as_bytes();
    let m = match b.first() {
        Some(b'-' | b'*') => 1,
        Some(c) if c.is_ascii_digit() => {
            let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
            if b.get(digits) != Some(&b'.') {
                return 0;
            }
            digits + 1
        }
        _ => return 0,
    };
    if t[m..].starts_with(is_space) { m } else { 0 }
}

// Pruv: plan::codex_plan (30 items, 18000 blob bytes, `more` counts the items past the 30th)
fn codex_plan(text: &str, at: Option<String>) -> Option<Plan> {
    let mut plan = Plan { items: Vec::new(), more: 0, at };
    let mut blob = 0;
    for line in text.split('\n') {
        let line = trim(line);
        let item = trim(&line[marker_end(line)..]);
        if item.is_empty() {
            continue;
        }
        let item = clip_units(item, PLAN_UNITS);
        if plan.items.len() == PLAN_ITEMS {
            plan.more += 1;
        } else if blob + item.len() <= PLAN_BLOB {
            blob += item.len();
            plan.items.push(item.to_owned());
        }
    }
    (!plan.items.is_empty()).then_some(plan)
}

// ---- list ----

// Pruv: list::title (name, else preview; each trimmed, one line, 120 units)
fn list_title(t: &Value) -> String {
    let text = |key: &str| t[key].as_str().map_or("", trim);
    let name = text("name");
    one_line(if name.is_empty() { text("preview") } else { name }, 120)
}

// Pruv: list::build (data, else threads; the first 50 entries; valid ids only)
fn list_rows(result: &Value) -> Vec<Row<'_>> {
    let data = result["data"].as_array().or_else(|| result["threads"].as_array());
    data.into_iter().flatten().take(MAX_LIST).filter_map(|t| {
        let thread = t["id"].as_str().filter(|id| valid_thread(id))?;
        Some(Row {
            thread,
            title: list_title(t),
            updated: iso(&t["updatedAt"]),
            path: t["path"].as_str().filter(|p| !p.is_empty()),
            origin: t["originator"].as_str().map(|o| if o.starts_with("lair") { "lair" } else { "other" }),
        })
    }).collect()
}

// Pruv: root::codex_list
fn codex_list(rpc: &mut Rpc, folder: &str) -> Result<Vec<u8>, Fail> {
    let response = rpc.call(2, &req_list(2, folder))?;
    if response["error"].is_object() {
        return Err(fail_from(&response, "codex-unsupported"));
    }
    render_rows(&list_rows(&response["result"])).map_err(|_| "internal")
}

fn render_rows(rows: &[Row]) -> io::Result<Vec<u8>> {
    let mut o = vec![b'['];
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            o.push(b',');
        }
        put(&mut o, "{\"thread\":", row.thread)?;
        put(&mut o, ",\"title\":", &row.title)?;
        if let Some(updated) = &row.updated {
            put(&mut o, ",\"updatedAt\":", updated)?;
        }
        if let Some(path) = row.path {
            put(&mut o, ",\"path\":", path)?;
        }
        if let Some(origin) = row.origin {
            write!(o, ",\"origin\":\"{origin}\"")?;
        }
        o.push(b'}');
    }
    o.push(b']');
    Ok(o)
}

// ---- read ----

// Pruv: item::item_text (`text` when a string, else the nonempty `text` parts of `content` joined by a blank line)
fn item_text(item: &Value) -> Cow<'_, str> {
    if let Some(text) = item["text"].as_str() {
        return Cow::Borrowed(text);
    }
    let Some(parts) = item["content"].as_array() else { return Cow::Borrowed("") };
    Cow::Owned(parts.iter().filter_map(|part| part["text"].as_str()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n\n"))
}

// Pruv: item::fold_reply (TurnCollector.answer: 32768-byte budget `used`, "\n\n" separators, whole-scalar clip)
fn fold_reply(turn: &mut Turn, used: &mut usize, text: &str) {
    if text.is_empty() || *used >= MAX_FIELD {
        return;
    }
    let room = MAX_FIELD - *used;
    let sep = if turn.reply.is_empty() { 0 } else { 2 };
    if sep + text.len() <= room {
        turn.reply.push_str(&"\n\n"[..sep]);
        turn.reply.push_str(text);
        *used += sep + text.len();
        return;
    }
    // over budget: the separator cut to what is left, else a whole-scalar prefix of the text
    let (sep, text) = if sep == 2 && room < 2 { (room, "") } else { (sep, clip_bytes(text, room - sep)) };
    turn.reply.push_str(&"\n\n"[..sep]);
    turn.reply.push_str(text);
    *used = MAX_FIELD;
    turn.clipped = true;
}

// Pruv: item::read_turn (the turn, the raw reply bytes for admission, the last nonblank plan text)
fn read_turn(raw: &Value) -> (Turn, usize, Option<&str>) {
    let mut turn = Turn::default();
    let (mut used, mut reply_bytes, mut plan) = (0, 0, None);
    for item in raw["items"].as_array().into_iter().flatten().filter(|item| item.is_object()) {
        match item["type"].as_str() {
            Some("userMessage") => {
                let text = item_text(item);
                if !is_blank(&text) {
                    if !turn.prompt.is_empty() {
                        turn.prompt.push_str("\n\n");
                    }
                    turn.prompt.push_str(&text);
                }
            }
            Some("agentMessage") => {
                let text = item_text(item);
                if item["phase"] != "commentary" && !is_blank(&text) {
                    turn.replies += 1;
                    reply_bytes += text.len();
                    fold_reply(&mut turn, &mut used, &text);
                }
            }
            Some("plan") => plan = item["text"].as_str().filter(|text| !is_blank(text)).or(plan),
            Some("reasoning") => {}
            _ => turn.tools += 1,
        }
    }
    turn.created = iso(&raw["startedAt"]);
    turn.completed = iso(&raw["completedAt"]);
    (turn, reply_bytes, plan)
}

// Pruv: read::absorb (newest first; true when the turn or byte budget is full)
fn absorb(held: &mut Held, data: &[Value]) -> bool {
    for raw in data.iter().filter(|raw| raw.is_object()) {
        let (turn, reply_bytes, plan_text) = read_turn(raw);
        if let Some(text) = plan_text.filter(|_| !held.plan_seen) {
            held.plan_seen = true;
            held.plan = codex_plan(text, turn.completed.as_ref().or(turn.created.as_ref()).cloned());
        }
        if turn.prompt.is_empty() && turn.replies == 0 {
            continue;
        }
        let size = turn.prompt.len() + reply_bytes.min(MAX_FIELD);
        if held.turns.len() == MAX_TURNS || held.bytes + size > MAX_BYTES {
            return true;
        }
        held.bytes += size;
        held.turns.push(turn);
    }
    false
}

// Pruv: read::finish (with read::replay: the collector's prompt clip and 2 MiB trim pass; turns written oldest first)
fn finish(mut held: Held, title: Option<String>, model: Option<&str>, more: bool) -> io::Result<Vec<u8>> {
    for turn in &mut held.turns {
        turn.clipped |= turn.prompt.len() > MAX_FIELD;
        let end = clip_bytes(&turn.prompt, MAX_FIELD).len();
        turn.prompt.truncate(end);
    }
    let mut total = 0;
    let kept = held.turns.iter().take(MAX_TURNS).take_while(|t| {
        total += t.prompt.len() + t.reply.len();
        total <= MAX_BYTES
    }).count();
    let trimmed = kept < held.turns.len();
    held.turns.truncate(kept);
    held.turns.reverse();

    let mut o = b"{\"turns\":[".to_vec();
    for (i, turn) in held.turns.iter().enumerate() {
        if i > 0 {
            o.push(b',');
        }
        put(&mut o, "{\"prompt\":", &turn.prompt)?;
        put(&mut o, ",\"reply\":", &turn.reply)?;
        write!(o, ",\"tools\":{}", turn.tools)?;
        if let Some(created) = &turn.created {
            put(&mut o, ",\"createdAt\":", created)?;
        }
        if let Some(completed) = turn.completed.as_ref().filter(|_| turn.replies > 0) {
            put(&mut o, ",\"completedAt\":", completed)?;
        }
        if turn.clipped {
            o.extend_from_slice(b",\"clipped\":true");
        }
        o.push(b'}');
    }
    write!(o, "],\"omittedBefore\":{}", more || trimmed)?;
    if let Some(model) = model {
        put(&mut o, ",\"model\":", model)?;
    }
    if let Some(title) = &title {
        put(&mut o, ",\"title\":", title)?;
    }
    if let Some(plan) = &held.plan {
        o.extend_from_slice(b",\"lastPlan\":{");
        if let Some(at) = &plan.at {
            put(&mut o, "\"at\":", at)?;
            o.push(b',');
        }
        o.extend_from_slice(b"\"items\":[");
        for (i, item) in plan.items.iter().enumerate() {
            if i > 0 {
                o.push(b',');
            }
            put(&mut o, "{\"text\":", item)?;
            o.push(b'}');
        }
        o.push(b']');
        if plan.more > 0 {
            write!(o, ",\"more\":{}", plan.more)?;
        }
        o.push(b'}');
    }
    o.push(b'}');
    Ok(o)
}

// Pruv: root::codex_read_wire (thread/read, folder check, then up to 40 pages of thread/turns/list)
fn codex_read(rpc: &mut Rpc, folder: &str, thread: &str) -> Result<Vec<u8>, Fail> {
    let response = rpc.call(2, &req_read(2, thread))?;
    if response["error"].is_object() {
        return Err(fail_from(&response, "not-found"));
    }
    let meta = &response["result"]["thread"];
    if !same_folder(&meta["cwd"], folder) {
        return Err("not-found");
    }
    let title = meta["name"].as_str().map(trim).filter(|name| !name.is_empty()).map(|name| one_line(name, 120));
    let model = meta["model"].as_str().or_else(|| meta["model"]["id"].as_str()).filter(|m| units16(m) <= 200).map(trim).filter(|m| !m.is_empty());

    let mut held = Held::default();
    let (mut cursor, mut more) = (String::new(), false);
    for page in 0..MAX_PAGES {
        let id = page as u64 + 3;
        let response = rpc.call(id, &req_turns(id, thread, &cursor))?;
        if response["error"].is_object() {
            let code = match error_class(&response["error"]) {
                1 => "codex-unsupported",
                2 => "not-found",
                _ => "read-failed",
            };
            return Err(fail_from(&response, code));
        }
        let result = &response["result"];
        let data = result["data"].as_array().map_or(&[] as &[Value], |d| d.as_slice());
        if absorb(&mut held, data) {
            more = true;
            break;
        }
        match result["nextCursor"].as_str() {
            Some(next) if !next.is_empty() && !data.is_empty() => {
                cursor = next.to_owned();
                more = page + 1 == MAX_PAGES;
            }
            _ => break,
        }
    }
    finish(held, title, model, more).map_err(|_| "internal")
}

// Pruv: root::codex_list / root::codex_read (dispatch, an invalid thread never spawns, cleanup of the child)
fn run(request: &Value) -> Result<Vec<u8>, Fail> {
    let text = |key: &str| request[key].as_str().unwrap_or("");
    let (list, folder, thread) = (request["mode"] == "list", text("folder"), text("thread"));
    if !list && !valid_thread(thread) {
        return Err("not-found");
    }
    let args: Vec<&str> = request["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let ms = request["timeout"].as_i64().unwrap_or(20_000).clamp(0, 86_400_000) as u64;
    let (mut child, mut rpc) = start(text("binary"), &args, text("cwd"), ms)?;
    let result = open(&mut rpc).and_then(|()| if list { codex_list(&mut rpc, folder) } else { codex_read(&mut rpc, folder, thread) });
    drop(rpc);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn main() -> io::Result<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        match run(&serde_json::from_str(&line?)?) {
            Ok(result) => out.write_all(&result)?,
            Err(code) => write!(out, "{{\"error\":\"{code}\"}}")?,
        }
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}

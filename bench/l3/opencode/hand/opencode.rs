#![forbid(unsafe_code)]
// Hand baseline for the Rust-shape reset step 4.3g: the whole OpenCode import (list, read, rows, dataDir, folderKey and the
// direct plan), written as an experienced Rust programmer writes it: std + serde_json + rusqlite (bundled). One call takes the
// decoded configuration and returns the result packet words; the packet is encoded exactly once, in `encode_result`.
// Every function carries a `// Pruv: module::fn` line. Pruv helpers with no hand function of their own:
//   host step wire, no hand counterpart (row SQ: no SQLite/fs host API in Pruv): packet::step_packet, ctl_*, patch_*, request,
//     event_*, root::out_ok/ctl_ok_at/route/opencode_step -> direct calls into rusqlite, std::fs and std::process
//   packet::ok_result/assemble_result/meta_new/error/build_error/simple_error/ok_value/ok_plan_absent -> encode_result
//   util::to_bytes/span_bytes/cfg_str/str_at/raw/elements/bank_*/fits/pool_*/format_i64/real_* -> Vec, String, serde_json::Value,
//     rusqlite typed parameters and ValueRef (a REAL parameter is f64, an integer cell is i64)
//   root::cfg_ok/cfg_head_ok/env_ok/keys_unique/str_clean/key_same -> Config::decode (HashSet for key uniqueness)
//   sql::p200..p208, list::p300.., export::p400.. phase functions -> select_rows, list, read_handoff, export (straight line code)
//   rows::decode/relocate -> select_rows; path::fail_with/ask/proceed/append_patch/ok_event/rows_ok -> `?` and Result
//   path::db_error/internal_at/toobig_at/columns_at -> db_fail, fail, Fail

use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::types::ValueRef;
use rusqlite::{params, params_from_iter, Connection, OpenFlags};

const MAX_TURNS: usize = 200;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FIELD: usize = 32 * 1024;
const MAX_MESSAGES: usize = 1_048_576;
const MAX_DOC: usize = 16_777_216;
const MAX_EXPORT: u64 = 33_554_432;
const MAX_CFG_TEXT: usize = 1_000_000_000;
const MAX_TEXT: usize = 40_000_000;
const PLAN_ITEMS: usize = 30;
const PLAN_POOL: usize = 24_000;
const PLAN_UNITS: usize = 200;
const SAFE: i64 = 9_007_199_254_740_991;
const BANK: usize = 2_147_483_600;
const MAGIC: i64 = 1_329_812_017;
const SELECT_BASE: &str = "SELECT s.id AS id, s.directory AS directory, s.title AS title, s.time_updated AS updated FROM session s WHERE s.parent_id IS NULL";

/// A failed request: the stage, the error class (1 engine, 2 raw os, 3 core, 4 other), the code (1 not-found, 2 read-failed,
/// 3 engine-missing, 4 timeout, 5 input-too-large, 6 internal), the native number, the raw code text and the detail text.
#[derive(Clone, Debug)]
struct Fail {
    stage: i64,
    class: i64,
    code: i64,
    native: i64,
    raw_code: String,
    detail: &'static str,
}

#[derive(Default)]
struct Turn {
    prompt: String,
    reply: String,
    created: Option<String>,
    completed: Option<String>,
    tools: i64,
    clipped: bool,
}

struct Draft {
    turn: Turn,
    used: usize,
}

#[derive(Default)]
struct Collector {
    drafts: Vec<Draft>,
    open: bool,
    model: Option<(String, String)>,
}

struct Item {
    text: String,
    status: i64,
}

struct Plan {
    items: Vec<Item>,
    more: i64,
    at: String,
}

struct Window {
    turns: Vec<Turn>,
    trimmed: bool,
    title: String,
    model: String,
    plan: Option<Plan>,
}

struct Row {
    id: String,
    directory: String,
    title: String,
    updated_ms: i64,
    messages: Option<i64>,
}

struct Handoff {
    fallback: String,
    dir: String,
    thread: String,
}

struct Config {
    mode: i64,
    export_ms: u64,
    term_ms: u64,
    kill_ms: u64,
    env_replace: bool,
    home: Option<String>,
    folder: Option<String>,
    thread: Option<String>,
    binary: Option<String>,
    temp_root: Option<String>,
    os_home: Option<String>,
    plan: Option<Vec<u8>>,
    env: Vec<(String, String)>,
}

struct Admitted {
    db: Option<String>,
}

enum Output {
    List { rows: Vec<Row>, now: i64 },
    Rows(Vec<Row>),
    Read(Window),
    Oversized(String),
    Value(Vec<u8>),
    Plan(Option<Plan>),
}

// ---- failures ----

// Pruv: packet::simple_error / path::internal_at / path::toobig_at / path::columns_at
fn fail(stage: i64, class: i64, code: i64) -> Fail {
    Fail { stage, class, code, native: 0, raw_code: String::new(), detail: "" }
}

fn with_detail(mut f: Fail, detail: &'static str) -> Fail {
    f.detail = detail;
    f
}

// Pruv: export::raw_error (a failed fs or process call: raw os error)
fn raw_error(stage: i64, e: &io::Error) -> Fail {
    Fail { native: i64::from(e.raw_os_error().unwrap_or(0)), raw_code: e.kind().to_string(), ..fail(stage, 2, 0) }
}

// Pruv: path::db_error (class 4 -> InputTooLarge, 5 -> InternalInvariant, else read-failed with the native code kept)
fn db_fail(stage: i64, e: &rusqlite::Error) -> Fail {
    let (native, raw) = match e.sqlite_error() {
        Some(s) => (i64::from(s.extended_code).max(0), format!("{:?}", s.code)),
        None => (0, String::new()),
    };
    Fail { native, raw_code: raw, ..fail(stage, 1, 2) }
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

// Pruv: time::format_iso (civil-from-days); the same arithmetic prints the extended-year form of format_ext
fn civil(ms: i64) -> (i64, i64, i64, i64) {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day, rem)
}

// Pruv: time::format_iso
fn format_iso(ms: i64) -> String {
    let (year, month, day, rem) = civil(ms);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

// Pruv: rows::format_ext (years 10000..275760: `+YYYYYY-...`)
fn format_ext(ms: i64) -> String {
    let (year, month, day, rem) = civil(ms);
    format!("+{year:06}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

// Pruv: time::from_integer (seconds below 1e11, else milliseconds; the valid millisecond range only)
fn from_integer(n: i64) -> Option<String> {
    let ms = if n < 100_000_000_000 { n * 1000 } else { n };
    (-62_167_219_200_000..=253_402_300_799_999).contains(&ms).then(|| format_iso(ms))
}

// Pruv: time::iso_of. Err(()) is a defect (an integer beyond 2^53-1: internal failure), Ok(None) is absent
fn iso(value: &Value) -> Result<Option<String>, ()> {
    match value {
        Value::Number(n) if n.is_i64() || n.is_u64() => match n.as_i64() {
            Some(v) if v.abs() <= SAFE => Ok(from_integer(v)),
            _ => Err(()),
        },
        Value::String(s) => {
            let t = trim(s);
            if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
                return Ok(t.parse::<i64>().ok().filter(|&n| n <= SAFE).and_then(from_integer));
            }
            Ok(canonical_iso(s).then(|| s.clone()))
        }
        _ => Ok(None),
    }
}

// Pruv: rows::db_iso (seconds or milliseconds as iso_of; beyond year 9999 the extended form of the raw millisecond number)
fn db_iso(v: i64) -> Option<String> {
    if v <= 0 {
        return None;
    }
    if let Some(t) = from_integer(v) {
        return Some(t);
    }
    (253_402_300_800_000..=8_640_000_000_000_000).contains(&v).then(|| format_ext(v))
}

// ---- paths ----

// Pruv: path::normalize (POSIX, as Node path.posix.normalize)
fn normalize(p: &str) -> String {
    if p.is_empty() {
        return ".".to_owned();
    }
    let (rooted, dir) = (p.starts_with('/'), p.ends_with('/'));
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if seg == ".." {
            match out.last() {
                Some(&last) if last != ".." => {
                    out.pop();
                }
                _ if rooted => {}
                _ => out.push(seg),
            }
        } else {
            out.push(seg);
        }
    }
    let mut s = out.join("/");
    if s.is_empty() && !rooted {
        s.push('.');
    }
    if dir && !s.is_empty() && !s.ends_with('/') {
        s.push('/');
    }
    if rooted {
        s.insert(0, '/');
    }
    s
}

// Pruv: path::join
fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => ".".to_owned(),
        (_, true) => normalize(a),
        (true, _) => normalize(b),
        _ => normalize(&format!("{a}/{b}")),
    }
}

// Pruv: path::dirname / path::dirname_bytes
fn dirname(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) => "/",
        Some(i) => t[..i].trim_end_matches('/'),
        None if p.starts_with('/') => "/",
        None => ".",
    }
}

// Pruv: path::contained (the file starts with the parent and a '/')
fn contained(parent: &str, file: &str) -> bool {
    !parent.is_empty() && file.strip_prefix(parent).map_or(false, |rest| parent.ends_with('/') || rest.starts_with('/'))
}

// Pruv: path::key_len / path::folder_key (strip every trailing '/', an all-slash string stays)
fn folder_key(s: &str) -> &str {
    let t = s.trim_end_matches('/');
    if t.is_empty() { s } else { t }
}

// Pruv: path::same_key
fn same_key(a: &str, b: &str) -> bool {
    folder_key(a) == folder_key(b)
}

// Pruv: path::env_value (the first pair with that key, "" when absent)
fn env_value<'a>(conf: &'a Config, name: &str) -> &'a str {
    conf.env.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str())
}

// Pruv: path::data_dir
fn data_dir(conf: &Config) -> String {
    let xdg = trim(env_value(conf, "XDG_DATA_HOME"));
    if xdg.starts_with('/') {
        return join(xdg, "opencode");
    }
    join(conf.os_home.as_deref().unwrap_or(""), ".local/share/opencode")
}

// Pruv: path::db_path (also path::db_none: OPENCODE_DB trimmed is ":memory:")
fn db_path(conf: &Config) -> Option<String> {
    let given = trim(env_value(conf, "OPENCODE_DB"));
    if given == ":memory:" {
        return None;
    }
    let home = conf.home.as_deref().unwrap_or("");
    Some(match given {
        "" => join(home, "opencode.db"),
        g if g.starts_with('/') => g.to_owned(),
        g => join(home, g),
    })
}

// Pruv: names::valid_thread / rows::valid_id
fn valid_thread(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=128).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b[1..].iter().all(|c| c.is_ascii_alphanumeric() || matches!(*c, b'_' | b'-'))
}

// ---- configuration ----

impl Config {
    // Pruv: root::cfg_ok / cfg_head_ok / env_ok / keys_unique / str_clean / key_same
    fn decode(cfg: &[i64], text: &[u8]) -> Option<Config> {
        let (len, n) = (cfg.len(), cfg.len().checked_sub(32)? / 4);
        if len > 2_147_483_647 || text.len() > 2_147_483_647 || (len - 32) % 4 != 0 || cfg[18] != n as i64 {
            return None;
        }
        let in_range = |v: i64| (1..=2_147_483_647).contains(&v);
        if cfg[0] != 1 || !(0..=5).contains(&cfg[1]) || !in_range(cfg[2]) || !in_range(cfg[3]) || !in_range(cfg[4]) || !(0..=1).contains(&cfg[5]) {
            return None;
        }
        // a ref is absent (-1, 0) or inside the text
        let span = |s: i64, l: i64| -> Option<Option<&[u8]>> {
            if s == -1 && l == 0 {
                return Some(None);
            }
            let (s, l) = (usize::try_from(s).ok()?, usize::try_from(l).ok()?);
            (l <= text.len().checked_sub(s)?).then(|| Some(&text[s..s + l]))
        };
        let string = |at: usize| Some(span(cfg[at], cfg[at + 1])?.map(|b| String::from_utf8_lossy(b).into_owned()));
        let (home, folder, thread, binary, temp_root, os_home) = (string(6)?, string(8)?, string(10)?, string(12)?, string(14)?, string(16)?);
        let plan = span(cfg[19], cfg[20])?.map(<[u8]>::to_vec);
        if (cfg[1] == 5) != plan.is_some() || cfg[21..32].iter().any(|&w| w != 0) {
            return None;
        }
        let mut env = Vec::with_capacity(n);
        let mut seen: HashSet<&[u8]> = HashSet::with_capacity(n);
        for i in 0..n {
            let b = 32 + 4 * i;
            let (Some(key), Some(value)) = (span(cfg[b], cfg[b + 1])?, span(cfg[b + 2], cfg[b + 3])?) else { return None };
            if key.is_empty() || key.iter().any(|&c| c == 0 || c == b'=') || value.contains(&0) || !seen.insert(key) {
                return None;
            }
            env.push((String::from_utf8_lossy(key).into_owned(), String::from_utf8_lossy(value).into_owned()));
        }
        Some(Config {
            mode: cfg[1],
            export_ms: cfg[2] as u64,
            term_ms: cfg[3] as u64,
            kill_ms: cfg[4] as u64,
            env_replace: cfg[5] == 1,
            home,
            folder,
            thread,
            binary,
            temp_root,
            os_home,
            plan,
            env,
        })
    }
}

// ---- admission ----

// Pruv: admit::p100 / p101 / p102 / p103 (any refusal is "no database": Ok(Admitted { db: None }))
fn admit(conf: &Config) -> Result<Admitted, Fail> {
    let absent = Ok(Admitted { db: None });
    if conf.mode == 1 && !valid_thread(conf.thread.as_deref().unwrap_or("")) {
        return Err(fail(0, 1, 1));
    }
    let Some(db) = db_path(conf) else { return absent };
    if db.len() > 1_000_000_000 {
        return Err(fail(1, 3, 5));
    }
    let Ok(parent) = fs::canonicalize(dirname(&db)) else { return absent };
    let parent = parent.to_string_lossy().into_owned();
    if parent.is_empty() {
        return absent;
    }
    match fs::symlink_metadata(&db) {
        Ok(m) if m.file_type().is_file() => {}
        _ => return absent,
    }
    let Ok(file) = fs::canonicalize(&db) else { return absent };
    if !contained(&parent, &file.to_string_lossy()) {
        return absent;
    }
    Ok(Admitted { db: Some(db) })
}

// ---- SQL ----

// Pruv: sql::has_name (the column names of a table; non-string names do not count)
fn columns(conn: &Connection, table: &str, stage: i64) -> Result<Vec<String>, Fail> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?)").map_err(|e| db_fail(stage, &e))?;
    let mut rows = stmt.query(params![table]).map_err(|e| db_fail(stage, &e))?;
    let mut names = Vec::new();
    while let Some(row) = rows.next().map_err(|e| db_fail(stage, &e))? {
        if let ValueRef::Text(t) = row.get_ref(0).map_err(|e| db_fail(stage, &e))? {
            names.push(String::from_utf8_lossy(t).into_owned());
        }
    }
    Ok(names)
}

// Pruv: sql::select_sql
fn select_sql(mode: i64, archived: bool) -> String {
    let mut sql = String::from(SELECT_BASE);
    if archived {
        sql.push_str(" AND s.time_archived IS NULL");
    }
    sql.push_str(" AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id)");
    match mode {
        1 => sql.push_str(" AND s.id = ?"),
        0 => sql.push_str(" AND s.directory IN (?, ?) ORDER BY s.time_updated DESC LIMIT ?"),
        _ => {}
    }
    sql
}

// Pruv: rows::clean_title (only TEXT counts; the default prefixes mean no title)
fn clean_title(s: Option<&str>) -> String {
    match s {
        Some(s) if !s.is_empty() && !s.starts_with("New session - ") && !s.starts_with("Child session - ") => one_line(s, 120),
        _ => String::new(),
    }
}

// Pruv: rows::updated_number (INTEGER, integral REAL, digit TEXT within +-(2^53-1); anything else 0)
fn updated_number(v: ValueRef) -> i64 {
    match v {
        ValueRef::Integer(n) => n,
        ValueRef::Real(f) if f.is_finite() && f.fract() == 0.0 && f.abs() <= SAFE as f64 => f as i64,
        ValueRef::Text(t) => {
            let Ok(s) = std::str::from_utf8(t) else { return 0 };
            let s = trim(s);
            let (neg, digits) = match s.as_bytes().first() {
                Some(b'-') => (true, &s[1..]),
                Some(b'+') => (false, &s[1..]),
                _ => (false, s),
            };
            if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
                return 0;
            }
            match digits.parse::<i64>() {
                Ok(n) if n <= SAFE => if neg { -n } else { n },
                _ => 0,
            }
        }
        _ => 0,
    }
}

fn cell_text(c: ValueRef<'_>) -> Option<std::borrow::Cow<'_, str>> {
    match c {
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t)),
        _ => None,
    }
}

// Pruv: sql::p200..p208 + rows::decode (schema checks, the typed selection, the kept rows in SQL order)
fn select_rows(conn: &Connection, mode: i64, filter: &str) -> Result<Vec<Row>, Fail> {
    let session = columns(conn, "session", 5)?;
    if !["id", "parent_id", "directory", "title", "time_updated"].iter().all(|c| session.iter().any(|n| n == c)) {
        return Err(with_detail(fail(5, 1, 2), "columns"));
    }
    if !columns(conn, "message", 6)?.iter().any(|n| n == "session_id") {
        return Err(with_detail(fail(6, 1, 2), "columns"));
    }
    if mode != 2 && filter.len() > 100_000_000 {
        return Err(fail(7, 3, 5));
    }
    let sql = select_sql(mode, session.iter().any(|n| n == "time_archived"));
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    match mode {
        1 => args.push(filter.to_owned().into()),
        0 => {
            args.push(filter.to_owned().into());
            args.push(format!("{filter}/").into());
            args.push(50.0f64.into());
        }
        _ => {}
    }
    let sel = |e: rusqlite::Error| db_fail(7, &e);
    let mut stmt = conn.prepare(&sql).map_err(sel)?;
    let mut rows = stmt.query(params_from_iter(args)).map_err(sel)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(sel)? {
        let cells = [row.get_ref(0).map_err(sel)?, row.get_ref(1).map_err(sel)?, row.get_ref(2).map_err(sel)?, row.get_ref(3).map_err(sel)?];
        if cells.iter().any(|c| matches!(c, ValueRef::Integer(n) if n.abs() > SAFE)) {
            return Err(Fail { raw_code: "binding-range".to_owned(), ..fail(7, 1, 2) });
        }
        let (Some(id), Some(directory)) = (cell_text(cells[0]), cell_text(cells[1])) else { continue };
        if !valid_thread(&id) || directory.is_empty() {
            continue;
        }
        out.push(Row { id: id.into_owned(), directory: directory.into_owned(), title: clean_title(cell_text(cells[2]).as_deref()), updated_ms: updated_number(cells[3]), messages: None });
    }
    Ok(out)
}

// Pruv: list::p300.. (the message count of each row; a count that is not a safe nonnegative integer is absent)
fn list(conn: &Connection, rows: &mut [Row]) -> Result<(), Fail> {
    let mut stmt = conn.prepare("SELECT COUNT(*) AS n FROM message WHERE session_id = ?").map_err(|e| db_fail(8, &e))?;
    for row in rows {
        let count = |e: rusqlite::Error| db_fail(9, &e);
        let mut found = stmt.query(params![row.id]).map_err(count)?;
        row.messages = match found.next().map_err(count)? {
            Some(r) => match r.get_ref(0).map_err(count)? {
                ValueRef::Integer(n) if (0..=SAFE).contains(&n) => Some(n),
                _ => None,
            },
            None => None,
        };
    }
    Ok(())
}

// Pruv: list::clock (one wall-clock sample after the DB is closed; a time outside 0..2^53-1 is internal)
fn clock() -> Result<i64, Fail> {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| Fail { raw_code: "clock".to_owned(), ..fail(12, 2, 0) })?;
    match i64::try_from(d.as_millis()) {
        Ok(ms) if (0..=SAFE).contains(&ms) => Ok(ms),
        _ => Err(fail(12, 3, 6)),
    }
}

// Pruv: sql::p200 / p201 / p390 (open read-only by the original path; a saved error is returned after the close)
fn read_db(conf: &Config, db: &str) -> Result<Vec<Row>, Fail> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| db_fail(4, &e))?;
    let filter = if conf.mode == 1 { conf.thread.as_deref().unwrap_or("") } else { folder_key(conf.folder.as_deref().unwrap_or("")) };
    let mut result = select_rows(&conn, conf.mode, filter);
    if let (0, Ok(rows)) = (conf.mode, result.as_mut()) {
        if let Err(f) = list(&conn, rows) {
            result = Err(f);
        }
    }
    let _ = conn.close();
    result
}

// Pruv: list::read_handoff (the first row must be in the folder; the binary and the temp root must exist)
fn read_handoff(conf: &Config, rows: &[Row]) -> Result<Handoff, Fail> {
    let Some(first) = rows.first() else { return Err(fail(11, 1, 1)) };
    if !same_key(&first.directory, conf.folder.as_deref().unwrap_or("")) {
        return Err(fail(11, 1, 1));
    }
    if conf.binary.as_deref().map_or(true, str::is_empty) || conf.temp_root.as_deref().map_or(true, str::is_empty) {
        return Err(fail(11, 1, 3));
    }
    Ok(Handoff { fallback: first.title.clone(), dir: first.directory.clone(), thread: first.id.clone() })
}

// ---- export ----

// Pruv: export::on_mkdtemp (join_prefix, then a fresh 0700 directory)
fn make_temp(root: &str) -> Result<String, Fail> {
    let prefix = join(root, "opencode-");
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for i in 0..100u128 {
        let dir = format!("{prefix}{:x}", seed.wrapping_add(i * 7919) ^ u128::from(std::process::id()));
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(raw_error(13, &e)),
        }
    }
    Err(raw_error(13, &last))
}

// Pruv: export::on_signal (TERM or KILL to the child's process group; signal errors are ignored)
fn signal(pid: u32, name: &str) {
    let _ = Command::new("kill").arg(format!("-{name}")).arg(format!("-{pid}")).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

// Pruv: export::on_spawn / on_clock / on_deadline / on_close / on_error (the budget is never restarted; TERM, then KILL, then give up)
fn run_child(conf: &Config, handoff: &Handoff, out: &File) -> Result<(), Fail> {
    let stdout = out.try_clone().map_err(|e| raw_error(15, &e))?;
    let mut cmd = Command::new(conf.binary.as_deref().unwrap_or(""));
    cmd.args(["--pure", "export", &handoff.thread]).current_dir(&handoff.dir).stdin(Stdio::null()).stdout(stdout).stderr(Stdio::null()).process_group(0);
    if conf.env_replace {
        cmd.env_clear();
    }
    cmd.envs(conf.env.iter().map(|(k, v)| (k, v)));
    let mut child = cmd.spawn().map_err(|_| fail(15, 1, 3))?;
    let pid = child.id();
    let mut deadline = Instant::now() + Duration::from_millis(conf.export_ms);
    let mut expired = 0;
    loop {
        match child.try_wait() {
            Err(_) => return Err(fail(16, 1, 3)),
            Ok(Some(status)) => {
                if expired > 0 {
                    return Err(fail(16, 1, 4));
                }
                if status.success() {
                    return Ok(());
                }
                return Err(Fail { native: i64::from(status.code().unwrap_or(0)), ..fail(16, 1, 2) });
            }
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            expired += 1;
            match expired {
                1 => {
                    signal(pid, "TERM");
                    deadline = Instant::now() + Duration::from_millis(conf.term_ms);
                }
                2 => {
                    signal(pid, "KILL");
                    deadline = Instant::now() + Duration::from_millis(conf.kill_ms);
                }
                _ => return Err(fail(16, 1, 4)),
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Pruv: export::p400..p423 + on_output / on_stat / on_read / on_cleanup (None is the oversized export, no read)
fn export(conf: &Config, handoff: &Handoff) -> Result<Option<Vec<u8>>, Fail> {
    let dir = make_temp(conf.temp_root.as_deref().unwrap_or(""))?;
    let path = Path::new(&dir).join("export.json");
    let result = (|| {
        let out = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| raw_error(14, &e))?;
        let ran = run_child(conf, handoff, &out);
        // the output close: a failure replaces the pending result
        out.sync_all().map_err(|e| raw_error(20, &e))?;
        drop(out);
        ran?;
        let size = fs::metadata(&path).map_err(|e| raw_error(21, &e))?.len();
        if size > MAX_EXPORT {
            return Ok(None);
        }
        fs::read(&path).map(Some).map_err(|e| raw_error(22, &e))
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

// ---- entry: the collector ----

// Pruv: entry::stamp
fn stamp<'a>(info: &'a Value, key: &str) -> &'a Value {
    &info["time"][key]
}

// Pruv: entry::text_of (nonblank text parts joined by a blank line; users skip synthetic and ignored parts)
fn text_of(parts: &[Value], skip_marked: bool) -> String {
    let mut out = String::new();
    let mut first = true;
    for part in parts {
        let Some(text) = part["text"].as_str().filter(|_| part["type"] == "text") else { continue };
        if is_blank(text) || (skip_marked && (part["synthetic"] == true || part["ignored"] == true)) {
            continue;
        }
        if !first {
            out.push_str("\n\n");
        }
        first = false;
        out.push_str(text);
        if out.len() > MAX_TEXT {
            break;
        }
    }
    out
}

// Pruv: entry::count_tools
fn count_tools(parts: &[Value]) -> i64 {
    parts.iter().filter(|p| p.is_object() && p["type"] == "tool").count() as i64
}

// Pruv: turns::begin (the prompt clipped to 32768 bytes on a scalar boundary)
fn begin(c: &mut Collector, prompt: &str, at: Option<String>) {
    let turn = Turn { clipped: prompt.len() > MAX_FIELD, prompt: clip_bytes(prompt, MAX_FIELD).to_owned(), created: at, ..Turn::default() };
    c.drafts.push(Draft { turn, used: 0 });
}

// Pruv: turns::answer (the reply budget `used`, "\n\n" separators, whole-scalar clip)
fn answer(c: &mut Collector, text: &str, at: Option<String>) {
    let Some(Draft { turn, used }) = c.drafts.last_mut() else { return };
    if at.is_some() {
        turn.completed = at;
    }
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
    let (sep, text) = if sep == 2 && room < 2 { (room, "") } else { (sep, clip_bytes(text, room - sep)) };
    turn.reply.push_str(&"\n\n"[..sep]);
    turn.reply.push_str(text);
    *used = MAX_FIELD;
    turn.clipped = true;
}

// Pruv: turns::tool
fn tool(c: &mut Collector, n: i64) {
    if let Some(d) = c.drafts.last_mut() {
        d.turn.tools = (d.turn.tools + n).min(1_000_000_000_000_000);
    }
}

// Pruv: entry::feed_user
fn feed_user(c: &mut Collector, info: &Value, parts: &[Value]) -> Result<(), ()> {
    let text = text_of(parts, true);
    if is_blank(&text) {
        c.open = false;
        return Ok(());
    }
    begin(c, &text, iso(stamp(info, "created"))?);
    c.open = true;
    Ok(())
}

// Pruv: entry::feed_assistant
fn feed_assistant(c: &mut Collector, info: &Value, parts: &[Value]) -> Result<(), ()> {
    if info["summary"] == true {
        return Ok(());
    }
    if let (Some(p), Some(m)) = (info["providerID"].as_str().filter(|s| !s.is_empty()), info["modelID"].as_str().filter(|s| !s.is_empty())) {
        c.model = Some((p.to_owned(), m.to_owned()));
    }
    if !c.open {
        return Ok(());
    }
    answer(c, &text_of(parts, false), iso(stamp(info, "completed"))?);
    tool(c, count_tools(parts));
    Ok(())
}

// Pruv: entry::feed (info must be an object and parts an array; an unknown role changes nothing)
fn feed(c: &mut Collector, msg: &Value) -> Result<(), ()> {
    let (info, Some(parts)) = (&msg["info"], msg["parts"].as_array()) else { return Err(()) };
    if !info.is_object() {
        return Err(());
    }
    match info["role"].as_str() {
        Some("user") => feed_user(c, info, parts),
        Some("assistant") => feed_assistant(c, info, parts),
        _ => Ok(()),
    }
}

// Pruv: turns::finish (the last 200 drafts, the newest suffix within 2 MiB, written oldest first)
fn finish(c: Collector) -> (Vec<Turn>, bool) {
    let mut turns: Vec<Turn> = c.drafts.into_iter().map(|d| d.turn).collect();
    let begun = turns.len();
    turns.reverse();
    let mut total = 0;
    let kept = turns.iter().take(MAX_TURNS).take_while(|t| {
        total += t.prompt.len() + t.reply.len();
        total <= MAX_BYTES
    }).count();
    turns.truncate(kept);
    turns.reverse();
    (turns, kept < begun)
}

// ---- plan ----

// Pruv: plan::status_code
fn status_code(s: &Value) -> i64 {
    match s.as_str() {
        Some("pending") => 1,
        Some("in_progress") => 2,
        Some("completed") => 3,
        _ => 0,
    }
}

// Pruv: plan::fit (trimmed nonblank contents; the first 30 stored, `more` counts the rest)
fn fit(todos: &[Value], at: String) -> Option<Plan> {
    let (mut items, mut count, mut pool) = (Vec::new(), 0usize, 0usize);
    for todo in todos.iter().filter(|t| t.is_object()) {
        let Some(content) = todo["content"].as_str().map(trim).filter(|t| !t.is_empty()) else { continue };
        count += 1;
        if items.len() < PLAN_ITEMS {
            let text = clip_units(content, PLAN_UNITS);
            if pool + text.len() <= PLAN_POOL {
                pool += text.len();
                items.push(Item { text: text.to_owned(), status: status_code(&todo["status"]) });
            }
        }
    }
    (!items.is_empty()).then(|| Plan { items, more: count.saturating_sub(PLAN_ITEMS) as i64, at })
}

// Pruv: plan::candidate (a todowrite tool part whose state is not an error and carries input.todos)
fn candidate(part: &Value) -> Option<&Vec<Value>> {
    let state = part.get("state").filter(|s| s.is_object())?;
    if part["type"] != "tool" || part["tool"] != "todowrite" || state["status"] == "error" || !state["input"].is_object() {
        return None;
    }
    state["input"]["todos"].as_array()
}

// Pruv: plan::find_todos (parts last first; the first candidate ends the search)
fn find_todos(parts: &[Value]) -> Option<&Vec<Value>> {
    parts.iter().rev().find_map(candidate)
}

// Pruv: plan::select (kind 0 is the export messages, kind 1 the direct input); Err is the status: 1 shape, 2 internal
fn select(arr: &Value, kind: u8) -> Result<Option<Plan>, u8> {
    let bad = if kind == 1 { 1 } else { 2 };
    let Some(messages) = arr.as_array() else { return Err(bad) };
    if messages.len() > MAX_MESSAGES {
        return Err(2);
    }
    if kind == 1 && !messages.iter().all(|m| m.is_object() && m["parts"].is_array()) {
        return Err(1);
    }
    for m in messages.iter().rev() {
        let (true, Some(parts)) = (m.is_object(), m["parts"].as_array()) else { return Err(bad) };
        let mut at = String::new();
        if kind == 0 {
            let info = &m["info"];
            if !info.is_object() {
                return Err(2);
            }
            if info["role"] != "assistant" || info["summary"] == true {
                continue;
            }
            let Some(todos) = find_todos(parts) else { continue };
            at = match iso(stamp(info, "completed")).map_err(|_| 2u8)? {
                Some(t) => t,
                None => iso(stamp(info, "created")).map_err(|_| 2u8)?.unwrap_or_default(),
            };
            return Ok(fit(todos, at));
        }
        let Some(todos) = find_todos(parts) else { continue };
        if let Some(s) = m["at"].as_str() {
            at = s.to_owned();
        }
        return Ok(fit(todos, at));
    }
    Ok(None)
}

// ---- convert ----

// Pruv: convert::model ("providerID/modelID" of the last eligible pair)
fn model(c: &Collector) -> String {
    c.model.as_ref().map_or_else(String::new, |(p, m)| format!("{p}/{m}"))
}

// Pruv: convert::decode (the size, internal and json failures of the parse)
fn parse(bytes: &[u8]) -> Result<Value, Fail> {
    let stage = 24;
    if bytes.len() > MAX_DOC {
        return Err(fail(stage, 3, 5));
    }
    serde_json::from_slice(bytes).map_err(|e| {
        if e.to_string().contains("recursion limit") {
            fail(stage, 3, 5)
        } else {
            with_detail(fail(stage, 1, 2), "json")
        }
    })
}

// Pruv: convert::convert_doc / convert::assemble (the whole export document to the window)
fn convert(doc: &[u8], fallback: &str) -> Result<Window, Fail> {
    let root = parse(doc)?;
    let (Some(_), Some(messages)) = (root.as_object(), root["messages"].as_array()) else { return Err(with_detail(fail(25, 1, 2), "shape")) };
    if messages.len() > MAX_MESSAGES {
        return Err(fail(26, 3, 6));
    }
    let mut collector = Collector::default();
    for m in messages {
        if !m.is_object() || !m["info"].is_object() || !m["parts"].is_array() {
            return Err(with_detail(fail(25, 1, 2), "shape"));
        }
        feed(&mut collector, m).map_err(|()| fail(26, 3, 6))?;
    }
    let mut title = clean_title(root["info"]["title"].as_str());
    if title.is_empty() {
        title = fallback.to_owned();
    }
    let model = model(&collector);
    let plan = select(&root["messages"], 0).map_err(|_| fail(27, 3, 6))?;
    let (turns, trimmed) = finish(collector);
    Ok(Window { turns, trimmed, title, model, plan })
}

// Pruv: plan::direct (mode 5: the plan input is a JSON array of messages)
fn direct(conf: &Config) -> Result<Output, Fail> {
    let bytes = conf.plan.as_deref().unwrap_or(&[]);
    let value = parse(bytes)?;
    match select(&value, 1) {
        Ok(plan) => Ok(Output::Plan(plan)),
        Err(1) => Err(with_detail(fail(25, 1, 2), "shape")),
        Err(_) => Err(fail(27, 3, 6)),
    }
}

// ---- result packet ----

struct Packet {
    head: [i64; 40],
    body: Vec<i64>,
    pool: Vec<u8>,
}

// Pruv: packet::meta_new
fn meta_new(mode: i64) -> [i64; 40] {
    let mut h = [0i64; 40];
    h[0] = MAGIC;
    h[1] = 1;
    h[3] = mode;
    for at in [10, 12, 14, 16, 23, 25, 27] {
        h[at] = -1;
    }
    h
}

/// Append `s` to the pool and return its (start, len) ref.
fn pool_str(pool: &mut Vec<u8>, s: &str) -> (i64, i64) {
    let at = pool.len() as i64;
    pool.extend_from_slice(s.as_bytes());
    (at, s.len() as i64)
}

fn put_ref(body: &mut Vec<i64>, r: (i64, i64)) {
    body.extend_from_slice(&[r.0, r.1]);
}

// Pruv: packet::assemble_result (the plan: at ref, more, and the 3-word items after the rows)
fn put_plan(p: &mut Packet, plan: &Plan) -> Vec<i64> {
    p.head[9] = 1;
    p.head[18] = plan.more;
    if !plan.at.is_empty() {
        let r = pool_str(&mut p.pool, &plan.at);
        p.head[16] = r.0;
        p.head[17] = r.1;
    }
    let mut items = Vec::with_capacity(3 * plan.items.len());
    for item in &plan.items {
        let (start, len) = pool_str(&mut p.pool, &item.text);
        items.extend_from_slice(&[start, len, item.status]);
    }
    items
}

fn encode_list(rows: &[Row], now: i64, p: &mut Packet) {
    p.head[4] = rows.len() as i64;
    p.head[5] = 10;
    p.body.reserve(10 * rows.len());
    for row in rows {
        put_ref(&mut p.body, pool_str(&mut p.pool, &row.id));
        put_ref(&mut p.body, pool_str(&mut p.pool, &row.title));
        match db_iso(row.updated_ms) {
            Some(t) => put_ref(&mut p.body, pool_str(&mut p.pool, &t)),
            None => put_ref(&mut p.body, (-1, 0)),
        }
        let open = row.updated_ms > 0 && now - row.updated_ms < 120_000;
        p.body.extend_from_slice(&[i64::from(row.messages.is_some()), row.messages.unwrap_or(0), i64::from(open), 0]);
    }
}

fn encode_rows(rows: &[Row], p: &mut Packet) {
    p.head[4] = rows.len() as i64;
    p.head[5] = 7;
    p.body.reserve(7 * rows.len());
    for row in rows {
        put_ref(&mut p.body, pool_str(&mut p.pool, &row.id));
        put_ref(&mut p.body, pool_str(&mut p.pool, &row.directory));
        put_ref(&mut p.body, pool_str(&mut p.pool, &row.title));
        p.body.push(row.updated_ms);
    }
}

fn encode_read(w: &Window, p: &mut Packet) -> Vec<i64> {
    p.head[4] = w.turns.len() as i64;
    p.head[5] = 10;
    p.head[8] = i64::from(w.trimmed);
    p.body.reserve(10 * w.turns.len());
    for t in &w.turns {
        put_ref(&mut p.body, pool_str(&mut p.pool, &t.prompt));
        put_ref(&mut p.body, pool_str(&mut p.pool, &t.reply));
        for at in [&t.created, &t.completed] {
            put_ref(&mut p.body, at.as_deref().map_or((-1, 0), |s| pool_str(&mut p.pool, s)));
        }
        p.body.extend_from_slice(&[t.tools, i64::from(t.clipped)]);
    }
    if !w.title.is_empty() {
        let r = pool_str(&mut p.pool, &w.title);
        (p.head[10], p.head[11]) = r;
    }
    if !w.model.is_empty() {
        let r = pool_str(&mut p.pool, &w.model);
        (p.head[12], p.head[13]) = r;
    }
    w.plan.as_ref().map_or_else(Vec::new, |plan| put_plan(p, plan))
}

// Pruv: packet::error / build_error (the raw code, the name and the detail share the pool)
fn encode_error(mode: i64, f: &Fail) -> Vec<i64> {
    let bad = !(0..=28).contains(&f.stage) || !(1..=4).contains(&f.class) || !(0..=6).contains(&f.code) || f.native < 0 || f.raw_code.len() > 10_000_000;
    let (stage, class, code) = if bad { (28, 3, 6) } else { (f.stage, f.class, f.code) };
    let (raw, detail) = if bad { ("", "") } else { (f.raw_code.as_str(), f.detail) };
    let mut h = meta_new(mode);
    h[2] = 1;
    h[7] = (raw.len() + detail.len()) as i64;
    h[19] = stage;
    h[20] = class;
    h[21] = code;
    h[22] = if bad { 0 } else { f.native };
    if !raw.is_empty() {
        (h[23], h[24]) = (0, raw.len() as i64);
    }
    if !detail.is_empty() {
        (h[27], h[28]) = (raw.len() as i64, detail.len() as i64);
    }
    let mut out = Vec::with_capacity(40 + raw.len() + detail.len());
    out.extend_from_slice(&h);
    out.extend(raw.bytes().chain(detail.bytes()).map(i64::from));
    out
}

// Pruv: packet::ok_result / ok_value / ok_plan_absent / oversized (the one place that writes the packet words)
fn encode_result(mode: i64, result: Result<Output, Fail>) -> Vec<i64> {
    let output = match result {
        Ok(o) => o,
        Err(f) => return encode_error(mode, &f),
    };
    let mut p = Packet { head: meta_new(mode), body: Vec::new(), pool: Vec::new() };
    let items = match &output {
        Output::List { rows, now } => {
            encode_list(rows, *now, &mut p);
            Vec::new()
        }
        Output::Rows(rows) => {
            encode_rows(rows, &mut p);
            Vec::new()
        }
        Output::Read(w) => encode_read(w, &mut p),
        Output::Oversized(title) => {
            p.head[5] = 10;
            p.head[8] = 1;
            if !title.is_empty() {
                let r = pool_str(&mut p.pool, title);
                (p.head[10], p.head[11]) = r;
            }
            Vec::new()
        }
        Output::Value(bytes) => {
            p.pool.extend_from_slice(bytes);
            (p.head[14], p.head[15]) = (0, bytes.len() as i64);
            Vec::new()
        }
        Output::Plan(plan) => plan.as_ref().map_or_else(Vec::new, |plan| put_plan(&mut p, plan)),
    };
    p.body.extend_from_slice(&items);
    let total = 40 + p.body.len() + p.pool.len();
    if p.body.len() + p.pool.len() > BANK {
        return encode_error(mode, &fail(0, 3, 5));
    }
    p.head[6] = (items.len() / 3) as i64;
    p.head[7] = p.pool.len() as i64;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&p.head);
    out.extend_from_slice(&p.body);
    out.extend(p.pool.iter().map(|&b| i64::from(b)));
    out
}

// ---- entry point ----

// Pruv: root::begin / root::opencode_step (modes 0..2: admit, read the rows, then list or export and convert)
fn import(conf: &Config) -> Result<Output, Fail> {
    let Some(db) = admit(conf)?.db else {
        return match conf.mode {
            0 => Ok(Output::List { rows: Vec::new(), now: clock()? }),
            1 => Err(fail(11, 1, 1)),
            _ => Ok(Output::Rows(Vec::new())),
        };
    };
    let rows = read_db(conf, &db)?;
    match conf.mode {
        0 => Ok(Output::List { rows, now: clock()? }),
        2 => Ok(Output::Rows(rows)),
        _ => {
            let handoff = read_handoff(conf, &rows)?;
            match export(conf, &handoff)? {
                Some(doc) => convert(&doc, &handoff.fallback).map(Output::Read),
                None => Ok(Output::Oversized(handoff.fallback)),
            }
        }
    }
}

// Pruv: path::utility (modes 3 and 4) and the mode dispatch of root::begin
pub fn run(cfg: &[i64], text: &[u8]) -> Vec<i64> {
    let safe_mode = cfg.get(1).copied().filter(|m| (0..=5).contains(m)).unwrap_or(0);
    let Some(conf) = Config::decode(cfg, text) else { return encode_result(safe_mode, Err(fail(0, 3, 6))) };
    if text.len() > MAX_CFG_TEXT {
        return encode_result(conf.mode, Err(fail(0, 3, 5)));
    }
    let result = match conf.mode {
        0..=2 => import(&conf),
        3 => Ok(Output::Value(data_dir(&conf).into_bytes())),
        4 => Ok(Output::Value(folder_key(conf.folder.as_deref().unwrap_or("")).as_bytes().to_vec())),
        _ => direct(&conf),
    };
    encode_result(conf.mode, result)
}

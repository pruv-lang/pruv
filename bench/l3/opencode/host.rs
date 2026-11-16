// OpenCode import primitive host (L3-7, DESIGN.md sections 2, 3, 5, 6). Included by host/src/lib.rs as crate `opencode_host`.
// Only plain `//` comments here (include! forbids inner doc comments).
//
// The host makes no import decisions: it builds cfg, calls the generated entries (through the `Sol` trait), applies
// delta patches to the six opaque banks, performs exactly the one request of an action-0 step, delivers its completion,
// and renders the final result packet as one JSON line. No JSON parsing of exports, no SQL building, no row filtering.
//
// Opcodes (request word 1): 1 realpath, 2 lstat, 3 db_open_readonly, 4 db_query (flags 0 prepare, 1 all, 2 get),
// 5 db_close, 6 mkdtemp, 7 open (wx 0600), 8 spawn (flags 0 stdout = the open file, 1 drained pipe, mutant only),
// 9 observe, 10 kill, 11 close, 12 stat, 13 read, 14 rm, 15 clock_now.
//
// Stdin: one JSON case config per line (fields: mode folder thread sourceHome homedir env{mode,values} binary tempRoot nowMs
// rpcMs schedule planJson|plan trace artifactDir). All paths are already absolute (the runner expands tokens).
// Stdout: one JSON envelope per case line. Stderr: JSONL {"trace":{v,seq,event,id,op,stage,handle,args,result,error}}.
// Exit 3 with {"status":"host-protocol-error","detail":...} on any protocol violation (HarnessError).
// `--sqlite-version` prints {"sqliteVersion":...} from a separate in-memory connection and exits.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::IntoRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAGIC_STEP: i64 = 0x4f435331;
const MAGIC_RESULT: i64 = 0x4f435231;
const CTL_WORDS: usize = 64;
const HEADER: usize = 76;
const MAX_BANK: i64 = 1 << 31;
const MAX_SAFE: i64 = 9_007_199_254_740_991;
const STEP_GUARD: u64 = 50_000_000;

// ---------------------------------------------------------------------------------------------------------------
// public surface

pub struct Banks {
    pub query_words: Vec<i64>,
    pub query_bytes: Vec<u8>,
    pub rows: Vec<i64>,
    pub text: Vec<u8>,
    pub document: Vec<u8>,
    pub saved: Vec<i64>,
}

// bank numbers: 0 query words, 1 query bytes, 2 rows, 3 text, 4 document, 5 saved result words
impl Banks {
    fn new() -> Banks {
        Banks { query_words: Vec::new(), query_bytes: Vec::new(), rows: Vec::new(), text: Vec::new(), document: Vec::new(), saved: Vec::new() }
    }
    fn is_words(b: usize) -> bool {
        b == 0 || b == 2 || b == 5
    }
    fn len(&self, b: usize) -> usize {
        match b {
            0 => self.query_words.len(),
            1 => self.query_bytes.len(),
            2 => self.rows.len(),
            3 => self.text.len(),
            4 => self.document.len(),
            _ => self.saved.len(),
        }
    }
    fn words_mut(&mut self, b: usize) -> &mut Vec<i64> {
        match b {
            0 => &mut self.query_words,
            2 => &mut self.rows,
            _ => &mut self.saved,
        }
    }
    fn bytes_mut(&mut self, b: usize) -> &mut Vec<u8> {
        match b {
            1 => &mut self.query_bytes,
            3 => &mut self.text,
            _ => &mut self.document,
        }
    }
    fn resize(&mut self, b: usize, n: usize) {
        if Banks::is_words(b) {
            let v = self.words_mut(b);
            if n > v.len() && v.try_reserve_exact(n - v.len()).is_err() {
                die("bank resize: out of memory");
            }
            v.resize(n, 0);
        } else {
            let v = self.bytes_mut(b);
            if n > v.len() && v.try_reserve_exact(n - v.len()).is_err() {
                die("bank resize: out of memory");
            }
            v.resize(n, 0);
        }
    }
}

// The generated module as seen by the host; harness.rs implements it with the `sol` functions.
pub trait Sol {
    fn begin(&self, cfg: &[i64], cfg_text: &[u8]) -> Vec<i64>;
    fn step(&self, cfg: &[i64], cfg_text: &[u8], ctl: &[i64], banks: &Banks, event: &[i64], completion: &[u8]) -> Vec<i64>;
}

pub fn run<S: Sol>(sol: &S) {
    if std::env::args().any(|a| a == "--sqlite-version") {
        print_sqlite_version();
        return;
    }
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => die(&format!("stdin: {}", e)),
        };
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => die(&format!("config is not JSON: {}", e)),
        };
        let out = run_one(sol, &v);
        println!("{}", out);
        let _ = io::stdout().flush();
    }
}

fn print_sqlite_version() {
    let c = match Connection::open_in_memory() {
        Ok(c) => c,
        Err(e) => die(&format!("metadata connection: {}", e)),
    };
    let v: String = match c.query_row("SELECT sqlite_version()", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(e) => die(&format!("sqlite_version: {}", e)),
    };
    println!("{}", json!({"sqliteVersion": v}));
    let _ = io::stdout().flush();
}

fn die(msg: &str) -> ! {
    println!("{{\"status\":\"host-protocol-error\",\"detail\":{}}}", qs(msg));
    let _ = io::stdout().flush();
    std::process::exit(3);
}

fn qs(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    s
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return Err("odd hex length".to_string());
    }
    let val = |c: u8| -> Result<u8, String> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("bad hex digit".to_string()),
        }
    };
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push(val(b[i])? * 16 + val(b[i + 1])?);
        i += 2;
    }
    Ok(out)
}

fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

// ---------------------------------------------------------------------------------------------------------------
// error values

#[derive(Clone)]
struct ErrInfo {
    class: i64,
    native: i64,
    code: String,
    msg: String,
}

const ERRNOS: [(&str, i32); 28] = [
    ("EPERM", libc::EPERM),
    ("ENOENT", libc::ENOENT),
    ("ESRCH", libc::ESRCH),
    ("EINTR", libc::EINTR),
    ("EIO", libc::EIO),
    ("ENXIO", libc::ENXIO),
    ("E2BIG", libc::E2BIG),
    ("ENOEXEC", libc::ENOEXEC),
    ("EBADF", libc::EBADF),
    ("ECHILD", libc::ECHILD),
    ("EAGAIN", libc::EAGAIN),
    ("ENOMEM", libc::ENOMEM),
    ("EACCES", libc::EACCES),
    ("EBUSY", libc::EBUSY),
    ("EEXIST", libc::EEXIST),
    ("EXDEV", libc::EXDEV),
    ("ENOTDIR", libc::ENOTDIR),
    ("EISDIR", libc::EISDIR),
    ("EINVAL", libc::EINVAL),
    ("ENFILE", libc::ENFILE),
    ("EMFILE", libc::EMFILE),
    ("EFBIG", libc::EFBIG),
    ("ENOSPC", libc::ENOSPC),
    ("EROFS", libc::EROFS),
    ("EPIPE", libc::EPIPE),
    ("ENAMETOOLONG", libc::ENAMETOOLONG),
    ("ENOTEMPTY", libc::ENOTEMPTY),
    ("ELOOP", libc::ELOOP),
];

fn errno_of(name: &str) -> Option<i32> {
    ERRNOS.iter().find(|e| e.0 == name).map(|e| e.1)
}

fn errno_name(n: i32) -> String {
    match ERRNOS.iter().find(|e| e.1 == n) {
        Some(e) => e.0.to_string(),
        None => "UNKNOWN".to_string(),
    }
}

fn os_err(e: &io::Error) -> ErrInfo {
    let n = e.raw_os_error().unwrap_or(libc::EINVAL);
    ErrInfo { class: 1, native: n as i64, code: errno_name(n), msg: e.to_string() }
}

const SQLITE_NAMES: [(&str, i64); 26] = [
    ("SQLITE_ERROR", 1),
    ("SQLITE_INTERNAL", 2),
    ("SQLITE_PERM", 3),
    ("SQLITE_ABORT", 4),
    ("SQLITE_BUSY", 5),
    ("SQLITE_LOCKED", 6),
    ("SQLITE_NOMEM", 7),
    ("SQLITE_READONLY", 8),
    ("SQLITE_INTERRUPT", 9),
    ("SQLITE_IOERR", 10),
    ("SQLITE_CORRUPT", 11),
    ("SQLITE_NOTFOUND", 12),
    ("SQLITE_FULL", 13),
    ("SQLITE_CANTOPEN", 14),
    ("SQLITE_PROTOCOL", 15),
    ("SQLITE_EMPTY", 16),
    ("SQLITE_SCHEMA", 17),
    ("SQLITE_TOOBIG", 18),
    ("SQLITE_CONSTRAINT", 19),
    ("SQLITE_MISMATCH", 20),
    ("SQLITE_MISUSE", 21),
    ("SQLITE_NOLFS", 22),
    ("SQLITE_AUTH", 23),
    ("SQLITE_FORMAT", 24),
    ("SQLITE_RANGE", 25),
    ("SQLITE_NOTADB", 26),
];

fn sqlite_name(primary: i64) -> String {
    match SQLITE_NAMES.iter().find(|e| e.1 == primary) {
        Some(e) => e.0.to_string(),
        None => "SQLITE_ERROR".to_string(),
    }
}

fn sqlite_err(e: &rusqlite::Error) -> ErrInfo {
    match e {
        rusqlite::Error::SqliteFailure(f, msg) => {
            let ext = f.extended_code as i64;
            let native = if ext > 0 { ext } else { 1 };
            ErrInfo { class: 2, native, code: sqlite_name(native & 0xff), msg: msg.clone().unwrap_or_else(|| e.to_string()) }
        }
        _ => ErrInfo { class: 2, native: 1, code: "SQLITE_ERROR".to_string(), msg: e.to_string() },
    }
}

fn range_err() -> ErrInfo {
    ErrInfo { class: 3, native: 0, code: "ERR_OUT_OF_RANGE".to_string(), msg: "integer outside the safe Number range".to_string() }
}

fn capacity_err(what: &str) -> ErrInfo {
    ErrInfo { class: 4, native: 0, code: "InputTooLarge".to_string(), msg: what.to_string() }
}

// ---------------------------------------------------------------------------------------------------------------
// config

struct Cfg {
    words: Vec<i64>,
    text: Vec<u8>,
    trace: bool,
    now_ms: Option<i64>,
    artifact_dir: Option<String>,
}

fn opt_str(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        _ => die(&format!("config field {} must be a string", k)),
    }
}

fn put_ref(words: &mut [i64], text: &mut Vec<u8>, at: usize, s: &Option<String>) {
    match s {
        None => {
            words[at] = -1;
            words[at + 1] = 0;
        }
        Some(t) => {
            words[at] = text.len() as i64;
            words[at + 1] = t.len() as i64;
            text.extend_from_slice(t.as_bytes());
        }
    }
}

fn parse_mode(v: &Value) -> i64 {
    match v.get("mode") {
        Some(Value::String(s)) => match s.as_str() {
            "list" => 0,
            "read" => 1,
            "rows" => 2,
            "dataDir" => 3,
            "folderKey" => 4,
            "plan" => 5,
            _ => die(&format!("unknown mode {}", s)),
        },
        Some(Value::Number(n)) => match n.as_i64() {
            Some(k) if (0..=5).contains(&k) => k,
            _ => die("mode number out of range"),
        },
        _ => die("config mode missing"),
    }
}

fn build_cfg(v: &Value) -> Cfg {
    let mode = parse_mode(v);
    let rpc = match v.get("rpcMs") {
        None | Some(Value::Null) => 20000,
        Some(x) => match x.as_i64() {
            Some(k) if k > 0 => k,
            _ => die("rpcMs must be a positive integer"),
        },
    };
    let now_ms = match v.get("nowMs") {
        None | Some(Value::Null) => None,
        Some(x) => match x.as_i64() {
            Some(k) => Some(k),
            None => die("nowMs must be an integer"),
        },
    };
    let trace = v.get("trace").and_then(|x| x.as_bool()).unwrap_or(true);
    let artifact_dir = opt_str(v, "artifactDir");
    let env_mode_full = match v.get("env").and_then(|e| e.get("mode")).and_then(|m| m.as_str()) {
        None | Some("inherited") => false,
        Some("full") => true,
        Some(m) => die(&format!("env.mode {} unknown", m)),
    };
    // effective environment table: inherited = snapshot of the host's own environment, full = only the supplied keys
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    if env_mode_full {
        if let Some(o) = v.get("env").and_then(|e| e.get("values")).and_then(|x| x.as_object()) {
            for (k, x) in o {
                match x.as_str() {
                    Some(s) => {
                        table.insert(k.clone(), s.to_string());
                    }
                    None => die(&format!("env value of {} must be a string", k)),
                }
            }
        }
    } else {
        for (k, x) in std::env::vars_os() {
            match (k.into_string(), x.into_string()) {
                (Ok(k), Ok(x)) => {
                    table.insert(k, x);
                }
                _ => die("inherited environment is not valid UTF-8"),
            }
        }
    }
    for (k, x) in &table {
        if k.is_empty() || k.contains('\0') || k.contains('=') || x.contains('\0') {
            die(&format!("environment pair {:?} is not representable", k));
        }
    }
    let mut words = vec![0i64; 32 + 4 * table.len()];
    let mut text: Vec<u8> = Vec::new();
    words[0] = 1;
    words[1] = mode;
    words[2] = rpc;
    words[3] = 1500;
    words[4] = 1000;
    words[5] = if env_mode_full { 1 } else { 0 };
    put_ref(&mut words, &mut text, 6, &opt_str(v, "sourceHome"));
    put_ref(&mut words, &mut text, 8, &opt_str(v, "folder"));
    put_ref(&mut words, &mut text, 10, &opt_str(v, "thread"));
    put_ref(&mut words, &mut text, 12, &opt_str(v, "binary"));
    put_ref(&mut words, &mut text, 14, &opt_str(v, "tempRoot"));
    put_ref(&mut words, &mut text, 16, &opt_str(v, "homedir"));
    words[18] = table.len() as i64;
    // direct plan JSON (mode 5 only): the raw text of the array, kept in cfg_text
    let plan: Option<String> = match (v.get("planJson"), v.get("plan")) {
        (Some(Value::String(s)), _) => Some(s.clone()),
        (_, Some(x)) if !x.is_null() => Some(serde_json::to_string(x).unwrap()),
        _ => None,
    };
    if plan.is_some() && mode != 5 {
        die("a direct plan input is only for mode plan");
    }
    put_ref(&mut words, &mut text, 19, &plan);
    for (i, (k, x)) in table.iter().enumerate() {
        let at = 32 + 4 * i;
        words[at] = text.len() as i64;
        words[at + 1] = k.len() as i64;
        text.extend_from_slice(k.as_bytes());
        words[at + 2] = text.len() as i64;
        words[at + 3] = x.len() as i64;
        text.extend_from_slice(x.as_bytes());
    }
    Cfg { words, text, trace, now_ms, artifact_dir }
}

// ---------------------------------------------------------------------------------------------------------------
// schedule (DESIGN section 6)

#[derive(Clone)]
enum Act {
    ReplaceFile { path: String, bytes: Vec<u8>, mode: Option<u32> },
    Symlink { path: String, target: String },
    Hardlink { path: String, of: String },
    Mkdir { path: String, mode: Option<u32> },
    Unlink { path: String },
    Append { path: String, bytes: Vec<u8> },
    Truncate { path: String, size: u64 },
    DbExec { path: String, sql: String, params: Vec<SqlValue> },
}

impl Act {
    // the same action with `@N` resolved in path/target/of (None: that temp directory does not exist yet)
    fn resolved(&self, r: &dyn Fn(&str) -> Option<String>) -> Option<Act> {
        Some(match self {
            Act::ReplaceFile { path, bytes, mode } => Act::ReplaceFile { path: r(path)?, bytes: bytes.clone(), mode: *mode },
            Act::Symlink { path, target } => Act::Symlink { path: r(path)?, target: r(target)? },
            Act::Hardlink { path, of } => Act::Hardlink { path: r(path)?, of: r(of)? },
            Act::Mkdir { path, mode } => Act::Mkdir { path: r(path)?, mode: *mode },
            Act::Unlink { path } => Act::Unlink { path: r(path)? },
            Act::Append { path, bytes } => Act::Append { path: r(path)?, bytes: bytes.clone() },
            Act::Truncate { path, size } => Act::Truncate { path: r(path)?, size: *size },
            Act::DbExec { path, sql, params } => Act::DbExec { path: r(path)?, sql: sql.clone(), params: params.clone() },
        })
    }

    fn label(&self) -> (&'static str, String) {
        match self {
            Act::ReplaceFile { path, .. } => ("replace-file", path.clone()),
            Act::Symlink { path, .. } => ("symlink", path.clone()),
            Act::Hardlink { path, .. } => ("hardlink", path.clone()),
            Act::Mkdir { path, .. } => ("mkdir", path.clone()),
            Act::Unlink { path } => ("unlink", path.clone()),
            Act::Append { path, .. } => ("append", path.clone()),
            Act::Truncate { path, .. } => ("truncate", path.clone()),
            Act::DbExec { path, .. } => ("db-exec", path.clone()),
        }
    }
}

struct Rule {
    op: String,
    phase: Option<String>,
    sql: Option<String>,
    path: Option<String>,
    occurrence: u64,
    after: bool,
    actions: Vec<Act>,
    error: Option<ErrInfo>,
}

const OP_NAMES: [&str; 15] = [
    "realpath", "lstat", "db_open_readonly", "db_query", "db_close", "mkdtemp", "open", "spawn", "observe", "kill", "close", "stat", "read", "rm",
    "clock_now",
];

fn check_keys(v: &Value, allowed: &[&str], what: &str) {
    if let Some(o) = v.as_object() {
        for k in o.keys() {
            if !allowed.contains(&k.as_str()) {
                die(&format!("schedule {}: unsupported key {}", what, k));
            }
        }
    } else {
        die(&format!("schedule {} must be an object", what));
    }
}

fn bytes_spec(v: &Value) -> Vec<u8> {
    if let Some(s) = v.get("utf8").and_then(|x| x.as_str()) {
        return s.as_bytes().to_vec();
    }
    if let Some(s) = v.get("hex").and_then(|x| x.as_str()) {
        return match unhex(s) {
            Ok(b) => b,
            Err(e) => die(&format!("bytes spec: {}", e)),
        };
    }
    die("schedule bytes spec needs utf8 or hex (the runner expands recipes to hex)")
}

fn typed_value(v: &Value) -> SqlValue {
    let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("");
    match kind {
        "null" => SqlValue::Null,
        "integer" => match v.get("decimal").and_then(|x| x.as_str()).and_then(|s| s.parse::<i64>().ok()) {
            Some(n) => SqlValue::Integer(n),
            None => die("typed integer needs a decimal string"),
        },
        "real" => match v.get("bits").and_then(|x| x.as_str()).and_then(|s| u64::from_str_radix(s, 16).ok()) {
            Some(b) => SqlValue::Real(f64::from_bits(b)),
            None => die("typed real needs 16 hex digits"),
        },
        "text" => match v.get("text").and_then(|x| x.as_str()) {
            Some(s) => SqlValue::Text(s.to_string()),
            None => die("typed text needs text"),
        },
        "blob" => match v.get("hex").and_then(|x| x.as_str()).map(unhex) {
            Some(Ok(b)) => SqlValue::Blob(b),
            _ => die("typed blob needs hex"),
        },
        _ => die(&format!("unknown typed value kind {:?}", kind)),
    }
}

fn parse_action(a: &Value) -> Act {
    let kind = match a.get("kind").and_then(|x| x.as_str()) {
        Some(k) => k,
        None => die("schedule action needs kind"),
    };
    let path = match a.get("path").and_then(|x| x.as_str()) {
        Some(p) => p.to_string(),
        None => die("schedule action needs path"),
    };
    let mode: Option<u32> = a.get("mode").and_then(|x| x.as_u64()).map(|m| m as u32);
    let bytes = |a: &Value| -> Vec<u8> {
        match a.get("bytes") {
            Some(b) => bytes_spec(b),
            None => die("schedule action needs bytes"),
        }
    };
    match kind {
        "replace-file" => Act::ReplaceFile { path, bytes: bytes(a), mode },
        "symlink" => match a.get("target").and_then(|x| x.as_str()) {
            Some(t) => Act::Symlink { path, target: t.to_string() },
            None => die("symlink action needs target"),
        },
        "hardlink" => match a.get("of").and_then(|x| x.as_str()) {
            Some(t) => Act::Hardlink { path, of: t.to_string() },
            None => die("hardlink action needs of"),
        },
        "mkdir" => Act::Mkdir { path, mode },
        "unlink" => Act::Unlink { path },
        "append" => Act::Append { path, bytes: bytes(a) },
        "truncate" => Act::Truncate { path, size: a.get("size").and_then(|x| x.as_u64()).unwrap_or(0) },
        "db-exec" => {
            let sql = match a.get("sql").and_then(|x| x.as_str()) {
                Some(s) => s.to_string(),
                None => die("db-exec action needs sql"),
            };
            let mut params = Vec::new();
            if let Some(ps) = a.get("params").and_then(|x| x.as_array()) {
                for p in ps {
                    params.push(typed_value(p));
                }
            }
            Act::DbExec { path, sql, params }
        }
        _ => die(&format!("unknown action kind {}", kind)),
    }
}

fn parse_error_spec(e: &Value) -> ErrInfo {
    check_keys(e, &["class", "code", "native", "message"], "error");
    let class = e.get("class").and_then(|x| x.as_str()).unwrap_or("");
    let code = match e.get("code").and_then(|x| x.as_str()) {
        Some(c) => c.to_string(),
        None => die("error replacement needs code"),
    };
    let native_over = e.get("native").and_then(|x| x.as_i64());
    let msg = format!("injected {}", code);
    match class {
        "OS" => match errno_of(&code) {
            Some(n) => ErrInfo { class: 1, native: native_over.unwrap_or(n as i64), code, msg },
            None => die(&format!("unknown errno name {}", code)),
        },
        "SQLite" => {
            let native = native_over.or_else(|| SQLITE_NAMES.iter().find(|x| x.0 == code).map(|x| x.1)).unwrap_or(1);
            ErrInfo { class: 2, native, code, msg }
        }
        "Range" | "binding-range" => ErrInfo { class: 3, native: native_over.unwrap_or(0), code, msg },
        "capacity" | "host-capacity" => ErrInfo { class: 4, native: native_over.unwrap_or(0), code, msg },
        _ => die(&format!("unknown error class {:?}", class)),
    }
}

fn parse_schedule(v: Option<&Value>) -> Vec<Rule> {
    let mut rules = Vec::new();
    let v = match v {
        None | Some(Value::Null) => return rules,
        Some(x) => x,
    };
    check_keys(v, &["rules"], "schedule");
    let arr = match v.get("rules") {
        None | Some(Value::Null) => return rules,
        Some(Value::Array(a)) => a,
        _ => die("schedule.rules must be an array"),
    };
    for r in arr {
        check_keys(r, &["match", "actions", "error"], "rule");
        let m = match r.get("match") {
            Some(m) => m,
            None => die("schedule rule needs match"),
        };
        check_keys(m, &["op", "phase", "sql", "path", "occurrence", "barrier"], "match");
        let mut op = match m.get("op").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => die("schedule match needs op"),
        };
        if op == "spawn_stdout_fd" {
            op = "spawn".to_string();
        }
        if !OP_NAMES.contains(&op.as_str()) {
            die(&format!("schedule op {} unknown", op));
        }
        let phase = m.get("phase").and_then(|x| x.as_str()).map(|s| s.to_string());
        if let Some(p) = &phase {
            if !["prepare", "all", "get"].contains(&p.as_str()) {
                die(&format!("schedule phase {} unknown", p));
            }
        }
        let after = match m.get("barrier").and_then(|x| x.as_str()) {
            None | Some("before-call") => false,
            Some("after-result-before-delivery") => true,
            Some(b) => die(&format!("unknown barrier {}", b)),
        };
        let mut actions = Vec::new();
        if let Some(acts) = r.get("actions").and_then(|x| x.as_array()) {
            for a in acts {
                actions.push(parse_action(a));
            }
        }
        rules.push(Rule {
            op,
            phase,
            sql: m.get("sql").and_then(|x| x.as_str()).map(|s| s.to_string()),
            path: m.get("path").and_then(|x| x.as_str()).map(|s| s.to_string()),
            occurrence: m.get("occurrence").and_then(|x| x.as_u64()).unwrap_or(1),
            after,
            actions,
            error: r.get("error").map(parse_error_spec),
        });
    }
    rules
}

fn run_action(a: &Act) -> Result<(), String> {
    fn wipe(p: &str) -> Result<(), String> {
        match fs::symlink_metadata(p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {}", p, e)),
            Ok(m) => {
                let r = if m.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
                r.map_err(|e| format!("{}: {}", p, e))
            }
        }
    }
    match a {
        Act::ReplaceFile { path, bytes, mode } => {
            wipe(path)?;
            let mut o = OpenOptions::new();
            o.write(true).create_new(true);
            o.mode(mode.unwrap_or(0o644));
            let mut f = o.open(path).map_err(|e| format!("{}: {}", path, e))?;
            f.write_all(bytes).map_err(|e| format!("{}: {}", path, e))?;
            if let Some(m) = mode {
                fs::set_permissions(path, fs::Permissions::from_mode(*m)).map_err(|e| format!("{}: {}", path, e))?;
            }
            Ok(())
        }
        Act::Symlink { path, target } => {
            wipe(path)?;
            std::os::unix::fs::symlink(target, path).map_err(|e| format!("{}: {}", path, e))
        }
        Act::Hardlink { path, of } => {
            wipe(path)?;
            fs::hard_link(of, path).map_err(|e| format!("{}: {}", path, e))
        }
        Act::Mkdir { path, mode } => {
            let mut b = fs::DirBuilder::new();
            b.mode(mode.unwrap_or(0o777));
            b.create(path).map_err(|e| format!("{}: {}", path, e))?;
            if let Some(m) = mode {
                fs::set_permissions(path, fs::Permissions::from_mode(*m)).map_err(|e| format!("{}: {}", path, e))?;
            }
            Ok(())
        }
        Act::Unlink { path } => wipe(path),
        Act::Append { path, bytes } => {
            let mut f = OpenOptions::new().append(true).create(true).open(path).map_err(|e| format!("{}: {}", path, e))?;
            f.write_all(bytes).map_err(|e| format!("{}: {}", path, e))
        }
        Act::Truncate { path, size } => {
            let f = OpenOptions::new().write(true).open(path).map_err(|e| format!("{}: {}", path, e))?;
            f.set_len(*size).map_err(|e| format!("{}: {}", path, e))
        }
        Act::DbExec { path, sql, params } => {
            // a separate setup connection, never the importer's
            let c = Connection::open(path).map_err(|e| format!("{}: {}", path, e))?;
            if params.is_empty() {
                c.execute_batch(sql).map_err(|e| format!("db-exec {}: {}", path, e))
            } else {
                c.execute(sql, rusqlite::params_from_iter(params.iter())).map(|_| ()).map_err(|e| format!("db-exec {}: {}", path, e))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// step packet

struct Step {
    action: i64,
    ctl: Vec<i64>,
    patches: Vec<Vec<i64>>,
    requests: Vec<Vec<i64>>,
    words: Vec<i64>,
    bytes: Vec<u8>,
    result: Vec<i64>,
}

fn parse_step(p: &[i64]) -> Step {
    if p.len() < HEADER {
        die(&format!("step packet too short: {} words", p.len()));
    }
    if p[0] != MAGIC_STEP {
        die("step packet magic mismatch");
    }
    if p[1] != 1 {
        die("step packet version must be 1");
    }
    let action = p[2];
    if !(0..=2).contains(&action) {
        die("step packet action out of range");
    }
    if p[3] != CTL_WORDS as i64 {
        die("step packet ctl word count must be 64");
    }
    let (np, nr, nw, nb, nres) = (p[4], p[5], p[6], p[7], p[8]);
    if np < 0 || nr < 0 || nw < 0 || nb < 0 || nres < 0 {
        die("step packet negative count");
    }
    if p[9] != 0 || p[10] != 0 || p[11] != 0 {
        die("step packet reserved words must be 0");
    }
    let total: i128 = HEADER as i128 + 8 * np as i128 + 12 * nr as i128 + nw as i128 + nb as i128 + nres as i128;
    if total != p.len() as i128 {
        die(&format!("step packet length {} does not match the formula {}", p.len(), total));
    }
    match action {
        0 => {
            if nr != 1 {
                die("an await step issues exactly one request");
            }
            if nres != 0 {
                die("only a done step carries a result");
            }
        }
        1 => {
            if nr != 0 {
                die("a done step must not issue requests");
            }
            if nres == 0 {
                die("a done step carries a result");
            }
        }
        _ => {
            if nr != 0 || nres != 0 {
                die("a continue step carries no request and no result");
            }
        }
    }
    let (np, nr, nw, nb, nres) = (np as usize, nr as usize, nw as usize, nb as usize, nres as usize);
    let ctl = p[12..12 + CTL_WORDS].to_vec();
    let mut at = HEADER;
    let mut patches = Vec::with_capacity(np);
    for _ in 0..np {
        let w = p[at..at + 8].to_vec();
        at += 8;
        if w[0] != 0 && w[0] != 1 {
            die("patch kind out of range");
        }
        if !(0..=5).contains(&w[1]) {
            die("patch bank out of range");
        }
        if w[2] < 0 || w[3] < 0 || w[5] < 0 {
            die("patch has a negative destination, count or source offset");
        }
        if !(0..=4).contains(&w[4]) {
            die("patch source out of range");
        }
        if w[6] != 0 || w[7] != 0 {
            die("patch reserved words must be 0");
        }
        patches.push(w);
    }
    let mut requests = Vec::with_capacity(nr);
    for i in 0..nr {
        requests.push(p[at + 12 * i..at + 12 * i + 12].to_vec());
    }
    at += 12 * nr;
    let words = p[at..at + nw].to_vec();
    at += nw;
    let mut bytes = Vec::with_capacity(nb);
    for i in 0..nb {
        let x = p[at + i];
        if !(0..=255).contains(&x) {
            die("literal byte pool value outside 0..255");
        }
        bytes.push(x as u8);
    }
    at += nb;
    let result = p[at..at + nres].to_vec();
    Step { action, ctl, patches, requests, words, bytes, result }
}

enum Src<'a> {
    Words(&'a [i64]),
    Bytes(&'a [u8]),
}

fn apply_patches(b: &mut Banks, st: &Step, event: &[i64], comp: &[u8]) {
    for w in &st.patches {
        let bank = w[1] as usize;
        let (dst, cnt, source, soff) = (w[2], w[3], w[4], w[5]);
        if w[0] == 0 {
            if dst != 0 || source != 0 || soff != 0 {
                die("resize patch must have destination 0 and no source");
            }
            if cnt > MAX_BANK {
                die("resize patch exceeds the bank cap 2^31");
            }
            b.resize(bank, cnt as usize);
        } else {
            if source == 0 {
                die("copy patch needs a source");
            }
            if dst as i128 + cnt as i128 > MAX_BANK as i128 {
                die("copy patch exceeds the bank cap 2^31");
            }
            if dst as i128 + cnt as i128 > b.len(bank) as i128 {
                die("copy patch destination range outside the bank");
            }
            let src = match source {
                1 => Src::Words(event),
                2 => Src::Bytes(comp),
                3 => Src::Words(&st.words),
                _ => Src::Bytes(&st.bytes),
            };
            let slen = match &src {
                Src::Words(s) => s.len(),
                Src::Bytes(s) => s.len(),
            };
            if soff as i128 + cnt as i128 > slen as i128 {
                die("copy patch source range outside the source");
            }
            let (dst, cnt, soff) = (dst as usize, cnt as usize, soff as usize);
            match (Banks::is_words(bank), src) {
                (true, Src::Words(s)) => b.words_mut(bank)[dst..dst + cnt].copy_from_slice(&s[soff..soff + cnt]),
                (true, Src::Bytes(s)) => {
                    let d = b.words_mut(bank);
                    for i in 0..cnt {
                        d[dst + i] = s[soff + i] as i64;
                    }
                }
                (false, Src::Words(s)) => {
                    let d = b.bytes_mut(bank);
                    for i in 0..cnt {
                        let x = s[soff + i];
                        if !(0..=255).contains(&x) {
                            die("patch copies a word outside 0..255 into a byte bank");
                        }
                        d[dst + i] = x as u8;
                    }
                }
                (false, Src::Bytes(s)) => b.bytes_mut(bank)[dst..dst + cnt].copy_from_slice(&s[soff..soff + cnt]),
            }
        }
    }
}

fn empty_event() -> Vec<i64> {
    let mut e = vec![0i64; 16];
    e[0] = 1;
    e[8] = -1;
    e[10] = -1;
    e
}

// ---------------------------------------------------------------------------------------------------------------
// requests

struct Req {
    id: i64,
    op: i64,
    stage: i64,
    flags: i64,
    handle: i64,
    a: i64,
    args: Vec<i64>,
}

fn op_name(op: i64) -> &'static str {
    OP_NAMES[(op - 1) as usize]
}

fn parse_req(r: &[i64], words: &[i64]) -> Req {
    if r[10] != 0 || r[11] != 0 {
        die("request reserved words must be 0");
    }
    if r[0] < 1 {
        die("request id must be at least 1");
    }
    if !(1..=15).contains(&r[1]) {
        die("request opcode out of range");
    }
    if !(0..=28).contains(&r[2]) {
        die("request stage out of range");
    }
    if !(0..=0xFFFF).contains(&r[3]) {
        die("request flags out of range");
    }
    if r[4] != -1 && r[4] < 1 {
        die("request handle must be -1 or at least 1");
    }
    if r[6] != 0 || r[7] != 0 {
        die("request scalar words b and c must be 0");
    }
    if r[8] < 0 || r[9] < 0 || r[8] as i128 + r[9] as i128 > words.len() as i128 {
        die("request argument range outside the word pool");
    }
    let args = words[r[8] as usize..(r[8] + r[9]) as usize].to_vec();
    Req { id: r[0], op: r[1], stage: r[2], flags: r[3], handle: r[4], a: r[5], args }
}

fn ref_bytes(lit: &[u8], s: i64, n: i64, what: &str) -> Vec<u8> {
    if s < 0 || n < 0 || s as i128 + n as i128 > lit.len() as i128 {
        die(&format!("request {} ref outside the byte pool", what));
    }
    lit[s as usize..(s + n) as usize].to_vec()
}

// A NUL byte is not a protocol error: Node refuses such a path/argv/env string with ERR_INVALID_ARG_VALUE, so `execute`
// turns it into a failed completion (`call_has_nul`) after the before-call barrier, like the native call would fail.
fn path_arg(args: &[i64], at: usize, lit: &[u8], what: &str) -> Vec<u8> {
    ref_bytes(lit, args[at], args[at + 1], what)
}

fn call_has_nul(call: &Call) -> bool {
    match call {
        Call::Realpath(p) | Call::Lstat(p) | Call::Stat(p) | Call::ReadFile(p) | Call::Rm(p) | Call::DbOpen(p) | Call::Mkdtemp(p) => p.contains(&0),
        Call::Open { path, .. } => path.contains(&0),
        Call::Spawn(s) => {
            s.binary.contains(&0) || s.cwd.contains(&0) || s.argv.iter().any(|a| a.contains(&0)) || s.env.iter().any(|(k, v)| k.contains(&0) || v.contains(&0))
        }
        _ => false,
    }
}

fn invalid_arg_err() -> ErrInfo {
    ErrInfo { class: 3, native: 0, code: "ERR_INVALID_ARG_VALUE".to_string(), msg: "argument contains a NUL byte".to_string() }
}

fn sql_value(args: &[i64], at: usize, lit: &[u8]) -> SqlValue {
    let (k, a, b, c) = (args[at], args[at + 1], args[at + 2], args[at + 3]);
    match k {
        0 => {
            if a != 0 || b != 0 || c != 0 {
                die("typed NULL must have zero payload words");
            }
            SqlValue::Null
        }
        1 => {
            if b != 0 || c != 0 {
                die("typed INTEGER uses one word");
            }
            SqlValue::Integer(a)
        }
        2 => {
            if !(0..=0xFFFF_FFFF).contains(&a) || !(0..=0xFFFF_FFFF).contains(&b) || c != 0 {
                die("typed REAL needs unsigned high32/low32 words");
            }
            SqlValue::Real(f64::from_bits(((a as u64) << 32) | b as u64))
        }
        3 => {
            if c != 0 {
                die("typed TEXT word c must be 0");
            }
            match String::from_utf8(ref_bytes(lit, a, b, "text parameter")) {
                Ok(s) => SqlValue::Text(s),
                Err(_) => die("typed TEXT parameter is not UTF-8"),
            }
        }
        4 => {
            if c != 0 {
                die("typed BLOB word c must be 0");
            }
            SqlValue::Blob(ref_bytes(lit, a, b, "blob parameter"))
        }
        _ => die("typed value kind out of range"),
    }
}

struct SpawnSpec {
    binary: Vec<u8>,
    cwd: Vec<u8>,
    argv: Vec<Vec<u8>>,
    env_mode: i64,
    env: Vec<(Vec<u8>, Vec<u8>)>,
    detached: bool,
    pipe: bool,
}

enum Call {
    Realpath(Vec<u8>),
    Lstat(Vec<u8>),
    Stat(Vec<u8>),
    ReadFile(Vec<u8>),
    Rm(Vec<u8>),
    DbOpen(Vec<u8>),
    Prepare { conn: i64, sql: String },
    Exec { stmt: i64, get: bool, params: Vec<SqlValue> },
    DbClose,
    Mkdtemp(Vec<u8>),
    Open { path: Vec<u8>, mode: u32 },
    Spawn(SpawnSpec),
    Observe { deadline: i64 },
    Kill { signal: i32 },
    Close,
    Clock { wall: bool },
}

fn need_handle(req: &Req) {
    if req.handle < 1 {
        die(&format!("{} needs a handle", op_name(req.op)));
    }
}

fn no_handle(req: &Req) {
    if req.handle != -1 {
        die(&format!("{} takes no handle", op_name(req.op)));
    }
}

fn parse_call(req: &Req, lit: &[u8]) -> Call {
    let a = &req.args;
    let one_path = |what: &str| -> Vec<u8> {
        if a.len() != 2 {
            die(&format!("{} takes one path ref", op_name(req.op)));
        }
        path_arg(a, 0, lit, what)
    };
    let plain = |flags_ok: bool| {
        if !flags_ok {
            die(&format!("{} flags {} not allowed", op_name(req.op), req.flags));
        }
        if req.a != 0 {
            die(&format!("{} scalar a must be 0", op_name(req.op)));
        }
    };
    match req.op {
        1 | 2 | 12 | 13 => {
            plain(req.flags == 0);
            no_handle(req);
            let p = one_path("path");
            match req.op {
                1 => Call::Realpath(p),
                2 => Call::Lstat(p),
                12 => Call::Stat(p),
                _ => Call::ReadFile(p),
            }
        }
        14 => {
            plain(req.flags == 3);
            no_handle(req);
            Call::Rm(one_path("path"))
        }
        3 => {
            plain(req.flags == 0);
            no_handle(req);
            Call::DbOpen(one_path("path"))
        }
        4 => {
            plain(req.flags <= 2);
            need_handle(req);
            if req.flags == 0 {
                if a.len() != 2 {
                    die("db_query prepare takes one sql ref");
                }
                match String::from_utf8(ref_bytes(lit, a[0], a[1], "sql")) {
                    Ok(sql) => Call::Prepare { conn: req.handle, sql },
                    Err(_) => die("sql is not UTF-8"),
                }
            } else {
                if a.is_empty() || a[0] < 0 || (a.len() as i128) != 1 + 4 * a[0] as i128 {
                    die("db_query execute argument count does not match P");
                }
                let mut params = Vec::new();
                for i in 0..a[0] as usize {
                    params.push(sql_value(a, 1 + 4 * i, lit));
                }
                Call::Exec { stmt: req.handle, get: req.flags == 2, params }
            }
        }
        5 => {
            plain(req.flags == 0);
            need_handle(req);
            if !a.is_empty() {
                die("db_close takes no arguments");
            }
            Call::DbClose
        }
        6 => {
            plain(req.flags == 0);
            no_handle(req);
            Call::Mkdtemp(one_path("prefix"))
        }
        7 => {
            if req.flags != 1 {
                die("open flags must be 1 (write/create/exclusive)");
            }
            if req.a <= 0 || req.a > 0o777 {
                die("open mode out of range");
            }
            no_handle(req);
            Call::Open { path: one_path("path"), mode: req.a as u32 }
        }
        8 => {
            plain(req.flags <= 1);
            need_handle(req);
            if a.len() < 12 {
                die("spawn arguments too short");
            }
            let binary = path_arg(a, 0, lit, "binary");
            let cwd = path_arg(a, 2, lit, "cwd");
            let (argc, env_mode, env_count, shell, stdin_mode, stderr_mode, detached, hide) = (a[4], a[5], a[6], a[7], a[8], a[9], a[10], a[11]);
            if argc < 0 || env_count < 0 {
                die("spawn counts negative");
            }
            if shell != 0 || stdin_mode != 0 || stderr_mode != 0 {
                die("spawn requires shell=0 and ignored stdin/stderr");
            }
            if !(0..=1).contains(&env_mode) || !(0..=1).contains(&detached) || !(0..=1).contains(&hide) {
                die("spawn envMode/detached/windowsHide must be 0 or 1");
            }
            if a.len() as i128 != 12 + 2 * argc as i128 + 4 * env_count as i128 {
                die("spawn argument word count does not match argc/envCount");
            }
            let mut argv = Vec::new();
            for i in 0..argc as usize {
                let b = path_arg(a, 12 + 2 * i, lit, "argv");
                argv.push(b);
            }
            let base = 12 + 2 * argc as usize;
            let mut env = Vec::new();
            for i in 0..env_count as usize {
                let k = path_arg(a, base + 4 * i, lit, "env key");
                let v = path_arg(a, base + 4 * i + 2, lit, "env value");
                if k.is_empty() || k.contains(&b'=') {
                    die("env key empty or contains '='");
                }
                env.push((k, v));
            }
            Call::Spawn(SpawnSpec { binary, cwd, argv, env_mode, env, detached: detached == 1, pipe: req.flags == 1 })
        }
        9 => {
            if req.flags != 0 {
                die("observe flags must be 0");
            }
            need_handle(req);
            if !a.is_empty() || req.a < 0 {
                die("observe takes a nonnegative deadline and no arguments");
            }
            Call::Observe { deadline: req.a }
        }
        10 => {
            if req.flags != 1 {
                die("kill flags must be 1 (isolated group target)");
            }
            need_handle(req);
            if !a.is_empty() || (req.a != 15 && req.a != 9) {
                die("kill takes signal 15 or 9 and no arguments");
            }
            Call::Kill { signal: req.a as i32 }
        }
        11 => {
            plain(req.flags == 0);
            need_handle(req);
            if !a.is_empty() {
                die("close takes no arguments");
            }
            Call::Close
        }
        _ => {
            plain(req.flags <= 1);
            no_handle(req);
            if !a.is_empty() {
                die("clock_now takes no arguments");
            }
            Call::Clock { wall: req.flags == 1 }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// host state

struct Done {
    payload: Vec<i64>,
    bytes: Vec<u8>,
    result: Value,
    header_err: Option<ErrInfo>,
}

fn done(payload: Vec<i64>, bytes: Vec<u8>, result: Value) -> Result<Done, ErrInfo> {
    Ok(Done { payload, bytes, result, header_err: None })
}

struct ConnEntry {
    conn: Connection,
    path: String,
}

struct StmtEntry {
    conn: i64,
    sql: String,
}

struct FileEntry {
    file: File,
    path: String,
}

enum ChildEv {
    Close { code: Option<i32>, signal: Option<i32> },
    Error(ErrInfo),
}

struct ChildEntry {
    pid: i32,
    rx: mpsc::Receiver<ChildEv>,
    queue: VecDeque<ChildEv>,
    ended: Arc<AtomicBool>,
    binary: String,
}

struct Host {
    trace: bool,
    seq: u64,
    start: Instant,
    now_ms: Option<i64>,
    artifact_dir: Option<String>,
    rules: Vec<Rule>,
    occ: HashMap<(String, String, String), u64>,
    last_id: i64,
    next_handle: i64,
    conns: HashMap<i64, ConnEntry>,
    stmts: HashMap<i64, StmtEntry>,
    files: HashMap<i64, FileEntry>,
    children: HashMap<i64, ChildEntry>,
    temp_dirs: Vec<String>,
}

fn kind_of(t: &fs::FileType) -> i64 {
    if t.is_file() {
        1
    } else if t.is_dir() {
        2
    } else if t.is_symlink() {
        3
    } else if t.is_fifo() {
        4
    } else if t.is_socket() {
        5
    } else if t.is_char_device() {
        6
    } else if t.is_block_device() {
        7
    } else {
        8
    }
}

fn meta_done(m: &fs::Metadata) -> Result<Done, ErrInfo> {
    let kind = kind_of(&m.file_type());
    let size = m.size();
    if size > i64::MAX as u64 {
        return Err(capacity_err("file size exceeds i64"));
    }
    done(vec![kind, size as i64], Vec::new(), json!({"kind": kind, "size": size as i64}))
}

fn path_of(b: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(b))
}

// one statement execution: all (every row) or get (at most one row), typed values, whole-query failure on any unsafe INTEGER
fn run_query(conn: &Connection, sql: &str, params: &[SqlValue], get: bool) -> Result<(Vec<String>, Vec<Vec<SqlValue>>), ErrInfo> {
    let mut stmt = conn.prepare_cached(sql).map_err(|e| sqlite_err(&e))?;
    let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let n = cols.len();
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter())).map_err(|e| sqlite_err(&e))?;
    let mut out: Vec<Vec<SqlValue>> = Vec::new();
    loop {
        let row = match rows.next() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => return Err(sqlite_err(&e)),
        };
        let mut vals = Vec::with_capacity(n);
        for i in 0..n {
            let v = match row.get_ref(i) {
                Ok(v) => v,
                Err(e) => return Err(sqlite_err(&e)),
            };
            vals.push(match v {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(x) => {
                    if x > MAX_SAFE || x < -MAX_SAFE {
                        return Err(range_err());
                    }
                    SqlValue::Integer(x)
                }
                ValueRef::Real(f) => SqlValue::Real(f),
                ValueRef::Text(t) => SqlValue::Text(String::from_utf8_lossy(AsRef::<[u8]>::as_ref(t)).into_owned()),
                ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
            });
        }
        out.push(vals);
        if get {
            break;
        }
    }
    Ok((cols, out))
}

impl Host {
    fn new(cfg: &Cfg, v: &Value) -> Host {
        Host {
            trace: cfg.trace,
            seq: 0,
            start: Instant::now(),
            now_ms: cfg.now_ms,
            artifact_dir: cfg.artifact_dir.clone(),
            rules: parse_schedule(v.get("schedule")),
            occ: HashMap::new(),
            last_id: 0,
            next_handle: 1,
            conns: HashMap::new(),
            stmts: HashMap::new(),
            files: HashMap::new(),
            children: HashMap::new(),
            temp_dirs: Vec::new(),
        }
    }

    fn mono(&self) -> i64 {
        self.start.elapsed().as_millis() as i64
    }

    fn new_handle(&mut self) -> i64 {
        let h = self.next_handle;
        self.next_handle += 1;
        h
    }

    // ---- trace ----------------------------------------------------------------------------------------------

    // trace JSON, hashes and artifact files exist only for the trace; with trace off they are never built (M06)
    fn artifact(&self, data: &[u8]) -> Value {
        if !self.trace {
            return Value::Null;
        }
        let h = sha256_hex(data);
        if let Some(dir) = &self.artifact_dir {
            let p = Path::new(dir).join(&h);
            if !p.exists() {
                let _ = fs::write(&p, data);
            }
        }
        json!({"length": data.len(), "sha256": h})
    }

    fn typed_json(&self, v: &SqlValue) -> Value {
        if !self.trace {
            return Value::Null;
        }
        match v {
            SqlValue::Null => json!({"kind": "null"}),
            SqlValue::Integer(x) => json!({"kind": "integer", "decimal": x.to_string()}),
            SqlValue::Real(f) => json!({"kind": "real", "bits": format!("{:016x}", f.to_bits())}),
            SqlValue::Text(s) => {
                if s.len() > 65536 {
                    let a = self.artifact(s.as_bytes());
                    json!({"kind": "text", "length": a["length"], "sha256": a["sha256"]})
                } else {
                    json!({"kind": "text", "text": s})
                }
            }
            SqlValue::Blob(b) => {
                if b.len() > 4096 {
                    let a = self.artifact(b);
                    json!({"kind": "blob", "length": a["length"], "sha256": a["sha256"]})
                } else {
                    json!({"kind": "blob", "hex": hex(b)})
                }
            }
        }
    }

    fn emit(&mut self, event: &str, id: Value, op: Value, stage: Value, handle: Value, args: Value, result: Value, error: Value) {
        if !self.trace {
            return;
        }
        self.seq += 1;
        let v = json!({"trace": {"v": 1, "seq": self.seq, "event": event, "id": id, "op": op, "stage": stage, "handle": handle,
            "args": args, "result": result, "error": error}});
        eprintln!("{}", v);
    }

    fn emit_simple(&mut self, event: &str, req: &Req, op: &str, args: Value) {
        let handle = if req.handle >= 1 { json!(req.handle) } else { Value::Null };
        self.emit(event, json!(req.id), json!(op), json!(req.stage), handle, args, Value::Null, Value::Null);
    }

    // ---- rule matching -----------------------------------------------------------------------------------------

    // `opencode-@1` in a rule path names the random suffix of the first mkdtemp result
    fn resolve_path(&self, pat: &str) -> Option<String> {
        let b = pat.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'@' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let n: usize = pat[i + 1..j].parse().ok()?;
                let dir = self.temp_dirs.get(n.checked_sub(1)?)?;
                let base = dir.rsplit('/').next()?;
                let suffix = base.get(base.len().checked_sub(6)?..)?;
                out.push_str(suffix);
                i = j;
            } else {
                let ch = pat[i..].chars().next()?;
                out.push(ch);
                i += ch.len_utf8();
            }
        }
        Some(out)
    }

    fn rule_hits(&self, i: usize, op: &str, phase: Option<&str>, key: &Option<String>, occ: u64) -> bool {
        let r = &self.rules[i];
        if r.op != op || r.occurrence != occ {
            return false;
        }
        if let Some(p) = &r.phase {
            if Some(p.as_str()) != phase {
                return false;
            }
        }
        if let Some(s) = &r.sql {
            if op != "db_query" || key.as_deref() != Some(s.as_str()) {
                return false;
            }
        }
        if let Some(p) = &r.path {
            match self.resolve_path(p) {
                Some(x) => {
                    if key.as_deref() != Some(x.as_str()) {
                        return false;
                    }
                }
                None => return false,
            }
        }
        true
    }

    // (op name, logical path or SQL, phase, trace args)
    // operation name, schedule-match key and phase, plus the trace arguments (built only with trace on, M06)
    fn describe(&self, call: &Call, req: &Req) -> (&'static str, Option<String>, Option<&'static str>, Value) {
        let op = op_name(req.op);
        if !self.trace {
            let (key, phase) = self.match_key(call, req);
            return (op, key, phase, Value::Null);
        }
        match call {
            Call::Realpath(p) | Call::Lstat(p) | Call::Stat(p) | Call::Rm(p) | Call::DbOpen(p) | Call::Mkdtemp(p) | Call::ReadFile(p) => {
                let s = lossy(p);
                let args = match call {
                    Call::Rm(_) => json!({"path": s, "recursive": true, "force": true}),
                    Call::Mkdtemp(_) => json!({"prefix": s}),
                    _ => json!({"path": s}),
                };
                (op, Some(s), None, args)
            }
            Call::Open { path, mode } => {
                let s = lossy(path);
                (op, Some(s.clone()), None, json!({"path": s, "flags": "wx", "mode": mode}))
            }
            Call::Prepare { sql, .. } => (op, Some(sql.clone()), Some("prepare"), json!({"phase": "prepare", "sql": sql})),
            Call::Exec { stmt, get, params } => {
                let sql = self.stmts.get(stmt).map(|s| s.sql.clone()).unwrap_or_default();
                let phase = if *get { "get" } else { "all" };
                let ps: Vec<Value> = params.iter().map(|p| self.typed_json(p)).collect();
                (op, Some(sql.clone()), Some(phase), json!({"phase": phase, "sql": sql, "params": ps}))
            }
            Call::DbClose => {
                let p = self.conns.get(&req.handle).map(|c| c.path.clone()).unwrap_or_default();
                (op, Some(p), None, json!({}))
            }
            Call::Close => {
                let p = self.files.get(&req.handle).map(|c| c.path.clone()).unwrap_or_default();
                (op, Some(p), None, json!({}))
            }
            Call::Spawn(s) => {
                let out = self.files.get(&req.handle).map(|c| c.path.clone()).unwrap_or_default();
                let mut env: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
                if s.env_mode == 0 {
                    for (k, v) in std::env::vars_os() {
                        env.insert(k.as_bytes().to_vec(), v.as_bytes().to_vec());
                    }
                }
                for (k, v) in &s.env {
                    env.insert(k.clone(), v.clone());
                }
                let envj: Vec<Value> = env.iter().map(|(k, v)| json!([lossy(k), lossy(v)])).collect();
                let binary = lossy(&s.binary);
                let argvj: Vec<String> = s.argv.iter().map(|a| lossy(a)).collect();
                let stdoutj = if s.pipe { json!({"kind": "pipe"}) } else { json!({"kind": "file", "path": out}) };
                let cwdj = lossy(&s.cwd);
                let args = json!({
                    "binary": binary, "cwd": cwdj, "argv": argvj,
                    "envMode": s.env_mode, "env": envj, "shell": 0, "stdin": "ignore", "stderr": "ignore",
                    "detached": s.detached, "windowsHide": true, "stdout": stdoutj,
                });
                (op, Some(binary), None, args)
            }
            Call::Observe { deadline } => {
                let b = self.children.get(&req.handle).map(|c| c.binary.clone());
                (op, b, None, json!({"deadlineMs": deadline}))
            }
            Call::Kill { signal } => {
                let b = self.children.get(&req.handle).map(|c| c.binary.clone());
                (op, b, None, json!({"signal": signal, "group": true}))
            }
            Call::Clock { wall } => (op, None, None, json!({"clock": if *wall { "wall" } else { "monotonic" }})),
        }
    }

    // the schedule-match key and phase of `describe` without the trace arguments
    fn match_key(&self, call: &Call, req: &Req) -> (Option<String>, Option<&'static str>) {
        match call {
            Call::Realpath(p) | Call::Lstat(p) | Call::Stat(p) | Call::Rm(p) | Call::DbOpen(p) | Call::Mkdtemp(p) | Call::ReadFile(p) => (Some(lossy(p)), None),
            Call::Open { path, .. } => (Some(lossy(path)), None),
            Call::Prepare { sql, .. } => (Some(sql.clone()), Some("prepare")),
            Call::Exec { stmt, get, .. } => {
                let sql = self.stmts.get(stmt).map(|s| s.sql.clone()).unwrap_or_default();
                (Some(sql), Some(if *get { "get" } else { "all" }))
            }
            Call::DbClose => (Some(self.conns.get(&req.handle).map(|c| c.path.clone()).unwrap_or_default()), None),
            Call::Close => (Some(self.files.get(&req.handle).map(|c| c.path.clone()).unwrap_or_default()), None),
            Call::Spawn(s) => (Some(lossy(&s.binary)), None),
            Call::Observe { .. } | Call::Kill { .. } => (self.children.get(&req.handle).map(|c| c.binary.clone()), None),
            Call::Clock { .. } => (None, None),
        }
    }

    // ---- native operations ---------------------------------------------------------------------------------------

    fn exec_call(&mut self, call: &Call, req: &Req) -> Result<Done, ErrInfo> {
        match call {
            Call::Realpath(p) => match fs::canonicalize(path_of(p)) {
                Ok(r) => {
                    let b = r.as_os_str().as_bytes().to_vec();
                    done(vec![0, b.len() as i64], b.clone(), json!({"path": lossy(&b)}))
                }
                Err(e) => Err(os_err(&e)),
            },
            Call::Lstat(p) => match fs::symlink_metadata(path_of(p)) {
                Ok(m) => meta_done(&m),
                Err(e) => Err(os_err(&e)),
            },
            Call::Stat(p) => match fs::metadata(path_of(p)) {
                Ok(m) => meta_done(&m),
                Err(e) => Err(os_err(&e)),
            },
            Call::ReadFile(p) => match fs::read(path_of(p)) {
                Ok(d) => {
                    if d.len() as i64 >= MAX_BANK {
                        return Err(capacity_err("file does not fit a 2^31 bank"));
                    }
                    let a = self.artifact(&d);
                    done(vec![0, d.len() as i64], d, json!({"data": a}))
                }
                Err(e) => Err(os_err(&e)),
            },
            Call::Rm(p) => {
                let path = path_of(p);
                match fs::symlink_metadata(path) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => done(Vec::new(), Vec::new(), json!({})),
                    Err(e) => Err(os_err(&e)),
                    Ok(m) => {
                        let r = if m.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
                        match r {
                            Ok(()) => done(Vec::new(), Vec::new(), json!({})),
                            Err(e) => Err(os_err(&e)),
                        }
                    }
                }
            }
            Call::DbOpen(p) => match Connection::open_with_flags(path_of(p), OpenFlags::SQLITE_OPEN_READ_ONLY) {
                Ok(c) => {
                    // node:sqlite DatabaseSync has no busy handler (timeout 0); rusqlite installs a 5 s one by default
                    if let Err(e) = c.busy_timeout(Duration::ZERO) {
                        return Err(sqlite_err(&e));
                    }
                    let h = self.new_handle();
                    self.conns.insert(h, ConnEntry { conn: c, path: lossy(p) });
                    done(vec![h], Vec::new(), json!({"connection": h}))
                }
                Err(e) => Err(sqlite_err(&e)),
            },
            Call::Prepare { conn, sql } => {
                let ent = match self.conns.get(conn) {
                    Some(c) => c,
                    None => die("db_query prepare on an unknown connection handle"),
                };
                let cols: Vec<String> = {
                    let stmt = ent.conn.prepare_cached(sql).map_err(|e| sqlite_err(&e))?;
                    stmt.column_names().iter().map(|s| s.to_string()).collect()
                };
                let h = self.new_handle();
                self.stmts.insert(h, StmtEntry { conn: *conn, sql: sql.clone() });
                let mut payload = vec![h, cols.len() as i64];
                let mut bytes = Vec::new();
                for c in &cols {
                    payload.push(bytes.len() as i64);
                    payload.push(c.len() as i64);
                    bytes.extend_from_slice(c.as_bytes());
                }
                done(payload, bytes, json!({"statement": h, "columns": cols}))
            }
            Call::Exec { stmt, get, params } => {
                let (cid, sql) = match self.stmts.get(stmt) {
                    Some(s) => (s.conn, s.sql.clone()),
                    None => die("db_query execute on an unknown statement handle"),
                };
                let ent = match self.conns.get(&cid) {
                    Some(c) => c,
                    None => die("db_query execute: the statement's connection is closed"),
                };
                let (cols, rows) = run_query(&ent.conn, &sql, params, *get)?;
                let c = cols.len();
                let mut payload = vec![c as i64, rows.len() as i64];
                let mut bytes: Vec<u8> = Vec::new();
                for name in &cols {
                    payload.push(bytes.len() as i64);
                    payload.push(name.len() as i64);
                    bytes.extend_from_slice(name.as_bytes());
                }
                let mut jrows: Vec<Value> = Vec::new();
                for row in &rows {
                    let mut jr: Vec<Value> = Vec::new();
                    for v in row {
                        match v {
                            SqlValue::Null => payload.extend_from_slice(&[0, 0, 0, 0]),
                            SqlValue::Integer(x) => payload.extend_from_slice(&[1, *x, 0, 0]),
                            SqlValue::Real(f) => {
                                let bits = f.to_bits();
                                payload.extend_from_slice(&[2, (bits >> 32) as i64, (bits & 0xffff_ffff) as i64, 0]);
                            }
                            SqlValue::Text(s) => {
                                payload.extend_from_slice(&[3, bytes.len() as i64, s.len() as i64, 0]);
                                bytes.extend_from_slice(s.as_bytes());
                            }
                            SqlValue::Blob(b) => {
                                payload.extend_from_slice(&[4, bytes.len() as i64, b.len() as i64, 0]);
                                bytes.extend_from_slice(b);
                            }
                        }
                        if self.trace {
                            jr.push(self.typed_json(v));
                        }
                    }
                    if self.trace {
                        jrows.push(Value::Array(jr));
                    }
                }
                done(payload, bytes, json!({"columns": cols, "rows": jrows}))
            }
            Call::DbClose => {
                let ent = match self.conns.remove(&req.handle) {
                    Some(c) => c,
                    None => die("db_close on an unknown connection handle"),
                };
                self.stmts.retain(|_, s| s.conn != req.handle);
                ent.conn.flush_prepared_statement_cache();
                match ent.conn.close() {
                    Ok(()) => done(Vec::new(), Vec::new(), json!({})),
                    Err((_c, e)) => Err(sqlite_err(&e)),
                }
            }
            Call::Mkdtemp(prefix) => {
                let mut t = prefix.clone();
                t.extend_from_slice(b"XXXXXX");
                t.push(0);
                let ptr = unsafe { libc::mkdtemp(t.as_mut_ptr() as *mut libc::c_char) };
                if ptr.is_null() {
                    return Err(os_err(&io::Error::last_os_error()));
                }
                t.pop();
                let s = lossy(&t);
                self.temp_dirs.push(s.clone());
                done(vec![0, t.len() as i64], t, json!({"path": s}))
            }
            Call::Open { path, mode } => {
                let mut o = OpenOptions::new();
                o.write(true).create_new(true).mode(*mode);
                match o.open(path_of(path)) {
                    Ok(f) => {
                        let h = self.new_handle();
                        self.files.insert(h, FileEntry { file: f, path: lossy(path) });
                        done(vec![h], Vec::new(), json!({"handle": h}))
                    }
                    Err(e) => Err(os_err(&e)),
                }
            }
            Call::Spawn(s) => self.do_spawn(req, s),
            Call::Observe { deadline } => self.do_observe(req, *deadline),
            Call::Kill { signal } => {
                let ch = match self.children.get(&req.handle) {
                    Some(c) => c,
                    None => die("kill on an unknown child handle"),
                };
                if ch.ended.load(Ordering::SeqCst) || ch.pid <= 0 {
                    return done(vec![0], Vec::new(), json!({"sent": 0}));
                }
                let r = unsafe { libc::kill(-ch.pid, *signal) };
                if r != 0 {
                    return Err(os_err(&io::Error::last_os_error()));
                }
                done(vec![1], Vec::new(), json!({"sent": 1}))
            }
            Call::Close => {
                let ent = match self.files.remove(&req.handle) {
                    Some(f) => f,
                    None => die("close on an unknown file handle"),
                };
                let fd = ent.file.into_raw_fd();
                let r = unsafe { libc::close(fd) };
                if r != 0 {
                    return Err(os_err(&io::Error::last_os_error()));
                }
                done(Vec::new(), Vec::new(), json!({}))
            }
            Call::Clock { wall } => {
                let ms = if *wall {
                    match self.now_ms {
                        Some(n) => n,
                        None => SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
                    }
                } else {
                    self.mono()
                };
                done(vec![ms], Vec::new(), json!({"ms": ms}))
            }
        }
    }

    fn do_spawn(&mut self, req: &Req, s: &SpawnSpec) -> Result<Done, ErrInfo> {
        let dup = match self.files.get(&req.handle) {
            Some(f) => {
                if s.pipe {
                    None
                } else {
                    Some(f.file.try_clone().map_err(|e| os_err(&e))?)
                }
            }
            None => die("spawn: unknown output file handle"),
        };
        let mut cmd = Command::new(OsStr::from_bytes(&s.binary));
        for a in &s.argv {
            cmd.arg(OsStr::from_bytes(a));
        }
        cmd.current_dir(OsStr::from_bytes(&s.cwd));
        if s.env_mode == 1 {
            cmd.env_clear();
        }
        for (k, v) in &s.env {
            cmd.env(OsStr::from_bytes(k), OsStr::from_bytes(v));
        }
        cmd.stdin(Stdio::null());
        cmd.stderr(Stdio::null());
        match dup {
            Some(f) => {
                cmd.stdout(Stdio::from(f));
            }
            None => {
                cmd.stdout(Stdio::piped());
            }
        }
        if s.detached {
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        let (tx, rx) = mpsc::channel::<ChildEv>();
        let ended = Arc::new(AtomicBool::new(false));
        let binary = lossy(&s.binary);
        match cmd.spawn() {
            Ok(mut child) => {
                let pid = child.id() as i32;
                if let Some(mut out) = child.stdout.take() {
                    std::thread::spawn(move || {
                        let mut buf = vec![0u8; 65536];
                        loop {
                            match out.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {}
                            }
                        }
                    });
                }
                let flag = ended.clone();
                std::thread::spawn(move || {
                    let r = child.wait();
                    flag.store(true, Ordering::SeqCst);
                    let ev = match r {
                        Ok(st) => ChildEv::Close { code: st.code(), signal: st.signal() },
                        Err(e) => ChildEv::Error(os_err(&e)),
                    };
                    let _ = tx.send(ev);
                });
                let h = self.new_handle();
                self.children.insert(h, ChildEntry { pid, rx, queue: VecDeque::new(), ended, binary });
                done(vec![h], Vec::new(), json!({"child": h}))
            }
            Err(e) => {
                let n = e.raw_os_error();
                if n == Some(libc::ENOENT) || n == Some(libc::EACCES) {
                    // Node reports a missing/unexecutable program asynchronously: spawn succeeds, then an error event
                    let _ = tx.send(ChildEv::Error(os_err(&e)));
                    ended.store(true, Ordering::SeqCst);
                    let h = self.new_handle();
                    self.children.insert(h, ChildEntry { pid: 0, rx, queue: VecDeque::new(), ended, binary });
                    done(vec![h], Vec::new(), json!({"child": h}))
                } else {
                    Err(os_err(&e))
                }
            }
        }
    }

    fn do_observe(&mut self, req: &Req, deadline: i64) -> Result<Done, ErrInfo> {
        let start = self.start;
        let mono = move || start.elapsed().as_millis() as i64;
        let ch = match self.children.get_mut(&req.handle) {
            Some(c) => c,
            None => die("observe on an unknown child handle"),
        };
        loop {
            while let Ok(ev) = ch.rx.try_recv() {
                ch.queue.push_back(ev);
            }
            if !ch.queue.is_empty() {
                break;
            }
            let now = mono();
            if now >= deadline {
                break;
            }
            let wait = std::cmp::min(deadline - now, 3_600_000) as u64;
            match ch.rx.recv_timeout(Duration::from_millis(wait)) {
                Ok(ev) => {
                    ch.queue.push_back(ev);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(wait)),
            }
        }
        let now = mono();
        let exited = if ch.ended.load(Ordering::SeqCst) { 1 } else { 0 };
        match ch.queue.pop_front() {
            Some(ChildEv::Close { code, signal }) => {
                let payload = vec![1, code.is_some() as i64, code.unwrap_or(0) as i64, signal.unwrap_or(0) as i64, now, exited];
                let result = json!({"eventKind": 1, "codePresent": code.is_some() as i64, "exitCode": code.unwrap_or(0), "signalNumber": signal.unwrap_or(0), "monoMs": now, "exited": exited});
                done(payload, Vec::new(), result)
            }
            Some(ChildEv::Error(e)) => {
                let payload = vec![2, 0, 0, 0, now, exited];
                let result = json!({"eventKind": 2, "codePresent": 0, "exitCode": 0, "signalNumber": 0, "monoMs": now, "exited": exited});
                Ok(Done { payload, bytes: Vec::new(), result, header_err: Some(e) })
            }
            None => {
                let payload = vec![0, 0, 0, 0, now, exited];
                let result = json!({"eventKind": 0, "codePresent": 0, "exitCode": 0, "signalNumber": 0, "monoMs": now, "exited": exited});
                done(payload, Vec::new(), result)
            }
        }
    }

    // ---- one request: schedule barriers, native call, trace, completion --------------------------------------------

    fn run_rules(&mut self, after: bool, req: &Req, opn: &str, phase: Option<&str>, key: &Option<String>, occ: u64) -> Option<ErrInfo> {
        for i in 0..self.rules.len() {
            if self.rules[i].after != after || !self.rule_hits(i, opn, phase, key, occ) {
                continue;
            }
            let ev = if after { "barrier-after-result-before-delivery" } else { "barrier-before-call" };
            let args = json!({"phase": phase, "path": key});
            self.emit_simple(ev, req, opn, args);
            let acts = self.rules[i].actions.clone();
            for a in &acts {
                let (kind, path) = a.label();
                self.emit_simple("schedule", req, opn, json!({"kind": kind, "path": path}));
                // `@N` is also resolved in action paths (as in the reference harness); the trace keeps the path as written
                let real = match a.resolved(&|p| self.resolve_path(p)) {
                    Some(x) => x,
                    None => die(&format!("schedule action {}: {} names a temp directory that does not exist yet", kind, path)),
                };
                if let Err(e) = run_action(&real) {
                    die(&format!("schedule action failed: {}", e));
                }
            }
            if let Some(e) = self.rules[i].error.clone() {
                return Some(e);
            }
        }
        None
    }

    fn execute(&mut self, r: &[i64], words: &[i64], lit: &[u8]) -> (Vec<i64>, Vec<u8>) {
        let req = parse_req(r, words);
        if req.id <= self.last_id {
            die(&format!("request id {} does not increase (last {})", req.id, self.last_id));
        }
        self.last_id = req.id;
        let call = parse_call(&req, lit);
        let (opn, key, phase, args) = self.describe(&call, &req);
        let occ = {
            let k = (opn.to_string(), phase.unwrap_or("").to_string(), key.clone().unwrap_or_default());
            let e = self.occ.entry(k).or_insert(0);
            *e += 1;
            *e
        };
        let handle = if req.handle >= 1 { json!(req.handle) } else { Value::Null };
        self.emit("issue", json!(req.id), json!(opn), json!(req.stage), handle.clone(), args, Value::Null, Value::Null);
        let before = self.run_rules(false, &req, opn, phase, &key, occ);
        let mut outcome: Result<Done, ErrInfo> = match before {
            Some(e) => Err(e),
            None if call_has_nul(&call) => Err(invalid_arg_err()),
            None => {
                let res = self.exec_call(&call, &req);
                if res.is_ok() {
                    match self.run_rules(true, &req, opn, phase, &key, occ) {
                        Some(e) => Err(e),
                        None => res,
                    }
                } else {
                    res
                }
            }
        };
        // a native success replaced by an injected error leaves its host object in place (teardown owns it)
        let (rjson, ejson) = if !self.trace {
            (Value::Null, Value::Null)
        } else {
            match &outcome {
            Ok(d) => (d.result.clone(), d.header_err.as_ref().map(|e| json!({"class": e.class, "code": e.code, "native": e.native})).unwrap_or(Value::Null)),
            Err(e) => (Value::Null, json!({"class": e.class, "code": e.code, "native": e.native})),
            }
        };
        self.emit("complete", json!(req.id), json!(opn), json!(req.stage), handle, Value::Null, rjson, ejson);
        build_completion(&req, &mut outcome)
    }

    fn teardown(&mut self) {
        let mut killed = 0;
        for (_, c) in self.children.iter() {
            if !c.ended.load(Ordering::SeqCst) && c.pid > 0 {
                unsafe {
                    libc::kill(-c.pid, libc::SIGKILL);
                }
                killed += 1;
            }
        }
        if killed > 0 {
            eprintln!("{{\"teardown\":{{\"killedChildren\":{}}}}}", killed);
        }
        self.children.clear();
        self.stmts.clear();
        self.conns.clear();
        self.files.clear();
    }
}

fn build_completion(req: &Req, outcome: &mut Result<Done, ErrInfo>) -> (Vec<i64>, Vec<u8>) {
    let mut ev = vec![0i64; 16];
    ev[0] = 1;
    ev[1] = req.id;
    ev[2] = req.op;
    let mut bytes: Vec<u8>;
    let payload: Vec<i64>;
    let err: Option<ErrInfo>;
    match outcome {
        Ok(d) => {
            ev[3] = 0;
            bytes = std::mem::take(&mut d.bytes);
            payload = std::mem::take(&mut d.payload);
            err = d.header_err.clone();
        }
        Err(e) => {
            ev[3] = 1;
            bytes = Vec::new();
            payload = Vec::new();
            err = Some(e.clone());
        }
    }
    match &err {
        Some(e) => {
            ev[4] = e.class;
            ev[5] = e.native;
            ev[8] = bytes.len() as i64;
            ev[9] = e.code.len() as i64;
            bytes.extend_from_slice(e.code.as_bytes());
            ev[10] = bytes.len() as i64;
            ev[11] = e.msg.len() as i64;
            bytes.extend_from_slice(e.msg.as_bytes());
        }
        None => {
            ev[8] = -1;
            ev[10] = -1;
        }
    }
    ev[6] = payload.len() as i64;
    ev[7] = bytes.len() as i64;
    ev.extend_from_slice(&payload);
    (ev, bytes)
}

// ---------------------------------------------------------------------------------------------------------------
// driver

fn run_one<S: Sol>(sol: &S, v: &Value) -> String {
    let cfg = build_cfg(v);
    let mut host = Host::new(&cfg, v);
    let begin = sol.begin(&cfg.words, &cfg.text);
    let mut ctl: Vec<i64>;
    let mut first: Option<Vec<i64>> = None;
    if begin.len() == CTL_WORDS {
        ctl = begin;
    } else if begin.len() >= HEADER && begin[0] == MAGIC_STEP {
        ctl = vec![0i64; CTL_WORDS];
        first = Some(begin);
    } else {
        die(&format!("opencode_begin returned {} words; expected 64 (ctl) or a step packet", begin.len()));
    }
    let mut banks = Banks::new();
    let mut event = empty_event();
    let mut comp: Vec<u8> = Vec::new();
    let mut steps: u64 = 0;
    loop {
        steps += 1;
        if steps > STEP_GUARD {
            die("step guard exceeded: the importer does not terminate");
        }
        let pkt = match first.take() {
            Some(p) => p,
            None => sol.step(&cfg.words, &cfg.text, &ctl, &banks, &event, &comp),
        };
        let st = parse_step(&pkt);
        ctl = st.ctl.clone();
        // patches first, in order, reading the event/completion this step was given
        apply_patches(&mut banks, &st, &event, &comp);
        match st.action {
            1 => {
                let out = render_result(&st.result);
                host.teardown();
                return out;
            }
            2 => {
                event = empty_event();
                comp = Vec::new();
            }
            _ => {
                let (ev, cb) = host.execute(&st.requests[0], &st.words, &st.bytes);
                event = ev;
                comp = cb;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// result packet rendering (DESIGN section 5)

const MODE_NAMES: [&str; 6] = ["list", "read", "rows", "dataDir", "folderKey", "plan"];
const STAGE_NAMES: [&str; 29] = [
    "input", "db-parent", "db-lstat", "db-realpath", "db-open", "schema-session", "schema-message", "select", "count-prepare", "count",
    "db-close", "read-admission", "list-clock", "temp-create", "output-open", "spawn", "wait", "term", "kill", "give-up", "output-close",
    "output-stat", "output-read", "cleanup", "parse", "shape", "convert", "plan", "result",
];
const CLASS_NAMES: [&str; 5] = ["none", "EngineImportError", "RawOsError", "CoreError", "OtherError"];
const CODE_NAMES: [&str; 7] = ["none", "not-found", "read-failed", "engine-missing", "timeout", "InputTooLarge", "InternalInvariant"];
const PLAN_STATUS: [&str; 4] = ["", "pending", "in_progress", "completed"];

fn ref_ok(tl: usize, s: i64, n: i64) -> bool {
    if s == -1 {
        return n == 0;
    }
    if s < 0 || n < 0 {
        return false;
    }
    (s as i128 + n as i128) <= tl as i128
}

// Some(string) for a present ref (valid UTF-8 at scalar boundaries), None for the absent ref
fn rstr<'a>(text: &'a [u8], s: i64, n: i64, what: &str) -> Option<&'a str> {
    if !ref_ok(text.len(), s, n) {
        die(&format!("result ref {} out of range", what));
    }
    if s == -1 {
        return None;
    }
    match std::str::from_utf8(&text[s as usize..(s + n) as usize]) {
        Ok(t) => Some(t),
        Err(_) => die(&format!("result ref {} is not UTF-8 at scalar boundaries", what)),
    }
}

fn jopt(x: Option<&str>) -> String {
    match x {
        Some(t) => qs(t),
        None => "null".to_string(),
    }
}

fn need<'a>(x: Option<&'a str>, what: &str) -> &'a str {
    match x {
        Some(t) => t,
        None => die(&format!("result {} must be present", what)),
    }
}

fn zero_one(x: i64, what: &str) -> bool {
    if x != 0 && x != 1 {
        die(&format!("result {} must be 0 or 1", what));
    }
    x == 1
}

pub fn render_result(p: &[i64]) -> String {
    if p.len() < 40 {
        die("result packet too short");
    }
    if p[0] != MAGIC_RESULT {
        die("result magic mismatch");
    }
    if p[1] != 1 {
        die("result version must be 1");
    }
    let status = p[2];
    if status != 0 && status != 1 {
        die("result status out of range");
    }
    if !(0..=5).contains(&p[3]) {
        die("result mode out of range");
    }
    let mode = p[3] as usize;
    let (n, w, k, t) = (p[4], p[5], p[6], p[7]);
    if n < 0 || w < 0 || k < 0 || t < 0 {
        die("result negative count");
    }
    let total: i128 = 40 + n as i128 * w as i128 + 3 * k as i128 + t as i128;
    if total != p.len() as i128 {
        die(&format!("result length {} does not match 40+N*W+3K+T = {}", p.len(), total));
    }
    if p[29..40].iter().any(|&x| x != 0) {
        die("result reserved words must be 0");
    }
    let omitted = zero_one(p[8], "omittedBefore");
    let plan_present = zero_one(p[9], "planPresent");
    let width: i64 = match mode {
        0 | 1 => 10,
        2 => 7,
        _ => 0,
    };
    if width == 0 && n != 0 {
        die("result rows are not allowed for this mode");
    }
    if n > 0 && w != width {
        die("result row width does not match the mode");
    }
    if n == 0 && w != width && w != 0 {
        die("result row width does not match the mode");
    }
    let (n, w, k, t) = (n as usize, w as usize, k as usize, t as usize);
    let rows_at = 40;
    let plan_at = rows_at + n * w;
    let text_at = plan_at + 3 * k;
    let mut text: Vec<u8> = Vec::with_capacity(t);
    for &x in &p[text_at..] {
        if !(0..=255).contains(&x) {
            die("result text value outside 0..255");
        }
        text.push(x as u8);
    }
    let title = rstr(&text, p[10], p[11], "title");
    let model = rstr(&text, p[12], p[13], "model");
    let value = rstr(&text, p[14], p[15], "value");
    let plan_atstr = rstr(&text, p[16], p[17], "plan.at");
    let more = p[18];
    let (stage, class, code, native) = (p[19], p[20], p[21], p[22]);
    let raw_code = rstr(&text, p[23], p[24], "error.rawCode");
    let err_name = rstr(&text, p[25], p[26], "error.name");
    let detail = rstr(&text, p[27], p[28], "error.detail");
    if !(0..=28).contains(&stage) || !(0..=4).contains(&class) || !(0..=6).contains(&code) || native < 0 || more < 0 {
        die("result error/plan scalar out of range");
    }
    let mname = MODE_NAMES[mode];
    if status == 1 {
        if class == 0 {
            die("error result has class none");
        }
        if n != 0 || w != 0 || k != 0 || omitted || plan_present || more != 0 || title.is_some() || model.is_some() || value.is_some() || plan_atstr.is_some() {
            die("error result carries rows, plan, value or metadata");
        }
        return format!(
            "{{\"status\":\"error\",\"mode\":{},\"error\":{{\"class\":{},\"code\":{},\"name\":{},\"detail\":{},\"stage\":{},\"rawCode\":{},\"native\":{}}}}}",
            qs(mname),
            qs(CLASS_NAMES[class as usize]),
            if code == 0 { "null".to_string() } else { qs(CODE_NAMES[code as usize]) },
            jopt(err_name),
            jopt(detail),
            qs(STAGE_NAMES[stage as usize]),
            jopt(raw_code),
            native
        );
    }
    if stage != 0 || class != 0 || code != 0 || native != 0 || raw_code.is_some() || err_name.is_some() || detail.is_some() {
        die("success result carries error fields");
    }
    if omitted && mode != 1 {
        die("omittedBefore is only for read");
    }
    if (title.is_some() || model.is_some()) && mode != 1 {
        die("title/model are only for read");
    }
    if value.is_some() != (mode == 3 || mode == 4) {
        die("result value is present exactly for dataDir/folderKey");
    }
    if plan_present && mode != 1 && mode != 5 {
        die("a plan is only for read/plan");
    }
    if plan_present {
        if !(1..=30).contains(&k) {
            die("a present plan needs 1..=30 items");
        }
    } else if k != 0 || more != 0 || plan_atstr.is_some() {
        die("an absent plan has no items, more or at");
    }
    let mut out_rows: Vec<String> = Vec::with_capacity(n);
    for j in 0..n {
        let r = &p[rows_at + j * w..rows_at + (j + 1) * w];
        let s = match mode {
            0 => {
                let thread = need(rstr(&text, r[0], r[1], "list.thread"), "list.thread");
                let ttl = need(rstr(&text, r[2], r[3], "list.title"), "list.title");
                let upd = rstr(&text, r[4], r[5], "list.updatedAt");
                let has_msgs = zero_one(r[6], "list.messagesPresent");
                if (!has_msgs && r[7] != 0) || r[7] < 0 {
                    die("list row messages invalid");
                }
                let open = zero_one(r[8], "list.maybeOpen");
                if r[9] != 0 {
                    die("list row reserved word must be 0");
                }
                let mut s = format!("{{\"thread\":{},\"title\":{}", qs(thread), qs(ttl));
                if let Some(u) = upd {
                    s.push_str(&format!(",\"updatedAt\":{}", qs(u)));
                }
                if has_msgs {
                    s.push_str(&format!(",\"messages\":{}", r[7]));
                }
                if open {
                    s.push_str(",\"maybeOpen\":true");
                }
                s.push('}');
                s
            }
            1 => {
                let prompt = need(rstr(&text, r[0], r[1], "read.prompt"), "read.prompt");
                let reply = need(rstr(&text, r[2], r[3], "read.reply"), "read.reply");
                let created = rstr(&text, r[4], r[5], "read.createdAt");
                let completed = rstr(&text, r[6], r[7], "read.completedAt");
                if r[8] < 0 {
                    die("read row tools negative");
                }
                let clipped = zero_one(r[9], "read.clipped");
                let mut s = format!("{{\"prompt\":{},\"reply\":{}", qs(prompt), qs(reply));
                if let Some(c) = created {
                    s.push_str(&format!(",\"createdAt\":{}", qs(c)));
                }
                if let Some(c) = completed {
                    s.push_str(&format!(",\"completedAt\":{}", qs(c)));
                }
                s.push_str(&format!(",\"tools\":{}", r[8]));
                if clipped {
                    s.push_str(",\"clipped\":true");
                }
                s.push('}');
                s
            }
            _ => {
                let id = need(rstr(&text, r[0], r[1], "rows.id"), "rows.id");
                let dir = need(rstr(&text, r[2], r[3], "rows.directory"), "rows.directory");
                let ttl = need(rstr(&text, r[4], r[5], "rows.title"), "rows.title");
                format!("{{\"id\":{},\"directory\":{},\"title\":{},\"updatedMs\":{}}}", qs(id), qs(dir), qs(ttl), r[6])
            }
        };
        out_rows.push(s);
    }
    let plan_json = if plan_present {
        let mut items: Vec<String> = Vec::with_capacity(k);
        for i in 0..k {
            let it = &p[plan_at + 3 * i..plan_at + 3 * i + 3];
            let txt = need(rstr(&text, it[0], it[1], "plan.item.text"), "plan.item.text");
            if !(0..=3).contains(&it[2]) {
                die("plan item status out of range");
            }
            if it[2] == 0 {
                items.push(format!("{{\"text\":{}}}", qs(txt)));
            } else {
                items.push(format!("{{\"text\":{},\"status\":{}}}", qs(txt), qs(PLAN_STATUS[it[2] as usize])));
            }
        }
        let mut s = String::from("{");
        if let Some(a) = plan_atstr {
            s.push_str(&format!("\"at\":{},", qs(a)));
        }
        s.push_str(&format!("\"items\":[{}]", items.join(",")));
        if more > 0 {
            s.push_str(&format!(",\"more\":{}", more));
        }
        s.push('}');
        Some(s)
    } else {
        None
    };
    match mode {
        0 | 2 => format!("{{\"status\":\"ok\",\"mode\":{},\"rows\":[{}]}}", qs(mname), out_rows.join(",")),
        1 => {
            let mut s = format!("{{\"status\":\"ok\",\"mode\":\"read\",\"turns\":[{}],\"omittedBefore\":{}", out_rows.join(","), if omitted { "true" } else { "false" });
            if let Some(t) = title {
                s.push_str(&format!(",\"title\":{}", qs(t)));
            }
            if let Some(m) = model {
                s.push_str(&format!(",\"model\":{}", qs(m)));
            }
            if let Some(pl) = &plan_json {
                s.push_str(&format!(",\"lastPlan\":{}", pl));
            }
            s.push('}');
            s
        }
        3 | 4 => format!("{{\"status\":\"ok\",\"mode\":{},\"value\":{}}}", qs(mname), qs(value.unwrap_or(""))),
        _ => match &plan_json {
            Some(pl) => format!("{{\"status\":\"ok\",\"mode\":\"plan\",\"present\":true,\"value\":{}}}", pl),
            None => "{\"status\":\"ok\",\"mode\":\"plan\",\"present\":false}".to_string(),
        },
    }
}

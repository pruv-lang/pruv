// Grok import primitive host (L3-6, DESIGN.md sections 1, 2, 4, 6). Included by host/src/lib.rs as crate `grok_host`.
// Only plain `//` comments here (include! forbids inner doc comments).
//
// The host makes no import decisions: it builds cfg, calls the generated entries (through the `Sol` trait), applies
// storage patches to the opaque banks, executes the 13 primitives, delivers completions in arrival order and
// renders the final result packet as one JSON line.
//
// Opcode table (request word 2):
//    1 readdir            path1; flags 1 = withFileTypes (kinds), 0 = names only
//    2 lstat              path1
//    3 realpath           path1                      (completion data = path)
//    4 open-read          path1; flags 1 = O_NOFOLLOW (returns a handle id, starting at 1)
//    5 fstat              handle
//    6 pread              handle, offset, count      (completion data = bytes read)
//    7 close              handle
//    8 mkdir              path1; flags 1 = recursive; mode
//    9 rename             path1 -> path2
//   10 rm                 path1; recursive + force
//   11 exclusive create   path1; O_CREAT|O_EXCL|O_WRONLY, mode (returns a handle id)
//   12 pwrite             handle, offset, data
//   13 entropy            count (6) bytes of OS entropy; test mode replays schedule entropyHex in order
//
// Exit codes: 0 ok (all config lines processed), 3 host protocol error (an invalid step/result packet, a bad
// config or schedule, a readdir schedule name set that differs from the real one, a failed schedule action, an
// await with nothing deliverable). On exit 3 one line {"status":"host-protocol-error","detail":...} goes to stdout.
//
// Config line (JSONL): {mode, sourceHome, folder, thread, targetHome, binary, sourceEngine, smallPath,
//   smallHomeReal, cleanupPath, rpcMs, schedule, trace}. Absent fields become (-1,0) refs.
// Assumption: grok_begin returns either the initial 256-word ctl, or a complete step packet (magic 0x47525331).

use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

const MAGIC: i64 = 0x47525331;
const CTL_WORDS: usize = 256;
const HEADER: usize = 268;
const MAX_BANK: i64 = 1 << 31;
const MAX_COUNT: i64 = 1 << 26;

#[cfg(target_os = "macos")]
const O_NOFOLLOW: i32 = 0x100;
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
const O_NOFOLLOW: i32 = 0o100000;
#[cfg(all(target_os = "linux", not(any(target_arch = "aarch64", target_arch = "arm"))))]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const O_NOFOLLOW: i32 = 0;

// ---------------------------------------------------------------------------------------------------------------
// public surface

pub struct Banks {
    pub dir: Vec<i64>,
    pub rows: Vec<i64>,
    pub text: Vec<u8>,
    pub small: [Vec<u8>; 4],
    pub history: Vec<u8>,
}

impl Banks {
    fn new() -> Banks {
        Banks { dir: Vec::new(), rows: Vec::new(), text: Vec::new(), small: [Vec::new(), Vec::new(), Vec::new(), Vec::new()], history: Vec::new() }
    }
    fn len(&self, b: usize) -> usize {
        match b {
            0 => self.dir.len(),
            1 => self.rows.len(),
            2 => self.text.len(),
            3..=6 => self.small[b - 3].len(),
            _ => self.history.len(),
        }
    }
    fn resize(&mut self, b: usize, n: usize) {
        match b {
            0 => self.dir.resize(n, 0),
            1 => self.rows.resize(n, 0),
            2 => self.text.resize(n, 0),
            3..=6 => self.small[b - 3].resize(n, 0),
            _ => self.history.resize(n, 0),
        }
    }
    fn bytes_mut(&mut self, b: usize) -> &mut Vec<u8> {
        match b {
            2 => &mut self.text,
            3..=6 => &mut self.small[b - 3],
            _ => &mut self.history,
        }
    }
    fn put_words(&mut self, b: usize, off: usize, s: &[i64]) -> Result<(), String> {
        match b {
            0 => self.dir[off..off + s.len()].copy_from_slice(s),
            1 => self.rows[off..off + s.len()].copy_from_slice(s),
            _ => {
                for &x in s {
                    if !(0..=255).contains(&x) {
                        return Err(format!("patch copies word {} into byte bank {}", x, b));
                    }
                }
                let d = self.bytes_mut(b);
                for (i, &x) in s.iter().enumerate() {
                    d[off + i] = x as u8;
                }
            }
        }
        Ok(())
    }
    fn put_bytes(&mut self, b: usize, off: usize, s: &[u8]) {
        match b {
            0 => {
                for (i, &x) in s.iter().enumerate() {
                    self.dir[off + i] = x as i64;
                }
            }
            1 => {
                for (i, &x) in s.iter().enumerate() {
                    self.rows[off + i] = x as i64;
                }
            }
            _ => self.bytes_mut(b)[off..off + s.len()].copy_from_slice(s),
        }
    }
}

// The generated module as seen by the host. The harness implements this with the `sol` functions.
pub trait Sol {
    fn begin(&self, cfg: &[i64], cfg_text: &[u8]) -> Vec<i64>;
    fn step(&self, cfg: &[i64], cfg_text: &[u8], ctl: &[i64], banks: &Banks, event: &[i64], completion: &[u8]) -> Vec<i64>;
    fn load(&self, binary: &str, home: &str, folder: &str, thread: &str, timeout_ms: i64) -> Vec<i64>;
    fn caller_cleanup(&self, path: &str) -> Vec<i64>;
}

pub fn run<S: Sol>(sol: &S) {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => die(&format!("stdin: {}", e)),
        };
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => die(&format!("config is not JSON: {}", e)),
        };
        let out = run_one(sol, &v);
        println!("{}", out);
        let _ = io::stdout().flush();
    }
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
    let mut out = Vec::with_capacity(b.len() / 2);
    let val = |c: u8| -> Result<u8, String> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("bad hex digit".to_string()),
        }
    };
    let mut i = 0;
    while i < b.len() {
        out.push(val(b[i])? * 16 + val(b[i + 1])?);
        i += 2;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------
// sha256 (trace digests; no extra crates)

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::new();
    for x in h.iter() {
        out.push_str(&format!("{:08x}", x));
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// errno tables

fn errno_table() -> Vec<(&'static str, i32)> {
    let mac = cfg!(target_os = "macos");
    vec![
        ("EPERM", 1),
        ("ENOENT", 2),
        ("EIO", 5),
        ("ENXIO", 6),
        ("EBADF", 9),
        ("ENOMEM", 12),
        ("EACCES", 13),
        ("EBUSY", 16),
        ("EEXIST", 17),
        ("EXDEV", 18),
        ("ENODEV", 19),
        ("ENOTDIR", 20),
        ("EISDIR", 21),
        ("EINVAL", 22),
        ("ENFILE", 23),
        ("EMFILE", 24),
        ("ETXTBSY", 26),
        ("EFBIG", 27),
        ("ENOSPC", 28),
        ("EROFS", 30),
        ("EPIPE", 32),
        ("EAGAIN", if mac { 35 } else { 11 }),
        ("ENAMETOOLONG", if mac { 63 } else { 36 }),
        ("ENOTEMPTY", if mac { 66 } else { 39 }),
        ("ELOOP", if mac { 62 } else { 40 }),
    ]
}

fn errno_of(name: &str) -> Option<i32> {
    errno_table().into_iter().find(|e| e.0 == name).map(|e| e.1)
}

fn errno_name(n: i32) -> String {
    match errno_table().into_iter().find(|e| e.1 == n) {
        Some(e) => e.0.to_string(),
        None => "UNKNOWN".to_string(),
    }
}

fn errno_class(n: i32) -> i64 {
    match n {
        2 => 1,
        13 => 2,
        17 => 3,
        _ => 4,
    }
}

fn op_of(name: &str) -> Option<i64> {
    Some(match name {
        "readdir" => 1,
        "lstat" => 2,
        "realpath" => 3,
        "open" | "open-read" => 4,
        "fstat" => 5,
        "pread" | "read" => 6,
        "close" => 7,
        "mkdir" => 8,
        "rename" => 9,
        "rm" | "remove" => 10,
        "create" | "exclusive-create" | "open-write" => 11,
        "pwrite" | "write" => 12,
        "entropy" => 13,
        _ => return None,
    })
}

fn trace_op(op: i64) -> &'static str {
    match op {
        4 => "open",
        6 => "read",
        12 => "write",
        n => op_name(n),
    }
}

fn kind_name(k: i64) -> &'static str {
    match k {
        1 => "file",
        2 => "dir",
        3 => "symlink",
        4 => "fifo",
        5 => "socket",
        6 => "char",
        7 => "block",
        _ => "other",
    }
}

fn op_name(op: i64) -> &'static str {
    match op {
        1 => "readdir",
        2 => "lstat",
        3 => "realpath",
        4 => "open-read",
        5 => "fstat",
        6 => "pread",
        7 => "close",
        8 => "mkdir",
        9 => "rename",
        10 => "rm",
        11 => "create",
        12 => "pwrite",
        13 => "entropy",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------------------------------------------
// config

struct Cfg {
    words: Vec<i64>,
    text: Vec<u8>,
    mode: i64,
    src: String,
    dst: String,
    binary: String,
    folder: String,
    thread: String,
    home: String,
    rpc: i64,
    trace: bool,
}

fn opt_str(v: &serde_json::Value, k: &str) -> Option<String> {
    match v.get(k) {
        None => None,
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
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

fn parse_cfg(v: &serde_json::Value) -> Cfg {
    let mode = match v.get("mode") {
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "list" => 0,
            "read" => 1,
            "copy" => 2,
            "load" => 3,
            "small-json" => 4,
            "caller-cleanup" => 5,
            _ => die(&format!("unknown mode {}", s)),
        },
        Some(serde_json::Value::Number(n)) => match n.as_i64() {
            Some(k) if (0..=5).contains(&k) => k,
            _ => die("mode number out of range"),
        },
        _ => die("config mode missing"),
    };
    let rpc = match v.get("rpcMs") {
        None | Some(serde_json::Value::Null) => 20000,
        Some(x) => match x.as_i64() {
            Some(k) => k,
            None => die("rpcMs must be an integer"),
        },
    };
    let trace = v.get("trace").and_then(|x| x.as_bool()).unwrap_or(false);
    let source_home = opt_str(v, "sourceHome");
    let folder = opt_str(v, "folder");
    let thread = opt_str(v, "thread");
    let target_home = opt_str(v, "targetHome");
    let binary = opt_str(v, "binary");
    let engine = opt_str(v, "sourceEngine");
    let small_path = opt_str(v, "smallPath");
    let small_home = opt_str(v, "smallHomeReal");
    let cleanup = opt_str(v, "cleanupPath");
    let mut words = vec![0i64; 24];
    let mut text: Vec<u8> = Vec::new();
    words[0] = 1;
    words[1] = mode;
    words[2] = rpc;
    put_ref(&mut words, &mut text, 4, &source_home);
    put_ref(&mut words, &mut text, 6, &folder);
    put_ref(&mut words, &mut text, 8, &thread);
    put_ref(&mut words, &mut text, 10, &target_home);
    put_ref(&mut words, &mut text, 12, &binary);
    put_ref(&mut words, &mut text, 14, &engine);
    put_ref(&mut words, &mut text, 16, &small_path);
    put_ref(&mut words, &mut text, 18, &small_home);
    put_ref(&mut words, &mut text, 20, &cleanup);
    Cfg {
        words,
        text,
        mode,
        src: source_home.unwrap_or_default(),
        dst: target_home.clone().unwrap_or_default(),
        binary: binary.unwrap_or_default(),
        folder: folder.unwrap_or_default(),
        thread: thread.unwrap_or_default(),
        home: target_home.unwrap_or_default(),
        rpc,
        trace,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// schedule

#[derive(Clone)]
enum Action {
    ReplaceFile { path: String, bytes: Vec<u8>, mode: Option<u32> },
    ReplaceSymlink { path: String, target: String },
    Mkdir { path: String, mode: Option<u32> },
    Unlink { path: String },
    Append { path: String, bytes: Vec<u8> },
    Truncate { path: String, size: u64 },
}

struct Matcher {
    op: i64,
    path: Option<String>,
    occurrence: u64,
    after: bool,
}

impl Matcher {
    // `occ` is the call's occurrence among calls with the same (op, path) (reference `begun`: r.match.occurrence === occ)
    fn hit(&self, op: i64, path: &str, occ: u64) -> bool {
        if op != self.op {
            return false;
        }
        if let Some(p) = &self.path {
            if p != path {
                return false;
            }
        }
        occ == self.occurrence
    }
}

// releaseWhen of a hold (reference condMet): either {held: n} or {event, op, path?, occurrence?}
struct Release {
    held: Option<usize>,
    event: String,
    op: i64,
    path: Option<String>,
    occurrence: u64,
}

impl Release {
    fn met(&self, seen: &HashMap<(String, i64, Option<String>), u64>, held_count: usize) -> bool {
        if let Some(n) = self.held {
            return held_count >= n;
        }
        let n = seen.get(&(self.event.clone(), self.op, self.path.clone())).copied().unwrap_or(0);
        n >= self.occurrence
    }
}

struct Rule {
    m: Matcher,
    actions: Vec<Action>,
    inject: Option<(i32, String)>,
    hold: Option<usize>,
}

struct Hold {
    release: Release,
    released: bool,
    buf: Vec<Comp>,
}

struct Sched {
    readdir: HashMap<String, Vec<Vec<u8>>>,
    entropy: Vec<Vec<u8>>,
    test_entropy: bool,
    rules: Vec<Rule>,
    holds: Vec<Hold>,
}

fn bytes_spec(v: &serde_json::Value) -> Result<Vec<u8>, String> {
    if let Some(s) = v.get("utf8").and_then(|x| x.as_str()) {
        return Ok(s.as_bytes().to_vec());
    }
    if let Some(s) = v.get("hex").and_then(|x| x.as_str()) {
        return unhex(s);
    }
    Err("bytes spec needs utf8 or hex".to_string())
}

fn parse_matcher(v: &serde_json::Value, sub: &dyn Fn(&str) -> String) -> Matcher {
    let opn = match v.get("op").and_then(|x| x.as_str()) {
        Some(s) => s,
        None => die("schedule match needs op"),
    };
    let op = match op_of(opn) {
        Some(o) => o,
        None => die(&format!("schedule op {} unknown", opn)),
    };
    let path = v.get("path").and_then(|x| x.as_str()).map(|s| sub(s));
    let occurrence = v.get("occurrence").and_then(|x| x.as_u64()).unwrap_or(1);
    let after = match v.get("barrier").and_then(|x| x.as_str()) {
        None | Some("before-call") => false,
        Some("after-result-before-delivery") => true,
        Some(b) => die(&format!("unknown barrier {}", b)),
    };
    Matcher { op, path, occurrence, after }
}

fn parse_release(v: &serde_json::Value, sub: &dyn Fn(&str) -> String) -> Release {
    let occurrence = v.get("occurrence").and_then(|x| x.as_u64()).unwrap_or(1);
    if let Some(n) = v.get("held").and_then(|x| x.as_u64()) {
        return Release { held: Some(n as usize), event: String::new(), op: 0, path: None, occurrence };
    }
    let opn = match v.get("op").and_then(|x| x.as_str()) {
        Some(s) => s,
        None => die("releaseWhen needs op or held"),
    };
    let op = match op_of(opn) {
        Some(o) => o,
        None => die(&format!("schedule op {} unknown", opn)),
    };
    // reference: a missing event never matches any emitted event (key "undefined|..."); keep that
    let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("undefined").to_string();
    let path = v.get("path").and_then(|x| x.as_str()).map(|s| sub(s));
    Release { held: None, event, op, path, occurrence }
}

fn action_bytes(a: &serde_json::Value, blobs: &HashMap<String, Vec<u8>>) -> Vec<u8> {
    if let Some(n) = a.get("blob").and_then(|x| x.as_str()) {
        return match blobs.get(n) {
            Some(b) => b.clone(),
            None => die(&format!("unknown blob {}", n)),
        };
    }
    if let Some(b) = a.get("bytes") {
        return match bytes_spec(b) {
            Ok(x) => x,
            Err(e) => die(&e),
        };
    }
    match bytes_spec(a) {
        Ok(x) => x,
        Err(e) => die(&e),
    }
}

fn parse_action(a: &serde_json::Value, blobs: &HashMap<String, Vec<u8>>, sub: &dyn Fn(&str) -> String) -> Action {
    let kind = match a.get("kind").and_then(|x| x.as_str()) {
        Some(k) => k,
        None => die("schedule action needs kind"),
    };
    let path = match a.get("path").and_then(|x| x.as_str()) {
        Some(p) => sub(p),
        None => die("schedule action needs path"),
    };
    let mode: Option<u32> = a.get("mode").and_then(|x| x.as_u64()).map(|m| m as u32);
    match kind {
        // exactly the reference harness kinds (reference-primitives.mjs runAction); any other is a fixture error
        "replace-file" => Action::ReplaceFile { path, bytes: action_bytes(a, blobs), mode },
        "symlink" => {
            let target = match a.get("target").and_then(|x| x.as_str()) {
                Some(t) => sub(t),
                None => die("symlink action needs target"),
            };
            Action::ReplaceSymlink { path, target }
        }
        "mkdir" => Action::Mkdir { path, mode },
        "unlink" => Action::Unlink { path },
        "append" => Action::Append { path, bytes: action_bytes(a, blobs) },
        "truncate" => Action::Truncate { path, size: a.get("size").and_then(|x| x.as_u64()).unwrap_or(0) },
        _ => die(&format!("unknown action kind {}", kind)),
    }
}

fn parse_sched(v: Option<&serde_json::Value>, src: &str, dst: &str) -> Sched {
    let mut s = Sched { readdir: HashMap::new(), entropy: Vec::new(), test_entropy: false, rules: Vec::new(), holds: Vec::new() };
    let v = match v {
        Some(x) if x.is_object() => x,
        Some(serde_json::Value::Null) | None => return s,
        _ => die("schedule must be an object"),
    };
    let src = src.to_string();
    let dst = dst.to_string();
    let sub = move |t: &str| -> String { t.replace("$SRC", &src).replace("$DST", &dst) };
    let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
    if let Some(o) = v.get("blobs").and_then(|x| x.as_object()) {
        for (k, b) in o.iter() {
            match bytes_spec(b) {
                Ok(x) => {
                    blobs.insert(k.clone(), x);
                }
                Err(e) => die(&format!("blob {}: {}", k, e)),
            }
        }
    }
    if let Some(a) = v.get("readdir").and_then(|x| x.as_array()) {
        for e in a {
            let p = match e.get("path").and_then(|x| x.as_str()) {
                Some(p) => sub(p),
                None => die("readdir schedule entry needs path"),
            };
            let names = match e.get("names").and_then(|x| x.as_array()) {
                Some(n) => n,
                None => die("readdir schedule entry needs names"),
            };
            let mut list = Vec::new();
            for n in names {
                match n.as_str() {
                    Some(t) => list.push(t.as_bytes().to_vec()),
                    None => die("readdir schedule name must be a string"),
                }
            }
            s.readdir.insert(p, list);
        }
    }
    if let Some(a) = v.get("entropyHex").and_then(|x| x.as_array()) {
        s.test_entropy = true;
        for e in a {
            let h = match e.as_str() {
                Some(h) => h,
                None => die("entropyHex entries must be strings"),
            };
            match unhex(h) {
                Ok(b) => s.entropy.push(b),
                Err(m) => die(&format!("entropyHex: {}", m)),
            }
        }
    }
    if let Some(a) = v.get("rules").and_then(|x| x.as_array()) {
        for r in a {
            let m = match r.get("match") {
                Some(m) => parse_matcher(m, &sub),
                None => die("schedule rule needs match"),
            };
            let mut actions = Vec::new();
            if let Some(acts) = r.get("actions").and_then(|x| x.as_array()) {
                for a in acts {
                    actions.push(parse_action(a, &blobs, &sub));
                }
            }
            let inject = match r.get("error") {
                None => None,
                Some(e) => {
                    let code = match e.get("code").and_then(|x| x.as_str()) {
                        Some(c) => c,
                        None => die("error injection needs code"),
                    };
                    match errno_of(code) {
                        Some(n) => Some((n, code.to_string())),
                        None => die(&format!("unknown error code {}", code)),
                    }
                }
            };
            let mut hold = None;
            if r.get("hold").is_some() {
                let rel = match r.get("releaseWhen") {
                    Some(x) => parse_release(x, &sub),
                    None => die("hold needs releaseWhen"),
                };
                s.holds.push(Hold { release: rel, released: false, buf: Vec::new() });
                hold = Some(s.holds.len() - 1);
            }
            s.rules.push(Rule { m, actions, inject, hold });
        }
    }
    s
}

fn apply_action(a: &Action) -> Result<(), String> {
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
        Action::ReplaceFile { path, bytes, mode } => {
            wipe(path)?;
            // reference: writeFileSync(p, bytes, {mode: mode ?? 0o644}) then chmodSync when a mode is given
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
        Action::ReplaceSymlink { path, target } => {
            wipe(path)?;
            std::os::unix::fs::symlink(target, path).map_err(|e| format!("{}: {}", path, e))
        }
        Action::Mkdir { path, mode } => {
            // reference: mkdirSync(p, {mode: mode ?? 0o777}) then chmodSync when a mode is given
            let mut b = fs::DirBuilder::new();
            b.mode(mode.unwrap_or(0o777));
            b.create(path).map_err(|e| format!("{}: {}", path, e))?;
            if let Some(m) = mode {
                fs::set_permissions(path, fs::Permissions::from_mode(*m)).map_err(|e| format!("{}: {}", path, e))?;
            }
            Ok(())
        }
        Action::Unlink { path } => wipe(path),
        Action::Append { path, bytes } => {
            // reference: appendFileSync creates a missing file (default mode 0o666, as OpenOptions)
            let mut f = OpenOptions::new().append(true).create(true).open(path).map_err(|e| format!("{}: {}", path, e))?;
            f.write_all(bytes).map_err(|e| format!("{}: {}", path, e))
        }
        Action::Truncate { path, size } => {
            let f = OpenOptions::new().write(true).open(path).map_err(|e| format!("{}: {}", path, e))?;
            f.set_len(*size).map_err(|e| format!("{}: {}", path, e))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// requests, completions, executor

struct Req {
    id: i64,
    slot: i64,
    op: i64,
    flags: i64,
    handle: i64,
    offset: i64,
    count: i64,
    mode: i64,
    path1: Vec<u8>,
    path2: Vec<u8>,
    data: Vec<u8>,
    rpath: String,
    entropy: Option<Vec<u8>>,
    hold: Option<usize>,
    hold_after: bool,
    inject: Option<(i32, String)>,
    inject_after: bool,
    after: Vec<Action>,
}

struct Comp {
    id: i64,
    slot: i64,
    op: i64,
    status: i64,
    eclass: i64,
    errno: i64,
    kind: i64,
    size: i64,
    dev: u64,
    ino: u64,
    handle: i64,
    nbytes: i64,
    eof: i64,
    names: Vec<(usize, usize, i64)>,
    data: Vec<u8>,
    code: String,
    msg: String,
    file: Option<File>,
    fatal: Option<String>,
    hold: Option<usize>,
    hold_after: bool,
    rpath: String,
    injected_after: bool,
}

impl Comp {
    fn new(r: &Req) -> Comp {
        Comp {
            id: r.id,
            slot: r.slot,
            op: r.op,
            status: 0,
            eclass: 0,
            errno: 0,
            kind: 0,
            size: 0,
            dev: 0,
            ino: 0,
            handle: -1,
            nbytes: 0,
            eof: 0,
            names: Vec::new(),
            data: Vec::new(),
            code: String::new(),
            msg: String::new(),
            file: None,
            fatal: None,
            hold: r.hold,
            hold_after: r.hold_after,
            rpath: r.rpath.clone(),
            injected_after: false,
        }
    }
}

type Table = Arc<Mutex<HashMap<i64, (Arc<File>, String)>>>;
type ReaddirMap = Arc<HashMap<String, Vec<Vec<u8>>>>;

fn path_of(b: &[u8]) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(b))
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

fn fail(c: &mut Comp, e: &io::Error) {
    let n = e.raw_os_error().unwrap_or(0);
    c.status = 1;
    c.errno = n as i64;
    c.eclass = errno_class(n);
    c.code = if n == 0 { "UNKNOWN".to_string() } else { errno_name(n) };
    c.msg = e.to_string();
    c.data.clear();
    c.names.clear();
    c.file = None;
}

fn fail_errno(c: &mut Comp, n: i32, code: &str, msg: &str) {
    c.status = 1;
    c.errno = n as i64;
    c.eclass = errno_class(n);
    c.code = code.to_string();
    c.msg = msg.to_string();
    c.data.clear();
    c.names.clear();
    c.file = None;
    c.handle = -1;
    c.nbytes = 0;
    c.eof = 0;
}

fn fill_meta(c: &mut Comp, m: &fs::Metadata) {
    c.kind = kind_of(&m.file_type());
    c.size = m.size().min(i64::MAX as u64) as i64;
    c.dev = m.dev();
    c.ino = m.ino();
}

fn table_get(t: &Table, h: i64) -> Option<Arc<File>> {
    t.lock().unwrap().get(&h).map(|e| e.0.clone())
}

fn run_req(req: Req, table: &Table, rd: &ReaddirMap) -> Comp {
    let mut c = Comp::new(&req);
    let inj = req.inject.clone();
    match &inj {
        Some((n, code)) if !req.inject_after => fail_errno(&mut c, *n, code, &format!("injected {}", code)),
        _ => exec_op(&req, &mut c, table, rd),
    }
    // the reference runs after-result barriers (their actions, then the injected error) only when the call returned
    let returned = c.status == 0;
    if returned {
        for a in &req.after {
            if let Err(e) = apply_action(a) {
                c.fatal = Some(format!("schedule action failed: {}", e));
            }
        }
        if req.inject_after {
            if let Some((n, code)) = &inj {
                c.injected_after = true;
                fail_errno(&mut c, *n, code, &format!("injected {}", code));
            }
        }
    }
    c
}

fn exec_op(req: &Req, c: &mut Comp, table: &Table, rd: &ReaddirMap) {
    let p1 = path_of(&req.path1);
    match req.op {
        1 => match fs::read_dir(&p1) {
            Err(e) => fail(c, &e),
            Ok(it) => {
                let mut ents: Vec<(Vec<u8>, i64)> = Vec::new();
                for r in it {
                    match r {
                        Ok(en) => {
                            let name = en.file_name().as_bytes().to_vec();
                            let kind = if req.flags & 1 == 1 { en.file_type().map(|t| kind_of(&t)).unwrap_or(0) } else { 0 };
                            ents.push((name, kind));
                        }
                        Err(e) => {
                            fail(c, &e);
                            return;
                        }
                    }
                }
                if let Some(order) = rd.get(&req.rpath) {
                    let mut a: Vec<Vec<u8>> = ents.iter().map(|e| e.0.clone()).collect();
                    a.sort();
                    let mut b: Vec<Vec<u8>> = order.clone();
                    b.sort();
                    if a != b {
                        c.fatal = Some(format!("schedule readdir name set differs from the real one for {}", req.rpath));
                        return;
                    }
                    let mut out: Vec<(Vec<u8>, i64)> = Vec::new();
                    for n in order {
                        let k = ents.iter().find(|e| &e.0 == n).map(|e| e.1).unwrap_or(0);
                        out.push((n.clone(), k));
                    }
                    ents = out;
                }
                for (n, k) in ents {
                    let start = c.data.len();
                    c.data.extend_from_slice(&n);
                    c.names.push((start, n.len(), k));
                }
            }
        },
        2 => match fs::symlink_metadata(&p1) {
            Ok(m) => fill_meta(c, &m),
            Err(e) => fail(c, &e),
        },
        3 => match fs::canonicalize(&p1) {
            Ok(p) => c.data = p.as_os_str().as_bytes().to_vec(),
            Err(e) => fail(c, &e),
        },
        4 => {
            let mut o = OpenOptions::new();
            o.read(true);
            if req.flags & 1 == 1 {
                o.custom_flags(O_NOFOLLOW);
            }
            match o.open(&p1) {
                Ok(f) => c.file = Some(f),
                Err(e) => fail(c, &e),
            }
        }
        5 => match table_get(table, req.handle) {
            None => fail_errno(c, 9, "EBADF", "bad file descriptor"),
            Some(f) => match f.metadata() {
                Ok(m) => fill_meta(c, &m),
                Err(e) => fail(c, &e),
            },
        },
        6 => match table_get(table, req.handle) {
            None => fail_errno(c, 9, "EBADF", "bad file descriptor"),
            Some(f) => {
                let want = req.count.max(0) as usize;
                let mut buf = vec![0u8; want];
                loop {
                    match f.read_at(&mut buf, req.offset as u64) {
                        Ok(n) => {
                            buf.truncate(n);
                            c.nbytes = n as i64;
                            c.eof = if n == 0 || n < want { 1 } else { 0 };
                            c.data = buf;
                            break;
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            fail(c, &e);
                            break;
                        }
                    }
                }
            }
        },
        7 => {
            let removed = table.lock().unwrap().remove(&req.handle);
            if removed.is_none() {
                fail_errno(c, 9, "EBADF", "bad file descriptor");
            }
        }
        8 => {
            let mut b = fs::DirBuilder::new();
            b.recursive(req.flags & 1 == 1);
            if req.mode > 0 {
                b.mode(req.mode as u32);
            }
            if let Err(e) = b.create(&p1) {
                fail(c, &e);
            }
        }
        9 => {
            let p2 = path_of(&req.path2);
            if let Err(e) = fs::rename(&p1, &p2) {
                fail(c, &e);
            }
        }
        10 => match fs::symlink_metadata(&p1) {
            Err(e) => {
                if !(e.kind() == io::ErrorKind::NotFound && req.flags & 2 != 0) {
                    fail(c, &e);
                }
            }
            Ok(m) => {
                let r = if m.is_dir() { fs::remove_dir_all(&p1) } else { fs::remove_file(&p1) };
                if let Err(e) = r {
                    fail(c, &e);
                }
            }
        },
        11 => {
            let mut o = OpenOptions::new();
            o.write(true).create_new(true);
            if req.mode > 0 {
                o.mode(req.mode as u32);
            }
            match o.open(&p1) {
                Ok(f) => c.file = Some(f),
                Err(e) => fail(c, &e),
            }
        }
        12 => match table_get(table, req.handle) {
            None => fail_errno(c, 9, "EBADF", "bad file descriptor"),
            Some(f) => loop {
                match f.write_at(&req.data, req.offset as u64) {
                    Ok(n) => {
                        c.nbytes = n as i64;
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        fail(c, &e);
                        break;
                    }
                }
            },
        },
        13 => {
            let n = if req.count > 0 { req.count as usize } else { 6 };
            match &req.entropy {
                Some(b) => {
                    if b.len() != n {
                        c.fatal = Some(format!("entropyHex entry has {} bytes, request wants {}", b.len(), n));
                    } else {
                        c.data = b.clone();
                    }
                }
                None => {
                    let mut buf = vec![0u8; n];
                    let r = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf));
                    match r {
                        Ok(()) => c.data = buf,
                        Err(e) => fail(c, &e),
                    }
                }
            }
            c.nbytes = c.data.len() as i64;
        }
        _ => c.fatal = Some(format!("unknown opcode {}", req.op)),
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

fn ref_ok(len: usize, s: i64, n: i64) -> bool {
    if s == -1 {
        return n == 0;
    }
    if s < 0 || n < 0 {
        return false;
    }
    (s as i128 + n as i128) <= len as i128
}

fn parse_step(p: &[i64]) -> Step {
    if p.len() < HEADER {
        die(&format!("step packet too short: {} words", p.len()));
    }
    if p[0] != MAGIC {
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
        die("step packet ctl word count must be 256");
    }
    let (np, nr, nw, nb, nres) = (p[4], p[5], p[6], p[7], p[8]);
    if np < 0 || nr < 0 || nw < 0 || nb < 0 || nres < 0 {
        die("step packet negative count");
    }
    if nr > 4 {
        die("step packet request count above 4");
    }
    if p[9] != 0 || p[10] != 0 || p[11] != 0 {
        die("step packet reserved words must be 0");
    }
    let total: i128 = HEADER as i128 + 8 * np as i128 + 16 * nr as i128 + nw as i128 + nb as i128 + nres as i128;
    if total != p.len() as i128 {
        die(&format!("step packet length {} does not match the formula {}", p.len(), total));
    }
    if action == 1 && nres == 0 {
        die("done step carries no result");
    }
    if action != 1 && nres != 0 {
        die("only a done step carries a result");
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
        if !(0..=7).contains(&w[1]) {
            die("patch bank out of range");
        }
        if w[2] < 0 || w[3] < 0 || w[5] < 0 {
            die("patch has a negative offset or count");
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
    let req_at = at;
    at += 16 * nr;
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
    for i in 0..nr {
        let r = p[req_at + 16 * i..req_at + 16 * i + 16].to_vec();
        if r[0] < 1 {
            die("request id must be at least 1");
        }
        if !(0..=3).contains(&r[1]) {
            die("request slot out of range");
        }
        if !(1..=13).contains(&r[2]) {
            die("request opcode out of range");
        }
        if !(0..=0xFFFF).contains(&r[3]) {
            die("request flags out of range");
        }
        if r[4] != -1 && r[4] < 1 {
            die("request handle must be -1 or at least 1");
        }
        if r[5] < 0 {
            die("request offset negative");
        }
        if !(0..=MAX_COUNT).contains(&r[6]) {
            die("request count out of range");
        }
        if !(0..=4095).contains(&r[7]) {
            die("request mode out of range");
        }
        if !ref_ok(nb, r[8], r[9]) || !ref_ok(nb, r[10], r[11]) || !ref_ok(nb, r[12], r[13]) {
            die("request ref outside the literal byte pool");
        }
        if !(0..=30).contains(&r[14]) {
            die("request stage out of range");
        }
        if r[15] != 0 {
            die("request reserved word must be 0");
        }
        match r[2] {
            1 | 2 | 3 | 4 | 8 | 10 | 11 => {
                if r[8] < 0 {
                    die("request needs path1");
                }
            }
            9 => {
                if r[8] < 0 || r[10] < 0 {
                    die("rename needs path1 and path2");
                }
            }
            5 | 6 | 7 => {
                if r[4] < 1 {
                    die("request needs a handle");
                }
            }
            12 => {
                if r[4] < 1 || r[12] < 0 {
                    die("pwrite needs a handle and data");
                }
            }
            _ => {}
        }
        requests.push(r);
    }
    if action == 1 && nr != 0 {
        die("done step must not issue requests");
    }
    Step { action, ctl, patches, requests, words, bytes, result }
}

fn apply_patches(b: &mut Banks, st: &Step, event: &[i64], comp: &[u8]) {
    for w in &st.patches {
        let bank = w[1] as usize;
        let (off, cnt, src, soff) = (w[2], w[3], w[4], w[5]);
        if w[0] == 0 {
            if off != 0 || src != 0 || soff != 0 {
                die("resize patch must have offset 0 and no source");
            }
            if cnt > MAX_BANK {
                die("resize patch length too large");
            }
            b.resize(bank, cnt as usize);
        } else {
            if src == 0 {
                die("copy patch needs a source");
            }
            if off as i128 + cnt as i128 > b.len(bank) as i128 {
                die("copy patch destination range outside the bank");
            }
            let (off, cnt, soff) = (off as usize, cnt as usize, soff as usize);
            if src == 1 || src == 3 {
                let s: &[i64] = if src == 1 { event } else { &st.words };
                if soff as i128 + cnt as i128 > s.len() as i128 {
                    die("copy patch source range outside the word source");
                }
                if let Err(e) = b.put_words(bank, off, &s[soff..soff + cnt]) {
                    die(&e);
                }
            } else {
                let s: &[u8] = if src == 2 { comp } else { &st.bytes };
                if soff as i128 + cnt as i128 > s.len() as i128 {
                    die("copy patch source range outside the byte source");
                }
                b.put_bytes(bank, off, &s[soff..soff + cnt]);
            }
        }
    }
}

fn empty_event() -> Vec<i64> {
    let mut e = vec![0i64; 24];
    e[0] = 1;
    e[2] = -1;
    e
}

// ---------------------------------------------------------------------------------------------------------------
// host state

struct Host {
    trace: bool,
    src: String,
    dst: String,
    sched: Sched,
    rd: ReaddirMap,
    table: Table,
    next_handle: i64,
    tx: mpsc::Sender<Comp>,
    rx: mpsc::Receiver<Comp>,
    running: usize,
    ready: VecDeque<Comp>,
    outstanding: HashMap<i64, i64>,
    traced: HashMap<i64, (i64, i64, i64, Vec<u8>)>,
    after_bar: HashMap<i64, usize>,
    slot_busy: [bool; 4],
    entropy_idx: usize,
    // reference S.seen: emitted-event counts by (event, op, path); S.occ: call counts by (op, path)
    seen: HashMap<(String, i64, Option<String>), u64>,
    occ: HashMap<(i64, String), u64>,
}

impl Host {
    fn new(cfg: &Cfg, v: &serde_json::Value) -> Host {
        let mut sched = parse_sched(v.get("schedule"), &cfg.src, &cfg.dst);
        let rd = Arc::new(std::mem::take(&mut sched.readdir));
        let (tx, rx) = mpsc::channel::<Comp>();
        Host {
            trace: cfg.trace,
            src: cfg.src.clone(),
            dst: cfg.dst.clone(),
            sched,
            rd,
            table: Arc::new(Mutex::new(HashMap::new())),
            next_handle: 1,
            tx,
            rx,
            running: 0,
            ready: VecDeque::new(),
            outstanding: HashMap::new(),
            traced: HashMap::new(),
            after_bar: HashMap::new(),
            slot_busy: [false; 4],
            entropy_idx: 0,
            seen: HashMap::new(),
            occ: HashMap::new(),
        }
    }

    fn logical(&self, p: &str) -> String {
        let mut best: Option<(usize, &str)> = None;
        for (root, name) in [(&self.src, "$SRC"), (&self.dst, "$DST")] {
            if root.is_empty() || !p.starts_with(root.as_str()) {
                continue;
            }
            let rest = &p[root.len()..];
            if !(rest.is_empty() || rest.starts_with('/') || root.ends_with('/')) {
                continue;
            }
            if best.map(|b| root.len() > b.0).unwrap_or(true) {
                best = Some((root.len(), name));
            }
        }
        match best {
            Some((n, name)) => format!("{}{}", name, &p[n..]),
            None => p.to_string(),
        }
    }

    // TS reference shape (reference-primitives.mjs emit): op names open/read/write, unused fields null, errno as the
    // code string, kind as a name. The issue's handle/data are kept per id for the completion line.
    fn trace_issue(&mut self, req: &Req) {
        if !self.trace {
            return;
        }
        let name = trace_op(req.op);
        let hop = matches!(req.op, 5 | 6 | 7 | 12);
        let handle = if hop { serde_json::json!(req.handle) } else { serde_json::Value::Null };
        let (offset, count) = match req.op {
            6 => (serde_json::json!(req.offset), serde_json::json!(req.count)),
            12 => (serde_json::json!(req.offset), serde_json::json!(req.data.len())),
            13 => (serde_json::Value::Null, serde_json::json!(req.count)),
            _ => (serde_json::Value::Null, serde_json::Value::Null),
        };
        let mode = if req.op == 8 || req.op == 11 { serde_json::json!(req.mode) } else { serde_json::Value::Null };
        let path = if req.op == 13 { serde_json::Value::Null } else { serde_json::json!(self.logical(&req.rpath)) };
        self.traced.insert(req.id, (req.handle, req.mode, req.count, if req.op == 12 { req.data.clone() } else { Vec::new() }));
        let v = serde_json::json!({"trace": {
            "event": "issue", "op": name, "path": path, "id": req.id, "slot": null, "handle": handle,
            "offset": offset, "count": count, "mode": mode, "errno": null, "kind": null, "dev": null, "ino": null,
            "bytes": null, "digest": null,
        }});
        eprintln!("{}", v);
    }

    // Counts one reference `emit` event (for releaseWhen) and releases the holds whose condition is now met.
    fn note(&mut self, event: &str, op: i64, rpath: &str) {
        let path = if op == 13 { None } else { Some(rpath.to_string()) };
        *self.seen.entry((event.to_string(), op, path)).or_insert(0) += 1;
        self.release_check();
    }

    fn release_check(&mut self) {
        let held_count: usize = self.sched.holds.iter().filter(|h| !h.released).map(|h| h.buf.len()).sum();
        let seen = &self.seen;
        let ready = &mut self.ready;
        for h in self.sched.holds.iter_mut() {
            if !h.released && h.release.met(seen, held_count) {
                h.released = true;
                for c in std::mem::take(&mut h.buf) {
                    ready.push_back(c);
                }
            }
        }
    }

    fn barrier_event(&mut self, event: &str, op: i64, rpath: &str, id: i64) {
        self.trace_barrier(event, op, rpath, id);
        self.note(event, op, rpath);
    }

    fn trace_barrier(&self, event: &str, op: i64, rpath: &str, id: i64) {
        if !self.trace {
            return;
        }
        let path = if op == 13 { serde_json::Value::Null } else { serde_json::json!(self.logical(rpath)) };
        let v = serde_json::json!({"trace": {
            "event": event, "op": trace_op(op), "path": path, "id": id, "slot": null, "handle": null, "offset": null,
            "count": null, "mode": null, "errno": null, "kind": null, "dev": null, "ino": null, "bytes": null, "digest": null,
        }});
        eprintln!("{}", v);
    }

    fn trace_complete(&mut self, c: &Comp) {
        if !self.trace {
            return;
        }
        let (rh, rmode, rcount, wdata) = self.traced.remove(&c.id).unwrap_or((-1, 0, 0, Vec::new()));
        let null = serde_json::Value::Null;
        let hop = matches!(c.op, 5 | 6 | 7 | 12);
        let mut f = serde_json::json!({
            "event": "complete", "op": trace_op(c.op),
            "path": if c.op == 13 { null.clone() } else { serde_json::json!(self.logical(&c.rpath)) },
            "id": c.id, "slot": null, "handle": if hop && c.op != 13 { serde_json::json!(rh) } else { null.clone() },
            "offset": null, "count": null, "mode": null, "errno": null, "kind": null, "dev": null, "ino": null,
            "bytes": null, "digest": null,
        });
        if c.status != 0 {
            f["errno"] = serde_json::json!(if c.code.is_empty() { "EIO".to_string() } else { c.code.clone() });
        } else {
            match c.op {
                2 | 5 => {
                    f["kind"] = serde_json::json!(kind_name(c.kind));
                    f["dev"] = serde_json::json!(c.dev);
                    f["ino"] = serde_json::json!(c.ino);
                }
                4 => f["handle"] = serde_json::json!(c.handle),
                11 => {
                    f["handle"] = serde_json::json!(c.handle);
                    f["mode"] = serde_json::json!(rmode);
                }
                6 => {
                    f["bytes"] = serde_json::json!(c.nbytes);
                    f["digest"] = serde_json::json!(sha256_hex(&c.data[..(c.nbytes.max(0) as usize).min(c.data.len())]));
                }
                12 => {
                    let n = (c.nbytes.max(0) as usize).min(wdata.len());
                    f["bytes"] = serde_json::json!(c.nbytes);
                    f["digest"] = serde_json::json!(sha256_hex(&wdata[..n]));
                }
                1 => {
                    f["count"] = serde_json::json!(c.names.len());
                }
                13 => {
                    f["count"] = serde_json::json!(rcount);
                    f["bytes"] = serde_json::json!(c.data.len());
                    f["digest"] = serde_json::json!(sha256_hex(&c.data));
                }
                _ => {}
            }
        }
        eprintln!("{}", serde_json::json!({"trace": f}));
    }

    fn issue(&mut self, r: &[i64], lit: &[u8]) {
        let (id, slot, op) = (r[0], r[1], r[2]);
        if self.outstanding.contains_key(&id) {
            die(&format!("request id {} is already outstanding", id));
        }
        if self.slot_busy[slot as usize] {
            die(&format!("slot {} already has an outstanding request", slot));
        }
        let cut = |s: i64, n: i64| -> Vec<u8> {
            if s < 0 {
                Vec::new()
            } else {
                lit[s as usize..(s + n) as usize].to_vec()
            }
        };
        let path1 = cut(r[8], r[9]);
        let path2 = cut(r[10], r[11]);
        let data = cut(r[12], r[13]);
        let rpath = match op {
            5 | 6 | 7 | 12 => self.table.lock().unwrap().get(&r[4]).map(|e| e.1.clone()).unwrap_or_default(),
            13 => String::new(),
            _ => String::from_utf8_lossy(&path1).into_owned(),
        };
        let mut req = Req {
            id,
            slot,
            op,
            flags: r[3],
            handle: r[4],
            offset: r[5],
            count: r[6],
            mode: r[7],
            path1,
            path2,
            data,
            rpath,
            entropy: None,
            hold: None,
            hold_after: false,
            inject: None,
            inject_after: false,
            after: Vec::new(),
        };
        // reference: once the scheduled entropyHex entries are used up, randomBytes falls back to OS entropy
        // (req.entropy stays None, exec_op reads /dev/urandom)
        if op == 13 && self.sched.test_entropy && self.entropy_idx < self.sched.entropy.len() {
            req.entropy = Some(self.sched.entropy[self.entropy_idx].clone());
            self.entropy_idx += 1;
        }
        self.trace_issue(&req);
        // occurrence of this call among calls with the same (op, path), counted for every rule alike (reference `begun`)
        let occ = {
            let e = self.occ.entry((op, req.rpath.clone())).or_insert(0);
            *e += 1;
            *e
        };
        // the issue event counts first and may release holds (reference emit -> notify)
        self.note("issue", op, &req.rpath);
        // reference: rules run in schedule order per barrier kind; the first rule carrying an error ends that kind
        // (its own actions and hold still ran), later rules of the same kind are skipped entirely
        let mut before_err = false;
        let mut after_err = false;
        for i in 0..self.sched.rules.len() {
            let hit = self.sched.rules[i].m.hit(op, &req.rpath, occ);
            if !hit {
                continue;
            }
            let after = self.sched.rules[i].m.after;
            // (a before-call error means the call never happens, so no after-result rule is reached either)
            if before_err || (after && after_err) {
                continue;
            }
            if after {
                *self.after_bar.entry(id).or_insert(0) += 1;
            } else {
                self.barrier_event("barrier-before-call", op, &req.rpath, id);
            }
            if let Some(h) = self.sched.rules[i].hold {
                req.hold = Some(h);
                req.hold_after = after;
            }
            if let Some(e) = self.sched.rules[i].inject.clone() {
                req.inject = Some(e);
                req.inject_after = after;
                if after {
                    after_err = true;
                } else {
                    before_err = true;
                }
            }
            let acts = self.sched.rules[i].actions.clone();
            if after {
                req.after.extend(acts);
            } else {
                for a in &acts {
                    if let Err(e) = apply_action(a) {
                        die(&format!("schedule action failed: {}", e));
                    }
                }
            }
        }
        self.outstanding.insert(id, slot);
        self.slot_busy[slot as usize] = true;
        self.running += 1;
        let tx = self.tx.clone();
        let table = self.table.clone();
        let rd = self.rd.clone();
        std::thread::spawn(move || {
            let c = run_req(req, &table, &rd);
            let _ = tx.send(c);
        });
    }

    fn next_completion(&mut self) -> Comp {
        loop {
            if let Some(c) = self.ready.pop_front() {
                return c;
            }
            if self.running == 0 {
                die("await has nothing deliverable: no outstanding request, or every outstanding request is held");
            }
            let c = match self.rx.recv() {
                Ok(c) => c,
                Err(_) => die("completion channel closed"),
            };
            self.running -= 1;
            if let Some(f) = &c.fatal {
                die(f);
            }
            // after-result barriers: the reference emits them once the call has returned and before it awaits the
            // after-result hold, so a hold released by its own barrier event releases at once (a real failure skips
            // them; an injected after-result error is raised by the barrier itself, so its line is still written)
            if let Some(n) = self.after_bar.remove(&c.id) {
                if c.status == 0 || c.injected_after {
                    for _ in 0..n {
                        self.barrier_event("barrier-after-result-before-delivery", c.op, &c.rpath, c.id);
                    }
                }
            }
            // reference: an after-result hold is awaited only once the call returned (a real failure never reaches it);
            // an injected after-result error is raised after the hold, so that call is held too
            let hold_applies = !c.hold_after || c.status == 0 || c.injected_after;
            let held = match c.hold {
                Some(h) => hold_applies && !self.sched.holds[h].released,
                None => false,
            };
            if held {
                let h = c.hold.unwrap();
                self.sched.holds[h].buf.push(c);
                // the reference re-evaluates releaseWhen when a waiter is added (`held` conditions)
                self.release_check();
            } else {
                self.ready.push_back(c);
            }
        }
    }

    // Registers a new handle, drops bookkeeping, traces, and builds (event words, completion bytes).
    fn deliver(&mut self, mut c: Comp) -> (Vec<i64>, Vec<u8>) {
        if c.status == 0 {
            if let Some(f) = c.file.take() {
                let h = self.next_handle;
                self.next_handle += 1;
                self.table.lock().unwrap().insert(h, (Arc::new(f), c.rpath.clone()));
                c.handle = h;
            }
        }
        self.outstanding.remove(&c.id);
        self.slot_busy[c.slot as usize] = false;
        self.trace_complete(&c);
        self.note("complete", c.op, &c.rpath);
        let mut ev = vec![0i64; 24 + 3 * c.names.len()];
        ev[0] = 1;
        ev[1] = c.id;
        ev[2] = c.slot;
        ev[3] = c.op;
        ev[4] = c.status;
        ev[5] = c.eclass;
        ev[6] = c.errno;
        ev[7] = c.kind;
        ev[8] = c.size;
        ev[9] = (c.dev >> 32) as i64;
        ev[10] = (c.dev & 0xffff_ffff) as i64;
        ev[11] = (c.ino >> 32) as i64;
        ev[12] = (c.ino & 0xffff_ffff) as i64;
        ev[13] = c.handle;
        ev[14] = c.nbytes;
        ev[15] = c.eof;
        ev[16] = c.names.len() as i64;
        ev[17] = if c.op == 1 && c.status == 0 { 3 } else { 0 };
        let bytes: Vec<u8>;
        if c.status == 1 {
            let mut b = c.code.as_bytes().to_vec();
            ev[19] = 0;
            ev[20] = c.code.len() as i64;
            ev[21] = c.code.len() as i64;
            ev[22] = c.msg.len() as i64;
            b.extend_from_slice(c.msg.as_bytes());
            bytes = b;
        } else {
            ev[19] = -1;
            ev[20] = 0;
            ev[21] = -1;
            ev[22] = 0;
            bytes = std::mem::take(&mut c.data);
        }
        ev[18] = bytes.len() as i64;
        for (i, n) in c.names.iter().enumerate() {
            ev[24 + 3 * i] = n.0 as i64;
            ev[25 + 3 * i] = n.1 as i64;
            ev[26 + 3 * i] = n.2;
        }
        (ev, bytes)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// driver

// The reference's spawn record (stubs.ts spawn): path = basename(binary), count/digest of the JSON argv; a failed spawn
// carries the errno code the OS would give for the binary (missing -> ENOENT, not executable -> EACCES).
fn trace_spawn(binary: &str) {
    let argv = "[\"agent\",\"--no-leader\",\"stdio\"]";
    let base = binary.rsplit('/').next().unwrap_or(binary).to_string();
    let digest = sha256_hex(argv.as_bytes());
    let found: Option<std::path::PathBuf> = if binary.contains('/') {
        Some(std::path::PathBuf::from(binary))
    } else {
        std::env::var_os("PATH").and_then(|ps| std::env::split_paths(&ps).map(|d| d.join(binary)).find(|c| c.is_file()))
    };
    let errno: Option<&str> = match found.as_ref().map(|f| fs::metadata(f)) {
        None | Some(Err(_)) => Some("ENOENT"),
        Some(Ok(m)) => {
            if m.is_dir() || m.permissions().mode() & 0o111 == 0 { Some("EACCES") } else { None }
        }
    };
    let id = 1_000_000_000i64;
    let line = |event: &str, ok: bool| {
        serde_json::json!({"trace": {
            "event": event, "op": "spawn", "path": base, "id": id, "slot": null, "handle": null, "offset": null,
            "count": if ok { serde_json::json!(3) } else { serde_json::Value::Null }, "mode": null,
            "errno": if event == "complete" { errno.map(|e| serde_json::json!(e)).unwrap_or(serde_json::Value::Null) } else { serde_json::Value::Null },
            "kind": null, "dev": null, "ino": null, "bytes": null,
            "digest": if ok { serde_json::json!(digest) } else { serde_json::Value::Null },
        }})
    };
    eprintln!("{}", line("issue", true));
    eprintln!("{}", line("complete", errno.is_none()));
}

fn run_one<S: Sol>(sol: &S, v: &serde_json::Value) -> String {
    let cfg = parse_cfg(v);
    if cfg.mode == 3 {
        if cfg.trace {
            trace_spawn(&cfg.binary);
        }
        let p = sol.load(&cfg.binary, &cfg.home, &cfg.folder, &cfg.thread, cfg.rpc);
        return render_result(&p);
    }
    let mut host = Host::new(&cfg, v);
    let begin = sol.begin(&cfg.words, &cfg.text);
    let mut ctl: Vec<i64>;
    let mut first: Option<Vec<i64>> = None;
    if begin.len() == CTL_WORDS {
        ctl = begin;
    } else if begin.len() >= HEADER && begin[0] == MAGIC {
        ctl = vec![0i64; CTL_WORDS];
        first = Some(begin);
    } else {
        die(&format!("grok_begin returned {} words; expected 256 (ctl) or a step packet", begin.len()));
    }
    let mut banks = Banks::new();
    let mut event = empty_event();
    let mut comp: Vec<u8> = Vec::new();
    loop {
        let pkt = match first.take() {
            Some(p) => p,
            None => sol.step(&cfg.words, &cfg.text, &ctl, &banks, &event, &comp),
        };
        let st = parse_step(&pkt);
        // the copy's ACP check (rpc::load_check) runs inside the step that sets ctl word 13; the runtime spawns the child
        // itself, so the host writes the reference's spawn record for it (before that step's requests)
        if cfg.trace && cfg.mode == 2 && ctl.get(13) != Some(&1) && st.ctl.get(13) == Some(&1) {
            trace_spawn(&cfg.binary);
        }
        ctl = st.ctl.clone();
        apply_patches(&mut banks, &st, &event, &comp);
        for r in &st.requests {
            host.issue(r, &st.bytes);
        }
        match st.action {
            1 => {
                if !host.outstanding.is_empty() {
                    die("done step while requests are still outstanding");
                }
                return render_result(&st.result);
            }
            2 => {
                event = empty_event();
                comp = Vec::new();
            }
            _ => {
                if host.outstanding.is_empty() {
                    die("await step with no outstanding request");
                }
                let c = host.next_completion();
                let (ev, cb) = host.deliver(c);
                event = ev;
                comp = cb;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// result packet rendering (DESIGN section 2)

const MODE_NAMES: [&str; 6] = ["list", "read", "copy", "load", "small-json", "caller-cleanup"];
const STAGE_NAMES: [&str; 31] = [
    "input", "realHome", "chain", "readdir", "candidate-dir", "file-lstat", "containment", "open",
    "identity-fstat", "size-fstat", "read", "close", "parse", "summary", "sort", "active",
    "first-target-lstat", "entropy", "parent-mkdir", "temp-mkdir", "exclusive-create", "write", "second-target-lstat", "rename",
    "spawn", "initialize", "session/load", "temp-cleanup", "target-cleanup", "caller-cleanup", "result",
];
const CLASS_NAMES: [&str; 8] = ["none", "EngineImportError", "AbortError", "RpcResponseError", "RawOsError", "URIError", "CoreError", "OtherError"];
const CODE_NAMES: [&str; 9] = [
    "none", "InputTooLarge", "InternalInvariant", "not-found", "grok-exists", "grok-load-failed", "timeout", "read-failed", "engine-missing",
];
const ERRNO_CLASS_NAMES: [&str; 5] = ["none", "ENOENT", "EACCES", "EEXIST", "other"];

fn chk_ref(tl: usize, s: i64, n: i64, what: &str) {
    if !ref_ok(tl, s, n) {
        die(&format!("result ref {} out of range", what));
    }
}

fn text_json(text: &[u8], s: i64, n: i64) -> String {
    if s < 0 {
        return "null".to_string();
    }
    let b = &text[s as usize..(s + n) as usize];
    match std::str::from_utf8(b) {
        Ok(t) => qs(t),
        Err(_) => format!("{{\"hex\":\"{}\"}}", hex(b)),
    }
}

fn raw_fragment(text: &[u8], s: i64, n: i64) -> Option<String> {
    if s < 0 {
        return None;
    }
    let b = &text[s as usize..(s + n) as usize];
    let t = match std::str::from_utf8(b) {
        Ok(t) => t,
        Err(_) => die("raw JSON fragment is not UTF-8"),
    };
    match serde_json::from_str::<Box<serde_json::value::RawValue>>(t) {
        Ok(r) => Some(r.get().to_string()),
        Err(e) => die(&format!("raw JSON fragment is invalid: {}", e)),
    }
}

pub fn render_result(p: &[i64]) -> String {
    if p.len() < 28 {
        die("result packet too short");
    }
    if p[0] != 0 && p[0] != 1 {
        die("result status out of range");
    }
    if p[1] != 24 {
        die("result metadata word count must be 24");
    }
    let (rw, bl) = (p[2], p[3]);
    if rw < 0 || bl < 0 {
        die("result negative row/byte length");
    }
    if 28 + rw as i128 + bl as i128 != p.len() as i128 {
        die("result packet length mismatch");
    }
    let status = p[0];
    let meta = &p[4..28];
    let rows = &p[28..28 + rw as usize];
    let mut text: Vec<u8> = Vec::with_capacity(bl as usize);
    for &x in &p[28 + rw as usize..] {
        if !(0..=255).contains(&x) {
            die("result text value outside 0..255");
        }
        text.push(x as u8);
    }
    let tl = text.len();
    if meta[0] != 2 {
        die("result schema must be 2");
    }
    if !(0..=5).contains(&meta[1]) {
        die("result mode out of range");
    }
    let mode = meta[1];
    if meta[2] != 0 && meta[2] != 1 {
        die("result omittedBefore must be 0/1");
    }
    if meta[3] < 0 {
        die("result row count negative");
    }
    let count = meta[3] as usize;
    let width: i64 = match mode {
        0 => 12,
        1 => 10,
        _ => 0,
    };
    // width 0 with no rows is the error/empty form of every mode (IMPL_NOTES result(): width 0 => count 0)
    let width = if meta[10] == 0 && meta[3] == 0 { 0 } else { width };
    if meta[10] != width {
        die("result row width does not match the mode");
    }
    if meta[3].checked_mul(width) != Some(rw) {
        die("result row words do not match count*width");
    }
    if meta[11] != 0 && meta[11] != 1 {
        die("result present must be 0/1");
    }
    if meta[12] < 0 {
        die("result copySkipped negative");
    }
    if !(0..=30).contains(&meta[13]) || !(0..=7).contains(&meta[14]) || !(0..=8).contains(&meta[15]) || !(0..=4).contains(&meta[16]) {
        die("result error enum out of range");
    }
    for (i, nm) in [(4usize, "title"), (6, "model"), (8, "value"), (18, "error.raw"), (20, "error.name"), (22, "error.detail")] {
        chk_ref(tl, meta[i], meta[i + 1], nm);
    }
    if status == 0 {
        if meta[13..=17].iter().any(|&x| x != 0) {
            die("success result carries error scalars");
        }
        for i in [18usize, 20, 22] {
            if meta[i] != -1 || meta[i + 1] != 0 {
                die("success result carries an error ref");
            }
        }
    } else if meta[14] == 0 {
        die("error result has class none");
    }
    // rows
    let mut out_rows: Vec<String> = Vec::with_capacity(count);
    for j in 0..count {
        let r = &rows[j * width as usize..(j + 1) * width as usize];
        let nrefs = if mode == 0 { 5 } else { 4 };
        for k in 0..nrefs {
            chk_ref(tl, r[2 * k], r[2 * k + 1], "row");
        }
        let mut s = String::from("{");
        if mode == 0 {
            if r[10] != 0 && r[10] != 1 {
                die("list row maybeOpen must be 0/1");
            }
            if r[11] != 0 {
                die("list row reserved word must be 0");
            }
            s.push_str(&format!("\"thread\":{},\"title\":{}", text_json(&text, r[0], r[1]), text_json(&text, r[2], r[3])));
            if r[4] >= 0 {
                s.push_str(&format!(",\"updatedAt\":{}", text_json(&text, r[4], r[5])));
            }
            s.push_str(&format!(",\"path\":{}", text_json(&text, r[6], r[7])));
            if r[8] >= 0 {
                let tok = &text[r[8] as usize..(r[8] + r[9]) as usize];
                if tok.is_empty() || tok.len() > 20 || !tok.iter().all(|c| c.is_ascii_digit()) {
                    die("list row messages token is not a decimal integer");
                }
                s.push_str(&format!(",\"messages\":{}", String::from_utf8_lossy(tok)));
            }
            if r[10] == 1 {
                s.push_str(",\"maybeOpen\":true");
            }
        } else {
            if r[8] < 0 {
                die("read row tools negative");
            }
            if r[9] != 0 && r[9] != 1 {
                die("read row clipped must be 0/1");
            }
            s.push_str(&format!("\"prompt\":{},\"reply\":{}", text_json(&text, r[0], r[1]), text_json(&text, r[2], r[3])));
            if r[4] >= 0 {
                s.push_str(&format!(",\"createdAt\":{}", text_json(&text, r[4], r[5])));
            }
            if r[6] >= 0 {
                s.push_str(&format!(",\"completedAt\":{}", text_json(&text, r[6], r[7])));
            }
            s.push_str(&format!(",\"tools\":{}", r[8]));
            if r[9] == 1 {
                s.push_str(",\"clipped\":true");
            }
        }
        s.push('}');
        out_rows.push(s);
    }
    let value = if mode == 4 {
        if meta[11] == 1 {
            match raw_fragment(&text, meta[8], meta[9]) {
                Some(f) => f,
                None => die("small-json present but value ref absent"),
            }
        } else {
            "null".to_string()
        }
    } else {
        text_json(&text, meta[8], meta[9])
    };
    let error = if status == 1 {
        let code = if meta[15] == 0 { "null".to_string() } else { qs(CODE_NAMES[meta[15] as usize]) };
        let errno_class = qs(ERRNO_CLASS_NAMES[meta[16] as usize]);
        // an absent raw code (ref -1) omits the key, like the reference (absent differs from JSON null)
        let raw = match raw_fragment(&text, meta[18], meta[19]) {
            Some(f) => format!(",\"raw\":{}", f),
            None => String::new(),
        };
        format!(
            "{{\"stage\":{},\"class\":{},\"code\":{},\"errnoClass\":{},\"native\":{}{},\"name\":{},\"detail\":{}}}",
            qs(STAGE_NAMES[meta[13] as usize]),
            qs(CLASS_NAMES[meta[14] as usize]),
            code,
            errno_class,
            meta[17],
            raw,
            text_json(&text, meta[20], meta[21]),
            text_json(&text, meta[22], meta[23]),
        )
    } else {
        "null".to_string()
    };
    format!(
        "{{\"status\":{},\"mode\":{},\"omittedBefore\":{},\"rows\":[{}],\"title\":{},\"model\":{},\"value\":{},\"present\":{},\"copySkipped\":{},\"error\":{}}}",
        if status == 0 { "\"ok\"" } else { "\"error\"" },
        qs(MODE_NAMES[mode as usize]),
        if meta[2] == 1 { "true" } else { "false" },
        out_rows.join(","),
        text_json(&text, meta[4], meta[5]),
        text_json(&text, meta[6], meta[7]),
        value,
        if meta[11] == 1 { "true" } else { "false" },
        meta[12],
        error,
    )
}

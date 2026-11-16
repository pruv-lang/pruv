//! G4b-1 (docs/G4B_SPEC.md sections 3, 4.1, 5): package manifests (pruv.json) and the package graph
//! (counterpart of pruvc/manifest.py).
//!
//! `load_graph(manifest_path)` validates the program manifest and, depth first in key order, every dependency manifest;
//! it checks that the graph is acyclic, that there is one directory per package name and that every version requirement
//! is satisfied. All errors are stage "pkg"; no machine path appears in any message.
//!
//! A manifest error carries the display of the manifest as its `file`. The diagnostic format derives `file` from the file
//! tag of the line number, so `PkgError::into_pruv` registers that one display as file 1 and tags the line with it.
//! pruv-core has no JSON parser, so a small strict one lives here; it accepts exactly what Python `json.loads` accepts
//! for the cases that matter (see `parse_json`).

use crate::diagnostic::{set_files, tagged_line, PruvError};
use crate::lexer::{KEYWORDS, RUST_KEYWORDS};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FIX: &str = "see docs/G4B_SPEC.md section 3";
const MAX: u64 = 2147483647; // 2**31 - 1

/// A manifest / graph error: the diagnostic (line 1, col 1, untagged) and the display path it is reported for.
#[derive(Debug, Clone)]
pub struct PkgError {
    pub err: PruvError,
    pub file: String,
}

impl PkgError {
    fn new(code: &str, message: String, fix: String, file: &str) -> PkgError {
        PkgError { err: PruvError::new("pkg", 1, 1, code, message, fix), file: file.to_string() }
    }

    /// The diagnostic with its `file`: the display is registered as the only file (tag 1) of the process.
    pub fn into_pruv(self) -> PruvError {
        set_files(vec![self.file]);
        let mut e = self.err;
        e.line = tagged_line(e.line, 1);
        e
    }
}

type Res<T> = Result<T, PkgError>;

/// One package of the graph.
#[derive(Debug, Clone)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    /// canonical package directory
    pub dir: PathBuf,
    /// root file, relative to dir, `/` separators
    pub root: String,
    /// (mount key, canonical dir, requirement text) in JSON key order
    pub deps: Vec<(String, PathBuf, String)>,
    /// display of this package's manifest
    pub display: String,
    /// (requiring package index, mount key, requirement text) in discovery order
    pub reqs: Vec<(usize, String, String)>,
    /// (requiring package index, mount key) that first reached this directory; None = the program
    pub first: Option<(usize, String)>,
}

#[derive(Debug, Clone)]
pub struct Graph {
    /// discovery order, the program package first (index 0)
    pub pkgs: Vec<Pkg>,
    /// canonical dir -> index into pkgs
    pub by_dir: HashMap<PathBuf, usize>,
}

impl Graph {
    pub fn program(&self) -> &Pkg {
        &self.pkgs[0]
    }
}

/// `os.path.join(dir, *rel.split("/"))`
pub fn join_rel(dir: &Path, rel: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
    }
    p
}

// ---------------------------------------------------------------------------
// JSON (Python json.loads semantics)
// ---------------------------------------------------------------------------

/// a JSON value; only strings and objects matter, everything else (numbers, literals, arrays) is `Other`
enum Jv {
    Obj(Obj),
    Str(String),
    Other,
}

/// a JSON object that keeps its key order (first insertion position, last value, like a Python dict) and remembers its
/// duplicate keys
struct Obj {
    items: Vec<(String, Jv)>,
    dups: Vec<String>,
}

impl Obj {
    fn get(&self, k: &str) -> Option<&Jv> {
        self.items.iter().find(|(n, _)| n == k).map(|(_, v)| v)
    }
}

/// nesting limit standing in for the Python recursion limit (RecursionError is a failure to parse)
const MAX_DEPTH: usize = 980;
/// Python 3.11+ refuses to convert an integer literal of more than 4300 digits
const MAX_INT_DIGITS: usize = 4300;

struct JP<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

fn hex4(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    let mut v: u32 = 0;
    for &c in &b[..4] {
        let d = (c as char).to_digit(16)?;
        v = v * 16 + d;
    }
    Some(v)
}

impl<'a> JP<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn lit(&mut self, word: &str) -> Option<()> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Jv> {
        match self.peek()? {
            b'"' => {
                self.i += 1;
                Some(Jv::Str(self.string()?))
            }
            b'{' => {
                self.i += 1;
                self.object()
            }
            b'[' => {
                self.i += 1;
                self.array()
            }
            b'n' => self.lit("null").map(|_| Jv::Other),
            b't' => self.lit("true").map(|_| Jv::Other),
            b'f' => self.lit("false").map(|_| Jv::Other),
            b'N' => self.lit("NaN").map(|_| Jv::Other),
            b'I' => self.lit("Infinity").map(|_| Jv::Other),
            b'-' | b'0'..=b'9' => self.number().map(|_| Jv::Other),
            _ => None,
        }
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`, or `-Infinity`
    fn number(&mut self) -> Option<()> {
        if self.peek() == Some(b'-') {
            if self.s[self.i..].starts_with(b"-Infinity") {
                self.i += 9;
                return Some(());
            }
            self.i += 1;
        }
        let ds = self.i;
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return None,
        }
        let int_digits = self.i - ds;
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.s.get(self.i + 1), Some(b'0'..=b'9')) {
            is_float = true;
            self.i += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let mut j = self.i + 1;
            if matches!(self.s.get(j), Some(b'+') | Some(b'-')) {
                j += 1;
            }
            if matches!(self.s.get(j), Some(b'0'..=b'9')) {
                is_float = true;
                while matches!(self.s.get(j), Some(b'0'..=b'9')) {
                    j += 1;
                }
                self.i = j;
            }
        }
        if !is_float && int_digits > MAX_INT_DIGITS {
            return None;
        }
        Some(())
    }

    /// after the opening quote; \uXXXX escapes and surrogate pairs decode like Python (a lone surrogate, which a Rust
    /// String cannot hold, becomes U+FFFD)
    fn string(&mut self) -> Option<String> {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(buf).ok(),
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'"' => buf.push(b'"'),
                        b'\\' => buf.push(b'\\'),
                        b'/' => buf.push(b'/'),
                        b'b' => buf.push(8),
                        b'f' => buf.push(12),
                        b'n' => buf.push(b'\n'),
                        b'r' => buf.push(b'\r'),
                        b't' => buf.push(b'\t'),
                        b'u' => {
                            let mut cp = hex4(&self.s[self.i..])?;
                            self.i += 4;
                            if (0xD800..=0xDBFF).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                if let Some(lo) = hex4(&self.s[self.i + 2..]) {
                                    if (0xDC00..=0xDFFF).contains(&lo) {
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                        self.i += 6;
                                    }
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut tmp = [0u8; 4];
                            buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return None,
                    }
                }
                0..=0x1F => return None, // strict: no raw control characters
                _ => buf.push(c),
            }
        }
    }

    /// after `{`
    fn object(&mut self) -> Option<Jv> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let mut o = Obj { items: Vec::new(), dups: Vec::new() };
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            self.depth -= 1;
            return Some(Jv::Obj(o));
        }
        loop {
            if self.peek() != Some(b'"') {
                return None;
            }
            self.i += 1;
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return None;
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            match o.items.iter().position(|(n, _)| *n == k) {
                Some(p) => {
                    o.dups.push(k);
                    o.items[p].1 = v;
                }
                None => o.items.push((k, v)),
            }
            self.ws();
            match self.peek()? {
                b',' => {
                    self.i += 1;
                    self.ws();
                }
                b'}' => {
                    self.i += 1;
                    self.depth -= 1;
                    return Some(Jv::Obj(o));
                }
                _ => return None,
            }
        }
    }

    /// after `[`
    fn array(&mut self) -> Option<Jv> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            self.depth -= 1;
            return Some(Jv::Other);
        }
        loop {
            self.ws();
            self.value()?;
            self.ws();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    self.depth -= 1;
                    return Some(Jv::Other);
                }
                _ => return None,
            }
        }
    }
}

/// `json.loads(bytes.decode("utf-8"), object_pairs_hook=...)` reduced to what the manifest needs; None for invalid
/// UTF-8, a BOM, a syntax error or trailing garbage. NaN, Infinity and -Infinity are accepted as numbers.
fn parse_json(bytes: &[u8]) -> Option<Jv> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.starts_with('\u{FEFF}') {
        return None;
    }
    let mut p = JP { s: text.as_bytes(), i: 0, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return None;
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// paths (posixpath semantics)
// ---------------------------------------------------------------------------

/// posixpath.normpath on a string
fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let mut initial = if p.starts_with('/') { 1 } else { 0 };
    if p.starts_with("//") && !p.starts_with("///") {
        initial = 2;
    }
    let mut comps: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let r = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if r.is_empty() {
        ".".to_string()
    } else {
        r
    }
}

/// os.path.abspath
fn abspath(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    let full = if s.starts_with('/') {
        s
    } else {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{}/{}", cwd, s)
    };
    PathBuf::from(normpath(&full))
}

/// os.path.realpath (non-strict: components that do not exist are kept, `..` pops them)
fn realpath(p: &Path) -> PathBuf {
    use std::path::Component;
    fn parts(p: &Path) -> Vec<std::ffi::OsString> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_os_string()),
                Component::ParentDir => Some(std::ffi::OsString::from("..")),
                _ => None,
            })
            .collect()
    }
    let mut out = PathBuf::from("/");
    let mut todo: Vec<std::ffi::OsString> = parts(p);
    todo.reverse();
    let mut links = 0usize;
    while let Some(c) = todo.pop() {
        if c == ".." {
            out.pop();
            continue;
        }
        let next = out.join(&c);
        match std::fs::read_link(&next) {
            Ok(target) if links < 255 => {
                links += 1;
                if target.has_root() {
                    out = PathBuf::from("/");
                }
                let mut tp = parts(&target);
                tp.reverse();
                todo.extend(tp);
            }
            _ => out = next,
        }
    }
    out
}

fn is_file(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// validation
// ---------------------------------------------------------------------------

fn bad<T>(display: &str, keypath: &str, reason: &str) -> Res<T> {
    Err(PkgError::new("manifest_invalid", format!("invalid manifest key {}: {}", keypath, reason), FIX.to_string(), display))
}

/// `[a-z][a-z0-9_]*` (upper = false) or `[a-z][A-Za-z0-9_]*` (upper = true, the module-name rule)
fn is_ident(s: &str, upper: bool) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_lowercase()
        && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || (upper && c.is_ascii_uppercase()))
}

/// `0|[1-9][0-9]*`; values beyond u64 saturate (they are over the limit anyway)
fn num(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.is_empty() || !b.iter().all(|c| c.is_ascii_digit()) || (b[0] == b'0' && b.len() > 1) {
        return None;
    }
    if b.len() > 18 {
        return Some(u64::MAX);
    }
    s.parse::<u64>().ok()
}

/// `NUM.NUM.NUM` -> parts (None when the shape is wrong)
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let v: Option<Vec<u64>> = s.split('.').map(num).collect();
    v.filter(|v| v.len() == 3)
}

/// `NUM(.NUM){0,2}` -> parts
fn parse_req(s: &str) -> Option<Vec<u64>> {
    let v: Option<Vec<u64>> = s.split('.').map(num).collect();
    v.filter(|v| (1..=3).contains(&v.len()))
}

/// a relative path with `/` separators and no empty segment
fn plain_rel(p: &str, allow_dotdot: bool) -> bool {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') || p.contains('\0') {
        return false;
    }
    let mut cs = p.chars();
    if let (Some(a), Some(':')) = (cs.next(), cs.next()) {
        if a.is_alphabetic() {
            return false;
        }
    }
    let segs: Vec<&str> = p.split('/').collect();
    if segs.iter().any(|x| x.is_empty()) {
        return false;
    }
    allow_dotdot || !segs.contains(&"..")
}

/// duplicate and unknown keys of one object (path = key path prefix, "" at the top)
fn check_obj(obj: &Obj, path: &str, allowed: &[&str], display: &str) -> Res<()> {
    let pre = if path.is_empty() { String::new() } else { format!("{}.", path) };
    if let Some(d) = obj.dups.first() {
        return bad(display, &format!("{}{}", pre, d), "duplicate key");
    }
    for (k, _) in &obj.items {
        if !allowed.contains(&k.as_str()) {
            return bad(display, &format!("{}{}", pre, k), "unknown key");
        }
    }
    Ok(())
}

/// a string value of `obj[key]` (the key exists: `check_obj` passed and presence was tested)
fn str_of<'a>(obj: &'a Obj, key: &str, keypath: &str, display: &str) -> Res<&'a str> {
    match obj.get(key) {
        None => bad(display, keypath, "missing"),
        Some(Jv::Str(s)) => Ok(s.as_str()),
        Some(_) => bad(display, keypath, "expected a string"),
    }
}

/// read and validate one manifest; the dependencies come back as canonical directories
fn read(pkgdir: &Path, mfile: &Path, display: &str) -> Res<Pkg> {
    let data = match std::fs::read(mfile).ok().and_then(|b| parse_json(&b)) {
        Some(Jv::Obj(o)) => o,
        _ => {
            return Err(PkgError::new(
                "manifest_invalid",
                "invalid manifest: not a UTF-8 JSON object".to_string(),
                FIX.to_string(),
                display,
            ))
        }
    };
    check_obj(&data, "", &["name", "version", "root", "dependencies"], display)?;
    // presence and type of the three strings, in order (a missing or non-string `name` wins over a bad `version`)
    for k in ["name", "version", "root"] {
        str_of(&data, k, k, display)?;
    }
    let name = str_of(&data, "name", "name", display)?;
    let version = str_of(&data, "version", "version", display)?;
    let root = str_of(&data, "root", "root", display)?;
    if !is_ident(name, false)
        || name.contains("__")
        || name.starts_with("pruv_")
        || name == "std"
        || name == "crate"
        || RUST_KEYWORDS.contains(&name)
        || KEYWORDS.contains(&name)
    {
        return bad(display, "name", "bad package name");
    }
    // a dependency manifest is reported under the package's real name, not its mount key
    let display: &str = &if display.starts_with("@pkg/") { format!("@pkg/{}/pruv.json", name) } else { display.to_string() };
    match parse_version(version) {
        Some(v) if v.iter().all(|x| *x <= MAX) => {}
        _ => return bad(display, "version", "bad version"),
    }
    if !plain_rel(root, false) || !root.ends_with(".pruv") {
        return bad(display, "root", "bad root path");
    }
    let rpath = join_rel(pkgdir, root);
    if !is_file(&rpath) {
        return bad(display, "root", "root file not found");
    }
    // the root file (after resolving symlinks) must stay inside the canonical package directory, by path components
    match std::fs::canonicalize(&rpath) {
        Ok(r) if r.starts_with(pkgdir) => {}
        _ => return bad(display, "root", "root file is outside the package directory"),
    }
    let mut deps: Vec<(String, PathBuf, String)> = Vec::new();
    match data.get("dependencies") {
        None => {}
        Some(Jv::Obj(dd)) => {
            if let Some(d) = dd.dups.first() {
                return bad(display, &format!("dependencies.{}", d), "duplicate key");
            }
            for (key, ent) in &dd.items {
                let kp = format!("dependencies.{}", key);
                // the key must be a name that `mod KEY;` can declare (same predicate as the package name)
                if !is_ident(key, true)
                    || key == "std"
                    || key.contains("__")
                    || key.starts_with("pruv_")
                    || key == "crate"
                    || RUST_KEYWORDS.contains(&key.as_str())
                    || KEYWORDS.contains(&key.as_str())
                {
                    return bad(display, &kp, "bad mount name");
                }
                let ent = match ent {
                    Jv::Obj(o) => o,
                    _ => return bad(display, &kp, "expected an object"),
                };
                check_obj(ent, &kp, &["path", "version"], display)?;
                for k in ["path", "version"] {
                    str_of(ent, k, &format!("{}.{}", kp, k), display)?;
                }
                let p = str_of(ent, "path", &kp, display)?;
                let req = str_of(ent, "version", &kp, display)?;
                if !plain_rel(p, true) {
                    return bad(display, &format!("{}.path", kp), "bad dependency path");
                }
                let ddir = realpath(&join_rel(pkgdir, p));
                if !is_file(&ddir.join("pruv.json")) {
                    return bad(display, &format!("{}.path", kp), "no pruv.json in the dependency directory");
                }
                match parse_req(req) {
                    Some(v) if v.iter().all(|x| *x <= MAX) => {}
                    _ => return bad(display, &format!("{}.version", kp), "bad version requirement"),
                }
                deps.push((key.clone(), ddir, req.to_string()));
            }
        }
        Some(_) => return bad(display, "dependencies", "expected an object"),
    }
    Ok(Pkg {
        name: name.to_string(),
        version: version.to_string(),
        dir: pkgdir.to_path_buf(),
        root: root.to_string(),
        deps,
        display: display.to_string(),
        reqs: Vec::new(),
        first: None,
    })
}

/// the compatibility line of a version: the major when it is at least 1, otherwise 0.MINOR
fn line(v: &[u64]) -> Vec<u64> {
    if v[0] >= 1 {
        vec![v[0]]
    } else {
        vec![0, v[1]]
    }
}

/// a requirement padded with zeros to MAJOR.MINOR.PATCH
fn pad(req: &str) -> Vec<u64> {
    let mut v = parse_req(req).unwrap_or_default();
    while v.len() < 3 {
        v.push(0);
    }
    v
}

fn visit(g: &mut Graph, stack: &mut Vec<usize>, pkg: usize) -> Res<()> {
    let deps = g.pkgs[pkg].deps.clone();
    for (key, ddir, req) in deps {
        for (i, &s) in stack.iter().enumerate() {
            if g.pkgs[s].dir == ddir {
                let mut names: Vec<&str> = stack[i..].iter().map(|x| g.pkgs[*x].name.as_str()).collect();
                names.push(g.pkgs[s].name.as_str());
                return Err(PkgError::new(
                    "dependency_cycle",
                    format!("dependency cycle: {}", names.join(" -> ")),
                    "remove one of the dependencies on the cycle".to_string(),
                    &g.pkgs[pkg].display,
                ));
            }
        }
        match g.by_dir.get(&ddir).copied() {
            None => {
                let d = read(&ddir, &ddir.join("pruv.json"), &format!("@pkg/{}/pruv.json", key))?;
                for o in &g.pkgs {
                    if o.name == d.name {
                        let a = match &o.first {
                            None => format!("the program package {} (version {})", o.name, o.version),
                            Some((who, k)) => {
                                format!("required by {} as {} (version {})", g.pkgs[*who].name, k, o.version)
                            }
                        };
                        return Err(PkgError::new(
                            "identity_conflict",
                            format!(
                                "package {} is loaded from two places: {} and by {} as {} (version {})",
                                d.name, a, g.pkgs[pkg].name, key, d.version
                            ),
                            format!("point every dependency on {} at one directory", d.name),
                            &d.display,
                        ));
                    }
                }
                let mut d = d;
                d.first = Some((pkg, key.clone()));
                d.reqs.push((pkg, key.clone(), req.clone()));
                let di = g.pkgs.len();
                g.pkgs.push(d);
                g.by_dir.insert(ddir.clone(), di);
                stack.push(di);
                visit(g, stack, di)?;
                stack.pop();
            }
            Some(di) => g.pkgs[di].reqs.push((pkg, key.clone(), req.clone())),
        }
    }
    Ok(())
}

/// Validates the program manifest and every dependency manifest; checks the graph (cycles, one directory per name,
/// version requirements). `manifest_path` is the path given on the command line.
pub fn load_graph(manifest_path: &Path) -> Res<Graph> {
    let mpath = abspath(manifest_path);
    let dir = mpath.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"));
    let raw = manifest_path.to_string_lossy().into_owned();
    let display = raw.rsplit('/').next().unwrap_or("").to_string();
    let prog = read(&realpath(&dir), &mpath, &display)?;
    let mut g = Graph { pkgs: Vec::new(), by_dir: HashMap::new() };
    g.by_dir.insert(prog.dir.clone(), 0);
    g.pkgs.push(prog);
    let mut stack = vec![0usize];
    visit(&mut g, &mut stack, 0)?;
    for p in &g.pkgs {
        let found = parse_version(&p.version).unwrap_or_default();
        let bad_first = p.reqs.iter().find(|r| {
            let want = pad(&r.2);
            !(line(&found) == line(&want) && found >= want)
        });
        if let Some(b) = bad_first {
            let listed: Vec<String> = p.reqs.iter().map(|r| format!("{} requires {}", g.pkgs[r.0].name, r.2)).collect();
            return Err(PkgError::new(
                "version_unsatisfied",
                format!("package {} version {} does not satisfy: {}", p.name, p.version, listed.join(", ")),
                "change the requirement or the dependency's version".to_string(),
                &g.pkgs[b.0].display,
            ));
        }
    }
    Ok(g)
}

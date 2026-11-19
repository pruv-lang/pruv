//! G4b-3 proof cache (docs/G4B_SPEC.md section 7): a local, content-addressed cache of solver `unsat` answers.
//!
//! Key: lowercase hex SHA-256 of `<compiler id>\0<solver>\0<query text>`. Entry file: `DIR/<key[..2]>/<key>` holding exactly
//! `pruv-proof-cache 1\nunsat\n`. Single-threaded (counters are `Cell`s); share through `Rc`.

use std::cell::Cell;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// The exact bytes of a valid entry.
const ENTRY: &[u8] = b"pruv-proof-cache 1\nunsat\n";

fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == 0x0b || c == 0x0c
}

/// `<prefix>!<lowercase letters><digits>` with a nonempty prefix, split at the last `!`: `<prefix>!<letters>`.
fn fresh_prefix(sym: &str) -> Option<&str> {
    let k = sym.rfind('!')?;
    let tag = &sym.as_bytes()[k + 1..];
    let l = tag.iter().take_while(|d| d.is_ascii_lowercase()).count();
    if k > 0 && l < tag.len() && tag[l..].iter().all(|d| d.is_ascii_digit()) {
        Some(&sym[..k + 1 + l])
    } else {
        None
    }
}

fn is_let(sym: &str) -> bool {
    let b = sym.as_bytes();
    b.len() > 2 && (b[0] == b'?' || b[0] == b'$') && b[1] == b'x' && b[2..].iter().all(|d| d.is_ascii_digit())
}

/// docs/G4B_SPEC.md 7 rule 9 (proof_cache.py `canonical`): (canonical text, fresh names). Splits the text into SMT-LIB
/// lexemes as `Z3_benchmark_to_smtlib_string` prints them (quoted symbol `|...|`, string literal `"..."` with `""` as two
/// adjacent literals, `;` comment, an unclosed quote or literal running to the end, or an atom of the bytes other than
/// whitespace, `(`, `)`, `|`, `"`, `;`) and renumbers by first appearance, one counter each: the let names `?x<id>` /
/// `$x<id>` (atoms, sigil kept) and the fresh names `<prefix>!<letters><n>` (atoms or quoted symbols; prefix, lowercase
/// letters and quoting kept).
/// The fresh names and let-form names that first appear before the `assert` command (the declarations; never a bound
/// variable alone or a let name) are returned in local order, without quotes.
pub fn canonical(text: &str) -> (String, Vec<String>) {
    let b = text.as_bytes();
    let mut lets: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut fresh: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    // true once the atom `assert` has been seen: fresh names first seen after it are not returned
    let mut body = false;
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if is_ws(c) || c == b'(' || c == b')' {
            let j = i + b[i..].iter().position(|&d| !(is_ws(d) || d == b'(' || d == b')')).unwrap_or(b.len() - i);
            out.push_str(&text[i..j]);
            i = j;
            continue;
        }
        if c == b'"' || c == b';' {
            let close = if c == b';' { b'\n' } else { b'"' };
            let j = match b[i + 1..].iter().position(|&d| d == close) {
                // a comment stops before its newline; a literal includes its closing quote
                Some(k) => i + 1 + k + usize::from(c == b'"'),
                None => b.len(),
            };
            out.push_str(&text[i..j]);
            i = j;
            continue;
        }
        let (j, sym, quoted) = if c == b'|' {
            match b[i + 1..].iter().position(|&d| d == b'|') {
                Some(k) => (i + k + 2, Some(&text[i + 1..i + 1 + k]), true),
                None => (b.len(), None, true),
            }
        } else {
            let j = i + b[i..]
                .iter()
                .position(|&d| is_ws(d) || d == b'(' || d == b')' || d == b'|' || d == b'"' || d == b';')
                .unwrap_or(b.len() - i);
            (j, Some(&text[i..j]), false)
        };
        match sym {
            // the command: each command is printed at the start of a line, a declared symbol `assert` never is
            Some("assert") if !quoted && i > 0 && b[i - 1] == b'(' && (i == 1 || b[i - 2] == b'\n') => {
                body = true;
                out.push_str("assert");
            }
            Some(sym) if !quoted && is_let(sym) => {
                let n = lets.len() + 1;
                let k = *lets.entry(sym).or_insert_with(|| {
                    if !body {
                        // a declared symbol of the let form: shared across the query like a fresh name
                        order.push(sym.to_string());
                    }
                    n
                });
                out.push(c as char);
                out.push('x');
                out.push_str(&k.to_string());
            }
            Some(sym) if fresh_prefix(sym).is_some() => {
                let pre = fresh_prefix(sym).unwrap();
                let n = fresh.len() + 1;
                let k = *fresh.entry(sym).or_insert_with(|| {
                    if !body {
                        order.push(sym.to_string());
                    }
                    n
                });
                if quoted {
                    out.push('|');
                }
                out.push_str(pre);
                out.push_str(&k.to_string());
                if quoted {
                    out.push('|');
                }
            }
            _ => out.push_str(&text[i..j]),
        }
        i = j;
    }
    (out, order)
}

/// docs/G4B_SPEC.md 7 rule 9 (proof_cache.py `piece`): (lowercase hex SHA-256 of the canonical text, fresh names in
/// local order).
pub fn piece(text: &str) -> (String, Vec<String>) {
    let (c, names) = canonical(text);
    (hex(&Sha256::digest(c.as_bytes())), names)
}

fn hex(digest: &[u8]) -> String {
    let mut s = String::with_capacity(2 * digest.len() + 1);
    for b in digest {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[derive(Debug)]
pub struct ProofCache {
    dir: PathBuf,
    /// `rs <version>\0<solver>\0`
    prefix: String,
    hits: Cell<u64>,
    misses: Cell<u64>,
    stores: Cell<u64>,
    /// Temporary file sequence number.
    n: Cell<u64>,
}

impl ProofCache {
    /// `version` is the compiler version (the `pruvc` crate version, "0.0.1").
    pub fn new(dir: PathBuf, version: &str) -> ProofCache {
        let prefix = format!("rs {}\0{}\0", version, crate::sys::full_version());
        ProofCache { dir, prefix, hits: Cell::new(0), misses: Cell::new(0), stores: Cell::new(0), n: Cell::new(0) }
    }

    /// Lowercase hex SHA-256 of `prefix + text`.
    pub fn key(&self, text: &str) -> String {
        let mut h = Sha256::new();
        h.update(self.prefix.as_bytes());
        h.update(text.as_bytes());
        hex(&h.finalize())
    }

    /// True only when the entry file's bytes are exactly the valid entry. Counts a hit.
    pub fn lookup(&self, text: &str) -> bool {
        let key = self.key(text);
        let path = self.dir.join(&key[..2]).join(&key);
        match std::fs::read(path) {
            Ok(b) if b == ENTRY => {
                self.hits.set(self.hits.get() + 1);
                true
            }
            _ => false,
        }
    }

    /// Writes the entry (temporary file + rename). Any io error drops the store silently. Counts a store on success.
    pub fn store(&self, text: &str) {
        let key = self.key(text);
        let sub = self.dir.join(&key[..2]);
        if std::fs::create_dir_all(&sub).is_err() {
            return;
        }
        let n = self.n.get();
        self.n.set(n + 1);
        let tmp = sub.join(format!(".tmp.{}.{}", std::process::id(), n));
        if std::fs::write(&tmp, ENTRY).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        if std::fs::rename(&tmp, sub.join(&key)).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        self.stores.set(self.stores.get() + 1);
    }

    /// An attempt with the cache on that called the solver.
    pub fn count_miss(&self) {
        self.misses.set(self.misses.get() + 1);
    }

    pub fn hits(&self) -> u64 {
        self.hits.get()
    }
    pub fn misses(&self) -> u64 {
        self.misses.get()
    }
    pub fn stores(&self) -> u64 {
        self.stores.get()
    }
}

// ---------------------------------------------------------------------------
// PR1: module proof records, `--module-cache DIR` (docs/G4B_SPEC.md section 7.8)
// ---------------------------------------------------------------------------

pub const MODULE_ENTRY: &[u8] = b"pruv-module-proof 1\nproved\n";

static START: std::sync::OnceLock<std::time::SystemTime> = std::sync::OnceLock::new();

/// Records the process start time (call first thing in `main`): `code_identity` refuses a binary modified after it.
pub fn note_start() {
    START.get_or_init(std::time::SystemTime::now);
}

/// Compiler code identity for module keys. Linux: SHA-256 of the running binary read through /proc/self/exe (it embeds
/// the bundled std and every built-in catalog); None (module records off) when it cannot be read, when it was modified
/// after `note_start`, or when it changed while being hashed. Other targets: the build-time hash of every compiler
/// input (build.rs, PRUV_BUILD_ID), because re-opening the executable by path races with a replacement of that path.
/// Call it early (`main` does when `--module-cache` is given).
pub fn code_identity() -> Option<String> {
    #[cfg(not(target_os = "linux"))]
    {
        // Re-opening the executable by path races with a replacement of that path, so other targets use the
        // build-time identity of every compiler input (build.rs).
        Some(format!("build:{}", env!("PRUV_BUILD_ID")))
    }
    #[cfg(target_os = "linux")]
    {
        static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        ID.get_or_init(|| {
            let exe = PathBuf::from("/proc/self/exe");
            let stamp = |m: &std::fs::Metadata| -> Option<(std::time::SystemTime, u64, u64)> {
                Some((m.modified().ok()?, m.len(), std::os::unix::fs::MetadataExt::ino(m)))
            };
            let before = stamp(&std::fs::metadata(&exe).ok()?)?;
            let start = *START.get_or_init(std::time::SystemTime::now);
            if before.0 > start {
                return None;
            }
            let bytes = std::fs::read(&exe).ok()?;
            let after = stamp(&std::fs::metadata(&exe).ok()?)?;
            if after != before || bytes.len() as u64 != before.1 {
                return None;
            }
            Some(hex(&Sha256::digest(&bytes)))
        })
        .clone()
    }
}

/// Module proof records under DIR/<key[:2]>/<key>. A record means: every function of the module verified with no error
/// under exactly the inputs hashed into the key.
#[derive(Debug)]
pub struct ModuleRecords {
    dir: PathBuf,
    /// `rs <version>\0<binary sha256>\0<solver>\0`; None = the binary could not be hashed (never hit, never store)
    prefix: Option<String>,
    hits: Cell<u64>,
    misses: Cell<u64>,
    stores: Cell<u64>,
    n: Cell<u64>,
}

impl ModuleRecords {
    pub fn new(dir: PathBuf, version: &str) -> ModuleRecords {
        let prefix = code_identity().map(|c| format!("rs {}\0{}\0{}\0", version, c, crate::sys::full_version()));
        ModuleRecords { dir, prefix, hits: Cell::new(0), misses: Cell::new(0), stores: Cell::new(0), n: Cell::new(0) }
    }

    pub fn key(&self, text: &str) -> Option<String> {
        let p = self.prefix.as_ref()?;
        let mut h = Sha256::new();
        h.update(p.as_bytes());
        h.update(text.as_bytes());
        Some(hex(&h.finalize()))
    }

    /// True only for a well-formed record. Counts a hit or a miss.
    pub fn lookup(&self, key: Option<&str>) -> bool {
        let ok = match key {
            Some(k) => read_bounded(&self.dir.join(&k[..2]).join(k)).is_some_and(|b| b == MODULE_ENTRY),
            None => false,
        };
        if ok {
            self.hits.set(self.hits.get() + 1);
        } else {
            self.misses.set(self.misses.get() + 1);
        }
        ok
    }

    /// Writes the record (temporary file + rename); any io error drops it silently. Counts a store on success.
    pub fn store(&self, key: Option<&str>) {
        let Some(key) = key else { return };
        let sub = self.dir.join(&key[..2]);
        if std::fs::create_dir_all(&sub).is_err() {
            return;
        }
        let n = self.n.get();
        self.n.set(n + 1);
        let tmp = sub.join(format!(".tmp.{}.{}", std::process::id(), n));
        // exclusive create: an existing file of that name (another writer) is never shared or truncated
        let file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => f,
            Err(_) => return,
        };
        let ok = {
            use std::io::Write;
            let mut f = file;
            f.write_all(MODULE_ENTRY).is_ok()
        };
        if !ok || std::fs::rename(&tmp, sub.join(key)).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        self.stores.set(self.stores.get() + 1);
    }

    pub fn hits(&self) -> u64 {
        self.hits.get()
    }
    pub fn misses(&self) -> u64 {
        self.misses.get()
    }
    pub fn stores(&self) -> u64 {
        self.stores.get()
    }
}

/// At most `MODULE_ENTRY.len() + 1` bytes of the record file (a longer file is malformed anyway).
fn read_bounded(p: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut b = Vec::with_capacity(MODULE_ENTRY.len() + 1);
    std::fs::File::open(p).ok()?.take(MODULE_ENTRY.len() as u64 + 1).read_to_end(&mut b).ok()?;
    Some(b)
}

/// What `verify_program_opts` needs to skip and record whole modules.
#[derive(Debug)]
pub struct ModulePlan {
    pub recs: std::rc::Rc<ModuleRecords>,
    /// parallel to `Program.functions`: the index of the defining module
    pub fn_module: Vec<usize>,
    /// per module: the key text of its record
    pub key_texts: Vec<String>,
    /// set on first use: a plan describes one program, so a second use (a library caller reusing the options on another
    /// program) verifies everything instead of trusting keys built for the first
    pub spent: Cell<bool>,
}

/// Builds the plan of an explicit-module program (`g4` resolved). Key text of module `mi`: the format line, explicit
/// mode, `--infer`, G9 activation with the G9 metadata entries that mention an item of the module's closure (an entry
/// naming only outside items cannot reach the closure's encoding), the module itself, then one line per module of its
/// closure (transitive `Module.deps`, sorted, itself included) with its source digest and resolved-items digest.
pub fn module_plan(
    prog: &pruv_core::ast::Program,
    g4: &pruv_core::modules::G4,
    recs: std::rc::Rc<ModuleRecords>,
    infer: bool,
) -> Option<ModulePlan> {
    if !g4.resolved || g4.fn_meta.len() != prog.functions.len() {
        return None;
    }
    let mods = &g4.modules;
    let known: std::collections::HashSet<&str> =
        prog.functions.iter().map(|f| f.name.as_str()).chain(prog.types.iter().map(|d| d.name())).collect();
    let mut item_mod: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, f) in prog.functions.iter().enumerate() {
        item_mod.insert(f.name.as_str(), g4.fn_meta[i].module);
    }
    for (i, d) in prog.types.iter().enumerate() {
        item_mod.insert(d.name(), g4.ty_meta[i].module);
    }
    let mut key_texts = Vec::with_capacity(mods.len());
    for mi in 0..mods.len() {
        let mut seen = vec![false; mods.len()];
        seen[mi] = true;
        let mut stack = vec![mi];
        while let Some(j) = stack.pop() {
            for &d in &mods[j].deps {
                if !seen[d] {
                    seen[d] = true;
                    stack.push(d);
                }
            }
        }
        let g9 = match &prog.g9 {
            None => "off".to_string(),
            Some(meta) => {
                let mentions = |x: &str| -> bool {
                    let head = x.split('.').next().unwrap_or(x);
                    let up = head.rsplit_once("::").map_or(head, |p| p.0);
                    for c in [head, up] {
                        if known.contains(c) {
                            return item_mod.get(c).is_some_and(|&m| seen[m]);
                        }
                    }
                    true // not an item identity: keep it (conservative)
                };
                let one = |v: &Vec<String>| -> Vec<String> { v.iter().filter(|x| mentions(x)).cloned().collect() };
                let many = |v: &Vec<Vec<String>>| -> Vec<Vec<String>> {
                    v.iter().filter(|xs| xs.iter().any(|x| mentions(x))).cloned().collect()
                };
                format!(
                    "boxed={:?} extended={:?} fn_sccs={:?} type_sccs={:?}",
                    one(&meta.boxed),
                    one(&meta.extended),
                    many(&meta.fn_sccs),
                    many(&meta.type_sccs)
                )
            }
        };
        let m = &mods[mi];
        let mut t = format!(
            "pruv-module-proof 1\nexplicit 1\ninfer {}\ng9 {}\nmodule {}\0{}\n",
            infer as u8,
            g9,
            m.pkg.as_deref().unwrap_or(""),
            m.path.join("::")
        );
        for (j, d) in mods.iter().enumerate() {
            if seen[j] {
                t.push_str(&format!(
                    "dep {}\0{}\0{}\0{}\n",
                    d.pkg.as_deref().unwrap_or(""),
                    d.path.join("::"),
                    d.digest,
                    d.rdigest
                ));
            }
        }
        key_texts.push(t);
    }
    Some(ModulePlan { recs, fn_module: g4.fn_meta.iter().map(|m| m.module).collect(), key_texts, spent: Cell::new(false) })
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn renumbers_let_and_fresh_names() {
        let t = "(let (($x143 (f ?x24 k!14 |a b!3| i!4 k!14)))\n (! (|q ?x5| ?x5 \"?x5 \"\" k!2\" ?x7 ; ?x5 k!9\n?x5 |a b!3| x! !3 a!b!7 a!0x ?x24)))";
        let (c, names) = canonical(t);
        assert_eq!(c, "(let (($x1 (f ?x2 k!1 |a b!2| i!3 k!1)))\n (! (|q ?x5| ?x3 \"?x5 \"\" k!2\" ?x4 ; ?x5 k!9\n?x3 |a b!2| x! !3 a!b!4 a!0x ?x2)))");
        // no `assert` command: every name is a declaration
        assert_eq!(names, vec!["$x143", "?x24", "k!14", "a b!3", "i!4", "?x5", "?x7", "a!b!7"]);
        assert_eq!(canonical("a?x1 ?x1b ?x ?y1 $x2)").0, "a?x1 ?x1b ?x ?y1 $x1)");
        assert_eq!(canonical("?x3 |a ?x3").0, "?x1 |a ?x3");
        assert_eq!(canonical("|a|?x3 ?x3").0, "|a|?x1 ?x1");
        assert_eq!(canonical("\"ab").0, "\"ab");
        let (c, names) = canonical("(forall ((j!q14 Int)) (= j!q14 j!dk3 m1buf!int !q2 a!Q2 a!q k!7))");
        assert_eq!(c, "(forall ((j!q1 Int)) (= j!q1 j!dk2 m1buf!int !q2 a!Q2 a!q k!3))");
        assert_eq!(names, vec!["j!q14", "j!dk3", "k!7"]);
        let (c, names) = canonical("(declare-fun k!7 () Int)\n(assert (forall ((j!q14 Int)) (= j!q14 k!7 i!2)))");
        assert_eq!(c, "(declare-fun k!1 () Int)\n(assert (forall ((j!q2 Int)) (= j!q2 k!1 i!3)))");
        assert_eq!(names, vec!["k!7"]);
        // a declared `?x7` is shared across the query; a declared `assert` does not end the declarations
        let (c, names) = canonical("(declare-fun ?x7 () Int)\n(declare-fun assert () Int)\n(declare-fun x!8 () Int)\n(assert (= ?x7 assert x!8))");
        assert_eq!(c, "(declare-fun ?x1 () Int)\n(declare-fun assert () Int)\n(declare-fun x!1 () Int)\n(assert (= ?x1 assert x!1))");
        assert_eq!(names, vec!["?x7", "x!8"]);
    }
}

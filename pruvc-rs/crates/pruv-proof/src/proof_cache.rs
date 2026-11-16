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

//! P9b (R-6x-4): the Rust shape of impl blocks. Every pass up to and including the inline policy sees the impl
//! functions as the free functions `Owner__name` (one representation, the same inlining decisions); this last pass
//! over the finished Rust text prints them as Rust writes them by hand:
//!
//! - the items of `Owner__*` functions (with their attribute lines) go into `impl Owner { .. }` blocks (consecutive
//!   items share one block) named without the prefix, and the receiver parameter prints as `self` / `&self` /
//!   `&mut self` / `mut self`;
//! - a call of a function with a receiver prints as the method call `recv.name(args)` (a leading `&` / `&mut ` of the
//!   printed receiver is dropped: method auto-referencing restores it); every other use prints as `Owner::name`.
//!
//! `__` is reserved in source names, so an identifier `Owner__..` of a type with methods is always one of these
//! generated names. String, raw string and char literals are copied unchanged (panic texts keep the internal name,
//! as the LLVM backend prints it).

use crate::ast::{Program, Receiver};
use std::collections::HashSet;

/// the suffixes of the generated items that take the receiver first (the function itself, its checked body and the
/// entry checks); other generated items of a method are called by path
const DOT_SUFFIXES: [&str; 4] = ["", "__pruv_body", "__pruv_chk", "__pruv_par_chk"];

struct Shape {
    /// (owner, name) of every impl function
    all: HashSet<(String, String)>,
    /// (owner, method name) of the impl functions with a receiver
    recv: HashSet<(String, String)>,
    /// generated helpers that take the receiver after other parameters: they stay free functions
    free: HashSet<String>,
}

pub(crate) fn impl_shape(text: String, prog: &Program) -> String {
    if prog.methods.is_empty() {
        return text;
    }
    let mut sh = Shape {
        all: prog.methods.iter().map(|m| (m.owner.clone(), m.name.clone())).collect(),
        recv: prog.methods.iter().filter(|m| m.recv != Receiver::None).map(|m| (m.owner.clone(), m.name.clone())).collect(),
        free: HashSet::new(),
    };
    for l in text.split('\n') {
        if l.contains(", self: ") || l.contains(", mut self: ") {
            if let Some(name) = sh.item_name(l.trim_start()) {
                sh.free.insert(name.to_string());
            }
        }
    }
    let text = sh.calls(&text);
    sh.items(&text)
}

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// the end (exclusive) of the literal starting at i, when a string, raw string or char literal starts there
fn literal_end(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    if b[i] == b'"' {
        let mut j = i + 1;
        while j < n && b[j] != b'"' {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        return Some((j + 1).min(n));
    }
    if b[i] == b'r' && (i == 0 || !ident_byte(b[i - 1])) {
        let mut j = i + 1;
        while j < n && b[j] == b'#' {
            j += 1;
        }
        if j < n && b[j] == b'"' {
            let hashes = j - i - 1;
            let mut k = j + 1;
            while k < n {
                if b[k] == b'"' && k + hashes < n + 1 && b[k + 1..(k + 1 + hashes).min(n)].iter().all(|&c| c == b'#')
                    && k + 1 + hashes <= n
                {
                    return Some(k + 1 + hashes);
                }
                k += 1;
            }
            return Some(n);
        }
    }
    if b[i] == b'\'' {
        // 'x' or '\x..' (a lifetime `'a` is not a literal)
        if i + 2 < n && b[i + 1] != b'\\' && b[i + 2] == b'\'' {
            return Some(i + 3);
        }
        if i + 1 < n && b[i + 1] == b'\\' {
            let mut j = i + 2;
            while j < n && j < i + 12 && b[j] != b'\'' {
                j += 1;
            }
            if j < n && b[j] == b'\'' {
                return Some(j + 1);
            }
        }
        // a multi-byte char literal
        if i + 1 < n && b[i + 1] >= 0x80 {
            let mut j = i + 1;
            while j < n && j < i + 6 && b[j] != b'\'' {
                j += 1;
            }
            if j < n && b[j] == b'\'' {
                return Some(j + 1);
            }
        }
    }
    None
}

/// the index of the `)` closing the `(` at open, and the top-level comma-separated pieces between them
fn call_args(b: &[u8], open: usize) -> Option<(usize, Vec<(usize, usize)>)> {
    let mut depth = 0i64;
    let mut i = open;
    let mut start = open + 1;
    let mut pieces = Vec::new();
    while i < b.len() {
        if let Some(e) = literal_end(b, i) {
            i = e;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    if i > start || !pieces.is_empty() {
                        pieces.push((start, i));
                    }
                    return Some((i, pieces));
                }
            }
            b',' if depth == 1 => {
                pieces.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `x`, `x.f.g`, `(..)` as a whole: no parentheses needed in front of `.m(..)`
fn simple_receiver(r: &str) -> bool {
    if r.bytes().all(|c| ident_byte(c) || c == b'.') {
        return true;
    }
    let b = r.as_bytes();
    if b.first() == Some(&b'(') {
        if let Some((close, _)) = call_args(b, 0) {
            return close == b.len() - 1;
        }
    }
    false
}

impl Shape {
    /// (owner, rest) when `id` is a generated `Owner__name[__suffix]` name of an impl function `name` of `Owner`
    /// (other generated names such as the bindings' `Pruv__Array` are left alone). An owner may end with `_`
    /// (`A___get` is `A_` + `get`) and an impl function name never starts with `_` (parse/impl_syntax), so at most
    /// one split fits. The name may end with `_` (`A__get___pruv_body` is `get_` + `__pruv_body`): the name is the
    /// whole rest or the part before a `__` that is followed by a generated suffix (never starting with `_`).
    fn split<'a>(&self, id: &'a str) -> Option<(&'a str, &'a str)> {
        let seps = |s: &'a str| (1..s.len().saturating_sub(1)).filter(move |&k| s.as_bytes()[k] == b'_' && s.as_bytes()[k + 1] == b'_');
        seps(id).map(|k| (&id[..k], &id[k + 2..])).find(|(o, rest)| {
            !rest.is_empty() && !rest.starts_with('_')
                && std::iter::once(rest.len()).chain(seps(rest).filter(|&j| rest.as_bytes().get(j + 2).is_some_and(|&c| c != b'_')))
                    .any(|j| self.all.contains(&(o.to_string(), rest[..j].to_string())))
        })
    }

    fn dot_call(&self, owner: &str, rest: &str) -> bool {
        DOT_SUFFIXES.iter().any(|s| {
            rest.strip_suffix(s).is_some_and(|m| !m.is_empty() && !m.contains("__") && self.recv.contains(&(owner.to_string(), m.to_string())))
        })
    }

    fn calls(&self, text: &str) -> String {
        let b = text.as_bytes();
        let n = b.len();
        let mut out = String::with_capacity(n + n / 16);
        let mut i = 0;
        let mut copied = 0;
        while i < n {
            if let Some(e) = literal_end(b, i) {
                i = e;
                continue;
            }
            if !ident_byte(b[i]) || (i > 0 && ident_byte(b[i - 1])) {
                i += 1;
                continue;
            }
            let mut e = i;
            while e < n && ident_byte(b[e]) {
                e += 1;
            }
            let id = &text[i..e];
            let Some((owner, rest)) = self.split(id).filter(|_| !self.free.contains(id)) else {
                i = e;
                continue;
            };
            if i >= 3 && &b[i - 3..i] == b"fn " {
                i = e; // a definition: renamed by `items`
                continue;
            }
            out.push_str(&text[copied..i]);
            // `super::Owner__m(..)` (a module copy of a body) keeps the path form `super::Owner::m(..)`
            let qualified = i >= 2 && &b[i - 2..i] == b"::";
            if e < n && b[e] == b'(' && !qualified && self.dot_call(owner, rest) {
                if let Some((close, pieces)) = call_args(b, e) {
                    if let Some(&(s0, e0)) = pieces.first() {
                        let mut r = text[s0..e0].trim();
                        if let Some(x) = r.strip_prefix("&mut ") {
                            r = x.trim_start();
                        } else if let Some(x) = r.strip_prefix('&') {
                            r = x;
                        }
                        let r = self.calls(r);
                        if simple_receiver(&r) {
                            out.push_str(&r);
                        } else {
                            out.push('(');
                            out.push_str(&r);
                            out.push(')');
                        }
                        out.push('.');
                        out.push_str(rest);
                        out.push('(');
                        let others: Vec<String> = pieces[1..].iter().map(|&(s, t)| self.calls(text[s..t].trim())).collect();
                        out.push_str(&others.join(", "));
                        out.push(')');
                        i = close + 1;
                        copied = i;
                        continue;
                    }
                }
            }
            out.push_str(owner);
            out.push_str("::");
            out.push_str(rest);
            i = e;
            copied = e;
        }
        out.push_str(&text[copied..]);
        out
    }

    /// the owner and the item name when line `h` is the header of an `Owner__*` function item
    fn item_header<'a>(&self, h: &'a str) -> Option<(&'a str, &'a str)> {
        self.split(self.item_name(h)?)
    }

    /// the function name when line `h` starts a (possibly `pub` / `pub(crate)`) `fn` item
    fn item_name<'a>(&self, h: &'a str) -> Option<&'a str> {
        let p = if h.starts_with("pub fn ") {
            4
        } else if h.starts_with("pub(crate) fn ") {
            11
        } else {
            0
        };
        let r = h[p..].strip_prefix("fn ")?;
        let e = r.bytes().position(|c| !ident_byte(c)).unwrap_or(r.len());
        Some(&r[..e])
    }

    fn items(&self, text: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        // (first line incl. attributes, last line, owner)
        let mut items: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let Some((owner, _)) = self.item_header(lines[i]) else {
                i += 1;
                continue;
            };
            let h = lines[i];
            if h.contains(", self: ") || h.contains(", mut self: ") {
                i += 1; // a generated helper with the receiver later in its list stays a free function (renamed below)
                continue;
            }
            let mut start = i;
            while start > 0 && lines[start - 1].starts_with("#[") {
                start -= 1;
            }
            let mut end = i;
            if !(h.ends_with('}') && h.matches('{').count() == h.matches('}').count()) {
                while end < lines.len() && lines[end] != "}" {
                    end += 1;
                }
            }
            items.push((start, end.min(lines.len() - 1), owner.to_string()));
            i = end + 1;
        }
        let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2 * items.len());
        let mut next = 0;
        let mut k = 0;
        while k < items.len() {
            let owner = items[k].2.clone();
            let mut last = k;
            while last + 1 < items.len() && items[last + 1].2 == owner
                && lines[items[last].1 + 1..items[last + 1].0].iter().all(|l| l.is_empty())
            {
                last += 1;
            }
            out.extend(lines[next..items[k].0].iter().map(|l| l.to_string()));
            out.push(format!("impl {} {{", owner));
            for (li, l) in lines.iter().enumerate().take(items[last].1 + 1).skip(items[k].0) {
                if l.is_empty() {
                    out.push(String::new());
                    continue;
                }
                let l = if self.item_header(l).is_some() && items[k..=last].iter().any(|it| it.0 <= li && li <= it.1) {
                    receiver_header(&l.replacen(&format!("fn {}__", owner), "fn ", 1), &owner)
                } else {
                    l.to_string()
                };
                out.push(format!("    {}", l));
            }
            out.push("}".to_string());
            next = items[last].1 + 1;
            k = last + 1;
        }
        out.extend(lines[next..].iter().map(|l| l.to_string()));
        let mut text = out.join("\n");
        if text.contains(", self: ") || text.contains(", mut self: ") {
            text = rename_free_receivers(&text);
        }
        text
    }
}

/// `(self: &mut Owner` -> `(&mut self`, `(self: &Owner` -> `(&self`, `(self: Owner` -> `(self`, `(mut self: Owner` ->
/// `(mut self`
fn receiver_header(h: &str, owner: &str) -> String {
    let forms: [(String, &str); 4] = [
        (format!("(self: &mut {}", owner), "(&mut self"),
        (format!("(self: &{}", owner), "(&self"),
        (format!("(mut self: {}", owner), "(mut self"),
        (format!("(self: {}", owner), "(self"),
    ];
    for (from, to) in forms {
        if let Some(k) = h.find(&from) {
            let after = h.as_bytes().get(k + from.len()).copied();
            if matches!(after, Some(b',') | Some(b')')) {
                return format!("{}{}{}", &h[..k], to, &h[k + from.len()..]);
            }
        }
    }
    h.to_string()
}

/// A generated helper that takes the receiver after other parameters (the small-result variant) cannot be a method:
/// it stays a free function whose receiver binding is spelled `pruv_self`.
fn rename_free_receivers(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let h = lines[i];
        let free = !h.starts_with(' ') && h.contains("fn ") && (h.contains(", self: ") || h.contains(", mut self: "));
        if !free {
            out.push(h.to_string());
            i += 1;
            continue;
        }
        let mut end = i;
        if !(h.ends_with('}') && h.matches('{').count() == h.matches('}').count()) {
            while end < lines.len() && lines[end] != "}" {
                end += 1;
            }
        }
        for l in &lines[i..=end.min(lines.len() - 1)] {
            out.push(rename_self(l));
        }
        i = end + 1;
    }
    out.join("\n")
}

fn rename_self(l: &str) -> String {
    let b = l.as_bytes();
    let mut out = String::with_capacity(l.len() + 8);
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        if let Some(e) = literal_end(b, i) {
            i = e;
            continue;
        }
        if l[i..].starts_with("self") && (i == 0 || !ident_byte(b[i - 1])) && !b.get(i + 4).is_some_and(|&c| ident_byte(c) || c == b':' && b.get(i + 5) == Some(&b':')) {
            out.push_str(&l[copied..i]);
            out.push_str("pruv_self");
            i += 4;
            copied = i;
            continue;
        }
        i += 1;
    }
    out.push_str(&l[copied..]);
    out
}

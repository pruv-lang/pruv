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
//! generated names. String, raw string and char literals are copied unchanged (panic texts already name a method
//! `Owner::name`, as the LLVM backend prints it).
//!
//! P9b-2: the names are looked up per function (`prog.methods` carries the printed names: Rust stems in a program with
//! modules, such as `pruv_m_f_..` for the function and `pruv_m_t_..` for its owner type), so no name shape is assumed.

use crate::ast::{Program, Receiver};
use std::collections::{HashMap, HashSet};

/// the suffixes of the generated items that take the receiver first (the function itself, its checked body and the
/// entry checks); other generated items of a method are called by path
const DOT_SUFFIXES: [&str; 4] = ["", "__pruv_body", "__pruv_chk", "__pruv_par_chk"];

struct Shape {
    /// printed name of every impl function -> (printed owner type, method name, receiver)
    funcs: HashMap<String, (String, String, Receiver)>,
    /// generated helpers that take the receiver after other parameters: they stay free functions
    free: HashSet<String>,
}

/// Methods of prelude traits that the derived (`Clone, Copy, Debug, PartialEq, Eq`) or blanket impls give a user type
/// or a reference to it. A dot call `p.eq(o)` would let Rust's method probe pick the trait method (on `&mut C` or
/// `&C`) before the inherent one, so a method with one of these names keeps the path form `C::eq(p, o)`.
const PRELUDE_TRAIT_METHODS: &[&str] = &[
    "eq", "ne", "lt", "le", "gt", "ge", "cmp", "partial_cmp", "max", "min", "clamp", "clone", "clone_from",
    "to_owned", "clone_into", "to_string", "into", "try_into", "from", "try_from", "borrow", "borrow_mut", "as_ref",
    "as_mut", "fmt", "hash", "type_id", "into_iter", "assert_receiver_is_total_eq",
];

/// One generated item name `base[__suffix]` of the impl function `base`.
struct Hit<'a> {
    owner: &'a str,
    name: &'a str,
    /// the generated suffix (`""`, `__pruv_body`, ..)
    suffix: &'a str,
    recv: Receiver,
}

pub(crate) fn impl_shape(text: String, prog: &Program) -> String {
    if prog.methods.is_empty() {
        return text;
    }
    let mut sh = Shape {
        funcs: prog.methods.iter().map(|m| (m.func.clone(), (m.owner.clone(), m.name.clone(), m.recv))).collect(),
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
    // P9b-2: a postfix chain of paths and calls (`C::new(k)`, `b.twice()`) binds tighter than `.`, as in hand Rust
    if !b.first().is_some_and(|&c| ident_byte(c)) {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if ident_byte(c) || c == b'.' || c == b':' => i += 1,
            b'(' => match call_args(b, i) {
                Some((close, _)) => i = close + 1,
                None => return false,
            },
            _ => return false,
        }
    }
    true
}

impl Shape {
    /// The impl function and suffix when `id` is the printed name of an impl function or one of its generated items
    /// `name__suffix` (other generated names such as the bindings' `Pruv__Array` are left alone). `__` is reserved in
    /// source names and a generated suffix never starts with `_`, so the base is the whole name or the part before a
    /// `__` followed by another character than `_` (`A__get___pruv_body` is `A__get_` + `__pruv_body`).
    fn split<'a>(&'a self, id: &'a str) -> Option<Hit<'a>> {
        let b = id.as_bytes();
        let cuts = std::iter::once(id.len())
            .chain((1..id.len().saturating_sub(2)).filter(|&k| b[k] == b'_' && b[k + 1] == b'_' && b[k + 2] != b'_'));
        for k in cuts {
            if let Some((owner, name, recv)) = self.funcs.get(&id[..k]) {
                return Some(Hit { owner, name, suffix: &id[k..], recv: *recv });
            }
        }
        None
    }

    fn dot_call(h: &Hit) -> bool {
        h.recv != Receiver::None && DOT_SUFFIXES.contains(&h.suffix) && !PRELUDE_TRAIT_METHODS.contains(&h.name)
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
            let Some(hit) = self.split(id).filter(|_| !self.free.contains(id)) else {
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
            if e < n && b[e] == b'(' && !qualified && Self::dot_call(&hit) {
                if let Some((close, pieces)) = call_args(b, e) {
                    if let Some(&(s0, e0)) = pieces.first() {
                        let mut r = text[s0..e0].trim();
                        if let Some(x) = r.strip_prefix("&mut ") {
                            r = x.trim_start();
                        } else if let Some(x) = r.strip_prefix('&') {
                            r = x;
                        }
                        let mut r = self.calls(r);
                        // `(*p).m()` is `p.m()` in hand Rust: the dot call dereferences the place itself
                        if let Some(inner) = r.strip_prefix("(*").and_then(|x| x.strip_suffix(')')) {
                            if !inner.is_empty() && inner.bytes().all(ident_byte) {
                                r = inner.to_string();
                            }
                        }
                        if simple_receiver(&r) {
                            out.push_str(&r);
                        } else {
                            out.push('(');
                            out.push_str(&r);
                            out.push(')');
                        }
                        out.push('.');
                        out.push_str(hit.name);
                        out.push_str(hit.suffix);
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
            out.push_str(hit.owner);
            out.push_str("::");
            out.push_str(hit.name);
            out.push_str(hit.suffix);
            i = e;
            copied = e;
        }
        out.push_str(&text[copied..]);
        out
    }

    /// the impl function when line `h` is the header of one of its items
    fn item_header<'a>(&'a self, h: &'a str) -> Option<Hit<'a>> {
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
            let Some(hit) = self.item_header(lines[i]) else {
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
            items.push((start, end.min(lines.len() - 1), hit.owner.to_string()));
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
                let hit = self.item_header(l).filter(|_| items[k..=last].iter().any(|it| it.0 <= li && li <= it.1));
                let l = if let Some(hit) = hit {
                    let printed = self.item_name(l).unwrap_or_default();
                    let own = format!("fn {}{}", hit.name, hit.suffix);
                    receiver_header(&l.replacen(&format!("fn {}", printed), &own, 1), &owner)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::MethodItem;

    fn prog(methods: &[(&str, &str, &str, Receiver)]) -> Program {
        let mut p = Program::default();
        p.methods = methods
            .iter()
            .map(|&(owner, name, func, recv)| MethodItem {
                line: 1, col: 1, owner: owner.into(), name: name.into(), func: func.into(), recv, is_pub: false,
            })
            .collect();
        p
    }

    #[test]
    fn items_and_calls() {
        let p = prog(&[("C", "new", "C__new", Receiver::None), ("C", "get", "C__get", Receiver::Read),
            ("C", "bump", "C__bump", Receiver::Inout)]);
        let text = "fn C__new(k: i64) -> C {\n    C { n: k }\n}\nfn C__get(self: C) -> i64 {\n    self.n\n}\n\
fn C__bump(self: &mut C) {\n    (*self).n += 1;\n    C__get((*self));\n}\nfn f(k: i64) -> i64 {\n    let mut c: C = C__new(k);\n\
    C__bump(&mut c);\n    C__get(C__new(k)) + C__get(c)\n}";
        let out = impl_shape(text.to_string(), &p);
        assert!(out.contains("impl C {"), "{}", out);
        assert!(out.contains("fn new(k: i64) -> C {"), "{}", out);
        assert!(out.contains("fn get(self) -> i64 {"), "{}", out);
        assert!(out.contains("fn bump(&mut self) {"), "{}", out);
        assert!(out.contains("    self.get();"), "{}", out); // `(*self).get()` prints as hand Rust writes it
        assert!(out.contains("c.bump();"), "{}", out);
        assert!(out.contains("C::new(k).get() + c.get()"), "{}", out);
        assert!(!out.contains("C__"), "{}", out);
    }

    #[test]
    fn prelude_trait_names_keep_the_path_form() {
        // `self.eq(o)` on `self: &mut C` would pick `<&mut C as PartialEq>::eq` before the inherent `C::eq`
        let p = prog(&[("C", "eq", "C__eq", Receiver::Read), ("C", "same", "C__same", Receiver::Inout)]);
        let text = "fn C__eq(self: C, o: C) -> bool {\n    self.n == o.n\n}\n\
fn C__same(self: &mut C, o: C) -> bool {\n    C__eq((*self), o)\n}\nfn f(mut c: C) -> bool {\n    C__same(&mut c, c)\n}";
        let out = impl_shape(text.to_string(), &p);
        assert!(out.contains("fn eq(self, o: C) -> bool {"), "{}", out);
        assert!(out.contains("C::eq((*self), o)"), "{}", out);
        assert!(out.contains("c.same(c)"), "{}", out);
    }

    #[test]
    fn stems_and_suffixes() {
        // a program with modules: the printed names are stems, looked up per function
        let p = prog(&[("pruv_m_t_1_T", "area", "pruv_m_f_1_area", Receiver::Read)]);
        let text = "fn pruv_m_f_1_area__pruv_body(self: pruv_m_t_1_T) -> i64 {\n    1\n}\n\
fn g(t: pruv_m_t_1_T) -> i64 {\n    pruv_m_f_1_area__pruv_body(t) + 0\n}\nfn h() { let s = \"pruv_m_f_1_area(\"; }";
        let out = impl_shape(text.to_string(), &p);
        assert!(out.contains("impl pruv_m_t_1_T {"), "{}", out);
        assert!(out.contains("fn area__pruv_body(self) -> i64 {"), "{}", out);
        assert!(out.contains("t.area__pruv_body() + 0"), "{}", out);
        assert!(out.contains("\"pruv_m_f_1_area(\""), "literals are kept: {}", out);
    }

    #[test]
    fn receivers() {
        assert!(simple_receiver("c"));
        assert!(simple_receiver("c.f.g"));
        assert!(simple_receiver("C::new(k)"));
        assert!(simple_receiver("b.twice().get()"));
        assert!(simple_receiver("(a + b)"));
        assert!(!simple_receiver("a + b"));
        assert!(!simple_receiver("C { n: k }"));
        assert!(!simple_receiver("*p"));
    }
}

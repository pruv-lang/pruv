//! Error format (vast.PruvError). line/col are 1-based, in Unicode characters.

use crate::pyjson::{dumps, Json};
use std::sync::Mutex;

// ---------- G4 A0: source files of an explicit-mode program ----------
// Python tags every AST node with its `file`. Here the file is carried in the line number of the node instead: the
// lexer of module file number k (the root is 1, k = position in the module list + 1) starts counting at
// `1 + (k << 32)`. A legacy program has tag 0 everywhere, so its line numbers are the plain ones. Everything that prints
// a line goes through `plain_line`; `PruvError::to_json_value` also emits `"file"` for a tagged line.
pub const LINE_MASK: i64 = 0xFFFF_FFFF;

/// The line number without the file tag.
pub fn plain_line(l: i64) -> i64 {
    if l < 0 {
        l
    } else {
        l & LINE_MASK
    }
}

/// The file tag of a line number (0 = no file: legacy mode or a synthetic position).
pub fn line_tag(l: i64) -> usize {
    if l < 0 {
        0
    } else {
        (l >> 32) as usize
    }
}

/// `line` (plain or tagged) carrying the file tag `tag`.
pub fn tagged_line(line: i64, tag: usize) -> i64 {
    plain_line(line) | ((tag as i64) << 32)
}

static FILES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Registers the file names of the program being compiled (index = tag - 1); the empty list for a legacy program.
pub fn set_files(files: Vec<String>) {
    let mut g = FILES.lock().unwrap_or_else(|e| e.into_inner());
    *g = files;
}

/// The file name of a tag, if registered.
pub fn file_of_tag(tag: usize) -> Option<String> {
    if tag == 0 {
        return None;
    }
    let g = FILES.lock().unwrap_or_else(|e| e.into_inner());
    g.get(tag - 1).cloned()
}

// ---------- P9b-2: methods show under their written name ----------
// An impl method is lowered to the function `T__f` (`crate::m::T__f` in explicit mode). Diagnostics are built from
// function names, so `PruvError::to_json_value` turns each lowered name back into `T::f` in message and fix.
static METHOD_NAMES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Registers (lowered name, written name) of every method of the program being compiled; empty for none.
pub fn set_method_names(mut names: Vec<(String, String)>) {
    names.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    let mut g = METHOD_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    *g = names;
}

/// `text` with every whole-word lowered method name replaced by its written name.
pub fn method_text(text: &str) -> String {
    let g = METHOD_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_empty() || !text.contains("__") {
        return text.to_string();
    }
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < b.len() {
        if i == 0 || !word(b[i - 1]) {
            for (low, shown) in g.iter() {
                let e = i + low.len();
                if text[i..].starts_with(low.as_str()) && (e == b.len() || !word(b[e])) {
                    out.push_str(shown);
                    i = e;
                    continue 'scan;
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct PruvError {
    pub stage: String,
    pub line: i64,
    pub col: i64,
    pub code: String,
    pub message: String,
    pub fix: String,
    pub counterexample: Option<Json>,
}

impl PruvError {
    pub fn new(stage: &str, line: i64, col: i64, code: &str, message: impl Into<String>, fix: impl Into<String>) -> PruvError {
        PruvError {
            stage: stage.to_string(),
            line,
            col,
            code: code.to_string(),
            message: message.into(),
            fix: fix.into(),
            counterexample: None,
        }
    }

    /// Insertion order stage,line,col,code,message,fix[,file][,counterexample]; line/col are JSON numbers.
    /// G4: a tagged line number (explicit mode) prints as its plain line and adds `file` after `fix`.
    pub fn to_json_value(&self) -> Json {
        let tag = line_tag(self.line);
        let file = file_of_tag(tag);
        let mut kv = vec![
            ("stage".to_string(), Json::Str(self.stage.clone())),
            ("line".to_string(), Json::int(plain_line(self.line))),
            ("col".to_string(), Json::int(self.col)),
            ("code".to_string(), Json::Str(self.code.clone())),
            ("message".to_string(), Json::Str(method_text(&self.message))),
            ("fix".to_string(), Json::Str(method_text(&self.fix))),
        ];
        if let Some(f) = file {
            kv.push(("file".to_string(), Json::Str(f)));
        }
        if let Some(c) = &self.counterexample {
            kv.push(("counterexample".to_string(), c.clone()));
        }
        Json::Obj(kv)
    }

    /// One line identical to Python `json.dumps(d, ensure_ascii=False)`.
    pub fn to_cli_json(&self) -> String {
        dumps(&self.to_json_value())
    }
}

impl std::fmt::Display for PruvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PruvError {}

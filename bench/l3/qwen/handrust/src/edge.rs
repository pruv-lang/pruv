//! Hand Rust baseline for the Qwen edge reads (TS readQwenHead and tailInfo over readEdgeLines).
//! Same serde_json shape as `window_import`: split the edge buffer at LF, parse each line into
//! an object, read the few fields by key. Pruv `import::read_head` and `import::tail_info` follow this.

use super::{es_trim, iso, one_line, parse_line, Object, Value};

/// TS IMPORT_LIMITS.edge
pub const EDGE_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Head {
    pub cwd: Option<String>,
    pub first_prompt: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tail {
    pub title: Option<String>,
    pub last_at: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

/// TS humanText: a real user line (provenance missing or real_user), its non-thought text parts
/// joined with a blank line, kept when not blank.
fn human_text(entry: &Object) -> Option<String> {
    if entry.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    if entry.get("provenance").is_some_and(|v| v.as_str() != Some("real_user")) {
        return None;
    }
    let parts = entry.get("message")?.get("parts")?.as_array()?;
    let joined = parts.iter().filter_map(Value::as_object)
        .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    if es_trim(&joined).is_empty() { None } else { Some(joined) }
}

/// TS readEdgeLines('head'): the head buffer split at LF; a cut buffer drops its last piece.
pub fn head_import(data: &[u8], cut_after: bool) -> Result<Head, TooLarge> {
    if data.len() > EDGE_BYTES {
        return Err(TooLarge);
    }
    let mut lines: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
    if cut_after {
        lines.pop();
    }
    let mut head = Head { cwd: None, first_prompt: None };
    let mut scratch = Vec::new();
    for line in lines.into_iter().filter(|l| !l.is_empty()) {
        let Some(entry) = parse_line(line, &mut scratch) else { continue };
        if head.cwd.is_none() {
            if let Some(Value::String(cwd)) = entry.get("cwd") {
                head.cwd = Some(cwd.clone());
            }
        }
        if head.first_prompt.is_none() {
            head.first_prompt = human_text(&entry);
        }
        if head.cwd.is_some() && head.first_prompt.is_some() {
            break;
        }
    }
    Ok(head)
}

/// TS readEdgeLines('tail') + tailInfo: a cut buffer skips its first piece; the last custom title
/// and the last readable timestamp win.
pub fn tail_import(data: &[u8], cut_before: bool) -> Result<Tail, TooLarge> {
    if data.len() > EDGE_BYTES {
        return Err(TooLarge);
    }
    let mut lines = data.split(|&b| b == b'\n');
    if cut_before {
        lines.next();
    }
    let mut tail = Tail { title: None, last_at: None };
    let mut best = None;
    let mut scratch = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let Some(entry) = parse_line(line, &mut scratch) else { continue };
        if let Some(at) = entry.get("timestamp").and_then(iso) {
            tail.last_at = Some(at);
        }
        if entry.get("type").and_then(Value::as_str) == Some("system")
            && entry.get("subtype").and_then(Value::as_str) == Some("custom_title")
        {
            if let Some(text) = entry.get("systemPayload")
                .and_then(|payload| payload.get("customTitle"))
                .and_then(Value::as_str)
                .filter(|text| !es_trim(text).is_empty())
            {
                best = Some(text.to_owned());
            }
        }
    }
    // Pruv: one_line once on the last raw title (row T1)
    tail.title = best.map(|t| one_line(&t, 120));
    Ok(tail)
}

#![forbid(unsafe_code)]

use serde_json::{json, Value};
use std::borrow::Cow;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

const MAX_PAGES: usize = 40;
const MAX_TURNS: usize = 200;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FIELD: usize = 32 * 1024;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Rpc {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    line: String,
}

impl Rpc {
    fn send(&mut self, request: Value) -> Result<()> {
        writeln!(self.input, "{request}")?;
        Ok(())
    }

    fn call(&mut self, id: usize, method: &str, params: Value) -> Result<Value> {
        self.send(json!({"id": id, "method": method, "params": params}))?;
        self.line.clear();
        if self.output.read_line(&mut self.line)? == 0 {
            return Err(format!("{method}: replay closed stdout").into());
        }
        let mut response: Value = serde_json::from_str(&self.line)?;
        if response["id"].as_u64() != Some(id as u64) || response.get("result").is_none() {
            return Err(format!("{method}: unexpected response").into());
        }
        Ok(response["result"].take())
    }
}

#[derive(Default)]
struct Turn {
    prompt: String,
    reply: String,
    tools: usize,
    created: Option<String>,
    completed: Option<String>,
    clipped: bool,
}

fn item_text(item: &Value) -> Cow<'_, str> {
    if let Some(text) = item["text"].as_str() {
        return Cow::Borrowed(text);
    }
    let Some(parts) = item["content"].as_array() else {
        return Cow::Borrowed("");
    };
    Cow::Owned(parts.iter().filter_map(|part| part["text"].as_str())
        .filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n\n"))
}

fn append(target: &mut String, text: &str) {
    if !target.is_empty() {
        target.push_str("\n\n");
    }
    target.push_str(text);
}

fn clip(text: &mut String) -> bool {
    if text.len() <= MAX_FIELD {
        return false;
    }
    let mut end = MAX_FIELD;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

fn read_turn(raw: &Value) -> Option<(Turn, usize)> {
    let mut turn = Turn::default();
    let mut reply_bytes = 0;
    for item in raw["items"].as_array()? {
        if !item.is_object() {
            continue;
        }
        match item["type"].as_str() {
            Some("userMessage") => {
                let text = item_text(item);
                if !text.trim().is_empty() {
                    append(&mut turn.prompt, &text);
                }
            }
            Some("agentMessage") => {
                if item["phase"] != "commentary" {
                    let text = item_text(item);
                    if !text.trim().is_empty() {
                        reply_bytes += text.len();
                        append(&mut turn.reply, &text);
                    }
                }
            }
            Some("plan" | "reasoning") => {}
            _ => turn.tools += 1,
        }
    }
    if turn.prompt.is_empty() && turn.reply.is_empty() {
        return None;
    }
    // Admission counts unclipped prompts and caps reply text before separators.
    let bytes = turn.prompt.len() + reply_bytes.min(MAX_FIELD);
    turn.clipped = clip(&mut turn.prompt) | clip(&mut turn.reply);
    // Same admission cost as the Pruv stamp(): only a 24-byte canonical ISO string is kept.
    turn.created = iso(&raw["startedAt"]);
    if !turn.reply.is_empty() {
        turn.completed = iso(&raw["completedAt"]);
    }
    Some((turn, bytes))
}

/// YYYY-MM-DDTHH:MM:SS.sssZ with digit, range and separator checks.
fn iso(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    let b = s.as_bytes();
    if b.len() != 24 || b[23] != b'Z' {
        return None;
    }
    for (i, &c) in b[..23].iter().enumerate() {
        let want = match i { 4 | 7 => b'-', 10 => b'T', 13 | 16 => b':', 19 => b'.', _ => 0 };
        if want != 0 { if c != want { return None; } } else if !c.is_ascii_digit() { return None; }
    }
    let num = |a: usize, n: usize| b[a..a + n].iter().fold(0u32, |v, &c| v * 10 + u32::from(c - b'0'));
    let (month, day, hour, minute, second) = (num(5, 2), num(8, 2), num(11, 2), num(14, 2), num(17, 2));
    let year = num(0, 4);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month { 2 if leap => 29, 2 => 28, 4 | 6 | 9 | 11 => 30, _ => 31 };
    if !(1..=12).contains(&month) || !(1..=days).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(s.to_owned())
}

/// folderKey (non-win32): strip trailing '/', but keep an all-slash string unchanged.
fn folder_key(s: &str) -> &str {
    let t = s.trim_end_matches('/');
    if t.is_empty() { s } else { t }
}

fn read(rpc: &mut Rpc, request: &Value, out: &mut impl Write) -> Result<()> {
    rpc.call(1, "initialize", json!({
        "clientInfo": {"name": "lair", "title": "Lair", "version": "0.0.1"},
        "capabilities": {"experimentalApi": true, "requestAttestation": false},
    }))?;
    rpc.send(json!({"method": "initialized", "params": {}}))?;

    let thread = request["thread"].as_str().ok_or("missing thread")?;
    let meta = rpc.call(2, "thread/read", json!({"threadId": thread, "includeTurns": false}))?;
    let meta = &meta["thread"];
    if meta["cwd"].as_str().map(folder_key) != request["folder"].as_str().map(folder_key) {
        writeln!(out, "{{\"error\":\"not-found\"}}")?;
        return Ok(());
    }

    let mut turns = Vec::new();
    let mut bytes = 0;
    let mut cursor = String::new();
    let mut omitted = false;
    'pages: for page in 0..MAX_PAGES {
        let mut params = json!({"threadId": thread, "limit": 20, "sortDirection": "desc", "itemsView": "summary"});
        if !cursor.is_empty() {
            params["cursor"] = json!(cursor);
        }
        let result = rpc.call(page + 3, "thread/turns/list", params)?;
        let data = result["data"].as_array().ok_or("missing page data")?;
        for raw in data {
            if let Some((turn, size)) = read_turn(raw) {
                if turns.len() == MAX_TURNS || bytes + size > MAX_BYTES {
                    omitted = true;
                    break 'pages;
                }
                bytes += size;
                turns.push(turn);
            }
        }
        let next = result["nextCursor"].as_str().unwrap_or_default();
        if next.is_empty() || data.is_empty() {
            break;
        }
        cursor.clear();
        cursor.push_str(next);
        omitted = page + 1 == MAX_PAGES;
    }

    // Separators and clipping can change the final number of bytes kept.
    let mut kept_bytes = 0;
    let kept = turns.iter().take_while(|turn| {
        kept_bytes += turn.prompt.len() + turn.reply.len();
        kept_bytes <= MAX_BYTES
    }).count();
    omitted |= kept < turns.len();
    turns.truncate(kept);
    write!(out, "{{\"turns\":[")?;
    for (index, turn) in turns.iter().rev().enumerate() {
        if index != 0 {
            write!(out, ",")?;
        }
        write!(out, "{{\"prompt\":{},\"reply\":{},\"tools\":{}",
            serde_json::to_string(&turn.prompt)?, serde_json::to_string(&turn.reply)?, turn.tools)?;
        write_text(out, "createdAt", turn.created.as_deref())?;
        write_text(out, "completedAt", turn.completed.as_deref())?;
        if turn.clipped {
            write!(out, ",\"clipped\":true")?;
        }
        write!(out, "}}")?;
    }
    write!(out, "],\"omittedBefore\":{omitted}")?;
    let model = meta["model"].as_str().or_else(|| meta["model"]["id"].as_str());
    write_text(out, "model", model.map(str::trim))?;
    write_text(out, "title", meta["name"].as_str().map(str::trim))?;
    writeln!(out, "}}")?;
    Ok(())
}

fn write_text(out: &mut impl Write, key: &str, value: Option<&str>) -> Result<()> {
    if let Some(text) = value.filter(|text| !text.is_empty()) {
        write!(out, ",\"{key}\":{}", serde_json::to_string(text)?)?;
    }
    Ok(())
}

fn run(request: &Value, out: &mut impl Write) -> Result<()> {
    if request["mode"] != "read" {
        return Err("R09 supports read requests only".into());
    }
    let binary = request["binary"].as_str().ok_or("missing binary")?;
    let args = request["args"].as_array().into_iter().flatten()
        .map(|arg| arg.as_str().ok_or("invalid argument")).collect::<std::result::Result<Vec<_>, _>>()?;
    let mut command = Command::new(binary);
    command.args(["app-server", "--stdio"]).args(args)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(cwd) = request["cwd"].as_str().filter(|cwd| !cwd.is_empty()) {
        command.current_dir(cwd);
    }
    let mut child = command.spawn()?;
    let mut rpc = Rpc {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        line: String::new(),
    };
    let result = read(&mut rpc, request, out);
    drop(rpc);
    let _ = child.kill();
    child.wait()?;
    result
}

fn main() -> Result<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        run(&serde_json::from_str(&line?)?, &mut out)?;
        out.flush()?;
    }
    Ok(())
}

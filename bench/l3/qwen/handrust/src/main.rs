#![forbid(unsafe_code)]

use std::env;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Instant;

fn read_window(path: &std::ffi::OsStr, limit: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = vec![0; (size - start) as usize];
    file.read_exact(&mut buffer)?;
    Ok((buffer, start > 0))
}

fn quote(out: &mut String, text: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => write!(out, "\\u{:04x}", ch as u32).unwrap(),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn optional(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        out.push(',');
        quote(out, key);
        out.push(':');
        quote(out, value);
    }
}

fn record(window: &qwen_hand::Window) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("{\"turns\":[");
    for (i, turn) in window.turns.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"prompt\":");
        quote(&mut out, &turn.prompt);
        out.push_str(",\"reply\":");
        quote(&mut out, &turn.reply);
        optional(&mut out, "createdAt", turn.created_at.as_deref());
        optional(&mut out, "completedAt", turn.completed_at.as_deref());
        write!(out, ",\"tools\":{}", turn.tools).unwrap();
        if turn.clipped {
            out.push_str(",\"clipped\":true");
        }
        out.push('}');
    }
    write!(out, "],\"omittedBefore\":{}", window.omitted_before).unwrap();
    optional(&mut out, "title", window.title.as_deref());
    optional(&mut out, "model", window.model.as_deref());
    out.push('}');
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args_os().nth(1).ok_or("usage: qwen-hand FILE")?;
    let (buffer, cut_before) = read_window(&path, qwen_hand::WINDOW_BYTES as u64)?;
    let start = Instant::now();
    let window = qwen_hand::window_import(&buffer, cut_before)?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{ms} {}", record(&window));
    Ok(())
}

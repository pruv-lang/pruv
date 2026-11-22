//! Renders the Pruv result packet as the JSON the pinned TypeScript produces (key order as TS builds it):
//! list -> array of ImportListItem, read -> ExternalRecord, failure -> {"error":"<code name>"}.
//! Packet layout: docs/CODEX_IMPORT_SPEC.md "Boundary representation".

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// Text of ref (start,len) at p[at], p[at+1] inside the text bytes; None when absent.
fn text_ref<'a>(p: &[i64], text: &'a [u8], at: usize) -> Option<&'a str> {
    let s = p[at];
    let n = p[at + 1];
    if s < 0 {
        assert!(s == -1 && n == 0, "bad absent ref");
        return None;
    }
    assert!(n >= 0 && (s as usize) + (n as usize) <= text.len(), "ref out of range");
    Some(std::str::from_utf8(&text[s as usize..(s + n) as usize]).expect("packet text is UTF-8"))
}

pub fn error_name(code: i64) -> &'static str {
    match code {
        1 => "input-too-large",
        2 => "internal-invariant",
        3 => "not-found",
        4 => "codex-unsupported",
        5 => "timeout",
        6 => "read-failed",
        7 => "engine-missing",
        _ => "unknown",
    }
}

pub fn render(p: &[i64]) -> String {
    assert!(p.len() >= 3);
    if p[0] != 0 {
        assert!(p.len() as i64 == 3 + p[2]);
        return format!("{{\"error\":{}}}", q(error_name(p[0])));
    }
    assert!(p.len() >= 20 && p[1] == 16, "bad success packet");
    let rw = p[2] as usize;
    let bl = p[3] as usize;
    assert!(p.len() == 20 + rw + bl, "bad packet length");
    let meta = &p[4..20];
    let rows = &p[20..20 + rw];
    let text: Vec<u8> = p[20 + rw..].iter().map(|&x| x as u8).collect();
    assert!(meta[0] == 1);
    match meta[1] {
        0 => render_list(meta, rows, &text),
        1 => render_read(meta, rows, &text),
        _ => panic!("bad mode"),
    }
}

/// Render the read wire straight into `out` (DB44): the same bytes as `render_wire`, without holding the rendered
/// result. The turn texts are escaped by serde_json directly into the writer.
pub fn render_wire_to<W: std::io::Write>(wire: &[u8], out: &mut W) -> std::io::Result<()> {
    fn word(bytes: &[u8]) -> i64 {
        let encoded = i64::from_le_bytes(bytes.try_into().unwrap());
        assert!((0..(1_i64 << 56)).contains(&encoded));
        encoded - 1
    }

    assert!(wire.len() >= 24);
    let code = word(&wire[0..8]);
    if code != 0 {
        let detail_len = usize::try_from(word(&wire[16..24])).unwrap();
        assert_eq!(wire.len() - 24, detail_len);
        return write!(out, "{{\"error\":{}}}", q(error_name(code)));
    }
    assert!(wire.len() >= 160 && word(&wire[8..16]) == 16, "bad read header");
    let rw = usize::try_from(word(&wire[16..24])).unwrap();
    let bl = usize::try_from(word(&wire[24..32])).unwrap();
    assert!(rw <= 2060);
    let header = 8 * (20 + rw);
    assert!(wire.len() >= header && wire.len() - header == bl, "bad wire length");
    let words: Vec<i64> = wire[32..header].chunks_exact(8).map(word).collect();
    let (meta, rows) = words.split_at(16);
    assert!(meta[0] == 1 && meta[1] == 1, "bad read metadata");
    render_read_to(meta, rows, &wire[header..], out)
}

/// Render the read wire. Decode only its words and borrow the raw UTF-8 payload.
pub fn render_wire(wire: &[u8]) -> String {
    let mut out = Vec::new();
    render_wire_to(wire, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn render_read_to<W: std::io::Write>(meta: &[i64], rows: &[i64], text: &[u8], out: &mut W) -> std::io::Result<()> {
    fn qw<W: std::io::Write>(out: &mut W, s: &str) -> std::io::Result<()> {
        serde_json::to_writer(&mut *out, s).map_err(std::io::Error::from)
    }
    let kept = meta[3] as usize;
    let plan_items = if meta[11] < 0 { 0 } else { meta[11] as usize };
    assert!(meta[10] == 10 && rows.len() == 10 * kept + 2 * plan_items);
    out.write_all(b"{\"turns\":[")?;
    for j in 0..kept {
        let b = 10 * j;
        if j != 0 {
            out.write_all(b",")?;
        }
        out.write_all(b"{\"prompt\":")?;
        qw(out, text_ref(rows, text, b).expect("prompt"))?;
        out.write_all(b",\"reply\":")?;
        qw(out, text_ref(rows, text, b + 2).expect("reply"))?;
        write!(out, ",\"tools\":{}", rows[b + 8])?;
        if let Some(c) = text_ref(rows, text, b + 4) {
            out.write_all(b",\"createdAt\":")?;
            qw(out, c)?;
        }
        if let Some(c) = text_ref(rows, text, b + 6) {
            out.write_all(b",\"completedAt\":")?;
            qw(out, c)?;
        }
        if rows[b + 9] == 1 {
            out.write_all(b",\"clipped\":true")?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"],\"omittedBefore\":")?;
    out.write_all(if meta[2] == 1 { b"true" } else { b"false" })?;
    if let Some(m) = text_ref(meta, text, 6) {
        out.write_all(b",\"model\":")?;
        qw(out, m)?;
    }
    if let Some(t) = text_ref(meta, text, 4) {
        out.write_all(b",\"title\":")?;
        qw(out, t)?;
    }
    if meta[11] >= 0 {
        out.write_all(b",\"lastPlan\":{")?;
        if let Some(at) = text_ref(meta, text, 8) {
            out.write_all(b"\"at\":")?;
            qw(out, at)?;
            out.write_all(b",")?;
        }
        out.write_all(b"\"items\":[")?;
        let base = 10 * kept;
        for i in 0..plan_items {
            if i != 0 {
                out.write_all(b",")?;
            }
            out.write_all(b"{\"text\":")?;
            qw(out, text_ref(rows, text, base + 2 * i).expect("plan item"))?;
            out.write_all(b"}")?;
        }
        out.write_all(b"]")?;
        if meta[12] > 0 {
            write!(out, ",\"more\":{}", meta[12])?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"}")
}

fn render_list(meta: &[i64], rows: &[i64], text: &[u8]) -> String {
    let count = meta[3] as usize;
    assert!(meta[10] == 9 && rows.len() == 9 * count);
    let mut items = Vec::new();
    for j in 0..count {
        let b = 9 * j;
        let mut s = String::from("{");
        s.push_str(&format!("\"thread\":{}", q(text_ref(rows, text, b).expect("thread"))));
        s.push_str(&format!(",\"title\":{}", q(text_ref(rows, text, b + 2).expect("title"))));
        if let Some(u) = text_ref(rows, text, b + 4) {
            s.push_str(&format!(",\"updatedAt\":{}", q(u)));
        }
        if let Some(pa) = text_ref(rows, text, b + 6) {
            s.push_str(&format!(",\"path\":{}", q(pa)));
        }
        match rows[b + 8] {
            -1 => {}
            0 => s.push_str(",\"origin\":\"other\""),
            1 => s.push_str(",\"origin\":\"lair\""),
            _ => panic!("bad origin"),
        }
        s.push('}');
        items.push(s);
    }
    format!("[{}]", items.join(","))
}

fn render_read(meta: &[i64], rows: &[i64], text: &[u8]) -> String {
    let mut out = Vec::with_capacity(text.len() + text.len() / 16 + 160 * meta[3] as usize + 1024);
    render_read_to(meta, rows, text, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

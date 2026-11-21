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

/// Render the read wire. Decode only its words and borrow the raw UTF-8 payload.
pub fn render_wire(wire: &[u8]) -> String {
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
        return format!("{{\"error\":{}}}", q(error_name(code)));
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
    render_read(meta, rows, &wire[header..])
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
    let kept = meta[3] as usize;
    let plan_items = if meta[11] < 0 { 0 } else { meta[11] as usize };
    assert!(meta[10] == 10 && rows.len() == 10 * kept + 2 * plan_items);
    let mut turns = Vec::new();
    for j in 0..kept {
        let b = 10 * j;
        let mut s = String::from("{");
        s.push_str(&format!("\"prompt\":{}", q(text_ref(rows, text, b).expect("prompt"))));
        s.push_str(&format!(",\"reply\":{}", q(text_ref(rows, text, b + 2).expect("reply"))));
        s.push_str(&format!(",\"tools\":{}", rows[b + 8]));
        if let Some(c) = text_ref(rows, text, b + 4) {
            s.push_str(&format!(",\"createdAt\":{}", q(c)));
        }
        if let Some(c) = text_ref(rows, text, b + 6) {
            s.push_str(&format!(",\"completedAt\":{}", q(c)));
        }
        if rows[b + 9] == 1 {
            s.push_str(",\"clipped\":true");
        }
        s.push('}');
        turns.push(s);
    }
    let mut out = format!("{{\"turns\":[{}],\"omittedBefore\":{}", turns.join(","), if meta[2] == 1 { "true" } else { "false" });
    if let Some(m) = text_ref(meta, text, 6) {
        out.push_str(&format!(",\"model\":{}", q(m)));
    }
    if let Some(t) = text_ref(meta, text, 4) {
        out.push_str(&format!(",\"title\":{}", q(t)));
    }
    if meta[11] >= 0 {
        let mut plan = String::from("{");
        let mut first = true;
        if let Some(at) = text_ref(meta, text, 8) {
            plan.push_str(&format!("\"at\":{}", q(at)));
            first = false;
        }
        if !first {
            plan.push(',');
        }
        let base = 10 * kept;
        let mut items = Vec::new();
        for i in 0..plan_items {
            let t = text_ref(rows, text, base + 2 * i).expect("plan item");
            items.push(format!("{{\"text\":{}}}", q(t)));
        }
        plan.push_str(&format!("\"items\":[{}]", items.join(",")));
        if meta[12] > 0 {
            plan.push_str(&format!(",\"more\":{}", meta[12]));
        }
        plan.push('}');
        out.push_str(&format!(",\"lastPlan\":{}", plan));
    }
    out.push('}');
    out
}

//! Archived V-word framing and projection for the typed T11 entries.
use crate::generated::{self, Sess, St};
pub use generated::{m11_perf_card, m11_perf_chain};

pub fn new_state() -> St {
    St { sessions: Default::default(), next_sessions: 0, incoming: Default::default() }
}

const ESCAPE: char = '\u{e000}';
const SURROGATE_START: u32 = 0xd800;
const ESCAPED_SURROGATE_START: u32 = 0xe100;

fn key_text(units: impl Iterator<Item = u16>) -> String {
    let mut text = String::new();
    for scalar in char::decode_utf16(units) {
        match scalar {
            Ok(c) => {
                if c == ESCAPE { text.push(ESCAPE); }
                text.push(c);
            }
            Err(lone) => {
                text.push(ESCAPE);
                let code = u32::from(lone.unpaired_surrogate()) - SURROGATE_START;
                text.push(char::from_u32(ESCAPED_SURROGATE_START + code).unwrap());
            }
        }
    }
    text
}

fn write_text(out: &mut Vec<i64>, text: &str) {
    let start = out.len();
    out.push(0);
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == ESCAPE {
            let escaped = chars.next().expect("admitted escape");
            out.push(if escaped == ESCAPE { i64::from(ESCAPE as u32) }
                     else { i64::from(escaped as u32 - ESCAPED_SURROGATE_START + SURROGATE_START) });
        } else {
            out.extend(c.encode_utf16(&mut [0; 2]).iter().copied().map(i64::from));
        }
    }
    out[start] = (out.len() - start - 1) as i64;
}

fn decode_key(words: &[i64]) -> Result<Option<String>, i64> {
    match generated::admit_key(words) {
        0 => {
            let units = (0..words[0] as usize).map(|i| (words[1 + i / 4] >> (48 - 16 * (i % 4))) as u16);
            Ok(Some(key_text(units)))
        }
        1 => Ok(None),
        status => Err(status),
    }
}

fn take<'a>(words: &mut &'a [i64], n: usize) -> Option<&'a [i64]> {
    let (head, tail) = words.split_at_checked(n)?;
    *words = tail;
    Some(head)
}

fn word(words: &mut &[i64]) -> Option<i64> {
    Some(take(words, 1)?[0])
}

fn text(words: &mut &[i64]) -> Option<String> {
    let n = usize::try_from(word(words)?).ok()?;
    Some(key_text(take(words, n)?.iter().map(|&unit| unit as u16)))
}

fn decode_node(b: &mut Sess, words: &mut &[i64], name: String) -> Option<()> {
    let tag = word(words)?;
    let mut number = 0;
    let mut value = String::new();
    match tag {
        1..=4 => {}
        5 | 7 => number = word(words)?,
        6 => value = text(words)?,
        8 | 9 => number = word(words)?,
        _ => return None,
    }
    let index = b.tags.len();
    b.tags.push(tag);
    b.numbers.push(number);
    b.names.push(name);
    b.texts.push(value);
    b.ends.push(0);
    if tag == 8 || tag == 9 {
        for _ in 0..number {
            let child_name = if tag == 9 { text(words)? } else { String::new() };
            decode_node(b, words, child_name)?;
        }
    }
    b.ends[index] = b.tags.len() as i64;
    Some(())
}

fn decode_budget(mut words: &[i64]) -> Sess {
    let mut b = Sess {
        ordinal: -1, tags: vec![], ends: vec![], numbers: vec![], names: vec![], texts: vec![],
        id_index: -1, closed: false, halted: false, card_index: -1, has_card: false,
        updated: false, input: vec![], by: vec![],
        fields: generated::pruv_m_t_2_8_bud_card_4_Keys { p0: -1, p1: -1, p2: -1, p3: -1, p4: -1, p5: -1 },
        now: 0,
    };
    if !words.is_empty() {
        decode_node(&mut b, &mut words, String::new()).expect("admitted V budget");
    }
    b
}

fn ref_id(words: &[i64]) -> Result<Option<String>, i64> {
    match generated::admit_ref(words) {
        -2 => Err(2),
        -1 => Ok(None),
        at => Ok(Some(text(&mut &words[at as usize + 1..]).expect("admitted budgetId"))),
    }
}

fn encode_node(b: &Sess, i: usize, out: &mut Vec<i64>) {
    let tag = b.tags[i];
    let num = b.numbers[i];
    out.push(tag);
    match tag {
        5 | 7 => out.push(num),
        6 => write_text(out, &b.texts[i]),
        8 | 9 => {
            let root = i == 0 && b.updated;
            out.push(num + i64::from(root && b.card_index < 0));
            let mut child = i + 1;
            while child < b.ends[i] as usize {
                if tag == 9 { write_text(out, &b.names[child]); }
                if root && child as i64 == b.card_index {
                    append_card(b, out);
                } else {
                    encode_node(b, child, out);
                }
                child = b.ends[child] as usize;
            }
            if root && b.card_index < 0 {
                write_text(out, "card");
                append_card(b, out);
            }
        }
        _ => {}
    }
}

fn append_card(b: &Sess, out: &mut Vec<i64>) {
    let input: Vec<u8> = b.input.iter().map(|&x| x as u8).collect();
    let by: Vec<u8> = b.by.iter().map(|&x| x as u8).collect();
    out.extend(generated::wire_card(&input, b.fields.clone(), &by, b.now));
}

fn write_budget(b: &Sess, out: &mut Vec<i64>) {
    if !b.tags.is_empty() { encode_node(b, 0, out); }
}

pub fn m11_load(st: &mut St, key: &[i64], blob: &[i64], out: &mut Vec<i64>) -> i64 {
    let Ok(Some(key)) = decode_key(key) else { return 2 };
    if !generated::admit_wire_budget(blob) { return 2; }
    let b = decode_budget(blob);
    st.incoming.insert(key.clone(), b);
    let fresh = generated::m11_load(st, &key);
    if fresh < 0 { return 2; }
    out.clear();
    out.extend([1, fresh]);
    0
}

// Exercise the generated constructor with malformed typed columns.
pub fn malformed_load(st: &mut St, key: &[i64], blob: &[i64]) -> i64 {
    let key = decode_key(key).unwrap().unwrap();
    let mut budget = decode_budget(&[8, 1, 1]);
    budget.tags = blob[..2].to_vec();
    budget.ends = blob[2..4].to_vec();
    budget.numbers = blob[4..].to_vec();
    st.incoming.insert(key.clone(), budget);
    if generated::m11_load(st, &key) < 0 { 2 } else { 0 }
}

fn finish(out: &mut [i64]) -> i64 {
    out[0] = (out.len() - 1) as i64;
    0
}

fn set_with_key(st: &mut St, id: Option<&str>, sid: Option<&str>, card: &[u8], lang: &[u8],
                by: &[u8], now: i64, out: &mut Vec<i64>, native: bool) -> i64 {
    let set = if native { generated::m11_set_card_id } else { generated::m11_set_card_key };
    let r = set(st, id.unwrap_or(""), id.is_some(),
        sid.unwrap_or(""), sid.is_some(), card, lang, by, now);
    if r.status != 0 { return r.status; }
    out.clear();
    if r.kind < 0 {
        out.extend([0, 1, 1, 1, 2, 9, 2, 2, 111, 107, 4]);
        write_text(out, "budget");
        write_budget(&st.sessions[sid.unwrap()], out);
    } else {
        out.extend([0, i64::from(r.used_now), 0, 0, 2, 9, 2, 2, 111, 107, 3]);
        write_text(out, "error");
        out.push(6);
        let bytes = generated::wire_error(r.kind, r.lang);
        let error = std::str::from_utf8(&bytes).expect("card error is UTF-8");
        out.push(error.encode_utf16().count() as i64);
        out.extend(error.encode_utf16().map(i64::from));
    }
    finish(out)
}

fn query_with_key(st: &St, op: i64, id: Option<&str>, sid: Option<&str>, out: &mut Vec<i64>, native: bool) -> i64 {
    let query = if native { generated::m11_query_id } else { generated::m11_query_key };
    let r = query(st, op, id.unwrap_or(""), id.is_some(), sid.unwrap_or(""), sid.is_some());
    if r.status != 0 { return r.status; }
    out.clear();
    out.extend([0, 1]);
    if op == 1 && r.found {
        write_budget(&st.sessions[sid.unwrap()], out);
    } else {
        out.push(if op == 1 { 1 } else if r.flag { 4 } else { 3 });
    }
    finish(out)
}

pub fn set_text(st: &mut St, id: Option<&str>, sid: Option<&str>, card: &[u8], lang: &[u8],
                by: &[u8], now: i64, out: &mut Vec<i64>) -> i64 {
    set_with_key(st, id, sid, card, lang, by, now, out, true)
}

pub fn query_text(st: &St, op: i64, id: Option<&str>, sid: Option<&str>, out: &mut Vec<i64>) -> i64 {
    query_with_key(st, op, id, sid, out, true)
}

pub fn m11_set_card(st: &mut St, rf: &[i64], sid: &[i64], card: &[u8], lang: &[u8],
                    by: &[u8], now: i64, out: &mut Vec<i64>) -> i64 {
    let Ok(id) = ref_id(rf) else { return 2 };
    let Ok(key) = decode_key(sid) else { return 2 };
    set_with_key(st, id.as_deref(), key.as_deref(), card, lang, by, now, out, false)
}

pub fn m11_query(st: &St, op: i64, rf: &[i64], sid: &[i64], out: &mut Vec<i64>) -> i64 {
    let Ok(id) = ref_id(rf) else { return 2 };
    let Ok(key) = decode_key(sid) else { return 2 };
    query_with_key(st, op, id.as_deref(), key.as_deref(), out, false)
}

pub fn set_id(st: &mut St, id: &[i64], sid: &[i64], card: &[u8], lang: &[u8],
              by: &[u8], now: i64, out: &mut Vec<i64>) -> i64 {
    let Ok(id) = decode_key(id) else { return 2 };
    let Ok(key) = decode_key(sid) else { return 2 };
    set_with_key(st, id.as_deref(), key.as_deref(), card, lang, by, now, out, false)
}

pub fn query_id(st: &St, op: i64, id: &[i64], sid: &[i64], out: &mut Vec<i64>) -> i64 {
    let Ok(id) = decode_key(id) else { return 2 };
    let Ok(key) = decode_key(sid) else { return 2 };
    query_with_key(st, op, id.as_deref(), key.as_deref(), out, false)
}

pub fn m11_dump(st: &St, out: &mut Vec<i64>) -> i64 {
    let mut dump = generated::m11_dump(st);
    dump.keys.sort_unstable_by_key(|key| st.sessions[key].ordinal);
    out.clear();
    out.extend([0, dump.keys.len() as i64]);
    for key in dump.keys {
        write_text(out, &key);
        let p = out.len();
        out.push(0);
        write_budget(&st.sessions[&key], out);
        out[p] = (out.len() - p - 1) as i64;
    }
    finish(out)
}

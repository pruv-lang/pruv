// The Q protocol keeps the old unit layout at egress and accepts it for raw-state fixtures.
use crate::sol;

pub struct Put {
    pub words: Vec<i64>,
    pub text: Vec<u8>,
}

fn key_end(words: &[i64], at: usize) -> Option<usize> {
    let n = usize::try_from(*words.get(at)?).ok()?;
    if n > 1 << 30 {
        return None;
    }
    let end = at + 1 + n.div_ceil(4);
    let packed = words.get(at + 1..end)?;
    let lanes = n % 4;
    if lanes != 0 && (packed[packed.len() - 1] as u64) << (16 * lanes) != 0 {
        return None;
    }
    Some(end)
}

fn strings(record: &[i64]) -> Option<[&[i64]; 6]> {
    if record.len() < 30 || !(0..=2).contains(&record[0]) {
        return None;
    }
    let mut fields: [&[i64]; 6] = [&[]; 6];
    let mut at = 24;
    for (i, field) in fields.iter_mut().enumerate() {
        let end = if i == 0 || i == 4 {
            key_end(record, at)?
        } else {
            let n = usize::try_from(*record.get(at)?).ok()?;
            (at + 1).checked_add(n)?
        };
        *field = record.get(at..end)?;
        at = end;
    }
    (at == record.len()).then_some(fields)
}

fn halt_parts(rest: &[i64]) -> Option<(&[i64], &[i64], &[i64])> {
    if rest.len() < 6 || rest[0] != -2 || !(0..=4).contains(&rest[1])
        || !matches!(rest[2], 1 | 2)
    {
        return None;
    }
    let n = usize::try_from(rest[4]).ok()?;
    let ids = 5usize.checked_add(n)?;
    let count = usize::try_from(*rest.get(ids)?).ok()?;
    let mut end = ids + 1;
    for _ in 0..count {
        end = key_end(rest, end)?;
    }
    Some((&rest[5..ids], &rest[ids..end], &rest[end..]))
}

fn append_text(record: &mut Vec<i64>, text: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.is_empty() {
        record.push(0);
    } else {
        record.extend([2, bytes.len() as i64, text.len() as i64]);
        text.extend(bytes);
    }
}

// Encode supplementary characters and preserve isolated surrogates from raw-unit Q fixtures.
fn encode_units(units: &[i64]) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let units = units.iter().map(|&u| u16::try_from(u).ok()).collect::<Option<Vec<_>>>()?;
    for code in char::decode_utf16(units) {
        match code {
            Ok(c) => bytes.extend(c.encode_utf8(&mut [0; 4]).as_bytes()),
            Err(e) => {
                let u = e.unpaired_surrogate();
                bytes.extend([0xe0 | (u >> 12) as u8, 0x80 | ((u >> 6) & 63) as u8, 0x80 | (u & 63) as u8]);
            }
        }
    }
    Some(bytes)
}

fn legacy_record(record: &[i64]) -> Option<Put> {
    let fields = strings(record)?;
    let mut words = record[..24].to_vec();
    let mut text = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if i == 0 || i == 4 {
            words.extend(*field);
        } else if i == 5 {
            let rest = &field[1..];
            if let Some((at, ids, tail)) = halt_parts(rest) {
                let start = words.len();
                words.push(0);
                words.extend(&rest[..4]);
                append_text(&mut words, &mut text, &encode_units(at)?);
                words.extend(ids);
                if !tail.is_empty() {
                    let handle = words.len();
                    append_text(&mut words, &mut text, &encode_units(tail)?);
                    words.remove(handle); // The halted tail has no length word.
                }
                words[start] = (words.len() - start - 1) as i64;
            } else {
                append_text(&mut words, &mut text, &encode_units(rest)?);
            }
        } else {
            append_text(&mut words, &mut text, &encode_units(&field[1..])?);
        }
    }
    Some(Put { words, text })
}

pub fn parse_put(args: &[&str]) -> Put {
    let split = args.iter().position(|&s| s == "|");
    let mut words: Vec<i64> = args[..split.unwrap_or(args.len())].iter().map(|s| s.parse().unwrap()).collect();
    if let Some(split) = split {
        let text = args[split + 1..].iter().map(|s| s.parse().unwrap()).collect();
        return Put { words, text };
    }
    if let Some(at) = key_end(&words, 0) {
        if at < words.len() && words[at] == (words.len() - at - 1) as i64 {
            if let Some(record) = legacy_record(&words[at + 1..]) {
                words.truncate(at);
                words.push(record.words.len() as i64);
                words.extend(record.words);
                return Put { words, text: record.text };
            }
        }
    }
    Put { words, text: Vec::new() }
}

fn decode_handle(tx: &[u8], handle: &[i64], units: &mut Vec<i64>) {
    if handle.is_empty() {
        return;
    }
    let mut bytes = vec![0; usize::try_from(handle[0]).unwrap()];
    assert_eq!(sol::t_text(tx, handle[1], handle[0], &mut bytes), 0);
    let mut remaining = bytes.as_slice();
    loop {
        match std::str::from_utf8(remaining) {
            Ok(s) => {
                units.extend(s.encode_utf16().map(i64::from));
                return;
            }
            Err(e) => {
                let (prefix, suffix) = remaining.split_at(e.valid_up_to());
                units.extend(std::str::from_utf8(prefix).unwrap().encode_utf16().map(i64::from));
                assert!(matches!(suffix, [0xed, 0xa0..=0xbf, 0x80..=0xbf, ..]), "invalid WTF-8");
                units.push(0xd000 | (((suffix[1] & 63) as i64) << 6) | (suffix[2] & 63) as i64);
                remaining = &suffix[3..];
            }
        }
    }
}

pub fn unit_record(tx: &[u8], record: &[i64]) -> Vec<i64> {
    let Some(fields) = strings(record) else { return record.to_vec(); };
    let rest = &fields[5][1..];
    let halt = if rest.len() > 2 { halt_parts(rest) } else { None };
    if fields[1..4].iter().any(|f| !matches!(f.len(), 1 | 3))
        || (rest.len() > 2 && !matches!(halt, Some((at, _, tail)) if matches!(at.len(), 0 | 2) && matches!(tail.len(), 0 | 2)))
        || rest.len() == 1
    {
        return record.to_vec();
    }

    let mut out = record[..24].to_vec();
    for (i, field) in fields.iter().enumerate() {
        if i == 0 || i == 4 {
            out.extend(*field);
            continue;
        }
        let start = out.len();
        out.push(0);
        if let (5, Some((at, ids, tail))) = (i, halt) {
            out.extend(&rest[..4]);
            let at_len = out.len();
            out.push(0);
            decode_handle(tx, at, &mut out);
            out[at_len] = (out.len() - at_len - 1) as i64;
            out.extend(ids);
            decode_handle(tx, tail, &mut out);
        } else {
            decode_handle(tx, &field[1..], &mut out);
        }
        out[start] = (out.len() - start - 1) as i64;
    }
    out
}

pub fn observation(tx: &[u8], words: &[i64]) -> String {
    let mut result = vec![words[0]];
    let mut at = 1;
    for _ in 0..words[0] {
        let key_end = at + 1 + words[at] as usize;
        result.extend_from_slice(&words[at..key_end]);
        at = key_end + 1 + words[key_end] as usize;
        let units = unit_record(tx, &words[key_end + 1..at]);
        result.push(units.len() as i64);
        result.extend(units);
    }
    result.extend_from_slice(&words[at..]);
    use std::fmt::Write;
    let mut out = String::from("O");
    for word in result {
        write!(&mut out, " {word}").unwrap();
    }
    out
}

pub fn perf_put(mut session: Vec<i64>, id: Vec<i64>, state: i64, rest: &str) -> Put {
    let mut record = vec![0; 24];
    record[0] = state;
    record[3..8].copy_from_slice(&[120, 24, 3, 2, 1]);
    record[20] = 1;
    record.extend(id);
    let mut text = Vec::new();
    append_text(&mut record, &mut text, b"2023-11-14T22:13:20.000Z");
    record.extend([0, 0, 0]); // activeSince and ended.at text, then the follows key.
    append_text(&mut record, &mut text, rest.as_bytes());
    session.push(record.len() as i64);
    session.extend(record);
    Put { words: session, text }
}

//! Wire adapter for the generated typed entries.
use crate::generated::{self, St};

pub use generated::{m11_perf_card, m11_perf_chain};

pub fn new_state() -> St {
    St { sessions: Default::default(), next_sessions: 0 }
}

// Like M8's typed driver, map each UTF-16 unit to a distinct scalar.
// Supplementary scalars represent surrogate units, including unpaired ones.
const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

fn decode_key(words: &[i64]) -> Option<String> {
    let (&count, packed) = words.split_first()?;
    let count = usize::try_from(count).ok()?;
    if count > 1 << 30 || packed.len() != count.div_ceil(4) {
        return None;
    }
    let padding = (4 - count % 4) % 4;
    if padding != 0 && (*packed.last()? as u64 & ((1 << (padding * 16)) - 1)) != 0 {
        return None;
    }
    let mut key = String::with_capacity(count);
    for i in 0..count {
        let unit = ((packed[i / 4] as u64 >> (16 * (3 - i % 4))) & 0xffff) as u32;
        key.push(char::from_u32(unit)
            .unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap()));
    }
    Some(key)
}

pub fn decode_sid(words: &[i64]) -> Option<(String, bool)> {
    if words.is_empty() {
        return Some((String::new(), false));
    }
    decode_key(words).map(|key| (key, true))
}

pub fn m11_load(st: &mut St, key: &[i64], blob: &[i64], out: &mut [i64]) -> i64 {
    let Some(key) = decode_key(key) else { return 2 };
    generated::m11_load(st, &key, blob, out)
}

pub fn m11_set_card(st: &mut St, rf: &[i64], sid: &[i64], card: &[u8], lang: &[u8],
                    by: &[u8], now: i64, out: &mut [i64]) -> i64 {
    let Some((key, present)) = decode_sid(sid) else { return 2 };
    generated::m11_set_card(st, rf, &key, present, card, lang, by, now, out)
}

pub fn m11_query(st: &St, op: i64, rf: &[i64], sid: &[i64], out: &mut [i64]) -> i64 {
    let Some((key, present)) = decode_sid(sid) else { return 2 };
    generated::m11_query(st, op, rf, &key, present, out)
}

pub fn m11_dump(st: &St, out: &mut [i64]) -> i64 {
    let dump = generated::m11_dump(st);
    let mut words = dump.words.as_slice();
    let mut rows = Vec::with_capacity(dump.keys.len());
    for key in &dump.keys {
        let [ordinal, len, rest @ ..] = words else { return 2 };
        let Some(blob) = rest.get(..*len as usize) else { return 2 };
        rows.push((*ordinal, key, blob));
        words = &rest[blob.len()..];
    }
    if !words.is_empty() {
        return 2;
    }
    rows.sort_unstable_by_key(|row| row.0);
    let needed = 2 + rows.iter().map(|(_, key, blob)| 2 + key.chars().count() + blob.len()).sum::<usize>();
    let Some(out) = out.get_mut(..needed) else { return 3 };
    out[0] = (needed - 1) as i64;
    out[1] = rows.len() as i64;
    let mut pos = 2;
    for (_, key, blob) in rows {
        out[pos] = key.chars().count() as i64;
        pos += 1;
        for c in key.chars() {
            let scalar = c as u32;
            out[pos] = i64::from(if scalar < 0x10000 { scalar } else { scalar - SURROGATE_OFFSET });
            pos += 1;
        }
        out[pos] = blob.len() as i64;
        pos += 1;
        out[pos..pos + blob.len()].copy_from_slice(blob);
        pos += blob.len();
    }
    0
}

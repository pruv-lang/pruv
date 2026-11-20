//! Synchronous dependency replies. A checkpoint restores stream positions on Grow.
use std::cell::RefCell;

#[derive(Default)]
struct Facts {
    unit: Vec<i64>,
    uuids: Option<Vec<Vec<i64>>>,
    running: Vec<Vec<i64>>,
    uuid_pos: usize,
    running_pos: usize,
    reads: Vec<Read>,
    observing: bool,
    finds: Vec<(usize, Vec<i64>)>,
    typed_running: Vec<i64>,
}

pub struct Read {
    pub offset: usize,
    pub code: i64,
    pub reference: Vec<i64>,
}

thread_local! {
    static FACTS: RefCell<Facts> = RefCell::new(Facts::default());
}

pub fn reset(unit: Vec<i64>, uuids: Option<Vec<Vec<i64>>>, running: Vec<Vec<i64>>) {
    FACTS.with(|slot| *slot.borrow_mut() = Facts { unit, uuids, running, ..Facts::default() });
}

pub fn observe() {
    FACTS.with(|slot| slot.borrow_mut().observing = true);
}

pub fn observing() -> bool {
    FACTS.with(|slot| slot.borrow().observing)
}

pub fn observe_find(offset: i64, budget: &[i64]) {
    FACTS.with(|slot| slot.borrow_mut().finds.push((offset as usize, budget.to_vec())));
}

pub fn take_finds() -> Vec<(usize, Vec<i64>)> {
    FACTS.with(|slot| std::mem::take(&mut slot.borrow_mut().finds))
}

pub fn checkpoint() -> (usize, usize) {
    FACTS.with(|slot| {
        let f = slot.borrow();
        (f.uuid_pos, f.running_pos)
    })
}

pub fn rewind((uuid_pos, running_pos): (usize, usize)) {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        f.uuid_pos = uuid_pos;
        f.running_pos = running_pos;
        f.reads.clear();
        f.finds.clear();
    });
}

pub fn take_reads() -> Vec<Read> {
    FACTS.with(|slot| std::mem::take(&mut slot.borrow_mut().reads))
}

pub fn unit(offset: i64) -> Vec<i64> {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        f.reads.push(Read { offset: offset as usize, code: 5, reference: vec![] });
        f.unit.clone()
    })
}

pub fn uuid(offset: i64) -> Vec<i64> {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        f.reads.push(Read { offset: offset as usize, code: 6, reference: vec![] });
        let pos = f.uuid_pos;
        f.uuid_pos += 1;
        if let Some(stream) = &f.uuids {
            return stream[pos].clone();
        }
        let id = format!("00000000-0000-4000-8000-{:012x}", pos + 1);
        let mut packed = vec![id.len() as i64];
        packed.extend(id.as_bytes().chunks(4).map(|chunk| {
            chunk.iter().enumerate().fold(0u64, |word, (lane, &byte)| {
                word | ((byte as u64) << (48 - 16 * lane))
            }) as i64
        }));
        packed
    })
}

pub fn running_ids(reference: &[i64], offset: i64) -> Vec<i64> {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        f.reads.push(Read { offset: offset as usize, code: 7, reference: reference.to_vec() });
        let pos = f.running_pos;
        f.running_pos += 1;
        f.running.get(pos).cloned().unwrap_or_else(|| vec![0])
    })
}

pub fn running_ids_words() -> i64 {
    FACTS.with(|slot| {
        let f = slot.borrow();
        f.running.get(f.running_pos).map_or(1, |ids| ids.len() as i64)
    })
}

const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

fn unit_char(unit: u16) -> char {
    let unit = u32::from(unit);
    char::from_u32(unit).unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap())
}

pub fn text_units(text: &str) -> impl Iterator<Item = u16> + '_ {
    text.chars().map(|c| {
        let cp = c as u32;
        if cp < 0x10000 { cp as u16 } else { (cp - SURROGATE_OFFSET) as u16 }
    })
}

pub fn packed_text(words: &[i64]) -> Option<String> {
    let (&count, packed) = words.split_first()?;
    let count = usize::try_from(count).ok()?;
    if count > 1 << 30 || packed.len() != count.div_ceil(4) {
        return None;
    }
    let padding = (4 - count % 4) % 4;
    if padding != 0 && (*packed.last()? as u64 & ((1 << (padding * 16)) - 1)) != 0 {
        return None;
    }
    Some((0..count).map(|i| unit_char((packed[i / 4] >> (16 * (3 - i % 4))) as u16)).collect())
}

pub fn decode_key(words: &[i64]) -> String {
    packed_text(words).unwrap_or_default()
}

pub fn v_units(words: &[i64], at: i64, count: i64) -> String {
    let (at, count) = (at as usize, count as usize);
    words.get(at..at.saturating_add(count)).unwrap_or_default()
        .iter().map(|&unit| unit_char(unit as u16)).collect()
}

// The Pruv JB admission check has already validated the WTF-8 text span.
pub fn j_units(packet: &[u8], at: i64, count: i64) -> String {
    let (at, count) = (at as usize, count as usize);
    let bytes = packet.get(at..at.saturating_add(count)).unwrap_or_default();
    if bytes.is_ascii() {
        return String::from_utf8(bytes.to_vec()).expect("ASCII");
    }
    let mut text = String::with_capacity(count);
    let mut i = 0;
    while i < bytes.len() {
        let lead = bytes[i];
        let (width, mask) = match lead {
            0..=127 => (1, 127),
            192..=223 => (2, 31),
            224..=239 => (3, 15),
            _ => (4, 7),
        };
        let Some(seq) = bytes.get(i..i + width) else { break; };
        let cp = seq[1..].iter().fold(u32::from(lead & mask), |cp, &b| (cp << 6) | u32::from(b & 63));
        if cp < 0x10000 {
            text.push(unit_char(cp as u16));
        } else {
            let cp = cp - 0x10000;
            text.push(unit_char((0xd800 + (cp >> 10)) as u16));
            text.push(unit_char((0xdc00 + (cp & 1023)) as u16));
        }
        i += width;
    }
    text
}

pub fn wire_units(text: &str) -> Vec<i64> {
    let mut words = Vec::with_capacity(text.len() + 1);
    words.push(0);
    words.extend(text_units(text).map(i64::from));
    words[0] = (words.len() - 1) as i64;
    words
}

pub fn encode_key(text: &str) -> Vec<i64> {
    let n = text.chars().count();
    let mut packed = vec![0; 1 + n.div_ceil(4)];
    packed[0] = n as i64;
    for (i, unit) in text_units(text).enumerate() {
        packed[1 + i / 4] |= i64::from(unit) << (48 - 16 * (i % 4));
    }
    packed
}

// The typed effect index is replaced by the M12 wire offset in running_wire.
pub fn running_position(_: i64) {}

pub fn running_count(_: &str, _: &str) -> i64 {
    FACTS.with(|slot| {
        let f = slot.borrow();
        f.running.get(f.running_pos).and_then(|row| row.first()).copied().unwrap_or(0)
    })
}

pub fn running_take(_: &str, _: &str) -> Vec<String> {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        let row = f.running.get(f.running_pos).cloned().unwrap_or_else(|| vec![0]);
        f.running_pos += 1;
        let mut ids = Vec::new();
        let mut rest = row.get(1..).unwrap_or_default();
        while let Some(&count) = rest.first() {
            let words = 1 + (count as usize).div_ceil(4);
            let Some(key) = rest.get(..words).and_then(packed_text) else { break; };
            ids.push(key);
            rest = &rest[words..];
        }
        f.typed_running = row;
        ids
    })
}

pub fn running_wire(reference: &[i64], offset: i64) -> Vec<i64> {
    FACTS.with(|slot| {
        let mut f = slot.borrow_mut();
        f.reads.push(Read { offset: offset as usize, code: 7, reference: reference.to_vec() });
        std::mem::take(&mut f.typed_running)
    })
}

pub fn is_quiet_text(_: &str, _: i64) -> i64 {
    unreachable!("M12 does not expose maybeClose")
}

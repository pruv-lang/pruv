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

pub fn encode_words(words: &[i64]) -> String {
    words.iter().flat_map(|&word| [48, 32, 16, 0].map(move |shift| unit_char((word >> shift) as u16))).collect()
}

pub fn decode_words(text: &str) -> Vec<i64> {
    let mut units = text_units(text);
    std::iter::from_fn(|| {
        let word = units.next()? as i64;
        Some([units.next()?, units.next()?, units.next()?]
            .into_iter().fold(word, |word, unit| (word << 16) | i64::from(unit)))
    }).collect()
}

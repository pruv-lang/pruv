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

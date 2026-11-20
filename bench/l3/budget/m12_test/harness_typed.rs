
use crate::generated::{self, St};

pub use generated::{m12_observe, m12_seed_tool, m12_grow, m12_init};

pub struct World {
    pub st: St,
    out: Vec<i64>,
}

impl World {
    pub fn new() -> Self {
        Self {
            st: St { sessions: Default::default(), ordinals: Default::default(), next_session: 0 },
            out: vec![0; 4096],
        }
    }

    pub fn call(&mut self, f: &dyn Fn(&mut St, &mut [i64]) -> i64) -> Result<&[i64], i64> {
        loop {
            match f(&mut self.st, &mut self.out) {
                0 => return Ok(&self.out[1..=self.out[0] as usize]),
                3 => self.out.resize(self.out.len().checked_mul(2).ok_or(3)?, 0),
                status => return Err(status),
            }
        }
    }
}

fn optional_key(words: &[i64]) -> Option<String> {
    if words.is_empty() { Some(String::new()) } else { m12_facts::packed_text(words) }
}

pub fn m12_load(st: &mut St, key: &[i64], blob: &[i64], out: &mut [i64]) -> i64 {
    let Some(key) = m12_facts::packed_text(key) else { return 2; };
    generated::load(st, &key, blob, out)
}

pub fn m12_query(st: &St, op: i64, rf: &[i64], key: &[i64], out: &mut [i64]) -> i64 {
    let Some(sid) = optional_key(key) else { return 2; };
    generated::query(st, op, rf, &sid, !key.is_empty(), out)
}

pub fn m12_set_card(st: &mut St, rf: &[i64], key: &[i64], card: &[u8],
                    lang: &[u8], by: &[u8], now: i64, out: &mut [i64]) -> i64 {
    let Some(sid) = optional_key(key) else { return 2; };
    generated::set_card(st, rf, &sid, !key.is_empty(), card, lang, by, now, out)
}

pub fn m12_auto(st: &mut St, op: i64, rf: &[i64], key: &[i64], caller: &[i64],
                packet: &[u8], out: &mut [i64]) -> i64 {
    let (Some(sid), Some(caller_sid)) = (optional_key(key), optional_key(caller)) else { return 2; };
    generated::auto(st, op, rf, key, &sid, caller, &caller_sid, packet, out)
}

pub fn m12_legacy_effects(st: &St, kind: i64, op: i64, rf: &[i64], key: &[i64],
                          caller: &[i64], now: i64, reply: &[i64]) -> Vec<i64> {
    let sid = m12_facts::decode_key(key);
    generated::legacy_effects(st, kind, op, rf, key, &sid, caller, now, reply)
}

pub fn m12_session(st: &St, key: &[i64]) -> Vec<i64> {
    optional_key(key).map_or_else(Vec::new, |sid| generated::session(st, &sid))
}

pub fn m12_dump(st: &St, out: &mut [i64]) -> i64 {
    let mut rows: Vec<_> = st.ordinals.iter().collect();
    rows.sort_unstable_by_key(|(_, ordinal)| ordinal.value);
    let mut words = vec![0, rows.len() as i64];
    for (sid, _) in rows {
        words.push(sid.chars().count() as i64);
        words.extend(m12_facts::text_units(sid).map(i64::from));
        let budget = m12_facts::decode_words(&st.sessions[sid].rest);
        words.push(budget.len() as i64);
        words.extend(budget);
    }
    words[0] = (words.len() - 1) as i64;
    let Some(dest) = out.get_mut(..words.len()) else { return 3; };
    dest.copy_from_slice(&words);
    0
}

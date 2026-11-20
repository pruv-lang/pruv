
use crate::generated;
pub use crate::generated::St;

pub use generated::{m12_observe, m12_seed_tool, m12_grow, m12_init, m12_reset_tools};

#[derive(Clone)]
pub struct World {
    pub st: St,
    out: Vec<i64>,
}

impl World {
    pub fn new() -> Self {
        Self {
            st: new_state(),
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

type ClockState = generated::pruv_m_t_2_10_bud_tstate_10_ClockState;

pub fn new_state() -> St {
    St {
        sessions: Default::default(),
        ordinals: Default::default(),
        next_session: 0,
        clock_state: ClockState { clocks: Default::default(), next_clock: 0 },
    }
}

fn lifecycle_reply(packet: Vec<i64>) -> Result<Vec<i64>, i64> {
    let (_, _, status, _, _) = super::effects(&packet).last().ok_or(2)?;
    if status == 0 { Ok(packet) } else { Err(status) }
}

pub fn m12_life(shared: &mut St, instance: &mut St, op: i64, rf: &[i64], sid: &[i64],
                arg: &[i64], now: i64) -> Result<Vec<i64>, i64> {
    let key = optional_key(sid).ok_or(2)?;
    let keys = if matches!(op, 2 | 3) {
        let mut rows: Vec<_> = instance.clock_state.clocks.iter().collect();
        rows.sort_unstable_by_key(|(_, clock)| clock.ordinal);
        rows.into_iter().flat_map(|(id, clock)| {
            m12_facts::encode_key(id).into_iter().chain(m12_facts::encode_key(&clock.owner))
        }).collect()
    } else {
        Vec::new()
    };
    with_sessions(shared, instance, |st| generated::life(st, op, rf, sid, &key, arg, now, &keys))
}

pub fn m12_fire(shared: &mut St, instance: &mut St, sid: &[i64], bid: &[i64],
                budget_first: bool, now: i64) -> Result<Vec<i64>, i64> {
    let key = optional_key(sid).ok_or(2)?;
    with_sessions(shared, instance, |st| generated::fire_timer(st, sid, &key, bid, budget_first, now))
}

// Each instance keeps its own clocks and borrows the shared sessions for a call.
fn with_sessions(shared: &mut St, instance: &mut St,
                 run: impl FnOnce(&mut St) -> Vec<i64>) -> Result<Vec<i64>, i64> {
    std::mem::swap(&mut shared.sessions, &mut instance.sessions);
    let packet = run(instance);
    std::mem::swap(&mut shared.sessions, &mut instance.sessions);
    lifecycle_reply(packet)
}

pub fn observe_clocks(instance: &St, aux: &[i64]) -> Vec<i64> {
    fn key(out: &mut Vec<i64>, text: &str) {
        out.push(text.chars().count() as i64);
        out.extend(m12_facts::text_units(text).map(i64::from));
    }

    let mut rows: Vec<_> = instance.clock_state.clocks.iter().collect();
    rows.sort_unstable_by_key(|(_, clock)| clock.ordinal);
    let mut out = vec![0, rows.len() as i64];
    for (id, clock) in rows {
        key(&mut out, id);
        key(&mut out, &clock.owner);
        out.extend([i64::from(clock.has_since), clock.since, i64::from(clock.armed), clock.active]);
        let mut runs: Vec<_> = clock.runs.iter().collect();
        runs.sort_unstable_by_key(|(_, ordinal)| ordinal.abs());
        out.push(runs.len() as i64);
        for (run, ordinal) in runs {
            key(&mut out, run);
            out.push(i64::from(*ordinal < 0));
        }
    }
    out.extend_from_slice(&aux[2..]);
    out
}

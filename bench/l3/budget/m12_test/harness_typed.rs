
use crate::generated;
pub use crate::generated::St;

pub use generated::{decode_life, LifeKeys};

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
        let budget = generated::session(st, sid);
        words.push((budget.len() - 1) as i64);
        words.extend_from_slice(&budget[1..]);
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
        wire: generated::Wire { nodes: Default::default(), next: 1 },
        clock_state: ClockState { clocks: Default::default(), next_clock: 0 },
        clock_order: Vec::new(),
        clock_ordinals: Vec::new(),
        tools: Default::default(),
    }
}

fn lifecycle_reply(packet: Vec<i64>) -> Result<Vec<i64>, i64> {
    let (_, _, status, _, _) = super::effects(&packet).last().ok_or(2)?;
    if status == 0 { Ok(packet) } else { Err(status) }
}

pub fn m12_life(shared: &mut St, instance: &mut St, op: i64, rf: &[i64], sid: &[i64],
                arg: &[i64], now: i64) -> Result<Vec<i64>, i64> {
    let key = optional_key(sid).ok_or(2)?;
    let keys = decode_life(rf, arg);
    life_decoded(shared, instance, op, rf, sid, &key, arg, now, &keys)
}

pub fn life_decoded(shared: &mut St, instance: &mut St, op: i64, rf: &[i64], sid: &[i64],
                    key: &str, arg: &[i64], now: i64, keys: &LifeKeys) -> Result<Vec<i64>, i64> {
    with_sessions(shared, instance, |st| {
        generated::life(st, op, rf, sid, key, arg, now, &keys.bid, &keys.text[0], &keys.text[1], keys.has_run)
    })
}

pub fn seed_tool(instance: &mut St, turn: &[i64], activity: &[i64]) -> Result<i64, i64> {
    let turn = m12_facts::packed_text(turn).ok_or(2)?;
    let activity = m12_facts::packed_text(activity).ok_or(2)?;
    match generated::seed_tool(instance, &turn, &activity) {
        n if n < 0 => Err(-n),
        fresh => Ok(fresh),
    }
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
    std::mem::swap(&mut shared.wire, &mut instance.wire);
    let packet = run(instance);
    std::mem::swap(&mut shared.sessions, &mut instance.sessions);
    std::mem::swap(&mut shared.wire, &mut instance.wire);
    lifecycle_reply(packet)
}

pub fn observe_clocks(instance: &St) -> Vec<i64> {
    fn key(out: &mut Vec<i64>, text: &str) {
        out.push(text.chars().count() as i64);
        out.extend(m12_facts::text_units(text).map(i64::from));
    }

    let mut out = vec![0, instance.clock_state.clocks.len() as i64];
    for (id, &ordinal) in instance.clock_order.iter().zip(&instance.clock_ordinals) {
        let Some(clock) = instance.clock_state.clocks.get(id) else { continue; };
        if clock.ordinal != ordinal { continue; }
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
    let mut turns: Vec<_> = instance.tools.iter().collect();
    turns.sort_unstable_by_key(|(_, turn)| turn.ordinal);
    out.push(turns.len() as i64);
    for (id, turn) in turns {
        key(&mut out, id);
        let mut activities: Vec<_> = turn.activities.iter().collect();
        activities.sort_unstable_by_key(|(_, ordinal)| **ordinal);
        out.push(activities.len() as i64);
        for (activity, _) in activities { key(&mut out, activity); }
    }
    out
}

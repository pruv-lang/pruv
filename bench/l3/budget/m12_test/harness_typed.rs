
use crate::generated;
pub use crate::generated::St;

pub use generated::{decode_life, LifeKeys};

#[derive(Clone)]
pub struct World {
    pub st: St,
    out: Vec<i64>,
    pub trace_owner: Vec<i64>,
}

impl World {
    pub fn new() -> Self {
        Self {
            st: new_state(),
            out: vec![0; 4096],
            trace_owner: Vec::new(),
        }
    }

    pub fn call(&mut self, f: &dyn Fn(&mut St, &mut Vec<i64>) -> i64) -> Result<&[i64], i64> {
        loop {
            match f(&mut self.st, &mut self.out) {
                0 => return Ok(&self.out[1..=self.out[0] as usize]),
                3 => self.out.resize(self.out.len().checked_mul(2).ok_or(3)?, 0),
                status => return Err(status),
            }
        }
    }

    pub fn auto(&mut self, op: i64, rf: &[i64], sid: &[i64], caller: &[i64], packet: &[u8]) -> Result<&[i64], i64> {
        let status = m12_auto(&mut self.trace_owner, &mut self.st, op, rf, sid, caller, packet, &mut self.out);
        if status != 0 { return Err(status); }
        Ok(&self.out[1..])
    }
}

fn packed_key(words: &[i64], optional: bool) -> Option<String> {
    let mut key = generated::PackedKey { value: String::new() };
    (generated::admit_packed_key(&mut key, words, optional) == 0).then_some(key.value)
}

fn adapter_lane() -> bool {
    static ADAPTER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ADAPTER.get_or_init(|| std::env::var("M12_LANE").as_deref() == Ok("adapter"))
}

fn reply(out: &mut Vec<i64>, words: impl IntoIterator<Item = i64>) {
    out.clear();
    out.push(0);
    out.extend(words);
    out[0] = (out.len() - 1) as i64;
}

pub fn m12_load(st: &mut St, key: &[i64], blob: &[i64], out: &mut Vec<i64>) -> i64 {
    let Some(key) = packed_key(key, false) else { return 2; };
    let loaded = if adapter_lane() {
        generated::load_packet(st, &key, blob)
    } else {
        let input = generated::decode_budget(st, blob);
        generated::load_key(st, &key, input)
    };
    reply(out, [i64::from(loaded.fresh)]);
    loaded.status
}

pub fn m12_query(st: &St, op: i64, rf: &[i64], key: &[i64], out: &mut Vec<i64>) -> i64 {
    let Some(sid) = packed_key(key, true) else { return 2; };
    let result = if adapter_lane() {
        generated::query_packet(st, op, rf, &sid, !key.is_empty())
    } else {
        let reference = generated::decode_ref(rf);
        if !reference.valid { return 2; }
        generated::query_decoded(st, op, &sid, !key.is_empty(), reference.ids.first().map_or("", String::as_str), reference.has_id)
    };
    if result.status != 0 { return result.status; }
    if op == 1 && result.root != 0 {
        reply(out, [1].into_iter().chain(generated::encode_budget(st, result.root)));
    } else {
        reply(out, [1, if op == 1 { 1 } else { 3 + i64::from(result.value) }]);
    }
    0
}

pub fn m12_set_card(st: &mut St, rf: &[i64], key: &[i64], card: &[u8],
                    lang: &[u8], by: &[u8], now: i64, out: &mut Vec<i64>) -> i64 {
    let Some(sid) = packed_key(key, true) else { return 2; };
    let result = if adapter_lane() {
        generated::set_card_packet(st, rf, &sid, !key.is_empty(), card, lang, by, now)
    } else {
        let reference = generated::decode_ref(rf);
        if !reference.valid { return 2; }
        generated::set_card_decoded(st, &sid, !key.is_empty(), reference.ids.first().map_or("", String::as_str), reference.has_id, card, lang, by, now)
    };
    if result.status != 0 { return result.status; }
    let saved = i64::from(result.root != 0);
    reply(out, [i64::from(result.used_now), saved, saved, 2, 9, 2, 2, 111, 107, 3 + saved]);
    if saved == 1 {
        out.extend([6, 98, 117, 100, 103, 101, 116]);
        out.extend(generated::encode_budget(st, result.root));
    } else {
        let error: Vec<_> = std::str::from_utf8(&generated::card_error(result.kind, result.lang)).expect("card error UTF-8")
            .encode_utf16().map(i64::from).collect();
        out.extend([5, 101, 114, 114, 111, 114, 6, error.len() as i64]);
        out.extend(error);
    }
    out[0] = (out.len() - 1) as i64;
    0
}

pub fn m12_auto(trace_owner: &mut Vec<i64>, st: &mut St, op: i64, rf: &[i64], key: &[i64], caller: &[i64],
                packet: &[u8], out: &mut Vec<i64>) -> i64 {
    let (Some(sid), Some(caller_sid)) = (packed_key(key, true), packed_key(caller, true)) else { return 2; };
    let result = if adapter_lane() {
        generated::auto_packet(st, op, rf, &sid, !key.is_empty(), &caller_sid, !caller.is_empty(), packet)
    } else {
        let reference = generated::decode_ref(rf);
        if !reference.valid { return 2; }
        generated::auto_decoded(st, op, &sid, !key.is_empty(), reference.ids.first().map_or("", String::as_str), reference.has_id,
                        reference.truthy, &caller_sid, !caller.is_empty(), packet)
    };
    if result.status != 0 { return result.status; }
    let saved = i64::from(result.saved);
    reply(out, [result.finds, saved, saved, result.value]);
    // Pruv selects the owner. Both packed inputs were validated above.
    let owner = match result.owner {
        generated::AutoOwner::Reference => Some(key),
        generated::AutoOwner::Caller => Some(caller),
        generated::AutoOwner::NoOwner => None,
    };
    if let Some(owner) = owner {
        trace_owner.clear();
        trace_owner.extend_from_slice(owner);
    }
    if result.saved {
        out.extend_from_slice(trace_owner);
        out[0] = (out.len() - 1) as i64;
    }
    0
}

pub fn m12_legacy_effects(kind: i64, op: i64, rf: &[i64], key: &[i64],
                          selected: &[i64], now: i64, reply: &[i64]) -> Vec<i64> {
    generated::legacy_effects(kind, op, rf, key, selected, now, reply)
}

pub fn m12_session(st: &St, key: &[i64]) -> Vec<i64> {
    let Some(sid) = packed_key(key, true) else { return Vec::new(); };
    let view = generated::session_key(st, &sid);
    if view.present {
        [1].into_iter().chain(generated::encode_budget(st, view.root)).collect()
    } else {
        Vec::new()
    }
}

pub fn m12_dump(st: &St, out: &mut Vec<i64>) -> i64 {
    let mut rows: Vec<_> = st.ordinals.iter().collect();
    rows.sort_unstable_by_key(|(_, ordinal)| ordinal.value);
    let mut words = vec![0, rows.len() as i64];
    for (sid, _) in rows {
        words.push(sid.chars().count() as i64);
        words.extend(m12_facts::text_units(sid).map(i64::from));
        let view = generated::session_key(st, sid);
        let budget = generated::encode_budget(st, view.root);
        words.push(budget.len() as i64);
        words.extend(budget);
    }
    words[0] = (words.len() - 1) as i64;
    *out = words;
    0
}

type ClockState = generated::pruv_m_t_2_10_bud_tstate_10_ClockState;

pub fn new_state() -> St {
    St {
        sessions: Default::default(),
        ordinals: Default::default(),
        next_session: 0,
        wire: generated::Wire {
            nodes: Default::default(), next: 1, prepared_budget: 0, prepared_ticket: 0,
        },
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
    let key = packed_key(sid, true).ok_or(2)?;
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
    let turn = packed_key(turn, false).ok_or(2)?;
    let activity = packed_key(activity, false).ok_or(2)?;
    match generated::seed_tool(instance, &turn, &activity) {
        n if n < 0 => Err(-n),
        fresh => Ok(fresh),
    }
}

pub fn m12_fire(shared: &mut St, instance: &mut St, sid: &[i64], bid: &[i64],
                budget_first: bool, now: i64) -> Result<Vec<i64>, i64> {
    let key = packed_key(sid, true).ok_or(2)?;
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


pub fn malformed_typed() -> bool {
    let mut st = new_state();
    let wrong_length = generated::prepare_budget(&mut st, &[6, 0, 0, 0, 3, b'a']);
    if generated::load(&mut st, "s", wrong_length).status != 2 || generated::session(&st, "s").present {
        return false;
    }
    let wrong_ref = generated::decode_ref(&[9, 1, 8, 98, 117, 100, 103, 101, 116, 73, 100, 5, 99]);
    if !wrong_ref.valid || wrong_ref.has_id { return false; }
    let bad_unit = generated::decode_ref(&[9, 1, 8, 98, 117, 100, 103, 101, 116, 73, 100, 6, 1, 65634]);
    if !bad_unit.valid || bad_unit.has_id { return false; }
    let budget = [9, 0, 0, 0, 1, 0, 0, 0, 2, b'i', b'd', 6, 0, 0, 0, 1, b'b'];
    let stale = generated::prepare_budget(&mut st, &budget);
    let active = generated::prepare_budget(&mut st, &budget);
    let reused = active.clone();
    let loaded = generated::load(&mut st, "s", active);
    loaded.status == 0 && loaded.fresh
        && generated::load(&mut st, "old", stale).status == 2
        && generated::load(&mut st, "reuse", reused).status == 2
        && generated::query(&st, 1, "s", true, "stale", true).root == 0
        && generated::query(&st, 1, "s", true, "", false).root == 0
        && generated::query(&st, 1, "s", true, "b", true).root != 0
        && generated::set_card(&mut st, "s", true, "b", true, &[0], &[], &[], 0).status == 2
        && prepared_budget_checks()
        && packed_key_checks()
        && session_key_checks(&budget)
}

fn session_key_checks(budget: &[u8]) -> bool {
    let (mut native, mut packed) = (new_state(), new_state());
    let sid = [2, ((0xd83d_u64 << 48) | (0xde00_u64 << 32)) as i64];
    let rf = [9, 1, 8, 98, 117, 100, 103, 101, 116, 73, 100, 6, 1, 98];
    let packed_budget = [9, 1, 2, 105, 100, 6, 1, 98];
    let (mut expected, mut actual) = (Vec::new(), Vec::new());
    let input = generated::prepare_budget(&mut native, budget);
    if generated::load(&mut native, "😀", input).status != 0
        || m12_load(&mut packed, &sid, &packed_budget, &mut expected) != 0
        || m12_query(&packed, 1, &rf, &sid, &mut expected) != 0
        || expected.get(2..) != Some(packed_budget.as_slice())
        || m12_query(&native, 1, &rf, &sid, &mut actual) != 0
        || actual != expected
    {
        return false;
    }
    let result = generated::query(&packed, 1, "😀", true, "b", true);
    result.status == 0 && result.root != 0
        && generated::encode_budget(&packed, result.root).as_slice() == packed_budget.as_slice()
}

fn prepared_budget_checks() -> bool {
    fn prepare(st: &mut St, id: &str) -> generated::BudgetInput {
        let mut jb = vec![9, 0, 0, 0, 1, 0, 0, 0, 2, b'i', b'd', 6];
        jb.extend_from_slice(&(id.len() as u32).to_be_bytes());
        jb.extend_from_slice(id.as_bytes());
        generated::prepare_budget(st, &jb)
    }

    let (mut a, mut b) = (new_state(), new_state());
    let a_input = prepare(&mut a, "😀");
    let b_input = prepare(&mut b, "other");
    if a_input.root == 0 || a_input.root != b_input.root
        || generated::load(&mut b, "wrong", a_input.clone()).status != 2
        || generated::session(&b, "wrong").present
        || generated::load(&mut b, "s", b_input).status != 0
        || generated::query(&b, 1, "s", true, "other", true).root == 0
        || generated::load(&mut a, "s", a_input).status != 0
    {
        return false;
    }

    let sid = [1, 115 << 48];
    let mut rf = vec![9, 1, 8, 98, 117, 100, 103, 101, 116, 73, 100, 6, 2, 0xd83d, 0xde00];
    let packed_budget = [9, 1, 2, 105, 100, 6, 2, 0xd83d, 0xde00];
    let mut out = Vec::new();
    if generated::query(&a, 1, "s", true, "😀", true).root == 0
        || m12_query(&a, 1, &rf, &sid, &mut out) != 0
        || out.get(2..) != Some(packed_budget.as_slice())
        || m12_load(&mut b, &sid, &packed_budget, &mut out) != 0
        || generated::query(&b, 1, "s", true, "😀", true).root == 0
    {
        return false;
    }

    // A lone surrogate stays distinct from the native scalar used to encode it.
    rf.truncate(13);
    rf[12] = 1;
    rf.push(0xd800);
    let lone = [9, 1, 2, 105, 100, 6, 1, 0xd800];
    m12_load(&mut b, &sid, &lone, &mut out) == 0
        && generated::query(&b, 1, "s", true, "\u{10000}", true).root == 0
        && m12_query(&b, 1, &rf, &sid, &mut out) == 0
        && out.get(2..) == Some(lone.as_slice())
}

fn packed_key_checks() -> bool {
    let mut st = new_state();
    let mut out = Vec::new();
    let sid = [1, 115 << 48];
    let bad_keys: &[&[i64]] = &[&[1], &[0, 0], &[1, (115 << 48) | 1], &[-1], &[1 << 30 | 1]];
    m12_load(&mut st, &sid, &[], &mut out) == 0
        && bad_keys.iter().all(|key| {
            packed_key(key, true).is_none()
                && m12_load(&mut st, key, &[], &mut out) == 2
                && m12_query(&st, 1, &[], key, &mut out) == 2
        })
        && m12_load(&mut st, &[], &[], &mut out) == 2
        && packed_key(&[], true).as_deref() == Some("")
        && m12_load(&mut st, &[0], &[], &mut out) == 0
        && generated::session(&st, "s").present
}

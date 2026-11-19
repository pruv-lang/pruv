//! Native state for the hand-written lifecycle baseline. V words are used at the replay boundary.
use std::collections::HashMap;

use super::{unpack, Store};

type Text = Vec<u16>;
const COUNTS: [&str; 4] = ["delegations", "commands", "reviews", "checks"];
const LIMITS: [&str; 5] = ["timeMinutes", "delegations", "commands", "reviews", "checks"];
const REASONS: [&str; 5] = ["time", "delegations", "commands", "reviews", "checks"];
const CHARGES: [&str; 5] = ["delegations", "commands", "reviews", "checks", "retries"];
const SAFE: i64 = 9_007_199_254_740_991;

fn text(s: &str) -> Text { s.encode_utf16().collect() }
fn is_text(s: &[u16], value: &str) -> bool { s.iter().copied().eq(value.encode_utf16()) }
fn read_units(words: &mut impl Iterator<Item = i64>, n: i64) -> Text {
    (0..n).map(|_| words.next().unwrap() as u16).collect()
}
fn unpack_text(key: &[i64]) -> Text { unpack(key).map(|word| word as u16).collect() }

fn packed(s: &[u16]) -> Vec<i64> {
    let mut out = Vec::with_capacity(1 + s.len().div_ceil(4));
    out.push(s.len() as i64);
    out.extend(s.chunks(4).map(|chunk| {
        chunk.iter().enumerate().fold(0, |word, (i, &unit)| word | (i64::from(unit) << (48 - 16 * i)))
    }));
    out
}

fn write_key(out: &mut Vec<i64>, key: &[u16]) {
    out.push(key.len() as i64);
    out.extend(key.iter().map(|&unit| i64::from(unit)));
}

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Integer(i64),
    Float(u64),
    String(Text),
    Array(Vec<Value>),
    Object(Vec<(Text, Value)>),
}

impl Value {
    pub fn decode(words: &[i64]) -> Self {
        Self::read(&mut words.iter().copied())
    }

    fn read(words: &mut impl Iterator<Item = i64>) -> Self {
        match words.next().expect("V tag") {
            1 => Self::Undefined,
            2 => Self::Null,
            3 => Self::Bool(false),
            4 => Self::Bool(true),
            5 => Self::Integer(words.next().unwrap()),
            6 => {
                let n = words.next().unwrap();
                Self::String(read_units(words, n))
            }
            7 => Self::Float(words.next().unwrap() as u64),
            8 => {
                let n = words.next().unwrap();
                Self::Array((0..n).map(|_| Self::read(words)).collect())
            }
            9 => {
                let n = words.next().unwrap();
                Self::Object((0..n).map(|_| {
                    let len = words.next().unwrap();
                    (read_units(words, len), Self::read(words))
                }).collect())
            }
            tag => panic!("unknown V tag {tag}"),
        }
    }

    fn write(&self, out: &mut Vec<i64>) {
        match self {
            Self::Undefined => out.push(1),
            Self::Null => out.push(2),
            Self::Bool(value) => out.push(3 + i64::from(*value)),
            Self::Integer(value) => out.extend([5, *value]),
            Self::Float(bits) => out.extend([7, *bits as i64]),
            Self::String(s) => {
                out.push(6);
                write_key(out, s);
            }
            Self::Array(values) => {
                out.extend([8, values.len() as i64]);
                for value in values { value.write(out); }
            }
            Self::Object(fields) => {
                out.extend([9, fields.len() as i64]);
                for (key, value) in fields {
                    write_key(out, key);
                    value.write(out);
                }
            }
        }
    }

    pub fn encode(&self) -> Vec<i64> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Self {
        Self::Object(fields.into_iter().map(|(key, value)| (text(key), value)).collect())
    }

    fn get(&self, key: &str) -> &Self {
        if let Self::Object(fields) = self {
            if let Some((_, value)) = fields.iter().rfind(|(name, _)| is_text(name, key)) {
                return value;
            }
        }
        &Self::Undefined
    }

    fn put(&mut self, key: &'static str, value: Self) {
        if !matches!(self, Self::Object(_)) {
            *self = Self::Object(Vec::new());
        }
        let Self::Object(fields) = self else { unreachable!() };
        if let Some((_, field)) = fields.iter_mut().rfind(|(name, _)| is_text(name, key)) {
            *field = value;
        } else {
            fields.push((text(key), value));
        }
    }

    fn edit_object(&mut self, key: &'static str, edit: impl FnOnce(&mut Self)) {
        if let Self::Object(fields) = self {
            if let Some((_, value)) = fields.iter_mut().rfind(|(name, _)| is_text(name, key)) {
                edit(value);
                return;
            }
        }
        let mut object = Self::Object(Vec::new());
        edit(&mut object);
        self.put(key, object);
    }

    fn remove(&mut self, key: &str) {
        if let Self::Object(fields) = self {
            fields.retain(|(name, _)| !is_text(name, key));
        }
    }

    fn string(&self) -> Option<&[u16]> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    fn is(&self, s: &str) -> bool { self.string().is_some_and(|v| is_text(v, s)) }

    fn integer(&self) -> i64 {
        match self {
            Self::Integer(n) if (-SAFE..=SAFE).contains(n) => *n,
            _ => 0,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Bool(value) => *value,
            Self::Integer(value) => *value != 0,
            Self::Float(bits) => {
                let value = f64::from_bits(*bits);
                value != 0.0 && !value.is_nan()
            }
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    fn date(now: i64) -> Self { Self::String(text(&super::iso(now))) }
    fn literal(s: &str) -> Self { Self::String(text(s)) }
}

#[derive(Clone, Default)]
pub struct Sessions {
    entries: HashMap<Text, Option<Value>>,
    order: Vec<Text>,
}

impl Sessions {
    pub fn load(&mut self, sid: &[i64], budget: &[i64]) -> i64 {
        let sid = unpack_text(sid);
        let value = (!budget.is_empty()).then(|| Value::decode(budget));
        if let Some(existing) = self.entries.get_mut(&sid) {
            *existing = value;
            return 0;
        }
        self.order.push(sid.clone());
        self.entries.insert(sid, value);
        1
    }

    pub fn dump(&self, out: &mut Vec<i64>) {
        out.clear();
        out.push(self.order.len() as i64);
        for sid in &self.order {
            write_key(out, sid);
            let len_at = out.len();
            out.push(0);
            if let Some(budget) = &self.entries[sid] { budget.write(out); }
            out[len_at] = (out.len() - len_at - 1) as i64;
        }
    }

    pub fn observe_session(&self, key: &[i64]) -> Vec<i64> {
        let Some(budget) = self.entries.get(&unpack_text(key)) else { return Vec::new(); };
        let mut out = vec![1];
        if let Some(budget) = budget { budget.write(&mut out); }
        out
    }

    fn budget(&self, sid: &[u16]) -> Option<&Value> {
        self.entries.get(sid).and_then(Option::as_ref)
    }

    fn current(&mut self, sid: &[u16], rf: &Value) -> Option<&mut Value> {
        let budget = self.entries.get_mut(sid)?.as_mut()?;
        let bid = rf.get("budgetId").string()?;
        (budget.get("id").string() == Some(bid)).then_some(budget)
    }

    // Use the existing hand implementation for the old operations. Only their
    // candidate sessions cross the old V-word ABI.
    pub fn legacy(&mut self, kind: i64, op: i64, a: &[Vec<i64>], now: i64)
        -> Result<(Vec<i64>, Vec<i64>), i64>
    {
        let sid = &a[2];
        let caller = if kind == 1 && op == 3 { &a[3][..] } else { &[] };
        let rf = Value::decode(&a[1]);
        let mut store = Store::new();
        for key in [sid.as_slice(), caller].into_iter().filter(|key| !key.is_empty()) {
            if let Some(budget) = self.budget(&unpack_text(key)) {
                store.sessions.insert(key.to_vec(), budget.encode());
            }
        }
        let bytes = |i: usize| a[i].iter().map(|&word| u8::try_from(word).unwrap()).collect::<Vec<_>>();
        let mut out = Vec::new();
        let status = match kind {
            1 => store.auto(op, &a[1], sid, &a[3], &bytes(4), &mut out),
            2 => store.set_status(&a[1], sid, &bytes(3), &bytes(4), &bytes(5), now, &mut out),
            _ => store.query(op, &a[1], sid, &mut out),
        };
        if status != 0 { return Err(status); }
        let w = &out[1..];
        let mut effects = Effects::default();
        let id = rf.get("sessionId");
        let mut owner = Value::String(key_text(sid));
        let (saved, changed) = match kind {
            1 => {
                if op == 3 {
                    if w[0] == 2 { effects.value(1, id); }
                    if w[0] == 1 || self.current(&key_text(sid), &rf).is_none() {
                        owner = Value::String(key_text(caller));
                    }
                    effects.value(1, &owner);
                    if w[1] == 1 { owner = Value::String(key_text(&w[4..])); }
                } else if w[0] == 1 {
                    effects.value(1, id);
                }
                (w[1], w[2])
            }
            2 => {
                for _ in 0..w[3] { effects.value(1, id); }
                if w[0] == 1 { effects.tick(now); }
                (w[1], w[2])
            }
            _ => {
                if w[0] == 1 { effects.value(1, id); }
                (0, 0)
            }
        };
        if saved == 1 { effects.value(3, &owner); }
        if changed == 1 { effects.mark(4); }
        if kind != 3 {
            for (key, budget) in store.sessions {
                *self.entries.get_mut(&unpack_text(&key)).unwrap() = Some(Value::decode(&budget));
            }
        }
        Ok((w.to_vec(), effects.0))
    }
}

fn key_text(key: &[i64]) -> Text {
    if key.is_empty() { Vec::new() } else { unpack_text(key) }
}

#[derive(Default)]
struct Effects(Vec<i64>);

impl Effects {
    fn push(&mut self, code: i64, delay: i64, effective: i64, write: impl FnOnce(&mut Vec<i64>)) {
        let at = self.0.len();
        self.0.extend([code, 0, delay, effective]);
        write(&mut self.0);
        self.0[at + 1] = (self.0.len() - at - 4) as i64;
    }

    fn value(&mut self, code: i64, value: &Value) {
        self.push(code, 0, 0, |out| value.write(out));
    }

    fn string(&mut self, code: i64, value: &[u16]) {
        self.push(code, 0, 0, |out| {
            out.push(6);
            write_key(out, value);
        });
    }

    fn mark(&mut self, code: i64) { self.push(code, 0, 0, |_| {}); }
    fn tick(&mut self, now: i64) { self.value(2, &Value::Integer(now)); }
    fn timer(&mut self, code: i64, bid: &[u16]) {
        self.push(code, 0, 0, |out| out.extend(packed(bid)));
    }

    fn reply(mut self, result: &Value) -> Vec<i64> {
        self.value(0, result);
        self.0
    }
}

#[derive(Clone)]
struct Clock {
    owner: Text,
    runs: Vec<(Text, bool)>,
    since: Option<i64>,
    armed: bool,
}

#[derive(Clone, Default)]
pub struct Clocks {
    entries: HashMap<Text, Clock>,
    order: Vec<Text>,
    tool_seen: Vec<(Text, Vec<Text>)>,
}

impl Clocks {
    fn clock_of(&mut self, bid: &[u16], sid: &[u16]) -> &mut Clock {
        self.entries.entry(bid.to_vec()).or_insert_with(|| {
            self.order.push(bid.to_vec());
            Clock { owner: sid.to_vec(), runs: Vec::new(), since: None, armed: false }
        })
    }

    fn drop_clock(&mut self, bid: &[u16], effects: &mut Effects) {
        if let Some(clock) = self.entries.remove(bid) {
            if clock.armed { effects.timer(13, bid); }
            self.order.retain(|key| key != bid);
        }
    }

    fn forget(&mut self, sid: &[u16], effects: &mut Effects) {
        self.order.retain(|bid| {
            let clock = &self.entries[bid];
            if clock.owner != sid { return true; }
            if clock.armed { effects.timer(13, bid); }
            self.entries.remove(bid);
            false
        });
    }

    fn dispose(&mut self, effects: &mut Effects) {
        for bid in &self.order {
            if self.entries[bid].armed { effects.timer(13, bid); }
        }
        self.entries.clear();
        self.order.clear();
        self.tool_seen.clear();
    }

    pub fn seed_tool(&mut self, turn: &[i64], activity: &[i64]) -> bool {
        let turn = unpack_text(turn);
        let activity = unpack_text(activity);
        if let Some((_, seen)) = self.tool_seen.iter_mut().find(|(id, _)| *id == turn) {
            if seen.contains(&activity) { return false; }
            seen.push(activity);
        } else {
            self.tool_seen.push((turn, vec![activity]));
        }
        true
    }

    pub fn observe(&self) -> Vec<i64> {
        let mut out = vec![0, self.order.len() as i64];
        for bid in &self.order {
            let clock = &self.entries[bid];
            write_key(&mut out, bid);
            write_key(&mut out, &clock.owner);
            out.extend([
                i64::from(clock.since.is_some()), clock.since.unwrap_or(0),
                i64::from(clock.armed), clock.runs.iter().filter(|(_, waiting)| !waiting).count() as i64,
                clock.runs.len() as i64,
            ]);
            for (run, waiting) in &clock.runs {
                write_key(&mut out, run);
                out.push(i64::from(*waiting));
            }
        }
        out.push(self.tool_seen.len() as i64);
        for (turn, activities) in &self.tool_seen {
            write_key(&mut out, turn);
            out.push(activities.len() as i64);
            for activity in activities { write_key(&mut out, activity); }
        }
        out
    }

    pub fn call(&mut self, sessions: &mut Sessions, op: i64, rf: &Value, sid: &[u16], arg: &Value, now: i64)
        -> Result<Vec<i64>, i64>
    {
        let mut effects = Effects::default();
        let mut result = Value::Undefined;
        let bid = rf.get("budgetId").string().unwrap_or_default();
        match op {
            1 => {
                if let Some(Some(budget)) = sessions.entries.get_mut(sid) {
                    if let Some(id) = budget.get("id").string() { self.drop_clock(id, &mut effects); }
                    budget.remove("activeSince");
                }
            }
            2 => self.forget(sid, &mut effects),
            3 => self.dispose(&mut effects),
            4..=6 => self.run(sessions, op, rf, sid, arg, now, &mut effects)?,
            7 => self.fire(sessions, rf, sid, now, &mut effects)?,
            8 => result = self.attach(sessions, sid, arg, now, &mut effects)?,
            9 => result = self.charge(sessions, rf, sid, arg, now, &mut effects)?,
            10 => {
                let reason = REASONS.into_iter().find(|r| arg.is(r)).ok_or(2i64)?;
                self.halt(sessions, rf, sid, bid, reason, now, &mut effects);
            }
            _ => return Err(2),
        }
        Ok(effects.reply(&result))
    }

    pub fn call_words(&mut self, sessions: &mut Sessions, op: i64, rf: &[i64], sid: &[i64], arg: &[i64], now: i64)
        -> Result<Vec<i64>, i64>
    {
        self.call(sessions, op, &Value::decode(rf), &key_text(sid), &Value::decode(arg), now)
    }

    pub fn fire_words(&mut self, sessions: &mut Sessions, sid: &[i64], bid: &[i64], budget_first: bool, now: i64)
        -> Result<Vec<i64>, i64>
    {
        let sid = key_text(sid);
        let rf = reference(&sid, key_text(bid), budget_first);
        self.call(sessions, 7, &rf, &sid, &Value::Undefined, now)
    }

    fn attach(&mut self, sessions: &mut Sessions, sid: &[u16], arg: &Value, now: i64, effects: &mut Effects)
        -> Result<Value, i64>
    {
        let resident = sessions.entries.get_mut(sid).ok_or(2i64)?;
        if let Some(budget) = resident {
            if let Some(stale) = budget.get("id").string() {
                if !budget.get("state").is("closed") { return Ok(reference(sid, stale.to_vec(), false)); }
                if self.entries.get(stale).is_some_and(|clock| clock.armed) {
                    effects.timer(13, stale);
                }
            }
        }
        let unit_words = m12_facts::unit(effects.0.len() as i64);
        let unit = Value::decode(&unit_words);
        effects.value(5, &unit);
        let size = arg.get("size");
        let (times, unlimited) = if matches!(size, Value::Null | Value::Undefined) {
            (1, unit.get("defaultUnlimited").truthy())
        } else if size.is("unlimited") {
            (1, true)
        } else if matches!(size.integer(), 1 | 2 | 4 | 8) {
            (size.integer(), false)
        } else {
            return Err(2);
        };
        let bid = key_text(&m12_facts::uuid(effects.0.len() as i64));
        effects.string(6, &bid);
        effects.tick(now);
        let stale = resident.as_ref().and_then(|budget| budget.get("id").string());
        if stale == Some(bid.as_slice()) || self.entries.contains_key(&bid) { return Err(2); }
        let limits = LIMITS.map(|key| (key, unit.get(key).integer() * times));
        if limits.iter().any(|&(_, value)| !(-SAFE..=SAFE).contains(&value)) { return Err(5); }
        let limits = Value::object(limits.map(|(key, value)| (key, Value::Integer(value))));
        let used = Value::object(["activeMs", "delegations", "commands", "reviews", "checks", "tools", "failures", "retries"]
            .map(|key| (key, Value::Integer(0))));
        let mut budget = Value::object([
            ("id", Value::String(bid.clone())), ("openedAt", Value::date(now)),
            ("origin", arg.get("origin").clone()), ("state", Value::literal("open")), ("limits", limits),
        ]);
        if unlimited { budget.put("unlimited", Value::Bool(true)); }
        budget.put("used", used);
        if let Some(stale) = stale {
            if self.entries.remove(stale).is_some() {
                self.order.retain(|key| key != stale);
            }
        }
        *resident = Some(budget);
        effects.mark(4);
        Ok(reference(sid, bid, false))
    }

    fn charge(&mut self, sessions: &mut Sessions, rf: &Value, sid: &[u16], arg: &Value, now: i64, effects: &mut Effects)
        -> Result<Value, i64>
    {
        effects.value(1, rf.get("sessionId"));
        let Some(budget) = sessions.current(sid, rf) else { return Ok(charged(None)); };
        let state = budget.get("state");
        if state.is("halted") {
            let reason = REASONS.into_iter().find(|r| budget.get("halt").get("reason").is(r));
            return Ok(charged(Some(reason.unwrap_or("delegations"))));
        }
        if !state.is("open") { return Ok(charged(None)); }

        let counters = budget.get("used");
        let used = CHARGES.map(|key| counters.get(key).integer());
        let amounts = CHARGES.map(|key| arg.get(key).integer());
        if !budget.get("unlimited").truthy() {
            let limits = budget.get("limits");
            for (index, key) in COUNTS.into_iter().enumerate() {
                let amount = amounts[index];
                if amount <= 0 { continue; }
                if amount > SAFE - used[index] { return Err(5); }
                if amount > limits.get(key).integer() - used[index] {
                    let bid = rf.get("budgetId").string().unwrap();
                    self.halt(sessions, rf, sid, bid, key, now, effects);
                    return Ok(charged(Some(key)));
                }
            }
        }
        let updated: [i64; CHARGES.len()] = std::array::from_fn(|index| used[index] + amounts[index]);
        if updated.iter().any(|value| !(-SAFE..=SAFE).contains(value)) { return Err(5); }
        budget.edit_object("used", |used| {
            for (key, value) in CHARGES.into_iter().zip(updated) { used.put(key, Value::Integer(value)); }
        });
        Ok(charged(None))
    }

    fn halt(&mut self, sessions: &mut Sessions, rf: &Value, sid: &[u16], bid: &[u16], reason: &str, now: i64, effects: &mut Effects) {
        if m12_facts::observing() {
            if let Some(budget) = sessions.budget(sid) {
                let words = budget.encode();
                m12_facts::observe_find(effects.0.len() as i64, &words);
                m12_facts::observe_find((effects.0.len() + 6 + sid.len()) as i64, &words);
            }
        }
        effects.string(1, sid);
        effects.string(1, sid);
        let Some(budget) = sessions.current(sid, rf).filter(|b| b.get("state").is("open")) else { return; };
        if let Some(clock) = self.entries.get_mut(bid) { settle(clock, budget, sid, bid, now, effects); }
        let words = m12_facts::running_ids(&rf.encode(), effects.0.len() as i64);
        let mut rest = &words[1..];
        let mut ids = Vec::with_capacity(words[0] as usize);
        for _ in 0..words[0] {
            let (key, tail) = rest.split_at(1 + (rest[0] as usize).div_ceil(4));
            ids.push(Value::String(unpack_text(key)));
            rest = tail;
        }
        let list = Value::Array(ids.clone());
        effects.value(7, &Value::Array(vec![rf.clone(), list.clone()]));
        budget.put("state", Value::literal("halted"));
        effects.tick(now);
        budget.put("halt", Value::object([
            ("reason", Value::literal(reason)), ("at", Value::date(now)), ("stoppedDelegationIds", list),
        ]));
        effects.string(8, sid);
        effects.string(9, sid);
        let owner = Value::String(sid.to_vec());
        for id in ids { effects.value(10, &Value::Array(vec![owner.clone(), id])); }
        effects.value(11, rf);
        effects.value(15, &Value::Array(vec![owner, Value::String(bid.to_vec())]));
        effects.string(3, sid);
        effects.mark(4);
        effects.string(16, sid);
    }

    fn run(&mut self, sessions: &mut Sessions, op: i64, rf: &Value, sid: &[u16], arg: &Value, now: i64, effects: &mut Effects)
        -> Result<(), i64>
    {
        let id = rf.get("sessionId");
        let bid = rf.get("budgetId").string().unwrap_or_default();
        let run = arg.get("run").string().unwrap_or_default();
        let clock = if op == 4 {
            effects.value(1, id);
            if sessions.current(sid, rf).is_none() { return Ok(()); }
            let clock = self.clock_of(bid, sid);
            if let Some((_, waiting)) = clock.runs.iter_mut().find(|(key, _)| key == run) {
                *waiting = false;
            } else {
                clock.runs.push((run.to_vec(), false));
            }
            clock
        } else {
            let Some(clock) = self.entries.get_mut(bid) else { return Ok(()); };
            let Some(at) = clock.runs.iter().position(|(key, _)| key == run) else { return Ok(()); };
            if op == 5 {
                let waiting = arg.get("waiting").truthy();
                if clock.runs[at].1 == waiting { return Ok(()); }
                clock.runs[at].1 = waiting;
            } else {
                clock.runs.remove(at);
            }
            clock
        };
        effects.value(1, id);
        let Some(budget) = sessions.current(sid, rf) else { return Ok(()); };
        let consuming = budget.get("state").is("open") && clock.runs.iter().any(|(_, waiting)| !waiting);
        if consuming == clock.since.is_some() { return Ok(()); }
        if !consuming {
            settle(clock, budget, sid, bid, now, effects);
        } else if !begin(clock, budget, rf, bid, now, effects) {
            self.fire(sessions, rf, sid, now, effects)?;
        }
        Ok(())
    }

    fn fire(&mut self, sessions: &mut Sessions, rf: &Value, sid: &[u16], now: i64, effects: &mut Effects)
        -> Result<(), i64>
    {
        let bid = rf.get("budgetId").string().unwrap_or_default();
        if m12_facts::observing() {
            if let Some(budget) = sessions.budget(sid) {
                m12_facts::observe_find(effects.0.len() as i64, &budget.encode());
            }
        }
        effects.value(1, rf.get("sessionId"));
        let budget = sessions.current(sid, rf);
        let clock = self.entries.get_mut(bid);
        if let Some(clock) = &clock {
            if clock.armed { effects.timer(14, bid); }
        }
        let clock = clock.map(|clock| {
            clock.armed = false;
            clock
        });
        let Some(budget) = budget.filter(|b| b.get("state").is("open") && !b.get("unlimited").truthy()) else {
            return Ok(());
        };
        if let Some(clock) = clock {
            settle(clock, budget, sid, bid, now, effects);
            if left(budget) > 0 && !clock.runs.is_empty() {
                begin(clock, budget, rf, bid, now, effects);
                return Ok(());
            }
        }
        self.halt(sessions, rf, sid, bid, "time", now, effects);
        Ok(())
    }
}

pub fn reference(sid: &[u16], bid: Text, budget_first: bool) -> Value {
    let mut fields = [("sessionId", Value::String(sid.to_vec())), ("budgetId", Value::String(bid))];
    if budget_first { fields.swap(0, 1); }
    Value::object(fields)
}

fn charged(reason: Option<&str>) -> Value {
    let mut value = Value::object([("ok", Value::Bool(reason.is_none()))]);
    if let Some(reason) = reason { value.put("reason", Value::literal(reason)); }
    value
}

fn left(budget: &Value) -> i64 {
    let minutes = budget.get("limits").get("timeMinutes").integer();
    let limit = minutes.clamp(-150_000_000_000_000, 150_000_000_000_000) * 60_000;
    limit - budget.get("used").get("activeMs").integer()
}

fn settle(clock: &mut Clock, budget: &mut Value, sid: &[u16], bid: &[u16], now: i64, effects: &mut Effects) {
    if clock.owner != sid { return; }
    if clock.armed {
        effects.timer(13, bid);
        clock.armed = false;
    }
    if let Some(since) = clock.since.take() {
        effects.tick(now);
        budget.remove("activeSince");
        budget.edit_object("used", |used| {
            let active = used.get("activeMs").integer() + (now - since).max(0);
            used.put("activeMs", Value::Integer(active.clamp(-SAFE - 1, SAFE + 1)));
        });
    }
}

// False means the timer is already due and FIRE must run synchronously.
fn begin(clock: &mut Clock, budget: &mut Value, rf: &Value, bid: &[u16], now: i64, effects: &mut Effects) -> bool {
    effects.tick(now);
    clock.since = Some(now);
    budget.put("activeSince", Value::date(now));
    if budget.get("unlimited").truthy() { return true; }
    let delay = left(budget);
    if delay <= 0 { return false; }
    let budget_first = matches!(rf, Value::Object(fields) if fields.iter()
        .find(|(key, _)| is_text(key, "budgetId") || is_text(key, "sessionId"))
        .is_some_and(|(key, _)| is_text(key, "budgetId")));
    effects.push(12, delay, if delay > i64::from(i32::MAX) { 1 } else { delay }, |out| {
        out.extend(packed(bid));
        out.push(i64::from(budget_first));
    });
    clock.armed = true;
    true
}

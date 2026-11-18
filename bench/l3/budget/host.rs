// Host of the L3-10 request budget, milestone M5 (crate `budget_host`): the side-effect mechanics only.
// It executes an "effect program" (generated from the Node oracle elsewhere) mechanically and computes every
// host-owned value itself: now, uuid, timer tokens / due times / firing order, query replies, findSession results,
// sref / bref labels, session storage, final sessions and active timers. It makes no budget decisions.
// Mirrors oracle-world.mjs + reference.mjs. Also holds the resident-state driver (ResidentState) over the M4 entries.
// Included from src/lib.rs before leaf.rs; only plain `//` comments (include! forbids inner doc comments).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::rc::Rc;

const DEFAULT_CLOCK_START: f64 = 1700000000000.0;
const REPLY_DEPS: &[&str] = &["runningDelegationIds", "isQuiet", "releaseHeld"];
const METHODS: &[&str] = &[
    "current", "isHalted", "attach", "replaceForUserMessage", "reopen", "charge", "noteFailure", "noteTool", "forgetTurn", "runStarted",
    "runWaiting", "runEnded", "halt", "maybeClose", "raise", "decide", "setCard", "hasCard", "clearAutoContinue", "recordAutoContinue",
    "stopAutoContinue", "bootRestore", "forgetSession", "dispose",
];
const HELPERS: &[&str] = &["limitText", "refusalWire", "refusal", "continueText", "cardMissing", "budgetUnitOf"];
const SESSION_FIRST: &[&str] = &["attach", "replaceForUserMessage", "maybeClose", "bootRestore"];

fn qidx(name: &str) -> Option<usize> {
    REPLY_DEPS.iter().position(|d| *d == name)
}

// Order-preserving key of an f64 for the timer set (ascending = ascending due; -0 and 0 equal; NaN last).
fn okey(x: f64) -> u64 {
    if x.is_nan() {
        return u64::MAX;
    }
    let x = if x == 0.0 { 0.0 } else { x };
    let b = x.to_bits();
    if b >> 63 == 1 {
        !b
    } else {
        b | (1u64 << 63)
    }
}

pub enum Slot {
    Obj(usize),
    Plain(V),
}

pub struct Timer {
    // the token is "t<tok>"; tok is also the registration sequence number
    pub tok: u64,
    // JSON text of the program key that registered the timer
    pub key: String,
    pub delay: V,
    pub due: f64,
    pub i: Option<String>,
}

pub struct World {
    pub tracing: bool,
    pub heap: Heap,
    pub now: V,
    now_override: VecDeque<V>,
    pub unit: V,
    uuids: Option<Vec<V>>,
    uuid_n: usize,
    queues: [VecDeque<V>; 3],
    throws: HashMap<String, Vec<u64>>,
    dep_calls: HashMap<String, u64>,
    timers: Vec<Timer>,
    active: BTreeSet<(u64, u64)>,
    key_tok: HashMap<String, u64>,
    timer_has_unref: bool,
    pub stored: Vec<Slot>,
    // string id (as `idx_key` bytes) -> (position in `stored`, heap handle) of the first Obj slot with that id
    idx: HashMap<Vec<u8>, (usize, usize)>,
    pub trace: Vec<J>,
    requests: Vec<J>,
    excluded: Vec<String>,
    pub violations: Vec<String>,
    cur_r: J,
    cur_i: Option<String>,
    segs: Rc<Vec<J>>,
    pos: usize,
    has_prog: bool,
    // a driver error raised inside a call / timer callback (uuid exhaustion): reported after the callback ends,
    // like the oracle, whose OracleError passes through the pinned class and is rethrown at the end of the call.
    deferred: Option<String>,
}

fn opt_s(o: &Option<String>) -> J {
    match o {
        Some(s) => J::s(s),
        None => J::Null,
    }
}

fn req_obj(id: &J, inst: &str, op: &str, terminal: &str, value: &J) -> J {
    J::Obj(vec![
        (ku("id"), id.clone()),
        (ku("instance"), J::s(inst)),
        (ku("op"), J::s(op)),
        (ku("terminal"), J::s(terminal)),
        (ku("value"), value.clone()),
    ])
}

// A fire segment without items (perf).
pub fn empty_fire_seg(key: &str) -> J {
    J::Obj(vec![(ku("kind"), J::s("fire")), (ku("key"), J::s(key)), (ku("items"), J::Arr(Vec::new()))])
}

// Index key bytes of a UTF-16 string id: the units themselves when all are ASCII, else 0xFF and then each unit's two
// bytes (little endian). ASCII keys never hold 0xFF, so two different ids never share a key.
fn idx_key(u: &[u16]) -> Vec<u8> {
    if u.iter().all(|&c| c < 0x80) {
        return u.iter().map(|&c| c as u8).collect();
    }
    let mut v = Vec::with_capacity(1 + 2 * u.len());
    v.push(0xFF);
    for &c in u {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v
}

// A session object value {id, n} (perf).
pub fn session_v(id: &str, n: f64) -> V {
    V::Obj(vec![(ku("id"), V::Str(u16s(id))), (ku("n"), V::Num(n))])
}

impl World {
    pub fn new(tracing: bool) -> World {
        World {
            tracing,
            heap: Heap::new(),
            now: V::Num(DEFAULT_CLOCK_START),
            now_override: VecDeque::new(),
            unit: V::Undef,
            uuids: None,
            uuid_n: 0,
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            throws: HashMap::new(),
            dep_calls: HashMap::new(),
            timers: Vec::new(),
            active: BTreeSet::new(),
            key_tok: HashMap::new(),
            timer_has_unref: true,
            stored: Vec::new(),
            idx: HashMap::new(),
            trace: Vec::new(),
            requests: Vec::new(),
            excluded: Vec::new(),
            violations: Vec::new(),
            cur_r: J::Null,
            cur_i: None,
            segs: Rc::new(Vec::new()),
            pos: 0,
            has_prog: false,
            deferred: None,
        }
    }

    pub fn set_program(&mut self, segs: Vec<J>) {
        self.segs = Rc::new(segs);
        self.pos = 0;
        self.has_prog = true;
    }

    pub fn rewind(&mut self) {
        self.pos = 0;
    }

    pub fn segs_left(&self) -> usize {
        self.segs.len() - self.pos
    }

    fn now_f(&self) -> f64 {
        match &self.now {
            V::Num(x) => *x,
            _ => f64::NAN,
        }
    }

    // ---- decoding / lookup ------------------------------------------------------------------------------------

    fn id_is(&self, h: usize, id: &V) -> bool {
        match field(&self.heap.objs[h].val, "id") {
            Some(x) => strict_eq(x, id),
            None => false,
        }
    }

    // Rebuild the id index: string id -> position of the first Obj slot with that id (first wins).
    fn rebuild_idx(&mut self) {
        let mut m: HashMap<Vec<u8>, (usize, usize)> = HashMap::new();
        for (pos, s) in self.stored.iter().enumerate() {
            if let Slot::Obj(h) = s {
                if let Some(V::Str(u)) = field(&self.heap.objs[*h].val, "id") {
                    m.entry(idx_key(u)).or_insert((pos, *h));
                }
            }
        }
        self.idx = m;
    }

    #[inline]
    fn find_key(&self, u: &[u16]) -> Option<usize> {
        if u.len() <= 64 && u.iter().all(|&c| c < 0x80) {
            // An ASCII id is its own key: narrowed into a stack buffer (no allocation per lookup).
            let mut buf = [0u8; 64];
            for (b, &c) in buf.iter_mut().zip(u) {
                *b = c as u8;
            }
            return self.idx.get(&buf[..u.len()]).map(|&(_, h)| h);
        }
        self.idx.get(idx_key(u).as_slice()).map(|&(_, h)| h)
    }

    // A string id: an ASCII id probes the index with its own bytes; any other id goes through its UTF-16 units.
    #[inline]
    fn find_str(&self, id: &str) -> Option<usize> {
        if id.is_ascii() {
            return self.idx.get(id.as_bytes()).map(|&(_, h)| h);
        }
        self.find_key(&u16s(id))
    }

    fn find_h(&self, id: &V) -> Option<usize> {
        if let V::Str(u) = id {
            return self.find_key(u);
        }
        for s in &self.stored {
            if let Slot::Obj(h) = s {
                if self.id_is(*h, id) {
                    return Some(*h);
                }
            }
        }
        None
    }

    fn dec(&self, j: &J) -> Result<V, String> {
        let f = |id: &[u16]| -> Option<usize> { self.find_h(&V::Str(id.to_vec())) };
        decode(j, &f)
    }

    fn dec_opt(&self, j: Option<&J>) -> Result<V, String> {
        match j {
            Some(x) => self.dec(x),
            None => Ok(V::Undef),
        }
    }

    // ---- trace -------------------------------------------------------------------------------------------------

    fn push_trace(&mut self, c: &str, a: Option<J>, o: Option<J>) {
        if !self.tracing {
            return;
        }
        let mut e = vec![(ku("c"), J::s(c)), (ku("r"), self.cur_r.clone()), (ku("i"), opt_s(&self.cur_i))];
        if let Some(a) = a {
            e.push((ku("a"), a));
        }
        if let Some(o) = o {
            e.push((ku("o"), o));
        }
        self.trace.push(J::Obj(e));
    }

    // Program JV: raise the label counters, then write sref snapshots through to the shared objects.
    #[inline]
    pub fn scan(&mut self, j: &J) -> Result<(), String> {
        self.heap.note_raw(j);
        self.heap.write_through(j)
    }

    // ---- world setup ---------------------------------------------------------------------------------------------

    fn init_world(&mut self, sc: &J) -> Result<(), String> {
        let empty = J::Obj(Vec::new());
        let world: &J = match nn(sc.get("world")) {
            Some(w) => w,
            None => &empty,
        };
        if let Some(c) = nn(world.get("clockStart")) {
            self.now = self.dec(c)?;
        }
        if let Some(J::Bool(b)) = nn(world.get("timerHasUnref")) {
            self.timer_has_unref = *b;
        }
        let mut sessions: Vec<Slot> = Vec::new();
        if let Some(J::Arr(list)) = nn(world.get("sessions")) {
            for x in list {
                let v = self.dec(x)?;
                if matches!(v, V::Obj(_)) {
                    let h = self.heap.alloc(v);
                    sessions.push(Slot::Obj(h));
                } else {
                    sessions.push(Slot::Plain(v));
                }
            }
        }
        self.stored = sessions;
        self.rebuild_idx();
        self.unit = match world.get("unit") {
            Some(u) => self.dec(u)?,
            None => V::Undef,
        };
        if let Some(J::Arr(list)) = world.get("uuids") {
            let mut us: Vec<V> = Vec::with_capacity(list.len());
            for x in list {
                us.push(self.dec(x)?);
            }
            self.uuids = Some(us);
        }
        if let Some(deps) = nn(world.get("deps")) {
            for (qi, name) in REPLY_DEPS.iter().enumerate() {
                if let Some(J::Arr(list)) = deps.get(name) {
                    let mut q: VecDeque<V> = VecDeque::new();
                    for x in list {
                        q.push_back(self.dec(x)?);
                    }
                    self.queues[qi] = q;
                }
            }
            if let Some(J::Obj(th)) = deps.get("throws") {
                for e in th {
                    if let J::Arr(ns) = &e.1 {
                        let mut list: Vec<u64> = Vec::new();
                        for n in ns {
                            if let J::Num(t) = n {
                                if let Ok(x) = t.parse::<f64>() {
                                    if x >= 0.0 {
                                        list.push(x as u64);
                                    }
                                }
                            }
                        }
                        self.throws.insert(String::from_utf16_lossy(&e.0), list);
                    }
                }
            }
        }
        Ok(())
    }

    // ---- program consumption -------------------------------------------------------------------------------------

    fn next_seg(&mut self) -> Result<Rc<Vec<J>>, String> {
        if self.pos >= self.segs.len() {
            return Err("program: segments exhausted".to_string());
        }
        Ok(self.segs.clone())
    }

    // ---- scenario ------------------------------------------------------------------------------------------------

    fn run_inner(&mut self, sc: &J) -> Result<(), String> {
        if !self.has_prog {
            let id = match sc.get("id") {
                Some(x) => plain(x),
                None => "null".to_string(),
            };
            return Err(format!("program: no program for case {}", id));
        }
        self.init_world(sc)?;
        let mut instances: Vec<String> = Vec::new();
        match nn(sc.get("instances")) {
            Some(J::Arr(a)) => {
                for x in a {
                    instances.push(plain(x));
                }
            }
            _ => instances.push("A".to_string()),
        }
        let script: &[J] = match sc.get("script") {
            Some(J::Arr(a)) => a.as_slice(),
            _ => &[],
        };
        for step in script {
            self.run_step(step, &instances)?;
        }
        if self.pos < self.segs.len() {
            return Err(format!("program: {} unused segments", self.segs.len() - self.pos));
        }
        Ok(())
    }

    fn run_step(&mut self, step: &J, instances: &[String]) -> Result<(), String> {
        if !matches!(step, J::Obj(_)) {
            return Err("script step is not an object".to_string());
        }
        let ev = step.get("ev").and_then(|x| x.text()).unwrap_or_default();
        match ev.as_str() {
            "call" => self.call_step(step, instances),
            "advance" => {
                let ms = self.dec_opt(step.get("ms"))?;
                if self.tracing {
                    let e = self.heap.enc(&ms);
                    self.push_trace("advance", Some(enc_obj(vec![("ms", e)])), None);
                }
                match ms {
                    V::Num(x) if !x.is_nan() => self.advance_by(x),
                    _ => Err("advance needs a numeric ms".to_string()),
                }
            }
            "setTime" => {
                let v = self.dec_opt(step.get("now"))?;
                self.now = v;
                if self.tracing {
                    let nv = self.now.clone();
                    let e = self.heap.enc(&nv);
                    self.push_trace("setTime", Some(enc_obj(vec![("now", e)])), None);
                }
                Ok(())
            }
            "deliverTimer" => {
                let captured = matches!(step.get("captured"), Some(J::Bool(true)));
                let tok = step.get("token").unwrap_or(&JNULL);
                self.deliver_timer(tok, captured)
            }
            "setUnit" => {
                let v = self.dec_opt(step.get("unit"))?;
                self.unit = v;
                if self.tracing {
                    let u = self.unit.clone();
                    let e = self.heap.enc(&u);
                    self.push_trace("setUnit", Some(e), None);
                }
                Ok(())
            }
            "putSession" => {
                let v = self.dec_opt(step.get("session"))?;
                let h = self.put_session(v)?;
                if self.tracing {
                    let e = self.heap.enc(&V::Sess(h));
                    self.push_trace("putSession", Some(e), None);
                }
                Ok(())
            }
            "removeSession" => {
                let id = self.dec_opt(step.get("id"))?;
                let heap = &self.heap;
                let before = self.stored.len();
                self.stored.retain(|s| match s {
                    Slot::Obj(h) => !match field(&heap.objs[*h].val, "id") {
                        Some(x) => strict_eq(x, &id),
                        None => false,
                    },
                    Slot::Plain(_) => true,
                });
                // Positions shift only when something was removed.
                if self.stored.len() != before {
                    self.rebuild_idx();
                }
                if self.tracing {
                    let e = self.heap.enc(&id);
                    self.push_trace("removeSession", Some(enc_obj(vec![("id", e)])), None);
                }
                Ok(())
            }
            "setDepReply" => {
                let dep = step.get("dep").unwrap_or(&JNULL);
                let qi = match dep.text().and_then(|t| qidx(&t)) {
                    Some(i) => i,
                    None => return Err(format!("setDepReply: {} has no reply queue", json_text(dep))),
                };
                let list = match step.get("replies") {
                    Some(J::Arr(a)) => a,
                    _ => return Err("setDepReply needs a replies array".to_string()),
                };
                let mut q: VecDeque<V> = VecDeque::new();
                for x in list {
                    q.push_back(self.dec(x)?);
                }
                if self.tracing {
                    let mut encs: Vec<J> = Vec::with_capacity(q.len());
                    for v in q.iter() {
                        encs.push(self.heap.enc(v));
                    }
                    self.push_trace("setDepReply", Some(enc_obj(vec![("dep", J::s(REPLY_DEPS[qi])), ("replies", J::Arr(encs))])), None);
                }
                self.queues[qi] = q;
                Ok(())
            }
            "snapshot" => self.snapshot_step(step),
            _ => Err(format!("unknown step ev {}", json_text(step.get("ev").unwrap_or(&JNULL)))),
        }
    }

    // putSession: an object with a string id; a new object replaces the stored one with the same id in place.
    pub fn put_session(&mut self, v: V) -> Result<usize, String> {
        let ok = match &v {
            V::Obj(_) => matches!(field(&v, "id"), Some(V::Str(_))),
            V::Sess(h) => matches!(field(&self.heap.objs[*h].val, "id"), Some(V::Str(_))),
            _ => false,
        };
        if !ok {
            return Err("putSession needs an object session with a string id".to_string());
        }
        let h = match v {
            V::Sess(h) => h,
            other => self.heap.alloc(other),
        };
        // The id is a V::Str here (checked above); look the first match up in the index.
        let key: Vec<u8> = match field(&self.heap.objs[h].val, "id") {
            Some(V::Str(u)) => idx_key(u),
            _ => Vec::new(),
        };
        match self.idx.get_mut(&key) {
            // Same id, same position: only the handle changes.
            Some(e) => {
                self.stored[e.0] = Slot::Obj(h);
                e.1 = h;
            }
            None => {
                self.stored.push(Slot::Obj(h));
                self.idx.insert(key, (self.stored.len() - 1, h));
            }
        }
        Ok(h)
    }

    // findSession as a world query without trace (perf): the encoded result.
    // Lookup only: the stored session's heap handle (the lookup step of a non-tracing find; perf breakdown).
    #[inline]
    pub fn find_session(&self, id: &str) -> Option<usize> {
        self.find_str(id)
    }

    pub fn find_session_enc(&mut self, id: &str) -> J {
        match self.find_str(id) {
            Some(h) => self.heap.enc(&V::Sess(h)),
            None => J::Obj(vec![(ku("undef"), J::int(1))]),
        }
    }

    // find_session_enc written into `out` (cleared first) as JSON text.
    pub fn find_session_text(&mut self, id: &str, out: &mut String) {
        out.clear();
        match self.find_str(id) {
            Some(h) => self.heap.enc_into(&V::Sess(h), out),
            None => out.push_str("{\"undef\":1}"),
        }
    }

    pub fn encode_all_sessions(&mut self) -> Vec<J> {
        let mut out: Vec<J> = Vec::with_capacity(self.stored.len());
        for s in &self.stored {
            match s {
                Slot::Obj(h) => out.push(self.heap.enc(&V::Sess(*h))),
                Slot::Plain(v) => out.push(self.heap.enc(v)),
            }
        }
        out
    }

    fn snapshot_step(&mut self, step: &J) -> Result<(), String> {
        let segs = self.next_seg()?;
        let seg = &segs[self.pos];
        if seg.get("kind").and_then(|x| x.text()).as_deref() != Some("snap") {
            return Err("program: expected snap segment".to_string());
        }
        self.pos += 1;
        let clocks = seg.get("clocks").cloned().unwrap_or(J::Obj(Vec::new()));
        let tool_seen = seg.get("toolSeen").cloned().unwrap_or(J::Obj(Vec::new()));
        self.scan(&clocks)?;
        self.scan(&tool_seen)?;
        if !self.tracing {
            return Ok(());
        }
        let label = self.dec_opt(step.get("label"))?;
        let le = self.heap.enc(&label);
        let a = enc_obj(vec![("label", le)]);
        let now = self.now.clone();
        let now_e = self.heap.enc(&now);
        let unit = self.unit.clone();
        let unit_e = self.heap.enc(&unit);
        let sess = self.encode_all_sessions();
        let mut timers: Vec<J> = Vec::new();
        for &(_, tok) in self.active.iter() {
            let t = &self.timers[(tok - 1) as usize];
            let de = self.heap.enc(&t.delay);
            let ue = enc_num(t.due);
            timers.push(enc_obj(vec![("token", J::s(&format!("t{}", t.tok))), ("delay", de), ("due", ue), ("i", opt_s(&t.i))]));
        }
        let o = enc_obj(vec![
            ("now", now_e),
            ("unit", unit_e),
            ("sessions", J::Arr(sess)),
            ("clocks", clocks),
            ("toolSeen", tool_seen),
            ("timers", J::Arr(timers)),
        ]);
        self.push_trace("snapshot", Some(a), Some(o));
        Ok(())
    }

    // ---- call step ------------------------------------------------------------------------------------------------

    fn call_step(&mut self, step: &J, instances: &[String]) -> Result<(), String> {
        let op = step.get("op").and_then(|x| x.text()).unwrap_or_default();
        let helper = HELPERS.contains(&op.as_str());
        if !helper && !METHODS.contains(&op.as_str()) {
            return Err(format!("unknown op {}", json_text(step.get("op").unwrap_or(&JNULL))));
        }
        let name = match nn(step.get("i")) {
            Some(x) => plain(x),
            None => "A".to_string(),
        };
        if !helper && !instances.contains(&name) {
            return Err(format!("unknown instance {}", json_text(&J::s(&name))));
        }
        let args_j: &[J] = match step.get("args") {
            None => &[],
            Some(J::Arr(a)) => a.as_slice(),
            Some(_) => return Err("call args must be an array".to_string()),
        };
        let now_j: &[J] = match step.get("now") {
            None => &[],
            Some(J::Arr(a)) => a.as_slice(),
            Some(_) => return Err("call now must be an array".to_string()),
        };
        let mut args: Vec<V> = Vec::with_capacity(args_j.len());
        for x in args_j {
            args.push(self.dec(x)?);
        }
        let mut now_list: VecDeque<V> = VecDeque::new();
        for x in now_j {
            now_list.push_back(self.dec(x)?);
        }
        if SESSION_FIRST.contains(&op.as_str()) {
            if let Some(first) = args.get_mut(0) {
                if matches!(first, V::Obj(_)) {
                    let v = std::mem::replace(first, V::Undef);
                    let h = self.heap.alloc(v);
                    *first = V::Sess(h);
                }
            }
        }
        let req = step.get("req").cloned().unwrap_or(J::Null);
        let ri = self.requests.len();
        self.requests.push(req_obj(&req, &name, &op, "pending", &J::Null));
        self.cur_r = req.clone();
        self.cur_i = Some(name.clone());
        self.now_override = now_list;
        if self.tracing {
            let mut encs: Vec<J> = Vec::with_capacity(args.len());
            for a in &args {
                encs.push(self.heap.enc(a));
            }
            self.push_trace("call.start", Some(enc_obj(vec![("op", enc_str(&u16s(&op))), ("args", J::Arr(encs))])), None);
        }

        let segs = self.next_seg()?;
        let seg = &segs[self.pos];
        let kind_ok = seg.get("kind").and_then(|x| x.text()).as_deref() == Some("call");
        if !kind_ok || seg.get("r") != Some(&req) {
            return Err(format!("program: expected call {}", plain(&req)));
        }
        self.pos += 1;
        if let Some(J::Arr(items)) = seg.get("items") {
            self.exec_items(items)?;
        }
        let terminal = seg.get("terminal").and_then(|x| x.text()).unwrap_or_else(|| "ok".to_string());
        let value = seg.get("value").cloned().unwrap_or(J::Null);
        self.scan(&value)?;
        let value = self.heap.host_view(&value);
        self.requests[ri] = req_obj(&req, &name, &op, &terminal, &value);
        self.push_trace("call.end", Some(enc_obj(vec![("terminal", J::s(&terminal))])), None);
        self.cur_r = J::Null;
        self.cur_i = None;
        self.now_override.clear();
        match self.deferred.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    // ---- program items ----------------------------------------------------------------------------------------------

    fn exec_items(&mut self, items: &[J]) -> Result<(), String> {
        for it in items {
            self.exec_item(it)?;
        }
        Ok(())
    }

    // Count one dependency call; when a scripted throw hits, record it and return true (the caller skips the effect).
    fn dep_call(&mut self, name: &str, a: Option<&J>) -> bool {
        if self.throws.is_empty() {
            return false;
        }
        let n = {
            let e = self.dep_calls.entry(name.to_string()).or_insert(0);
            *e += 1;
            *e
        };
        let hit = match self.throws.get(name) {
            Some(l) => l.contains(&n),
            None => false,
        };
        if hit {
            if !self.excluded.iter().any(|x| x == "dep-throw") {
                self.excluded.push("dep-throw".to_string());
            }
            let err = J::Obj(vec![
                (ku("name"), J::s("Error")),
                (ku("message"), J::s(&format!("dep-throw {} {}", name, n))),
            ]);
            let o = J::Obj(vec![(ku("throw"), J::Obj(vec![(ku("error"), err)]))]);
            self.push_trace(name, a.cloned(), Some(o));
        }
        hit
    }

    fn exec_item(&mut self, it: &J) -> Result<(), String> {
        let k = match it.get("k").and_then(|x| x.text()) {
            Some(k) => k,
            None => return Err("program: item without k".to_string()),
        };
        match k.as_str() {
            "now" => {
                if self.dep_call("now", None) {
                    return Ok(());
                }
                let v = match self.now_override.pop_front() {
                    Some(x) => x,
                    None => self.now.clone(),
                };
                if self.tracing {
                    let e = self.heap.enc(&v);
                    self.push_trace("now", None, Some(e));
                }
            }
            "unit" => {
                if self.dep_call("unit", None) {
                    return Ok(());
                }
                if self.tracing {
                    let u = self.unit.clone();
                    let e = self.heap.enc(&u);
                    self.push_trace("unit", None, Some(e));
                }
            }
            "uuid" => {
                let v = match &self.uuids {
                    Some(list) => {
                        if self.uuid_n >= list.len() {
                            if self.deferred.is_none() {
                                self.deferred = Some("uuid stream exhausted".to_string());
                            }
                            return Ok(());
                        }
                        let v = list[self.uuid_n].clone();
                        self.uuid_n += 1;
                        v
                    }
                    None => {
                        self.uuid_n += 1;
                        V::Str(u16s(&format!("00000000-0000-4000-8000-{:012x}", self.uuid_n)))
                    }
                };
                if self.tracing {
                    let e = self.heap.enc(&v);
                    self.push_trace("uuid", None, Some(e));
                }
            }
            "find" => {
                let idj = it.get("id").cloned().unwrap_or(J::Null);
                self.scan(&idj)?;
                let a = J::Arr(vec![idj.clone()]);
                if self.dep_call("findSession", Some(&a)) {
                    return Ok(());
                }
                let idv = self.dec(&idj)?;
                let found = self.find_h(&idv);
                if self.tracing {
                    let e = match found {
                        Some(h) => self.heap.enc(&V::Sess(h)),
                        None => J::Obj(vec![(ku("undef"), J::int(1))]),
                    };
                    self.push_trace("findSession", Some(a), Some(e));
                }
            }
            "bset" => {
                let s = it.get("s").and_then(|x| x.text()).ok_or_else(|| "program: bset without s".to_string())?;
                if let Some(b) = it.get("b") {
                    self.scan(b)?;
                }
                self.heap.set_budget(&s, it.get("b"))?;
            }
            "dep" | "q" => {
                let name = match it.get("c").and_then(|x| x.text()) {
                    Some(n) => n,
                    None => return Err("program: item without c".to_string()),
                };
                let empty: Vec<J> = Vec::new();
                let args: &Vec<J> = match it.get("a") {
                    Some(J::Arr(v)) => v,
                    _ => &empty,
                };
                for x in args {
                    self.scan(x)?;
                }
                // The trace shows the host objects, not the program's snapshot; built only when it can be recorded.
                let a = if args.is_empty() || (!self.tracing && self.throws.is_empty()) {
                    None
                } else {
                    Some(J::Arr(args.iter().map(|x| self.heap.host_view(x)).collect()))
                };
                let qi = if k == "q" {
                    match qidx(&name) {
                        Some(i) => Some(i),
                        None => return Err(format!("program: {} is not a query dependency", name)),
                    }
                } else {
                    None
                };
                if self.dep_call(&name, a.as_ref()) {
                    return Ok(());
                }
                match qi {
                    None => self.push_trace(&name, a, None),
                    Some(i) => {
                        let reply = match self.queues[i].pop_front() {
                            Some(r) => r,
                            None => {
                                if i == 0 {
                                    V::Arr(Vec::new())
                                } else {
                                    V::Bool(false)
                                }
                            }
                        };
                        if self.tracing {
                            let e = self.heap.enc(&reply);
                            self.push_trace(&name, a, Some(e));
                        }
                    }
                }
            }
            "tset" => {
                let key = json_text(it.get("key").unwrap_or(&JNULL));
                let delay = self.dec_opt(it.get("delay"))?;
                let unref = matches!(it.get("unref"), Some(J::Bool(true)));
                self.tset_v(key, delay, unref);
            }
            "tclear" => {
                let key = match nn(it.get("key")) {
                    Some(kj) => Some(json_text(kj)),
                    None => None,
                };
                self.tclear_k(key.as_deref());
            }
            "tthrow" => {
                let o = it.get("o").cloned().unwrap_or(J::Null);
                self.scan(&o)?;
                self.push_trace("timer.throw", None, Some(o));
            }
            other => return Err(format!("program: unknown item {}", other)),
        }
        Ok(())
    }

    // ---- timers -----------------------------------------------------------------------------------------------------

    fn tset_v(&mut self, key: String, delay: V, unref: bool) {
        let tok = self.timers.len() as u64 + 1;
        let effective = match &delay {
            V::Num(d) if *d >= 1.0 && *d <= 2147483647.0 => *d,
            _ => 1.0,
        };
        let due = self.now_f() + effective;
        if self.tracing {
            let de = self.heap.enc(&delay);
            let a = enc_obj(vec![("token", J::s(&format!("t{}", tok))), ("delay", de), ("due", enc_num(due))]);
            self.timers.push(Timer { tok, key: key.clone(), delay, due, i: self.cur_i.clone() });
            self.active.insert((okey(due), tok));
            self.key_tok.insert(key, tok);
            self.push_trace("timer.set", Some(a), None);
            if unref && self.timer_has_unref {
                self.push_trace("timer.unref", Some(enc_obj(vec![("token", J::s(&format!("t{}", tok)))])), None);
            }
        } else {
            self.timers.push(Timer { tok, key: key.clone(), delay, due, i: self.cur_i.clone() });
            self.active.insert((okey(due), tok));
            self.key_tok.insert(key, tok);
        }
    }

    fn tclear_k(&mut self, key: Option<&str>) {
        let tok = match key {
            Some(k) => self.key_tok.get(k).copied(),
            None => None,
        };
        if let Some(t) = tok {
            let due = self.timers[(t - 1) as usize].due;
            self.active.remove(&(okey(due), t));
        }
        if self.tracing {
            let tj = match tok {
                Some(t) => J::s(&format!("t{}", t)),
                None => J::Null,
            };
            self.push_trace("timer.clear", Some(enc_obj(vec![("token", tj)])), None);
        }
    }

    // perf entry points (the key is a plain string, converted like a program string key)
    pub fn tset(&mut self, key: &str, delay: f64, unref: bool) {
        self.tset_v(json_text(&J::s(key)), V::Num(delay), unref);
    }

    pub fn tclear(&mut self, key: &str) {
        let k = json_text(&J::s(key));
        self.tclear_k(Some(&k));
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    // Move the clock forward by ms, firing due timers in (due, registration) order; now = due before each fire.
    pub fn advance_by(&mut self, ms: f64) -> Result<(), String> {
        let target = self.now_f() + ms;
        loop {
            let first = match self.active.iter().next() {
                Some(p) => *p,
                None => break,
            };
            let idx = (first.1 - 1) as usize;
            let due = self.timers[idx].due;
            if !(due <= target) {
                break;
            }
            self.active.remove(&first);
            self.now = V::Num(due);
            self.run_timer(idx)?;
        }
        self.now = V::Num(target);
        Ok(())
    }

    fn token_idx(&self, tok: &J) -> Option<usize> {
        let t = tok.text()?;
        let rest = t.strip_prefix('t')?;
        let n: usize = rest.parse().ok()?;
        if n >= 1 && n <= self.timers.len() && format!("{}", n) == rest {
            Some(n - 1)
        } else {
            None
        }
    }

    fn deliver_timer(&mut self, tok: &J, captured: bool) -> Result<(), String> {
        let idx = match self.token_idx(tok) {
            Some(i) => i,
            None => return Err(format!("deliverTimer: unknown token {}", json_text(tok))),
        };
        let key = (okey(self.timers[idx].due), self.timers[idx].tok);
        if self.active.remove(&key) {
            return self.run_timer(idx);
        }
        if !captured {
            return Err(format!("deliverTimer: token {} is not active (cleared or fired) and captured is not true", plain(tok)));
        }
        self.run_timer(idx)
    }

    fn run_timer(&mut self, idx: usize) -> Result<(), String> {
        let saved_r = std::mem::replace(&mut self.cur_r, J::Null);
        let saved_i = std::mem::replace(&mut self.cur_i, self.timers[idx].i.clone());
        if self.tracing {
            let tok = self.timers[idx].tok;
            self.push_trace("timer.fire", Some(enc_obj(vec![("token", J::s(&format!("t{}", tok)))])), None);
        }
        let res = self.fire_segment(idx);
        self.cur_r = saved_r;
        self.cur_i = saved_i;
        self.now_override.clear();
        res?;
        match self.deferred.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn fire_segment(&mut self, idx: usize) -> Result<(), String> {
        let segs = self.next_seg()?;
        let seg = &segs[self.pos];
        let kind_ok = seg.get("kind").and_then(|x| x.text()).as_deref() == Some("fire");
        let key_ok = match seg.get("key") {
            Some(kj) => json_text(kj) == self.timers[idx].key,
            None => false,
        };
        if !kind_ok || !key_ok {
            return Err(format!("program: expected fire segment for timer t{}", self.timers[idx].tok));
        }
        self.pos += 1;
        if let Some(J::Arr(items)) = seg.get("items") {
            self.exec_items(items)?;
        }
        Ok(())
    }

    // ---- observation ------------------------------------------------------------------------------------------------

    fn observation(&mut self, id: &J, domain: Option<&J>, fin: Option<&J>) -> J {
        if let Some(J::Arr(bs)) = fin.and_then(|f| f.get("bsets")) {
            for b in bs {
                if let Err(e) = self.exec_item(b) {
                    self.violations.push(format!("driver: final state: {}", e));
                }
            }
        }
        let sessions = self.encode_all_sessions();
        let mut timers: Vec<J> = Vec::new();
        for &(_, tok) in self.active.iter() {
            let t = &self.timers[(tok - 1) as usize];
            let de = self.heap.enc(&t.delay);
            timers.push(J::Obj(vec![
                (ku("token"), J::s(&format!("t{}", t.tok))),
                (ku("delay"), de),
                (ku("due"), enc_num(t.due)),
                (ku("i"), opt_s(&t.i)),
            ]));
        }
        let clocks = fin.and_then(|f| f.get("clocks")).cloned().unwrap_or(J::Obj(Vec::new()));
        let tool_seen = fin.and_then(|f| f.get("toolSeen")).cloned().unwrap_or(J::Obj(Vec::new()));
        let driver = if self.violations.is_empty() {
            J::Null
        } else {
            J::Obj(vec![(ku("violations"), J::Arr(self.violations.iter().map(|v| J::s(v)).collect()))])
        };
        let mut o = vec![
            (ku("id"), id.clone()),
            (ku("requests"), J::Arr(std::mem::take(&mut self.requests))),
            (ku("trace"), J::Arr(std::mem::take(&mut self.trace))),
            (ku("finalSessions"), J::Arr(sessions)),
            (ku("finalClocks"), clocks),
            (ku("finalToolSeen"), tool_seen),
            (ku("activeTimers"), J::Arr(timers)),
            (ku("driver"), driver),
            (ku("excluded"), J::Arr(self.excluded.iter().map(|v| J::s(v)).collect())),
        ];
        if let Some(d) = domain {
            o.push((ku("domain"), d.clone()));
        }
        J::Obj(o)
    }
}

// ---- driver: scenarios JSONL on stdin -> observations JSONL on stdout --------------------------------------------------

fn bad_line(msg: &str) -> J {
    J::Obj(vec![
        (ku("id"), J::Null),
        (ku("requests"), J::Arr(Vec::new())),
        (ku("trace"), J::Arr(Vec::new())),
        (ku("finalSessions"), J::Arr(Vec::new())),
        (ku("finalClocks"), J::Obj(Vec::new())),
        (ku("finalToolSeen"), J::Obj(Vec::new())),
        (ku("activeTimers"), J::Arr(Vec::new())),
        (ku("driver"), J::Obj(vec![(ku("violations"), J::Arr(vec![J::s(msg)]))])),
        (ku("excluded"), J::Arr(Vec::new())),
    ])
}

pub fn run_one(sc: &J, progs: &HashMap<String, J>) -> J {
    let id = sc.get("id").cloned().unwrap_or(J::Null);
    let prog = sc.get("id").and_then(|x| x.text()).and_then(|t| progs.get(&t));
    let mut w = World::new(true);
    if let Some(p) = prog {
        let segs = match p.get("segs") {
            Some(J::Arr(a)) => a.clone(),
            _ => Vec::new(),
        };
        w.set_program(segs);
    }
    if let Err(e) = w.run_inner(sc) {
        w.violations.push(format!("driver: {}", e));
    }
    w.observation(&id, sc.get("domain"), prog.and_then(|p| p.get("final")))
}

pub fn run_q(prog_path: &str) -> Result<(), String> {
    let text = match std::fs::read_to_string(prog_path) {
        Ok(t) => t,
        Err(e) => return Err(format!("cannot read program {}: {}", prog_path, e)),
    };
    let mut progs: HashMap<String, J> = HashMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let j = match parse_json(line) {
            Ok(j) => j,
            Err(e) => return Err(format!("program line {}: {}", n + 1, e)),
        };
        if let Some(id) = j.get("id").and_then(|x| x.text()) {
            progs.insert(id, j);
        }
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => return Err(format!("stdin: {}", e)),
        };
        if line.trim().is_empty() {
            continue;
        }
        let obs = match parse_json(&line) {
            Err(e) => bad_line(&format!("bad scenario line: {}", e)),
            Ok(sc) => {
                if matches!(sc, J::Obj(_)) {
                    run_one(&sc, &progs)
                } else {
                    bad_line("bad scenario line: not an object")
                }
            }
        };
        let mut s = String::new();
        write_json(&obs, &mut s);
        s.push('\n');
        if out.write_all(s.as_bytes()).is_err() {
            return Err("stdout write failed".to_string());
        }
    }
    if out.flush().is_err() {
        return Err("stdout flush failed".to_string());
    }
    Ok(())
}

// ---- resident state (M4 protocol) ----------------------------------------------------------------------------------------
// The host owns the state buffer but never reads or writes its words: it allocates, calls the entries, and on Grow
// (status 1, N = out[0]) allocates a zeroed buffer of max(N, 2*len), calls grow, drops the old buffer and retries the
// same op (at most 3 retries). The slice types are Vec references so that the generated entries (whichever of
// `&mut [i64]` / `&mut Vec<i64>` the Pruv compiler emits for `inout [i64]`) accept the arguments unchanged.

pub trait Core {
    fn init(&self, st: &mut Vec<i64>) -> i64;
    fn grow(&self, prev: &Vec<i64>, next: &mut Vec<i64>) -> i64;
    fn op(&self, st: &mut Vec<i64>, inp: &[i64], out: &mut Vec<i64>) -> i64;
}

pub struct ResidentState<C: Core> {
    core: C,
    st: Vec<i64>,
    out: Vec<i64>,
    pub grows: u64,
    pub grow_ns: u128,
    pub outsmall: u64,
    // Poison flag (same semantics as pruv_raw::PruvStore): checked once at the start of each public call, set before
    // any core call or allocation, cleared only when the call returns normally (Ok or Err). A panic inside a call
    // (core or allocation) that is caught by the caller leaves it set.
    poisoned: bool,
}

// Err value `step` returns on a poisoned state (no state is touched). Distinct from every other step error.
pub const POISONED_ERR: &str = "resident state poisoned";

impl<C: Core> ResidentState<C> {
    pub fn new(core: C, initial_len: usize) -> Result<ResidentState<C>, String> {
        let mut st = vec![0i64; initial_len];
        let r = core.init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            let r2 = core.init(&mut st);
            if r2 != 0 {
                return Err(format!("init failed twice (status {})", r2));
            }
        } else if r < 0 {
            return Err(format!("init returned {}", r));
        }
        Ok(ResidentState { core, st, out: vec![0i64; 4096], grows: 0, grow_ns: 0, outsmall: 0, poisoned: false })
    }

    pub fn st_len(&self) -> usize {
        self.st.len()
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    // One op. Ok(result words) for status 0 (out[0] = k words follow). On a poisoned state: Err(POISONED_ERR), nothing touched.
    pub fn step(&mut self, inp: &[i64]) -> Result<&[i64], String> {
        if self.poisoned {
            return Err(POISONED_ERR.to_string());
        }
        self.poisoned = true;
        let r = self.step_inner(inp);
        self.poisoned = false;
        let k = r?;
        let hi = (1 + k).min(self.out.len());
        Ok(&self.out[1..hi])
    }

    // The op loop; Ok(k) = status 0 with k result words at out[1..].
    fn step_inner(&mut self, inp: &[i64]) -> Result<usize, String> {
        let mut retries = 0u32;
        let mut doublings = 0u32;
        loop {
            let s = self.core.op(&mut self.st, inp, &mut self.out);
            match s {
                0 => {
                    return Ok(self.out[0].max(0) as usize);
                }
                1 => {
                    if retries >= 3 {
                        return Err("grow retry limit reached".to_string());
                    }
                    retries += 1;
                    let t = std::time::Instant::now();
                    let n = self.out[0].max(0) as usize;
                    let want = std::cmp::max(n, 2 * self.st.len());
                    let mut fresh = vec![0i64; want];
                    let g = self.core.grow(&self.st, &mut fresh);
                    if g != 0 {
                        return Err(format!("grow failed (status {})", g));
                    }
                    self.st = fresh;
                    self.grows += 1;
                    self.grow_ns += t.elapsed().as_nanos();
                }
                2 => return Err("invalid op".to_string()),
                3 => {
                    doublings += 1;
                    if doublings > 24 {
                        return Err("out buffer too small".to_string());
                    }
                    self.outsmall += 1;
                    let l = self.out.len().max(1) * 2;
                    self.out.resize(l, 0);
                }
                other => return Err(format!("unknown status {}", other)),
            }
        }
    }
}

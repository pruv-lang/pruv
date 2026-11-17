// L3-9 request host (ABI.md 1, 3, 9): drives req_begin/req_step over a scenario and prints oracle-shape observations.
// Included by host/src/lib.rs as the root of crate `req_host`. Only plain `//` comments (include! forbids inner doc comments).
//
// The host owns transport, scripted leaf effects, identity and observation recording. It decides no handler outcome:
// source projection, first match and error choice are Pruv's. M0-M1 scope: op 1 (sources) end to end plus the
// sourceUnknown / accounts-throw paths of ops 2 and 3. The scheduler (FIFO microtasks, timers ordered by (due, id)) and
// the world/frame bookkeeping are in place as the skeleton the later milestones extend; other events are refused.
//
// Stdin: one scenario JSON per line. Stdout: one observation per line:
//   {"id","requests":[{id,terminal,value}],"trace":[..],"createdSessions":[],"openingEvents":[],"finalOpening":{inst:[]},
//    "activeTimers":[],"activeListeners":{},"driver":null}   or   {"id":..,"error":"case: .."}

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{self, BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::value::RawValue;
use serde_json::Value;

pub use eh::{json_str, json_units, render_input_json, Leaf};

// The generated module as seen by the host; harness.rs implements it with the `sol` functions.
pub trait Req {
    fn begin(&self, world: &[i64], input: &[i64]) -> Vec<i64>;
    fn step(&self, world: &[i64], prev: &[i64], completion: &[i64]) -> Vec<i64>;
}

const M_REQ: i64 = 1381057841;
const M_WORLD: i64 = 1381062449;
const M_BUNDLE: i64 = 1381057073;
const HDR: usize = 32;

// ---------------------------------------------------------------------------------------------------------------
// request packet envelope (ABI.md 3.2)

#[derive(Clone)]
struct Rp {
    w: Vec<i64>,
    secs: Vec<Vec<i64>>,
}

fn parse_rp(w: Vec<i64>) -> Result<Rp, String> {
    if w.len() < HDR + 6 || w[0] != M_REQ || w[1] != 1 {
        return Err("internal: request packet header is invalid".to_string());
    }
    let nsec = w[24] as usize;
    if w[25] as usize != w.len() || nsec < 6 || HDR + nsec > w.len() {
        return Err("internal: request packet length fields are invalid".to_string());
    }
    let mut at = HDR + nsec;
    let mut secs = Vec::new();
    for i in 0..nsec {
        let l = w[HDR + i];
        if l < 0 || at + l as usize > w.len() {
            return Err("internal: request packet section length is invalid".to_string());
        }
        secs.push(w[at..at + l as usize].to_vec());
        at += l as usize;
    }
    if at != w.len() {
        return Err("internal: request packet sections do not cover the payload".to_string());
    }
    Ok(Rp { w, secs })
}

fn build_rp(head: &[i64], secs: &[Vec<i64>]) -> Vec<i64> {
    let mut w: Vec<i64> = head.to_vec();
    w.resize(HDR, 0);
    w[0] = M_REQ;
    w[1] = 1;
    w[24] = secs.len() as i64;
    for s in secs {
        w.push(s.len() as i64);
    }
    for s in secs {
        w.extend_from_slice(s);
    }
    w[25] = w.len() as i64;
    w
}

// generation (world word 2) of a bundle's world, -1 when the bundle is malformed
fn bundle_gen(b: &[i64]) -> i64 {
    if b.len() < 7 || b[0] != M_BUNDLE || b[2] < 3 { -1 } else { b[6] }
}

// A core result the driver must not adopt (ABI.md 10): a malformed bundle, a world that is not exactly one generation past
// the driver's latest honest world, or a done record of core InvalidPacket. The trusted driver keeps its last honest state.
fn rejected(b: &[i64], old_gen: i64) -> bool {
    if bundle_gen(b) != old_gen + 1 {
        return true;
    }
    match split_bundle(b.to_vec()) {
        Err(_) => true,
        Ok((_, rp)) => rp.w[2] == 3 && rp.secs.get(1).map_or(false, |r| r.len() >= 2 && r[0] == 5 && r[1] == 0),
    }
}

// bundle: [M_BUNDLE, 1, world_words, packet_words, world..., packet...]
fn split_bundle(b: Vec<i64>) -> Result<(Vec<i64>, Rp), String> {
    if b.len() < 4 || b[0] != M_BUNDLE || b[1] != 1 {
        return Err("internal: bundle header is invalid".to_string());
    }
    let (ww, pw) = (b[2], b[3]);
    if ww < 0 || pw < 0 || b.len() as i64 != 4 + ww + pw {
        return Err("internal: bundle length fields are invalid".to_string());
    }
    let ww = ww as usize;
    let world = b[4..4 + ww].to_vec();
    let rp = parse_rp(b[4 + ww..].to_vec())?;
    Ok((world, rp))
}

// ---------------------------------------------------------------------------------------------------------------
// scheduler skeleton (ABI.md 7): FIFO microtasks and timers ordered by (due, id); M1 only enqueues request resumptions

enum Task {
    Resume { req: usize, secs: Vec<Vec<i64>> },
}

struct Sched {
    micro: VecDeque<Task>,
    timers: BTreeMap<(i64, i64), Task>,
    next_timer: i64,
    now: i64,
}

impl Sched {
    fn new() -> Sched {
        Sched { micro: VecDeque::new(), timers: BTreeMap::new(), next_timer: 1, now: 0 }
    }
    fn micro(&mut self, t: Task) {
        self.micro.push_back(t);
    }
    #[allow(dead_code)]
    fn timer(&mut self, due: i64, t: Task) -> i64 {
        let id = self.next_timer;
        self.next_timer += 1;
        self.timers.insert((due, id), t);
        id
    }
    fn next_micro(&mut self) -> Option<Task> {
        self.micro.pop_front()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// scenario run

struct Slot {
    name: String,
    inst: String,
    inst_id: i64,
    rid: i64,
    op: &'static str,
    prev: Option<Rp>,
    done: Option<(String, String)>,
}

struct Case<'a, S: Req> {
    sol: &'a S,
    leaf: Leaf,
    world: Vec<i64>,
    insts: Vec<String>,
    reqs: Vec<Slot>,
    by_name: HashMap<String, usize>,
    trace: Vec<String>,
    sched: Sched,
    // ABI.md 5.5 storage: immutable blocks (owner = (instance id, request id), previous block of the chain, words)
    blocks: Vec<(i64, i64, i64, Vec<i64>)>,
}

fn initial_world(n: usize) -> Vec<i64> {
    let mut w = vec![M_WORLD, 1, 0, 1, 1, n as i64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for i in 0..n {
        w.push(i as i64 + 1);
        w.push(0);
    }
    w
}

fn op_of(name: &str) -> Result<(i64, &'static str), String> {
    match name {
        "sources" => Ok((1, "sources")),
        "list" => Ok((2, "list")),
        "open" => Ok((3, "open")),
        o => Err(format!("case: unknown op {}", o)),
    }
}

fn tline(c: &str, r: &str, i: &str, a: Option<String>) -> String {
    let mut s = format!("{{\"c\":{},\"r\":{},\"i\":{}", json_str(c), json_str(r), json_str(i));
    if let Some(a) = a {
        s.push_str(&format!(",\"a\":{}", a));
    }
    s.push('}');
    s
}

impl<'a, S: Req> Case<'a, S> {
    fn start(&mut self, ev: &HashMap<String, Box<RawValue>>, world: &Value) -> Result<(), String> {
        let field = |k: &str| -> Option<Value> { ev.get(k).and_then(|r| serde_json::from_str::<Value>(r.get()).ok()) };
        let name = match field("req") {
            Some(Value::String(s)) => s,
            _ => return Err("case: start needs a req name".to_string()),
        };
        if self.by_name.contains_key(&name) {
            return Err(format!("case: duplicate request name {}", name));
        }
        let inst = match field("i") {
            Some(Value::String(s)) => s,
            _ => self.insts[0].clone(),
        };
        let inst_id = match self.insts.iter().position(|x| *x == inst) {
            Some(p) => p as i64 + 1,
            None => return Err(format!("case: unknown instance {}", inst)),
        };
        let opname = match field("op") {
            Some(Value::String(s)) => s,
            _ => return Err("case: start needs an op".to_string()),
        };
        let (op, opn) = op_of(&opname)?;
        let input_v = field("input");
        let mut a = format!("{{\"op\":{}", json_str(opn));
        if let Some(raw) = ev.get("input") {
            a.push_str(&format!(",\"input\":{}", render_input_json(raw.get())?));
        }
        a.push('}');
        self.trace.push(tline("request.start", &name, &inst, Some(a)));
        let rid = self.reqs.len() as i64 + 1;
        let idx = self.reqs.len();
        let from_app = match ev.get("fromAppAction") {
            Some(r) => serde_json::from_str::<Value>(r.get()).ok(),
            None => None,
        };
        let cancel = match field("cancel") {
            Some(Value::String(s)) => Some(s),
            _ => None,
        };
        self.leaf.set_req(idx as i64 + 1, &name, &inst, inst_id, from_app, cancel);
        self.reqs.push(Slot { name: name.clone(), inst, inst_id, rid, op: opn, prev: None, done: None });
        self.by_name.insert(name, idx);
        let (pool, rec) = self.leaf.encode_input(op, world, input_v.as_ref())?;
        let mut head = vec![0i64; HDR];
        head[3] = op;
        head[4] = inst_id;
        head[5] = rid;
        let empty: Vec<i64> = Vec::new();
        let input = build_rp(&head, &[pool, rec, empty.clone(), empty.clone(), empty.clone(), empty]);
        let bundle = self.sol.begin(&self.world, &input);
        if rejected(&bundle, self.world.get(2).copied().unwrap_or(0)) {
            // an input the core refuses is not admitted: no world change and nothing to clean up
            self.reject_line(idx);
            self.end_rejected(idx);
            return self.drain();
        }
        self.accept(idx, bundle)?;
        self.drain()
    }

    // ABI.md 5.5: one storage operation of request (inst, rid). The reply record is [0, ...] or [1] for a refused operation
    // (a handle of another request, a range outside the block). Storage is not observable: no trace line, no call count.
    fn serve_storage(&mut self, inst: i64, rid: i64, op: &[i64]) -> Vec<i64> {
        let own = |b: &(i64, i64, i64, Vec<i64>)| b.0 == inst && b.1 == rid;
        let get = |h: i64, bl: &Vec<(i64, i64, i64, Vec<i64>)>| -> Option<usize> {
            if h >= 1 && (h as usize) <= bl.len() && own(&bl[h as usize - 1]) { Some(h as usize - 1) } else { None }
        };
        match op.first().copied() {
            Some(60) if op.len() >= 2 => {
                let prev = op[1];
                if prev != 0 && get(prev, &self.blocks).is_none() {
                    return vec![1];
                }
                self.blocks.push((inst, rid, prev, op[2..].to_vec()));
                vec![0, self.blocks.len() as i64]
            }
            Some(61) if op.len() == 4 => {
                let (off, cnt) = (op[2], op[3]);
                match get(op[1], &self.blocks) {
                    Some(k) if off >= 0 && cnt >= 0 && (off + cnt) as usize <= self.blocks[k].3.len() => {
                        let mut r = vec![0];
                        r.extend_from_slice(&self.blocks[k].3[off as usize..(off + cnt) as usize]);
                        r
                    }
                    _ => vec![1],
                }
            }
            Some(65) if op.len() == 2 => {
                let mut chain: Vec<usize> = Vec::new();
                let mut h = op[1];
                while h != 0 {
                    match get(h, &self.blocks) {
                        Some(k) => {
                            chain.push(k);
                            h = self.blocks[k].2;
                        }
                        None => return vec![1],
                    }
                }
                let mut r = vec![0, chain.len() as i64];
                for &k in chain.iter().rev() {
                    r.push(self.blocks[k].3.len() as i64);
                    r.extend_from_slice(&self.blocks[k].3);
                }
                r
            }
            _ => vec![1],
        }
    }

    // Storage awaits (ck 4) are served at once and stepped in place (no microtask: storage is part of the core's own turn).
    // Returns the first bundle that is not a storage await, or None when the request ended rejected.
    fn run_storage(&mut self, idx: usize, mut bundle: Vec<i64>) -> Result<Option<Vec<i64>>, String> {
        loop {
            let (world, rp) = split_bundle(bundle.clone())?;
            if rp.w[2] != 1 || rp.w[10] != 4 {
                return Ok(Some(bundle));
            }
            self.world = world;
            self.leaf.ww = self.world.clone();
            if rp.secs.len() < 6 {
                return Err("internal: storage await without its operation".to_string());
            }
            let reply = self.serve_storage(rp.w[4], rp.w[5], &rp.secs[5]);
            let empty: Vec<i64> = Vec::new();
            let secs = vec![empty.clone(), reply, empty.clone(), empty.clone(), empty.clone(), empty];
            let mut head = rp.w[..HDR].to_vec();
            head[2] = 2;
            let comp = build_rp(&head, &secs);
            let old_gen = self.world.get(2).copied().unwrap_or(0);
            let mut next = self.sol.step(&self.world, &rp.w, &comp);
            if rejected(&next, old_gen) {
                self.reject_line(idx);
                next = self.sol.step(&self.world, &rp.w, &comp);
                if rejected(&next, old_gen) {
                    self.reject_line(idx);
                    self.end_rejected(idx);
                    return Ok(None);
                }
            }
            bundle = next;
        }
    }

    // Takes a bundle: stores the new world, and either records the terminal or serves the child await and queues the step.
    fn accept(&mut self, idx: usize, bundle: Vec<i64>) -> Result<(), String> {
        let bundle = match self.run_storage(idx, bundle)? {
            Some(b) => b,
            None => return Ok(()),
        };
        let (world, rp) = split_bundle(bundle)?;
        self.world = world;
        self.leaf.ww = self.world.clone();
        match rp.w[2] {
            3 => {
                let (terminal, value) = self.leaf.render_done(rp.secs.clone())?;
                let evs = rp.secs.get(5).cloned().unwrap_or_default();
                self.finish(idx, terminal, value, &evs)?;
                Ok(())
            }
            1 => {
                self.leaf.cur = idx as i64 + 1;
                if rp.secs.len() < 6 || (rp.w[10] != 2 && rp.secs[5].is_empty()) {
                    return Err("internal: await packet has no child entry packet".to_string());
                }
                let empty: Vec<i64> = Vec::new();
                self.leaf.now = self.sched.now;
                let secs = if rp.w[10] == 3 {
                    self.flush_prims(idx);
                    let (cp, cr, s5) = self.leaf.serve_call(&rp.secs[0], &rp.secs[1], &rp.secs[4], &rp.secs[3], &rp.secs[5], self.sched.now)?;
                    self.flush_events(idx);
                    vec![cp, cr, empty.clone(), empty.clone(), empty, s5]
                } else if rp.w[10] == 2 {
                    // checkFolder: the folder is the request record's slot 27 (string slot over the request pool)
                    let (pool, rec) = (&rp.secs[0], &rp.secs[1]);
                    if rec.len() < 30 || rec[27] != 6 || (rec[28] + rec[29]) as usize > pool.len() {
                        return Err("internal: checkFolder await without a string folder slot".to_string());
                    }
                    let units: Vec<u16> = pool[rec[28] as usize..(rec[28] + rec[29]) as usize].iter().map(|&x| x as u16).collect();
                    self.flush_prims(idx);
                    let (cp, cr, o) = self.leaf.serve_folder(&units)?;
                    let (n, i) = (self.reqs[idx].name.clone(), self.reqs[idx].inst.clone());
                    let a = format!("[{}]", json_units(&units));
                    self.trace.push(format!("{{\"c\":\"checkFolder\",\"r\":{},\"i\":{},\"a\":{},\"o\":{}}}", json_str(&n), json_str(&i), a, o));
                    vec![cp, cr, empty.clone(), empty.clone(), empty.clone(), empty]
                } else {
                    let comp = self.leaf.serve_child(&rp.secs[5])?;
                    self.flush_prims(idx);
                    self.flush_events(idx);
                    vec![empty.clone(), vec![0], empty.clone(), empty.clone(), empty, comp]
                };
                self.reqs[idx].prev = Some(rp);
                let reply = (secs[0].clone(), secs[1].clone(), secs[5].clone());
                if !self.leaf.arm(reply) {
                    self.sched.micro(Task::Resume { req: idx, secs });
                }
                Ok(())
            }
            k => Err(format!("internal: request packet kind {} is neither await nor done", k)),
        }
    }

    fn flush_events(&mut self, idx: usize) {
        for e in self.leaf.take_events() {
            let who = if e.who == 0 { idx as i64 + 1 } else { e.who };
            let mut l = if who < 0 {
                format!("{{\"c\":{},\"r\":null,\"i\":null,\"a\":{}", json_str(&e.c), e.a)
            } else {
                let k = (who - 1) as usize;
                format!("{{\"c\":{},\"r\":{},\"i\":{},\"a\":{}", json_str(&e.c), json_str(&self.reqs[k].name), json_str(&self.reqs[k].inst), e.a)
            };
            if e.pid != 0 {
                l.push_str(&format!(",\"o\":__P{}__", e.pid));
            } else if let Some(o) = e.o {
                l.push_str(&format!(",\"o\":{}", o));
            }
            l.push('}');
            self.trace.push(l);
        }
    }

    // Queues the replies of the requests a settle, abort or timer woke.
    fn wake(&mut self, idx: usize, rs: Vec<(i64, (Vec<i64>, Vec<i64>, Vec<i64>))>) {
        self.flush_events(idx);
        let empty: Vec<i64> = Vec::new();
        for (owner, (cp, cr, s5)) in rs {
            self.sched.micro(Task::Resume { req: owner as usize - 1, secs: vec![cp, cr, empty.clone(), empty.clone(), empty.clone(), s5] });
        }
    }

    fn flush_prims(&mut self, idx: usize) {
        let (n, i) = (self.reqs[idx].name.clone(), self.reqs[idx].inst.clone());
        for t in self.leaf.take_trace() {
            self.trace.push(tline("prim", &n, &i, Some(t)));
        }
    }

    fn finish(&mut self, idx: usize, terminal: String, value: String, evs: &[i64]) -> Result<(), String> {
        self.flush_prims(idx);
        self.flush_events(idx);
        self.leaf.finish_cleanup(idx as i64 + 1, evs)?;
        self.flush_events(idx);
        let (n, i) = (self.reqs[idx].name.clone(), self.reqs[idx].inst.clone());
        self.trace.push(tline("request.done", &n, &i, Some(format!("{{\"terminal\":{}}}", json_str(&terminal)))));
        self.reqs[idx].prev = None;
        self.reqs[idx].done = Some((terminal, value));
        Ok(())
    }

    fn reject_line(&mut self, idx: usize) {
        let (n, i) = (self.reqs[idx].name.clone(), self.reqs[idx].inst.clone());
        self.trace.push(tline("request.reject", &n, &i, Some("{\"code\":\"InvalidPacket\"}".to_string())));
    }

    // A request whose packets the core keeps refusing ends as core InvalidPacket without adopting any world and without
    // cleanup commands (ABI.md 10: a rejection has no new external effects).
    fn end_rejected(&mut self, idx: usize) {
        self.flush_prims(idx);
        self.flush_events(idx);
        let (n, i) = (self.reqs[idx].name.clone(), self.reqs[idx].inst.clone());
        self.trace.push(tline("request.done", &n, &i, Some("{\"terminal\":\"core\"}".to_string())));
        self.reqs[idx].prev = None;
        self.reqs[idx].done = Some(("core".to_string(), "{\"code\":\"InvalidPacket\"}".to_string()));
    }

    fn drain(&mut self) -> Result<(), String> {
        while let Some(t) = self.sched.next_micro() {
            match t {
                Task::Resume { req, secs } => {
                    let prev = match self.reqs[req].prev.clone() {
                        Some(p) => p,
                        None => return Err("internal: resume of a request with no continuation".to_string()),
                    };
                    let mut head = prev.w[..HDR].to_vec();
                    head[2] = 2;
                    let comp = build_rp(&head, &secs);
                    let old_gen = self.world.get(2).copied().unwrap_or(0);
                    let mut bundle = self.sol.step(&self.world, &prev.w, &comp);
                    // A rejected step is not adopted: the driver keeps its latest honest world and continuation, records the
                    // rejection and resubmits the same honest triple once. A second rejection ends the request (no cleanup).
                    if rejected(&bundle, old_gen) {
                        self.reject_line(req);
                        bundle = self.sol.step(&self.world, &prev.w, &comp);
                        if rejected(&bundle, old_gen) {
                            self.reject_line(req);
                            self.end_rejected(req);
                            continue;
                        }
                    }
                    self.accept(req, bundle)?;
                }
            }
        }
        Ok(())
    }
}

fn ev_str(ev: &HashMap<String, Box<RawValue>>, k: &str) -> Result<String, String> {
    match ev.get(k).and_then(|r| serde_json::from_str::<String>(r.get()).ok()) {
        Some(s) => Ok(s),
        None => Err(format!("case: event needs a string {}", k)),
    }
}

fn run_case<S: Req>(sol: &S, line: &str) -> Result<String, String> {
    let top: HashMap<String, Box<RawValue>> = serde_json::from_str(line).map_err(|e| format!("case: bad JSON: {}", e))?;
    let get = |k: &str| top.get(k).and_then(|r| serde_json::from_str::<Value>(r.get()).ok());
    let id = match top.get("id") {
        Some(r) => r.get().to_string(),
        None => "null".to_string(),
    };
    let world = get("world").unwrap_or(Value::Object(serde_json::Map::new()));
    let insts: Vec<String> = match get("instances") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
        _ => vec!["A".to_string()],
    };
    if insts.is_empty() {
        return Err("case: instances is empty".to_string());
    }
    let leaf = Leaf::new(&world, top.get("world").map(|r| r.get()).unwrap_or("{}"))?;
    let mut c = Case {
        sol,
        leaf,
        world: initial_world(insts.len()),
        insts: insts.clone(),
        reqs: Vec::new(),
        by_name: HashMap::new(),
        trace: Vec::new(),
        sched: Sched::new(),
        blocks: Vec::new(),
    };
    let script: Vec<Box<RawValue>> = match top.get("script") {
        Some(r) => serde_json::from_str(r.get()).map_err(|e| format!("case: bad script: {}", e))?,
        None => Vec::new(),
    };
    for raw in &script {
        let ev: HashMap<String, Box<RawValue>> = serde_json::from_str(raw.get()).map_err(|e| format!("case: bad event: {}", e))?;
        if ev.get("ev").map(|r| r.get()) == Some("\"settle\"") {
            if let (Some(l), Some(k)) = (ev.get("label").and_then(|r| serde_json::from_str::<String>(r.get()).ok()), ["ok", "reject"].iter().find(|k| ev.contains_key(**k))) {
                let js = format!("{{\"{}\":{}}}", k, ev.get(*k).unwrap().get());
                c.leaf.set_override(&l, &js)?;
            }
        }
    }
    for raw in &script {
        let ev: HashMap<String, Box<RawValue>> = serde_json::from_str(raw.get()).map_err(|e| format!("case: bad event: {}", e))?;
        let kind = match ev.get("ev").and_then(|r| serde_json::from_str::<String>(r.get()).ok()) {
            Some(k) => k,
            None => return Err("case: event without ev".to_string()),
        };
        match kind.as_str() {
            "start" => c.start(&ev, &world)?,
            "handle" => {
                let n = ev_str(&ev, "name")?;
                c.leaf.handle_new(&n);
            }
            "cancel" => {
                let n = ev_str(&ev, "handle")?;
                let rr = match ev.get("reason").and_then(|r| serde_json::from_str::<Value>(r.get()).ok()) {
                    Some(v) => v.get("ref").and_then(|x| x.as_str()).map(|x| x.to_string()),
                    None => None,
                };
                c.leaf.now = c.sched.now;
                let rs = c.leaf.handle_cancel(&n, rr)?;
                c.wake(0, rs);
                c.drain()?;
            }
            "mutate" => {
                let op = ev_str(&ev, "op")?;
                let sv = |k: &str| -> Result<Option<Value>, String> {
                    match ev.get(k) {
                        Some(r) => Ok(Some(serde_json::from_str::<Value>(r.get()).map_err(|e| e.to_string())?)),
                        None => Ok(None),
                    }
                };
                let idv = sv("id")?;
                match op.as_str() {
                    "add" => match sv("session")? {
                        Some(v) => c.leaf.mutate_add(v),
                        None => return Err("case: mutate add needs a session".to_string()),
                    },
                    "remove" => c.leaf.mutate_remove(&idv.clone().unwrap_or(Value::Null))?,
                    "replace" => match sv("session")? {
                        Some(v) => c.leaf.mutate_replace(idv.as_ref(), v)?,
                        None => return Err("case: mutate replace needs a session".to_string()),
                    },
                    o => return Err(format!("case: mutate op {} is not supported yet", o)),
                }
                let id = match (&idv, op.as_str()) {
                    (Some(v), "remove" | "replace") => v.to_string(),
                    _ => "null".to_string(),
                };
                c.trace.push(format!("{{\"c\":\"mutate\",\"r\":null,\"i\":null,\"a\":{{\"op\":{},\"id\":{}}}}}", json_str(&op), id));
            }
            "settle" => {
                let l = ev_str(&ev, "label")?;
                // A hold that was never made because Pruv ended its request at the 32-active-request bound
                // (core InputTooLarge) has nothing to settle; the scenario names it as if the request had run.
                let bound_hit = c.reqs.iter().any(|r| matches!(&r.done, Some((t, v)) if t == "core" && v.contains("InputTooLarge")));
                if bound_hit && !c.leaf.has_hold(&l) {
                    continue;
                }
                c.leaf.now = c.sched.now;
                let rs = c.leaf.settle_label(&l)?;
                c.wake(0, rs);
                c.drain()?;
            }
            "advance" => {
                let ms = match ev.get("ms").and_then(|r| serde_json::from_str::<i64>(r.get()).ok()) {
                    Some(m) => m,
                    None => return Err("case: advance needs ms".to_string()),
                };
                c.trace.push(format!("{{\"c\":\"advance\",\"r\":null,\"i\":null,\"a\":{{\"ms\":{}}}}}", ms));
                let target = c.sched.now + ms;
                c.leaf.now = c.sched.now;
                while let Some(rs) = c.leaf.fire_next(target)? {
                    c.wake(0, rs);
                    c.drain()?;
                }
                c.sched.now = target;
                c.leaf.now = target;
            }
            k => return Err(format!("case: unsupported in M1: ev {}", k)),
        }
    }
    let mut reqs = Vec::new();
    for r in &c.reqs {
        let (t, v) = match &r.done {
            Some((t, v)) => (t.clone(), v.clone()),
            None => ("pending".to_string(), "null".to_string()),
        };
        reqs.push(format!("{{\"id\":{},\"terminal\":{},\"value\":{}}}", json_str(&r.name), json_str(&t), v));
    }
    let tail = c.leaf.observation_tail(&insts);
    let mut trace = c.trace.join(",");
    for (pid, o) in &c.leaf.patches {
        trace = trace.replace(&format!("__P{}__", pid), o);
    }
    while let Some(at) = trace.find("__P") {
        let end = trace[at + 3..].find("__").map_or(trace.len(), |e| at + 3 + e + 2);
        trace.replace_range(at..end, "{\"pending\":true}");
    }
    Ok(format!(
        "{{\"id\":{},\"requests\":[{}],\"trace\":[{}],{},\"driver\":null}}",
        id,
        reqs.join(","),
        trace,
        tail
    ))
}

fn run_one<S: Req>(sol: &S, line: &str) -> String {
    let id = serde_json::from_str::<HashMap<String, Box<RawValue>>>(line)
        .ok()
        .and_then(|m| m.get("id").map(|r| r.get().to_string()))
        .unwrap_or_else(|| "null".to_string());
    match catch_unwind(AssertUnwindSafe(|| run_case(sol, line))) {
        Ok(Ok(s)) => s,
        Ok(Err(m)) => format!("{{\"id\":{},\"error\":{}}}", id, json_str(&m)),
        Err(_) => format!("{{\"id\":{},\"error\":\"case: host panic\"}}", id),
    }
}

pub fn run<S: Req>(sol: &S) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let out = run_one(sol, &line);
        let mut o = stdout.lock();
        let _ = writeln!(o, "{}", out);
        let _ = o.flush();
    }
}

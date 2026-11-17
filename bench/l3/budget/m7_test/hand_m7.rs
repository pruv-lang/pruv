// Hand-written idiomatic Rust baseline for the M7 perf items: the same lifecycle work as bud_lifecycle.pruv +
// driver_m7.rs run_perf (session lookup by id, budget id check, attach / OPEN, CLOSE with the clock part, reopen,
// replaceForUserMessage with follows + carryHeld), on owned structs in a HashMap. Instants the module writes are kept
// as numbers (no ISO string), like the Pruv side; the other budget properties (card, halt, ...) are one opaque string,
// like the record's rest text. Effects go to a Vec the host drains, like the fx frame.
//   hand_m7 <S> <reps>   prints `item size ns_per_call` for current, attach, closereopen, replace
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Open,
    Halted,
    Closed,
}

#[derive(Clone, Copy, PartialEq)]
enum Origin {
    User,
    Schedule,
    Direct,
    Release,
}

#[derive(Clone, Copy, PartialEq)]
enum How {
    Quiet,
    UserEnd,
    UserMessage,
}

#[derive(Clone)]
enum At {
    Text(String),
    Ms(i64),
}

#[derive(Clone)]
struct Budget {
    id: String,
    state: State,
    origin: Origin,
    unlimited: bool,
    limits: [i64; 5],
    used: [i64; 8],
    active_since: Option<i64>,
    ended: Option<(How, At)>,
    opened_at: At,
    follows: Option<String>,
    rest: String,
}

struct Session {
    budget: Option<Budget>,
}

struct Ref {
    session_id: String,
    budget_id: String,
}

enum Effect {
    TimerClear,
    Changed,
    CarryHeld(String),
    Uuid,
}

struct World {
    sessions: HashMap<String, Session>,
    // budget id -> runs on its clock (the M9 clock map; empty in these items)
    clocks: HashMap<String, usize>,
    fx: Vec<Effect>,
    uuids: Vec<String>,
    next_uuid: usize,
    now: i64,
}

impl World {
    fn current(&self, r: &Ref) -> Option<&Budget> {
        match self.sessions.get(&r.session_id) {
            Some(Session { budget: Some(b) }) if b.id == r.budget_id => Some(b),
            _ => None,
        }
    }

    fn uuid(&mut self) -> String {
        let u = self.uuids[self.next_uuid].clone();
        self.next_uuid += 1;
        self.fx.push(Effect::Uuid);
        u
    }

    fn fresh(&mut self, origin: Origin, size: Option<i64>, unit: &[i64; 6], follows: Option<String>) -> Budget {
        let times = size.unwrap_or(1).max(1);
        let mut limits = [0i64; 5];
        for i in 0..5 {
            limits[i] = unit[i] * times;
        }
        Budget {
            id: self.uuid(),
            state: State::Open,
            origin,
            unlimited: size == Some(0) || (size.is_none() && unit[5] == 1),
            limits,
            used: [0; 8],
            active_since: None,
            ended: None,
            opened_at: At::Ms(self.now),
            follows,
            rest: String::new(),
        }
    }

    // CLOSE (clock part: SETTLE, state closed, DROP; then ended)
    fn close(clocks: &mut HashMap<String, usize>, fx: &mut Vec<Effect>, b: &mut Budget, how: How, now: i64) {
        if let Some(since) = b.active_since.take() {
            b.used[0] += now - since;
        }
        b.state = State::Closed;
        if clocks.remove(&b.id).is_some() {
            fx.push(Effect::TimerClear);
        }
        b.ended = Some((how, At::Ms(now)));
    }

    fn attach(&mut self, sid: &str, origin: Origin, size: Option<i64>, unit: &[i64; 6]) -> Option<Ref> {
        let open = match self.sessions.get(sid)? {
            Session { budget: Some(b) } if b.state != State::Closed => Some(b.id.clone()),
            _ => None,
        };
        let id = match open {
            Some(id) => id,
            None => {
                let b = self.fresh(origin, size, unit, None);
                let id = b.id.clone();
                self.sessions.get_mut(sid)?.budget = Some(b);
                self.fx.push(Effect::Changed);
                id
            }
        };
        Some(Ref { session_id: sid.to_string(), budget_id: id })
    }

    fn replace(&mut self, sid: &str, size: Option<i64>, origin: Option<Origin>, unit: &[i64; 6]) -> Option<Ref> {
        let origin = origin.unwrap_or(Origin::User);
        let halted = matches!(self.sessions.get(sid)?, Session { budget: Some(b) } if b.state == State::Halted);
        if !halted {
            return self.attach(sid, origin, size, unit);
        }
        let now = self.now;
        let s = self.sessions.get_mut(sid)?;
        let b = s.budget.as_mut()?;
        World::close(&mut self.clocks, &mut self.fx, b, How::UserMessage, now);
        let old = std::mem::take(&mut b.id);
        let follows = if origin != Origin::Schedule { Some(old.clone()) } else { None };
        let nb = self.fresh(origin, size, unit, follows);
        let id = nb.id.clone();
        self.sessions.get_mut(sid)?.budget = Some(nb);
        self.fx.push(Effect::Changed);
        if origin != Origin::Schedule {
            self.fx.push(Effect::CarryHeld(old));
        }
        Some(Ref { session_id: sid.to_string(), budget_id: id })
    }

    fn reopen(&mut self, r: &Ref) -> bool {
        let b = match self.sessions.get_mut(&r.session_id) {
            Some(Session { budget: Some(b) }) if b.id == r.budget_id => b,
            _ => return false,
        };
        if b.state != State::Closed || !matches!(b.ended, Some((How::Quiet, _))) {
            return false;
        }
        b.state = State::Open;
        b.ended = None;
        self.fx.push(Effect::Changed);
        true
    }

    fn maybe_close(&mut self, sid: &str, is_quiet: impl FnOnce() -> bool) -> bool {
        let now = self.now;
        let b = match self.sessions.get_mut(sid) {
            Some(Session { budget: Some(b) }) if b.state == State::Open => b,
            _ => return false,
        };
        if self.clocks.get(&b.id).copied().unwrap_or(0) > 0 || !is_quiet() {
            return false;
        }
        World::close(&mut self.clocks, &mut self.fx, b, How::Quiet, now);
        self.fx.push(Effect::Changed);
        true
    }
}

const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn rec(id: String, state: State) -> Budget {
    Budget {
        id,
        state,
        origin: Origin::User,
        unlimited: false,
        limits: [120, 24, 3, 2, 1],
        used: [0; 8],
        active_since: None,
        ended: None,
        opened_at: At::Text("2023-11-14T22:13:20.000Z".to_string()),
        follows: None,
        rest: PERF_REST.to_string(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size: usize = args[1].parse().unwrap();
    let reps: usize = args[2].parse().unwrap();
    let sids: Vec<String> = (0..size).map(|i| format!("s{:04}", i)).collect();
    let bid = |i: usize| format!("{:08x}-0000-4000-8000-000000000001", i);
    let unit = [120i64, 24, 3, 2, 1, 0];
    let setup = |state: State| -> World {
        let mut sessions = HashMap::new();
        for (i, s) in sids.iter().enumerate() {
            sessions.insert(s.clone(), Session { budget: Some(rec(bid(i), state)) });
        }
        World {
            sessions,
            clocks: HashMap::new(),
            fx: Vec::with_capacity(16),
            uuids: vec!["00000000-0000-4000-8000-0000000000aa".to_string()],
            next_uuid: 0,
            now: 1700000000000,
        }
    };
    let last = size - 1;
    let sid = sids[last].clone();
    let lref = Ref { session_id: sid.clone(), budget_id: bid(last) };
    let _ = (Origin::Direct, Origin::Release, How::UserEnd);

    // current
    let w = setup(State::Open);
    for _ in 0..1000 {
        black_box(w.current(black_box(&lref)));
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.current(black_box(&lref)));
    }
    println!("current {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);

    // isHalted
    let w = setup(State::Open);
    for _ in 0..1000 {
        black_box(w.current(black_box(&lref)).map_or(false, |b| b.state == State::Halted));
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.current(black_box(&lref)).map_or(false, |b| b.state == State::Halted));
    }
    println!("ishalted {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);

    // attach (open budget: ref)
    let mut w = setup(State::Open);
    let mut go = |w: &mut World| {
        w.fx.clear();
        w.next_uuid = 0;
        black_box(w.attach(black_box(&sid), Origin::User, None, &unit));
    };
    for _ in 0..1000 {
        go(&mut w);
    }
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w);
    }
    println!("attach {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);

    // maybeClose (quiet) then reopen
    let mut w = setup(State::Open);
    let mut go = |w: &mut World| {
        w.fx.clear();
        black_box(w.maybe_close(black_box(&sid), || true));
        w.fx.clear();
        black_box(w.reopen(black_box(&lref)));
    };
    for _ in 0..1000 {
        go(&mut w);
    }
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w);
    }
    println!("closereopen {} {:.1}", size, t.elapsed().as_nanos() as f64 / (2 * reps) as f64);

    // replace on a halted budget, then the halted budget is put back
    let mut w = setup(State::Halted);
    let halted = rec(bid(last), State::Halted);
    let mut go = |w: &mut World| {
        w.fx.clear();
        w.next_uuid = 0;
        black_box(w.replace(black_box(&sid), Some(2), Some(Origin::User), &unit));
        w.sessions.get_mut(black_box(&sid)).unwrap().budget = Some(halted.clone());
    };
    for _ in 0..1000 {
        go(&mut w);
    }
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w);
    }
    println!("replace {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
}

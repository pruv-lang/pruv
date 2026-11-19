// Hand-written idiomatic Rust baseline for the M10 halt perf items: the same work as bud_halt.pruv::halt +
// driver_m10.rs, in the order of spec §2.4:
//   CURRENT (session lookup, budget id compare, not halted), SETTLE (clock lookup; the perf budget has no clock, as
//   on the Pruv side), runningDelegationIds (the host moves its answer out, built per call like the Pruv lane's
//   `set_running(vec![ids.clone()])`), the halted state (halt section with reason, at, the id list moved in), then
//   the effects (cancel, stopMain, D ordered stopDel, stopTurns, hold, save, changed, announce) pushed to a reused
//   list and walked by the host.
// The timed loop also puts the record back open (`putback` times that alone): the budget is overwritten in place
// from an open template, which frees the halted id list.
// hand_m10 <D> <reps> prints `halt D ns` and `putback D ns`.
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone)]
struct Halt {
    reason: u8,
    at: i64,
    ids: Vec<String>,
}

#[derive(Clone)]
struct Budget {
    id: String,
    created_at: String,
    // activeSince as text (dropped by halt unless the clock keeps running), as in the record's string 2
    active_since: String,
    note: String,
    owner: String,
    rest: String,
    open: bool,
    halted: bool,
    time_minutes: i64,
    active_ms: i64,
    active_runs: i64,
    halt: Option<Halt>,
}

struct Clock {
    since: Option<i64>,
    timer: Option<u64>,
}

struct Ref {
    session_id: String,
    budget_id: String,
}

#[derive(Clone, Copy)]
enum Effect {
    Clear(u64),
    Cancel,
    StopMain,
    StopDel(usize),
    StopTurns,
    Hold,
    Save,
    Changed,
    Announce,
}

// The host's runningDelegationIds: the answer for the call's one query, moved out.
struct Host {
    answer: Vec<String>,
}

impl Host {
    fn running_delegation_ids(&mut self, _r: &Ref) -> Vec<String> {
        std::mem::take(&mut self.answer)
    }
}

struct World {
    sessions: HashMap<String, Budget>,
    clocks: HashMap<String, Clock>,
    fx: Vec<Effect>,
    now: i64,
}

impl World {
    // halt(ref, reason): 0 done (or a no-op on a stale ref / halted budget), 2 bad reason
    fn halt(&mut self, r: &Ref, reason: u8, host: &mut Host) -> u8 {
        self.fx.clear();
        if reason > 4 {
            return 2;
        }
        let Some(b) = self.sessions.get_mut(&r.session_id) else { return 0 };
        if b.id != r.budget_id || b.halted {
            return 0;
        }
        // SETTLE: fold the running interval into activeMs and clear the timer
        let mut keep_since = false;
        if let Some(c) = self.clocks.get_mut(&r.budget_id) {
            if let Some(s) = c.since.take() {
                b.active_ms += self.now - s;
                c.since = Some(self.now);
                keep_since = true;
            }
            if let Some(t) = c.timer.take() {
                self.fx.push(Effect::Clear(t));
            }
        }
        let ids = host.running_delegation_ids(r);
        let d = ids.len();
        b.halted = true;
        if !keep_since {
            b.active_since.clear();
        }
        b.halt = Some(Halt { reason, at: self.now, ids });
        self.fx.push(Effect::Cancel);
        self.fx.push(Effect::StopMain);
        for i in 0..d {
            self.fx.push(Effect::StopDel(i));
        }
        self.fx.extend([Effect::StopTurns, Effect::Hold, Effect::Save, Effect::Changed, Effect::Announce]);
        0
    }

    // The host performs the effects in order; a stopDel names the halted id.
    fn perform(&self, r: &Ref) {
        let b = &self.sessions[&r.session_id];
        for e in &self.fx {
            match *e {
                Effect::StopDel(i) => {
                    black_box(&b.halt.as_ref().unwrap().ids[i]);
                }
                e => {
                    black_box(e);
                }
            }
        }
    }
}

const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let d: usize = args[1].parse().unwrap();
    let reps: usize = args[2].parse().unwrap();
    let bid = "99b568f7-0000-4000-8000-000000000001";
    let open = Budget {
        id: bid.to_string(),
        created_at: "2023-11-14T22:13:20.000Z".to_string(),
        active_since: String::new(),
        note: String::new(),
        owner: String::new(),
        rest: PERF_REST.to_string(),
        open: true,
        halted: false,
        time_minutes: 120,
        active_ms: 0,
        active_runs: 0,
        halt: None,
    };
    let ids: Vec<String> = (0..d).map(|i| format!("d{:04}", i)).collect();
    let r = Ref { session_id: "session-perf".to_string(), budget_id: bid.to_string() };
    let mut w = World { sessions: HashMap::new(), clocks: HashMap::new(), fx: Vec::new(), now: 1_700_000_000_000 };
    w.sessions.insert(r.session_id.clone(), open.clone());
    let mut host = Host { answer: Vec::new() };
    let put = |w: &mut World| {
        *w.sessions.get_mut("session-perf").unwrap() = black_box(&open).clone();
    };
    let mut go = |w: &mut World, host: &mut Host| {
        host.answer = ids.clone();
        let s = w.halt(black_box(&r), 1, host);
        assert!(s == 0);
        w.perform(&r);
        put(w);
    };
    for _ in 0..1000 {
        go(&mut w, &mut host);
    }
    // the halt ran: D stops, halted, and the putback reopened it
    host.answer = ids.clone();
    assert!(w.halt(&r, 1, &mut host) == 0 && w.fx.len() == d + 7);
    assert!(w.sessions["session-perf"].halt.as_ref().unwrap().ids.len() == d);
    put(&mut w);
    assert!(!w.sessions["session-perf"].halted);
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w, &mut host);
    }
    println!("halt {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
    let t = Instant::now();
    for _ in 0..reps {
        put(&mut w);
    }
    println!("putback {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
}

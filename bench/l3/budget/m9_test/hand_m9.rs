// Hand-written idiomatic Rust baseline for the M9 perf items: the same clock work as bud_clock.pruv + driver_m9.rs
// (session lookup + budget id check, run map update, REFRESH scan, SETTLE / BEGIN+ARM, and the same host scheduler:
// a timer list plus a budget -> token map). activeSince is kept as the begin instant (no ISO string), like the Pruv
// side, which stores the instant and leaves formatting to the boundary.
//   hand_m9 <R> <reps>   prints `item size ns_per_call` for refresh, cycle, fire (see driver_m9.rs run_perf)
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

struct Budget {
    id: String,
    open: bool,
    unlimited: bool,
    time_minutes: i64,
    active_ms: i64,
    active_since: Option<i64>,
}

struct Clock {
    session_id: String,
    runs: HashMap<String, bool>,
    since: Option<i64>,
    timer: Option<u64>,
}

struct Timer {
    token: u64,
    due: i64,
    bid: String,
}

#[derive(Clone)]
struct Ref {
    session_id: String,
    budget_id: String,
}

struct World {
    sessions: HashMap<String, Budget>,
    clocks: HashMap<String, Clock>,
    timers: Vec<Timer>,
    by_bid: HashMap<String, u64>,
    seq: u64,
    now: i64,
}

impl World {
    fn current(&self, r: &Ref) -> bool {
        matches!(self.sessions.get(&r.session_id), Some(b) if b.id == r.budget_id)
    }

    fn clear(&mut self, bid: &str) {
        if let Some(tok) = self.by_bid.remove(bid) {
            self.timers.retain(|t| t.token != tok);
        }
    }

    fn set_timer(&mut self, bid: &str, left: i64) -> u64 {
        let eff = if left > 2147483647 { 1 } else { left };
        self.clear(bid);
        self.seq += 1;
        self.timers.push(Timer { token: self.seq, due: self.now + eff, bid: bid.to_string() });
        self.by_bid.insert(bid.to_string(), self.seq);
        self.seq
    }

    fn run_started(&mut self, r: &Ref, key: &str) -> bool {
        if !self.current(r) {
            return false;
        }
        let c = self.clocks.entry(r.budget_id.clone()).or_insert_with(|| Clock {
            session_id: r.session_id.clone(),
            runs: HashMap::new(),
            since: None,
            timer: None,
        });
        match c.runs.get_mut(key) {
            Some(w) => *w = false,
            None => {
                c.runs.insert(key.to_string(), false);
            }
        }
        self.refresh(r)
    }

    fn run_waiting(&mut self, r: &Ref, key: &str, waiting: bool) -> bool {
        let Some(c) = self.clocks.get_mut(&r.budget_id) else { return false };
        let Some(w) = c.runs.get_mut(key) else { return false };
        if *w == waiting {
            return false;
        }
        *w = waiting;
        self.refresh(r)
    }

    fn run_ended(&mut self, r: &Ref, key: &str) -> bool {
        let Some(c) = self.clocks.get_mut(&r.budget_id) else { return false };
        if c.runs.remove(key).is_none() {
            return false;
        }
        self.refresh(r)
    }

    // true = time halt
    fn refresh(&mut self, r: &Ref) -> bool {
        if !self.current(r) {
            return false;
        }
        let open = self.sessions[&r.session_id].open;
        let Some(c) = self.clocks.get(&r.budget_id) else { return false };
        let consuming = open && c.runs.values().any(|w| !*w);
        if consuming == c.since.is_some() {
            return false;
        }
        if !consuming {
            self.settle(r);
            return false;
        }
        self.begin(r);
        self.arm(r)
    }

    fn begin(&mut self, r: &Ref) {
        let now = self.now;
        self.clocks.get_mut(&r.budget_id).unwrap().since = Some(now);
        self.sessions.get_mut(&r.session_id).unwrap().active_since = Some(now);
    }

    fn arm(&mut self, r: &Ref) -> bool {
        let b = &self.sessions[&r.session_id];
        if b.unlimited {
            return false;
        }
        let left = b.time_minutes * 60000 - b.active_ms;
        if left <= 0 {
            return self.fire(r);
        }
        let tok = self.set_timer(&r.budget_id, left);
        self.clocks.get_mut(&r.budget_id).unwrap().timer = Some(tok);
        false
    }

    fn fire(&mut self, r: &Ref) -> bool {
        if let Some(c) = self.clocks.get_mut(&r.budget_id) {
            c.timer = None;
        }
        if !self.current(r) {
            return false;
        }
        let b = &self.sessions[&r.session_id];
        if !b.open || b.unlimited {
            return false;
        }
        self.settle(r);
        let b = &self.sessions[&r.session_id];
        let runs = self.clocks.get(&r.budget_id).map(|c| c.runs.len()).unwrap_or(0);
        if b.active_ms < b.time_minutes * 60000 && runs > 0 {
            self.begin(r);
            return self.arm(r);
        }
        true
    }

    fn settle(&mut self, r: &Ref) -> i64 {
        let Some(c) = self.clocks.get_mut(&r.budget_id) else { return 0 };
        if c.session_id != r.session_id {
            return 0;
        }
        let had = c.timer.take().is_some();
        let since = c.since.take();
        if had {
            self.clear(&r.budget_id);
        }
        let Some(since) = since else { return 0 };
        let added = (self.now - since).max(0);
        let b = self.sessions.get_mut(&r.session_id).unwrap();
        b.active_since = None;
        b.active_ms += added;
        added
    }

    // host delivery of the active timer at index i (driver_m9 fire_token)
    fn deliver(&mut self, i: usize) -> bool {
        let t = self.timers.remove(i);
        self.by_bid.remove(&t.bid);
        let owner = self.clocks.get(&t.bid).map(|c| c.session_id.clone());
        match owner {
            Some(s) => self.fire(&Ref { session_id: s, budget_id: t.bid }),
            None => false,
        }
    }
}

fn world() -> (World, Ref) {
    let r = Ref { session_id: "session-perf".to_string(), budget_id: "99b568f7-0000-4000-8000-000000000001".to_string() };
    let mut sessions = HashMap::new();
    sessions.insert(r.session_id.clone(), Budget {
        id: r.budget_id.clone(),
        open: true,
        unlimited: false,
        time_minutes: 1000000,
        active_ms: 0,
        active_since: None,
    });
    (World { sessions, clocks: HashMap::new(), timers: Vec::new(), by_bid: HashMap::new(), seq: 0, now: 1700000000000 },
     r)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rsz: usize = a[1].parse().unwrap();
    let reps: usize = a[2].parse().unwrap();
    // refresh_R
    let (mut w, r) = world();
    let keys: Vec<String> = (0..rsz).map(|i| format!("run-{:04}", i)).collect();
    for k in &keys {
        w.run_started(&r, k);
    }
    for k in &keys[..rsz - 1] {
        w.run_waiting(&r, k, true);
    }
    let last = keys[rsz - 1].clone();
    for _ in 0..1000 {
        w.run_waiting(&r, &last, true);
        w.run_waiting(&r, &last, false);
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.run_waiting(black_box(&r), black_box(&last), true));
        black_box(w.run_waiting(black_box(&r), black_box(&last), false));
    }
    println!("refresh {} {:.1}", rsz, t.elapsed().as_nanos() as f64 / (2 * reps) as f64);
    // cycle
    let (mut w, r) = world();
    let k0 = "run-0000".to_string();
    for _ in 0..1000 {
        w.run_started(&r, &k0);
        w.run_ended(&r, &k0);
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(w.run_started(black_box(&r), black_box(&k0)));
        black_box(w.run_ended(black_box(&r), black_box(&k0)));
    }
    println!("cycle 1 {:.1}", t.elapsed().as_nanos() as f64 / (2 * reps) as f64);
    // fire
    let (mut w, r) = world();
    w.run_started(&r, &k0);
    let t = Instant::now();
    for _ in 0..reps {
        w.now += 1;
        let i = w.timers.len() - 1;
        black_box(w.deliver(i));
    }
    println!("fire 1 {:.1}", t.elapsed().as_nanos() as f64 / reps as f64);
}

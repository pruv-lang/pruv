// Hand-written idiomatic Rust baseline for the M9 perf items: the same clock work as bud_clock.pruv + driver_m9.rs
// (session lookup + budget id check, run map update, active-run count, REFRESH, SETTLE / BEGIN+ARM, and the same host
// scheduler: a timer list plus a budget -> token map, keys copied only when a timer captures its ref or a budget gets
// its first map slot). activeSince is kept as the begin instant (no ISO string), like the Pruv
// side, which stores the instant and leaves formatting to the boundary.
// hand_m9 <R> <reps> prints `item size ns` for refresh, cycle, fire, and the following items:
//   drop: start R runs, then clear the timer and remove the clock and its run map.
//   close: start R runs, advance 1 ms, settle and close the budget, then remove the clock.
//          Reset the budget outside the timing of each repetition.
//   stale/staleu: deliver a captured callback after replacing the budget id or making the budget unlimited.
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
    // runs with waiting = false, kept by the run methods (bud_clock field 17); REFRESH reads it instead of scanning
    active: usize,
    since: Option<i64>,
    timer: Option<u64>,
}

struct Timer {
    token: u64,
    due: i64,
    sid: String,
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

    // setTimeout with the captured ref; the budget's previous handle (none in these items) is overwritten
    fn set_timer(&mut self, r: &Ref, left: i64) -> u64 {
        let eff = if left > 2147483647 { 1 } else { left };
        self.seq += 1;
        self.timers.push(Timer { token: self.seq, due: self.now + eff, sid: r.session_id.clone(), bid: r.budget_id.clone() });
        match self.by_bid.get_mut(&r.budget_id) {
            Some(v) => *v = self.seq,
            None => {
                self.by_bid.insert(r.budget_id.clone(), self.seq);
            }
        }
        self.seq
    }

    fn run_started(&mut self, r: &Ref, key: &str) -> bool {
        if !self.current(r) {
            return false;
        }
        // borrowed-key lookup; the budget id is cloned only when the clock is created
        if !self.clocks.contains_key(&r.budget_id) {
            self.clocks.insert(r.budget_id.clone(), Clock {
                session_id: r.session_id.clone(),
                runs: HashMap::new(),
                active: 0,
                since: None,
                timer: None,
            });
        }
        let c = self.clocks.get_mut(&r.budget_id).unwrap();
        match c.runs.get_mut(key) {
            Some(w) => {
                if *w {
                    c.active += 1;
                }
                *w = false
            }
            None => {
                c.runs.insert(key.to_string(), false);
                c.active += 1;
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
        if waiting {
            c.active = c.active.saturating_sub(1);
        } else {
            c.active += 1;
        }
        self.refresh(r)
    }

    fn run_ended(&mut self, r: &Ref, key: &str) -> bool {
        let Some(c) = self.clocks.get_mut(&r.budget_id) else { return false };
        match c.runs.remove(key) {
            None => return false,
            Some(false) => c.active = c.active.saturating_sub(1),
            Some(true) => {}
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
        let consuming = open && c.active > 0;
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
        let tok = self.set_timer(r, left);
        self.clocks.get_mut(&r.budget_id).unwrap().timer = Some(tok);
        false
    }

    fn fire(&mut self, r: &Ref) -> bool {
        if let Some(c) = self.clocks.get_mut(&r.budget_id) {
            // forget the handle without cancelling (timerForget)
            if c.timer.take().is_some() {
                self.by_bid.remove(&r.budget_id);
            }
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

    fn drop_clock(&mut self, r: &Ref) {
        if self.clocks.remove(&r.budget_id).is_some_and(|clock| clock.timer.is_some()) {
            self.clear(&r.budget_id);
        }
    }

    fn close(&mut self, r: &Ref) {
        if self.current(r) {
            self.settle(r);
            self.sessions.get_mut(&r.session_id).unwrap().open = false;
            self.drop_clock(r);
        }
    }

    fn advance(&mut self, ms: i64) -> bool {
        let target = self.now + ms;
        while let Some((i, timer)) = self.timers.iter().enumerate()
            .filter(|(_, timer)| timer.due <= target)
            .min_by_key(|(_, timer)| (timer.due, timer.token))
        {
            self.now = timer.due;
            if self.deliver(i) {
                return true;
            }
        }
        self.now = target;
        false
    }

    // host delivery of the active timer at index i (driver_m9 fire_token): the callback runs with its captured ref
    fn deliver(&mut self, i: usize) -> bool {
        let t = self.timers.remove(i);
        self.fire(&Ref { session_id: t.sid, budget_id: t.bid })
    }
}

fn budget(id: String, unlimited: bool) -> Budget {
    Budget { id, open: true, unlimited, time_minutes: 60, active_ms: 0, active_since: None }
}

fn world() -> (World, Ref) {
    let r = Ref { session_id: "session-perf".to_string(), budget_id: "99b568f7-0000-4000-8000-000000000001".to_string() };
    let mut sessions = HashMap::new();
    sessions.insert(r.session_id.clone(), budget(r.budget_id.clone(), false));
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
    // fire: same budget as the driver's fire item (timeMinutes 1000000, delay capped to 1 ms, never exhausted)
    let (mut w, r) = world();
    w.sessions.get_mut(&r.session_id).unwrap().time_minutes = 1000000;
    w.run_started(&r, &k0);
    let t = Instant::now();
    for _ in 0..reps {
        w.now += 1;
        let i = w.timers.len() - 1;
        black_box(w.deliver(i));
    }
    println!("fire 1 {:.1}", t.elapsed().as_nanos() as f64 / reps as f64);

    for (item, closing) in [("drop", false), ("close", true)] {
        let (mut w, r) = world();
        let mut elapsed = std::time::Duration::ZERO;
        for _ in 0..reps {
            for key in &keys {
                black_box(w.run_started(black_box(&r), black_box(key)));
            }
            if closing {
                w.advance(1);
            }
            assert!(w.timers.len() == 1 && w.timers[0].due > w.now, "{}: timer not alive", item);
            let t = Instant::now();
            if closing {
                black_box(w.close(black_box(&r)));
            } else {
                black_box(w.drop_clock(black_box(&r)));
            }
            elapsed += t.elapsed();
            assert!(w.timers.is_empty(), "{}: timer not cleared", item);
            if closing {
                assert!(w.sessions[&r.session_id].active_ms == 1, "close: settled ms");
                w.sessions.insert(r.session_id.clone(), budget(r.budget_id.clone(), false));
            }
        }
        println!("{} {} {:.1}", item, rsz, elapsed.as_nanos() as f64 / reps as f64);
    }
    if rsz == 10 {
        for (item, unlimited) in [("stale", false), ("staleu", true)] {
            let (mut w, r) = world();
            let changed_id = if unlimited {
                r.budget_id.clone()
            } else {
                format!("{}2", &r.budget_id[..r.budget_id.len() - 1])
            };
            let mut elapsed = std::time::Duration::ZERO;
            for _ in 0..reps {
                w.sessions.insert(r.session_id.clone(), budget(r.budget_id.clone(), false));
                w.run_started(&r, &k0);
                w.sessions.insert(r.session_id.clone(), budget(changed_id.clone(), unlimited));
                let captured = w.timers.pop().unwrap();
                let captured_ref = Ref { session_id: captured.sid, budget_id: captured.bid };
                let t = Instant::now();
                black_box(w.fire(black_box(&captured_ref)));
                elapsed += t.elapsed();
                w.drop_clock(&r);
            }
            println!("{} 1 {:.1}", item, elapsed.as_nanos() as f64 / reps as f64);
        }
    }
}

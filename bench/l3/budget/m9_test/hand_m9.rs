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

#[cfg(m9_typed)]
use super::{sol, World as DriverWorld};

#[cfg(not(m9_typed))]
struct Budget {
    id: String,
    open: bool,
    unlimited: bool,
    time_minutes: i64,
    active_ms: i64,
    active_since: Option<i64>,
}

#[cfg(not(m9_typed))]
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
    captured: Ref,
}

#[derive(Clone)]
struct Ref {
    session_id: String,
    budget_id: String,
}

// Both benchmark lanes use the same timer I/O and captured ref.
struct TimerHost {
    timers: Vec<Timer>,
    by_bid: HashMap<String, u64>,
    seq: u64,
    now: i64,
}

impl TimerHost {
    fn clear(&mut self, bid: &str) {
        if let Some(tok) = self.by_bid.remove(bid) {
            self.timers.retain(|t| t.token != tok);
        }
    }

    fn set_timer(&mut self, r: &Ref, delay: i64) -> u64 {
        self.seq += 1;
        self.timers.push(Timer { token: self.seq, due: self.now + delay, captured: r.clone() });
        match self.by_bid.get_mut(&r.budget_id) {
            Some(v) => *v = self.seq,
            None => {
                self.by_bid.insert(r.budget_id.clone(), self.seq);
            }
        }
        self.seq
    }
}

struct World {
    #[cfg(not(m9_typed))]
    sessions: HashMap<String, Budget>,
    #[cfg(not(m9_typed))]
    clocks: HashMap<String, Clock>,
    #[cfg(m9_typed)]
    st: sol::St,
    host: TimerHost,
}

#[cfg(not(m9_typed))]
impl World {
    fn seed(&mut self, r: &Ref, id: &str, unlimited: bool, minutes: i64) {
        self.sessions.insert(r.session_id.clone(), Budget {
            id: id.to_string(), open: true, unlimited, time_minutes: minutes, active_ms: 0, active_since: None,
        });
    }

    fn active_ms(&self, r: &Ref) -> i64 {
        self.sessions[&r.session_id].active_ms
    }

    fn current(&self, r: &Ref) -> bool {
        matches!(self.sessions.get(&r.session_id), Some(b) if b.id == r.budget_id)
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
        let now = self.host.now;
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
        let delay = if left > 2147483647 { 1 } else { left };
        let tok = self.host.set_timer(r, delay);
        self.clocks.get_mut(&r.budget_id).unwrap().timer = Some(tok);
        false
    }

    fn fire(&mut self, r: &Ref) -> bool {
        if let Some(c) = self.clocks.get_mut(&r.budget_id) {
            // forget the handle without cancelling (timerForget)
            if c.timer.take().is_some() {
                self.host.by_bid.remove(&r.budget_id);
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
            self.host.clear(&r.budget_id);
        }
        let Some(since) = since else { return 0 };
        let added = (self.host.now - since).max(0);
        let b = self.sessions.get_mut(&r.session_id).unwrap();
        b.active_since = None;
        b.active_ms += added;
        added
    }

    fn drop_clock(&mut self, r: &Ref) {
        if self.clocks.remove(&r.budget_id).is_some_and(|clock| clock.timer.is_some()) {
            self.host.clear(&r.budget_id);
        }
    }

    fn close(&mut self, r: &Ref) {
        if self.current(r) {
            self.settle(r);
            self.sessions.get_mut(&r.session_id).unwrap().open = false;
            self.drop_clock(r);
        }
    }
}

#[cfg(m9_typed)]
impl World {
    fn seed(&mut self, r: &Ref, id: &str, unlimited: bool, minutes: i64) {
        assert_eq!(sol::t_put(&mut self.st, &r.session_id, id, 0, unlimited, minutes, 0, 0, 0), 0);
    }

    fn active_ms(&self, r: &Ref) -> i64 {
        self.st.sessions[&r.session_id].active_ms
    }

    fn call(&mut self, code: i64, r: &Ref, key: &str, waiting: bool) -> bool {
        let status = sol::t_call(&mut self.st, code, &r.session_id, &r.budget_id, key, waiting, self.host.now);
        for effect in &self.st.out.fx {
            match effect.tag {
                12 => { self.host.set_timer(r, effect.effective); }
                13 => self.host.clear(&r.budget_id),
                14 => { self.host.by_bid.remove(&r.budget_id); }
                _ => unreachable!("unexpected M9 timer effect"),
            }
        }
        status == 4
    }

    fn run_started(&mut self, r: &Ref, key: &str) -> bool { self.call(1, r, key, false) }
    fn run_waiting(&mut self, r: &Ref, key: &str, waiting: bool) -> bool { self.call(2, r, key, waiting) }
    fn run_ended(&mut self, r: &Ref, key: &str) -> bool { self.call(3, r, key, false) }
    fn fire(&mut self, r: &Ref) -> bool { self.call(4, r, "", false) }
    fn drop_clock(&mut self, r: &Ref) { self.call(5, r, "", false); }
    fn close(&mut self, r: &Ref) { self.call(6, r, "", false); }
}

impl World {
    fn deliver(&mut self, i: usize) -> bool {
        let timer = self.host.timers.remove(i);
        self.fire(&timer.captured)
    }
}

fn world(minutes: i64) -> (World, Ref) {
    let r = Ref { session_id: "session-perf".to_string(), budget_id: "99b568f7-0000-4000-8000-000000000001".to_string() };
    let mut w = World {
        #[cfg(not(m9_typed))]
        sessions: HashMap::new(),
        #[cfg(not(m9_typed))]
        clocks: HashMap::new(),
        #[cfg(m9_typed)]
        st: DriverWorld::<false>::new_state(),
        host: TimerHost { timers: Vec::new(), by_bid: HashMap::new(), seq: 0, now: 1700000000000 },
    };
    w.seed(&r, &r.budget_id, false, minutes);
    (w, r)
}

#[cfg(not(m9_typed))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    run_perf(&args[1..]);
}

pub(crate) fn run_perf(args: &[String]) {
    let rsz: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    // refresh_R
    let (mut w, r) = world(60);
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
    let (mut w, r) = world(60);
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
    let (mut w, r) = world(1000000);
    w.run_started(&r, &k0);
    let t = Instant::now();
    for _ in 0..reps {
        w.host.now += 1;
        let i = w.host.timers.len() - 1;
        black_box(w.deliver(i));
    }
    println!("fire 1 {:.1}", t.elapsed().as_nanos() as f64 / reps as f64);

    for (item, closing) in [("drop", false), ("close", true)] {
        let (mut w, r) = world(60);
        let mut elapsed = std::time::Duration::ZERO;
        for _ in 0..reps {
            for key in &keys {
                black_box(w.run_started(black_box(&r), black_box(key)));
            }
            if closing {
                w.host.now += 1;
            }
            assert!(w.host.timers.len() == 1 && w.host.timers[0].due > w.host.now, "{}: timer not alive", item);
            let t = Instant::now();
            if closing {
                black_box(w.close(black_box(&r)));
            } else {
                black_box(w.drop_clock(black_box(&r)));
            }
            elapsed += t.elapsed();
            assert!(w.host.timers.is_empty(), "{}: timer not cleared", item);
            if closing {
                assert!(w.active_ms(&r) == 1, "close: settled ms");
                w.seed(&r, &r.budget_id, false, 60);
            }
        }
        println!("{} {} {:.1}", item, rsz, elapsed.as_nanos() as f64 / reps as f64);
    }
    if rsz == 10 {
        for (item, unlimited) in [("stale", false), ("staleu", true)] {
            let (mut w, r) = world(60);
            let changed_id = if unlimited {
                r.budget_id.clone()
            } else {
                format!("{}2", &r.budget_id[..r.budget_id.len() - 1])
            };
            let mut elapsed = std::time::Duration::ZERO;
            for _ in 0..reps {
                w.seed(&r, &r.budget_id, false, 60);
                w.run_started(&r, &k0);
                w.seed(&r, &changed_id, unlimited, 60);
                let captured_ref = w.host.timers.pop().unwrap().captured;
                let t = Instant::now();
                black_box(w.fire(black_box(&captured_ref)));
                elapsed += t.elapsed();
                w.drop_clock(&r);
            }
            println!("{} 1 {:.1}", item, elapsed.as_nanos() as f64 / reps as f64);
        }
    }
}

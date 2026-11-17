// Hand-written idiomatic Rust baseline for the M8 perf items: the same accounting work as bud_charge.pruv +
// bud_tools.pruv + driver_m8.rs (session lookup + budget id check, the four limit checks, the counter writes; the
// per-turn activity sets as a map of sets). Same items and shapes as driver_m8.rs run_perf.
//   hand_m8 <N> <reps>   prints `item size ns_per_call`
// No `chk` lines: this baseline keeps UTF-8 String keys and typed fields with no observe word format, so it cannot
// produce the Pruv observe words cheaply; run_m8.py compares chk only between Pruv and hand_m8_u16.
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
enum State {
    Open,
    Halted,
    Closed,
}

struct Budget {
    id: String,
    state: State,
    unlimited: bool,
    halt_reason: Option<usize>,
    limits: [i64; 4],
    used: [i64; 4],
    tools: i64,
    failures: i64,
    retries: i64,
}

struct Ref {
    session_id: String,
    budget_id: String,
}

#[derive(Default)]
struct What {
    counts: [Option<i64>; 4],
    retries: Option<i64>,
}

enum Charge {
    Ok,
    Refused(usize),
}

struct World {
    sessions: HashMap<String, Budget>,
    tool_seen: HashMap<String, HashSet<String>>,
}

impl World {
    fn current(&mut self, r: &Ref) -> Option<&mut Budget> {
        match self.sessions.get_mut(&r.session_id) {
            Some(b) if b.id == r.budget_id => Some(b),
            _ => None,
        }
    }

    fn charge(&mut self, r: &Ref, what: &What) -> Charge {
        let b = match self.current(r) {
            Some(b) if b.state != State::Closed => b,
            _ => return Charge::Ok,
        };
        if b.state == State::Halted {
            return Charge::Refused(b.halt_reason.unwrap_or(1));
        }
        for k in 0..4 {
            let a = what.counts[k].unwrap_or(0);
            if !b.unlimited && a > 0 && b.used[k] + a > b.limits[k] {
                // halt(ref, k) would run here (M10)
                return Charge::Refused(1 + k);
            }
        }
        for k in 0..4 {
            b.used[k] += what.counts[k].unwrap_or(0);
        }
        b.retries += what.retries.unwrap_or(0);
        Charge::Ok
    }

    fn note_failure(&mut self, r: &Ref) {
        if let Some(b) = self.current(r) {
            b.failures += 1;
        }
    }

    fn note_tool(&mut self, r: &Ref, turn: &str, activity: &str) {
        let seen = match self.tool_seen.get_mut(turn) {
            Some(s) => s,
            None => self.tool_seen.entry(turn.to_string()).or_default(),
        };
        if seen.contains(activity) {
            return;
        }
        seen.insert(activity.to_string());
        if let Some(b) = self.current(r) {
            b.tools += 1;
        }
    }

    fn forget_turn(&mut self, turn: &str) {
        self.tool_seen.remove(turn);
    }
}

fn bid_of(i: usize) -> String {
    format!("99b568f7-0000-4000-8000-{:012}", i)
}

fn refk(i: usize) -> Ref {
    Ref { session_id: format!("session-{:05}", i), budget_id: bid_of(i) }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args[1].parse().unwrap();
    let reps: usize = args[2].parse().unwrap();
    // optional item name: only that item's timed loop runs (setup always runs), as in driver_m8.rs
    let only = args.get(3).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut w = World { sessions: HashMap::new(), tool_seen: HashMap::new() };
    for i in 0..n {
        w.sessions.insert(
            format!("session-{:05}", i),
            Budget {
                id: bid_of(i),
                state: State::Open,
                unlimited: false,
                halt_reason: None,
                limits: [1_000_000_000_000; 4],
                used: [0; 4],
                tools: 0,
                failures: 0,
                retries: 0,
            },
        );
    }
    let refs: Vec<Ref> = (0..n).map(refk).collect();
    let what = What { counts: [None, Some(1), None, None], retries: Some(1) };
    for i in 0..1000 {
        w.charge(&refs[i % n], &what);
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.charge(black_box(&refs[i % n]), black_box(&what)));
        }
        println!("charge {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            w.note_failure(black_box(&refs[i % n]));
        }
        println!("failure {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    let acts: Vec<String> = (0..n).map(|j| format!("activity-{:05}", j)).collect();
    for a in &acts {
        w.note_tool(&refs[0], "turn-1", a);
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            w.note_tool(black_box(&refs[0]), black_box("turn-1"), black_box(&acts[i % n]));
        }
        println!("tooldup {} {:.1}", n, t.elapsed().as_nanos() as f64 / reps as f64);
    }
    for a in &acts {
        w.note_tool(&refs[0], "turn-2", a);
    }
    w.forget_turn("turn-2");
    if on("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let t = Instant::now();
        for _ in 0..rounds {
            for a in &acts {
                w.note_tool(black_box(&refs[0]), black_box("turn-2"), black_box(a));
            }
            w.forget_turn(black_box("turn-2"));
        }
        println!("toolgrow {} {:.1}", n, t.elapsed().as_nanos() as f64 / (rounds * n) as f64);
    }
    black_box(&w.sessions);
}

// Hand Rust raise baseline. This process owns its typed state and mock host.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, layout, size)
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

pub fn allocations() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}

pub const SID: &str = "session-perf";
pub const BID: &str = "99b568f7-0000-4000-8000-000000000001";
const LIMITS: [i64; 5] = [120, 24, 3, 2, 1];
const MAX_SAFE: i64 = (1 << 53) - 1;

pub struct Setup {
    pub d: usize,
    pub reps: usize,
    pub size: i64,
    pub unit: [i64; 6],
    pub effect_words: i64,
}

impl Setup {
    pub fn parse(args: &[String]) -> Self {
        let (size, unit, effect_words) = match args[0].as_str() {
            "time" => (2, [0, 120, 0, 0, 0, 0], 5),
            "limits" => (2, [0, 0, 24, 3, 2, 1], 1),
            "unlimited" => (0, [0; 6], 2),
            other => panic!("unknown path {other}"),
        };
        Self {
            d: args[1].parse().unwrap(),
            reps: args[2].parse().unwrap(),
            size,
            unit,
            effect_words,
        }
    }
}

pub fn measure(reps: usize, mut op: impl FnMut() -> (Duration, u64)) {
    for _ in 0..1000 {
        op();
    }
    let mut elapsed = Duration::ZERO;
    let mut allocs = 0;
    for _ in 0..reps {
        let (duration, count) = op();
        elapsed += duration;
        allocs += count;
    }
    println!("{:.1} {:.4}", elapsed.as_nanos() as f64 / reps as f64, allocs as f64 / reps as f64);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    Halted,
    Closed,
}

struct Budget {
    id: String,
    state: State,
    unlimited: bool,
    limits: [i64; 5],
    active_ms: i64,
    active_since: Option<i64>,
    halt: Option<Halt>,
}

struct Halt {
    at: i64,
    stopped: Vec<String>,
}

struct Clock {
    owner: String,
    since: Option<i64>,
    armed: bool,
    active: usize,
    runs: HashMap<String, bool>,
}

#[derive(Clone, Copy)]
enum Effect {
    Clear,
    Set(i64),
    CancelRecovery,
    StopMain,
    StopDelegation(usize),
    StopAttachedTurns,
    HoldQueued,
    Save,
    Changed,
    Announce,
}

struct Host {
    running: Vec<String>,
    timer: Option<i64>,
}

impl Host {
    fn running_delegation_ids(&self, _sid: &str, _bid: &str) -> Vec<String> {
        self.running.clone()
    }

    fn perform(&mut self, effects: &[Effect], now: i64, sessions: &HashMap<String, Budget>, sid: &str) {
        for &effect in effects {
            match effect {
                Effect::Clear => self.timer = None,
                Effect::Set(left) => self.timer = Some(now + if left > i32::MAX.into() { 1 } else { left }),
                Effect::StopDelegation(i) => { black_box(&sessions[sid].halt.as_ref().unwrap().stopped[i]); }
                Effect::Announce => { black_box(sessions[sid].halt.as_ref().unwrap().at); }
                other => { black_box(other); }
            }
        }
    }
}

fn settle(clock: &mut Clock, budget: &mut Budget, now: i64, effects: &mut Vec<Effect>) {
    if std::mem::take(&mut clock.armed) {
        effects.push(Effect::Clear);
    }
    if let Some(since) = clock.since.take() {
        budget.active_ms += (now - since).max(0);
        budget.active_since = None;
    }
}

fn raise(
    sessions: &mut HashMap<String, Budget>,
    clocks: &mut HashMap<String, Clock>,
    sid: &str,
    bid: &str,
    size: i64,
    now: i64,
    effects: &mut Vec<Effect>,
    host: &Host,
) -> Result<bool, ()> {
    effects.clear();
    let Some(budget) = sessions.get_mut(sid) else { return Ok(false) };
    if budget.id != bid || budget.state == State::Closed || size == 1 {
        return Ok(false);
    }
    let clock = clocks.get_mut(bid);
    if size == 0 {
        if budget.unlimited {
            return Ok(false);
        }
        budget.unlimited = true;
        if let Some(clock) = clock {
            if std::mem::take(&mut clock.armed) {
                effects.push(Effect::Clear);
            }
        }
    } else {
        if !matches!(size, 2 | 4 | 8) {
            return Err(());
        }
        let unit = bud_facts::unit(0);
        let [0, t, d, c, r, ch] = unit.as_slice() else { return Err(()) };
        let values = [*t, *d, *c, *r, *ch];
        if values.iter().any(|v| !(0..=MAX_SAFE / size).contains(v)) {
            return Err(());
        }
        let next = std::array::from_fn(|i| budget.limits[i].max(size * values[i]));
        if next == budget.limits {
            return Ok(false);
        }
        let time_raised = next[0] > budget.limits[0];
        budget.limits = next;
        if let Some(clock) = clock {
            if time_raised && clock.since.is_some() && !budget.unlimited {
                if clock.owner == sid {
                    settle(clock, budget, now, effects);
                }
                if budget.state == State::Open && clock.active > 0 && clock.since.is_none() {
                    clock.since = Some(now);
                    budget.active_since = Some(now);
                    let left = next[0].clamp(-150_000_000_000_000, 150_000_000_000_000) * 60_000
                        - budget.active_ms;
                    if left > 0 {
                        clock.armed = true;
                        effects.push(Effect::Set(left));
                    } else {
                        settle(clock, budget, now, effects);
                        let stopped = host.running_delegation_ids(sid, bid);
                        let count = stopped.len();
                        budget.state = State::Halted;
                        budget.halt = Some(Halt { at: now, stopped });
                        effects.extend([Effect::CancelRecovery, Effect::StopMain]);
                        for i in 0..count {
                            effects.push(Effect::StopDelegation(i));
                        }
                        effects.extend([Effect::StopAttachedTurns, Effect::HoldQueued,
                                        Effect::Save, Effect::Changed, Effect::Announce]);
                    }
                }
            }
        }
    }
    effects.push(Effect::Changed);
    Ok(true)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let setup = Setup::parse(&args);
    let mut sessions = HashMap::from([(SID.to_string(), Budget {
        id: BID.to_string(), state: State::Open, unlimited: false, limits: LIMITS,
        active_ms: 0, active_since: None, halt: None,
    })]);
    let mut clocks = HashMap::from([(BID.to_string(), Clock {
        owner: SID.to_string(), since: None, armed: false, active: 1,
        runs: (0..setup.d).map(|i| (format!("d{i:04}"), i != 0)).collect(),
    })]);
    let mut host = Host {
        running: (0..setup.d).map(|i| format!("d{i:04}")).collect(),
        timer: None,
    };
    let mut effects = Vec::with_capacity(8);
    let mut now = 1_700_000_000_000;
    bud_facts::set_unit(&setup.unit);
    measure(setup.reps, || {
        let budget = sessions.get_mut(SID).unwrap();
        budget.limits = LIMITS;
        budget.unlimited = false;
        budget.active_ms = 0;
        budget.active_since = Some(now);
        let clock = clocks.get_mut(BID).unwrap();
        assert_eq!(clock.runs.len(), setup.d);
        clock.since = Some(now);
        clock.armed = true;
        host.timer = Some(now + LIMITS[0] * 60_000);
        now += 1;

        let before = allocations();
        let start = Instant::now();
        let result = raise(&mut sessions, &mut clocks, black_box(SID), black_box(BID),
                           black_box(setup.size), now, &mut effects, &host);
        host.perform(&effects, now, &sessions, SID);
        black_box(host.timer);
        let elapsed = start.elapsed();
        let allocs = allocations() - before;
        assert_eq!(result, Ok(true));
        assert_eq!(effects.iter().map(|effect| if matches!(effect, Effect::Set(_)) { 3 } else { 1 })
                   .sum::<i64>(), setup.effect_words);
        (elapsed, allocs)
    });
}

//! Lifecycle benchmark fixtures and measurement loop.
use super::*;

const NOW: i64 = 1_700_000_000_000;
const WIDTH: usize = 64;

fn string_field(out: &mut Vec<i64>, name: &str, value: &str) {
    key(out, name);
    sv(out, value);
}

fn integer_field(out: &mut Vec<i64>, name: &str, value: i64) {
    key(out, name);
    out.extend([5, value]);
}

fn limits(out: &mut Vec<i64>, minutes: i64) {
    out.extend([9, 5]);
    integer_field(out, "timeMinutes", minutes);
    for (name, count) in [("delegations", 2), ("commands", 3), ("reviews", 4), ("checks", 5)] {
        integer_field(out, name, count);
    }
}

fn budget(id: &str, minutes: i64, unlimited: bool) -> Vec<i64> {
    let mut out = vec![9, 6 + i64::from(unlimited)];
    string_field(&mut out, "id", id);
    string_field(&mut out, "openedAt", "2023-11-14T22:13:20.000Z");
    string_field(&mut out, "origin", "user");
    string_field(&mut out, "state", "open");
    key(&mut out, "limits");
    limits(&mut out, minutes);
    if unlimited {
        key(&mut out, "unlimited");
        out.push(4);
    }
    key(&mut out, "used");
    out.extend([9, 8]);
    for name in ["activeMs", "delegations", "commands", "reviews", "checks", "tools", "failures", "retries"] {
        integer_field(&mut out, name, 0);
    }
    out
}

struct Request {
    op: i64,
    rf: Vec<i64>,
    sid: Vec<i64>,
    arg: Vec<i64>,
    #[cfg(m12_typed)]
    typed_sid: String,
    #[cfg(m12_typed)]
    typed_keys: sol::LifeKeys,
    hand_rf: hand::life::Value,
    hand_sid: Vec<u16>,
    hand_arg: hand::life::Value,
    now: i64,
}

impl Request {
    fn new(op: i64, sid: &str, bid: &str, arg: Vec<i64>, now: i64) -> Self {
        let mut rf = vec![1];
        if !bid.is_empty() {
            rf.clear();
            rf.extend([9, 2]);
            string_field(&mut rf, "sessionId", sid);
            string_field(&mut rf, "budgetId", bid);
        }
        Self {
            #[cfg(m12_typed)]
            typed_sid: m12_facts::decode_key(&pkey(sid)),
            #[cfg(m12_typed)]
            typed_keys: sol::decode_life(&rf, &arg),
            op, hand_rf: hand::life::Value::decode(&rf), hand_arg: hand::life::Value::decode(&arg),
            rf, sid: pkey(sid), arg, hand_sid: sid.encode_utf16().collect(), now,
        }
    }
}

#[derive(Clone)]
struct Sample {
    sessions: Lane,
    clocks: Clocks,
    scheduler: Scheduler,
}

impl Sample {
    fn new(is_hand: bool) -> Self {
        let (sessions, clocks) = if is_hand {
            (Lane::HandReplay { sessions: hand::life::Sessions::default(), out: Vec::new() },
             Clocks::Hand(hand::life::Clocks::default()))
        } else {
            #[cfg(m12_typed)]
            let clocks = Clocks::Pruv(sol::new_state());
            #[cfg(not(m12_typed))]
            let clocks = Clocks::Pruv(World::new());
            (Lane::new(false), clocks)
        };
        Self { sessions, clocks, scheduler: Scheduler { unref: true, ..Scheduler::default() } }
    }

    fn apply(&mut self, request: &Request, log: &mut Vec<i64>) -> Vec<i64> {
        self.scheduler.now = request.now;
        let packet = match (&mut self.clocks, &mut self.sessions) {
            (Clocks::Hand(clocks), Lane::HandReplay { sessions, .. }) => {
                clocks.call(sessions, request.op, &request.hand_rf, &request.hand_sid, &request.hand_arg, request.now)
            }
            #[cfg(m12_typed)]
            (Clocks::Pruv(clocks), Lane::Pruv(world)) => {
                sol::life_decoded(&mut world.st, clocks, request.op, &request.rf, &request.sid,
                                  &request.typed_sid, &request.arg, request.now, &request.typed_keys)
            }
            (clocks, sessions) => {
                clocks.call(sessions, request.op, &request.rf, &request.sid, &request.arg, request.now)
            }
        }.unwrap();
        self.scheduler.apply(0, &request.sid, &packet, log);
        packet
    }
}

fn run_arg() -> Vec<i64> {
    let mut arg = vec![9, 1];
    string_field(&mut arg, "run", "run");
    arg
}

fn prepare(is_hand: bool, item: &str, size: usize) -> (Sample, Vec<Request>) {
    let cleanup = matches!(item, "F" | "D" | "O");
    let target = size / 2;
    let mut sample = Sample::new(is_hand);
    let mut log = Vec::new();
    for i in 0..size {
        let matching = match item {
            "F" | "D" => i % 2 == 0,
            "O" => i == target,
            _ => false,
        };
        let sid = if matching { "owner".to_string() } else { format!("s{i}") };
        let bid = format!("b{i}");
        let initial = if item == "A" && i == target {
            Vec::new()
        } else {
            budget(&bid, if cleanup { (size - i) as i64 } else { 1 }, cleanup && !matching && i % 3 == 0)
        };
        sample.sessions.load(&pkey(&sid), &initial).unwrap();
        if cleanup || (item == "B" && i == target) {
            sample.apply(&Request::new(4, &sid, &bid, run_arg(), NOW), &mut log);
            log.clear();
        }
    }
    if item == "D" {
        for turn in ["a", "b", "c"] {
            for activity in ["activity-1", "activity-2"] {
                assert_eq!(sample.clocks.seed_tool(&pkey(turn), &pkey(activity)), Ok(1));
            }
        }
    }
    let sid = format!("s{target}");
    let bid = format!("b{target}");
    let mut requests = Vec::new();
    let request = match item {
        "F" | "O" => Request::new(2, "owner", "", vec![1], NOW),
        "D" => Request::new(3, "", "", vec![1], NOW),
        "B" => Request::new(1, &sid, "", vec![1], NOW),
        "A" => {
            let mut arg = vec![9, 2];
            string_field(&mut arg, "origin", "user");
            integer_field(&mut arg, "size", 1);
            Request::new(8, &sid, "", arg, NOW)
        }
        "H" => {
            let mut arg = vec![9, 1];
            integer_field(&mut arg, "commands", 1);
            Request::new(9, &sid, &bid, arg, NOW)
        }
        "U" => {
            requests.push(Request::new(4, &sid, &bid, run_arg(), NOW));
            Request::new(6, &sid, &bid, run_arg(), NOW + 1)
        }
        _ => unreachable!(),
    };
    requests.push(request);
    (sample, requests)
}

fn fold(checksum: &mut u64, words: &[i64]) {
    for &word in words {
        *checksum = (*checksum ^ word as u64).wrapping_mul(1_099_511_628_211);
    }
}

pub(super) fn measure(is_hand: bool, item: &str, size: usize, reps: usize) -> String {
    assert!(size > 0 && reps > 0);
    let mut unit = Vec::new();
    limits(&mut unit, 1);
    m12_facts::reset(unit.clone(), None, Vec::new());
    let (sample, requests) = prepare(is_hand, item, size);
    let mut template = Some(sample);
    let mut elapsed = std::time::Duration::ZERO;
    let mut allocs = 0;
    let mut checksum = 14_695_981_039_346_656_037;
    let mut remaining = reps;
    while remaining > 0 {
        let width = remaining.min(WIDTH);
        let mut samples: Vec<_> = (1..width).map(|_| template.as_ref().unwrap().clone()).collect();
        samples.push(if remaining == width { template.take().unwrap() } else { template.as_ref().unwrap().clone() });
        let mut logs = vec![Vec::new(); width];
        let mut packets: Vec<_> = (0..width * requests.len()).map(|_| Vec::new()).collect();
        m12_facts::reset(unit.clone(), None, Vec::new());

        let before = ALLOCS.load(Ordering::Relaxed);
        let start = Instant::now();
        for ((sample, log), output) in samples.iter_mut().zip(&mut logs).zip(packets.chunks_mut(requests.len())) {
            for (request, packet) in requests.iter().zip(output) {
                *packet = black_box(sample.apply(request, log));
            }
        }
        elapsed += start.elapsed();
        allocs += ALLOCS.load(Ordering::Relaxed) - before;

        // Drop each lane after observing it so enlarged dump buffers do not accumulate.
        for ((mut sample, log), outputs) in samples.into_iter().zip(&logs).zip(packets.chunks(requests.len())) {
            fold(&mut checksum, log);
            for packet in outputs {
                fold(&mut checksum, effects(packet).last().unwrap().4);
            }
            fold(&mut checksum, sample.sessions.dump().unwrap());
            fold(&mut checksum, &sample.clocks.observe().unwrap());
            fold(&mut checksum, &[sample.scheduler.active.len() as i64]);
            for (&(due, _), timer) in &sample.scheduler.active {
                fold(&mut checksum, &[due, timer.address.0 as i64, timer.address.1.len() as i64]);
                fold(&mut checksum, &timer.address.1);
            }
        }
        remaining -= width;
    }
    format!("ns {:.1} {} alloc {}", elapsed.as_nanos() as f64 / reps as f64, checksum, allocs)
}

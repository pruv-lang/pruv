// The host owns one generated St and translates the q wire format.
use budget_host::{ResidentState, TypedCore};
use std::io::{self, BufRead, Write};
use sol::pruv_m_t_2_8_bud_keys_12_KeyAdmission as KeyAdmission;

fn empty_state() -> sol::St {
    sol::St {
        sessions: Default::default(),
        next_sessions: 0,
        clocks: Default::default(),
        next_clocks: 0,
        tools: Default::default(),
        next_tools: 0,
    }
}

// Key admission (count, padding, surrogate mapping, UTF-8) is decided in Pruv
// (bud_keys::admit_key); the host only copies the admitted bytes out.
fn take_key(key: &mut KeyAdmission, words: &mut &[i64]) -> Option<String> {
    if sol::admit_key(key, words) != 0 {
        return None;
    }
    let text = std::str::from_utf8(&key.bytes[..key.used as usize]).expect("admitted key is UTF-8");
    *words = &words[key.end as usize..];
    Some(text.to_owned())
}

#[derive(Clone, Copy)]
enum Lane {
    Direct,
    Adapter,
}

impl Lane {
    fn from_env() -> Self {
        match std::env::var("M4_LANE").as_deref() {
            Ok("direct") | Err(std::env::VarError::NotPresent) => Self::Direct,
            Ok("adapter") => Self::Adapter,
            _ => panic!("M4_LANE must be direct or adapter"),
        }
    }
}

enum Op<'a> {
    SessionPut(String, &'a [i64]),
    SessionGet(String),
    SessionRemove(String),
    SessionList,
    ClockPut(String, String),
    ClockRemove(String),
    ClockSet(String, sol::ClockTiming),
    ClockGet(String),
    RunPut(String, String, bool),
    RunRemove(String, String),
    RunList(String),
    ClockList,
    ToolNote(String, String),
    ToolRemove(String),
    ToolCount(String),
    ActivityList(String),
    ToolList,
    LiveCount,
}

// Q input is shared by both lanes. Admission precedes any state mutation.
fn decode(op: &[i64]) -> Option<Op<'_>> {
    let (&code, mut rest) = op.split_first()?;
    let mut key_buf = KeyAdmission { end: 0, used: 0, bytes: Vec::new() };
    let k = &mut key_buf;
    let decoded = match code {
        1 => {
            let key = take_key(k, &mut rest)?;
            let (&declared, blob) = rest.split_first()?;
            return sol::blob_input(declared, blob).then_some(Op::SessionPut(key, blob));
        }
        2 => Op::SessionGet(take_key(k, &mut rest)?),
        3 => Op::SessionRemove(take_key(k, &mut rest)?),
        4 => Op::SessionList,
        5 => Op::ClockPut(take_key(k, &mut rest)?, take_key(k, &mut rest)?),
        6 => Op::ClockRemove(take_key(k, &mut rest)?),
        7 => {
            let key = take_key(k, &mut rest)?;
            return match sol::clock_input(rest) {
                sol::ClockInput::Valid(timing) => Some(Op::ClockSet(key, timing)),
                sol::ClockInput::Invalid => None,
            };
        }
        8 => Op::ClockGet(take_key(k, &mut rest)?),
        9 => {
            let key = take_key(k, &mut rest)?;
            let run = take_key(k, &mut rest)?;
            return match sol::waiting_input(rest) {
                sol::WaitingInput::Valid(waiting) => Some(Op::RunPut(key, run, waiting)),
                sol::WaitingInput::Invalid => None,
            };
        }
        10 => Op::RunRemove(take_key(k, &mut rest)?, take_key(k, &mut rest)?),
        11 => Op::RunList(take_key(k, &mut rest)?),
        12 => Op::ClockList,
        13 => Op::ToolNote(take_key(k, &mut rest)?, take_key(k, &mut rest)?),
        14 => Op::ToolRemove(take_key(k, &mut rest)?),
        15 => Op::ToolCount(take_key(k, &mut rest)?),
        16 => Op::ActivityList(take_key(k, &mut rest)?),
        17 => Op::ToolList,
        18 => Op::LiveCount,
        _ => return None,
    };
    sol::frame_end(rest).then_some(decoded)
}

struct M4;

impl TypedCore for M4 {
    type State = sol::St;
    type Operation<'a> = &'a Op<'a>;
    type Reply = sol::Reply;

    fn apply(&self, st: &mut sol::St, op: &Op<'_>) -> sol::Reply {
        match op {
            Op::SessionPut(key, blob) => sol::session_put_admitted(st, key, blob),
            Op::SessionGet(key) => sol::session_get_admitted(st, key),
            Op::SessionRemove(key) => sol::session_remove_admitted(st, key),
            Op::SessionList => sol::session_list(st),
            Op::ClockPut(key, owner) => sol::clock_put_admitted(st, key, owner),
            Op::ClockRemove(key) => sol::clock_remove_admitted(st, key),
            Op::ClockSet(key, timing) => sol::clock_set_admitted(st, key, *timing),
            Op::ClockGet(key) => sol::clock_get_admitted(st, key),
            Op::RunPut(key, run, waiting) => sol::run_put_admitted(st, key, run, *waiting),
            Op::RunRemove(key, run) => sol::run_remove_admitted(st, key, run),
            Op::RunList(key) => sol::run_list_admitted(st, key),
            Op::ClockList => sol::clock_list(st),
            Op::ToolNote(key, activity) => sol::tool_note_admitted(st, key, activity),
            Op::ToolRemove(key) => sol::tool_remove_admitted(st, key),
            Op::ToolCount(key) => sol::tool_count_admitted(st, key),
            Op::ActivityList(key) => sol::activity_list_admitted(st, key),
            Op::ToolList => sol::tool_list(st),
            Op::LiveCount => sol::live_count(st),
        }
    }
}

type Resident = ResidentState<M4, sol::St>;

fn adapter(st: &mut Resident, packet: &[i64]) -> Option<(&'static str, Vec<i64>)> {
    let op = decode(packet)?;
    encode(&op, st.step_typed(&op))
}

fn wire_step(lane: Lane, st: &mut Resident, packet: &[i64]) -> Option<(&'static str, Vec<i64>)> {
    match lane {
        Lane::Direct => {
            let op = decode(packet)?;
            let reply = st.step_typed(&op);
            encode(&op, reply)
        }
        Lane::Adapter => adapter(st, packet),
    }
}

fn push_key(out: &mut Vec<i64>, key: &str) {
    let count = out.len();
    out.push(0);
    let mut chars = key.chars();
    while let Some(c) = chars.next() {
        if c == '\u{e000}' {
            let escaped = chars.next().expect("admitted key escape") as u32;
            let unit = if escaped == 0xe000 { escaped } else { escaped - 0xe100 + 0xd800 };
            out.push(i64::from(unit));
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                out.push(i64::from(*unit));
            }
        }
    }
    out[count] = (out.len() - count - 1) as i64;
}

fn push_blob(out: &mut Vec<i64>, blob: Vec<i64>) {
    out.push(blob.len() as i64);
    out.extend(blob);
}

fn push_clock(out: &mut Vec<i64>, clock: &sol::ClockView) {
    push_key(out, &clock.owner);
    let timing = clock.timing;
    out.extend([i64::from(timing.present), timing.since, i64::from(timing.armed), timing.deadline]);
}

fn push_runs(out: &mut Vec<i64>, mut runs: Vec<sol::RunView>) {
    runs.sort_unstable_by_key(|run| run.ordinal);
    out.push(runs.len() as i64);
    for run in runs {
        push_key(out, &run.key);
        out.push(i64::from(run.waiting));
    }
}

fn encode(op: &Op<'_>, mut reply: sol::Reply) -> Option<(&'static str, Vec<i64>)> {
    use sol::Outcome::*;
    let mut out = Vec::new();
    match reply.outcome {
        Invalid => return None,
        Changed(changed) => out.push(i64::from(changed)),
        MissingClock => out.push(2),
        Missing => out.push(0),
        Count(count) => out.push(count),
        Found => match op {
            Op::SessionGet(_) => {
                out.push(1);
                push_blob(&mut out, reply.blob);
            }
            Op::ClockGet(_) => {
                let [clock] = reply.clocks.as_slice() else { panic!("expected one clock") };
                out.push(1);
                push_clock(&mut out, clock);
                out.push(clock.run_count);
            }
            Op::RunList(_) => {
                out.push(1);
                push_runs(&mut out, reply.runs);
            }
            Op::SessionList | Op::ClockList | Op::ToolList | Op::ActivityList(_) => {
                reply.names.sort_unstable_by_key(|name| name.ordinal);
                out.push(reply.names.len() as i64);
                for name in reply.names {
                    push_key(&mut out, &name.key);
                }
            }
            _ => panic!("unexpected reply for operation"),
        },
    }
    let prefix = if matches!(op, Op::LiveCount) { "stats" } else { "ok" };
    Some((prefix, out))
}

fn observation(mut reply: sol::Reply) -> Vec<i64> {
    use std::collections::HashMap;
    assert!(matches!(reply.outcome, sol::Outcome::Found));
    let mut out = Vec::new();
    reply.sessions.sort_unstable_by_key(|entry| entry.ordinal);
    reply.clocks.sort_unstable_by_key(|entry| entry.ordinal);
    reply.names.sort_unstable_by_key(|entry| entry.ordinal);
    let mut runs: HashMap<i64, Vec<_>> = HashMap::new();
    for run in reply.runs {
        runs.entry(run.parent).or_default().push(run);
    }
    let mut activities: HashMap<i64, Vec<_>> = HashMap::new();
    for activity in reply.activities {
        activities.entry(activity.parent).or_default().push(activity);
    }

    out.push(reply.sessions.len() as i64);
    for entry in reply.sessions {
        push_key(&mut out, &entry.key);
        push_blob(&mut out, entry.blob);
    }
    out.push(reply.clocks.len() as i64);
    for entry in reply.clocks {
        push_key(&mut out, &entry.key);
        push_clock(&mut out, &entry);
        push_runs(&mut out, runs.remove(&entry.ordinal).unwrap_or_default());
    }
    out.push(reply.names.len() as i64);
    for entry in reply.names {
        push_key(&mut out, &entry.key);
        let mut items = activities.remove(&entry.ordinal).unwrap_or_default();
        items.sort_unstable_by_key(|item| item.ordinal);
        out.push(items.len() as i64);
        for item in items {
            push_key(&mut out, &item.key);
        }
    }
    out
}

fn write_words(out: &mut impl Write, prefix: &str, words: &[i64]) -> io::Result<()> {
    write!(out, "{prefix}")?;
    for word in words {
        write!(out, " {word}")?;
    }
    writeln!(out)
}

fn wire_key(out: &mut Vec<i64>, k: &str) {
    let units: Vec<u16> = k.encode_utf16().collect();
    out.push(units.len() as i64);
    for chunk in units.chunks(4) {
        let mut w = 0u64;
        for i in 0..4 {
            w = (w << 16) | chunk.get(i).copied().unwrap_or(0) as u64;
        }
        out.push(w as i64);
    }
}

fn op_k(code: i64, k: &str) -> Vec<i64> {
    let mut v = vec![code];
    wire_key(&mut v, k);
    v
}

fn op_kk(code: i64, k: &str, o: &str) -> Vec<i64> {
    let mut v = op_k(code, k);
    wire_key(&mut v, o);
    v
}

fn op_session_put(k: &str, blob: &[i64]) -> Vec<i64> {
    let mut v = op_k(1, k);
    v.push(blob.len() as i64);
    v.extend_from_slice(blob);
    v
}

// Same sessions and tools cycles as m4_hand, with prefill and op encoding untimed.
fn perf(lane: Lane, axis: &str, n: usize, reps: usize) {
    use std::hint::black_box;
    let mut state = empty_state();
    let blob = [11, 22, 33, 44, 55, 66, 77, 88];
    let cycle = match axis {
        "sessions" => {
            for i in 0..n {
                sol::session_put(&mut state, &format!("s{i}"), &blob);
            }
            let key = format!("s{}", n / 2);
            vec![Op::SessionGet(key.clone()), Op::SessionPut(key, &blob)]
        }
        "tools" => {
            for i in 0..n {
                sol::tool_note(&mut state, "t0", &format!("a{i}"));
            }
            let mut cycle = Vec::new();
            for r in 0..n.max(1).min(reps.max(1)) {
                cycle.push(Op::ToolNote("t0".into(), format!("a{r}")));
                cycle.push(Op::ToolNote("x".into(), "y".into()));
                cycle.push(Op::ToolRemove("x".into()));
            }
            cycle
        }
        _ => panic!("unknown axis {axis}"),
    };
    let width = if axis == "tools" { 3 } else { 2 };
    let nops = (reps * width).max(1);

    fn measure<T>(cycle: &[T], nops: usize, mut step: impl FnMut(&T) -> i64) -> u128 {
        let t0 = std::time::Instant::now();
        let mut sink = 0i64;
        let mut j = 0;
        for _ in 0..nops {
            sink = sink.wrapping_add(black_box(step(black_box(&cycle[j]))));
            j += 1;
            if j == cycle.len() {
                j = 0;
            }
        }
        black_box(sink);
        t0.elapsed().as_nanos()
    }

    let (label, total) = match lane {
        Lane::Direct => ("direct", measure(&cycle, nops, |op| {
            let reply = match op {
                Op::SessionGet(key) => sol::session_get(&state, key),
                Op::SessionPut(key, data) => sol::session_put(&mut state, key, data),
                Op::ToolNote(key, activity) => sol::tool_note(&mut state, key, activity),
                Op::ToolRemove(key) => sol::tool_remove(&mut state, key),
                _ => unreachable!(),
            };
            match reply.outcome {
                sol::Outcome::Changed(value) => i64::from(value) + 1,
                sol::Outcome::Found => reply.blob.len() as i64 + 3,
                _ => panic!("perf: op failed"),
            }
        })),
        Lane::Adapter => {
            let mut st = Resident::from_typed(M4, state);
            let packets: Vec<_> = cycle.iter().map(|op| match op {
                Op::SessionGet(key) => op_k(2, key),
                Op::SessionPut(key, data) => op_session_put(key, data),
                Op::ToolNote(key, activity) => op_kk(13, key, activity),
                Op::ToolRemove(key) => op_k(14, key),
                _ => unreachable!(),
            }).collect();
            ("adapter", measure(&packets, nops, |packet| {
                let (_, words) = adapter(&mut st, packet).expect("perf: op failed");
                words.len() as i64 + words[0]
            }))
        }
    };
    println!("lane={label} axis={axis} n={n} core_ns_per_op={:.1} ops={nops}", total as f64 / nops as f64);
}

fn churn() {
    let mut st = empty_state();
    let apply = |st: &mut sol::St, op: &Op<'_>, expected| {
        assert!(matches!(M4.apply(st, op).outcome, sol::Outcome::Changed(value) if value == expected));
    };
    let keep = Op::ToolNote("t0".into(), "a0".into());
    let create = Op::ToolNote("x".into(), "y".into());
    let delete = Op::ToolRemove("x".into());
    apply(&mut st, &keep, true);
    let capacity_bound = st.tools.capacity() + 16;
    let mut peak = st.tools.capacity();
    for _ in 0..200_000 {
        apply(&mut st, &keep, false);
        apply(&mut st, &create, true);
        peak = peak.max(st.tools.capacity());
        apply(&mut st, &delete, true);
        assert_eq!(st.tools.len(), 1);
        assert_eq!(st.tools.get("t0").map(|tool| tool.ordinal), Some(0));
        assert!(peak <= capacity_bound);
    }
    assert_eq!(st.next_tools, 200_001);
    println!("m4typed: churn 200000 deletions, live=1 peak_capacity={peak} final_capacity={} bound={capacity_bound}",
             st.tools.capacity());
}

fn malformed(lane: Lane) {
    use sol::{ClockInput, Outcome, WaitingInput};

    assert!(matches!(sol::clock_input(&[1, 0, 0]), ClockInput::Invalid));
    assert!(matches!(sol::clock_input(&[2, 0, 0, 0]), ClockInput::Invalid));
    assert!(matches!(sol::clock_input(&[1, 0, -1, 0]), ClockInput::Invalid));
    assert!(matches!(sol::waiting_input(&[0, 1]), WaitingInput::Invalid));
    assert!(matches!(sol::waiting_input(&[-1]), WaitingInput::Invalid));
    assert!(!sol::blob_input(-1, &[]));
    assert!(!sol::blob_input(2, &[0]));
    assert!(sol::blob_input(1, &[0]));

    // Packed keys: Pruv admission rejects a negative or over-2^30 count, a count past the
    // remaining words, and nonzero padding; a lone surrogate maps to its own scalar.
    let mut key = KeyAdmission { end: 0, used: 0, bytes: Vec::new() };
    let mut at = |words: &[i64]| { let mut w = words; take_key(&mut key, &mut w).map(|s| (s, w.len())) };
    assert_eq!(at(&[-1]), None);
    assert_eq!(at(&[(1 << 30) + 1, 0]), None);
    assert_eq!(at(&[5, 0]), None);
    assert_eq!(at(&[1, 0x0061_0000_0000_0001]), None);
    assert_eq!(at(&[2, 0x0061_0062_0000_0000, 7]), Some(("ab".into(), 1)));
    assert_eq!(at(&[1, 0xd800_0000_0000_0000u64 as i64]), Some(("\u{e000}\u{e100}".into(), 0)));
    assert!(decode(&[2, 5, 0x0061_0000_0000_0000]).is_none());
    assert!(decode(&[2, 1, 0x0061_0000_0000_0000, 9]).is_none());

    let mut st = empty_state();
    assert!(matches!(sol::run_put(&mut st, "clock", "run", true).outcome, Outcome::MissingClock));
    assert!(matches!(sol::clock_put(&mut st, "clock", "owner").outcome, Outcome::Changed(true)));
    assert!(matches!(sol::run_put(&mut st, "clock", "run", true).outcome, Outcome::Changed(true)));
    assert!(matches!(sol::clock_remove(&mut st, "clock").outcome, Outcome::Changed(true)));
    assert!(matches!(sol::run_put(&mut st, "clock", "run", true).outcome, Outcome::MissingClock));
    assert!(matches!(sol::clock_put(&mut st, "clock", "owner").outcome, Outcome::Changed(true)));
    assert!(sol::run_list(&st, "clock").runs.is_empty());
    assert!(matches!(sol::run_put(&mut st, "clock", "run", false).outcome, Outcome::Changed(true)));
    assert!(matches!(sol::run_put(&mut st, "clock", "run", true).outcome, Outcome::Changed(false)));

    let mut st = empty_state();
    assert!(matches!(sol::session_put(&mut st, "😀", &[42]).outcome, Outcome::Changed(true)));
    let mut resident = Resident::from_typed(M4, st);
    assert_eq!(wire_step(lane, &mut resident, &op_k(2, "😀")), Some(("ok", vec![1, 1, 42])));
    assert_eq!(wire_step(lane, &mut resident, &op_session_put("😀", &[43])), Some(("ok", vec![0])));
    assert_eq!(sol::session_get(resident.typed_state(), "😀").blob, [43]);

    let surrogate_word = 0xd800_0000_0000_0000_u64 as i64;
    assert_eq!(wire_step(lane, &mut resident, &[1, 1, surrogate_word, 1, 44]), Some(("ok", vec![1])));
    assert_eq!(wire_step(lane, &mut resident, &[2, 1, surrogate_word]), Some(("ok", vec![1, 1, 44])));
    assert_eq!(wire_step(lane, &mut resident, &op_session_put("\u{e000}\u{e100}", &[45])), Some(("ok", vec![1])));
    assert_eq!(sol::session_get(resident.typed_state(), "\u{e000}\u{e100}").blob, [45]);
    assert_eq!(wire_step(lane, &mut resident, &[4]), Some(("ok", vec![3, 2, 0xd83d, 0xde00, 1, 0xd800, 2, 0xe000, 0xe100])));

    let mut remove = op_k(3, "😀");
    remove.push(9);
    assert_eq!(wire_step(lane, &mut resident, &remove), None);
    let mut note = op_kk(13, "tool", "activity");
    note.push(9);
    assert_eq!(wire_step(lane, &mut resident, &note), None);
    assert_eq!(wire_step(lane, &mut resident, &[4, 9]), None);
    assert_eq!(wire_step(lane, &mut resident, &op_k(2, "😀")), Some(("ok", vec![1, 1, 43])));
    assert_eq!(wire_step(lane, &mut resident, &op_k(15, "tool")), Some(("ok", vec![0])));
    println!("m4typed: typed admission and stale/reused keys passed");
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let lane = Lane::from_env();
    if args.get(1).map(String::as_str) == Some("perf") {
        perf(lane, &args[2], args[3].parse().unwrap(), args[4].parse().unwrap());
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("churn") {
        churn();
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("malformed") {
        malformed(lane);
        return Ok(());
    }
    assert_eq!(args.get(1).map(String::as_str), Some("q"),
               "usage: driver_m4_typed q | churn | malformed | perf AXIS N REPS");
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut state = None;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("init") => {
                let _: usize = fields.next().expect("init needs L").parse().unwrap();
                state = Some(Resident::from_typed(M4, empty_state()));
                writeln!(out, "init ok")?;
            }
            Some("op") => {
                let op: Vec<i64> = fields.map(|word| word.parse().expect("bad word")).collect();
                let st = state.as_mut().expect("op before init");
                let result = wire_step(lane, st, &op);
                match result {
                    Some((prefix, words)) => write_words(&mut out, prefix, &words)?,
                    _ => writeln!(out, "invalid")?,
                }
            }
            Some("observe") => {
                let st = state.as_ref().expect("observe before init");
                let words = observation(sol::m4_observe(st.typed_state()));
                write_words(&mut out, &format!("observe {}", words.len()), &words)?;
            }
            None => {}
            Some(cmd) => panic!("unknown command {cmd}"),
        }
    }
    out.flush()
}

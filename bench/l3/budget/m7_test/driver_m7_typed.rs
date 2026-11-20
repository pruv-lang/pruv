use record_text::{parse_put, unit_record};
use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

use sol::pruv_m_t_2_10_bud_tstate_6_Budget as Budget;
use sol::pruv_m_t_2_10_bud_tstate_10_ClockState as ClockState;
use sol::pruv_m_t_2_10_bud_tstate_6_Limits as Limits;
use sol::pruv_m_t_2_10_bud_tstate_3_Out as Out;
use sol::pruv_m_t_2_10_bud_tstate_5_Reply as Reply;

// Q codec follows the M10b typed lane's JavaScript text encoding.
fn take_key(words: &mut &[i64]) -> Option<String> {
    let (&n, packed) = words.split_first()?;
    if !(0..=1 << 30).contains(&n) { return None; }
    let n = n as usize;
    let count = n.div_ceil(4);
    let data = packed.get(..count)?;
    let remainder = n % 4;
    if remainder != 0 && (data[count - 1] as u64) & (u64::MAX >> (16 * remainder)) != 0 {
        return None;
    }
    let units = (0..n).map(|i| (data[i / 4] >> (48 - 16 * (i % 4))) as u16);
    *words = &packed[count..];
    Some(bud_facts::encode_js_text(units))
}

fn append_text(words: &mut Vec<i64>, text: &str) {
    words.push(bud_facts::decode_js_text(text).into_iter().count() as i64);
    words.extend(bud_facts::decode_js_text(text).into_iter().map(i64::from));
}

fn take_text(words: &mut &[i64]) -> Option<String> {
    let (&n, tail) = words.split_first()?;
    let n = usize::try_from(n).ok()?;
    let units = tail.get(..n)?.iter().map(|&u| u16::try_from(u).ok()).collect::<Option<Vec<_>>>()?;
    *words = &tail[n..];
    Some(bud_facts::encode_js_text(units))
}

// Wire decode only: a record the typed Budget cannot represent (an empty or short blob, a malformed tail) is
// refused at the Q boundary with X 3 instead of panicking.
fn read_budget(words: &[i64]) -> Option<Budget> {
    let empty = [0; 30];
    let absent = words == [-1];
    let words = if absent { &empty[..] } else { words };
    let mut tail = words.get(24..)?;
    let id = take_key(&mut tail)?;
    let opened_at = take_text(&mut tail)?;
    let since_text = take_text(&mut tail)?;
    let ended_at = take_text(&mut tail)?;
    let follows = take_key(&mut tail)?;
    let mut halt_reason = -1;
    let mut halt_kind = 0;
    let mut halt_ms = 0;
    let mut halt_text = String::new();
    let mut stopped = Vec::new();
    let mut rest = tail.get(1..)?;
    if rest.first() == Some(&-2) {
        let head = rest.get(1..4)?;
        halt_reason = head[0];
        halt_kind = head[1];
        halt_ms = head[2];
        rest = &rest[4..];
        halt_text = take_text(&mut rest)?;
        let (&count, more) = rest.split_first()?;
        rest = more;
        for _ in 0..count {
            stopped.push(take_key(&mut rest)?);
        }
    }
    let rest = bud_facts::encode_js_text(rest.iter().map(|&u| u as u16));
    Some(Budget {
        id, state: if absent { -1 } else { words[0] }, origin: words[1],
        unlimited: words[2] == 1, unlimited_kind: words[2],
        limits: Limits { time: words[3], delegations: words[4], commands: words[5], reviews: words[6], checks: words[7] },
        active_ms: words[8], delegations: words[9], commands: words[10], reviews: words[11], checks: words[12],
        tools: words[13], failures: words[14], retries: words[15], since_kind: words[16],
        active_since: words[17],
        ended_how: words[18], ended_kind: words[19], metadata: words[20],
        opened_ms: words[21], ended_ms: words[22], follows_kind: words[23],
        opened_at, since_text, ended_at, follows, rest, halt_reason, halt_kind, halt_ms, halt_text, stopped,
    })
}

fn budget_words(b: &Budget) -> Vec<i64> {
    if b.state == -1 { return vec![-1]; }
    let mut words = vec![
        b.state, b.origin, b.unlimited_kind, b.limits.time, b.limits.delegations, b.limits.commands,
        b.limits.reviews, b.limits.checks, b.active_ms, b.delegations, b.commands, b.reviews,
        b.checks, b.tools, b.failures, b.retries, b.since_kind, b.active_since, b.ended_how,
        b.ended_kind, b.metadata, b.opened_ms, b.ended_ms, b.follows_kind,
    ];
    words.extend(key(&b.id));
    append_text(&mut words, &b.opened_at);
    append_text(&mut words, &b.since_text);
    append_text(&mut words, &b.ended_at);
    words.extend(key(&b.follows));
    let start = words.len();
    words.push(0);
    if b.halt_kind != 0 {
        words.extend([-2, b.halt_reason, b.halt_kind, b.halt_ms]);
        append_text(&mut words, &b.halt_text);
        words.push(b.stopped.len() as i64);
        for id in &b.stopped { words.extend(key(id)); }
    }
    words.extend(bud_facts::decode_js_text(&b.rest).into_iter().map(i64::from));
    words[start] = (words.len() - start - 1) as i64;
    words
}

fn key(s: &str) -> Vec<i64> {
    let u: Vec<u16> = bud_facts::decode_js_text(s);
    let mut v = vec![u.len() as i64];
    for c in u.chunks(4) {
        let mut x: u64 = 0;
        for l in 0..4 {
            x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
        }
        v.push(x as i64);
    }
    v
}

// Typed M7 driver. Record serialization is confined to the Q wire and untimed checksums.
mod sol {
    pub use crate::generated::*;

    // Q put handles point into that input line's text bytes.
    pub fn t_text(input: &[u8], offset: i64, count: i64, out: &mut [u8]) -> i64 {
        let start = offset as usize;
        out.copy_from_slice(&input[start..start + count as usize]);
        0
    }
}

struct Call {
    code: i64,
    origin: i64,
    size: i64,
    unit: [i64; 6],
    sid: String,
    bid: String,
}

impl Call {
    // Wire decode only: a malformed call line yields None and the driver answers X 2.
    fn decode(words: &[i64]) -> Option<Self> {
        let mut keys = words.get(9..)?;
        let sid = take_key(&mut keys)?;
        let bid = take_key(&mut keys)?;
        if !keys.is_empty() { return None; }
        Some(Self {
            code: words[0], origin: words[1], size: words[2],
            unit: words[3..9].try_into().ok()?, sid, bid,
        })
    }

    fn dispatch(&self, st: &mut sol::St, uuid: &str, now: i64) -> Reply {
        sol::t_call(st, &self.sid, &self.bid, uuid, self.code, now, self.origin, self.size, &self.unit)
    }

    fn result_words(&self, st: &sol::St, reply: &Reply) -> Vec<i64> {
        let mut words = vec![reply.value];
        if reply.value == 3 {
            let budget = &st.sessions[&self.sid];
            append_text(&mut words, &budget.id);
            if reply.a == 1 {
                append_text(&mut words, &budget.follows);
            }
        } else if reply.value == 4 {
            let record = budget_words(&st.sessions[&self.sid]);
            words.push(record.len() as i64);
            words.extend(record);
        }
        words
    }
}

fn new_state() -> sol::St {
    sol::St {
        sessions: HashMap::new(),
        ordinals: HashMap::new(),
        next_session: 0,
        clock_state: ClockState { clocks: HashMap::new(), next_clock: 0 },
        out: Out { fx: Vec::new() },
    }
}

fn nums(tokens: &[&str]) -> Vec<i64> {
    tokens.iter().map(|token| token.parse().unwrap()).collect()
}

fn write_words(out: &mut impl Write, label: &str, words: &[i64]) {
    write!(out, "{label}").unwrap();
    for word in words {
        write!(out, " {word}").unwrap();
    }
    writeln!(out).unwrap();
}

fn run_q() {
    let mut st = new_state();
    let mut now = 1700000000000;
    let mut uuids = Vec::new();
    let mut next_uuid = 0;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let tokens: Vec<_> = line.split_whitespace().collect();
        let Some(command) = tokens.first() else { continue; };
        match *command {
            "case" => {
                st = new_state();
                next_uuid = 0;
                uuids.clear();
                bud_facts::record_queries();
                bud_facts::set_answers(&[], 0);
                bud_facts::set_calls(0);
                writeln!(out, "case {}", tokens[1]).unwrap();
            }
            "now" => now = tokens[1].parse().unwrap(),
            "adv" => now += tokens[1].parse::<i64>().unwrap(),
            "quiet" => bud_facts::set_answers(&nums(&tokens[1..]), 0),
            "uuids" => {
                let values = nums(&tokens[1..]);
                let mut tail = values.as_slice();
                while !tail.is_empty() {
                    uuids.push(take_key(&mut tail).unwrap());
                }
            }
            "put" => {
                let put = parse_put(&tokens[1..]);
                let mut tail = put.words.as_slice();
                let sid = take_key(&mut tail).unwrap();
                assert_eq!(tail[0] as usize, tail.len() - 1);
                let Some(budget) = read_budget(&unit_record(&put.text, &tail[1..])) else {
                    writeln!(out, "X 3").unwrap();
                    continue;
                };
                let status = sol::t_put(&mut st, &sid, &budget);
                if status != 0 {
                    writeln!(out, "X {status}").unwrap();
                }
            }
            "del" => {
                let values = nums(&tokens[1..]);
                sol::t_del(&mut st, &take_key(&mut values.as_slice()).unwrap());
            }
            "call" => {
                writeln!(out, "@ {}", tokens[1]).unwrap();
                let Some(call) = Call::decode(&nums(&tokens[2..])) else {
                    writeln!(out, "X 2").unwrap();
                    continue;
                };
                let before = bud_facts::calls();
                let uuid = uuids.get(next_uuid).map_or("", String::as_str);
                let reply = call.dispatch(&mut st, uuid, now);
                for i in before..bud_facts::calls() {
                    writeln!(out, "{}", bud_facts::quiet_log(i)).unwrap();
                }
                if reply.status != 0 {
                    writeln!(out, "X {}", reply.status).unwrap();
                    continue;
                }
                for effect in &st.out.fx {
                    let line = match effect.tag {
                        13 => "E clear",
                        20 => "E changed",
                        21 => "E carry",
                        22 => {
                            next_uuid += 1;
                            "E uuid"
                        }
                        tag => panic!("unexpected effect {tag}"),
                    };
                    writeln!(out, "{line}").unwrap();
                }
                write_words(&mut out, "V", &call.result_words(&st, &reply));
            }
            "end" => {
                let mut keys: Vec<_> = st.ordinals.iter().collect();
                keys.sort_unstable_by_key(|(_, ordinal)| ordinal.value);
                let mut words = vec![keys.len() as i64];
                for (sid, _) in keys {
                    append_text(&mut words, sid);
                    let record = budget_words(&st.sessions[sid]);
                    words.push(record.len() as i64);
                    words.extend(record);
                }
                write_words(&mut out, "O", &words);
                writeln!(out, "end").unwrap();
            }
            other => panic!("unexpected Q command {other}"),
        }
    }
}

const NOW: i64 = 1700000000000;
const UUID: &str = "00000000-0000-4000-8000-0000000000aa";
const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn perf_budget(bid: &str, state: i64) -> Budget {
    let mut words = vec![0; 24];
    words[0] = state;
    words[3..8].copy_from_slice(&[120, 24, 3, 2, 1]);
    words[20] = 1;
    words.extend(key(bid));
    append_text(&mut words, "2023-11-14T22:13:20.000Z");
    words.extend([0, 0, 0]);
    append_text(&mut words, PERF_REST);
    read_budget(&words).unwrap()
}

fn fold(call: &Call, st: &sol::St, reply: &Reply, sum: u64) -> u64 {
    let result = if reply.status == 0 { call.result_words(st, reply) } else { Vec::new() };
    [reply.status, st.out.fx.len() as i64].into_iter()
        .chain(st.out.fx.iter().map(|effect| effect.tag))
        .chain(result)
        .fold(sum, |sum, word| sum.wrapping_mul(31).wrapping_add(word as u64))
}

fn run_perf(args: &[String]) {
    let size: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let sids: Vec<_> = (0..size).map(|i| format!("s{i:04}")).collect();
    let bid = |i: usize| format!("{i:08x}-0000-4000-8000-000000000001");
    let last = size - 1;
    let unit = [120, 24, 3, 2, 1, 0];
    let make_call = |i: usize, code, origin, size, with_bid| Call {
        code, origin, size, unit, sid: sids[i].clone(),
        bid: if with_bid { bid(i) } else { String::new() },
    };
    let setup = |state| {
        let mut st = new_state();
        for (i, sid) in sids.iter().enumerate() {
            assert_eq!(sol::t_put(&mut st, sid, &perf_budget(&bid(i), state)), 0);
        }
        st
    };
    bud_facts::set_answers(&[], 1);
    bud_facts::set_calls(0);
    let verify_reps = reps.min(2000);
    for (item, calls) in [
        ("ishalted", vec![make_call(last, 2, -1, -1, true)]),
        ("current", vec![make_call(last, 1, -1, -1, true)]),
        ("attach", vec![make_call(last, 3, 0, -1, false)]),
        ("close", (0..size).map(|i| make_call(i, 6, -1, -1, false)).collect()),
        ("closereopen", vec![make_call(last, 6, -1, -1, false), make_call(last, 5, -1, -1, true)]),
        ("reopenmiss", vec![make_call(last, 5, -1, -1, true)]),
        ("replace", vec![make_call(last, 4, 0, 2, false)]),
    ] {
        let replace = item == "replace";
        let state = i64::from(replace);
        let mut st = setup(state);
        let halted = perf_budget(&bid(last), 1);
        let restore = |st: &mut sol::St| {
            if replace {
                // Restore the benchmark fixture here, including the same reset in the timed pass.
                *st.sessions.get_mut(black_box(sids[last].as_str())).unwrap() = halted.clone();
            }
        };
        let (elapsed, count, checksum);
        if item == "close" {
            let open = st.sessions.clone();
            for call in &calls {
                black_box(call.dispatch(&mut st, UUID, NOW));
            }
            let mut time = std::time::Duration::ZERO;
            for start in (0..reps).step_by(size) {
                st.sessions.clone_from(&open);
                let timer = Instant::now();
                for call in &calls[..size.min(reps - start)] {
                    let reply = black_box(call).dispatch(&mut st, UUID, NOW);
                    black_box(reply);
                }
                time += timer.elapsed();
            }
            let mut sum = 0;
            for start in (0..verify_reps).step_by(size) {
                st.sessions.clone_from(&open);
                for call in &calls[..size.min(verify_reps - start)] {
                    let reply = call.dispatch(&mut st, UUID, NOW);
                    sum = fold(call, &st, &reply, sum);
                }
            }
            (elapsed, count, checksum) = (time, reps, sum);
        } else {
            for _ in 0..1000 {
                for call in &calls {
                    black_box(call.dispatch(&mut st, UUID, NOW));
                }
                restore(&mut st);
            }
            let timer = Instant::now();
            for _ in 0..reps {
                for call in &calls {
                    let reply = black_box(call).dispatch(&mut st, UUID, NOW);
                    black_box(reply);
                }
                restore(&mut st);
            }
            elapsed = timer.elapsed();
            count = reps * calls.len();
            let mut verified = setup(state);
            let mut sum = 0;
            for _ in 0..verify_reps {
                for call in &calls {
                    let reply = call.dispatch(&mut verified, UUID, NOW);
                    sum = fold(call, &verified, &reply, sum);
                }
                restore(&mut verified);
            }
            checksum = sum;
        }
        println!("{item} {size} {:.1}", elapsed.as_nanos() as f64 / count as f64);
        println!("rchk {item} {size} {checksum}");
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "q" => run_q(),
        "perf" => run_perf(&args[1..]),
        other => panic!("unexpected mode {other}"),
    }
}

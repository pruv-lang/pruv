// M10 typed host: Q wire codec, fact staging, and timer/effect I/O.
use record_text::{parse_put, perf_put, unit_record, Put};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

use sol::pruv_m_t_2_10_bud_tstate_6_Budget as Budget;
use sol::pruv_m_t_2_10_bud_tstate_10_ClockState as ClockState;
use sol::pruv_m_t_2_10_bud_tstate_6_Limits as Limits;
use sol::pruv_m_t_2_10_bud_tstate_3_Out as Out;

struct Timer {
    token: u64,
    due: i64,
    sid: String,
    bid: String,
}

struct World {
    st: sol::St,
    now: i64,
    seq: u64,
    active: Vec<Timer>,
    by_bid: HashMap<String, u64>,
    log: Vec<String>,
    silent: bool,
}

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

fn unit_list(text: &str) -> String {
    bud_facts::decode_js_text(text).into_iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",")
}

fn append_text(words: &mut Vec<i64>, text: &str) {
    let units = bud_facts::decode_js_text(text);
    words.push(units.len() as i64);
    words.extend(units.into_iter().map(i64::from));
}

fn take_text(words: &mut &[i64]) -> String {
    let n = usize::try_from(words[0]).unwrap();
    let units: Vec<_> = words[1..1 + n].iter().map(|&u| u16::try_from(u).unwrap()).collect();
    *words = &words[1 + n..];
    bud_facts::encode_js_text(units)
}

fn read_budget(words: &[i64]) -> Budget {
    let empty = [0; 30];
    let absent = words == [-1];
    let words = if absent { &empty[..] } else { words };
    let mut tail = &words[24..];
    let id = take_key(&mut tail).unwrap();
    let opened_at = take_text(&mut tail);
    let since_text = take_text(&mut tail);
    let ended_at = take_text(&mut tail);
    let follows = take_key(&mut tail).unwrap();
    let mut halt_reason = -1;
    let mut halt_kind = 0;
    let mut halt_ms = 0;
    let mut halt_text = String::new();
    let mut stopped = Vec::new();
    let mut rest = &tail[1..];
    if rest.first() == Some(&-2) {
        halt_reason = rest[1];
        halt_kind = rest[2];
        halt_ms = rest[3];
        rest = &rest[4..];
        halt_text = take_text(&mut rest);
        let count = rest[0];
        rest = &rest[1..];
        for _ in 0..count {
            stopped.push(take_key(&mut rest).unwrap());
        }
    }
    let rest = bud_facts::encode_js_text(rest.iter().map(|&u| u as u16));
    Budget {
        id, state: if absent { -1 } else { words[0] }, origin: words[1],
        unlimited: words[2] == 1, unlimited_kind: words[2],
        limits: Limits { time: words[3], delegations: words[4], commands: words[5], reviews: words[6], checks: words[7] },
        active_ms: words[8], delegations: words[9], commands: words[10], reviews: words[11], checks: words[12],
        tools: words[13], failures: words[14], retries: words[15], since_kind: words[16],
        active_since: if words[16] == 2 { words[17] } else { -1 },
        ended_how: words[18], ended_kind: words[19], metadata: words[20],
        opened_ms: words[21], ended_ms: words[22], follows_kind: words[23],
        opened_at, since_text, ended_at, follows, rest, halt_reason, halt_kind, halt_ms, halt_text, stopped,
    }
}

fn decode_put(put: &Put) -> (String, Budget) {
    let mut record = put.words.as_slice();
    let sid = take_key(&mut record).unwrap();
    assert_eq!(record[0] as usize, record.len() - 1);
    (sid, read_budget(&unit_record(&put.text, &record[1..])))
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

impl World {
    fn new() -> Self {
        Self {
            st: sol::St {
                sessions: HashMap::new(),
                clock_state: ClockState { clocks: HashMap::new(), next_clock: 0 },
                out: Out { fx: Vec::new() },
                session_ordinals: HashMap::new(),
                next_session: 0,
                charge_reason: String::new(),
            },
            now: 1700000000000, seq: 0, active: Vec::new(), by_bid: HashMap::new(),
            log: Vec::new(), silent: false,
        }
    }

    fn put(&mut self, put: &Put) {
        let (sid, budget) = decode_put(put);
        assert_eq!(sol::t_put(&mut self.st, &sid, &budget), 0);
    }

    fn note(&mut self, line: String) {
        if !self.silent { self.log.push(line); }
    }

    fn call(&mut self, body: &[i64]) -> i64 {
        let keys = body.get(12..).and_then(|mut keys| {
            let sid = take_key(&mut keys)?;
            let bid = take_key(&mut keys)?;
            let run = take_key(&mut keys)?;
            keys.is_empty().then_some((sid, bid, run))
        });
        let Some((sid, bid, run)) = keys else {
            self.note("X 2".to_owned());
            return 2;
        };
        self.call_typed(body[0], body[11], &sid, &bid, &run, &body[1..11])
    }

    fn call_typed(&mut self, code: i64, flag: i64, sid: &str, bid: &str, run: &str, what: &[i64]) -> i64 {
        let quiet_before = bud_facts::calls();
        let running_before = bud_facts::running_calls();
        let reply = sol::t_call(&mut self.st, sid, bid, run, code, self.now, flag, what);
        let mut running = running_before;
        let effects = &self.st.out.fx;
        let close_bid = (code == 6 && effects.iter().any(|e| matches!(e.tag, 12..=14)))
            .then(|| self.st.sessions[sid].id.clone());
        let timer_bid = close_bid.as_deref().unwrap_or(bid);
        let mut stopped = None;
        for i in 0..=effects.len() {
            let i = i as i64;
            if !self.silent && running < bud_facts::running_calls() {
                let q = bud_facts::running_query(running);
                if q.fxn == i {
                    if self.by_bid.contains_key(bid) { self.log.push("X armed".to_owned()); }
                    let queried_sid = take_key(&mut q.sid.as_slice()).unwrap();
                    let queried_bid = take_key(&mut q.bid.as_slice()).unwrap();
                    self.log.push(format!("E rdi s:{} b:{}", unit_list(&queried_sid), unit_list(&queried_bid)));
                    running += 1;
                }
            }
            if i == 0 && !self.silent {
                for q in quiet_before..bud_facts::calls() { self.log.push(bud_facts::quiet_log(q)); }
            }
            if let Some(effect) = effects.get(i as usize) {
                let line = match effect.tag {
                    12 => {
                        self.seq += 1;
                        self.active.push(Timer { token: self.seq, due: self.now + effect.b,
                            sid: sid.to_owned(), bid: timer_bid.to_owned() });
                        self.by_bid.insert(timer_bid.to_owned(), self.seq);
                        (!self.silent).then(|| format!("E set t{} {} {}", self.seq, effect.a, effect.b))
                    }
                    13 => {
                        let token = self.by_bid.remove(timer_bid);
                        if let Some(token) = token { self.active.retain(|t| t.token != token); }
                        (!self.silent).then(|| match token {
                            Some(token) => format!("E clear t{token}"),
                            None => "E clear -".to_owned(),
                        })
                    }
                    14 => { self.by_bid.remove(timer_bid); None }
                    33 if self.silent => {
                        let ids = stopped.get_or_insert_with(|| &self.st.sessions[sid].stopped);
                        black_box(&ids[effect.a as usize]);
                        None
                    }
                    _ if self.silent => None,
                    20 => Some("E changed".to_owned()),
                    31 => Some("E cancel".to_owned()),
                    32 => Some("E stopMain".to_owned()),
                    33 => {
                        let ids = stopped.get_or_insert_with(|| &self.st.sessions[sid].stopped);
                        Some(format!("E stopDel {}", unit_list(&ids[effect.a as usize])))
                    }
                    34 => Some("E stopTurns".to_owned()),
                    35 => Some("E hold".to_owned()),
                    36 => Some("E save".to_owned()),
                    37 => Some("E announce".to_owned()),
                    tag => panic!("unexpected effect {tag}"),
                };
                if let Some(line) = line { self.log.push(line); }
            }
        }
        if reply.status != 0 {
            if !self.silent { self.log.push(format!("X {}", reply.status)); }
        }
        else if !self.silent {
            let value = match reply.value {
                4 => {
                    let words = budget_words(&self.st.sessions[sid]);
                    format!("V 4 {} {}", words.len(), words.iter().map(i64::to_string).collect::<Vec<_>>().join(" "))
                }
                5 => {
                    let reason = if reply.a == 1 {
                        ["time", "delegations", "commands", "reviews", "checks"]
                            .iter().position(|&s| s == self.st.charge_reason).unwrap()
                    } else { 0 };
                    format!("V 5 {} {reason}", reply.a)
                }
                x => format!("V {x}"),
            };
            self.log.push(value);
        }
        reply.status
    }

    fn fire(&mut self, index: usize) {
        let timer = self.active.remove(index);
        if !self.silent { self.log.push(format!("F t{}", timer.token)); }
        self.call_typed(11, 0, &timer.sid, &timer.bid, "", &[0; 10]);
        if self.log.last().is_some_and(|line| line.starts_with("V ")) { self.log.pop(); }
    }

    fn observe(&mut self) -> String {
        let mut observation = sol::t_observe(&mut self.st);
        let mut words = vec![observation.sessions.len() as i64];
        observation.sessions.sort_unstable_by_key(|sid| sol::t_get(&mut self.st, sid));
        for sid in &observation.sessions {
            append_text(&mut words, sid);
            let budget = budget_words(&self.st.sessions[sid]);
            words.push(budget.len() as i64);
            words.extend(budget);
        }
        words.push(observation.clocks.len() as i64);
        observation.clocks.sort_unstable_by_key(|bid| self.st.clock_state.clocks[bid].ordinal);
        for bid in &observation.clocks {
            let clock = &self.st.clock_state.clocks[bid];
            append_text(&mut words, bid);
            append_text(&mut words, &clock.owner);
            words.extend([i64::from(clock.has_since), clock.since, i64::from(clock.armed), clock.active]);
            words.push(clock.runs.len() as i64);
            let mut runs: Vec<_> = clock.runs.iter().collect();
            runs.sort_unstable_by_key(|(_, seq)| seq.abs());
            for (run, seq) in runs {
                append_text(&mut words, run);
                words.push(i64::from(*seq < 0));
            }
        }
        let mut line = String::from("O");
        for word in words { write!(&mut line, " {word}").unwrap(); }
        line.push_str(" | T");
        for t in &self.active {
            let own = self.by_bid.get(&t.bid) == Some(&t.token);
            write!(&mut line, " t{}:{}:{}:{}", t.token, t.due, unit_list(&t.bid), i32::from(own)).unwrap();
        }
        line
    }
    fn advance(&mut self, ms: i64) {
        let target = self.now + ms;
        loop {
            let mut best: Option<usize> = None;
            for (i, t) in self.active.iter().enumerate() {
                if t.due <= target {
                    match best {
                        None => best = Some(i),
                        Some(b) => {
                            let bt = &self.active[b];
                            if t.due < bt.due || (t.due == bt.due && t.token < bt.token) {
                                best = Some(i)
                            }
                        }
                    }
                }
            }
            match best {
                None => break,
                Some(i) => {
                    self.now = self.active[i].due;
                    self.fire(i);
                }
            }
        }
        self.now = target;
    }

}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

fn run_q() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut w = World::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        match p[0] {
            "case" => {
                w = World::new();
                bud_facts::record_queries();
                bud_facts::set_answers(&[], 0);
                bud_facts::set_calls(0);
                bud_facts::record_running();
                bud_facts::set_running(Vec::new());
                bud_facts::set_running_calls(0);
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "now" => w.now = p[1].parse().unwrap(),
            "quiet" => bud_facts::set_answers(&nums(&p[1..]), 0),
            "del" => {
                let key = nums(&p[1..]);
                let sid = take_key(&mut key.as_slice()).unwrap();
                sol::t_del(&mut w.st, &sid);
            }
            "rdi" => {
                let a = nums(&p[1..]);
                let mut v = vec![0i64];
                v.extend_from_slice(&a);
                bud_facts::push_running(v);
            }
            "put" => w.put(&parse_put(&p[1..])),
            "adv" => {
                w.log.push("@ -".to_string());
                w.advance(p[1].parse::<i64>().unwrap());
            }
            "call" => {
                w.log.push(format!("@ {}", p[1]));
                w.call(&nums(&p[2..]));
            }
            "end" => {
                for l in w.log.drain(..) {
                    writeln!(o, "{}", l).unwrap();
                }
                let ob = w.observe();
                writeln!(o, "{}", ob).unwrap();
                writeln!(o, "end").unwrap();
            }
            other => panic!("bad line {other}"),
        }
    }
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

pub const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn run_perf(args: &[String]) {
    bud_facts::set_answers(&[], 1);
    bud_facts::set_calls(0);
    let d: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let bid = "99b568f7-0000-4000-8000-000000000001";
    let (sid, budget) = decode_put(&perf_put(key("session-perf"), key(bid), 0, PERF_REST));
    let mut ids = vec![0i64, d as i64];
    for i in 0..d {
        ids.extend(key(&format!("d{:04}", i)));
    }
    let mut w = World::new();
    w.silent = true;
    let putback = |w: &mut World| {
        assert_eq!(sol::t_put(&mut w.st, &sid, black_box(&budget)), 0);
    };
    putback(&mut w);
    let go = |w: &mut World| {
        bud_facts::set_running(vec![ids.clone()]);
        bud_facts::set_running_calls(0);
        assert_eq!(black_box(w.call_typed(12, 1, &sid, bid, "", &[0; 10])), 0);
        putback(w);
    };
    for _ in 0..1000 {
        go(&mut w);
    }
    // the halt ran: one query, D stops (2D + 7 effect words, no timerClear: the perf budget has no clock)
    bud_facts::set_running(vec![ids.clone()]);
    bud_facts::set_running_calls(0);
    assert_eq!(w.call_typed(12, 1, &sid, bid, "", &[0; 10]), 0);
    assert_eq!(w.st.out.fx.len(), d + 7);
    assert_eq!(w.st.sessions[&sid].stopped.len(), d);
    assert_eq!(bud_facts::running_calls(), 1);
    putback(&mut w);
    let t = Instant::now();
    for _ in 0..reps {
        go(&mut w);
    }
    println!("halt {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
    let t = Instant::now();
    for _ in 0..reps {
        putback(&mut w);
    }
    println!("putback {} {:.1}", d, t.elapsed().as_nanos() as f64 / reps as f64);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else {
        run_q();
    }
}

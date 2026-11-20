// The driver owns a generated St, translates packed wire keys, and prints the
// same results as driver_m8. Pruv owns accounting and supplies insertion ordinals.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;
use std::time::Instant;

// M8_LANE=adapter routes the account codes through the test-only `t_call`; anything else is the direct lane.
fn direct_lane() -> bool {
    static DIRECT: OnceLock<bool> = OnceLock::new();
    *DIRECT.get_or_init(|| std::env::var("M8_LANE").as_deref() != Ok("adapter"))
}

// Encode each UTF-16 unit as a distinct scalar, using a supplementary range
// for surrogates. This also preserves unpaired surrogates.
const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

fn take_key(words: &mut &[i64], key: &mut String) -> Option<()> {
    let (&len, rest) = words.split_first()?;
    if !(0..=1 << 30).contains(&len) {
        return None;
    }
    let len = len as usize;
    let count = len.div_ceil(4);
    let packed = rest.get(..count)?;
    let padding = (4 - len % 4) % 4;
    if padding != 0 && (*packed.last()? as u64 & ((1 << (padding * 16)) - 1)) != 0 {
        return None;
    }
    *words = &rest[count..];
    key.clear();
    key.reserve(len);
    for i in 0..len {
        let unit = ((packed[i / 4] as u64 >> (16 * (3 - i % 4))) & 0xffff) as u32;
        let scalar = char::from_u32(unit)
            .unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap());
        key.push(scalar);
    }
    Some(())
}

fn utf16_units(key: &str) -> impl Iterator<Item = u16> + '_ {
    key.chars().map(|c| {
        let scalar = c as u32;
        if scalar < 0x10000 { scalar as u16 } else { (scalar - SURROGATE_OFFSET) as u16 }
    })
}

fn append_observed_key(out: &mut Vec<i64>, key: &str) {
    out.push(key.chars().count() as i64);
    out.extend(utf16_units(key).map(i64::from));
}

fn observe_words(st: &mut sol::St) -> Vec<i64> {
    let reply = sol::t_observe(st);
    let mut words = reply.words.iter().copied();
    let mut keys = reply.keys.iter().map(String::as_str);
    let mut out = Vec::with_capacity(reply.words.len());

    let session_count = words.next().unwrap();
    let mut sessions = Vec::new();
    for _ in 0..session_count {
        let ordinal = words.next().unwrap();
        let sid = keys.next().unwrap();
        let bid = keys.next().unwrap();
        let budget: [i64; 14] = std::array::from_fn(|_| words.next().unwrap());
        sessions.push((ordinal, sid, bid, budget));
    }
    sessions.sort_unstable_by_key(|session| session.0);
    out.push(session_count);
    for (_, sid, bid, budget) in sessions {
        append_observed_key(&mut out, sid);
        out.push(15 + bid.chars().count().div_ceil(4) as i64);
        out.extend(budget);
        append_packed_key(&mut out, utf16_units(bid));
    }

    out.push(words.next().unwrap()); // Clock count.
    let turn_count = words.next().unwrap();
    let mut turns = Vec::new();
    for _ in 0..turn_count {
        let ordinal = words.next().unwrap();
        let turn = keys.next().unwrap();
        let activity_count = words.next().unwrap();
        let mut activities = Vec::new();
        for _ in 0..activity_count {
            activities.push((words.next().unwrap(), keys.next().unwrap()));
        }
        turns.push((ordinal, turn, activities));
    }
    turns.sort_unstable_by_key(|turn| turn.0);
    out.push(turn_count);
    for (_, turn, mut activities) in turns {
        append_observed_key(&mut out, turn);
        activities.sort_unstable_by_key(|activity| activity.0);
        out.push(activities.len() as i64);
        for (_, activity) in activities {
            append_observed_key(&mut out, activity);
        }
    }
    out
}

struct World {
    st: sol::St,
    keys: [String; 4],
}

impl World {
    fn new() -> Self {
        Self {
            st: sol::St {
                sessions: Default::default(),
                next_session: 0,
                turns: Default::default(),
                next_turn: 0,
            },
            keys: Default::default(),
        }
    }

    fn put(&mut self, op: &[i64]) -> i64 {
        self.put_checked(op).unwrap_or(2)
    }

    fn put_checked(&mut self, mut op: &[i64]) -> Option<i64> {
        let [sid, bid, ..] = &mut self.keys;
        take_key(&mut op, sid)?;
        let (&length, record) = op.split_first()?;
        if usize::try_from(length).ok()? != record.len() {
            return None;
        }
        let (counters, mut id) = record.split_at_checked(14)?;
        take_key(&mut id, bid)?;
        if !id.is_empty() {
            return None;
        }
        Some(sol::t_put(&mut self.st, sid, bid, counters))
    }

    // Return the wire status and the charge result (when present).
    #[inline]
    fn call(&mut self, op: &[i64]) -> (i64, i64) {
        self.call_checked(op).unwrap_or((2, 0))
    }

    fn call_checked(&mut self, op: &[i64]) -> Option<(i64, i64)> {
        let (&code, mut rest) = op.split_first()?;
        let [sid, bid, turn, activity] = &mut self.keys;
        if code == 4 {
            take_key(&mut rest, turn)?;
            return rest.is_empty().then(|| (sol::t_forget(&mut self.st, turn), 0));
        }
        take_key(&mut rest, sid)?;
        take_key(&mut rest, bid)?;
        // Pruv chooses every status (P32 D66); Rust only decodes keys and picks the entry.
        let answer = match code {
            3 => {
                take_key(&mut rest, turn)?;
                take_key(&mut rest, activity)?;
                return rest.is_empty().then(|| (sol::t_tool(&mut self.st, sid, bid, turn, activity), 0));
            }
            _ if !direct_lane() => sol::t_call(&mut self.st, code, sid, bid, rest),
            1 => match sol::t_charge_words(rest) {
                Some(charge) => sol::t_charge(&mut self.st, sid, bid, charge),
                None => sol::t_invalid(),
            },
            2 if rest.is_empty() => return Some((sol::t_fail(&mut self.st, sid, bid), 0)),
            _ => sol::t_invalid(),
        };
        Some((answer.status, answer.result))
    }

    fn checksum(&mut self) -> u64 {
        observe_words(&mut self.st).into_iter().fold(0u64, |sum, word| {
            sum.wrapping_mul(31).wrapping_add(word as u64)
        })
    }
}

fn write_words(out: &mut impl Write, prefix: &str, words: &[i64]) {
    write!(out, "{prefix}").unwrap();
    for word in words {
        write!(out, " {word}").unwrap();
    }
    writeln!(out).unwrap();
}

fn nums(fields: &[&str]) -> Vec<i64> {
    fields.iter().map(|word| word.parse().unwrap()).collect()
}

fn run_q() {
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut world = World::new();
    let mut stopped = false;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields.first().copied() {
            Some("case") => {
                world = World::new();
                stopped = false;
                writeln!(out, "case {}", fields[1]).unwrap();
            }
            Some("step") if !stopped => writeln!(out, "@ {}", fields[1]).unwrap(),
            Some("put") => {
                let status = world.put(&nums(&fields[1..]));
                if status != 0 {
                    writeln!(out, "P X {status}").unwrap();
                }
            }
            Some("call") if !stopped => {
                let op = nums(&fields[1..]);
                match world.call(&op) {
                    (0, result) if op[0] == 1 => writeln!(out, "R {result}").unwrap(),
                    (0, _) => writeln!(out, "R -").unwrap(),
                    (6, result) => {
                        let mut frame = vec![20, result - 32];
                        frame.extend_from_slice(&op[1..op.len() - 10]);
                        write_words(&mut out, "H", &frame);
                        stopped = true;
                    }
                    (status, _) => {
                        writeln!(out, "X {status}").unwrap();
                        stopped = true;
                    }
                }
            }
            Some("end") => {
                write_words(&mut out, "O", &observe_words(&mut world.st));
                writeln!(out, "end").unwrap();
            }
            Some("step" | "call") | None => {}
            Some(other) => panic!("unsupported typed Q command {other}"),
        }
    }
}

fn push_key(v: &mut Vec<i64>, s: &str) {
    append_packed_key(v, s.encode_utf16());
}

fn append_packed_key(v: &mut Vec<i64>, units: impl Iterator<Item = u16>) {
    let at = v.len();
    v.push(0);
    let (mut n, mut x, mut l) = (0i64, 0u64, 0u32);
    for u in units {
        x = (x << 16) | u as u64;
        n += 1;
        l += 1;
        if l == 4 {
            v.push(x as i64);
            x = 0;
            l = 0;
        }
    }
    if l > 0 {
        v.push((x << (16 * (4 - l))) as i64);
    }
    v[at] = n;
}

// Perf text keys, formatted into one reused String.
struct Text(String);

impl Text {
    fn session(&mut self, i: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "session-{:05}", i);
        &self.0
    }
    fn bid(&mut self, i: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "99b568f7-0000-4000-8000-{:012}", i);
        &self.0
    }
    fn activity(&mut self, j: usize) -> &str {
        use std::fmt::Write as _;
        self.0.clear();
        let _ = write!(self.0, "activity-{:05}", j);
        &self.0
    }
}

// Perf op frames, all in one word buffer: op i is w[at[i]..at[i + 1]].
struct Ops {
    w: Vec<i64>,
    at: Vec<usize>,
}

impl Ops {
    fn new(n: usize, words: usize) -> Ops {
        let mut at = Vec::with_capacity(n + 1);
        at.push(0);
        Ops { w: Vec::with_capacity(n * words), at }
    }
    fn end(&mut self) {
        self.at.push(self.w.len());
    }
    #[inline]
    fn get(&self, i: usize) -> &[i64] {
        &self.w[self.at[i]..self.at[i + 1]]
    }
    fn len(&self) -> usize {
        self.at.len() - 1
    }
}

fn push_ref(v: &mut Vec<i64>, t: &mut Text, i: usize) {
    push_key(v, t.session(i));
    push_key(v, t.bid(i));
}

// N sessions `session-<i>` with an open budget (limits 10^12, so charges never reach a limit).
fn world_n(n: usize, t: &mut Text) -> World {
    let mut w = World::new();
    let mut put: Vec<i64> = Vec::with_capacity(64);
    for i in 0..n {
        put.clear();
        push_key(&mut put, t.session(i));
        let rl = put.len();
        put.push(0);
        put.extend_from_slice(&[0i64, 0, -1]);
        put.extend_from_slice(&[1_000_000_000_000i64; 4]);
        put.extend_from_slice(&[0i64; 7]);
        push_key(&mut put, t.bid(i)); // [bn] + packed budgetId words, so the record is 14 + 1 + ceil(bn/4) words
        put[rl] = (put.len() - rl - 1) as i64;
        let r = w.put(&put);
        if r != 0 {
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
    }
    w
}

// perf <N> <reps>, prints `item size ns_per_call`:
//   charge   : charge(ref_i, {commands: 1, retries: 1}) round-robin over N sessions (successful charge)
//   failure  : noteFailure(ref_i) round-robin over N sessions
//   tooldup  : noteTool(ref_0, t1, a_j) on a turn that already holds N activities (dedup hit, no CURRENT)
//   toolnew   : noteTool(ref_0, t2, a_j) for N new activities; ns per noteTool
//   toolforget: forgetTurn(t2) on a turn holding N activities; ns per forgetTurn
//   toolgrow  : instruction item; same loop, ns per noteTool with forget included
// An optional third argument names one item: only that item's timed loop runs (the setup of every item timed before it
// still runs; charge and failure stop before the tool setup, as every hand lane does), so `/usr/bin/time -l` instruction counts at two rep counts give the instructions per call of that item alone.
fn run_perf(args: &[String]) {
    let n: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only = args.get(2).map(|s| s.as_str());
    let on = |item: &str| only.map_or(true, |o| o == item);
    let mut t = Text(String::with_capacity(64));
    let mut w = world_n(n, &mut t);
    let mut ops = Ops::new(n, 32);
    for i in 0..n {
        ops.w.push(1);
        push_ref(&mut ops.w, &mut t, i);
        ops.w.extend_from_slice(&[0, 0, 2, 1, 0, 0, 0, 0, 2, 1]);
        ops.end();
    }
    for i in 0..1000 {
        w.call(ops.get(i % n));
    }
    if on("charge") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(ops.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("charge {} {:.1}", n, ns);
        println!("chk charge {} {}", n, w.checksum()); // outside the timing
    }
    let mut fo = Ops::new(n, 20);
    for i in 0..n {
        fo.w.push(2);
        push_ref(&mut fo.w, &mut t, i);
        fo.end();
    }
    if on("failure") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(fo.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("failure {} {:.1}", n, ns);
        println!("chk failure {} {}", n, w.checksum());
    }
    // charge and failure are timed before any tool setup runs, so a run that names one of them stops here; the tool
    // setup below would only add its own state to that item's peak RSS (in every lane alike).
    if matches!(only, Some("charge") | Some("failure")) {
        return;
    }
    let tools = |turn: &str, t: &mut Text| -> Ops {
        let mut o = Ops::new(n, 32);
        for j in 0..n {
            o.w.push(3);
            push_ref(&mut o.w, t, 0);
            push_key(&mut o.w, turn);
            push_key(&mut o.w, t.activity(j));
            o.end();
        }
        o
    };
    let dup = tools("turn-1", &mut t);
    for j in 0..dup.len() {
        w.call(dup.get(j));
    }
    if on("tooldup") {
        let t = Instant::now();
        for i in 0..reps {
            black_box(w.call(black_box(dup.get(i % n))));
        }
        let ns = t.elapsed().as_nanos() as f64 / reps as f64;
        println!("tooldup {} {:.1}", n, ns);
        println!("chk tooldup {} {}", n, w.checksum());
    }
    let grow = tools("turn-2", &mut t);
    let mut forget = vec![4];
    push_key(&mut forget, "turn-2");
    for j in 0..grow.len() {
        w.call(grow.get(j));
    }
    w.call(&forget);
    if on("toolnew") || on("toolforget") || only == Some("toolgrow") {
        let rounds = std::cmp::max(1, reps / n);
        let mut t_new = 0;
        let mut t_forget = 0;
        for _ in 0..rounds {
            let start = Instant::now();
            for j in 0..grow.len() {
                black_box(w.call(black_box(grow.get(j))));
            }
            let after_new = Instant::now();
            black_box(w.call(black_box(&forget)));
            let after_forget = Instant::now();
            t_new += (after_new - start).as_nanos();
            t_forget += (after_forget - after_new).as_nanos();
        }
        // Untimed extra round: the insert result is checksummed independently of the forget.
        for j in 0..grow.len() {
            w.call(grow.get(j));
        }
        let chk_new = w.checksum();
        w.call(&forget);
        let chk_forget = w.checksum();
        if only == Some("toolgrow") {
            let ns = (t_new + t_forget) as f64 / (rounds * n) as f64;
            println!("toolgrow {} {:.1}", n, ns);
            println!("chk toolgrow {} {}:{}", n, chk_new, chk_forget);
        }
        if on("toolnew") {
            let ns = t_new as f64 / (rounds * n) as f64;
            println!("toolnew {} {:.1}", n, ns);
            println!("chk toolnew {} {}", n, chk_new);
        }
        if on("toolforget") {
            let ns = t_forget as f64 / rounds as f64;
            println!("toolforget {} {:.1}", n, ns);
            println!("chk toolforget {} {}", n, chk_forget);
        }
    }
}

fn run_churn() {
    let mut w = World::new();
    let st = &mut w.st;
    assert_eq!(sol::t_put(st, "session", "budget", &[0; 14]), 0);
    assert_eq!(sol::t_tool(st, "session", "budget", "temp", "first"), 0);
    assert_eq!(sol::t_tool(st, "session", "budget", "keep", "first"), 0);
    assert_eq!(sol::t_tool(st, "session", "budget", "keep", "second"), 0);
    assert_eq!(sol::t_forget(st, "temp"), 0);

    for ordinal in 2..10_002 {
        for activity in ["first", "second", "third"] {
            assert_eq!(sol::t_tool(st, "session", "budget", "temp", activity), 0);
        }
        let temp = &st.turns["temp"];
        assert_eq!(temp.ordinal, ordinal);
        assert_eq!(temp.next_activity, 3);
        assert_eq!(temp.activities.len(), 3);
        assert_eq!(temp.activities["first"], 0);
        assert_eq!(temp.activities["second"], 1);
        assert_eq!(temp.activities["third"], 2);
        assert_eq!(sol::t_forget(st, "temp"), 0);
        assert_eq!(st.turns.len(), 1);
    }

    assert_eq!(st.turns["keep"].ordinal, 1);
    assert_eq!(sol::t_tool(st, "session", "budget", "temp", "third"), 0);
    let mut expected = vec![0, 2]; // Clocks, then turns.
    append_observed_key(&mut expected, "keep");
    expected.push(2);
    append_observed_key(&mut expected, "first");
    append_observed_key(&mut expected, "second");
    append_observed_key(&mut expected, "temp");
    expected.push(1);
    append_observed_key(&mut expected, "third");
    assert!(observe_words(st).ends_with(&expected));
    assert_eq!(sol::t_forget(st, "keep"), 0);
    assert_eq!(sol::t_forget(st, "temp"), 0);
    assert!(st.turns.is_empty());
    assert_eq!(st.next_turn, 10_003);

    const CAP: i64 = 1 << 40;
    st.next_session = CAP;
    assert_eq!(sol::t_put(st, "session", "budget", &[0; 14]), 0);
    assert_eq!(st.sessions["session"].ordinal, 0);
    assert_eq!(sol::t_put(st, "new", "budget", &[0; 14]), 5);
    st.next_turn = CAP - 1;
    assert_eq!(sol::t_tool(st, "session", "budget", "last", "first"), 0);
    assert_eq!(st.turns["last"].ordinal, CAP - 1);
    assert_eq!(sol::t_tool(st, "session", "budget", "new", "first"), 5);
    st.turns.get_mut("last").unwrap().next_activity = CAP;
    assert_eq!(sol::t_tool(st, "session", "budget", "last", "first"), 0);
    assert_eq!(sol::t_tool(st, "session", "budget", "last", "second"), 5);
    assert_eq!(st.sessions["session"].tools, 1);
    println!("churn: 10000 cycles, reinsertion order and ordinal capacity checked");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else if args.get(1).map(String::as_str) == Some("churn") {
        run_churn();
    } else {
        run_q();
    }
}

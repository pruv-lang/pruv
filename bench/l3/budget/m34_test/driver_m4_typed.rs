// The host owns one generated St and translates the q wire format.
use std::io::{self, BufRead, Write};

fn empty_state() -> sol::St {
    sol::St {
        sessions: Default::default(),
        session_order: Vec::new(),
        clocks: Default::default(),
        clock_order: Vec::new(),
        tools: Default::default(),
        turn_order: Vec::new(),
    }
}

// Map each UTF-16 unit independently. Surrogates use otherwise unused
// supplementary scalars so lone surrogates round-trip without collisions.
const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

fn take_key(words: &mut &[i64]) -> Option<String> {
    let (&n, rest) = words.split_first()?;
    if !(0..=1 << 30).contains(&n) {
        return None;
    }
    let n = n as usize;
    let count = n.div_ceil(4);
    let packed = rest.get(..count)?;
    let padding = (4 - n % 4) % 4;
    if padding != 0 && ((*packed.last()? as u64) & ((1 << (padding * 16)) - 1)) != 0 {
        return None;
    }
    *words = &rest[count..];
    let mut key = String::with_capacity(n);
    for i in 0..n {
        let unit = ((packed[i / 4] as u64 >> (16 * (3 - i % 4))) & 0xffff) as u32;
        let scalar = char::from_u32(unit)
            .unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap());
        key.push(scalar);
    }
    Some(key)
}

fn exec(st: &mut sol::St, op: &[i64]) -> Option<(i64, sol::Reply)> {
    let (&code, mut rest) = op.split_first()?;
    let mut key = String::new();
    let mut other = String::new();
    let data = match code {
        4 | 12 | 17 | 18 if rest.is_empty() => rest,
        1 => {
            key = take_key(&mut rest)?;
            let (&length, blob) = rest.split_first()?;
            if usize::try_from(length).ok()? != blob.len() {
                return None;
            }
            blob
        }
        2 | 3 | 6 | 7 | 8 | 11 | 14 | 15 | 16 => {
            key = take_key(&mut rest)?;
            if rest.len() != if code == 7 { 4 } else { 0 } {
                return None;
            }
            rest
        }
        5 | 9 | 10 | 13 => {
            key = take_key(&mut rest)?;
            other = take_key(&mut rest)?;
            if rest.len() != if code == 9 { 1 } else { 0 } {
                return None;
            }
            rest
        }
        _ => return None,
    };
    Some((code, sol::m4_op(st, code, &key, &other, data)))
}

fn wire_words(reply: sol::Reply) -> Vec<i64> {
    let mut keys = reply.keys.into_iter().peekable();
    let mut out = Vec::with_capacity(reply.words.len());
    for (i, word) in reply.words.into_iter().map(Some).chain([None]).enumerate() {
        while keys.peek().is_some_and(|key| key.offset == i as i64) {
            let key = keys.next().unwrap();
            out.push(key.s.chars().count() as i64);
            for c in key.s.chars() {
                let scalar = c as u32;
                let unit = if scalar < 0x10000 { scalar } else { scalar - SURROGATE_OFFSET };
                out.push(unit as i64);
            }
        }
        out.extend(word);
    }
    assert!(keys.next().is_none(), "unordered reply key offsets");
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

// Small perf run (P31T5 step 2): the sessions and tools op cycles of driver_m4.rs perf(), replayed through exec()
// (wire decode + m4_op) so the ns/op is comparable to the flat lane's core_ns_per_op.
fn perf(axis: &str, n: usize, reps: usize) {
    use std::hint::black_box;
    let mut st = empty_state();
    let blob: Vec<i64> = vec![11, 22, 33, 44, 55, 66, 77, 88];
    let ok = |st: &mut sol::St, op: &[i64]| match exec(st, op) {
        Some((_, r)) if r.status == 0 => r,
        _ => panic!("perf: op failed"),
    };
    let cycle: Vec<Vec<i64>> = match axis {
        "sessions" => {
            for i in 0..n {
                ok(&mut st, &op_session_put(&format!("s{i}"), &blob));
            }
            let k = format!("s{}", n / 2);
            vec![op_k(2, &k), op_session_put(&k, &blob)]
        }
        "tools" => {
            for i in 0..n {
                ok(&mut st, &op_kk(13, "t0", &format!("a{i}")));
            }
            let mut c = Vec::new();
            for r in 0..n.max(1).min(reps.max(1)) {
                c.push(op_kk(13, "t0", &format!("a{}", r % n.max(1))));
                c.push(op_kk(13, "x", "y"));
                c.push(op_k(14, "x"));
            }
            c
        }
        _ => panic!("unknown axis {axis}"),
    };
    let width = if axis == "tools" { 3 } else { 2 };
    let nops = (reps * width).max(1);
    let t0 = std::time::Instant::now();
    let mut sink = 0i64;
    let mut j = 0;
    for _ in 0..nops {
        let r = ok(&mut st, black_box(&cycle[j]));
        sink = sink.wrapping_add(r.words.first().copied().unwrap_or(0));
        j += 1;
        if j == cycle.len() {
            j = 0;
        }
    }
    let total = t0.elapsed().as_nanos();
    black_box(sink);
    println!("axis={axis} n={n} core_ns_per_op={:.1} ops={nops}", total as f64 / nops as f64);
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("perf") {
        perf(&args[2], args[3].parse().unwrap(), args[4].parse().unwrap());
        return Ok(());
    }
    assert_eq!(args.get(1).map(String::as_str), Some("q"), "usage: driver_m4_typed q | perf AXIS N REPS");
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut state = None;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("init") => {
                let _: usize = fields.next().expect("init needs L").parse().unwrap();
                state = Some(empty_state());
                writeln!(out, "init ok")?;
            }
            Some("op") => {
                let op: Vec<i64> = fields.map(|word| word.parse().expect("bad word")).collect();
                let st = state.as_mut().expect("op before init");
                match exec(st, &op) {
                    Some((code, reply)) if reply.status == 0 => {
                        let prefix = if code == 18 { "stats" } else { "ok" };
                        write_words(&mut out, prefix, &wire_words(reply))?;
                    }
                    _ => writeln!(out, "invalid")?,
                }
            }
            Some("observe") => {
                let reply = sol::m4_observe(state.as_mut().expect("observe before init"));
                assert_eq!(reply.status, 0);
                let words = wire_words(reply);
                write_words(&mut out, &format!("observe {}", words.len()), &words)?;
            }
            None => {}
            Some(cmd) => panic!("unknown command {cmd}"),
        }
    }
    out.flush()
}

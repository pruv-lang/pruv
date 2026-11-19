// M7 driver (appended to a main file that already declares `mod sol;`, see run_m7.py).
//
//   driver_m7 q        program on stdin, one case after another:
//                        case <id> | now <ms> | uuids <key>... | quiet <0/1>... | put <words...> | del <key words...>
//                        | adv <ms> | call <req> <op words without now/reserved word/uuid> | end
//   driver_m7 perf <S> <reps>   perf program, see run_m7.py (cmd_perf). After each item's timed loop it runs an untimed
//                        verification pass (fresh identical setup, same ops, V = min(reps, 2000) iterations) and prints
//                        `rchk <item> <size> <value>`: s = s*31 + w over every op's status, effect words and result
//                        words (see rchk_fold). hand_m7_u16 perf prints the same encoding; run_m7.py compares them.
//                        Lanes per item: wire (`<item>`), typed (`t`, `rchk_t`) and, for ishalted, current, attach,
//                        closereopen and reopenmiss, the fair typed lane (`f`, `rchk_f`, T5c; see fair_loop).
//   driver_m7 perf <S> <reps> <item> <t|f>   only that lane of that item (instruction counts, t5c_floor.py)
//
// Boundary rule: the host owns the state buffer and the scratch frames and supplies the inputs of a call (now, the
// next randomUUID value); it never reads state words and decides nothing. It reads the result
// tag in out only to print the words (a budget result is a view [4, n, off]: the host prints st[off..off+n), as a
// caller reads through the returned reference). Effects: [20] changed, [21] carryHeld, [22] uuid consumed (the host
// advances its UUID stream), [13] timerClear. isQuiet takes the next answer synchronously (default false).
// Grow -N: buffer of exactly N (Pruv sizes it, N >= 2*len), t_grow, retry.
// Wire keys are packed u16 keys `n, words(n) packed words` (bud_keys layout), packed once by the encoder (run_m7.py
// for q mode, key() below for perf / boundary); the records hold the budget id and follows packed the same way
// (bud_lifecycle.pruv header). A ref result keeps its ids packed too ([3, n, words(n) words (, m, words(m) words)]);
// the host unpacks them to units at egress (ref_units) for the q-mode log and rchk, so those stay `len, units...`.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    bv: Vec<i64>,
    fx: Vec<i64>,
    op: Vec<i64>,
    now: i64,
    uuids: Vec<Vec<i64>>,
    next_uuid: usize,
    log: Vec<String>,
    silent: bool,
    audit: bool,
    // perf only (silent): the ops built so far, keyed by (body address, body length, uuid index). The perf
    // bodies are fixed vectors, so a host that keeps its prepared arguments does not rebuild them on every call.
    ops: Vec<((usize, usize, usize), Vec<i64>)>,
}

impl World {
    fn new() -> World {
        let mut st = vec![0i64; 256];
        let r = sol::t_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            if sol::t_init(&mut st) != 0 {
                eprintln!("t_init failed");
                std::process::exit(2);
            }
        }
        World {
            st,
            out: vec![0; 4096],
            bv: vec![0; 8],
            fx: vec![0; 16],
            op: Vec::with_capacity(256),
            now: 1700000000000,
            uuids: Vec::new(),
            next_uuid: 0,
            log: Vec::new(),
            silent: false,
            audit: false,
            ops: Vec::new(),
        }
    }

    fn grow(&mut self, n: usize) {
        let want = n; // N from Pruv (bud_store::need, N >= 2*len): the host makes no sizing choice (L3 P13b)
        let mut fresh = vec![0i64; want];
        if sol::t_grow(&self.st, &mut fresh) != 0 {
            eprintln!("t_grow failed");
            std::process::exit(2);
        }
        self.st = fresh;
    }

    fn put(&mut self, op: &[i64]) {
        for _ in 0..4 {
            let r = sol::t_put(&mut self.st, op, &mut self.out);
            if r == 0 {
                return;
            }
            if r == 1 {
                let n = self.out[0].max(0) as usize;
                self.grow(n);
                continue;
            }
            eprintln!("t_put status {}", r);
            std::process::exit(2);
        }
    }

    fn del(&mut self, op: &[i64]) {
        let r = sol::t_del(&mut self.st, op, &mut self.out);
        if r != 0 && !self.silent {
            self.log.push(format!("X del {}", r));
        }
    }

    // One public call: body = [code, origin, size, unit x6, sessionId key, budgetId key]; the host inserts now, a reserved
    // zero word, and the next UUID key.
    fn call(&mut self, body: &[i64]) -> i64 {
        let (slot, op) = self.take_op(body);
        self.call_with(slot, op)
    }

    // The host op build / cache lookup of `call` (returns the op taken out of its slot; give it back with put_op).
    fn take_op(&mut self, body: &[i64]) -> (usize, Vec<i64>) {
        let k = (body.as_ptr() as usize, body.len(), self.next_uuid);
        let mut slot = usize::MAX;
        if self.silent {
            slot = match self.ops.iter().position(|(key, _)| *key == k) {
                Some(i) => i,
                None => {
                    self.ops.push((k, Vec::new()));
                    self.ops.len() - 1
                }
            };
        }
        let mut op = if slot == usize::MAX { std::mem::take(&mut self.op) } else { std::mem::take(&mut self.ops[slot].1) };
        if op.is_empty() || slot == usize::MAX {
            op.clear();
            op.push(body[0]);
            op.push(self.now);
            op.push(0);
            op.extend_from_slice(&body[1..]);
            match self.uuids.get(self.next_uuid) {
                Some(u) => op.extend_from_slice(u),
                None => op.push(0),
            }
        }
        op[1] = self.now;
        (slot, op)
    }

    fn put_op(&mut self, slot: usize, op: Vec<i64>) {
        if slot == usize::MAX {
            self.op = op;
        } else {
            self.ops[slot].1 = op;
        }
    }

    fn call_with(&mut self, slot: usize, op: Vec<i64>) -> i64 {
        let mut r = 0;
        let before = bud_facts::calls();
        for _ in 0..8 {
            self.fx[0] = 0;
            let snapshot = self.audit.then(|| (self.st.clone(), self.fx.clone(), self.out.clone(), bud_facts::calls()));
            r = sol::t_call(&mut self.st, &op, &mut self.bv, &mut self.fx, &mut self.out);
            if let Some((st, fx, out, before)) = snapshot {
                self.log.push(format!("A {} {} {} {} {} {} {:?}",
                    r, bud_facts::calls() - before, self.st == st, self.fx == fx,
                    self.out == out, self.out[0], &self.fx[1..1 + self.fx[0] as usize]));
            }
            if r < 0 {
                self.grow((-r) as usize);
                continue;
            }
            break;
        }
        let after = bud_facts::calls();
        if !self.silent {
            for i in before..after {
                self.log.push(bud_facts::quiet_log(i));
            }
        }
        self.put_op(slot, op);
        let used = self.fx[0].max(0) as usize;
        if r != 0 {
            if !self.silent {
                self.log.push(format!("X {}", r));
            }
            return r;
        }
        for i in 1..=used {
            let e = self.fx[i];
            if e == 22 {
                self.next_uuid += 1;
            }
            if !self.silent {
                self.log.push(match e {
                    20 => "E changed".to_string(),
                    21 => "E carry".to_string(),
                    22 => "E uuid".to_string(),
                    13 => "E clear".to_string(),
                    x => format!("X fx{}", x),
                });
            }
        }
        if !self.silent {
            let w: Vec<String> = match self.out[0] {
                3 => {
                    // the ref's ids are packed in out; print them as units
                    let carry = self.fx[1..=used].contains(&21);
                    ref_units(&self.out, carry).iter().map(|x| x.to_string()).collect()
                }
                4 => {
                    // budget view: the record is st[off..off+n)
                    let n = self.out[1].max(0) as usize;
                    let off = self.out[2].max(0) as usize;
                    let mut v = vec!["4".to_string(), n.to_string()];
                    v.extend(self.st[off..off + n].iter().map(|x| x.to_string()));
                    v
                }
                x => vec![x.to_string()],
            };
            self.log.push(format!("V {}", w.join(" ")));
        }
        0
    }

    fn observe(&mut self) -> String {
        let need = std::cmp::max(4096, 2 * self.st.len());
        if self.out.len() < need {
            self.out.resize(need, 0);
        }
        let r = sol::t_observe(&self.st, &mut self.out);
        if r != 0 {
            return format!("O X {}", r);
        }
        let n = self.out[0].max(0) as usize;
        let mut s = String::from("O");
        for x in &self.out[1..1 + n] {
            s.push(' ');
            s.push_str(&x.to_string());
        }
        s
    }
}

fn nums(it: &[&str]) -> Vec<i64> {
    it.iter().map(|x| x.parse::<i64>().unwrap()).collect()
}

// Data words of a packed key of n units: ceil(n / 4).
fn words(n: i64) -> usize {
    (n.max(0) as usize + 3) / 4
}

// Host egress of a ref result (out[0] == 3): out = [3, n, packed id words (words(n)) (, m, packed old id words(m) when
// effect 21 is held)]. Returns the unit form [3, n, id units.. (, m, old id units..)], the words q mode prints and
// rchk folds. Untimed paths only.
fn ref_units(out: &[i64], carry: bool) -> Vec<i64> {
    let mut v = vec![3i64];
    let mut p = 1;
    for _ in 0..(if carry { 2 } else { 1 }) {
        let n = out[p].max(0) as usize;
        v.push(n as i64);
        for j in 0..n {
            let word = out[p + 1 + j / 4] as u64;
            v.push(((word >> (48 - 16 * (j % 4))) & 0xffff) as i64);
        }
        p += 1 + words(n as i64);
    }
    v
}

// Packed words of a ref result: 2 + words(n) (+ 1 + words(m) when the old id is carried).
fn ref_len(out: &[i64], carry: bool) -> usize {
    let l = 2 + words(out[1]);
    if carry {
        l + 1 + words(out[l])
    } else {
        l
    }
}

// A run of packed keys `n, words(n) packed words` into separate keys (each with its frame word).
fn keys(w: &[i64]) -> Vec<Vec<i64>> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < w.len() {
        let m = words(w[p]);
        v.push(w[p..p + 1 + m].to_vec());
        p += 1 + m;
    }
    v
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
                writeln!(o, "case {}", p[1]).unwrap();
            }
            "now" => w.now = p[1].parse().unwrap(),
            "uuids" => w.uuids = keys(&nums(&p[1..])),
            "audit" => {
                w.audit = true;
                w.out.fill(-99);
                w.fx[1..].fill(-99);
            }
            // Test-only: consume the remaining arena with unreferenced padding.
            "fill_arena" => w.st[10] = w.st[11],
            "dump" => w.log.push(format!("S {:?}", w.st)),
            "tombstones" => {
                // Turn unused slots into tombstones without moving any live entry.
                let slots = w.st[4];
                assert!(w.st[12] <= slots * 3 / 8);
                for i in 16..16 + slots as usize {
                    if w.st[i] == 0 && w.st[13] <= slots / 2 {
                        w.st[i] = -1;
                        w.st[13] += 1;
                    }
                }
                assert_eq!(w.st[13], slots / 2 + 1);
                assert!(w.st[11] - w.st[10] >= 64);
            }
            "quiet" => bud_facts::set_answers(&nums(&p[1..]), 0),
            "put" => w.put(&nums(&p[1..])),
            "del" => w.del(&nums(&p[1..])),
            "adv" => w.now += p[1].parse::<i64>().unwrap(),
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
            other => {
                eprintln!("bad line {}", other);
                std::process::exit(2);
            }
        }
    }
}

// perf <S> <reps>: S sessions s0000..s<S-1>, each with an open budget (rest text of a typical card). Items:
//   current_S : current(ref) of the last session (hit)
//   replace_S : replaceForUserMessage on a halted budget of the last session (CLOSE + OPEN + carryHeld), then the
//               record is put back halted (the put is outside the timed call... see below: timed together, both sides)
//   reopen_S  : maybeClose(quiet) then reopen on the last session (2 calls per rep)
//   attach_S  : attach on an open budget of the last session (ref result)
// prints `item size ns_per_call`.
// Record words (bud_lifecycle.pruv header): the id (string 0) and follows (string 4, empty here) are packed keys.
fn rec(id: &str, state: i64, rest: &str) -> Vec<i64> {
    let mut h = vec![0i64; 24];
    h[0] = state;
    h[3] = 120;
    h[4] = 24;
    h[5] = 3;
    h[6] = 2;
    h[7] = 1;
    h[20] = 1;
    let mut v = h;
    let mut s = |t: &str, v: &mut Vec<i64>| {
        let u: Vec<i64> = t.encode_utf16().map(|x| x as i64).collect();
        v.push(u.len() as i64);
        v.extend(u);
    };
    v.extend(key(id));
    s("2023-11-14T22:13:20.000Z", &mut v);
    s("", &mut v);
    s("", &mut v);
    v.extend(key(""));
    s(rest, &mut v);
    v
}

// Packed-u16 words (bud_keys layout): four big-endian lanes per word, tail lanes zero.
fn pack_u16(units: &[u16]) -> Vec<i64> {
    units
        .chunks(4)
        .map(|c| {
            let mut x: u64 = 0;
            for l in 0..4 {
                x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
            }
            x as i64
        })
        .collect()
}

// Wire key: `n, packed words(n)` (1 + ceil(n/4) words).
fn key(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    v.extend(pack_u16(&u));
    v
}

pub const PERF_REST: &str = "{\"card\":{\"deliverable\":\"d\",\"done\":[\"x\"],\"state\":\"done\",\"updatedAt\":\"2023-11-14T22:13:20.000Z\"}}";

fn run_perf(args: &[String]) {
    bud_facts::set_answers(&[], 1);
    bud_facts::set_calls(0);
    let size: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let sids: Vec<String> = (0..size).map(|i| format!("s{:04}", i)).collect();
    let bid = |i: usize| format!("{:08x}-0000-4000-8000-000000000001", i);
    let unit = [120i64, 24, 3, 2, 1, 0];
    let setup = |state: i64| -> World {
        let mut w = World::new();
        w.silent = true;
        for (i, s) in sids.iter().enumerate() {
            let mut op = key(s);
            let r = rec(&bid(i), state, PERF_REST);
            op.push(r.len() as i64);
            op.extend(r);
            w.put(&op);
        }
        w.uuids = vec![key("00000000-0000-4000-8000-0000000000aa")];
        w
    };
    let last = size - 1;
    let body = |code: i64, origin: i64, sz: i64, b: &str| -> Vec<i64> {
        let mut v = vec![code, origin, sz];
        v.extend_from_slice(&unit);
        v.extend(key(&sids[last]));
        v.extend(key(b));
        v
    };
    let time = |w: &mut World, ops: &[Vec<i64>], per: usize| -> f64 {
        for _ in 0..1000 {
            for op in ops {
                w.next_uuid = 0;
                bud_facts::set_calls(0);
                w.call(op);
            }
        }
        let t = Instant::now();
        for _ in 0..reps {
            for op in ops {
                w.next_uuid = 0;
                bud_facts::set_calls(0);
                let r = w.call(black_box(op));
                black_box((r, w.out[0]));
            }
        }
        t.elapsed().as_nanos() as f64 / (reps * per) as f64
    };
    // dispatch probe: the host op build + t_call's checks (valid, 3 key scans) with an unknown code (status 2)
    // Untimed per-op result verification (rchk): a fresh setup identical to the timed one, the same ops with the same
    // per-call resets (next_uuid and the host fact counter to 0), V = min(reps, 2000) iterations, every call's status, effect words
    // and result words folded by rchk_fold. Runs after the item's timed loop; nothing is added inside it.
    let vn = reps.min(2000);
    let verify = |w: &mut World, ops: &[Vec<i64>]| -> u64 {
        let mut s = 0u64;
        for _ in 0..vn {
            for op in ops {
                w.next_uuid = 0;
                bud_facts::set_calls(0);
                let r = w.call(op);
                s = rchk_fold(w, r, s);
            }
        }
        s
    };
    // Typed-entry lane: the same calls through c_direct(w, &Prep) (prepared once, outside the timed loop): one direct
    // typed call per op, like the hand typed lane's bare `w.current(..)`; the Grow retry loop (c_call) runs only
    // when a call answers Grow. The host fact uses the perf default answer. Prints
    // `t <item> <size> <ns>` and, from an untimed fresh run of vn iterations, `rchk_t <item> <size> <value>`.
    let typed = |name: &str, state: i64, bodies: &[Vec<i64>], per: usize, put: Option<&Vec<i64>>| {
        let mk = || -> (World, Vec<Prep>) {
            let mut w = setup(state);
            let ps: Vec<Prep> = bodies.iter().map(|b| prep_of(&mut w, b)).collect();
            (w, ps)
        };
        let step = |w: &mut World, ps: &[Prep], s: Option<&mut u64>| {
            let mut s = s;
            for p in ps {
                let r = c_direct(w, black_box(p));
                // consume the status and the result tag in one black_box, like the hand typed lane's one
                // black_box of its Res (current/isHalted value, reopenmiss Bool): out[0] is observed every call,
                // not only by the untimed rchk run
                black_box((r, w.out[0]));
                if let Some(s) = s.as_mut() {
                    **s = rchk_fold(w, r, **s);
                }
            }
            if let Some(p) = put {
                w.put(black_box(p));
            }
        };
        let (mut w, ps) = mk();
        for _ in 0..1000 {
            step(&mut w, &ps, None);
        }
        let t = Instant::now();
        for _ in 0..reps {
            step(&mut w, &ps, None);
        }
        println!("t {} {} {:.1}", name, size, t.elapsed().as_nanos() as f64 / (reps * per) as f64);
        let (mut w, ps) = mk();
        let mut s = 0u64;
        for _ in 0..vn {
            step(&mut w, &ps, Some(&mut s));
        }
        println!("rchk_t {} {} {}", name, size, s);
    };
    // Fair typed lane (T5c): the same c_* calls, shaped like hand16's typed loop. See fair_loop. Prints
    // `f <item> <size> <ns>` and, from an untimed fresh run of vn iterations through the same call, `rchk_f ..`.
    // `fair` is called right after `typed` for each item; the existing typed lane above is unchanged.
    let fair = |name: &str, bodies: &[Vec<i64>], per: usize| {
        let mk = || -> (World, Vec<Prep>) {
            let mut w = setup(0);
            let ps: Vec<Prep> = bodies.iter().map(|b| prep_of(&mut w, b)).collect();
            (w, ps)
        };
        let (mut w, ps) = mk();
        let fa: Vec<FArgs> = ps.iter().map(FArgs::of).collect();
        fair_loop(name, &mut w, &fa, 1000);
        let t = Instant::now();
        fair_loop(name, &mut w, &fa, reps);
        println!("f {} {} {:.1}", name, size, t.elapsed().as_nanos() as f64 / (reps * per) as f64);
        let (mut w, ps) = mk();
        let fa: Vec<FArgs> = ps.iter().map(FArgs::of).collect();
        println!("rchk_f {} {} {}", name, size, fair_rchk(name, &mut w, &fa, vn));
    };
    // `perf S reps ITEM LANE` (LANE t or f; T5c instruction counts): only that lane of ITEM runs, through the same
    // `typed` / `fair` closure as the full run (warm-up, timed loop, then its rchk pass of min(reps, 2000)
    // iterations, which cancels in (I(5R) - I(R)) for R >= 2000). No other item and no wire lane run.
    if let (Some(item), Some(lane)) = (args.get(2), args.get(3)) {
        let code = |c: i64, b: &str| body(c, -1, -1, b);
        let lb = bid(last);
        let (ops, per) = match item.as_str() {
            "ishalted" => (vec![code(2, &lb)], 1),
            "current" => (vec![code(1, &lb)], 1),
            "attach" => (vec![body(3, 0, -1, "")], 1),
            "closereopen" => (vec![code(6, ""), code(5, &lb)], 2),
            "reopenmiss" => (vec![code(5, &lb)], 1),
            _ => {
                eprintln!("perf: item must be ishalted|current|attach|closereopen|reopenmiss, got {}", item);
                std::process::exit(2);
            }
        };
        match lane.as_str() {
            "t" => typed(item, 0, &ops, per, None),
            "f" => fair(item, &ops, per),
            _ => {
                eprintln!("perf: lane must be t|f, got {}", lane);
                std::process::exit(2);
            }
        }
        return;
    }
    let mut w = setup(0);
    let ops = vec![body(0, -1, -1, &bid(last))];
    println!("dispatch {} {:.1}", size, time(&mut w, &ops, 1));
    println!("rchk dispatch {} {}", size, verify(&mut setup(0), &ops)); // pruv only
    // raw t_call with a prepared op (no World::call): what the dispatch probe costs inside t_call itself
    {
        let mut w = setup(0);
        let mut op = vec![0i64, w.now, 0];
        op.extend_from_slice(&body(0, -1, -1, &bid(last))[1..]);
        op.push(0);
        let t = Instant::now();
        for _ in 0..reps {
            let r = sol::t_call(&mut w.st, black_box(&op), &mut w.bv, &mut w.fx, &mut w.out);
            black_box((r, w.out[0]));
        }
        println!("rawcall {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
        let mut w = setup(0);
        let mut s = 0u64;
        for _ in 0..vn {
            w.fx[0] = 0;
            let r = sol::t_call(&mut w.st, &op, &mut w.bv, &mut w.fx, &mut w.out);
            s = rchk_fold(&w, r, s);
        }
        println!("rchk rawcall {} {}", size, s); // pruv only
    }
    // isHalted (no record copy)
    let mut w = setup(0);
    let ops = vec![body(2, -1, -1, &bid(last))];
    println!("ishalted {} {:.1}", size, time(&mut w, &ops, 1));
    println!("rchk ishalted {} {}", size, verify(&mut setup(0), &ops));
    typed("ishalted", 0, &ops, 1, None);
    fair("ishalted", &ops, 1);
    // current
    let mut w = setup(0);
    let ops = vec![body(1, -1, -1, &bid(last))];
    println!("current {} {:.1}", size, time(&mut w, &ops, 1));
    println!("rchk current {} {}", size, verify(&mut setup(0), &ops));
    typed("current", 0, &ops, 1, None);
    fair("current", &ops, 1);
    // attach (open budget: ref)
    let mut w = setup(0);
    let ops = vec![body(3, 0, -1, "")];
    println!("attach {} {:.1}", size, time(&mut w, &ops, 1));
    println!("rchk attach {} {}", size, verify(&mut setup(0), &ops));
    typed("attach", 0, &ops, 1, None);
    fair("attach", &ops, 1);
    let close: Vec<_> = sids.iter().map(|sid| {
        let mut v = vec![6, -1, -1];
        v.extend(unit);
        v.extend(key(sid));
        v.push(0);
        v
    }).collect();
    for direct in [false, true] {
        let mut w = setup(0);
        let open = w.st.clone();
        let ps: Vec<_> = close.iter().map(|b| prep_of(&mut w, b)).collect();
        let call = |w: &mut World, i: usize| {
            if direct { c_direct(w, &ps[i]) } else { w.call(black_box(&close[i])) }
        };
        for i in 0..size {
            assert_eq!(call(&mut w, i), 0);
        }
        let mut elapsed = std::time::Duration::ZERO;
        for start in (0..reps).step_by(size) {
            w.st.copy_from_slice(&open);
            let t = Instant::now();
            for i in 0..size.min(reps - start) {
                let r = call(&mut w, i);
                black_box((r, w.out[0]));
            }
            elapsed += t.elapsed();
        }
        let mut sum = 0;
        for start in (0..vn).step_by(size) {
            w.st.copy_from_slice(&open);
            for i in 0..size.min(vn - start) {
                let r = call(&mut w, i);
                assert_eq!((r, w.out[0], &w.fx[..2]), (0, 2, &[1, 20][..]));
                sum = rchk_fold(&w, r, sum);
            }
        }
        let ns = elapsed.as_nanos() as f64 / reps as f64;
        if direct {
            println!("t close {} {:.1}", size, ns);
            println!("rchk_t close {} {}", size, sum);
        } else {
            println!("close {} {:.1}", size, ns);
            println!("rchk close {} {}", size, sum);
        }
    }
    // close + reopen cycle: maybeClose (quiet) then reopen
    let mut w = setup(0);
    let ops = vec![body(6, -1, -1, ""), body(5, -1, -1, &bid(last))];
    println!("closereopen {} {:.1}", size, time(&mut w, &ops, 2));
    println!("rchk closereopen {} {}", size, verify(&mut setup(0), &ops));
    typed("closereopen", 0, &ops, 2, None);
    fair("closereopen", &ops, 2);
    // reopen on an open budget: CURRENT lookup + state test, answers [1], changes nothing (reopen's lookup alone)
    let mut w = setup(0);
    let ops = vec![body(5, -1, -1, &bid(last))];
    println!("reopenmiss {} {:.1}", size, time(&mut w, &ops, 1));
    println!("rchk reopenmiss {} {}", size, verify(&mut setup(0), &ops));
    typed("reopenmiss", 0, &ops, 1, None);
    fair("reopenmiss", &ops, 1);
    // replace on a halted budget (CLOSE + OPEN + follows + carryHeld), then put the halted record back
    let mut w = setup(1);
    let rp = body(4, 0, 2, "");
    let mut put = key(&sids[last]);
    let r = rec(&bid(last), 1, PERF_REST);
    put.push(r.len() as i64);
    put.extend(r);
    for _ in 0..1000 {
        w.next_uuid = 0;
        w.call(&rp);
        w.put(&put);
    }
    let t = Instant::now();
    for _ in 0..reps {
        w.next_uuid = 0;
        let r = w.call(black_box(&rp));
        black_box((r, w.out[0]));
        w.put(black_box(&put));
    }
    println!("replace {} {:.1}", size, t.elapsed().as_nanos() as f64 / reps as f64);
    // untimed verification: fresh halted setup, the same call + put-back per iteration
    let mut w = setup(1);
    let mut s = 0u64;
    for _ in 0..vn {
        w.next_uuid = 0;
        let r = w.call(&rp);
        s = rchk_fold(&w, r, s);
        w.put(&put);
    }
    println!("rchk replace {} {}", size, s);
    typed("replace", 1, std::slice::from_ref(&rp), 1, Some(&put));
}

// rchk: per-op result checksum of the perf verification pass, s = s*31 + w (wrapping u64). Per call it folds, in this
// order: the status r, the effect count fx[0], every effect code fx[1..=fx[0]] (13 timerClear, 20 changed, 21 carryHeld,
// 22 uuid), and, for status 0, the result words the host reads (the same words q mode prints after `V `):
// [0] undefined, [1] false, [2] true, [3, n, id units.. (, m, old id units.. when effect 21 is held)] a ref,
// [4, n, record words..] a budget view (st[off..off+n)). hand_m7_u16.rs perf produces the identical words.
fn rchk_fold(w: &World, r: i64, sum: u64) -> u64 {
    let mut s = sum;
    let mut fold = |x: i64| s = s.wrapping_mul(31).wrapping_add(x as u64);
    fold(r);
    let used = w.fx[0].max(0) as usize;
    fold(used as i64);
    for x in &w.fx[1..=used] {
        fold(*x);
    }
    if r != 0 {
        return s;
    }
    match w.out[0] {
        4 => {
            let n = w.out[1].max(0) as usize;
            let off = w.out[2].max(0) as usize;
            fold(4);
            fold(n as i64);
            for x in &w.st[off..off + n] {
                fold(*x);
            }
        }
        3 => {
            // the ref's packed ids are unpacked to units, so the folded words equal hand16's
            for x in ref_units(&w.out, w.fx[1..=used].contains(&21)) {
                fold(x);
            }
        }
        x => fold(x),
    }
    s
}

// ---- boundary <S> <reps> [item] [variant] (P01 boundary experiment, see run_m7.py cmd_boundary) ----
//
// Variants per item (current, ishalted, attach, closereopen, reopenmiss, replace):
//   A    World::call (host op build/cache + sol::t_call + effect handling)
//   B    sol::t_call on the prepared op Vec (the words A builds), no wrapper
//   Bnv  sol::t_call_nv: B without bud_store::valid(st)
//   C    sol::c_* typed entries (separate key slices, no key_end parsing, no valid)
//   prep the host op build of A only (no t_call)
//   observe  result consumption only (checksum fold), on the result of one call
// item noop: A and B with code 0 (unknown code, status 2).
// Every timed variant resets next_uuid and the host fact counter before each call and folds the full result (status, effect
// words fx[1..=fx[0]], and the out / budget-view words) into a checksum; observe consumes the same.
// Output line: `item variant S ns_per_call checksum allocs_per_call`.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[derive(Clone, Copy, PartialEq)]
enum Var {
    A,
    B,
    Bnv,
    C,
}

// Prepared inputs of one call: the op A would build and the typed pieces C takes from it.
struct Prep {
    op: Vec<i64>,
    code: i64,
    now: i64,
    origin: i64,
    size: i64,
    unit: Vec<i64>,
    // packed key words (no frame word) and unit counts
    sk: Vec<i64>,
    sn: i64,
    bk: Vec<i64>,
    bn: i64,
    uk: Vec<i64>,
    un: i64,
}

fn prep_of(w: &mut World, body: &[i64]) -> Prep {
    w.next_uuid = 0;
    bud_facts::set_calls(0);
    let (slot, op) = w.take_op(body);
    let p = op.clone();
    w.put_op(slot, op);
    let sn = p[11];
    let e1 = 12 + words(sn);
    let bn = p[e1];
    let e2 = e1 + 1 + words(bn);
    let un = p[e2];
    Prep {
        code: p[0],
        now: p[1],
        origin: p[3],
        size: p[4],
        unit: p[5..11].to_vec(),
        sk: p[12..e1].to_vec(),
        sn,
        bk: p[e1 + 1..e2].to_vec(),
        bn,
        uk: p[e2 + 1..e2 + 1 + words(un)].to_vec(),
        un,
        op: p,
    }
}

// Retry a call that answered Grow (-N), as World::call does.
fn raw_call(w: &mut World, op: &Vec<i64>, nv: bool) -> i64 {
    let mut r = 0;
    for _ in 0..8 {
        w.fx[0] = 0;
        r = if nv {
            sol::t_call_nv(&mut w.st, op, &mut w.bv, &mut w.fx, &mut w.out)
        } else {
            sol::t_call(&mut w.st, op, &mut w.bv, &mut w.fx, &mut w.out)
        };
        if r < 0 {
            w.grow((-r) as usize);
            continue;
        }
        break;
    }
    r
}

fn c_call(w: &mut World, p: &Prep) -> i64 {
    let mut r = 0;
    for _ in 0..8 {
        w.fx[0] = 0;
        r = match p.code {
            1 => sol::c_current(&w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.out),
            2 => sol::c_is_halted(&w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.out),
            3 => sol::c_attach(&mut w.st, black_box(&p.sk), p.sn, p.origin, p.size, &p.unit, black_box(&p.uk), p.un, p.now, &mut w.fx, &mut w.out),
            4 => sol::c_replace(&mut w.st, black_box(&p.sk), p.sn, p.origin, p.size, &p.unit, black_box(&p.uk), p.un, p.now, &mut w.bv, &mut w.fx, &mut w.out),
            5 => sol::c_reopen(&mut w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.fx, &mut w.out),
            6 => sol::c_maybe_close(&mut w.st, black_box(&p.sk), p.sn, p.now, &mut w.bv, &mut w.fx, &mut w.out),
            _ => 2,
        };
        if r < 0 {
            w.grow((-r) as usize);
            continue;
        }
        break;
    }
    r
}

// One direct typed call (the timed typed path); only a Grow answer falls back to the c_call retry loop.
#[inline(always)]
fn c_direct(w: &mut World, p: &Prep) -> i64 {
    w.fx[0] = 0;
    let r = match p.code {
        1 => sol::c_current(&w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.out),
        2 => sol::c_is_halted(&w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.out),
        3 => sol::c_attach(&mut w.st, black_box(&p.sk), p.sn, p.origin, p.size, &p.unit, black_box(&p.uk), p.un, p.now, &mut w.fx, &mut w.out),
        4 => sol::c_replace(&mut w.st, black_box(&p.sk), p.sn, p.origin, p.size, &p.unit, black_box(&p.uk), p.un, p.now, &mut w.bv, &mut w.fx, &mut w.out),
        5 => sol::c_reopen(&mut w.st, black_box(&p.sk), p.sn, black_box(&p.bk), p.bn, &mut w.fx, &mut w.out),
        6 => sol::c_maybe_close(&mut w.st, black_box(&p.sk), p.sn, p.now, &mut w.bv, &mut w.fx, &mut w.out),
        _ => 2,
    };
    if r < 0 {
        return c_call(w, p);
    }
    r
}

// ---- fair typed lane (T5c) ----
// The timed loop has the shape of hand_m7_u16's typed loop (`black_box(w.current(black_box(sid), black_box(lbid)))`):
// - the entry is chosen by a const generic, so there is no per-op match on the prep code and no step closure;
// - the key slices are borrowed once (FArgs), so each op passes a black_box'ed slice, as hand passes `black_box(sid)`;
// - `fx[0] = 0` only where hand clears fx (attach, reopen, maybeClose: hand runs `w.fx.clear()` before each call),
//   not for current / isHalted, which take no fx;
// - no Grow fallback: these five cells never answer Grow (no record grows); the rchk_f pass asserts r >= 0 per call;
// - the result is consumed as in the typed lane, `black_box((status, out[0]))`, the counterpart of hand's one
//   black_box of its result value.
// P18b: the lane calls the proven entries (sol::pruv_proven, emitted by `--proven-entries`). Each array is
// length-checked against 2^40 once, where the host acquires it (PruvArr::new / new_mut: the key slices in FArgs::of,
// the World buffers in FBuf::of before the timed loop), and the proven entry takes the checked type with no per-call
// guard. The guarded c_* entries (T0) stay for every other caller; the emptied-body floor empties the proven bodies.
struct FArgs<'a> {
    sk: &'a sol::PruvArr<i64>,
    sn: i64,
    bk: &'a sol::PruvArr<i64>,
    bn: i64,
    origin: i64,
    size: i64,
    unit: &'a sol::PruvArr<i64>,
    uk: &'a sol::PruvArr<i64>,
    un: i64,
    now: i64,
}

impl<'a> FArgs<'a> {
    fn of(p: &'a Prep) -> FArgs<'a> {
        let a = sol::PruvArr::new;
        FArgs { sk: a(&p.sk), sn: p.sn, bk: a(&p.bk), bn: p.bn, origin: p.origin, size: p.size, unit: a(&p.unit), uk: a(&p.uk), un: p.un, now: p.now }
    }
}

// The World buffers the fair entries take, each checked once (P18b).
struct FBuf<'a> {
    st: &'a mut sol::PruvArr<i64>,
    out: &'a mut sol::PruvArr<i64>,
    bv: &'a mut sol::PruvArr<i64>,
    fx: &'a mut sol::PruvArr<i64>,
}

impl<'a> FBuf<'a> {
    fn of(w: &'a mut World) -> FBuf<'a> {
        let m = sol::PruvArr::new_mut;
        FBuf { st: m(&mut w.st), out: m(&mut w.out), bv: m(&mut w.bv), fx: m(&mut w.fx) }
    }
}

// P18c: current and isHalted take the by-value form (sol::pruv_proven::val, `K` result words returned with the
// status, no `out` buffer): their bodies inline into the loop, so the result words stay in registers, as hand's
// `Option<&Budget>` / `bool` result does. attach, reopen and maybeClose keep the `out` twin (fair_out, the P18b
// shape unchanged): their bodies are calls, so a by-value result would be a zeroed stack array passed by pointer
// (measured slower, Appendix P18c). K: current 3 ([4, n, off] on a hit), isHalted 1.
const K_CURRENT: usize = 3;
const K_HALTED: usize = 1;

#[inline(always)]
fn fair_val<const C: i64, const K: usize>(b: &mut FBuf, a: &FArgs) -> (i64, [i64; K]) {
    let (r, o) = match C {
        1 => sol::pruv_proven::val::c_current::<K>(b.st, black_box(a.sk), a.sn, black_box(a.bk), a.bn),
        2 => sol::pruv_proven::val::c_is_halted::<K>(b.st, black_box(a.sk), a.sn, black_box(a.bk), a.bn),
        _ => unreachable!(),
    };
    black_box((r, o));
    (r, o)
}

#[inline(always)]
fn fair_out<const C: i64>(b: &mut FBuf, a: &FArgs) -> i64 {
    let r = match C {
        3 => {
            b.fx.get_mut()[0] = 0;
            sol::pruv_proven::c_attach(b.st, black_box(a.sk), a.sn, a.origin, a.size, a.unit, black_box(a.uk), a.un, a.now, b.fx, b.out)
        }
        5 => {
            b.fx.get_mut()[0] = 0;
            sol::pruv_proven::c_reopen(b.st, black_box(a.sk), a.sn, black_box(a.bk), a.bn, b.fx, b.out)
        }
        6 => {
            b.fx.get_mut()[0] = 0;
            sol::pruv_proven::c_maybe_close(b.st, black_box(a.sk), a.sn, a.now, b.bv, b.fx, b.out)
        }
        _ => unreachable!(),
    };
    black_box((r, b.out.get()[0]));
    r
}

// The timed fair loop of one item: `n` iterations; closereopen is one maybeClose + reopen pair per iteration, like
// hand's closereopen loop. The item match is outside the loop; the buffers are checked once, before it.
#[inline(never)]
fn fair_loop(item: &str, w: &mut World, a: &[FArgs], n: usize) {
    let b = &mut FBuf::of(w);
    match item {
        "ishalted" => for _ in 0..n { fair_val::<2, K_HALTED>(b, &a[0]); },
        "current" => for _ in 0..n { fair_val::<1, K_CURRENT>(b, &a[0]); },
        "attach" => for _ in 0..n { fair_out::<3>(b, &a[0]); },
        "reopenmiss" => for _ in 0..n { fair_out::<5>(b, &a[0]); },
        "closereopen" => for _ in 0..n {
            fair_out::<6>(b, &a[0]);
            fair_out::<5>(b, &a[1]);
        },
        _ => unreachable!("fair lane item {}", item),
    }
}

// Untimed check of the fair lane: the same fair_val / fair_out calls on a fresh setup, every call folded by rchk_fold (must equal
// rchk / rchk_t and hand16's rchk); a Grow answer would be a harness error here. The by-value words are put in
// w.out[..K] first (the `out` cells pass none: theirs are already there), so rchk_fold reads the by-value result.
fn fair_rchk(item: &str, w: &mut World, a: &[FArgs], n: usize) -> u64 {
    let mut s = 0u64;
    let mut fold = |w: &mut World, r: i64, o: &[i64]| {
        assert!(r >= 0, "fair lane: {} answered {}", item, r);
        w.out[..o.len()].copy_from_slice(o);
        s = rchk_fold(w, r, s);
    };
    for _ in 0..n {
        match item {
            "ishalted" => { let (r, o) = fair_val::<2, K_HALTED>(&mut FBuf::of(w), &a[0]); fold(w, r, &o) }
            "current" => { let (r, o) = fair_val::<1, K_CURRENT>(&mut FBuf::of(w), &a[0]); fold(w, r, &o) }
            "attach" => { let r = fair_out::<3>(&mut FBuf::of(w), &a[0]); fold(w, r, &[]) }
            "reopenmiss" => { let r = fair_out::<5>(&mut FBuf::of(w), &a[0]); fold(w, r, &[]) }
            "closereopen" => {
                let r = fair_out::<6>(&mut FBuf::of(w), &a[0]);
                fold(w, r, &[]);
                let r = fair_out::<5>(&mut FBuf::of(w), &a[1]);
                fold(w, r, &[])
            }
            _ => unreachable!(),
        }
    }
    s
}

// One call of variant v with the same reset of the host's UUID / isQuiet streams.
fn one(v: Var, w: &mut World, body: &[i64], p: &Prep) -> i64 {
    w.next_uuid = 0;
    bud_facts::set_calls(0);
    match v {
        Var::A => w.call(black_box(body)),
        Var::B => raw_call(w, black_box(&p.op), false),
        Var::Bnv => raw_call(w, black_box(&p.op), true),
        Var::C => c_direct(w, p),
    }
}

// Fold the full result into the checksum, identically for every variant (A, B, Bnv, C) and for observe:
// the status; every effect word fx[1..=fx[0]] (effects are bare codes, they carry no payload words in fx; the
// words they refer to live in out and are folded below), also after a failed call (fx is reset before each
// call by all variants); and for status 0 a budget view (st[off..off+n)), a ref (out[..2+n], plus the carried
// old id when effect [21] is held) or the tag.
fn consume(w: &World, r: i64, sum: u64) -> u64 {
    let mut s = sum.wrapping_mul(1000003).wrapping_add(r as u64);
    let used = w.fx[0].max(0) as usize;
    s = s.wrapping_mul(31).wrapping_add(used as u64);
    for x in &w.fx[1..=used] {
        s = s.wrapping_mul(31).wrapping_add(*x as u64);
    }
    if r != 0 {
        return s;
    }
    let mut fold = |x: i64| s = s.wrapping_mul(31).wrapping_add(x as u64);
    match w.out[0] {
        4 => {
            let n = w.out[1].max(0) as usize;
            let off = w.out[2].max(0) as usize;
            for x in &w.st[off..off + n] {
                fold(*x);
            }
        }
        3 => {
            let used = w.fx[0].max(0) as usize;
            // packed ref words: [3, n, words(n) id words (, m, words(m) old id words)]
            let k = ref_len(&w.out, w.fx[1..=used].contains(&21));
            for x in &w.out[..k] {
                fold(*x);
            }
        }
        x => fold(x),
    }
    s
}

struct Item {
    name: &'static str,
    bodies: Vec<Vec<i64>>,
    state: i64,
    put: Option<Vec<i64>>,
}

fn run_boundary(args: &[String]) {
    bud_facts::set_answers(&[], 1);
    let size: usize = args[0].parse().unwrap();
    let reps: usize = args[1].parse().unwrap();
    let only_item = args.get(2).map(|s| s.as_str()).filter(|s| *s != "all");
    let only_var = args.get(3).map(|s| s.as_str()).filter(|s| *s != "all");
    let sids: Vec<String> = (0..size).map(|i| format!("s{:04}", i)).collect();
    let bid = |i: usize| format!("{:08x}-0000-4000-8000-000000000001", i);
    let unit = [120i64, 24, 3, 2, 1, 0];
    let last = size - 1;
    let setup = |it: &Item| -> World {
        let mut w = World::new();
        w.silent = true;
        for (i, s) in sids.iter().enumerate() {
            let mut op = key(s);
            let r = rec(&bid(i), it.state, PERF_REST);
            op.push(r.len() as i64);
            op.extend(r);
            w.put(&op);
        }
        w.uuids = vec![key("00000000-0000-4000-8000-0000000000aa")];
        w
    };
    let body = |code: i64, origin: i64, sz: i64, b: &str| -> Vec<i64> {
        let mut v = vec![code, origin, sz];
        v.extend_from_slice(&unit);
        v.extend(key(&sids[last]));
        v.extend(key(b));
        v
    };
    let mut put = key(&sids[last]);
    let r = rec(&bid(last), 1, PERF_REST);
    put.push(r.len() as i64);
    put.extend(r);
    let close = sids.iter().map(|sid| {
        let mut v = vec![6, -1, -1];
        v.extend(unit);
        v.extend(key(sid));
        v.push(0);
        v
    }).collect();
    let items = vec![
        Item { name: "current", bodies: vec![body(1, -1, -1, &bid(last))], state: 0, put: None },
        Item { name: "ishalted", bodies: vec![body(2, -1, -1, &bid(last))], state: 0, put: None },
        Item { name: "attach", bodies: vec![body(3, 0, -1, "")], state: 0, put: None },
        Item {
            name: "closereopen",
            bodies: vec![body(6, -1, -1, ""), body(5, -1, -1, &bid(last))],
            state: 0,
            put: None,
        },
        Item { name: "close", bodies: close, state: 0, put: None },
        Item { name: "reopenmiss", bodies: vec![body(5, -1, -1, &bid(last))], state: 0, put: None },
        Item { name: "replace", bodies: vec![body(4, 0, 2, "")], state: 1, put: Some(put) },
        Item { name: "noop", bodies: vec![body(0, -1, -1, &bid(last))], state: 0, put: None },
    ];

    // One sequence of an item (all its calls, then the put): used by the self-check and the timed loops.
    let seq = |it: &Item, v: Var, w: &mut World, ps: &[Prep], sum: &mut u64| {
        for (i, b) in it.bodies.iter().enumerate() {
            let r = black_box(one(v, w, b, &ps[i]));
            *sum = consume(w, r, *sum);
        }
        if let Some(p) = &it.put {
            w.put(black_box(p));
        }
    };

    // Self-check: each variant once on fresh state, in lockstep with A; after every call the status, out, fx, bv and
    // state words must equal A's (compared per step, so memory stays linear in the state size).
    for it in &items {
        if only_item.map_or(false, |o| o != it.name) {
            continue;
        }
        let vars: &[Var] = if it.name == "noop" { &[Var::B] } else { &[Var::B, Var::Bnv, Var::C] };
        let expect = if it.name == "noop" { 2 } else { 0 };
        for &v in vars {
            let mut wa = setup(it);
            let mut wv = setup(it);
            let pa: Vec<Prep> = it.bodies.iter().map(|b| prep_of(&mut wa, b)).collect();
            let pv: Vec<Prep> = it.bodies.iter().map(|b| prep_of(&mut wv, b)).collect();
            for (i, b) in it.bodies.iter().enumerate() {
                let ra = one(Var::A, &mut wa, b, &pa[i]);
                let rv = one(v, &mut wv, b, &pv[i]);
                if ra != expect || rv != expect {
                    eprintln!("self-check: {} step {} status A {} variant {} {} (want {})", it.name, i, ra, v as usize, rv, expect);
                    std::process::exit(2);
                }
                if (&wa.out, &wa.fx, &wa.bv, &wa.st) != (&wv.out, &wv.fx, &wv.bv, &wv.st) {
                    eprintln!("self-check: {} step {} variant {} differs from A (out/fx/bv/state words)", it.name, i, v as usize);
                    std::process::exit(2);
                }
            }
        }
    }

    for it in &items {
        if only_item.map_or(false, |o| o != it.name) {
            continue;
        }
        let names: &[&str] = if it.name == "noop" { &["A", "B"] } else { &["A", "B", "Bnv", "C", "prep", "observe"] };
        for &vn in names {
            if only_var.map_or(false, |o| o != vn) {
                continue;
            }
            let mut w = setup(it);
            let ps: Vec<Prep> = it.bodies.iter().map(|b| prep_of(&mut w, b)).collect();
            let per = it.bodies.len();
            let mut sum: u64 = 0;
            let (ns, allocs);
            if it.name == "close" && matches!(vn, "prep" | "observe") {
                // exactly reps operations, cycling the sessions as the timed close loop does (one close per op)
                let mut r = 0;
                if vn == "observe" {
                    r = one(Var::A, &mut w, &it.bodies[0], &ps[0]);
                }
                let mut op_k = |w: &mut World, k: usize, sum: &mut u64| {
                    if vn == "prep" {
                        let b = &it.bodies[k % size];
                        w.next_uuid = 0;
                        bud_facts::set_calls(0);
                        let (slot, op) = w.take_op(black_box(b));
                        *sum = sum.wrapping_add(black_box(&op).len() as u64);
                        w.put_op(slot, op);
                    } else {
                        *sum = consume(black_box(w), black_box(r), *sum);
                    }
                };
                for k in 0..1000 {
                    op_k(&mut w, k, &mut sum);
                }
                sum = 0;
                let a0 = ALLOCS.load(Ordering::Relaxed);
                let t = Instant::now();
                for k in 0..reps {
                    op_k(&mut w, k, &mut sum);
                }
                ns = t.elapsed().as_nanos() as f64 / reps as f64;
                allocs = (ALLOCS.load(Ordering::Relaxed) - a0) as f64 / reps as f64;
                println!("{} {} {} {:.2} {} {:.3}", it.name, vn, size, ns, black_box(sum), allocs);
                continue;
            }
            if it.name == "close" && matches!(vn, "A" | "B" | "Bnv" | "C") {
                let v = match vn { "A" => Var::A, "B" => Var::B, "Bnv" => Var::Bnv, _ => Var::C };
                let open = w.st.clone();
                seq(it, v, &mut w, &ps, &mut sum);
                sum = 0;
                let mut elapsed = std::time::Duration::ZERO;
                let mut allocated = 0;
                for start in (0..reps).step_by(size) {
                    w.st.copy_from_slice(&open);
                    let a0 = ALLOCS.load(Ordering::Relaxed);
                    let t = Instant::now();
                    for i in 0..size.min(reps - start) {
                        let r = one(v, &mut w, &it.bodies[i], &ps[i]);
                        sum = consume(&w, r, sum);
                    }
                    elapsed += t.elapsed();
                    allocated += ALLOCS.load(Ordering::Relaxed) - a0;
                }
                ns = elapsed.as_nanos() as f64 / reps as f64;
                allocs = allocated as f64 / reps as f64;
                println!("{} {} {} {:.2} {} {:.3}", it.name, vn, size, ns, black_box(sum), allocs);
                continue;
            }
            match vn {
                "prep" => {
                    let mut once = |w: &mut World, sum: &mut u64| {
                        for b in &it.bodies {
                            w.next_uuid = 0;
                            bud_facts::set_calls(0);
                            let (slot, op) = w.take_op(black_box(b));
                            *sum = sum.wrapping_add(black_box(&op).len() as u64);
                            w.put_op(slot, op);
                        }
                    };
                    for _ in 0..1000 {
                        once(&mut w, &mut sum);
                    }
                    sum = 0;
                    let a0 = ALLOCS.load(Ordering::Relaxed);
                    let t = Instant::now();
                    for _ in 0..reps {
                        once(&mut w, &mut sum);
                    }
                    ns = t.elapsed().as_nanos() as f64 / (reps * per) as f64;
                    allocs = (ALLOCS.load(Ordering::Relaxed) - a0) as f64 / (reps * per) as f64;
                }
                "observe" => {
                    // one call of A leaves the result words; only their consumption is timed
                    let mut s0 = 0u64;
                    let mut r = 0;
                    for (i, b) in it.bodies.iter().enumerate() {
                        r = one(Var::A, &mut w, b, &ps[i]);
                    }
                    s0 = consume(&w, r, s0);
                    let _ = s0;
                    for _ in 0..1000 {
                        for _ in 0..per {
                            sum = consume(black_box(&w), black_box(r), sum);
                        }
                    }
                    sum = 0;
                    let a0 = ALLOCS.load(Ordering::Relaxed);
                    let t = Instant::now();
                    for _ in 0..reps {
                        for _ in 0..per {
                            sum = consume(black_box(&w), black_box(r), sum);
                        }
                    }
                    ns = t.elapsed().as_nanos() as f64 / (reps * per) as f64;
                    allocs = (ALLOCS.load(Ordering::Relaxed) - a0) as f64 / (reps * per) as f64;
                }
                _ => {
                    let v = match vn {
                        "A" => Var::A,
                        "B" => Var::B,
                        "Bnv" => Var::Bnv,
                        _ => Var::C,
                    };
                    for _ in 0..1000 {
                        seq(it, v, &mut w, &ps, &mut sum);
                    }
                    sum = 0;
                    let a0 = ALLOCS.load(Ordering::Relaxed);
                    let t = Instant::now();
                    for _ in 0..reps {
                        seq(it, v, &mut w, &ps, &mut sum);
                    }
                    ns = t.elapsed().as_nanos() as f64 / (reps * per) as f64;
                    allocs = (ALLOCS.load(Ordering::Relaxed) - a0) as f64 / (reps * per) as f64;
                }
            }
            println!("{} {} {} {:.2} {} {:.3}", it.name, vn, size, ns, black_box(sum), allocs);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "perf" {
        run_perf(&args[2..]);
    } else if args.len() > 1 && args[1] == "boundary" {
        run_boundary(&args[2..]);
    } else {
        run_q();
    }
}

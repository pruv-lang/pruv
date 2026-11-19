// M12 line protocol is documented in ABI.md.
#[path = "@SOL@"] mod sol;
#[path = "@HAND@"] mod hand;
use std::io::{self, BufRead, Write};
use std::time::Instant;
use std::hint::black_box;

struct World {
    st: Vec<i64>,
    out: Vec<i64>,
    grows: i64,
}

impl World {
    fn new() -> World {
        let mut st = vec![0i64; 4096];
        let r = sol::m12_init(&mut st);
        if r > 0 {
            st = vec![0i64; r as usize];
            assert_eq!(sol::m12_init(&mut st), 0);
        }
        World { st, out: vec![0; 4096], grows: 0 }
    }
    fn grow(&mut self) -> bool {
        let n = self.out[0].max(0) as usize;
        let mut fresh = vec![0i64; n]; // N from Pruv (bud_store::need, N >= 2*len), L3 P13b
        if sol::m12_grow(&self.st, &mut fresh) != 0 {
            return false;
        }
        self.st = fresh;
        self.grows += 1;
        true
    }
    // runs f with retries on Grow (1) and out too small (3); each retry at least doubles a buffer,
    // so 64 rounds cover every size the ABI allows
    fn call(&mut self, f: &dyn Fn(&mut Vec<i64>, &mut Vec<i64>) -> i64) -> Result<&[i64], i64> {
        for _ in 0..64 {
            let s = f(&mut self.st, &mut self.out);
            match s {
                0 => {
                    let k = (self.out[0].max(0) as usize).min(self.out.len() - 1);
                    return Ok(&self.out[1..1 + k]);
                }
                1 => {
                    if !self.grow() {
                        return Err(1);
                    }
                }
                3 => {
                    let n = 2 * self.out.len();
                    self.out.resize(n, 0);
                }
                s => return Err(s),
            }
        }
        Err(9)
    }
}

fn words(v: &[i64]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")
}

fn res(r: Result<&[i64], i64>) -> String {
    match r {
        Ok(w) => format!("ok {}", words(w)),
        Err(s) => format!("err {}", s),
    }
}

fn args(rest: &str) -> Vec<Vec<i64>> {
    rest.split('|').map(|p| p.split_whitespace().map(|x| x.parse::<i64>().unwrap()).collect()).collect()
}

// ---- perf inputs (host builds input words; no judgment) ----

// packed-u16 words of units (same layout as run_m11.py pack / driver_m8.rs pack_u16)
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
// framed packed key `n, words(n)` of a string
fn pkey(s: &str) -> Vec<i64> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut v = vec![u.len() as i64];
    v.extend(pack_u16(&u));
    v
}

fn key(o: &mut Vec<i64>, k: &str) {
    o.push(k.len() as i64);
    o.extend(k.bytes().map(|b| b as i64));
}
fn sv(o: &mut Vec<i64>, s: &str) {
    o.push(6);
    o.push(s.encode_utf16().count() as i64);
    o.extend(s.encode_utf16().map(|u| u as i64));
}
// ---- JB byte packets (big-endian; see hand.rs header): 6 str, 7 f64 bits, 8 array, 9 object ----
fn bkey(o: &mut Vec<u8>, k: &str) {
    o.extend_from_slice(&(k.len() as u32).to_be_bytes());
    o.extend_from_slice(k.as_bytes());
}
fn bstr(o: &mut Vec<u8>, s: &str) {
    o.push(6);
    bkey(o, s);
}
fn bnum(o: &mut Vec<u8>, x: f64) {
    o.push(7);
    o.extend_from_slice(&x.to_be_bytes());
}
fn bhead(o: &mut Vec<u8>, tag: u8, n: u32) {
    o.push(tag);
    o.extend_from_slice(&n.to_be_bytes());
}
// the harness input words of a byte argument: each word must be 0..255
fn bytes_of(w: &[i64]) -> Vec<u8> {
    w.iter().map(|&x| u8::try_from(x).expect("byte word outside 0..255")).collect()
}
fn perf_ref() -> Vec<i64> {
    let mut o = vec![9, 2];
    key(&mut o, "sessionId");
    sv(&mut o, "s1");
    key(&mut o, "budgetId");
    sv(&mut o, "b1");
    o
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, layout, n)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) { System.dealloc(p, layout); }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

enum Lane {
    Pruv(World),
    Hand { store: hand::Store, order: Vec<Vec<i64>>, out: Vec<i64> },
}

impl Lane {
    fn new(hand: bool) -> Self {
        if hand {
            Self::Hand { store: hand::Store::new(), order: Vec::new(), out: Vec::with_capacity(4096) }
        } else {
            Self::Pruv(World::new())
        }
    }

    fn load(&mut self, key: &[i64], budget: &[i64]) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_load(st, key, budget, out)),
            Self::Hand { store, order, out } => {
                let fresh = store.load(key, budget);
                if fresh == 1 { order.push(key.to_vec()); }
                out.clear();
                out.extend([fresh]);
                Ok(out)
            }
        }
    }

    fn auto(&mut self, op: i64, rf: &[i64], sid: &[i64], caller: &[i64], packet: &[u8]) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_auto(st, op, rf, sid, caller, packet, out)),
            Self::Hand { store, out, .. } => hand_result(store.auto(op, rf, sid, caller, packet, out), out),
        }
    }

    fn set(&mut self, a: &[Vec<i64>]) -> Result<&[i64], i64> {
        let (card, lang, by) = (bytes_of(&a[3]), bytes_of(&a[4]), bytes_of(&a[5]));
        let now = a[0][0];
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_set_card(st, &a[1], &a[2], &card, &lang, &by, now, out)),
            Self::Hand { store, out, .. } => {
                hand_result(store.set_status(&a[1], &a[2], &card, &lang, &by, now, out), out)
            }
        }
    }

    fn query(&mut self, a: &[Vec<i64>]) -> Result<&[i64], i64> {
        let op = a[0][0];
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_query(st, op, &a[1], &a[2], out)),
            Self::Hand { store, out, .. } => hand_result(store.query(op, &a[1], &a[2], out), out),
        }
    }

    fn dump(&mut self) -> Result<&[i64], i64> {
        match self {
            Self::Pruv(w) => w.call(&|st, out| sol::m12_dump(st, out)),
            Self::Hand { store, order, out } => {
                store.dump(order, out);
                Ok(&out[1..])
            }
        }
    }
}

fn hand_result(s: i64, out: &[i64]) -> Result<&[i64], i64> {
    if s == 0 { Ok(&out[1..]) } else { Err(s) }
}

fn perf_budget(item: &str, card_text: &str) -> Vec<i64> {
    let mut v = vec![9, if item == "C" || item == "T" { 4 } else { 3 }];
    key(&mut v, "id");
    sv(&mut v, "b1");
    key(&mut v, "state");
    sv(&mut v, "open");
    // Keep the large card after autoContinue to exercise the clear tail move.
    if item == "C" || item == "T" {
        key(&mut v, "autoContinue");
        v.extend([9, 2]);
        key(&mut v, "sessionId");
        sv(&mut v, "s1");
        key(&mut v, "stopped");
        v.push(3);
    }
    key(&mut v, "card");
    v.extend([9, 3]);
    key(&mut v, "state");
    sv(&mut v, "working");
    key(&mut v, "by");
    v.extend([9, 1]);
    key(&mut v, "sessionId");
    sv(&mut v, "s1");
    key(&mut v, "deliverable");
    sv(&mut v, card_text);
    v
}

fn perf(is_hand: bool, item: &str, size: usize, reps: usize) -> String {
    let text = "x".repeat(size);
    let sid = pkey("s1");
    let rf = perf_ref();
    let mut record = Vec::new();
    bhead(&mut record, 9, 4);
    bkey(&mut record, "sessionId");
    bstr(&mut record, "s1");
    bkey(&mut record, "userTurnId");
    bstr(&mut record, &text);
    bkey(&mut record, "count");
    bnum(&mut record, 99.0);
    bkey(&mut record, "stopped");
    record.push(3);
    let (op, reference, ref_sid, packet) = match item {
        "R" => (2, rf.as_slice(), sid.as_slice(), record.as_slice()),
        "C" => (1, rf.as_slice(), sid.as_slice(), &[][..]),
        "T" | "S" => (3, &[1][..], &[][..], &[][..]),
        _ => panic!("unknown perf item"),
    };
    let initial = perf_budget(item, if item == "R" { "" } else { &text });
    // Preallocate room using a large initial card. Then load the actual input
    // and reuse the same store buffers between measured batches.
    let capacity = perf_budget(item, &"x".repeat(size + 128));
    let width = reps.min(64);
    let mut lanes: Vec<_> = (0..width).map(|_| {
        let mut lane = Lane::new(is_hand);
        lane.load(&sid, &capacity).unwrap();
        lane
    }).collect();
    let mut time = std::time::Duration::ZERO;
    let mut allocs = 0;
    let mut checksum = 0;
    let mut remaining = reps;
    while remaining > 0 {
        let batch = &mut lanes[..remaining.min(width)];
        for lane in &mut *batch { lane.load(&sid, &initial).unwrap(); }
        let before = ALLOCS.load(Ordering::Relaxed);
        let start = Instant::now();
        for lane in &mut *batch {
            let out = black_box(lane.auto(op, reference, ref_sid, &sid, packet).unwrap());
            checksum += out[3];
        }
        time += start.elapsed();
        allocs += ALLOCS.load(Ordering::Relaxed) - before;
        remaining -= batch.len();
    }
    format!("ns {:.1} {} alloc {}", time.as_nanos() as f64 / reps as f64, checksum, allocs)
}

fn dispatch(lane: &mut Lane, cmd: &str, rest: &str) -> String {
    let a = args(rest);
    match cmd {
        "load" => res(lane.load(&a[0], &a[1])),
        "auto" => res(lane.auto(a[0][0], &a[1], &a[2], &a[3], &bytes_of(&a[4]))),
        "set" => res(lane.set(&a)),
        "q" => res(lane.query(&a)),
        "dump" => res(lane.dump()),
        // test probe, Pruv lane only: Grow count so far and free arena words (min(len, word 11) - word 10)
        "room" => match lane {
            Lane::Pruv(w) => format!("ok {} {}", w.grows, (w.st.len() as i64).min(w.st[11]) - w.st[10]),
            Lane::Hand { .. } => String::from("ok 0 0"),
        },
        _ => panic!("unknown input command"),
    }
}

fn main() {
    let is_hand = std::env::args().nth(1).as_deref() == Some("--hand");
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    let mut lane = None;
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let (cmd, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let answer = if cmd == "init" {
            lane = Some(Lane::new(is_hand));
            String::from("ok")
        } else if let Some(item) = cmd.strip_prefix("perf").or_else(|| cmd.strip_prefix("hand")) {
            let a = args(rest);
            perf(cmd.starts_with("hand"), item, a[0][0] as usize, a[0][1] as usize)
        } else {
            dispatch(lane.as_mut().expect("init required"), cmd, rest)
        };
        writeln!(o, "{}", answer).unwrap();
    }
}

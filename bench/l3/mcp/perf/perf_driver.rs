// P05M packet-core driver (run_perf.py writes work/main.rs = header + include!("gen.rs") + this file, with the @...@
// names of the generated Pruv items filled in; hand_packet.rs is mounted as `mod hand`).
//
// A "chain" is one or more tools/list pages joined with commas (file names); a call of a row processes the pages in
// order, each page continuing the (packet, count, objects) of the previous one, starting from the 5010-word packet.
// Parsing happens once up front, outside every timed region.
//
//   perf_driver check <chain>...        run old, new (when wired), hand on every chain, compare the full packet of every
//                                       stage word for word; prints `chain len sum` once per chain (the hand packet);
//                                       exit 1 on any mismatch
//   perf_driver time <row> <chain> <ms> per-call ns (median/min/max over calls for ~<ms> ms) + checksum of the output
//   perf_driver instr <row> <chain> <reps>   reps bare calls (nothing else timed or folded): for /usr/bin/time -l
//   perf_driver counters <row> <chain>  allocation calls / bytes of one call (counting global allocator) + output size
// rows: old new hand. Output checksum = fold s = s*31 + w (wrapping, i64) over every word.
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

// ---- counting allocator: every Pruv copy is a fresh allocation, so allocated bytes ~ copied bytes ----
struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size() as u64, Relaxed);
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size() as u64, Relaxed);
        System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        REALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(n as u64, Relaxed);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

type Doc = @DOC@;

fn parse_doc(b: &[u8]) -> Doc {
    match @PARSE@(b) {
        @PARSED@(d) => d,
        _ => panic!("parse failed"),
    }
}

// The packet every cell starts from: the shape root.pruv's mcp_check gives it (len 5010, p[1]=1, p[4]=-1, p[6]=-1).
fn start_packet() -> Vec<i64> {
    let mut p = vec![0i64; 5010];
    p[1] = 1;
    p[4] = -1;
    p[6] = -1;
    p
}

fn fold(p: &[i64]) -> i64 {
    let mut s: i64 = 0;
    for &w in p {
        s = s.wrapping_mul(31).wrapping_add(w);
    }
    s
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Old,
    New,
    Hand,
}

fn row_of(s: &str) -> Row {
    match s {
        "old" => Row::Old,
        "new" => Row::New,
        "hand" => Row::Hand,
        _ => panic!("row must be old|new|hand"),
    }
}

const HAS_NEW: bool = @HAS_NEW@;

#[inline(never)]
fn run_old(p: &[i64], d: &Doc, c: i64, o: i64) -> Vec<i64> {
    @CORE_OLD@(p, d, c, o)
}
#[inline(never)]
fn run_new(p: &[i64], d: &Doc, c: i64, o: i64) -> Vec<i64> {
    @CORE_NEW@
}
#[inline(never)]
fn run_hand(d: &Doc, p: &[i64], c: i64, o: i64) -> (Vec<i64>, i64, i64) {
    hand::core_hand(&d.source, &d.tape, p, c, o)
}

struct Page {
    d: Doc,
    c: i64,
    o: i64,
}

// Parse the chain and derive (count, objects) at the start of each page from the hand row.
fn load(chain: &str) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut p = start_packet();
    let (mut c, mut o) = (0i64, 0i64);
    let files: Vec<&str> = chain.split(',').collect();
    for (i, f) in files.iter().enumerate() {
        let bytes = std::fs::read(f).unwrap_or_else(|e| panic!("read {}: {}", f, e));
        let d = parse_doc(&bytes);
        // The last page needs no hand pass: (count, objects) only feed the next page. Skipping it keeps a freed
        // hand-sized packet out of every row's process, so peak RSS measures the row's own call (R00).
        if i + 1 == files.len() {
            pages.push(Page { d, c, o });
            break;
        }
        let (np, nc, no) = run_hand(&d, &p, c, o);
        pages.push(Page { d, c, o });
        p = np;
        c = nc;
        o = no;
    }
    pages
}

// One call of a row: the whole chain; returns the final packet.
#[inline(never)]
fn call(row: Row, pages: &[Page], start: &[i64]) -> Vec<i64> {
    let mut cur: Vec<i64> = Vec::new();
    for (i, pg) in pages.iter().enumerate() {
        let inp: &[i64] = if i == 0 { start } else { &cur };
        let next = match row {
            Row::Old => run_old(inp, &pg.d, pg.c, pg.o),
            Row::New => run_new(inp, &pg.d, pg.c, pg.o),
            Row::Hand => run_hand(&pg.d, inp, pg.c, pg.o).0,
        };
        cur = next;
    }
    cur
}

fn check(chains: &[String]) -> i32 {
    let mut bad = 0;
    for chain in chains {
        let mut pl = Vec::new();
        let mut packets: [Vec<i64>; 3] = [start_packet(), start_packet(), start_packet()];
        let (mut c, mut o) = (0i64, 0i64);
        let mut stage = 0;
        for f in chain.split(',') {
            let bytes = std::fs::read(f).unwrap();
            let d = parse_doc(&bytes);
            let (hp, nc, no) = run_hand(&d, &packets[2], c, o);
            let op = run_old(&packets[0], &d, c, o);
            let np = if HAS_NEW { run_new(&packets[1], &d, c, o) } else { hp.clone() };
            for (name, got) in [("old", &op), ("new", &np)] {
                if name == "new" && !HAS_NEW {
                    continue;
                }
                if got.len() != hp.len() {
                    eprintln!("MISMATCH {} stage {}: {} len {} vs hand {}", chain, stage, name, got.len(), hp.len());
                    bad += 1;
                } else if let Some(i) = (0..hp.len()).find(|&i| got[i] != hp[i]) {
                    eprintln!("MISMATCH {} stage {}: {} word {} = {} vs hand {}", chain, stage, name, i, got[i], hp[i]);
                    bad += 1;
                }
            }
            // each row continues from its own packet only while it agrees; on a mismatch fall back to hand's
            packets = [if op == hp { op } else { hp.clone() }, if HAS_NEW && np == hp { np } else { hp.clone() }, hp];
            c = nc;
            o = no;
            stage += 1;
            pl.push(f.to_string());
        }
        println!("{} {} {} {} {}", chain, packets[2].len(), fold(&packets[2]), c, o);
    }
    if bad > 0 {
        1
    } else {
        0
    }
}

fn pct(v: &[u64], q: usize) -> u64 {
    v[(v.len() - 1) * q / 100]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map(|s| s.as_str()).unwrap_or("");
    match mode {
        "check" => std::process::exit(check(&a[2..])),
        "time" => {
            let row = row_of(&a[2]);
            let pages = load(&a[3]);
            let budget_ns: u128 = a[4].parse::<u128>().unwrap() * 1_000_000;
            let start = start_packet();
            black_box(call(row, &pages, black_box(&start)));
            let mut t: Vec<u64> = Vec::new();
            let all = Instant::now();
            while t.len() < 3 || (all.elapsed().as_nanos() < budget_ns && t.len() < 2_000_000) {
                let t0 = Instant::now();
                let r = call(row, &pages, black_box(&start));
                black_box(&r);
                drop(r);
                t.push(t0.elapsed().as_nanos() as u64);
            }
            let sum = fold(&call(row, &pages, &start));
            t.sort_unstable();
            // Trimmed mean (slowest 5% dropped): sub-tick resolution for small cells, where the per-call median is
            // quantized to the 41.67 ns timer tick (P05N).
            let keep = &t[..(t.len() * 95 / 100).max(1)];
            let tmean = keep.iter().map(|&x| x as f64).sum::<f64>() / keep.len() as f64;
            println!("{} {} {} {} {} {:.1}", pct(&t, 50), t[0], t[t.len() - 1], t.len(), sum, tmean);
        }
        "instr" => {
            let row = row_of(&a[2]);
            let pages = load(&a[3]);
            let reps: usize = a[4].parse().unwrap();
            let start = start_packet();
            for _ in 0..reps {
                let r = call(row, &pages, black_box(&start));
                black_box(r.len());
            }
        }
        "counters" => {
            let row = row_of(&a[2]);
            let pages = load(&a[3]);
            let start = start_packet();
            drop(call(row, &pages, &start)); // warm-up
            let (a0, r0, b0) = (ALLOCS.load(Relaxed), REALLOCS.load(Relaxed), BYTES.load(Relaxed));
            let r = call(row, &pages, black_box(&start));
            let (a1, r1, b1) = (ALLOCS.load(Relaxed), REALLOCS.load(Relaxed), BYTES.load(Relaxed));
            println!("{} {} {} {} {}", a1 - a0, b1 - b0, r1 - r0, r.len(), fold(&r));
        }
        _ => {
            eprintln!("usage: perf_driver check|time|instr|counters ...");
            std::process::exit(2);
        }
    }
}

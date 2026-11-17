// P01 hand Rust lane: natural safe Rust over UTF-16 keys, HashMap<Box<[u16]>, u32> with either the std hasher
// (SipHash, `sip`, default) or an inline FxHasher (rustc-hash 1.x algorithm, no crates, `fx`).
//
//   hand_keys <set> <N> <reps> [row|all] [sip|fx]
//       same key sets, N, miss keys, warm-up, black_box and checksum rule as driver_keys perf.
//       rows: ret  map.get(&[u16]) of key j (hits; value = insertion index, so the checksum equals the Pruv ret row)
//             miss map.get of an absent key
//             raw  identical to ret: the raw input of this lane is already &[u16], there is nothing to encode/check
//             cmp  slice Ord on &[u16] (UTF-16 code-unit lexical order) of key j vs key j+1 (cyclic)
//       There is no enc row here (no encode step); checksums are the same for sip and fx.
//       output: `set hand <sip|fx> row N ns_per_op checksum words_per_key bytes_per_key` (2 bytes per unit, no header)
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, Hasher};
use std::hint::black_box;
use std::time::Instant;

fn key_units(set: &str, i: usize) -> Vec<u16> {
    match set {
        "sid" => format!("session-{:05}", i).encode_utf16().collect(),
        "uuid" => format!("99b568f7-0000-4000-8000-{:012}", i).encode_utf16().collect(),
        "nonascii" => format!("세션-{:05}-ключ", i).encode_utf16().collect(),
        "surr" => {
            let mut v: Vec<u16> = format!("surr-{:05}-", i).encode_utf16().collect();
            v.extend_from_slice(&[0xD800, 0x78, 0xDC00, 0xD83D, 0xDE00]);
            v
        }
        other => {
            eprintln!("unknown key set {}", other);
            std::process::exit(2);
        }
    }
}

fn bench<F: FnMut(usize) -> i64>(n: usize, reps: usize, mut f: F) -> (f64, i64) {
    let mut j = 0usize;
    let mut cs: i64 = 0;
    for _ in 0..1000 {
        cs = cs.wrapping_add(f(j));
        j += 1;
        if j == n {
            j = 0;
        }
    }
    black_box(cs);
    cs = 0;
    j = 0;
    let t0 = Instant::now();
    for _ in 0..reps {
        let r = black_box(f(black_box(j)));
        cs = cs.wrapping_add(r.wrapping_add(1));
        j += 1;
        if j == n {
            j = 0;
        }
    }
    let ns = t0.elapsed().as_nanos() as f64 / reps.max(1) as f64;
    (ns, cs)
}

// Inline FxHasher (rustc-hash 1.x algorithm, 64-bit): hash = (hash.rotate_left(5) ^ word) * SEED per word;
// write() consumes 8-byte chunks, then a 4-, 2- and 1-byte tail (native-endian), like rustc-hash 1.x.
const FX_SEED: u64 = 0x517cc1b727220a95;

#[derive(Default, Clone, Copy)]
struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline(always)]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            self.add(u64::from_ne_bytes(bytes[..8].try_into().unwrap()));
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            self.add(u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as u64);
            bytes = &bytes[4..];
        }
        if bytes.len() >= 2 {
            self.add(u16::from_ne_bytes(bytes[..2].try_into().unwrap()) as u64);
            bytes = &bytes[2..];
        }
        if let Some(&b) = bytes.first() {
            self.add(b as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxBuild = BuildHasherDefault<FxHasher>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: hand_keys <set> <N> <reps> [row|all] [sip|fx]");
        std::process::exit(2);
    }
    let hasher = args.get(4).map(|s| s.as_str()).unwrap_or("sip");
    match hasher {
        "sip" => run::<RandomState>(&args, "sip"),
        "fx" => run::<FxBuild>(&args, "fx"),
        other => {
            eprintln!("unknown hasher {} (sip|fx)", other);
            std::process::exit(2);
        }
    }
}

fn run<S: BuildHasher + Default>(args: &[String], hname: &str) {
    let set = args[0].as_str();
    let n: usize = args[1].parse().unwrap();
    let reps: usize = args[2].parse().unwrap();
    let row = args.get(3).map(|s| s.as_str()).unwrap_or("all");
    if n == 0 {
        eprintln!("N must be > 0");
        std::process::exit(2);
    }
    let want = |r: &str| row == "all" || row == r;
    let keys: Vec<Vec<u16>> = (0..n).map(|i| key_units(set, i)).collect();
    let miss: Vec<Vec<u16>> = (n..2 * n).map(|i| key_units(set, i)).collect();
    let mut map: HashMap<Box<[u16]>, u32, S> = HashMap::default();
    for (i, k) in keys.iter().enumerate() {
        map.insert(k.clone().into_boxed_slice(), i as u32);
    }
    let units = keys.iter().map(|k| k.len()).sum::<usize>() as f64 / n as f64;
    let bpk = units * 2.0;
    let wpk = bpk / 8.0;
    let emit = |r: &str, x: (f64, i64)| {
        println!("{} hand {} {} {} {:.3} {} {:.2} {:.2}", set, hname, r, n, x.0, x.1, wpk, bpk);
    };
    let look = |k: &[u16]| -> i64 {
        match map.get(k) {
            Some(&v) => v as i64,
            None => -1,
        }
    };
    if want("ret") {
        let x = bench(n, reps, |j| look(black_box(&keys[j][..])));
        emit("ret", x);
    }
    if want("miss") {
        let x = bench(n, reps, |j| look(black_box(&miss[j][..])));
        emit("miss", x);
    }
    if want("raw") {
        let x = bench(n, reps, |j| look(black_box(&keys[j][..])));
        emit("raw", x);
    }
    if want("cmp") {
        let x = bench(n, reps, |j| {
            let k = if j + 1 == n { 0 } else { j + 1 };
            black_box(&keys[j][..]).cmp(black_box(&keys[k][..])) as i64
        });
        emit("cmp", x);
    }
}

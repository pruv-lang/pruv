// P01 hand Rust lanes over UTF-16 keys. Four hand rows, selected by the last argument:
//   sip    HashMap<Box<[u16]>, u32> with the std hasher (SipHash 1-3), natural safe Rust (default)
//   fx     the same map with an inline FxHasher (rustc-hash 1.x algorithm, no crates)
//   fxmix  the same map with the same Fx-style multiply-rotate per word, plus a murmur3 fmix64 avalanche in finish()
//   oa     "same-structure optimized hand": open addressing over packed P records, one flat Vec<u64> arena, a
//          power-of-two Vec<u32> slot table (id+1, 0 = empty, linear probing, no removal), an entry table that stores
//          the hash, and the SAME hash as the Pruv P/w row (keys.pruv hash_w: one mix per data word, then
//          fin(h ^ header)), so the probe sequences match Pruv's. Typed safe Rust, bounds-checked indexing, no unsafe.
//
//   hand_keys <set> <N> <reps> [row|all] [sip|fx|fxmix|oa]
//       same key sets, N, miss keys, warm-up, black_box and checksum rule as driver_keys perf.
//       rows: ret  lookup of key j (hits; value = insertion index, so the checksum equals the Pruv ret row)
//             miss lookup of an absent key
//             raw  sip/fx/fxmix: identical to ret (the raw input of those lanes is already &[u16], nothing to encode).
//                  oa: the lane receives i64 words like Pruv, so raw = host encode to the P record + the canonicality
//                  check (length, zero padding; same as k_check_p) + lookup. A failed check returns -9 as in the driver.
//             cmp  sip/fx/fxmix: slice Ord on &[u16] (UTF-16 code-unit lexical order) of key j vs key j+1 (cyclic);
//                  oa: unsigned word-by-word compare of the P records, then the length
//       For oa the ret/miss rows take the pre-encoded P record (&[i64]) exactly like the Pruv ret row; find does not
//       re-validate there (Pruv's k_find_p_hw does not either), it is total: any word sequence gives an id or -1.
//       There is no enc row here. Checksums are the same for all four hands and equal the Pruv rows.
//       output: `set hand <hasher> row N ns_per_op checksum words_per_key bytes_per_key`
//       (sip/fx/fxmix: 2 bytes per unit, no header; oa: the record words incl. the header word, 8 bytes per word)
//   hand_keys stats <set> <N>
//       bucket-index statistics of the hashes over the N keys (see run_stats).
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
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
// MIX = false: finish() returns the raw state (`fx`). MIX = true: finish() applies the murmur3 fmix64 avalanche
// (`fxmix`); the per-word step and the write() chunking are identical, so only finish() differs.
const FX_SEED: u64 = 0x517cc1b727220a95;

#[inline(always)]
fn fmix64(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^ (h >> 33)
}

#[derive(Default, Clone, Copy)]
struct FxHasherG<const MIX: bool> {
    hash: u64,
}

impl<const MIX: bool> FxHasherG<MIX> {
    #[inline(always)]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
}

impl<const MIX: bool> Hasher for FxHasherG<MIX> {
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
        if MIX {
            fmix64(self.hash)
        } else {
            self.hash
        }
    }
}

type FxBuild = BuildHasherDefault<FxHasherG<false>>;
type FxMixBuild = BuildHasherDefault<FxHasherG<true>>;

// ---- Pruv hashes, mirrored from keys.pruv (mix / fin / pack4 / seed / hash_w / hu_lanes) ----

const GOLD: u64 = 0x9E3779B97F4A7C15; // -7046029254386353131 as u64
const FIN_C: u64 = 0xFF51AFD7ED558CCD; // -49064778989728563 as u64

#[inline(always)]
fn pmix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(GOLD)
}

#[inline(always)]
fn pfin(h: u64) -> u64 {
    let a = h ^ (h >> 33);
    let b = a.wrapping_mul(FIN_C);
    b ^ (b >> 33)
}

#[inline(always)]
fn pseed() -> u64 {
    pmix(1469598103934665603, 0)
}

// hash_w(k, 1): one mix per data word k[1..], then fin(h ^ k[0]). An empty record hashes like Pruv's `n <= hw` case
// (rd(k, 0) = 0).
#[inline]
fn hash_w(k: &[i64]) -> u64 {
    let Some((&n0, data)) = k.split_first() else {
        return pfin(pseed());
    };
    let mut h = pseed();
    for &w in data {
        h = pmix(h, w as u64);
    }
    pfin(h ^ n0 as u64)
}

// hu_rec_p: bud_map::hash_words over the units (4 units per mix, lanes repacked little-endian), only used by `stats`.
fn hash_u(units: &[u16]) -> u64 {
    let mut h = pseed();
    for ch in units.chunks(4) {
        let mut w: u64 = 0;
        for (i, &u) in ch.iter().enumerate() {
            w |= (u as u64) << (16 * i);
        }
        h = pmix(h, w);
    }
    pfin(h ^ units.len() as u64)
}

// ---- P records (host encoder and canonicality check, mirrors driver_keys enc_p and keys.pruv k_check_p) ----

fn enc_p_into(s: &[u16], out: &mut Vec<i64>) {
    out.clear();
    out.push(s.len() as i64);
    for ch in s.chunks(4) {
        let mut w: u64 = 0;
        for &u in ch {
            w = (w << 16) | u as u64;
        }
        w <<= 16 * (4 - ch.len()) as u32;
        out.push(w as i64);
    }
}

fn enc_p(s: &[u16]) -> Vec<i64> {
    let mut v = Vec::with_capacity(2 + s.len() / 4);
    enc_p_into(s, &mut v);
    v
}

// 0 canonical -> true. Same tests as k_check_p: length word in range, exactly ceil(n/4) data words, the lanes past n
// in the last word are zero (every 16-bit lane is a valid unit, so there is no per-unit range test for packed words).
fn check_p(k: &[i64]) -> bool {
    if k.is_empty() {
        return false;
    }
    let n = k[0];
    if !(0..=1073741824).contains(&n) {
        return false;
    }
    let m = ((n + 3) >> 2) as usize;
    if k.len() != 1 + m {
        return false;
    }
    let r = (n & 3) as u32;
    if r != 0 && m >= 1 && (k[m] as u64) & (u64::MAX >> (16 * r)) != 0 {
        return false;
    }
    true
}

// k_cmp_p: unsigned compare of the data words (big-endian lanes, zero padding), then the unit lengths.
fn cmp_p(a: &[i64], b: &[i64]) -> i64 {
    let (na, nb) = (a[0], b[0]);
    let m = (((na + 3) >> 2).min((nb + 3) >> 2)) as usize;
    for i in 1..=m {
        let (x, y) = (a[i] as u64, b[i] as u64);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    na.cmp(&nb) as i64
}

// ---- oa: open addressing over packed records ----

struct Ent {
    hash: u64,
    off: u32,
    len: u32,
}

struct OaMap {
    slots: Vec<u32>, // 0 empty, id + 1
    mask: usize,
    ents: Vec<Ent>,
    arena: Vec<u64>, // the inserted records copied whole (header word included), like the Pruv arena
}

impl OaMap {
    // slots = next power of two >= 2 * n_keys (min 2), the Pruv table sizing
    fn new(n_keys: usize, arena_words: usize) -> OaMap {
        let slots = (2 * n_keys.max(1)).next_power_of_two().max(2);
        OaMap {
            slots: vec![0u32; slots],
            mask: slots - 1,
            ents: Vec::with_capacity(slots / 2),
            arena: Vec::with_capacity(arena_words),
        }
    }

    // Id of the record equal to k (hash h), or -1. Bounded by the slot count; linear probing from h & mask.
    #[inline]
    fn find_h(&self, k: &[i64], h: u64) -> i64 {
        let mut p = (h as usize) & self.mask;
        for _ in 0..self.slots.len() {
            let s = self.slots[p];
            if s == 0 {
                return -1;
            }
            let e = &self.ents[(s - 1) as usize];
            if e.hash == h && e.len as usize == k.len() {
                let off = e.off as usize;
                let rec = &self.arena[off..off + e.len as usize];
                if rec.iter().zip(k.iter()).all(|(a, b)| *a == *b as u64) {
                    return (s - 1) as i64;
                }
            }
            p = (p + 1) & self.mask;
        }
        -1
    }

    #[inline]
    fn find(&self, k: &[i64]) -> i64 {
        self.find_h(k, hash_w(k))
    }

    // Existing id or a new entry; -1 when the table is full (capacity slots / 2, as Pruv).
    fn insert(&mut self, k: &[i64]) -> i64 {
        let h = hash_w(k);
        let f = self.find_h(k, h);
        if f >= 0 {
            return f;
        }
        if self.ents.len() >= self.slots.len() / 2 {
            return -1;
        }
        let mut p = (h as usize) & self.mask;
        while self.slots[p] != 0 {
            p = (p + 1) & self.mask;
        }
        let id = self.ents.len();
        self.ents.push(Ent { hash: h, off: self.arena.len() as u32, len: k.len() as u32 });
        self.arena.extend(k.iter().map(|&w| w as u64));
        self.slots[p] = id as u32 + 1;
        id as i64
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("stats") {
        run_stats(&args[1..]);
        return;
    }
    if args.len() < 3 {
        eprintln!("usage: hand_keys <set> <N> <reps> [row|all] [sip|fx|fxmix|oa] | hand_keys stats <set> <N>");
        std::process::exit(2);
    }
    let hasher = args.get(4).map(|s| s.as_str()).unwrap_or("sip");
    match hasher {
        "sip" => run::<RandomState>(&args, "sip"),
        "fx" => run::<FxBuild>(&args, "fx"),
        "fxmix" => run::<FxMixBuild>(&args, "fxmix"),
        "oa" => run_oa(&args),
        other => {
            eprintln!("unknown hasher {} (sip|fx|fxmix|oa)", other);
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

fn run_oa(args: &[String]) {
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
    let recs: Vec<Vec<i64>> = keys.iter().map(|k| enc_p(k)).collect();
    let mrecs: Vec<Vec<i64>> = (n..2 * n).map(|i| enc_p(&key_units(set, i))).collect();
    for r in recs.iter().chain(mrecs.iter()) {
        if !check_p(r) {
            eprintln!("oa: host encoding is not canonical: {:?}", r);
            std::process::exit(2);
        }
    }
    let mut map = OaMap::new(n, recs.iter().map(|r| r.len()).sum());
    for (i, r) in recs.iter().enumerate() {
        let id = map.insert(r);
        if id != i as i64 {
            eprintln!("oa insert key {}: got {}", i, id);
            std::process::exit(2);
        }
    }
    let wpk = recs.iter().map(|r| r.len()).sum::<usize>() as f64 / n as f64;
    let emit = |r: &str, x: (f64, i64)| {
        println!("{} hand oa {} {} {:.3} {} {:.2} {:.2}", set, r, n, x.0, x.1, wpk, wpk * 8.0);
    };
    if want("ret") {
        let x = bench(n, reps, |j| map.find(black_box(&recs[j][..])));
        emit("ret", x);
    }
    if want("miss") {
        let x = bench(n, reps, |j| map.find(black_box(&mrecs[j][..])));
        emit("miss", x);
    }
    if want("raw") {
        let mut buf: Vec<i64> = Vec::with_capacity(64);
        let x = bench(n, reps, |j| {
            enc_p_into(black_box(&keys[j][..]), &mut buf);
            if !check_p(&buf[..]) {
                return -9;
            }
            map.find(&buf[..])
        });
        emit("raw", x);
    }
    if want("cmp") {
        let x = bench(n, reps, |j| {
            let k = if j + 1 == n { 0 } else { j + 1 };
            cmp_p(black_box(&recs[j][..]), black_box(&recs[k][..]))
        });
        emit("cmp", x);
    }
}

// ---- stats: why a hash collides ----
//
//   stats <set> <N>
//   output per (hash, table): `stats set N hash table cap distinct max_probe mean_probe`
//     hash:  fx (Fx 1.x via Hash for [u16], i.e. length prefix + one write of the bytes), fxmix, pruvw (hash_w of the
//            P record, the P/w row), pruvu (the unit hash, the P/u row), ideal (expected distinct for a uniform hash)
//     table: hb = hashbrown bucket count for N items (next_pow2(N * 8 / 7)), pruv = Pruv slots (next_pow2(2N))
//     distinct: number of distinct (hash & (cap - 1)) over the N keys
//     max_probe / mean_probe: linear probing from the bucket index, keys inserted in order 0..N-1, probe length =
//            displacement + 1 of each stored key (what a hit lookup walks). This is a model of the Pruv/oa table;
//            hashbrown itself probes 8/16-slot groups, so for `hb` it is a lower-level indicator, not its exact cost.
fn run_stats(args: &[String]) {
    if args.len() < 2 {
        eprintln!("usage: hand_keys stats <set> <N>");
        std::process::exit(2);
    }
    let set = args[0].as_str();
    let n: usize = args[1].parse().unwrap();
    let keys: Vec<Vec<u16>> = (0..n).map(|i| key_units(set, i)).collect();
    let hb = if n < 4 {
        4
    } else if n < 8 {
        8
    } else {
        (n * 8 / 7).next_power_of_two()
    };
    let pruv = (2 * n.max(1)).next_power_of_two().max(2);
    let hashes: Vec<(&str, Vec<u64>)> = vec![
        ("fx", keys.iter().map(|k| FxBuild::default().hash_one(&k[..])).collect()),
        ("fxmix", keys.iter().map(|k| FxMixBuild::default().hash_one(&k[..])).collect()),
        ("pruvw", keys.iter().map(|k| hash_w(&enc_p(k))).collect()),
        ("pruvu", keys.iter().map(|k| hash_u(k)).collect()),
    ];
    for (tname, cap) in [("hb", hb), ("pruv", pruv)] {
        for (hname, hs) in &hashes {
            let mask = cap - 1;
            let mut distinct: HashSet<usize> = HashSet::new();
            let mut occ = vec![false; cap];
            let (mut mx, mut sum) = (0usize, 0usize);
            for &h in hs {
                let home = (h as usize) & mask;
                distinct.insert(home);
                let mut p = home;
                let mut len = 1usize;
                while occ[p] {
                    p = (p + 1) & mask;
                    len += 1;
                }
                occ[p] = true;
                mx = mx.max(len);
                sum += len;
            }
            println!("stats {} {} {} {} {} {} {} {:.2}", set, n, hname, tname, cap, distinct.len(), mx,
                     sum as f64 / n as f64);
        }
        let c = cap as f64;
        let ideal = c * (1.0 - (1.0 - 1.0 / c).powi(n as i32));
        println!("stats {} {} ideal {} {} {:.1} - -", set, n, tname, cap, ideal);
    }
}

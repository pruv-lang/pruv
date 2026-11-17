// P01 key layout driver (appended to a main file that already declares `#[path = "gen.rs"] mod sol;`, see
// run_keys.py). Record formats and the table layout: header comment of keys.pruv.
//
//   driver_keys perf <set> <N> <reps> [row|all] [L|all] [H|all]
//       sets: sid ("session-%05d", 13 ASCII), uuid ("99b568f7-0000-4000-8000-%012d", 36 ASCII),
//             nonascii ("세션-%05d-ключ", mixed BMP), surr ("surr-%05d-" + D800 'x' DC00 + astral pair D83D DE00)
//       N keys i = 0..N-1 are inserted; the miss keys are i = N..2N-1 (same shape, absent).
//       L: U P A8 F1; H: u (unit hash) w (word hash).
//       rows (per L and H): ret  find of a retained (pre-encoded) record, cycling over all N keys (hits)
//                           miss find of a retained record of an absent key
//                           raw  host encode from &[u16] + k_check + find (hits); k_check and find read the
//                                whole buffer, so the encode result is fully consumed
//            (per L, H "-"): enc  host encode + a full-buffer fold (s = s*31 + w over EVERY word of the produced
//                                buffer, same fold for every layout; the fold cost is part of the row)
//                           chk  k_check only (pre-encoded records)
//                           cmp  k_cmp of neighbours (key j vs key j+1, cyclic)
//       reps = timed operations per row (keys are cycled), after a 1000-operation warm-up. Inputs go through
//       black_box, every result is folded into the checksum (sum of result + 1).
//       output: `set L H row N ns_per_op checksum words_per_key bytes_per_key` (record words incl. header).
//   driver_keys check
//       stdin: JSON lines, either {"id": .., "a": [units], "b": [units]} or {"id": .., "neg": "U|P|A8|F1",
//       "rec": [words]}. stdout: one JSON line per input line (see check_case / the neg branch below).
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

// ---- host encoders (HOST ENCODE cost: `out` is cleared and reused, no allocation once it has grown) ----

fn enc_u(s: &[u16], out: &mut Vec<i64>) {
    out.clear();
    out.push(s.len() as i64);
    out.extend(s.iter().map(|&u| u as i64));
}

// 4 units per word, big-endian 16-bit lanes, zero tail padding.
#[inline(always)]
fn put_lanes16(s: &[u16], out: &mut Vec<i64>) {
    for ch in s.chunks(4) {
        let mut w: u64 = 0;
        for &u in ch {
            w = (w << 16) | u as u64;
        }
        w <<= 16 * (4 - ch.len()) as u32;
        out.push(w as i64);
    }
}

// 8 byte units per word, big-endian bytes, zero tail padding (the caller has checked every unit < 256).
#[inline(always)]
fn put_bytes8(s: &[u16], out: &mut Vec<i64>) {
    for ch in s.chunks(8) {
        let mut w: u64 = 0;
        for &u in ch {
            w = (w << 8) | (u as u64 & 0xFF);
        }
        w <<= 8 * (8 - ch.len()) as u32;
        out.push(w as i64);
    }
}

#[inline(always)]
fn is_ascii16(s: &[u16]) -> bool {
    s.iter().all(|&u| u < 128)
}

fn enc_p(s: &[u16], out: &mut Vec<i64>) {
    out.clear();
    out.push(s.len() as i64);
    put_lanes16(s, out);
}

fn enc_a8(s: &[u16], out: &mut Vec<i64>) {
    out.clear();
    let n = s.len() as i64;
    if is_ascii16(s) {
        out.push(2 * n);
        put_bytes8(s, out);
    } else {
        out.push(2 * n + 1);
        put_lanes16(s, out);
    }
}

// Byte stream packer: 8 bytes per word, big-endian, zero padding at finish.
struct BytePack {
    w: u64,
    k: u32,
}

impl BytePack {
    #[inline(always)]
    fn push(&mut self, out: &mut Vec<i64>, b: u64) {
        self.w = (self.w << 8) | b;
        self.k += 1;
        if self.k == 8 {
            out.push(self.w as i64);
            self.w = 0;
            self.k = 0;
        }
    }
    #[inline(always)]
    fn finish(&self, out: &mut Vec<i64>) {
        if self.k > 0 {
            out.push((self.w << (8 * (8 - self.k))) as i64);
        }
    }
}

fn enc_f1(s: &[u16], out: &mut Vec<i64>) {
    out.clear();
    let n = s.len();
    if is_ascii16(s) {
        out.push((2 * n) as i64);
        out.push(n as i64);
        put_bytes8(s, out);
    } else {
        let bl = 1 + 2 * n;
        out.push((2 * bl + 1) as i64);
        out.push(n as i64);
        let mut p = BytePack { w: 0, k: 0 };
        p.push(out, 0xFF);
        for &u in s {
            p.push(out, (u & 0xFF) as u64);
            p.push(out, (u >> 8) as u64);
        }
        p.finish(out);
    }
}

// ---- key sets ----

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

// ---- table ----

// Empty table for n_keys keys (slots = next power of two >= 2 * n_keys, capacity slots / 2) with `arena` words.
fn new_table(n_keys: usize, arena: usize) -> Vec<i64> {
    let slots = (2 * n_keys.max(1)).next_power_of_two().max(2);
    let len = 8 + slots + 3 * (slots / 2) + arena;
    let mut t = vec![0i64; len];
    let r = sol::k_init(&mut t[..], slots as i64);
    if r != 0 {
        eprintln!("k_init failed: {}", r);
        std::process::exit(2);
    }
    t
}

// ---- perf ----

struct Ctx {
    set: String,
    n: usize,
    reps: usize,
    row: String,
    hsel: String,
    keys: Vec<Vec<u16>>,
    miss: Vec<Vec<u16>>,
}

impl Ctx {
    fn want(&self, row: &str) -> bool {
        self.row == "all" || self.row == row
    }
}

// Runs f(j) for j cycling over 0..n: 1000 warm-up calls, then `reps` timed calls. Returns (ns per op, checksum).
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

fn emit(c: &Ctx, l: &str, h: &str, row: &str, r: (f64, i64), wpk: f64) {
    println!("{} {} {} {} {} {:.3} {} {:.2} {:.2}", c.set, l, h, row, c.n, r.0, r.1, wpk, wpk * 8.0);
}

// Full-buffer fold for the enc row: every produced word goes into the result (s = s * 31 + w), so no layout can skip
// work; the same fold for every layout.
#[inline(always)]
fn fold_words(buf: &[i64]) -> i64 {
    let mut s: i64 = buf.len() as i64;
    for &w in buf {
        s = s.wrapping_mul(31).wrapping_add(w);
    }
    s
}

fn encode_all<E: Fn(&[u16], &mut Vec<i64>)>(keys: &[Vec<u16>], enc: &E) -> Vec<Vec<i64>> {
    keys.iter()
        .map(|k| {
            let mut v = Vec::new();
            enc(&k[..], &mut v);
            v
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn perf_hash<E, C, I, F>(c: &Ctx, l: &str, h: &str, enc: &E, chk: &C, ins: I, find: F, recs: &[Vec<i64>],
                         mrecs: &[Vec<i64>], wpk: f64)
where
    E: Fn(&[u16], &mut Vec<i64>),
    C: Fn(&[i64]) -> i64,
    I: Fn(&mut [i64], &[i64]) -> i64,
    F: Fn(&[i64], &[i64]) -> i64,
{
    if c.hsel != "all" && c.hsel != h {
        return;
    }
    if !(c.want("ret") || c.want("miss") || c.want("raw")) {
        return;
    }
    let arena: usize = recs.iter().map(|r| r.len()).sum();
    let mut t = new_table(c.n, arena);
    for (i, r) in recs.iter().enumerate() {
        let id = ins(&mut t[..], &r[..]);
        if id != i as i64 {
            eprintln!("insert {} {} key {}: got {}", l, h, i, id);
            std::process::exit(2);
        }
    }
    let tb: &[i64] = &t[..];
    let n = c.n;
    if c.want("ret") {
        let r = bench(n, c.reps, |j| find(black_box(tb), black_box(&recs[j][..])));
        emit(c, l, h, "ret", r, wpk);
    }
    if c.want("miss") {
        let r = bench(n, c.reps, |j| find(black_box(tb), black_box(&mrecs[j][..])));
        emit(c, l, h, "miss", r, wpk);
    }
    if c.want("raw") {
        let keys = &c.keys;
        let mut buf: Vec<i64> = Vec::with_capacity(64);
        let r = bench(n, c.reps, |j| {
            enc(black_box(&keys[j][..]), &mut buf);
            if chk(&buf[..]) != 0 {
                return -9;
            }
            find(black_box(tb), &buf[..])
        });
        emit(c, l, h, "raw", r, wpk);
    }
}

#[allow(clippy::too_many_arguments)]
fn perf_layout<E, C, M, IU, FU, IW, FW>(c: &Ctx, l: &str, enc: E, chk: C, cmp: M, iu: IU, fu: FU, iw: IW, fw: FW)
where
    E: Fn(&[u16], &mut Vec<i64>),
    C: Fn(&[i64]) -> i64,
    M: Fn(&[i64], &[i64]) -> i64,
    IU: Fn(&mut [i64], &[i64]) -> i64,
    FU: Fn(&[i64], &[i64]) -> i64,
    IW: Fn(&mut [i64], &[i64]) -> i64,
    FW: Fn(&[i64], &[i64]) -> i64,
{
    let recs = encode_all(&c.keys, &enc);
    let mrecs = encode_all(&c.miss, &enc);
    for r in recs.iter().chain(mrecs.iter()) {
        if chk(&r[..]) != 0 {
            eprintln!("{}: host encoding is not canonical: {:?}", l, r);
            std::process::exit(2);
        }
    }
    let wpk = recs.iter().map(|r| r.len()).sum::<usize>() as f64 / c.n as f64;
    perf_hash(c, l, "u", &enc, &chk, iu, fu, &recs, &mrecs, wpk);
    perf_hash(c, l, "w", &enc, &chk, iw, fw, &recs, &mrecs, wpk);
    if !(c.hsel == "all" || c.hsel == "-") {
        return;
    }
    let n = c.n;
    if c.want("enc") {
        let keys = &c.keys;
        let mut buf: Vec<i64> = Vec::with_capacity(64);
        let r = bench(n, c.reps, |j| {
            enc(black_box(&keys[j][..]), &mut buf);
            fold_words(&buf[..])
        });
        emit(c, l, "-", "enc", r, wpk);
    }
    if c.want("chk") {
        let r = bench(n, c.reps, |j| chk(black_box(&recs[j][..])));
        emit(c, l, "-", "chk", r, wpk);
    }
    if c.want("cmp") {
        let r = bench(n, c.reps, |j| {
            let k = if j + 1 == n { 0 } else { j + 1 };
            cmp(black_box(&recs[j][..]), black_box(&recs[k][..]))
        });
        emit(c, l, "-", "cmp", r, wpk);
    }
}

fn run_perf(args: &[String]) {
    if args.len() < 3 {
        eprintln!("usage: perf <set> <N> <reps> [row|all] [L|all] [H|all]");
        std::process::exit(2);
    }
    let set = args[0].clone();
    let n: usize = args[1].parse().unwrap();
    let reps: usize = args[2].parse().unwrap();
    let row = args.get(3).cloned().unwrap_or_else(|| "all".to_string());
    let lsel = args.get(4).cloned().unwrap_or_else(|| "all".to_string());
    let hsel = args.get(5).cloned().unwrap_or_else(|| "all".to_string());
    if n == 0 {
        eprintln!("N must be > 0");
        std::process::exit(2);
    }
    let keys: Vec<Vec<u16>> = (0..n).map(|i| key_units(&set, i)).collect();
    let miss: Vec<Vec<u16>> = (n..2 * n).map(|i| key_units(&set, i)).collect();
    let c = Ctx { set, n, reps, row, hsel, keys, miss };
    let on = |l: &str| lsel == "all" || lsel == l;
    if on("U") {
        perf_layout(&c, "U", enc_u, sol::k_check_u, sol::k_cmp_u, sol::k_insert_u_hu, sol::k_find_u_hu,
                    sol::k_insert_u_hw, sol::k_find_u_hw);
    }
    if on("P") {
        perf_layout(&c, "P", enc_p, sol::k_check_p, sol::k_cmp_p, sol::k_insert_p_hu, sol::k_find_p_hu,
                    sol::k_insert_p_hw, sol::k_find_p_hw);
    }
    if on("A8") {
        perf_layout(&c, "A8", enc_a8, sol::k_check_a8, sol::k_cmp_a8, sol::k_insert_a8_hu, sol::k_find_a8_hu,
                    sol::k_insert_a8_hw, sol::k_find_a8_hw);
    }
    if on("F1") {
        perf_layout(&c, "F1", enc_f1, sol::k_check_f1, sol::k_cmp_f1, sol::k_insert_f1_hu, sol::k_find_f1_hu,
                    sol::k_insert_f1_hw, sol::k_find_f1_hw);
    }
}

// ---- check ----

// Minimal JSON reader for the lines run_keys.py writes (objects, arrays, integers, plain strings, literals).
enum J {
    Num(i64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
    Lit,
}

struct Pj<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Pj<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn val(&mut self) -> J {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = match self.val() {
                        J::Str(s) => s,
                        _ => panic!("object key is not a string"),
                    };
                    self.ws();
                    assert_eq!(self.s[self.i], b':', "expected ':'");
                    self.i += 1;
                    let x = self.val();
                    v.push((k, x));
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Obj(v)
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.val());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Arr(v)
            }
            b'"' => {
                self.i += 1;
                let st = self.i;
                while self.s[self.i] != b'"' {
                    if self.s[self.i] == b'\\' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                self.i += 1;
                J::Str(t)
            }
            b't' | b'f' | b'n' => {
                while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
                    self.i += 1;
                }
                J::Lit
            }
            _ => {
                let st = self.i;
                if self.s[self.i] == b'-' {
                    self.i += 1;
                }
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                J::Num(std::str::from_utf8(&self.s[st..self.i]).unwrap().parse().unwrap())
            }
        }
    }
}

fn get<'a>(o: &'a J, k: &str) -> Option<&'a J> {
    match o {
        J::Obj(v) => v.iter().find(|(n, _)| n == k).map(|(_, x)| x),
        _ => None,
    }
}

fn ints(j: Option<&J>) -> Vec<i64> {
    match j {
        Some(J::Arr(v)) => v.iter().map(|x| if let J::Num(n) = x { *n } else { panic!("not a number") }).collect(),
        _ => panic!("expected an array"),
    }
}

fn text(j: Option<&J>) -> String {
    match j {
        Some(J::Str(s)) => s.clone(),
        Some(J::Num(n)) => n.to_string(),
        _ => panic!("expected a string"),
    }
}

fn jl(v: &[i64]) -> String {
    let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    format!("[{}]", parts.join(","))
}

// One table with hash kind (ins, find): insert a, look up a and b, insert b, look up both again.
fn one_hash<I, F>(ra: &[i64], rb: &[i64], ins: I, find: F) -> String
where
    I: Fn(&mut [i64], &[i64]) -> i64,
    F: Fn(&[i64], &[i64]) -> i64,
{
    let mut t = new_table(2, ra.len() + rb.len());
    let ins_a = ins(&mut t[..], ra);
    let find_a = find(&t[..], ra);
    let find_b = find(&t[..], rb);
    let ins_b = ins(&mut t[..], rb);
    let find_b2 = find(&t[..], rb);
    let find_a2 = find(&t[..], ra);
    format!(
        "{{\"ins_a\":{},\"find_a\":{},\"find_b\":{},\"ins_b\":{},\"find_b2\":{},\"find_a2\":{}}}",
        ins_a, find_a, find_b, ins_b, find_b2, find_a2
    )
}

#[allow(clippy::too_many_arguments)]
fn check_layout<E, C, M, HU, IU, FU, IW, FW>(a: &[u16], b: &[u16], enc: E, chk: C, cmp: M, hu: HU, iu: IU, fu: FU,
                                             iw: IW, fw: FW) -> String
where
    E: Fn(&[u16], &mut Vec<i64>),
    C: Fn(&[i64]) -> i64,
    M: Fn(&[i64], &[i64]) -> i64,
    HU: Fn(&[i64]) -> i64,
    IU: Fn(&mut [i64], &[i64]) -> i64,
    FU: Fn(&[i64], &[i64]) -> i64,
    IW: Fn(&mut [i64], &[i64]) -> i64,
    FW: Fn(&[i64], &[i64]) -> i64,
{
    let mut ra = Vec::new();
    enc(a, &mut ra);
    let mut rb = Vec::new();
    enc(b, &mut rb);
    format!(
        "{{\"rec_a\":{},\"rec_b\":{},\"chk_a\":{},\"chk_b\":{},\"hu_a\":{},\"hu_b\":{},\"cmp_ab\":{},\"cmp_ba\":{},\"cmp_aa\":{},\"u\":{},\"w\":{}}}",
        jl(&ra),
        jl(&rb),
        chk(&ra[..]),
        chk(&rb[..]),
        hu(&ra[..]),
        hu(&rb[..]),
        cmp(&ra[..], &rb[..]),
        cmp(&rb[..], &ra[..]),
        cmp(&ra[..], &ra[..]),
        one_hash(&ra[..], &rb[..], iu, fu),
        one_hash(&ra[..], &rb[..], iw, fw)
    )
}

fn check_case(a: &[u16], b: &[u16]) -> String {
    let u = check_layout(a, b, enc_u, sol::k_check_u, sol::k_cmp_u, sol::k_hu_u, sol::k_insert_u_hu,
                         sol::k_find_u_hu, sol::k_insert_u_hw, sol::k_find_u_hw);
    let p = check_layout(a, b, enc_p, sol::k_check_p, sol::k_cmp_p, sol::k_hu_p, sol::k_insert_p_hu,
                         sol::k_find_p_hu, sol::k_insert_p_hw, sol::k_find_p_hw);
    let a8 = check_layout(a, b, enc_a8, sol::k_check_a8, sol::k_cmp_a8, sol::k_hu_a8, sol::k_insert_a8_hu,
                          sol::k_find_a8_hu, sol::k_insert_a8_hw, sol::k_find_a8_hw);
    let f1 = check_layout(a, b, enc_f1, sol::k_check_f1, sol::k_cmp_f1, sol::k_hu_f1, sol::k_insert_f1_hu,
                          sol::k_find_f1_hu, sol::k_insert_f1_hw, sol::k_find_f1_hw);
    format!("\"U\":{},\"P\":{},\"A8\":{},\"F1\":{}", u, p, a8, f1)
}

fn units16(v: &[i64]) -> Vec<u16> {
    v.iter()
        .map(|&x| {
            if !(0..=65535).contains(&x) {
                eprintln!("unit out of range: {}", x);
                std::process::exit(2);
            }
            x as u16
        })
        .collect()
}

fn run_check() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let j = Pj { s: line.as_bytes(), i: 0 }.val();
        let id = text(get(&j, "id"));
        if let Some(l) = get(&j, "neg") {
            let l = text(Some(l));
            let rec = ints(get(&j, "rec"));
            let r = match l.as_str() {
                "U" => sol::k_check_u(&rec[..]),
                "P" => sol::k_check_p(&rec[..]),
                "A8" => sol::k_check_a8(&rec[..]),
                "F1" => sol::k_check_f1(&rec[..]),
                _ => -99,
            };
            writeln!(o, "{{\"id\":\"{}\",\"chk\":{}}}", id, r).unwrap();
            continue;
        }
        let a = units16(&ints(get(&j, "a")));
        let b = units16(&ints(get(&j, "b")));
        writeln!(o, "{{\"id\":\"{}\",{}}}", id, check_case(&a, &b)).unwrap();
    }
}

// ---- hdr: corrupt-header controls ----
// Symbolic header values, evaluated on the table BEFORE any corruption of that case is applied:
//   len, cap, slots, sb, eb, plain integers, and cap-1, cap+1, slots*2, 2^31 (as the integer), slot0 (index of the
//   first non-empty slot word, i.e. the slot of an inserted key).
fn hdr_sym(t: &[i64], s: &str) -> i64 {
    let (slots, cap, sb, eb) = (t[0], t[3], t[4], t[5]);
    match s {
        "len" => t.len() as i64,
        "cap" => cap,
        "slots" => slots,
        "sb" => sb,
        "eb" => eb,
        "cap-1" => cap - 1,
        "cap+1" => cap + 1,
        "slots*2" => slots * 2,
        "slot0" => {
            let mut p = sb as usize;
            while t[p] == 0 {
                p += 1;
            }
            p as i64
        }
        other => other.parse::<i64>().unwrap_or_else(|_| {
            eprintln!("bad hdr symbol {}", other);
            std::process::exit(2);
        }),
    }
}

fn hdr_layout<E, I, F>(enc: E, ins: I, find: F, keys: &[Vec<u16>], probes: &[Vec<u16>], sets: &[(String, String)])
    -> String
where
    E: Fn(&[u16], &mut Vec<i64>),
    I: Fn(&mut [i64], &[i64]) -> i64,
    F: Fn(&[i64], &[i64]) -> i64,
{
    let enc1 = |u: &Vec<u16>| {
        let mut v = Vec::new();
        enc(&u[..], &mut v);
        v
    };
    let kr: Vec<Vec<i64>> = keys.iter().map(&enc1).collect();
    let pr: Vec<Vec<i64>> = probes.iter().map(&enc1).collect();
    let arena: usize = kr.iter().chain(pr.iter()).map(|r| r.len()).sum();
    // room for every probe key too, so a healthy table accepts the new-key insert
    let mut t = new_table(kr.len() + pr.len(), arena);
    for r in &kr {
        ins(&mut t[..], &r[..]);
    }
    let resolved: Vec<(usize, i64)> =
        sets.iter().map(|(i, v)| (hdr_sym(&t[..], i) as usize, hdr_sym(&t[..], v))).collect();
    for (i, v) in resolved {
        t[i] = v;
    }
    let mut out: Vec<i64> = Vec::new();
    for r in kr.iter().chain(pr.iter()) {
        out.push(find(&t[..], &r[..]));
    }
    // insert: a stored key (hit) and a new key, each followed by finds of every key
    for r in [kr.first(), pr.first()].into_iter().flatten() {
        out.push(ins(&mut t[..], &r[..]));
        for q in kr.iter().chain(pr.iter()) {
            out.push(find(&t[..], &q[..]));
        }
    }
    out.extend_from_slice(&t[..8]);
    jl(&out)
}

// Input line: {"id", "L", "H", "keys": [[units]], "probes": [[units]], "set": [["idx", "value"], ..]}
// (idx / value are strings holding an integer or a hdr_sym symbol). Output: {"id", "res": [..]}: finds of every key
// and probe on the corrupted table, then insert(first key) + all finds, insert(first probe) + all finds, then t[0..8].
// Command (one JSON line per case): driver_keys hdr
fn run_hdr() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut o = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let j = Pj { s: line.as_bytes(), i: 0 }.val();
        let id = text(get(&j, "id"));
        let l = text(get(&j, "L"));
        let h = text(get(&j, "H"));
        let lists = |k: &str| -> Vec<Vec<u16>> {
            match get(&j, k) {
                Some(J::Arr(v)) => v.iter().map(|x| units16(&ints(Some(x)))).collect(),
                _ => panic!("expected an array of unit lists"),
            }
        };
        let keys = lists("keys");
        let probes = lists("probes");
        let sets: Vec<(String, String)> = match get(&j, "set") {
            Some(J::Arr(v)) => v
                .iter()
                .map(|p| match p {
                    J::Arr(q) if q.len() == 2 => (text(Some(&q[0])), text(Some(&q[1]))),
                    _ => panic!("set entries are [idx, value]"),
                })
                .collect(),
            _ => panic!("expected set array"),
        };
        let r = match (l.as_str(), h.as_str()) {
            ("U", "u") => hdr_layout(enc_u, sol::k_insert_u_hu, sol::k_find_u_hu, &keys, &probes, &sets),
            ("U", "w") => hdr_layout(enc_u, sol::k_insert_u_hw, sol::k_find_u_hw, &keys, &probes, &sets),
            ("P", "u") => hdr_layout(enc_p, sol::k_insert_p_hu, sol::k_find_p_hu, &keys, &probes, &sets),
            ("P", "w") => hdr_layout(enc_p, sol::k_insert_p_hw, sol::k_find_p_hw, &keys, &probes, &sets),
            ("A8", "u") => hdr_layout(enc_a8, sol::k_insert_a8_hu, sol::k_find_a8_hu, &keys, &probes, &sets),
            ("A8", "w") => hdr_layout(enc_a8, sol::k_insert_a8_hw, sol::k_find_a8_hw, &keys, &probes, &sets),
            ("F1", "u") => hdr_layout(enc_f1, sol::k_insert_f1_hu, sol::k_find_f1_hu, &keys, &probes, &sets),
            ("F1", "w") => hdr_layout(enc_f1, sol::k_insert_f1_hw, sol::k_find_f1_hw, &keys, &probes, &sets),
            _ => {
                eprintln!("bad layout/hash {} {}", l, h);
                std::process::exit(2);
            }
        };
        writeln!(o, "{{\"id\":\"{}\",\"res\":{}}}", id, r).unwrap();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("perf") => run_perf(&args[2..]),
        Some("check") => run_check(),
        Some("hdr") => run_hdr(),
        _ => {
            eprintln!("usage: driver_keys perf <set> <N> <reps> [row|all] [L|all] [H|all] | driver_keys check | driver_keys hdr");
            std::process::exit(2);
        }
    }
}

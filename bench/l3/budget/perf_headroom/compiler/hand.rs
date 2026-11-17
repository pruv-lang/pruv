// Hand-written Rust baselines for the perf_headroom compiler items (built with rustc -O --edition 2021).
// Same inputs, same n, same loop and same result consumption as m6_test/root.pruv + perf_extra.pruv:
// per call, len + parity of the first, middle and last unit; input alternates by +0/+1.
// Writings per item:
//   <op>      idiomatic std Rust: to_string / format! into a String (UTF-8; byte length, so p_ref's checksum differs)
//   <op>_reuse  reference only (excluded from the bar): reuses one buffer across calls
//   <op>_u    same output representation as Pruv (Vec<i64> of UTF-16 units, exact length, one allocation, plain
//             indexing with Rust's normal bounds checks); checksums equal Pruv's
// Input: `op n args..` per line. Output: `ns <elapsed ns of the whole loop> <checksum>`.
use std::fmt::Write as _;
use std::io::{self, BufRead};
use std::time::Instant;

fn eat_units(r: &[i64]) -> i64 {
    let l = r.len();
    let mut c = 0;
    if l > 0 {
        c = (r[0] + r[l / 2] + r[l - 1]) & 1;
    }
    l.min(999999) as i64 + c
}

fn eat_str(s: &str) -> i64 {
    let b = s.as_bytes();
    let l = b.len();
    let mut c = 0;
    if l > 0 {
        c = (b[0] as i64 + b[l / 2] as i64 + b[l - 1] as i64) & 1;
    }
    l.min(999999) as i64 + c
}

// ------------------------------------------------------------ integers
fn digits(x: i64) -> usize {
    let mut v = x.unsigned_abs();
    let mut n = 1;
    while v >= 10 {
        v /= 10;
        n += 1;
    }
    n + (x < 0) as usize
}

// writes String(x) into o[at..at + digits(x)]
fn put_int(o: &mut [i64], at: usize, x: i64, n: usize) {
    let mut v = x.unsigned_abs();
    if x < 0 {
        o[at] = 45;
    }
    let mut k = at + n;
    loop {
        k -= 1;
        o[k] = 48 + (v % 10) as i64;
        v /= 10;
        if v == 0 {
            break;
        }
    }
}

fn int_units(x: i64) -> Vec<i64> {
    let n = digits(x);
    let mut o = vec![0i64; n];
    put_int(&mut o, 0, x, n);
    o
}

// stack-buffer writing (the shape the Pruv small-result path emits)
fn int_units_stack(x: i64, buf: &mut [i64; 20]) -> usize {
    let n = digits(x);
    put_int(&mut buf[..], 0, x, n);
    n
}

// ------------------------------------------------------------ dates (days-from-civil inverse, as JS Date)
fn civil(ms: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y, m, d, rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

fn iso_string(ms: i64) -> String {
    let (y, m, d, h, mi, s, f) = civil(ms);
    if (0..=9999).contains(&y) {
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, h, mi, s, f)
    } else {
        format!("{}{:06}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", if y < 0 { '-' } else { '+' }, y.abs(), m, d, h, mi, s, f)
    }
}

fn two(o: &mut [i64], at: usize, v: i64) {
    o[at] = 48 + v / 10 % 10;
    o[at + 1] = 48 + v % 10;
}

fn iso_units(ms: i64) -> Vec<i64> {
    let (y, m, d, h, mi, s, f) = civil(ms);
    let (mut o, at) = if (0..=9999).contains(&y) {
        let mut o = vec![0i64; 24];
        two(&mut o, 0, y / 100);
        two(&mut o, 2, y);
        (o, 4)
    } else {
        let mut o = vec![0i64; 27];
        let ay = y.abs();
        o[0] = if y < 0 { 45 } else { 43 };
        two(&mut o, 1, ay / 10000);
        two(&mut o, 3, ay / 100);
        two(&mut o, 5, ay);
        (o, 7)
    };
    o[at] = 45;
    two(&mut o, at + 1, m);
    o[at + 3] = 45;
    two(&mut o, at + 4, d);
    o[at + 6] = 84;
    two(&mut o, at + 7, h);
    o[at + 9] = 58;
    two(&mut o, at + 10, mi);
    o[at + 12] = 58;
    two(&mut o, at + 13, s);
    o[at + 15] = 46;
    o[at + 16] = 48 + f / 100 % 10;
    two(&mut o, at + 17, f);
    o[at + 19] = 90;
    o
}

// ------------------------------------------------------------ refusal text (reason 2, lang 0, limited)
const PRE: [u16; 6] = [50696, 49328, 32, 54620, 46020, 40];
const CN: [u16; 5] = [48716, 46300, 183, 47749, 47161];
const POST: [u16; 78] = [
    41, 50640, 32, 45823, 50500, 32, 49324, 50857, 51088, 44032, 32, 45720, 47532, 44144, 45208, 32, 45149, 45236, 44592,
    47484, 32, 44256, 47484, 32, 46412, 44620, 51648, 32, 47688, 52632, 45796, 46, 32, 49352, 32, 50948, 51076, 183,
    47749, 47161, 51012, 32, 49884, 46020, 54616, 51648, 32, 50506, 44256, 32, 51648, 44552, 44620, 51648, 32, 44208,
    44284, 50752, 32, 54869, 51064, 54616, 51648, 32, 47803, 54620, 32, 44163, 47564, 32, 51687, 44172, 32, 48372, 44256,
    54620, 45796, 46,
];

fn refusal_string(pre: &str, cn: &str, post: &str, used: i64, limit: i64) -> String {
    format!("{pre}{cn} {used}/{limit}{post}")
}

fn refusal_units(used: i64, limit: i64) -> Vec<i64> {
    let lu = digits(used);
    let ll = digits(limit);
    let mut o = Vec::with_capacity(PRE.len() + CN.len() + 2 + lu + ll + POST.len());
    o.extend(PRE.iter().map(|&u| u as i64));
    o.extend(CN.iter().map(|&u| u as i64));
    o.push(32);
    let at = o.len();
    o.resize(at + lu, 0);
    put_int(&mut o, at, used, lu);
    o.push(47);
    let at = o.len();
    o.resize(at + ll, 0);
    put_int(&mut o, at, limit, ll);
    o.extend(POST.iter().map(|&u| u as i64));
    o
}

// ------------------------------------------------------------ amount "used/limit"
fn amount_units(used: i64, limit: i64) -> Vec<i64> {
    let lu = digits(used);
    let ll = digits(limit);
    let mut o = vec![0i64; lu + 1 + ll];
    put_int(&mut o, 0, used, lu);
    o[lu] = 47;
    put_int(&mut o, lu + 1, limit, ll);
    o
}

// ------------------------------------------------------------ ent_u16 copy loop (put_all)
fn put_range(o: &mut [i64], at: usize, src: &[i64], s: usize, m: usize) -> usize {
    for k in 0..m {
        o[at + k] = src[s + k];
    }
    at + m
}

fn run(op: &str, w: &[i64]) -> i64 {
    let n = w[0];
    let mut acc = 0i64;
    match op {
        "p_int" => {
            for i in 0..n {
                acc += eat_str(&(w[1] + (i & 1)).to_string());
            }
        }
        "p_int_u" => {
            let mut buf = [0i64; 20];
            for i in 0..n {
                let l = int_units_stack(w[1] + (i & 1), &mut buf);
                acc += eat_units(&buf[..l]);
            }
        }
        "p_int_v" => {
            for i in 0..n {
                acc += eat_units(&int_units(w[1] + (i & 1)));
            }
        }
        "p_iso" => {
            for i in 0..n {
                acc += eat_str(&iso_string(w[1] + (i & 1)));
            }
        }
        "p_iso_u" => {
            for i in 0..n {
                acc += eat_units(&iso_units(w[1] + (i & 1)));
            }
        }
        "p_ref" => {
            // args as m6 p_ref: reason 2, unlimited 0, used, limit, lang 0
            let pre = String::from_utf16(&PRE).unwrap();
            let cn = String::from_utf16(&CN).unwrap();
            let post = String::from_utf16(&POST).unwrap();
            for i in 0..n {
                acc += eat_str(&refusal_string(&pre, &cn, &post, w[3] + (i & 1), w[4]));
            }
        }
        "p_ref_u" => {
            for i in 0..n {
                acc += eat_units(&refusal_units(w[3] + (i & 1), w[4]));
            }
        }
        // reference only, not the bar: keeps one String across calls (not the same work as a fresh result)
        "p_amt_reuse" => {
            let mut s = String::new();
            for i in 0..n {
                s.clear();
                let _ = write!(s, "{}/{}", w[1] + (i & 1), w[2]);
                acc += eat_str(&s);
            }
        }
        "p_amt" => {
            for i in 0..n {
                acc += eat_str(&format!("{}/{}", w[1] + (i & 1), w[2]));
            }
        }
        "p_amt_u" => {
            for i in 0..n {
                acc += eat_units(&amount_units(w[1] + (i & 1), w[2]));
            }
        }
        // p_wr: index loop with Rust's bounds checks; p_wr_c: copy_from_slice (memcpy)
        "p_wr" | "p_wr_c" => {
            let x = w[1];
            let copy = op == "p_wr_c";
            let mut src: Vec<i64> = (0..64).map(|k| (x + k) & 65535).collect();
            let mut o = vec![0i64; 64];
            for i in 0..n {
                if copy {
                    o[..64].copy_from_slice(&src[..64]);
                } else {
                    let _ = put_range(&mut o, 0, &src, 0, src.len());
                }
                acc += (o[((i * 7) % 64) as usize] & 1) + 1;
                let j = (i % 64) as usize;
                src[j] = ((src[j] & 65535) + 1) & 65535;
            }
        }
        _ => panic!("unknown op {}", op),
    }
    acc
}

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut it = line.split_whitespace();
        let op = match it.next() { Some(o) => o.to_string(), None => continue };
        let w: Vec<i64> = it.map(|x| x.parse().unwrap()).collect();
        let t = Instant::now();
        let c = run(&op, &w);
        println!("ns {} {}", t.elapsed().as_nanos(), c);
    }
}

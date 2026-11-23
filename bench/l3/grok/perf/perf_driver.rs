// Grok history core driver (run_perf.py writes work/main.rs = header + include!("gen.rs") + `mod hand { include!("hand_history.rs"); }`
// + this file). One call = one window() over a window file, omitted=false, short ASCII title/model; the returned Vec<i64> is the
// read result packet. Rows: pruv (grok_history_window from the generated code), hand (hand::history_window).
//
//   perf_driver check <file>...        both rows on every file, omitted=false and omitted=true, packets compared word for word;
//                                      prints `file len fold` (the hand packet); exit 1 on any mismatch
//   perf_driver time <row> <file> <ms> per-call ns (median/min/max over calls for ~<ms> ms, at least 3) + checksum. A timed call
//                                      is window() including the drop of the returned Vec; the window is read before timing
//   perf_driver rss <row> <file>       read the window, one call, print the checksum, exit (run under /usr/bin/time -l: peak RSS,
//                                      which includes the input window bytes for both rows)
// Checksum = fold s = s*31 + w (wrapping, i64) over every packet word.
use std::hint::black_box;
use std::time::Instant;

const TITLE: &str = "Fixing the flaky retry in sync";
const MODEL: &str = "grok-4";

fn fold(p: &[i64]) -> i64 {
    let mut s: i64 = 0;
    for &w in p {
        s = s.wrapping_mul(31).wrapping_add(w);
    }
    s
}

fn run(row: &str, win: &[u8], omitted: bool) -> Vec<i64> {
    match row {
        "pruv" => grok_history_window(win, omitted, TITLE, MODEL),
        "hand" => hand::history_window(win, omitted, TITLE, MODEL),
        _ => panic!("row must be pruv|hand"),
    }
}

fn check(files: &[String]) -> i32 {
    let mut bad = 0;
    for f in files {
        let win = std::fs::read(f).unwrap_or_else(|e| panic!("read {}: {}", f, e));
        for omitted in [false, true] {
            let a = run("pruv", &win, omitted);
            let b = run("hand", &win, omitted);
            if a.len() != b.len() {
                eprintln!("MISMATCH {} omitted={}: pruv len {} vs hand len {}", f, omitted, a.len(), b.len());
                bad += 1;
            } else if let Some(i) = (0..a.len()).find(|&i| a[i] != b[i]) {
                eprintln!("MISMATCH {} omitted={}: word {} pruv {} vs hand {}", f, omitted, i, a[i], b[i]);
                bad += 1;
            }
            if !omitted {
                println!("{} {} {}", f, b.len(), fold(&b));
            }
        }
    }
    if bad > 0 {
        1
    } else {
        0
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()).unwrap_or("") {
        "check" => std::process::exit(check(&a[2..])),
        "time" => {
            let row = a[2].as_str();
            let win = std::fs::read(&a[3]).unwrap();
            let budget_ns: u128 = a[4].parse::<u128>().unwrap() * 1_000_000;
            black_box(run(row, black_box(&win), false)); // warm-up
            let mut t: Vec<u64> = Vec::new();
            let all = Instant::now();
            while t.len() < 3 || (all.elapsed().as_nanos() < budget_ns && t.len() < 1_000_000) {
                let t0 = Instant::now();
                let r = run(row, black_box(&win), false);
                black_box(&r);
                drop(r);
                t.push(t0.elapsed().as_nanos() as u64);
            }
            let sum = fold(&run(row, &win, false));
            t.sort_unstable();
            let n = t.len();
            let med = if n % 2 == 0 { (t[n / 2 - 1] + t[n / 2]) / 2 } else { t[n / 2] };
            println!("{} {} {} {} {}", med, t[0], t[t.len() - 1], t.len(), sum);
        }
        "rss" => {
            let row = a[2].as_str();
            let win = std::fs::read(&a[3]).unwrap();
            let r = run(row, black_box(&win), false);
            black_box(&r);
            println!("{}", fold(&r)); // after the peak: lets a caller compare every RSS sample with hand
        }
        _ => {
            eprintln!("usage: perf_driver check|time|rss ...");
            std::process::exit(2);
        }
    }
}

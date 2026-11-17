// Leg D driver: times one call of the Pruv kernel (sol) or the hand Rust kernel (hand) on a gen.py data file.
//   main <int|str> <pruv|hand> <datafile> <reps>
// Only the call is timed (not parsing). Prints {"variant","impl","best_ms","median_ms","checksum"}.
#![allow(dead_code, unused_imports, unused_variables)]
#[path = "@SOL@"] mod sol;
#[path = "@HAND@"] mod hand;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (variant, imp, file, reps) = (a[1].as_str(), a[2].as_str(), &a[3], a[4].parse::<usize>().unwrap());
    let text = std::fs::read_to_string(file).unwrap();
    let rows: Vec<Vec<i64>> = text.lines().map(|l| l.split(' ').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect()).collect();
    let (sess, items) = (&rows[0], &rows[1]);
    let empty = Vec::new();
    let pool = rows.get(2).unwrap_or(&empty);
    let k = 8;
    let mut times = Vec::new();
    let mut checksum = 0i64;
    for _ in 0..reps + 1 {
        let t0 = Instant::now();
        let out: Vec<i64> = match (variant, imp) {
            ("int", "pruv") => sol::bound_int(black_box(sess), black_box(k as i64), black_box(items)),
            ("int", "hand") => hand::bound_int(black_box(sess), black_box(k), black_box(items)),
            ("str", "pruv") => sol::bound_str(black_box(sess), black_box(k as i64), black_box(items), black_box(pool)),
            ("str", "hand") => hand::bound_str(black_box(sess), black_box(k), black_box(items), black_box(pool)),
            _ => panic!("bad args"),
        };
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        checksum = black_box(out).iter().sum();
    }
    times.remove(0); // first call is warm-up
    times.sort_by(|x, y| x.partial_cmp(y).unwrap());
    println!("{{\"variant\":\"{}\",\"impl\":\"{}\",\"best_ms\":{:.3},\"median_ms\":{:.3},\"checksum\":{}}}", variant, imp, times[0], times[times.len() / 2], checksum);
}

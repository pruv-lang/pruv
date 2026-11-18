// P11 admission driver (appended to a main file that already declares `mod sol;`, see run.py admit).
// Prints `case len result entries_visited free_steps slots_visited` for each case of root_admit.pruv, then the clean
// case on larger buffers (admission work grows with top + slots only).
fn main() {
    // keys: one packed word each, all four u16 lanes non-zero (kn = 4)
    let keys: Vec<i64> = (0..8i64).map(|i| 0x0001_0002_0003_0004 + (i << 16)).collect();
    let mut lines = Vec::new();
    for (case, len) in (0..21i64).map(|c| (c, 1024usize)).chain([(0i64, 4096usize), (0, 65536), (21, 65536), (22, 65536)]) {
        let mut st = vec![0i64; len];
        let mut cnt = vec![0i64; 3];
        let r = sol::admit_case(&mut st, &keys, case, &mut cnt);
        lines.push(format!("{} {} {} {} {} {}", case, len, r, cnt[0], cnt[1], cnt[2]));
    }
    println!("{}", lines.join("\n"));
}

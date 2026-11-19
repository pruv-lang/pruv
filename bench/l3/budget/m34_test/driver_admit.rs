// P11 admission driver (appended to a main file that already declares `mod sol;`, see run.py admit).
// Prints `case len result entries_visited free_steps slots_visited` for each case of root_admit.pruv, then the clean
// case on larger buffers (admission work grows with top + slots only). Then, for every admitted case at len 1024,
// `grow case result live_prev live_next pay_diff` from admit_grow (grow into a 2048-word buffer, L3 P13b; case 24 has
// a root payload that must survive, P13j). Last line `maxslot addr0 addr1`: entry_addr on a header-only store at s = 2^32.
fn main() {
    // keys: one packed word each, all four u16 lanes non-zero (kn = 4)
    let keys: Vec<i64> = (0..8i64).map(|i| 0x0001_0002_0003_0004 + (i << 16)).collect();
    let mut lines = Vec::new();
    for (case, len) in (0..21i64).map(|c| (c, 1024usize)).chain([(0i64, 4096usize), (0, 65536), (21, 65536), (22, 65536), (23, 65536), (24, 1024), (25, 1024), (26, 1024)]) {
        let mut st = vec![0i64; len];
        let mut cnt = vec![0i64; 3];
        let r = sol::admit_case(&mut st, &keys, case, &mut cnt);
        lines.push(format!("{} {} {} {} {} {}", case, len, r, cnt[0], cnt[1], cnt[2]));
    }
    for (case, len) in (0..21i64).map(|c| (c, 1024usize)).chain([(21i64, 65536usize), (22, 65536), (23, 65536), (24, 1024), (25, 1024), (26, 1024)]) {
        let mut st = vec![0i64; len];
        let mut nxt = vec![0i64; 2 * len];
        let mut cnt = vec![0i64; 3];
        let mut live = vec![0i64; 3];
        let g = sol::admit_grow(&mut st, &keys, case, &mut nxt, &mut cnt, &mut live);
        if g != -1000 {
            lines.push(format!("grow {} {} {} {} {}", case, g, live[0], live[1], live[2]));
        }
    }
    // L3 P13j: entry base at the maximum slot count (header words only, not a valid store)
    let mut h = vec![0i64; 64];
    h[3] = 16;
    h[4] = 1 << 32;
    h[5] = (1 << 32) + 16;
    h[6] = 1 << 31;
    lines.push(format!("maxslot {} {}", sol::entry_addr(&h, 0), sol::entry_addr(&h, 1)));
    println!("{}", lines.join("\n"));
}

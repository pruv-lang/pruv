fn hand_cmp(a: &[u16], b: &[u16]) -> i64 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn main() {
    let keys: [(i64, i64, i64); 7] = [
        (0, 0, 0),
        (1, 0xD800, 0),
        (1, 0xDFFF, 0),
        (2, 0xD83D, 0xDE00),
        (2, 0xDE00, 0xD83D),
        (1, 0xFFFF, 0),
        (2, 0xD800, 0xD800),
    ];
    let mut line = String::new();
    let mut hand = String::new();
    for &(n, x, y) in keys.iter() {
        for &(m, u, v) in keys.iter() {
            let got = sol::compare(std::hint::black_box(n), x, y, m, u, v);
            let ka: Vec<u16> = [x as u16, y as u16][..n as usize].to_vec();
            let kb: Vec<u16> = [u as u16, v as u16][..m as usize].to_vec();
            line.push_str(&format!("{} ", got));
            hand.push_str(&format!("{} ", hand_cmp(&ka, &kb)));
        }
    }
    println!("{}", line.trim_end());
    println!("{}", if line == hand { "same as hand" } else { "differs from hand" });
    println!("{}", sol::grow());
    let n1 = std::hint::black_box(1i64);
    println!(
        "{} {} {} {} {}",
        sol::eq_units(0, 256, n1),
        sol::eq_units(256, 256, n1),
        sol::eq_units(0, 256, std::hint::black_box(0i64)),
        sol::eq_one(0, 256),
        sol::eq_one(0xFFFF, 0xFFFF)
    );
}

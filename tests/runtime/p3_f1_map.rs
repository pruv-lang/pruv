fn main() {
    let a: Vec<i64> = (0..2000i64).map(|k| k % 1000).collect();
    let b: Vec<i64> = (0..2000i64).map(|k| (k * 7) % 1001).collect();
    let c = sol::add(&a, &b);
    println!("{}", c.len());
    println!("{}", c[0]);
    println!("{}", c[1]);
    println!("{}", c[1999]);
    let m = sol::mix(&a, &b);
    println!("{}", m[0]);
    println!("{}", m[1]);
    println!("{}", m[1999]);
    let mut bad = b.clone();
    bad[1500] = 1001;
    let d = sol::add(&a, &bad);
    println!("unreachable {}", d.len());
}

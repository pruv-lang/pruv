fn main() {
    let a: Vec<i64> = (0..10i64).collect();
    let b: Vec<i64> = (0..20i64).map(|k| k * 2).collect();
    let c = sol::add(&a, &b);
    println!("{}", c.len());
    println!("{}", c[9]);
    let mut b2 = b.clone();
    b2[15] = 5000;
    let d = sol::add(&a, &b2);
    println!("unreachable {}", d.len());
}

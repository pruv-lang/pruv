fn main() {
    let a: Vec<i64> = vec![10i64; 1000];
    let b: Vec<i64> = vec![20i64; 1000];
    let c = sol::add(&a, &b);
    println!("{}", c.len());
    println!("{}", c[999]);
    let mut a2 = a.clone();
    let mut b2 = b.clone();
    a2[900] = 2000;
    b2[5] = 5000;
    let d = sol::add(&a2, &b2);
    println!("unreachable {}", d.len());
}

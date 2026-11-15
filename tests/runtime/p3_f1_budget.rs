fn main() {
    let c = sol::add(&vec![1i64; 400], &vec![2i64; 400]);
    println!("{}", c.len());
    println!("{}", c[399]);
    let d = sol::add(&vec![1i64; 600], &vec![2i64; 600]);
    println!("unreachable {}", d.len());
}

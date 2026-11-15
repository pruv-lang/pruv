fn main() {
    sol::small(&vec![50i64; 500]);
    let mut a = vec![50i64; 500];
    a[400] = 101;
    sol::small(&a);
    println!("unreachable");
}

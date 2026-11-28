fn main() {
    let v: Vec<u16> = sol::keep(vec![0xD800u16, 7]);
    println!("{} {}", v.len(), v[0]);
    let w: Vec<u16> = sol::dup(&[1u16, 2, 3]);
    println!("{}", w.iter().map(|x| *x as u64).sum::<u64>());
    println!("{}", sol::dup(&[]).len());
}

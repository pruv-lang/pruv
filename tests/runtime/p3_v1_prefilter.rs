fn main() {
    let a: Vec<i64> = (0..1000).map(|i| i % 101).collect();
    sol::small(&a);
    let w: Vec<u8> = (0..999u32).map(|i| (i % 3) as u8 + 1).collect();
    sol::wave(&w);
    let mut b = vec![5i64; 300];
    b[299] = -1;
    sol::has_neg(&b);
    b[299] = 7;
    sol::sevens_or(&b, 1);
    sol::sevens_or(&vec![7i64; 64], 2);
    sol::past_end(&vec![3i64; 40], 100);
    sol::bounded(&a, 100);
    sol::none(&vec![]);
}

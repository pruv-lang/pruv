fn main() {
    let sid: [u16; 3] = [0xD800, 7, 0xFFFF];
    println!("{}", sol::first_unit(&sid));
    println!("{}", sol::unit_sum(&sid));
    println!("{}", sol::byte_at(&[1u8, 2, 255], 2u32));
    println!("{}", sol::widen(255u8, 65535u16, 4294967295u32, 2147483647i32));
    println!("{}", sol::halve(u64::MAX));
    println!("{}", sol::negate(-2147483647i32));
    println!("{}", sol::keep_big(u64::MAX));
    println!("{}", sol::small(-128i8, 32767i16 - 200));
    println!("{}", sol::pair_sum(sol::Pair { lo: 255, hi: 65535 }));
}

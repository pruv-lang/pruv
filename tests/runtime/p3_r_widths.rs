fn main() {
    let b = std::hint::black_box;
    println!("{} {}", sol::e_u32(b(0)), sol::e_u32(b(4294967294)));
    println!("{} {}", sol::e_i32(b(-2147483648)), sol::e_i32(b(2147483646)));
    println!("{} {}", sol::e_i8(b(-128)), sol::e_i8(b(126)));
    println!("{} {}", sol::e_i16(b(-32768)), sol::e_i16(b(32766)));
    println!("{} {}", sol::e_isize(b(i64::MIN)), sol::e_isize(b(i64::MAX - 1)));
    println!("{} {}", sol::e_u64(b(0)), sol::e_u64(b(i64::MAX)));
    println!("{} {}", sol::e_usize(b(0)), sol::e_usize(b(1000)));
    println!("{} {}", sol::narrow8(b(0)), sol::narrow8(b(127)));
}

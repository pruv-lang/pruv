fn main() {
    let sid: [u16; 2] = [0xD800, 7];
    println!("{}", sol::mark_at(&sid, 0u32));
    println!("{}", sol::mark_at(&sid, 5u32));
}

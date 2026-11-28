fn main() {
    let sid: [u16; 3] = [1, 0xD800, 7];
    println!("{}", sol::after_mark(&sid, 1u32));
    println!("{}", sol::after_mark(&sid, 0u32));
}

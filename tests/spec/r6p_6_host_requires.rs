fn main() {
    let sid: [u16; 2] = [3, 4];
    println!("{}", sol::pick(&sid, 1u8));
    println!("{}", sol::pick(&sid, 2u8));
}

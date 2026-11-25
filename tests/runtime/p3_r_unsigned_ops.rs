fn main() {
    let b = std::hint::black_box;
    sol::ops(b(3), b(-128));
    sol::ops(b(0), b(127));
}

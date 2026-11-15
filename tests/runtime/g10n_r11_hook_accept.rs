fn main() {
    // no peer: with the hook the accept fails at once instead of blocking
    for _ in 0..3 {
        assert_eq!(sol::accept_fault("127.0.0.1"), 0, "accept fault");
    }
    println!("accept faults are values");
}

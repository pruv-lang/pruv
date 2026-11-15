fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::wait_fail(&child), 0);
    let big = vec![1u8; 1 << 20];
    assert_eq!(sol::first_error_wins(&child, &big), 0);
}

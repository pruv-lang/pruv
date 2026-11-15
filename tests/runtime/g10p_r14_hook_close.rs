fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::close_fail(&child), 0);
    assert_eq!(sol::close_fail(&child), 0);
}

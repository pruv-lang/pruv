fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::copy_after(&child, b"x\nyz\n"), 0);
}

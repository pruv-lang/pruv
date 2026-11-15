fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::read_fail(&child), 0);
    assert_eq!(sol::read_all_fail(&child), 0);
    assert_eq!(sol::stderr_still_reads(&child), 0);
}

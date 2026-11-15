fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // 4 MiB on stderr (pipes hold 16-64 KiB) before the first byte of stdout
    assert_eq!(sol::stderr_flood(&child, "4194304", "1000", 4_194_304, 1000), 0);
    assert_eq!(sol::stderr_flood(&child, "300000", "300000", 300_000, 300_000), 0);
    assert_eq!(sol::stderr_flood(&child, "1000000", "0", 1_000_000, 0), 0);
    // wait alone keeps draining and discarding both outputs
    assert_eq!(sol::wait_only(&child, "3000000", "3000000"), 0);
}

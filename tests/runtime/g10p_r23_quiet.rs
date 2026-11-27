fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // pattern(2, 0..4) is "\x132Qp"; the runner checks that "2Qp" never shows on this process's stderr
    assert_eq!(sol::quiet_run(&child, "4", "1000", 1000), 0);
    // 4 MiB on stderr, more than a pipe holds, before the first byte of stdout: nothing blocks
    assert_eq!(sol::quiet_run(&child, "4194304", "300000", 300_000), 0);
    assert_eq!(sol::quiet_exit(&child, "7"), 7);
    assert_eq!(sol::quiet_bad("/nonexistent/pruv-quiet-child"), 2);
    assert_eq!(sol::quiet_bad(""), 1);
}

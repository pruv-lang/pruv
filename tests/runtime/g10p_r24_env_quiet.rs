fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(
        sol::eq_env(&child, "PRUV_ENV_NEW", "fresh", "PRUV_ENV_NEW", "PRUV_ENV_NONE", "PRUV_ENV_NONE",
                    b"PRUV_ENV_NEW=fresh\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n"),
        0
    );
    // pattern(2, 0..4) is "\x132Qp"; the runner checks that "2Qp" never shows on this process's stderr
    assert_eq!(sol::eq_err(&child, "4", "1000"), 1000);
    assert_eq!(sol::eq_err(&child, "4194304", "300000"), 300_000);
    assert_eq!(sol::eq_ev(&child, "4194304", "300000"), 300_000);
    assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(sol::eq_bad(&child, "A=B", "v"), 1);
    assert_eq!(sol::eq_bad("", "PRUV_ENV_A", "v"), 1);
    assert_eq!(sol::eq_bad("/nonexistent/pruv-quiet-child", "PRUV_ENV_A", "v"), 2);
    println!("env quiet ok");
}

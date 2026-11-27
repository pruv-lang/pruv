fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    assert_eq!(sol::quiet_eof(&child, "4194304", "300000"), 300_000);
    assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0);
    println!("quiet eof ok");
}

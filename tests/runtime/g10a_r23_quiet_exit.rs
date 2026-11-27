fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    for _ in 0..20 {
        assert_eq!(sol::quiet_exit(&child), 3);
    }
    assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0);
    println!("quiet exit ok");
}

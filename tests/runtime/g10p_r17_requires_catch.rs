use std::sync::Mutex;

static LAST: Mutex<String> = Mutex::new(String::new());

// the child process is `quick` (exits at once), so a violation that leaves a handle behind is cleaned up by its Drop
fn expect_violation<F: FnOnce() -> i64 + std::panic::UnwindSafe>(op: &str, f: F) {
    LAST.lock().unwrap().clear();
    let r = std::panic::catch_unwind(f);
    let msg = LAST.lock().unwrap().clone();
    assert!(r.is_err(), "{}: no panic", op);
    let want = format!("pruv: foreign requires violated: {}:", op);
    assert!(msg.starts_with(&want), "{}: message {:?}", op, msg);
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    std::panic::set_hook(Box::new(|info| {
        let p = info.payload();
        let m = if let Some(s) = p.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = p.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        };
        *LAST.lock().unwrap() = m;
    }));
    let missing = "/no/such/dir/pruv-missing-program";

    // an OS failure is a value: error 2, wait returns Err(2): 2 * 1000 + (1000 + 2)
    assert_eq!(sol::os_failure(missing), 3002);
    assert!(LAST.lock().unwrap().is_empty(), "the OS failure panicked");

    expect_violation("proc_write_str", || sol::write_closed(&child));
    expect_violation("proc_write_bytes", || sol::write_bytes_closed(&child, &[1, 2, 3]));
    expect_violation("proc_close_stdin", || sol::close_twice(&child));
    expect_violation("proc_read", || sol::read_bad_stream(&child));
    expect_violation("proc_read", || sol::read_zero_max(&child));
    expect_violation("proc_read", || sol::read_huge_max(&child));
    expect_violation("proc_read_all", || sol::read_all_open(&child));
    expect_violation("proc_byte", || sol::byte_out_of_range(&child));
    expect_violation("proc_eof", || sol::eof_bad_stream(&child));
    expect_violation("proc_size", || sol::size_bad_stream(&child));
    expect_violation("proc_bytes", || sol::bytes_bad_stream(&child));
    expect_violation("proc_read", || sol::read_failed_handle(missing));
    expect_violation("proc_write_str", || sol::write_failed_handle(missing));

    // after all those panics the runtime still works
    assert_eq!(sol::os_failure(missing), 3002);
    println!("survived");
}

use std::net::TcpListener;
use std::sync::Mutex;

static LAST: Mutex<String> = Mutex::new(String::new());

fn expect_violation<F: FnOnce() -> i64 + std::panic::UnwindSafe>(op: &str, f: F) {
    LAST.lock().unwrap().clear();
    let r = std::panic::catch_unwind(f);
    let msg = LAST.lock().unwrap().clone();
    assert!(r.is_err(), "{}: no panic", op);
    let want = format!("pruv: foreign requires violated: {}:", op);
    assert!(msg.starts_with(&want), "{}: message {:?}", op, msg);
}

// a connect to the port of a closed listener; another process may bind that port before the connect, so a bounded
// number of fresh ports
fn os_failure_refused() -> i64 {
    let mut got = 0;
    for _ in 0..5 {
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port() as i64
        };
        got = sol::os_failure("127.0.0.1", closed);
        if got == 3002 {
            break;
        }
    }
    got
}

fn main() {
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
    let h = "127.0.0.1";
    // a live listener that never calls accept (the backlog completes the handshake); a failed handle comes from an empty
    // host (error 1, the OS is never asked)
    let live = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = live.local_addr().unwrap().port() as i64;

    // an OS failure is a value: error 2, and the close returns Err(2): 2 * 1000 + (1000 + 2)
    assert_eq!(os_failure_refused(), 3002);
    assert!(LAST.lock().unwrap().is_empty(), "the OS failure panicked");

    expect_violation("net_write_str", || sol::write_str_after_shutdown(h, port));
    expect_violation("net_write_bytes", || sol::write_bytes_after_shutdown(h, port, &[1, 2, 3]));
    expect_violation("net_shutdown_write", || sol::shutdown_twice(h, port));
    expect_violation("net_read", || sol::read_zero_max(h, port));
    expect_violation("net_read", || sol::read_huge_max(h, port));
    expect_violation("net_set_timeout", || sol::timeout_negative(h, port));
    expect_violation("net_set_timeout", || sol::timeout_huge(h, port));
    expect_violation("net_byte", || sol::byte_out_of_range(h, port));
    expect_violation("net_read", || sol::read_failed_handle("", 80));
    expect_violation("net_read_all", || sol::read_all_failed_handle("", 80));
    expect_violation("net_write_str", || sol::write_failed_handle("", 80));
    expect_violation("net_shutdown_write", || sol::shutdown_failed_handle("", 80));
    expect_violation("net_set_timeout", || sol::timeout_failed_handle("", 80));
    expect_violation("net_connect", || sol::connect_port_zero(h));
    expect_violation("net_connect", || sol::connect_port_big(h));
    expect_violation("net_listen", || sol::listen_port_negative(h));
    expect_violation("net_listen", || sol::listen_port_big(h));
    expect_violation("net_accept", || sol::accept_failed_listener(h));

    // after all those panics the runtime still works
    LAST.lock().unwrap().clear();
    assert_eq!(os_failure_refused(), 3002);
    assert!(LAST.lock().unwrap().is_empty(), "the OS failure panicked after the violations");
    println!("survived");
}

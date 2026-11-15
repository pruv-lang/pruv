use std::net::TcpListener;

fn main() {
    // the port is taken by a live listener of this process: listener error 2 (twice, the failure leaves nothing behind)
    let busy = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy_port = busy.local_addr().unwrap().port();
    assert_eq!(sol::listen_error("127.0.0.1", busy_port as i64), 2, "port in use");
    assert_eq!(sol::listen_error("127.0.0.1", busy_port as i64), 2, "port in use again");

    // bad input never reaches the OS: listener error 1
    assert_eq!(sol::listen_error("", 0), 1, "empty host");
    assert_eq!(sol::listen_error("", busy_port as i64), 1, "empty host with a port");
    assert_eq!(sol::listen_error("a\0b", 0), 1, "NUL in the host");

    // an ephemeral port is bound, reported and closed again
    assert_eq!(sol::listen_error("127.0.0.1", 0), 0, "ephemeral port");

    // the reported port is the real one: the listener accepts a connection made to exactly that port, and what the
    // client wrote arrives on the accepted socket
    let payload: Vec<u8> = (0..=255u8).chain(0..=9u8).collect();
    for _ in 0..4 {
        let port = sol::self_connect("127.0.0.1", &payload);
        assert!(port >= 1 && port <= 65535, "self_connect returned {}", port);
    }
    println!("listeners ok");
}

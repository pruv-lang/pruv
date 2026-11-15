use std::net::TcpListener;

// the port of a listener that has been closed: connection refused
fn closed_port() -> i64 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port() as i64
}

// another process may bind a closed port before the connect: a bounded number of fresh ports, each tried twice
fn refused_twice() -> (i64, i64) {
    let mut last = (0, 0);
    for _ in 0..5 {
        let closed = closed_port();
        last = (sol::connect_error("127.0.0.1", closed), sol::connect_error("127.0.0.1", closed));
        if last == (2, 2) {
            break;
        }
    }
    last
}

fn main() {
    assert_eq!(refused_twice(), (2, 2), "refused, and refused again");

    // bad input never reaches the OS
    assert_eq!(sol::connect_error("", 80), 1, "empty host");
    assert_eq!(sol::connect_error("local\0host", 80), 1, "NUL in the host");
    assert_eq!(sol::connect_error("127.0.0.1\0", 80), 1, "trailing NUL");
    assert_eq!(sol::connect_error("\0", 65535), 1, "only a NUL");

    // control: a live listener (it never calls accept, the backlog completes the handshake)
    let live = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = live.local_addr().unwrap().port();
    assert_eq!(sol::connect_error("127.0.0.1", port as i64), 0, "live listener");
    println!("errors are values");
}

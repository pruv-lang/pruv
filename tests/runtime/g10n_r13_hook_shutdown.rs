use std::net::TcpListener;

fn main() {
    // the listener never calls accept: the backlog completes the handshake, which is all the client needs
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port() as i64;
    for _ in 0..3 {
        assert_eq!(sol::shutdown_fault("127.0.0.1", port), 0, "net_shutdown_write fault");
    }
    println!("shutdown faults are values");
}

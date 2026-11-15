use std::net::TcpListener;

fn main() {
    // the listener never calls accept: the backlog completes the handshake, which is all the client needs
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port() as i64;
    assert_eq!(sol::write_str_fault("127.0.0.1", port), 0, "net_write_str fault");
    assert_eq!(sol::write_bytes_fault("127.0.0.1", port, &[1, 2, 3]), 0, "net_write_bytes fault");
    assert_eq!(sol::write_bytes_fault("127.0.0.1", port, &[]), 0, "net_write_bytes fault, no data");
    println!("write faults are values");
}

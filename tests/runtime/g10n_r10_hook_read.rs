use std::net::TcpListener;

fn main() {
    // the listener never calls accept: the backlog completes the handshake, which is all the client needs
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port() as i64;
    assert_eq!(sol::read_fault("127.0.0.1", port), 0, "net_read fault");
    assert_eq!(sol::read_all_fault("127.0.0.1", port), 0, "net_read_all fault");
    // the runtime is still usable after a fault
    assert_eq!(sol::read_fault("127.0.0.1", port), 0, "net_read fault again");
    println!("read faults are values");
}

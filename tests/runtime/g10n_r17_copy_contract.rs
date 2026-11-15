use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    // the peer sends one message, reads to EOF (the client's shutdown_write) and closes
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.write_all(b"hello").unwrap();
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());
    });
    assert_eq!(sol::copy_after("127.0.0.1", port as i64, b"hello"), 0);
    peer.join().unwrap();
    println!("copied");
}

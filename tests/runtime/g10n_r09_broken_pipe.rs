use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

// a peer that accepts one connection, reads nothing and closes it at once
fn closing_peer() -> (u16, Receiver<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        drop(s);
        let _ = tx.send(());
    });
    (port, rx)
}

fn main() {
    // Rust's std ignores SIGPIPE in a Rust main (this driver) and sets SO_NOSIGPIPE where it exists; the generated runtime
    // installs no signal handler. Either way the write error is a value (error 4), and the process goes on.
    let data = vec![0x5au8; 64 * 1024];
    for round in 0..3 {
        let (port, closed) = closing_peer();
        let r = sol::broken_write("127.0.0.1", port as i64, &data, 100_000);
        assert_eq!(r, 0, "broken pipe, round {}", round);
        closed.recv_timeout(Duration::from_secs(30)).expect("the closing peer did not finish");
    }
    println!("survived");
}

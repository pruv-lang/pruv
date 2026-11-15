use std::io::Write;
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

fn spawn_peer<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

fn wait<T>(rx: &Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(60)).expect("the peer did not finish")
}

fn pattern(i: usize) -> u8 {
    ((i * 31 + 5) % 251) as u8
}

// sender: accept one connection, write `total` pattern bytes in one write, close
fn sender(total: usize) -> (u16, Receiver<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let rx = spawn_peer(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        let data: Vec<u8> = (0..total).map(pattern).collect();
        s.write_all(&data).unwrap();
    });
    (port, rx)
}

fn main() {
    // always more than the 250 bytes the two small reads can take, so neither of them ends at EOF
    for total in [300usize, 5_000, 100_000, 1_500_000] {
        let (port, peer) = sender(total);
        let r = sol::no_repeat("127.0.0.1", port as i64, total as i64);
        assert_eq!(r, 0, "stream of {} bytes", total);
        wait(&peer);
    }
    println!("no repeats");
}

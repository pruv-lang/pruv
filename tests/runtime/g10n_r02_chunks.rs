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

// sender: accept one connection and write `total` pattern bytes in writes of changing sizes (13, 1, 4096, 7, 65536, 1000
// bytes), then close; the chunk boundaries the client sees are up to the OS
fn sender(total: usize) -> (u16, Receiver<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let rx = spawn_peer(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_nodelay(true).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        let data: Vec<u8> = (0..total).map(pattern).collect();
        let sizes = [13usize, 1, 4096, 7, 65536, 1000];
        let (mut off, mut k) = (0usize, 0usize);
        while off < data.len() {
            let n = std::cmp::min(sizes[k % sizes.len()], data.len() - off);
            s.write_all(&data[off..off + n]).unwrap();
            off += n;
            k += 1;
        }
    });
    (port, rx)
}

fn main() {
    // (bytes, max): tiny, odd and huge chunk limits; an empty stream; sizes around the runtime's block sizes; a transfer
    // larger than the 1 MiB one-call cap with a max of 2^40
    let cases: Vec<(usize, i64)> = vec![
        (20_000, 7),
        (5_000, 4096),
        (100_000, 8192),
        (8_191, 1),
        (65_537, 65_536),
        (70_000, 100_000),
        (1, 1),
        (0, 5),
        (2_500_000, 1 << 40),
        (300_000, 1_048_575),
    ];
    for (total, max) in cases {
        let (port, peer) = sender(total);
        let r = sol::chunked("127.0.0.1", port as i64, max, total as i64);
        assert_eq!(r, 0, "bytes {} max {}", total, max);
        wait(&peer);
    }
    println!("chunked");
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

// run `f` on its own thread; the result (or the panic of the thread) is awaited with a bound
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

// echo server on 127.0.0.1: accept one connection, read to EOF, answer with the same bytes in 1021-byte writes, close
fn echo_server() -> (u16, Receiver<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let rx = spawn_peer(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).unwrap();
        for chunk in got.chunks(1021) {
            s.write_all(chunk).unwrap();
        }
        got
    });
    (port, rx)
}

fn main() {
    let binary: Vec<u8> = (0..=255u8).collect();
    let mid: Vec<u8> = (0..70_000u32).map(|i| (i % 253) as u8).collect();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| ((i.wrapping_mul(7) + 1) % 251) as u8).collect();
    let cases: Vec<Vec<u8>> = vec![vec![], vec![0u8], vec![10, 13, 0, 255], binary, mid, big];
    for data in &cases {
        let (port, peer) = echo_server();
        let r = sol::echo_roundtrip("127.0.0.1", port as i64, data, data);
        assert_eq!(r, 0, "echo of {} bytes", data.len());
        // the peer received exactly what was written
        assert_eq!(&wait(&peer), data, "peer input of {} bytes", data.len());
    }
    println!("echoed");
}

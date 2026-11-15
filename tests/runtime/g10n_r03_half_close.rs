use std::io::{Read, Write};
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

// the peer reads to EOF (it only returns once the client half-closed), then answers and closes
fn peer(reply: &'static [u8]) -> (u16, Receiver<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let rx = spawn_peer(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).unwrap();
        s.write_all(reply).unwrap();
        got
    });
    (port, rx)
}

fn main() {
    let cases: Vec<(&str, &'static [u8])> = vec![
        ("request", &b"reply after EOF"[..]),
        ("", &b"no request at all"[..]),
        ("\u{65e5}\u{672c}\u{8a9e} \u{1f642}\n", &b"\x00\x01\xff"[..]),
        ("x", &b""[..]),
    ];
    for (text, reply) in cases {
        let (port, rx) = peer(reply);
        let r = sol::half_close("127.0.0.1", port as i64, text, reply);
        assert_eq!(r, 0, "text {:?}", text);
        assert_eq!(wait(&rx), text.as_bytes().to_vec(), "the peer saw the request and then EOF");
    }
    println!("half-closed");
}

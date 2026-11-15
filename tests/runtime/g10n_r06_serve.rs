use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
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

// a port that is free right now (the listener that held it is closed again)
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

// the Pruv listener is not up yet when the client threads start: connect is retried (bounded) until it is
fn connect_retry(port: u16) -> TcpStream {
    for _ in 0..1000 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            return s;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the Pruv listener never came up on port {}", port);
}

// one client after the other: connect, write the payload, half-close, read the reply to EOF
fn clients(port: u16, payloads: Vec<Vec<u8>>) -> Receiver<Vec<Vec<u8>>> {
    spawn_peer(move || {
        let mut replies = Vec::new();
        for p in payloads {
            let mut s = connect_retry(port);
            s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
            s.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
            s.write_all(&p).unwrap();
            s.shutdown(Shutdown::Write).unwrap();
            let mut reply = Vec::new();
            s.read_to_end(&mut reply).unwrap();
            replies.push(reply);
        }
        replies
    })
}

fn main() {
    let binary: Vec<u8> = (0..=255u8).collect();
    let big: Vec<u8> = (0..200_000u32).map(|i| ((i * 13 + 3) % 251) as u8).collect();
    let payloads: Vec<Vec<u8>> = vec![b"first".to_vec(), vec![], binary, big, b"last\n".to_vec()];
    // another process may bind the free port before net_listen does (serve_n returns 100): a bounded number of fresh
    // ports, each with its own client thread (a thread of an abandoned try is never waited on)
    let mut attempt = None;
    for _ in 0..5 {
        let port = free_port();
        let rx = clients(port, payloads.clone());
        let r = sol::serve_n("127.0.0.1", port as i64, payloads.len() as i64);
        if r != 100 {
            attempt = Some((r, rx));
            break;
        }
    }
    let (r, rx) = attempt.expect("net_listen failed on five fresh ports");
    assert_eq!(r, 0, "serve_n");
    let replies = wait(&rx);
    assert_eq!(replies.len(), payloads.len());
    for (i, (got, want)) in replies.iter().zip(payloads.iter()).enumerate() {
        assert_eq!(got, want, "client {} got its own bytes back", i);
    }
    println!("served");
}

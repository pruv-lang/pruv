use std::io::Read;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

// more sockets than a default descriptor limit (256 on some systems) allows at the same time; kept close to it because
// every round takes a loopback port that stays in TIME_WAIT for a while, and parallel runs share the ephemeral range
const ROUNDS: i64 = 300;

fn spawn_peer<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

fn wait<T>(rx: &Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(120)).expect("the peer did not finish")
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn connect_retry(port: u16) -> TcpStream {
    for _ in 0..6000 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            return s;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the Pruv listener never came up on port {}", port);
}

fn main() {
    // connected sockets: the peer accepts and drops every connection
    for attempt in 0..5 {
        // a loaded machine may refuse a loopback connect now and then (connect_churn returns 100): a fresh listener and
        // peer thread after a short pause (an abandoned peer is never waited on)
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let peer = spawn_peer(move || {
            for _ in 0..ROUNDS {
                let (s, _) = l.accept().unwrap();
                drop(s);
            }
        });
        let r = sol::connect_churn("127.0.0.1", port as i64, ROUNDS);
        if r == 100 && attempt < 4 {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        assert_eq!(r, 0, "connect churn");
        wait(&peer);
        break;
    }

    // failed handles: every connect to a closed port is refused (error 2); another process may bind a closed port, so a
    // bounded number of fresh ports
    {
        let mut got = -1;
        for _ in 0..5 {
            got = sol::refused_churn("127.0.0.1", free_port() as i64, ROUNDS);
            if got == 0 {
                break;
            }
        }
        assert_eq!(got, 0, "refused churn");
    }

    // accepted sockets: clients connect one after the other, half-close and wait for the server's close
    for attempt in 0..5 {
        // another process may bind the free port before net_listen does (accept_churn returns 100): a fresh port and
        // client thread (an abandoned thread is never waited on)
        let port = free_port();
        let clients = spawn_peer(move || {
            for _ in 0..ROUNDS {
                let mut s = connect_retry(port);
                s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
                s.shutdown(Shutdown::Write).unwrap();
                let mut rest = Vec::new();
                s.read_to_end(&mut rest).unwrap();
                assert!(rest.is_empty());
            }
        });
        let r = sol::accept_churn("127.0.0.1", port as i64, ROUNDS);
        if r == 100 && attempt < 4 {
            continue;
        }
        assert_eq!(r, 0, "accept churn");
        wait(&clients);
        // the listener is closed: this process can bind the port itself right away
        TcpListener::bind(("127.0.0.1", port)).expect("the port is free after net_listener_close");
        break;
    }

    // a listener port is reused at once after net_listener_close, also after connections went through it
    for attempt in 0..5 {
        // the first listen may lose the free port to another process (reuse returns 100): a fresh port
        let port = free_port();
        let r = sol::reuse("127.0.0.1", port as i64, 50);
        if r == 100 && attempt < 4 {
            continue;
        }
        assert_eq!(r, 0, "port reuse");
        TcpListener::bind(("127.0.0.1", port)).expect("the port is free after the last net_listener_close");
        break;
    }
    println!("released");
}

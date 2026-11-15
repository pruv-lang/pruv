use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

// a peer that accepts one connection and then neither reads nor writes until it is released (or 30 s have passed)
fn silent_peer() -> (u16, Sender<()>, Receiver<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (release_tx, release_rx) = channel::<()>();
    let (done_tx, done_rx) = channel::<()>();
    std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(30));
        drop(s);
        let _ = done_tx.send(());
    });
    (port, release_tx, done_rx)
}

fn main() {
    // net_read and net_read_all against a silent peer: error 6 after (about) the timeout, not before and not much later
    for all in [false, true] {
        let (port, release, done) = silent_peer();
        let t0 = Instant::now();
        let r = sol::silent_read("127.0.0.1", port as i64, 300, all);
        let waited = t0.elapsed();
        assert_eq!(r, 0, "silent read (read_all: {})", all);
        assert!(waited >= Duration::from_millis(200), "returned after {:?}: no wait for the timeout", waited);
        assert!(waited < Duration::from_secs(20), "returned after {:?}", waited);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(30)).expect("the silent peer did not finish");
    }

    // a write that cannot make progress: the buffers of both sides fill up (a few MiB at most), then the write times out
    let (port, release, done) = silent_peer();
    let data = vec![0x5au8; 1 << 20];
    let t0 = Instant::now();
    let r = sol::silent_write("127.0.0.1", port as i64, 300, &data, 256);
    let waited = t0.elapsed();
    assert_eq!(r, 0, "silent write");
    assert!(waited < Duration::from_secs(30), "returned after {:?}", waited);
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(30)).expect("the silent peer did not finish");
    println!("timed out");
}

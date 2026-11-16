use std::io::{Read, Write};
use std::net::TcpListener;
fn main() {
    let mode: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    println!("PORT {}", l.local_addr().unwrap().port());
    std::io::stdout().flush().unwrap();
    let mut workers = Vec::new();
    for _ in 0..50 {
        let (mut s, _) = l.accept().unwrap();
        s.set_nodelay(true).unwrap();
        workers.push(std::thread::spawn(move || {
            // per_byte: one-byte reads and one-byte writes, the writes coalesced by a 4 KiB buffer like the Pruv
            // runtime writer coalesces queued one-byte ev_writes (one-byte TCP segments lose the tail on macOS).
            let mut out = std::io::BufWriter::with_capacity(4096, s.try_clone().unwrap());
            let mut buf = [0u8; 4096]; let mut total = 0;
            loop {
                let limit = if mode == 0 { 1 } else { 4096 };
                let n = s.read(&mut buf[..limit]).unwrap(); if n == 0 { break; }
                if mode == 0 { for b in &buf[..n] { out.write_all(&[*b]).unwrap(); } }
                else { let copy = buf[..n].to_vec(); s.write_all(&copy).unwrap(); }
                total += n;
            }
            out.flush().unwrap();
            assert_eq!(total, 65536);
        }));
    }
    for w in workers { w.join().unwrap(); }
}

use std::net::TcpListener;
use std::io::{Read,Write};
fn main() {
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port() as i64;
 let h=std::thread::spawn(move || {let (mut s,_)=l.accept().unwrap(); let bytes:Vec<u8>=(0..200000).map(|i|(i%251) as u8).collect();s.write_all(&bytes).unwrap();s.shutdown(std::net::Shutdown::Write).unwrap();let mut got=Vec::new();s.read_to_end(&mut got).unwrap();assert_eq!(got,bytes); });
 assert_eq!(sol::socket(port, 0, &[65u8]), 1);
 h.join().unwrap(); assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0); println!("async runtime ok");
}

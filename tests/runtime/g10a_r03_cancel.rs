use std::net::TcpListener;
use std::io::{Read,Write};
fn main() {
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port() as i64;
 let h=std::thread::spawn(move || {let (mut s,_)=l.accept().unwrap(); let mut b=[0];assert!(s.read(&mut b).map_or(true,|n|n==0)); });
 assert_eq!(sol::socket(port, 1, &[65u8]), 0);
 h.join().unwrap(); assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0); println!("async runtime ok");
}

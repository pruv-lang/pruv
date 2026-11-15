use std::net::TcpListener;
use std::io::{Read,Write};
fn main() {
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port() as i64;
 let h=std::thread::spawn(move || {let (mut s,_)=l.accept().unwrap(); let mut b=Vec::new();let _=s.read_to_end(&mut b); });
 assert_eq!(sol::socket(port, 2, &[65u8]), -4);
 h.join().unwrap(); assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0); println!("async runtime ok");
}

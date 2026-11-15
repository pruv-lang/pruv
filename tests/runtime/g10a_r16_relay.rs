use std::io::Read;
fn main(){
 let l=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port();
 let h=std::thread::spawn(move ||{let (mut s,_)=l.accept().unwrap();s.set_read_timeout(Some(std::time::Duration::from_secs(6))).unwrap();let mut got=Vec::new();s.read_to_end(&mut got).unwrap();let expected:Vec<u8>=(0..2097152u64).map(|i|((i*31+12)%251) as u8).collect();assert_eq!(got,expected);});
 let child=std::env::var("PRUV_PROC_CHILD").unwrap();assert_eq!(sol::relay(&child,"127.0.0.1",port as i64,5000),1);h.join().unwrap();
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0); println!("async runtime ok");}
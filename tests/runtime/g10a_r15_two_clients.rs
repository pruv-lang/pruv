use std::io::{Read,Write};
fn main(){
 let l=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port();drop(l);
 let h=std::thread::spawn(move ||sol::echo_two(port as i64));
 let peers:Vec<_>=(0..2).map(|id|std::thread::spawn(move ||{
  let mut s=None;for _ in 0..100{match std::net::TcpStream::connect(("127.0.0.1",port)){Ok(x)=>{s=Some(x);break;},Err(_)=>std::thread::sleep(std::time::Duration::from_millis(2))}}
  let mut s=s.unwrap();s.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
  let bytes:Vec<u8>=(0..50000+id*123).map(|i|((i+id*13)%251) as u8).collect();
  for b in bytes.chunks(997+id*100){s.write_all(b).unwrap();}
  s.shutdown(std::net::Shutdown::Write).unwrap();let mut got=Vec::new();s.read_to_end(&mut got).unwrap();assert_eq!(got,bytes);
 })).collect();
 for p in peers{p.join().unwrap();}assert_eq!(h.join().unwrap(),100123);
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0); println!("async runtime ok");}
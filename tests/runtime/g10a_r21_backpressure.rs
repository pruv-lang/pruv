use std::io::{Read,Write};
fn main(){
 let l=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=l.local_addr().unwrap().port();
 let h=std::thread::spawn(move ||{
  let(mut s,_)=l.accept().unwrap();let mut reader=s.try_clone().unwrap();
  let read=std::thread::spawn(move ||{let mut got=Vec::new();let mut buf=vec![0u8;65536];loop{match reader.read(&mut buf){Ok(0)=>break,Ok(n)=>got.extend_from_slice(&buf[..n]),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(std::time::Duration::from_millis(1)),Err(e)=>panic!("{e}")}}got});
  s.set_nonblocking(true).unwrap();
  let bytes:Vec<u8>=(0..33554432).map(|i|(i%251) as u8).collect();let mut off=0;let mut stopped=false;
  while off<bytes.len(){match s.write(&bytes[off..(off+65536).min(bytes.len())]){Ok(n)=>off+=n,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{stopped=true;break;},Err(e)=>panic!("{e}")}}
  assert!(stopped);s.set_nonblocking(false).unwrap();s.set_write_timeout(Some(std::time::Duration::from_secs(5))).unwrap();s.write_all(&bytes[off..]).unwrap();s.shutdown(std::net::Shutdown::Write).unwrap();assert_eq!(read.join().unwrap(),bytes);
 });
 assert_eq!(sol::socket(port as i64,3,&[]),1);h.join().unwrap();
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);println!("async runtime ok");}
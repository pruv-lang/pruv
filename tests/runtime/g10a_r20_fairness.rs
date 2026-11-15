use std::io::{Read,Write};
fn main(){
 let a=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let pa=a.local_addr().unwrap().port();
 let b=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let pb=b.local_addr().unwrap().port();
 let peers:Vec<_>=[a,b].into_iter().map(|l|std::thread::spawn(move ||{let(mut s,_)=l.accept().unwrap();s.write_all(&[1;100]).unwrap();let mut b=Vec::new();let _=s.read_to_end(&mut b);})).collect();
 assert_eq!(sol::fair(pa as i64,pb as i64),2);for h in peers{h.join().unwrap();}
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);println!("async runtime ok");}
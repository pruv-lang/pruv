use std::io::Write;
fn main() {
 let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
 let port = listener.local_addr().unwrap().port();
 let peer = std::thread::spawn(move || {
  let (mut socket, _) = listener.accept().unwrap();
  socket.write_all(&[65, 0, 255, 66, 67, 68]).unwrap();
  let mut b = [0];
  let _ = std::io::Read::read(&mut socket, &mut b);
 });
 assert_eq!(sol::copy_after(port as i64, &[65, 0, 255]), 1);
 peer.join().unwrap();
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0);
 println!("async runtime ok");
}

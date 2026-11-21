use std::io::{self, Write};
use std::net::TcpListener;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("read-into-child") {
        io::stdout().write_all(b"ABCDEF").unwrap();
        return;
    }
    let child = std::env::current_exe().unwrap();
    assert!(sol::process(child.to_str().unwrap()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = i64::from(listener.local_addr().unwrap().port());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"ABCDEF").unwrap();
    });
    assert!(sol::socket(port));
    server.join().unwrap();
    assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst), 0);
    println!("async runtime ok");
}

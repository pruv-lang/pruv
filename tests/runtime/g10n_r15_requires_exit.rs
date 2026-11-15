use std::net::TcpListener;

fn closed_port() -> i64 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port() as i64
}

fn main() {
    // an OS failure never takes the panic path: error 2, and the close returns Err(2): 2 * 1000 + 2. Another process may
    // bind a closed port before the connect: a bounded number of fresh ports.
    let mut got = 0;
    for _ in 0..5 {
        got = sol::os_failure("127.0.0.1", closed_port());
        if got == 2002 {
            break;
        }
    }
    assert_eq!(got, 2002);
    println!("os failure is a value");
    // a requires violation does: net_read on a failed socket (an empty host is error 1 without the OS) ends the process
    // with exit code 101
    sol::bad_read("", 80);
    println!("not reached");
}

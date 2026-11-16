fn main() {
    let mode: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let result = sol::serve(mode);
    if result == -1 {
        eprintln!("Pruv net_listen failed; probing bind for OS diagnostic");
        match std::net::TcpListener::bind(("127.0.0.1", 0)) {
            Err(e) => eprintln!("bind: {}", e),
            Ok(_) => eprintln!("diagnostic bind succeeded; original failure cause unavailable"),
        }
    }
    assert_eq!(result, 3276800);
}

fn main() {
    if let Some(which) = std::env::args().nth(1) {
        sol::bad(which.parse().unwrap());
        panic!("requires violation returned");
    }
    for which in ["0", "1"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg(which).output().unwrap();
        assert_eq!(output.status.code(), Some(101));
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("pruv: foreign requires violated:"));
    }
    println!("async runtime ok");
}

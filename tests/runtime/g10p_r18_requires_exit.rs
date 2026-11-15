fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // an OS failure never takes the panic path
    assert_eq!(sol::os_failure("/no/such/dir/pruv-missing-program"), 3002);
    println!("os failure is a value");
    // a requires violation does: exit code 101
    sol::bad_read(&child);
    println!("not reached");
}

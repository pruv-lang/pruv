fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // write after close: error 4 published, wait returns Err(4): 4 * 10000 + (1000 + 4)
    assert_eq!(sol::write_after_close(&child, 1), 41004);
    // read on a failed handle (error 2): the handle stays as it is, wait returns Err(2)
    assert_eq!(sol::read_after_failure(&child, 1), 21002);
    println!("no panic with checks off");
    // an invalid stream: the representation defense stops the process (exit 101)
    sol::bad_stream(&child, 3);
    println!("not reached");
}

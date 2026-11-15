fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // a good spawn: no error, wait gives Ok
    assert_eq!(sol::spawn_code(&child, "", "quick"), 9);
    // error 1 = bad input: empty program, NUL in the program, an argument or the cwd
    assert_eq!(sol::spawn_code("", "", "x"), 11);
    assert_eq!(sol::spawn_code("a\0b", "", "x"), 11);
    assert_eq!(sol::spawn_code(&child, "", "x\0y"), 11);
    assert_eq!(sol::spawn_code(&child, "no\0dir", "quick"), 11);
    // error 2 = the OS refused: a missing program, a missing working directory
    assert_eq!(sol::spawn_code("/no/such/dir/pruv-missing-program", "", "x"), 22);
    assert_eq!(sol::spawn_code(&child, "no-such-working-dir", "quick"), 22);
    #[cfg(unix)]
    {
        // a directory is not an executable
        assert_eq!(sol::spawn_code(".", "", "x"), 22);
    }
    // the process is still alive and well after every failure
    assert_eq!(sol::spawn_code(&child, "", "quick"), 9);
}

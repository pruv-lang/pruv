fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // exit 0 and a non-zero exit code are Ok values, told apart
    assert_eq!(sol::run_exit(&child, "0"), 0);
    assert_eq!(sol::run_exit(&child, "7"), 7);
    assert_eq!(sol::run_exit(&child, "255"), 255);
    // a running child is killed: on Unix the status is the negative signal number (SIGKILL = 9); never 0, never 9
    // (the exit code of `hold` when nobody kills it)
    let k = sol::kill_running(&child);
    assert!(k != 0 && k != 9 && k < 1000, "kill of a running child gave {}", k);
    #[cfg(unix)]
    assert_eq!(k, -9);
    // killing a child that already exited (or is just exiting) is a success, not error 6: either its own exit code 5
    // or the kill won the race on Unix (-9)
    for _ in 0..5 {
        let e = sol::kill_exited(&child);
        assert!(e == 5 || e == -9, "kill of an exited child gave {}", e);
    }
    // deterministic: the child has certainly exited (its pid is a zombie, awaited by a `waitfile` helper child inside the
    // Pruv function) before proc_kill runs, so the answer is exactly its own exit code, never the -9 of the race
    for i in 0..5 {
        let mark = format!("mark5_{}.txt", i);
        assert_eq!(sol::kill_exited_sync(&child, &mark), 5, "kill of a child that had certainly exited (round {})", i);
    }
    // a failed spawn: the handle is consumed by wait or kill and both return the spawn error as Err (1 or 2); an Ok result
    // would show as 1000 + code, so it cannot be mistaken for Err(2)
    assert_eq!(sol::failed_wait("/no/such/dir/pruv-missing-program"), 2);
    assert_eq!(sol::failed_kill("/no/such/dir/pruv-missing-program"), 2);
    assert_eq!(sol::failed_wait(""), 1);
    assert_eq!(sol::failed_kill(""), 1);
}

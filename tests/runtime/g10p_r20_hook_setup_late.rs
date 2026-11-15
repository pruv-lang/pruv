#[cfg(unix)]
extern "C" {
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
}

// Waits (bounded, 10 s) for the pid file of the `pidfile` fixture mode and returns the pid in it.
fn wait_pid_file(path: &str) -> Option<i32> {
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(path) {
            return Some(s.trim().parse().unwrap());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // watchdog: if the failure path hung (a reader thread that was never stopped or joined, a child left alive holding the
    // pipes), the process ends with exit 3 instead of blocking the suite
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = done.clone();
    std::thread::spawn(move || {
        for _ in 0..600 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
        }
        eprintln!("setup_late failure path did not return within 60 s");
        std::process::exit(3);
    });

    // control: the fixture really writes its pid file when nobody kills it (so that the check below is not vacuous)
    let mut c = std::process::Command::new(&child)
        .args(["pidfile", "control.pid"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let cpid = wait_pid_file("control.pid").expect("the pidfile fixture did not write its file");
    assert_eq!(cpid as u32, c.id());
    drop(c.stdin.take()); // EOF: the child exits 0
    assert!(c.wait().unwrap().success());

    // the faulty spawn: both readers were started, then the setup fails; the call must RETURN (readers stopped and joined,
    // the child killed so that its pipes reach EOF) with error 2
    assert_eq!(sol::setup_late_fail(&child, "late.pid"), 0);
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    // The direct child must already be reaped when spawn returned: the control child was waited above, so this process
    // has no child left at all. waitpid(-1, WNOHANG) gives 0 for a live child, a pid for an unreaped zombie, and -1 with
    // ECHILD (10) only when there is none. (It may have been killed before writing its pid file; this check does not care.)
    #[cfg(unix)]
    {
        let mut st: i32 = 0;
        let r = unsafe { waitpid(-1, &mut st, 1 /* WNOHANG */) };
        let errno = std::io::Error::last_os_error().raw_os_error();
        assert_eq!(r, -1, "a child of the failed spawn was not reaped (waitpid gave {})", r);
        assert_eq!(errno, Some(10), "expected ECHILD");
    }
}

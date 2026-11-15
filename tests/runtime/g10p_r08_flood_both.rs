// pattern(stream, i) = (i * 31 + stream * 7 + 5) % 251, the same formula as tests/g10_proc/child.rs
fn pattern0(n: usize) -> Vec<u8> {
    (0..n as u64).map(|i| ((i * 31 + 5) % 251) as u8).collect()
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // 2 MiB to each output and 1 MiB to stdin
    let nout: usize = 2 * 1024 * 1024;
    let nin: usize = 1024 * 1024;
    let data = pattern0(nin);
    let r = sol::flood(&child, &nout.to_string(), &nin.to_string(), &data, nout as i64);
    assert_eq!(r, 0);
    // smaller, around the pipe capacities
    for (a, b) in [(70_000usize, 70_000usize), (100_000, 0), (0, 100_000), (0, 0)] {
        let d = pattern0(b);
        let r = sol::flood(&child, &a.to_string(), &b.to_string(), &d, a as i64);
        assert_eq!(r, 0, "out {} in {}", a, b);
    }
}

#[cfg(unix)]
extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

#[cfg(unix)]
fn default_sigpipe() {
    unsafe {
        signal(13, 0);
    }
}

#[cfg(not(unix))]
fn default_sigpipe() {}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // A Rust main ignores SIGPIPE; put the default disposition back so that the test proves the generated runtime itself
    // keeps a broken pipe from killing the Pruv process (it ignores SIGPIPE before the first spawn).
    default_sigpipe();
    // far more than any pipe holds: the child exits without reading, so the write fails half way through
    let big = vec![0x5au8; 1 << 20];
    for _ in 0..3 {
        assert_eq!(sol::broken_write(&child, &big), 0);
    }
    // part of the write arrived, then the rest failed: `take 16` consumes exactly 16 bytes, saves them, and exits; the write
    // of 1 MiB publishes error 4 and the file holds exactly the first 16 bytes of the data (a pattern, not a constant)
    let data: Vec<u8> = (0..(1usize << 20)).map(|i| (i % 251) as u8).collect();
    let _ = std::fs::remove_file("take_out.txt");
    assert_eq!(sol::partial_write(&child, &data, "take_out.txt"), 0);
    assert_eq!(std::fs::read("take_out.txt").unwrap(), data[..16].to_vec());
    // control: the same child with exactly 16 bytes and a closed stdin prints them on stdout and finishes normally
    assert_eq!(sol::take_ok(&child, &data[..16]), 16);
    // closing stdin is a normal operation on a live child
    assert_eq!(sol::close_only(&child), 0);
    println!("survived");
}

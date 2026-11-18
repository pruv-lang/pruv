// a read-only anonymous mapping that is never touched: the slice is valid memory, so its length can be > 2^40
fn reserve(bytes: usize) -> *const u8 {
    extern "C" { fn mmap(a: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8; }
    #[cfg(target_os = "macos")]
    const FLAGS: i32 = 0x1000 | 0x2 | 0x40; // MAP_ANON | MAP_PRIVATE | MAP_NORESERVE
    #[cfg(not(target_os = "macos"))]
    const FLAGS: i32 = 0x20 | 0x2 | 0x4000; // MAP_ANONYMOUS | MAP_PRIVATE | MAP_NORESERVE
    let p = unsafe { mmap(std::ptr::null_mut(), bytes, 1, FLAGS, -1, 0) };
    assert!(p as isize != -1, "mmap failed");
    p
}

fn main() {
    let n = (1usize << 40) + 1;
    // zero bytes are valid UTF-8
    let t: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(reserve(n), n)) };
    println!("{}", sol::g(t));
}

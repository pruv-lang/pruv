fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // (stderr bytes, stdout bytes, max): tiny, odd and huge chunk limits; empty streams; sizes around the reader's 8192
    // byte block and the child's 4096 byte writes
    let cases: Vec<(i64, i64, i64)> = vec![
        (20_000, 30_000, 7),
        (5_000, 0, 4096),
        (0, 100_000, 8192),
        (8_191, 8_193, 1),
        (4_096, 4_097, 4_095),
        (70_000, 90_000, 100_000),
        (1, 1, 1),
        (0, 0, 5),
    ];
    for (ne, no, max) in cases {
        let r = sol::chunked(&child, &ne.to_string(), &no.to_string(), max, ne, no);
        assert_eq!(r, 0, "stderr {} stdout {} max {}", ne, no, max);
    }
}

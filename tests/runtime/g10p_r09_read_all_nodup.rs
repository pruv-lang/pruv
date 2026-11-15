fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    // more than the three small reads can take, so that read_all really has something left in both streams
    for (ne, no) in [(3_000i64, 50_000i64), (100_000, 100_000), (5_000, 400), (4_096, 4_096)] {
        let r = sol::no_repeat(&child, &ne.to_string(), &no.to_string(), ne, no);
        assert_eq!(r, 0, "stderr {} stdout {}", ne, no);
    }
}

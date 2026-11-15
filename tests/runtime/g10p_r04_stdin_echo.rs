fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    let binary: Vec<u8> = (0..=255u8).collect();
    let big: Vec<u8> = (0..300_000u32).map(|i| ((i * 7 + 1) % 251) as u8).collect();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("hello, child\n", binary.clone()),
        ("no newline at the end", vec![]),
        ("", binary.clone()),
        ("", vec![]),
        ("\u{65e5}\u{672c}\u{8a9e} \u{1f642}\n", vec![0, 0, 0, 10, 13, 255]),
        ("big", big),
    ];
    for (text, data) in &cases {
        let mut want: Vec<u8> = text.as_bytes().to_vec();
        want.extend_from_slice(data);
        let r = sol::echo_roundtrip(&child, text, data, &want);
        assert_eq!(r, 0, "text {:?} ({} data bytes)", text, data.len());
    }
    // nothing written at all: the explicit EOF still ends the child
    assert_eq!(sol::empty_session(&child), 0);
}

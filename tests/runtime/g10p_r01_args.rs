fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    let cases: Vec<[&str; 4]> = vec![
        ["plain", "two words", "", "x"],
        ["quote\"s", "single'q", "back\\slash", "tab\there"],
        ["h\u{e9}llo", "\u{65e5}\u{672c}\u{8a9e}", "emoji \u{1f642}", "  lead and trail  "],
        ["--flag=value", "-", "$HOME", "*.rs"],
        ["a;b|c&d", "`cmd`", "(paren)", "%PATH%"],
        ["", "", "", ""],
        ["trailing backslash\\", "\\\\two", "\"", "'"],
    ];
    for c in &cases {
        let mut want: Vec<u8> = Vec::new();
        for a in c.iter() {
            want.extend_from_slice(a.as_bytes());
            want.push(b'\n');
        }
        let r = sol::echo_args(&child, c[0], c[1], c[2], c[3], &want);
        assert_eq!(r, 0, "case {:?}", c);
    }
    // the builder keeps an empty string as one argument and never splits on spaces
    assert_eq!(sol::args_len("", "a b", ""), 3);
}

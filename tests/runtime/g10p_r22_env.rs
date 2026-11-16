#[allow(unused_unsafe)]
fn set_parent(k: &str, v: &str) {
    // set_var is unsafe from edition 2024 on, safe before
    unsafe {
        std::env::set_var(k, v);
    }
}

const NONE: &str = "PRUV_ENV_NONE";

// runs `child env q0 q1 q2` with the given pairs (names and values in order) and compares the child's report with `want`
fn go(child: &str, pairs: &[(&str, &str)], q: [&str; 3], want: &str) -> i64 {
    fn get<'a>(pairs: &[(&'a str, &'a str)], i: usize, k: usize) -> &'a str {
        pairs.get(i).map(|p| if k == 0 { p.0 } else { p.1 }).unwrap_or("")
    }
    let get = |i: usize, k: usize| get(pairs, i, k);
    let n = pairs.len() as i64;
    sol::env_run(
        child, n, n,
        get(0, 0), get(0, 1), get(1, 0), get(1, 1), get(2, 0), get(2, 1),
        q[0], q[1], q[2], want.as_bytes(),
    )
}

fn bad(child: &str, names: &[&str], values: &[&str]) -> i64 {
    let n = |l: &[&str], i: usize| -> String { l.get(i).map(|s| s.to_string()).unwrap_or_default() };
    sol::env_bad(child, names.len() as i64, values.len() as i64, &n(names, 0), &n(values, 0), &n(names, 1), &n(values, 1))
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    set_parent("PRUV_ENV_KEEP", "parent");
    set_parent("PRUV_ENV_OVER", "old");

    // (1) an inherited variable is visible, (2) a new one is set, an unset name is reported as unset
    assert_eq!(
        go(&child, &[("PRUV_ENV_NEW", "fresh")], ["PRUV_ENV_KEEP", "PRUV_ENV_NEW", NONE],
           "PRUV_ENV_KEEP=parent\nPRUV_ENV_NEW=fresh\nPRUV_ENV_NONE!\n"),
        0
    );
    // (3) a pair overrides a parent variable; the others stay
    assert_eq!(
        go(&child, &[("PRUV_ENV_OVER", "new")], ["PRUV_ENV_OVER", "PRUV_ENV_KEEP", NONE],
           "PRUV_ENV_OVER=new\nPRUV_ENV_KEEP=parent\nPRUV_ENV_NONE!\n"),
        0
    );
    // (4) a repeated name: the last pair wins, also over a parent variable
    assert_eq!(
        go(&child, &[("PRUV_ENV_DUP", "a"), ("PRUV_ENV_DUP", "b"), ("PRUV_ENV_DUP", "c")], ["PRUV_ENV_DUP", NONE, NONE],
           "PRUV_ENV_DUP=c\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n"),
        0
    );
    assert_eq!(
        go(&child, &[("PRUV_ENV_OVER", "1"), ("PRUV_ENV_DUP", "x"), ("PRUV_ENV_OVER", "2")], ["PRUV_ENV_OVER", "PRUV_ENV_DUP", NONE],
           "PRUV_ENV_OVER=2\nPRUV_ENV_DUP=x\nPRUV_ENV_NONE!\n"),
        0
    );
    // (5) an empty value is a set variable, and it does not remove a parent variable
    assert_eq!(
        go(&child, &[("PRUV_ENV_EMPTY", "")], ["PRUV_ENV_EMPTY", NONE, NONE], "PRUV_ENV_EMPTY=\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n"),
        0
    );
    assert_eq!(
        go(&child, &[("PRUV_ENV_KEEP", "")], ["PRUV_ENV_KEEP", NONE, NONE], "PRUV_ENV_KEEP=\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n"),
        0
    );
    // (6) Unicode, spaces, quotes, backslashes and shell characters arrive exactly
    let v1 = "h\u{e9}llo \u{65e5}\u{672c}\u{8a9e} \u{1f642}";
    let v2 = "  lead \"dq\" 'sq' back\\slash trail  ";
    let v3 = "$HOME %PATH% a;b|c&d `x` *.rs";
    let want = format!("PRUV_ENV_U1={}\nPRUV_ENV_U2={}\nPRUV_ENV_U3={}\n", v1, v2, v3);
    assert_eq!(
        go(&child, &[("PRUV_ENV_U1", v1), ("PRUV_ENV_U2", v2), ("PRUV_ENV_U3", v3)], ["PRUV_ENV_U1", "PRUV_ENV_U2", "PRUV_ENV_U3"], &want),
        0
    );
    // (7) zero pairs is a plain inherit
    assert_eq!(
        go(&child, &[], ["PRUV_ENV_KEEP", "PRUV_ENV_OVER", NONE], "PRUV_ENV_KEEP=parent\nPRUV_ENV_OVER=old\nPRUV_ENV_NONE!\n"),
        0
    );
    // a bare program name is looked up on the new PATH (Unix: the PATH of the pairs)
    #[cfg(unix)]
    {
        let path = std::path::Path::new(&child);
        let dir = path.parent().unwrap().to_str().unwrap().to_string();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let want = format!("PATH={}\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n", dir);
        assert_eq!(go(&name, &[("PATH", &dir)], ["PATH", NONE, NONE], &want), 0);
    }

    // (8) error 1, and the failed handle is consumed (the wait returns the same error as Err): 11
    assert_eq!(bad(&child, &["PRUV_ENV_A"], &[]), 11); // 1 name, 0 values
    assert_eq!(bad(&child, &[], &["v"]), 11); // 0 names, 1 value
    assert_eq!(bad(&child, &["PRUV_ENV_A", "PRUV_ENV_B"], &["v"]), 11); // 2 vs 1
    assert_eq!(bad(&child, &[""], &["v"]), 11); // empty name
    assert_eq!(bad(&child, &["PRUV_ENV_A", ""], &["v", "w"]), 11); // empty name in the second pair
    assert_eq!(bad(&child, &["A=B"], &["v"]), 11); // '=' in a name
    assert_eq!(bad(&child, &["=A"], &["v"]), 11);
    assert_eq!(bad(&child, &["A\0B"], &["v"]), 11); // NUL in a name
    assert_eq!(bad(&child, &["PRUV_ENV_A"], &["v\0w"]), 11); // NUL in a value
    assert_eq!(bad(&child, &["PRUV_ENV_A", "PRUV_ENV_B"], &["v", "w\0"]), 11);
    // a program error stays error 1 whatever the pairs
    assert_eq!(bad("", &["PRUV_ENV_A"], &["v"]), 11);
    // good pairs: no error, wait gives Ok; a missing program is error 2
    assert_eq!(bad(&child, &["PRUV_ENV_A"], &["v"]), 9);
    assert_eq!(bad(&child, &[], &[]), 9);
    assert_eq!(bad("/no/such/dir/pruv-missing-program", &["PRUV_ENV_A"], &["v"]), 22);

    // after the failures a good spawn still works
    assert_eq!(
        go(&child, &[("PRUV_ENV_NEW", "again")], ["PRUV_ENV_NEW", NONE, NONE], "PRUV_ENV_NEW=again\nPRUV_ENV_NONE!\nPRUV_ENV_NONE!\n"),
        0
    );

    // (9) the parent's own environment is unchanged by all of it
    assert_eq!(std::env::var("PRUV_ENV_KEEP").as_deref(), Ok("parent"));
    assert_eq!(std::env::var("PRUV_ENV_OVER").as_deref(), Ok("old"));
    for k in ["PRUV_ENV_NEW", "PRUV_ENV_DUP", "PRUV_ENV_EMPTY", "PRUV_ENV_U1", "PRUV_ENV_U2", "PRUV_ENV_U3", "PRUV_ENV_A", "PRUV_ENV_B"] {
        assert!(std::env::var_os(k).is_none(), "parent gained {}", k);
    }
}

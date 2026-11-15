fn line(p: &std::path::Path) -> Vec<u8> {
    // the child prints its working directory as the OS reports it (a resolved path)
    let mut v = std::fs::canonicalize(p).unwrap().to_string_lossy().as_bytes().to_vec();
    v.push(b'\n');
    v
}

fn main() {
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    let start = std::env::current_dir().unwrap();
    std::fs::create_dir("sub dir").unwrap();

    // inherited: cwd == ""
    assert_eq!(sol::child_cwd(&child, "", &line(&start)), 0);
    // explicit, relative to the parent's directory, with a space in the name
    assert_eq!(sol::child_cwd(&child, "sub dir", &line(&start.join("sub dir"))), 0);
    // explicit, absolute
    let abs = start.join("sub dir");
    assert_eq!(sol::child_cwd(&child, abs.to_str().unwrap(), &line(&abs)), 0);
    // and the inherited one again: the earlier explicit cwds changed nothing
    assert_eq!(sol::child_cwd(&child, "", &line(&start)), 0);

    // the parent's own working directory never changed
    assert_eq!(std::env::current_dir().unwrap(), start);
}

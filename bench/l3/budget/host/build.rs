// Compiles the host's Pruv decisions (bud_delay.pruv: the timer delay rule, D75) with the Rust Pruv compiler into
// OUT_DIR; src/lib.rs includes the generated module. PRUVC overrides the compiler path (default: the repo's
// pruvc-rs release build; a .py path runs under PRUVC_PYTHON). The compiler also verifies the module.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let budget = here.parent().unwrap().to_path_buf();
    let src = budget.join("bud_delay.pruv");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bud_delay.rs");
    let pruvc = match std::env::var("PRUVC") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => budget.join("../../../pruvc-rs/target/release/pruvc"),
    };
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed=PRUVC");
    println!("cargo:rerun-if-env-changed=PRUVC_PYTHON");
    // A .py compiler (pruvc/pruvc.py) runs under PRUVC_PYTHON (default python3) and must emit Rust.
    let mut cmd = if pruvc.extension().map_or(false, |e| e == "py") {
        let mut c = Command::new(std::env::var("PRUVC_PYTHON").unwrap_or_else(|_| "python3".into()));
        c.arg(&pruvc);
        c
    } else {
        Command::new(&pruvc)
    };
    cmd.arg(&src).arg("-o").arg(&out);
    if pruvc.extension().map_or(false, |e| e == "py") {
        cmd.args(["--backend", "rust"]);
    }
    let st = cmd.status()
        .unwrap_or_else(|e| panic!("budget_host: cannot run pruvc {}: {}", pruvc.display(), e));
    if !st.success() {
        panic!("budget_host: pruvc failed on {} (compiler {})", src.display(), pruvc.display());
    }
}

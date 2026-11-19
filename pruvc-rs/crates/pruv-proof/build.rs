//! PR1: build-time code identity for the module proof records (`--module-cache`), used where the running
//! executable cannot be read race-free (every target except Linux, which hashes /proc/self/exe instead).
//! PRUV_BUILD_ID = sha256 of: every file under pruvc-rs/crates, pruvc-rs/{Cargo.toml,Cargo.lock,rust-toolchain.toml},
//! stdlib/, pruvc/runtime.c, the `rustc -vV` output and the target/profile/flag variables. Any change in these reruns
//! this script (rerun-if-changed on each path, directories are scanned recursively by cargo) and rebuilds the crate.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn walk(root: &Path, rel: &Path, h: &mut Sha256) {
    let p = root.join(rel);
    let md = fs::symlink_metadata(&p).unwrap_or_else(|e| panic!("stat {}: {e}", p.display()));
    if md.is_dir() {
        let mut names: Vec<_> = fs::read_dir(&p).expect("read_dir").map(|e| e.expect("dir entry").file_name()).collect();
        names.sort();
        for n in names {
            walk(root, &rel.join(n), h);
        }
    } else {
        let bytes = if md.file_type().is_symlink() {
            fs::read_link(&p).expect("read_link").to_string_lossy().into_owned().into_bytes()
        } else {
            fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
        };
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0u8]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ws = manifest.join("../..");
    let repo = ws.join("..");
    let mut h = Sha256::new();
    let inputs: [(&Path, &str); 6] = [
        (&ws, "crates"),
        (&ws, "Cargo.toml"),
        (&ws, "Cargo.lock"),
        (&ws, "rust-toolchain.toml"),
        (&repo, "stdlib"),
        (&repo, "pruvc/runtime.c"),
    ];
    for (base, rel) in inputs {
        println!("cargo:rerun-if-changed={}", base.join(rel).display());
        h.update(b"path\0");
        walk(base, Path::new(rel), &mut h);
    }
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = std::process::Command::new(&rustc).arg("-vV").output().expect("rustc -vV");
    assert!(out.status.success(), "rustc -vV failed");
    h.update(b"rustc\0");
    h.update(&out.stdout);
    for v in ["TARGET", "HOST", "PROFILE", "OPT_LEVEL", "DEBUG", "CARGO_ENCODED_RUSTFLAGS", "CARGO_CFG_TARGET_FEATURE"] {
        println!("cargo:rerun-if-env-changed={v}");
        h.update(v.as_bytes());
        h.update([0u8]);
        h.update(env::var(v).unwrap_or_default().as_bytes());
        h.update([0u8]);
    }
    let id: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=PRUV_BUILD_ID={id}");
}

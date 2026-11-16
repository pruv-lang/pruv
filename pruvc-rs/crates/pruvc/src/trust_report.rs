//! G4b-2 (docs/G4B_SPEC.md section 6): the `--trust-report PATH` JSON. Apart from the `compiler` object the bytes are the
//! same as the Python compiler's. The trust data comes from the Rust-source generator (`codegen::gen_rust_trust_data`);
//! this module adds packages, versions, source hashes and flags, and prints with the foreign manifest's JSON printer.

use crate::cli::Args;
use pruv_core::codegen::TrustData;
use pruv_core::foreign_codegen::{dump_json, J};
use pruv_core::manifest::Graph;
use pruv_core::modules::G4;
use sha2::{Digest, Sha256};
use std::path::Path;

/// One package of the report before it is printed.
struct PkgOut {
    /// "" for the program package of a program without a manifest
    name: String,
    version: String,
    display: String,
    /// the key used by the trust data: None = the program package, Some(name) = a dependency
    key: Option<String>,
    /// (display path, raw bytes) in module load order
    files: Vec<(String, Vec<u8>)>,
}

fn read_bytes(p: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| pruv_core::pyio::io_error_message(p, &e))
}

/// `<display path>\0<byte length>\0<bytes>` of every file, SHA-256, lowercase hex.
fn hash_files(files: &[(String, Vec<u8>)]) -> String {
    let mut h = Sha256::new();
    for (disp, bytes) in files {
        h.update(disp.as_bytes());
        h.update([0u8]);
        h.update(bytes.len().to_string().as_bytes());
        h.update([0u8]);
        h.update(bytes);
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

fn strs(xs: &[String]) -> J {
    J::A(xs.iter().map(|x| J::s(x)).collect())
}

/// Builds the report text (with the final newline). `Err` is an internal-error message (a source file cannot be read again).
pub fn build(a: &Args, g4: Option<&G4>, graph: Option<&Graph>, data: &TrustData) -> Result<String, String> {
    let basename: String = Path::new(&a.input)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    // ---- the packages (program first, the rest sorted by name) with their files
    let mut pkgs: Vec<PkgOut> = Vec::new();
    match graph {
        Some(g) => {
            let p0 = g.program();
            pkgs.push(PkgOut {
                name: p0.name.clone(),
                version: p0.version.clone(),
                display: String::new(),
                key: None,
                files: Vec::new(),
            });
            let mut deps: Vec<PkgOut> = g
                .pkgs
                .iter()
                .skip(1)
                .map(|p| PkgOut {
                    name: p.name.clone(),
                    version: p.version.clone(),
                    display: format!("@pkg/{}", p.name),
                    key: Some(p.name.clone()),
                    files: Vec::new(),
                })
                .collect();
            deps.sort_by(|x, y| x.name.cmp(&y.name));
            pkgs.extend(deps);
        }
        None => pkgs.push(PkgOut {
            name: String::new(),
            version: String::new(),
            display: String::new(),
            key: None,
            files: Vec::new(),
        }),
    }
    match g4 {
        Some(x) => {
            for m in &x.modules {
                // the embedded `@std/std.pruv` has no path; it belongs to the compiler (its version), not to a package hash
                let bytes = match &m.src_path {
                    Some(p) => read_bytes(p)?,
                    None => continue,
                };
                if let Some(p) = pkgs.iter_mut().find(|p| p.key == m.pkg) {
                    p.files.push((m.file.clone(), bytes));
                }
            }
        }
        None => {
            // a legacy single-file program: the input file under its basename
            let bytes = read_bytes(Path::new(&a.input))?;
            pkgs[0].files.push((basename.clone(), bytes));
        }
    }

    // ---- JSON
    let mut plist: Vec<J> = Vec::new();
    for p in &pkgs {
        let mut tr: Vec<(String, i64, String, bool)> = data
            .trusted
            .iter()
            .filter(|t| t.pkg == p.key)
            .map(|t| (t.file.clone().unwrap_or_else(|| basename.clone()), t.line, t.item.clone(), t.reachable))
            .collect();
        tr.sort_by(|x, y| (&x.0, x.1, &x.2).cmp(&(&y.0, y.1, &y.2)));
        let trusted: Vec<J> = tr
            .into_iter()
            .map(|(file, line, item, reachable)| {
                J::O(vec![
                    ("kind".to_string(), J::s("trusted_rust_fn")),
                    ("item".to_string(), J::Str(item)),
                    ("file".to_string(), J::Str(file)),
                    ("line".to_string(), J::I(line)),
                    ("reachable".to_string(), J::B(reachable)),
                ])
            })
            .collect();
        let key = p.key.clone().unwrap_or_default();
        let catalog: Vec<J> = match data.catalog.get(&key) {
            Some(set) => set
                .iter()
                .map(|(op, targs)| J::O(vec![("op".to_string(), J::s(op)), ("type_args".to_string(), strs(targs))]))
                .collect(),
            None => Vec::new(),
        };
        plist.push(J::O(vec![
            ("name".to_string(), J::s(&p.name)),
            ("version".to_string(), J::s(&p.version)),
            ("display".to_string(), J::s(&p.display)),
            ("sha256".to_string(), J::Str(hash_files(&p.files))),
            ("trusted".to_string(), J::A(trusted)),
            ("catalog".to_string(), J::A(catalog)),
        ]));
    }

    let raw_extern: Vec<String> = a.rust_extern_specs.iter().map(|(n, p)| format!("{}={}", n, p)).collect();
    let doc = J::O(vec![
        ("format".to_string(), J::I(1)),
        (
            "compiler".to_string(),
            J::O(vec![
                ("impl".to_string(), J::s("rs")),
                ("version".to_string(), J::s(env!("CARGO_PKG_VERSION"))),
                ("solver".to_string(), J::Str(pruv_proof::z3_full_version())),
            ]),
        ),
        (
            "flags".to_string(),
            J::O(vec![
                ("verify".to_string(), J::B(a.verify)),
                ("trust_callers".to_string(), J::B(a.trust_callers)),
                ("foreign_checks".to_string(), J::s(&a.foreign_checks)),
                ("infer".to_string(), J::B(a.infer)),
                ("rust_extern".to_string(), strs(&raw_extern)),
                ("rust_search".to_string(), strs(&a.rust_search)),
                ("proof_cache".to_string(), J::B(false)),
            ]),
        ),
        ("packages".to_string(), J::A(plist)),
        (
            "proof_cache".to_string(),
            J::O(vec![("hits".to_string(), J::I(0)), ("stores".to_string(), J::I(0))]),
        ),
    ]);
    let mut out = String::new();
    dump_json(&doc, 0, &mut out);
    out.push('\n');
    Ok(out)
}

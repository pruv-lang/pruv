//! G4 A0: program files, name resolution and visibility (counterpart of pruvc/modules.py; docs/G4_SPEC.md sections 2, 3, 5, 8, 10).
//!
//! `load_program(path)`  the file-taking entry point. A root with neither `mod` nor a top-level `pub` is returned exactly as
//!                       the parser produced it (legacy mode, `g4` is None). Otherwise the explicit visibility mode is active
//!                       for the whole program: the declared child files are loaded (root first, depth first, declaration
//!                       order) and `Program.functions` / `types` hold the items of all modules in that order.
//! `resolve(prog, g4)`   explicit mode only (called by check::check_explicit): validates declaration names, replaces every
//!                       lookup key by the item's resolved identity `crate::a::b::name` (root items `crate::name`), checks
//!                       access (private_item), public interfaces (public_interface) and the root main, and sets the
//!                       host-entry classification.
//!
//! Python tags every node with its `file`; here the source file of a node is the tag in its line number (diagnostic.rs),
//! so a diagnostic carries `file` without any change of the AST. `Function.pub` / `src_name` / `mod_path` / `host_entry`
//! are the parallel vectors `G4.fn_meta` / `ty_meta` (same indices as `Program.functions` / `types`).

use crate::ast::*;
use crate::diagnostic::{set_files, tagged_line, PruvError};
use crate::foreign::is_catalog;
use crate::gtype::{decl_owner, dependencies, fn_owner, GType};
use crate::pyio::{io_error_message, utf8_error_message};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

type Res<T> = Result<T, PruvError>;

/// vparser / check names that a function may not carry (check.py BUILTIN_FNS).
const BUILTIN_FNS: [&str; 32] = [
    "len", "alloc", "copy", "print", "is_char_boundary", "substr", "truncate", "band", "bor", "bxor", "wrapping_shl",
    "ushr", "wrapping_add", "wrapping_mul", "load_be", "store_be", "load_le", "store_le", "utf8_valid", "utf8_valid_span", "str_from_utf8",
    "eq_range", // L3-10: eq_range
    "find_byte", // LB6 MC
    "resize",   // P05G
    "subview",
    "str_into_bytes",
    "str_bytes",
    "copy_range", // L-copy-range
    "copy_within",
    "arr_reserve",
    "arr_push",
    "arr_append",
];

/// Display form of a module path: crate, crate::a, crate::a::b.
pub fn disp(path: &[String]) -> String {
    disp_pkg(path, None)
}

/// `disp` of a module of package `pkg` (G4b-1; a dependency package: @json, @json::a; None = the program package).
pub fn disp_pkg(path: &[String], pkg: Option<&str>) -> String {
    let mut s = match pkg {
        None => "crate".to_string(),
        Some(p) => format!("@{}", p),
    };
    for p in path {
        s.push_str("::");
        s.push_str(p);
    }
    s
}

/// The identity of an item: crate::a::b::name (root items crate::name).
pub fn ident(path: &[String], name: &str) -> String {
    ident_in(path, name, None)
}

/// The identity of an item of package `pkg`: @pkg::a::name (None: the program package, crate::a::name).
pub fn ident_in(path: &[String], name: &str, pkg: Option<&str>) -> String {
    format!("{}::{}", disp_pkg(path, pkg), name)
}

/// G3 key token of a declaration identity (Python `mono.ident_key`): crate::a::x -> encode_name(a, x);
/// @pkg::a::x -> "p" + encode_name(pkg, a, x).
pub fn ident_key(name: &str) -> String {
    match name.strip_prefix('@') {
        Some(rest) => {
            let comps: Vec<String> = rest.split("::").map(|s| s.to_string()).collect();
            format!("p{}", encode_name(&comps))
        }
        None => encode_name(&identity_components(name)),
    }
}

/// G4 A2 (docs/G4_SPEC.md 6; Python `encode_name`): the count of the components, then for each one `_<ASCII byte length>_<component>`.
pub fn encode_name(components: &[String]) -> String {
    let mut s = components.len().to_string();
    for c in components {
        s.push_str(&format!("_{}_{}", c.len(), c));
    }
    s
}

/// Rust stem of an item in explicit mode (Python `encode_stem`): kind "f" (function) or "t" (nominal type); components = the
/// module path followed by the item's source name.
pub fn encode_stem(kind: &str, components: &[String]) -> String {
    format!("pruv_m_{}_{}", kind, encode_name(components))
}

/// "crate::a::b::x" -> ["a", "b", "x"] (Python `identity_components`)
pub fn identity_components(identity: &str) -> Vec<String> {
    identity.split("::").skip(1).map(|s| s.to_string()).collect()
}

/// A type whose nominal names (identities) are replaced by their `encode_name` (Python `safe_type`), so that the G3 stem and
/// argument encodings only ever see [0-9A-Za-z_] names; injective like the identity.
pub fn safe_type(ty: &GType) -> GType {
    safe_type_cow(ty).into_owned()
}

/// `safe_type` that borrows a type it leaves unchanged (legacy types and builtin-only trees are never copied)
pub fn safe_type_cow(ty: &GType) -> std::borrow::Cow<'_, GType> {
    use std::borrow::Cow;
    if ty.owner.is_some() {
        return Cow::Borrowed(ty);
    }
    let args: Vec<Cow<'_, GType>> = ty.args.iter().map(safe_type_cow).collect();
    if crate::typeinfo::is_builtin_type_name(&ty.name) || !(ty.name.starts_with("crate::") || ty.name.starts_with('@')) {
        if args.iter().all(|a| matches!(a, Cow::Borrowed(_))) {
            return Cow::Borrowed(ty);
        }
        return Cow::Owned(GType::new(&ty.name, args.into_iter().map(Cow::into_owned).collect()));
    }
    // a dependency package's identity: "p" + encode_name(pkg, a, x) (G4b-1)
    Cow::Owned(GType::new(&ident_key(&ty.name), args.into_iter().map(Cow::into_owned).collect()))
}

#[derive(Debug, Clone)]
pub struct Module {
    /// module names from the root; empty is the root
    pub path: Vec<String>,
    /// path relative to the root directory, `/` separators
    pub file: String,
    /// G4b-2: the path the loader opened (absolute for the root); None for the embedded `@std/std.pruv`
    pub src_path: Option<PathBuf>,
    pub parent: Option<usize>,
    /// G4b-1: package name of a dependency module, None = the program package
    pub pkg: Option<String>,
    /// manifest mode: index of this module's package in the manifest Graph
    pub pkginfo: Option<usize>,
    /// ModDecl list in source order (duplicates included)
    pub mods: Vec<ModDecl>,
    /// name -> loaded child (a duplicate declaration is not loaded again)
    pub children: Vec<(String, usize)>,
    /// the items of this module in source order (indices into Program.functions / types)
    pub items: Vec<Item>,
    aliases: Vec<AliasItem>,
    alias_positions: Vec<crate::parser::AliasPosition>,
    consts: HashMap<String, ConstItem>,
    /// source name -> index into Program.functions (filled by resolve)
    pub funcs: HashMap<String, usize>,
    /// source name -> index into Program.types (filled by resolve)
    pub types: HashMap<String, usize>,
    /// PR1: lowercase hex SHA-256 of the module's source text (module proof record key)
    pub digest: String,
    /// PR1: lowercase hex SHA-256 of the module's items right after resolution (`module_edges`)
    pub rdigest: String,
    /// PR1: sorted indices of the other modules whose items an item of this module mentions (filled by resolve)
    pub deps: Vec<usize>,
}

impl Module {
    fn new(path: Vec<String>, file: String, parent: Option<usize>) -> Module {
        Module {
            path,
            file,
            src_path: None,
            parent,
            pkg: None,
            pkginfo: None,
            mods: Vec::new(),
            children: Vec::new(),
            items: Vec::new(),
            aliases: Vec::new(),
            alias_positions: vec![],
            consts: HashMap::new(),
            funcs: HashMap::new(),
            types: HashMap::new(),
            digest: String::new(),
            rdigest: String::new(),
            deps: Vec::new(),
        }
    }

    fn child(&self, name: &str) -> Option<usize> {
        self.children.iter().find(|(n, _)| n == name).map(|(_, i)| *i)
    }
}

/// The explicit-mode attributes of one function / type declaration.
#[derive(Debug, Clone)]
pub struct ItemMeta {
    /// index into G4.modules of the defining module
    pub module: usize,
    /// `pub`
    pub is_pub: bool,
    /// the name as written (Function.name / the declaration name then hold the resolved identity)
    pub src_name: String,
    /// root `pub` nongeneric ordinary function (set by resolve)
    pub host_entry: bool,
    /// G4b-1: package name of a dependency item, None = the program package
    pub pkg: Option<String>,
}

/// Explicit-mode data of a program (Python `Program.explicit` / `modules` / `items` / `entries` / `main`).
#[derive(Debug, Clone)]
pub struct G4 {
    /// root-first depth-first order
    pub modules: Vec<Module>,
    /// all items in program order
    pub items: Vec<Item>,
    pub fn_meta: Vec<ItemMeta>,
    pub ty_meta: Vec<ItemMeta>,
    /// identities of the host entries (after resolve)
    pub entries: Vec<String>,
    /// identity of the root main (after resolve)
    pub main: Option<String>,
    pub resolved: bool,
}


impl G4 {
    /// Keep item indices aligned with the checked program after predicate expansion.
    /// `opaque`: also remove opaque preds (R-6o-5: they stay through verification and go in check::opaque_inline).
    pub(crate) fn remove_preds(&mut self, functions: &[Function], opaque: bool) {
        let gone = |f: &Function| f.pred && (opaque || !f.opaque);
        if !functions.iter().any(gone) {
            return;
        }
        let mut next = 0;
        let indices: Vec<_> = functions.iter().map(|f| {
            if gone(f) {
                None
            } else {
                let index = next;
                next += 1;
                Some(index)
            }
        }).collect();
        let remap = |items: &mut Vec<Item>| {
            items.retain_mut(|item| match item {
                Item::Ty(_) => true,
                Item::Fn(i) => match indices[*i] {
                    Some(index) => {
                        *i = index;
                        true
                    }
                    None => false,
                },
            });
        };
        remap(&mut self.items);
        for module in &mut self.modules {
            remap(&mut module.items);
            module.funcs.retain(|_, index| {
                if let Some(mapped) = indices[*index] {
                    *index = mapped;
                    true
                } else {
                    false
                }
            });
        }
        let removed: HashSet<_> = functions.iter().filter(|f| gone(f)).map(|f| f.name.as_str()).collect();
        self.entries.retain(|name| !removed.contains(name.as_str()));
        let mut functions = functions.iter();
        self.fn_meta.retain(|_| !gone(functions.next().unwrap()));
    }
}

pub struct Loaded {
    pub prog: Program,
    /// Some in explicit mode
    pub g4: Option<G4>,
}

pub enum LoadError {
    /// the root file cannot be read (Python OSError message)
    Io(String),
    /// the root file is not UTF-8 (Python UnicodeDecodeError message)
    Decode(String),
    /// a language error (parse / module_file / module_alias)
    Lang(PruvError),
}

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

/// the identity of a file (device, inode), computed once per loaded file; None when the file cannot be inspected
#[cfg(unix)]
fn file_id(a: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(a).ok().map(|x| format!("{}:{}", x.dev(), x.ino()))
}

#[cfg(not(unix))]
fn file_id(a: &Path) -> Option<String> {
    a.canonicalize().ok().map(|x| x.to_string_lossy().into_owned())
}

struct Loader<'g> {
    modules: Vec<Module>,
    parsed: Vec<crate::parser::ParsedFile>,
    /// file identity -> display path of every loaded file
    loaded: HashMap<String, String>,
    /// `--lib` directories in flag order (G4b-0); only consulted for `mod NAME;` declared in the root
    libs: Vec<PathBuf>,
    /// per module (same index as `modules`): the directory its children are resolved in (the root directory for the
    /// root tree) and the display prefix
    /// ("" for the root tree, "@lib/" for a library module and its children, "@pkg/NAME/sub/" for a dependency package)
    origin: Vec<(PathBuf, String)>,
    /// G4b-1: the package graph of `--manifest`; packages are mounted by `mod KEY;` in package roots
    graph: Option<&'g crate::manifest::Graph>,
    /// canonical dependency dir -> index of its root module (one per package, a diamond reuses it)
    pkg_mods: HashMap<PathBuf, usize>,
}

impl<'g> Loader<'g> {
    /// A `pkg` diagnostic at line 1, col 1 reported for `display` (a manifest, not a loaded module file).
    fn pkg_err(&self, display: &str, code: &str, message: String, fix: String) -> PruvError {
        self.publish(Some(display));
        PruvError::new("pkg", tagged_line(1, self.modules.len() + 1), 1, code, message, fix)
    }

    /// Mounts dependency package `di` as child `md.name` of the package root `mi` (its root module is built once).
    fn mount(&mut self, mi: usize, md: &ModDecl, di: usize) -> Res<()> {
        let graph: &'g crate::manifest::Graph = match self.graph {
            Some(g) => g,
            None => return Ok(()),
        };
        let dpkg = &graph.pkgs[di];
        if let Some(&done) = self.pkg_mods.get(&dpkg.dir) {
            self.modules[mi].children.push((md.name.clone(), done));
            return Ok(());
        }
        let cpath = vec![md.name.clone()];
        let sub: &str = match dpkg.root.rfind('/') {
            Some(i) => &dpkg.root[..i],
            None => "",
        };
        let rel = format!("@pkg/{}/{}", dpkg.name, dpkg.root);
        let fpath = crate::manifest::join_rel(&dpkg.dir, &dpkg.root);
        let text = match std::fs::read(&fpath) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(_) => {
                    return Err(PruvError::new(
                        "parse", md.line, md.col, "module_file",
                        format!("the file of module {} is not valid UTF-8", disp(&cpath)),
                        format!("save {} as UTF-8", rel),
                    ));
                }
            },
            Err(_) => {
                return Err(PruvError::new(
                    "parse", md.line, md.col, "module_file",
                    format!("cannot read the file of module {} (expected {})", disp(&cpath), rel),
                    format!("restore the root file of package {}", dpkg.name),
                ));
            }
        };
        let fid = file_id(&fpath);
        if let Some(orel) = fid.as_ref().and_then(|k| self.loaded.get(k)) {
            return Err(PruvError::new(
                "parse", md.line, md.col, "module_alias",
                format!("the file {} of module {} was already loaded as {}", rel, disp(&cpath), orel),
                "every module needs its own file; remove the alias",
            ));
        }
        if let Some(k) = fid {
            self.loaded.insert(k, rel.clone());
        }
        let tag = self.modules.len() + 1;
        let parsed = match crate::parser::parse_with_shapes(&text, tag, None, true) {
            Ok(p) => p,
            Err(e) => {
                self.publish(Some(&rel)); // the error carries the tag of this file
                return Err(e);
            }
        };
        let ci = self.modules.len();
        let mut child = Module::new(Vec::new(), rel, Some(mi));
        child.digest = sha256_hex(text.as_bytes());
        child.src_path = Some(fpath.clone());
        child.pkg = Some(dpkg.name.clone());
        child.pkginfo = Some(di);
        child.mods = parsed.mods.clone();
        let base = if sub.is_empty() { dpkg.dir.clone() } else { crate::manifest::join_rel(&dpkg.dir, sub) };
        let relpre = if sub.is_empty() { format!("@pkg/{}/", dpkg.name) } else { format!("@pkg/{}/{}/", dpkg.name, sub) };
        self.pkg_mods.insert(dpkg.dir.clone(), ci);
        self.modules[mi].children.push((md.name.clone(), ci));
        self.modules.push(child);
        self.origin.push((base, relpre));
        self.parsed.push(parsed);
        self.publish(None);
        self.visit(ci)
    }

    /// Registers the file names of the modules loaded so far (diagnostic line tags), plus one pending file.
    fn publish(&self, extra: Option<&str>) {
        let mut v: Vec<String> = self.modules.iter().map(|m| m.file.clone()).collect();
        if let Some(x) = extra {
            v.push(x.to_string());
        }
        set_files(v);
    }

    fn visit(&mut self, mi: usize) -> Res<()> {
        let mut seen: HashSet<String> = HashSet::new();
        let mods = self.modules[mi].mods.clone();
        for md in &mods {
            if !seen.insert(md.name.clone()) {
                continue; // reported by resolve as type/duplicate_module; the file is loaded once
            }
            let mut cpath = self.modules[mi].path.clone();
            cpath.push(md.name.clone());
            let (base, prefix) = self.origin[mi].clone();
            let mut origin = (base.clone(), prefix.clone());
            let mut rel = format!("{}{}.pruv", prefix, cpath.join("/"));
            let mut fp = base;
            for c in &cpath {
                fp.push(c);
            }
            let mut fpath = PathBuf::from(format!("{}.pruv", fp.to_string_lossy()));
            // G4b-1: in a package root `mod NAME;` may mount a dependency package (the program root is the root of the
            // program's own package)
            if let (Some(graph), true) = (self.graph, self.modules[mi].path.is_empty()) {
                let pi = self.modules[mi].pkginfo.unwrap_or(0);
                if md.name == "std" {
                    if let Some(p) = self.modules[mi].pkg.clone() {
                        return Err(PruvError::new(
                            "pkg", md.line, md.col, "std_in_dependency",
                            format!("`mod std;` is only available in the program root, not in package {}", p),
                            "remove `mod std;`",
                        ));
                    }
                }
                if let Some((_, ddir, _)) = graph.pkgs[pi].deps.iter().find(|d| d.0 == md.name) {
                    let di = graph.by_dir[ddir];
                    if std::fs::symlink_metadata(&fpath).is_ok() {
                        return Err(PruvError::new(
                            "pkg", md.line, md.col, "mount_conflict",
                            format!("module {} is found in more than one place: {}, @pkg/{}", md.name, rel, graph.pkgs[di].name),
                            "remove the local file or rename the dependency key",
                        ));
                    }
                    self.mount(mi, md, di)?;
                    continue;
                }
            }
            // G4b-0: a root declaration may also be found in a library directory (never std, never below the root)
            if mi == 0 && md.name != "std" && !self.libs.is_empty() && self.graph.is_none() {
                let mut found: Vec<(PathBuf, String, Option<usize>)> = Vec::new();
                if std::fs::symlink_metadata(&fpath).is_ok() {
                    found.push((fpath.clone(), rel.clone(), None));
                }
                for (k, d) in self.libs.iter().enumerate() {
                    let p = d.join(format!("{}.pruv", md.name));
                    if std::fs::symlink_metadata(&p).is_ok() {
                        found.push((p, format!("@lib/{}.pruv", md.name), Some(k)));
                    }
                }
                if found.len() > 1 {
                    let places: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
                    return Err(PruvError::new(
                        "parse", md.line, md.col, "lib_module_conflict",
                        format!("module {} is found in more than one place: {}", md.name, places.join(", ")),
                        "remove the duplicate file or drop one --lib",
                    ));
                }
                if let Some((p, r, Some(k))) = found.pop() {
                    fpath = p;
                    rel = r;
                    origin = (self.libs[k].clone(), "@lib/".to_string());
                }
            }
            let text = if md.name == "std" {
                if !self.modules[mi].path.is_empty() {
                    return Err(PruvError::new(
                        "parse", md.line, md.col, "std_import_location",
                        "std may only be imported at the program root",
                        "move `mod std;` to the root and call `crate::std::name`",
                    ));
                }
                // Like lexists: directories and dangling symlinks also conflict.
                if std::fs::symlink_metadata(&fpath).is_ok() {
                    return Err(PruvError::new(
                        "parse", md.line, md.col, "std_module_conflict",
                        "local std.pruv conflicts with the bundled std module",
                        "rename the user module/file and its paths",
                    ));
                }
                rel = "@std/std.pruv".to_string();
                // Embed bytes so invalid UTF-8 retains the import-site diagnostic.
                std::str::from_utf8(include_bytes!("../../../../stdlib/std.pruv"))
                    .map_err(|_| PruvError::new(
                        "parse", md.line, md.col, "std_source",
                        "cannot read the bundled std module as UTF-8",
                        "restore/reinstall the matching compiler resource",
                    ))?.to_string()
            } else {
                match std::fs::read(&fpath) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(s) => s,
                        Err(_) => {
                            return Err(PruvError::new(
                                "parse",
                                md.line,
                                md.col,
                                "module_file",
                                format!("the file of module {} is not valid UTF-8", disp(&cpath)),
                                format!("save {} as UTF-8", rel),
                            ));
                        }
                    },
                    Err(_) => {
                        return Err(PruvError::new(
                            "parse",
                            md.line,
                            md.col,
                            "module_file",
                            format!(
                                "cannot read the file of module {} (expected {})",
                                disp_pkg(&cpath, self.modules[mi].pkg.as_deref()),
                                rel
                            ),
                            if self.graph.is_some() && self.modules[mi].path.is_empty() {
                                format!("create {} next to the root file, add {} to dependencies, or remove `mod {};`", rel, md.name, md.name)
                            } else if mi == 0 {
                                format!("create {} next to the root file, pass --lib DIR, or remove `mod {};`", rel, md.name)
                            } else {
                                format!("create {} next to the root file, or remove `mod {};`", rel, md.name)
                            },
                        ));
                    }
                }
            };
            let fid = if md.name == "std" { None } else { file_id(&fpath) };
            if let Some(orel) = fid.as_ref().and_then(|k| self.loaded.get(k)) {
                return Err(PruvError::new(
                    "parse",
                    md.line,
                    md.col,
                    "module_alias",
                    format!("the file {} of module {} was already loaded as {}", rel, disp(&cpath), orel),
                    "every module needs its own file; remove the alias",
                ));
            }
            if let Some(k) = fid {
                self.loaded.insert(k, rel.clone());
            }
            let tag = self.modules.len() + 1;
            let parsed = match crate::parser::parse_with_shapes(&text, tag, None, true) {
                Ok(p) => p,
                Err(e) => {
                    self.publish(Some(&rel)); // the error carries the tag of this file
                    return Err(e);
                }
            };
            let ci = self.modules.len();
            let mut child = Module::new(cpath, rel, Some(mi));
            child.digest = sha256_hex(text.as_bytes());
            if md.name != "std" {
                child.src_path = Some(fpath.clone());
            }
            child.pkg = self.modules[mi].pkg.clone();
            child.pkginfo = self.modules[mi].pkginfo;
            child.mods = parsed.mods.clone();
            self.modules[mi].children.push((md.name.clone(), ci));
            self.modules.push(child);
            self.origin.push(origin);
            self.parsed.push(parsed);
            self.publish(None);
            self.visit(ci)?;
        }
        if let (Some(graph), true) = (self.graph, self.modules[mi].path.is_empty()) {
            // every dependency key must be mounted by a `mod` of the package root
            let pkg = &graph.pkgs[self.modules[mi].pkginfo.unwrap_or(0)];
            for (key, _, _) in &pkg.deps {
                if !seen.contains(key) {
                    return Err(self.pkg_err(
                        &pkg.display,
                        "unused_dependency",
                        format!("dependency {} is declared but not mounted by `mod {};` in the package root", key, key),
                        format!("add `mod {};` to the package root or remove the dependency", key),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Reads the root file (same failures as Python `open(path).read()`) and loads the program.
pub fn load_program(path: &Path) -> Result<Loaded, LoadError> {
    load_program_with_libs(path, &[])
}

/// `load_program` with the ordered `--lib` directories (docs/G4B_LIB_SPEC.md section 2); an empty list is `load_program`.
pub fn load_program_with_libs(path: &Path, libs: &[PathBuf]) -> Result<Loaded, LoadError> {
    load_program_full(path, libs, None)
}

/// `load_program` of a program with a manifest (G4b-1): `mod KEY;` in a package root mounts the dependency package of
/// that key. `graph` comes from `manifest::load_graph`; `path` is the program root (the root file of its manifest).
pub fn load_program_with_manifest(path: &Path, graph: &crate::manifest::Graph) -> Result<Loaded, LoadError> {
    load_program_full(path, &[], Some(graph))
}

fn load_program_full(path: &Path, libs: &[PathBuf], graph: Option<&crate::manifest::Graph>) -> Result<Loaded, LoadError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::Io(io_error_message(path, &e)))?;
    let src = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(e) => return Err(LoadError::Decode(utf8_error_message(&bytes, &e))),
    };
    load_source_full(path, &src, libs, graph)
}

/// `load_program` for an already decoded root text (`path` still names the root: children are relative to its directory).
pub fn load_source(path: &Path, src: &str) -> Result<Loaded, LoadError> {
    load_source_with_libs(path, src, &[])
}

/// `load_source` with the ordered `--lib` directories.
pub fn load_source_with_libs(path: &Path, src: &str, libs: &[PathBuf]) -> Result<Loaded, LoadError> {
    load_source_full(path, src, libs, None)
}

fn load_source_full(
    path: &Path,
    src: &str,
    libs: &[PathBuf],
    graph: Option<&crate::manifest::Graph>,
) -> Result<Loaded, LoadError> {
    let root_file = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let first = match crate::parser::parse_with_shapes(src, 0, None, true) {
        Ok(f) => f,
        Err(mut e) => {
            set_files(Vec::new());
            if e.stage != "internal" && crate::parser::sniff_explicit(src) {
                // a root that uses module syntax: its error carries `file`
                set_files(vec![root_file]);
                e.line = tagged_line(e.line, 1);
            }
            return Err(LoadError::Lang(e));
        }
    };
    if first.mods.is_empty() && !first.has_pub {
        if let Some(g) = graph {
            if let Some((key, _, _)) = g.program().deps.first() {
                // a legacy root mounts nothing: the first declared dependency is unused
                return Err(LoadError::Lang(
                    crate::manifest::PkgError {
                        err: PruvError::new(
                            "pkg", 1, 1, "unused_dependency",
                            format!("dependency {} is declared but not mounted by `mod {};` in the package root", key, key),
                            format!("add `mod {};` to the package root or remove the dependency", key),
                        ),
                        file: g.program().display.clone(),
                    }
                    .into_pruv(),
                ));
            }
        }
        set_files(Vec::new());
        let mut prog = first.prog;
        prog.raw_entries = graph.and_then(|g| g.program().raw_entries.clone()); // P06: applied by codegen in P09
        prog.pkg_name = graph.map(|g| g.program().name.clone()); // P10H: prefix of every global C symbol
        return Ok(Loaded { prog, g4: None }); // legacy mode: untouched
    }
    // explicit mode: the root is parsed again with its file tag
    let root = match crate::parser::parse_with_shapes(src, 1, None, true) {
        Ok(f) => f,
        Err(e) => {
            set_files(vec![root_file]);
            return Err(LoadError::Lang(e));
        }
    };
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    };
    let root_dir = abs.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut root_mod = Module::new(Vec::new(), root_file.clone(), None);
    root_mod.src_path = Some(abs.clone());
    root_mod.digest = sha256_hex(src.as_bytes());
    root_mod.pkginfo = graph.map(|_| 0);
    root_mod.mods = root.mods.clone();
    let mut loaded = HashMap::new();
    if let Some(k) = file_id(&abs) {
        loaded.insert(k, root_file);
    }
    // library paths are made absolute like the root (relative ones are relative to the working directory)
    let libs: Vec<PathBuf> = libs.iter().map(|d| std::path::absolute(d).unwrap_or_else(|_| d.clone())).collect();
    let mut ld = Loader {
        origin: vec![(root_dir, String::new())],
        modules: vec![root_mod],
        parsed: vec![root],
        loaded,
        libs,
        graph,
        pkg_mods: HashMap::new(),
    };
    ld.publish(None);
    ld.visit(0).map_err(LoadError::Lang)?;

    // the flat program: the items of all modules in module order
    let Loader { mut modules, mut parsed, .. } = ld;
    if parsed.iter().any(|pf| pf.deferred.is_some()) {
        for (m, pf) in modules.iter_mut().zip(&parsed) { m.aliases = pf.prog.aliases.clone(); }
        let mut headers = G4 { modules, items: vec![], fn_meta: vec![], ty_meta: vec![],
            entries: vec![], main: None, resolved: false };
        for (mi, pf) in parsed.iter().enumerate() {
            let names = match &pf.deferred {
                Some(file) => file.nominals.clone(),
                None => pf.prog.types.iter().zip(&pf.ty_pub).map(|(d, vis)| (d.name().to_string(), *vis)).collect(),
            };
            for (name, is_pub) in names {
                let index = headers.ty_meta.len();
                headers.modules[mi].types.insert(name.clone(), index);
                headers.ty_meta.push(ItemMeta { module: mi, is_pub, src_name: name,
                    host_entry: false, pkg: headers.modules[mi].pkg.clone() });
            }
        }
        let roots = headers.modules.iter().enumerate().filter(|(_, m)| m.path.is_empty())
            .map(|(i, m)| (m.pkg.clone(), i)).collect();
        let tuples = Default::default();
        for (mi, pf) in parsed.iter_mut().enumerate() {
            if let Some(file) = pf.deferred.take() {
                let resolver = Rs { cur: mi, g4: &headers, roots: &roots, tuples: &tuples,
                    old_owner: String::new(), new_owner: String::new() };
                *pf = crate::parser::parse_with_shapes(&file.src, mi + 1, Some(&|g,l,c| resolver.rtype(g,l,c)), false)
                    .map_err(LoadError::Lang)?;
            }
        }
        modules = headers.modules;
        for m in &mut modules { m.types.clear(); }
    }
    let mut prog = Program::default();
    let mut all_items: Vec<Item> = Vec::new();
    let mut fn_meta: Vec<ItemMeta> = Vec::new();
    let mut ty_meta: Vec<ItemMeta> = Vec::new();
    for (mi, pf) in parsed.into_iter().enumerate() {
        let crate::parser::ParsedFile { prog: pp, items, fn_pub, ty_pub, .. } = pf;
        // P9b-2: impl blocks and their functions by identity (`crate::m::Owner`, `crate::m::Owner__name`)
        let (mpath, mpkg) = (modules[mi].path.clone(), modules[mi].pkg.clone());
        modules[mi].aliases = pp.aliases;
        modules[mi].alias_positions = pf.positions;
        for mut c in pp.consts {
            let source = c.name.clone();
            let id = ident_in(&mpath, &source, mpkg.as_deref());
            c.name = format!("pruv_const_{}", ident_key(&id));
            modules[mi].consts.insert(source, c.clone());
            prog.consts.push(c);
        }
        for mut m in pp.methods {
            m.owner = ident_in(&mpath, &m.owner, mpkg.as_deref());
            m.func = ident_in(&mpath, &m.func, mpkg.as_deref());
            prog.methods.push(m);
        }
        for (owner, l, c, n) in pp.impls {
            prog.impls.push((ident_in(&mpath, &owner, mpkg.as_deref()), l, c, n));
        }
        let mut fns: Vec<Option<Function>> = pp.functions.into_iter().map(Some).collect();
        let mut tys: Vec<Option<TypeDecl>> = pp.types.into_iter().map(Some).collect();
        let mut mitems: Vec<Item> = Vec::new();
        for it in items {
            match it {
                Item::Fn(i) => {
                    if let Some(f) = fns[i].take() {
                        let gi = prog.functions.len();
                        fn_meta.push(ItemMeta {
                            module: mi,
                            is_pub: fn_pub[i],
                            src_name: f.name.clone(),
                            host_entry: false,
                            pkg: modules[mi].pkg.clone(),
                        });
                        prog.functions.push(f);
                        mitems.push(Item::Fn(gi));
                    }
                }
                Item::Ty(i) => {
                    if let Some(d) = tys[i].take() {
                        if crate::gtype::is_tuple_name(d.name()) { continue; }
                        let gi = prog.types.len();
                        ty_meta.push(ItemMeta {
                            module: mi,
                            is_pub: ty_pub[i],
                            src_name: d.name().to_string(),
                            host_entry: false,
                            pkg: modules[mi].pkg.clone(),
                        });
                        prog.types.push(d);
                        mitems.push(Item::Ty(gi));
                    }
                }
            }
        }
        modules[mi].items = mitems.clone();
        all_items.extend(mitems);
    }
    let g4 = G4 { modules, items: all_items, fn_meta, ty_meta, entries: Vec::new(), main: None, resolved: false };
    prog.raw_entries = graph.and_then(|g| g.program().raw_entries.clone()); // P06: applied by codegen in P09
    prog.pkg_name = graph.map(|g| g.program().name.clone()); // P10H: prefix of every global C symbol
    Ok(Loaded { prog, g4: Some(g4) })
}

// ---------------------------------------------------------------------------
// resolution
// ---------------------------------------------------------------------------

fn err(line: i64, col: i64, code: &str, msg: impl Into<String>, fix: impl Into<String>) -> PruvError {
    PruvError::new("type", line, col, code, msg, fix)
}

pub fn resolve(prog: &mut Program, g4: &mut G4) -> Res<()> {
    if g4.resolved {
        return Ok(());
    }
    g4.resolved = true;
    run(prog, g4)
}

fn tparams_of(prog: &Program, it: Item) -> Vec<TParam> {
    match it {
        Item::Fn(i) => prog.functions[i].tparams.clone(),
        Item::Ty(i) => prog.types[i].tparams().to_vec(),
    }
}

fn run(prog: &mut Program, g4: &mut G4) -> Res<()> {
    // 1. declaration names per module (functions, then types, like the legacy checker), module declarations
    for mi in 0..g4.modules.len() {
        let items = g4.modules[mi].items.clone();
        let path = g4.modules[mi].path.clone();
        let pkg = g4.modules[mi].pkg.clone();
        let pk = pkg.as_deref();
        for it in &items {
            if let Item::Fn(fi) = *it {
                let f = &prog.functions[fi];
                if g4.modules[mi].funcs.contains_key(&f.name) {
                    // P9b-2: user names cannot contain `__`, so `T__f` is the impl function `f` of `T` (R-6x-1)
                    if let Some((owner, name)) = f.name.rsplit_once("__") {
                        return Err(err(f.line, f.col, "dup_method",
                            format!("{} has two impl functions named {}", crate::pyrepr::repr_str(owner), crate::pyrepr::repr_str(name)), "rename one of them"));
                    }
                    return Err(err(f.line, f.col, "dup_fn", format!("function {} defined twice", ident_in(&path, &f.name, pk)), ""));
                }
                if BUILTIN_FNS.contains(&f.name.as_str()) || crate::check::contract_only(&f.name).is_some() || is_catalog(&f.name) {
                    return Err(err(f.line, f.col, "reserved", format!("{} is a builtin name", f.name), "rename the function"));
                }
                g4.modules[mi].funcs.insert(f.name.clone(), fi);
            }
        }
        for it in &items {
            if let Item::Ty(ti) = *it {
                let d = &prog.types[ti];
                let (name, line, col) = (d.name().to_string(), d.line(), d.col());
                if name != "subview" && !crate::check::type_name_ok(&name) {
                    return Err(err(
                        line,
                        col,
                        "type_name",
                        format!("type name {} must start with an upper-case letter", name),
                        format!("rename it, e.g. {}", crate::check::cap_first(&name, true)),
                    ));
                }
                if name == "subview" || crate::check::RESERVED_TYPE_NAMES.contains(&name.as_str()) {
                    return Err(err(line, col, "reserved", format!("{} is a builtin name", name), "rename the type"));
                }
                if g4.modules[mi].types.contains_key(&name) {
                    return Err(err(line, col, "dup_type", format!("type {} defined twice", ident_in(&path, &name, pk)), ""));
                }
                if g4.modules[mi].funcs.contains_key(&name) {
                    return Err(err(
                        line,
                        col,
                        "name_collision",
                        format!("{} is both a type and a function", ident_in(&path, &name, pk)),
                        "rename the type or the function",
                    ));
                }
                g4.modules[mi].types.insert(name, ti);
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mods = g4.modules[mi].mods.clone();
        for md in &mods {
            if seen.contains(&md.name) {
                return Err(err(
                    md.line,
                    md.col,
                    "duplicate_module",
                    format!("module {} is declared twice in {}", md.name, disp_pkg(&path, pk)),
                    format!("remove the second `mod {};`", md.name),
                ));
            }
            seen.insert(md.name.clone());
            if g4.modules[mi].funcs.contains_key(&md.name) {
                return Err(err(
                    md.line,
                    md.col,
                    "name_collision",
                    format!("{} is both a module and a function in {}", md.name, disp_pkg(&path, pk)),
                    "rename the module or the function",
                ));
            }
        }
    }
    // 1b. a type parameter may not carry the name of a type of its module (builtin names: the checker)
    let items = g4.items.clone();
    for it in &items {
        let mi = match *it {
            Item::Fn(i) => g4.fn_meta[i].module,
            Item::Ty(i) => g4.ty_meta[i].module,
        };
        for tp in tparams_of(prog, *it) {
            if g4.modules[mi].types.contains_key(&tp.name) {
                return Err(err(
                    tp.line,
                    tp.col,
                    "type_parameter",
                    format!("type parameter {} shadows a declared or built-in type", tp.name),
                    "choose a name that is not a type, e.g. T",
                ));
            }
        }
    }
    // 2. resolve every reference (items in program order); then rename the items to their identities
    let tuples = std::cell::RefCell::new(std::collections::BTreeMap::new());
    let mut public_aliases = Vec::new();
    {
        let g4r: &G4 = &*g4;
        // the root module of every package (`crate` of a module is the root of its own package)
        let mut roots: HashMap<Option<String>, usize> = HashMap::new();
        for (i, m) in g4r.modules.iter().enumerate() {
            if m.path.is_empty() {
                roots.entry(m.pkg.clone()).or_insert(i);
            }
        }
        let mut rs = Rs { g4: g4r, cur: 0, old_owner: String::new(), new_owner: String::new(), roots: &roots, tuples: &tuples };
        for (mi, md) in g4r.modules.iter().enumerate() {
            rs.cur = mi;
            for pos in &md.alias_positions {
                let (m, path) = rs.walk(&pos.ty.name);
                if path.len() != 1 || !rs.module(m).aliases.iter().any(|a| a.name == path[0]) { continue; }
                let shape = rs.rtype(&pos.ty, pos.line, pos.col)?;
                if !pos.fixed_ok && shape.to_key().contains(crate::gtype::FIXED_PREFIX) {
                    return Err(err(pos.line, pos.col, "fixed_array_position", "fixed array in an unsupported type position", ""));
                }
                if pos.generic && dependencies(&shape).iter().any(|n| crate::gtype::is_tuple_name(n)) {
                    return Err(PruvError::new("parse", pos.line, pos.col, "tuple_generic",
                        "tuple types are not available inside a generic declaration", "use a named struct"));
                }
            }
            for a in &md.aliases {
                let rhs = rs.rtype(&GType::from_key(&a.rhs), a.line, a.col)?;
                if a.is_pub { public_aliases.push((a.clone(), rhs)); }
            }
        }
        for it in &items {
            match *it {
                Item::Fn(fi) => {
                    rs.cur = g4r.fn_meta[fi].module;
                    rs.fn_(&mut prog.functions[fi], &g4r.fn_meta[fi])?;
                }
                Item::Ty(ti) => {
                    rs.cur = g4r.ty_meta[ti].module;
                    rs.tdecl(&mut prog.types[ti], &g4r.ty_meta[ti])?;
                }
            }
        }
    }
    // 3. public interfaces (G4 A1: transitive over the field / payload types of public declarations). The map is built
    // after resolution, so the field types are already identities.
    let mut type_by_id: HashMap<String, TyIface> = HashMap::new();
    for m in &g4.modules {
        for (name, ti) in &m.types {
            let fields: Vec<(String, GType)> = decl_fields(&prog.types[*ti])
                .into_iter()
                .map(|fd| (fd.name.clone(), GType::from_key(&fd.ty)))
                .collect();
            type_by_id.insert(ident_in(&m.path, name, m.pkg.as_deref()), TyIface { is_pub: g4.ty_meta[*ti].is_pub, fields });
        }
    }
    for (name, (components, line, col)) in tuples.into_inner() {
        let fields: Vec<_> = components.iter().enumerate().map(|(i, ty)| FieldDecl {
            borrowed: false, line, col, name: i.to_string(), ty: ty.to_key(),
        }).collect();
        type_by_id.insert(name.clone(), TyIface { is_pub: true,
            fields: fields.iter().zip(components).map(|(f, t)| (f.name.clone(), t)).collect() });
        let i = prog.types.len();
        prog.types.push(TypeDecl::Struct(StructDecl { line, col, name: name.clone(), fields,
            invariants: vec![], invariant_lines: vec![], tparams: vec![] }));
        g4.ty_meta.push(ItemMeta { module: 0, src_name: name, is_pub: false, host_entry: false, pkg: None });
        g4.items.push(Item::Ty(i));
    }
    // one visited set for the whole program: the first leak aborts the check, so every public declaration visited by an
    // earlier item has a closure without private declarations and need not be walked again
    let mut iface_seen: HashSet<String> = HashSet::new();
    // P06: function identity -> `pub` (a public contract may expand only public functions)
    let fn_pub: HashMap<String, bool> =
        prog.functions.iter().enumerate().map(|(i, f)| (f.name.clone(), g4.fn_meta[i].is_pub)).collect();
    for it in &items {
        match *it {
            Item::Fn(fi) => {
                if g4.fn_meta[fi].is_pub {
                    iface_fn(&type_by_id, &fn_pub, &prog.functions[fi], &mut iface_seen)?;
                }
            }
            Item::Ty(ti) => {
                if g4.ty_meta[ti].is_pub {
                    iface_ty(&type_by_id, &prog.types[ti], &mut iface_seen)?;
                }
            }
        }
    }
    for (alias, rhs) in public_aliases {
        iface_type(&type_by_id, &rhs, alias.line, alias.col, false, &alias.name, "alias definition", &mut iface_seen)?;
    }
    // 4. root main and host entries
    entries(prog, g4)?;
    module_edges(prog, g4);
    Ok(())
}

/// lowercase hex SHA-256 of `bytes`
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in d.iter() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// PR1 (Python `_Resolver.module_edges`): `Module.deps` = the other modules whose items an item of the module mentions,
/// and `Module.rdigest` = SHA-256 of the module's resolved items, both read right after resolution (before check and
/// refinement expansion rewrite calls). The `Debug` text of each item (and its `ItemMeta`) is the serialization: it is
/// deterministic (no hash map inside a pre-mono item) and holds every resolved identity as a quoted string. A quoted
/// string equal to an item identity (or a type-parameter owner `fn:<identity>` / `type:<identity>`) of another module is an
/// edge, so calls, reveals, types, constructors, variants, literals, type arguments and refinement bases all count; an
/// identity nested in a type key string (`option<crate::types::Flag>`) is found by splitting the string on the type-key
/// separators. Over-approximation is safe: an extra edge only makes a module proof record depend on more files.
fn module_edges(prog: &Program, g4: &mut G4) {
    // Debug-quoted identity -> defining module
    let mut owner: HashMap<String, usize> = HashMap::new();
    // raw identity -> defining module, for identities nested inside a larger type key string
    let mut raw_owner: HashMap<String, usize> = HashMap::new();
    for (i, f) in prog.functions.iter().enumerate() {
        let m = g4.fn_meta[i].module;
        owner.insert(format!("{:?}", f.name), m);
        owner.insert(format!("{:?}", format!("fn:{}", f.name)), m);
        raw_owner.insert(f.name.to_string(), m);
        raw_owner.insert(format!("fn:{}", f.name), m);
    }
    for (i, d) in prog.types.iter().enumerate() {
        let m = g4.ty_meta[i].module;
        owner.insert(format!("{:?}", d.name()), m);
        owner.insert(format!("{:?}", format!("type:{}", d.name())), m);
        raw_owner.insert(d.name().to_string(), m);
        raw_owner.insert(format!("type:{}", d.name()), m);
    }
    fn component_modules(key: &str, owners: &HashMap<String, usize>, out: &mut HashSet<usize>) {
        for name in dependencies(&GType::from_key(key)) {
            if let Some(keys) = crate::gtype::tuple_components(&name) {
                for key in keys { component_modules(&key, owners, out); }
            } else if let Some(owner) = owners.get(&name) { out.insert(*owner); }
        }
    }
    let mut tuple_modules = HashMap::new();
    for d in &prog.types {
        if crate::gtype::is_tuple_name(d.name()) {
            let mut deps = HashSet::new();
            component_modules(d.name(), &raw_owner, &mut deps);
            tuple_modules.insert(d.name(), deps);
            owner.remove(&format!("{:?}", d.name()));
            raw_owner.remove(d.name());
        }
    }
    for mi in 0..g4.modules.len() {
        let mut text = String::new();
        for it in &g4.modules[mi].items {
            match *it {
                Item::Fn(i) => text.push_str(&format!("F{:?}\n{:?}\n", prog.functions[i], g4.fn_meta[i])),
                Item::Ty(i) => text.push_str(&format!("T{:?}\n{:?}\n", prog.types[i], g4.ty_meta[i])),
            }
        }
        let mut deps: Vec<usize> = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'"' {
                i += 1;
                continue;
            }
            let start = i;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            let end = (i + 1).min(b.len());
            let mut hit = |j: usize| {
                if j != mi && !deps.contains(&j) {
                    deps.push(j);
                }
            };
            if let Some(&j) = owner.get(&text[start..end]) {
                hit(j);
            }
            // identities nested in a type key string (`option<crate::types::Flag>`, tuple/fn keys joined by `\u{1}`):
            // split the escaped inner text on type-key separators and look each piece up as a raw identity
            let inner = text[start + 1..end.saturating_sub(1).max(start + 1)].replace("\\u{1}", "<");
            for tok in inner.split(|c: char| matches!(c, '<' | '>' | ',' | ' ' | '(' | ')' | '[' | ']' | ';' | '&')) {
                if !tok.is_empty() {
                    if let Some(modules) = tuple_modules.get(tok) {
                        for j in modules { hit(*j); }
                    }
                    if let Some(&j) = raw_owner.get(tok) {
                        hit(j);
                    }
                }
            }
            i = end;
        }
        deps.sort_unstable();
        let m = &mut g4.modules[mi];
        m.rdigest = sha256_hex(text.as_bytes());
        m.deps = deps;
    }
}

/// resolver state (Python `_Resolver`): the module tree is read-only here, the items are rewritten in place
struct Rs<'a> {
    g4: &'a G4,
    /// the module of the item being resolved
    cur: usize,
    old_owner: String,
    new_owner: String,
    /// package (None = the program package) -> index of its root module
    roots: &'a HashMap<Option<String>, usize>,
    tuples: &'a std::cell::RefCell<std::collections::BTreeMap<String, (Vec<GType>, i64, i64)>>,
}

impl<'a> Rs<'a> {
    fn module(&self, i: usize) -> &'a Module {
        let g: &'a G4 = self.g4;
        &g.modules[i]
    }

    /// crate::a::b::x -> (innermost module reached by module segments, remaining segments)
    fn walk(&self, path: &str) -> (usize, Vec<String>) {
        let segs: Vec<&str> = path.split("::").skip(1).collect();
        // `crate` = the root of the package of the module being resolved
        let p: &Option<String> = &self.module(self.cur).pkg;
        let mut m = self.roots.get(p).copied().unwrap_or(0);
        let mut i = 0usize;
        while i + 1 < segs.len() {
            match self.module(m).child(segs[i]) {
                Some(c) => {
                    if self.module(c).pkg != self.module(m).pkg && self.module(m).pkg != *p {
                        break; // a package never sees its dependencies' dependencies
                    }
                    m = c;
                    i += 1;
                }
                None => break,
            }
        }
        (m, segs[i..].iter().map(|s| s.to_string()).collect())
    }

    fn access(&self, is_pub: bool, m: usize, line: i64, col: i64, path: &str) -> Res<()> {
        if !is_pub && m != self.cur {
            let md = self.module(m);
            return Err(err(
                line,
                col,
                "private_item",
                format!("{} is private to module {}", path, disp_pkg(&md.path, md.pkg.as_deref())),
                format!("mark it `pub` in {}, or call it through a public item of that module", md.file),
            ));
        }
        Ok(())
    }

    fn unknown_module(&self, m: usize, rem: &[String], line: i64, col: i64, path: &str) -> PruvError {
        let md = self.module(m);
        err(
            line,
            col,
            "unknown_module",
            format!("unknown module {} in {} (path {})", rem[0], disp_pkg(&md.path, md.pkg.as_deref()), path),
            format!("declare it with `mod {};` in {}, or correct the path", rem[0], md.file),
        )
    }

    fn type_item(&self, path: &str, line: i64, col: i64) -> Res<String> {
        let (m, rem) = self.walk(path);
        if rem.len() > 1 {
            return Err(self.unknown_module(m, &rem, line, col, path));
        }
        let md = self.module(m);
        let ti = match md.types.get(&rem[0]) {
            Some(t) => *t,
            None => {
                if md.child(&rem[0]).is_some() {
                    return Err(err(line, col, "unknown_type", format!("{} is a module, not a type", path), "name a struct or enum of the module"));
                }
                return Err(err(
                    line,
                    col,
                    "unknown_type",
                    format!("unknown type {}", path),
                    format!("declare it with `struct`/`enum` in {}", md.file),
                ));
            }
        };
        self.access(self.g4.ty_meta[ti].is_pub, m, line, col, path)?;
        Ok(ident_in(&md.path, &rem[0], md.pkg.as_deref()))
    }

    /// a nominal name as written -> identity (qualified: resolved; unqualified: the current module only; otherwise
    /// unchanged: builtin/catalog spelling or an unknown name for the checker)
    fn local_or_path_type(&self, name: &str, line: i64, col: i64) -> Res<String> {
        if name.starts_with("crate::") {
            return self.type_item(name, line, col);
        }
        let cur = self.module(self.cur);
        if cur.types.contains_key(name) {
            return Ok(ident_in(&cur.path, name, cur.pkg.as_deref()));
        }
        Ok(name.to_string())
    }

    fn fn_item(&self, m: usize, rem: &[String], line: i64, col: i64, path: &str) -> Res<String> {
        if rem.len() != 1 {
            return Err(self.unknown_module(m, rem, line, col, path));
        }
        let md = self.module(m);
        let fi = match md.funcs.get(&rem[0]) {
            Some(f) => *f,
            None => {
                if md.child(&rem[0]).is_some() {
                    return Err(err(line, col, "unknown_fn", format!("{} is a module, not a function", path), "name a function of the module"));
                }
                return Err(err(line, col, "unknown_fn", format!("unknown function {}", path), format!("declare it in {}", md.file)));
            }
        };
        self.access(self.g4.fn_meta[fi].is_pub, m, line, col, path)?;
        Ok(ident_in(&md.path, &rem[0], md.pkg.as_deref()))
    }

    // ---- types ----
    fn rtype(&self, ty: &GType, line: i64, col: i64) -> Res<GType> {
        self.expand(ty, line, col, &mut Vec::new())
    }

    fn expand(&self, ty: &GType, line: i64, col: i64, busy: &mut Vec<(usize, String)>) -> Res<GType> {
        if let Some(keys) = crate::gtype::tuple_components(&ty.name) {
            let comps: Vec<_> = keys.iter().map(|k| self.expand(&GType::from_key(k), line, col, busy)).collect::<Res<_>>()?;
            if comps.iter().any(|c| c == &GType::unit() || c.name == "@ref" || c.view_payload().is_some()) {
                return Err(PruvError::new("parse", line, col, "tuple_component", "a tuple component must be a value", ""));
            }
            let name = crate::gtype::tuple_name(&comps);
            self.tuples.borrow_mut().entry(name.clone()).or_insert((comps, line, col));
            return Ok(GType::new(&name, vec![]));
        }
        if let Some(owner) = &ty.owner {
            if *owner == self.old_owner {
                return Ok(GType { name: ty.name.clone(), args: ty.args.clone(), owner: Some(self.new_owner.clone()), index: ty.index });
            }
            return Ok(ty.clone());
        }
        if ty.args.is_empty() && crate::typeinfo::is_builtin_type_name(&ty.name) {
            return Ok(ty.clone());
        }
        let mut args: Vec<GType> = Vec::with_capacity(ty.args.len());
        for a in &ty.args {
            args.push(self.expand(a, line, col, busy)?);
        }
        if ty.name.starts_with("crate::") {
            let (m, rest) = self.walk(&ty.name);
            if let [name] = rest.as_slice() {
                if let Some(a) = self.module(m).aliases.iter().find(|a| a.name == *name) {
                    self.access(a.is_pub, m, line, col, &ty.name)?;
                    let id = (m, name.clone());
                    if busy.contains(&id) {
                        return Err(err(line, col, "alias_cycle", format!("cyclic alias {}", ty.name), "break the alias cycle"));
                    }
                    if a.params.len() != args.len() {
                        return Err(err(line, col, "type_arity", format!("{} takes {} type arguments", ty.name, a.params.len()), ""));
                    }
                    busy.push(id);
                    let decl = Rs { cur: m, old_owner: String::new(), new_owner: String::new(), ..*self };
                    let rhs = decl.expand(&GType::from_key(&a.rhs), a.line, a.col, busy)?;
                    busy.pop();
                    let mapping = args.into_iter().enumerate().map(|(i,arg)| ((format!("@alias {}", name), i), arg)).collect();
                    return Ok(crate::gtype::substitute(&rhs, &mapping));
                }
            }
        }
        let name = if ty.name == "@ref" || crate::typeinfo::is_builtin_type_name(&ty.name) {
            ty.name.clone()
        } else {
            self.local_or_path_type(&ty.name, line, col)?
        };
        let resolved = GType::new(&name, args);
        if ty.args.first().is_some_and(|a| a.name.starts_with("crate::")) {
            if let Some((elem, len)) = resolved.fixed_shape() {
                if !(elem.int_width().is_some() || elem == &GType::bool() || elem == &GType::str()) {
                    return Err(err(line, col, "fixed_array_element", "a fixed array requires scalar elements", ""));
                }
                if let Some(n) = len.const_value() { crate::parser::fixed_size(line, col, elem, n)?; }
            }
            if resolved.name == "@ref" && !resolved.args[0].is_array() && resolved.args[0] != GType::str() {
                return Err(PruvError::new("parse", line, col, "syntax", "`ref` takes an array or str type", ""));
            }
        }
        if resolved == *ty { return Ok(resolved); }
        Ok(crate::gtype::substitute(&resolved, &Default::default()))
    }

    /// `rtype` on an AST type key (an unchanged type keeps its key text)
    fn rtype_key(&self, key: &str, line: i64, col: i64) -> Res<String> {
        let g = GType::from_key(key);
        let r = self.rtype(&g, line, col)?;
        if r == g {
            Ok(key.to_string())
        } else {
            Ok(r.to_key())
        }
    }

    fn targs(&self, targs: &mut Option<Vec<GType>>, line: i64, col: i64) -> Res<()> {
        if let Some(v) = targs {
            let mut out: Vec<GType> = Vec::with_capacity(v.len());
            for t in v.iter() {
                out.push(self.rtype(t, line, col)?);
            }
            *v = out;
        }
        Ok(())
    }

    fn rrefine(&self, r: &mut Option<Refinement>) -> Res<()> {
        if let Some(r) = r {
            if !crate::typeinfo::is_builtin_type_name(&r.base) {
                r.base = self.rtype_key(&r.base, r.line, r.col)?;
            }
        }
        Ok(())
    }

    // ---- items ----
    fn tdecl(&mut self, d: &mut TypeDecl, meta: &ItemMeta) -> Res<()> {
        let path = &self.module(meta.module).path;
        self.old_owner = decl_owner(&meta.src_name);
        self.new_owner = decl_owner(&ident_in(path, &meta.src_name, meta.pkg.as_deref()));
        let new_name = ident_in(path, &meta.src_name, meta.pkg.as_deref());
        match d {
            TypeDecl::Struct(s) => {
                for fd in s.fields.iter_mut() {
                    fd.ty = self.rtype_key(&fd.ty, fd.line, fd.col)?;
                }
                // LG16: the invariant clauses resolve like a requires (calls to functions of the module)
                for e in s.invariants.iter_mut() {
                    self.rexpr(e)?;
                }
                s.name = new_name;
            }
            TypeDecl::Enum(e) => {
                for v in e.variants.iter_mut() {
                    for fd in v.fields.iter_mut() {
                        fd.ty = self.rtype_key(&fd.ty, fd.line, fd.col)?;
                    }
                }
                e.name = new_name;
            }
        }
        Ok(())
    }

    fn fn_(&mut self, f: &mut Function, meta: &ItemMeta) -> Res<()> {
        let path = &self.module(meta.module).path;
        self.old_owner = fn_owner(&meta.src_name);
        self.new_owner = fn_owner(&ident_in(path, &meta.src_name, meta.pkg.as_deref()));
        let (fl, fc) = (f.line, f.col);
        for p in f.params.iter_mut() {
            p.ty = self.rtype_key(&p.ty, fl, fc)?;
            self.rrefine(&mut p.refine)?;
        }
        f.ret = self.rtype_key(&f.ret, fl, fc)?;
        self.rrefine(&mut f.ret_refine)?;
        for e in f.requires.iter_mut() {
            self.rexpr(e)?;
        }
        for e in f.ensures.iter_mut() {
            self.rexpr(e)?;
        }
        if let Some(d) = f.cost.as_mut() { self.rexpr(d)?; }
        if let Some(d) = f.decreases.as_mut() {
            self.rexpr(d)?;
        }
        self.rstmts(&mut f.body)?;
        f.name = ident_in(path, &meta.src_name, meta.pkg.as_deref());
        Ok(())
    }

    // ---- statements ----
    fn rstmts(&self, stmts: &mut [Stmt]) -> Res<()> {
        for st in stmts.iter_mut() {
            match st {
                Stmt::Let(l) => {
                    if !l.ty.is_empty() { l.ty = self.rtype_key(&l.ty, l.line, l.col)?; }
                    self.rrefine(&mut l.refine)?;
                    self.rexpr(&mut l.value)?;
                }
                Stmt::Assign(a) => self.rexpr(&mut a.value)?,
                Stmt::FieldAssign(a) => self.rexpr(&mut a.value)?,
                Stmt::IndexAssign(a) => {
                    self.rexpr(&mut a.index)?;
                    self.rexpr(&mut a.value)?;
                }
                Stmt::If(i) => {
                    self.rexpr(&mut i.cond)?;
                    self.rstmts(&mut i.then)?;
                    self.rstmts(&mut i.els)?;
                }
                Stmt::While(w) => {
                    self.rexpr(&mut w.cond)?;
                    for e in w.invariants.iter_mut() {
                        self.rexpr(e)?;
                    }
                    self.rexpr(&mut w.decreases)?;
                    self.rstmts(&mut w.body)?;
                }
                Stmt::ForMap(w) => {
                    self.rexpr(&mut w.place)?;
                    for inv in &mut w.invariants {
                        self.rexpr(inv)?;
                    }
                    self.rstmts(&mut w.body)?;
                }
                Stmt::Break(_) | Stmt::Continue(_) => {}
                Stmt::Return(r) => {
                    if let Some(v) = r.value.as_mut() {
                        self.rexpr(v)?;
                    }
                }
                Stmt::Assert(a) => {
                    self.rexpr(&mut a.cond)?;
                    if let Some((n, l, c)) = a.reveal.take() {
                        // P13o: the revealed name resolves like a call
                        let mut ce = Expr::Call(Call {
                            line: l, col: c, name: n.clone(), args: vec![], ty: None, targs: None, inst: None,
                            old: None, opaque_c: false,
                        });
                        self.rexpr(&mut ce)?;
                        let n2 = if let Expr::Call(cc) = &ce { cc.name.clone() } else { n };
                        a.reveal = Some((n2, l, c));
                    }
                }
                Stmt::Par(x) => {
                    for c in &mut x.calls {
                        self.rexpr(c)?;
                    }
                }
                Stmt::ExprStmt(x) => self.rexpr(&mut x.expr)?,
                Stmt::Match(m) => {
                    self.rexpr(&mut m.scrutinee)?;
                    for a in m.arms.iter_mut() {
                        if let Some(en) = &a.enum_name {
                            let n = self.local_or_path_type(en, a.line, a.col)?;
                            a.enum_name = Some(n);
                        }
                        let (l, c) = (a.line, a.col);
                        crate::ast::SubPat::map_slots(&mut a.binders, &mut |en| self.local_or_path_type(en, l, c))?;
                        self.rstmts(&mut a.body)?;
                    }
                    self.rstmts(&mut m.els)?; // P8: the else block of `if let` / `let ... else`
                }
            }
        }
        Ok(())
    }

    // ---- expressions ----
    fn rexpr(&self, e: &mut Expr) -> Res<()> {
        if let Expr::IntLit(lit) = e {
            if let LitKind::Const(name) = &mut lit.kind {
                if let Some(c) = self.module(self.cur).consts.get(name) { *name = c.name.clone(); }
            }
        }
        if let Expr::Var(v) = e {
            if v.name.starts_with("crate::") {
                let (m, rest) = self.walk(&v.name);
                if rest.len() != 1 { return Err(self.unknown_module(m, &rest, v.line, v.col, &v.name)); }
                let c = self.module(m).consts.get(&rest[0]).ok_or_else(|| err(v.line, v.col, "unknown_name",
                    format!("unknown constant {}", v.name), "declare the constant in that module"))?;
                self.access(c.is_pub, m, v.line, v.col, &v.name)?;
                *e = Expr::IntLit(IntLit { line: v.line, col: v.col, value: c.value.to_string(), ty: Some(c.ty.clone()), kind: LitKind::Const(c.name.clone()) });
                return Ok(());
            }
        }
        if let Expr::Call(call) = e {
            if PlaceIf::matches(call) || crate::ast::ValueIf::matches(call) {
                return call.args.iter_mut().try_for_each(|arg| self.rexpr(arg));
            }
            if crate::ast::ValueMatch::matches(call) {
                // P9a-5: an arm pattern names its enum like a statement match arm does
                let mut arms = crate::ast::ValueMatch::arms(call);
                for a in arms.iter_mut() {
                    if let Some(en) = &a.enum_name {
                        a.enum_name = Some(self.local_or_path_type(en, call.line, call.col)?);
                    }
                    let (l, c) = (call.line, call.col);
                    crate::ast::SubPat::map_slots(&mut a.binders, &mut |en| self.local_or_path_type(en, l, c))?;
                }
                call.name = crate::ast::ValueMatch::rename(&arms);
                return call.args.iter_mut().try_for_each(|arg| self.rexpr(arg));
            }
        }
        // P9b-2: `crate::m::Owner::f(args)` calls the impl function `Owner__f` of module m (the type and the function
        // must both be visible here)
        if let Expr::Call(c) = &mut *e {
            if c.name.starts_with("crate::") {
                let (m, rem) = self.walk(&c.name);
                let md = self.module(m);
                let lowered = rem.get(1).map(|f| format!("{}__{}", rem[0], f));
                if let (2, Some(&ti), Some(&fi)) =
                    (rem.len(), md.types.get(&rem[0]), lowered.as_ref().and_then(|l| md.funcs.get(l)))
                {
                    let owner = format!("{}::{}", disp_pkg(&md.path, md.pkg.as_deref()), rem[0]);
                    self.access(self.g4.ty_meta[ti].is_pub, m, c.line, c.col, &owner)?;
                    self.access(self.g4.fn_meta[fi].is_pub, m, c.line, c.col, &c.name)?;
                    c.name = ident_in(&md.path, lowered.as_deref().unwrap_or_default(), md.pkg.as_deref());
                    self.targs(&mut c.targs, c.line, c.col)?;
                    return c.args.iter_mut().try_for_each(|a| self.rexpr(a));
                }
            }
        }
        // `crate::m::Enum::Variant(args)` parses as a Call: a variant constructor replaces the node
        let mut replace: Option<Expr> = None;
        if let Expr::Call(c) = &mut *e {
            if c.name.starts_with("crate::") {
                let (m, rem) = self.walk(&c.name);
                if rem.len() == 2 && self.module(m).types.contains_key(&rem[0]) {
                    if c.args.is_empty() {
                        let prefix = match c.name.rfind("::") {
                            Some(i) => c.name[..i].to_string(),
                            None => c.name.clone(),
                        };
                        return Err(PruvError::new(
                            "parse",
                            c.line,
                            c.col,
                            "syntax",
                            "empty parentheses after a variant",
                            format!("write `{}::{}` for a variant without fields, or pass its fields as arguments", prefix, rem[1]),
                        ));
                    }
                    let md = self.module(m);
                    let ti = md.types[&rem[0]];
                    self.access(
                        self.g4.ty_meta[ti].is_pub,
                        m,
                        c.line,
                        c.col,
                        &format!("{}::{}", disp_pkg(&md.path, md.pkg.as_deref()), rem[0]),
                    )?;
                    let args = std::mem::take(&mut c.args);
                    replace = Some(Expr::EnumCtor(EnumCtor {
                        line: c.line,
                        col: c.col,
                        enum_name: ident_in(&md.path, &rem[0], md.pkg.as_deref()),
                        variant: rem[1].clone(),
                        args,
                        ty: None,
                        // P9b-3: `crate::m::Enum::<A>::V(args)` keeps its type arguments
                        targs: c.targs.take(),
                        inst: None,
                    }));
                }
            }
        }
        if let Some(mut n) = replace {
            if let Expr::EnumCtor(ec) = &mut n {
                self.targs(&mut ec.targs, ec.line, ec.col)?;
                for a in ec.args.iter_mut() {
                    self.rexpr(a)?;
                }
            }
            *e = n;
            return Ok(());
        }
        match e {
            Expr::Call(c) => {
                if c.name.starts_with("crate::") {
                    let (m, rem) = self.walk(&c.name);
                    let nm = self.fn_item(m, &rem, c.line, c.col, &c.name)?;
                    c.name = nm;
                } else {
                    let cur = self.module(self.cur);
                    if cur.funcs.contains_key(&c.name) {
                        c.name = ident_in(&cur.path, &c.name, cur.pkg.as_deref());
                    }
                }
                self.targs(&mut c.targs, c.line, c.col)?;
                for a in c.args.iter_mut() {
                    self.rexpr(a)?;
                }
            }
            Expr::EnumCtor(x) => {
                x.enum_name = self.local_or_path_type(&x.enum_name, x.line, x.col)?;
                self.targs(&mut x.targs, x.line, x.col)?;
                for a in x.args.iter_mut() {
                    self.rexpr(a)?;
                }
            }
            Expr::StructLit(x) => {
                x.name = self.local_or_path_type(&x.name, x.line, x.col)?;
                self.targs(&mut x.targs, x.line, x.col)?;
                for fi in x.fields.iter_mut() {
                    self.rexpr(&mut fi.value)?;
                }
            }
            Expr::IsVariant(x) => {
                x.enum_name = self.local_or_path_type(&x.enum_name, x.line, x.col)?;
                self.rexpr(&mut x.value)?;
            }
            Expr::VariantField(x) => {
                x.enum_name = self.local_or_path_type(&x.enum_name, x.line, x.col)?;
                self.rexpr(&mut x.value)?;
            }
            Expr::Ctor(x) => {
                self.targs(&mut x.targs, x.line, x.col)?;
                if let Some(a) = x.arg.as_mut() {
                    self.rexpr(a)?;
                }
            }
            Expr::Unary(x) => self.rexpr(&mut x.operand)?,
            Expr::Binary(x) => {
                self.rexpr(&mut x.left)?;
                self.rexpr(&mut x.right)?;
            }
            Expr::Index(x) => {
                self.rexpr(&mut x.array)?;
                self.rexpr(&mut x.index)?;
            }
            Expr::Quant(x) => {
                self.rexpr(&mut x.lo)?;
                self.rexpr(&mut x.hi)?;
                self.rexpr(&mut x.body)?;
                for t in x.triggers.iter_mut() {
                    self.rexpr(t)?;
                }
            }
            Expr::FieldAccess(x) => self.rexpr(&mut x.base)?,
            Expr::InoutArg(x) => self.rexpr(&mut x.expr)?, // G5 A0
            Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) | Expr::Move(_) => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// public interfaces
// ---------------------------------------------------------------------------

/// visibility and field / payload types (in declaration order) of one declared type, by identity
struct TyIface {
    is_pub: bool,
    fields: Vec<(String, GType)>,
}

/// the fields of a declaration (Python `decl_fields`): struct fields, or the payload fields of every variant in order
fn decl_fields(d: &TypeDecl) -> Vec<&FieldDecl> {
    match d {
        TypeDecl::Struct(s) => s.fields.iter().collect(),
        TypeDecl::Enum(e) => e.variants.iter().flat_map(|v| v.fields.iter()).collect(),
    }
}

/// G4 A1 (spec 5): the first private declaration reachable from the nominal `names` (identities): the names themselves,
/// then, transitively, the field / payload types of every public declaration reached. `seen` is the visited set shared by
/// the whole program (cycles terminate). Returns (private identity, via) where via is the chain "P.field -> Q.g" leading to
/// it ("" for a direct mention).
fn private_in(types: &HashMap<String, TyIface>, names: &[String], seen: &mut HashSet<String>) -> Option<(String, String)> {
    for n in names {
        let d = match types.get(n) {
            Some(d) => d,
            None => continue,
        };
        if !d.is_pub {
            return Some((n.clone(), String::new()));
        }
        if seen.contains(n) {
            continue;
        }
        seen.insert(n.clone());
        for (fname, fty) in &d.fields {
            if let Some((p, via)) = private_in(types, &dependencies(fty), seen) {
                let chain = if via.is_empty() { format!("{}.{}", n, fname) } else { format!("{}.{} -> {}", n, fname, via) };
                return Some((p, chain));
            }
        }
    }
    None
}

fn leak(line: i64, col: i64, is_fn: bool, item_name: &str, r: (String, String), what: &str) -> PruvError {
    let (n, via) = r;
    err(
        line,
        col,
        "public_interface",
        format!(
            "public {} {} exposes the private type {} in its {}{}",
            if is_fn { "function" } else { "type" },
            item_name,
            n,
            what,
            if via.is_empty() { String::new() } else { format!(" (through {})", via) }
        ),
        format!("make {} `pub`, or keep {} private", n, item_name),
    )
}

#[allow(clippy::too_many_arguments)]
fn iface_type(
    types: &HashMap<String, TyIface>,
    ty: &GType,
    line: i64,
    col: i64,
    is_fn: bool,
    item_name: &str,
    what: &str,
    seen: &mut HashSet<String>,
) -> Res<()> {
    if let Some(r) = private_in(types, &dependencies(ty), seen) {
        return Err(leak(line, col, is_fn, item_name, r, what));
    }
    Ok(())
}

/// the expression children in the order of refine.py `_kids`
fn kids(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Spent(_) => Vec::new(),
        Expr::Unary(x) => vec![&*x.operand],
        Expr::Binary(x) => vec![&*x.left, &*x.right],
        Expr::Index(x) => vec![&*x.array, &*x.index],
        Expr::Ctor(x) => x.arg.iter().map(|a| &**a).collect(),
        Expr::Quant(x) => {
            let mut v = vec![&*x.lo, &*x.hi, &*x.body];
            v.extend(x.triggers.iter());
            v
        }
        Expr::Call(x) => x.args.iter().collect(),
        Expr::StructLit(x) => x.fields.iter().map(|f| &f.value).collect(),
        Expr::EnumCtor(x) => x.args.iter().collect(),
        Expr::FieldAccess(x) => vec![&*x.base],
        Expr::IsVariant(x) => vec![&*x.value],
        Expr::VariantField(x) => vec![&*x.value],
        _ => Vec::new(),
    }
}

/// full syntax of a contract expression, unreachable short-circuit operands included
fn iface_expr(
    types: &HashMap<String, TyIface>,
    fns: &HashMap<String, bool>,
    e: &Expr,
    item_name: &str,
    seen: &mut HashSet<String>,
) -> Res<()> {
    let (line, col) = (e.line(), e.col());
    let name: Option<&String> = match e {
        Expr::StructLit(x) => Some(&x.name),
        Expr::EnumCtor(x) => Some(&x.enum_name),
        Expr::IsVariant(x) => Some(&x.enum_name),
        Expr::VariantField(x) => Some(&x.enum_name),
        _ => None,
    };
    if let Some(n) = name {
        if let Some(r) = private_in(types, std::slice::from_ref(n), seen) {
            return Err(leak(line, col, true, item_name, r, "contract"));
        }
    }
    if let Expr::Call(x) = e {
        // P06: a public contract expands only public functions (their bodies are its meaning)
        if fns.get(&x.name) == Some(&false) {
            return Err(err(
                line,
                col,
                "public_interface",
                format!("public function {} exposes the private function {} in its contract", item_name, x.name),
                format!("make {} `pub`, or keep {} private", x.name, item_name),
            ));
        }
    }
    let targs: Option<&Vec<GType>> = match e {
        Expr::Call(x) => x.targs.as_ref(),
        Expr::StructLit(x) => x.targs.as_ref(),
        Expr::EnumCtor(x) => x.targs.as_ref(),
        Expr::Ctor(x) => x.targs.as_ref(),
        _ => None,
    };
    if let Some(ts) = targs {
        for t in ts {
            iface_type(types, t, line, col, true, item_name, "contract", seen)?;
        }
    }
    for k in kids(e) {
        iface_expr(types, fns, k, item_name, seen)?;
    }
    Ok(())
}

fn iface_fn(types: &HashMap<String, TyIface>, fns: &HashMap<String, bool>, f: &Function, seen: &mut HashSet<String>) -> Res<()> {
    for p in &f.params {
        iface_type(types, &GType::from_key(&p.ty), f.line, f.col, true, &f.name, "signature", seen)?;
    }
    iface_type(types, &GType::from_key(&f.ret), f.line, f.col, true, &f.name, "signature", seen)?;
    for e in f.requires.iter().chain(f.ensures.iter()).chain(f.decreases.iter()).chain(f.cost.iter()) {
        iface_expr(types, fns, e, &f.name, seen)?;
    }
    Ok(())
}

/// P06 for dot calls (P9b-2): `c.m(..)` in a contract is still `@method|m` when `iface_fn` runs, so after
/// `methods::resolve` a public function's contract must not call a method without `pub` (R-6x-5).
pub fn contract_methods(prog: &Program, g4: &G4) -> Res<()> {
    let private: HashSet<&str> = prog.methods.iter().filter(|m| !m.is_pub).map(|m| m.func.as_str()).collect();
    if private.is_empty() {
        return Ok(());
    }
    for (i, f) in prog.functions.iter().enumerate() {
        if !g4.fn_meta.get(i).is_some_and(|m| m.is_pub) {
            continue;
        }
        let mut hit: Option<(i64, i64, String)> = None;
        for e in f.requires.iter().chain(f.ensures.iter()).chain(f.decreases.iter()).chain(f.cost.iter()) {
            crate::foreign_shim::walk_expr(e, &mut |n| {
                if let crate::foreign_shim::Node::Expr(Expr::Call(x)) = n {
                    if hit.is_none() && private.contains(x.name.as_str()) {
                        hit = Some((x.line, x.col, x.name.clone()));
                    }
                }
            });
        }
        if let Some((line, col, name)) = hit {
            return Err(err(
                line,
                col,
                "public_interface",
                format!("public function {} exposes the private function {} in its contract", f.name, name),
                format!("make {} `pub`, or keep {} private", name, f.name),
            ));
        }
    }
    Ok(())
}

fn iface_ty(types: &HashMap<String, TyIface>, d: &TypeDecl, seen: &mut HashSet<String>) -> Res<()> {
    for fd in decl_fields(d) {
        iface_type(types, &GType::from_key(&fd.ty), fd.line, fd.col, false, d.name(), &format!("field {}", fd.name), seen)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// entries
// ---------------------------------------------------------------------------

fn entries(prog: &Program, g4: &mut G4) -> Res<()> {
    let mut out: Vec<String> = Vec::new();
    let impl_fns: HashSet<&str> = prog.methods.iter().map(|m| m.func.as_str()).collect();
    for (i, f) in prog.functions.iter().enumerate() {
        let meta_module = g4.fn_meta[i].module;
        if !g4.modules[meta_module].path.is_empty() || g4.fn_meta[i].pkg.is_some() {
            continue; // not the program root (a dependency root's `pub fn` is an ordinary public function)
        }
        if g4.fn_meta[i].src_name == "main" {
            if !g4.fn_meta[i].is_pub {
                return Err(err(f.line, f.col, "entry_main", "the root main must be `pub fn main() -> ()`", "write `pub fn main() -> () { .. }`"));
            }
            if !f.tparams.is_empty() {
                return Err(err(
                    f.line,
                    f.col,
                    "entry_main",
                    "the root main cannot declare type parameters",
                    "make main a plain function; call generic functions from it with explicit type arguments",
                ));
            }
            if !f.params.is_empty() {
                return Err(err(f.line, f.col, "entry_main", "the root main takes no parameters", "remove the parameters of main"));
            }
            if f.ret != "unit" {
                return Err(err(f.line, f.col, "entry_main", "the root main must return ()", "write `pub fn main() -> ()`"));
            }
            g4.main = Some(f.name.clone());
        }
        // R03L: a pred is never a host entry; P9b-2: nor is an impl function (`pub fn` in an impl block)
        if g4.fn_meta[i].is_pub && f.tparams.is_empty() && f.foreign.is_none() && !f.pred
            && !impl_fns.contains(f.name.as_str())
        {
            g4.fn_meta[i].host_entry = true;
            out.push(f.name.clone());
        }
    }
    g4.entries = out;
    Ok(())
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    const FN: &str = "pub fn f() -> i64 ensures result == 1 { return 1; }\n";

    /// a fresh scratch directory with the given files (relative path, text)
    fn project(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pruv_lib_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        for (p, t) in files {
            let f = d.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        }
        d
    }

    fn files_of(r: Result<Loaded, LoadError>) -> Result<Vec<String>, PruvError> {
        match r {
            Ok(l) => Ok(l.g4.unwrap().modules.iter().map(|m| m.file.clone()).collect()),
            Err(LoadError::Lang(e)) => Err(e),
            Err(_) => panic!("unexpected io/decode error"),
        }
    }

    #[test]
    fn library_module_found() {
        let d = project("found", &[("app/main.pruv", "mod json;\n"), ("lib/json.pruv", FN)]);
        let r = load_program_with_libs(&d.join("app/main.pruv"), &[d.join("lib")]);
        assert_eq!(files_of(r).unwrap(), vec!["main.pruv", "@lib/json.pruv"]);
        // without the library the file is missing and the root hint names --lib
        let e = files_of(load_program(&d.join("app/main.pruv"))).unwrap_err();
        assert_eq!(e.code, "module_file");
        assert_eq!(e.fix, "create json.pruv next to the root file, pass --lib DIR, or remove `mod json;`");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_and_library_conflict() {
        let d = project(
            "conflict",
            &[("app/main.pruv", "mod json;\n"), ("app/json.pruv", FN), ("l1/json.pruv", FN), ("l2/json.pruv", FN)],
        );
        let e = files_of(load_program_with_libs(&d.join("app/main.pruv"), &[d.join("l1"), d.join("l2")])).unwrap_err();
        assert_eq!(e.code, "lib_module_conflict");
        assert_eq!(e.message, "module json is found in more than one place: json.pruv, @lib/json.pruv, @lib/json.pruv");
        assert_eq!(e.fix, "remove the duplicate file or drop one --lib");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn library_module_child() {
        let d = project(
            "child",
            &[("app/main.pruv", "mod json;\n"), ("lib/json.pruv", "mod x;\n"), ("lib/json/x.pruv", FN)],
        );
        let r = load_program_with_libs(&d.join("app/main.pruv"), &[d.join("lib")]);
        assert_eq!(files_of(r).unwrap(), vec!["main.pruv", "@lib/json.pruv", "@lib/json/x.pruv"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}

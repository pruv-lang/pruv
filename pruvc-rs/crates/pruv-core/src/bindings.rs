//! Rust bindings generator for the LLVM backend (counterparts of llvmgen.py's rust_ty, rs_ser, rs_de, gen_bindings).
//! The output must be byte-identical to Python's.

use crate::ast::*;
use crate::emit_common::{emit_public_types, emit_public_types_with, is_spec_fn, rty};
use crate::gtype::GType;
use crate::llvmgen::{g_arg, is_agg, is_gopt};
use crate::typeinfo::{is_array_name, is_user_type, uses_array_type, uses_str, TypeTable};

pub const ALLOW: &str = "#[allow(dead_code, non_snake_case, unused)]";

// P3: the host helpers are called only by runtime.c, which has already checked every argument: alloc gets
// len > 0 within the runtime's bound and a valid layout, free / realloc get a non-null block with its own layout
// (pruv_free / pruv_free_v2 / pruv_node_free_v1 / pruv_shrink_v2 skip NULL and len <= 0). So the layout is built
// unchecked; a null result still goes to handle_alloc_error.
const HOST_ALLOC: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc(len: i64) -> *mut i64 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8) };
    let p = unsafe { ::std::alloc::alloc(layout) } as *mut i64;
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}"##;

const HOST_ALLOC_ZEROED: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_zeroed(len: i64) -> *mut i64 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8) };
    let p = unsafe { ::std::alloc::alloc_zeroed(layout) } as *mut i64;
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}"##;

const HOST_ALLOC_V2: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_v2(len: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::alloc(layout) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}"##;

const HOST_ALLOC_ZEROED_V2: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_zeroed_v2(len: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::alloc_zeroed(layout) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}"##;

const HOST_FREE_V2: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_free_v2(data: *mut u8, len: i64, size: i64, align: i64) {
    unsafe { ::std::alloc::dealloc(data, ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize)) };
}"##;

const HOST_REALLOC_V2: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_realloc_v2(data: *mut u8, len: i64, m: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::realloc(data, layout, (m as usize) * (size as usize)) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}"##;

const HOST_FLUSH: &str = r##"#[no_mangle]
pub extern "C" fn pruv_host_flush() {
    let _ = ::std::io::Write::flush(&mut ::std::io::stdout());
}"##;

const HOST_FREE: &str = r##"#[no_mangle]
pub unsafe extern "C" fn pruv_host_free(data: *mut i64, len: i64) {
    unsafe { ::std::alloc::dealloc(data as *mut u8, ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8)) };
}"##;

/// Number of i64 words in the external (pruv_abi_*) form (llvmgen.words).
fn words(tt: &TypeTable, ty: &str) -> Result<i64, String> {
    // the same (memoized) count as the wrapper generator, so both sides agree by construction
    crate::llvmgen::words(tt, ty).map(|n| n as i64)
}

/// Python's `tt.decls[name]` KeyError (str(e) == the quoted name).
fn decl<'a>(tt: &'a TypeTable, ty: &str) -> Result<&'a TypeDecl, String> {
    tt.decl(ty).ok_or_else(|| format!("'{}'", ty))
}

/// Whether the function name is mangled (it changes if it has a `[^A-Za-z0-9_]` character).
fn needs_mangling(n: &str) -> bool {
    n.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
}

/// G2 A5: if there is a [bool] parameter, the binding calls pruv_abi_<f>__typed, which has no byte check.
fn typed_abi_name(f: &Function) -> String {
    // G2 B2: the same for &str
    if f.params.iter().any(|p| p.ty == "array_bool" || p.ty == "str") {
        format!("{}__typed", f.name)
    } else {
        f.name.clone()
    }
}

fn rust_ty(t: &str, param: bool) -> String {
    if is_gopt(t) {
        // G3: ::std::option::Option<R(X)> / ::std::result::Result<R(X), i64>
        return rty(t, false);
    }
    if is_user_type(t) {
        return t.to_string();
    }
    match t {
        "i64" => "i64".to_string(),
        "bool" => "bool".to_string(),
        "unit" => "()".to_string(),
        "option" => "::std::option::Option<i64>".to_string(),
        "result" => "::std::result::Result<i64, i64>".to_string(),
        "array_u8" => (if param { "&[u8]" } else { "::std::vec::Vec<u8>" }).to_string(),
        "array_bool" => (if param { "&[bool]" } else { "::std::vec::Vec<bool>" }).to_string(), // G2 A5
        "str" => (if param { "&str" } else { "::std::string::String" }).to_string(), // G2 B2
        _ => (if param { "&[i64]" } else { "::std::vec::Vec<i64>" }).to_string(),
    }
}

/// Rust statements that write the value `expr` (Copy) into the word array `arr` at `off`.
fn rs_ser(tt: &TypeTable, ty: &str, expr: &str, arr: &str, off: i64, ctr: &mut i64) -> Result<Vec<String>, String> {
    if is_gopt(ty) {
        // G3: the word array starts zeroed, so only the active words are written
        *ctr += 1;
        let v = format!("pruv__b{}", *ctr);
        let x = g_arg(ty);
        let inner = rs_ser(tt, x, &v, arr, off + 1, ctr)?.join(" ");
        if ty.starts_with("option<") {
            return Ok(vec![format!(
                "match {} {{ Some({}) => {{ {}[{}] = 1; {} }} None => {{ {}[{}] = 0; }} }}",
                expr, v, arr, off, inner, arr, off
            )]);
        }
        return Ok(vec![format!(
            "match {} {{ Ok({}) => {{ {}[{}] = 1; {} }} Err({}) => {{ {}[{}] = 0; {}[{}] = {}; }} }}",
            expr,
            v,
            arr,
            off,
            inner,
            v,
            arr,
            off,
            arr,
            off + 1 + words(tt, x)?,
            v
        )]);
    }
    match ty {
        "i64" => return Ok(vec![format!("{}[{}] = {};", arr, off, expr)]),
        "bool" => return Ok(vec![format!("{}[{}] = ({}) as i64;", arr, off, expr)]),
        "option" | "result" => {
            *ctr += 1;
            let v = format!("pruv__b{}", *ctr);
            if ty == "option" {
                return Ok(vec![format!(
                    "match {} {{ Some({}) => {{ {}[{}] = 1; {}[{}] = {}; }} None => {{ {}[{}] = 0; {}[{}] = 0; }} }}",
                    expr, v, arr, off, arr, off + 1, v, arr, off, arr, off + 1
                )]);
            }
            return Ok(vec![format!(
                "match {} {{ Ok({}) => {{ {}[{}] = 1; {}[{}] = {}; }} Err({}) => {{ {}[{}] = 0; {}[{}] = {}; }} }}",
                expr, v, arr, off, arr, off + 1, v, v, arr, off, arr, off + 1, v
            )]);
        }
        _ => {}
    }
    match decl(tt, ty)? {
        TypeDecl::Struct(s) => {
            let mut out = Vec::new();
            let mut o = off;
            for f in &s.fields {
                out.extend(rs_ser(tt, &f.ty, &format!("{}.{}", expr, f.name), arr, o, ctr)?);
                o += words(tt, &f.ty)?;
            }
            Ok(out)
        }
        TypeDecl::Enum(d) => {
            let mut arms: Vec<String> = Vec::new();
            let mut o = off + 1;
            for (k, v) in d.variants.iter().enumerate() {
                let mut names: Vec<String> = Vec::new();
                let mut stmts: Vec<String> = vec![format!("{}[{}] = {};", arr, off, k)];
                for f in &v.fields {
                    *ctr += 1;
                    let nm = format!("pruv__b{}", *ctr);
                    stmts.extend(rs_ser(tt, &f.ty, &nm, arr, o, ctr)?);
                    names.push(nm);
                    o += words(tt, &f.ty)?;
                }
                let pat = if names.is_empty() {
                    format!("{}::{}", d.name, v.name)
                } else {
                    format!("{}::{}({})", d.name, v.name, names.join(", "))
                };
                arms.push(format!("{} => {{ {} }}", pat, stmts.join(" ")));
            }
            Ok(vec![format!("match {} {{ {} }}", expr, arms.join(" "))])
        }
    }
}

/// Rust expression that rebuilds a ty value from the word array `arr` at `off` (safe constructors only).
fn rs_de(tt: &TypeTable, ty: &str, arr: &str, off: i64) -> Result<String, String> {
    if is_gopt(ty) {
        // G3
        let x = g_arg(ty);
        let xe = rs_de(tt, x, arr, off + 1)?;
        if ty.starts_with("option<") {
            return Ok(format!("(if {}[{}] != 0 {{ Some({}) }} else {{ None }})", arr, off, xe));
        }
        return Ok(format!(
            "(if {}[{}] != 0 {{ Ok({}) }} else {{ Err({}[{}]) }})",
            arr,
            off,
            xe,
            arr,
            off + 1 + words(tt, x)?
        ));
    }
    match ty {
        "i64" => return Ok(format!("{}[{}]", arr, off)),
        "bool" => return Ok(format!("({}[{}] != 0)", arr, off)),
        "option" => {
            return Ok(format!("(if {}[{}] != 0 {{ Some({}[{}]) }} else {{ None }})", arr, off, arr, off + 1));
        }
        "result" => {
            return Ok(format!(
                "(if {}[{}] != 0 {{ Ok({}[{}]) }} else {{ Err({}[{}]) }})",
                arr, off, arr, off + 1, arr, off + 1
            ));
        }
        _ => {}
    }
    match decl(tt, ty)? {
        TypeDecl::Struct(s) => {
            let mut parts: Vec<String> = Vec::new();
            let mut o = off;
            for f in &s.fields {
                parts.push(format!("{}: {}", f.name, rs_de(tt, &f.ty, arr, o)?));
                o += words(tt, &f.ty)?;
            }
            Ok(format!("{} {{ {} }}", s.name, parts.join(", ")))
        }
        TypeDecl::Enum(d) => {
            let mut arms: Vec<String> = Vec::new();
            let mut o = off + 1;
            for (k, v) in d.variants.iter().enumerate() {
                let mut args: Vec<String> = Vec::new();
                for f in &v.fields {
                    args.push(rs_de(tt, &f.ty, arr, o)?);
                    o += words(tt, &f.ty)?;
                }
                let tail = if args.is_empty() { String::new() } else { format!("({})", args.join(", ")) };
                arms.push(format!("{} => {}::{}{},", k, d.name, v.name, tail));
            }
            Ok(format!("(match {}[{}] {{ {} _ => ::std::unreachable!() }})", arr, off, arms.join(" ")))
        }
    }
}

pub fn gen_bindings(prog: &Program) -> Result<String, String> {
    gen_bindings_with(prog, None, true)
}

/// G4 A3 (Python `entry_type_closure`): names of the nominal types reachable from the host entry signatures (parameters and
/// results), following Option/Result arguments and struct fields / enum payloads, with a visited set (cycles).
fn entry_type_closure(prog: &Program, tt: &TypeTable) -> std::collections::HashSet<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut work: Vec<GType> = Vec::new();
    if let Some(x) = &prog.explicit {
        for (f, mk) in prog.functions.iter().zip(x.fns.iter()) {
            if crate::llvmgen::is_host_abi_fn(f, tt, true, mk.host_entry) {
                for p in &f.params {
                    work.push(GType::from_key(&p.ty));
                }
                work.push(GType::from_key(&f.ret));
            }
        }
    }
    while let Some(t) = work.pop() {
        work.extend(t.args.iter().cloned());
        if t.is_user_type() && !seen.contains(&t.name) {
            seen.insert(t.name.clone());
            if let Some(d) = tt.decl(&t.name) {
                for fd in d.all_fields() {
                    work.push(GType::from_key(&fd.ty));
                }
            }
        }
    }
    seen
}

/// shim: the `mod pruv_g6_shim` text of a G6 program (foreign_shim::build_shim); None for every other program.
pub fn gen_bindings_with(prog: &Program, shim: Option<&str>, verified: bool) -> Result<String, String> {
    let tt = crate::llvmgen::new_tt(prog); // G9 A3: Python new_tt (extended / boxed metadata)
    let mut o: Vec<String> = Vec::new();
    o.push("// generated by pruvc llvmgen -- Rust bindings for the LLVM back end (include with #[path] mod sol;)".to_string());
    let explicit = prog.explicit.is_some();
    let raw: Option<String> = if explicit { prog.raw_entries.clone() } else { None }; // P09
    let mut raw_items: Vec<(String, String, String, Vec<String>)> = Vec::new();
    if !prog.types.is_empty() {
        let publ = if explicit {
            // G4 A3 (docs/G4_SPEC.md 7): only the nominal types the host entry signatures depend on, transitively
            // (declaration order of the type table is kept)
            let need = entry_type_closure(prog, &tt);
            if need.is_empty() {
                String::new()
            } else {
                let mut t2 = tt.clone();
                let keep: Vec<usize> = t2.order.iter().copied().filter(|&i| need.contains(t2.decls[i].name())).collect();
                t2.order = keep;
                emit_public_types_with(prog, &t2)
            }
        } else {
            emit_public_types(prog)
        };
        if publ.contains("#[derive") {
            o.push(publ.replace("#[derive", &format!("{}\n#[derive", ALLOW)));
        } else if !publ.is_empty() {
            o.push(publ);
        }
    }
    o.push(ALLOW.to_string());
    o.push("#[repr(C)]\npub struct Pruv__Array { pub data: *mut i64, pub len: i64 }".to_string());
    if uses_array_type(prog, "array_u8") {
        o.push(ALLOW.to_string());
        o.push("#[repr(C)]\npub struct Pruv__ArrayU8 { pub data: *mut u8, pub len: i64 }".to_string());
    }
    if uses_array_type(prog, "array_bool") {
        // G2 A5
        o.push(ALLOW.to_string());
        o.push("#[repr(C)]\npub struct Pruv__ArrayBool { pub data: *mut u8, pub len: i64 }".to_string());
    }
    if uses_str(prog, !verified) {
        // G2 B2
        o.push(ALLOW.to_string());
        o.push("#[repr(C)]\npub struct Pruv__Str { pub data: *mut u8, pub len: i64 }".to_string());
    }
    o.push(ALLOW.to_string());
    o.push("#[repr(C)]\npub struct Pruv__Tagged { pub tag: i64, pub value: i64 }".to_string());
    for h in [HOST_ALLOC, HOST_ALLOC_ZEROED, HOST_FLUSH, HOST_FREE, HOST_ALLOC_V2, HOST_ALLOC_ZEROED_V2, HOST_FREE_V2, HOST_REALLOC_V2] {
        o.push(ALLOW.to_string());
        o.push(h.to_string());
    }
    // G3: specializations of generic functions are private (no ABI symbol, not callable from the bindings)
    // G6 A3: trusted declarations and collection-bearing functions have no binding
    // G9 A3: a function with an extended type in its signature has no binding either
    // G4 A3: in explicit mode only the host entries have a binding
    let hosts: Vec<bool> = match &prog.explicit {
        Some(x) => x.fns.iter().map(|mk| mk.host_entry).collect(),
        None => Vec::new(),
    };
    let pubs: Vec<&Function> = prog
        .functions
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            (!explicit || hosts.get(*i).copied().unwrap_or(false))
                && !is_spec_fn(&f.name)
                && f.foreign.is_none()
                && !(f.params.iter().any(|p| crate::llvmgen::is_collection(&p.ty)) || crate::llvmgen::is_collection(&f.ret))
                && !crate::llvmgen::has_ext_sig(f, &tt)
                // an inout entry has a binding only as an explicit-mode host entry (legacy inout fns stay internal)
                && (!crate::llvmgen::has_inout(f) || (explicit && hosts.get(*i).copied().unwrap_or(false)))
        })
        .map(|(_, f)| f)
        .collect();
    let returns_array = pubs.iter().any(|f| is_array_name(&f.ret) || f.ret == "str"); // G2 B2: str returns are counted too
    let mut decls: Vec<String> = Vec::new();
    for f in &pubs {
        if needs_mangling(&f.name) {
            return Err(format!(
                "llvmgen: function name {} needs mangling; the bindings use the raw name",
                f.name
            ));
        }
        let mut ps: Vec<&str> = Vec::new();
        for p in &f.params {
            if p.mode_pos.is_some() && (p.ty == "array_u8" || p.ty == "array_bool") {
                // inout [u8] / [bool]: *mut u8
                ps.extend(["*mut u8", "i64"]);
            } else if p.mode_pos.is_some() && p.ty == "array" {
                // inout [i64]: *mut i64
                ps.extend(["*mut i64", "i64"]);
            } else if p.ty == "array_u8" || p.ty == "array_bool" || p.ty == "str" {
                // G2 A5: bool is also *const u8 (G2 B2: so is str)
                ps.extend(["*const u8", "i64"]);
            } else if p.ty == "array" {
                ps.extend(["*const i64", "i64"]);
            } else if p.ty == "option" || p.ty == "result" {
                ps.extend(["i64", "i64"]);
            } else if is_agg(&p.ty) {
                ps.push("*const i64");
            } else {
                ps.push("i64");
            }
        }
        if f.ret == "array_u8" {
            ps.push("*mut Pruv__ArrayU8");
        } else if f.ret == "array_bool" {
            ps.push("*mut Pruv__ArrayBool"); // G2 A5
        } else if f.ret == "str" {
            ps.push("*mut Pruv__Str"); // G2 B2
        } else if f.ret == "array" {
            ps.push("*mut Pruv__Array");
        } else if is_agg(&f.ret) {
            ps.push("*mut i64");
        }
        let ret = if f.ret == "i64" || f.ret == "bool" {
            " -> i64"
        } else if f.ret == "option" || f.ret == "result" {
            " -> Pruv__Tagged"
        } else {
            ""
        };
        let plist: Vec<String> = ps.iter().enumerate().map(|(i, t)| format!("p{}: {}", i, t)).collect();
        decls.push(format!("    fn pruv_abi_{}({}){};", typed_abi_name(f), plist.join(", "), ret));
    }
    if pubs.iter().any(|f| f.ret == "array_bool") {
        // G2 A5: after all pruv_abi_* lines, before the marker line
        decls.push("    fn pruv_check_bool(p0: *const u8, p1: i64);".to_string());
    }
    if pubs.iter().any(|f| f.ret == "str") {
        // G2 B2: after the check_bool line, before the marker line
        decls.push("    fn pruv_check_utf8(p0: *const u8, p1: i64);".to_string());
    }
    if returns_array {
        decls.push("    fn pruv_runtime_rust_allocator_v2();".to_string());
    }
    o.push(ALLOW.to_string());
    o.push(format!("extern \"C\" {{\n{}\n}}", decls.join("\n")));
    for f in &pubs {
        let ps: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                if p.mode_pos.is_some() {
                    // inout: a mutable slice (checked to be [i64] / [u8] / [bool])
                    let el = match p.ty.as_str() {
                        "array_u8" => "u8",
                        "array_bool" => "bool",
                        _ => "i64",
                    };
                    format!("{}: &mut [{}]", p.name, el)
                } else {
                    format!("{}: {}", p.name, rust_ty(&p.ty, true))
                }
            })
            .collect();
        let ps = ps.join(", ");
        let ret = if f.ret == "unit" { String::new() } else { format!(" -> {}", rust_ty(&f.ret, false)) };
        let mut pre: Vec<String> = Vec::new();
        let mut args: Vec<String> = Vec::new();
        for p in &f.params {
            if p.mode_pos.is_some() && (p.ty == "array" || p.ty == "array_u8" || p.ty == "array_bool") {
                // inout slice: mutable pointer + length, no copy
                if p.ty == "array_bool" {
                    args.push(format!("{}.as_mut_ptr() as *mut u8", p.name));
                } else {
                    args.push(format!("{}.as_mut_ptr()", p.name));
                }
                args.push(format!("{}.len() as i64", p.name));
            } else if p.ty == "i64" {
                args.push(p.name.clone());
            } else if p.ty == "bool" {
                args.push(format!("{} as i64", p.name));
            } else if p.ty == "array_bool" {
                // G2 A5
                args.push(format!("{}.as_ptr() as *const u8", p.name));
                args.push(format!("{}.len() as i64", p.name));
            } else if p.ty == "str" {
                // G2 B2
                args.push(format!("{}.as_ptr()", p.name));
                args.push(format!("{}.len() as i64", p.name));
            } else if is_array_name(&p.ty) {
                args.push(format!("{}.as_ptr()", p.name));
                args.push(format!("{}.len() as i64", p.name));
            } else if is_agg(&p.ty) {
                let w = format!("pruv__w_{}", p.name);
                pre.push(format!("let mut {} = [0i64; {}];", w, words(&tt, &p.ty)?));
                let mut ctr = 0i64;
                pre.extend(rs_ser(&tt, &p.ty, &p.name, &w, 0, &mut ctr)?);
                args.push(format!("{}.as_ptr()", w));
            } else {
                if p.ty == "option" {
                    pre.push(format!(
                        "let (pruv__t_{}, pruv__v_{}): (i64, i64) = match {} {{ Some(v) => (1, v), None => (0, 0) }};",
                        p.name, p.name, p.name
                    ));
                } else {
                    pre.push(format!(
                        "let (pruv__t_{}, pruv__v_{}): (i64, i64) = match {} {{ Ok(v) => (1, v), Err(v) => (0, v) }};",
                        p.name, p.name, p.name
                    ));
                }
                args.push(format!("pruv__t_{}", p.name));
                args.push(format!("pruv__v_{}", p.name));
            }
        }
        let mut body: Vec<String> = Vec::new();
        if f.effects.iter().any(|e| e == "io") {
            body.push("::std::io::Write::flush(&mut ::std::io::stdout());".to_string());
        }
        if is_array_name(&f.ret) || f.ret == "str" {
            body.push(
                "let _ = ::std::hint::black_box(pruv_runtime_rust_allocator_v2 as unsafe extern \"C\" fn());".to_string(),
            );
        }
        body.extend(pre);
        let call = format!("pruv_abi_{}({})", typed_abi_name(f), args.join(", "));
        let mut args_out = args.clone();
        args_out.push("&mut pruv__out".to_string());
        let callo = format!("pruv_abi_{}({})", typed_abi_name(f), args_out.join(", "));
        if f.ret == "unit" {
            body.push(format!("{};", call));
        } else if f.ret == "i64" {
            body.push(call);
        } else if f.ret == "bool" {
            body.push(format!("{} != 0", call));
        } else if f.ret == "array_bool" {
            // G2 A5: after the check, take it as a Rust bool
            body.push("let mut pruv__out = Pruv__ArrayBool { data: ::std::ptr::null_mut(), len: 0 };".to_string());
            body.push(format!("{};", callo));
            body.push("pruv_check_bool(pruv__out.data, pruv__out.len);".to_string());
            body.push(
                "if pruv__out.len > 0 { ::std::vec::Vec::from_raw_parts(pruv__out.data as *mut bool, pruv__out.len as usize, \
                 pruv__out.len as usize) } else { ::std::vec::Vec::new() }"
                    .to_string(),
            );
        } else if f.ret == "str" {
            // G2 B2: after the check, take it as a Rust String
            body.push("let mut pruv__out = Pruv__Str { data: ::std::ptr::null_mut(), len: 0 };".to_string());
            body.push(format!("{};", callo));
            body.push("pruv_check_utf8(pruv__out.data, pruv__out.len);".to_string());
            body.push(
                "if pruv__out.len > 0 { ::std::string::String::from_utf8_unchecked(::std::vec::Vec::from_raw_parts(pruv__out.data, \
                 pruv__out.len as usize, pruv__out.len as usize)) } else { ::std::string::String::new() }"
                    .to_string(),
            );
        } else if is_array_name(&f.ret) {
            let out_ty = if f.ret == "array_u8" { "Pruv__ArrayU8" } else { "Pruv__Array" };
            body.push(format!("let mut pruv__out = {} {{ data: ::std::ptr::null_mut(), len: 0 }};", out_ty));
            body.push(format!("{};", callo));
            body.push(
                "if pruv__out.len > 0 { ::std::vec::Vec::from_raw_parts(pruv__out.data, pruv__out.len as usize, \
                 pruv__out.len as usize) } else { ::std::vec::Vec::new() }"
                    .to_string(),
            );
        } else if is_agg(&f.ret) {
            body.push(format!("let mut pruv__out = [0i64; {}];", words(&tt, &f.ret)?));
            let mut a2 = args.clone();
            a2.push("pruv__out.as_mut_ptr()".to_string());
            body.push(format!("pruv_abi_{}({});", typed_abi_name(f), a2.join(", ")));
            body.push(rs_de(&tt, &f.ret, "pruv__out", 0)?);
        } else {
            body.push(format!("let pruv__out = {};", call));
            if f.ret == "option" {
                body.push("if pruv__out.tag != 0 { Some(pruv__out.value) } else { None }".to_string());
            } else {
                body.push("if pruv__out.tag != 0 { Ok(pruv__out.value) } else { Err(pruv__out.value) }".to_string());
            }
        }
        o.push(ALLOW.to_string());
        let body_txt: Vec<String> = body.iter().map(|b| format!("        {}", b)).collect();
        o.push(format!(
            "#[inline]\n{}fn {}({}){} {{\n    unsafe {{\n{}\n    }}\n}}",
            if raw.is_some() { "pub(crate) " } else { "pub " },
            f.name,
            ps,
            ret,
            body_txt.join("\n")
        ));
        if raw.is_some() {
            raw_items.push((f.name.clone(), ps.clone(), ret.clone(), f.params.iter().map(|p| p.name.clone()).collect()));
        }
    }
    if let Some(mode) = &raw {
        // P09: manifest raw_entries; the wrappers above are pub(crate)
        // P10: "c" emits the "crate" module, then the pruv_c adapter (before the G6 shim)
        let is_c = mode == "c";
        o.push(crate::codegen::raw_module(if is_c { "crate" } else { mode }, &raw_items));
        if is_c {
            let entries: Vec<CEntry> = pubs.iter().filter_map(|f| c_entry(f)).collect();
            o.push(c_module(&entries));
        }
    }
    if let Some(sh) = shim {
        // G6 A3: the bridge shim module, exactly once, last (Python shim.rstrip("\n"))
        o.push(sh.trim_end_matches('\n').to_string());
    }
    let mut s = o.join("\n");
    s.push('\n');
    Ok(s)
}

/// P10: kind of a non-state parameter of a C entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CKind {
    I64,
    Bool,
    RoI64,
    RoU8,
    RoBool,
    Str,
    MutI64,
    MutU8,
    MutBool,
}

/// P10: return kind of a C entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CRet {
    I64,
    Bool,
    Unit,
}

/// P10: what `c_module` needs of one eligible host entry.
#[derive(Clone, Debug)]
pub struct CEntry {
    pub name: String,
    pub state_mut: bool,
    pub params: Vec<(String, CKind)>,
    pub ret: CRet,
    pub rebuild: bool,
}

/// P10: the C entry of a host function; None when it is not eligible (no pruv_h_ function).
pub fn c_entry(f: &Function) -> Option<CEntry> {
    let first = f.params.first()?;
    if first.ty != "array" {
        return None;
    }
    let mut params: Vec<(String, CKind)> = Vec::new();
    for p in &f.params[1..] {
        let inout = p.mode_pos.is_some();
        let k = match (p.ty.as_str(), inout) {
            ("i64", false) => CKind::I64,
            ("bool", false) => CKind::Bool,
            ("array", false) => CKind::RoI64,
            ("array_u8", false) => CKind::RoU8,
            ("array_bool", false) => CKind::RoBool,
            ("str", false) => CKind::Str,
            ("array", true) => CKind::MutI64,
            ("array_u8", true) => CKind::MutU8,
            ("array_bool", true) => CKind::MutBool,
            _ => return None,
        };
        params.push((p.name.clone(), k));
    }
    let ret = match f.ret.as_str() {
        "i64" => CRet::I64,
        "bool" => CRet::Bool,
        "unit" => CRet::Unit,
        _ => return None,
    };
    let rebuild = f.params.len() == 2
        && first.mode_pos.is_none()
        && ret == CRet::I64
        && params.len() == 1
        && params[0].1 == CKind::MutI64;
    Some(CEntry { name: f.name.clone(), state_mut: first.mode_pos.is_some(), params, ret, rebuild })
}

const C_HEAD: &str = r##"// P10: manifest raw_entries = "c": pruv_c is the C adapter. One opaque handle owns the resident state in a
// pruv_raw::PruvStore; every extern "C" function catches a Rust panic (no unwind crosses into C) and returns a status.
// A poisoned handle refuses every call but is still freed. Only the LLVM bindings carry this module.
#[allow(dead_code, unexpected_cfgs)]
pub mod pruv_c {
    //! Caller contract of every `pub unsafe extern "C" fn` below (the adapter cannot check what it cannot see):
    //! - the handle comes from pruv_handle_new and has not been freed;
    //! - no concurrent or reentrant use of one handle;
    //! - each (pointer, length) pair with length > 0 points to `length` initialized elements inside one live
    //!   allocation, readable (writable for an inout parameter) and not accessed by anyone else for the duration
    //!   of the call;
    //! - ret points to a writable i64.
    //!
    //! The adapter checks null, sign, alignment, size, address overflow and overlap among the call's buffers and
    //! the handle, but it cannot check allocation size or provenance.
    /// Statuses of every pruv_h_* / pruv_hr_* call: 0 ok, 1 poisoned (refused before any argument is read, nothing
    /// ran), 2 bad arguments (refused, nothing ran), 3 a Rust panic unwound to the adapter (if it happened inside the
    /// store it poisoned the handle). Checked failures inside LLVM-compiled code (range, arithmetic, requires) and G6
    /// host-callback failures end the process (exit 101); they never return a status.
    pub const PRUV_OK: i32 = 0;
    pub const PRUV_POISONED: i32 = 1;
    pub const PRUV_BAD_ARGS: i32 = 2;
    pub const PRUV_PANICKED: i32 = 3;
    /// Opaque to C (one pointer). Owns the resident state; the C side never sees the state buffer.
    pub struct PruvHandle {
        store: super::pruv_raw::PruvStore<::std::vec::Vec<i64>>,
    }
    // A C (pointer, length) pair as a slice: None for a negative or oversized length, or for a null or misaligned
    // pointer with a nonzero length, or whose end address overflows. Length 0 never reads the pointer.
    #[inline(always)]
    unsafe fn ro<'a, T>(p: *const T, n: i64) -> Option<&'a [T]> {
        if n == 0 {
            return Some(&[]);
        }
        if n < 0 || n as u64 > (isize::MAX as u64) / (::std::mem::size_of::<T>() as u64)
            || (p as usize).checked_add(n as usize * ::std::mem::size_of::<T>()).is_none() || p.is_null()
            || (p as usize) % ::std::mem::align_of::<T>() != 0 {
            return None;
        }
        Some(unsafe { ::std::slice::from_raw_parts(p, n as usize) })
    }
    #[inline(always)]
    unsafe fn mt<'a, T>(p: *mut T, n: i64) -> Option<&'a mut [T]> {
        if n == 0 {
            return Some(&mut []);
        }
        if n < 0 || n as u64 > (isize::MAX as u64) / (::std::mem::size_of::<T>() as u64)
            || (p as usize).checked_add(n as usize * ::std::mem::size_of::<T>()).is_none() || p.is_null()
            || (p as usize) % ::std::mem::align_of::<T>() != 0 {
            return None;
        }
        Some(unsafe { ::std::slice::from_raw_parts_mut(p, n as usize) })
    }
    // Byte range of a (pointer, length) pair; empty for a length <= 0.
    #[inline(always)]
    fn span<T>(p: *const T, n: i64) -> (usize, usize) {
        let s = p as usize;
        (s, s.saturating_add((n.max(0) as usize).saturating_mul(::std::mem::size_of::<T>())))
    }
    // Empty ranges never overlap.
    #[inline(always)]
    fn apart(a: (usize, usize), b: (usize, usize)) -> bool {
        !(a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1)
    }
    #[inline(always)]
    fn bools(s: &[u8]) -> Option<&[bool]> {
        if s.iter().all(|b| *b <= 1) { Some(unsafe { &*(s as *const [u8] as *const [bool]) }) } else { None }
    }
    #[inline(always)]
    fn bools_mut(s: &mut [u8]) -> Option<&mut [bool]> {
        if s.iter().all(|b| *b <= 1) { Some(unsafe { &mut *(s as *mut [u8] as *mut [bool]) }) } else { None }
    }
    // Runs one C call; a caught panic becomes PRUV_PANICKED (its payload is leaked, never dropped across C).
    #[inline(always)]
    fn guard(f: impl FnOnce() -> i32) -> i32 {
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(f)) {
            Ok(s) => s,
            Err(e) => {
                ::std::mem::forget(e);
                PRUV_PANICKED
            }
        }
    }
    /// A new handle whose state is `len` zero words (call the program's init entry next); null for a negative or
    /// oversized length. Free it with pruv_handle_free.
    #[no_mangle]
    pub extern "C" fn pruv_handle_new(len: i64) -> *mut PruvHandle {
        if len < 0 || len as u64 > (isize::MAX as u64) / 8 {
            return ::std::ptr::null_mut();
        }
        match ::std::panic::catch_unwind(|| {
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(PruvHandle {
                store: super::pruv_raw::PruvStore::new(::std::vec![0i64; len as usize]),
            }))
        }) {
            Ok(p) => p,
            Err(e) => {
                ::std::mem::forget(e);
                ::std::ptr::null_mut()
            }
        }
    }
    /// Frees a handle and its state, poisoned or not; null is ignored. The handle must come from pruv_handle_new and
    /// is never used again: without a registry a double free or a later call cannot be detected.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[no_mangle]
    pub unsafe extern "C" fn pruv_handle_free(h: *mut PruvHandle) {
        if h.is_null() {
            return;
        }
        let _ = guard(|| {
            drop(unsafe { ::std::boxed::Box::from_raw(h) });
            PRUV_OK
        });
    }
    /// 1 if a panic poisoned the handle, 0 if not, PRUV_BAD_ARGS for null.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[no_mangle]
    pub unsafe extern "C" fn pruv_handle_poisoned(h: *const PruvHandle) -> i32 {
        match unsafe { h.as_ref() } {
            Some(h) => h.store.is_poisoned() as i32,
            None => PRUV_BAD_ARGS,
        }
    }
    /// Probe builds only (rustc --cfg pruv_c_probe): panics inside the store with a payload whose Drop itself
    /// panics, as a failing call would; guard must never drop the payload.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[cfg(pruv_c_probe)]
    #[no_mangle]
    pub unsafe extern "C" fn pruv_handle_probe_panic(h: *mut PruvHandle) -> i32 {
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("pruv_c probe payload dropped")
            }
        }
        guard(|| {
            let hd = match unsafe { h.as_mut() } {
                Some(h) => h,
                None => return PRUV_BAD_ARGS,
            };
            match hd.store.with(|_| -> () { ::std::panic::panic_any(Bomb) }) {
                Ok(()) => PRUV_OK,
                Err(_) => PRUV_POISONED,
            }
        })
    }"##;

const C_RET_CHECK: [&str; 3] = [
    "            if ret.is_null() || (ret as usize) % ::std::mem::align_of::<i64>() != 0 || (ret as usize).checked_add(::std::mem::size_of::<i64>()).is_none() {",
    "                return PRUV_BAD_ARGS;",
    "            }",
];

fn c_kind_doc(k: &CKind) -> &'static str {
    match k {
        CKind::I64 => "i64",
        CKind::Bool => "bool, 0/1 as i64",
        CKind::RoI64 => "[i64] pointer, length",
        CKind::RoU8 => "[u8] pointer, length",
        CKind::RoBool => "[bool] pointer, length; bytes 0/1",
        CKind::Str => "str pointer, length; UTF-8",
        CKind::MutI64 => "inout [i64] pointer, length",
        CKind::MutU8 => "inout [u8] pointer, length",
        CKind::MutBool => "inout [bool] pointer, length; bytes 0/1",
    }
}

fn c_entry_text(e: &CEntry) -> String {
    let has_ret = e.ret != CRet::Unit;
    let mut l: Vec<String> = Vec::new();
    let mut doc = format!("    /// C entry for `{}`", e.name);
    if !e.params.is_empty() {
        let items: Vec<String> = e
            .params
            .iter()
            .enumerate()
            .map(|(i, (un, k))| format!("a{} = `{}` ({})", i, un, c_kind_doc(k)))
            .collect();
        doc.push_str(": ");
        doc.push_str(&items.join(", "));
    }
    doc.push_str("; the handle's state is its first argument.");
    if has_ret {
        doc.push_str(" On PRUV_OK the result is in *ret.");
    }
    l.push(doc);
    l.push("    /// # Safety".to_string());
    l.push("    /// The pruv_c caller contract (module docs) holds.".to_string());
    l.push("    #[no_mangle]".to_string());
    // no user parameter name is a Rust identifier below: the i-th parameter after the state is a{i}
    let params: Vec<(String, CKind)> = e.params.iter().enumerate().map(|(i, (_, k))| (format!("a{}", i), *k)).collect();
    let mut sig: Vec<String> = vec!["h: *mut PruvHandle".to_string()];
    for (n, k) in &params {
        match k {
            CKind::I64 | CKind::Bool => sig.push(format!("{}: i64", n)),
            CKind::RoI64 => sig.push(format!("{n}: *const i64, {n}_len: i64", n = n)),
            CKind::RoU8 | CKind::RoBool | CKind::Str => sig.push(format!("{n}: *const u8, {n}_len: i64", n = n)),
            CKind::MutI64 => sig.push(format!("{n}: *mut i64, {n}_len: i64", n = n)),
            CKind::MutU8 | CKind::MutBool => sig.push(format!("{n}: *mut u8, {n}_len: i64", n = n)),
        }
    }
    if has_ret {
        sig.push("ret: *mut i64".to_string());
    }
    l.push(format!("    pub unsafe extern \"C\" fn pruv_h_{}({}) -> i32 {{", e.name, sig.join(", ")));
    l.push("        guard(|| unsafe {".to_string());
    for s in [
        "            let hd = match h.as_mut() {",
        "                Some(h) => h,",
        "                None => return PRUV_BAD_ARGS,",
        "            };",
        "            if hd.store.is_poisoned() {",
        "                return PRUV_POISONED;",
        "            }",
    ] {
        l.push(s.to_string());
    }
    if has_ret {
        l.extend(C_RET_CHECK.iter().map(|s| s.to_string()));
    }
    for (n, k) in &params {
        if *k == CKind::Bool {
            l.push(format!("            if {n} != 0 && {n} != 1 {{", n = n));
            l.push("                return PRUV_BAD_ARGS;".to_string());
            l.push("            }".to_string());
        }
    }
    // overlap check: (local, span text, mutable); each byte range is computed once (P10.3: recomputing it per
    // pair cost ~1 ns a call).
    let mut b: Vec<(String, String, bool)> = Vec::new();
    for (n, k) in &params {
        match k {
            CKind::RoI64 | CKind::RoU8 | CKind::RoBool | CKind::Str => {
                b.push((format!("sp_{}", n), format!("span({n}, {n}_len)", n = n), false))
            }
            CKind::MutI64 | CKind::MutU8 | CKind::MutBool => {
                b.push((format!("sp_{}", n), format!("span({n}, {n}_len)", n = n), true))
            }
            _ => {}
        }
    }
    if has_ret {
        b.push(("sp_ret".to_string(), "span(ret, 1)".to_string(), true));
    }
    b.push(("sp_h".to_string(), "span(h as *const PruvHandle, 1)".to_string(), true));
    let mut pairs: Vec<String> = Vec::new();
    for i in 0..b.len() {
        for j in (i + 1)..b.len() {
            if b[i].2 || b[j].2 {
                pairs.push(format!("apart({}, {})", b[i].0, b[j].0));
            }
        }
    }
    if !pairs.is_empty() {
        let names: Vec<&str> = b.iter().map(|v| v.0.as_str()).collect();
        let spans: Vec<&str> = b.iter().map(|v| v.1.as_str()).collect();
        l.push(format!("            let ({}) = ({});", names.join(", "), spans.join(", ")));
        l.push(format!("            if !({}) {{", pairs.join(" && ")));
        l.push("                return PRUV_BAD_ARGS;".to_string());
        l.push("            }".to_string());
    }
    let mut args: Vec<String> = vec![if e.state_mut { "&mut st[..]".to_string() } else { "&st[..]".to_string() }];
    for (n, k) in &params {
        let expr = match k {
            CKind::I64 => {
                args.push(n.clone());
                continue;
            }
            CKind::Bool => {
                args.push(format!("{} != 0", n));
                continue;
            }
            CKind::RoI64 | CKind::RoU8 => format!("ro({n}, {n}_len)", n = n),
            CKind::Str => format!("ro({n}, {n}_len).and_then(|b| ::std::str::from_utf8(b).ok())", n = n),
            CKind::RoBool => format!("ro({n}, {n}_len).and_then(bools)", n = n),
            CKind::MutI64 | CKind::MutU8 => format!("mt({n}, {n}_len)", n = n),
            CKind::MutBool => format!("mt({n}, {n}_len).and_then(bools_mut)", n = n),
        };
        l.push(format!("            let {} = match {} {{", n, expr));
        l.push("                Some(v) => v,".to_string());
        l.push("                None => return PRUV_BAD_ARGS,".to_string());
        l.push("            };".to_string());
        args.push(n.clone());
    }
    l.push(format!("            match hd.store.with(|st| super::{}({})) {{", e.name, args.join(", ")));
    match e.ret {
        CRet::Unit => l.push("                Ok(()) => PRUV_OK,".to_string()),
        r => {
            l.push("                Ok(v) => {".to_string());
            l.push(if r == CRet::Bool { "                    *ret = v as i64;" } else { "                    *ret = v;" }.to_string());
            l.push("                    PRUV_OK".to_string());
            l.push("                }".to_string());
        }
    }
    l.push("                Err(_) => PRUV_POISONED,".to_string());
    l.push("            }".to_string());
    l.push("        })".to_string());
    l.push("    }".to_string());
    l.join("\n")
}

fn c_rebuild_text(name: &str) -> String {
    let mut l: Vec<String> = Vec::new();
    l.push(format!(
        "    /// C rebuild for `{}`: runs it from the handle's state into a fresh zeroed buffer of new_len words and, when",
        name
    ));
    l.push("    /// it returns 0, makes that buffer the handle's state (the old one is freed); otherwise the state is unchanged.".to_string());
    l.push("    /// # Safety".to_string());
    l.push("    /// The pruv_c caller contract (module docs) holds.".to_string());
    l.push("    #[no_mangle]".to_string());
    l.push(format!("    pub unsafe extern \"C\" fn pruv_hr_{}(h: *mut PruvHandle, new_len: i64, ret: *mut i64) -> i32 {{", name));
    l.push("        guard(|| unsafe {".to_string());
    for s in [
        "            let hd = match h.as_mut() {",
        "                Some(h) => h,",
        "                None => return PRUV_BAD_ARGS,",
        "            };",
        "            if hd.store.is_poisoned() {",
        "                return PRUV_POISONED;",
        "            }",
    ] {
        l.push(s.to_string());
    }
    l.extend(C_RET_CHECK.iter().map(|s| s.to_string()));
    for s in [
        "            if new_len < 0 || new_len as u64 > (isize::MAX as u64) / 8 {",
        "                return PRUV_BAD_ARGS;",
        "            }",
        "            if !apart(span(ret, 1), span(h as *const PruvHandle, 1)) {",
        "                return PRUV_BAD_ARGS;",
        "            }",
        "            match hd.store.with(|st| {",
        "                let mut next = ::std::vec![0i64; new_len as usize];",
    ] {
        l.push(s.to_string());
    }
    l.push(format!("                let r = super::{}(&st[..], &mut next[..]);", name));
    for s in [
        "                if r == 0 {",
        "                    *st = next;",
        "                }",
        "                r",
        "            }) {",
        "                Ok(v) => {",
        "                    *ret = v;",
        "                    PRUV_OK",
        "                }",
        "                Err(_) => PRUV_POISONED,",
        "            }",
        "        })",
        "    }",
    ] {
        l.push(s.to_string());
    }
    l.join("\n")
}

/// P10: the `pruv_c` module text (manifest raw_entries = "c"), without a trailing newline.
pub fn c_module(entries: &[CEntry]) -> String {
    let mut parts: Vec<String> = vec![C_HEAD.to_string()];
    for e in entries {
        parts.push(c_entry_text(e));
        if e.rebuild {
            parts.push(c_rebuild_text(&e.name));
        }
    }
    parts.push("}".to_string());
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(name: &str, params: Vec<(&str, &str)>, ret: &str) -> Function {
        Function {
            line: 1, col: 1, name: name.into(),
            params: params.into_iter().map(|(n, t)| Param { name: n.into(), ty: t.into(), refine: None, take: false, mode_pos: None }).collect(),
            ret: ret.into(), effects: vec![], requires: vec![], ensures: vec![], body: vec![], ret_refine: None, tparams: vec![],
            foreign: None, cost: None, decreases: None, small_bound: None, x_auto: None,
        }
    }

    fn inout(mut f: Function, idx: usize) -> Function {
        f.params[idx].mode_pos = Some((1, 1));
        f
    }

    #[test]
    fn c_entry_eligibility() {
        let op = inout(inout(func("m4_op", vec![("st", "array"), ("op", "array"), ("out", "array")], "i64"), 0), 2);
        let e = c_entry(&op).unwrap();
        assert!(e.state_mut && !e.rebuild);
        assert_eq!(e.params, vec![("op".to_string(), CKind::RoI64), ("out".to_string(), CKind::MutI64)]);
        let grow = inout(func("m4_grow", vec![("prev", "array"), ("next", "array")], "i64"), 1);
        assert!(c_entry(&grow).unwrap().rebuild);
        assert!(c_entry(&func("f", vec![("x", "i64")], "i64")).is_none());
        assert!(c_entry(&func("f", vec![("st", "array")], "option")).is_none());
        assert!(c_entry(&func("f", vec![("st", "array"), ("o", "option")], "i64")).is_none());
    }

    #[test]
    fn c_module_entry_text() {
        let op = inout(inout(func("m4_op", vec![("st", "array"), ("op", "array"), ("out", "array")], "i64"), 0), 2);
        let t = c_module(&[c_entry(&op).unwrap()]);
        assert!(t.starts_with("// P10: manifest raw_entries = \"c\": pruv_c is the C adapter."));
        assert!(t.ends_with("    }\n}"));
        assert!(t.contains("    /// C entry for `m4_op`: a0 = `op` ([i64] pointer, length), a1 = `out` (inout [i64] pointer, length); the handle's state is its first argument. On PRUV_OK the result is in *ret.\n    /// # Safety\n    /// The pruv_c caller contract (module docs) holds.\n    #[no_mangle]\n    pub unsafe extern \"C\" fn pruv_h_m4_op(h: *mut PruvHandle, a0: *const i64, a0_len: i64, a1: *mut i64, a1_len: i64, ret: *mut i64) -> i32 {\n        guard(|| unsafe {\n            let hd = match h.as_mut() {\n                Some(h) => h,\n                None => return PRUV_BAD_ARGS,\n            };\n            if hd.store.is_poisoned() {\n                return PRUV_POISONED;\n            }\n            if ret.is_null()"));
        assert!(t.contains("            let (sp_a0, sp_a1, sp_ret, sp_h) = (span(a0, a0_len), span(a1, a1_len), span(ret, 1), span(h as *const PruvHandle, 1));\n            if !(apart(sp_a0, sp_a1) && apart(sp_a0, sp_ret) && apart(sp_a0, sp_h) && apart(sp_a1, sp_ret) && apart(sp_a1, sp_h) && apart(sp_ret, sp_h)) {\n"));
        assert!(t.contains("            match hd.store.with(|st| super::m4_op(&mut st[..], a0, a1)) {\n                Ok(v) => {\n                    *ret = v;\n                    PRUV_OK\n                }\n                Err(_) => PRUV_POISONED,\n            }\n        })\n    }\n}"));
        assert!(!t.contains("fn pruv_hr_"));
        assert!(!t.contains("op_len") && !t.contains("out_len") && !t.contains("fn pruv_h_m4_op(h: *mut PruvHandle, op"));
    }

    #[test]
    fn c_module_bool_unit_and_rebuild() {
        let f = func("put", vec![("st", "array"), ("b", "bool"), ("s", "str")], "unit");
        let t = c_module(&[c_entry(&f).unwrap()]);
        assert!(t.contains("    /// C entry for `put`: a0 = `b` (bool, 0/1 as i64), a1 = `s` (str pointer, length; UTF-8); the handle's state is its first argument.\n"));
        assert!(t.contains("fn pruv_h_put(h: *mut PruvHandle, a0: i64, a1: *const u8, a1_len: i64) -> i32 {"));
        assert!(t.contains("            if hd.store.is_poisoned() {\n                return PRUV_POISONED;\n            }\n            if a0 != 0 && a0 != 1 {\n                return PRUV_BAD_ARGS;\n            }\n"));
        assert!(t.contains("            let (sp_a1, sp_h) = (span(a1, a1_len), span(h as *const PruvHandle, 1));\n            if !(apart(sp_a1, sp_h)) {\n"));
        assert!(t.contains("            let a1 = match ro(a1, a1_len).and_then(|b| ::std::str::from_utf8(b).ok()) {"));
        assert!(t.contains("super::put(&st[..], a0 != 0, a1)) {\n                Ok(()) => PRUV_OK,\n"));
        assert!(!t.contains("apart(sp_a1, sp_a"));
        let g = inout(func("m4_grow", vec![("prev", "array"), ("next", "array")], "i64"), 1);
        let t = c_module(&[c_entry(&g).unwrap()]);
        assert!(t.contains("pruv_hr_m4_grow(h: *mut PruvHandle, new_len: i64, ret: *mut i64) -> i32 {"));
        assert!(t.contains("let r = super::m4_grow(&st[..], &mut next[..]);"));
        assert!(t.contains("            if hd.store.is_poisoned() {\n                return PRUV_POISONED;\n            }\n            if ret.is_null()"));
        assert!(t.contains("            }\n            if !apart(span(ret, 1), span(h as *const PruvHandle, 1)) {\n                return PRUV_BAD_ARGS;\n            }\n            match hd.store.with(|st| {\n"));
    }

    #[test]
    fn raw_entries_c_order() {
        let mut prog = Program { functions: vec![func("f", vec![("x", "i64")], "i64")], types: vec![], refined: false, g9: None, explicit: None, uses_inout: None, raw_entries: Some("c".into()) };
        // not explicit: raw_entries is ignored, so no pruv_c
        assert!(!gen_bindings(&prog).unwrap().contains("pub mod pruv_c"));
        prog.raw_entries = None;
        assert!(!gen_bindings(&prog).unwrap().contains("pub mod pruv_c"));
    }

    #[test]
    fn simple_function() {
        let prog = Program { functions: vec![func("f", vec![("x", "i64"), ("a", "array")], "bool")], types: vec![], refined: false, g9: None, explicit: None, uses_inout: None, raw_entries: None };
        let s = gen_bindings(&prog).unwrap();
        assert!(s.contains("    fn pruv_abi_f(p0: i64, p1: *const i64, p2: i64) -> i64;"));
        assert!(s.contains("pub fn f(x: i64, a: &[i64]) -> bool {"));
        assert!(s.contains("        pruv_abi_f(x, a.as_ptr(), a.len() as i64) != 0"));
        assert!(s.ends_with("    }\n}\n"));
    }

    #[test]
    fn mangling_error() {
        let prog = Program { functions: vec![func("f\u{e9}", vec![], "unit")], types: vec![], refined: false, g9: None, explicit: None, uses_inout: None, raw_entries: None };
        assert_eq!(
            gen_bindings(&prog).unwrap_err(),
            "llvmgen: function name f\u{e9} needs mangling; the bindings use the raw name"
        );
    }

    #[test]
    fn struct_param_ser() {
        let st = TypeDecl::Struct(StructDecl {
            line: 1, col: 1, name: "P".into(),
            fields: vec![
                FieldDecl { line: 1, col: 1, name: "x".into(), ty: "i64".into() },
                FieldDecl { line: 1, col: 1, name: "b".into(), ty: "bool".into() },
            ],
            tparams: vec![],
        });
        let tt = TypeTable::new(&[st]);
        let mut c = 0;
        assert_eq!(
            rs_ser(&tt, "P", "p", "w", 0, &mut c).unwrap(),
            vec!["w[0] = p.x;".to_string(), "w[1] = (p.b) as i64;".to_string()]
        );
        assert_eq!(rs_de(&tt, "P", "o", 0).unwrap(), "P { x: o[0], b: (o[1] != 0) }");
    }
}

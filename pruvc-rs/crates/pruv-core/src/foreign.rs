//! G6 A0 (docs/G6_SPEC.md section 10; counterpart of foreign.py): the fixed standard catalog, validation of
//! `trusted rust fn` declarations, the ownership/liveness checker for owned bindings, and the A0 guard `first_g6_use`.
//!
//! Everything here runs after (or inside) type checking and only looks at typed ASTs.
//! Messages, fixes and positions are those of foreign.py character for character.

use crate::ast::*;
use crate::diagnostic::PruvError;
use crate::gtype::{fn_owner, format_type, key_is_collection, mk_array, mk_option, mk_param, GType};
use crate::refine::kids as expr_kids;
use std::collections::{HashMap, HashSet};

type Res<T> = Result<T, PruvError>;

fn err(line: i64, col: i64, code: &str, msg: impl Into<String>, fix: impl Into<String>) -> PruvError {
    PruvError::new("type", line, col, code, msg, fix)
}

// ---------------------------------------------------------------------------
// catalog (section 10.3): pseudo declarations without bodies, typed through the ordinary generic call path
// ---------------------------------------------------------------------------

/// One parameter of a catalog function.
#[derive(Debug, Clone)]
pub struct CatParam {
    pub name: &'static str,
    pub ty: GType,
    pub take: bool,
    pub inout: bool,
}

/// A catalog function: generic (explicit type arguments are required), effects as declared.
#[derive(Debug, Clone)]
pub struct CatFn {
    pub requires: Vec<Expr>,
    pub ensures: Vec<Expr>,
    pub params: Vec<CatParam>,
    pub ret: GType,
    pub effects: Vec<String>,
    pub tparams: Vec<TParam>,
}

/// CATALOG_NAMES of foreign.py (insertion order).
pub const CATALOG_NAMES: [&str; 111] = [
    "vec_new",
    "vec_push",
    "vec_get",
    "map_new",
    "map_get",
    "map_get_mut",
    "map_remove",
    "map_take",
    "vec_clear",
    "vec_reserve",
    "vec_extend_from_slice",
    "string_eq",
    "vec_at_mut",
    "map_insert",
    "string_new",
    "string_from",
    "string_push_str",
    "string_push_char",
    "string_byte",
    "string_to_str",
    "string_into_str",
    "file_open_read",
    "file_create",
    "file_read_all",
    "file_write_str",
    "file_write_bytes",
    "file_close",
    "file_ok",
    "file_readable",
    "file_writable",
    "file_size",
    "file_byte",
    "proc_args_new",
    "proc_args_push",
    "proc_args_len",
    "proc_spawn",
    "proc_spawn_env",
    "proc_write_str",
    "proc_write_bytes",
    "proc_close_stdin",
    "proc_read",
    "proc_read_all",
    "proc_bytes",
    "proc_wait",
    "proc_kill",
    "proc_ok",
    "proc_error",
    "proc_stdin_open",
    "proc_eof",
    "proc_size",
    "proc_byte",
    "ev_new",
    "ev_close",
    "ev_add_socket",
    "ev_add_listener",
    "ev_add_process",
    "ev_add_timer",
    "ev_wait",
    "ev_read",
    "ev_read_into",
    "ev_write",
    "ev_shutdown_write",
    "ev_accept",
    "ev_proc_read",
    "ev_proc_read_into",
    "ev_proc_write",
    "ev_proc_close_stdin",
    "ev_proc_wait",
    "ev_proc_wait_for",
    "ev_proc_write_for",
    "ev_remove",
    "ev_bytes",
    "ev_ok",
    "ev_error",
    "ev_cap",
    "ev_count",
    "ev_last",
    "ev_kind",
    "ev_eof",
    "ev_write_open",
    "ev_event_slot",
    "ev_event_kind",
    "ev_size",
    "ev_byte",
    "ev_exit",
    "net_listen",
    "net_accept",
    "net_listener_close",
    "net_connect",
    "net_write_str",
    "net_write_bytes",
    "net_shutdown_write",
    "net_set_timeout",
    "net_read",
    "net_read_all",
    "net_bytes",
    "net_close",
    "net_ok",
    "net_error",
    "net_write_open",
    "net_eof",
    "net_size",
    "net_byte",
    "net_listener_ok",
    "net_listener_error",
    "net_listener_port",
    "map_lookup",
    "map_updated",
    "string_with_capacity",
    "string_reserve",
    "string_prepend_char",
];

/// CATALOG_CONTRACT_ONLY of foreign.py.
pub const CATALOG_CONTRACT_ONLY: [&str; 2] = ["map_lookup", "map_updated"];

/// FILE_OBSERVERS of foreign.py (G10 A0): catalog reads that are also allowed in contracts.
pub const FILE_OBSERVERS: [&str; 5] = ["file_ok", "file_readable", "file_writable", "file_size", "file_byte"];

pub fn is_file_observer(name: &str) -> bool {
    FILE_OBSERVERS.contains(&name)
}

/// PROC_OBSERVERS of foreign.py (G10 proc A0): proc catalog reads allowed in contracts; every other proc_* operation
/// (effects, the allocating proc_bytes) is type/foreign_contract there.
pub const PROC_OBSERVERS: [&str; 7] =
    ["proc_args_len", "proc_ok", "proc_error", "proc_stdin_open", "proc_eof", "proc_size", "proc_byte"];

pub fn is_proc_observer(name: &str) -> bool {
    PROC_OBSERVERS.contains(&name)
}

pub fn is_proc_op(name: &str) -> bool {
    name.starts_with("proc_") && CATALOG_NAMES.contains(&name)
}

/// NET_OBSERVERS of foreign.py (G10 net A0): net catalog reads allowed in contracts; every other net_* operation
/// (effects, the allocating net_bytes) is type/foreign_contract there.
pub const NET_OBSERVERS: [&str; 9] = [
    "net_ok",
    "net_error",
    "net_write_open",
    "net_eof",
    "net_size",
    "net_byte",
    "net_listener_ok",
    "net_listener_error",
    "net_listener_port",
];

pub const ASYNC_OBSERVERS: [&str; 13] = ["ev_ok","ev_error","ev_cap","ev_count","ev_last","ev_kind","ev_eof","ev_write_open","ev_event_slot","ev_event_kind","ev_size","ev_byte","ev_exit"];
pub const ASYNC_OPS: [&str; 34] = ["ev_new","ev_close","ev_add_socket","ev_add_listener","ev_add_process","ev_add_timer","ev_wait","ev_read","ev_read_into","ev_write","ev_shutdown_write","ev_accept","ev_proc_read","ev_proc_read_into","ev_proc_write","ev_proc_close_stdin","ev_proc_wait","ev_proc_wait_for","ev_proc_write_for","ev_remove","ev_bytes","ev_ok","ev_error","ev_cap","ev_count","ev_last","ev_kind","ev_eof","ev_write_open","ev_event_slot","ev_event_kind","ev_size","ev_byte","ev_exit"];
pub fn is_async_observer(name: &str) -> bool { ASYNC_OBSERVERS.contains(&name) }
pub fn is_async_op(name: &str) -> bool { ASYNC_OPS.contains(&name) }

pub fn is_net_observer(name: &str) -> bool {
    NET_OBSERVERS.contains(&name)
}

pub fn is_net_op(name: &str) -> bool {
    name.starts_with("net_") && CATALOG_NAMES.contains(&name)
}

pub fn is_catalog(name: &str) -> bool {
    CATALOG_NAMES.contains(&name)
}

pub fn is_catalog_contract_only(name: &str) -> bool {
    CATALOG_CONTRACT_ONLY.contains(&name)
}

fn p(name: &'static str, ty: GType, take: bool) -> CatParam {
    CatParam { name, ty, take, inout: false }
}

fn inout(name: &'static str, ty: GType) -> CatParam {
    CatParam { inout: true, ..p(name, ty, false) }
}

fn vec_of(t: GType) -> GType {
    GType::new("vec", vec![t])
}

fn map_of(k: GType, v: GType) -> GType {
    GType::new("hashmap", vec![k, v])
}

fn string_t() -> GType {
    GType::new("string", Vec::new())
}

fn file_t() -> GType {
    GType::new("file", Vec::new())
}

fn process_t() -> GType {
    GType::new("process", Vec::new())
}

fn process_args_t() -> GType {
    GType::new("process_args", Vec::new())
}

fn socket_t() -> GType {
    GType::new("socket", Vec::new())
}

fn loop_t() -> GType { GType::new("loop", vec![]) }

fn listener_t() -> GType {
    GType::new("listener", Vec::new())
}

fn tp(name: &str, bound: Option<&str>) -> TParam {
    TParam { line: 0, col: 0, name: name.to_string(), bound: bound.map(|b| b.to_string()) }
}

/// The catalog declaration of `name`, or None when `name` is not a catalog function.
pub fn catalog_fn(name: &str) -> Option<CatFn> {
    let owner = fn_owner(name);
    // _T = [("T", None)] and _KV = [("K", "Element"), ("V", None)]: parameters are identified by (owner, index)
    let t = || mk_param(&owner, 0, "T");
    let k = || mk_param(&owner, 0, "K");
    let v = || mk_param(&owner, 1, "V");
    let alloc = || vec!["alloc".to_string()];
    let fs = || vec!["fs".to_string()];
    let proc_alloc = || vec!["proc".to_string(), "alloc".to_string()];
    let net_alloc = || vec!["net".to_string(), "alloc".to_string()];
    let tp_t = || vec![tp("T", None)];
    let tp_kv = || vec![tp("K", Some("Element")), tp("V", None)];
    let mut f = match name {
        "vec_new" => CatFn { requires: vec![], ensures: vec![], params: vec![], ret: vec_of(t()), effects: alloc(), tparams: tp_t() },
        "vec_push" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), true), p("x", t(), false)],
            ret: vec_of(t()),
            effects: alloc(),
            tparams: tp_t(),
        },
        "vec_get" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), false), p("i", GType::i64(), false)],
            ret: t(),
            effects: vec![],
            tparams: tp_t(),
        },
        "map_new" => CatFn { requires: vec![], ensures: vec![], params: vec![], ret: map_of(k(), v()), effects: alloc(), tparams: tp_kv() },
        "map_get" | "map_get_mut" | "map_take" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("m", map_of(k(), v()), false), p("k", k(), false)],
            ret: mk_option(v()),
            effects: vec![],
            tparams: tp_kv(),
        },
        "map_remove" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("m", map_of(k(), v()), false), p("k", k(), false)],
            ret: GType::bool(), effects: vec![], tparams: tp_kv(),
        },
        "vec_clear" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), false)], ret: GType::unit(),
            effects: vec![], tparams: tp_t(),
        },
        "vec_reserve" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), false), p("n", GType::i64(), false)],
            ret: GType::unit(), effects: alloc(), tparams: tp_t(),
        },
        "vec_extend_from_slice" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), false), p("s", mk_array(t()), false)],
            ret: GType::unit(), effects: alloc(), tparams: tp_t(),
        },
        "string_eq" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("a", GType::str(), false), p("b", GType::str(), false)],
            ret: GType::bool(), effects: vec![], tparams: vec![],
        },
        "vec_at_mut" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("v", vec_of(t()), false), p("i", GType::i64(), false)],
            ret: mk_option(t()), effects: vec![], tparams: tp_t(),
        },
        "map_insert" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("m", map_of(k(), v()), true), p("k", k(), false), p("v", v(), false)],
            ret: map_of(k(), v()),
            effects: alloc(),
            tparams: tp_kv(),
        },
        "string_new" => CatFn { requires: vec![], ensures: vec![], params: vec![], ret: string_t(), effects: alloc(), tparams: vec![] },
        "string_from" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("text", GType::str(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_push_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), true), p("text", GType::str(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_push_char" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), true), p("c", GType::i64(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_with_capacity" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("n", GType::i64(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_reserve" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), true), p("n", GType::i64(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_prepend_char" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), true), p("c", GType::i64(), false)],
            ret: string_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "string_byte" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), false), p("i", GType::i64(), false)],
            ret: GType::i64(),
            effects: vec![],
            tparams: vec![],
        },
        "string_to_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), false)],
            ret: GType::str(),
            effects: alloc(),
            tparams: vec![],
        },
        // L-gaps 3: consume the String and hand its buffer over as the str (no copy)
        "string_into_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", string_t(), true)],
            ret: GType::str(),
            effects: vec![],
            tparams: vec![],
        },
        // G10 A0: signatures/effects; typed A1 contracts are defined below.
        "file_open_read" | "file_create" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("path", GType::str(), false)],
            ret: file_t(),
            effects: fs(),
            tparams: vec![],
        },
        "file_read_all" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("f", file_t(), true)],
            ret: file_t(),
            effects: vec!["fs".to_string(), "alloc".to_string()],
            tparams: vec![],
        },
        "file_write_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("f", file_t(), true), p("text", GType::str(), false)],
            ret: file_t(),
            effects: fs(),
            tparams: vec![],
        },
        "file_write_bytes" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("f", file_t(), true), p("data", GType::array_u8(), false)],
            ret: file_t(),
            effects: fs(),
            tparams: vec![],
        },
        "file_close" => CatFn { requires: vec![], ensures: vec![], params: vec![p("f", file_t(), true)], ret: GType::bool(), effects: fs(), tparams: vec![] },
        "file_ok" | "file_readable" | "file_writable" => {
            CatFn { requires: vec![], ensures: vec![], params: vec![p("f", file_t(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] }
        }
        "file_size" => CatFn { requires: vec![], ensures: vec![], params: vec![p("f", file_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "file_byte" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("f", file_t(), false), p("i", GType::i64(), false)],
            ret: GType::i64(),
            effects: vec![],
            tparams: vec![],
        },
        // G10 proc: signatures and effects. Since A1 the section 2.3 / 2.4 / 4 contracts (requires, fresh result,
        // well-formedness, ensures, preservation facts) of every proc_* operation are encoded by the verifier
        // (pruv-proof symbolic.rs proc_call, twin of verify.py FnVerifier.proc_call): the Process record and its
        // byte-map equalities are not expressible as typed contract ASTs. Both backends lower Process
        // programs (A2 Rust, A3 LLVM).
        "proc_args_new" => CatFn { requires: vec![], ensures: vec![], params: vec![], ret: process_args_t(), effects: alloc(), tparams: vec![] },
        "proc_args_push" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("a", process_args_t(), true), p("arg", GType::str(), false)],
            ret: process_args_t(),
            effects: alloc(),
            tparams: vec![],
        },
        "proc_args_len" => CatFn { requires: vec![], ensures: vec![], params: vec![p("a", process_args_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "proc_spawn" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("program", GType::str(), false), p("args", process_args_t(), false), p("cwd", GType::str(), false)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_spawn_env" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("program", GType::str(), false), p("args", process_args_t(), false), p("cwd", GType::str(), false),
                         p("env_names", process_args_t(), false), p("env_values", process_args_t(), false)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_write_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), true), p("text", GType::str(), false)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_write_bytes" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), true), p("data", GType::array_u8(), false)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_close_stdin" | "proc_read_all" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), true)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_read" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), true), p("stream", GType::i64(), false), p("max", GType::i64(), false)],
            ret: process_t(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_bytes" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), false), p("stream", GType::i64(), false)],
            ret: GType::array_u8(),
            effects: alloc(),
            tparams: vec![],
        },
        "proc_wait" | "proc_kill" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), true)],
            ret: GType::result(),
            effects: proc_alloc(),
            tparams: vec![],
        },
        "proc_ok" | "proc_stdin_open" => CatFn { requires: vec![], ensures: vec![], params: vec![p("p", process_t(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "proc_error" => CatFn { requires: vec![], ensures: vec![], params: vec![p("p", process_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "proc_eof" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), false), p("stream", GType::i64(), false)],
            ret: GType::bool(),
            effects: vec![],
            tparams: vec![],
        },
        "proc_size" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), false), p("stream", GType::i64(), false)],
            ret: GType::i64(),
            effects: vec![],
            tparams: vec![],
        },
        "proc_byte" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("p", process_t(), false), p("stream", GType::i64(), false), p("i", GType::i64(), false)],
            ret: GType::i64(),
            effects: vec![],
            tparams: vec![],
        },
        // G10 net (docs/G10_NET_SPEC.md 2.3): signatures and effects. Since A1 the section 2.3 / 2.4 / 4 contracts are
        // encoded for every net_* operation by symbolic.rs `net_call` (pruv-proof). Both backends lower Socket / Listener
        // (A2 Rust source backend, codegen.rs NET_PRELUDE; A3 LLVM backend through the Rust shim).

        // G10 async A1: catalog contracts are encoded in pruv-proof symbolic.rs.
        "ev_new" => CatFn { requires: vec![], ensures: vec![], params: vec![p("cap", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_close" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true)], ret: GType::new("result", vec![]), effects: vec!["async".to_string(), "net".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_add_socket" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("s", socket_t(), true)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_add_listener" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("x", listener_t(), true)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_add_process" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("p", process_t(), true)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_add_timer" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("ms", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_wait" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("timeout_ms", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_read" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("max", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_read_into" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), inout("dst", GType::new("array_u8", vec![])), inout("n", GType::i64())], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_write" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("data", GType::new("array_u8", vec![]), false)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_shutdown_write" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_accept" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_read" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("stream", GType::i64(), false), p("max", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_read_into" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("stream", GType::i64(), false), inout("dst", GType::new("array_u8", vec![])), inout("n", GType::i64())], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_write" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("data", GType::new("array_u8", vec![]), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_close_stdin" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_wait" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_wait_for" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("timeout_ms", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_proc_write_for" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false), p("data", GType::new("array_u8", vec![]), false), p("timeout_ms", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_remove" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), true), p("slot", GType::i64(), false)], ret: loop_t(), effects: vec!["async".to_string(), "net".to_string(), "proc".to_string(), "alloc".to_string()], tparams: vec![] },
        "ev_bytes" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::new("array_u8", vec![]), effects: vec!["alloc".to_string()], tparams: vec![] },
        "ev_ok" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "ev_error" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_cap" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_count" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_last" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_kind" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false), p("slot", GType::i64(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_eof" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false), p("slot", GType::i64(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "ev_write_open" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false), p("slot", GType::i64(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "ev_event_slot" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_event_kind" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_size" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_byte" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false), p("i", GType::i64(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "ev_exit" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", loop_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "net_listen" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("host", GType::str(), false), p("port", GType::i64(), false)],
            ret: listener_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_accept" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("l", listener_t(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_listener_close" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("l", listener_t(), true)],
            ret: GType::result(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_connect" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("host", GType::str(), false), p("port", GType::i64(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_write_str" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true), p("text", GType::str(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_write_bytes" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true), p("data", GType::array_u8(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_shutdown_write" | "net_read_all" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_set_timeout" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true), p("ms", GType::i64(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_read" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true), p("max", GType::i64(), false)],
            ret: socket_t(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_bytes" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), false)],
            ret: GType::array_u8(),
            effects: alloc(),
            tparams: vec![],
        },
        "net_close" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), true)],
            ret: GType::result(),
            effects: net_alloc(),
            tparams: vec![],
        },
        "net_ok" | "net_write_open" | "net_eof" => CatFn { requires: vec![], ensures: vec![], params: vec![p("s", socket_t(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "net_error" | "net_size" => CatFn { requires: vec![], ensures: vec![], params: vec![p("s", socket_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        "net_byte" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("s", socket_t(), false), p("i", GType::i64(), false)],
            ret: GType::i64(),
            effects: vec![],
            tparams: vec![],
        },
        "net_listener_ok" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", listener_t(), false)], ret: GType::bool(), effects: vec![], tparams: vec![] },
        "net_listener_error" | "net_listener_port" => CatFn { requires: vec![], ensures: vec![], params: vec![p("l", listener_t(), false)], ret: GType::i64(), effects: vec![], tparams: vec![] },
        // contract-only observers
        "map_lookup" => CatFn { requires: vec![], ensures: vec![],
            params: vec![p("m", map_of(k(), v()), false), p("k", k(), false)],
            ret: mk_option(v()),
            effects: vec![],
            tparams: tp_kv(),
        },
        "map_updated" => CatFn { requires: vec![], ensures: vec![],
            params: vec![
                p("after", map_of(k(), v()), false),
                p("before", map_of(k(), v()), false),
                p("k", k(), false),
                p("v", v(), false),
            ],
            ret: GType::bool(),
            effects: vec![],
            tparams: tp_kv(),
        },
        _ => return None,
    };
    (f.requires, f.ensures) = file_contracts(name);
    Some(f)
}

/// G10 A1: Python _file_contracts, preserving typed AST shapes, positions and order.
fn file_contracts(name: &str) -> (Vec<Expr>, Vec<Expr>) {
    fn var(n: &str, t: &str) -> Expr { Expr::Var(Var { line: 0, col: 0, name: n.into(), ty: Some(t.into()) }) }
    fn obs(n: &str, v: Expr) -> Expr {
        Expr::Call(Call { line: 0, col: 0, name: format!("file_{}", n), args: vec![v],
            ty: Some(if n == "size" { "i64" } else { "bool" }.into()), targs: None, inst: None, old: None, opaque_c: false })
    }
    fn bin(op: &str, a: Expr, b: Expr) -> Expr {
        Expr::Binary(Binary { line: 0, col: 0, op: op.into(), left: Box::new(a), right: Box::new(b), ty: Some("bool".into()) })
    }
    fn neg(a: Expr) -> Expr { Expr::Unary(Unary { line: 0, col: 0, op: "!".into(), operand: Box::new(a), ty: Some("bool".into()) }) }
    fn int(n: &str) -> Expr { Expr::IntLit(IntLit { line: 0, col: 0, value: n.into(), ty: Some("i64".into()), kind: Default::default() }) }
    let result = var("result", "file");
    let f = var("f", "file");
    let ok = obs("ok", result.clone());
    let size = obs("size", result.clone());
    let read = vec![bin("==", obs("readable", result.clone()), ok.clone()), neg(obs("writable", result.clone())), bin("==", size.clone(), int("0"))];
    let write = vec![bin("==", obs("writable", result.clone()), ok.clone()), neg(obs("readable", result.clone())), bin("==", size.clone(), int("0"))];
    match name {
        "file_open_read" => (vec![], read),
        "file_create" => (vec![], write),
        "file_read_all" => (vec![obs("readable", f)], vec![neg(obs("writable", result)), read[0].clone(), bin("==>", ok.clone(), bin(">=", size.clone(), int("0"))), bin("==>", neg(ok), bin("==", size, int("0")))]),
        "file_write_str" | "file_write_bytes" => (vec![obs("writable", f)], vec![neg(obs("readable", result)), write[0].clone(), write[2].clone()]),
        "file_close" => (vec![], vec![bin("==>", var("result", "bool"), obs("ok", f))]),
        "file_size" => (vec![], vec![bin(">=", var("result", "i64"), int("0"))]),
        "file_byte" => (vec![bin("&&", bin("<=", int("0"), var("i", "i64")), bin("<", var("i", "i64"), obs("size", f)))], vec![bin("&&", bin("<=", int("0"), var("result", "i64")), bin("<=", var("result", "i64"), int("255")))]),
        _ => (vec![], vec![]),
    }
}

// ---------------------------------------------------------------------------
// trusted rust fn declarations (section 10.4)
// ---------------------------------------------------------------------------

/// Option<i64|bool> / Result<i64|bool>
fn scalar_opt(t: &GType) -> bool {
    t.owner.is_none()
        && (t.name == "option" || t.name == "result")
        && t.args.len() <= 1
        && (t.args.is_empty() || t.args[0] == GType::i64() || t.args[0] == GType::bool())
}

/// G6b: catalog String, Vec<i64> or Vec<String> (structural: no nominal owner)
pub fn owned_result(t: &GType) -> bool {
    t.owner.is_none()
        && (*t == GType::new("string", vec![]) || matches!(t.to_key().as_str(), "vec<i64>" | "vec<string>"))
}

fn allowed_result(t: &GType) -> bool {
    *t == GType::i64() || *t == GType::bool() || *t == GType::unit() || scalar_opt(t) || owned_result(t)
}

fn allowed_param(t: &GType) -> bool {
    *t == GType::i64()
        || *t == GType::bool()
        || *t == GType::array()
        || *t == GType::array_u8()
        || *t == GType::array_bool()
        || *t == GType::str()
        || scalar_opt(t)
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => cs.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

/// fullmatch of `::[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*`
fn path_ok(path: &str) -> bool {
    match path.strip_prefix("::") {
        Some(rest) => rest.split("::").all(is_ident),
        None => false,
    }
}

/// signature (foreign_signature), then path (foreign_path) of a trusted rust fn
pub fn check_foreign_decl(f: &Function, externs: &[String]) -> Res<()> {
    let (fl, fc) = (f.line, f.col);
    if crate::intwidth::is_narrow(&f.ret) || f.params.iter().any(|p| crate::intwidth::is_narrow(&p.ty)) {
        return Err(err(
            fl,
            fc,
            "small_int_unsupported",
            crate::emit_common::SMALL_INT_HOST_MESSAGE,
            "use i64 in the host signature",
        ));
    }
    if !f.tparams.is_empty() {
        return Err(err(
            fl,
            fc,
            "foreign_signature",
            "a trusted rust fn cannot be generic",
            "declare one concrete trusted rust fn per type",
        ));
    }
    let ret = GType::from_key(&f.ret);
    if !allowed_result(&ret) {
        return Err(err(
            fl,
            fc,
            "foreign_signature",
            format!("result type {} is not allowed on a trusted rust fn", format_type(&ret)),
            "return i64, bool, (), Option<i64|bool>, Result<i64|bool>, String, Vec<i64> or Vec<String>; convert other results in a Rust adapter",
        ));
    }
    for prm in &f.params {
        let pt = GType::from_key(&prm.ty);
        if let Some((ml, mc)) = prm.mode_pos {
            // G5 A0: rejected whether or not the declaration is ever called; points at the `inout` token
            return Err(err(
                ml,
                mc,
                "foreign_signature",
                format!("inout parameter {} is not allowed on a trusted rust fn", prm.name),
                "remove `inout`; a foreign function only reads its arguments",
            ));
        }
        if prm.take {
            return Err(err(
                fl,
                fc,
                "foreign_signature",
                format!("take parameter {} is not allowed on a trusted rust fn", prm.name),
                "remove `take`; a foreign function only reads its arguments",
            ));
        }
        if pt.contains_collection() {
            return Err(err(
                fl,
                fc,
                "foreign_signature",
                format!("collection parameter {} is not allowed on a trusted rust fn", prm.name),
                "pass i64, bool, Option, Result, arrays or str; wrap collections in a Rust adapter",
            ));
        }
        if !allowed_param(&pt) {
            return Err(err(
                fl,
                fc,
                "foreign_signature",
                format!("parameter {} of type {} is not allowed on a trusted rust fn", prm.name, format_type(&pt)),
                "use i64, bool, Option<i64|bool>, Result<i64|bool>, [i64], [u8], [bool] or str",
            ));
        }
    }
    if f.ensures.is_empty() {
        return Err(err(
            fl,
            fc,
            "foreign_signature",
            "a trusted rust fn needs at least one ensures",
            "add an ensures clause; `ensures true` documents an unconstrained result",
        ));
    }
    let info = match &f.foreign {
        Some(i) => i,
        None => return Ok(()),
    };
    let (al, ac) = (info.line, info.col);
    let path = info.path.as_str();
    if !path_ok(path) {
        return Err(err(
            al,
            ac,
            "foreign_path",
            "invalid Rust path",
            "write the path as ::std::cmp::min: `::` and identifiers only, no expressions, generics or spaces",
        ));
    }
    let root = path[2..].split("::").next().unwrap_or("");
    if matches!(root, "crate" | "self" | "super") {
        return Err(err(
            al,
            ac,
            "foreign_path",
            "relative Rust path root",
            "start the path with ::std or the name of a crate given with --rust-extern",
        ));
    }
    if root != "std" && !externs.iter().any(|x| x == root) {
        return Err(err(
            al,
            ac,
            "foreign_path",
            format!("Rust crate {} is not supplied", root),
            format!("pass --rust-extern {}=PATH or use a ::std path", root),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ownership / liveness (section 10.4)
// State: ordered map name -> live | dead | maybe for owned collection bindings; None = unreachable path.
// The map keeps insertion order exactly like the Python dict (the first reported name of a loop error depends on it).
// Borrowed (unmarked collection) parameters are kept in `bor` and never change.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    Live,
    Dead,
    Maybe,
}

/// Insertion-ordered map with O(1) lookups: `slots` keeps the order (a popped entry leaves a `None`
/// tombstone, a re-inserted name goes to the end, as in a Python dict), `idx` maps a name to its slot.
#[derive(Clone, Default, Debug)]
struct St {
    slots: Vec<Option<(String, V)>>,
    idx: HashMap<String, usize>,
}

impl St {
    fn get(&self, n: &str) -> Option<V> {
        self.idx.get(n).and_then(|&i| self.slots[i].as_ref()).map(|(_, v)| *v)
    }
    fn set(&mut self, n: &str, v: V) {
        match self.idx.get(n) {
            Some(&i) => {
                if let Some(slot) = self.slots[i].as_mut() {
                    slot.1 = v;
                }
            }
            None => {
                self.idx.insert(n.to_string(), self.slots.len());
                self.slots.push(Some((n.to_string(), v)));
            }
        }
    }
    fn pop(&mut self, n: &str) {
        if let Some(i) = self.idx.remove(n) {
            self.slots[i] = None;
        }
    }
    fn contains(&self, n: &str) -> bool {
        self.idx.contains_key(n)
    }
    fn iter(&self) -> impl Iterator<Item = &(String, V)> {
        self.slots.iter().flatten()
    }
    /// L-gaps M (R-6g8-5): drop the partial-move keys `name.f..` of a binding that is replaced whole or ends
    fn pop_partials(&mut self, name: &str) {
        let pre = format!("{name}.");
        let ks: Vec<String> = self.iter().filter(|(k, _)| k.starts_with(&pre)).map(|(k, _)| k.clone()).collect();
        for k in ks {
            self.pop(&k);
        }
    }
}

fn join(a: Option<St>, b: Option<St>) -> Option<St> {
    let a = match a {
        None => return b,
        Some(a) => a,
    };
    let b = match b {
        None => return Some(a),
        Some(b) => b,
    };
    let mut out = St::default();
    for (n, v) in a.iter() {
        if let Some(bv) = b.get(n) {
            out.set(n, if bv == *v { *v } else { V::Maybe });
        } else if n.contains('.') && *v != V::Live {
            // L-gaps M (R-6g8-5): a field moved on one path only is maybe-moved after the join
            out.set(n, V::Maybe);
        }
    }
    for (n, v) in b.iter() {
        if n.contains('.') && *v != V::Live && a.get(n).is_none() {
            out.set(n, V::Maybe);
        }
    }
    Some(out)
}

/// L-gaps M: the root variable and field path of a field-access chain `v.f.g`
fn field_root(e: &Expr) -> Option<(&Var, Vec<String>)> {
    let mut fields = Vec::new();
    let mut cur = e;
    while let Expr::FieldAccess(x) = cur {
        fields.push(x.field.clone());
        cur = &x.base;
    }
    match cur {
        Expr::Var(v) if !fields.is_empty() => {
            fields.reverse();
            Some((v, fields))
        }
        _ => None,
    }
}

fn key_is_array(key: &str) -> bool {
    GType::try_from_key(key).is_ok_and(|ty| ty.is_array())
}

fn is_coll(ty: &Option<String>) -> bool {
    ty.as_deref().is_some_and(crate::gtype::key_is_collection_owner)
}

/// name -> `take` flags of the program's functions (what Python reads from `s.funcs[name].params[i].take`)
pub type Takes = HashMap<String, Vec<bool>>;

struct Ownership<'a> {
    funcs: &'a Takes,
    aggregates: &'a HashSet<String>,
    /// G-DB16-1 / G-R09-4: non-generic user enum and struct names (such a local is tracked so a binder move or a
    /// consuming field move can consume it)
    enums: &'a HashSet<String>,
    /// G-DB16-1: (enum, variant) -> field type keys of the program's non-generic user enums
    variants: &'a HashMap<(String, String), Vec<String>>,
    /// G-R09-4: position of the `move d.a` of a `let` that no next statement restores
    consumed_field: Option<(i64, i64)>,
    /// L-gaps M: some field was moved out in this function (partial-move keys `name.f..` exist)
    partial: bool,
    map_binders: Vec<String>,
    bor: HashSet<String>,
    /// binders of the user-enum match arms being checked (payload views; only owned collection fields are collections)
    arm_bor: Vec<String>,
    /// binders of the value `match` arms being walked (they view the scrutinee; only the move hint reads this)
    value_bor: Vec<String>,
    string_error_binders: HashSet<String>,
    /// LB9 (R-6g8-4): record binders of a consuming match (field moves only)
    field_only: HashSet<String>,
    /// name -> "move" | "borrow" of the outermost call being checked (shared by nested calls)
    track: Option<Track>,
    /// G10 A0: must-consume bindings, name -> "Process" | "Socket" | "Listener"; never File, String or ProcessArgs
    proc: HashMap<String, &'static str>,
    /// Place bindings that refer to a must-consume handle.
    borrowed_handles: HashMap<String, &'static str>,
    /// name -> (line, col) of the Let that declared it (location of a scope-end proc_unconsumed / net_unconsumed)
    decl: HashMap<String, (i64, i64)>,
    /// P5: enclosing while/for loops, innermost last: (label, state at loop entry, states at each `break`).
    loops: Vec<(Option<String>, St, Vec<St>)>,
}

type Track = HashMap<String, &'static str>;

/// G10 net A0 (G10_NET_SPEC 3.3): "Process" / "Socket" / "Listener" for a must-consume handle type key, else None.
fn must_kind(key: &str) -> Option<&'static str> {
    match key {
        "process" => Some("Process"),
        "socket" => Some("Socket"),
        "listener" => Some("Listener"),
        "loop" => Some("Loop"),
        _ => None,
    }
}

/// G10 proc A0 / net A0 (G10_PROC_SPEC 3.3 / 7.1, G10_NET_SPEC 3.3 / 7.1): a must-consume duty is left at
/// (line, col); the code follows the type of the handle that loses its duty
fn handle_lost(kind: &str, line: i64, col: i64, msg: String) -> PruvError {
    if kind == "Loop" {
        return err(line, col, "async_unconsumed", msg, "consume it with ev_close(move l), or return / transfer ownership");
    }
    if kind == "Process" {
        return err(line, col, "proc_unconsumed", msg, "consume it with proc_wait(move p) or proc_kill(move p), or return / transfer ownership");
    }
    err(line, col, "net_unconsumed", msg, "consume it with net_close(move s) or net_listener_close(move l), or return / transfer ownership")
}

impl<'a> Ownership<'a> {
    fn run(&mut self, f: &Function) -> Res<()> {
        self.bor = f.params.iter().filter(|p| !p.take && (crate::gtype::key_is_collection_owner(&p.ty) || self.aggregates.contains(&p.ty)))
            .map(|p| p.name.clone()).collect();
        self.proc = f
            .params
            .iter()
            .filter(|p| p.take)
            .filter_map(|p| must_kind(&p.ty).map(|k| (p.name.clone(), k)))
            .collect();
        let mut st = St::default();
        for p in f.params.iter().filter(|p| p.take) {
            st.set(&p.name, V::Live);
        }
        let end = self.block(&f.body, st)?;
        if let Some(end) = &end {
            // fallthrough: a live take must-consume parameter is a lost duty
            for (n, v) in end.iter() {
                if let Some(kind) = self.proc.get(n) {
                    if *v != V::Dead {
                        return Err(handle_lost(kind, f.line, f.col, format!("{} {} is not consumed before the end of the function", kind, n)));
                    }
                }
            }
        }
        Ok(())
    }

    fn block(&mut self, stmts: &[Stmt], st: St) -> Res<Option<St>> {
        let mut declared: Vec<String> = Vec::new();
        let mut cur: Option<St> = Some(st);
        for (i, x) in stmts.iter().enumerate() {
            if let Some(s) = cur.take() {
                // G-R09-4 / LG9 (G-LG8-1): the `move d.a` that consumes d: the let initializer itself, or the one
                // field move passed directly as a call argument of a let initializer or call statement (the type
                // checker admits it only for a take parameter), with no restore of that field by the next statement
                self.consumed_field = None;
                let value = match x {
                    Stmt::Let(l) => Some(&l.value),
                    Stmt::ExprStmt(e) => Some(&e.expr),
                    _ => None,
                };
                let moved = match value {
                    Some(Expr::Move(m)) if matches!(x, Stmt::Let(_)) => Some(m),
                    Some(Expr::Call(c)) => {
                        let ms: Vec<&Move> = c.args.iter().filter_map(|a| match a {
                            Expr::Move(m) if !m.fields.is_empty() => Some(m),
                            _ => None,
                        }).collect();
                        if ms.len() == 1 { Some(ms[0]) } else { None }
                    }
                    _ => None,
                };
                if let Some(m) = moved {
                    let restored = matches!(stmts.get(i + 1), Some(Stmt::FieldAssign(r))
                        if r.name == m.name && r.fields == m.fields);
                    // P6: a destructuring temporary gives each component up once and is never named again, so
                    // moving one component leaves the others readable
                    if !m.fields.is_empty() && !restored && !m.name.starts_with("pruv_tup_d") {
                        self.consumed_field = Some((m.line, m.col));
                    }
                }
                cur = self.stmt(x, s, &mut declared)?;
            } else {
                self.dead_stmt(x)?;
            }
        }
        for n in &declared {
            self.borrowed_handles.remove(n);
            if let Some(s) = cur.as_mut() {
                if let Some(kind) = self.proc.get(n) {
                    if matches!(s.get(n), Some(V::Live) | Some(V::Maybe)) {
                        let (l, c) = self.decl.get(n).copied().unwrap_or((0, 0));
                        return Err(handle_lost(kind, l, c, format!("{} {} is not consumed before the end of its scope", kind, n)));
                    }
                }
                s.pop(n);
                s.pop_partials(n);
            }
        }
        Ok(cur)
    }

    fn loop_body(&mut self, line: i64, col: i64, body: &[Stmt], entry: &St, st: St) -> Res<Option<St>> {
        let end = self.block(body, st.clone())?;
        if let Some(end) = &end {
            for (n, v) in entry.iter() {
                if *v == V::Live && end.get(n) != Some(V::Live) {
                    return Err(err(line, col, "foreign_loop_move", "moved in the loop body and not restored",
                        format!("{} must be live again at the end of every path of the body: reassign it, e.g. \
                                 {} = f(move {}, ..), or do not consume it inside the loop", n, n, n)));
                }
            }
            // L-gaps M (R-6g8-5): a field of an outer binding moved in the body must be restored by its end
            for (n, v) in end.iter() {
                if n.contains('.') && *v != V::Live && entry.contains(n.split('.').next().unwrap_or(""))
                    && entry.get(n) != Some(*v) {
                    return Err(err(line, col, "foreign_loop_move", "moved in the loop body and not restored",
                        format!("field {} must be reassigned before the end of every path of the body, or do not \
                                 move it inside the loop", n)));
                }
            }
        }
        Ok(join(Some(st), end))
    }

    fn stmt(&mut self, x: &Stmt, mut st: St, declared: &mut Vec<String>) -> Res<Option<St>> {
        match x {
            Stmt::Let(l) if l.is_place() => {
                // Evaluate the borrow while keeping the resource duty on its owner.
                self.expr(&l.value, &mut st)?;
                if let Some(kind) = must_kind(&l.ty) {
                    self.borrowed_handles.insert(l.name.clone(), kind);
                    declared.push(l.name.clone());
                }
                Ok(Some(st))
            }
            Stmt::Let(l) => {
                self.consume(&l.value, &mut st, "let")?;
                self.consumed_field = None;
                st.pop_partials(&l.name);
                if key_is_array(&l.ty) || l.ty == "str" || crate::gtype::key_is_collection_owner(&l.ty) || self.aggregates.contains(&l.ty)
                    || self.enums.contains(&l.ty)
                {
                    st.set(&l.name, V::Live);
                    declared.push(l.name.clone());
                    if let Some(kind) = must_kind(&l.ty) {
                        self.proc.insert(l.name.clone(), kind);
                        self.decl.insert(l.name.clone(), (l.line, l.col));
                    } else {
                        self.proc.remove(&l.name);
                    }
                }
                Ok(Some(st))
            }
            Stmt::Assign(a) => {
                self.consume(&a.value, &mut st, "assign")?; // the right-hand side is consumed first
                let mut kind = self.borrowed_handles.get(&a.name);
                if kind.is_none() && matches!(st.get(&a.name), Some(V::Live) | Some(V::Maybe)) {
                    kind = self.proc.get(&a.name);
                }
                if let Some(kind) = kind {
                    return Err(handle_lost(kind, a.line, a.col, format!("{} {} is overwritten while it is not consumed", kind, a.name)));
                }
                if st.contains(&a.name) {
                    st.set(&a.name, V::Live);
                }
                st.pop_partials(&a.name);
                Ok(Some(st))
            }
            Stmt::IndexAssign(a) => {
                self.expr(&a.index, &mut st)?;
                self.expr(&a.value, &mut st)?;
                if a.fields.is_empty() {
                    self.use_(&a.name, a.line, a.col, &st)?;
                } else {
                    // L-gaps M (R-6g8-5): `s.g[i] = v` needs only `s.g` live
                    self.use_path(&a.name, &a.fields, a.line, a.col, &st, false)?;
                }
                Ok(Some(st))
            }
            Stmt::FieldAssign(a) => {
                // L-gaps M (R-6g8-5): writing a moved field (or a field under it) restores it; writing inside a
                // moved field is a use of the moved value
                self.use_path(&a.name, &a.fields, a.line, a.col, &st, true)?;
                self.consume(&a.value, &mut st, "assign")?;
                let path = format!("{}.{}", a.name, a.fields.join("."));
                st.pop(&path);
                st.pop_partials(&path);
                Ok(Some(st))
            }
            Stmt::If(i) => {
                self.expr(&i.cond, &mut st)?;
                let a = self.block(&i.then, st.clone())?;
                let b = self.block(&i.els, st.clone())?;
                Ok(join(a, b))
            }
            Stmt::While(w) => {
                let entry = st.clone();
                // R03L: an invariant holds before the condition runs
                for clause in &w.invariants {
                    self.array_reads(clause, &st)?;
                }
                self.expr(&w.cond, &mut st)?;
                self.array_reads(&w.decreases, &st)?;
                self.loops.push((w.label.clone(), entry.clone(), Vec::new()));
                let out = self.loop_body(w.line, w.col, &w.body, &entry, st);
                let (_, _, breaks) = self.loops.pop().expect("loop frame");
                // P5: the state after the loop joins the normal exit and every `break`
                Ok(breaks.into_iter().fold(out?, |acc, b| join(acc, Some(b))))
            }
            Stmt::Break(j) | Stmt::Continue(j) => {
                let brk = matches!(x, Stmt::Break(_));
                let at = match &j.label {
                    None => self.loops.len().checked_sub(1),
                    Some(l) => self.loops.iter().rposition(|(n, _, _)| n.as_deref() == Some(l.as_str())),
                };
                let Some(at) = at else { return Ok(None) }; // the checker rejects a jump with no target
                let entry = &self.loops[at].1;
                // a handle declared inside the loops being left must already be consumed
                for (n, v) in st.iter() {
                    if let Some(kind) = self.proc.get(n) {
                        if !entry.contains(n) && *v != V::Dead {
                            let what = if brk { "break" } else { "continue" };
                            return Err(handle_lost(kind, j.line, j.col, format!("{} {} is not consumed before {}", kind, n, what)));
                        }
                    }
                }
                if brk {
                    self.loops[at].2.push(st);
                } else {
                    for (n, v) in entry.iter() {
                        if *v == V::Live && st.get(n) != Some(V::Live) {
                            return Err(err(j.line, j.col, "foreign_loop_move", "moved in the loop body and not restored",
                                format!("{} must be live again at every `continue` and at the end of every path of the body", n)));
                        }
                    }
                    // L-gaps M (R-6g8-5): so must a field of a binding declared outside the loop
                    for (n, v) in st.iter() {
                        let root = n.split('.').next().unwrap_or("");
                        if n.contains('.') && *v != V::Live && entry.contains(root) && entry.get(n) != Some(*v) {
                            return Err(err(j.line, j.col, "foreign_loop_move", "moved in the loop body and not restored",
                                format!("field {} must be reassigned before every `continue` and the end of every path \
                                         of the body, or do not move it inside the loop", n)));
                        }
                    }
                }
                Ok(None)
            }
            Stmt::ForMap(w) => {
                let entry = st.clone();
                for clause in &w.invariants {
                    self.array_reads(clause, &st)?;
                }
                self.expr(&w.place, &mut st)?;
                self.loop_body(w.line, w.col, &w.body, &entry, st)
            }
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    self.consume(v, &mut st, "return")?;
                }
                // every owned handle leaving the function must be dead (the value was consumed first)
                for (n, v) in st.iter() {
                    if let Some(kind) = self.proc.get(n) {
                        if *v != V::Dead {
                            return Err(handle_lost(kind, r.line, r.col, format!("{} {} is not consumed before return", kind, n)));
                        }
                    }
                }
                Ok(None)
            }
            Stmt::Par(p) => { for c in &p.calls { self.expr(c, &mut st)?; } Ok(Some(st)) }
            Stmt::ExprStmt(e) => {
                self.expr(&e.expr, &mut st)?;
                Ok(Some(st))
            }
            Stmt::Match(m) => {
                let owned_result = crate::llvmgen::expr_ty(&m.scrutinee).as_deref()
                    .is_some_and(|t| GType::from_key(t).has_string_error());
                if owned_result {
                    self.consume_payload(&m.scrutinee, &mut st)?;
                } else {
                    self.expr(&m.scrutinee, &mut st)?;
                }
                let mut out: Option<St> = None;
                let mut first = true;
                let mutable_map = matches!(&m.scrutinee, Expr::Call(c)
                    if matches!(c.name.as_str(), "map_get_mut" | "vec_at_mut") && !self.funcs.contains_key(&c.name));
                let take = matches!(&m.scrutinee, Expr::Call(c)
                    if c.name == "map_take" && !self.funcs.contains_key(&c.name));
                let consuming = match &m.scrutinee {
                    Expr::Var(sv) if st.contains(&sv.name) && !self.bor.contains(&sv.name)
                        && crate::codegen::match_moves_binder(m) => Some(sv.name.clone()),
                    _ => None,
                };
                for a in &m.arms {
                    let n = self.push_arm(a);
                    let saved = self.map_binders.len();
                    if mutable_map {
                        self.map_binders.extend(a.binders.iter().cloned());
                    }
                    let mut arm_state = st.clone();
                    // L-gaps M (R-6g8-5): an arm binder's moved fields live in the arm only; an outer binding of
                    // the same name keeps its own
                    let outer_partials: Vec<(String, V)> = arm_state.iter()
                        .filter(|(k, _)| a.binders.iter().any(|b| k.starts_with(&format!("{b}."))))
                        .cloned().collect();
                    for b in &a.binders {
                        arm_state.pop_partials(b);
                    }
                    if take {
                        for binder in &a.binders {
                            arm_state.set(binder, V::Live);
                            self.proc.remove(binder);
                        }
                    }
                    // G-DB16-1: the checker admits `move binder` of an array binder only for a plain owned enum
                    // scrutinee that no arm names. A match where some arm moves a binder takes the scrutinee by
                    // value: it is consumed on entry to every arm, and its array binders are tracked owners of the arm
                    let mut srcs: Vec<String> = Vec::new();
                    let mut records: Vec<String> = Vec::new();
                    if let (Some(sc), Some(en)) = (&consuming, &a.enum_name) {
                        arm_state.set(sc, V::Dead);
                        let fields = self.variants.get(&(en.clone(), a.ctor.clone())).cloned().unwrap_or_default();
                        // LG36 (G-DB59-1): str binders too, so a branch, loop or closed arm sees their moves
                        // LB9 (R-6g8-4): user struct and enum binders too, so a use after `let t = move b.f;` is a use
                        // after move on every path; they stay field-only (a whole `move b` is still refused, see mv)
                        records = a.binders.iter().zip(&fields).filter(|(_, t)| self.enums.contains(*t))
                            .map(|(b, _)| b.clone()).collect();
                        srcs = a.binders.iter().zip(&fields)
                            .filter(|(b, t)| key_is_array(t) || t.as_str() == "str" || records.contains(*b))
                            .map(|(b, _)| b.clone()).collect();
                        for b in &srcs {
                            arm_state.set(b, V::Live);
                        }
                        self.field_only.extend(records.iter().cloned());
                    }
                    let string_error = (owned_result && a.ctor == "Err").then(|| &a.binders[0]);
                    if let Some(binder) = string_error {
                        arm_state.set(binder, V::Live);
                        self.string_error_binders.insert(binder.clone());
                    }
                    let r = self.block(&a.body, arm_state);
                    for b in &records {
                        self.field_only.remove(b);
                    }
                    if let Some(binder) = string_error {
                        self.string_error_binders.remove(binder);
                    }
                    self.map_binders.truncate(saved);
                    self.arm_bor.truncate(self.arm_bor.len() - n);
                    let mut r = r?;
                    if let Some(state) = r.as_mut() {
                        if let Some(binder) = string_error {
                            state.pop(binder);
                        }
                        for b in &srcs {
                            state.pop(b);
                        }
                        for b in &a.binders {
                            state.pop_partials(b);
                        }
                        for (k, v) in &outer_partials {
                            state.set(k, *v);
                        }
                    }
                    if let (true, Some(state)) = (take, &mut r) {
                        for binder in &a.binders {
                            state.pop(binder);
                        }
                    }
                    out = if first { r } else { join(out, r) };
                    first = false;
                }
                Ok(out)
            }
            Stmt::Assert(a) => {
                self.array_reads(&a.cond, &st)?;
                Ok(Some(st))
            }
        }
    }

    /// Check array reads in proof clauses without applying executable call rules.
    fn array_reads(&self, e: &Expr, st: &St) -> Res<()> {
        if let (true, Some((v, fields))) = (self.partial, field_root(e)) {
            // L-gaps M (R-6g8-5): a field read in a proof clause checks only its own path
            if matches!(st.get(&v.name), Some(V::Dead | V::Maybe)) {
                self.use_(&v.name, e.line(), e.col(), st)?;
            }
            return self.use_path(&v.name, &fields, e.line(), e.col(), st, false);
        }
        if let Expr::Var(v) = e {
            // LG8: a record or enum root consumed by a field or binder move is checked too
            if v.ty.as_deref().is_some_and(|t| key_is_array(t) || t == "str")
                || (st.contains(&v.name)
                    && v.ty.as_ref().is_some_and(|t| self.enums.contains(t) || GType::from_key(t).has_string_error()))
            {
                self.use_(&v.name, v.line, v.col, st)?;
            }
            // L-gaps M (R-6g8-5): a proof clause naming a record whole conflicts with a field moved out of it
            self.use_path(&v.name, &[], v.line, v.col, st, false)?;
        }
        for child in expr_kids(e) {
            self.array_reads(child, st)?;
        }
        Ok(())
    }

    /// Check owning positions in unreachable code without tracking liveness.
    fn dead_stmt(&mut self, x: &Stmt) -> Res<()> {
        match x {
            Stmt::Let(l) => self.dead_expr(&l.value, true)?,
            Stmt::Assign(a) => self.dead_expr(&a.value, true)?,
            Stmt::FieldAssign(a) => self.dead_expr(&a.value, true)?,
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    self.dead_expr(v, true)?;
                }
            }
            Stmt::IndexAssign(a) => {
                self.dead_expr(&a.index, false)?;
                self.dead_expr(&a.value, false)?;
            }
            Stmt::ExprStmt(e) => self.dead_expr(&e.expr, false)?,
            Stmt::Par(p) => {
                for c in &p.calls {
                    self.dead_expr(c, false)?;
                }
            }
            Stmt::If(i) => {
                self.dead_expr(&i.cond, false)?;
                for stmt in i.then.iter().chain(&i.els) {
                    self.dead_stmt(stmt)?;
                }
            }
            Stmt::While(w) => {
                self.dead_expr(&w.cond, false)?;
                for stmt in &w.body {
                    self.dead_stmt(stmt)?;
                }
            }
            Stmt::ForMap(w) => {
                self.dead_expr(&w.place, false)?;
                for stmt in &w.body {
                    self.dead_stmt(stmt)?;
                }
            }
            Stmt::Match(m) => {
                let owned_result = crate::llvmgen::expr_ty(&m.scrutinee).as_deref()
                    .is_some_and(|t| GType::from_key(t).has_string_error());
                self.dead_expr(&m.scrutinee, owned_result)?;
                for a in &m.arms {
                    let n = self.push_arm(a);
                    let r = a.body.iter().try_for_each(|stmt| self.dead_stmt(stmt));
                    self.arm_bor.truncate(self.arm_bor.len() - n);
                    r?;
                }
            }
            Stmt::Assert(_) => {}
        }
        Ok(())
    }

    fn dead_expr(&mut self, e: &Expr, owned: bool) -> Res<()> {
        if matches!(e, Expr::Move(_)) && !owned {
            // R03L: unreachable code keeps the consuming-position rule
            return Err(err(
                e.line(),
                e.col(),
                "foreign_move",
                "move only where ownership is consumed",
                "use move for a take argument, a let initializer, an assignment or return",
            ));
        }
        if let Expr::Move(m) = e {
            let mut st = St::default();
            // Check the operand without carrying liveness across unreachable statements.
            if !self.bor.contains(&m.name) && !self.arm_bor.contains(&m.name) {
                st.set(&m.name, V::Live);
            }
            self.mv(m, &mut st, true, false)?;
        }
        if owned {
            Self::check_field_alias(e)?;
            if let Expr::Var(v) = e {
                if is_coll(&v.ty) && (self.bor.contains(&v.name) || self.arm_bor.contains(&v.name)) {
                    return Err(err(
                        e.line(),
                        e.col(),
                        "foreign_borrow",
                        "borrowed alias",
                        format!("{} is a read parameter and cannot be aliased", v.name),
                    ));
                }
            }
        }
        if let Expr::Call(c) = e {
            Self::check_call_move_alias(c)?;
            for (arg, take) in c.args.iter().zip(self.takes(c)) {
                self.dead_expr(arg, take)?;
            }
        } else {
            let owns_fields = matches!(e, Expr::StructLit(_) | Expr::EnumCtor(_))
                || matches!(e, Expr::Ctor(x) if x.name == "Err" && x.ty.as_deref()
                    .is_some_and(|t| GType::from_key(t).has_string_error()));
            for k in expr_kids(e) {
                self.dead_expr(k, owns_fields)?;
            }
        }
        Ok(())
    }

    /// push the binders of a user-enum arm; returns how many were pushed
    fn push_arm(&mut self, a: &MatchArm) -> usize {
        if a.enum_name.is_none() {
            return 0;
        }
        self.arm_bor.extend(a.binders.iter().cloned());
        a.binders.len()
    }

    fn check_field_alias(e: &Expr) -> Res<()> {
        if matches!(e, Expr::FieldAccess(_) | Expr::VariantField(_)) && is_coll(crate::llvmgen::expr_ty(e)) {
            return Err(err(
                e.line(),
                e.col(),
                "foreign_borrow",
                "borrowed alias",
                "the field is a read view and cannot be aliased",
            ));
        }
        Ok(())
    }

    // ---- uses ----
    fn use_(&self, name: &str, line: i64, col: i64, st: &St) -> Res<()> {
        match st.get(name) {
            Some(V::Dead) => Err(err(
                line,
                col,
                "foreign_move",
                "use after move",
                format!("{} was moved earlier; reassign it before using it again, or avoid the move", name),
            )),
            Some(V::Maybe) => Err(err(
                line,
                col,
                "foreign_move",
                "binding may have been moved",
                format!("{} is moved on some paths; make every path leave it live, or do not use it afterwards", name),
            )),
            _ => self.use_path(name, &[], line, col, st, false),
        }
    }

    /// L-gaps M (R-6g8-5): a use of `name.fields` conflicts with a moved field on the same path: a prefix of it
    /// (the read is inside the moved value) or, unless `write`, an extension of it (the read contains the hole)
    fn use_path(&self, name: &str, fields: &[String], line: i64, col: i64, st: &St, write: bool) -> Res<()> {
        if !self.partial {
            return Ok(());
        }
        if !fields.is_empty() {
            if let Some(v @ (V::Dead | V::Maybe)) = st.get(name) {
                let _ = v;
                return self.use_(name, line, col, st);
            }
        }
        let pre = format!("{name}.");
        for (k, v) in st.iter() {
            let Some(rest) = k.strip_prefix(&pre) else { continue };
            if *v == V::Live {
                continue;
            }
            let kp: Vec<&str> = rest.split('.').collect();
            let n = kp.len().min(fields.len());
            if kp[..n].iter().zip(&fields[..n]).any(|(a, b)| *a != b.as_str()) {
                continue;
            }
            if write && kp.len() >= fields.len() {
                continue;
            }
            let what = if *v == V::Dead { "use after move" } else { "binding may have been moved" };
            return Err(err(line, col, "foreign_move", what,
                format!("field {} was moved out of {}; reassign the field before using it or {} whole", k, name, name)));
        }
        Ok(())
    }

    fn mv(&mut self, e: &Move, st: &mut St, consuming: bool, tracked: bool) -> Res<()> {
        if !e.fields.is_empty() {
            self.use_path(&e.name, &e.fields, e.line, e.col, st, false)?;
            if matches!(st.get(&e.name), Some(V::Dead | V::Maybe)) {
                self.use_(&e.name, e.line, e.col, st)?;
            }
            if self.consumed_field == Some((e.line, e.col)) {
                // G-R09-4 / L-gaps M (R-6g8-5): no restore follows, so the field is moved out (a Rust partial
                // move); the record is unusable whole until the field is reassigned, its other fields stay usable
                st.set(&format!("{}.{}", e.name, e.fields.join(".")), V::Dead);
                self.partial = true;
            }
            // Otherwise the type checker enforces restoration. The root stays live.
            return Ok(());
        }
        let (line, col) = (e.line, e.col);
        if !is_coll(&e.ty) && !e.ty.as_deref().is_some_and(key_is_array) && e.ty.as_deref() != Some("str")
            && !(e.ty.as_ref().is_some_and(|t| self.aggregates.contains(t)) && st.contains(&e.name)
                && !self.field_only.contains(&e.name)) {
            return Err(err(
                line,
                col,
                "foreign_move",
                "move of a non-collection value",
                "move applies to Vec, HashMap and String bindings only; use the value directly",
            ));
        }
        let n = e.name.as_str();
        if let (true, Some(t)) = (tracked, self.track.as_ref()) {
            if let Some(kind) = t.get(n) {
                if *kind == "move" {
                    return Err(err(
                        line,
                        col,
                        "foreign_move",
                        "consumed twice",
                        format!("consume {} at most once per call", n),
                    ));
                }
                return Err(err(
                    line,
                    col,
                    "foreign_borrow",
                    "borrowed and moved in one call",
                    format!("do not pass {} both read-only and with move in one call", n),
                ));
            }
        }
        self.use_(n, line, col, st)?;
        if !consuming {
            return Err(err(
                line,
                col,
                "foreign_move",
                "move only where ownership is consumed",
                "use move for a take argument, a let initializer, an assignment or return",
            ));
        }
        // G-DB16-1: an untracked array binder of an enum arm is a legal move source (like Python, which only rejects `bor`)
        let arm_array = e.ty.as_deref().is_some_and(key_is_array) && self.arm_bor.iter().any(|b| b == n);
        if self.bor.contains(n) || (!st.contains(n) && !arm_array) {
            // G10 specifies foreign_borrow; the established G6 diagnostic stays for the other types.
            let code = if matches!(e.ty.as_deref(), Some("file") | Some("process") | Some("socket") | Some("listener") | Some("loop")) { "foreign_borrow" } else { "foreign_move" };
            return Err(err(
                line,
                col,
                code,
                "moving a borrowed parameter",
                if self.value_bor.iter().any(|b| b == n) {
                    // P9a-6 remaining (R-6w-7): an arm value cannot move a value `match` binder
                    format!("{} is a binder of a value `match`, which cannot be moved; use `copy({})` or a `match` statement", n, n)
                } else {
                    format!("{} is a read parameter; declare it `take {}: ...` to consume it", n, n)
                },
            ));
        }
        st.set(n, V::Dead);
        if let (true, Some(t)) = (tracked, self.track.as_mut()) {
            t.insert(n.to_string(), "move");
        }
        Ok(())
    }

    /// let initializer / assignment / return value
    fn consume(&mut self, e: &Expr, st: &mut St, pos: &str) -> Res<()> {
        match e {
            Expr::Move(m) => self.mv(m, st, true, pos == "field"),
            Expr::Var(v) if self.map_binders.contains(&v.name) => Err(err(
                v.line, v.col, "foreign_move", "a mutable collection binder cannot be used as an owned value", "",
            )),
            Expr::Var(v) if is_coll(&v.ty) => {
                self.use_(&v.name, v.line, v.col, st)?;
                if self.bor.contains(&v.name) || !st.contains(&v.name) {
                    if pos == "return" {
                        return Err(err(
                            v.line,
                            v.col,
                            "foreign_borrow",
                            "borrowed return",
                            format!("{} is a read parameter; declare it `take`", v.name),
                        ));
                    }
                    return Err(err(
                        v.line,
                        v.col,
                        "foreign_borrow",
                        "borrowed alias",
                        format!("{} is a read parameter and cannot be aliased", v.name),
                    ));
                }
                if pos == "return" {
                    return Err(err(
                        v.line,
                        v.col,
                        "foreign_move",
                        "returning an owned binding without move",
                        format!("write return move {};", v.name),
                    ));
                }
                Err(err(
                    v.line,
                    v.col,
                    "foreign_move",
                    "alias of an owned binding without move",
                    format!("write move {}", v.name),
                ))
            }
            _ => self.expr_o(e, st, true),
        }
    }

    /// A match or String error constructor takes its payload's ownership.
    fn consume_payload(&mut self, e: &Expr, st: &mut St) -> Res<()> {
        match e {
            Expr::Var(v) => self.mv(&Move {
                line: v.line, col: v.col, name: v.name.clone(), fields: Vec::new(), ty: v.ty.clone(),
            }, st, true, true),
            _ => self.consume(e, st, "field"),
        }
    }

    // ---- expressions ----
    fn expr(&mut self, e: &Expr, st: &mut St) -> Res<()> {
        self.expr_o(e, st, false)
    }

    /// owned: the value is taken at once by a binding, a return or a take argument (not a dropped temporary)
    fn expr_o(&mut self, e: &Expr, st: &mut St, owned: bool) -> Res<()> {
        if owned {
            Self::check_field_alias(e)?;
        }
        match e {
            Expr::Spent(_) => Ok(()),
            Expr::Var(v) => {
                if is_coll(&v.ty) || v.ty.as_deref().is_some_and(key_is_array)
                    || v.ty.as_ref().is_some_and(|t| self.aggregates.contains(t) || self.enums.contains(t))
                    || st.contains(&v.name) {
                    self.use_(&v.name, v.line, v.col, st)?;
                }
                Ok(())
            }
            Expr::FieldAccess(_) if self.partial && field_root(e).is_some() => {
                // L-gaps M (R-6g8-5): a field read checks only its own path against the moved fields
                let (v, fields) = field_root(e).unwrap();
                if matches!(st.get(&v.name), Some(V::Dead | V::Maybe)) {
                    self.use_(&v.name, e.line(), e.col(), st)?;
                }
                self.use_path(&v.name, &fields, e.line(), e.col(), st, false)
            }
            Expr::Move(m) => self.mv(m, st, false, false),
            Expr::Index(x) if matches!(&*x.array, Expr::Var(v) if v.ty.as_deref().is_some_and(key_is_array)) => {
                self.expr(&x.index, st)?;
                self.expr(&x.array, st)
            }
            Expr::Call(c) if PlaceIf::matches(c) => {
                let [cond, then, els] = c.args.as_slice() else { unreachable!() };
                self.expr(cond, st)?;
                let mut other = st.clone();
                self.expr(then, st)?;
                self.expr(els, &mut other)?;
                *st = join(Some(std::mem::take(st)), Some(other)).unwrap();
                Ok(())
            }
            Expr::Call(c) if crate::ast::ValueMatch::matches(c) => {
                // P9a-5: only one arm runs, and a value moved on one arm is maybe-moved after the join. P9a-6
                // remaining: a `Result<_, String>` scrutinee is consumed as by the statement form (a read parameter
                // is `type/foreign_move`)
                let owned_result = crate::llvmgen::expr_ty(&c.args[0]).as_deref()
                    .is_some_and(|t| GType::from_key(t).has_string_error());
                if owned_result {
                    self.consume_payload(&c.args[0], st)?;
                } else {
                    self.expr(&c.args[0], st)?;
                }
                let base = st.clone();
                let mut joined: Option<St> = None;
                for (arm, va) in c.args[1..].iter().zip(crate::ast::ValueMatch::arms(c)) {
                    let mut s = base.clone();
                    // binders view the scrutinee: they are borrowed (moving one names the binder in the hint)
                    let n = va.binders.len();
                    self.value_bor.extend(va.binders.iter().cloned());
                    let r = if owned { self.consume(arm, &mut s, "field") } else { self.expr_o(arm, &mut s, false) };
                    self.value_bor.truncate(self.value_bor.len() - n);
                    r?;
                    joined = join(joined, Some(s));
                }
                *st = joined.unwrap_or(base);
                Ok(())
            }
            Expr::Call(c) if crate::ast::ValueIf::matches(c) => {
                // P9a-3: only one arm runs; a value moved on one path is maybe-moved after the join
                let [cond, then, els] = c.args.as_slice() else { unreachable!() };
                self.expr(cond, st)?;
                let mut other = st.clone();
                for (arm, st) in [(then, &mut *st), (els, &mut other)] {
                    if owned { self.consume(arm, st, "field")? } else { self.expr_o(arm, st, false)? }
                }
                *st = join(Some(std::mem::take(st)), Some(other)).unwrap();
                Ok(())
            }
            Expr::Call(c) => {
                self.call(c, st)?;
                if !owned {
                    // a handle temporary that is borrowed or dropped keeps its duty
                    if let Some(kind) = c.ty.as_deref().and_then(must_kind) {
                        return Err(handle_lost(kind, c.line, c.col, format!("temporary {} is not consumed", kind)));
                    }
                }
                Ok(())
            }
            Expr::StructLit(lit) => {
                for f in &lit.fields {
                    self.consume(&f.value, st, "field")?;
                }
                Ok(())
            }
            Expr::EnumCtor(ctor) => {
                for arg in &ctor.args {
                    self.consume(arg, st, "field")?;
                }
                Ok(())
            }
            Expr::Ctor(x) if x.name == "Err" && x.ty.as_deref()
                .is_some_and(|t| GType::from_key(t).has_string_error()) =>
            {
                self.consume_payload(x.arg.as_deref().unwrap(), st)
            }
            Expr::Binary(b) if b.op == "&&" || b.op == "||" => {
                self.expr(&b.left, st)?;
                let before = st.clone();
                self.expr(&b.right, st)?;
                // the right operand runs conditionally
                let joined = join(Some(before), Some(st.clone()));
                if let Some(j) = joined {
                    *st = j;
                }
                Ok(())
            }
            _ => {
                for k in expr_kids(e) {
                    self.expr(k, st)?;
                }
                Ok(())
            }
        }
    }

    fn takes(&self, e: &Call) -> Vec<bool> {
        if e.name == "replace" && !self.funcs.contains_key(&e.name) && matches!(e.args.first(), Some(Expr::InoutArg(_))) {
            return vec![false, true]; // R-6g9-12: `replace(inout P, e)` consumes e into P
        }
        match if self.funcs.contains_key(&e.name) { None } else { catalog_fn(&e.name) } {
            Some(f) => f.params.iter().enumerate().map(|(i, p)| {
                p.take || (e.name == "map_insert" && i == 1 && crate::llvmgen::expr_ty(&e.args[i]).as_deref() == Some("string")) || (i + 1 == e.args.len() && matches!(e.name.as_str(), "vec_push" | "map_insert")
                    && crate::llvmgen::expr_ty(&e.args[i]).as_ref().is_some_and(|t| t == "string" || self.aggregates.contains(t)))
            }).collect(),
            None => match self.funcs.get(&e.name) {
                Some(t) => t.clone(),
                None => vec![false; e.args.len()],
            },
        }
    }

    fn call(&mut self, e: &Call, st: &mut St) -> Res<()> {
        // nested calls share the outermost call's track (audit R1)
        let outer = self.track.is_some();
        if !outer {
            self.track = Some(HashMap::new());
        }
        let r = self.call_args(e, st, &self.takes(e));
        if !outer {
            self.track = None;
        }
        r
    }

    fn check_call_move_alias(e: &Call) -> Res<()> {
        for (i, arg) in e.args.iter().enumerate() {
            // D85: a move inside a constructor argument conflicts with any other mention
            let mut moves = Vec::new();
            ctor_moves(arg, &mut moves);
            for m in moves {
                for (j, other) in e.args.iter().enumerate() {
                    if i != j && crate::check::mentions(other, &m.name) {
                        let node = &e.args[i.max(j)];
                        return Err(err(node.line(), node.col(), "foreign_borrow", "borrowed and moved in one call",
                            format!("do not pass {} both read-only and with move in one call", m.name)));
                    }
                }
            }
            if let Expr::Move(m) = arg {
                if m.fields.is_empty() {
                    for (j, other) in e.args.iter().enumerate() {
                        if i != j && !matches!(other, Expr::Move(x) if x.fields.is_empty())
                            && crate::check::mentions(other, &m.name)
                        {
                            let node = &e.args[i.max(j)];
                            return Err(err(node.line(), node.col(), "foreign_borrow", "borrowed and moved in one call",
                                format!("do not pass {} both read-only and with move in one call", m.name)));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn call_args(&mut self, e: &Call, st: &mut St, takes: &[bool]) -> Res<()> {
        Self::check_call_move_alias(e)?;
        for (arg, take) in e.args.iter().zip(takes.iter()) {
            if e.name == "string_to_str" && matches!(arg, Expr::Var(v) if self.string_error_binders.contains(&v.name)) {
                self.consume_payload(arg, st)?;
            } else if *take {
                match arg {
                    Expr::Move(m) => self.mv(m, st, true, true)?,
                    Expr::Var(v) if is_coll(&v.ty) => {
                        self.use_(&v.name, v.line, v.col, st)?;
                        if self.bor.contains(&v.name) || !st.contains(&v.name) {
                            return Err(err(
                                v.line,
                                v.col,
                                "foreign_move",
                                "moving a borrowed parameter",
                                format!("{} is a read parameter; declare it `take {}: ...` to consume it", v.name, v.name),
                            ));
                        }
                        return Err(err(
                            v.line,
                            v.col,
                            "foreign_move",
                            "missing move in a consuming argument",
                            format!("write move {}", v.name),
                        ));
                    }
                    _ => self.consume(arg, st, "argument")?,
                }
            } else {
                match arg {
                    Expr::Move(m) => self.mv(m, st, false, true)?,
                    Expr::Var(v) if is_coll(&v.ty) || st.contains(&v.name) => {
                        if self.track.as_ref().and_then(|t| t.get(&v.name)) == Some(&"move") {
                            return Err(err(
                                v.line,
                                v.col,
                                "foreign_borrow",
                                "borrowed and moved in one call",
                                format!("do not pass {} both read-only and with move in one call", v.name),
                            ));
                        }
                        self.use_(&v.name, v.line, v.col, st)?;
                        if let Some(t) = self.track.as_mut() {
                            t.entry(v.name.clone()).or_insert("borrow");
                        }
                    }
                    // R-6e-11: a field or a vec_get element of a place borrows the place's root
                    Expr::FieldAccess(_) | Expr::Call(_) if e.name == "string_eq" => {
                        let root = match arg {
                            Expr::Call(c) if c.name == "vec_get" && !c.args.is_empty() => crate::check::place_root(&c.args[0]),
                            Expr::Call(_) => None,
                            _ => crate::check::place_root(arg),
                        };
                        if let Some(root) = root {
                            if self.track.as_ref().and_then(|t| t.get(root)) == Some(&"move") {
                                return Err(err(arg.line(), arg.col(), "foreign_borrow", "borrowed and moved in one call",
                                    format!("do not pass {} both read-only and with move in one call", root)));
                            }
                            if let Some(t) = self.track.as_mut() {
                                t.entry(root.to_string()).or_insert("borrow");
                            }
                        }
                        self.expr(arg, st)?;
                    }
                    _ => self.expr(arg, st)?,
                }
            }
        }
        Ok(())
    }
}

/// whole-binding moves nested inside an argument, through constructors and calls (D85)
fn ctor_moves<'e>(e: &'e Expr, out: &mut Vec<&'e Move>) {
    for k in expr_kids(e) {
        match k {
            Expr::Move(m) if m.fields.is_empty() => out.push(m),
            _ => ctor_moves(k, out),
        }
    }
}

/// ownership/liveness of one typed, non-foreign function; funcs: name -> `take` flags of the program's functions
pub fn check_ownership(f: &Function, funcs: &Takes, aggregates: &HashSet<String>, enums: &HashSet<String>,
                       variants: &HashMap<(String, String), Vec<String>>) -> Res<()> {
    Ownership {
        funcs, aggregates, enums, variants, map_binders: Vec::new(), bor: HashSet::new(), arm_bor: Vec::new(), value_bor: Vec::new(),
        string_error_binders: HashSet::new(), field_only: HashSet::new(), track: None,
        proc: HashMap::new(), borrowed_handles: HashMap::new(), decl: HashMap::new(), consumed_field: None, partial: false,
        loops: Vec::new(),
    }.run(f)
}

// ---------------------------------------------------------------------------
// A0 guard (section 10.5)
// ---------------------------------------------------------------------------
type Cands = Vec<(i64, i64)>;

fn key_contains_collection(key: &str) -> bool {
    match GType::try_from_key(key) {
        Ok(t) => t.contains_collection(),
        Err(_) => false,
    }
}

fn cost_ctors(e: &Expr, out: &mut Cands) {
    if let Expr::Ctor(x) = e {
        if x.ty.as_deref().is_some_and(key_contains_collection) {
            out.push((x.line, x.col));
        }
    }
    for k in crate::refine::kids(e) {
        cost_ctors(k, out);
    }
}

fn g6_expr(e: &Expr, out: &mut Cands) {
    match e {
        Expr::Move(m) if !m.ty.as_deref().is_some_and(key_is_array) => out.push((m.line, m.col)),
        Expr::Move(_) => {}
        Expr::Call(c) => {
            if let Some(t) = &c.ty {
                if key_contains_collection(t) || c.name == "string_eq" {
                    out.push((c.line, c.col));
                }
            }
            for a in &c.args {
                g6_expr(a, out);
            }
        }
        Expr::Unary(x) => g6_expr(&x.operand, out),
        Expr::Binary(x) => {
            g6_expr(&x.left, out);
            g6_expr(&x.right, out);
        }
        Expr::Index(x) => {
            g6_expr(&x.array, out);
            g6_expr(&x.index, out);
        }
        Expr::Ctor(x) => {
            // LG12: Ok::<T, String>(..)
            if x.ty.as_deref().is_some_and(key_contains_collection) {
                out.push((x.line, x.col));
            }
            if let Some(a) = &x.arg {
                g6_expr(a, out);
            }
        }
        Expr::Quant(x) => {
            g6_expr(&x.lo, out);
            g6_expr(&x.hi, out);
            g6_expr(&x.body, out);
            for t in &x.triggers {
                g6_expr(t, out);
            }
        }
        Expr::StructLit(x) => {
            for f in &x.fields {
                g6_expr(&f.value, out);
            }
        }
        Expr::EnumCtor(x) => {
            for a in &x.args {
                g6_expr(a, out);
            }
        }
        Expr::FieldAccess(x) => g6_expr(&x.base, out),
        Expr::IsVariant(x) => g6_expr(&x.value, out),
        Expr::VariantField(x) => g6_expr(&x.value, out),
        Expr::InoutArg(x) => {
            if x.ty.as_deref().is_some_and(key_is_collection) { out.push((x.line, x.col)); }
            g6_expr(&x.expr, out);
        }
        Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) => {}
    }
}

fn g6_stmts(stmts: &[Stmt], out: &mut Cands) {
    for s in stmts {
        match s {
            Stmt::Let(l) => {
                if key_contains_collection(&l.ty) {
                    out.push((l.line, l.col));
                }
                g6_expr(&l.value, out);
            }
            Stmt::Assign(a) => g6_expr(&a.value, out),
            Stmt::IndexAssign(a) => {
                g6_expr(&a.index, out);
                g6_expr(&a.value, out);
            }
            Stmt::If(i) => {
                g6_expr(&i.cond, out);
                g6_stmts(&i.then, out);
                g6_stmts(&i.els, out);
            }
            Stmt::While(w) => {
                g6_expr(&w.cond, out);
                for inv in &w.invariants {
                    g6_expr(inv, out);
                }
                g6_expr(&w.decreases, out);
                g6_stmts(&w.body, out);
            }
            Stmt::ForMap(w) => {
                g6_expr(&w.place, out);
                for inv in &w.invariants {
                    g6_expr(inv, out);
                }
                g6_stmts(&w.body, out);
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    g6_expr(v, out);
                }
            }
            Stmt::Assert(a) => g6_expr(&a.cond, out),
            Stmt::Par(e) => {
                for c in &e.calls {
                    g6_expr(c, out);
                }
            }
            Stmt::ExprStmt(e) => g6_expr(&e.expr, out),
            Stmt::FieldAssign(f) => g6_expr(&f.value, out),
            Stmt::Match(m) => {
                g6_expr(&m.scrutinee, out);
                for a in &m.arms {
                    g6_stmts(&a.body, out);
                }
            }
        }
    }
}

/// first G6 construct of a checked program -> (line, col, what) or None. Source order: functions in order,
/// the signature (foreign declaration, take or collection parameter/result) before the body.
pub fn first_g6_use(prog: &Program) -> Option<(i64, i64, String)> {
    for d in &prog.types {
        if let Some(f) = d.all_fields().into_iter().find(|f| key_contains_collection(&f.ty)) {
            return Some((f.line, f.col, format!("collection field {}.{}", d.name(), f.name)));
        }
    }
    for f in &prog.functions {
        if f.foreign.is_some() {
            return Some((f.line, f.col, format!("trusted rust fn {}", f.name)));
        }
        for p in &f.params {
            if (p.take && !key_is_array(&p.ty)) || key_contains_collection(&p.ty) {
                return Some((f.line, f.col, format!("parameter {}", p.name)));
            }
        }
        if key_contains_collection(&f.ret) {
            return Some((f.line, f.col, format!("result type of {}", f.name)));
        }
        let mut cands: Cands = Vec::new();
        if let Some(cost) = &f.cost {
            g6_expr(cost, &mut cands);
            // LG12: a constructor that only appears in the cost clause is not a G6 use (the clause is erased
            // after verification and evaluated as a contract otherwise), as in Python
            let mut ctors: Cands = Vec::new();
            cost_ctors(cost, &mut ctors);
            cands.retain(|c| !ctors.contains(c));
        }
        g6_stmts(&f.body, &mut cands);
        // min by (line, col); the first one wins on a tie, like Python's min(key=..)
        let mut best: Option<(i64, i64)> = None;
        for c in cands {
            if best.map_or(true, |b| c < b) {
                best = Some(c);
            }
        }
        if let Some((l, c)) = best {
            return Some((l, c, "a G6 construct".to_string()));
        }
    }
    None
}

/// PruvError (unsupported_type) for the first G6 construct, or None. Run right after type checking.
pub fn g6_unimplemented_error(prog: &Program, stage: &str) -> Option<PruvError> {
    let (line, col, _what) = first_g6_use(prog)?;
    Some(PruvError::new(
        stage,
        line,
        col,
        "unsupported_type",
        "G6 foreign calls and collections are not implemented by the llvm codegen layer yet",
        "avoid trusted rust fn, Vec, HashMap, String, take and move for now",
    ))
}

// ---------------------------------------------------------------------------
// G10 proc: first Process use (counterpart of foreign.py first_proc_use)
// ---------------------------------------------------------------------------

fn key_has_proc(key: &str) -> bool {
    fn has(t: &GType) -> bool {
        (t.owner.is_none() && (t.name == "process" || t.name == "process_args")) || t.args.iter().any(has)
    }
    match GType::try_from_key(key) {
        Ok(t) => has(&t),
        Err(_) => false,
    }
}

fn proc_expr(e: &Expr, out: &mut Cands) {
    match e {
        Expr::Call(c) => {
            if is_proc_op(&c.name) {
                out.push((c.line, c.col));
            }
            for a in &c.args {
                proc_expr(a, out);
            }
        }
        Expr::Unary(x) => proc_expr(&x.operand, out),
        Expr::Binary(x) => {
            proc_expr(&x.left, out);
            proc_expr(&x.right, out);
        }
        Expr::Index(x) => {
            proc_expr(&x.array, out);
            proc_expr(&x.index, out);
        }
        Expr::Ctor(x) => {
            if let Some(a) = &x.arg {
                proc_expr(a, out);
            }
        }
        Expr::Quant(x) => {
            proc_expr(&x.lo, out);
            proc_expr(&x.hi, out);
            proc_expr(&x.body, out);
            for t in &x.triggers {
                proc_expr(t, out);
            }
        }
        Expr::StructLit(x) => {
            for f in &x.fields {
                proc_expr(&f.value, out);
            }
        }
        Expr::EnumCtor(x) => {
            for a in &x.args {
                proc_expr(a, out);
            }
        }
        Expr::FieldAccess(x) => proc_expr(&x.base, out),
        Expr::IsVariant(x) => proc_expr(&x.value, out),
        Expr::VariantField(x) => proc_expr(&x.value, out),
        Expr::InoutArg(x) => proc_expr(&x.expr, out),
        Expr::Move(_) | Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) => {}
    }
}

fn proc_stmts(stmts: &[Stmt], out: &mut Cands) {
    for s in stmts {
        match s {
            Stmt::Let(l) => {
                if key_has_proc(&l.ty) {
                    out.push((l.line, l.col));
                }
                proc_expr(&l.value, out);
            }
            Stmt::Assign(a) => proc_expr(&a.value, out),
            Stmt::IndexAssign(a) => {
                proc_expr(&a.index, out);
                proc_expr(&a.value, out);
            }
            Stmt::If(i) => {
                proc_expr(&i.cond, out);
                proc_stmts(&i.then, out);
                proc_stmts(&i.els, out);
            }
            Stmt::While(w) => {
                proc_expr(&w.cond, out);
                for inv in &w.invariants {
                    proc_expr(inv, out);
                }
                proc_expr(&w.decreases, out);
                proc_stmts(&w.body, out);
            }
            Stmt::ForMap(w) => {
                proc_expr(&w.place, out);
                for inv in &w.invariants {
                    proc_expr(inv, out);
                }
                proc_stmts(&w.body, out);
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    proc_expr(v, out);
                }
            }
            Stmt::Assert(a) => proc_expr(&a.cond, out),
            Stmt::Par(e) => {
                for c in &e.calls {
                    proc_expr(c, out);
                }
            }
            Stmt::ExprStmt(e) => proc_expr(&e.expr, out),
            Stmt::FieldAssign(f) => proc_expr(&f.value, out),
            Stmt::Match(m) => {
                proc_expr(&m.scrutinee, out);
                for a in &m.arms {
                    proc_stmts(&a.body, out);
                }
            }
        }
    }
}

/// The first use of Process, ProcessArgs or a proc_* operation -> (line, col): functions in order; the signature (a
/// Process / ProcessArgs parameter or result) before the body, whose first use is the smallest source position.
/// A function that only declares `! proc` is not a use.
pub fn first_proc_use(prog: &Program) -> Option<(i64, i64)> {
    for f in &prog.functions {
        if f.params.iter().any(|p| key_has_proc(&p.ty)) || key_has_proc(&f.ret) {
            return Some((f.line, f.col));
        }
        let mut cands: Cands = Vec::new();
        proc_stmts(&f.body, &mut cands);
        if let Some(best) = cands.into_iter().min() {
            return Some(best);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// G10 net: first Socket / Listener use (counterpart of foreign.py first_net_use)
// ---------------------------------------------------------------------------

fn key_has_net(key: &str) -> bool {
    fn has(t: &GType) -> bool {
        (t.owner.is_none() && (t.name == "socket" || t.name == "listener")) || t.args.iter().any(has)
    }
    match GType::try_from_key(key) {
        Ok(t) => has(&t),
        Err(_) => false,
    }
}

fn net_expr(e: &Expr, out: &mut Cands) {
    match e {
        Expr::Call(c) => {
            if is_net_op(&c.name) {
                out.push((c.line, c.col));
            }
            for a in &c.args {
                net_expr(a, out);
            }
        }
        Expr::Unary(x) => net_expr(&x.operand, out),
        Expr::Binary(x) => {
            net_expr(&x.left, out);
            net_expr(&x.right, out);
        }
        Expr::Index(x) => {
            net_expr(&x.array, out);
            net_expr(&x.index, out);
        }
        Expr::Ctor(x) => {
            if let Some(a) = &x.arg {
                net_expr(a, out);
            }
        }
        Expr::Quant(x) => {
            net_expr(&x.lo, out);
            net_expr(&x.hi, out);
            net_expr(&x.body, out);
            for t in &x.triggers {
                net_expr(t, out);
            }
        }
        Expr::StructLit(x) => {
            for f in &x.fields {
                net_expr(&f.value, out);
            }
        }
        Expr::EnumCtor(x) => {
            for a in &x.args {
                net_expr(a, out);
            }
        }
        Expr::FieldAccess(x) => net_expr(&x.base, out),
        Expr::IsVariant(x) => net_expr(&x.value, out),
        Expr::VariantField(x) => net_expr(&x.value, out),
        Expr::InoutArg(x) => net_expr(&x.expr, out),
        Expr::Move(_) | Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) => {}
    }
}

fn net_stmts(stmts: &[Stmt], out: &mut Cands) {
    for s in stmts {
        match s {
            Stmt::Let(l) => {
                if key_has_net(&l.ty) {
                    out.push((l.line, l.col));
                }
                net_expr(&l.value, out);
            }
            Stmt::Assign(a) => net_expr(&a.value, out),
            Stmt::IndexAssign(a) => {
                net_expr(&a.index, out);
                net_expr(&a.value, out);
            }
            Stmt::If(i) => {
                net_expr(&i.cond, out);
                net_stmts(&i.then, out);
                net_stmts(&i.els, out);
            }
            Stmt::While(w) => {
                net_expr(&w.cond, out);
                for inv in &w.invariants {
                    net_expr(inv, out);
                }
                net_expr(&w.decreases, out);
                net_stmts(&w.body, out);
            }
            Stmt::ForMap(w) => {
                net_expr(&w.place, out);
                for inv in &w.invariants {
                    net_expr(inv, out);
                }
                net_stmts(&w.body, out);
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    net_expr(v, out);
                }
            }
            Stmt::Assert(a) => net_expr(&a.cond, out),
            Stmt::Par(e) => {
                for c in &e.calls {
                    net_expr(c, out);
                }
            }
            Stmt::ExprStmt(e) => net_expr(&e.expr, out),
            Stmt::FieldAssign(f) => net_expr(&f.value, out),
            Stmt::Match(m) => {
                net_expr(&m.scrutinee, out);
                for a in &m.arms {
                    net_stmts(&a.body, out);
                }
            }
        }
    }
}

/// The first use of Socket, Listener or a net_* operation -> (line, col): functions in order; the signature (a Loop parameter or result) before the body, whose first use is the smallest source position. A function that
/// only declares `! net` is not a use.
pub fn first_net_use(prog: &Program) -> Option<(i64, i64)> {
    for f in &prog.functions {
        if f.params.iter().any(|p| key_has_net(&p.ty)) || key_has_net(&f.ret) {
            return Some((f.line, f.col));
        }
        let mut cands: Cands = Vec::new();
        net_stmts(&f.body, &mut cands);
        if let Some(best) = cands.into_iter().min() {
            return Some(best);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// G10 File runtime detection (counterpart of foreign.py uses_files)
// ---------------------------------------------------------------------------

fn key_has_file(key: &str) -> bool {
    fn has(t: &GType) -> bool {
        (t.owner.is_none() && t.name == "file") || t.args.iter().any(has)
    }
    match GType::try_from_key(key) {
        Ok(t) => has(&t),
        Err(_) => false,
    }
}

fn ty_has_file(t: &Option<String>) -> bool {
    t.as_deref().map_or(false, key_has_file)
}

fn g10_expr(e: &Expr) -> bool {
    let own = match e {
        Expr::Spent(_) => false,
        Expr::Call(c) => (is_catalog(&c.name) && c.name.starts_with("file_")) || ty_has_file(&c.ty),
        Expr::Var(v) => ty_has_file(&v.ty),
        Expr::Move(m) => ty_has_file(&m.ty),
        _ => false,
    };
    own || expr_kids(e).into_iter().any(g10_expr)
}

fn g10_stmts(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Let(l) => key_has_file(&l.ty) || g10_expr(&l.value),
        Stmt::Assign(a) => g10_expr(&a.value),
        Stmt::IndexAssign(a) => g10_expr(&a.index) || g10_expr(&a.value),
        Stmt::If(i) => g10_expr(&i.cond) || g10_stmts(&i.then) || g10_stmts(&i.els),
        Stmt::While(w) => {
            g10_expr(&w.cond) || w.invariants.iter().any(g10_expr) || g10_expr(&w.decreases) || g10_stmts(&w.body)
        }
        Stmt::ForMap(w) => {
            g10_expr(&w.place) || w.invariants.iter().any(g10_expr) || g10_stmts(&w.body)
        }
        Stmt::Break(_) | Stmt::Continue(_) => false,
        Stmt::Return(r) => r.value.as_ref().map_or(false, g10_expr),
        Stmt::Assert(a) => g10_expr(&a.cond),
        Stmt::Par(e) => e.calls.iter().any(g10_expr),
        Stmt::ExprStmt(e) => g10_expr(&e.expr),
        Stmt::FieldAssign(f) => g10_expr(&f.value),
        Stmt::Match(m) => g10_expr(&m.scrutinee) || m.arms.iter().any(|a| g10_stmts(&a.body)),
    })
}

/// Whether the program needs the G10 File runtime.
pub fn uses_files(prog: &Program) -> bool {
    prog.functions.iter().any(|f| {
        f.params.iter().any(|p| key_has_file(&p.ty))
            || key_has_file(&f.ret)
            || f.requires.iter().any(g10_expr)
            || f.ensures.iter().any(g10_expr)
            || f.cost.as_ref().map_or(false, g10_expr)
            || f.decreases.as_ref().map_or(false, g10_expr)
            || g10_stmts(&f.body)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_rule() {
        assert!(path_ok("::std::cmp::min"));
        assert!(path_ok("::a"));
        assert!(!path_ok("std::cmp"));
        assert!(!path_ok("::"));
        assert!(!path_ok("::a::"));
        assert!(!path_ok(":::a"));
        assert!(!path_ok("::1a"));
        assert!(!path_ok("::a b"));
    }

    #[test]
    fn catalog_shapes() {
        assert_eq!(CATALOG_NAMES.len(), 111);
        for n in CATALOG_NAMES.iter() {
            assert!(catalog_fn(n).is_some());
        }
        assert!(catalog_fn("len").is_none());
        let f = catalog_fn("vec_push").unwrap();
        assert!(f.params[0].take && !f.params[1].take);
        assert_eq!(f.tparams.len(), 1);
    }

    #[test]
    fn join_keeps_order_and_maybe() {
        let mut a = St::default();
        a.set("x", V::Live);
        a.set("y", V::Dead);
        let mut b = St::default();
        b.set("y", V::Dead);
        b.set("x", V::Dead);
        let j = join(Some(a), Some(b)).unwrap();
        assert_eq!(j.iter().cloned().collect::<Vec<_>>(), vec![("x".to_string(), V::Maybe), ("y".to_string(), V::Dead)]);
    }
}

/// Native targets cannot link the Rust scoped-thread bridge.
pub fn parallel_error(p: &Par) -> PruvError {
    PruvError::new("codegen", p.line, p.col, "parallel_target", "native obj/exe emission is unavailable for par", "emit LLVM IR with --rust-bindings and link using Rust")
}
pub fn first_par(stmts: &[Stmt]) -> Option<&Par> {
    stmts.iter().filter_map(|s| match s {
        Stmt::Par(p) => Some(p),
        Stmt::If(x) => [first_par(&x.then), first_par(&x.els)].into_iter().flatten().min_by_key(|p| (p.line,p.col)),
        Stmt::While(x) => first_par(&x.body),
        Stmt::ForMap(x) => first_par(&x.body),
        Stmt::Match(x) => x.arms.iter().filter_map(|a| first_par(&a.body)).min_by_key(|p| (p.line,p.col)),
        Stmt::Let(_) | Stmt::Assign(_) | Stmt::IndexAssign(_) | Stmt::FieldAssign(_) | Stmt::Return(_) | Stmt::Assert(_) | Stmt::ExprStmt(_) | Stmt::Break(_) | Stmt::Continue(_) => None,
    }).min_by_key(|p| (p.line,p.col))
}
pub fn g7_target_error(prog: &Program) -> Option<PruvError> {
    prog.functions.iter().find_map(|f| first_par(&f.body)).map(parallel_error)
}

fn key_has_async(key: &str) -> bool {
    fn has(t: &GType) -> bool {
        (t.owner.is_none() && t.name == "loop") || t.args.iter().any(has)
    }
    match GType::try_from_key(key) {
        Ok(t) => has(&t),
        Err(_) => false,
    }
}

fn async_expr(e: &Expr, out: &mut Cands, names: &HashSet<&str>) {
    match e {
        Expr::Call(c) => {
            if is_async_op(&c.name) && !names.contains(c.name.as_str()) {
                out.push((c.line, c.col));
            }
            for a in &c.args {
                async_expr(a, out, names);
            }
        }
        Expr::Unary(x) => async_expr(&x.operand, out, names),
        Expr::Binary(x) => {
            async_expr(&x.left, out, names);
            async_expr(&x.right, out, names);
        }
        Expr::Index(x) => {
            async_expr(&x.array, out, names);
            async_expr(&x.index, out, names);
        }
        Expr::Ctor(x) => {
            if let Some(a) = &x.arg {
                async_expr(a, out, names);
            }
        }
        Expr::Quant(x) => {
            async_expr(&x.lo, out, names);
            async_expr(&x.hi, out, names);
            async_expr(&x.body, out, names);
            for t in &x.triggers {
                async_expr(t, out, names);
            }
        }
        Expr::StructLit(x) => {
            for f in &x.fields {
                async_expr(&f.value, out, names);
            }
        }
        Expr::EnumCtor(x) => {
            for a in &x.args {
                async_expr(a, out, names);
            }
        }
        Expr::FieldAccess(x) => async_expr(&x.base, out, names),
        Expr::IsVariant(x) => async_expr(&x.value, out, names),
        Expr::VariantField(x) => async_expr(&x.value, out, names),
        Expr::InoutArg(x) => async_expr(&x.expr, out, names),
        Expr::Move(_) | Expr::Spent(_) | Expr::IntLit(_) | Expr::BoolLit(_) | Expr::StrLit(_) | Expr::Var(_) => {}
    }
}

fn async_stmts(stmts: &[Stmt], out: &mut Cands, names: &HashSet<&str>) {
    for s in stmts {
        match s {
            Stmt::Let(l) => {
                if key_has_async(&l.ty) {
                    out.push((l.line, l.col));
                }
                async_expr(&l.value, out, names);
            }
            Stmt::Assign(a) => async_expr(&a.value, out, names),
            Stmt::IndexAssign(a) => {
                async_expr(&a.index, out, names);
                async_expr(&a.value, out, names);
            }
            Stmt::If(i) => {
                async_expr(&i.cond, out, names);
                async_stmts(&i.then, out, names);
                async_stmts(&i.els, out, names);
            }
            Stmt::While(w) => {
                async_expr(&w.cond, out, names);
                for inv in &w.invariants {
                    async_expr(inv, out, names);
                }
                async_expr(&w.decreases, out, names);
                async_stmts(&w.body, out, names);
            }
            Stmt::ForMap(w) => {
                async_expr(&w.place, out, names);
                for inv in &w.invariants {
                    async_expr(inv, out, names);
                }
                async_stmts(&w.body, out, names);
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(r) => {
                if let Some(v) = &r.value {
                    async_expr(v, out, names);
                }
            }
            Stmt::Assert(a) => async_expr(&a.cond, out, names),
            Stmt::Par(e) => {
                for c in &e.calls {
                    async_expr(c, out, names);
                }
            }
            Stmt::ExprStmt(e) => async_expr(&e.expr, out, names),
            Stmt::FieldAssign(f) => async_expr(&f.value, out, names),
            Stmt::Match(m) => {
                async_expr(&m.scrutinee, out, names);
                for a in &m.arms {
                    async_stmts(&a.body, out, names);
                }
            }
        }
    }
}

/// The first use of Loop or a ev_* operation -> (line, col): functions in order; the signature (a Loop parameter or result) before the body, whose first use is the smallest source position. A function that
/// only declares `! async` is not a use.
pub fn first_async_use(prog: &Program) -> Option<(i64, i64)> {
    let names: HashSet<&str> = prog.functions.iter().map(|f| f.name.as_str()).collect();
    for f in &prog.functions {
        if f.params.iter().any(|p| key_has_async(&p.ty)) || key_has_async(&f.ret) {
            return Some((f.line, f.col));
        }
        let mut cands: Cands = Vec::new();
        async_stmts(&f.body, &mut cands, &names);
        if let Some(best) = cands.into_iter().min() {
            return Some(best);
        }
    }
    None
}


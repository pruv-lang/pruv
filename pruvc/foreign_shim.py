"""G6 A3 (docs/G6_SPEC.md section 13): the Rust shim of the LLVM bridge.

build_shim() returns the text of `mod pruv_g6_shim { .. }`, which gen_bindings() appends to the generated Rust host
bindings of a G6 program.  The module contains
  * the contract/G6 preludes the helpers need (the same texts the Rust source backend emits),
  * the bridge runtime: live-box counter (PRUV_G6_COUNT atexit report), catch_unwind guard (fflush(NULL) +
    exit(101)), checked conversions (bool, handle, borrowed slice, str),
  * exactly the A2 boundary helpers the program uses (texts produced by a codegen.Gen / ForeignGen created for the
    same program and check mode, so the checks and messages are identical to the Rust source backend),
  * one `#[no_mangle] pub extern "C"` adapter per helper, named `pruv_g6_v1_<helper name>`, one
    `pruv_g6_v1_drop__<C>` and `pruv_g6_v1_len__<C>` per collection type <C>, `pruv_g6_v1_string_bytes` (only when
    string_to_str is used; it replaces the A2 clone helper by a borrowed view) and
    `#[no_mangle] pub static pruv_g6_bridge_v1: u8 = 1;`.
<C> is the G3 structural encoding of the collection type: vec__i, vec__b, hashmap__i__b, string.

Bridge representations (section 13.2): i64 as i64; bool as i64 0/1 (anything else -> `pruv: invalid representation:
bool`); a collection handle as an opaque pointer to a Box of the concrete Rust collection; a borrowed array/str as
(ptr, i64 len); Option<i64|bool> results through (ptr tag_out, ptr val_out).  Consuming updates return the same
pointer (File updates consume the old box and return a fresh box).  The helper-name part of a symbol is the full A2 helper name (`pruv_g6_vec_push__i` ->
`pruv_g6_v1_pruv_g6_vec_push__i`, `trusted rust fn min2` -> `pruv_g6_v1_min2`).
"""
from vast import *
from typeinfo import encode, is_collection, uses_array_type, uses_str
from codegen import (Gen, rty, CONTRACT_PRELUDE_G6, CONTRACT_PRELUDE_U8, CONTRACT_PRELUDE_BOOL,
                     CONTRACT_PRELUDE_STR, G6_PRELUDE, FILE_PRELUDE, proc_prelude, net_prelude, async_prelude, async_proc_prelude)
from foreign import CATALOG, CATALOG_CONTRACT_ONLY, FILE_OBSERVERS, PROC_OBSERVERS, NET_OBSERVERS, ASYNC_OBSERVERS
from shared_data import load

SHIM_EDGES = load("shim_edges")["SHIM_EDGES"]

PREFIX = "pruv_g6_v1_"
BRIDGE_SYM = "pruv_g6_bridge_v1"
STRING_BYTES_SYM = PREFIX + "string_bytes"
STRING_TO_STR = "pruv_g6_string_to_str"       # A2 helper name replaced by the borrowed view string_bytes
STRING_TAKE_SYM = PREFIX + "string_take"        # P3 M3: takes the buffer of a dying String (emitted only when used)


def helper_name(op, src):
    """the A2 helper name of a catalog instance (ForeignGen.catalog_helper)"""
    return "pruv_g6_" + op + "".join("__" + encode(t) for t in src)


def adapter_sym(helper):
    return PREFIX + helper


def unbox_adapter_sym(name):
    """P3 M5: the unboxed-String adapter of the trusted function `name` (fills a str array, no String box)"""
    return PREFIX + name + "__str"


def cname(t):
    """<C>: G3 encoding of a collection type"""
    if t.name == "vec":
        return "vec__" + encode(t.args[0])
    if t.name == "hashmap":
        return "hashmap__%s__%s" % (encode(t.args[0]), encode(t.args[1]))
    return {"file": "File", "process": "Process", "process_args": "ProcessArgs",
            "socket": "Socket", "listener": "Listener", "loop": "Loop"}.get(t.name, "string")


def drop_sym(t):
    return "%sdrop__%s" % (PREFIX, cname(t))


def len_sym(t):
    return "%slen__%s" % (PREFIX, cname(t))


def inst_types(e):
    """(low, src) type arguments of a catalog call as codegen.catalog_call computes them"""
    low = tuple(e.inst["args"]) if e.inst else ()
    src = tuple(e.inst.get("src_args", low)) if e.inst else ()
    if not src and low:
        src = low
    return low, src


# ---------------------------------------------------------------------------
# program scan
# ---------------------------------------------------------------------------
def _walk(node, fn):
    if isinstance(node, (list, tuple)):
        for x in node:
            _walk(x, fn)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    fn(node)
    for fname in node.__dataclass_fields__:
        _walk(getattr(node, fname), fn)


def collect(prog):
    """(cats, colls, trusted): cats: helper name -> (op, low types); colls: <C> -> collection Type (sorted by the
    caller); trusted: name -> Function of the trusted declarations called from a body"""
    cats, colls, trusted = {}, {}, {}
    fmap = {}
    for f in prog.functions:
        fmap.setdefault(f.name, f)

    def add(t):
        if t is not None and is_collection(t):
            colls.setdefault(cname(t), t)

    def visit(n):
        if isinstance(n, Let):
            add(n.type)
        ty = getattr(n, "ty", None)
        if ty is not None:
            add(ty)
        if isinstance(n, Call):
            if n.name in CATALOG and n.name not in CATALOG_CONTRACT_ONLY:
                low, src = inst_types(n)
                cats.setdefault(helper_name(n.name, src), (n.name, low))
                # helper edges (incl. G10 proc/net A3 bounds guards) live in pruvc/data/shim_edges.json
                for caller, bound in SHIM_EDGES:
                    if n.name == caller:
                        cats.setdefault(helper_name(bound, ()), (bound, ()))
            else:
                f = fmap.get(n.name)
                if f is not None and f.foreign is not None:
                    trusted.setdefault(n.name, f)

    for f in prog.functions:
        if f.foreign is not None:
            continue
        for p in f.params:
            add(p.type)
        add(f.ret)
        _walk(f.body, visit)
        def contract_visit(n):
            if isinstance(n, Call) and (n.name in FILE_OBSERVERS or n.name in PROC_OBSERVERS or n.name in NET_OBSERVERS or n.name in ASYNC_OBSERVERS):
                visit(n)
        _walk(f.requires, contract_visit)
        _walk(f.ensures, contract_visit)
        _walk(getattr(f, "cost", None), contract_visit)
    return cats, colls, trusted


# ---------------------------------------------------------------------------
# bridge runtime text
# ---------------------------------------------------------------------------
BRIDGE_RUNTIME = """\
mod pruv_g6_c {
    extern "C" {
        pub fn fflush(stream: *mut ::std::ffi::c_void) -> i32;
        pub fn atexit(cb: extern "C" fn()) -> i32;
    }
}
type PruvH = *mut ::std::ffi::c_void;
#[no_mangle]
pub static pruv_g6_bridge_v1: u8 = 1;
static PRUV_G6_LIVE: ::std::sync::atomic::AtomicI64 = ::std::sync::atomic::AtomicI64::new(0);
static PRUV_G6_ONCE: ::std::sync::Once = ::std::sync::Once::new();
extern "C" fn pruv_g6_report() {
    let n: i64 = PRUV_G6_LIVE.load(::std::sync::atomic::Ordering::SeqCst);
    let _ = ::std::io::Write::write_fmt(&mut ::std::io::stderr(), format_args!("pruv-g6-live: {}\\n", n));
}
fn pruv_g6_register() {
    PRUV_G6_ONCE.call_once(|| {
        if ::std::env::var("PRUV_G6_COUNT").map(|v| v == "1").unwrap_or(false) {
            unsafe { pruv_g6_c::atexit(pruv_g6_report); }
        }
    });
}
fn pruv_g6_guard<R, F: FnOnce() -> R>(f: F) -> R {
    match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            unsafe { pruv_g6_c::fflush(::std::ptr::null_mut()); }
            ::std::process::exit(101)
        }
    }
}
fn pruv_g6_b(x: i64) -> bool { match x { 0 => false, 1 => true, _ => pruv_g6_repr("bool") } }
fn pruv_g6_new<T>(v: T) -> PruvH {
    pruv_g6_register();
    PRUV_G6_LIVE.fetch_add(1, ::std::sync::atomic::Ordering::SeqCst);
    ::std::boxed::Box::into_raw(::std::boxed::Box::new(v)) as PruvH
}
fn pruv_g6_ref<'a, T>(h: PruvH) -> &'a T {
    if h.is_null() { pruv_g6_repr("null handle") }
    unsafe { &*(h as *const T) }
}
fn pruv_g6_upd<T: Default, F: FnOnce(T) -> T>(h: PruvH, f: F) -> PruvH {
    if h.is_null() { pruv_g6_repr("null handle") }
    unsafe {
        let p: *mut T = h as *mut T;
        let old: T = ::std::mem::take(&mut *p);
        let new: T = f(old);
        *p = new;
    }
    h
}
fn pruv_g6_free<T>(h: PruvH) {
    if h.is_null() { return; }
    unsafe { drop(::std::boxed::Box::from_raw(h as *mut T)); }
    PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst);
}
fn pruv_g6_nlen(n: i64) -> usize { if n < 0 || n > (1i64 << 40) { pruv_g6_repr("length") } n as usize }
fn pruv_g6_slice<'a, T>(p: *const T, n: i64) -> &'a [T] {
    let n: usize = pruv_g6_nlen(n);
    if n == 0 { return &[]; }
    if p.is_null() { pruv_g6_repr("NULL data") }
    if (p as usize) % ::std::mem::align_of::<T>() != 0 { pruv_g6_repr("alignment") }
    unsafe { ::std::slice::from_raw_parts(p, n) }
}
fn pruv_g6_bools<'a>(p: *const u8, n: i64) -> &'a [bool] {
    let s: &[u8] = pruv_g6_slice::<u8>(p, n);
    for b in s.iter() { if *b > 1 { pruv_g6_repr("bool") } }
    unsafe { ::std::slice::from_raw_parts(s.as_ptr() as *const bool, s.len()) }
}
fn pruv_g6_str<'a>(p: *const u8, n: i64) -> &'a str {
    match ::std::str::from_utf8(pruv_g6_slice::<u8>(p, n)) { Ok(s) => s, Err(_) => pruv_g6_repr("utf-8") }
}
fn pruv_g6_out(p: *mut i64) -> *mut i64 { if p.is_null() { pruv_g6_repr("null output") } p }
"""

# P3 M3: a taken String buffer is freed by pruv_free_v2, which is the Rust allocator only under
# -DPRUV_RUST_ALLOCATOR; the take adapter references the runtime marker so a malloc runtime fails to link.
TAKE_RUNTIME = """\
mod pruv_g6_alloc {
    extern "C" {
        pub fn pruv_runtime_rust_allocator_v2();
    }
}
"""

MOD_ALLOW = ("#[allow(dead_code, unused_imports, unused_variables, unused_mut, unused_parens, unused_unsafe, "
             "non_snake_case, non_camel_case_types, non_upper_case_globals, improper_ctypes_definitions, "
             "unreachable_code, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]")


# ---------------------------------------------------------------------------
# adapters
# ---------------------------------------------------------------------------
def _cin(t, x):
    return x if t == I64 else "pruv_g6_b(%s)" % x


def _cout(t, x):
    return x if t == I64 else "(%s) as i64" % x


def _fn(sym, params, ret, body):
    return '#[no_mangle]\npub extern "C" fn %s(%s)%s {\n    %s\n}\n' % (
        sym, params, "" if ret is None else " -> " + ret, body)


def _vec_reserve_adapter(helper, elem, checked):
    vec = rty(Type("vec", (elem,)))
    guard = ('if n < 0 || n > (1i64 << 40) { pruv_g6_req("vec_reserve", "0 <= n && n <= 2^40"); } '
             if checked else "")
    return _fn(adapter_sym(helper), "v: PruvH, n: i64", None,
               'pruv_g6_guard(|| { %sif v.is_null() { pruv_g6_repr("null handle") } '
               'unsafe { (&mut *(v as *mut %s)).reserve(n as usize) } })' % (guard, vec))


def _cat_adapter(helper, op, low):
    S = adapter_sym(helper)
    if op.startswith("file_"):
        return _file_adapter(S, helper, op)
    if op.startswith("proc_"):
        return _proc_adapter(S, helper, op)
    if op.startswith("ev_"):
        return _async_adapter(S, helper, op)
    if op.startswith("net_"):
        return _net_adapter(S, helper, op)
    if op in ("vec_new", "vec_push", "vec_get"):
        T = low[0]
        C = rty(Type("vec", (T,)))
    elif op in ("map_new", "map_get", "map_insert"):
        K, V = low[0], low[1]
        C = rty(Type("hashmap", (K, V)))
    else:
        C = rty(Type("string"))
    if op in ("vec_new", "map_new", "string_new"):
        return _fn(S, "", "PruvH", "pruv_g6_guard(|| pruv_g6_new::<%s>(%s()))" % (C, helper))
    if op == "vec_push":
        return _fn(S, "v: PruvH, x: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_x: %s = %s; pruv_g6_upd::<%s, _>(v, |pruv_o| %s(pruv_o, pruv_x)) })" % (
                       rty(T), _cin(T, "x"), C, helper))
    if op == "vec_get":
        return _fn(S, "v: PruvH, i: i64", "i64",
                   "pruv_g6_guard(|| { let pruv_r: %s = %s(pruv_g6_ref::<%s>(v), i); %s })" % (
                       rty(T), helper, C, _cout(T, "pruv_r")))
    if op == "map_get":
        return _fn(S, "m: PruvH, k: i64, tag: *mut i64, val: *mut i64", None,
                   "pruv_g6_guard(|| { let pruv_k: %s = %s; let pruv_r: ::std::option::Option<%s> = %s(pruv_g6_ref::<%s>(m), pruv_k); "
                   "let pruv_t: *mut i64 = pruv_g6_out(tag); let pruv_v: *mut i64 = pruv_g6_out(val); "
                   "unsafe { match pruv_r { ::std::option::Option::Some(pruv_x) => { *pruv_t = 1; *pruv_v = %s; } "
                   "::std::option::Option::None => { *pruv_t = 0; *pruv_v = 0; } } } })" % (
                       rty(K), _cin(K, "k"), rty(V), helper, C, _cout(V, "pruv_x")))
    if op == "map_insert":
        return _fn(S, "m: PruvH, k: i64, v: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_k: %s = %s; let pruv_v: %s = %s; "
                   "pruv_g6_upd::<%s, _>(m, |pruv_o| %s(pruv_o, pruv_k, pruv_v)) })" % (
                       rty(K), _cin(K, "k"), rty(V), _cin(V, "v"), C, helper))
    if op == "string_from":
        return _fn(S, "p: *const u8, n: i64", "PruvH",
                   "pruv_g6_guard(|| pruv_g6_new(%s(pruv_g6_str(p, n))))" % helper)
    if op == "string_push_str":
        return _fn(S, "s: PruvH, p: *const u8, n: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_t: &str = pruv_g6_str(p, n); "
                   "pruv_g6_upd::<%s, _>(s, |pruv_o| %s(pruv_o, pruv_t)) })" % (C, helper))
    if op == "string_byte":
        return _fn(S, "s: PruvH, i: i64", "i64", "pruv_g6_guard(|| %s(pruv_g6_ref::<%s>(s), i))" % (helper, C))
    raise Exception("foreign_shim: no adapter for catalog operation %s" % op)


def _file_adapter(sym, helper, op):
    if op in ("file_open_read", "file_create"):
        return _fn(sym, "p: *const u8, n: i64", "PruvH",
                   "pruv_g6_guard(|| pruv_g6_new(%s(pruv_g6_str(p, n))))" % helper)
    params, arg = "h: PruvH", ""
    if op == "file_byte":
        params += ", i: i64"
        arg = ", i"
    if op in ("file_ok", "file_readable", "file_writable", "file_size", "file_byte"):
        return _fn(sym, params, "i64",
                   "pruv_g6_guard(|| %s(pruv_g6_ref::<pruv_File>(h)%s) as i64)" % (helper, arg))
    body = ""
    if op in ("file_write_str", "file_write_bytes"):
        params += ", p: *const u8, n: i64"
        arg = ", pruv_data"
        conv = "pruv_g6_str(p, n)" if op == "file_write_str" else "pruv_g6_slice::<u8>(p, n)"
        body += "let pruv_data = %s; " % conv
    # Consume the box before the A2 helper; File has no Default placeholder.
    body += ('if h.is_null() { pruv_g6_repr("null handle") } '
             "let f: pruv_File = unsafe { *::std::boxed::Box::from_raw(h as *mut pruv_File) }; "
             "PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); ")
    call = "%s(f%s)" % (helper, arg)
    body += call + " as i64" if op == "file_close" else "pruv_g6_new(%s)" % call
    return _fn(sym, params, "i64" if op == "file_close" else "PruvH", "pruv_g6_guard(|| { %s })" % body)


def _proc_consume(var, ty):
    """G10 proc A3: take a Process / ProcessArgs box out of its handle (no Default placeholder; the handle is dead)"""
    return ('if %s.is_null() { pruv_g6_repr("null handle") } '
            "let pruv_o: %s = unsafe { *::std::boxed::Box::from_raw(%s as *mut %s) }; "
            "PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); " % (var, ty, var, ty))


def _proc_adapter(sym, helper, op):
    """G10 proc A3 (G10_PROC_SPEC 6): adapters of the A2 helpers. Updates consume the box and return a new one; the
    observers borrow it and return i64 (bool as 0/1); wait / kill consume it and write (tag 1 = Ok, value); proc_bytes
    hands out a borrowed (data, len) view of the public buffer, which the LLVM caller copies at once."""
    P, A = "pruv_Process", "pruv_ProcessArgs"
    if op == "proc_args_new":
        return _fn(sym, "", "PruvH", "pruv_g6_guard(|| pruv_g6_new(%s()))" % helper)
    if op == "proc_args_push":
        return _fn(sym, "a: PruvH, p: *const u8, n: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_t: &str = pruv_g6_str(p, n); %spruv_g6_new(%s(pruv_o, pruv_t)) })" % (
                       _proc_consume("a", A), helper))
    if op == "proc_args_len":
        return _fn(sym, "a: PruvH", "i64", "pruv_g6_guard(|| %s(pruv_g6_ref::<%s>(a)))" % (helper, A))
    if op == "proc_spawn":
        return _fn(sym, "p: *const u8, n: i64, a: PruvH, q: *const u8, m: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_prog: &str = pruv_g6_str(p, n); let pruv_cwd: &str = pruv_g6_str(q, m); "
                   "pruv_g6_new(%s(pruv_prog, pruv_g6_ref::<%s>(a), pruv_cwd)) })" % (helper, A))
    if op == "proc_spawn_env":
        return _fn(sym, "p: *const u8, n: i64, a: PruvH, q: *const u8, m: i64, en: PruvH, ev: PruvH", "PruvH",
                   "pruv_g6_guard(|| { let pruv_prog: &str = pruv_g6_str(p, n); let pruv_cwd: &str = pruv_g6_str(q, m); "
                   "pruv_g6_new(%s(pruv_prog, pruv_g6_ref::<%s>(a), pruv_cwd, pruv_g6_ref::<%s>(en), pruv_g6_ref::<%s>(ev))) })" % (helper, A, A, A))
    if op in ("proc_write_str", "proc_write_bytes", "proc_close_stdin", "proc_read", "proc_read_all"):
        params, pre, arg = "h: PruvH", "", ""
        if op == "proc_write_str":
            params, pre, arg = params + ", p: *const u8, n: i64", "let pruv_d: &str = pruv_g6_str(p, n); ", ", pruv_d"
        elif op == "proc_write_bytes":
            params, pre, arg = params + ", p: *const u8, n: i64", "let pruv_d: &[u8] = pruv_g6_slice::<u8>(p, n); ", ", pruv_d"
        elif op == "proc_read":
            params, arg = params + ", stream: i64, max: i64", ", stream, max"
        return _fn(sym, params, "PruvH", "pruv_g6_guard(|| { %s%spruv_g6_new(%s(pruv_o%s)) })" % (
            pre, _proc_consume("h", P), helper, arg))
    if op in ("proc_wait", "proc_kill"):
        return _fn(sym, "h: PruvH, tag: *mut i64, val: *mut i64", None,
                   "pruv_g6_guard(|| { let pruv_t: *mut i64 = pruv_g6_out(tag); let pruv_v: *mut i64 = pruv_g6_out(val); "
                   "%slet pruv_r: ::std::result::Result<i64, i64> = %s(pruv_o); "
                   "unsafe { match pruv_r { ::std::result::Result::Ok(pruv_x) => { *pruv_t = 1; *pruv_v = pruv_x; } "
                   "::std::result::Result::Err(pruv_e) => { *pruv_t = 0; *pruv_v = pruv_e; } } } })" % (
                       _proc_consume("h", P), helper))
    if op == "proc_bytes":
        # the helper's requires (stream 1 or 2, by --foreign-checks) runs on the borrowed handle; the view itself is
        # the public buffer (pruv_proc_buf rejects any other stream as an invalid representation)
        return _fn(sym, "h: PruvH, stream: i64, data: *mut *const u8, len: *mut i64", None,
                   "pruv_g6_guard(|| { if data.is_null() || len.is_null() { pruv_g6_repr(\"null output\") } "
                   "let pruv_p: &%s = pruv_g6_ref::<%s>(h); let _ = %s(pruv_p, stream); "
                   "let pruv_b: &::std::vec::Vec<u8> = pruv_proc_buf(pruv_p, stream); "
                   "unsafe { *data = pruv_b.as_ptr(); *len = pruv_b.len() as i64; } })" % (P, P, helper))
    params, arg = "h: PruvH", ""
    if op in ("proc_eof", "proc_size"):
        params, arg = params + ", stream: i64", ", stream"
    elif op == "proc_byte":
        params, arg = params + ", stream: i64, i: i64", ", stream, i"
    if op in ("proc_ok", "proc_error", "proc_stdin_open", "proc_eof", "proc_size", "proc_byte"):
        return _fn(sym, params, "i64", "pruv_g6_guard(|| %s(pruv_g6_ref::<%s>(h)%s) as i64)" % (helper, P, arg))
    raise Exception("foreign_shim: no adapter for catalog operation %s" % op)


def _net_adapter(sym, helper, op):
    """G10 net A3 (G10_NET_SPEC 6): adapters of the A2 helpers, parallel to _proc_adapter. connect / listen build a new
    box; accept borrows the Listener and builds a Socket box; updates and reads consume the Socket box and return a
    new one; close / listener_close consume it and write (tag 1 = Ok, value 0); net_bytes hands out a borrowed
    (data, len) view of the ORIGINAL s.buf, which the LLVM caller copies at once; the observers borrow the handle and
    return i64 (bool as 0/1)."""
    S, L = "pruv_Socket", "pruv_Listener"
    if op in ("net_connect", "net_listen"):
        return _fn(sym, "p: *const u8, n: i64, port: i64", "PruvH",
                   "pruv_g6_guard(|| { let pruv_h: &str = pruv_g6_str(p, n); pruv_g6_new(%s(pruv_h, port)) })" % helper)
    if op == "net_accept":
        return _fn(sym, "l: PruvH", "PruvH", "pruv_g6_guard(|| pruv_g6_new(%s(pruv_g6_ref::<%s>(l))))" % (helper, L))
    if op in ("net_write_str", "net_write_bytes", "net_shutdown_write", "net_set_timeout", "net_read", "net_read_all"):
        params, pre, arg = "h: PruvH", "", ""
        if op == "net_write_str":
            params, pre, arg = params + ", p: *const u8, n: i64", "let pruv_d: &str = pruv_g6_str(p, n); ", ", pruv_d"
        elif op == "net_write_bytes":
            params, pre, arg = params + ", p: *const u8, n: i64", "let pruv_d: &[u8] = pruv_g6_slice::<u8>(p, n); ", ", pruv_d"
        elif op == "net_set_timeout":
            params, arg = params + ", ms: i64", ", ms"
        elif op == "net_read":
            params, arg = params + ", max: i64", ", max"
        return _fn(sym, params, "PruvH", "pruv_g6_guard(|| { %s%spruv_g6_new(%s(pruv_o%s)) })" % (
            pre, _proc_consume("h", S), helper, arg))
    if op in ("net_close", "net_listener_close"):
        T = L if op == "net_listener_close" else S
        return _fn(sym, "h: PruvH, tag: *mut i64, val: *mut i64", None,
                   "pruv_g6_guard(|| { let pruv_t: *mut i64 = pruv_g6_out(tag); let pruv_v: *mut i64 = pruv_g6_out(val); "
                   "%slet pruv_r: ::std::result::Result<i64, i64> = %s(pruv_o); "
                   "unsafe { match pruv_r { ::std::result::Result::Ok(pruv_x) => { *pruv_t = 1; *pruv_v = pruv_x; } "
                   "::std::result::Result::Err(pruv_e) => { *pruv_t = 0; *pruv_v = pruv_e; } } } })" % (
                       _proc_consume("h", T), helper))
    if op == "net_bytes":
        # the view is s.buf on the borrowed handle; the LLVM caller copies it once (no helper call, which would copy
        # the buffer again only to drop it)
        return _fn(sym, "h: PruvH, data: *mut *const u8, len: *mut i64", None,
                   "pruv_g6_guard(|| { if data.is_null() || len.is_null() { pruv_g6_repr(\"null output\") } "
                   "let pruv_s: &%s = pruv_g6_ref::<%s>(h); "
                   "unsafe { *data = pruv_s.buf.as_ptr(); *len = pruv_s.buf.len() as i64; } })" % (S, S))
    params, arg = "h: PruvH", ""
    if op == "net_byte":
        params, arg = params + ", i: i64", ", i"
    if op in ("net_ok", "net_error", "net_write_open", "net_eof", "net_size", "net_byte"):
        return _fn(sym, params, "i64", "pruv_g6_guard(|| %s(pruv_g6_ref::<%s>(h)%s) as i64)" % (helper, S, arg))
    if op in ("net_listener_ok", "net_listener_error", "net_listener_port"):
        return _fn(sym, params, "i64", "pruv_g6_guard(|| %s(pruv_g6_ref::<%s>(h)) as i64)" % (helper, L))
    raise Exception("foreign_shim: no adapter for catalog operation %s" % op)


def _trusted_params(f):
    params, args = [], []
    for i, p in enumerate(f.params):
        t = p.type
        if t == I64:
            params.append("a%d: i64" % i)
            args.append("a%d" % i)
        elif t == BOOL:
            params.append("a%d: i64" % i)
            args.append("pruv_g6_b(a%d)" % i)
        elif t == STR:
            params += ["a%d: *const u8" % i, "n%d: i64" % i]
            args.append("pruv_g6_str(a%d, n%d)" % (i, i))
        elif t == ARRAY:
            params += ["a%d: *const i64" % i, "n%d: i64" % i]
            args.append("pruv_g6_slice::<i64>(a%d, n%d)" % (i, i))
        elif t == ARRAY_U8:
            params += ["a%d: *const u8" % i, "n%d: i64" % i]
            args.append("pruv_g6_slice::<u8>(a%d, n%d)" % (i, i))
        elif t == ARRAY_BOOL:
            params += ["a%d: *const u8" % i, "n%d: i64" % i]
            args.append("pruv_g6_bools(a%d, n%d)" % (i, i))
        else:
            raise Exception("foreign_shim: unsupported trusted parameter type %r" % (t,))
    return params, args


def _unbox_adapter(f):
    """P3 M5: `let x: String = f(args)` kept as a plain owned str array: the String result is written as (data, len)
    with no box and no PRUV_G6_LIVE change; an empty String gives (NULL, 0).  into_boxed_str makes the block layout
    exactly (len, 1), as in the take adapter."""
    params, args = _trusted_params(f)
    return _fn(unbox_adapter_sym(f.name), ", ".join(params + ["data: *mut *mut u8", "len: *mut i64"]), None,
               "pruv_g6_guard(|| { "
               "let _ = ::std::hint::black_box(pruv_g6_alloc::pruv_runtime_rust_allocator_v2 as unsafe extern \"C\" fn()); "
               "if data.is_null() || len.is_null() { pruv_g6_repr(\"null output\") } "
               "let pruv_b: ::std::boxed::Box<str> = %s(%s).into_boxed_str(); "
               "let pruv_n: usize = pruv_b.len(); "
               "if pruv_n == 0 { drop(pruv_b); unsafe { *data = ::std::ptr::null_mut(); *len = 0; } } "
               "else { unsafe { *data = ::std::boxed::Box::into_raw(pruv_b) as *mut u8; *len = pruv_n as i64; } } })"
               % (f.name, ", ".join(args)))


def _trusted_adapter(f):
    params, args = _trusted_params(f)
    call = "%s(%s)" % (f.name, ", ".join(args))
    S = adapter_sym(f.name)
    if is_collection(f.ret):   # G6b: the helper checked the result; box it once, even when empty
        return _fn(S, ", ".join(params), "PruvH", "pruv_g6_guard(|| pruv_g6_new(%s))" % call)
    if f.ret == UNIT:
        return _fn(S, ", ".join(params), None, "pruv_g6_guard(|| { %s; })" % call)
    if f.ret == BOOL:
        return _fn(S, ", ".join(params), "i64", "pruv_g6_guard(|| { let pruv_r: bool = %s; pruv_r as i64 })" % call)
    return _fn(S, ", ".join(params), "i64", "pruv_g6_guard(|| %s)" % call)


def _coll_adapters(t):
    C = rty(t)
    if t.name in ("file", "process", "process_args", "socket", "listener", "loop"):   # G10: a handle has no len adapter
        return _fn(drop_sym(t), "h: PruvH", None, "pruv_g6_guard(|| pruv_g6_free::<pruv_%s>(h))" % cname(t))
    return (_fn(len_sym(t), "h: PruvH", "i64", "pruv_g6_guard(|| pruv_g6_ref::<%s>(h).len() as i64)" % C) +
            _fn(drop_sym(t), "h: PruvH", None, "pruv_g6_guard(|| pruv_g6_free::<%s>(h))" % C))


def _string_bytes_adapter():
    return _fn(STRING_BYTES_SYM, "h: PruvH, data: *mut *const u8, len: *mut i64", None,
               "pruv_g6_guard(|| { let pruv_s: &::std::string::String = pruv_g6_ref::<::std::string::String>(h); "
               "if data.is_null() || len.is_null() { pruv_g6_repr(\"null output\") } "
               "unsafe { *data = pruv_s.as_ptr(); *len = pruv_s.len() as i64; } })")


def _string_take_adapter():
    """P3 M3: consume the String box and hand its buffer to the caller as (data, len, cap); an empty String gives
    (NULL, 0, 0), like pruv_copy_v2 of length 0.  The block layout is (cap, 1), which pruv_free_v2 frees with the
    cap, so no shrinking realloc is needed."""
    return _fn(STRING_TAKE_SYM, "h: PruvH, data: *mut *mut u8, len: *mut i64, cap: *mut i64", None,
               "pruv_g6_guard(|| { "
               "let _ = ::std::hint::black_box(pruv_g6_alloc::pruv_runtime_rust_allocator_v2 as unsafe extern \"C\" fn()); "
               "if data.is_null() || len.is_null() || cap.is_null() { pruv_g6_repr(\"null output\") } "
               "if h.is_null() { pruv_g6_repr(\"null handle\") } "
               "let pruv_b: ::std::boxed::Box<::std::string::String> = "
               "unsafe { ::std::boxed::Box::from_raw(h as *mut ::std::string::String) }; "
               "PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); "
               "let mut pruv_s = ::std::mem::ManuallyDrop::new(*pruv_b); "
               "let pruv_n: usize = pruv_s.len(); "
               "if pruv_n == 0 { unsafe { ::std::mem::ManuallyDrop::drop(&mut pruv_s); "
               "*data = ::std::ptr::null_mut(); *len = 0; *cap = 0; } } "
               "else { unsafe { *data = pruv_s.as_mut_ptr(); *len = pruv_n as i64; *cap = pruv_s.capacity() as i64; } } })")


# ---------------------------------------------------------------------------
# the module
# ---------------------------------------------------------------------------
def prefix_adapter(text, pre):
    """P10H: put the program prefix `pre` (`<pkg>__` or "") in front of the name of every adapter `_fn` defined in
    `text` (they all read `pub extern "C" fn pruv_g6_v1_...`); the helper texts are never touched"""
    if not pre:
        return text
    key = 'pub extern "C" fn ' + PREFIX
    return text.replace(key, 'pub extern "C" fn ' + pre + PREFIX)


def build_shim(prog, verified=True, trust_callers=False, budget=1 << 30, foreign_checks="requires", take=False,
               unbox=(), pre=""):
    """text of `mod pruv_g6_shim` for the (concrete) program prog; deterministic.  take: the LLVM lowering used
    pruv_g6_v1_string_take (P3 M3), so the take adapter and the allocator marker reference are emitted;
    unbox: sorted names of the trusted String functions lowered unboxed (P3 M5), one `<f>__str` adapter each;
    pre: P10H symbol prefix (`<pkg>__`, or "" for a program without a package name) of every `pruv_g6_v1_*` adapter
    and of the `pruv_g6_bridge_v1` marker, as the .ll side references them"""
    cats, colls, trusted = collect(prog)
    # helper discovery: exactly the A2 helpers (same texts, same mode and budget).  The entry checks of the Rust
    # source backend are not emitted here (trust=True): the LLVM entries evaluate requires themselves.
    g = Gen(prog, verified, True, budget, foreign_checks)
    for f in prog.functions:
        g.fn(f)
    for op, low in cats.values():
        # vec_get: a verified Rust body inlines a proved read (R-6d-2), but the LLVM entries still call the helper;
        # vec_new/vec_push: the same for `vec_push(vec_new(), x)` inlined as `vec![x]` (P32L)
        if (op in FILE_OBSERVERS or op in PROC_OBSERVERS or op in NET_OBSERVERS or op in ASYNC_OBSERVERS
                or op in ("vec_get", "vec_new", "vec_push")):
            g.fg.catalog_helper(op, low, low)
    items = g.fg.items
    out = [MOD_ALLOW, "mod pruv_g6_shim {"]
    if g.g10:
        out.append(FILE_PRELUDE)
    if g.proc or g.ev:
        out.append(async_proc_prelude(g.proc_hook, g.async_hook) if g.ev else proc_prelude(g.proc_hook))
    if g.net or g.ev:
        out.append(net_prelude(g.net_hook))
    if g.ev:
        out.append(async_prelude(g.async_hook))
    out.append(CONTRACT_PRELUDE_G6)
    if uses_array_type(prog, "array_u8") or g.g10:
        out.append(CONTRACT_PRELUDE_U8)
    if uses_array_type(prog, "array_bool"):
        out.append(CONTRACT_PRELUDE_BOOL)
    if uses_str(prog) or uses_array_type(prog, "string"):   # G6b: a String result indexed in an ensures
        out.append(CONTRACT_PRELUDE_STR)
    out.append(G6_PRELUDE)
    out.append(BRIDGE_RUNTIME.replace("pub static " + BRIDGE_SYM, "pub static " + pre + BRIDGE_SYM) if pre
               else BRIDGE_RUNTIME)
    if take or unbox:
        out.append(TAKE_RUNTIME)
    adapters = []
    for name, text in items.items():
        if name in cats and cats[name][0] == "string_to_str":
            continue        # replaced by the borrowed view pruv_g6_v1_string_bytes
        if name not in cats and name not in trusted:
            continue        # contract scans (map_updated) are not used by the LLVM entries
        out.append(text)
        if name in cats:
            adapters.append(_cat_adapter(name, cats[name][0], cats[name][1]))
        else:
            adapters.append(_trusted_adapter(trusted[name]))
    for helper in sorted(h for h, (op, _) in cats.items() if op == "vec_reserve"):     # name order, as the Rust port
        adapters.append(_vec_reserve_adapter(helper, cats[helper][1][0], not g.unchecked_catalog()))
    if any(op == "string_to_str" for op, _ in cats.values()):
        adapters.append(_string_bytes_adapter())
        if take:
            adapters.append(_string_take_adapter())
    for name in unbox:
        if name in trusted:
            adapters.append(_unbox_adapter(trusted[name]))
    for key in sorted(colls):
        adapters.append(_coll_adapters(colls[key]))
    out.extend(prefix_adapter(a, pre) for a in adapters)
    out.append("}")
    if g.ev:
        out.append("pub use self::pruv_g6_shim::PRUV_ASYNC_THREADS;")
    return "\n".join(out) + "\n"


def _async_adapter(sym, helper, op):
    adapters = {
        'ev_new': ('a0: i64', 'PruvH', 'pruv_g6_guard(|| { pruv_g6_new({helper}(a0)) })'),
        'ev_close': ('a0: PruvH, tag: *mut i64, val: *mut i64', None, 'pruv_g6_guard(|| { let pruv_t = pruv_g6_out(tag); let pruv_v = pruv_g6_out(val); if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); let pruv_r = {helper}(pruv_o0); unsafe { match pruv_r { ::std::result::Result::Ok(x) => { *pruv_t = 1; *pruv_v = x; } ::std::result::Result::Err(e) => { *pruv_t = 0; *pruv_v = e; } } } })'),
        'ev_add_socket': ('a0: PruvH, a1: PruvH', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); if a1.is_null() { pruv_g6_repr("null handle") } let pruv_o1: pruv_Socket = unsafe { *::std::boxed::Box::from_raw(a1 as *mut pruv_Socket) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, pruv_o1)) })'),
        'ev_add_listener': ('a0: PruvH, a1: PruvH', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); if a1.is_null() { pruv_g6_repr("null handle") } let pruv_o1: pruv_Listener = unsafe { *::std::boxed::Box::from_raw(a1 as *mut pruv_Listener) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, pruv_o1)) })'),
        'ev_add_process': ('a0: PruvH, a1: PruvH', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); if a1.is_null() { pruv_g6_repr("null handle") } let pruv_o1: pruv_Process = unsafe { *::std::boxed::Box::from_raw(a1 as *mut pruv_Process) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, pruv_o1)) })'),
        'ev_add_timer': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_wait': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_read': ('a0: PruvH, a1: i64, a2: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, a2)) })'),
        'ev_read_into': ('a0: PruvH, a1: i64, data: *mut u8, len: i64, n: *mut i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); let len = pruv_g6_nlen(len); let dst = if len == 0 { &mut [] } else { if data.is_null() { pruv_g6_repr("NULL data") } unsafe { ::std::slice::from_raw_parts_mut(data, len) } }; pruv_g6_new({helper}(pruv_o0, a1, dst, unsafe { &mut *pruv_g6_out(n) })) })'),
        'ev_write': ('a0: PruvH, a1: i64, a2: *const u8, n2: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, pruv_g6_slice::<u8>(a2, n2))) })'),
        'ev_shutdown_write': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_accept': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_proc_read': ('a0: PruvH, a1: i64, a2: i64, a3: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, a2, a3)) })'),
        'ev_proc_read_into': ('a0: PruvH, a1: i64, stream: i64, data: *mut u8, len: i64, n: *mut i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); let len = pruv_g6_nlen(len); let dst = if len == 0 { &mut [] } else { if data.is_null() { pruv_g6_repr("NULL data") } unsafe { ::std::slice::from_raw_parts_mut(data, len) } }; pruv_g6_new({helper}(pruv_o0, a1, stream, dst, unsafe { &mut *pruv_g6_out(n) })) })'),
        'ev_proc_write': ('a0: PruvH, a1: i64, a2: *const u8, n2: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, pruv_g6_slice::<u8>(a2, n2))) })'),
        'ev_proc_close_stdin': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_proc_wait': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_proc_wait_for': ('a0: PruvH, a1: i64, a2: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, a2)) })'),
        'ev_proc_write_for': ('a0: PruvH, a1: i64, a2: *const u8, n2: i64, a3: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1, pruv_g6_slice::<u8>(a2, n2), a3)) })'),
        'ev_remove': ('a0: PruvH, a1: i64', 'PruvH', 'pruv_g6_guard(|| { if a0.is_null() { pruv_g6_repr("null handle") } let pruv_o0: pruv_Loop = unsafe { *::std::boxed::Box::from_raw(a0 as *mut pruv_Loop) }; PRUV_G6_LIVE.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); pruv_g6_new({helper}(pruv_o0, a1)) })'),
        'ev_bytes': ('a0: PruvH, data: *mut *const u8, len: *mut i64', None, 'pruv_g6_guard(|| { if data.is_null() || len.is_null() { pruv_g6_repr("null output") } let pruv_l: &pruv_Loop = pruv_g6_ref::<pruv_Loop>(a0); unsafe { *data = pruv_l.buf.as_ptr(); *len = pruv_l.buf.len() as i64; } })'),
        'ev_ok': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_error': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_cap': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_count': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_last': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_kind': ('a0: PruvH, a1: i64', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0), a1) as i64 })'),
        'ev_eof': ('a0: PruvH, a1: i64', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0), a1) as i64 })'),
        'ev_write_open': ('a0: PruvH, a1: i64', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0), a1) as i64 })'),
        'ev_event_slot': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_event_kind': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_size': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
        'ev_byte': ('a0: PruvH, a1: i64', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0), a1) as i64 })'),
        'ev_exit': ('a0: PruvH', 'i64', 'pruv_g6_guard(|| { {helper}(pruv_g6_ref::<pruv_Loop>(a0)) as i64 })'),
    }
    params, ret, body = adapters[op]
    return _fn(sym, params, ret, body.replace("{helper}", helper))

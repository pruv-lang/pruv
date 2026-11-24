"""G6 A0 (docs/G6_SPEC.md section 10): the fixed standard catalog, validation of `trusted rust fn` declarations,
the ownership/liveness checker for collection bindings, and the A0 guard `first_g6_use`.

Everything here runs after (or inside) type checking and only looks at typed ASTs; a program without G6 syntax
passes through unchanged."""
import re

from vast import *
from typeinfo import TypeTable, fn_owner, is_array, is_collection, contains_collection, is_owned_field_type, is_nested_collection_field, type_name, is_nongeneric_struct, field_place, is_owned_result, is_owned_value
from refine import _kids as expr_kids


def _err(node, code, msg, fix=""):
    raise PruvError("type", node.line, node.col, code, msg, fix, file=getattr(node, "file", None))   # G4: file only in explicit mode


# ---------------------------------------------------------------------------
# catalog (section 10.3): pseudo declarations without bodies, typed through the ordinary generic call path
# ---------------------------------------------------------------------------
def _p(name, ty, take=False):
    p = Param(name, ty)
    if take:
        p.take = True
    return p


def _vec(t):
    return Type("vec", (t,))


def _map(k, v):
    return Type("hashmap", (k, v))


STRING = Type("string")
FILE = Type("file")
PROCESS = Type("process")
PROCESS_ARGS = Type("process_args")
SOCKET = Type("socket")
LISTENER = Type("listener")
LOOP = Type("loop")
CATALOG = {}


def _add(name, bounds, build, effects=()):
    owner = fn_owner(name)
    ps = [mk_param(owner, i, n) for i, (n, _) in enumerate(bounds)]
    params, ret = build(*ps)
    f = Function(0, 0, name, params, ret, list(effects), [], [], [])
    f.tparams = [TypeParam(0, 0, n, b) for n, b in bounds]
    CATALOG[name] = f


_T = [("T", None)]
_KV = [("K", "Element"), ("V", None)]
_add("vec_new", _T, lambda T: ([], _vec(T)), ["alloc"])
_add("vec_push", _T, lambda T: ([_p("v", _vec(T), True), _p("x", T)], _vec(T)), ["alloc"])
_add("vec_get", _T, lambda T: ([_p("v", _vec(T)), _p("i", I64)], T))
_add("map_new", _KV, lambda K, V: ([], _map(K, V)), ["alloc"])
_add("map_get", _KV, lambda K, V: ([_p("m", _map(K, V)), _p("k", K)], mk_option(V)))
_add("map_get_mut", _KV, lambda K, V: ([_p("m", _map(K, V)), _p("k", K)], mk_option(V)))
CATALOG["map_get_mut"].params[0].mode = "inout"
_add("map_remove", _KV, lambda K, V: ([_p("m", _map(K, V)), _p("k", K)], BOOL))
CATALOG["map_remove"].params[0].mode = "inout"
_add("map_take", _KV, lambda K, V: ([_p("m", _map(K, V)), _p("k", K)], mk_option(V)))
CATALOG["map_take"].params[0].mode = "inout"
_add("vec_clear", _T, lambda T: ([_p("v", _vec(T))], UNIT))
CATALOG["vec_clear"].params[0].mode = "inout"
_add("vec_reserve", _T, lambda T: ([_p("v", _vec(T)), _p("n", I64)], UNIT), ["alloc"])
CATALOG["vec_reserve"].params[0].mode = "inout"
_add("vec_extend_from_slice", _T, lambda T: ([_p("v", _vec(T)), _p("s", mk_array(T))], UNIT), ["alloc"])   # D95
CATALOG["vec_extend_from_slice"].params[0].mode = "inout"
_add("string_eq", [], lambda: ([_p("a", STR), _p("b", STR)], BOOL))
_add("vec_at_mut", _T, lambda T: ([_p("v", _vec(T)), _p("i", I64)], mk_option(T)))
CATALOG["vec_at_mut"].params[0].mode = "inout"

_add("map_insert", _KV, lambda K, V: ([_p("m", _map(K, V), True), _p("k", K), _p("v", V)], _map(K, V)), ["alloc"])
_add("string_new", [], lambda: ([], STRING), ["alloc"])
_add("string_from", [], lambda: ([_p("text", STR)], STRING), ["alloc"])
_add("string_push_str", [], lambda: ([_p("s", STRING, True), _p("text", STR)], STRING), ["alloc"])
_add("string_push_char", [], lambda: ([_p("s", STRING, True), _p("c", I64)], STRING), ["alloc"])   # LB9 (R-6g8-2)
_add("string_byte", [], lambda: ([_p("s", STRING), _p("i", I64)], I64))
_add("string_to_str", [], lambda: ([_p("s", STRING)], STR), ["alloc"])
# G10: signatures/effects; section 2.3 contracts are installed below.
_add("file_open_read", [], lambda: ([_p("path", STR)], FILE), ["fs"])
_add("file_create", [], lambda: ([_p("path", STR)], FILE), ["fs"])
_add("file_read_all", [], lambda: ([_p("f", FILE, True)], FILE), ["fs", "alloc"])
_add("file_write_str", [], lambda: ([_p("f", FILE, True), _p("text", STR)], FILE), ["fs"])
_add("file_write_bytes", [], lambda: ([_p("f", FILE, True), _p("data", ARRAY_U8)], FILE), ["fs"])
_add("file_close", [], lambda: ([_p("f", FILE, True)], BOOL), ["fs"])
_add("file_ok", [], lambda: ([_p("f", FILE)], BOOL))
_add("file_readable", [], lambda: ([_p("f", FILE)], BOOL))
_add("file_writable", [], lambda: ([_p("f", FILE)], BOOL))
_add("file_size", [], lambda: ([_p("f", FILE)], I64))
_add("file_byte", [], lambda: ([_p("f", FILE), _p("i", I64)], I64))
# G10 proc (docs/G10_PROC_SPEC.md 2.3): signatures and effects. Since A1 the section 2.3 / 2.4 / 4 contracts (requires,
# fresh result, well-formedness, ensures, preservation facts) are encoded for every proc_* operation by
# verify.py FnVerifier.proc_call (and its Rust twin in pruv-proof symbolic.rs), because the Process record and its
# byte-map equalities are not expressible as typed contract ASTs. Both backends lower Process programs (A2 Rust, A3 LLVM).
_add("proc_args_new", [], lambda: ([], PROCESS_ARGS), ["alloc"])
_add("proc_args_push", [], lambda: ([_p("a", PROCESS_ARGS, True), _p("arg", STR)], PROCESS_ARGS), ["alloc"])
_add("proc_args_len", [], lambda: ([_p("a", PROCESS_ARGS)], I64))
_add("proc_spawn", [], lambda: ([_p("program", STR), _p("args", PROCESS_ARGS), _p("cwd", STR)], PROCESS),
     ["proc", "alloc"])
_add("proc_spawn_env", [], lambda: ([_p("program", STR), _p("args", PROCESS_ARGS), _p("cwd", STR), _p("env_names", PROCESS_ARGS), _p("env_values", PROCESS_ARGS)], PROCESS),
     ["proc", "alloc"])
_add("proc_write_str", [], lambda: ([_p("p", PROCESS, True), _p("text", STR)], PROCESS), ["proc", "alloc"])
_add("proc_write_bytes", [], lambda: ([_p("p", PROCESS, True), _p("data", ARRAY_U8)], PROCESS), ["proc", "alloc"])
_add("proc_close_stdin", [], lambda: ([_p("p", PROCESS, True)], PROCESS), ["proc", "alloc"])
_add("proc_read", [], lambda: ([_p("p", PROCESS, True), _p("stream", I64), _p("max", I64)], PROCESS),
     ["proc", "alloc"])
_add("proc_read_all", [], lambda: ([_p("p", PROCESS, True)], PROCESS), ["proc", "alloc"])
_add("proc_bytes", [], lambda: ([_p("p", PROCESS), _p("stream", I64)], ARRAY_U8), ["alloc"])
_add("proc_wait", [], lambda: ([_p("p", PROCESS, True)], RESULT), ["proc", "alloc"])
_add("proc_kill", [], lambda: ([_p("p", PROCESS, True)], RESULT), ["proc", "alloc"])
_add("proc_ok", [], lambda: ([_p("p", PROCESS)], BOOL))
_add("proc_error", [], lambda: ([_p("p", PROCESS)], I64))
_add("proc_stdin_open", [], lambda: ([_p("p", PROCESS)], BOOL))
_add("proc_eof", [], lambda: ([_p("p", PROCESS), _p("stream", I64)], BOOL))
_add("proc_size", [], lambda: ([_p("p", PROCESS), _p("stream", I64)], I64))
_add("proc_byte", [], lambda: ([_p("p", PROCESS), _p("stream", I64), _p("i", I64)], I64))
# G10 net (docs/G10_NET_SPEC.md 2.3): signatures and effects. Since A1 the section 2.3 / 2.4 / 4 contracts (requires,
# fresh result, well-formedness, ensures, preservation facts) are encoded for every net_* operation by
# verify.py FnVerifier.net_call (and its Rust twin in pruv-proof symbolic.rs). Both backends lower Socket / Listener
# (A2 Rust source backend, codegen.NET_PRELUDE; A3 LLVM backend through the Rust shim).
_add("net_listen", [], lambda: ([_p("host", STR), _p("port", I64)], LISTENER), ["net", "alloc"])
_add("net_accept", [], lambda: ([_p("l", LISTENER)], SOCKET), ["net", "alloc"])
_add("net_listener_close", [], lambda: ([_p("l", LISTENER, True)], RESULT), ["net", "alloc"])
_add("net_connect", [], lambda: ([_p("host", STR), _p("port", I64)], SOCKET), ["net", "alloc"])
_add("net_write_str", [], lambda: ([_p("s", SOCKET, True), _p("text", STR)], SOCKET), ["net", "alloc"])
_add("net_write_bytes", [], lambda: ([_p("s", SOCKET, True), _p("data", ARRAY_U8)], SOCKET), ["net", "alloc"])
_add("net_shutdown_write", [], lambda: ([_p("s", SOCKET, True)], SOCKET), ["net", "alloc"])
_add("net_set_timeout", [], lambda: ([_p("s", SOCKET, True), _p("ms", I64)], SOCKET), ["net", "alloc"])
_add("net_read", [], lambda: ([_p("s", SOCKET, True), _p("max", I64)], SOCKET), ["net", "alloc"])
_add("net_read_all", [], lambda: ([_p("s", SOCKET, True)], SOCKET), ["net", "alloc"])
_add("net_bytes", [], lambda: ([_p("s", SOCKET)], ARRAY_U8), ["alloc"])
_add("net_close", [], lambda: ([_p("s", SOCKET, True)], RESULT), ["net", "alloc"])
_add("net_ok", [], lambda: ([_p("s", SOCKET)], BOOL))
_add("net_error", [], lambda: ([_p("s", SOCKET)], I64))
_add("net_write_open", [], lambda: ([_p("s", SOCKET)], BOOL))
_add("net_eof", [], lambda: ([_p("s", SOCKET)], BOOL))
_add("net_size", [], lambda: ([_p("s", SOCKET)], I64))
_add("net_byte", [], lambda: ([_p("s", SOCKET), _p("i", I64)], I64))
_add("net_listener_ok", [], lambda: ([_p("l", LISTENER)], BOOL))
_add("net_listener_error", [], lambda: ([_p("l", LISTENER)], I64))
_add("net_listener_port", [], lambda: ([_p("l", LISTENER)], I64))

# G10 async A1: full catalog contracts are encoded in the symbolic verifier.
_add('ev_new', [], lambda: ([_p('cap', I64)], LOOP), ['async', 'alloc'])
_add('ev_close', [], lambda: ([_p('l', LOOP, True)], RESULT), ['async', 'net', 'proc', 'alloc'])
_add('ev_add_socket', [], lambda: ([_p('l', LOOP, True), _p('s', SOCKET, True)], LOOP), ['async', 'net', 'alloc'])
_add('ev_add_listener', [], lambda: ([_p('l', LOOP, True), _p('x', LISTENER, True)], LOOP), ['async', 'net', 'alloc'])
_add('ev_add_process', [], lambda: ([_p('l', LOOP, True), _p('p', PROCESS, True)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_add_timer', [], lambda: ([_p('l', LOOP, True), _p('ms', I64)], LOOP), ['async', 'alloc'])
_add('ev_wait', [], lambda: ([_p('l', LOOP, True), _p('timeout_ms', I64)], LOOP), ['async', 'alloc'])
_add('ev_read', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('max', I64)], LOOP), ['async', 'net', 'alloc'])
_add('ev_read_into', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('dst', ARRAY_U8), _p('n', I64)], LOOP), ['async', 'net', 'alloc'])
for _param in CATALOG['ev_read_into'].params[-2:]:
    _param.mode = 'inout'
_add('ev_write', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('data', ARRAY_U8)], LOOP), ['async', 'net', 'alloc'])
_add('ev_shutdown_write', [], lambda: ([_p('l', LOOP, True), _p('slot', I64)], LOOP), ['async', 'net', 'alloc'])
_add('ev_accept', [], lambda: ([_p('l', LOOP, True), _p('slot', I64)], LOOP), ['async', 'net', 'alloc'])
_add('ev_proc_read', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('stream', I64), _p('max', I64)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_proc_read_into', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('stream', I64), _p('dst', ARRAY_U8), _p('n', I64)], LOOP), ['async', 'proc', 'alloc'])
for _param in CATALOG['ev_proc_read_into'].params[-2:]:
    _param.mode = 'inout'
_add('ev_proc_write', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('data', ARRAY_U8)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_proc_close_stdin', [], lambda: ([_p('l', LOOP, True), _p('slot', I64)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_proc_wait', [], lambda: ([_p('l', LOOP, True), _p('slot', I64)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_proc_wait_for', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('timeout_ms', I64)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_proc_write_for', [], lambda: ([_p('l', LOOP, True), _p('slot', I64), _p('data', ARRAY_U8), _p('timeout_ms', I64)], LOOP), ['async', 'proc', 'alloc'])
_add('ev_remove', [], lambda: ([_p('l', LOOP, True), _p('slot', I64)], LOOP), ['async', 'net', 'proc', 'alloc'])
_add('ev_bytes', [], lambda: ([_p('l', LOOP)], ARRAY_U8), ['alloc'])
_add('ev_ok', [], lambda: ([_p('l', LOOP)], BOOL), [])
_add('ev_error', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_cap', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_count', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_last', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_kind', [], lambda: ([_p('l', LOOP), _p('slot', I64)], I64), [])
_add('ev_eof', [], lambda: ([_p('l', LOOP), _p('slot', I64)], BOOL), [])
_add('ev_write_open', [], lambda: ([_p('l', LOOP), _p('slot', I64)], BOOL), [])
_add('ev_event_slot', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_event_kind', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_size', [], lambda: ([_p('l', LOOP)], I64), [])
_add('ev_byte', [], lambda: ([_p('l', LOOP), _p('i', I64)], I64), [])
_add('ev_exit', [], lambda: ([_p('l', LOOP)], I64), [])

# contract-only observers
_add("map_lookup", _KV, lambda K, V: ([_p("m", _map(K, V)), _p("k", K)], mk_option(V)))
_add("map_updated", _KV, lambda K, V: ([_p("after", _map(K, V)), _p("before", _map(K, V)), _p("k", K), _p("v", V)],
                                       BOOL))
CATALOG_NAMES = tuple(CATALOG)
MUTABLE_COLLECTION_MATCH = ("map_get_mut", "vec_at_mut")
BORROWED_MAP_KEY = ("map_get", "map_get_mut", "map_remove", "map_take", "map_lookup")
CATALOG_CONTRACT_ONLY = ("map_lookup", "map_updated")
FILE_OBSERVERS = ("file_ok", "file_readable", "file_writable", "file_size", "file_byte")
# G10 proc A0: the observers allowed in contracts; every other proc_* operation (effects, allocating proc_bytes)
# is type/foreign_contract there.
PROC_OBSERVERS = ("proc_args_len", "proc_ok", "proc_error", "proc_stdin_open", "proc_eof", "proc_size", "proc_byte")
PROC_OPS = tuple(n for n in CATALOG if n.startswith("proc_"))
# G10 net A0: the observers allowed in contracts; every other net_* operation (effects, allocating net_bytes) is
# type/foreign_contract there.
NET_OBSERVERS = ("net_ok", "net_error", "net_write_open", "net_eof", "net_size", "net_byte",
                 "net_listener_ok", "net_listener_error", "net_listener_port")
NET_OPS = tuple(n for n in CATALOG if n.startswith("net_"))
ASYNC_OBSERVERS = ('ev_ok', 'ev_error', 'ev_cap', 'ev_count', 'ev_last', 'ev_kind', 'ev_eof', 'ev_write_open', 'ev_event_slot', 'ev_event_kind', 'ev_size', 'ev_byte', 'ev_exit')
ASYNC_OPS = tuple(n for n in CATALOG if n.startswith("ev_"))


# G10 A1: typed contract ASTs, evaluated by the catalog-call verifier.
def _file_contracts():
    def var(name, ty=FILE):
        return Var(0, 0, name, ty)

    def obs(name, value):
        return Call(0, 0, "file_" + name, [value], I64 if name == "size" else BOOL)

    def binary(op, a, b):
        return Binary(0, 0, op, a, b, BOOL)

    def neg(a):
        return Unary(0, 0, "!", a, BOOL)

    zero = IntLit(0, 0, 0, I64)
    result, f = var("result"), var("f")
    ok = obs("ok", result)
    size = obs("size", result)
    read = [binary("==", obs("readable", result), ok), neg(obs("writable", result)),
            binary("==", size, zero)]
    write = [binary("==", obs("writable", result), ok), neg(obs("readable", result)),
             binary("==", size, zero)]
    CATALOG["file_open_read"].ensures = read
    CATALOG["file_create"].ensures = write
    CATALOG["file_read_all"].requires = [obs("readable", f)]
    CATALOG["file_read_all"].ensures = [
        neg(obs("writable", result)), binary("==", obs("readable", result), ok),
        binary("==>", ok, binary(">=", size, zero)),
        binary("==>", neg(ok), binary("==", size, zero))]
    for name in ("file_write_str", "file_write_bytes"):
        CATALOG[name].requires = [obs("writable", f)]
        CATALOG[name].ensures = [neg(obs("readable", result)), write[0], write[2]]
    CATALOG["file_close"].ensures = [binary("==>", var("result", BOOL), obs("ok", f))]
    r, i = var("result", I64), var("i", I64)
    CATALOG["file_size"].ensures = [binary(">=", r, zero)]
    CATALOG["file_byte"].requires = [
        binary("&&", binary("<=", zero, i), binary("<", i, obs("size", f)))]
    CATALOG["file_byte"].ensures = [
        binary("&&", binary("<=", zero, r), binary("<=", r, IntLit(0, 0, 255, I64)))]


_file_contracts()


# ---------------------------------------------------------------------------
# trusted rust fn declarations (section 10.4)
# ---------------------------------------------------------------------------
_PATH_RE = re.compile(r"::[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*")


def _scalar_opt(t):
    """Option<i64|bool> / Result<i64|bool>"""
    return t.owner is None and t.name in ("option", "result") and t.args in ((), (I64,), (BOOL,))


def _owned_result(t):
    """G6b: catalog String, Vec<i64> or Vec<String> (structural: no nominal owner)"""
    return t.owner is None and t in (STRING, _vec(I64), _vec(STRING))


def _allowed_result(t):
    return t in (I64, BOOL, UNIT) or _scalar_opt(t) or _owned_result(t)


def _allowed_param(t):
    return t in (I64, BOOL, ARRAY, ARRAY_U8, ARRAY_BOOL, STR) or _scalar_opt(t)



def _consumed_field(v, nxt):
    """G-R09-4 / LG9 (G-LG8-1): the `move d.a` that consumes d: the let initializer itself, or the one field move
    passed directly as a call argument (the type checker admits it only for a take parameter), with no restore of
    that field by the next statement"""
    if isinstance(v, Call):
        moves = [a for a in v.args if isinstance(a, Move) and a.fields]
        v = moves[0] if len(moves) == 1 else None
    if (isinstance(v, Move) and v.fields and not (
            isinstance(nxt, FieldAssign) and nxt.name == v.name and tuple(nxt.fields) == tuple(v.fields))):
        return v
    return None

class _Pos:
    def __init__(s, line, col, file=None):
        s.line, s.col, s.file = line, col, file


def check_foreign_decl(f, externs):
    """signature (foreign_signature), then path (foreign_path) of a trusted rust fn"""
    if f.tparams:
        _err(f, "foreign_signature", "a trusted rust fn cannot be generic",
             "declare one concrete trusted rust fn per type")
    if not _allowed_result(f.ret):
        _err(f, "foreign_signature", "result type %s is not allowed on a trusted rust fn" % type_name(f.ret),
             "return i64, bool, (), Option<i64|bool>, Result<i64|bool>, String, Vec<i64> or Vec<String>; convert other results in a Rust adapter")
    for p in f.params:
        if p.mode is not None:     # G5 A0: rejected whether or not the declaration is ever called
            _err(_Pos(p.mode_pos[0], p.mode_pos[1], getattr(p, "file", None) or getattr(f, "file", None)),
                 "foreign_signature", "inout parameter %s is not allowed on a trusted rust fn" % p.name,
                 "remove `inout`; a foreign function only reads its arguments")
        if p.take:
            _err(f, "foreign_signature", "take parameter %s is not allowed on a trusted rust fn" % p.name,
                 "remove `take`; a foreign function only reads its arguments")
        if contains_collection(p.type):
            _err(f, "foreign_signature", "collection parameter %s is not allowed on a trusted rust fn" % p.name,
                 "pass i64, bool, Option, Result, arrays or str; wrap collections in a Rust adapter")
        if not _allowed_param(p.type):
            _err(f, "foreign_signature", "parameter %s of type %s is not allowed on a trusted rust fn" % (
                p.name, type_name(p.type)),
                 "use i64, bool, Option<i64|bool>, Result<i64|bool>, [i64], [u8], [bool] or str")
    if not f.ensures:
        _err(f, "foreign_signature", "a trusted rust fn needs at least one ensures",
             "add an ensures clause; `ensures true` documents an unconstrained result")
    info = f.foreign
    at = _Pos(info["line"], info["col"], getattr(f, "file", None))
    path = info["path"]
    if not _PATH_RE.fullmatch(path):
        _err(at, "foreign_path", "invalid Rust path",
             "write the path as ::std::cmp::min: `::` and identifiers only, no expressions, generics or spaces")
    root = path[2:].split("::")[0]
    if root in ("crate", "self", "super"):
        _err(at, "foreign_path", "relative Rust path root",
             "start the path with ::std or the name of a crate given with --rust-extern")
    if root != "std" and root not in externs:
        _err(at, "foreign_path", "Rust crate %s is not supplied" % root,
             "pass --rust-extern %s=PATH or use a ::std path" % root)


# ---------------------------------------------------------------------------
# ownership / liveness (section 10.4)
# State: dict name -> "live" | "dead" | "maybe" for owned bindings; None = unreachable path.
# Borrowed (unmarked collection) parameters are kept in self.bor and never change.
# ---------------------------------------------------------------------------
def _join(a, b):
    if a is None:
        return b
    if b is None:
        return a
    return dict((n, v if b[n] == v else "maybe") for n, v in a.items() if n in b)


_MUST_CONSUME = {"process": "Process", "socket": "Socket", "listener": "Listener", "loop": "Loop"}


def _must_kind(t):
    """G10 net A0 (G10_NET_SPEC 3.3): "Process" / "Socket" / "Listener" for a must-consume handle type, else None.
    File, String, ProcessArgs, Vec and HashMap are never must-consume."""
    if t is None or t.owner is not None:
        return None
    return _MUST_CONSUME.get(t.name)


def _handle_lost(node, kind, msg):
    """G10 proc A0 / net A0 (G10_PROC_SPEC 3.3 / 7.1, G10_NET_SPEC 3.3 / 7.1): a must-consume duty is left at `node`;
    the code follows the type of the handle that loses its duty"""
    if kind == "Loop":
        _err(node, "async_unconsumed", msg,
             "consume it with ev_close(move l), or return / transfer ownership")
    if kind == "Process":
        _err(node, "proc_unconsumed", msg,
             "consume it with proc_wait(move p) or proc_kill(move p), or return / transfer ownership")
    _err(node, "net_unconsumed", msg,
         "consume it with net_close(move s) or net_listener_close(move l), or return / transfer ownership")


def _ctor_moves(e):
    """whole-binding moves nested inside an argument, through constructors and calls (D85)"""
    for k in expr_kids(e):
        if isinstance(k, Move) and not k.fields:
            yield k
        else:
            yield from _ctor_moves(k)


class _Ownership:
    def __init__(s, funcs, types, aggregates):
        s.funcs = funcs
        s.types = types
        s.aggregates = aggregates   # owned aggregates and recursive vector element types
        s.field_only = set()   # LB9 (R-6g8-4): record binders of a consuming match (field moves only)
        s.bor = set()
        s.arm_bor = []   # binders of the enclosing user-enum arms (Rust port arm_bor)
        s.track = None   # name -> "move" | "borrow" of the outermost call being checked (shared by nested calls)
        s.proc = {}      # G10 A0: must-consume bindings, name -> "Process" | "Socket" | "Listener"; never File etc.
        s.borrowed_handles = {}   # place bindings that refer to a must-consume handle
        s.decl = {}      # name -> the Let that declared it (location of a scope-end proc_unconsumed / net_unconsumed)
        s.nxt = None     # G-R09-4: the statement after the one being checked in its block
        s.consumed_field = None   # G-R09-4: the `move d.a` of a `let` that no next statement restores

    def enum_value(s, ty):
        decl = s.types.get(ty.name)
        return ty.owner is None and not ty.args and isinstance(decl, EnumDecl) and not decl.tparams

    def run(s, f):
        s.bor = {p.name for p in f.params
                 if not p.take and (is_owned_value(p.type) or is_nongeneric_struct(p.type, s.types))}
        s.proc = dict((p.name, _must_kind(p.type)) for p in f.params if p.take and _must_kind(p.type) is not None)
        st = s.block(f.body, dict((p.name, "live") for p in f.params if p.take))
        if st is not None:     # fallthrough: a live take must-consume parameter is a lost duty
            for n in st:
                if n in s.proc and st[n] != "dead":
                    _handle_lost(f, s.proc[n], "%s %s is not consumed before the end of the function" % (s.proc[n], n))

    def block(s, stmts, st):
        declared = []
        for i, x in enumerate(stmts):
            if st is None:
                s.dead_stmt(x)
            else:
                s.nxt = stmts[i + 1] if i + 1 < len(stmts) else None
                st = s.stmt(x, st, declared)
        for n in declared:
            s.borrowed_handles.pop(n, None)
            if st is not None:
                if n in s.proc and st.get(n) in ("live", "maybe"):
                    _handle_lost(s.decl[n], s.proc[n],
                                 "%s %s is not consumed before the end of its scope" % (s.proc[n], n))
                st.pop(n, None)
        return st

    def stmt(s, x, st, declared):
        if isinstance(x, Let):
            if x.place:
                # Evaluate the borrow while keeping the resource duty on its owner.
                s.expr(x.value, st)
                kind = _must_kind(x.type)
                if kind is not None:
                    s.borrowed_handles[x.name] = kind
                    declared.append(x.name)
                return st
            nxt = s.nxt
            v = x.value
            s.consumed_field = _consumed_field(v, nxt)
            s.consume(x.value, st, "let")
            s.consumed_field = None
            if (is_array(x.type) or x.type == STR or is_owned_value(x.type) or is_nongeneric_struct(x.type, s.types)
                    or s.enum_value(x.type)):     # G-DB16-1: an enum local is tracked so a binder move can consume it
                st[x.name] = "live"
                declared.append(x.name)
                kind = _must_kind(x.type)
                if kind is not None:
                    s.proc[x.name] = kind
                    s.decl[x.name] = x
                else:
                    s.proc.pop(x.name, None)
            return st
        if isinstance(x, Assign):
            s.consume(x.value, st, "assign")       # the right-hand side is consumed first
            kind = s.borrowed_handles.get(x.name)
            if kind is None and st.get(x.name) in ("live", "maybe"):
                kind = s.proc.get(x.name)
            if kind is not None:
                _handle_lost(x, kind, "%s %s is overwritten while it is not consumed" % (kind, x.name))
            if x.name in st:
                st[x.name] = "live"
            return st
        if isinstance(x, IndexAssign):
            s.expr(x.index, st)
            s.expr(x.value, st)
            s.use(x, st)
            return st
        if isinstance(x, FieldAssign):
            s.use(x, st)
            s.consume(x.value, st, "assign")
            return st
        if isinstance(x, If):
            s.expr(x.cond, st)
            return _join(s.block(x.then, dict(st)), s.block(x.els, dict(st)))
        if isinstance(x, (While, ForMap)):
            entry = dict(st)
            for clause in x.invariants:    # R03L: an invariant holds before the condition runs
                s.array_reads(clause, st)
            if isinstance(x, While):
                s.expr(x.cond, st)
                s.array_reads(x.decreases, st)
            else:
                s.expr(x.place, st)
            end = s.block(x.body, dict(st))
            if end is not None:
                for n, v in entry.items():
                    if v == "live" and end.get(n) != "live":
                        _err(x, "foreign_loop_move", "moved in the loop body and not restored",
                             "%s must be live again at the end of every path of the body: reassign it, e.g. "
                             "%s = f(move %s, ..), or do not consume it inside the loop" % (n, n, n))
            return _join(st, end)
        if isinstance(x, Return):
            if x.value is not None:
                s.consume(x.value, st, "return")
            for n in st:       # every owned handle leaving the function must be dead (the value was consumed first)
                if n in s.proc and st[n] != "dead":
                    _handle_lost(x, s.proc[n], "%s %s is not consumed before return" % (s.proc[n], n))
            return None
        if isinstance(x, ExprStmt):
            s.consumed_field = _consumed_field(x.expr, s.nxt)    # LG9 (G-LG8-1): `f(move d.a);`
            s.expr(x.expr, st)
            s.consumed_field = None
            return st
        if isinstance(x, Par):      # G7 A0: each branch is an ordinary call
            for c in x.calls:
                s.expr(c, st)
            return st
        if isinstance(x, Match):
            sc = x.scrutinee
            owned_result = is_owned_result(sc.ty)
            if owned_result and isinstance(sc, (Var, Move)):
                s.move(sc, st, True, s.track)
            else:
                s.expr(sc, st, owned_result)
            map_take = isinstance(sc, Call) and sc.name == "map_take" and sc.name not in s.funcs
            out = None
            first = True
            import codegen
            consuming = (isinstance(sc, Var) and sc.name in st and sc.name not in s.bor
                         and codegen.match_moves_binder(x))
            for a in x.arms:
                borrowed = s.arm_borrowed(a)
                s.bor.update(borrowed)
                views = s.arm_views(a)
                s.arm_bor.extend(views)
                inner = dict(st)
                if map_take:
                    for b in a.binders:
                        inner[b] = "live"
                        s.proc.pop(b, None)
                # G-DB16-1: the checker admits `move binder` of an array binder only for a plain owned enum scrutinee
                # that no arm names. A match where some arm moves a binder takes the scrutinee by value: it is
                # consumed on entry to every arm, and its array binders are tracked owners of the arm
                srcs = ()
                if owned_result and a.ctor == "Err":
                    srcs = a.binders
                    for b in srcs:
                        inner[b] = "live"
                elif consuming and a.enum_name is not None:
                    inner[sc.name] = "dead"
                    decl = s.types.get(a.enum_name)
                    variant = next(v for v in decl.variants if v.name == a.ctor)
                    # LG36 (G-DB59-1): str binders too, so a branch, loop or closed arm sees their moves
                    # LB9 (R-6g8-4): user struct and enum binders too, so a use after `let t = move b.f;` is a use
                    # after move on every path; they stay field-only (a whole `move b` is still refused, see move)
                    records = {b for b, fd in zip(a.binders, variant.fields)
                               if is_nongeneric_struct(fd.type, s.types) or s.enum_value(fd.type)}
                    srcs = [b for b, fd in zip(a.binders, variant.fields)
                            if is_array(fd.type) or fd.type == STR or b in records]
                    for b in srcs:
                        inner[b] = "live"
                    s.field_only |= records
                r = s.block(a.body, inner)
                if consuming and a.enum_name is not None:
                    s.field_only -= records
                if r is not None:
                    for b in srcs:
                        r.pop(b, None)
                if map_take and r is not None:
                    for b in a.binders:
                        r.pop(b)
                s.bor.difference_update(borrowed)
                del s.arm_bor[len(s.arm_bor) - len(views):]
                out = r if first else _join(out, r)
                first = False
            return out
        if isinstance(x, Assert):
            s.array_reads(x.cond, st)
        return st

    def array_reads(s, e, st):
        """Check array reads in proof clauses without applying executable call rules."""
        if e is None:
            return
        if isinstance(e, Var) and (is_array(e.ty) or e.ty == STR or (e.name in st and e.ty is not None and (
                is_owned_result(e.ty) or is_nongeneric_struct(e.ty, s.types) or s.enum_value(e.ty)))):
            s.use(e, st)     # LG8: a record or enum root consumed by a field or binder move
        for k in expr_kids(e):
            s.array_reads(k, st)

    def dead_stmt(s, x):
        """Check owning positions in unreachable code without tracking liveness."""
        if isinstance(x, (Let, Assign, FieldAssign, Return)):
            if x.value is not None:
                s.dead_expr(x.value, True)
        elif isinstance(x, IndexAssign):
            s.dead_expr(x.index)
            s.dead_expr(x.value)
        elif isinstance(x, ExprStmt):
            s.dead_expr(x.expr)
        elif isinstance(x, Par):
            for c in x.calls:
                s.dead_expr(c)
        elif isinstance(x, ForMap):
            s.dead_expr(x.place)
            for stmt in x.body:
                s.dead_stmt(stmt)
        elif isinstance(x, (If, While, Match)):
            if isinstance(x, Match):
                sc = x.scrutinee
                owned_result = is_owned_result(sc.ty)
                if owned_result and isinstance(sc, Var) and sc.name in s.bor:
                    s.move(sc, {}, True, None)
                s.dead_expr(sc, owned_result)
                for a in x.arms:
                    borrowed = s.arm_borrowed(a)
                    s.bor.update(borrowed)
                    views = s.arm_views(a)
                    s.arm_bor.extend(views)
                    for stmt in a.body:
                        s.dead_stmt(stmt)
                    s.bor.difference_update(borrowed)
                    del s.arm_bor[len(s.arm_bor) - len(views):]
                return
            s.dead_expr(x.cond)
            for body in ((x.then, x.els) if isinstance(x, If) else (x.body,)):
                for stmt in body:
                    s.dead_stmt(stmt)

    def dead_expr(s, e, owned=False):
        if isinstance(e, Move) and not owned:   # R03L: unreachable code keeps the consuming-position rule
            _err(e, "foreign_move", "move only where ownership is consumed",
                 "use move for a take argument, a let initializer, an assignment or return")
        if isinstance(e, Move):
            # Check the operand without carrying liveness across unreachable statements.
            live = {} if e.name in s.bor or e.name in s.arm_bor else {e.name: "live"}
            s.move(e, live, True, None)
        if owned:
            s.check_field_alias(e)
            if isinstance(e, Var) and is_owned_value(e.ty) and (e.name in s.bor or e.name in s.arm_bor):
                _err(e, "foreign_borrow", "borrowed alias", "%s is a read parameter and cannot be aliased" % e.name)
        if isinstance(e, Call):
            s.check_call_move_alias(e)
            for arg, take in zip(e.args, s.takes(e)):
                s.dead_expr(arg, take)
        else:
            for k in expr_kids(e):
                if isinstance(e, Ctor) and e.name == "Err" and e.ty is not None and is_owned_result(e.ty):
                    if isinstance(k, Var) and k.name in s.bor:
                        s.move(k, {}, True, None)
                    s.dead_expr(k, True)
                    continue
                s.dead_expr(k, isinstance(e, (StructLit, EnumCtor)))

    @staticmethod
    def check_field_alias(e):
        if isinstance(e, (FieldAccess, VariantField)) and is_owned_value(e.ty):
            _err(e, "foreign_borrow", "borrowed alias", "the field is a read view and cannot be aliased")

    def arm_views(s, a):
        """every binder of a user enum arm views the scrutinee (Rust port arm_bor): unreachable code cannot move it"""
        return list(a.binders) if a.enum_name is not None else []

    def arm_borrowed(s, a):
        """binders of a user enum arm that view an owned collection field"""
        decl = s.types.get(a.enum_name)
        if decl is None:
            return set()
        variant = next(v for v in decl.variants if v.name == a.ctor)
        return {b for b, fd in zip(a.binders, variant.fields) if is_owned_field_type(fd.type)}

    # ---- uses ----
    def use(s, e, st, name=None):
        name = e.name if name is None else name
        v = st.get(name)
        if v == "dead":
            _err(e, "foreign_move", "use after move",
                 "%s was moved earlier; reassign it before using it again, or avoid the move" % name)
        if v == "maybe":
            _err(e, "foreign_move", "binding may have been moved",
                 "%s is moved on some paths; make every path leave it live, or do not use it afterwards" % name)

    def aggregate(s, ty):
        return ty is not None and ty.owner is None and not ty.args and ty.name in s.aggregates

    def move(s, e, st, consuming, track):
        if isinstance(e, Move) and e.fields:
            s.use(e, st)
            if e is s.consumed_field:    # G-R09-4: no restore follows, so the record is wholly moved
                st[e.name] = "dead"
            return       # Otherwise the type checker enforces restoration. The root stays live.
        n = e.name
        if not (is_array(e.ty) or e.ty == STR or is_owned_value(e.ty) or (s.aggregate(e.ty) and e.name in st and n not in s.field_only)):
            _err(e, "foreign_move", "move of a non-collection value",
                 "move applies to Vec, HashMap and String bindings only; use the value directly")
        if track is not None and n in track:
            if track[n] == "move":
                _err(e, "foreign_move", "consumed twice", "consume %s at most once per call" % n)
            _err(e, "foreign_borrow", "borrowed and moved in one call",
                 "do not pass %s both read-only and with move in one call" % n)
        s.use(e, st)
        if not consuming:
            _err(e, "foreign_move", "move only where ownership is consumed",
                 "use move for a take argument, a let initializer, an assignment or return")
        if n in s.bor:
            # G10 specifies foreign_borrow; preserve the established G6 diagnostic for other types.
            _err(e, "foreign_borrow" if e.ty in (FILE, PROCESS, SOCKET, LISTENER, LOOP) else "foreign_move", "moving a borrowed parameter",
                 "%s is a read parameter; declare it `take %s: ...` to consume it" % (n, n))
        st[n] = "dead"
        if track is not None:
            track[n] = "move"

    def consume(s, e, st, pos):
        """let initializer / assignment / return value"""
        if isinstance(e, Move):
            s.move(e, st, True, s.track)
        elif isinstance(e, Var) and is_owned_value(e.ty):
            s.use(e, st)
            if e.name in s.bor:
                if pos == "return":
                    _err(e, "foreign_borrow", "borrowed return", "%s is a read parameter; declare it `take`" % e.name)
                _err(e, "foreign_borrow", "borrowed alias", "%s is a read parameter and cannot be aliased" % e.name)
            if pos == "return":
                _err(e, "foreign_move", "returning an owned binding without move", "write return move %s;" % e.name)
            _err(e, "foreign_move", "alias of an owned binding without move", "write move %s" % e.name)
        else:
            s.expr(e, st, True)

    # ---- expressions ----
    def expr(s, e, st, owned=False):
        """owned: the value is taken at once by a binding, a return or a take argument (not a dropped temporary)"""
        if owned:
            s.check_field_alias(e)
        if isinstance(e, Var):
            s.use(e, st)
        elif isinstance(e, Move):
            s.move(e, st, False, None)
        elif isinstance(e, Index) and isinstance(e.array, Var) and is_array(e.array.ty):
            s.expr(e.index, st)
            s.use(e.array, st)
        elif isinstance(e, PlaceIf):
            s.expr(e.cond, st)
            then, els = dict(st), dict(st)
            s.expr(e.then, then)
            s.expr(e.els, els)
            st.clear()
            st.update(_join(then, els))
        elif isinstance(e, Call):
            kind = _must_kind(e.ty)
            if not owned and kind is not None:    # a handle temporary that is borrowed or dropped keeps its duty
                s.call(e, st)
                _handle_lost(e, kind, "temporary %s is not consumed" % kind)
            s.call(e, st)
        elif isinstance(e, Ctor) and e.name == "Err" and is_owned_result(e.ty):
            if isinstance(e.arg, (Var, Move)):
                s.move(e.arg, st, True, s.track)
            else:
                s.expr(e.arg, st, True)
        elif isinstance(e, Binary) and e.op in ("&&", "||"):
            s.expr(e.left, st)
            before = dict(st)
            s.expr(e.right, st)
            joined = _join(before, st)      # the right operand runs conditionally
            st.clear()
            st.update(joined)
        else:
            for k in expr_kids(e):
                if isinstance(e, (StructLit, EnumCtor)) and (
                        is_collection(k.ty) or isinstance(k, Move) and (
                            is_array(k.ty) or k.ty == STR or is_nongeneric_struct(k.ty, s.types))):   # D85: owned aggregate field init
                    s.consume(k, st, "let")
                else:
                    s.expr(k, st)

    def takes(s, e):
        f = s.funcs.get(e.name) or CATALOG.get(e.name)
        takes = [bool(p.take) for p in f.params] if f is not None else [False] * len(e.args)
        if e.name in ("vec_push", "map_insert") and e.args:
            if is_nongeneric_struct(e.args[-1].ty, s.types):
                takes[-1] = True
            if e.name not in s.funcs:
                payload = e.args[-1].ty
                takes[-1] |= payload == STRING or (e.name == "vec_push" and s.aggregate(payload))
                if e.name == "map_insert":
                    takes[1] = e.args[1].ty == STRING
        return takes

    def call(s, e, st):
        outer = s.track
        track = outer if outer is not None else {}   # nested calls share the outermost call's track (R1)
        s.track = track
        try:
            s.call_args(e, st, s.takes(e), track)
        finally:
            s.track = outer

    @staticmethod
    def check_call_move_alias(e):
        from check import _mentions
        for i, arg in enumerate(e.args):
            if isinstance(arg, Move) and not arg.fields:
                for j, other in enumerate(e.args):
                    if (i != j and not (isinstance(other, Move) and not other.fields)
                            and _mentions(other, arg.name)):
                        _err(e.args[max(i, j)], "foreign_borrow", "borrowed and moved in one call",
                             "do not pass %s both read-only and with move in one call" % arg.name)
            for m in _ctor_moves(arg):   # D85: a move inside a constructor argument conflicts with any other mention
                for j, other in enumerate(e.args):
                    if i != j and _mentions(other, m.name):
                        _err(e.args[max(i, j)], "foreign_borrow", "borrowed and moved in one call",
                             "do not pass %s both read-only and with move in one call" % m.name)

    def call_args(s, e, st, takes, track):
        s.check_call_move_alias(e)
        for arg, take in zip(e.args, takes):
            if getattr(e, "take_result_error", False):
                s.move(arg, st, True, track)
                continue
            root = arg
            if e.name == "string_eq":   # R-6e-11: a field or a vec_get element of a place borrows the place's root
                inner = arg.args[0] if isinstance(arg, Call) and arg.name == "vec_get" and arg.args else arg
                if isinstance(inner, FieldAccess) or (inner is not arg and isinstance(inner, Var)):
                    root = field_place(inner)[0]
            if take:
                if isinstance(arg, Move):
                    s.move(arg, st, True, track)
                elif isinstance(arg, Var) and is_owned_value(arg.ty):
                    s.use(arg, st)
                    if arg.name in s.bor:
                        _err(arg, "foreign_move", "moving a borrowed parameter",
                             "%s is a read parameter; declare it `take %s: ...` to consume it" % (arg.name, arg.name))
                    _err(arg, "foreign_move", "missing move in a consuming argument",
                         "write move %s" % arg.name)
                else:
                    s.expr(arg, st, True)
            elif isinstance(arg, Move):
                s.move(arg, st, False, track)
            elif isinstance(root, Var) and (root is not arg or is_owned_value(root.ty) or root.name in st):
                if track.get(root.name) == "move":
                    _err(arg, "foreign_borrow", "borrowed and moved in one call",
                         "do not pass %s both read-only and with move in one call" % root.name)
                s.expr(arg, st)
                track.setdefault(root.name, "borrow")
            else:
                s.expr(arg, st)


def check_ownership(f, funcs, types, aggregates):
    """ownership/liveness of one typed, non-foreign function; funcs: name -> Function of the program"""
    _Ownership(funcs, types, aggregates).run(f)


# ---------------------------------------------------------------------------
# A0 guard (section 10.5)
# ---------------------------------------------------------------------------
def _g6_candidates(node, out):
    if isinstance(node, list):
        for x in node:
            _g6_candidates(x, out)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    if isinstance(node, Let) and contains_collection(node.type):
        out.append((node.line, node.col, "a collection local"))
    elif isinstance(node, InoutArg) and node.ty is not None and is_collection(node.ty):
        out.append((node.line, node.col, "a mutable collection place"))
    elif isinstance(node, Move) and not is_array(node.ty):
        out.append((node.line, node.col, "move"))
    elif isinstance(node, Call) and node.ty is not None and (contains_collection(node.ty) or node.name == "string_eq"):
        out.append((node.line, node.col, "a collection value"))
    elif isinstance(node, Ctor) and node.ty is not None and contains_collection(node.ty):
        out.append((node.line, node.col, "a collection value"))     # LG12: Ok::<T, String>(..)
    for fname in node.__dataclass_fields__:
        _g6_candidates(getattr(node, fname), out)


def first_g6_use(prog):
    """first G6 construct of a checked program -> (line, col, what) or None. Source order: functions in order,
    the signature (foreign declaration, take or collection parameter/result) before the body."""
    for f in prog.functions:
        if f.foreign is not None:
            return (f.line, f.col, "trusted rust fn %s" % f.name)
        for p in f.params:
            if (p.take and not is_array(p.type)) or contains_collection(p.type):
                return (f.line, f.col, "parameter %s" % p.name)
        if contains_collection(f.ret):
            return (f.line, f.col, "result type of %s" % f.name)
        cands = []
        _g6_candidates(f.body, cands)
        if cands:
            return min(cands, key=lambda c: (c[0], c[1]))
    for d in prog.types:
        for fd in TypeTable._all_fields(d):
            if (is_owned_field_type(fd.type) or is_nested_collection_field(fd.type) or contains_collection(fd.type)
                    or fd.type.name == "hashmap" and fd.type.args[0] == STRING):
                return (fd.line, fd.col, "an owned collection field")
    return None


def g6_unimplemented_error(prog, stage="codegen"):
    """PruvError (unsupported_type) for the first G6 construct, or None. Run right after type checking."""
    hit = first_g6_use(prog)
    if hit is None:
        return None
    line, col, _what = hit
    return PruvError(stage, line, col, "unsupported_type",
                     "G6 foreign calls and collections are not implemented by the llvm codegen layer yet",
                     "avoid trusted rust fn, Vec, HashMap, String, take and move for now")


def _is_proc_type(t):
    return t.owner is None and (t.name in ("process", "process_args") or any(_is_proc_type(a) for a in t.args))


def _proc_candidates(node, out):
    if isinstance(node, (list, tuple)):
        for x in node:
            _proc_candidates(x, out)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    if isinstance(node, Let) and _is_proc_type(node.type):
        out.append((node.line, node.col))
    elif isinstance(node, Call) and node.name in PROC_OPS:
        out.append((node.line, node.col))
    for fname in node.__dataclass_fields__:
        _proc_candidates(getattr(node, fname), out)


def first_proc_use(prog):
    """G10 proc A0: the first use of Process, ProcessArgs or a proc_* operation of a checked program ->
    (node, line, col) or None. Functions in order; the signature (a Process / ProcessArgs parameter or result) comes
    before the body, whose first use is the smallest source position. A function that only declares `! proc` is
    not a use."""
    for f in prog.functions:
        if any(_is_proc_type(p.type) for p in f.params) or _is_proc_type(f.ret):
            return (f, f.line, f.col)
        cands = []
        _proc_candidates(f.body, cands)
        if cands:
            line, col = min(cands)
            return (f, line, col)
    return None


def _is_net_type(t):
    return t.owner is None and (t.name in ("socket", "listener") or any(_is_net_type(a) for a in t.args))


def _net_candidates(node, out):
    if isinstance(node, (list, tuple)):
        for x in node:
            _net_candidates(x, out)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    if isinstance(node, Let) and _is_net_type(node.type):
        out.append((node.line, node.col))
    elif isinstance(node, Call) and node.name in NET_OPS:
        out.append((node.line, node.col))
    for fname in node.__dataclass_fields__:
        _net_candidates(getattr(node, fname), out)


def first_net_use(prog):
    """G10 net A0: the first use of Socket, Listener or a net_* operation of a checked program -> (node, line, col) or
    None. Functions in order; the signature (a Socket / Listener parameter or result) comes before the body, whose
    first use is the smallest source position. A function that only declares `! net` is not a use."""
    for f in prog.functions:
        if any(_is_net_type(p.type) for p in f.params) or _is_net_type(f.ret):
            return (f, f.line, f.col)
        cands = []
        _net_candidates(f.body, cands)
        if cands:
            line, col = min(cands)
            return (f, line, col)
    return None


def _par_nodes(node, out):
    if isinstance(node, list):
        for x in node:
            _par_nodes(x, out)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    if isinstance(node, Par):
        out.append(node)
    for fname in node.__dataclass_fields__:
        _par_nodes(getattr(node, fname), out)


def first_par(prog):
    """G7 A0: the first par statement of a checked program (functions in order, smallest position inside a function)
    or None."""
    for f in prog.functions:
        found = []
        _par_nodes(f.body, found)
        if found:
            return min(found, key=lambda p: (p.line, p.col))
    return None


def g7_target_error(prog):
    """Native targets cannot link the Rust scoped-thread bridge."""
    p = first_par(prog)
    if p is None:
        return None
    return PruvError("codegen", p.line, p.col, "parallel_target",
                     "native obj/exe emission is unavailable for par",
                     "emit LLVM IR with --rust-bindings and link using Rust",
                     file=getattr(p, "file", None))


def uses_files(prog):
    """Whether the program needs the G10 File runtime."""
    def uses_file(node):
        if isinstance(node, (list, tuple)):
            return any(uses_file(x) for x in node)
        if isinstance(node, Type):
            return node == FILE or any(uses_file(a) for a in node.args)
        if isinstance(node, Call) and node.name in CATALOG and node.name.startswith("file_"):
            return True
        if not hasattr(node, "__dataclass_fields__"):
            return False
        return any(uses_file(getattr(node, name)) for name in node.__dataclass_fields__)

    return any(uses_file(f) for f in prog.functions)


def _is_async_type(t):
    return t.owner is None and (t.name in ("loop",) or any(_is_async_type(a) for a in t.args))


def _async_candidates(node, out, names):
    if isinstance(node, (list, tuple)):
        for x in node:
            _async_candidates(x, out, names)
        return
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return
    if isinstance(node, Let) and _is_async_type(node.type):
        out.append((node.line, node.col))
    elif isinstance(node, Call) and node.name in ASYNC_OPS and node.name not in names:
        out.append((node.line, node.col))
    for fname in node.__dataclass_fields__:
        _async_candidates(getattr(node, fname), out, names)


def first_async_use(prog):
    """G10 async A0: the first use of Loop or an ev_* operation of a checked program -> (node, line, col) or
    None. Functions in order; the signature (a Loop parameter or result) comes before the body, whose
    first use is the smallest source position. A function that only declares `! async` is not a use."""
    for f in prog.functions:
        if any(_is_async_type(p.type) for p in f.params) or _is_async_type(f.ret):
            return (f, f.line, f.col)
        cands = []
        _async_candidates(f.body, cands, {fn.name for fn in prog.functions})
        if cands:
            line, col = min(cands)
            return (f, line, col)
    return None


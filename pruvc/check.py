"""Pruv type / effect / structure checker. Fills Expr.ty."""
import copy
import re
from vast import *
from typeinfo import (TypeTable, BUILTIN_TYPE_NAMES, type_name, is_user_type, is_array, array_elem,  # G2 A5: array_elem
                      TypeArityError, check_arity, bind_args, substitute, is_value, is_element,
                      dependencies, decl_owner, fn_owner,  # G3 A1
                      COLLECTION_ARITY, COLLECTION_SPELLING, is_collection, contains_collection,  # G6 A0
                      program_uses_inout, is_owned_field_type, is_scalar_vec, is_nongeneric_struct,
                      field_place, is_nested_collection_field, is_string_record_map, is_vec_slice_arg, is_owned_value,
                      borrowing_copy)
from refine import expand as refine_expand
from refine import _kids as expr_kids, _stmt_names
from foreign import (CATALOG, CATALOG_NAMES, CATALOG_CONTRACT_ONLY, FILE_OBSERVERS, PROC_OBSERVERS, NET_OBSERVERS, ASYNC_OBSERVERS, ASYNC_OPS,
                     check_foreign_decl, check_ownership, STRING, MUTABLE_COLLECTION_MATCH, BORROWED_MAP_KEY)  # G6 A0 / G10 A0

# name -> (argument kind, result: concrete type or the selected payload type)
CONTRACT_ONLY = {"is_some": ("option", BOOL), "some_val": ("option", "payload"), "is_ok": ("result", BOOL),
                 "ok_val": ("result", "payload"), "err_val": ("result", "error"),
                 "size": ("size", I64)}   # G9 A0: structural size of a non-generic nominal value
# P2: pure (i64, i64) -> i64 bit builtins: name -> (Rust operator, LLVM instruction); later batches add entries here
BIT_BUILTINS = {"band": ("&", "and"), "bor": ("|", "or"), "bxor": ("^", "xor"),
                "wrapping_shl": ("<<", "shl"), "ushr": (">>", "lshr"),
                "wrapping_add": ("wrapping_add", "add"), "wrapping_mul": ("wrapping_mul", "mul")}
def x_has_quant(e):
    """R03L: does the expression have a quantifier anywhere"""
    return isinstance(e, Quant) or any(x_has_quant(k) for k in expr_kids(e))


def x_has_bit(e):
    """P07: does the expression call a bit builtin anywhere"""
    if isinstance(e, Call) and e.name in BIT_BUILTINS:
        return True
    return any(x_has_bit(k) for k in expr_kids(e))


# P2 batch 4: byte-word access on [u8]: load_be(a, i, n) and the statement-only store_be(inout a, i, w, n)
WORD_BUILTINS = ("load_be", "store_be", "load_le", "store_le")
ARRAY_GROW_BUILTINS = ("arr_reserve", "arr_push", "arr_append")
ARR_PUSH_MAX = 16   # LB6 SX (R-6g5-5): the most values one arr_push appends

BUILTIN_FNS = ARRAY_GROW_BUILTINS + ("len", "alloc", "copy", "print", "is_char_boundary", "substr", "utf8_valid", "utf8_valid_span", "str_from_utf8", "truncate", "eq_range", "find_byte", "resize", "copy_range", "copy_within", "subview") + tuple(BIT_BUILTINS) + WORD_BUILTINS   # L3-10: eq_range; L-copy-range
RESERVED_TYPE_NAMES = ("Option", "Result", "Some", "None", "Ok", "Err")
TEXT_VIEW_OPS = ("string_push_str", "string_from", "proc_args_push")   # LB7 (R-6g6-3), LB9 (R-6g8-1): text argument may be a str subview
TYPE_NAME_RE = re.compile(r"[A-Z][A-Za-z0-9_]*\Z")
FIELD_NAME_RE = re.compile(r"[a-z][A-Za-z0-9_]*\Z")


def nested_child_type(s, ty):
    """D90: a direct field type that is an R-6e-6 record (owned aggregate with nested collection fields)"""
    return is_nongeneric_struct(ty, s.decls) and ty.name in s.nested_records and ty.name in s.aggregates


def terr(node, code, msg, fix="", stage="type"):
    raise PruvError(stage, node.line, node.col, code, msg, fix, file=getattr(node, "file", None))   # G4: file only in explicit mode


def is_try(e):
    return isinstance(e, Unary) and e.op == "?"


def check_try_position(e, at):
    if is_try(e):
        try_position_error(at)
    for child in expr_kids(e):
        check_try_position(child, at)


def try_position_error(at):
    terr(at, "try_position",
         "`?` is only allowed in the value of a let, a local assignment or a return, an expression statement or an"
         " if condition, and not in a quantifier, in the right operand of ==>, in the right operand of && / || when"
         " the statement passes inout or the operator holds a quantifier or a subview or moves a name used outside it"
         " (or uses one moved outside it, or a binding of a type that is not a value used again), or after an operand evaluated before it that cannot be bound to a temporary (one with inout,"
         " a quantifier, a move of a name used again or a type that is not a value)",
         "bind the value with `let x: T = e?;` first (for a Result<(), E>, write `e?;` as a statement)")


def _has_kind(e, kind):
    return isinstance(e, kind) or any(_has_kind(k, kind) for k in expr_kids(e))


_TRY_PURE_OPS = ("==", "!=", "<", "<=", ">", ">=", "&&", "||", "==>", "!")


def _try_pure(e):
    """LG13: a node that may run after a later `?` without changing what happens: a read of a binding, a literal
    other than a string, a field read, `!` and a comparison or logic operator (none can fail, allocate or move)"""
    return isinstance(e, (Var, IntLit, BoolLit, FieldAccess)) or isinstance(e, (Unary, Binary)) and e.op in _TRY_PURE_OPS


TRY_TEMP = "__pruv_try_t"


def find_try(e):
    """LG13/LG14: (the first `?` of e in evaluation order (operands left to right, then the node itself), the operands
    to bind to fresh temporaries first), or None when e has no `?` that can be lowered. An operand evaluated before the
    `?` that is not _try_pure (a call, a constructor, a move, an index, arithmetic) is bound to a temporary when the
    whole operand holds no `inout` argument and no quantifier and moves no binding mentioned elsewhere in e (LG14,
    G-LG13-1); its type is checked by Checker.try_temps. A `?` only in the right operand of && / || gives that
    operator node instead (LG15, G-LG13-2; lowered by Checker.try_short) unless the statement passes `inout` or the
    node holds a quantifier or a subview or moves a name mentioned outside it (or mentions one moved outside it). A
    `?` inside a quantifier, after an operand that cannot be bound, or whose operand passes `inout` while the
    statement reads a binding outside that operand stays for check_try_position."""
    root = e
    seen = {"impure": False}
    path = []          # (node, index of the kid on the way to the current node)

    def visit(e):
        if isinstance(e, Quant):
            if any(is_try(x) for x in _walk(e)):
                raise _NoTry
            seen["impure"] = True      # G-LG13-7: a quantifier is not code that may move after a `?`
            return None
        if isinstance(e, Binary) and e.op in ("&&", "||", "==>"):
            before = seen["impure"]
            path.append((e, 0))
            found = visit(e.left)
            path.pop()
            if found is not None:
                return found
            if any(is_try(x) for x in _walk(e.right)):
                if e.op == "==>":       # ==> is contract-only; lowering it would hide type/contract_only
                    raise _NoTry
                return short(e, before)
            note(e.right)
            note_node(e)
            return None
        if is_try(e):
            before = seen["impure"]
            path.append((e, 0))
            found = visit(e.operand)
            path.pop()
            if found is not None:
                return found
            if _has_kind(e.operand, InoutArg):
                reads = lambda x: sum(isinstance(y, (Var, Move)) for y in _walk(x))
                if reads(root) > reads(e.operand):
                    raise _NoTry
            if before:
                return e, temps(e.operand)
            # LG21 (G-LG18-1): with nothing evaluated before the `?` nothing is bound, but a `?` operand still may not
            # mention the root of an inout argument outside it (inout_context unlowered); an inout argument inside
            # the operand itself (`set(inout x)?`) stays as before
            for t in _walk(root):
                if is_try(t):
                    inner = {id(x) for x in _walk(t.operand)}
                    outs = set()
                    for a in _walk(root):
                        if isinstance(a, InoutArg) and id(a) not in inner:
                            place = field_place(a.expr)[0]
                            if not isinstance(place, Var):
                                raise _NoTry
                            outs.add(place.name)
                    if any(isinstance(x, (Var, Move)) and x.name in outs for x in _walk(t.operand)):
                        raise _NoTry
            return e, []
        for i, k in enumerate(expr_kids(e)):
            path.append((e, i))
            found = visit(k)
            path.pop()
            if found is not None:
                return found
        note_node(e)
        return None

    def temps(operand=None):
        out = []
        inout = set()
        for a in _walk(root):
            if isinstance(a, InoutArg):
                place = field_place(a.expr)[0]
                if not isinstance(place, Var):
                    raise _NoTry
                inout.add(place.name)
        # LG18 (G-LG14-2 a): with inout in the statement, neither the temporaries nor the `?` operand may hold an
        # inout argument or mention an inout root (that would be inout_context unlowered); Checker.try_temps rejects
        # a temporary with an effect call (effect_before)
        # (every `?` of the statement: a later `?` finds its earlier operands already bound and would skip this)
        if inout and any(is_try(t) and any(isinstance(x, (Var, Move)) and x.name in inout for x in _walk(t.operand))
                         for t in _walk(root)):
            raise _NoTry
        for node, i in path:
            for k in expr_kids(node)[:i]:
                if all(_try_pure(x) for x in _walk(k)) or isinstance(k, Move) and k.name.startswith(TRY_TEMP):
                    continue        # LG18: `move __pruv_try_tN` is an already bound temporary passed to take
                if any(isinstance(x, Quant) or isinstance(x, Call) and x.name == "subview" for x in _walk(k)):
                    raise _NoTry
                if inout and (_has_kind(k, InoutArg)
                              or any(isinstance(x, (Var, Move)) and x.name in inout for x in _walk(k))):
                    raise _NoTry
                for m in _walk(k):
                    if isinstance(m, Move) and _mentions_count(root, m.name) > _mentions_count(k, m.name):
                        raise _NoTry
                    if isinstance(m, Move) and _moves_count(root, m.name) > _moves_count(k, m.name):
                        raise _NoTry
                    if isinstance(m, Var) and _moves_count(root, m.name) > _moves_count(k, m.name):
                        # LG18 (G-LG14-2 b): a borrow bound early keeps its order before the later move; only a
                        # move in the same call (one borrow track, foreign_borrow unlowered) must stay rejected
                        for x in _walk(root):
                            if isinstance(x, Move) and x.name == m.name and _shared_call(root, k, x):
                                raise _NoTry
                out.append((node, k))
        return out

    def short(e, before):
        # LG15 (G-LG13-2): a `?` only in the right operand of && / ||: the whole operator is lowered to a nested
        # if (Checker.try_short), so it runs before the rest of the statement; it must not pass inout, hold a
        # quantifier or a subview, or move a name the rest of the statement mentions (or mention one it moves)
        if _has_kind(root, InoutArg):
            raise _NoTry
        if any(isinstance(x, Quant) or isinstance(x, Call) and x.name == "subview" for x in _walk(e)):
            raise _NoTry
        for m in _walk(e):
            if isinstance(m, Move) and _mentions_count(root, m.name) > _mentions_count(e, m.name):
                raise _NoTry
            if isinstance(m, (Var, Move)) and _moves_count(root, m.name) > _moves_count(e, m.name):
                raise _NoTry
        return e, (temps() if before else [])

    def note_node(e):
        if not _try_pure(e):
            seen["impure"] = True

    def note(e):
        for x in _walk(e):
            note_node(x)

    try:
        return visit(e)
    except _NoTry:
        return None


def _shared_call(root, a, b):
    """LG18: some call of root holds both nodes a and b (by identity): their arguments share one borrow track"""
    return any(isinstance(x, Call) and any(y is a for y in _walk(x)) and any(y is b for y in _walk(x))
               for x in _walk(root))


def _moves_count(e, name):
    return sum(isinstance(x, Move) and x.name == name for x in _walk(e))


def _mentions_count(e, name):
    return sum(isinstance(x, (Var, Move)) and x.name == name for x in _walk(e))


class _NoTry(Exception):
    pass


def _walk(e):
    yield e
    for k in expr_kids(e):
        yield from _walk(k)


def invariant_base(name, ty, path, line, col, file, guards=None):
    """LG23: the value reached from the variable `name` of type ty through path = ((label, type, kind, extra), ...);
    the guard of each Option / Result / enum step (is_some, is_ok, is_variant) is appended to guards"""
    def mk(n):
        n.file = file
        return n
    base, cur = mk(Var(line, col, name, ty)), ty
    for label, sty, kind, extra in path:
        if kind == "field":
            nxt = FieldAccess(line, col, base, extra, sty)
        elif kind == "elem":
            # LG27: the element of a top-level Vec, read at the quantified index $inv_e
            nxt = Index(line, col, base, mk(Var(line, col, "$inv_e", I64)), sty)
        elif kind in ("some", "ok"):
            if guards is not None:
                guards.append(mk(Call(line, col, "is_" + kind, [base], BOOL)))
            nxt = Call(line, col, kind + "_val", [base], sty)
        elif kind == "err":
            if guards is not None:
                guards.append(mk(Unary(line, col, "!", mk(Call(line, col, "is_ok", [base], BOOL)), BOOL)))
            nxt = Call(line, col, "err_val", [base], sty)
        else:
            if guards is not None:
                g = IsVariant(line, col, base, extra[0], extra[1], BOOL)
                g.enum_ty = cur
                guards.append(mk(g))
            nxt = VariantField(line, col, base, extra[0], extra[1], extra[2], sty)
            nxt.enum_ty = cur
        base, cur = mk(nxt), sty
    return base


def invariant_clause(expr, ty, name, metadata, path=()):
    """Bind free field names in a copy of an already typed struct clause. path = ((label, type, kind, extra), ...) is
    the chain of steps (struct fields, Option / Result payloads, enum variant fields) from the named value to the
    struct that owns the clause (G-DB31-3, LG23); a guarded step wraps the clause in `!guard || ...`."""
    expr = copy.deepcopy(expr)

    def bind(e, bound):
        if isinstance(e, Var):
            if e.name in bound:
                e.name = bound[e.name]
            else:
                base = invariant_base(name, ty, path, e.line, e.col, e.file)
                e.__class__ = FieldAccess
                e.base, e.field = base, e.name
                del e.name
        elif isinstance(e, Quant):
            bind(e.lo, bound)
            bind(e.hi, bound)
            original = e.var
            if e.var == name:
                e.var = "$inv_" + e.var
                # LG27: keep the renamed binder distinct from the element index (symbolic.rs inv_rewrite)
                if path and path[0][0] == "[]" and e.var == "$inv_e":
                    e.var += "_bound"
            inner = {**bound, original: e.var}
            bind(e.body, inner)
            for trigger in e.triggers:
                bind(trigger, inner)
        else:
            for child in expr_kids(e):
                bind(child, bound)

    bind(expr, {})
    guards = []
    invariant_base(name, ty, path, expr.line, expr.col, getattr(expr, "file", None), guards)
    for g in reversed(guards):
        neg = Unary(expr.line, expr.col, "!", g, BOOL)
        neg.file = g.file
        wrapped = Binary(expr.line, expr.col, "||", neg, expr, BOOL)
        wrapped.file = g.file
        expr = wrapped
    expr.struct_invariant = metadata
    return expr


def replace_expr(holder, target, new):
    """LG13: replace the node `target` (by identity) inside the expression fields of `holder` with `new`"""
    for key, value in vars(holder).items():
        if value is target:
            setattr(holder, key, new)
            return True
        if isinstance(value, list):
            for i, item in enumerate(value):
                if item is target:
                    value[i] = new
                    return True
                if isinstance(item, (Expr, FieldInit)) and replace_expr(item, target, new):
                    return True
        elif isinstance(value, Expr) and replace_expr(value, target, new):
            return True
    return False


def try_site(st, scope):
    """LG13/LG14: (the `?` to lower first in statement st, the operands to bind to temporaries first) or None. Lowered positions: the value of a let, of an assignment
    to a local variable and of a return, the expression of an expression statement and the condition of an if."""
    if isinstance(st, Assign):
        if st.deref or st.name in scope and scope[st.name][2] != "local":
            return None
        e = st.value
    elif isinstance(st, Let):
        e = st.value
    elif isinstance(st, ExprStmt):
        e = st.expr
    elif isinstance(st, Return):
        e = st.value
    elif isinstance(st, If):
        e = st.cond
    else:
        return None
    return None if e is None else find_try(e)


class _At:
    """a (line, col, file) position for diagnostics that point at a token without an AST node (G5: `inout` marker)"""
    def __init__(s, line, col, file=None):
        s.line, s.col, s.file = line, col, file


def _mode_at(f, p):
    """position of the `inout` token of parameter p of function f"""
    return _At(p.mode_pos[0], p.mode_pos[1], getattr(p, "file", None) or getattr(f, "file", None))


def _mentions(e, name):
    """G5: does the full syntax of e (every operand, reachable or not) mention the binding `name`?"""
    if isinstance(e, (Var, Move)) and e.name == name:
        return True
    return any(_mentions(k, name) for k in expr_kids(e))


def push_only(body, name):
    """Whether the loop body writes this binding only by pushing independent elements."""
    pushed = False

    def is_root(e):
        while isinstance(e, (FieldAccess, Index, InoutArg, VariantField)):
            if isinstance(e, FieldAccess):
                e = e.base
            elif isinstance(e, Index):
                e = e.array
            elif isinstance(e, VariantField):
                e = e.value
            else:
                e = e.expr
        return isinstance(e, (Var, Move)) and e.name == name

    def reads_only(e):
        for part in _walk(e):
            if isinstance(part, Move) and part.name == name:
                return False
            if isinstance(part, InoutArg) and is_root(part.expr):
                return False
            if isinstance(part, Call) and part.name == "vec_pop" and any(is_root(a) for a in part.args):
                return False
        return True

    def walk(stmts):
        nonlocal pushed
        for st in stmts:
            if isinstance(st, (Let, Assign, FieldAssign, IndexAssign)) and st.name == name:
                call = st.value
                if (not isinstance(st, Assign) or st.deref or not isinstance(call, Call)
                        or call.name != "vec_push" or len(call.args) != 2):
                    return False
                moved, item = call.args
                if (not isinstance(moved, Move) or moved.name != name or moved.fields
                        or _mentions(item, name)):
                    return False
                pushed = True
                continue
            if isinstance(st, (Let, Assign, FieldAssign, Return)):
                exprs = (st.value,)
            elif isinstance(st, IndexAssign):
                exprs = (st.index, st.value)
            elif isinstance(st, Assert):
                exprs = (st.cond,)
            elif isinstance(st, ExprStmt):
                exprs = (st.expr,)
            elif isinstance(st, Par):
                exprs = st.calls
            elif isinstance(st, If):
                exprs = (st.cond,)
                if not walk(st.then) or not walk(st.els):
                    return False
            elif isinstance(st, While):
                exprs = (st.cond, *st.invariants, st.decreases)
                if not walk(st.body):
                    return False
            elif isinstance(st, ForMap):
                exprs = (st.place, *st.invariants)
                if name in (st.key, st.val) or not walk(st.body):
                    return False
            elif isinstance(st, Match):
                exprs = (st.scrutinee,)
                if any(name in a.binders or not walk(a.body) for a in st.arms):
                    return False
            else:
                return False
            if any(e is not None and not reads_only(e) for e in exprs):
                return False
        return True

    return walk(body) and pushed


def _field_moves(e):
    if isinstance(e, Move) and e.fields:
        yield e
    for child in expr_kids(e):
        yield from _field_moves(child)


def _paths_overlap(p, q):
    """one field path is a prefix of the other (the whole root is the empty path)"""
    n = min(len(p), len(q))
    return tuple(p[:n]) == tuple(q[:n])


def _place_mentions(e):
    """Yield each mentioned place once, using the full field path for a field read."""
    if isinstance(e, Move):
        yield e.name, e.fields
        return
    root, path = field_place(e)
    if isinstance(root, Var):
        yield root.name, path
        return
    for k in expr_kids(e):
        yield from _place_mentions(k)


def same_place(a, b):
    ar, ap = field_place(a)
    br, bp = field_place(b)
    return isinstance(ar, Var) and isinstance(br, Var) and ar.name == br.name and ap == bp


def array_resize_params(prog, *, length_only=False):
    """Find inout arrays that may be replaced or regrown, including through forwarding."""
    from foreign_shim import _walk

    arrays = {(f.name, i) for f in prog.functions for i, p in enumerate(f.params)
              if p.mode == "inout" and is_array(p.type)}
    resizes = set()
    forwarded = {}
    for f in prog.functions:
        params = {p.name: (f.name, i) for i, p in enumerate(f.params) if (f.name, i) in arrays}
        if not params:
            continue

        def visit(node):
            if isinstance(node, Assign) and node.name in params:
                resizes.add(params[node.name])
            elif isinstance(node, Call):
                for i, arg in enumerate(node.args):
                    target = (node.name, i)
                    if not isinstance(arg, InoutArg) or not isinstance(arg.expr, Var) or arg.expr.name not in params:
                        continue
                    source = params[arg.expr.name]
                    if node.name in ARRAY_GROW_BUILTINS and (node.name != "arr_reserve" or not length_only):
                        resizes.add(source)
                    elif target in arrays:
                        forwarded.setdefault(target, set()).add(source)

        _walk(f.body, visit)
    pending = list(resizes)
    while pending:
        for source in forwarded.get(pending.pop(), ()):
            if source not in resizes:
                resizes.add(source)
                pending.append(source)
    return resizes


def array_resize_args(f, resizes):
    """Bare bindings passed to resizable array parameters in f."""
    from foreign_shim import _walk

    names = set()

    def visit(node):
        if isinstance(node, Call):
            for i, arg in enumerate(node.args):
                if (((node.name, i) in resizes or node.name in ARRAY_GROW_BUILTINS) and isinstance(arg, InoutArg)
                        and isinstance(arg.expr, Var)):
                    names.add(arg.expr.name)

    _walk(f.body, visit)
    return names


def _field_move_error(e):
    terr(e, "foreign_move", "a field moved out must be reassigned by the next statement",
         "restore the field immediately, or pass the move to a take call assigned back to the same field")


def _sccs(n, adj):
    """G9 A0: strongly connected components of the graph on nodes 0..n-1 (adj[i]: successors of i), iterative Tarjan.
    -> list of member lists (each ascending), ordered by their smallest member."""
    index_of = [-1] * n
    low = [0] * n
    on_stack = [False] * n
    stack = []
    out = []
    counter = 0
    for root in range(n):
        if index_of[root] != -1:
            continue
        index_of[root] = low[root] = counter
        counter += 1
        stack.append(root)
        on_stack[root] = True
        work = [(root, 0)]
        while work:
            v, i = work[-1]
            if i < len(adj[v]):
                work[-1] = (v, i + 1)
                w = adj[v][i]
                if index_of[w] == -1:
                    index_of[w] = low[w] = counter
                    counter += 1
                    stack.append(w)
                    on_stack[w] = True
                    work.append((w, 0))
                elif on_stack[w]:
                    low[v] = min(low[v], index_of[w])
            else:
                work.pop()
                if work:
                    p = work[-1][0]
                    low[p] = min(low[p], low[v])
                if low[v] == index_of[v]:
                    comp = []
                    while True:
                        w = stack.pop()
                        on_stack[w] = False
                        comp.append(w)
                        if w == v:
                            break
                    out.append(sorted(comp))
    out.sort(key=lambda c: c[0])
    return out


def _cycle_through(start, adj, members):
    """shortest cycle start -> ... -> start inside `members` (breadth first, successors in adjacency order);
    -> list of nodes beginning and ending with start"""
    members = set(members)
    prev = {start: None}
    queue = [start]
    qi = 0
    while qi < len(queue):
        v = queue[qi]
        qi += 1
        for w in adj[v]:
            if w not in members:
                continue
            if w == start:
                x = v
                chain = []
                while x is not None:
                    chain.append(x)
                    x = prev[x]
                chain.reverse()          # start ... v
                return chain + [start]
            if w not in prev:
                prev[w] = v
                queue.append(w)
    return [start, start]


class Checker:
    def __init__(s, prog, externs=()):
        s.prog = prog
        if prog.explicit:                # G4 A0: explicit visibility mode: resolve names to identities first
            from modules import resolve
            resolve(prog)
        s.externs = frozenset(externs)   # G6: crate names given with --rust-extern
        s.uses_inout = program_uses_inout(prog)   # G5 A0: enables the contextual builtin `old`
        prog.uses_inout = s.uses_inout            # G5 A0: lets the staging guard skip its walk for inout-free programs
        s.ckind = "code"          # G5: kind of the expression being checked: requires/ensures/decreases/invariant/assert/code
        s.boundary = None         # statement boundary for the statement-only builtins
        s.statement_call = None
        s.restore_array = None
        s.match_movable = set()   # G-DB16-1: array binders of the enclosing arms that may be moved (see Match)
        s.arm_binders = set()     # G-DB16-1: every binder of an enclosing match arm (a view, never an owner)
        s.in_par = False
        s.in_view = False         # LG6: checking the operands of a subview(...) argument
        s.funcs = {}
        for f in prog.functions:
            if f.name in s.funcs:
                terr(f, "dup_fn", "function %s defined twice" % f.name)
            if f.name in BUILTIN_FNS or f.name in CONTRACT_ONLY or (f.name in CATALOG_NAMES and f.name not in ASYNC_OPS):
                terr(f, "reserved", "%s is a builtin name" % f.name, "rename the function")
            if s.uses_inout and (f.src_name or f.name) == "old":
                terr(f, "reserved", "old is a builtin name in a program that uses inout", "rename the function")
            s.funcs[f.name] = f
        s.calls = {}
        s.decls = {}      # type name -> TypeDecl (first declaration)
        s.tt = None
        s.alloc_hint = None   # G2: (alloc Call node, expected array type) for the three allowed sites
        # G3: type-parameter context of the declaration being checked
        s.ctx_names = set()       # names of its type parameters
        s.ctx_generic = False     # does the declaration declare type parameters?
        s.elem_params = set()     # identities (owner, index) of its Element-bounded parameters
        s.arg_decls = {}          # type name -> (number of type parameters, []) for typeinfo.check_arity/is_value
        # G9 A0 metadata
        s.type_sccs = []          # recursive type SCCs (lists of names in declaration order), SCC-number order
        s.rec_vec_edges = set()   # (declaration, element) for intra-SCC Vec fields
        s.rec_vec_elems = set()
        s.boxed = []              # "Decl.field" / "Enum::Variant.field" of every intra-SCC (boxed) field
        s.boxed_decls = set()     # (declaration name, variant name or None) holding at least one boxed field
        s.extended = set()        # names of extended declarations
        s.buffered = set()        # declarations transitively containing a buffer (array/str) field
        s.fn_sccs = []            # recursive function SCCs (lists of names in function order)
        s.call_log = []           # (caller Function, Call node, callee Function) of every checked user call
        s.match_call = s.field_receiver = s.read_arg = None
        s.map_iterations = []     # read-only (root, field path) of enclosing map loops
        s.map_borrows = []        # (root, field path, binders) of the enclosing mutable collection Some arms
        s.field_paths = {}        # id(root Var) -> field path of the outermost field read over it
        s.active_roots = []       # root variable names of the enclosing match arms on extended scrutinees
        s.map_read_roots = []     # the active_roots entries pushed by map_get Some arms
        s.map_read_paths = []     # (root, path) places held by map_get Some arms (R-6e-14 overlap check)
        s.match_paths = []        # (root, field path) of enclosing matches on extended places of inout roots
        s.s5_errors = []
        s.par_groups = []         # G7 A0: (enclosing Function, Par statement, [branch Function]) of every checked group
        s.x_raw = {}              # P06: function name -> reason it cannot be expanded (str) or its raw body
        s.x_memo = {}             # P06: function name -> typed expansion template over its parameter names
        s.x_busy = set()          # P06: templates being built (recursion)
        s.x_pending = None        # P06: expansion replacing the Call node just checked

    def run(s):
        refine_expand(s.prog)  # STAGE1 A: validate refinement annotations, desugar them into contracts/asserts
        s.declare_types()      # names, duplicates
        s.collection_records = set()
        s.borrowing_structs = {d.name for d in s.prog.types if isinstance(d, StructDecl)
                               and any(fd.type.name == "ref" for fd in d.fields)}
        s.normalize_types()    # G6: Vec/HashMap/String become catalog types unless the program declares them
        s.arg_decls = {n: (len(d.tparams), []) for n, d in s.decls.items()}
        s.check_tparams()      # G3: type-parameter lists (type_parameter, generic_bound, generic_main)
        value_decls = {n: (len(d.tparams), [fd.type for _, fd in s.decl_fields(d)])
                       for n, d in s.decls.items()}
        s.value_decls = value_decls
        record_map_values = {n for n, d in s.decls.items() if is_nongeneric_struct(Type(n), s.decls)
                             and all(f.type == STRING or is_value(f.type, value_decls, (n,))
                                     for f in d.fields)}
        def record_map_field(ty):
            return is_string_record_map(ty) and ty.args[-1].name in record_map_values

        s.aggregates = {n for n, d in s.decls.items() if is_nongeneric_struct(Type(n), s.decls)
                        and all(is_owned_field_type(f.type) or f.type == ARRAY_U8 or is_nested_collection_field(f.type)
                                or n in s.collection_records and record_map_field(f.type)
                                or is_value(f.type, value_decls, (n,))
                                for f in d.fields)}
        s.nested_records = {n for n, d in s.decls.items() if isinstance(d, StructDecl)
                            and any(is_nested_collection_field(f.type)
                                    or n in s.collection_records and record_map_field(f.type)
                                    for f in d.fields)}
        # D90: one level of record nesting. A holder is a non-generic struct with a direct field of an R-6e-6
        # record (an owned aggregate with nested collection fields); its other fields follow the aggregate rules.
        s.holder_records = {n for n, d in s.decls.items() if is_nongeneric_struct(Type(n), s.decls)
                            and n not in s.nested_records
                            and any(nested_child_type(s, f.type) for f in d.fields)
                            and all(nested_child_type(s, f.type) or is_owned_field_type(f.type) or f.type == ARRAY_U8
                                    or is_value(f.type, value_decls, (n,)) for f in d.fields)}
        s.aggregates |= s.holder_records
        s.nested_records |= s.holder_records
        s.resolve_types()      # every type reference and the recursive type SCCs
        s.inhabitance()        # G9 A0: every declaration needs a finite value
        s.compute_extended()
        s.check_extended_uses()
        s.check_modes()        # G5 A0: parameter modes, before host entries and bodies
        if s.prog.explicit:
            s.check_entries()      # G4 A0: host-entry signatures
        s.tt = TypeTable(s.prog)
        s.x_prepare()          # P06: which functions a contract may expand, and their raw bodies
        s.check_struct_invariants()
        for f in s.prog.functions:
            s.fn(f)
        if s.prog.explicit:
            resizes = array_resize_params(s.prog)
            for f in s.prog.functions:
                if f.host_entry:
                    for i, p in enumerate(f.params):
                        if (f.name, i) in resizes:
                            terr(_mode_at(f, p), "entry_signature",
                                 "host entry %s cannot reassign inout array parameter %s" %
                                 (getattr(f, "disp", f.name), p.name))
        s.check_fn_sccs()      # G9 A0: measures and stable type parameters of recursive call SCCs
        s.check_par_eligibility()   # G7 A0: transitive eligibility of every par branch (effects, foreign, nesting)
        s.check_costs()
        s.record_g9()
        s.x_auto()             # P07: code calls of an expandable function know its body (result == expansion)
        if any(f.pred for f in s.prog.functions):   # R03L (R-6-2): every pred use is expanded; no pred reaches a back end
            s.prog.functions = [f for f in s.prog.functions if not f.pred]
            s.prog.items = [i for i in s.prog.items if not getattr(i, "pred", False)]

    def check_costs(s):
        """Ordinary typing wins, followed by cost/spent diagnostics and direct call effects."""
        for f in s.prog.functions:
            if f.cost is None:
                continue
            s.f = f
            s.enter(fn_owner(f.name), f.tparams)
            s.ckind = "cost"
            scope = {p.name: (p.type, False, "inout" if p.mode else "param") for p in f.params}
            before = len(s.s5_errors)
            check_try_position(f.cost, f.cost)
            ty = s.expr(f.cost, scope, True)
            if ty != I64 and len(s.s5_errors) == before:
                s.s5_errors.append((f, f.cost, "cost_type", "cost expression must have type i64"))
        if s.s5_errors:
            order = {id(f): i for i, f in enumerate(s.prog.functions)}
            _, e, code, msg = min(s.s5_errors, key=lambda x: (order[id(x[0])], x[1].line, x[1].col))
            terr(e, code, msg)
        for f in s.prog.functions:
            if f.cost is None:
                continue
            items = []
            s.par_items(f.body, items)
            for kind, e in sorted(items, key=lambda x: (x[1].line, x[1].col)):
                if kind != "call":
                    continue
                callee = s.funcs.get(e.name)
                if e.name in CATALOG or callee is not None and callee.foreign is not None:
                    terr(e, "cost_unsupported", "cost functions cannot call foreign, catalog or file operations", stage="effect")
                if callee is not None and callee.cost is None:
                    terr(e, "cost_callee", "`%s` has no cost clause, so its work is unbounded here" %
                         (callee.src_name or callee.name), stage="effect")

    # ---- user type declarations ----
    def declare_types(s):
        for d in s.prog.types:
            if d.src_name is None and d.name != "subview" and not TYPE_NAME_RE.match(d.name):   # G4: explicit mode checked the written names
                terr(d, "type_name", "type name %s must start with an upper-case letter" % d.name,
                     "rename it, e.g. %s" % (d.name[:1].upper() + d.name[1:]))
            if d.src_name is None and (d.name == "subview" or d.name in RESERVED_TYPE_NAMES):
                terr(d, "reserved", "%s is a builtin name" % d.name, "rename the type")
            if d.name in s.decls:
                terr(d, "dup_type", "type %s defined twice" % d.name)
            if d.name in s.funcs:
                terr(d, "name_collision", "%s is both a type and a function" % d.name,
                     "rename the type or the function")
            s.decls[d.name] = d
            if isinstance(d, StructDecl):
                s.check_field_names(d.fields, "struct %s" % d.name)
            else:
                if not d.variants:
                    terr(d, "empty_enum", "enum %s has no variants" % d.name, "add at least one variant")
                seen = set()
                for v in d.variants:
                    if not TYPE_NAME_RE.match(v.name):
                        terr(v, "variant_name", "variant name %s must start with an upper-case letter" % v.name,
                             "rename it, e.g. %s" % (v.name[:1].upper() + v.name[1:]))
                    if v.name in seen:
                        terr(v, "dup_variant", "variant %s defined twice in enum %s" % (v.name, d.name))
                    seen.add(v.name)
                    s.check_field_names(v.fields, "variant %s::%s" % (d.name, v.name))

    # ---- G6 A0: collection spellings ----
    def norm_type(s, ty):
        """the parser yields nominal Vec/HashMap/String/File; map them to the catalog types unless a user type of
        that name is declared (an existing user type keeps its meaning)"""
        if ty.owner is not None:
            return ty
        args = tuple(s.norm_type(a) for a in ty.args)
        name = ty.name
        if name not in s.decls and name in ("Vec", "HashMap", "String", "File", "Process", "ProcessArgs", "Socket",
                                            "Listener", "Loop"):
            name = {"Vec": "vec", "HashMap": "hashmap", "String": "string", "File": "file",
                    "Process": "process", "ProcessArgs": "process_args", "Socket": "socket",
                    "Listener": "listener", "Loop": "loop"}[name]
        if name in ("vec", "hashmap") and args and args[-1].owner is None and not args[-1].args:
            s.collection_records.add(args[-1].name)
        if name == "result" and len(args) in (1, 2):
            return mk_result(*args)
        if name == ty.name and args == ty.args:
            return ty
        if name == "array" and len(args) == 1:
            return mk_array(args[0])
        if name == "option" and len(args) == 1:
            return mk_option(args[0])
        return Type(name, args)

    def borrowed_type(s, ty):
        return ty.owner is None and (ty.name == "ref" or ty.name in s.borrowing_structs)

    def contains_borrow(s, ty):
        return s.borrowed_type(ty) or any(s.contains_borrow(t) for t in ty.args)

    def borrow_signature(s, node, message):
        terr(node, "borrow_signature", message)

    def borrow_type_position(s, node, ty, allowed):
        if not s.contains_borrow(ty):
            return ty
        if (not allowed or not s.borrowed_type(ty) or
                (ty.name == "ref" and ty.args not in ((ARRAY_U8,), (STR,)))):
            s.borrow_signature(node, "borrowed type is not allowed in this position")
        if s.prog.borrow_site is None:
            s.prog.borrow_site = node
        # The verifier uses the ordinary value type. Checking and lowering retain
        # borrow information on the declaring node and on borrowed expressions.
        return ty.args[0] if ty.name == "ref" else ty

    def normalize_types(s):
        for d in s.prog.types:
            if d.name in s.borrowing_structs:
                d.borrowing = True
            for fd in (d.fields if isinstance(d, StructDecl) else [x for v in d.variants for x in v.fields]):
                fd.ref = fd.type.name == "ref"
                fd.type = s.norm_type(s.borrow_type_position(
                    fd, fd.type, d.name in s.borrowing_structs and not d.tparams))
        for f in s.prog.functions:
            boundary = f.host_entry or f.foreign is not None or bool(f.tparams)
            for p in f.params:
                p.type = s.norm_type(s.borrow_type_position(
                    f, p.type, not (boundary or p.mode or p.take or p.type.name == "ref")))
            borrowed_result = s.borrowed_type(f.ret)
            f.ref_result = f.ret.name == "ref"
            f.ret = s.norm_type(s.borrow_type_position(f, f.ret, not boundary))
            if borrowed_result or f.borrow_from is not None:
                params = {p.name: p for p in f.params}
                if (not borrowed_result or not f.borrow_from or len(set(f.borrow_from)) != len(f.borrow_from)
                        or any(n not in params or params[n].take
                               or not (params[n].type in (ARRAY_U8, STR)
                                       or not params[n].mode and s.borrowed_type(params[n].type))
                               for n in f.borrow_from)):
                    s.borrow_signature(f, "a borrowed result needs a from list naming its source parameters")
                for ens in f.ensures:
                    if any(params[n].mode and _mentions(ens, n) for n in f.borrow_from):
                        s.borrow_signature(ens, "ensures cannot read an inout source of a borrowed result")
            for e in f.requires + f.ensures:
                s.norm_expr(e)
            if f.decreases is not None:
                s.norm_expr(f.decreases)
            if f.cost is not None:
                s.norm_expr(f.cost)
            s.norm_stmts(f.body, bool(f.tparams))

    def norm_stmts(s, stmts, generic=False):
        for st in stmts:
            if isinstance(st, Let):
                st.ref = st.type.name == "ref"
                st.type = s.norm_type(s.borrow_type_position(st, st.type, not (st.mutable or generic)))
                s.norm_expr(st.value)
            elif isinstance(st, (Assign, FieldAssign, Return)):
                if st.value is not None:
                    s.norm_expr(st.value)
            elif isinstance(st, IndexAssign):
                s.norm_expr(st.index)
                s.norm_expr(st.value)
            elif isinstance(st, If):
                s.norm_expr(st.cond)
                s.norm_stmts(st.then, generic)
                s.norm_stmts(st.els, generic)
            elif isinstance(st, While):
                s.norm_expr(st.cond)
                for i in st.invariants:
                    s.norm_expr(i)
                s.norm_expr(st.decreases)
                s.norm_stmts(st.body, generic)
            elif isinstance(st, ForMap):
                s.norm_expr(st.place)
                for i in st.invariants:
                    s.norm_expr(i)
                s.norm_stmts(st.body, generic)
            elif isinstance(st, Assert):
                s.norm_expr(st.cond)
            elif isinstance(st, ExprStmt):
                s.norm_expr(st.expr)
            elif isinstance(st, Par):      # G7 A0
                for c in st.calls:
                    s.norm_expr(c)
            elif isinstance(st, Match):
                s.norm_expr(st.scrutinee)
                for a in st.arms:
                    s.norm_stmts(a.body, generic)

    def norm_expr(s, e):
        targs = getattr(e, "targs", None)
        if targs is not None:
            e.targs = [s.norm_type(s.borrow_type_position(e, t, False)) for t in targs]
            if (isinstance(e, Call) and e.name in CATALOG and e.name not in s.funcs
                    and e.name.startswith(("vec_", "map_")) and e.targs
                    and e.targs[-1].owner is None and not e.targs[-1].args):
                s.collection_records.add(e.targs[-1].name)
        for k in expr_kids(e):
            s.norm_expr(k)

    def check_field_names(s, fields, where):
        seen = set()
        for f in fields:
            if not FIELD_NAME_RE.match(f.name):
                terr(f, "field_name", "field name %s must start with a lower-case letter" % f.name,
                     "rename it, e.g. %s" % (f.name[:1].lower() + f.name[1:]))
            if f.name in seen:
                terr(f, "dup_field", "field %s defined twice in %s" % (f.name, where))
            seen.add(f.name)

    # ---- G3: type parameters and type applications ----
    def enter(s, owner, tparams):
        """make the type-parameter list of the declaration being checked the current context"""
        s.ctx_names = set(tp.name for tp in tparams)
        s.ctx_generic = bool(tparams)
        s.elem_params = set((owner, i) for i, tp in enumerate(tparams) if tp.bound == "Element")

    def check_tparams(s):
        for d in s.prog.types:
            s.tparam_list(d.tparams)
        for f in s.prog.functions:
            if f.name == "main" and f.tparams:
                terr(f, "generic_main", "main cannot declare type parameters",
                     "make main a plain function; call generic functions from it with explicit type arguments")
            s.tparam_list(f.tparams)

    def tparam_list(s, tparams):
        seen = set()
        for tp in tparams:
            if not TYPE_NAME_RE.match(tp.name):
                terr(tp, "type_parameter", "type parameter %s must start with an upper-case letter" % tp.name,
                     "rename it, e.g. %s" % (tp.name[:1].upper() + tp.name[1:]))
            if tp.name in RESERVED_TYPE_NAMES or tp.name in s.decls:
                terr(tp, "type_parameter", "type parameter %s shadows a declared or built-in type" % tp.name,
                     "choose a name that is not a type, e.g. T")
            if tp.name in seen:
                terr(tp, "type_parameter", "type parameter %s is declared twice" % tp.name,
                     "give every parameter of a declaration its own name")
            seen.add(tp.name)
            if tp.bound is not None and tp.bound != "Element":
                terr(tp, "generic_bound", "unknown bound %s on type parameter %s" % (tp.bound, tp.name),
                     "the only bound is Element: <%s: Element>" % tp.name)

    def check_type(s, node, ty):
        """every type reference: arity, known names, type-argument and bound rules (G3_SPEC 3.1, 3.2, 3.4)"""
        s.check_type_arity(node, ty)
        s.check_type_rec(node, ty)

    def check_type_arity(s, node, ty):
        try:
            check_arity(ty, s.arg_decls)
        except TypeArityError as ex:
            terr(node, "type_arity", str(ex), "give the exact number of type arguments, e.g. Option<i64>, Pair<i64, bool>")

    def check_type_rec(s, node, ty):
        if ty.owner is not None:
            return
        if ty.name in ("option", "result"):
            if ty.args and not (ty.name == "result" and ty.args[0] == UNIT):   # LB8 R-6g7-1: Result<(), E>
                s.check_arg(node, ty.args[0], None)
            if ty.name == "result" and len(ty.args) == 2:
                s.check_result_error(node, result_error(ty))
            return
        if ty.name in COLLECTION_ARITY:      # G6: Vec<T>, HashMap<K, V>, String
            for a in ty.args:
                s.check_type_rec(node, a)
            if ty.name == "vec":
                if ty.args[0] != STRING:
                    s.collection_payload(node, ty.args[0], "a Vec element")
            elif ty.name == "hashmap":
                k = ty.args[0]
                if k != STRING and not is_element(k, s.elem_params):
                    terr(node, "foreign_owned_type", "%s cannot be a HashMap key: only i64, bool, String and Element type "
                         "parameters can" % type_name(k),
                         "use i64, bool or String keys, or declare the parameter `: Element`")
                v = ty.args[1]
                s.collection_payload(node, v, "a HashMap value")
                if k == STRING and v not in (I64, BOOL) and v.name not in s.aggregates:
                    terr(node, "foreign_owned_type", "String-keyed maps require i64, bool or owned aggregate values")
                if k not in (I64, STRING) and v.name in s.aggregates and (v.name in s.holder_records or any(   # D90
                        is_collection(f.type) or f.type == ARRAY_U8 for f in s.decls[v.name].fields)):
                    terr(node, "foreign_owned_type", "aggregate HashMap values with owned fields require i64 or String keys")
            return
        if ty.name == "array":
            if ty.args:                      # [X] for X other than i64/u8/bool
                el = ty.args[0]
                s.check_type_rec(node, el)
                if contains_collection(el):
                    terr(node, "foreign_owned_type", "an array cannot hold the collection type %s" % type_name(el),
                         "arrays hold i64, bool or an Element type parameter")
                if el.owner is not None:
                    if (el.owner, el.index) not in s.elem_params:
                        terr(node, "generic_bound", "[%s] requires %s: Element" % (el.name, el.name),
                             "declare the parameter as <%s: Element>" % el.name)
                else:
                    terr(node, "type_argument", "%s cannot be an array element" % type_name(el),
                         "arrays hold i64, bool or an Element type parameter")
            return
        if ty.name in BUILTIN_TYPE_NAMES:
            return
        if ty.name in s.ctx_names:
            terr(node, "type_parameter", "type parameter %s is not a type constructor" % ty.name,
                 "write %s without type arguments" % ty.name)
        d = s.decls.get(ty.name)
        if d is None:
            if s.ctx_generic:
                terr(node, "type_parameter", "unknown type %s: it is neither declared nor a type parameter of this declaration" % ty.name,
                     "declare it in the type-parameter list, e.g. <%s>, or declare the type" % ty.name)
            terr(node, "unknown_type", "unknown type %s" % ty.name,
                 "declare it with `struct %s { ... }` or `enum %s { ... }`" % (ty.name, ty.name))
        for a, tp in zip(ty.args, d.tparams):
            s.check_arg(node, a, tp)

    def collection_payload(s, node, a, what):
        """G6: a collection payload is a plain G3 value (never another collection)"""
        if contains_collection(a):
            terr(node, "foreign_owned_type", "%s cannot be the collection type %s" % (what, type_name(a)),
                 "collection payloads are plain values; keep every collection in its own binding")
        if not is_value(a, s.arg_decls):
            terr(node, "foreign_owned_type", "%s cannot have type %s: only i64, bool, Option, Result, structs, "
                 "enums and type parameters can" % (what, type_name(a)),
                 "arrays, str and () cannot be collection payloads")

    def check_result_error(s, node, error):
        # Checking all declared fields also excludes transitive buffers and owned collections.
        if error != STRING and (error.owner is not None or error.name in ("option", "result")
                                or not is_value(error, s.value_decls)):
            terr(node, "result_error_type",
                 "%s is not supported as a Result error type" % type_name(error),
                 "use i64, bool, String or a value struct or enum")
        s.check_type_rec(node, error)

    def check_arg(s, node, a, tp):
        """a: a type argument; tp: the TypeParam it instantiates (None for Option/Result)"""
        s.check_type_rec(node, a)
        # Option, Result and user declarations cannot take collections or records with nested fields.
        if contains_collection(a) or any(n in s.nested_records for n in dependencies(a)):
            terr(node, "foreign_owned_type", "%s cannot be a type argument" % type_name(a),
                 "keep collections in their own bindings; Vec<T> and HashMap<K, V> are the only places they appear")
        if not is_value(a, s.arg_decls):
            terr(node, "type_argument", "%s is not a valid type argument: only i64, bool, Option, Result, "
                 "structs, enums and type parameters are" % type_name(a),
                 "arrays, str and () cannot be type arguments")
        if tp is not None and tp.bound == "Element" and not is_element(a, s.elem_params):
            terr(node, "generic_bound", "%s does not satisfy the bound Element of %s" % (type_name(a), tp.name),
                 "only i64, bool and type parameters declared `: Element` satisfy Element")

    def instantiate(s, e, what, owner, tparams, targs):
        """steps 1-4 of G3_SPEC 3.4 for a call / literal / constructor `e` written with `targs` (None = no type
        application) -> substitution {(owner, index): Type}, or None for a non-generic declaration"""
        if targs is None:
            if tparams:
                terr(e, "type_arity", "%s expects %d type argument(s)" % (what, len(tparams)),
                     "write the type arguments explicitly, e.g. %s::<T>; they are never inferred" % what.split()[-1])
            return None
        if not tparams:
            terr(e, "type_arity", "%s takes no type arguments" % what, "remove the `::<...>`")
        if len(targs) != len(tparams):
            terr(e, "type_arity", "%s expects %d type argument(s), got %d" % (what, len(tparams), len(targs)),
                 "give exactly one type argument per type parameter")
        catalog_collection = (isinstance(e, Call) and e.name not in s.funcs and (
            e.name in ("vec_new", "vec_push", "vec_get", "vec_at_mut", "vec_clear", "vec_reserve", "vec_extend_from_slice",
                       "map_new", "map_get", "map_get_mut", "map_insert", "map_remove", "map_take")
            or e.name == "map_lookup" and targs[0] == STRING))
        if catalog_collection:
            if e.name == "map_take":
                k, v = targs
                if k not in (I64, STRING):
                    terr(e, "foreign_owned_type", "map_take requires i64 or String keys")
                if v not in (I64, BOOL) and (v.owner is not None or v.args or v.name not in s.aggregates):
                    terr(e, "foreign_owned_type", "map_take requires i64, bool or owned aggregate values")
            if e.name == "vec_extend_from_slice" and targs[0] not in (I64, BOOL):   # D95
                terr(e, "foreign_owned_type", "vec_extend_from_slice requires i64 or bool elements")
            s.check_type(e, Type("vec" if e.name.startswith("vec_") else "hashmap", tuple(targs)))
        for a, tp in zip(targs, tparams):
            s.check_type(e, a)
            if catalog_collection and e.name.startswith("vec_") and s.rec_vec_elem(a):
                continue
            if catalog_collection:
                s.ext_misuse(e, a, "coll")
            else:
                s.check_arg(e, a, tp)
                s.ext_misuse(e, a, "arg")
        return bind_args(owner, len(tparams), targs)

    def resolve_types(s):
        graph_error = None
        try:
            s.type_graph()
        except PruvError as ex:
            if ex.code == "recursive_generic_type":
                raise
            graph_error = ex
        for d in s.prog.types:
            s.enter(decl_owner(d.name), d.tparams)
            groups = [d.fields] if isinstance(d, StructDecl) else [v.fields for v in d.variants]
            for fields in groups:
                for f in fields:
                    if d.tparams and (is_array(f.type) or f.type == STR):   # G9 A0
                        terr(f, "field_type", "generic type %s cannot have the array/str field %s" % (d.name, f.name),
                             "use a dedicated non-generic declaration")
                    if f.type == UNIT or (is_array(f.type) and f.type not in (ARRAY, ARRAY_U8, ARRAY_BOOL)) or (
                            f.type.owner is None and f.type.name == "result" and len(f.type.args) in (1, 2) and f.type.args[0] == UNIT):
                        terr(f, "field_type", "field %s cannot have type %s" % (f.name, type_name(f.type)),
                             "fields are i64, bool, Option<i64>, Result<i64> or a user struct/enum")
                    s.check_type(f, f.type)
                    s.check_collection_field(d, f)
        for f in s.prog.functions:
            s.enter(fn_owner(f.name), f.tparams)
            for p in f.params:
                s.check_type(f, p.type)
            s.check_type(f, f.ret)
        if graph_error is not None:
            raise graph_error

    def check_collection_field(s, d, f):
        if s.rec_vec(f.type) and (isinstance(d, StructDecl) or (d.name, f.type.args[0].name) in s.rec_vec_edges):
            if not d.tparams:
                return
        state_field = isinstance(d, StructDecl) and (
            f.type.name == "vec" or f.type.name == "hashmap" and f.type.args[0] in (I64, STRING)
        ) and not f.type.args[-1].args and f.type.args[-1].name in s.aggregates
        nested_field = isinstance(d, StructDecl) and is_nested_collection_field(f.type)
        if nested_field and f.type.name == "hashmap" and f.type.args[-1] == I64:
            nested_field = d.name in s.collection_records
        holder_field = (d.name in s.holder_records and f.type.name not in s.holder_records
                        and nested_child_type(s, f.type))
        if not state_field and not holder_field and any(n in s.nested_records for n in dependencies(f.type)):
            terr(f, "foreign_owned_type", "a record with nested collection fields cannot be a direct field")
        if contains_collection(f.type) and (
                d.tparams or not (is_owned_field_type(f.type) or state_field or nested_field)):
            terr(f, "foreign_owned_type", "field %s cannot have the collection type %s" % (
                f.name, type_name(f.type)),
                 "only non-generic declarations may have Vec<i64>, Vec<bool> or String fields")

    # ---- G9 A0: type graph, inhabitance, extended types ----
    @staticmethod
    def decl_fields(d):
        """[(variant name or None, FieldDecl)] of a declaration in variant/field source order"""
        if isinstance(d, StructDecl):
            return [(None, fd) for fd in d.fields]
        return [(v.name, fd) for v in d.variants for fd in v.fields]

    @staticmethod
    def field_label(d, vname, fd):
        return "%s.%s" % (d.name, fd.name) if vname is None else "%s::%s.%s" % (d.name, vname, fd.name)

    def is_plain_nominal(s, ty):
        """a declared user type written without type arguments"""
        return ty.owner is None and not ty.args and ty.name in s.decls and ty.name not in BUILTIN_TYPE_NAMES

    def rec_vec_elem(s, ty):
        return ty.owner is None and not ty.args and ty.name in s.rec_vec_elems

    def rec_vec(s, ty):
        return ty.owner is None and ty.name == "vec" and s.rec_vec_elem(ty.args[0])

    def type_graph(s):
        types = s.prog.types
        index = {}
        for i, d in enumerate(types):
            index.setdefault(d.name, i)
        adj = [[] for _ in types]
        for i, d in enumerate(types):
            for _, fd in s.decl_fields(d):
                for n in dependencies(fd.type):    # G3: nested type arguments count
                    j = index.get(n)
                    if j is not None and j not in adj[i]:
                        adj[i].append(j)
        for comp in _sccs(len(types), adj):
            if not (len(comp) > 1 or comp[0] in adj[comp[0]]):
                continue
            names = [types[i].name for i in comp]
            members = set(names)
            s.type_sccs.append(names)
            # (a) generic declarations cannot be recursive
            for i in comp:
                d = types[i]
                if d.tparams:
                    cyc = [types[k].name for k in _cycle_through(i, adj, comp)]
                    terr(d, "recursive_generic_type", "generic type %s cannot be recursive (cycle: %s)" % (
                        d.name, " -> ".join(cyc)),
                         "recursive types must be non-generic; write a dedicated enum for the element type")
            # (b) a Vec supplies its own indirection; direct nominal edges need a Box
            for i in comp:
                d = types[i]
                for vname, fd in s.decl_fields(d):
                    if not any(n in members for n in dependencies(fd.type)):
                        continue
                    if (fd.type.owner is None and fd.type.name == "vec" and len(fd.type.args) == 1
                            and s.is_plain_nominal(fd.type.args[0])
                            and fd.type.args[0].name in members):
                        elem = fd.type.args[0].name
                        s.rec_vec_edges.add((d.name, elem))
                        s.rec_vec_elems.add(elem)
                        continue
                    if not (s.is_plain_nominal(fd.type) and fd.type.name in members):
                        terr(fd, "recursive_type", "recursive field %s must have the type itself, not hide it inside %s" % (
                            fd.name, type_name(fd.type)),
                             "use a direct field of a dedicated non-generic type")
                    s.boxed.append(s.field_label(d, vname, fd))
                    s.boxed_decls.add((d.name, vname))

    def inhabitance(s):
        marked = set()
        # worklist: per struct / per variant count of fields whose type is not yet inhabited
        types = s.prog.types
        counts = []
        rev = {}        # type name -> [(decl index, unit index)], one entry per dependent field
        queue = []
        for i, d in enumerate(types):
            units = [d.fields] if isinstance(d, StructDecl) else [v.fields for v in d.variants]
            cs = []
            for u, fields in enumerate(units):
                c = 0
                for fd in fields:
                    if s.is_plain_nominal(fd.type):
                        c += 1
                        rev.setdefault(fd.type.name, []).append((i, u))
                cs.append(c)
            counts.append(cs)
            if 0 in cs:
                queue.append(i)
        while queue:
            i = queue.pop()
            name = types[i].name
            if name in marked:
                continue
            marked.add(name)
            for j, u in rev.get(name, ()):
                counts[j][u] -= 1
                if counts[j][u] == 0 and types[j].name not in marked:
                    queue.append(j)
        for d in s.prog.types:
            if d.name in marked:
                continue
            path = [d.name]
            cur = d
            while True:
                nxt = None
                for _, fd in s.decl_fields(cur):
                    if s.is_plain_nominal(fd.type) and fd.type.name not in marked:
                        nxt = fd.type.name
                        break
                if nxt is None:
                    break
                path.append(nxt)
                if path.index(nxt) != len(path) - 1:
                    break
                cur = s.decls[nxt]
            terr(d, "recursive_type", "recursive type has no finite value: %s" % " -> ".join(path),
                 "add a variant without recursive fields (a base case such as `Empty`)")

    def compute_extended(s):
        """extended = recursive, or with a buffer field, or with a field mentioning an extended declaration;
        buffered = with a buffer field directly or through nested declarations"""
        rec = set(n for names in s.type_sccs for n in names)
        direct = set()
        for d in s.prog.types:
            if any(is_array(fd.type) or fd.type == STR or is_collection(fd.type) for _, fd in s.decl_fields(d)):
                direct.add(d.name)
        ext = set(rec) | direct
        buf = set(direct)
        # reverse dependency edges built once: mentioned name -> declaration names mentioning it
        rev = {}
        for d in s.prog.types:
            for n in set(n for _, fd in s.decl_fields(d) for n in dependencies(fd.type)):
                rev.setdefault(n, []).append(d.name)
        for marked in (ext, buf):
            queue = sorted(marked)
            while queue:
                n = queue.pop()
                for m in rev.get(n, ()):
                    if m not in marked:
                        marked.add(m)
                        queue.append(m)
        s.extended = ext
        s.buffered = buf

    def is_extended(s, ty):
        return ty.owner is None and not ty.args and ty.name in s.extended

    def is_zero_sized_struct(s, ty):
        if ty.owner is not None:
            return False
        d = s.decls.get(ty.name)
        if not isinstance(d, StructDecl) or s.is_extended(ty):
            return False
        subst = bind_args(decl_owner(d.name), len(d.tparams), ty.args)
        return all(s.is_zero_sized_struct(substitute(fd.type, subst)) for fd in d.fields)

    def ext_misuse(s, node, ty, inside=None):
        """an extended type may not be a type argument (type_argument) or a G6 collection payload (foreign_owned_type)"""
        if ty.owner is not None:
            return
        if is_collection(ty):
            if s.rec_vec(ty):
                return
            for a in ty.args:
                s.ext_misuse(node, a, "coll")
            return
        if inside == "coll" and s.is_zero_sized_struct(ty):
            name = type_name(ty)
            terr(node, "foreign_owned_type", "%s has no data and cannot be a Vec or HashMap element" % name,
                 "give %s a field" % name)
        if inside == "coll" and not ty.args and ty.name in s.aggregates and not s.rec_vec_elem(ty):
            return
        if ty.name in s.extended and not ty.args and ty.name in s.decls:
            if inside in ("coll", "coll_nested"):
                terr(node, "foreign_owned_type", "%s cannot be a Vec or HashMap element: it is recursive or holds an "
                     "array/str" % type_name(ty), "keep extended values in their own bindings")
            if inside == "arg":
                terr(node, "type_argument", "%s cannot be a type argument or an Option/Result payload: it is "
                     "recursive or holds an array/str" % type_name(ty),
                     "use a dedicated non-generic enum instead")
            return
        for a in ty.args:
            s.ext_misuse(node, a, "coll_nested" if inside in ("coll", "coll_nested") else "arg")

    def check_extended_uses(s):
        for d in s.prog.types:
            for _, fd in s.decl_fields(d):
                if d.tparams:
                    hit = [n for n in dependencies(fd.type) if n in s.extended]
                    if hit:
                        terr(fd, "field_type", "generic type %s cannot have the field %s of the extended type %s" % (
                            d.name, fd.name, hit[0]), "use a dedicated non-generic declaration")
                s.ext_misuse(fd, fd.type)
        for f in s.prog.functions:
            for p in f.params:
                s.ext_misuse(f, p.type)
            s.ext_misuse(f, f.ret)

    def check_modes(s):
        """An inout parameter is a scalar, array, non-generic struct or map/vector place."""
        for f in s.prog.functions:
            for p in f.params:
                if p.mode is not None and not (p.type in (I64, BOOL) or is_array(p.type)
                                               or p.type == STR and p.name in (f.borrow_from or ())
                                               or is_nongeneric_struct(p.type, s.decls)
                                               or p.type.name == "vec" and p.type.owner is None
                                               or p.type.name == "hashmap" and p.type.owner is None
                                               and p.type.args[-1].name in s.aggregates
                                               and not p.type.args[-1].args):
                    terr(_mode_at(f, p), "inout_type", "inout parameter %s cannot have the type %s" %
                         (p.name, type_name(p.type)),
                         "use a scalar, an array, a non-generic struct, or a vector or a map of aggregate records, or remove inout")

    def is_result_record(s, ty):
        """R-6f-3: plain fields, Vec<i64>/Vec<bool>/Vec<String>, or Vec<E> of an owned aggregate
        E without nested collection fields (R-6e-6)"""
        if not is_nongeneric_struct(ty, s.decls):
            return False
        def ok(ft):
            if is_scalar_vec(ft) or ft == Type("vec", (STRING,)):
                return True
            if ft.owner is None and ft.name == "vec" and len(ft.args) == 1:
                e = ft.args[0]
                return (e.owner is None and not e.args and e.name in s.aggregates
                        and e.name not in s.nested_records
                        and not any(fd.type == ARRAY_U8 for fd in s.decls[e.name].fields))
            return (not contains_collection(ft) and not is_array(ft) and ft != STR
                    and not any(n in s.extended for n in dependencies(ft)))
        return any(is_collection(fd.type) for fd in s.decls[ty.name].fields) and \
            all(ok(fd.type) for fd in s.decls[ty.name].fields)

    def is_result_enum(s, ty):
        """R-6f-3: a non-generic enum whose payload fields are plain values, Vec<i64>/Vec<bool>/Vec<String> or R-6f-3 result records"""
        d = s.decls.get(ty.name)
        if ty.owner is not None or ty.args or not isinstance(d, EnumDecl) or d.tparams:
            return False
        fields = [fd.type for v in d.variants for fd in v.fields]
        def owned(ft):
            return is_scalar_vec(ft) or ft == Type("vec", (STRING,)) or s.is_result_record(ft)
        def ok(ft):
            return owned(ft) or (not contains_collection(ft) and not is_array(ft) and ft != STR
                                              and not any(n in s.extended for n in dependencies(ft)))
        return any(owned(ft) for ft in fields) and all(ok(ft) for ft in fields)

    def effective_invariants(s, ty, stack=()):
        """G-DB31-3: [(owner decl, clause, ((field, field type), ...))] of the struct type ty: its own clauses in
        declaration order, then those of each struct-typed field in declaration order (pre-order, no cycles)"""
        if ty in stack:
            return []
        d = s.decls.get(ty.name)
        out = []
        if ty.owner is None and not ty.args and isinstance(d, StructDecl):
            out = [(d, clause, ()) for clause in d.invariants]
        for step in s.invariant_steps(ty):
            if not s.reaches_invariant(step[1]):
                continue    # a position with no invariant below it is not walked (no path-wise revisit)
            for owner, clause, path in s.effective_invariants(step[1], stack + (ty,)):
                out.append((owner, clause, (step,) + path))
        return out

    def invariant_steps(s, ty):
        """LG23: the positions below ty that can hold an invariant struct, in order: (label, type, kind, extra) with
        kind field (struct or generic struct instance field), some (Option payload), ok (Result<U, E> Ok payload) or
        variant (field of a non-generic enum variant)"""
        if ty.owner is not None:
            return []
        d = s.decls.get(ty.name)
        if isinstance(d, StructDecl):
            if len(ty.args) != len(d.tparams):
                return []
            # d.fields, not fields_of: check_entries runs before the type table exists
            m = bind_args(decl_owner(ty.name), len(d.tparams), ty.args)
            return [(fd.name, substitute(fd.type, m), "field", fd.name) for fd in d.fields]
        if isinstance(d, EnumDecl):
            if len(ty.args) != len(d.tparams):
                return []
            m = bind_args(decl_owner(ty.name), len(d.tparams), ty.args)
            return [("%s.%s" % (v.name, fd.name), substitute(fd.type, m), "variant", (ty.name, v.name, fd.name))
                    for v in d.variants for fd in v.fields]
        if ty.name == "option" and len(ty.args) == 1:
            return [("some", ty.args[0], "some", None)]
        if ty.name == "result" and len(ty.args) == 2:
            return [("ok", ty.args[0], "ok", None), ("err", ty.args[1], "err", None)]
        return []

    def reaches_invariant(s, ty):
        """whether a plain struct with an invariant is reachable from ty through the invariant_steps (memoized)"""
        memo = s.__dict__.setdefault("_reach_inv", {})
        if ty not in memo:
            seen, todo, found = {ty}, [ty], False
            while todo and not found:
                cur = todo.pop()
                d = s.decls.get(cur.name)
                found = cur.owner is None and not cur.args and isinstance(d, StructDecl) and bool(d.invariants)
                for step in s.invariant_steps(cur):
                    if step[1] not in seen:
                        seen.add(step[1])
                        todo.append(step[1])
            memo[ty] = found
        return memo[ty]

    def invariant_struct(s, ty):
        if not s.reaches_invariant(ty):
            return None
        d = s.decls.get(ty.name)
        if ty.owner is None and not ty.args and isinstance(d, StructDecl) and d.invariants:
            return d
        return s.effective_invariants(ty)[0][0]

    def check_struct_invariants(s):
        decls = [d for d in s.prog.types if isinstance(d, StructDecl) and d.invariants]
        if not decls:
            return
        resolver = None
        if s.prog.explicit:
            from modules import _Resolver
            resolver = _Resolver(s.prog)
        for d in decls:
            if d.tparams:
                terr(d, "invariant_decl", "struct %s with type parameters cannot have an invariant" % d.name)
            for clause in d.invariants:
                at = _At(clause.invariant_line, clause.invariant_col, clause.file)
                if any(isinstance(e, (Var, Move)) and e.name == "result"
                       or isinstance(e, Call) and e.name == "old" for e in _walk(clause)):
                    terr(_At(clause.line, clause.col, clause.file), "invariant_decl",
                         "invariant of struct %s cannot use old() or result" % d.name)
            if resolver is not None:
                resolver.m = resolver.by_path[(d.pkg, d.mod_path)]
                resolver.own(d, decl_owner)
                resolved = []
                for clause in d.invariants:
                    expr = resolver.rexpr(clause)
                    expr.invariant_line = clause.invariant_line
                    expr.invariant_col = clause.invariant_col
                    resolved.append(expr)
                d.invariants = resolved
        if resolver is not None:
            # The digest walk includes instance attributes and records resolved cross-module references.
            resolver.module_edges()
        s.ckind = "requires"
        s.field_move = None
        for d in decls:
            s.f = d
            s.enter(decl_owner(d.name), d.tparams)
            scope = {fd.name: (fd.type, False, "param") for fd in d.fields}
            for clause in d.invariants:
                at = _At(clause.invariant_line, clause.invariant_col, clause.file)
                saved, s.collection_records = s.collection_records, set()
                try:
                    s.norm_expr(clause)
                finally:
                    s.collection_records = saved
                check_try_position(clause, clause)
                if s.expr(clause, scope, contract=True) != BOOL:
                    terr(_At(clause.line, clause.col, clause.file), "invariant_decl",
                         "invariant of struct %s must be bool" % d.name)
                clause.invariant_line, clause.invariant_col = at.line, at.col
        # Written contracts must build their own templates and record backend requirements.
        s.x_memo.clear()
        s.field_paths.clear()
        s.ckind = "code"
        for f in s.prog.functions:
            for p in f.params:
                d = s.invariant_struct(p.type)
                if d is not None and p.name == "result":
                    terr(f, "invariant_decl", "parameter result of type %s cannot be used: %s has an invariant" %
                         (d.name, d.name))
        if not s.prog.explicit:
            # LG17 (G-LG16-2): legacy mode has no host-entry check and every ordinary function is callable
            # from the host, so none may take a struct with an invariant (its callers are not proved).
            for f in s.prog.functions:
                if f.pred or f.foreign is not None:
                    continue
                for p in f.params:
                    d = s.invariant_struct(p.type)
                    if d is not None:
                        terr(f, "entry_signature",
                             "host entry %s cannot take parameter %s of struct %s with an invariant" %
                             (getattr(f, "disp", f.name), p.name, d.name))

    def invariant_vec_elem(s, ty):
        # LG27: element type of a top-level Vec<T> whose T reaches an invariant struct, else None
        if ty.owner is None and ty.name == "vec" and len(ty.args) == 1 and s.reaches_invariant(ty.args[0]):
            return ty.args[0]
        return None

    def invariant_clauses(s, ty, name):
        """the synthesized clauses of the value `name` of type ty (LG27: Vec<T> gets one forall per T clause)"""
        elem = s.invariant_vec_elem(ty)
        if elem is None:
            out = []
            for owner, clause, path in s.effective_invariants(ty):
                tag = (owner.name, clause.invariant_line, ".".join([name] + [x[0] for x in path]))
                out.append(invariant_clause(clause, ty, name, tag, path))
            return out
        out = []
        for owner, clause, path in s.effective_invariants(elem):
            tag = (owner.name, clause.invariant_line, name + "[]" + "".join("." + x[0] for x in path))
            body = invariant_clause(clause, ty, name, tag, (("[]", elem, "elem", None),) + tuple(path))
            ln, col, file = clause.line, clause.col, getattr(clause, "file", None)
            vec = Var(ln, col, name, ty)
            vec.file = file
            lo = IntLit(ln, col, 0, I64)
            lo.file = file
            hi = Call(ln, col, "len", [vec], I64)
            hi.file = file
            q = Quant(ln, col, "forall", "$inv_e", lo, hi, body, BOOL)
            q.file = file
            q.struct_invariant = tag
            out.append(q)
        return out

    def struct_contracts(s, f, m=None, node=None):
        """(requires, ensures) synthesized from the struct invariants of f's parameter and result types, with the
        type parameters substituted by m (LG24: instantiated generic call sites)"""
        requires, ensures = [], []

        for p in f.params:
            pty = substitute(p.type, m) if m else p.type
            d = s.invariant_struct(s.invariant_vec_elem(pty) or pty)
            if d is None:
                continue
            if p.name == "result":
                if m or node is not None:
                    continue        # LG24: instantiated call site; the callee is then not parametric-safe
                terr(node or f, "invariant_decl", "parameter result of type %s cannot be used: %s has an invariant" %
                     (d.name, d.name))
            for r in s.invariant_clauses(pty, p.name):
                requires.append(r)
                if p.mode == "inout":
                    ensures.append(copy.deepcopy(r))
        rty = substitute(f.ret, m) if m else f.ret
        if s.invariant_struct(s.invariant_vec_elem(rty) or rty) is not None:
            ensures.extend(s.invariant_clauses(rty, "result"))
        return requires, ensures

    @staticmethod
    def parametric_safe(f, m):
        """LG24: result-invariant ensures may be instantiated only if every type parameter of f in its parameter
        types sits in a walked position (option / two-argument result), no such parameter is named result,
        and every type parameter of the result type occurs in some parameter type"""
        def has_tp(ty):
            return ty.owner is not None and (ty.owner, ty.index) in m or any(has_tp(a) for a in ty.args)

        def tps(ty, out):
            if ty.owner is not None:
                if (ty.owner, ty.index) in m:
                    out.add((ty.owner, ty.index))
                return out
            for a in ty.args:
                tps(a, out)
            return out

        def walked(ty):
            if not has_tp(ty) or ty.owner is not None:
                return True
            if ty.name == "option" and len(ty.args) == 1:
                return walked(ty.args[0])
            if ty.name == "result" and len(ty.args) == 2:
                return walked(ty.args[0]) and walked(ty.args[1])
            return False

        in_params = set()
        for p in f.params:
            if not has_tp(p.type):
                continue
            if p.name == "result" or not walked(p.type):
                return False
            tps(p.type, in_params)
        return tps(f.ret, set()) <= in_params

    def add_struct_contracts(s, f):
        requires, ensures = s.struct_contracts(f)
        if requires:
            f.inv_requires = requires
        if ensures:
            f.inv_ensures = ensures

    def loop_frame_candidates(s, f):
        frame = {}
        n = 0

        def walk(stmts, env):
            # Match verify.walk_loops preorder and infer._scope_types visibility.
            nonlocal n
            for st in stmts:
                if isinstance(st, Let):
                    env[st.name] = st.type
                elif isinstance(st, While):
                    loop_no = n
                    n += 1
                    cands = []
                    for name in sorted(env):
                        ty = env[name]
                        if s.invariant_vec_elem(ty) is not None and push_only(st.body, name):
                            for clause in s.invariant_clauses(ty, name):
                                cid = "F%d#%d" % (loop_no, len(cands))
                                text = "invariant of %s (line %d) for %s" % clause.struct_invariant
                                cands.append((cid, clause, text))
                    if cands:
                        frame[loop_no] = cands
                    walk(st.body, dict(env))
                elif isinstance(st, ForMap):
                    walk(st.body, dict(env))
                elif isinstance(st, If):
                    walk(st.then, dict(env))
                    walk(st.els, dict(env))
                elif isinstance(st, Match):
                    for arm in st.arms:
                        walk(arm.body, {name: ty for name, ty in env.items() if name not in arm.binders})

        walk(f.body, {p.name: p.type for p in f.params})
        return frame

    def check_entries(s):
        """Host entry signatures admit plain values, array/str parameters and scalar Vec results."""
        def new_collection(ty):
            return (is_nested_collection_field(ty)
                    or ty.name == "hashmap" and ty.args[0] == STRING
                    or any(new_collection(a) for a in ty.args))

        rust_records = {d.name for d in s.decls.values()
                        if any(new_collection(fd.type) for _, fd in s.decl_fields(d))}
        while True:
            dependent = {d.name for d in s.decls.values()
                         if any(n in rust_records for _, fd in s.decl_fields(d) for n in dependencies(fd.type))}
            if dependent <= rust_records:
                break
            rust_records.update(dependent)
        # A Rust entry can borrow a record with the new collection fields, and a
        # typed resident state entry can also borrow its String key parameters.
        for f in s.prog.functions:
            if not f.host_entry:
                continue
            rust_state = bool(f.params and f.params[0].mode == "inout" and f.params[0].type.name in rust_records)
            positions = []
            for i, p in enumerate(f.params):
                d = s.invariant_struct(p.type)
                if d is not None:
                    terr(f, "entry_signature",
                         "host entry %s cannot take parameter %s of struct %s with an invariant" %
                         (getattr(f, "disp", f.name), p.name, d.name))
                if p.mode == "inout" and p.type in (I64, BOOL):
                    terr(f, "entry_signature", "host entry %s cannot take scalar inout parameter %s" % (f.name, p.name))
                if p.mode == "inout" and is_nongeneric_struct(p.type, s.decls):
                    if i == 0:
                        continue
                    terr(f, "entry_signature", "host entry %s must take its inout struct first" % f.name)
                if p.take and p.type == STRING:
                    continue
                if not p.take and p.mode is None and (p.type.name in rust_records or rust_state and p.type == STRING):
                    continue
                if p.mode is None and p.type.owner is None and not p.type.args and p.type.name in s.aggregates:
                    continue
                positions.append(("parameter %s" % p.name, p.type))
            if not is_scalar_vec(f.ret) and not s.is_result_record(f.ret) and not s.is_result_enum(f.ret):
                positions.append(("the result", f.ret))
            for what, ty in positions:
                if contains_collection(ty) or any(n in s.extended for n in dependencies(ty)):
                    terr(f, "entry_signature", "host entry %s cannot have the type %s in %s" % (
                        f.name, type_name(ty), what),
                         "keep it Pruv-callable (child `pub` or private) and expose a scalar/value wrapper at the root")

    def check_fn_sccs(s):
        names = [f.name for f in s.prog.functions]
        index = {n: i for i, n in enumerate(names)}
        adj = [[] for _ in names]
        for caller, _call, callee in s.call_log:
            i, j = index[caller.name], index[callee.name]
            if j not in adj[i]:
                adj[i].append(j)
        comps = [comp for comp in _sccs(len(names), adj) if len(comp) > 1 or comp[0] in adj[comp[0]]]
        # bucket the call log once per recursive SCC, keeping the original relative order
        scc_of = {i: k for k, comp in enumerate(comps) for i in comp}
        buckets = [[] for _ in comps]
        for c, call, callee in s.call_log:
            k = scc_of.get(index[c.name])
            if k is not None and scc_of.get(index[callee.name]) == k:
                buckets[k].append((index[c.name], call, callee))
        for comp, bucket in zip(comps, buckets):
            s.fn_sccs.append([names[i] for i in comp])
            for i in comp:
                f = s.prog.functions[i]
                if f.decreases is None:
                    cyc = [names[k] for k in _cycle_through(i, adj, comp)]
                    terr(f, "recursive_decreases", "recursive function %s needs a decreases clause (cycle: %s)" % (
                        f.name, " -> ".join(cyc)), "add `decreases <expr>` after requires/ensures, e.g. `decreases n`")
            edges = bucket
            edges.sort(key=lambda x: (x[0], x[1].line, x[1].col))
            for ci, call, callee in edges:
                caller = s.prog.functions[ci]
                ok = len(caller.tparams) == len(callee.tparams) and \
                    all(a.bound == b.bound for a, b in zip(caller.tparams, callee.tparams))
                if ok:
                    targs = call.targs if call.targs is not None else []
                    ok = len(targs) == len(caller.tparams) and all(
                        t.owner == fn_owner(caller.name) and t.index == k and not t.args
                        for k, t in enumerate(targs))
                if not ok:
                    terr(call, "recursive_polymorphism", "recursive call to %s must pass the type parameters <%s> "
                         "unchanged" % (callee.name, ", ".join(tp.name for tp in caller.tparams)),
                         "call with ::<T, ...> in the same order")

    def record_g9(s):
        """G9 metadata on the program and the type table, only when a G9 feature is present"""
        if not (s.type_sccs or s.extended or s.fn_sccs or any(f.decreases is not None for f in s.prog.functions)):
            return
        meta = {"type_sccs": [list(x) for x in s.type_sccs], "boxed": list(s.boxed),
                "extended": [d.name for d in s.prog.types if d.name in s.extended],
                "fn_sccs": [list(x) for x in s.fn_sccs]}
        if s.rec_vec_elems:
            meta["rec_vec_elems"] = sorted(s.rec_vec_elems)
        s.prog.g9 = meta
        s.tt.g9 = meta

    # ---- function ----
    def fn(s, f):
        s.f = f
        s.try_index = 0
        s.enter(fn_owner(f.name), f.tparams)
        s.calls[f.name] = set()
        s.add_struct_contracts(f)
        if f.foreign is not None:
            check_foreign_decl(f, s.externs)
            if f.decreases is not None:     # G9 A0: foreign termination is trusted
                terr(f.decreases, "recursive_decreases", "a trusted rust fn cannot have a decreases clause",
                     "remove it; foreign termination is trusted")
        elif f.name == "main" and (is_owned_value(f.ret) or any(is_owned_value(p.type) for p in f.params)):
            terr(f, "foreign_export", "main cannot have a collection parameter or result",
                 "give main plain value parameters and result; use collections inside it")
        scope = {}
        for p in f.params:
            if p.name in scope:
                terr(f, "dup_param", "duplicate parameter %s" % p.name)
            if p.type == UNIT:
                terr(f, "unit_param", "parameter %s cannot have type ()" % p.name)
            if p.take and not (is_array(p.type) or is_owned_value(p.type) or p.type.name in s.aggregates):
                terr(f, "foreign_move", "take is only for array, collection or aggregate parameters",
                     "remove `take` from %s; scalars and str are passed by value or borrowed" % p.name)
            # Whole assignment to inout parameters is checked separately from local mutability.
            scope[p.name] = (p.type, False, "inout" if p.mode is not None else "take" if p.take else "param")
        if f.pred:      # R03L (R-6-2): a pred body is one bool contract expression, never code
            for st in f.body:
                if isinstance(st, Return):
                    check_try_position(st.value, st)
            if (f.ret != BOOL or f.effects or f.requires or f.ensures or f.decreases is not None or f.tparams
                    or f.cost is not None or not (len(f.body) == 1 and isinstance(f.body[0], Return)
                                                  and f.body[0].value is not None)):
                terr(f, "pred_signature", "pred %s must return bool with no effects, contracts or type parameters "
                     "and a body of one `return <expr>;`" % f.name,
                     "write `pred fn %s(..) -> bool { return <contract expression>; }`" % f.name)
            s.ckind = "ensures"
            s.expect(f.body[0].value, BOOL, dict(scope), contract=True)
            s.ckind = "code"
            return
        s.used_effects = set()
        s.field_move = None
        s.ckind = "requires"
        for r in f.requires:
            check_try_position(r, r)
            s.expect(r, BOOL, scope, contract=True)
        escope = dict(scope)
        if f.ret != UNIT:
            escope["result"] = (f.ret, False, "result")
        s.ckind = "ensures"
        for e in f.ensures:
            check_try_position(e, e)
            s.expect(e, BOOL, escope, contract=True)
        s.ckind = "decreases"
        if f.decreases is not None:      # G9 A0: scalar measure over the parameters only (no result)
            check_try_position(f.decreases, f.decreases)
            s.expect(f.decreases, I64, scope, contract=True)
        s.ckind = "code"
        if f.foreign is not None:
            if is_collection(f.ret) and "alloc" not in f.effects:   # G6b: an owned result allocates
                terr(f, "missing_effect", "trusted rust fn %s returns %s and must declare `alloc`" % (
                    f.name, type_name(f.ret)),
                     "add `! %s` after the return type" % ", ".join(sorted(set(f.effects) | {"alloc"})),
                     stage="effect")
            return                       # a trusted rust fn has no body to check
        s.block(f.body, dict(scope))
        if s.prog.borrow_site is not None:
            f.borrowed_roots = set()
            s.check_view_block(f.body, {p.name: {p.name: False} for p in f.params if s.borrowed_type(p.type)})
        if f.ret != UNIT and not s.ends_in_return(f.body):
            terr(f, "missing_return", "function %s may reach its end without returning" % f.name,
                 "end every path with `return ...;`")
        for e in sorted(s.used_effects):
            if e not in f.effects:
                terr(f, "missing_effect", "function %s uses effect `%s` but does not declare it" % (f.name, e),
                     "add `! %s` after the return type" % ", ".join(sorted(set(f.effects) | s.used_effects)),
                     stage="effect")
        check_ownership(f, s.funcs, s.decls, s.aggregates | s.rec_vec_elems)
        f.frame_cands = s.loop_frame_candidates(f)

    def ends_in_return(s, stmts):
        if not stmts:
            return False
        last = stmts[-1]
        if isinstance(last, Return):
            return True
        if isinstance(last, If):
            return s.ends_in_return(last.then) and s.ends_in_return(last.els)
        if isinstance(last, Match):
            return all(s.ends_in_return(a.body) for a in last.arms)
        return False

    # ---- statements ----
    def block(s, stmts, scope):
        i = 0
        while i < len(stmts):
            st = stmts[i]
            if isinstance(st, Assign) and not st.deref and any(is_try(x) for x in _walk(st.value)):
                # G-LG13-6: the target passed inout inside a statement with a `?`
                for arg in _walk(st.value):
                    root = field_place(arg.expr)[0] if isinstance(arg, InoutArg) else None
                    if isinstance(root, Var) and root.name == st.name:
                        terr(arg, "inout_context",
                             "%s is passed inout and also mentioned elsewhere in this statement" % st.name,
                             "use a separate statement for the inout call")
            site = try_site(st, scope)
            if site is not None:
                candidate, temps = site
                if isinstance(candidate, Binary):   # LG15: checked on the statement before its temporaries
                    s.try_short_check(st, candidate, scope)
                # LG21 (G-LG20-3): the split and effect checks run on every lowered statement, not only on one
                # with earlier operands to bind
                s.try_checks(st, temps, scope)
                if temps:                       # LG14 (G-LG13-1): bind the earlier operands first
                    stmts[i:i] = s.try_temps(st, temps, scope)
                    continue
                if isinstance(candidate, Binary):   # LG15 (G-LG13-2): the operator becomes a nested if
                    stmts[i:i] = s.try_short(st, candidate, scope)
                    continue
                match = Match(st.line, st.col, candidate.operand, [])
                if hasattr(st, "file"):
                    match.file = st.file
                s.stmt(match, scope, None, try_tail=(st, stmts[i + 1:], candidate))
                stmts[i:] = [match]
                return
            s.stmt(st, scope, stmts[i + 1] if i + 1 < len(stmts) else None)
            i += 1

    def try_temps(s, st, temps, scope):
        """LG14 (G-LG13-1): `let __pruv_try_tN: T = k;` for each operand k evaluated before the statement's first `?`,
        in evaluation order, with k replaced by the fresh local in st; T is a value type or, per LG18 (G-LG14-1), an owned call
        result or move that find_try admitted as a direct user-call argument"""
        lets = []
        inout = any(isinstance(x, InoutArg) for v in vars(st).values() if isinstance(v, Expr) for x in _walk(v))
        for parent, k in temps:
            typed = copy.deepcopy(k)
            t = s.expr(typed, dict(scope))
            if t == UNIT:
                try_position_error(st)
            if inout:
                s.try_effect_check(st, k)
            take = False
            if not is_value(t, s.value_decls):
                take = s.try_owned_temp(st, parent, k, t, typed)
            elif not all(s.try_plain_arg(c) for c in expr_kids(typed) if isinstance(c, Expr)):
                # LG20 (G-LG19-1): a value temporary's own operands follow the same rule, or the `let` would drop
                # an owned value built inside it (`lend(mk(2), 0)`) before the `?` operand runs
                try_position_error(st)
            name = "%s%d" % (TRY_TEMP, s.try_index)
            s.try_index += 1
            let = Let(st.line, st.col, name, False, t, k)
            var = Move(st.line, st.col, name) if take else Var(st.line, st.col, name)
            replace_expr(st, k, var)
            for node in (let, var):
                if hasattr(st, "file"):
                    node.file = st.file
            lets.append(let)
        return lets

    def try_checks(s, st, temps, scope):
        """LG18 (G-LG14-2) checks of a statement find_try lowers, with or without temporaries (LG21, G-LG20-3): the
        split check over the temporaries and every `?` operand, and with inout the effect check over every `?` operand"""
        roots = [v for v in vars(st).values() if isinstance(v, Expr)]
        inout = any(isinstance(x, InoutArg) for v in roots for x in _walk(v))
        tries = [x.operand for v in roots for x in _walk(v) if is_try(x)]
        s.try_split_check(st, roots, [k for _, k in temps] + tries, scope, bool(temps))
        for q in tries if inout else []:
            s.try_effect_check(st, q)

    def try_effect_check(s, st, k):
        """LG18 (G-LG14-2 a): in a statement with inout, a temporary or `?` operand calling print or a function with
        an effect other than alloc is type/try_position (unlowered, inout_context's effect-before rule rejects it)"""
        for x in _walk(k):
            if isinstance(x, Call):
                callee = s.funcs.get(x.name) or CATALOG.get(x.name)
                if x.name == "print" or (callee is not None and any(fx != "alloc" for fx in callee.effects)):
                    try_position_error(st)

    def try_split_check(s, st, roots, parts, scope, temps=True):
        """LG18 (G-LG14-2 b): type/try_position when a part (a temporary or a `?` operand) mentions a binding whose
        type is not a value type and a call holding the part also mentions it outside the part, except as a direct
        argument of a borrowed parameter of a user function. Nested calls share the outermost call's borrow track and
        lowering moves the part out of that call, so a borrow in it and a move (explicit, or implicit through Err or a
        constructor argument) of the same name in the call would no longer meet (foreign_borrow unlowered)"""
        parent = {}
        for r in roots:
            for x in _walk(r):
                for i, c in enumerate(expr_kids(x)):
                    parent[id(c)] = (x, i)
        for part in parts:
            inside = {id(x) for x in _walk(part)}
            names = {x.name for x in _walk(part) if isinstance(x, (Var, Move)) and x.name in scope
                     and not is_value(scope[x.name][0], s.value_decls)}
            for r in roots:
                for c in _walk(r):
                    if not isinstance(c, Call) or id(part) not in {id(y) for y in _walk(c)}:
                        continue
                    for x in _walk(c):
                        if id(x) in inside or not isinstance(x, (Var, Move)) or x.name not in names:
                            continue
                        p, i = parent.get(id(x), (None, None))
                        f = s.funcs.get(p.name) if isinstance(x, Var) and isinstance(p, Call) else None
                        if f is None and isinstance(x, Var) and isinstance(p, Call) and not temps:
                            # LG21 (G-LG20-3): with nothing bound, a builtin's borrowed argument (`len(s)`) also
                            # only reads the name after the `?` operand ran; string_to_str consumes a String error
                            # binder and is not a read
                            cf = CATALOG.get(p.name)
                            if p.name != "string_to_str" and (cf is None or i < len(cf.params)
                                                              and not cf.params[i].take):
                                continue
                        if not (f is not None and f.foreign is None and i < len(f.params) and not f.params[i].take):
                            try_position_error(st)

    def try_owned_temp(s, st, parent, k, t, typed):
        """LG18 (G-LG14-1): an operand of an owned type (Vec, HashMap, String) evaluated before a `?` may be bound to
        a temporary only when it is a call result (not subview) or a plain move passed directly as an argument of a
        non-foreign user function to a take parameter of exactly its type: the temporary is then moved into the same
        call, so it is consumed where the value was (a borrowed parameter would drop it at the end of the block, not
        the statement, and a coercion such as Vec to [T] applies to places only). Returns True; any other operand of a
        type that is not a value is type/try_position"""
        f = s.funcs.get(parent.name) if isinstance(parent, Call) else None
        j = next((i for i, a in enumerate(parent.args) if a is k), None) if f is not None else None
        ok = (f is not None and f.foreign is None and j is not None and j < len(f.params) and f.params[j].take
              and f.params[j].type == t and t.owner is None and t.name in ("vec", "hashmap", "string")
              and (isinstance(k, Call) and k.name != "subview" or isinstance(k, Move) and not k.fields))
        # LG19 (G-LG18-4): a lifted call result is bound only when its own arguments create no owned temporary: the
        # `let` would drop a value the call borrowed at its end, before the `?` operand runs, instead of at the end of
        # the statement
        if not ok or isinstance(k, Call) and not all(s.try_plain_arg(a) for a in typed.args):
            try_position_error(st)
        return True

    def try_plain_arg(s, a):
        """LG19 (G-LG18-4): a is a move, an int / bool literal, a place (a binding or a field of one), or an operator
        or index whose operands are all plain in this sense, so evaluating it creates no temporary that needs a drop.
        LG20 (G-LG19-3): or any other node whose type is a value type (a call such as `len(s)`) with plain operands.
        a is a node of the temporary's typed copy, so its type is the one the temporary's own check gave (no second
        type check); a string literal is not plain (str is not a value type; the generated code may build a String)"""
        if isinstance(a, (Move, IntLit, BoolLit)) or isinstance(field_place(a)[0], Var):
            return True
        if getattr(a, "op", "") == "?":
            return False
        if isinstance(a, (Unary, Binary, Index)) or (getattr(a, "ty", None) is not None
                                                     and is_value(a.ty, s.value_decls)):
            return all(s.try_plain_arg(c) for c in expr_kids(a) if isinstance(c, Expr))
        return False

    def try_short_check(s, st, node, scope):
        """LG15: type/try_position when the operator node mentions a binding whose declared type is not a value type
        and st mentions it again. Called on the statement before its LG14 temporaries are bound (a temporary would
        hide a borrow in an earlier operand) and again before the lowering"""
        root = st.cond if isinstance(st, If) else st.expr if isinstance(st, ExprStmt) else st.value
        for m in _walk(node):
            if isinstance(m, (Var, Move)) and m.name in scope and _mentions_count(root, m.name) > 1:
                if not is_value(scope[m.name][0], s.value_decls):
                    try_position_error(st)

    def try_short(s, st, node, scope):
        """LG15 (G-LG13-2): `let mut __pruv_try_tN: bool = D; if C { __pruv_try_tN = R; }` for the operator node
        `L op R` whose right operand R holds the statement's first `?`, with node replaced by the fresh local in st:
        D, C = false, L for &&; true, !L for ||. The assignment is a lowered position, so the `?` in R is lowered there
        and runs only when R runs, as in the hand-written nested if. A binding of a type that is not a value type
        mentioned in the operator and mentioned again in the statement is type/try_position: splitting the statement
        would hide a borrow or an implicit move between the two mentions"""
        s.try_short_check(st, node, scope)
        pos = (st.line, st.col)
        name = "%s%d" % (TRY_TEMP, s.try_index)
        s.try_index += 1
        let = Let(*pos, name, True, BOOL, BoolLit(*pos, node.op != "&&"))
        cond = Unary(*pos, "!", node.left) if node.op == "||" else node.left
        assign = Assign(*pos, name, node.right)
        branch = If(*pos, cond, [assign], [])
        var = Var(*pos, name)
        replace_expr(st, node, var)
        if hasattr(st, "file"):
            for x in (let, let.value, cond, assign, branch, var):
                x.file = st.file
        return [let, branch]

    def try_arms(s, st, rest, target, operand):
        if operand.owner is not None or operand.name != "result":
            terr(st, "try_operand", "`?` needs a Result operand, got %s" % type_name(operand),
                 "use `?` only on a Result value")
        ret = s.f.ret
        if ret.owner is not None or ret.name != "result":
            terr(st, "try_return", "`?` needs the enclosing function to return a Result",
                 "return a Result or use an explicit match")
        error = result_error(ret)
        if result_error(operand) != error:
            terr(st, "try_error_type", "`?` error type %s does not match the function error type %s"
                 % (type_name(result_error(operand) or I64), type_name(error or I64)),
                 "use an explicit match and convert the error")

        pos = (st.line, st.col)
        index = s.try_index
        s.try_index += 1
        ok_name = "__pruv_try_v%d" % index
        err_name = "__pruv_try_e%d" % index
        err_value = Var(*pos, err_name)
        err = Ctor(*pos, "Err", err_value)
        err.targs = [ret.args[0] if ret.args else I64]
        if error != I64:
            err.targs.append(error)
        early = Return(*pos, err)
        bare = isinstance(st, ExprStmt) and st.expr is target
        unit = bool(operand.args) and operand.args[0] == UNIT
        if unit and not bare:   # LB8 R-6g7-1: the unit value of `e?` is only discarded
            try_position_error(st)
        ok = MatchArm(*pos, None, "Ok", [] if unit else [ok_name], rest)
        fail = MatchArm(*pos, None, "Err", [err_name], [early])
        nodes = [ok, fail, early, err, err_value]
        if not bare:
            value = Var(*pos, ok_name)
            replace_expr(st, target, value)
            ok.body.insert(0, st)
            nodes.append(value)
        if hasattr(st, "file"):
            for node in nodes:
                node.file = st.file
        return [ok, fail]

    def check_inout_context(s, st, *exprs):
        """R-6g-1: check the typed expressions of one statement, without entering its blocks."""
        if not s.uses_inout:
            return
        mentions = []
        if isinstance(st, (Assign, IndexAssign, FieldAssign)):
            mentions.append((st.name, tuple(st.fields) if isinstance(st, (FieldAssign, IndexAssign)) else ()))
        calls = []

        def visit(e, conditional):
            short_circuit = isinstance(e, Binary) and e.op in ("&&", "||")
            for k in expr_kids(e):
                visit(k, conditional or (short_circuit and k is e.right))
            if isinstance(e, Call):
                calls.append((e, conditional))

        for e in exprs:
            mentions.extend(_place_mentions(e))
            visit(e, isinstance(st, While))
        effect_before = False
        for e, conditional in calls:
            f = s.funcs.get(e.name)
            marked = ([arg for arg in e.args if isinstance(arg, InoutArg)]
                      if f is not None or e.name in ARRAY_GROW_BUILTINS +
                      ("map_remove", "map_take", "vec_clear", "vec_reserve", "vec_extend_from_slice", "ev_read_into", "ev_proc_read_into") else [])
            if marked:
                for arg in marked:
                    root, path = field_place(arg.expr)
                    if sum(name == root.name and _paths_overlap(path, other)
                           for name, other in mentions) > 1:
                        terr(arg, "inout_context",
                             "%s is passed inout and also mentioned elsewhere in this statement" % root.name,
                             "use a separate statement for the inout call")
                if conditional:
                    terr(e, "inout_context", "the inout call may be skipped or repeated",
                         "use a separate statement where the call must run once")
                if isinstance(st, (IndexAssign, FieldAssign)):
                    terr(e, "inout_context", "an inout call is not allowed in an element or field assignment",
                         "compute the call's result before the assignment")
                if effect_before:
                    terr(e, "inout_context", "an inout call is not allowed here because a call with effects runs before it",
                         "evaluate the earlier call in a separate statement")
            callee = f if f is not None else CATALOG.get(e.name)
            if e.name == "print" or (callee is not None and any(effect != "alloc" for effect in callee.effects)):
                effect_before = True

    def stmt(s, st, scope, next_stmt, try_tail=None):
        for value in vars(st).values():
            if isinstance(value, Expr):
                check_try_position(value, st)
            elif isinstance(value, list):
                for item in value:
                    if isinstance(item, Expr):
                        check_try_position(item, st)
        f = s.f
        s.ckind = "code"
        s.boundary = None
        s.statement_call = st.expr if isinstance(st, ExprStmt) else None
        s.field_move = None
        if isinstance(st, Return) and st.value is not None:
            if isinstance(st.value, Call):     # LG9 (G-LG8-1): `return f(move d.a);` with a take parameter
                s.field_move = s.field_move_arg(st.value, scope, None)
            for e in _field_moves(st.value):
                if e is s.field_move:
                    continue
                s.check_borrow(e, e.name, e.fields)
                s.lookup(e, e.name, scope, e.fields)
                _field_move_error(e)
        if s.map_iterations and isinstance(st, Return):
            terr(st, "foreign_contract", "cannot return inside a map iteration")
        if s.map_borrows and isinstance(st, Return):
            terr(st, "foreign_contract", "cannot return inside a mutable collection Some arm")
        if isinstance(st, Let):
            if st.name in scope:
                terr(st, "shadow", "%s is already defined; shadowing is not allowed" % st.name,
                     "use a new name")
            if st.type == UNIT:
                terr(st, "unit_let", "cannot bind a () value")
            s.check_type(st, st.type)
            s.ext_misuse(st, st.type)
            s.boundary = st.value
            s.alloc_hint_for(st.value, st.type)
            s.field_move = s.field_move_out(st, st.type, scope, next_stmt)
            if st.ref:
                s.view_value(st.value, st.type, scope)
            else:
                s.expect(st.value, st.type, scope)
            s.check_inout_context(st, st.value)
            if not st.ref:
                s.own(st.value)
                if is_array(st.type) or st.type == STR:
                    s.array_source(st, st.value, scope)
            scope[st.name] = (st.type, st.mutable, "ref" if st.ref else "local")
        elif isinstance(st, Assign):
            v = s.lookup(st, st.name, scope)
            s.check_borrow(st, st.name, [])
            scalar_binder = v[2] == "map_mut" and v[0] in (I64, BOOL)
            record_binder = (v[2] == "map_mut" and v[0].owner is None and not v[0].args
                             and (v[0].name in s.aggregates or s.rec_vec_elem(v[0])))
            scalar_inout = v[2] == "inout" and v[0] in (I64, BOOL)
            array_inout = v[2] == "inout" and is_array(v[0])
            if st.deref and not (scalar_binder or record_binder):
                terr(st, "type_mismatch", "dereference requires a mutable scalar or record collection binder")
            if not (v[1] or scalar_binder or record_binder or scalar_inout or array_inout):
                if v[2] == "inout":
                    terr(st, "immutable", "inout parameter %s cannot be reassigned as a whole" % st.name,
                         "store elements with %s[i] = v, or copy into a `let mut` local" % st.name)
                terr(st, "immutable", "%s is not mutable" % st.name,
                     "declare it with `let mut`" if v[2] == "local" else "parameters are immutable; copy into a `let mut` local")
            if record_binder and (_mentions(st.value, st.name)
                                  or any(st.name in binders and _mentions(st.value, root)
                                         for root, _, binders in s.map_borrows)):
                terr(st.value, "recursive_borrow", "a record replacement cannot mention its binder or a borrowed root")
            s.boundary = st.value
            s.alloc_hint_for(st.value, v[0])
            s.restore_array = st.name if array_inout else None
            s.expect(st.value, v[0], scope)
            if (array_inout and st.name in (f.borrow_from or ()) and isinstance(st.value, Call)
                    and st.value.name == "resize" and isinstance(st.value.args[0], (Var, Move))
                    and st.value.args[0].name == st.name and not getattr(st.value.args[0], "fields", ())):
                st.value.resize_scratch = True
            s.restore_array = None
            s.own(st.value)
            if record_binder and not (
                    isinstance(st.value, (StructLit, EnumCtor, Call))
                    or isinstance(st.value, Move) and not st.value.fields
                    and scope[st.value.name][2] in ("local", "take")):
                terr(st.value, "foreign_move", "a record replacement needs a constructor, call result or move")
            s.check_inout_context(st, st.value)
            if is_array(v[0]) or v[0] == STR:
                s.array_source(st, st.value, scope)
        elif isinstance(st, IndexAssign) and st.fields:
            # LG4: `p.a.b[i] = e` -- the place is the field path of a mutable local / inout parameter
            fpath = tuple(st.fields)
            v = s.lookup(st, st.name, scope, fpath)
            s.check_borrow(st, st.name, fpath)
            cur = v[0]
            for fname in fpath:
                if cur.owner is not None:
                    terr(st, "generic_operation", "cannot assign field %s: %s is an abstract type parameter" % (
                        fname, type_name(cur)), "a type parameter has no fields")
                if not s.tt.is_struct(cur):
                    terr(st, "not_struct", "cannot assign field %s: %s is not a struct" % (fname, type_name(cur)),
                         "field assignment paths run through structs only; assign the whole enum value instead")
                if s.ref_field(cur, fname):
                    terr(st, "immutable", "a borrowed view is read-only")
                cur = s.field_type(st, cur, fname)
            place = ".".join([st.name] + list(fpath))
            if cur == STR:
                terr(st, "immutable_string", "%s is a str; its bytes cannot be changed" % place,
                     "build a new str with substr() or copy() instead")
            if cur not in (ARRAY, ARRAY_U8, ARRAY_BOOL):
                terr(st, "not_array", "%s is not an array" % place)
            if not v[1] and v[2] not in ("inout", "map_mut"):
                terr(st, "immutable", "%s is not mutable" % st.name,
                     "declare it with `let mut`" if v[2] == "local" else "parameters are immutable; copy into a `let mut` local")
            s.expect(st.index, I64, scope)
            s.expect(st.value, s.elem_type(cur), scope)
            if getattr(st.value, "map_iter_view", False):
                s.own(st.value)
            s.check_inout_context(st, st.index, st.value)
        elif isinstance(st, IndexAssign):
            s.check_iteration_borrow(st, st.name, ())
            v = s.lookup(st, st.name, scope)
            if v[2] == "ref":
                terr(st, "immutable", "a borrowed view is read-only")
            if v[0] == STR:
                terr(st, "immutable_string", "%s is a str; its bytes cannot be changed" % st.name,
                     "build a new str with substr() or copy() instead")
            if not is_array(v[0]):
                terr(st, "not_array", "%s is not an array" % st.name)
            if not v[1] and v[2] != "inout":   # G5 A0: an inout parameter permits element stores
                terr(st, "immutable", "array %s is read-only" % st.name,
                     "use `let mut b: %s = copy(%s);` (needs alloc)" % (type_name(v[0]), st.name))
            s.expect(st.index, I64, scope)
            # G2 A5: stored value is bool for a bool array; G3: exactly T for a [T]
            s.expect(st.value, s.elem_type(v[0]), scope)
            if getattr(st.value, "map_iter_view", False):
                s.own(st.value)
            s.check_inout_context(st, st.index, st.value)
        elif isinstance(st, FieldAssign):
            v = s.lookup(st, st.name, scope, tuple(st.fields))
            s.check_borrow(st, st.name, st.fields)
            cur = v[0]
            for fname in st.fields:
                if cur.owner is not None:
                    terr(st, "generic_operation", "cannot assign field %s: %s is an abstract type parameter" % (
                        fname, type_name(cur)), "a type parameter has no fields")
                if not s.tt.is_struct(cur):
                    terr(st, "not_struct", "cannot assign field %s: %s is not a struct" % (fname, type_name(cur)),
                         "field assignment paths run through structs only; assign the whole enum value instead")
                cur = s.field_type(st, cur, fname)
            s.field_move = s.field_move_out(st, cur, scope, next_stmt) or s.field_update_move(st, cur, v, scope)
            if not v[1] and v[2] not in ("inout", "map_mut"):
                terr(st, "immutable", "%s is not mutable" % st.name,
                     "declare it with `let mut`" if v[2] == "local" else "parameters are immutable; copy into a `let mut` local")
            s.alloc_hint_for(st.value, cur)
            s.expect(st.value, cur, scope)
            s.check_inout_context(st, st.value)
            s.own(st.value)
            if is_array(cur) or cur == STR:      # G9 A0: G2 explicit ownership for a buffer field
                s.array_source(st, st.value, scope)
        elif isinstance(st, If):
            s.expect(st.cond, BOOL, scope, op="a condition")
            s.check_inout_context(st, st.cond)
            s.block(st.then, dict(scope))
            s.block(st.els, dict(scope))
        elif isinstance(st, While):
            s.expect(st.cond, BOOL, scope, op="a condition")
            s.check_inout_context(st, st.cond)
            s.ckind = "invariant"
            for i in st.invariants:
                s.expect(i, BOOL, scope, contract=True)
            s.ckind = "decreases"
            s.expect(st.decreases, I64, scope, contract=True)
            s.ckind = "code"
            s.block(st.body, dict(scope))
        elif isinstance(st, ForMap):
            s.for_map(st, scope)
        elif isinstance(st, Return):
            if st.value is None:
                if f.ret != UNIT:
                    terr(st, "return_type", "function returns %s; `return;` is not enough" % type_name(f.ret))
            else:
                if f.ret == UNIT:
                    terr(st, "return_type", "function returns (); use `return;`")
                s.boundary = st.value
                s.alloc_hint_for(st.value, f.ret)
                if f.ref_result:
                    s.view_value(st.value, f.ret, scope)
                else:
                    s.expect(st.value, f.ret, scope)
                s.check_inout_context(st, st.value)
                # LG9 (G-DB15-2): a str binder of a consuming match is returned like an owned local
                binder_ret = (f.ret == STR and isinstance(st.value, Var) and st.value.name in s.match_movable
                              and st.value.ty == STR)
                if not f.ref_result:
                    s.own(st.value)
                    if (is_array(f.ret) or f.ret == STR) and not binder_ret:
                        s.array_source(st, st.value, scope, allow_local_var=True)
                if binder_ret:
                    s.used_effects.add("alloc")
                if binder_ret:
                    # it means `return copy(x)`: rewrite it so both backends and the cost proof see the copy
                    st.value = Call(st.value.line, st.value.col, "copy", [st.value], STR)
        elif isinstance(st, Assert) and st.reveal is not None:     # P13o: `reveal F;` names an opaque function
            rf = s.funcs.get(st.reveal.name)
            if rf is None or not rf.opaque:
                terr(st.reveal, "reveal_target", "reveal %s: %s" % (
                    st.reveal.name, "unknown function" if rf is None else "the function is not opaque"),
                     "reveal names an `opaque fn`; a function without `opaque` is always transparent")
        elif isinstance(st, Assert):
            s.ckind = "assert"
            s.expect(st.cond, BOOL, scope, contract=True)
            s.ckind = "code"
        elif isinstance(st, ExprStmt):
            if not isinstance(st.expr, Call):
                for e in _field_moves(st.expr):
                    _field_move_error(e)
                terr(st, "useless_expr", "expression statement must be a call", "bind it with let or remove it")
            s.boundary = st.expr
            s.field_move = s.field_move_out(st, None, scope, next_stmt)   # LG9 (G-LG8-1): `f(move d.a);`
            t = s.expr(st.expr, scope)
            s.check_inout_context(st, st.expr)
            if t.owner is None and t.name == "process":   # G10 proc: a discarded Process result keeps its duty
                terr(st, "proc_unconsumed", "temporary Process is not consumed",
                     "consume it with proc_wait(move p) or proc_kill(move p), or return / transfer ownership")
            if t.owner is None and t.name == "socket":    # G10 net A0: a discarded Socket / Listener keeps its duty
                terr(st, "net_unconsumed", "temporary Socket is not consumed",
                     "consume it with net_close(move s) or net_listener_close(move l), or return / transfer ownership")
            if t.owner is None and t.name == "loop":
                terr(st, "async_unconsumed", "temporary Loop is not consumed",
                     "consume it with ev_close(move l), or return / transfer ownership")
            if t.owner is None and t.name == "listener":
                terr(st, "net_unconsumed", "temporary Listener is not consumed",
                     "consume it with net_close(move s) or net_listener_close(move l), or return / transfer ownership")
            if t != UNIT:
                terr(st, "unused_value", "value of type %s is dropped" % type_name(t),
                     "bind it with `let` or `match` it")
        elif isinstance(st, Par):
            s.par(st, scope)
        elif isinstance(st, Match):
            s.match_call = st.scrutinee
            t = s.expr(st.scrutinee, scope)
            s.match_call = None
            if try_tail is not None:
                st.arms = s.try_arms(*try_tail, t)
            call = st.scrutinee
            catalog_match = isinstance(call, Call) and call.name not in s.funcs
            mutable_map = catalog_match and call.name in MUTABLE_COLLECTION_MATCH
            map_take = catalog_match and call.name == "map_take"
            map_read = catalog_match and call.name == "map_get"
            nested_map_path = None
            if map_read and is_string_record_map(call.args[0].ty):
                base, path = field_place(call.args[0])
                if path and isinstance(base, Var) and scope[base.name][2] == "map_mut":
                    nested_map_path = (base.name, path)
            map_view = map_read and (s.is_extended(call.targs[-1]) or nested_map_path is not None)
            s.check_inout_context(st, st.scrutinee)
            user = s.tt.is_enum(t)
            mpath = s.match_inout_path(call, t, scope)
            vec_read = field_place(call)[0]
            if s.rec_vec_get(vec_read):
                base = field_place(vec_read.args[0])[0]
                while s.rec_vec_get(base):  # a nested borrowed element read borrows the outermost vector
                    base = field_place(base.args[0])[0]
                root = base.name if isinstance(base, Var) else None
            else:
                root = None if mpath is not None else s.match_root(call, t, scope)
            if t.owner is not None:
                terr(st, "generic_operation", "cannot match a value of the abstract type %s" % t.name,
                     "a type parameter has no variants; match on an Option, Result or enum that contains it")
            if t.name == "option":
                need = {"Some", "None"}
            elif t.name == "result":
                need = {"Ok", "Err"}
            elif user:
                need = [v.name for v in s.tt.decl(t).variants]
            else:
                terr(st, "match_type", "can only match Option<i64>, Result<i64> or a user enum")
            if not user:
                got = [a.ctor for a in st.arms]
                if set(got) != need or len(got) != 2 or any(a.enum_name is not None for a in st.arms):
                    terr(st, "foreign_contract" if mutable_map else "match_arms", "match needs exactly arms %s" % " and ".join(sorted(need)))
            else:
                seen = set()
                for a in st.arms:
                    if a.enum_name != t.name:
                        terr(a, "match_arms", "arm %s does not belong to enum %s" % (
                            ("%s::%s" % (a.enum_name, a.ctor)) if a.enum_name else a.ctor, t.name),
                             "match arms are written %s::Variant" % t.name)
                    if a.ctor not in need:
                        terr(a, "unknown_variant", "enum %s has no variant %s" % (t.name, a.ctor))
                    if a.ctor in seen:
                        terr(a, "match_arms", "arm %s::%s appears twice" % (t.name, a.ctor))
                    seen.add(a.ctor)
                missing = [n for n in need if n not in seen]
                if missing:
                    terr(st, "match_arms", "match is missing arm(s): %s" % ", ".join(
                        "%s::%s" % (t.name, n) for n in missing))
            # G-DB16-1: an array binder may be moved when the scrutinee is a plain owned local enum value that
            # no arm names again; the move then consumes the scrutinee (foreign.py marks it moved)
            # LG36 (G-LG35-1): not while an enclosing match arm still views the same owner (active_roots): the
            # outer binder reads the payload the move would hand over
            movable_scrut = False
            if (user and not t.args and isinstance(st.scrutinee, Var) and st.scrutinee.name not in s.arm_binders
                    and st.scrutinee.name not in s.active_roots):
                skind = scope.get(st.scrutinee.name, (None, None, None))[2]
                if skind == "local":
                    named = set()
                    for a in st.arms:
                        _stmt_names(a.body, named)
                    movable_scrut = st.scrutinee.name not in named
            for a in st.arms:
                sc = dict(scope)
                node = a if user else st
                saved_movable, saved_binders = s.match_movable, s.arm_binders
                s.arm_binders = saved_binders | set(a.binders)
                if user:
                    ftypes = [ft for _, ft in s.fields_of(t, a.ctor)]
                    if len(a.binders) != len(ftypes):
                        terr(a, "match_arity", "%s::%s has %d field(s), the pattern binds %d" % (
                            t.name, a.ctor, len(ftypes), len(a.binders)),
                             "bind every field, in declaration order")
                else:
                    payload = result_error(t) if a.ctor == "Err" else t.args[0] if t.args else I64
                    if a.ctor == "Ok" and (payload == UNIT) != (not a.binders):   # LB8 R-6g7-1
                        terr(a, "match_arity", "Ok of %s %s" % (type_name(t), "binds no value, write `Ok(())`"
                             if payload == UNIT else "carries a value, the pattern must bind it"),
                             "write `Ok(())` for Result<(), E> and `Ok(v)` otherwise")
                    ftypes = [payload] * len(a.binders)
                for b, bt in zip(a.binders, ftypes):
                    if b in sc:
                        terr(node, "shadow", "%s is already defined" % b, "use a new name")
                    # G9 A0: a buffer payload is a read view (like a parameter): returning it needs copy()
                    kind = "param" if (is_array(bt) or bt == STR or s.rec_vec(bt)) else "local"
                    if a.ctor == "Err" and not user and bt == STRING:
                        kind = "result_error"
                    sc[b] = (bt, map_take and a.ctor == "Some", kind)
                if movable_scrut:
                    s.match_movable = saved_movable | {b for b, bt in zip(a.binders, ftypes) if is_array(bt)}
                    # LG9 (G-DB15-2): `return x` of a str binder of a consuming match moves it (see Return)
                    s.match_movable = s.match_movable | {b for b, bt in zip(a.binders, ftypes) if bt == STR}
                    # LB9 (R-6g8-4): `let t = move b.f;` of a struct binder consumes the scrutinee like a binder move
                    s.match_movable = s.match_movable | {b for b, bt in zip(a.binders, ftypes) if s.tt.is_struct(bt)}
                if root is not None:
                    s.active_roots.append(root)
                if mpath is not None:
                    s.match_paths.append(mpath)
                read_path = None
                if a.ctor == "Some" and map_read:
                    rbase, rpath = field_place(call.args[0])
                    if isinstance(rbase, Var):
                        read_path = (rbase.name, tuple(rpath))
                        s.map_read_paths.append(read_path)
                if a.ctor == "Some" and (mutable_map or map_view):
                    operand = st.scrutinee.args[0]
                    base, path = field_place(operand.expr if mutable_map else operand)
                    borrows = s.map_borrows if mutable_map else s.match_paths if nested_map_path else s.active_roots
                    for b, bt in zip(a.binders, ftypes):
                        sc[b] = (bt, False, "map_mut" if mutable_map else "param")
                    read_root = not mutable_map and not nested_map_path and isinstance(base, Var)
                    if isinstance(base, Var):
                        borrows.append((base.name, path, a.binders) if mutable_map
                                       else nested_map_path or base.name)
                    if read_root:
                        s.map_read_roots.append(base.name)
                    s.block(a.body, sc)
                    if read_root:
                        s.map_read_roots.pop()
                    if isinstance(base, Var):
                        borrows.pop()
                else:
                    s.block(a.body, sc)
                if read_path is not None:
                    s.map_read_paths.pop()
                if root is not None:
                    s.active_roots.pop()
                if mpath is not None:
                    s.match_paths.pop()
                s.match_movable, s.arm_binders = saved_movable, saved_binders
        else:
            terr(st, "internal", "unknown statement")

    def for_map(s, st, scope):
        if s.f.cost is not None:
            terr(st, "foreign_contract", "map iteration is not allowed in a function with a cost clause")
        root, path = field_place(st.place)
        if not isinstance(root, Var):
            terr(st.place, "foreign_contract", "map iteration requires a binding or field path")
        ty = s.expr(st.place, scope)
        if (ty.owner is not None or ty.name != "hashmap" or ty.args[0] != STRING
                or (ty.args[1] not in (I64, BOOL) and
                    (ty.args[1].owner is not None or ty.args[1].args or ty.args[1].name not in s.aggregates))):
            terr(st.place, "foreign_owned_type", "map iteration requires HashMap<String, V> with scalar or owned aggregate values")
        s.ckind = "invariant"
        for inv in st.invariants:
            s.expect(inv, BOOL, scope, contract=True)
        s.ckind = "code"
        inner = dict(scope)
        for name, t in ((st.key, STRING), (st.val, ty.args[1])):
            if name in inner:
                terr(st, "shadow", "%s is already defined" % name, "use a new name")
            inner[name] = (t, False, "map_iter")
        s.map_iterations.append((root.name, path))
        s.block(st.body, inner)
        s.map_iterations.pop()

    # ---- G7 A0: par groups (G7_SPEC 3, 4, 7) ----
    def par(s, st, scope):
        """one par statement: arity, direct targets, ordinary typing of every branch (G5 rules included), the restricted
        argument form, then cross-branch overlap, in source order. The transitive eligibility summary (effects, foreign
        leaves, nesting) needs every function typed, so it runs later in check_par_eligibility."""
        calls = st.calls
        if len(calls) < 2:
            terr(st, "parallel_arity", "a par group needs at least two branch calls, found %d" % len(calls),
                 "write a single ordinary call instead of a par group")
        for c in calls:
            s.par_target(c)
        s.in_par = True
        for c in calls:
            s.ckind = "code"
            s.boundary = c
            s.expr(c, scope)
        s.in_par = False
        s.boundary = None
        for c in calls:
            s.par_args(c)
        s.par_overlap(calls)
        s.par_groups.append((s.f, st, list(calls)))

    def par_target(s, c):
        """direct branch target: a user Pruv function returning () (builtins, catalog operations, trusted rust fns and
        non-unit functions are type/parallel_call; an unknown name is left to the ordinary resolution)"""
        if not isinstance(c, Call):
            terr(c, "parallel_call", "a par branch must be a call to a Pruv function",
                 "call a unit Pruv helper that takes the arrays as inout parameters")
        n = c.name
        if n in s.funcs:
            f = s.funcs[n]
            if f.foreign is not None:
                terr(c, "parallel_call", "%s is a trusted rust fn: it cannot be a par branch" % n,
                     "call a unit Pruv helper, and keep trusted rust fns outside the par group")
            if f.borrow_from or any(s.borrowed_type(p.type) for p in f.params):
                s.borrow_signature(c, "borrowed values cannot cross a par branch boundary")
            if f.ret != UNIT:
                terr(c, "parallel_call", "%s returns %s: a par branch must return ()" % (n, type_name(f.ret)),
                     "call a unit Pruv helper and pass results back through whole inout arrays")
        elif n in BUILTIN_FNS or n in CONTRACT_ONLY or n in CATALOG or (n == "old" and s.uses_inout):
            terr(c, "parallel_call", "%s is a builtin or catalog operation: it cannot be a par branch" % n,
                 "call a unit Pruv helper that performs the operation")

    def par_type_ok(s, ty):
        """boundary types: i64, bool, [i64], [u8], [bool], and T / [T] for an Element-bounded type parameter"""
        if ty in (I64, BOOL, ARRAY, ARRAY_U8, ARRAY_BOOL):
            return True
        if ty.owner is not None:
            return (ty.owner, ty.index) in s.elem_params
        if ty.name == "array" and len(ty.args) == 1:
            el = ty.args[0]
            return el.owner is not None and (el.owner, el.index) in s.elem_params
        return False

    def par_args(s, c):
        for arg in c.args:
            if arg.borrowed_view or arg.ty is not None and s.borrowed_type(arg.ty):
                s.borrow_signature(arg, "borrowed values cannot cross a par branch boundary")
            if (not isinstance(arg, (InoutArg, Var, IntLit, BoolLit))
                    or isinstance(arg, InoutArg) and not isinstance(arg.expr, Var)):
                terr(arg, "parallel_argument", "a par branch argument must be a binding name, an integer or bool "
                     "literal, or `inout name`",
                     "bind the value before the group, e.g. `let n: i64 = len(a);`, and pass n")
            if arg.ty is None or not s.par_type_ok(arg.ty):
                terr(arg, "parallel_argument", "an argument of type %s cannot cross a par branch boundary" % (
                    type_name(arg.ty) if arg.ty is not None else "?"),
                     "bind a scalar first, or pass whole [i64], [u8] or [bool] arrays")

    def par_overlap(s, calls):
        """W_i must not meet W_j or R_j, and W_j must not meet R_i, for i < j: the later branch is reported"""
        sets = []
        for c in calls:
            w, r = {}, {}
            for arg in c.args:
                if isinstance(arg, InoutArg) and isinstance(arg.expr, Var):
                    w.setdefault(arg.expr.name, arg)
                elif isinstance(arg, Var):
                    r.setdefault(arg.name, arg)
            sets.append((w, r))
        for j in range(1, len(calls)):
            wj, rj = sets[j]
            for i in range(j):
                wi, ri = sets[i]
                for name, node in wj.items():
                    if name in wi:
                        s.par_alias(node, name, j, i, "writes", "also writes")
                    if name in ri:
                        s.par_alias(node, name, j, i, "writes", "reads")
                for name, node in rj.items():
                    if name in wi:
                        s.par_alias(node, name, j, i, "reads", "writes")

    def par_alias(s, node, name, j, i, mine, theirs):
        terr(node, "parallel_alias", "branch %d %s `%s`, which branch %d %s" % (j + 1, mine, name, i + 1, theirs),
             "use separate bindings for the two branches, or call the helpers one after another")

    # transitive eligibility summary (G7_SPEC 4.2): per function, from the typed bodies; foreign and catalog leaves kept
    def par_info(s, f, cache):
        info = cache.get(f.name)
        if info is not None:
            return info
        info = {"effects": [e for e in ("io", "fs", "proc", "net", "async") if e in f.effects],
                "foreign": f.name if f.foreign is not None else None, "par": False, "callees": []}
        items = []
        s.par_items(f.body, items)
        for kind, x in items:
            if kind == "par":
                info["par"] = True
            elif x.name in s.funcs:
                if x.name not in info["callees"]:
                    info["callees"].append(x.name)
            elif x.name in CATALOG and info["foreign"] is None:
                info["foreign"] = x.name
        cache[f.name] = info
        return info

    def par_items(s, stmts, out):
        """executable calls and par statements of a body in source order (contracts are not executable)"""
        def ex(e):
            if isinstance(e, Call):
                out.append(("call", e))
            for k in expr_kids(e):
                ex(k)
        for st in stmts:
            if isinstance(st, (Let, Assign, FieldAssign)):
                ex(st.value)
            elif isinstance(st, IndexAssign):
                ex(st.index); ex(st.value)
            elif isinstance(st, If):
                ex(st.cond); s.par_items(st.then, out); s.par_items(st.els, out)
            elif isinstance(st, While):
                ex(st.cond); s.par_items(st.body, out)
            elif isinstance(st, ForMap):
                s.par_items(st.body, out)
            elif isinstance(st, Return):
                if st.value is not None:
                    ex(st.value)
            elif isinstance(st, ExprStmt):
                ex(st.expr)
            elif isinstance(st, Par):
                out.append(("par", st))
                for c in st.calls:
                    ex(c)
            elif isinstance(st, Match):
                ex(st.scrutinee)
                for a in st.arms:
                    s.par_items(a.body, out)

    def par_witness(s, start, pred, cache):
        """first function in a deterministic source-order depth-first walk from `start` (itself included) whose summary
        satisfies pred -> the call path start .. witness, or None"""
        seen = set()
        parent = {}
        stack = [(start, None)]
        while stack:
            n, p = stack.pop()
            if n in seen:
                continue
            seen.add(n)
            parent[n] = p
            info = s.par_info(s.funcs[n], cache)
            if pred(info):
                path = []
                x = n
                while x is not None:
                    path.append(x)
                    x = parent[x]
                path.reverse()
                return path
            for c in reversed(info["callees"]):
                if c not in seen:
                    stack.append((c, n))
        return None

    def check_par_eligibility(s):
        cache = {}
        for _f, _st, calls in s.par_groups:
            for c in calls:
                n = c.name
                path = s.par_witness(n, lambda i: bool(i["effects"]), cache)
                if path is not None:
                    eff = s.par_info(s.funcs[path[-1]], cache)["effects"][0]
                    terr(c, "parallel_effect", "par branch %s reaches %s, which declares the `%s` effect (%s)" % (
                        n, path[-1], eff, " -> ".join(path)),
                         "move the %s work outside the par group" % (
                             "I/O" if eff == "io" else "file" if eff == "fs" else "async" if eff == "async" else "network" if eff == "net"
                             else "process"), stage="effect")
                path = s.par_witness(n, lambda i: i["foreign"] is not None, cache)
                if path is not None:
                    leaf = s.par_info(s.funcs[path[-1]], cache)["foreign"]
                    chain = path if leaf == path[-1] else path + [leaf]
                    terr(c, "parallel_foreign", "par branch %s reaches the trusted rust fn or catalog operation %s (%s)" % (
                        n, leaf, " -> ".join(chain)),
                         "use Pruv computation below the par group, and keep foreign calls outside it")
                path = s.par_witness(n, lambda i: i["par"], cache)
                if path is not None:
                    terr(c, "parallel_nested", "par branch %s reaches %s, which contains a par group (%s)" % (
                        n, path[-1], " -> ".join(path)),
                         "keep only the outer par group; call the inner helpers one after another")

    def array_source(s, st, value, scope, allow_local_var=False):
        """Arrays in owning positions need a fresh result or an explicit move."""
        if value.borrowed_view:
            terr(st, "array_alias", "a borrowed view cannot be stored or returned as an owned value",
                 "use copy() to make an independent value")
        if isinstance(value, (Call, StrLit)) or (isinstance(value, Move) and (is_array(value.ty) or value.ty == STR)):
            return   # LG17 (G-DB28-2): `move t` of an owned str binding also hands over its bytes
        if isinstance(value, Var) and scope.get(value.name, (None, None, ""))[2] == "inout":   # G5 A0
            terr(st, "array_alias", "inout parameter %s is a borrowed array: it cannot be stored or returned" % value.name,
                 "write copy(%s) to make an independent array (needs `! alloc`)" % value.name)
        if allow_local_var and isinstance(value, Var) and scope.get(value.name, (None, None, ""))[2] == "local":
            return
        if value.ty == STR:   # G2 B0
            terr(st, "array_alias", "a str value here must come from a string literal, copy(), substr() or a function call",
                 "write copy(x) to make an explicit copy (needs `! alloc`)")
        terr(st, "array_alias", "an array value here must come from move, alloc(), copy() or a function call",
             "write copy(x) to make an explicit copy (needs `! alloc`)")

    # ---- G9 A0: ownership of extended values, active-match borrows ----
    def rec_vec_get(s, e):
        return (isinstance(e, Call) and e.name == "vec_get" and e.name not in s.funcs
                and e.ty is not None and s.rec_vec_elem(e.ty))

    def own(s, value):
        """`value` (already typed) is moved into an owning position (let, assignment, constructor field, return):
        a non-fresh extended value is cloned, which needs `alloc`; constructors, literals and calls are fresh"""
        t = value.ty
        if getattr(value, "map_iter_view", False):
            terr(value, "foreign_move", "a map iteration binder cannot be used as an owned value")
        if getattr(value, "map_mut_view", False):
            terr(value, "foreign_move", "a mutable collection binder cannot be used as an owned value")
        if t is not None:
            if borrowing_copy(s.decls.get(t.name), s.extended, s.decls):
                return
            if ((s.is_extended(t) or is_owned_value(t) or is_array(t) or t == STR)
                    and s.rec_vec_get(field_place(value)[0])):
                terr(value, "foreign_owned_type", "vec_get cannot copy %s" % type_name(t))
            if s.is_extended(t) and not isinstance(value, (EnumCtor, StructLit, Call, Move)):
                s.used_effects.add("alloc")

    def own_field(s, value, ftype, scope, c):
        """a constructor / struct-literal field argument: owning position, G2 explicit ownership for buffers"""
        if c:
            return
        s.own(value)
        if is_array(ftype) or ftype == STR:
            s.array_source(value, value, scope)

    def ctor_allocs(s, decl_name, variant, c):
        """a constructor that initializes a boxed field allocates"""
        if not c and (decl_name, variant) in s.boxed_decls:
            s.used_effects.add("alloc")

    def match_root(s, scrut, t, scope):
        """root variable of a match scrutinee that is a variable / field path over an extended type, else None"""
        if not s.is_extended(t):
            return None
        e = scrut
        while isinstance(e, FieldAccess):
            e = e.base
        if isinstance(e, Var) and e.name in scope and scope[e.name][2] in ("local", "param"):
            return e.name
        return None

    def match_inout_path(s, scrut, t, scope):
        """R-6e-8: the held path of an extended match on an inout, element binding or mutable local field."""
        if not s.is_extended(t):
            return None
        e, fields = scrut, []
        while isinstance(e, FieldAccess):
            fields.insert(0, e.field)
            e = e.base
        if isinstance(e, Var) and e.name in scope:
            _, mutable, kind = scope[e.name]
            if kind in ("inout", "map_mut") or fields and kind == "local" and mutable:
                return (e.name, tuple(fields))
        return None

    def match_path_held(s, name, path):
        return any(root == name and _paths_overlap(path, held) for root, held in s.match_paths)

    def check_iteration_borrow(s, node, name, path):
        if any(root == name and _paths_overlap(path, held) for root, held in s.map_iterations):
            terr(node, "recursive_borrow", "cannot mutate or move %s during map iteration" % ".".join([name] + list(path)))

    def check_borrow(s, st, name, fields):
        s.check_iteration_borrow(st, name, fields)
        if name in s.active_roots or s.match_path_held(name, tuple(fields)):
            terr(st, "recursive_borrow", "cannot assign to %s while a match on it is active" % ".".join([name] + list(fields)),
                 "copy the needed payload into a local, finish the match, then assign")

    def field_move_out(s, st, ty, scope, next_stmt):
        e = st.expr if isinstance(st, ExprStmt) else st.value
        if isinstance(st, (Let, ExprStmt)) and isinstance(e, Call):
            return s.field_move_arg(e, scope, next_stmt)
        if not isinstance(e, Move) or not e.fields:
            return None
        s.check_borrow(e, e.name, e.fields)
        _, mutable, kind = s.lookup(e, e.name, scope, e.fields)
        typed = ty == STRING or is_scalar_vec(ty) or ty in (ARRAY, ARRAY_U8, ARRAY_BOOL)
        restored = (isinstance(next_stmt, FieldAssign) and next_stmt.name == e.name
                    and tuple(next_stmt.fields) == tuple(e.fields))
        # LB9 (R-6g8-4): or a struct binder of a consuming match (the arm owns the payload; see Match)
        binder = e.name in s.match_movable and kind == "local"
        if (typed and not restored and isinstance(st, Let)
                and (((kind == "local" and mutable) or kind == "take") and e.name not in s.arm_binders or binder)):
            # G-R09-4: `let t = move d.a;` of a `let mut` local record (LG9: or a take record parameter, which the
            # callee owns; LG9 G-LG8-1: any owned buffer field type, any field path) consumes d; foreign.py marks d
            # moved
            return e
        if not (mutable or kind in ("inout", "map_mut")) or not typed:
            _field_move_error(e)
        if isinstance(st, FieldAssign) and (st.name != e.name or _paths_overlap(st.fields, e.fields)):
            _field_move_error(e)
        if (not isinstance(next_stmt, FieldAssign) or next_stmt.name != e.name
                or tuple(next_stmt.fields) != tuple(e.fields)):
            _field_move_error(e)
        restore = next_stmt.value
        chained = isinstance(restore, Move) and restore.name == e.name and restore.fields
        if not isinstance(restore, (Call, Move)) or (not chained and _mentions(restore, e.name)):
            _field_move_error(e)
        return e

    def field_move_arg(s, call, scope, next_stmt):
        """LG9 (G-LG8-1): `f(move d.a)` as a let initializer or call statement, the field move a direct argument of a
        take parameter, consumes d like `let t = move d.a;` (no restore follows; no other argument mentions d)"""
        if not any(isinstance(arg, Move) and arg.fields for arg in call.args):
            return None
        moves = list(_field_moves(call))
        if len(moves) != 1:
            return None
        e = moves[0]
        callee = s.funcs.get(call.name) or CATALOG.get(call.name)
        if callee is None or not any(arg is e and p.take for arg, p in zip(call.args, callee.params)):
            return None
        if any(_mentions(arg, e.name) for arg in call.args if arg is not e):
            return None
        if (isinstance(next_stmt, FieldAssign) and next_stmt.name == e.name
                and tuple(next_stmt.fields) == tuple(e.fields)):
            return None
        s.check_borrow(e, e.name, e.fields)
        _, mutable, kind = s.lookup(e, e.name, scope, e.fields)
        if not ((kind == "local" and mutable) or kind == "take") or e.name in s.arm_binders:
            return None
        ty = s.lookup(e, e.name, scope, e.fields)[0]
        for field in e.fields:
            if not s.tt.is_struct(ty):
                return None
            ty = s.field_type(e, ty, field)
        if not (ty == STRING or is_scalar_vec(ty) or ty in (ARRAY, ARRAY_U8, ARRAY_BOOL)):
            return None
        return e

    def field_update_move(s, st, ty, binding, scope):
        """Select a take argument consumed by the call that replaces its field."""
        moves = list(_field_moves(st.value))
        if not moves:
            return None
        for move in moves:
            s.check_borrow(move, move.name, move.fields)
            s.lookup(move, move.name, scope, move.fields)
        e = moves[0]
        if ty in (ARRAY, ARRAY_U8, ARRAY_BOOL):    # LG4: as check.rs, a nested array field move is array_alias
            terr(e, "array_alias", "an array field cannot be moved", "use copy() to make an independent array")
        if (len(moves) != 1 or e.fields != tuple(st.fields) or e.name != st.name
                or not (binding[1] or binding[2] in ("inout", "map_mut")) or not is_collection(ty) or not isinstance(st.value, Call)):
            _field_move_error(e)
        call = st.value
        callee = s.funcs.get(call.name) or CATALOG.get(call.name)
        if (callee is None or not any(arg is e and p.take for arg, p in zip(call.args, callee.params))
                or any(_mentions(arg, st.name) for arg in call.args if arg is not e)):
            _field_move_error(e)
        return e

    def alloc_hint_for(s, value, expected):
        """G2: only a builtin alloc call that IS the right-hand side of a typed let / array reassignment /
        return takes its array type from the expected type"""
        if isinstance(value, Call) and value.name == "alloc" and is_array(expected):
            s.alloc_hint = (value, expected)

    def expect_array(s, e, scope, c, coll=False, *, borrow_element=False):
        """the argument must be an array of any element type (G2 B0: or a str); same diagnostic as expect(e, ARRAY, ...)
        coll (G6): a Vec/HashMap/String is accepted too"""
        t = s.expr(e, scope, c, borrow_element=borrow_element)
        if t.owner is not None:
            terr(e, "generic_operation", "%s is an abstract type parameter, not an array" % type_name(t),
                 "only [T] with T: Element can be indexed, measured or copied")
        if not is_array(t) and t != STR and not (coll and is_collection(t)):
            terr(e, "type_mismatch", "expected %s, found %s" % (type_name(ARRAY), type_name(t)))
        return t

    @staticmethod
    def elem_type(at):
        """type of an element read from / stored into the array type `at`: T for [T]; bool for [bool]; else i64"""
        if at.name == "array" and at.args:
            return at.args[0]
        return BOOL if at == ARRAY_BOOL else I64

    def lookup(s, node, name, scope, path=None):
        """path: the field path of the place read through `name`; a mutable collection Some arm admits a path of
        its root that is disjoint from every borrowed path of that root (R-6e-8)"""
        if path is None:
            path = s.field_paths.get(id(node))
        if any(root == name and (path is None or _paths_overlap(path, held)) for root, held, _ in s.map_borrows):
            terr(node, "recursive_borrow", "cannot use %s inside a mutable collection Some arm" % name)
        if name not in scope:
            terr(node, "unknown_name", "unknown variable %s" % name)
        return scope[name]

    # ---- expressions ----
    def expect(s, e, ty, scope, contract=False, op=None, *, borrow_element=False, borrow_vector=False):
        """op: what the operand is used for (arithmetic, comparison, ...) when the operation needs a concrete
        type: an abstract type parameter there is a generic_operation error rather than a plain mismatch"""
        t = s.expr(e, scope, contract, borrow_element=borrow_element)
        if ty == STR and t == STRING and borrow_element and getattr(e, "map_iter_view", False):
            return t
        if borrow_vector and isinstance(field_place(e)[0], Var) and is_vec_slice_arg(t, ty):
            return t
        if t != ty:
            if op is not None and t.owner is not None:
                terr(e, "generic_operation", "%s is not allowed on the abstract type %s" % (op, t.name),
                     "a type parameter can be copied, passed, returned and stored; == works on it in contracts only")
            terr(e, "type_mismatch", "expected %s, found %s" % (type_name(ty), type_name(t)))
        return t

    def expr(s, e, scope, contract=False, *, borrow_element=False):
        t = s._expr(e, scope, contract, borrow_element)
        e.ty = t
        return t

    def _expr(s, e, scope, c, borrow_element):
        if s.ckind == "cost" and (isinstance(e, Spent)
                or isinstance(e, Var) and e.name == "result"
                or isinstance(e, Call) and e.name == "old"):
            s.s5_errors.append((s.f, e, "cost_expr", "cost cannot contain spent, result or old"))
            if isinstance(e, Call):
                types = [s.expr(a, scope, c) for a in e.args]
                return types[0] if types else I64
            return s.f.ret if isinstance(e, Var) and s.f.ret != UNIT else I64
        if isinstance(e, Spent):
            if isinstance(s.f, StructDecl):
                terr(e, "spent_context", "spent is only allowed in invariant/assert of a cost function")
            if s.f.cost is None or s.ckind not in ("invariant", "assert"):
                s.s5_errors.append((s.f, e, "spent_context", "spent is only allowed in invariant/assert of a cost function"))
            return I64
        if isinstance(e, IntLit):
            if not (-2 ** 63 <= e.value < 2 ** 63):
                terr(e, "int_range", "integer literal out of i64 range")
            return I64
        if isinstance(e, BoolLit):
            return BOOL
        if isinstance(e, StrLit):   # G2 B0
            if not c and not e.static_str:
                s.used_effects.add("alloc")
            return STR
        if isinstance(e, Var):
            if e.name == "result" and "result" not in scope:
                terr(e, "result_scope", "`result` is only allowed in ensures")
            binding = s.lookup(e, e.name, scope)
            e.borrowed_view = binding[2] == "ref"
            if binding[2] == "map_mut" and binding[0] not in (I64, BOOL):
                e.map_mut_view = True
            if binding[2] == "map_iter":
                e.map_iter_view = True
            return binding[0]
        if isinstance(e, Unary):
            if e.op == "*":
                x = e.operand
                if not isinstance(x, Var):
                    terr(e, "type_mismatch", "dereference requires a mutable scalar collection binder")
                ty, _, mode = s.lookup(e, x.name, scope)
                if mode != "map_mut" or ty not in (I64, BOOL):
                    terr(e, "type_mismatch", "dereference requires a mutable scalar collection binder")
                # Scalar binders already read through the reference in subsequent passes.
                e.__class__ = Var
                e.__dict__ = vars(x).copy()
                return ty
            if e.op == "-":
                s.expect(e.operand, I64, scope, c, op="unary minus"); return I64
            if e.op == "~":
                s.expect(e.operand, I64, scope, c); return I64
            s.expect(e.operand, BOOL, scope, c, op="a boolean operation"); return BOOL
        if isinstance(e, Binary):
            op = e.op
            if op in ("+", "-", "*", "/", "%"):
                s.expect(e.left, I64, scope, c, op="arithmetic")
                s.expect(e.right, I64, scope, c, op="arithmetic"); return I64
            if op in ("&", "|", "^", "<<", ">>"):
                s.expect(e.left, I64, scope, c)
                s.expect(e.right, I64, scope, c); return I64
            if op in ("<", "<=", ">", ">="):
                s.expect(e.left, I64, scope, c, op="ordering")
                s.expect(e.right, I64, scope, c, op="ordering"); return BOOL
            if op in ("==", "!="):
                lt = s.expr(e.left, scope, c)
                if is_owned_value(lt):   # G6 / LG11 E2
                    terr(e, "eq_type", "collections cannot be compared with ==",
                         "compare lengths and elements: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]")
                if is_array(lt):
                    terr(e, "eq_type", "arrays cannot be compared with ==",
                         "compare lengths and elements: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]")
                if lt == STR:   # G2 B0
                    terr(e, "eq_type", "str values cannot be compared with ==",
                         "compare lengths and bytes: len(a) == len(b) && forall k in 0..len(a): a[k] == b[k]")
                if lt.owner is None and any(n in s.buffered for n in dependencies(lt)):   # G9 A0
                    terr(e, "eq_type", "values of %s contain arrays or str and cannot be compared with ==" % type_name(lt),
                         "compare tags, lengths and indexed contents instead")
                if lt.owner is not None and not c:
                    terr(e, "generic_operation", "== is not allowed on the abstract type %s in code" % lt.name,
                         "runtime equality only compares i64 or bool; == on a type parameter works in contracts")
                if lt not in (I64, BOOL) and not c:
                    terr(e, "eq_type", "== only compares i64 or bool in code", "use match")
                s.expect(e.right, lt, scope, c)
                return BOOL
            if op in ("&&", "||"):
                s.expect(e.left, BOOL, scope, c, op="a boolean operation")
                s.expect(e.right, BOOL, scope, c, op="a boolean operation"); return BOOL
            if op == "==>":
                if not c:
                    terr(e, "contract_only", "==> is only allowed in contracts")
                s.expect(e.left, BOOL, scope, c, op="a boolean operation")
                s.expect(e.right, BOOL, scope, c, op="a boolean operation"); return BOOL
        if isinstance(e, Move):   # G6: liveness and position rules are checked by foreign.check_ownership
            s.check_borrow(e, e.name, tuple(e.fields))
            if scope.get(e.name, (None, None, None))[2] == "map_iter":
                terr(e, "foreign_move", "a map iteration binder cannot be moved")
            if e.fields:
                ty = s.lookup(e, e.name, scope, tuple(e.fields))[0]
                for field in e.fields:
                    if not s.tt.is_struct(ty):
                        _field_move_error(e)
                    if s.ref_field(ty, field):
                        terr(e, "array_alias", "a borrowed view cannot be moved")
                    ty = s.field_type(e, ty, field)
                if is_array(ty) and e is not s.field_move:
                    terr(e, "array_alias", "an array field cannot be moved", "use copy() to make an independent array")
                if e is not s.field_move:
                    _field_move_error(e)
                return ty
            if c:
                terr(e, "foreign_move", "move only where ownership is consumed",
                     "contracts read parameters and locals directly; remove `move`")
            ty, _, kind = s.lookup(e, e.name, scope)
            if kind == "ref" or s.borrowed_type(ty):
                terr(e, "array_alias", "a borrowed value cannot be moved")
            if e.name in s.arm_binders and s.rec_vec(ty):
                terr(e, "foreign_move", "a borrowed vector payload cannot be moved")
            if ((is_array(ty) or ty == STR) and kind not in ("local", "take")
                    and not (kind == "inout" and e.name == s.restore_array)
                    and e.name not in s.match_movable):    # G-DB16-1: array binder of a consuming match
                if kind in ("param", "inout"):
                    terr(e, "array_alias", "parameter %s is a borrowed array and cannot be moved" % e.name,
                         "declare `take %s: %s` or use copy(%s)" % (e.name, type_name(ty), e.name))
                terr(e, "array_alias", "only an owned array binding can be moved", "use copy() to make an independent array")
            return ty
        if isinstance(e, Index):
            at = s.expect_array(e.array, scope, c, coll=True)
            if isinstance(e.array, Move) and is_array(at):
                terr(e.array, "array_alias", "an array element cannot be moved", "read the element without move")
            if at.name == "hashmap" and at.owner is None:
                terr(e.array, "type_mismatch", "expected %s, found %s" % (type_name(ARRAY), type_name(at)))
            s.expect(e.index, I64, scope, c)
            if is_collection(at):     # G6: contract-only observation of a Vec / String
                if not c:
                    terr(e, "contract_only", "indexing a %s is only allowed in contracts" % type_name(at),
                         "use vec_get in code" if at.name == "vec" else "use string_byte in code")
                if at.name == "vec" and at.args[0] == STRING:
                    terr(e, "foreign_owned_type", "Vec<String> elements cannot be read")
                return at.args[0] if at.name == "vec" else I64
            return s.elem_type(at)  # G2 A5: bool array reads bool; G3: [T] reads T
        if isinstance(e, Ctor):
            return s.ctor(e, scope, c)
        if isinstance(e, Quant):
            agg = e.kind in ("sum", "count", "max", "min")
            if not c:
                if agg:
                    terr(e, "contract_only", "%s is only allowed in contracts" % e.kind,
                         "compute it with a while loop and state it in ensures, "
                         "e.g. ensures result == sum(k in 0..len(a): a[k])")
                terr(e, "contract_only", "%s is only allowed in contracts" % e.kind)
            s.expect(e.lo, I64, scope, c); s.expect(e.hi, I64, scope, c)
            if e.var in scope:
                terr(e, "shadow", "%s is already defined" % e.var, "use a new name")
            sc = dict(scope); sc[e.var] = (I64, False, "bound")
            for ti, tg in enumerate(e.triggers):    # R-6-3
                why = None
                if agg:
                    why = "%s takes no trigger" % e.kind
                elif not isinstance(tg, Index):
                    why = "a trigger must be an array read a[e]"
                elif not _mentions(tg, e.var):
                    why = "a trigger must mention %s" % e.var
                if why is not None:
                    terr(e, "bad_trigger", "trigger %d of quantifier over %s: %s" % (ti + 1, e.var, why),
                         "write trigger a[...] with an array read that uses %s" % e.var)
                s.expr(tg, sc, c)
                if not _mentions(tg, e.var):    # a contract call in the trigger is expanded in place
                    terr(e, "bad_trigger", "trigger %d of quantifier over %s: after expanding calls it does not "
                         "mention %s" % (ti + 1, e.var, e.var),
                         "write trigger a[...] with an array read that uses %s" % e.var)
            s.expect(e.body, BOOL if e.kind in ("forall", "exists", "count") else I64, sc, c,
                     op="a quantifier body" if e.kind in ("forall", "exists", "count") else "an aggregate (%s)" % e.kind)
            return I64 if agg else BOOL
        if isinstance(e, StructLit):
            return s.struct_lit(e, scope, c)
        if isinstance(e, EnumCtor):
            ety = s.enum_type(e, e.enum_name)
            m = s.instantiate(e, "enum %s" % e.enum_name, decl_owner(e.enum_name), s.decls[e.enum_name].tparams,
                              e.targs)
            if m is not None:
                ety = mk_nominal(e.enum_name, e.targs)
                e.inst = {"decl": decl_owner(e.enum_name), "args": tuple(e.targs), "subst": m}
            s.variant_decl(e, ety, e.variant)
            fields = s.fields_of(ety, e.variant)
            if len(e.args) != len(fields):
                terr(e, "arity", "%s::%s takes %d argument(s), got %d" % (
                    e.enum_name, e.variant, len(fields), len(e.args)))
            for arg, (fd, ft) in zip(e.args, fields):
                s.expect(arg, ft, scope, c)
                s.own_field(arg, ft, scope, c)
            s.ctor_allocs(e.enum_name, e.variant, c)
            return ety
        if isinstance(e, FieldAccess):
            froot, fpath = field_place(e)
            if isinstance(froot, Var):
                s.field_paths.setdefault(id(froot), fpath)
            saved, s.field_receiver = s.field_receiver, e.base
            bt = s.expr(e.base, scope, c)
            s.field_receiver = saved
            if bt.owner is not None:
                terr(e, "generic_operation", "cannot read field %s: %s is an abstract type parameter" % (
                    e.field, type_name(bt)), "a type parameter has no fields")
            if not s.tt.is_struct(bt):
                terr(e, "not_struct", "cannot read field %s: %s is not a struct" % (e.field, type_name(bt)),
                     "enum payloads are read with match" if s.tt.is_enum(bt) else "")
            e.borrowed_view = s.ref_field(bt, e.field)
            return s.field_type(e, bt, e.field)
        if isinstance(e, IsVariant):
            if not c:
                terr(e, "contract_only", "is_variant is only allowed in contracts", "use match in code")
            e.enum_ty = s.variant_target(e, scope, c)
            return BOOL
        if isinstance(e, VariantField):
            if not c:
                terr(e, "contract_only", "variant_field is only allowed in contracts", "use match in code")
            ety = s.variant_target(e, scope, c)
            e.enum_ty = ety
            for fd, ft in s.fields_of(ety, e.variant):
                if fd.name == e.field:
                    return ft
            terr(e, "unknown_field", "variant %s::%s has no field %s" % (e.enum_name, e.variant, e.field))
        if isinstance(e, InoutArg):   # G5 A0: valid only as a direct argument of a call (see inout_args)
            terr(e, "inout_argument", "the `inout` marker is only valid on an argument of a function whose parameter is inout",
                 "remove `inout`")
        if isinstance(e, Call):
            callee = s.funcs.get(e.name)   # s.call can rewrite e in place (old(c).f becomes a field read)
            t = s.call(e, scope, c, borrow_element)
            e.borrowed_view = callee is not None and callee.ref_result
            x = s.x_pending
            if x is not None:     # P06: a contract call to a user function becomes its expansion, in place
                s.x_pending = None
                e.__class__ = x.__class__
                e.__dict__ = x.__dict__
            return t
        terr(e, "internal", "unknown expression")

    # ---- user type helpers ----
    def fields_of(s, ty, variant=None):
        """[(FieldDecl, field type)] of the struct / enum variant `ty` names, with the type arguments of `ty`
        substituted into the declared field types (G3_SPEC 3.4)"""
        d = s.decls[ty.name]
        m = bind_args(decl_owner(ty.name), len(d.tparams), ty.args)
        return [(fd, substitute(fd.type, m)) for fd in s.tt.fields(ty, variant)]

    def field_type(s, node, sty, name):
        # find the field first, then substitute only its type (substituting every field made x.f O(fields))
        for fd in s.tt.fields(sty, None):
            if fd.name == name:
                if not sty.args:
                    return fd.type
                d = s.decls[sty.name]
                return substitute(fd.type, bind_args(decl_owner(sty.name), len(d.tparams), sty.args))
        terr(node, "unknown_field", "struct %s has no field %s" % (sty.name, name))

    def enum_type(s, node, name):
        """Type of a named enum; unknown_type / not_enum otherwise"""
        if name not in s.decls:
            terr(node, "unknown_type", "unknown type %s" % name)
        ty = Type(name)
        if not s.tt.is_enum(ty):
            terr(node, "not_enum", "%s is not an enum" % name)
        return ty

    def variant_decl(s, node, ety, variant):
        for v in s.tt.decl(ety).variants:
            if v.name == variant:
                return v
        terr(node, "unknown_variant", "enum %s has no variant %s" % (ety.name, variant))

    def variant_target(s, e, scope, c):
        """shared by is_variant / variant_field: value, enum and variant checks"""
        vt = s.expr(e.value, scope, c)
        ety = s.enum_type(e, e.enum_name)
        s.variant_decl(e, ety, e.variant)
        # G3: no explicit type arguments here; they come from the checked scrutinee, which keeps its full type
        if vt.owner is not None or vt.name != ety.name:
            terr(e.value, "type_mismatch", "expected %s, found %s" % (type_name(ety), type_name(vt)))
        return vt

    def struct_lit(s, e, scope, c):
        if e.name not in s.decls:
            terr(e, "unknown_type", "unknown type %s" % e.name)
        sty = Type(e.name)
        if not s.tt.is_struct(sty):
            terr(e, "not_struct", "%s is not a struct" % e.name,
                 "enum values are written %s::Variant" % e.name)
        m = s.instantiate(e, "struct %s" % e.name, decl_owner(e.name), s.decls[e.name].tparams, e.targs)
        if m is not None:
            sty = mk_nominal(e.name, e.targs)
            e.inst = {"decl": decl_owner(e.name), "args": tuple(e.targs), "subst": m}
        fields = [fd for fd, _ in s.fields_of(sty)]
        ftypes = [ft for _, ft in s.fields_of(sty)]
        declared = [f.name for f in fields]
        for fi in e.fields:
            if fi.name not in declared:
                terr(fi, "unknown_field", "struct %s has no field %s" % (e.name, fi.name))
        given = [fi.name for fi in e.fields]
        if given != declared:
            if len(set(given)) != len(given):
                dup = [n for n in given if given.count(n) > 1][0]
                why = "field %s is given twice" % dup
            elif set(given) != set(declared):
                why = "missing field(s): %s" % ", ".join(n for n in declared if n not in given)
            else:
                why = "fields must be written in declaration order: %s" % ", ".join(declared)
            terr(e, "struct_fields", "%s literal: %s" % (e.name, why),
                 "write every field exactly once, in declaration order")
        for fi, ft, fd in zip(e.fields, ftypes, fields):
            if fd.ref:
                s.view_value(fi.value, ft, scope, c)
            else:
                s.expect(fi.value, ft, scope, c)
                s.own_field(fi.value, ft, scope, c)
        s.ctor_allocs(e.name, None, c)
        return sty

    def ctor(s, e, scope, c):
        """Some / None / Ok / Err. Without type arguments they keep the legacy i64 meaning (G3_SPEC 0.0 item 8);
        `Some::<X>(x)`, `None::<X>`, `Ok::<X>(x)`, `Err::<X>(code)` build Option<X> / Result<X> (3.5)."""
        if e.targs is None:
            if e.name == "None":
                return OPTION
            if e.arg is None:      # LB8 R-6g7-1: unqualified Ok keeps Result<i64>
                s.unit_payload(e, I64)
            s.expect(e.arg, I64, scope, c)
            if not c:
                s.own(e.arg)
            return OPTION if e.name == "Some" else RESULT
        is_result = e.name in ("Ok", "Err")
        if len(e.targs) != 1 and not (is_result and len(e.targs) == 2):
            if is_result:
                terr(e, "type_arity", "%s::<..> takes 1 or 2 type arguments, got %d" % (e.name, len(e.targs)),
                     "write %s::<T> or %s::<T, E>" % (e.name, e.name))
            terr(e, "type_arity", "%s::<..> takes exactly 1 type argument, got %d" % (e.name, len(e.targs)),
                 "write %s::<T>" % e.name)
        x = e.targs[0]
        s.check_type(e, x)
        if not (is_result and x == UNIT):      # LB8 R-6g7-1: Ok::<(), E>(()), Err::<(), E>(e)
            s.check_arg(e, x, None)
            s.ext_misuse(e, x, "arg")      # G9 A0
        error = e.targs[1] if len(e.targs) == 2 else I64
        if error != I64:
            s.check_type_arity(e, error)
            s.check_result_error(e, error)
        args = (x,) if error == I64 else (x, error)
        owner = "builtin:" + e.name
        e.inst = {"decl": owner, "args": args, "subst": bind_args(owner, len(args), args)}
        if e.name == "None":
            return mk_option(x)
        if e.name == "Ok" and (x == UNIT) != (e.arg is None):
            s.unit_payload(e, x)
        if e.arg is None:
            return mk_result(x, error)
        s.expect(e.arg, error if e.name == "Err" else x, scope, c)
        if not c:
            s.own(e.arg)
        return mk_option(x) if e.name == "Some" else mk_result(x, error)

    def unit_payload(s, e, x):
        """LB8 R-6g7-1: the unit value `()` is written only as `Ok::<(), E>(())`, and that Ok takes nothing else"""
        if e.arg is None:
            terr(e, "type_mismatch", "expected %s, found ()" % type_name(x),
                 "`Ok(())` builds a Result<(), E>: write Ok::<(), E>(())")
        terr(e.arg, "type_mismatch", "expected (), found a value",
             "the Ok payload of a Result<(), E> is written Ok::<(), E>(())")

    def observer_args(s, e, f, m, scope, c):
        """map_lookup / map_updated arguments: a key whose type differs from a map argument's key type is a
        foreign_contract error; any other mismatch is the ordinary type_mismatch"""
        got = [s.expr(arg, scope, c) for arg in e.args]
        kidx = [p.name for p in f.params].index("k")
        for g in got:
            if (g.name == "hashmap" and g.owner is None and g.args[0] != got[kidx]
                    and not (e.name == "map_lookup" and g.args[0] == STRING)):
                terr(e.args[kidx], "foreign_contract", "key of type %s does not match the map key type %s" % (
                    type_name(got[kidx]), type_name(g.args[0])),
                     "pass a key of the map's key type")
        for arg, g, p in zip(e.args, got, f.params):
            want = substitute(p.type, m)
            if e.name == "map_lookup" and p.name == "k" and want == STRING:
                s.check_text_key(arg, scope, c, g)
            elif g != want:
                terr(arg, "type_mismatch", "expected %s, found %s" % (type_name(want), type_name(g)))

    def check_text_key(s, arg, scope, c, actual=None):
        if isinstance(arg, StrLit):
            arg.ty = STR
            return
        if actual is None:
            actual = s.expr(arg, scope, c, borrow_element=True)
        if actual == STR or actual == STRING and isinstance(arg, (Var, FieldAccess, VariantField)):
            return
        terr(arg, "type_mismatch", "expected str or a borrowed String read, found %s" % type_name(actual))

    # ---- G5 A0: inout call arguments and old(..) ----
    def ref_field(s, ty, name):
        decl = s.decls.get(ty.name)
        return isinstance(decl, StructDecl) and any(fd.name == name and fd.ref for fd in decl.fields)

    def view_value(s, e, want, scope, contract=False):
        s.user_arg(e, want, scope, contract, True)
        if e.ty != want or not (e.borrowed_view or isinstance(e, StrLit) or isinstance(field_place(e)[0], Var)
                or isinstance(e, Call) and e.name == "subview"):
            s.borrow_signature(e, "a borrowed view needs a place, subview, borrowed result or string literal")

    def view_provenance(s, e, loans, exclusive=False):
        if isinstance(e, StrLit):
            return {}
        if isinstance(e, InoutArg):
            return s.view_provenance(e.expr, loans, True)
        if isinstance(e, Call):
            if e.name == "subview":
                return s.view_provenance(e.args[0], loans, exclusive)
            f = s.funcs.get(e.name)
            if f is None or not f.borrow_from:
                s.borrow_signature(e, "a borrowed result source must be a place or a borrowed value")
            sources = [s.view_provenance(arg, loans, exclusive or bool(p.mode))
                       for p, arg in zip(f.params, e.args) if p.name in f.borrow_from]
        elif isinstance(e, FieldAccess) and (e.borrowed_view or s.borrowed_type(e.ty)):
            return s.view_provenance(e.base, loans, exclusive)
        elif isinstance(e, StructLit) and s.borrowed_type(e.ty):
            sources = [s.view_provenance(fi.value, loans, exclusive)
                       for fi, fd in zip(e.fields, s.decls[e.name].fields)
                       if fd.ref or s.borrowed_type(fd.type)]
        else:
            root, path = field_place(e)
            if not isinstance(root, Var):
                s.borrow_signature(e, "a borrowed result source must be a place or a borrowed value")
            if not path and root.name in loans:
                return {name: held or exclusive for name, held in loans[root.name].items()}
            if path and any(
                    p.name == root.name and s.borrowed_type(p.type) for p in s.f.params):
                # An owned field of a borrowing-struct parameter lives in the caller's record, which the
                # Rust signature `&Doc<'a>` does not lend to the result.
                return {root.name: exclusive, root.name + ".storage": exclusive}
            return {root.name: exclusive}
        merged = {}
        for source in sources:
            for name, held in source.items():
                merged[name] = merged.get(name, False) or held
        return merged

    @staticmethod
    def view_stmt_items(st):
        for value in vars(st).values():
            yield from value if isinstance(value, list) else (value,)

    def view_uses(s, st, nested=False):
        """Yield (root, write) for the syntax of this statement."""
        if isinstance(st, (Assign, FieldAssign, IndexAssign)):
            yield st.name, True
        for item in s.view_stmt_items(st):
            if isinstance(item, Expr):
                for e in _walk(item):
                    if isinstance(e, (Var, Move)):
                        yield e.name, isinstance(e, Move)
                    elif isinstance(e, InoutArg):
                        root, _ = field_place(e.expr)
                        if isinstance(root, Var):
                            yield root.name, True
            elif nested and isinstance(item, Stmt):
                yield from s.view_uses(item, True)
            elif nested and isinstance(item, MatchArm):
                for child in item.body:
                    yield from s.view_uses(child, True)

    def check_view_block(s, stmts, loans, active=None):
        loans = dict(loans)
        inherited = {} if active is None else active
        live = []
        last = {name: i for i, st in enumerate(stmts) for name, _ in s.view_uses(st, True)}
        for i, st in enumerate(stmts):
            live = [(end, provenance) for end, provenance in live if i <= end]
            held = dict(inherited)
            for _, provenance in live:
                for name, exclusive in provenance.items():
                    held[name] = held.get(name, False) or exclusive
            for name, write in s.view_uses(st):
                if name in held and (write or held[name]):
                    terr(st, "borrow_conflict", "%s is still borrowed by a live view" % name)
            # Validate borrowed calls even when their results are used only inside the statement.
            for item in s.view_stmt_items(st):
                if isinstance(item, Expr):
                    for e in _walk(item):
                        if isinstance(e, Call) and e.name in s.funcs and s.funcs[e.name].borrow_from:
                            s.view_provenance(e, loans)
            if isinstance(st, Let) and (st.ref or s.borrowed_type(st.type)):
                provenance = s.view_provenance(st.value, loans)
                loans[st.name] = provenance
                s.f.borrowed_roots.update(provenance)
                if last.get(st.name, i) > i:
                    live.append((last[st.name], provenance))
            elif isinstance(st, Return) and s.f.borrow_from and st.value is not None:
                provenance = s.view_provenance(st.value, loans)
                if set(provenance) - set(s.f.borrow_from):
                    terr(st, "borrow_escape", "the result borrows a value outside the from parameters")
            if isinstance(st, If):
                s.check_view_block(st.then, loans, held)
                s.check_view_block(st.els, loans, held)
            elif isinstance(st, (While, ForMap)):
                s.check_view_block(st.body, loans, held)
            elif isinstance(st, Match):
                for arm in st.arms:
                    s.check_view_block(arm.body, loans, held)

    @staticmethod
    def view_context(e):
        terr(e, "view_context", "subview(...) is only allowed as a direct argument to a non-inout array parameter of a function call",
             "pass it straight to a function, or use copy_range to make an owned array")

    def call_user_arg(s, e, want, scope, c, borrow):
        if borrow and want == ARRAY_U8 and isinstance(e, StrLit):
            e.ty = ARRAY_U8
            e.static_str = True
            return
        s.user_arg(e, want, scope, c, borrow)

    def user_arg(s, e, want, scope, c, borrow):
        if borrow and want == STR and isinstance(e, StrLit):
            e.static_str = True
        if isinstance(e, Call) and e.name == "subview":
            if c or not borrow or not (is_array(want) or want == STR) or s.in_view:
                s.view_context(e)
            if e.targs is not None:
                terr(e, "type_arity", "builtin subview takes no type arguments", "remove the `::<...>`")
            if len(e.args) != 3:
                terr(e, "arity", "subview takes 3 argument(s), got %d" % len(e.args))
            root = e.args[0]
            while isinstance(root, FieldAccess):
                root = root.base
            s.in_view = True
            if not isinstance(root, Var):
                try:
                    if want == STR:
                        s.expr(e.args[0], scope, c)
                    else:
                        s.expect(e.args[0], want, scope, c)   # a view nested in the source is reported where it is
                finally:
                    s.in_view = False
                s.view_context(e)
            try:
                if want == STR:   # LG25 (R-6g-10): a str or [u8] source viewed as str
                    st = s.expr(e.args[0], scope, c)
                    if st != STR and st != ARRAY_U8:
                        terr(e.args[0], "type_mismatch", "expected %s, found %s" % (type_name(STR), type_name(st)))
                else:
                    s.expect(e.args[0], want, scope, c)
                s.expect(e.args[1], I64, scope, c)
                s.expect(e.args[2], I64, scope, c)
            finally:
                s.in_view = False
            e.ty = want
        else:
            saved, s.read_arg = s.read_arg, e if borrow and s.rec_vec_elem(want) else None
            try:
                s.expect(e, want, scope, c, borrow_element=want == STR, borrow_vector=borrow)
            finally:
                s.read_arg = saved
            if not borrow and s.rec_vec_get(field_place(e)[0]):
                s.own(e)

    def copy_subview_arg(s, e, scope, c):
        """LG8: subview(a, lo, hi) as the only direct argument of the builtin copy; the array type is the source's"""
        if e.targs is not None:
            terr(e, "type_arity", "builtin subview takes no type arguments", "remove the `::<...>`")
        if len(e.args) != 3:
            terr(e, "arity", "subview takes 3 argument(s), got %d" % len(e.args))
        root = e.args[0]
        while isinstance(root, FieldAccess):
            root = root.base
        s.in_view = True
        try:
            at = s.expect_array(e.args[0], scope, c)   # a view nested in the source is reported where it is
            if not isinstance(root, Var):
                s.in_view = False
                s.view_context(e)
            if not is_array(at):
                terr(e.args[0], "type_mismatch", "expected %s, found %s" % (type_name(ARRAY), type_name(at)))
            s.expect(e.args[1], I64, scope, c)
            s.expect(e.args[2], I64, scope, c)
        finally:
            s.in_view = False
        e.ty = at
        return at

    def inout_args(s, e, f, m, scope, c):
        """arguments of a resolved call to a user function whose signature has an inout parameter (or that carries a
        marker): per argument marker/place validity and the instantiated type, then exclusive use of every marked
        binding across the whole argument list"""
        n = e.name
        marked = []
        for arg, p in zip(e.args, f.params):
            want = substitute(p.type, m)
            if p.mode is None:
                if isinstance(arg, InoutArg):
                    terr(arg, "inout_argument", "parameter %s of %s is not inout: the `inout` marker is not allowed here" % (
                        p.name, n), "remove `inout`, or declare the parameter `inout %s: ...`" % p.name)
                if f is CATALOG.get(n) and n in BORROWED_MAP_KEY and want == STRING:
                    s.check_text_key(arg, scope, c)
                else:
                    user_call = f is not CATALOG.get(n)
                    if user_call:
                        s.call_user_arg(arg, want, scope, c, f.foreign is None and not p.take)
                    else:
                        s.expect(arg, want, scope, c)
                if p.take and getattr(arg, "map_iter_view", False):
                    s.own(arg)
                continue
            if isinstance(arg, Call) and arg.name == "subview":
                s.view_context(arg)
            if not isinstance(arg, InoutArg):
                terr(arg, "inout_argument", "parameter %s of %s is inout: the argument needs the `inout` marker" % (p.name, n),
                     "write `inout name` with a `let mut` local array or an inout parameter")
            op = arg.expr
            user_call = f is not CATALOG.get(n)
            allow_path = user_call or n in MUTABLE_COLLECTION_MATCH + ("map_remove", "map_take", "vec_clear", "vec_reserve", "vec_extend_from_slice")
            place, path = field_place(op)
            if isinstance(place, Var):
                s.check_iteration_borrow(arg, place.name, path)
            if not isinstance(place, Var) or (path and not allow_path):
                what = ("a field" if isinstance(op, FieldAccess) else "an element" if isinstance(op, Index)
                        else "a call" if isinstance(op, Call) else "an expression")
                terr(arg, "inout_argument", "an inout argument must be the whole name of a local array or an inout "
                     "parameter, not %s" % what, "name a `let mut` local array or an inout parameter of this function")
            v = s.lookup(place, place.name, scope, path)
            if v[2] == "local" and not v[1]:
                terr(arg, "inout_argument", "%s is not mutable: it cannot be passed inout" % place.name,
                     "declare it with `let mut`")
            if v[2] not in ("inout", "local", "map_mut"):
                terr(arg, "inout_argument", "%s is a read-only %s: it cannot be passed inout" % (
                    place.name, "parameter" if v[2] == "param" else "binding"),
                     "declare the parameter `inout %s: ...`, or copy into a `let mut` local" % place.name)
            actual = s.expr(op, scope, c) if path else v[0]
            if actual != want:
                terr(arg, "inout_argument", "inout argument %s has type %s but parameter %s of %s expects %s" % (
                    place.name, type_name(actual), p.name, n, type_name(want)),
                     "pass an array of exactly the parameter's element type")
            take = not user_call and n == "map_take"
            # R-6e-14: a take checks map_get arms by place overlap instead of by root
            root_held = (s.active_roots.count(place.name) > s.map_read_roots.count(place.name) if take
                         else place.name in s.active_roots)
            read_held = take and any(r == place.name and _paths_overlap(tuple(path), held)
                                     for r, held in s.map_read_paths)
            if root_held or read_held or s.match_path_held(place.name, tuple(path)):     # G9, R-6e-8
                terr(arg, "recursive_borrow", "cannot pass %s inout while a match on it is active" % place.name,
                     "finish the match before the call")
            op.ty = actual
            arg.ty = actual
            marked.append((arg, place.name, path if user_call else ()))
        for arg, name, path in marked:
            if any(other == name and _paths_overlap(path, other_path)
                   for x in e.args if x is not arg
                   for other, other_path in _place_mentions(x)):
                terr(arg, "inout_alias", "%s is passed inout and also used elsewhere in the arguments of this call" % name,
                     "compute the other value first, e.g. `let n: i64 = len(%s);`, then pass n" % name)
        if not c and s.in_par and e is not s.boundary:
            terr(e, "inout_context", "a par branch cannot contain a nested inout call",
                 "evaluate the call in a separate statement before the par group")

    def word_call(s, e, scope, c):
        """P2 batch 4: load_be(a: [u8], i, n) -> i64 (code and contracts) and the statement-only
        store_be(inout a, i, w, n) -> (); n is an integer literal 1..=8 (a non-literal or out-of-range n is
        type/type_mismatch, a wrong argument count type/arity)"""
        n, a = e.name, e.args
        store = n in ("store_be", "store_le")
        k = 4 if store else 3
        if len(a) != k:
            terr(e, "arity", "%s takes %d argument(s), got %d" % (n, k, len(a)))
        if store:
            if c: terr(e, "effect_in_contract", "%s not allowed in contracts" % n)
            arg = a[0]
            if not isinstance(arg, InoutArg):
                terr(arg, "inout_argument", "the array of %s needs the `inout` marker" % n,
                     "write `%s(inout name, i, w, n)` with a `let mut` local array or an inout parameter" % n)
            op = arg.expr
            root, path = field_place(op)
            if not isinstance(root, Var):
                terr(arg, "inout_argument", "an inout argument must be the whole name of a local array or an inout "
                     "parameter", "name a `let mut` local array or an inout parameter of this function")
            if path:
                s.check_iteration_borrow(arg, root.name, path)
            v = s.lookup(root, root.name, scope, path) if path else s.lookup(op, op.name, scope)
            if v[2] == "local" and not v[1]:
                terr(arg, "inout_argument", "%s is not mutable: it cannot be passed inout" % root.name,
                     "declare it with `let mut`")
            if v[2] not in ("inout", "local") and not (path and v[2] == "map_mut"):
                terr(arg, "inout_argument", "%s is a read-only %s: it cannot be passed inout" % (
                    root.name, "parameter" if v[2] == "param" else "binding"),
                     "declare the parameter `inout %s: ...`, or copy into a `let mut` local" % root.name)
            ty = s.expr(op, scope, c) if path else v[0]
            if ty != ARRAY_U8:
                terr(arg, "type_mismatch", "expected %s, found %s" % (type_name(ARRAY_U8), type_name(ty)))
            if root.name in s.active_roots or s.match_path_held(root.name, path):
                terr(arg, "recursive_borrow", "cannot pass %s inout while a match on it is active" % root.name,
                     "finish the match before the call")
            op.ty = ty
            arg.ty = ty
            s.expect(a[1], I64, scope, c)
            s.expect(a[2], I64, scope, c)
            if any(name == root.name and _paths_overlap(path, other)
                   for x in a[1:3] for name, other in _place_mentions(x)) if path else \
                    any(_mentions(x, op.name) for x in a[1:]):
                terr(arg, "inout_alias", "%s is passed inout and also used elsewhere in the arguments of this call" % root.name,
                     "compute the other value first, e.g. `let n: i64 = len(%s);`, then pass n" % root.name)
            if e is not s.boundary:
                terr(e, "inout_context", "%s must be a whole expression statement" % n,
                     "write `%s(inout a, i, w, n);` as a statement" % n)
        else:
            s.expect(a[0], ARRAY_U8, scope, c)
            s.expect(a[1], I64, scope, c)
        w = a[-1]
        if not (isinstance(w, IntLit) and 1 <= w.value <= 8):
            terr(w, "type_mismatch", "the width argument of %s must be an integer literal 1..=8" % n,
                 "write the byte count as a literal such as 8")
        w.ty = I64
        return UNIT if store else I64

    def array_stmt_call(s, e, scope, c):
        """Check array statements with an exclusive destination place."""
        a = e.args
        n = e.name
        within = n == "copy_within"
        grow = n in ARRAY_GROW_BUILTINS
        append = n == "arr_append"
        argc = 4 if append or within else 2 if grow else 5
        if c: terr(e, "effect_in_contract", "%s is not allowed in contracts" % n)
        if grow and e is not s.statement_call:
            terr(e, "inout_context", "%s must be a whole expression statement" % n)
        if n == "arr_push" and 2 < len(a) <= ARR_PUSH_MAX + 1:
            argc = len(a)   # LB6 SX (R-6g5-5): arr_push(inout a, v1, .., vk) appends k values in one step
        if len(a) != argc:
            terr(e, "arity", "%s takes %s argument(s), got %d" % (
                n, "2 to %d" % (ARR_PUSH_MAX + 1) if n == "arr_push" else argc, len(a)))
        arg = a[0]
        if not isinstance(arg, InoutArg):
            fix = ("name a `let mut` local array or an inout parameter" if grow else
                   "write `%s(inout name, %s)` with a `let mut` local array or an inout parameter" %
                   (n, "dst, src, n" if within else "i, src, j, n"))
            terr(arg, "inout_argument", "the destination array of %s needs the `inout` marker" % n, fix)
        op = arg.expr
        root, path = field_place(op)
        if not isinstance(root, Var) or (within and path):    # LB6 IF: arr_reserve/arr_push take a field path too
            terr(arg, "inout_argument", "an inout argument must be the whole name of a local array or an inout "
                 "parameter", "name a `let mut` local array or an inout parameter of this function")
        v = s.lookup(root, root.name, scope, path)
        if v[2] == "local" and not v[1]:
            terr(arg, "inout_argument", "%s is not mutable: it cannot be passed inout" % root.name,
                 "declare it with `let mut`")
        if v[2] not in ("inout", "local") and not (path and v[2] == "map_mut"):
            terr(arg, "inout_argument", "%s is a read-only %s: it cannot be passed inout" % (
                root.name, "parameter" if v[2] == "param" else "binding"),
                 "declare the parameter `inout %s: ...`, or copy into a `let mut` local" % root.name)
        ty = s.expr(op, scope, c) if path else v[0]
        if grow and not append:
            if ty not in (ARRAY, ARRAY_U8, ARRAY_BOOL):
                terr(arg, "type_mismatch", "%s takes [i64], [u8] or [bool], found %s" % (n, type_name(ty)))
        elif ty not in (ARRAY, ARRAY_U8):
            terr(arg, "type_mismatch", "expected %s or %s, found %s" % (
                type_name(ARRAY), type_name(ARRAY_U8), type_name(ty)))
        if root.name in s.active_roots or s.match_path_held(root.name, path):
            terr(arg, "recursive_borrow", "cannot pass %s inout while a match on it is active" % root.name,
                 "finish the match before the call")
        if path:
            s.check_iteration_borrow(arg, root.name, path)
        op.ty = ty
        arg.ty = ty
        def byte_src(x):    # LB6 SB (R-6g5-1): a str (or a literal, lent statically) is the byte source of a [u8] copy
            if ty == ARRAY_U8 and isinstance(x, StrLit):
                x.ty = ARRAY_U8
                x.static_str = True
                return ARRAY_U8
            got = s.expr(x, scope, c)
            return ARRAY_U8 if ty == ARRAY_U8 and got == STR else got
        if append:
            src = byte_src(a[1])
            if src != ty and not (ty == ARRAY and src == ARRAY_U8):
                terr(a[1], "type_mismatch", "arr_append cannot append %s to %s" % (type_name(src), type_name(ty)))
            s.expect(a[2], I64, scope, c)
            s.expect(a[3], I64, scope, c)
        else:
            for v in (a[1:] if n == "arr_push" else a[1:2]):
                s.expect(v, s.elem_type(ty) if n == "arr_push" else I64, scope, c)
        if not grow:
            if within:
                s.expect(a[2], I64, scope, c)
            elif byte_src(a[2]) != ty:
                terr(a[2], "type_mismatch", "expected %s, found %s" % (type_name(ty), type_name(a[2].ty)))
            s.expect(a[3], I64, scope, c)
            if not within:
                s.expect(a[4], I64, scope, c)
        others = a[1:2] + a[3:] if not grow and not within and same_place(op, a[2]) else a[1:]
        if any(name == root.name and _paths_overlap(path, other)
               for x in others for name, other in _place_mentions(x)):
            terr(arg, "inout_alias", "%s is passed inout and also used elsewhere in the arguments of this call" % root.name,
                 "compute the value before the call" if grow else
                 "compute the index arguments before the call" if within else
                 "copy the source first with `let mut t: [..] = copy(%s);` (needs alloc), then pass t" %
                 ".".join((root.name,) + path))
        if not grow and e is not s.boundary:
            terr(e, "inout_context", "%s must be a whole expression statement" % n,
                 "write `%s(%s);` as a statement" %
                 (n, "inout a, dst, src, n" if within else "inout dst, i, src, j, n"))
        if grow:
            s.used_effects.add("alloc")
        return UNIT

    def old_call(s, e, scope):
        """the contextual builtin old(a): context first, then arity and operand"""
        if s.ckind not in ("ensures", "invariant", "assert"):
            terr(e, "old_context", "old(..) is only allowed in ensures, loop invariants and asserts",
                 "state the property in a contract clause; in code use copy(a) (needs alloc) if the entry value is needed")
        a = e.args
        if len(a) != 1:
            terr(e, "old_argument", "old takes exactly 1 argument, got %d" % len(a),
                 "write old(a) with the name of an inout parameter of this function")
        arg = a[0]
        # LG23 (G-LG22-2): old(len(P)) for a place chain P is lowered to len(old(root)...)
        len_op = (isinstance(arg, Call) and arg.name == "len" and len(arg.args) == 1
                  and getattr(arg, "targs", None) is None)
        place = arg.args[0] if len_op else arg
        steps, root = [], place
        while isinstance(root, (FieldAccess, Index)):
            steps.append(root)
            root = root.base if isinstance(root, FieldAccess) else root.array
        ok = isinstance(root, Var) and root.name in scope and scope[root.name][2] == "inout"
        if ok:
            rty = scope[root.name][0]
            # a field step right on an array root is not a place (as pruvc-rs)
            ok = is_nongeneric_struct(rty, s.decls) or (
                is_array(rty) and not (steps and isinstance(steps[-1], FieldAccess))) or (
                rty in (I64, BOOL) and not steps and not len_op)     # LB6 OS (R-6g5-4): a scalar inout
        if not ok:
            terr(e, "old_argument", "the operand of old must be the name of an inout parameter of this function",
                 "write old(a) where `inout a: [..]` is a parameter of the enclosing function")
        inouts = {p.name for p in s.f.params if p.mode == "inout"}
        for st in steps:
            if isinstance(st, Index) and any(
                    isinstance(x, (Var, Move)) and x.name in inouts or isinstance(x, Call) and x.name == "old"
                    for x in _walk(st.index)):
                terr(e, "old_argument", "the index of an old operand cannot read an inout parameter; "
                     "write old(a)[old(b)[0]] explicitly")
        root.ty = rty
        old = {"param": root.name, "index": [p.name for p in s.f.params].index(root.name)}
        if not steps and not len_op:
            e.old = old
            return rty
        # G-DB31-2: old(x.f1...fn) is lowered to the field chain old(x).f1...fn
        # the field nodes keep their source positions; the innermost base becomes old(x) at the position of old,
        # and the chain is typed as an ordinary read (as pruvc-rs does)
        inner = Call(e.line, e.col, "old", [root], rty)
        inner.file = e.file
        if steps:
            if isinstance(steps[-1], FieldAccess):
                steps[-1].base = inner
            else:
                steps[-1].array = inner
        else:
            place = inner
            if len_op:
                arg.args[0] = inner
        cur = s.expr(place, scope, True)
        if len_op:
            if not is_array(cur):
                terr(e, "old_argument", "the operand of old must be the name of an inout parameter of this function",
                     "write old(a) where `inout a: [..]` is a parameter of the enclosing function")
            cur = s.expr(arg, scope, True)
            node = arg
        else:
            node = place
        # the outermost node replaces this Call in place (the caller stores the type on it)
        e.__class__ = node.__class__
        e.__dict__ = vars(node).copy()
        return cur

    # ---- P06: expanding user functions in contracts (docs/L3_RESTRUCTURE_SPEC.md appendix P06) ----
    def x_prepare(s):
        """the reason each function cannot be expanded in a contract, or a raw copy of its body when its signature
        and statements allow it (checked before any body is typed, so every template starts from source)"""
        for f in s.prog.functions:
            read_borrowing = not f.opaque and any(s.borrowed_type(p.type) for p in f.params)
            r = ("has a borrowed result" if f.borrow_from else
                 x_sig_reason(f, s.decls, allow_requires=read_borrowing,
                              allow_str=s.prog.borrow_site is not None and not f.opaque)
                 or x_scan_block(f, f.body, 0, s.funcs))
            s.x_raw[f.name] = r if r is not None else copy.deepcopy(f.body)
            if f.pred:                        # G-LG13-5: a `?` in a pred is try_position before pred_signature
                for st in f.body:
                    if isinstance(st, Return) and st.value is not None:
                        check_try_position(st.value, st)
            if f.pred and r is not None:      # R03L (R-6-2): a pred always expands
                terr(f, "pred_signature", "pred %s must expand in a contract: %s" % (f.name, r), X_FIX)
            if f.opaque and (r is not None or f.ret != BOOL):   # P13o: an opaque function is an expandable predicate
                terr(f, "opaque_signature", "opaque function %s %s" % (
                    f.name, "must return bool" if r is None else "must expand in a contract: " + r),
                     "keep `opaque` for a bool predicate that a contract may expand (" + X_FIX + ")")

    def x_auto(s):
        """P07 (docs/L3_RESTRUCTURE_SPEC.md appendix P07): every function whose body expands as a contract keeps its
        template as f.x_auto, so the verifier can assume result == expansion at each code call. A function whose
        template cannot be built (not expandable, too large, calls a function that cannot be expanded) gets None;
        that is never an error, the function simply keeps its written contract only. An i64 function whose template
        uses a bit builtin also gets None: its value would put a bit-vector term into every later query of the caller
        (the P2 kernels time out), while a bool predicate only adds its case split."""
        for f in s.prog.functions:
            f.x_auto = None
            if isinstance(s.x_raw.get(f.name), str) or f.name not in s.x_raw:
                continue
            s.f = f
            s.enter(fn_owner(f.name), f.tparams)
            s.ckind = "ensures"
            try:
                xa = s.x_template(f, f)
                f.x_auto = xa if f.ret == BOOL or not x_has_bit(xa) else None
                if f.pred or not f.opaque and (x_has_quant(xa) or any(
                        x_record(p.type, s.decls) and not s.borrowed_type(p.type) for p in f.params)):
                    f.x_auto = None     # R03L: a record or quantified predicate gives contracts only, no code fact
            except PruvError:
                s.x_busy = set()
                if f.opaque:        # P13o: an opaque body is its definition, so it must build
                    raise
            s.ckind = "code"

    def x_expand(s, e, f):
        """the expansion of the checked contract call e of the user function f"""
        if not isinstance(s.x_raw[f.name], str):
            reason = x_sig_reason(f, s.decls)
            if reason is not None:
                terr(e, "call_in_contract", X_MSG % (f.name, reason), X_FIX)
        return x_instantiate(s.x_template(e, f), f, e)

    def x_template(s, e, f):
        n = f.name
        t = s.x_memo.get(n)
        if t is not None:
            return t
        raw = s.x_raw[n]
        if isinstance(raw, str):
            terr(e, "call_in_contract", X_MSG % (n, raw), X_FIX)
        if n in s.x_busy:
            terr(e, "call_in_contract", X_MSG % (n, "it is recursive"), X_FIX)
        s.x_busy.add(n)
        scope = {p.name: (p.type, False, "param") for p in f.params}
        if isinstance(s.f, StructDecl):
            raw = copy.deepcopy(raw)
        t = s.x_walk(e, f, raw, 0, scope, {})
        s.x_busy.discard(n)
        s.x_memo[n] = t
        return t

    def x_walk(s, site, f, stmts, i, scope, env):
        """type the statements from stmts[i] in contract mode and fold them into one expression over the parameters"""
        while True:
            st = stmts[i]
            if isinstance(st, Let):
                if st.name in scope:
                    terr(st, "shadow", "%s is already defined; shadowing is not allowed" % st.name, "use a new name")
                s.expect(st.value, st.type, scope, True)
                env[st.name] = x_sized(site, f, x_subst(st.value, env))
                scope[st.name] = (st.type, False, "local")
            elif isinstance(st, Return):
                s.expect(st.value, f.ret, scope, True)
                return x_sized(site, f, x_subst(st.value, env))
            else:
                s.expect(st.cond, BOOL, scope, True, op="a condition")
                cond = x_sized(site, f, x_subst(st.cond, env))
                t = s.x_walk(site, f, st.then, 0, dict(scope), dict(env))
                if st.els:
                    el = s.x_walk(site, f, st.els, 0, dict(scope), dict(env))
                else:
                    el = s.x_walk(site, f, stmts, i + 1, dict(scope), dict(env))
                return x_sized(site, f, x_join(st, cond, t, el))
            i += 1

    def call(s, e, scope, c, borrow_element):
        n, a = e.name, e.args

        def arity(k):
            if len(a) != k:
                terr(e, "arity", "%s takes %d argument(s), got %d" % (n, k, len(a)))
        if n == "subview":
            s.view_context(e)
        for arg in a:
            if isinstance(arg, InoutArg):
                arg = arg.expr
            elif n in s.funcs:
                continue
            if isinstance(arg, Call) and arg.name == "subview":
                if n == "copy" and n not in s.funcs and len(a) == 1 and not c and arg is a[0] and not s.in_view:
                    continue   # LG8: copy(subview(a, lo, hi))
                if n in TEXT_VIEW_OPS and n not in s.funcs and arg is a[-1] and not c and not s.in_view:
                    continue   # LB7 (R-6g6-3): string_push_str(move s, subview(t, lo, hi))
                s.view_context(arg)
        is_old = n == "old" and s.uses_inout and n not in s.funcs   # G5 A0: contextual builtin
        if e.targs is not None and n not in s.funcs and (n in BUILTIN_FNS or n in CONTRACT_ONLY or is_old):
            terr(e, "type_arity", "builtin %s takes no type arguments" % n, "remove the `::<...>`")
        if is_old:
            return s.old_call(e, scope)
        if n == "len":
            arity(1); s.expect_array(a[0], scope, c, coll=True, borrow_element=True); return I64   # G6: also Vec/HashMap/String
        if n == "alloc":
            if c: terr(e, "effect_in_contract", "alloc not allowed in contracts")
            arity(2); s.expect(a[0], I64, scope, c)
            h = s.alloc_hint
            # G2 A5: the fill value is bool when the hinted type is [bool]; G3: T when it is [T]
            s.expect(a[1], s.elem_type(h[1]) if h is not None and h[0] is e else I64, scope, c)
            s.used_effects.add("alloc")
            return h[1] if h is not None and h[0] is e else ARRAY
        if n == "copy":
            if c: terr(e, "effect_in_contract", "copy not allowed in contracts")
            arity(1)
            if isinstance(a[0], Call) and a[0].name == "subview":
                at = s.copy_subview_arg(a[0], scope, c)   # LG8
            else:
                at = s.expect_array(a[0], scope, c)
            s.used_effects.add("alloc"); return at
        if n == "truncate":   # the first m elements of an array as a new array (moves the array at its last use)
            if c: terr(e, "effect_in_contract", "truncate not allowed in contracts")
            arity(2); at = s.expect_array(a[0], scope, c)
            if at == STR:
                terr(a[0], "type_mismatch", "truncate takes an array, found str", "use substr(s, 0, m) for a str")
            s.expect(a[1], I64, scope, c)
            s.used_effects.add("alloc"); return at
        if n == "resize":   # P05G: a copy of a grown to length n, the new tail set to v (no prefix loop, one allocation)
            if c: terr(e, "effect_in_contract", "resize not allowed in contracts")
            arity(3); at = s.expect_array(a[0], scope, c)
            if at == STR:
                terr(a[0], "type_mismatch", "resize takes an array, found str", "build a String with push for a str")
            if at.args:
                terr(a[0], "type_mismatch", "resize takes [i64], [u8] or [bool], found %s" % type_name(at),
                     "use alloc and a copy loop for other element types")
            s.expect(a[1], I64, scope, c); s.expect(a[2], s.elem_type(at), scope, c)
            s.used_effects.add("alloc"); return at
        if n == "is_char_boundary":   # G2 B0
            arity(2); s.expect(a[0], STR, scope, c); s.expect(a[1], I64, scope, c)
            return BOOL
        if n in BIT_BUILTINS:   # P2: pure i64 bit operation, allowed in code and contracts
            arity(2); s.expect(a[0], I64, scope, c); s.expect(a[1], I64, scope, c)
            return I64
        if n in WORD_BUILTINS:  # P2 batch 4
            return s.word_call(e, scope, c)
        if n in ("copy_range", "copy_within") + ARRAY_GROW_BUILTINS:
            return s.array_stmt_call(e, scope, c)
        if n == "eq_range":     # L3-10: eq_range(a, i, b, j, n) -> bool, code only, a and b of one integer array type
            if c: terr(e, "effect_in_contract", "eq_range is not allowed in contracts")
            arity(5)
            ta = s.expect_array(a[0], scope, c)
            if ta not in (ARRAY, ARRAY_U8):
                terr(a[0], "type_mismatch", "expected %s or %s, found %s" % (
                    type_name(ARRAY), type_name(ARRAY_U8), type_name(ta)))
            s.expect(a[1], I64, scope, c)
            s.expect(a[2], ta, scope, c)
            s.expect(a[3], I64, scope, c)
            s.expect(a[4], I64, scope, c)
            return BOOL
        if n == "find_byte":    # LB6 MC (R-6g5-3): find_byte(a, b, lo, hi) -> i64, code only, a is [u8] or str
            if c: terr(e, "effect_in_contract", "find_byte is not allowed in contracts")
            arity(4)
            if isinstance(a[0], StrLit):  # a literal is borrowed as its static bytes, no alloc
                a[0].ty = ARRAY_U8; a[0].static_str = True; ta = ARRAY_U8
            else:
                ta = s.expr(a[0], scope, c)
            if ta not in (ARRAY_U8, STR):
                terr(a[0], "type_mismatch", "expected %s or %s, found %s" % (
                    type_name(ARRAY_U8), type_name(STR), type_name(ta)))
            s.expect(a[1], I64, scope, c)
            s.expect(a[2], I64, scope, c)
            s.expect(a[3], I64, scope, c)
            return I64
        if n in ("utf8_valid", "str_from_utf8"):
            if c and n == "str_from_utf8":
                terr(e, "effect_in_contract", "str_from_utf8 not allowed in contracts")
            arity(1); s.expect(a[0], ARRAY_U8, scope, c)
            if n == "utf8_valid":
                return BOOL
            s.used_effects.add("alloc"); return STR
        if n == "utf8_valid_span":      # LG26 (R-6b-3, G-LG25-1): contract only, b[lo..hi] is valid UTF-8
            if not c or s.ckind == "cost":
                terr(e, "contract_only", "utf8_valid_span is only allowed in contracts")
            arity(3); s.expect(a[0], ARRAY_U8, scope, c); s.expect(a[1], I64, scope, c); s.expect(a[2], I64, scope, c)
            return BOOL
        if n == "substr":   # G2 B0
            if c: terr(e, "effect_in_contract", "substr not allowed in contracts")
            arity(3); s.expect(a[0], STR, scope, c); s.expect(a[1], I64, scope, c); s.expect(a[2], I64, scope, c)
            s.used_effects.add("alloc"); return STR
        if n == "print":
            if c: terr(e, "effect_in_contract", "print not allowed in contracts")
            arity(1)
            t = s.expr(a[0], scope, c)
            if t.owner is not None:
                terr(e, "generic_operation", "print cannot take the abstract type %s" % t.name,
                     "print takes a concrete i64 or bool")
            if t not in (I64, BOOL):
                terr(e, "print_type", "print takes i64 or bool")
            s.used_effects.add("io"); return UNIT
        if n in CONTRACT_ONLY:
            if not c:
                terr(e, "contract_only", "%s is only allowed in contracts" % n,
                     "size is a contract measure; use len or a counter in code" if n == "size" else "use match in code")
            arity(1)
            kind, rt = CONTRACT_ONLY[n]
            t = s.expr(a[0], scope, c)
            if n == "size":     # G9 A0: a non-generic user struct/enum value
                if not (s.is_plain_nominal(t) and not s.decls[t.name].tparams):
                    terr(e, "size_type", "size needs a non-generic struct or enum value, got %s" % type_name(t),
                         "size counts constructors; use len for arrays and strings")
                return I64
            if t.owner is not None or t.name != kind:
                terr(a[0], "type_mismatch", "expected %s, found %s" % (type_name(OPTION if kind == "option" else RESULT),
                                                                         type_name(t)))
            if rt == "payload":     # G3: the payload type of Option<X> / Result<X>
                if t.args and t.args[0] == UNIT:   # LB8 R-6g7-1: the unit payload carries nothing to read
                    terr(e, "type_mismatch", "ok_val of %s has no value" % type_name(t), "use is_ok")
                return t.args[0] if t.args else I64
            if rt == "error":
                return result_error(t)
            return rt
        catalog = n in CATALOG and n not in s.funcs       # G6: fixed standard catalog (G6_SPEC 10.3)
        if catalog:
            if n in ("vec_clear", "vec_reserve", "vec_extend_from_slice") and e is not s.statement_call and not c:
                terr(e, "inout_context", "%s must be a whole expression statement" % n)
            if n in MUTABLE_COLLECTION_MATCH + ("map_take",) and e is not s.match_call:
                terr(e, "foreign_contract", "%s is only allowed as a direct match scrutinee" % n)
            if n in CATALOG_CONTRACT_ONLY:
                if not c:
                    terr(e, "foreign_contract", "%s is only allowed in contracts" % n,
                         "use map_get in code" if n == "map_lookup" else "state the update in ensures")
            elif c and n not in FILE_OBSERVERS and n not in PROC_OBSERVERS and n not in NET_OBSERVERS and n not in ASYNC_OBSERVERS:
                terr(e, "foreign_contract", "%s cannot be called in a contract" % n,
                     "contracts observe collections with len(x), x[i], map_lookup and map_updated")
            f = CATALOG[n]
        elif n not in s.funcs:
            if n in ("sum", "count", "max", "min"):
                terr(e, "unknown_fn", "unknown function %s" % n,
                     "aggregates use the form sum(k in 0..len(a): a[k])")
            terr(e, "unknown_fn", "unknown function %s" % n)
        else:
            f = s.funcs[n]
            if c and f.foreign is not None:
                terr(e, "foreign_contract", "trusted rust fn %s cannot be called in a contract" % n,
                     "state the property with the arguments' own contents instead")
        # G3_SPEC 3.4: resolve + arity + admissibility + bounds, substitute the signature, check the arguments
        m = s.instantiate(e, "function %s" % n, fn_owner(n), f.tparams, e.targs)
        if m is not None:
            e.inst = {"decl": fn_owner(n), "args": tuple(e.targs), "subst": m}
            if f is s.funcs.get(n) and f.tparams and m:
                req, ens = s.struct_contracts(f, m, e)      # LG24: invariants of the instantiated type arguments
                if not s.parametric_safe(f, m):
                    # LG24: unwalked payloads; keep the uninstantiated ensures (built here: a callee declared
                    # later has no inv_ensures yet)
                    ens = s.struct_contracts(f, None, e)[1]
                if req or ens:
                    e.inv_inst = (req, ens)
        else:
            m = {}
        arity(len(f.params))
        if (catalog and n == "vec_get" and s.rec_vec_elem(e.targs[0])
                and all(e is not read for read in (s.field_receiver, s.match_call, s.read_arg))):
            terr(e, "foreign_owned_type", "vec_get cannot copy %s" % type_name(e.targs[0]))
        if catalog and n == "string_eq":
            for arg in a:
                if isinstance(arg, StrLit):
                    arg.ty = STR
                    continue
                got = s.expr(arg, scope, c, borrow_element=True)
                if got not in (STRING, STR, ARRAY_U8) or isinstance(arg, Move):
                    terr(arg, "type_mismatch", "expected String, str or [u8], found %s" % type_name(got))
        elif n in CATALOG_CONTRACT_ONLY:
            s.observer_args(e, f, m, scope, c)
        elif any(p.mode is not None for p in f.params) or not catalog and any(isinstance(x, InoutArg) for x in a):
            s.inout_args(e, f, m, scope, c)     # G5 A0: marker, place, type and same-call exclusivity
        else:
            for arg, p in zip(a, f.params):
                want = substitute(p.type, m)
                if catalog and n in BORROWED_MAP_KEY and want == STRING:
                    s.check_text_key(arg, scope, c)
                elif catalog and n in TEXT_VIEW_OPS and want == STR and isinstance(arg, Call) and arg.name == "subview":
                    s.user_arg(arg, want, scope, c, True)   # LB7 (R-6g6-3): a borrowed str range, no copy
                else:
                    if catalog:
                        s.expect(arg, want, scope, c)
                    else:
                        s.call_user_arg(arg, want, scope, c, f.foreign is None and not p.take)
                if p.take and getattr(arg, "map_iter_view", False):
                    s.own(arg)
        if catalog:
            if n == "string_to_str" and isinstance(a[0], Var) and scope[a[0].name][2] == "result_error":
                e.take_result_error = True
            if n == "vec_get" and e.targs[0] == STRING:
                if not borrow_element:
                    terr(e, "foreign_owned_type", "a Vec<String> element must be a direct borrowed text argument")
                if not isinstance(field_place(a[0])[0], Var):
                    terr(a[0], "foreign_borrow", "a borrowed Vec<String> element needs a vector place")
                return STR
            if n == "vec_at_mut" and e.targs[0].name not in s.aggregates and not s.rec_vec_elem(e.targs[0]):
                terr(e, "foreign_owned_type", "vec_at_mut requires an owned aggregate element")
            if n == "map_remove" and e.targs[0] not in (I64, STRING):
                terr(e, "foreign_owned_type", "map_remove requires i64 or String keys")
            if n in ("vec_push", "map_insert") and e.targs[0] == STRING:
                arg = a[1]
                if not (isinstance(arg, Call) or isinstance(arg, Move) and not arg.fields):
                    terr(arg, "foreign_move", "an owned String argument needs a call result or move")
            if (n in ("vec_get", "map_get") and s.is_extended(e.targs[-1])
                    and not (n == "vec_get" and s.rec_vec_elem(e.targs[-1]))):
                if e is not (s.field_receiver if n == "vec_get" else s.match_call):
                    terr(e, "foreign_owned_type", "%s cannot copy %s" % (n, type_name(e.targs[-1])))
            if n in ("vec_push", "map_insert"):
                x = a[-1]
                if getattr(x, "map_iter_view", False):
                    s.own(x)
                payload = e.targs[-1]
                rec_push = n == "vec_push" and s.rec_vec_elem(payload)
                if rec_push:
                    s.own(x)
                if getattr(x, "map_mut_view", False) or (
                        (rec_push or payload.owner is None and payload.name in s.aggregates)
                        and not (isinstance(x, (StructLit, EnumCtor, Call)) or isinstance(x, Move) and not x.fields
                                 and scope[x.name][2] in ("local", "take"))):
                    terr(x, "foreign_move", "an aggregate collection payload needs a constructor, call result or move")
            # Includes fs for G10 operations, through the same path as alloc.
            s.used_effects.update(f.effects)
            return substitute(f.ret, m)
        for arg, p in zip(a, f.params):
            if p.take and arg.borrowed_view:
                terr(arg, "array_alias", "a borrowed view cannot be passed to a consuming parameter")
            if p.take and is_array(substitute(p.type, m)) and not isinstance(arg, Move):
                terr(arg, "foreign_move", "missing move in a consuming argument",
                     "write move %s" % arg.name if isinstance(arg, Var) else "bind the array to a local, then move it")
            if p.take and substitute(p.type, m).name in s.aggregates:
                if not (isinstance(arg, (StructLit, Call)) or isinstance(arg, Move) and not arg.fields
                        and scope[arg.name][2] in ("local", "take")):
                    terr(arg, "foreign_move", "a take struct argument needs a constructor, call result or move")
        if not c and f.pred:    # R03L (R-6-2)
            terr(e, "pred_in_code", "pred %s is only allowed in contracts" % n,
                 "use it in requires, ensures, invariant or assert, or write a fn for code")
        if c and f.effects:
            terr(e, "effect_in_contract", "function %s has effects; not allowed in contracts" % n)
        if c:
            # Contract expansion can erase the call before backend validation.
            if isinstance(s.f, Function) and getattr(s.prog, "contract_vec_slice", None) is None and any(
                    is_vec_slice_arg(arg.ty, substitute(p.type, m)) for arg, p in zip(a, f.params)):
                s.prog.contract_vec_slice = (e.line, e.col, getattr(s.f, "file", None))
            if s.ckind == "cost":
                terr(e, "call_in_contract", "calling functions in contracts is not supported in v0.1",
                     "inline the property")
            if f.opaque:        # P13o: an opaque call stays a call; the verifier applies its uninterpreted symbol
                e.opaque_c = True   # code generation inlines it (opaque_inline)
                return substitute(f.ret, m)
            s.x_pending = s.x_expand(e, f)    # P06: expand the call (docs/L3_RESTRUCTURE_SPEC.md appendix P06)
            return substitute(f.ret, m)
        s.used_effects.update(f.effects)
        s.calls[s.f.name].add(n)
        if f.foreign is None:     # G9 A0: foreign calls are leaves of the call graph
            s.call_log.append((s.f, e, f))
        return substitute(f.ret, m)


# ---- P06: contract expansion of user functions ----
X_MSG = "function %s cannot be expanded in a contract: %s"
X_FIX = ("keep it loop-free with no requires (immutable lets, if/return over i64, bool and array parameters), "
         "or inline the property")
X_MAX = 4096          # nodes in one template or one expansion
X_PARAM_TYPES = (I64, BOOL, ARRAY, ARRAY_U8, ARRAY_BOOL)
X_R_STMT = "its body has a statement other than an immutable i64 or bool let, if and return"
X_R_EXPR = ("its body has an expression other than literals, names, operators, indexing, field reads, quantifiers, "
            "aggregates and calls")


def x_record(t, decls):
    """R03L: a non-generic struct type, the record parameter of a named predicate"""
    return is_user_type(t) and not t.args and isinstance(decls.get(t.name), StructDecl)


def x_sig_reason(f, decls, allow_requires=False, allow_str=False):
    if f.tparams:
        return "it is generic"
    if any(p.mode is not None or p.take for p in f.params):
        return "it has an inout or take parameter"
    if any(p.type not in X_PARAM_TYPES and not (allow_str and p.type == STR)
           and not x_record(p.type, decls) for p in f.params):
        return "its parameters are not all i64, bool, arrays of i64, u8 or bool, or structs"
    if f.ret not in (I64, BOOL):
        return "it does not return bool or i64"
    if f.requires and not allow_requires:
        return "it has requires"
    if f.decreases is not None:
        return "it has decreases"
    if f.cost is not None:
        return "it has a cost clause"
    return None


def x_scan_block(f, stmts, i, funcs):
    while i < len(stmts):
        st = stmts[i]
        if isinstance(st, Let):
            if st.mutable or st.refine is not None or st.type not in (I64, BOOL):
                return X_R_STMT
            r = x_scan_expr(st.value, funcs)
            if r is not None:
                return r
        elif isinstance(st, Return):
            if st.value is None:
                return X_R_STMT
            return x_scan_expr(st.value, funcs)
        elif isinstance(st, If):
            if f.ret != BOOL:
                return "it returns i64 through an if (only bool functions may branch)"
            r = x_scan_expr(st.cond, funcs)
            if r is None:
                r = x_scan_block(f, st.then, 0, funcs)
            if r is not None:
                return r
            return x_scan_block(f, st.els, 0, funcs) if st.els else x_scan_block(f, stmts, i + 1, funcs)
        else:
            return X_R_STMT
        i += 1
    return "an if branch does not end in return"


def x_scan_expr(e, funcs):
    if isinstance(e, (IntLit, BoolLit, Var)):
        return None
    if isinstance(e, Unary):
        return x_scan_expr(e.operand, funcs)
    if isinstance(e, Binary):
        r = x_scan_expr(e.left, funcs)
        return r if r is not None else x_scan_expr(e.right, funcs)
    if isinstance(e, Index):
        r = x_scan_expr(e.array, funcs)
        return r if r is not None else x_scan_expr(e.index, funcs)
    if isinstance(e, FieldAccess):      # R03L: a field read of a struct parameter
        return x_scan_expr(e.base, funcs)
    if isinstance(e, Quant):     # R03L: a quantified fact or an aggregate (sum, count, max, min)
        for k in [e.lo, e.hi, e.body] + list(e.triggers):
            r = x_scan_expr(k, funcs)
            if r is not None:
                return r
        return None
    if isinstance(e, Call):
        if e.name in funcs or e.name == "len" or e.name in BIT_BUILTINS or e.name in ("load_be", "load_le", "utf8_valid", "utf8_valid_span"):
            f = funcs.get(e.name)
            for i, a in enumerate(e.args):
                if (isinstance(a, StrLit) and f is not None and f.foreign is None and i < len(f.params)
                        and f.params[i].mode is None and not f.params[i].take and f.params[i].type == ARRAY_U8):
                    continue
                r = x_scan_expr(a, funcs)
                if r is not None:
                    return r
            return None
        return "it calls %s" % e.name
    return X_R_EXPR


def x_size(e):
    if isinstance(e, Unary):
        return 1 + x_size(e.operand)
    if isinstance(e, Binary):
        return 1 + x_size(e.left) + x_size(e.right)
    if isinstance(e, Index):
        return 1 + x_size(e.array) + x_size(e.index)
    if isinstance(e, Call):
        return 1 + sum(x_size(a) for a in e.args)
    if isinstance(e, FieldAccess):
        return 1 + x_size(e.base)
    if isinstance(e, Quant):
        return 1 + x_size(e.lo) + x_size(e.hi) + x_size(e.body) + sum(x_size(t) for t in e.triggers)
    return 1


def x_sized(site, f, e):
    if x_size(e) > X_MAX:
        terr(site, "call_in_contract", X_MSG % (f.name, "its expansion is too large"), X_FIX)
    return e


def x_map(e, leaf, bound=frozenset()):
    """a fresh copy of the expansion expression e; leaf(Var) gives the replacement of a free name or None to copy it
    (R03L: a name bound by a quantifier inside e is never replaced)"""
    if isinstance(e, Var) and e.name not in bound:
        r = leaf(e)
        if r is not None:
            return r
    n = copy.copy(e)
    if isinstance(e, Unary):
        n.operand = x_map(e.operand, leaf, bound)
    elif isinstance(e, Binary):
        n.left = x_map(e.left, leaf, bound)
        n.right = x_map(e.right, leaf, bound)
    elif isinstance(e, Index):
        n.array = x_map(e.array, leaf, bound)
        n.index = x_map(e.index, leaf, bound)
    elif isinstance(e, Call):
        n.args = [x_map(a, leaf, bound) for a in e.args]
    elif isinstance(e, FieldAccess):
        n.base = x_map(e.base, leaf, bound)
    elif isinstance(e, Quant):
        n.lo, n.hi = x_map(e.lo, leaf, bound), x_map(e.hi, leaf, bound)
        n.body = x_map(e.body, leaf, bound | {e.var})
        n.triggers = [x_map(t, leaf, bound | {e.var}) for t in e.triggers]
    return n


def x_subst(e, env):
    """e with every let name replaced by its folded value"""
    return x_map(e, lambda v: x_map(env[v.name], lambda _: None) if v.name in env else None)


def x_join(st, c, t, el):
    """if c { t } else { el } over bool values, short-circuit form"""
    def mk(cls, *a):
        n = cls(st.line, st.col, *a, ty=BOOL)
        if st.file is not None:
            n.file = st.file
        return n
    if isinstance(t, BoolLit) and not t.value:
        return mk(Binary, "&&", mk(Unary, "!", c), el)
    if isinstance(t, BoolLit) and t.value:
        return mk(Binary, "||", c, el)
    if isinstance(el, BoolLit) and not el.value:
        return mk(Binary, "&&", c, t)
    if isinstance(el, BoolLit) and el.value:
        return mk(Binary, "||", mk(Unary, "!", c), t)
    return mk(Binary, "||", mk(Binary, "&&", c, t), mk(Binary, "&&", mk(Unary, "!", x_map(c, lambda _: None)), el))


def x_instantiate(tpl, f, site):
    """the template with each parameter replaced by a copy of the checked argument; every template node takes the
    call site's position (the arguments keep theirs)"""
    args = {p.name: a for p, a in zip(f.params, site.args)}

    def at(e, bound=frozenset()):
        if isinstance(e, Var) and e.name in args and e.name not in bound:
            return x_map(args[e.name], lambda _: None)
        n = copy.copy(e)
        n.line, n.col = site.line, site.col
        if site.file is not None or n.file is not None:
            n.file = site.file
        if isinstance(e, Unary):
            n.operand = at(e.operand, bound)
        elif isinstance(e, Binary):
            n.left, n.right = at(e.left, bound), at(e.right, bound)
        elif isinstance(e, Index):
            n.array, n.index = at(e.array, bound), at(e.index, bound)
        elif isinstance(e, Call):
            n.args = [at(a, bound) for a in e.args]
        elif isinstance(e, FieldAccess):
            n.base = at(e.base, bound)
        elif isinstance(e, Quant):
            if e.var in free:   # R03L: the bound name would capture a name of an argument
                terr(site, "call_in_contract", X_MSG % (f.name, "an argument uses the name %s that its body binds in "
                     "%s" % (e.var, e.kind)), "rename %s in the argument or in the body of %s" % (e.var, f.name))
            n.lo, n.hi, n.body = at(e.lo, bound), at(e.hi, bound), at(e.body, bound | {e.var})
            n.triggers = [at(t, bound | {e.var}) for t in e.triggers]
        return n
    free = set()
    for a in site.args:
        x_names(a, free)
    return x_sized(site, f, at(tpl))


def x_names(e, out):
    """R03L: every name read in e (bound names included: a superset of its free names is enough to rule out capture)"""
    if isinstance(e, Var):
        out.add(e.name)
    for k in expr_kids(e):
        x_names(k, out)


def opaque_inline(prog):
    """P13o (appendix P13o.1): after verification, every contract application of an opaque function becomes its
    expansion, so the code generators see what P06 would have produced (a run-time contract check evaluates the
    body). Idempotent: an inlined program has no opaque contract call left."""
    funcs = {f.name: f for f in prog.functions}
    if not any(f.opaque for f in prog.functions):
        return
    seen = set()

    def inline(c):
        f = funcs[c.name]
        args = {p.name: walk_e(a) for p, a in zip(f.params, c.args)}
        return walk_e(x_map(f.x_auto, lambda v: x_map(args[v.name], lambda _: None) if v.name in args else None))

    def walk_e(e):
        if isinstance(e, Call) and getattr(e, "opaque_c", False):
            return inline(e)
        walk(e)
        return e

    def walk(o):
        if id(o) in seen or isinstance(o, (Type, Function)):
            return
        seen.add(id(o))
        for k, v in list(vars(o).items()):
            if k in ("x_auto", "x_raw", "ty"):
                continue
            if isinstance(v, Expr):
                setattr(o, k, walk_e(v))
            elif isinstance(v, list):
                for i, x in enumerate(v):
                    if isinstance(x, Expr):
                        v[i] = walk_e(x)
                    elif hasattr(x, "__dict__") and not isinstance(x, (Type, Function)):
                        walk(x)
            elif hasattr(v, "__dict__") and not isinstance(v, (Type, Function)) and type(v).__module__ == "vast":
                walk(v)

    for f in prog.functions:
        seen.add(id(f))
        for k, v in list(vars(f).items()):
            if k in ("x_auto", "x_raw", "inv_requires", "inv_ensures"):
                continue
            if isinstance(v, Expr):
                setattr(f, k, walk_e(v))
            elif isinstance(v, list):
                for i, x in enumerate(v):
                    if isinstance(x, Expr):
                        v[i] = walk_e(x)
                    elif hasattr(x, "__dict__"):
                        walk(x)
            elif hasattr(v, "__dict__") and type(v).__module__ == "vast" and not isinstance(v, Type):
                walk(v)


def check(prog, externs=()):
    """externs: names given with --rust-extern (crates a trusted rust fn path may start with besides std)"""
    Checker(prog, externs).run()

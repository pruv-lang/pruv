"""Inline policy pass over generated Rust text, shared rule with pruvc-rs inline_policy.rs."""
from shared_data import load

# Inline policy (SPEC "Inline policy"): a pass over the finished Rust text, so the Python and Rust
# compilers decide the same way by construction. A non-recursive `#[inline]` fn (one that cannot reach itself through
# calls) gets `#[inline(always)]` when its inclusive size (own body lines plus, once per call, the inclusive size of
# each callee that gets always) is at most INLINE_SMALL, when it has at most one call site, or when the copies it adds
# beyond one, incl * (call sites - 1), fit INLINE_BUDGET. The rest keep `#[inline]` and rustc's own heuristics.
_INLINE = load("inline_policy")   # shared with pruvc-rs (pruvc/data/inline_policy.json, tools/gen_shared.py)
INLINE_SMALL = _INLINE["INLINE_SMALL"]
INLINE_BUDGET = _INLINE["INLINE_BUDGET"]


# Private codegen markers consumed before the Rust text is returned.
PIN_INLINE = "#[inline] // pruv: explicit"
PIN_ALWAYS = "#[inline(always)] // pruv: explicit"


def _ident_byte(c):
    return 48 <= c <= 57 or 65 <= c <= 90 or 97 <= c <= 122 or c == 95


def _calls(text, names):
    """name -> number of call tokens in text: an identifier of names followed by '(', not after '.' or 'fn '"""
    b = text.encode()
    got = {}
    i, n = 0, len(b)
    while i < n:
        if not _ident_byte(b[i]):
            i += 1
            continue
        e = i
        while e < n and _ident_byte(b[e]):
            e += 1
        if e < n and b[e] == 40 and (i == 0 or b[i - 1] != 46) and not (i >= 3 and b[i - 3:i] == b"fn "):
            w = b[i:e].decode()
            if w in names:
                got[w] = got.get(w, 0) + 1
        i = e
    return got


def inline_policy(text):
    lines = text.split("\n")
    fns = []    # (attribute line, name, size, body text, pinned always)
    for i in range(len(lines) - 1):
        if lines[i] == PIN_INLINE:
            lines[i] = "#[inline]"
            continue
        pinned = lines[i] == PIN_ALWAYS
        if pinned:
            lines[i] = "#[inline(always)]"
        elif lines[i] != "#[inline]":
            continue
        h = lines[i + 1]
        p = 4 if h.startswith("pub fn ") else (11 if h.startswith("pub(crate) fn ") else 0)   # P09: pub(crate)
        if not h.startswith("fn ", p):
            continue
        e = p + 3
        while e < len(h) and _ident_byte(ord(h[e])):
            e += 1
        if e == p + 3 or e >= len(h) or h[e] != "(":
            continue
        if h.endswith("}") and h.count("{") == h.count("}"):
            fns.append((i, h[p + 3:e], 1, h, pinned))
            continue
        end = i + 2
        while end < len(lines) and lines[end] != "}":
            end += 1
        if end == len(lines):
            continue
        fns.append((i, h[p + 3:e], max(end - i - 2, 1), "\n".join(lines[i + 2:end]), pinned))
    if not fns:
        return "\n".join(lines)
    index = {}
    for k, f in enumerate(fns):
        index.setdefault(f[1], k)
    sites = _calls(text, index)
    edges = []      # per fn: (callee index, call count) in callee order
    for f in fns:
        got = _calls(f[3], index)
        edges.append(sorted((index[m], c) for m, c in got.items()))
    nf = len(fns)
    # Recursive automatic candidates add nothing. Explicit always still contributes its finite size.
    rec = [False] * nf
    for v in range(nf):
        seen = [False] * nf
        todo = [w for w, _ in edges[v]]
        while todo:
            w = todo.pop()
            if w == v:
                rec[v] = True
                break
            if not seen[w]:
                seen[w] = True
                todo.extend(x for x, _ in edges[w])
    # Cut edges between recursive functions so pinned recursion has a finite inclusive size.
    for v in range(nf):
        if rec[v]:
            edges[v] = [(w, c) for w, c in edges[v] if not rec[w]]
    # Evaluate the remaining DAG in post-order (iterative, call chains can be deep).
    done = [rec[v] and not fns[v][4] for v in range(nf)]
    incl = [0] * nf
    always = [False] * nf
    for root in range(nf):
        if done[root]:
            continue
        stack = [(root, 0)]
        while stack:
            v, k = stack[-1]
            if k < len(edges[v]):
                stack[-1] = (v, k + 1)
                w = edges[v][k][0]
                if not done[w]:
                    stack.append((w, 0))
                continue
            stack.pop()
            done[v] = True
            tot = fns[v][2]
            for w, c in edges[v]:
                if always[w]:
                    tot += c * incl[w]
            incl[v] = tot
            if fns[v][4]:
                always[v] = True
            else:
                calls = sites.get(fns[v][1], 0)
                always[v] = tot <= INLINE_SMALL or calls <= 1 or tot * (calls - 1) <= INLINE_BUDGET
    for v, f in enumerate(fns):
        if always[v]:
            lines[f[0]] = "#[inline(always)]"
    return "\n".join(lines)

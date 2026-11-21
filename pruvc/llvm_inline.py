"""Apply the inline policy to finished LLVM IR."""
import re

from inline_policy import INLINE_BUDGET, INLINE_SMALL


_SYMBOL = re.compile(r"@([^\s@(]+)\(")


def llvm_inline(text):
    lines = text.split("\n")
    fns = []
    for i, header in enumerate(lines):
        if not (header.startswith("define internal ") and header.endswith(" {")
                and "alwaysinline" not in header):
            continue
        symbol = _SYMBOL.search(header)
        if symbol is None or not symbol[1].endswith("__pruv_body"):
            continue
        end = i + 1
        while end < len(lines) and lines[end] != "}":
            end += 1
        if end < len(lines):
            fns.append((i, symbol[1], max(end - i - 1, 1), end))
    if not fns:
        return text

    index = {name: v for v, (_, name, _, _) in enumerate(fns)}
    sites = [0] * len(fns)
    for line in lines:
        for name in _SYMBOL.findall(line):
            if name in index:
                sites[index[name]] += 1
    edges = []
    for i, name, _, end in fns:
        sites[index[name]] -= lines[i].count("@" + name + "(")
        got = {}
        for line in lines[i + 1:end]:
            for callee in _SYMBOL.findall(line):
                if callee in index:
                    w = index[callee]
                    got[w] = got.get(w, 0) + 1
        edges.append(sorted(got.items()))

    nf = len(fns)
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
    for v in range(nf):
        if rec[v]:
            edges[v] = [(w, c) for w, c in edges[v] if not rec[w]]

    done = rec[:]
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
            tot = fns[v][2] + sum(c * incl[w] for w, c in edges[v] if always[w])
            incl[v] = tot
            calls = sites[v]
            always[v] = tot <= INLINE_SMALL or calls == 1 or (calls >= 2 and tot * (calls - 1) <= INLINE_BUDGET)
    for v, (i, _, _, _) in enumerate(fns):
        if always[v]:
            lines[i] = lines[i][:-2] + " alwaysinline {"
    return "\n".join(lines)

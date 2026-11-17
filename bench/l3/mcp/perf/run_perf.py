#!/usr/bin/env python3
"""P05M packet-core perf and counter harness (OLD per-tool path, NEW planned path, HAND safe Rust).

  python3 bench/l3/mcp/perf/run_perf.py [check|perf|all] [--rounds 5] [--time-ms 300] [--no-instr] [--no-chain]
                                        [--no-new] [--cells 0,1,10,50,100,250,500] [--pruvc PATH]

Measured work: one parsed tools/list page (JsonDocument = {source, tape}) plus the current packet (>= 5010 words) in, the
new packet out. Rows: old (old_packet.pruv + the root loop with array_at), new (packet.pruv plan + write, wrapper
core_new in perf_root.pruv, wired only when packet.pruv defines `plan`), hand (hand_packet.rs, safe Rust over the same
source + tape boundary). Parsing is outside every timed region.

check: generated edge-case pages (and chains of pages) go through driver `check`, which compares OLD, NEW and HAND word
for word on every stage, and the Python oracle (written from the packet.pruv semantics, independent of both) is compared
with the hand packet by length and checksum. Any mismatch exits 1.

perf: every (row, cell) in its own process, the row order rotated every round; per process median/min/max ns per call and
the output checksum (s = s*31 + w, wrapping i64), then a counting-allocator process (alloc calls, bytes, realloc calls)
and instructions per call (/usr/bin/time -l, macOS: instructions retired of R vs 5R calls, divided by the 4R extra
calls). Cells: N tools on one page (default 0,1,10,50,100,250,500) from a 5010-word start packet, plus 5x100 (five pages
of 100 tools chained, optional). Table to stdout (markdown), JSON to perf/work/perf_result.json.

The Pruv rows are verified-mode pruvc output (private fns, called through include!(gen.rs)); the hand row's output
Vec is allocated once at an upper bound, so its alloc_bytes counts the reservation, not the used words.
"""
import argparse, json, os, re, shutil, statistics, subprocess, sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
MCP = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(MCP, "..", "..", ".."))
WORK = os.path.join(HERE, "work")
CASES = os.path.join(WORK, "cases")
sys.path.insert(1, os.path.join(ROOT, "bench", "l3", "budget"))
from run_meta import run_meta, UNAVAILABLE  # noqa: E402  (read only)

PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"
ROWS = ["old", "new", "hand"]
MASK = (1 << 64) - 1
HEADER = ('#![allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case, unused_parens, '
          'unused_assignments, non_camel_case_types, private_interfaces)]\n')


# ---------------------------------------------------------------- build
def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


RUSTC_FLAGS = ["-C", "opt-level=3", "--edition", "2021"]


def sh(cmd, cwd=None, what=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s%s" % (what or cmd[0], r.stdout[-4000:], r.stderr[-6000:]))
    return r


def has_plan():
    txt = open(os.path.join(MCP, "packet.pruv"), encoding="utf-8").read()
    return bool(re.search(r"^\s*(pub\s+)?fn\s+plan\b", txt, re.M)) and bool(re.search(r"^\s*(pub\s+)?fn\s+write\b", txt, re.M))


def strip_new(src):
    return re.sub(r"// NEW-BEGIN.*?// NEW-END\n?", "", src, flags=re.S)


def assemble(new):
    os.makedirs(WORK, exist_ok=True)
    root = open(os.path.join(HERE, "perf_root.pruv"), encoding="utf-8").read()
    open(os.path.join(WORK, "perf_root.pruv"), "w", encoding="utf-8").write(root if new else strip_new(root))
    shutil.copy(os.path.join(MCP, "util.pruv"), os.path.join(WORK, "util.pruv"))
    shutil.copy(os.path.join(HERE, "old_packet.pruv"), os.path.join(WORK, "old_packet.pruv"))
    pk = os.path.join(WORK, "packet.pruv")
    if new:
        shutil.copy(os.path.join(MCP, "packet.pruv"), pk)
    elif os.path.exists(pk):
        os.remove(pk)
    man = {"name": "l3_mcp_perf", "version": "0.1.0", "root": "perf_root.pruv",
           "dependencies": {k: {"path": "../../../pkgs/" + k, "version": "0.1"} for k in ("json", "access", "lines", "jstr")}}
    json.dump(man, open(os.path.join(WORK, "perf_root.pruv.json"), "w"))
    rlib = os.path.join(WORK, "libpruv_str.rlib")
    sh([rustc(), "--crate-name", "pruv_str", "--crate-type", "rlib", os.path.join(MCP, "str_adapter.rs"), "-o", rlib],
       what="rustc str_adapter")


def compile_pruv(pruvc):
    gen = os.path.join(WORK, "gen.rs")
    if os.path.exists(gen):
        os.remove(gen)  # a stale gen.rs must never pass for a fresh compile
    r = subprocess.run([pruvc, "perf_root.pruv", "-o", "gen.rs", "--manifest", "perf_root.pruv.json",
                        "--rust-extern", "pruv_str=libpruv_str.rlib"], cwd=WORK, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(gen):
        return None, r.stdout[-4000:] + r.stderr[-4000:]
    return gen, ""


def build(pruvc, want_new):
    new = want_new and has_plan()
    assemble(new)
    gen, err = compile_pruv(pruvc)
    if gen is None and new:
        print("WARNING: pruvc failed with the NEW row wired, retrying without it:\n" + err, file=sys.stderr)
        new = False
        assemble(False)
        gen, err = compile_pruv(pruvc)
    if gen is None:
        sys.exit("pruvc failed:\n" + err)
    g = open(gen, encoding="utf-8").read()

    def one(pat, what):
        m = re.search(pat, g)
        if not m:
            sys.exit("gen.rs: cannot find " + what)
        return m.group(1)
    doc = one(r"pub struct (\w+_JsonDocument)\b", "JsonDocument")
    parse_t = one(r"pub enum (\w+_JsonParse)\b", "JsonParse")
    parse = one(r"fn (pruv_m_f_\d+_\d+_parse)\(", "parse")
    core_old = one(r"fn (pruv_m_f_\d+_\d+_core_old(?:__pruv_body)?)\(", "core_old")
    if new:
        core_new = "%s(p, d, c, o)" % one(r"fn (pruv_m_f_\d+_\d+_core_new(?:__pruv_body)?)\(", "core_new")
    else:
        core_new = "{ let _ = (p, d, c, o); Vec::new() }"
    drv = open(os.path.join(HERE, "perf_driver.rs"), encoding="utf-8").read()
    for k, v in (("@DOC@", doc), ("@PARSE@", parse), ("@PARSED@", parse_t + "::Parsed"),
                 ("@HAS_NEW@", "true" if new else "false"), ("@CORE_OLD@", core_old), ("@CORE_NEW@", core_new)):
        drv = drv.replace(k, v)
    shutil.copy(os.path.join(HERE, "hand_packet.rs"), os.path.join(WORK, "hand_packet.rs"))
    main = os.path.join(WORK, "main.rs")
    with open(main, "w", encoding="utf-8") as f:
        f.write(HEADER + 'include!("gen.rs");\nmod hand { include!("hand_packet.rs"); }\n' + drv)
    exe = os.path.join(WORK, "perf_driver")
    sh([rustc()] + RUSTC_FLAGS + ["-o", exe, main, "--extern", "pruv_str=" + os.path.join(WORK, "libpruv_str.rlib")],
       what="rustc driver")
    return exe, new


# ---------------------------------------------------------------- cases
def u16(s):
    b = s.encode("utf-16-le", "surrogatepass")
    return [b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2)]


def fold(words):
    s = 0
    for w in words:
        s = (s * 31 + w) & MASK
    return s - (1 << 64) if s >> 63 else s


def usable(u):
    return 1 <= len(u) <= 200 and all(x >= 32 and not (127 <= x <= 159) and x not in (8232, 8233) for x in u)


def cut(u, cap):
    if len(u) <= cap:
        return u
    n = cap
    if n > 0 and 0xD800 <= u[n - 1] <= 0xDBFF and 0xDC00 <= u[n] <= 0xDFFF:
        n -= 1
    return u[:n]


def oracle_page(raw_bytes, packet, count, objects):
    """The packet.pruv semantics straight from Python's own JSON decoding (last duplicate wins)."""
    doc = json.loads(raw_bytes.decode("utf-8"))
    p = list(packet)
    res = doc.get("result") if isinstance(doc, dict) else None
    tools = res.get("tools") if isinstance(res, dict) else None
    if not isinstance(tools, list):
        return p, count, objects
    for t, raw in zip(tools, split_tools(raw_bytes)):
        if objects >= 500:
            break
        if not isinstance(t, dict):
            continue
        objects += 1
        nm = t.get("name")
        if not isinstance(nm, str) or count >= 500:
            continue
        nu = u16(nm)
        if not usable(nu):
            continue
        base = 10 + 10 * count
        p[base], p[base + 1] = len(p), len(nu)
        p.extend(nu)
        for slot, key, capv, nonempty in ((2, "title", 16000000, True), ("4", "description", 65536, False)):
            slot = int(slot)
            p[base + slot] = -1
            v = t.get(key)
            if isinstance(v, str):
                u = cut(u16(v), capv)
                if (not nonempty) or len(u) > 0:
                    p[base + slot], p[base + slot + 1] = len(p), len(u)
                    p.extend(u)
        a = t.get("annotations")
        ro = a.get("readOnlyHint") if isinstance(a, dict) else None
        if ro is True:
            p[base + 6] = 1
        dh = a.get("destructiveHint") if isinstance(a, dict) else None
        p[base + 7] = 1 if dh is True else (0 if dh is False else -1)
        p[base + 8], p[base + 9] = len(p), len(raw)
        p.extend(raw)
        count += 1
    return p, count, objects


def split_tools(raw_bytes):
    """Raw JSON bytes of every element of result.tools, in order (the generator keeps pages compact enough that a
    brace/bracket scan of the tools array is exact: it tracks strings and escapes)."""
    doc_end = len(raw_bytes)
    i = 0
    # find the tools array by the generator's marker: pages are written as {"result":{"tools":[...]...}} (plus a few
    # shaped variants), so locate the LAST top-level "tools" array start via a structural scan.
    spans = []
    depth = 0
    in_str = False
    esc = False
    path = []  # (kind, key)
    cur_key = None
    elem_start = None
    arr_depth = None
    last_arr = None
    stack = []
    key_buf = None
    j = 0
    s = raw_bytes
    result_spans = {}
    # simple recursive descent instead: parse values and note spans
    def skip_ws(k):
        while k < doc_end and s[k] in b" \t\r\n":
            k += 1
        return k

    def val(k):
        k = skip_ws(k)
        c = s[k]
        if c == 0x22:
            k += 1
            while s[k] != 0x22:
                k += 2 if s[k] == 0x5C else 1
            return k + 1
        if c in b"{[":
            close = 0x7D if c == 0x7B else 0x5D
            k = skip_ws(k + 1)
            if s[k] == close:
                return k + 1
            while True:
                if c == 0x7B:
                    k = val(k)
                    k = skip_ws(k) + 1  # colon
                k = val(k)
                k = skip_ws(k)
                if s[k] == close:
                    return k + 1
                k += 1  # comma
        while k < doc_end and s[k] not in b",]} \t\r\n":
            k += 1
        return k

    def members(k):
        """[(key_text_raw, vstart, vend)] of the object at k."""
        out = []
        k = skip_ws(k + 1)
        if s[k] == 0x7D:
            return out
        while True:
            ks = skip_ws(k)
            ke = val(ks)
            key = json.loads(s[ks:ke].decode("utf-8"))
            k = skip_ws(ke) + 1
            vs = skip_ws(k)
            ve = val(vs)
            out.append((key, vs, ve))
            k = skip_ws(ve)
            if s[k] == 0x7D:
                return out
            k += 1

    st = skip_ws(0)
    if s[st] != 0x7B:
        return []
    r = [m for m in members(st) if m[0] == "result"]
    if not r or s[r[-1][1]] != 0x7B:
        return []
    t = [m for m in members(r[-1][1]) if m[0] == "tools"]
    if not t or s[t[-1][1]] != 0x5B:
        return []
    out = []
    k = skip_ws(t[-1][1] + 1)
    if s[k] == 0x5D:
        return out
    while True:
        ks = skip_ws(k)
        ke = val(ks)
        out.append(list(s[ks:ke]))
        k = skip_ws(ke)
        if s[k] == 0x5D:
            return out
        k += 1


def tool_json(i, big=False):
    schema = {"type": "object", "properties": {"path": {"type": "string", "description": "file path %d" % i},
                                               "depth": {"type": "integer", "minimum": 0}}, "required": ["path"]}
    d = {"name": "tool_%d" % i, "title": "Tool number %d" % i,
         "description": "Reads and transforms item %d of the workspace, returning a short summary. " % i * 3,
         "annotations": {"readOnlyHint": i % 2 == 0, "destructiveHint": i % 3 == 0, "idempotentHint": True},
         "inputSchema": schema}
    return json.dumps(d, separators=(",", ":"))


def page(tools_text, pre='{"jsonrpc":"2.0","id":1,"result":{"tools":[', post=']}}'):
    return (pre + ",".join(tools_text) + post).encode("utf-8")


def gen_cases():
    os.makedirs(CASES, exist_ok=True)
    pages = {}
    ex = lambda **k: json.dumps(k, separators=(",", ":"), ensure_ascii=False)
    pages["basic"] = page([tool_json(i) for i in range(3)])
    pages["empty_tools"] = page([])
    pages["no_result"] = b'{"jsonrpc":"2.0","id":1}'
    pages["tools_not_array"] = b'{"result":{"tools":{"name":"x"}}}'
    pages["root_array"] = b'[{"name":"x"}]'
    pages["dup_result"] = b'{"result":{"tools":[{"name":"first"}]},"result":{"tools":[{"name":"second"}]}}'
    pages["dup_tools"] = b'{"result":{"tools":[{"name":"a"}],"tools":[{"name":"b"},{"name":"c"}]}}'
    pages["escaped_keys"] = b'{"re\\u0073ult":{"t\\u006fols":[{"na\\u006De":"esc","\\u0074itle":"T","descr\\u0069ption":"D"}]}}'
    pages["whitespace"] = (b' \n{ "result" :\t{ "tools" : [ \n { "name" : "ws" , "title" : "t" } , { "name":"w2" } ] } } ')
    pages["name_cases"] = page([
        ex(name="eé中"), ex(name="astral \U0001F600 x"), '{"name":"esc \\u00e9 \\ud83d\\ude00 \\/ \\\\ \\" q"}',
        '{"name":"nl\\n"}', '{"name":"tab\\t"}', ex(name="a" * 200), ex(name="a" * 201), ex(name="b" * 199 + "\U0001F600"),
        '{"name":"del\\u007f"}', '{"name":"c1\\u009f"}', '{"name":"nbsp\\u00a0ok"}', '{"name":"ls\\u2028"}',
        '{"name":"ps\\u2029"}', '{"name":""}', '{"name":5}', '{"name":null}', '{"title":"no name"}', '{"nam":"x"}',
        '{"name":"first","name":"last"}', '{"name":"good","name":""}', '{"name":"","name":"good2"}', '{"name":["a"]}',
        '{"name":"lone\\ud800"}', '{"name":"lone\\udc00 x"}'])
    pages["field_cases"] = page([
        '{"name":"t0","title":""}', '{"name":"t1","title":5}', '{"name":"t2","title":"T","title":""}',
        '{"name":"t3","title":"","title":"T3"}', '{"name":"d0","description":""}', '{"name":"d1","description":null}',
        '{"name":"d2","description":"\\u00e9\\ud83d\\ude00"}', '{"name":"d3","title":"x\\ud83d"}',
        ex(name="d4", description="a" * 65536), ex(name="d5", description="a" * 65537),
        ex(name="d6", description="a" * 65535 + "\U0001F600"), ex(name="d7", description="a" * 65534 + "\U0001F600z"),
        ex(name="d8", description="a" * 70000), ex(name="d9", description="\U0001F600" * 40000),
        '{"name":"h0","annotations":{"readOnlyHint":true,"destructiveHint":false}}',
        '{"name":"h1","annotations":{"readOnlyHint":false,"destructiveHint":true}}',
        '{"name":"h2","annotations":{"readOnlyHint":1,"destructiveHint":"true"}}',
        '{"name":"h3","annotations":{"readOnlyHint":null,"destructiveHint":null}}',
        '{"name":"h4","annotations":[true]}', '{"name":"h5","annotations":5}',
        '{"name":"h6","annotations":{"readOnlyHint":false,"readOnlyHint":true,"destructiveHint":true,"destructiveHint":false}}',
        '{"name":"h7","annotations":{"readOnlyHint":true},"annotations":{"destructiveHint":true}}',
        '{"name":"h8","annotations":{"nested":{"readOnlyHint":true}}}'])
    pages["array_mix"] = page(['5', '"str"', '[{"name":"inner"}]', 'null', 'true', '{"name":"real"}', '{"name":2}',
                               '{"inner":{"name":"nope"},"name":"x"}', '{}', '{"name":"after"}'])
    pages["utf8_raw"] = page(['{"name":"héllo \U0001F600","title":"中文","description":"ü \U0001F600"}'])
    pages["many_600"] = page([('{"name":"n%d"}' % i) if i % 7 else '{"name":""}' for i in range(600)])
    pages["mixed_objects"] = page([('{"name":"m%d"}' % i) if i % 2 else '5' for i in range(1100)])
    for k, v in pages.items():
        open(os.path.join(CASES, k + ".json"), "wb").write(v)
    chains = {k: [k] for k in pages}
    # chained pages: each continues (packet, count, objects)
    for i in range(3):
        open(os.path.join(CASES, "p200_%d.json" % i), "wb").write(page([tool_json(200 * i + j) for j in range(200)]))
    chains["chain_3x200"] = ["p200_0", "p200_1", "p200_2"]
    chains["chain_basic_x3"] = ["basic", "field_cases", "name_cases"]
    # perf cells
    for n in (0, 1, 10, 50, 100, 250, 500, 600):
        open(os.path.join(CASES, "perf_%d.json" % n), "wb").write(page([tool_json(i) for i in range(n)]))
    for i in range(5):
        open(os.path.join(CASES, "c100_%d.json" % i), "wb").write(page([tool_json(100 * i + j) for j in range(100)]))
    chains["chain_5x100"] = ["c100_%d" % i for i in range(5)]
    return chains


def start_packet():
    p = [0] * 5010
    p[1], p[4], p[6] = 1, -1, -1
    return p


def chain_arg(names):
    return ",".join(os.path.join(CASES, n + ".json") for n in names)


# ---------------------------------------------------------------- check
def run_check(exe, chains):
    names = sorted(chains)
    args = [chain_arg(chains[n]) for n in names]
    r = subprocess.run([exe, "check"] + args, capture_output=True, text=True)
    bad = 0
    if r.stderr.strip():
        sys.stderr.write(r.stderr[-6000:])
    if r.returncode != 0:
        print("CHECK: driver comparison (OLD / NEW / HAND) reported mismatches")
        bad += 1
    lines = {}
    for ln in r.stdout.splitlines():
        t = ln.split()
        if len(t) == 5:
            lines[t[0]] = [int(x) for x in t[1:]]
    for n, a in zip(names, args):
        p, c, o = start_packet(), 0, 0
        for nm in chains[n]:
            p, c, o = oracle_page(open(os.path.join(CASES, nm + ".json"), "rb").read(), p, c, o)
        want = [len(p), fold(p), c, o]
        got = lines.get(a)
        if got != want:
            print("CHECK: oracle vs hand mismatch on %s: oracle %s hand %s" % (n, want, got))
            bad += 1
    print("check: %d cases (%d single pages, %d chains), %d mismatch(es)" % (
        len(names), sum(1 for n in names if len(chains[n]) == 1), sum(1 for n in names if len(chains[n]) > 1), bad))
    return bad


# ---------------------------------------------------------------- perf
def instr_retired(cmd):
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "instructions retired" in l:
            return int(l.split()[0])
    return None


def fmt(x, d=0):
    return UNAVAILABLE if x is None else ("%.*f" % (d, x) if d else "%d" % round(x))


def run_perf(exe, new, args):
    cells = []
    for n in [int(x) for x in args.cells.split(",")]:
        cells.append(("%d" % n, os.path.join(CASES, "perf_%d.json" % n)))
    if not args.no_chain:
        cells.append(("5x100", chain_arg(["c100_%d" % i for i in range(5)])))
    rows = [r for r in ROWS if r != "new" or new]
    res = {}
    for rd in range(args.rounds):
        for cell, arg in cells:
            order = rows[rd % len(rows):] + rows[:rd % len(rows)]
            for row in order:
                r = subprocess.run([exe, "time", row, arg, str(args.time_ms)], capture_output=True, text=True)
                if r.returncode != 0:
                    sys.exit("time %s %s failed:\n%s" % (row, cell, r.stderr[-2000:]))
                med, mn, mx, n, ck, tm = r.stdout.split()
                e = res.setdefault((row, cell), {"med": [], "min": [], "max": [], "tmean": [], "calls": 0, "ck": set()})
                e["med"].append(int(med)); e["min"].append(int(mn)); e["max"].append(int(mx))
                e["tmean"].append(float(tm))
                e["calls"] += int(n); e["ck"].add(int(ck))
        print("round %d/%d done" % (rd + 1, args.rounds), file=sys.stderr, flush=True)
    out = []
    for cell, arg in cells:
        for row in rows:
            e = res[(row, cell)]
            cr = subprocess.run([exe, "counters", row, arg], capture_output=True, text=True)
            ac, ab, ar, ow, ock = [int(x) for x in cr.stdout.split()]
            ins = None
            if not args.no_instr:
                per = e["med"] and statistics.median(e["med"])
                reps = max(5, min(2000, int(30e6 / max(per, 1))))  # ~30 ms of calls for R
                vals = []
                for _ in range(3):
                    lo = instr_retired([exe, "instr", row, arg, str(reps)])
                    hi = instr_retired([exe, "instr", row, arg, str(5 * reps)])
                    if lo is not None and hi is not None:
                        vals.append((hi - lo) / (4 * reps))
                ins = statistics.median(vals) if vals else None
            out.append({"cell": cell, "row": row, "median_ns": statistics.median(e["med"]), "min_ns": min(e["min"]),
                        "max_ns": max(e["max"]), "tmean_ns": statistics.median(e["tmean"]),
                        "rounds_tmean_ns": e["tmean"], "calls": e["calls"], "rounds_median_ns": e["med"],
                        "checksum": sorted(e["ck"]), "counter_checksum": ock, "out_words": ow,
                        "alloc_count": ac, "alloc_bytes": ab, "realloc_count": ar, "instr_per_call": ins})
    return cells, rows, out


def report(cells, rows, out, args, new, pruvc):
    by = {(o["cell"], o["row"]): o for o in out}
    bad = 0
    print("\n| cell | row | median ns | min | max | tmean ns | instr/call | alloc_count | alloc_bytes | out words | checksum | vs hand (ns) | vs old (ns) | vs hand (tmean) |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for cell, _ in cells:
        cks = {c for r in rows for c in by[(cell, r)]["checksum"]} | {by[(cell, r)]["counter_checksum"] for r in rows}
        agree = len(cks) == 1
        if not agree:
            bad += 1
            print("ERROR: checksums disagree in cell %s: %s" % (cell, sorted(cks)), file=sys.stderr)
        for row in rows:
            o = by[(cell, row)]
            h, ol = by[(cell, "hand")], by[(cell, "old")]
            print("| %s | %s | %d | %d | %d | %.0f | %s | %d | %d | %d | %d%s | %.2fx | %.2fx | %.2fx |" % (
                cell, row, o["median_ns"], o["min_ns"], o["max_ns"], o["tmean_ns"], fmt(o["instr_per_call"]),
                o["alloc_count"], o["alloc_bytes"], o["out_words"], o["counter_checksum"], "" if agree else " !",
                o["median_ns"] / max(h["median_ns"], 1), o["median_ns"] / max(ol["median_ns"], 1),
                o["tmean_ns"] / max(h["tmean_ns"], 1e-9)))
    meta = run_meta(pruvc=pruvc, backend="rust", rustc=rustc(), flags=RUSTC_FLAGS, profile="p05m-packet-core",
                    lanes=rows, extra={"new_wired": new, "rounds": args.rounds, "time_ms": args.time_ms,
                                       "cells": [c for c, _ in cells]})
    json.dump({"run_meta": meta, "results": out}, open(os.path.join(WORK, "perf_result.json"), "w"), indent=1)
    print("\nJSON: " + os.path.join(WORK, "perf_result.json"))
    return bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", nargs="?", default="all", choices=["check", "perf", "all"])
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--time-ms", type=int, default=300)
    ap.add_argument("--cells", default="0,1,10,50,100,250,500")
    ap.add_argument("--no-instr", action="store_true")
    ap.add_argument("--no-chain", action="store_true")
    ap.add_argument("--no-new", action="store_true")
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    args = ap.parse_args()
    chains = gen_cases()
    exe, new = build(args.pruvc, not args.no_new)
    print("NEW row wired: %s" % ("yes" if new else "no (packet.pruv has no plan/write, or --no-new)"))
    bad = 0
    if args.mode in ("check", "all"):
        bad += run_check(exe, chains)
    if args.mode in ("perf", "all") and bad == 0:
        cells, rows, out = run_perf(exe, new, args)
        bad += report(cells, rows, out, args, new, args.pruvc)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

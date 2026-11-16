"""G4b-2: the trust report (docs/G4B_SPEC.md section 6).

build_report(...) returns the JSON text of `--trust-report PATH`; write_report(path, text) writes it as UTF-8 bytes.
Apart from the `compiler` object the text is byte-identical to the one pruvc-rs writes.
"""
import hashlib
import json
import os

VERSION = "0.0.1"


def _solver():
    import z3
    return z3.get_full_version()


def _sha256(files):
    """files: [(display path, absolute path)] in load order; each is <display>\\0<byte length>\\0<raw bytes>"""
    h = hashlib.sha256()
    for disp, ap in files:
        with open(ap, "rb") as fh:
            raw = fh.read()
        h.update(disp.encode("utf-8") + b"\0" + str(len(raw)).encode() + b"\0" + raw)
    return h.hexdigest()


def build_report(prog, trust, graph, input_path, verify, trust_callers, foreign_checks, infer,
                 rust_extern, rust_search, cache=None, cache_flag=False):
    """prog: the loaded program (modules in load order); trust: the dict filled by gen_rust_ex(trust_out=...);
    graph: manifest.Graph or None; input_path: the root file given on the command line."""
    cprog = trust.get("program")
    fg = trust.get("fg")
    base = os.path.basename(input_path)
    # package descriptors: (key, name, version, display); key None = the program package
    if graph is not None:
        pk = [(None, graph.program.name, graph.program.version, "")]
        for p in sorted((p for p in graph.pkgs if p is not graph.program), key=lambda p: p.name):
            pk.append((p.name, p.name, p.version, "@pkg/" + p.name))
    else:
        pk = [(None, "", "", "")]
    # files per package, in module load order
    files = {k[0]: [] for k in pk}
    mods = getattr(prog, "modules", None)
    if mods:
        for m in mods:
            if m.pkg in files and m.abspath is not None:
                files[m.pkg].append((m.file, m.abspath))
    else:
        files[None].append((base, os.path.abspath(input_path)))
    # trusted functions of the concrete program
    trusted = {k[0]: [] for k in pk}
    if fg is not None and cprog is not None:
        for f in cprog.functions:
            if f.foreign is None:
                continue
            key = getattr(f, "pkg", None)
            if key not in trusted:
                key = None
            trusted[key].append({"kind": "trusted_rust_fn", "item": getattr(f, "disp", None) or f.name,
                                 "file": getattr(f, "file", None) or base, "line": int(f.line),
                                 "reachable": f.name in fg.called})
    packages = []
    for key, name, version, display in pk:
        tr = sorted(trusted[key], key=lambda t: (t["file"], t["line"], t["item"]))
        cat = sorted(fg.pkg_catalog.get(key, ())) if fg is not None else []
        packages.append({"name": name, "version": version, "display": display,
                         "sha256": _sha256(files[key]),
                         "trusted": tr,
                         "catalog": [{"op": op, "type_args": list(ta)} for op, ta in cat]})
    doc = {"format": 1,
           "compiler": {"impl": "py", "version": VERSION, "solver": _solver()},
           "flags": {"verify": bool(verify), "trust_callers": bool(trust_callers),
                     "foreign_checks": foreign_checks, "infer": bool(infer),
                     "rust_extern": list(rust_extern), "rust_search": list(rust_search),
                     "proof_cache": bool(cache_flag)},
           "packages": packages,
           "proof_cache": {"hits": cache.hits if cache is not None else 0,
                           "misses": cache.misses if cache is not None else 0,
                           "stores": cache.stores if cache is not None else 0}}
    return json.dumps(doc, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def write_report(path, text):
    with open(path, "wb") as fh:
        fh.write(text.encode("utf-8"))

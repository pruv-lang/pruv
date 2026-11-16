"""G6 A2 (docs/G6_SPEC.md section 12): Rust-source boundary helpers for the standard catalog and for
`trusted rust fn` declarations, the three check modes, and the `<out>.foreign.json` manifest.

ForeignGen is owned by a codegen.Gen (only created for a program that uses G6). Helpers are private Rust functions
named
    pruv_g6_<op>__<enc>[__<enc>]      catalog instance; <enc> = typeinfo.encode of each (concrete, un-lowered) type
                                      argument, e.g. pruv_g6_vec_push__i, pruv_g6_map_insert__i__b
    pruv_g6_map_updated__<enc>__<enc> contract scan used by a requires clause (not a catalog operation)
    NAME                              the helper of `trusted rust fn NAME` (same name as the declaration)
and are emitted once each, in first-use order, in front of the Pruv functions.

Check modes (Gen.fmode): requires = requires checks in every helper; all = also ensures (snapshots are `.clone()`s
taken before the operation; the actual contents are compared afterwards); off = no contract checks.  Indexing is
checked in every mode (`vec_get`, `string_byte`).  `all` spends the contract budget: one unit per visited element
or entry, a fresh budget per helper call; exhaustion -> `pruv: foreign contract undefined: NAME: budget`.
"""
import json

from vast import *
from typeinfo import encode, format_type, is_collection
from codegen import rty, cty, conv, show, rstr
from modules import safe_type     # G4: nominal identities in helper names use their [0-9A-Za-z_] encoding


def _budget_line(n):
    return "let mut pruv_budget: i64 = %s;" % format(n, "_")


class ForeignGen:
    def __init__(s, gen):
        s.g = gen
        s.mode = gen.fmode
        s.req = s.mode != "off"
        s.ens = s.mode == "all"
        s.items = {}       # helper name -> text (insertion order = first use)
        s.catalog = []     # manifest: executable catalog instances in first-use order
        s.called = set()   # trusted fn names called from an emitted function
        s.pkg_catalog = {}   # G4b-2: package (None = program) -> set of (op, tuple(type_args)); every use, not only first

    def _note_catalog(s, op, src):
        """G4b-2: record a catalog use for the package of the function being emitted"""
        key = getattr(s.g, "function_pkg", None)
        s.pkg_catalog.setdefault(key, set()).add((op, tuple(format_type(t) for t in src)))

    # ------------------------------------------------------------------ results
    def helper_texts(s):
        return list(s.items.values())

    def manifest(s, externs):
        g = s.g
        trusted = []
        for f in g.prog.functions:
            if f.foreign is None:
                continue
            # G4 A2: explicit mode lists the source-qualified identity (crate::m::f), never the encoded stem
            trusted.append({"name": getattr(f, "disp", f.name), "path": f.foreign["path"], "line": f.line, "col": f.col,
                            "effects": list(f.effects),
                            "requires": [show(r) for r in f.requires],
                            "ensures": [show(x) for x in f.ensures],
                            "reachable": f.name in s.called})
        doc = {"schema": "pruv-foreign-v1", "backend": "rust", "check_mode": s.mode, "rustc_checked": False,
               "trusted": trusted, "catalog": list(s.catalog),
               "externs": [{"name": n, "path": p} for n, p in externs]}
        return json.dumps(doc, sort_keys=True, indent=2, ensure_ascii=False) + "\n"

    # ------------------------------------------------------------------ helper skeleton
    def _fn(s, name, params, ret, body):
        """text of `fn name(params) -> ret { guard; budget; body }`; body: lines (already without indentation)"""
        g = s.g
        L = [g.allow, "fn %s(%s)%s {" % (name, params, "" if ret is None else " -> " + ret)]
        if s.req:
            L.append("    let pruv_g = pruv_ctx(%s);" % rstr(s.op))
        if s.ens:
            L.append("    " + _budget_line(g.budget))
        L.extend("    " + x for x in body)
        L.append("}")
        return "\n".join(L) + "\n"

    def _req(s, cond, text):
        return ["if !(%s) { pruv_g6_req(%s, %s); }" % (cond, rstr(s.op), rstr(text))] if s.req else []

    def _ens(s, cond, text):
        return ["if !(%s) { pruv_g6_ens(%s, %s); }" % (cond, rstr(s.op), rstr(text))] if s.ens else []

    def _snap(s, ty, src):
        """snapshot of a consumed input (mode all): charge its elements, then an independent clone"""
        if not s.ens:
            return []
        return ["pruv_g6_charge(&mut pruv_budget, %s.len());" % src, "let pruv_snap: %s = %s.clone();" % (ty, src)]

    # ------------------------------------------------------------------ catalog
    def catalog_helper(s, op, low, src):
        if not src and low:
            src = low
        s._note_catalog(op, src)
        name = "pruv_g6_" + op + "".join("__" + encode(safe_type(t)) for t in src)
        if name not in s.items:
            s.op = op
            s.items[name] = s._catalog_text(op, name, low)
            s.catalog.append({"op": op, "type_args": [format_type(t) for t in src]})
        return name

    def _catalog_text(s, op, name, low):
        if op.startswith("file_"):
            return s._file_text(op, name)
        if op.startswith("proc_"):
            return s._proc_text(op, name)
        if op.startswith("ev_"):
            return s._async_text(op, name)
        if op.startswith("net_"):
            return s._net_text(op, name)
        STRING = "::std::string::String"
        if op in ("vec_new", "vec_push", "vec_get"):
            T = rty(low[0])
            V = rty(Type("vec", (low[0],)))
        elif op in ("map_new", "map_get", "map_insert"):
            K, T = rty(low[0]), rty(low[1])
            M = rty(Type("hashmap", (low[0], low[1])))
            OPT = rty(mk_option(low[1]))
        if op == "vec_new":
            body = ["let pruv_r: %s = ::std::vec::Vec::<%s>::new();" % (V, T)]
            body += s._ens("pruv_r.len() == 0", "len(result) == 0")
            body += ["pruv_r"]
            return s._fn(name, "", V, body)
        if op == "vec_push":
            body = s._req("(v.len() as u64) < (1u64 << 40)", "len(v) < 2^40")
            body += s._snap(V, "v")
            body += ["v.push(x);"]
            body += s._ens("v.len() == pruv_snap.len() + 1", "len(result) == len(v) + 1")
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, pruv_snap.len());"]
            body += s._ens("v[..pruv_snap.len()] == pruv_snap[..]", "result[..len(v)] == v")
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, 1);"]   # one visited element (G6_SPEC 12.3)
            body += s._ens("v[pruv_snap.len()] == x", "result[len(v)] == x")
            body += ["v"]
            return s._fn(name, "mut v: %s, x: %s" % (V, T), V, body)
        if op == "vec_get":
            body = s._req("i >= 0 && (i as u64) < (v.len() as u64)", "0 <= i && i < len(v)")
            body += ["let pruv_k: usize = if i >= 0 && (i as u64) < (v.len() as u64) { i as usize } "
                     "else { pruv_g6_repr(\"index\") };",
                     "let pruv_r: %s = v[pruv_k];" % T]
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, 1);"]   # one visited element (G6_SPEC 12.3)
            body += s._ens("v.get(pruv_k).copied() == Some(pruv_r)", "result == v[i]")
            body += ["pruv_r"]
            return s._fn(name, "v: &%s, i: i64" % V, T, body)
        if op == "map_new":
            body = ["let pruv_r: %s = ::std::collections::HashMap::<%s, %s>::new();" % (M, K, T)]
            body += s._ens("pruv_r.len() == 0", "len(result) == 0")
            body += ["pruv_r"]
            return s._fn(name, "", M, body)
        if op == "map_get":
            body = ["let pruv_r: %s = m.get(&k).copied();" % OPT]
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, 1);"]   # one visited element (G6_SPEC 12.3)
            body += s._ens("m.contains_key(&k) == pruv_r.is_some() && m.get(&k).copied() == pruv_r",
                           "result == map_lookup(m, k)")
            body += ["pruv_r"]
            return s._fn(name, "m: &%s, k: %s" % (M, K), OPT, body)
        if op == "map_insert":
            body = s._req("m.contains_key(&k) || (m.len() as u64) < (1u64 << 40)", "k in m || len(m) < 2^40")
            body += s._snap(M, "m")
            body += ["let _ = m.insert(k, v);"]
            body += s._ens("m.len() == pruv_snap.len() + (if pruv_snap.contains_key(&k) { 0 } else { 1 })",
                           "len(result) == len(m) + (k absent ? 1 : 0)")
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, 1);"]   # one visited element (G6_SPEC 12.3)
            body += s._ens("m.get(&k) == Some(&v)", "result[k] == v")
            if s.ens:
                body += ["for (pruv_sk, pruv_sv) in pruv_snap.iter() {",
                         "    pruv_g6_charge(&mut pruv_budget, 1);",
                         "    if *pruv_sk != k && m.get(pruv_sk) != Some(pruv_sv) { "
                         "pruv_g6_ens(%s, \"entries other than k are unchanged\"); }" % rstr(op),
                         "}"]
            body += ["m"]
            return s._fn(name, "mut m: %s, k: %s, v: %s" % (M, K, T), M, body)
        if op == "string_new":
            body = ["let pruv_r: %s = ::std::string::String::new();" % STRING]
            body += s._ens("pruv_r.len() == 0", "len(result) == 0")
            body += ["pruv_r"]
            return s._fn(name, "", STRING, body)
        if op == "string_push_str":
            body = s._req("(s.len() as u64) + (text.len() as u64) <= (1u64 << 40)", "len(s) + len(text) <= 2^40")
            body += s._snap(STRING, "s")
            body += ["s.push_str(text);"]
            body += s._ens("s.len() == pruv_snap.len() + text.len()", "len(result) == len(s) + len(text)")
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, pruv_snap.len());"]
            body += s._ens("s.as_bytes()[..pruv_snap.len()] == pruv_snap.as_bytes()[..]", "result[..len(s)] == s")
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, text.len());"]
            body += s._ens("s.as_bytes()[pruv_snap.len()..] == text.as_bytes()[..]",
                           "result[len(s)..] == text")
            body += ["s"]
            return s._fn(name, "mut s: %s, text: &str" % STRING, STRING, body)
        if op == "string_byte":
            body = s._req("i >= 0 && (i as u64) < (s.len() as u64)", "0 <= i && i < len(s)")
            body += ["let pruv_k: usize = if i >= 0 && (i as u64) < (s.len() as u64) { i as usize } "
                     "else { pruv_g6_repr(\"index\") };",
                     "let pruv_r: i64 = s.as_bytes()[pruv_k] as i64;"]
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, 1);"]   # one visited element (G6_SPEC 12.3)
            body += s._ens("s.as_bytes().get(pruv_k).map(|pruv_b| *pruv_b as i64) == Some(pruv_r)",
                           "result == s[i]")
            body += ["pruv_r"]
            return s._fn(name, "s: &%s, i: i64" % STRING, "i64", body)
        if op == "string_to_str":
            body = []
            if s.ens:
                body += ["pruv_g6_charge(&mut pruv_budget, s.len());"]
            body += ["let pruv_r: %s = s.clone();" % STRING]
            body += s._ens("pruv_r.as_bytes() == s.as_bytes()", "bytes of result equal bytes of s")
            body += ["pruv_r"]
            return s._fn(name, "s: &%s" % STRING, STRING, body)
        raise Exception("codegen: unknown catalog operation %s" % op)

    def _file_text(s, op, name):
        """G10: fallible I/O keeps failure in the owned handle, never unwraps an I/O result."""
        if op in ("file_ok", "file_readable", "file_writable"):
            return s._fn(name, "f: &pruv_File", "bool", ["f." + op[5:]])
        if op == "file_size":
            body = ["let pruv_r = f.buf.len() as i64;"]
            body += s._ens("pruv_r >= 0", "result >= 0")
            return s._fn(name, "f: &pruv_File", "i64", body + ["pruv_r"])
        if op == "file_byte":
            cond = "i >= 0 && (i as u64) < (f.buf.len() as u64)"
            body = s._req(cond, "0 <= i && i < file_size(f)")
            body += ['let pruv_k = if %s { i as usize } else { pruv_g6_repr("index") };' % cond,
                     "let pruv_r = f.buf[pruv_k] as i64;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255")
            return s._fn(name, "f: &pruv_File, i: i64", "i64", body + ["pruv_r"])
        if op in ("file_open_read", "file_create"):
            read = op == "file_open_read"
            body = ["let file = ::std::fs::File::%s(path).ok();" % ("open" if read else "create"),
                    "let ok = file.is_some();",
                    "let f = pruv_File { file, ok, readable: %s, writable: %s, buf: ::std::vec::Vec::new() };" %
                    ("ok" if read else "false", "false" if read else "ok")]
            params = "path: &str"
        elif op == "file_close":
            body = ["let pruv_ok = f.ok;",
                    "let pruv_r = f.ok && match f.file.as_mut() {",
                    "    Some(file) => ::std::io::Write::flush(file).is_ok(), None => false",
                    "};"]
            body += s._ens("!pruv_r || pruv_ok", "result ==> file_ok(f)")
            return s._fn(name, "mut f: pruv_File", "bool", body + ["pruv_r"])
        else:
            read = op == "file_read_all"
            field = "readable" if read else "writable"
            body = s._req("f." + field, "file_%s(f)" % field)
            params = "mut f: pruv_File"
            if read:
                body += ["f.buf.clear();"]
                action = "::std::io::Read::read_to_end(file, &mut f.buf).is_ok()"
            else:
                params += ", text: &str" if op == "file_write_str" else ", data: &[u8]"
                data = "text.as_bytes()" if op == "file_write_str" else "data"
                action = "::std::io::Write::write_all(file, %s).is_ok()" % data
            body += ["let pruv_ok = f.ok && f.%s && match f.file.as_mut() {" % field,
                     "    Some(file) => %s, None => false" % action,
                     "};",
                     "if !pruv_ok { f.ok = false; f.file = None; f.buf.clear(); }",
                     "f.readable = %s; f.writable = %s;" %
                     ("f.ok" if read else "false", "false" if read else "f.ok")]
        if read:
            body += s._ens("!f.writable && f.readable == f.ok", "!file_writable(result) && file_readable(result) == file_ok(result)")
            if op == "file_read_all":
                body += s._ens("if f.ok { (f.buf.len() as i64) >= 0 } else { f.buf.is_empty() }",
                               "file_ok(result) ? file_size(result) >= 0 : file_size(result) == 0")
            else:
                body += s._ens("f.buf.is_empty()", "file_size(result) == 0")
        else:
            body += s._ens("!f.readable && f.writable == f.ok && f.buf.is_empty()",
                           "!file_readable(result) && file_writable(result) == file_ok(result) && file_size(result) == 0")
        return s._fn(name, params, "pruv_File", body + ["f"])

    def _proc_text(s, op, name):
        """G10 proc A2: thin helpers over the Process runtime (codegen.PROC_PRELUDE). Failure stays in the owned handle;
        runtime requires follow --foreign-checks, the ensures (mode all) re-check the public state of the result."""
        P, A = "pruv_Process", "pruv_ProcessArgs"
        if op == "proc_args_new":
            body = ["let pruv_r: %s = pruv_proc_args_new();" % A]
            body += s._ens("pruv_r.args.len() == 0", "proc_args_len(result) == 0")
            return s._fn(name, "", A, body + ["pruv_r"])
        if op == "proc_args_push":
            body = s._req("(a.args.len() as u64) < (1u64 << 40)", "proc_args_len(a) < 2^40")
            if s.ens:
                body += ["let pruv_n: usize = a.args.len();"]
            body += ["let pruv_r: %s = pruv_proc_args_push(a, arg);" % A]
            body += s._ens("pruv_r.args.len() == pruv_n + 1", "proc_args_len(result) == proc_args_len(a) + 1")
            return s._fn(name, "a: %s, arg: &str" % A, A, body + ["pruv_r"])
        if op == "proc_args_len":
            body = ["let pruv_r: i64 = a.args.len() as i64;"]
            body += s._ens("pruv_r >= 0 && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40")
            return s._fn(name, "a: &%s" % A, "i64", body + ["pruv_r"])
        if op == "proc_spawn":
            body = ["let pruv_r: %s = pruv_proc_spawn(program, args, cwd);" % P]
            body += s._ens("if pruv_r.error == 0 { pruv_r.stdin_open } else { (pruv_r.error == 1 || pruv_r.error == 2) && pruv_proc_wf(&pruv_r) }",
                           "ok(result) && open(result) || !ok(result) && !open(result) && (err(result) == 1 || err(result) == 2)")
            body += s._ens("pruv_r.stdout_buf.is_empty() && pruv_r.stderr_buf.is_empty() && !pruv_r.stdout_eof && !pruv_r.stderr_eof",
                           "both public buffers empty and both EOFs false")
            return s._fn(name, "program: &str, args: &%s, cwd: &str" % A, P, body + ["pruv_r"])
        if op in ("proc_write_str", "proc_write_bytes"):
            text = op == "proc_write_str"
            body = s._req("p.error == 0 && p.stdin_open", "proc_ok(p) && proc_stdin_open(p)")
            body += ["let pruv_r: %s = pruv_proc_write(p, %s);" % (P, "text.as_bytes()" if text else "data")]
            body += s._ens("(pruv_r.error == 0 || pruv_r.error == 4) && pruv_proc_wf(&pruv_r)",
                           "ok(result) || err(result) == 4")
            return s._fn(name, "p: %s, %s" % (P, "text: &str" if text else "data: &[u8]"), P, body + ["pruv_r"])
        if op == "proc_close_stdin":
            body = s._req("p.error == 0 && p.stdin_open", "proc_ok(p) && proc_stdin_open(p)")
            body += ["let pruv_r: %s = pruv_proc_close(p);" % P]
            body += s._ens("(pruv_r.error == 0 && !pruv_r.stdin_open) || (pruv_r.error == 4 && pruv_proc_wf(&pruv_r))",
                           "ok(result) && !open(result) || err(result) == 4")
            return s._fn(name, "p: %s" % P, P, body + ["pruv_r"])
        if op == "proc_read":
            body = s._req("p.error == 0", "proc_ok(p)")
            body += s._req("stream == 1 || stream == 2", "stream == 1 || stream == 2")
            body += s._req("0 < max && max <= (1i64 << 40)", "0 < max && max <= 2^40")
            body += ["let pruv_r: %s = pruv_proc_read(p, stream, max);" % P]
            body += s._ens("(pruv_r.error == 0 || pruv_r.error == 3) && pruv_proc_wf(&pruv_r)", "ok(result) || err(result) == 3")
            body += s._ens("pruv_r.error != 0 || { let pruv_s: usize = if stream == 1 { pruv_r.stdout_buf.len() } else { pruv_r.stderr_buf.len() }; "
                           "let pruv_e: bool = if stream == 1 { pruv_r.stdout_eof } else { pruv_r.stderr_eof }; "
                           "(pruv_s as i64) <= max && pruv_e == (pruv_s == 0) }",
                           "size(result, stream) <= max && eof(result, stream) == (size(result, stream) == 0)")
            return s._fn(name, "p: %s, stream: i64, max: i64" % P, P, body + ["pruv_r"])
        if op == "proc_read_all":
            body = s._req("p.error == 0 && !p.stdin_open", "proc_ok(p) && !proc_stdin_open(p)")
            body += ["let pruv_r: %s = pruv_proc_read_all(p);" % P]
            body += s._ens("(pruv_r.error == 0 && !pruv_r.stdin_open && pruv_r.stdout_eof && pruv_r.stderr_eof) || "
                           "(pruv_r.error == 3 && pruv_proc_wf(&pruv_r))",
                           "ok(result) && both EOFs true || err(result) == 3")
            return s._fn(name, "p: %s" % P, P, body + ["pruv_r"])
        if op == "proc_bytes":
            body = s._req("stream == 1 || stream == 2", "stream == 1 || stream == 2")
            body += ["let pruv_r: ::std::vec::Vec<u8> = pruv_proc_buf(p, stream).clone();"]
            body += s._ens("pruv_r[..] == pruv_proc_buf(p, stream)[..]", "result is the public buffer of the stream")
            return s._fn(name, "p: &%s, stream: i64" % P, "::std::vec::Vec<u8>", body + ["pruv_r"])
        if op in ("proc_wait", "proc_kill"):
            body = ["let pruv_r: ::std::result::Result<i64, i64> = pruv_proc_finish(p, %s);" % ("true" if op == "proc_kill" else "false")]
            body += s._ens("match pruv_r { Ok(_) => true, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 6 }", "Err carries an error code 1..6")
            return s._fn(name, "p: %s" % P, "::std::result::Result<i64, i64>", body + ["pruv_r"])
        if op == "proc_ok":
            return s._fn(name, "p: &%s" % P, "bool", ["p.error == 0"])
        if op == "proc_stdin_open":
            return s._fn(name, "p: &%s" % P, "bool", ["p.stdin_open"])
        if op == "proc_error":
            body = ["let pruv_r: i64 = p.error;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 6", "0 <= result && result <= 6")
            return s._fn(name, "p: &%s" % P, "i64", body + ["pruv_r"])
        if op == "proc_eof":
            body = s._req("stream == 1 || stream == 2", "stream == 1 || stream == 2")
            body += ["let pruv_r: bool = if stream == 1 { p.stdout_eof } else if stream == 2 { p.stderr_eof } else { pruv_g6_repr(\"stream\") };",
                     "pruv_r"]
            return s._fn(name, "p: &%s, stream: i64" % P, "bool", body)
        if op == "proc_size":
            body = s._req("stream == 1 || stream == 2", "stream == 1 || stream == 2")
            body += ["let pruv_r: i64 = pruv_proc_buf(p, stream).len() as i64;"]
            body += s._ens("0 <= pruv_r && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40")
            return s._fn(name, "p: &%s, stream: i64" % P, "i64", body + ["pruv_r"])
        if op == "proc_byte":
            cond = "(stream == 1 || stream == 2) && i >= 0 && (i as u64) < (pruv_proc_buf(p, stream).len() as u64)"
            body = s._req(cond, "valid stream and 0 <= i && i < proc_size(p, stream)")
            body += ["let pruv_b: &::std::vec::Vec<u8> = pruv_proc_buf(p, stream);",
                     "let pruv_k: usize = if i >= 0 && (i as u64) < (pruv_b.len() as u64) { i as usize } else { pruv_g6_repr(\"index\") };",
                     "let pruv_r: i64 = pruv_b[pruv_k] as i64;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255")
            return s._fn(name, "p: &%s, stream: i64, i: i64" % P, "i64", body + ["pruv_r"])
        raise Exception("codegen: unknown catalog operation %s" % op)

    def _net_text(s, op, name):
        """G10 net A2: thin helpers over the Socket / Listener runtime (codegen.NET_PRELUDE). Failure stays in the owned handle;
        runtime requires follow --foreign-checks, the ensures (mode all) re-check the public state of the result."""
        S, L = "pruv_Socket", "pruv_Listener"
        RES = "::std::result::Result<i64, i64>"
        VEC = "::std::vec::Vec<u8>"
        # mode all: the entry state the preservation ensures compare against
        snap = []
        if s.ens:
            snap = ["pruv_g6_charge(&mut pruv_budget, s.buf.len());",
                    "let pruv_sb: ::std::vec::Vec<u8> = s.buf.clone();",
                    "let pruv_se: bool = s.eof;",
                    "let pruv_sw: bool = s.write_open;"]
        keep = "pruv_r.eof == pruv_se && pruv_r.buf == pruv_sb"
        # the reads replace the buffer: only the flags are compared, no buffer copy is charged
        snap_flags = snap[2:]
        if op == "net_listen":
            body = s._req("port >= 0 && port <= 65535", "0 <= port && port <= 65535")
            body += ["let pruv_r: %s = pruv_net_listen(host, port);" % L]
            body += s._ens("pruv_net_lwf(&pruv_r)",
                           "lok(result) && 1 <= port(result) && port(result) <= 65535 || !lok(result) && port(result) == 0")
            return s._fn(name, "host: &str, port: i64", L, body + ["pruv_r"])
        if op == "net_accept":
            body = s._req("l.error == 0", "lok(l)")
            body += ["let pruv_r: %s = pruv_net_accept(l);" % S]
            body += s._ens("(if pruv_r.error == 0 { pruv_r.write_open } else { pruv_r.error == 5 }) && pruv_net_wf(&pruv_r) && "
                           "pruv_r.buf.is_empty() && !pruv_r.eof",
                           "ok(result) && open(result) || err(result) == 5, empty buffer, EOF false")
            return s._fn(name, "l: &%s" % L, S, body + ["pruv_r"])
        if op == "net_listener_close":
            body = ["let pruv_r: %s = pruv_net_lclose(l);" % RES]
            body += s._ens("match pruv_r { Ok(pruv_v) => pruv_v == 0, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 2 }",
                           "Ok(0) or Err carries a listener error code 1..2")
            return s._fn(name, "l: %s" % L, RES, body + ["pruv_r"])
        if op == "net_connect":
            body = s._req("port >= 1 && port <= 65535", "1 <= port && port <= 65535")
            body += ["let pruv_r: %s = pruv_net_connect(host, port);" % S]
            body += s._ens("(if pruv_r.error == 0 { pruv_r.write_open } else { pruv_r.error == 1 || pruv_r.error == 2 }) && "
                           "pruv_net_wf(&pruv_r) && pruv_r.buf.is_empty() && !pruv_r.eof",
                           "ok(result) && open(result) || err(result) == 1 || err(result) == 2, empty buffer, EOF false")
            return s._fn(name, "host: &str, port: i64", S, body + ["pruv_r"])
        if op in ("net_write_str", "net_write_bytes"):
            text = op == "net_write_str"
            body = s._req("s.error == 0 && s.write_open", "ok(s) && open(s)")
            body += snap
            body += ["let pruv_r: %s = pruv_net_write(s, %s);" % (S, "text.as_bytes()" if text else "data")]
            body += s._ens("(pruv_r.error == 0 || pruv_r.error == 4 || pruv_r.error == 6) && pruv_net_wf(&pruv_r)",
                           "ok(result) || err(result) == 4 || err(result) == 6")
            body += s._ens("pruv_r.error != 0 || (pruv_r.write_open == pruv_sw && %s)" % keep,
                           "write-open, buffer and EOF preserved")
            return s._fn(name, "s: %s, %s" % (S, "text: &str" if text else "data: &[u8]"), S, body + ["pruv_r"])
        if op == "net_shutdown_write":
            body = s._req("s.error == 0 && s.write_open", "ok(s) && open(s)")
            body += snap
            body += ["let pruv_r: %s = pruv_net_shutdown(s);" % S]
            body += s._ens("!pruv_r.write_open && (pruv_r.error == 0 || (pruv_r.error == 4 && pruv_net_wf(&pruv_r)))",
                           "!open(result) && (ok(result) || err(result) == 4)")
            body += s._ens("pruv_r.error != 0 || (%s)" % keep, "buffer and EOF preserved")
            return s._fn(name, "s: %s" % S, S, body + ["pruv_r"])
        if op == "net_set_timeout":
            body = s._req("s.error == 0", "ok(s)")
            body += s._req("ms >= 0 && ms <= 86400000", "0 <= ms && ms <= 86400000")
            body += snap
            body += ["let pruv_r: %s = pruv_net_set_timeout(s, ms);" % S]
            body += s._ens("pruv_r.error == 0 || (pruv_r.error == 4 && pruv_net_wf(&pruv_r))", "ok(result) || err(result) == 4")
            body += s._ens("pruv_r.error != 0 || (pruv_r.write_open == pruv_sw && %s)" % keep,
                           "write-open, buffer and EOF preserved")
            return s._fn(name, "s: %s, ms: i64" % S, S, body + ["pruv_r"])
        if op == "net_read":
            body = s._req("s.error == 0", "ok(s)")
            body += s._req("0 < max && max <= (1i64 << 40)", "0 < max && max <= 2^40")
            body += snap_flags
            body += ["let pruv_r: %s = pruv_net_read(s, max);" % S]
            body += s._ens("(pruv_r.error == 0 || pruv_r.error == 3 || pruv_r.error == 6) && pruv_net_wf(&pruv_r)",
                           "ok(result) || err(result) == 3 || err(result) == 6")
            body += s._ens("pruv_r.error != 0 || ((pruv_r.buf.len() as u64) <= (max as u64) && pruv_r.eof == pruv_r.buf.is_empty() && "
                           "pruv_r.write_open == pruv_sw && (!pruv_se || pruv_r.buf.is_empty()))",
                           "size(result) <= max && eof(result) == (size(result) == 0), write-open preserved")
            return s._fn(name, "s: %s, max: i64" % S, S, body + ["pruv_r"])
        if op == "net_read_all":
            body = s._req("s.error == 0", "ok(s)")
            body += snap_flags
            body += ["let pruv_r: %s = pruv_net_read_all(s);" % S]
            body += s._ens("(pruv_r.error == 0 && pruv_r.eof && pruv_r.write_open == pruv_sw) || "
                           "((pruv_r.error == 3 || pruv_r.error == 6) && pruv_net_wf(&pruv_r))",
                           "ok(result) && eof(result) || err(result) == 3 || err(result) == 6")
            return s._fn(name, "s: %s" % S, S, body + ["pruv_r"])
        if op == "net_bytes":
            body = ["let pruv_r: %s = s.buf.clone();" % VEC]
            body += s._ens("pruv_r[..] == s.buf[..]", "result is the public buffer")
            return s._fn(name, "s: &%s" % S, VEC, body + ["pruv_r"])
        if op == "net_close":
            body = ["let pruv_r: %s = pruv_net_close(s);" % RES]
            body += s._ens("match pruv_r { Ok(pruv_v) => pruv_v == 0, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 6 }",
                           "Ok(0) or Err carries an error code 1..6")
            return s._fn(name, "s: %s" % S, RES, body + ["pruv_r"])
        if op == "net_ok":
            return s._fn(name, "s: &%s" % S, "bool", ["s.error == 0"])
        if op == "net_write_open":
            return s._fn(name, "s: &%s" % S, "bool", ["s.write_open"])
        if op == "net_eof":
            return s._fn(name, "s: &%s" % S, "bool", ["s.eof"])
        if op == "net_error":
            body = ["let pruv_r: i64 = s.error;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 6", "0 <= result && result <= 6")
            return s._fn(name, "s: &%s" % S, "i64", body + ["pruv_r"])
        if op == "net_size":
            body = ["let pruv_r: i64 = s.buf.len() as i64;"]
            body += s._ens("0 <= pruv_r && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40")
            return s._fn(name, "s: &%s" % S, "i64", body + ["pruv_r"])
        if op == "net_byte":
            cond = "i >= 0 && (i as u64) < (s.buf.len() as u64)"
            body = s._req(cond, "0 <= i && i < net_size(s)")
            body += ["let pruv_k: usize = if %s { i as usize } else { pruv_g6_repr(\"index\") };" % cond,
                     "let pruv_r: i64 = s.buf[pruv_k] as i64;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255")
            return s._fn(name, "s: &%s, i: i64" % S, "i64", body + ["pruv_r"])
        if op == "net_listener_ok":
            return s._fn(name, "l: &%s" % L, "bool", ["l.error == 0"])
        if op == "net_listener_error":
            body = ["let pruv_r: i64 = l.error;"]
            body += s._ens("0 <= pruv_r && pruv_r <= 2", "0 <= result && result <= 2")
            return s._fn(name, "l: &%s" % L, "i64", body + ["pruv_r"])
        if op == "net_listener_port":
            body = ["let pruv_r: i64 = l.port;"]
            body += s._ens("(l.error == 0 && pruv_r >= 1 && pruv_r <= 65535) || (l.error != 0 && pruv_r == 0)",
                           "ok implies 1 <= result && result <= 65535; failed implies result == 0")
            return s._fn(name, "l: &%s" % L, "i64", body + ["pruv_r"])
        raise Exception("codegen: unknown catalog operation %s" % op)

    def _async_text(s, op, name):
        args, ret, req, expr, ens = ASYNC_HELPERS[op]
        body = s._req(req, req) if req else []
        if s.ens and ret == "pruv_Loop" and op != "ev_new":
            body += ["pruv_g6_charge(&mut pruv_budget, l.public.len() + l.buf.len());",
                     "let pruv_snap = pruv_ev_snapshot(&l);"]
        body += ["let pruv_r: %s = %s;" % (ret, expr)]
        body += s._ens(ens or "pruv_ev_wf(&pruv_r)", ens or "well-formed loop") if ret == "pruv_Loop" or ens else []
        if s.ens and ret == "pruv_Loop" and op != "ev_new":
            body += s._ens('pruv_ev_post("%s", &pruv_snap, &pruv_r, %s)' % (op, "slot" if "slot:" in args else "-1"), "public state frame")
        body += ["pruv_r"]
        return s._fn(name, args, ret, body)

    # ------------------------------------------------------------------ contract-only scan (requires)
    def map_updated_helper(s, kt, vt, src):
        s._note_catalog("map_updated", src)
        name = "pruv_g6_map_updated" + "".join("__" + encode(safe_type(t)) for t in src)
        if name in s.items:
            return name
        s.op = "map_updated"
        M = rty(Type("hashmap", (kt, vt)))
        K = rty(kt)
        keyconv = "i64::try_from(pruv_kk).ok()" if kt == I64 else "Some(pruv_kk)"
        L = [s.g.allow,
             "fn %s(pruv_after: &%s, pruv_before: &%s, pruv_kk: %s, pruv_vv: %s, pruv_budget: &mut i64) -> bool {" % (
                 name, M, M, cty(kt), cty(vt)),
             "    let pruv_nk: %s = match %s { Some(pruv_x) => pruv_x, None => return false };" % (K, keyconv),
             "    let pruv_was: bool = pruv_before.contains_key(&pruv_nk);",
             "    if pruv_after.len() != pruv_before.len() + (if pruv_was { 0 } else { 1 }) { return false; }",
             "    match pruv_after.get(&pruv_nk) { Some(pruv_a) => { if !(%s == pruv_vv) { return false; } } "
             "None => { return false; } }" % conv(vt, "(*pruv_a)"),
             "    for (pruv_bk, pruv_bv) in pruv_before.iter() {",
             "        *pruv_budget -= 1; if *pruv_budget < 0 { pruv_ud(\"contract iteration budget exceeded\"); }",
             "        if *pruv_bk != pruv_nk {",
             "            match pruv_after.get(pruv_bk) { Some(pruv_av) => { if !(%s == %s) { return false; } } "
             "None => { return false; } }" % (conv(vt, "(*pruv_av)"), conv(vt, "(*pruv_bv)")),
             "        }",
             "    }",
             "    true",
             "}"]
        s.items[name] = "\n".join(L) + "\n"
        s.catalog.append({"op": "map_updated", "type_args": [format_type(t) for t in src]})
        return name

    # ------------------------------------------------------------------ trusted rust fn
    def trusted_helper(s, f):
        """private helper `NAME`: selected requires, catch_unwind around the Rust call, selected ensures"""
        name = f.name
        if name in s.items:
            return name
        g = s.g
        s.op = name
        disp = getattr(f, "disp", name)     # G4 A2: messages name the source identity, the helper keeps its stem
        saved = (g.params, g.vt, g.cp, g.bound, g.tmp, g.borrowed)
        g.params = set(p.name for p in f.params) | {"result"}
        g.vt = dict((p.name, p.type) for p in f.params)
        g.cp, g.bound, g.tmp, g.borrowed = None, [], 0, set()
        try:
            body = []
            if s.req:
                for r in f.requires:
                    body.append("if !(%s) { pruv_g6_req(%s, %s); }" % (g.c(r), rstr(disp), rstr(show(r))))
            args = ", ".join(p.name for p in f.params)
            rt = rty(f.ret)
            body.append("let result: %s = match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| %s(%s))) {" % (
                rt, f.foreign["path"], args))
            body.append("    Ok(pruv_r) => pruv_r,")
            body.append("    Err(_) => panic!(%s)," % rstr("pruv: foreign panic: " + disp))
            body.append("};")
            if is_collection(f.ret):    # G6b: an owned result's length fits the 2^40 model in every check mode
                body.append("if (result.len() as u128) > (1u128 << 40) { pruv_g6_repr(%s); }"
                            % rstr(disp + ": result length"))
            if s.ens:
                for x in f.ensures:
                    body.append("if !(%s) { pruv_g6_ens(%s, %s); }" % (g.c(x), rstr(disp), rstr(show(x))))
            body.append("result")
            ps = ", ".join("%s: %s" % (p.name, rty(p.type, True)) for p in f.params)
            text = s._fn_trusted(name, ps, None if f.ret == UNIT else rt, body, disp)
        finally:
            g.params, g.vt, g.cp, g.bound, g.tmp, g.borrowed = saved
        s.items[name] = text
        return name

    def _fn_trusted(s, name, params, ret, body, disp=None):
        g = s.g
        L = [g.allow, "fn %s(%s)%s {" % (name, params, "" if ret is None else " -> " + ret)]
        if s.req:
            L.append("    let pruv_g = pruv_ctx(%s);" % rstr(name if disp is None else disp))
            L.append("    " + _budget_line(g.budget))   # shared by requires and ensures of one call
        L.extend("    " + x for x in body)
        L.append("}")
        return "\n".join(L) + "\n"

ASYNC_HELPERS = {'ev_new': ('cap: i64', 'pruv_Loop', 'cap >= 1 && cap <= 4096', 'pruv_ev_new(cap)', 'pruv_ev_wf(&pruv_r) && pruv_ev_count(&pruv_r)==0'), 'ev_close': ('l: pruv_Loop', '::std::result::Result<i64, i64>', '', 'pruv_ev_close(l)', 'match pruv_r {Ok(n)=>n>=0,Err(e)=>(1..=8).contains(&e)}'), 'ev_add_socket': ('l: pruv_Loop, s: pruv_Socket', 'pruv_Loop', 'l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && s.error == 0', 'pruv_ev_socket(l, s)', ''), 'ev_add_listener': ('l: pruv_Loop, x: pruv_Listener', 'pruv_Loop', 'l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && x.error == 0', 'pruv_ev_listener(l, x)', ''), 'ev_add_process': ('l: pruv_Loop, p: pruv_Process', 'pruv_Loop', 'l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && p.error == 0', 'pruv_ev_process(l, p)', ''), 'ev_add_timer': ('l: pruv_Loop, ms: i64', 'pruv_Loop', 'l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && ms >= 0 && ms <= 86400000', 'pruv_ev_timer(l, ms)', ''), 'ev_wait': ('l: pruv_Loop, timeout_ms: i64', 'pruv_Loop', 'l.error == 0 && timeout_ms >= -1 && timeout_ms <= 86400000', 'pruv_ev_wait(l, timeout_ms)', ''), 'ev_read': ('l: pruv_Loop, slot: i64, max: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 1) && max > 0 && max <= (1i64 << 40)', 'pruv_ev_read(l, slot, 1, max, false)', ''), 'ev_proc_read': ('l: pruv_Loop, slot: i64, stream: i64, max: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 3) && max > 0 && max <= (1i64 << 40) && (stream == 1 || stream == 2)', 'pruv_ev_read(l, slot, stream, max, true)', ''), 'ev_write': ('l: pruv_Loop, slot: i64, data: &[u8]', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 1) && l.public[slot as usize].open', 'pruv_ev_write(l, slot, data, 0)', ''), 'ev_shutdown_write': ('l: pruv_Loop, slot: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 1) && l.public[slot as usize].open', 'pruv_ev_write(l, slot, &[], 1)', ''), 'ev_proc_write': ('l: pruv_Loop, slot: i64, data: &[u8]', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 3) && l.public[slot as usize].open', 'pruv_ev_write(l, slot, data, 2)', ''), 'ev_proc_close_stdin': ('l: pruv_Loop, slot: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 3) && l.public[slot as usize].open', 'pruv_ev_write(l, slot, &[], 3)', ''), 'ev_accept': ('l: pruv_Loop, slot: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 2) && pruv_ev_count(&l) < l.slots.len() as i64', 'pruv_ev_accept(l, slot)', ''), 'ev_proc_wait': ('l: pruv_Loop, slot: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 3)', 'pruv_ev_remove(l, slot, true)', ''), 'ev_remove': ('l: pruv_Loop, slot: i64', 'pruv_Loop', 'pruv_ev_valid(&l, slot, 0) && l.public[slot as usize].kind != 0', 'pruv_ev_remove(l, slot, false)', ''), 'ev_ok': ('l: &pruv_Loop', 'bool', '', 'l.error == 0', ''), 'ev_error': ('l: &pruv_Loop', 'i64', '', 'l.error', ''), 'ev_cap': ('l: &pruv_Loop', 'i64', '', 'l.public.len() as i64', ''), 'ev_count': ('l: &pruv_Loop', 'i64', '', 'l.public.iter().filter(|p|p.kind!=0).count() as i64', ''), 'ev_last': ('l: &pruv_Loop', 'i64', '', 'l.last', ''), 'ev_event_slot': ('l: &pruv_Loop', 'i64', '', 'l.event_slot', ''), 'ev_event_kind': ('l: &pruv_Loop', 'i64', '', 'l.event_kind', ''), 'ev_size': ('l: &pruv_Loop', 'i64', '', 'l.buf.len() as i64', ''), 'ev_exit': ('l: &pruv_Loop', 'i64', '', 'l.exit', ''), 'ev_bytes': ('l: &pruv_Loop', '::std::vec::Vec<u8>', '', 'l.buf.clone()', ''), 'ev_kind': ('l: &pruv_Loop, slot: i64', 'i64', 'slot >= 0 && slot < l.public.len() as i64', 'l.public[pruv_cev_slot(l, slot as i128)].kind', ''), 'ev_eof': ('l: &pruv_Loop, slot: i64', 'bool', 'slot >= 0 && slot < l.public.len() as i64', 'l.public[pruv_cev_slot(l, slot as i128)].eof', ''), 'ev_write_open': ('l: &pruv_Loop, slot: i64', 'bool', 'slot >= 0 && slot < l.public.len() as i64', 'l.public[pruv_cev_slot(l, slot as i128)].open', ''), 'ev_byte': ('l: &pruv_Loop, i: i64', 'i64', 'i >= 0 && i < l.buf.len() as i64', 'if i >= 0 && i < l.buf.len() as i64 { l.buf[i as usize] as i64 } else { pruv_ud("loop byte out of range") }', '')}

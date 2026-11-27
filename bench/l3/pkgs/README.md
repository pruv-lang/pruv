# L3 shared packages

Fifteen Pruv packages. The first seven (json, jstr, lex, parse, lines, access, tail) were migrated from the flat
modules in `bench/l3/lib` (item bodies unchanged; each root only adds `mod DEP;` lines for the packages it names as
`crate::DEP::`). The other eight moved out of single pieces when a second piece needed them (L3-5, Codex import):

- `bytes` (`copy_span`, from the end of `qwen/boundary.pruv`), `text`, `names`, `time`, `turns` from `bench/l3/qwen`;
  `turns` now calls `crate::bytes::copy_span` instead of `crate::boundary::copy_span`.
- `args`, `transport` from `bench/l3/mcp`; `transport::receive` takes the line cap as a parameter
  (`1 <= cap <= 8388608`; mcp passes 4194304, codex 8388608).
- `framing` (4.4i) from `bench/l3/claude` and `bench/l3/qwen` (byte-identical copies); it now owns
  `limit_line_bytes` / `limit_drop_head`, which were in each `boundary.pruv` and read only by the framing.

Each package has a `pruv.json` (version 0.1.0).

Dependency graph (package -> packages it mounts):

    json   -> (none)          lex    -> (none)          tail   -> (none)
    bytes  -> (none)          text   -> (none)          names  -> (none)
    args   -> (none)
    jstr   -> json            parse  -> json, lex
    lines  -> json, lex, parse
    access -> json, jstr
    time   -> json, jstr, access, text
    turns  -> bytes, text
    transport -> tail
    framing -> lines

Pieces (`bench/l3/{json,mcp,qwen,claude,supervisor,codex}`) compile with `--manifest <root>.pruv.json` (or `pruv.json` when
the folder has one root). Each piece manifest depends only on the packages its own files name directly
(`../pkgs/NAME`). After changing a package, rerun the tests of every piece that depends on it.

`bench/l3/lib` stays unchanged for the `--lib` test row (`g4_frontend/lib`); the pieces no longer use it.

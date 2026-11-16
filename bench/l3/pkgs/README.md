# L3 shared packages

Seven Pruv packages, migrated from the flat modules in `bench/l3/lib` (item bodies unchanged; each root only adds
`mod DEP;` lines for the packages it names as `crate::DEP::`). Each package has a `pruv.json` (version 0.1.0).

Dependency graph (package -> packages it mounts):

    json   -> (none)          lex    -> (none)          tail   -> (none)
    jstr   -> json            parse  -> json, lex
    lines  -> json, lex, parse
    access -> json, jstr

Pieces (`bench/l3/{json,mcp,qwen,supervisor}`) compile with `--manifest <root>.pruv.json` (or `pruv.json` when the
folder has one root). Each piece manifest depends only on the packages its own files name directly (`../pkgs/NAME`).

`bench/l3/lib` stays unchanged for the `--lib` test row (`g4_frontend/lib`); the pieces no longer use it.

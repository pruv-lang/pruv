# L3 shared Pruv library

This directory holds the single copy of the verified modules that several L3 pieces use:
`json`, `jstr`, `lex`, `lines`, `parse`, `access` (JSON/JSONL stack) and `tail` (output tail buffer).

A root file declares a module with `mod NAME;` and is compiled with `--lib bench/l3/lib`. The compiler reads
`NAME.pruv` from the library only when no `NAME.pruv` sits next to the root; having it in both places is an error.

Library modules refer to each other as `crate::OTHER`, so a user must declare the dependencies too:
`lines` needs `json`, `lex`, `parse`; `parse` needs `json`, `lex`; `jstr` needs `json`; `access` needs `json`, `jstr`.

`bench/l3/claude` keeps its own frozen copies (the L3-1 record); do not edit them or point drivers at them.
Pieces that need a variant (for example a mutant) build a temporary copy of this directory and pass that as `--lib`.
Design: `docs/G4B_LIB_SPEC.md` section 4.

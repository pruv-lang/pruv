# L3 shared Pruv library

This directory holds the single copy of the verified modules that several L3 pieces use:
`json`, `jstr`, `lex`, `lines`, `parse`, `access` (JSON/JSONL stack) and `tail` (output tail buffer).
The L3-8 entry (`bench/l3/entry`, shared with `bench/l3/req`) adds the `ent_*` modules and `entry_api` (prefix `ent_` avoids clashes with the `packet`/`path` modules of other pieces).

A root file declares a module with `mod NAME;` and is compiled with `--lib bench/l3/lib`. The compiler reads
`NAME.pruv` from the library only when no `NAME.pruv` sits next to the root; having it in both places is an error.

Library modules refer to each other as `crate::OTHER`, so a user must declare the dependencies too:
`lines` needs `json`, `lex`, `parse`; `parse` needs `json`, `lex`; `jstr` needs `json`; `access` needs `json`, `jstr`.
Entry modules (all refer to each other as `crate::ent_X`, so the root declares all of them plus `entry_api`): `ent_u16` (none), `ent_packet` (u16), `ent_path` (u16), `ent_value` (u16, packet), `ent_state` (u16, packet), `ent_accounts` (u16, packet, path, value), `ent_dispatch` (u16, packet, state, value), `ent_bound` (u16, packet, state, value, dispatch), `ent_canon` (u16, packet, state, value, path, accounts), `ent_sources` (u16, packet, state, value, path, accounts, canon), `entry_api` (u16, packet, state, accounts, bound, canon, dispatch, sources). Exact deps: grep `crate::ent_` in each file. Root example: `bench/l3/entry/root.pruv`.

`bench/l3/claude` keeps its own frozen copies (the L3-1 record); do not edit them or point drivers at them.
Pieces that need a variant (for example a mutant) build a temporary copy of this directory and pass that as `--lib`.
Design: `docs/G4B_LIB_SPEC.md` section 4.

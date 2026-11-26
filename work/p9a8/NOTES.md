# P9a-8 notes (2026-11-27)

Done:
1. Value match/if result type with a type parameter (`Option<T>` binder): value_branch_kind accepts owner-free type
   parameters (R-6w-2, R-6w-6). tests/must_reject/p9_if_generic removed (now accepted).
2. Value match on `map_get_mut`/`vec_at_mut`: checker gate removed; symbolic.rs `mut_accessor` models it as the
   read accessor (no write-back in a value arm).
3a. pat_stmts leaves rename binders inside the arm body (rename_stmts_all) instead of `let t = pruv_patN` copies (R-6u-12).
3b. slice_match: if let / let else slice pattern covering every array follows the R-6u-9 irrefutable rule.
3c. or_groups / or_alt_types / tree_or_types: `type/or_pattern_type` when alternatives give a name different types.
5a. parser distribute_payload_or: `Err(A | B)` -> top-level alternatives, outermost `|` first, max 64.

Not reached (recorded in docs/RUST_PARITY_SPEC.md "P9a-8 remaining"): array binder move out of nested patterns
(R-6e-12a), guards on slice/tuple arms, source-form printing, shared bodies.

Golden updates: mutate cases shift because new test files renumber the temp files (`m%05d.pruv` appears in parse
error JSON); p9_rt_pattern_names output changed because binders are renamed rather than copied.

Audits (2026-11-27): A and B both found the renamed-binder shadow regression (body re-declaring a renamed binder read
the payload); fixed with `body_decls` -> `type/shadow` (tests/must_reject/p9_rename_shadow_{let,match}). A also found
the 64-alternative limit counted rounds; now counts alternatives (tests/must_reject/p9_or_payload_limit). Optional
findings recorded under "P9a-8 remaining" in docs/RUST_PARITY_SPEC.md.
Confirm audit: the guard path (subst_binds, statement and value form) had the same hole; now a value match in a guard
that binds an arm binder again is `type/shadow` (tests/must_reject/p9_rename_shadow_guard{,_value}). Fix 2 boundary
confirmed (64 accepted, 65 rejected).

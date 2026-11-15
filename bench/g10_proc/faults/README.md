# G10 proc compiler fault injections

Each patch is applied alone with `patch -p1` to a disposable copy of the current `pruvc/` under
`bench/g10_proc/.work`; the source compiler is never patched. Run `python3 bench/g10_proc/faults/check.py`.
A fault is detected when a named probe passes on the clean copy and fails on the faulty one.

| Fault | Patch SHA256 prefix | Probe(s) | Result |
|---|---|---|---|
| f1_must_consume.diff | d8d9d40f1f49d09a | reject `g10p_early_return` | detected |
| f2_catalog_requires.diff | e88eeb60bfb2e5ac | reject `g10p_v_bad_stream` | detected |
| f3_snapshot_keep.diff | f167e74d76eb8a19 | local_accept `p_keep_other`, local_reject `p_keep_read` | detected |
| f4_result_tag.diff | 6c7f087f2d816d39 | runtime `g10p_r10_exit_kill` | detected |

Probe kinds: `reject` is a tests/must_reject row (its stage/code JSON); `local_accept` / `local_reject`
are the programs in this folder (verified / rejected with an `assert` diagnostic); `runtime` builds the
tests/runtime driver on the LLVM backend (the Rust shim carries the Result tag) and expects exit 0.
The faults: f1 drops the must-consume check at `return`; f2 drops the `stream` 1-or-2 requires of
`proc_read`; f3 keeps the read stream instead of the other one; f4 swaps the Ok/Err tag of
`proc_wait`/`proc_kill` in the shim. Full commands and outputs are in `../results/faults.json`.

# G10 net compiler fault injections

Each patch is applied alone with `patch -p1` to a disposable copy of the current `pruvc/` under
`bench/g10_net/.work`; the source compiler is never patched. Run `python3 bench/g10_net/faults/check.py`.
A fault is detected when a named probe passes on the clean copy and fails on the faulty one.

| Fault | Patch SHA256 prefix | Probe(s) | Result |
|---|---|---|---|
| f1_must_consume.diff | d8d9d40f1f49d09a | reject `g10n_early_return` | detected |
| f2_catalog_requires.diff | 2dca42f9332442e6 | reject `g10n_v_write_after_shutdown` | detected |
| f3_write_keep.diff | 60e8cfb2136a167c | local_accept `n_keep_write` | detected |
| f4_result_tag.diff | d266c71cb7f08e9e | runtime `g10n_r01_echo_bytes` | detected |

Probe kinds: `reject` is a tests/must_reject row (its stage/code JSON); `local_accept` / `local_reject`
are the programs in this folder (verified / rejected with an `assert` diagnostic); `runtime` builds the
tests/runtime driver on the LLVM backend (the Rust shim carries the Result tag) and expects exit 0.
The faults: f1 drops the must-consume check at `return` (Socket and Listener); f2 drops the
`net_write_open` requires of the write operations; f3 stops a successful write from keeping the received
snapshot; f4 swaps the Ok/Err tag of `net_close`/`net_listener_close` in the shim. Full commands and
outputs are in `../results/faults.json`.

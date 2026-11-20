# M11 typed resident state

The typed lane keeps each session's budget V words in a `Vec<i64>` inside `HashMap<String, Sess>`. Pruv does the
lookup, the card judgment, the store and the insertion ordinals. The Rust harness owns the state, converts between
UTF-16 keys and the Q wire format, and serializes the dump in the ordinal order Pruv assigned.

A `Vec<i64>` field cannot be passed directly as an `[i64]` argument of `bud_card`, so Pruv copies it into an `[i64]`
(`budget_words`). Typed perf includes this copy and the key conversion. Repro: `work/p31t7/w2/repro_vec_slice.pruv`
(not committed; see Appendix P31T (attempt 7) in docs/L3_RESTRUCTURE_SPEC.md).

Run from `bench/l3/budget`:

```sh
python3 m11_test/run_m11.py --typed --pairs py,rs
python3 m11_test/run_m11.py --pairs py,rs
python3 m11_test/run_m11.py --perf --typed
python3 m11_test/run_m11.py --perf
```

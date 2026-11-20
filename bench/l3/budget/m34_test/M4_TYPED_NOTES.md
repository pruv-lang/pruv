# Typed M4

The driver holds one Pruv `St` per initialized Q program. The five maps
assign insertion ordinals from their own monotonic counters. Fresh inserts
return status 2 if incrementing the counter would reach 2^40. Updates retain
their ordinal, and deletion is a single map removal.

Pruv emits the live entries and ordinals using read-only map iteration.
The Rust Q formatter sorts each emitted map by ordinal, including runs
within a clock and activities within a turn. The reply uses `Vec<String>`.
The reply shape locates keys without offset fields or extra offset words.
The codec preserves raw UTF-16 units, including lone surrogates.

The churn check keeps one turn live across 200,000 create/delete cycles and
checks bounded map capacity and monotonic ordinal assignment.

Q checks the live entry count and full observation. The flat-store grow,
import admission, and size fields are outside the typed lane.

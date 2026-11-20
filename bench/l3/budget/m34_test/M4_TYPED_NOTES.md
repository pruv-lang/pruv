# Typed M4

The host's typed resident state holds a generated Pruv `St` per Q program.
The five maps assign insertion ordinals from their own monotonic counters.
Fresh inserts return `Outcome::Invalid` if incrementing the counter would
reach 2^40. Updates retain their ordinal, and deletion is a single map removal.

Each M4 operation has a typed Pruv entry. `Reply` carries an outcome and
named collections of typed records. The Rust Q formatter groups child records
by their parent ordinals and sorts each group by insertion order. The codec
preserves raw UTF-16 units, including lone surrogates.

`run.py m4typed --lane direct|adapter` selects the test lane. The input is
the same serialized Q in both cases. The direct test decodes at its Q boundary
and calls the typed resident path. The adapter accepts the packet and returns
encoded Q results around the same typed entries. Clock timing, waiting flags
and blob lengths are admitted in Pruv. Pruv also admits packed keys and
rejects trailing words. Native text and packed UTF-16 share a key representation
that pairs supplementary characters and escapes lone surrogates.
The direct perf cycle is prepared with typed arguments before the timed loop.

The churn check keeps one turn live across 200,000 create/delete cycles and
checks bounded map capacity and monotonic ordinal assignment.

Q checks the live entry count and full observation. The flat-store grow,
import admission, and size fields are outside the typed lane.

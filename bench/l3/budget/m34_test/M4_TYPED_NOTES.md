# Typed M4

The driver holds one Pruv `St` for each initialized Q program.
Sessions, clocks, and tool turns occupy separate String-keyed maps.
Clocks own their runs, and turns own their activities.
Order vectors preserve live insertion order and compact after deletion when slots exceed twice the live count plus 16.
The codec preserves raw UTF-16 units, including lone surrogates.
R-6f-3 omits `Vec<String>` in entry results, so the reply uses `Vec<Key>` with an owned String per key.
The typed lane checks the live entry count and the complete final observation.
Grow, flat-buffer import admission, and flat-store size fields are skipped.

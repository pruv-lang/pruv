# budget M12 reshape to the hand Rust shape

This file maps the hand baseline `m12_test/hand_life.rs` (the value model of
M12) to the typed Pruv lane `m12_test/root_m12_typed.pruv` and the shared
`bud_tstate.pruv`, following docs/RUST_SHAPE_RESET.md 4.6. Verdicts: S = same
structure, D = structure differs and the language can express the hand shape,
G = structure differs because the language cannot express the hand shape (the
gap row is named).

The port is blocked. The hand keeps every budget as a recursive
`enum Value { .., Array(Vec<Value>), Object(Vec<(Text, Value)>) }` inside
`HashMap<Text, Option<Value>>`, edits it in place through `&mut Value`, and
keeps clocks as `HashMap<Text, Clock>` with `runs: Vec<(Text, bool)>`. The
recursive enum itself compiles (R-6e-20), and so do a local `Vec<Value>` and a
`Vec<Run { key: [u16], waiting: bool }>`. Every container that would hold the
hand state, and every in-place edit of a `Value`, is rejected by the main
compiler (rows M12-01 to M12-11). Under the no-workaround rule the product code
is left as it is: the node arena `Wire { nodes: HashMap<i64, WireNode> }`, the
fixed-field `bud_tstate::Budget`, `ClockState` and the `[i64]` word packets stay.
No function reaches zero structure difference from product state, so nothing
was measured.

## §1 Structure table

Prefixes: R = m12_test/root_m12_typed.pruv, T = bud_tstate.pruv, L = bud_tlife.pruv, E = bud_effect.pruv,
W = bud_vwords.pruv. Hand lines are hand_life.rs unless marked hand.rs (m12_test/hand.rs). D rows are
expressible on their own, but every D row reads or writes state that only exists in the hand shape behind a
G row (a stored Value, a Text-keyed map, or the effect writer), so none can be ported without the G rows.

| hand fn line | Pruv (file:line) | Structure | Note |
|---|---|---|---|
| text 13 | R:32 native_key, R:58 text | D | Hand collects UTF-16 units into an owned Text; the lane builds a reversible String. An owned `[u16]` local compiles. |
| is_text 14 | R:450 str_is, R:907 node_text_is | D | Hand compares borrowed units with encode_utf16(); the lane walks V words or a node String. |
| read_units 15 | R:87 v_units, R:143 read_node | D | Hand consumes n units from a shared cursor; the lane passes position and length and the caller moves the cursor. |
| unpack_text 18 | R:30 decode_admitted_key, R:52 decode_key, W:73 unpack | D | Hand unpacks words into Text; the lane yields a String or a fresh tagged `[i64]`. |
| packed 20 | W:61 packed, bud_keys.pruv:302 pack_units, R:1498 encode_key | D | Hand folds four units per word into one reserved Vec; the lane checks the V range then walks again, and repacks clock keys from str. |
| write_key 29 | R:91 wire_units, R:240 write_text | D | Hand writes into the caller's `&mut Vec<i64>`; the lane builds a temporary Vec and copies it word by word. |
| Value::decode 47 | R:386 decode_budget, R:321 replace_wire, R:143 read_node | G M12-09 | Hand returns an owned Value; the lane validates V, mutates the Wire arena and returns root/ticket. `Float(u64)` payload is rejected. |
| Value::read 51 | R:143 read_node, bud_card.pruv:104 v_end | G M12-09 | Hand recurses on one cursor into owned Vec children; the lane inserts arena nodes and links siblings. |
| Value::write 78 | R:284 write_node, R:240 write_text, R:265 write_synthetic | G M12-09 | Hand walks the borrowed enum into one output; the lane looks up child/next IDs with depth and count caps. |
| Value::encode 103 | R:580 encode_budget, R:318 budget_words, R:61 words_array | G M12-09 | Hand writes once into an empty Vec; the lane copies the built Vec again into `[i64]`. |
| Value::object 109 | R:1295 object_node, W:29 field, W:32 object | G M12-05 | Hand moves (Text, Value) pairs into one Vec; the lane records first child ID and count or concatenates word arrays. |
| Value::get 113 | R:855 find_field, W:8 get | G M12-02 | Hand stops at the last match (rfind) and returns `&Self`; the lane scans forward to the end and returns IDs or a position. |
| Value::put 122 | R:936 replace_field, R:707 card_field, W:35 put | G M12-03 M12-04 M12-05 | Hand turns a non-object into an object and moves the value into the last match; the lane links a new node and deletes the old subtree, or copies both V halves. |
| Value::edit_object 134 | R:1604 charge_budget, R:977 set_wire_number, R:1734 after_clock | G M12-03 M12-04 M12-10 | Hand hands the last child as `&mut Value` to a FnOnce; the lane has per-counter and per-clock branches. |
| Value::remove 146 | R:998 delete_fields, R:1266 no_active_since | G M12-03 M12-04 M12-06 | Hand retains once, dropping duplicates; the lane unlinks the last match and rescans, or copies V halves. |
| Value::string 152 | R:916 node_string, R:955 node_has_tag | G M12-02 | Hand returns `Option<&[u16]>`; node_string copies and returns an empty String for non-strings. |
| Value::is 159 | R:907 node_text_is, R:450 str_is | D | Tag and content check; the lane reads by ID or V offset. A read param `v: Value` with a match compiles. |
| Value::integer 161 | R:925 node_num, R:1217 num | D | Same tag and safe range, 0 on failure; the lane uses a map lookup or word indexing instead of an enum match. |
| Value::truthy 168 | R:891 node_truthy, R:1020 jb_truthy | G M12-09 | Hand reads `Float(u64)` through f64::from_bits; the lane uses tag and bit masks. |
| Value::date 182 | W:48 iso, bud_date.pruv:66 iso_parts, R:1704 edit_clock_wire | D | Hand owns the ISO text in `Value::String`; `Value::Str(units: [u16])` compiles (probe g_a). The lane fills tagged words and converts again on Wire updates. |
| Value::literal 183 | W:26 text, R:1039 atom | D | Hand builds one Text into the variant; the lane joins header and body or inserts a node. |
| Sessions::load 193 | R:386 decode_budget, R:407 load_key, R:333 make_budget | G M12-01 M12-07 | Hand replaces the slot of `HashMap<Text, Option<Value>>`; the lane consumes a ticket, deletes the subtree and updates Budget and ordinals. |
| Sessions::dump 205 | harness_typed.rs:159 over R:571 session_key, R:580 encode_budget | G M12-01 | Hand writes in insertion order into one output; the adapter sorts ordinals and copies each encoded budget. |
| Sessions::observe_session 217 | harness_typed.rs:149 over R:571, R:580 | G M12-01 | Hand writes after `[1]` directly; the lane returns present/root and the adapter encodes. |
| Sessions::budget 224 | R:571 session_key, R:438 session_words | G M12-01 M12-02 M12-07 | Hand looks up by borrowed units and gets `Option<&Value>`; the lane looks up by str and returns a root or a snapshot. |
| Sessions::current 228 | R:583 current_root, R:1935 life, L:216 run_event | G M12-01 M12-02 M12-03 | Hand checks both IDs are strings and returns `Option<&mut Value>`; life compares the cached `b.id`. |
| Sessions::legacy 236 | R:1096 auto_decoded, R:813 set_card_decoded, R:843 query_decoded, R:1128 legacy_effects | G M12-01 M12-07 | Hand encodes at most two candidates into a temporary Store and decodes after; the lane edits the resident Wire. |
| key_text 295 | R:40 admit_packed_key, R:52 decode_key | D | Both branch on empty vs packed key; Text vs reversible String. |
| Effects::push 303 | E:3 push | G M12-10 | Hand reserves a header and lets a FnOnce write into the same Vec, then patches the length; the lane takes a finished `[i64]` and copies it. |
| Effects::value 310 | E:22 value, E:3 push | G M12-09 M12-10 | Hand serialises `&Value` inside the effect buffer; the lane receives a serialised array. |
| Effects::string 314 | W:73 unpack, E:22 value, R:1746 halt_wire | D | Hand writes tag, length and units directly; the lane builds a V string array first and copies it per effect. |
| Effects::mark 321 | E:25 mark | D | Zero-length body on both sides; hand uses an empty writer, the lane an empty array. |
| Effects::tick 322 | R:1263 tick, W:19 pair, E:22 value | D | Same code 2 and integer time; the lane allocates two words then copies. |
| Effects::timer 323 | R:1503 drop_clock, R:1498 encode_key, R:1896 life_timers, E:22 value | D | Hand packs the borrowed bid once; the lane packs a stored String and copies via words_array. |
| Effects::reply 327 | E:28 reply, R:1935 life, R:61 words_array | G M12-09 | Hand writes the borrowed result Value into the owned Vec; the lane records status/grow and copies the whole Vec. |
| Clocks::clock_of 349 | L:216 run_event (246-259), R:1833 life_step | G M12-02 M12-07 M12-08 | Hand uses `entry` on a UTF-16 key and returns `&mut Clock`; the lane checks absence, builds a clock and updates parallel order vectors. |
| Clocks::drop_clock 356 | R:1503 drop_clock, R:1515 compact_clock_order | G M12-06 M12-07 M12-08 | Hand removes and `order.retain`s at once; the lane keeps stale ordinals and compacts at a threshold. |
| Clocks::forget 363 | R:1543 cleanup_clocks, R:1574 cleanup | G M12-06 M12-08 | Hand retains the order vector in place; the lane walks keys and ordinals together and checks the generation. |
| Clocks::dispose 373 | R:1543 cleanup_clocks, R:1574 cleanup | G M12-08 | Hand cancels in order then clears three containers; the lane skips stale ordinals and replaces maps. |
| Clocks::seed_tool 382 | R:1165 seed_tool | G M12-08 | Hand searches `Vec<(Text, Vec<Text>)>` linearly; the lane uses `HashMap<String, ToolTurn>` with nested maps and ordinals. |
| Clocks::observe 394 | harness_typed.rs:238 over R:17-27, T:25-30, R:1165 | G M12-08 | Hand writes along insertion order and counts non-waiting runs; the adapter filters ordinals, sorts and prints cached `active`. |
| Clocks::call 419 | R:1935 life, R:1574 cleanup, R:1465 attach, R:1813 life_charge, R:1833 life_step | G M12-01 M12-07 M12-08 | Hand takes `&mut Sessions`, `&mut Clocks` and a borrowed Value; the lane takes one St, raw words and pre-decoded keys. |
| Clocks::call_words 447 | R:1791 decode_life, R:40 admit_packed_key, R:1935 life | G M12-09 | Hand decodes ref and arg once into owned Values; the lane extracts bid/run and keeps the raw V. |
| Clocks::fire_words 453 | R:2016 fire_timer, R:1255 ref_value | D | Both build an order-preserving reference and call op 7; the lane also unpacks the same bid into a String. |
| Clocks::attach 461 | R:1465 attach, R:1354 attach_fresh, R:1303 fresh_budget_wire | G M12-01 M12-03 M12-06 | Hand borrows the resident slot and clones only `origin`; the lane encodes and slices the whole budget and builds a new Wire. |
| Clocks::charge 512 | R:1813 life_charge, R:1604 charge_budget, T:107 charge_words, T:149 charge_typed | G M12-03 M12-04 | Hand reads five fields of `used` and updates the borrowed object; the lane allocates 10 words and updates fixed fields and the Wire. |
| Clocks::halt 548 | L:279 explicit_halt, L:56 halt, R:1914 life_halted, R:1746 halt_wire, R:1661 running_list | G M12-03 M12-04 M12-05 | Hand decodes the running IDs once and moves the array into effects and budget; the lane builds `Vec<String>` and rewrites the budget. |
| Clocks::run 586 | R:1686 run_view, R:1935 life, L:216 run_event, T:197 update_run, L:124 refresh | G M12-06 M12-08 | Hand searches and edits `Vec<(Text, bool)>` and removes at index; the lane keeps signed ordinals in a map and an active count. |
| Clocks::fire 626 | L:216 run_event, L:137 fire, R:1855 life_settle, R:1896 life_timers, R:1914 life_halted | G M12-03 M12-08 | Hand emits effects from the mutable clock and budget; the lane walks typed effects and updates Wire and packets. |
| reference 660 | R:1255 ref_value | G M12-11 | Hand takes `&[u16]` sid and an owned bid at the pub boundary; the lane unpacks both packed keys and joins two field arrays. |
| charged 666 | R:1286 charged, R:1230 named | G M12-02 | Hand takes `Option<&str>`; the lane takes a bool and an integer code and joins field words. |
| left 672 | T:64 time_left, T:45 bounded | D | Same clamp and 60,000 factor; hand walks the object, the lane reads a cached field. |
| settle 678 | T:78 settle, T:55 settle_time, R:1855 life_settle, R:1704 edit_clock_wire | G M12-03 M12-04 | Hand `since.take()`s and edits the budget object; the lane flips presence bits and fixed fields, then the Wire. |
| begin 695 | T:92 begin, L:108 arm, R:511 timer_budget_first, R:1896 life_timers | G M12-03 M12-10 | Hand writes `Some(now)` and the date and writes timers into the same buffer; the lane stores typed effects and rescans the ref words. |
| Store::auto hand.rs:760 | R:1096 auto_decoded, R:1083 stop_owner, R:1047 stop_auto, R:2065 auto_packet | G M12-07 | Hand drains/splices the V Vec in `HashMap<Vec<i64>, Vec<i64>>`; the lane edits String-keyed Wire sibling links. |
| Store::set_status hand.rs:853 | R:813 set_card_decoded, bud_card.pruv:1387 judge, R:776 stored_card, R:2057 set_card_packet | G M12-07 | Hand clears a reused output and stores a new V; the lane swaps Wire card nodes and returns CardResult. |
| Store::query hand.rs:859 | R:843 query_decoded, R:583 current_root, R:2050 query_packet | G M12-07 | Hand writes from a borrowed budget slice into the output; the lane returns root/bool and the caller encodes. |

### Pruv functions with no hand counterpart (root_m12_typed.pruv)

| Kind | Functions (line) | Hand part doing the same job |
|---|---|---|
| Key entry and representation | 30 decode_admitted_key, 32 native_key, 37 packed_key_ok, 40 admit_packed_key, 52 decode_key, 58 text, 87 v_units, 89 j_units, 91 wire_units, 1498 encode_key | Text owns the units; only the packed boundary packs/unpacks. |
| Arena ownership and sibling links | 94 append_node, 134 link, 208 delete_node, 231 node_next, 321 replace_wire, 991 wire_room | Value and child Vec ownership, assignment and drop. |
| Load preparation and budget projection | 333 make_budget, 372 next_budget_ticket, 375 prepared, 394 prepare_budget | The decoded `Option<Value>` moves into the session slot. |
| Per-node special forms and edit path choice | 257 synthetic_tag, 265 write_synthetic, 707 card_field, 955 node_has_tag, 964 editable_number, 969 number_node, 977 set_wire_number, 1039 atom, 1295 object_node, 1303 fresh_budget_wire | Plain object/integer/string values and last-field assignment. |
| Word buffers and scalar packing | 61 words_array, 259 push_word, 318 budget_words, 438 session_words, 446 rd, 496 at_b, 502 b01, 1211 get, 1213 part, 1215 cat, 1226 number, 1228 numeric | Borrowed values and slices, local scalars, the caller's output Vec. |
| JB read straight into the Wire | 103 j_count, 110 j_bits, 116 safe_int, 125 int_val, 657 packet_tag, 663 packet_field, 687 packet_value, 693 trimmed, 713 card_list, 752 card_object | The hand card path parses JB too; the extra part is the key String per position and arena assembly. |
| Reference pre-decoding | 468 ref_budget_id, 544 decode_ref, 558 ref_id, 1791 decode_life | Read the Value once and borrow children. |
| Public wrappers per key form | 402 load, 566 session, 2025 query, 2030 set_card, 2036 auto, 2044 load_packet, 2050 query_packet, 2057 set_card_packet, 2065 auto_packet | One hand method with its own argument form. |
| Return and branch intermediates | 1034 auto_result, 1083 stop_owner, 1230 named, 1242 reason_code, 1354 attach_fresh, 1813 life_charge | Local slices, Option/Result and direct returns. |
| Clock generations and effect rebuild | 1515 compact_clock_order, 1686 run_view, 1704 edit_clock_wire, 1734 after_clock, 1833 life_step, 1855 life_settle, 1896 life_timers, 1914 life_halted | Immediate removal from the order Vec, direct edits of borrowed Clock and Value, one effect Vec. |
| V rewrite and interim lists | 1266 no_active_since, 1661 running_list, 1746 halt_wire | `retain`, an owned ID list, `budget.put`. |
| Host fact links | 1195 unit, 1199 uuid, 1203 observing, 1207 observe_find, 1500 running_wire | Direct m12_facts calls. |

## §2 Gap rows

Each row was reproduced with the main compiler
(`/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc`, the binary at
the head of this branch). Probe header:
`struct Field { key: [u16], value: Value }` and
`enum Value { Undefined, Null, Bool(b: bool), Integer(n: i64), Str(units: [u16]), Array(items: Vec<Value>), Object(fields: Vec<Field>) }`
(accepted).

| Row | Hand shape (hand_life.rs) | What the compiler says | Blocks |
|---|---|---|---|
| M12-01 | `entries: HashMap<Text, Option<Value>>` (188); `Clocks.entries: HashMap<Text, Clock>` (343) | `HashMap<String, Value>` and `HashMap<String, Doc { root: Value }>`: `type/foreign_owned_type` "String-keyed maps require i64, bool or owned aggregate values". `HashMap<i64, Doc>`: `foreign_owned_type` "Doc cannot be a Vec or HashMap element: it is recursive or holds an array/str" | Sessions::load/dump/observe_session/budget/current/legacy; any store of a Value budget |
| M12-02 | `&Self`, `Option<&Value>`, `Option<&mut Value>`, `Option<&[u16]>`, `Option<&str>` (113, 152, 224, 228, 349, 666) | `Option<Value>`: `type/type_argument` "Value cannot be a type argument or an Option/Result payload"; `Option<String>`: `foreign_owned_type` "String cannot be a type argument"; returning a child `vec_get::<Field>(fs, i).value`: `foreign_owned_type` "vec_get cannot copy Value" (no borrowed return) | Value::get/string, Sessions::budget/current, Clocks::clock_of, charged |
| M12-03 | `&mut Value` receivers and params: put 122, edit_object 134, remove 146, settle 678, begin 695 | `fn f(inout v: Value)`: `type/inout_type` "inout parameter v cannot have the type Value" (wrapping it in `struct Doc { root: Value }` passes this check only) | Value::put/edit_object/remove, settle, begin, attach, charge, halt, run, fire |
| M12-04 | `if let Value::Object(fields) = self { fields.iter_mut().rfind(..) }` (127, 136, 148) | binder of an enum payload on a mutable place, including `let r = inout d.root; match r { Value::Object(fs) => fs = vec_push(..) }` and a nested match inside a `vec_at_mut` arm: `type/immutable` "fs is not mutable" | Value::put/edit_object/remove (nested edits of `used`, `halt`, `activeSince`) |
| M12-05 | `put(&mut self, key, value: Self)` moves an owned Value in (122); `*slot = Some(Value::decode(..))` (288) | `take v: Value`: `type/foreign_move` "take is only for array, collection or aggregate parameters"; a read param stored into a field compiles to `Clone::clone(value)`; moving an enum field out and restoring it next statement: `foreign_move` "a field moved out must be reassigned by the next statement" (R-6e-16 covers String, Vec<i64>, Vec<bool> and buffer fields only) | Value::put, edit_object callers, legacy |
| M12-06 | `fields.retain(..)` (148), `order.retain(..)` (359, 364, 504), `runs.remove(at)` (610), `entries.remove(..)` returning the value (357, 503) | no `vec_remove` / retain builtin: `unknown_fn` | Value::remove, drop_clock, forget, attach, run |
| M12-07 | `type Text = Vec<u16>` as map key (`HashMap<Text, ..>`, 188, 343); `HashMap<Vec<i64>, Vec<i64>>` (hand.rs:617) | `HashMap<[u16], i64>`: `parse/small_int_unsupported` "[u16] type arguments require P11"; `HashMap<Vec<i64>, i64>`: `foreign_owned_type` "Vec<i64> cannot be a HashMap key: only i64, bool, String and Element type parameters can" | owned UTF-16 keys for Sessions and Clocks (the lane keeps `String` keys); Store::auto/set_status/query |
| M12-08 | `Clock { runs: Vec<(Text, bool)> }` as a map value (334-343); `tool_seen: Vec<(Text, Vec<Text>)>` (345) | `HashMap<String, Clock { owner: String, runs: Vec<Run>, .. }>`: `foreign_owned_type` "String-keyed maps require .. owned aggregate values" (the same Clock with `runs: Vec<i64>` passes); `Vec<Seen { turn: [u16], activities: Vec<Key { units: [u16] } > }>`: `foreign_owned_type` "recursive or holds an array/str" | Clocks::clock_of/drop_clock/forget/dispose/seed_tool/observe, run |
| M12-09 | `Float(u64)` variant (40, 62, 84) | `enum Num { Float(bits: u64), .. }`: `type/small_int_unsupported` "non-i64 integer enum payloads require P11" | Value::decode/read/write/encode/truthy, Effects::value/reply, call_words |
| M12-10 | `impl FnOnce(&mut Self)` and `impl FnOnce(&mut Vec<i64>)` parameters (134, 303) | no function-typed parameter: `fn apply(f: fn(i64) -> i64, ..)` is `parse/syntax` "expected identifier, found 'fn'" | Value::edit_object, Effects::push/value, begin |
| M12-11 | `pub fn reference(sid: &[u16], ..)` (660) | `pub fn reference(sid: [u16])`: `parse/small_int_unsupported` "[u16] at the host boundary is bundle P12" | reference |

## §3 Counts

Table §1: S 0 / D 17 / G 40 (57 rows). Gap rows: M12-01 to M12-11 (11).
No row is S, so no function was measured and there is no perf_cells.json for this bundle.

## §4 Behaviour differences reported by the table draft (checked in HC-10)

The research draft that produced §1 also reported four branch and number differences. HC-10 ran each one
through the Node oracle (`oracle/`) and both M12 lanes (flat and `--typed`) with the hand harness alongside.

Fixed in the lane (the lane was wrong against the oracle):

- Clocks::attach without a string `id` on a non-empty, non-closed budget: the oracle returns the existing
  reference `{sessionId, budgetId: budget.id}` (undefined when absent) with no effects. The lane built a ref
  whose `budgetId` had no value. Both lanes now copy the raw `id` (`bud_vlife::raw_ref`, typed `raw_ref`).
- Clocks::run op 4 with no `run` key: the oracle accepts it and settles the run. The lane returned status 2;
  both lanes now use the empty key, as the hand does.

Known differences (not fixed):

- Hand Clocks::attach 461 creates a new budget in the case above. The hand is wrong against the oracle; the
  hand is a reference port and is not changed by this bundle.
- A missing `run` and `run: ""` share one key in the lane and the hand; they are distinct Map keys in the
  oracle. Reachable only when a caller mixes both forms in one session.
- settle 678 vs T:55: the hand clamps to +-(SAFE+1), the lane to +-SAFE. Past SAFE the oracle gives an unsafe
  number, which is the excluded domain of KD-M6-1 (spec §8.4); neither clamp is compared there.
- Store::query hand.rs:859: an op other than 1 or 2 takes the hand's card query, and the lane returns status 2
  outside 1..3 (the draft's "1..2" is wrong). The oracle API maps query names to 1..3, so other ops exist only
  in the harness wire format.

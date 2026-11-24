# LI1: inline policy by code size (loop condition)

Status: **not ready to merge.** The rule removes the MCP +16 KB code page but regresses MCP time and instructions
at every M/L cell (gate: no cell at <= 1.0x hand may get worse beyond noise). Branch
`lair/compiler-bundle-li1-inline-pol-aad0`, 2026-11-24.

## Rule tried

R-7-3 budget promotion (`calls >= 2 and incl * (calls - 1) <= INLINE_BUDGET`) additionally requires
`incl <= INLINE_LOOP` (64) or no loop line in the fn's own body (a line that, after leading spaces, starts with
`while `, `for ` or `loop {`). Same in `pruvc/inline_policy.py` and `pruv-core/src/inline_policy.rs`; LLVM (R-7-7)
unchanged. Gates passed: `cargo test --release`, `diff_rust.py --fuzz-count 20`, `g8_frontend` on both compilers,
runtime 1378/1378, `tests/check_generated.py` (4 new loop cases).

## MCP packet bench (`bench/l3/mcp/perf/run_perf.py perf`, 5 rounds, lock `/tmp/pruv-perf.lock`, load 4.5-5.5)

Two back-to-back runs, NEW then OLD compiler; hand/old-row/new-row rotate inside each run. Ratios are new-row/hand.

| cell | time OLD pruvc | time NEW pruvc | instr/call OLD | instr/call NEW | RSS OLD | RSS NEW |
|---|---:|---:|---:|---:|---:|---:|
| 0 tools | 1.00x | 1.00x | 19504 | 19602 | 1.009x | 1.000x |
| 1 tool | 1.21x | 1.26x | 26878 | 27936 | 1.010x | 1.000x |
| 10 tools | 0.93x | 0.99x | 93377 | 102800 | 1.009x | 0.991x |
| 50 tools | 0.91x | 0.97x | 393707 | 440824 | 1.007x | 1.000x |
| 100 tools | 0.92x | 0.97x | 769217 | 863496 | 1.006x | 1.000x |
| 250 tools | 0.92x | 1.03x | 1910837 | 2148024 | 1.004x | 1.004x |
| 500 tools | 0.91x | 1.03x | 3815049 | 4290101 | 1.002x | 1.000x |
| 5x100 | 0.95x | 0.99x | 5014093 | 5490794 | 1.001x | 1.000x |

`__text` 295,868 -> 278,972 bytes; `run_new` 12,956 -> 3,028 bytes. RSS reaches hand (the +16 KB page is gone),
but instructions grow 10-12% at M/L sizes: a regression beyond noise (instructions are deterministic).

### Which fn (one-attribute variants of the NEW `gen.rs`, cells 100/500, 3 rounds, instr/call)

| variant | 100 tools | 500 tools | RSS 500 |
|---|---:|---:|---:|
| NEW (full run above) | 863,496 | 4,290,101 | 1.000x |
| NEW + `packet::step` always | 958,203 | 4,765,807 | 1.002x |
| NEW + `packet::field` always | **721,747** | **3,570,938** | 1.000x |
| NEW + `packet::fields4` always | 761,096 | 3,772,151 | 1.000x |
| NEW + the 8 json/jstr/parse/access fns always | 863,570 | 4,290,355 | 1.002x |
| OLD (all of the above always) | 769,217 | 3,815,049 | 1.006x |

`packet::field` (incl 93, 2 sites, both once per call in the wrapper) cannot be hot by itself, and forcing the
hot `step` makes things worse. So the cost is not the call itself: forcing `field` or `fields4` back changes how
rustc/LLVM inline `step` into the `page` loop (caller-size effect; not verified in the asm). A loop/size rule on
the callee alone does not predict it. "NEW + `field` always" beats OLD by 6% instructions and keeps RSS at hand,
but no rule found here selects `field` and `fields4` without `step` (all three: loop, 2-4 sites, incl 93-158).

## Other benches

Not measured: the MCP gate already fails. Attribute changes include the M8 hot path (`bud_map` `put_key_h`,
`entry_matches_w`, `set_blob`, `bud_store::need`), where forced inlining gave the R-7-3 wins, so M8 would need
a measurement before any variant of this rule.

## Next

- Look at the `page` asm in NEW vs NEW+`field` to confirm the `step` inlining decision, then decide between a
  caller-side rule (e.g. never demote a fn called from a fn that holds a hot loop) and leaving R-7-3 as it was.
- Gaps: one-line contract loops `{ let ...; while ... }` are not seen by the loop test (audit).

## Per-bench attribute changes (old -> new pruvc-rs, every bench root that compiles)

| bench root | fn | old | new |
|---|---|---|---|
| `bench/perf/base64/base64.pruv` | `pruv_m_f_1_6_encode__pruv_body` | always | inline |
| `bench/l0/chunk_page.pruv` | `pruv_m_f_1_10_split_into__pruv_body` | always | inline |
| `bench/l0/clip_sheet.pruv` | `pruv_m_f_1_9_draw_text__pruv_body` | always | inline |
| `bench/l0/docx_parts.pruv` | `pruv_m_f_1_11_inline_text` | always | inline |
| `bench/l0/docx_parts.pruv` | `pruv_m_f_1_4_walk` | inline | always |
| `bench/l0/docx_parts.pruv` | `pruv_m_f_1_8_collapse` | always | inline |
| `bench/l0/docx_parts.pruv` | `pruv_m_f_1_8_esc_text` | always | inline |
| `bench/l0/docx_parts.pruv` | `pruv_m_f_1_8_open_tag` | always | inline |
| `bench/l0/extract_version.pruv` | `sha256_hex` | inline | always |
| `bench/l0/extract_version.pruv` | `sha256_hex__pruv_body` | always | inline |
| `bench/l0/group_text.pruv` | `pruv_m_f_1_10_group_into__pruv_body` | always | inline |
| `bench/l0/parse_html.pruv` | `pruv_m_f_1_11_decode_into__pruv_body` | always | inline |
| `bench/l0/parse_html.pruv` | `pruv_m_f_1_8_ent_code__pruv_body` | always | inline |
| `bench/l0/row_pieces.pruv` | `pruv_m_f_1_8_row_into__pruv_body` | always | inline |
| `bench/l0/sheet_chunks.pruv` | `pruv_m_f_1_11_chunks_into__pruv_body` | always | inline |
| `bench/l0/split_text.pruv` | `pruv_m_f_1_10_split_into__pruv_body` | always | inline |
| `bench/l0/tokenize.pruv` | `pruv_m_f_1_4_emit__pruv_body` | always | inline |
| `bench/l3/json/a2_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_17_string_text_owned__pruv_body` | inline | always |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_19_string_text_scanned__pruv_body` | inline | always |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_25_string_text_scanned_owned__pruv_body` | inline | always |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/json/a3_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_6_access_13_field_if_used` | inline | always |
| `bench/l3/json/a4_root.pruv` | `pruv_p_f_2_6_access_5_field` | always | inline |
| `bench/l3/json/a5_root.pruv` | `pruv_p_f_2_4_json_10_check_tape` | always | inline |
| `bench/l3/json/a5_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/a5_root.pruv` | `pruv_p_f_2_5_parse_4_scan__pruv_body` | always | inline |
| `bench/l3/json/b_root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/json/b_root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/json/b_root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/json/b_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_m_f_1_9_canonical` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_4_json_10_check_tape` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/json/q_root.pruv` | `pruv_p_f_2_5_parse_4_scan__pruv_body` | always | inline |
| `bench/l3/claude/a0_root.pruv` | `pruv_p_f_2_4_json_10_check_tape` | always | inline |
| `bench/l3/claude/a0_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/claude/a0_root.pruv` | `pruv_p_f_2_5_parse_4_scan__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_1_9_ctl_ok_at__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_4_path_4_join__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_4_path_9_env_value__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_4_plan_6_select__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_4_util_6_str_at` | inline | always |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_5_entry_4_feed__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_5_entry_7_text_of` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_5_entry_9_iso_bytes` | inline | always |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_6_packet_12_result_shape__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_6_packet_7_plan_ok__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_m_f_2_6_packet_9_ok_result__pruv_body` | inline | always |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_4_text_8_one_line__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_4_text_9_trim_span` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/opencode/root.pruv` | `pruv_p_f_2_5_turns_7_reserve__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_m_f_2_5_entry_12_string_field` | inline | always |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_json_10_check_tape` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_text_8_one_line__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_4_text_9_trim_span` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_5_lines_11_parse_range__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_5_parse_4_scan__pruv_body` | always | inline |
| `bench/l3/qwen/root.pruv` | `pruv_p_f_2_5_turns_7_reserve__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_m_f_2_4_util_5_field` | inline | always |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_m_f_2_6_packet_4_step__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_m_f_2_6_packet_5_field__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_m_f_2_6_packet_7_fields4__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_4_json_10_check_tape` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_4_jstr_11_string_utf8__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_4_jstr_19_string_utf8_scanned__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_4_jstr_4_step__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_5_lines_11_parse_range__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_5_parse_12_string_token__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_5_parse_4_scan__pruv_body` | always | inline |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_6_access_13_field_if_used` | inline | always |
| `bench/l3/mcp/perf/work/perf_root.pruv` | `pruv_p_f_2_6_access_5_field` | always | inline |
| `bench/l3/entry/root.pruv` | `entry_begin` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_10_ent_packet_11_rows1_check` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_10_ent_packet_12_env_check_at` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_10_ent_packet_14_env_check_wire` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_10_ent_packet_9_env_check` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_11_ent_sources_8_emit_row` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_11_ent_sources_8_home_row` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_12_ent_accounts_16_valid_account_id` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_12_ent_accounts_8_io_check` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_8_ent_path_16_normalize_string` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_8_ent_wire_7_pool_ok` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_canon_11_resolve_cwd` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_canon_11_state_ok_ie` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_state_6_log_ok` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_10_key_put_at` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_10_span_units` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_11_append_span__pruv_body` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_11_decode_into__pruv_body` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_11_pstr_eq_lit` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_12_prefix_bytes` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_14_pstr_eq_lit_at` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_7_key_put` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_7_span_ok` | always | inline |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_8_pool_cat` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_ent_value_9_str_units` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_entry_api_10_index_step` | inline | always |
| `bench/l3/entry/root.pruv` | `pruv_m_f_2_9_entry_api_19_index_begin_checked` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_1_3_bad` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_1_7_done_ev` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_1_8_done_req` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_1_9_done_same` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_1_9_read_done` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_10_ent_packet_11_rows1_check` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_10_ent_packet_12_env_check_at` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_10_ent_packet_14_env_check_wire` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_10_ent_packet_9_env_check` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_10_req_packet_2_mk` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_11_req_sources_5_tlens` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_12_ent_accounts_16_valid_account_id` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_12_ent_accounts_8_io_check` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_8_ent_path_16_normalize_string` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_8_ent_wire_7_pool_ok` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_8_req_fill_10_title_head` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_canon_11_resolve_cwd` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_canon_11_state_ok_ie` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_state_6_log_ok` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_10_key_put_at` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_10_span_units` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_11_append_span__pruv_body` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_11_decode_into__pruv_body` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_12_prefix_bytes` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_14_pstr_eq_lit_at` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_7_key_put` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_7_span_ok` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_ent_value_9_str_units` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_entry_api_19_index_begin_checked` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_req_value_5_units__pruv_body` | inline | always |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_req_world_5_apply` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_req_world_8_world_ok` | always | inline |
| `bench/l3/req/root.pruv` | `pruv_m_f_2_9_req_world_9_open_find` | always | inline |
| `bench/l3/budget/m34_test/root_m3.pruv` | `pruv_m_f_2_12_bud_test_abi_11_effects_end` | always | inline |
| `bench/l3/budget/m34_test/root_m3.pruv` | `pruv_m_f_2_12_bud_test_abi_11_put_effects` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_15_entry_matches_w` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_4_room` | inline | always |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_6_room_w` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_8_set_blob` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_9_make_room` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_7_bud_map_9_put_key_h` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_9_bud_store_10_init_store` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `pruv_m_f_2_9_bud_store_4_need` | always | inline |
| `bench/l3/budget/m34_test/root_admit.pruv` | `root_pay_diff` | inline | always |
| `bench/l3/budget/m34_test/root_admit.pruv` | `root_pay_diff__pruv_body` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `m4_init` | inline | always |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_7_bud_map_15_entry_matches_w` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_7_bud_map_4_room` | inline | always |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_7_bud_map_6_room_w` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_7_bud_map_8_set_blob` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_7_bud_map_9_put_key_h` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_store_10_init_store` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_store_4_need` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_world_10_emit_owner__pruv_body` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_world_7_keys_of` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_world_8_blob_out__pruv_body` | always | inline |
| `bench/l3/budget/m34_test/root_m4.pruv` | `pruv_m_f_2_9_bud_world_8_emit_key__pruv_body` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_1_7_cur_off` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_7_bud_map_15_entry_matches_w` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_7_bud_map_4_room` | inline | always |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_7_bud_map_6_seek_w` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_7_bud_map_8_set_blob` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_7_bud_map_9_put_key_h` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_store_10_init_store` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_store_4_need` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_10_clock_keys` | inline | always |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_10_emit_owner__pruv_body` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_12_session_keys` | inline | always |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_7_keys_of` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_8_blob_out__pruv_body` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_8_emit_key__pruv_body` | always | inline |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_9_tool_list` | inline | always |
| `bench/l3/budget/m8_test/root_m8.pruv` | `pruv_m_f_2_9_bud_world_9_turn_keys` | inline | always |
| `bench/l3/budget/m8_test/root_m8.pruv` | `t_init` | inline | always |

Not compared (34 rows: missing inputs, must_reject tests, or roots that need a prepared work dir): `bench/g10_proc/faults/p_keep_read.pruv`, `bench/l3/budget/m10_test/root_m10.pruv`, `bench/l3/budget/m10_test/root_m10_typed.pruv`, `bench/l3/budget/m10b_test/root_m10b_typed.pruv`, `bench/l3/budget/m11_test/work/m11_typed/src/root_m11_typed.pruv`, `bench/l3/budget/m7_test/root.pruv`, `bench/l3/budget/m7_test/root_m7_typed.pruv`, `bench/l3/budget/m9_test/root_m9.pruv`, `bench/l3/budget/m9_test/root_m9_typed.pruv`, `bench/l3/budget/work/m11/src/root.pruv`, `bench/l3/budget/work/m34/pkg_m4eq/root_m4.pruv`, `bench/l3/budget/work/m34/pkg_m4nv/root_m4.pruv`, `bench/l3/budget/work/m34/pkg_m4raw/root_m4.pruv`, `bench/l3/budget/work/m6/src/root.pruv`, `bench/l3/budget/work/p01_keys/old_src/bench/l3/budget/p01_keys/keys.pruv`, `bench/l3/budget/work/perf_compiler/src/root.pruv`, `bench/l3/claude/a1_root.pruv`, `bench/l3/claude/a2_root.pruv`, `bench/l3/claude/a3_root.pruv`, `bench/l3/claude/a4_root.pruv`, `bench/l3/claude/q_root.pruv`, `bench/l3/grok/perf/work/perf_root.pruv`, `bench/l3/grok/perf/work/py-rust/perf_root.pruv`, `bench/l3/json/a2_root.pruv`, `bench/l3/json/a5_root.pruv`, `bench/l3/mcp/root.pruv`, `bench/l3/perf/work/b/src/root.pruv`, `bench/l3/perf/work/c/src/root.pruv`, `bench/l3/qwen/root.pruv`, `bench/l3/supervisor/supervise.pruv`, `tests/must_reject/g10n_early_return.pruv`, `tests/must_reject/g10n_v_write_after_shutdown.pruv`, `tests/must_reject/g10p_early_return.pruv`, `tests/must_reject/g10p_v_bad_stream.pruv`; `bench/l3/grok/root.pruv` and `bench/l3/codex/root.pruv` were skipped (compile over 6 minutes).

#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// Links the generated Pruv module `sol` to the primitive host driver (opencode_host).
// One JSON case per stdin line (see host.rs); one JSON envelope per stdout line; exit 3 on a host protocol error.
// Generated signatures: [i64]/[u8] params are &[i64]/&[u8], returns are Vec<i64>.
struct G;
impl opencode_host::Sol for G {
    fn begin(&self, cfg: &[i64], cfg_text: &[u8]) -> Vec<i64> {
        sol::opencode_begin(cfg, cfg_text)
    }
    fn step(&self, cfg: &[i64], cfg_text: &[u8], ctl: &[i64], banks: &opencode_host::Banks, event: &[i64], completion: &[u8]) -> Vec<i64> {
        sol::opencode_step(
            cfg, cfg_text, ctl,
            &banks.query_words, &banks.query_bytes, &banks.rows, &banks.text, &banks.document, &banks.saved,
            event, completion,
        )
    }
}
fn main() {
    opencode_host::run(&G);
}

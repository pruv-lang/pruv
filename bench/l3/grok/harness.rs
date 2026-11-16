#[path="@SOL@"] mod sol;
// Links the generated Pruv module `sol` to the primitive host driver (grok_host).
// One JSON config per stdin line (see host.rs); one JSON result per stdout line; exit 3 on a host protocol error.
// Generated signatures: [i64]/[u8] params are &[i64]/&[u8], str params are &str, returns are Vec<i64>.
struct G;
impl grok_host::Sol for G {
    fn begin(&self, cfg: &[i64], cfg_text: &[u8]) -> Vec<i64> {
        sol::grok_begin(cfg, cfg_text)
    }
    fn step(&self, cfg: &[i64], cfg_text: &[u8], ctl: &[i64], b: &grok_host::Banks, event: &[i64], completion: &[u8]) -> Vec<i64> {
        sol::grok_step(
            cfg, cfg_text, ctl, &b.dir, &b.rows, &b.text,
            &b.small[0], &b.small[1], &b.small[2], &b.small[3],
            &b.history, event, completion,
        )
    }
    fn load(&self, binary: &str, home: &str, folder: &str, thread: &str, timeout_ms: i64) -> Vec<i64> {
        sol::grok_load(binary, home, folder, thread, timeout_ms)
    }
    fn caller_cleanup(&self, path: &str) -> Vec<i64> {
        sol::grok_caller_cleanup(path)
    }
}
fn main() {
    grok_host::run(&G);
}

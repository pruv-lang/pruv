#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// Links the generated Pruv module `sol` to the entry host driver (entry_host).
// One JSON case per stdin line (see host.rs); one JSON result line per case on stdout.
// Generated signatures: [i64] params are &[i64], returns are Vec<i64>.
struct G;
impl entry_host::Sol for G {
    fn begin(&self, input: &[i64]) -> Vec<i64> {
        sol::entry_begin(input)
    }
    fn step(&self, prev: &[i64], completion: &[i64]) -> Vec<i64> {
        sol::entry_step(prev, completion)
    }
}
fn main() {
    entry_host::run(&G);
}

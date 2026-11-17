#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// Links the generated Pruv request module `sol` to the request host (req_host).
// One scenario JSON per stdin line; one observation JSON line per scenario on stdout.
struct G;
impl req_host::Req for G {
    fn begin(&self, world: &[i64], input: &[i64]) -> Vec<i64> {
        sol::req_begin(world, input)
    }
    fn step(&self, world: &[i64], prev: &[i64], completion: &[i64]) -> Vec<i64> {
        sol::req_step(world, prev, completion)
    }
}
fn main() {
    req_host::run(&G);
}

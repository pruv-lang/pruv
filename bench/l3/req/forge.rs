#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// L3-9 forge harness: the same driver as harness.rs (req_host::run), but wraps the Req trait so one step call can be
// resumed from a TAMPERED prev / world / completion. host.rs never forges; this wrapper is how the core validation
// (and the absence of a driver latest-state check) is exercised.
//
//   FORGE_DUMP    path   append one JSON line per begin/step call: {"n","kind","world","prev","comp","out"} (n = 0 for begin
//                        calls, otherwise the 1-based step number). Used by forge_test.py to capture real await packets.
//   FORGE_TAMPER  JSON   {"at":N,"world":[..]?,"prev":[..]?,"comp":[..]?}  replace the named arguments of the N-th step call
//                        (global, 1-based). The mutation itself is computed by forge_test.py from a dumped honest run.
// The harness also prints, at exit of each step, nothing to stdout (stdout belongs to req_host::run).
use std::cell::Cell;
use std::io::Write;
use serde_json::{json, Value};

struct G { n: Cell<usize>, tamper: Option<Value>, dump: Option<String> }

fn words(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

impl G {
    fn dump(&self, n: usize, kind: &str, world: &[i64], prev: &[i64], comp: &[i64], out: &[i64]) {
        if let Some(p) = &self.dump {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap();
            let l = json!({"n": n, "kind": kind, "world": world, "prev": prev, "comp": comp, "out": out});
            writeln!(f, "{}", l).unwrap();
        }
    }
}

impl req_host::Req for G {
    fn begin(&self, world: &[i64], input: &[i64]) -> Vec<i64> {
        let out = sol::req_begin(world, input);
        self.dump(0, "begin", world, &[], input, &out);
        out
    }
    fn step(&self, world: &[i64], prev: &[i64], completion: &[i64]) -> Vec<i64> {
        let n = self.n.get() + 1;
        self.n.set(n);
        let (mut w, mut p, mut c) = (world.to_vec(), prev.to_vec(), completion.to_vec());
        if let Some(t) = &self.tamper {
            if t["at"].as_u64().unwrap() as usize == n {
                if let Some(v) = t.get("world") { w = words(v); }
                if let Some(v) = t.get("prev") { p = words(v); }
                if let Some(v) = t.get("comp") { c = words(v); }
            }
        }
        let out = sol::req_step(&w, &p, &c);
        self.dump(n, "step", &w, &p, &c, &out);
        out
    }
}
fn main() {
    let tamper = std::env::var("FORGE_TAMPER").ok().map(|s| serde_json::from_str::<Value>(&s).unwrap());
    let dump = std::env::var("FORGE_DUMP").ok();
    req_host::run(&G { n: Cell::new(0), tamper, dump });
}

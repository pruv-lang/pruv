#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// L3-9 forge harness: the same driver as harness.rs (req_host::run), but wraps the Req trait so one step call can be
// resumed from a TAMPERED prev / world / completion. host.rs never forges; this wrapper is how the core validation
// (and the absence of a driver latest-state check) is exercised.
//
//   FORGE_DUMP    path   append one JSON line per begin/step call: {"n","kind","world","prev","comp","out"} (n = 0 for begin
//                        calls, otherwise the 1-based step number). Used by forge_test.py to capture real await packets.
//   FORGE_TAMPER  JSON   {"at":N,"world":[..]?,"prev":[..]?,"comp":[..]?,"comp5_fill":m?}  replace the named arguments of the N-th step call
//                        (global, 1-based). The mutation itself is computed by forge_test.py from a dumped honest run.
//                        {"at":0,"input":[..]} replaces the begin input.
// The harness also prints, at exit of each step, nothing to stdout (stdout belongs to req_host::run).
use std::cell::Cell;
use std::io::Write;
use serde_json::{json, Value};

struct G { n: Cell<usize>, tamper: Option<Value>, dump: Option<String> }

fn words(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

const LIMIT: i64 = 16777216;

// req packet: 32 header words, w[24] sections, their lengths from word 32, w[25] total. Section 0 is the pool [nu, nb, W..].
fn req_secs(w: &[i64]) -> Vec<Vec<i64>> {
    let ns = w[24] as usize;
    let mut at = 32 + ns;
    (0..ns).map(|i| { let l = w[32 + i] as usize; let s = w[at..at + l].to_vec(); at += l; s }).collect()
}

// entry packet: [1, kind, op_id, seq, len, op, nsec, lens.., sections]
fn ent_secs(w: &[i64]) -> Vec<Vec<i64>> {
    let ns = w[6] as usize;
    let mut at = 7 + ns;
    (0..ns).map(|i| { let l = w[7 + i] as usize; let s = w[at..at + l].to_vec(); at += l; s }).collect()
}

// Pad the pool of the child completion in section 5 with ASCII bytes, unreferenced by any slot. mode 0: the child's
// logical size (len - l0 + nu) is exactly the limit, so the parent is over it. mode 1: the parent's logical size is
// exactly the limit (C2 recheck: section 5 counts by the child's logical size, not its packed length).
fn comp5_fill(c: &[i64], mode: i64) -> Vec<i64> {
    let mut ps = req_secs(c);
    let child = ps[5].clone();
    let mut cs = ent_secs(&child);
    let pool = &cs[0];
    let (nu, nb) = (pool[0], pool[1] as usize);
    let child_logical = child.len() as i64 - pool.len() as i64 + nu;
    let parent_other = c.len() as i64 - child.len() as i64 - ps[0].len() as i64 + ps[0][0];
    let want = if mode == 0 { LIMIT } else { LIMIT - parent_other };
    let k = (want - child_logical) as usize;
    let mut bytes: Vec<u8> = (0..nb).map(|j| ((pool[2 + j / 7] >> (8 * (6 - j % 7))) & 255) as u8).collect();
    bytes.resize(nb + k, b'a');
    let mut o = vec![nu + k as i64, bytes.len() as i64];
    for chunk in bytes.chunks(7) {
        let mut word = 0i64;
        for (i, b) in chunk.iter().enumerate() { word |= (*b as i64) << (8 * (6 - i)); }
        o.push(word);
    }
    cs[0] = o;
    let mut nc: Vec<i64> = vec![child[0], child[1], child[2], child[3], 0, child[5], cs.len() as i64];
    for s in &cs { nc.push(s.len() as i64); }
    for s in &cs { nc.extend_from_slice(s); }
    nc[4] = (nc.len() - 5) as i64;
    ps[5] = nc;
    let mut out: Vec<i64> = c[..32].to_vec();
    for s in &ps { out.push(s.len() as i64); }
    for s in &ps { out.extend_from_slice(s); }
    out[25] = out.len() as i64;
    out
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
        let forged = self.tamper.as_ref().filter(|t| t["at"].as_u64() == Some(0))
            .and_then(|t| t.get("input")).map(words);
        let input = forged.as_deref().unwrap_or(input);
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
                if let Some(v) = t.get("comp5_fill") { c = comp5_fill(&c, v.as_i64().unwrap()); }
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

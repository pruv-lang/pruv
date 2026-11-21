// WA forge harness: the same driver as harness.rs, but entry_step receives a TAMPERED copy of the await packet it is
// resumed from. The host (host.rs) never forges, so this wrapper is how the R1 replay validation is exercised.
//
//   FORGE_TAMPER  JSON {"at": N, "ops": [...]}   tamper the prev of the N-th entry_step call (1-based) with the ops in order
//   FORGE_STATS   1                              print one line per entry_step to stderr:  step prev_len state_len entries
//
// The await packet is [1, kind, op_id, seq, len, op, nsec, len_0.., sections]; section 2 is the state
// [magic, op, code, aux, ck0..ck7, L, G, blob(L), log(G)] and a log entry is [n, rcode, aux, completion packet].
// Ops (indices j of log entries are 0-based; words are absolute inside the named section):
//   {"ck":[i,v]}  {"code":v}  {"aux":v}
//   {"entry_seq":[j,v]} {"entry_op":[j,v]} {"entry_rc":[j,v]}      completion packet seq / op word, entry request code
//   {"drop":j} {"dup":j} {"swap":[i,j]} {"push":[j,seq]}          remove / duplicate / swap / append a copy of entry j with seq
//   {"rec":[k,v]} {"pool":[k,v]} {"tab":[k,v]}                      words of section 1 / 0 / 4 of prev
//   {"blob_pool":[k,v]}                                            word k of the pool of the repacked input in the state
//   "index_swap": true (top level)                                use the op 9 await from swapped temporary slots
//   "index_tail": b (top level)                                   append an unreferenced lone high, byte b, lone low
//                                                                 to the op 9 input pool (b is the separator byte)
use std::cell::Cell;
use serde_json::Value;

pub struct Forge { n: Cell<usize>, tamper: Option<Value>, stats: bool }

fn sec_bounds(w: &[i64]) -> Vec<(usize, usize)> {
    let ns = w[6] as usize;
    let mut at = 7 + ns;
    let mut o = Vec::new();
    for i in 0..ns {
        let l = w[7 + i] as usize;
        o.push((at, at + l));
        at += l;
    }
    o
}

fn rebuild(w: &[i64], secs: &[Vec<i64>]) -> Vec<i64> {
    let ns = secs.len();
    let mut o: Vec<i64> = vec![w[0], w[1], w[2], w[3], 0, w[5], ns as i64];
    for s in secs { o.push(s.len() as i64); }
    for s in secs { o.extend_from_slice(s); }
    o[4] = (o.len() - 5) as i64;
    o
}

fn entries(lg: &[i64]) -> Vec<Vec<i64>> {
    let mut o = Vec::new();
    let mut p = 0;
    while p < lg.len() {
        let n = lg[p] as usize;
        o.push(lg[p..p + n].to_vec());
        p += n;
    }
    o
}

fn tamper(prev: &[i64], t: &Value) -> Vec<i64> {
    let b = sec_bounds(prev);
    let mut secs: Vec<Vec<i64>> = b.iter().map(|&(s, e)| prev[s..e].to_vec()).collect();
    let mut st = secs[2].clone();
    let num = |v: &Value, i: usize| v.get(i).and_then(|x| x.as_i64()).unwrap();
    // The op 9 state is 14 words with no blob or log (the adapter keeps the input): apply only the
    // state-head, record, pool and table ops.
    if st[1] == 9 {
        for op in t["ops"].as_array().unwrap() {
            if let Some(v) = op.get("ck") { st[4 + num(v, 0) as usize] = num(v, 1); }
            if let Some(v) = op.get("code") { st[2] = v.as_i64().unwrap(); }
            if let Some(v) = op.get("aux") { st[3] = v.as_i64().unwrap(); }
            if let Some(v) = op.get("rec") { secs[1][num(v, 0) as usize] = num(v, 1); }
            if let Some(v) = op.get("pool") { secs[0][num(v, 0) as usize] = num(v, 1); }
            if let Some(v) = op.get("tab") { secs[4][num(v, 0) as usize] = num(v, 1); }
        }
        secs[2] = st;
        return rebuild(prev, &secs);
    }
    let l = st[12] as usize;
    let mut head: Vec<i64> = st[..12].to_vec();
    let mut blob: Vec<i64> = st[14..14 + l].to_vec();
    let mut ents = entries(&st[14 + l..]);
    for op in t["ops"].as_array().unwrap() {
        if let Some(v) = op.get("ck") { head[4 + num(v, 0) as usize] = num(v, 1); }
        if let Some(v) = op.get("code") { head[2] = v.as_i64().unwrap(); }
        if let Some(v) = op.get("aux") { head[3] = v.as_i64().unwrap(); }
        if let Some(v) = op.get("entry_seq") { ents[num(v, 0) as usize][3 + 3] = num(v, 1); }
        if let Some(v) = op.get("entry_op") { ents[num(v, 0) as usize][3 + 5] = num(v, 1); }
        if let Some(v) = op.get("entry_rc") { ents[num(v, 0) as usize][1] = num(v, 1); }
        if let Some(v) = op.get("drop") { ents.remove(v.as_i64().unwrap() as usize); }
        if let Some(v) = op.get("dup") { let e = ents[v.as_i64().unwrap() as usize].clone(); ents.push(e); }
        if let Some(v) = op.get("swap") { ents.swap(num(v, 0) as usize, num(v, 1) as usize); }
        if let Some(v) = op.get("push") {
            let mut e = ents[num(v, 0) as usize].clone();
            e[3 + 3] = num(v, 1);
            ents.push(e);
        }
        if let Some(v) = op.get("rec") { secs[1][num(v, 0) as usize] = num(v, 1); }
        if let Some(v) = op.get("pool") { secs[0][num(v, 0) as usize] = num(v, 1); }
        if let Some(v) = op.get("tab") { secs[4][num(v, 0) as usize] = num(v, 1); }
        if let Some(v) = op.get("blob_pool") {
            let bb = sec_bounds(&blob);
            blob[bb[0].0 + num(v, 0) as usize] = num(v, 1);
        }
    }
    let lg: Vec<i64> = ents.concat();
    head[0 + 0] = head[0];
    let mut o: Vec<i64> = vec![st[0], st[1], head[2], head[3]];
    o.extend_from_slice(&head[4..12]);
    o.push(blob.len() as i64);
    o.push(lg.len() as i64);
    o.extend_from_slice(&blob);
    o.extend_from_slice(&lg);
    secs[2] = o;
    rebuild(prev, &secs)
}

impl Forge {
    pub fn from_env() -> Self {
        let tamper = std::env::var("FORGE_TAMPER").ok().map(|s| serde_json::from_str::<Value>(&s).unwrap());
        let stats = std::env::var("FORGE_STATS").is_ok();
        Self { n: Cell::new(0), tamper, stats }
    }

    pub fn index_input(&self, input: &[i64]) -> Option<Vec<i64>> {
        if self.tamper.as_ref()?.get("index_swap")?.as_bool() != Some(true) {
            return None;
        }
        let bounds = sec_bounds(input);
        let record = bounds[1].0;
        let (base, end) = bounds[input[record + 1] as usize];
        assert!(end - base >= 60);
        let mut other = input.to_vec();
        for word in 3..6 {
            other.swap(base + word, base + 30 + word);
        }
        Some(other)
    }

    pub fn index_tail(&self, input: Vec<i64>) -> Vec<i64> {
        let Some(sep) = self.tamper.as_ref().and_then(|t| t.get("index_tail")).and_then(|v| v.as_i64()) else {
            return input;
        };
        let bounds = sec_bounds(&input);
        let mut secs: Vec<Vec<i64>> = bounds.iter().map(|&(a, b)| input[a..b].to_vec()).collect();
        let pool = &secs[0];
        let nb = pool[1] as usize;
        let mut bytes: Vec<u8> = (0..nb).map(|k| ((pool[2 + k / 7] >> (8 * (6 - k % 7))) & 255) as u8).collect();
        bytes.extend_from_slice(&[0xED, 0xA0, 0x80, sep as u8, 0xED, 0xB0, 0x80]);
        let mut o = vec![pool[0] + 2, bytes.len() as i64];
        for chunk in bytes.chunks(7) {
            let mut word = 0i64;
            for (i, b) in chunk.iter().enumerate() { word |= (*b as i64) << (8 * (6 - i)); }
            o.push(word);
        }
        secs[0] = o;
        rebuild(&input, &secs)
    }

    pub fn index_stats(&self, emitted: &[i64], progress: &[i64]) {
        if self.stats {
            let bounds = sec_bounds(emitted);
            let state = &emitted[bounds[2].0..bounds[2].1];
            eprintln!("index state {} {} {} progress {}", state[4], state[12], state[13], progress[1]);
        }
    }

    pub fn prev(&self, prev: &[i64]) -> Option<Vec<i64>> {
        let n = self.n.get() + 1;
        self.n.set(n);
        if self.stats {
            let b = sec_bounds(prev);
            let st = &prev[b[2].0..b[2].1];
            let ne = if st[1] == 9 { 0 } else { entries(&st[14 + st[12] as usize..]).len() };
            eprintln!("step {} prev_len {} state_len {} entries {}", n, prev.len(), st.len(), ne);
        }
        self.tamper.as_ref().filter(|t| t["at"].as_u64().unwrap() as usize == n)
            .map(|t| tamper(prev, t))
    }
}

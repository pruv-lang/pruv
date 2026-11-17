// Hand-written idiomatic Rust baselines for the M5 host perf items `store_find` and `store_save`
// (bench/l3/budget/harness.rs, measures (c) and (d)). Same inputs, same n, same consumption of results.
//
//   m5_hand perf <n> <reps>   one JSON line per measure: {"measure":..,"n":..,"reps":..,"ns_per_op":..}
//   m5_hand sample            prints the encoded find result and a saved session (shape check against the host)
//
// Scope of each measure (setup untimed, minimum over reps after one warm-up, like harness.rs `best`):
//   store_find_text  n lookups by string id in an id index (std HashMap) and the find result encoded as the same JV
//                    frame the host builds ({"sref":"sK","v":{"obj":[["id",..],["n",..]]}}, labels assigned on first
//                    encode), written as JSON text into one reused String; the caller consumes its length.
//   store_find_tree  same lookup, the frame built as an owned value tree (String keys, like serde_json::Value) and
//                    dropped by the caller (what the host does with its J tree).
//   store_find_core  lookup only: id -> &Session (the lookup step of a non-tracing find).
//   store_find_core_u16  lookup only with JS string keys: id -> UTF-16 units in a stack buffer -> HashMap<Vec<u16>,_>.
//   store_save       n program saves. Input: the same parsed program JV as harness.rs (an owned tree, built
//                    untimed): {"sref":"s<i+1>","v":{"obj":[["id","s<i>"],["budget",{"bref":"b<i+1>","v":{"obj":
//                    [["used",i+1]]}}]]}}. Work per save: walk the whole value raising the s/b label counters,
//                    find the sref label, resolve it to the stored session, take only its budget pair, decode the
//                    budget JV ({"obj":[[k,v],..]} with Object.fromEntries duplicate-key semantics) into a JS value,
//                    set or append the session's "budget" key and record the budget label. Checked untimed after.
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

// ---- program JV input (parsed JSON) -------------------------------------------------------------------------------
enum Jv {
    Num(f64),
    Str(String),
    Arr(Vec<Jv>),
    Obj(Vec<(String, Jv)>),
}

impl Jv {
    fn get(&self, k: &str) -> Option<&Jv> {
        match self {
            Jv::Obj(es) => es.iter().rev().find(|e| e.0 == k).map(|e| &e.1),
            _ => None,
        }
    }
}

// ---- JS values held by the store ----------------------------------------------------------------------------------
#[derive(Clone)]
enum Val {
    Num(f64),
    Str(String),
    Obj(Vec<(String, Val)>),
}

struct Session {
    fields: Vec<(String, Val)>,
    label: Option<u64>,
    bud: Option<String>,
}

struct Store {
    sessions: Vec<Session>,
    idx: HashMap<String, usize>,
    by_label: HashMap<u64, usize>,
    s_seq: u64,
    b_seq: u64,
}

fn esc(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn num_text(x: f64, out: &mut String) {
    if x.fract() == 0.0 && x.abs() <= 9007199254740991.0 && !(x == 0.0 && x.is_sign_negative()) {
        let _ = write!(out, "{}", x as i64);
    } else {
        let _ = write!(out, "{{\"num\":\"{:016x}\"}}", x.to_bits());
    }
}

fn val_text(v: &Val, out: &mut String) {
    match v {
        Val::Num(x) => num_text(*x, out),
        Val::Str(s) => esc(s, out),
        Val::Obj(es) => {
            out.push_str("{\"obj\":[");
            for (i, (k, x)) in es.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                esc(k, out);
                out.push(',');
                val_text(x, out);
                out.push(']');
            }
            out.push_str("]}");
        }
    }
}

// Owned tree form of the frame (String keys, numbers as f64, like serde_json::Value).
#[allow(dead_code)] // built and dropped, never read (the measured work is building it)
enum Tree {
    Num(f64),
    Str(String),
    Arr(Vec<Tree>),
    Obj(Vec<(String, Tree)>),
}

fn val_tree(v: &Val) -> Tree {
    match v {
        Val::Num(x) => Tree::Num(*x),
        Val::Str(s) => Tree::Str(s.clone()),
        Val::Obj(es) => Tree::Obj(vec![(
            "obj".to_string(),
            Tree::Arr(es.iter().map(|(k, x)| Tree::Arr(vec![Tree::Str(k.clone()), val_tree(x)])).collect()),
        )]),
    }
}

fn label_num(s: &str, p: char) -> Option<u64> {
    s.strip_prefix(p)?.parse().ok()
}

fn index_key(k: &str) -> bool {
    !k.is_empty() && k.len() <= 10 && !(k.len() > 1 && k.starts_with('0')) && k.bytes().all(|b| b.is_ascii_digit())
        && k.parse::<u64>().map_or(false, |n| n <= 4294967294)
}

// JV decode of a budget value: numbers, strings and {"obj":[[k,v],..]} (duplicate key keeps first position, index keys first).
fn decode(j: &Jv) -> Result<Val, String> {
    match j {
        Jv::Num(x) => Ok(Val::Num(*x)),
        Jv::Str(s) => Ok(Val::Str(s.clone())),
        Jv::Obj(es) if es.len() == 1 && es[0].0 == "obj" => {
            let pairs = match &es[0].1 {
                Jv::Arr(a) => a,
                _ => return Err("bad {obj}".into()),
            };
            let mut out: Vec<(String, Val)> = Vec::with_capacity(pairs.len());
            for p in pairs {
                let (k, v) = match p {
                    Jv::Arr(kv) if kv.len() == 2 => match &kv[0] {
                        Jv::Str(k) => (k, decode(&kv[1])?),
                        _ => return Err("non-string key".into()),
                    },
                    _ => return Err("bad {obj} pair".into()),
                };
                match out.iter().position(|e| e.0 == *k) {
                    Some(i) => out[i].1 = v,
                    None => out.push((k.clone(), v)),
                }
            }
            if out.iter().any(|e| index_key(&e.0)) {
                out.sort_by_key(|e| if index_key(&e.0) { (0, e.0.parse::<u64>().unwrap_or(0)) } else { (1, 0) });
            }
            Ok(Val::Obj(out))
        }
        _ => Err("unsupported JV".into()),
    }
}

impl Store {
    fn new() -> Store {
        Store { sessions: Vec::new(), idx: HashMap::new(), by_label: HashMap::new(), s_seq: 0, b_seq: 0 }
    }

    fn put(&mut self, id: &str, n: f64) {
        let s = Session { fields: vec![("id".to_string(), Val::Str(id.to_string())), ("n".to_string(), Val::Num(n))], label: None, bud: None };
        match self.idx.get(id) {
            Some(&i) => self.sessions[i] = s,
            None => {
                self.sessions.push(s);
                self.idx.insert(id.to_string(), self.sessions.len() - 1);
            }
        }
    }

    fn label_of(&mut self, h: usize) -> u64 {
        match self.sessions[h].label {
            Some(l) => l,
            None => {
                self.s_seq += 1;
                self.sessions[h].label = Some(self.s_seq);
                self.by_label.insert(self.s_seq, h);
                self.s_seq
            }
        }
    }

    fn find_text(&mut self, id: &str, out: &mut String) {
        out.clear();
        let h = match self.idx.get(id) {
            Some(&h) => h,
            None => {
                out.push_str("{\"undef\":1}");
                return;
            }
        };
        let l = self.label_of(h);
        let _ = write!(out, "{{\"sref\":\"s{}\",\"v\":", l);
        out.push_str("{\"obj\":[");
        for (i, (k, x)) in self.sessions[h].fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            esc(k, out);
            out.push(',');
            val_text(x, out);
            out.push(']');
        }
        out.push_str("]}}");
    }

    fn find_tree(&mut self, id: &str) -> Tree {
        let h = match self.idx.get(id) {
            Some(&h) => h,
            None => return Tree::Obj(vec![("undef".to_string(), Tree::Num(1.0))]),
        };
        self.enc_tree(h)
    }

    // The frame of a found session as an owned tree (labels on first encode).
    fn enc_tree(&mut self, h: usize) -> Tree {
        let l = self.label_of(h);
        let pairs: Vec<Tree> = self.sessions[h].fields.iter().map(|(k, x)| Tree::Arr(vec![Tree::Str(k.clone()), val_tree(x)])).collect();
        Tree::Obj(vec![
            ("sref".to_string(), Tree::Str(format!("s{}", l))),
            ("v".to_string(), Tree::Obj(vec![("obj".to_string(), Tree::Arr(pairs))])),
        ])
    }

    fn note_labels(&mut self, j: &Jv) {
        match j {
            Jv::Arr(a) => a.iter().for_each(|x| self.note_labels(x)),
            Jv::Obj(es) => {
                for (k, v) in es {
                    if let Jv::Str(s) = v {
                        if k == "sref" {
                            if let Some(n) = label_num(s, 's') {
                                self.s_seq = self.s_seq.max(n);
                            }
                        } else if k == "bref" {
                            if let Some(n) = label_num(s, 'b') {
                                self.b_seq = self.b_seq.max(n);
                            }
                        }
                    }
                    self.note_labels(v);
                }
            }
            _ => {}
        }
    }

    // One program save: {"sref":"sK","v":{"obj":[..,["budget",{"bref":"bJ","v":W}],..]}} on a known session.
    fn save(&mut self, j: &Jv) -> Result<(), String> {
        self.note_labels(j);
        let lab = match j.get("sref") {
            Some(Jv::Str(s)) => s,
            _ => return Err("no sref".into()),
        };
        let h = *label_num(lab, 's').and_then(|n| self.by_label.get(&n)).ok_or("unknown session")?;
        let pairs = match j.get("v").and_then(|v| v.get("obj")) {
            Some(Jv::Arr(p)) => p,
            _ => return Err("bad v".into()),
        };
        let bj = pairs.iter().find_map(|p| match p {
            Jv::Arr(kv) if kv.len() == 2 && matches!(&kv[0], Jv::Str(k) if k == "budget") => Some(&kv[1]),
            _ => None,
        });
        let sess = &mut self.sessions[h];
        let pos = sess.fields.iter().position(|e| e.0 == "budget");
        match bj {
            None => {
                if let Some(p) = pos {
                    sess.fields.remove(p);
                }
                sess.bud = None;
            }
            Some(bj) => {
                let (bud, val) = match (bj.get("bref"), bj.get("v")) {
                    (Some(Jv::Str(l)), Some(w)) => (Some(l.clone()), decode(w)?),
                    _ => (None, decode(bj)?),
                };
                match pos {
                    Some(p) => sess.fields[p].1 = val,
                    None => sess.fields.push(("budget".to_string(), val)),
                }
                sess.bud = bud;
            }
        }
        Ok(())
    }
}

fn best<F: FnMut() -> u128>(reps: usize, mut f: F) -> u128 {
    f();
    let mut b = u128::MAX;
    for _ in 0..reps.max(1) {
        b = b.min(f());
    }
    b
}

fn line(measure: &str, n: usize, reps: usize, ns: u128) {
    println!("{{\"measure\":\"{}\",\"n\":{},\"reps\":{},\"ns_per_op\":{:.1}}}", measure, n, reps, ns as f64 / n.max(1) as f64);
}

fn filled(n: usize, v: impl Fn(usize) -> f64) -> Store {
    let mut s = Store::new();
    for i in 0..n {
        s.put(&format!("s{}", i), v(i));
    }
    s
}

fn save_input(i: usize) -> Jv {
    let budget = Jv::Obj(vec![
        ("bref".into(), Jv::Str(format!("b{}", i + 1))),
        ("v".into(), Jv::Obj(vec![("obj".into(), Jv::Arr(vec![Jv::Arr(vec![Jv::Str("used".into()), Jv::Num((i + 1) as f64)])]))])),
    ]);
    let v = Jv::Obj(vec![(
        "obj".into(),
        Jv::Arr(vec![Jv::Arr(vec![Jv::Str("id".into()), Jv::Str(format!("s{}", i))]), Jv::Arr(vec![Jv::Str("budget".into()), budget])]),
    )]);
    Jv::Obj(vec![("sref".into(), Jv::Str(format!("s{}", i + 1))), ("v".into(), v)])
}

fn perf(n: usize, reps: usize) {
    let ids: Vec<String> = (0..n).map(|i| format!("s{}", (i * 7919) % n.max(1))).collect();

    let ns = best(reps, || {
        let mut s = filled(n, |i| i as f64);
        let mut out = String::with_capacity(128);
        let t = Instant::now();
        for id in &ids {
            s.find_text(black_box(id), &mut out);
            black_box(out.len());
        }
        t.elapsed().as_nanos()
    });
    line("store_find_text", n, reps, ns);

    let ns = best(reps, || {
        let mut s = filled(n, |i| i as f64);
        let t = Instant::now();
        for id in &ids {
            black_box(s.find_tree(black_box(id)));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_tree", n, reps, ns);

    // Encode only, like harness store_find_enc: labels assigned and handles found untimed.
    let ns = best(reps, || {
        let mut s = filled(n, |i| i as f64);
        let hs: Vec<usize> = ids.iter().map(|id| s.idx[id.as_str()]).collect();
        for &h in &hs {
            s.label_of(h);
        }
        let t = Instant::now();
        for &h in &hs {
            black_box(s.enc_tree(black_box(h)));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_enc_tree", n, reps, ns);

    let ns = best(reps, || {
        let s = filled(n, |i| i as f64);
        let t = Instant::now();
        for id in &ids {
            black_box(s.idx.get(black_box(id.as_str())).map(|&h| &s.sessions[h]));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_core", n, reps, ns);

    // Lookup only with JS string keys: the id as UTF-16 units (stack buffer) probing a HashMap<Vec<u16>, usize>.
    let ns = best(reps, || {
        let s = filled(n, |i| i as f64);
        let idx16: HashMap<Vec<u16>, usize> = s.idx.iter().map(|(k, &v)| (k.encode_utf16().collect(), v)).collect();
        let t = Instant::now();
        for id in &ids {
            let id = black_box(id.as_str());
            let mut buf = [0u16; 64];
            let mut k = 0;
            for c in id.encode_utf16() {
                buf[k] = c;
                k += 1;
            }
            black_box(idx16.get(&buf[..k]).map(|&h| &s.sessions[h]));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_core_u16", n, reps, ns);

    let ns = best(reps, || {
        let mut s = filled(n, |_| 0.0);
        for h in 0..n {
            s.label_of(h); // labels s1..sn, like the host's untimed encode_all_sessions
        }
        let raws: Vec<Jv> = (0..n).map(save_input).collect();
        let t = Instant::now();
        for r in &raws {
            s.save(black_box(r)).expect("save");
        }
        let e = t.elapsed().as_nanos();
        let saved = s.sessions.iter().filter(|x| x.fields.iter().any(|f| f.0 == "budget") && x.bud.is_some()).count();
        assert_eq!(saved, n, "store_save: not every budget was written");
        e
    });
    line("store_save", n, reps, ns);
}

fn sample() {
    // Labels in id order (like the host's encode_all_sessions), then find s5 and save the program value for s5.
    let mut s = filled(10, |i| i as f64);
    for h in 0..10 {
        s.label_of(h);
    }
    let mut out = String::new();
    s.find_text("s5", &mut out);
    println!("find {}", out);
    s.save(&save_input(5)).expect("save");
    let h = s.idx["s5"];
    let mut t = String::new();
    val_text(&Val::Obj(s.sessions[h].fields.clone()), &mut t);
    println!("saved s5 bud={:?} {}", s.sessions[h].bud, t);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("perf") => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let reps: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            if n == 0 || reps == 0 {
                eprintln!("usage: m5_hand perf <n> <reps>");
                std::process::exit(2);
            }
            perf(n, reps);
        }
        Some("sample") => sample(),
        _ => {
            eprintln!("usage: m5_hand perf <n> <reps> | sample");
            std::process::exit(2);
        }
    }
}

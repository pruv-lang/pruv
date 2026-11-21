// L3-9 leaf wrapper (included inside `mod eh` after the L3-8 entry host, bench/l3/entry/host.rs, which stays unchanged).
// The request host asks this wrapper to serve the child entry await of a request (the scripted leaf calls), to encode the
// request input record, to render a done packet (identity refs by name) and to render a scenario input with its key order.
// The wrapper makes no handler decision: it serves the script exactly as the L3-8 host does and reports the trace.
// Only plain `//` comments here (include! forbids inner doc comments and inner attributes).

fn he(e: HostErr) -> String {
    match e {
        HostErr::Case(s) => format!("case: {}", s),
        HostErr::Core(s) => format!("internal: {}", s),
    }
}

// A trace event the leaf produces for the request host (the host adds the request and instance names).
#[derive(Default)]
pub struct Ev {
    pub c: String,
    pub a: String,
    pub o: Option<String>,
    // who the line belongs to: 0 the current request, -1 none (r and i null), k > 0 request k - 1; pid: a patchable line id (0 none)
    pub who: i64,
    pub pid: i64,
}

struct ListSt {
    source: Vec<u16>,
    folder: Vec<u16>,
    codex: Option<String>,
    timer: i64,
    call: i64,
    snap: Vec<Value>,
    script: Option<(Value, Option<i64>)>,
    closed: bool,
}

// What the host knows about a running request (set by the request host at start).
#[derive(Clone)]
struct ReqInfo {
    name: String,
    inst: String,
    inst_id: i64,
    from_app: Option<Value>,
    cancel: Option<String>,
}

// The open handler's per-request state: the held key, its timer and listener, the options of the read.
struct OpenSt {
    win: Option<usize>,
    engine: String,
    source: Vec<u16>,
    codex: Option<String>,
    opencode: Option<String>,
    // the Grok home facts the grokHomeFor dependency answered (home units, binary and env rendered) and the copy path
    ghome: Option<(Vec<u16>, String, String)>,
    copied: Option<Vec<u16>>,
}

struct Handle {
    name: String,
    aborted: bool,
    active: i64,
}

// A reply of a host call: (completion pool, completion record, completion section 5). A child await answers
// (empty, [0], completion packet), so one shape covers both.
pub type Reply = (Vec<i64>, Vec<i64>, Vec<i64>);

// The rest of a deferred host call: runs when the call settles (via_abort tells that the abort of its signal settled it).
type Cont = Box<dyn FnOnce(&mut Leaf, bool) -> Result<Step, String>>;

pub enum Step {
    Reply(Reply),
    Hold(Hold),
}

// A scripted dependency call that stays pending until the scenario settles it (world entry "defer" label). The request
// that made the call waits; its reply is `pre` (or what `cont` computes when the call settles). A hold bound to a window
// with `abort_pre`/`cont` is also settled by the abort of that window's signal (the entry's onAbort).
pub struct Hold {
    label: String,
    call: String,
    owner: i64,
    rej: bool,
    win: Option<usize>,
    pid: i64,
    norm_o: Option<String>,
    abort_o: Option<String>,
    pre: Reply,
    abort_pre: Option<Reply>,
    abort_comp: Option<Vec<i64>>,
    cont: Option<Cont>,
}

// What a copy needs to continue after it settles.
struct CopyCtx {
    pid: i64,
    entry: Oj,
    home: Vec<u16>,
    bin: String,
    env: String,
    folder: Vec<u16>,
    thread: Vec<u16>,
}

// A timed() window: the cancel listener, the 20000 ms timer and the signal combining them.
struct Win {
    owner: i64,
    tid: i64,
    sig: i64,
    aborted: bool,
    lis: Option<(String, i64)>,
    lis_active: bool,
    handle: Option<String>,
    closed: bool,
    hold: Option<String>,
}

pub struct Leaf {
    h: Host,
    world: Value,
    raw: Oj,
    used: HashMap<String, usize>,
    events: Vec<Ev>,
    timer_seq: i64,
    sig_seq: i64,
    sl_calls: i64,
    row_seq: i64,
    lists: HashMap<i64, ListSt>,
    pub cur: i64,
    sess: Vec<Value>,
    infos: HashMap<i64, ReqInfo>,
    opens: HashMap<i64, OpenSt>,
    exts: HashMap<i64, Oj>,
    oj_refs: HashMap<i64, Oj>,
    pub ww: Vec<i64>,
    opening_events: Vec<String>,
    created: Vec<String>,
    handles: Vec<Handle>,
    listener_seq: i64,
    active_timers: Vec<(i64, i64)>,
    created_seq: i64,
    uuid_seq: usize,
    label_count: HashMap<String, i64>,
    ovr: HashMap<String, Oj>,
    fired: Vec<i64>,
    pending_hold: Option<Hold>,
    holds: Vec<Hold>,
    wins: Vec<Win>,
    pub patches: HashMap<i64, String>,
    reasons: HashMap<String, Option<String>>,
    pid_seq: i64,
    pub now: i64,
    wrap_seq: i64,
    ext_seq: i64,
}

fn oj_get<'a>(o: &'a Oj, k: &str) -> Option<&'a Oj> {
    match o {
        Oj::Obj(p) => p.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
        _ => None,
    }
}

fn oj_nth(o: &Oj, n: usize) -> Option<&Oj> {
    match o {
        Oj::Arr(a) => {
            if a.is_empty() {
                None
            } else {
                Some(&a[n.min(a.len() - 1)])
            }
        }
        other => Some(other),
    }
}

fn units_at(pool: &[i64], off: i64, len: i64) -> Result<Vec<u16>, String> {
    if off < 0 || len < 0 || (off + len) as usize > pool.len() {
        return Err("internal: string slot outside the pool".to_string());
    }
    Ok(pool[off as usize..(off + len) as usize].iter().map(|&x| x as u16).collect())
}

impl Leaf {
    // `world` is the scenario world object (accounts, ambient, homedir, realpath, stat, ... as in SCENARIO.md 2).
    pub fn new(world: &Value, raw: &str) -> Result<Leaf, String> {
        let mut sw = world.clone();
        if let Some(m) = sw.as_object_mut() {
            m.remove("isTemporary");
        }
        let script = parse_script(Some(&sw), None).map_err(he)?;
        let rawj = if raw.trim().is_empty() { Oj::Obj(Vec::new()) } else { (OjP { b: raw.as_bytes(), at: 0 }).val()? };
        let sess0: Vec<Value> = match oj_get(&rawj, "sessions") {
            Some(Oj::Arr(a)) => a.iter().map(session_value).collect(),
            _ => Vec::new(),
        };
        Ok(Leaf {
            h: Host {
                refs: Refs::new(),
                script,
                trace: Vec::new(),
                acct: [Vec::new(), Vec::new()],
                rows: Vec::new(),
                seen: HashSet::new(),
                errno_n: 0,
            },
            world: world.clone(),
            raw: rawj,
            used: HashMap::new(),
            events: Vec::new(),
            timer_seq: 0,
            sig_seq: 0,
            sl_calls: 0,
            row_seq: 0,
            lists: HashMap::new(),
            cur: 0,
            sess: sess0,
            infos: HashMap::new(),
            opens: HashMap::new(),
            exts: HashMap::new(),
            oj_refs: HashMap::new(),
            ww: Vec::new(),
            opening_events: Vec::new(),
            created: Vec::new(),
            handles: Vec::new(),
            listener_seq: 0,
            active_timers: Vec::new(),
            created_seq: 0,
            uuid_seq: 0,
            label_count: HashMap::new(),
            ovr: HashMap::new(),
            fired: Vec::new(),
            pending_hold: None,
            holds: Vec::new(),
            wins: Vec::new(),
            patches: HashMap::new(),
            reasons: HashMap::new(),
            pid_seq: 0,
            now: 0,
            ext_seq: 0,
            wrap_seq: 0,
        })
    }

    // The request input: pool and record. Op 1 record = [dataDir, env x7] (24 words); ops 2/3 append the source and the
    // folder slots (30 words); further op 3 input slots are added by the later milestones.
    pub fn encode_input(&mut self, op: i64, world: &Value, input: Option<&Value>) -> Result<(Vec<i64>, Vec<i64>), String> {
        let wm = world.as_object();
        let mut args = serde_json::Map::new();
        args.insert(
            "dataDir".to_string(),
            wm.and_then(|m| m.get("dataDir")).cloned().unwrap_or(Value::String("/data".to_string())),
        );
        args.insert(
            "env".to_string(),
            wm.and_then(|m| m.get("env")).cloned().unwrap_or(Value::Object(serde_json::Map::new())),
        );
        let mut enc = encode_input(&mut self.h.refs, 1, &Value::Object(args)).map_err(he)?;
        if op != 1 {
            let im = input.and_then(|v| v.as_object());
            let keys: &[&str] = if op == 3 { &["source", "folder", "thread", "mode", "targetAccount"] } else { &["source", "folder"] };
            for k in keys.iter().copied() {
                let j = jv_opt(im.and_then(|m| m.get(k))).map_err(he)?;
                let vs = enc.jv_vs(&mut self.h.refs, &j);
                put(&mut enc.rec, vs);
            }
        }
        Ok((enc.units(), enc.rec))
    }

    // Serves one child entry await (an L3-8 entry packet, kind 1) and returns the completion entry packet (kind 2).
    pub fn serve_child(&mut self, child: &[i64]) -> Result<Vec<i64>, String> {
        let p = parse_pkt_units(child).map_err(he)?;
        if p.kind != 1 {
            return Err("internal: child packet is not an await".to_string());
        }
        let code = p.word(1, 0).map_err(he)?;
        if code == 9 {
            let enc = self.serve_reader(&p)?;
            return Ok(mk_packet_units(2, p.op_id, p.seq, p.op, &enc));
        }
        if code == 8 {
            // isTemporary: world.isTemporary[id] (default false); the entry host's prim line becomes an oracle event
            let v = p.val(1, 1).map_err(he)?;
            let (sec, count) = p.table(&v, 3).map_err(he)?;
            let mut vals: Vec<FsRes<Jv>> = Vec::new();
            let mut raws: Vec<String> = Vec::new();
            for i in 0..count {
                let key = match p.val(sec, i * 3).map_err(he)? {
                    V::Str(u) => Some(String::from_utf16_lossy(&u)),
                    _ => None,
                };
                let raw = key
                    .and_then(|k| self.world.get("isTemporary").and_then(|t| t.as_object()).and_then(|m| m.get(&k)).cloned())
                    .unwrap_or(Value::Bool(false));
                let j = parse_jv(&raw).map_err(he)?;
                raws.push(render_jv(&j));
                vals.push(FsRes::Ok(j));
            }
            let saved = std::mem::replace(&mut self.h.script.is_temp, Queue::new(vals));
            // Only the lines this call adds are scanned: re-walking the whole trace per call was O(S^2) (spec 13).
            let base = self.h.trace.len();
            let r = self.h.serve(&p);
            self.h.script.is_temp = saved;
            let enc = r.map_err(he)?;
            let lines = self.h.trace.split_off(base.min(self.h.trace.len()));
            let mut k = 0usize;
            for l in lines {
                if l.starts_with("[\"isTemporary\",") {
                    let o = raws.get(k).cloned().unwrap_or_else(|| "false".to_string());
                    k += 1;
                    self.events.push(Ev { c: "isTemporary".to_string(), a: format!("[{}", &l["[\"isTemporary\",".len()..]), o: Some(format!("{{\"ok\":{}}}", o)), ..Default::default() });
                } else {
                    self.h.trace.push(l);
                }
            }
            return Ok(mk_packet_units(2, p.op_id, p.seq, p.op, &enc));
        }
        let enc = self.h.serve(&p).map_err(he)?;
        Ok(mk_packet_units(2, p.op_id, p.seq, p.op, &enc))
    }

    pub fn take_events(&mut self) -> Vec<Ev> {
        std::mem::take(&mut self.events)
    }

    fn sessions(&self) -> Vec<Value> {
        match self.world.get("sessions") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        }
    }

    fn sess_id(&self, s: &Value) -> Result<String, String> {
        Ok(render_jv(&jv_opt(s.get("id")).map_err(he)?))
    }

    // The n-th entry (last repeats) of the raw world list at path, counted per path.
    fn nth_ok(&mut self, path: &[&str]) -> Result<Option<(String, Value)>, String> {
        let key = format!("world:{}", path.join("."));
        let n = {
            let e = self.used.entry(key).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        let mut node: &Oj = &self.raw;
        for k in path {
            node = match oj_get(node, k) {
                Some(x) => x,
                None => return Ok(None),
            };
        }
        let entry = match oj_nth(node, n) {
            Some(e) => e,
            None => return Ok(None),
        };
        if oj_get(entry, "ok").is_none() {
            return Err("case: list script entries other than ok are not supported yet".to_string());
        }
        let ok = oj_get(entry, "ok").unwrap();
        Ok(Some((render_oj(ok)?, oj_value(ok))))
    }

    // invoke_engine (code 9, list): the reader call. Logs the reader event and answers the rows [rowRef, title].
    fn serve_reader(&mut self, p: &Pkt) -> Result<Enc, String> {
        let route = p.word(1, 1).map_err(he)?;
        let opk = p.word(1, 2).map_err(he)?;
        if !(0..=4).contains(&route) || !(0..=1).contains(&opk) {
            return Err("internal: reader call with an unsupported route or op".to_string());
        }
        if opk == 1 {
            return self.serve_read(p, route);
        }
        let folder = render_v(&self.h.refs, &p.val(1, 6).map_err(he)?).map_err(he)?;
        let engine = ["claude", "codex", "opencode", "qwen", "grok"][route as usize];
        let (source, codex) = match self.lists.get(&self.cur) {
            Some(st) => (jstr(&st.source), st.codex.clone()),
            None => return Err("internal: reader call without list state".to_string()),
        };
        let tid = self.lists.get(&self.cur).map(|st| st.timer).ok_or("internal: reader call without list state")?;
        let win = self.wins.iter().position(|w| w.tid == tid).ok_or("internal: list reader without a window")?;
        let sg = self.win_sig(win);
        let mut keys = "\"signal\",\"env\"".to_string();
        let mut extra = String::new();
        if route == 1 {
            keys.push_str(",\"codex\"");
            extra = format!(",\"codex\":{}", codex.unwrap_or_else(|| "{\"ref\":\"codexRun\"}".to_string()));
        }
        let path = format!("readers.{}.list", engine);
        let n = {
            let e = self.used.entry(format!("world:{}", path)).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        let entry: Oj = match self.lookup(&path).and_then(|node| oj_nth(node, n)) {
            Some(e) => self.ov(e.clone()),
            None => return Err(format!("case: no reader script for {}.list", engine)),
        };
        let defer = Self::entry_defer(&entry)?;
        let a = format!(
            "{{\"engine\":{},\"op\":\"list\",\"source\":{},\"folder\":{},\"options\":{{\"keys\":[{}],\"signal\":\"sig{}\",\"aborted\":false,\"env\":\"deps.env\"{}}}}}",
            jstr_s(engine),
            source,
            folder,
            keys,
            sg,
            extra
        );
        let mut enc = Enc::new();
        let o;
        let is_reject;
        if let Some(r) = oj_get(&entry, "reject") {
            let sp = parse_err_l(&oj_value(r)).map_err(he)?;
            self.h.throw_rec(&mut enc, &sp).map_err(he)?;
            o = format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref));
            is_reject = true;
        } else if let Some(okv) = oj_get(&entry, "ok") {
            let items = oj_value(okv);
            o = format!("{{\"ok\":{}}}", render_oj(okv)?);
            is_reject = false;
            let arr = match items {
                Value::Array(a) => a,
                _ => return Err("case: reader list ok must be an array".to_string()),
            };
            let sec = enc.reserve();
            let mut t = Vec::new();
            for it in arr.iter() {
                self.row_seq += 1;
                let name = format!("$row{}", self.row_seq);
                let id = self.h.refs.id(&name);
                self.h.refs.set(id, RefVal::Row(it.clone()));
                self.h.seen.insert(id);
                put(&mut t, [9, id, 0]);
                match jv_opt(it.get("title")).map_err(he)? {
                    Jv::Str(s) => {
                        let vs = enc.str_vs(&s);
                        put(&mut t, vs);
                    }
                    _ => return Err("case: reader row title must be a string".to_string()),
                }
            }
            enc.fill(sec, t);
            enc.rec.push(0);
            put(&mut enc.rec, [7, sec as i64, arr.len() as i64]);
        } else {
            return Err("case: reader list entries need ok or reject".to_string());
        }
        match defer {
            None => self.events.push(Ev { c: "reader".to_string(), a, o: Some(o), ..Default::default() }),
            Some(l) => {
                let pid = self.emit_p("reader", a);
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, &format!("reader:{}.list", engine), pid);
                h.rej = is_reject;
                h.norm_o = Some(o);
                h.win = Some(win);
                if let Some(ab) = oj_get(&entry, "onAbort") {
                    let sp = parse_err_l(&oj_value(ab)).map_err(he)?;
                    let mut e2 = Enc::new();
                    self.h.throw_rec(&mut e2, &sp).map_err(he)?;
                    h.abort_o = Some(format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref)));
                    h.abort_comp = Some(mk_packet_units(2, p.op_id, p.seq, p.op, &e2));
                }
                self.pending_hold = Some(h);
            }
        }
        Ok(enc)
    }


    fn lookup(&self, path: &str) -> Option<&Oj> {
        let mut node: &Oj = &self.raw;
        for k in path.split('.') {
            node = oj_get(node, k)?;
        }
        Some(node)
    }

    // invoke_engine (code 9, open): the reader read call. ok registers the read result as a record ref.
    fn serve_read(&mut self, p: &Pkt, route: i64) -> Result<Enc, String> {
        let folder = render_v(&self.h.refs, &p.val(1, 6).map_err(he)?).map_err(he)?;
        let thread = render_v(&self.h.refs, &p.val(1, 9).map_err(he)?).map_err(he)?;
        let engine = ["claude", "codex", "opencode", "qwen", "grok"][route as usize];
        let (source, codex, opencode) = match self.opens.get(&self.cur) {
            Some(st) => (jstr(&st.source), st.codex.clone(), st.opencode.clone()),
            None => return Err("internal: reader read without open state".to_string()),
        };
        let win = self.opens.get(&self.cur).and_then(|st| st.win).ok_or("internal: reader read without a window")?;
        let sg = self.win_sig(win);
        let wab = self.wins[win].aborted;
        let mut keys = "\"signal\",\"env\"".to_string();
        let mut extra = String::new();
        if let Some(c) = &codex {
            keys.push_str(",\"codex\"");
            extra.push_str(&format!(",\"codex\":{}", c));
        }
        if let Some(o) = &opencode {
            keys.push_str(",\"opencode\"");
            extra.push_str(&format!(",\"opencode\":{}", o));
        }
        let path = format!("readers.{}.read", engine);
        let key = format!("world:{}", path);
        let n = {
            let e = self.used.entry(key).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        let entry: Oj = match self.lookup(&path).and_then(|node| oj_nth(node, n)) {
            Some(e) => self.ov(e.clone()),
            None => return Err(format!("case: no reader script for {}.read", engine)),
        };
        let defer = match oj_get(&entry, "defer") {
            Some(d) => Some(oj_units(d).map(|u| String::from_utf16_lossy(&u)).ok_or("case: defer label must be a string")?),
            None => None,
        };
        let a = format!(
            "{{\"engine\":{},\"op\":\"read\",\"source\":{},\"folder\":{},\"thread\":{},\"options\":{{\"keys\":[{}],\"signal\":\"sig{}\",\"aborted\":{},\"env\":\"deps.env\"{}}}}}",
            jstr_s(engine),
            source,
            folder,
            thread,
            keys,
            sg,
            wab,
            extra
        );
        let mut enc = Enc::new();
        let o;
        let is_reject;
        if let Some(r) = oj_get(&entry, "reject") {
            let sp = parse_err_l(&oj_value(r)).map_err(he)?;
            self.h.throw_rec(&mut enc, &sp).map_err(he)?;
            o = format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref));
            is_reject = true;
        } else if let Some(ok) = oj_get(&entry, "ok") {
            self.ext_seq += 1;
            let name = format!("$ext{}", self.ext_seq);
            let id = self.h.refs.id(&name);
            self.h.refs.set(id, RefVal::Record);
            self.h.seen.insert(id);
            self.exts.insert(id, ok.clone());
            enc.rec.push(0);
            put(&mut enc.rec, [9, id, 0]);
            o = format!("{{\"ok\":{}}}", render_oj(ok)?);
            is_reject = false;
        } else {
            return Err("case: reader read entries need ok or reject".to_string());
        }
        match defer {
            None => self.events.push(Ev { c: "reader".to_string(), a, o: Some(o), ..Default::default() }),
            Some(l) => {
                let pid = self.emit_p("reader", a);
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, &format!("reader:{}.read", engine), pid);
                h.rej = is_reject;
                h.norm_o = Some(o);
                h.win = Some(win);
                if let Some(ab) = oj_get(&entry, "onAbort") {
                    let sp = parse_err_l(&oj_value(ab)).map_err(he)?;
                    let mut e2 = Enc::new();
                    self.h.throw_rec(&mut e2, &sp).map_err(he)?;
                    h.abort_o = Some(format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref)));
                    h.abort_comp = Some(mk_packet_units(2, p.op_id, p.seq, p.op, &e2));
                }
                self.pending_hold = Some(h);
            }
        }
        Ok(enc)
    }

    // The host calls of the list and open handlers (ck 3). `pool`, `rec`, `wk`, `rows` are sections 0, 1, 4, 3 of the await
    // packet and `call` is its section 5. Returns (completion pool, completion record, completion section 5).
    pub fn serve_call(&mut self, pool: &[i64], rec: &[i64], wk: &[i64], rows: &[i64], call: &[i64], now: i64) -> Result<(Vec<i64>, Vec<i64>, Vec<i64>), String> {
        let code = *call.first().ok_or("internal: empty host call")?;
        let arg = |k: usize| -> Result<i64, String> { call.get(k).copied().ok_or("internal: host call argument missing".to_string()) };
        let cur = self.cur;
        let mut enc = Enc::new();
        let mut s5: Vec<i64> = Vec::new();
        match code {
            11 => {
                let isc = arg(1)? == 1;
                if rec.len() < 27 || rec[24] != 6 || wk.len() < 18 || wk[15] != 6 {
                    return Err("internal: list call without source and folder strings".to_string());
                }
                let source = units_at(pool, rec[25], rec[26])?;
                let folder = units_at(pool, wk[16], wk[17])?;
                self.timer_seq += 1;
                let tid = self.timer_seq;
                self.active_timers.push((tid, now + 20000));
                self.events.push(Ev { c: "timer.set".to_string(), a: format!("{{\"id\":{},\"delay\":20000,\"due\":{}}}", tid, now + 20000), o: None, ..Default::default() });
                self.wins.push(Win { owner: cur, tid, sig: 0, aborted: false, lis: None, lis_active: false, handle: None, closed: false, hold: None });
                let mut codex = None;
                if isc {
                    let o = match self.nth_ok(&["makeCodexRunDeps"])? {
                        Some((r, _)) => r,
                        None => "{\"ref\":\"codexRun\"}".to_string(),
                    };
                    let dd = match self.world.get("dataDir") {
                        Some(Value::String(d)) => d.clone(),
                        _ => "/data".to_string(),
                    };
                    self.events.push(Ev {
                        c: "makeCodexRunDeps".to_string(),
                        a: format!("{{\"source\":{},\"dataDir\":{}}}", jstr(&source), jstr_s(&dd)),
                        o: Some(format!("{{\"ok\":{}}}", o)),
                        ..Default::default()
                    });
                    codex = Some(o);
                }
                self.lists.insert(cur, ListSt { source, folder, codex, timer: tid, call: 0, snap: Vec::new(), script: None, closed: false });
                let s = self.h.refs.id(&format!("$source{}", cur));
                let o = self.h.refs.id(&format!("$options{}", cur));
                enc.rec.push(0);
                put(&mut enc.rec, [9, s, 0]);
                put(&mut enc.rec, [9, o, 0]);
            }
            12 => {
                let m = arg(1)? as usize;
                let tid = self.lists.get(&cur).map(|s| s.timer).ok_or("internal: read without list state")?;
                let live = self.active_timers.iter().any(|t| t.0 == tid);
                self.active_timers.retain(|t| t.0 != tid);
                if let Some(st) = self.lists.get_mut(&cur) {
                    st.closed = true;
                }
                self.events.push(Ev { c: "timer.clear".to_string(), a: format!("{{\"id\":{},\"active\":{}}}", tid, live), o: None, ..Default::default() });
                let sess = self.sess.clone();
                self.sl_calls += 1;
                let c = self.sl_calls;
                let script = self.sl_script(c);
                if let Some((t, None)) = &script {
                    let (r, o) = self.thrown_reply(t)?;
                    self.events.push(Ev { c: "sessionList".to_string(), a: format!("{{\"call\":{}}}", c), o: Some(o), ..Default::default() });
                    return Ok((Vec::new(), r, Vec::new()));
                }
                self.events.push(Ev { c: "sessionList".to_string(), a: format!("{{\"call\":{},\"size\":{}}}", c, sess.len()), o: None, ..Default::default() });
                let tat = script.as_ref().and_then(|x| x.1);
                let mut rows_enc = Enc::new();
                let mut body: Vec<i64> = Vec::new();
                body.push(0);
                body.push(sess.len() as i64);
                for i in 0..=sess.len() {
                    if tat == Some(i as i64) {
                        let (r, o) = self.thrown_reply(&script.as_ref().unwrap().0)?;
                        self.events.push(Ev { c: "sessionList.next".to_string(), a: format!("{{\"call\":{},\"index\":{}}}", c, i), o: Some(o), ..Default::default() });
                        return Ok((Vec::new(), r, Vec::new()));
                    }
                    if i == sess.len() {
                        break;
                    }
                    let s = &sess[i];
                    let id = self.sess_id(s)?;
                    self.events.push(Ev { c: "sessionList.next".to_string(), a: format!("{{\"call\":{},\"index\":{},\"id\":{}}}", c, i, id), o: None, ..Default::default() });
                }
                // The titles come back with the items (ABI.md 8.2): the core keeps them in a storage block it digests, so
                // the bound searches carry no titles and the finish reads exactly the snapshot observed here.
                let ns = sess.len();
                enc = rows_enc;
                enc.rec = body;
                self.events.push(Ev { c: "sessionList.done".to_string(), a: format!("{{\"call\":{},\"consumed\":{}}}", c, ns), o: None, ..Default::default() });
                for k in 0..m {
                    let id = arg(2 + k)?;
                    let row = match self.h.refs.vals.get((id - 1) as usize) {
                        Some(RefVal::Row(v)) => v.clone(),
                        _ => return Err("internal: read row reference is not a reader row".to_string()),
                    };
                    for key in ["title", "thread", "updatedAt", "size", "messages", "path", "maybeOpen"] {
                        let j = jv_opt(row.get(key)).map_err(he)?;
                        let vs = enc.jv_vs(&mut self.h.refs, &j);
                        put(&mut enc.rec, vs);
                    }
                }
                for s in &sess {
                    for k in ["id", "title"] {
                        let j = jv_opt(s.get(k)).map_err(he)?;
                        let vs = enc.jv_vs(&mut self.h.refs, &j);
                        put(&mut enc.rec, vs);
                    }
                }
            }
            14 | 17 => {
                let j = arg(1)? as usize;
                if code == 14 && j == 0 {
                    let snap = self.sess.clone();
                    self.sl_calls += 1;
                    let c = self.sl_calls;
                    let script = self.sl_script(c);
                    let st = self.lists.entry(cur).or_insert_with(new_list);
                    st.call = c;
                    st.snap = snap;
                    st.script = script.clone();
                    if let Some((t, None)) = &script {
                        let (r, o) = self.thrown_reply(t)?;
                        self.events.push(Ev { c: "sessionList".to_string(), a: format!("{{\"call\":{}}}", c), o: Some(o), ..Default::default() });
                        return Ok((Vec::new(), r, Vec::new()));
                    }
                }
                if code == 14 {
                    let sc = self.lists.get(&cur).and_then(|st| st.script.clone());
                    if let Some((t, Some(at))) = sc {
                        if at == j as i64 {
                            let c = self.lists.get(&cur).map(|st| st.call).unwrap_or(0);
                            if j == 0 {
                                let n0 = self.lists.get(&cur).map(|st| st.snap.len()).unwrap_or(0);
                                self.events.push(Ev { c: "sessionList".to_string(), a: format!("{{\"call\":{},\"size\":{}}}", c, n0), o: None, ..Default::default() });
                            }
                            let (r, o) = self.thrown_reply(&t)?;
                            self.events.push(Ev { c: "sessionList.next".to_string(), a: format!("{{\"call\":{},\"index\":{}}}", c, j), o: Some(o), ..Default::default() });
                            return Ok((Vec::new(), r, Vec::new()));
                        }
                    }
                }
                // Clone only the session this step reads: cloning the whole snapshot per step made a
                // list or open over S sessions cost O(S^2) (spec 13).
                let (sj, n, st_call) = {
                    let st = self.lists.entry(cur).or_insert_with(new_list);
                    (st.snap.get(j).cloned(), st.snap.len(), st.call)
                };
                if code == 14 && j == 0 {
                    self.events.push(Ev { c: "sessionList".to_string(), a: format!("{{\"call\":{},\"size\":{}}}", st_call, n), o: None, ..Default::default() });
                }
                if code == 14 && j >= n {
                    self.events.push(Ev { c: "sessionList.done".to_string(), a: format!("{{\"call\":{},\"consumed\":{}}}", st_call, n), o: None, ..Default::default() });
                    enc.rec.extend_from_slice(&[0, 1]);
                } else {
                    if j >= n {
                        return Err("internal: session index past the snapshot".to_string());
                    }
                    let sj = sj.ok_or("internal: session index past the snapshot")?;
                    if code == 14 {
                        let id = self.sess_id(&sj)?;
                        self.events.push(Ev { c: "sessionList.next".to_string(), a: format!("{{\"call\":{},\"index\":{},\"id\":{}}}", st_call, j, id), o: None, ..Default::default() });
                        enc.rec.extend_from_slice(&[0, 0]);
                    } else {
                        enc.rec.push(0);
                    }
                    let op = if code == 14 { 8 } else { 7 };
                    let mut e2 = Enc::new();
                    let mut r2 = Vec::new();
                    enc_session(&mut e2, &mut self.h.refs, &sj, &mut r2).map_err(he)?;
                    e2.rec = r2;
                    s5 = mk_packet_units(0, op, 0, op, &e2);
                }
            }
            16 => {
                let j = arg(1)?;
                let (c, sj) = {
                    let st = self.lists.entry(cur).or_insert_with(new_list);
                    (st.call, st.snap.get(j as usize).cloned())
                };
                self.events.push(Ev { c: "sessionList.return".to_string(), a: format!("{{\"call\":{},\"consumed\":{}}}", c, j + 1), o: None, ..Default::default() });
                enc.rec.push(0);
                if wk.len() != 54 {
                    // list: the bound session id, compared with the titles by the core when the list finishes
                    let sj = sj.ok_or("internal: bound session outside the snapshot")?;
                    let j = jv_opt(sj.get("id")).map_err(he)?;
                    let vs = enc.jv_vs(&mut self.h.refs, &j);
                    put(&mut enc.rec, vs);
                } else {
                    let sj = sj.ok_or("internal: bound session outside the snapshot")?;
                    match jv_opt(sj.get("id")).map_err(he)? {
                        Jv::Str(u) => {
                            let vs = enc.str_vs(&u);
                            put(&mut enc.rec, vs);
                        }
                        _ => return Err("case: a bound session id must be a string".to_string()),
                    }
                }
            }
            30 => {
                let v = match self.infos.get(&cur).and_then(|i| i.from_app.clone()) {
                    Some(v) => v,
                    None => self.world.get("fromAppAction").cloned().unwrap_or(Value::Bool(false)),
                };
                if v.get("throw").is_some() {
                    let t = v.get("throw").cloned().unwrap();
                    let (r, o) = self.thrown_reply(&t)?;
                    self.events.push(Ev { c: "fromAppAction".to_string(), a: "[]".to_string(), o: Some(o), ..Default::default() });
                    return Ok((Vec::new(), r, Vec::new()));
                }
                let r = render_jv(&parse_jv(&v).map_err(he)?);
                self.events.push(Ev { c: "fromAppAction".to_string(), a: "[]".to_string(), o: Some(format!("{{\"ok\":{}}}", r)), ..Default::default() });
                enc.rec.push(0);
                enc.rec.push(if val_truthy(&v) { 1 } else { 0 });
            }
            31 => {
                let target = units_at(pool, arg(1)?, arg(2)?)?;
                let accts = match oj_get(&self.raw, "grokAccounts") {
                    Some(o) => o.clone(),
                    None => Oj::Arr(Vec::new()),
                };
                if let Some(t) = oj_get(&accts, "throw") {
                    let (r, o) = self.thrown_reply(&oj_value(t))?;
                    self.events.push(Ev { c: "grokAccounts".to_string(), a: "[]".to_string(), o: Some(o), ..Default::default() });
                    return Ok((Vec::new(), r, Vec::new()));
                }
                self.events.push(Ev { c: "grokAccounts".to_string(), a: "[]".to_string(), o: Some(format!("{{\"ok\":{}}}", render_oj(&accts)?)), ..Default::default() });
                let mut known = false;
                if let Oj::Arr(a) = &accts {
                    for x in a {
                        if let Some(id) = oj_get(x, "id") {
                            if oj_units(id).map_or(false, |u| u == target) {
                                known = true;
                            }
                        }
                    }
                }
                enc.rec.push(0);
                enc.rec.push(if known { 1 } else { 0 });
            }
            27 => return self.open_acquire(pool, rec, wk, rows, call),
            29 => {
                // the read window closes (timer clear, listener removal); the facts: whether the cancel handle is aborted
                self.open_cleanup(cur);
                let (ab, rs) = self.cancel_fact();
                enc.rec.push(0);
                enc.rec.push(ab);
                put(&mut enc.rec, rs);
            }
            25 => {
                let tag0 = arg(1)?;
                let mut hold = None;
                if tag0 >= 10 {
                    // the catch's rm of the copy (Pruv asked for it; a rejection is swallowed there)
                    let (_, h) = self.grok_rm()?;
                    hold = h;
                }
                let (wkv, callv) = (wk.to_vec(), call.to_vec());
                if let Some(mut h) = hold {
                    h.cont = Some(Box::new(move |l: &mut Leaf, _| l.finish25(tag0 % 10, &wkv, &callv)));
                    self.pending_hold = Some(h);
                    return Ok((Vec::new(), Vec::new(), Vec::new()));
                }
                return match self.finish25(tag0 % 10, &wkv, &callv)? {
                    Step::Reply(r) => Ok(r),
                    Step::Hold(_) => Err("internal: finish of the failure facts held".to_string()),
                };
            }
            33 => {
                // the facts of the read result the Pruv handler needs for the recipe: title, omittedBefore, lastPlan, then per turn
                // prompt, reply, createdAt, completedAt, tools, clipped (raw values as slots, absent as tag 10)
                let ext_id = *call.get(3).ok_or("internal: view call without a reference")?;
                let ext = self.exts.get(&ext_id).cloned().ok_or("internal: view call with an unknown read result")?;
                let items: Vec<Oj> = match oj_get(&ext, "turns") {
                    Some(Oj::Arr(a)) => a.clone(),
                    _ => return Err("case: reader ok needs a turns array".to_string()),
                };
                enc.rec.push(0);
                for k in ["title", "omittedBefore", "lastPlan"] {
                    let s = self.oj_slot(&mut enc, oj_field(&ext, k));
                    put(&mut enc.rec, s);
                }
                enc.rec.push(items.len() as i64);
                for it in items.iter() {
                    for k in ["prompt", "reply", "createdAt", "completedAt", "tools", "clipped"] {
                        let s = self.oj_slot(&mut enc, oj_field(it, k));
                        put(&mut enc.rec, s);
                    }
                }
            }
            32 => return self.open_create(pool, rec, wk, rows, call, now),
            34 => return self.grok_home(pool, call),
            35 => {
                return match self.grok_copy(pool, rec, wk)? {
                    Step::Reply(r) => Ok(r),
                    Step::Hold(h) => {
                        self.pending_hold = Some(h);
                        Ok((Vec::new(), Vec::new(), Vec::new()))
                    }
                };
            }
            36 => {
                let (r, h) = self.grok_rm()?;
                if let Some(mut h) = h {
                    let r2 = r.clone();
                    h.cont = Some(Box::new(move |_: &mut Leaf, _| Ok(Step::Reply(r2))));
                    self.pending_hold = Some(h);
                    return Ok((Vec::new(), Vec::new(), Vec::new()));
                }
                return Ok(r);
            }
            c => return Err(format!("internal: unknown host call {}", c)),
        }
        Ok((enc.units(), enc.rec, s5))
    }


    // The finally and the failure facts of the open handler (host call 25): the read window and the list timer if live, the
    // registry release, the classification of the thrown reference and the cancel facts (evaluated after the rm settled).
    fn finish25(&mut self, tag: i64, wk: &[i64], call: &[i64]) -> Result<Step, String> {
        let cur = self.cur;
        let mut enc = Enc::new();
        self.open_cleanup(cur);
        if let Some((tid, false)) = self.lists.get(&cur).map(|s| (s.timer, s.closed)) {
            let live = self.active_timers.iter().any(|t| t.0 == tid);
            if live || self.fired.contains(&tid) {
                self.active_timers.retain(|t| t.0 != tid);
                if let Some(st) = self.lists.get_mut(&cur) {
                    st.closed = true;
                }
                self.events.push(Ev { c: "timer.clear".to_string(), a: format!("{{\"id\":{},\"active\":{}}}", tid, live), o: None, ..Default::default() });
            }
        }
        let (evs, _) = Self::parse_eb(call, 2)?;
        self.apply_events(&evs)?;
        let mut is_eie = 0i64;
        let mut name_u: Option<Vec<u16>> = None;
        let mut code_u: Option<Vec<u16>> = None;
        if tag == 1 && wk.len() > 23 && wk[21] == 9 {
            if let Some(RefVal::Thrown(spec)) = self.h.refs.vals.get((wk[22] - 1) as usize) {
                let name = if let Some(r) = &spec.rename {
                    Some(r.clone())
                } else if let Some(n) = &spec.name {
                    Some(n.clone())
                } else {
                    match spec.class {
                        Class::Error => Some(Jv::Str(u16s("Error"))),
                        Class::TypeError => Some(Jv::Str(u16s("TypeError"))),
                        Class::Eie => Some(Jv::Str(u16s("EngineImportError"))),
                        _ => None,
                    }
                };
                if let Some(Jv::Str(n)) = name {
                    name_u = Some(n);
                }
                if spec.class == Class::Eie {
                    is_eie = 1;
                    let ecode = if let Some(c) = &spec.errcode { Some(c.clone()) } else { spec.code.clone() };
                    if let Some(Jv::Str(c)) = ecode {
                        code_u = Some(c);
                    }
                }
            }
        }
        // a cancelled request throws the cancel reason before any mapping (the facts below say so)
        let (ab, rs) = self.cancel_fact();
        if ab == 1 {
            is_eie = 0;
            name_u = None;
            code_u = None;
        }
        enc.rec.push(0);
        enc.rec.push(is_eie);
        for u in [&name_u, &code_u] {
            match u {
                Some(c) => {
                    let vs = enc.str_vs(c);
                    put(&mut enc.rec, vs);
                }
                None => put(&mut enc.rec, [10, 0, 0]),
            }
        }
        enc.rec.push(ab);
        put(&mut enc.rec, rs);
        Ok(Step::Reply((enc.units(), enc.rec, Vec::new())))
    }


    // The scripted sessionList entry of the call with this number: (throw spec, throwAt) when it throws.
    fn sl_script(&self, call: i64) -> Option<(Value, Option<i64>)> {
        let q = self.world.get("sessionList")?;
        let e = match q {
            Value::Array(a) => {
                if a.is_empty() {
                    return None;
                }
                a[std::cmp::min((call - 1) as usize, a.len() - 1)].clone()
            }
            v => v.clone(),
        };
        let t = e.get("throw")?.clone();
        Some((t, e.get("throwAt").and_then(|x| x.as_i64())))
    }

    // A thrown reply for a host call: [2, thrown slot] and the trace outcome of the throw.
    fn thrown_reply(&mut self, spec: &Value) -> Result<(Vec<i64>, String), String> {
        let sp = parse_err_l(spec).map_err(he)?;
        let mut e = Enc::new();
        self.h.throw_rec(&mut e, &sp).map_err(he)?;
        Ok((vec![2, e.rec[1], e.rec[2], e.rec[3]], format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref))))
    }

    fn info(&self) -> Result<ReqInfo, String> {
        self.infos.get(&self.cur).cloned().ok_or("internal: request without info".to_string())
    }

    // An opening-set event: the trace line and the observation entry.
    fn op_event(&mut self, op: &str, key: &[u16], result: bool) -> Result<(), String> {
        let info = self.info()?;
        self.events.push(Ev { c: format!("opening.{}", op), a: format!("{{\"key\":{}}}", jstr(key)), o: Some(format!("{{\"ok\":{}}}", result)), ..Default::default() });
        self.opening_events.push(format!(
            "{{\"i\":{},\"r\":{},\"op\":{},\"key\":{},\"result\":{}}}",
            jstr_s(&info.inst),
            jstr_s(&info.name),
            jstr_s(op),
            jstr(key),
            result
        ));
        Ok(())
    }

    // The finally of the open handler: the read window's timer and listener (once; the host owns them).
    fn open_cleanup(&mut self, cur: i64) {
        if let Some(w) = self.opens.get(&cur).and_then(|o| o.win) {
            self.win_close(w);
        }
    }

    // The opening rows of the world as (instance id, request id, owner token, key units), in row order.
    fn world_rows(&self) -> Vec<(i64, i64, i64, Vec<u16>)> {
        let w = &self.ww;
        let mut out = Vec::new();
        if w.len() < 16 {
            return out;
        }
        let (np, ni, na, no) = (w[8] as usize, w[5] as usize, w[6] as usize, w[7] as usize);
        let base = 16 + np + 2 * ni + 10 * na;
        for r in 0..no {
            let q = base + 6 * r;
            if q + 6 > w.len() {
                break;
            }
            let (off, n) = (w[q + 1] as usize, w[q + 2] as usize);
            let key: Vec<u16> = (0..n).map(|j| w.get(16 + off + j).copied().unwrap_or(0) as u16).collect();
            out.push((w[q], w[q + 3], w[q + 4], key));
        }
        out
    }

    // Reads an event block [n, (op, result, owner, klen, key...)*] at `at`; returns the events and the next index.
    pub fn parse_eb(words: &[i64], at: usize) -> Result<(Vec<(i64, i64, i64, Vec<u16>)>, usize), String> {
        let n = *words.get(at).ok_or("internal: missing event block")?;
        let mut i = at + 1;
        let mut out = Vec::new();
        for _ in 0..n {
            if i + 4 > words.len() {
                return Err("internal: truncated opening event".to_string());
            }
            let (op, res, owner, kl) = (words[i], words[i + 1], words[i + 2], words[i + 3] as usize);
            if i + 4 + kl > words.len() {
                return Err("internal: truncated opening event key".to_string());
            }
            out.push((op, res, owner, words[i + 4..i + 4 + kl].iter().map(|&x| x as u16).collect()));
            i += 4 + kl;
        }
        Ok((out, i))
    }

    // Records the OPENING_EVENTs Pruv reported (ABI 5.5 kind 70). The host validates each against the world it was given
    // (it decides nothing): an add must be a row of this request with this owner, a delete must leave no such key, a has
    // result must match the key's presence in the world (not counting an add reported in the same block).
    pub fn apply_events(&mut self, evs: &[(i64, i64, i64, Vec<u16>)]) -> Result<(), String> {
        let rows = self.world_rows();
        let info = self.info()?;
        for (k, (op, res, owner, key)) in evs.iter().enumerate() {
            let present = rows.iter().any(|r| r.0 == info.inst_id && r.3 == *key);
            let name = match op {
                0 => {
                    let added = evs[k + 1..].iter().any(|e| e.0 == 1 && e.3 == *key);
                    if (*res == 1) != (present && !added) {
                        return Err("internal: opening has event disagrees with the world".to_string());
                    }
                    "has"
                }
                1 => {
                    if !rows.iter().any(|r| r.0 == info.inst_id && r.3 == *key && r.1 == self.cur && r.2 == *owner) {
                        return Err("internal: opening add event is not a row of the world".to_string());
                    }
                    "add"
                }
                2 => {
                    if present {
                        return Err("internal: opening delete event but the key is still in the world".to_string());
                    }
                    "delete"
                }
                _ => return Err("internal: unknown opening event op".to_string()),
            };
            self.op_event(name, key, *res == 1)?;
        }
        Ok(())
    }

    // Called by the request host when a request ends: live timer and listener are cleared, then the done packet's events are recorded.
    pub fn finish_cleanup(&mut self, cur: i64, done_events: &[i64]) -> Result<(), String> {
        self.open_cleanup(cur);
        if !done_events.is_empty() {
            let save = self.cur;
            self.cur = cur;
            let (evs, _) = Self::parse_eb(done_events, 0)?;
            let r = self.apply_events(&evs);
            self.cur = save;
            r?;
        }
        Ok(())
    }

    pub fn set_req(&mut self, cur: i64, name: &str, inst: &str, inst_id: i64, from_app: Option<Value>, cancel: Option<String>) {
        if let Some(c) = &cancel {
            if !self.handles.iter().any(|h| h.name == *c) {
                self.handles.push(Handle { name: c.clone(), aborted: false, active: 0 });
            }
        }
        self.infos.insert(cur, ReqInfo { name: name.to_string(), inst: inst.to_string(), inst_id, from_app, cancel });
    }

    pub fn handle_new(&mut self, name: &str) {
        if !self.handles.iter().any(|h| h.name == name) {
            self.handles.push(Handle { name: name.to_string(), aborted: false, active: 0 });
        }
    }

    pub fn has_hold(&self, label: &str) -> bool {
        self.holds.iter().any(|h| h.label == label)
    }

    pub fn mutate_add(&mut self, session: Value) {
        self.sess.push(session);
    }

    // mutate remove {id}: the sessions whose id is not (===) the given one stay.
    pub fn mutate_remove(&mut self, id: &Value) -> Result<(), String> {
        let key = render_jv(&jv_opt(Some(id)).map_err(he)?);
        let mut keep = Vec::new();
        for s in std::mem::take(&mut self.sess) {
            if self.sess_id(&s)? != key {
                keep.push(s);
            }
        }
        self.sess = keep;
        Ok(())
    }

    // mutate replace {id, session}: the sessions with that id become the new session.
    pub fn mutate_replace(&mut self, id: Option<&Value>, session: Value) -> Result<(), String> {
        let key = render_jv(&jv_opt(id).map_err(he)?);
        let mut out = Vec::new();
        for s in std::mem::take(&mut self.sess) {
            if self.sess_id(&s)? == key {
                out.push(session.clone());
            } else {
                out.push(s);
            }
        }
        self.sess = out;
        Ok(())
    }

    pub fn observation_tail(&self, insts: &[String]) -> String {
        let fin: Vec<String> = insts
            .iter()
            .enumerate()
            .map(|(k, i)| {
                let keys: Vec<String> = self.world_rows().iter().filter(|r| r.0 == k as i64 + 1).map(|r| jstr(&r.3)).collect();
                format!("{}:[{}]", jstr_s(i), keys.join(","))
            })
            .collect();
        let timers: Vec<String> = self.active_timers.iter().map(|(id, due)| format!("{{\"id\":{},\"delay\":20000,\"due\":{}}}", id, due)).collect();
        let hs: Vec<String> = self.handles.iter().map(|h| format!("{}:{}", jstr_s(&h.name), h.active)).collect();
        format!(
            "\"createdSessions\":[{}],\"openingEvents\":[{}],\"finalOpening\":{{{}}},\"activeTimers\":[{}],\"activeListeners\":{{{}}}",
            self.created.join(","),
            self.opening_events.join(","),
            fin.join(","),
            timers.join(","),
            hs.join(",")
        )
    }

    // The n-th entry (last repeats) of the raw world queue at `path`, as an `ok` value or a reject spec. `default` is the
    // JSON of the ok value used when the world has no queue.
    fn dep(&mut self, path: &str, default: &str) -> Result<Result<Oj, Value>, String> {
        let key = format!("world:{}", path);
        let n = {
            let e = self.used.entry(key).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        let entry: Oj = match self.lookup(path).and_then(|node| oj_nth(node, n)) {
            Some(e) => self.ov(e.clone()),
            None => {
                let j = format!("{{\"ok\":{}}}", default);
                (OjP { b: j.as_bytes(), at: 0 }).val()?
            }
        };
        if let Some(r) = oj_get(&entry, "reject") {
            return Ok(Err(oj_value(r)));
        }
        if let Some(o) = oj_get(&entry, "ok") {
            return Ok(Ok(o.clone()));
        }
        Err(format!("case: {} entries other than ok and reject are not supported yet", path))
    }

    // A dependency call served by dep(): emits the event and returns the ok value or the throw reply (status 2).
    fn dep_call(&mut self, path: &str, default: &str, c: &str, a: String) -> Result<Result<Oj, Vec<i64>>, String> {
        // a deferred entry: the call line is pending until the scenario settles the label; the reply waits with it
        let probe = {
            let key = format!("world:{}", path);
            let n = self.used.get(&key).copied().unwrap_or(0);
            self.lookup(path).and_then(|node| oj_nth(node, n)).cloned()
        };
        if let Some(e) = probe {
            if let Some(l) = Self::entry_defer(&e)? {
                let out = self.dep(path, default)?;
                let o = self.outcome_json(&match &out {
                    Ok(v) => Ok(v.clone()),
                    Err(sp) => Err(sp.clone()),
                })?;
                let pid = self.emit_p(c, a);
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, c, pid);
                h.rej = out.is_err();
                h.norm_o = Some(o);
                self.pending_hold = Some(h);
                return match out {
                    Ok(v) => Ok(Ok(v)),
                    Err(spec) => {
                        let sp = parse_err_l(&spec).map_err(he)?;
                        let mut e = Enc::new();
                        self.h.throw_rec(&mut e, &sp).map_err(he)?;
                        Ok(Err(vec![0, 2, e.rec[1], e.rec[2], e.rec[3], 0, 0, 0]))
                    }
                };
            }
        }
        match self.dep(path, default)? {
            Ok(o) => {
                self.events.push(Ev { c: c.to_string(), a, o: Some(format!("{{\"ok\":{}}}", render_oj(&o)?)), ..Default::default() });
                Ok(Ok(o))
            }
            Err(spec) => {
                let sp = parse_err_l(&spec).map_err(he)?;
                let mut e = Enc::new();
                self.h.throw_rec(&mut e, &sp).map_err(he)?;
                self.events.push(Ev { c: c.to_string(), a, o: Some(format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref))), ..Default::default() });
                Ok(Err(vec![0, 2, e.rec[1], e.rec[2], e.rec[3], 0, 0, 0]))
            }
        }
    }

    // Host call 27: opening.has, the busy check, opening.add, the listener and timer, the read options.
    fn open_acquire(&mut self, pool: &[i64], rec: &[i64], wk: &[i64], rows: &[i64], call: &[i64]) -> Result<(Vec<i64>, Vec<i64>, Vec<i64>), String> {
        let cur = self.cur;
        let info = self.info()?;
        if wk.len() != 54 || rec.len() < 39 {
            return Err("internal: open acquire outside an open request".to_string());
        }
        let found = wk[13] as usize;
        let rb = 18 * found;
        if rows.len() < rb + 18 || rows[rb + 3] != 6 {
            return Err("internal: open acquire without a source row".to_string());
        }
        let engine_u = units_at(pool, rows[rb + 4], rows[rb + 5])?;
        let engine = String::from_utf16_lossy(&engine_u);
        let source = units_at(pool, rec[25], rec[26])?;
        let folder = units_at(pool, wk[16], wk[17])?;
        let thread = units_at(pool, rec[31], rec[32])?;
        // Pruv decided the registry (has, busy, add) and reported it in the call's event block; the host records it.
        let (evs, _) = Self::parse_eb(call, 2)?;
        self.apply_events(&evs)?;
        self.opens.insert(cur, OpenSt { win: None, engine: engine.clone(), source: source.clone(), codex: None, opencode: None, ghome: None, copied: None });
        let w = self.win_open()?;
        self.opens.get_mut(&cur).unwrap().win = Some(w);
        if engine == "codex" {
            let dd = match self.world.get("dataDir") {
                Some(Value::String(d)) => d.clone(),
                _ => "/data".to_string(),
            };
            let a = format!("{{\"source\":{},\"dataDir\":{}}}", jstr(&source), jstr_s(&dd));
            match self.dep_call("makeCodexRunDeps", "{\"ref\":\"codexRun\"}", "makeCodexRunDeps", a)? {
                Ok(o) => self.opens.get_mut(&cur).unwrap().codex = Some(render_oj(&o)?),
                Err(r) => return Ok((Vec::new(), r, Vec::new())),
            }
        } else if engine == "opencode" {
            match self.dep_call("locateAcp", "{\"binary\":\"/bin/opencode\"}", "locateAcp", "[\"opencode\"]".to_string())? {
                Ok(o) => {
                    let bin = oj_get(&o, "binary").cloned();
                    if let Some(b) = bin {
                        if oj_truthy(&b) {
                            match self.dep_call("tempRoot", "\"/tmp/lair-root\"", "tempRoot", "[]".to_string())? {
                                Ok(t) => {
                                    let r = format!("{{\"obj\":[[\"binary\",{}],[\"tempRoot\",{}]]}}", render_oj(&b)?, render_oj(&t)?);
                                    self.opens.get_mut(&cur).unwrap().opencode = Some(r);
                                }
                                Err(r) => return Ok((Vec::new(), r, Vec::new())),
                            }
                        }
                    }
                }
                Err(r) => return Ok((Vec::new(), r, Vec::new())),
            }
        }
        let s = self.h.refs.id(&format!("$source{}", cur));
        let o = self.h.refs.id(&format!("$options{}", cur));
        Ok((Vec::new(), vec![0, 0, 9, s, 0, 9, o, 0], Vec::new()))
    }

    // Host call 32: now, one uuid per turn, modelFor, createImported (the fill), and the key release.
    // A value of the read result as a slot: absent 10, undef 0, null 1, bool 2/3, integers 4, other numbers 5, strings 6,
    // anything else an opaque reference into oj_refs.
    fn oj_slot(&mut self, enc: &mut Enc, v: Option<&Oj>) -> [i64; 3] {
        let v = match v {
            None => return [10, 0, 0],
            Some(v) => v,
        };
        if oj_is_undef(v) {
            return [0, 0, 0];
        }
        match v {
            Oj::Scalar(Value::Null) => [1, 0, 0],
            Oj::Scalar(Value::Bool(false)) => [2, 0, 0],
            Oj::Scalar(Value::Bool(true)) => [3, 0, 0],
            Oj::Scalar(Value::Number(n)) if n.as_i64().is_some() => [4, n.as_i64().unwrap_or(0), 0],
            Oj::Scalar(Value::String(s)) => enc.str_vs(&u16s(s)),
            Oj::Obj(p) if p.len() == 1 && p[0].0 == "u16" && oj_units(v).is_some() => enc.str_vs(&oj_units(v).unwrap()),
            Oj::Obj(p) if p.len() == 1 && p[0].0 == "num" => match &p[0].1 {
                Oj::Scalar(Value::String(h)) => {
                    let b = u64::from_str_radix(h, 16).unwrap_or(0);
                    [5, (b & 0xffff_ffff) as i64, (b >> 32) as i64]
                }
                _ => [10, 0, 0],
            },
            other => {
                let id = 1_000_000_000 + self.oj_refs.len() as i64;
                self.oj_refs.insert(id, other.clone());
                [9, id, 0]
            }
        }
    }

    // The value of the slot at q of the call words (the inverse of oj_slot); an absent slot reads as undefined.
    fn slot_oj(&self, pool: &[i64], call: &[i64], q: usize) -> Result<Oj, String> {
        let w = |k: usize| call.get(q + k).copied().ok_or("internal: recipe slot out of range".to_string());
        Ok(match w(0)? {
            0 | 10 => oj_undef(),
            1 => Oj::Scalar(Value::Null),
            2 => Oj::Scalar(Value::Bool(false)),
            3 => Oj::Scalar(Value::Bool(true)),
            4 => Oj::Scalar(Value::from(w(1)?)),
            5 => {
                let b = ((w(2)? as u64) << 32) | (w(1)? as u64 & 0xffff_ffff);
                Oj::Obj(vec![("num".to_string(), Oj::Scalar(Value::String(format!("{:016x}", b))))])
            }
            6 => units_oj(&units_at(pool, w(1)?, w(2)?)?),
            9 => self.oj_refs.get(&w(1)?).cloned().ok_or("internal: recipe reference is unknown")?,
            _ => return Err("internal: recipe slot tag".to_string()),
        })
    }

    // The units of the string slot at q of the call words.
    fn slot_units(pool: &[i64], call: &[i64], q: usize) -> Result<Vec<u16>, String> {
        if call.get(q).copied() != Some(6) {
            return Err("internal: recipe string slot expected".to_string());
        }
        units_at(pool, call[q + 1], call[q + 2])
    }

    fn open_create(&mut self, pool: &[i64], rec: &[i64], wk: &[i64], rows: &[i64], call: &[i64], now: i64) -> Result<(Vec<i64>, Vec<i64>, Vec<i64>), String> {
        let cur = self.cur;
        let (release, at) = Self::parse_eb(call, 2)?;
        let ext_id = *call.get(at + 1).ok_or("internal: create call without a reference")?;
        let ext = self.exts.get(&ext_id).cloned().ok_or("internal: create call with an unknown read result")?;
        let rb = 18 * (wk[13] as usize);
        // the recipe built by the Pruv handler (req_fill.pruv): slots over the request pool, flags, one block per turn
        let r = at;
        if call.len() < r + 29 {
            return Err("internal: create call without a recipe".to_string());
        }
        let flags = call[r + 3];
        let n_turns = call[r + 28] as usize;
        if n_turns > 16777216 || call.len() != r + 29 + 16 * n_turns {
            return Err("internal: create recipe length".to_string());
        }
        let slot_name = String::from_utf16_lossy(&Self::slot_units(pool, call, r + 4)?);
        let title = Self::slot_units(pool, call, r + 7)?;
        let source = Self::slot_units(pool, call, r + 13)?;
        let thread = Self::slot_units(pool, call, r + 16)?;
        let mode = Self::slot_units(pool, call, r + 19)?;
        let folder = Self::slot_units(pool, call, r + 22)?;
        let engine = String::from_utf16_lossy(&Self::slot_units(pool, call, r + 25)?);
        let account = match rows[rb + 9] {
            0 | 10 => Jv::Undef,
            1 => Jv::Null,
            6 => Jv::Str(units_at(pool, rows[rb + 10], rows[rb + 11])?),
            _ => return Err("internal: unexpected account slot".to_string()),
        };
        if let Some(ct) = oj_get(&self.raw, "clock").and_then(|c| oj_get(c, "throw")).cloned() {
            let (r, o) = self.reject_reply(&oj_value(&ct))?;
            self.events.push(Ev { c: "now".to_string(), a: "[]".to_string(), o: Some(o), ..Default::default() });
            return Ok((Vec::new(), r, Vec::new()));
        }
        let clock_start = match oj_get(&self.raw, "clockStart") {
            Some(Oj::Scalar(Value::Number(n))) => n.as_i64().unwrap_or(1_700_000_000_000),
            _ => 1_700_000_000_000,
        };
        let iso = iso_from_ms(clock_start + now);
        self.events.push(Ev { c: "now".to_string(), a: "[]".to_string(), o: Some(format!("{{\"ok\":{}}}", jstr_s(&iso))), ..Default::default() });
        let now_oj = Oj::Scalar(Value::String(iso.clone()));
        let mut turns: Vec<Oj> = Vec::new();
        let mut last_id: Option<String> = None;
        for k in 0..n_turns {
            let tb = r + 29 + 16 * k;
            let tf = call[tb];
            let id = match self.next_uuid()? {
                Ok(id) => id,
                Err(r) => return Ok((Vec::new(), r, Vec::new())),
            };
            last_id = Some(id.clone());
            let created = if tf & 1 != 0 { now_oj.clone() } else { self.slot_oj(pool, call, tb + 7)? };
            let mut t: Vec<(String, Oj)> = vec![
                ("id".to_string(), Oj::Scalar(Value::String(id))),
                ("prompt".to_string(), self.slot_oj(pool, call, tb + 1)?),
                ("reply".to_string(), self.slot_oj(pool, call, tb + 4)?),
                ("progress".to_string(), Oj::Scalar(Value::String(String::new()))),
                ("status".to_string(), Oj::Scalar(Value::String("completed".to_string()))),
                ("usage".to_string(), Oj::Scalar(Value::Null)),
                ("createdAt".to_string(), created),
            ];
            if tf & 2 != 0 {
                t.push(("completedAt".to_string(), self.slot_oj(pool, call, tb + 10)?));
            }
            let mut imp: Vec<(String, Oj)> = vec![("engine".to_string(), Oj::Scalar(Value::String(engine.clone())))];
            if tf & 4 != 0 {
                imp.push(("tools".to_string(), self.slot_oj(pool, call, tb + 13)?));
            }
            if tf & 8 != 0 {
                imp.push(("clipped".to_string(), Oj::Scalar(Value::Bool(true))));
            }
            t.push(("imported".to_string(), Oj::Obj(imp)));
            turns.push(Oj::Obj(t));
        }
        let mut bound: Vec<(String, Oj)> = vec![("threadId".to_string(), units_oj(&thread))];
        if flags & 16 != 0 {
            bound.push(("lastTurnId".to_string(), Oj::Scalar(Value::String(last_id.clone().ok_or("internal: last turn flag without a turn")?))));
        }
        if flags & 2 != 0 {
            bound.push(("fork".to_string(), Oj::Scalar(Value::Bool(true))));
        }
        let mut origin: Vec<(String, Oj)> = vec![
            ("engine".to_string(), Oj::Scalar(Value::String(engine.clone()))),
            ("source".to_string(), units_oj(&source)),
            ("thread".to_string(), units_oj(&thread)),
            ("mode".to_string(), units_oj(&mode)),
            ("importedAt".to_string(), Oj::Scalar(Value::String(iso.clone()))),
        ];
        if flags & 4 != 0 {
            origin.push(("omittedBefore".to_string(), Oj::Scalar(Value::Bool(true))));
        }
        if flags & 8 != 0 {
            origin.push(("lastPlan".to_string(), self.slot_oj(pool, call, r + 10)?));
        }
        // modelFor
        let wanted = oj_field(&ext, "model").cloned().unwrap_or_else(oj_undef);
        let acc_oj = match &account {
            Jv::Str(u) => units_oj(u),
            Jv::Null => Oj::Scalar(Value::Null),
            _ => oj_undef(),
        };
        let a = format!("{{\"engine\":{},\"account\":{},\"wanted\":{}}}", jstr_s(&engine), render_oj(&acc_oj)?, render_oj(&wanted)?);
        let mentry = if oj_get(&self.raw, "modelFor").is_some() { self.world_entry("modelFor") } else { None };
        if let Some(t) = mentry.as_ref().and_then(|e| oj_get(e, "throw")).cloned() {
            let (r, o) = self.reject_reply(&oj_value(&t))?;
            self.events.push(Ev { c: "modelFor".to_string(), a, o: Some(o), ..Default::default() });
            return Ok((Vec::new(), r, Vec::new()));
        }
        let model: Oj = match mentry.as_ref().and_then(|e| oj_get(e, "ok")) {
            Some(o) => o.clone(),
            None => {
                if oj_truthy(&wanted) {
                    wanted.clone()
                } else {
                    Oj::Scalar(Value::String("default-model".to_string()))
                }
            }
        };
        self.events.push(Ev { c: "modelFor".to_string(), a, o: Some(format!("{{\"ok\":{}}}", render_oj(&model)?)), ..Default::default() });
        // createImported
        let has_acct = flags & 1 != 0;
        let mut seed_keys = vec!["\"engine\"".to_string()];
        if has_acct {
            seed_keys.push("\"account\"".to_string());
        }
        seed_keys.extend(["\"model\"".to_string(), "\"folder\"".to_string(), "\"fill\"".to_string()]);
        let mut seed_f = format!("\"engine\":{}", jstr_s(&engine));
        if has_acct {
            seed_f.push_str(&format!(",\"account\":{}", render_oj(&acc_oj)?));
        }
        seed_f.push_str(&format!(",\"model\":{},\"folder\":{}", render_oj(&model)?, jstr(&folder)));
        let ce = match oj_get(&self.raw, "createImported") {
            Some(node) => {
                let key = "world:createImported".to_string();
                let n = {
                    let e = self.used.entry(key).or_insert(0);
                    let n = *e;
                    *e += 1;
                    n
                };
                oj_nth(node, n).cloned()
            }
            None => None,
        };
        let mode: String = match ce.as_ref().and_then(|e| oj_get(e, "mode")) {
            Some(m) => String::from_utf16_lossy(&oj_units(m).ok_or("case: createImported mode must be a string")?),
            None => "ok".to_string(),
        };
        if mode != "ok" && mode != "fail-before-fill" && mode != "fail-after-fill" {
            return Err("case: createImported mode is not known".to_string());
        }
        let fail_spec: Option<Value> = if mode == "ok" { None } else { ce.as_ref().and_then(|e| oj_get(e, "throw")).map(oj_value) };
        if mode == "fail-before-fill" {
            let (r, o) = self.reject_reply(fail_spec.as_ref().ok_or("case: createImported failure needs a throw")?)?;
            self.events.push(Ev { c: "createImported".to_string(), a: format!("{{\"seedKeys\":[{}],{}}}", seed_keys.join(","), seed_f), o: Some(o), ..Default::default() });
            return Ok((Vec::new(), r, Vec::new()));
        }
        self.created_seq += 1;
        let new_id: Oj = match ce.as_ref().and_then(|e| oj_get(e, "id")) {
            Some(i) => i.clone(),
            None => Oj::Scalar(Value::String(format!("new-{}", self.created_seq))),
        };
        let base: Oj = match ce.as_ref().and_then(|e| oj_get(e, "session")) {
            Some(s) => s.clone(),
            None => match oj_get(&self.raw, "newSession") {
                Some(s) => s.clone(),
                None => Oj::Obj(vec![("title".to_string(), Oj::Scalar(Value::String(String::new()))), ("turns".to_string(), Oj::Arr(Vec::new()))]),
            },
        };
        let mut sess: Vec<(String, Oj)> = match base {
            Oj::Obj(p) => p,
            _ => return Err("case: a new session must be an object".to_string()),
        };
        oj_set(&mut sess, "id", new_id.clone());
        let before = render_oj(&Oj::Obj(sess.clone()))?;
        oj_set(&mut sess, "title", units_oj(&title));
        oj_set(&mut sess, "turns", Oj::Arr(turns));
        oj_set(&mut sess, "externalThread", Oj::Obj(origin));
        let mut et: Vec<(String, Oj)> = match sess.iter().find(|(k, _)| k == "engineThreads") {
            Some((_, Oj::Obj(p))) => p.clone(),
            _ => Vec::new(),
        };
        oj_set(&mut et, &slot_name, Oj::Obj(bound));
        oj_set(&mut sess, "engineThreads", Oj::Obj(et.clone()));
        let after = render_oj(&Oj::Obj(sess.clone()))?;
        let id_r = render_oj(&new_id)?;
        self.created.push(format!(
            "{{\"id\":{},\"seedKeys\":[{}],\"seed\":{{{}}},\"before\":{},\"after\":{},\"outcome\":\"{}\"}}",
            id_r,
            seed_keys.join(","),
            seed_f,
            before,
            after,
            mode
        ));
        if mode == "fail-after-fill" {
            let (r, o) = self.reject_reply(fail_spec.as_ref().ok_or("case: createImported failure needs a throw")?)?;
            self.events.push(Ev { c: "createImported".to_string(), a: format!("{{\"seedKeys\":[{}],{}}}", seed_keys.join(","), seed_f), o: Some(o), ..Default::default() });
            return Ok((Vec::new(), r, Vec::new()));
        }
        self.events.push(Ev { c: "createImported".to_string(), a: format!("{{\"seedKeys\":[{}],{}}}", seed_keys.join(","), seed_f), o: Some(format!("{{\"ok\":{}}}", id_r)), ..Default::default() });
        // the stored session: engineThreads in the entry host's {"obj":[[k,v],..]} form
        let mut m = serde_json::Map::new();
        for (k, v) in sess.iter() {
            if k == "engineThreads" {
                let pairs: Vec<Value> = et.iter().map(|(a, b)| Value::Array(vec![Value::String(a.clone()), oj_value(b)])).collect();
                let mut o = serde_json::Map::new();
                o.insert("obj".to_string(), Value::Array(pairs));
                m.insert(k.clone(), Value::Object(o));
            } else {
                m.insert(k.clone(), oj_value(v));
            }
        }
        self.sess.push(Value::Object(m));
        self.open_cleanup(cur);
        self.apply_events(&release)?;
        let mut enc = Enc::new();
        enc.rec.push(0);
        match oj_units(&new_id) {
            Some(u) => {
                let vs = enc.str_vs(&u);
                put(&mut enc.rec, vs);
            }
            None => return Err("case: a created session id must be a string".to_string()),
        }
        Ok((enc.units(), enc.rec, Vec::new()))
    }

    // The n-th entry (last repeats) of the raw world queue at `path`, or None when the world has none.
    fn world_entry(&mut self, path: &str) -> Option<Oj> {
        let key = format!("world:{}", path);
        let n = {
            let e = self.used.entry(key).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        self.lookup(path).and_then(|node| oj_nth(node, n)).cloned().map(|e| self.ov(e))
    }

    // A thrown reply of a dependency reject spec: the reply record [2, thrown slot] and the trace outcome.
    fn reject_reply(&mut self, spec: &Value) -> Result<(Vec<i64>, String), String> {
        self.thrown_reply(spec)
    }

    // Host call 34: the grokHomeFor dependency (the account Pruv decided on, undefined when none). The ok value is kept as the
    // facts the copy needs; a rejection is a thrown reply.
    fn grok_home(&mut self, pool: &[i64], call: &[i64]) -> Result<(Vec<i64>, Vec<i64>, Vec<i64>), String> {
        let cur = self.cur;
        let acct = match call.get(2) {
            Some(6) => jstr(&units_at(pool, call[3], call[4])?),
            _ => "{\"undef\":1}".to_string(),
        };
        let a = format!("[{}]", acct);
        let dflt = "{\"home\":\"/grok/home\",\"binary\":\"/grok/bin\",\"env\":{\"ref\":\"grokEnv\"}}";
        match self.dep("grokHomeFor", dflt)? {
            Ok(o) => {
                self.events.push(Ev { c: "grokHomeFor".to_string(), a, o: Some(format!("{{\"ok\":{}}}", render_oj(&o)?)), ..Default::default() });
                let home = oj_field(&o, "home").and_then(oj_units).ok_or("case: grokHomeFor ok needs a string home")?;
                let bin = render_oj(oj_field(&o, "binary").unwrap_or(&oj_undef()))?;
                let env = render_oj(oj_field(&o, "env").unwrap_or(&oj_undef()))?;
                if let Some(st) = self.opens.get_mut(&cur) {
                    st.ghome = Some((home, bin, env));
                }
                Ok((Vec::new(), vec![0], Vec::new()))
            }
            Err(spec) => {
                self.events.push(Ev { c: "grokHomeFor".to_string(), a, o: None, ..Default::default() });
                let idx = self.events.len() - 1;
                let (r, o) = self.reject_reply(&spec)?;
                self.events[idx].o = Some(o);
                Ok((Vec::new(), r, Vec::new()))
            }
        }
    }

    // The n-th entry (last repeats) of the world queue at `path`; the default ok entry when the world has none.
    fn dep_entry(&mut self, path: &str, default: &str) -> Result<Oj, String> {
        let key = format!("world:{}", path);
        let n = {
            let e = self.used.entry(key).or_insert(0);
            let n = *e;
            *e += 1;
            n
        };
        match self.lookup(path).and_then(|node| oj_nth(node, n)) {
            Some(e) => Ok(self.ov(e.clone())),
            None => {
                let j = format!("{{\"ok\":{}}}", default);
                (OjP { b: j.as_bytes(), at: 0 }).val()
            }
        }
    }

    // The outcome an entry settles with (reject spec or ok value; an entry with neither is ok undefined).
    fn entry_outcome(e: &Oj) -> Result<Oj, Value> {
        if let Some(r) = oj_get(e, "reject") {
            return Err(oj_value(r));
        }
        Ok(oj_get(e, "ok").cloned().unwrap_or_else(oj_undef))
    }

    fn entry_defer(e: &Oj) -> Result<Option<String>, String> {
        match oj_get(e, "defer") {
            Some(d) => Ok(Some(oj_units(d).map(|u| String::from_utf16_lossy(&u)).ok_or("case: defer label must be a string")?)),
            None => Ok(None),
        }
    }

    // Host call 35: the copyGrokSession dependency (the world's copy entry), with the verify callback it runs (the loadCheck
    // dependency under a new timed window). A verify failure other than AbortError is wrapped the way the real copy wraps it.
    // The copy entry and the loadCheck entry may be deferred; the reply is then given when they settle.
    fn grok_copy(&mut self, pool: &[i64], rec: &[i64], wk: &[i64]) -> Result<Step, String> {
        let cur = self.cur;
        let info = self.info()?;
        let (source, ghome) = match self.opens.get(&cur) {
            Some(st) => (st.source.clone(), st.ghome.clone().ok_or("internal: copy without the grok home")?),
            None => return Err("internal: copy without open state".to_string()),
        };
        let folder = units_at(pool, wk[16], wk[17])?;
        let thread = units_at(pool, rec[31], rec[32])?;
        let entry = self.world_entry("copy").ok_or("case: unscripted call: copyGrokSession")?;
        let (sig, aborted) = match &info.cancel {
            Some(h) => (jstr_s(h), self.handles.iter().find(|x| x.name == *h).map_or(false, |x| x.aborted)),
            None => ("null".to_string(), false),
        };
        let a = format!(
            "{{\"source\":{},\"folder\":{},\"thread\":{},\"targetHome\":{},\"verify\":\"function\",\"signal\":{},\"aborted\":{}}}",
            jstr(&source),
            jstr(&folder),
            jstr(&thread),
            jstr(&ghome.0),
            sig,
            aborted
        );
        if let Some(t) = oj_get(&entry, "sync_throw") {
            let (rr, o) = self.reject_reply(&oj_value(t))?;
            self.events.push(Ev { c: "copy".to_string(), a, o: Some(o), ..Default::default() });
            return Ok(Step::Reply((Vec::new(), rr, Vec::new())));
        }
        let pid = self.emit_p("copy", a);
        let ctx = CopyCtx { pid, entry: entry.clone(), home: ghome.0, bin: ghome.1, env: ghome.2, folder, thread };
        match Self::entry_defer(&entry)? {
            None => self.copy_run(ctx),
            Some(l) => {
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, "copy", pid);
                h.rej = oj_get(&entry, "reject").is_some();
                h.cont = Some(Box::new(move |lf: &mut Leaf, _| lf.copy_run(ctx)));
                Ok(Step::Hold(h))
            }
        }
    }

    // The copy after its own gate: the rejection, or the verify callback, then the result.
    fn copy_run(&mut self, ctx: CopyCtx) -> Result<Step, String> {
        if let Some(r) = oj_get(&ctx.entry, "reject") {
            let (rr, o) = self.reject_reply(&oj_value(r))?;
            self.patch(ctx.pid, o);
            return Ok(Step::Reply((Vec::new(), rr, Vec::new())));
        }
        if !matches!(oj_get(&ctx.entry, "verify"), Some(Oj::Scalar(Value::Bool(true)))) {
            return self.copy_done(ctx);
        }
        self.events.push(Ev { c: "copy.verify".to_string(), a: format!("{{\"home\":{}}}", jstr(&ctx.home)), o: None, ..Default::default() });
        let w = self.win_open()?;
        let sg = self.win_sig(w);
        let wab = self.wins[w].aborted;
        let a2 = format!(
            "{{\"checkKeys\":[\"binary\",\"env\"],\"binary\":{},\"env\":{},\"home\":{},\"folder\":{},\"thread\":{},\"signal\":\"sig{}\",\"aborted\":{}}}",
            ctx.bin,
            ctx.env,
            jstr(&ctx.home),
            jstr(&ctx.folder),
            jstr(&ctx.thread),
            sg,
            wab
        );
        let lent = self.dep_entry("loadCheck", "{\"undef\":1}")?;
        let out = Self::entry_outcome(&lent);
        let o = self.outcome_json(&out)?;
        match Self::entry_defer(&lent)? {
            None => {
                self.events.push(Ev { c: "loadCheck".to_string(), a: a2, o: Some(o), ..Default::default() });
                self.copy_after_check(ctx, w, out)
            }
            Some(l) => {
                let pid2 = self.emit_p("loadCheck", a2);
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, "loadCheck", pid2);
                h.rej = out.is_err();
                h.norm_o = Some(o);
                h.win = Some(w);
                let onab = oj_get(&lent, "onAbort").map(oj_value);
                if let Some(ab) = &onab {
                    let sp = parse_err_l(ab).map_err(he)?;
                    h.abort_o = Some(format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref)));
                }
                h.cont = Some(Box::new(move |lf: &mut Leaf, via: bool| {
                    let out2 = if via { Err(onab.ok_or("internal: abort of a hold without onAbort")?) } else { out };
                    lf.copy_after_check(ctx, w, out2)
                }));
                Ok(Step::Hold(h))
            }
        }
    }

    // The trace outcome of a settled entry.
    fn outcome_json(&mut self, out: &Result<Oj, Value>) -> Result<String, String> {
        match out {
            Ok(o) => Ok(format!("{{\"ok\":{}}}", render_oj(o)?)),
            Err(spec) => {
                let sp = parse_err_l(spec).map_err(he)?;
                Ok(format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref)))
            }
        }
    }

    // After the verify callback: the window closes, then the copy's translation of a failure or its result.
    fn copy_after_check(&mut self, ctx: CopyCtx, w: usize, out: Result<Oj, Value>) -> Result<Step, String> {
        self.win_close(w);
        match out {
            Ok(_) => self.copy_done(ctx),
            Err(spec) => {
                let sp = parse_err_l(&spec).map_err(he)?;
                let mut e = Enc::new();
                self.h.throw_rec(&mut e, &sp).map_err(he)?;
                let is_abort = match &sp.name {
                    Some(Jv::Str(n)) => *n == u16s("AbortError"),
                    _ => false,
                };
                if is_abort {
                    self.patch(ctx.pid, format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&sp.name_ref)));
                    return Ok(Step::Reply((Vec::new(), vec![2, e.rec[1], e.rec[2], e.rec[3]], Vec::new())));
                }
                // the wrapped EngineImportError('grok-load-failed', undefined, {cause})
                self.wrap_seq += 1;
                let wsp = ErrSpec { class: Class::Eie, name_ref: format!("$wrap{}", self.wrap_seq), code: Some(Jv::Str(u16s("grok-load-failed"))), name: None, message: None, errcode: None, value: None, rename: None };
                let mut we = Enc::new();
                self.h.throw_rec(&mut we, &wsp).map_err(he)?;
                self.patch(
                    ctx.pid,
                    format!("{{\"throw\":{{\"eie\":{{\"code\":\"grok-load-failed\",\"message\":\"grok-load-failed\",\"hasCause\":true,\"cause\":{{\"ref\":{}}}}}}}}}", jstr_s(&sp.name_ref)),
                );
                Ok(Step::Reply((Vec::new(), vec![2, we.rec[1], we.rec[2], we.rec[3]], Vec::new())))
            }
        }
    }

    // The copy's result {path, skipped} (entry.ok or the default), the copied path kept, and the cancel facts as the reply.
    fn copy_done(&mut self, ctx: CopyCtx) -> Result<Step, String> {
        let cur = self.cur;
        let path: Vec<u16> = match oj_get(&ctx.entry, "ok") {
            None => {
                self.patch(ctx.pid, "{\"ok\":{\"obj\":[[\"path\",\"/grok/home/sessions/copy\"],[\"skipped\",0]]}}".to_string());
                u16s("/grok/home/sessions/copy")
            }
            Some(o) => {
                let p = oj_field(o, "path").and_then(oj_units).ok_or("case: copy ok needs a string path")?;
                self.patch(ctx.pid, format!("{{\"ok\":{}}}", render_oj(o)?));
                p
            }
        };
        if let Some(st) = self.opens.get_mut(&cur) {
            st.copied = Some(path);
        }
        let (ab, rs) = self.cancel_fact();
        let mut rec = vec![0, ab];
        put(&mut rec, rs);
        Ok(Step::Reply((Vec::new(), rec, Vec::new())))
    }

    // Host call 36 and the catch's rm: rm of the copied path (recursive, force) of the open request; the outcome is traced and a
    // rejection is answered as a thrown reply (Pruv swallows it). A deferred rm comes back as a hold for the caller to finish.
    fn grok_rm(&mut self) -> Result<(Reply, Option<Hold>), String> {
        let cur = self.cur;
        let path = self.opens.get(&cur).and_then(|st| st.copied.clone()).ok_or("internal: rm without a copy")?;
        let a = format!("{{\"path\":{},\"options\":{{\"obj\":[[\"recursive\",true],[\"force\",true]]}}}}", jstr(&path));
        let entry = self.dep_entry("rm", "{\"undef\":1}")?;
        let out = Self::entry_outcome(&entry);
        let o = self.outcome_json(&out)?;
        let reply: Reply = match &out {
            Ok(_) => (Vec::new(), vec![0], Vec::new()),
            Err(spec) => {
                let sp = parse_err_l(spec).map_err(he)?;
                let mut e = Enc::new();
                self.h.throw_rec(&mut e, &sp).map_err(he)?;
                (Vec::new(), vec![2, e.rec[1], e.rec[2], e.rec[3]], Vec::new())
            }
        };
        match Self::entry_defer(&entry)? {
            None => {
                self.events.push(Ev { c: "rm".to_string(), a, o: Some(o), ..Default::default() });
                Ok((reply, None))
            }
            Some(l) => {
                let pid = self.emit_p("rm", a);
                let lab = self.new_label(&l);
                let mut h = self.make_hold(lab, "rm", pid);
                h.rej = out.is_err();
                h.norm_o = Some(o);
                Ok((reply, Some(h)))
            }
        }
    }

    fn next_uuid(&mut self) -> Result<Result<String, Vec<i64>>, String> {
        let list: Vec<Oj> = match oj_get(&self.raw, "uuids") {
            Some(Oj::Arr(a)) => a.clone(),
            _ => Vec::new(),
        };
        let n = self.uuid_seq;
        self.uuid_seq += 1;
        let v = if n < list.len() {
            match &list[n] {
                Oj::Scalar(Value::String(s)) => s.clone(),
                Oj::Obj(_) if oj_get(&list[n], "throw").is_some() => {
                    let t = oj_value(oj_get(&list[n], "throw").unwrap());
                    let (r, o) = self.reject_reply(&t)?;
                    self.events.push(Ev { c: "uuid".to_string(), a: "[]".to_string(), o: Some(o), ..Default::default() });
                    return Ok(Err(r));
                }
                _ => return Err("case: uuid entries other than strings are not supported yet".to_string()),
            }
        } else {
            format!("00000000-0000-4000-8000-{:012}", n - list.len() + 1)
        };
        self.events.push(Ev { c: "uuid".to_string(), a: "[]".to_string(), o: Some(format!("{{\"ok\":{}}}", jstr_s(&v))), ..Default::default() });
        Ok(Ok(v))
    }

    // checkFolder (world.checkFolder: per-path queue, "*" fallback, default ok path). Returns the completion
    // (pool, record) and the oracle trace outcome JSON ({"ok":..} or {"throw":{"ref":..}}).
    pub fn serve_folder(&mut self, path: &[u16]) -> Result<(Vec<i64>, Vec<i64>, String), String> {
        let key = String::from_utf16_lossy(path);
        let queue: Option<Value> = {
            let t = self.world.get("checkFolder").and_then(|v| v.as_object());
            t.and_then(|m| m.get(&key).or_else(|| m.get("*"))).cloned()
        };
        let list: Vec<Value> = match queue {
            Some(Value::Array(a)) => a,
            Some(v) => vec![v],
            None => Vec::new(),
        };
        let entry: Value = if list.is_empty() {
            let mut m = serde_json::Map::new();
            m.insert("ok".to_string(), Value::String(key.clone()));
            Value::Object(m)
        } else {
            let k = format!("checkFolder:{}", key);
            let n = *self.used.get(&k).unwrap_or(&0);
            self.used.insert(k, n + 1);
            list[n.min(list.len() - 1)].clone()
        };
        let mut enc = Enc::new();
        let o: String;
        if let Some(t) = entry.get("throw") {
            let spec = parse_err_l(t).map_err(he)?;
            self.h.throw_rec(&mut enc, &spec).map_err(he)?;
            o = format!("{{\"throw\":{{\"ref\":{}}}}}", jstr_s(&spec.name_ref));
        } else {
            let v = match entry.get("ok") {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("case: checkFolder ok entries must be strings".to_string()),
            };
            Host::ok_str(&mut enc, &u16s(&v));
            o = format!("{{\"ok\":{}}}", jstr_s(&v));
        }
        Ok((enc.units(), enc.rec, o))
    }

    // The prim records of the calls served since the last call (JSON arrays such as ["accounts","claude"]).
    pub fn take_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.h.trace)
    }

    // Renders a done packet's record (section 1) into (terminal, value JSON). `secs` are the request-packet sections.
    pub fn render_done(&self, secs: Vec<Vec<i64>>) -> Result<(String, String), String> {
        let p = Pkt { kind: 3, op_id: 0, seq: 0, op: 0, secs: secs.iter().map(Vec::as_slice).collect(), pool: Pool::Words(secs.first().map(Vec::as_slice).unwrap_or(&[])) };
        self.render_done_in(&p).map_err(he)
    }

    fn render_done_in(&self, p: &Pkt) -> R<(String, String)> {
        let refs = &self.h.refs;
        match p.word(1, 0)? {
            0 => Ok(("ok".to_string(), self.slot(p, 1, 1)?)),
            1 => match p.val(1, 1)? {
                V::Ref(id) => {
                    if let Some(c) = refs.name(id).and_then(|n| self.cancel_value(n)) {
                        return Ok(("thrown".to_string(), c));
                    }
                    Ok(("thrown".to_string(), render_ref(refs, id)?))
                }
                _ => ie("thrown identity is not a reference"),
            },
            2 => {
                let k = self.slot(p, 1, 1)?;
                let m = self.slot(p, 1, 4)?;
                Ok(("wire".to_string(), format!("{{\"wire\":{{\"key\":{},\"message\":{}}}}}", k, m)))
            }
            3 => Ok(("constructed".to_string(), self.slot(p, 1, 1)?)),
            5 => {
                let name = match p.word(1, 1)? {
                    0 => "InvalidPacket",
                    1 => "InputTooLarge",
                    2 => "InternalInvariant",
                    _ => return ie("unknown core code"),
                };
                Ok(("core".to_string(), format!("{{\"code\":\"{}\"}}", name)))
            }
            _ => ie("unknown done record tag"),
        }
    }

    // A value slot with its tables as oracle JV: arrays (tag 7, 3-word rows) and objects (tag 8, 6-word rows).
    fn slot(&self, p: &Pkt, sec: usize, off: usize) -> R<String> {
        let refs = &self.h.refs;
        let tag = p.word(sec, off)?;
        match tag {
            7 | 8 => {
                let (s, n) = match p.val(sec, off)? {
                    V::Table(s, n) => (s, n),
                    _ => return ie("table slot expected"),
                };
                let w = if tag == 7 { 3 } else { 6 };
                if p.secs[s].len() != n * w {
                    return ie("table section length does not match count * width");
                }
                let mut out: Vec<String> = Vec::new();
                for i in 0..n {
                    if tag == 7 {
                        out.push(self.slot(p, s, i * 3)?);
                    } else {
                        let k = self.slot(p, s, i * 6)?;
                        let v = self.slot(p, s, i * 6 + 3)?;
                        out.push(format!("[{},{}]", k, v));
                    }
                }
                if tag == 7 {
                    Ok(format!("{{\"arr\":[{}]}}", out.join(",")))
                } else {
                    Ok(format!("{{\"obj\":[{}]}}", out.join(",")))
                }
            }
            _ => render_v(refs, &p.val(sec, off)?),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// scenario input rendering with key order (the oracle renders plain objects as {"obj":[[key,value],..]})

#[derive(Clone)]
enum Oj {
    Scalar(Value),
    Arr(Vec<Oj>),
    Obj(Vec<(String, Oj)>),
}

struct OjP<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> OjP<'a> {
    fn ws(&mut self) {
        while self.at < self.b.len() && (self.b[self.at] as char).is_ascii_whitespace() {
            self.at += 1;
        }
    }
    fn tok_str(&mut self) -> Result<String, String> {
        let start = self.at;
        self.at += 1;
        while self.at < self.b.len() {
            match self.b[self.at] {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    let s = std::str::from_utf8(&self.b[start..self.at]).map_err(|e| e.to_string())?;
                    return serde_json::from_str::<String>(s).map_err(|e| e.to_string());
                }
                _ => self.at += 1,
            }
        }
        Err("unterminated string".to_string())
    }
    fn val(&mut self) -> Result<Oj, String> {
        self.ws();
        if self.at >= self.b.len() {
            return Err("unexpected end".to_string());
        }
        match self.b[self.at] {
            b'{' => {
                self.at += 1;
                let mut out = Vec::new();
                loop {
                    self.ws();
                    if self.b.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        break;
                    }
                    if self.b.get(self.at) == Some(&b',') {
                        self.at += 1;
                        continue;
                    }
                    let k = self.tok_str()?;
                    self.ws();
                    if self.b.get(self.at) != Some(&b':') {
                        return Err("expected ':'".to_string());
                    }
                    self.at += 1;
                    let v = self.val()?;
                    out.push((k, v));
                }
                Ok(Oj::Obj(out))
            }
            b'[' => {
                self.at += 1;
                let mut out = Vec::new();
                loop {
                    self.ws();
                    if self.b.get(self.at) == Some(&b']') {
                        self.at += 1;
                        break;
                    }
                    if self.b.get(self.at) == Some(&b',') {
                        self.at += 1;
                        continue;
                    }
                    out.push(self.val()?);
                }
                Ok(Oj::Arr(out))
            }
            b'"' => {
                let s = self.tok_str()?;
                Ok(Oj::Scalar(Value::String(s)))
            }
            _ => {
                let start = self.at;
                while self.at < self.b.len() && !matches!(self.b[self.at], b',' | b'}' | b']') && !(self.b[self.at] as char).is_ascii_whitespace() {
                    self.at += 1;
                }
                let s = std::str::from_utf8(&self.b[start..self.at]).map_err(|e| e.to_string())?;
                serde_json::from_str::<Value>(s).map(Oj::Scalar).map_err(|e| e.to_string())
            }
        }
    }
}

fn oj_value(o: &Oj) -> Value {
    match o {
        Oj::Scalar(v) => v.clone(),
        Oj::Arr(a) => Value::Array(a.iter().map(oj_value).collect()),
        Oj::Obj(p) => {
            let mut m = serde_json::Map::new();
            for (k, v) in p {
                m.insert(k.clone(), oj_value(v));
            }
            Value::Object(m)
        }
    }
}

fn render_jv(j: &Jv) -> String {
    match j {
        Jv::Undef => "{\"undef\":1}".to_string(),
        Jv::Null => "null".to_string(),
        Jv::Bool(b) => b.to_string(),
        Jv::Num(n) => render_num(*n),
        Jv::Str(s) => jstr(s),
        Jv::Ref(n) => format!("{{\"ref\":{}}}", jstr_s(n)),
        Jv::Absent => "{\"undef\":1}".to_string(),
    }
}

fn is_absent(o: &Oj) -> bool {
    match o {
        Oj::Obj(p) => p.len() == 1 && p[0].0 == "absent",
        _ => false,
    }
}

fn render_oj(o: &Oj) -> Result<String, String> {
    match o {
        Oj::Scalar(v) => Ok(render_jv(&parse_jv(v).map_err(he)?)),
        Oj::Arr(a) => {
            let mut out = Vec::new();
            for x in a {
                out.push(render_oj(x)?);
            }
            Ok(format!("{{\"arr\":[{}]}}", out.join(",")))
        }
        Oj::Obj(p) => {
            if p.len() == 1 && matches!(p[0].0.as_str(), "u16" | "undef" | "num" | "ref") {
                return Ok(render_jv(&parse_jv(&oj_value(o)).map_err(he)?));
            }
            if p.len() == 1 && p[0].0 == "arr" {
                if let Oj::Arr(_) = p[0].1 {
                    return render_oj(&p[0].1);
                }
            }
            let mut out = Vec::new();
            for (k, v) in p {
                if is_absent(v) {
                    continue;
                }
                out.push(format!("[{},{}]", jstr_s(k), render_oj(v)?));
            }
            Ok(format!("{{\"obj\":[{}]}}", out.join(",")))
        }
    }
}

pub fn render_input_json(raw: &str) -> Result<String, String> {
    let mut p = OjP { b: raw.as_bytes(), at: 0 };
    let o = p.val()?;
    render_oj(&o)
}

// A JSON string literal of `s` (for the request host's trace lines).
pub fn json_str(s: &str) -> String {
    jstr_s(s)
}

// A JSON string literal of UTF-16 units (for the request host's trace lines).
pub fn json_units(u: &[u16]) -> String {
    jstr(u)
}

// ---------------------------------------------------------------------------------------------------------------
// JavaScript value helpers over Oj (the open handler's turn, title and truthiness rules)

fn oj_undef() -> Oj {
    Oj::Obj(vec![("undef".to_string(), Oj::Scalar(Value::from(1)))])
}

fn oj_is_undef(o: &Oj) -> bool {
    match o {
        Oj::Obj(p) => p.len() == 1 && (p[0].0 == "undef" || p[0].0 == "absent"),
        _ => false,
    }
}

fn oj_is_nullish(o: &Oj) -> bool {
    oj_is_undef(o) || matches!(o, Oj::Scalar(Value::Null))
}

// A field of an object value; None when missing or absent.
fn oj_field<'a>(o: &'a Oj, k: &str) -> Option<&'a Oj> {
    match oj_get(o, k) {
        Some(v) if !is_absent(v) => Some(v),
        _ => None,
    }
}

fn val_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map_or(true, |f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) => true,
        Value::Object(m) => {
            if m.len() == 1 {
                if m.contains_key("undef") || m.contains_key("absent") {
                    return false;
                }
                if let Some(Value::Array(a)) = m.get("u16") {
                    return !a.is_empty();
                }
                if let Some(Value::String(h)) = m.get("num") {
                    if let Ok(b) = u64::from_str_radix(h, 16) {
                        let f = f64::from_bits(b);
                        return f != 0.0 && !f.is_nan();
                    }
                }
            }
            true
        }
    }
}

fn oj_truthy(o: &Oj) -> bool {
    match o {
        Oj::Arr(_) => true,
        other => val_truthy(&oj_value(other)),
    }
}

fn oj_gt0(o: &Oj) -> bool {
    match o {
        Oj::Scalar(Value::Number(n)) => n.as_f64().map_or(false, |f| f > 0.0),
        Oj::Obj(p) if p.len() == 1 && p[0].0 == "num" => match &p[0].1 {
            Oj::Scalar(Value::String(h)) => u64::from_str_radix(h, 16).map_or(false, |b| f64::from_bits(b) > 0.0),
            _ => false,
        },
        _ => false,
    }
}

fn oj_units(o: &Oj) -> Option<Vec<u16>> {
    match o {
        Oj::Scalar(Value::String(s)) => Some(s.encode_utf16().collect()),
        Oj::Obj(p) if p.len() == 1 && p[0].0 == "u16" => match &p[0].1 {
            Oj::Arr(a) => a
                .iter()
                .map(|x| match x {
                    Oj::Scalar(Value::Number(n)) => n.as_u64().map(|v| v as u16),
                    _ => None,
                })
                .collect(),
            _ => None,
        },
        _ => None,
    }
}

fn units_oj(u: &[u16]) -> Oj {
    match String::from_utf16(u) {
        Ok(s) => Oj::Scalar(Value::String(s)),
        Err(_) => Oj::Obj(vec![("u16".to_string(), Oj::Arr(u.iter().map(|&x| Oj::Scalar(Value::from(x as u64))).collect()))]),
    }
}

fn oj_set(p: &mut Vec<(String, Oj)>, k: &str, v: Oj) {
    match p.iter_mut().find(|(kk, _)| kk == k) {
        Some(e) => e.1 = v,
        None => p.push((k.to_string(), v)),
    }
}

fn iso_from_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

fn new_list() -> ListSt {
    ListSt { source: Vec::new(), folder: Vec::new(), codex: None, timer: 0, call: 0, snap: Vec::new(), script: None, closed: false }
}

// A scenario session in the entry host's form: engineThreads as {"obj":[[k,bound],..]} when it is a plain object.
fn session_value(o: &Oj) -> Value {
    let p = match o {
        Oj::Obj(p) => p,
        other => return oj_value(other),
    };
    let mut m = serde_json::Map::new();
    for (k, v) in p {
        let conv = match (k.as_str(), v) {
            ("engineThreads", Oj::Obj(q)) if !(q.len() == 1 && matches!(q[0].0.as_str(), "obj" | "undef" | "absent")) => {
                let pairs: Vec<Value> = q.iter().map(|(a, b)| Value::Array(vec![Value::String(a.clone()), oj_value(b)])).collect();
                let mut ob = serde_json::Map::new();
                ob.insert("obj".to_string(), Value::Array(pairs));
                Value::Object(ob)
            }
            _ => oj_value(v),
        };
        m.insert(k.clone(), conv);
    }
    Value::Object(m)
}


// Holds, windows and the scheduler facts of the leaf (ABI 7): what is pending, which timers and listeners are live and what
// the abort of a signal settles. The leaf records the facts; the handler decisions stay in Pruv.
impl Leaf {
    // The scenario's settle overrides (label -> {ok} or {reject}), applied to the deferred world entry when it is served.
    pub fn set_override(&mut self, label: &str, json: &str) -> Result<(), String> {
        let v = (OjP { b: json.as_bytes(), at: 0 }).val()?;
        self.ovr.insert(label.to_string(), v);
        Ok(())
    }

    // The entry as the scenario will settle it: a deferred entry whose label gets an override loses its own ok/reject.
    fn ov(&self, e: Oj) -> Oj {
        let l = match oj_get(&e, "defer").and_then(oj_units) {
            Some(u) => String::from_utf16_lossy(&u),
            None => return e,
        };
        let c = self.label_count.get(&l).copied().unwrap_or(0) + 1;
        let lab = if c == 1 { l } else { format!("{}#{}", l, c) };
        let o = match self.ovr.get(&lab) {
            Some(o) => o,
            None => return e,
        };
        let (Oj::Obj(ep), Oj::Obj(op)) = (&e, o) else { return e };
        let mut out: Vec<(String, Oj)> = ep.iter().filter(|(k, _)| k != "ok" && k != "reject").cloned().collect();
        for (k, v) in op {
            out.push((k.clone(), v.clone()));
        }
        Oj::Obj(out)
    }

    fn new_label(&mut self, l: &str) -> String {
        let c = self.label_count.entry(l.to_string()).or_insert(0);
        *c += 1;
        if *c == 1 {
            l.to_string()
        } else {
            format!("{}#{}", l, c)
        }
    }

    fn new_pid(&mut self) -> i64 {
        self.pid_seq += 1;
        self.pid_seq
    }

    fn patch(&mut self, pid: i64, o: String) {
        if pid != 0 {
            self.patches.insert(pid, o);
        }
    }

    // A trace line whose outcome is filled in when the call settles (placeholder until then, `pending` at the end).
    fn emit_p(&mut self, c: &str, a: String) -> i64 {
        let pid = self.new_pid();
        self.events.push(Ev { c: c.to_string(), a, o: None, pid, ..Default::default() });
        pid
    }

    // A fresh hold for the call being served; the host arms it with the reply once the call is served.
    fn make_hold(&mut self, label: String, call: &str, pid: i64) -> Hold {
        Hold {
            label,
            call: call.to_string(),
            owner: self.cur,
            rej: false,
            win: None,
            pid,
            norm_o: None,
            abort_o: None,
            pre: (Vec::new(), Vec::new(), Vec::new()),
            abort_pre: None,
            abort_comp: None,
            cont: None,
        }
    }

    // Called by the request host after a call is served: when the call is held, registers the hold with the reply and
    // returns true (the request then waits); otherwise false (the reply is delivered at once).
    pub fn arm(&mut self, reply: Reply) -> bool {
        match self.pending_hold.take() {
            Some(mut h) => {
                if h.cont.is_none() {
                    h.pre = reply;
                }
                if h.abort_pre.is_none() {
                    if let Some(c) = h.abort_comp.take() {
                        h.abort_pre = Some((Vec::new(), vec![0], c));
                    }
                }
                self.register(h);
                true
            }
            None => false,
        }
    }

    fn register(&mut self, h: Hold) {
        if let (Some(w), true) = (h.win, h.abort_o.is_some()) {
            self.wins[w].hold = Some(h.label.clone());
        }
        self.holds.push(h);
    }

    // The scenario settles a label (ok or reject as the world entry says): the requests to wake with their replies.
    pub fn settle_label(&mut self, label: &str) -> Result<Vec<(i64, Reply)>, String> {
        let pos = self.holds.iter().position(|h| h.label == label).ok_or(format!("case: settle of an unknown label {}", label))?;
        let h = self.holds.remove(pos);
        self.resolve(h, false, -1)
    }

    fn resolve(&mut self, h: Hold, via: bool, who: i64) -> Result<Vec<(i64, Reply)>, String> {
        let a = if via {
            format!("{{\"label\":{},\"call\":{},\"via\":\"abort\"}}", jstr_s(&h.label), jstr_s(&h.call))
        } else {
            format!("{{\"label\":{},\"call\":{}}}", jstr_s(&h.label), jstr_s(&h.call))
        };
        let o = if via || h.rej { "{\"reject\":true}" } else { "{\"ok\":true}" };
        self.events.push(Ev { c: "settle".to_string(), a, o: Some(o.to_string()), who, ..Default::default() });
        if let Some(w) = h.win {
            self.wins[w].hold = None;
        }
        let po = if via { h.abort_o.clone() } else { h.norm_o.clone() };
        if let Some(po) = po {
            self.patch(h.pid, po);
        }
        let owner = h.owner;
        match h.cont {
            Some(c) => {
                let save = self.cur;
                self.cur = owner;
                let n0 = self.events.len();
                let r = c(self, via);
                for e in self.events[n0..].iter_mut() {
                    if e.who == 0 {
                        e.who = owner;
                    }
                }
                self.cur = save;
                match r? {
                    Step::Reply(r) => Ok(vec![(owner, r)]),
                    Step::Hold(nh) => {
                        self.register(nh);
                        Ok(Vec::new())
                    }
                }
            }
            None => {
                let r = if via { h.abort_pre.ok_or("internal: abort of a hold without an abort reply")? } else { h.pre };
                Ok(vec![(owner, r)])
            }
        }
    }

    // A timed() window of the current request: the cancel listener (none when there is no handle; not added when the handle
    // is already aborted) and the 20000 ms timer.
    fn win_open(&mut self) -> Result<usize, String> {
        let owner = self.cur;
        let handle = self.info()?.cancel;
        let mut lis = None;
        let mut aborted = false;
        if let Some(h) = &handle {
            let ha = self.handles.iter().find(|x| x.name == *h).map_or(false, |x| x.aborted);
            if ha {
                aborted = true;
            } else {
                self.listener_seq += 1;
                let id = self.listener_seq;
                if let Some(hd) = self.handles.iter_mut().find(|x| x.name == *h) {
                    hd.active += 1;
                }
                self.events.push(Ev { c: "listener.add".to_string(), a: format!("{{\"handle\":{},\"type\":\"abort\",\"once\":true,\"id\":{}}}", jstr_s(h), id), o: None, ..Default::default() });
                lis = Some((h.clone(), id));
            }
        }
        self.timer_seq += 1;
        let tid = self.timer_seq;
        let due = self.now + 20000;
        self.active_timers.push((tid, due));
        self.events.push(Ev { c: "timer.set".to_string(), a: format!("{{\"id\":{},\"delay\":20000,\"due\":{}}}", tid, due), o: None, ..Default::default() });
        let lis_active = lis.is_some();
        self.wins.push(Win { owner, tid, sig: 0, aborted, lis, lis_active, handle, closed: false, hold: None });
        Ok(self.wins.len() - 1)
    }

    // The window's timer clear and listener removal (once; later calls do nothing).
    fn win_close(&mut self, w: usize) {
        if self.wins[w].closed {
            return;
        }
        self.wins[w].closed = true;
        let tid = self.wins[w].tid;
        let live = self.active_timers.iter().any(|t| t.0 == tid);
        self.active_timers.retain(|t| t.0 != tid);
        self.events.push(Ev { c: "timer.clear".to_string(), a: format!("{{\"id\":{},\"active\":{}}}", tid, live), o: None, ..Default::default() });
        if let Some(h) = self.wins[w].handle.clone() {
            let (id, act) = match &self.wins[w].lis {
                Some((_, id)) => (id.to_string(), self.wins[w].lis_active),
                None => ("null".to_string(), false),
            };
            if act {
                if let Some(hd) = self.handles.iter_mut().find(|x| x.name == h) {
                    hd.active -= 1;
                }
                self.wins[w].lis_active = false;
            }
            self.events.push(Ev { c: "listener.remove".to_string(), a: format!("{{\"handle\":{},\"type\":\"abort\",\"id\":{},\"active\":{}}}", jstr_s(&h), id, act), o: None, ..Default::default() });
        }
    }

    // The signal number of the window, assigned at its first use in a trace line.
    fn win_sig(&mut self, w: usize) -> i64 {
        if self.wins[w].sig == 0 {
            self.sig_seq += 1;
            self.wins[w].sig = self.sig_seq;
        }
        self.wins[w].sig
    }

    // The window's signal aborts (timer fired or handle aborted): the abort line when the signal was ever named, then the
    // settlement of the hold bound to it (the entry's onAbort).
    fn win_abort(&mut self, w: usize, who: i64) -> Result<Vec<(i64, Reply)>, String> {
        self.wins[w].aborted = true;
        if self.wins[w].sig != 0 {
            let s = self.wins[w].sig;
            self.events.push(Ev { c: "abort".to_string(), a: format!("{{\"signal\":\"sig{}\",\"reason\":{{\"domAbort\":1}}}}", s), o: None, who, ..Default::default() });
        }
        if let Some(label) = self.wins[w].hold.take() {
            if let Some(pos) = self.holds.iter().position(|h| h.label == label) {
                let h = self.holds.remove(pos);
                return self.resolve(h, true, who);
            }
        }
        Ok(Vec::new())
    }

    // The next live window timer due at or before `target` fires (the scheduler order is (due, id)); None when none is due.
    pub fn fire_next(&mut self, target: i64) -> Result<Option<Vec<(i64, Reply)>>, String> {
        let mut best: Option<(i64, i64)> = None;
        for (id, due) in &self.active_timers {
            if *due <= target && best.map_or(true, |b| (*due, *id) < b) {
                best = Some((*due, *id));
            }
        }
        let (due, tid) = match best {
            None => return Ok(None),
            Some(b) => b,
        };
        self.now = self.now.max(due);
        self.active_timers.retain(|t| t.0 != tid);
        self.fired.push(tid);
        let w = self.wins.iter().position(|w| w.tid == tid && !w.closed).ok_or("case: only the timers of timed windows can fire")?;
        let who = self.wins[w].owner;
        self.events.push(Ev { c: "timer.fire".to_string(), a: format!("{{\"id\":{},\"at\":{}}}", tid, due), o: None, who, ..Default::default() });
        Ok(Some(self.win_abort(w, who)?))
    }

    // The abort of a cancel handle (the scenario's cancel event) with the optional reason reference.
    pub fn handle_cancel(&mut self, name: &str, reason_ref: Option<String>) -> Result<Vec<(i64, Reply)>, String> {
        self.handle_new(name);
        let idx = self.handles.iter().position(|h| h.name == name).unwrap();
        if self.handles[idx].aborted {
            return Ok(Vec::new());
        }
        self.handles[idx].aborted = true;
        self.reasons.insert(name.to_string(), reason_ref.clone());
        let r = match &reason_ref {
            Some(x) => format!("{{\"cancelReason\":{},\"ref\":{}}}", jstr_s(name), jstr_s(x)),
            None => format!("{{\"cancelReason\":{}}}", jstr_s(name)),
        };
        self.events.push(Ev { c: "abort".to_string(), a: format!("{{\"signal\":{},\"reason\":{}}}", jstr_s(name), r), o: None, who: -1, ..Default::default() });
        let mut out = Vec::new();
        let ws: Vec<usize> = (0..self.wins.len()).filter(|&w| !self.wins[w].closed && self.wins[w].lis_active && self.wins[w].handle.as_deref() == Some(name)).collect();
        for w in ws {
            self.wins[w].lis_active = false;
            if let Some(hd) = self.handles.iter_mut().find(|x| x.name == name) {
                hd.active -= 1;
            }
            out.extend(self.win_abort(w, -1)?);
        }
        Ok(out)
    }

    // The aborted fact of the current request's cancel handle as (flag, reason slot): the reason is an opaque reference.
    fn cancel_fact(&mut self) -> (i64, [i64; 3]) {
        let h = match self.info().ok().and_then(|i| i.cancel) {
            Some(h) => h,
            None => return (0, [0, 0, 0]),
        };
        if !self.handles.iter().any(|x| x.name == h && x.aborted) {
            return (0, [0, 0, 0]);
        }
        let id = self.h.refs.id(&format!("$cancel:{}", h));
        self.h.seen.insert(id);
        (1, [9, id, 0])
    }

    // The cancel reason of a `$cancel:<handle>` reference rendered as the oracle's value.
    fn cancel_value(&self, name: &str) -> Option<String> {
        let h = name.strip_prefix("$cancel:")?;
        Some(match self.reasons.get(h) {
            Some(Some(r)) => format!("{{\"cancelReason\":{},\"ref\":{}}}", jstr_s(h), jstr_s(r)),
            _ => format!("{{\"cancelReason\":{}}}", jstr_s(h)),
        })
    }
}

// The reference builds EngineImportError(code, detail) whose message is `code: detail` for a non-empty detail and `code`
// otherwise; the entry host's error observation (shared with L3-8) takes the scenario message as it is, so the L3-9
// leaf folds the code in before the spec is used.
fn parse_err_l(v: &Value) -> R<ErrSpec> {
    let mut s = parse_err(v)?;
    if s.class == Class::Eie {
        if let Some(Jv::Str(c)) = s.code.clone() {
            s.message = match s.message.take() {
                Some(Jv::Str(m)) if !m.is_empty() => {
                    let mut o = c.clone();
                    o.extend(u16s(": "));
                    o.extend(m);
                    Some(Jv::Str(o))
                }
                Some(Jv::Str(_)) | None => None,
                other => other,
            };
        }
    }
    Ok(s)
}

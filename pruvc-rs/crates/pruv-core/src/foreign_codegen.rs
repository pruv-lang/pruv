//! G6 A2 (docs/G6_SPEC.md section 12; counterpart of pruvc/foreign_codegen.py): Rust-source boundary helpers for the
//! standard catalog and for `trusted rust fn` declarations, the three check modes, and the `<out>.foreign.json` manifest.
//!
//! ForeignGen is owned by a codegen Gen (only created for a program that uses G6). Helpers are private Rust functions
//! named `pruv_g6_<op>__<enc>[__<enc>]` (catalog instance; <enc> = gtype::encode of each concrete, un-lowered type
//! argument), `pruv_g6_map_updated__<enc>__<enc>` (contract scan) and `NAME` (the helper of `trusted rust fn NAME`),
//! and are emitted once each, in first-use order, in front of the Pruv functions.
//!
//! The trusted-helper body needs the contract generator (`Gen::c`), so it lives in codegen.rs; this module only owns the
//! item table, the text skeletons and the manifest.

use crate::ast::Program;
use crate::codegen::{fmt_underscore, ALLOW_G6};
use crate::emit_common::{rstr, rty_g, show, try_conv_g, try_cty_g};
use crate::gtype::{encode, format_type, mk_option, GType};
use crate::modules::safe_type_cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};

type R<T> = Result<T, String>;

/// op families that do I/O: their helpers are not inlined
const IO_OP_PREFIXES: [&str; 4] = ["file_", "proc_", "ev_", "net_"];

/// calls that reach pruv_ud (the only reader of PRUV_CTX): pruv_ud itself, the budget charge, the contract tick,
/// the checked arithmetic / index helpers of codegen, and the contract accessors of the proc/net/loop runtimes
const UD_CALLS: [&str; 19] = [
    "pruv_ud(", "pruv_g6_charge(", "pruv_g9_tick(",
    "pruv_cidx(", "pruv_cadd(", "pruv_csub(", "pruv_cmul(", "pruv_cneg(", "pruv_cdiv(", "pruv_crem(",
    "pruv_cidx_u8(", "pruv_cidx_bool(", "pruv_cidx_str(", "pruv_cload_be(",
    "pruv_cproc_buf(", "pruv_cproc_eof(", "pruv_cproc_byte(", "pruv_cnet_byte(", "pruv_cev_slot(",
];

fn budget_line(n: u64) -> String {
    format!("let mut pruv_budget: i64 = {};", fmt_underscore(n))
}

fn idx_err() -> String {
    "list index out of range".to_string()
}

pub struct ForeignGen {
    /// requires | all | off
    pub mode: String,
    /// requires checks are emitted (every mode but off)
    pub req: bool,
    /// ensures checks are emitted (mode all)
    pub ens: bool,
    budget: u64,
    /// helper name -> text (insertion order = first use)
    items: Vec<(String, String)>,
    /// manifest: executable catalog instances in first-use order: (op, type_args)
    catalog: Vec<(String, Vec<String>)>,
    /// trusted fn names called from an emitted function
    pub called: HashSet<String>,
    op: String,
    /// G4b-2: package of the function being emitted (None = the program package); set by Gen::func
    pub cur_pkg: Option<String>,
    /// G4b-2: package name ("" = program package) -> every (op, type_args) catalog_helper was asked for (not only first use)
    pkg_catalog: BTreeMap<String, BTreeSet<(String, Vec<String>)>>,
}

impl ForeignGen {
    pub fn new(mode: &str, budget: u64) -> ForeignGen {
        ForeignGen {
            mode: mode.to_string(),
            req: mode != "off",
            ens: mode == "all",
            budget,
            items: Vec::new(),
            catalog: Vec::new(),
            called: HashSet::new(),
            op: String::new(),
            cur_pkg: None,
            pkg_catalog: BTreeMap::new(),
        }
    }

    /// G4b-2: per-package catalog entries, keyed by package name ("" = program package), sorted and deduplicated.
    pub fn pkg_catalog(&self) -> &BTreeMap<String, BTreeSet<(String, Vec<String>)>> {
        &self.pkg_catalog
    }

    fn record_pkg_catalog(&mut self, op: &str, targs: Vec<String>) {
        let key = self.cur_pkg.clone().unwrap_or_default();
        self.pkg_catalog.entry(key).or_default().insert((op.to_string(), targs));
    }

    // ------------------------------------------------------------------ results
    pub fn helper_texts(&self) -> Vec<String> {
        self.items.iter().map(|(_, t)| t.clone()).collect()
    }

    /// G6 A3: (helper name, text) of every emitted helper, in first-use order
    pub fn items(&self) -> &[(String, String)] {
        &self.items
    }

    pub fn has_item(&self, name: &str) -> bool {
        self.items.iter().any(|(n, _)| n == name)
    }

    pub fn set_op(&mut self, op: &str) {
        self.op = op.to_string();
    }

    pub fn add_item(&mut self, name: &str, text: String) {
        self.items.push((name.to_string(), text));
    }

    pub fn manifest(&self, prog: &Program, externs: &[(String, String)]) -> String {
        let mut trusted: Vec<J> = Vec::new();
        for (fi, f) in prog.functions.iter().enumerate() {
            let info = match &f.foreign {
                Some(i) => i,
                None => continue,
            };
            // G4 A2: explicit mode lists the source-qualified identity (crate::m::f), never the encoded stem
            let disp: &str = match &prog.explicit {
                Some(x) => &x.fns[fi].disp,
                None => &f.name,
            };
            trusted.push(J::O(vec![
                ("name".to_string(), J::s(disp)),
                ("path".to_string(), J::s(&info.path)),
                ("line".to_string(), J::I(crate::diagnostic::plain_line(f.line))),
                ("col".to_string(), J::I(f.col)),
                ("effects".to_string(), J::A(f.effects.iter().map(|e| J::s(e)).collect())),
                ("requires".to_string(), J::A(f.requires.iter().map(|r| J::Str(show(r))).collect())),
                ("ensures".to_string(), J::A(f.ensures.iter().map(|x| J::Str(show(x))).collect())),
                ("reachable".to_string(), J::B(self.called.contains(&f.name))),
            ]));
        }
        let catalog: Vec<J> = self
            .catalog
            .iter()
            .map(|(op, targs)| {
                J::O(vec![
                    ("op".to_string(), J::s(op)),
                    ("type_args".to_string(), J::A(targs.iter().map(|t| J::s(t)).collect())),
                ])
            })
            .collect();
        let ext: Vec<J> = externs
            .iter()
            .map(|(n, p)| J::O(vec![("name".to_string(), J::s(n)), ("path".to_string(), J::s(p))]))
            .collect();
        let doc = J::O(vec![
            ("schema".to_string(), J::s("pruv-foreign-v1")),
            ("backend".to_string(), J::s("rust")),
            ("check_mode".to_string(), J::s(&self.mode)),
            ("rustc_checked".to_string(), J::B(false)),
            ("trusted".to_string(), J::A(trusted)),
            ("catalog".to_string(), J::A(catalog)),
            ("externs".to_string(), J::A(ext)),
        ]);
        let mut out = String::new();
        dump_json(&doc, 0, &mut out);
        out.push('\n');
        out
    }

    // ------------------------------------------------------------------ helper skeleton
    /// text of `fn name(params) -> ret { guard; budget; body }`; body: lines (already without indentation)
    fn fn_text(&self, name: &str, params: &str, ret: Option<&str>, body: &[String]) -> String {
        let mut l: Vec<String> = Vec::new();
        l.push(ALLOW_G6.to_string());
        // pure data accessors are inlined into the caller (P10H); I/O families keep a real call
        if !IO_OP_PREFIXES.iter().any(|p| self.op.starts_with(p)) {
            l.push("#[inline(always)]".to_string());
        }
        let arrow = match ret {
            None => String::new(),
            Some(r) => format!(" -> {}", r),
        };
        l.push(format!("fn {}({}){} {{", name, params, arrow));
        // the ctx guard (two thread-local writes) only serves pruv_ud, so emit it only when pruv_ud is reachable (P10H)
        if self.req && (self.ens || body.iter().any(|x| UD_CALLS.iter().any(|c| x.contains(c)))) {
            l.push(format!("    let pruv_g = pruv_ctx({});", rstr(&self.op)));
        }
        if self.ens {
            l.push(format!("    {}", budget_line(self.budget)));
        }
        for x in body {
            l.push(format!("    {}", x));
        }
        l.push("}".to_string());
        format!("{}\n", l.join("\n"))
    }

    /// the helper of a trusted fn: the guard and the budget are shared by requires and ensures of one call
    /// `disp`: the name the boundary guard reports (G4 A2: the source identity; the helper itself keeps its stem `name`)
    pub fn fn_trusted(&self, name: &str, params: &str, ret: Option<&str>, body: &[String], disp: &str, checks: bool) -> String {
        let mut l: Vec<String> = Vec::new();
        l.push(ALLOW_G6.to_string());
        // inlined into its caller: one host call is one call, not a shim call plus the host call (P10H)
        l.push("#[inline(always)]".to_string());
        let arrow = match ret {
            None => String::new(),
            Some(r) => format!(" -> {}", r),
        };
        l.push(format!("fn {}({}){} {{", name, params, arrow));
        // the ctx guard (two thread-local writes) and the budget only serve contract checks (P10H)
        if self.req && checks {
            l.push(format!("    let pruv_g = pruv_ctx({});", rstr(disp)));
            l.push(format!("    {}", budget_line(self.budget)));
        }
        for x in body {
            l.push(format!("    {}", x));
        }
        l.push("}".to_string());
        format!("{}\n", l.join("\n"))
    }

    fn req_line(&self, cond: &str, text: &str) -> Vec<String> {
        if self.req {
            vec![format!("if !({}) {{ pruv_g6_req({}, {}); }}", cond, rstr(&self.op), rstr(text))]
        } else {
            Vec::new()
        }
    }

    fn ens_line(&self, cond: &str, text: &str) -> Vec<String> {
        if self.ens {
            vec![format!("if !({}) {{ pruv_g6_ens({}, {}); }}", cond, rstr(&self.op), rstr(text))]
        } else {
            Vec::new()
        }
    }

    /// snapshot of a consumed input (mode all): charge its elements, then an independent clone
    fn snap(&self, ty: &str, src: &str) -> Vec<String> {
        if !self.ens {
            return Vec::new();
        }
        vec![
            format!("pruv_g6_charge(&mut pruv_budget, {}.len());", src),
            format!("let pruv_snap: {} = {}.clone();", ty, src),
        ]
    }

    // ------------------------------------------------------------------ catalog
    pub fn catalog_helper(&mut self, op: &str, low: &[GType], src: &[GType]) -> R<String> {
        let src: &[GType] = if src.is_empty() && !low.is_empty() { low } else { src };
        let mut name = format!("pruv_g6_{}", op);
        for t in src {
            name.push_str("__");
            name.push_str(&encode(&safe_type_cow(t))?); // G4: nominal identities use their [0-9A-Za-z_] encoding
        }
        self.record_pkg_catalog(op, src.iter().map(format_type).collect());
        if !self.has_item(&name) {
            self.op = op.to_string();
            let text = self.catalog_text(op, &name, low)?;
            self.items.push((name.clone(), text));
            self.catalog.push((op.to_string(), src.iter().map(format_type).collect()));
        }
        Ok(name)
    }

    fn async_text(&self, op:&str, name:&str)->R<String> {
        let (args,ret,req,expr,ens)=match op {
            "ev_new"=>("cap: i64","pruv_Loop","cap >= 1 && cap <= 4096","pruv_ev_new(cap)","pruv_ev_wf(&pruv_r) && pruv_ev_count(&pruv_r)==0"),
            "ev_close"=>("l: pruv_Loop","::std::result::Result<i64, i64>","","pruv_ev_close(l)","match pruv_r {Ok(n)=>n>=0,Err(e)=>(1..=8).contains(&e)}"),
            "ev_add_socket"=>("l: pruv_Loop, s: pruv_Socket","pruv_Loop","l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && s.error == 0","pruv_ev_socket(l, s)",""),
            "ev_add_listener"=>("l: pruv_Loop, x: pruv_Listener","pruv_Loop","l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && x.error == 0","pruv_ev_listener(l, x)",""),
            "ev_add_process"=>("l: pruv_Loop, p: pruv_Process","pruv_Loop","l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && p.error == 0","pruv_ev_process(l, p)",""),
            "ev_add_timer"=>("l: pruv_Loop, ms: i64","pruv_Loop","l.error == 0 && pruv_ev_count(&l) < l.slots.len() as i64 && ms >= 0 && ms <= 86400000","pruv_ev_timer(l, ms)",""),
            "ev_wait"=>("l: pruv_Loop, timeout_ms: i64","pruv_Loop","l.error == 0 && timeout_ms >= -1 && timeout_ms <= 86400000","pruv_ev_wait(l, timeout_ms)",""),
            "ev_read"=>("l: pruv_Loop, slot: i64, max: i64","pruv_Loop","pruv_ev_valid(&l, slot, 1) && max > 0 && max <= (1i64 << 40)","pruv_ev_read(l, slot, 1, max, false)",""),
            "ev_proc_read"=>("l: pruv_Loop, slot: i64, stream: i64, max: i64","pruv_Loop","pruv_ev_valid(&l, slot, 3) && max > 0 && max <= (1i64 << 40) && (stream == 1 || stream == 2)","pruv_ev_read(l, slot, stream, max, true)",""),
            "ev_write"=>("l: pruv_Loop, slot: i64, data: &[u8]","pruv_Loop","pruv_ev_valid(&l, slot, 1) && l.public[slot as usize].open","pruv_ev_write(l, slot, data, 0)",""),
            "ev_shutdown_write"=>("l: pruv_Loop, slot: i64","pruv_Loop","pruv_ev_valid(&l, slot, 1) && l.public[slot as usize].open","pruv_ev_write(l, slot, &[], 1)",""),
            "ev_proc_write"=>("l: pruv_Loop, slot: i64, data: &[u8]","pruv_Loop","pruv_ev_valid(&l, slot, 3) && l.public[slot as usize].open","pruv_ev_write(l, slot, data, 2)",""),
            "ev_proc_close_stdin"=>("l: pruv_Loop, slot: i64","pruv_Loop","pruv_ev_valid(&l, slot, 3) && l.public[slot as usize].open","pruv_ev_write(l, slot, &[], 3)",""),
            "ev_accept"=>("l: pruv_Loop, slot: i64","pruv_Loop","pruv_ev_valid(&l, slot, 2) && pruv_ev_count(&l) < l.slots.len() as i64","pruv_ev_accept(l, slot)",""),
            "ev_proc_wait"=>("l: pruv_Loop, slot: i64","pruv_Loop","pruv_ev_valid(&l, slot, 3)","pruv_ev_remove(l, slot, true)",""),
            "ev_proc_wait_for"=>("l: pruv_Loop, slot: i64, timeout_ms: i64","pruv_Loop","pruv_ev_valid(&l, slot, 3) && timeout_ms >= 0 && timeout_ms <= 86400000","pruv_ev_wait_for(l, slot, timeout_ms)",""),
            "ev_proc_write_for"=>("l: pruv_Loop, slot: i64, data: &[u8], timeout_ms: i64","pruv_Loop","pruv_ev_valid(&l, slot, 3) && l.public[slot as usize].open && timeout_ms >= 0 && timeout_ms <= 86400000","pruv_ev_write_for(l, slot, data, timeout_ms)",""),
            "ev_remove"=>("l: pruv_Loop, slot: i64","pruv_Loop","pruv_ev_valid(&l, slot, 0) && l.public[slot as usize].kind != 0","pruv_ev_remove(l, slot, false)",""),
            "ev_ok"=>("l: &pruv_Loop","bool","","l.error == 0",""),
            "ev_error"=>("l: &pruv_Loop","i64","","l.error",""),
            "ev_cap"=>("l: &pruv_Loop","i64","","l.public.len() as i64",""),
            "ev_count"=>("l: &pruv_Loop","i64","","l.public.iter().filter(|p|p.kind!=0).count() as i64",""),
            "ev_last"=>("l: &pruv_Loop","i64","","l.last",""),
            "ev_event_slot"=>("l: &pruv_Loop","i64","","l.event_slot",""),
            "ev_event_kind"=>("l: &pruv_Loop","i64","","l.event_kind",""),
            "ev_size"=>("l: &pruv_Loop","i64","","l.buf.len() as i64",""),
            "ev_exit"=>("l: &pruv_Loop","i64","","l.exit",""),
            "ev_bytes"=>("l: &pruv_Loop","::std::vec::Vec<u8>","","l.buf.clone()",""),
            "ev_kind"=>("l: &pruv_Loop, slot: i64","i64","slot >= 0 && slot < l.public.len() as i64","l.public[pruv_cev_slot(l, slot as i128)].kind",""),
            "ev_eof"=>("l: &pruv_Loop, slot: i64","bool","slot >= 0 && slot < l.public.len() as i64","l.public[pruv_cev_slot(l, slot as i128)].eof",""),
            "ev_write_open"=>("l: &pruv_Loop, slot: i64","bool","slot >= 0 && slot < l.public.len() as i64","l.public[pruv_cev_slot(l, slot as i128)].open",""),
            "ev_byte"=>("l: &pruv_Loop, i: i64","i64","i >= 0 && i < l.buf.len() as i64","if i >= 0 && i < l.buf.len() as i64 { l.buf[i as usize] as i64 } else { pruv_ud(\"loop byte out of range\") }",""),
            _=>return Err(format!("unknown async operation {}",op)),
        };
        let mut body=if req.is_empty(){Vec::new()}else{self.req_line(req,req)};
        if self.ens && ret=="pruv_Loop" && op!="ev_new" {
            body.push("pruv_g6_charge(&mut pruv_budget, l.public.len() + l.buf.len());".to_string());
            body.push("let pruv_snap = pruv_ev_snapshot(&l);".to_string());
        }
        body.push(format!("let pruv_r: {} = {};",ret,expr));
        if ret=="pruv_Loop" || !ens.is_empty() {
            body.extend(self.ens_line(if ens.is_empty() {"pruv_ev_wf(&pruv_r)"}else{ens},if ens.is_empty() {"well-formed loop"}else{ens}));
        }
        body.push("pruv_r".to_string());
        if self.ens && ret=="pruv_Loop" && op!="ev_new" {
            let slot=if args.contains("slot:") {"slot"}else{"-1"};
            body.pop();
            body.extend(self.ens_line(&format!("pruv_ev_post(\"{}\", &pruv_snap, &pruv_r, {})",op,slot),"public state frame"));
            body.push("pruv_r".to_string());
        }
        Ok(self.fn_text(name,args,Some(ret),&body))
    }
    fn catalog_text(&self, op: &str, name: &str, low: &[GType]) -> R<String> {
        if op.starts_with("file_") {
            return Ok(self.file_text(op, name));
        }
        if op.starts_with("proc_") {
            return self.proc_text(op, name);
        }
        if op.starts_with("ev_") { return self.async_text(op, name); }
        if op.starts_with("net_") {
            return self.net_text(op, name);
        }
        const STRING: &str = "::std::string::String";
        let g = |i: usize| low.get(i).ok_or_else(idx_err);
        match op {
            "vec_new" => {
                let t = rty_g(g(0)?, false);
                let v = rty_g(&GType::new("vec", vec![g(0)?.clone()]), false);
                let mut body = vec![format!("let pruv_r: {} = ::std::vec::Vec::<{}>::new();", v, t)];
                body.extend(self.ens_line("pruv_r.len() == 0", "len(result) == 0"));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, "", Some(v.as_str()), &body))
            }
            "vec_push" => {
                let t = rty_g(g(0)?, false);
                let v = rty_g(&GType::new("vec", vec![g(0)?.clone()]), false);
                let mut body = self.req_line("(v.len() as u64) < (1u64 << 40)", "len(v) < 2^40");
                body.extend(self.snap(&v, "v"));
                body.push("v.push(x);".to_string());
                body.extend(self.ens_line("v.len() == pruv_snap.len() + 1", "len(result) == len(v) + 1"));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, pruv_snap.len());".to_string());
                }
                body.extend(self.ens_line("v[..pruv_snap.len()] == pruv_snap[..]", "result[..len(v)] == v"));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, 1);".to_string()); // one visited element (G6_SPEC 12.3)
                }
                body.extend(self.ens_line("v[pruv_snap.len()] == x", "result[len(v)] == x"));
                body.push("v".to_string());
                Ok(self.fn_text(name, &format!("mut v: {}, x: {}", v, t), Some(v.as_str()), &body))
            }
            "vec_get" => {
                let t = rty_g(g(0)?, false);
                let v = rty_g(&GType::new("vec", vec![g(0)?.clone()]), false);
                let mut body = self.req_line("i >= 0 && (i as u64) < (v.len() as u64)", "0 <= i && i < len(v)");
                body.push(
                    "let pruv_k: usize = if i >= 0 && (i as u64) < (v.len() as u64) { i as usize } else { pruv_g6_repr(\"index\") };"
                        .to_string(),
                );
                body.push(format!("let pruv_r: {} = v[pruv_k];", t));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, 1);".to_string()); // one visited element (G6_SPEC 12.3)
                }
                body.extend(self.ens_line("v.get(pruv_k).copied() == Some(pruv_r)", "result == v[i]"));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, &format!("v: &{}, i: i64", v), Some(t.as_str()), &body))
            }
            "map_new" => {
                let k = rty_g(g(0)?, false);
                let t = rty_g(g(1)?, false);
                let m = rty_g(&GType::new("hashmap", vec![g(0)?.clone(), g(1)?.clone()]), false);
                let mut body = vec![format!("let pruv_r: {} = ::std::collections::HashMap::<{}, {}>::new();", m, k, t)];
                body.extend(self.ens_line("pruv_r.len() == 0", "len(result) == 0"));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, "", Some(m.as_str()), &body))
            }
            "map_get" => {
                let k = rty_g(g(0)?, false);
                let m = rty_g(&GType::new("hashmap", vec![g(0)?.clone(), g(1)?.clone()]), false);
                let opt = rty_g(&mk_option(g(1)?.clone()), false);
                let mut body = vec![format!("let pruv_r: {} = m.get(&k).copied();", opt)];
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, 1);".to_string()); // one visited element (G6_SPEC 12.3)
                }
                body.extend(self.ens_line(
                    "m.contains_key(&k) == pruv_r.is_some() && m.get(&k).copied() == pruv_r",
                    "result == map_lookup(m, k)",
                ));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, &format!("m: &{}, k: {}", m, k), Some(opt.as_str()), &body))
            }
            "map_insert" => {
                let k = rty_g(g(0)?, false);
                let t = rty_g(g(1)?, false);
                let m = rty_g(&GType::new("hashmap", vec![g(0)?.clone(), g(1)?.clone()]), false);
                let mut body = self.req_line("m.contains_key(&k) || (m.len() as u64) < (1u64 << 40)", "k in m || len(m) < 2^40");
                body.extend(self.snap(&m, "m"));
                body.push("let _ = m.insert(k, v);".to_string());
                body.extend(self.ens_line(
                    "m.len() == pruv_snap.len() + (if pruv_snap.contains_key(&k) { 0 } else { 1 })",
                    "len(result) == len(m) + (k absent ? 1 : 0)",
                ));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, 1);".to_string()); // one visited element (G6_SPEC 12.3)
                }
                body.extend(self.ens_line("m.get(&k) == Some(&v)", "result[k] == v"));
                if self.ens {
                    body.push("for (pruv_sk, pruv_sv) in pruv_snap.iter() {".to_string());
                    body.push("    pruv_g6_charge(&mut pruv_budget, 1);".to_string());
                    body.push(format!(
                        "    if *pruv_sk != k && m.get(pruv_sk) != Some(pruv_sv) {{ pruv_g6_ens({}, \"entries other than k are unchanged\"); }}",
                        rstr(op)
                    ));
                    body.push("}".to_string());
                }
                body.push("m".to_string());
                Ok(self.fn_text(name, &format!("mut m: {}, k: {}, v: {}", m, k, t), Some(m.as_str()), &body))
            }
            "string_new" => {
                let mut body = vec![format!("let pruv_r: {} = ::std::string::String::new();", STRING)];
                body.extend(self.ens_line("pruv_r.len() == 0", "len(result) == 0"));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, "", Some(STRING), &body))
            }
            "string_push_str" => {
                let mut body =
                    self.req_line("(s.len() as u64) + (text.len() as u64) <= (1u64 << 40)", "len(s) + len(text) <= 2^40");
                body.extend(self.snap(STRING, "s"));
                body.push("s.push_str(text);".to_string());
                body.extend(self.ens_line("s.len() == pruv_snap.len() + text.len()", "len(result) == len(s) + len(text)"));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, pruv_snap.len());".to_string());
                }
                body.extend(self.ens_line(
                    "s.as_bytes()[..pruv_snap.len()] == pruv_snap.as_bytes()[..]",
                    "result[..len(s)] == s",
                ));
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, text.len());".to_string());
                }
                body.extend(self.ens_line("s.as_bytes()[pruv_snap.len()..] == text.as_bytes()[..]", "result[len(s)..] == text"));
                body.push("s".to_string());
                Ok(self.fn_text(name, &format!("mut s: {}, text: &str", STRING), Some(STRING), &body))
            }
            "string_byte" => {
                let mut body = self.req_line("i >= 0 && (i as u64) < (s.len() as u64)", "0 <= i && i < len(s)");
                body.push(
                    "let pruv_k: usize = if i >= 0 && (i as u64) < (s.len() as u64) { i as usize } else { pruv_g6_repr(\"index\") };"
                        .to_string(),
                );
                body.push("let pruv_r: i64 = s.as_bytes()[pruv_k] as i64;".to_string());
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, 1);".to_string()); // one visited element (G6_SPEC 12.3)
                }
                body.extend(self.ens_line(
                    "s.as_bytes().get(pruv_k).map(|pruv_b| *pruv_b as i64) == Some(pruv_r)",
                    "result == s[i]",
                ));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, &format!("s: &{}, i: i64", STRING), Some("i64"), &body))
            }
            "string_to_str" => {
                let mut body: Vec<String> = Vec::new();
                if self.ens {
                    body.push("pruv_g6_charge(&mut pruv_budget, s.len());".to_string());
                }
                body.push(format!("let pruv_r: {} = s.clone();", STRING));
                body.extend(self.ens_line("pruv_r.as_bytes() == s.as_bytes()", "bytes of result equal bytes of s"));
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, &format!("s: &{}", STRING), Some(STRING), &body))
            }
            other => Err(format!("codegen: unknown catalog operation {}", other)),
        }
    }

    /// G10: I/O failures stay in the owned handle.
    fn file_text(&self, op: &str, name: &str) -> String {
        if matches!(op, "file_ok" | "file_readable" | "file_writable") {
            return self.fn_text(name, "f: &pruv_File", Some("bool"), &[format!("f.{}", &op[5..])]);
        }
        if op == "file_size" {
            let mut body = vec!["let pruv_r = f.buf.len() as i64;".to_string()];
            body.extend(self.ens_line("pruv_r >= 0", "result >= 0"));
            body.push("pruv_r".to_string());
            return self.fn_text(name, "f: &pruv_File", Some("i64"), &body);
        }
        if op == "file_byte" {
            let cond = "i >= 0 && (i as u64) < (f.buf.len() as u64)";
            let mut body = self.req_line(cond, "0 <= i && i < file_size(f)");
            body.push(format!("let pruv_k = if {} {{ i as usize }} else {{ pruv_g6_repr(\"index\") }};", cond));
            body.push("let pruv_r = f.buf[pruv_k] as i64;".to_string());
            body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255"));
            body.push("pruv_r".to_string());
            return self.fn_text(name, "f: &pruv_File, i: i64", Some("i64"), &body);
        }
        let read;
        let params;
        let mut body;
        if matches!(op, "file_open_read" | "file_create") {
            read = op == "file_open_read";
            body = vec![
                format!("let file = ::std::fs::File::{}(path).ok();", if read { "open" } else { "create" }),
                "let ok = file.is_some();".to_string(),
                format!("let f = pruv_File {{ file, ok, readable: {}, writable: {}, buf: ::std::vec::Vec::new() }};",
                    if read { "ok" } else { "false" }, if read { "false" } else { "ok" }),
            ];
            params = "path: &str".to_string();
        } else if op == "file_close" {
            body = vec![
                "let pruv_ok = f.ok;".to_string(),
                "let pruv_r = f.ok && match f.file.as_mut() {".to_string(),
                "    Some(file) => ::std::io::Write::flush(file).is_ok(), None => false".to_string(),
                "};".to_string(),
            ];
            body.extend(self.ens_line("!pruv_r || pruv_ok", "result ==> file_ok(f)"));
            body.push("pruv_r".to_string());
            return self.fn_text(name, "mut f: pruv_File", Some("bool"), &body);
        } else {
            read = op == "file_read_all";
            let field = if read { "readable" } else { "writable" };
            body = self.req_line(&format!("f.{}", field), &format!("file_{}(f)", field));
            let mut ps = "mut f: pruv_File".to_string();
            let action;
            if read {
                body.push("f.buf.clear();".to_string());
                action = "::std::io::Read::read_to_end(file, &mut f.buf).is_ok()".to_string();
            } else {
                ps.push_str(if op == "file_write_str" { ", text: &str" } else { ", data: &[u8]" });
                let data = if op == "file_write_str" { "text.as_bytes()" } else { "data" };
                action = format!("::std::io::Write::write_all(file, {}).is_ok()", data);
            }
            params = ps;
            body.extend([
                format!("let pruv_ok = f.ok && f.{} && match f.file.as_mut() {{", field),
                format!("    Some(file) => {}, None => false", action),
                "};".to_string(),
                "if !pruv_ok { f.ok = false; f.file = None; f.buf.clear(); }".to_string(),
                format!("f.readable = {}; f.writable = {};", if read { "f.ok" } else { "false" }, if read { "false" } else { "f.ok" }),
            ]);
        }
        if read {
            body.extend(self.ens_line("!f.writable && f.readable == f.ok", "!file_writable(result) && file_readable(result) == file_ok(result)"));
            if op == "file_read_all" {
                body.extend(self.ens_line("if f.ok { (f.buf.len() as i64) >= 0 } else { f.buf.is_empty() }",
                    "file_ok(result) ? file_size(result) >= 0 : file_size(result) == 0"));
            } else {
                body.extend(self.ens_line("f.buf.is_empty()", "file_size(result) == 0"));
            }
        } else {
            body.extend(self.ens_line("!f.readable && f.writable == f.ok && f.buf.is_empty()",
                "!file_readable(result) && file_writable(result) == file_ok(result) && file_size(result) == 0"));
        }
        body.push("f".to_string());
        self.fn_text(name, &params, Some("pruv_File"), &body)
    }

    /// G10 proc A2: thin helpers over the Process runtime (codegen.rs PROC_PRELUDE). Failure stays in the owned handle;
    /// runtime requires follow --foreign-checks, the ensures (mode all) re-check the public state of the result.
    fn proc_text(&self, op: &str, name: &str) -> R<String> {
        const P: &str = "pruv_Process";
        const A: &str = "pruv_ProcessArgs";
        const RES: &str = "::std::result::Result<i64, i64>";
        const VEC: &str = "::std::vec::Vec<u8>";
        let with = |mut body: Vec<String>, last: &str| -> Vec<String> {
            body.push(last.to_string());
            body
        };
        match op {
            "proc_args_new" => {
                let mut body = vec![format!("let pruv_r: {} = pruv_proc_args_new();", A)];
                body.extend(self.ens_line("pruv_r.args.len() == 0", "proc_args_len(result) == 0"));
                Ok(self.fn_text(name, "", Some(A), &with(body, "pruv_r")))
            }
            "proc_args_push" => {
                let mut body = self.req_line("(a.args.len() as u64) < (1u64 << 40)", "proc_args_len(a) < 2^40");
                if self.ens {
                    body.push("let pruv_n: usize = a.args.len();".to_string());
                }
                body.push(format!("let pruv_r: {} = pruv_proc_args_push(a, arg);", A));
                body.extend(self.ens_line("pruv_r.args.len() == pruv_n + 1", "proc_args_len(result) == proc_args_len(a) + 1"));
                Ok(self.fn_text(name, &format!("a: {}, arg: &str", A), Some(A), &with(body, "pruv_r")))
            }
            "proc_args_len" => {
                let mut body = vec!["let pruv_r: i64 = a.args.len() as i64;".to_string()];
                body.extend(self.ens_line("pruv_r >= 0 && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40"));
                Ok(self.fn_text(name, &format!("a: &{}", A), Some("i64"), &with(body, "pruv_r")))
            }
            "proc_spawn" => {
                let mut body = vec![format!("let pruv_r: {} = pruv_proc_spawn(program, args, cwd);", P)];
                body.extend(self.ens_line(
                    "if pruv_r.error == 0 { pruv_r.stdin_open } else { (pruv_r.error == 1 || pruv_r.error == 2) && pruv_proc_wf(&pruv_r) }",
                    "ok(result) && open(result) || !ok(result) && !open(result) && (err(result) == 1 || err(result) == 2)",
                ));
                body.extend(self.ens_line(
                    "pruv_r.stdout_buf.is_empty() && pruv_r.stderr_buf.is_empty() && !pruv_r.stdout_eof && !pruv_r.stderr_eof",
                    "both public buffers empty and both EOFs false",
                ));
                Ok(self.fn_text(name, &format!("program: &str, args: &{}, cwd: &str", A), Some(P), &with(body, "pruv_r")))
            }
            "proc_spawn_env" => {
                let mut body = vec![format!("let pruv_r: {} = pruv_proc_spawn_env(program, args, cwd, env_names, env_values);", P)];
                body.extend(self.ens_line(
                    "if pruv_r.error == 0 { pruv_r.stdin_open } else { (pruv_r.error == 1 || pruv_r.error == 2) && pruv_proc_wf(&pruv_r) }",
                    "ok(result) && open(result) || !ok(result) && !open(result) && (err(result) == 1 || err(result) == 2)",
                ));
                body.extend(self.ens_line(
                    "pruv_r.stdout_buf.is_empty() && pruv_r.stderr_buf.is_empty() && !pruv_r.stdout_eof && !pruv_r.stderr_eof",
                    "both public buffers empty and both EOFs false",
                ));
                Ok(self.fn_text(name, &format!("program: &str, args: &{}, cwd: &str, env_names: &{}, env_values: &{}", A, A, A), Some(P), &with(body, "pruv_r")))
            }
            "proc_write_str" | "proc_write_bytes" => {
                let text = op == "proc_write_str";
                let mut body = self.req_line("p.error == 0 && p.stdin_open", "proc_ok(p) && proc_stdin_open(p)");
                body.push(format!(
                    "let pruv_r: {} = pruv_proc_write(p, {});",
                    P,
                    if text { "text.as_bytes()" } else { "data" }
                ));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 || pruv_r.error == 4) && pruv_proc_wf(&pruv_r)",
                    "ok(result) || err(result) == 4",
                ));
                Ok(self.fn_text(
                    name,
                    &format!("p: {}, {}", P, if text { "text: &str" } else { "data: &[u8]" }),
                    Some(P),
                    &with(body, "pruv_r"),
                ))
            }
            "proc_close_stdin" => {
                let mut body = self.req_line("p.error == 0 && p.stdin_open", "proc_ok(p) && proc_stdin_open(p)");
                body.push(format!("let pruv_r: {} = pruv_proc_close(p);", P));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 && !pruv_r.stdin_open) || (pruv_r.error == 4 && pruv_proc_wf(&pruv_r))",
                    "ok(result) && !open(result) || err(result) == 4",
                ));
                Ok(self.fn_text(name, &format!("p: {}", P), Some(P), &with(body, "pruv_r")))
            }
            "proc_read" => {
                let mut body = self.req_line("p.error == 0", "proc_ok(p)");
                body.extend(self.req_line("stream == 1 || stream == 2", "stream == 1 || stream == 2"));
                body.extend(self.req_line("0 < max && max <= (1i64 << 40)", "0 < max && max <= 2^40"));
                body.push(format!("let pruv_r: {} = pruv_proc_read(p, stream, max);", P));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 || pruv_r.error == 3) && pruv_proc_wf(&pruv_r)",
                    "ok(result) || err(result) == 3",
                ));
                body.extend(self.ens_line(
                    "pruv_r.error != 0 || { let pruv_s: usize = if stream == 1 { pruv_r.stdout_buf.len() } else { pruv_r.stderr_buf.len() }; let pruv_e: bool = if stream == 1 { pruv_r.stdout_eof } else { pruv_r.stderr_eof }; (pruv_s as i64) <= max && pruv_e == (pruv_s == 0) }",
                    "size(result, stream) <= max && eof(result, stream) == (size(result, stream) == 0)",
                ));
                Ok(self.fn_text(name, &format!("p: {}, stream: i64, max: i64", P), Some(P), &with(body, "pruv_r")))
            }
            "proc_read_all" => {
                let mut body = self.req_line("p.error == 0 && !p.stdin_open", "proc_ok(p) && !proc_stdin_open(p)");
                body.push(format!("let pruv_r: {} = pruv_proc_read_all(p);", P));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 && !pruv_r.stdin_open && pruv_r.stdout_eof && pruv_r.stderr_eof) || (pruv_r.error == 3 && pruv_proc_wf(&pruv_r))",
                    "ok(result) && both EOFs true || err(result) == 3",
                ));
                Ok(self.fn_text(name, &format!("p: {}", P), Some(P), &with(body, "pruv_r")))
            }
            "proc_bytes" => {
                let mut body = self.req_line("stream == 1 || stream == 2", "stream == 1 || stream == 2");
                body.push(format!("let pruv_r: {} = pruv_proc_buf(p, stream).clone();", VEC));
                body.extend(self.ens_line("pruv_r[..] == pruv_proc_buf(p, stream)[..]", "result is the public buffer of the stream"));
                Ok(self.fn_text(name, &format!("p: &{}, stream: i64", P), Some(VEC), &with(body, "pruv_r")))
            }
            "proc_wait" | "proc_kill" => {
                let mut body = vec![format!(
                    "let pruv_r: {} = pruv_proc_finish(p, {});",
                    RES,
                    if op == "proc_kill" { "true" } else { "false" }
                )];
                body.extend(self.ens_line(
                    "match pruv_r { Ok(_) => true, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 6 }",
                    "Err carries an error code 1..6",
                ));
                Ok(self.fn_text(name, &format!("p: {}", P), Some(RES), &with(body, "pruv_r")))
            }
            "proc_ok" => Ok(self.fn_text(name, &format!("p: &{}", P), Some("bool"), &["p.error == 0".to_string()])),
            "proc_stdin_open" => Ok(self.fn_text(name, &format!("p: &{}", P), Some("bool"), &["p.stdin_open".to_string()])),
            "proc_error" => {
                let mut body = vec!["let pruv_r: i64 = p.error;".to_string()];
                body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 6", "0 <= result && result <= 6"));
                Ok(self.fn_text(name, &format!("p: &{}", P), Some("i64"), &with(body, "pruv_r")))
            }
            "proc_eof" => {
                let mut body = self.req_line("stream == 1 || stream == 2", "stream == 1 || stream == 2");
                body.push(
                    "let pruv_r: bool = if stream == 1 { p.stdout_eof } else if stream == 2 { p.stderr_eof } else { pruv_g6_repr(\"stream\") };"
                        .to_string(),
                );
                body.push("pruv_r".to_string());
                Ok(self.fn_text(name, &format!("p: &{}, stream: i64", P), Some("bool"), &body))
            }
            "proc_size" => {
                let mut body = self.req_line("stream == 1 || stream == 2", "stream == 1 || stream == 2");
                body.push("let pruv_r: i64 = pruv_proc_buf(p, stream).len() as i64;".to_string());
                body.extend(self.ens_line("0 <= pruv_r && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40"));
                Ok(self.fn_text(name, &format!("p: &{}, stream: i64", P), Some("i64"), &with(body, "pruv_r")))
            }
            "proc_byte" => {
                let cond = "(stream == 1 || stream == 2) && i >= 0 && (i as u64) < (pruv_proc_buf(p, stream).len() as u64)";
                let mut body = self.req_line(cond, "valid stream and 0 <= i && i < proc_size(p, stream)");
                body.push("let pruv_b: &::std::vec::Vec<u8> = pruv_proc_buf(p, stream);".to_string());
                body.push(
                    "let pruv_k: usize = if i >= 0 && (i as u64) < (pruv_b.len() as u64) { i as usize } else { pruv_g6_repr(\"index\") };"
                        .to_string(),
                );
                body.push("let pruv_r: i64 = pruv_b[pruv_k] as i64;".to_string());
                body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255"));
                Ok(self.fn_text(name, &format!("p: &{}, stream: i64, i: i64", P), Some("i64"), &with(body, "pruv_r")))
            }
            other => Err(format!("codegen: unknown catalog operation {}", other)),
        }
    }

    /// G10 net A2: thin helpers over the Socket / Listener runtime (codegen.rs NET_PRELUDE). Failure stays in the owned
    /// handle; runtime requires follow --foreign-checks, the ensures (mode all) re-check the public state of the result.
    fn net_text(&self, op: &str, name: &str) -> R<String> {
        const S: &str = "pruv_Socket";
        const L: &str = "pruv_Listener";
        const RES: &str = "::std::result::Result<i64, i64>";
        const VEC: &str = "::std::vec::Vec<u8>";
        let with = |mut body: Vec<String>, last: &str| -> Vec<String> {
            body.push(last.to_string());
            body
        };
        // mode all: the entry state the preservation ensures compare against
        let snap: Vec<String> = if self.ens {
            vec![
                "pruv_g6_charge(&mut pruv_budget, s.buf.len());".to_string(),
                "let pruv_sb: ::std::vec::Vec<u8> = s.buf.clone();".to_string(),
                "let pruv_se: bool = s.eof;".to_string(),
                "let pruv_sw: bool = s.write_open;".to_string(),
            ]
        } else {
            Vec::new()
        };
        let keep = "pruv_r.eof == pruv_se && pruv_r.buf == pruv_sb";
        // the reads replace the buffer: only the flags are compared, no buffer copy is charged
        let snap_flags: Vec<String> = snap.iter().skip(2).cloned().collect();
        match op {
            "net_listen" => {
                let mut body = self.req_line("port >= 0 && port <= 65535", "0 <= port && port <= 65535");
                body.push(format!("let pruv_r: {} = pruv_net_listen(host, port);", L));
                body.extend(self.ens_line(
                    "pruv_net_lwf(&pruv_r)",
                    "lok(result) && 1 <= port(result) && port(result) <= 65535 || !lok(result) && port(result) == 0",
                ));
                Ok(self.fn_text(name, "host: &str, port: i64", Some(L), &with(body, "pruv_r")))
            }
            "net_accept" => {
                let mut body = self.req_line("l.error == 0", "lok(l)");
                body.push(format!("let pruv_r: {} = pruv_net_accept(l);", S));
                body.extend(self.ens_line(
                    "(if pruv_r.error == 0 { pruv_r.write_open } else { pruv_r.error == 5 }) && pruv_net_wf(&pruv_r) && pruv_r.buf.is_empty() && !pruv_r.eof",
                    "ok(result) && open(result) || err(result) == 5, empty buffer, EOF false",
                ));
                Ok(self.fn_text(name, &format!("l: &{}", L), Some(S), &with(body, "pruv_r")))
            }
            "net_listener_close" => {
                let mut body = vec![format!("let pruv_r: {} = pruv_net_lclose(l);", RES)];
                body.extend(self.ens_line(
                    "match pruv_r { Ok(pruv_v) => pruv_v == 0, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 2 }",
                    "Ok(0) or Err carries a listener error code 1..2",
                ));
                Ok(self.fn_text(name, &format!("l: {}", L), Some(RES), &with(body, "pruv_r")))
            }
            "net_connect" => {
                let mut body = self.req_line("port >= 1 && port <= 65535", "1 <= port && port <= 65535");
                body.push(format!("let pruv_r: {} = pruv_net_connect(host, port);", S));
                body.extend(self.ens_line(
                    "(if pruv_r.error == 0 { pruv_r.write_open } else { pruv_r.error == 1 || pruv_r.error == 2 }) && pruv_net_wf(&pruv_r) && pruv_r.buf.is_empty() && !pruv_r.eof",
                    "ok(result) && open(result) || err(result) == 1 || err(result) == 2, empty buffer, EOF false",
                ));
                Ok(self.fn_text(name, "host: &str, port: i64", Some(S), &with(body, "pruv_r")))
            }
            "net_write_str" | "net_write_bytes" => {
                let text = op == "net_write_str";
                let mut body = self.req_line("s.error == 0 && s.write_open", "ok(s) && open(s)");
                body.extend(snap.iter().cloned());
                body.push(format!(
                    "let pruv_r: {} = pruv_net_write(s, {});",
                    S,
                    if text { "text.as_bytes()" } else { "data" }
                ));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 || pruv_r.error == 4 || pruv_r.error == 6) && pruv_net_wf(&pruv_r)",
                    "ok(result) || err(result) == 4 || err(result) == 6",
                ));
                body.extend(self.ens_line(
                    &format!("pruv_r.error != 0 || (pruv_r.write_open == pruv_sw && {})", keep),
                    "write-open, buffer and EOF preserved",
                ));
                Ok(self.fn_text(
                    name,
                    &format!("s: {}, {}", S, if text { "text: &str" } else { "data: &[u8]" }),
                    Some(S),
                    &with(body, "pruv_r"),
                ))
            }
            "net_shutdown_write" => {
                let mut body = self.req_line("s.error == 0 && s.write_open", "ok(s) && open(s)");
                body.extend(snap.iter().cloned());
                body.push(format!("let pruv_r: {} = pruv_net_shutdown(s);", S));
                body.extend(self.ens_line(
                    "!pruv_r.write_open && (pruv_r.error == 0 || (pruv_r.error == 4 && pruv_net_wf(&pruv_r)))",
                    "!open(result) && (ok(result) || err(result) == 4)",
                ));
                body.extend(self.ens_line(&format!("pruv_r.error != 0 || ({})", keep), "buffer and EOF preserved"));
                Ok(self.fn_text(name, &format!("s: {}", S), Some(S), &with(body, "pruv_r")))
            }
            "net_set_timeout" => {
                let mut body = self.req_line("s.error == 0", "ok(s)");
                body.extend(self.req_line("ms >= 0 && ms <= 86400000", "0 <= ms && ms <= 86400000"));
                body.extend(snap.iter().cloned());
                body.push(format!("let pruv_r: {} = pruv_net_set_timeout(s, ms);", S));
                body.extend(self.ens_line(
                    "pruv_r.error == 0 || (pruv_r.error == 4 && pruv_net_wf(&pruv_r))",
                    "ok(result) || err(result) == 4",
                ));
                body.extend(self.ens_line(
                    &format!("pruv_r.error != 0 || (pruv_r.write_open == pruv_sw && {})", keep),
                    "write-open, buffer and EOF preserved",
                ));
                Ok(self.fn_text(name, &format!("s: {}, ms: i64", S), Some(S), &with(body, "pruv_r")))
            }
            "net_read" => {
                let mut body = self.req_line("s.error == 0", "ok(s)");
                body.extend(self.req_line("0 < max && max <= (1i64 << 40)", "0 < max && max <= 2^40"));
                body.extend(snap_flags.iter().cloned());
                body.push(format!("let pruv_r: {} = pruv_net_read(s, max);", S));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 || pruv_r.error == 3 || pruv_r.error == 6) && pruv_net_wf(&pruv_r)",
                    "ok(result) || err(result) == 3 || err(result) == 6",
                ));
                body.extend(self.ens_line(
                    "pruv_r.error != 0 || ((pruv_r.buf.len() as u64) <= (max as u64) && pruv_r.eof == pruv_r.buf.is_empty() && pruv_r.write_open == pruv_sw && (!pruv_se || pruv_r.buf.is_empty()))",
                    "size(result) <= max && eof(result) == (size(result) == 0), write-open preserved",
                ));
                Ok(self.fn_text(name, &format!("s: {}, max: i64", S), Some(S), &with(body, "pruv_r")))
            }
            "net_read_all" => {
                let mut body = self.req_line("s.error == 0", "ok(s)");
                body.extend(snap_flags.iter().cloned());
                body.push(format!("let pruv_r: {} = pruv_net_read_all(s);", S));
                body.extend(self.ens_line(
                    "(pruv_r.error == 0 && pruv_r.eof && pruv_r.write_open == pruv_sw) || ((pruv_r.error == 3 || pruv_r.error == 6) && pruv_net_wf(&pruv_r))",
                    "ok(result) && eof(result) || err(result) == 3 || err(result) == 6",
                ));
                Ok(self.fn_text(name, &format!("s: {}", S), Some(S), &with(body, "pruv_r")))
            }
            "net_bytes" => {
                let mut body = vec![format!("let pruv_r: {} = s.buf.clone();", VEC)];
                body.extend(self.ens_line("pruv_r[..] == s.buf[..]", "result is the public buffer"));
                Ok(self.fn_text(name, &format!("s: &{}", S), Some(VEC), &with(body, "pruv_r")))
            }
            "net_close" => {
                let mut body = vec![format!("let pruv_r: {} = pruv_net_close(s);", RES)];
                body.extend(self.ens_line(
                    "match pruv_r { Ok(pruv_v) => pruv_v == 0, Err(pruv_e) => pruv_e >= 1 && pruv_e <= 6 }",
                    "Ok(0) or Err carries an error code 1..6",
                ));
                Ok(self.fn_text(name, &format!("s: {}", S), Some(RES), &with(body, "pruv_r")))
            }
            "net_ok" => Ok(self.fn_text(name, &format!("s: &{}", S), Some("bool"), &["s.error == 0".to_string()])),
            "net_write_open" => Ok(self.fn_text(name, &format!("s: &{}", S), Some("bool"), &["s.write_open".to_string()])),
            "net_eof" => Ok(self.fn_text(name, &format!("s: &{}", S), Some("bool"), &["s.eof".to_string()])),
            "net_error" => {
                let mut body = vec!["let pruv_r: i64 = s.error;".to_string()];
                body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 6", "0 <= result && result <= 6"));
                Ok(self.fn_text(name, &format!("s: &{}", S), Some("i64"), &with(body, "pruv_r")))
            }
            "net_size" => {
                let mut body = vec!["let pruv_r: i64 = s.buf.len() as i64;".to_string()];
                body.extend(self.ens_line("0 <= pruv_r && (pruv_r as u64) <= (1u64 << 40)", "0 <= result && result <= 2^40"));
                Ok(self.fn_text(name, &format!("s: &{}", S), Some("i64"), &with(body, "pruv_r")))
            }
            "net_byte" => {
                let cond = "i >= 0 && (i as u64) < (s.buf.len() as u64)";
                let mut body = self.req_line(cond, "0 <= i && i < net_size(s)");
                body.push(format!("let pruv_k: usize = if {} {{ i as usize }} else {{ pruv_g6_repr(\"index\") }};", cond));
                body.push("let pruv_r: i64 = s.buf[pruv_k] as i64;".to_string());
                body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 255", "0 <= result && result <= 255"));
                Ok(self.fn_text(name, &format!("s: &{}, i: i64", S), Some("i64"), &with(body, "pruv_r")))
            }
            "net_listener_ok" => Ok(self.fn_text(name, &format!("l: &{}", L), Some("bool"), &["l.error == 0".to_string()])),
            "net_listener_error" => {
                let mut body = vec!["let pruv_r: i64 = l.error;".to_string()];
                body.extend(self.ens_line("0 <= pruv_r && pruv_r <= 2", "0 <= result && result <= 2"));
                Ok(self.fn_text(name, &format!("l: &{}", L), Some("i64"), &with(body, "pruv_r")))
            }
            "net_listener_port" => {
                let mut body = vec!["let pruv_r: i64 = l.port;".to_string()];
                body.extend(self.ens_line(
                    "(l.error == 0 && pruv_r >= 1 && pruv_r <= 65535) || (l.error != 0 && pruv_r == 0)",
                    "ok implies 1 <= result && result <= 65535; failed implies result == 0",
                ));
                Ok(self.fn_text(name, &format!("l: &{}", L), Some("i64"), &with(body, "pruv_r")))
            }
            other => Err(format!("codegen: unknown catalog operation {}", other)),
        }
    }

    // ------------------------------------------------------------------ contract-only scan (requires)
    pub fn map_updated_helper(&mut self, kt: &GType, vt: &GType, src: &[GType]) -> R<String> {
        let mut name = String::from("pruv_g6_map_updated");
        for t in src {
            name.push_str("__");
            name.push_str(&encode(&safe_type_cow(t))?); // G4: nominal identities use their [0-9A-Za-z_] encoding
        }
        // G4b-2: every request counts for the package (the global catalog below only keeps the first use)
        self.record_pkg_catalog("map_updated", src.iter().map(format_type).collect());
        if self.has_item(&name) {
            return Ok(name);
        }
        self.op = "map_updated".to_string();
        let m = rty_g(&GType::new("hashmap", vec![kt.clone(), vt.clone()]), false);
        let k = rty_g(kt, false);
        let keyconv = if *kt == GType::i64() { "i64::try_from(pruv_kk).ok()" } else { "Some(pruv_kk)" };
        let l: Vec<String> = vec![
            ALLOW_G6.to_string(),
            format!(
                "fn {}(pruv_after: &{}, pruv_before: &{}, pruv_kk: {}, pruv_vv: {}, pruv_budget: &mut i64) -> bool {{",
                name,
                m,
                m,
                try_cty_g(kt)?,
                try_cty_g(vt)?
            ),
            format!("    let pruv_nk: {} = match {} {{ Some(pruv_x) => pruv_x, None => return false }};", k, keyconv),
            "    let pruv_was: bool = pruv_before.contains_key(&pruv_nk);".to_string(),
            "    if pruv_after.len() != pruv_before.len() + (if pruv_was { 0 } else { 1 }) { return false; }".to_string(),
            format!(
                "    match pruv_after.get(&pruv_nk) {{ Some(pruv_a) => {{ if !({} == pruv_vv) {{ return false; }} }} None => {{ return false; }} }}",
                try_conv_g(vt, "(*pruv_a)")?
            ),
            "    for (pruv_bk, pruv_bv) in pruv_before.iter() {".to_string(),
            "        *pruv_budget -= 1; if *pruv_budget < 0 { pruv_ud(\"contract iteration budget exceeded\"); }".to_string(),
            "        if *pruv_bk != pruv_nk {".to_string(),
            format!(
                "            match pruv_after.get(pruv_bk) {{ Some(pruv_av) => {{ if !({} == {}) {{ return false; }} }} None => {{ return false; }} }}",
                try_conv_g(vt, "(*pruv_av)")?,
                try_conv_g(vt, "(*pruv_bv)")?
            ),
            "        }".to_string(),
            "    }".to_string(),
            "    true".to_string(),
            "}".to_string(),
        ];
        self.items.push((name.clone(), format!("{}\n", l.join("\n"))));
        self.catalog.push(("map_updated".to_string(), src.iter().map(format_type).collect()));
        Ok(name)
    }
}

// ---------------------------------------------------------------------- manifest JSON
// Python json.dumps(doc, sort_keys=True, indent=2, ensure_ascii=False)

pub enum J {
    Str(String),
    I(i64),
    B(bool),
    A(Vec<J>),
    O(Vec<(String, J)>),
}

impl J {
    pub fn s(x: &str) -> J {
        J::Str(x.to_string())
    }
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn dump_json(j: &J, level: usize, out: &mut String) {
    match j {
        J::Str(s) => json_str(out, s),
        J::I(n) => out.push_str(&n.to_string()),
        J::B(b) => out.push_str(if *b { "true" } else { "false" }),
        J::A(xs) => {
            if xs.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&"  ".repeat(level + 1));
                dump_json(x, level + 1, out);
            }
            out.push('\n');
            out.push_str(&"  ".repeat(level));
            out.push(']');
        }
        J::O(es) => {
            if es.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut sorted: Vec<&(String, J)> = es.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push_str("{\n");
            for (i, (k, v)) in sorted.into_iter().map(|e| (&e.0, &e.1)).enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&"  ".repeat(level + 1));
                json_str(out, k);
                out.push_str(": ");
                dump_json(v, level + 1, out);
            }
            out.push('\n');
            out.push_str(&"  ".repeat(level));
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_layout() {
        let doc = J::O(vec![
            ("b".to_string(), J::A(vec![])),
            ("a".to_string(), J::A(vec![J::s("x\"é"), J::B(true), J::I(3)])),
        ]);
        let mut out = String::new();
        dump_json(&doc, 0, &mut out);
        assert_eq!(out, "{\n  \"a\": [\n    \"x\\\"é\",\n    true,\n    3\n  ],\n  \"b\": []\n}");
    }
}

//! pruv-proof: a crate porting the z3 layer of the Python `pruvc/verify.py`.
//!
//! Layout (same structure as verify.py):
//! - `sys`        : a small wrapper over z3-sys. **unsafe lives only in this module** (see `deny` below).
//! - `model`      : `Opt`/`Res` Datatype, user struct/enum `Model`, `_MODEL_SEQ`
//! - `symbolic`   : `State`/`Arr`/`FnVerifier`: fresh, wf, havoc, aggregate lemmas, expression `ev`, domain `dom`, statement `exec`, `run`
//! - `obligation` : `prove` (pc → guard → (lemmas) → Not(ob), timeout 8000ms, seen key), counterexample `cex`/`fmt_val`, trace
//! - `samples`    : builders hand-porting the obligations of the 6 sample .pruv files (0B prototype; used by `pruv-proof-spike`)
//!
//! The public entry point is [`verify_program`]. For the same input, the SMT-LIB text passed to z3 per obligation, the verdicts and the error JSON
//! must be byte-identical to Python (SPEC §6).
//!
//! Why use sys directly instead of the high-level `z3` crate: the SMT text must match z3py character for character
//! (SPEC §6), and the high-level crate may differ from z3py in quantifier weight/qid, how Datatypes are created, etc.
//! The `z3` dependency is kept only to pull in z3-sys/z3-src via `vendored`.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod sys;

pub mod infer;
pub mod model;
pub mod obligation;
pub mod proof_cache;
pub mod samples;
pub mod symbolic;

use pruv_core::ast::Program;
use pruv_core::diagnostic::PruvError;
use pruv_core::typeinfo::TypeTable;

use crate::model::{decls_from_ast, next_seq, Model, Prelude};
use crate::obligation::Trace;
use crate::symbolic::FnVerifier;
use crate::sys::Z3;

/// Full version string of the running libz3 (counterpart of Python `z3.get_full_version()`).
pub fn z3_full_version() -> String {
    sys::full_version()
}

/// verify.py `verify_program(prog)` (line 864). Returns the proof errors of a program that passed checking, in function/obligation order.
///
/// - Python creates one `Model` for the whole program (incrementing `_MODEL_SEQ` once) shared by all functions. Here
///   the sequence number is taken once, and for each function the `Model` is rebuilt in a fresh z3 context with the same number (the expression text is identical).
/// - `tt` is unused: Python also builds a fresh `TypeTable(prog)` inside `Model(prog)` (the same table the checker built).
/// - `rec`: adapter trace recording (`Trace::adapter()`). If None, nothing is recorded.
/// - A Python exception (internal failure) becomes `Err(message)` — errors gathered so far are discarded (Python likewise returns no value on an exception).
pub fn verify_program(prog: &Program, tt: &TypeTable, rec: Option<&mut Trace>) -> Result<Vec<PruvError>, String> {
    verify_program_opts(prog, tt, rec, &VerifyOptions::default(), None)
}

/// `verify_program` with an optional proof cache (G4b-3).
pub fn verify_program_cached(
    prog: &Program,
    tt: &TypeTable,
    rec: Option<&mut Trace>,
    cache: Option<std::rc::Rc<proof_cache::ProofCache>>,
) -> Result<Vec<PruvError>, String> {
    verify_program_opts(prog, tt, rec, &VerifyOptions { infer: false, proof_cache: cache, ..Default::default() }, None)
}

/// STAGE1 B2 options.
#[derive(Clone, Debug, Default)]
pub struct VerifyOptions {
    /// `--infer`: per function, run template Houdini invariant inference (infer.rs) and verify together with the surviving candidates.
    pub infer: bool,
    /// `--proof-cache DIR` (G4b-3). None: no extra work.
    pub proof_cache: Option<std::rc::Rc<proof_cache::ProofCache>>,
    /// PR1 `--module-cache DIR` (docs/G4B_SPEC.md 7.8): skip and record whole modules. None: no extra work.
    pub modules: Option<std::rc::Rc<proof_cache::ModulePlan>>,
}

/// verify.py `verify_program(prog, rec, infer, report)`. `infer=false` keeps the existing path unchanged.
/// If `report` is Some, per-loop report records (docs/STAGE1_B_INFER.md §5) are pushed in function order, then loop number order
/// (after finishing each function: even if a later function fails internally, earlier functions' records remain).
/// In inference mode a fresh z3 context (+Prelude, Model with the same seq) is created per function, per search round and per final verification.
pub fn verify_program_opts(
    prog: &Program,
    _tt: &TypeTable,
    rec: Option<&mut Trace>,
    opts: &VerifyOptions,
    mut report: Option<&mut Vec<pruv_core::pyjson::Json>>,
) -> Result<Vec<PruvError>, String> {
    // G3 A2: the verifier handles generics (no guard here); the code generators and the driver still reject them.
    // The structured `[T]` is never a G2 hit (the plain `[i64]` is not either), so "array" needs no skip entry.
    if let Some(e) = pruv_core::typeinfo::g2_unimplemented_error(prog, "verify", None, &["array_u8", "array_u16", "array_bool", "str"], None) { return Ok(vec![e]); } // G2 B1
    // PR1: module records apply only without a trace and without an inference report (as in Python).
    // A plan is used once: consumed here even when trace/report turn records off, so a later call cannot reuse its keys.
    let plan = opts.modules.as_deref().filter(|p| !p.spent.replace(true));
    let plan = if rec.is_none() && report.is_none() { plan } else { None };
    let mut local = Trace::off();
    let trace: &mut Trace = match rec {
        Some(t) => t,
        None => &mut local,
    };
    // G4b-3: the cache travels with the trace so run_attempt can reach it.
    trace.cache = opts.proof_cache.clone();
    let decls = decls_from_ast(&prog.types);
    let seq = next_seq();
    {
        // Python builds the Model first regardless of the function count: a model construction failure occurs even with no functions.
        let z = Z3::new();
        let pre = Prelude::new(&z);
        Model::for_program(&z, &pre, &decls, prog, seq)?;
    }
    let mut errors: Vec<PruvError> = Vec::new();
    // One verification of a function (plain, or inference + final verification) with the given LG31 loop frame clause
    // candidates. Returns (errors, inference report records, failed frame candidates (id, (code, status))); nothing is
    // appended to the caller's lists. An empty `frame` is exactly the old code.
    let attempt = |f: &pruv_core::ast::Function,
                   frame: &symbolic::Extra,
                   trace: &mut Trace,
                   want_recs: bool|
     -> Result<(Vec<PruvError>, Vec<pruv_core::pyjson::Json>, Vec<(String, (String, String))>), String> {
        let mut errs: Vec<PruvError> = Vec::new();
        let mut recs: Vec<pruv_core::pyjson::Json> = Vec::new();
        let mut failed: Vec<(String, (String, String))> = Vec::new();
        if opts.infer {
            let res = infer::infer_function(prog, f, &decls, seq, &mut *trace, frame)?;
            if res.status == infer::InferStatus::Timeout {
                errs.push(PruvError {
                    stage: "verify".to_string(),
                    line: f.line,
                    col: f.col,
                    code: "inference_timeout".to_string(),
                    message: format!(
                        "invariant inference for {} hit the time safety limit (5 s per function, 500 ms per query)",
                        f.name
                    ),
                    fix: "write the loop invariants explicitly, or compile without --infer".to_string(),
                    counterexample: None,
                });
            } else {
                let z = Z3::new();
                let pre = Prelude::new(&z);
                let model = Model::for_program(&z, &pre, &decls, prog, seq)?;
                let mut fv = FnVerifier::for_function(&z, &pre, &model, &mut *trace, prog, f);
                fv.set_extra(res.extra());
                if !frame.is_empty() {
                    fv.set_frame(frame.clone());
                }
                fv.run()?;
                errs.append(&mut fv.errors);
                failed = std::mem::take(&mut fv.frame_failed);
            }
            if want_recs {
                recs = res.records(errs.len());
            }
            return Ok((errs, recs, failed));
        }
        // A fresh context per function (a non-rc context never frees ASTs, so it is dropped wholesale when the function ends).
        // Counterexample model values may differ from Python (the SMT text and verdicts are the same). Suspected cause: z3py uses an rc context, so AST ids are reused
        // and the chosen model differs (switching to one context per program left the counterexample difference unchanged, RESULTS §18).
        let z = Z3::new();
        let pre = Prelude::new(&z);
        let model = Model::for_program(&z, &pre, &decls, prog, seq)?;
        let mut fv = FnVerifier::for_function(&z, &pre, &model, &mut *trace, prog, f);
        if !frame.is_empty() {
            fv.set_frame(frame.clone());
        }
        fv.run()?;
        errs.append(&mut fv.errors);
        failed = std::mem::take(&mut fv.frame_failed);
        Ok((errs, recs, failed))
    };
    // LG31: the function is verified with its automatic loop frame clauses; a frame candidate whose entry/preservation
    // obligation fails reports nothing, is removed (the others keep their ids) and the function is verified again, until
    // no frame candidate fails. With no (more) candidates this is exactly the old path.
    let one = |f: &pruv_core::ast::Function,
                   errors: &mut Vec<PruvError>,
                   trace: &mut Trace,
                   report: &mut Option<&mut Vec<pruv_core::pyjson::Json>>|
     -> Result<(), String> {
        // A round with frame clauses is kept only if it proves every clause and reports no error; otherwise its trace
        // entries are rewound, and with no failed clause (another error, an inference timeout) the old path decides.
        let mut frame = infer::frame_candidates(prog, f);
        loop {
            if frame.is_empty() {
                let (mut errs, recs, _) = attempt(f, &frame, &mut *trace, report.is_some())?;
                errors.append(&mut errs);
                if let Some(rep) = report.as_mut() {
                    rep.extend(recs);
                }
                return Ok(());
            }
            let mark = trace.obligations.len();
            let (errs, recs, failed) = attempt(f, &frame, &mut *trace, report.is_some())?;
            if failed.is_empty() && errs.is_empty() {
                if let Some(rep) = report.as_mut() {
                    rep.extend(recs);
                }
                return Ok(());
            }
            trace.obligations.truncate(mark);
            if failed.is_empty() {
                frame = std::collections::HashMap::new();
                continue;
            }
            let dead: std::collections::HashSet<&str> = failed.iter().map(|(id, _)| id.as_str()).collect();
            let mut next: symbolic::Extra = std::collections::HashMap::new();
            for (n, cands) in &frame {
                let kept: Vec<symbolic::ExtraInv> =
                    cands.iter().filter(|c| !dead.contains(c.0.as_str())).cloned().collect();
                if !kept.is_empty() {
                    next.insert(*n, std::rc::Rc::new(kept));
                }
            }
            frame = next;
        }
    };
    // a plan is used once (`ModulePlan.spent`, consumed above): it was built for one program
    let Some(plan) = plan.filter(|p| p.fn_module.len() == prog.functions.len()) else {
        for f in &prog.functions {
            one(f, &mut errors, &mut *trace, &mut report)?;
        }
        return Ok(errors);
    };
    // per module: (key, record found, errors added by its functions); filled on the module's first function
    let mut state: Vec<Option<(Option<String>, bool, usize)>> = vec![None; plan.key_texts.len()];
    for (i, f) in prog.functions.iter().enumerate() {
        let mi = plan.fn_module[i];
        let st = state[mi].get_or_insert_with(|| {
            let k = plan.recs.key(&plan.key_texts[mi]);
            let hit = plan.recs.lookup(k.as_deref());
            (k, hit, 0)
        });
        if st.1 {
            continue;
        }
        let before = errors.len();
        one(f, &mut errors, &mut *trace, &mut report)?;
        st.2 += errors.len() - before;
    }
    for (k, hit, nerr) in state.into_iter().flatten() {
        if !hit && nerr == 0 {
            plan.recs.store(k.as_deref());
        }
    }
    Ok(errors)
}

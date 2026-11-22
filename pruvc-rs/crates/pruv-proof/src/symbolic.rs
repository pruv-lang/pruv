//! Symbolic execution of verify.py: `Arr`, `State`, `FnVerifier` (fresh, wf, rebuild, havoc, aggregates, expression `ev`, domain `dom`,
//! statement `exec`, loops, `run`).
//!
//! Function structure, call order, `s.k` increment order, z3 expression construction order and lemma accumulation order are kept the same as verify.py.
//! Line numbers in comments refer to verify.py. Obligations (`prove`), counterexamples and trace are in obligation.rs.
//!
//! In Python, `ev(e, st, g, code, env=None)` reads the same dict via `env = st.env` while appending to `st.pc`.
//! In Rust, `ev` takes `env: &Env` (a snapshot cloned on entry) separately from `st: &mut State`. Since `ev` does not modify env,
//! the semantics are the same. The counterexample (`cex`) reads `st.env`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use pruv_core::ast::{self as ast, Expr, Stmt};
use pruv_core::diagnostic::PruvError;
use pruv_core::gtype::{self, GType, Mapping};

use crate::model::{Coll, Elem, Model, NomKind, Prelude, Ty};
use crate::obligation::Trace;
use crate::sys::{Ast, FuncDecl, Sort, Z3};

/// (line, col), 1-based. The line is the AST line number as is, i.e. with the G4 file tag in bits 32.. (tag 0 in legacy mode
/// and for the prototype literals). Use `pos_line` for the plain per-file line; the `seen` key uses the plain line (Python's
/// key has no file either).
pub type Pos = (u64, u32);

/// A `Pos` from a (possibly file-tagged) AST line number.
fn mk_pos(line: i64, col: i64) -> Pos {
    (line.max(0) as u64, col as u32)
}

/// The plain per-file line of a `Pos` (the value printed everywhere).
pub fn pos_line(p: Pos) -> u32 {
    pruv_core::diagnostic::plain_line(p.0 as i64) as u32
}

/// Internal failures (Python exceptions) are propagated as strings.
pub type R<T> = Result<T, String>;

/// verify.py line 7.
pub const MAX_LEN: i64 = 1 << 40;
/// verify.py line 8.
pub const TIMEOUT_MS: u32 = 8000;

/// verify.py `QueryConfig`: recording boundary of one solver query (phase, round, resource limits).
#[derive(Clone, Debug)]
pub struct QueryConfig {
    /// "verification" | "inference"
    pub phase: String,
    pub round: Option<u32>,
    pub timeout_ms: u32,
    pub rlimit: Option<u32>,
    pub seed: Option<u32>,
}

impl Default for QueryConfig {
    fn default() -> Self {
        QueryConfig { phase: "verification".to_string(), round: None, timeout_ms: TIMEOUT_MS, rlimit: None, seed: None }
    }
}

/// STAGE1 B2: one candidate that inference adds to a loop = (candidate ID, expr, string). An item of verify.py `extra`.
pub type ExtraInv = (String, Expr, String);
/// STAGE1 B2: loop number → that loop's candidates (sorted order). If empty, the existing path.
pub type Extra = HashMap<usize, std::rc::Rc<Vec<ExtraInv>>>;

/// verify.py `BudgetExhausted` / `InferenceTimeout`. Python escapes from deep inside `prove` via an exception, but Rust's
/// `prove` returns no value, so it is flagged in `Probe.abort`, later obligations pass without query or recording, and `exec_block` stops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeAbort {
    Budget,
    Timeout,
}

/// verify.py `Probe`: search state of one function (shared by all rounds).
pub struct Probe {
    pub max_queries: usize,
    pub deadline: Instant,
    /// Number of attempts so far (budget counter)
    pub queries: usize,
    /// Candidates that failed this round {ID: (obligation code, attempt status)}
    pub failed: HashMap<String, (String, String)>,
    pub abort: Option<ProbeAbort>,
}

impl Probe {
    pub fn new(max_queries: usize, deadline: Instant) -> Probe {
        Probe { max_queries, deadline, queries: 0, failed: HashMap::new(), abort: None }
    }
    pub fn begin_round(&mut self) {
        self.failed.clear();
    }
    /// verify.py `Probe.tick`: before an attempt, budget → wall clock → count. If exceeded, sets abort and returns false.
    pub fn tick(&mut self) -> bool {
        if self.queries >= self.max_queries {
            self.abort = Some(ProbeAbort::Budget);
            return false;
        }
        if Instant::now() > self.deadline {
            self.abort = Some(ProbeAbort::Timeout);
            return false;
        }
        self.queries += 1;
        true
    }
}

/// verify.py `walk_loops`: (While, the block containing it, index within the block), in pre-order (statement order, a While before its body,
/// If then → els, Match arm order). Loop number = position in this sequence.
pub fn walk_loops(stmts: &[Stmt]) -> Vec<(&ast::While, &[Stmt], usize)> {
    fn go<'s>(stmts: &'s [Stmt], out: &mut Vec<(&'s ast::While, &'s [Stmt], usize)>) {
        for (i, st) in stmts.iter().enumerate() {
            match st {
                Stmt::ForMap(w) => go(&w.body, out),
                Stmt::While(w) => {
                    out.push((w, stmts, i));
                    go(&w.body, out);
                }
                Stmt::If(x) => {
                    go(&x.then, out);
                    go(&x.els, out);
                }
                Stmt::Match(m) => {
                    for a in &m.arms {
                        go(&a.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    go(stmts, &mut out);
    out
}

/// One invariant item of `loop_`: (candidate ID or None, expr, candidate string or None).
type InvRef<'x> = (Option<&'x str>, &'x Expr, Option<&'x str>);

/// Wording inside verify.py `loop`: (domain what, obligation what).
fn inv_what(cid: Option<&str>, text: Option<&str>, phase: &str) -> (String, String) {
    match cid {
        None => (
            format!("variant_field lookups in loop invariant ({}) are defined", phase),
            if phase == "entry" { "loop invariant on entry".to_string() } else { "loop invariant preserved".to_string() },
        ),
        Some(_) => {
            let t = text.unwrap_or("");
            (
                format!("partial operations in inferred loop invariant `{}` ({}) are defined", t, phase),
                if phase == "entry" {
                    format!("inferred loop invariant `{}` on entry", t)
                } else {
                    format!("inferred loop invariant `{}` preserved", t)
                },
            )
        }
    }
}

/// G6b: the catalog type Vec<i64> (no nominal owner).
fn is_vec_i64(t: &Ty) -> bool {
    let g = t.to_gtype();
    g.owner.is_none() && g == GType::new("vec", vec![GType::i64()])
}

/// verify.py `in_range` (line 168). `x >= I64_MIN` is a comparison with a Python int, so it is not flipped (raw).
pub fn in_range(z: &Z3, x: &Ast) -> Ast {
    z.and(&[x.raw_ge(&z.int(i64::MIN)), x.raw_le(&z.int(i64::MAX))])
}

/// verify.py `_conj` (line 72): None is skipped, one item is returned as is, two or more become And.
pub fn conj(z: &Z3, xs: Vec<Option<Ast>>) -> Option<Ast> {
    let xs: Vec<Ast> = xs.into_iter().flatten().collect();
    match xs.len() {
        0 => None,
        1 => Some(xs[0].clone()),
        _ => Some(z.and(&xs)),
    }
}

/// verify.py `_imp` (line 79).
pub fn imp(c: &Ast, d: Option<Ast>) -> Option<Ast> {
    d.map(|d| c.implies(&d))
}

/// Render an integer literal (sign + decimal string) in Python `str(int)` form: strip a leading `+` and zeros; `-0` becomes `0`.
pub fn norm_int(s: &str) -> String {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let d = rest.trim_start_matches('0');
    if d.is_empty() {
        "0".to_string()
    } else if neg {
        format!("-{}", d)
    } else {
        d.to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// Expression-shape helpers (verify.py lines 83-132)
// ---------------------------------------------------------------------------------------------

/// The kind of `_partial_here`: None=all, "variant", "other".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PKind {
    All,
    Variant,
    Other,
}

fn is_tag_fn(n: &str) -> bool {
    matches!(n, "some_val" | "ok_val" | "err_val")
}

/// P2: bit builtins evaluated as 64-bit bit-vector operations (verify.py BIT_Z3); later batches add names here.
fn is_bit_z3(n: &str) -> bool {
    matches!(n, "band" | "bor" | "bxor" | "wrapping_shl" | "ushr" | "wrapping_add" | "wrapping_mul")
}

/// G10 proc A1 (verify.py `_PROC_DOM`): the Process observers with a stream (and index) domain.
fn is_proc_dom(n: &str) -> bool {
    matches!(n, "proc_eof" | "proc_size" | "proc_byte")
}

/// G10 net A1 (verify.py `_NET_DOM`): the Socket observer with an index domain.
fn is_async_dom(n: &str) -> bool { matches!(n, "ev_kind" | "ev_eof" | "ev_write_open" | "ev_byte") }

fn is_net_dom(n: &str) -> bool {
    n == "net_byte"
}

/// verify.py `_partial_here` (line 86).
/// P2 batch 4: the integer literal width argument of load_be / store_be (the checker guarantees 1..=8).
fn lit_width(e: &Expr) -> i64 {
    match e {
        Expr::IntLit(l) => l.value.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn bit_operator(op: &str) -> bool {
    matches!(op, "&" | "|" | "^" | "<<" | ">>")
}

fn shift_factor(e: &Expr) -> Option<i128> {
    let Expr::IntLit(lit) = e else { return None; };
    let k = lit.value.parse::<u32>().ok()?;
    (k < 64).then(|| 1i128 << k)
}

fn low_mask_modulus(e: &Expr) -> Option<i64> {
    let Expr::IntLit(lit) = e else { return None; };
    let m = lit.value.parse::<i64>().ok()?.checked_add(1)?;
    ((2..=1i64 << 62).contains(&m) && m & (m - 1) == 0).then_some(m)
}

fn partial_here(e: &Expr, kind: PKind) -> bool {
    match e {
        Expr::Spent(_) => false,
        Expr::VariantField(_) => kind != PKind::Other,
        _ if kind == PKind::Variant => false,
        Expr::Index(_) => true,
        Expr::Binary(b) => b.op == "/" || b.op == "%" || bit_operator(&b.op),
        Expr::Call(c) => {
            is_tag_fn(&c.name) || c.name == "file_byte" || is_bit_z3(&c.name) || matches!(c.name.as_str(), "load_be" | "load_le") || is_proc_dom(&c.name)
                || is_net_dom(&c.name) || is_async_dom(&c.name)
        }
        Expr::Quant(q) => q.kind == "max" || q.kind == "min",
        _ => false,
    }
}

/// verify.py `_kids` (line 103).
fn kids(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Spent(_) => Vec::new(),
        Expr::Unary(x) => vec![&*x.operand],
        Expr::Binary(x) => vec![&*x.left, &*x.right],
        Expr::Index(x) => vec![&*x.array, &*x.index],
        Expr::Ctor(x) => x.arg.iter().map(|a| &**a).collect(),
        Expr::Quant(x) => vec![&*x.lo, &*x.hi, &*x.body],
        Expr::Call(x) => x.args.iter().collect(),
        Expr::StructLit(x) => x.fields.iter().map(|f| &f.value).collect(),
        Expr::EnumCtor(x) => x.args.iter().collect(),
        Expr::FieldAccess(x) => vec![&*x.base],
        Expr::IsVariant(x) => vec![&*x.value],
        Expr::VariantField(x) => vec![&*x.value],
        _ => Vec::new(),
    }
}

/// verify.py `_has_vf` (line 127).
pub fn has_vf(e: &Expr, kind: PKind) -> bool {
    if partial_here(e, kind) {
        return true;
    }
    kids(e).into_iter().any(|c| has_vf(c, kind))
}

fn pos(e: &Expr) -> Pos {
    mk_pos(e.line(), e.col())
}

/// verify.py `_enum_ty`: the instantiated enum type of an is_variant / variant_field scrutinee: the type of the value
/// (already substituted), else the plain enum name.
fn enum_ty(enum_name: &str, v: &Val) -> Ty {
    if let Val::Dt(_, t) = v {
        if t.name() == enum_name {
            return t.clone();
        }
    }
    Ty::User(enum_name.to_string())
}

/// The datatype value of an argument of is_some / some_val / ... and its type (None for any other value).
fn dt_arg(v: &Val) -> R<(&Ast, &Ty)> {
    match v {
        Val::Dt(a, t) => Ok((a, t)),
        _ => Err("expected an Option/Result value".to_string()),
    }
}

/// The active constructor payload type, with an implicit i64 for legacy types.
fn payload_ty(ty: Option<&Ty>, error: bool) -> Ty {
    match ty {
        Some(Ty::Inst(g)) if error => g.result_error().map(Ty::from_gtype).unwrap_or(Ty::I64),
        Some(Ty::Inst(g)) => g.args.first().map(Ty::from_gtype).unwrap_or(Ty::I64),
        _ => Ty::I64,
    }
}

#[derive(Clone, Copy)]
enum Loop<'a> {
    While(&'a ast::While),
    Map(&'a ast::ForMap),
}

fn stmt_pos(s: &Stmt) -> Pos {
    let (l, c) = match s {
        Stmt::Let(x) => (x.line, x.col),
        Stmt::Assign(x) => (x.line, x.col),
        Stmt::IndexAssign(x) => (x.line, x.col),
        Stmt::If(x) => (x.line, x.col),
        Stmt::While(x) => (x.line, x.col),
        Stmt::ForMap(x) => (x.line, x.col),
        Stmt::Return(x) => (x.line, x.col),
        Stmt::Assert(x) => (x.line, x.col),
        Stmt::Par(x) => (x.line, x.col),
        Stmt::ExprStmt(x) => (x.line, x.col),
        Stmt::FieldAssign(x) => (x.line, x.col),
        Stmt::Match(x) => (x.line, x.col),
    };
    mk_pos(l, c)
}

/// verify.py `assigned` (line 671): assigned names (Let excluded). A name-ordered set, so havoc iteration is in sorted order.
pub fn assigned(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            Stmt::Assign(a) => {
                out.insert(a.name.clone());
            }
            Stmt::IndexAssign(a) => {
                out.insert(a.name.clone());
            }
            Stmt::FieldAssign(a) => {
                out.insert(a.name.clone());
            }
            Stmt::If(i) => {
                assigned(&i.then, out);
                assigned(&i.els, out);
            }
            Stmt::While(w) => assigned(&w.body, out),
            Stmt::ForMap(w) => assigned(&w.body, out),
            Stmt::Match(m) => {
                for a in &m.arms {
                    assigned(&a.body, out);
                }
            }
            _ => {}
        }
    }
}

/// Names of the whole-array inout operands throughout an expression.
fn inout_ops(e: &Expr, out: &mut BTreeSet<String>, select: &impl Fn(&ast::Call, usize) -> bool) {
    if let Expr::Call(c) = e {
        for (i, a) in c.args.iter().enumerate() {
            if !select(c, i) { continue; }
            if let Expr::InoutArg(ia) = a {
                let mut place = ia.expr.as_ref();
                while let Expr::FieldAccess(f) = place {
                    place = &f.base;
                }
                if let Expr::Var(v) = place {
                    out.insert(v.name.clone());
                }
            }
        }
    }
    for child in kids(e) {
        inout_ops(child, out, select);
    }
}

/// verify.py `call_writes`: names passed inout in executable expressions, recursively through statement blocks.
fn call_writes(stmts: &[Stmt], out: &mut BTreeSet<String>, select: &impl Fn(&ast::Call, usize) -> bool) {
    for st in stmts {
        match st {
            Stmt::Let(x) => inout_ops(&x.value, out, select),
            Stmt::Assign(x) => inout_ops(&x.value, out, select),
            Stmt::Return(x) => {
                if let Some(v) = &x.value {
                    inout_ops(v, out, select);
                }
            }
            Stmt::Par(x) => {
                for c in &x.calls {
                    inout_ops(c, out, select);
                }
            }
            Stmt::ExprStmt(x) => inout_ops(&x.expr, out, select),
            Stmt::If(x) => {
                inout_ops(&x.cond, out, select);
                call_writes(&x.then, out, select);
                call_writes(&x.els, out, select);
            }
            Stmt::While(x) => call_writes(&x.body, out, select),
            Stmt::ForMap(x) => call_writes(&x.body, out, select),
            Stmt::Match(m) => {
                inout_ops(&m.scrutinee, out, select);
                for a in &m.arms {
                    call_writes(&a.body, out, select);
                }
            }
            Stmt::Assert(_) | Stmt::FieldAssign(_) | Stmt::IndexAssign(_) => {}
        }
    }
}

/// verify.py `whole_assigned` (G5 A1): names that a whole-variable assignment in stmts can replace (nested blocks included).
fn whole_assigned(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    pruv_core::check::walk_assignments(stmts, &mut |name, path| {
        if path.is_empty() {
            out.insert(name.to_string());
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Values, environment, state
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Arr {
    pub a: Ast,
    pub n: Ast,
    pub name: String,
    pub elem: Elem,
    /// G3: the element type of an array of abstract elements (`elem == Elem::Abs`), None otherwise.
    pub ety: Option<GType>,
}

impl Arr {
    /// Pure assembly (does not touch the fresh counter).
    pub fn new(a: Ast, n: Ast, name: String, elem: Elem) -> Arr {
        Arr { a, n, name, elem, ety: None }
    }
    /// An array whose elements have the abstract type `ety` (a type parameter).
    pub fn abs(a: Ast, n: Ast, name: String, ety: GType) -> Arr {
        Arr { a, n, name, elem: Elem::Abs, ety: Some(ety) }
    }
    /// A new array value of the same element type (mk_arr(.., arr.elem)).
    pub fn like(&self, a: Ast, n: Ast, name: &str) -> Arr {
        Arr { a, n, name: name.to_string(), elem: self.elem, ety: self.ety.clone() }
    }
    /// The Pruv type of an element read from an abstract array.
    pub fn elem_ty(&self) -> Ty {
        match &self.ety {
            Some(g) => Ty::from_gtype(g),
            None => Ty::I64,
        }
    }
    /// The array type of this value (to make a fresh one of the same shape).
    pub fn array_ty(&self) -> Ty {
        match &self.ety {
            Some(g) => Ty::GArray(g.clone()),
            None => Ty::Array(self.elem),
        }
    }
}

/// G6 A1 (verify.py `Map`, G6_SPEC 11.2): a HashMap<K, V> value: membership `d: Array(Sort(K), Bool)`, payload
/// `a: Array(Sort(K), Sort(V))`, size `n`. `kty`/`vty` are the Pruv key/value types.
#[derive(Clone)]
pub struct MapV {
    pub d: Ast,
    pub a: Ast,
    pub n: Ast,
    pub name: String,
    pub kty: GType,
    pub vty: GType,
}

/// G6 A1: the array value of a `Vec<e>` (verify.py `mk_arr(.., elem)`): i64 / bool elements use the legacy kinds, every other
/// element type (an abstract parameter, an Option, a struct, ...) is `Elem::Abs` with the element type kept in `ety`.
fn coll_arr(a: Ast, n: Ast, base: &str, e: &GType) -> Arr {
    if *e == GType::i64() {
        Arr::new(a, n, base.to_string(), Elem::I64)
    } else if *e == GType::bool() {
        Arr::new(a, n, base.to_string(), Elem::Bool)
    } else {
        Arr::abs(a, n, base.to_string(), e.clone())
    }
}

/// G10: independent status fields and a String-shaped byte buffer.
#[derive(Clone)]
pub struct FileV {
    pub ok: Ast,
    pub readable: Ast,
    pub writable: Ast,
    pub buf: Arr,
}

/// G10 proc A1 (verify.py `Process`, G10_PROC_SPEC 4.1): the public state of a Process only. `out` / `err` are the public
/// stdout / stderr buffers as U8 arrays (size, byte map). The child, PID, descriptors, queues and reader timing are not modeled.
#[derive(Clone)]
pub struct ProcV {
    pub error: Ast,
    pub stdin_open: Ast,
    pub stdout_eof: Ast,
    pub stderr_eof: Ast,
    pub out: Arr,
    pub err: Arr,
}

/// G10 net A1 (verify.py `Socket`, G10_NET_SPEC 4.1): the public state of a Socket only. `buf` is the public buffer as a
/// U8 array (length, byte map). The descriptor, peer, address and kernel buffers are not modeled.
#[derive(Clone)]
pub struct SockV {
    pub error: Ast,
    pub write_open: Ast,
    pub eof: Ast,
    pub buf: Arr,
}

/// G10 net A1 (verify.py `Listener`, G10_NET_SPEC 4.1): `{error: Int, port: Int}`.
#[derive(Clone)]
pub struct LisV {
    pub error: Ast,
    pub port: Ast,
}

#[derive(Clone)]
pub struct LoopV {
    pub error: Ast,
    pub cap: Ast,
    pub count: Ast,
    pub last: Ast,
    pub ev_slot: Ast,
    pub ev_kind_: Ast,
    pub exit: Ast,
    pub kind: Ast,
    pub eof: Ast,
    pub write_open: Ast,
    pub buf: Arr,
}

/// Environment value. verify.py distinguishes Int/Bool/Datatype by the z3 expression's sort, but here they are carried as tags.
#[derive(Clone)]
pub enum Val {
    File(FileV),
    /// G10 proc A1: a Process record.
    Proc(ProcV),
    /// G10 net A1: a Socket record.
    Sock(SockV),
    /// G10 net A1: a Listener record.
    Lis(LisV),
    Loop(LoopV),
    /// G10 proc A1: a ProcessArgs value is its argument count only.
    PArgs(Ast),
    Int(Ast),
    Bool(Ast),
    /// An Opt/Res/user struct/enum value and its Pruv type (= `Model.type_of`).
    Dt(Ast, Ty),
    Arr(Arr),
    /// G6 A1: a HashMap value.
    Map(MapV),
    /// Python `None` (print, unit function call).
    Unit,
}

impl Val {
    pub fn ast(&self) -> &Ast {
        match self {
            Val::Int(a) | Val::Bool(a) | Val::Dt(a, _) => a,
            Val::Arr(_) | Val::Map(_) | Val::File(_) | Val::Proc(_) | Val::Sock(_) | Val::Lis(_) | Val::Loop(_) | Val::PArgs(_) | Val::Unit => {
                panic!("value has no single ast")
            }
        }
    }
    pub fn arr(&self) -> &Arr {
        match self {
            Val::Arr(a) => a,
            _ => panic!("not an array"),
        }
    }
    pub fn try_ast(&self) -> R<&Ast> {
        match self {
            Val::Int(a) | Val::Bool(a) | Val::Dt(a, _) => Ok(a),
            Val::Arr(_) => Err("expected a z3 value, got an array".to_string()),
            Val::File(_) => Err("expected a z3 value, got a file".to_string()),
            Val::Proc(_) => Err("expected a z3 value, got a process".to_string()),
            Val::Sock(_) => Err("expected a z3 value, got a socket".to_string()),
            Val::Loop(_) => Err("expected a z3 value, got a loop".into()),
            Val::Lis(_) => Err("expected a z3 value, got a listener".to_string()),
            Val::PArgs(_) => Err("expected a z3 value, got process args".to_string()),
            Val::Map(_) => Err("expected a z3 value, got a map".to_string()),
            Val::Unit => Err("expected a z3 value, got None".to_string()),
        }
    }
    pub fn try_proc(&self) -> R<&ProcV> {
        match self { Val::Proc(p) => Ok(p), _ => Err("expected a process".into()) }
    }
    pub fn try_sock(&self) -> R<&SockV> {
        match self { Val::Sock(p) => Ok(p), _ => Err("expected a socket".into()) }
    }
    pub fn try_loop(&self) -> R<&LoopV> {
        match self { Val::Loop(p) => Ok(p), _ => Err("expected a loop".into()) }
    }
    pub fn try_lis(&self) -> R<&LisV> {
        match self { Val::Lis(p) => Ok(p), _ => Err("expected a listener".into()) }
    }
    pub fn try_pargs(&self) -> R<&Ast> {
        match self { Val::PArgs(a) => Ok(a), _ => Err("expected process args".into()) }
    }
    pub fn try_file(&self) -> R<&FileV> {
        match self { Val::File(f) => Ok(f), _ => Err("expected a file".into()) }
    }
    pub fn try_map(&self) -> R<&MapV> {
        match self {
            Val::Map(m) => Ok(m),
            _ => Err("expected a map".to_string()),
        }
    }
    pub fn try_arr(&self) -> R<&Arr> {
        match self {
            Val::Arr(a) => Ok(a),
            _ => Err("expected an array".to_string()),
        }
    }
    /// The value tag of accessor result `a` whose field type is `ty` (the same classification as Python, which distinguishes by z3 sort).
    pub fn from_field(ty: &Ty, a: Ast) -> R<Val> {
        match ty {
            Ty::I64 => Ok(Val::Int(a)),
            Ty::Bool => Ok(Val::Bool(a)),
            Ty::Opt | Ty::Res | Ty::User(_) | Ty::Param(_) | Ty::Inst(_) => Ok(Val::Dt(a, ty.clone())),
            Ty::Array(_) | Ty::GArray(_) | Ty::Str | Ty::Unit => Err(format!("'{}'", ty.name())),
        }
    }
}

/// An environment that preserves insertion order (Python dict).
#[derive(Clone, Default)]
pub struct Env(pub Vec<(String, Val)>);

impl Env {
    pub fn get(&self, name: &str) -> &Val {
        self.try_get(name).unwrap_or_else(|| panic!("unbound variable {name}"))
    }
    pub fn try_get(&self, name: &str) -> Option<&Val> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
    /// dict assignment: if already present, replace only the value, keeping its position.
    pub fn set(&mut self, name: &str, v: Val) {
        if let Some(e) = self.0.iter_mut().find(|(k, _)| k == name) {
            e.1 = v;
        } else {
            self.0.push((name.to_string(), v));
        }
    }
    /// `del env[name]`.
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(k, _)| k != name);
    }
    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|(k, _)| k == name)
    }
}

#[derive(Clone)]
pub struct State {
    pub env: Env,
    pub pc: Vec<Ast>,
    /// G9 A1 (verify.py `State.obs`): ids of the recursive-value facts already in `pc` (immutable, shared by forks).
    pub obs: std::rc::Rc<HashSet<u32>>,
    /// P13o (verify.py `State.rev`): names of the opaque functions revealed on this path.
    pub rev: std::rc::Rc<BTreeSet<String>>,
    /// P13o (verify.py `State.apps`): (function, application term, argument values) met on this path, in order.
    pub apps: std::rc::Rc<Vec<(String, Ast, Vec<Val>)>>,
    pub app_ids: std::rc::Rc<HashSet<u32>>,
}

impl State {
    /// A fresh state (empty `obs`).
    pub fn new(env: Env, pc: Vec<Ast>) -> State {
        State {
            env,
            pc,
            obs: std::rc::Rc::new(HashSet::new()),
            rev: std::rc::Rc::new(BTreeSet::new()),
            apps: std::rc::Rc::new(Vec::new()),
            app_ids: std::rc::Rc::new(HashSet::new()),
        }
    }

    /// verify.py `State.fork` (line 146).
    pub fn fork(&self, extra: Option<Ast>) -> State {
        let mut pc = self.pc.clone();
        if let Some(e) = extra {
            pc.push(e);
        }
        State {
            env: self.env.clone(),
            pc,
            obs: std::rc::Rc::clone(&self.obs),
            rev: std::rc::Rc::clone(&self.rev),
            apps: std::rc::Rc::clone(&self.apps),
            app_ids: std::rc::Rc::clone(&self.app_ids),
        }
    }
}

/// P13o (verify.py `_opq_parts`): the argument encodings of an opaque application: an array is its contents and its length.
fn opq_parts(args: &[Val]) -> R<Vec<Ast>> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Val::Arr(x) => {
                out.push(x.a.clone());
                out.push(x.n.clone());
            }
            Val::Map(m) => {
                out.push(m.d.clone());
                out.push(m.a.clone());
                out.push(m.n.clone());
            }
            v => out.push(v.try_ast()?.clone()),
        }
    }
    Ok(out)
}

/// P13o: the argument values of a call, read back from the callee environment in parameter order.
fn args_of(cenv: &Env, f: &ast::Function) -> Vec<Val> {
    f.params.iter().map(|p| cenv.get(&p.name).clone()).collect()
}

/// verify.py `_expr_text`: source-like text of a contract expression (used to name measures in messages).
pub(crate) fn expr_text(e: &Expr) -> String {
    match e {
        Expr::Spent(_) => "spent".to_string(),
        Expr::IntLit(x) => norm_int(&x.value),
        Expr::BoolLit(x) => (if x.value { "true" } else { "false" }).to_string(),
        Expr::Var(x) => x.name.clone(),
        Expr::Unary(x) => format!("{}{}", x.op, expr_text(&x.operand)),
        Expr::Binary(x) => {
            let side = |y: &Expr| -> String {
                if matches!(y, Expr::Binary(_)) {
                    format!("({})", expr_text(y))
                } else {
                    expr_text(y)
                }
            };
            format!("{} {} {}", side(&x.left), x.op, side(&x.right))
        }
        Expr::Call(x) => format!("{}({})", x.name, x.args.iter().map(expr_text).collect::<Vec<_>>().join(", ")),
        Expr::Index(x) => format!("{}[{}]", expr_text(&x.array), expr_text(&x.index)),
        Expr::FieldAccess(x) => format!("{}.{}", expr_text(&x.base), x.field),
        Expr::Ctor(x) => match &x.arg {
            None => x.name.clone(),
            Some(a) => format!("{}({})", x.name, expr_text(a)),
        },
        Expr::Quant(x) => format!(
            "{}({} in {}..{}: {})",
            x.kind,
            x.var,
            expr_text(&x.lo),
            expr_text(&x.hi),
            expr_text(&x.body)
        ),
        Expr::IsVariant(x) => format!("is_variant({}, {}::{})", expr_text(&x.value), x.enum_name, x.variant),
        Expr::VariantField(x) => format!(
            "variant_field({}, {}::{}, {})",
            expr_text(&x.value),
            x.enum_name,
            x.variant,
            x.field
        ),
        _ => "...".to_string(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arith {
    Add,
    Sub,
    Mul,
}

// ---------------------------------------------------------------------------------------------
// FnVerifier
// ---------------------------------------------------------------------------------------------

/// verify.py `FnVerifier` (line 189). One per function; `k`/`seen`/`aggs`/`lemmas` are per function.
pub struct FnVerifier<'a> {
    pub z: Z3,
    pub prelude: &'a Prelude,
    pub model: &'a Model,
    pub trace: &'a mut Trace,
    pub fname: String,
    /// `s.k`
    pub k: u32,
    pub(crate) seen: HashSet<(u32, u32, String, Option<String>)>,
    pub qstack: Vec<Ast>,
    /// P13o: opaque function name -> its uninterpreted symbol `opq!F`.
    pub(crate) opq_fns: HashMap<String, FuncDecl>,
    /// `s.aggs`: (kind, Tc.sexpr(), len(outer)) → F. Insertion order preserved (`len(aggs)` goes into the F name).
    pub(crate) aggs: Vec<((String, String, usize), FuncDecl)>,
    /// LG4 `s.agg_shapes`: (kind, F, body, shape, arity) for the aggregate frame lemma.
    pub(crate) agg_shapes: Vec<(String, FuncDecl, Ast, String, usize)>,
    pub lemmas: Vec<Ast>,
    /// LG20 (G-LG19-4): the count range split lemmas, asserted only in a third attempt (verify.py `split_lemmas`)
    pub split_lemmas: Vec<Ast>,
    /// The `(kind, F)` item of `lemma_keys`
    pub(crate) lemma_fn_keys: HashSet<(String, String)>,
    /// The `L.sexpr()` item of `lemma_keys`
    pub(crate) lemma_text_keys: HashSet<String>,
    /// `s.params`
    pub params: Env,
    /// `s.errors`(collected by verify_program in function order).
    pub errors: Vec<PruvError>,
    /// `s.funcs`: name → function (the later one if names repeat).
    resize_params: HashSet<(String, usize)>,
    struct_len_changes: LenChanges,
    pub(crate) funcs: HashMap<String, &'a ast::Function>,
    /// `s.f`: the function being proved (None on the prototype path).
    pub(crate) f: Option<&'a ast::Function>,
    /// Solver query configuration (default: verification, TIMEOUT_MS, no rlimit/seed).
    pub qcfg: QueryConfig,
    /// STAGE1 B2 `extra`: loop number → candidates. If empty, the existing path (fresh numbers, SMT and errors unchanged).
    pub extra: Extra,
    /// Loop (While node address) → loop number. Filled only when `extra` is non-empty (`set_extra`).
    pub(crate) loop_no: HashMap<usize, usize>,
    /// STAGE1 B2 search state. If Some, only candidate obligations are queried and no errors are produced.
    pub probe: Option<&'a mut Probe>,
    /// G3 A2: callee type-parameter substitution while a callee contract is re-interpreted (verify.py `theta`).
    pub(crate) theta: Mapping,
    /// G3 A2 (4.7): per abstract sort, the model values seen so far in this counterexample (their order gives `opaque#N`).
    pub(crate) opq: RefCell<HashMap<String, Vec<String>>>,
    /// G9 A1: function name -> recursive function SCC number (`Program.g9.fn_sccs`); empty without G9 metadata.
    pub(crate) scc_id: HashMap<String, usize>,
    /// G9 A1: the frozen entry value of this function's decreases measure (`s.m0`).
    pub(crate) m0: Option<Ast>,
    /// Stage 5: frozen entry cost bound.
    pub(crate) cost0: Option<Ast>,
    /// G4 A1: the file tag of the function being proved (0 in legacy mode). A function of an explicit-mode program has a
    /// nonzero tag, so `fn_tag != 0` is the explicit visibility mode (Python `s.explicit`); legacy programs keep tag 0.
    pub(crate) fn_tag: u32,
    /// G5 A1 (`s.snap`): immutable entry snapshots of inout array and struct parameters; separate from the current
    /// environment. Empty for a function without inout parameters.
    pub(crate) snap: Env,
    iter_places: Vec<Expr>,
    /// G4b-3 (verify.py `s._kmemo`): AST id -> key piece of that assertion (digest, fresh names; obligation.rs
    /// `key_text`).
    pub(crate) kmemo: HashMap<u32, (String, Vec<String>)>,
    /// LG16 (G-DB26-1): struct name -> (field names, annotated invariant clauses) of every struct with an invariant.
    /// Plain struct declarations for effective invariant clauses and entry snapshots.
    invs: HashMap<String, &'a ast::StructDecl>,
}

/// LG16: one synthesized struct-invariant clause of a function contract (verifier only).
pub(crate) struct InvClause {
    /// the clause with every bare field `f` rewritten to `p.f` (source positions of the clause kept)
    pub expr: Expr,
    pub sname: String,
    /// the clause line (as in `postcondition (line N)`)
    pub line: i64,
    /// the parameter name, or "result"
    pub who: String,
    /// an inout parameter (the clause is an exit obligation on its final value)
    pub inout: bool,
}

/// LG16: rewrite the bare field names of a struct invariant clause to `base.field` (a quantifier variable of the same
/// name shadows the field inside its body and triggers).
fn inv_rewrite(e: &mut Expr, fields: &[String], base: &str, base_ty: &str, path: &[&ast::FieldDecl], bound: &mut Vec<(String, String)>) {
    if let Expr::Var(v) = &mut *e {
        if let Some((_, to)) = bound.iter().rev().find(|(b, _)| *b == v.name) {
            v.name = to.clone();
            return;
        }
        if fields.iter().any(|f| *f == v.name) {
            let mut place = Expr::Var(ast::Var {
                line: v.line, col: v.col, name: base.to_string(), ty: Some(base_ty.to_string()),
            });
            for fd in path {
                place = Expr::FieldAccess(ast::FieldAccess {
                    line: v.line, col: v.col, base: Box::new(place), field: fd.name.clone(), ty: Some(fd.ty.clone()),
                });
            }
            let fa = Expr::FieldAccess(ast::FieldAccess {
                line: v.line,
                col: v.col,
                base: Box::new(place),
                field: v.name.clone(),
                ty: v.ty.clone(),
            });
            *e = fa;
        }
        return;
    }
    match e {
        Expr::InoutArg(x) => inv_rewrite(&mut x.expr, fields, base, base_ty, path, bound),
        Expr::Unary(x) => inv_rewrite(&mut x.operand, fields, base, base_ty, path, bound),
        Expr::Binary(x) => {
            inv_rewrite(&mut x.left, fields, base, base_ty, path, bound);
            inv_rewrite(&mut x.right, fields, base, base_ty, path, bound);
        }
        Expr::Call(x) => x.args.iter_mut().for_each(|a| inv_rewrite(a, fields, base, base_ty, path, bound)),
        Expr::Index(x) => {
            inv_rewrite(&mut x.array, fields, base, base_ty, path, bound);
            inv_rewrite(&mut x.index, fields, base, base_ty, path, bound);
        }
        Expr::Ctor(x) => {
            if let Some(a) = x.arg.as_mut() {
                inv_rewrite(a, fields, base, base_ty, path, bound);
            }
        }
        Expr::Quant(x) => {
            inv_rewrite(&mut x.lo, fields, base, base_ty, path, bound);
            inv_rewrite(&mut x.hi, fields, base, base_ty, path, bound);
            // a quantifier variable named like the parameter would capture `base.f`: rename it (check.py invariant_clause)
            let orig = x.var.clone();
            if x.var == base {
                x.var = format!("$inv_{}", x.var);
            }
            bound.push((orig, x.var.clone()));
            inv_rewrite(&mut x.body, fields, base, base_ty, path, bound);
            x.triggers.iter_mut().for_each(|t| inv_rewrite(t, fields, base, base_ty, path, bound));
            bound.pop();
        }
        Expr::StructLit(x) => x.fields.iter_mut().for_each(|fi| inv_rewrite(&mut fi.value, fields, base, base_ty, path, bound)),
        Expr::EnumCtor(x) => x.args.iter_mut().for_each(|a| inv_rewrite(a, fields, base, base_ty, path, bound)),
        Expr::FieldAccess(x) => inv_rewrite(&mut x.base, fields, base, base_ty, path, bound),
        Expr::IsVariant(x) => inv_rewrite(&mut x.value, fields, base, base_ty, path, bound),
        Expr::VariantField(x) => inv_rewrite(&mut x.value, fields, base, base_ty, path, bound),
        _ => {}
    }
}

type LenChanges = HashMap<(String, usize), BTreeSet<Vec<String>>>;

/// Each path marks all array fields below it. The empty path marks the entire argument.
fn struct_len_changes(prog: &ast::Program, resizes: &HashSet<(String, usize)>) -> LenChanges {
    use pruv_core::foreign_shim::{walk_stmts, Node};
    let structs: HashSet<_> = prog.types.iter().filter_map(|d| {
        if let ast::TypeDecl::Struct(d) = d { Some(d.name.as_str()) } else { None }
    }).collect();
    let funcs: HashMap<_, _> = prog.functions.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut changes = LenChanges::new();
    let mut forwarded = Vec::new();
    for f in &prog.functions {
        let params: HashMap<_, _> = f.params.iter().enumerate()
            .filter(|(_, p)| p.mode_pos.is_some() && structs.contains(p.ty.as_str()))
            .map(|(i, p)| (p.name.as_str(), (f.name.clone(), i)))
            .collect();
        if params.is_empty() { continue; }
        if f.foreign.is_some() {
            for key in params.values() {
                changes.entry(key.clone()).or_default().insert(Vec::new());
            }
        }
        pruv_core::check::walk_assignments(&f.body, &mut |name, path| {
            if let Some(key) = params.get(name) {
                changes.entry(key.clone()).or_default().insert(path.to_vec());
            }
        });
        walk_stmts(&f.body, &mut |node| {
            let Node::Expr(expr) = node else { return };
            match expr {
                Expr::Move(m) => {
                    if let Some(key) = params.get(m.name.as_str()) {
                        changes.entry(key.clone()).or_default().insert(m.fields.clone());
                    }
                }
                Expr::Call(call) => {
                    for (i, arg) in call.args.iter().enumerate() {
                        let Expr::InoutArg(arg) = arg else { continue };
                        let mut root = arg.expr.as_ref();
                        let mut path = Vec::new();
                        while let Expr::FieldAccess(x) = root {
                            path.push(x.field.clone());
                            root = &x.base;
                        }
                        let Expr::Var(root) = root else { continue };
                        let Some(key) = params.get(root.name.as_str()) else { continue };
                        path.reverse();
                        let target = (call.name.clone(), i);
                        if let Some(callee) = funcs.get(call.name.as_str()).filter(|f| f.foreign.is_none()) {
                            if callee.params.get(i).is_some_and(|p| structs.contains(p.ty.as_str())) {
                                forwarded.push((key.clone(), path, target));
                            } else if resizes.contains(&target) {
                                changes.entry(key.clone()).or_default().insert(path);
                            }
                        } else if !matches!(call.name.as_str(),
                            "copy_range" | "copy_within" | "store_be" | "store_le" | "vec_reserve")
                        {
                            changes.entry(key.clone()).or_default().insert(path);
                        }
                    }
                }
                _ => {}
            }
        });
    }
    loop {
        let mut changed = false;
        for (source, prefix, target) in &forwarded {
            let paths: Vec<_> = changes.get(target).into_iter().flatten()
                .map(|suffix| [prefix.as_slice(), suffix.as_slice()].concat()).collect();
            let source = changes.entry(source.clone()).or_default();
            for path in paths {
                if !source.iter().any(|p| path.starts_with(p)) {
                    source.insert(path);
                    changed = true;
                }
            }
        }
        if !changed { return changes; }
    }
}

/// G5 A1: the environment key of a frozen pre-call value.
fn old_key(name: &str) -> String {
    format!("\u{1}old:{}", name)
}

impl<'a> FnVerifier<'a> {
    /// G4 A1 (verify.py `s.explicit`): explicit visibility mode.
    pub(crate) fn explicit(&self) -> bool {
        self.fn_tag != 0
    }

    /// G4 A1 (verify.py `file_of`): the file name of an obligation's node (the function's file for a synthesized node);
    /// None for every legacy program, so legacy output is unchanged.
    pub(crate) fn file_of(&self, node: Pos) -> Option<String> {
        let tag = pruv_core::diagnostic::line_tag(node.0 as i64) as u32;
        let tag = if tag != 0 { tag } else { self.fn_tag };
        pruv_core::diagnostic::file_of_tag(tag as usize)
    }

    /// The line number of a verify error for `node`: the plain line carrying the node's file tag (its function's tag for a
    /// synthesized node); `PruvError::to_json_value` prints it untagged and adds `file`.
    pub(crate) fn err_line(&self, node: Pos) -> i64 {
        let tag = pruv_core::diagnostic::line_tag(node.0 as i64) as u32;
        let tag = if tag != 0 { tag } else { self.fn_tag };
        pruv_core::diagnostic::tagged_line(pos_line(node) as i64, tag as usize)
    }

    /// Prototype (hand-builder) path: uses only the pieces, without a program or function.
    pub fn new(z: &Z3, prelude: &'a Prelude, model: &'a Model, trace: &'a mut Trace, fname: &str) -> Self {
        FnVerifier {
            z: z.clone(),
            prelude,
            model,
            trace,
            fname: fname.to_string(),
            k: 0,
            seen: HashSet::new(),
            qstack: Vec::new(),
            opq_fns: HashMap::new(),
            aggs: Vec::new(),
            agg_shapes: Vec::new(),
            lemmas: Vec::new(),
            split_lemmas: Vec::new(),
            lemma_fn_keys: HashSet::new(),
            lemma_text_keys: HashSet::new(),
            params: Env::default(),
            errors: Vec::new(),
            funcs: HashMap::new(),
            resize_params: HashSet::new(),
            struct_len_changes: HashMap::new(),
            f: None,
            qcfg: QueryConfig::default(),
            extra: HashMap::new(),
            loop_no: HashMap::new(),
            probe: None,
            theta: Mapping::new(),
            opq: RefCell::new(HashMap::new()),
            scc_id: HashMap::new(),
            m0: None,
            cost0: None,
            fn_tag: 0,
            snap: Env::default(),
            iter_places: Vec::new(),
            kmemo: HashMap::new(),
            invs: HashMap::new(),
        }
    }

    /// LG16: the synthesized invariant clauses of `f`: `ensures == false` the requires (every parameter of type exactly S,
    /// by value, take or inout), `ensures == true` the ensures (every inout parameter of type S on its final value, then the
    /// result when the return type is exactly S). Parameters in order. Empty when no struct has an invariant.
    pub(crate) fn inv_clauses(&self, f: &ast::Function, ensures: bool) -> Vec<InvClause> {
        let mut out = Vec::new();
        if self.invs.is_empty() {
            return out;
        }
        let add = |ty: &str, who: &str, inout: bool, out: &mut Vec<InvClause>| {
            let lookup = |name: &str| self.invs.get(name).copied();
            for (s, path) in pruv_core::check::effective_invariant_structs(&GType::from_key(ty), &lookup) {
                let fields: Vec<_> = s.fields.iter().map(|fd| fd.name.clone()).collect();
                let label = std::iter::once(who).chain(path.iter().map(|fd| fd.name.as_str()))
                    .collect::<Vec<_>>().join(".");
                for (c, &line) in s.invariants.iter().zip(&s.invariant_lines) {
                    let mut expr = c.clone();
                    inv_rewrite(&mut expr, &fields, who, ty, &path, &mut Vec::new());
                    out.push(InvClause { expr, sname: s.name.clone(), line, who: label.clone(), inout });
                }
            }
        };
        for p in &f.params {
            if ensures && p.mode_pos.is_none() {
                continue;
            }
            add(p.ty.as_str(), p.name.as_str(), p.mode_pos.is_some(), &mut out);
        }
        if ensures {
            add(f.ret.as_str(), "result", false, &mut out);
        }
        out
    }

    /// verify.py `T(ty)`: a checked node type (a type key) under the current contract-instantiation substitution.
    /// None when the node has no type (or an unparsable one).
    pub(crate) fn t(&self, key: Option<&str>) -> Option<Ty> {
        let k = key?;
        let g = GType::try_from_key(k).ok()?;
        let g = if self.theta.is_empty() { g } else { gtype::substitute(&g, &self.theta) };
        Some(Ty::from_gtype(&g))
    }

    /// verify.py `FnVerifier(.., extra=..)`: fixes the candidates and, if non-empty, builds the loop number table.
    pub fn set_extra(&mut self, extra: Extra) {
        self.loop_no.clear();
        if !extra.is_empty() {
            if let Some(f) = self.f {
                for (n, (w, _, _)) in walk_loops(&f.body).into_iter().enumerate() {
                    self.loop_no.insert(w as *const ast::While as usize, n);
                }
            }
        }
        self.extra = extra;
    }

    pub(crate) fn probe_aborted(&self) -> bool {
        matches!(&self.probe, Some(p) if p.abort.is_some())
    }

    /// `FnVerifier(prog, f, errors, model)`.
    pub fn for_function(
        z: &Z3,
        prelude: &'a Prelude,
        model: &'a Model,
        trace: &'a mut Trace,
        prog: &'a ast::Program,
        f: &'a ast::Function,
    ) -> Self {
        let mut v = FnVerifier::new(z, prelude, model, trace, &f.name);
        for g in &prog.functions {
            v.funcs.insert(g.name.clone(), g);
        }
        v.resize_params = pruv_core::check::resize_params(prog);
        v.struct_len_changes = struct_len_changes(prog, &v.resize_params);
        // LG16: struct invariants (proof-only)
        for d in &prog.types {
            if let ast::TypeDecl::Struct(s) = d {
                if s.tparams.is_empty() {
                    v.invs.insert(s.name.clone(), s);
                }
            }
        }
        // G9 A1: function SCCs (Program.g9)
        if let Some(meta) = &prog.g9 {
            for (i, names) in meta.fn_sccs.iter().enumerate() {
                for n in names {
                    v.scc_id.insert(n.clone(), i);
                }
            }
        }
        v.f = Some(f);
        v.fn_tag = pruv_core::diagnostic::line_tag(f.line) as u32;
        v
    }

    // ---------- fresh (line 201) ----------
    /// `fresh(base, ty)`: bump k first, then the name is `base!k`; an array length is `base!k.len`.
    pub fn try_fresh(&mut self, base: &str, ty: &Ty) -> R<Val> {
        self.k += 1;
        let n = format!("{}!{}", base, self.k);
        let z = &self.z;
        if *ty == Ty::User("file".into()) {
            let buf = Arr::new(z.constant(&n, &z.array_sort(&z.int_sort(), &z.int_sort())),
                z.int_const(&format!("{}.size", n)), base.into(), Elem::U8);
            return Ok(Val::File(FileV { ok: z.bool_const(&format!("{}.ok", n)),
                readable: z.bool_const(&format!("{}.readable", n)),
                writable: z.bool_const(&format!("{}.writable", n)), buf }));
        }
        // G10 proc A1 (4.1): the public state record, same constant names as verify.py fresh
        if *ty == Ty::User("process".into()) {
            let arr_sort = z.array_sort(&z.int_sort(), &z.int_sort());
            let out = Arr::new(z.constant(&format!("{}.stdout", n), &arr_sort),
                z.int_const(&format!("{}.stdout.size", n)), base.into(), Elem::U8);
            let err = Arr::new(z.constant(&format!("{}.stderr", n), &arr_sort),
                z.int_const(&format!("{}.stderr.size", n)), base.into(), Elem::U8);
            return Ok(Val::Proc(ProcV { error: z.int_const(&format!("{}.error", n)),
                stdin_open: z.bool_const(&format!("{}.stdin_open", n)),
                stdout_eof: z.bool_const(&format!("{}.stdout_eof", n)),
                stderr_eof: z.bool_const(&format!("{}.stderr_eof", n)), out, err }));
        }
        if *ty == Ty::User("process_args".into()) {
            return Ok(Val::PArgs(z.int_const(&format!("{}.argc", n))));
        }
        // G10 net A1 (4.1): the public state records, same constant names as verify.py fresh
        if *ty == Ty::User("socket".into()) {
            let buf = Arr::new(z.constant(&format!("{}.buf", n), &z.array_sort(&z.int_sort(), &z.int_sort())),
                z.int_const(&format!("{}.buf.len", n)), base.into(), Elem::U8);
            return Ok(Val::Sock(SockV { error: z.int_const(&format!("{}.error", n)),
                write_open: z.bool_const(&format!("{}.write_open", n)),
                eof: z.bool_const(&format!("{}.eof", n)), buf }));
        }
        if *ty == Ty::User("loop".into()) {
            return Ok(Val::Loop(LoopV {
                error: z.int_const(&format!("{}.error", n)),
                cap: z.int_const(&format!("{}.cap", n)),
                count: z.int_const(&format!("{}.count", n)),
                last: z.int_const(&format!("{}.last", n)),
                ev_slot: z.int_const(&format!("{}.ev_slot", n)),
                ev_kind_: z.int_const(&format!("{}.ev_kind_", n)),
                exit: z.int_const(&format!("{}.exit", n)),
                kind: z.constant(&format!("{}.kind", n), &z.array_sort(&z.int_sort(), &z.int_sort())),
                eof: z.constant(&format!("{}.eof", n), &z.array_sort(&z.int_sort(), &z.bool_sort())),
                write_open: z.constant(&format!("{}.write_open", n), &z.array_sort(&z.int_sort(), &z.bool_sort())),
                buf: Arr::new(z.constant(&format!("{}.buf", n), &z.array_sort(&z.int_sort(), &z.int_sort())), z.int_const(&format!("{}.buf.len", n)), base.into(), Elem::U8),
            }));
        }
        if *ty == Ty::User("listener".into()) {
            return Ok(Val::Lis(LisV { error: z.int_const(&format!("{}.error", n)),
                port: z.int_const(&format!("{}.port", n)) }));
        }
        // G6 A1: Vec = [T] array, String = u8 array, HashMap = (d, a, n). Python checks this after the legacy array
        // cases, but a collection type is never one of them, so the order is not observable.
        if let Some(c) = ty.coll() {
            let int = z.int_sort();
            return match c {
                Coll::Str => {
                    let arr_sort = z.array_sort(&int, &int);
                    Ok(Val::Arr(Arr::new(
                        z.constant(&n, &arr_sort),
                        z.int_const(&format!("{}.len", n)),
                        base.to_string(),
                        Elem::U8,
                    )))
                }
                Coll::Vec(e) => {
                    let esort = self.model.try_sort_of(&Ty::from_gtype(&e))?;
                    let arr_sort = z.array_sort(&int, &esort);
                    Ok(Val::Arr(coll_arr(z.constant(&n, &arr_sort), z.int_const(&format!("{}.len", n)), base, &e)))
                }
                Coll::Map(k, v) => {
                    // ks, vs = sort_of(K), sort_of(V) in this order (instance numbering of generic datatypes)
                    let ks = self.model.map_key_sort(&k)?;
                    let vs = self.model.try_sort_of(&Ty::from_gtype(&v))?;
                    let d_sort = z.array_sort(&ks, &z.bool_sort());
                    let a_sort = z.array_sort(&ks, &vs);
                    Ok(Val::Map(MapV {
                        d: z.constant(&format!("{}.d", n), &d_sort),
                        a: z.constant(&format!("{}.a", n), &a_sort),
                        n: z.int_const(&format!("{}.len", n)),
                        name: base.to_string(),
                        kty: k,
                        vty: v,
                    }))
                }
            };
        }
        match ty {
            Ty::I64 => Ok(Val::Int(z.int_const(&n))),
            Ty::Bool => Ok(Val::Bool(z.bool_const(&n))),
            Ty::Opt => Ok(Val::Dt(z.constant(&n, &self.prelude.opt.sort), ty.clone())),
            Ty::Res => Ok(Val::Dt(z.constant(&n, &self.prelude.res.sort), ty.clone())),
            Ty::Array(e) => {
                let int = z.int_sort();
                let esort = if *e == Elem::Bool { z.bool_sort() } else { z.int_sort() };
                let arr_sort = z.array_sort(&int, &esort);
                Ok(Val::Arr(Arr::new(
                    z.constant(&n, &arr_sort),
                    z.int_const(&format!("{}.len", n)),
                    base.to_string(),
                    *e,
                )))
            }
            Ty::Str => {
                let int = z.int_sort();
                let arr_sort = z.array_sort(&int, &int);
                Ok(Val::Arr(Arr::new(
                    z.constant(&n, &arr_sort),
                    z.int_const(&format!("{}.len", n)),
                    base.to_string(),
                    Elem::U8,
                )))
            }
            Ty::User(name) => match self.model.sort_opt(name) {
                Some(s) => Ok(Val::Dt(z.constant(&n, s), ty.clone())),
                None => Err(format!("fresh: {}", name)),
            },
            // G3: an abstract value, or an instance Option<X> / Result<X> / D<..>
            Ty::Param(_) | Ty::Inst(_) => {
                let s = self.model.try_sort_of(ty)?;
                Ok(Val::Dt(z.constant(&n, &s), ty.clone()))
            }
            // G3: [T] = (Array Int sort(T), length)
            Ty::GArray(e) => {
                let int = z.int_sort();
                let len = z.int_const(&format!("{}.len", n));
                if e.owner.is_some() {
                    let esort = self.model.try_sort_of(&Ty::from_gtype(e))?;
                    let arr_sort = z.array_sort(&int, &esort);
                    Ok(Val::Arr(Arr::abs(z.constant(&n, &arr_sort), len, base.to_string(), e.clone())))
                } else {
                    let (esort, elem) = match Ty::from_gtype(e) {
                        Ty::Bool => (z.bool_sort(), Elem::Bool),
                        Ty::User(u) if u == "u8" => (z.int_sort(), Elem::U8),
                        _ => (z.int_sort(), Elem::I64),
                    };
                    let arr_sort = z.array_sort(&int, &esort);
                    Ok(Val::Arr(Arr::new(z.constant(&n, &arr_sort), len, base.to_string(), elem)))
                }
            }
            Ty::Unit => Err("fresh: unit".to_string()),
        }
    }
    /// For the prototype: `Val::Unit` on failure.
    pub fn fresh(&mut self, base: &str, ty: &Ty) -> Val {
        self.try_fresh(base, ty).unwrap_or(Val::Unit)
    }
    pub fn fresh_int(&mut self, base: &str) -> Ast {
        self.fresh(base, &Ty::I64).ast().clone()
    }

    /// Even if the `!dk` constant made by dom()'s Quant branch is unused, `s.k += 1` still happens (line 499).
    pub fn bump_k(&mut self) {
        self.k += 1;
    }

    // ---------- value domain (line 219) ----------
    fn wf_file(&self, f: &FileV) -> Ast { self.z.and(&[f.buf.n.ge_i(0), self.u8_dom(&f.buf)]) }

    /// G10 proc A1 (verify.py `_wf_proc`, 4.2): 0 <= error <= 6; buffer sizes in 0..M with bytes in 0..255; an open stdin
    /// implies no error; an error means stdin closed, both buffers empty, both EOFs false.
    fn wf_proc(&self, p: &ProcV) -> Ast {
        let z = &self.z;
        z.and(&[
            p.error.ge_i(0), p.error.le_i(6),
            p.out.n.ge_i(0), p.out.n.le_i(MAX_LEN), p.err.n.ge_i(0), p.err.n.le_i(MAX_LEN),
            self.u8_dom(&p.out), self.u8_dom(&p.err),
            p.stdin_open.implies(&p.error.raw_eq(&z.int(0))),
            p.error.ne_i(0).implies(&z.and(&[
                p.stdin_open.not(), p.out.n.raw_eq(&z.int(0)), p.err.n.raw_eq(&z.int(0)),
                p.stdout_eof.not(), p.stderr_eof.not(),
            ])),
        ])
    }

    /// G10 net A1 (verify.py `_wf_socket`, 4.2): 0 <= error <= 6; buffer length in 0..M with bytes in 0..255; an open write
    /// side implies no error; an error means write side closed, empty buffer, EOF false.
    fn wf_sock(&self, p: &SockV) -> Ast {
        let z = &self.z;
        z.and(&[
            p.error.ge_i(0), p.error.le_i(6),
            p.buf.n.ge_i(0), p.buf.n.le_i(MAX_LEN), self.u8_dom(&p.buf),
            p.write_open.implies(&p.error.raw_eq(&z.int(0))),
            p.error.ne_i(0).implies(&z.and(&[p.write_open.not(), p.buf.n.raw_eq(&z.int(0)), p.eof.not()])),
        ])
    }

    /// G10 net A1 (verify.py `_wf_listener`, 4.2): 0 <= error <= 2; ok implies 1 <= port <= 65535; failed implies port == 0.
    fn wf_lis(&self, l: &LisV) -> Ast {
        let z = &self.z;
        z.and(&[
            l.error.ge_i(0), l.error.le_i(2),
            l.error.raw_eq(&z.int(0)).implies(&z.and(&[l.port.ge_i(1), l.port.le_i(65535)])),
            l.error.ne_i(0).implies(&l.port.raw_eq(&z.int(0))),
        ])
    }

    /// verify.py `_net_keep_buf`: the public buffer snapshot (length, byte map) and EOF of r equal those of p
    fn net_keep_buf(&self, r: &SockV, p: &SockV) -> Ast {
        self.z.and(&[r.buf.n.eq(&p.buf.n), r.buf.a.eq(&p.buf.a), r.eof.eq(&p.eof)])
    }

    /// G10 proc A1 (4.3): 0 <= argc <= M
    fn wf_pargs(&self, argc: &Ast) -> Ast { self.z.and(&[argc.ge_i(0), argc.le_i(MAX_LEN)]) }

    /// proc_size(p, sm): stream 1 is stdout, any other (domain-checked) stream is stderr
    fn proc_size_t(&self, p: &ProcV, sm: &Ast) -> Ast {
        self.z.ite(&sm.raw_eq(&self.z.int(1)), &p.out.n, &p.err.n)
    }
    fn proc_eof_t(&self, p: &ProcV, sm: &Ast) -> Ast {
        self.z.ite(&sm.raw_eq(&self.z.int(1)), &p.stdout_eof, &p.stderr_eof)
    }
    fn proc_stream_ok(&self, sm: &Ast) -> Ast {
        self.z.or(&[sm.raw_eq(&self.z.int(1)), sm.raw_eq(&self.z.int(2))])
    }
    /// the stdout snapshot (size, byte map, EOF) of r equals that of p
    fn proc_keep_out(&self, r: &ProcV, p: &ProcV) -> Ast {
        self.z.and(&[r.out.n.eq(&p.out.n), r.out.a.eq(&p.out.a), r.stdout_eof.eq(&p.stdout_eof)])
    }
    fn proc_keep_err(&self, r: &ProcV, p: &ProcV) -> Ast {
        self.z.and(&[r.err.n.eq(&p.err.n), r.err.a.eq(&p.err.a), r.stderr_eof.eq(&p.stderr_eof)])
    }

    /// `wf(ty, v)`: all active leaves are i64. None if trivial.
    pub fn wf(&self, ty: &Ty, v: &Ast) -> Option<Ast> {
        self.try_wf(ty, v).ok().flatten()
    }

    /// `wf` that reports an internal failure (Python raises) instead of hiding it.
    pub fn try_wf(&self, ty: &Ty, v: &Ast) -> R<Option<Ast>> {
        let z = &self.z;
        let p = self.prelude;
        let m = self.model;
        if matches!(ty, Ty::Inst(_)) && m.wf_trivial(ty)? {
            // G3: no constrained leaf anywhere inside (e.g. Dup<Dup<T>>); create the datatype as the full walk
            // would, but skip the field recursion, which is exponential in the nesting depth (verify.py `wf`)
            match ty {
                Ty::Inst(g) if g.name == "option" => {
                    m.opt(Some(ty))?;
                }
                Ty::Inst(g) if g.name == "result" => {
                    m.res(Some(ty))?;
                }
                _ => match m.nominal_kind(ty) {
                    Some(NomKind::Struct) => {
                        m.struct_info(ty)?;
                    }
                    Some(NomKind::Enum) => {
                        for vn in m.variant_names(ty) {
                            m.variant_fns(ty, &vn)?;
                        }
                    }
                    None => {}
                },
            }
            return Ok(None);
        }
        Ok(match ty {
            Ty::I64 => Some(in_range(z, v)),
            Ty::Opt => Some(p.is_some(v).implies(&in_range(z, &p.sval(v)))),
            Ty::Res => Some(z.and(&[
                p.is_ok(v).implies(&in_range(z, &p.oval(v))),
                p.is_err(v).implies(&in_range(z, &p.eval(v))),
            ])),
            // G3: an abstract value has no well-formedness predicate
            Ty::Param(_) => None,
            // G3: Option<X>, X other than i64
            Ty::Inst(g) if g.name == "option" => {
                let o = m.opt(Some(ty))?;
                let w = self.try_wf(&Ty::from_gtype(&g.args[0]), &o.sval.apply1(v))?;
                w.map(|w| o.is_some.apply1(v).implies(&w))
            }
            Ty::Inst(g) if g.name == "result" => {
                let o = m.res(Some(ty))?;
                let w = self.try_wf(&Ty::from_gtype(&g.args[0]), &o.oval.apply1(v))?;
                let mut parts: Vec<Option<Ast>> = Vec::new();
                if let Some(w) = w {
                    parts.push(Some(o.is_ok.apply1(v).implies(&w)));
                }
                if let Some(w) = self.wf_field(&payload_ty(Some(ty), true), &o.eval.apply1(v))? {
                    parts.push(Some(o.is_err.apply1(v).implies(&w)));
                }
                conj(z, parts)
            }
            // G9 A1: a recursive type is the application of its wf_D definition, never a Python expansion
            Ty::User(n) if m.in_scc(n) => Some(m.wf_fn(n)?.apply1(v)),
            Ty::User(_) | Ty::Inst(_) => self.wf_step(ty, v)?,
            Ty::Bool | Ty::Unit | Ty::Array(_) | Ty::GArray(_) | Ty::Str => None,
        })
    }

    /// verify.py `_wf_step`: one structural step of well_formed for a struct/enum type (the fields are checked with `wf_field`).
    pub(crate) fn wf_step(&self, ty: &Ty, v: &Ast) -> R<Option<Ast>> {
        let z = &self.z;
        let m = self.model;
        Ok(match m.nominal_kind(ty) {
            Some(NomKind::Struct) => {
                let accs = m.struct_info(ty)?.accs;
                let mut parts: Vec<Option<Ast>> = Vec::new();
                for ((_, ft), acc) in m.fields_of(ty, None)?.iter().zip(accs.iter()) {
                    parts.push(self.wf_field(ft, &acc.apply1(v))?);
                }
                conj(z, parts)
            }
            Some(NomKind::Enum) => {
                let mut parts: Vec<Option<Ast>> = Vec::new();
                for vn in m.variant_names(ty) {
                    let vf = m.variant_fns(ty, &vn)?;
                    let mut ws: Vec<Option<Ast>> = Vec::new();
                    for ((_, ft), acc) in m.fields_of(ty, Some(&vn))?.iter().zip(vf.accs.iter()) {
                        ws.push(self.wf_field(ft, &acc.apply1(v))?);
                    }
                    if let Some(w) = conj(z, ws) {
                        parts.push(Some(vf.rec.apply1(v).implies(&w)));
                    }
                }
                conj(z, parts)
            }
            None => None,
        })
    }

    /// `_wf_of(m, ft, m.field_get(ft, acc(v), name))`: a stored buffer field is viewed as an Arr (`Model::buf_parts`),
    /// every other field is checked as is.
    fn wf_field(&self, ft: &Ty, term: &Ast) -> R<Option<Ast>> {
        if Model::is_buf(ft) {
            let (a, n) = self.model.buf_parts(ft, term)?;
            return Ok(Some(self.wf_buf(ft, &n, &a)));
        }
        if matches!(ft.coll(), Some(Coll::Map(..))) {
            let (_, f) = self.model.map_field(ft)?;
            let n = f.accs[2].apply1(term);
            return Ok(Some(self.z.and(&[n.ge_i(0), n.le_i(MAX_LEN)])));
        }
        self.try_wf(ft, term)
    }

    /// The Arr branch of verify.py `_wf_of` for a buffer field: `0 <= len <= 2^40` and, for u8/str, the byte domain.
    fn wf_buf(&self, ty: &Ty, n: &Ast, a: &Ast) -> Ast {
        let ln = self.z.and(&[n.ge_i(0), n.le_i(MAX_LEN)]);
        if Model::buf_elem(ty) == Elem::U8 {
            let arr = Arr::new(a.clone(), n.clone(), String::new(), Elem::U8);
            self.z.and(&[ln, self.u8_dom(&arr)])
        } else {
            ln
        }
    }

    /// `wf(ty, v)` of an environment value: an Arr view of a buffer field (G9) uses the buffer branch, anything else `try_wf`.
    fn wf_of_val(&self, ty: &Ty, v: &Val) -> R<Option<Ast>> {
        if let Val::Map(m) = v {
            return Ok(Some(self.z.and(&[m.n.ge_i(0), m.n.le_i(MAX_LEN)])));
        }
        if let Val::File(f) = v { return Ok(Some(self.wf_file(f))); }
        if let Val::Proc(p) = v { return Ok(Some(self.wf_proc(p))); }
        if let Val::Sock(p) = v { return Ok(Some(self.wf_sock(p))); }
        if let Val::Loop(p) = v { return Ok(Some(self.wf_loop(p))); }
        if let Val::Lis(l) = v { return Ok(Some(self.wf_lis(l))); }
        if let Val::PArgs(a) = v { return Ok(Some(self.wf_pargs(a))); }
        if let Val::Arr(a) = v {
            if Model::is_buf(ty) {
                return Ok(Some(self.wf_buf(ty, &a.n, &a.a)));
            }
        }
        self.try_wf(ty, v.try_ast()?)
    }

    /// verify.py `Model.field_get` for the verifier's values: a buffer field becomes an Arr view named `name`, any other field
    /// is classified by its type (`Val::from_field`).
    fn field_val(&self, fty: &Ty, term: Ast, name: &str) -> R<Val> {
        if Model::is_buf(fty) {
            let (a, n) = self.model.buf_parts(fty, &term)?;
            let arr = if let Some(Coll::Vec(elem)) = fty.coll() {
                coll_arr(a, n, name, &elem)
            } else {
                Arr::new(a, n, name.to_string(), Model::buf_elem(fty))
            };
            return Ok(Val::Arr(arr));
        }
        if let Some(Coll::Map(kty, vty)) = fty.coll() {
            let (_, f) = self.model.map_field(fty)?;
            return Ok(Val::Map(MapV {
                d: f.accs[0].apply1(&term), a: f.accs[1].apply1(&term),
                n: f.accs[2].apply1(&term), name: name.to_string(), kty, vty,
            }));
        }
        Val::from_field(fty, term)
    }

    /// verify.py `Model.field_put`: an Arr stored into a buffer field becomes a Buf value, any other value is stored as is.
    fn field_put(&self, fty: &Ty, v: &Val) -> R<Ast> {
        if let Val::Arr(a) = v {
            if Model::is_buf(fty) {
                return Ok(self.model.buf_put(fty, &a.n, &a.a)?);
            }
        }
        if let Val::Map(m) = v {
            let (_, f) = self.model.map_field(fty)?;
            return Ok(f.ctor.apply(&[m.d.clone(), m.a.clone(), m.n.clone()]));
        }
        Ok(v.try_ast()?.clone())
    }

    /// verify.py `FnVerifier.observe`: add the valid one-step size/wf facts of the observed nominal term `t` (`facts`) to the
    /// path condition of `st`, once per fact. Only for recursive-related types, or any non-generic nominal when `force`d (an
    /// explicit `size` call). A no-op for programs without recursive types. (The Python early return for an Arr/Map term is
    /// the caller's: only datatype values are passed.)
    pub(crate) fn observe(&self, st: &mut State, t: &Ast, ty: Option<&Ty>, force: bool) -> R<()> {
        let m = self.model;
        let Some(ty) = ty else { return Ok(()) };
        if !(m.rel(ty) || (force && m.rel_nominal(ty))) {
            return Ok(());
        }
        // z3 AST ids: a term (datatype sort) never shares an id with a fact (Bool)
        let tid = t.id();
        if st.obs.contains(&tid) {
            return Ok(());
        }
        let mut new: Vec<u32> = vec![tid];
        for fct in self.facts(ty, t)? {
            let i = fct.id();
            if !st.obs.contains(&i) {
                new.push(i);
                st.pc.push(fct);
            }
        }
        let obs = std::rc::Rc::make_mut(&mut st.obs);
        for i in new {
            obs.insert(i);
        }
        Ok(())
    }

    /// verify.py `Model.facts`: valid facts about one observed term `t` of the non-generic nominal type `ty` (see
    /// tests/g9/model_notes.md): size positivity, the one-step size equation, for a recursive type the one-step wf equation, and
    /// positivity of the size of every direct nominal child (so child-smaller follows from the equation). It lives here
    /// because the wf step needs `in_range` / `u8_dom` / `conj`.
    pub(crate) fn facts(&self, ty: &Ty, t: &Ast) -> R<Vec<Ast>> {
        let m = self.model;
        let n = ty.name().to_string();
        let sf = m.size_fn(&n)?;
        let mut out: Vec<Ast> = Vec::new();
        out.push(sf.apply1(t).ge_i(1));
        let sv = m.size_value(ty, t)?;
        out.push(sf.apply1(t).eq(&sv));
        if m.in_scc(&n) {
            let body = self.wf_step(ty, t)?;
            let rhs = match body {
                Some(b) => b,
                None => self.z.bool_val(true),
            };
            out.push(m.wf_fn(&n)?.apply1(t).eq(&rhs));
        }
        let groups: Vec<(Option<String>, Vec<FuncDecl>)> = match m.nominal_kind(ty) {
            Some(NomKind::Struct) => vec![(None, m.struct_info(ty)?.accs)],
            _ => {
                let mut gs: Vec<(Option<String>, Vec<FuncDecl>)> = Vec::new();
                for vn in m.variant_names(ty) {
                    let accs = m.variant_fns(ty, &vn)?.accs;
                    gs.push((Some(vn), accs));
                }
                gs
            }
        };
        for (vname, accs) in &groups {
            for ((_fd, fty), acc) in m.fields_of(ty, vname.as_deref())?.iter().zip(accs.iter()) {
                if m.rel_nominal(fty) {
                    out.push(m.size_fn(fty.name())?.apply1(&acc.apply1(t)).ge_i(1));
                } else if matches!(fty, Ty::Inst(_)) && m.is_user(fty) && m.fixed_size(fty)?.is_none() {
                    out.push(m.size_inst(fty)?.apply1(&acc.apply1(t)).ge_i(1));
                } else if let Ty::Inst(g) = fty {
                    // E1: the Err payload of a Result<T, E> field counts in the weight, so its size is positive too
                    if g.name == "result" {
                        if let Some(e) = g.result_error() {
                            let ety = Ty::from_gtype(e);
                            let ev = m.res(Some(fty))?.eval.apply1(&acc.apply1(t));
                            if m.rel_nominal(&ety) {
                                out.push(m.size_fn(ety.name())?.apply1(&ev).ge_i(1));
                            } else if matches!(ety, Ty::Inst(_)) && m.is_user(&ety) && m.fixed_size(&ety)?.is_none() {
                                out.push(m.size_inst(&ety)?.apply1(&ev).ge_i(1));
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// `rebuild(ty, cur, path, new)` (line 242): the struct value `cur` with the field path replaced by `new`.
    pub fn try_rebuild(&self, ty: &Ty, cur: &Ast, path: &[&str], new: &Ast) -> R<Ast> {
        self.try_rebuild_v(ty, cur, path, &Val::Int(new.clone()))
    }

    /// `rebuild` with an environment value as the new field value (G9 A1: a buffer field stores a Buf datatype value built from
    /// an Arr, `Model.field_put`).
    pub fn try_rebuild_v(&self, ty: &Ty, cur: &Ast, path: &[&str], new: &Val) -> R<Ast> {
        if path.is_empty() {
            return Ok(new.try_ast()?.clone());
        }
        let m = self.model;
        let i = m.field_index_of(ty, path[0], None)?;
        let sf = m.struct_info(ty)?;
        let mut vals: Vec<Ast> = sf.accs.iter().map(|a| a.apply1(cur)).collect();
        let fty = m.fields_of(ty, None)?[i].1.clone();
        if path.len() == 1 {
            // G9 A1: a buffer field stores a Buf datatype value
            vals[i] = self.field_put(&fty, new)?;
        } else {
            let sub = self.try_rebuild_v(&fty, &vals[i], &path[1..], new)?;
            vals[i] = sub;
        }
        Ok(sf.ctor.apply(&vals))
    }

    // ---------- first part of run() (line 835): for the prototype ----------
    /// `fresh` for each parameter; arrays put `0 <= len <= MAX_LEN` into pc, everything else puts `wf`. Also fills `self.params`.
    pub fn init_params(&mut self, params: &[(&str, Ty)]) -> State {
        let mut env = Env::default();
        let mut pc: Vec<Ast> = Vec::new();
        for (name, ty) in params {
            let v = self.fresh(name, ty);
            match &v {
                Val::Arr(a) => pc.push(self.z.and(&[a.n.ge_i(0), a.n.le_i(MAX_LEN)])),
                Val::File(f) => pc.push(self.wf_file(f)),
                Val::Proc(p) => pc.push(self.wf_proc(p)),
                Val::Sock(p) => pc.push(self.wf_sock(p)),
                Val::Loop(p) => pc.push(self.wf_loop(p)),
                Val::Lis(l) => pc.push(self.wf_lis(l)),
                Val::PArgs(a) => pc.push(self.wf_pargs(a)),
                Val::Map(m) => pc.push(self.z.and(&[m.n.ge_i(0), m.n.le_i(MAX_LEN)])),
                Val::Unit => {}
                other => {
                    if let Some(w) = self.wf(ty, other.ast()) {
                        pc.push(w);
                    }
                }
            }
            env.set(name, v);
        }
        self.params = env.clone();
        State::new(env, pc)
    }

    /// First part of `ensures_ok` (line 647): overlays the parameters' original values on the current env and inserts `result`.
    pub fn ensures_env(&self, st: &State, result: Option<Val>) -> Env {
        let mut env = st.env.clone();
        for (n, v) in &self.params.0 {
            env.set(n, v.clone());
        }
        if let Some(r) = result {
            env.set("result", r);
        }
        env
    }

    // ---------- ev pieces (for the prototype) ----------
    /// `assume(st, g, fact)` (line 340).
    pub fn assume(&self, st: &mut State, g: &[Ast], fact: Ast) {
        if g.is_empty() {
            st.pc.push(fact);
        } else {
            st.pc.push(self.z.and(g).implies(&fact));
        }
    }

    /// Binary `+ - *` (lines 361-385): builds the result and the `in_range(res)` obligation (code "overflow").
    pub fn ev_arith(&mut self, st: &State, g: &[Ast], op: Arith, l: &Ast, r: &Ast, node: Pos) -> Ast {
        let res = match op {
            Arith::Add => l.add(r),
            Arith::Sub => l.sub(r),
            Arith::Mul => l.mul(r),
        };
        let ob = in_range(&self.z, &res);
        self.prove(st, g, &ob, node, "overflow", None);
        res
    }

    /// Binary `/ %` (lines 373-376): obligation `r > 0 && l >= 0` (code "div").
    pub fn ev_div(&mut self, st: &State, g: &[Ast], is_mod: bool, l: &Ast, r: &Ast, node: Pos) -> Ast {
        let ob = self.z.and(&[r.gt_i(0), l.ge_i(0)]);
        self.prove(st, g, &ob, node, "div", None);
        if is_mod {
            l.modulo(r)
        } else {
            l.div(r)
        }
    }

    /// Unary `-` (lines 358-360).
    pub fn ev_neg(&mut self, st: &State, g: &[Ast], v: &Ast, node: Pos) -> Ast {
        let ob = in_range(&self.z, &v.neg());
        self.prove(st, g, &ob, node, "overflow", None);
        v.neg()
    }

    /// Index (lines 386-395): obligation `0 <= i < n` (code "index_oob"); assumes `in_range` of the value read into pc.
    pub fn ev_index(&mut self, st: &mut State, g: &[Ast], arr: &Arr, i: &Ast, node: Pos) -> Ast {
        let ob = self.z.and(&[i.ge_i(0), i.lt(&arr.n)]);
        self.prove(st, g, &ob, node, "index_oob", None);
        let v = arr.a.select(i);
        let w = in_range(&self.z, &v);
        self.assume(st, g, w);
        v
    }

    /// Call `alloc(n, init)` (lines 609-613). `args` = [n, init]. After the "alloc_negative" obligation, `s.k += 1`.
    pub fn ev_alloc(&mut self, st: &State, g: &[Ast], n: &Ast, init: &Ast, node: Pos) -> Arr {
        let ob = self.z.and(&[n.ge_i(0), n.le_i(MAX_LEN)]);
        self.prove(st, g, &ob, node, "alloc_negative", None);
        self.k += 1;
        Arr::new(self.z.const_array(&self.z.int_sort(), init), n.clone(), "alloc".to_string(), Elem::I64)
    }

    // ---------- exec pieces (for the prototype; st is a state the caller has already forked) ----------
    /// IndexAssign (lines 718-725): after the obligation `0 <= i < arr.n` ("index_oob", node = statement), `Store`.
    pub fn index_assign(&mut self, st: &mut State, name: &str, i: &Ast, v: &Ast, node: Pos) {
        let arr = st.env.get(name).arr().clone();
        let ob = self.z.and(&[i.ge_i(0), i.lt(&arr.n)]);
        self.prove(st, &[], &ob, node, "index_oob", None);
        st.env.set(name, Val::Arr(Arr::new(arr.a.store(i, v), arr.n.clone(), name.to_string(), arr.elem)));
    }

    /// FieldAssign (lines 761-766): the right-hand value `v` is computed first by the caller.
    pub fn field_assign(&mut self, st: &mut State, name: &str, path: &[&str], v: &Ast) {
        let Val::Dt(root, ty) = st.env.get(name).clone() else { panic!("field assign on non-struct") };
        let new = self.try_rebuild(&ty, &root, path, v).unwrap_or_else(|_| v.clone());
        st.env.set(name, Val::Dt(new, ty));
    }

    /// One check_dom case (lines 524-543): no query if the domain `d` is None or `simplify(d)` is true,
    /// otherwise `prove(.., code)` and return d (what is returned goes to the caller's `prev`).
    pub fn check_dom_one(&mut self, st: &State, g: &[Ast], d: Option<Ast>, node: Pos, code: &str) -> Option<Ast> {
        let d = d?;
        if d.simplify().is_true() {
            return None;
        }
        self.prove(st, g, &d, node, code, None);
        Some(d)
    }

    // ---------- havoc (line 684) ----------
    /// Iterates in name order (byte order = code point order, Python `sorted`) (the 0A sorting of SPEC §6).
    /// `keep` (G5 A1): array names whose havoc retains the length term (inout parameters, and locals changed only by inout
    /// calls); every other array gets an arbitrary well-formed length exactly as before.
    pub fn havoc_set(&mut self, st: &State, names: &BTreeSet<String>, keep: &BTreeSet<String>) -> R<State> {
        let mut h = st.fork(None);
        for n in names {
            let Some(old) = h.env.try_get(n).cloned() else { continue };
            match old {
                Val::File(_) => {
                    let nv = self.try_fresh(n, &Ty::User("file".into()))?;
                    h.pc.push(self.wf_file(nv.try_file()?));
                    h.env.set(n, nv);
                }
                // G10 proc A1 (4.8): only well-formedness survives a havoc
                Val::Proc(_) => {
                    let nv = self.try_fresh(n, &Ty::User("process".into()))?;
                    h.pc.push(self.wf_proc(nv.try_proc()?));
                    h.env.set(n, nv);
                }
                // G10 net A1 (4.8): only well-formedness survives a havoc
                Val::Sock(_) => {
                    let nv = self.try_fresh(n, &Ty::User("socket".into()))?;
                    h.pc.push(self.wf_sock(nv.try_sock()?));
                    h.env.set(n, nv);
                }
                Val::Loop(_) => {
                    let nv = self.try_fresh(n, &Ty::User("loop".into()))?;
                    h.pc.push(self.wf_loop(nv.try_loop()?));
                    h.env.set(n, nv);
                }
                Val::Lis(_) => {
                    let nv = self.try_fresh(n, &Ty::User("listener".into()))?;
                    h.pc.push(self.wf_lis(nv.try_lis()?));
                    h.env.set(n, nv);
                }
                Val::PArgs(_) => {
                    let nv = self.try_fresh(n, &Ty::User("process_args".into()))?;
                    h.pc.push(self.wf_pargs(nv.try_pargs()?));
                    h.env.set(n, nv);
                }
                Val::Arr(old_arr) => {
                    // G6 A1: an element type without a legacy array type (a Vec<X>, X not a parameter) is re-created as a Vec
                    let fty = match (&old_arr.elem, &old_arr.ety) {
                        (Elem::Abs, Some(e)) if e.owner.is_none() => Ty::Inst(GType::new("vec", vec![e.clone()])),
                        _ => old_arr.array_ty(),
                    };
                    let nv = self.try_fresh(n, &fty)?;
                    let mut a = nv.try_arr()?.clone();
                    if keep.contains(n) {
                        a.n = old_arr.n.clone();
                    } else {
                        h.pc.push(self.z.and(&[a.n.ge_i(0), a.n.le_i(MAX_LEN)]));
                    }
                    if a.elem == Elem::U8 {
                        h.pc.push(self.u8_dom(&a));
                    }
                    h.env.set(n, Val::Arr(a));
                }
                Val::Map(old_map) => {
                    let nv = self.try_fresh(n, &Ty::Inst(GType::new("hashmap", vec![old_map.kty.clone(), old_map.vty.clone()])))?;
                    let m = nv.try_map()?.clone();
                    h.pc.push(self.z.and(&[m.n.ge_i(0), m.n.le_i(MAX_LEN)]));
                    h.env.set(n, Val::Map(m));
                }
                Val::Int(_) => {
                    let nv = self.try_fresh(n, &Ty::I64)?;
                    let a = nv.try_ast()?.clone();
                    if n != "spent" || !self.has_cost() {
                        h.pc.push(in_range(&self.z, &a));
                    }
                    h.env.set(n, Val::Int(a));
                }
                Val::Bool(_) => {
                    let nv = self.try_fresh(n, &Ty::Bool)?;
                    h.env.set(n, nv);
                }
                Val::Dt(old_ast, ty) => {
                    // Set value: in `name!h{k}`, k uses the value before incrementing and is bumped afterwards (lines 701-702).
                    let nv = self.z.constant(&format!("{}!h{}", n, self.k), &old_ast.sort());
                    self.k += 1;
                    h.env.set(n, Val::Dt(nv.clone(), ty.clone()));
                    if let Some(w) = self.try_wf(&ty, &nv)? {
                        h.pc.push(w);
                    }
                }
                Val::Unit => return Err("havoc: None".to_string()),
            }
        }
        // A disjoint sibling write can havoc the root of a read-only iteration place.
        for i in 0..self.iter_places.len() {
            let place = self.iter_places[i].clone();
            let mut root = &place;
            while let Expr::FieldAccess(f) = root {
                root = &f.base;
            }
            if matches!(root, Expr::Var(v) if names.contains(&v.name)) {
                let saved = self.ev0(&place, &mut st.fork(None), &[], false)?;
                self.store_collection_place(&place, &mut h.env, saved)?;
            }
        }
        Ok(h)
    }

    /// For the prototype: accepts any order and sorts internally. On failure, returns only forks.
    pub fn havoc(&mut self, st: &State, names: &[&str]) -> State {
        let set: BTreeSet<String> = names.iter().map(|s| s.to_string()).collect();
        self.havoc_set(st, &set, &BTreeSet::new()).unwrap_or_else(|_| st.fork(None))
    }

    // ---------- quantifier pieces (lines 401-417; for the prototype) ----------
    /// Mid-Quant evaluation: evaluate lo/hi first, then `s.k += 1`, constant `var!q{k}`, qstack push. After evaluating the body, `quant_exit`.
    pub fn quant_enter(&mut self, var: &str) -> Ast {
        self.k += 1;
        let k = self.z.int_const(&format!("{}!q{}", var, self.k));
        self.qstack.push(k.clone());
        k
    }
    pub fn quant_exit(&mut self) {
        self.qstack.pop();
    }
    /// `ForAll([k], Implies(And(k >= lo, k < hi), body))` — no patterns.
    pub fn forall_q(&self, k: &Ast, lo: &Ast, hi: &Ast, body: &Ast) -> Ast {
        let rng = self.z.and(&[k.ge(lo), k.lt(hi)]);
        self.z.forall(std::slice::from_ref(k), &rng.implies(body), &[])
    }
    /// `Exists([k], And(And(k >= lo, k < hi), body))`.
    pub fn exists_q(&self, k: &Ast, lo: &Ast, hi: &Ast, body: &Ast) -> Ast {
        let rng = self.z.and(&[k.ge(lo), k.lt(hi)]);
        self.z.exists(std::slice::from_ref(k), &self.z.and(&[rng, body.clone()]), &[])
    }

    /// G2 A2: element domain of a U8 array`ForAll([k], Implies(And(k >= 0, k < n), And(A[k] >= 0, A[k] <= 255)))`.
    /// The bound variable has the fixed name `elem!k` (does not touch self.k). No patterns.
    pub fn u8_dom(&self, arr: &Arr) -> Ast {
        let k = self.z.int_const("elem!k");
        let rng = self.z.and(&[k.ge_i(0), k.lt(&arr.n)]);
        let sel = arr.a.select(&k);
        let body = self.z.and(&[sel.ge_i(0), sel.le_i(255)]);
        self.z.forall(std::slice::from_ref(&k), &rng.implies(&body), &[])
    }

    /// G6b: verify.py `i64_dom`: every element of a Vec<i64> returned by a trusted rust fn is an i64
    /// (`elem!k` bound variable, no patterns).
    pub fn i64_dom(&self, arr: &Arr) -> Ast {
        let k = self.z.int_const("elem!k");
        let rng = self.z.and(&[k.ge_i(0), k.lt(&arr.n)]);
        let body = in_range(&self.z, &arr.a.select(&k));
        self.z.forall(std::slice::from_ref(&k), &rng.implies(&body), &[])
    }

    /// G2 B1: is `i` a UTF-8 character boundary of the byte array `arr` (total function, no domain)?
    /// `Or(i == 0, i == n, And(i > 0, i < n, Or(b < 128, b >= 192)))`, `b = Select(arr.a, i)`.
    pub fn str_boundary(&self, arr: &Arr, i: &Ast) -> Ast {
        let z = &self.z;
        let b = arr.a.select(i);
        z.or(&[
            i.raw_eq(&z.int(0)),
            i.eq(&arr.n),
            z.and(&[i.gt_i(0), i.lt(&arr.n), z.or(&[b.lt_i(128), b.ge_i(192)])]),
        ])
    }

    // ---------- aggregates + lemmas (lines 545-602) ----------
    /// LG4 G-R03D-1 aggregate frame lemma (verify.py `agg_frame`). For each earlier aggregate of the same kind, arity
    /// and body shape: over the same range the two agree, or some index in the range has different bodies.
    /// True of any two aggregates; the witness `W` names that index.
    fn agg_frame(&mut self, kind: &str, f: &FuncDecl, tc: &Ast, k0: &Ast, ps: &[Ast]) {
        let z = self.z.clone();
        let shape = arr_shape(tc);
        let arity = 2 + ps.len();
        let mut new_lemmas = Vec::new();
        for (kind2, f2, tc2, shape2, arity2) in self.agg_shapes.iter() {
            if kind2 != kind || *arity2 != arity || *shape2 != shape {
                continue;
            }
            let x = z.int_const("aggf!x");
            let y = z.int_const("aggf!y");
            let os: Vec<Ast> = (0..ps.len()).map(|i| z.int_const(&format!("aggf!o{}", i))).collect();
            let int = z.int_sort();
            let dom: Vec<Sort> = (0..arity).map(|_| int.clone()).collect();
            let wf = FuncDecl::new(&z, &format!("aggf!w{}_{}", self.agg_shapes.len(), f2.name()), &dom, &int);
            let mut args = vec![x.clone(), y.clone()];
            args.extend(os.iter().cloned());
            let w = wf.apply(&args);
            let mut from = vec![k0.clone()];
            let mut to = vec![w.clone()];
            from.extend(ps.iter().cloned());
            to.extend(os.iter().cloned());
            let b1 = tc2.substitute(&from, &to);
            let b2 = tc.substitute(&from, &to);
            let a1 = f2.apply(&args);
            let a2 = f.apply(&args);
            let body = x.lt(&y).implies(&z.or(&[a1.eq(&a2), z.and(&[x.le(&w), w.lt(&y), b1.ne(&b2)])]));
            new_lemmas.push(z.forall_multi(&args, &body, &[a1, a2]));
        }
        self.lemmas.extend(new_lemmas);
        self.agg_shapes.push((kind.to_string(), f.clone(), tc.clone(), shape, arity));
    }

    /// Translates sum/count/max/min over lo<=k<hi into an uninterpreted function `F(lo, hi, *outer)` and unrolling lemmas.
    /// At call time, this aggregate's own variable `k` must already be gone from `qstack` (after `quant_exit`).
    pub fn aggregate(&mut self, kind: &str, k: &Ast, lo: &Ast, hi: &Ast, t: &Ast) -> Ast {
        let z = self.z.clone();
        let outer: Vec<Ast> = self.qstack.iter().filter(|c| t.occurs(c)).cloned().collect();
        let k0 = z.int_const("agg!k");
        let ps: Vec<Ast> = (0..outer.len()).map(|i| z.int_const(&format!("agg!p{}", i))).collect();

        let mut from = vec![k.clone()];
        let mut to = vec![k0.clone()];
        for (o, p) in outer.iter().zip(ps.iter()) {
            from.push(o.clone());
            to.push(p.clone());
        }
        let tc = t.substitute(&from, &to);
        let key = (kind.to_string(), tc.sexpr(), outer.len());

        let existing = self.aggs.iter().find(|(kk, _)| *kk == key).map(|(_, f)| f.clone());
        let f = match existing {
            Some(f) => f,
            None => {
                let int = z.int_sort();
                let dom: Vec<Sort> = (0..2 + outer.len()).map(|_| int.clone()).collect();
                let f = FuncDecl::new(&z, &format!("agg!{}{}", kind, self.aggs.len()), &dom, &int);
                self.aggs.push((key.clone(), f.clone()));
                self.agg_frame(kind, &f, &tc, &k0, &ps);
                f
            }
        };

        let body_at = |x: &Ast| -> Ast {
            let mut from = vec![k0.clone()];
            let mut to = vec![x.clone()];
            for (p, o) in ps.iter().zip(outer.iter()) {
                from.push(p.clone());
                to.push(o.clone());
            }
            tc.substitute(&from, &to)
        };
        let val = |x: &Ast| -> Ast {
            let b = body_at(x);
            if kind == "count" {
                z.ite(&b, &z.int(1), &z.int(0))
            } else {
                b
            }
        };
        // The aggregate of the smaller range with element x attached. `empty`: the smaller range is empty.
        let comb = |smaller: &Ast, x: &Ast, empty: &Ast| -> Ast {
            if kind == "sum" || kind == "count" {
                z.ite(empty, &z.int(0), smaller).add(&val(x))
            } else {
                let better = if kind == "max" { smaller.ge(&val(x)) } else { smaller.le(&val(x)) };
                z.ite(empty, &val(x), &z.ite(&better, smaller, &val(x)))
            }
        };
        let fa = |a: &Ast, b: &Ast| -> Ast {
            let mut args = vec![a.clone(), b.clone()];
            args.extend(outer.iter().cloned());
            f.apply(&args)
        };

        let fk = (kind.to_string(), f.name().to_string());
        if !self.lemma_fn_keys.contains(&fk) {
            // Empty range: sum/count is 0 (no new terms are created, so no matching loop)
            self.lemma_fn_keys.insert(fk);
            if kind == "sum" || kind == "count" {
                let x = z.int_const("agg!x");
                let y = z.int_const("agg!y");
                let os: Vec<Ast> = (0..outer.len()).map(|i| z.int_const(&format!("agg!o{}", i))).collect();
                let mut args = vec![x.clone(), y.clone()];
                args.extend(os.iter().cloned());
                let fapp = f.apply(&args);
                let mut bound = vec![x.clone(), y.clone()];
                bound.extend(os.iter().cloned());
                let body = y.le(&x).implies(&fapp.raw_eq(&z.int(0)));
                self.lemmas.push(z.forall(&bound, &body, &[fapp.clone()]));
            }
            if kind == "count" {
                // LG19 (R03L-G2): range split, count(lo, hi) == count(lo, m) + count(m, hi) for lo <= m <= hi. Every
                // trigger pairs two existing terms, and the instance only adds a term over their endpoints, so the
                // endpoint set never grows and there is no matching loop.
                let x = z.int_const("agg!x");
                let m = z.int_const("agg!m");
                let y = z.int_const("agg!y");
                let os: Vec<Ast> = (0..outer.len()).map(|i| z.int_const(&format!("agg!o{}", i))).collect();
                let ap = |a: &Ast, b: &Ast| -> Ast {
                    let mut args = vec![a.clone(), b.clone()];
                    args.extend(os.iter().cloned());
                    f.apply(&args)
                };
                let (fxy, fxm, fmy) = (ap(&x, &y), ap(&x, &m), ap(&m, &y));
                let mut bound = vec![x.clone(), m.clone(), y.clone()];
                bound.extend(os.iter().cloned());
                let body = z.and(&[x.le(&m), m.le(&y)]).implies(&fxy.eq(&fxm.add(&fmy)));
                self.split_lemmas.push(z.forall_groups(
                    &bound,
                    &body,
                    &[vec![&fxm, &fmy], vec![&fxy, &fxm], vec![&fxy, &fmy]],
                ));
            }
        }

        let app = fa(lo, hi);
        let mut facts: Vec<Ast> = Vec::new();
        if kind == "count" {
            facts.push(lo.lt(hi).implies(&z.and(&[app.ge_i(0), app.le(&hi.sub(lo))])));
        }
        let bound_lo = self.qstack.iter().any(|c| lo.occurs(c));
        let bound_hi = self.qstack.iter().any(|c| hi.occurs(c));
        // Unroll by moving only one end: so the quantified lemma does not match its own output again.
        if !bound_hi {
            let hm1 = hi.sub_i(1);
            facts.push(lo.lt(hi).implies(&app.eq(&comb(&fa(lo, &hm1), &hm1, &hm1.le(lo)))));
            facts.push(lo.le(hi).implies(&fa(lo, &hi.add_i(1)).eq(&comb(&app, hi, &hi.le(lo)))));
        }
        if !bound_lo {
            let lp1 = lo.add_i(1);
            facts.push(lo.lt(hi).implies(&app.eq(&comb(&fa(&lp1, hi), lo, &hi.le(&lp1)))));
            let lm1 = lo.sub_i(1);
            facts.push(lo.le(hi).implies(&fa(&lm1, hi).eq(&comb(&app, &lm1, &hi.le(lo)))));
        }
        if !facts.is_empty() {
            let mut l = z.and(&facts);
            let bs: Vec<Ast> = self
                .qstack
                .iter()
                .filter(|c| outer.iter().any(|o| c.same(o)) || lo.occurs(c) || hi.occurs(c))
                .cloned()
                .collect();
            if !bs.is_empty() {
                l = z.forall(&bs, &l, std::slice::from_ref(&app));
            }
            let lk = l.sexpr();
            if self.lemma_text_keys.insert(lk) {
                self.lemmas.push(l);
            }
        }
        app
    }

    // =========================================================================================
    // Expressions (`ev` at line 345)
    // =========================================================================================

    /// `ev` with `st.env` as the snapshot (the env=None call).
    fn ev0(&mut self, e: &Expr, st: &mut State, g: &[Ast], code: bool) -> R<Val> {
        // R-6g-1: nested inout calls update st in place. Their arrays occur nowhere
        // else in the statement, so other operands can still read the entry snapshot.
        let env = st.env.clone();
        self.ev(e, st, g, code, &env)
    }

    fn ev0_ast(&mut self, e: &Expr, st: &mut State, g: &[Ast], code: bool) -> R<Ast> {
        let v = self.ev0(e, st, g, code)?;
        Ok(v.try_ast()?.clone())
    }

    fn ev_ast(&mut self, e: &Expr, st: &mut State, g: &[Ast], code: bool, env: &Env) -> R<Ast> {
        let v = self.ev(e, st, g, code, env)?;
        Ok(v.try_ast()?.clone())
    }

    /// verify.py `ev(e, st, g, code, env)`. code=True: generates proof obligations and i64 semantics.
    pub(crate) fn ev(&mut self, e: &Expr, st: &mut State, g: &[Ast], code: bool, env: &Env) -> R<Val> {
        let model: &'a Model = self.model;
        match e {
            Expr::Spent(_) => Ok(st.env.get("spent").clone()),
            Expr::IntLit(x) => Ok(Val::Int(self.z.int_str(&norm_int(&x.value)))),
            Expr::BoolLit(x) => Ok(Val::Bool(self.z.bool_val(x.value))),
            Expr::StrLit(x) => {
                // G2 B1: constant byte array. No counters, obligations or assumptions.
                let mut a = self.z.const_array(&self.z.int_sort(), &self.z.int(0));
                for (i, b) in x.value.iter().enumerate() {
                    a = a.store(&self.z.int(i as i64), &self.z.int(*b as i64));
                }
                Ok(Val::Arr(Arr::new(a, self.z.int(x.value.len() as i64), "literal".to_string(), Elem::U8)))
            }
            Expr::Var(x) => env.try_get(&x.name).cloned().ok_or_else(|| format!("'{}'", x.name)),
            Expr::Unary(x) => {
                let v = self.ev_ast(&x.operand, st, g, code, env)?;
                if x.op == "!" {
                    return Ok(Val::Bool(v.not()));
                }
                if x.op == "~" {
                    return Ok(Val::Int(v.neg().sub(&self.z.int(1))));
                }
                if code {
                    let ob = in_range(&self.z, &v.neg());
                    self.prove_core(&st, g, &ob, pos(e), "overflow", "no overflow in unary -", None);
                }
                Ok(Val::Int(v.neg()))
            }
            Expr::Binary(x) => {
                let op = x.op.as_str();
                if op == "&&" || op == "||" || op == "==>" {
                    let l = self.ev_ast(&x.left, st, g, code, env)?;
                    let mut g2: Vec<Ast> = g.to_vec();
                    g2.push(if op != "||" { l.clone() } else { l.not() });
                    let r = self.ev_ast(&x.right, st, &g2, code, env)?;
                    let res = if op == "&&" {
                        self.z.and(&[l, r])
                    } else if op == "||" {
                        self.z.or(&[l, r])
                    } else {
                        l.implies(&r)
                    };
                    return Ok(Val::Bool(res));
                }
                let l = self.ev_ast(&x.left, st, g, code, env)?;
                let r = self.ev_ast(&x.right, st, g, code, env)?;
                if bit_operator(op) {
                    let shift = matches!(op, "<<" | ">>");
                    if code && shift {
                        let ob = self.z.and(&[r.ge_i(0), r.lt_i(64)]);
                        self.prove_core(st, g, &ob, pos(e), "shift_range",
                            &format!("shift count in 0..64 in `{}`", op), None);
                    }
                    if let Some(factor) = shift_factor(&x.right).filter(|_| shift) {
                        let factor = self.z.int_str(&factor.to_string());
                        let value = if op == "<<" { l.mul(&factor) } else { l.div(&factor) };
                        if code && op == "<<" {
                            self.prove_core(st, g, &in_range(&self.z, &value), pos(e),
                                "overflow", "no i64 overflow in `<<`", None);
                        }
                        return Ok(Val::Int(value));
                    }
                    let a = l.int2bv(64);
                    let k = r.int2bv(64);
                    let word = match op {
                        "&" => a.bvand(&k),
                        "|" => a.bvor(&k),
                        "^" => a.bvxor(&k),
                        "<<" => a.bvshl(&k),
                        _ => a.bvashr(&k),
                    };
                    if code && op == "<<" {
                        self.prove_core(st, g, &word.bvashr(&k).raw_eq(&a), pos(e),
                            "overflow", "no i64 overflow in `<<`", None);
                    }
                    let u = word.bv2int(false);
                    let value = self.z.ite(
                        &u.raw_lt(&self.z.int_str("9223372036854775808")),
                        &u, &u.sub(&self.z.int_str("18446744073709551616")),
                    );
                    if code {
                        self.assume(st, g, in_range(&self.z, &value));
                        let nonnegative = self.z.and(&[value.ge_i(0), value.le(&l)]);
                        match op {
                            "&" => {
                                self.assume(st, g, r.ge_i(0).implies(&self.z.and(&[value.ge_i(0), value.le(&r)])));
                                self.assume(st, g, l.ge_i(0).implies(&nonnegative));
                                let mask = low_mask_modulus(&x.right).map(|m| (&l, m))
                                    .or_else(|| low_mask_modulus(&x.left).map(|m| (&r, m)));
                                if let Some((arg, m)) = mask {
                                    self.assume(st, g, arg.ge_i(0).implies(&value.eq(&arg.modulo(&self.z.int(m)))));
                                }
                            }
                            "|" | "^" => {
                                let fact = self.z.and(&[l.ge_i(0), r.ge_i(0)])
                                    .implies(&self.z.and(&[value.ge_i(0), value.le(&l.add(&r))]));
                                self.assume(st, g, fact);
                            }
                            ">>" => {
                                self.assume(st, g, l.ge_i(0).implies(&nonnegative));
                                self.assume(st, g, l.lt_i(0).implies(&self.z.and(&[value.ge(&l), value.lt_i(0)])));
                            }
                            _ => {}
                        }
                        if shift {
                            self.assume(st, g, r.raw_eq(&self.z.int(0)).implies(&value.eq(&l)));
                        }
                    }
                    return Ok(Val::Int(value));
                }
                let res = match op {
                    "+" => l.add(&r),
                    "-" => l.sub(&r),
                    "*" => l.mul(&r),
                    "/" | "%" => {
                        if code {
                            let ob = self.z.and(&[r.gt_i(0), l.ge_i(0)]);
                            self.prove_core(&st, g, &ob, pos(e), "div", "divisor > 0 and dividend >= 0", None);
                        }
                        return Ok(Val::Int(if op == "/" { l.div(&r) } else { l.modulo(&r) }));
                    }
                    // Both sides are z3 expressions, so eq/ne/lt/.. follow the z3py reflection rule (flipped if the right side is a numeric literal)
                    "==" => return Ok(Val::Bool(l.eq(&r))),
                    "!=" => return Ok(Val::Bool(l.ne(&r))),
                    "<" => return Ok(Val::Bool(l.lt(&r))),
                    "<=" => return Ok(Val::Bool(l.le(&r))),
                    ">" => return Ok(Val::Bool(l.gt(&r))),
                    ">=" => return Ok(Val::Bool(l.ge(&r))),
                    other => return Err(format!("local variable 'res' referenced before assignment ({})", other)),
                };
                if code {
                    let ob = in_range(&self.z, &res);
                    self.prove_core(&st, g, &ob, pos(e), "overflow", &format!("no i64 overflow in `{}`", op), None);
                }
                Ok(Val::Int(res))
            }
            Expr::Index(x) => {
                let arr = self.ev(&x.array, st, g, code, env)?;
                let arr = arr.try_arr()?.clone();
                let i = self.ev_ast(&x.index, st, g, code, env)?;
                if code {
                    // `i >= 0` is with a Python int, `i < arr.n` is between z3 expressions (reflection rule)
                    let ob = self.z.and(&[i.ge_i(0), i.lt(&arr.n)]);
                    self.prove_core(
                        &st,
                        g,
                        &ob,
                        pos(e),
                        "index_oob",
                        &format!("index in bounds for {}[...]", arr.name),
                        None,
                    );
                }
                let v = arr.a.select(&i);
                // G2 A5: a bool array yields a Bool value without an in_range assumption
                if arr.elem == Elem::Bool {
                    return Ok(Val::Bool(v));
                }
                // G3: an abstract element has no numeric domain: no in_range assumption
                if arr.elem == Elem::Abs {
                    return Ok(Val::Dt(v, arr.elem_ty()));
                }
                if code {
                    let w = in_range(&self.z, &v);
                    self.assume(st, g, w);
                }
                Ok(Val::Int(v))
            }
            Expr::Ctor(x) => {
                // G3: the datatype of the (substituted) type of the constructor application; legacy = Opt / Res
                let cty = self.t(x.ty.as_deref());
                if x.name == "None" {
                    let o = model.opt(cty.as_ref())?;
                    return Ok(Val::Dt(o.none.clone(), cty.unwrap_or(Ty::Opt)));
                }
                let arg = x.arg.as_ref().ok_or_else(|| "None".to_string())?;
                let v = self.ev(arg, st, g, code, env)?;
                let v = self.field_put(&payload_ty(cty.as_ref(), x.name == "Err"), &v)?;
                match x.name.as_str() {
                    "Some" => {
                        let o = model.opt(cty.as_ref())?;
                        Ok(Val::Dt(o.some.apply1(&v), cty.unwrap_or(Ty::Opt)))
                    }
                    "Ok" | "Err" => {
                        let o = model.res(cty.as_ref())?;
                        let r = if x.name == "Ok" { o.ok.apply1(&v) } else { o.err.apply1(&v) };
                        Ok(Val::Dt(r, cty.unwrap_or(Ty::Res)))
                    }
                    other => Err(format!("'{}'", other)),
                }
            }
            Expr::Quant(x) => {
                let lo = self.ev_ast(&x.lo, st, g, false, env)?;
                let hi = self.ev_ast(&x.hi, st, g, false, env)?;
                self.k += 1;
                let k = self.z.int_const(&format!("{}!q{}", x.var, self.k));
                let mut env2 = env.clone();
                env2.set(&x.var, Val::Int(k.clone()));
                self.qstack.push(k.clone());
                let body = self.ev_ast(&x.body, st, g, false, &env2);
                // R-6-3: trigger terms, evaluated like the body (a term, no domain obligation)
                let mut trig: Vec<Ast> = Vec::new();
                let mut trig_err: Option<String> = None;
                if body.is_ok() {
                    for t in &x.triggers {
                        match self.ev_ast(t, st, g, false, &env2) {
                            Ok(a) => trig.push(a),
                            Err(er) => {
                                trig_err = Some(er);
                                break;
                            }
                        }
                    }
                }
                self.qstack.pop();
                let body = body?;
                if let Some(er) = trig_err {
                    return Err(er);
                }
                if matches!(x.kind.as_str(), "sum" | "count" | "max" | "min") {
                    return Ok(Val::Int(self.aggregate(&x.kind, &k, &lo, &hi, &body)));
                }
                let rng = self.z.and(&[k.ge(&lo), k.lt(&hi)]);
                if x.kind == "forall" {
                    let f = rng.implies(&body);
                    if trig.is_empty() {
                        Ok(Val::Bool(self.z.forall(std::slice::from_ref(&k), &f, &[])))
                    } else {
                        Ok(Val::Bool(self.z.forall_multi(std::slice::from_ref(&k), &f, &trig)))
                    }
                } else {
                    let f = self.z.and(&[rng, body]);
                    if trig.is_empty() {
                        Ok(Val::Bool(self.z.exists(std::slice::from_ref(&k), &f, &[])))
                    } else {
                        Ok(Val::Bool(self.z.exists_multi(std::slice::from_ref(&k), &f, &trig)))
                    }
                }
            }
            Expr::Call(x) => self.call(e, x, st, g, code, env),
            Expr::StructLit(x) => {
                if model.is_g9() {
                    // G9 A1: buffer fields store Buf values; recursive results expose their facts
                    let mut vals: Vec<Val> = Vec::new();
                    for fi in &x.fields {
                        vals.push(self.ev(&fi.value, st, g, code, env)?);
                    }
                    let sty = match self.t(x.ty.as_deref()) {
                        Some(t) => t,
                        None => Ty::User(x.name.clone()),
                    };
                    let mut asts: Vec<Ast> = Vec::new();
                    for ((_, ft), xv) in model.fields_of(&sty, None)?.iter().zip(vals.iter()) {
                        asts.push(self.field_put(ft, xv)?);
                    }
                    let res = model.struct_info(&sty)?.ctor.apply(&asts);
                    self.observe(st, &res, Some(&sty), false)?;
                    return Ok(Val::Dt(res, sty));
                }
                let mut vals: Vec<Ast> = Vec::new();
                for fi in &x.fields {
                    vals.push(self.ev_ast(&fi.value, st, g, code, env)?);
                }
                let sty = match self.t(x.ty.as_deref()) {
                    Some(t) => t,
                    None => Ty::User(x.name.clone()),
                };
                let sf = model.struct_info(&sty)?;
                Ok(Val::Dt(sf.ctor.apply(&vals), sty))
            }
            Expr::EnumCtor(x) => {
                if model.is_g9() {
                    let mut vals: Vec<Val> = Vec::new();
                    for a in &x.args {
                        vals.push(self.ev(a, st, g, code, env)?);
                    }
                    let ety = match self.t(x.ty.as_deref()) {
                        Some(t) => t,
                        None => Ty::User(x.enum_name.clone()),
                    };
                    let mut asts: Vec<Ast> = Vec::new();
                    for ((_, ft), xv) in model.fields_of(&ety, Some(&x.variant))?.iter().zip(vals.iter()) {
                        asts.push(self.field_put(ft, xv)?);
                    }
                    let res = model.variant_fns(&ety, &x.variant)?.ctor.apply(&asts);
                    self.observe(st, &res, Some(&ety), false)?;
                    return Ok(Val::Dt(res, ety));
                }
                let mut vals: Vec<Ast> = Vec::new();
                for a in &x.args {
                    vals.push(self.ev_ast(a, st, g, code, env)?);
                }
                let ety = match self.t(x.ty.as_deref()) {
                    Some(t) => t,
                    None => Ty::User(x.enum_name.clone()),
                };
                let vf = model.variant_fns(&ety, &x.variant)?;
                Ok(Val::Dt(vf.ctor.apply(&vals), ety))
            }
            Expr::FieldAccess(x) => {
                let v = self.ev(&x.base, st, g, code, env)?;
                let Val::Dt(va, sty) = &v else { return Err("field access on a non-struct value".to_string()) };
                let sf = model.struct_info(sty)?;
                let idx = model.field_index_of(sty, &x.field, None)?;
                let fty = model.fields_of(sty, None)?[idx].1.clone();
                if model.is_g9() {
                    // G9 A1: adapt a buffer field to an Arr; parent and nominal child are observed terms
                    let r = sf.accs[idx].apply1(va);
                    self.observe(st, va, Some(sty), false)?; // the parent's one-step wf/size equations give the field's own facts
                    let rv = self.field_val(&fty, r, &x.field)?;
                    if let Val::Dt(a, t) = &rv {
                        self.observe(st, a, Some(t), false)?;
                    }
                    return Ok(rv);
                }
                Val::from_field(&fty, sf.accs[idx].apply1(va))
            }
            Expr::IsVariant(x) => {
                let v = self.ev(&x.value, st, g, code, env)?;
                let ety = enum_ty(&x.enum_name, &v);
                let vf = model.variant_fns(&ety, &x.variant)?;
                Ok(Val::Bool(vf.rec.apply1(v.try_ast()?)))
            }
            // A move reads the old value. Field assignment replaces it after evaluating the call.
            Expr::Move(x) => {
                let mut v = env.try_get(&x.name).ok_or_else(|| format!("'{}'", x.name))?.clone();
                for field in &x.fields {
                    let Val::Dt(va, sty) = v else { return Err("field move on a non-struct value".to_string()) };
                    let sf = model.struct_info(&sty)?;
                    let idx = model.field_index_of(&sty, field, None)?;
                    let fty = &model.fields_of(&sty, None)?[idx].1;
                    self.observe(st, &va, Some(&sty), false)?;
                    v = self.field_val(fty, sf.accs[idx].apply1(&va), field)?;
                }
                Ok(v)
            }
            // G5 A0: never reaches the proof layer (the driver's A0 guard stops the program earlier)
            Expr::InoutArg(x) => self.ev(&x.expr, st, g, code, env),
            Expr::VariantField(x) => {
                let v = self.ev(&x.value, st, g, code, env)?;
                let ety = enum_ty(&x.enum_name, &v);
                let vf = model.variant_fns(&ety, &x.variant)?;
                let idx = model.field_index_of(&ety, &x.field, Some(&x.variant))?;
                let fty = model.fields_of(&ety, Some(&x.variant))?[idx].1.clone();
                if model.is_g9() {
                    let r = vf.accs[idx].apply1(v.try_ast()?);
                    self.observe(st, v.try_ast()?, Some(&ety), false)?;
                    let rv = self.field_val(&fty, r, &x.field)?;
                    if let Val::Dt(a, t) = &rv {
                        self.observe(st, a, Some(t), false)?;
                    }
                    return Ok(rv);
                }
                Val::from_field(&fty, vf.accs[idx].apply1(v.try_ast()?))
            }
        }
    }

    /// verify.py `call` (line 604).
    fn call(&mut self, e: &Expr, x: &ast::Call, st: &mut State, g: &[Ast], code: bool, env: &Env) -> R<Val> {
        let n = x.name.as_str();
        if let Some((param, _)) = &x.old {
            // G5 A1: old(a) is the frozen entry array. A callee contract being instantiated at a call site supplies its
            // own pre-call values under ("old", name) in its environment; otherwise it is this function's snapshot.
            let key = old_key(param);
            return match env.try_get(&key) {
                Some(v) => Ok(v.clone()),
                None => self.snap.try_get(param).cloned().ok_or_else(|| format!("'{}'", param)),
            };
        }
        let mut args: Vec<Val> = Vec::new();
        for a in &x.args {
            args.push(self.ev(a, st, g, code, env)?);
        }
        let arg = |i: usize| -> R<&Val> { args.get(i).ok_or_else(|| "tuple index out of range".to_string()) };
        match n {
            "len" => {
                // G6 A1: `len(c)` is `c.n` for an array, a Vec, a String and a HashMap alike
                return Ok(Val::Int(match arg(0)? {
                    Val::Map(m) => m.n.clone(),
                    other => other.try_arr()?.n.clone(),
                }));
            }
            "size" => {
                // G9 A1: contract-only structural size of a non-generic nominal value
                let (a, sty) = match arg(0)? {
                    Val::Dt(a, t) => (a.clone(), t.clone()),
                    _ => return Err("'NoneType' object has no attribute 'name'".to_string()),
                };
                self.observe(st, &a, Some(&sty), true)?;
                return Ok(Val::Int(self.model.size_fn(sty.name())?.apply1(&a)));
            }
            "alloc" => {
                let a0 = arg(0)?.try_ast()?.clone();
                let a1 = arg(1)?.try_ast()?.clone();
                let ob = self.z.and(&[a0.ge_i(0), a0.le_i(MAX_LEN)]);
                self.prove_core(&st, g, &ob, pos(e), "alloc_negative", "alloc length in 0..2^40", None);
                // the element kind comes from the unsubstituted type name (verify.py `tname`), then a structured [T] overrides it
                let tname: Option<String> =
                    x.ty.as_deref().and_then(|k| GType::try_from_key(k).ok()).map(|g| g.name);
                let mut aelem = match tname.as_deref() {
                    Some("array_u8") => Elem::U8,
                    Some("array_bool") => Elem::Bool, // G2 A5
                    _ => Elem::I64,
                };
                let mut aety: Option<GType> = None;
                if let Some(Ty::GArray(el)) = self.t(x.ty.as_deref()) {
                    // G3: alloc of a [T]
                    aelem = if el.owner.is_some() {
                        Elem::Abs
                    } else if el == GType::bool() {
                        Elem::Bool
                    } else if el == GType::u8() {
                        Elem::U8
                    } else {
                        Elem::I64
                    };
                    if el.owner.is_some() {
                        aety = Some(el);
                    }
                }
                if aelem == Elem::U8 {
                    let ob = self.z.and(&[a1.ge_i(0), a1.le_i(255)]);
                    self.prove_core(
                        &st,
                        g,
                        &ob,
                        pos(e),
                        "elem_range",
                        "u8 element range 0..=255 for the alloc fill value",
                        None,
                    );
                }
                self.spend(st, g, self.z.int(1).add(&a0));
                self.k += 1;
                let ka = self.z.const_array(&self.z.int_sort(), &a1);
                return Ok(Val::Arr(match aety {
                    Some(ety) => Arr::abs(ka, a0, "alloc".to_string(), ety),
                    None => Arr::new(ka, a0, "alloc".to_string(), aelem),
                }));
            }
            "copy" => {
                let a = arg(0)?.try_arr()?;
                self.spend(st, g, self.z.int(1).add(&a.n));
                return Ok(Val::Arr(a.like(a.a.clone(), a.n.clone(), "copy")));
            }
            "truncate" => {
                // the first m elements of a, as a fresh array of length m
                let av = arg(0)?.try_arr()?.clone();
                let m = arg(1)?.try_ast()?.clone();
                let ob = self.z.and(&[m.ge_i(0), m.le(&av.n)]);
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "index_oob",
                    &format!("truncate length 0 <= m <= len({})", av.name),
                    None,
                );
                self.spend(st, g, self.z.int(1).add(&m));
                return Ok(Val::Arr(av.like(av.a.clone(), m, "truncate")));
            }
            "resize" => {
                // P05G: a grown to length m; k < len(a) keeps a[k], len(a) <= k < m is v
                let av = arg(0)?.try_arr()?.clone();
                let m = arg(1)?.try_ast()?.clone();
                let v = arg(2)?.try_ast()?.clone();
                let ob = self.z.and(&[av.n.le(&m), m.le_i(MAX_LEN)]);
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "index_oob",
                    &format!("resize length len({}) <= n <= 2^40", av.name),
                    None,
                );
                if av.elem == Elem::U8 {
                    let ob = self.z.and(&[v.ge_i(0), v.le_i(255)]);
                    self.prove_core(
                        &st,
                        g,
                        &ob,
                        pos(e),
                        "elem_range",
                        "u8 element range 0..=255 for the resize fill value",
                        None,
                    );
                }
                self.spend(st, g, self.z.int(1).add(&m));
                let k = self.z.int_const("resize_k");
                let body = self.z.ite(&k.lt(&av.n), &av.a.select(&k), &v);
                let lam = self.z.lambda(&[k], &body);
                return Ok(Val::Arr(av.like(lam, m, "resize")));
            }
            "is_char_boundary" => {
                // G2 B1: no obligations or assumptions.
                let sv = arg(0)?.try_arr()?;
                let i = arg(1)?.try_ast()?;
                return Ok(Val::Bool(self.str_boundary(sv, i)));
            }
            "eq_range" => {
                // P16: five bounds obligations in order, then a fresh bool r with r == forall k in 0..n: a[i+k] == b[j+k] assumed on the call's path (the arrays as they are at the call)
                let av = arg(0)?.try_arr()?.clone();
                let i = arg(1)?.try_ast()?.clone();
                let bv = arg(2)?.try_arr()?.clone();
                let j = arg(3)?.try_ast()?.clone();
                let cnt = arg(4)?.try_ast()?.clone();
                if code {
                    let obs: Vec<(Ast, String)> = vec![
                        (i.ge_i(0), format!("eq_range start 0 <= i for {}", av.name)),
                        (j.ge_i(0), format!("eq_range start 0 <= j for {}", bv.name)),
                        (cnt.ge_i(0), "eq_range length 0 <= n".to_string()),
                        (i.add(&cnt).le(&av.n), format!("eq_range i + n <= len({})", av.name)),
                        (j.add(&cnt).le(&bv.n), format!("eq_range j + n <= len({})", bv.name)),
                    ];
                    for (ob, msg) in obs {
                        self.prove_core(&st, g, &ob, pos(e), "index_oob", &msg, None);
                    }
                }
                let r = self.fresh("eq_range", &Ty::Bool);
                let r_ast = r.ast().clone();
                self.k += 1;
                let k = self.z.int_const(&format!("eq_range!q{}", self.k));
                let rng = self.z.and(&[k.ge_i(0), k.lt(&cnt)]);
                let body = av.a.select(&i.add(&k)).eq(&bv.a.select(&j.add(&k)));
                let fa = self.z.forall(std::slice::from_ref(&k), &rng.implies(&body), &[]);
                self.assume(st, g, r_ast.eq(&fa));
                return Ok(r);
            }
            "copy_range" | "copy_within" => {
                let dv = arg(0)?.try_arr()?.clone();
                let i = arg(1)?.try_ast()?.clone();
                let (sv, j, cnt, dst, src) = if n == "copy_within" {
                    (dv.clone(), arg(2)?.try_ast()?.clone(), arg(3)?.try_ast()?.clone(), "dst", "src")
                } else {
                    (arg(2)?.try_arr()?.clone(), arg(3)?.try_ast()?.clone(), arg(4)?.try_ast()?.clone(), "i", "j")
                };
                let obs = [
                    (i.ge_i(0), format!("{n} start 0 <= {dst} for {}", dv.name)),
                    (j.ge_i(0), format!("{n} start 0 <= {src} for {}", sv.name)),
                    (cnt.ge_i(0), format!("{n} length 0 <= n")),
                    (i.add(&cnt).le(&dv.n), format!("{n} {dst} + n <= len({})", dv.name)),
                    (j.add(&cnt).le(&sv.n), format!("{n} {src} + n <= len({})", sv.name)),
                ];
                for (ob, msg) in obs {
                    self.prove_core(&st, g, &ob, pos(e), "index_oob", &msg, None);
                }
                // a definitional (array-theory) update, no quantified assumption: the content fact, the frame fact and
                // len(dst') == len(dst) hold by construction
                let p = self.z.int_const(&format!("{n}!p"));
                let inr = self.z.and(&[i.le(&p), p.lt(&i.add(&cnt))]);
                let body = self.z.ite(&inr, &sv.a.select(&j.add(&p).sub(&i)), &dv.a.select(&p));
                let lam = self.z.lambda(&[p], &body);
                let Expr::InoutArg(dst_place) = &x.args[0] else {
                    return Err("copy destination is not an inout place".into());
                };
                let name = match dst_place.expr.as_ref() {
                    Expr::Var(v) => &v.name,
                    _ => &dv.name,
                };
                let nv = dv.like(lam, dv.n.clone(), name);
                if nv.elem == Elem::U8 {
                    // both operands are in the byte domain, so the result is too
                    let dom = self.u8_dom(&nv);
                    self.assume(st, g, dom);
                }
                self.store_collection_place(&dst_place.expr, &mut st.env, Val::Arr(nv))?;
                self.post_refine(e, &[0], st, g)?;
                return Ok(Val::Unit);
            }
            "load_be" | "load_le" => {
                let av = arg(0)?.try_arr()?.clone();
                let i = arg(1)?.try_ast()?.clone();
                let nb = lit_width(&x.args[2]);
                if code {
                    let ob = self.z.and(&[i.ge_i(0), i.add_i(nb).le(&av.n)]);
                    self.prove_core(
                        &st,
                        g,
                        &ob,
                        pos(e),
                        "index_oob",
                        &format!("word in bounds for {}[i..i+{}]", av.name, nb),
                        None,
                    );
                }
                let offset = |k| if n == "load_le" { nb - 1 - k } else { k };
                let mut cat = av.a.select(&i.add_i(offset(0))).int2bv(8);
                for k in 1..nb {
                    let b = av.a.select(&i.add_i(offset(k))).int2bv(8);
                    cat = cat.concat(&b);
                }
                let u = cat.bv2int(false);
                let r = if nb == 8 {
                    self.z.ite(
                        &u.raw_lt(&self.z.int_str("9223372036854775808")),
                        &u,
                        &u.sub(&self.z.int_str("18446744073709551616")),
                    )
                } else {
                    u
                };
                return Ok(Val::Int(r));
            }
            "store_be" | "store_le" => {
                let av = arg(0)?.try_arr()?.clone();
                let i = arg(1)?.try_ast()?.clone();
                let w = arg(2)?.try_ast()?.clone();
                let nb = lit_width(&x.args[3]);
                let ob = self.z.and(&[i.ge_i(0), i.add_i(nb).le(&av.n)]);
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "index_oob",
                    &format!("word in bounds for {}[i..i+{}]", av.name, nb),
                    None,
                );
                let wb = w.int2bv(64);
                let mut arr = av.a.clone();
                for k in 0..nb {
                    let sh = (8 * if n == "store_le" { k } else { nb - 1 - k }) as u32;
                    let byte = wb.extract(sh + 7, sh).bv2int(false);
                    arr = arr.store(&i.add_i(k), &byte);
                }
                let Expr::InoutArg(dst) = &x.args[0] else {
                    return Err("store destination is not an inout place".into());
                };
                let name = match dst.expr.as_ref() {
                    Expr::Var(v) => &v.name,
                    _ => &av.name,
                };
                let value = Val::Arr(av.like(arr, av.n.clone(), name));
                self.store_collection_place(&dst.expr, &mut st.env, value)?;
                self.post_refine(e, &[0], st, g)?;
                return Ok(Val::Unit);
            }
            _ if is_bit_z3(n) => {
                // P2: exact 64-bit two's-complement result as a signed Int (Python BIT_Z3)
                let x0 = arg(0)?.try_ast()?.clone();
                let y0 = arg(1)?.try_ast()?.clone();
                let bx = x0.int2bv(64);
                let by = y0.int2bv(64);
                let u = match n {
                    "band" => bx.bvand(&by),
                    "bor" => bx.bvor(&by),
                    "wrapping_shl" => bx.bvshl(&by),
                    "ushr" => bx.bvlshr(&by),
                    "wrapping_add" => bx.bvadd(&by),
                    "wrapping_mul" => bx.bvmul(&by),
                    _ => bx.bvxor(&by),
                }
                .bv2int(false);
                let r = self.z.ite(
                    &u.raw_lt(&self.z.int_str("9223372036854775808")),
                    &u,
                    &u.sub(&self.z.int_str("18446744073709551616")),
                );
                if code && (n == "wrapping_shl" || n == "ushr") {
                    // shift count must be in 0..64
                    let ob = self.z.and(&[y0.ge_i(0), y0.lt_i(64)]);
                    self.prove_core(&st, g, &ob, pos(e), "shift_range", "shift count in 0..64", None);
                }
                if code {
                    let f = self.z.and(&[r.ge_i(i64::MIN), r.le_i(i64::MAX)]);
                    self.assume(st, g, f);
                    if n == "band" {
                        let f = y0.ge_i(0).implies(&self.z.and(&[r.ge_i(0), r.le(&y0)]));
                        self.assume(st, g, f);
                        let f = x0.ge_i(0).implies(&self.z.and(&[r.ge_i(0), r.le(&x0)]));
                        self.assume(st, g, f);
                    } else if n == "wrapping_shl" {
                        let f = y0.raw_eq(&self.z.int(0)).implies(&r.eq(&x0));
                        self.assume(st, g, f);
                    } else if n == "ushr" {
                        let f = y0.raw_eq(&self.z.int(0)).implies(&r.eq(&x0));
                        self.assume(st, g, f);
                        let f = x0.ge_i(0).implies(&self.z.and(&[r.ge_i(0), r.le(&x0)]));
                        self.assume(st, g, f);
                        let f = y0.gt_i(0).implies(&r.ge_i(0));
                        self.assume(st, g, f);
                        // literal shift count c with 0 < c < 64 (syntactically an int literal in the source)
                        let c: Option<i128> = match &x.args[1] {
                            Expr::IntLit(l) => l.value.parse::<i128>().ok(),
                            _ => None,
                        };
                        if let Some(c) = c.filter(|c| *c > 0 && *c < 64) {
                            let f = r.raw_le(&self.z.int_str(&((1i128 << (64 - c)) - 1).to_string()));
                            self.assume(st, g, f);
                            let wide = self.z.ite(&x0.lt_i(0), &x0.add(&self.z.int_str("18446744073709551616")), &x0);
                            let f = r.eq(&wide.div(&self.z.int_str(&(1i128 << c).to_string())));
                            self.assume(st, g, f);
                        }
                    } else if n == "bor" || n == "bxor" {
                        // P2 batch 3: wrapping_add / wrapping_mul get no extra facts
                        let f = self
                            .z
                            .and(&[x0.ge_i(0), y0.ge_i(0)])
                            .implies(&self.z.and(&[r.ge_i(0), r.le(&x0.add(&y0))]));
                        self.assume(st, g, f);
                    }
                }
                return Ok(Val::Int(r));
            }
            "utf8_valid" => {
                let b = arg(0)?.try_arr()?;
                return Ok(Val::Bool(self.model.utf8_valid(&b.n, &b.a)));
            }
            "str_from_utf8" => {
                let b = arg(0)?.try_arr()?;
                let ob = self.model.utf8_valid(&b.n, &b.a);
                self.prove_core(&st, g, &ob, pos(e), "utf8_invalid",
                    &format!("valid UTF-8 bytes in {}", b.name), None);
                let res_v = self.try_fresh("str_from_utf8", &Ty::Str)?;
                let res = res_v.try_arr()?;
                self.assume(st, g, res.n.eq(&b.n));
                let k = self.z.int_const("str!k");
                let rng = self.z.and(&[k.ge_i(0), k.lt(&res.n)]);
                let body = res.a.select(&k).eq(&b.a.select(&k));
                let fa = self.z.forall(std::slice::from_ref(&k), &rng.implies(&body), &[]);
                self.assume(st, g, fa);
                return Ok(res_v);
            }
            "subview" => {
                let base = arg(0)?.try_arr()?;
                let lo = arg(1)?.try_ast()?;
                let hi = arg(2)?.try_ast()?;
                let ob = self.z.and(&[lo.ge_i(0), lo.le(hi), hi.le(&base.n)]);
                self.prove_core(st, g, &ob, pos(e), "index_oob",
                    &format!("subview range 0 <= lo <= hi <= len({})", base.name), None);
                let p = self.z.int_const("subview!p");
                let body = base.a.select(&lo.add(&p));
                let slice = self.z.lambda(&[p], &body);
                return Ok(Val::Arr(base.like(slice, hi.sub(lo), "subview")));
            }
            "substr" => {
                // G2 B1: range → bounds → new result → length assumption → byte-relation assumption.
                let sv = arg(0)?.try_arr()?.clone();
                let lo = arg(1)?.try_ast()?.clone();
                let hi = arg(2)?.try_ast()?.clone();
                let ob = self.z.and(&[lo.ge_i(0), lo.le(&hi), hi.le(&sv.n)]);
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "string_slice_range",
                    &format!("substr range 0 <= lo <= hi <= len({})", sv.name),
                    None,
                );
                let ob = self.z.and(&[self.str_boundary(&sv, &lo), self.str_boundary(&sv, &hi)]);
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "utf8_boundary",
                    &format!("substr cuts of {} at UTF-8 character boundaries", sv.name),
                    None,
                );
                let res_v = self.try_fresh("substr", &Ty::Str)?;
                let res = res_v.try_arr()?.clone();
                let len_fact = res.n.eq(&hi.sub(&lo));
                self.assume(st, g, len_fact);
                let k = self.z.int_const("str!k");
                let rng = self.z.and(&[k.ge_i(0), k.lt(&res.n)]);
                let body = res.a.select(&k).eq(&sv.a.select(&lo.add(&k)));
                let fa = self.z.forall(std::slice::from_ref(&k), &rng.implies(&body), &[]);
                self.assume(st, g, fa);
                return Ok(Val::Arr(res));
            }
            // G6 A1 (G6_SPEC 11.3): the standard catalog (unless the program declares a function of that name)
            _ if pruv_core::foreign::is_catalog(n) && !self.funcs.contains_key(n) => {
                return self.catalog_call(e, x, &args, st, g, code);
            }
            "print" => return Ok(Val::Unit),
            // G3: the datatype of the argument's own type (the legacy one for Option<i64> / Result<i64>)
            "is_some" | "some_val" => {
                let (a, ty) = dt_arg(arg(0)?)?;
                let o = self.model.opt(Some(ty))?;
                return if n == "is_some" {
                    Ok(Val::Bool(o.is_some.apply1(a)))
                } else {
                    Val::from_field(&payload_ty(Some(ty), false), o.sval.apply1(a))
                };
            }
            "is_ok" | "ok_val" | "err_val" => {
                let (a, ty) = dt_arg(arg(0)?)?;
                let o = self.model.res(Some(ty))?;
                return match n {
                    "is_ok" => Ok(Val::Bool(o.is_ok.apply1(a))),
                    "ok_val" => Val::from_field(&payload_ty(Some(ty), false), o.oval.apply1(a)),
                    _ => self.field_val(&payload_ty(Some(ty), true), o.eval.apply1(a), n),
                };
            }
            _ => {}
        }
        let f: &'a ast::Function = match self.funcs.get(n) {
            Some(f) => f,
            None => return Err(format!("'{}'", n)),
        };
        if f.opaque && !code {
            // P13o: a contract application of an opaque function is its symbol
            return Ok(Val::Bool(self.opq(f, &args, st)?));
        }
        let mut cenv = Env::default();
        for (p, a) in f.params.iter().zip(args.iter()) {
            cenv.set(&p.name, a.clone());
        }
        // G5 A1 (5.2 step 2): the pre-call values freeze as the callee's old bindings (scoped to this contract evaluation:
        // the caller's own snapshot map is never touched)
        for (p, a) in f.params.iter().zip(args.iter()) {
            if p.mode_pos.is_some() {
                cenv.set(&old_key(&p.name), a.clone());
            }
        }
        // G3 A2 (4.4): typed re-interpretation of the callee contract under the checker's recorded substitution
        let theta: Mapping = match &x.inst {
            Some(i) => i.subst.iter().map(|(o, idx, t)| ((o.clone(), *idx), t.clone())).collect(),
            None => Mapping::new(),
        };
        let saved = std::mem::replace(&mut self.theta, theta);
        let r = self.call_contract(e, f, st, g, code, cenv);
        self.theta = saved;
        r
    }

    /// P13o (appendix P13o.1, verify.py `opq`): the application of the opaque function f to the argument values: the
    /// uninterpreted `opq!F` over the argument encodings (an array is its contents and its length). Under a quantifier
    /// whose bound variable reaches an argument the application is expanded in place instead (a reveal equation cannot
    /// mention a bound variable). A new application is recorded on the path; if F is revealed on the path its equation
    /// is appended to pc at once.
    fn opq(&mut self, f: &'a ast::Function, args: &[Val], st: &mut State) -> R<Ast> {
        let parts = opq_parts(args)?;
        if !self.qstack.is_empty() && parts.iter().any(|t| self.qstack.iter().any(|c| t.occurs(c))) {
            return self.opq_body(f, args, st);
        }
        if !self.opq_fns.contains_key(&f.name) {
            let dom: Vec<Sort> = parts.iter().map(|t| t.sort()).collect();
            let fd = self.z.func_decl(&format!("opq!{}", f.name), &dom, &self.z.bool_sort());
            self.opq_fns.insert(f.name.clone(), fd);
        }
        let app = self.opq_fns[&f.name].apply(&parts);
        let i = app.id();
        if !st.app_ids.contains(&i) {
            std::rc::Rc::make_mut(&mut st.apps).push((f.name.clone(), app.clone(), args.to_vec()));
            std::rc::Rc::make_mut(&mut st.app_ids).insert(i);
            if st.rev.contains(&f.name) {
                let b = self.opq_body(f, args, st)?;
                st.pc.push(app.eq(&b));
            }
        }
        Ok(app)
    }

    /// P13o: the body of the opaque f over the argument values (its contract template; opaque calls in it stay
    /// symbols, so a reveal does not cascade).
    fn opq_body(&mut self, f: &'a ast::Function, args: &[Val], st: &mut State) -> R<Ast> {
        let xa = f.x_auto.as_ref().ok_or_else(|| format!("opaque {} has no contract expansion", f.name))?;
        let mut cenv = Env::default();
        for (p, a) in f.params.iter().zip(args.iter()) {
            cenv.set(&p.name, a.clone());
        }
        self.ev_ast(xa, st, &[], false, &cenv)
    }

    /// P13o: `reveal F;` makes F transparent on the rest of this path: the equation of every application already
    /// met, in the order met; later applications add theirs when met (`opq`).
    fn reveal(&mut self, name: &str, st: &State) -> R<State> {
        let mut st = st.fork(None);
        if st.rev.contains(name) {
            return Ok(st);
        }
        std::rc::Rc::make_mut(&mut st.rev).insert(name.to_string());
        let apps = std::rc::Rc::clone(&st.apps);
        for (fname, app, args) in apps.iter() {
            if fname == name {
                let f: &'a ast::Function = self.funcs[fname.as_str()];
                let b = self.opq_body(f, args, &mut st)?;
                st.pc.push(app.eq(&b));
            }
        }
        Ok(st)
    }

    /// P13o: the opaque functions applied on this path and not revealed, sorted.
    pub(crate) fn hidden(&self, st: &State) -> Vec<String> {
        let s: BTreeSet<&String> = st.apps.iter().map(|(n, _, _)| n).filter(|n| !st.rev.contains(*n)).collect();
        s.into_iter().cloned().collect()
    }

    /// The part of `call` that runs under the callee's substitution (verify.py `try:` body): requires, fresh result, ensures.
    fn call_contract(&mut self, e: &Expr, f: &'a ast::Function, st: &mut State, g: &[Ast], code: bool, mut cenv: Env) -> R<Val> {
        let n = f.name.as_str();
        if code {
            // LG16: the synthesized struct-invariant requires come before the user's
            for (i, ic) in self.inv_clauses(f, false).into_iter().enumerate() {
                let ob = self.ev_ast(&ic.expr, st, g, false, &cenv)?;
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "struct_invariant",
                    &format!("invariant of {} (line {}) for parameter {} of {}", ic.sname, pruv_core::diagnostic::plain_line(ic.line), ic.who, n),
                    Some(&i.to_string()),
                );
            }
            // The call site does not prove the callee's requires domain (the callee's own run() already proves it).
            for r in &f.requires {
                let ob = self.ev_ast(r, st, g, false, &cenv)?;
                self.prove_core(
                    &st,
                    g,
                    &ob,
                    pos(e),
                    "call_requires",
                    &format!("precondition of {} (line {})", n, pruv_core::diagnostic::plain_line(r.line())),
                    None,
                );
            }
            // G9 A1: only a call inside the caller's own recursive SCC
            self.termination(e, f, st, g, &cenv)?;
            self.call_cost(e, f, st, g, &cenv)?;
        }
        let ios: Vec<usize> = f.params.iter().enumerate().filter(|(_, p)| p.mode_pos.is_some()).map(|(i, _)| i).collect();
        if !ios.is_empty() {
            self.inout_finals(e, f, &ios, &mut cenv, st, g)?; // G5 A1 (5.2 step 5): fresh same-length contents
        }
        let ret = Ty::from_name(&f.ret);
        if ret == Ty::Unit {
            // G6 A1 (11.4): a trusted unit result still assumes its ensures; G9 A1: so does a member of a recursive SCC, after
            // the termination obligations above (also when called from outside the SCC).
            // G4 A1 (G4_SPEC 5): in explicit visibility mode every ordinary unit call assumes its ensures too.
            // G5 A1: every inout call applies its ensures, in legacy single-file mode too
            if !ios.is_empty() || f.foreign.is_some() || self.scc_id.contains_key(n) || self.explicit() {
                // (legacy ordinary unit functions keep the legacy behaviour)
                for ic in self.inv_clauses(f, true) {
                    let v = self.ev_ast(&ic.expr, st, g, false, &cenv)?; // LG16
                    self.assume(st, g, v);
                }
                for en in &f.ensures {
                    let v = self.ev_ast(en, st, g, false, &cenv)?;
                    self.assume(st, g, v);
                }
            }
            if !ios.is_empty() {
                self.post_refine(e, &ios, st, g)?;
            }
            return Ok(Val::Unit);
        }
        let rty = if self.theta.is_empty() {
            ret
        } else {
            Ty::from_gtype(&gtype::substitute(&ret.to_gtype(), &self.theta))
        };
        let res = self.try_fresh(&format!("ret_{}", n), &rty)?;
        if let Val::Arr(ra) = &res {
            let w = self.z.and(&[ra.n.ge_i(0), ra.n.le_i(MAX_LEN)]);
            self.assume(st, g, w);
            if ra.elem == Elem::U8 {
                let d = self.u8_dom(ra);
                self.assume(st, g, d);
            } else if f.foreign.is_some() && is_vec_i64(&rty) {
                // G6b: a fresh foreign Vec<i64>
                let d = self.i64_dom(ra);
                self.assume(st, g, d);
            }
        } else if let Val::File(f) = &res {
            self.assume(st, g, self.wf_file(f));
        } else if let Val::Proc(p) = &res {
            self.assume(st, g, self.wf_proc(p));
        } else if let Val::Sock(p) = &res {
            self.assume(st, g, self.wf_sock(p));
        } else if let Val::Loop(p) = &res {
            self.assume(st, g, self.wf_loop(p));
        } else if let Val::Lis(l) = &res {
            self.assume(st, g, self.wf_lis(l));
        } else if let Val::PArgs(a) = &res {
            self.assume(st, g, self.wf_pargs(a));
        } else if let Val::Map(rm) = &res {
            // G6 A1
            let w = self.z.and(&[rm.n.ge_i(0), rm.n.le_i(MAX_LEN)]);
            self.assume(st, g, w);
        } else if let Some(w) = self.try_wf(&rty, res.try_ast()?)? {
            self.assume(st, g, w);
        }
        // P07: the callee's body is its contract expansion (no requires, so its code proof covers every argument):
        // the result equals the template over the arguments. Evaluated before "result" is bound, so a parameter
        // named result keeps its argument value.
        let xv = match (code, &f.x_auto) {
            // P13o: the result is the symbol; its body is known only where F is revealed
            (true, _) if f.opaque => Some(self.opq(f, &args_of(&cenv, f), st)?),
            (true, Some(xa)) => Some(self.ev_ast(xa, st, g, false, &cenv)?),
            _ => None,
        };
        cenv.set("result", res.clone());
        for ic in self.inv_clauses(f, true) {
            let v = self.ev_ast(&ic.expr, st, g, false, &cenv)?; // LG16
            self.assume(st, g, v);
        }
        for en in &f.ensures {
            let v = self.ev_ast(en, st, g, false, &cenv)?;
            self.assume(st, g, v);
        }
        if let Some(v) = xv {
            let eq = res.try_ast()?.eq(&v);
            self.assume(st, g, eq);
        }
        if !ios.is_empty() {
            self.post_refine(e, &ios, st, g)?;
        }
        Ok(res)
    }

    /// verify.py `par` (G7 A1): entry actuals and requires, all finals, joined ensures,
    /// then refinements checked using only the corresponding branch's ensures.
    fn par(&mut self, p: &ast::Par, st: &State) -> R<State> {
        let mut st = st.fork(None);
        let mut branches = Vec::new();
        for e in &p.calls {
            let c = match e {
                Expr::Call(c) => c,
                _ => return Err("par: not a call".into()),
            };
            let f: &'a ast::Function = self.funcs.get(&c.name).copied().ok_or_else(|| format!("'{}'", c.name))?;
            let mut args = Vec::new();
            for a in &c.args {
                args.push(self.ev0(a, &mut st, &[], true)?);
            }
            let mut cenv = Env::default();
            for (pm, a) in f.params.iter().zip(&args) {
                cenv.set(&pm.name, a.clone());
            }
            let ios: Vec<usize> = f.params.iter().enumerate().filter(|(_, pm)| pm.mode_pos.is_some()).map(|(i, _)| i).collect();
            for &i in &ios {
                cenv.set(&old_key(&f.params[i].name), args[i].clone());
            }
            let theta: Mapping = match &c.inst {
                Some(i) => i.subst.iter().map(|(o, idx, t)| ((o.clone(), *idx), t.clone())).collect(),
                None => Mapping::new(),
            };
            branches.push((e, f, cenv, ios, theta));
        }
        for (e, f, cenv, _, theta) in &branches {
            let saved = std::mem::replace(&mut self.theta, theta.clone());
            let r = (|| -> R<()> {
                for (i, ic) in self.inv_clauses(f, false).into_iter().enumerate() {
                    let ob = self.ev_ast(&ic.expr, &mut st, &[], false, cenv)?; // LG16
                    self.prove_core(&st, &[], &ob, pos(e), "struct_invariant",
                        &format!("invariant of {} (line {}) for parameter {} of {}", ic.sname, pruv_core::diagnostic::plain_line(ic.line), ic.who, f.name), Some(&i.to_string()));
                }
                for req in &f.requires {
                    let ob = self.ev_ast(req, &mut st, &[], false, cenv)?;
                    self.prove_core(&st, &[], &ob, pos(e), "call_requires",
                        &format!("precondition of {} (line {})", f.name, pruv_core::diagnostic::plain_line(req.line())), None);
                }
                self.termination(e, f, &mut st, &[], cenv)?;
                self.call_cost(e, f, &mut st, &[], cenv)
            })();
            self.theta = saved;
            r?;
        }
        for (e, f, cenv, ios, theta) in &mut branches {
            if !ios.is_empty() {
                // LG5: a generic resizable final takes the branch's element type
                let saved = std::mem::replace(&mut self.theta, theta.clone());
                let r = self.inout_finals(e, f, ios, cenv, &mut st, &[]);
                self.theta = saved;
                r?;
            }
        }
        let base = st.fork(None);
        for (_, f, cenv, _, theta) in &branches {
            let saved = std::mem::replace(&mut self.theta, theta.clone());
            let r = (|| -> R<()> {
                for ic in self.inv_clauses(f, true) {
                    let ob = self.ev_ast(&ic.expr, &mut st, &[], false, cenv)?; // LG16
                    self.assume(&mut st, &[], ob);
                }
                for en in &f.ensures {
                    let ob = self.ev_ast(en, &mut st, &[], false, cenv)?;
                    self.assume(&mut st, &[], ob);
                }
                Ok(())
            })();
            self.theta = saved;
            r?;
        }
        for (e, f, cenv, ios, theta) in &branches {
            let c = match e { Expr::Call(c) => c, _ => unreachable!() };
            if !ios.iter().any(|&i| matches!(&c.args[i], Expr::InoutArg(ia) if !ia.post_refine.is_empty())) {
                continue;
            }
            let mut b = base.fork(None);
            let saved = std::mem::replace(&mut self.theta, theta.clone());
            let r = (|| -> R<()> {
                for ic in self.inv_clauses(f, true) {
                    let ob = self.ev_ast(&ic.expr, &mut b, &[], false, cenv)?; // LG16
                    self.assume(&mut b, &[], ob);
                }
                for en in &f.ensures {
                    let ob = self.ev_ast(en, &mut b, &[], false, cenv)?;
                    self.assume(&mut b, &[], ob);
                }
                Ok(())
            })();
            self.theta = saved;
            r?;
            for &i in ios {
                if let Expr::InoutArg(ia) = &c.args[i] {
                    for a in &ia.post_refine {
                        st.pc.push(self.check_assert(a, &mut b)?);
                    }
                }
            }
        }
        Ok(st)
    }

    /// verify.py `inout_finals` (G5 A1, G5_SPEC 5.2 step 5): for every inout argument in parameter order, fresh contents of
    /// exactly the pre-call length (same length term, same element type, u8 domain for [u8]). The final array is what the
    /// callee's ensures denote by the parameter name, and it rebinds the caller's marked place. All finals exist before any
    /// ensures clause is assumed, so relational ensures may connect several of them.
    fn inout_finals(
        &mut self,
        e: &Expr,
        f: &'a ast::Function,
        ios: &[usize],
        cenv: &mut Env,
        st: &mut State,
        g: &[Ast],
    ) -> R<()> {
        let call = match e {
            Expr::Call(c) => c,
            _ => return Err("inout_finals: not a call".to_string()),
        };
        for &i in ios {
            let pname = f.params[i].name.as_str();
            let place = match call.args.get(i) {
                Some(Expr::InoutArg(ia)) => ia.expr.as_ref(),
                _ => return Err("inout_finals: missing inout argument".to_string()),
            };
            let mut root = place;
            while let Expr::FieldAccess(field) = root {
                root = &field.base;
            }
            let Expr::Var(root) = root else { return Err("inout_finals: operand is not a place".into()) };
            let name = &root.name;
            let pre = cenv.get(&old_key(pname));
            let ty = Ty::from_name(&f.params[i].ty);
            let ty = Ty::from_gtype(&gtype::substitute(&ty.to_gtype(), &self.theta));
            let nv = match pre {
                Val::Arr(pre) if !self.resize_params.contains(&(f.name.clone(), i)) => {
                    let fresh_ty = if matches!(ty.coll(), Some(Coll::Vec(_))) { ty.clone() } else { pre.array_ty() };
                    let fv = self.try_fresh(name, &fresh_ty)?;
                    let fa = fv.try_arr()?;
                    let nv = pre.like(fa.a.clone(), pre.n.clone(), name);
                    if nv.elem == Elem::U8 {
                        let d = self.u8_dom(&nv);
                        self.assume(st, g, d);
                    }
                    Val::Arr(nv)
                }
                _ => {
                    let nv = self.try_fresh(name, &ty)?;
                    if let Some(wf) = self.wf_of_val(&ty, &nv)? {
                        self.assume(st, g, wf);
                    }
                    if let Val::Arr(a) = &nv {
                        // LG5: a generic [T] has no wf, so its new length gets the entry bounds
                        let b = self.z.and(&[a.n.ge_i(0), a.n.le_i(MAX_LEN)]);
                        self.assume(st, g, b);
                    }
                    nv
                }
            };
            if let (Val::Dt(before, _), Val::Dt(after, _)) = (pre, &nv) {
                let changed = self.struct_len_changes.get(&(f.name.clone(), i));
                for frame in self.model.array_field_len_frame(&ty, before, after, changed)? {
                    self.assume(st, g, frame);
                }
            }
            cenv.set(pname, nv.clone());
            self.store_collection_place(place, &mut st.env, nv)?;
        }
        Ok(())
    }

    /// verify.py `post_refine` (G5 A1, G5_SPEC 5.2 step 8): the refinement of every marked refined local must hold right after
    /// the call (refinement_assign, proved from the callee ensures and the remaining facts; never assumed beforehand). The
    /// assertions were attached to the inout argument by refine.rs.
    fn post_refine(&mut self, e: &Expr, ios: &[usize], st: &mut State, _g: &[Ast]) -> R<()> {
        let call = match e {
            Expr::Call(c) => c,
            _ => return Ok(()),
        };
        for &i in ios {
            if let Some(Expr::InoutArg(ia)) = call.args.get(i) {
                for a in &ia.post_refine {
                    let ob = self.check_assert(a, st)?;
                    st.pc.push(ob);
                }
            }
        }
        Ok(())
    }

    /// verify.py `check_assert` (G5 A1): prove one assertion (user or synthesized by refine.rs) on st; returns its value term.
    fn check_assert(&mut self, s: &ast::Assert, st: &mut State) -> R<Ast> {
        let spos = mk_pos(s.line, s.col);
        self.check_dom_e(st, &[], &s.cond, spos, "variant_field lookups in assertion are defined", None)?;
        let ob = self.ev0_ast(&s.cond, st, &[], false)?;
        match &s.origin_code {
            None => self.prove_core(&st, &[], &ob, spos, "assert", "assertion", None),
            Some(code) => {
                // An assert synthesized by refine.rs: its own code and the source description. The description also goes into the duplicate-suppression key,
                // so multiple points of one annotation are all reported.
                let what = s.origin_what.as_deref().unwrap_or("");
                self.prove_core(&st, &[], &ob, spos, code, what, Some(what));
            }
        }
        Ok(ob)
    }

    fn has_cost(&self) -> bool {
        self.f.is_some_and(|f| f.cost.is_some())
    }

    fn spend(&self, st: &mut State, g: &[Ast], units: Ast) {
        if self.has_cost() {
            let units = if g.is_empty() { units } else { self.z.ite(&self.z.and(g), &units, &self.z.int(0)) };
            st.env.set("spent", Val::Int(st.env.get("spent").ast().add(&units)));
        }
    }

    fn cost_value(&mut self, expr: &Expr, st: &mut State, g: &[Ast], node: Pos, env: &Env) -> R<Ast> {
        if let Some(domain) = self.dom(expr, st, g, env, PKind::All)? {
            self.prove_core(&st, g, &domain, node, "contract_domain",
                &format!("cost `{}` is defined", expr_text(expr)), None);
        }
        self.ev_ast(expr, st, g, false, env)
    }

    fn call_cost(&mut self, e: &Expr, f: &'a ast::Function, st: &mut State, g: &[Ast], cenv: &Env) -> R<()> {
        if self.has_cost() {
            let expr = f.cost.as_ref().ok_or_else(|| "callee has no cost".to_string())?;
            let bound = self.cost_value(expr, st, g, pos(e), cenv)?;
            self.spend(st, g, self.z.int(1).add(&bound));
        }
        Ok(())
    }

    /// verify.py `termination` (G9_SPEC 5): the obligations of a call s.f -> f inside one recursive SCC, after the callee requires:
    /// definedness of the instantiated callee measure (existing domain codes), M_f(actuals) >= 0 (`recursive_nonneg`),
    /// M_f(actuals) < M_s.f(frozen entry value) (`recursive_decrease`). Only after these the caller introduces the fresh result
    /// and assumes the callee ensures (so the call's own ensures never justify its own decrease or precondition).
    /// `cenv`: the callee parameters bound to the actual argument values.
    fn termination(&mut self, e: &Expr, f: &'a ast::Function, st: &mut State, g: &[Ast], cenv: &Env) -> R<()> {
        let Some(sf) = self.f else { return Ok(()) };
        let a = self.scc_id.get(&f.name).copied();
        let b = self.scc_id.get(&sf.name).copied();
        let (Some(fd), Some(m0)) = (&f.decreases, self.m0.clone()) else { return Ok(()) };
        if a.is_none() || a != b {
            return Ok(());
        }
        self.check_dom_e(
            st,
            g,
            fd,
            pos(e),
            &format!("variant_field lookups in the decreases clause of {} are defined", f.name),
            Some(cenv),
        )?;
        let mg = self.ev_ast(fd, st, g, false, cenv)?;
        let tg = expr_text(fd);
        let tf = match &sf.decreases {
            Some(d) => expr_text(d),
            None => return Err("'NoneType' object has no attribute 'decreases'".to_string()),
        };
        let ob = mg.ge_i(0);
        self.prove_core(
            &st,
            g,
            &ob,
            pos(e),
            "recursive_nonneg",
            &format!("decreases measure of {} (`{}`) >= 0 at this call", f.name, tg),
            None,
        );
        let ob = mg.lt(&m0);
        self.prove_core(
            &st,
            g,
            &ob,
            pos(e),
            "recursive_decrease",
            &format!(
                "decreases measure of {} (`{}`) < decreases measure of {} at its entry (`{}`)",
                f.name, tg, sf.name, tf
            ),
            None,
        );
        Ok(())
    }

    /// One `call_requires` obligation of a catalog operation (verify.py proc_call `need`): in code only.
    fn proc_need(&mut self, st: &State, g: &[Ast], e: &Expr, code: bool, what: &str, ob: Ast) {
        if code {
            self.prove_core(&st, g, &ob, pos(e), "call_requires", what, None);
        }
    }

    /// verify.py `FnVerifier.proc_call` (G10_PROC_SPEC 2.3, 2.4, 4): the catalog contracts of the proc operations. The requires
    /// of an operation are call_requires obligations in order (code only); an effect operation then gets a fresh result with its
    /// well-formedness and the ensures facts. The argument builder and the observers are exact terms. Parameter observers in the
    /// ensures denote the values at call entry (the actuals in args). Every effect result is a fresh symbol: two identical calls
    /// are never equated.
    #[allow(clippy::too_many_arguments)]
    fn proc_call(&mut self, e: &Expr, n: &str, args: &[Val], st: &mut State, g: &[Ast], code: bool, rty: Option<Ty>) -> R<Val> {
        let z = self.z.clone();
        let what = format!("precondition of {} (catalog)", n);
        let arg = |i: usize| -> R<&Val> { args.get(i).ok_or_else(|| "not enough values to unpack".to_string()) };
        // ProcessArgs: argc only
        match n {
            "proc_args_new" => return Ok(Val::PArgs(z.int(0))),
            "proc_args_push" => {
                let a = arg(0)?.try_pargs()?.clone();
                self.proc_need(st, g, e, code, &what, a.lt_i(MAX_LEN));
                return Ok(Val::PArgs(a.add_i(1)));
            }
            "proc_args_len" => return Ok(Val::Int(arg(0)?.try_pargs()?.clone())),
            // observers: field reads and array selects
            "proc_ok" => return Ok(Val::Bool(arg(0)?.try_proc()?.error.raw_eq(&z.int(0)))),
            "proc_error" => return Ok(Val::Int(arg(0)?.try_proc()?.error.clone())),
            "proc_stdin_open" => return Ok(Val::Bool(arg(0)?.try_proc()?.stdin_open.clone())),
            "proc_eof" | "proc_size" | "proc_byte" => {
                let p = arg(0)?.try_proc()?.clone();
                let sm = arg(1)?.try_ast()?.clone();
                let sok = self.proc_stream_ok(&sm);
                self.proc_need(st, g, e, code, &what, sok);
                if n == "proc_eof" {
                    return Ok(Val::Bool(self.proc_eof_t(&p, &sm)));
                }
                if n == "proc_size" {
                    return Ok(Val::Int(self.proc_size_t(&p, &sm)));
                }
                let i = arg(2)?.try_ast()?.clone();
                let size = self.proc_size_t(&p, &sm);
                self.proc_need(st, g, e, code, &what, z.and(&[i.ge_i(0), i.lt(&size)]));
                let r = z.ite(&sm.raw_eq(&z.int(1)), &p.out.a.select(&i), &p.err.a.select(&i));
                if code {
                    self.assume(st, g, z.and(&[r.ge_i(0), r.le_i(255)]));
                }
                return Ok(Val::Int(r));
            }
            "proc_bytes" => {
                // 4.6: a new array with the snapshot's size and byte map
                let p = arg(0)?.try_proc()?.clone();
                let sm = arg(1)?.try_ast()?.clone();
                let sok = self.proc_stream_ok(&sm);
                self.proc_need(st, g, e, code, &what, sok);
                let a = z.ite(&sm.raw_eq(&z.int(1)), &p.out.a, &p.err.a);
                let size = self.proc_size_t(&p, &sm);
                return Ok(Val::Arr(Arr::new(a, size, n.to_string(), Elem::U8)));
            }
            _ => {}
        }
        // effect operations: requires -> fresh result -> well-formedness -> ensures
        let pr: Option<ProcV> = if n == "proc_spawn" || n == "proc_spawn_env" { None } else { Some(arg(0)?.try_proc()?.clone()) };
        let no_p = || "not enough values to unpack".to_string();
        match n {
            "proc_spawn" | "proc_spawn_env" => {}
            "proc_write_str" | "proc_write_bytes" | "proc_close_stdin" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                self.proc_need(st, g, e, code, &what, z.and(&[p.error.raw_eq(&z.int(0)), p.stdin_open.clone()]));
            }
            "proc_read" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                self.proc_need(st, g, e, code, &what, p.error.raw_eq(&z.int(0)));
                let sok = self.proc_stream_ok(arg(1)?.try_ast()?);
                self.proc_need(st, g, e, code, &what, sok);
                let mx = arg(2)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[mx.gt_i(0), mx.le_i(MAX_LEN)]));
            }
            "proc_read_all" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                self.proc_need(st, g, e, code, &what, z.and(&[p.error.raw_eq(&z.int(0)), p.stdin_open.not()]));
            }
            _ => {}
        }
        let rt = rty.ok_or_else(|| "fresh: None".to_string())?;
        let res = self.try_fresh(n, &rt)?;
        if let Some(w) = self.wf_of_val(&rt, &res)? {
            self.assume(st, g, w);
        }
        if n == "proc_wait" || n == "proc_kill" {
            // 4.7: the existing Result<i64> encoding; Ok(0) is never assumed
            let p = pr.as_ref().ok_or_else(no_p)?;
            let r = res.try_ast()?.clone();
            let f1 = p.error.ne_i(0).implies(&r.eq(&self.prelude.err(&p.error)));
            let f2 = self.prelude.is_err(&r).implies(&z.and(&[self.prelude.eval(&r).ge_i(1), self.prelude.eval(&r).le_i(6)]));
            self.assume(st, g, f1);
            self.assume(st, g, f2);
            return Ok(res);
        }
        let rp = res.try_proc()?.clone();
        let ok = rp.error.raw_eq(&z.int(0));
        let mut facts: Vec<Ast> = Vec::new();
        match n {
            "proc_spawn" | "proc_spawn_env" => {
                facts.push(ok.implies(&rp.stdin_open));
                facts.push(ok.not().implies(&z.and(&[
                    rp.stdin_open.not(),
                    z.or(&[rp.error.raw_eq(&z.int(1)), rp.error.raw_eq(&z.int(2))]),
                ])));
                facts.push(z.and(&[
                    rp.out.n.raw_eq(&z.int(0)), rp.err.n.raw_eq(&z.int(0)), rp.stdout_eof.not(), rp.stderr_eof.not(),
                ]));
            }
            "proc_write_str" | "proc_write_bytes" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                facts.push(ok.implies(&z.and(&[
                    rp.stdin_open.eq(&p.stdin_open), self.proc_keep_out(&rp, p), self.proc_keep_err(&rp, p),
                ])));
            }
            "proc_close_stdin" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                facts.push(rp.stdin_open.not());
                facts.push(ok.implies(&z.and(&[self.proc_keep_out(&rp, p), self.proc_keep_err(&rp, p)])));
            }
            "proc_read" => {
                let p = pr.as_ref().ok_or_else(no_p)?;
                let sm = arg(1)?.try_ast()?.clone();
                let mx = arg(2)?.try_ast()?.clone();
                let sz = self.proc_size_t(&rp, &sm);
                facts.push(ok.implies(&z.and(&[sz.ge_i(0), sz.le(&mx)])));
                facts.push(ok.implies(&self.proc_eof_t(&rp, &sm).eq(&sz.raw_eq(&z.int(0)))));
                facts.push(z.and(&[ok.clone(), self.proc_eof_t(p, &sm)]).implies(&sz.raw_eq(&z.int(0))));
                facts.push(z.and(&[ok.clone(), sm.raw_eq(&z.int(1))]).implies(&self.proc_keep_err(&rp, p)));
                facts.push(z.and(&[ok.clone(), sm.raw_eq(&z.int(2))]).implies(&self.proc_keep_out(&rp, p)));
                facts.push(ok.implies(&rp.stdin_open.eq(&p.stdin_open)));
            }
            "proc_read_all" => {
                facts.push(rp.stdin_open.not());
                facts.push(ok.implies(&z.and(&[rp.stdout_eof.clone(), rp.stderr_eof.clone()])));
            }
            _ => return Err(format!("catalog: {}", n)),
        }
        for f in facts {
            self.assume(st, g, f);
        }
        Ok(res)
    }

    /// G10 async section 4: only public snapshots are constrained.
    fn wf_loop(&self, p: &LoopV) -> Ast {
        let z = &self.z;
        let i = z.int_const("ev.wf.slot");
        // Built in the order of Python `_wf_loop` (term creation order steers the solver on quantified goals).
        let head = [p.error.ge_i(0), p.error.le_i(2),
            p.cap.ge_i(1), p.cap.le_i(4096),
            p.count.ge_i(0), p.count.le(&p.cap),
            p.last.ge_i(-1), p.last.lt(&p.cap),
            p.ev_slot.ge_i(-1), p.ev_slot.lt(&p.cap),
            p.ev_kind_.ge_i(0), p.ev_kind_.le_i(5),
            p.ev_slot.raw_eq(&z.int(-1)).raw_eq(&p.ev_kind_.raw_eq(&z.int(0))),
            p.buf.n.ge_i(0), p.buf.n.le_i(MAX_LEN),
            p.last.ne_i(-1).implies(&p.kind.select(&p.last).ne_i(0)),
            p.error.ne_i(0).implies(&z.and(&[p.last.raw_eq(&z.int(-1)), p.ev_slot.raw_eq(&z.int(-1)), p.buf.n.raw_eq(&z.int(0))])),
            self.u8_dom(&p.buf)];
        let guard = z.and(&[i.ge_i(0), i.lt(&p.cap)]);
        let k0 = p.kind.select(&i).ge_i(0);
        let k4 = p.kind.select(&i).le_i(4);
        let open = z.or(&[p.eof.select(&i), p.write_open.select(&i)]);
        let k13 = z.or(&[p.kind.select(&i).raw_eq(&z.int(1)), p.kind.select(&i).raw_eq(&z.int(3))]);
        let slot = z.and(&[k0, k4, open.implies(&k13)]);
        let all = z.forall(std::slice::from_ref(&i), &guard.implies(&slot), &[]);
        let mut v: Vec<Ast> = head.to_vec();
        v.push(all);
        z.and(&v)
    }

    fn async_call(&mut self, e: &Expr, n: &str, args: &[Val], st: &mut State, g: &[Ast], code: bool, rty: Option<Ty>) -> R<Val> {
        let z = self.z.clone();
        let into = matches!(n, "ev_read_into" | "ev_proc_read_into");
        let what = format!("precondition of {} (catalog)", n);
        let p = if n != "ev_new" { Some(args[0].try_loop()?.clone()) } else { None };
        if matches!(n, "ev_ok") {
            return Ok(Val::Bool(p.as_ref().unwrap().error.raw_eq(&z.int(0))));
        }
        if matches!(n, "ev_error") {
            return Ok(Val::Int(p.as_ref().unwrap().error.clone()));
        }
        if matches!(n, "ev_cap") {
            return Ok(Val::Int(p.as_ref().unwrap().cap.clone()));
        }
        if matches!(n, "ev_count") {
            return Ok(Val::Int(p.as_ref().unwrap().count.clone()));
        }
        if matches!(n, "ev_last") {
            return Ok(Val::Int(p.as_ref().unwrap().last.clone()));
        }
        if matches!(n, "ev_event_slot") {
            return Ok(Val::Int(p.as_ref().unwrap().ev_slot.clone()));
        }
        if matches!(n, "ev_event_kind") {
            return Ok(Val::Int(p.as_ref().unwrap().ev_kind_.clone()));
        }
        if matches!(n, "ev_exit") {
            return Ok(Val::Int(p.as_ref().unwrap().exit.clone()));
        }
        if matches!(n, "ev_size") {
            return Ok(Val::Int(p.as_ref().unwrap().buf.n.clone()));
        }
        if matches!(n, "ev_kind" | "ev_eof" | "ev_write_open" | "ev_byte") {
            let p = p.as_ref().unwrap(); let i = args[1].try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, z.and(&[i.ge_i(0), i.lt(if n == "ev_byte" { &p.buf.n } else { &p.cap })]));
            let a = match n { "ev_kind" => &p.kind, "ev_eof" => &p.eof, "ev_write_open" => &p.write_open, _ => &p.buf.a };
            return Ok(if n == "ev_eof" || n == "ev_write_open" { Val::Bool(a.select(&i)) } else { Val::Int(a.select(&i)) });
        }
        if matches!(n, "ev_bytes") {
            let p = p.as_ref().unwrap(); return Ok(Val::Arr(Arr::new(p.buf.a.clone(), p.buf.n.clone(), n.into(), Elem::U8)));
        }
        if n == "ev_new" {
            let cap = args[0].try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, z.and(&[(cap.clone()).ge_i(1), (cap.clone()).le_i(4096)]));
            let res = self.try_fresh(n, &rty.unwrap())?; let r = res.try_loop()?.clone();
            self.assume(st, g, self.wf_loop(&r));
            self.assume(st, g, (r.error.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, (r.cap.clone()).eq(&cap.clone()));
            self.assume(st, g, (r.count.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, (r.last.clone()).raw_eq(&z.int(-1)));
            self.assume(st, g, (r.ev_slot.clone()).raw_eq(&z.int(-1)));
            self.assume(st, g, (r.ev_kind_.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, (r.exit.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, (r.buf.n.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, r.kind.eq(&z.const_array(&z.int_sort(), &z.int(0))));
            self.assume(st, g, r.eof.eq(&z.const_array(&z.int_sort(), &z.bool_val(false))));
            self.assume(st, g, r.write_open.eq(&z.const_array(&z.int_sort(), &z.bool_val(false))));
            return Ok(res);
        }
        let p = p.unwrap();
        if matches!(n, "ev_close") {
            let res = self.try_fresh(n, &rty.unwrap())?; let rv = res.try_ast()?.clone();
        
            self.assume(st, g, p.error.ne_i(0).implies(&self.prelude.is_err(&rv)));
            self.assume(st, g, self.prelude.is_ok(&rv).implies(&z.and(&[p.error.raw_eq(&z.int(0)), self.prelude.oval(&rv).eq(&p.count)])));
            self.assume(st, g, self.prelude.is_err(&rv).implies(&z.and(&[self.prelude.eval(&rv).ge(&z.ite(&p.error.raw_eq(&z.int(0)), &z.int(3), &z.int(1))), self.prelude.eval(&rv).le_i(8)])));
            return Ok(res);
        }
        self.proc_need(st, g, e, code, &what, (p.error.clone()).raw_eq(&z.int(0)));
        if matches!(n, "ev_add_socket" | "ev_add_listener" | "ev_add_process" | "ev_add_timer" | "ev_accept") {
            self.proc_need(st, g, e, code, &what, (p.count.clone()).lt(&p.cap.clone()));
        }
        if matches!(n, "ev_add_socket") {
            let h = args[1].try_sock()?;
            self.proc_need(st, g, e, code, &what, (h.error.clone()).raw_eq(&z.int(0)));
        }
        if matches!(n, "ev_add_listener") {
            let h = args[1].try_lis()?;
            self.proc_need(st, g, e, code, &what, (h.error.clone()).raw_eq(&z.int(0)));
        }
        if matches!(n, "ev_add_process") {
            let h = args[1].try_proc()?;
            self.proc_need(st, g, e, code, &what, (h.error.clone()).raw_eq(&z.int(0)));
        }
        if matches!(n, "ev_add_timer" | "ev_wait") {
            let ms = args[1].try_ast()?.clone();
            self.proc_need(st,g,e,code,&what,z.and(&[ms.ge_i(if n == "ev_wait" { -1 } else { 0 }),ms.le_i(86400000)]));
        }
        if matches!(n, "ev_proc_wait_for" | "ev_proc_write_for") {
            let ms = args.last().unwrap().try_ast()?.clone();
            self.proc_need(st,g,e,code,&what,z.and(&[ms.ge_i(0),ms.le_i(86400000)]));
        }
        if into || matches!(n, "ev_read" | "ev_write" | "ev_shutdown_write" | "ev_accept" | "ev_proc_read" | "ev_proc_write" | "ev_proc_close_stdin" | "ev_proc_wait" | "ev_remove" | "ev_proc_wait_for" | "ev_proc_write_for") {
            let i = args[1].try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, z.and(&[(i.clone()).ge_i(0), (i.clone()).lt(&p.cap.clone())]));
            let k = p.kind.select(&i);
            let expected = if n == "ev_accept" { 2 } else if n.starts_with("ev_proc_") { 3 } else { 1 };
            self.proc_need(st,g,e,code,&what,if n == "ev_remove" { k.ne_i(0) } else { k.raw_eq(&z.int(expected)) });
        }
        if matches!(n, "ev_write" | "ev_shutdown_write" | "ev_proc_write" | "ev_proc_close_stdin" | "ev_proc_write_for") {
            let i = args[1].try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, (p.write_open.clone()).select(&i.clone()));
        }
        if matches!(n, "ev_read" | "ev_proc_read") {
            let mx = args.last().unwrap().try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, z.and(&[(mx.clone()).gt_i(0), (mx.clone()).le_i(MAX_LEN)]));
        }
        if matches!(n, "ev_proc_read" | "ev_proc_read_into") {
            let stream = args[2].try_ast()?.clone();
            self.proc_need(st, g, e, code, &what, z.or(&[(stream.clone()).raw_eq(&z.int(1)), (stream.clone()).raw_eq(&z.int(2))]));
        }
        let ios = [args.len().saturating_sub(2), args.len().saturating_sub(1)];
        let read_count = if into {
            let Expr::Call(call) = e else { return Err("read into: not a call".into()); };
            let [Expr::InoutArg(dst), Expr::InoutArg(count)] = &call.args[ios[0]..] else {
                return Err("read into: missing inout argument".into());
            };
            let pre = args[ios[0]].try_arr()?;
            let Expr::Var(dst_var) = dst.expr.as_ref() else { return Err("read into: invalid destination".into()); };
            let Expr::Var(count_var) = count.expr.as_ref() else { return Err("read into: invalid count".into()); };
            let fresh = self.try_fresh(&dst_var.name, &pre.array_ty())?;
            let after = pre.like(fresh.try_arr()?.a.clone(), pre.n.clone(), &dst_var.name);
            self.assume(st, g, self.u8_dom(&after));
            let count_val = self.try_fresh(&count_var.name, &Ty::I64)?;
            let count_after = count_val.try_ast()?.clone();
            if let Some(w) = self.try_wf(&Ty::I64, &count_after)? { self.assume(st, g, w); }
            self.assume(st, g, z.and(&[count_after.ge_i(0), count_after.le(&after.n)]));
            st.env.set(&dst_var.name, Val::Arr(after));
            st.env.set(&count_var.name, count_val);
            Some(count_after)
        } else {
            None
        };
        let res = self.try_fresh(n, &rty.unwrap())?; let r = res.try_loop()?.clone();
        self.assume(st, g, self.wf_loop(&r));
        self.assume(st, g, (r.cap.clone()).eq(&p.cap.clone()));
        if matches!(n, "ev_add_socket" | "ev_add_listener" | "ev_add_process" | "ev_add_timer" | "ev_accept") {
            let a = r.last.clone();
            let ok = r.error.raw_eq(&z.int(0));
            let added = z.and(&[ok.clone(), a.ne_i(-1)]);
            let k = z.int(if n == "ev_add_timer" { 4 } else if n == "ev_add_process" { 3 } else if n == "ev_add_listener" { 2 } else { 1 });
            self.assume(st, g, (added.clone()).implies(&z.and(&[(a.clone()).ge_i(0), (a.clone()).lt(&p.cap.clone()), ((p.kind.clone()).select(&a.clone())).raw_eq(&z.int(0)), (r.count.clone()).eq(&(p.count.clone()).add(&z.int(1))), (r.kind.clone()).eq(&(p.kind.clone()).store(&a.clone(), &k.clone())), (r.eof.clone()).eq(&(p.eof.clone()).store(&a.clone(), &z.bool_val(false)))])));
            self.assume(st, g, ((added.clone()).not()).implies(&z.and(&[(r.last.clone()).raw_eq(&z.int(-1)), (r.count.clone()).eq(&p.count.clone()), (r.kind.clone()).eq(&p.kind.clone()), (r.eof.clone()).eq(&p.eof.clone()), (r.write_open.clone()).eq(&p.write_open.clone())])));
            let opened = if n == "ev_add_socket" { args[1].try_sock()?.write_open.clone() } else if n == "ev_add_process" { args[1].try_proc()?.stdin_open.clone() } else { z.bool_val(n == "ev_accept") };
            self.assume(st, g, (added.clone()).implies(&(r.write_open.clone()).eq(&(p.write_open.clone()).store(&a.clone(), &opened.clone()))));
            if n != "ev_accept" { self.assume(st,g,ok.implies(&a.ne_i(-1))); }
            if n == "ev_add_timer" { self.assume(st,g,ok); }
        }
        else {
            self.assume(st, g, (r.error.clone()).raw_eq(&z.int(0)));
            self.assume(st, g, (r.last.clone()).raw_eq(&z.int(-1)));
        }
        if matches!(n, "ev_wait") {
            let i = r.ev_slot.clone(); let k = r.ev_kind_.clone();
            let ms = args[1].try_ast()?.clone();
            self.assume(st, g, ((ms.clone()).raw_eq(&z.int(-1))).implies(&(k.clone()).ne_i(0)));
            self.assume(st, g, (z.or(&[(k.clone()).raw_eq(&z.int(1)), (k.clone()).raw_eq(&z.int(3))])).implies(&z.or(&[((p.kind.clone()).select(&i.clone())).raw_eq(&z.int(1)), ((p.kind.clone()).select(&i.clone())).raw_eq(&z.int(3))])));
            self.assume(st, g, ((k.clone()).raw_eq(&z.int(2))).implies(&((p.kind.clone()).select(&i.clone())).raw_eq(&z.int(2))));
            self.assume(st, g, ((k.clone()).raw_eq(&z.int(4))).implies(&((p.kind.clone()).select(&i.clone())).raw_eq(&z.int(4))));
            self.assume(st, g, ((k.clone()).raw_eq(&z.int(5))).implies(&z.and(&[((p.kind.clone()).select(&i.clone())).ge_i(1), ((p.kind.clone()).select(&i.clone())).le_i(3)])));
            self.assume(st, g, ((k.clone()).raw_eq(&z.int(4))).implies(&z.and(&[(r.kind.clone()).eq(&(p.kind.clone()).store(&i.clone(), &z.int(0))), (r.eof.clone()).eq(&(p.eof.clone()).store(&i.clone(), &z.bool_val(false))), (r.write_open.clone()).eq(&(p.write_open.clone()).store(&i.clone(), &z.bool_val(false))), (r.count.clone()).eq(&(p.count.clone()).sub(&z.int(1)))])));
            self.assume(st, g, ((k.clone()).ne_i(4)).implies(&z.and(&[(r.kind.clone()).eq(&p.kind.clone()), (r.count.clone()).eq(&p.count.clone())])));
            self.assume(st, g, ((k.clone()).raw_eq(&z.int(5))).implies(&z.and(&[(r.eof.clone()).eq(&(p.eof.clone()).store(&i.clone(), &z.bool_val(false))), (r.write_open.clone()).eq(&(p.write_open.clone()).store(&i.clone(), &z.bool_val(false)))])));
            self.assume(st, g, (z.and(&[(k.clone()).ne_i(4), (k.clone()).ne_i(5)])).implies(&z.and(&[(r.eof.clone()).eq(&p.eof.clone()), (r.write_open.clone()).eq(&p.write_open.clone())])));
        }
        if into || matches!(n, "ev_read" | "ev_write" | "ev_shutdown_write" | "ev_proc_read" | "ev_proc_write" | "ev_proc_close_stdin" | "ev_proc_wait" | "ev_remove" | "ev_proc_wait_for" | "ev_proc_write_for") {
            let i = args[1].try_ast()?.clone();
            let freed = matches!(n,"ev_remove" | "ev_proc_wait");
            let k = if freed { z.int(0) } else if n == "ev_proc_wait_for" { r.kind.select(&i) } else { p.kind.select(&i) };
            self.assume(st, g, (r.kind.clone()).eq(&(p.kind.clone()).store(&i.clone(), &k.clone())));
            self.assume(st, g, (r.eof.clone()).eq(&(p.eof.clone()).store(&i.clone(), &(r.eof.clone()).select(&i.clone()))));
            self.assume(st, g, (r.write_open.clone()).eq(&(p.write_open.clone()).store(&i.clone(), &(r.write_open.clone()).select(&i.clone()))));
            if n == "ev_proc_wait_for" {
                // G10-ASYNC-DEADLINE: either the slot was freed (as ev_proc_wait) or it was kept with stdin closed
                self.assume(st, g, r.write_open.select(&i).not());
                self.assume(st, g, z.or(&[z.and(&[r.kind.select(&i).raw_eq(&z.int(0)), r.eof.select(&i).not(), r.count.eq(&p.count.sub(&z.int(1)))]),
                                          z.and(&[r.kind.select(&i).raw_eq(&z.int(3)), r.eof.select(&i).eq(&p.eof.select(&i)), r.count.eq(&p.count), r.exit.eq(&p.exit)])]));
            }
            else {
                self.assume(st,g,r.count.eq(&if freed { p.count.sub(&z.int(1)) } else { p.count.clone() }));
            }
            if n == "ev_proc_wait_for" {
            }
            else if freed {
                self.assume(st, g, ((r.eof.clone()).select(&i.clone())).not());
                self.assume(st, g, ((r.write_open.clone()).select(&i.clone())).not());
            }
            else if let Some(count) = &read_count {
                self.assume(st, g, p.eof.select(&i).implies(&r.eof.select(&i)));
                self.assume(st, g, r.eof.select(&i).implies(&count.raw_eq(&z.int(0))));
            }
            else if matches!(n,"ev_read" | "ev_proc_read") {
                let mx = args.last().unwrap().try_ast()?.clone();
                self.assume(st, g, (r.buf.n.clone()).le(&mx.clone()));
                self.assume(st, g, ((p.eof.clone()).select(&i.clone())).implies(&(r.eof.clone()).select(&i.clone())));
                self.assume(st, g, ((r.eof.clone()).select(&i.clone())).implies(&(r.buf.n.clone()).raw_eq(&z.int(0))));
                self.assume(st, g, ((r.write_open.clone()).select(&i.clone())).implies(&(p.write_open.clone()).select(&i.clone())));
            }
            else {
                self.assume(st, g, ((r.eof.clone()).select(&i.clone())).eq(&(p.eof.clone()).select(&i.clone())));
            }
            if matches!(n,"ev_shutdown_write" | "ev_proc_close_stdin") { self.assume(st,g,r.write_open.select(&i).not()); }
        }
        if n != "ev_wait" {
            self.assume(st, g, r.error.raw_eq(&z.int(0)).implies(&(r.ev_slot.clone()).eq(&p.ev_slot.clone())));
            self.assume(st, g, r.error.raw_eq(&z.int(0)).implies(&(r.ev_kind_.clone()).eq(&p.ev_kind_.clone())));
        }
        if !matches!(n, "ev_proc_wait" | "ev_proc_wait_for") {
            self.assume(st, g, (r.exit.clone()).eq(&p.exit.clone()));
        }
        if !matches!(n,"ev_read" | "ev_proc_read") {
            self.assume(st, g, r.error.raw_eq(&z.int(0)).implies(&(r.buf.n.clone()).eq(&p.buf.n.clone())));
            self.assume(st, g, r.error.raw_eq(&z.int(0)).implies(&(r.buf.a.clone()).eq(&p.buf.a.clone())));
        }
        if into { self.post_refine(e, &ios, st, g)?; }
        Ok(res)
    }

    /// G10 net catalog contracts, using call-entry snapshots.
    #[allow(clippy::too_many_arguments)]
    fn net_call(&mut self, e: &Expr, n: &str, args: &[Val], st: &mut State, g: &[Ast], code: bool, rty: Option<Ty>) -> R<Val> {
        let z = self.z.clone();
        let what = format!("precondition of {} (catalog)", n);
        let arg = |i: usize| -> R<&Val> { args.get(i).ok_or_else(|| "not enough values to unpack".to_string()) };
        // observers: field reads and array selects
        match n {
            "net_ok" => return Ok(Val::Bool(arg(0)?.try_sock()?.error.raw_eq(&z.int(0)))),
            "net_error" => return Ok(Val::Int(arg(0)?.try_sock()?.error.clone())),
            "net_write_open" => return Ok(Val::Bool(arg(0)?.try_sock()?.write_open.clone())),
            "net_eof" => return Ok(Val::Bool(arg(0)?.try_sock()?.eof.clone())),
            "net_size" => return Ok(Val::Int(arg(0)?.try_sock()?.buf.n.clone())),
            "net_byte" => {
                let sv = arg(0)?.try_sock()?.clone();
                let i = arg(1)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[i.ge_i(0), i.lt(&sv.buf.n)]));
                let r = sv.buf.a.select(&i);
                if code {
                    self.assume(st, g, z.and(&[r.ge_i(0), r.le_i(255)]));
                }
                return Ok(Val::Int(r));
            }
            "net_listener_ok" => return Ok(Val::Bool(arg(0)?.try_lis()?.error.raw_eq(&z.int(0)))),
            "net_listener_error" => return Ok(Val::Int(arg(0)?.try_lis()?.error.clone())),
            "net_listener_port" => return Ok(Val::Int(arg(0)?.try_lis()?.port.clone())),
            "net_bytes" => {
                // 4.6: a new array with the snapshot's length and byte map
                let sv = arg(0)?.try_sock()?.clone();
                return Ok(Val::Arr(Arr::new(sv.buf.a.clone(), sv.buf.n.clone(), n.to_string(), Elem::U8)));
            }
            _ => {}
        }
        // effect operations: requires -> fresh result -> well-formedness -> ensures
        match n {
            "net_listen" => {
                let port = arg(1)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[port.ge_i(0), port.le_i(65535)]));
            }
            "net_accept" => {
                let l = arg(0)?.try_lis()?.clone();
                self.proc_need(st, g, e, code, &what, l.error.raw_eq(&z.int(0)));
            }
            "net_connect" => {
                let port = arg(1)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[port.ge_i(1), port.le_i(65535)]));
            }
            "net_write_str" | "net_write_bytes" | "net_shutdown_write" => {
                let p = arg(0)?.try_sock()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[p.error.raw_eq(&z.int(0)), p.write_open.clone()]));
            }
            "net_set_timeout" => {
                let p = arg(0)?.try_sock()?.clone();
                self.proc_need(st, g, e, code, &what, p.error.raw_eq(&z.int(0)));
                let ms = arg(1)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[ms.ge_i(0), ms.le_i(86400000)]));
            }
            "net_read" => {
                let p = arg(0)?.try_sock()?.clone();
                self.proc_need(st, g, e, code, &what, p.error.raw_eq(&z.int(0)));
                let mx = arg(1)?.try_ast()?.clone();
                self.proc_need(st, g, e, code, &what, z.and(&[mx.gt_i(0), mx.le_i(MAX_LEN)]));
            }
            "net_read_all" => {
                let p = arg(0)?.try_sock()?.clone();
                self.proc_need(st, g, e, code, &what, p.error.raw_eq(&z.int(0)));
            }
            _ => {}
        }
        let rt = rty.ok_or_else(|| "fresh: None".to_string())?;
        let res = self.try_fresh(n, &rt)?;
        if let Some(w) = self.wf_of_val(&rt, &res)? {
            self.assume(st, g, w);
        }
        if n == "net_close" || n == "net_listener_close" {
            // 4.7: the existing Result<i64> encoding; Ok(0) exactly when there is no entry error
            let perr = if n == "net_close" { arg(0)?.try_sock()?.error.clone() } else { arg(0)?.try_lis()?.error.clone() };
            let top = if n == "net_close" { 6 } else { 2 };
            let r = res.try_ast()?.clone();
            let f1 = perr.ne_i(0).implies(&r.eq(&self.prelude.err(&perr)));
            let f2 = perr.raw_eq(&z.int(0)).implies(&r.eq(&self.prelude.ok(&z.int(0))));
            let f3 = self.prelude.is_err(&r).implies(&z.and(&[self.prelude.eval(&r).ge_i(1), self.prelude.eval(&r).le_i(top)]));
            self.assume(st, g, f1);
            self.assume(st, g, f2);
            self.assume(st, g, f3);
            return Ok(res);
        }
        let mut facts: Vec<Ast> = Vec::new();
        if n == "net_listen" {
            let rl = res.try_lis()?.clone();
            let ok = rl.error.raw_eq(&z.int(0));
            facts.push(ok.not().implies(&z.or(&[rl.error.raw_eq(&z.int(1)), rl.error.raw_eq(&z.int(2))])));
        } else {
            let rp = res.try_sock()?.clone();
            let ok = rp.error.raw_eq(&z.int(0));
            let fail = ok.not();
            let empty_fresh = || z.and(&[rp.write_open.clone(), rp.buf.n.raw_eq(&z.int(0)), rp.eof.not()]);
            match n {
                "net_accept" => {
                    facts.push(ok.implies(&empty_fresh()));
                    facts.push(fail.implies(&z.and(&[rp.error.raw_eq(&z.int(5)), rp.buf.n.raw_eq(&z.int(0)), rp.eof.not()])));
                }
                "net_connect" => {
                    facts.push(ok.implies(&empty_fresh()));
                    facts.push(fail.implies(&z.and(&[
                        z.or(&[rp.error.raw_eq(&z.int(1)), rp.error.raw_eq(&z.int(2))]),
                        rp.buf.n.raw_eq(&z.int(0)),
                        rp.eof.not(),
                    ])));
                }
                "net_write_str" | "net_write_bytes" => {
                    let p = arg(0)?.try_sock()?.clone();
                    facts.push(ok.implies(&z.and(&[rp.write_open.eq(&p.write_open), self.net_keep_buf(&rp, &p)])));
                    facts.push(fail.implies(&z.or(&[rp.error.raw_eq(&z.int(4)), rp.error.raw_eq(&z.int(6))])));
                }
                "net_shutdown_write" => {
                    let p = arg(0)?.try_sock()?.clone();
                    facts.push(rp.write_open.not());
                    facts.push(ok.implies(&self.net_keep_buf(&rp, &p)));
                    facts.push(fail.implies(&rp.error.raw_eq(&z.int(4))));
                }
                "net_set_timeout" => {
                    let p = arg(0)?.try_sock()?.clone();
                    facts.push(ok.implies(&z.and(&[rp.write_open.eq(&p.write_open), self.net_keep_buf(&rp, &p)])));
                    facts.push(fail.implies(&rp.error.raw_eq(&z.int(4)))); // the OS refused the timeout (A2 audit)
                }
                "net_read" => {
                    let p = arg(0)?.try_sock()?.clone();
                    let mx = arg(1)?.try_ast()?.clone();
                    let sz = rp.buf.n.clone();
                    facts.push(ok.implies(&z.and(&[sz.ge_i(0), sz.le(&mx)])));
                    facts.push(ok.implies(&rp.eof.eq(&sz.raw_eq(&z.int(0)))));
                    facts.push(z.and(&[ok.clone(), p.eof.clone()]).implies(&sz.raw_eq(&z.int(0))));
                    facts.push(ok.implies(&rp.write_open.eq(&p.write_open)));
                    facts.push(fail.implies(&z.or(&[rp.error.raw_eq(&z.int(3)), rp.error.raw_eq(&z.int(6))])));
                }
                "net_read_all" => {
                    let p = arg(0)?.try_sock()?.clone();
                    facts.push(ok.implies(&z.and(&[rp.eof.clone(), rp.write_open.eq(&p.write_open)])));
                    facts.push(fail.implies(&z.or(&[rp.error.raw_eq(&z.int(3)), rp.error.raw_eq(&z.int(6))])));
                }
                _ => return Err(format!("catalog: {}", n)),
            }
        }
        for f in facts {
            self.assume(st, g, f);
        }
        Ok(res)
    }

    /// verify.py `catalog_call` (G6_SPEC 11.3): the standard catalog; every requires is a `call_requires` obligation
    /// (only in code), then an exact term or a fresh symbol with catalog facts is the result.
    fn catalog_call(&mut self, e: &Expr, x: &ast::Call, args: &[Val], st: &mut State, g: &[Ast], code: bool) -> R<Val> {
        let n = x.name.as_str();
        let rty = self.t(x.ty.as_deref());
        let z = self.z.clone();
        fn av<'v>(args: &'v [Val], i: usize) -> R<&'v Val> {
            args.get(i).ok_or_else(|| "not enough values to unpack".to_string())
        }
        let argv = move |i: usize| av(args, i);
        // need(ob): `precondition of NAME (catalog)`, in code only
        let what = format!("precondition of {} (catalog)", n);
        if n.starts_with("proc_") {
            return self.proc_call(e, n, args, st, g, code, rty);
        }
        if n.starts_with("ev_") {
            return self.async_call(e, n, args, st, g, code, rty);
        }
        if n.starts_with("net_") {
            return self.net_call(e, n, args, st, g, code, rty);
        }
        if pruv_core::foreign::is_file_observer(n) {
            let f = argv(0)?.try_file()?;
            return Ok(match n {
                "file_size" => Val::Int(f.buf.n.clone()),
                "file_byte" => {
                    let i = argv(1)?.try_ast()?;
                    if code { self.prove_core(&st, g, &z.and(&[i.ge_i(0), i.lt(&f.buf.n)]), pos(e), "call_requires", &what, None); }
                    let r = f.buf.a.select(i);
                    if code { self.assume(st, g, z.and(&[r.ge_i(0), r.le_i(255)])); }
                    Val::Int(r)
                }
                "file_ok" => Val::Bool(f.ok.clone()),
                "file_readable" => Val::Bool(f.readable.clone()),
                _ => Val::Bool(f.writable.clone()),
            });
        }
        if n.starts_with("file_") {
            let f = pruv_core::foreign::catalog_fn(n).ok_or_else(|| "unknown file operation".to_string())?;

            let mut cenv = Env::default();
            for (p, a) in f.params.iter().zip(args) { cenv.set(p.name, a.clone()); }
            for req in &f.requires {
                let ob = self.ev_ast(req, st, g, false, &cenv)?;
                if code { self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None); }
            }
            let rt = rty.ok_or_else(|| "fresh: None".to_string())?;
            let res = self.try_fresh(n, &rt)?;
            if let Some(w) = self.wf_of_val(&rt, &res)? { self.assume(st, g, w); }
            cenv.set("result", res.clone());
            for en in &f.ensures {
                let v = self.ev_ast(en, st, g, false, &cenv)?;
                self.assume(st, g, v);
            }
            return Ok(res);
        }
        match n {
            "vec_new" | "string_new" | "map_new" => {
                let rt = rty.clone().ok_or_else(|| "fresh: None".to_string())?;
                let r = self.try_fresh(n, &rt)?;
                if n == "map_new" {
                    let r = r.try_map()?.clone();
                    let ksort = self.model.map_key_sort(&r.kty)?;
                    let kt = z.const_array(&ksort, &z.bool_val(false));
                    return Ok(Val::Map(MapV { d: kt, a: r.a.clone(), n: z.int(0), name: n.to_string(), kty: r.kty.clone(), vty: r.vty.clone() }));
                }
                let r = r.try_arr()?;
                Ok(Val::Arr(r.like(r.a.clone(), z.int(0), n)))
            }
            "vec_push" => {
                let v = argv(0)?.try_arr()?.clone();
                let xv = self.field_put(&v.elem_ty(), argv(1)?)?;
                if code {
                    let ob = v.n.lt_i(MAX_LEN);
                    self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None);
                }
                Ok(Val::Arr(v.like(v.a.store(&v.n, &xv), v.n.add_i(1), n)))
            }
            "vec_clear" => {
                let v = argv(0)?.try_arr()?;
                let Expr::InoutArg(place) = &x.args[0] else { return Err("missing inout place".into()); };
                let empty = v.like(v.a.clone(), z.int(0), &v.name);
                self.store_collection_place(&place.expr, &mut st.env, Val::Arr(empty))?;
                Ok(Val::Unit)
            }
            "vec_reserve" => {
                let cnt = argv(1)?.try_ast()?;
                if code {
                    self.prove_core(&st, g, &z.and(&[cnt.ge_i(0), cnt.le_i(MAX_LEN)]), pos(e), "call_requires", &what, None);
                }
                Ok(Val::Unit)
            }
            "vec_extend_from_slice" => {
                let v = argv(0)?.try_arr()?;
                let src = argv(1)?.try_arr()?;
                let len = v.n.add(&src.n);
                if code {
                    self.prove_core(&st, g, &len.le_i(MAX_LEN), pos(e), "call_requires", &what, None);
                }
                let Expr::InoutArg(place) = &x.args[0] else { return Err("missing inout place".into()); };
                let ty = self.t(place.ty.as_deref()).ok_or("missing collection place type")?;
                let fresh = self.try_fresh(n, &ty)?;
                let a = &fresh.try_arr()?.a;
                let k = z.int_const(&format!("vec_extend!q{}", self.k));
                self.k += 1;
                let prefix = z.and(&[k.ge_i(0), k.lt(&v.n)])
                    .implies(&a.select(&k).eq(&v.a.select(&k)));
                let suffix = z.and(&[k.ge_i(0), k.lt(&src.n)])
                    .implies(&a.select(&v.n.add(&k)).eq(&src.a.select(&k)));
                self.assume(st, g, z.forall(&[k.clone()], &prefix, &[]));
                self.assume(st, g, z.forall(&[k], &suffix, &[]));
                let out = v.like(a.clone(), len, &v.name);
                self.store_collection_place(&place.expr, &mut st.env, Val::Arr(out))?;
                Ok(Val::Unit)
            }
            "string_eq" => {
                let a = argv(0)?.try_arr()?;
                let b = argv(1)?.try_arr()?;
                let result = self.try_fresh(n, &Ty::Bool)?;
                let k = z.int_const(&format!("string_eq!q{}", self.k));
                self.k += 1;
                let range = z.and(&[k.ge_i(0), k.lt(&a.n)]);
                let body = range.implies(&a.a.select(&k).eq(&b.a.select(&k)));
                let equal = z.forall(&[k], &body, &[]);
                self.assume(st, g, result.try_ast()?.eq(&z.and(&[a.n.eq(&b.n), equal])));
                Ok(result)
            }
            "vec_get" | "string_byte" => {
                let v = argv(0)?.try_arr()?.clone();
                let i = argv(1)?.try_ast()?.clone();
                if code {
                    let ob = z.and(&[i.ge_i(0), i.lt(&v.n)]);
                    self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None);
                }
                let r = v.a.select(&i);
                if v.elem_ty() == Ty::User("string".into()) {
                    let ty = v.elem_ty();
                    let text = self.field_val(&ty, r, n)?;
                    if code {
                        if let Some(wf) = self.wf_of_val(&ty, &text)? {
                            self.assume(st, g, wf);
                        }
                    }
                    return Ok(text);
                }
                match v.elem {
                    Elem::Bool => Ok(Val::Bool(r)),
                    Elem::Abs => {
                        let ty = v.elem_ty();
                        // verify.py: only a struct element read in code assumes its well-formedness
                        if code && self.model.nominal_kind(&ty) == Some(NomKind::Struct) {
                            if let Some(wf) = self.try_wf(&ty, &r)? {
                                self.assume(st, g, wf);
                            }
                        }
                        Ok(Val::Dt(r, ty))
                    },
                    Elem::I64 | Elem::U8 => {
                        // as an array read: an i64/u8 element is a machine integer
                        if code {
                            let w = in_range(&z, &r);
                            self.assume(st, g, w);
                        }
                        Ok(Val::Int(r))
                    }
                }
            }
            "map_get" | "map_lookup" => {
                let m = argv(0)?.try_map()?.clone();
                let k = self.map_key(&m.kty, argv(1)?)?;
                self.map_lookup(&m, &k, rty.as_ref())
            }
            "map_insert" => {
                let m = argv(0)?.try_map()?.clone();
                let k = self.map_key(&m.kty, argv(1)?)?;
                let xv = self.field_put(&Ty::from_gtype(&m.vty), argv(2)?)?;
                let has = m.d.select(&k);
                if code {
                    let ob = z.or(&[has.clone(), m.n.lt_i(MAX_LEN)]);
                    self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None);
                }
                let nn = m.n.add(&z.ite(&has, &z.int(0), &z.int(1)));
                Ok(Val::Map(MapV {
                    d: m.d.store(&k, &z.bool_val(true)),
                    a: m.a.store(&k, &xv),
                    n: nn,
                    name: n.to_string(),
                    kty: m.kty.clone(),
                    vty: m.vty.clone(),
                }))
            }
            "map_remove" | "map_take" => {
                let mut m = argv(0)?.try_map()?.clone();
                let k = self.map_key(&m.kty, argv(1)?)?;
                let has = m.d.select(&k);
                self.assume(st, g, has.implies(&m.n.gt_i(0)));
                let result = if n == "map_take" {
                    self.map_lookup(&m, &k, rty.as_ref())?
                } else {
                    Val::Bool(has.clone())
                };
                m.d = m.d.store(&k, &z.bool_val(false));
                m.n = m.n.sub(&z.ite(&has, &z.int(1), &z.int(0)));
                let Expr::InoutArg(place) = &x.args[0] else { return Err("missing inout place".into()) };
                self.store_collection_place(&place.expr, &mut st.env, Val::Map(m))?;
                Ok(result)
            }
            "map_updated" => {
                let aft = argv(0)?.try_map()?.clone();
                let bef = argv(1)?.try_map()?.clone();
                let k = self.map_key(&aft.kty, argv(2)?)?;
                let xv = self.field_put(&Ty::from_gtype(&aft.vty), argv(3)?)?;
                let jsort = self.model.map_key_sort(&aft.kty)?;
                let j = z.constant("map!j", &jsort);
                let c1 = aft.d.eq(&bef.d.store(&k, &z.bool_val(true)));
                let c2 = aft.n.eq(&bef.n.add(&z.ite(&bef.d.select(&k), &z.int(0), &z.int(1))));
                let body = aft.d.select(&j).implies(&aft.a.select(&j).eq(&bef.a.store(&k, &xv).select(&j)));
                let c3 = z.forall(std::slice::from_ref(&j), &body, &[]);
                Ok(Val::Bool(z.and(&[c1, c2, c3])))
            }
            "string_from" => {
                let t = argv(0)?.try_arr()?;
                if code {
                    let ob = t.n.le_i(MAX_LEN);
                    self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None);
                }
                let res_v = self.try_fresh(n, &Ty::User("string".to_string()))?;
                let res = res_v.try_arr()?.clone();
                self.assume(st, g, res.n.eq(&t.n));
                let k = z.int_const("str!k");
                let range = z.and(&[k.ge_i(0), k.lt(&t.n)]);
                let bytes_equal = res.a.select(&k).eq(&t.a.select(&k));
                self.assume(st, g, z.forall(std::slice::from_ref(&k), &range.implies(&bytes_equal), &[]));
                Ok(Val::Arr(res))
            }
            "string_push_str" => {
                let sv = argv(0)?.try_arr()?.clone();
                let t = argv(1)?.try_arr()?.clone();
                if code {
                    let ob = sv.n.add(&t.n).le_i(MAX_LEN);
                    self.prove_core(&st, g, &ob, pos(e), "call_requires", &what, None);
                }
                let res_v = self.try_fresh(n, &Ty::User("string".to_string()))?;
                let res = res_v.try_arr()?.clone();
                let len_fact = res.n.eq(&sv.n.add(&t.n));
                self.assume(st, g, len_fact);
                let k = z.int_const("str!k");
                let rng1 = z.and(&[k.ge_i(0), k.lt(&sv.n)]);
                let body1 = res.a.select(&k).eq(&sv.a.select(&k));
                let f1 = z.forall(std::slice::from_ref(&k), &rng1.implies(&body1), &[]);
                self.assume(st, g, f1);
                let rng2 = z.and(&[k.ge_i(0), k.lt(&t.n)]);
                let body2 = res.a.select(&sv.n.add(&k)).eq(&t.a.select(&k));
                let f2 = z.forall(std::slice::from_ref(&k), &rng2.implies(&body2), &[]);
                self.assume(st, g, f2);
                Ok(Val::Arr(res))
            }
            "string_to_str" => {
                let a = argv(0)?.try_arr()?;
                Ok(Val::Arr(Arr::new(a.a.clone(), a.n.clone(), n.to_string(), Elem::U8)))
            }
            other => Err(format!("catalog: {}", other)),
        }
    }

    fn store_collection_place(&self, mut place: &Expr, env: &mut Env, value: Val) -> R<()> {
        let mut path = Vec::new();
        while let Expr::FieldAccess(f) = place {
            path.push(f.field.as_str());
            place = &f.base;
        }
        let Expr::Var(root) = place else { return Err("invalid collection place".into()) };
        if path.is_empty() {
            env.set(&root.name, value);
        } else {
            path.reverse();
            let Val::Dt(term, ty) = env.get(&root.name) else { return Err("invalid collection root".into()) };
            let rebuilt = self.try_rebuild_v(ty, term, &path, &value)?;
            env.set(&root.name, Val::Dt(rebuilt, ty.clone()));
        }
        Ok(())
    }

    fn map_key(&self, ty: &GType, value: &Val) -> R<Ast> {
        if ty.to_key() == "string" {
            let key = value.try_arr()?;
            Ok(self.model.text_map_key(&key.n, &key.a))
        } else {
            Ok(value.try_ast()?.clone())
        }
    }

    /// verify.py `map_lookup`: `If(m.d[k], Some(m.a[k]), None)` as an Option<V> of the call's own result type.
    fn map_lookup(&mut self, m: &MapV, k: &Ast, rty: Option<&Ty>) -> R<Val> {
        let o = self.model.opt(rty)?;
        let r = self.z.ite(&m.d.select(k), &o.some.apply1(&m.a.select(k)), &o.none);
        Ok(Val::Dt(r, rty.cloned().unwrap_or(Ty::Opt)))
    }

    // =========================================================================================
    // Domain of a contract (lines 440-543)
    // =========================================================================================

    /// verify.py `dom(e, st, g, env, kind)`: the condition under which all partial operations of contract e are defined (short-circuit order). None=trivial.
    pub(crate) fn dom(&mut self, e: &Expr, st: &mut State, g: &[Ast], env: &Env, kind: PKind) -> R<Option<Ast>> {
        if !has_vf(e, kind) {
            return Ok(None);
        }
        let wo = kind != PKind::Variant; // None / "other"
        let wv = kind != PKind::Other; // None / "variant"
        let model: &'a Model = self.model;
        match e {
            Expr::Unary(x) => self.dom(&x.operand, st, g, env, kind),
            Expr::Binary(x) => {
                let dl = self.dom(&x.left, st, g, env, kind)?;
                if matches!(x.op.as_str(), "&&" | "||" | "==>") {
                    let l = self.ev_ast(&x.left, st, g, false, env)?;
                    let dr = self.dom(&x.right, st, g, env, kind)?;
                    let guard = if x.op == "||" { l.not() } else { l };
                    return Ok(conj(&self.z, vec![dl, imp(&guard, dr)]));
                }
                let dr = self.dom(&x.right, st, g, env, kind)?;
                let mut parts = vec![dl, dr];
                if wo && (x.op == "/" || x.op == "%") {
                    let r = self.ev_ast(&x.right, st, g, false, env)?;
                    parts.push(Some(r.ne_i(0)));
                }
                if wo && bit_operator(&x.op) {
                    let l = self.ev_ast(&x.left, st, g, false, env)?;
                    let r = self.ev_ast(&x.right, st, g, false, env)?;
                    parts.push(Some(in_range(&self.z, &l)));
                    if matches!(x.op.as_str(), "<<" | ">>") {
                        parts.push(Some(self.z.and(&[r.ge_i(0), r.lt_i(64)])));
                        if x.op == "<<" {
                            let fits = if let Some(factor) = shift_factor(&x.right) {
                                in_range(&self.z, &l.mul(&self.z.int_str(&factor.to_string())))
                            } else {
                                let a = l.int2bv(64);
                                let k = r.int2bv(64);
                                a.bvshl(&k).bvashr(&k).raw_eq(&a)
                            };
                            parts.push(Some(fits));
                        }
                    } else {
                        parts.push(Some(in_range(&self.z, &r)));
                    }
                }
                Ok(conj(&self.z, parts))
            }
            Expr::Index(x) => {
                let p0 = self.dom(&x.array, st, g, env, kind)?;
                let p1 = self.dom(&x.index, st, g, env, kind)?;
                let mut parts = vec![p0, p1];
                if wo {
                    let arr = self.ev(&x.array, st, g, false, env)?;
                    let arr = arr.try_arr()?.clone();
                    let i = self.ev_ast(&x.index, st, g, false, env)?;
                    parts.push(Some(self.z.and(&[i.ge_i(0), i.lt(&arr.n)])));
                }
                Ok(conj(&self.z, parts))
            }
            Expr::Ctor(x) => match &x.arg {
                Some(a) => self.dom(a, st, g, env, kind),
                None => Ok(None),
            },
            Expr::Call(x) => {
                let mut parts: Vec<Option<Ast>> = Vec::new();
                for a in &x.args {
                    parts.push(self.dom(a, st, g, env, kind)?);
                }
                if wo && x.name == "file_byte" {
                    let f = self.ev(&x.args[0], st, g, false, env)?;
                    let i = self.ev_ast(&x.args[1], st, g, false, env)?;
                    parts.push(Some(self.z.and(&[i.ge_i(0), i.lt(&f.try_file()?.buf.n)])));
                }
                if wo && is_proc_dom(&x.name) {
                    // G10 proc A1 (4.9): a valid stream, and for proc_byte a valid index
                    let pv = self.ev(&x.args[0], st, g, false, env)?;
                    let sm = self.ev_ast(&x.args[1], st, g, false, env)?;
                    parts.push(Some(self.proc_stream_ok(&sm)));
                    if x.name == "proc_byte" {
                        let i = self.ev_ast(&x.args[2], st, g, false, env)?;
                        let size = self.proc_size_t(pv.try_proc()?, &sm);
                        parts.push(Some(self.z.and(&[i.ge_i(0), i.lt(&size)])));
                    }
                }
                if wo && is_async_dom(&x.name) {
                    let lv = self.ev(&x.args[0], st, g, false, env)?;
                    let i = self.ev_ast(&x.args[1], st, g, false, env)?;
                    let p = lv.try_loop()?;
                    parts.push(Some(self.z.and(&[i.ge_i(0), i.lt(if x.name == "ev_byte" { &p.buf.n } else { &p.cap })])));
                }
                if wo && is_net_dom(&x.name) {
                    // G10 net A1 (4.9): net_byte needs a valid index
                    let sv = self.ev(&x.args[0], st, g, false, env)?;
                    let i = self.ev_ast(&x.args[1], st, g, false, env)?;
                    parts.push(Some(self.z.and(&[i.ge_i(0), i.lt(&sv.try_sock()?.buf.n)])));
                }
                if wo && matches!(x.name.as_str(), "load_be" | "load_le") {
                    // P2 batch 4: the word lies inside the array
                    let arr = self.ev(&x.args[0], st, g, false, env)?;
                    let arr = arr.try_arr()?.clone();
                    let i = self.ev_ast(&x.args[1], st, g, false, env)?;
                    parts.push(Some(self.z.and(&[i.ge_i(0), i.add_i(lit_width(&x.args[2])).le(&arr.n)])));
                }
                if wo && is_bit_z3(&x.name) {
                    // P2: both arguments inside i64
                    let a0 = self.ev_ast(&x.args[0], st, g, false, env)?;
                    let a1 = self.ev_ast(&x.args[1], st, g, false, env)?;
                    parts.push(Some(self.z.and(&[
                        a0.ge_i(i64::MIN),
                        a0.le_i(i64::MAX),
                        a1.ge_i(i64::MIN),
                        a1.le_i(i64::MAX),
                    ])));
                    if x.name == "wrapping_shl" || x.name == "ushr" {
                        // shift count in 0..64
                        parts.push(Some(self.z.and(&[a1.ge_i(0), a1.lt_i(64)])));
                    }
                }
                if wo && is_tag_fn(&x.name) {
                    let a0 = x.args.first().ok_or_else(|| "tuple index out of range".to_string())?;
                    let vv = self.ev(a0, st, g, false, env)?;
                    // G3: the Option/Result instance of the argument
                    let (v, vty) = dt_arg(&vv)?;
                    if x.name == "some_val" {
                        parts.push(Some(model.opt(Some(vty))?.is_some.apply1(v)));
                    } else {
                        let ro = model.res(Some(vty))?;
                        let ok = ro.is_ok.apply1(v);
                        parts.push(Some(if x.name == "ok_val" { ok } else { ok.not() }));
                    }
                }
                Ok(conj(&self.z, parts))
            }
            Expr::StructLit(x) => {
                let mut parts: Vec<Option<Ast>> = Vec::new();
                for fi in &x.fields {
                    parts.push(self.dom(&fi.value, st, g, env, kind)?);
                }
                Ok(conj(&self.z, parts))
            }
            Expr::EnumCtor(x) => {
                let mut parts: Vec<Option<Ast>> = Vec::new();
                for a in &x.args {
                    parts.push(self.dom(a, st, g, env, kind)?);
                }
                Ok(conj(&self.z, parts))
            }
            Expr::FieldAccess(x) => self.dom(&x.base, st, g, env, kind),
            Expr::IsVariant(x) => self.dom(&x.value, st, g, env, kind),
            Expr::VariantField(x) => {
                let inner = self.dom(&x.value, st, g, env, kind)?;
                if !wv {
                    return Ok(inner);
                }
                let vv = self.ev(&x.value, st, g, false, env)?;
                let ety = enum_ty(&x.enum_name, &vv);
                let vf = model.variant_fns(&ety, &x.variant)?;
                Ok(conj(&self.z, vec![inner, Some(vf.rec.apply1(vv.try_ast()?))]))
            }
            Expr::Quant(x) => {
                let lo = self.ev_ast(&x.lo, st, g, false, env)?;
                let hi = self.ev_ast(&x.hi, st, g, false, env)?;
                let dlo = self.dom(&x.lo, st, g, env, kind)?;
                let dhi = self.dom(&x.hi, st, g, env, kind)?;
                let mut outer = vec![dlo, dhi];
                if wo && (x.kind == "max" || x.kind == "min") {
                    outer.push(Some(lo.lt(&hi)));
                }
                let outer = conj(&self.z, outer);
                self.k += 1;
                let k = self.z.int_const(&format!("{}!dk{}", x.var, self.k));
                let mut envk = env.clone();
                envk.set(&x.var, Val::Int(k.clone()));
                self.qstack.push(k.clone());
                let dbody = self.dom(&x.body, st, g, &envk, kind);
                self.qstack.pop();
                let Some(dbody) = dbody? else { return Ok(outer) };
                let mut rng = self.z.and(&[k.ge(&lo), k.lt(&hi)]);
                if x.kind == "forall" || x.kind == "exists" {
                    // The body at k is evaluated only when all earlier bodies are true (forall) / false (exists).
                    // The number in the name is the current s.k after computing dbody (line 512).
                    let j = self.z.int_const(&format!("{}!dj{}", x.var, self.k));
                    let mut envj = env.clone();
                    envj.set(&x.var, Val::Int(j.clone()));
                    self.qstack.push(j.clone());
                    let bj = self.ev_ast(&x.body, st, g, false, &envj);
                    self.qstack.pop();
                    let bj = bj?;
                    let cond = self.z.and(&[j.ge(&lo), j.lt(&k)]);
                    let inner = if x.kind == "forall" { bj } else { bj.not() };
                    let prior = self.z.forall(std::slice::from_ref(&j), &cond.implies(&inner), &[]);
                    rng = self.z.and(&[rng, prior]);
                }
                let fa = self.z.forall(std::slice::from_ref(&k), &rng.implies(&dbody), &[]);
                Ok(conj(&self.z, vec![outer, Some(fa)]))
            }
            _ => Ok(None),
        }
    }

    /// verify.py `dom_obligations` (line 524): variant_field → variant_domain, other partial operations → contract_domain.
    fn dom_obligations(&mut self, e: &Expr, st: &mut State, g: &[Ast], env: &Env) -> R<Vec<(Ast, &'static str)>> {
        let mut out: Vec<(Ast, &'static str)> = Vec::new();
        for (kind, code) in [(PKind::Variant, "variant_domain"), (PKind::Other, "contract_domain")] {
            if !has_vf(e, kind) {
                continue;
            }
            let Some(d) = self.dom(e, st, g, env, kind)? else { continue };
            if d.simplify().is_true() {
                continue;
            }
            out.push((d, code));
        }
        Ok(out)
    }

    /// verify.py `check_dom` (line 537): proves D(e) separately per code and returns the domains proved.
    pub(crate) fn check_dom_e(
        &mut self,
        st: &mut State,
        g: &[Ast],
        e: &Expr,
        node: Pos,
        what: &str,
        env: Option<&Env>,
    ) -> R<Vec<Ast>> {
        self.check_dom_cand(st, g, e, node, what, env, None)
    }

    /// `check_dom_e` + STAGE1 B2 candidate ID (`cand`: both the duplicate-suppression key and the recording tag).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_dom_cand(
        &mut self,
        st: &mut State,
        g: &[Ast],
        e: &Expr,
        node: Pos,
        what: &str,
        env: Option<&Env>,
        cand: Option<&str>,
    ) -> R<Vec<Ast>> {
        let envc: Env = match env {
            Some(x) => x.clone(),
            None => st.env.clone(),
        };
        let obs = self.dom_obligations(e, st, g, &envc)?;
        for (d, code) in &obs {
            let w = if *code == "variant_domain" {
                what.to_string()
            } else {
                what.replace(
                    "variant_field lookups",
                    "partial operations (index, /, %, some_val/ok_val/err_val, max/min)",
                )
            };
            self.prove_core_cand(&st, g, d, node, code, &w, cand, cand);
        }
        Ok(obs.into_iter().map(|(d, _)| d).collect())
    }

    // =========================================================================================
    // Statements (lines 647-833)
    // =========================================================================================

    /// verify.py `ensures_ok` (line 647).
    fn ensures_ok(&mut self, st: &mut State, val: Option<Val>, node: Pos) -> R<()> {
        let f = self.f.ok_or_else(|| "no function".to_string())?;
        if let Some(cost) = &f.cost {
            let ob = st.env.get("spent").ast().le(self.cost0.as_ref().ok_or_else(|| "no entry cost".to_string())?);
            self.prove_core(&st, &[], &ob, node, "cost_bound",
                &format!("spent <= cost `{}` (C at entry)", expr_text(cost)), None);
        }
        let mut env = st.env.clone();
        for p in &f.params {
            // G5 A1: ordinary parameters denote their entry values; an inout parameter keeps its current (final) value,
            // and old(a) resolves through s.snap
            if p.mode_pos.is_none() {
                let v = self.params.try_get(&p.name).cloned().ok_or_else(|| format!("'{}'", p.name))?;
                env.set(&p.name, v);
            }
        }
        if let Some(v) = val {
            if !matches!(v, Val::Unit) {
                env.set("result", v);
            }
        }
        let mut prev: Vec<Ast> = Vec::new(); // earlier clauses (domain + value) were proved above and can justify lookups in later clauses
        // LG16: the synthesized struct-invariant ensures come before the user's (inout parameters on their final values)
        for (i, ic) in self.inv_clauses(f, true).into_iter().enumerate() {
            let what = if ic.inout {
                format!("invariant of {} (line {}) for inout {} at exit", ic.sname, pruv_core::diagnostic::plain_line(ic.line), ic.who)
            } else {
                format!("invariant of {} (line {}) for {}", ic.sname, pruv_core::diagnostic::plain_line(ic.line), ic.who)
            };
            for (d, code) in self.dom_obligations(&ic.expr, st, &prev, &env)? {
                self.prove_core(&st, &prev, &d, node, "struct_invariant", &what, Some(&format!("({}, '{}')", i, code)));
                prev.push(d);
            }
            let ob = self.ev_ast(&ic.expr, st, &[], false, &env)?;
            self.prove_core(&st, &[], &ob, node, "struct_invariant", &what, Some(&i.to_string()));
            prev.push(ob);
        }
        for en in &f.ensures {
            let what = format!(
                "variant_field lookups in postcondition (line {}) are defined",
                pruv_core::diagnostic::plain_line(en.line())
            );
            let ds = self.check_dom_e(st, &prev, en, node, &what, Some(&env))?;
            prev.extend(ds);
            let ob = self.ev_ast(en, st, &[], false, &env)?;
            self.prove_core(&st, &[], &ob, node, "ensures", &format!("postcondition (line {})", pruv_core::diagnostic::plain_line(en.line())), None);
            prev.push(ob);
        }
        Ok(())
    }

    /// verify.py `exec_block` (line 660).
    pub(crate) fn exec_block(&mut self, stmts: &[Stmt], st: State) -> R<Vec<State>> {
        let mut states = vec![st];
        for stmt in stmts {
            // STAGE1 B2: on budget exhaustion or timeout, Python escapes from here via an exception.
            if self.probe_aborted() {
                return Ok(Vec::new());
            }
            let mut nxt: Vec<State> = Vec::new();
            for cur in states {
                nxt.extend(self.exec(stmt, cur)?);
            }
            states = nxt;
            if states.is_empty() {
                break;
            }
        }
        Ok(states)
    }

    /// Let / Assign (lines 711-717).
    fn exec_bind(&mut self, name: &str, value: &Expr, st: State) -> R<Vec<State>> {
        let mut st = st.fork(None);
        let v = self.ev0(value, &mut st, &[], true)?;
        let v = match v {
            Val::Arr(a) => Val::Arr(a.like(a.a.clone(), a.n.clone(), name)),
            other => other,
        };
        st.env.set(name, v);
        Ok(vec![st])
    }

    /// verify.py `exec` (line 710).
    fn exec(&mut self, stmt: &Stmt, st: State) -> R<Vec<State>> {
        if self.probe_aborted() {
            return Ok(Vec::new());
        }
        let model: &'a Model = self.model;
        let spos = stmt_pos(stmt);
        match stmt {
            Stmt::Let(s) => self.exec_bind(&s.name, &s.value, st),
            Stmt::Assign(s) => self.exec_bind(&s.name, &s.value, st),
            Stmt::IndexAssign(s) if !s.fields.is_empty() => {
                // LG4: `p.a.b[i] = e`: the field array is read with the ordinary field read (an Arr from the Buf field);
                // index, then value, in the state before the write; same obligations as `x[i] = v`
                let mut st = st.fork(None);
                let root = st.env.try_get(&s.name).ok_or_else(|| format!("'{}'", s.name))?.clone();
                let Val::Dt(root_ast, root_ty) = root.clone() else {
                    return Err(format!("element store through '{}' which is not a struct value", s.name));
                };
                let mut cur = root;
                for field in &s.fields {
                    let Val::Dt(va, sty) = cur else { return Err("element store through a non-struct value".to_string()) };
                    let sf = model.struct_info(&sty)?;
                    let idx = model.field_index_of(&sty, field, None)?;
                    let fty = &model.fields_of(&sty, None)?[idx].1;
                    self.observe(&mut st, &va, Some(&sty), false)?;
                    cur = self.field_val(fty, sf.accs[idx].apply1(&va), field)?;
                }
                let arr = cur.try_arr()?.clone();
                let place = format!("{}.{}", s.name, s.fields.join("."));
                let i = self.ev0_ast(&s.index, &mut st, &[], true)?;
                let v = self.ev0_ast(&s.value, &mut st, &[], true)?;
                let ob = self.z.and(&[i.ge_i(0), i.lt(&arr.n)]);
                self.prove_core(&st, &[], &ob, spos, "index_oob", &format!("index in bounds for {}[...] =", place), None);
                if arr.elem == Elem::U8 {
                    let ob = self.z.and(&[v.ge_i(0), v.le_i(255)]);
                    self.prove_core(
                        &st,
                        &[],
                        &ob,
                        spos,
                        "elem_range",
                        &format!("u8 element range 0..=255 for {}[...] =", place),
                        None,
                    );
                }
                if let Some((lo, hi, what)) = &s.elem_refine {
                    let lo: i64 = lo.parse().map_err(|_| format!("refinement bound '{}'", lo))?;
                    let hi: i64 = hi.parse().map_err(|_| format!("refinement bound '{}'", hi))?;
                    let ob = self.z.and(&[v.ge_i(lo), v.le_i(hi)]);
                    self.prove_core(&st, &[], &ob, spos, "refinement_assign", what.as_str(), Some(what.as_str()));
                }
                let last = s.fields.last().map(|f| f.as_str()).unwrap_or("");
                let new = Val::Arr(arr.like(arr.a.store(&i, &v), arr.n.clone(), last));
                let path: Vec<&str> = s.fields.iter().map(|f| f.as_str()).collect();
                let rebuilt = self.try_rebuild_v(&root_ty, &root_ast, &path, &new)?;
                st.env.set(&s.name, Val::Dt(rebuilt, root_ty));
                Ok(vec![st])
            }
            Stmt::IndexAssign(s) => {
                let mut st = st.fork(None);
                let arr = st.env.try_get(&s.name).ok_or_else(|| format!("'{}'", s.name))?.try_arr()?.clone();
                let i = self.ev0_ast(&s.index, &mut st, &[], true)?;
                let v = self.ev0_ast(&s.value, &mut st, &[], true)?;
                let ob = self.z.and(&[i.ge_i(0), i.lt(&arr.n)]);
                self.prove_core(
                    &st,
                    &[],
                    &ob,
                    spos,
                    "index_oob",
                    &format!("index in bounds for {}[...] =", s.name),
                    None,
                );
                if arr.elem == Elem::U8 {
                    let ob = self.z.and(&[v.ge_i(0), v.le_i(255)]);
                    self.prove_core(
                        &st,
                        &[],
                        &ob,
                        spos,
                        "elem_range",
                        &format!("u8 element range 0..=255 for {}[...] =", s.name),
                        None,
                    );
                }
                if let Some((lo, hi, what)) = &s.elem_refine {
                    // refine.rs: one range obligation on the stored value (already computed). No domain check, re-evaluation or forall, and it is not assumed afterwards.
                    let lo: i64 = lo.parse().map_err(|_| format!("refinement bound '{}'", lo))?;
                    let hi: i64 = hi.parse().map_err(|_| format!("refinement bound '{}'", hi))?;
                    let ob = self.z.and(&[v.ge_i(lo), v.le_i(hi)]);
                    self.prove_core(&st, &[], &ob, spos, "refinement_assign", what.as_str(), Some(what.as_str()));
                }
                st.env.set(&s.name, Val::Arr(arr.like(arr.a.store(&i, &v), arr.n.clone(), &s.name)));
                Ok(vec![st])
            }
            Stmt::If(s) => {
                let mut st = st.fork(None);
                let c = self.ev0_ast(&s.cond, &mut st, &[], true)?;
                let mut out = self.exec_block(&s.then, st.fork(Some(c.clone())))?;
                out.extend(self.exec_block(&s.els, st.fork(Some(c.not())))?);
                Ok(out)
            }
            Stmt::Match(s) => {
                let mut st = st.fork(None);
                let mut collection_update = None;
                let sv = if let Expr::Call(call) = &s.scrutinee {
                    if matches!(call.name.as_str(), "map_get_mut" | "vec_at_mut") && !self.funcs.contains_key(&call.name) {
                        let Expr::InoutArg(arg) = &call.args[0] else { return Err("missing inout place".into()) };
                        let collection = self.ev0(&arg.expr, &mut st, &[], true)?;
                        let index = self.ev0(&call.args[1], &mut st, &[], true)?;
                        let ty = Ty::from_name(call.ty.as_deref().ok_or("missing match result type")?);
                        let (key, value) = if call.name == "vec_at_mut" {
                            let v = collection.try_arr()?;
                            let i = index.try_ast()?;
                            let opt = self.model.opt(Some(&ty))?;
                            let has = self.z.and(&[i.ge_i(0), i.lt(&v.n)]);
                            (i.clone(), Val::Dt(self.z.ite(&has, &opt.some.apply1(&v.a.select(i)), &opt.none), ty))
                        } else {
                            let m = collection.try_map()?;
                            let k = self.map_key(&m.kty, &index)?;
                            let value = self.map_lookup(m, &k, Some(&ty))?;
                            (k, value)
                        };
                        collection_update = Some((arg.expr.as_ref(), collection, key));
                        value
                    } else {
                        self.ev0(&s.scrutinee, &mut st, &[], true)?
                    }
                } else {
                    self.ev0(&s.scrutinee, &mut st, &[], true)?
                };
                let v = sv.try_ast()?.clone();
                let outer: HashSet<String> = st.env.0.iter().map(|(k, _)| k.clone()).collect();
                let mut out: Vec<State> = Vec::new();
                // G3: the scrutinee's complete (possibly generic) type
                let sty: Option<Ty> = match &sv {
                    Val::Dt(_, t) => Some(t.clone()),
                    _ => None,
                };
                if model.is_g9() {
                    self.observe(&mut st, &v, sty.as_ref(), false)?; // G9 A1: one-step facts of the matched value
                }
                for a in &s.arms {
                    let cond: Ast;
                    let vals: Vec<Ast>;
                    let ftys: Vec<Ty>;
                    if let Some(en) = &a.enum_name {
                        let ety = match &sty {
                            Some(t) if t.name() == en.as_str() => t.clone(),
                            _ => Ty::User(en.clone()),
                        };
                        let vf = model.variant_fns(&ety, &a.ctor)?;
                        cond = vf.rec.apply1(&v);
                        vals = vf.accs.iter().map(|acc| acc.apply1(&v)).collect();
                        ftys = model.fields_of(&ety, Some(&a.ctor))?.into_iter().map(|(_, ft)| ft).collect();
                    } else {
                        let (c, val) = if a.ctor == "Some" || a.ctor == "None" {
                            let o = model.opt(sty.as_ref())?;
                            if a.ctor == "Some" {
                                (o.is_some.apply1(&v), Some(o.sval.apply1(&v)))
                            } else {
                                (o.is_none.apply1(&v), None)
                            }
                        } else {
                            let o = model.res(sty.as_ref())?;
                            if a.ctor == "Ok" {
                                (o.is_ok.apply1(&v), Some(o.oval.apply1(&v)))
                            } else {
                                (o.is_err.apply1(&v), Some(o.eval.apply1(&v)))
                            }
                        };
                        cond = c;
                        vals = val.into_iter().collect();
                        let ft = payload_ty(sty.as_ref(), a.ctor == "Err");
                        ftys = vals.iter().map(|_| ft.clone()).collect();
                    }
                    let mut b = st.fork(Some(cond));
                    for ((name, val), fty) in a.binders.iter().zip(vals.iter()).zip(ftys.iter()) {
                        if model.is_g9() {
                            // G9 A1: buffer payloads become Arr views; nominal payloads are observed
                            let vv = self.field_val(fty, val.clone(), name)?;
                            if let Val::Dt(da, dt) = &vv {
                                self.observe(&mut b, da, Some(dt), false)?;
                            }
                            b.env.set(name, vv.clone());
                            if let Some(w) = self.wf_of_val(fty, &vv)? {
                                b.pc.push(w);
                            }
                            continue;
                        }
                        b.env.set(name, self.field_val(fty, val.clone(), name)?);
                        if let Some(w) = self.wf_field(fty, val)? {
                            b.pc.push(w);
                        }
                    }
                    for mut end in self.exec_block(&a.body, b)? {
                        if let (Some((place, before, key)), Some(name)) = (&collection_update, a.binders.first()) {
                            let value = end.env.get(name).try_ast()?;
                            let after = match before {
                                Val::Map(m) => {
                                    let mut m = m.clone();
                                    m.a = m.a.store(key, value);
                                    Val::Map(m)
                                }
                                Val::Arr(v) => Val::Arr(v.like(v.a.store(key, value), v.n.clone(), &v.name)),
                                _ => return Err("invalid mutable collection".into()),
                            };
                            self.store_collection_place(place, &mut end.env, after)?;
                        }
                        // The binder and the arm's local variables end with the arm; only updates to outer variables remain
                        let drop_keys: Vec<String> =
                            end.env.0.iter().filter(|(k, _)| !outer.contains(k)).map(|(k, _)| k.clone()).collect();
                        for k in drop_keys {
                            end.env.remove(&k);
                        }
                        out.push(end);
                    }
                }
                Ok(out)
            }
            Stmt::FieldAssign(s) => {
                let mut st = st.fork(None);
                // right-hand side first, in the state before storing (G9 A1: an environment value, an Arr for a buffer field)
                let v = self.ev0(&s.value, &mut st, &[], true)?;
                let (root, ty) = match st.env.try_get(&s.name) {
                    Some(Val::Dt(r, t)) => (r.clone(), t.clone()),
                    _ => return Err(format!("field assignment on '{}' which is not a struct value", s.name)),
                };
                let path: Vec<&str> = s.fields.iter().map(|f| f.as_str()).collect();
                let new = self.try_rebuild_v(&ty, &root, &path, &v)?;
                st.env.set(&s.name, Val::Dt(new, ty));
                Ok(vec![st])
            }
            Stmt::Return(s) => {
                let mut st = st.fork(None);
                let v = match &s.value {
                    Some(e) => Some(self.ev0(e, &mut st, &[], true)?),
                    None => None,
                };
                self.ensures_ok(&mut st, v, spos)?;
                Ok(Vec::new())
            }
            Stmt::Assert(s) if s.reveal.is_some() => {
                // P13o: proof-only, no query
                Ok(vec![self.reveal(&s.reveal.as_ref().unwrap().0, &st)?])
            }
            Stmt::Assert(s) => {
                let mut st = st;
                let ob = self.check_assert(s, &mut st)?;
                Ok(vec![st.fork(Some(ob))])
            }
            Stmt::Par(p) => Ok(vec![self.par(p, &st)?]),
            Stmt::ExprStmt(s) => {
                let mut st = st.fork(None);
                self.ev0(&s.expr, &mut st, &[], true)?;
                Ok(vec![st])
            }
            Stmt::While(w) => {
                let mut st = st;
                self.loop_(Loop::While(w), &mut st)
            }
            Stmt::ForMap(w) => {
                let mut st = st;
                self.iter_places.push(w.place.clone());
                let result = self.loop_(Loop::Map(w), &mut st);
                self.iter_places.pop();
                result
            }
        }
    }

    /// Loop start state: havoc the assigned variables and assume each invariant's domain (variant) and value.
    fn loop_assume_invariants(&mut self, invs: &[InvRef<'_>], h: &mut State) -> R<()> {
        for &(_cid, inv, _text) in invs {
            let envh = h.env.clone();
            if let Some(d) = self.dom(inv, h, &[], &envh, PKind::Variant)? {
                h.pc.push(d);
            }
            let v = self.ev0_ast(inv, h, &[], false)?;
            h.pc.push(v);
        }
        Ok(())
    }

    /// verify.py `loop` (line 786).
    fn loop_(&mut self, head: Loop<'_>, st: &mut State) -> R<Vec<State>> {
        if self.probe_aborted() {
            return Ok(Vec::new());
        }
        let (invariants, body) = match head {
            Loop::While(w) => (&w.invariants, &w.body),
            Loop::Map(w) => (&w.invariants, &w.body),
        };
        // Append inferred candidates after the written invariants of a while loop.
        let extra_here = match head {
            Loop::While(w) if !self.extra.is_empty() => {
                let n = self.loop_no.get(&(w as *const ast::While as usize))
                    .ok_or_else(|| "loop number not found".to_string())?;
                self.extra.get(n).cloned()
            }
            _ => None,
        }.unwrap_or_default();
        let mut invs: Vec<InvRef<'_>> = invariants.iter().map(|i| (None, i, None)).collect();
        for (id, e, t) in extra_here.iter() {
            invs.push((Some(id.as_str()), e, Some(t.as_str())));
        }
        let mut prev: Vec<Ast> = Vec::new(); // invariants in order: an earlier one (proved here) can justify lookups in a later one
        for &(cid, inv, text) in &invs {
            let (w_dom, w_ob) = inv_what(cid, text, "entry");
            let ds = self.check_dom_cand(st, &prev, inv, pos(inv), &w_dom, None, cid)?;
            prev.extend(ds);
            let ob = self.ev0_ast(inv, st, &[], false)?;
            self.prove_core_cand(&st, &[], &ob, pos(inv), "invariant_entry", &w_ob, cid, cid);
            prev.push(ob);
        }
        let mut names: BTreeSet<String> = BTreeSet::new();
        assigned(body, &mut names);
        if self.has_cost() { names.insert("spent".to_string()); }
        // G5 A1 (5.3): inout writes keep the loop-head length unless the
        // body can replace the whole array or call a resizing helper.
        // Without inout, calls = {} and snap = {}: names/keep (and the SMT) are exactly the old ones.
        let mut calls: BTreeSet<String> = BTreeSet::new();
        call_writes(body, &mut calls, &|call, _| call.name != "vec_reserve");
        let mut keep: BTreeSet<String> = BTreeSet::new();
        if !calls.is_empty() || !self.snap.0.is_empty() {
            let mut whole: BTreeSet<String> = BTreeSet::new();
            whole_assigned(body, &mut whole);
            names.extend(calls.iter().cloned());
            let mut resizes = BTreeSet::new();
            call_writes(body, &mut resizes, &|call, i| {
                pruv_core::check::vec_resize_builtin(&call.name) || self.resize_params.contains(&(call.name.clone(), i))
            });
            for n in &names {
                if (self.snap.contains(n) || calls.contains(n)) && !whole.contains(n) && !resizes.contains(n) {
                    keep.insert(n.clone());
                }
            }
        }
        // An arbitrary single iteration: at the start the invariants held (and were defined)
        let mut b = self.havoc_set(st, &names, &keep)?;
        self.loop_assume_invariants(&invs, &mut b)?;
        let measure = match head {
            Loop::While(w) => {
                let c = self.ev0_ast(&w.cond, &mut b, &[], true)?;
                b.pc.push(c);
                let dpos = pos(&w.decreases);
                self.check_dom_e(&mut b, &[], &w.decreases, dpos, "variant_field lookups in decreases are defined", None)?;
                let d0 = self.ev0_ast(&w.decreases, &mut b, &[], false)?;
                let ob = d0.ge_i(0);
                self.prove_core(&b, &[], &ob, dpos, "decreases_nonneg", "decreases >= 0", None);
                Some((&w.decreases, d0))
            }
            Loop::Map(w) => {
                self.map_iteration(w, &mut b)?;
                None
            }
        };
        self.spend(&mut b, &[], self.z.int(1));
        for mut end in self.exec_block(body, b)? {
            let mut prev: Vec<Ast> = Vec::new();
            for &(cid, inv, text) in &invs {
                let (w_dom, w_ob) = inv_what(cid, text, "preserved");
                let ds = self.check_dom_cand(&mut end, &prev, inv, pos(inv), &w_dom, None, cid)?;
                prev.extend(ds);
                let ob = self.ev0_ast(inv, &mut end, &[], false)?;
                self.prove_core_cand(&end, &[], &ob, pos(inv), "invariant_preserved", &w_ob, cid, cid);
                prev.push(ob);
            }
            let Some((decreases, d0)) = &measure else { continue };
            let dpos = pos(decreases);
            if has_vf(decreases, PKind::All) {
                // decreases is read at the end of the body: the invariants just proved can justify that lookup
                let mut extra: Vec<Ast> = Vec::new();
                for &(_cid, inv, _text) in &invs {
                    let envp = end.env.clone();
                    if let Some(d) = self.dom(inv, &mut end, &[], &envp, PKind::All)? {
                        extra.push(d);
                    }
                    let v = self.ev0_ast(inv, &mut end, &[], false)?;
                    extra.push(v);
                }
                self.check_dom_e(
                    &mut end,
                    &extra,
                    decreases,
                    dpos,
                    "variant_field lookups in decreases are defined",
                    None,
                )?;
            }
            let dv = self.ev0_ast(decreases, &mut end, &[], false)?;
            let ob = dv.lt(d0);
            self.prove_core(&end, &[], &ob, dpos, "decreases_decrease", "decreases strictly decreases", None);
        }
        // After the loop
        let mut a = self.havoc_set(st, &names, &keep)?;
        self.loop_assume_invariants(&invs, &mut a)?;
        if let Loop::While(w) = head {
            let c = self.ev0_ast(&w.cond, &mut a, &[], true)?;
            a.pc.push(c.not());
        }
        Ok(vec![a])
    }

    fn map_iteration(&mut self, w: &ast::ForMap, st: &mut State) -> R<()> {
        let map = self.ev0(&w.place, st, &[], true)?;
        let map = map.try_map()?;
        let key_ty = Ty::from_gtype(&map.kty);
        let val_ty = Ty::from_gtype(&map.vty);
        let key = self.try_fresh(&w.key, &key_ty)?;
        let index = self.map_key(&map.kty, &key)?;
        let value = self.field_val(&val_ty, map.a.select(&index), &w.val)?;
        st.pc.push(map.d.select(&index));
        for (name, ty, term) in [(&w.key, key_ty, key), (&w.val, val_ty, value)] {
            if let Val::Dt(ast, _) = &term {
                self.observe(st, ast, Some(&ty), false)?;
            }
            if let Some(wf) = self.wf_of_val(&ty, &term)? {
                st.pc.push(wf);
            }
            st.env.set(name, term);
        }
        Ok(())
    }

    /// verify.py `run_foreign`: domain-only check of the ensures of a trusted rust fn: fresh result `result!k` with its
    /// well-formedness, then each ensures is domain-checked under the requires, the result facts and the earlier ensures.
    fn run_foreign(&mut self, st: &mut State) -> R<()> {
        let f = self.f.ok_or_else(|| "no function".to_string())?;
        let mut env = st.env.clone();
        let rty = Ty::from_name(&f.ret);
        if rty != Ty::Unit {
            let res = self.try_fresh("result", &rty)?;
            match &res {
                Val::Arr(a) => {
                    st.pc.push(self.z.and(&[a.n.ge_i(0), a.n.le_i(MAX_LEN)]));
                    if a.elem == Elem::U8 {
                        let d = self.u8_dom(a);
                        st.pc.push(d);
                    } else if is_vec_i64(&rty) {
                        // G6b: a foreign Vec<i64> result has i64 elements
                        let d = self.i64_dom(a);
                        st.pc.push(d);
                    }
                }
                Val::File(f) => st.pc.push(self.wf_file(f)),
                Val::Proc(p) => st.pc.push(self.wf_proc(p)),
                Val::Sock(p) => st.pc.push(self.wf_sock(p)),
                Val::Loop(p) => st.pc.push(self.wf_loop(p)),
                Val::Lis(l) => st.pc.push(self.wf_lis(l)),
                Val::PArgs(a) => st.pc.push(self.wf_pargs(a)),
                Val::Map(m) => st.pc.push(self.z.and(&[m.n.ge_i(0), m.n.le_i(MAX_LEN)])),
                other => {
                    if let Some(w) = self.try_wf(&rty, other.try_ast()?)? {
                        st.pc.push(w);
                    }
                }
            }
            env.set("result", res);
        }
        let mut prev: Vec<Ast> = Vec::new();
        for en in &f.ensures {
            let what = format!(
                "variant_field lookups in postcondition (line {}) are defined",
                pruv_core::diagnostic::plain_line(en.line())
            );
            let ds = self.check_dom_e(st, &prev, en, pos(en), &what, Some(&env))?;
            prev.extend(ds);
            let v = self.ev_ast(en, st, &[], false, &env)?;
            prev.push(v);
        }
        Ok(())
    }

    /// verify.py `run` (line 835).
    pub fn run(&mut self) -> R<()> {
        let f = self.f.ok_or_else(|| "no function".to_string())?;
        let mut env = Env::default();
        let mut pc: Vec<Ast> = Vec::new();
        // P13o: the facts every value of the parameter types meets in a contract too
        let mut shape: Vec<Ast> = Vec::new();
        for p in &f.params {
            let pty = Ty::from_name(&p.ty);
            let v = self.try_fresh(&p.name, &pty)?;
            match &v {
                Val::Arr(a) => {
                    pc.push(self.z.and(&[a.n.ge_i(0), a.n.le_i(MAX_LEN)]));
                    shape.push(pc[pc.len() - 1].clone());
                    if a.elem == Elem::U8 {
                        pc.push(self.u8_dom(a));
                        shape.push(pc[pc.len() - 1].clone());
                    }
                }
                Val::File(f) => pc.push(self.wf_file(f)),
                Val::Proc(p) => pc.push(self.wf_proc(p)),
                Val::Sock(p) => pc.push(self.wf_sock(p)),
                Val::Loop(p) => pc.push(self.wf_loop(p)),
                Val::Lis(l) => pc.push(self.wf_lis(l)),
                Val::PArgs(a) => pc.push(self.wf_pargs(a)),
                Val::Map(m) => {
                    // G6 A1
                    pc.push(self.z.and(&[m.n.ge_i(0), m.n.le_i(MAX_LEN)]));
                    shape.push(pc[pc.len() - 1].clone());
                }
                other => {
                    if let Some(w) = self.try_wf(&pty, other.try_ast()?)? {
                        pc.push(w);
                    }
                }
            }
            env.set(&p.name, v);
        }
        self.params = env.clone();
        // G5 A1 (5.1): the immutable entry snapshots of the inout parameters; the executable name starts as the same value
        // and is rebound by element stores, calls and loop havoc, never the snapshot
        self.snap = Env(f
            .params
            .iter()
            .filter(|p| p.mode_pos.is_some() && (GType::from_key(&p.ty).is_array() || self.invs.contains_key(&p.ty)))
            .map(|p| (p.name.clone(), env.get(&p.name).clone()))
            .collect());
        let mut st = State::new(env.clone(), pc);
        if let (true, Some(xa)) = (f.opaque, &f.x_auto) {
            // P13o (P13o.1 totality): the body is defined for every argument, proven once here, so a reveal equation
            // never stands on an undefined body. Contract arithmetic is unbounded, so a contract may apply F to a
            // scalar outside i64 (or to an array element, which has no i64 fact): the proof assumes only the array and
            // map shape facts, never the i64 range of a scalar parameter (audit P13o.9)
            let mut st0 = State::new(env, shape);
            let envo = st0.env.clone();
            let what = format!("the body of opaque {} is defined for every argument", f.name);
            for (d, _) in self.dom_obligations(xa, &mut st0, &[], &envo)? {
                self.prove_core(&st0, &[], &d, mk_pos(f.line, f.col), "opaque_total", &what, None);
            }
        }
        // LG16: the synthesized struct-invariant requires come first and are handled like the user's
        let invs: Vec<Expr> = self.inv_clauses(f, false).into_iter().map(|ic| ic.expr).collect();
        for r in invs.iter().chain(f.requires.iter()) {
            // D(r) is proved from the preceding requires only, then D(r) && r is assumed in declaration order
            let what = format!(
                "variant_field lookups in requires (line {}) are defined",
                pruv_core::diagnostic::plain_line(r.line())
            );
            self.check_dom_e(&mut st, &[], r, pos(r), &what, None)?;
            // Only the variant_field part is assumed: the remaining domains were proved above but are not assumed
            let envc = st.env.clone();
            if let Some(d) = self.dom(r, &mut st, &[], &envc, PKind::Variant)? {
                st.pc.push(d);
            }
            let v = self.ev0_ast(r, &mut st, &[], false)?;
            st.pc.push(v);
        }
        if let Some(cost) = &f.cost {
            let env = st.env.clone();
            self.cost0 = Some(self.cost_value(cost, &mut st, &[], pos(cost), &env)?);
            st.env.set("spent", Val::Int(self.z.int(0)));
        }
        if let (Some(d), None) = (&f.decreases, &f.foreign) {
            // G9 A1 (G9_SPEC 5): the measure is defined and >= 0 under the requires at every entry (errors point to the
            // clause); its value is frozen here and is the caller measure of every recursive call in the body
            self.check_dom_e(&mut st, &[], d, pos(d), "variant_field lookups in the decreases clause are defined", None)?;
            let m0 = self.ev0_ast(d, &mut st, &[], false)?;
            let ob = m0.ge_i(0);
            self.m0 = Some(m0);
            let what = format!("decreases measure `{}` >= 0 at function entry", expr_text(d));
            self.prove_core(&st, &[], &ob, pos(d), "recursive_nonneg", &what, None);
        }
        if f.foreign.is_some() {
            // G6 A1 (11.4): a trusted declaration has no body and no ensures proof
            return self.run_foreign(&mut st);
        }
        let ret_unit = Ty::from_name(&f.ret) == Ty::Unit;
        let node: Pos = mk_pos(f.line, f.col);
        for mut end in self.exec_block(&f.body, st)? {
            if ret_unit {
                self.ensures_ok(&mut end, None, node)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflected_and_literal_comparisons() {
        let z = Z3::new();
        let x = z.int_const("x");
        // If a z3 numeric literal is on the right, reflect (`IntVal(0).__le__(x)`); if it is a Python int, leave as is
        assert_eq!(x.ge(&z.int(0)).sexpr(), "(<= 0 x)");
        assert_eq!(x.ge_i(0).sexpr(), "(>= x 0)");
        assert_eq!(x.ne_i(0).sexpr(), "(distinct x 0)");
    }

    #[test]
    fn int_literals_normalize() {
        assert_eq!(norm_int("+007"), "7");
        assert_eq!(norm_int("-0"), "0");
        assert_eq!(norm_int("-12"), "-12");
        assert_eq!(norm_int("9223372036854775808"), "9223372036854775808");
    }
}

/// LG4 (verify.py `_arr_shape`): an aggregate body with every array-sorted subterm replaced by `A`.
/// A quantifier or other non-application keeps its full text, so it pairs only with an identical body.
fn arr_shape(t: &Ast) -> String {
    if t.sort().kind_id() == 5 {
        return "A".to_string();
    }
    let ch = t.children();
    if t.is_app() && !ch.is_empty() {
        let parts: Vec<String> = ch.iter().map(arr_shape).collect();
        return format!("({} {})", t.decl_name().unwrap_or_default(), parts.join(" "));
    }
    t.sexpr()
}

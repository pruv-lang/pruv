//! Pruv AST (counterpart of vast.py): 15 expressions + 10 statements + 9 declarations/helpers = 34 nodes.
//! Type is a single name string. Integers are a sign plus a normalized decimal string (arbitrary length).
//! `ty` is always None at the parse stage (the check stage fills it in).

pub type Type = String;
pub type Ty = Option<String>;
/// P7-1: a fixed-length array literal `[a, b, c]` is a `Call` of this name with the elements as arguments.
pub const ARRAY_LIST: &str = "@array";
/// P9b: the unresolved method call `recv.m(args)` is the call `@method|m(recv, args)` until the checker resolves it
pub const METHOD_CALL: &str = "@method|";
/// P7-1: `[x; N]` is a `Call` named `@repeat:<N>` with the one argument `x`.
pub const ARRAY_REPEAT: &str = "@repeat:";

/// P7-1: the length of a `[x; N]` literal call name.
pub fn array_repeat_len(name: &str) -> Option<u64> {
    name.strip_prefix(ARRAY_REPEAT)?.parse().ok()
}

/// P7-1: a fixed-length array literal (`[a, b]` or `[x; N]`, both parsed as a reserved `Call`).
pub fn is_array_lit(name: &str) -> bool {
    name == ARRAY_LIST || array_repeat_len(name).is_some()
}

// ---------- G3: generics ----------
// Names are always plain. The G3 extras are real optional fields (Python keeps them as class attributes, vast.py):
//  * `tparams` on Function / StructDecl / EnumDecl (empty = not generic), filled by the parser;
//  * `targs` on Call / StructLit / EnumCtor / Ctor (None = no type application written), filled by the parser;
//  * `inst` on the same four nodes: the instantiation the checker recorded (declaration identity, type arguments, substitution);
//  * `enum_ty` on IsVariant / VariantField: the checked type of the scrutinee, filled by the checker.
// A structured type inside `ty` / `type` / `ret` strings is still a `GType` key (gtype.rs `to_key`/`try_from_key`);
// a legacy type is its plain name, as before.
use crate::gtype::GType;
use std::collections::HashMap;

/// One entry of a type-parameter list (`T` or `T: Element`); `bound` is the bound's spelling.
#[derive(Debug, Clone, PartialEq)]
pub struct TParam {
    pub line: i64,
    pub col: i64,
    pub name: String,
    pub bound: Option<String>,
}

/// The instantiation recorded by the checker on a generic Call / StructLit / EnumCtor / Ctor (Python `node.inst`).
/// `decl`: the declaration identity (`fn:f`, `type:S`, `builtin:Some`); `subst`: (owner, index, replacement) ordered by index.
#[derive(Debug, Clone, PartialEq)]
pub struct Inst {
    pub decl: String,
    pub args: Vec<GType>,
    pub subst: Vec<(String, usize, GType)>,
    /// G6 A2: set only by mono on a catalog call: the concrete, NOT lowered type arguments (`args` are then the lowered
    /// ones); None as the checker leaves it.
    pub src_args: Option<Vec<GType>>,
}

// ---------- expressions ----------
#[derive(Debug, Clone, PartialEq)]
pub struct Spent { pub line: i64, pub col: i64, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct IntLit { pub line: i64, pub col: i64, pub value: String, pub ty: Ty, /** P7: how the literal was written */ pub kind: LitKind }
/// P7: the source spelling of an integer-valued literal. The value is always the decimal `value`.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum LitKind {
    #[default]
    Int,
    /// `'a'`: type `char`, value = the code point.
    Char,
    /// `b'a'`: type `u8`.
    Byte,
    /// A use of the module constant of this name, replaced by its evaluated value (SPEC §6t).
    Const(String),
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoolLit { pub line: i64, pub col: i64, pub value: bool, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct StrLit { pub line: i64, pub col: i64, pub value: Vec<u8>, pub ty: Ty, /** R-6g9-3: `[]`, the static empty view */ pub empty: bool }
#[derive(Debug, Clone, PartialEq)]
pub struct Var { pub line: i64, pub col: i64, pub name: String, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct Unary { pub line: i64, pub col: i64, pub op: String, pub operand: Box<Expr>, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct Binary { pub line: i64, pub col: i64, pub op: String, pub left: Box<Expr>, pub right: Box<Expr>, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub line: i64, pub col: i64, pub name: String, pub args: Vec<Expr>, pub ty: Ty,
    pub targs: Option<Vec<GType>>, pub inst: Option<Inst>,
    /// G5 A0: set by the checker on a resolved builtin `old(a)`: (parameter name, parameter position).
    pub old: Option<(String, usize)>,
    /// P13o: set by the checker on a contract application of an opaque function (Python `Call.opaque_c`); the code
    /// generators see its expansion (check::opaque_inline). Not emitted in the dump.
    pub opaque_c: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Index { pub line: i64, pub col: i64, pub array: Box<Expr>, pub index: Box<Expr>, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct Ctor {
    pub line: i64, pub col: i64, pub name: String, pub arg: Option<Box<Expr>>, pub ty: Ty,
    pub targs: Option<Vec<GType>>, pub inst: Option<Inst>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Quant {
    pub line: i64, pub col: i64, pub kind: String, pub var: String,
    pub lo: Box<Expr>, pub hi: Box<Expr>, pub body: Box<Expr>, pub ty: Ty,
    /// R-6-3: user-named trigger terms (one multi-pattern); empty when absent.
    pub triggers: Vec<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct StructLit {
    pub line: i64, pub col: i64, pub name: String, pub fields: Vec<FieldInit>, pub ty: Ty,
    pub targs: Option<Vec<GType>>, pub inst: Option<Inst>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct EnumCtor {
    pub line: i64, pub col: i64, pub enum_name: String, pub variant: String, pub args: Vec<Expr>, pub ty: Ty,
    pub targs: Option<Vec<GType>>, pub inst: Option<Inst>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FieldAccess { pub line: i64, pub col: i64, pub base: Box<Expr>, pub field: String, pub ty: Ty }
#[derive(Debug, Clone, PartialEq)]
pub struct IsVariant {
    pub line: i64, pub col: i64, pub value: Box<Expr>, pub enum_name: String, pub variant: String, pub ty: Ty,
    pub enum_ty: Option<GType>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct VariantField {
    pub line: i64, pub col: i64, pub value: Box<Expr>, pub enum_name: String,
    pub variant: String, pub field: String, pub ty: Ty, pub enum_ty: Option<GType>,
}

/// G6 A0: `move name` (consumes an owned collection binding). Counterpart of vast.Move.
#[derive(Debug, Clone, PartialEq)]
pub struct Move { pub line: i64, pub col: i64, pub name: String, pub fields: Vec<String>, pub ty: Ty }

/// G5 A0: the call-argument marker `inout <operand>`; line/col = the `inout` token. The operand is kept as written (the
/// checker requires a plain Var naming a mutable local array or an inout parameter of the caller). Counterpart of vast.InoutArg.
#[derive(Debug, Clone, PartialEq)]
/// `post_refine`: G5 A1, set by refine.rs (Python `a.post_refine`): the synthesized assertions of the refined local this
/// argument names, proved right after the call transition. Not emitted in the dump; empty everywhere else (also in mono copies).
pub struct InoutArg { pub line: i64, pub col: i64, pub expr: Box<Expr>, pub ty: Ty, pub post_refine: Vec<Assert> }

/// Conditional place syntax, encoded in the expression enum's call payload.
/// The reserved name cannot be written in source, and the front end handles this
/// payload as a place node without resolving it as a call.
pub struct PlaceIf {
    pub line: i64,
    pub col: i64,
    pub cond: Expr,
    pub then: Expr,
    pub els: Expr,
    pub ty: Ty,
}

impl PlaceIf {
    const NAME: &str = "@place_if";

    pub fn into_expr(self) -> Expr {
        Expr::Call(Call {
            line: self.line, col: self.col, name: Self::NAME.into(),
            args: vec![self.cond, self.then, self.els], ty: self.ty,
            targs: None, inst: None, old: None, opaque_c: false,
        })
    }

    pub fn matches(call: &Call) -> bool {
        call.name == Self::NAME
    }
}

/// P9a-3: the value `if c { e1 } else { e2 }`, encoded like `PlaceIf` in the call payload with the arguments
/// `[cond, then, els]` (an `else if` chain nests in `els`). Only the chosen arm is evaluated; the reserved name cannot
/// be written in source and is never resolved as a function.
pub struct ValueIf;

impl ValueIf {
    pub const NAME: &str = "@value_if";

    pub fn expr(line: i64, col: i64, cond: Expr, then: Expr, els: Expr) -> Expr {
        Expr::Call(Call {
            line, col, name: Self::NAME.into(), args: vec![cond, then, els], ty: None,
            targs: None, inst: None, old: None, opaque_c: false,
        })
    }

    pub fn matches(call: &Call) -> bool {
        call.name == Self::NAME
    }
}

/// P9a-5: the value `match v { P1 => e1, .. }`, encoded like `ValueIf` as a reserved call with the arguments
/// `[v, e1, .., en]`. The arm patterns (variant or `Some`/`None`/`Ok`/`Err` with plain binders) are spelled in the
/// call name after `NAME`, one `|enum;ctor;b1,b2` segment per arm, so the generic `Expr` walkers see every value
/// and need no new node. The reserved name cannot be written in source and is never resolved as a function.
pub struct ValueMatch;

/// P9a-7: the reserved call of a match guard: the statement `@arm_guard(g);` in front of an arm body, or the arm value
/// `@arm_guard(g, e)` of a value match. The checker lowers every guarded match (`Checker::pattern_match`), so later
/// passes never see one.
pub const ARM_GUARD: &str = "@arm_guard";

/// P9a-6: a pattern below the top constructor of a match arm. It is spelled in a binder slot of
/// `MatchArm::binders` / `ValueArm::binders` so every pass before the checker carries it as is: a name, `_`, an
/// integer literal `#-3`, a `bool` literal `#true`, or a nested variant `{enum;ctor;p1,p2}` (an empty enum for
/// `Some`/`None`/`Ok`/`Err`). The checker lowers a match holding such a slot, or a top-level `_` arm (ctor `_`), to
/// nested plain matches and `if`s (`check::Checker::lower_patterns`), so later passes never see one.
#[derive(Debug, Clone, PartialEq)]
pub enum SubPat {
    Bind(String),
    Wild,
    Int(String),
    Bool(bool),
    Variant(Option<String>, String, Vec<SubPat>),
    /// P9a-7: `name @ p`: binds the matched value to `name` and tests it against `p` (encoded `{@name;p}`).
    At(String, Box<SubPat>),
}

impl SubPat {
    /// Whether a binder slot is a plain name (every slot before P9a-6).
    pub fn is_plain(slot: &str) -> bool {
        !(slot == "_" || slot.starts_with('#') || slot.starts_with('{'))
    }

    pub fn decode(slot: &str) -> SubPat {
        if slot == "_" {
            return SubPat::Wild;
        }
        if let Some(l) = slot.strip_prefix('#') {
            return match l {
                "true" => SubPat::Bool(true),
                "false" => SubPat::Bool(false),
                _ => SubPat::Int(l.to_string()),
            };
        }
        if let Some(inner) = slot.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            if let Some(at) = inner.strip_prefix('@') {
                let (name, sub) = at.split_once(';').unwrap_or((at, "_"));
                return SubPat::At(name.to_string(), Box::new(Self::decode(sub)));
            }
            let mut it = inner.splitn(3, ';');
            let en = it.next().unwrap_or("");
            let ctor = it.next().unwrap_or("").to_string();
            let subs = Self::split(it.next().unwrap_or("")).iter().map(|s| Self::decode(s)).collect();
            return SubPat::Variant((!en.is_empty()).then(|| en.to_string()), ctor, subs);
        }
        SubPat::Bind(slot.to_string())
    }

    pub fn encode(&self) -> String {
        match self {
            SubPat::Bind(n) => n.clone(),
            SubPat::Wild => "_".into(),
            SubPat::Int(v) => format!("#{v}"),
            SubPat::Bool(b) => format!("#{b}"),
            SubPat::Variant(en, ctor, subs) => format!("{{{};{};{}}}", en.as_deref().unwrap_or(""), ctor,
                subs.iter().map(SubPat::encode).collect::<Vec<_>>().join(",")),
            SubPat::At(n, p) => format!("{{@{};{}}}", n, p.encode()),
        }
    }

    /// The binder slots of a list written `p1,p2` (commas inside a nested `{..}` do not split).
    pub fn split(list: &str) -> Vec<String> {
        let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
        for ch in list.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(ch);
        }
        if !cur.is_empty() || !out.is_empty() {
            out.push(cur);
        }
        out
    }

    /// Rewrites every nested variant's enum name with `f` (module resolution of `m::E::V` sub-patterns).
    pub fn map_enums<E>(&mut self, f: &mut dyn FnMut(&str) -> Result<String, E>) -> Result<(), E> {
        match self {
            SubPat::Variant(en, _, subs) => {
                if let Some(n) = en {
                    *n = f(n)?;
                }
                for s in subs {
                    s.map_enums(f)?;
                }
            }
            SubPat::At(_, p) => p.map_enums(f)?,
            _ => {}
        }
        Ok(())
    }

    /// `map_enums` over the encoded binder slots of one arm.
    pub fn map_slots<E>(slots: &mut [String], f: &mut dyn FnMut(&str) -> Result<String, E>) -> Result<(), E> {
        for s in slots.iter_mut().filter(|s| s.starts_with('{')) {
            let mut p = Self::decode(s);
            p.map_enums(f)?;
            *s = p.encode();
        }
        Ok(())
    }
}

/// P9a-5: an arm pattern of a value `match` (see `ValueMatch`).
#[derive(Debug, Clone, PartialEq)]
pub struct ValueArm {
    pub enum_name: Option<String>, pub ctor: String, pub binders: Vec<String>,
    /// P9a-6c: per binder, the tuple pattern it was written as (as `MatchArm::pats`; empty = none). A top-level
    /// tuple arm `(a, b) =>` has ctor `()` and one binder; a slice arm has the slice shape as ctor (`[]`, `[..]`,
    /// `[..rest]`) and the prefix binders.
    pub pats: Vec<Option<TuplePat>>,
}

impl ValueMatch {
    pub const NAME: &str = "@value_match";
    /// The marker statement `@arm_value(e)` an arm body holds when a value match is checked or lowered through the
    /// statement `match` machinery.
    pub const YIELD: &str = "@arm_value";

    pub fn expr(line: i64, col: i64, scrutinee: Expr, arms: Vec<(ValueArm, Expr)>) -> Expr {
        let mut name = Self::NAME.to_string();
        let mut args = vec![scrutinee];
        for (a, e) in arms {
            name.push('|');
            name.push_str(&Self::encode(&a));
            args.push(e);
        }
        Expr::Call(Call { line, col, name, args, ty: None, targs: None, inst: None, old: None, opaque_c: false })
    }

    pub fn encode(a: &ValueArm) -> String {
        let mut s = format!("{};{};{}", a.enum_name.as_deref().unwrap_or(""), a.ctor, a.binders.join(","));
        if a.pats.iter().any(Option::is_some) {
            // after `~` (no binder or variant slot holds one): one tuple pattern or nothing per binder, `&`-separated
            s.push('~');
            s.push_str(&a.pats.iter().map(|p| p.as_ref().map(TuplePat::encode).unwrap_or_default()).collect::<Vec<_>>().join("&"));
        }
        s
    }

    pub fn matches(call: &Call) -> bool {
        // an empty value match (`match e {}`) carries no `|`; it is still a value match (rejected as type/match_arms)
        call.name == Self::NAME || call.name.starts_with("@value_match|")
    }

    /// The call name for these arms (after the arm enum names were resolved).
    pub fn rename(arms: &[ValueArm]) -> String {
        let mut name = Self::NAME.to_string();
        for a in arms {
            name.push('|');
            name.push_str(&Self::encode(a));
        }
        name
    }

    pub fn arms(call: &Call) -> Vec<ValueArm> {
        call.name.split('|').skip(1).map(|seg| {
            let (seg, tups) = seg.split_once('~').map_or((seg, None), |(a, b)| (a, Some(b)));
            let pats = tups.map(|t| t.split('&').map(|p| (!p.is_empty()).then(|| TuplePat::decode(p, call.line, call.col)))
                .collect()).unwrap_or_default();
            let mut it = seg.splitn(3, ';');
            let en = it.next().unwrap_or("");
            let ctor = it.next().unwrap_or("").to_string();
            let bs = it.next().unwrap_or("");
            ValueArm {
                enum_name: (!en.is_empty()).then(|| en.to_string()),
                ctor,
                binders: SubPat::split(bs),
                pats,
            }
        }).collect()
    }

    /// The statement `match` with the same scrutinee and patterns whose arm `k` body is `@arm_value(ek)`.
    pub fn as_match(call: &Call) -> Match {
        let arms = Self::arms(call).into_iter().zip(call.args[1..].iter()).map(|(a, e)| MatchArm {
            line: e.line(), col: e.col(), enum_name: a.enum_name, ctor: a.ctor, binders: a.binders,
            body: vec![Stmt::ExprStmt(ExprStmt { line: e.line(), col: e.col(), expr: Expr::Call(Call {
                line: e.line(), col: e.col(), name: Self::YIELD.into(), args: vec![e.clone()], ty: None, targs: None,
                inst: None, old: None, opaque_c: false,
            }) })],
            pats: Vec::new(), pat_lets: 0,
        }).collect();
        Match { line: call.line, col: call.col, scrutinee: call.args[0].clone(), arms, form: MatchForm::Match, els: Vec::new() }
    }

    pub fn is_yield(call: &Call) -> bool {
        call.name == Self::YIELD
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Spent(Spent),
    InoutArg(InoutArg),
    Move(Move),
    IntLit(IntLit),
    BoolLit(BoolLit),
    StrLit(StrLit),
    Var(Var),
    Unary(Unary),
    Binary(Binary),
    Call(Call),
    Index(Index),
    Ctor(Ctor),
    Quant(Quant),
    StructLit(StructLit),
    EnumCtor(EnumCtor),
    FieldAccess(FieldAccess),
    IsVariant(IsVariant),
    VariantField(VariantField),
}

impl Expr {
    pub fn line(&self) -> i64 {
        match self {
            Expr::InoutArg(x) => x.line,
            Expr::Move(x) => x.line,
            Expr::Spent(x) => x.line,
            Expr::IntLit(x) => x.line,
            Expr::BoolLit(x) => x.line,
            Expr::StrLit(x) => x.line,
            Expr::Var(x) => x.line,
            Expr::Unary(x) => x.line,
            Expr::Binary(x) => x.line,
            Expr::Call(x) => x.line,
            Expr::Index(x) => x.line,
            Expr::Ctor(x) => x.line,
            Expr::Quant(x) => x.line,
            Expr::StructLit(x) => x.line,
            Expr::EnumCtor(x) => x.line,
            Expr::FieldAccess(x) => x.line,
            Expr::IsVariant(x) => x.line,
            Expr::VariantField(x) => x.line,
        }
    }
    pub fn col(&self) -> i64 {
        match self {
            Expr::InoutArg(x) => x.col,
            Expr::Move(x) => x.col,
            Expr::Spent(x) => x.col,
            Expr::IntLit(x) => x.col,
            Expr::BoolLit(x) => x.col,
            Expr::StrLit(x) => x.col,
            Expr::Var(x) => x.col,
            Expr::Unary(x) => x.col,
            Expr::Binary(x) => x.col,
            Expr::Call(x) => x.col,
            Expr::Index(x) => x.col,
            Expr::Ctor(x) => x.col,
            Expr::Quant(x) => x.col,
            Expr::StructLit(x) => x.col,
            Expr::EnumCtor(x) => x.col,
            Expr::FieldAccess(x) => x.col,
            Expr::IsVariant(x) => x.col,
            Expr::VariantField(x) => x.col,
        }
    }
}

// ---------- declarations/helpers ----------
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl { pub borrowed: bool, pub line: i64, pub col: i64, pub name: String, pub ty: Type }
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDecl { pub line: i64, pub col: i64, pub name: String, pub fields: Vec<FieldDecl> }
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl { pub line: i64, pub col: i64, pub name: String, pub fields: Vec<FieldDecl>, pub tparams: Vec<TParam>,
    /// LG16: `invariant <expr>` clauses (field names bare). Not emitted in the dump.
    pub invariants: Vec<Expr>,
    pub invariant_lines: Vec<i64> }
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl { pub line: i64, pub col: i64, pub name: String, pub variants: Vec<VariantDecl>, pub tparams: Vec<TParam> }
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInit { pub line: i64, pub col: i64, pub name: String, pub value: Expr }
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub line: i64, pub col: i64, pub enum_name: Option<String>, pub ctor: String,
    pub binders: Vec<String>, pub body: Vec<Stmt>,
    /// P8: per binder, the tuple pattern it was written as (`Some((a, b))`); the binder is then the temporary
    /// `pruv_tup_d<N>` the checker destructures into the first `pat_lets` statements of `body`. Empty = no tuple.
    pub pats: Vec<Option<TuplePat>>,
    /// P8: the number of leading `body` lets the checker lowered from `pats` (0 until checked).
    pub pat_lets: usize,
}

/// P6/P8: a destructuring pattern: `_`, `[mut] name` or `(p0, p1, ..)`.
#[derive(Debug, Clone, PartialEq)]
pub enum TuplePat {
    Wild,
    Bind(i64, i64, String, bool),
    Tuple(i64, i64, Vec<TuplePat>),
}

impl TuplePat {
    /// P9a-6c: the text form a value `match` arm carries in its call name: `_`, `name`, `!name` (`mut name`) or
    /// `(p0,p1,..)`; positions are not kept (`decode` takes the arm's).
    pub fn encode(&self) -> String {
        match self {
            TuplePat::Wild => "_".into(),
            TuplePat::Bind(_, _, n, m) => format!("{}{}", if *m { "!" } else { "" }, n),
            TuplePat::Tuple(_, _, ps) => format!("({})", ps.iter().map(TuplePat::encode).collect::<Vec<_>>().join(",")),
        }
    }

    pub fn decode(s: &str, line: i64, col: i64) -> TuplePat {
        fn go(b: &[u8], i: &mut usize, line: i64, col: i64) -> TuplePat {
            if b[*i] == b'(' {
                *i += 1;
                let mut ps = Vec::new();
                while *i < b.len() && b[*i] != b')' {
                    ps.push(go(b, i, line, col));
                    if *i < b.len() && b[*i] == b',' {
                        *i += 1;
                    }
                }
                *i += 1;
                return TuplePat::Tuple(line, col, ps);
            }
            let start = *i;
            while *i < b.len() && !matches!(b[*i], b',' | b')') {
                *i += 1;
            }
            let t = std::str::from_utf8(&b[start..*i]).unwrap_or("_");
            match t {
                "_" => TuplePat::Wild,
                _ => match t.strip_prefix('!') {
                    Some(n) => TuplePat::Bind(line, col, n.to_string(), true),
                    None => TuplePat::Bind(line, col, t.to_string(), false),
                },
            }
        }
        go(s.as_bytes(), &mut 0, line, col)
    }
}

/// P8: the source form of a `Match`. `IfLet` and `LetElse` are lowered to a full match by the checker: arm 0 is the
/// written pattern (for `LetElse` its body is the rest of the enclosing block), every other arm is a complement
/// variant whose body is the `else` block. The Rust backend prints the source form back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchForm {
    #[default]
    Match,
    IfLet,
    LetElse,
    /// P8b: the head of `while let P = e { B }`, which the parser writes as `while true { match e { P => B } }` with
    /// this form and `els = [break;]`; the checker adds the complement arms (`While::let_head`).
    WhileLet,
}
/// STAGE1 A refinement annotation (vast.Refinement). Position = the first token of the annotated type. Type stays the base type.
/// shape "scalar" | "array". lo/hi/maxlen are a sign plus a normalized decimal string (arbitrary length; range checks are in refine.rs).
#[derive(Debug, Clone, PartialEq)]
pub struct Refinement {
    pub line: i64, pub col: i64, pub shape: String, pub base: String,
    pub lo: Option<String>, pub hi: Option<String>, pub maxlen: Option<String>,
}
/// No line/col. `take`: G6 A0 `take name: Type` (consuming collection parameter).
/// `mode_pos`: G5 A0 `inout name: [..]` -> Some((line, col)) of the `inout` token (Python `Param.mode == "inout"` + `mode_pos`).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String, pub ty: Type, pub refine: Option<Refinement>, pub take: bool,
    pub mode_pos: Option<(i64, i64)>,
}

/// G6 A0: the declaration data of a `trusted rust fn` (Python `Function.foreign`): the Rust path and the
/// line/col of the path string token. The body of such a function is empty.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignInfo { pub path: String, pub line: i64, pub col: i64 }

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDecl {
    Struct(StructDecl),
    Enum(EnumDecl),
}

// ---------- statements ----------
#[derive(Debug, Clone, PartialEq)]
pub struct Let {
    pub borrowed: bool,
    pub line: i64, pub col: i64, pub name: String, pub mutable: bool,
    /// Empty only for an unannotated place binding before type resolution.
    pub ty: Type,
    pub value: Expr,
    pub refine: Option<Refinement>,
}
impl Let {
    pub fn is_place(&self) -> bool {
        matches!(&self.value, Expr::InoutArg(_))
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Assign { pub line: i64, pub col: i64, pub name: String, pub value: Expr, pub deref: bool }
#[derive(Debug, Clone, PartialEq)]
/// elem_refine: (lo, hi, origin_what) — set by refine.rs only when storing into a refined array that has an element range.
pub struct IndexAssign {
    pub line: i64, pub col: i64, pub name: String, pub index: Expr, pub value: Expr,
    pub elem_refine: Option<(String, String, String)>,
    /// LG4: `s.a.b[i] = v` -> name = "s", fields = ["a", "b"]; empty for the plain `x[i] = v`.
    pub fields: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct If { pub line: i64, pub col: i64, pub cond: Expr, pub then: Vec<Stmt>, pub els: Vec<Stmt> }
#[derive(Debug, Clone, PartialEq)]
pub struct While {
    pub line: i64, pub col: i64, pub cond: Expr, pub invariants: Vec<Expr>, pub decreases: Expr, pub body: Vec<Stmt>,
    /// P4: set when the parser lowered a `for` loop into this `while` (see parser `for_loop`).
    pub for_head: Option<ForHead>,
    /// P5: the Rust-style loop label `'name` (without the quote), on `'name: while` or `'name: for`.
    pub label: Option<String>,
}
impl While {
    /// P8b: the head match of a `while let` loop (see `MatchForm::WhileLet`).
    pub fn let_head(&self) -> Option<&Match> {
        match (&self.cond, self.body.first()) {
            (Expr::BoolLit(b), Some(Stmt::Match(m))) if b.value && m.form == MatchForm::WhileLet => Some(m),
            _ => None,
        }
    }

    /// P8b: the cost-mode form of a `while let` loop. A failed head costs no iteration (R-6u-5), so the loop
    /// line does not charge; arm 0 starts (after its pattern lets) with `while true { break; }`, one iteration of an empty loop, which
    /// charges the unit in both backends. Callers drop their own per-iteration charge for this loop.
    pub fn cost_let_loop(&self) -> Option<While> {
        self.let_head()?;
        let mut w = self.clone();
        let Some(Stmt::Match(m)) = w.body.first_mut() else { return None };
        let (line, col) = (m.arms[0].line, m.arms[0].col);
        let tick = Stmt::While(While {
            line, col,
            cond: Expr::BoolLit(BoolLit { line, col, value: true, ty: Some("bool".into()) }),
            invariants: Vec::new(),
            decreases: Expr::IntLit(IntLit { line, col, value: "1".into(), ty: Some("i64".into()), kind: LitKind::default() }),
            body: vec![Stmt::Break(Jump { line, col, label: None })],
            for_head: None,
            label: None,
        });
        // after the lowered tuple-pattern lets, which the Rust backend folds back into the pattern
        let at = m.arms[0].pat_lets;
        m.arms[0].body.insert(at, tick);
        Some(w)
    }
}
/// P5: `break ['label];` or `continue ['label];`. `label` is the name without the quote.
#[derive(Debug, Clone, PartialEq)]
pub struct Jump { pub line: i64, pub col: i64, pub label: Option<String> }
/// P4: a `for` loop lowered by the parser to `if true { let ...; while ... }`. The wrapper `If` holds the hidden
/// bound lets; the last body statement is the synthesized step `idx = idx + 1` at the `for` position.
#[derive(Debug, Clone, PartialEq)]
pub struct ForHead {
    /// The user binder: the range variable, or the slice element binder.
    pub var: String,
    /// The counter the step advances: `var` for a range, a hidden index for a slice.
    pub counter: String,
    /// `for x in v` over a slice or Vec (the first body statement binds `x`); false for `for i in a..b`.
    pub slice: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ForMap {
    pub line: i64, pub col: i64, pub key: String, pub val: String, pub place: Expr,
    pub invariants: Vec<Expr>, pub body: Vec<Stmt>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Return { pub line: i64, pub col: i64, pub value: Option<Expr> }
#[derive(Debug, Clone, PartialEq)]
/// origin_code/origin_what: present only on asserts synthesized by refine.rs (verification error code and a description of the origin).
pub struct Assert {
    pub line: i64, pub col: i64, pub cond: Expr,
    pub origin_code: Option<String>, pub origin_what: Option<String>,
    /// P13o: `reveal F;` (Python `Assert.reveal`): the opaque function name, with its position; cond is `true`.
    pub reveal: Option<(String, i64, i64)>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt { pub line: i64, pub col: i64, pub expr: Expr }
#[derive(Debug, Clone, PartialEq)]
pub struct FieldAssign {
    pub line: i64,
    pub col: i64,
    pub name: String,
    pub fields: Vec<String>,
    pub value: Expr,
    // R-6g9-6: the field stores a view or a borrowing struct.
    pub ref_store: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Match { pub line: i64, pub col: i64, pub scrutinee: Expr, pub arms: Vec<MatchArm>,
    /// P8: source form; for `IfLet`/`LetElse` before checking, `arms` holds only the written pattern and `els` the
    /// else block, which the checker copies into the complement arms (then `els` is emptied).
    pub form: MatchForm, pub els: Vec<Stmt> }

#[derive(Debug, Clone, PartialEq)]
pub struct Par { pub line: i64, pub col: i64, pub calls: Vec<Expr> }

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Par(Par),
    Let(Let),
    Assign(Assign),
    IndexAssign(IndexAssign),
    If(If),
    While(While),
    ForMap(ForMap),
    Return(Return),
    Assert(Assert),
    ExprStmt(ExprStmt),
    FieldAssign(FieldAssign),
    Match(Match),
    /// P5: leaves the target loop.
    Break(Jump),
    /// P5: goes to the next iteration of the target loop (a `for` loop runs its step first).
    Continue(Jump),
}

impl Stmt {
    pub(crate) fn position(&self) -> (i64, i64) {
        match self {
            Self::Par(s) => (s.line, s.col),
            Self::Let(s) => (s.line, s.col),
            Self::Assign(s) => (s.line, s.col),
            Self::IndexAssign(s) => (s.line, s.col),
            Self::If(s) => (s.line, s.col),
            Self::While(s) => (s.line, s.col),
            Self::ForMap(s) => (s.line, s.col),
            Self::Return(s) => (s.line, s.col),
            Self::Assert(s) => (s.line, s.col),
            Self::ExprStmt(s) => (s.line, s.col),
            Self::FieldAssign(s) => (s.line, s.col),
            Self::Match(s) => (s.line, s.col),
            Self::Break(s) | Self::Continue(s) => (s.line, s.col),
        }
    }
}

// ---------- top level ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnAttribute {
    Cold,
    Inline,
    InlineAlways,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Parameter names and dotted paths to owned buffer fields.
    pub borrow_from: Option<Vec<String>>,
    pub borrowed_result: bool,
    pub line: i64, pub col: i64, pub name: String, pub params: Vec<Param>, pub ret: Type,
    pub effects: Vec<String>, pub requires: Vec<Expr>, pub ensures: Vec<Expr>, pub body: Vec<Stmt>,
    pub ret_refine: Option<Refinement>, pub tparams: Vec<TParam>,
    /// G6 A0: Some for a `trusted rust fn`.
    pub foreign: Option<ForeignInfo>,
    /// G9 A0: the `decreases Expr` clause of the function header (None = absent).
    pub decreases: Option<Expr>,
    pub cost: Option<Expr>,
    /// KD-M6-3: K of the source `ensures len(result) <= K`, kept across specialization (Python `Function.small_bound`).
    /// Not emitted in the dump.
    pub small_bound: Option<i64>,
    /// P07 (docs/L3_RESTRUCTURE_SPEC.md appendix P07): the contract expansion template of an expandable function, set
    /// by the checker (Python `Function.x_auto`); a code call assumes `result == template`. Not emitted in the dump.
    pub x_auto: Option<Expr>,
    /// P13o: `opaque fn` (Python `Function.opaque`). Not emitted in the dump.
    pub opaque: bool,
    /// R-6-2: a named predicate, expanded in contracts and omitted after checking.
    pub pred: bool,
    /// Source attribute, kept by specialization and omitted from the AST dump.
    pub attribute: Option<FnAttribute>,
}

impl Function {
    pub(crate) fn has_places(&self) -> bool {
        fn block(body: &[Stmt]) -> bool {
            body.iter().any(|st| match st {
                Stmt::Let(s) => s.is_place(),
                Stmt::If(s) => block(&s.then) || block(&s.els),
                Stmt::While(s) => block(&s.body),
                Stmt::ForMap(s) => block(&s.body),
                Stmt::Match(s) => s.arms.iter().any(|arm| block(&arm.body)),
                _ => false,
            })
        }
        block(&self.body)
    }

    /// Field paths lent by this parameter, relative to the parameter itself.
    pub(crate) fn lent_fields<'a>(&'a self, param: &'a str) -> impl Iterator<Item = &'a str> {
        self.borrow_from.iter().flatten().filter_map(move |entry| {
            let (root, path) = entry.split_once('.')?;
            (root == param).then_some(path)
        })
    }

    /// An inout buffer lent by a borrowed result serves as resizable scratch storage.
    pub fn borrows_inout_buffer(&self, source: &Expr) -> bool {
        let name = match source {
            Expr::Var(v) => &v.name,
            Expr::Move(m) if m.fields.is_empty() => &m.name,
            _ => return false,
        };
        self.borrow_from.as_ref().is_some_and(|from| from.contains(name))
            && self.params.iter().any(|p| p.name == *name && p.mode_pos.is_some()
                && crate::typeinfo::is_array_name(&p.ty))
    }

    /// Only `buf = resize(buf, ..)` on such a buffer may shrink it in place (Python `Call.resize_scratch`).
    pub fn resize_scratch(&self, target: &str, args: &[Expr]) -> bool {
        matches!(args.first(), Some(Expr::Var(v)) if v.name == target)
            || matches!(args.first(), Some(Expr::Move(m)) if m.name == target && m.fields.is_empty())
    }
}

/// G9 A0: checker metadata recorded on the program and the type table, only when a G9 feature is present
/// (Python `Program.g9` / `TypeTable.g9`, a dict with these four keys).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct G9Meta {
    /// recursive type SCCs (names in declaration order), SCC-number order
    pub type_sccs: Vec<Vec<String>>,
    /// "Decl.field" / "Enum::Variant.field" of every intra-SCC (boxed) field
    pub boxed: Vec<String>,
    /// names of extended declarations, in declaration order
    pub extended: Vec<String>,
    /// recursive function SCCs (names in function order)
    pub fn_sccs: Vec<Vec<String>>,
}

// ---------- G4 A0: modules and visibility (parse level) ----------
// `Program` is unchanged: the explicit-mode data (modules, `pub`, source names, host entries) lives in modules.rs `G4`,
// next to the program the module loader returns. Node positions carry the source file in their line number (diagnostic.rs).

/// `mod name;` (vast.ModDecl). line/col = the `mod` token.
#[derive(Debug, Clone, PartialEq)]
pub struct ModDecl { pub line: i64, pub col: i64, pub name: String }

/// A top-level item of one file, in source order: an index into `Program.functions` / `Program.types` (Python `Program.items`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item { Fn(usize), Ty(usize) }

/// G4 A2: the explicit-mode attributes of one concrete function (Python `pub` / `host_entry` / `disp` set by mono).
#[derive(Debug, Clone, PartialEq)]
pub struct FnMark {
    pub is_pub: bool,
    pub host_entry: bool,
    /// the source-qualified identity (panic text, manifests)
    pub disp: String,
}

/// G4 A2: explicit-mode data of a CONCRETE (specialized) program (Python `Program.explicit` plus the attributes mono puts on
/// the concrete declarations). Only mono.rs builds it; `None` on every other program.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Explicit {
    /// parallel to `Program.functions`
    pub fns: Vec<FnMark>,
    /// `pub` of every declaration, parallel to `Program.types`
    pub type_pub: Vec<bool>,
    /// Rust name (stem) of a nominal type -> the identity it was declared with (Python `src_name` of the nodes)
    pub tdisp: HashMap<String, String>,
}

/// No line/col.
#[derive(Debug, Clone, PartialEq, Default)]
/// refined: a marker that refine::expand has already run (Python `_refined`). Not emitted in the dump.
/// g9: G9 A0 checker metadata (Python `Program.g9`), None unless a G9 feature is present. Not emitted in the dump.
/// explicit: G4 A2, Some only on the concrete program of an explicit visibility program (built by mono.rs).
/// uses_inout: G5 A0, set by the checker (Python `Program.uses_inout`); None = not checked yet. Not emitted in the dump.
pub struct Program {
    pub functions: Vec<Function>,
    pub types: Vec<TypeDecl>,
    pub refined: bool,
    pub g9: Option<G9Meta>,
    pub explicit: Option<Explicit>,
    pub uses_inout: Option<bool>,
    /// Source position of a vector-to-slice call in a contract.
    pub contract_vec_slice: Option<(i64, i64)>,
    /// P06: the manifest's `raw_entries` ("crate" or "unsafe"; None = host entries stay `pub fn`), applied in P09
    pub raw_entries: Option<String>,
    /// P10H: the manifest's program package name (Python `Program.pkg_name`); prefix `<pkg>__` of every global C symbol
    pub pkg_name: Option<String>,
    /// P7: module-level `const NAME: T = expr;` items, evaluated by the parser (uses are already literals)
    pub consts: Vec<ConstItem>,
    /// P9: the used type aliases of a single-file program (uses are already expanded); only the Rust output prints them
    pub aliases: Vec<AliasItem>,
    /// P9b: the functions of `impl` blocks (lowered to free functions named `Owner__name`), in source order
    pub methods: Vec<MethodItem>,
    /// P9b: the owner type, line and column of every `impl` block (an empty block is checked like any other);
    /// P9b-3: and the number of its type parameters
    pub impls: Vec<(String, i64, i64, usize)>,
}

/// P9b: how a method takes its receiver (`self`, `inout self`, `take self`), or none (an associated function).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver { None, Read, Inout, Take }

/// P9b: one function of an `impl Owner { .. }` block; `func` is its lowered name `Owner__name`. In explicit mode
/// the loader replaces `owner` and `func` by their identities (`crate::m::Owner`, `crate::m::Owner__name`), and the
/// concrete program of mono by their Rust stems.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodItem {
    pub line: i64,
    pub col: i64,
    pub owner: String,
    pub name: String,
    pub func: String,
    pub recv: Receiver,
    /// P9b-2: `pub fn` in the impl block (visible outside its module; never a host entry, so no `--proven-entries` twin)
    pub is_pub: bool,
    /// P9b-3: the number of type parameters of the impl block (`impl<T> S<T>`: 1); they are the first type
    /// parameters of `func`
    pub impl_params: usize,
    /// P9b-3: the owner as written in the impl block (`Box2`); messages of a single-file program name the method
    /// `shown::name` whatever stem `owner` becomes
    pub shown: String,
}

/// P9: `type name<params> = rhs;` with `rhs` a type key whose parameters are plain names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AliasItem {
    pub name: String,
    pub params: Vec<String>,
    pub rhs: String,
}

/// P7: one evaluated module constant; `ty` is an int width name or `char`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConstItem {
    pub line: i64,
    pub col: i64,
    pub name: String,
    pub ty: String,
    pub value: i128,
}

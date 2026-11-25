//! Recursive-descent parser (a line-by-line port of Parser in vparser.py).

use crate::intwidth;
use crate::ast::*;
use crate::diagnostic::PruvError;
use crate::gtype::{fn_owner, decl_owner, mk_array, mk_option, mk_param, mk_result, mk_result_with_error, GType};
use crate::lexer::{lex_tagged, Kind, Tok};
use crate::pyrepr::repr_str;
use std::collections::HashMap;

type R<T> = Result<T, PruvError>;

const CMP_OPS: [&str; 6] = ["==", "!=", "<=", ">=", "<", ">"];

/// Public entry: first applies the universal newline handling of Python open().read().
pub fn parse_source(src: &str) -> R<Program> {
    Ok(parse_file(src, 0)?.prog)
}

/// One parsed source file (G4 A0): the program plus the parse-level module data (Python `Program.mods` / `has_pub` /
/// `items`, and the `pub` flag of every function / type declaration).
pub struct ParsedFile {
    pub prog: Program,
    pub mods: Vec<ModDecl>,
    pub has_pub: bool,
    pub items: Vec<Item>,
    pub fn_pub: Vec<bool>,
    pub ty_pub: Vec<bool>,
}

fn normalized_chars(src: &str) -> Vec<char> {
    let norm = src.replace("\r\n", "\n").replace('\r', "\n");
    norm.chars().collect()
}

/// Parses one file whose line numbers carry the module file number `tag` (0 = a plain, legacy-style file).
pub fn parse_file(src: &str, tag: usize) -> R<ParsedFile> {
    let chars = normalized_chars(src);
    let mut p = Parser {
        t: lex_tagged(&chars, tag)?,
        i: 0,
        depth: 0,
        scope: HashMap::new(),
        mods: Vec::new(),
        has_pub: false,
        items: Vec::new(),
        fn_pub: Vec::new(),
        ty_pub: Vec::new(),
        tuples: Vec::new(),
        tup_tmp: 0,
        pending: Vec::new(),
        consts: HashMap::new(),
        fixed_ok: false,
    };
    let consts = p.const_prepass()?;
    let mut prog = p.program()?;
    if let Some(first) = consts.first() {
        if tag != 0 || p.has_pub || !p.mods.is_empty() {
            // P7 row P7-3: constants in module programs (paths, `pub const`) are not designed yet.
            return Err(PruvError::new("parse", first.line, first.col, "const_module",
                "constants are available only in a single-file program without `pub` or `mod` items (parity row P7-3)",
                "write the value inline, or keep the program in one file"));
        }
    }
    prog.consts = consts;
    if let Some(first) = p.tuples.first() {
        if tag != 0 || p.has_pub || !p.mods.is_empty() {
            // P6 row P6-2: module identities of the synthetic tuple structs are not designed yet.
            return Err(PruvError::new("parse", first.2, first.3, "tuple_module",
                "tuple types are available only in a single-file program without `pub` or `mod` items (parity row P6-2)",
                "declare a named struct for this module program"));
        }
    }
    for (name, comps, line, col) in std::mem::take(&mut p.tuples) {
        let fields = comps.iter().enumerate().map(|(i, k)| FieldDecl {
            borrowed: false, line, col, name: i.to_string(), ty: k.to_key(),
        }).collect();
        prog.types.push(TypeDecl::Struct(StructDecl { line, col, name, fields, tparams: Vec::new(),
            invariants: Vec::new(), invariant_lines: Vec::new() }));
        p.ty_pub.push(false);
        p.items.push(Item::Ty(prog.types.len() - 1));
    }
    Ok(ParsedFile { prog, mods: p.mods, has_pub: p.has_pub, items: p.items, fn_pub: p.fn_pub, ty_pub: p.ty_pub })
}

/// vparser `_sniff_explicit`: a root that failed to parse: does it use module syntax (a `mod` / `pub` token)?
/// One lexing pass: the tokens read before a lex error count, so a `pub` ahead of a bad character still counts.
pub fn sniff_explicit(src: &str) -> bool {
    let chars = normalized_chars(src);
    let mut toks: Vec<Tok> = Vec::new();
    let _ = crate::lexer::lex_into(&chars, 0, &mut toks);
    toks.iter().any(|t| t.kind == Kind::Kw && (t.val == "mod" || t.val == "pub"))
}

/// `[a-z][A-Za-z0-9_]*` (module names)
fn mod_name_ok(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_lowercase() => it.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

fn lower_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => format!("{}{}", c.to_ascii_lowercase(), it.as_str()),
        None => String::new(),
    }
}

/// Decodes the raw text of a string token (quotes included, already validated by the lexer) into a UTF-8 byte sequence.
fn decode_str_lit(raw: &str) -> Vec<u8> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 1usize;
    let end = chars.len().saturating_sub(1);
    let mut buf = [0u8; 4];
    while i < end {
        let c = chars[i];
        if c == '\\' && i + 1 < end {
            i += 1;
            let b = match chars[i] {
                '"' => b'"',
                '\\' => b'\\',
                'n' => b'\n',
                'r' => b'\r',
                't' => b'\t',
                _ => 0u8,
            };
            out.push(b);
        } else {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        i += 1;
    }
    out
}

/// Sign negation of a decimal string for negative folding ("0" stays as is).
fn negate(v: &str) -> String {
    if v == "0" {
        "0".to_string()
    } else if let Some(rest) = v.strip_prefix('-') {
        rest.to_string()
    } else {
        format!("-{}", v)
    }
}

/// P4: in the invariants of `for x in v`, `index(x)` is the number of elements already visited (the hidden counter).
/// The binder cannot shadow an outer name, so `index(x)` cannot mean a user call on another `x`.
fn for_index_rewrite(e: &mut Expr, binder: &str, counter: &str) {
    if let Expr::Call(c) = e {
        if c.name == "index" && c.targs.is_none() && matches!(c.args.as_slice(), [Expr::Var(v)] if v.name == binder) {
            *e = Expr::Var(Var { line: c.line, col: c.col, name: counter.to_string(), ty: None });
            return;
        }
    }
    let mut go = |x: &mut Expr| for_index_rewrite(x, binder, counter);
    match e {
        Expr::Unary(u) => go(&mut u.operand),
        Expr::Binary(b) => { go(&mut b.left); go(&mut b.right); }
        Expr::Call(c) => c.args.iter_mut().for_each(go),
        Expr::Index(i) => { go(&mut i.array); go(&mut i.index); }
        Expr::Ctor(c) => if let Some(a) = c.arg.as_mut() { go(a) },
        Expr::Quant(q) => { go(&mut q.lo); go(&mut q.hi); go(&mut q.body); }
        Expr::StructLit(l) => l.fields.iter_mut().for_each(|f| go(&mut f.value)),
        Expr::EnumCtor(c) => c.args.iter_mut().for_each(go),
        Expr::FieldAccess(f) => go(&mut f.base),
        Expr::IsVariant(v) => go(&mut v.value),
        Expr::VariantField(v) => go(&mut v.value),
        Expr::InoutArg(a) => go(&mut a.expr),
        _ => {}
    }
}

/// P7: the closed evaluator of module constants.
struct ConstEval<'a> {
    decls: &'a [(Tok, &'static intwidth::IntWidth, Expr)],
    vals: HashMap<String, i128>,
    busy: Vec<String>,
}

impl<'a> ConstEval<'a> {
    fn fail(e: &Expr, code: &str, msg: String, fix: &str) -> PruvError {
        PruvError::new("type", e.line(), e.col(), code, msg, fix)
    }

    fn value_of(&mut self, k: usize) -> R<i128> {
        let (n, w, e) = &self.decls[k];
        if let Some(v) = self.vals.get(&n.val) {
            return Ok(*v);
        }
        if self.busy.contains(&n.val) {
            return Err(PruvError::new("type", n.line, n.col, "const_expr",
                format!("constant {} depends on itself", repr_str(&n.val)), "break the cycle"));
        }
        self.busy.push(n.val.clone());
        let v = self.eval(e, w)?;
        self.busy.pop();
        self.vals.insert(n.val.clone(), v);
        Ok(v)
    }

    /// The type an expression has on its own; None for an untyped integer literal.
    fn ty_of(&self, e: &Expr) -> Option<&'static intwidth::IntWidth> {
        match e {
            Expr::IntLit(l) => match l.kind {
                LitKind::Char => Some(&intwidth::CHAR),
                LitKind::Byte => Some(&intwidth::U8),
                // P7-1: a constant use inside an array length keeps its declared width
                LitKind::Const(_) => l.ty.as_deref().and_then(intwidth::of),
                _ => None,
            },
            Expr::Var(x) => self.decls.iter().find(|d| d.0.val == x.name).map(|d| d.1),
            Expr::Call(c) if c.args.len() == 1 && c.targs.is_none() => intwidth::of(&c.name),
            Expr::Unary(u) => self.ty_of(&u.operand),
            Expr::Binary(b) => self.ty_of(&b.left).or_else(|| self.ty_of(&b.right)),
            _ => None,
        }
    }

    fn eval(&mut self, e: &Expr, w: &'static intwidth::IntWidth) -> R<i128> {
        if let Some(t) = self.ty_of(e) {
            if t.name != w.name {
                return Err(Self::fail(e, "const_type", format!("expected {}, found {}", w.name, t.name),
                    "convert explicitly, e.g. u32(X)"));
            }
        }
        let range = |v: Option<i128>, what: &str| -> R<i128> {
            match v {
                Some(v) if w.contains(v) => Ok(v),
                _ => Err(Self::fail(e, "const_range", format!("constant {} is out of range for {}", what, w.name),
                    "keep every intermediate value within the declared type")),
            }
        };
        match e {
            Expr::IntLit(l) if l.kind == LitKind::Int && w.is_char() => Err(Self::fail(e, "const_type",
                "expected char, found an integer literal".to_string(), "write a char literal such as 'a', or char(X) with X: u32")),
            Expr::IntLit(l) => range(l.value.parse::<i128>().ok(), "literal"),
            Expr::Var(x) => match self.decls.iter().position(|d| d.0.val == x.name) {
                Some(k) => self.value_of(k),
                None => Err(Self::fail(e, "const_expr", format!("{} is not a constant", repr_str(&x.name)),
                    "a constant uses only literals, other constants, arithmetic and int-width casts")),
            },
            Expr::Unary(u) if u.op == "-" && !w.is_char() => {
                let v = self.eval(&u.operand, w)?;
                range(v.checked_neg(), "negation")
            }
            Expr::Binary(b) if matches!(b.op.as_str(), "+" | "-" | "*" | "/" | "%") && !w.is_char() => {
                let l = self.eval(&b.left, w)?;
                let r = self.eval(&b.right, w)?;
                if (b.op == "/" || b.op == "%") && r == 0 {
                    return Err(Self::fail(e, "const_div_zero", format!("division by zero in `{}`", b.op),
                        "a constant divisor must be nonzero"));
                }
                if (b.op == "/" || b.op == "%") && (r < 0 || l < 0) {
                    // the run-time rule (R-6p) asks divisor > 0 and dividend >= 0; Rust truncates where Euclid floors
                    return Err(Self::fail(e, "const_expr", format!("`{}` on a negative operand", b.op),
                        "a constant `/` or `%` takes a dividend >= 0 and a divisor > 0, as at run time"));
                }
                let v = match b.op.as_str() {
                    "+" => l.checked_add(r),
                    "-" => l.checked_sub(r),
                    "*" => l.checked_mul(r),
                    "/" => l.checked_div_euclid(r),
                    _ => l.checked_rem_euclid(r),
                };
                range(v, &format!("arithmetic `{}`", b.op))
            }
            Expr::Call(c) if c.args.len() == 1 && c.targs.is_none() && intwidth::of(&c.name).is_some() => {
                let to = intwidth::of(&c.name).unwrap_or(&intwidth::I64);
                let src = self.ty_of(&c.args[0]).unwrap_or(&intwidth::I64);
                if (to.is_char() && !src.is_char() && src.name != "u8" && src.name != "u32") || (src.is_char() && !intwidth::CHAR.fits_in(to)) {
                    return Err(Self::fail(e, "const_type", format!("no cast from {} to {}", src.name, to.name),
                        "char converts from u8 or u32 and to u32 or wider"));
                }
                let v = self.eval(&c.args[0], src)?;
                range(Some(v).filter(|v| to.contains(*v)), "cast")
            }
            _ => Err(Self::fail(e, "const_expr", "this expression cannot be evaluated at compile time".to_string(),
                "a constant uses only literals, other constants, + - * / %, unary - and int-width casts")),
        }
    }
}

/// P7-1: an expression that is a bool by its form (a literal, `!`, a comparison or `&&`/`||`), so a fixed-array
/// length written that way is `type/const_type` rather than `type/const_expr`.
fn is_bool_shaped(e: &Expr) -> bool {
    match e {
        Expr::BoolLit(_) => true,
        Expr::Unary(u) => u.op == "!",
        Expr::Binary(b) => matches!(b.op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||"),
        _ => false,
    }
}

/// P7-1: an element type a fixed-length array cannot hold yet.
fn fixed_element(line: i64, col: i64, what: &str) -> PruvError {
    PruvError::new("type", line, col, "fixed_array_element",
        format!("a fixed-length array holds scalar integers, char or bool, found {}", what),
        "use a buffer ([T]) or a Vec for other elements")
}

/// P7-1: the storage cap of one fixed-length array (checked_mul of the element size and the length).
pub(crate) fn fixed_size(line: i64, col: i64, elem: &GType, n: u64) -> R<()> {
    let bytes = elem.int_width().map_or(1, |w| (w.bits / 8) as u64);
    match bytes.checked_mul(n) {
        Some(b) if b <= crate::gtype::FIXED_MAX_BYTES => Ok(()),
        _ => Err(PruvError::new("type", line, col, "fixed_array_length",
            format!("a fixed-length array is at most {} bytes; [{}; {}] is larger", crate::gtype::FIXED_MAX_BYTES,
                crate::gtype::format_type(elem), n),
            "use a buffer (alloc) for large arrays")),
    }
}

fn is_upper(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
    /// Depth of the AST path being built (recursion plus binary/postfix chains that pile up to the left). Beyond MAX_DEPTH it stops with an internal error,
    /// like Python's RecursionError: an AST that deep can overflow even a 1GB stack in the recursion of the parser, checker, verifier and generator.
    depth: usize,
    /// G3: type-parameter name -> bound parameter type, inside a generic declaration (empty elsewhere).
    scope: HashMap<String, GType>,
    // ---- G4 A0 parse-level module data (see ParsedFile) ----
    mods: Vec<ModDecl>,
    has_pub: bool,
    items: Vec<Item>,
    fn_pub: Vec<bool>,
    ty_pub: Vec<bool>,
    // ---- P6 tuples ----
    /// tuple types in order of first use: (name, components, line, col of the first use)
    tuples: Vec<(String, Vec<GType>, i64, i64)>,
    /// counter of the destructuring temporaries `pruv_tup_d<N>`
    tup_tmp: usize,
    /// statements a destructuring `let` adds after the one `stmt` returns
    pending: Vec<Stmt>,
    // ---- P7 constants ----
    /// evaluated module constants: name -> (type name, value); a use of the name parses as that literal
    consts: HashMap<String, (String, i128)>,
    /// P7-1: the next `type_g` may read a fixed-length array (only a local `let` annotation for now, P7-1 A)
    fixed_ok: bool,
}


/// P6: a type key that owns a heap payload the destructuring let must move (str, a buffer, a Vec, or a tuple of one).
fn owned_key(k: &str) -> bool {
    let g = GType::from_key(k);
    if let Some(cs) = crate::gtype::tuple_components(&g.name) {
        return cs.iter().any(|c| owned_key(c));
    }
    matches!(g.name.as_str(), "str" | "string" | "String" | "vec" | "Vec") || (g.args.is_empty() && crate::typeinfo::is_array_name(&g.name))
}

/// AST depth limit. Python stops with RecursionError at about 1000 levels (outside the cross-check range), so this is set much larger than that,
/// but small enough that the later stages' recursion fits in the 1GB stack (RESULTS §20).
pub const MAX_DEPTH: usize = 50_000;

impl Parser {
    // ---- helpers ----
    fn idx(&self, k: usize) -> usize {
        (self.i + k).min(self.t.len() - 1)
    }

    /// Adds one level of depth (internal error if exceeded). Undone by the paired leave.
    fn enter(&mut self, n: usize) -> R<()> {
        self.depth += n;
        if self.depth > MAX_DEPTH {
            return Err(PruvError::new("internal", 0, 0, "internal_error", "maximum recursion depth exceeded", ""));
        }
        Ok(())
    }

    // On an error exit the depth is not restored (the whole parse stops). So the subtraction is saturating.
    fn leave(&mut self, n: usize) {
        self.depth = self.depth.saturating_sub(n);
    }

    fn peek(&self, k: usize) -> &Tok {
        &self.t[self.idx(k)]
    }

    fn at(&self, val: &str, k: usize) -> bool {
        let t = &self.t[self.idx(k)];
        (t.kind == Kind::Op || t.kind == Kind::Kw) && t.val == val
    }

    fn at_ident(&self, val: &str, k: usize) -> bool {
        let t = self.peek(k);
        t.kind == Kind::Id && t.val == val
    }

    fn next(&mut self) -> Tok {
        let t = self.t[self.idx(0)].clone();
        self.i += 1;
        t
    }

    fn err<M: Into<String>, F: Into<String>>(&self, msg: M, fix: F, tok: Option<&Tok>) -> PruvError {
        let t = match tok {
            Some(t) => t,
            None => self.peek(0),
        };
        PruvError::new("parse", t.line, t.col, "syntax", msg, fix)
    }

    fn expect(&mut self, val: &str) -> R<()> {
        if !self.at(val, 0) {
            let t = self.peek(0);
            let found = if t.kind == Kind::Eof { "end of file".to_string() } else { t.repr() };
            return Err(self.err(format!("expected '{}' but found {}", val, found), format!("insert '{}'", val), None));
        }
        self.bump();
        Ok(())
    }

    // Clone the received token only when needed (audit: removed token cloning on a hot path).
    fn expect_tok(&mut self, val: &str) -> R<Tok> {
        self.expect(val)?;
        Ok(self.t[self.i - 1].clone())
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    fn ident(&mut self) -> R<Tok> {
        let t = self.peek(0);
        if t.kind == Kind::Op && t.val == "_" {
            return Err(self.reserved_name(&t.clone()));
        }
        if t.kind != Kind::Id {
            return Err(self.err(format!("expected identifier, found {}", t.repr()), "", None));
        }
        if self.consts.contains_key(&t.val) {
            // P7: Rust reads a binder named like a constant as a pattern; a declared name never reuses a constant's.
            return Err(PruvError::new("parse", t.line, t.col, "const_shadow",
                format!("{} is a constant and cannot be declared again here", repr_str(&t.val)),
                "pick another name"));
        }
        Ok(self.next())
    }

    /// P7: reads every `const NAME: T = expr;` item up front and evaluates them with a closed evaluator
    /// (literals, other constants, arithmetic and int-width casts), so uses anywhere in the file parse as literals.
    fn const_prepass(&mut self) -> R<Vec<ConstItem>> {
        let mut decls: Vec<(Tok, &'static intwidth::IntWidth, Expr)> = Vec::new();
        let mut j = 0;
        while j < self.t.len() {
            if !(self.t[j].kind == Kind::Kw && self.t[j].val == "const") {
                j += 1;
                continue;
            }
            self.i = j + 1;
            let n = self.ident()?;
            if !n.val.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') || !is_upper(&n.val) {
                // Rust's const style; it also keeps constants apart from parameter and local names in the generated module
                return Err(PruvError::new("parse", n.line, n.col, "const_name",
                    format!("constant {} must be written in upper case, e.g. MAX_LEN", repr_str(&n.val)),
                    "rename it with capital letters, digits and _"));
            }
            self.expect(":")?;
            let tt = self.next();
            let w = match intwidth::of(&tt.val) {
                Some(w) if tt.kind == Kind::Id => w,
                _ => {
                    return Err(PruvError::new("type", tt.line, tt.col, "const_type",
                        format!("a constant has an integer width or char type, found {}", tt.repr()),
                        "bool and compound constants are not available yet (parity row P7-4); write the value inline"));
                }
            };
            self.expect("=")?;
            let v = self.expr()?;
            self.expect(";")?;
            if decls.iter().any(|d| d.0.val == n.val) {
                return Err(PruvError::new("parse", n.line, n.col, "const_shadow",
                    format!("constant {} is declared twice", repr_str(&n.val)), "remove one declaration"));
            }
            decls.push((n, w, v));
            j = self.i;
        }
        self.i = 0;
        let mut ev = ConstEval { decls: &decls, vals: HashMap::new(), busy: Vec::new() };
        let mut out = Vec::new();
        for k in 0..decls.len() {
            let v = ev.value_of(k)?;
            out.push(ConstItem { line: decls[k].0.line, col: decls[k].0.col, name: decls[k].0.val.clone(),
                ty: decls[k].1.name.to_string(), value: v });
        }
        for c in &out {
            self.consts.insert(c.name.clone(), (c.ty.clone(), c.value));
        }
        Ok(out)
    }

    /// P7-1: the length of `[T; N]` / `[x; N]` after the `;`: a closed constant expression, computed in its own
    /// width (an untyped literal expression in i64), then taken as a nonnegative length.
    fn fixed_len(&mut self) -> R<u64> {
        let e = self.expr()?;
        if is_bool_shaped(&e) {
            return Err(PruvError::new("type", e.line(), e.col(), "const_type",
                "an array length is an integer, found bool", "write an integer constant expression"));
        }
        let mut ev = ConstEval { decls: &[], vals: HashMap::new(), busy: Vec::new() };
        let w = ev.ty_of(&e).unwrap_or(&intwidth::I64);
        if w.is_char() {
            return Err(PruvError::new("type", e.line(), e.col(), "const_type",
                "an array length is an integer, found char", "write an integer constant expression"));
        }
        let v = ev.eval(&e, w)?;
        if v < 0 {
            return Err(PruvError::new("type", e.line(), e.col(), "fixed_array_length",
                format!("an array length must be >= 0, found {}", v), "write a nonnegative length"));
        }
        u64::try_from(v).map_err(|_| PruvError::new("type", e.line(), e.col(), "fixed_array_length",
            "the array length is too large", ""))
    }

    /// P7-1: `[T; N]` after `[T` (at the `;`): only scalar integer and bool elements, at most FIXED_MAX_BYTES.
    fn fixed_array_type(&mut self, open: &Tok, e: &Tok, fixed_ok: bool) -> R<GType> {
        let elem = if e.val == "bool" { GType::bool() } else if intwidth::of(&e.val).is_some() {
            GType::new(&e.val, Vec::new())
        } else {
            return Err(fixed_element(e.line, e.col, &e.val));
        };
        self.expect(";")?;
        let n = self.fixed_len()?;
        self.expect("]")?;
        if !fixed_ok {
            return Err(PruvError::new("type", open.line, open.col, "fixed_array_position",
                "a fixed-length array is a local value only for now: not a parameter, result, field or type argument (parity row P7-1 B)",
                "declare it with let inside the function body"));
        }
        fixed_size(open.line, open.col, &elem, n)?;
        Ok(GType::fixed(elem, n))
    }

    /// A field name after `.`: an identifier, or a tuple index `0`, `1`, .. (P6).
    fn field_ident(&mut self) -> R<Tok> {
        let t = self.peek(0).clone();
        if t.kind == Kind::Int {
            if t.end - t.col != t.val.len() as i64 {
                return Err(PruvError::new("parse", t.line, t.col, "tuple_index",
                    "a tuple index is a plain decimal number", "write t.0, t.1, .."));
            }
            return Ok(self.next());
        }
        self.ident()
    }

    /// A variant binder of a pattern: a name, or (P8) a tuple pattern bound to a fresh `pruv_tup_d<N>` temporary
    /// the checker destructures.
    fn arm_binder(&mut self, binders: &mut Vec<String>, pats: &mut Vec<Option<TuplePat>>) -> R<()> {
        if self.at("(", 0) {
            let pat = self.tuple_pat()?;
            binders.push(format!("pruv_tup_d{}", self.tup_tmp));
            self.tup_tmp += 1;
            pats.push(Some(pat));
        } else {
            binders.push(self.ident()?.val);
            pats.push(None);
        }
        Ok(())
    }

    /// The `(b0, b1, ..)` binder list of a variant pattern.
    fn arm_binders(&mut self, what: &str) -> R<(Vec<String>, Vec<Option<TuplePat>>)> {
        let (mut binders, mut pats) = (Vec::new(), Vec::new());
        if self.at("(", 0) {
            let lp = self.next();
            if self.at(")", 0) {
                return Err(self.err(
                    "a pattern lists one name per field; a variant without fields has no parentheses",
                    format!("write `{} =>`", what),
                    Some(&lp),
                ));
            }
            loop {
                self.arm_binder(&mut binders, &mut pats)?;
                if !self.at(",", 0) {
                    break;
                }
                self.bump();
            }
            self.expect(")")?;
        }
        if pats.iter().all(Option::is_none) {
            pats.clear();
        }
        Ok((binders, pats))
    }

    /// One variant pattern of a match arm, `if let` or `let else`, without its body: `crate::m::E::V(..)`,
    /// `E::V(..)`, `Some(x)`, `None`, `Ok(x)`, `Ok(())` or `Err(e)`; a binder may be a tuple pattern (P8).
    fn arm_pattern(&mut self) -> R<MatchArm> {
        let arm = |ct: &Tok, enum_name: Option<String>, ctor: String, (binders, pats): (Vec<String>, Vec<Option<TuplePat>>)|
            MatchArm { line: ct.line, col: ct.col, enum_name, ctor, binders, body: Vec::new(), pats, pat_lets: 0 };
        if self.at("crate", 0) {
            // G4: `crate::m::Enum::Variant(x) =>`; the last segment is the variant
            let (ct, segs) = self.crate_path(false)?;
            if segs.len() < 2 {
                return Err(self.path_syntax(&ct, "a variant pattern is written crate::module::Enum::Variant"));
            }
            let enum_path = format!("crate::{}", segs[..segs.len() - 1].join("::"));
            let variant = segs[segs.len() - 1].clone();
            let bs = self.arm_binders(&format!("{}::{}", enum_path, variant))?;
            return Ok(arm(&ct, Some(enum_path), variant, bs));
        }
        let ct = self.next();
        if ct.kind == Kind::Id && self.at("::", 0) {
            self.bump();
            let vt = self.ident()?;
            let bs = self.arm_binders(&format!("{}::{}", ct.val, vt.val))?;
            return Ok(arm(&ct, Some(ct.val.clone()), vt.val, bs));
        }
        let is_ctor = ct.kind != Kind::Int && matches!(ct.val.as_str(), "Some" | "None" | "Ok" | "Err");
        if !is_ctor {
            return Err(self.err(
                "match arm must be Some(x), None, Ok(x), Err(x) or Enum::Variant(...)",
                "",
                Some(&ct),
            ));
        }
        let (mut binders, mut pats) = (Vec::new(), Vec::new());
        if ct.val == "Ok" && self.at("(", 0) && self.at("(", 1) && self.at(")", 2) {
            // LB8 R-6g7-1: `Ok(())` binds nothing
            self.bump();
            self.bump();
            self.bump();
            self.expect(")")?;
        } else if ct.val != "None" {
            self.expect("(")?;
            self.arm_binder(&mut binders, &mut pats)?;
            self.expect(")")?;
        }
        if pats.iter().all(Option::is_none) {
            pats.clear();
        }
        Ok(arm(&ct, None, ct.val.clone(), (binders, pats)))
    }

    /// P8b: a slice pattern arm `[]`, `[a, _, b]`, `[a, ..]`, `[a, tail @ ..]` or `[..]`. The prefix binders are
    /// `binders` (`_` skips an element); the shape is in `ctor`: `[]` for an exact length, `[..]` for an unnamed rest
    /// and `[..name]` for a named rest (see `check::slice_arm_shape`). A rest that is not last is
    /// parse/slice_pattern_rest.
    fn slice_arm(&mut self) -> R<MatchArm> {
        let open = self.next();
        let mut binders = Vec::new();
        let mut rest: Option<String> = None;
        let mut has_rest = false;
        while !self.at("]", 0) {
            if has_rest {
                let t = self.peek(0).clone();
                return Err(PruvError::new("parse", t.line, t.col, "slice_pattern_rest",
                    "a slice pattern has at most one rest `..`, and it is the last element",
                    "match the prefix and bind the remainder with `name @ ..` at the end"));
            }
            if self.at("..", 0) {
                self.bump();
                has_rest = true;
            } else if self.at("_", 0) && self.peek(0).kind == Kind::Op {
                self.bump();
                binders.push("_".to_string());
            } else {
                let n = self.ident()?;
                if self.at("@", 0) {
                    self.bump();
                    self.expect("..")?;
                    has_rest = true;
                    rest = Some(n.val);
                } else {
                    binders.push(n.val);
                }
            }
            if !self.at(",", 0) {
                break;
            }
            self.bump();
        }
        self.expect("]")?;
        let ctor = match (has_rest, rest) {
            (false, _) => "[]".to_string(),
            (true, None) => "[..]".to_string(),
            (true, Some(n)) => format!("[..{}]", n),
        };
        Ok(MatchArm { line: open.line, col: open.col, enum_name: None, ctor, binders, body: Vec::new(),
            pats: Vec::new(), pat_lets: 0 })
    }

    /// P6: the tuple type with these components; registers its synthetic struct.
    fn tuple_type(&mut self, open: &Tok, comps: Vec<GType>) -> R<GType> {
        if comps.len() < 2 || comps.len() > crate::gtype::TUPLE_MAX_ARITY {
            return Err(PruvError::new("parse", open.line, open.col, "tuple_arity",
                format!("a tuple type has 2 to {} components, found {}", crate::gtype::TUPLE_MAX_ARITY, comps.len()),
                "write (A, B) up to 8 components; a single type needs no parentheses"));
        }
        if !self.scope.is_empty() {
            return Err(PruvError::new("parse", open.line, open.col, "tuple_generic",
                "tuple types are not available inside a generic declaration",
                "declare a named generic struct, or use the tuple in a non-generic function"));
        }
        for c in &comps {
            if c == &GType::unit() || c.name == "@ref" || c.view_base().is_some() || c.view_payload().is_some() {
                return Err(PruvError::new("parse", open.line, open.col, "tuple_component",
                    format!("{} cannot be a tuple component", crate::gtype::format_type(c)),
                    "tuple components are scalars, structs, enums and owned values; use a named struct for views"));
            }
        }
        let name = crate::gtype::tuple_name(&comps);
        if !self.tuples.iter().any(|t| t.0 == name) {
            self.tuples.push((name.clone(), comps, open.line, open.col));
        }
        Ok(GType::new(&name, Vec::new()))
    }

    /// P6 destructuring pattern: `_`, `[mut] name` or `(p0, p1, ..)`.
    fn tuple_pat(&mut self) -> R<TuplePat> {
        if self.at("_", 0) && self.peek(0).kind == Kind::Op {
            self.bump();
            return Ok(TuplePat::Wild);
        }
        if self.at("(", 0) {
            let open = self.next();
            let mut ps = Vec::new();
            loop {
                ps.push(self.tuple_pat()?);
                if self.at(",", 0) {
                    self.bump();
                    if self.at(")", 0) {
                        break;
                    }
                    continue;
                }
                break;
            }
            self.expect(")")?;
            if ps.len() < 2 || ps.len() > crate::gtype::TUPLE_MAX_ARITY {
                return Err(PruvError::new("parse", open.line, open.col, "tuple_arity",
                    format!("a tuple pattern has 2 to {} components, found {}", crate::gtype::TUPLE_MAX_ARITY, ps.len()),
                    "write let (a, b) = e;"));
            }
            return Ok(TuplePat::Tuple(open.line, open.col, ps));
        }
        let mut mutable = false;
        if self.at("mut", 0) {
            self.bump();
            mutable = true;
        }
        let n = self.ident()?;
        Ok(TuplePat::Bind(n.line, n.col, n.val, mutable))
    }

    /// The component types of the lets `lower_pat` emits for `pats` matched against tuple type `g`, in the same order.
    fn type_pat(g: &GType, pats: &[TuplePat], line: i64, col: i64, out: &mut Vec<String>) -> R<()> {
        let comps = if g.args.is_empty() { crate::gtype::tuple_components(&g.name) } else { None };
        let Some(comps) = comps else {
            return Err(PruvError::new("parse", line, col, "tuple_pattern_type",
                format!("a tuple pattern needs a tuple type, found {}", crate::gtype::format_type(g)),
                "annotate the let with the tuple type (A, B)"));
        };
        if comps.len() != pats.len() {
            return Err(PruvError::new("parse", line, col, "tuple_pattern_type",
                format!("the pattern has {} components but {} has {}", pats.len(), crate::gtype::format_type(g), comps.len()),
                "write one binder or `_` per component"));
        }
        for (p, k) in pats.iter().zip(comps.iter()) {
            match p {
                TuplePat::Wild => {}
                TuplePat::Bind(..) => out.push(k.clone()),
                TuplePat::Tuple(l, c, inner) => {
                    out.push(k.clone());
                    Self::type_pat(&GType::from_key(k), inner, *l, *c, out)?;
                }
            }
        }
        Ok(())
    }

    /// Lowers a destructuring pattern bound to `tmp` into field lets (pushed onto `self.pending`).
    fn lower_pat(&mut self, tmp: &str, pats: Vec<TuplePat>, line: i64, col: i64) {
        for (i, p) in pats.into_iter().enumerate() {
            let read = |l: i64, c: i64| Expr::FieldAccess(FieldAccess {
                line: l, col: c,
                base: Box::new(Expr::Var(Var { line: l, col: c, name: tmp.to_string(), ty: None })),
                field: i.to_string(), ty: None,
            });
            match p {
                TuplePat::Wild => {}
                TuplePat::Bind(l, c, name, mutable) => {
                    self.pending.push(Stmt::Let(Let { borrowed: false, line: l, col: c, name, mutable,
                        ty: String::new(), value: read(l, c), refine: None }));
                }
                TuplePat::Tuple(l, c, inner) => {
                    let t = format!("pruv_tup_d{}", self.tup_tmp);
                    self.tup_tmp += 1;
                    self.pending.push(Stmt::Let(Let { borrowed: false, line: l, col: c, name: t.clone(), mutable: false,
                        ty: String::new(), value: read(l, c), refine: None }));
                    self.lower_pat(&t, inner, line, col);
                }
            }
        }
    }

    // ---- program ----
    fn program(&mut self) -> R<Program> {
        let mut p = Program::default();
        while self.peek(0).kind != Kind::Eof {
            let mut attribute = None;
            if self.at("#[", 0) {
                let at_tok = self.next();
                let name = self.peek(0);
                let (kind, len) = match name.val.as_str() {
                    "cold" if name.kind == Kind::Id && self.at("]", 1) => (FnAttribute::Cold, 2),
                    "inline" if name.kind == Kind::Id && self.at("]", 1) => (FnAttribute::Inline, 2),
                    "inline" if name.kind == Kind::Id && self.at("(", 1)
                        && self.peek(2).kind == Kind::Id && self.peek(2).val == "always"
                        && self.at(")", 3) && self.at("]", 4) => (FnAttribute::InlineAlways, 5),
                    _ => return Err(self.unknown_attribute(&at_tok)),
                };
                self.i += len;
                if !self.at("fn", 0) {
                    return Err(self.attribute_position(&at_tok));
                }
                attribute = Some(kind);
            }
            let mut pub_tok: Option<Tok> = None;
            if self.at("pub", 0) {
                // G4: visibility of the next item
                let pt = self.next();
                if self.at("pub", 0) {
                    let t = self.peek(0).clone();
                    return Err(self.visibility_syntax(&t, "`pub` is written once", "remove the second `pub`"));
                }
                if self.at("mod", 0) {
                    return Err(self.visibility_syntax(
                        &pt,
                        "`pub mod` is not supported: module namespaces have no visibility",
                        "write `mod name;`",
                    ));
                }
                pub_tok = Some(pt);
            }
            let is_pub = pub_tok.is_some();
            if self.at("const", 0) {
                let ct = self.next();
                if let Some(pt) = &pub_tok {
                    return Err(PruvError::new("parse", pt.line, pt.col, "const_module",
                        "`pub const` is not available yet (parity row P7-3)", "drop `pub`; the constant stays private"));
                }
                // already parsed and evaluated by const_prepass: skip to the closing `;`
                while !self.at(";", 0) {
                    if self.peek(0).kind == Kind::Eof {
                        return Err(self.err("expected ';' after a constant", "", Some(&ct)));
                    }
                    self.bump();
                }
                self.bump();
                continue;
            }
            if self.at("mod", 0) {
                let m = self.mod_decl()?;
                self.mods.push(m);
            } else if self.at("struct", 0) {
                let d = TypeDecl::Struct(self.struct_decl()?);
                self.ty_pub.push(is_pub);
                p.types.push(d);
                self.items.push(Item::Ty(p.types.len() - 1));
            } else if self.at("enum", 0) {
                let d = TypeDecl::Enum(self.enum_decl()?);
                self.ty_pub.push(is_pub);
                p.types.push(d);
                self.items.push(Item::Ty(p.types.len() - 1));
            } else if self.at("extern", 0) {
                let tok = self.next();
                self.function(Some(tok.clone()), is_pub)?;
                return Err(self.reserved_name(&tok));
            } else if self.peek(0).kind == Kind::Id && self.peek(0).val == "trusted" {
                // G6 A0: `trusted rust fn name(..) -> T ensures .. = "::path";`
                let t = self.peek(0).clone();
                if let Some(pt) = &pub_tok {
                    return Err(self.visibility_syntax(
                        pt,
                        "a trusted rust fn cannot be `pub`",
                        "keep it private and expose it through a public Pruv function with explicit contracts",
                    ));
                }
                if !(self.peek(1).kind == Kind::Id && self.peek(1).val == "rust" && self.at("fn", 2)) {
                    return Err(self.foreign_syntax(
                        &t,
                        "expected `rust fn` after `trusted`",
                        "write `trusted rust fn name(..) -> T ensures .. = \"::path\";`",
                    ));
                }
                self.bump();
                self.bump();
                let f = self.function(Some(t), false)?;
                self.fn_pub.push(false);
                p.functions.push(f);
                self.items.push(Item::Fn(p.functions.len() - 1));
            } else if self.peek(0).kind == Kind::Id && self.peek(0).val == "opaque" && self.at("fn", 1) {
                // P13o: `opaque fn` keeps its body out of contracts until `reveal`
                self.bump();
                let mut f = self.function(None, is_pub)?;
                f.opaque = true;
                self.fn_pub.push(is_pub);
                p.functions.push(f);
                self.items.push(Item::Fn(p.functions.len() - 1));
            } else if self.peek(0).kind == Kind::Id && self.peek(0).val == "pred" && self.at("fn", 1) {
                self.bump();
                let mut f = self.function(None, is_pub)?;
                f.pred = true;
                self.fn_pub.push(is_pub);
                p.functions.push(f);
                self.items.push(Item::Fn(p.functions.len() - 1));
            } else if self.peek(0).kind == Kind::Id && self.peek(0).val == "rust" && self.at("fn", 1) {
                let t = self.peek(0).clone();
                return Err(self.foreign_syntax(&t, "missing `trusted`", "write `trusted rust fn ...`"));
            } else {
                let mut f = self.function(None, is_pub)?;
                f.attribute = attribute;
                self.fn_pub.push(is_pub);
                p.functions.push(f);
                self.items.push(Item::Fn(p.functions.len() - 1));
            }
            if is_pub {
                self.has_pub = true;
            }
        }
        Ok(p)
    }

    fn unknown_attribute(&self, tok: &Tok) -> PruvError {
        PruvError::new(
            "parse",
            tok.line,
            tok.col,
            "unknown_attribute",
            "unknown function attribute",
            "use #[cold], #[inline] or #[inline(always)]",
        )
    }

    fn attribute_position(&self, tok: &Tok) -> PruvError {
        PruvError::new(
            "parse",
            tok.line,
            tok.col,
            "attribute_position",
            "a function attribute must be followed by a private fn item",
            "put one attribute directly before a private fn",
        )
    }

    fn small_int_unsupported(&self, tok: &Tok, msg: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "small_int_unsupported",
            format!("{}: {}", tok.val, msg), "use i64")
    }

    fn foreign_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "foreign_syntax", msg, fix)
    }

    // ---- G4 A0: module declarations and paths ----
    fn visibility_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "visibility_syntax", msg, fix)
    }

    fn module_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "module_syntax", msg, fix)
    }

    fn path_syntax(&self, tok: &Tok, msg: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "path_syntax", msg, "write an absolute path such as crate::math::clamp")
    }

    /// `mod name;` at top level -> ModDecl
    fn mod_decl(&mut self) -> R<ModDecl> {
        let kw = self.next();
        let t = self.peek(0).clone();
        if t.kind == Kind::Str {
            return Err(self.module_syntax(&t, "a module path cannot be a string", "write `mod name;`; the file is <name>.pruv"));
        }
        if self.at("{", 0) {
            return Err(self.module_syntax(&t, "inline modules are not supported", "move the items into <name>.pruv and write `mod name;`"));
        }
        if t.kind != Kind::Id {
            return Err(self.module_syntax(&t, "expected a module name after `mod`", "write `mod name;`"));
        }
        self.bump();
        if !mod_name_ok(&t.val) {
            return Err(self.module_syntax(
                &t,
                &format!("module name {} must match [a-z][A-Za-z0-9_]*", t.val),
                &format!("rename it, e.g. {}", lower_first(&t.val)),
            ));
        }
        if self.at("{", 0) {
            let p = self.peek(0).clone();
            return Err(self.module_syntax(
                &p,
                "inline modules are not supported",
                &format!("move the items into {}.pruv and write `mod {};`", t.val, t.val),
            ));
        }
        if !self.at(";", 0) {
            let p = self.peek(0).clone();
            return Err(self.module_syntax(&p, &format!("expected ';' after `mod {}`", t.val), &format!("write `mod {};`", t.val)));
        }
        self.bump();
        Ok(ModDecl { line: kw.line, col: kw.col, name: t.val })
    }

    /// At `crate`: `crate::a::b` -> (crate token, [segment names]); stops before a `::<` when allow_targs.
    fn crate_path(&mut self, allow_targs: bool) -> R<(Tok, Vec<String>)> {
        let t = self.next();
        let mut segs: Vec<String> = Vec::new();
        while self.at("::", 0) {
            if self.at("<", 1) {
                if allow_targs && !segs.is_empty() {
                    break;
                }
                let p1 = self.peek(1).clone();
                return Err(PruvError::new(
                    "parse",
                    p1.line,
                    p1.col,
                    "path_syntax",
                    "type arguments are not allowed here",
                    "attach type arguments to the item: crate::m::f::<T>(x)",
                ));
            }
            self.bump();
            let n = self.peek(0).clone();
            if n.kind != Kind::Id {
                let found = if n.kind == Kind::Eof { "end of file".to_string() } else { n.repr() };
                return Err(self.path_syntax(&n, &format!("expected a name after `::` in a path, found {}", found)));
            }
            segs.push(self.next().val);
        }
        if segs.is_empty() {
            let p = self.peek(0).clone();
            return Err(self.path_syntax(&p, "a path needs at least one name after `crate::`"));
        }
        Ok((t, segs))
    }

    /// A `crate::..` path in a type position -> a pseudo identifier token whose value is the full path.
    fn type_path(&mut self) -> R<Tok> {
        let (t, segs) = self.crate_path(false)?;
        Ok(Tok { kind: Kind::Id, val: format!("crate::{}", segs.join("::")), line: t.line, col: t.col, end: 0 })
    }

    fn inout_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "inout_syntax", msg, fix)
    }

    /// G5: at an `inout` that marks a call argument: the token after it starts an operand (a name, number, string, or a
    /// keyword/path that can start an expression). `inout` followed by an operator, `(`, `[`, `.`, `,` or `)` stays an
    /// ordinary identifier.
    fn is_inout_marker(&self) -> bool {
        let t = self.peek(0);
        if !(t.kind == Kind::Id && t.val == "inout") {
            return false;
        }
        let n = self.peek(1);
        if n.kind == Kind::Id || n.kind == Kind::Int || n.kind == Kind::Str {
            return true;
        }
        n.kind == Kind::Kw && matches!(n.val.as_str(), "true" | "false" | "None" | "Some" | "Ok" | "Err" | "crate")
    }

    /// G5: one call argument: an expression, or the marker `inout <operand>` (one postfix expression, kept as written so
    /// the checker can reject fields, elements and calls as places).
    fn arg_expr(&mut self) -> R<Expr> {
        if self.is_inout_marker() {
            let t = self.next();
            let e = self.postfix()?;
            if !(self.at(",", 0) || self.at(")", 0)) {
                return Err(self.inout_syntax(
                    &t,
                    "malformed inout argument: the marker takes a single operand",
                    "write `inout name`; compute other values into a local first",
                ));
            }
            return Ok(Expr::InoutArg(InoutArg { line: t.line, col: t.col, expr: Box::new(e), ty: None, post_refine: Vec::new() }));
        }
        self.expr()
    }

    /// At `(`: the argument list of a call or constructor.
    fn call_args(&mut self) -> R<Vec<Expr>> {
        self.bump();
        let mut args = Vec::new();
        while !self.at(")", 0) {
            args.push(self.arg_expr()?);
            if !self.at(")", 0) {
                self.expect(",")?;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    /// At `{`: `name: expr, ...` up to and including `}`.
    fn struct_inits(&mut self) -> R<Vec<FieldInit>> {
        self.bump();
        let mut inits = Vec::new();
        while !self.at("}", 0) {
            let fname = self.ident()?;
            self.expect(":")?;
            let value = self.expr()?;
            inits.push(FieldInit { line: fname.line, col: fname.col, name: fname.val, value });
            if !self.at("}", 0) {
                self.expect(",")?;
            }
        }
        self.expect("}")?;
        Ok(inits)
    }

    /// G4: an absolute path in an expression.
    ///   crate::m::f(args) / crate::m::Enum::Variant(args) -> Call(name = the whole path) (the resolver tells a function from a variant)
    ///   crate::m::Enum::Variant                           -> EnumCtor without arguments
    ///   crate::m::S { .. }                                -> StructLit
    ///   crate::m::f::<T>(args), crate::m::S::<A> { .. }, crate::m::E::<A>::V(args): type arguments attach to the item
    fn crate_expr(&mut self) -> R<Expr> {
        let (t, segs) = self.crate_path(true)?;
        let (line, col) = (t.line, t.col);
        let path = format!("crate::{}", segs.join("::"));
        let last_upper = segs.last().map_or(false, |s| is_upper(s));
        if self.at("::", 0) && self.at("<", 1) {
            let targs = self.expr_targs()?;
            if !(self.at("::", 0) || self.at("(", 0) || (self.at("{", 0) && last_upper)) {
                let p = self.peek(0).clone();
                return Err(self.generic_syntax(
                    &p,
                    "a type application must be followed by `(`, `{` or `::Variant`",
                    "write crate::m::f::<T>(x), crate::m::Pair::<A, B> { .. } or crate::m::Choice::<A, B>::Left(x)",
                ));
            }
            if self.at("::", 0) {
                self.bump();
                let vt = self.ident()?;
                let mut args = Vec::new();
                if self.at("(", 0) {
                    let lp = self.peek(0).clone();
                    args = self.call_args()?;
                    if args.is_empty() {
                        return Err(self.err(
                            "empty parentheses after a variant",
                            format!("write `{}::{}` for a variant without fields, or pass its fields as arguments", path, vt.val),
                            Some(&lp),
                        ));
                    }
                }
                return Ok(Expr::EnumCtor(EnumCtor { line, col, enum_name: path, variant: vt.val, args, ty: None, targs: Some(targs), inst: None }));
            }
            if self.at("{", 0) {
                let fields = self.struct_inits()?;
                return Ok(Expr::StructLit(StructLit { line, col, name: path, fields, ty: None, targs: Some(targs), inst: None }));
            }
            let args = self.call_args()?;
            return Ok(Expr::Call(Call { line, col, name: path, args, ty: None, targs: Some(targs), inst: None, old: None, opaque_c: false }));
        }
        if self.at("(", 0) {
            let args = self.call_args()?;
            return Ok(Expr::Call(Call { line, col, name: path, args, ty: None, targs: None, inst: None, old: None, opaque_c: false }));
        }
        if self.at("{", 0) && last_upper {
            let fields = self.struct_inits()?;
            return Ok(Expr::StructLit(StructLit { line, col, name: path, fields, ty: None, targs: None, inst: None }));
        }
        if segs.len() < 2 {
            return Err(PruvError::new(
                "parse",
                t.line,
                t.col,
                "path_syntax",
                "a path expression must be a call, a struct literal or an enum variant",
                "write crate::module::f(x), crate::module::S { .. } or crate::module::Enum::Variant",
            ));
        }
        let enum_name = format!("crate::{}", segs[..segs.len() - 1].join("::"));
        Ok(Expr::EnumCtor(EnumCtor {
            line,
            col,
            enum_name,
            variant: segs[segs.len() - 1].clone(),
            args: Vec::new(),
            ty: None,
            targs: None,
            inst: None,
        }))
    }

    fn reserved_name(&self, tok: &Tok) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "reserved_name",
            format!("name {} is reserved for the compiler", repr_str(&tok.val)),
            "rename it; names starting with pruv_, containing __, `_` and Rust keywords are reserved")
    }

    fn field_decl(&mut self) -> R<FieldDecl> {
        let n = self.ident()?;
        self.expect(":")?;
        let ty = self.type_()?;
        Ok(FieldDecl { borrowed: false, line: n.line, col: n.col, name: n.val, ty })
    }

    /// `name: Type, ...` up to close (close is not consumed); a trailing comma is allowed
    fn field_list(&mut self, close: &str) -> R<Vec<FieldDecl>> {
        let mut out = Vec::new();
        while !self.at(close, 0) {
            out.push(self.field_decl()?);
            if !self.at(close, 0) {
                self.expect(",")?;
            }
        }
        Ok(out)
    }

    // ---- G3 A1: type parameters and type applications ----
    fn generic_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "generic_syntax", msg, fix)
    }

    /// `<T, U: Element>` (the `<` is the current token). Also installs the parse scope: inside the declaration a bare
    /// parameter name is a bound parameter type (owner identity + index).
    fn type_params(&mut self, owner: &str) -> R<Vec<TParam>> {
        self.expect("<")?;
        if self.at(">", 0) {
            let t = self.peek(0).clone();
            return Err(self.generic_syntax(
                &t,
                "a type-parameter list cannot be empty",
                "remove `<>`, or declare at least one parameter, e.g. <T>",
            ));
        }
        let mut out: Vec<TParam> = Vec::new();
        loop {
            let n = self.ident()?;
            let mut bound: Option<String> = None;
            if self.at(":", 0) {
                self.bump();
                let b = self.peek(0).clone();
                if b.kind != Kind::Id {
                    return Err(self.generic_syntax(&b, "expected a bound name after ':'", "the only bound is Element: <T: Element>"));
                }
                self.bump();
                bound = Some(b.val.clone());
                if self.at("+", 0) {
                    let t = self.peek(0).clone();
                    return Err(self.generic_syntax(
                        &t,
                        "bound lists are not supported",
                        "a parameter has at most one bound: <T: Element>",
                    ));
                }
            }
            out.push(TParam { line: n.line, col: n.col, name: n.val, bound });
            if !self.at(",", 0) {
                break;
            }
            self.bump();
            if self.at(">", 0) {
                let t = self.peek(0).clone();
                return Err(self.generic_syntax(&t, "a type-parameter list has no trailing comma", "remove the last ','"));
            }
        }
        self.expect(">")?;
        self.scope = HashMap::new();
        for (i, tp) in out.iter().enumerate() {
            self.scope.entry(tp.name.clone()).or_insert_with(|| mk_param(owner, i, &tp.name));
        }
        Ok(out)
    }

    /// Type arguments after an already consumed `<`, through the closing `>`. expr: expression position,
    /// where a malformed list is a generic_syntax error.
    fn type_arg_list(&mut self, expr: bool) -> R<Vec<GType>> {
        let mut out: Vec<GType> = Vec::new();
        loop {
            if self.at(">", 0) || self.at(",", 0) {
                let t = self.peek(0).clone();
                return Err(self.generic_syntax(
                    &t,
                    "missing type argument",
                    "write a type between the angle brackets, e.g. Pair<i64, bool>",
                ));
            }
            let at = self.peek(0).clone();
            let arg = self.type_g(false)?.0;
            if intwidth::is_narrow(&arg.name) || arg.name == "array_u16"
                || arg.view_base().is_some_and(|base| base.name == "array_u16")
            {
                return Err(self.small_int_unsupported(&at, "non-i64 integer and [u16] type arguments require P11"));
            }
            out.push(arg);
            if !self.at(",", 0) {
                break;
            }
            self.bump();
        }
        if !self.at(">", 0) {
            if expr {
                let t = self.peek(0).clone();
                return Err(self.generic_syntax(
                    &t,
                    "malformed type application: expected '>' after the type arguments",
                    "write f::<T>(x), Pair::<A, B> { .. } or Choice::<A, B>::Left(x)",
                ));
            }
            self.expect(">")?;
        }
        self.bump();
        Ok(out)
    }

    /// At `::` `<` in an expression: consumes `::<T, ...>`.
    fn expr_targs(&mut self) -> R<Vec<GType>> {
        self.expr_targs_for_result(false)
    }

    fn expr_targs_for_result(&mut self, result: bool) -> R<Vec<GType>> {
        self.expect("::")?;
        self.expect("<")?;
        let mut args = self.type_arg_list(true)?;
        // `Ok::<T, i64>` / `Err::<T, i64>` is the one-argument form (canonical Result<T>).
        if result && args.len() == 2 && args[1] == GType::new("i64", Vec::new()) {
            args.pop();
        }
        Ok(args)
    }

    /// n: an already consumed identifier naming a type parameter, Option/Result or a user type (no refinement;
    /// type arguments are parsed here).
    fn named_type(&mut self, n: &Tok) -> R<GType> {
        if n.val == "Option" || n.val == "Result" {
            self.expect("<")?;
            let result = n.val == "Result";
            let mut args = self.type_arg_list(false)?;
            if result && args.len() == 2 {
                let e = args.pop().unwrap();
                return Ok(mk_result_with_error(args.pop().unwrap(), e));
            }
            if args.len() == 1 {
                let x = args.swap_remove(0);
                return Ok(if n.val == "Option" { mk_option(x) } else { mk_result(x) });
            }
            // wrong arity: the checker reports it
            return Ok(GType::new(if n.val == "Option" { "option" } else { "result" }, args));
        }
        if self.at("<", 0) {
            self.bump();
            // also a parameter applied to arguments (`T<X>`): kept unresolved, the checker reports it
            let args = self.type_arg_list(false)?;
            return Ok(GType::new(&n.val, args));
        }
        if let Some(t) = self.scope.get(&n.val) {
            return Ok(t.clone());
        }
        Ok(GType::new(&n.val, Vec::new()))
    }

    fn struct_decl(&mut self) -> R<StructDecl> {
        let kw = self.expect_tok("struct")?;
        let n = self.ident()?;
        let tparams = if self.at("<", 0) { self.type_params(&decl_owner(&n.val))? } else { Vec::new() };
        if self.at("(", 0) {
            // P6: tuple structs are not part of the tuple bundle
            return Err(PruvError::new("parse", n.line, n.col, "tuple_struct",
                format!("tuple struct {} is not available; structs have named fields", n.val),
                format!("write `struct {} {{ a: A, b: B }}`, or use a tuple type (A, B)", n.val)));
        }
        self.expect("{")?;
        // LG16: fields, then zero or more `invariant <expr>` clauses, each optionally followed by `,`
        let mut fields = Vec::new();
        while !self.at("}", 0) && !self.at("invariant", 0) {
            fields.push(self.field_decl()?);
            if !self.at("}", 0) && !self.at("invariant", 0) {
                self.expect(",")?;
            }
        }
        let mut invariants = Vec::new();
        let mut invariant_lines = Vec::new();
        while self.at("invariant", 0) {
            invariant_lines.push(self.next().line);
            invariants.push(self.expr()?);
            if self.at(",", 0) {
                self.bump();
            }
        }
        self.expect("}")?;
        self.scope = HashMap::new();
        Ok(StructDecl { line: kw.line, col: kw.col, name: n.val, fields, tparams, invariants, invariant_lines })
    }

    fn enum_decl(&mut self) -> R<EnumDecl> {
        let kw = self.expect_tok("enum")?;
        let n = self.ident()?;
        let tparams = if self.at("<", 0) { self.type_params(&decl_owner(&n.val))? } else { Vec::new() };
        self.expect("{")?;
        let mut variants = Vec::new();
        while !self.at("}", 0) {
            let v = self.ident()?;
            let mut fields = Vec::new();
            if self.at("(", 0) {
                let lp = self.next();
                if self.at(")", 0) {
                    return Err(self.err(
                        "a variant without fields is written without parentheses",
                        format!("write `{}` instead of `{}()`", v.val, v.val),
                        Some(&lp),
                    ));
                }
                fields = self.field_list(")")?;
                self.expect(")")?;
            }
            variants.push(VariantDecl { line: v.line, col: v.col, name: v.val, fields });
            if !self.at("}", 0) {
                self.expect(",")?;
            }
        }
        self.expect("}")?;
        self.scope = HashMap::new();
        Ok(EnumDecl { line: kw.line, col: kw.col, name: n.val, variants, tparams })
    }

    /// A type in a position where refinement annotations are not allowed (fields, etc.)
    fn type_(&mut self) -> R<Type> {
        Ok(self.type_r(false)?.0)
    }

    // ---- STAGE1 A: refinement annotations ----
    fn ref_position(&self, tok: &Tok) -> PruvError {
        PruvError::new(
            "parse",
            tok.line,
            tok.col,
            "refinement_position",
            "a refinement annotation is only allowed on a parameter, a return type or a let/let mut type",
            "move the condition into requires/ensures/invariant, or annotate the parameter/local instead",
        )
    }

    /// If fix is empty, the default hint text.
    fn ref_syntax(&self, tok: &Tok, msg: String, fix: &str) -> PruvError {
        let fix = if fix.is_empty() {
            "write bounds as signed decimal integer literals, e.g. -1_000_000_000..=1_000_000_000"
        } else {
            fix
        };
        PruvError::new("parse", tok.line, tok.col, "refinement_syntax", msg, fix)
    }

    /// Signed decimal integer literal (arbitrary-length normalized string; range checks are in refine.rs)
    fn ref_int(&mut self, what: &str) -> R<String> {
        let t = self.peek(0).clone();
        let mut neg = false;
        if self.at("-", 0) || self.at("+", 0) {
            neg = t.val == "-";
            self.bump();
        }
        let n = self.peek(0).clone();
        if n.kind != Kind::Int {
            let found = if n.kind == Kind::Eof { "end of file".to_string() } else { n.repr() };
            return Err(self.ref_syntax(&n, format!("refinement {} must be a decimal integer literal, found {}", what, found), ""));
        }
        self.bump();
        let nx = self.peek(0).clone();
        if nx.kind == Kind::Id && nx.line == n.line && nx.col == n.end {
            let mut cs = nx.val.chars(); // the exponent wording only for exponent notation (e/E plus digits)
            let first = cs.next();
            let fix = if matches!(first, Some('e') | Some('E')) && cs.all(|c| c.is_ascii_digit()) {
                "exponent notation is not supported: write the full decimal number, e.g. 1_000_000_000 for 1e9"
            } else {
                "only plain decimal integer literals are allowed: write the full decimal number, e.g. 16 instead of 0x10"
            };
            return Err(self.ref_syntax(
                &nx,
                format!("refinement {} must be a decimal integer literal; `{}{}` is not one", what, n.val, nx.val),
                fix,
            ));
        }
        for op in ["*", "+", "-", "/", "%"] {
            if self.at(op, 0) {
                return Err(self.ref_syntax(
                    &nx,
                    format!("refinement {} must be a single integer literal, found an expression with `{}`", what, op),
                    "bounds are integer literals only: compute the value and write it out, e.g. 0..=6 instead of 0..=2*3",
                ));
            }
        }
        Ok(if neg { negate(&n.val) } else { n.val })
    }

    /// `L..=U` after `|`
    fn ref_range(&mut self) -> R<(String, String)> {
        let lo = self.ref_int("lower bound")?;
        if !self.at("..", 0) {
            let t = self.peek(0).clone();
            return Err(self.ref_syntax(
                &t,
                "expected '..=' after the lower bound of a refinement".to_string(),
                "a value range is written closed: `L..=U` (both ends included)",
            ));
        }
        let dd = self.next();
        let eq = self.peek(0).clone();
        if !(self.at("=", 0) && eq.line == dd.line && eq.col == dd.col + 2) {
            return Err(self.ref_syntax(
                &eq,
                "expected '..=' (closed range) in a refinement, found '..'".to_string(),
                "a value range is written `L..=U`; `L..U` is only used by forall/exists/sum/count/max/min and excludes U",
            ));
        }
        self.bump();
        let hi = self.ref_int("upper bound")?;
        Ok((lo, hi))
    }

    /// `; <= N` (the `;` has not been consumed yet)
    fn ref_len(&mut self) -> R<String> {
        self.expect(";")?;
        self.expect("<=")?;
        self.ref_int("length bound")
    }

    /// -> (Type, Refinement or None). allow: whether a refinement annotation is legal in this position.
    fn type_r(&mut self, allow: bool) -> R<(Type, Option<Refinement>)> {
        let (g, r) = self.type_g(allow)?;
        Ok((g.to_key(), r))
    }

    /// The structured form of `type_r`.
    fn type_g(&mut self, allow: bool) -> R<(GType, Option<Refinement>)> {
        let fixed_ok = std::mem::replace(&mut self.fixed_ok, false);
        if self.at_ident(crate::lexer::REF_IDENT, 0) && (self.at("[", 1) || self.at_ident("str", 1)) {
            self.bump();
            let (base, refinement) = self.type_g(allow)?;
            return Ok((GType::new("@ref", vec![base]), refinement));
        }
        let t = self.peek(0).clone();
        if self.at("(", 0) {
            let open = self.next();
            if self.at(")", 0) {
                self.bump();
                return Ok((GType::unit(), None));
            }
            // P6: a tuple type `(T1, T2, ..)`
            let mut comps = Vec::new();
            loop {
                let (c, r) = self.type_g(false)?;
                if r.is_some() {
                    return Err(PruvError::new("parse", open.line, open.col, "tuple_component",
                        "a tuple component cannot carry a refinement",
                        "refine the whole binding with a named struct invariant, or state it as a contract"));
                }
                comps.push(c);
                if self.at(",", 0) {
                    self.bump();
                    if self.at(")", 0) {
                        break;
                    }
                    continue;
                }
                break;
            }
            self.expect(")")?;
            return Ok((self.tuple_type(&open, comps)?, None));
        }
        if self.at("[", 0) {
            let open = self.next();
            if self.at("[", 0) || self.at("(", 0) {
                // P7-1: a nested or tuple element of a fixed-length array
                let inner = self.peek(0).clone();
                let _ = self.type_g(false)?;
                if self.at(";", 0) {
                    return Err(fixed_element(inner.line, inner.col, "a compound type"));
                }
                return Err(self.err("only [i64], [u8], [u16] and [bool] arrays exist", "use [i64], [u8], [u16] or [bool]", Some(&inner)));
            }
            let e = if self.at("crate", 0) { self.type_path()? } else { self.ident()? }; // G4: a qualified element type
            if self.at(";", 0) && !self.at("<=", 1) {
                return Ok((self.fixed_array_type(&open, &e, fixed_ok)?, None));
            }
            if intwidth::PENDING_INTS.contains(&e.val.as_str()) {
                return Err(self.small_int_unsupported(&e, "this integer type requires P3"));
            }
            let elem: GType = if e.val == "i64" {
                GType::i64()
            } else if e.val == "u8" {
                GType::u8()
            } else if e.val == "u16" {
                GType::u16()
            } else if e.val == "bool" {
                GType::bool()
            } else if crate::intwidth::is_narrow(&e.val) {
                // P3: the other scalar widths have no buffer type yet (parity row P3-1)
                return Err(self.small_int_unsupported(&e,
                    "arrays of this width are not available yet (parity row P3-1); only [i64], [u8], [u16] and [bool] arrays exist"));
            } else if is_upper(&e.val) || e.val.starts_with("crate::") {
                // G3: a type parameter (the checker: needs Element); other types: the checker rejects them
                self.named_type(&e)?
            } else {
                return Err(self.err("only [i64], [u8], [u16] and [bool] arrays exist", "use [i64], [u8], [u16] or [bool]", Some(&e)));
            };
            // Concrete arrays use their builtin type names.
            let arr_ty = mk_array(elem);
            let (mut lo, mut hi, mut maxlen): (Option<String>, Option<String>, Option<String>) = (None, None, None);
            if self.at("|", 0) {
                if !allow {
                    return Err(self.ref_position(&self.peek(0).clone()));
                }
                self.bump();
                let (l, h) = self.ref_range()?;
                lo = Some(l);
                hi = Some(h);
            }
            if self.at(";", 0) && self.at("<=", 1) {
                if !allow {
                    return Err(self.ref_position(&self.peek(0).clone()));
                }
                maxlen = Some(self.ref_len()?);
            } else if lo.is_some() && self.at(";", 0) {
                let p = self.peek(0).clone();
                return Err(self.ref_syntax(&p, "expected ']' or '; <=N' after the element range".to_string(), ""));
            } else if self.at(";", 0) {
                let p = self.peek(0).clone();
                return Err(self.ref_syntax(
                    &p,
                    "expected '<=' after ';' in an array length bound".to_string(),
                    "write the length bound as `<=N`, e.g. [i64; <=8]",
                ));
            }
            self.expect("]")?;
            if lo.is_none() && maxlen.is_none() {
                return Ok((arr_ty, None));
            }
            return Ok((
                arr_ty,
                Some(Refinement { line: t.line, col: t.col, shape: "array".to_string(), base: e.val.clone(), lo, hi, maxlen }),
            ));
        }
        let n = if self.at("crate", 0) { self.type_path()? } else { self.ident()? }; // G4: a qualified type path
        let ty: GType;
        if intwidth::PENDING_INTS.contains(&n.val.as_str()) {
            return Err(self.small_int_unsupported(&n, "this integer type requires P3"));
        } else if intwidth::of(&n.val).is_some() {
            ty = GType::new(&n.val, Vec::new());
        } else if n.val == "bool" {
            ty = GType::bool();
        } else if n.val == "str" {
            ty = GType::str();
        } else {
            // Option/Result, a type parameter or a user type (G3: with optional type arguments); the checker resolves the names.
            // The lowercase spelling of a built-in Type name must not become an alias.
            if matches!(n.val.as_str(), "unit" | "array" | "option" | "result" | "array_u8" | "array_u16" | "array_bool" | "array_i32") {
                return Err(self.err(
                    format!("unknown type {}", repr_str(&n.val)),
                    "use i64, bool, (), [i64], Option<i64>, Result<i64> or a declared struct/enum",
                    Some(&n),
                ));
            }
            ty = self.named_type(&n)?;
        }
        if self.at("|", 0) {
            if !allow {
                return Err(self.ref_position(&self.peek(0).clone()));
            }
            self.bump();
            let (lo, hi) = self.ref_range()?;
            // A base type other than i64 is reported by refine.rs as type/refinement_base
            return Ok((
                ty,
                Some(Refinement { line: t.line, col: t.col, shape: "scalar".to_string(), base: n.val, lo: Some(lo), hi: Some(hi), maxlen: None }),
            ));
        }
        Ok((ty, None))
    }

    /// foreign: the `trusted` token of a `trusted rust fn` declaration (its `rust` was consumed), else None.
    fn function(&mut self, foreign: Option<Tok>, is_pub: bool) -> R<Function> {
        let mut kw = self.expect_tok("fn")?;
        if let Some(t) = &foreign {
            kw = t.clone();
        }
        let name_tok = self.ident()?;
        if intwidth::of(&name_tok.val).is_some() {
            return Err(PruvError::new("parse", name_tok.line, name_tok.col, "reserved",
                format!("{} is a builtin name", name_tok.val), "rename the function"));
        }
        let name = name_tok.val;
        let tparams = if self.at("<", 0) { self.type_params(&fn_owner(&name))? } else { Vec::new() };
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.at(")", 0) {
            let mut take = false;
            let mut mode_tok: Option<Tok> = None;
            let t0 = self.peek(0).clone();
            if t0.kind == Kind::Id && t0.val == "take" && self.peek(1).kind == Kind::Id {
                if self.peek(2).kind == Kind::Op && self.peek(2).val == ":" {
                    self.bump();
                    take = true;
                } else if self.peek(1).val == "inout" && self.peek(2).kind == Kind::Id {
                    // G5: `take inout name`
                    return Err(self.inout_syntax(
                        &t0,
                        "a parameter cannot be both take and inout",
                        "keep one mode: `take name: Type` or `inout name: [i64]`",
                    ));
                } else {
                    return Err(self.foreign_syntax(&t0, "malformed take parameter", "write `take name: Type`"));
                }
            } else if t0.kind == Kind::Id && t0.val == "inout" && !(self.peek(1).kind == Kind::Op && self.peek(1).val == ":") {
                // G5: contextual `inout` before a parameter name (`inout: T` is an ordinary parameter named inout)
                let mt = self.next();
                let n1 = self.peek(0).clone();
                if n1.kind != Kind::Id {
                    return Err(self.inout_syntax(&mt, "inout needs a parameter name", "write `inout name: [i64]`"));
                }
                if (n1.val == "take" || n1.val == "inout") && self.peek(1).kind == Kind::Id {
                    return Err(self.inout_syntax(
                        &n1,
                        "a parameter has at most one mode",
                        "keep one mode: `take name: Type` or `inout name: [i64]`",
                    ));
                }
                mode_tok = Some(mt);
            }
            let pn = self.ident()?.val;
            self.expect(":")?;
            let (ty, refine) = self.type_r(true)?;
            params.push(Param { name: pn, ty, refine, take, mode_pos: mode_tok.map(|t| (t.line, t.col)) });
            if !self.at(")", 0) {
                self.expect(",")?;
            }
        }
        self.expect(")")?;
        if self.at("cost", 0) {
            return Err(cost_syntax(self.peek(0)));
        }
        self.expect("->")?;
        let (ret, ret_refine) = self.type_r(true)?;
        if is_pub || foreign.is_some() {
            let signature = || std::iter::once(&ret).chain(params.iter().map(|p| &p.ty));
            if signature().any(|ty| matches!(ty.as_str(), "array_u16" | "@ref<array_u16>")) {
                return Err(self.small_int_unsupported(&kw, "[u16] at the host boundary is bundle P12"));
            }
            if signature().any(|ty| intwidth::is_narrow(ty)) {
                return Err(self.small_int_unsupported(&kw, "non-i64 integers in public or foreign signatures require P12"));
            }
        }
        if kw.val == "extern" {
            return Err(self.reserved_name(&kw));
        }
        let borrow_from = if self.at_ident("from", 0) && self.at("(", 1) {
            self.bump();
            self.expect("(")?;
            let mut names = Vec::new();
            while !self.at(")", 0) {
                let mut path = self.ident()?.val;
                while self.at(".", 0) {
                    self.bump();
                    path.push('.');
                    path.push_str(&self.ident()?.val);
                }
                names.push(path);
                if !self.at(")", 0) {
                    self.expect(",")?;
                }
            }
            self.expect(")")?;
            Some(names)
        } else {
            None
        };
        let mut effects = Vec::new();
        if self.at("!", 0) {
            self.bump();
            loop {
                let e = if self.at("async", 0) { self.expect_tok("async")? } else { self.ident()? };
                if e.val != "io" && e.val != "alloc" && e.val != "fs" && e.val != "proc" && e.val != "net" && e.val != "async" {
                    return Err(self.err(format!("unknown effect {}", repr_str(&e.val)), "effects are io, alloc, fs, proc, net, async", Some(&e)));
                }
                effects.push(e.val);
                if !self.at(",", 0) {
                    break;
                }
                self.bump();
            }
        }
        let (mut req, mut ens) = (Vec::new(), Vec::new());
        let mut late_req: Option<Tok> = None; // Stage 5: first requires after an ensures (rejected only in cost headers)
        while self.at("requires", 0) || self.at("ensures", 0) {
            let kt = self.next();
            let k = kt.val.clone();
            if k == "requires" && !ens.is_empty() && late_req.is_none() {
                late_req = Some(kt);
            }
            let e = self.expr()?;
            if k == "requires" {
                req.push(e);
            } else {
                ens.push(e);
            }
        }
        let decreases = self.decreases_clause()?; // G9 A0: at most one measure, after all requires/ensures
        let cost = self.cost_clause()?;
        if let (Some(_), Some(t)) = (&cost, &late_req) {
            return Err(cost_syntax(t));
        }
        let mut info: Option<ForeignInfo> = None;
        let body;
        if foreign.is_some() {
            if self.at("{", 0) {
                let t = self.peek(0).clone();
                return Err(self.foreign_syntax(&t, "a trusted rust fn has no body", "end it with `= \"::path\";`"));
            }
            if !self.at("=", 0) {
                let t = self.peek(0).clone();
                return Err(self.foreign_syntax(&t, "missing Rust path", "write `= \"::std::cmp::min\";` after the contracts"));
            }
            self.bump();
            let pt = self.peek(0).clone();
            if pt.kind != Kind::Str {
                return Err(self.foreign_syntax(
                    &pt,
                    "missing Rust path",
                    "write the path as a string, e.g. \"::std::cmp::min\"",
                ));
            }
            self.bump();
            self.expect(";")?;
            let path = String::from_utf8_lossy(&decode_str_lit(&pt.val)).into_owned();
            info = Some(ForeignInfo { path, line: pt.line, col: pt.col });
            body = Vec::new();
        } else {
            body = self.block()?;
        }
        self.scope = HashMap::new();
        Ok(Function { borrow_from, borrowed_result: false,
            line: kw.line, col: kw.col, name, params, ret, effects, requires: req, ensures: ens, body, ret_refine, tparams,
            foreign: info,
            decreases,
            cost,
            small_bound: None, x_auto: None, opaque: false, pred: false, attribute: None,
        })
    }

    fn cost_clause(&mut self) -> R<Option<Expr>> {
        if !self.at("cost", 0) { return Ok(None); }
        let t = self.next();
        let bad = cost_syntax;
        if ["{", "=", ";", "cost", "requires", "ensures", "decreases"].contains(&self.peek(0).val.as_str()) || self.peek(0).kind == Kind::Eof {
            return Err(bad(&t));
        }
        let e = self.expr().map_err(|_| bad(self.peek(0)))?;
        if ["cost", "requires", "ensures", "decreases", ",", "!", "->"].contains(&self.peek(0).val.as_str()) {
            return Err(bad(self.peek(0)));
        }
        Ok(Some(e))
    }

    fn decreases_syntax(&self, tok: &Tok, msg: &str, fix: &str) -> PruvError {
        PruvError::new("parse", tok.line, tok.col, "decreases_syntax", msg, fix)
    }

    /// G9 A0: optional `decreases Expr` of a function header (after requires/ensures, before `{` or `=`).
    /// Duplicate, empty, tuple and misplaced clauses are parse/decreases_syntax.
    fn decreases_clause(&mut self) -> R<Option<Expr>> {
        let mut expr: Option<Expr> = None;
        if self.at("decreases", 0) {
            self.bump();
            let t = self.peek(0).clone();
            if (t.kind == Kind::Op && matches!(t.val.as_str(), "{" | "=" | ";")) || t.kind == Kind::Eof {
                return Err(self.decreases_syntax(&t, "empty decreases clause", "write one scalar measure, e.g. `decreases n`"));
            }
            if self.at("(", 0) {
                // a parenthesized group with a top-level comma is a tuple measure
                let mut depth: i64 = 0;
                let mut k: usize = 0;
                loop {
                    let tk = self.peek(k).clone();
                    if tk.kind == Kind::Eof {
                        break;
                    }
                    if tk.kind == Kind::Op && (tk.val == "(" || tk.val == "[") {
                        depth += 1;
                    } else if tk.kind == Kind::Op && (tk.val == ")" || tk.val == "]") {
                        depth -= 1;
                        if depth <= 0 {
                            break;
                        }
                    } else if tk.kind == Kind::Op && tk.val == "," && depth == 1 {
                        return Err(self.decreases_syntax(
                            &tk,
                            "a decreases clause takes one scalar measure, not a tuple",
                            "combine the measures into one i64 expression, e.g. `decreases 2 * n + 1`",
                        ));
                    } else if tk.kind == Kind::Op && matches!(tk.val.as_str(), "{" | "}" | ";") {
                        break;
                    }
                    k += 1;
                }
            }
            expr = Some(self.expr()?);
            if self.at(",", 0) {
                let t = self.peek(0).clone();
                return Err(self.decreases_syntax(
                    &t,
                    "a decreases clause takes one scalar measure, not a list",
                    "combine the measures into one i64 expression, e.g. `decreases 2 * n + 1`",
                ));
            }
        }
        if self.at("decreases", 0) {
            let t = self.peek(0).clone();
            return Err(self.decreases_syntax(
                &t,
                "a function has at most one decreases clause",
                "keep a single `decreases <expr>` after requires/ensures",
            ));
        }
        if self.at("requires", 0) || self.at("ensures", 0) {
            let t = self.peek(0).clone();
            return Err(self.decreases_syntax(
                &t,
                &format!("{} must come before decreases", t.val),
                "move `decreases` after all requires/ensures clauses",
            ));
        }
        Ok(expr)
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.enter(1)?;
        let r = self.block_inner();
        self.leave(1);
        r
    }

    fn block_inner(&mut self) -> R<Vec<Stmt>> {
        self.expect("{")?;
        let out = self.block_rest()?;
        self.expect("}")?;
        Ok(out)
    }

    /// The statements up to the closing `}` (not consumed). P8: the statements after a `let P = e else { .. };`
    /// become the body of its pattern arm, where the binders are in scope.
    fn block_rest(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        while !self.at("}", 0) {
            if self.peek(0).kind == Kind::Eof {
                return Err(self.err("unclosed block", "add '}'", None));
            }
            let st = self.stmt()?;
            let let_else = matches!(&st, Stmt::Match(m) if m.form == MatchForm::LetElse);
            out.push(st);
            out.append(&mut self.pending);
            if let_else {
                self.enter(1)?;
                let rest = self.block_rest();
                self.leave(1);
                let Some(Stmt::Match(m)) = out.last_mut() else { unreachable!("pushed above") };
                m.arms[0].body = rest?;
                break;
            }
        }
        Ok(out)
    }

    /// P8: `if let P = e { A } [else { B } | else if ..]`, lowered by the checker to a match.
    fn if_let(&mut self) -> R<Stmt> {
        let t = self.expect_tok("if")?;
        self.expect("let")?;
        let mut arm = self.arm_pattern()?;
        self.expect("=")?;
        let scr = self.expr()?;
        arm.body = self.block()?;
        let mut els = Vec::new();
        if self.at("else", 0) {
            self.bump();
            if self.at("if", 0) {
                self.enter(1)?;
                let r = if self.at("let", 1) { self.if_let() } else { self.if_().map(Stmt::If) };
                self.leave(1);
                els = vec![r?];
            } else {
                els = self.block()?;
            }
        }
        Ok(Stmt::Match(Match { line: t.line, col: t.col, scrutinee: scr, arms: vec![arm], form: MatchForm::IfLet, els }))
    }

    /// P8: whether `let` at 0 starts `let P = e else { .. };` (a variant pattern, not a binding name).
    fn at_let_else(&self) -> bool {
        let p = self.peek(1);
        (p.kind == Kind::Kw && matches!(p.val.as_str(), "Some" | "None" | "Ok" | "Err" | "crate"))
            || (p.kind == Kind::Id && (self.at("(", 2) || self.at("::", 2)))
    }

    /// Contextual `par {`: members must be semicolon-terminated calls, with no trailing semicolon.
    fn par_stmt(&mut self) -> R<Stmt> {
        let t = self.peek(0).clone();
        self.bump();
        self.bump(); // opening brace
        let mut calls = Vec::new();
        while !self.at("}", 0) {
            let m = self.peek(0).clone();
            if m.kind == Kind::Eof {
                return Err(PruvError::new("parse", m.line, m.col, "parallel_syntax",
                    "unclosed par block", "add '}'"));
            }
            let e = self.expr().map_err(|ex| {
                if ex.code == "syntax" {
                    PruvError::new("parse", ex.line, ex.col, "parallel_syntax",
                        format!("malformed par block: {}", ex.message),
                        "write semicolon-terminated calls: `par { f(inout a); g(inout b); }`")
                } else { ex }
            })?;
            if !matches!(e, Expr::Call(_)) && !matches!(&e, Expr::Unary(u) if u.op == "?") {
                return Err(PruvError::new("parse", m.line, m.col, "parallel_syntax",
                    "a par block member must be a call", "write only calls, e.g. `f(inout a);`"));
            }
            if !self.at(";", 0) {
                let m = self.peek(0);
                return Err(PruvError::new("parse", m.line, m.col, "parallel_syntax",
                    "a par block call must end with ';'", "insert ';' after the call"));
            }
            self.bump();
            calls.push(e);
        }
        self.bump(); // closing brace
        if self.at(";", 0) {
            let m = self.peek(0);
            return Err(PruvError::new("parse", m.line, m.col, "parallel_syntax",
                "a par block takes no trailing ';'", "remove the ';' after '}'"));
        }
        Ok(Stmt::Par(Par { line: t.line, col: t.col, calls }))
    }

    // ---- statements ----
    fn stmt(&mut self) -> R<Stmt> {
        let t = self.peek(0).clone();
        let (l, c) = (t.line, t.col);
        if t.kind == Kind::Id && t.val == "par" && self.at("{", 1) {
            return self.par_stmt();
        }
        if self.at("let", 0) && self.at_let_else() {
            // P8: `let P = e else { .. };`; block_rest moves the following statements into the pattern arm
            self.bump();
            let arm = self.arm_pattern()?;
            self.expect("=")?;
            let scr = self.expr()?;
            if !self.at("else", 0) {
                let n = self.peek(0).clone();
                return Err(PruvError::new("parse", n.line, n.col, "let_else_syntax",
                    "a variant pattern in let needs an else block", "write `let Some(x) = e else { return ..; };`"));
            }
            self.bump();
            let els = self.block()?;
            self.expect(";")?;
            return Ok(Stmt::Match(Match { line: l, col: c, scrutinee: scr, arms: vec![arm], form: MatchForm::LetElse, els }));
        }
        if self.at("if", 0) && self.at("let", 1) {
            return self.if_let();
        }
        if self.at("let", 0) && self.at("(", 1) {
            // P6: `let (a, mut b, _): (A, B, C) = e;` lowers to a temporary and one field let per binder.
            self.bump();
            let pat = self.tuple_pat()?;
            let TuplePat::Tuple(pl, pc, pats) = pat else { unreachable!("pattern starts with `(`") };
            if !self.at(":", 0) {
                return Err(PruvError::new("parse", pl, pc, "tuple_needs_type",
                    "a destructuring let needs the tuple type", "write `let (a, b): (i64, i64) = e;`"));
            }
            self.bump();
            let (g, refine) = self.type_g(false)?;
            if refine.is_some() {
                return Err(self.err("a destructuring let cannot carry a refinement", "drop the refinement", None));
            }
            self.expect("=")?;
            let value = self.expr()?;
            self.expect(";")?;
            let tmp = format!("pruv_tup_d{}", self.tup_tmp);
            self.tup_tmp += 1;
            let mut lets = Vec::new();
            Self::type_pat(&g, &pats, pl, pc, &mut lets)?;
            let save = std::mem::take(&mut self.pending);
            self.lower_pat(&tmp, pats, pl, pc);
            // fill each lowered let with its component type, in lowering order
            let mut lowered = std::mem::take(&mut self.pending);
            for (st, ty) in lowered.iter_mut().zip(lets.into_iter()) {
                if let Stmt::Let(lt) = st {
                    lt.ty = ty;
                }
            }
            // an owned component (str, a buffer, a Vec, or a tuple holding one) is moved out of its temporary, which
            // is then a `let mut` the existing field-move rule consumes
            let mut movers: Vec<String> = Vec::new();
            for st in lowered.iter_mut() {
                let Stmt::Let(lt) = st else { continue };
                if !owned_key(&lt.ty) {
                    continue;
                }
                let Expr::FieldAccess(fa) = &lt.value else { continue };
                let Expr::Var(v) = &*fa.base else { continue };
                movers.push(v.name.clone());
                lt.value = Expr::Move(Move { line: fa.line, col: fa.col, name: v.name.clone(),
                    fields: vec![fa.field.clone()], ty: None });
            }
            for st in lowered.iter_mut() {
                if let Stmt::Let(lt) = st {
                    if movers.contains(&lt.name) {
                        lt.mutable = true;
                    }
                }
            }
            self.pending = save;
            self.pending.append(&mut lowered);
            return Ok(Stmt::Let(Let { borrowed: false, line: l, col: c, mutable: movers.contains(&tmp), name: tmp,
                ty: g.to_key(), value, refine: None }));
        }
        if self.at("let", 0) {
            self.bump();
            let mut mutable = false;
            if self.at("mut", 0) {
                self.bump();
                mutable = true;
            }
            let n = self.ident()?.val;
            let (ty, refine) = if self.at(":", 0) {
                self.bump();
                self.fixed_ok = true;
                self.type_r(true)?
            } else if self.at("=", 0) && self.at_ident("inout", 1) {
                (String::new(), None)
            } else {
                return Err(self.err("let needs a type annotation", format!("write `let {}: i64 = ...;`", n), None));
            };
            self.expect("=")?;
            let value = if self.at_ident("inout", 0) {
                let t = self.next();
                Expr::InoutArg(InoutArg {
                    line: t.line, col: t.col, expr: Box::new(self.place_operand()?),
                    ty: None, post_refine: Vec::new(),
                })
            } else {
                self.expr()?
            };
            self.expect(";")?;
            return Ok(Stmt::Let(Let { borrowed: false, line: l, col: c, name: n, mutable, ty, value, refine }));
        }
        if self.at("if", 0) {
            return Ok(Stmt::If(self.if_()?));
        }
        if self.peek(0).kind == Kind::Op && self.peek(0).val.starts_with('\'') {
            // P5: `'label: while ...` or `'label: for x in ...`
            let lab = self.next();
            self.expect(":")?;
            let name = lab.val[1..].to_string();
            if !(self.at("while", 0) || (self.at("for", 0) && !self.at("(", 1))) {
                return Err(PruvError::new("parse", lab.line, lab.col, "loop_label_target",
                    "a loop label must be followed by `while` or `for x in ...`",
                    "`loop` and labeled blocks are not available; label a while or for loop"));
            }
            let mut st = self.stmt()?;
            let w = match &mut st {
                Stmt::While(w) => w,
                Stmt::If(i) => match i.then.last_mut() {
                    Some(Stmt::While(w)) => w,
                    _ => unreachable!("for lowering ends with its while"),
                },
                _ => unreachable!("labeled statement is a loop"),
            };
            w.label = Some(name);
            return Ok(st);
        }
        if self.at("break", 0) || self.at("continue", 0) {
            let k = self.next();
            let label = if self.peek(0).kind == Kind::Op && self.peek(0).val.starts_with('\'') {
                Some(self.next().val[1..].to_string())
            } else {
                None
            };
            if !self.at(";", 0) {
                // A value after `break` is break_value; a missing `;` (`break }`) is plain jump syntax.
                let ends = self.at("}", 0) || self.peek(0).kind == Kind::Eof;
                let (code, msg, hint) = if k.val == "break" && !ends {
                    ("break_value", "`break` with a value is not available",
                     "only `loop` can break with a value and `loop` is not available; assign the value to a `let mut` before `break;`")
                } else {
                    ("loop_jump_syntax", "`break` and `continue` take an optional label and then `;`",
                     "write `break;`, `continue;`, `break 'label;` or `continue 'label;`")
                };
                return Err(PruvError::new("parse", k.line, k.col, code, msg, hint));
            }
            self.bump();
            let j = Jump { line: l, col: c, label };
            return Ok(if k.val == "break" { Stmt::Break(j) } else { Stmt::Continue(j) });
        }
        if self.at("loop", 0) {
            return Err(PruvError::new("parse", l, c, "loop_unsupported",
                "`loop { }` is not available: every loop needs a measure",
                "write `while cond decreases m { }` with a `break` inside"));
        }
        if self.at("while", 0) {
            self.bump();
            // P8b: `while let P = e` is the loop `while true` whose body is the head match (MatchForm::WhileLet)
            let head = if self.at("let", 0) {
                self.bump();
                let arm = self.arm_pattern()?;
                self.expect("=")?;
                Some((arm, self.expr()?))
            } else {
                None
            };
            let cond = match head {
                Some(_) => Expr::BoolLit(BoolLit { line: l, col: c, value: true, ty: None }),
                None => self.expr()?,
            };
            let mut invs = Vec::new();
            let mut dec: Option<Expr> = None;
            while self.at("invariant", 0) || self.at("decreases", 0) {
                let k = self.next();
                if k.val == "invariant" {
                    invs.push(self.expr()?);
                } else {
                    if dec.is_some() {
                        return Err(self.err("only one decreases clause", "", Some(&k)));
                    }
                    dec = Some(self.expr()?);
                }
            }
            let dec = match dec {
                Some(d) => d,
                None => {
                    return Err(self.err(
                        "while loop needs a `decreases <expr>` clause",
                        "add `decreases n - i` before '{'",
                        None,
                    ))
                }
            };
            let mut body = self.block()?;
            if let Some((mut arm, scrutinee)) = head {
                arm.body = body;
                body = vec![Stmt::Match(Match { line: l, col: c, scrutinee, arms: vec![arm], form: MatchForm::WhileLet,
                    els: vec![Stmt::Break(Jump { line: l, col: c, label: None })] })];
            }
            return Ok(Stmt::While(While { line: l, col: c, cond, invariants: invs, decreases: dec, body, for_head: None, label: None }));
        }
        if self.at("for", 0) && self.at("(", 1) {
            self.bump();
            self.expect("(")?;
            let key = self.ident()?.val;
            self.expect(",")?;
            let val = self.ident()?.val;
            self.expect(")")?;
            self.expect("in")?;
            let place = self.map_iteration_place()?;
            let mut invariants = Vec::new();
            while self.at("invariant", 0) {
                self.bump();
                invariants.push(self.expr()?);
            }
            let body = self.block()?;
            return Ok(Stmt::ForMap(ForMap { line: l, col: c, key, val, place, invariants, body }));
        }
        if self.at("for", 0) {
            if self.peek(1).kind == Kind::Id && self.at("in", 2) {
                self.bump();
                let var = self.ident()?;
                self.bump();
                return self.for_loop(l, c, var);
            }
            return Err(self.reserved_name(&t));
        }
        if self.at("return", 0) {
            self.bump();
            if self.at(";", 0) {
                self.bump();
                return Ok(Stmt::Return(Return { line: l, col: c, value: None }));
            }
            let v = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Return(Return { line: l, col: c, value: Some(v) }));
        }
        if self.at("assert", 0) {
            self.bump();
            let v = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Assert(Assert { line: l, col: c, cond: v, origin_code: None, origin_what: None, reveal: None }));
        }
        if self.peek(0).kind == Kind::Id
            && self.peek(0).val == "reveal"
            && (self.peek(1).kind == Kind::Id || self.at("crate", 1))
        {
            // P13o: `reveal F;` is a proof-only statement (an assert of true that carries the revealed function)
            self.bump();
            let nt = self.peek(0).clone();
            let name = if self.at("crate", 0) {
                let (_, segs) = self.crate_path(false)?;
                format!("crate::{}", segs.join("::"))
            } else {
                self.ident()?.val
            };
            self.expect(";")?;
            return Ok(Stmt::Assert(Assert {
                line: l,
                col: c,
                cond: Expr::BoolLit(BoolLit { line: l, col: c, value: true, ty: None }),
                origin_code: None,
                origin_what: None,
                reveal: Some((name, nt.line, nt.col)),
            }));
        }
        if self.at("match", 0) {
            self.bump();
            let scr = self.expr()?;
            self.expect("{")?;
            let mut arms = Vec::new();
            while !self.at("}", 0) {
                let mut arm = if self.at("(", 0) {
                    // P8: `(a, b) =>` destructures a tuple scrutinee; the checker (Checker::block) turns the
                    // single-arm match into the destructuring lets
                    let ct = self.peek(0).clone();
                    let (mut binders, mut pats) = (Vec::new(), Vec::new());
                    self.arm_binder(&mut binders, &mut pats)?;
                    MatchArm { line: ct.line, col: ct.col, enum_name: None, ctor: "()".to_string(), binders,
                        body: Vec::new(), pats, pat_lets: 0 }
                } else if self.at("[", 0) {
                    self.slice_arm()?
                } else {
                    self.arm_pattern()?
                };
                self.expect("=>")?;
                arm.body = self.block()?;
                arms.push(arm);
                if self.at(",", 0) {
                    self.bump();
                }
            }
            self.expect("}")?;
            return Ok(Stmt::Match(Match { line: l, col: c, scrutinee: scr, arms, form: MatchForm::Match, els: Vec::new() }));
        }
        let deref = self.at("*", 0) && self.at("=", 2);
        if deref || t.kind == Kind::Id && self.at("=", 1) {
            let n = if deref {
                self.bump();
                self.ident()?.val
            } else {
                self.next().val
            };
            self.expect("=")?;
            let v = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Assign(Assign { line: l, col: c, name: n, value: v, deref }));
        }
        if t.kind == Kind::Id && self.at("[", 1) {
            // It may also be an index assignment
            let save = self.i;
            let n = self.next().val;
            self.bump();
            let idx = self.expr()?;
            self.expect("]")?;
            if self.at("=", 0) {
                self.bump();
                let v = self.expr()?;
                self.expect(";")?;
                return Ok(Stmt::IndexAssign(IndexAssign { line: l, col: c, name: n, index: idx, value: v, elem_refine: None, fields: Vec::new() }));
            }
            self.i = save;
        }
        if t.kind == Kind::Id && self.at(".", 1) {
            // Field assignment `p.a.b = e;`, otherwise an expression statement
            let save = self.i;
            let n = self.next().val;
            let mut path = Vec::new();
            while self.at(".", 0) {
                self.bump();
                path.push(self.field_ident()?.val);
            }
            if self.at("=", 0) {
                self.bump();
                let v = self.expr()?;
                self.expect(";")?;
                return Ok(Stmt::FieldAssign(FieldAssign { line: l, col: c, name: n, fields: path, value: v, ref_store: false }));
            }
            if self.at("[", 0) {
                // LG4: element write through a field place `p.a.b[i] = e;` (one index level)
                self.bump();
                let idx = self.expr()?;
                self.expect("]")?;
                if self.at("=", 0) {
                    self.bump();
                    let v = self.expr()?;
                    self.expect(";")?;
                    return Ok(Stmt::IndexAssign(IndexAssign { line: l, col: c, name: n, index: idx, value: v, elem_refine: None, fields: path }));
                }
            }
            self.i = save;
        }
        let e = self.expr()?;
        self.expect(";")?;
        Ok(Stmt::ExprStmt(ExprStmt { line: l, col: c, expr: e }))
    }

    /// P4: a `for` bound or iterated place. An uppercase binding followed by the body block is a variable, not a
    /// struct literal.
    fn for_operand(&mut self) -> R<Expr> {
        let name = self.peek(0);
        if name.kind == Kind::Id && is_upper(&name.val) && !self.consts.contains_key(&name.val) && (self.at("{", 1) || self.at("invariant", 1)) {
            let name = self.next();
            return Ok(Expr::Var(Var { line: name.line, col: name.col, name: name.val, ty: None }));
        }
        self.add()
    }

    /// P4: `for i in a..b { ... }` and `for x in v { ... }`, lowered here to the `while` CFG so drops, borrow
    /// write-back and invariants take the one `while` path:
    ///
    /// ```text
    /// if true {                                        if true {
    ///   let S: T = a; let E: T = b;                      let E: i64 = len(v);
    ///   let mut i: T = S;                                let mut K: i64 = 0;
    ///   while i < E                                      while K < E
    ///     invariant S <= i && (i <= E || i == S)           invariant 0 <= K && K <= E
    ///     invariant <user>                                 invariant <user>
    ///     decreases E - i                                  decreases E - K
    ///   { body; i = i + 1; }                             { let x: _ = v[K]; body; K = K + 1; }
    /// }                                                }
    /// ```
    ///
    /// The bounds are evaluated once, `a` before `b`. `T` (written `i64` here) is resolved by the checker from the
    /// bounds; the checker also makes `i` read-only in the body except for the synthesized step, holds `v` read-only,
    /// and turns a `Vec` element read into `vec_get`. The `if true` scope drops the hidden names after the loop.
    fn for_loop(&mut self, l: i64, c: i64, var: Tok) -> R<Stmt> {
        let first = self.for_operand()?;
        let range = self.at("..", 0);
        let end = if range {
            self.bump();
            if self.at("=", 0) {
                return Err(PruvError::new("parse", l, c, "for_iterable",
                    "a for loop takes a half-open range `a..b`; `a..=b` is not available",
                    "write `a..b + 1` with a proof that b + 1 is in range"));
            }
            Some(self.for_operand()?)
        } else {
            None
        };
        let mut user_invs = Vec::new();
        while self.at("invariant", 0) {
            self.bump();
            user_invs.push(self.expr()?);
        }
        if self.at("decreases", 0) {
            return Err(PruvError::new("parse", l, c, "for_decreases",
                "a for loop generates its own measure; remove the decreases clause", ""));
        }
        if !self.at("{", 0) {
            return Err(PruvError::new("parse", l, c, "for_iterable",
                "a for loop iterates a half-open range `a..b` or a slice/Vec place `v`",
                "iterator adapters (iter, enumerate, zip, rev, map, filter) are not available yet (bundle P15)"));
        }
        let mut body = self.block()?;
        let var_ = |name: &str| Expr::Var(Var { line: l, col: c, name: name.to_string(), ty: None });
        let lit = |v: &str| Expr::IntLit(IntLit { line: l, col: c, value: v.to_string(), ty: None, kind: Default::default() });
        let bin = |op: &str, a: Expr, b: Expr| Expr::Binary(Binary {
            line: l, col: c, op: op.to_string(), left: Box::new(a), right: Box::new(b), ty: None,
        });
        let let_ = |name: &str, mutable: bool, value: Expr, line: i64, col: i64| Stmt::Let(Let {
            borrowed: false, line, col, name: name.to_string(), mutable, ty: "i64".to_string(), value, refine: None,
        });
        let end_name = format!("__pruv_for_end_{l}_{c}");
        let mut lets = Vec::new();
        // R-6q-2: a slice loop also keeps `len(P) == E`, so a write to a sibling place (not P) cannot lose the bound
        let mut len_inv = None;
        let (counter, lower) = match end {
            Some(end) => {
                let start_name = format!("__pruv_for_start_{l}_{c}");
                lets.push(let_(&start_name, false, first, l, c));
                lets.push(let_(&end_name, false, end, l, c));
                lets.push(let_(&var.val, true, var_(&start_name), var.line, var.col));
                (var.val.clone(), var_(&start_name))
            }
            None => {
                let mut place = &first;
                while let Expr::FieldAccess(f) = place {
                    place = &f.base;
                }
                if !matches!(place, Expr::Var(_)) {
                    return Err(PruvError::new("parse", first.line(), first.col(), "for_iterable",
                        "a for loop iterates a half-open range `a..b` or a slice/Vec place `v` (a binding or field path)",
                        "bind the collection to a local first; iterator adapters are bundle P15"));
                }
                let counter = format!("__pruv_for_i_{l}_{c}");
                let len = Expr::Call(Call {
                    line: l, col: c, name: "len".to_string(), args: vec![first.clone()], ty: None,
                    targs: None, inst: None, old: None, opaque_c: false,
                });
                len_inv = Some(bin("==", len.clone(), var_(&end_name)));
                lets.push(let_(&end_name, false, len, l, c));
                lets.push(let_(&counter, true, lit("0"), l, c));
                let read = Expr::Index(Index {
                    line: l, col: c, array: Box::new(first), index: Box::new(var_(&counter)), ty: None,
                });
                body.insert(0, let_(&var.val, false, read, var.line, var.col));
                (counter, lit("0"))
            }
        };
        body.push(Stmt::Assign(Assign {
            line: l, col: c, name: counter.clone(), value: bin("+", var_(&counter), lit("1")), deref: false,
        }));
        let i = || var_(&counter);
        let e = || var_(&end_name);
        let mut invariants = vec![if range {
            bin("&&", bin("<=", lower.clone(), i()), bin("||", bin("<=", i(), e()), bin("==", i(), lower.clone())))
        } else {
            bin("&&", bin("<=", lower.clone(), i()), bin("<=", i(), e()))
        }];
        invariants.extend(len_inv);
        for mut inv in user_invs {
            if !range {
                for_index_rewrite(&mut inv, &var.val, &counter);
            }
            invariants.push(inv);
        }
        lets.push(Stmt::While(While {
            line: l, col: c, cond: bin("<", i(), e()), invariants, decreases: bin("-", e(), i()), body,
            for_head: Some(ForHead { var: var.val, counter, slice: !range }), label: None,
        }));
        Ok(Stmt::If(If { line: l, col: c, cond: Expr::BoolLit(BoolLit { line: l, col: c, value: true, ty: None }),
            then: lets, els: Vec::new() }))
    }

    fn map_iteration_place(&mut self) -> R<Expr> {
        // An uppercase binding followed by a block would otherwise start a struct literal.
        let name = self.peek(0);
        if name.kind == Kind::Id && is_upper(&name.val) && self.at("{", 1) {
            let fields = self.peek(2).kind == Kind::Id && self.at(":", 3);
            let continued = self.at("}", 2) && matches!(self.peek(3).val.as_str(),
                "{" | "invariant" | "decreases" | "[" | "." | "==>" | "||" | "&&"
                | "==" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | "%");
            let deref_assign = continued && self.at("*", 3) && self.peek(4).kind == Kind::Id && self.at("=", 5);
            if !fields && (!continued || deref_assign) {
                let name = self.next();
                return Ok(Expr::Var(Var { line: name.line, col: name.col, name: name.val, ty: None }));
            }
        }
        self.expr()
    }

    fn if_(&mut self) -> R<If> {
        let t = self.expect_tok("if")?;
        let cond = self.expr()?;
        let then = self.block()?;
        let mut els = Vec::new();
        if self.at("else", 0) {
            self.bump();
            if self.at("if", 0) {
                // An else-if chain is counted as depth too (checked after the fix: outside the count, a million of them overflowed the stack)
                self.enter(1)?;
                let r = if self.at("let", 1) { self.if_let() } else { self.if_().map(Stmt::If) };
                self.leave(1);
                els = vec![r?];
            } else {
                els = self.block()?;
            }
        }
        Ok(If { line: t.line, col: t.col, cond, then, els })
    }

    // ---- expressions (precedence climbing) ----
    fn bin(t: &Tok, op: &str, l: Expr, r: Expr) -> Expr {
        Expr::Binary(Binary { line: t.line, col: t.col, op: op.to_string(), left: Box::new(l), right: Box::new(r), ty: None })
    }

    fn expr(&mut self) -> R<Expr> {
        self.enter(1)?;
        let r = self.implies();
        self.leave(1);
        r
    }

    fn implies(&mut self) -> R<Expr> {
        let l = self.or_()?;
        if self.at("==>", 0) {
            let t = self.next();
            self.enter(1)?;
            let r = self.implies();
            self.leave(1);
            return Ok(Self::bin(&t, "==>", l, r?));
        }
        Ok(l)
    }

    fn or_(&mut self) -> R<Expr> {
        let mut l = self.and_()?;
        let mut chain = 0; // a binary chain that piles up to the left is counted as depth too
        while self.at("||", 0) {
            let t = self.next();
            if let Err(e) = self.enter(1) {
                self.leave(chain + 1);
                return Err(e);
            }
            chain += 1;
            let r = match self.and_() {
                Ok(r) => r,
                Err(e) => {
                    self.leave(chain);
                    return Err(e);
                }
            };
            l = Self::bin(&t, "||", l, r);
        }
        self.leave(chain);
        Ok(l)
    }

    fn and_(&mut self) -> R<Expr> {
        let mut l = self.cmp()?;
        let mut chain = 0; // a binary chain that piles up to the left is counted as depth too
        while self.at("&&", 0) {
            let t = self.next();
            if let Err(e) = self.enter(1) {
                self.leave(chain + 1);
                return Err(e);
            }
            chain += 1;
            let r = match self.cmp() {
                Ok(r) => r,
                Err(e) => {
                    self.leave(chain);
                    return Err(e);
                }
            };
            l = Self::bin(&t, "&&", l, r);
        }
        self.leave(chain);
        Ok(l)
    }

    fn cmp(&mut self) -> R<Expr> {
        let l = self.bitwise(0)?;
        for op in CMP_OPS.iter() {
            if self.at(op, 0) {
                let t = self.next();
                let r = self.bitwise(0)?;
                for op2 in CMP_OPS.iter() {
                    if self.at(op2, 0) {
                        return Err(self.err("comparisons cannot be chained", "use `a < b && b < c`", None));
                    }
                }
                return Ok(Self::bin(&t, op, l, r));
            }
        }
        Ok(l)
    }

    fn bitwise(&mut self, level: usize) -> R<Expr> {
        const OPS: [&str; 4] = ["|", "^", "&", "<<"];
        let Some(&op) = OPS.get(level) else { return self.add(); };
        let mut left = self.bitwise(level + 1)?;
        let mut chain = 0;
        let result = (|| {
            loop {
                let right_shift = op == "<<" && self.at(">", 0) && self.at(">", 1)
                    && self.peek(0).line == self.peek(1).line && self.peek(0).end == self.peek(1).col;
                if !self.at(op, 0) && !right_shift {
                    return Ok(left);
                }
                let tok = self.next();
                if right_shift {
                    self.next();
                }
                chain += 1;
                self.enter(1)?;
                let right = self.bitwise(level + 1)?;
                left = Self::bin(&tok, if right_shift { ">>" } else { op }, left, right);
            }
        })();
        self.leave(chain);
        result
    }

    fn add(&mut self) -> R<Expr> {
        let mut l = self.mul()?;
        let mut chain = 0; // a binary chain that piles up to the left is counted as depth too
        while self.at("+", 0) || self.at("-", 0) {
            let t = self.next();
            if let Err(e) = self.enter(1) {
                self.leave(chain + 1);
                return Err(e);
            }
            chain += 1;
            let r = match self.mul() {
                Ok(r) => r,
                Err(e) => {
                    self.leave(chain);
                    return Err(e);
                }
            };
            l = Self::bin(&t, &t.val, l, r);
        }
        self.leave(chain);
        Ok(l)
    }

    fn mul(&mut self) -> R<Expr> {
        let mut l = self.unary()?;
        let mut chain = 0; // a binary chain that piles up to the left is counted as depth too
        while self.at("*", 0) || self.at("/", 0) || self.at("%", 0) {
            let t = self.next();
            if let Err(e) = self.enter(1) {
                self.leave(chain + 1);
                return Err(e);
            }
            chain += 1;
            let r = match self.unary() {
                Ok(r) => r,
                Err(e) => {
                    self.leave(chain);
                    return Err(e);
                }
            };
            l = Self::bin(&t, &t.val, l, r);
        }
        self.leave(chain);
        Ok(l)
    }

    fn unary(&mut self) -> R<Expr> {
        if self.at("-", 0) || self.at("!", 0) || self.at("~", 0) || self.at("*", 0) {
            let t = self.next();
            self.enter(1)?;
            let o = self.unary();
            self.leave(1);
            let o = o?;
            if t.val == "-" {
                match &o {
                    Expr::IntLit(il) if il.kind == crate::ast::LitKind::Int => {
                        return Ok(Expr::IntLit(IntLit { line: t.line, col: t.col, value: negate(&il.value), ty: None, kind: Default::default() }));
                    }
                    _ => {}
                }
            }
            return Ok(Expr::Unary(Unary { line: t.line, col: t.col, op: t.val, operand: Box::new(o), ty: None }));
        }
        self.postfix()
    }

    fn place_operand(&mut self) -> R<Expr> {
        if !self.at("if", 0) {
            return self.postfix();
        }
        self.enter(1)?;
        let t = self.next();
        let cond = self.expr()?;
        self.expect("{")?;
        let then = self.place_operand()?;
        self.expect("}")?;
        self.expect("else")?;
        self.expect("{")?;
        let els = self.place_operand()?;
        self.expect("}")?;
        self.leave(1);
        Ok(PlaceIf { line: t.line, col: t.col, cond, then, els, ty: None }.into_expr())
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        let mut chain = 0; // a postfix chain that piles up to the left, like a[i][j].f, is counted as depth too
        loop {
            if self.at("?", 0) {
                self.bump();
                e = Expr::Unary(Unary { line: e.line(), col: e.col(), op: "?".into(), operand: Box::new(e), ty: None });
            } else if self.at("[", 0) {
                let t = self.next();
                let i = self.expr()?;
                self.expect("]")?;
                e = Expr::Index(Index { line: t.line, col: t.col, array: Box::new(e), index: Box::new(i), ty: None });
            } else if self.at(".", 0) {
                self.bump();
                let f = self.field_ident()?;
                e = Expr::FieldAccess(FieldAccess { line: e.line(), col: e.col(), base: Box::new(e), field: f.val, ty: None });
            } else {
                break;
            }
            self.enter(1)?;
            chain += 1;
        }
        self.leave(chain);
        Ok(e)
    }

    /// `Enum::Variant` -> (enum token, variant token); G4: also `crate::m::Enum::Variant` (enum token value = the enum's path)
    fn enum_ref(&mut self) -> R<(Tok, Tok)> {
        if self.at("crate", 0) {
            let (ct, segs) = self.crate_path(false)?;
            if segs.len() < 2 {
                return Err(self.path_syntax(&ct, "a variant is written crate::module::Enum::Variant"));
            }
            let vt = Tok { kind: Kind::Id, val: segs[segs.len() - 1].clone(), line: ct.line, col: ct.col, end: 0 };
            let en = Tok {
                kind: Kind::Id,
                val: format!("crate::{}", segs[..segs.len() - 1].join("::")),
                line: ct.line,
                col: ct.col,
                end: 0,
            };
            return Ok((en, vt));
        }
        let en = self.ident()?;
        self.expect("::")?;
        let vt = self.ident()?;
        Ok((en, vt))
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.peek(0).clone();
        let (line, col) = (t.line, t.col);
        if t.kind == Kind::Kw && t.val == "loop" {
            // P5: `loop` as an expression (`let x = loop { .. }`) gets the same code as the statement form.
            return Err(PruvError::new("parse", line, col, "loop_unsupported",
                "`loop { }` is not available: every loop needs a measure",
                "write `while cond decreases m { }` with a `break` inside"));
        }
        if t.kind == Kind::Int {
            self.bump();
            return Ok(Expr::IntLit(IntLit { line, col, value: t.val, ty: None, kind: Default::default() }));
        }
        if t.kind == Kind::Char || t.kind == Kind::Byte {
            // P7: a char literal is a `char`, a byte literal a `u8`, whatever the context
            self.bump();
            let (ty, kind) = if t.kind == Kind::Char { ("char", crate::ast::LitKind::Char) } else { ("u8", crate::ast::LitKind::Byte) };
            return Ok(Expr::IntLit(IntLit { line, col, value: t.val, ty: Some(ty.to_string()), kind }));
        }
        if t.kind == Kind::Str {
            self.bump();
            return Ok(Expr::StrLit(StrLit { line, col, value: decode_str_lit(&t.val), ty: None, empty: false }));
        }
        if self.at("[", 0) && self.at("]", 1) {
            // R-6g9-3: `[]`, the static empty view
            self.bump();
            self.bump();
            return Ok(Expr::StrLit(StrLit { line, col, value: Vec::new(), ty: None, empty: true }));
        }
        if self.at("[", 0) {
            // P7-1: array literals `[a, b, c]` (a trailing comma is allowed) and `[x; N]`
            self.bump();
            let first = self.expr()?;
            if self.at(";", 0) {
                self.bump();
                let n = self.fixed_len()?;
                self.expect("]")?;
                return Ok(Expr::Call(Call { line, col, name: format!("{}{}", crate::ast::ARRAY_REPEAT, n), args: vec![first],
                    ty: None, targs: None, inst: None, old: None, opaque_c: false }));
            }
            let mut args = vec![first];
            while self.at(",", 0) {
                self.bump();
                if self.at("]", 0) {
                    break;
                }
                args.push(self.expr()?);
            }
            self.expect("]")?;
            return Ok(Expr::Call(Call { line, col, name: crate::ast::ARRAY_LIST.to_string(), args,
                ty: None, targs: None, inst: None, old: None, opaque_c: false }));
        }
        if t.kind == Kind::Move {
            // Parse the full field path so the checker can enforce where a field move is legal.
            self.bump();
            let n = self.peek(0).clone();
            if n.kind != Kind::Id {
                return Err(self.foreign_syntax(&t, "move takes a binding name", "write `move name`"));
            }
            self.bump();
            let mut fields = Vec::new();
            while self.at(".", 0) {
                self.bump();
                fields.push(self.field_ident()?.val);
                self.enter(1)?;
            }
            self.leave(fields.len());
            return Ok(Expr::Move(Move { line, col, name: n.val, fields, ty: None }));
        }
        if self.at("true", 0) || self.at("false", 0) {
            self.bump();
            return Ok(Expr::BoolLit(BoolLit { line, col, value: t.val == "true", ty: None }));
        }
        if self.at("(", 0) {
            self.bump();
            let e = self.expr()?;
            if self.at(",", 0) {
                // P6: a tuple literal `(e0, e1, ..)`; the checker takes its type from the expected type.
                let mut fields = vec![FieldInit { line: e.line(), col: e.col(), name: "0".into(), value: e }];
                while self.at(",", 0) {
                    self.bump();
                    if self.at(")", 0) {
                        break;
                    }
                    let v = self.expr()?;
                    fields.push(FieldInit { line: v.line(), col: v.col(), name: fields.len().to_string(), value: v });
                }
                self.expect(")")?;
                if fields.len() < 2 || fields.len() > crate::gtype::TUPLE_MAX_ARITY {
                    return Err(PruvError::new("parse", line, col, "tuple_arity",
                        format!("a tuple has 2 to {} components, found {}", crate::gtype::TUPLE_MAX_ARITY, fields.len()),
                        "write (a, b) up to 8 components; `(a,)` is not a tuple in Pruv"));
                }
                return Ok(Expr::StructLit(StructLit { line, col, name: crate::gtype::TUPLE_PENDING.to_string(),
                    fields, ty: None, targs: None, inst: None }));
            }
            self.expect(")")?;
            return Ok(e);
        }
        if self.at("None", 0) {
            self.bump();
            let mut targs: Option<Vec<GType>> = None;
            if self.at("::", 0) && self.at("<", 1) {
                targs = Some(self.expr_targs()?); // G3: None::<T>
            }
            return Ok(Expr::Ctor(Ctor { line, col, name: "None".to_string(), arg: None, ty: None, targs, inst: None }));
        }
        if self.at("Some", 0) || self.at("Ok", 0) || self.at("Err", 0) {
            self.bump();
            let targs = if self.at("::", 0) && self.at("<", 1) {
                Some(self.expr_targs_for_result(t.val != "Some")?)
            } else {
                None
            };
            self.expect("(")?;
            // LB8 R-6g7-1: Ok(()) carries the unit value, no payload
            let a = if t.val == "Ok" && self.at("(", 0) && self.at(")", 1) {
                self.bump();
                self.bump();
                None
            } else {
                Some(Box::new(self.expr()?))
            };
            self.expect(")")?;
            return Ok(Expr::Ctor(Ctor { line, col, name: t.val.clone(), arg: a, ty: None, targs, inst: None }));
        }
        if self.at("spent", 0) {
            self.next();
            return Ok(Expr::Spent(Spent { line: t.line, col: t.col, ty: None }));
        }
        if self.at("result", 0) {
            self.bump();
            return Ok(Expr::Var(Var { line, col, name: "result".to_string(), ty: None }));
        }
        if self.at("forall", 0) || self.at("exists", 0) {
            self.bump();
            let v = self.ident()?.val;
            self.expect("in")?;
            let lo = self.add()?;
            self.expect("..")?;
            let hi = self.add()?;
            let triggers = self.quant_triggers()?;
            self.expect(":")?;
            let body = self.expr()?;
            return Ok(Self::quant(&t, v, lo, hi, body, triggers));
        }
        if t.kind == Kind::Id
            && matches!(t.val.as_str(), "sum" | "count" | "max" | "min")
            && self.at("(", 1)
            && self.peek(2).kind == Kind::Id
            && self.at("in", 3)
        {
            self.bump();
            self.bump();
            let v = self.ident()?.val;
            self.expect("in")?;
            let lo = self.add()?;
            self.expect("..")?;
            let hi = self.add()?;
            let triggers = self.quant_triggers()?;
            self.expect(":")?;
            let body = self.expr()?;
            self.expect(")")?;
            return Ok(Self::quant(&t, v, lo, hi, body, triggers));
        }
        if self.at("is_variant", 0) {
            self.bump();
            self.expect("(")?;
            let v = self.expr()?;
            self.expect(",")?;
            let (en, vt) = self.enum_ref()?;
            self.expect(")")?;
            return Ok(Expr::IsVariant(IsVariant { line, col, value: Box::new(v), enum_name: en.val, variant: vt.val, ty: None, enum_ty: None }));
        }
        if self.at("variant_field", 0) {
            self.bump();
            self.expect("(")?;
            let v = self.expr()?;
            self.expect(",")?;
            let (en, vt) = self.enum_ref()?;
            self.expect(",")?;
            let f = self.ident()?;
            self.expect(")")?;
            return Ok(Expr::VariantField(VariantField {
                line, col, value: Box::new(v), enum_name: en.val, variant: vt.val, field: f.val, ty: None, enum_ty: None,
            }));
        }
        if self.at("crate", 0) {
            return self.crate_expr();
        }
        if t.kind == Kind::Id {
            self.bump();
            if let Some((ty, v)) = self.consts.get(&t.val) {
                if !self.at("(", 0) && !self.at("::", 0) {
                    // P7: a constant use is its evaluated literal; codegen spells it by name
                    return Ok(Expr::IntLit(IntLit { line, col, value: v.to_string(), ty: Some(ty.clone()),
                        kind: LitKind::Const(t.val.clone()) }));
                }
            }
            let mut targs: Option<Vec<GType>> = None;
            if self.at("::", 0) && self.at("<", 1) {
                // G3: f::<T>(x), S::<A, B> { .. }, E::<A, B>::V(x)
                targs = Some(self.expr_targs()?);
                if !(self.at("::", 0) || self.at("(", 0) || (self.at("{", 0) && is_upper(&t.val))) {
                    let p = self.peek(0).clone();
                    return Err(self.generic_syntax(
                        &p,
                        "a type application must be followed by `(`, `{` or `::Variant`",
                        "write f::<T>(x), Pair::<A, B> { .. } or Choice::<A, B>::Left(x)",
                    ));
                }
            }
            if self.at("::", 0) {
                self.bump();
                let vt = self.ident()?;
                let mut args = Vec::new();
                if self.at("(", 0) {
                    let lp = self.next();
                    if self.at(")", 0) {
                        return Err(self.err(
                            "empty parentheses after a variant",
                            format!(
                                "write `{}::{}` for a variant without fields, or pass its fields as arguments",
                                t.val, vt.val
                            ),
                            Some(&lp),
                        ));
                    }
                    while !self.at(")", 0) {
                        args.push(self.expr()?);
                        if !self.at(")", 0) {
                            self.expect(",")?;
                        }
                    }
                    self.expect(")")?;
                }
                return Ok(Expr::EnumCtor(EnumCtor { line, col, enum_name: t.val.clone(), variant: vt.val, args, ty: None, targs, inst: None }));
            }
            if is_upper(&t.val) && self.at("{", 0) {
                self.bump();
                let mut inits = Vec::new();
                while !self.at("}", 0) {
                    let fname = self.ident()?;
                    self.expect(":")?;
                    let value = self.expr()?;
                    inits.push(FieldInit { line: fname.line, col: fname.col, name: fname.val, value });
                    if !self.at("}", 0) {
                        self.expect(",")?;
                    }
                }
                self.expect("}")?;
                return Ok(Expr::StructLit(StructLit { line, col, name: t.val.clone(), fields: inits, ty: None, targs, inst: None }));
            }
            if self.at("(", 0) {
                self.bump();
                let mut args = Vec::new();
                while !self.at(")", 0) {
                    args.push(self.arg_expr()?); // G5: `inout name` marker
                    if !self.at(")", 0) {
                        self.expect(",")?;
                    }
                }
                self.expect(")")?;
                return Ok(Expr::Call(Call { line, col, name: t.val.clone(), args, ty: None, targs, inst: None, old: None, opaque_c: false }));
            }
            return Ok(Expr::Var(Var { line, col, name: t.val, ty: None }));
        }
        Err(self.err(format!("expected an expression, found {}", t.repr()), "", None))
    }

    fn quant(t: &Tok, var: String, lo: Expr, hi: Expr, body: Expr, triggers: Vec<Expr>) -> Expr {
        Expr::Quant(Quant {
            line: t.line, col: t.col, kind: t.val.clone(), var,
            lo: Box::new(lo), hi: Box::new(hi), body: Box::new(body), ty: None, triggers,
        })
    }

    /// R-6-3: `trigger t1, t2` right after the hi bound; `trigger` is a contextual word (an ordinary identifier elsewhere).
    fn quant_triggers(&mut self) -> R<Vec<Expr>> {
        let mut out = Vec::new();
        let t = self.peek(0);
        if !(t.kind == Kind::Id && t.val == "trigger") {
            return Ok(out);
        }
        self.bump();
        out.push(self.add()?);
        while self.at(",", 0) {
            self.bump();
            out.push(self.add()?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ret_expr(src: &str) -> Expr {
        let p = parse_source(&format!("fn f() -> i64 {{ return {}; }}", src)).unwrap();
        match &p.functions[0].body[0] {
            Stmt::Return(Return { value: Some(e), .. }) => e.clone(),
            _ => panic!("not a return"),
        }
    }

    fn int_of(e: &Expr) -> (String, i64) {
        match e {
            Expr::IntLit(i) => (i.value.clone(), i.col),
            other => panic!("not an IntLit: {:?}", other),
        }
    }

    #[test]
    fn cold_attribute() {
        let p = parse_source("#[cold] fn f() -> i64 { return 1; }\nfn g() -> i64 { return 2; }").unwrap();
        assert_eq!(p.functions[0].attribute, Some(FnAttribute::Cold));
        assert_eq!(p.functions[1].attribute, None);
        let e = parse_source("#[hot] fn f() -> i64 { return 1; }").unwrap_err();
        assert_eq!((e.code.as_str(), e.line, e.col), ("unknown_attribute", 1, 1));
        let e = parse_source("#[cold] pub fn f() -> i64 { return 1; }").unwrap_err();
        assert_eq!((e.code.as_str(), e.line, e.col), ("attribute_position", 1, 1));
        let e = parse_source("fn f() -> i64 {\n  #[cold]\n  return 1;\n}").unwrap_err();
        assert_eq!((e.code.as_str(), e.line, e.col), ("attribute_position", 2, 3));
    }

    #[test]
    fn negative_folding() {
        assert_eq!(int_of(&ret_expr("--5")).0, "5");
        assert_eq!(int_of(&ret_expr("-(5)")).0, "-5");
        assert_eq!(int_of(&ret_expr("-0")).0, "0");
        assert_eq!(int_of(&ret_expr("-9223372036854775808")), ("-9223372036854775808".to_string(), 24));
        assert!(matches!(ret_expr("!5"), Expr::Unary(_)));
        assert_eq!(int_of(&ret_expr("340282366920938463463374607431768211456")).0, "340282366920938463463374607431768211456");
    }

    #[test]
    fn unicode_column_and_errors() {
        let e = parse_source("fn // éé").unwrap_err();
        assert_eq!((e.line, e.col), (1, 9));
        assert_eq!(e.message, "expected identifier, found None");
        let e = parse_source("fn f() -> i64 {\r\n  return 1\r\n}").unwrap_err();
        assert_eq!((e.line, e.col), (3, 1));
        assert_eq!(e.message, "expected ';' but found '}'");
        assert_eq!(e.fix, "insert ';'");
    }

    #[test]
    fn crlf_and_cr() {
        let e = parse_source("fn f() -> i64 {\r  $").unwrap_err();
        assert_eq!((e.code.as_str(), e.line, e.col), ("bad_char", 2, 3));
    }
}

fn cost_syntax(t: &Tok) -> PruvError {
    PruvError::new("parse", t.line, t.col, "cost_syntax",
        "cost needs one expression and must be the last header clause",
        "write requires* ensures* decreases? cost <i64 expression>")
}

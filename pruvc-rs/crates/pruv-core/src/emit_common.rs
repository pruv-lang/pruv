//! Common Python-output functions (counterparts of codegen.py's rty, cty, conv, emit_public_types, show,
//! match_sorted_requires, rstr). The output must be byte-identical to Python's.

use crate::ast::*;
use crate::gtype::{is_g3_key, GType};
use crate::typeinfo::{is_user_type, TypeTable};

/// A directly declared u16 buffer in an internal function's signature.
pub(crate) fn has_u16_buffer_sig(f: &Function) -> bool {
    f.ret == "array_u16" || f.params.iter().any(|p| p.ty == "array_u16")
}

/// G3: a generic specialization function (stem `pruv_g3_f_...`); such functions are private Rust items.
/// The prefix `pruv_` is reserved, so no user function can carry it.
pub fn is_spec_fn(name: &str) -> bool {
    name.starts_with("pruv_g3_f_")
}

thread_local! {
    /// G4 A2: Rust name of a nominal type -> its source-qualified identity, for the program being generated (Python keeps it
    /// as `src_name` on the specialized nodes). Empty outside explicit visibility mode.
    static TYPE_DISP: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// G4 A2: registers the identities of the explicit-mode program about to be generated (an empty map clears them).
pub fn set_type_disp(m: std::collections::HashMap<String, String>) {
    TYPE_DISP.with(|t| *t.borrow_mut() = m);
}

/// G4 A3 (Python `getattr(d, "disp", name)`): the registered source-qualified identity of a nominal type, or the name itself
/// outside explicit visibility mode (unlike `src_name`, a G3 stem is NOT unwrapped here).
pub fn type_disp(name: &str) -> String {
    TYPE_DISP.with(|t| t.borrow().get(name).cloned()).unwrap_or_else(|| name.to_string())
}

/// G3: the declaration name a generated type stem (`pruv_g3_t_<len>_<name>_...`) was made from; any other name as is.
/// G4 A2: in explicit mode the identity the type was declared with (`crate::m::Name`).
/// Used where Python keeps `src_name` (panic messages of requires).
pub fn src_name(name: &str) -> String {
    if let Some(d) = TYPE_DISP.with(|t| t.borrow().get(name).cloned()) {
        return d;
    }
    if let Some(rest) = name.strip_prefix("pruv_g3_t_") {
        if let Some(us) = rest.find('_') {
            if let Ok(n) = rest[..us].parse::<usize>() {
                let start = us + 1;
                if let Some(s) = rest.get(start..start + n) {
                    return s.to_string();
                }
            }
        }
    }
    name.to_string()
}

/// The type name of a key: the text before the argument list (`option` for `option<bool>`).
pub fn key_name(t: &str) -> &str {
    t.split('<').next().unwrap_or(t)
}

/// A user struct/enum name; a structured key (`option<bool>`) is not one.
pub fn is_user_key(t: &str) -> bool {
    !is_g3_key(t) && !crate::intwidth::is_narrow(t) && is_user_type(t)
}

pub(crate) const SMALL_INT_HOST_MESSAGE: &str = "integer types other than i64 at the host boundary is bundle P12";
pub(crate) const SMALL_INT_HOST_ERROR: &str = "small_int_unsupported: integer types other than i64 at the host boundary is bundle P12";

/// A u8/u16 value or a fixed-length array in the signature, directly or inside a user struct/enum (P1: such functions stay off the host boundary).
pub(crate) fn has_narrow_scalar_sig(f: &crate::ast::Function, tt: &TypeTable) -> bool {
    let mut seen = std::collections::HashSet::new();
    has_narrow(tt, &f.ret, &mut seen) || f.params.iter().any(|p| has_narrow(tt, &p.ty, &mut seen))
}

/// Every type name in a key (`option<Byte>` reaches `Byte`), each user type visited once.
fn has_narrow(tt: &TypeTable, ty: &str, seen: &mut std::collections::HashSet<String>) -> bool {
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == ':'))
        .filter(|n| !n.is_empty())
        .any(|n| {
            crate::intwidth::is_narrow(n)
                || n.starts_with(&crate::gtype::FIXED_PREFIX[1..]) // P7-1: `[T; N]` stays off the host boundary too
                || (seen.insert(n.to_string())
                    && tt.decl(n).is_some_and(|d| d.all_fields().iter().any(|x| has_narrow(tt, &x.ty, seen))))
        })
}

fn structured_builtin(g: &GType) -> bool {
    g.owner.is_none() && !g.args.is_empty() && (g.name == "option" || g.name == "result")
}

/// Rust spelling of a view wrapper, with the optional result lifetime on its payload.
pub(crate) fn view_wrapper_rty(g: &GType, base: &GType, lifetime: &str) -> String {
    let view = rty_g(base, true);
    let payload = format!("&{lifetime}{}", &view[1..]);
    if g.name == "option" {
        format!("Option<{payload}>")
    } else {
        format!("Result<{payload}, i64>")
    }
}

/// Rust type spelling of a structured Option or Result.
pub fn rty_g(g: &GType, param: bool) -> String {
    if let Some(base) = g.view_base() {
        return rty_g(base, true);
    }
    if let Some(base) = g.view_payload() {
        return view_wrapper_rty(g, base, "");
    }
    if let Some((elem, n)) = g.fixed_parts() {
        // P7-1: a fixed-length array is a Rust array by value
        return format!("[{}; {}]", rty_g(elem, false), n);
    }
    if structured_builtin(g) {
        let x = rty_g(&g.args[0], false);
        if g.name == "option" {
            return format!("::std::option::Option<{}>", x);
        }
        let e = result_error_rty(g);
        let result = format!("::std::result::Result<{}, {}>", x, e);
        return if param && g.has_string_error() { format!("&{}", result) } else { result };
    }
    // G6: owned collections; a borrowed parameter is a reference
    if g.owner.is_none() {
        let r = match (g.name.as_str(), g.args.len()) {
            ("vec", 1) => Some(format!("::std::vec::Vec<{}>", rty_g(&g.args[0], false))),
            ("hashmap", 2) => Some(format!(
                "::std::collections::HashMap<{}, {}>",
                rty_g(&g.args[0], false),
                rty_g(&g.args[1], false)
            )),
            _ => None,
        };
        if let Some(r) = r {
            return if param { format!("&{}", r) } else { r };
        }
    }
    rty_name(&g.name, param)
}

pub(crate) fn result_error_rty(g: &GType) -> String {
    if g.has_string_error() {
        "::std::string::String".to_string()
    } else {
        g.result_error().map_or_else(|| "i64".to_string(), |e| rty_g(e, false))
    }
}

/// Rust type spelling for execution. It covers all 6 built-ins and user types, so there is no failure path.
pub fn rty(t: &str, param: bool) -> String {
    if is_g3_key(t) {
        if let Ok(g) = GType::try_from_key(t) {
            return rty_g(&g, param);
        }
    }
    rty_name(t, param)
}

fn rty_name(t: &str, param: bool) -> String {
    if let Some(comps) = crate::gtype::tuple_components(t) {
        // P6: a native Rust tuple; a component is stored by value, so it uses the non-parameter spelling
        let cs: Vec<String> = comps.iter().map(|k| rty(k, false)).collect();
        return format!("({})", cs.join(", "));
    }
    match t {
        _ if crate::intwidth::of(t).is_some() => t.to_string(),
        "bool" => "bool".to_string(),
        "unit" => "()".to_string(),
        "option" => "::std::option::Option<i64>".to_string(),
        "result" => "::std::result::Result<i64, i64>".to_string(),
        "array" => {
            if param { "&[i64]".to_string() } else { "::std::vec::Vec<i64>".to_string() }
        }
        "array_u8" | "array_u16" => {
            let elem = crate::typeinfo::array_elem_name(t).unwrap();
            if param { format!("&[{elem}]") } else { format!("::std::vec::Vec<{elem}>") }
        }
        // G2 A5: stated explicitly instead of relying on the fallback
        "array_bool" => {
            if param { "&[bool]".to_string() } else { "::std::vec::Vec<bool>".to_string() }
        }
        // G2 B2: str is &str as a parameter, String as a local or return value
        "str" => {
            if param { "&str".to_string() } else { "::std::string::String".to_string() }
        }
        // G10: File is owned; a borrowed parameter is a reference
        "file" => {
            if param { "&pruv_File".to_string() } else { "pruv_File".to_string() }
        }
        // G10 proc A2: Process and ProcessArgs are owned; a borrowed parameter is a reference
        "process" => {
            if param { "&pruv_Process".to_string() } else { "pruv_Process".to_string() }
        }
        "process_args" => {
            if param { "&pruv_ProcessArgs".to_string() } else { "pruv_ProcessArgs".to_string() }
        }
        // G10 net A2: Socket and Listener are owned; a borrowed parameter is a reference
        "socket" => {
            if param { "&pruv_Socket".to_string() } else { "pruv_Socket".to_string() }
        }
        "loop" => { if param { "&pruv_Loop".to_string() } else { "pruv_Loop".to_string() } }
        "listener" => {
            if param { "&pruv_Listener".to_string() } else { "pruv_Listener".to_string() }
        }
        // G6: String is owned; a borrowed parameter is a reference
        "string" => {
            if param { "&::std::string::String".to_string() } else { "::std::string::String".to_string() }
        }
        other => other.to_string(),
    }
}

/// A Type printed with Python's `%r`.
fn type_repr(t: &str) -> String {
    format!("Type(name='{}')", t)
}

/// Contract mirror of a structured Option or Result.
pub fn try_cty_g(g: &GType) -> Result<String, String> {
    if let Some((e, n)) = g.fixed_parts() {
        // P7-1 B: a `[T; N]` struct field mirrors to `[i128; N]` (`[bool; N]`)
        return Ok(format!("[{}; {}]", try_cty_g(e)?, n));
    }
    if let Some(base) = g.view_base() {
        return Ok(rty_g(base, true));
    }
    if structured_builtin(g) {
        // LB8: the Ok payload of Result<(), E> mirrors to ()
        let x = if g.name == "result" && g.args[0].name == "unit" && g.args[0].owner.is_none() {
            "()".to_string()
        } else {
            try_cty_g(&g.args[0])?
        };
        if g.name == "option" {
            return Ok(format!("::std::option::Option<{}>", x));
        }
        let e = g.result_error().map(try_cty_g).transpose()?.unwrap_or_else(|| "i128".to_string());
        return Ok(format!("::std::result::Result<{}, {}>", x, e));
    }
    try_cty_name(&g.name)
}

/// Contract (i128) mirror type. Err for unit and array.
pub fn try_cty(t: &str) -> Result<String, String> {
    if is_g3_key(t) {
        if let Ok(g) = GType::try_from_key(t) {
            return try_cty_g(&g);
        }
    }
    try_cty_name(t)
}

fn try_cty_name(t: &str) -> Result<String, String> {
    if let Some(comps) = crate::gtype::tuple_components(t) {
        let cs: Result<Vec<String>, String> = comps.iter().map(|k| try_cty(k)).collect();
        return Ok(format!("({})", cs?.join(", ")));
    }
    match t {
        _ if crate::intwidth::of(t).is_some() => Ok("i128".to_string()),
        "bool" => Ok("bool".to_string()),
        "option" => Ok("::std::option::Option<i128>".to_string()),
        "result" => Ok("::std::result::Result<i128, i128>".to_string()),
        "string" => Ok("::std::string::String".to_string()),
        _ if is_user_type(t) => Ok(format!("pruv_contract__{}", t)),
        _ => Err(format!("codegen: type {} has no contract representation", type_repr(t))),
    }
}

/// Normal path only (panics for a type with no contract representation; use try_cty to receive the failure).
pub fn cty(t: &str) -> String {
    try_cty(t).unwrap()
}

/// Contract conversion of a structured type (G3): recursive, the payload first.
pub fn try_conv_g(g: &GType, x: &str) -> Result<String, String> {
    if let Some((e, _)) = g.fixed_parts() {
        let inner = try_conv_g(e, "pruv_e")?;
        return Ok(if inner == "pruv_e" { x.to_string() } else { format!("{}.map(|pruv_e| {})", x, inner) });
    }
    if g.view_base().is_some() {
        return Ok(x.to_string());
    }
    if structured_builtin(g) {
        let inner = if g.name == "result" && g.args[0].name == "unit" && g.args[0].owner.is_none() {
            "v".to_string()
        } else {
            try_conv_g(&g.args[0], "v")?
        };
        let value = if g.has_string_error() { format!("{}.clone()", x) } else { x.to_string() };
        let mapped = if inner == "v" { value } else { format!("{}.map(|v| {})", value, inner) };
        if g.name == "option" {
            return Ok(mapped);
        }
        let err = match g.result_error() {
            Some(e) => try_conv_g(e, "v")?,
            None => "v as i128".to_string(),
        };
        if err == "v" {
            return Ok(mapped);
        }
        return Ok(format!("{}.map_err(|v| {})", mapped, err));
    }
    try_conv_name(&g.name, x)
}

/// Rust expression converting the runtime value `x` (of type t) to a contract value. Err for unit and array.
pub fn try_conv(t: &str, x: &str) -> Result<String, String> {
    if is_g3_key(t) {
        if let Ok(g) = GType::try_from_key(t) {
            return try_conv_g(&g, x);
        }
    }
    try_conv_name(t, x)
}

fn try_conv_name(t: &str, x: &str) -> Result<String, String> {
    match t {
        _ if crate::intwidth::of(t).is_some() => Ok(format!("({} as i128)", x)),
        "bool" => Ok(x.to_string()),
        "option" => Ok(format!("{}.map(|v| v as i128)", x)),
        "result" => Ok(format!("{}.map(|v| v as i128).map_err(|v| v as i128)", x)),
        "string" => Ok(x.to_string()),
        _ if is_user_type(t) => Ok(format!("pruv_to_c__{}({})", t, x)),
        _ => Err(format!("codegen: type {} has no contract representation", type_repr(t))),
    }
}

/// Normal path only (panics on failure; use try_conv to receive the failure).
pub fn conv(t: &str, x: &str) -> String {
    try_conv(t, x).unwrap()
}

/// G9 A2 (Python `condensed_type_order`): indices into `tt.decls` in dependency-first order over the SCC condensation of
/// the field-dependency graph. Iterative Tarjan: roots in declaration order, successors in field order (first
/// occurrence), a component is emitted when its root finishes, the members of one component in declaration order.
pub fn condensed_type_order(tt: &TypeTable) -> Vec<usize> {
    let count = tt.decls.len();
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, d) in tt.decls.iter().enumerate() {
        index.insert(d.name(), i);
    }
    let mut adj: Vec<Vec<usize>> = Vec::with_capacity(count);
    for d in &tt.decls {
        let mut out: Vec<usize> = Vec::new();
        for f in d.all_fields() {
            for m in crate::gtype::dependencies(&GType::from_key(&f.ty)) {
                if let Some(&j) = index.get(m.as_str()) {
                    if !out.contains(&j) {
                        out.push(j);
                    }
                }
            }
        }
        adj.push(out);
    }
    let mut num: Vec<i64> = vec![-1; count];
    let mut low: Vec<i64> = vec![0; count];
    let mut onstack: Vec<bool> = vec![false; count];
    let mut stack: Vec<usize> = Vec::new();
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut counter: i64 = 0;
    for root in 0..count {
        if num[root] != -1 {
            continue;
        }
        num[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        onstack[root] = true;
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((v, pos)) = work.pop() {
            if pos < adj[v].len() {
                work.push((v, pos + 1));
                let w = adj[v][pos];
                if num[w] == -1 {
                    num[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    onstack[w] = true;
                    work.push((w, 0));
                } else if onstack[w] {
                    low[v] = low[v].min(num[w]);
                }
            } else {
                if low[v] == num[v] {
                    let mut comp: Vec<usize> = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        onstack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comps.push(comp);
                }
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
            }
        }
    }
    let mut out: Vec<usize> = Vec::with_capacity(count);
    for mut comp in comps {
        comp.sort();
        out.extend(comp);
    }
    out
}

/// G9 A2 (Python `make_type_table`): the type table of a program; a G9 program gets the condensed dependency order.
pub fn make_type_table(prog: &Program) -> TypeTable {
    let mut tt = TypeTable::new(&prog.types);
    if prog.g9.is_some() {
        tt.order = condensed_type_order(&tt);
    }
    tt
}

/// A `pub struct`/`pub enum` per user type, dependencies first (TypeTable.order).
pub fn emit_public_types(prog: &Program) -> String {
    emit_public_types_with(prog, &make_type_table(prog))
}

/// `emit_public_types` over a given table (Python `emit_public_types(prog, order)`): G9: an extended type derives Clone,
/// never Copy; a boxed field (recursive edge) is `::std::boxed::Box<Child>`.
pub fn emit_public_types_with(prog: &Program, tt: &TypeTable) -> String {
    let (ext, boxed): (std::collections::HashSet<&str>, std::collections::HashSet<&str>) = match &prog.g9 {
        Some(m) => (m.extended.iter().map(|s| s.as_str()).collect(), m.boxed.iter().map(|s| s.as_str()).collect()),
        None => (Default::default(), Default::default()),
    };
    let fty = |ty: &str, label: &str| -> String {
        let r = rty(ty, false);
        if boxed.contains(label) { format!("::std::boxed::Box<{}>", r) } else { r }
    };
    // G4 A2: in explicit mode only a source-`pub` type is a `pub` Rust item
    let explicit = prog.explicit.as_ref();
    let pubs: std::collections::HashMap<&str, bool> = match explicit {
        Some(x) => prog.types.iter().zip(x.type_pub.iter()).map(|(d, p)| (d.name(), *p)).collect(),
        None => Default::default(),
    };
    let mut out: Vec<String> = Vec::new();
    for d in tt.ordered() {
        if crate::gtype::is_tuple_name(d.name()) {
            continue; // P6: a tuple is a native Rust tuple, no declaration
        }
        let pv = if explicit.is_none() || pubs.get(d.name()).copied().unwrap_or(true) { "pub " } else { "" };
        out.push(format!(
            "#[allow(dead_code, non_camel_case_types, non_snake_case{})]",
            if explicit.is_some() { ", private_interfaces" } else { "" }
        ));
        if ext.contains(d.name()) {
            out.push("#[derive(Clone, Debug)]".to_string());
        } else {
            out.push("#[derive(Clone, Copy, Debug, PartialEq, Eq)]".to_string());
        }
        match d {
            TypeDecl::Struct(s) => {
                if !s.fields.is_empty() {
                    let body: Vec<String> = s
                        .fields
                        .iter()
                        .map(|f| {
                            let ty = if f.borrowed {
                                format!("&'a {}", &rty(&f.ty, true)[1..])
                            } else if tt.decl(&f.ty).is_some_and(crate::borrowed::borrowing_struct) {
                                format!("{}<'a>", f.ty)
                            } else {
                                fty(&f.ty, &format!("{}.{}", s.name, f.name))
                            };
                            format!("{}{}: {}", pv, f.name, ty)
                        })
                        .collect();
                    let lifetime = if crate::borrowed::borrowing_struct(d) { "<'a>" } else { "" };
                    out.push(format!("{}struct {}{} {{ {} }}", pv, s.name, lifetime, body.join(", ")));
                } else {
                    out.push(format!("{}struct {} {{}}", pv, s.name));
                }
            }
            TypeDecl::Enum(e) => {
                let mut vs: Vec<String> = Vec::new();
                for v in &e.variants {
                    if !v.fields.is_empty() {
                        let fs: Vec<String> = v
                            .fields
                            .iter()
                            .map(|f| fty(&f.ty, &format!("{}::{}.{}", e.name, v.name, f.name)))
                            .collect();
                        vs.push(format!("{}({})", v.name, fs.join(", ")));
                    } else {
                        vs.push(v.name.clone());
                    }
                }
                out.push(format!("{}enum {} {{ {} }}", pv, e.name, vs.join(", ")));
            }
        }
        out.push(String::new());
    }
    let mut s = out.join("\n");
    if !out.is_empty() {
        s.push('\n');
    }
    s
}

fn show_list(xs: &[Expr]) -> String {
    xs.iter().map(show).collect::<Vec<_>>().join(", ")
}

/// Human-readable text of a contract expression (for panic messages).
pub fn show(e: &Expr) -> String {
    match e {
        Expr::Spent(_) => "spent".to_string(),
        Expr::IntLit(x) => x.value.clone(),
        Expr::BoolLit(x) => (if x.value { "true" } else { "false" }).to_string(),
        Expr::StrLit(_) => "<str>".to_string(),
        Expr::Var(x) => x.name.clone(),
        Expr::Move(x) => {
            let place = std::iter::once(x.name.as_str()).chain(x.fields.iter().map(String::as_str)).collect::<Vec<_>>();
            format!("move {}", place.join("."))
        }
        Expr::InoutArg(x) => format!("inout {}", show(&x.expr)), // G5 A0: never reaches a generator (guarded in the driver)
        Expr::Unary(x) => format!("{}{}", x.op, show(&x.operand)),
        Expr::Binary(x) => format!("({} {} {})", show(&x.left), x.op, show(&x.right)),
        Expr::Index(x) => format!("{}[{}]", show(&x.array), show(&x.index)),
        Expr::Call(x) => format!("{}({})", x.name, show_list(&x.args)),
        Expr::Ctor(x) => match &x.arg {
            None => x.name.clone(),
            Some(a) => format!("{}({})", x.name, show(a)),
        },
        Expr::Quant(x) => format!("{} {} in {}..{}: {}", x.kind, x.var, show(&x.lo), show(&x.hi), show(&x.body)),
        // G3: a specialized node is named by its stem; the message shows the declaration name as written
        Expr::StructLit(x) => {
            if x.fields.is_empty() {
                format!("{} {{}}", src_name(&x.name))
            } else {
                let fs: Vec<String> = x.fields.iter().map(|f| format!("{}: {}", f.name, show(&f.value))).collect();
                format!("{} {{ {} }}", src_name(&x.name), fs.join(", "))
            }
        }
        Expr::EnumCtor(x) => {
            if x.args.is_empty() {
                format!("{}::{}", src_name(&x.enum_name), x.variant)
            } else {
                format!("{}::{}({})", src_name(&x.enum_name), x.variant, show_list(&x.args))
            }
        }
        Expr::FieldAccess(x) => format!("{}.{}", show(&x.base), x.field),
        Expr::IsVariant(x) => format!("is_variant({}, {}::{})", show(&x.value), src_name(&x.enum_name), x.variant),
        Expr::VariantField(x) => {
            format!("variant_field({}, {}::{}, {})", show(&x.value), src_name(&x.enum_name), x.variant, x.field)
        }
    }
}

/// The name of a if this is a `len(a)` call.
fn len_of(x: &Expr) -> Option<&str> {
    if let Expr::Call(c) = x {
        if c.name == "len" && c.args.len() == 1 {
            if let Expr::Var(v) = &c.args[0] {
                return Some(&v.name);
            }
        }
    }
    None
}

/// The array name a if e is structurally `forall i in 0..len(a): forall j in i..len(a): a[i] <= a[j]`
/// (inner start `i` or `i + 1`), otherwise None.
pub fn match_sorted_requires(e: &Expr) -> Option<String> {
    let q1 = match e {
        Expr::Quant(q) if q.kind == "forall" => q,
        _ => return None,
    };
    match &*q1.lo {
        Expr::IntLit(l) if l.value == "0" => {}
        _ => return None,
    }
    let a = len_of(&q1.hi)?;
    if a.is_empty() {
        return None;
    }
    let q2 = match &*q1.body {
        Expr::Quant(q) if q.kind == "forall" => q,
        _ => return None,
    };
    if q2.var == q1.var || a == q1.var || a == q2.var {
        return None;
    }
    let mut lo: &Expr = &q2.lo;
    if let Expr::Binary(b) = lo {
        if b.op == "+" {
            if let Expr::IntLit(r) = &*b.right {
                if r.value == "1" {
                    lo = &*b.left;
                }
            }
        }
    }
    match lo {
        Expr::Var(v) if v.name == q1.var => {}
        _ => return None,
    }
    if len_of(&q2.hi) != Some(a) {
        return None;
    }
    let b = match &*q2.body {
        Expr::Binary(b) if b.op == "<=" => b,
        _ => return None,
    };
    let (l, r) = match (&*b.left, &*b.right) {
        (Expr::Index(l), Expr::Index(r)) => (l, r),
        _ => return None,
    };
    for (ix, v) in [(l, &q1.var), (r, &q2.var)] {
        let ok = matches!(&*ix.array, Expr::Var(x) if x.name == a)
            && matches!(&*ix.index, Expr::Var(x) if &x.name == v);
        if !ok {
            return None;
        }
    }
    Some(a.to_string())
}

/// A Rust string literal using ASCII only.
pub fn rstr(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        let c = ch as u32;
        if ch == '\\' || ch == '"' {
            out.push('\\');
            out.push(ch);
        } else if (32..127).contains(&c) {
            out.push(ch);
        } else {
            out.push_str(&format!("\\u{{{:x}}}", c));
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(n: &str) -> Expr {
        Expr::Var(Var { line: 1, col: 1, name: n.into(), ty: None })
    }
    fn int(v: &str) -> Expr {
        Expr::IntLit(IntLit { line: 1, col: 1, value: v.into(), ty: None, kind: Default::default() })
    }
    fn bin(op: &str, l: Expr, r: Expr) -> Expr {
        Expr::Binary(Binary { line: 1, col: 1, op: op.into(), left: Box::new(l), right: Box::new(r), ty: None })
    }
    fn len(n: &str) -> Expr {
        Expr::Call(Call { line: 1, col: 1, name: "len".into(), args: vec![var(n)], ty: None, targs: None, inst: None, old: None, opaque_c: false })
    }
    fn idx(a: &str, i: &str) -> Expr {
        Expr::Index(Index { line: 1, col: 1, array: Box::new(var(a)), index: Box::new(var(i)), ty: None })
    }
    fn forall(v: &str, lo: Expr, hi: Expr, body: Expr) -> Expr {
        Expr::Quant(Quant {
            line: 1, col: 1, kind: "forall".into(), var: v.into(),
            lo: Box::new(lo), hi: Box::new(hi), body: Box::new(body), ty: None, triggers: Vec::new(),
        })
    }
    fn sorted(inner_lo: Expr) -> Expr {
        forall("i", int("0"), len("a"), forall("j", inner_lo, len("a"), bin("<=", idx("a", "i"), idx("a", "j"))))
    }

    #[test]
    fn show_basic() {
        let e = bin("+", var("x"), Expr::BoolLit(BoolLit { line: 1, col: 1, value: true, ty: None }));
        assert_eq!(show(&e), "(x + true)");
        assert_eq!(show(&sorted(var("i"))), "forall i in 0..len(a): forall j in i..len(a): (a[i] <= a[j])");
        let c = Expr::Call(Call { line: 1, col: 1, name: "f".into(), args: vec![int("-3"), var("y")], ty: None, targs: None, inst: None, old: None, opaque_c: false });
        assert_eq!(show(&c), "f(-3, y)");
    }

    #[test]
    fn sorted_match() {
        assert_eq!(match_sorted_requires(&sorted(var("i"))), Some("a".to_string()));
        assert_eq!(match_sorted_requires(&sorted(bin("+", var("i"), int("1")))), Some("a".to_string()));
        assert_eq!(match_sorted_requires(&sorted(bin("+", var("i"), int("2")))), None);
        assert_eq!(match_sorted_requires(&var("a")), None);
    }

    #[test]
    fn rstr_escapes() {
        assert_eq!(rstr("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(rstr("é\n"), "\"\\u{e9}\\u{a}\"");
        assert_eq!(rstr("~\u{7f}"), "\"~\\u{7f}\"");
    }

    #[test]
    fn types() {
        assert_eq!(rty("array", true), "&[i64]");
        assert_eq!(rty("array", false), "::std::vec::Vec<i64>");
        assert_eq!(rty("array_u8", true), "&[u8]");
        assert_eq!(rty("array_u8", false), "::std::vec::Vec<u8>");
        assert_eq!(rty("array_u16", true), "&[u16]");
        assert_eq!(rty("array_u16", false), "::std::vec::Vec<u16>");
        assert_eq!(cty("Point"), "pruv_contract__Point");
        assert_eq!(conv("option", "x"), "x.map(|v| v as i128)");
        assert!(try_cty("unit").is_err());
    }
}

/// Share the existing escape analysis for fixed u16 buffers. Its byte-buffer
/// model gets twice the element count so the same stack limits apply.
#[derive(Default)]
pub struct BufferStackPlan {
    pub base: crate::stack::StackPlan,
    u16_lengths: std::collections::HashMap<usize, usize>,
}

impl BufferStackPlan {
    pub fn new(prog: &Program, storage: &crate::storage::StoragePlan, verified: bool) -> Self {
        let base = crate::stack::plan_stack(prog, storage, verified);
        let mut u16_lengths = std::collections::HashMap::new();
        if crate::typeinfo::uses_array_type(prog, "array_u16") {
            fn project(stmts: &mut [Stmt], lets: &mut Vec<usize>, ordinal: &mut usize) {
                for stmt in stmts {
                    let number = *ordinal;
                    *ordinal += 1;
                    match stmt {
                        Stmt::Let(l) => {
                            lets.push(number);
                            if l.ty == "array_u16" && !l.borrowed {
                                if let Expr::Call(c) = &mut l.value {
                                    if let Some(Expr::IntLit(n)) = c.args.first_mut().filter(|_| c.name == "alloc") {
                                        if let Some(bytes) = n.value.parse::<usize>().ok().and_then(|n| n.checked_mul(2)) {
                                            n.value = bytes.to_string();
                                            l.ty = "array_u8".into();
                                            continue;
                                        }
                                    }
                                }
                            }
                            l.ty.clear();
                        }
                        Stmt::If(s) => {
                            project(&mut s.then, lets, ordinal);
                            project(&mut s.els, lets, ordinal);
                        }
                        Stmt::While(s) => project(&mut s.body, lets, ordinal),
                        Stmt::ForMap(s) => project(&mut s.body, lets, ordinal),
                        Stmt::Match(s) => {
                            for arm in &mut s.arms { project(&mut arm.body, lets, ordinal); }
                        }
                        _ => {}
                    }
                }
            }

            let mut projected = prog.clone();
            let mut original = std::collections::HashMap::new();
            for (source, proxy) in prog.functions.iter().zip(&mut projected.functions) {
                if source.foreign.is_some() { continue; }
                for p in &mut proxy.params {
                    if p.ty == "array_u16" { p.ty = "array_u8".into(); }
                }
                let mut lets = Vec::new();
                let mut ordinal = 0;
                project(&mut proxy.body, &mut lets, &mut ordinal);
                let mut ordinals = lets.into_iter();
                crate::foreign_shim::walk_stmts(&source.body, &mut |node| {
                    if let crate::foreign_shim::Node::Let(l) = node {
                        original.insert((source.name.as_str(), ordinals.next().unwrap()), l as *const Let as usize);
                    }
                });
            }
            let mut used = std::collections::HashMap::new();
            for (f, _, _, len, size) in &base.entries {
                *used.entry(f.as_str()).or_insert(0) += len * size;
            }
            let extra = crate::stack::plan_stack(&projected, &Default::default(), verified);
            for (f, ordinal, _, bytes, _) in &extra.entries {
                let total = used.entry(f.as_str()).or_insert(0);
                if *total + bytes <= crate::stack::MAX_TOTAL_BYTES {
                    *total += bytes;
                    u16_lengths.insert(original[&(f.as_str(), *ordinal)], bytes / 2);
                }
            }
        }
        Self { base, u16_lengths }
    }

    pub fn contains(&self, node: &Let) -> bool {
        self.base.contains(node) || self.u16_lengths.contains_key(&(node as *const Let as usize))
    }

    pub fn length(&self, node: &Let) -> usize {
        self.u16_lengths.get(&(node as *const Let as usize)).copied().unwrap_or_else(|| self.base.length(node))
    }
}

/// P3 vectorization (codegen.py fast_forall): the array reads (name, offset), first occurrence order without repeats,
/// when the forall e can be prefiltered by a branch-free loop, else None. The body may use only comparisons of integer
/// atoms joined by !, &&, || and ==>; an atom is an integer literal, the quantified variable, a plain i64 variable,
/// len(a) or a[k + c] (k the quantified variable, c a literal of at most 2^31). arr_ok says whether the backend can
/// read the array variable directly.
pub fn fast_forall(e: &Quant, arr_ok: &dyn Fn(&Expr) -> bool) -> Option<Vec<(String, i128)>> {
    if e.kind != "forall" {
        return None;
    }
    fn arr(x: &Expr, q: &Quant, arr_ok: &dyn Fn(&Expr) -> bool) -> bool {
        match x {
            Expr::Var(v) => {
                matches!(v.ty.as_deref(), Some("array") | Some("array_u8") | Some("array_u16")) && v.name != q.var && arr_ok(x)
            }
            _ => false,
        }
    }
    fn atom(x: &Expr, q: &Quant, arr_ok: &dyn Fn(&Expr) -> bool, reads: &mut Vec<(String, i128)>) -> bool {
        match x {
            Expr::IntLit(_) => true,
            Expr::Var(v) => v.name == q.var || v.ty.as_deref() == Some("i64"),
            Expr::Call(c) => c.name == "len" && c.args.len() == 1 && arr(&c.args[0], q, arr_ok),
            Expr::Index(ix) => {
                if !arr(&ix.array, q, arr_ok) {
                    return false;
                }
                let c: i128 = match &*ix.index {
                    Expr::Var(v) if v.name == q.var => 0,
                    Expr::Binary(b) if b.op == "+" || b.op == "-" => {
                        let lv = matches!(&*b.left, Expr::Var(v) if v.name == q.var);
                        let rv = match &*b.right {
                            Expr::IntLit(l) => l.value.parse::<i128>().ok().filter(|n| *n >= 0 && *n <= 1i128 << 31),
                            _ => None,
                        };
                        match (lv, rv) {
                            (true, Some(n)) => if b.op == "+" { n } else { -n },
                            _ => return false,
                        }
                    }
                    _ => return false,
                };
                let name = match &*ix.array { Expr::Var(v) => v.name.clone(), _ => unreachable!() };
                if !reads.iter().any(|(a, o)| *a == name && *o == c) {
                    reads.push((name, c));
                }
                true
            }
            _ => false,
        }
    }
    fn pred(x: &Expr, q: &Quant, arr_ok: &dyn Fn(&Expr) -> bool, reads: &mut Vec<(String, i128)>) -> bool {
        match x {
            Expr::BoolLit(_) => true,
            Expr::Unary(u) => u.op == "!" && pred(&u.operand, q, arr_ok, reads),
            Expr::Binary(b) => {
                if b.op == "&&" || b.op == "||" || b.op == "==>" {
                    pred(&b.left, q, arr_ok, reads) && pred(&b.right, q, arr_ok, reads)
                } else if ["==", "!=", "<", "<=", ">", ">="].contains(&b.op.as_str()) {
                    atom(&b.left, q, arr_ok, reads) && atom(&b.right, q, arr_ok, reads)
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    let mut reads = Vec::new();
    if pred(&e.body, e, arr_ok, &mut reads) { Some(reads) } else { None }
}

/// P3-F1 (codegen.py FusePlan): `start` = index of the first requires of the trailing run, `arrs` = the arrays of that run
/// in order, `n_arr` = X (the array whose length is the result length), `elem` = the map element expression E,
/// `idx` = the loop variable name.
#[derive(Debug, Clone)]
pub struct FusePlan {
    pub start: usize,
    pub arrs: Vec<String>,
    pub n_arr: String,
    pub elem: Expr,
    pub idx: String,
}

/// P3-F1 (codegen.py fuse_map, docs/P3_FUSE_DESIGN.md decision 1): the plan of a fused requires check for an element-wise
/// map, or None. Purely syntactic. f returns [i64], has requires, no cost clause, no inout parameter, no refinement; its
/// maximal trailing run of requires f.requires[start..] are all `forall t in 0..len(x): P` over distinct [i64] parameters x
/// that fast_forall accepts reading exactly x[t]; its body is exactly
///     let c: [i64] = alloc(len(X), v); let i: i64 = 0;
///     while i < len(X) { c[i] = E; i = i + 1; }  return c;
/// with X one of those arrays and E built from + - *, integer literals and x[i] for x among them.
pub fn fuse_map(f: &Function) -> Option<FusePlan> {
    if f.requires.is_empty() || f.cost.is_some() || f.ret != "array" || f.ret_refine.is_some() {
        return None;
    }
    if f.params.iter().any(|p| p.mode_pos.is_some() || p.refine.is_some()) {
        return None;
    }
    let arrp: Vec<&str> = f.params.iter().filter(|p| p.ty == "array").map(|p| p.name.as_str()).collect();
    let mut start = f.requires.len();
    let mut arrs: Vec<String> = Vec::new();
    for r in f.requires.iter().rev() {
        let q = match r {
            Expr::Quant(q) => q,
            _ => break,
        };
        if q.kind != "forall" {
            break;
        }
        match &*q.lo {
            Expr::IntLit(l) if l.value == "0" => {}
            _ => break,
        }
        let x = match &*q.hi {
            Expr::Call(c) if c.name == "len" && c.args.len() == 1 => match &c.args[0] {
                Expr::Var(v) => v.name.clone(),
                _ => break,
            },
            _ => break,
        };
        let ok = |a: &Expr| matches!(a, Expr::Var(v) if arrp.contains(&v.name.as_str()));
        if !arrp.contains(&x.as_str()) || fast_forall(q, &ok) != Some(vec![(x.clone(), 0)]) {
            break;
        }
        start -= 1;
        arrs.insert(0, x);
    }
    if start == f.requires.len() {
        return None;
    }
    for (k, a) in arrs.iter().enumerate() {
        if arrs[..k].contains(a) {
            return None;
        }
    }
    if f.body.len() != 4 {
        return None;
    }
    let (s1, s2, s3, s4) = (&f.body[0], &f.body[1], &f.body[2], &f.body[3]);
    let s1 = match s1 {
        Stmt::Let(l) => l,
        _ => return None,
    };
    if !(s1.ty == "array" && s1.refine.is_none()) {
        return None;
    }
    let n = match &s1.value {
        Expr::Call(c) if c.name == "alloc" && c.args.len() == 2 && matches!(c.args[1], Expr::IntLit(_)) => &c.args[0],
        _ => return None,
    };
    let c = s1.name.as_str();
    let xn = match n {
        Expr::Call(l) if l.name == "len" && l.args.len() == 1 => match &l.args[0] {
            Expr::Var(v) => v.name.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let s2 = match s2 {
        Stmt::Let(l) if l.ty == "i64" && l.refine.is_none() && matches!(&l.value, Expr::IntLit(v) if v.value == "0") => l,
        _ => return None,
    };
    let i = s2.name.as_str();
    if !arrs.contains(&xn)
        || i == c
        || arrp.contains(&c)
        || arrp.contains(&i)
        || f.params.iter().any(|p| p.name == c)
        || f.params.iter().any(|p| p.name == i)
    {
        return None;
    }
    let w = match s3 {
        Stmt::While(w) => w,
        _ => return None,
    };
    let cond_ok = match &w.cond {
        Expr::Binary(b) if b.op == "<" => {
            matches!(&*b.left, Expr::Var(v) if v.name == i)
                && match &*b.right {
                    Expr::Call(l) if l.name == "len" && l.args.len() == 1 => {
                        matches!(&l.args[0], Expr::Var(v) if v.name == xn)
                    }
                    _ => false,
                }
        }
        _ => false,
    };
    if !cond_ok || w.body.len() != 2 {
        return None;
    }
    let st = match &w.body[0] {
        Stmt::IndexAssign(st)
            if st.name == c && matches!(&st.index, Expr::Var(v) if v.name == i) && st.elem_refine.is_none() =>
        {
            st
        }
        _ => return None,
    };
    let inc_ok = match &w.body[1] {
        Stmt::Assign(a) if a.name == i => match &a.value {
            Expr::Binary(b) if b.op == "+" => {
                matches!(&*b.left, Expr::Var(v) if v.name == i) && matches!(&*b.right, Expr::IntLit(l) if l.value == "1")
            }
            _ => false,
        },
        _ => false,
    };
    if !inc_ok {
        return None;
    }
    match s4 {
        Stmt::Return(r) if matches!(&r.value, Some(Expr::Var(v)) if v.name == c) => {}
        _ => return None,
    }
    fn elem_ok(e: &Expr, arrs: &[String], i: &str) -> bool {
        match e {
            Expr::IntLit(_) => true,
            Expr::Binary(b) => {
                (b.op == "+" || b.op == "-" || b.op == "*") && elem_ok(&b.left, arrs, i) && elem_ok(&b.right, arrs, i)
            }
            Expr::Index(ix) => {
                matches!(&*ix.array, Expr::Var(v) if arrs.contains(&v.name))
                    && matches!(&*ix.index, Expr::Var(v) if v.name == i)
            }
            _ => false,
        }
    }
    if !elem_ok(&st.value, &arrs, i) {
        return None;
    }
    Some(FusePlan { start, arrs, n_arr: xn, elem: st.value.clone(), idx: i.to_string() })
}

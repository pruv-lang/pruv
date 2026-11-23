// Hand Rust twin of value_ops.pruv, written in the hand_life.rs style (get/put use rfind, put edits in place).
// Build: rustc -O --edition 2021 value_ops_hand.rs; the output must equal the compiled value_ops.pruv.
type Text = Vec<u16>;

#[allow(dead_code)]
enum Value {
    Undefined,
    Null,
    Bool(bool),
    Integer(i64),
    Str(Text),
    Array(Vec<Value>),
    Object(Vec<(Text, Value)>),
}

fn text(s: &str) -> Text { s.encode_utf16().collect() }

fn put(fields: &mut Vec<(Text, Value)>, key: &str, value: i64) {
    let key = text(key);
    if let Some((_, field)) = fields.iter_mut().rfind(|(name, _)| *name == key) {
        *field = Value::Integer(value);
    } else {
        fields.push((key, Value::Integer(value)));
    }
}

fn get_int(fields: &[(Text, Value)], key: &str) -> i64 {
    let key = text(key);
    match fields.iter().rfind(|(name, _)| *name == key) {
        None => -1,
        Some((_, Value::Integer(n))) => *n,
        Some(_) => 0,
    }
}

fn nodes(v: &Value) -> i64 {
    match v {
        Value::Array(items) => items.iter().fold(1, |t, c| (t + nodes(c)) % 1_000_000),
        Value::Object(fields) => fields.iter().fold(1, |t, (_, c)| (t + nodes(c)) % 1_000_000),
        _ => 1,
    }
}

fn sum(v: &Value) -> i64 {
    match v {
        Value::Undefined | Value::Null => 0,
        Value::Bool(b) => *b as i64,
        Value::Integer(n) => if *n < 0 { 0 } else { n % 1_000_000 },
        Value::Str(u) => u.len() as i64 % 1_000_000,
        Value::Array(items) => items.iter().fold(0, |t, c| (t + sum(c)) % 1_000_000),
        Value::Object(fields) => fields.iter().fold(0, |t, (_, c)| (t + sum(c)) % 1_000_000),
    }
}

fn main() {
    let mut fields: Vec<(Text, Value)> = Vec::new();
    put(&mut fields, "ti", 5);
    put(&mut fields, "de", 2);
    fields.push((text("ti"), Value::Integer(7)));
    println!("{}", fields.len());
    println!("{}", get_int(&fields, "ti"));
    put(&mut fields, "ti", 9);
    println!("{}", get_int(&fields, "ti"));
    println!("{}", get_int(&fields, "de"));
    println!("{}", get_int(&fields, "ch"));
    println!("{}", fields.len());
    println!("{}", if let Value::Integer(n) = fields[0].1 { n } else { -2 });
    let mut items = vec![Value::Null, Value::Str(text("hi")), Value::Bool(true)];
    items[0] = Value::Integer(40);
    fields.push((text("ch"), Value::Array(items)));
    let root = Value::Object(fields);
    println!("{}", nodes(&root));
    println!("{}", sum(&root));
}

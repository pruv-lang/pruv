// spec: R-6g-3 accept
fn main() {
    let mut st = sol::St {
        recs: std::collections::HashMap::from([(String::from("key"), sol::Rec { n: 7 })]),
        out: sol::Out { fx: Vec::new() },
    };
    for _ in 0..2 {
        let len = sol::step(&mut st, "key");
        println!("{} {}", len, st.recs["key"].n);
    }
}

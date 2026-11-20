// spec: R-6e-13 accept
fn main() {
    let mut st = sol::State {
        values: std::collections::HashMap::from([
            (String::from("oak"), sol::Value { n: 2 }),
            (String::from("pine"), sol::Value { n: 5 }),
            (String::from("ash"), sol::Value { n: 7 }),
        ]),
    };
    let totals = sol::totals(&mut st);
    println!("{}", totals.sum);
    println!("{}", totals.keys);
}

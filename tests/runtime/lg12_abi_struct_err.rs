fn main() {
    match sol::parse(55, 3) {
        Ok(v) => println!("ok {}", v),
        Err(e) => println!("err {} {}", e.pos, e.code),
    }
    match sol::parse(65, 4) {
        Ok(v) => println!("ok {}", v),
        Err(e) => println!("err {} {}", e.pos, e.code),
    }
    println!("{}", sol::code_of(Ok(9)));
    println!("{}", sol::code_of(Err(sol::ParseErr { pos: 1, code: 70 })));
    match sol::flip(Ok(true)) {
        Ok(b) => println!("ok {}", b),
        Err(e) => println!("err {} {}", e.pos, e.code),
    }
    match sol::flip(Err(sol::ParseErr { pos: 6, code: 2 })) {
        Ok(b) => println!("ok {}", b),
        Err(e) => println!("err {} {}", e.pos, e.code),
    }
}

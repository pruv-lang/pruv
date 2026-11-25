fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(46340), b(46340)));
    println!("{}", sol::f(b(46341), b(46341)));
}

fn main() {
    let leaf = sol::Node { data: vec![1, 2], children: vec![] };
    let mut root = sol::Node { data: vec![3], children: vec![leaf] };
    println!("children: {}", sol::arity(&mut root));
}

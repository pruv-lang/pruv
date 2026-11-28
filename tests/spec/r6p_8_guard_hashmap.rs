fn main() {
    let mut st = sol::State { items: Default::default() };
    st.items.insert(7, sol::Item { data: vec![1, 2] });
    println!("items: {}", sol::count(&mut st));
}

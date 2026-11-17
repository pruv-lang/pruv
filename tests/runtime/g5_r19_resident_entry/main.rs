// Host driver: allocates the buffers, calls the entries and prints; every decision (validation, status, growth request)
// is made by the Pruv code.
fn main() {
    let mut st: Vec<i64> = vec![-1; 4];
    sol::init(&mut st);
    println!("init count={} len={}", st[0], st.len());
    let mut out: Vec<i64> = vec![0; 2];
    let calls: [[i64; 2]; 7] = [[1, 10], [1, 20], [2, 5], [1, 5000], [1, 30], [1, 40], [1, 50]];
    for call in calls.iter() {
        let mut s = sol::step(&mut st, &mut out, call);
        if s == 2 {
            println!("step {:?} -> {} count={} len={}", call, s, st[0], st.len());
            let mut next: Vec<i64> = vec![0; st.len() * 2];
            sol::grow(&st, &mut next);
            st = next;
            println!("grow -> len {}", st.len());
            s = sol::step(&mut st, &mut out, call);
        }
        println!("step {:?} -> {} count={} out={:?}", call, s, st[0], out);
    }
    println!("state {:?}", st);
    let mut bytes: Vec<u8> = vec![0; 4];
    let p1 = sol::poke(&mut bytes, 2, 200);
    let p2 = sol::poke(&mut bytes, 9, 1);
    let p3 = sol::poke(&mut bytes, 1, 300);
    println!("poke {} {} {} {:?}", p1, p2, p3, bytes);
    let mut flags: Vec<bool> = vec![false; 3];
    let f1 = sol::set_flag(&mut flags, 1, true);
    let f2 = sol::set_flag(&mut flags, 3, true);
    println!("flag {} {} {:?}", f1, f2, flags);
}

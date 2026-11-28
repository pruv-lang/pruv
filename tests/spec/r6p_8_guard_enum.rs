fn main() {
    let mut st = sol::State { packet: sol::Packet::Empty };
    assert_eq!(sol::packet_len(&mut st), 0);
    st.packet = sol::Packet::Bytes(vec![10, 20, 30]);
    println!("packet bytes: {}", sol::packet_len(&mut st));
}

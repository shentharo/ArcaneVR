use arcane_bridge_core::encode_motion;
fn main() {
    // Deterministic cross-language fixture, no network or gameplay side effects.
    let packet = encode_motion(0.5, 0.0, true, true, 1234.5, 42).unwrap();
    for b in packet {
        print!("{:02x}", b);
    }
    println!();
}

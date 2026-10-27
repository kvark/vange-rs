//! The car physics gives the same bits on every platform: the server runs
//! it natively (Linux, also macOS and Windows for local play) and the web
//! client as wasm, and a client that drifts by one bit a quantum shows it
//! as server corrections within seconds, most of all in the air.
//!
//! A scripted tumble on the procedural test level (driving, turning,
//! jumps, rolls, braking) is hashed bit for bit and checked against the
//! record. CI runs it on Linux, macOS and Windows; `bin/determ` runs the
//! same tumble as wasm under node (see its `determ.mjs`) and prints the
//! record line by line, to compare against the native one.
//!
//! When a deliberate physics change moves the record, update the hash
//! (the test prints the new one).

#[path = "../bin/determ/trace.rs"]
#[allow(dead_code)]
mod trace;

/// FNV-1a of the terrain, then of every quantum's pose, of
/// `trace::tumble(test level, (100, 100), 600)`.
const LEVEL_HASH: u64 = 0xa1eb_e2ce_de8d_7339;
const TRACE_HASH: u64 = 0x4417_998e_d2e8_73c5;

#[test]
fn tumble_on_the_test_level_is_bit_exact() {
    let world = trace::World::test();
    let poses = trace::tumble(&world, (100, 100), 600);

    // The script has to reach the air for the record to mean much.
    let clearance = poses
        .iter()
        .map(|p| p[2] - world.level.get((p[0] as i32, p[1] as i32)).high())
        .fold(f32::MIN, f32::max);
    println!("most clearance {clearance:.1}");
    assert!(clearance > 6.0, "never airborne: {clearance}");

    let mut h = trace::Fnv::default();
    for p in &poses {
        h.f32s(p);
    }
    let level = world.level_hash();
    println!("level {level:#018x}, trace {:#018x}", h.0);
    assert_eq!(level, LEVEL_HASH, "the test level changed: {level:#018x}");
    assert_eq!(
        h.0,
        TRACE_HASH,
        "the tumble changed: {:#018x}; first poses {:?}",
        h.0,
        &poses[..3]
    );
}

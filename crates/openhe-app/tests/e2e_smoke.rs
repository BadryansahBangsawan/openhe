//! The harness itself: the app connects to the simulator and loads it.

mod support;
use support::*;

#[test]
fn app_connects_to_the_simulator_and_loads_all_keys() {
    let w = World::new();
    let snap = w.snapshot();
    assert_eq!(snap.keys.len(), 68);
    assert_eq!(snap.matrix.len(), 68);
    assert_eq!(snap.info.firmware, "App V1.1.3");
    println!("{:?}", w.labels());
}

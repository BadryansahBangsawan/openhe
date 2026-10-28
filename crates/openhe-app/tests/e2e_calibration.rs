//! Kalibrasi end to end: start, simulated presses, keys marked, Simpan,
//! and the stop that must always follow.

mod support;
use support::*;

fn start() -> World {
    let mut w = World::new();
    w.page("Kalibrasi");
    w.click("Mulai kalibrasi");
    w.until("calibration started", 10, |w| w.sim().calibrating);
    w
}

/// Presses every key to the bottom, then waits until the page counts all.
fn press_all(w: &mut World) {
    // Step 1 (2 s settle) must pass before presses count.
    w.until("settled", 10, |w| w.has_text("2. Tekan tiap tombol"));
    std::thread::sleep(std::time::Duration::from_millis(2100));
    {
        let mut s = w.sim();
        for k in sim::keys() {
            s.press(k, 3400);
        }
    }
    w.until("all keys measured", 10, |w| {
        w.has_text("Semua tombol sudah terukur.")
    });
    let mut s = w.sim();
    for k in sim::keys() {
        s.press(k, 0);
    }
}

#[test]
fn start_press_every_key_then_simpan_stops() {
    let mut w = start();
    assert_eq!(w.sim().calibration_starts, 1);
    press_all(&mut w);
    w.click("Simpan");
    w.until("stopped", 10, |w| !w.sim().calibrating);
    w.idle();
    assert_eq!(w.sim().calibration_stops, 1);
    assert!(
        w.has_text("Kalibrasi selesai dan disimpan di keyboard."),
        "{:?}",
        w.labels()
    );
    // The date is recorded in the data file.
    let data = openhe_app::storage::AppData::load_from(
        &openhe_app::storage::AppData::default_dir().unwrap(),
    )
    .unwrap();
    assert!(data.last_calibrated.is_some());
    // Writes are unlocked again.
    w.page("Performa");
    w.click("WASD");
    w.edit_value("2.000 mm", 0, "1.5");
    assert!(w.enabled("Terapkan ke keyboard"));
}

#[test]
fn leaving_the_page_mid_calibration_sends_stop() {
    let mut w = start();
    w.page("Remap");
    w.until("stopped", 10, |w| !w.sim().calibrating);
    assert_eq!(w.sim().calibration_stops, 1);
    w.idle();
    assert!(!w.worker.calibration_guard());
}

#[test]
fn batal_stops_too() {
    let mut w = start();
    w.click("Batal");
    w.until("stopped", 10, |w| !w.sim().calibrating);
    assert_eq!(w.sim().calibration_stops, 1);
}

#[test]
fn polling_reads_travel_only() {
    // The calibration values (SUB 6) are not shown anywhere; reading them
    // on every sample doubled the round trips of each poll.
    let mut w = start();
    w.frames(30);
    w.click("Batal");
    w.until("stopped", 10, |w| !w.sim().calibrating);
    let s = w.sim();
    let travel = s
        .log
        .iter()
        .filter(|(c, p)| *c == 0x12 && p[0] == 2)
        .count();
    let values = s
        .log
        .iter()
        .filter(|(c, p)| *c == 0x12 && p[0] == 6)
        .count();
    assert!(travel > 0, "no travel polls");
    assert_eq!(
        values, 0,
        "{values} calibration-value reads for {travel} travel reads"
    );
}

#[test]
fn unplugged_mid_calibration_is_stopped_on_reconnect() {
    let mut w = start();
    w.sim().unplug();
    w.frames(20);
    w.sim().replug();
    w.until("stopped after reconnect", 15, |w| {
        !w.sim().calibrating && w.worker.snapshot().is_some()
    });
    w.idle();
    assert!(w.sim().calibration_stops >= 1);
    assert!(!w.worker.calibration_guard());
}

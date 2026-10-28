//! Performa end to end: the real page on the simulated keyboard.

mod support;
use eframe::egui::accesskit::Role;
use support::sim::Reject;
use support::*;

const W: u8 = 0x1A;
const A: u8 = 0x04;
const S: u8 = 0x16;
const D: u8 = 0x07;
const WASD: [u8; 4] = [W, A, S, D];

const ACT: u8 = 4;
const RTP: u8 = 20;
const RTR: u8 = 21;
const DP: u8 = 22;
const DR: u8 = 23;
const AXIS: u8 = 25;

#[test]
fn wasd_rapid_trigger_is_written_and_shown() {
    let mut w = World::new();
    w.click("WASD");
    assert!(w.has_text("4 tombol dipilih"));
    w.click("Rapid Trigger");
    w.edit_value("2.000 mm", 0, "0.4");
    // RT press; release follows while "Samakan" is on.
    w.edit_value("0.020 mm", 0, "0.15");
    w.apply("Terapkan ke keyboard");
    {
        let s = w.sim();
        for k in WASD {
            assert_eq!(s.p().mode(k), 0x20, "0x{k:02X} touch RT");
            assert_eq!(s.p().key(k, ACT), 400, "0x{k:02X} actuation");
            assert_eq!(s.p().key(k, RTP), 150, "0x{k:02X} press");
            assert_eq!(s.p().key(k, RTR), 150, "0x{k:02X} release");
            assert_eq!(s.p().key(k, DP), 200);
        }
        // Nothing else was written.
        assert_eq!(s.p().key(0x14, ACT), 2000);
        assert_eq!(s.p().mode(0x14), 0);
    }
    // The form shows what the keyboard now holds, and says it is saved.
    assert!(w.has("Tersimpan di keyboard"), "{:?}", w.labels());
    assert!(w.has_text("=0.400 mm"), "{:?}", w.labels());
    assert!(w.has_text("=0.150 mm"), "{:?}", w.labels());
    // The keys show the new value and the RT badge (snapshot updated).
    let snap = w.snapshot();
    assert_eq!(snap.keys[&W].actuation.0, 400);
}

#[test]
fn separate_release_sensitivity() {
    let mut w = World::new();
    w.click("WASD");
    w.click("Rapid Trigger");
    // Turn "Samakan tekan & lepas" off: the only checkbox on the page.
    w.click_role(Role::CheckBox, 0);
    w.edit_value("0.020 mm", 0, "0.1");
    w.edit_value("0.020 mm", 0, "0.3");
    w.apply("Terapkan ke keyboard");
    let s = w.sim();
    for k in WASD {
        assert_eq!(s.p().key(k, RTP), 100);
        assert_eq!(s.p().key(k, RTR), 300);
    }
}

#[test]
fn deadzones_are_written() {
    let mut w = World::new();
    w.click_key(0x14, false); // Q
    w.edit_value("0.20", 0, "0.35");
    w.edit_value("0.20", 0, "0.05");
    w.apply("Terapkan ke keyboard");
    let s = w.sim();
    assert_eq!(s.p().key(0x14, DP), 350);
    assert_eq!(s.p().key(0x14, DR), 50);
    // Deadzones alone keep the key Global and its actuation.
    assert_eq!(s.p().mode(0x14), 0);
    assert_eq!(s.p().key(0x14, ACT), 2000);
}

#[test]
fn a_shorter_switch_clamps_and_sends_the_clamped_values() {
    let mut w = World::new();
    w.click_key(0x14, false);
    w.edit_value("2.000 mm", 0, "3.4");
    w.click_role(Role::ComboBox, 0);
    w.click_text("Magnetic Jade Pro");
    assert!(w.has_text("disesuaikan ke batasnya"), "{:?}", w.labels());
    w.apply("Terapkan ke keyboard");
    let s = w.sim();
    assert_eq!(s.p().key(0x14, AXIS), 1);
    assert_eq!(s.p().key(0x14, ACT), 3300);
}

#[test]
fn a_rejected_write_shows_the_error_and_the_values_the_keyboard_kept() {
    let mut w = World::new();
    // The keyboard refuses every KEY SET.
    w.sim().rejects.push(Reject {
        cmd: 0x23,
        sets_only: true,
        at: vec![],
        times: u32::MAX,
    });
    w.click_key(0x14, false);
    w.edit_value("2.000 mm", 0, "1.5");
    w.apply("Terapkan ke keyboard");
    assert!(w.has("Gagal menerapkan"), "{:?}", w.labels());
    assert_eq!(w.sim().p().key(0x14, ACT), 2000);
    assert_eq!(w.snapshot().keys[&0x14].actuation.0, 2000);
    // Retry after the keyboard accepts writes again.
    w.sim().rejects.clear();
    w.click("Coba lagi");
    w.idle();
    assert_eq!(w.sim().p().key(0x14, ACT), 1500);
    assert!(!w.has("Gagal menerapkan"));
}

/// A shorter switch on several keys: a key other than the first one may
/// hold values beyond it. Each key is clamped to the new travel, the others
/// keep their own values, and the write goes through.
#[test]
fn a_shorter_switch_on_several_keys_clamps_every_key() {
    let mut w = World::new();
    w.click_key(W, false);
    w.edit_value("2.000 mm", 0, "3.4");
    w.apply("Terapkan ke keyboard");
    assert_eq!(w.sim().p().key(W, ACT), 3400);
    w.click_key(0x14, false);
    w.click_key(W, true);
    w.click_role(Role::ComboBox, 0);
    w.click_text("Magneto");
    assert!(w.has_text("disesuaikan ke batasnya"), "{:?}", w.labels());
    w.apply("Terapkan ke keyboard");
    assert!(!w.has("Gagal menerapkan"), "{:?}", w.labels());
    let s = w.sim();
    assert_eq!(s.p().key(0x14, AXIS), 3);
    assert_eq!(s.p().key(W, AXIS), 3);
    assert_eq!(s.p().key(W, ACT), 3300, "W clamped");
    assert_eq!(s.p().key(0x14, ACT), 2000, "Q keeps its own value");
}

/// Top-bar switch to profile `label` ("3"), waiting for the reconnect.
fn switch_profile(w: &mut World, label: &str) {
    let want: u8 = label.parse::<u8>().unwrap() - 1;
    w.click(label);
    w.until("profile switched", 20, |w| {
        w.worker.snapshot().is_some_and(|s| s.profile == want) && !w.worker.is_restarting()
    });
    w.idle();
}

/// An apply finishing while another page is shown is only read by this
/// page later; by then the keyboard may be on another profile, whose
/// snapshot must not get the old profile's values.
#[test]
fn an_apply_read_after_a_profile_switch_is_not_patched_into_the_new_profile() {
    let mut w = World::new();
    w.click_key(0x14, false);
    w.edit_value("2.000 mm", 0, "1.5");
    w.sim().set_delay = std::time::Duration::from_millis(20);
    w.click("Terapkan ke keyboard");
    assert!(w.worker.is_busy());
    w.page("Remap");
    w.sim().set_delay = std::time::Duration::ZERO;
    w.idle();
    assert_eq!(w.sim().profiles[0].key(0x14, ACT), 1500);
    switch_profile(&mut w, "3");
    w.page("Performa");
    w.frames(5);
    assert_eq!(w.sim().p().key(0x14, ACT), 2000);
    assert_eq!(w.snapshot().keys[&0x14].actuation.0, 2000, "stale patch");
}

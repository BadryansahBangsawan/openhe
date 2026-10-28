//! Lampu end to end on the simulated keyboard (PRGB with the observed
//! firmware behaviour, KRGB with its FE commit).

mod support;
use eframe::egui::accesskit::Role;
use support::sim::prgb;
use support::*;

fn lampu() -> World {
    let mut w = World::new();
    w.page("Lampu");
    w.idle();
    w
}

/// Sets the n-th 1..=5 slider (brightness 0, speed 1) to `level`.
fn set_level(w: &mut World, n: usize, level: u8) {
    use egui_kittest::kittest::Queryable;
    let r =
        w.h.query_all_by_role(Role::Slider)
            .nth(n)
            .expect("slider")
            .rect();
    let t = f32::from(level - 1) / 4.0;
    let x = r.left() + 2.0 + t * (r.width() - 4.0);
    w.click_at(eframe::egui::pos2(x, r.center().y));
}

#[test]
fn reading_uses_the_len15_get_and_shows_the_keyboard_state() {
    let w = lampu();
    let s = w.sim();
    assert_eq!(s.bad_prgb_gets, 0, "a LEN-44 GET reached the keyboard");
    assert!(s.rgbnack <= 1, "RGBNACK more than once per connection");
    drop(s);
    // Factory: Ripple Light (mode 3), brightness 3 → "4 / 5".
    assert!(
        w.selected("Riak cahaya") || w.has_text("4 / 5"),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("=80%"), "{:?}", w.labels());
}

#[test]
fn effect_brightness_and_colour_are_written_with_unknown_flags_kept() {
    let mut w = lampu();
    w.click("Statis");
    set_level(&mut w, 0, 5);
    // One colour: palette slot 3, set to #123456 through the hex box.
    w.click("Satu warna");
    w.click("Warna 3");
    w.type_in_last_field("#123456");
    w.apply("Terapkan ke keyboard");
    let l = w.sim().p().lighting;
    assert_eq!(l[prgb::MODE], 0, "static");
    assert_eq!(l[prgb::BRIGHTNESS], 4, "level 5");
    assert_eq!(l[prgb::STATIC], 2, "palette slot 3");
    assert_eq!(l[prgb::SPEED], 0, "static has no speed");
    // Colour 3 in B, G, R order.
    assert_eq!(
        &l[prgb::COLORS + 8..prgb::COLORS + 12],
        &[0x56, 0x34, 0x12, 0xFF]
    );
    // The keyboard's own flag bits (0xC1) survive: on, bits 6 and 7.
    assert_eq!(l[prgb::FLAGS], 0xC1, "flags {:02X}", l[prgb::FLAGS]);
    assert!(w.has("Tersimpan di keyboard"), "{:?}", w.labels());
    assert_eq!(w.sim().bad_prgb_gets, 0);
}

#[test]
fn turning_the_light_off_clears_only_bit0() {
    let mut w = lampu();
    w.click_role(Role::CheckBox, 0);
    w.apply("Terapkan ke keyboard");
    assert_eq!(w.sim().p().lighting[prgb::FLAGS], 0xC0);
    w.click_role(Role::CheckBox, 0);
    w.apply("Terapkan ke keyboard");
    assert_eq!(w.sim().p().lighting[prgb::FLAGS], 0xC1);
}

#[test]
fn per_key_colours_are_written_and_committed() {
    let mut w = lampu();
    w.click("Warna per tombol");
    w.idle();
    w.click_key(0x1A, false);
    w.click_key(0x04, true);
    w.type_in_last_field("#FF8000");
    w.click("Cat tombol terpilih");
    assert!(w.has_text("2 tombol berubah"), "{:?}", w.labels());
    w.apply("Terapkan warna tombol");
    let s = w.sim();
    assert_eq!(s.krgb_commits, 1, "one FE commit after the batch");
    assert_eq!(s.p().krgb.get(&0x1A), Some(&[0xFF, 0x80, 0x00]));
    assert_eq!(s.p().krgb.get(&0x04), Some(&[0xFF, 0x80, 0x00]));
    assert!(s.p().krgb_pending.is_empty(), "uncommitted colours left");
    assert_eq!(s.sets(0x2A).len(), 3, "two keys and the commit");
}

#[test]
fn custom_mode_from_the_key_tab() {
    let mut w = lampu();
    w.click("Warna per tombol");
    w.idle();
    w.click("Pilih efek Kustom");
    w.apply("Terapkan ke keyboard");
    assert_eq!(w.sim().p().lighting[prgb::MODE], 21);
}

#[test]
fn a_lighting_change_on_another_profile_is_read_after_the_switch() {
    let sim = Sim::new();
    sim.state().profiles[2].lighting[prgb::BRIGHTNESS] = 0;
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Lampu");
    w.idle();
    assert!(w.has_text("=80%"));
    w.click("3"); // top bar profile 3
    w.until("profile 3 loaded", 20, |w| {
        w.worker.snapshot().is_some_and(|s| s.profile == 2) && !w.worker.is_restarting()
    });
    w.idle();
    w.frames(5);
    assert!(w.has_text("=20%"), "{:?}", w.labels());
}

#[test]
fn a_keyboard_that_ignores_rgbnack_still_shows_its_lighting() {
    // RGBNACK is optional (the GET works without it): a firmware that
    // does not answer it at all must not make the page fail.
    let sim = Sim::new();
    sim.state().rgbnack_silent = true;
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Lampu");
    w.idle();
    assert!(
        !w.has_text("Gagal membaca pengaturan lampu"),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("=80%"), "{:?}", w.labels());
}

fn switch_to_profile_3(w: &mut World) {
    w.click("3"); // top bar profile 3
    w.until("profile 3 loaded", 20, |w| {
        w.worker.snapshot().is_some_and(|s| s.profile == 2) && !w.worker.is_restarting()
    });
    w.idle();
    w.frames(5);
}

#[test]
fn unsent_effect_edits_are_dropped_on_a_profile_switch() {
    let mut w = lampu();
    w.click("Statis");
    assert!(w.has_text("Perubahan belum dikirim"), "{:?}", w.labels());
    switch_to_profile_3(&mut w);
    // Profile 3 shows its own lighting, not profile 1's unsent edits.
    assert!(!w.has_text("Perubahan belum dikirim"), "{:?}", w.labels());
    assert!(w.has_text("Belum ada perubahan"), "{:?}", w.labels());
    assert!(!w.enabled("Terapkan ke keyboard"));
}

#[test]
fn unsent_key_paint_is_dropped_on_a_profile_switch() {
    let mut w = lampu();
    w.click("Warna per tombol");
    w.idle();
    w.click_key(0x1A, false);
    w.type_in_last_field("#FF8000");
    w.click("Cat tombol terpilih");
    assert!(w.has_text("1 tombol berubah"), "{:?}", w.labels());
    switch_to_profile_3(&mut w);
    assert!(w.has_text("0 tombol berubah"), "{:?}", w.labels());
}

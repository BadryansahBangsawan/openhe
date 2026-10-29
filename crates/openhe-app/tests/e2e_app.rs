//! Cross-page flows and the app shell: changes seen on other pages after a
//! reload, a disconnect in the middle of an apply, closing the app while
//! busy, and data that must survive closing.

mod support;
use eframe::egui::accesskit::Role;
use eframe::App as _;
use openhe_app::storage::AppData;
use support::*;

fn saved() -> AppData {
    AppData::load_from(&AppData::default_dir().unwrap()).unwrap_or_default()
}

#[test]
fn long_profile_names_never_run_into_the_top_bar_title() {
    let mut data = AppData::default();
    data.profile_names = [
        "Kerja kantor harian",
        "Gaming kompetitif",
        "Menulis dokumen",
        "Desain grafis",
    ]
    .map(String::from);
    let mut w = World::with(Sim::new(), data, fast_config());
    w.page("Macro");
    w.h.set_size(eframe::egui::vec2(1280.0, 800.0));
    w.frames(5);
    let subtitle = w.rect("Rekam urutan tombol, pasang ke tombol mana saja");
    let profil = w.rect("Profil");
    assert!(
        subtitle.right() <= profil.left(),
        "subtitle {subtitle:?} runs under the profile switch at {profil:?}"
    );
    // A shortened name keeps its number to click.
    assert!(w.has_text("1 · Kerja"), "{:?}", w.labels());
}

#[test]
fn a_profile_name_typed_just_before_closing_is_saved() {
    let mut w = World::new();
    w.page("Pengaturan");
    w.click_role(Role::TextInput, 0); // the name field of Profil 1
    w.type_text("Kerja");
    // Closed within the 0.8 s typing pause.
    w.app_mut().on_exit(None);
    assert_eq!(saved().profile_names[0], "Kerja");
}

#[test]
fn a_change_on_one_page_shows_on_the_others() {
    let mut w = World::new();
    // Remap: Caps → Esc on Base.
    w.page("Remap");
    w.idle();
    w.click_key(0x39, false);
    w.click_role(Role::TextInput, 0);
    w.replace_text("esc");
    let n = w.h_count("ESC");
    w.click_nth("ESC", n - 1);
    w.apply("Terapkan");
    // Tombol Lanjutan: an END function on G.
    w.page("Tombol Lanjutan");
    w.click_key(0x0A, false);
    w.frames(3);
    w.click("END");
    w.pick_code("Kosong", 0, "f1", "F1");
    w.apply("Terapkan");
    // Performa: G's actuation.
    w.page("Performa");
    w.click_key(0x0A, false);
    w.edit_value("2.000 mm", 0, "1.2");
    w.apply("Terapkan ke keyboard");
    // A full reload (as after a reconnect) reads it all back.
    w.worker.request_reload();
    w.idle();
    let snap = w.snapshot();
    assert_eq!(snap.keys[&0x0A].actuation.0, 1200);
    assert_eq!(
        snap.keys[&0x0A].mode.advanced,
        openhe_proto::key::Advanced::End,
        "the Performa write kept G's END function"
    );
    w.page("Tombol Lanjutan");
    w.idle();
    assert!(w.has("G · END"), "{:?}", w.labels());
    w.page("Remap");
    w.idle();
    w.click_key(0x39, false);
    assert!(w.has_text("Caps sekarang mengirim ESC"), "{:?}", w.labels());
}

#[test]
fn a_disconnect_in_the_middle_of_an_apply_is_reported_and_reloaded() {
    let mut w = World::new();
    w.click("Pilih semua");
    w.edit_value("2.000 mm", 0, "1.0");
    // The keyboard drops off after 2 more writes, then comes back.
    w.sim().unplug_after_sets = Some(2);
    let c = w.worker.connects();
    w.click("Terapkan ke keyboard");
    w.until("lost", 10, |w| !w.sim().present);
    w.sim().replug();
    w.until("back", 20, |w| w.worker.connects() > c);
    w.idle();
    assert!(
        w.has_text("Keyboard terputus. Sebagian perubahan mungkin sudah terkirim"),
        "{:?}",
        w.labels()
    );
    // The screen shows what the keyboard really holds.
    let s = w.sim();
    let snap = w.snapshot();
    for k in sim::keys() {
        assert_eq!(snap.keys[&k].actuation.0, s.p().key(k, 4), "0x{k:02X}");
    }
}

#[test]
fn closing_while_a_write_runs_waits_for_it() {
    use eframe::egui::{ViewportCommand, ViewportEvent, ViewportId};
    let mut w = World::new();
    w.click("Pilih semua");
    w.edit_value("2.000 mm", 0, "1.0");
    // A slow keyboard: every write takes 20 ms (68 keys: over a second).
    w.sim().set_delay = std::time::Duration::from_millis(20);
    w.click("Terapkan ke keyboard");
    assert!(w.worker.is_busy());
    let close = |w: &mut World| {
        w.h.input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .events
            .push(ViewportEvent::Close);
        w.h.step();
        w.h.output().viewport_output[&ViewportId::ROOT]
            .commands
            .clone()
    };
    let cmds = close(&mut w);
    assert!(
        cmds.contains(&ViewportCommand::CancelClose),
        "close not held while writing: {cmds:?}"
    );
    assert!(w.has_text("Menunggu"), "{:?}", w.labels());
    // Once the write is done the window closes by itself.
    let mut closed = false;
    w.until("closed after the write", 30, |w| {
        let c = &w.h.output().viewport_output[&ViewportId::ROOT].commands;
        closed |= c.contains(&ViewportCommand::Close);
        closed
    });
    assert!(!w.worker.is_busy(), "closed before the write finished");
    let s = w.sim();
    for k in sim::keys() {
        assert_eq!(s.p().key(k, 4), 1000, "0x{k:02X}");
    }
}

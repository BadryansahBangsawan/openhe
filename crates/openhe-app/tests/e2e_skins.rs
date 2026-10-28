//! Keyboard colours ("Warna keyboard"): the first-connect dialog, the
//! Pengaturan chips and the custom editor, on the simulator.

mod support;
use openhe_app::skin::{KeyboardSkin, PRESETS};
use openhe_app::storage::AppData;
use support::*;

const TITLE: &str = "Warna keyboard kamu yang mana?";

/// The data file as saved.
fn saved() -> AppData {
    AppData::load_from(&AppData::default_dir().unwrap()).unwrap()
}

fn saved_skin() -> Option<KeyboardSkin> {
    saved().keyboard_skins.get(SIM_SERIAL).copied()
}

#[test]
fn dialog_asks_once_and_stores_the_choice() {
    let mut w = World::first_connect(AppData::default());
    w.frames(3);
    assert!(w.has_text(TITLE), "{:?}", w.labels());
    assert!(w.has_text("WIN 68 HE MAX TERSAMBUNG"));
    // Abu Kuning is offered first; pick Hitam Merah instead.
    assert!(w.selected("Abu Kuning"));
    w.click("Hitam Merah");
    assert!(w.selected("Hitam Merah"));
    assert!(!w.selected("Abu Kuning"));
    w.click("Pakai warna ini");
    w.frames(3);
    assert!(!w.has_text(TITLE));
    assert_eq!(saved_skin(), Some(KeyboardSkin::BlackRed));
    assert_eq!(saved().last_keyboard.as_deref(), Some(SIM_SERIAL));
    // The app runs on: no dialog again.
    w.frames(5);
    assert!(!w.has_text(TITLE));
    // Nor when the app opens again with the saved data.
    let data = saved();
    drop(w);
    let mut w = World::first_connect(data);
    w.frames(5);
    assert!(!w.has_text(TITLE));
}

#[test]
fn later_stores_the_dark_skin_and_does_not_ask_again() {
    let mut w = World::first_connect(AppData::default());
    w.frames(3);
    assert!(w.has_text(TITLE));
    w.click("Nanti saja");
    w.frames(3);
    assert!(!w.has_text(TITLE));
    assert_eq!(saved_skin(), Some(KeyboardSkin::Gelap));
    let data = saved();
    drop(w);
    let mut w = World::first_connect(data);
    w.frames(5);
    assert!(!w.has_text(TITLE));
}

#[test]
fn answering_gives_the_page_back() {
    let mut w = World::first_connect(AppData::default());
    w.frames(3);
    assert!(w.has_text(TITLE));
    w.click("Pakai warna ini");
    assert_eq!(saved_skin(), Some(KeyboardSkin::GreyYellow));
    w.click("WASD");
    assert!(w.key_selected(0x1A));
}

#[test]
fn escape_closes_for_now_and_asks_next_time() {
    use eframe::egui::{Key, Modifiers};
    let mut w = World::first_connect(AppData::default());
    w.frames(3);
    assert!(w.has_text(TITLE));
    w.key(Key::Escape, Modifiers::NONE);
    w.frames(3);
    assert!(!w.has_text(TITLE));
    assert_eq!(saved_skin(), None, "nothing stored");
    w.frames(5);
    assert!(!w.has_text(TITLE), "not again this session");
    drop(w);
    let mut w = World::first_connect(AppData::default());
    w.frames(3);
    assert!(w.has_text(TITLE));
}

#[test]
fn settings_chips_switch_the_skin() {
    let mut w = World::new();
    w.page("Pengaturan");
    w.idle();
    assert!(w.has("Warna keyboard"));
    // The test world counts the keyboard as Gelap.
    assert!(w.selected("Gelap"));
    w.click("Putih Biru");
    assert!(w.selected("Putih Biru"));
    assert!(!w.selected("Gelap"));
    assert_eq!(saved_skin(), Some(KeyboardSkin::WhiteBlue));
    w.click("Abu Kuning");
    assert_eq!(saved_skin(), Some(KeyboardSkin::GreyYellow));
    w.click("Hitam Merah");
    assert_eq!(saved_skin(), Some(KeyboardSkin::BlackRed));
    // The other pages draw with it and still work.
    w.page("Performa");
    w.click("WASD");
    assert!(w.key_selected(0x04));
}

#[test]
fn custom_editor_saves() {
    let mut w = World::new();
    w.page("Pengaturan");
    w.idle();
    w.click("Kustom…");
    assert!(w.has_text("Keycap kustom"), "{:?}", w.labels());
    assert!(w.has_text("#D9D4C7"));
    // A swatch for the accent field (active first), then a typed alpha.
    w.click("Warna contoh 4");
    assert!(w.has_text("#C42330"));
    let fields = w.role_count(eframe::egui::accesskit::Role::TextInput);
    // The three hex fields come last: alpha, mod, accent.
    w.click_role(eframe::egui::accesskit::Role::TextInput, fields - 3);
    w.replace_text("#112233");
    w.frames(2);
    w.click("Simpan");
    w.frames(2);
    assert!(!w.has_text("Keycap kustom"));
    assert_eq!(
        saved_skin(),
        Some(KeyboardSkin::custom(
            [0x11, 0x22, 0x33],
            [0x3A, 0x3F, 0x4B],
            PRESETS[3]
        ))
    );
    assert!(w.selected("Kustom…"));
    // Opening it again starts from the saved colours.
    w.click("Kustom…");
    assert!(w.has_text("#112233"));
    w.click("Batal");
}

#[test]
fn custom_editor_cancel_keeps_the_skin() {
    let mut w = World::new();
    w.page("Pengaturan");
    w.idle();
    w.click("Putih Biru");
    w.click("Kustom…");
    w.click("Warna contoh 1");
    w.click("Batal");
    w.frames(2);
    assert!(!w.has_text("Keycap kustom"));
    assert_eq!(saved_skin(), Some(KeyboardSkin::WhiteBlue));
    assert!(w.selected("Putih Biru"));
}

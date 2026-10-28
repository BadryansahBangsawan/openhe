//! Renders every page, in a realistic state, to PNGs under
//! `target/screenshots/`, for comparing with `docs/design/mockups/`.
//! Ignored by default (it needs a GPU adapter); run with
//! `cargo test -p openhe-app --test screenshots -- --ignored`.

mod support;
use eframe::egui::{Vec2, ViewportId};
use openhe_app::storage::{AppData, SavedMacro, SavedMacroAction};
use std::path::PathBuf;
use support::*;

/// The minimum window (`theme::size::MIN_WINDOW`).
const DESKTOP: Vec2 = Vec2::new(1280.0, 800.0);
/// The start window (`theme::size::START_WINDOW`); the mockups' proportions
/// match this size.
const START: Vec2 = Vec2::new(1440.0, 900.0);
const SMALL: Vec2 = Vec2::new(900.0, 600.0);
/// `World::with`'s window size.
const TALL: Vec2 = Vec2::new(1600.0, 1500.0);

const W: u8 = 0x1A;
const A: u8 = 0x04;
const S: u8 = 0x16;
const D: u8 = 0x07;
const E: u8 = 0x08;
const R: u8 = 0x15;
const CAPS: u8 = 0x39;
const SPACE: u8 = 0x2C;

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app, driven in the harness's tall default window (so every control
/// is on screen to click); `shot` resizes it.
fn world(data: AppData) -> World {
    World::with(Sim::new(), data, fast_config())
}

/// Sets the native scale. Clicks go through AccessKit rects, which are in
/// physical pixels, so pages are driven at 1x and only rendered at 2x.
fn scale(w: &mut World, pixels_per_point: f32) {
    let input = w.h.input_mut();
    input
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(pixels_per_point);
}

fn profile_names() -> AppData {
    let mut d = AppData::default();
    d.profile_names[0] = "Kerja".into();
    d.profile_names[1] = "Gaming".into();
    d
}

/// Lets layout settle at 2x (like the 2x mockups), writes `name.png`,
/// then goes back to 1x.
fn shot(w: &mut World, name: &str, size: Vec2) {
    w.h.set_size(size);
    scale(w, 2.0);
    w.frames(20);
    let img = w.h.render().expect("render");
    let path = out_dir().join(format!("{name}.png"));
    img.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
    w.h.set_size(TALL);
    scale(w, 1.0);
    w.frames(3);
}

/// `name.png` at the minimum window and `name-1440x900.png` at the start
/// window.
fn shots(w: &mut World, name: &str) {
    shot(w, name, DESKTOP);
    shot(w, &format!("{name}-1440x900"), START);
}

/// Performa with W, A, S, D on rapid trigger and selected; also shot at a
/// small window size, to check clipping.
#[test]
#[ignore]
fn screenshot_01_performa() {
    let mut w = world(profile_names());
    w.click("WASD");
    w.click("Rapid Trigger");
    w.edit_value("2.000 mm", 0, "0.4");
    w.edit_value("0.020 mm", 0, "0.15");
    w.apply("Terapkan ke keyboard");
    shots(&mut w, "01-performa");
    shot(&mut w, "01-performa-900x600", SMALL);
}

#[test]
#[ignore]
fn screenshot_02_tombol_lanjutan() {
    let mut w = world(profile_names());
    w.page("Tombol Lanjutan");
    // Caps: MT (tap Esc, hold Ctrl).
    w.click_key(CAPS, false);
    w.frames(3);
    w.click("MT");
    w.pick_code("Kosong", 0, "ctrl kiri", "Ctrl kiri");
    w.pick_code("Kosong", 0, "esc", "ESC");
    w.apply("Terapkan");
    // E: TGL.
    w.click_key(E, false);
    w.frames(3);
    w.click("TGL");
    w.pick_code("Kosong", 0, "e", "E");
    w.apply("Terapkan");
    // R: MPT.
    w.click_key(R, false);
    w.frames(3);
    w.click("MPT");
    for (search, chip) in [("r", "R"), ("f1", "F1"), ("f2", "F2")] {
        w.pick_code("Kosong", 0, search, chip);
    }
    w.apply("Terapkan");
    // A <-> D: SOCD.
    w.click_key(A, false);
    w.frames(3);
    w.click("SOCD");
    w.click("Pilih di keyboard");
    w.click_key(D, false);
    w.apply("Terapkan");
    assert!(w.has("A ↔ D · SOCD"), "{:?}", w.labels());
    // Space: DKS, being edited (not applied).
    w.click_key(SPACE, false);
    w.frames(3);
    w.click("DKS");
    w.pick_code("Kosong", 0, "space", "Space");
    w.pick_code("Kosong", 0, "c", "C");
    w.click("DKS 1 · Mulai tekan");
    w.click("DKS 2 · Tekan penuh");
    shots(&mut w, "02-tombol-lanjutan");
}

#[test]
#[ignore]
fn screenshot_03_remap() {
    let mut w = world(profile_names());
    w.page("Remap");
    w.idle();
    w.click("Fn1");
    w.idle();
    for (key, hex) in [(A, "0x0050"), (S, "0x0051"), (D, "0x004F"), (W, "0x0052")] {
        w.click_key(key, false);
        w.click_role(eframe::egui::accesskit::Role::TextInput, 0);
        w.replace_text(hex);
        let chip = match hex {
            "0x0050" => "←",
            "0x0051" => "↓",
            "0x004F" => "→",
            _ => "↑",
        };
        let n = w.h_count(chip);
        assert!(n > 0, "no chip {chip:?}: {:?}", w.labels());
        w.click_nth(chip, n - 1);
    }
    // The picker on its Navigasi tab, as in the mockup.
    w.click_role(eframe::egui::accesskit::Role::TextInput, 0);
    w.replace_text("");
    w.click("Navigasi");
    shots(&mut w, "03-remap");
}

fn step(code: u16, down: bool, delay_ms: u32) -> SavedMacroAction {
    SavedMacroAction {
        code,
        down,
        delay_ms,
    }
}

fn macro_named(name: &str, actions: Vec<SavedMacroAction>) -> SavedMacro {
    SavedMacro {
        actions,
        ..SavedMacro::new(name)
    }
}

#[test]
#[ignore]
fn screenshot_04_macro() {
    let mut data = profile_names();
    const CTRL: u16 = 0xE0;
    const C: u16 = 0x06;
    const V: u16 = 0x19;
    data.macros = vec![
        macro_named(
            "Salin + tempel cepat",
            vec![
                step(CTRL, true, 20),
                step(C, true, 15),
                step(C, false, 100),
                step(V, true, 15),
                step(V, false, 20),
                step(CTRL, false, 0),
            ],
        ),
        macro_named(
            "Buka terminal",
            vec![
                step(CTRL, true, 10),
                step(0x17, true, 10),
                step(0x17, false, 10),
                step(CTRL, false, 0),
            ],
        ),
        macro_named(
            "Spam klik kiri",
            vec![step(0x04, true, 5), step(0x04, false, 5)],
        ),
        macro_named(
            "Tanda tangan email",
            vec![step(0x0B, true, 5), step(0x0B, false, 0)],
        ),
    ];
    let mut w = world(data);
    w.page("Macro");
    w.click_text("Salin + tempel cepat");
    w.click("Pilih tombol");
    w.click_key(0x0A, false); // G
    w.idle();
    shots(&mut w, "04-macro");
}

#[test]
#[ignore]
fn screenshot_05_lampu() {
    let mut w = world(profile_names());
    w.page("Lampu");
    w.idle();
    w.click("Gelombang");
    w.frames(3);
    if w.has("Pelangi") {
        w.click("Pelangi");
    }
    shots(&mut w, "05-lampu");
}

#[test]
#[ignore]
fn screenshot_06_kalibrasi() {
    let mut w = world(profile_names());
    w.page("Kalibrasi");
    w.click("Mulai kalibrasi");
    w.until("calibration started", 10, |w| w.sim().calibrating);
    w.until("settled", 10, |w| w.has_text("2. Tekan tiap tombol"));
    std::thread::sleep(std::time::Duration::from_millis(2100));
    {
        let mut s = w.sim();
        for k in sim::keys() {
            if ![W, A, S, D].contains(&k) {
                s.press(k, 3400);
            }
        }
    }
    w.frames(40);
    {
        let mut s = w.sim();
        for k in sim::keys() {
            if ![W, A, S, D].contains(&k) {
                s.press(k, 0);
            }
        }
        for k in [W, A, S, D] {
            s.press(k, 1800);
        }
    }
    w.frames(40);
    shots(&mut w, "06-kalibrasi");
    w.click("Batal");
    w.until("stopped", 10, |w| !w.sim().calibrating);
}

#[test]
#[ignore]
fn screenshot_07_pengaturan() {
    let mut w = world(profile_names());
    w.page("Pengaturan");
    w.idle();
    shots(&mut w, "07-pengaturan");
}

/// Template: a saved template and a backup listed, the Gaming FPS
/// preview open (summary and highlighted keys).
#[test]
#[ignore]
fn screenshot_09_template() {
    let mut w = world(profile_names());
    w.page("Template");
    w.click("Simpan setting sekarang…");
    w.type_in_last_field("Kerja harian");
    w.click("Simpan");
    w.until("saved", 20, |w| w.has_text("Template disimpan."));
    for name in ["Caps jadi Ctrl", "Gaming FPS"] {
        w.click(name);
        w.until("the preview", 20, |w| {
            w.app().templates().preview_keys().is_some()
        });
        if name == "Caps jadi Ctrl" {
            w.click("Terapkan");
            w.until("applied", 30, |w| {
                w.has_text("Tersimpan di keyboard") && !w.worker.is_busy()
            });
            w.idle();
        }
    }
    shots(&mut w, "09-template");
}

fn skin_slug(s: &openhe_app::skin::KeyboardSkin) -> &'static str {
    use openhe_app::skin::KeyboardSkin::*;
    match s {
        GreyYellow => "abu-kuning",
        BlackRed => "hitam-merah",
        WhiteBlue => "putih-biru",
        Gelap => "gelap",
        Custom { .. } => "kustom",
    }
}

/// Performa (as in screenshot 01) in every skin, at 1280×800.
#[test]
#[ignore]
fn screenshot_08_performa_skins() {
    use openhe_app::skin::KeyboardSkin;
    for skin in [
        KeyboardSkin::GreyYellow,
        KeyboardSkin::BlackRed,
        KeyboardSkin::WhiteBlue,
        KeyboardSkin::Gelap,
        KeyboardSkin::custom([0xD9, 0xD4, 0xC7], [0x3A, 0x3F, 0x4B], [0xE2, 0x72, 0x5B]),
    ] {
        let mut data = profile_names();
        data.keyboard_skins.insert(SIM_SERIAL.into(), skin);
        let mut w = world(data);
        w.click("WASD");
        w.click("Rapid Trigger");
        w.edit_value("2.000 mm", 0, "0.4");
        w.apply("Terapkan ke keyboard");
        shot(
            &mut w,
            &format!("08-performa-{}", skin_slug(&skin)),
            DESKTOP,
        );
    }
}

/// The first-connect dialog (mockup 08c).
#[test]
#[ignore]
fn screenshot_08c_dialog() {
    let mut w = World::first_connect(profile_names());
    w.frames(3);
    shots(&mut w, "08c-warna-dialog");
}

/// Pengaturan with the "Warna keyboard" row (mockup 08d), and the custom
/// editor (mockup 08e).
#[test]
#[ignore]
fn screenshot_08d_pengaturan_and_08e_kustom() {
    let mut w = world(profile_names());
    w.page("Pengaturan");
    w.idle();
    w.click("Abu Kuning");
    shots(&mut w, "08d-warna-pengaturan");
    w.click("Kustom…");
    shots(&mut w, "08e-warna-kustom");
}

/// The keyboard widget in every skin, WIN 68 and WIN 60, with W selected
/// and A, S, D marked (compare with mockups 08 and 08b).
#[test]
#[ignore]
fn screenshot_08_skins() {
    use eframe::egui;
    use openhe_app::skin::KeyboardSkin;
    use openhe_app::widgets::keyboard::{reference_matrix, KeyDecoration, KeyboardView, Selection};
    use openhe_proto::Model;
    let skins = [
        KeyboardSkin::GreyYellow,
        KeyboardSkin::BlackRed,
        KeyboardSkin::WhiteBlue,
        KeyboardSkin::Gelap,
        KeyboardSkin::custom([0xD9, 0xD4, 0xC7], [0x3A, 0x3F, 0x4B], [0xE2, 0x72, 0x5B]),
    ];
    for model in [Model::Win68HeMax, Model::Win60HeMax] {
        let mut ready = false;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1800.0, 1100.0))
            .with_pixels_per_point(2.0)
            .build_ui(move |ui| {
                if !ready {
                    openhe_app::theme::apply(ui.ctx());
                    ready = true;
                }
                openhe_app::theme::ensure_fonts(ui.ctx());
                let m = reference_matrix(model);
                egui::Grid::new("g").spacing([24.0, 24.0]).show(ui, |ui| {
                    for (i, s) in skins.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.set_width(860.0);
                            ui.label(format!("{s:?}"));
                            let mut sel = Selection::default();
                            sel.set([0x1A]);
                            KeyboardView::new(model, &m)
                                .skin(*s)
                                .max_unit(50.0)
                                .id_salt(["k0", "k1", "k2", "k3", "k4"][i])
                                .show(ui, &mut sel, |c| KeyDecoration {
                                    badge: [0x04, 0x16, 0x07]
                                        .contains(&c)
                                        .then(|| (String::new(), openhe_app::theme::color::ACCENT)),
                                    ..Default::default()
                                });
                        });
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            });
        h.run_steps(4);
        let img = h.render().expect("render");
        let name = match model {
            Model::Win68HeMax => "08-skins-win68",
            Model::Win60HeMax => "08b-skins-win60",
        };
        let path = out_dir().join(format!("{name}.png"));
        img.save(&path).expect("save png");
        eprintln!("wrote {}", path.display());
    }
}

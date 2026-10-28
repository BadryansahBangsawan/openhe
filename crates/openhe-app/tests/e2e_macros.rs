//! Macro end to end: record from key events, edit steps, the balance
//! guard, binding to a key (also over another function) and the library
//! file.

mod support;
use eframe::egui::{Key, Modifiers};
use openhe_app::storage::AppData;
use support::*;

const G: u8 = 0x0A;

fn saved() -> AppData {
    AppData::load_from(&AppData::default_dir().unwrap()).expect("data file")
}

/// A new macro with H and I tapped, recorded from key events.
fn recorded() -> World {
    let mut w = World::new();
    w.page("Macro");
    w.click("+ Baru");
    w.click("● Rekam");
    for k in [Key::H, Key::I] {
        w.h.input_mut().events.push(eframe::egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        w.frames(3);
        w.h.input_mut().events.push(eframe::egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        w.frames(3);
    }
    w.click("● Berhenti");
    w
}

fn pick_target(w: &mut World, key: u8) {
    w.click("Pilih tombol");
    w.click_key(key, false);
}

#[test]
fn recording_turns_key_events_into_steps_and_saves_the_library() {
    let w = recorded();
    assert!(w.has_text("4 langkah"), "{:?}", w.labels());
    let data = saved();
    assert_eq!(data.macros.len(), 1);
    let codes: Vec<(u16, bool)> = data.macros[0]
        .actions
        .iter()
        .map(|a| (a.code, a.down))
        .collect();
    assert_eq!(
        codes,
        vec![(0x0B, true), (0x0B, false), (0x0C, true), (0x0C, false)]
    );
}

#[test]
fn editing_steps_and_the_balance_guard() {
    let mut w = recorded();
    // Delete the last step (I up): I stays pressed, the bind is refused.
    let n = w.h_count("×");
    w.click_nth("×", n - 1);
    assert!(
        w.has_text("ditekan tetapi tidak dilepas"),
        "{:?}",
        w.labels()
    );
    pick_target(&mut w, G);
    assert!(
        !w.enabled("Pasang ke tombol"),
        "an unbalanced macro can be bound"
    );
    w.click("Tambahkan pelepasan otomatis");
    assert!(w.enabled("Pasang ke tombol"));
    assert_eq!(saved().macros[0].actions.len(), 4);
    // Change a delay.
    w.edit_value("0 ms", 0, "250");
    assert!(saved().macros[0].actions.iter().any(|a| a.delay_ms == 250));
}

#[test]
fn bind_to_a_key_writes_data_mode_and_play_settings() {
    let mut w = recorded();
    w.click("Nyala/mati");
    pick_target(&mut w, G);
    w.idle();
    w.apply("Pasang ke tombol");
    assert!(w.has("Macro terpasang di keyboard"), "{:?}", w.labels());
    let s = w.sim();
    assert_eq!(s.p().mode(G) & 0x0F, 6);
    assert_eq!(s.p().macro_data.len(), 4);
    assert_eq!(s.p().macro_data[&256].0, 0x0B);
    let mm = &s.p().macro_mode[&G];
    assert_eq!(mm[3], 4, "len written");
    assert_eq!(mm[4], 1, "toggle mode");
    drop(s);
    assert_eq!(
        w.snapshot().keys[&G].mode.advanced,
        openhe_proto::key::Advanced::Macro
    );
    // The read-back reports len 0 (finding 24): no step count from it.
    assert!(!w.has_text("0 langkah"), "{:?}", w.labels());
}

#[test]
fn binding_over_a_socd_key_asks_first_and_frees_the_partner() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.keys.insert((0x04, 8), 0x0008);
        p.keys.insert((0x07, 8), 0x0008);
        let socd = vec![0x04, 0x07, 0x04, 0x00, 0x07, 0x00, 0, 0, 0, 0];
        p.funcs.insert((0x2C, 0x04), socd.clone());
        p.funcs.insert((0x2C, 0x07), socd);
    }
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Macro");
    w.click("+ Baru");
    w.click("+ Tambah langkah");
    // Add a tap of the first offered key.
    w.click("Tekan + lepas");
    pick_target(&mut w, 0x04);
    w.idle();
    assert!(w.has_text("SOCD"), "{:?}", w.labels());
    w.click("Pasang ke tombol");
    assert!(w.has("Ya, ganti dengan macro"), "{:?}", w.labels());
    assert_eq!(
        w.sim().p().mode(0x04) & 0x0F,
        8,
        "nothing written before the confirm"
    );
    w.apply("Ya, ganti dengan macro");
    let s = w.sim();
    assert_eq!(s.p().mode(0x04) & 0x0F, 6);
    assert_eq!(s.p().mode(0x07) & 0x0F, 0, "partner freed");
    drop(s);
    w.idle();
    assert_eq!(
        w.snapshot().keys[&0x07].mode.advanced,
        openhe_proto::key::Advanced::None
    );
}

#[test]
fn library_duplicate_rename_and_delete_are_saved() {
    let mut w = recorded();
    w.click("Duplikat");
    assert_eq!(saved().macros.len(), 2);
    w.type_into_first_field("Salin dua");
    assert!(
        saved().macros.iter().any(|m| m.name == "Salin dua"),
        "{:?}",
        saved().macros
    );
    w.click("Hapus");
    w.click("Yakin hapus?");
    assert_eq!(saved().macros.len(), 1);
}

#[test]
fn the_fn_key_cannot_be_a_target() {
    let mut w = recorded();
    pick_target(&mut w, 0x01);
    assert!(w.has_text("Tombol Fn tidak bisa diberi macro."));
    assert!(!w.enabled("Pasang ke tombol"));
}

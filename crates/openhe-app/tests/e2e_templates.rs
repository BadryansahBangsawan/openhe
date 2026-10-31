//! Template end to end, on the simulator: a preset's preview and apply,
//! "Urungkan", "Kembali ke bawaan", saved templates, a bad import, and a
//! profile switch dropping a preview.
//!
//! Every wait is on observable state (page text, the page's preview, the
//! simulator), never a fixed number of frames.

mod support;
use openhe_app::templates::{store, Builtin, TemplateFile};
use support::sim::Reject;
use support::*;

const W: u8 = 0x1A;
const A: u8 = 0x04;
const S: u8 = 0x16;
const D: u8 = 0x07;
const Q: u8 = 0x14;
const CAPS: u8 = 0x39;
const H: u8 = 0x0B;
const SOCD: u8 = 0x2C;

const DONE: &str = "Tersimpan di keyboard";

fn templates(sim: Sim) -> World {
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Template");
    w
}

/// Picks a template tile and waits for its preview.
fn preview(w: &mut World, name: &str) {
    w.click(name);
    w.until("the preview", 20, |w| {
        w.app().templates().preview_keys().is_some()
    });
}

/// Clicks `label` and waits until the page reports `done` with the worker
/// idle (the apply runs a read, a backup and a write).
fn run_until(w: &mut World, label: &str, done: &str) {
    w.click(label);
    w.until(done, 30, |w| {
        w.has_text(done) && !w.worker.is_busy() && w.worker.snapshot().is_some()
    });
    w.idle();
}

fn touch(w: &World, key: u8) -> u16 {
    (w.sim().p().mode(key) >> 4) & 0x0F
}

fn nibble(w: &World, key: u8) -> u16 {
    w.sim().p().mode(key) & 0x0F
}

#[test]
fn gaming_fps_previews_then_writes_wasd_and_socd() {
    let mut w = templates(Sim::new());
    preview(&mut w, "Gaming FPS");
    let (changed, skipped) = w.app().templates().preview_keys().unwrap();
    assert_eq!(changed, vec![A, D, S, W]);
    assert!(skipped.is_empty());
    assert!(w.has_text("Mengubah 4 tombol"), "{:?}", w.labels());
    assert!(
        w.has_text("W, A, S, D → 0.40 mm, RT 0.15 mm"),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("SOCD A↔D, W↔S (input terakhir menang)"));
    assert_eq!(w.sim().set_count(), 0, "a preview writes nothing");

    run_until(&mut w, "Terapkan", DONE);
    for k in [W, A, S, D] {
        let s = w.sim();
        assert_eq!(s.p().key(k, 4), 400, "actuation of 0x{k:02X}");
        assert_eq!(s.p().key(k, 20), 150, "RT press of 0x{k:02X}");
        assert_eq!(s.p().key(k, 21), 150, "RT release of 0x{k:02X}");
        assert_eq!(s.p().key(k, 22), 200, "deadzone untouched");
        drop(s);
        assert_eq!(touch(&w, k), 2, "rapid trigger");
        assert_eq!(nibble(&w, k), 8, "SOCD");
    }
    {
        let s = w.sim();
        let ad = &s.p().funcs[&(SOCD, A)];
        assert_eq!(ad[..2], [A, D]);
        assert_eq!(ad[7], 0, "mode 0 = last input wins");
        assert_eq!(s.p().funcs[&(SOCD, W)][..2], [W, S]);
        // Other keys untouched.
        assert_eq!(s.p().key(Q, 4), 2000);
        assert_eq!(s.p().mode(Q), 0);
    }
    assert_eq!(w.snapshot().keys[&W].actuation.0, 400, "snapshot re-read");
    // The backup is listed for this keyboard.
    assert!(w.has_text("Sebelum Gaming FPS"), "{:?}", w.labels());
    assert!(w.has("Urungkan"));
}

#[test]
fn urungkan_restores_the_previous_values_exactly() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        // W on 0.55 mm Single with its own RT values; Q on a Black Emperor.
        p.keys.insert((W, 8), 0x0010);
        p.keys.insert((W, 4), 550);
        p.keys.insert((W, 20), 30);
        p.keys.insert((Q, 25), 2);
        p.keys.insert((CAPS, 0), 0x00E0);
    }
    let before = sim.state().p().keys.clone();
    let mut w = templates(sim);
    preview(&mut w, "Kompetitif (semua RT)");
    assert!(
        w.has_text("Semua tombol (68) → 0.60 mm, RT 0.20 mm"),
        "{:?}",
        w.labels()
    );
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(Q, 4), 600);
    assert_eq!(touch(&w, Q), 2);
    assert_ne!(w.sim().p().keys, before);

    // Urungkan previews the undo first; nothing is written until Terapkan.
    w.click("Urungkan");
    assert!(w.has_text("Urungkan Kompetitif"), "{:?}", w.labels());
    assert_eq!(w.sim().p().key(Q, 4), 600, "the preview writes nothing");
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().keys, before, "every key value back as it was");
    assert_eq!(w.snapshot().keys[&W].actuation.0, 550);
    // The undo was backed up too, so it can be undone in turn.
    assert!(
        w.has_text("Sebelum Urungkan Kompetitif"),
        "{:?}",
        w.labels()
    );
}

#[test]
fn urungkan_after_gaming_fps_clears_the_socd_pairs() {
    let sim = Sim::new();
    let before = sim.state().p().keys.clone();
    let mut w = templates(sim);
    preview(&mut w, "Gaming FPS");
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(nibble(&w, A), 8);
    w.click("Urungkan");
    assert!(w.has_text("Urungkan Gaming FPS"), "{:?}", w.labels());
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().keys, before);
    for k in [W, A, S, D] {
        assert_eq!(nibble(&w, k), 0);
        assert_eq!(touch(&w, k), 0, "back to Global");
    }
}

#[test]
fn kembali_ke_bawaan_resets_keys_layers_and_functions() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.keys.insert((Q, 8), 0x0010);
        p.keys.insert((Q, 4), 1000);
        p.keys.insert((Q, 22), 50);
        p.keys.insert((Q, 25), 2); // switch: kept
        p.keys.insert((W, 8), 0x0020);
        p.keys.insert((W, 4), 400);
        p.keys.insert((W, 20), 150);
        p.keys.insert((CAPS, 0), 0x00E0);
        p.keys.insert((H, 1), 0x0050);
        // END on G.
        p.keys.insert((0x0A, 8), 0x0005);
        p.funcs.insert((0x28, 0x0A), vec![0x0A, 0x3A, 0x00, 0, 0]);
    }
    let mut w = templates(sim);
    preview(&mut w, "Kembali ke bawaan");
    assert!(w.has_text("Fungsi lanjutan dihapus: G"), "{:?}", w.labels());
    run_until(&mut w, "Terapkan", DONE);
    let s = w.sim();
    let p = s.p();
    for (k, layout, want) in [
        (Q, 4, 2000),
        (Q, 20, 20),
        (Q, 21, 20),
        (Q, 22, 200),
        (Q, 23, 200),
        (Q, 25, 2),
        (W, 4, 2000),
        (W, 20, 20),
        (CAPS, 0, 0x0039),
        (H, 1, 0x0001),
        (0x01, 0, 0xF001),
    ] {
        assert_eq!(p.key(k, layout), want, "key 0x{k:02X} layout {layout}");
    }
    for k in [Q, W, 0x0A] {
        assert_eq!(p.mode(k), 0, "0x{k:02X}: Global, no function");
    }
    assert_eq!(s.factory_resets, 0, "never the factory reset command");
}

#[test]
fn a_backup_from_the_list_is_previewed_then_restored() {
    let sim = Sim::new();
    let before = sim.state().p().keys.clone();
    let mut w = templates(sim);
    preview(&mut w, "Caps jadi Ctrl");
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(CAPS, 0), 0x00E0);
    preview(&mut w, "Panah ala Vim (Fn1)");
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(H, 1), 0x0050);
    // Newest first: "Sebelum Panah…", then "Sebelum Caps…". Restore the
    // older one: back to before both.
    w.click_nth("Pulihkan", 1);
    w.until("the restore preview", 20, |w| {
        w.app().templates().preview_keys().is_some()
    });
    assert!(w.has_text("Mengubah 5 tombol"), "{:?}", w.labels());
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().keys, before);
}

#[test]
fn a_bad_import_shows_an_error_and_writes_nothing() {
    let mut w = templates(Sim::new());
    let dir = std::env::temp_dir().join(format!("openhe-e2e-tpl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let keys: Vec<u8> = w.snapshot().matrix.keys().map(|(_, _, c)| c).collect();
    let good = TemplateFile::from_template(
        &Builtin::GamingFps.template(openhe_proto::Model::Win68HeMax, &keys),
    );
    let mut wrong_model = good.clone();
    wrong_model.model = "WIN 60 HE MAX".into();
    let mut too_low = good.clone();
    too_low.performance[0].actuation_um = Some(20);
    let cases = [
        (
            "broken.openhe-template",
            "{ not json".to_string(),
            "bukan JSON",
        ),
        (
            "profile.openhe-template",
            r#"{"format": "openhe-profile", "version": 1}"#.to_string(),
            "bukan template OpenHE",
        ),
        (
            "newer.openhe-template",
            good.to_json().replace("\"version\": 1", "\"version\": 7"),
            "OpenHE yang lebih baru",
        ),
        (
            "win60.openhe-template",
            wrong_model.to_json(),
            "Template ini untuk keyboard lain",
        ),
        (
            "low.openhe-template",
            too_low.to_json(),
            "nilai yang tidak valid",
        ),
    ];
    for (name, text, error) in cases {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        w.app_mut().import_template(&path);
        w.until(error, 20, |w| w.has_text(error) && !w.worker.is_busy());
        assert!(w.app().templates().preview_keys().is_none(), "{name}");
        assert_eq!(w.sim().set_count(), 0, "{name}: nothing written");
    }
    // A good file previews (and still writes nothing until "Terapkan").
    let path = dir.join("good.openhe-template");
    std::fs::write(&path, good.to_json()).unwrap();
    w.app_mut().import_template(&path);
    w.until("the preview", 20, |w| {
        w.app().templates().preview_keys().is_some()
    });
    assert!(w.has("Simpan ke Template saya"));
    assert_eq!(w.sim().set_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_profile_switch_drops_a_pending_preview() {
    let mut w = templates(Sim::new());
    preview(&mut w, "Gaming FPS");
    let c = w.worker.connects();
    w.click("2");
    w.until("reconnected on profile 2", 30, |w| {
        w.worker.connects() > c
            && !w.worker.is_restarting()
            && w.worker.snapshot().map(|s| s.profile) == Some(1)
    });
    w.idle();
    assert!(w.app().templates().preview_keys().is_none());
    assert!(w.has_text("pratinjau dibatalkan"), "{:?}", w.labels());
    assert!(!w.has("Terapkan"));
    let s = w.sim();
    assert!(
        s.sets(0x23).is_empty() && s.sets(SOCD).is_empty(),
        "nothing written"
    );
}

#[test]
fn settings_saved_as_a_template_are_listed_and_apply_back() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.keys.insert((W, 8), 0x0020);
        p.keys.insert((W, 4), 700);
        p.keys.insert((CAPS, 0), 0x00E0);
    }
    let mut w = templates(sim);
    w.click("Simpan setting sekarang…");
    w.type_in_last_field("Kerja");
    // Remap and functions only: performance left out.
    // The checkbox, not the sidebar entry of the same name.
    let n = w.h_count("Performa");
    w.click_nth("Performa", n - 1);
    w.click("Simpan");
    w.until("saved", 20, |w| w.has_text("Template disimpan."));
    assert!(
        w.has_text("Remap · Tombol lanjutan · WIN 68 HE MAX"),
        "{:?}",
        w.labels()
    );
    let base = openhe_app::storage::AppData::default_dir().unwrap();
    let saved = store::list_templates(&base);
    assert_eq!(saved.len(), 1);
    assert!(saved[0].template.performance.is_empty());
    // Change the keyboard, then apply the saved template.
    {
        let mut s = w.sim();
        let p = s.p_mut();
        p.keys.insert((CAPS, 0), 0x0039);
        p.keys.insert((W, 4), 900);
    }
    w.worker.request_reload();
    w.idle();
    w.click("Pratinjau");
    w.until("the preview", 20, |w| {
        w.app().templates().preview_keys().is_some()
    });
    assert_eq!(
        w.app().templates().preview_keys().unwrap().0,
        vec![CAPS],
        "only the remap differs"
    );
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(CAPS, 0), 0x00E0);
    assert_eq!(w.sim().p().key(W, 4), 900, "performance was not in it");
}

/// "Urungkan" puts back only what the template wrote: a key changed
/// elsewhere since is named in its preview (and put back), a key the
/// template did not touch keeps its later change.
#[test]
fn urungkan_names_later_changes_and_leaves_other_keys_alone() {
    let sim = Sim::new();
    let w_before = sim.state().p().key(W, 4);
    let mut w = templates(sim);
    preview(&mut w, "Gaming FPS");
    run_until(&mut w, "Terapkan", DONE);
    // Changed on another page (or by another program) since.
    {
        let mut s = w.sim();
        let p = s.p_mut();
        p.keys.insert((W, 4), 900);
        p.keys.insert((Q, 4), 900);
    }
    w.click("Urungkan");
    w.until("the undo preview", 20, |w| {
        w.app().templates().preview_keys().is_some()
    });
    assert!(
        w.has_text("Sejak template diterapkan, tombol ini diubah lagi: W."),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("Hanya yang diubah template tadi yang dikembalikan"));
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(W, 4), w_before);
    assert_eq!(w.sim().p().key(Q, 4), 900, "not the template's: kept");
    assert_eq!(nibble(&w, A), 0);
}

/// A backup is applied only on the profile it was read from.
#[test]
fn a_backup_of_another_profile_is_refused() {
    let mut w = templates(Sim::new());
    preview(&mut w, "Caps jadi Ctrl");
    run_until(&mut w, "Terapkan", DONE);
    let c = w.worker.connects();
    w.click("2");
    w.until("reconnected on profile 2", 30, |w| {
        w.worker.connects() > c
            && !w.worker.is_restarting()
            && w.worker.snapshot().map(|s| s.profile) == Some(1)
    });
    w.idle();
    let on_2 = w.sim().profiles[1].keys.clone();
    let sets = w.sim().sets(0x23).len();
    w.click("Pulihkan");
    w.frames(3);
    assert!(w.has_text("Cadangan dari profil lain"), "{:?}", w.labels());
    assert!(
        w.has_text("Cadangan ini dibaca dari Profil 1, sedangkan yang aktif Profil 2."),
        "{:?}",
        w.labels()
    );
    assert!(w.app().templates().preview_keys().is_none());
    w.idle();
    assert_eq!(w.sim().profiles[1].keys, on_2, "nothing written");
    assert_eq!(w.sim().sets(0x23).len(), sets);
}

/// A key that cannot be read does not stop a preview, an apply or "Simpan
/// setting sekarang": it is left out and named.
#[test]
fn an_unreadable_key_is_left_out_and_named() {
    let mut w = templates(Sim::new());
    // Every read of Q is refused from now on.
    w.sim().rejects.push(Reject {
        cmd: 0x23,
        sets_only: false,
        at: vec![(0, 0x00), (1, Q)],
        times: u32::MAX,
    });
    preview(&mut w, "Kompetitif (semua RT)");
    assert!(
        w.has_text("Dilewati, belum terbaca dari keyboard: Q"),
        "{:?}",
        w.labels()
    );
    let (_, skipped) = w.app().templates().preview_keys().unwrap();
    assert_eq!(skipped, vec![Q]);
    run_until(&mut w, "Terapkan", DONE);
    assert_eq!(w.sim().p().key(W, 4), 600);
    assert_eq!(w.sim().p().key(Q, 4), 2000, "never written");
    assert_eq!(touch(&w, Q), 0);

    w.click("Simpan setting sekarang…");
    w.type_in_last_field("Tanpa Q");
    w.click("Simpan");
    w.until("saved", 20, |w| w.has_text("Template disimpan."));
    assert!(
        w.has_text("Tidak terbaca, jadi tidak ikut disimpan: Q."),
        "{:?}",
        w.labels()
    );
}

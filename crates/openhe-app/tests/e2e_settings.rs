//! Pengaturan end to end: polling (re-enumeration), Win/Mac, profile
//! switches from the top bar and the cards, export/import and the factory
//! reset.

mod support;
use openhe_app::pages::settings::profile_file::{export_job, import_job, plan_import, ProfileFile};
use openhe_app::worker::Pending;
use openhe_device::Error;
use openhe_proto::{PollingRate, SystemMode};
use support::*;

fn settings() -> World {
    let mut w = World::new();
    w.page("Pengaturan");
    w
}

fn wait<R>(w: &mut World, mut p: Pending<R>) -> Result<R, Error> {
    let mut out = None;
    w.until("job", 30, |_| {
        out = p.poll();
        out.is_some()
    });
    out.unwrap()
}

fn reconnected(w: &mut World, connects: u64) {
    w.until("reconnected", 30, |w| {
        w.worker.connects() > connects && !w.worker.is_restarting()
    });
    w.idle();
}

#[test]
fn polling_change_reenumerates_reconnects_and_is_verified() {
    let mut w = settings();
    let c = w.worker.connects();
    w.click("1K");
    reconnected(&mut w, c);
    assert_eq!(w.sim().p().polling, 3);
    assert_eq!(w.sim().reenumerations, 1);
    assert_eq!(w.snapshot().polling, PollingRate::Hz1000);
    assert!(
        w.has_text("Polling rate sekarang 1000 Hz."),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("=1000 Hz"), "device card: {:?}", w.labels());
}

#[test]
fn polling_write_that_never_left_is_reported_as_not_sent() {
    // The handle went stale: the SET fails before it reaches the keyboard,
    // so nothing restarts and nothing changed.
    let mut w = settings();
    w.sim().drop_before = Some((0x00, 0x50));
    let c = w.worker.connects();
    w.click("1K");
    w.until("reconnected", 30, |w| w.worker.connects() > c);
    w.idle();
    assert_eq!(w.sim().p().polling, 0);
    assert_eq!(w.sim().reenumerations, 0);
    assert!(
        w.has_text("Perintah tidak terkirim."),
        "a write that was never sent must not wait for a restart: {:?}",
        w.labels()
    );
}

#[test]
fn win_mac_switch_from_the_page() {
    let mut w = settings();
    w.click("macOS");
    w.idle();
    assert_eq!(w.sim().p().win, 0);
    assert_eq!(w.snapshot().system_mode, Some(SystemMode::Mac));
    // The firmware swaps LAlt / LWin on Base.
    assert_eq!(w.sim().p().key(0xE2, 0), 0xE3);
    assert!(w.selected("Mac"), "top bar follows");
    w.click("Win");
    w.idle();
    assert_eq!(w.sim().p().win, 1);
    assert_eq!(w.snapshot().system_mode, Some(SystemMode::Windows));
}

#[test]
fn profile_switch_from_the_top_bar_reads_the_new_profiles_mode() {
    let sim = Sim::new();
    sim.state().profiles[2].win = 0; // profile 3 is in Mac mode
    let mut w = World::with(sim, Default::default(), fast_config());
    let c = w.worker.connects();
    w.click("3");
    reconnected(&mut w, c);
    assert_eq!(w.sim().profile, 2);
    assert_eq!(w.snapshot().profile, 2);
    assert_eq!(w.snapshot().system_mode, Some(SystemMode::Mac));
    assert!(w.selected("3"));
    assert!(w.selected("Mac"));
}

#[test]
fn profile_switch_from_the_cards() {
    let mut w = settings();
    let c = w.worker.connects();
    let r = w.rect("Profil 4");
    w.click_at(r.left_top() + eframe::egui::vec2(20.0, 12.0));
    reconnected(&mut w, c);
    assert_eq!(w.sim().profile, 3);
    assert!(w.selected("Profil 4"), "{:?}", w.labels());
    assert!(
        w.has_text("Keyboard tersambung lagi dengan Profil 4."),
        "{:?}",
        w.labels()
    );
}

#[test]
fn factory_reset_needs_the_typed_word_and_resets_the_keyboard() {
    let sim = Sim::new();
    sim.state().p_mut().keys.insert((0x14, 4), 1000);
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Pengaturan");
    w.click("Reset keyboard…");
    w.type_into_first_field("reset");
    assert!(!w.enabled("Reset sekarang"), "lower case must not do");
    w.type_into_first_field("RESET");
    assert!(w.enabled("Reset sekarang"));
    let c = w.worker.connects();
    w.click("Reset sekarang");
    reconnected(&mut w, c);
    assert_eq!(w.sim().factory_resets, 1);
    assert_eq!(w.sim().p().key(0x14, 4), 2000);
    assert_eq!(w.snapshot().keys[&0x14].actuation.0, 2000);
    assert!(
        w.has_text("Keyboard sudah direset ke bawaan pabrik"),
        "{:?}",
        w.labels()
    );
}

#[test]
fn export_modify_import_round_trip() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.keys.insert((0x1A, 8), 0x0020);
        p.keys.insert((0x1A, 4), 400);
        p.keys.insert((0x39, 0), 0x0029);
        p.keys.insert((0x0A, 8), 0x0005);
        p.funcs.insert((0x28, 0x0A), vec![0x0A, 0x3A, 0x00, 0, 0]);
        p.krgb.insert(0x04, [1, 2, 3]);
    }
    let mut w = World::with(sim, Default::default(), fast_config());
    let keys: Vec<u8> = w.snapshot().matrix.keys().map(|(_, _, c)| c).collect();
    let p = w.worker.submit("export", move |kb| export_job(kb, &keys));
    let exported = wait(&mut w, p).expect("export");
    assert!(exported.skipped.is_empty(), "{:?}", exported.skipped);
    let json = ProfileFile::from_data(&exported.data).to_json();
    // Modify the file: W's actuation 0.40 → 0.55.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let mut hit = false;
    for k in v["keys"].as_array_mut().unwrap() {
        if k["code"] == 0x1A {
            k["actuation_um"] = serde_json::json!(550);
            hit = true;
        }
    }
    assert!(hit, "W not found in the file: {json}");
    // Then wipe the keyboard back to factory and import the file.
    *w.sim().p_mut() = sim::Profile::factory();
    w.worker.request_reload();
    w.idle();
    let data = ProfileFile::from_json(&v.to_string())
        .and_then(|f| f.to_data())
        .expect("parse");
    let plan = plan_import(&data, &w.snapshot()).expect("plan");
    let p = w.worker.submit("import", move |kb| import_job(kb, &plan));
    let report = wait(&mut w, p).expect("import");
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let s = w.sim();
    assert_eq!(s.p().key(0x1A, 4), 550);
    assert_eq!(s.p().mode(0x1A), 0x0020);
    assert_eq!(s.p().key(0x39, 0), 0x0029);
    assert_eq!(s.p().mode(0x0A) & 0x0F, 5);
    assert_eq!(s.p().funcs[&(0x28, 0x0A)][1..3], [0x3A, 0x00]);
    assert_eq!(s.p().krgb.get(&0x04), Some(&[1, 2, 3]));
    assert_eq!(s.p().lighting[37], 0xC1, "flags kept");
}

#[test]
fn import_across_win_and_mac_keeps_alt_and_win_in_place() {
    // Exported in Mac mode: Base LAlt reads 0xE3 and LWin 0xE2.
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.win = 0;
        p.keys.insert((0xE2, 0), 0xE3);
        p.keys.insert((0xE3, 0), 0xE2);
    }
    let mut w = World::with(sim, Default::default(), fast_config());
    let keys: Vec<u8> = w.snapshot().matrix.keys().map(|(_, _, c)| c).collect();
    let p = w.worker.submit("export", move |kb| export_job(kb, &keys));
    let json = ProfileFile::from_data(&wait(&mut w, p).unwrap().data).to_json();
    // Imported on a keyboard in Windows mode.
    *w.sim().p_mut() = sim::Profile::factory();
    w.worker.request_reload();
    w.idle();
    let data = ProfileFile::from_json(&json)
        .and_then(|f| f.to_data())
        .unwrap();
    let plan = plan_import(&data, &w.snapshot()).unwrap();
    let p = w.worker.submit("import", move |kb| import_job(kb, &plan));
    assert!(wait(&mut w, p).unwrap().failed.is_empty());
    let s = w.sim();
    assert_eq!(s.p().key(0xE2, 0), 0xE2, "LAlt still Alt in Windows mode");
    assert_eq!(s.p().key(0xE3, 0), 0xE3, "LWin still Win in Windows mode");
}

#[test]
fn import_refuses_0x0001_on_base() {
    let mut w = World::new();
    let keys: Vec<u8> = w.snapshot().matrix.keys().map(|(_, _, c)| c).collect();
    let p = w.worker.submit("export", move |kb| export_job(kb, &keys));
    let mut data = wait(&mut w, p).unwrap().data;
    let caps = data.keys.iter_mut().find(|k| k.code == 0x39).unwrap();
    caps.layers[0] = 0x0001;
    let err = plan_import(&data, &w.snapshot()).expect_err("0x0001 on Base accepted");
    let text = openhe_app::pages::settings::file_error_text(&err);
    assert!(text.contains("0x0001"), "{text}");
    assert_eq!(w.sim().set_count(), 0);
}

#[test]
fn launch_at_login_shows_the_os_state_not_the_saved_flag() {
    // The data file says "on", but the (test) home has no autostart entry,
    // e.g. after a recovered data file or a moved app.
    let mut data = openhe_app::storage::AppData::default();
    data.launch_at_login = true;
    let mut w = World::with(Sim::new(), data, fast_config());
    w.page("Pengaturan");
    // Checkboxes: per-app switching, then launch at login (then the two
    // of Pembaruan).
    assert_eq!(
        w.role_count(eframe::egui::accesskit::Role::CheckBox),
        4,
        "{:?}",
        w.labels()
    );
    assert!(!w.role_checked(eframe::egui::accesskit::Role::CheckBox, 1));
}

#[test]
fn a_restart_result_goes_once_the_profile_changes_elsewhere() {
    let mut w = settings();
    let c = w.worker.connects();
    let r = w.rect("Profil 3");
    w.click_at(r.left_top() + eframe::egui::vec2(20.0, 12.0));
    reconnected(&mut w, c);
    let done = "Keyboard tersambung lagi dengan Profil 3.";
    assert!(w.has_text(done), "{:?}", w.labels());
    // Back to profile 1 from the top bar: the card's note is stale.
    let c = w.worker.connects();
    w.click("1");
    reconnected(&mut w, c);
    assert_eq!(w.sim().profile, 0);
    assert!(!w.has_text(done), "{:?}", w.labels());
}

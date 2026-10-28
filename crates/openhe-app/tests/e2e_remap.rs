//! Remap end to end on the simulated keyboard: every layer, the picker
//! search, the pending list, Terapkan, the layer resets and the 0x0001 /
//! Fn2 warnings.

mod support;
use eframe::egui::accesskit::Role;
use support::sim::{factory_layer, Reject};
use support::*;

const CAPS: u8 = 0x39;
const Q: u8 = 0x14;
const A: u8 = 0x04;
const LWIN: u8 = 0xE3;
const LALT: u8 = 0xE2;
const RCTRL: u8 = 0xE4;
const FN: u8 = 0x01;

fn remap() -> World {
    let mut w = World::new();
    w.page("Remap");
    w.idle();
    w
}

/// Types into the picker's search box and clicks the chip `chip`.
fn search_pick(w: &mut World, search: &str, chip: &str) {
    w.click_role(Role::TextInput, 0);
    w.replace_text(search);
    let n = w.h_count(chip);
    assert!(n > 0, "no chip {chip:?}: {:?}", w.labels());
    w.click_nth(chip, n - 1);
}

#[test]
fn base_remap_through_search_pending_list_and_terapkan() {
    let mut w = remap();
    w.click_key(CAPS, false);
    search_pick(&mut w, "esc", "ESC");
    // The pending list shows the change before anything is written.
    assert!(w.has_text("Caps · Base"), "{:?}", w.labels());
    assert!(w.has_text("Cap → ESC"), "{:?}", w.labels());
    assert_eq!(w.sim().sets(0x23).len(), 0);
    w.apply("Terapkan");
    assert_eq!(w.sim().p().key(CAPS, 0), 0x29);
    assert!(w.has("Tersimpan di keyboard"), "{:?}", w.labels());
    assert!(w.has_text("Belum ada perubahan"));
    // Discarding pending changes writes nothing.
    w.click_key(Q, false);
    search_pick(&mut w, "esc", "ESC");
    let sets = w.sim().set_count();
    w.click("Batalkan");
    assert_eq!(w.sim().set_count(), sets);
    assert!(w.has_text("Belum ada perubahan"));
}

#[test]
fn every_fn_layer_is_written_to_its_own_layout() {
    let mut w = remap();
    for (layer, layout, chip, code) in [
        ("Fn1", 1u8, "F5", 0x3Eu16),
        ("Fn2", 2, "F6", 0x3F),
        ("Fn3", 3, "F7", 0x40),
    ] {
        w.click(layer);
        w.idle();
        w.click_key(Q, false);
        search_pick(&mut w, &chip.to_lowercase(), chip);
        w.apply("Terapkan");
        assert_eq!(w.sim().p().key(Q, layout), code, "{layer}");
    }
    // Base untouched.
    assert_eq!(w.sim().p().key(Q, 0), u16::from(Q));
}

#[test]
fn fn_layer_offers_follow_base_and_base_does_not() {
    let mut w = remap();
    w.click_key(Q, false);
    w.click_role(Role::TextInput, 0);
    w.replace_text("ikut");
    assert!(
        !w.has("Ikut Base (transparan)"),
        "Base must not offer 0x0001"
    );
    w.click("Fn1");
    w.idle();
    w.click_key(A, false);
    search_pick(&mut w, "ikut", "Ikut Base (transparan)");
    w.apply("Terapkan");
    assert_eq!(w.sim().p().key(A, 1), 0x0001);
}

#[test]
fn base_reset_skips_win_alt_and_fn() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        p.keys.insert((CAPS, 0), 0x29);
        p.keys.insert((Q, 0), 0x04);
        // Win and Alt swapped (as in Mac mode): reset must not touch them.
        p.keys.insert((LWIN, 0), 0xE2);
        p.keys.insert((LALT, 0), 0xE3);
    }
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Remap");
    w.idle();
    w.click("Kembalikan ke bawaan");
    w.apply("Ya, kembalikan");
    let s = w.sim();
    assert_eq!(s.p().key(CAPS, 0), u16::from(CAPS));
    assert_eq!(s.p().key(Q, 0), u16::from(Q));
    assert_eq!(s.p().key(LWIN, 0), 0xE2, "Win skipped");
    assert_eq!(s.p().key(LALT, 0), 0xE3, "Alt skipped");
    assert_eq!(s.p().key(FN, 0), 0xF001, "Fn untouched");
    assert!(
        !s.sets(0x23).iter().any(|p| p[1..]
            .chunks(4)
            .any(|e| e[1] == 0 && matches!(e[0], LWIN | LALT | FN))),
        "a Base write touched Win, Alt or Fn"
    );
}

#[test]
fn fn1_reset_writes_the_factory_table() {
    let sim = Sim::new();
    {
        let mut s = sim.state();
        let p = s.p_mut();
        for k in sim::keys() {
            p.keys.insert((k, 1), 0x0005);
        }
        // Keep one Fn2 hold so the layer is reachable.
        p.keys.insert((RCTRL, 1), 0xF002);
    }
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Remap");
    w.idle();
    w.click("Fn1");
    w.idle();
    w.click("Kembalikan ke bawaan");
    w.apply("Ya, kembalikan");
    let s = w.sim();
    for k in sim::keys() {
        assert_eq!(s.p().key(k, 1), factory_layer(k, 1), "Fn1 0x{k:02X}");
    }
}

#[test]
fn transparent_on_base_is_flagged() {
    let sim = Sim::new();
    sim.state().p_mut().keys.insert((CAPS, 0), 0x0001);
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Remap");
    w.idle();
    w.click_key(CAPS, false);
    assert!(
        w.has_text("Kode 0x0001 — bukan Fn"),
        "the right panel explains 0x0001 on Base: {:?}",
        w.labels()
    );
}

#[test]
fn removing_the_last_fn2_hold_warns() {
    let mut w = remap();
    w.click("Fn1");
    w.idle();
    w.click_key(RCTRL, false);
    assert!(w.has_text("satu-satunya Fn2 (tahan)"), "{:?}", w.labels());
    search_pick(&mut w, "f5", "F5");
    assert!(
        w.has_text("layer Fn2 tidak bisa dibuka lagi"),
        "{:?}",
        w.labels()
    );
}

#[test]
fn removing_fn2_hold_warns_also_when_fn1_was_read_only_partly() {
    // RCtrl's Fn1 code cannot be read: the page does not know that RCtrl
    // holds the only Fn2, so the warning must err on the safe side.
    let sim = Sim::new();
    sim.state().rejects.push(Reject {
        cmd: 0x23,
        sets_only: false,
        at: vec![(0, 0), (1, RCTRL), (2, 1)],
        times: u32::MAX,
    });
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Remap");
    w.idle();
    w.click("Fn1");
    w.idle();
    assert!(
        w.has_text("Sebagian tombol tidak terbaca"),
        "{:?}",
        w.labels()
    );
    w.click_key(RCTRL, false);
    search_pick(&mut w, "f5", "F5");
    assert!(
        w.has_text("layer Fn2 tidak bisa dibuka lagi"),
        "unread Fn1 key replaced without a warning: {:?}",
        w.labels()
    );
}

#[test]
fn the_fn_key_is_locked_on_base() {
    let mut w = remap();
    w.click_key(FN, false);
    assert!(
        w.has_text("Kunci") || w.has_text("terkunci"),
        "{:?}",
        w.labels()
    );
    w.click_role(Role::TextInput, 0);
    w.replace_text("esc");
    let n = w.h_count("ESC");
    if n > 0 {
        assert!(
            !w.enabled_nth("ESC", n - 1),
            "picker enabled for the Fn key"
        );
    }
}

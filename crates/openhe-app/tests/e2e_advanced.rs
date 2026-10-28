//! Tombol Lanjutan end to end on the simulated keyboard: create, edit and
//! clear each function type, SOCD partner cleanup and the Fn guard.

mod support;
use support::*;

const G: u8 = 0x0A;
const H: u8 = 0x0B;
const A: u8 = 0x04;
const S: u8 = 0x16;
const D: u8 = 0x07;

fn nibble(w: &World, key: u8) -> u16 {
    w.sim().p().mode(key) & 0x0F
}

fn open(key: u8) -> World {
    let mut w = World::new();
    w.page("Tombol Lanjutan");
    w.click_key(key, false);
    w.frames(3);
    w
}

#[test]
fn mt_create_edit_clear() {
    let mut w = open(G);
    w.click("MT");
    w.pick_code("Kosong", 0, "f1", "F1");
    w.pick_code("Kosong", 0, "esc", "ESC");
    w.apply("Terapkan");
    assert!(w.has("Tersimpan di keyboard"), "{:?}", w.labels());
    assert_eq!(nibble(&w, G), 3);
    {
        let s = w.sim();
        assert_eq!(s.p().key(G, 9), 0x3A, "hold");
        assert_eq!(s.p().key(G, 10), 0x29, "tap");
        assert_eq!(s.p().key(G, 19), 20, "200 ms in 10 ms units");
    }
    // The table lists it.
    assert!(w.has("G · MT"), "{:?}", w.labels());
    // Edit: a different hold key and delay.
    w.pick_code("F1", 0, "b", "B");
    w.edit_value("200 ms", 0, "350");
    w.apply("Terapkan");
    {
        let s = w.sim();
        assert_eq!(s.p().key(G, 9), 0x05);
        assert_eq!(s.p().key(G, 19), 35);
    }
    // Clear.
    w.apply("Hapus");
    assert_eq!(nibble(&w, G), 0);
    assert!(!w.has("G · MT"));
    assert_eq!(
        w.snapshot().keys[&G].mode.advanced,
        openhe_proto::key::Advanced::None
    );
}

#[test]
fn tgl_and_end_and_type_change() {
    let mut w = open(G);
    w.click("TGL");
    w.pick_code("Kosong", 0, "esc", "ESC");
    w.apply("Terapkan");
    assert_eq!(nibble(&w, G), 4);
    assert_eq!(w.sim().p().funcs[&(0x25, G)][1..3], [0x29, 0x00]);
    // Change the type to END on the same key.
    w.click("END");
    w.pick_code("Kosong", 0, "f1", "F1");
    w.apply("Terapkan");
    assert_eq!(nibble(&w, G), 5);
    assert_eq!(w.sim().p().funcs[&(0x28, G)][1..3], [0x3A, 0x00]);
    assert!(w.has("G · END"), "{:?}", w.labels());
    w.apply("Hapus");
    assert_eq!(nibble(&w, G), 0);
}

#[test]
fn mpt_three_depths() {
    let mut w = open(H);
    w.click("MPT");
    for (i, (search, chip)) in [("f1", "F1"), ("esc", "ESC"), ("b", "B")]
        .iter()
        .enumerate()
    {
        let _ = i;
        w.pick_code("Kosong", 0, search, chip);
    }
    w.apply("Terapkan");
    assert_eq!(nibble(&w, H), 2);
    let f = w.sim().p().funcs[&(0x27, H)].clone();
    let le = |i: usize| u16::from_le_bytes([f[i], f[i + 1]]);
    assert_eq!([le(1), le(3), le(5)], [0x3A, 0x29, 0x05]);
    assert_eq!([le(7), le(9), le(11)], [1000, 2000, 3000]);
}

#[test]
fn dks_bindings_and_phases() {
    let mut w = open(H);
    w.click("DKS");
    w.pick_code("Kosong", 0, "f1", "F1");
    // Press at the start, hold through the bottom.
    w.click("DKS 1 · Mulai tekan");
    w.apply("Terapkan");
    assert_eq!(nibble(&w, H), 1);
    let s = w.sim();
    assert_eq!(s.p().key(H, 9), 0x3A);
    assert_eq!(s.p().key(H, 13), 0x01, "trps 1 = press at P0");
    assert_eq!(s.p().key(H, 5), 1400, "default first actuation");
}

#[test]
fn socd_pair_and_partner_cleanup() {
    let mut w = open(A);
    w.click("SOCD");
    w.click("Pilih di keyboard");
    w.click_key(D, false);
    w.apply("Terapkan");
    assert_eq!(nibble(&w, A), 8);
    assert_eq!(nibble(&w, D), 8);
    assert!(w.has("A ↔ D · SOCD"), "{:?}", w.labels());
    // S takes D as its partner: A loses its SOCD (no half pair left).
    w.click_key(S, false);
    w.frames(3);
    w.click("SOCD");
    w.click("Pilih di keyboard");
    w.click_key(D, false);
    w.apply("Terapkan");
    assert_eq!(nibble(&w, S), 8);
    assert_eq!(nibble(&w, D), 8);
    assert_eq!(nibble(&w, A), 0, "old partner cleared");
    assert!(!w.has("A ↔ D · SOCD"), "{:?}", w.labels());
    // Clearing S clears D as well.
    w.click_key(S, false);
    w.frames(3);
    w.apply("Hapus");
    assert_eq!(nibble(&w, S), 0);
    assert_eq!(nibble(&w, D), 0);
}

#[test]
fn the_fn_key_cannot_get_a_function() {
    let w = open(0x01);
    assert!(w.has_text("Tombol Fn tidak bisa diberi fungsi lanjutan."));
    assert!(!w.enabled("Terapkan"));
    assert_eq!(w.sim().sets(0x23).len(), 0);
}

#[test]
fn macro_key_shows_macro_hint_and_can_be_replaced() {
    // A macro bound outside the app (mode nibble 6).
    let sim = Sim::new();
    sim.state().p_mut().keys.insert((G, 8), 0x0006);
    let mut w = World::with(sim, Default::default(), fast_config());
    w.page("Tombol Lanjutan");
    w.click_key(G, false);
    w.frames(3);
    assert!(w.has_text("Macro"), "{:?}", w.labels());
    w.click("END");
    w.pick_code("Kosong", 0, "esc", "ESC");
    w.apply("Terapkan");
    assert_eq!(nibble(&w, G), 5);
}

/// Selecting another key while an apply runs must not leave the applied
/// key's form under the new key's title (Terapkan then wrote it there).
#[test]
fn selecting_another_key_during_an_apply_keeps_that_keys_own_editor() {
    let mut w = open(H);
    w.click("MT");
    w.pick_code("Kosong", 0, "f1", "F1");
    w.pick_code("Kosong", 0, "esc", "ESC");
    w.apply("Terapkan");
    assert_eq!(w.sim().p().key(H, 9), 0x3A);
    // G gets MT B/C on a slow keyboard; H is clicked while it is written.
    w.click_key(G, false);
    w.frames(3);
    w.click("MT");
    w.pick_code("Kosong", 0, "b", "B");
    w.pick_code("Kosong", 0, "c", "C");
    w.sim().set_delay = std::time::Duration::from_millis(30);
    w.click("Terapkan");
    assert!(w.worker.is_busy());
    w.click_key(H, false);
    w.idle();
    w.sim().set_delay = std::time::Duration::ZERO;
    w.idle();
    assert_eq!(w.sim().p().key(G, 9), 0x05, "G written");
    // The panel edits H with H's own codes.
    assert!(w.has("F1"), "{:?}", w.labels());
    assert!(!w.has("B"), "G's form shown for H: {:?}", w.labels());
    // Applying now leaves H as it was.
    w.apply("Terapkan");
    let s = w.sim();
    assert_eq!(s.p().key(H, 9), 0x3A, "H hold");
    assert_eq!(s.p().key(H, 10), 0x29, "H tap");
}

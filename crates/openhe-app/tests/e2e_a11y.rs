//! Custom-painted controls are visible to screen readers (AccessKit), which
//! is also how the end-to-end tests find them.

mod support;
use support::*;

#[test]
fn sidebar_pages_keys_and_segments_have_accessibility_nodes() {
    let mut w = World::new();
    for page in [
        "Performa",
        "Tombol Lanjutan",
        "Remap",
        "Macro",
        "Lampu",
        "Kalibrasi",
        "Pengaturan",
    ] {
        assert!(w.has(page), "sidebar item {page:?}: {:?}", w.labels());
    }
    assert!(w.selected("Performa"), "the open page is marked selected");
    // Every one of the 68 keys is its own node, named like its hover text.
    for k in sim::keys() {
        let tag = format!("· 0x{k:02X}");
        assert!(
            w.labels().iter().any(|l| l.ends_with(&tag)),
            "key 0x{k:02X}"
        );
    }
    // Clicking a key node selects that key.
    w.click_key(0x1A, false);
    assert!(w.key_selected(0x1A));
    assert!(w.has_text("1 tombol dipilih"));
    // Segmented options are selectable nodes.
    assert!(w.selected("Normal"));
    w.click("Rapid Trigger");
    assert!(w.selected("Rapid Trigger"));
    // Sidebar navigation works through the node.
    w.page("Remap");
    assert!(w.selected("Remap"));
}

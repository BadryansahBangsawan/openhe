//! Remap: change what each key sends, on the Base and the three Fn layers.
//!
//! Mockup `docs/design/mockups/03-remap.png`. The user selects keys on the
//! keyboard and clicks a replacement in the key picker; that only queues a
//! pending change. "Terapkan" sends all pending changes in one worker job
//! (`Keyboard::set_layer_codes`). The Fn key on the Base layer is locked: the
//! UI never queues a change for it, and "Kembalikan ke bawaan" skips it.
//!
//! The codes of a layer are read for all keys by one job when its tab is
//! opened and cached per layer. After a failed write the written keys are read
//! again; when that fails too the cache is dropped and a reload requested.
//!
//! The key picker ([`picker_ui`], [`code_field`]) and the Indonesian key names
//! ([`code_name`]) are also used by the Tombol Lanjutan page.

use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{
    legend_list, reference_matrix, KeyDecoration, KeyboardView, Selection,
};
use crate::widgets::segmented::Segmented;
use crate::worker::Pending;
use eframe::egui::{
    self, Align, CornerRadius, Frame, Layout, Margin, Popup, PopupCloseBehavior, RichText, Ui,
};
use openhe_device::{Error, Keyboard, Transport};
use openhe_proto::keycodes::{self, Category, KeyCode};
use openhe_proto::{Layer, Model};
use std::collections::{BTreeMap, BTreeSet};

mod strings {
    pub const LAYER: &str = "Layer";
    pub const HINT_BASE: &str = "Layer utama · tombol Fn terkunci";
    pub const HINT_FN: &str = "Aktif saat Fn ditahan · tombol kosong = ikut Base";
    pub const RESET: &str = "Kembalikan ke bawaan";
    pub const RESET_BASE_ONLY: &str =
        "Hanya untuk layer Base; layer Fn tidak punya isi bawaan yang dikenal OpenHE.";
    pub const RESET_CONFIRM: &str =
        "Semua tombol di layer Base kembali ke fungsi bawaannya (tombol Fn dilewati). Perubahan Base yang belum diterapkan dibuang.";
    pub const RESET_YES: &str = "Ya, kembalikan";
    pub const CANCEL: &str = "Batal";

    pub const SEARCH: &str = "Cari tombol…";
    pub const NO_MATCH: &str = "Tidak ada tombol yang cocok.";
    pub const PICK_HINT: &str = "Pilih tombol di keyboard, lalu klik fungsi pengganti di sini.";
    pub const PICK_DISABLED: &str = "Menunggu keyboard…";
    pub const EMPTY: &str = "Kosong";

    pub const NOTHING_SELECTED: &str = "Pilih tombol";
    pub const NOTHING_SELECTED_BODY: &str =
        "Klik tombol di keyboard, lalu pilih fungsi penggantinya di bawah. Seret untuk memilih beberapa.";
    pub const KEY_PREFIX: &str = "Tombol";
    pub const LAYER_WORD: &str = "layer";
    pub const NOW_SENDS: &str = "sekarang mengirim";
    pub const ORIGINAL: &str = "Asli";
    pub const NEW: &str = "Baru";
    pub const UNCHANGED: &str = "—";
    pub const LOCKED: &str =
        "Tombol Fn di layer Base terkunci, supaya keyboard selalu punya tombol Fn. Di layer Fn tombol ini boleh diubah.";
    pub const LOCKED_BADGE: &str = "Kunci";
    pub const FN_SKIPPED: &str = "Tombol Fn (Base) dilewati.";

    pub const PENDING: &str = "Perubahan tertunda";
    pub const NO_PENDING: &str = "Belum ada perubahan";
    pub const MORE: &str = "lagi";
    pub const DISCARD: &str = "Batalkan";
    pub const APPLY: &str = "Terapkan";
    pub const APPLYING: &str = "Menerapkan…";
    pub const APPLIED: &str = "Tersimpan di keyboard";
    pub const SAVED_IN_PROFILE: &str = "Disimpan di profil";
    pub const WORKS_WITHOUT_APP: &str = "tetap aktif tanpa app";
    pub const APPLY_FAILED: &str = "Gagal menerapkan";
    pub const RELOADED_HINT: &str = "Isi layer dibaca ulang dari keyboard.";
    pub const PARTIAL_WRITE: &str =
        "Keyboard terputus. Sebagian perubahan mungkin sudah terkirim; isi layer dibaca ulang saat tersambung.";
    pub const READING: &str = "Membaca isi layer…";
    pub const READ_FAILED: &str = "Gagal membaca isi layer";
    pub const READ_PARTIAL: &str = "Sebagian tombol tidak terbaca.";

    pub const CAT_BASIC: &str = "Dasar";
    pub const CAT_NAVIGATION: &str = "Navigasi";
    pub const CAT_FUNCTION: &str = "Fungsi";
    pub const CAT_NUMPAD: &str = "Numpad";
    pub const CAT_MODIFIER: &str = "Pengubah";
    pub const CAT_MEDIA: &str = "Media";
    pub const CAT_MOUSE: &str = "Mouse";
    pub const CAT_LIGHTING: &str = "Lampu";
    pub const CAT_SYSTEM: &str = "Sistem";
    pub const CAT_SPECIAL: &str = "Khusus";
}
use strings as t;

/// The Fn key's own default code. Locked on the Base layer.
pub const FN_KEY: u8 = 0x01;

pub const LAYERS: [(Layer, &str); 4] = [
    (Layer::Base, "Base"),
    (Layer::Fn1, "Fn1"),
    (Layer::Fn2, "Fn2"),
    (Layer::Fn3, "Fn3"),
];

pub fn layer_name(l: Layer) -> &'static str {
    LAYERS[l as usize].1
}

/// The Fn key on the Base layer can never be changed.
pub fn is_locked(layer: Layer, key: u8) -> bool {
    layer == Layer::Base && key == FN_KEY
}

// ------------------------------------------------------------- key names

pub const CATEGORIES: [Category; 10] = [
    Category::Basic,
    Category::Navigation,
    Category::Function,
    Category::Numpad,
    Category::Modifier,
    Category::Media,
    Category::Mouse,
    Category::Lighting,
    Category::System,
    Category::Special,
];

pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Basic => t::CAT_BASIC,
        Category::Navigation => t::CAT_NAVIGATION,
        Category::Function => t::CAT_FUNCTION,
        Category::Numpad => t::CAT_NUMPAD,
        Category::Modifier => t::CAT_MODIFIER,
        Category::Media => t::CAT_MEDIA,
        Category::Mouse => t::CAT_MOUSE,
        Category::Lighting => t::CAT_LIGHTING,
        Category::System => t::CAT_SYSTEM,
        Category::Special => t::CAT_SPECIAL,
    }
}

/// Indonesian (long, short) names for the codes whose table label is an
/// unresolved token like "i18n:key5". Meanings follow the HID consumer usages
/// and `docs/protocol/keycodes.txt` (icon column). 0x00C7 and 0x00C8 have no
/// known meaning and stay "Kode 0x…".
fn indonesian(code: u16) -> Option<(&'static str, &'static str)> {
    Some(match code {
        0x106F => ("Kecerahan layar +", "Cerah +"),
        0x1070 => ("Kecerahan layar −", "Cerah −"),
        0x10B5 => ("Lagu berikutnya", "Lagu >"),
        0x10B6 => ("Lagu sebelumnya", "Lagu <"),
        0x10B7 => ("Berhenti", "Stop"),
        0x10CD => ("Putar / jeda", "Putar"),
        0x10E2 => ("Bisukan", "Mute"),
        0x10E9 => ("Volume +", "Vol +"),
        0x10EA => ("Volume −", "Vol −"),
        0x7301 => ("Klik kiri", "Klik ki"),
        0x7302 => ("Klik kanan", "Klik ka"),
        0x7303 => ("Klik tengah", "Klik tg"),
        0x7304 => ("Gerak kursor kiri", "Gerak ki"),
        0x7305 => ("Gerak kursor kanan", "Gerak ka"),
        0x7306 => ("Gerak kursor atas", "Gerak at"),
        0x7307 => ("Gerak kursor bawah", "Gerak bw"),
        0x7308 => ("Roda gulir naik", "Roda +"),
        0x7309 => ("Roda gulir turun", "Roda −"),
        0xF100 => ("Pulihkan bawaan", "Reset"),
        0xF103 => ("Kalibrasi", "Kalibrasi"),
        0xF108 => ("Profil 1", "Profil 1"),
        0xF109 => ("Profil 2", "Profil 2"),
        0xF10A => ("Profil 3", "Profil 3"),
        0xF10C => ("Profil 4", "Profil 4"),
        0xF10B => ("Tukar WASD / panah", "WASD"),
        0xF317 => ("Lampu utama hidup / mati", "Lampu"),
        0xF311 => ("Arah efek lampu", "Arah"),
        0xF310 => ("Efek lampu +", "Efek +"),
        0xF32F => ("Efek lampu −", "Efek −"),
        0xF312 => ("Kecerahan lampu +", "Terang +"),
        0xF313 => ("Kecerahan lampu −", "Terang −"),
        0xF314 => ("Kecepatan lampu +", "Cepat +"),
        0xF315 => ("Kecepatan lampu −", "Cepat −"),
        0xF326 => ("Lampu logo hidup / mati", "Logo"),
        0xF327 => ("Arah lampu logo", "Logo arah"),
        0xF325 => ("Efek logo +", "Efek lg +"),
        0xF32C => ("Efek logo −", "Efek lg −"),
        0xF32A => ("Kecerahan logo +", "Terang lg+"),
        0xF32B => ("Kecerahan logo −", "Terang lg−"),
        0xF328 => ("Kecepatan logo +", "Cepat lg +"),
        0xF329 => ("Kecepatan logo −", "Cepat lg −"),
        _ => return None,
    })
}

/// Vendor labels that are too long for a key cap.
fn abbreviate(label: &str) -> String {
    match label {
        "Back Space" => "Bksp".into(),
        "Caps Lock" | "Cap" => "Caps".into(),
        "Num Lock" | "NumLc" => "NumLk".into(),
        "Scroll Lock" | "ScrLc" => "ScrLk".into(),
        "Sys Request" => "SysRq".into(),
        "Desktop Power Down" => "Mati".into(),
        "Desktop Sleep" => "Tidur".into(),
        "Static-Light-Change" => "Statis".into(),
        other if other.chars().count() > 7 => {
            let head: String = other.chars().take(6).collect();
            format!("{head}…")
        }
        other => other.to_string(),
    }
}

fn unknown(code: u16) -> String {
    format!("Kode 0x{code:04X}")
}

/// Full name of a keycode for lists and tooltips. Never an "i18n:" token;
/// codes the table does not know show "Kode 0x…".
pub fn code_name(code: u16) -> String {
    if code == 0 {
        return t::EMPTY.to_string();
    }
    if let Some((long, _)) = indonesian(code) {
        return long.to_string();
    }
    match keycodes::lookup(code) {
        Some(k) if !k.label.starts_with("i18n:") && k.label != "-" => k.label.to_string(),
        _ => unknown(code),
    }
}

/// Short name that fits on a key cap.
pub fn short_code_name(code: u16) -> String {
    if code == 0 {
        return t::EMPTY.to_string();
    }
    if let Some((_, short)) = indonesian(code) {
        return short.to_string();
    }
    match keycodes::lookup(code) {
        Some(k) if !k.label.starts_with("i18n:") && k.label != "-" => abbreviate(k.label),
        _ => unknown(code),
    }
}

/// Codes offered by the picker: reserved placeholders ("-"), the unnamed
/// non-US keys and the two codes with no known meaning are hidden.
pub fn pickable(k: &KeyCode) -> bool {
    !(k.label == "-" && k.category == Category::Special)
        && !matches!(k.code, 0x0032 | 0x0064 | 0x00C7 | 0x00C8)
}

/// Search match on the Indonesian name, the vendor label or the hex code.
pub fn picker_matches(k: &KeyCode, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let q_hex = q.trim_start_matches("0x");
    code_name(k.code).to_lowercase().contains(&q)
        || k.label.to_lowercase().contains(&q)
        || format!("{:04x}", k.code).contains(q_hex)
}

// ------------------------------------------------------- pending changes

/// Changes waiting for "Terapkan", across all layers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    map: BTreeMap<(u8, u8), u16>,
}

fn layer_of(i: u8) -> Layer {
    LAYERS[usize::from(i.min(3))].0
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&self, layer: Layer, key: u8) -> Option<u16> {
        self.map.get(&(layer as u8, key)).copied()
    }

    /// Queues `code` for `keys` on `layer`; returns how many keys were
    /// accepted. The Fn key on Base is never queued. A code equal to the
    /// key's current one (`current`) removes a queued change instead.
    pub fn queue(
        &mut self,
        layer: Layer,
        keys: &[u8],
        code: u16,
        current: impl Fn(Layer, u8) -> Option<u16>,
    ) -> usize {
        let mut accepted = 0;
        for &key in keys {
            if is_locked(layer, key) {
                continue;
            }
            accepted += 1;
            if current(layer, key) == Some(code) {
                self.map.remove(&(layer as u8, key));
            } else {
                self.map.insert((layer as u8, key), code);
            }
        }
        accepted
    }

    /// The entries for `set_layer_codes`, ordered by layer, then key.
    pub fn entries(&self) -> Vec<(u8, Layer, u16)> {
        self.map
            .iter()
            .map(|(&(l, k), &c)| (k, layer_of(l), c))
            .collect()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn clear_layer(&mut self, layer: Layer) {
        self.map.retain(|&(l, _), _| l != layer as u8);
    }

    /// Drops entries that were sent, unless they were edited since.
    pub fn remove_applied(&mut self, sent: &[(u8, Layer, u16)]) {
        for &(k, l, c) in sent {
            if self.map.get(&(l as u8, k)) == Some(&c) {
                self.map.remove(&(l as u8, k));
            }
        }
    }
}

/// "Kembalikan ke bawaan": every key's own default code on Base, except Fn.
pub fn reset_entries(keys: impl IntoIterator<Item = u8>) -> Vec<(u8, Layer, u16)> {
    keys.into_iter()
        .filter(|&k| !is_locked(Layer::Base, k))
        .map(|k| (k, Layer::Base, u16::from(k)))
        .collect()
}

// ---------------------------------------------------------------- jobs

/// What reading one layer gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRead {
    pub layer: Layer,
    pub codes: Vec<(u8, u16)>,
    /// Set when reading stopped early; `codes` then holds what was read.
    pub error: Option<Error>,
}

/// Reads the code of every key in `keys` on `layer`. A disconnect is
/// returned as `Err(Disconnected)`; another error stops the read and is
/// reported with the partial result.
pub fn read_layer_job<T: Transport>(
    kb: &mut Keyboard<T>,
    layer: Layer,
    keys: &[u8],
) -> Result<LayerRead, Error> {
    let mut codes = Vec::with_capacity(keys.len());
    let mut error = None;
    for &k in keys {
        match kb.layer_code(k, layer) {
            Ok(c) => codes.push((k, c)),
            Err(Error::Disconnected) => return Err(Error::Disconnected),
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    Ok(LayerRead {
        layer,
        codes,
        error,
    })
}

/// What the apply job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub write: Result<(), Error>,
    /// Re-read entries (only after a failed write; a successful write is
    /// already verified by `set_layer_codes`).
    pub reread: Vec<(u8, Layer, u16)>,
    pub reload_error: Option<Error>,
}

/// The remap write job: `set_layer_codes`, and after a failed write a re-read
/// of every entry (a batch can fail halfway). Generic over the transport so
/// tests drive it with `MockTransport`.
pub fn apply_job<T: Transport>(
    kb: &mut Keyboard<T>,
    entries: &[(u8, Layer, u16)],
) -> Result<ApplyOutcome, Error> {
    let write = kb.set_layer_codes(entries);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let mut reread = Vec::new();
    let mut reload_error = None;
    // `InvalidInput` is raised before any I/O: nothing was written.
    if write.is_err() && !matches!(write, Err(Error::InvalidInput(_))) {
        for &(k, l, _) in entries {
            match kb.layer_code(k, l) {
                Ok(c) => reread.push((k, l, c)),
                Err(Error::Disconnected) => return Err(Error::Disconnected),
                Err(e) => {
                    reload_error = Some(e);
                    break;
                }
            }
        }
    }
    Ok(ApplyOutcome {
        write,
        reread,
        reload_error,
    })
}

/// What the page does with a finished apply job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyEffects {
    /// Codes to store in the layer cache.
    pub patch: Vec<(u8, Layer, u16)>,
    /// Drop the cached layers (their content is unknown).
    pub invalidate: bool,
    pub request_reload: bool,
    pub error: Option<String>,
    pub technical: Option<String>,
    /// All entries were written and verified.
    pub applied: bool,
}

pub fn apply_effects(
    entries: &[(u8, Layer, u16)],
    result: Result<ApplyOutcome, Error>,
) -> ApplyEffects {
    match result {
        Ok(out) => match &out.write {
            Ok(()) => ApplyEffects {
                patch: entries.to_vec(),
                invalidate: false,
                request_reload: false,
                error: None,
                technical: None,
                applied: true,
            },
            Err(e) => {
                let mut msg = i18n::device_error(e);
                if out.reload_error.is_none() && !out.reread.is_empty() {
                    msg = format!("{msg} {}", t::RELOADED_HINT);
                }
                ApplyEffects {
                    patch: out.reread,
                    invalidate: out.reload_error.is_some(),
                    request_reload: out.reload_error.is_some(),
                    error: Some(msg),
                    technical: Some(i18n::technical(e)),
                    applied: false,
                }
            }
        },
        // Mid-batch disconnect: some keys may be written. The worker
        // reloads on reconnect; the cache is dropped.
        Err(Error::Disconnected) => ApplyEffects {
            patch: Vec::new(),
            invalidate: true,
            request_reload: false,
            error: Some(t::PARTIAL_WRITE.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            applied: false,
        },
        Err(e) => ApplyEffects {
            patch: Vec::new(),
            invalidate: true,
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            applied: false,
        },
    }
}

// ------------------------------------------------------------- picker UI

/// State of a key picker (category tab and search text).
#[derive(Debug, Clone, Default)]
pub struct PickerState {
    pub category: Option<Category>,
    pub search: String,
}

fn chip(ui: &mut Ui, text: &str, active: bool, enabled: bool) -> egui::Response {
    let (fill, stroke, text_c) = if active {
        (color::ACCENT_SOFT, color::ACCENT, color::ACCENT)
    } else {
        (color::KEY, color::BORDER_STRONG, color::TEXT)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(text_c))
            .fill(fill)
            .stroke(line(1.0, stroke))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

/// The codes the picker shows for `state`: matching the search across all
/// categories, else the active category.
pub fn picker_codes(state: &PickerState) -> Vec<&'static KeyCode> {
    let searching = !state.search.trim().is_empty();
    let cat = state.category.unwrap_or(Category::Basic);
    keycodes::all()
        .iter()
        .filter(|k| pickable(k))
        .filter(|k| {
            if searching {
                picker_matches(k, &state.search)
            } else {
                k.category == cat
            }
        })
        .collect()
}

/// Category tabs, a search box and the code buttons. Returns the code the
/// user clicked. `max_height` bounds the scrolling code list.
pub fn picker_ui(
    ui: &mut Ui,
    state: &mut PickerState,
    current: Option<u16>,
    enabled: bool,
    max_height: f32,
) -> Option<u16> {
    let mut picked = None;
    ui.add_enabled_ui(enabled, |ui| {
        let searching = !state.search.trim().is_empty();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for c in CATEGORIES {
                let active = !searching && state.category.unwrap_or(Category::Basic) == c;
                if chip(ui, category_name(c), active, enabled).clicked() {
                    state.category = Some(c);
                    state.search.clear();
                }
            }
        });
        ui.add_space(6.0);
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::TextEdit::singleline(&mut state.search)
                .hint_text(t::SEARCH)
                .vertical_align(Align::Center),
        );
        ui.add_space(8.0);
        let codes = picker_codes(state);
        egui::ScrollArea::vertical()
            .max_height(max_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if codes.is_empty() {
                    card::hint(ui, t::NO_MATCH);
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for k in codes {
                        let name = code_name(k.code);
                        let resp = chip(ui, &name, current == Some(k.code), enabled)
                            .on_hover_text(format!("{name} · 0x{:04X}", k.code));
                        if resp.clicked() {
                            picked = Some(k.code);
                        }
                    }
                });
            });
    });
    picked
}

/// A button showing a code's name; clicking opens a popup with the picker.
/// Returns true when `value` changed. `id` must be unique on the page.
pub fn code_field(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    value: &mut u16,
    enabled: bool,
    width: f32,
) -> bool {
    let resp = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(code_name(*value)).color(color::TEXT))
            .fill(color::PANEL)
            .stroke(line(1.0, color::BORDER_STRONG))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
            .min_size(egui::vec2(width, 34.0)),
    );
    let state_id = resp.id.with(("code-picker", id_hash(&id)));
    let mut changed = false;
    Popup::from_toggle_button_response(&resp)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(332.0)
        .show(|ui| {
            ui.set_width(316.0);
            let mut st: PickerState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
            let picked = picker_ui(ui, &mut st, Some(*value), true, 260.0);
            ui.data_mut(|d| d.insert_temp(state_id, st));
            if let Some(code) = picked {
                *value = code;
                changed = true;
                ui.close();
            }
        });
    changed
}

fn id_hash(id: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

// ------------------------------------------------------------------ page

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    Changes,
    Reset,
}

/// Entries for `set_layer_codes`.
type Entries = Vec<(u8, Layer, u16)>;

struct ReadReq {
    layer: Layer,
    /// Worker generation when submitted; a result from before a reload is
    /// dropped.
    generation: u64,
    pending: Pending<LayerRead>,
}

pub struct Page {
    layer: Layer,
    selection: Selection,
    picker: PickerState,
    /// Codes read from the keyboard, by layer index, then default key code.
    codes: BTreeMap<u8, BTreeMap<u8, u16>>,
    changes: Changes,
    read: Option<ReadReq>,
    /// Layers whose read failed; not retried until the data reloads.
    read_failed: BTreeSet<u8>,
    read_error: Option<(String, String)>,
    pending: Option<Pending<ApplyOutcome>>,
    last_sent: Option<(Entries, Sent)>,
    error: Option<String>,
    error_technical: Option<String>,
    applied: bool,
    confirm_reset: bool,
    /// Worker generation and profile the cache belongs to.
    seen: Option<(u64, u8)>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            layer: Layer::Base,
            selection: Selection::default(),
            picker: PickerState::default(),
            codes: BTreeMap::new(),
            changes: Changes::default(),
            read: None,
            read_failed: BTreeSet::new(),
            read_error: None,
            pending: None,
            last_sent: None,
            error: None,
            error_technical: None,
            applied: false,
            confirm_reset: false,
            seen: None,
        }
    }
}

fn code_of(codes: &BTreeMap<u8, BTreeMap<u8, u16>>, layer: Layer, key: u8) -> Option<u16> {
    codes.get(&(layer as u8))?.get(&key).copied()
}

/// Per-key decoration for `layer`: the key's code under its legend, a stroke
/// when it differs from the default, the accent for a pending change and a
/// lock for the Fn key on Base.
pub fn decoration(
    layer: Layer,
    key: u8,
    current: Option<u16>,
    pending: Option<u16>,
) -> KeyDecoration {
    if is_locked(layer, key) {
        return KeyDecoration {
            label2: Some(t::LOCKED_BADGE.to_string()),
            fill: None,
            stroke: Some(color::FAINT),
            badge: None,
        };
    }
    if let Some(code) = pending {
        return KeyDecoration {
            label2: Some(short_code_name(code)),
            fill: Some(color::ACCENT_SOFT),
            stroke: Some(color::ACCENT),
            badge: None,
        };
    }
    let Some(code) = current else {
        return KeyDecoration::default();
    };
    let default = match layer {
        Layer::Base => u16::from(key),
        _ => 0,
    };
    if code == default {
        return KeyDecoration::default();
    }
    KeyDecoration {
        label2: Some(short_code_name(code)),
        fill: None,
        stroke: Some(color::INFO.gamma_multiply(0.7)),
        badge: None,
    }
}

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.poll(ctx);
        self.sync(ctx);
        page_with_right_panel(
            ui,
            "remap-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    // --------------------------------------------------------------- state

    fn poll(&mut self, ctx: &AppCtx) {
        if let Some(r) = self.read.as_mut() {
            if let Some(result) = r.pending.poll() {
                let (layer, generation) = (r.layer, r.generation);
                self.read = None;
                if generation == ctx.worker.generation() {
                    self.finish_read(layer, result);
                }
            }
        }
        if let Some(p) = self.pending.as_mut() {
            if let Some(result) = p.poll() {
                self.pending = None;
                self.finish_apply(ctx, result);
            }
        }
    }

    fn finish_read(&mut self, layer: Layer, result: Result<LayerRead, Error>) {
        match result {
            Ok(read) => {
                let map = self.codes.entry(layer as u8).or_default();
                for (k, c) in read.codes {
                    map.insert(k, c);
                }
                match read.error {
                    None => self.read_error = None,
                    Some(e) => {
                        self.read_failed.insert(layer as u8);
                        self.read_error = Some((
                            format!("{} {}", i18n::device_error(&e), t::READ_PARTIAL),
                            i18n::technical(&e),
                        ));
                    }
                }
            }
            Err(e) => {
                // Disconnected: the worker reconnects and reloads.
                self.read_failed.insert(layer as u8);
                self.read_error = Some((i18n::device_error(&e), i18n::technical(&e)));
            }
        }
    }

    fn finish_apply(&mut self, ctx: &AppCtx, result: Result<ApplyOutcome, Error>) {
        let sent = self.last_sent.clone().map(|(e, _)| e).unwrap_or_default();
        let fx = apply_effects(&sent, result);
        if fx.invalidate {
            self.codes.clear();
            self.read_failed.clear();
        }
        for (k, l, c) in &fx.patch {
            self.codes.entry(*l as u8).or_default().insert(*k, *c);
        }
        if fx.request_reload {
            ctx.worker.request_reload();
        }
        if fx.applied {
            self.changes.remove_applied(&sent);
        }
        self.error = fx.error;
        self.error_technical = fx.technical;
        self.applied = fx.applied;
        // On failure the pending changes stay, so "Coba lagi" resends them.
    }

    /// Drops stale data after a reload or profile change, then reads the
    /// open layer when it is not cached yet.
    fn sync(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else {
            self.codes.clear();
            self.read_failed.clear();
            self.confirm_reset = false;
            self.seen = None;
            return;
        };
        self.selection.retain_present(&snap.matrix);
        let generation = ctx.worker.generation();
        match self.seen {
            Some((g, p)) if g != generation || p != snap.profile => {
                self.codes.clear();
                self.read_failed.clear();
                self.read_error = None;
                if p != snap.profile {
                    self.changes.clear();
                }
            }
            _ => {}
        }
        self.seen = Some((generation, snap.profile));
        let li = self.layer as u8;
        if self.read.is_none()
            && self.pending.is_none()
            && !self.codes.contains_key(&li)
            && !self.read_failed.contains(&li)
            && ctx.can_write()
        {
            let keys: Vec<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
            let layer = self.layer;
            self.read = Some(ReadReq {
                layer,
                generation,
                pending: ctx
                    .worker
                    .submit("remap-read", move |kb| read_layer_job(kb, layer, &keys)),
            });
        }
    }

    fn submit(&mut self, ctx: &AppCtx, entries: Vec<(u8, Layer, u16)>, kind: Sent) {
        self.error = None;
        self.error_technical = None;
        self.applied = false;
        self.last_sent = Some((entries.clone(), kind));
        self.pending = Some(
            ctx.worker
                .submit("remap-apply", move |kb| apply_job(kb, &entries)),
        );
    }

    fn apply_changes(&mut self, ctx: &AppCtx) {
        let entries = self.changes.entries();
        if !entries.is_empty() {
            self.submit(ctx, entries, Sent::Changes);
        }
    }

    fn apply_reset(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else {
            return;
        };
        let entries = reset_entries(snap.matrix.keys().map(|(_, _, c)| c));
        self.changes.clear_layer(Layer::Base);
        self.confirm_reset = false;
        self.submit(ctx, entries, Sent::Reset);
    }

    fn retry(&mut self, ctx: &AppCtx) {
        if let Some((entries, kind)) = self.last_sent.clone() {
            self.submit(ctx, entries, kind);
        }
    }

    fn ordered(&self, ctx: &AppCtx) -> Vec<u8> {
        match ctx.snapshot {
            Some(s) => self.selection.codes_in(&s.matrix),
            None => self.selection.codes(),
        }
    }

    fn editable(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && self.pending.is_none()
    }

    fn pick(&mut self, ctx: &AppCtx, code: u16) {
        let keys = self.ordered(ctx);
        let layer = self.layer;
        let codes = &self.codes;
        self.changes
            .queue(layer, &keys, code, |l, k| code_of(codes, l, k));
        self.applied = false;
        self.error = None;
        self.error_technical = None;
    }

    /// What the first selected key sends now: pending, else read.
    fn effective(&self, key: u8) -> Option<u16> {
        self.changes
            .get(self.layer, key)
            .or_else(|| code_of(&self.codes, self.layer, key))
    }

    // -------------------------------------------------------------- centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let before = self.selection.clone();
        self.center_body(ctx, ui);
        if self.selection != before {
            // The right panel was drawn first; show it the new selection.
            ui.ctx().request_repaint();
        }
    }

    fn center_body(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let connected = ctx.connected();
        let model = ctx.snapshot.map_or(Model::Win68HeMax, |s| s.model);
        let fallback;
        let matrix = match ctx.snapshot {
            Some(s) => &s.matrix,
            None => {
                fallback = reference_matrix(model);
                &fallback
            }
        };

        ui.horizontal(|ui| {
            ui.set_min_height(40.0);
            ui.label(RichText::new(t::LAYER).color(color::MUTED));
            let old = self.layer;
            Segmented::new(&LAYERS)
                .enabled(connected)
                .show(ui, &mut self.layer);
            if self.layer != old {
                self.confirm_reset = false;
            }
            ui.label(
                RichText::new(if self.layer == Layer::Base {
                    t::HINT_BASE
                } else {
                    t::HINT_FN
                })
                .font(font::small())
                .color(color::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = self.editable(ctx) && self.layer == Layer::Base;
                let btn = egui::Button::new(RichText::new(t::RESET).color(color::TEXT))
                    .fill(color::BG)
                    .stroke(line(1.0, color::BORDER_STRONG))
                    .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
                    .min_size(egui::vec2(0.0, 38.0));
                let mut resp = ui.add_enabled(enabled, btn);
                if self.layer != Layer::Base {
                    resp = resp.on_disabled_hover_text(t::RESET_BASE_ONLY);
                }
                if resp.clicked() {
                    self.confirm_reset = true;
                }
            });
        });
        if self.confirm_reset {
            ui.add_space(8.0);
            Card::new()
                .fill(color::ACCENT_SOFT)
                .stroke(color::ACCENT.gamma_multiply(0.6))
                .padding(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(RichText::new(t::RESET_CONFIRM).color(color::TEXT)).wrap(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.editable(ctx), egui::Button::new(t::RESET_YES))
                            .clicked()
                        {
                            self.apply_reset(ctx);
                        }
                        if ui.button(t::CANCEL).clicked() {
                            self.confirm_reset = false;
                        }
                    });
                });
        }
        ui.add_space(12.0);

        let layer = self.layer;
        let snap = ctx.snapshot;
        {
            let codes = &self.codes;
            let changes = &self.changes;
            ui.vertical_centered(|ui| {
                KeyboardView::new(model, matrix)
                    .enabled(connected)
                    .id_salt("remap-keyboard")
                    .show(ui, &mut self.selection, |key| {
                        if snap.is_none() {
                            return KeyDecoration::default();
                        }
                        decoration(
                            layer,
                            key,
                            code_of(codes, layer, key),
                            changes.get(layer, key),
                        )
                    });
            });
        }
        if !connected {
            ui.add_space(8.0);
            waiting_banner(ui, ctx);
        } else if self.read.is_some() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(t::READING)
                        .font(font::small())
                        .color(color::MUTED),
                );
            });
        } else if let Some((msg, tech)) = self.read_error.clone() {
            ui.add_space(8.0);
            let retry = self.editable(ctx).then_some(i18n::common::RETRY);
            if card::error_box(ui, t::READ_FAILED, &msg, retry, Some(&tech)) {
                self.read_error = None;
                self.read_failed.remove(&(self.layer as u8));
            }
        }
        ui.add_space(16.0);

        let h = (ui.available_height() - 130.0).clamp(120.0, 420.0);
        let editable = self.editable(ctx);
        let keys = self.ordered(ctx);
        let current = keys.first().and_then(|&k| self.effective(k));
        let selectable = keys.iter().any(|&k| !is_locked(self.layer, k));
        let mut picked = None;
        Card::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            picked = picker_ui(ui, &mut self.picker, current, editable && selectable, h);
            ui.add_space(8.0);
            card::hint(
                ui,
                if connected {
                    t::PICK_HINT
                } else {
                    t::PICK_DISABLED
                },
            );
        });
        if let Some(code) = picked {
            self.pick(ctx, code);
        }
    }

    // --------------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("remap-apply")
            .show_separator_line(false)
            .frame(Frame::new().inner_margin(Margin {
                top: 12,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.apply_area(ctx, ui));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin {
                        right: 10,
                        ..Default::default()
                    })
                    .show(ui, |ui| self.editor(ctx, ui));
            });
    }

    fn editor(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let keys = self.ordered(ctx);
        if ctx.snapshot.is_none() || keys.is_empty() {
            ui.label(
                RichText::new(t::NOTHING_SELECTED)
                    .font(font::heading())
                    .color(color::TEXT)
                    .strong(),
            );
            card::hint(
                ui,
                if ctx.connected() {
                    t::NOTHING_SELECTED_BODY
                } else {
                    i18n::common::WAITING_DEVICE_BODY
                },
            );
            self.pending_list(ui);
            return;
        }
        let layer = self.layer;
        ui.label(
            RichText::new(format!(
                "{} {} · {} {}",
                t::KEY_PREFIX,
                legend_list(&keys, 6),
                t::LAYER_WORD,
                layer_name(layer)
            ))
            .font(font::heading())
            .color(color::TEXT)
            .strong(),
        );
        let first = keys[0];
        if keys.len() == 1 {
            let now = match code_of(&self.codes, layer, first) {
                Some(c) => code_name(c),
                None => "…".to_string(),
            };
            let who = if layer == Layer::Base {
                keycodes::legend(first).to_string()
            } else {
                format!("Fn + {}", keycodes::legend(first))
            };
            card::hint(ui, format!("{who} {} {now}", t::NOW_SENDS));
        } else {
            card::hint(ui, format!("{} tombol dipilih", keys.len()));
        }
        if keys.iter().any(|&k| is_locked(layer, k)) {
            card::hint(
                ui,
                if keys.len() == 1 {
                    t::LOCKED
                } else {
                    t::FN_SKIPPED
                },
            );
        }
        ui.add_space(10.0);

        if !is_locked(layer, first) {
            Card::new().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let original = code_of(&self.codes, layer, first);
                    let new = self.changes.get(layer, first);
                    cap_tile(
                        ui,
                        t::ORIGINAL,
                        &original.map_or("…".to_string(), short_code_name),
                        false,
                    );
                    ui.label(RichText::new("→").font(font::title()).color(color::MUTED));
                    cap_tile(
                        ui,
                        t::NEW,
                        &new.map_or(t::UNCHANGED.to_string(), short_code_name),
                        new.is_some(),
                    );
                });
            });
            ui.add_space(10.0);
        }
        self.pending_list(ui);
    }

    fn pending_list(&self, ui: &mut Ui) {
        card::section(ui, t::PENDING);
        ui.add_space(4.0);
        if self.changes.is_empty() {
            card::hint(ui, t::NO_PENDING);
            return;
        }
        const SHOWN: usize = 8;
        let entries = self.changes.entries();
        for (k, l, c) in entries.iter().take(SHOWN) {
            let was = code_of(&self.codes, *l, *k).map_or("…".to_string(), code_name);
            Frame::new()
                .fill(color::PANEL)
                .stroke(line(1.0, color::BORDER))
                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} · {}", keycodes::legend(*k), layer_name(*l)))
                                .color(color::TEXT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{was} → {}", code_name(*c)))
                                    .font(font::small())
                                    .color(color::ACCENT),
                            );
                        });
                    });
                });
        }
        if entries.len() > SHOWN {
            card::hint(ui, format!("+{} {}", entries.len() - SHOWN, t::MORE));
        }
    }

    fn apply_area(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let idle = self.editable(ctx);
        if let Some(err) = self.error.clone() {
            let retry = self.last_sent.is_some() && idle;
            let clicked = card::error_box(
                ui,
                t::APPLY_FAILED,
                &err,
                retry.then_some(i18n::common::RETRY),
                self.error_technical.as_deref(),
            );
            if clicked {
                self.retry(ctx);
            }
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            let discard = egui::Button::new(RichText::new(t::DISCARD).color(color::TEXT))
                .fill(color::BG)
                .stroke(line(1.0, color::BORDER_STRONG))
                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
                .min_size(egui::vec2(100.0, 46.0));
            if ui
                .add_enabled(idle && !self.changes.is_empty(), discard)
                .clicked()
            {
                self.changes.clear();
                self.error = None;
                self.applied = false;
            }
            let can_apply = idle && !self.changes.is_empty();
            if card::primary_button(ui, t::APPLY, can_apply).clicked() {
                self.apply_changes(ctx);
            }
        });
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(t::APPLYING)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            } else if self.applied && self.changes.is_empty() {
                ui.label(
                    RichText::new(t::APPLIED)
                        .font(font::small())
                        .color(color::SUCCESS),
                );
            } else if let Some(s) = ctx.snapshot {
                ui.label(
                    RichText::new(format!(
                        "{} {} · {}",
                        t::SAVED_IN_PROFILE,
                        s.profile + 1,
                        t::WORKS_WITHOUT_APP
                    ))
                    .font(font::small())
                    .color(color::MUTED),
                );
            }
        });
    }
}

const CAP_TILE_W: f32 = 76.0;

/// A small key-cap tile with a caption ("Asli" / "Baru").
fn cap_tile(ui: &mut Ui, caption: &str, text: &str, highlight: bool) {
    ui.vertical(|ui| {
        let (fill, stroke, text_c) = if highlight {
            (color::ACCENT_SOFT, color::ACCENT, color::ACCENT)
        } else {
            (color::KEY, color::BORDER_STRONG, color::TEXT)
        };
        Frame::new()
            .fill(fill)
            .stroke(line(1.0, stroke))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(8, 12))
            .show(ui, |ui| {
                ui.set_width(CAP_TILE_W);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(text).font(font::heading()).color(text_c));
                });
            });
        ui.allocate_ui_with_layout(
            egui::vec2(CAP_TILE_W + 16.0, 16.0),
            Layout::top_down(Align::Center),
            |ui| {
                ui.label(
                    RichText::new(caption)
                        .font(font::small())
                        .color(color::MUTED),
                );
            },
        );
    });
}

fn waiting_banner(ui: &mut Ui, ctx: &AppCtx) {
    let (title, body) = match ctx.status {
        crate::worker::ConnectionStatus::Error(e) => (i18n::device::ERROR, e.clone()),
        _ if ctx.busy => (
            i18n::common::LOADING_DEVICE,
            i18n::common::WAITING_DEVICE_BODY.to_string(),
        ),
        _ => (
            i18n::common::WAITING_DEVICE,
            i18n::common::WAITING_DEVICE_BODY.to_string(),
        ),
    };
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(title).color(color::ACCENT));
        ui.label(RichText::new(body).font(font::small()).color(color::MUTED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::MockTransport;
    use openhe_proto::frame::FrameError;

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn fail_reply() -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xFF;
        raw
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    fn none(_: Layer, _: u8) -> Option<u16> {
        None
    }

    // ---- pending changes

    #[test]
    fn fn_on_base_is_never_queued() {
        let mut c = Changes::default();
        let n = c.queue(Layer::Base, &[0x01, 0x04], 0x05, none);
        assert_eq!(n, 1);
        assert_eq!(c.entries(), vec![(0x04, Layer::Base, 0x05)]);
        assert_eq!(c.queue(Layer::Base, &[0x01], 0x04, none), 0);
        assert_eq!(c.get(Layer::Base, 0x01), None);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn fn_on_fn_layers_and_fn_as_a_code_are_allowed() {
        let mut c = Changes::default();
        for l in [Layer::Fn1, Layer::Fn2, Layer::Fn3] {
            assert_eq!(c.queue(l, &[0x01], 0x04, none), 1);
        }
        // Caps -> Fn on Base is fine: code 0x01 is not the locked key.
        assert_eq!(c.queue(Layer::Base, &[0x39], 0x01, none), 1);
        assert_eq!(c.get(Layer::Base, 0x39), Some(0x01));
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn choosing_the_current_code_drops_the_change() {
        let mut c = Changes::default();
        c.queue(Layer::Fn1, &[0x1A], 0x52, none);
        assert_eq!(c.len(), 1);
        let cur = |_: Layer, k: u8| (k == 0x1A).then_some(0x52);
        c.queue(Layer::Fn1, &[0x1A], 0x52, cur);
        assert!(c.is_empty());
    }

    #[test]
    fn entries_are_ordered_and_partly_removable() {
        let mut c = Changes::default();
        c.queue(Layer::Fn2, &[0x05], 0x10, none);
        c.queue(Layer::Base, &[0x07, 0x06], 0x11, none);
        let e = c.entries();
        assert_eq!(
            e,
            vec![
                (0x06, Layer::Base, 0x11),
                (0x07, Layer::Base, 0x11),
                (0x05, Layer::Fn2, 0x10)
            ]
        );
        // Entry edited after sending survives.
        c.queue(Layer::Base, &[0x06], 0x12, none);
        c.remove_applied(&e);
        assert_eq!(c.entries(), vec![(0x06, Layer::Base, 0x12)]);
        c.clear_layer(Layer::Base);
        assert!(c.is_empty());
    }

    #[test]
    fn reset_skips_the_fn_key_and_writes_default_codes() {
        let e = reset_entries([0x29, 0x01, 0x04, 0x39]);
        assert_eq!(
            e,
            vec![
                (0x29, Layer::Base, 0x29),
                (0x04, Layer::Base, 0x04),
                (0x39, Layer::Base, 0x39)
            ]
        );
        let all = reset_entries(
            reference_matrix(Model::Win68HeMax)
                .keys()
                .map(|(_, _, c)| c),
        );
        assert_eq!(all.len(), 67, "68 keys minus Fn");
        assert!(all
            .iter()
            .all(|&(k, l, c)| k != 0x01 && l == Layer::Base && c == u16::from(k)));
    }

    // ---- names and picker

    #[test]
    fn no_name_is_an_unresolved_token() {
        for k in keycodes::all() {
            assert!(!code_name(k.code).starts_with("i18n:"), "{:#06x}", k.code);
            assert!(
                !short_code_name(k.code).starts_with("i18n:"),
                "{:#06x}",
                k.code
            );
        }
        // The two with no known meaning, and a code outside the table.
        assert_eq!(code_name(0x00C7), "Kode 0x00C7");
        assert_eq!(code_name(0xABCD), "Kode 0xABCD");
        assert_eq!(code_name(0x0000), "Kosong");
        assert_eq!(code_name(0x10E9), "Volume +");
        assert_eq!(code_name(0x0004), "A");
    }

    #[test]
    fn all_i18n_tokens_have_an_indonesian_name() {
        for k in keycodes::all()
            .iter()
            .filter(|k| k.label.starts_with("i18n:"))
        {
            if matches!(k.code, 0x00C7 | 0x00C8) {
                continue;
            }
            assert!(indonesian(k.code).is_some(), "{:#06x} {}", k.code, k.label);
        }
    }

    #[test]
    fn every_pickable_code_is_in_exactly_one_category() {
        for k in keycodes::all().iter().filter(|k| pickable(k)) {
            let n = CATEGORIES.iter().filter(|&&c| c == k.category).count();
            assert_eq!(n, 1, "{:#06x}", k.code);
        }
        let mut st = PickerState::default();
        let mut seen = 0;
        for c in CATEGORIES {
            st.category = Some(c);
            seen += picker_codes(&st).len();
        }
        assert_eq!(seen, keycodes::all().iter().filter(|k| pickable(k)).count());
    }

    #[test]
    fn search_matches_names_labels_and_hex_across_categories() {
        let mut st = PickerState {
            search: "volume".into(),
            ..PickerState::default()
        };
        let found: Vec<u16> = picker_codes(&st).iter().map(|k| k.code).collect();
        assert!(found.contains(&0x10E9) && found.contains(&0x10EA));
        st.search = "0x0029".into();
        assert!(picker_codes(&st).iter().any(|k| k.code == 0x29));
        st.search = "zzzzzz".into();
        assert!(picker_codes(&st).is_empty());
        st.search.clear();
        st.category = Some(Category::Mouse);
        assert_eq!(picker_codes(&st).len(), 9);
    }

    // ---- decoration

    #[test]
    fn decoration_marks_lock_pending_and_changed_keys() {
        let lock = decoration(Layer::Base, 0x01, Some(0x01), None);
        assert_eq!(lock.label2.as_deref(), Some(t::LOCKED_BADGE));
        assert!(lock.stroke.is_some() && lock.fill.is_none());
        // The Fn key on Fn1 is an ordinary key.
        assert_eq!(
            decoration(Layer::Fn1, 0x01, Some(0), None),
            KeyDecoration::default()
        );
        let pend = decoration(Layer::Fn1, 0x1A, Some(0), Some(0x52));
        assert_eq!(pend.label2.as_deref(), Some("↑"));
        assert_eq!(pend.stroke, Some(color::ACCENT));
        // Base: default code shows nothing; a changed one shows its name.
        assert_eq!(
            decoration(Layer::Base, 0x04, Some(0x04), None),
            KeyDecoration::default()
        );
        assert_eq!(
            decoration(Layer::Base, 0x04, Some(0x05), None)
                .label2
                .as_deref(),
            Some("B")
        );
        // Fn layer: empty follows Base and shows nothing.
        assert_eq!(
            decoration(Layer::Fn2, 0x04, Some(0), None),
            KeyDecoration::default()
        );
        assert_eq!(
            decoration(Layer::Fn2, 0x04, None, None),
            KeyDecoration::default()
        );
    }

    // ---- jobs

    #[test]
    fn apply_success_is_verified_and_patches_the_written_codes() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0B)); // SET echo
        m.push_reply(key_reply(0x0A, 0, 0x0B)); // verify
        m.push_reply(key_reply(0x0A, 1, 0x4F));
        let entries = [(0x0A, Layer::Base, 0x0B), (0x0A, Layer::Fn1, 0x4F)];
        let out = apply_job(&mut kb(m), &entries).unwrap();
        assert_eq!(out.write, Ok(()));
        assert!(out.reread.is_empty());
        let fx = apply_effects(&entries, Ok(out));
        assert!(fx.applied && !fx.invalidate && !fx.request_reload);
        assert_eq!(fx.patch, entries.to_vec());
        assert_eq!(fx.error, None);
    }

    #[test]
    fn the_job_never_writes_fn_on_base_even_if_asked() {
        let mut k = kb(MockTransport::new());
        let out = apply_job(&mut k, &[(0x01, Layer::Base, 0x04)]).unwrap();
        assert!(matches!(out.write, Err(Error::InvalidInput(_))));
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn verify_failure_rereads_and_patches_the_true_codes() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0B)); // SET echo
        m.push_reply(key_reply(0x0A, 0, 0x0C)); // verify: did not stick
        m.push_reply(key_reply(0x0A, 0, 0x0C)); // re-read
        let entries = [(0x0A, Layer::Base, 0x0B)];
        let out = apply_job(&mut kb(m), &entries).unwrap();
        assert!(matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed")));
        assert_eq!(out.reread, vec![(0x0A, Layer::Base, 0x0C)]);
        let fx = apply_effects(&entries, Ok(out));
        assert!(!fx.applied && !fx.invalidate && !fx.request_reload);
        assert_eq!(fx.patch, vec![(0x0A, Layer::Base, 0x0C)]);
        assert_eq!(
            fx.error.as_deref(),
            Some(format!("{} {}", i18n::error::VERIFY, t::RELOADED_HINT).as_str())
        );
        assert!(fx.technical.unwrap().contains("verify failed"));
    }

    #[test]
    fn a_failing_reread_invalidates_the_cache_and_asks_for_a_reload() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0B));
        m.push_reply(key_reply(0x0A, 0, 0x0C)); // verify mismatch
                                                // No reply for the re-read: it times out.
        let entries = [(0x0A, Layer::Base, 0x0B)];
        let out = apply_job(&mut kb(m), &entries).unwrap();
        assert_eq!(out.reload_error, Some(Error::Timeout));
        let fx = apply_effects(&entries, Ok(out));
        assert!(fx.invalidate && fx.request_reload && !fx.applied);
        assert!(fx.patch.is_empty());
    }

    #[test]
    fn a_batch_rejected_halfway_rereads_every_entry() {
        let entries: Vec<_> = (0..15u8)
            .map(|i| (0x10 + i, Layer::Fn1, 0x0100 + u16::from(i)))
            .collect();
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x10, 1, 0x0100)); // packet 1 echo
        m.push_reply(fail_reply()); // packet 2 rejected
        for &(k, _, v) in &entries {
            // Packet 1 was written, the 15th key was not.
            m.push_reply(key_reply(k, 1, if k == 0x1E { 0 } else { v }));
        }
        let out = apply_job(&mut kb(m), &entries).unwrap();
        assert_eq!(out.write, Err(Error::Frame(FrameError::DeviceFail)));
        assert_eq!(out.reread.len(), 15);
        assert_eq!(out.reread[14], (0x1E, Layer::Fn1, 0));
        let fx = apply_effects(&entries, Ok(out));
        assert_eq!(fx.patch.len(), 15, "the partial state is shown as it is");
        assert!(!fx.applied);
    }

    /// Fails every write after `n`, like a keyboard pulled mid-batch.
    struct UnplugAfter {
        inner: MockTransport,
        writes_left: usize,
    }

    impl Transport for UnplugAfter {
        fn write(&mut self, r: &[u8; 64]) -> Result<(), Error> {
            if self.writes_left == 0 {
                return Err(Error::Disconnected);
            }
            self.writes_left -= 1;
            self.inner.write(r)
        }
        fn read(&mut self, t: u32) -> Result<Option<[u8; 64]>, Error> {
            self.inner.read(t)
        }
    }

    #[test]
    fn unplugged_mid_batch_is_disconnected_with_a_partial_write_message() {
        let entries: Vec<_> = (0..15u8)
            .map(|i| (0x10 + i, Layer::Fn1, 0x0100 + u16::from(i)))
            .collect();
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x10, 1, 0x0100));
        let t = UnplugAfter {
            inner: m,
            writes_left: 1, // the first packet goes out, the second fails
        };
        let r = apply_job(&mut Keyboard::new(t, Model::Win68HeMax), &entries);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = apply_effects(&entries, r);
        assert_eq!(fx.error.as_deref(), Some(t::PARTIAL_WRITE));
        assert!(fx.invalidate && !fx.applied && fx.patch.is_empty());
    }

    #[test]
    fn other_job_errors_reload_everything() {
        let fx = apply_effects(
            &[],
            Err(Error::Io(format!(
                "{}: boom",
                crate::worker::INTERNAL_ERROR
            ))),
        );
        assert!(fx.request_reload && fx.invalidate);
        assert_eq!(fx.error.as_deref(), Some(i18n::error::INTERNAL));
    }

    #[test]
    fn reading_a_layer_collects_codes_and_reports_a_partial_read() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 2, 0x0050));
        m.push_reply(key_reply(0x05, 2, 0));
        // Third key: no reply, so reading stops with a timeout.
        let r = read_layer_job(&mut kb(m), Layer::Fn2, &[0x04, 0x05, 0x06]).unwrap();
        assert_eq!(r.codes, vec![(0x04, 0x0050), (0x05, 0)]);
        assert_eq!(r.error, Some(Error::Timeout));
    }

    #[test]
    fn reading_while_unplugged_is_disconnected() {
        let t = UnplugAfter {
            inner: MockTransport::new(),
            writes_left: 0,
        };
        let r = read_layer_job(
            &mut Keyboard::new(t, Model::Win68HeMax),
            Layer::Base,
            &[0x04],
        );
        assert_eq!(r, Err(Error::Disconnected));
    }

    // ---- page, headless

    #[test]
    fn the_page_reads_a_layer_once_and_never_queues_fn_on_base() {
        use crate::app::test_support::*;
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;

        let worker = offline_worker();
        let snap = snapshot68();
        worker.inject_status(ConnectionStatus::Connected(snap.clone()));
        let theme = Theme::default();
        let data = RefCell::new(AppData::default());
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut page = Page::default();
        let frame = |page: &mut Page, n: usize| {
            run_frames(&ctx, n, |c| {
                let status = worker.status();
                let app_ctx = AppCtx {
                    worker: &worker,
                    snapshot: match &status {
                        ConnectionStatus::Connected(s) => Some(s),
                        _ => None,
                    },
                    status: &status,
                    busy: worker.is_busy(),
                    theme: &theme,
                    data: &data,
                };
                egui::CentralPanel::default().show(c, |ui| page.ui(&app_ctx, ui));
            })
        };
        // The offline worker fails the read job with Disconnected.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !page.read_failed.contains(&0) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut page, 1);
        }
        assert!(page.read_error.is_some());
        frame(&mut page, 2);
        assert!(
            page.read.is_none(),
            "a failed layer is not retried every frame"
        );

        // Selecting Fn plus W and picking a code queues only W.
        page.selection.set([0x01, 0x1A]);
        {
            let status = worker.status();
            let app_ctx = AppCtx {
                worker: &worker,
                snapshot: None,
                status: &status,
                busy: false,
                theme: &theme,
                data: &data,
            };
            page.pick(&app_ctx, 0x52);
        }
        assert_eq!(page.changes.entries(), vec![(0x1A, Layer::Base, 0x52)]);
        frame(&mut page, 2); // everything renders with pending changes
    }
}

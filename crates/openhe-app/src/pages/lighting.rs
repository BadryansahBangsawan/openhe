//! Lampu: effects, the 7-colour palette, brightness/speed/direction/sleep,
//! and per-key colours.
//!
//! Mockup `docs/design/mockups/05-lampu.png`. Two tabs:
//!
//! * **Efek**: edits a [`Form`] that mirrors `openhe_proto::lighting::Lighting`
//!   in UI units (brightness and speed 1..=5, colour choices). The form is
//!   read from the keyboard by one job when the page opens (and after every
//!   reconnect or reload); "Terapkan ke keyboard" sends it with
//!   [`apply_lighting_job`], which always re-reads the lighting afterwards
//!   (also after a failed write), so a verify failure shows the true state.
//! * **Warna per tombol**: paints keys with a colour. The current key colours
//!   are read by one job when the tab opens; "Terapkan" sends only the keys
//!   that changed with [`apply_key_colors_job`] (the device API commits).
//!
//! Lighting reads/writes and the KRGB echo are not verified on hardware yet,
//! so every failure is shown inline with "Detail teknis" and a re-read, never
//! as a crash.

use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{layout, reference_matrix, KeyDecoration, KeyboardView, Selection};
use crate::widgets::segmented::Segmented;
use crate::widgets::toggle::toggle;
use crate::worker::Pending;
use eframe::egui::ecolor::Hsva;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, StrokeKind, Ui,
};
use openhe_device::{Error, Keyboard, Transport};
use openhe_proto::lighting::{
    Lighting, Rgb, DEFAULT_EXTRA_FLAGS, EFFECTS, MODE_CUSTOM, MODE_STATIC, SLEEP_MINUTES,
};
use std::collections::BTreeMap;

mod strings {
    pub const TAB_EFFECTS: &str = "Efek";
    pub const TAB_KEYS: &str = "Warna per tombol";
    pub const LIGHT_ON: &str = "Lampu menyala";

    pub const EFFECTS_TITLE: &str = "Efek";
    pub const EFFECTS_HINT: &str = "Pratinjau di layar; kirim dengan Terapkan ke keyboard";
    pub const EFFECTS_NOTE: &str =
        "Nama efek diterjemahkan dari daftar bawaan firmware (20 efek + statis + kustom).";
    pub const STATIC: &str = "Statis";
    pub const CUSTOM: &str = "Kustom";

    /// Indonesian labels for the wire modes 1..=20, in the order of
    /// `openhe_proto::lighting::EFFECTS` (English names in the comments).
    pub const EFFECT_LABELS: [(u8, &str); 20] = [
        (1, "Riak"),               // Ripple
        (2, "Pasang surut"),       // Tide
        (3, "Riak cahaya"),        // Ripple Light
        (4, "Badai berputar"),     // Rotating Storm
        (5, "Pelangi beruntung"),  // Lucky Rainbow
        (6, "Pelangi berkilau"),   // Shining Rainbow
        (7, "Gemerlap"),           // Glitter
        (8, "Panel bergeser"),     // Moving Panels
        (9, "Gelombang"),          // Waveform
        (10, "Bayangan bergeser"), // Shadow Shift
        (11, "Air mengalir"),      // Flowing Water
        (12, "Gelombang sinus"),   // Sine Wave
        (13, "Mekar"),             // Blooming
        (14, "Mosaik"),            // Mosaic
        (15, "Hujan"),             // Rainfall
        (16, "Denyut"),            // Pulse
        (17, "Jejak salju"),       // Snow Trail
        (18, "Bunga plum"),        // Plum Blossom
        (19, "Laser"),             // Laser
        (20, "Riak air"),          // Water Ripple
    ];

    pub const DESC_STATIC: &str = "Semua tombol menyala dengan satu warna tetap.";
    pub const DESC_EFFECT: &str = "Efek bergerak di seluruh keyboard.";
    pub const DESC_CUSTOM: &str = "Memakai warna tiap tombol dari tab Warna per tombol.";

    pub const COLOR: &str = "Warna";
    pub const COLOR_RAINBOW: &str = "Pelangi";
    pub const COLOR_ONE: &str = "Satu warna";
    pub const COLOR_CUSTOM_HINT: &str = "Efek Kustom tidak memakai palet; warna diatur per tombol.";
    pub const PALETTE_HINT: &str = "Tujuh warna palet disimpan di keyboard.";
    pub const HEX: &str = "Hex";
    pub const BRIGHTNESS: &str = "Kecerahan";
    pub const SPEED: &str = "Kecepatan";
    pub const SPEED_OFF_HINT: &str = "Efek statis dan kustom tidak punya kecepatan.";
    pub const DIRECTION: &str = "Arah";
    pub const DIRECTION_FWD: &str = "→";
    pub const DIRECTION_REV: &str = "←";
    pub const DIRECTION_HINT: &str = "Hanya berlaku untuk efek bergerak.";
    pub const SLEEP: &str = "Mati otomatis";
    pub const SLEEP_NEVER: &str = "Tidak pernah";
    pub const SLEEP_MIN: &str = "menit";
    pub const SLEEP_HOUR: &str = "jam";

    pub const APPLY: &str = "Terapkan ke keyboard";
    pub const APPLYING: &str = "Menerapkan…";
    pub const APPLIED: &str = "Tersimpan di keyboard";
    pub const NO_CHANGES: &str = "Belum ada perubahan";
    pub const UNSENT: &str = "Perubahan belum dikirim ke keyboard";
    pub const APPLY_FAILED: &str = "Gagal menerapkan lampu";
    pub const RELOADED_HINT: &str = "Nilai di layar sudah dibaca ulang dari keyboard.";
    pub const PARTIAL_WRITE: &str =
        "Keyboard terputus. Sebagian perubahan mungkin sudah terkirim; nilai dibaca ulang saat tersambung.";
    pub const LOADING: &str = "Membaca pengaturan lampu…";
    pub const LOAD_FAILED: &str = "Gagal membaca pengaturan lampu";
    pub const OFFLINE: &str = "Sambungkan keyboard untuk mengatur lampu.";
    pub const EXTERNAL_CHANGE: &str =
        "Lampu diubah dari keyboard. Muat ulang untuk melihatnya (perubahan di layar dibuang), atau abaikan untuk tetap memakai perubahan di layar.";
    pub const IGNORE: &str = "Abaikan";
    pub const UNVERIFIED: &str =
        "Pengaturan lampu belum diuji di keyboard sungguhan. Jika hasilnya berbeda, nilai dibaca ulang.";

    pub const KEYS_PAINT: &str = "Warna kuas";
    pub const KEYS_PAINT_DESC: &str = "Pilih tombol di keyboard lalu cat dengan warna ini.";
    pub const KEYS_PALETTE: &str = "Palet";
    pub const KEYS_PAINT_SELECTED: &str = "Cat tombol terpilih";
    pub const KEYS_PAINT_ALL: &str = "Cat semua tombol";
    pub const KEYS_BLACK: &str = "Matikan tombol terpilih";
    pub const KEYS_SELECT_ALL: &str = "Pilih semua";
    pub const KEYS_CLEAR: &str = "Hapus pilihan";
    pub const KEYS_SELECTED: &str = "tombol dipilih";
    pub const KEYS_CHANGED: &str = "tombol berubah";
    pub const KEYS_HINT: &str = "Klik atau seret untuk memilih tombol. Shift+klik menambah.";
    pub const KEYS_MODE_HINT: &str =
        "Warna per tombol hanya terlihat saat efek Kustom aktif di tab Efek.";
    pub const KEYS_USE_CUSTOM: &str = "Pilih efek Kustom";
    pub const KEYS_LOADING: &str = "Membaca warna tombol…";
    pub const KEYS_LOAD_FAILED: &str = "Gagal membaca warna tombol";
    pub const KEYS_APPLY: &str = "Terapkan warna tombol";
    pub const KEYS_APPLY_FAILED: &str = "Gagal menerapkan warna tombol";
    pub const KEYS_APPLIED: &str = "Warna tombol tersimpan di keyboard";
    pub const KEYS_PARTIAL: &str =
        "Keyboard terputus. Sebagian warna mungkin sudah terkirim; nilai dibaca ulang saat tersambung.";
    pub const KEYS_NOT_COMMITTED: &str =
        "Warna yang terkirim sebagian belum disimpan permanen. Coba terapkan lagi.";
}

// ------------------------------------------------------------ form logic

/// How a colour is picked: from the 7-colour palette or "colourful".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    /// Rainbow / random colours chosen by the firmware.
    Colorful,
    /// Palette slot 0..=6.
    Palette(u8),
}

impl ColorChoice {
    /// Static mode: 0..=6 is a palette slot, 7 is colourful.
    pub fn from_static(v: u8) -> ColorChoice {
        if v >= 7 {
            ColorChoice::Colorful
        } else {
            ColorChoice::Palette(v)
        }
    }

    pub fn to_static(self) -> u8 {
        match self {
            ColorChoice::Colorful => 7,
            ColorChoice::Palette(i) => i.min(6),
        }
    }

    /// Dynamic effects: 0 is colourful, n (1..=7) is palette slot n - 1.
    pub fn from_dynamic(v: u8) -> ColorChoice {
        match v {
            0 => ColorChoice::Colorful,
            n => ColorChoice::Palette((n - 1).min(6)),
        }
    }

    pub fn to_dynamic(self) -> u8 {
        match self {
            ColorChoice::Colorful => 0,
            ColorChoice::Palette(i) => i.min(6) + 1,
        }
    }
}

/// What the Efek tab edits: `Lighting` in UI units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    pub on: bool,
    pub reverse: bool,
    /// 1..=5 (wire 0..=4).
    pub brightness: u8,
    /// Wire mode: 0 static, 1..=20 effects, 21 custom.
    pub mode: u8,
    /// 1..=5 (wire 0..=4); irrelevant for static and custom.
    pub speed: u8,
    pub sleep_minutes: u8,
    pub static_color: ColorChoice,
    pub dynamic_color: ColorChoice,
    pub colors: [Rgb; 7],
    /// Flag bits the page does not edit, kept as read so applying writes
    /// them back unchanged (`Lighting::extra_flags`).
    pub extra_flags: u8,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            on: true,
            reverse: false,
            brightness: 3,
            mode: MODE_STATIC,
            speed: 3,
            sleep_minutes: 0,
            static_color: ColorChoice::Colorful,
            dynamic_color: ColorChoice::Colorful,
            colors: DEFAULT_PALETTE,
            extra_flags: DEFAULT_EXTRA_FLAGS,
        }
    }
}

/// A neutral starting palette (used until the keyboard's own is read).
pub const DEFAULT_PALETTE: [Rgb; 7] = [
    Rgb {
        r: 0xFF,
        g: 0x3B,
        b: 0x5C,
    },
    Rgb {
        r: 0xFF,
        g: 0xB2,
        b: 0x24,
    },
    Rgb {
        r: 0x3D,
        g: 0xDC,
        b: 0x97,
    },
    Rgb {
        r: 0x2F,
        g: 0xB8,
        b: 0xFF,
    },
    Rgb {
        r: 0x7C,
        g: 0x5C,
        b: 0xFF,
    },
    Rgb {
        r: 0xFF,
        g: 0xFF,
        b: 0xFF,
    },
    Rgb {
        r: 0xFF,
        g: 0x7A,
        b: 0x2F,
    },
];

impl Form {
    /// Wire values to UI units (brightness/speed + 1).
    pub fn from_lighting(l: &Lighting) -> Form {
        Form {
            on: l.on,
            reverse: l.reverse,
            brightness: l.brightness.min(4) + 1,
            mode: l.mode.min(MODE_CUSTOM),
            speed: l.speed.min(4) + 1,
            sleep_minutes: l.sleep_minutes,
            static_color: ColorChoice::from_static(l.static_color),
            dynamic_color: ColorChoice::from_dynamic(l.dynamic_color),
            colors: l.colors,
            extra_flags: l.extra_flags,
        }
    }

    /// UI units to wire values. The speed of static and custom modes is 0,
    /// whatever the slider shows.
    pub fn to_lighting(&self) -> Lighting {
        Lighting {
            on: self.on,
            reverse: self.reverse,
            brightness: self.brightness.clamp(1, 5) - 1,
            mode: self.mode,
            speed: if self.has_speed() {
                self.speed.clamp(1, 5) - 1
            } else {
                0
            },
            sleep_minutes: self.sleep_minutes,
            static_color: self.static_color.to_static(),
            dynamic_color: self.dynamic_color.to_dynamic(),
            colors: self.colors,
            extra_flags: self.extra_flags,
        }
    }

    /// Dynamic effects have a speed; static and custom do not.
    pub fn has_speed(&self) -> bool {
        self.mode != MODE_STATIC && self.mode != MODE_CUSTOM
    }

    /// The colour choice that applies to the current mode (none for custom).
    pub fn choice(&self) -> Option<ColorChoice> {
        match self.mode {
            MODE_STATIC => Some(self.static_color),
            MODE_CUSTOM => None,
            _ => Some(self.dynamic_color),
        }
    }

    pub fn set_choice(&mut self, c: ColorChoice) {
        match self.mode {
            MODE_STATIC => self.static_color = c,
            MODE_CUSTOM => {}
            _ => self.dynamic_color = c,
        }
    }

    /// True when sending this form would change `device`.
    pub fn differs_from(&self, device: &Lighting) -> bool {
        self.to_lighting() != Form::from_lighting(device).to_lighting()
    }
}

/// Indonesian name of a wire mode.
pub fn mode_label(mode: u8) -> &'static str {
    match mode {
        MODE_STATIC => strings::STATIC,
        MODE_CUSTOM => strings::CUSTOM,
        m => strings::EFFECT_LABELS
            .iter()
            .find(|(w, _)| *w == m)
            .map_or("?", |(_, l)| *l),
    }
}

/// Label for a sleep timeout value ("5 menit", "1 jam", "Tidak pernah").
pub fn sleep_label(minutes: u8) -> String {
    match minutes {
        0 => strings::SLEEP_NEVER.to_string(),
        60 => format!("1 {}", strings::SLEEP_HOUR),
        120 => format!("2 {}", strings::SLEEP_HOUR),
        m => format!("{m} {}", strings::SLEEP_MIN),
    }
}

/// `#RRGGBB`.
pub fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (any case).
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let t = s.trim().trim_start_matches('#');
    if !t.is_ascii() {
        return None;
    }
    let digit = |c: &str| u8::from_str_radix(c, 16).ok();
    match t.len() {
        6 => Some(Rgb {
            r: digit(&t[0..2])?,
            g: digit(&t[2..4])?,
            b: digit(&t[4..6])?,
        }),
        3 => {
            let d = |i: usize| digit(&t[i..i + 1]).map(|v| v * 17);
            Some(Rgb {
                r: d(0)?,
                g: d(1)?,
                b: d(2)?,
            })
        }
        _ => None,
    }
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn rgb_of(c: Color32) -> Rgb {
    Rgb {
        r: c.r(),
        g: c.g(),
        b: c.b(),
    }
}

fn rainbow(t: f32) -> Rgb {
    let h = egui::ecolor::Hsva::new(t.clamp(0.0, 1.0) * 0.85, 0.8, 1.0, 1.0);
    rgb_of(Color32::from(h))
}

/// The colour a key shows in the on-screen preview (an approximation of the
/// firmware effect: a static tint or a left-to-right gradient). `x` is the
/// key's position along the board, 0..=1; `per_key` is its custom colour.
/// `None` means "unlit".
pub fn preview_color(form: &Form, x: f32, per_key: Option<Rgb>) -> Option<Rgb> {
    if !form.on {
        return None;
    }
    let base = match form.mode {
        MODE_CUSTOM => per_key?,
        _ => match form.choice() {
            Some(ColorChoice::Palette(i)) => {
                let c = form.colors[usize::from(i.min(6))];
                if form.mode == MODE_STATIC {
                    c
                } else {
                    // A hint of movement: the tint breathes along the board.
                    let k = 0.65 + 0.35 * (x * std::f32::consts::TAU).cos().abs();
                    scale(c, k)
                }
            }
            _ => rainbow(x),
        },
    };
    Some(scale(base, f32::from(form.brightness.clamp(1, 5)) / 5.0))
}

fn scale(c: Rgb, k: f32) -> Rgb {
    let f = |v: u8| (f32::from(v) * k).round().clamp(0.0, 255.0) as u8;
    Rgb {
        r: f(c.r),
        g: f(c.g),
        b: f(c.b),
    }
}

/// Key fill for a lit key: the colour over the key's resting colour.
fn lit_fill(c: Rgb) -> Color32 {
    let a = 0.62;
    let base = color::KEY;
    let mix = |b: u8, v: u8| (f32::from(b) * (1.0 - a) + f32::from(v) * a).round() as u8;
    Color32::from_rgb(mix(base.r(), c.r), mix(base.g(), c.g), mix(base.b(), c.b))
}

// ------------------------------------------------------------------ jobs

/// What the lighting apply job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightingOutcome {
    pub write: Result<(), Error>,
    /// The lighting re-read after the write (also after a failed write).
    pub reread: Result<Lighting, Error>,
}

/// The "Terapkan ke keyboard" job: writes `l` (the device API verifies by
/// reading back), then always re-reads the lighting. A disconnect at any
/// point is returned as `Err(Disconnected)` so the worker reconnects.
pub fn apply_lighting_job<T: Transport>(
    kb: &mut Keyboard<T>,
    l: &Lighting,
) -> Result<LightingOutcome, Error> {
    let write = kb.set_lighting(l);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let reread = kb.lighting();
    if reread == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    Ok(LightingOutcome { write, reread })
}

/// The job that reads the lighting when the page opens.
pub fn read_lighting_job<T: Transport>(kb: &mut Keyboard<T>) -> Result<Lighting, Error> {
    kb.lighting()
}

/// What the page does with a finished lighting apply job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightingEffects {
    /// Freshly read lighting; the form is reset to it.
    pub reread: Option<Lighting>,
    /// Ask the worker for a full reload (the state is unknown).
    pub request_reload: bool,
    /// Indonesian text for the inline error box.
    pub error: Option<String>,
    /// Raw error text, for the "Detail teknis" tooltip.
    pub technical: Option<String>,
    /// The write went through and was verified.
    pub applied: bool,
}

pub fn lighting_effects(result: Result<LightingOutcome, Error>) -> LightingEffects {
    match result {
        Ok(out) => {
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => {
                    let mut msg = i18n::device_error(e);
                    if out.reread.is_ok() {
                        msg = format!("{msg} {}", strings::RELOADED_HINT);
                    }
                    (Some(msg), Some(i18n::technical(e)))
                }
            };
            LightingEffects {
                applied: out.write.is_ok(),
                request_reload: out.reread.is_err(),
                reread: out.reread.ok(),
                error,
                technical,
            }
        }
        Err(Error::Disconnected) => LightingEffects {
            reread: None,
            request_reload: false,
            error: Some(strings::PARTIAL_WRITE.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            applied: false,
        },
        Err(e) => LightingEffects {
            reread: None,
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            applied: false,
        },
    }
}

/// What the per-key apply job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyColorsOutcome {
    pub write: Result<(), Error>,
    /// The colours of the keys that were written, re-read afterwards.
    pub reread: Result<Vec<(u8, Rgb)>, Error>,
}

/// Writes per-key colours (the device API commits after a good batch), then
/// re-reads the written keys, also after a failure.
pub fn apply_key_colors_job<T: Transport>(
    kb: &mut Keyboard<T>,
    colors: &[(u8, Rgb)],
) -> Result<KeyColorsOutcome, Error> {
    let write = kb.set_key_colors(colors);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let keys: Vec<u8> = colors.iter().map(|(k, _)| *k).collect();
    let reread = kb.key_colors(&keys);
    if reread == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    Ok(KeyColorsOutcome { write, reread })
}

/// The job that reads the colours of `keys` when the per-key tab opens.
pub fn read_key_colors_job<T: Transport>(
    kb: &mut Keyboard<T>,
    keys: &[u8],
) -> Result<Vec<(u8, Rgb)>, Error> {
    kb.key_colors(keys)
}

/// What the page does with a finished per-key apply job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyColorEffects {
    /// Colours to store as the keyboard's (and to show).
    pub reread: Vec<(u8, Rgb)>,
    /// The re-read did not finish: read every key again.
    pub reread_failed: bool,
    pub request_reload: bool,
    pub error: Option<String>,
    pub technical: Option<String>,
    pub applied: bool,
}

pub fn key_color_effects(result: Result<KeyColorsOutcome, Error>) -> KeyColorEffects {
    match result {
        Ok(out) => {
            let reread_ok = out.reread.is_ok();
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => {
                    let mut msg = i18n::device_error(e);
                    if reread_ok {
                        msg = format!("{msg} {}", strings::RELOADED_HINT);
                    }
                    msg = format!("{msg} {}", strings::KEYS_NOT_COMMITTED);
                    (Some(msg), Some(i18n::technical(e)))
                }
            };
            KeyColorEffects {
                applied: out.write.is_ok(),
                reread_failed: !reread_ok,
                request_reload: false,
                reread: out.reread.unwrap_or_default(),
                error,
                technical,
            }
        }
        Err(Error::Disconnected) => KeyColorEffects {
            reread: Vec::new(),
            reread_failed: true,
            request_reload: false,
            error: Some(strings::KEYS_PARTIAL.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            applied: false,
        },
        Err(e) => KeyColorEffects {
            reread: Vec::new(),
            reread_failed: true,
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            applied: false,
        },
    }
}

/// The keys whose edited colour differs from the keyboard's, in key order.
pub fn changed_keys(device: &BTreeMap<u8, Rgb>, edit: &BTreeMap<u8, Rgb>) -> Vec<(u8, Rgb)> {
    edit.iter()
        .filter(|(k, c)| device.get(k) != Some(c))
        .map(|(k, c)| (*k, *c))
        .collect()
}

// ------------------------------------------------------------------ page

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Effects,
    Keys,
}

/// An inline error: Indonesian text plus the raw detail.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    title: &'static str,
    text: String,
    technical: Option<String>,
}

pub struct Page {
    tab: Tab,
    form: Form,
    /// The lighting last read from (or confirmed by) the keyboard.
    device: Option<Lighting>,
    /// The latest lighting read worked. "Terapkan" stays off otherwise, so
    /// nothing is written on top of a state that could not be read.
    read_ok: bool,
    /// `Worker::lighting_generation` last looked at.
    seen_lighting: Option<u64>,
    /// The keyboard profile the page shows (lighting is per profile).
    seen_profile: Option<u8>,
    /// The keyboard changed its lighting while the form had unsent edits:
    /// "Terapkan" waits until the user reloads or keeps the edits.
    external_change: bool,
    loading: Option<Pending<Lighting>>,
    loaded_generation: Option<u64>,
    pending: Option<Pending<LightingOutcome>>,
    last_sent: Option<Lighting>,
    failure: Option<Failure>,
    applied: bool,
    /// Palette slot shown in the colour editor.
    slot: u8,
    hex_buf: String,
    hex_editing: bool,

    // per-key tab
    preview_sel: Selection,
    selection: Selection,
    key_device: BTreeMap<u8, Rgb>,
    key_edit: BTreeMap<u8, Rgb>,
    key_loading: Option<Pending<Vec<(u8, Rgb)>>>,
    key_loaded_generation: Option<u64>,
    key_pending: Option<Pending<KeyColorsOutcome>>,
    key_last_sent: Option<Vec<(u8, Rgb)>>,
    key_failure: Option<Failure>,
    key_applied: bool,
    paint: Rgb,
    paint_hex_buf: String,
    paint_hex_editing: bool,
    /// Picker state (hue survives grey and black colours).
    slot_hsva: Hsva,
    paint_hsva: Hsva,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            tab: Tab::Effects,
            form: Form::default(),
            device: None,
            read_ok: false,
            seen_lighting: None,
            seen_profile: None,
            external_change: false,
            loading: None,
            loaded_generation: None,
            pending: None,
            last_sent: None,
            failure: None,
            applied: false,
            slot: 0,
            hex_buf: String::new(),
            hex_editing: false,
            preview_sel: Selection::default(),
            selection: Selection::default(),
            key_device: BTreeMap::new(),
            key_edit: BTreeMap::new(),
            key_loading: None,
            key_loaded_generation: None,
            key_pending: None,
            key_last_sent: None,
            key_failure: None,
            key_applied: false,
            paint: Rgb {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
            },
            paint_hex_buf: String::new(),
            paint_hex_editing: false,
            slot_hsva: Hsva::default(),
            paint_hsva: Hsva::default(),
        }
    }
}

impl Page {
    /// A page that already holds what the keyboard reported for worker
    /// generation `generation`, so it starts without device reads. For
    /// screenshots and tests that run without a keyboard.
    #[doc(hidden)]
    pub fn preloaded(
        generation: u64,
        lighting: Lighting,
        key_colors: impl IntoIterator<Item = (u8, Rgb)>,
    ) -> Page {
        let key_device: BTreeMap<u8, Rgb> = key_colors.into_iter().collect();
        Page {
            form: Form::from_lighting(&lighting),
            device: Some(lighting),
            read_ok: true,
            loaded_generation: Some(generation),
            key_loaded_generation: Some(generation),
            key_edit: key_device.clone(),
            key_device,
            ..Page::default()
        }
    }

    /// Opens the per-key tab (screenshots and tests).
    #[doc(hidden)]
    pub fn show_key_tab(&mut self) {
        self.tab = Tab::Keys;
    }

    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.poll(ctx);
        self.follow_profile(ctx.snapshot.map(|s| s.profile));
        self.follow_lighting(ctx.worker.lighting_generation());
        self.start_reads(ctx);
        page_with_right_panel(
            ui,
            "lighting-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    /// A job of this page is queued or running.
    fn job_pending(&self) -> bool {
        self.loading.is_some()
            || self.pending.is_some()
            || self.key_loading.is_some()
            || self.key_pending.is_some()
    }

    /// Controls may change values: connected, idle, and the lighting was read.
    fn editable(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && !self.job_pending() && self.device.is_some()
    }

    fn keys_editable(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && !self.job_pending() && self.key_loaded_generation.is_some()
    }

    /// "Terapkan ke keyboard" may send the form: the lighting was read
    /// (the latest read worked) and the form differs from it.
    fn effects_ready(&self) -> bool {
        self.read_ok && !self.external_change && self.device.is_some() && self.dirty()
    }

    /// The keyboard reported a lighting change of its own (Fn + light keys)
    /// when `lighting_generation` moved. Without unsent edits the lighting
    /// (and the key colours) are simply read again; with edits they are
    /// kept and the page says the keyboard changed (see
    /// [`Page::reload_from_keyboard`] and [`Page::keep_edits`]).
    fn follow_lighting(&mut self, lighting_generation: u64) {
        let moved = self.seen_lighting.is_some_and(|g| g != lighting_generation);
        self.seen_lighting = Some(lighting_generation);
        if !moved {
            return;
        }
        if self.dirty() {
            self.external_change = true;
        } else {
            self.loaded_generation = None;
        }
        // Key colours keep unsent paint on a re-read (see `finish_key_load`).
        self.key_loaded_generation = None;
    }

    /// Lighting and key colours belong to a profile: on a switch (top bar,
    /// keyboard, per-app rule) unsent edits and paint of the old profile
    /// are dropped, with the last apply's result, and everything is read
    /// again.
    fn follow_profile(&mut self, profile: Option<u8>) {
        let Some(p) = profile else {
            return;
        };
        let moved = self.seen_profile.is_some_and(|s| s != p);
        self.seen_profile = Some(p);
        if !moved {
            return;
        }
        if let Some(d) = &self.device {
            self.form = Form::from_lighting(d);
        }
        self.external_change = false;
        self.loaded_generation = None;
        self.failure = None;
        self.applied = false;
        self.last_sent = None;
        self.key_device.clear();
        self.key_edit.clear();
        self.key_loaded_generation = None;
        self.key_failure = None;
        self.key_applied = false;
        self.key_last_sent = None;
    }

    /// "Muat ulang" after a change on the keyboard: drop the edits and read
    /// the lighting again.
    fn reload_from_keyboard(&mut self) {
        if let Some(d) = &self.device {
            self.form = Form::from_lighting(d);
        }
        self.external_change = false;
        self.loaded_generation = None;
    }

    /// "Abaikan": keep the edits; "Terapkan" sends them as they are.
    fn keep_edits(&mut self) {
        self.external_change = false;
    }

    /// The key colours were read. A key whose colour on screen was not
    /// painted (it equals what the keyboard had) follows the keyboard;
    /// unsent paint is kept.
    fn finish_key_load(&mut self, r: Result<Vec<(u8, Rgb)>, Error>) {
        match r {
            Ok(colors) => {
                for (k, c) in colors {
                    let painted = self
                        .key_edit
                        .get(&k)
                        .is_some_and(|e| self.key_device.get(&k) != Some(e));
                    if !painted {
                        self.key_edit.insert(k, c);
                    }
                    self.key_device.insert(k, c);
                }
                self.key_failure = None;
            }
            Err(Error::Disconnected) => self.key_loaded_generation = None,
            Err(e) => {
                self.key_failure = Some(Failure {
                    title: strings::KEYS_LOAD_FAILED,
                    text: i18n::device_error(&e),
                    technical: Some(i18n::technical(&e)),
                });
            }
        }
    }

    /// A lighting read finished. Unsent edits are kept.
    fn finish_load(&mut self, r: Result<Lighting, Error>) {
        match r {
            Ok(l) => {
                let keep_edits = self.dirty();
                self.device = Some(l);
                if !keep_edits {
                    self.form = Form::from_lighting(&l);
                }
                self.failure = None;
                self.read_ok = true;
            }
            Err(Error::Disconnected) => {
                self.loaded_generation = None;
                self.read_ok = false;
            }
            Err(e) => {
                self.read_ok = false;
                self.failure = Some(Failure {
                    title: strings::LOAD_FAILED,
                    text: i18n::device_error(&e),
                    technical: Some(i18n::technical(&e)),
                });
            }
        }
    }

    fn dirty(&self) -> bool {
        self.device
            .as_ref()
            .is_some_and(|d| self.form.differs_from(d))
    }

    // ------------------------------------------------------------ jobs

    fn poll(&mut self, ctx: &AppCtx) {
        if let Some(p) = self.loading.as_mut() {
            if let Some(r) = p.poll() {
                self.loading = None;
                self.finish_load(r);
            }
        }
        if let Some(p) = self.pending.as_mut() {
            if let Some(r) = p.poll() {
                self.pending = None;
                let fx = lighting_effects(r);
                self.read_ok = fx.reread.is_some();
                if let Some(l) = fx.reread {
                    self.device = Some(l);
                    self.form = Form::from_lighting(&l);
                } else if fx.request_reload || fx.error.is_some() {
                    // The state is unknown until it is read again.
                    self.loaded_generation = None;
                    if fx.request_reload {
                        self.device = None;
                    }
                }
                if fx.request_reload {
                    ctx.worker.request_reload();
                }
                self.failure = fx.error.map(|text| Failure {
                    title: strings::APPLY_FAILED,
                    text,
                    technical: fx.technical,
                });
                self.applied = fx.applied;
            }
        }
        if let Some(p) = self.key_loading.as_mut() {
            if let Some(r) = p.poll() {
                self.key_loading = None;
                self.finish_key_load(r);
            }
        }
        if let Some(p) = self.key_pending.as_mut() {
            if let Some(r) = p.poll() {
                self.key_pending = None;
                let fx = key_color_effects(r);
                for (k, c) in &fx.reread {
                    self.key_device.insert(*k, *c);
                    // The screen shows what the keyboard holds now.
                    self.key_edit.insert(*k, *c);
                }
                if fx.reread_failed {
                    self.key_loaded_generation = None;
                }
                if fx.request_reload {
                    ctx.worker.request_reload();
                }
                self.key_failure = fx.error.map(|text| Failure {
                    title: strings::KEYS_APPLY_FAILED,
                    text,
                    technical: fx.technical,
                });
                self.key_applied = fx.applied;
            }
        }
    }

    /// Reads the lighting (and, on the per-key tab, the key colours) once
    /// per device generation.
    fn start_reads(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else {
            return;
        };
        if !ctx.can_write() || self.job_pending() {
            return;
        }
        let generation = ctx.worker.generation();
        if self.loaded_generation != Some(generation) {
            self.loaded_generation = Some(generation);
            self.loading = Some(ctx.worker.submit("lighting-read", read_lighting_job));
            return;
        }
        if self.tab == Tab::Keys && self.key_loaded_generation != Some(generation) {
            self.key_loaded_generation = Some(generation);
            let keys: Vec<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
            self.key_loading = Some(ctx.worker.submit("lighting-key-read", move |kb| {
                read_key_colors_job(kb, &keys)
            }));
        }
    }

    fn submit_lighting(&mut self, ctx: &AppCtx, l: Lighting) {
        self.failure = None;
        self.applied = false;
        self.last_sent = Some(l);
        self.pending = Some(
            ctx.worker
                .submit("lighting-apply", move |kb| apply_lighting_job(kb, &l)),
        );
    }

    fn submit_key_colors(&mut self, ctx: &AppCtx, colors: Vec<(u8, Rgb)>) {
        self.key_failure = None;
        self.key_applied = false;
        self.key_last_sent = Some(colors.clone());
        self.key_pending = Some(ctx.worker.submit("lighting-key-apply", move |kb| {
            apply_key_colors_job(kb, &colors)
        }));
    }

    // --------------------------------------------------------- centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let editable = self.editable(ctx);
        ui.horizontal(|ui| {
            ui.set_min_height(40.0);
            let tabs = [
                (Tab::Effects, strings::TAB_EFFECTS),
                (Tab::Keys, strings::TAB_KEYS),
            ];
            Segmented::new(&tabs).height(42.0).show(ui, &mut self.tab);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if toggle(ui, &mut self.form.on, editable).changed() {
                    self.applied = false;
                }
                ui.label(RichText::new(strings::LIGHT_ON).color(color::MUTED));
            });
        });
        ui.add_space(12.0);

        let model = ctx
            .snapshot
            .map_or(openhe_proto::Model::Win68HeMax, |s| s.model);
        let fallback;
        let matrix = match ctx.snapshot {
            Some(s) => &s.matrix,
            None => {
                fallback = reference_matrix(model);
                &fallback
            }
        };
        let lay = layout(model, matrix);
        let width = lay.width.max(1.0);
        let xs: BTreeMap<u8, f32> = lay
            .keys
            .iter()
            .map(|k| (k.code, ((k.x + k.w / 2.0) / width).clamp(0.0, 1.0)))
            .collect();

        let form = self.form;
        let per_tab = self.tab;
        let key_edit = &self.key_edit;
        let decorate = |code: u8| {
            let x = xs.get(&code).copied().unwrap_or(0.5);
            let fill = match per_tab {
                Tab::Effects => preview_color(&form, x, key_edit.get(&code).copied()).map(lit_fill),
                // The per-key tab always shows the paint itself.
                Tab::Keys => key_edit.get(&code).copied().map(lit_fill),
            };
            KeyDecoration {
                fill,
                ..KeyDecoration::default()
            }
        };
        ui.vertical_centered(|ui| match self.tab {
            Tab::Effects => {
                KeyboardView::new(model, matrix)
                    .id_salt("lighting-preview")
                    .enabled(true)
                    .show(ui, &mut self.preview_sel, decorate);
                // The preview is not a picker.
                self.preview_sel.clear();
            }
            Tab::Keys => {
                KeyboardView::new(model, matrix)
                    .id_salt("lighting-keys")
                    .enabled(ctx.connected())
                    .show(ui, &mut self.selection, decorate);
            }
        });
        if !ctx.connected() {
            ui.add_space(8.0);
            card::hint(ui, strings::OFFLINE);
        }
        ui.add_space(16.0);

        match self.tab {
            Tab::Effects => self.effects_card(ui, editable),
            Tab::Keys => self.keys_card(ctx, ui, matrix),
        }
    }

    fn effects_card(&mut self, ui: &mut Ui, editable: bool) {
        Card::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                card::title(
                    ui,
                    format!("{} · {}", strings::EFFECTS_TITLE, 2 + EFFECTS.len()),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(strings::EFFECTS_HINT)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            });
            ui.add_space(12.0);
            let mut modes: Vec<u8> = vec![MODE_STATIC];
            modes.extend(EFFECTS.iter().map(|(m, _)| *m));
            modes.push(MODE_CUSTOM);
            const COLS: usize = 6;
            let gap = 10.0;
            let w = (ui.available_width() - gap * (COLS as f32 - 1.0)) / COLS as f32;
            for row in modes.chunks(COLS) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for &m in row {
                        if tile(ui, w, mode_label(m), self.form.mode == m, editable).clicked() {
                            self.form.mode = m;
                            self.applied = false;
                        }
                    }
                });
            }
            ui.add_space(8.0);
            card::hint(ui, strings::EFFECTS_NOTE);
        });
    }

    fn keys_card(&mut self, ctx: &AppCtx, ui: &mut Ui, matrix: &openhe_proto::Matrix) {
        let editable = self.keys_editable(ctx);
        Card::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if tool_button(ui, strings::KEYS_SELECT_ALL, ctx.connected()).clicked() {
                    self.selection.select_all(matrix);
                }
                if tool_button(
                    ui,
                    strings::KEYS_CLEAR,
                    ctx.connected() && !self.selection.is_empty(),
                )
                .clicked()
                {
                    self.selection.clear();
                }
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        self.selection.len(),
                        strings::KEYS_SELECTED
                    ))
                    .color(color::MUTED),
                );
            });
            ui.add_space(6.0);
            card::hint(ui, strings::KEYS_HINT);
            if self.key_loading.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    card::hint(ui, strings::KEYS_LOADING);
                });
            }
            if self.form.mode != MODE_CUSTOM && self.device.is_some() {
                ui.add_space(6.0);
                card::hint(ui, strings::KEYS_MODE_HINT);
                if ui
                    .add_enabled(
                        self.editable(ctx),
                        egui::Button::new(strings::KEYS_USE_CUSTOM),
                    )
                    .clicked()
                {
                    self.form.mode = MODE_CUSTOM;
                    self.tab = Tab::Effects;
                }
            }
            let _ = editable;
        });
    }

    // ---------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("lighting-apply")
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
                    .show(ui, |ui| match self.tab {
                        Tab::Effects => self.effect_settings(ctx, ui),
                        Tab::Keys => self.paint_settings(ctx, ui),
                    });
            });
    }

    fn effect_settings(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let editable = self.editable(ctx);
        ui.label(
            RichText::new(mode_label(self.form.mode))
                .font(font::heading())
                .color(color::TEXT)
                .strong(),
        );
        let desc = match self.form.mode {
            MODE_STATIC => strings::DESC_STATIC,
            MODE_CUSTOM => strings::DESC_CUSTOM,
            _ => strings::DESC_EFFECT,
        };
        card::hint(ui, desc);
        if self.loading.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                card::hint(ui, strings::LOADING);
            });
        }
        ui.add_space(12.0);

        // ---- colour
        card::section(ui, strings::COLOR);
        match self.form.choice() {
            None => card::hint(ui, strings::COLOR_CUSTOM_HINT),
            Some(choice) => {
                let kinds = [(false, strings::COLOR_RAINBOW), (true, strings::COLOR_ONE)];
                let mut one = matches!(choice, ColorChoice::Palette(_));
                if Segmented::new(&kinds)
                    .full_width(true)
                    .height(40.0)
                    .enabled(editable)
                    .show(ui, &mut one)
                    .changed()
                {
                    self.form.set_choice(if one {
                        ColorChoice::Palette(self.slot.min(6))
                    } else {
                        ColorChoice::Colorful
                    });
                    self.applied = false;
                }
                if let ColorChoice::Palette(i) = self.form.choice().unwrap_or(choice) {
                    self.slot = i.min(6);
                    ui.add_space(8.0);
                    self.palette_editor(ui, editable);
                }
            }
        }
        ui.add_space(12.0);

        // ---- brightness
        let mut changed = false;
        value_header(
            ui,
            strings::BRIGHTNESS,
            // Levels 1..=5 read as 20..100 %, as in the mockup.
            &format!("{}%", u32::from(self.form.brightness.clamp(1, 5)) * 20),
        );
        changed |= level_slider(ui, &mut self.form.brightness, editable);
        ui.add_space(8.0);

        // ---- speed
        let speed_ok = editable && self.form.has_speed();
        value_header(ui, strings::SPEED, &format!("{} / 5", self.form.speed));
        changed |= level_slider(ui, &mut self.form.speed, speed_ok);
        if !self.form.has_speed() {
            card::hint(ui, strings::SPEED_OFF_HINT);
        }
        ui.add_space(8.0);

        // ---- direction and sleep
        ui.columns(2, |cols| {
            card::section(&mut cols[0], strings::DIRECTION);
            let dirs = [
                (false, strings::DIRECTION_FWD),
                (true, strings::DIRECTION_REV),
            ];
            changed |= Segmented::new(&dirs)
                .full_width(true)
                .height(40.0)
                .enabled(editable && self.form.has_speed())
                .show(&mut cols[0], &mut self.form.reverse)
                .changed();

            card::section(&mut cols[1], strings::SLEEP);
            let before = self.form.sleep_minutes;
            let ui2 = &mut cols[1];
            ui2.add_enabled_ui(editable, |ui| {
                egui::ComboBox::from_id_salt("lighting-sleep")
                    .width(ui.available_width() - 8.0)
                    .selected_text(sleep_label(self.form.sleep_minutes))
                    .show_ui(ui, |ui| {
                        for m in SLEEP_MINUTES {
                            ui.selectable_value(&mut self.form.sleep_minutes, m, sleep_label(m));
                        }
                    });
            });
            changed |= before != self.form.sleep_minutes;
        });
        if !self.form.has_speed() {
            card::hint(ui, strings::DIRECTION_HINT);
        }
        if changed {
            self.applied = false;
        }
        ui.add_space(8.0);
        card::hint(ui, strings::UNVERIFIED);
    }

    /// 7 swatches, a picker and a hex box for the selected palette slot.
    fn palette_editor(&mut self, ui: &mut Ui, editable: bool) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for i in 0..7u8 {
                let selected = self.slot == i;
                let (r, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        editable,
                        selected,
                        format!("{} {}", strings::COLOR, i + 1),
                    )
                });
                let radius = CornerRadius::same(8);
                ui.painter()
                    .rect_filled(r, radius, c32(self.form.colors[usize::from(i)]));
                if selected {
                    ui.painter().rect_stroke(
                        r.expand(2.0),
                        CornerRadius::same(10),
                        line(2.0, color::ACCENT),
                        StrokeKind::Outside,
                    );
                }
                if resp.clicked() && editable {
                    self.slot = i;
                    self.form.set_choice(ColorChoice::Palette(i));
                    self.applied = false;
                    self.hex_editing = false;
                }
            }
        });
        ui.add_space(8.0);
        let slot = usize::from(self.slot.min(6));
        if color_field(
            ui,
            &mut self.slot_hsva,
            &mut self.form.colors[slot],
            editable,
        ) {
            self.applied = false;
        }
        ui.add_space(6.0);
        if hex_box(
            ui,
            &mut self.hex_buf,
            &mut self.hex_editing,
            &mut self.form.colors[slot],
            editable,
        ) {
            self.applied = false;
        }
        card::hint(ui, strings::PALETTE_HINT);
    }

    fn paint_settings(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let editable = self.keys_editable(ctx);
        ui.label(
            RichText::new(strings::KEYS_PAINT)
                .font(font::heading())
                .color(color::TEXT)
                .strong(),
        );
        card::hint(ui, strings::KEYS_PAINT_DESC);
        ui.add_space(12.0);

        card::section(ui, strings::KEYS_PALETTE);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (i, c) in self.form.colors.into_iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        self.paint == c,
                        format!("{} {}", strings::KEYS_PALETTE, i + 1),
                    )
                });
                ui.painter().rect_filled(r, CornerRadius::same(8), c32(c));
                if self.paint == c {
                    ui.painter().rect_stroke(
                        r.expand(2.0),
                        CornerRadius::same(10),
                        line(2.0, color::ACCENT),
                        StrokeKind::Outside,
                    );
                }
                if resp.clicked() {
                    self.paint = c;
                    self.paint_hex_editing = false;
                }
            }
        });
        ui.add_space(8.0);
        color_field(ui, &mut self.paint_hsva, &mut self.paint, true);
        ui.add_space(6.0);
        hex_box(
            ui,
            &mut self.paint_hex_buf,
            &mut self.paint_hex_editing,
            &mut self.paint,
            true,
        );
        ui.add_space(12.0);

        let selected = ctx
            .snapshot
            .map(|s| self.selection.codes_in(&s.matrix))
            .unwrap_or_default();
        let paint = self.paint;
        let mut painted = false;
        ui.add_enabled_ui(editable, |ui| {
            if wide_button(ui, strings::KEYS_PAINT_SELECTED, !selected.is_empty()).clicked() {
                for k in &selected {
                    self.key_edit.insert(*k, paint);
                }
                painted = true;
            }
            if wide_button(ui, strings::KEYS_BLACK, !selected.is_empty()).clicked() {
                for k in &selected {
                    self.key_edit.insert(*k, Rgb::default());
                }
                painted = true;
            }
            if wide_button(ui, strings::KEYS_PAINT_ALL, true).clicked() {
                if let Some(s) = ctx.snapshot {
                    for (_, _, k) in s.matrix.keys() {
                        self.key_edit.insert(k, paint);
                    }
                    painted = true;
                }
            }
        });
        if painted {
            self.key_applied = false;
        }
        let n = changed_keys(&self.key_device, &self.key_edit).len();
        ui.add_space(8.0);
        card::hint(ui, format!("{n} {}", strings::KEYS_CHANGED));
        card::hint(ui, strings::UNVERIFIED);
    }

    fn apply_area(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let idle = ctx.can_write() && !self.job_pending();
        let (failure, retry_ok) = match self.tab {
            // Re-sending also waits for a lighting read that worked.
            Tab::Effects => (
                self.failure.clone(),
                self.last_sent.is_some() && self.read_ok,
            ),
            Tab::Keys => (self.key_failure.clone(), self.key_last_sent.is_some()),
        };
        if let Some(f) = failure {
            let reload = matches!(f.title, strings::LOAD_FAILED | strings::KEYS_LOAD_FAILED);
            let retry_label = (idle && (retry_ok || reload)).then_some(i18n::common::RETRY);
            if card::error_box(ui, f.title, &f.text, retry_label, f.technical.as_deref()) {
                self.retry(ctx, reload);
            }
            ui.add_space(8.0);
        }

        if self.external_change && self.tab == Tab::Effects {
            Card::new()
                .fill(color::ACCENT_SOFT)
                .stroke(color::ACCENT.gamma_multiply(0.5))
                .padding(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(strings::EXTERNAL_CHANGE).color(color::TEXT),
                        )
                        .wrap(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(idle, egui::Button::new(i18n::common::RELOAD))
                            .clicked()
                        {
                            self.reload_from_keyboard();
                        }
                        if ui.button(strings::IGNORE).clicked() {
                            self.keep_edits();
                        }
                    });
                });
            ui.add_space(8.0);
        }
        match self.tab {
            Tab::Effects => {
                let can = idle && self.effects_ready();
                if card::primary_button(ui, strings::APPLY, can).clicked() {
                    let l = self.form.to_lighting();
                    self.submit_lighting(ctx, l);
                }
                ui.add_space(6.0);
                let busy = self.pending.is_some();
                let applied = self.applied && !self.dirty();
                status_line(
                    ui,
                    busy,
                    applied,
                    strings::APPLYING,
                    strings::APPLIED,
                    || {
                        if self.dirty() {
                            strings::UNSENT.to_string()
                        } else {
                            strings::NO_CHANGES.to_string()
                        }
                    },
                );
            }
            Tab::Keys => {
                let changed = changed_keys(&self.key_device, &self.key_edit);
                let can = idle && self.key_loaded_generation.is_some() && !changed.is_empty();
                if card::primary_button(ui, strings::KEYS_APPLY, can).clicked() {
                    self.submit_key_colors(ctx, changed);
                }
                ui.add_space(6.0);
                let busy = self.key_pending.is_some();
                status_line(
                    ui,
                    busy,
                    self.key_applied,
                    strings::APPLYING,
                    strings::KEYS_APPLIED,
                    || strings::NO_CHANGES.to_string(),
                );
            }
        }
    }

    fn retry(&mut self, ctx: &AppCtx, reload: bool) {
        if reload {
            match self.tab {
                Tab::Effects => {
                    self.loaded_generation = None;
                    self.failure = None;
                }
                Tab::Keys => {
                    self.key_loaded_generation = None;
                    self.key_failure = None;
                }
            }
            return;
        }
        match self.tab {
            Tab::Effects => {
                if let Some(l) = self.last_sent {
                    self.form = Form::from_lighting(&l);
                    self.submit_lighting(ctx, l);
                }
            }
            Tab::Keys => {
                if let Some(c) = self.key_last_sent.clone() {
                    for (k, v) in &c {
                        self.key_edit.insert(*k, *v);
                    }
                    self.submit_key_colors(ctx, c);
                }
            }
        }
    }
}

// --------------------------------------------------------------- widgets

fn status_line(
    ui: &mut Ui,
    busy: bool,
    applied: bool,
    applying: &str,
    done: &str,
    idle_text: impl FnOnce() -> String,
) {
    ui.vertical_centered(|ui| {
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(applying)
                        .font(font::small())
                        .color(color::MUTED),
                );
            });
        } else if applied {
            ui.label(
                RichText::new(done)
                    .font(font::small())
                    .color(color::SUCCESS),
            );
        } else {
            ui.label(
                RichText::new(idle_text())
                    .font(font::small())
                    .color(color::MUTED),
            );
        }
    });
}

/// "Label ............ value" on one row.
fn value_header(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(color::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).color(color::TEXT).strong());
        });
    });
}

/// A 1..=5 slider; returns true when changed.
fn level_slider(ui: &mut Ui, v: &mut u8, enabled: bool) -> bool {
    ui.add_enabled_ui(enabled, |ui| {
        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
        ui.spacing_mut().slider_rail_height = 4.0;
        // The part up to the thumb in the accent colour (as `SliderMm`).
        ui.visuals_mut().selection.bg_fill = color::ACCENT;
        let r = ui.add(
            egui::Slider::new(v, 1..=5)
                .show_value(false)
                .integer()
                .trailing_fill(true),
        );
        // A solid thumb instead of egui's hollow one: drawn over it, at the
        // same place (egui: radius = height / 2.5, range shrunk by it).
        let rect = r.rect;
        let radius = rect.height() / 2.5;
        let t = f32::from((*v).clamp(1, 5) - 1) / 4.0;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let fill = if enabled { color::TEXT } else { color::MUTED };
        ui.painter().circle(
            egui::pos2(x, rect.center().y),
            radius * 0.7,
            fill,
            line(2.0, color::BG),
        );
        r.changed()
    })
    .inner
}

/// A hex text box bound to `value`; returns true when the colour changed.
fn hex_box(
    ui: &mut Ui,
    buf: &mut String,
    editing: &mut bool,
    value: &mut Rgb,
    enabled: bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(strings::HEX).color(color::MUTED));
        if !*editing {
            *buf = hex(*value);
        }
        let resp = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(buf)
                .desired_width(110.0)
                .font(font::value()),
        );
        if resp.gained_focus() || resp.has_focus() {
            *editing = true;
        }
        if resp.changed() {
            if let Some(c) = parse_hex(buf) {
                *value = c;
                changed = true;
            }
        }
        if resp.lost_focus() {
            *editing = false;
        }
    });
    changed
}

/// A wide saturation/value field with a hue bar underneath (the mockup's
/// picker). `hsva` keeps the hue while `value` is grey or black; when
/// `value` changed elsewhere (hex box, another swatch) it is re-derived.
/// Returns true when the user changed the colour.
fn color_field(ui: &mut Ui, hsva: &mut Hsva, value: &mut Rgb, enabled: bool) -> bool {
    if rgb_of(Color32::from(*hsva)) != *value {
        *hsva = Hsva::from(c32(*value));
    }
    let mut changed = false;
    let w = ui.available_width() - 4.0;
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };

    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 140.0), sense);
    if let (true, Some(p)) = (enabled, resp.interact_pointer_pos()) {
        hsva.s = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsva.v = ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0);
        changed = true;
    }
    let mut mesh = egui::Mesh::default();
    const NX: usize = 24;
    const NY: usize = 12;
    for yi in 0..=NY {
        for xi in 0..=NX {
            let (s, v) = (xi as f32 / NX as f32, 1.0 - yi as f32 / NY as f32);
            let pos = egui::pos2(
                egui::lerp(rect.x_range(), s),
                egui::lerp(rect.y_range(), 1.0 - v),
            );
            mesh.colored_vertex(pos, Color32::from(Hsva::new(hsva.h, s, v, 1.0)));
        }
    }
    for yi in 0..NY {
        for xi in 0..NX {
            let i = (yi * (NX + 1) + xi) as u32;
            let row = (NX + 1) as u32;
            mesh.add_triangle(i, i + 1, i + row);
            mesh.add_triangle(i + 1, i + row + 1, i + row);
        }
    }
    let radius = CornerRadius::same(size::CONTROL_RADIUS);
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(
        rect,
        radius,
        line(1.0, color::BORDER_STRONG),
        StrokeKind::Inside,
    );
    let knob = egui::pos2(
        egui::lerp(rect.x_range(), hsva.s),
        egui::lerp(rect.y_range(), 1.0 - hsva.v),
    );
    ui.painter()
        .circle_stroke(knob, 7.0, line(2.0, Color32::WHITE));
    ui.painter()
        .circle_stroke(knob, 8.5, line(1.0, Color32::from_black_alpha(160)));

    ui.add_space(6.0);
    let (hrect, hresp) = ui.allocate_exact_size(egui::vec2(w, 16.0), sense);
    if let (true, Some(p)) = (enabled, hresp.interact_pointer_pos()) {
        hsva.h = ((p.x - hrect.left()) / hrect.width()).clamp(0.0, 1.0);
        changed = true;
    }
    let mut bar = egui::Mesh::default();
    const NH: usize = 24;
    for i in 0..=NH {
        let t = i as f32 / NH as f32;
        let c = Color32::from(Hsva::new(t, 1.0, 1.0, 1.0));
        let x = egui::lerp(hrect.x_range(), t);
        bar.colored_vertex(egui::pos2(x, hrect.top()), c);
        bar.colored_vertex(egui::pos2(x, hrect.bottom()), c);
        if i < NH {
            let k = (2 * i) as u32;
            bar.add_triangle(k, k + 1, k + 2);
            bar.add_triangle(k + 1, k + 3, k + 2);
        }
    }
    ui.painter().add(egui::Shape::mesh(bar));
    ui.painter().rect_stroke(
        hrect,
        CornerRadius::same(8),
        line(1.0, color::BORDER_STRONG),
        StrokeKind::Inside,
    );
    let hx = egui::lerp(hrect.x_range(), hsva.h);
    ui.painter().circle_filled(
        egui::pos2(hx, hrect.center().y),
        hrect.height() / 2.0 + 2.0,
        Color32::from(Hsva::new(hsva.h, 1.0, 1.0, 1.0)),
    );
    ui.painter().circle_stroke(
        egui::pos2(hx, hrect.center().y),
        hrect.height() / 2.0 + 2.0,
        line(2.0, Color32::WHITE),
    );

    if changed {
        *value = rgb_of(Color32::from(*hsva));
    }
    changed
}

fn tool_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(color::TEXT))
            .fill(color::BG)
            .stroke(line(1.0, color::BORDER_STRONG))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .min_size(egui::vec2(0.0, 36.0)),
    )
}

fn wide_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(color::TEXT))
            .fill(color::BG)
            .stroke(line(1.0, color::BORDER_STRONG))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .min_size(egui::vec2(ui.available_width(), 38.0)),
    )
}

/// One effect tile of the grid.
fn tile(ui: &mut Ui, width: f32, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 54.0), sense);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label)
    });
    let hovered = resp.hovered() && enabled;
    let (fill, stroke, text) = if selected {
        (color::ACCENT_SOFT, color::ACCENT, color::ACCENT)
    } else if hovered {
        (color::KEY_HOVER, color::MUTED, color::TEXT)
    } else {
        (color::KEY, color::BORDER_STRONG, color::TEXT)
    };
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(
        rect,
        CornerRadius::same(size::CONTROL_RADIUS + 2),
        dim(fill),
        line(if selected { 1.5 } else { 1.0 }, dim(stroke)),
        StrokeKind::Inside,
    );
    // Long effect names wrap inside the tile instead of running out of it.
    let mut job =
        egui::text::LayoutJob::simple(label.to_string(), font::body(), dim(text), width - 12.0);
    job.halign = egui::Align::Center;
    let galley = ui.fonts(|f| f.layout_job(job));
    let pos = egui::pos2(rect.center().x, rect.center().y - galley.size().y / 2.0);
    ui.painter()
        .with_clip_rect(rect.shrink(1.0))
        .galley(pos, galley, dim(text));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::lighting;
    use openhe_proto::model::Model;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn sample() -> Lighting {
        Lighting {
            on: true,
            reverse: true,
            brightness: 3,
            mode: 9,
            speed: 2,
            sleep_minutes: 10,
            static_color: 7,
            dynamic_color: 3,
            colors: DEFAULT_PALETTE,
            extra_flags: 0xC0,
        }
    }

    // ---- UI state <-> Lighting

    #[test]
    fn unknown_flag_bits_pass_through_the_form() {
        // Read from the keyboard: kept, so applying writes them back.
        let f = Form::from_lighting(&sample());
        assert_eq!(f.to_lighting().extra_flags, 0xC0);
        // Built from scratch: the official app's 0x10, as before.
        let fresh = Form::default().to_lighting();
        assert_eq!(fresh.extra_flags, 0x10);
        let p = openhe_proto::lighting::set(&fresh).unwrap().payload;
        assert_eq!(p[37], 0x11, "on + bit4");
    }

    #[test]
    fn brightness_and_speed_map_one_to_five_onto_zero_to_four() {
        let l = sample();
        let f = Form::from_lighting(&l);
        assert_eq!((f.brightness, f.speed), (4, 3));
        assert_eq!(f.to_lighting(), l);
        let low = Form {
            brightness: 1,
            speed: 1,
            ..f
        };
        let high = Form {
            brightness: 5,
            speed: 5,
            ..f
        };
        assert_eq!(
            (low.to_lighting().brightness, low.to_lighting().speed),
            (0, 0)
        );
        assert_eq!(
            (high.to_lighting().brightness, high.to_lighting().speed),
            (4, 4)
        );
    }

    #[test]
    fn speed_is_zero_for_static_and_custom() {
        for mode in [MODE_STATIC, MODE_CUSTOM] {
            let f = Form {
                mode,
                speed: 5,
                ..Form::from_lighting(&sample())
            };
            assert!(!f.has_speed());
            assert_eq!(f.to_lighting().speed, 0);
        }
        let f = Form {
            mode: 3,
            speed: 5,
            ..Form::from_lighting(&sample())
        };
        assert!(f.has_speed());
        assert_eq!(f.to_lighting().speed, 4);
        // The result always passes the device's own validation.
        for mode in 0..=21 {
            for speed in 1..=5 {
                for brightness in 1..=5 {
                    let l = Form {
                        mode,
                        speed,
                        brightness,
                        ..Form::default()
                    }
                    .to_lighting();
                    assert_eq!(l.validate(), Ok(()), "mode {mode}");
                }
            }
        }
    }

    #[test]
    fn colour_choices_map_to_palette_indices() {
        assert_eq!(ColorChoice::from_static(0), ColorChoice::Palette(0));
        assert_eq!(ColorChoice::from_static(6), ColorChoice::Palette(6));
        assert_eq!(ColorChoice::from_static(7), ColorChoice::Colorful);
        assert_eq!(ColorChoice::Colorful.to_static(), 7);
        assert_eq!(ColorChoice::Palette(4).to_static(), 4);
        assert_eq!(ColorChoice::from_dynamic(0), ColorChoice::Colorful);
        assert_eq!(ColorChoice::from_dynamic(1), ColorChoice::Palette(0));
        assert_eq!(ColorChoice::from_dynamic(7), ColorChoice::Palette(6));
        assert_eq!(ColorChoice::Colorful.to_dynamic(), 0);
        assert_eq!(ColorChoice::Palette(0).to_dynamic(), 1);
        assert_eq!(ColorChoice::Palette(6).to_dynamic(), 7);
        for v in 0..=7 {
            assert_eq!(ColorChoice::from_static(v).to_static(), v);
            assert_eq!(ColorChoice::from_dynamic(v).to_dynamic(), v);
        }
    }

    #[test]
    fn the_colour_choice_follows_the_mode() {
        let mut f = Form::from_lighting(&sample());
        assert_eq!(f.choice(), Some(ColorChoice::Palette(2))); // dynamic 3
        f.set_choice(ColorChoice::Palette(0));
        assert_eq!(f.to_lighting().dynamic_color, 1);
        assert_eq!(f.to_lighting().static_color, 7);
        f.mode = MODE_STATIC;
        assert_eq!(f.choice(), Some(ColorChoice::Colorful));
        f.set_choice(ColorChoice::Palette(5));
        assert_eq!(f.to_lighting().static_color, 5);
        f.mode = MODE_CUSTOM;
        assert_eq!(f.choice(), None);
        f.set_choice(ColorChoice::Palette(1));
        assert_eq!(f.to_lighting().static_color, 5, "custom edits no choice");
    }

    #[test]
    fn palette_edits_reach_the_wire_in_natural_rgb() {
        let mut f = Form::from_lighting(&sample());
        f.colors[3] = rgb(1, 2, 3);
        assert_eq!(f.to_lighting().colors[3], rgb(1, 2, 3));
        // The proto crate turns it into B, G, R, 0xFF on the wire.
        let req = lighting::set(&f.to_lighting()).unwrap();
        assert_eq!(&req.payload[5 + 3 * 4..5 + 3 * 4 + 4], &[3, 2, 1, 0xFF]);
    }

    #[test]
    fn dirty_ignores_the_speed_of_static_and_custom() {
        let mut device = sample();
        device.mode = MODE_STATIC;
        device.speed = 0;
        let f = Form::from_lighting(&device);
        assert!(!f.differs_from(&device));
        let g = Form { speed: 5, ..f };
        assert!(!g.differs_from(&device), "speed is not sent for static");
        let h = Form { brightness: 1, ..f };
        assert!(h.differs_from(&device));
    }

    #[test]
    fn sleep_options_are_exactly_the_allowed_values() {
        assert_eq!(sleep_label(0), "Tidak pernah");
        assert_eq!(sleep_label(5), "5 menit");
        assert_eq!(sleep_label(60), "1 jam");
        assert_eq!(sleep_label(120), "2 jam");
        for m in SLEEP_MINUTES {
            let l = Form {
                sleep_minutes: m,
                ..Form::default()
            }
            .to_lighting();
            assert_eq!(l.validate(), Ok(()));
        }
    }

    // ---- labels

    #[test]
    fn every_effect_has_a_distinct_indonesian_label() {
        for (i, (mode, english)) in EFFECTS.iter().enumerate() {
            let (m, label) = strings::EFFECT_LABELS[i];
            assert_eq!(m, *mode, "label table is in wire order");
            assert!(!label.is_empty());
            if *english != "Laser" {
                assert_ne!(label, *english);
            }
            assert_eq!(mode_label(*mode), label);
        }
        let mut seen: Vec<&str> = strings::EFFECT_LABELS.iter().map(|(_, l)| *l).collect();
        seen.push(strings::STATIC);
        seen.push(strings::CUSTOM);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 22);
        assert_eq!(mode_label(MODE_STATIC), "Statis");
        assert_eq!(mode_label(MODE_CUSTOM), "Kustom");
    }

    // ---- hex

    #[test]
    fn hex_round_trip_and_parsing() {
        assert_eq!(hex(rgb(0xFF, 0x3B, 0x5C)), "#FF3B5C");
        assert_eq!(parse_hex("#ff3b5c"), Some(rgb(0xFF, 0x3B, 0x5C)));
        assert_eq!(parse_hex("FF3B5C"), Some(rgb(0xFF, 0x3B, 0x5C)));
        assert_eq!(parse_hex(" #0af "), Some(rgb(0x00, 0xAA, 0xFF)));
        for bad in ["", "#", "#12", "#12345", "#1234567", "#GG0000", "é12345"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
        let c = rgb(1, 2, 3);
        assert_eq!(parse_hex(&hex(c)), Some(c));
    }

    // ---- preview

    #[test]
    fn preview_follows_power_mode_and_brightness() {
        let mut f = Form {
            mode: MODE_STATIC,
            static_color: ColorChoice::Palette(0),
            brightness: 5,
            ..Form::default()
        };
        assert_eq!(preview_color(&f, 0.3, None), Some(f.colors[0]));
        f.brightness = 1;
        let dim = preview_color(&f, 0.3, None).unwrap();
        assert!(dim.r < f.colors[0].r && dim.r > 0);
        f.on = false;
        assert_eq!(preview_color(&f, 0.3, None), None);
        // Custom shows the per-key colour, or nothing without one.
        let c = Form {
            mode: MODE_CUSTOM,
            brightness: 5,
            ..Form::default()
        };
        assert_eq!(
            preview_color(&c, 0.5, Some(rgb(9, 8, 7))),
            Some(rgb(9, 8, 7))
        );
        assert_eq!(preview_color(&c, 0.5, None), None);
        // Colourful static is a rainbow: the ends differ.
        let r = Form {
            mode: MODE_STATIC,
            static_color: ColorChoice::Colorful,
            brightness: 5,
            ..Form::default()
        };
        assert_ne!(preview_color(&r, 0.0, None), preview_color(&r, 1.0, None));
    }

    #[test]
    fn changed_keys_are_the_edits_that_differ() {
        let dev: BTreeMap<u8, Rgb> = [(1, rgb(1, 1, 1)), (2, rgb(2, 2, 2))].into();
        let mut edit = dev.clone();
        assert!(changed_keys(&dev, &edit).is_empty());
        edit.insert(2, rgb(9, 9, 9));
        edit.insert(3, rgb(3, 3, 3));
        assert_eq!(
            changed_keys(&dev, &edit),
            vec![(2, rgb(9, 9, 9)), (3, rgb(3, 3, 3))]
        );
    }

    // ---- jobs (MockTransport)

    fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    fn nack_reply() -> [u8; 64] {
        reply(0x00, &[0, 0x47, 1, 0xFF])
    }

    fn prgb_reply(l: &Lighting) -> [u8; 64] {
        reply(0x18, &lighting::set(l).unwrap().payload)
    }

    fn krgb_reply(key: u8, c: Rgb) -> [u8; 64] {
        reply(0x2A, &[0, key, c.r, c.g, c.b])
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn apply_lighting_success_rereads_and_resets_the_form() {
        let l = sample();
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&l)); // SET
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&l)); // verify read
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&l)); // our re-read
        let out = apply_lighting_job(&mut kb(m), &l).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.reread, Ok(l));
        let fx = lighting_effects(Ok(out));
        assert!(fx.applied && !fx.request_reload);
        assert_eq!(fx.reread, Some(l));
        assert_eq!(fx.error, None);
    }

    #[test]
    fn apply_lighting_verify_failure_shows_the_true_state() {
        let l = sample();
        let mut other = l;
        other.brightness = 0;
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&l)); // SET
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&other)); // verify read: did not stick
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&other)); // our re-read
        let out = apply_lighting_job(&mut kb(m), &l).unwrap();
        assert!(matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed")));
        assert_eq!(out.reread, Ok(other));
        let fx = lighting_effects(Ok(out));
        assert!(!fx.applied && !fx.request_reload);
        assert_eq!(
            fx.reread,
            Some(other),
            "the form shows what the keyboard holds"
        );
        assert_eq!(
            fx.error.as_deref(),
            Some(format!("{} {}", i18n::error::VERIFY, strings::RELOADED_HINT).as_str())
        );
        assert!(fx.technical.unwrap().contains("verify failed"));
    }

    #[test]
    fn apply_lighting_failed_write_is_still_reread() {
        let l = sample();
        let mut m = MockTransport::new();
        m.push_reply(reply(0x7F, &[])); // device failure on SET
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&l));
        let out = apply_lighting_job(&mut kb(m), &l).unwrap();
        assert_eq!(out.write, Err(Error::Frame(FrameError::DeviceFail)));
        assert_eq!(out.reread, Ok(l));
    }

    #[test]
    fn apply_lighting_failing_reread_asks_for_a_reload() {
        let l = sample();
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&l));
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&l)); // verify ok; no replies left for the re-read
        let out = apply_lighting_job(&mut kb(m), &l).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.reread, Err(Error::Timeout));
        let fx = lighting_effects(Ok(out));
        assert!(fx.applied && fx.request_reload);
        assert_eq!(fx.reread, None);
    }

    #[test]
    fn invalid_lighting_is_refused_before_io_then_reread() {
        let mut l = sample();
        l.sleep_minutes = 4;
        let mut m = MockTransport::new();
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&sample()));
        let mut k = kb(m);
        let out = apply_lighting_job(&mut k, &l).unwrap();
        assert!(matches!(out.write, Err(Error::InvalidInput(_))));
        assert_eq!(out.reread, Ok(sample()));
    }

    /// Fails every write after `n`, like a keyboard pulled mid-job.
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
    fn disconnect_during_apply_is_returned_as_disconnected() {
        let l = sample();
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&l));
        let mut k = Keyboard::new(
            UnplugAfter {
                inner: m,
                writes_left: 1, // the SET goes through, the verify read fails
            },
            Model::Win68HeMax,
        );
        let r = apply_lighting_job(&mut k, &l);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = lighting_effects(r);
        assert_eq!(fx.error.as_deref(), Some(strings::PARTIAL_WRITE));
        assert!(!fx.applied && !fx.request_reload && fx.reread.is_none());
    }

    #[test]
    fn other_lighting_job_errors_reload_everything() {
        let fx = lighting_effects(Err(Error::Io(format!(
            "{}: boom",
            crate::worker::INTERNAL_ERROR
        ))));
        assert!(fx.request_reload);
        assert_eq!(fx.error.as_deref(), Some(i18n::error::INTERNAL));
    }

    #[test]
    fn read_job_returns_the_device_lighting() {
        let l = sample();
        let mut m = MockTransport::new();
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&l));
        assert_eq!(read_lighting_job(&mut kb(m)), Ok(l));
    }

    #[test]
    fn key_colors_apply_success_commits_and_rereads() {
        let colors = [(0x0A, rgb(1, 2, 3)), (0x0B, rgb(4, 5, 6))];
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, colors[0].1));
        m.push_reply(krgb_reply(0x0B, colors[1].1));
        m.push_reply(krgb_reply(0xFE, rgb(0xFE, 0xFE, 0xFE))); // commit
        m.push_reply(krgb_reply(0x0A, colors[0].1)); // re-read
        m.push_reply(krgb_reply(0x0B, colors[1].1));
        let out = apply_key_colors_job(&mut kb(m), &colors).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.reread, Ok(colors.to_vec()));
        let fx = key_color_effects(Ok(out));
        assert!(fx.applied && !fx.reread_failed);
        assert_eq!(fx.reread, colors.to_vec());
        assert_eq!(fx.error, None);
    }

    #[test]
    fn key_colors_echo_mismatch_is_shown_with_the_reread_values() {
        let colors = [(0x0A, rgb(1, 2, 3))];
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, rgb(9, 9, 9))); // wrong echo; no commit
        m.push_reply(krgb_reply(0x0A, rgb(7, 7, 7))); // re-read
        let out = apply_key_colors_job(&mut kb(m), &colors).unwrap();
        assert!(matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed")));
        let fx = key_color_effects(Ok(out));
        assert!(!fx.applied && !fx.reread_failed);
        assert_eq!(fx.reread, vec![(0x0A, rgb(7, 7, 7))]);
        let text = fx.error.unwrap();
        assert!(text.starts_with(i18n::error::VERIFY));
        assert!(text.contains(strings::KEYS_NOT_COMMITTED));
    }

    #[test]
    fn key_colors_failing_reread_asks_for_a_full_read() {
        let colors = [(0x0A, rgb(1, 2, 3))];
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, colors[0].1));
        m.push_reply(krgb_reply(0xFE, rgb(0xFE, 0xFE, 0xFE)));
        let out = apply_key_colors_job(&mut kb(m), &colors).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.reread, Err(Error::Timeout));
        let fx = key_color_effects(Ok(out));
        assert!(fx.applied && fx.reread_failed && fx.reread.is_empty());
    }

    #[test]
    fn key_colors_disconnect_is_a_partial_write_message() {
        let colors = [(0x0A, rgb(1, 2, 3)), (0x0B, rgb(4, 5, 6))];
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, colors[0].1));
        let mut k = Keyboard::new(
            UnplugAfter {
                inner: m,
                writes_left: 1,
            },
            Model::Win68HeMax,
        );
        let r = apply_key_colors_job(&mut k, &colors);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = key_color_effects(r);
        assert_eq!(fx.error.as_deref(), Some(strings::KEYS_PARTIAL));
        assert!(fx.reread_failed && !fx.applied);
    }

    #[test]
    fn read_key_colors_job_reads_every_key() {
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, rgb(1, 2, 3)));
        m.push_reply(krgb_reply(0x0B, rgb(4, 5, 6)));
        assert_eq!(
            read_key_colors_job(&mut kb(m), &[0x0A, 0x0B]),
            Ok(vec![(0x0A, rgb(1, 2, 3)), (0x0B, rgb(4, 5, 6))])
        );
    }

    // ---- rendering

    #[test]
    fn apply_needs_a_lighting_read_that_worked() {
        let mut page = Page::default();
        assert!(!page.effects_ready(), "nothing read yet");
        page.finish_load(Ok(sample()));
        page.form.brightness = 1; // an edit
        assert!(page.effects_ready());
        // A later read fails (e.g. the keyboard reported a value out of
        // range): the edits stay, but nothing is sent until a read works.
        page.finish_load(Err(Error::Io("unexpected lighting reply".into())));
        assert!(!page.effects_ready());
        assert_eq!(page.form.brightness, 1, "edits kept");
        assert!(page.failure.is_some());
        page.finish_load(Ok(sample()));
        assert!(page.effects_ready());
        assert_eq!(page.form.brightness, 1, "still kept: unsent edits");
    }

    #[test]
    fn an_out_of_range_lighting_read_gets_its_own_message() {
        // What the device layer reports for a PRGB reply with brightness 9.
        let mut body = openhe_proto::lighting::set(&sample()).unwrap().payload;
        body[38] = 9;
        let mut m = MockTransport::new();
        m.push_reply(nack_reply());
        m.push_reply(reply(0x18, &body));
        let e = read_lighting_job(&mut kb(m)).unwrap_err();
        let mut page = Page::default();
        page.finish_load(Err(e));
        let f = page.failure.clone().unwrap();
        assert!(
            f.text.starts_with(i18n::error::LIGHTING_UNKNOWN),
            "{}",
            f.text
        );
        assert!(f.text.contains("kecerahan"), "{}", f.text);
        assert!(f.text.contains(i18n::error::LIGHTING_RECOVER), "{}", f.text);
        assert!(!f.text.contains("brightness"), "no English: {}", f.text);
        assert!(!page.effects_ready(), "the parse is not loosened");
        // Each field gets its Indonesian name; other I/O errors stay generic.
        for (detail, name) in [
            ("mode 30 out of range 0..=21", "efek"),
            ("speed 7 out of range 0..=4", "kecepatan"),
            ("sleep timeout 4 min is not one of 0, 1", "mati otomatis"),
            ("static colour index 9 out of range 0..=7", "warna"),
            ("dynamic colour index 9 out of range 0..=7", "warna"),
        ] {
            let e = Error::Io(format!("unexpected lighting reply: {detail}"));
            assert!(i18n::device_error(&e).contains(name), "{detail}");
        }
        assert_eq!(
            i18n::device_error(&Error::Io(
                "unexpected lighting reply: reply is not a PRGB reply".into()
            )),
            i18n::error::IO
        );
    }

    #[test]
    fn a_lighting_change_on_the_keyboard_is_read_again_unless_edited() {
        let mut page = Page::preloaded(3, sample(), [(0x04, rgb(1, 2, 3))]);
        page.follow_lighting(0); // first look: nothing to compare
        assert_eq!(page.loaded_generation, Some(3));
        // Fn+light on the keyboard, no edits here: read it again.
        page.follow_lighting(1);
        assert_eq!(page.loaded_generation, None);
        assert_eq!(page.key_loaded_generation, None);
        assert!(!page.external_change);

        // With unsent edits: keep them, say so, and hold "Terapkan" back
        // until the user picks.
        page.loaded_generation = Some(3);
        page.key_loaded_generation = Some(3);
        page.form.brightness = 1;
        page.follow_lighting(2);
        assert_eq!(page.loaded_generation, Some(3), "edits are not replaced");
        assert!(page.external_change);
        assert!(!page.effects_ready());
        page.follow_lighting(2);
        assert!(page.external_change);
        // "Muat ulang": drop the edits and read the keyboard.
        page.reload_from_keyboard();
        assert!(!page.external_change);
        assert_eq!(page.loaded_generation, None);
        assert_eq!(page.form, Form::from_lighting(&sample()));
        // Or keep them on purpose: "Terapkan" is possible again.
        page.loaded_generation = Some(3);
        page.form.brightness = 1;
        page.follow_lighting(3);
        page.keep_edits();
        assert!(!page.external_change);
        assert!(page.effects_ready());
    }

    #[test]
    fn a_key_colour_reread_keeps_only_real_edits() {
        let mut page = Page::preloaded(3, sample(), [(0x04, rgb(1, 1, 1)), (0x05, rgb(2, 2, 2))]);
        page.key_edit.insert(0x05, rgb(9, 9, 9)); // painted, unsent
        page.finish_key_load(Ok(vec![(0x04, rgb(7, 7, 7)), (0x05, rgb(8, 8, 8))]));
        assert_eq!(page.key_edit[&0x04], rgb(7, 7, 7), "follows the keyboard");
        assert_eq!(page.key_edit[&0x05], rgb(9, 9, 9), "unsent paint kept");
        assert_eq!(
            changed_keys(&page.key_device, &page.key_edit),
            vec![(0x05, rgb(9, 9, 9))]
        );
    }

    #[test]
    fn the_page_renders_offline_and_connected() {
        use crate::app::test_support::{offline_worker, run_frames, snapshot68};
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;
        let worker = offline_worker();
        let theme = Theme::default();
        let data = RefCell::new(AppData::default());
        let egui_ctx = egui::Context::default();
        crate::theme::apply(&egui_ctx);
        let mut page = Page::default();
        for connected in [false, true] {
            let snap = snapshot68();
            let status = if connected {
                ConnectionStatus::Connected(snap.clone())
            } else {
                ConnectionStatus::Searching
            };
            for tab in [Tab::Effects, Tab::Keys] {
                page.tab = tab;
                run_frames(&egui_ctx, 2, |c| {
                    egui::CentralPanel::default().show(c, |ui| {
                        let ctx = AppCtx {
                            worker: &worker,
                            snapshot: connected.then_some(&snap),
                            status: &status,
                            busy: false,
                            theme: &theme,
                            data: &data,
                        };
                        page.ui(&ctx, ui);
                    });
                });
            }
        }
    }
}

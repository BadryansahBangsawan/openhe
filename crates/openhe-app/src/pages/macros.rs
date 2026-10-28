//! Macro: a local library of recorded key sequences, a recorder, a step
//! editor, and "Pasang ke tombol".
//!
//! Mockup `docs/design/mockups/04-macro.png`. The library lives in
//! [`crate::storage::AppData::macros`] (host side only, like AULA HUB). While
//! "Rekam" is active the page turns egui key press/release events into
//! `MacroAction`s with the real time between events ([`Recorder`]). Binding
//! sends one macro to one key on the Base layer with
//! `Keyboard::bind_macro` ([`bind_job`]), then re-reads that key's macro
//! settings and mode, also after a failed write.
//!
//! **Unverified on hardware:** the official app writes every macro at
//! address 256, so binding a new macro may replace the one on another key
//! (PROTOCOL open question 8), and the meaning of the per-action delay
//! (before or after the action) follows the mockup ("jeda setelahnya") but
//! is not confirmed. The UI says so.

use super::advanced::Func;
use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::storage::{SavedMacro, SavedMacroAction};
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{KeyboardView, Selection};
use crate::worker::{DeviceSnapshot, Pending};
use eframe::egui::{
    self, Align, Align2, CornerRadius, Event, Frame, Key, Layout, Margin, Modifiers, RichText,
    Sense, StrokeKind, Ui,
};
use openhe_device::keyboard::macros::MacroBinding;
use openhe_device::{Error, KeySettings, Keyboard, Transport};
use openhe_proto::keycodes::{self, Category};
use openhe_proto::macros::{MacroAction, PlayMode, MAX_ACTIONS, MAX_U24};

/// "Lepas" (key up) in the step table, as in the mockup.
const RELEASE_COLOR: egui::Color32 = egui::Color32::from_rgb(0xF4, 0x72, 0xB6);

mod strings {
    pub const LIBRARY: &str = "Macro";
    pub const NEW: &str = "+ Baru";
    pub const NEW_NAME: &str = "Macro baru";
    pub const COPY_SUFFIX: &str = "salinan";
    pub const LIBRARY_EMPTY: &str = "Belum ada macro. Tekan Baru untuk membuat satu.";
    pub const LIBRARY_NOTE: &str =
        "Daftar macro disimpan di komputer ini. Hanya macro yang dipasang ke tombol yang dikirim ke keyboard.";
    pub const NO_MACRO_SELECTED: &str = "Pilih atau buat macro di daftar sebelah kiri.";
    pub const STEPS: &str = "langkah";
    pub const TOTAL: &str = "Total";
    pub const SECONDS: &str = "detik";
    pub const NAME_HINT: &str = "Nama macro";
    pub const DUPLICATE: &str = "Duplikat";
    pub const DELETE: &str = "Hapus";
    pub const DELETE_CONFIRM: &str = "Yakin hapus?";
    pub const CLEAR: &str = "Kosongkan langkah";

    pub const RECORD: &str = "Rekam";
    pub const STOP: &str = "Berhenti";
    pub const RECORDING: &str =
        "Merekam. Tekan tombol di keyboard komputer; pencet Berhenti kalau selesai.";
    pub const RECORD_APPENDS: &str = "Hasil rekaman ditambahkan di akhir langkah.";
    pub const RECORD_FULL: &str = "Batas 255 langkah tercapai; rekaman dihentikan.";
    pub const RECORD_INTERRUPTED: &str = "Rekaman dihentikan karena halaman ditinggalkan.";
    pub const ADD_STEP: &str = "+ Tambah langkah";
    pub const ADD_KEY: &str = "Tombol";
    pub const ADD_PRESS: &str = "Tekan";
    pub const ADD_RELEASE: &str = "Lepas";
    pub const ADD_TAP: &str = "Tekan + lepas";
    pub const ADD_WHERE_END: &str = "Ditambahkan di akhir.";
    pub const ADD_WHERE_AFTER: &str = "Ditambahkan setelah langkah";

    pub const COL_STEP: &str = "#";
    pub const COL_ACTION: &str = "AKSI";
    pub const COL_KEY: &str = "TOMBOL";
    pub const COL_DELAY: &str = "JEDA SETELAHNYA";
    pub const PRESS: &str = "Tekan";
    pub const RELEASE: &str = "Lepas";
    pub const MS: &str = "ms";
    pub const NO_STEPS: &str = "Belum ada langkah. Tekan Rekam atau Tambah langkah.";
    pub const UP: &str = "Naikkan";
    pub const DOWN: &str = "Turunkan";
    pub const REMOVE: &str = "Hapus langkah";
    pub const DELAY_HINT: &str = "Jeda setelah langkah ini sebelum langkah berikutnya.";

    pub const SKIPPED: &str = "Tombol ini dilewati karena belum didukung perekam:";
    pub const UNBALANCED: &str = "Ada tombol yang ditekan tetapi tidak dilepas:";
    pub const UNBALANCED_BLOCK: &str =
        "Tambahkan pelepasan untuk tombol yang masih ditekan sebelum memasang, supaya tidak ada tombol yang macet.";
    pub const ADD_RELEASES: &str = "Tambahkan pelepasan otomatis";
    pub const TOO_MANY: &str = "Terlalu banyak langkah (maksimum 255).";
    pub const EMPTY_MACRO: &str = "Macro belum punya langkah.";
    pub const DELAY_TOO_LONG: &str = "Jeda terlalu panjang untuk keyboard.";
    pub const SAVE_FAILED: &str = "Gagal menyimpan daftar macro";

    pub const PLAY_TITLE: &str = "Cara diputar";
    pub const PLAY_DESC: &str = "Berlaku saat macro dipasang ke tombol";
    pub const MODE_ONCE: &str = "Sekali per tekan";
    pub const MODE_ONCE_DESC: &str = "Tekan sekali, jalan sekali";
    pub const MODE_TOGGLE: &str = "Nyala/mati";
    pub const MODE_TOGGLE_DESC: &str = "Tekan untuk mulai, tekan lagi untuk berhenti";
    pub const MODE_HELD: &str = "Selama ditahan";
    pub const MODE_HELD_DESC: &str = "Berulang sampai tombol dilepas";
    pub const MODE_HELD_FINISH: &str = "Ditahan, lalu tuntaskan";
    pub const MODE_HELD_FINISH_DESC: &str = "Setelah dilepas, selesaikan putaran dulu";
    pub const REPEAT: &str = "Jumlah ulang";
    pub const INTERVAL: &str = "Jeda antar ulangan";
    pub const REPEAT_HINT: &str = "Berapa kali urutan dijalankan tiap tekan.";

    pub const TARGET: &str = "Dipasang di tombol";
    pub const TARGET_NONE: &str = "Belum dipilih";
    pub const TARGET_LAYER: &str = "layer Base";
    pub const CHANGE: &str = "Ganti";
    pub const PICK: &str = "Pilih tombol";
    pub const PICK_TITLE: &str = "Pilih tombol untuk macro";
    pub const PICK_HINT: &str = "Klik tombol di keyboard. Macro dipasang di layer Base.";
    pub const PICK_CLOSE: &str = "Batal";
    pub const FN_BLOCKED: &str = "Tombol Fn tidak bisa diberi macro.";
    pub const CURRENT: &str = "Sekarang di tombol ini";
    pub const NO_BINDING: &str = "Belum ada macro";
    pub const BINDING_LOADING: &str = "Membaca…";
    pub const BINDING_FAILED: &str = "Gagal membaca macro di tombol ini";
    pub const WARN_REPLACE: &str =
        "Peringatan: memasang macro baru bisa menggantikan macro di tombol lain. Perilaku ini belum diuji di keyboard sungguhan.";
    pub const WARN_DELAY: &str =
        "Arti jeda (sebelum atau sesudah langkah) belum diuji di keyboard sungguhan.";

    pub const BIND: &str = "Pasang ke tombol";
    pub const REPLACES: &str = "Tombol ini punya fungsi lanjutan";
    pub const REPLACES_TAIL: &str = "Memasang macro menghapus fungsi itu.";
    pub const REPLACES_SOCD: &str = "Pasangan SOCD-nya ikut dilepas.";
    pub const CONFIRM_REPLACE: &str = "Ya, ganti dengan macro";
    pub const BINDING: &str = "Memasang…";
    pub const BOUND: &str = "Macro terpasang di keyboard";
    pub const BIND_FAILED: &str = "Gagal memasang macro";
    pub const RELOADED_HINT: &str = "Isi tombol dibaca ulang dari keyboard.";
    pub const PARTIAL_WRITE: &str =
        "Keyboard terputus. Macro mungkin terpasang sebagian; isi tombol dibaca ulang saat tersambung.";
    pub const OFFLINE: &str = "Sambungkan keyboard untuk memasang macro.";
    pub const READY: &str = "Pilih macro dan tombol, lalu pasang.";
}

// ------------------------------------------------------------ key mapping

/// The HID key code (PROTOCOL 4.3) of an egui key, `None` for keys the
/// recorder cannot map (the Copy/Cut/Paste pseudo-keys, F25..F35, browser
/// keys). Clipboard shortcuts arrive as events instead: see [`Clip`].
pub fn hid_code(key: Key) -> Option<u16> {
    use Key::*;
    Some(match key {
        A => 0x04,
        B => 0x05,
        C => 0x06,
        D => 0x07,
        E => 0x08,
        F => 0x09,
        G => 0x0A,
        H => 0x0B,
        I => 0x0C,
        J => 0x0D,
        K => 0x0E,
        L => 0x0F,
        M => 0x10,
        N => 0x11,
        O => 0x12,
        P => 0x13,
        Q => 0x14,
        R => 0x15,
        S => 0x16,
        T => 0x17,
        U => 0x18,
        V => 0x19,
        W => 0x1A,
        X => 0x1B,
        Y => 0x1C,
        Z => 0x1D,
        Num1 => 0x1E,
        Num2 => 0x1F,
        Num3 => 0x20,
        Num4 => 0x21,
        Num5 => 0x22,
        Num6 => 0x23,
        Num7 => 0x24,
        Num8 => 0x25,
        Num9 => 0x26,
        Num0 => 0x27,
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        // "+" shares the "=" key.
        Equals | Plus => 0x2E,
        OpenBracket | OpenCurlyBracket => 0x2F,
        CloseBracket | CloseCurlyBracket => 0x30,
        Backslash | Pipe => 0x31,
        Semicolon | Colon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash | Questionmark => 0x38,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        F13 => 0x68,
        F14 => 0x69,
        F15 => 0x6A,
        F16 => 0x6B,
        F17 => 0x6C,
        F18 => 0x6D,
        F19 => 0x6E,
        F20 => 0x6F,
        F21 => 0x70,
        F22 => 0x71,
        F23 => 0x72,
        F24 => 0x73,
        _ => return None,
    })
}

/// Modifier key codes in the order of [`Mods`]: Ctrl, Shift, Alt, GUI
/// (egui cannot tell left from right, so the left key is recorded).
pub const MODIFIER_CODES: [u16; 4] = [0xE0, 0xE1, 0xE2, 0xE3];

/// Which modifiers are down: Ctrl, Shift, Alt, GUI (Cmd on macOS; egui
/// does not report the Windows key).
pub type Mods = [bool; 4];

pub fn mods_of(m: &Modifiers) -> Mods {
    [m.ctrl, m.shift, m.alt, m.mac_cmd]
}

// --------------------------------------------------------------- recorder

/// A clipboard shortcut. egui-winit turns its key press into
/// `Event::Copy`, `Event::Cut` or `Event::Paste` and sends no key event for
/// it (Ctrl/Cmd + C, X, V; on Windows also Ctrl+Insert, Shift+Delete and
/// Shift+Insert). Only the release arrives as a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Cut,
    Paste,
}

const HID_C: u16 = 0x06;
const HID_X: u16 = 0x1B;
const HID_V: u16 = 0x19;
const HID_INSERT: u16 = 0x49;
const HID_DELETE: u16 = 0x4C;

impl Clip {
    /// The keys that can make this shortcut.
    fn keys(self) -> [u16; 2] {
        match self {
            Clip::Copy => [HID_C, HID_INSERT],
            Clip::Cut => [HID_X, HID_DELETE],
            Clip::Paste => [HID_V, HID_INSERT],
        }
    }

    /// The most likely key under `m`; the release corrects it.
    fn guess(self, m: &Modifiers) -> u16 {
        let k = self.keys();
        // Shift without Ctrl/Cmd is Windows' Shift+Delete / Shift+Insert.
        if self != Clip::Copy && m.shift && !m.command {
            k[1]
        } else {
            k[0]
        }
    }

    /// The shortcut a key `code` makes under `m`, if any.
    fn of(code: u16, m: &Modifiers) -> Option<Clip> {
        match code {
            HID_C if m.command => Some(Clip::Copy),
            HID_X if m.command => Some(Clip::Cut),
            HID_V if m.command => Some(Clip::Paste),
            HID_INSERT if m.ctrl => Some(Clip::Copy),
            HID_INSERT if m.shift => Some(Clip::Paste),
            HID_DELETE if m.shift => Some(Clip::Cut),
            _ => None,
        }
    }
}

/// What a finished recording holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub actions: Vec<MacroAction>,
    /// Names of keys that were pressed but cannot be recorded.
    pub skipped: Vec<String>,
}

/// Turns key events into macro actions. The delay of an action is the time
/// until the next recorded action ("jeda setelahnya"), measured between the
/// timestamps passed in (seconds); the last action has delay 0.
#[derive(Debug)]
pub struct Recorder {
    actions: Vec<MacroAction>,
    last_t: Option<f64>,
    held: Vec<u16>,
    mods: Mods,
    skipped: Vec<String>,
    /// Most actions this recording may hold, releases of held keys
    /// included.
    room: usize,
    /// Presses recorded from a clipboard event: the action index, the
    /// shortcut, and the guessed key that its release may correct.
    clips: Vec<(usize, Clip, u16)>,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder {
            actions: Vec::new(),
            last_t: None,
            held: Vec::new(),
            mods: Mods::default(),
            skipped: Vec::new(),
            room: MAX_ACTIONS,
            clips: Vec::new(),
        }
    }
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// A recorder that adds at most `room` actions (releases included), for
    /// appending to a macro that already has steps.
    pub fn with_room(room: usize) -> Recorder {
        Recorder {
            room: room.min(MAX_ACTIONS),
            ..Recorder::default()
        }
    }

    pub fn actions(&self) -> &[MacroAction] {
        &self.actions
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// No room for another press and its release (the keyboard stores at
    /// most 255 actions). The release of every key still held is always
    /// kept free, so a recording that hits the limit still ends with every
    /// key released.
    pub fn is_full(&self) -> bool {
        self.actions.len() + self.held.len() + 2 > self.room
    }

    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// Records a press of `code` (and holds it) if there is room for it and
    /// its release; false when full.
    fn press(&mut self, code: u16, t: f64) -> bool {
        if self.is_full() {
            return false;
        }
        self.held.push(code);
        self.push(code, true, t);
        true
    }

    /// Records the release of a held key (its room was kept free).
    fn release(&mut self, at: usize, t: f64) {
        let code = self.held.remove(at);
        self.push(code, false, t);
    }

    fn push(&mut self, code: u16, down: bool, t: f64) {
        if let (Some(last_t), Some(prev)) = (self.last_t, self.actions.last_mut()) {
            let ms = ((t - last_t) * 1000.0).round();
            prev.delay_ms = ms.clamp(0.0, f64::from(MAX_U24)) as u32;
        }
        self.last_t = Some(t);
        self.actions.push(MacroAction {
            code,
            down,
            delay_ms: 0,
        });
    }

    /// Records modifier presses and releases that changed since the last call.
    pub fn modifiers(&mut self, now: Mods, t: f64) {
        for i in 0..4 {
            if now[i] == self.mods[i] {
                continue;
            }
            self.mods[i] = now[i];
            let code = MODIFIER_CODES[i];
            if now[i] {
                if !self.held.contains(&code) {
                    self.press(code, t);
                }
            } else if let Some(p) = self.held.iter().position(|c| *c == code) {
                self.release(p, t);
            }
        }
    }

    /// One egui key event. `physical` (the key's position) is preferred over
    /// the layout-dependent `key`. Repeats and releases of keys that were not
    /// pressed during the recording are ignored; unsupported keys are noted.
    pub fn key(
        &mut self,
        key: Key,
        physical: Option<Key>,
        pressed: bool,
        repeat: bool,
        modifiers: &Modifiers,
        t: f64,
    ) {
        // A modifier that went down or up before this key comes first.
        self.modifiers(mods_of(modifiers), t);
        if repeat {
            return;
        }
        let Some(code) = physical.and_then(hid_code).or_else(|| hid_code(key)) else {
            let name = format!("{:?}", physical.unwrap_or(key));
            if pressed && !self.skipped.contains(&name) {
                self.skipped.push(name);
            }
            return;
        };
        if pressed {
            if self.held.contains(&code) {
                return;
            }
            self.press(code, t);
            return;
        }
        // The release of a clipboard shortcut names the real key.
        if !self.held.contains(&code) {
            if let Some(i) = self.clips.iter().position(|c| c.1.keys().contains(&code)) {
                let (at, _, guess) = self.clips[i];
                if let Some(h) = self.held.iter_mut().find(|h| **h == guess) {
                    *h = code;
                    self.actions[at].code = code;
                }
            } else if Clip::of(code, modifiers).is_some() {
                // A paste with an empty clipboard sends no event at all for
                // the press: record it now.
                self.press(code, t);
            }
        }
        self.clips.retain(|c| self.actions[c.0].code != code);
        if let Some(p) = self.held.iter().position(|c| *c == code) {
            self.release(p, t);
        }
    }

    /// A clipboard event (the press of its shortcut key); `modifiers` are
    /// the ones down in that frame.
    pub fn clipboard(&mut self, clip: Clip, modifiers: &Modifiers, t: f64) {
        self.modifiers(mods_of(modifiers), t);
        let code = clip.guess(modifiers);
        let held = clip.keys().iter().any(|k| self.held.contains(k));
        if held {
            return; // OS key repeat
        }
        if self.press(code, t) {
            self.clips.push((self.actions.len() - 1, clip, code));
        }
    }

    /// Ends the recording at time `t`. Keys still held get a release, so no
    /// key stays pressed on the keyboard.
    pub fn finish(mut self, t: f64) -> Recording {
        for code in std::mem::take(&mut self.held) {
            self.push(code, false, t);
        }
        Recording {
            actions: self.actions,
            skipped: self.skipped,
        }
    }
}

// ----------------------------------------------------------- step editor

/// Default delay of a step added by hand, in ms.
pub const DEFAULT_STEP_DELAY_MS: u32 = 10;

pub fn step_set_delay(steps: &mut [SavedMacroAction], i: usize, ms: u32) {
    if let Some(s) = steps.get_mut(i) {
        s.delay_ms = ms.min(MAX_U24);
    }
}

pub fn step_toggle(steps: &mut [SavedMacroAction], i: usize) {
    if let Some(s) = steps.get_mut(i) {
        s.down = !s.down;
    }
}

/// Deletes step `i`; returns the index that should stay selected.
pub fn step_delete(steps: &mut Vec<SavedMacroAction>, i: usize) -> Option<usize> {
    if i >= steps.len() {
        return None;
    }
    steps.remove(i);
    (!steps.is_empty()).then(|| i.min(steps.len() - 1))
}

/// Moves step `i` one place up; returns its new index.
pub fn step_move_up(steps: &mut [SavedMacroAction], i: usize) -> usize {
    if i == 0 || i >= steps.len() {
        return i;
    }
    steps.swap(i, i - 1);
    i - 1
}

/// Moves step `i` one place down; returns its new index.
pub fn step_move_down(steps: &mut [SavedMacroAction], i: usize) -> usize {
    if i + 1 >= steps.len() {
        return i;
    }
    steps.swap(i, i + 1);
    i + 1
}

/// Inserts `action`s after step `after` (at the end when `None`); returns
/// the index of the last inserted step. Stops at the 255-action limit.
pub fn step_insert(
    steps: &mut Vec<SavedMacroAction>,
    after: Option<usize>,
    actions: &[SavedMacroAction],
) -> Option<usize> {
    let start = after.map_or(steps.len(), |i| (i + 1).min(steps.len()));
    let mut last = None;
    for (n, a) in actions.iter().enumerate() {
        if steps.len() >= MAX_ACTIONS {
            break;
        }
        steps.insert(start + n, *a);
        last = Some(start + n);
    }
    last
}

/// A press and a release of `code`, `hold_ms` apart.
pub fn tap(code: u16, hold_ms: u32) -> [SavedMacroAction; 2] {
    [
        SavedMacroAction {
            code,
            down: true,
            delay_ms: hold_ms,
        },
        SavedMacroAction {
            code,
            down: false,
            delay_ms: 0,
        },
    ]
}

/// Keys that are pressed and never released (each listed once).
pub fn unbalanced_keys(steps: &[SavedMacroAction]) -> Vec<u16> {
    let mut held: Vec<u16> = Vec::new();
    for s in steps {
        if s.down {
            if !held.contains(&s.code) {
                held.push(s.code);
            }
        } else {
            held.retain(|c| *c != s.code);
        }
    }
    held
}

/// "Tambahkan pelepasan otomatis": appends a release for every key that is
/// pressed and never released, the last pressed first. Adds nothing when
/// they do not all fit in the 255-action limit. Returns how many were added.
pub fn append_releases(steps: &mut Vec<SavedMacroAction>) -> usize {
    let held = unbalanced_keys(steps);
    if held.is_empty() || steps.len() + held.len() > MAX_ACTIONS {
        return 0;
    }
    if let Some(last) = steps.last_mut() {
        if last.delay_ms == 0 {
            last.delay_ms = DEFAULT_STEP_DELAY_MS;
        }
    }
    let n = held.len();
    for (i, code) in held.into_iter().rev().enumerate() {
        steps.push(SavedMacroAction {
            code,
            down: false,
            delay_ms: if i + 1 < n { DEFAULT_STEP_DELAY_MS } else { 0 },
        });
    }
    n
}

/// Why the macro cannot be sent yet (Indonesian), or `None` when it is fine.
/// A key that is pressed and never released would stay pressed on the
/// computer, so such a macro is not sent either.
pub fn macro_problem(m: &SavedMacro) -> Option<&'static str> {
    if m.actions.is_empty() {
        return Some(strings::EMPTY_MACRO);
    }
    if m.actions.len() > MAX_ACTIONS {
        return Some(strings::TOO_MANY);
    }
    if m.actions.iter().any(|a| a.delay_ms > MAX_U24) || m.interval_ms > MAX_U24 {
        return Some(strings::DELAY_TOO_LONG);
    }
    if !unbalanced_keys(&m.actions).is_empty() {
        return Some(strings::UNBALANCED_BLOCK);
    }
    None
}

/// "0.24" for 240 ms.
pub fn seconds_label(ms: u64) -> String {
    format!("{:.2}", ms as f64 / 1000.0)
}

/// Label of a key code: the keycode table label with the code, e.g.
/// "C (0x06)"; the short key-cap legend for matrix codes.
pub fn code_label(code: u16) -> String {
    match u8::try_from(code) {
        Ok(c) if keycodes::lookup(code).is_some() => keycodes::legend(c).to_string(),
        _ => {
            keycodes::lookup(code).map_or_else(|| format!("0x{code:04X}"), |k| k.label.to_string())
        }
    }
}

// --------------------------------------------------------------- library

/// "Macro baru", "Macro baru 2", ... that is not used yet.
pub fn unique_name(existing: &[SavedMacro], base: &str) -> String {
    let taken = |n: &str| existing.iter().any(|m| m.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// Appends a new empty macro; returns its index.
pub fn library_new(lib: &mut Vec<SavedMacro>) -> usize {
    let name = unique_name(lib, strings::NEW_NAME);
    lib.push(SavedMacro::new(name));
    lib.len() - 1
}

/// Inserts a copy of macro `i` right after it; returns the copy's index.
pub fn library_duplicate(lib: &mut Vec<SavedMacro>, i: usize) -> Option<usize> {
    let src = lib.get(i)?;
    let base = format!("{} ({})", src.name, strings::COPY_SUFFIX);
    let name = unique_name(lib, &base);
    let copy = src.duplicate_named(name);
    lib.insert(i + 1, copy);
    Some(i + 1)
}

/// Removes macro `i`; returns the index that should be selected.
pub fn library_delete(lib: &mut Vec<SavedMacro>, i: usize) -> Option<usize> {
    if i >= lib.len() {
        return None;
    }
    lib.remove(i);
    (!lib.is_empty()).then(|| i.min(lib.len() - 1))
}

// ------------------------------------------------------------------ jobs

/// Keys that cannot take a macro: the Fn key.
pub fn target_blocked(code: u8) -> bool {
    code == 0x01
}

/// The advanced function (DKS, MT, TGL, MPT, SOCD or END; not a macro) that
/// `key` holds in the snapshot. Binding a macro clears it first (and the
/// partner of a SOCD pair), so the page asks before it does.
pub fn replaced_function(snap: &DeviceSnapshot, key: u8) -> Option<Func> {
    snap.keys
        .get(&key)
        .and_then(|s| Func::of_nibble(s.mode.advanced))
}

/// "Tombol ini punya fungsi lanjutan SOCD. Memasang macro ...".
pub fn replace_warning(f: Func) -> String {
    let mut text = format!(
        "{} {}. {}",
        strings::REPLACES,
        f.label(),
        strings::REPLACES_TAIL
    );
    if f == Func::Socd {
        text = format!("{text} {}", strings::REPLACES_SOCD);
    }
    text
}

/// What the bind job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindOutcome {
    pub write: Result<(), Error>,
    /// The key's macro settings re-read afterwards (also after a failure).
    pub binding: Result<Option<MacroBinding>, Error>,
    /// The key's settings re-read afterwards, to refresh the shared snapshot
    /// (its Mode now says "macro").
    pub settings: Result<KeySettings, Error>,
}

/// The "Pasang ke tombol" job: `bind_macro` (which verifies the macro mode
/// by reading it back), then always re-reads the key's macro settings and
/// key settings. A disconnect is returned as `Err(Disconnected)`.
pub fn bind_job<T: Transport>(
    kb: &mut Keyboard<T>,
    key: u8,
    actions: &[MacroAction],
    mode: PlayMode,
    repeat: u16,
    interval_ms: u32,
) -> Result<BindOutcome, Error> {
    let write = kb.bind_macro(key, actions, mode, repeat, interval_ms);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let binding = kb.macro_binding(key);
    if binding == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let settings = kb.key_settings(key);
    if settings == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    Ok(BindOutcome {
        write,
        binding,
        settings,
    })
}

/// Reads the macro bound to `key` (when the target key changes).
pub fn binding_job<T: Transport>(
    kb: &mut Keyboard<T>,
    key: u8,
) -> Result<Option<MacroBinding>, Error> {
    kb.macro_binding(key)
}

/// What the page does with a finished bind job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindEffects {
    /// The binding as re-read; `Some(None)` is "no macro on the key".
    pub binding: Option<Option<MacroBinding>>,
    /// Fresh key settings for the shared snapshot.
    pub patch: Option<KeySettings>,
    pub request_reload: bool,
    pub error: Option<String>,
    pub technical: Option<String>,
    pub bound: bool,
}

pub fn bind_effects(result: Result<BindOutcome, Error>) -> BindEffects {
    match result {
        Ok(out) => {
            let reread = out.binding.is_ok();
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => {
                    let mut msg = i18n::device_error(e);
                    if reread {
                        msg = format!("{msg} {}", strings::RELOADED_HINT);
                    }
                    (Some(msg), Some(i18n::technical(e)))
                }
            };
            BindEffects {
                bound: out.write.is_ok(),
                request_reload: out.binding.is_err() && out.settings.is_err(),
                binding: out.binding.ok(),
                patch: out.settings.ok(),
                error,
                technical,
            }
        }
        Err(Error::Disconnected) => BindEffects {
            binding: None,
            patch: None,
            request_reload: false,
            error: Some(strings::PARTIAL_WRITE.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            bound: false,
        },
        Err(e) => BindEffects {
            binding: None,
            patch: None,
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            bound: false,
        },
    }
}

fn mode_title(m: PlayMode) -> &'static str {
    match m {
        PlayMode::Once => strings::MODE_ONCE,
        PlayMode::Toggle => strings::MODE_TOGGLE,
        PlayMode::WhileHeld => strings::MODE_HELD,
        PlayMode::WhileHeldFinish => strings::MODE_HELD_FINISH,
    }
}

fn mode_desc(m: PlayMode) -> &'static str {
    match m {
        PlayMode::Once => strings::MODE_ONCE_DESC,
        PlayMode::Toggle => strings::MODE_TOGGLE_DESC,
        PlayMode::WhileHeld => strings::MODE_HELD_DESC,
        PlayMode::WhileHeldFinish => strings::MODE_HELD_FINISH_DESC,
    }
}

/// "6 langkah · Sekali per tekan · 1× · 100 ms". The step count is left
/// out when the keyboard reports len 0, which FW App V1.1.3 always does
/// (PROTOCOL 7.x finding 24).
pub fn binding_text(b: &MacroBinding) -> String {
    let rest = format!(
        "{} · {}× · {} {}",
        mode_title(b.mode),
        b.repeat,
        b.interval_ms,
        strings::MS
    );
    if b.len == 0 {
        rest
    } else {
        format!("{} {} · {rest}", b.len, strings::STEPS)
    }
}

// ------------------------------------------------------------------ page

/// An inline error: Indonesian text plus the raw detail.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    title: &'static str,
    text: String,
    technical: Option<String>,
}

pub struct Page {
    selected: Option<usize>,
    step_sel: Option<usize>,
    confirm_delete: bool,
    adding: bool,
    add_code: u16,
    recorder: Option<Recorder>,
    /// Frame time of the last `ui` call, to notice a hidden page.
    last_ui_time: f64,
    notice: Option<String>,
    save_error: Option<String>,

    picking: bool,
    pick_sel: Selection,
    target: Option<u8>,
    blocked_note: bool,
    binding: Option<Result<Option<MacroBinding>, Failure>>,
    binding_for: Option<(u8, u64)>,
    binding_pending: Option<Pending<Option<MacroBinding>>>,
    bind_pending: Option<Pending<BindOutcome>>,
    /// The key the running bind goes to (its result belongs to it).
    bind_key: Option<u8>,
    /// The last bind sent, for "Coba lagi"; forgotten when the selected
    /// macro changes or is edited.
    last_bind: Option<(u8, SavedMacro)>,
    failure: Option<Failure>,
    bound: bool,
    /// The user saw that the target's function will be replaced and
    /// clicked once; the next click binds.
    confirm_replace: bool,
    /// The running bind replaces a function: reload afterwards (a SOCD
    /// partner changed too).
    bind_replaces: bool,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            selected: None,
            step_sel: None,
            confirm_delete: false,
            adding: false,
            add_code: 0x04,
            recorder: None,
            last_ui_time: 0.0,
            notice: None,
            save_error: None,
            picking: false,
            pick_sel: Selection::default(),
            target: None,
            blocked_note: false,
            binding: None,
            binding_for: None,
            binding_pending: None,
            bind_pending: None,
            bind_key: None,
            last_bind: None,
            failure: None,
            bound: false,
            confirm_replace: false,
            bind_replaces: false,
        }
    }
}

impl Page {
    /// A page showing macro `selected` with `target` as the chosen key and
    /// `binding` already read for worker generation `generation`. For
    /// screenshots and tests that run without a keyboard.
    #[doc(hidden)]
    pub fn preview(
        selected: usize,
        target: Option<u8>,
        generation: u64,
        binding: Option<MacroBinding>,
    ) -> Page {
        Page {
            selected: Some(selected),
            target,
            binding_for: target.map(|t| (t, generation)),
            binding: target.map(|_| Ok(binding)),
            ..Page::default()
        }
    }

    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.sync_selection(ctx);
        self.forget_stale_bind(ctx);
        self.record_frame(ctx, ui);
        self.poll(ctx);
        self.start_reads(ctx);
        page_with_right_panel(
            ui,
            "macros-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
        self.picker_window(ctx, ui);
    }

    fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    fn job_pending(&self) -> bool {
        self.binding_pending.is_some() || self.bind_pending.is_some()
    }

    fn sync_selection(&mut self, ctx: &AppCtx) {
        let n = ctx.data.borrow().macros.len();
        self.selected = match self.selected {
            _ if n == 0 => None,
            Some(i) if i < n => Some(i),
            _ => Some(0),
        };
    }

    /// The last bind's result and "Coba lagi" are about one macro: once
    /// another macro is selected, or this one is edited, they go.
    fn forget_stale_bind(&mut self, ctx: &AppCtx) {
        let Some((_, sent)) = &self.last_bind else {
            return;
        };
        if self.current(ctx).as_ref() != Some(sent) {
            self.last_bind = None;
            self.bound = false;
            self.failure = None;
        }
    }

    fn current(&self, ctx: &AppCtx) -> Option<SavedMacro> {
        let i = self.selected?;
        ctx.data.borrow().macros.get(i).cloned()
    }

    /// Replaces the selected macro and saves the library.
    fn store(&mut self, ctx: &AppCtx, new: SavedMacro) {
        if let Some(i) = self.selected {
            if let Some(slot) = ctx.data.borrow_mut().macros.get_mut(i) {
                *slot = new;
            }
            self.persist(ctx);
        }
    }

    fn persist(&mut self, ctx: &AppCtx) {
        self.save_error = ctx.save_data().err().map(|e| e.to_string());
    }

    // ------------------------------------------------------------ recording

    /// Starts recording onto a macro that already has `existing` steps: the
    /// recording stops before the 255-action limit, releases included.
    fn start_recording(&mut self, time: f64, existing: usize) {
        self.recorder = Some(Recorder::with_room(MAX_ACTIONS.saturating_sub(existing)));
        self.last_ui_time = time;
        self.notice = None;
        self.step_sel = None;
        self.confirm_delete = false;
        self.adding = false;
    }

    fn stop_recording(&mut self, ctx: &AppCtx, time: f64, interrupted: bool) {
        let Some(rec) = self.recorder.take() else {
            return;
        };
        let full = rec.is_full();
        let result = rec.finish(time);
        let mut notes = Vec::new();
        if !result.skipped.is_empty() {
            notes.push(format!(
                "{} {}.",
                strings::SKIPPED,
                result.skipped.join(", ")
            ));
        }
        if full {
            notes.push(strings::RECORD_FULL.to_string());
        }
        if interrupted {
            notes.push(strings::RECORD_INTERRUPTED.to_string());
        }
        self.notice = (!notes.is_empty()).then(|| notes.join(" "));
        if let Some(mut m) = self.current(ctx) {
            let new: Vec<SavedMacroAction> = result.actions.into_iter().map(Into::into).collect();
            step_insert(&mut m.actions, None, &new);
            self.store(ctx, m);
        }
    }

    /// Feeds this frame's key events to the recorder, and keeps keyboard
    /// focus on an inert widget so Tab, Space and the arrows are recorded
    /// instead of driving the UI.
    fn record_frame(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let (time, events, modifiers) = ui.input(|i| (i.time, i.events.clone(), i.modifiers));
        if self.recording() && time - self.last_ui_time > 1.0 {
            // The page was not drawn for a while (another page was open).
            self.stop_recording(ctx, self.last_ui_time, true);
        }
        self.last_ui_time = time;
        let Some(rec) = self.recorder.as_mut() else {
            return;
        };
        for e in &events {
            if let Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
                ..
            } = e
            {
                rec.key(*key, *physical_key, *pressed, *repeat, modifiers, time);
            }
            let clip = match e {
                Event::Copy => Some(Clip::Copy),
                Event::Cut => Some(Clip::Cut),
                Event::Paste(_) => Some(Clip::Paste),
                _ => None,
            };
            if let Some(clip) = clip {
                rec.clipboard(clip, &modifiers, time);
            }
        }
        rec.modifiers(mods_of(&modifiers), time);
        let full = rec.is_full();

        let id = egui::Id::new("macro-recorder-surface");
        let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(1.0, 1.0));
        ui.interact(rect, id, Sense::focusable_noninteractive());
        ui.memory_mut(|m| {
            m.request_focus(id);
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        ui.ctx().request_repaint();
        if full {
            self.stop_recording(ctx, time, false);
        }
    }

    // ----------------------------------------------------------------- jobs

    fn poll(&mut self, ctx: &AppCtx) {
        if let Some(p) = self.binding_pending.as_mut() {
            if let Some(r) = p.poll() {
                self.binding_pending = None;
                self.binding = match r {
                    Ok(b) => Some(Ok(b)),
                    Err(Error::Disconnected) => {
                        self.binding_for = None;
                        None
                    }
                    Err(e) => Some(Err(Failure {
                        title: strings::BINDING_FAILED,
                        text: i18n::device_error(&e),
                        technical: Some(i18n::technical(&e)),
                    })),
                };
            }
        }
        if let Some(p) = self.bind_pending.as_mut() {
            if let Some(r) = p.poll() {
                self.bind_pending = None;
                let fx = bind_effects(r);
                // The result is the bound key's, whatever is selected now.
                let key = self.bind_key.take();
                if let (Some(key), Some(s)) = (key, fx.patch) {
                    ctx.worker.patch_snapshot(move |snap| {
                        snap.keys.insert(key, s);
                    });
                }
                match fx.binding {
                    Some(b) if key == self.target => self.binding = Some(Ok(b)),
                    _ => self.binding_for = None, // read again
                }
                // Binding over a function also changed its SOCD partner (and
                // the job re-reads only the target): read everything again.
                if fx.request_reload || std::mem::take(&mut self.bind_replaces) {
                    ctx.worker.request_reload();
                }
                self.failure = fx.error.map(|text| Failure {
                    title: strings::BIND_FAILED,
                    text,
                    technical: fx.technical,
                });
                // "Terpasang" only while the bound macro is still the one
                // shown.
                self.bound = fx.bound && self.last_bind.is_some();
            }
        }
    }

    /// Reads the macro on the target key once per key and device generation.
    fn start_reads(&mut self, ctx: &AppCtx) {
        let Some(key) = self.target else {
            return;
        };
        if !ctx.can_write() || self.job_pending() {
            return;
        }
        let generation = ctx.worker.generation();
        if self.binding_for == Some((key, generation)) {
            return;
        }
        self.binding_for = Some((key, generation));
        self.binding = None;
        self.binding_pending = Some(
            ctx.worker
                .submit("macro-binding", move |kb| binding_job(kb, key)),
        );
    }

    /// "Pasang ke tombol" was clicked; true when the bind should go out. A
    /// target that holds another function needs a second, confirming click.
    fn bind_clicked(&mut self, replaces: Option<Func>) -> bool {
        if replaces.is_some() && !self.confirm_replace {
            self.confirm_replace = true;
            return false;
        }
        self.confirm_replace = false;
        true
    }

    fn submit_bind(&mut self, ctx: &AppCtx, key: u8, m: SavedMacro) {
        self.failure = None;
        self.bound = false;
        self.confirm_replace = false;
        self.bind_replaces = ctx
            .snapshot
            .is_some_and(|s| replaced_function(s, key).is_some());
        self.bind_key = Some(key);
        self.last_bind = Some((key, m.clone()));
        let actions = m.to_actions();
        let (mode, repeat, interval) = (m.play_mode(), m.repeat_count(), m.interval_ms);
        self.bind_pending = Some(ctx.worker.submit("macro-bind", move |kb| {
            bind_job(kb, key, &actions, mode, repeat, interval)
        }));
    }

    // --------------------------------------------------------------- centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let h = ui.available_height();
        ui.horizontal_top(|ui| {
            let list_w = 280.0;
            ui.allocate_ui_with_layout(egui::vec2(list_w, h), Layout::top_down(Align::Min), |ui| {
                self.library_card(ctx, ui, h)
            });
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), h),
                Layout::top_down(Align::Min),
                |ui| self.editor_card(ctx, ui, h),
            );
        });
    }

    fn library_card(&mut self, ctx: &AppCtx, ui: &mut Ui, h: f32) {
        Card::new().padding(16).show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 34.0));
            ui.set_max_width(ui.available_width());
            ui.horizontal(|ui| {
                card::title(ui, strings::LIBRARY);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let new = egui::Button::new(
                        RichText::new(strings::NEW).color(color::ON_ACCENT).strong(),
                    )
                    .fill(color::TEXT)
                    .corner_radius(CornerRadius::same(size::CONTROL_RADIUS));
                    if ui.add_enabled(!self.recording(), new).clicked() {
                        let idx = library_new(&mut ctx.data.borrow_mut().macros);
                        self.selected = Some(idx);
                        self.confirm_delete = false;
                        self.persist(ctx);
                    }
                });
            });
            ui.add_space(8.0);
            let items: Vec<(String, usize, u64)> = ctx
                .data
                .borrow()
                .macros
                .iter()
                .map(|m| (m.name.clone(), m.actions.len(), m.total_ms()))
                .collect();
            if items.is_empty() {
                card::hint(ui, strings::LIBRARY_EMPTY);
            }
            let list_h = (ui.available_height() - 64.0).max(80.0);
            egui::ScrollArea::vertical()
                .max_height(list_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, (name, n, ms)) in items.iter().enumerate() {
                        let sub = format!(
                            "{n} {} · {} {}",
                            strings::STEPS,
                            seconds_label(*ms),
                            strings::SECONDS
                        );
                        let selected = self.selected == Some(i);
                        if library_row(ui, name, &sub, selected, !self.recording()).clicked() {
                            self.selected = Some(i);
                            self.step_sel = None;
                            self.confirm_delete = false;
                            self.notice = None;
                        }
                    }
                });
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                card::hint(ui, strings::LIBRARY_NOTE);
            });
        });
    }

    fn editor_card(&mut self, ctx: &AppCtx, ui: &mut Ui, h: f32) {
        Card::new().padding(0).show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 2.0));
            ui.set_max_width(ui.available_width());
            let Some(mut m) = self.current(ctx) else {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    card::hint(ui, strings::NO_MACRO_SELECTED);
                });
                return;
            };
            let original = m.clone();
            Frame::new()
                .inner_margin(Margin::symmetric(20, 16))
                .show(ui, |ui| self.editor_header(ctx, ui, &mut m));
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter()
                .hline(rect.x_range(), rect.center().y, line(1.0, color::BORDER));
            Frame::new()
                .inner_margin(Margin::symmetric(20, 10))
                .show(ui, |ui| self.step_table(ui, &mut m));
            if m != original {
                self.store(ctx, m);
            }
        });
    }

    fn editor_header(&mut self, ctx: &AppCtx, ui: &mut Ui, m: &mut SavedMacro) {
        let recording = self.recording();
        ui.horizontal(|ui| {
            ui.add_enabled(
                !recording,
                egui::TextEdit::singleline(&mut m.name)
                    .hint_text(strings::NAME_HINT)
                    .font(font::heading())
                    .desired_width(ui.available_width() - 270.0),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, fill, stroke) = if recording {
                    (strings::STOP, color::DANGER, color::DANGER)
                } else {
                    (strings::RECORD, color::DANGER_SOFT, color::DANGER)
                };
                let text_color = if recording {
                    color::ON_ACCENT
                } else {
                    color::DANGER
                };
                let rec = egui::Button::new(
                    RichText::new(format!("● {label}"))
                        .color(text_color)
                        .strong(),
                )
                .fill(fill)
                .stroke(line(1.0, stroke))
                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
                .min_size(egui::vec2(0.0, 36.0));
                if ui.add(rec).clicked() {
                    let time = ui.input(|i| i.time);
                    if recording {
                        self.stop_recording(ctx, time, false);
                        // `m` was read before the recording was stored.
                        if let Some(fresh) = self.current(ctx) {
                            *m = fresh;
                        }
                    } else {
                        self.start_recording(time, m.actions.len());
                    }
                }
                let add = egui::Button::new(strings::ADD_STEP)
                    .fill(color::BG)
                    .stroke(line(1.0, color::BORDER_STRONG))
                    .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
                    .min_size(egui::vec2(0.0, 36.0));
                if ui.add_enabled(!recording, add).clicked() {
                    self.adding = !self.adding;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {} {} · {} {}",
                    strings::TOTAL,
                    seconds_label(m.total_ms()),
                    strings::SECONDS,
                    m.actions.len(),
                    strings::STEPS
                ))
                .font(font::small())
                .color(color::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let idle = !recording;
                if text_button(
                    ui,
                    self.confirm_delete,
                    strings::DELETE,
                    strings::DELETE_CONFIRM,
                    idle,
                ) {
                    if self.confirm_delete {
                        let mut lib = ctx.data.borrow_mut();
                        if let Some(i) = self.selected {
                            self.selected = library_delete(&mut lib.macros, i);
                        }
                        drop(lib);
                        self.confirm_delete = false;
                        self.persist(ctx);
                    } else {
                        self.confirm_delete = true;
                    }
                }
                if text_button(ui, false, strings::DUPLICATE, "", idle) {
                    if let Some(i) = self.selected {
                        let new = library_duplicate(&mut ctx.data.borrow_mut().macros, i);
                        self.selected = new.or(self.selected);
                        self.confirm_delete = false;
                        self.persist(ctx);
                    }
                }
                if text_button(ui, false, strings::CLEAR, "", idle && !m.actions.is_empty()) {
                    m.actions.clear();
                    self.step_sel = None;
                }
            });
        });
        if recording {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(format!(
                        "{} ({} {})",
                        strings::RECORDING,
                        self.recorder.as_ref().map_or(0, Recorder::len),
                        strings::STEPS
                    ))
                    .font(font::small())
                    .color(color::DANGER),
                );
            });
            card::hint(ui, strings::RECORD_APPENDS);
        }
        if let Some(n) = &self.notice {
            ui.add_space(6.0);
            card::hint(ui, n.clone());
        }
        if let Some(e) = &self.save_error {
            ui.add_space(6.0);
            card::error_box(ui, strings::SAVE_FAILED, e, None, None);
        }
        if self.adding && !recording {
            ui.add_space(8.0);
            self.add_step_row(ui, m);
        }
        let unbalanced = unbalanced_keys(&m.actions);
        if !unbalanced.is_empty() && !recording {
            ui.add_space(6.0);
            let names: Vec<String> = unbalanced.iter().map(|c| code_label(*c)).collect();
            card::hint(ui, format!("{} {}.", strings::UNBALANCED, names.join(", ")));
            let fits = m.actions.len() + unbalanced.len() <= MAX_ACTIONS;
            if ui
                .add_enabled(fits, egui::Button::new(strings::ADD_RELEASES))
                .clicked()
            {
                append_releases(&mut m.actions);
            }
        }
        if let Some(p) = macro_problem(m) {
            if !m.actions.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(p).font(font::small()).color(color::DANGER));
            }
        }
    }

    fn add_step_row(&mut self, ui: &mut Ui, m: &mut SavedMacro) {
        Frame::new()
            .fill(color::BG)
            .stroke(line(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(strings::ADD_KEY).color(color::MUTED));
                    egui::ComboBox::from_id_salt("macro-add-key")
                        .width(150.0)
                        .selected_text(format!(
                            "{} (0x{:02X})",
                            code_label(self.add_code),
                            self.add_code
                        ))
                        .show_ui(ui, |ui| {
                            for k in keycodes::all().iter().filter(|k| addable(k)) {
                                ui.selectable_value(
                                    &mut self.add_code,
                                    k.code,
                                    format!("{} (0x{:02X})", k.label, k.code),
                                );
                            }
                        });
                    let code = self.add_code;
                    let mut new: Vec<SavedMacroAction> = Vec::new();
                    if ui.button(strings::ADD_PRESS).clicked() {
                        new.push(SavedMacroAction {
                            code,
                            down: true,
                            delay_ms: DEFAULT_STEP_DELAY_MS,
                        });
                    }
                    if ui.button(strings::ADD_RELEASE).clicked() {
                        new.push(SavedMacroAction {
                            code,
                            down: false,
                            delay_ms: DEFAULT_STEP_DELAY_MS,
                        });
                    }
                    if ui.button(strings::ADD_TAP).clicked() {
                        new.extend(tap(code, DEFAULT_STEP_DELAY_MS));
                    }
                    if !new.is_empty() {
                        let last = step_insert(&mut m.actions, self.step_sel, &new);
                        self.step_sel = last;
                    }
                });
                let hint = match self.step_sel {
                    Some(i) => format!("{} {}.", strings::ADD_WHERE_AFTER, i + 1),
                    None => strings::ADD_WHERE_END.to_string(),
                };
                card::hint(ui, hint);
            });
    }

    fn step_table(&mut self, ui: &mut Ui, m: &mut SavedMacro) {
        let recording = self.recording();
        if m.actions.is_empty() && !recording {
            ui.add_space(8.0);
            card::hint(ui, strings::NO_STEPS);
            return;
        }
        let editable = !recording;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("macro-steps")
                    .num_columns(5)
                    .spacing([14.0, 8.0])
                    .min_col_width(24.0)
                    .show(ui, |ui| {
                        for h in [
                            strings::COL_STEP,
                            strings::COL_ACTION,
                            strings::COL_KEY,
                            strings::COL_DELAY,
                            "",
                        ] {
                            ui.label(RichText::new(h).font(font::tiny()).color(color::MUTED));
                        }
                        ui.end_row();
                        let n = m.actions.len();
                        let mut op: Option<StepOp> = None;
                        for i in 0..n {
                            let selected = self.step_sel == Some(i);
                            let num = RichText::new((i + 1).to_string()).color(if selected {
                                color::ACCENT
                            } else {
                                color::MUTED
                            });
                            if ui
                                .add_enabled(editable, egui::Label::new(num).sense(Sense::click()))
                                .clicked()
                            {
                                self.step_sel = if selected { None } else { Some(i) };
                            }
                            let down = m.actions[i].down;
                            let (text, c) = if down {
                                (format!("↓ {}", strings::PRESS), color::SUCCESS)
                            } else {
                                (format!("↑ {}", strings::RELEASE), RELEASE_COLOR)
                            };
                            let b = egui::Button::new(RichText::new(text).color(c)).frame(false);
                            if ui.add_enabled(editable, b).clicked() {
                                op = Some(StepOp::Toggle(i));
                            }
                            key_chip(ui, &code_label(m.actions[i].code));
                            delay_box(ui, &mut m.actions[i].delay_ms, editable);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let small = |t: &str| {
                                    egui::Button::new(RichText::new(t).color(color::MUTED))
                                        .frame(false)
                                };
                                if ui
                                    .add_enabled(editable && i > 0, small("↑"))
                                    .on_hover_text(strings::UP)
                                    .clicked()
                                {
                                    op = Some(StepOp::Up(i));
                                }
                                if ui
                                    .add_enabled(editable && i + 1 < n, small("↓"))
                                    .on_hover_text(strings::DOWN)
                                    .clicked()
                                {
                                    op = Some(StepOp::Down(i));
                                }
                                if ui
                                    .add_enabled(editable, small("×"))
                                    .on_hover_text(strings::REMOVE)
                                    .clicked()
                                {
                                    op = Some(StepOp::Delete(i));
                                }
                            });
                            ui.end_row();
                        }
                        if let Some(op) = op {
                            match op {
                                StepOp::Toggle(i) => step_toggle(&mut m.actions, i),
                                StepOp::Up(i) => {
                                    let to = step_move_up(&mut m.actions, i);
                                    self.step_sel = Some(to);
                                }
                                StepOp::Down(i) => {
                                    let to = step_move_down(&mut m.actions, i);
                                    self.step_sel = Some(to);
                                }
                                StepOp::Delete(i) => {
                                    self.step_sel = step_delete(&mut m.actions, i);
                                }
                            }
                        }
                    });
                if recording {
                    ui.add_space(8.0);
                    card::hint(ui, strings::RECORD_APPENDS);
                }
                card::hint(ui, strings::DELAY_HINT);
            });
    }

    // ---------------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("macros-apply")
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
                    .show(ui, |ui| self.settings(ctx, ui));
            });
    }

    fn settings(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        ui.label(
            RichText::new(strings::PLAY_TITLE)
                .font(font::heading())
                .color(color::TEXT)
                .strong(),
        );
        card::hint(ui, strings::PLAY_DESC);
        ui.add_space(10.0);

        let editable = !self.recording();
        if let Some(mut m) = self.current(ctx) {
            let original = m.clone();
            for mode in [
                PlayMode::Once,
                PlayMode::Toggle,
                PlayMode::WhileHeld,
                PlayMode::WhileHeldFinish,
            ] {
                let selected = m.play_mode() == mode;
                if mode_card(ui, mode_title(mode), mode_desc(mode), selected, editable).clicked() {
                    m.mode = mode as u8;
                }
                ui.add_space(2.0);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(strings::REPEAT).color(color::TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut r = m.repeat_count();
                    if ui
                        .add_enabled(
                            editable,
                            egui::DragValue::new(&mut r).range(1..=u16::MAX).speed(0.2),
                        )
                        .changed()
                    {
                        m.repeat = r;
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(strings::INTERVAL).color(color::TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled(
                        editable,
                        egui::DragValue::new(&mut m.interval_ms)
                            .range(0..=MAX_U24)
                            .speed(1.0)
                            .suffix(format!(" {}", strings::MS)),
                    );
                });
            });
            card::hint(ui, strings::REPEAT_HINT);
            if m != original {
                self.store(ctx, m);
            }
        }
        ui.add_space(14.0);

        card::section(ui, strings::TARGET);
        self.target_row(ctx, ui);
        ui.add_space(6.0);
        self.binding_row(ctx, ui);
        ui.add_space(12.0);
        card::hint(ui, strings::WARN_REPLACE);
        card::hint(ui, strings::WARN_DELAY);
    }

    fn target_row(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        Frame::new()
            .fill(color::PANEL)
            .stroke(line(1.0, color::BORDER_STRONG))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    match self.target {
                        Some(k) => {
                            ui.label(
                                RichText::new(code_label(u16::from(k)))
                                    .color(color::TEXT)
                                    .strong(),
                            );
                            ui.label(RichText::new(strings::TARGET_LAYER).color(color::MUTED));
                        }
                        None => {
                            ui.label(RichText::new(strings::TARGET_NONE).color(color::MUTED));
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if self.target.is_some() {
                            strings::CHANGE
                        } else {
                            strings::PICK
                        };
                        let b = egui::Button::new(RichText::new(label).color(color::ACCENT))
                            .frame(false);
                        if ui
                            .add_enabled(
                                ctx.connected() && !self.recording() && self.bind_pending.is_none(),
                                b,
                            )
                            .clicked()
                        {
                            self.picking = true;
                            self.pick_sel.clear();
                        }
                    });
                });
            });
        if self.blocked_note {
            ui.label(
                RichText::new(strings::FN_BLOCKED)
                    .font(font::small())
                    .color(color::DANGER),
            );
        }
    }

    fn binding_row(&self, ctx: &AppCtx, ui: &mut Ui) {
        let Some(key) = self.target else {
            return;
        };
        if let Some(f) = ctx.snapshot.and_then(|s| replaced_function(s, key)) {
            ui.label(
                RichText::new(replace_warning(f))
                    .font(font::small())
                    .color(color::ACCENT),
            );
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{}:", strings::CURRENT))
                    .font(font::small())
                    .color(color::MUTED),
            );
            let (text, c) = match &self.binding {
                None => (strings::BINDING_LOADING.to_string(), color::MUTED),
                Some(Ok(None)) => (strings::NO_BINDING.to_string(), color::MUTED),
                Some(Ok(Some(b))) => (binding_text(b), color::TEXT),
                Some(Err(f)) => (format!("{}. {}", f.title, f.text), color::DANGER),
            };
            ui.label(RichText::new(text).font(font::small()).color(c));
        });
    }

    fn apply_area(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let idle = ctx.can_write() && !self.job_pending() && !self.recording();
        if let Some(f) = self.failure.clone() {
            let retry = (idle && self.last_bind.is_some()).then_some(i18n::common::RETRY);
            if card::error_box(ui, f.title, &f.text, retry, f.technical.as_deref()) {
                if let Some((k, m)) = self.last_bind.clone() {
                    self.submit_bind(ctx, k, m);
                }
            }
            ui.add_space(8.0);
        }
        let current = self.current(ctx);
        let problem = current.as_ref().and_then(macro_problem);
        let can = idle && self.target.is_some() && current.is_some() && problem.is_none();
        let replaces = match (ctx.snapshot, self.target) {
            (Some(s), Some(k)) => replaced_function(s, k),
            _ => None,
        };
        if replaces.is_none() {
            self.confirm_replace = false;
        }
        let label = if self.confirm_replace {
            strings::CONFIRM_REPLACE
        } else {
            strings::BIND
        };
        if card::primary_button(ui, label, can).clicked() && self.bind_clicked(replaces) {
            if let (Some(k), Some(m)) = (self.target, current) {
                self.submit_bind(ctx, k, m);
            }
        }
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if self.bind_pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(strings::BINDING)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            } else if self.bound {
                ui.label(
                    RichText::new(strings::BOUND)
                        .font(font::small())
                        .color(color::SUCCESS),
                );
            } else {
                let text = if !ctx.connected() {
                    strings::OFFLINE
                } else if let Some(p) = problem {
                    p
                } else {
                    strings::READY
                };
                ui.label(RichText::new(text).font(font::small()).color(color::MUTED));
            }
        });
    }

    // --------------------------------------------------------- key picker

    fn picker_window(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        if !self.picking {
            return;
        }
        let Some(snap) = ctx.snapshot else {
            self.picking = false;
            return;
        };
        let mut open = true;
        let mut picked: Option<u8> = None;
        egui::Window::new(strings::PICK_TITLE)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .frame(
                Frame::window(&ui.ctx().style())
                    .fill(color::PANEL)
                    .inner_margin(Margin::same(20)),
            )
            .show(ui.ctx(), |ui| {
                ui.set_width(820.0);
                card::hint(ui, strings::PICK_HINT);
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    KeyboardView::new(snap.model, &snap.matrix)
                        .id_salt("macro-target")
                        .show(ui, &mut self.pick_sel, |_| Default::default());
                });
                picked = self.pick_sel.codes_in(&snap.matrix).first().copied();
                ui.add_space(8.0);
                if ui.button(strings::PICK_CLOSE).clicked() {
                    self.picking = false;
                }
            });
        if !open {
            self.picking = false;
        }
        if self.bind_pending.is_some() {
            picked = None; // the target stays put until the bind is done
        }
        if let Some(k) = picked {
            self.picking = false;
            self.pick_sel.clear();
            if target_blocked(k) {
                self.blocked_note = true;
            } else {
                self.blocked_note = false;
                self.target = Some(k);
                self.binding_for = None;
                self.failure = None;
                self.bound = false;
                self.confirm_replace = false;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum StepOp {
    Toggle(usize),
    Up(usize),
    Down(usize),
    Delete(usize),
}

/// Keys offered by "Tambah langkah": the HID keyboard page minus media,
/// mouse and vendor codes.
fn addable(k: &keycodes::KeyCode) -> bool {
    k.code >= 0x04
        && k.code <= 0xE7
        && matches!(
            k.category,
            Category::Basic
                | Category::Navigation
                | Category::Function
                | Category::Numpad
                | Category::Modifier
        )
}

// --------------------------------------------------------------- widgets

fn text_button(ui: &mut Ui, armed: bool, text: &str, armed_text: &str, enabled: bool) -> bool {
    let (label, c) = if armed {
        (armed_text, color::DANGER)
    } else {
        (text, color::MUTED)
    };
    let b = egui::Button::new(RichText::new(label).font(font::small()).color(c)).frame(false);
    ui.add_enabled(enabled, b).clicked()
}

fn key_chip(ui: &mut Ui, label: &str) {
    Frame::new()
        .fill(color::SEGMENT_ON)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).color(color::TEXT).strong());
        });
}

fn delay_box(ui: &mut Ui, ms: &mut u32, enabled: bool) {
    ui.add_enabled(
        enabled,
        egui::DragValue::new(ms)
            .range(0..=MAX_U24)
            .speed(1.0)
            .suffix(format!(" {}", strings::MS)),
    );
}

/// One entry of the macro list.
fn library_row(
    ui: &mut Ui,
    name: &str,
    sub: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), sense);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, name)
    });
    let (fill, stroke, title) = if selected {
        (color::ACCENT_SOFT, color::ACCENT, color::ACCENT)
    } else if resp.hovered() && enabled {
        (color::KEY_HOVER, color::BORDER, color::TEXT)
    } else {
        (color::PANEL, color::PANEL, color::TEXT)
    };
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(size::CONTROL_RADIUS + 2),
        fill,
        line(1.0, stroke),
        StrokeKind::Inside,
    );
    let name_pos = rect.left_top() + egui::vec2(14.0, 10.0);
    p.text(
        name_pos,
        Align2::LEFT_TOP,
        elide(name, 26),
        font::bold(14.0),
        title,
    );
    p.text(
        rect.left_bottom() + egui::vec2(14.0, -10.0),
        Align2::LEFT_BOTTOM,
        sub,
        font::small(),
        color::MUTED,
    );
    resp
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max - 1).collect();
        format!("{head}…")
    }
}

/// A radio-style card for a play mode.
fn mode_card(
    ui: &mut Ui,
    title: &str,
    desc: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), sense);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, title)
    });
    let hovered = resp.hovered() && enabled;
    let (fill, stroke) = if selected {
        (color::ACCENT_SOFT, color::ACCENT)
    } else if hovered {
        (color::KEY_HOVER, color::BORDER_STRONG)
    } else {
        (color::PANEL, color::BORDER_STRONG)
    };
    let dim = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.55) };
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(size::CONTROL_RADIUS + 2),
        dim(fill),
        line(1.0, dim(stroke)),
        StrokeKind::Inside,
    );
    let c = egui::pos2(rect.left() + 20.0, rect.center().y);
    p.circle_stroke(
        c,
        8.0,
        line(
            1.5,
            dim(if selected {
                color::ACCENT
            } else {
                color::FAINT
            }),
        ),
    );
    if selected {
        p.circle_filled(c, 4.5, dim(color::ACCENT));
    }
    p.text(
        egui::pos2(rect.left() + 40.0, rect.top() + 11.0),
        Align2::LEFT_TOP,
        title,
        font::bold(14.0),
        dim(color::TEXT),
    );
    p.text(
        egui::pos2(rect.left() + 40.0, rect.bottom() - 10.0),
        Align2::LEFT_BOTTOM,
        desc,
        font::small(),
        dim(color::MUTED),
    );
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
    use openhe_proto::macros;
    use openhe_proto::model::Model;
    use std::collections::BTreeSet;

    fn act(code: u16, down: bool, delay_ms: u32) -> SavedMacroAction {
        SavedMacroAction {
            code,
            down,
            delay_ms,
        }
    }

    fn mac(code: u16, down: bool, delay_ms: u32) -> MacroAction {
        MacroAction {
            code,
            down,
            delay_ms,
        }
    }

    const NONE: Modifiers = Modifiers::NONE;

    fn ctrl() -> Modifiers {
        Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        }
    }

    // ---- key mapping

    #[test]
    fn egui_keys_map_to_hid_codes() {
        assert_eq!(hid_code(Key::A), Some(0x04));
        assert_eq!(hid_code(Key::Z), Some(0x1D));
        assert_eq!(hid_code(Key::Num1), Some(0x1E));
        assert_eq!(hid_code(Key::Num0), Some(0x27));
        assert_eq!(hid_code(Key::Enter), Some(0x28));
        assert_eq!(hid_code(Key::Escape), Some(0x29));
        assert_eq!(hid_code(Key::Space), Some(0x2C));
        assert_eq!(hid_code(Key::F1), Some(0x3A));
        assert_eq!(hid_code(Key::F12), Some(0x45));
        assert_eq!(hid_code(Key::F13), Some(0x68));
        assert_eq!(hid_code(Key::ArrowUp), Some(0x52));
        assert_eq!(hid_code(Key::Plus), hid_code(Key::Equals));
        assert_eq!(hid_code(Key::Colon), hid_code(Key::Semicolon));
        assert_eq!(hid_code(Key::Copy), None);
        assert_eq!(hid_code(Key::F30), None);
    }

    #[test]
    fn every_mapped_code_is_in_the_keycode_table() {
        use Key::*;
        let all = [
            A,
            B,
            C,
            D,
            E,
            F,
            G,
            H,
            I,
            J,
            K,
            L,
            M,
            N,
            O,
            P,
            Q,
            R,
            S,
            T,
            U,
            V,
            W,
            X,
            Y,
            Z,
            Num0,
            Num1,
            Num2,
            Num3,
            Num4,
            Num5,
            Num6,
            Num7,
            Num8,
            Num9,
            Enter,
            Escape,
            Backspace,
            Tab,
            Space,
            Minus,
            Equals,
            Plus,
            OpenBracket,
            CloseBracket,
            OpenCurlyBracket,
            CloseCurlyBracket,
            Backslash,
            Pipe,
            Semicolon,
            Colon,
            Quote,
            Backtick,
            Comma,
            Period,
            Slash,
            Questionmark,
            F1,
            F2,
            F3,
            F4,
            F5,
            F6,
            F7,
            F8,
            F9,
            F10,
            F11,
            F12,
            F13,
            F14,
            F15,
            F16,
            F17,
            F18,
            F19,
            F20,
            F21,
            F22,
            F23,
            F24,
            Insert,
            Home,
            PageUp,
            Delete,
            End,
            PageDown,
            ArrowRight,
            ArrowLeft,
            ArrowDown,
            ArrowUp,
        ];
        let mut seen = BTreeSet::new();
        for k in all {
            let c = hid_code(k).unwrap_or_else(|| panic!("{k:?} unmapped"));
            assert!(keycodes::lookup(c).is_some(), "{k:?} -> 0x{c:02X}");
            seen.insert(c);
        }
        for c in MODIFIER_CODES {
            assert!(keycodes::lookup(c).is_some());
        }
        // Colliding aliases only: Plus/Equals, brackets, backslash/pipe, ;/:, //?.
        assert_eq!(seen.len(), all.len() - 6);
    }

    // ---- recorder

    #[test]
    fn recorder_converts_events_with_real_delays() {
        let mut r = Recorder::new();
        r.modifiers(mods_of(&ctrl()), 1.000);
        r.key(Key::C, Some(Key::C), true, false, &ctrl(), 1.020);
        r.key(Key::C, Some(Key::C), false, false, &ctrl(), 1.120);
        r.modifiers(mods_of(&NONE), 1.140);
        let rec = r.finish(1.140);
        assert_eq!(
            rec.actions,
            vec![
                mac(0xE0, true, 20),
                mac(0x06, true, 100),
                mac(0x06, false, 20),
                mac(0xE0, false, 0),
            ]
        );
        assert!(rec.skipped.is_empty());
    }

    #[test]
    fn clipboard_shortcuts_are_recorded_as_the_key() {
        // egui sends Copy instead of the C press; the release is a key event.
        let mut r = Recorder::new();
        r.modifiers(mods_of(&ctrl()), 1.0);
        r.clipboard(Clip::Copy, &ctrl(), 1.02);
        r.clipboard(Clip::Copy, &ctrl(), 1.05); // OS key repeat
        r.key(Key::C, Some(Key::C), false, false, &ctrl(), 1.10);
        r.modifiers(mods_of(&NONE), 1.15);
        assert_eq!(
            r.finish(1.15).actions,
            vec![
                mac(0xE0, true, 20),
                mac(0x06, true, 80),
                mac(0x06, false, 50),
                mac(0xE0, false, 0),
            ]
        );
        // Windows Ctrl+Insert also arrives as Copy: the release names the key.
        let mut r = Recorder::new();
        r.clipboard(Clip::Copy, &ctrl(), 0.0);
        r.key(Key::Insert, Some(Key::Insert), false, false, &ctrl(), 0.1);
        let codes: Vec<(u16, bool)> = r
            .finish(0.2)
            .actions
            .iter()
            .map(|a| (a.code, a.down))
            .collect();
        assert_eq!(
            codes,
            vec![(0xE0, true), (0x49, true), (0x49, false), (0xE0, false)]
        );
        // Shift+Delete (Windows cut) is guessed as Delete straight away.
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let mut r = Recorder::new();
        r.clipboard(Clip::Cut, &shift, 0.0);
        assert_eq!(r.actions()[1], mac(0x4C, true, 0));
        // A layout without Latin letters: the release carries the physical key.
        let mut r = Recorder::new();
        r.clipboard(Clip::Paste, &ctrl(), 0.0);
        r.key(Key::V, Some(Key::V), false, false, &ctrl(), 0.1);
        assert_eq!(r.actions()[1], mac(0x19, true, 100));
        assert_eq!(r.actions()[2], mac(0x19, false, 0));
    }

    #[test]
    fn a_paste_with_an_empty_clipboard_is_still_recorded() {
        // egui sends nothing for the press then; only the release arrives.
        let mut r = Recorder::new();
        r.modifiers(mods_of(&ctrl()), 0.0);
        r.key(Key::V, Some(Key::V), false, false, &ctrl(), 0.1);
        r.modifiers(mods_of(&NONE), 0.2);
        let codes: Vec<(u16, bool)> = r
            .finish(0.2)
            .actions
            .iter()
            .map(|a| (a.code, a.down))
            .collect();
        assert_eq!(
            codes,
            vec![(0xE0, true), (0x19, true), (0x19, false), (0xE0, false)]
        );
        // A stray release without the shortcut modifier is still ignored.
        let mut r = Recorder::new();
        r.key(Key::V, Some(Key::V), false, false, &NONE, 0.1);
        assert!(r.is_empty());
    }

    #[test]
    fn modifiers_on_a_key_event_come_before_the_key() {
        // Ctrl went down in the same frame as the key: Ctrl first.
        let mut r = Recorder::new();
        r.key(Key::V, Some(Key::V), true, false, &ctrl(), 2.0);
        let rec = r.finish(2.0);
        assert_eq!(rec.actions[0], mac(0xE0, true, 0));
        assert_eq!(rec.actions[1], mac(0x19, true, 0));
    }

    #[test]
    fn repeats_and_stray_releases_are_ignored() {
        let mut r = Recorder::new();
        r.key(Key::A, Some(Key::A), false, false, &NONE, 0.0); // pressed before recording
        r.key(Key::A, Some(Key::A), true, false, &NONE, 0.1);
        r.key(Key::A, Some(Key::A), true, true, &NONE, 0.2); // OS key repeat
        r.key(Key::A, Some(Key::A), true, false, &NONE, 0.3); // duplicate press
        r.key(Key::A, Some(Key::A), false, false, &NONE, 0.4);
        assert_eq!(
            r.finish(0.4).actions,
            vec![mac(0x04, true, 300), mac(0x04, false, 0)]
        );
    }

    #[test]
    fn unsupported_keys_are_skipped_and_named_once() {
        let mut r = Recorder::new();
        r.key(Key::Copy, None, true, false, &NONE, 0.0);
        r.key(Key::Copy, None, false, false, &NONE, 0.1);
        r.key(Key::Copy, None, true, false, &NONE, 0.2);
        r.key(Key::F30, None, true, false, &NONE, 0.3);
        r.key(Key::B, Some(Key::B), true, false, &NONE, 0.4);
        let rec = r.finish(0.5);
        assert_eq!(rec.skipped, vec!["Copy".to_string(), "F30".to_string()]);
        assert_eq!(rec.actions[0], mac(0x05, true, 100));
        assert_eq!(rec.actions.len(), 2);
    }

    #[test]
    fn the_physical_key_wins_over_the_layout_key() {
        // Dvorak: the key at the QWERTY "S" position types ";".
        let mut r = Recorder::new();
        r.key(Key::Semicolon, Some(Key::S), true, false, &NONE, 0.0);
        assert_eq!(r.actions()[0].code, 0x16);
        // Without a physical key the logical one is used.
        let mut r = Recorder::new();
        r.key(Key::Semicolon, None, true, false, &NONE, 0.0);
        assert_eq!(r.actions()[0].code, 0x33);
    }

    #[test]
    fn held_keys_are_released_when_the_recording_ends() {
        let mut r = Recorder::new();
        r.key(Key::W, Some(Key::W), true, false, &NONE, 0.0);
        r.key(Key::D, Some(Key::D), true, false, &NONE, 0.05);
        let rec = r.finish(0.25);
        assert_eq!(
            rec.actions,
            vec![
                mac(0x1A, true, 50),
                mac(0x07, true, 200),
                mac(0x1A, false, 0),
                mac(0x07, false, 0),
            ]
        );
        assert!(unbalanced_keys(
            &rec.actions
                .iter()
                .map(|a| SavedMacroAction::from(*a))
                .collect::<Vec<_>>()
        )
        .is_empty());
    }

    #[test]
    fn modifier_changes_are_recorded_once() {
        let mut r = Recorder::new();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        r.modifiers(mods_of(&shift), 0.0);
        r.modifiers(mods_of(&shift), 0.1); // unchanged
        r.modifiers(mods_of(&NONE), 0.3);
        assert_eq!(
            r.finish(0.3).actions,
            vec![mac(0xE1, true, 300), mac(0xE1, false, 0)]
        );
    }

    #[test]
    fn the_recorder_stops_at_255_actions_and_clamps_delays() {
        let mut r = Recorder::new();
        for i in 0..300 {
            r.key(Key::A, Some(Key::A), i % 2 == 0, false, &NONE, f64::from(i));
        }
        assert!(r.is_full());
        assert_eq!(r.len(), 254, "no room for one more press and its release");
        let mut r = Recorder::new();
        r.key(Key::A, Some(Key::A), true, false, &NONE, 0.0);
        r.key(Key::A, Some(Key::A), false, false, &NONE, 100_000.0);
        assert_eq!(r.actions()[0].delay_ms, MAX_U24, "24-bit delay limit");
    }

    #[test]
    fn keys_held_at_the_limit_are_still_released() {
        // A held all along, B tapped until the recorder is full: the
        // recording ends balanced and within 255 steps.
        let mut r = Recorder::new();
        r.key(Key::A, Some(Key::A), true, false, &NONE, 0.0);
        for i in 0..400 {
            r.key(Key::B, Some(Key::B), i % 2 == 0, false, &NONE, f64::from(i));
        }
        assert!(r.is_full());
        let rec = r.finish(500.0);
        let steps: Vec<SavedMacroAction> = rec.actions.iter().copied().map(Into::into).collect();
        assert!(steps.len() <= MAX_ACTIONS, "{}", steps.len());
        assert!(unbalanced_keys(&steps).is_empty());
        assert_eq!(steps.last().map(|a| (a.code, a.down)), Some((0x04, false)));
    }

    #[test]
    fn a_recording_appended_to_a_macro_fits_with_its_releases() {
        // 250 steps already: room for 5 more, releases included.
        let mut r = Recorder::with_room(MAX_ACTIONS - 250);
        r.key(Key::A, Some(Key::A), true, false, &NONE, 0.0);
        r.key(Key::B, Some(Key::B), true, false, &NONE, 0.1);
        r.key(Key::C, Some(Key::C), true, false, &NONE, 0.2); // no room
        assert!(r.is_full());
        let rec = r.finish(1.0);
        assert_eq!(rec.actions.len(), 4);
        let mut m: Vec<SavedMacroAction> = (0..250).map(|i| act(0x04, i % 2 == 0, 0)).collect();
        let new: Vec<SavedMacroAction> = rec.actions.into_iter().map(Into::into).collect();
        step_insert(&mut m, None, &new);
        assert_eq!(m.len(), 254);
        assert!(unbalanced_keys(&m).is_empty(), "nothing cut off");
        assert!(Recorder::with_room(1).is_full());
    }

    #[test]
    fn unbalanced_macros_cannot_be_bound_until_released() {
        let mut m = SavedMacro::new("x");
        m.actions = vec![act(0xE0, true, 0), act(0x06, true, 10), act(0xE0, false, 0)];
        assert_eq!(macro_problem(&m), Some(strings::UNBALANCED_BLOCK));
        // "Tambahkan pelepasan otomatis": the missing releases, last pressed
        // first.
        let mut two = vec![act(0xE0, true, 0), act(0x06, true, 10)];
        assert_eq!(append_releases(&mut two), 2);
        assert_eq!(
            two[2..],
            [act(0x06, false, DEFAULT_STEP_DELAY_MS), act(0xE0, false, 0)]
        );
        assert_eq!(append_releases(&mut m.actions), 1);
        assert_eq!(macro_problem(&m), None);
        assert_eq!(append_releases(&mut m.actions), 0, "nothing left");
        // No room for all of them: nothing is added, the problem stays.
        // 4 and 5 pressed, then 6 tapped up to the limit (pressed last).
        let mut full: Vec<SavedMacroAction> = (0..MAX_ACTIONS as u16)
            .map(|i| match i {
                0 | 1 => act(0x04 + i, true, 0),
                _ => act(0x06, i % 2 == 0, 0),
            })
            .collect();
        assert_eq!(unbalanced_keys(&full), vec![0x04, 0x05, 0x06]);
        assert_eq!(append_releases(&mut full), 0);
        assert_eq!(full.len(), MAX_ACTIONS);
    }

    // ---- step editor

    fn steps() -> Vec<SavedMacroAction> {
        vec![act(1, true, 10), act(2, true, 20), act(3, false, 30)]
    }

    #[test]
    fn step_edits() {
        let mut s = steps();
        step_set_delay(&mut s, 1, 99);
        step_set_delay(&mut s, 2, u32::MAX);
        assert_eq!((s[1].delay_ms, s[2].delay_ms), (99, MAX_U24));
        step_set_delay(&mut s, 9, 5); // out of range: ignored
        step_toggle(&mut s, 0);
        assert!(!s[0].down);
        assert_eq!(step_move_up(&mut s, 2), 1);
        assert_eq!(s.iter().map(|a| a.code).collect::<Vec<_>>(), vec![1, 3, 2]);
        assert_eq!(step_move_up(&mut s, 0), 0);
        assert_eq!(step_move_down(&mut s, 1), 2);
        assert_eq!(step_move_down(&mut s, 2), 2, "already last");
        assert_eq!(s.iter().map(|a| a.code).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn step_delete_keeps_a_sensible_selection() {
        let mut s = steps();
        assert_eq!(step_delete(&mut s, 2), Some(1));
        assert_eq!(step_delete(&mut s, 0), Some(0));
        assert_eq!(step_delete(&mut s, 0), None);
        assert_eq!(step_delete(&mut s, 0), None, "empty");
        assert!(s.is_empty());
    }

    #[test]
    fn step_insert_after_a_step_or_at_the_end() {
        let mut s = steps();
        let t = tap(9, 10);
        assert_eq!(step_insert(&mut s, Some(0), &t), Some(2));
        assert_eq!(
            s.iter().map(|a| a.code).collect::<Vec<_>>(),
            vec![1, 9, 9, 2, 3]
        );
        assert_eq!(step_insert(&mut s, None, &t[..1]), Some(5));
        assert_eq!(s.len(), 6);
        assert_eq!(step_insert(&mut s, Some(99), &t[..1]), Some(6));
        assert_eq!(t[0], act(9, true, 10));
        assert_eq!(t[1], act(9, false, 0));
    }

    #[test]
    fn step_insert_respects_the_255_limit() {
        let mut s: Vec<_> = (0..254).map(|_| act(4, true, 0)).collect();
        let t = tap(5, 1);
        assert_eq!(step_insert(&mut s, None, &t), Some(254));
        assert_eq!(s.len(), 255);
        assert_eq!(step_insert(&mut s, None, &t), None);
        assert_eq!(s.len(), 255);
    }

    #[test]
    fn unbalanced_keys_and_problems() {
        assert_eq!(
            unbalanced_keys(&[act(1, true, 0), act(2, true, 0), act(1, false, 0)]),
            vec![2]
        );
        assert!(unbalanced_keys(&[act(1, true, 0), act(1, false, 0)]).is_empty());
        let mut m = SavedMacro::new("x");
        assert_eq!(macro_problem(&m), Some(strings::EMPTY_MACRO));
        m.actions = steps(); // keys 1 and 2 are never released
        assert_eq!(macro_problem(&m), Some(strings::UNBALANCED_BLOCK));
        m.actions = vec![act(1, true, 10), act(1, false, 0)];
        assert_eq!(macro_problem(&m), None);
        m.actions[0].delay_ms = MAX_U24 + 1;
        assert_eq!(macro_problem(&m), Some(strings::DELAY_TOO_LONG));
        m.actions = (0..256).map(|_| act(4, true, 0)).collect();
        assert_eq!(macro_problem(&m), Some(strings::TOO_MANY));
    }

    #[test]
    fn labels() {
        assert_eq!(seconds_label(240), "0.24");
        assert_eq!(seconds_label(0), "0.00");
        assert_eq!(code_label(0x06), "C");
        assert_eq!(code_label(0xE0), "Ctrl");
        assert_eq!(code_label(0x28), "Enter");
        assert!(code_label(0xFFFF).starts_with("0x"));
    }

    // ---- library

    #[test]
    fn library_new_names_are_unique() {
        let mut lib = Vec::new();
        assert_eq!(library_new(&mut lib), 0);
        assert_eq!(library_new(&mut lib), 1);
        assert_eq!(library_new(&mut lib), 2);
        let names: Vec<_> = lib.iter().map(|m| m.name.clone()).collect();
        assert_eq!(names, vec!["Macro baru", "Macro baru 2", "Macro baru 3"]);
        assert_eq!(lib[0].mode, 0);
        assert_eq!(lib[0].repeat, 1);
    }

    #[test]
    fn library_duplicate_and_delete() {
        let mut lib = vec![SavedMacro::new("A"), SavedMacro::new("B")];
        lib[0].actions = steps();
        assert_eq!(library_duplicate(&mut lib, 0), Some(1));
        assert_eq!(lib[1].name, "A (salinan)");
        assert_eq!(lib[1].actions, lib[0].actions);
        assert_eq!(library_duplicate(&mut lib, 0), Some(1));
        assert_eq!(lib[1].name, "A (salinan) 2");
        assert_eq!(library_duplicate(&mut lib, 9), None);
        assert_eq!(lib.len(), 4);
        assert_eq!(library_delete(&mut lib, 3), Some(2));
        assert_eq!(library_delete(&mut lib, 0), Some(0));
        assert_eq!(library_delete(&mut lib, 7), None);
        assert_eq!(lib.len(), 2);
        lib.truncate(1);
        assert_eq!(library_delete(&mut lib, 0), None);
    }

    #[test]
    fn saved_macro_round_trips_through_proto_actions() {
        let mut m = SavedMacro::new("rt");
        m.actions = steps();
        let proto = m.to_actions();
        assert_eq!(proto[1], mac(2, true, 20));
        let back: Vec<SavedMacroAction> = proto.into_iter().map(Into::into).collect();
        assert_eq!(back, m.actions);
    }

    // ---- jobs

    fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let [lo, hi] = v.to_le_bytes();
        reply(0x23, &[0, key, layout, lo, hi])
    }

    fn data_reply() -> [u8; 64] {
        reply(0x20, &[0, 0, 1, 0])
    }

    fn mode_reply(key: u8, len: u8, mode: PlayMode, repeat: u16, interval: u32) -> [u8; 64] {
        let p = macros::mode_set(key, len, mode, repeat, interval)
            .unwrap()
            .payload;
        reply(0x21, &p)
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    const KEY: u8 = 0x0A;

    /// The replies of a successful `bind_macro` of one action.
    fn queue_bind(m: &mut MockTransport, read_back: PlayMode) {
        m.push_reply(key_reply(KEY, 8, 0x0000)); // Mode read
        m.push_reply(data_reply());
        m.push_reply(key_reply(KEY, 8, 0x0006)); // Mode SET echo
        m.push_reply(key_reply(KEY, 8, 0x0006)); // Mode read-back
        m.push_reply(mode_reply(KEY, 1, PlayMode::WhileHeld, 2, 100)); // SET reply
        m.push_reply(mode_reply(KEY, 1, read_back, 2, 100)); // verify GET
    }

    /// Replies of `macro_binding` (bound) and `key_settings` (macro mode).
    fn queue_reread(m: &mut MockTransport) {
        m.push_reply(key_reply(KEY, 8, 0x0006));
        m.push_reply(mode_reply(KEY, 1, PlayMode::WhileHeld, 2, 100));
        for (layout, v) in [
            (8, 0x0006),
            (4, 2000),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(KEY, layout, v));
        }
    }

    fn one() -> [MacroAction; 1] {
        [mac(4, true, 0)]
    }

    #[test]
    fn bind_job_success_rereads_binding_and_settings() {
        let mut m = MockTransport::new();
        queue_bind(&mut m, PlayMode::WhileHeld);
        queue_reread(&mut m);
        let out = bind_job(&mut kb(m), KEY, &one(), PlayMode::WhileHeld, 2, 100).unwrap();
        assert_eq!(out.write, Ok(()));
        let b = MacroBinding {
            len: 1,
            mode: PlayMode::WhileHeld,
            repeat: 2,
            interval_ms: 100,
        };
        assert_eq!(out.binding, Ok(Some(b)));
        assert_eq!(
            out.settings.as_ref().unwrap().mode.advanced,
            openhe_proto::key::Advanced::Macro
        );
        let fx = bind_effects(Ok(out));
        assert!(fx.bound && !fx.request_reload);
        assert_eq!(fx.binding, Some(Some(b)));
        assert!(fx.patch.is_some());
        assert_eq!(fx.error, None);
    }

    #[test]
    fn bind_verify_failure_shows_the_true_binding() {
        let mut m = MockTransport::new();
        queue_bind(&mut m, PlayMode::Once); // read-back differs
        m.push_reply(key_reply(KEY, 8, 0x0006));
        m.push_reply(mode_reply(KEY, 1, PlayMode::Once, 2, 100));
        for (layout, v) in [
            (8, 0x0006),
            (4, 2000),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(KEY, layout, v));
        }
        let out = bind_job(&mut kb(m), KEY, &one(), PlayMode::WhileHeld, 2, 100).unwrap();
        assert!(matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed")));
        let fx = bind_effects(Ok(out));
        assert!(!fx.bound && !fx.request_reload);
        assert_eq!(fx.binding.unwrap().unwrap().mode, PlayMode::Once);
        assert_eq!(
            fx.error.as_deref(),
            Some(format!("{} {}", i18n::error::VERIFY, strings::RELOADED_HINT).as_str())
        );
        assert!(fx.technical.unwrap().contains("verify failed"));
    }

    #[test]
    fn invalid_macro_is_refused_before_io_then_reread() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0010)); // macro_binding: not a macro key
        for (layout, v) in [
            (8, 0x0010),
            (4, 2000),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(KEY, layout, v));
        }
        let out = bind_job(&mut kb(m), KEY, &[], PlayMode::Once, 1, 100).unwrap();
        assert!(matches!(out.write, Err(Error::InvalidInput(_))));
        assert_eq!(out.binding, Ok(None));
        let fx = bind_effects(Ok(out));
        assert_eq!(fx.binding, Some(None));
        assert!(fx.error.is_some() && !fx.bound);
    }

    #[test]
    fn a_failing_reread_asks_for_a_reload() {
        let mut m = MockTransport::new();
        queue_bind(&mut m, PlayMode::WhileHeld);
        // No replies left: both re-reads time out.
        let out = bind_job(&mut kb(m), KEY, &one(), PlayMode::WhileHeld, 2, 100).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.binding, Err(Error::Timeout));
        let fx = bind_effects(Ok(out));
        assert!(fx.bound && fx.request_reload);
        assert_eq!(fx.binding, None);
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
    fn disconnect_during_bind_is_a_partial_write_message() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0000));
        m.push_reply(data_reply());
        let mut k = Keyboard::new(
            UnplugAfter {
                inner: m,
                writes_left: 2, // Mode read, data packet; then the Mode SET fails
            },
            Model::Win68HeMax,
        );
        let r = bind_job(&mut k, KEY, &one(), PlayMode::WhileHeld, 2, 100);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = bind_effects(r);
        assert_eq!(fx.error.as_deref(), Some(strings::PARTIAL_WRITE));
        assert!(!fx.bound && !fx.request_reload && fx.binding.is_none());
    }

    #[test]
    fn device_failure_on_write_is_still_reread() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0000));
        m.push_reply(reply(0x7F, &[])); // data packet rejected
        m.push_reply(key_reply(KEY, 8, 0x0000)); // macro_binding: still no macro
        for (layout, v) in [
            (8, 0x0000),
            (4, 2000),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(KEY, layout, v));
        }
        let out = bind_job(&mut kb(m), KEY, &one(), PlayMode::Once, 1, 100).unwrap();
        assert_eq!(out.write, Err(Error::Frame(FrameError::DeviceFail)));
        assert_eq!(out.binding, Ok(None));
    }

    #[test]
    fn other_bind_errors_reload_everything() {
        let fx = bind_effects(Err(Error::Io(format!(
            "{}: boom",
            crate::worker::INTERNAL_ERROR
        ))));
        assert!(fx.request_reload);
        assert_eq!(fx.error.as_deref(), Some(i18n::error::INTERNAL));
    }

    #[test]
    fn binding_job_reads_the_binding() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0026));
        m.push_reply(mode_reply(KEY, 5, PlayMode::Toggle, 3, 250));
        assert_eq!(
            binding_job(&mut kb(m), KEY),
            Ok(Some(MacroBinding {
                len: 5,
                mode: PlayMode::Toggle,
                repeat: 3,
                interval_ms: 250
            }))
        );
        let b = MacroBinding {
            len: 6,
            mode: PlayMode::Once,
            repeat: 1,
            interval_ms: 100,
        };
        assert_eq!(
            binding_text(&b),
            "6 langkah · Sekali per tekan · 1× · 100 ms"
        );
    }

    /// The keyboard reports len 0 (PROTOCOL 7.x finding 24): no "0 langkah".
    #[test]
    fn a_binding_without_a_known_length_leaves_the_step_count_out() {
        let b = MacroBinding {
            len: 0,
            mode: PlayMode::Toggle,
            repeat: 2,
            interval_ms: 50,
        };
        let text = binding_text(&b);
        assert!(!text.contains(strings::STEPS), "{text}");
        assert!(text.starts_with(mode_title(PlayMode::Toggle)), "{text}");
    }

    #[test]
    fn the_fn_key_cannot_be_a_target() {
        assert!(target_blocked(0x01));
        assert!(!target_blocked(0x0A));
    }

    // ---- rendering

    #[test]
    fn the_page_renders_offline_and_connected() {
        use crate::app::test_support::{offline_worker, run_frames, snapshot68};
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;
        let worker = offline_worker();
        let theme = Theme::default();
        let mut d = AppData::default();
        let mut m = SavedMacro::new("Salin");
        m.actions = vec![
            act(0xE0, true, 20),
            act(0x06, true, 15),
            act(0x06, false, 0),
        ];
        d.macros = vec![m, SavedMacro::new("Kosong")];
        // Saving must not touch the user's real data file in a test.
        d.set_save_blocked(true);
        let data = RefCell::new(d);
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
            for recording in [false, true] {
                page.recorder = recording.then(Recorder::new);
                // The key picker window (needs a connected keyboard).
                page.picking = connected && !recording;
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

    #[test]
    fn a_key_with_a_function_shows_it_and_needs_a_confirmed_bind() {
        use crate::app::test_support::snapshot68;
        use openhe_proto::key::Advanced;
        let mut snap = snapshot68();
        snap.keys.get_mut(&0x04).unwrap().mode.advanced = Advanced::Socd;
        snap.keys.get_mut(&0x16).unwrap().mode.advanced = Advanced::Macro;
        assert_eq!(replaced_function(&snap, 0x04), Some(Func::Socd));
        assert_eq!(replaced_function(&snap, 0x16), None, "a macro is rebound");
        assert_eq!(replaced_function(&snap, 0x1A), None);
        let text = replace_warning(Func::Socd);
        assert!(
            text.contains("SOCD") && text.contains(strings::REPLACES_SOCD),
            "{text}"
        );
        assert!(!replace_warning(Func::Mt).contains(strings::REPLACES_SOCD));

        let mut page = Page::default();
        // A key with a function: the first click only asks.
        assert!(!page.bind_clicked(Some(Func::Socd)));
        assert!(page.confirm_replace);
        assert!(page.bind_clicked(Some(Func::Socd)), "confirmed");
        assert!(!page.confirm_replace);
        // A plain key binds at once.
        assert!(page.bind_clicked(None));
    }

    #[test]
    fn egui_key_events_are_recorded_into_the_selected_macro() {
        use crate::app::test_support::offline_worker;
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;
        let worker = offline_worker();
        let theme = Theme::default();
        let mut d = AppData::default();
        d.macros = vec![SavedMacro::new("Rekaman")];
        // Never write the real data file from a test.
        d.set_save_blocked(true);
        let data = RefCell::new(d);
        let egui_ctx = egui::Context::default();
        crate::theme::apply(&egui_ctx);
        let status = ConnectionStatus::Searching;
        let mut page = Page::default();
        page.start_recording(0.0, 0);

        let key = |key, pressed, mods: Modifiers| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: mods,
        };
        let frames = [
            // egui-winit turns the Ctrl+C press into Event::Copy.
            (1.00, vec![Event::Copy], ctrl()),
            (1.10, vec![key(Key::C, false, ctrl())], ctrl()),
            (1.15, vec![], Modifiers::NONE),
        ];
        for (time, events, modifiers) in frames {
            let input = egui::RawInput {
                time: Some(time),
                modifiers,
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = egui_ctx.run(input, |c| {
                egui::CentralPanel::default().show(c, |ui| {
                    let ctx = AppCtx {
                        worker: &worker,
                        snapshot: None,
                        status: &status,
                        busy: false,
                        theme: &theme,
                        data: &data,
                    };
                    page.sync_selection(&ctx);
                    page.record_frame(&ctx, ui);
                });
            });
        }
        let ctx = AppCtx {
            worker: &worker,
            snapshot: None,
            status: &status,
            busy: false,
            theme: &theme,
            data: &data,
        };
        page.stop_recording(&ctx, 1.15, false);
        assert!(page.recorder.is_none());
        let got = data.borrow().macros[0].actions.clone();
        assert_eq!(
            got,
            vec![
                act(0xE0, true, 0),
                act(0x06, true, 100),
                act(0x06, false, 50),
                act(0xE0, false, 0),
            ]
        );
    }
}

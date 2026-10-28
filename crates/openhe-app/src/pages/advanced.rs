//! Tombol Lanjutan: DKS, MT, TGL, MPT, SOCD and END keys.
//!
//! Mockup `docs/design/mockups/02-tombol-lanjutan.png`. Keys carry a badge in
//! the colour of their function (the badges come from the snapshot's
//! `KeySettings.mode.advanced`, no extra reads). A table lists the assigned
//! functions with a description; its details are read lazily by one job for
//! all keys that are not cached yet. Selecting a key reads its current config
//! (`Keyboard::advanced`) into the right-panel editor; "Terapkan" and
//! "Hapus" run [`apply_job`], which writes with `set_advanced`, then always
//! re-reads the key, its SOCD partner(s) and their settings, so the screen
//! never shows stale badges.

use super::remap::{code_field, code_name, FN_KEY};
use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::pages::performance::SentOn;
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{reference_matrix, KeyDecoration, KeyboardView, Selection};
use crate::widgets::segmented::Segmented;
use crate::widgets::slider_mm::{mm_short, mm_value_box, SliderMm};
use crate::worker::{DeviceSnapshot, Pending};
use eframe::egui::{
    self, text::LayoutJob, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect,
    RichText, Sense, Ui,
};
use openhe_device::{AdvancedConfig, Error, KeySettings, Keyboard, Transport};
use openhe_proto::advanced::{
    trps_decode, trps_encode, Dks, End, Mpt, Mt, Socd, SocdMode, Tgl, MAX_DEPTH_UM, MAX_SOCD_DELAY,
    MIN_DEPTH_UM,
};
use openhe_proto::key::Advanced;
use openhe_proto::{keycodes, Micrometers, Model};
use std::collections::{BTreeMap, BTreeSet};

mod strings {
    pub const ALL: &str = "Semua";
    pub const EMPTY_TABLE: &str =
        "Belum ada tombol dengan fungsi lanjutan. Pilih tombol di keyboard lalu atur fungsinya di panel kanan.";
    pub const COL_KEY: &str = "TOMBOL";
    pub const COL_FUNC: &str = "FUNGSI";
    pub const COL_BEHAVIOUR: &str = "PERILAKU";
    pub const COL_ACTION: &str = "AKSI";
    pub const EDIT: &str = "Edit";
    pub const EDITING: &str = "Sedang diedit";
    pub const READING_TABLE: &str = "Membaca detail fungsi…";
    pub const UNKNOWN_BEHAVIOUR: &str = "Detail belum terbaca";

    pub const NOTHING_SELECTED: &str = "Pilih tombol";
    pub const NOTHING_SELECTED_BODY: &str =
        "Klik satu tombol di keyboard untuk memberinya fungsi lanjutan atau mengubahnya.";
    pub const READING: &str = "Membaca fungsi tombol…";
    pub const READ_FAILED: &str = "Gagal membaca fungsi tombol";
    pub const MACRO_HINT: &str =
        "Tombol ini terhubung ke macro. Mengatur fungsi di sini menggantikan macro tersebut.";
    pub const CHANGE_TYPE: &str = "Jenis fungsi";

    pub const DESC_DKS: &str =
        "Satu tombol, sampai 4 aksi berbeda tergantung seberapa dalam ditekan dan dilepas.";
    pub const DESC_MT: &str = "Ketuk sebentar mengirim satu tombol, ditahan mengirim tombol lain.";
    pub const DESC_TGL: &str = "Ketuk sekali, tombol tetap tertekan; ketuk lagi untuk melepas.";
    pub const DESC_MPT: &str = "Tiga tombol berbeda sesuai kedalaman tekan.";
    pub const DESC_SOCD: &str =
        "Dua tombol berlawanan (mis. A dan D) ditekan bersamaan: pilih tombol mana yang menang.";
    pub const DESC_END: &str = "Tombol mengirim tombol lain saat dilepas.";

    pub const DKS_PHASES: [&str; 4] = [
        "Mulai\ntekan",
        "Tekan\npenuh",
        "Mulai\nlepas",
        "Lepas\npenuh",
    ];
    pub const DKS_LEGEND: &str = "Lingkaran = tekan · garis = tahan antar titik";
    pub const DKS_FIRST: &str = "Titik aktuasi pertama";
    pub const DKS_BOTTOM: &str = "Dasar tetap 3.40 mm";

    pub const HOLD_KEY: &str = "Tombol saat ditahan";
    pub const TAP_KEY: &str = "Tombol saat diketuk";
    pub const HOLD_DELAY: &str = "Batas waktu tahan";
    pub const TGL_KEY: &str = "Tombol yang ditahan";
    pub const TGL_DELAY: &str = "Jeda";
    pub const MPT_POINT: &str = "Titik";
    pub const END_KEY: &str = "Tombol saat dilepas";

    pub const SOCD_PAIR: &str = "Pasangan tombol";
    pub const SOCD_PICK: &str = "Pilih di keyboard";
    pub const SOCD_NO_PARTNER: &str = "belum dipilih";
    pub const SOCD_MODE: &str = "Jika keduanya ditekan";
    pub const SOCD_LAST: &str = "Yang terakhir ditekan menang";
    pub const SOCD_WINS: &str = "menang";
    pub const SOCD_NEUTRAL: &str = "Netral (keduanya batal)";
    pub const SOCD_OUTPUT: &str = "Yang dikirim";
    pub const SOCD_POSITIONS: &str = "Posisi tombol";
    pub const SOCD_CODES: &str = "Kode pilihan";
    pub const SOCD_DELAY: &str = "Jeda";
    pub const SOCD_OUT_FOR: &str = "Kirim untuk";

    pub const DELETE: &str = "Hapus";
    pub const APPLY: &str = "Terapkan";
    pub const APPLYING: &str = "Menerapkan…";
    pub const APPLIED: &str = "Tersimpan di keyboard";
    pub const SAVED_IN_PROFILE: &str = "Disimpan di profil";
    pub const WORKS_WITHOUT_APP: &str = "tetap aktif tanpa app";
    pub const APPLY_FAILED: &str = "Gagal menerapkan";
    pub const RELOADED_HINT: &str = "Fungsi tombol dibaca ulang dari keyboard.";
    pub const PARTIAL_WRITE: &str =
        "Keyboard terputus. Perubahan mungkin sudah sebagian terkirim; nilai dibaca ulang saat tersambung.";

    // Validation (shown before anything is sent).
    pub const V_DKS_KEY: &str = "Pilih minimal satu tombol untuk DKS.";
    pub const V_DKS_ACTION: &str = "Nyalakan minimal satu lingkaran atau garis di tabel DKS.";
    pub const V_FIRST: &str = "Titik aktuasi pertama harus antara 0.10 dan 3.40 mm.";
    pub const V_MT_KEYS: &str = "Pilih tombol untuk ditahan dan untuk diketuk.";
    pub const V_TGL_KEY: &str = "Pilih tombol yang ditahan.";
    pub const V_MPT_KEYS: &str = "Pilih setidaknya satu tombol untuk salah satu kedalaman.";
    pub const V_END_KEY: &str = "Pilih tombol yang dikirim saat dilepas.";
    pub const V_DELAY: &str = "Waktu harus antara 10 dan 1000 ms.";
    pub const V_DEPTH_RANGE: &str = "Kedalaman harus antara 0.10 dan 3.40 mm.";
    pub const V_DEPTH_ORDER: &str =
        "Kedalaman harus makin dalam: titik 2 lebih dalam dari titik 1, titik 3 lebih dalam dari titik 2.";
    pub const V_SOCD_PARTNER: &str = "Pilih tombol pasangan di keyboard.";
    pub const V_SOCD_SAME: &str = "Pasangan harus tombol yang berbeda.";
    pub const V_SOCD_DELAY: &str = "Jeda SOCD harus antara 0 dan 50.";
    pub const V_FN_KEY: &str = "Tombol Fn tidak bisa diberi fungsi lanjutan.";
    pub const V_SOCD_FN: &str = "Tombol Fn tidak bisa jadi pasangan SOCD.";
    pub const SOCD_PICKING_NOT_FN: &str = "Klik tombol pasangan di keyboard (bukan Fn)…";
    pub const WARN_FUNC: &str = "Fungsi";
    pub const WARN_ON_KEY: &str = "di tombol";
    pub const WARN_REMOVED: &str = "akan dihapus.";
    pub const MACRO_NAME: &str = "Macro";

    // Table descriptions.
    pub const D_ACTUATION: &str = "aktuasi";
    pub const D_MM: &str = "mm";
    pub const D_MS: &str = "ms";
    pub const D_TAP: &str = "Ketuk";
    pub const D_HOLD: &str = "tahan";
    pub const D_TGL_ONCE: &str = "Ketuk sekali";
    pub const D_TGL_KEPT: &str = "tetap tertekan";
    pub const D_TGL_AGAIN: &str = "ketuk lagi = lepas";
    pub const D_RELEASE: &str = "Lepas";

    pub const V_SOCD_OUTPUT: &str = "Pilih tombol yang dikirim untuk kedua sisi.";
}
use strings as t;

// ------------------------------------------------------------- functions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Dks,
    Mt,
    Tgl,
    Mpt,
    Socd,
    End,
}

pub const FUNCS: [Func; 6] = [
    Func::Dks,
    Func::Mt,
    Func::Tgl,
    Func::Mpt,
    Func::Socd,
    Func::End,
];

impl Func {
    pub fn label(self) -> &'static str {
        match self {
            Func::Dks => "DKS",
            Func::Mt => "MT",
            Func::Tgl => "TGL",
            Func::Mpt => "MPT",
            Func::Socd => "SOCD",
            Func::End => "END",
        }
    }

    /// Badge colour (mockup): DKS #A78BFA, MT #38BDF8, TGL #34D399,
    /// MPT #F472B6, SOCD #FFB224, END #F87171.
    pub fn color(self) -> Color32 {
        match self {
            Func::Dks => Color32::from_rgb(0xA7, 0x8B, 0xFA),
            Func::Mt => Color32::from_rgb(0x38, 0xBD, 0xF8),
            Func::Tgl => Color32::from_rgb(0x34, 0xD3, 0x99),
            Func::Mpt => Color32::from_rgb(0xF4, 0x72, 0xB6),
            Func::Socd => Color32::from_rgb(0xFF, 0xB2, 0x24),
            Func::End => Color32::from_rgb(0xF8, 0x71, 0x71),
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Func::Dks => t::DESC_DKS,
            Func::Mt => t::DESC_MT,
            Func::Tgl => t::DESC_TGL,
            Func::Mpt => t::DESC_MPT,
            Func::Socd => t::DESC_SOCD,
            Func::End => t::DESC_END,
        }
    }

    pub fn of_nibble(a: Advanced) -> Option<Func> {
        Some(match a {
            Advanced::Dks => Func::Dks,
            Advanced::Mt => Func::Mt,
            Advanced::Tgl => Func::Tgl,
            Advanced::Mpt => Func::Mpt,
            Advanced::Socd => Func::Socd,
            Advanced::End => Func::End,
            Advanced::None | Advanced::Macro => return None,
        })
    }

    pub fn of_config(c: &AdvancedConfig) -> Option<Func> {
        Some(match c {
            AdvancedConfig::Dks(_) => Func::Dks,
            AdvancedConfig::Mt(_) => Func::Mt,
            AdvancedConfig::Tgl(_) => Func::Tgl,
            AdvancedConfig::Mpt(_) => Func::Mpt,
            AdvancedConfig::Socd(_) => Func::Socd,
            AdvancedConfig::End(_) => Func::End,
            AdvancedConfig::None | AdvancedConfig::Macro => return None,
        })
    }
}

// ----------------------------------------------------------- editor state

const DEFAULT_DELAY_MS: u16 = 200;
const DEFAULT_FIRST_UM: u16 = 1400;

/// DKS: four bindings, each with seven flags in the UI order
/// `[press@P0, hold P0-P1, press@P1, hold P1-P2, press@P2, hold P2-P3,
/// press@P3]`. They map to the `trps` byte with `trps_encode`:
/// 0x01, 0x02, 0x04, 0x18, 0x20, 0x40, 0x80 (PROTOCOL 3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DksForm {
    pub codes: [u16; 4],
    pub flags: [[bool; 7]; 4],
    pub first: u16,
}

impl Default for DksForm {
    fn default() -> Self {
        DksForm {
            codes: [0; 4],
            flags: [[false; 7]; 4],
            first: DEFAULT_FIRST_UM,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtForm {
    pub hold: u16,
    pub tap: u16,
    pub delay_ms: u16,
}

impl Default for MtForm {
    fn default() -> Self {
        MtForm {
            hold: 0,
            tap: 0,
            delay_ms: DEFAULT_DELAY_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TglForm {
    pub code: u16,
    pub delay_ms: u16,
}

impl Default for TglForm {
    fn default() -> Self {
        TglForm {
            code: 0,
            delay_ms: DEFAULT_DELAY_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MptForm {
    pub codes: [u16; 3],
    pub depths: [u16; 3],
}

impl Default for MptForm {
    fn default() -> Self {
        MptForm {
            codes: [0; 3],
            depths: [1000, 2000, 3000],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocdForm {
    /// The other key of the pair (picked on the keyboard).
    pub partner: Option<u8>,
    /// The edited key is `pos1` of the pair (else `pos2`).
    pub key_first: bool,
    /// `false`: bind the two key positions; `true`: send chosen codes.
    pub bind_values: bool,
    pub out_key: u16,
    pub out_partner: u16,
    pub mode: SocdMode,
    pub delay: u16,
}

impl Default for SocdForm {
    fn default() -> Self {
        SocdForm {
            partner: None,
            key_first: true,
            bind_values: false,
            out_key: 0,
            out_partner: 0,
            mode: SocdMode::LastInput,
            delay: 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndForm {
    pub code: u16,
}

/// All forms of one key; `func` says which one is active. Forms of the other
/// functions are kept, so switching the type back and forth loses nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    /// Default code of the key being edited.
    pub key: u8,
    pub func: Func,
    pub dks: DksForm,
    pub mt: MtForm,
    pub tgl: TglForm,
    pub mpt: MptForm,
    pub socd: SocdForm,
    pub end: EndForm,
    /// What the keyboard had when the editor was loaded.
    pub loaded: AdvancedConfig,
}

impl Editor {
    pub fn new(key: u8, cfg: &AdvancedConfig) -> Editor {
        let mut e = Editor {
            key,
            func: Func::Dks,
            dks: DksForm::default(),
            mt: MtForm::default(),
            tgl: TglForm::default(),
            mpt: MptForm::default(),
            socd: SocdForm::default(),
            end: EndForm::default(),
            loaded: *cfg,
        };
        match cfg {
            AdvancedConfig::Dks(d) => {
                e.func = Func::Dks;
                e.dks = DksForm {
                    codes: d.bindings,
                    flags: d.trps.map(trps_decode),
                    first: d.first_actuation.0,
                };
            }
            AdvancedConfig::Mt(m) => {
                e.func = Func::Mt;
                e.mt = MtForm {
                    hold: m.hold,
                    tap: m.tap,
                    delay_ms: m.delay_ms,
                };
            }
            AdvancedConfig::Tgl(g) => {
                e.func = Func::Tgl;
                e.tgl = TglForm {
                    code: g.code,
                    delay_ms: g.delay_ms,
                };
            }
            AdvancedConfig::Mpt(m) => {
                e.func = Func::Mpt;
                e.mpt = MptForm {
                    codes: m.codes,
                    depths: m.depths.map(|d| d.0),
                };
            }
            AdvancedConfig::End(n) => {
                e.func = Func::End;
                e.end = EndForm { code: n.code };
            }
            AdvancedConfig::Socd(s) => {
                e.func = Func::Socd;
                let key_first = s.pos1 == key;
                let partner = if key_first { s.pos2 } else { s.pos1 };
                let (out_key, out_partner) = if key_first {
                    (s.key1, s.key2)
                } else {
                    (s.key2, s.key1)
                };
                e.socd = SocdForm {
                    partner: Some(partner),
                    key_first,
                    bind_values: s.bind_values,
                    out_key,
                    out_partner,
                    mode: s.mode,
                    delay: s.delay,
                };
            }
            AdvancedConfig::None | AdvancedConfig::Macro => {}
        }
        e
    }

    /// The config the form describes, or the Indonesian reason it cannot be
    /// sent. Everything `set_advanced` would reject is caught here first.
    pub fn build(&self) -> Result<AdvancedConfig, String> {
        if self.key == FN_KEY {
            // Firmware behaviour of a function on the Fn key is unknown.
            return Err(t::V_FN_KEY.into());
        }
        match self.func {
            Func::Dks => self.build_dks(),
            Func::Mt => {
                let f = &self.mt;
                if f.hold == 0 || f.tap == 0 {
                    return Err(t::V_MT_KEYS.into());
                }
                Ok(AdvancedConfig::Mt(Mt {
                    hold: f.hold,
                    tap: f.tap,
                    delay_ms: round_delay(f.delay_ms)?,
                }))
            }
            Func::Tgl => {
                let f = &self.tgl;
                if f.code == 0 {
                    return Err(t::V_TGL_KEY.into());
                }
                Ok(AdvancedConfig::Tgl(Tgl {
                    code: f.code,
                    delay_ms: round_delay(f.delay_ms)?,
                }))
            }
            Func::Mpt => {
                let f = &self.mpt;
                if f.codes.iter().all(|&c| c == 0) {
                    // Nothing at any depth: the key would send nothing.
                    return Err(t::V_MPT_KEYS.into());
                }
                if f.depths
                    .iter()
                    .any(|d| !(MIN_DEPTH_UM..=MAX_DEPTH_UM).contains(d))
                {
                    return Err(t::V_DEPTH_RANGE.into());
                }
                if !(f.depths[0] < f.depths[1] && f.depths[1] < f.depths[2]) {
                    return Err(t::V_DEPTH_ORDER.into());
                }
                Ok(AdvancedConfig::Mpt(Mpt {
                    codes: f.codes,
                    depths: f.depths.map(Micrometers),
                }))
            }
            Func::End => {
                if self.end.code == 0 {
                    return Err(t::V_END_KEY.into());
                }
                Ok(AdvancedConfig::End(End {
                    code: self.end.code,
                }))
            }
            Func::Socd => self.build_socd(),
        }
    }

    fn build_dks(&self) -> Result<AdvancedConfig, String> {
        let f = &self.dks;
        if f.codes.iter().all(|&c| c == 0) {
            return Err(t::V_DKS_KEY.into());
        }
        if !f.flags.iter().flatten().any(|&on| on) {
            return Err(t::V_DKS_ACTION.into());
        }
        if !(MIN_DEPTH_UM..=MAX_DEPTH_UM).contains(&f.first) {
            return Err(t::V_FIRST.into());
        }
        Ok(AdvancedConfig::Dks(Dks {
            bindings: f.codes,
            trps: f.flags.map(trps_encode),
            first_actuation: Micrometers(f.first),
            bottom: Micrometers(MAX_DEPTH_UM),
        }))
    }

    fn build_socd(&self) -> Result<AdvancedConfig, String> {
        let f = &self.socd;
        let Some(partner) = f.partner else {
            return Err(t::V_SOCD_PARTNER.into());
        };
        if partner == self.key {
            return Err(t::V_SOCD_SAME.into());
        }
        if partner == FN_KEY {
            return Err(t::V_SOCD_FN.into());
        }
        if f.delay > MAX_SOCD_DELAY {
            return Err(t::V_SOCD_DELAY.into());
        }
        let (pos1, pos2) = if f.key_first {
            (self.key, partner)
        } else {
            (partner, self.key)
        };
        let (key1, key2) = if !f.bind_values {
            // Positions: the pair's own default codes.
            (u16::from(pos1), u16::from(pos2))
        } else if f.out_key == 0 || f.out_partner == 0 {
            return Err(t::V_SOCD_OUTPUT.into());
        } else if f.key_first {
            (f.out_key, f.out_partner)
        } else {
            (f.out_partner, f.out_key)
        };
        Ok(AdvancedConfig::Socd(Socd {
            pos1,
            pos2,
            key1,
            key2,
            bind_values: f.bind_values,
            mode: f.mode,
            delay: f.delay,
        }))
    }
}

/// MT/TGL delays go out in 10 ms units: rounded to the nearest 10 ms.
fn round_delay(ms: u16) -> Result<u16, String> {
    if !(10..=1000).contains(&ms) {
        return Err(t::V_DELAY.into());
    }
    Ok(((ms + 5) / 10 * 10).min(1000))
}

/// Keys whose Mode may change when `new` replaces `loaded` on `key`: the key
/// itself and both SOCD pairs (the old partner is cleared too).
///
/// When `new` is a SOCD pair, the partner the *other* key of the pair had
/// before is cleared by the keyboard as well (`set_advanced`), and this
/// page may not know it, so every key in SOCD mode now (`in_socd`, from
/// the snapshot) is re-read too.
pub fn affected_keys(
    key: u8,
    loaded: &AdvancedConfig,
    new: &AdvancedConfig,
    in_socd: &[u8],
) -> Vec<u8> {
    let mut keys = BTreeSet::from([key]);
    for c in [loaded, new] {
        if let AdvancedConfig::Socd(s) = c {
            keys.insert(s.pos1);
            keys.insert(s.pos2);
        }
    }
    if matches!(new, AdvancedConfig::Socd(_)) {
        keys.extend(in_socd.iter().copied());
    }
    keys.into_iter().collect()
}

// -------------------------------------------------------------- behaviour

fn key_name(code: u8) -> &'static str {
    keycodes::legend(code)
}

/// The Indonesian label of a SOCD mode, naming the two keys.
pub fn socd_mode_label(mode: SocdMode, key1: &str, key2: &str) -> String {
    match mode {
        SocdMode::LastInput => t::SOCD_LAST.to_string(),
        SocdMode::Key1Wins => format!("{key1} {}", t::SOCD_WINS),
        SocdMode::Key2Wins => format!("{key2} {}", t::SOCD_WINS),
        SocdMode::Neutral => t::SOCD_NEUTRAL.to_string(),
    }
}

/// One line for the table's "Perilaku" column.
pub fn describe(cfg: &AdvancedConfig) -> String {
    match cfg {
        AdvancedConfig::Dks(d) => {
            let names: Vec<String> = d
                .bindings
                .iter()
                .filter(|&&c| c != 0)
                .map(|&c| code_name(c))
                .collect();
            format!(
                "{} · {} {} {}",
                names.join(" + "),
                t::D_ACTUATION,
                mm_short(d.first_actuation.0),
                t::D_MM
            )
        }
        AdvancedConfig::Mt(m) => format!(
            "{} = {} · {} {} {} = {}",
            t::D_TAP,
            code_name(m.tap),
            t::D_HOLD,
            m.delay_ms,
            t::D_MS,
            code_name(m.hold)
        ),
        AdvancedConfig::Tgl(g) => format!(
            "{} = {} {} · {}",
            t::D_TGL_ONCE,
            code_name(g.code),
            t::D_TGL_KEPT,
            t::D_TGL_AGAIN
        ),
        AdvancedConfig::Mpt(m) => (0..3)
            .map(|i| {
                format!(
                    "{} {} = {}",
                    mm_short(m.depths[i].0),
                    t::D_MM,
                    code_name(m.codes[i])
                )
            })
            .collect::<Vec<_>>()
            .join(" · "),
        AdvancedConfig::Socd(s) => {
            let (a, b) = (key_name(s.pos1), key_name(s.pos2));
            format!("{a} ↔ {b} · {}", socd_mode_label(s.mode, a, b))
        }
        AdvancedConfig::End(e) => format!("{} = {}", t::D_RELEASE, code_name(e.code)),
        AdvancedConfig::Macro => t::MACRO_NAME.to_string(),
        AdvancedConfig::None => String::new(),
    }
}

/// The single key a selection stands for: the one new compared with `prev`
/// (a click adds exactly one), else the smallest code.
pub fn single_selection(prev: Option<u8>, sel: &BTreeSet<u8>) -> Option<u8> {
    if sel.len() <= 1 {
        return sel.iter().next().copied();
    }
    sel.iter()
        .find(|&&k| Some(k) != prev)
        .or_else(|| sel.iter().next())
        .copied()
}

/// While the user picks a SOCD partner, a click on another key chooses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartnerPick {
    Partner(u8),
    /// Nothing usable was clicked; keep waiting.
    Keep,
}

pub fn resolve_partner_pick(main: u8, sel: &BTreeSet<u8>) -> PartnerPick {
    match sel.iter().find(|&&k| k != main && k != FN_KEY) {
        Some(&k) => PartnerPick::Partner(k),
        None => PartnerPick::Keep,
    }
}

/// A warning when the chosen SOCD partner already has another function
/// (writing the pair replaces it). The partner of the same existing pair is
/// not a conflict.
pub fn partner_conflict(
    snap: &DeviceSnapshot,
    partner: u8,
    loaded: &AdvancedConfig,
) -> Option<String> {
    if matches!(loaded, AdvancedConfig::Socd(s) if s.pos1 == partner || s.pos2 == partner) {
        return None;
    }
    let adv = snap.keys.get(&partner)?.mode.advanced;
    let name = match adv {
        Advanced::None => return None,
        Advanced::Macro => t::MACRO_NAME,
        other => Func::of_nibble(other)?.label(),
    };
    Some(format!(
        "{} {name} {} {} {}",
        t::WARN_FUNC,
        t::WARN_ON_KEY,
        key_name(partner),
        t::WARN_REMOVED
    ))
}

// ------------------------------------------------------------- table rows

/// A row of the function table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: u8,
    pub func: Func,
    pub cfg: Option<AdvancedConfig>,
}

/// Rows in matrix order for every key with a function (optionally only one
/// `filter`). A SOCD pair is listed once, at its first key.
pub fn rows(
    snap: &DeviceSnapshot,
    cache: &BTreeMap<u8, AdvancedConfig>,
    filter: Option<Func>,
) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for (_, _, key) in snap.matrix.keys() {
        let Some(func) = snap
            .keys
            .get(&key)
            .and_then(|s| Func::of_nibble(s.mode.advanced))
        else {
            continue;
        };
        if filter.is_some_and(|f| f != func) {
            continue;
        }
        let cfg = cache.get(&key).copied();
        if let Some(AdvancedConfig::Socd(s)) = &cfg {
            // The pair is listed at its earlier key in matrix order.
            if out
                .iter()
                .any(|r| matches!(r.cfg, Some(AdvancedConfig::Socd(o)) if o.pos1 == s.pos1 && o.pos2 == s.pos2))
            {
                continue;
            }
        }
        out.push(Row { key, func, cfg });
    }
    out
}

/// Which read job to start next, if any: the selected key first (alone, so
/// the editor appears quickly), then every function key not cached yet.
/// Returns `(keys, key_for_the_editor)`.
pub fn plan_read(
    snap: &DeviceSnapshot,
    cache: &BTreeMap<u8, AdvancedConfig>,
    failed: &BTreeSet<u8>,
    selected: Option<u8>,
    editor_loaded: bool,
) -> Option<(Vec<u8>, Option<u8>)> {
    let has_func = |k: u8| {
        snap.keys
            .get(&k)
            .is_some_and(|s| Func::of_nibble(s.mode.advanced).is_some())
    };
    if let Some(k) = selected {
        if !editor_loaded && has_func(k) && !failed.contains(&k) {
            return Some((vec![k], Some(k)));
        }
    }
    let stale: Vec<u8> = snap
        .matrix
        .keys()
        .map(|(_, _, c)| c)
        .filter(|&k| has_func(k) && !cache.contains_key(&k) && !failed.contains(&k))
        .collect();
    (!stale.is_empty()).then_some((stale, None))
}

// ----------------------------------------------------------------- jobs

/// What a read job gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOutcome {
    pub configs: Vec<(u8, AdvancedConfig)>,
    /// Keys not read because reading stopped on `error`.
    pub missing: Vec<u8>,
    pub error: Option<Error>,
}

/// Reads the advanced config of each key (`Keyboard::advanced`). A
/// disconnect is `Err(Disconnected)`; another error stops the read.
pub fn read_job<T: Transport>(kb: &mut Keyboard<T>, keys: &[u8]) -> Result<ReadOutcome, Error> {
    let mut configs = Vec::with_capacity(keys.len());
    for (i, &k) in keys.iter().enumerate() {
        match kb.advanced(k) {
            Ok(c) => configs.push((k, c)),
            Err(Error::Disconnected) => return Err(Error::Disconnected),
            Err(e) => {
                return Ok(ReadOutcome {
                    configs,
                    missing: keys[i..].to_vec(),
                    error: Some(e),
                })
            }
        }
    }
    Ok(ReadOutcome {
        configs,
        missing: Vec::new(),
        error: None,
    })
}

/// What the apply job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub write: Result<(), Error>,
    /// The edited key's config as re-read from the keyboard.
    pub config: Option<AdvancedConfig>,
    /// Settings re-read for the affected keys, for the snapshot's badges.
    pub settings: Vec<(u8, KeySettings)>,
    pub reload_error: Option<Error>,
}

/// The write job: `set_advanced`, then **always** (also after a failed
/// write) a re-read of the key's config and of the settings of `reread`
/// (the key and its SOCD partners). A disconnect at any point is returned as
/// `Err(Disconnected)` so the worker drops the keyboard.
pub fn apply_job<T: Transport>(
    kb: &mut Keyboard<T>,
    key: u8,
    cfg: &AdvancedConfig,
    reread: &[u8],
) -> Result<ApplyOutcome, Error> {
    let write = kb.set_advanced(key, cfg);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let mut reload_error = None;
    let config = match kb.advanced(key) {
        Ok(c) => Some(c),
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(e) => {
            reload_error = Some(e);
            None
        }
    };
    let mut settings = Vec::with_capacity(reread.len());
    if reload_error.is_none() {
        for &k in reread {
            match kb.key_settings(k) {
                Ok(s) => settings.push((k, s)),
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
        config,
        settings,
        reload_error,
    })
}

/// What the page does with a finished apply job (pure, tested).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyEffects {
    pub patch: Vec<(u8, KeySettings)>,
    /// The key's config as the keyboard has it now.
    pub config: Option<AdvancedConfig>,
    pub request_reload: bool,
    pub error: Option<String>,
    pub technical: Option<String>,
    pub applied: bool,
}

pub fn apply_effects(result: Result<ApplyOutcome, Error>) -> ApplyEffects {
    match result {
        Ok(out) => {
            let reread = out.config.is_some();
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => {
                    let mut msg = i18n::device_error(e);
                    if reread {
                        msg = format!("{msg} {}", t::RELOADED_HINT);
                    }
                    (Some(msg), Some(i18n::technical(e)))
                }
            };
            ApplyEffects {
                applied: out.write.is_ok(),
                patch: out.settings,
                config: out.config,
                request_reload: out.reload_error.is_some(),
                error,
                technical,
            }
        }
        Err(Error::Disconnected) => ApplyEffects {
            patch: Vec::new(),
            config: None,
            request_reload: false,
            error: Some(t::PARTIAL_WRITE.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            applied: false,
        },
        Err(e) => ApplyEffects {
            patch: Vec::new(),
            config: None,
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            applied: false,
        },
    }
}

// ------------------------------------------------------------ decoration

/// The badge on a key is a dot in the function's colour; the function's name
/// goes in the key's second line. (A text badge would cover the legend of a
/// 1u key.)
const DOT: &str = "●";

/// Function name and colour for a key's function nibble.
pub fn badge(adv: Advanced) -> Option<(&'static str, Color32)> {
    match adv {
        Advanced::Macro => Some(("MAC", color::INFO)),
        other => Func::of_nibble(other).map(|f| (f.label(), f.color())),
    }
}

/// Per-key decoration: a badge, tint and outline in the function's colour.
/// With a `filter` only keys of that function are marked.
pub fn decorate(
    snap: Option<&DeviceSnapshot>,
    code: u8,
    filter: Option<Func>,
    partner: Option<u8>,
) -> KeyDecoration {
    let mut deco = KeyDecoration::default();
    if partner == Some(code) {
        deco.stroke = Some(Func::Socd.color());
        deco.label2 = Some(Func::Socd.label().into());
        deco.badge = Some((DOT.to_string(), Func::Socd.color()));
        return deco;
    }
    let Some(s) = snap.and_then(|s| s.keys.get(&code)) else {
        return deco;
    };
    let Some((text, c)) = badge(s.mode.advanced) else {
        return deco;
    };
    if let Some(f) = filter {
        if Func::of_nibble(s.mode.advanced) != Some(f) {
            return deco;
        }
    }
    deco.fill = Some(c.gamma_multiply(0.12));
    deco.stroke = Some(c.gamma_multiply(0.8));
    deco.label2 = Some(text.into());
    deco.badge = Some((DOT.to_string(), c));
    deco
}

// ------------------------------------------------------------------ page

struct ReadReq {
    /// Worker generation when submitted.
    generation: u64,
    for_key: Option<u8>,
    pending: Pending<ReadOutcome>,
}

#[derive(Default)]
pub struct Page {
    selection: Selection,
    key: Option<u8>,
    editor: Option<Editor>,
    filter: Option<Func>,
    /// Config of each function key, as last read.
    cache: BTreeMap<u8, AdvancedConfig>,
    read: Option<ReadReq>,
    read_failed: BTreeSet<u8>,
    read_error: Option<(String, String)>,
    pending: Option<Pending<ApplyOutcome>>,
    /// Profile and connection count when the pending apply was submitted.
    sent_on: Option<SentOn>,
    last_sent: Option<(u8, AdvancedConfig, Vec<u8>)>,
    error: Option<String>,
    error_technical: Option<String>,
    applied: bool,
    picking_partner: bool,
    profile: Option<u8>,
    /// An apply patched the snapshot this frame: skip the staleness checks
    /// once, the new snapshot arrives next frame.
    settle: bool,
    /// Worker generation the cache was last checked against.
    seen_generation: Option<u64>,
}

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.poll(ctx);
        self.sync(ctx);
        page_with_right_panel(
            ui,
            "advanced-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    // --------------------------------------------------------------- state

    fn poll(&mut self, ctx: &AppCtx) {
        if let Some(r) = self.read.as_mut() {
            if let Some(result) = r.pending.poll() {
                let (generation, for_key) = (r.generation, r.for_key);
                self.read = None;
                // A result from before a reload is dropped; the keys are
                // planned again because they are still not cached.
                if generation == ctx.worker.generation() {
                    self.finish_read(for_key, result);
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

    fn finish_read(&mut self, for_key: Option<u8>, result: Result<ReadOutcome, Error>) {
        match result {
            Ok(out) => {
                for (k, c) in &out.configs {
                    self.cache.insert(*k, *c);
                }
                if for_key.is_some() && for_key == self.key && self.editor.is_none() {
                    if let Some((k, c)) = out.configs.iter().find(|(k, _)| Some(*k) == for_key) {
                        self.editor = Some(Editor::new(*k, c));
                    }
                }
                match out.error {
                    None => self.read_error = None,
                    Some(e) => {
                        self.read_failed.extend(out.missing);
                        self.read_error = Some((i18n::device_error(&e), i18n::technical(&e)));
                    }
                }
            }
            Err(e) => {
                // Disconnected: the worker reconnects and reloads.
                self.read_error = Some((i18n::device_error(&e), i18n::technical(&e)));
                if let Some(k) = for_key {
                    self.read_failed.insert(k);
                }
            }
        }
    }

    fn finish_apply(&mut self, ctx: &AppCtx, result: Result<ApplyOutcome, Error>) {
        let fx = apply_effects(result);
        let sent_on = self.sent_on.take();
        if sent_on.is_none() || sent_on != SentOn::now(ctx) {
            // Read only after a profile switch or a reconnect: the snapshot
            // and the cache were loaded afresh since, so nothing read back
            // then is applied to them.
            self.error = fx.error;
            self.error_technical = fx.technical;
            self.applied = false;
            return;
        }
        let profile = sent_on.map(|s| s.profile);
        let sent = self.last_sent.clone();
        if let Some((key, _, affected)) = sent {
            // Cached configs of every affected key are stale now.
            for k in &affected {
                self.cache.remove(k);
                self.read_failed.remove(k);
            }
            match fx.config {
                Some(c) => {
                    self.cache.insert(key, c);
                    if let (true, AdvancedConfig::Socd(s)) = (fx.applied, c) {
                        self.cache.insert(s.pos1, c);
                        self.cache.insert(s.pos2, c);
                    }
                    // The user may have selected another key meanwhile:
                    // only the applied key's own editor is reloaded.
                    if self.key == Some(key) {
                        if fx.applied {
                            self.editor = Some(Editor::new(key, &c));
                        } else if let Some(e) = self.editor.as_mut().filter(|e| e.key == key) {
                            e.loaded = c;
                        }
                    }
                }
                // The keyboard's state is unknown: load the editor again.
                None if self.key == Some(key) => self.editor = None,
                None => {}
            }
        }
        if !fx.patch.is_empty() {
            let fresh = fx.patch;
            ctx.worker.patch_snapshot(move |s| {
                if Some(s.profile) != profile {
                    return;
                }
                for (k, v) in fresh {
                    s.keys.insert(k, v);
                }
            });
            // This frame's snapshot is from before the patch.
            self.settle = true;
        }
        if fx.request_reload {
            ctx.worker.request_reload();
        }
        self.error = fx.error;
        self.error_technical = fx.technical;
        self.applied = fx.applied;
    }

    /// Drops everything read from the keyboard when the worker's
    /// generation moved for any reason other than this page's own apply
    /// (`settle`): a reload, an import, a keyboard event or another page's
    /// write may have changed function parameters without changing the
    /// Mode nibble, and the editor must never send stale parameters back.
    /// The selection stays; the selected key is read again.
    fn follow_generation(&mut self, generation: u64, settle: bool) {
        let moved = self.seen_generation.is_some_and(|g| g != generation);
        self.seen_generation = Some(generation);
        if moved && !settle {
            self.cache.clear();
            self.read_failed.clear();
            self.read_error = None;
            self.editor = None;
            self.picking_partner = false;
        }
    }

    fn sync(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else {
            self.cache.clear();
            self.read_failed.clear();
            self.editor = None;
            self.picking_partner = false;
            // `profile` is kept: a reconnect onto another profile is still
            // seen as a profile change below.
            return;
        };
        if self.profile.is_some_and(|p| p != snap.profile) {
            self.cache.clear();
            self.read_failed.clear();
            self.read_error = None;
            self.editor = None;
            // "Coba lagi" must not resend the old profile's write here.
            self.last_sent = None;
            self.error = None;
            self.error_technical = None;
            self.applied = false;
        }
        self.profile = Some(snap.profile);
        self.selection.retain_present(&snap.matrix);

        let key = single_selection(self.key, self.selection.as_set());
        if key != self.key {
            self.key = key;
            self.editor = None;
            self.error = None;
            self.error_technical = None;
            self.applied = false;
            self.picking_partner = false;
            self.read_error = None;
            if let Some(k) = key {
                self.read_failed.remove(&k);
            }
        }
        if let Some(k) = self.key {
            if self.selection.len() != 1 || !self.selection.contains(k) {
                self.selection.set([k]);
            }
        }

        // Drop what no longer matches the keyboard.
        let settle = std::mem::take(&mut self.settle);
        self.follow_generation(ctx.worker.generation(), settle);
        self.cache.retain(|k, c| {
            settle
                || snap
                    .keys
                    .get(k)
                    .is_some_and(|s| s.mode.advanced == c.advanced())
        });
        let nibble = |k: u8| snap.keys.get(&k).map(|s| s.mode.advanced);
        // Never edit one key with another key's form.
        if self
            .editor
            .as_ref()
            .is_some_and(|e| Some(e.key) != self.key)
        {
            self.editor = None;
            self.picking_partner = false;
        }
        if let (Some(k), Some(e), false) = (self.key, self.editor.as_ref(), settle) {
            if nibble(k).is_some_and(|n| n != e.loaded.advanced()) {
                self.editor = None;
            }
        }
        // Keys without a function (or with a macro) need no read.
        if let Some(k) = self.key {
            if self.editor.is_none() {
                match nibble(k) {
                    Some(Advanced::None) => {
                        self.editor = Some(Editor::new(k, &AdvancedConfig::None))
                    }
                    Some(Advanced::Macro) => {
                        self.editor = Some(Editor::new(k, &AdvancedConfig::Macro))
                    }
                    _ => {}
                }
            }
        }

        if self.read.is_none() && self.pending.is_none() && ctx.can_write() {
            if let Some((keys, for_key)) = plan_read(
                snap,
                &self.cache,
                &self.read_failed,
                self.key,
                self.editor.is_some(),
            ) {
                self.read = Some(ReadReq {
                    generation: ctx.worker.generation(),
                    for_key,
                    pending: ctx
                        .worker
                        .submit("advanced-read", move |kb| read_job(kb, &keys)),
                });
            }
        }
    }

    fn editable(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && self.pending.is_none()
    }

    fn submit(&mut self, ctx: &AppCtx, key: u8, cfg: AdvancedConfig) {
        let loaded = self
            .editor
            .as_ref()
            .map_or(AdvancedConfig::None, |e| e.loaded);
        let in_socd: Vec<u8> = ctx.snapshot.map_or_else(Vec::new, |s| {
            s.keys
                .iter()
                .filter(|(_, v)| v.mode.advanced == Advanced::Socd)
                .map(|(&k, _)| k)
                .collect()
        });
        let affected = affected_keys(key, &loaded, &cfg, &in_socd);
        self.error = None;
        self.error_technical = None;
        self.applied = false;
        self.last_sent = Some((key, cfg, affected.clone()));
        self.sent_on = SentOn::now(ctx);
        self.pending = Some(ctx.worker.submit("advanced-apply", move |kb| {
            apply_job(kb, key, &cfg, &affected)
        }));
    }

    // -------------------------------------------------------------- centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let before = self.selection.clone();
        self.center_body(ctx, ui);
        if self.selection != before {
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

        // Filter chips.
        let total = ctx.snapshot.map_or(0, |s| rows(s, &self.cache, None).len());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            let all = format!("{} · {total}", t::ALL);
            if filter_chip(ui, &all, self.filter.is_none(), None, connected).clicked() {
                self.filter = None;
            }
            for f in FUNCS {
                let on = self.filter == Some(f);
                if filter_chip(ui, f.label(), on, Some(f.color()), connected).clicked() {
                    self.filter = if on { None } else { Some(f) };
                }
            }
        });
        ui.add_space(12.0);

        let snap = ctx.snapshot;
        let filter = self.filter;
        let picking = self.picking_partner;
        let main = self.key;
        let partner = self.editor.as_ref().and_then(|e| e.socd.partner);
        let partner =
            partner.filter(|_| self.editor.as_ref().is_some_and(|e| e.func == Func::Socd));
        ui.vertical_centered(|ui| {
            KeyboardView::new(model, matrix)
                .enabled(connected)
                .id_salt("advanced-keyboard")
                .show(ui, &mut self.selection, |code| {
                    decorate(snap, code, filter, partner)
                });
        });
        if picking {
            if let Some(m) = main {
                match resolve_partner_pick(m, self.selection.as_set()) {
                    PartnerPick::Partner(p) => {
                        if let Some(e) = self.editor.as_mut() {
                            e.socd.partner = Some(p);
                        }
                        self.picking_partner = false;
                    }
                    PartnerPick::Keep => {}
                }
                // The edited key stays selected while picking.
                self.selection.set([m]);
            }
        }
        if !connected {
            ui.add_space(8.0);
            waiting_banner(ui, ctx);
        } else if self.picking_partner {
            ui.add_space(8.0);
            ui.label(
                RichText::new(t::SOCD_PICKING_NOT_FN)
                    .font(font::small())
                    .color(Func::Socd.color()),
            );
        }
        ui.add_space(16.0);

        self.table(ctx, ui);
    }

    fn table(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let list = ctx
            .snapshot
            .map(|s| rows(s, &self.cache, self.filter))
            .unwrap_or_default();
        let h = ui.available_height().max(140.0);
        Card::new().padding(0).show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 4.0));
            let w = ui.available_width();
            // Header.
            let (hr, _) = ui.allocate_exact_size(egui::vec2(w, 38.0), Sense::hover());
            let cols = columns(hr);
            let p = ui.painter();
            for (x, text, align) in [
                (cols.key, t::COL_KEY, Align2::LEFT_CENTER),
                (cols.func, t::COL_FUNC, Align2::LEFT_CENTER),
                (cols.desc, t::COL_BEHAVIOUR, Align2::LEFT_CENTER),
                (cols.action, t::COL_ACTION, Align2::RIGHT_CENTER),
            ] {
                p.text(
                    egui::pos2(x, hr.center().y),
                    align,
                    text,
                    font::tiny(),
                    color::FAINT,
                );
            }
            p.hline(hr.x_range(), hr.bottom(), line(1.0, color::BORDER));
            if list.is_empty() {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(t::EMPTY_TABLE)
                                .font(font::small())
                                .color(color::MUTED),
                        )
                        .wrap(),
                    );
                });
                return;
            }
            let mut clicked = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for row in &list {
                        let selected = self.key == Some(row.key);
                        if table_row(ui, row, selected, ctx.connected()) {
                            clicked = Some(row.key);
                        }
                    }
                    if self.read.as_ref().is_some_and(|r| r.for_key.is_none()) {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            ui.spinner();
                            ui.label(
                                RichText::new(t::READING_TABLE)
                                    .font(font::small())
                                    .color(color::MUTED),
                            );
                        });
                    }
                });
            if let Some(k) = clicked {
                self.selection.set([k]);
            }
        });
    }

    // --------------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("advanced-apply")
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
                    .show(ui, |ui| self.editor_ui(ctx, ui));
            });
    }

    fn editor_ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let (Some(key), true) = (self.key, ctx.connected()) else {
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
            return;
        };
        let loaded_func = self
            .editor
            .as_ref()
            .and_then(|e| Func::of_config(&e.loaded));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(key_name(key))
                    .font(font::title())
                    .color(color::TEXT)
                    .strong(),
            );
            if let Some(f) = loaded_func {
                pill(ui, f.label(), f.color());
            }
        });
        if key == FN_KEY {
            card::hint(ui, t::V_FN_KEY);
            return;
        }
        let editable = self.editable(ctx);
        let Some(editor) = self.editor.as_mut() else {
            ui.add_space(8.0);
            if let Some((msg, tech)) = self.read_error.clone() {
                let retry = editable.then_some(i18n::common::RETRY);
                if card::error_box(ui, t::READ_FAILED, &msg, retry, Some(&tech)) {
                    self.read_error = None;
                    self.read_failed.remove(&key);
                }
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(t::READING)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            }
            return;
        };
        card::hint(ui, editor.func.blurb());
        if editor.loaded == AdvancedConfig::Macro {
            card::hint(ui, t::MACRO_HINT);
        }
        ui.add_space(10.0);

        let mut picking = self.picking_partner;
        let warning = match (editor.func, editor.socd.partner, ctx.snapshot) {
            (Func::Socd, Some(p), Some(snap)) => partner_conflict(snap, p, &editor.loaded),
            _ => None,
        };
        ui.add_enabled_ui(editable, |ui| {
            match editor.func {
                Func::Dks => dks_editor(ui, &mut editor.dks),
                Func::Mt => mt_editor(ui, &mut editor.mt),
                Func::Tgl => tgl_editor(ui, &mut editor.tgl),
                Func::Mpt => mpt_editor(ui, &mut editor.mpt),
                Func::Socd => {
                    socd_editor(ui, key, &mut editor.socd, &mut picking);
                    if let Some(w) = &warning {
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(w).font(font::small()).color(color::ACCENT),
                            )
                            .wrap(),
                        );
                    }
                }
                Func::End => end_editor(ui, &mut editor.end),
            }
            ui.add_space(16.0);
            card::section(ui, t::CHANGE_TYPE);
            ui.add_space(4.0);
            type_picker(ui, &mut editor.func);
        });
        self.picking_partner = picking && editor.func == Func::Socd;
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
                if let Some((key, cfg, _)) = self.last_sent.clone() {
                    self.submit(ctx, key, cfg);
                }
            }
            ui.add_space(8.0);
        }
        let built = self.editor.as_ref().map(Editor::build);
        if let (Some(Err(msg)), false) = (&built, self.key == Some(FN_KEY)) {
            ui.add(
                egui::Label::new(RichText::new(msg).font(font::small()).color(color::DANGER))
                    .wrap(),
            );
            ui.add_space(6.0);
        }
        let key = self.key;
        ui.horizontal(|ui| {
            let can_delete = idle
                && self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.loaded != AdvancedConfig::None);
            let del = egui::Button::new(RichText::new(t::DELETE).color(color::DANGER).strong())
                .fill(color::BG)
                .stroke(line(1.0, color::BORDER_STRONG))
                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
                .min_size(egui::vec2(100.0, 46.0));
            if ui.add_enabled(can_delete, del).clicked() {
                if let Some(k) = key {
                    self.submit(ctx, k, AdvancedConfig::None);
                }
            }
            let can_apply = idle && matches!(built, Some(Ok(_)));
            if card::primary_button(ui, t::APPLY, can_apply).clicked() {
                if let (Some(k), Some(Ok(cfg))) = (key, built.clone()) {
                    self.submit(ctx, k, cfg);
                }
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
            } else if self.applied {
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

// ------------------------------------------------------------- widgets

fn filter_chip(
    ui: &mut Ui,
    text: &str,
    active: bool,
    dot: Option<Color32>,
    enabled: bool,
) -> egui::Response {
    let accent = dot.unwrap_or(color::ACCENT);
    let (fill, stroke, text_c) = if active {
        (accent.gamma_multiply(0.18), accent, accent)
    } else {
        (color::BG, color::BORDER_STRONG, color::TEXT)
    };
    // The dot in the function's own colour, the name in the text colour.
    let mut label = LayoutJob::default();
    let style = ui.style();
    let body = egui::TextStyle::Button.resolve(style);
    if let Some(c) = dot {
        label.append(
            "●  ",
            0.0,
            egui::TextFormat::simple(
                body.clone(),
                if enabled { c } else { c.gamma_multiply(0.5) },
            ),
        );
    }
    label.append(text, 0.0, egui::TextFormat::simple(body, text_c));
    ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .fill(fill)
            .stroke(line(1.0, stroke))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .min_size(egui::vec2(0.0, 38.0)),
    )
}

fn pill(ui: &mut Ui, text: &str, c: Color32) {
    Frame::new()
        .fill(c.gamma_multiply(0.18))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(font::small()).color(c).strong());
        });
}

struct Columns {
    key: f32,
    func: f32,
    desc: f32,
    action: f32,
}

fn columns(r: Rect) -> Columns {
    Columns {
        key: r.left() + 20.0,
        func: r.left() + 150.0,
        desc: r.left() + 250.0,
        action: r.right() - 20.0,
    }
}

/// One table row; returns true when clicked.
fn table_row(ui: &mut Ui, row: &Row, selected: bool, enabled: bool) -> bool {
    let w = ui.available_width();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 46.0), sense);
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, color::ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, color::KEY_HOVER.gamma_multiply(0.5));
    }
    p.hline(rect.x_range(), rect.bottom(), line(1.0, color::BORDER));
    let cols = columns(rect);
    let cy = rect.center().y;

    let name = match row.cfg {
        Some(AdvancedConfig::Socd(s)) => format!("{} ↔ {}", key_name(s.pos1), key_name(s.pos2)),
        _ => key_name(row.key).to_string(),
    };
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            enabled,
            selected,
            format!("{name} · {}", row.func.label()),
        )
    });
    p.text(
        egui::pos2(cols.key, cy),
        Align2::LEFT_CENTER,
        name,
        font::bold(14.5),
        color::TEXT,
    );

    let c = row.func.color();
    let galley = p.layout_no_wrap(row.func.label().to_string(), font::bold(12.0), c);
    let pr = Rect::from_min_size(
        egui::pos2(cols.func, cy - 11.0),
        galley.size() + egui::vec2(16.0, 6.0),
    );
    p.rect_filled(pr, CornerRadius::same(6), c.gamma_multiply(0.18));
    p.galley(pr.min + egui::vec2(8.0, 3.0), galley, c);

    let action = if selected { t::EDITING } else { t::EDIT };
    let action_rect = p.text(
        egui::pos2(cols.action, cy),
        Align2::RIGHT_CENTER,
        action,
        font::body(),
        if selected {
            color::ACCENT
        } else {
            color::MUTED
        },
    );
    let text = match &row.cfg {
        Some(cfg) => describe(cfg),
        None => t::UNKNOWN_BEHAVIOUR.to_string(),
    };
    let mut job = LayoutJob::simple(
        text,
        font::body(),
        color::TEXT,
        (action_rect.left() - cols.desc - 16.0).max(40.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = p.layout_job(job);
    let pos = egui::pos2(cols.desc, cy - galley.size().y / 2.0);
    p.galley(pos, galley, color::TEXT);
    resp.clicked()
}

/// Two rows of three function buttons ("Ganti jenis fungsi").
fn type_picker(ui: &mut Ui, func: &mut Func) {
    for chunk in FUNCS.chunks(3) {
        let opts: Vec<(Func, &str)> = chunk.iter().map(|&f| (f, f.label())).collect();
        Segmented::new(&opts)
            .full_width(true)
            .height(40.0)
            .show(ui, func);
        ui.add_space(4.0);
    }
}

fn labelled_code(ui: &mut Ui, label: &str, id: impl std::hash::Hash, code: &mut u16) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(color::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            code_field(ui, id, code, true, 130.0);
        });
    });
    ui.add_space(4.0);
}

/// A slider in the page's colours (egui's default rail is nearly invisible
/// on the dark panel).
fn themed_slider(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.widgets.inactive.bg_fill = color::BORDER_STRONG;
        v.widgets.hovered.bg_fill = color::BORDER_STRONG;
        v.widgets.active.bg_fill = color::BORDER_STRONG;
        v.selection.bg_fill = color::ACCENT;
        ui.spacing_mut().slider_width = (ui.available_width() - 84.0).max(80.0);
        add(ui);
    });
}

fn ms_slider(ui: &mut Ui, label: &str, ms: &mut u16) {
    ui.label(RichText::new(label).color(color::TEXT));
    themed_slider(ui, |ui| {
        ui.add(
            egui::Slider::new(ms, 10..=1000)
                .step_by(10.0)
                .trailing_fill(true)
                .suffix(" ms"),
        );
    });
    ui.add_space(4.0);
}

fn mt_editor(ui: &mut Ui, f: &mut MtForm) {
    labelled_code(ui, t::HOLD_KEY, "mt-hold", &mut f.hold);
    labelled_code(ui, t::TAP_KEY, "mt-tap", &mut f.tap);
    ms_slider(ui, t::HOLD_DELAY, &mut f.delay_ms);
}

fn tgl_editor(ui: &mut Ui, f: &mut TglForm) {
    labelled_code(ui, t::TGL_KEY, "tgl-key", &mut f.code);
    ms_slider(ui, t::TGL_DELAY, &mut f.delay_ms);
}

fn end_editor(ui: &mut Ui, f: &mut EndForm) {
    labelled_code(ui, t::END_KEY, "end-key", &mut f.code);
}

fn mpt_editor(ui: &mut Ui, f: &mut MptForm) {
    for i in 0..3 {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} {}", t::MPT_POINT, i + 1))
                    .color(Func::Mpt.color())
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                code_field(ui, ("mpt", i), &mut f.codes[i], true, 130.0);
                mm_value_box(ui, &mut f.depths[i], MIN_DEPTH_UM..=MAX_DEPTH_UM, true);
            });
        });
        SliderMm::new(&mut f.depths[i], MIN_DEPTH_UM..=MAX_DEPTH_UM).show(ui);
        ui.add_space(6.0);
    }
}

fn socd_editor(ui: &mut Ui, key: u8, f: &mut SocdForm, picking: &mut bool) {
    card::section(ui, t::SOCD_PAIR);
    ui.horizontal(|ui| {
        let partner = f
            .partner
            .map_or(t::SOCD_NO_PARTNER.to_string(), |p| key_name(p).to_string());
        ui.label(
            RichText::new(format!("{} ↔ {partner}", key_name(key)))
                .font(font::heading())
                .color(color::TEXT),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let c = Func::Socd.color();
            let btn = egui::Button::new(RichText::new(t::SOCD_PICK).color(if *picking {
                c
            } else {
                color::TEXT
            }))
            .fill(if *picking {
                c.gamma_multiply(0.18)
            } else {
                color::BG
            })
            .stroke(line(1.0, if *picking { c } else { color::BORDER_STRONG }))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS));
            if ui.add(btn).clicked() {
                *picking = !*picking;
            }
        });
    });
    ui.add_space(10.0);

    card::section(ui, t::SOCD_MODE);
    let (k1, k2) = if f.key_first {
        (key_name(key), f.partner.map_or("?", key_name))
    } else {
        (f.partner.map_or("?", key_name), key_name(key))
    };
    for mode in [
        SocdMode::LastInput,
        SocdMode::Key1Wins,
        SocdMode::Key2Wins,
        SocdMode::Neutral,
    ] {
        let label = socd_mode_label(mode, k1, k2);
        let on = f.mode == mode;
        let c = Func::Socd.color();
        let btn = egui::Button::new(RichText::new(label).color(if on { c } else { color::TEXT }))
            .fill(if on {
                c.gamma_multiply(0.14)
            } else {
                color::BG
            })
            .stroke(line(1.0, if on { c } else { color::BORDER_STRONG }))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
            .min_size(egui::vec2(ui.available_width(), 34.0));
        if ui.add(btn).clicked() {
            f.mode = mode;
        }
    }
    ui.add_space(10.0);

    card::section(ui, t::SOCD_OUTPUT);
    let outputs = [(false, t::SOCD_POSITIONS), (true, t::SOCD_CODES)];
    Segmented::new(&outputs)
        .full_width(true)
        .height(38.0)
        .show(ui, &mut f.bind_values);
    if f.bind_values {
        ui.add_space(6.0);
        let partner_name = f.partner.map_or("?", key_name);
        labelled_code(
            ui,
            &format!("{} {}", t::SOCD_OUT_FOR, key_name(key)),
            "socd-out-key",
            &mut f.out_key,
        );
        labelled_code(
            ui,
            &format!("{} {partner_name}", t::SOCD_OUT_FOR),
            "socd-out-partner",
            &mut f.out_partner,
        );
    }
    ui.add_space(10.0);
    ui.label(RichText::new(t::SOCD_DELAY).color(color::TEXT));
    themed_slider(ui, |ui| {
        ui.add(egui::Slider::new(&mut f.delay, 0..=MAX_SOCD_DELAY).trailing_fill(true));
    });
}

/// The DKS grid: per binding a key button and a track with four phase nodes
/// (press events) joined by three hold links.
fn dks_editor(ui: &mut Ui, f: &mut DksForm) {
    let c = Func::Dks.color();
    let avail = ui.available_width();
    let code_w = 86.0;
    let track_w = avail - code_w - 8.0;
    ui.horizontal(|ui| {
        ui.add_space(code_w + 8.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(track_w, 30.0), Sense::hover());
        let node_x = |i: usize| r.left() + 9.0 + (r.width() - 18.0) * i as f32 / 3.0;
        for (i, label) in t::DKS_PHASES.iter().enumerate() {
            ui.painter().text(
                egui::pos2(node_x(i), r.center().y),
                Align2::CENTER_CENTER,
                label,
                font::tiny(),
                color::MUTED,
            );
        }
    });
    for row in 0..4 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            code_field(ui, ("dks", row), &mut f.codes[row], true, code_w);
            dks_track(ui, row, &mut f.flags[row], track_w, c);
        });
        ui.add_space(2.0);
    }
    card::hint(ui, t::DKS_LEGEND);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(t::DKS_FIRST).color(color::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            mm_value_box(ui, &mut f.first, MIN_DEPTH_UM..=MAX_DEPTH_UM, true);
        });
    });
    SliderMm::new(&mut f.first, MIN_DEPTH_UM..=MAX_DEPTH_UM)
        .end_labels(mm_short(MIN_DEPTH_UM), mm_short(MAX_DEPTH_UM))
        .show(ui);
    card::hint(ui, t::DKS_BOTTOM);
}

/// Seven flags: nodes at even indexes (press at P0..P3), links at odd ones
/// (hold between two phases).
fn dks_track(ui: &mut Ui, row: usize, flags: &mut [bool; 7], width: f32, c: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 34.0), Sense::hover());
    let r = 9.0;
    let cy = rect.center().y;
    let x = |i: usize| rect.left() + r + (rect.width() - 2.0 * r) * i as f32 / 3.0;
    let base = egui::Id::new(("dks-track", row));
    let enabled = ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    for i in 0..3 {
        let link = Rect::from_min_max(
            egui::pos2(x(i) + r, cy - 12.0),
            egui::pos2(x(i + 1) - r, cy + 12.0),
        );
        let resp = ui.interact(link, base.with(("link", i)), sense);
        if resp.clicked() {
            flags[2 * i + 1] = !flags[2 * i + 1];
        }
        let on = flags[2 * i + 1];
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                enabled,
                on,
                format!(
                    "DKS {} · {} {}–{}",
                    row + 1,
                    t::D_HOLD,
                    t::DKS_PHASES[i].replace('\n', " "),
                    t::DKS_PHASES[i + 1].replace('\n', " ")
                ),
            )
        });
        let on = flags[2 * i + 1];
        let bar = Rect::from_center_size(
            link.center(),
            egui::vec2(link.width(), if on { 6.0 } else { 2.0 }),
        );
        let fill = if on {
            c
        } else if resp.hovered() {
            color::MUTED
        } else {
            color::BORDER_STRONG
        };
        ui.painter().rect_filled(bar, CornerRadius::same(3), fill);
    }
    for i in 0..4 {
        let node = Rect::from_center_size(
            egui::pos2(x(i), cy),
            egui::vec2(2.0 * r + 4.0, 2.0 * r + 4.0),
        );
        let resp = ui.interact(node, base.with(("node", i)), sense);
        if resp.clicked() {
            flags[2 * i] = !flags[2 * i];
        }
        let on = flags[2 * i];
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                enabled,
                on,
                format!("DKS {} · {}", row + 1, t::DKS_PHASES[i].replace('\n', " ")),
            )
        });
        let p = ui.painter();
        let ctr = egui::pos2(x(i), cy);
        if flags[2 * i] {
            p.circle_filled(ctr, r, c);
        } else {
            p.circle_filled(ctr, r - 1.0, color::BG);
            p.circle_stroke(
                ctr,
                r - 1.0,
                line(1.5, if resp.hovered() { c } else { color::MUTED }),
            );
        }
    }
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
    use crate::widgets::keyboard::reference_matrix;
    use openhe_device::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::key::KeyMode;
    use openhe_proto::{DeviceInfo, PollingRate, TouchMode};

    fn um(v: u16) -> Micrometers {
        Micrometers(v)
    }

    fn ed() -> Editor {
        Editor::new(0x0A, &AdvancedConfig::None)
    }

    // ---- editor state -> config

    #[test]
    fn every_dks_phase_cell_maps_to_its_trps_bit() {
        // [press@P0, hold P0-P1, press@P1, hold P1-P2, press@P2, hold P2-P3, press@P3]
        let bits = [0x01u8, 0x02, 0x04, 0x18, 0x20, 0x40, 0x80];
        for binding in 0..4 {
            for (cell, bit) in bits.into_iter().enumerate() {
                let mut e = ed();
                e.func = Func::Dks;
                e.dks.codes = [0x04, 0x05, 0x06, 0x07];
                e.dks.flags[binding][cell] = true;
                let AdvancedConfig::Dks(d) = e.build().unwrap() else {
                    panic!("not DKS");
                };
                let mut want = [0u8; 4];
                want[binding] = bit;
                assert_eq!(d.trps, want, "binding {binding} cell {cell}");
            }
        }
    }

    #[test]
    fn dks_builds_all_fields_and_round_trips_through_the_editor() {
        let cfg = AdvancedConfig::Dks(Dks {
            bindings: [0x2C, 0x06, 0, 0],
            trps: [0x01 | 0x02 | 0x04, 0x18 | 0x20 | 0x80, 0, 0],
            first_actuation: um(1200),
            bottom: um(3400),
        });
        let e = Editor::new(0x2C, &cfg);
        assert_eq!(e.func, Func::Dks);
        assert!(e.dks.flags[0][..3].iter().all(|&b| b));
        assert!(e.dks.flags[1][3] && e.dks.flags[1][4] && e.dks.flags[1][6]);
        assert_eq!(e.build(), Ok(cfg));
    }

    #[test]
    fn dks_validation_messages() {
        let mut e = ed();
        e.func = Func::Dks;
        assert_eq!(e.build(), Err(t::V_DKS_KEY.to_string()));
        e.dks.codes[0] = 0x04;
        assert_eq!(e.build(), Err(t::V_DKS_ACTION.to_string()));
        e.dks.flags[0][0] = true;
        assert!(e.build().is_ok());
        e.dks.first = 50;
        assert_eq!(e.build(), Err(t::V_FIRST.to_string()));
        e.dks.first = 3500;
        assert_eq!(e.build(), Err(t::V_FIRST.to_string()));
        e.dks.first = 100;
        assert!(e.build().is_ok());
        e.dks.first = 3400;
        let AdvancedConfig::Dks(d) = e.build().unwrap() else {
            panic!()
        };
        assert_eq!(d.bottom, um(3400), "the bottom is fixed at 3.40 mm");
    }

    #[test]
    fn mt_maps_hold_tap_and_delay() {
        let mut e = ed();
        e.func = Func::Mt;
        assert_eq!(e.build(), Err(t::V_MT_KEYS.to_string()));
        e.mt = MtForm {
            hold: 0xE0,
            tap: 0x29,
            delay_ms: 200,
        };
        assert_eq!(
            e.build(),
            Ok(AdvancedConfig::Mt(Mt {
                hold: 0xE0,
                tap: 0x29,
                delay_ms: 200
            }))
        );
        // 10 ms steps: 205 rounds to 210; the range is 10..=1000.
        e.mt.delay_ms = 205;
        assert!(matches!(e.build(), Ok(AdvancedConfig::Mt(m)) if m.delay_ms == 210));
        for bad in [0, 9, 1001] {
            e.mt.delay_ms = bad;
            assert_eq!(e.build(), Err(t::V_DELAY.to_string()), "{bad}");
        }
        e.mt.delay_ms = 10;
        assert!(e.build().is_ok());
        e.mt.delay_ms = 1000;
        assert!(matches!(e.build(), Ok(AdvancedConfig::Mt(m)) if m.delay_ms == 1000));
    }

    #[test]
    fn tgl_and_end_map_their_key() {
        let mut e = ed();
        e.func = Func::Tgl;
        assert_eq!(e.build(), Err(t::V_TGL_KEY.to_string()));
        e.tgl = TglForm {
            code: 0x04,
            delay_ms: 300,
        };
        assert_eq!(
            e.build(),
            Ok(AdvancedConfig::Tgl(Tgl {
                code: 0x04,
                delay_ms: 300
            }))
        );
        e.tgl.delay_ms = 2000;
        assert_eq!(e.build(), Err(t::V_DELAY.to_string()));
        e.func = Func::End;
        assert_eq!(e.build(), Err(t::V_END_KEY.to_string()));
        e.end.code = 0x05;
        assert_eq!(e.build(), Ok(AdvancedConfig::End(End { code: 0x05 })));
    }

    #[test]
    fn mpt_depths_must_be_in_range_and_strictly_increasing() {
        let mut e = ed();
        e.func = Func::Mpt;
        e.mpt.codes = [0x04, 0x05, 0x06];
        assert_eq!(
            e.build(),
            Ok(AdvancedConfig::Mpt(Mpt {
                codes: [0x04, 0x05, 0x06],
                depths: [um(1000), um(2000), um(3000)]
            }))
        );
        for depths in [[2000, 2000, 3000], [3000, 2000, 1000], [1000, 3000, 3000]] {
            e.mpt.depths = depths;
            assert_eq!(e.build(), Err(t::V_DEPTH_ORDER.to_string()), "{depths:?}");
        }
        for depths in [[50, 2000, 3000], [1000, 2000, 3500]] {
            e.mpt.depths = depths;
            assert_eq!(e.build(), Err(t::V_DEPTH_RANGE.to_string()), "{depths:?}");
        }
        e.mpt.depths = [100, 101, 3400];
        assert!(e.build().is_ok());
    }

    #[test]
    fn mpt_needs_at_least_one_key() {
        let mut e = ed();
        e.func = Func::Mpt;
        e.mpt.codes = [0, 0, 0];
        assert_eq!(e.build(), Err(t::V_MPT_KEYS.to_string()));
        // One key at any depth is enough.
        e.mpt.codes = [0, 0x29, 0];
        assert!(e.build().is_ok());
    }

    #[test]
    fn socd_maps_pair_order_outputs_mode_and_delay() {
        let mut e = Editor::new(0x07, &AdvancedConfig::None); // D
        e.func = Func::Socd;
        assert_eq!(e.build(), Err(t::V_SOCD_PARTNER.to_string()));
        e.socd.partner = Some(0x07);
        assert_eq!(e.build(), Err(t::V_SOCD_SAME.to_string()));
        e.socd.partner = Some(0x04); // A
        e.socd.mode = SocdMode::Key2Wins;
        e.socd.delay = 50;
        // Positions: outputs are the pair's own default codes.
        assert_eq!(
            e.build(),
            Ok(AdvancedConfig::Socd(Socd {
                pos1: 0x07,
                pos2: 0x04,
                key1: 0x07,
                key2: 0x04,
                bind_values: false,
                mode: SocdMode::Key2Wins,
                delay: 50,
            }))
        );
        // Chosen codes: the edited key's output goes with its position.
        e.socd.bind_values = true;
        assert_eq!(e.build(), Err(t::V_SOCD_OUTPUT.to_string()));
        e.socd.out_key = 0x50;
        e.socd.out_partner = 0x4F;
        let Ok(AdvancedConfig::Socd(s)) = e.build() else {
            panic!()
        };
        assert_eq!((s.pos1, s.key1, s.pos2, s.key2), (0x07, 0x50, 0x04, 0x4F));
        // The edited key as the second of the pair.
        e.socd.key_first = false;
        let Ok(AdvancedConfig::Socd(s)) = e.build() else {
            panic!()
        };
        assert_eq!((s.pos1, s.key1, s.pos2, s.key2), (0x04, 0x4F, 0x07, 0x50));
        e.socd.delay = 51;
        assert_eq!(e.build(), Err(t::V_SOCD_DELAY.to_string()));
    }

    #[test]
    fn socd_round_trips_from_either_key_of_the_pair() {
        let cfg = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 0x50,
            key2: 0x4F,
            bind_values: true,
            mode: SocdMode::Neutral,
            delay: 7,
        });
        for key in [0x04, 0x07] {
            let e = Editor::new(key, &cfg);
            assert_eq!(e.func, Func::Socd);
            assert_eq!(e.socd.partner, Some(if key == 0x04 { 0x07 } else { 0x04 }));
            assert_eq!(e.build(), Ok(cfg), "from key 0x{key:02X}");
        }
    }

    #[test]
    fn every_function_round_trips_through_the_editor() {
        let cfgs = [
            AdvancedConfig::Mt(Mt {
                hold: 0xE0,
                tap: 0x29,
                delay_ms: 200,
            }),
            AdvancedConfig::Tgl(Tgl {
                code: 4,
                delay_ms: 300,
            }),
            AdvancedConfig::Mpt(Mpt {
                codes: [4, 5, 6],
                depths: [um(1000), um(2000), um(3000)],
            }),
            AdvancedConfig::End(End { code: 5 }),
        ];
        for c in cfgs {
            let e = Editor::new(0x0A, &c);
            assert_eq!(e.build(), Ok(c));
            assert_eq!(e.loaded, c);
        }
        let e = Editor::new(0x0A, &AdvancedConfig::None);
        assert_eq!(e.loaded, AdvancedConfig::None);
    }

    #[test]
    fn affected_keys_cover_both_socd_pairs() {
        let old = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 4,
            key2: 7,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        let new = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x16,
            key1: 4,
            key2: 0x16,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        assert_eq!(affected_keys(0x04, &old, &new, &[]), vec![0x04, 0x07, 0x16]);
        assert_eq!(
            affected_keys(0x04, &old, &AdvancedConfig::None, &[0x04, 0x07]),
            vec![0x04, 0x07]
        );
        let mt = AdvancedConfig::End(End { code: 5 });
        assert_eq!(
            affected_keys(0x0A, &AdvancedConfig::None, &mt, &[0x04, 0x07]),
            vec![0x0A]
        );
        // S (0x16) was paired with Z (0x1D): binding A+S clears Z too, so Z
        // is re-read although this page never read S's old pair.
        assert_eq!(
            affected_keys(0x04, &AdvancedConfig::None, &new, &[0x16, 0x1D]),
            vec![0x04, 0x16, 0x1D]
        );
    }

    // ---- descriptions, selection, decoration

    #[test]
    fn descriptions_are_indonesian() {
        let mt = AdvancedConfig::Mt(Mt {
            hold: 0xE0,
            tap: 0x29,
            delay_ms: 200,
        });
        assert_eq!(describe(&mt), "Ketuk = ESC · tahan 200 ms = Ctrl kiri");
        let socd = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 4,
            key2: 7,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        assert_eq!(describe(&socd), "A ↔ D · Yang terakhir ditekan menang");
        assert_eq!(socd_mode_label(SocdMode::Key1Wins, "A", "D"), "A menang");
        assert_eq!(socd_mode_label(SocdMode::Key2Wins, "A", "D"), "D menang");
        assert_eq!(
            socd_mode_label(SocdMode::Neutral, "A", "D"),
            t::SOCD_NEUTRAL
        );
        let mpt = AdvancedConfig::Mpt(Mpt {
            codes: [4, 5, 6],
            depths: [um(600), um(1800), um(3200)],
        });
        assert_eq!(describe(&mpt), "0.60 mm = A · 1.80 mm = B · 3.20 mm = C");
    }

    #[test]
    fn a_click_selects_one_key_whatever_the_drag_did() {
        let s = |v: &[u8]| v.iter().copied().collect::<BTreeSet<u8>>();
        assert_eq!(single_selection(None, &s(&[])), None);
        assert_eq!(single_selection(None, &s(&[0x04])), Some(0x04));
        assert_eq!(single_selection(Some(0x04), &s(&[0x04, 0x07])), Some(0x07));
        assert_eq!(single_selection(Some(0x04), &s(&[0x07, 0x16])), Some(0x07));
        assert_eq!(single_selection(None, &s(&[0x16, 0x07])), Some(0x07));
    }

    #[test]
    fn picking_a_partner_takes_the_other_clicked_key() {
        let s = |v: &[u8]| v.iter().copied().collect::<BTreeSet<u8>>();
        assert_eq!(resolve_partner_pick(0x04, &s(&[0x04])), PartnerPick::Keep);
        assert_eq!(resolve_partner_pick(0x04, &s(&[])), PartnerPick::Keep);
        assert_eq!(
            resolve_partner_pick(0x04, &s(&[0x07])),
            PartnerPick::Partner(0x07)
        );
        assert_eq!(
            resolve_partner_pick(0x04, &s(&[0x04, 0x07])),
            PartnerPick::Partner(0x07)
        );
    }

    fn settings(adv: Advanced) -> KeySettings {
        KeySettings {
            mode: KeyMode {
                touch: TouchMode::Single,
                advanced: adv,
                high: 0,
            },
            actuation: um(2000),
            rt_press: um(150),
            rt_release: um(150),
            top_dz: um(100),
            bottom_dz: um(200),
            switch: 0,
        }
    }

    fn snap(keys: &[(u8, Advanced)]) -> DeviceSnapshot {
        let matrix = reference_matrix(Model::Win68HeMax);
        let mut map: BTreeMap<u8, KeySettings> = matrix
            .keys()
            .map(|(_, _, c)| (c, settings(Advanced::None)))
            .collect();
        for &(k, a) in keys {
            map.insert(k, settings(a));
        }
        DeviceSnapshot {
            model: Model::Win68HeMax,
            info: DeviceInfo {
                board_id: 0,
                in_bootloader: false,
                serial: String::new(),
                firmware: String::new(),
                build_date: String::new(),
            },
            system_mode: None,
            profile: 0,
            polling: PollingRate::Hz1000,
            matrix,
            keys: map,
        }
    }

    #[test]
    fn badges_use_the_function_colours() {
        let s = snap(&[
            (0x2C, Advanced::Dks),
            (0x39, Advanced::Mt),
            (0x08, Advanced::Tgl),
            (0x15, Advanced::Mpt),
            (0x04, Advanced::Socd),
            (0x1D, Advanced::End),
            (0x1B, Advanced::Macro),
        ]);
        let hex = |c: Color32| (c.r(), c.g(), c.b());
        for (key, text, rgb) in [
            (0x2C, "DKS", (0xA7, 0x8B, 0xFA)),
            (0x39, "MT", (0x38, 0xBD, 0xF8)),
            (0x08, "TGL", (0x34, 0xD3, 0x99)),
            (0x15, "MPT", (0xF4, 0x72, 0xB6)),
            (0x04, "SOCD", (0xFF, 0xB2, 0x24)),
            (0x1D, "END", (0xF8, 0x71, 0x71)),
        ] {
            let d = decorate(Some(&s), key, None, None);
            let (_, c) = d.badge.expect("badge");
            assert_eq!(d.label2.as_deref(), Some(text));
            assert_eq!(hex(c), rgb, "{text}");
        }
        assert_eq!(
            decorate(Some(&s), 0x1B, None, None).label2.as_deref(),
            Some("MAC")
        );
        assert_eq!(
            decorate(Some(&s), 0x05, None, None),
            KeyDecoration::default()
        );
        assert_eq!(decorate(None, 0x2C, None, None), KeyDecoration::default());
        // A filter hides the others.
        assert!(decorate(Some(&s), 0x2C, Some(Func::Mt), None)
            .badge
            .is_none());
        assert!(decorate(Some(&s), 0x39, Some(Func::Mt), None)
            .badge
            .is_some());
        // While picking, the chosen partner is marked.
        assert!(decorate(Some(&s), 0x05, None, Some(0x05)).badge.is_some());
    }

    #[test]
    fn the_table_lists_function_keys_in_matrix_order_with_one_socd_row_per_pair() {
        let s = snap(&[
            (0x2C, Advanced::Dks),
            (0x04, Advanced::Socd),
            (0x07, Advanced::Socd),
            (0x39, Advanced::Mt),
            (0x1B, Advanced::Macro),
        ]);
        let pair = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 4,
            key2: 7,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        let cache = BTreeMap::from([(0x04, pair), (0x07, pair)]);
        let r = rows(&s, &cache, None);
        let keys: Vec<u8> = r.iter().map(|r| r.key).collect();
        assert_eq!(
            keys,
            vec![0x39, 0x04, 0x2C],
            "A and D are one row; macros are not listed"
        );
        assert_eq!(r[1].cfg, Some(pair));
        assert_eq!(rows(&s, &cache, Some(Func::Mt)).len(), 1);
        assert!(rows(&s, &cache, Some(Func::End)).is_empty());
    }

    #[test]
    fn reads_are_planned_selected_key_first_and_never_retried_after_a_failure() {
        let s = snap(&[(0x2C, Advanced::Dks), (0x39, Advanced::Mt)]);
        let cache = BTreeMap::new();
        let failed = BTreeSet::new();
        // The selected key alone first.
        assert_eq!(
            plan_read(&s, &cache, &failed, Some(0x39), false),
            Some((vec![0x39], Some(0x39)))
        );
        // Then the table: all uncached function keys, in one job.
        let r = plan_read(&s, &cache, &failed, Some(0x39), true).unwrap();
        assert_eq!(r.1, None);
        assert_eq!(r.0.len(), 2);
        // A key without a function needs no editor read.
        assert_eq!(
            plan_read(&s, &cache, &failed, Some(0x04), false).map(|r| r.1),
            Some(None)
        );
        // Cached or failed keys are not read again.
        let cache = BTreeMap::from([(0x2C, AdvancedConfig::None)]);
        let failed = BTreeSet::from([0x39]);
        assert_eq!(plan_read(&s, &cache, &failed, None, true), None);
        assert_eq!(plan_read(&s, &cache, &failed, Some(0x39), false), None);
    }

    // ---- jobs against MockTransport

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn cmd_reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
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

    const KEY: u8 = 0x0A;

    fn end_cfg() -> AdvancedConfig {
        AdvancedConfig::End(End { code: 5 })
    }

    /// The seven replies of `key_settings`, with the given Mode.
    fn push_settings(m: &mut MockTransport, key: u8, mode: u16) {
        for (layout, v) in [
            (8, mode),
            (4, 2000),
            (20, 20),
            (21, 20),
            (22, 100),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(key, layout, v));
        }
    }

    /// Replies of a successful END write: current Mode, END echo, Mode again,
    /// Mode echo and read-back, the END read-back.
    fn script_end_write(m: &mut MockTransport) {
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(cmd_reply(0x28, &[0, KEY]));
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(key_reply(KEY, 8, 0x0015));
        m.push_reply(key_reply(KEY, 8, 0x0015));
        m.push_reply(cmd_reply(0x28, &[0, KEY, 5, 0, 0, 0]));
    }

    #[test]
    fn apply_success_rereads_the_config_and_the_settings() {
        let mut m = MockTransport::new();
        script_end_write(&mut m);
        // Re-read: Mode + END, then the key's settings.
        m.push_reply(key_reply(KEY, 8, 0x0015));
        m.push_reply(cmd_reply(0x28, &[0, KEY, 5, 0, 0, 0]));
        push_settings(&mut m, KEY, 0x0015);
        let out = apply_job(&mut kb(m), KEY, &end_cfg(), &[KEY]).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.config, Some(end_cfg()));
        assert_eq!(out.settings.len(), 1);
        assert_eq!(out.settings[0].1.mode.advanced, Advanced::End);
        let fx = apply_effects(Ok(out));
        assert!(fx.applied && !fx.request_reload);
        assert_eq!(fx.error, None);
        assert_eq!(fx.config, Some(end_cfg()));
        assert_eq!(fx.patch.len(), 1);
    }

    #[test]
    fn verify_failure_still_rereads_and_patches_the_true_state() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(cmd_reply(0x28, &[0, KEY]));
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(key_reply(KEY, 8, 0x0015)); // Mode echo
        m.push_reply(key_reply(KEY, 8, 0x0010)); // read-back: did not stick
        m.push_reply(key_reply(KEY, 8, 0x0010)); // re-read: no function
        push_settings(&mut m, KEY, 0x0010);
        let out = apply_job(&mut kb(m), KEY, &end_cfg(), &[KEY]).unwrap();
        assert!(
            matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed (mode)"))
        );
        assert_eq!(out.config, Some(AdvancedConfig::None));
        assert_eq!(out.settings[0].1.mode.advanced, Advanced::None);
        let fx = apply_effects(Ok(out));
        assert!(!fx.applied && !fx.request_reload);
        assert_eq!(fx.patch.len(), 1, "the true value replaces the badge");
        assert_eq!(
            fx.error.as_deref(),
            Some(format!("{} {}", i18n::error::VERIFY, t::RELOADED_HINT).as_str())
        );
        assert!(fx.technical.unwrap().contains("verify failed"));
    }

    #[test]
    fn a_failing_reread_asks_for_a_full_reload() {
        let mut m = MockTransport::new();
        script_end_write(&mut m);
        // Re-read: no replies, so it times out.
        let out = apply_job(&mut kb(m), KEY, &end_cfg(), &[KEY]).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.config, None);
        assert_eq!(out.reload_error, Some(Error::Timeout));
        let fx = apply_effects(Ok(out));
        assert!(fx.applied && fx.request_reload);
        assert!(fx.patch.is_empty());
    }

    #[test]
    fn a_rejected_write_is_reported_and_state_is_read_again() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(fail_reply()); // the END packet is rejected
        m.push_reply(key_reply(KEY, 8, 0x0010));
        push_settings(&mut m, KEY, 0x0010);
        let out = apply_job(&mut kb(m), KEY, &end_cfg(), &[KEY]).unwrap();
        assert_eq!(out.write, Err(Error::Frame(FrameError::DeviceFail)));
        assert_eq!(out.config, Some(AdvancedConfig::None));
        let fx = apply_effects(Ok(out));
        assert!(!fx.applied);
        assert!(fx.error.unwrap().starts_with(i18n::error::FRAME));
    }

    #[test]
    fn an_invalid_config_is_refused_before_any_io() {
        let mut k = kb(MockTransport::new());
        let bad = AdvancedConfig::Mpt(Mpt {
            codes: [4, 5, 6],
            depths: [um(3000), um(2000), um(1000)],
        });
        let out = apply_job(&mut k, KEY, &bad, &[KEY]).unwrap();
        assert!(matches!(out.write, Err(Error::InvalidInput(_))));
        // Only GET frames (payload starts 0x00) went out: no SET was built.
        assert!(k
            .session_mut()
            .transport()
            .written()
            .iter()
            .all(|f| f[4] == 0x00));
    }

    /// Fails every write after `n`, like a keyboard pulled mid-job.
    struct UnplugAfter {
        inner: MockTransport,
        writes_left: usize,
    }

    impl openhe_device::Transport for UnplugAfter {
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
    fn unplugged_mid_write_is_disconnected_with_a_partial_write_message() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0010));
        m.push_reply(cmd_reply(0x28, &[0, KEY]));
        // Mode read and the END packet go out; the next write fails.
        let t = UnplugAfter {
            inner: m,
            writes_left: 2,
        };
        let mut k = Keyboard::new(t, Model::Win68HeMax);
        let r = apply_job(&mut k, KEY, &end_cfg(), &[KEY]);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = apply_effects(r);
        assert_eq!(fx.error.as_deref(), Some(t::PARTIAL_WRITE));
        assert!(!fx.applied && fx.patch.is_empty() && !fx.request_reload);
    }

    #[test]
    fn deleting_clears_the_function_and_rereads() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0015)); // current Mode: END
        m.push_reply(key_reply(KEY, 8, 0x0015)); // Mode re-read
        m.push_reply(key_reply(KEY, 8, 0x0010)); // Mode echo
        m.push_reply(key_reply(KEY, 8, 0x0010)); // read-back
        m.push_reply(key_reply(KEY, 8, 0x0010)); // re-read
        push_settings(&mut m, KEY, 0x0010);
        let out = apply_job(&mut kb(m), KEY, &AdvancedConfig::None, &[KEY]).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.config, Some(AdvancedConfig::None));
    }

    #[test]
    fn socd_apply_rereads_both_keys_of_the_pair() {
        // Writing a pair on A (0x04) and D (0x07): both keys are re-read.
        let socd = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 4,
            key2: 7,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        let mut m = MockTransport::new();
        // Both Modes before (no SOCD), the SOCD echo, both Modes again, the
        // batched Mode write (2 echoes... one packet) and its read-backs.
        m.push_reply(key_reply(0x04, 8, 0x0010));
        m.push_reply(key_reply(0x07, 8, 0x0010));
        m.push_reply(cmd_reply(0x2C, &[0, 0x04]));
        m.push_reply(key_reply(0x04, 8, 0x0010));
        m.push_reply(key_reply(0x07, 8, 0x0010));
        m.push_reply(key_reply(0x04, 8, 0x0018)); // SET echo
        m.push_reply(key_reply(0x04, 8, 0x0018));
        m.push_reply(key_reply(0x07, 8, 0x0018));
        m.push_reply(cmd_reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 0, 0, 0, 0]));
        // Re-read of 0x04's config (Mode, SOCD), then settings of both keys.
        m.push_reply(key_reply(0x04, 8, 0x0018));
        m.push_reply(cmd_reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 0, 0, 0, 0]));
        push_settings(&mut m, 0x04, 0x0018);
        push_settings(&mut m, 0x07, 0x0018);
        let out = apply_job(&mut kb(m), 0x04, &socd, &[0x04, 0x07]).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.config, Some(socd));
        let keys: Vec<u8> = out.settings.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![0x04, 0x07]);
        assert!(out
            .settings
            .iter()
            .all(|(_, s)| s.mode.advanced == Advanced::Socd));
    }

    #[test]
    fn reading_collects_configs_and_stops_on_an_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0015));
        m.push_reply(cmd_reply(0x28, &[0, KEY, 5, 0, 0, 0]));
        // The second key never answers.
        let r = read_job(&mut kb(m), &[KEY, 0x0B, 0x0C]).unwrap();
        assert_eq!(r.configs, vec![(KEY, end_cfg())]);
        assert_eq!(r.missing, vec![0x0B, 0x0C]);
        assert_eq!(r.error, Some(Error::Timeout));
        let t = UnplugAfter {
            inner: MockTransport::new(),
            writes_left: 0,
        };
        assert_eq!(
            read_job(&mut Keyboard::new(t, Model::Win68HeMax), &[KEY]),
            Err(Error::Disconnected)
        );
    }

    #[test]
    fn other_job_errors_reload_everything() {
        let fx = apply_effects(Err(Error::Io(format!(
            "{}: boom",
            crate::worker::INTERNAL_ERROR
        ))));
        assert!(fx.request_reload);
        assert_eq!(fx.error.as_deref(), Some(i18n::error::INTERNAL));
    }

    // ---- page, headless

    #[test]
    fn an_import_that_only_changes_dks_params_drops_the_cached_config() {
        use crate::worker::tests::push_snapshot_with_mode;
        use crate::worker::{Worker, WorkerConfig};
        use std::time::{Duration, Instant};
        // W and A are DKS keys before and after the import: the snapshot
        // the reload reads is equal to the old one.
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot_with_mode(&mut m, 0, 0x0001);
                push_snapshot_with_mode(&mut m, 0, 0x0001);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            WorkerConfig {
                retry_interval: Duration::from_millis(5),
                idle_poll: Duration::from_millis(2),
                state_poll: Duration::from_secs(3600),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.is_connected() {
            assert!(Instant::now() < deadline, "never connected");
            std::thread::sleep(Duration::from_millis(1));
        }
        let snap = w.snapshot().unwrap();
        let old = AdvancedConfig::Dks(Dks {
            bindings: [0x1A, 0, 0, 0],
            trps: [0x01, 0, 0, 0],
            first_actuation: um(1500),
            bottom: um(3400),
        });
        let mut page = Page::default();
        page.cache.insert(0x1A, old);
        page.cache.insert(0x04, old);
        page.follow_generation(w.generation(), false);
        // The import finished: Pengaturan asks the worker for a reload.
        let g = w.generation();
        w.request_reload();
        let deadline = Instant::now() + Duration::from_secs(5);
        while w.generation() == g {
            assert!(Instant::now() < deadline, "the reload never showed");
            std::thread::sleep(Duration::from_millis(1));
        }
        page.follow_generation(w.generation(), false);
        assert!(page.cache.is_empty(), "stale DKS parameters dropped");
        // ... and read again.
        let plan = plan_read(&snap, &page.cache, &page.read_failed, None, true);
        assert_eq!(plan.map(|(keys, _)| keys.len()), Some(2));
    }

    #[test]
    fn any_reload_drops_cached_configs_except_the_pages_own_apply() {
        let mut page = Page::default();
        let fill = |page: &mut Page| {
            page.cache.insert(KEY, end_cfg());
            page.key = Some(KEY);
            page.editor = Some(Editor::new(KEY, &end_cfg()));
            page.read_failed.insert(0x05);
        };
        fill(&mut page);
        page.follow_generation(5, false); // first look: nothing to compare
        assert!(page.cache.contains_key(&KEY));
        page.follow_generation(5, false);
        assert!(page.cache.contains_key(&KEY), "same generation");
        // The page's own apply patched the snapshot: its fresh reads stay.
        page.follow_generation(6, true);
        assert!(page.cache.contains_key(&KEY));
        assert!(page.editor.is_some());
        // Anything else (a reload, an import, an event, another page's
        // write): the cached parameters may be stale, so they are read
        // again before the editor can send them back.
        page.follow_generation(7, false);
        assert!(page.cache.is_empty());
        assert!(page.editor.is_none());
        assert!(page.read_failed.is_empty());
        assert_eq!(page.key, Some(KEY), "the selection stays");
    }

    #[test]
    fn the_page_renders_connected_with_badges_and_an_editor() {
        use crate::app::test_support::*;
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;

        let worker = offline_worker();
        let snap = snap(&[(0x2C, Advanced::Dks), (0x39, Advanced::Mt)]);
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
        frame(&mut page, 2);
        // A key without a function opens a blank editor at once.
        page.selection.set([0x04]);
        frame(&mut page, 2);
        assert_eq!(page.key, Some(0x04));
        assert_eq!(
            page.editor.as_ref().map(|e| e.loaded),
            Some(AdvancedConfig::None)
        );
        // A multi-selection collapses to one key.
        page.selection.set([0x04, 0x05]);
        frame(&mut page, 2);
        assert_eq!(page.selection.len(), 1);
        assert_eq!(page.key, Some(0x05));
        // The editors of every function render.
        for f in FUNCS {
            page.editor.as_mut().unwrap().func = f;
            frame(&mut page, 2);
        }
    }

    #[test]
    fn the_fn_key_cannot_get_a_function_or_be_a_socd_partner() {
        let fn_key = FN_KEY;
        for f in FUNCS {
            let mut e = Editor::new(fn_key, &AdvancedConfig::None);
            e.func = f;
            e.dks.codes[0] = 4;
            e.dks.flags[0][0] = true;
            e.mt = MtForm {
                hold: 4,
                tap: 5,
                delay_ms: 200,
            };
            e.tgl.code = 4;
            e.end.code = 4;
            e.socd.partner = Some(0x04);
            assert_eq!(e.build(), Err(t::V_FN_KEY.to_string()), "{f:?}");
        }
        let mut e = Editor::new(0x04, &AdvancedConfig::None);
        e.func = Func::Socd;
        e.socd.partner = Some(FN_KEY);
        assert_eq!(e.build(), Err(t::V_SOCD_FN.to_string()));
        // Clicking Fn while picking a partner is ignored.
        let s = |v: &[u8]| v.iter().copied().collect::<BTreeSet<u8>>();
        assert_eq!(resolve_partner_pick(0x04, &s(&[FN_KEY])), PartnerPick::Keep);
        assert_eq!(
            resolve_partner_pick(0x04, &s(&[FN_KEY, 0x07])),
            PartnerPick::Partner(0x07)
        );
    }

    #[test]
    fn a_partner_with_another_function_is_warned_about() {
        let s = snap(&[
            (0x07, Advanced::Mt),
            (0x16, Advanced::Macro),
            (0x04, Advanced::Socd),
        ]);
        let none = AdvancedConfig::None;
        assert_eq!(
            partner_conflict(&s, 0x07, &none).as_deref(),
            Some("Fungsi MT di tombol D akan dihapus.")
        );
        assert_eq!(
            partner_conflict(&s, 0x16, &none).as_deref(),
            Some("Fungsi Macro di tombol S akan dihapus.")
        );
        assert_eq!(partner_conflict(&s, 0x05, &none), None);
        // The partner of the pair being edited is not a conflict.
        let pair = AdvancedConfig::Socd(Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 4,
            key2: 7,
            bind_values: false,
            mode: SocdMode::LastInput,
            delay: 0,
        });
        let s2 = snap(&[(0x04, Advanced::Socd), (0x07, Advanced::Socd)]);
        assert_eq!(partner_conflict(&s2, 0x07, &pair), None);
        assert!(partner_conflict(&s2, 0x07, &none).is_some());
    }
}

//! Templates: named sets of changes ("presets") for the active profile.
//!
//! A [`Template`] names only what it changes: per-key performance values
//! (each field optional), layer codes and advanced functions. It comes from
//! one of three places:
//!
//! - the built-ins ([`Builtin`]), defined in code and expanded for the
//!   connected model's keys;
//! - the user's own templates, saved from the keyboard's current settings
//!   ([`Template::from_current`]) and kept as `.openhe-template` files
//!   ([`TemplateFile`], versioned and model-tagged) in the app data
//!   directory ([`store`]);
//! - a backup: the whole key state read just before a template was applied
//!   (also [`Template::from_current`]). "Pulihkan" applies all of it back;
//!   "Urungkan" only what the applied template wrote
//!   ([`Template::limited_to`] a [`plan::Footprint`]).
//!
//! [`plan::plan_template`] turns a template plus the keyboard's current
//! state into the writes (an [`crate::pages::settings::profile_file::ImportPlan`],
//! run by the profile import's `import_job`) and a human summary.

pub mod plan;
pub mod store;

use crate::pages::remap::{is_win_alt, LAYER_ORDER as LAYERS};
use crate::pages::settings::profile_file::{
    model_from, perf_problem, touch_from, touch_name, AdvancedEntry, KeyData,
};
use openhe_device::{AdvancedConfig, KeySettings, PerformanceChange, FN_KEY};
use openhe_proto::advanced::{Socd, SocdMode};
use openhe_proto::{Layer, Micrometers, Model, TouchMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// `format` field of every template file.
pub const FORMAT: &str = "openhe-template";
pub const VERSION: u32 = 1;
/// File extension of exported templates (without the dot).
pub const EXTENSION: &str = "openhe-template";

/// Layer code 0x0001: transparent on the Fn layers, a dead key on Base.
pub use crate::pages::remap::TRANSPARENT;

mod strings {
    pub const FPS: &str = "Gaming FPS";
    pub const FPS_BLURB: &str = "W A S D 0.40 mm dengan Rapid Trigger, SOCD A↔D dan W↔S";
    pub const COMPETITIVE: &str = "Kompetitif (semua RT)";
    pub const COMPETITIVE_BLURB: &str = "Semua tombol 0.60 mm, Rapid Trigger 0.20 mm";
    pub const TYPING: &str = "Ngetik nyaman";
    pub const TYPING_BLURB: &str = "Semua tombol 2.00 mm, tanpa Rapid Trigger";
    pub const VIM: &str = "Panah ala Vim (Fn1)";
    pub const VIM_BLURB: &str = "Tahan Fn: H J K L jadi panah kiri, bawah, atas, kanan";
    pub const CAPS: &str = "Caps jadi Ctrl";
    pub const CAPS_BLURB: &str = "Caps Lock di layer Base jadi Ctrl kiri";
    pub const FACTORY: &str = "Kembali ke bawaan";
    pub const FACTORY_BLURB: &str =
        "Aktuasi, RT, deadzone, remap dan fungsi lanjutan seperti baru (bukan reset pabrik)";
    pub const NOTE_WIN60_FN: &str =
        "Isi bawaan layer Fn WIN 60 HE MAX belum diketahui, jadi layer Fn tidak diubah.";
    pub const NOTE_FACTORY_SWITCH: &str = "Jenis switch tiap tombol tidak diubah.";
    pub const NOTE_FACTORY_MACRO: &str = "Tombol dengan macro tetap memakai macro-nya.";

    pub const ERR_NOT_JSON: &str = "File ini bukan JSON yang valid.";
    pub const ERR_NOT_TEMPLATE: &str = "File ini bukan template OpenHE.";
    pub const ERR_NEWER: &str = "File ini dibuat oleh OpenHE yang lebih baru.";
    pub const ERR_MODEL: &str = "Model keyboard di file tidak dikenal.";
    pub const ERR_WRONG_MODEL: &str = "Template ini untuk keyboard lain:";
    pub const ERR_BAD_VALUE: &str = "File berisi nilai yang tidak valid:";
    pub const ERR_TRANSPARENT_BASE: &str =
        "Template memberi kode 0x0001 di layer Base untuk tombol";
    pub const ERR_TRANSPARENT_BASE_TAIL: &str = "(itu tombol mati, bukan Fn).";
    pub const ERR_EMPTY: &str = "Template ini tidak mengubah apa pun.";
}
use strings as t;

// ------------------------------------------------------------ the model

/// What a template sets on one key's performance; `None` leaves it as is.
/// The switch type is never part of a template (it is the hardware).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyPerf {
    pub touch: Option<TouchMode>,
    pub actuation: Option<Micrometers>,
    pub rt_press: Option<Micrometers>,
    pub rt_release: Option<Micrometers>,
    pub top_dz: Option<Micrometers>,
    pub bottom_dz: Option<Micrometers>,
}

impl KeyPerf {
    pub fn is_empty(&self) -> bool {
        *self == KeyPerf::default()
    }

    /// The same values as a device change (the switch never changes).
    pub fn change(&self) -> PerformanceChange {
        PerformanceChange {
            touch: self.touch,
            actuation: self.actuation,
            rt_press: self.rt_press,
            rt_release: self.rt_release,
            top_dz: self.top_dz,
            bottom_dz: self.bottom_dz,
            switch: None,
        }
    }

    /// Every value of `s` (not the switch). A key in touch mode Global
    /// does not use its own actuation and RT: they are kept only when its
    /// switch could take them (so a restore puts them back exactly), and
    /// left out otherwise, as the profile import does, so a leftover value
    /// out of range never makes a restore skip the key or refuse the whole
    /// template.
    pub fn of(model: Model, key: u8, s: &KeySettings) -> KeyPerf {
        let mut p = KeyPerf {
            touch: Some(s.mode.touch),
            actuation: Some(s.actuation),
            rt_press: Some(s.rt_press),
            rt_release: Some(s.rt_release),
            top_dz: Some(s.top_dz),
            bottom_dz: Some(s.bottom_dz),
        };
        if s.mode.touch == TouchMode::Global {
            let max = model
                .switches()
                .iter()
                .find(|sw| sw.id == s.switch)
                .map(|sw| sw.max_travel);
            let travel = PerformanceChange {
                actuation: p.actuation,
                rt_press: p.rt_press,
                rt_release: p.rt_release,
                ..Default::default()
            };
            if max.is_none() || perf_problem(key, &travel, max).is_some() {
                p.actuation = None;
                p.rt_press = None;
                p.rt_release = None;
            }
        }
        p
    }
}

/// Which parts a template changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub performance: bool,
    pub remap: bool,
    pub advanced: bool,
}

impl Parts {
    pub const ALL: Parts = Parts {
        performance: true,
        remap: true,
        advanced: true,
    };

    pub fn any(&self) -> bool {
        self.performance || self.remap || self.advanced
    }
}

/// A named set of changes for one keyboard model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub model: Model,
    pub performance: Vec<(u8, KeyPerf)>,
    pub layers: Vec<(u8, Layer, u16)>,
    pub advanced: Vec<(u8, AdvancedConfig)>,
    /// Notes shown with the preview (not stored in files).
    pub notes: Vec<&'static str>,
}

impl Template {
    pub fn parts(&self) -> Parts {
        Parts {
            performance: !self.performance.is_empty(),
            remap: !self.layers.is_empty(),
            advanced: !self.advanced.is_empty(),
        }
    }

    /// Every key the template names.
    pub fn keys(&self) -> BTreeSet<u8> {
        self.performance
            .iter()
            .map(|(k, _)| *k)
            .chain(self.layers.iter().map(|(k, _, _)| *k))
            .chain(self.advanced.iter().map(|(k, _)| *k))
            .collect()
    }

    /// The whole state of `keys` (as read from the keyboard) as a template,
    /// limited to `parts`: every performance value but the switch (see
    /// [`KeyPerf::of`] for keys in touch mode Global), every layer code (not
    /// the Fn key's own Base code, never 0x0001 on Base), and every advanced
    /// function (a key without one is cleared when the template is
    /// applied). Macros cannot be read back, so a macro key's function is
    /// left out.
    pub fn from_current(name: &str, model: Model, keys: &[KeyData], parts: Parts) -> Template {
        let mut t = Template {
            name: name.to_string(),
            model,
            performance: Vec::new(),
            layers: Vec::new(),
            advanced: Vec::new(),
            notes: Vec::new(),
        };
        for k in keys {
            if parts.performance {
                t.performance
                    .push((k.code, KeyPerf::of(model, k.code, &k.settings)));
            }
            if parts.remap {
                for (layer, code) in LAYERS.into_iter().zip(k.layers) {
                    let fn_base = k.code == FN_KEY && layer == Layer::Base;
                    let dead = layer == Layer::Base && code == TRANSPARENT;
                    if !fn_base && !dead {
                        t.layers.push((k.code, layer, code));
                    }
                }
            }
            if parts.advanced && k.advanced != AdvancedConfig::Macro {
                t.advanced.push((k.code, k.advanced));
            }
        }
        t
    }

    /// Only what `written` names: the performance fields, layer codes and
    /// functions an applied plan wrote ("Urungkan" of a backup puts back
    /// those and leaves everything else as it is now).
    pub fn limited_to(mut self, written: &plan::Footprint) -> Template {
        let only = |v: Option<Micrometers>, w: Option<Micrometers>| v.filter(|_| w.is_some());
        self.performance = self
            .performance
            .into_iter()
            .filter_map(|(k, p)| {
                let w = written.perf.get(&k)?;
                let p = KeyPerf {
                    touch: p.touch.filter(|_| w.touch.is_some()),
                    actuation: only(p.actuation, w.actuation),
                    rt_press: only(p.rt_press, w.rt_press),
                    rt_release: only(p.rt_release, w.rt_release),
                    top_dz: only(p.top_dz, w.top_dz),
                    bottom_dz: only(p.bottom_dz, w.bottom_dz),
                };
                (!p.is_empty()).then_some((k, p))
            })
            .collect();
        self.layers
            .retain(|&(k, l, _)| written.layers.iter().any(|&(wk, wl, _)| wk == k && wl == l));
        self.advanced
            .retain(|(k, _)| written.functions.iter().any(|(wk, _)| wk == k));
        self
    }
}

// ------------------------------------------------------------ built-ins

/// Gaming FPS: W A S D actuation and RT sensitivity.
pub const FPS_ACTUATION_UM: u16 = 400;
pub const FPS_RT_UM: u16 = 150;
/// Kompetitif: every key.
pub const COMPETITIVE_ACTUATION_UM: u16 = 600;
pub const COMPETITIVE_RT_UM: u16 = 200;
/// Ngetik nyaman: every key, touch mode Single.
pub const TYPING_ACTUATION_UM: u16 = 2000;
/// Factory per-key values (PROTOCOL 7.x finding 9): touch Global,
/// actuation 2.000 mm, RT 0.020 / 0.020 mm, deadzones 0.200 / 0.200 mm.
pub const FACTORY_ACTUATION_UM: u16 = 2000;
pub const FACTORY_RT_UM: u16 = 20;
pub const FACTORY_DEADZONE_UM: u16 = 200;

const KEY_W: u8 = 0x1A;
const KEY_A: u8 = 0x04;
const KEY_S: u8 = 0x16;
const KEY_D: u8 = 0x07;
const KEY_CAPS: u8 = 0x39;
const CODE_LCTRL: u16 = 0x00E0;
/// Vim keys on Fn1: H J K L → ← ↓ ↑ →.
const VIM: [(u8, u16); 4] = [
    (0x0B, 0x0050),
    (0x0D, 0x0051),
    (0x0E, 0x0052),
    (0x0F, 0x004F),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    GamingFps,
    Competitive,
    Typing,
    VimArrows,
    CapsCtrl,
    Factory,
}

fn um(v: u16) -> Option<Micrometers> {
    Some(Micrometers(v))
}

/// An SOCD pair with "last input wins" (the Tombol Lanjutan defaults: no
/// delay, keys send their own codes).
fn last_input_pair(a: u8, b: u8) -> AdvancedConfig {
    AdvancedConfig::Socd(Socd {
        pos1: a,
        pos2: b,
        key1: u16::from(a),
        key2: u16::from(b),
        bind_values: false,
        mode: SocdMode::LastInput,
        delay: 0,
    })
}

impl Builtin {
    pub const ALL: [Builtin; 6] = [
        Builtin::GamingFps,
        Builtin::Competitive,
        Builtin::Typing,
        Builtin::VimArrows,
        Builtin::CapsCtrl,
        Builtin::Factory,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::GamingFps => t::FPS,
            Builtin::Competitive => t::COMPETITIVE,
            Builtin::Typing => t::TYPING,
            Builtin::VimArrows => t::VIM,
            Builtin::CapsCtrl => t::CAPS,
            Builtin::Factory => t::FACTORY,
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Builtin::GamingFps => t::FPS_BLURB,
            Builtin::Competitive => t::COMPETITIVE_BLURB,
            Builtin::Typing => t::TYPING_BLURB,
            Builtin::VimArrows => t::VIM_BLURB,
            Builtin::CapsCtrl => t::CAPS_BLURB,
            Builtin::Factory => t::FACTORY_BLURB,
        }
    }

    /// The template for a `model` keyboard whose keys are `keys` (its
    /// matrix). Keys a preset names that the keyboard lacks stay in the
    /// template; the plan skips them and says so.
    pub fn template(self, model: Model, keys: &[u8]) -> Template {
        let mut t = Template {
            name: self.name().to_string(),
            model,
            performance: Vec::new(),
            layers: Vec::new(),
            advanced: Vec::new(),
            notes: Vec::new(),
        };
        let all = |p: KeyPerf| keys.iter().map(|&k| (k, p)).collect::<Vec<_>>();
        match self {
            Builtin::GamingFps => {
                let p = KeyPerf {
                    touch: Some(TouchMode::RapidTrigger),
                    actuation: um(FPS_ACTUATION_UM),
                    rt_press: um(FPS_RT_UM),
                    rt_release: um(FPS_RT_UM),
                    ..Default::default()
                };
                t.performance = [KEY_W, KEY_A, KEY_S, KEY_D]
                    .into_iter()
                    .map(|k| (k, p))
                    .collect();
                let ad = last_input_pair(KEY_A, KEY_D);
                let ws = last_input_pair(KEY_W, KEY_S);
                t.advanced = vec![(KEY_A, ad), (KEY_D, ad), (KEY_W, ws), (KEY_S, ws)];
            }
            Builtin::Competitive => {
                t.performance = all(KeyPerf {
                    touch: Some(TouchMode::RapidTrigger),
                    actuation: um(COMPETITIVE_ACTUATION_UM),
                    rt_press: um(COMPETITIVE_RT_UM),
                    rt_release: um(COMPETITIVE_RT_UM),
                    ..Default::default()
                });
            }
            Builtin::Typing => {
                t.performance = all(KeyPerf {
                    touch: Some(TouchMode::Single),
                    actuation: um(TYPING_ACTUATION_UM),
                    ..Default::default()
                });
            }
            Builtin::VimArrows => {
                t.layers = VIM.iter().map(|&(k, c)| (k, Layer::Fn1, c)).collect();
            }
            Builtin::CapsCtrl => {
                t.layers = vec![(KEY_CAPS, Layer::Base, CODE_LCTRL)];
            }
            Builtin::Factory => {
                t.performance = all(KeyPerf {
                    touch: Some(TouchMode::Global),
                    actuation: um(FACTORY_ACTUATION_UM),
                    rt_press: um(FACTORY_RT_UM),
                    rt_release: um(FACTORY_RT_UM),
                    top_dz: um(FACTORY_DEADZONE_UM),
                    bottom_dz: um(FACTORY_DEADZONE_UM),
                });
                for &k in keys {
                    for layer in LAYERS {
                        let code = match (layer, k) {
                            (Layer::Base, FN_KEY) => None,
                            // LAlt / LWin: their own code; the plan keeps
                            // the Win/Mac swap (see plan_template).
                            (Layer::Base, k) if is_win_alt(k) => Some(u16::from(k)),
                            _ => crate::pages::remap::factory_code(model, layer, k),
                        };
                        if let Some(c) = code {
                            t.layers.push((k, layer, c));
                        }
                    }
                }
                t.advanced = keys.iter().map(|&k| (k, AdvancedConfig::None)).collect();
                if model == Model::Win60HeMax {
                    t.notes.push(t::NOTE_WIN60_FN);
                }
                t.notes.push(t::NOTE_FACTORY_SWITCH);
                t.notes.push(t::NOTE_FACTORY_MACRO);
            }
        }
        t
    }
}

// -------------------------------------------------------------- the file

/// Why a template cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    NotJson(String),
    NotATemplate,
    NewerVersion(u32),
    UnknownModel(String),
    WrongModel {
        file: String,
        keyboard: String,
    },
    BadValue(String),
    /// Code 0x0001 on the Base layer of this key (a dead key there).
    TransparentOnBase(u8),
    /// The template changes nothing at all.
    Empty,
}

/// Indonesian text for a template problem.
pub fn error_text(e: &TemplateError) -> String {
    match e {
        TemplateError::NotJson(_) => t::ERR_NOT_JSON.to_string(),
        TemplateError::NotATemplate => t::ERR_NOT_TEMPLATE.to_string(),
        TemplateError::NewerVersion(_) => t::ERR_NEWER.to_string(),
        TemplateError::UnknownModel(_) => t::ERR_MODEL.to_string(),
        TemplateError::WrongModel { file, keyboard } => {
            format!("{} {file} (keyboard ini {keyboard}).", t::ERR_WRONG_MODEL)
        }
        TemplateError::BadValue(v) => format!("{} {v}.", t::ERR_BAD_VALUE),
        TemplateError::TransparentOnBase(k) => format!(
            "{} {} {}",
            t::ERR_TRANSPARENT_BASE,
            openhe_proto::keycodes::legend(*k),
            t::ERR_TRANSPARENT_BASE_TAIL
        ),
        TemplateError::Empty => t::ERR_EMPTY.to_string(),
    }
}

fn bad(msg: String) -> TemplateError {
    TemplateError::BadValue(msg)
}

/// One key's performance values in a file; absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerfEntry {
    pub key: u8,
    /// "global", "single" or "rapid_trigger".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actuation_um: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rt_press_um: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rt_release_um: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_deadzone_um: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_deadzone_um: Option<u16>,
}

/// One layer code in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerEntry {
    pub key: u8,
    /// "base", "fn1", "fn2" or "fn3".
    pub layer: String,
    pub code: u16,
}

/// One advanced function in a file (`{"type": "none"}` clears it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionEntry {
    pub key: u8,
    pub function: AdvancedEntry,
}

/// A `.openhe-template` file: only the parts the template changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// "WIN 68 HE MAX" / "WIN 60 HE MAX".
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub performance: Vec<PerfEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advanced: Vec<FunctionEntry>,
}

pub fn layer_name(l: Layer) -> &'static str {
    match l {
        Layer::Base => "base",
        Layer::Fn1 => "fn1",
        Layer::Fn2 => "fn2",
        Layer::Fn3 => "fn3",
    }
}

fn layer_from(s: &str) -> Option<Layer> {
    LAYERS.into_iter().find(|&l| layer_name(l) == s)
}

impl TemplateFile {
    pub fn from_template(t: &Template) -> TemplateFile {
        TemplateFile {
            format: FORMAT.to_string(),
            version: VERSION,
            name: t.name.clone(),
            model: t.model.name().to_string(),
            performance: t
                .performance
                .iter()
                .map(|(k, p)| PerfEntry {
                    key: *k,
                    touch: p.touch.map(|t| touch_name(t).to_string()),
                    actuation_um: p.actuation.map(|v| v.0),
                    rt_press_um: p.rt_press.map(|v| v.0),
                    rt_release_um: p.rt_release.map(|v| v.0),
                    top_deadzone_um: p.top_dz.map(|v| v.0),
                    bottom_deadzone_um: p.bottom_dz.map(|v| v.0),
                })
                .collect(),
            layers: t
                .layers
                .iter()
                .map(|&(key, l, code)| LayerEntry {
                    key,
                    layer: layer_name(l).to_string(),
                    code,
                })
                .collect(),
            advanced: t
                .advanced
                .iter()
                .map(|(key, c)| FunctionEntry {
                    key: *key,
                    function: AdvancedEntry::from_config(c),
                })
                .collect(),
        }
    }

    /// Back to a [`Template`], checking the format, version, model and
    /// every closed set (touch names, layer names, function types and their
    /// packet validation), and refusing duplicates, macros (their contents
    /// are not in a template) and an empty template. Values that depend on
    /// the key (switch travel) are checked by the plan.
    pub fn to_template(&self) -> Result<Template, TemplateError> {
        if self.format != FORMAT {
            return Err(TemplateError::NotATemplate);
        }
        if self.version > VERSION {
            return Err(TemplateError::NewerVersion(self.version));
        }
        let model = model_from(&self.model)
            .ok_or_else(|| TemplateError::UnknownModel(self.model.clone()))?;
        let mut t = Template {
            name: self.name.trim().to_string(),
            model,
            performance: Vec::new(),
            layers: Vec::new(),
            advanced: Vec::new(),
            notes: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        for p in &self.performance {
            if !seen.insert(p.key) {
                return Err(bad(format!("key 0x{:02X} listed twice", p.key)));
            }
            let touch = match &p.touch {
                None => None,
                Some(s) => Some(touch_from(s).ok_or_else(|| bad(format!("touch {s:?}")))?),
            };
            let perf = KeyPerf {
                touch,
                actuation: p.actuation_um.map(Micrometers),
                rt_press: p.rt_press_um.map(Micrometers),
                rt_release: p.rt_release_um.map(Micrometers),
                top_dz: p.top_deadzone_um.map(Micrometers),
                bottom_dz: p.bottom_deadzone_um.map(Micrometers),
            };
            if perf.is_empty() {
                return Err(bad(format!("key 0x{:02X} without values", p.key)));
            }
            t.performance.push((p.key, perf));
        }
        let mut seen = BTreeSet::new();
        for l in &self.layers {
            let layer = layer_from(&l.layer).ok_or_else(|| bad(format!("layer {:?}", l.layer)))?;
            if !seen.insert((l.key, layer as u8)) {
                return Err(bad(format!("key 0x{:02X} {} listed twice", l.key, l.layer)));
            }
            if layer == Layer::Base && l.code == TRANSPARENT {
                return Err(TemplateError::TransparentOnBase(l.key));
            }
            t.layers.push((l.key, layer, l.code));
        }
        let mut seen = BTreeSet::new();
        for f in &self.advanced {
            if !seen.insert(f.key) {
                return Err(bad(format!("function of 0x{:02X} listed twice", f.key)));
            }
            let cfg = f
                .function
                .to_config()
                .map_err(|e| bad(format!("0x{:02X}: {e:?}", f.key)))?;
            if cfg == AdvancedConfig::Macro {
                return Err(bad(format!("macro on 0x{:02X}", f.key)));
            }
            t.advanced.push((f.key, cfg));
        }
        if !t.parts().any() {
            return Err(TemplateError::Empty);
        }
        Ok(t)
    }

    pub fn to_json(&self) -> String {
        // Plain data with string keys: serialising cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parses a file. The format and version are checked before the rest,
    /// so a newer file is reported as newer, not as broken.
    pub fn from_json(s: &str) -> Result<TemplateFile, TemplateError> {
        let v: serde_json::Value =
            serde_json::from_str(s).map_err(|e| TemplateError::NotJson(e.to_string()))?;
        if v.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
            return Err(TemplateError::NotATemplate);
        }
        match v.get("version").and_then(|n| n.as_u64()) {
            Some(n) if n > u64::from(VERSION) => {
                return Err(TemplateError::NewerVersion(
                    u32::try_from(n).unwrap_or(u32::MAX),
                ))
            }
            Some(_) => {}
            None => return Err(TemplateError::NotATemplate),
        }
        serde_json::from_value(v).map_err(|e| TemplateError::BadValue(e.to_string()))
    }
}

/// The keys of `snap` with the factory layer codes and no advanced
/// functions: what a factory-fresh keyboard with these settings reads
/// (the screenshot preview and tests use it instead of a read).
pub fn snapshot_keys(snap: &crate::worker::DeviceSnapshot) -> Vec<KeyData> {
    snap.keys
        .iter()
        .map(|(&code, s)| {
            let mut layers = [0u16; 4];
            for (slot, layer) in layers.iter_mut().zip(LAYERS) {
                *slot = crate::pages::remap::factory_code(snap.model, layer, code).unwrap_or(
                    match layer {
                        Layer::Base => u16::from(code),
                        _ => TRANSPARENT,
                    },
                );
            }
            KeyData {
                code,
                settings: *s,
                layers,
                advanced: AdvancedConfig::None,
            }
        })
        .collect()
}

/// Parses and checks a template file's text.
pub fn parse(text: &str) -> Result<Template, TemplateError> {
    TemplateFile::from_json(text)?.to_template()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::test_support::{snapshot60, snapshot68};
    use crate::worker::DeviceSnapshot;
    use openhe_device::KeySettings;
    use openhe_proto::advanced::Mt;

    /// The keys of `snap` as a factory keyboard reads them (Windows mode).
    pub fn factory_keys(snap: &DeviceSnapshot) -> Vec<KeyData> {
        snapshot_keys(snap)
    }

    fn keys_of(snap: &DeviceSnapshot) -> Vec<u8> {
        snap.matrix.keys().map(|(_, _, c)| c).collect()
    }

    #[test]
    fn every_builtin_round_trips_through_a_file() {
        for snap in [snapshot68(), snapshot60()] {
            for b in Builtin::ALL {
                let tpl = b.template(snap.model, &keys_of(&snap));
                let json = TemplateFile::from_template(&tpl).to_json();
                assert!(json.contains("\"format\": \"openhe-template\""));
                assert!(json.contains(snap.model.name()));
                let back = parse(&json).unwrap_or_else(|e| panic!("{}: {e:?}", b.name()));
                assert_eq!(
                    Template {
                        notes: Vec::new(),
                        ..tpl
                    },
                    back,
                    "{}",
                    b.name()
                );
            }
        }
    }

    #[test]
    fn a_file_holds_only_the_parts_it_changes() {
        let snap = snapshot68();
        let caps = Builtin::CapsCtrl.template(snap.model, &keys_of(&snap));
        let json = TemplateFile::from_template(&caps).to_json();
        assert!(!json.contains("performance"), "{json}");
        assert!(!json.contains("advanced"), "{json}");
        assert!(json.contains("\"layer\": \"base\""), "{json}");
        let fps = Builtin::GamingFps.template(snap.model, &keys_of(&snap));
        let json = TemplateFile::from_template(&fps).to_json();
        assert!(
            !json.contains("top_deadzone_um"),
            "untouched values stay out"
        );
        assert!(!json.contains("layers"));
        assert!(json.contains("\"type\": \"socd\""));
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        let snap = snapshot68();
        let good =
            TemplateFile::from_template(&Builtin::GamingFps.template(snap.model, &keys_of(&snap)));
        assert!(matches!(parse("{ nope"), Err(TemplateError::NotJson(_))));
        assert_eq!(parse("{}"), Err(TemplateError::NotATemplate));
        let profile = r#"{"format": "openhe-profile", "version": 1}"#;
        assert_eq!(parse(profile), Err(TemplateError::NotATemplate));
        // A newer file is "newer", even with fields this version lacks.
        let mut v: serde_json::Value = serde_json::from_str(&good.to_json()).unwrap();
        v["version"] = serde_json::json!(VERSION + 1);
        v["sparkles"] = serde_json::json!(true);
        assert_eq!(
            parse(&v.to_string()),
            Err(TemplateError::NewerVersion(VERSION + 1))
        );
        let mut cases: Vec<(&str, TemplateFile)> = Vec::new();
        let mut push = |name: &'static str, f: &dyn Fn(&mut TemplateFile)| {
            let mut x = good.clone();
            f(&mut x);
            cases.push((name, x));
        };
        push("unknown model", &|f| f.model = "WIN 99".into());
        push("touch name", &|f| {
            f.performance[0].touch = Some("turbo".into())
        });
        push("duplicate key", &|f| {
            let p = f.performance[0].clone();
            f.performance.push(p);
        });
        push("no values", &|f| {
            f.performance[0] = PerfEntry {
                key: 0x1A,
                ..Default::default()
            }
        });
        push("layer name", &|f| {
            f.layers.push(LayerEntry {
                key: 0x39,
                layer: "fn9".into(),
                code: 0xE0,
            })
        });
        push("0x0001 on base", &|f| {
            f.layers.push(LayerEntry {
                key: 0x39,
                layer: "base".into(),
                code: 0x0001,
            })
        });
        push("macro", &|f| f.advanced[0].function = AdvancedEntry::Macro);
        push("socd mode", &|f| {
            if let AdvancedEntry::Socd { mode, .. } = &mut f.advanced[0].function {
                *mode = 9;
            }
        });
        push("empty", &|f| {
            f.performance.clear();
            f.advanced.clear();
        });
        for (name, f) in cases {
            let r = parse(&f.to_json());
            assert!(r.is_err(), "{name}: {r:?}");
            assert!(!error_text(&r.unwrap_err()).is_empty());
        }
        // Unknown fields are a broken file, not silently ignored.
        let typo = good
            .to_json()
            .replacen("\"actuation_um\"", "\"actuation\"", 1);
        assert!(matches!(parse(&typo), Err(TemplateError::BadValue(_))));
    }

    #[test]
    fn from_current_keeps_every_part_but_macros_and_fns_base() {
        let snap = snapshot68();
        let mut keys = factory_keys(&snap);
        keys.iter_mut().find(|k| k.code == 0x08).unwrap().advanced = AdvancedConfig::Mt(Mt {
            hold: 0xE0,
            tap: 0x08,
            delay_ms: 200,
        });
        keys.iter_mut().find(|k| k.code == 0x10).unwrap().advanced = AdvancedConfig::Macro;
        let tpl = Template::from_current("x", snap.model, &keys, Parts::ALL);
        assert_eq!(tpl.performance.len(), 68);
        assert_eq!(tpl.layers.len(), 68 * 4 - 1);
        assert!(!tpl
            .layers
            .iter()
            .any(|&(k, l, _)| k == FN_KEY && l == Layer::Base));
        assert_eq!(tpl.advanced.len(), 67, "the macro key is left out");
        let only_perf = Template::from_current(
            "x",
            snap.model,
            &keys,
            Parts {
                performance: true,
                remap: false,
                advanced: false,
            },
        );
        assert!(!only_perf.parts().remap);
        assert!(only_perf.layers.is_empty() && only_perf.advanced.is_empty());
        let s: KeySettings = snap.keys[&0x1A];
        assert_eq!(
            only_perf
                .performance
                .iter()
                .find(|(k, _)| *k == 0x1A)
                .unwrap()
                .1,
            KeyPerf {
                touch: Some(s.mode.touch),
                actuation: Some(s.actuation),
                rt_press: Some(s.rt_press),
                rt_release: Some(s.rt_release),
                top_dz: Some(s.top_dz),
                bottom_dz: Some(s.bottom_dz),
            }
        );
        // And it survives a file.
        let back = parse(&TemplateFile::from_template(&tpl).to_json()).unwrap();
        assert_eq!(back, tpl);
    }

    /// A key in touch mode Global does not use its own actuation and RT.
    /// Values its switch could take are kept (a restore puts them back
    /// exactly); a leftover out of range is left out, as the profile
    /// import does, so it neither skips the key nor refuses the backup.
    #[test]
    fn global_keys_keep_only_travel_values_their_switch_takes() {
        let snap = snapshot68();
        let mut keys = factory_keys(&snap);
        let set = |keys: &mut Vec<KeyData>, code: u8, act: u16, rt: u16| {
            let k = keys.iter_mut().find(|k| k.code == code).unwrap();
            k.settings.mode.touch = TouchMode::Global;
            k.settings.actuation = Micrometers(act);
            k.settings.rt_press = Micrometers(rt);
        };
        set(&mut keys, 0x14, 1500, 100); // fine
        set(&mut keys, 0x1A, 9000, 100); // beyond any switch
        set(&mut keys, 0x04, 1500, 0); // RT below 0.005 mm
        let tpl = Template::from_current("b", snap.model, &keys, Parts::ALL);
        let perf = |code: u8| tpl.performance.iter().find(|(k, _)| *k == code).unwrap().1;
        assert_eq!(perf(0x14).actuation, Some(Micrometers(1500)));
        assert_eq!(perf(0x14).rt_press, Some(Micrometers(100)));
        for code in [0x1A, 0x04] {
            let p = perf(code);
            assert_eq!(p.touch, Some(TouchMode::Global));
            assert_eq!((p.actuation, p.rt_press, p.rt_release), (None, None, None));
            assert!(
                p.top_dz.is_some() && p.bottom_dz.is_some(),
                "deadzones stay"
            );
        }
        // The backup plans on a keyboard where those keys changed since.
        let mut now = factory_keys(&snap);
        for k in now.iter_mut().filter(|k| matches!(k.code, 0x1A | 0x04)) {
            k.settings.mode.touch = TouchMode::RapidTrigger;
        }
        let p = plan::plan_template(&tpl, &snap, &now).unwrap();
        assert!(p.skipped.is_empty() || p.skipped.iter().all(|(k, _)| *k == FN_KEY));
        assert!(p.changed_keys().contains(&0x1A) && p.changed_keys().contains(&0x04));
    }
}

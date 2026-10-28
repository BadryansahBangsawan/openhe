//! Import/export of the active onboard profile as a JSON file.
//!
//! The file holds every matrix key's performance settings (touch mode,
//! actuation, rapid trigger, deadzones, switch), its four layer codes and
//! its advanced function, plus the main lighting state and the per-key
//! colours. Macro contents cannot be read back from the keyboard, so a key
//! bound to a macro is exported as `"macro"` and skipped on import.
//!
//! - [`export_job`] reads everything (device side, [`ProfileData`]);
//!   [`ProfileFile`] is its serde mirror (the proto types have no serde).
//! - [`plan_import`] turns a file into the writes for this keyboard
//!   ([`ImportPlan`]): pure and tested; [`import_job`] runs it. Every write
//!   goes through the verified device API; afterwards the page asks the
//!   worker for a full reload.

use crate::worker::DeviceSnapshot;
use openhe_device::keyboard::performance::{MAX_DEADZONE_UM, MIN_RT_UM};
use openhe_device::{AdvancedConfig, Error, KeySettings, Keyboard, PerformanceChange, Transport};
use openhe_proto::advanced::{self, Dks, End, Mpt, Mt, Socd, SocdMode, Tgl};
use openhe_proto::key::{Advanced, KeyMode};
use openhe_proto::lighting::{Lighting, Rgb, COLOR_SLOTS, DEFAULT_EXTRA_FLAGS};
use openhe_proto::model::MIN_ACTUATION_UM;
use openhe_proto::{Layer, Micrometers, Model, TouchMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// `format` field of every profile file.
pub const FORMAT: &str = "openhe-profile";
pub const VERSION: u32 = 1;

const LAYERS: [Layer; 4] = [Layer::Base, Layer::Fn1, Layer::Fn2, Layer::Fn3];
/// The Fn key: its own base-layer code is never written.
const FN_KEY: u8 = 0x01;

// ------------------------------------------------------------ device side

/// One key as read from the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyData {
    pub code: u8,
    pub settings: KeySettings,
    /// Codes on Base, Fn1, Fn2, Fn3.
    pub layers: [u16; 4],
    pub advanced: AdvancedConfig,
}

/// A whole profile as read from (or to be written to) the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileData {
    pub model: Model,
    pub keys: Vec<KeyData>,
    /// `None` when the lighting could not be read.
    pub lighting: Option<Lighting>,
    pub key_colors: Vec<(u8, Rgb)>,
    /// The file did not say which lighting flag bits (other than on and
    /// reverse) to write: keep the keyboard's own on import.
    pub keep_lighting_flags: bool,
}

/// What an export read, plus the parts it had to leave out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub data: ProfileData,
    /// Lighting or per-key colours that could not be read (left out).
    pub skipped: Vec<(&'static str, Error)>,
}

/// Reads the active profile: every key (settings, layers, advanced), then
/// lighting and per-key colours. Key errors fail the export; the lighting
/// parts are best effort (left out and listed in `skipped`). A disconnect
/// is always `Err(Disconnected)`.
pub fn export_job<T: Transport>(kb: &mut Keyboard<T>, keys: &[u8]) -> Result<Exported, Error> {
    let mut out = Vec::with_capacity(keys.len());
    for &code in keys {
        let settings = kb.key_settings(code)?;
        let mut layers = [0u16; 4];
        for (slot, layer) in layers.iter_mut().zip(LAYERS) {
            *slot = kb.layer_code(code, layer)?;
        }
        let advanced = kb.advanced(code)?;
        out.push(KeyData {
            code,
            settings,
            layers,
            advanced,
        });
    }
    let mut skipped = Vec::new();
    let lighting = match kb.lighting() {
        Ok(l) => Some(l),
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(e) => {
            skipped.push(("lighting", e));
            None
        }
    };
    let key_colors = match kb.key_colors(keys) {
        Ok(c) => c,
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(e) => {
            skipped.push(("key_colors", e));
            Vec::new()
        }
    };
    Ok(Exported {
        data: ProfileData {
            model: kb.model,
            keys: out,
            lighting,
            key_colors,
            keep_lighting_flags: false,
        },
        skipped,
    })
}

// -------------------------------------------------------------- the file

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFile {
    pub format: String,
    pub version: u32,
    /// "WIN 68 HE MAX" / "WIN 60 HE MAX".
    pub model: String,
    pub keys: Vec<KeyEntry>,
    #[serde(default)]
    pub lighting: Option<LightingEntry>,
    /// `[key, r, g, b]`.
    #[serde(default)]
    pub key_colors: Vec<[u8; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEntry {
    /// Default keycode (matrix key id).
    pub code: u8,
    /// "global", "single" or "rapid_trigger".
    pub touch: String,
    pub actuation_um: u16,
    pub rt_press_um: u16,
    pub rt_release_um: u16,
    pub top_deadzone_um: u16,
    pub bottom_deadzone_um: u16,
    pub switch: u16,
    /// Codes on Base, Fn1, Fn2, Fn3.
    pub layers: [u16; 4],
    pub advanced: AdvancedEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdvancedEntry {
    None,
    Dks {
        bindings: [u16; 4],
        trps: [u8; 4],
        first_actuation_um: u16,
        bottom_um: u16,
    },
    Mt {
        hold: u16,
        tap: u16,
        delay_ms: u16,
    },
    Tgl {
        code: u16,
        delay_ms: u16,
    },
    Mpt {
        codes: [u16; 3],
        depths_um: [u16; 3],
    },
    End {
        code: u16,
    },
    Socd {
        pos1: u8,
        pos2: u8,
        key1: u16,
        key2: u16,
        bind_values: bool,
        /// 0 last input, 1 key 1 wins, 2 key 2 wins, 3 neutral.
        mode: u8,
        delay: u16,
    },
    /// Macro contents cannot be read back; skipped on import.
    Macro,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightingEntry {
    pub on: bool,
    pub reverse: bool,
    pub brightness: u8,
    pub mode: u8,
    pub speed: u8,
    pub sleep_minutes: u8,
    pub static_color: u8,
    pub dynamic_color: u8,
    /// Seven `[r, g, b]` colours.
    pub colors: Vec<[u8; 3]>,
    /// Flag bits other than on and reverse, as the keyboard reported them.
    /// Older exports do not have it: their import keeps the keyboard's own
    /// bits instead of guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_flags: Option<u8>,
}

/// Why a file cannot be used (shown to the user in Indonesian).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    NotJson(String),
    NotAProfile,
    NewerVersion(u32),
    UnknownModel(String),
    WrongModel { file: String, keyboard: String },
    BadValue(String),
}

fn touch_name(t: TouchMode) -> &'static str {
    match t {
        TouchMode::Global => "global",
        TouchMode::Single => "single",
        TouchMode::RapidTrigger => "rapid_trigger",
    }
}

fn touch_from(s: &str) -> Option<TouchMode> {
    match s {
        "global" => Some(TouchMode::Global),
        "single" => Some(TouchMode::Single),
        "rapid_trigger" => Some(TouchMode::RapidTrigger),
        _ => None,
    }
}

fn model_from(s: &str) -> Option<Model> {
    [Model::Win68HeMax, Model::Win60HeMax]
        .into_iter()
        .find(|m| m.name() == s.trim())
}

fn um(v: u16) -> Micrometers {
    Micrometers(v)
}

impl AdvancedEntry {
    fn from_config(c: &AdvancedConfig) -> AdvancedEntry {
        match *c {
            AdvancedConfig::None => AdvancedEntry::None,
            AdvancedConfig::Dks(d) => AdvancedEntry::Dks {
                bindings: d.bindings,
                trps: d.trps,
                first_actuation_um: d.first_actuation.0,
                bottom_um: d.bottom.0,
            },
            AdvancedConfig::Mt(m) => AdvancedEntry::Mt {
                hold: m.hold,
                tap: m.tap,
                delay_ms: m.delay_ms,
            },
            AdvancedConfig::Tgl(t) => AdvancedEntry::Tgl {
                code: t.code,
                delay_ms: t.delay_ms,
            },
            AdvancedConfig::Mpt(m) => AdvancedEntry::Mpt {
                codes: m.codes,
                depths_um: m.depths.map(|d| d.0),
            },
            AdvancedConfig::End(e) => AdvancedEntry::End { code: e.code },
            AdvancedConfig::Socd(s) => AdvancedEntry::Socd {
                pos1: s.pos1,
                pos2: s.pos2,
                key1: s.key1,
                key2: s.key2,
                bind_values: s.bind_values,
                mode: s.mode as u8,
                delay: s.delay,
            },
            AdvancedConfig::Macro => AdvancedEntry::Macro,
        }
    }

    fn to_config(&self) -> Result<AdvancedConfig, FileError> {
        Ok(match *self {
            AdvancedEntry::None => AdvancedConfig::None,
            AdvancedEntry::Dks {
                bindings,
                trps,
                first_actuation_um,
                bottom_um,
            } => AdvancedConfig::Dks(Dks {
                bindings,
                trps,
                first_actuation: um(first_actuation_um),
                bottom: um(bottom_um),
            }),
            AdvancedEntry::Mt {
                hold,
                tap,
                delay_ms,
            } => AdvancedConfig::Mt(Mt {
                hold,
                tap,
                delay_ms,
            }),
            AdvancedEntry::Tgl { code, delay_ms } => AdvancedConfig::Tgl(Tgl { code, delay_ms }),
            AdvancedEntry::Mpt { codes, depths_um } => AdvancedConfig::Mpt(Mpt {
                codes,
                depths: depths_um.map(um),
            }),
            AdvancedEntry::End { code } => AdvancedConfig::End(End { code }),
            AdvancedEntry::Socd {
                pos1,
                pos2,
                key1,
                key2,
                bind_values,
                mode,
                delay,
            } => AdvancedConfig::Socd(Socd {
                pos1,
                pos2,
                key1,
                key2,
                bind_values,
                mode: SocdMode::from_u8(mode)
                    .ok_or_else(|| FileError::BadValue(format!("SOCD mode {mode}")))?,
                delay,
            }),
            AdvancedEntry::Macro => AdvancedConfig::Macro,
        })
    }
}

fn nibble(c: &AdvancedConfig) -> Advanced {
    match c {
        AdvancedConfig::None => Advanced::None,
        AdvancedConfig::Dks(_) => Advanced::Dks,
        AdvancedConfig::Mt(_) => Advanced::Mt,
        AdvancedConfig::Tgl(_) => Advanced::Tgl,
        AdvancedConfig::Mpt(_) => Advanced::Mpt,
        AdvancedConfig::End(_) => Advanced::End,
        AdvancedConfig::Socd(_) => Advanced::Socd,
        AdvancedConfig::Macro => Advanced::Macro,
    }
}

impl ProfileFile {
    pub fn from_data(d: &ProfileData) -> ProfileFile {
        ProfileFile {
            format: FORMAT.to_string(),
            version: VERSION,
            model: d.model.name().to_string(),
            keys: d
                .keys
                .iter()
                .map(|k| KeyEntry {
                    code: k.code,
                    touch: touch_name(k.settings.mode.touch).to_string(),
                    actuation_um: k.settings.actuation.0,
                    rt_press_um: k.settings.rt_press.0,
                    rt_release_um: k.settings.rt_release.0,
                    top_deadzone_um: k.settings.top_dz.0,
                    bottom_deadzone_um: k.settings.bottom_dz.0,
                    switch: k.settings.switch,
                    layers: k.layers,
                    advanced: AdvancedEntry::from_config(&k.advanced),
                })
                .collect(),
            lighting: d.lighting.map(|l| LightingEntry {
                on: l.on,
                reverse: l.reverse,
                brightness: l.brightness,
                mode: l.mode,
                speed: l.speed,
                sleep_minutes: l.sleep_minutes,
                static_color: l.static_color,
                dynamic_color: l.dynamic_color,
                colors: l.colors.iter().map(|c| [c.r, c.g, c.b]).collect(),
                extra_flags: Some(l.extra_flags),
            }),
            key_colors: d
                .key_colors
                .iter()
                .map(|(k, c)| [*k, c.r, c.g, c.b])
                .collect(),
        }
    }

    /// Back to device types, checking the format and every value that has
    /// a closed set. The Mode high byte is not part of the file (it is kept
    /// as the keyboard has it when writing).
    pub fn to_data(&self) -> Result<ProfileData, FileError> {
        if self.format != FORMAT {
            return Err(FileError::NotAProfile);
        }
        if self.version > VERSION {
            return Err(FileError::NewerVersion(self.version));
        }
        let model =
            model_from(&self.model).ok_or_else(|| FileError::UnknownModel(self.model.clone()))?;
        let mut keys = Vec::with_capacity(self.keys.len());
        for k in &self.keys {
            let touch = touch_from(&k.touch)
                .ok_or_else(|| FileError::BadValue(format!("touch {:?}", k.touch)))?;
            let advanced = k.advanced.to_config()?;
            keys.push(KeyData {
                code: k.code,
                settings: KeySettings {
                    mode: KeyMode {
                        touch,
                        advanced: nibble(&advanced),
                        high: 0,
                    },
                    actuation: um(k.actuation_um),
                    rt_press: um(k.rt_press_um),
                    rt_release: um(k.rt_release_um),
                    top_dz: um(k.top_deadzone_um),
                    bottom_dz: um(k.bottom_deadzone_um),
                    switch: k.switch,
                },
                layers: k.layers,
                advanced,
            });
        }
        let lighting = match &self.lighting {
            None => None,
            Some(l) => {
                if l.colors.len() != COLOR_SLOTS {
                    return Err(FileError::BadValue(format!(
                        "{} lighting colours",
                        l.colors.len()
                    )));
                }
                let mut colors = [Rgb::default(); COLOR_SLOTS];
                for (slot, c) in colors.iter_mut().zip(&l.colors) {
                    *slot = Rgb {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                    };
                }
                let lighting = Lighting {
                    on: l.on,
                    reverse: l.reverse,
                    brightness: l.brightness,
                    mode: l.mode,
                    speed: l.speed,
                    sleep_minutes: l.sleep_minutes,
                    static_color: l.static_color,
                    dynamic_color: l.dynamic_color,
                    colors,
                    extra_flags: l.extra_flags.unwrap_or(DEFAULT_EXTRA_FLAGS),
                };
                lighting
                    .validate()
                    .map_err(|e| FileError::BadValue(e.to_string()))?;
                Some(lighting)
            }
        };
        let keep_lighting_flags = self
            .lighting
            .as_ref()
            .is_some_and(|l| l.extra_flags.is_none());
        Ok(ProfileData {
            model,
            keys,
            lighting,
            keep_lighting_flags,
            key_colors: self
                .key_colors
                .iter()
                .map(|c| {
                    (
                        c[0],
                        Rgb {
                            r: c[1],
                            g: c[2],
                            b: c[3],
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn to_json(&self) -> String {
        // Plain data with string keys: serialising cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Result<ProfileFile, FileError> {
        serde_json::from_str(s).map_err(|e| FileError::NotJson(e.to_string()))
    }
}

// ---------------------------------------------------------------- import

/// The writes an import makes on this keyboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportPlan {
    /// Keys sharing one change go in one `set_performance` call.
    pub performance: Vec<(Vec<u8>, PerformanceChange)>,
    /// Keys whose current function must be cleared first.
    pub clear_advanced: Vec<u8>,
    /// Functions to set (an SOCD pair once).
    pub advanced: Vec<(u8, AdvancedConfig)>,
    pub layers: Vec<(u8, Layer, u16)>,
    pub lighting: Option<Lighting>,
    /// Write the keyboard's current lighting flag bits instead of the
    /// plan's (an older file without them).
    pub keep_lighting_flags: bool,
    pub key_colors: Vec<(u8, Rgb)>,
    /// Keys with a macro in the file (not imported).
    pub skipped_macros: Vec<u8>,
    /// Keys in the file this keyboard does not have.
    pub skipped_unknown: Vec<u8>,
}

impl ImportPlan {
    /// Number of keys whose settings are written.
    pub fn key_count(&self) -> usize {
        self.performance.iter().map(|(k, _)| k.len()).sum()
    }
}

/// The performance write for one key. A key in touch mode Global keeps
/// only its deadzones and switch (the device API rejects actuation/RT with
/// Global, and they have no effect there).
fn perf_change(s: &KeySettings) -> PerformanceChange {
    let base = PerformanceChange {
        touch: Some(s.mode.touch),
        top_dz: Some(s.top_dz),
        bottom_dz: Some(s.bottom_dz),
        switch: Some(s.switch),
        ..Default::default()
    };
    if s.mode.touch == TouchMode::Global {
        base
    } else {
        PerformanceChange {
            actuation: Some(s.actuation),
            rt_press: Some(s.rt_press),
            rt_release: Some(s.rt_release),
            ..base
        }
    }
}

fn bad(msg: String) -> FileError {
    FileError::BadValue(msg)
}

/// The checks the device API would make while writing `change` to `key`
/// (switch id, actuation 0.1 mm..=max, RT 0.005 mm..=max, deadzones
/// 0..=1.0 mm), done up front so a bad file writes nothing.
fn check_performance(model: Model, key: u8, change: &PerformanceChange) -> Result<(), FileError> {
    let id = change.switch.unwrap_or(0);
    let sw = model
        .switches()
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| bad(format!("key 0x{key:02X}: unknown switch id {id}")))?;
    let max = sw.max_travel.0;
    if let Some(a) = change.actuation {
        if a.0 < MIN_ACTUATION_UM || a.0 > max {
            return Err(bad(format!("key 0x{key:02X}: actuation {} um", a.0)));
        }
    }
    for (name, v) in [
        ("RT press", change.rt_press),
        ("RT release", change.rt_release),
    ] {
        if let Some(v) = v {
            if v.0 < MIN_RT_UM || v.0 > max {
                return Err(bad(format!("key 0x{key:02X}: {name} {} um", v.0)));
            }
        }
    }
    for (name, v) in [
        ("top deadzone", change.top_dz),
        ("bottom deadzone", change.bottom_dz),
    ] {
        if let Some(v) = v {
            if v.0 > MAX_DEADZONE_UM {
                return Err(bad(format!("key 0x{key:02X}: {name} {} um", v.0)));
            }
        }
    }
    Ok(())
}

/// The function checks of `set_advanced`: the proto packet builders'
/// validation and SOCD positions that are real keys of this keyboard with
/// `key` one of them. Keycodes are not checked against the keycode table:
/// the factory map itself holds codes it does not know (0x6152, 0xF344). Like the Tombol Lanjutan page, the
/// Fn key never gets a function (macros included) and is never part of a
/// SOCD pair: what the firmware does then is unknown, and losing Fn loses
/// the Fn layers.
fn check_advanced(key: u8, cfg: &AdvancedConfig, present: &BTreeSet<u8>) -> Result<(), FileError> {
    let invalid = |e: openhe_proto::advanced::AdvancedError| bad(format!("key 0x{key:02X}: {e}"));
    if key == FN_KEY && *cfg != AdvancedConfig::None {
        return Err(bad(
            "the Fn key cannot have an advanced function".to_string()
        ));
    }
    match cfg {
        AdvancedConfig::None | AdvancedConfig::Macro => {}
        AdvancedConfig::Dks(d) => {
            advanced::dks_set(key, d).map_err(invalid)?;
        }
        AdvancedConfig::Mt(m) => {
            advanced::mt_set(key, m).map_err(invalid)?;
        }
        AdvancedConfig::Tgl(t) => {
            advanced::tgl_set(key, t).map_err(invalid)?;
        }
        AdvancedConfig::Mpt(m) => {
            advanced::mpt_set(key, m).map_err(invalid)?;
        }
        AdvancedConfig::End(_) => {}
        AdvancedConfig::Socd(s) => {
            advanced::socd_set(s).map_err(invalid)?;
            if key != s.pos1 && key != s.pos2 {
                return Err(bad(format!("key 0x{key:02X} is not in its SOCD pair")));
            }
            if s.pos1 == FN_KEY || s.pos2 == FN_KEY {
                return Err(bad("the Fn key cannot be in a SOCD pair".to_string()));
            }
            for p in [s.pos1, s.pos2] {
                if !present.contains(&p) {
                    return Err(bad(format!("SOCD position 0x{p:02X} is not a key")));
                }
            }
        }
    }
    Ok(())
}

/// Plans an import of `data` onto the keyboard in `snap`.
///
/// Everything the device API would refuse is checked here first, so a bad
/// file is rejected (`FileError`) before anything is written: the model,
/// duplicate key entries, every performance value against its switch,
/// every advanced function (packet validation, SOCD positions), and the
/// per-key colours (no duplicates). Keycodes the keycode table does not
/// know are accepted and written as they are (the factory Fn layers hold
/// some). The Fn
/// key's own base-layer code is left out of the writes, and a function on
/// the Fn key (or a SOCD pair with it) refuses the file.
pub fn plan_import(data: &ProfileData, snap: &DeviceSnapshot) -> Result<ImportPlan, FileError> {
    if data.model != snap.model {
        return Err(FileError::WrongModel {
            file: data.model.name().to_string(),
            keyboard: snap.model.name().to_string(),
        });
    }
    let present: BTreeSet<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
    let mut seen = BTreeSet::new();
    for k in &data.keys {
        if !seen.insert(k.code) {
            return Err(bad(format!("key 0x{:02X} is listed twice", k.code)));
        }
    }
    let mut colored = BTreeSet::new();
    for (k, _) in &data.key_colors {
        if !colored.insert(*k) {
            return Err(bad(format!("colour of key 0x{k:02X} is listed twice")));
        }
    }
    let mut plan = ImportPlan::default();
    let mut socd_done: Vec<(u8, u8)> = Vec::new();
    for k in &data.keys {
        if !present.contains(&k.code) {
            plan.skipped_unknown.push(k.code);
            continue;
        }
        let change = perf_change(&k.settings);
        check_performance(snap.model, k.code, &change)?;
        check_advanced(k.code, &k.advanced, &present)?;
        match plan.performance.iter_mut().find(|(_, c)| *c == change) {
            Some((keys, _)) => keys.push(k.code),
            None => plan.performance.push((vec![k.code], change)),
        }
        let current = snap.keys.get(&k.code).map(|s| s.mode.advanced);
        match k.advanced {
            AdvancedConfig::Macro => plan.skipped_macros.push(k.code),
            AdvancedConfig::None => {
                if current.is_some_and(|a| a != Advanced::None) || current.is_none() {
                    plan.clear_advanced.push(k.code);
                }
            }
            AdvancedConfig::Socd(s) => {
                let pair = (s.pos1.min(s.pos2), s.pos1.max(s.pos2));
                if !socd_done.contains(&pair) {
                    socd_done.push(pair);
                    plan.advanced.push((k.code, k.advanced));
                }
            }
            other => plan.advanced.push((k.code, other)),
        }
        for (layer, code) in LAYERS.into_iter().zip(k.layers) {
            if k.code == FN_KEY && layer == Layer::Base {
                continue;
            }
            plan.layers.push((k.code, layer, code));
        }
    }
    plan.lighting = data.lighting;
    plan.keep_lighting_flags = data.keep_lighting_flags;
    plan.key_colors = data
        .key_colors
        .iter()
        .copied()
        .filter(|(k, _)| present.contains(k))
        .collect();
    Ok(plan)
}

/// One part of an import that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStep {
    Performance,
    Advanced(u8),
    Layers,
    Lighting,
    KeyColors,
}

/// What [`import_job`] did: every step is tried, failures are collected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub failed: Vec<(ImportStep, Error)>,
}

/// Runs an import plan. Each step is independent: a failure is recorded
/// and the next step still runs. A disconnect stops at once with
/// `Err(Disconnected)`. The caller always reloads the snapshot afterwards.
pub fn import_job<T: Transport>(
    kb: &mut Keyboard<T>,
    plan: &ImportPlan,
) -> Result<ImportReport, Error> {
    let mut report = ImportReport::default();
    let mut record = |step: ImportStep, r: Result<(), Error>| -> Result<(), Error> {
        match r {
            Ok(()) => Ok(()),
            Err(Error::Disconnected) => Err(Error::Disconnected),
            Err(e) => {
                report.failed.push((step, e));
                Ok(())
            }
        }
    };
    for (keys, change) in &plan.performance {
        record(ImportStep::Performance, kb.set_performance(keys, change))?;
    }
    for &k in &plan.clear_advanced {
        record(
            ImportStep::Advanced(k),
            kb.set_advanced(k, &AdvancedConfig::None),
        )?;
    }
    for (k, cfg) in &plan.advanced {
        record(ImportStep::Advanced(*k), kb.set_advanced(*k, cfg))?;
    }
    if !plan.layers.is_empty() {
        record(ImportStep::Layers, kb.set_layer_codes(&plan.layers))?;
    }
    if let Some(l) = &plan.lighting {
        let mut l = *l;
        let flags = if plan.keep_lighting_flags {
            kb.lighting().map(|cur| cur.extra_flags)
        } else {
            Ok(l.extra_flags)
        };
        let r = flags.and_then(|f| {
            l.extra_flags = f;
            kb.set_lighting(&l)
        });
        record(ImportStep::Lighting, r)?;
    }
    if !plan.key_colors.is_empty() {
        record(ImportStep::KeyColors, kb.set_key_colors(&plan.key_colors))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::snapshot68;
    use openhe_device::MockTransport;

    fn settings(touch: TouchMode, adv: Advanced) -> KeySettings {
        KeySettings {
            mode: KeyMode {
                touch,
                advanced: adv,
                high: 0,
            },
            actuation: um(1200),
            rt_press: um(150),
            rt_release: um(200),
            top_dz: um(100),
            bottom_dz: um(200),
            switch: 2,
        }
    }

    fn key(code: u8, touch: TouchMode, advanced: AdvancedConfig) -> KeyData {
        KeyData {
            code,
            settings: settings(touch, nibble(&advanced)),
            layers: [u16::from(code), 0x3A, 0, 0x01],
            advanced,
        }
    }

    fn socd() -> Socd {
        Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 0x04,
            key2: 0x07,
            bind_values: false,
            mode: SocdMode::Neutral,
            delay: 5,
        }
    }

    fn sample() -> ProfileData {
        ProfileData {
            keep_lighting_flags: false,
            model: Model::Win68HeMax,
            keys: vec![
                key(0x29, TouchMode::Global, AdvancedConfig::None),
                key(0x1A, TouchMode::RapidTrigger, AdvancedConfig::None),
                key(
                    0x14,
                    TouchMode::Single,
                    AdvancedConfig::Dks(Dks {
                        bindings: [0x14, 0xE1, 0, 0],
                        trps: [0x01, 0x80, 0, 0],
                        first_actuation: um(1500),
                        bottom: um(3400),
                    }),
                ),
                key(
                    0x08,
                    TouchMode::Single,
                    AdvancedConfig::Mt(Mt {
                        hold: 0xE0,
                        tap: 0x08,
                        delay_ms: 200,
                    }),
                ),
                key(
                    0x15,
                    TouchMode::Single,
                    AdvancedConfig::Tgl(Tgl {
                        code: 0x15,
                        delay_ms: 150,
                    }),
                ),
                key(
                    0x17,
                    TouchMode::Single,
                    AdvancedConfig::Mpt(Mpt {
                        codes: [0x17, 0x18, 0x19],
                        depths: [um(1000), um(2000), um(3000)],
                    }),
                ),
                key(
                    0x1C,
                    TouchMode::Single,
                    AdvancedConfig::End(End { code: 0x1C }),
                ),
                key(0x04, TouchMode::RapidTrigger, AdvancedConfig::Socd(socd())),
                key(0x07, TouchMode::RapidTrigger, AdvancedConfig::Socd(socd())),
                key(0x10, TouchMode::Single, AdvancedConfig::Macro),
                key(0x01, TouchMode::Global, AdvancedConfig::None),
            ],
            lighting: Some(Lighting {
                on: true,
                reverse: false,
                brightness: 3,
                mode: 21,
                speed: 2,
                sleep_minutes: 10,
                static_color: 7,
                dynamic_color: 0,
                colors: [Rgb {
                    r: 255,
                    g: 178,
                    b: 36,
                }; COLOR_SLOTS],
                extra_flags: 0xC0,
            }),
            key_colors: vec![(0x1A, Rgb { r: 1, g: 2, b: 3 })],
        }
    }

    #[test]
    fn json_round_trip_keeps_everything() {
        let data = sample();
        let file = ProfileFile::from_data(&data);
        let json = file.to_json();
        assert!(json.contains("\"format\": \"openhe-profile\""));
        assert!(json.contains("\"type\": \"socd\""));
        let back = ProfileFile::from_json(&json).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.to_data().unwrap(), data);
    }

    #[test]
    fn lighting_flag_bits_round_trip_and_old_files_keep_the_keyboards() {
        let file = ProfileFile::from_data(&sample());
        let json = file.to_json();
        assert!(json.contains("\"extra_flags\": 192"));
        let back = ProfileFile::from_json(&json).unwrap().to_data().unwrap();
        assert!(!back.keep_lighting_flags);
        assert_eq!(back.lighting.unwrap().extra_flags, 0xC0);
        // A file written before the field existed: the keyboard's own bits
        // are kept on import (no guessed 0x10).
        let old = json.replace("\"extra_flags\": 192", "\"extra_flags_gone\": 0");
        let back = ProfileFile::from_json(&old).unwrap().to_data().unwrap();
        assert!(back.keep_lighting_flags);
        let plan = plan_import(&back, &snapshot68()).unwrap();
        assert!(plan.keep_lighting_flags);
    }

    /// An import with `keep_lighting_flags` reads the keyboard's lighting
    /// first and writes its flag bits back with the file's other values.
    #[test]
    fn an_old_file_import_writes_the_keyboards_own_flag_bits() {
        use openhe_proto::lighting;
        let mut l = sample().lighting.unwrap();
        l.extra_flags = 0x10;
        let plan = ImportPlan {
            lighting: Some(l),
            keep_lighting_flags: true,
            ..Default::default()
        };
        let mut on_kb = l;
        on_kb.extra_flags = 0xC0;
        let mut reply = [0u8; 64];
        reply[..4].copy_from_slice(&[0x5C, 0x2D, 0x98, 0x00]);
        let body = lighting::set(&on_kb).unwrap().payload;
        reply[5..4 + body.len()].copy_from_slice(&body[1..]);
        let mut nack = [0u8; 64];
        nack[..8].copy_from_slice(&[0x5C, 0x04, 0x80, 0x00, 0x00, 0x47, 0x01, 0xFF]);
        let mut m = MockTransport::new();
        m.push_reply(nack);
        m.push_reply(reply); // the current lighting
        m.push_reply(reply); // SET reply
        m.push_reply(reply); // read-back
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        let report = import_job(&mut kb, &plan).unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        let w = kb.session_mut().transport().written();
        let set = w
            .iter()
            .rev()
            .find(|f| f[2] == 0x18 && f[4] == 0x01)
            .unwrap();
        assert_eq!(set[4 + 37], 0xC0 | 0x01, "keyboard bits + on");
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        assert!(matches!(
            ProfileFile::from_json("{ not json"),
            Err(FileError::NotJson(_))
        ));
        let mut f = ProfileFile::from_data(&sample());
        f.format = "something-else".into();
        assert_eq!(f.to_data(), Err(FileError::NotAProfile));
        let mut f = ProfileFile::from_data(&sample());
        f.version = VERSION + 1;
        assert_eq!(f.to_data(), Err(FileError::NewerVersion(VERSION + 1)));
        let mut f = ProfileFile::from_data(&sample());
        f.model = "WIN 99".into();
        assert!(matches!(f.to_data(), Err(FileError::UnknownModel(_))));
        let mut f = ProfileFile::from_data(&sample());
        f.keys[0].touch = "turbo".into();
        assert!(matches!(f.to_data(), Err(FileError::BadValue(_))));
        let mut f = ProfileFile::from_data(&sample());
        f.lighting.as_mut().unwrap().brightness = 9;
        assert!(matches!(f.to_data(), Err(FileError::BadValue(_))));
    }

    #[test]
    fn plan_writes_what_the_file_says_and_protects_fn() {
        let snap = snapshot68();
        let plan = plan_import(&sample(), &snap).unwrap();
        // Global keys (Esc, Fn) keep only deadzones and switch.
        let global = plan
            .performance
            .iter()
            .find(|(k, _)| k.contains(&0x29))
            .unwrap();
        assert_eq!(global.0, vec![0x29, 0x01]);
        assert_eq!(global.1.touch, Some(TouchMode::Global));
        assert_eq!(global.1.actuation, None);
        assert_eq!(global.1.switch, Some(2));
        // Keys with the same change are batched.
        let rt = plan
            .performance
            .iter()
            .find(|(k, _)| k.contains(&0x1A))
            .unwrap();
        assert_eq!(rt.0, vec![0x1A, 0x04, 0x07]);
        assert_eq!(rt.1.actuation, Some(um(1200)));
        assert_eq!(plan.key_count(), 11);
        // The SOCD pair is written once; the macro key is skipped.
        let socd_writes = plan
            .advanced
            .iter()
            .filter(|(_, c)| matches!(c, AdvancedConfig::Socd(_)))
            .count();
        assert_eq!(socd_writes, 1);
        assert_eq!(plan.advanced.len(), 6);
        assert_eq!(plan.skipped_macros, vec![0x10]);
        // Nothing to clear: the snapshot has no functions.
        assert!(plan.clear_advanced.is_empty());
        // Every layer of every key except Fn's own base code.
        assert_eq!(plan.layers.len(), 11 * 4 - 1);
        assert!(!plan
            .layers
            .iter()
            .any(|&(k, l, _)| k == 0x01 && l == Layer::Base));
        assert!(plan.layers.contains(&(0x01, Layer::Fn1, 0x3A)));
        assert!(plan.lighting.is_some());
        assert_eq!(plan.key_colors.len(), 1);
    }

    #[test]
    fn plan_clears_functions_the_file_does_not_have() {
        let mut snap = snapshot68();
        snap.keys.get_mut(&0x29).unwrap().mode.advanced = Advanced::Dks;
        let plan = plan_import(&sample(), &snap).unwrap();
        assert_eq!(plan.clear_advanced, vec![0x29]);
    }

    /// Every way a file can be wrong where the device API would refuse it,
    /// placed in the file's last key so earlier keys would have been
    /// written by a check-as-you-go import.
    #[test]
    fn a_bad_value_late_in_the_file_writes_nothing() {
        let snap = snapshot68();
        let last = sample().keys.len() - 1; // the Fn key, Global
        let mut cases: Vec<(&str, ProfileData)> = Vec::new();
        let mut push = |name: &'static str, f: &dyn Fn(&mut ProfileData)| {
            let mut d = sample();
            f(&mut d);
            cases.push((name, d));
        };
        push("unknown switch", &|d| d.keys[last].settings.switch = 9);
        push("actuation beyond travel", &|d| {
            let k = &mut d.keys[1].settings; // rapid trigger key
            k.actuation = um(3500);
        });
        push("actuation below 0.1", &|d| {
            d.keys[1].settings.actuation = um(50)
        });
        push("RT below 0.005", &|d| d.keys[1].settings.rt_press = um(2));
        push("RT beyond travel", &|d| {
            d.keys[1].settings.rt_release = um(3401)
        });
        push("deadzone above 1 mm", &|d| {
            d.keys[last].settings.bottom_dz = um(1001)
        });
        push("DKS depth", &|d| {
            if let AdvancedConfig::Dks(x) = &mut d.keys[2].advanced {
                x.first_actuation = um(10);
            }
        });
        push("MPT depth", &|d| {
            if let AdvancedConfig::Mpt(x) = &mut d.keys[5].advanced {
                x.depths[2] = um(9000);
            }
        });
        push("MT delay", &|d| {
            if let AdvancedConfig::Mt(x) = &mut d.keys[3].advanced {
                x.delay_ms = 0;
            }
        });
        push("SOCD position not a key", &|d| {
            for k in &mut d.keys {
                if let AdvancedConfig::Socd(x) = &mut k.advanced {
                    x.pos2 = 0x68;
                }
            }
        });
        push("SOCD on a key outside its pair", &|d| {
            d.keys[last].advanced = AdvancedConfig::Socd(socd());
        });
        push("function on the Fn key", &|d| {
            d.keys[last].advanced = AdvancedConfig::Mt(Mt {
                hold: 0xE0,
                tap: 0x08,
                delay_ms: 200,
            });
        });
        push("END on the Fn key", &|d| {
            d.keys[last].advanced = AdvancedConfig::End(End { code: 0x29 });
        });
        push("macro on the Fn key", &|d| {
            d.keys[last].advanced = AdvancedConfig::Macro;
        });
        push("SOCD pair with the Fn key", &|d| {
            let pair = Socd {
                pos2: 0x01,
                ..socd()
            };
            d.keys[7].advanced = AdvancedConfig::Socd(pair); // key 0x04
            d.keys[8].advanced = AdvancedConfig::None; // key 0x07
        });
        push("duplicate key", &|d| {
            let k = d.keys[0].clone();
            d.keys.push(k);
        });
        push("duplicate colour", &|d| {
            let c = d.key_colors[0];
            d.key_colors.push(c);
        });
        for (name, data) in cases {
            let r = plan_import(&data, &snap);
            assert!(matches!(r, Err(FileError::BadValue(_))), "{name}: {r:?}");
            // The UI only runs import_job with a plan, so nothing is
            // written; show it with a keyboard that records writes.
            let mut kb = Keyboard::new(MockTransport::new(), Model::Win68HeMax);
            if let Ok(plan) = r {
                let _ = import_job(&mut kb, &plan);
            }
            assert!(
                kb.session_mut().transport().written().is_empty(),
                "{name}: wrote before failing"
            );
        }
        // The untouched sample still plans.
        assert!(plan_import(&sample(), &snap).is_ok());
    }

    /// The factory map itself holds codes the keycode table does not know
    /// (Fn2: 0x6152, 0x681B; Fn1 Caps: 0xF344), so an exported factory
    /// profile must import again. Unknown codes are written as they are.
    #[test]
    fn unknown_keycodes_are_imported_as_they_are() {
        let snap = snapshot68();
        let mut d = sample();
        let fn_key = d.keys.len() - 1;
        let a = d.keys.iter().position(|k| k.code == 0x04).unwrap();
        d.keys[a].layers = [0x0004, 0xF344, 0x6152, 0x681B];
        d.keys[fn_key].layers[2] = 0xBEEF;
        if let Some(AdvancedConfig::Tgl(x)) =
            d.keys.iter_mut().find_map(|k| match &mut k.advanced {
                AdvancedConfig::Tgl(_) => Some(&mut k.advanced),
                _ => None,
            })
        {
            x.code = 0xBEEF;
        } else {
            panic!("the sample has a TGL key");
        }
        let plan = plan_import(&d, &snap).expect("unknown codes are accepted");
        for want in [
            (0x04, Layer::Fn1, 0xF344),
            (0x04, Layer::Fn2, 0x6152),
            (0x04, Layer::Fn3, 0x681B),
            (0x01, Layer::Fn2, 0xBEEF),
        ] {
            assert!(plan.layers.contains(&want), "{want:?}");
        }
        assert!(plan
            .advanced
            .iter()
            .any(|(_, c)| matches!(c, AdvancedConfig::Tgl(t) if t.code == 0xBEEF)));
        // The Fn guard stays: Fn's own Base code is never written.
        assert!(!plan
            .layers
            .iter()
            .any(|&(k, l, _)| k == 0x01 && l == Layer::Base));
    }

    #[test]
    fn plan_refuses_another_model_and_skips_unknown_keys() {
        let mut data = sample();
        data.model = Model::Win60HeMax;
        assert!(matches!(
            plan_import(&data, &snapshot68()),
            Err(FileError::WrongModel { .. })
        ));
        let mut data = sample();
        data.keys
            .push(key(0x68, TouchMode::Single, AdvancedConfig::None)); // F13: not on a WIN 68
        let plan = plan_import(&data, &snapshot68()).unwrap();
        assert_eq!(plan.skipped_unknown, vec![0x68]);
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x5C, 0x39, 0xA3]);
        let [lo, hi] = v.to_le_bytes();
        r[4..9].copy_from_slice(&[0, key, layout, lo, hi]);
        r
    }

    #[test]
    fn import_runs_every_step_and_collects_failures() {
        let plan = ImportPlan {
            layers: vec![(0x04, Layer::Fn1, 0x05)],
            lighting: Some(sample().lighting.unwrap()),
            ..Default::default()
        };
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 1, 0x05)); // SET echo
        m.push_reply(key_reply(0x04, 1, 0x05)); // read-back
                                                // No lighting replies: that step times out, the job carries on.
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        let report = import_job(&mut kb, &plan).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, ImportStep::Lighting);
    }

    #[test]
    fn import_stops_on_disconnect() {
        let plan = plan_import(&sample(), &snapshot68()).unwrap();
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        assert_eq!(import_job(&mut kb, &plan), Err(Error::Disconnected));
    }

    #[test]
    fn export_reads_keys_and_leaves_out_unreadable_lighting() {
        let mut m = MockTransport::new();
        for (layout, v) in [
            (8, 0x0020), // Mode: rapid trigger, no function
            (4, 1200),
            (20, 150),
            (21, 200),
            (22, 100),
            (23, 200),
            (25, 2),
            (0, 0x1A),
            (1, 0x3A),
            (2, 0),
            (3, 0x01),
            (8, 0x0020), // advanced(): Mode again
        ] {
            m.push_reply(key_reply(0x1A, layout, v));
        }
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        let out = export_job(&mut kb, &[0x1A]).unwrap();
        assert_eq!(
            out.data.keys,
            vec![key(0x1A, TouchMode::RapidTrigger, AdvancedConfig::None)]
        );
        assert_eq!(out.data.lighting, None);
        assert_eq!(out.skipped.len(), 2);
        // The export still makes a valid file.
        let back = ProfileFile::from_json(&ProfileFile::from_data(&out.data).to_json())
            .unwrap()
            .to_data()
            .unwrap();
        assert_eq!(back, out.data);
    }
}

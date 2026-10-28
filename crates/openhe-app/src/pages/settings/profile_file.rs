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
use openhe_device::{AdvancedConfig, Error, KeySettings, Keyboard, PerformanceChange, Transport};
use openhe_proto::advanced::{Dks, End, Mpt, Mt, Socd, SocdMode, Tgl};
use openhe_proto::key::{Advanced, KeyMode};
use openhe_proto::lighting::{Lighting, Rgb, COLOR_SLOTS};
use openhe_proto::{Layer, Micrometers, Model, TouchMode};
use serde::{Deserialize, Serialize};

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
                };
                lighting
                    .validate()
                    .map_err(|e| FileError::BadValue(e.to_string()))?;
                Some(lighting)
            }
        };
        Ok(ProfileData {
            model,
            keys,
            lighting,
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

/// Plans an import of `data` onto the keyboard in `snap`. Refuses a file
/// for another model.
pub fn plan_import(data: &ProfileData, snap: &DeviceSnapshot) -> Result<ImportPlan, FileError> {
    if data.model != snap.model {
        return Err(FileError::WrongModel {
            file: data.model.name().to_string(),
            keyboard: snap.model.name().to_string(),
        });
    }
    let present: std::collections::BTreeSet<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
    let mut plan = ImportPlan::default();
    let mut socd_done: Vec<(u8, u8)> = Vec::new();
    for k in &data.keys {
        if !present.contains(&k.code) {
            plan.skipped_unknown.push(k.code);
            continue;
        }
        let change = perf_change(&k.settings);
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
        record(ImportStep::Lighting, kb.set_lighting(l))?;
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
                        bottom: um(3300),
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

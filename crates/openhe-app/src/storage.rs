//! Local app data (macro library, profile names, per-app rules, autostart),
//! stored as JSON under `ProjectDirs("dev", "OpenHE", "OpenHE")`.
//!
//! Nothing here is sent anywhere; the file lives in the user's config dir.

use crate::skin::KeyboardSkin;
use openhe_proto::macros::{MacroAction, PlayMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// File name inside the config directory.
pub const FILE_NAME: &str = "openhe.json";

/// Why [`AppData::save`] refused to write (Indonesian, shown to the user).
pub const WRITE_REFUSED: &str =
    "File data OpenHE tidak bisa dibaca saat dibuka, jadi perubahan tidak disimpan agar file lama tidak tertimpa.";

/// What happened to the data file at startup, when it was not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadNotice {
    /// The file did not parse; it was moved aside and defaults are used.
    Recovered { moved_to: PathBuf },
    /// The file exists but could not be read (permissions, I/O); defaults
    /// are used and the file was left in place.
    Unreadable(String),
}

/// `openhe.json.bad`, or `.bad.2`, `.bad.3`, ... if taken.
fn free_bad_path(dir: &Path) -> PathBuf {
    let first = dir.join(format!("{FILE_NAME}.bad"));
    if !first.exists() {
        return first;
    }
    (2u32..)
        .map(|n| dir.join(format!("{FILE_NAME}.bad.{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Defaults for a data file that could not be read or moved aside: saving
/// is blocked so the file stays as it is.
fn blocked_defaults() -> AppData {
    let mut d = AppData::default();
    d.set_save_blocked(true);
    d
}

/// One macro step. A serde mirror of `openhe_proto::macros::MacroAction`
/// (Track A2), kept separate so the proto crate needs no serde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedMacroAction {
    pub code: u16,
    pub down: bool,
    pub delay_ms: u32,
}

impl From<MacroAction> for SavedMacroAction {
    fn from(a: MacroAction) -> Self {
        SavedMacroAction {
            code: a.code,
            down: a.down,
            delay_ms: a.delay_ms,
        }
    }
}

impl From<SavedMacroAction> for MacroAction {
    fn from(a: SavedMacroAction) -> Self {
        MacroAction {
            code: a.code,
            down: a.down,
            delay_ms: a.delay_ms,
        }
    }
}

/// A macro in the local library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedMacro {
    pub name: String,
    pub actions: Vec<SavedMacroAction>,
    /// `openhe_proto::macros::PlayMode` as its wire value (0..=3).
    pub mode: u8,
    pub repeat: u16,
    pub interval_ms: u32,
}

impl SavedMacro {
    /// An empty macro: play once, repeat 1, 100 ms interval (the official
    /// app's defaults).
    pub fn new(name: impl Into<String>) -> SavedMacro {
        SavedMacro {
            name: name.into(),
            actions: Vec::new(),
            mode: PlayMode::Once as u8,
            repeat: 1,
            interval_ms: 100,
        }
    }

    pub fn to_actions(&self) -> Vec<MacroAction> {
        self.actions.iter().copied().map(Into::into).collect()
    }

    /// The stored mode; an unknown value reads as `Once`.
    pub fn play_mode(&self) -> PlayMode {
        PlayMode::from_u8(self.mode).unwrap_or(PlayMode::Once)
    }

    /// The stored repeat count; 0 (an old or hand-edited file) reads as 1,
    /// the smallest value the keyboard accepts.
    pub fn repeat_count(&self) -> u16 {
        self.repeat.max(1)
    }

    /// Sum of all step delays, in milliseconds.
    pub fn total_ms(&self) -> u64 {
        self.actions.iter().map(|a| u64::from(a.delay_ms)).sum()
    }

    pub fn duplicate_named(&self, name: impl Into<String>) -> SavedMacro {
        SavedMacro {
            name: name.into(),
            ..self.clone()
        }
    }
}

/// Everything the app keeps locally. Missing fields load as defaults, so
/// older files keep working when fields are added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppData {
    pub macros: Vec<SavedMacro>,
    /// Names for onboard profiles 1..=4; empty means unnamed.
    pub profile_names: [String; 4],
    /// `(app name, profile index 0..=3)`.
    pub app_profile_rules: Vec<(String, u8)>,
    pub launch_at_login: bool,
    /// Per-app automatic profile switching is on (restarted when the app
    /// opens).
    #[serde(default)]
    pub auto_profile_enabled: bool,
    /// When the last calibration was saved on the keyboard (Unix seconds).
    #[serde(default)]
    pub last_calibrated: Option<u64>,
    /// Key cap colours per keyboard ([`crate::skin::skin_key`]: the
    /// serial), picked by the user; the keyboard does not report them.
    /// Read leniently: an unknown entry reads as the dark skin.
    #[serde(default, deserialize_with = "crate::skin::deserialize_skins")]
    pub keyboard_skins: BTreeMap<String, KeyboardSkin>,
    /// Key (in `keyboard_skins`) of the keyboard last connected, so its
    /// colours show while it is unplugged.
    #[serde(default)]
    pub last_keyboard: Option<String>,
    /// Set when the data file existed but could not be read or moved aside
    /// at load. [`AppData::save`] then refuses to write, so that file is
    /// never replaced by defaults. Not stored in the file; only this crate
    /// sets it.
    #[serde(skip)]
    save_blocked: bool,
}

impl AppData {
    /// Saving is refused (see the `save_blocked` field).
    pub fn save_blocked(&self) -> bool {
        self.save_blocked
    }

    /// Blocks or allows saving (the loader blocks it for a file it could
    /// not read; tests block it so the real data file is never touched).
    pub(crate) fn set_save_blocked(&mut self, on: bool) {
        self.save_blocked = on;
    }

    /// The OS config directory for OpenHE, if the platform has one.
    pub fn default_dir() -> Option<PathBuf> {
        // Tests point this at a private folder: on Windows the per-user
        // folders come from the shell, not from HOME or APPDATA.
        if let Some(dir) = std::env::var_os("OPENHE_DATA_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("dev", "OpenHE", "OpenHE")
            .map(|d| d.config_dir().to_path_buf())
    }

    /// Loads from `dir`. A missing file gives the defaults.
    pub fn load_from(dir: &Path) -> io::Result<AppData> {
        match std::fs::read(dir.join(FILE_NAME)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(AppData::default()),
            Err(e) => Err(e),
        }
    }

    /// Saves to `dir` (created if needed), via a temp file and a rename so a
    /// crash never leaves a half-written file.
    ///
    /// Refuses (`PermissionDenied`, text [`WRITE_REFUSED`]) when
    /// [`AppData::save_blocked`] is true, without touching the disk.
    pub fn save_to(&self, dir: &Path) -> io::Result<()> {
        if self.save_blocked {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                WRITE_REFUSED,
            ));
        }
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, dir.join(FILE_NAME))
    }

    /// Loads from `dir` for the app: never fails. A file that does not
    /// parse is renamed to `openhe.json.bad` (or `.bad.2`, ...) so the next
    /// save cannot overwrite it, and the defaults are used; a file that
    /// cannot be read is left alone. Either way the notice says what
    /// happened, for the shell to show.
    pub fn load_or_recover(dir: &Path) -> (AppData, Option<LoadNotice>) {
        match Self::load_from(dir) {
            Ok(d) => (d, None),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let from = dir.join(FILE_NAME);
                let to = free_bad_path(dir);
                match std::fs::rename(&from, &to) {
                    Ok(()) => (
                        AppData::default(),
                        Some(LoadNotice::Recovered { moved_to: to }),
                    ),
                    Err(e) => (
                        blocked_defaults(),
                        Some(LoadNotice::Unreadable(e.to_string())),
                    ),
                }
            }
            Err(e) => (
                blocked_defaults(),
                Some(LoadNotice::Unreadable(e.to_string())),
            ),
        }
    }

    /// [`AppData::load_or_recover`] in [`AppData::default_dir`].
    pub fn load() -> (AppData, Option<LoadNotice>) {
        match Self::default_dir() {
            Some(dir) => Self::load_or_recover(&dir),
            None => (AppData::default(), None),
        }
    }

    /// Saves to [`AppData::default_dir`].
    pub fn save(&self) -> io::Result<()> {
        let dir = Self::default_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
        self.save_to(&dir)
    }

    /// The skin of keyboard `key` ([`crate::skin::skin_key`]), if one was
    /// chosen.
    pub fn skin_of(&self, key: &str) -> Option<KeyboardSkin> {
        self.keyboard_skins.get(key).copied()
    }

    /// The skin to draw: that of `key` (the keyboard connected now), else
    /// of the keyboard last connected, else the dark default.
    pub fn skin_for(&self, key: Option<&str>) -> KeyboardSkin {
        key.or(self.last_keyboard.as_deref())
            .and_then(|k| self.skin_of(k))
            .unwrap_or_default()
    }

    /// Stores `skin` for keyboard `key` (also as the last keyboard).
    pub fn set_skin(&mut self, key: &str, skin: KeyboardSkin) {
        self.keyboard_skins.insert(key.to_string(), skin);
        self.last_keyboard = Some(key.to_string());
    }

    /// Display name for profile `idx` (0-based): "1 · Kerja" or just "1".
    pub fn profile_label(&self, idx: u8) -> String {
        let n = idx + 1;
        match self.profile_names.get(usize::from(idx)) {
            Some(name) if !name.trim().is_empty() => format!("{n} · {}", name.trim()),
            _ => n.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir()
                .join(format!("openhe-test-{tag}-{}-{nanos}", std::process::id()));
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> AppData {
        AppData {
            macros: vec![SavedMacro {
                name: "Salin".into(),
                actions: vec![
                    SavedMacroAction {
                        code: 0xE0,
                        down: true,
                        delay_ms: 0,
                    },
                    SavedMacroAction {
                        code: 0x06,
                        down: true,
                        delay_ms: 35,
                    },
                    SavedMacroAction {
                        code: 0x06,
                        down: false,
                        delay_ms: 20,
                    },
                ],
                mode: 2,
                repeat: 1,
                interval_ms: 0,
            }],
            profile_names: [
                "Kerja".into(),
                "Gaming".into(),
                String::new(),
                String::new(),
            ],
            app_profile_rules: vec![("Valorant".into(), 1)],
            launch_at_login: true,
            ..AppData::default()
        }
    }

    #[test]
    fn round_trip_through_a_directory() {
        let dir = TempDir::new("roundtrip");
        let data = sample();
        data.save_to(&dir.0.join("nested")).unwrap();
        assert_eq!(AppData::load_from(&dir.0.join("nested")).unwrap(), data);
        assert!(!dir.0.join("nested").join("openhe.json.tmp").exists());
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = TempDir::new("missing");
        assert_eq!(AppData::load_from(&dir.0).unwrap(), AppData::default());
    }

    #[test]
    fn missing_fields_load_as_defaults() {
        let dir = TempDir::new("partial");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), br#"{"launch_at_login":true}"#).unwrap();
        let d = AppData::load_from(&dir.0).unwrap();
        assert!(d.launch_at_login);
        assert!(d.macros.is_empty());
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_reset() {
        let dir = TempDir::new("corrupt");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), b"{not json").unwrap();
        let e = AppData::load_from(&dir.0).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn nested_fields_default_too() {
        let dir = TempDir::new("nested-default");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(
            dir.0.join(FILE_NAME),
            br#"{"macros":[{"name":"x","actions":[{"code":4}]}]}"#,
        )
        .unwrap();
        let d = AppData::load_from(&dir.0).unwrap();
        assert_eq!(d.macros[0].name, "x");
        assert_eq!(d.macros[0].repeat, 0);
        assert_eq!(
            d.macros[0].actions[0],
            SavedMacroAction {
                code: 4,
                down: false,
                delay_ms: 0
            }
        );
    }

    #[test]
    fn corrupt_file_is_moved_aside_not_overwritten() {
        let dir = TempDir::new("recover");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), b"{not json").unwrap();
        let (d, notice) = AppData::load_or_recover(&dir.0);
        assert_eq!(d, AppData::default());
        let bad = dir.0.join("openhe.json.bad");
        assert_eq!(
            notice,
            Some(LoadNotice::Recovered {
                moved_to: bad.clone()
            })
        );
        assert_eq!(std::fs::read(&bad).unwrap(), b"{not json");
        assert!(!dir.0.join(FILE_NAME).exists());
        // Saving now cannot touch the moved file; a second bad file gets .bad.2.
        sample().save_to(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), br#"{"macros": 5}"#).unwrap();
        let (_, notice) = AppData::load_or_recover(&dir.0);
        assert_eq!(
            notice,
            Some(LoadNotice::Recovered {
                moved_to: dir.0.join("openhe.json.bad.2")
            })
        );
        assert_eq!(std::fs::read(&bad).unwrap(), b"{not json");
    }

    #[test]
    fn good_and_missing_files_give_no_notice() {
        let dir = TempDir::new("no-notice");
        assert_eq!(AppData::load_or_recover(&dir.0), (AppData::default(), None));
        sample().save_to(&dir.0).unwrap();
        assert_eq!(AppData::load_or_recover(&dir.0), (sample(), None));
    }

    #[test]
    fn unreadable_file_blocks_saving_and_is_left_untouched() {
        let dir = TempDir::new("unreadable");
        // A directory where the file should be: reading fails with an error
        // that is neither "not found" nor "invalid data".
        std::fs::create_dir_all(dir.0.join(FILE_NAME)).unwrap();
        let (d, notice) = AppData::load_or_recover(&dir.0);
        assert!(matches!(notice, Some(LoadNotice::Unreadable(_))));
        assert!(d.save_blocked());
        let err = d.save_to(&dir.0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains(WRITE_REFUSED));
        // Nothing was written or replaced.
        assert!(dir.0.join(FILE_NAME).is_dir());
        assert!(!dir.0.join("openhe.json.tmp").exists());
    }

    #[test]
    fn recovered_and_missing_files_can_be_saved() {
        let dir = TempDir::new("recovered-save");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), b"{not json").unwrap();
        let (d, notice) = AppData::load_or_recover(&dir.0);
        assert!(matches!(notice, Some(LoadNotice::Recovered { .. })));
        assert!(!d.save_blocked());
        d.save_to(&dir.0).unwrap();
        let fresh = TempDir::new("missing-save");
        let (d, notice) = AppData::load_or_recover(&fresh.0);
        assert_eq!((notice, d.save_blocked()), (None, false));
        d.save_to(&fresh.0).unwrap();
    }

    #[test]
    fn save_blocked_flag_is_never_written_to_disk() {
        let dir = TempDir::new("flag-not-saved");
        sample().save_to(&dir.0).unwrap();
        let text = std::fs::read_to_string(dir.0.join(FILE_NAME)).unwrap();
        assert!(!text.contains("save_blocked"));
    }

    #[test]
    fn auto_switching_and_last_calibration_are_stored() {
        let dir = TempDir::new("new-fields");
        // An older file without them: off / never.
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(FILE_NAME), br#"{"launch_at_login":true}"#).unwrap();
        let d = AppData::load_from(&dir.0).unwrap();
        assert!(!d.auto_profile_enabled);
        assert_eq!(d.last_calibrated, None);
        let d = AppData {
            auto_profile_enabled: true,
            last_calibrated: Some(1_793_000_000),
            ..sample()
        };
        d.save_to(&dir.0).unwrap();
        assert_eq!(AppData::load_from(&dir.0).unwrap(), d);
    }

    #[test]
    fn keyboard_skins_round_trip_and_old_files_load() {
        let dir = TempDir::new("skins");
        std::fs::create_dir_all(&dir.0).unwrap();
        // A file from before skins existed.
        std::fs::write(
            dir.0.join(FILE_NAME),
            br#"{"launch_at_login":true,"profile_names":["a","","",""]}"#,
        )
        .unwrap();
        let d = AppData::load_from(&dir.0).unwrap();
        assert!(d.keyboard_skins.is_empty());
        assert_eq!(d.profile_names[0], "a");
        let mut d = sample();
        d.keyboard_skins
            .insert("A68M240193000000".into(), KeyboardSkin::BlackRed);
        d.keyboard_skins.insert(
            "model:Win60HeMax".into(),
            KeyboardSkin::custom([0xD9, 0xD4, 0xC7], [0x3A, 0x3F, 0x4B], [0xE2, 0x72, 0x5B]),
        );
        d.save_to(&dir.0).unwrap();
        assert_eq!(AppData::load_from(&dir.0).unwrap(), d);
        // A skin from a newer version does not break the file.
        std::fs::write(
            dir.0.join(FILE_NAME),
            br#"{"macros":[],"keyboard_skins":{"X":{"kind":"hologram"},"Y":{"kind":"grey_yellow"}}}"#,
        )
        .unwrap();
        let d = AppData::load_from(&dir.0).unwrap();
        assert_eq!(d.keyboard_skins["X"], KeyboardSkin::Gelap);
        assert_eq!(d.keyboard_skins["Y"], KeyboardSkin::GreyYellow);
    }

    #[test]
    fn skin_lookup_falls_back_to_the_last_keyboard_then_gelap() {
        let mut d = AppData::default();
        assert_eq!(d.skin_for(Some("A")), KeyboardSkin::Gelap);
        assert_eq!(d.skin_for(None), KeyboardSkin::Gelap);
        assert_eq!(d.skin_of("A"), None);
        d.set_skin("A", KeyboardSkin::WhiteBlue);
        assert_eq!(d.skin_of("A"), Some(KeyboardSkin::WhiteBlue));
        assert_eq!(d.last_keyboard.as_deref(), Some("A"));
        // Unplugged: the last keyboard's colours.
        assert_eq!(d.skin_for(None), KeyboardSkin::WhiteBlue);
        // Another keyboard without a choice: the default.
        assert_eq!(d.skin_for(Some("B")), KeyboardSkin::Gelap);
    }

    #[test]
    fn save_blocked_is_set_only_inside_the_crate() {
        let mut d = AppData::default();
        assert!(!d.save_blocked());
        d.set_save_blocked(true);
        assert!(d.save_blocked());
        let dir = TempDir::new("blocked-setter");
        assert_eq!(
            d.save_to(&dir.0).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!dir.0.exists());
    }

    #[test]
    fn saved_macro_converts_to_and_from_proto_actions() {
        let m = sample().macros.remove(0);
        let actions = m.to_actions();
        assert_eq!(
            actions[1],
            MacroAction {
                code: 0x06,
                down: true,
                delay_ms: 35
            }
        );
        assert_eq!(m.play_mode(), PlayMode::WhileHeld);
        let back: Vec<SavedMacroAction> = actions.into_iter().map(Into::into).collect();
        assert_eq!(back, m.actions);
    }

    #[test]
    fn saved_macro_helpers() {
        let m = SavedMacro::new("Baru");
        assert_eq!((m.repeat, m.interval_ms, m.mode), (1, 100, 0));
        assert!(m.actions.is_empty());
        let s = sample().macros.remove(0);
        assert_eq!(s.total_ms(), 55);
        // Unknown stored mode falls back to Once; repeat 0 is sent as 1.
        let odd = SavedMacro {
            mode: 9,
            repeat: 0,
            ..s.clone()
        };
        assert_eq!(odd.play_mode(), PlayMode::Once);
        assert_eq!(odd.repeat_count(), 1);
        let copy = s.duplicate_named("Salin (salinan)");
        assert_eq!(copy.name, "Salin (salinan)");
        assert_eq!(copy.actions, s.actions);
    }

    #[test]
    fn profile_labels() {
        let d = sample();
        assert_eq!(d.profile_label(0), "1 · Kerja");
        assert_eq!(d.profile_label(2), "3");
    }
}

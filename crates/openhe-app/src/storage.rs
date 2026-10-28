//! Local app data (macro library, profile names, per-app rules, autostart),
//! stored as JSON under `ProjectDirs("dev", "OpenHE", "OpenHE")`.
//!
//! Nothing here is sent anywhere; the file lives in the user's config dir.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// File name inside the config directory.
pub const FILE_NAME: &str = "openhe.json";

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

/// One macro step. A serde mirror of `openhe_proto::macros::MacroAction`
/// (Track A2), kept separate so the proto crate needs no serde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedMacroAction {
    pub code: u16,
    pub down: bool,
    pub delay_ms: u32,
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
}

impl AppData {
    /// The OS config directory for OpenHE, if the platform has one.
    pub fn default_dir() -> Option<PathBuf> {
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
    pub fn save_to(&self, dir: &Path) -> io::Result<()> {
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
                        AppData::default(),
                        Some(LoadNotice::Unreadable(e.to_string())),
                    ),
                }
            }
            Err(e) => (
                AppData::default(),
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
    fn profile_labels() {
        let d = sample();
        assert_eq!(d.profile_label(0), "1 · Kerja");
        assert_eq!(d.profile_label(2), "3");
    }
}

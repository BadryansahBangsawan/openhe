//! Local files of the Template page, under the app data directory
//! ([`crate::storage::AppData::default_dir`]):
//!
//! - `templates/*.openhe-template`: the user's own templates;
//! - `backups/backup-<unix ms>.json` (`backup-<unix ms>-2.json`, … when
//!   taken): the key state read before each apply (a profile file with a
//!   small header); the newest [`KEEP_BACKUPS`] of each keyboard are kept.
//!
//! Nothing here is sent anywhere.

use super::{Template, TemplateFile, EXTENSION};
use crate::pages::settings::profile_file::{KeyData, ProfileData, ProfileFile};
use openhe_proto::Model;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TEMPLATES_DIR: &str = "templates";
pub const BACKUPS_DIR: &str = "backups";
/// Backups kept per keyboard (model and serial); older ones are deleted.
pub const KEEP_BACKUPS: usize = 10;
pub const BACKUP_FORMAT: &str = "openhe-backup";
pub const BACKUP_VERSION: u32 = 1;

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Writes via a temp file and a rename, so a crash never leaves half a file.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// `base`/`name`, or `base`/`name-2`, … when taken.
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2u32..)
        .map(|n| dir.join(format!("{stem}-{n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// A file name from a template name: letters and digits, others as "-".
pub fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "template".to_string()
    } else {
        s
    }
}

// ------------------------------------------------------------ templates

/// A template in the user's library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    pub template: Template,
}

/// Saves `t` as a new file in `base`/templates.
pub fn save_template(base: &Path, t: &Template) -> io::Result<PathBuf> {
    let dir = base.join(TEMPLATES_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = free_path(&dir, &slug(&t.name), EXTENSION);
    write_atomic(&path, &TemplateFile::from_template(t).to_json())?;
    Ok(path)
}

/// The library, sorted by name. Files that do not parse are left out (and
/// left alone).
pub fn list_templates(base: &Path) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(base.join(TEMPLATES_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let template = super::parse(&text).ok()?;
            Some(Saved { path, template })
        })
        .collect();
    out.sort_by(|a, b| {
        a.template
            .name
            .to_lowercase()
            .cmp(&b.template.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

pub fn delete_template(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

// --------------------------------------------------------------- backups

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub version: u32,
    pub created_ms: u64,
    /// What was about to be applied ("Gaming FPS").
    pub reason: String,
    /// Onboard profile (0-based) the keys were read from.
    pub profile: u8,
    pub serial: String,
    pub profile_file: ProfileFile,
}

/// A backup read back from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    pub created_ms: u64,
    pub reason: String,
    pub profile: u8,
    pub model: Model,
    pub serial: String,
    pub keys: Vec<KeyData>,
}

impl BackupFile {
    pub fn new(
        reason: &str,
        profile: u8,
        serial: &str,
        model: Model,
        keys: &[KeyData],
        created_ms: u64,
    ) -> BackupFile {
        BackupFile {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_VERSION,
            created_ms,
            reason: reason.to_string(),
            profile,
            serial: serial.to_string(),
            profile_file: ProfileFile::from_data(&ProfileData {
                model,
                keys: keys.to_vec(),
                lighting: None,
                key_colors: Vec::new(),
                keep_lighting_flags: false,
            }),
        }
    }

    fn to_backup(&self, path: PathBuf) -> Option<Backup> {
        if self.format != BACKUP_FORMAT || self.version > BACKUP_VERSION {
            return None;
        }
        let data = self.profile_file.to_data().ok()?;
        Some(Backup {
            path,
            created_ms: self.created_ms,
            reason: self.reason.clone(),
            profile: self.profile,
            model: data.model,
            serial: self.serial.clone(),
            keys: data.keys,
        })
    }
}

/// Saves a backup in `base`/backups, then deletes all but the newest
/// [`KEEP_BACKUPS`] of the same keyboard. Other keyboards' backups and
/// files that do not parse are left alone.
pub fn save_backup(base: &Path, b: &BackupFile) -> io::Result<PathBuf> {
    let dir = base.join(BACKUPS_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = free_path(&dir, &format!("backup-{:013}", b.created_ms), "json");
    let json = serde_json::to_string_pretty(b).map_err(io::Error::other)?;
    write_atomic(&path, &json)?;
    if let Some(new) = read_backup(&path) {
        for old in list_backups(base, new.model, &new.serial)
            .into_iter()
            .skip(KEEP_BACKUPS)
        {
            if old.path != path {
                let _ = std::fs::remove_file(old.path);
            }
        }
    }
    Ok(path)
}

/// The `-n` [`free_path`] added to a backup file name taken in the same
/// millisecond (0 without one): later saves get higher numbers.
fn collision_suffix(path: &Path) -> u32 {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("backup-"))
        .and_then(|s| s.split_once('-'))
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(0)
}

/// Newest first: by the time stored in the file, then by the collision
/// suffix (`backup-<ms>-2.json` is newer than `backup-<ms>.json`).
fn newest_first(a: &Backup, b: &Backup) -> Ordering {
    (b.created_ms, collision_suffix(&b.path)).cmp(&(a.created_ms, collision_suffix(&a.path)))
}

fn backup_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("backup-"))
        })
        .collect()
}

pub fn read_backup(path: &Path) -> Option<Backup> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: BackupFile = serde_json::from_str(&text).ok()?;
    file.to_backup(path.to_path_buf())
}

/// The backups of the keyboard `model` / `serial`, newest first.
pub fn list_backups(base: &Path, model: Model, serial: &str) -> Vec<Backup> {
    let mut out: Vec<Backup> = backup_files(&base.join(BACKUPS_DIR))
        .iter()
        .filter_map(|p| read_backup(p))
        .filter(|b| b.model == model && b.serial == serial)
        .collect();
    out.sort_by(newest_first);
    out
}

/// "baru saja", "5 menit lalu", "2 jam lalu", "3 hari lalu" (no time zone
/// needed).
pub fn age_text(created_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(created_ms) / 1000;
    match s {
        0..=59 => "baru saja".to_string(),
        60..=3599 => format!("{} menit lalu", s / 60),
        3600..=86_399 => format!("{} jam lalu", s / 3600),
        _ => format!("{} hari lalu", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::snapshot68;
    use crate::templates::tests::factory_keys;
    use crate::templates::Builtin;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!(
                "openhe-tpl-{tag}-{}-{}",
                std::process::id(),
                now_ms()
            ));
            let _ = std::fs::remove_dir_all(&p);
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn templates_are_saved_listed_and_deleted() {
        let dir = TempDir::new("lib");
        assert!(list_templates(&dir.0).is_empty());
        let snap = snapshot68();
        let keys: Vec<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
        let mut t = Builtin::GamingFps.template(snap.model, &keys);
        t.name = "Valorant / CS".into();
        let p1 = save_template(&dir.0, &t).unwrap();
        assert!(
            p1.ends_with("templates/valorant-cs.openhe-template"),
            "{p1:?}"
        );
        let p2 = save_template(&dir.0, &t).unwrap();
        assert_ne!(p1, p2, "a second save does not overwrite");
        std::fs::write(dir.0.join(TEMPLATES_DIR).join("junk.openhe-template"), "{").unwrap();
        let list = list_templates(&dir.0);
        assert_eq!(list.len(), 2, "the broken file is left out");
        assert_eq!(list[0].template.name, "Valorant / CS");
        delete_template(&p1).unwrap();
        assert_eq!(list_templates(&dir.0).len(), 1);
    }

    #[test]
    fn backups_round_trip_and_only_the_newest_ten_stay() {
        let dir = TempDir::new("bak");
        let snap = snapshot68();
        let keys = factory_keys(&snap);
        for i in 0..12u64 {
            let b = BackupFile::new("Gaming FPS", 1, "A68", snap.model, &keys, 1_000 + i);
            save_backup(&dir.0, &b).unwrap();
        }
        let list = list_backups(&dir.0, snap.model, "A68");
        assert_eq!(list.len(), KEEP_BACKUPS);
        assert_eq!(list[0].created_ms, 1_011, "newest first");
        assert_eq!(list.last().unwrap().created_ms, 1_002);
        assert_eq!(list[0].keys, keys, "keys come back exactly");
        assert_eq!(list[0].profile, 1);
        assert!(list_backups(&dir.0, snap.model, "other").is_empty());
        assert!(list_backups(&dir.0, Model::Win60HeMax, "A68").is_empty());
    }

    /// Ten are kept per keyboard: a second keyboard's backups never push
    /// the first one's out. Backups saved in the same millisecond keep
    /// their save order (`-2` is newer), not the file names' byte order.
    #[test]
    fn backups_are_pruned_per_keyboard_in_save_order() {
        let dir = TempDir::new("prune");
        let snap = snapshot68();
        let keys = factory_keys(&snap);
        let save = |serial: &str, reason: &str, ms: u64| {
            let b = BackupFile::new(reason, 0, serial, snap.model, &keys, ms);
            save_backup(&dir.0, &b).unwrap()
        };
        for i in 0..KEEP_BACKUPS as u64 {
            save("A68", "a", 1_000 + i);
        }
        for i in 0..KEEP_BACKUPS as u64 + 3 {
            save("B68", "b", 2_000 + i);
        }
        assert_eq!(list_backups(&dir.0, snap.model, "A68").len(), KEEP_BACKUPS);
        let b = list_backups(&dir.0, snap.model, "B68");
        assert_eq!(b.len(), KEEP_BACKUPS);
        assert_eq!(b.last().unwrap().created_ms, 2_003, "B's oldest went");
        // Three in one millisecond: backup-…, backup-…-2, backup-…-3.
        let p1 = save("A68", "first", 5_000);
        let p2 = save("A68", "second", 5_000);
        let p3 = save("A68", "third", 5_000);
        assert!(p2.to_string_lossy().ends_with("-2.json"), "{p2:?}");
        let a = list_backups(&dir.0, snap.model, "A68");
        let reasons: Vec<&str> = a.iter().take(3).map(|b| b.reason.as_str()).collect();
        assert_eq!(reasons, ["third", "second", "first"]);
        assert_eq!(a.len(), KEEP_BACKUPS);
        assert!(p1.exists() && p2.exists() && p3.exists(), "the newest stay");
        assert_eq!(a.last().unwrap().created_ms, 1_003);
        // One more in that millisecond: the oldest A goes, not "first".
        save("A68", "fourth", 5_000);
        let a = list_backups(&dir.0, snap.model, "A68");
        assert_eq!(a[0].reason, "fourth");
        assert_eq!(a.last().unwrap().created_ms, 1_004);
        assert!(p1.exists());
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(age_text(0, 30_000), "baru saja");
        assert_eq!(age_text(0, 5 * 60_000), "5 menit lalu");
        assert_eq!(age_text(0, 2 * 3_600_000), "2 jam lalu");
        assert_eq!(age_text(0, 3 * 86_400_000), "3 hari lalu");
        assert_eq!(slug("  "), "template");
    }
}

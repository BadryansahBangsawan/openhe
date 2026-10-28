//! Single-instance guard: only one OpenHE may talk to the keyboard at a
//! time (two apps would interleave their reports and read each other's
//! replies).
//!
//! The first instance holds an advisory exclusive lock on `openhe.lock` in
//! the config directory. The OS releases it when the process ends, also
//! after a crash, so a stale file never blocks the next start.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Lock file name inside the config directory.
pub const LOCK_FILE: &str = "openhe.lock";

/// Held for as long as the app runs; dropping it releases the lock.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

/// The outcome of [`acquire`].
#[derive(Debug)]
pub enum Acquire {
    /// This is the only instance; keep the lock alive until exit.
    Acquired(InstanceLock),
    /// Another instance holds the lock.
    AlreadyRunning,
    /// The lock file could not be created or locked (no config directory,
    /// a read-only disk, ...). The app starts anyway rather than refusing
    /// to run.
    Unavailable(io::Error),
}

/// Tries to become the only instance, with the lock file in `dir`
/// (created if needed).
pub fn acquire(dir: &Path) -> Acquire {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Acquire::Unavailable(e);
    }
    let file = match OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))
    {
        Ok(f) => f,
        Err(e) => return Acquire::Unavailable(e),
    };
    // Fully qualified: newer std has an inherent `File::try_lock` with
    // another error type, while the workspace builds on Rust 1.85 too.
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => Acquire::Acquired(InstanceLock { _file: file }),
        Err(fs4::TryLockError::WouldBlock) => Acquire::AlreadyRunning,
        Err(fs4::TryLockError::Error(e)) => Acquire::Unavailable(e),
    }
}

/// [`acquire`] in the app's config directory (where `openhe.json` lives).
pub fn acquire_default() -> Acquire {
    match crate::storage::AppData::default_dir() {
        Some(dir) => acquire(&dir),
        None => Acquire::Unavailable(io::Error::new(
            io::ErrorKind::NotFound,
            "no config directory",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("openhe-lock-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn a_second_instance_is_refused_until_the_first_ends() {
        let dir = temp_dir("second");
        let first = match acquire(&dir) {
            Acquire::Acquired(l) => l,
            other => panic!("{other:?}"),
        };
        assert!(dir.join(LOCK_FILE).exists());
        assert!(matches!(acquire(&dir), Acquire::AlreadyRunning));
        drop(first);
        assert!(matches!(acquire(&dir), Acquire::Acquired(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lock_file_left_behind_does_not_block() {
        // A crash leaves the file, but not the lock.
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCK_FILE), b"12345").unwrap();
        assert!(matches!(acquire(&dir), Acquire::Acquired(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_usable_directory_still_lets_the_app_start() {
        let dir = temp_dir("blocked");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, b"a file, not a directory").unwrap();
        assert!(matches!(acquire(&dir), Acquire::Unavailable(_)));
        let _ = std::fs::remove_file(&dir);
    }
}

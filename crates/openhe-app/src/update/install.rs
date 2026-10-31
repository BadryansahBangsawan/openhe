//! Putting a downloaded, verified update in place, per OS; cleaning up
//! after it on the next start; restarting into it.
//!
//! The decisions ([`plan_target`], the paths) are pure functions; the file
//! work is a few renames that tests run on plain folders and files. All of
//! it happens next to the installed app, on the same volume, so every
//! rename is atomic:
//!
//! - macOS: `…/OpenHE.app` (found from the running executable). The zip is
//!   unpacked with `ditto` into a staging folder beside the bundle, its
//!   version checked, then `OpenHE.app` → `OpenHE.app.old-update` and the
//!   new bundle → `OpenHE.app` (renamed back if that fails).
//! - Windows: the running `.exe` may be renamed (not overwritten):
//!   `openhe.exe` → `openhe.exe.old`, the new file → `openhe.exe`.
//! - Linux: only an AppImage (`$APPIMAGE`): the new file is made
//!   executable and renamed over it (the running copy stays mounted).
//!
//! Anywhere else (a `cargo run` build, a plain Linux binary) self-update is
//! not offered; the card links to the release page instead.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the staging folder beside the installed app.
pub const STAGING_DIR: &str = ".openhe-update";
/// Suffix of the previous macOS bundle (`OpenHE.app.old-update`).
pub const OLD_BUNDLE_SUFFIX: &str = ".old-update";
/// Suffix of the previous Windows executable (`openhe-app.exe.old`).
pub const OLD_EXE_SUFFIX: &str = ".old";
/// The bundle name inside the macOS release zip.
pub const ZIP_BUNDLE: &str = "OpenHE.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

/// What an update replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    /// The macOS bundle, `…/OpenHE.app`.
    MacBundle(PathBuf),
    /// The Windows executable.
    WindowsExe(PathBuf),
    /// The Linux AppImage file.
    AppImage(PathBuf),
}

/// Why this copy of OpenHE cannot update itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsupported {
    /// macOS, but not running from an `.app` bundle (`cargo run`).
    NotInBundle,
    /// macOS runs the bundle from a read-only random location (App
    /// Translocation, for an app opened from Downloads).
    Translocated,
    /// Linux, but not an AppImage.
    NotAppImage,
    /// No releases for this OS.
    Platform,
    /// The running executable could not be found.
    NoExe,
}

/// Where this copy can be updated, from the running executable and
/// `$APPIMAGE`.
pub fn plan_target(
    os: Os,
    exe: Option<&Path>,
    appimage: Option<&Path>,
) -> Result<InstallTarget, Unsupported> {
    match os {
        Os::Linux => match appimage {
            Some(p) if !p.as_os_str().is_empty() => Ok(InstallTarget::AppImage(p.to_path_buf())),
            _ => Err(Unsupported::NotAppImage),
        },
        Os::Mac => {
            let exe = exe.ok_or(Unsupported::NoExe)?;
            if exe
                .components()
                .any(|c| c.as_os_str() == "AppTranslocation")
            {
                return Err(Unsupported::Translocated);
            }
            enclosing_bundle(exe)
                .map(InstallTarget::MacBundle)
                .ok_or(Unsupported::NotInBundle)
        }
        Os::Windows => exe
            .map(|e| InstallTarget::WindowsExe(e.to_path_buf()))
            .ok_or(Unsupported::NoExe),
        Os::Other => Err(Unsupported::Platform),
    }
}

/// [`plan_target`] for this process.
pub fn current_target() -> Result<InstallTarget, Unsupported> {
    let exe = std::env::current_exe().ok();
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    plan_target(Os::current(), exe.as_deref(), appimage.as_deref())
}

/// `…/X.app` for `…/X.app/Contents/MacOS/<binary>`.
pub fn enclosing_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_app = bundle.extension().is_some_and(|e| e == "app");
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app)
        .then(|| bundle.to_path_buf())
}

/// `path` with `suffix` added to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

impl InstallTarget {
    /// The installed bundle or file.
    pub fn path(&self) -> &Path {
        match self {
            InstallTarget::MacBundle(p)
            | InstallTarget::WindowsExe(p)
            | InstallTarget::AppImage(p) => p,
        }
    }

    /// The folder holding it (the staging folder goes there).
    pub fn dir(&self) -> PathBuf {
        self.path()
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.dir().join(STAGING_DIR)
    }

    /// Where the previous version is kept until the next start.
    pub fn old_path(&self) -> Option<PathBuf> {
        match self {
            InstallTarget::MacBundle(p) => Some(with_suffix(p, OLD_BUNDLE_SUFFIX)),
            InstallTarget::WindowsExe(p) => Some(with_suffix(p, OLD_EXE_SUFFIX)),
            InstallTarget::AppImage(_) => None,
        }
    }

    /// What an update leaves behind: the staging folder and the previous
    /// version.
    pub fn leftovers(&self) -> Vec<PathBuf> {
        let mut v = vec![self.staging_dir()];
        v.extend(self.old_path());
        v
    }

    /// The command that starts the installed (new) version, telling it to
    /// wait for this one to close.
    pub fn restart_command(&self) -> Command {
        match self {
            InstallTarget::MacBundle(b) => {
                let mut c = Command::new("/usr/bin/open");
                c.arg("-n")
                    .arg(b)
                    .arg("--args")
                    .arg(super::AFTER_UPDATE_ARG);
                c
            }
            InstallTarget::WindowsExe(p) | InstallTarget::AppImage(p) => {
                let mut c = Command::new(p);
                c.arg(super::AFTER_UPDATE_ARG);
                c
            }
        }
    }
}

/// Removes a file or a folder (with its contents); a missing one is fine.
pub fn remove_any(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes what a previous update left behind, quietly (on Windows the old
/// executable may still be closing; the next start tries again).
pub fn cleanup_leftovers(target: &InstallTarget) {
    for p in target.leftovers() {
        let _ = remove_any(&p);
    }
}

/// A fresh, empty staging folder beside the installed app. Fails when that
/// folder cannot be written (the user must move OpenHE or update by hand).
pub fn prepare_staging(target: &InstallTarget) -> io::Result<PathBuf> {
    let dir = target.staging_dir();
    remove_any(&dir)?;
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// How a swap ended when it did not work.
#[derive(Debug)]
pub enum SwapError {
    /// Nothing was changed (the old copy could not be moved aside).
    NotReplaced(io::Error),
    /// The new copy could not be moved in; the old one is back in place.
    RolledBack(io::Error),
    /// Neither the new copy nor the old one is in place.
    RollbackFailed(io::Error, io::Error),
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwapError::NotReplaced(e) => write!(f, "not replaced: {e}"),
            SwapError::RolledBack(e) => write!(f, "rolled back: {e}"),
            SwapError::RollbackFailed(e, r) => {
                write!(f, "replace failed: {e}; rollback failed: {r}")
            }
        }
    }
}

/// Puts `new` at `current` and keeps the previous one at `old` (a stale
/// `old` is removed first). Works for a bundle folder and a single file
/// alike. If moving `new` in fails, `old` is moved back.
pub fn swap_in(current: &Path, new: &Path, old: &Path) -> Result<(), SwapError> {
    remove_any(old).map_err(SwapError::NotReplaced)?;
    std::fs::rename(current, old).map_err(SwapError::NotReplaced)?;
    if let Err(e) = std::fs::rename(new, current) {
        return match std::fs::rename(old, current) {
            Ok(()) => Err(SwapError::RolledBack(e)),
            Err(r) => Err(SwapError::RollbackFailed(e, r)),
        };
    }
    Ok(())
}

/// Linux: makes `new` executable (0755) and renames it over the AppImage.
pub fn replace_file(new: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(new, target)
}

/// `CFBundleShortVersionString` of an `Info.plist` (XML, as the build
/// script writes it).
pub fn plist_version(plist: &str) -> Option<&str> {
    let key = "<key>CFBundleShortVersionString</key>";
    let rest = &plist[plist.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix("<string>")?;
    Some(rest[..rest.find("</string>")?].trim())
}

/// Why an unpacked macOS bundle was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// No `OpenHE.app` (or no `Info.plist`) in the archive.
    Missing,
    /// It is another version (found).
    Version(String),
}

/// Checks the bundle unpacked from the zip: `Contents/Info.plist` names
/// `version`.
pub fn check_bundle(bundle: &Path, version: &str) -> Result<(), BundleError> {
    let plist = std::fs::read_to_string(bundle.join("Contents").join("Info.plist"))
        .map_err(|_| BundleError::Missing)?;
    match plist_version(&plist) {
        Some(v) if v == version => Ok(()),
        Some(v) => Err(BundleError::Version(v.to_string())),
        None => Err(BundleError::Version(String::new())),
    }
}

/// Unpacks a zip made by `ditto -c -k --keepParent` into `dest` with
/// `/usr/bin/ditto` (keeps permissions, symlinks and the code signature).
pub fn unzip_with_ditto(zip: &Path, dest: &Path) -> io::Result<()> {
    let out = Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(dest)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ditto: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Opens `url` in the system browser (the release page, for a manual
/// download). Only `https://` links are opened.
pub fn open_url(url: &str) -> io::Result<()> {
    if !url.starts_with("https://") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not https"));
    }
    let mut cmd = open_url_command(url);
    let mut child = cmd.spawn()?;
    // Reap it so no zombie stays behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn open_url_command(url: &str) -> Command {
    if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        // `start` is a cmd built-in; the empty title keeps it from taking
        // the URL as one. Our URLs hold no `&` or quotes.
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(url);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> TempDir {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir()
                .join(format!("openhe-upd-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn macos_finds_the_bundle_around_the_executable() {
        let exe = p("/Applications/OpenHE.app/Contents/MacOS/openhe-app");
        assert_eq!(
            plan_target(Os::Mac, Some(&exe), None),
            Ok(InstallTarget::MacBundle(p("/Applications/OpenHE.app")))
        );
        // A renamed bundle is updated where it is.
        let exe = p("/Users/a/Apps/OpenHE 2.app/Contents/MacOS/openhe-app");
        assert_eq!(
            plan_target(Os::Mac, Some(&exe), None),
            Ok(InstallTarget::MacBundle(p("/Users/a/Apps/OpenHE 2.app")))
        );
    }

    #[test]
    fn macos_outside_a_bundle_cannot_update_itself() {
        for exe in [
            "/Users/a/openhe/target/release/openhe-app",
            "/Applications/OpenHE/Contents/MacOS/openhe-app",
            "/Applications/OpenHE.app/Contents/openhe-app",
            "/Applications/OpenHE.app/MacOS/openhe-app",
            "openhe-app",
        ] {
            assert_eq!(
                plan_target(Os::Mac, Some(&p(exe)), None),
                Err(Unsupported::NotInBundle),
                "{exe}"
            );
        }
        assert_eq!(plan_target(Os::Mac, None, None), Err(Unsupported::NoExe));
        let translocated = p(
            "/private/var/folders/x/T/AppTranslocation/ABC/d/OpenHE.app/Contents/MacOS/openhe-app",
        );
        assert_eq!(
            plan_target(Os::Mac, Some(&translocated), None),
            Err(Unsupported::Translocated)
        );
    }

    #[test]
    fn windows_replaces_the_running_executable() {
        let exe = p(r"C:\Users\a\OpenHE\OpenHE.exe");
        let t = plan_target(Os::Windows, Some(&exe), None).unwrap();
        assert_eq!(t, InstallTarget::WindowsExe(exe.clone()));
        assert_eq!(t.old_path(), Some(p(r"C:\Users\a\OpenHE\OpenHE.exe.old")));
        assert_eq!(
            plan_target(Os::Windows, None, None),
            Err(Unsupported::NoExe)
        );
    }

    #[test]
    fn linux_only_updates_an_appimage() {
        let exe = p("/home/a/bin/openhe-app");
        let img = p("/home/a/Apps/OpenHE.AppImage");
        assert_eq!(
            plan_target(Os::Linux, Some(&exe), Some(&img)),
            Ok(InstallTarget::AppImage(img.clone()))
        );
        assert_eq!(
            plan_target(Os::Linux, Some(&exe), None),
            Err(Unsupported::NotAppImage)
        );
        assert_eq!(
            plan_target(Os::Linux, Some(&exe), Some(Path::new(""))),
            Err(Unsupported::NotAppImage)
        );
        assert_eq!(
            plan_target(Os::Other, Some(&exe), Some(&img)),
            Err(Unsupported::Platform)
        );
        let t = InstallTarget::AppImage(img);
        assert_eq!(t.old_path(), None);
        assert_eq!(t.staging_dir(), p("/home/a/Apps/.openhe-update"));
    }

    #[test]
    fn leftovers_sit_beside_the_app() {
        let t = InstallTarget::MacBundle(p("/Applications/OpenHE.app"));
        assert_eq!(
            t.leftovers(),
            [
                p("/Applications/.openhe-update"),
                p("/Applications/OpenHE.app.old-update")
            ]
        );
    }

    #[test]
    fn restart_commands_start_the_new_version() {
        let t = InstallTarget::MacBundle(p("/Applications/OpenHE.app"));
        let c = t.restart_command();
        assert_eq!(c.get_program(), "/usr/bin/open");
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(
            args,
            ["-n", "/Applications/OpenHE.app", "--args", "--after-update"]
        );
        let t = InstallTarget::AppImage(p("/x/OpenHE.AppImage"));
        let c = t.restart_command();
        assert_eq!(c.get_program(), "/x/OpenHE.AppImage");
        assert_eq!(c.get_args().collect::<Vec<_>>(), ["--after-update"]);
    }

    #[test]
    fn cleanup_removes_what_an_update_left_behind() {
        let d = TempDir::new("cleanup");
        let bundle = d.0.join("OpenHE.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        let old = d.0.join("OpenHE.app.old-update");
        std::fs::create_dir_all(old.join("Contents/MacOS")).unwrap();
        std::fs::write(old.join("Contents/MacOS/openhe-app"), b"old").unwrap();
        std::fs::create_dir_all(d.0.join(STAGING_DIR)).unwrap();
        std::fs::write(d.0.join(STAGING_DIR).join("download"), b"x").unwrap();
        let other = d.0.join("Notes.txt");
        std::fs::write(&other, b"keep").unwrap();
        let t = InstallTarget::MacBundle(bundle.clone());
        cleanup_leftovers(&t);
        assert!(!old.exists());
        assert!(!d.0.join(STAGING_DIR).exists());
        assert!(bundle.exists() && other.exists(), "only leftovers go");
        // Nothing left: quietly nothing.
        cleanup_leftovers(&t);
        // Windows: the old executable.
        let exe = d.0.join("openhe.exe");
        std::fs::write(&exe, b"new").unwrap();
        std::fs::write(d.0.join("openhe.exe.old"), b"old").unwrap();
        cleanup_leftovers(&InstallTarget::WindowsExe(exe.clone()));
        assert!(!d.0.join("openhe.exe.old").exists());
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    }

    #[test]
    fn staging_starts_empty() {
        let d = TempDir::new("staging");
        let t = InstallTarget::WindowsExe(d.0.join("openhe.exe"));
        std::fs::create_dir_all(t.staging_dir()).unwrap();
        std::fs::write(t.staging_dir().join("stale"), b"x").unwrap();
        let s = prepare_staging(&t).unwrap();
        assert_eq!(s, t.staging_dir());
        assert_eq!(std::fs::read_dir(&s).unwrap().count(), 0);
        // A folder that cannot hold it (here: its parent is a file).
        let blocked = InstallTarget::WindowsExe(d.0.join("file.txt").join("openhe.exe"));
        std::fs::write(d.0.join("file.txt"), b"").unwrap();
        assert!(prepare_staging(&blocked).is_err());
    }

    #[test]
    fn a_swap_keeps_the_old_copy_and_replaces_a_stale_one() {
        let d = TempDir::new("swap");
        let cur = d.0.join("openhe.exe");
        let new = d.0.join(STAGING_DIR).join("download");
        let old = d.0.join("openhe.exe.old");
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::write(&cur, b"v1").unwrap();
        std::fs::write(&new, b"v2").unwrap();
        std::fs::write(&old, b"v0").unwrap();
        swap_in(&cur, &new, &old).unwrap();
        assert_eq!(std::fs::read(&cur).unwrap(), b"v2");
        assert_eq!(std::fs::read(&old).unwrap(), b"v1");
        assert!(!new.exists());
    }

    #[test]
    fn a_bundle_swap_works_on_folders() {
        let d = TempDir::new("swap-dir");
        let cur = d.0.join("OpenHE.app");
        let new = d.0.join(STAGING_DIR).join("unpacked").join("OpenHE.app");
        let old = d.0.join("OpenHE.app.old-update");
        std::fs::create_dir_all(cur.join("Contents")).unwrap();
        std::fs::write(cur.join("Contents/v"), b"1").unwrap();
        std::fs::create_dir_all(new.join("Contents")).unwrap();
        std::fs::write(new.join("Contents/v"), b"2").unwrap();
        // A stale old bundle with contents.
        std::fs::create_dir_all(old.join("Contents")).unwrap();
        swap_in(&cur, &new, &old).unwrap();
        assert_eq!(std::fs::read(cur.join("Contents/v")).unwrap(), b"2");
        assert_eq!(std::fs::read(old.join("Contents/v")).unwrap(), b"1");
    }

    #[test]
    fn a_failed_swap_rolls_back() {
        let d = TempDir::new("rollback");
        let cur = d.0.join("openhe.exe");
        let old = d.0.join("openhe.exe.old");
        std::fs::write(&cur, b"v1").unwrap();
        // The new file is missing: moving it in fails, the old comes back.
        let missing = d.0.join("nope");
        match swap_in(&cur, &missing, &old) {
            Err(SwapError::RolledBack(_)) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&cur).unwrap(), b"v1");
        assert!(!old.exists());
        // The current copy cannot be moved aside: nothing changes.
        let gone = d.0.join("gone.exe");
        let new = d.0.join("new");
        std::fs::write(&new, b"v2").unwrap();
        match swap_in(&gone, &new, &d.0.join("gone.exe.old")) {
            Err(SwapError::NotReplaced(_)) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&new).unwrap(), b"v2");
    }

    #[test]
    fn an_appimage_is_replaced_and_executable() {
        let d = TempDir::new("appimage");
        let img = d.0.join("OpenHE.AppImage");
        let new = d.0.join("download");
        std::fs::write(&img, b"v1").unwrap();
        std::fs::write(&new, b"v2").unwrap();
        replace_file(&new, &img).unwrap();
        assert_eq!(std::fs::read(&img).unwrap(), b"v2");
        assert!(!new.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&img).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>openhe-app</string>
    <key>CFBundleShortVersionString</key>
    <string>0.3.0</string>
    <key>CFBundleVersion</key>
    <string>0.3.0</string>
</dict>
</plist>"#;

    #[test]
    fn the_unpacked_bundle_must_be_the_announced_version() {
        assert_eq!(plist_version(PLIST), Some("0.3.0"));
        assert_eq!(plist_version("<plist/>"), None);
        let d = TempDir::new("bundle");
        let app = d.0.join(ZIP_BUNDLE);
        assert_eq!(check_bundle(&app, "0.3.0"), Err(BundleError::Missing));
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), PLIST).unwrap();
        assert_eq!(check_bundle(&app, "0.3.0"), Ok(()));
        assert_eq!(
            check_bundle(&app, "0.4.0"),
            Err(BundleError::Version("0.3.0".into()))
        );
    }

    #[test]
    fn only_https_links_are_opened() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
    }
}

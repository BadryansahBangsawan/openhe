//! Pembaruan end to end: the real app with a fake GitHub (an in-memory
//! [`MemorySource`] serving a signed manifest), so no test touches the
//! network. Installs go to a file in a temp folder, never the test binary;
//! "Mulai ulang sekarang" is never clicked (it would start a process).

mod support;
use ed25519_dalek::{Signer, SigningKey};
use openhe_app::storage::AppData;
use openhe_app::update::install::{InstallTarget, Unsupported};
use openhe_app::update::manifest::sha256;
use openhe_app::update::source::MemorySource;
use openhe_app::update::{to_hex, Phase, UpdateEnv, MANIFEST_NAME, RELEASES_API, SIGNATURE_NAME};
use semver::Version;
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

const PLATFORM: &str = "windows-x86_64";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

/// A fake GitHub listing release `tag` (a signed manifest for one Windows
/// file, `payload`) with release notes.
fn github(tag: &str, payload: &[u8]) -> MemorySource {
    let s = MemorySource::new();
    let version = tag.trim_start_matches('v');
    let base = format!("https://github.com/x/releases/download/{tag}");
    let exe = format!("OpenHE-{tag}-windows-x86_64.exe");
    let manifest = format!(
        r#"{{"version":"{version}","assets":{{"{PLATFORM}":{{"name":"{exe}","sha256":"{}","size":{}}}}}}}"#,
        to_hex(&sha256(payload)),
        payload.len()
    );
    let sig = format!("{}\n", to_hex(&key().sign(manifest.as_bytes()).to_bytes()));
    s.insert(format!("{base}/{MANIFEST_NAME}"), manifest);
    s.insert(format!("{base}/{SIGNATURE_NAME}"), sig);
    s.insert(format!("{base}/{exe}"), payload.to_vec());
    s.insert(
        RELEASES_API,
        format!(
            r#"[{{"tag_name":"{tag}","draft":false,"prerelease":false,
                 "body":"Perbaikan lampu dan macro.",
                 "assets":[{{"name":"{MANIFEST_NAME}","browser_download_url":"{base}/{MANIFEST_NAME}"}},
                           {{"name":"{SIGNATURE_NAME}","browser_download_url":"{base}/{SIGNATURE_NAME}"}},
                           {{"name":"{exe}","browser_download_url":"{base}/{exe}"}}]}}]"#
        ),
    );
    s
}

fn env(source: MemorySource, target: Result<InstallTarget, Unsupported>) -> UpdateEnv {
    UpdateEnv {
        source: Arc::new(source),
        target,
        public_key: *key().verifying_key().as_bytes(),
        current: Version::new(0, 2, 0),
        platform: Some(PLATFORM),
        live: true,
    }
}

/// A folder under the test home holding a fake installed `openhe.exe`.
fn installed_exe(tag: &str) -> PathBuf {
    let dir = private_home().join(format!("install-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("openhe.exe");
    std::fs::write(&exe, b"old version").unwrap();
    exe
}

/// The app with automatic checks off (checks only by hand), on `env`.
fn manual(env: UpdateEnv) -> World {
    let mut data = AppData::default();
    data.update.auto_check = false;
    let mut w = World::with(Sim::new(), data, fast_config());
    w.app_mut().set_update_env(env);
    w.page("Pengaturan");
    w
}

fn settled(w: &mut World) {
    w.until("updater idle", 30, |w| !w.app().updater().is_busy());
}

#[test]
fn a_newer_release_is_offered_downloaded_and_installed() {
    let exe = installed_exe("newer");
    let mut w = manual(env(
        github("v0.3.0", b"new version"),
        Ok(InstallTarget::WindowsExe(exe.clone())),
    ));
    assert!(w.has_text("Versi terpasang"), "{:?}", w.labels());
    assert!(w.has_text("Belum pernah diperiksa."), "{:?}", w.labels());
    w.click("Periksa sekarang");
    settled(&mut w);
    assert!(
        w.has_text("Versi baru tersedia: v0.3.0"),
        "{:?}",
        w.labels()
    );
    assert!(w.has_text("Perbaikan lampu dan macro."), "release notes");
    assert!(w.has("Unduh & pasang"));
    // The check is stored.
    assert!(w.app().updater().offer().is_some());
    w.click("Unduh & pasang");
    settled(&mut w);
    assert_eq!(w.app().updater().phase(), Phase::Installed);
    assert!(w.has("Mulai ulang sekarang"), "{:?}", w.labels());
    assert!(w.enabled("Mulai ulang sekarang"));
    assert!(w.has_text("v0.3.0 sudah terpasang"), "{:?}", w.labels());
    assert_eq!(std::fs::read(&exe).unwrap(), b"new version");
    assert_eq!(
        std::fs::read(exe.with_file_name("openhe.exe.old")).unwrap(),
        b"old version"
    );
    // Nothing more to check until the restart.
    assert!(!w.enabled("Periksa sekarang"));
    // Nothing was written to the keyboard.
    assert_eq!(w.sim().set_count(), 0);
}

#[test]
fn nothing_newer_reads_up_to_date_and_stores_the_check() {
    let exe = installed_exe("same");
    let mut w = manual(env(
        github("v0.2.0", b"same version"),
        Ok(InstallTarget::WindowsExe(exe.clone())),
    ));
    w.click("Periksa sekarang");
    settled(&mut w);
    assert!(
        w.has_text("OpenHE sudah versi terbaru."),
        "{:?}",
        w.labels()
    );
    assert!(!w.has("Unduh & pasang"));
    assert!(
        w.has_text("Terakhir diperiksa: baru saja"),
        "{:?}",
        w.labels()
    );
    let saved = AppData::load_from(&AppData::default_dir().unwrap()).unwrap();
    assert!(saved.update.last_check.is_some(), "stored in the data file");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old version");
}

#[test]
fn a_manifest_with_a_bad_signature_is_refused() {
    let exe = installed_exe("badsig");
    let mut e = env(
        github("v0.3.0", b"evil"),
        Ok(InstallTarget::WindowsExe(exe.clone())),
    );
    e.public_key = *SigningKey::from_bytes(&[1u8; 32])
        .verifying_key()
        .as_bytes();
    let mut w = manual(e);
    w.click("Periksa sekarang");
    settled(&mut w);
    assert!(w.has_text("Gagal memeriksa pembaruan."), "{:?}", w.labels());
    assert!(
        w.has_text("Tanda tangan pembaruan tidak cocok"),
        "{:?}",
        w.labels()
    );
    assert!(!w.has("Unduh & pasang"));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old version");
}

#[test]
fn a_copy_that_cannot_update_itself_offers_the_download_page() {
    let mut w = manual(env(github("v0.3.0", b"new"), Err(Unsupported::NotInBundle)));
    w.click("Periksa sekarang");
    settled(&mut w);
    assert!(
        w.has_text("Versi baru tersedia: v0.3.0"),
        "{:?}",
        w.labels()
    );
    assert!(!w.has("Unduh & pasang"));
    // Not clicked: it would open the browser.
    assert!(w.has("Buka halaman unduhan"), "{:?}", w.labels());
    assert!(w.has_text("hanya untuk OpenHE.app"), "{:?}", w.labels());
}

#[test]
fn the_automatic_check_shows_a_dismissable_notice_in_the_sidebar() {
    // Defaults: automatic checks on, never checked.
    let mut w = World::new();
    assert!(!w.has_text("Versi 0.3.0 tersedia"));
    w.app_mut()
        .set_update_env(env(github("v0.3.0", b"new"), Err(Unsupported::Platform)));
    w.until("automatic check", 30, |w| {
        w.app().updater().offer().is_some() && !w.app().updater().is_busy()
    });
    assert!(w.has_text("Versi 0.3.0 tersedia"), "{:?}", w.labels());
    // "Lihat" opens Pengaturan, where the offer is.
    w.click("Lihat");
    w.frames(3);
    assert!(w.selected("Pengaturan"));
    assert!(
        w.has_text("Versi baru tersedia: v0.3.0"),
        "{:?}",
        w.labels()
    );
    // "Nanti" hides it for this version, also next time.
    w.click("Nanti");
    w.frames(2);
    assert!(!w.has_text("Versi 0.3.0 tersedia"), "{:?}", w.labels());
    let saved = AppData::load_from(&AppData::default_dir().unwrap()).unwrap();
    assert_eq!(saved.update.dismissed.as_deref(), Some("0.3.0"));
    assert!(saved.update.last_check.is_some());
}

#[test]
fn update_settings_are_stored() {
    let mut w = manual(env(MemorySource::new(), Err(Unsupported::Platform)));
    // Toggles: per-app switching, launch at login, then automatic checks
    // and beta.
    use eframe::egui::accesskit::Role;
    assert_eq!(w.role_count(Role::CheckBox), 4, "{:?}", w.labels());
    assert!(
        !w.role_checked(Role::CheckBox, 2),
        "automatic checks off here"
    );
    assert!(!w.role_checked(Role::CheckBox, 3), "beta off by default");
    w.click_role(Role::CheckBox, 3);
    w.click_role(Role::CheckBox, 2);
    let saved = AppData::load_from(&AppData::default_dir().unwrap()).unwrap();
    assert!(saved.update.beta);
    assert!(saved.update.auto_check);
}

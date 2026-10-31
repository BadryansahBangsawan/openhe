//! The update flows ([`check`], [`download_and_install`]) and [`Updater`],
//! which runs them on a background thread for the UI (like the keyboard
//! worker: the UI thread only polls a channel, it never waits).

use super::install::{self, BundleError, InstallTarget, SwapError, Unsupported};
use super::manifest::{self, Hasher, ManifestError};
use super::release;
use super::source::{FetchError, GithubSource, NoSource, UpdateSource};
use crate::i18n::update as t;
use semver::Version;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// Why a check or an install failed. Nothing installed was changed, except
/// for [`UpdateError::Replace`] with a failed rollback.
#[derive(Debug)]
pub enum UpdateError {
    Fetch(FetchError),
    /// The releases list did not parse.
    List(String),
    Manifest(ManifestError),
    /// The manifest's version is not the release's, or not newer.
    Version {
        manifest: Version,
        tag: String,
    },
    /// The download's size or SHA-256 does not match the manifest.
    Checksum,
    /// The staging folder beside the app could not be made.
    NotWritable(std::io::Error),
    /// Saving the download failed.
    Disk(std::io::Error),
    /// The macOS zip did not unpack.
    Extract(std::io::Error),
    /// The unpacked bundle is not the announced version.
    Bundle(BundleError),
    Replace(SwapError),
    /// No update was offered for this platform, or this copy cannot update
    /// itself (the UI does not offer it then).
    NotAvailable,
}

impl UpdateError {
    /// Indonesian text for the card.
    pub fn text(&self) -> String {
        match self {
            UpdateError::Fetch(FetchError::Status(403 | 429)) => t::ERR_RATE.to_string(),
            UpdateError::Fetch(FetchError::Status(s)) => format!("{} {s}.", t::ERR_HTTP),
            UpdateError::Fetch(FetchError::TooLarge) => t::ERR_TOO_LARGE.to_string(),
            UpdateError::Fetch(FetchError::Write(_)) | UpdateError::Disk(_) => {
                t::ERR_DISK.to_string()
            }
            UpdateError::Fetch(FetchError::Network(_)) => t::ERR_NETWORK.to_string(),
            UpdateError::List(_) => t::ERR_LIST.to_string(),
            UpdateError::Manifest(ManifestError::Invalid(_)) => t::ERR_MANIFEST.to_string(),
            UpdateError::Manifest(_) => t::ERR_SIGNATURE.to_string(),
            UpdateError::Version { .. } => t::ERR_VERSION.to_string(),
            UpdateError::Checksum => t::ERR_CHECKSUM.to_string(),
            UpdateError::NotWritable(_) => t::ERR_NOT_WRITABLE.to_string(),
            UpdateError::Extract(_) => t::ERR_EXTRACT.to_string(),
            UpdateError::Bundle(_) => t::ERR_BUNDLE.to_string(),
            UpdateError::Replace(SwapError::RollbackFailed(..)) => t::ERR_ROLLBACK.to_string(),
            UpdateError::Replace(_) => t::ERR_REPLACE.to_string(),
            UpdateError::NotAvailable => t::NO_ASSET.to_string(),
        }
    }

    /// The raw detail, for the "Detail teknis" tooltip.
    pub fn technical(&self) -> String {
        match self {
            UpdateError::Fetch(e) => e.to_string(),
            UpdateError::List(e) => format!("releases: {e}"),
            UpdateError::Manifest(e) => format!("manifest: {e:?}"),
            UpdateError::Version { manifest, tag } => format!("manifest {manifest}, tag {tag}"),
            UpdateError::Checksum => "size or sha256 mismatch".into(),
            UpdateError::NotWritable(e) => format!("staging: {e}"),
            UpdateError::Disk(e) => format!("disk: {e}"),
            UpdateError::Extract(e) => format!("extract: {e}"),
            UpdateError::Bundle(e) => format!("bundle: {e:?}"),
            UpdateError::Replace(e) => e.to_string(),
            UpdateError::NotAvailable => "not available".into(),
        }
    }
}

/// The file of an offered update for this platform (from the verified
/// manifest), with where to get it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferAsset {
    pub url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// A newer release whose manifest verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// `vX.Y.Z`.
    pub tag: String,
    pub version: Version,
    pub prerelease: bool,
    /// Release notes (unsigned; shown as plain text only).
    pub notes: String,
    /// `None`: the release has no file for this platform.
    pub asset: Option<OfferAsset>,
}

impl Offer {
    pub fn page(&self) -> String {
        super::release_page(&self.tag)
    }
}

/// Finds the update to offer: the newest suitable release (see
/// [`release::select`]), its manifest verified with `key`, its version
/// checked against the tag and `current`, its entry for `platform` (and
/// that asset's URL in the release). `Ok(None)`: up to date.
pub fn check(
    source: &dyn UpdateSource,
    key: &[u8; 32],
    current: &Version,
    beta: bool,
    platform: Option<&str>,
) -> Result<Option<Offer>, UpdateError> {
    let list = source
        .get_bytes(super::RELEASES_API, super::RELEASES_LIMIT)
        .map_err(UpdateError::Fetch)?;
    let releases = release::parse_releases(&list).map_err(|e| UpdateError::List(e.to_string()))?;
    let Some(c) = release::select(&releases, current, beta) else {
        return Ok(None);
    };
    let json = source
        .get_bytes(c.manifest_url, super::MANIFEST_LIMIT)
        .map_err(UpdateError::Fetch)?;
    let sig = source
        .get_bytes(c.signature_url, super::MANIFEST_LIMIT)
        .map_err(UpdateError::Fetch)?;
    let m = manifest::verify(&json, &sig, key).map_err(UpdateError::Manifest)?;
    // The signed version must be the release's, and newer (never a
    // downgrade, even with a validly signed old manifest on a new tag).
    if m.version != c.version || m.version <= *current {
        return Err(UpdateError::Version {
            manifest: m.version,
            tag: c.release.tag_name.clone(),
        });
    }
    let asset = platform.and_then(|p| m.asset(p)).and_then(|a| {
        Some(OfferAsset {
            url: c.release.asset_url(&a.name)?.to_string(),
            size: a.size,
            sha256: a.sha256,
        })
    });
    let notes = c.release.body.clone().unwrap_or_default();
    Ok(Some(Offer {
        tag: c.release.tag_name.clone(),
        prerelease: c.release.is_prerelease(&c.version),
        version: c.version,
        notes,
        asset,
    }))
}

/// A file being written that is hashed on the way.
struct HashingFile {
    file: std::fs::File,
    hash: Hasher,
}

impl Write for HashingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Downloads `asset` to `dest` (at most its size), then checks size and
/// SHA-256. A file that does not match is deleted.
pub fn download_verified(
    source: &dyn UpdateSource,
    asset: &OfferAsset,
    dest: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<(), UpdateError> {
    let file = std::fs::File::create(dest).map_err(UpdateError::Disk)?;
    let mut out = HashingFile {
        file,
        hash: Hasher::default(),
    };
    let fetched = source.get(&asset.url, asset.size, &mut out, progress);
    let flushed = out.file.sync_all();
    let ok = out.hash.len() == asset.size && out.hash.finish() == asset.sha256;
    let result = match fetched {
        Err(e) => Err(UpdateError::Fetch(e)),
        Ok(()) if !ok => Err(UpdateError::Checksum),
        Ok(()) => flushed.map_err(UpdateError::Disk),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// Name of the download inside the staging folder (never the asset name
/// from the manifest: no path comes from the network).
const DOWNLOAD: &str = "download";

/// Downloads the offer into a staging folder beside the app, checks it, and
/// puts it in place for `target` (see [`install`]). The staging folder is
/// removed afterwards, also after a failure.
pub fn download_and_install(
    source: &dyn UpdateSource,
    offer: &Offer,
    target: &InstallTarget,
    progress: &mut dyn FnMut(u64),
    installing: &mut dyn FnMut(),
) -> Result<(), UpdateError> {
    let asset = offer.asset.as_ref().ok_or(UpdateError::NotAvailable)?;
    let staging = install::prepare_staging(target).map_err(UpdateError::NotWritable)?;
    let result = (|| {
        let file = staging.join(DOWNLOAD);
        download_verified(source, asset, &file, progress)?;
        installing();
        apply(target, &file, &staging, &offer.version)
    })();
    let _ = install::remove_any(&staging);
    result
}

/// Puts the verified `file` in place.
fn apply(
    target: &InstallTarget,
    file: &Path,
    staging: &Path,
    version: &Version,
) -> Result<(), UpdateError> {
    match target {
        InstallTarget::MacBundle(bundle) => {
            let unpacked = staging.join("unpacked");
            install::unzip_with_ditto(file, &unpacked).map_err(UpdateError::Extract)?;
            let new = unpacked.join(install::ZIP_BUNDLE);
            install::check_bundle(&new, &version.to_string()).map_err(UpdateError::Bundle)?;
            let old = target.old_path().unwrap_or_default();
            install::swap_in(bundle, &new, &old).map_err(UpdateError::Replace)
        }
        InstallTarget::WindowsExe(exe) => {
            let old = target.old_path().unwrap_or_default();
            install::swap_in(exe, file, &old).map_err(UpdateError::Replace)
        }
        InstallTarget::AppImage(path) => install::replace_file(file, path)
            .map_err(|e| UpdateError::Replace(SwapError::NotReplaced(e))),
    }
}

/// Everything the updater depends on (tests swap all of it).
#[derive(Clone)]
pub struct UpdateEnv {
    pub source: Arc<dyn UpdateSource>,
    /// What an update replaces, or why this copy cannot update itself.
    pub target: Result<InstallTarget, Unsupported>,
    pub public_key: [u8; 32],
    pub current: Version,
    /// The manifest key of this platform ([`super::platform_key`]).
    pub platform: Option<&'static str>,
    /// The real thing: GitHub over HTTPS. Automatic checks only run then.
    pub live: bool,
}

impl UpdateEnv {
    /// The running app: GitHub, the release key, this executable.
    pub fn system() -> UpdateEnv {
        UpdateEnv {
            source: Arc::new(GithubSource::default()),
            target: install::current_target(),
            public_key: super::PUBLIC_KEY,
            current: current_version(),
            platform: super::current_platform(),
            live: true,
        }
    }

    /// No updates (tests and previews that inject nothing).
    pub fn off() -> UpdateEnv {
        UpdateEnv {
            source: Arc::new(NoSource),
            target: Err(Unsupported::Platform),
            public_key: super::PUBLIC_KEY,
            current: current_version(),
            platform: None,
            live: false,
        }
    }
}

/// [`super::CURRENT_VERSION`] as a version.
pub fn current_version() -> Version {
    Version::parse(super::CURRENT_VERSION).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// What the updater is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// Not checked in this run (or the last check failed).
    #[default]
    Idle,
    Checking,
    /// Checked: see [`Updater::offer`] (`None`: up to date).
    Checked,
    Downloading {
        done: u64,
        total: u64,
    },
    /// Verified; being put in place.
    Installing,
    /// The new version is in place; restart to use it.
    Installed,
}

enum Msg {
    Checked(Result<Option<Offer>, UpdateError>),
    Progress(u64),
    Installing,
    Installed(Result<(), UpdateError>),
}

/// What [`Updater::pump`] reports for the page to store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A check finished successfully (store the time).
    Checked,
}

/// The updater as the UI sees it. Every network and file step runs on its
/// own thread; [`Updater::pump`] (once per frame) collects the results.
pub struct Updater {
    env: UpdateEnv,
    phase: Phase,
    offer: Option<Offer>,
    /// The last failure (Indonesian text, technical detail).
    error: Option<(String, String)>,
    /// The failure was the install's (else the check's).
    install_failed: bool,
    rx: Option<Receiver<Msg>>,
    /// The automatic check of this run was considered.
    auto_done: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Updater::new(UpdateEnv::off())
    }
}

impl Updater {
    pub fn new(env: UpdateEnv) -> Updater {
        Updater {
            env,
            phase: Phase::Idle,
            offer: None,
            error: None,
            install_failed: false,
            rx: None,
            auto_done: false,
        }
    }

    /// Replaces the environment (the app installs the real one; tests a
    /// fake). Forgets any result.
    pub fn set_env(&mut self, env: UpdateEnv) {
        *self = Updater::new(env);
    }

    pub fn env(&self) -> &UpdateEnv {
        &self.env
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }

    /// The last failure: (Indonesian text, technical detail, was it the
    /// install).
    pub fn error(&self) -> Option<(&str, &str, bool)> {
        self.error
            .as_ref()
            .map(|(a, b)| (a.as_str(), b.as_str(), self.install_failed))
    }

    /// A check, download or install runs.
    pub fn is_busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Starts the run's automatic check once, if it is due (see
    /// [`super::auto_check_due`]) and this is the real app. True when one
    /// started.
    pub fn auto_check(&mut self, enabled: bool, beta: bool, last: Option<u64>, now: u64) -> bool {
        if self.auto_done || !self.env.live {
            return false;
        }
        self.auto_done = true;
        if !super::auto_check_due(enabled, last, now) || self.is_busy() {
            return false;
        }
        self.check_now(beta);
        true
    }

    /// "Periksa sekarang".
    pub fn check_now(&mut self, beta: bool) {
        if self.is_busy() || self.phase == Phase::Installed {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.phase = Phase::Checking;
        self.error = None;
        let env = self.env.clone();
        spawn(tx, move |tx| {
            let r = check(
                env.source.as_ref(),
                &env.public_key,
                &env.current,
                beta,
                env.platform,
            );
            let _ = tx.send(Msg::Checked(r));
        });
    }

    /// The offered update can be installed here (a file for this platform,
    /// and a copy that can update itself).
    pub fn can_install(&self) -> bool {
        self.env.target.is_ok() && self.offer.as_ref().is_some_and(|o| o.asset.is_some())
    }

    /// "Unduh & pasang".
    pub fn install(&mut self) {
        if self.is_busy() || self.phase != Phase::Checked || !self.can_install() {
            return;
        }
        let (Some(offer), Ok(target)) = (self.offer.clone(), self.env.target.clone()) else {
            return;
        };
        let total = offer.asset.as_ref().map_or(0, |a| a.size);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.phase = Phase::Downloading { done: 0, total };
        self.error = None;
        let source = Arc::clone(&self.env.source);
        spawn(tx, move |tx| {
            // At most ~100 progress messages.
            let step = (total / 100).max(64 * 1024);
            let mut sent = 0u64;
            let mut progress = |n: u64| {
                if n >= sent + step || n == total {
                    sent = n;
                    let _ = tx.send(Msg::Progress(n));
                }
            };
            let mut installing = || {
                let _ = tx.send(Msg::Installing);
            };
            let r = download_and_install(
                source.as_ref(),
                &offer,
                &target,
                &mut progress,
                &mut installing,
            );
            let _ = tx.send(Msg::Installed(r));
        });
    }

    /// "Mulai ulang sekarang": starts the new version (which waits for
    /// this one to close). The caller closes the window when it worked.
    pub fn restart(&mut self) -> bool {
        let Ok(target) = &self.env.target else {
            return false;
        };
        if self.phase != Phase::Installed {
            return false;
        }
        match target.restart_command().spawn() {
            Ok(_) => true,
            Err(e) => {
                self.error = Some((t::RESTART_FAILED.to_string(), e.to_string()));
                self.install_failed = true;
                false
            }
        }
    }

    /// Collects what the background thread finished. Call once per frame.
    pub fn pump(&mut self) -> Option<Event> {
        let mut event = None;
        loop {
            // A finished job drops the receiver: report what it said.
            let Some(rx) = self.rx.as_ref() else {
                return event;
            };
            let msg = match rx.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => return event,
                Err(TryRecvError::Disconnected) => {
                    // The thread died without an answer (a panic).
                    self.rx = None;
                    if self.phase != Phase::Installed {
                        self.phase = if self.offer.is_some() {
                            Phase::Checked
                        } else {
                            Phase::Idle
                        };
                        self.error = Some((t::INTERNAL.to_string(), "updater thread".into()));
                    }
                    return event;
                }
            };
            match msg {
                Msg::Checked(r) => {
                    self.rx = None;
                    match r {
                        Ok(offer) => {
                            self.offer = offer;
                            self.phase = Phase::Checked;
                            event = Some(Event::Checked);
                        }
                        Err(e) => {
                            self.phase = Phase::Idle;
                            self.install_failed = false;
                            self.error = Some((e.text(), e.technical()));
                        }
                    }
                }
                Msg::Progress(n) => {
                    if let Phase::Downloading { total, .. } = self.phase {
                        self.phase = Phase::Downloading { done: n, total };
                    }
                }
                Msg::Installing => self.phase = Phase::Installing,
                Msg::Installed(r) => {
                    self.rx = None;
                    match r {
                        Ok(()) => self.phase = Phase::Installed,
                        Err(e) => {
                            self.phase = Phase::Checked;
                            self.install_failed = true;
                            self.error = Some((e.text(), e.technical()));
                        }
                    }
                }
            }
        }
    }

    /// Screenshot and test hook: an offer, as if a check found it.
    #[doc(hidden)]
    pub fn set_offer(&mut self, offer: Option<Offer>, phase: Phase) {
        self.offer = offer;
        self.phase = phase;
    }
}

/// Runs `f` on a named thread, with the sender. Should no thread start,
/// the sender is dropped with the closure and [`Updater::pump`] reports it.
fn spawn(tx: Sender<Msg>, f: impl FnOnce(&Sender<Msg>) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("openhe-update".into())
        .spawn(move || f(&tx));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::install::tests::TempDir;
    use super::super::manifest::tests::{sign, test_key};
    use super::super::source::MemorySource;
    use super::super::{to_hex, MANIFEST_NAME, RELEASES_API, SIGNATURE_NAME};
    use super::*;
    use std::time::{Duration, Instant};

    const PLATFORM: &str = "windows-x86_64";

    /// A fake GitHub with one release `tag` whose Windows file is `payload`
    /// (signed manifest), plus an older release.
    pub(crate) fn fake_github(tag: &str, payload: &[u8], prerelease: bool) -> MemorySource {
        let s = MemorySource::new();
        publish(&s, tag, payload, prerelease);
        s
    }

    pub(crate) fn publish(s: &MemorySource, tag: &str, payload: &[u8], prerelease: bool) {
        let version = tag.trim_start_matches('v');
        let base = format!("https://github.com/x/releases/download/{tag}");
        let exe = format!("OpenHE-{tag}-windows-x86_64.exe");
        let manifest = format!(
            r#"{{"version":"{version}","assets":{{"{PLATFORM}":{{"name":"{exe}","sha256":"{}","size":{}}}}}}}"#,
            to_hex(&manifest::sha256(payload)),
            payload.len()
        );
        let sig = sign(&test_key(), manifest.as_bytes());
        s.insert(format!("{base}/{MANIFEST_NAME}"), manifest);
        s.insert(format!("{base}/{SIGNATURE_NAME}"), sig);
        s.insert(format!("{base}/{exe}"), payload.to_vec());
        let list = format!(
            r#"[{{"tag_name":"{tag}","draft":false,"prerelease":{prerelease},"body":"Catatan {tag}",
                "assets":[{{"name":"{MANIFEST_NAME}","browser_download_url":"{base}/{MANIFEST_NAME}"}},
                          {{"name":"{SIGNATURE_NAME}","browser_download_url":"{base}/{SIGNATURE_NAME}"}},
                          {{"name":"{exe}","browser_download_url":"{base}/{exe}"}}]}},
               {{"tag_name":"v0.0.1","draft":false,"prerelease":false,"assets":[]}}]"#
        );
        s.insert(RELEASES_API, list);
    }

    fn key() -> [u8; 32] {
        *test_key().verifying_key().as_bytes()
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn a_newer_signed_release_is_offered() {
        let s = fake_github("v0.3.0", b"new exe", false);
        let o = check(&s, &key(), &v("0.2.0"), false, Some(PLATFORM))
            .unwrap()
            .unwrap();
        assert_eq!(o.tag, "v0.3.0");
        assert_eq!(o.version, v("0.3.0"));
        assert_eq!(o.notes, "Catatan v0.3.0");
        assert!(!o.prerelease);
        let a = o.asset.unwrap();
        assert_eq!(a.size, 7);
        assert!(a.url.ends_with("/OpenHE-v0.3.0-windows-x86_64.exe"));
        assert_eq!(s.requests()[0], RELEASES_API);
    }

    #[test]
    fn up_to_date_is_no_offer_and_fetches_only_the_list() {
        let s = fake_github("v0.2.0", b"same", false);
        assert_eq!(
            check(&s, &key(), &v("0.2.0"), true, Some(PLATFORM)).unwrap(),
            None
        );
        assert_eq!(s.requests(), [RELEASES_API]);
    }

    #[test]
    fn a_release_for_another_platform_offers_no_file() {
        let s = fake_github("v0.3.0", b"x", false);
        let o = check(&s, &key(), &v("0.2.0"), false, Some("linux-x86_64"))
            .unwrap()
            .unwrap();
        assert_eq!(o.asset, None);
        let o = check(&s, &key(), &v("0.2.0"), false, None)
            .unwrap()
            .unwrap();
        assert_eq!(o.asset, None);
    }

    #[test]
    fn a_manifest_signed_with_another_key_is_refused() {
        let s = fake_github("v0.3.0", b"x", false);
        let err = check(
            &s,
            &super::super::PUBLIC_KEY,
            &v("0.2.0"),
            false,
            Some(PLATFORM),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            UpdateError::Manifest(ManifestError::BadSignature)
        ));
        assert_eq!(err.text(), t::ERR_SIGNATURE);
    }

    #[test]
    fn a_manifest_of_another_version_is_refused() {
        // A validly signed 0.3.0 manifest replayed on a v0.4.0 tag.
        let s = fake_github("v0.3.0", b"x", false);
        let list = String::from_utf8(s.get_bytes(RELEASES_API, 1 << 20).unwrap()).unwrap();
        s.insert(RELEASES_API, list.replacen("\"v0.3.0\"", "\"v0.4.0\"", 1));
        let err = check(&s, &key(), &v("0.2.0"), false, Some(PLATFORM)).unwrap_err();
        assert!(matches!(err, UpdateError::Version { .. }), "{err:?}");
    }

    #[test]
    fn network_failures_read_in_indonesian() {
        let s = MemorySource::new();
        let err = check(&s, &key(), &v("0.2.0"), false, Some(PLATFORM)).unwrap_err();
        assert_eq!(err.text(), format!("{} 404.", t::ERR_HTTP));
        assert_eq!(
            UpdateError::Fetch(FetchError::Status(403)).text(),
            t::ERR_RATE
        );
        assert_eq!(
            UpdateError::Fetch(FetchError::Network("dns".into())).text(),
            t::ERR_NETWORK
        );
        s.insert(RELEASES_API, "<html>");
        let err = check(&s, &key(), &v("0.2.0"), false, Some(PLATFORM)).unwrap_err();
        assert_eq!(err.text(), t::ERR_LIST);
    }

    fn offer_for(s: &MemorySource) -> Offer {
        check(s, &key(), &v("0.2.0"), false, Some(PLATFORM))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_download_must_match_size_and_hash() {
        let d = TempDir::new("dl");
        let s = fake_github("v0.3.0", b"payload", false);
        let o = offer_for(&s);
        let a = o.asset.clone().unwrap();
        let dest = d.0.join("download");
        let mut seen = 0;
        download_verified(&s, &a, &dest, &mut |n| seen = n).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
        assert_eq!(seen, 7);
        // Same size, other bytes: SHA-256 mismatch, and the file is gone.
        s.insert(a.url.clone(), b"PAYLOAD".to_vec());
        assert!(matches!(
            download_verified(&s, &a, &dest, &mut |_| {}),
            Err(UpdateError::Checksum)
        ));
        assert!(!dest.exists());
        // Shorter.
        s.insert(a.url.clone(), b"pay".to_vec());
        assert!(matches!(
            download_verified(&s, &a, &dest, &mut |_| {}),
            Err(UpdateError::Checksum)
        ));
        // Longer: stopped at the announced size.
        s.insert(a.url.clone(), b"payload and more".to_vec());
        assert!(matches!(
            download_verified(&s, &a, &dest, &mut |_| {}),
            Err(UpdateError::Fetch(FetchError::TooLarge))
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn a_windows_style_install_swaps_the_file_and_cleans_staging() {
        let d = TempDir::new("install");
        let exe = d.0.join("openhe.exe");
        std::fs::write(&exe, b"old exe").unwrap();
        let s = fake_github("v0.3.0", b"new exe", false);
        let target = InstallTarget::WindowsExe(exe.clone());
        let mut installing = false;
        download_and_install(&s, &offer_for(&s), &target, &mut |_| {}, &mut || {
            installing = true
        })
        .unwrap();
        assert!(installing);
        assert_eq!(std::fs::read(&exe).unwrap(), b"new exe");
        assert_eq!(
            std::fs::read(d.0.join("openhe.exe.old")).unwrap(),
            b"old exe"
        );
        assert!(!target.staging_dir().exists());
    }

    #[test]
    fn a_bad_download_replaces_nothing() {
        let d = TempDir::new("bad");
        let exe = d.0.join("openhe.exe");
        std::fs::write(&exe, b"old exe").unwrap();
        let s = fake_github("v0.3.0", b"new exe", false);
        let o = offer_for(&s);
        s.insert(o.asset.as_ref().unwrap().url.clone(), b"evil!!!".to_vec());
        let target = InstallTarget::WindowsExe(exe.clone());
        let err = download_and_install(&s, &o, &target, &mut |_| {}, &mut || {}).unwrap_err();
        assert!(matches!(err, UpdateError::Checksum));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old exe");
        assert!(!d.0.join("openhe.exe.old").exists());
        assert!(!target.staging_dir().exists());
    }

    #[test]
    fn an_appimage_install_replaces_the_file() {
        let d = TempDir::new("appimage-install");
        let img = d.0.join("OpenHE.AppImage");
        std::fs::write(&img, b"old").unwrap();
        let s = fake_github("v0.3.0", b"new img", false);
        let target = InstallTarget::AppImage(img.clone());
        download_and_install(&s, &offer_for(&s), &target, &mut |_| {}, &mut || {}).unwrap();
        assert_eq!(std::fs::read(&img).unwrap(), b"new img");
        assert!(!target.staging_dir().exists());
    }

    fn env(s: MemorySource, target: Result<InstallTarget, Unsupported>) -> UpdateEnv {
        UpdateEnv {
            source: Arc::new(s),
            target,
            public_key: key(),
            current: v("0.2.0"),
            platform: Some(PLATFORM),
            live: true,
        }
    }

    fn settle(u: &mut Updater) -> Option<Event> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ev = None;
        while u.is_busy() {
            ev = u.pump().or(ev);
            assert!(Instant::now() < deadline, "updater never finished");
            std::thread::sleep(Duration::from_millis(2));
        }
        ev
    }

    #[test]
    fn the_updater_checks_downloads_and_installs_in_the_background() {
        let d = TempDir::new("updater");
        let exe = d.0.join("openhe.exe");
        std::fs::write(&exe, b"old").unwrap();
        let s = fake_github("v0.3.0", b"new", false);
        let mut u = Updater::new(env(s, Ok(InstallTarget::WindowsExe(exe.clone()))));
        assert_eq!(u.phase(), Phase::Idle);
        u.check_now(false);
        assert_eq!(u.phase(), Phase::Checking);
        assert_eq!(settle(&mut u), Some(Event::Checked));
        assert_eq!(u.phase(), Phase::Checked);
        assert_eq!(u.offer().unwrap().tag, "v0.3.0");
        assert!(u.can_install());
        u.install();
        assert!(matches!(u.phase(), Phase::Downloading { total: 3, .. }));
        settle(&mut u);
        assert_eq!(u.phase(), Phase::Installed, "{:?}", u.error());
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        // Nothing more to check once installed.
        u.check_now(false);
        assert!(!u.is_busy());
    }

    #[test]
    fn a_failed_check_keeps_the_card_usable() {
        let mut u = Updater::new(env(MemorySource::new(), Err(Unsupported::Platform)));
        u.check_now(false);
        assert_eq!(settle(&mut u), None);
        assert_eq!(u.phase(), Phase::Idle);
        let (text, _, install) = u.error().unwrap();
        assert!(text.contains("404"), "{text}");
        assert!(!install);
    }

    #[test]
    fn an_unsupported_copy_is_offered_but_not_installed() {
        let s = fake_github("v0.3.0", b"new", false);
        let mut u = Updater::new(env(s, Err(Unsupported::NotInBundle)));
        u.check_now(false);
        settle(&mut u);
        assert!(u.offer().is_some());
        assert!(!u.can_install());
        u.install();
        assert_eq!(u.phase(), Phase::Checked, "nothing started");
        assert!(!u.restart());
    }

    #[test]
    fn the_automatic_check_runs_once_and_only_when_due() {
        let s = fake_github("v0.3.0", b"new", false);
        let mut u = Updater::new(env(s, Err(Unsupported::Platform)));
        assert!(
            !u.auto_check(true, false, Some(1_000), 1_001),
            "checked recently"
        );
        assert!(!u.auto_check(true, false, None, 1_001), "once per run");
        let s = fake_github("v0.3.0", b"new", false);
        let mut u = Updater::new(env(s, Err(Unsupported::Platform)));
        assert!(u.auto_check(true, false, None, 1_000));
        settle(&mut u);
        assert!(u.offer().is_some());
        // Never without a real source.
        let mut off = Updater::default();
        assert!(!off.auto_check(true, false, None, 1_000));
    }
}

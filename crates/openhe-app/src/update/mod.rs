//! In-app updater: the only part of OpenHE that uses the network. It asks
//! GitHub for the releases of the OpenHE repository, and downloads a newer
//! release when the user asks for it. Nothing else is sent anywhere; there
//! is no telemetry.
//!
//! # Trust
//!
//! Every release carries a manifest (`openhe-update.json`) with the name,
//! size and SHA-256 of each platform's file, and an Ed25519 signature of it
//! (`openhe-update.json.sig`, hex). The release workflow signs it with a
//! key kept in a GitHub secret; the app only accepts manifests that verify
//! against [`PUBLIC_KEY`]. The downloaded file must match the manifest's
//! size and SHA-256 before anything installed is touched. A release without
//! a manifest is never offered.
//!
//! # Flow
//!
//! 1. [`release`]: pick the newest release that is newer than the running
//!    version and has a manifest (pre-releases only on request).
//! 2. [`manifest`]: verify the signature, the version, and pick the entry
//!    of this platform.
//! 3. [`updater`]: download (streamed, with progress, size-limited) into a
//!    staging folder next to the installed app and check it.
//! 4. [`install`]: swap it in (macOS bundle, Windows exe, Linux AppImage),
//!    rolling back on failure, then restart on request.
//!
//! The network sits behind [`source::UpdateSource`], so tests run the whole
//! flow on an in-memory fake and never touch the network.

pub mod install;
pub mod manifest;
pub mod release;
pub mod source;
pub mod updater;

pub use updater::{Phase, UpdateEnv, Updater};

/// The GitHub repository releases come from.
pub const REPO: &str = "BadryansahBangsawan/openhe";

/// The releases API (newest first; 20 is plenty to find the newest stable
/// release among pre-releases).
pub const RELEASES_API: &str =
    "https://api.github.com/repos/BadryansahBangsawan/openhe/releases?per_page=20";

/// The releases page, for manual downloads.
pub const RELEASES_PAGE: &str = "https://github.com/BadryansahBangsawan/openhe/releases";

/// The page of one release (`tag` is a checked `vX.Y.Z[-pre]` tag).
pub fn release_page(tag: &str) -> String {
    format!("{RELEASES_PAGE}/tag/{tag}")
}

/// Asset name of the signed manifest.
pub const MANIFEST_NAME: &str = "openhe-update.json";
/// Asset name of its signature (128 lowercase hex characters).
pub const SIGNATURE_NAME: &str = "openhe-update.json.sig";

/// The raw Ed25519 public key the manifest signature must verify against
/// (its private half is the release workflow's `OPENHE_SIGNING_KEY`).
pub const PUBLIC_KEY: [u8; 32] = [
    0x5f, 0x63, 0x78, 0x61, 0x81, 0x4f, 0xb7, 0x7d, 0xbb, 0xce, 0x83, 0x5a, 0xed, 0x29, 0x01, 0xb3,
    0xa2, 0xa8, 0xf1, 0xae, 0xe7, 0xf8, 0x07, 0x36, 0x24, 0x1b, 0x31, 0x99, 0xc0, 0x20, 0x12, 0x94,
];

/// The running version (`Cargo.toml` of this crate).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Passed to the new app after an update: it waits for this instance to
/// close instead of saying "already running".
pub const AFTER_UPDATE_ARG: &str = "--after-update";

/// Automatic checks run at most this often.
pub const AUTO_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Size limits of what is read from GitHub (a bigger answer is refused).
pub const RELEASES_LIMIT: u64 = 4 * 1024 * 1024;
pub const MANIFEST_LIMIT: u64 = 64 * 1024;
/// The largest app file a manifest may announce.
pub const ASSET_LIMIT: u64 = 512 * 1024 * 1024;

/// Lowercase hex of `bytes`.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0F)]));
    }
    s
}

/// Bytes of a hex string (either case); `None` for an odd length or a
/// non-hex character.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.chunks(2)
        .map(|p| Some(nibble(p[0])? << 4 | nibble(p[1])?))
        .collect()
}

/// The manifest key of a platform (`std::env::consts::OS`, `ARCH`), if
/// releases are built for it.
pub fn platform_key(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// [`platform_key`] of the running build.
pub fn current_platform() -> Option<&'static str> {
    platform_key(std::env::consts::OS, std::env::consts::ARCH)
}

/// Unix seconds now.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An automatic check is due: enabled, and none in the last
/// [`AUTO_CHECK_INTERVAL_SECS`] (a clock that went back counts as due).
pub fn auto_check_due(enabled: bool, last_check: Option<u64>, now: u64) -> bool {
    enabled
        && match last_check {
            None => true,
            Some(t) => now < t || now - t >= AUTO_CHECK_INTERVAL_SECS,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        let bytes = [0x00, 0x5f, 0xab, 0xff];
        assert_eq!(to_hex(&bytes), "005fabff");
        assert_eq!(from_hex("005fabff").unwrap(), bytes);
        assert_eq!(from_hex("005FABFF").unwrap(), bytes);
        assert_eq!(from_hex("").unwrap(), Vec::<u8>::new());
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
        assert!(from_hex("0 ").is_none());
    }

    #[test]
    fn the_embedded_key_is_the_release_key() {
        assert_eq!(
            to_hex(&PUBLIC_KEY),
            "5f637861814fb77dbbce835aed2901b3a2a8f1aee7f80736241b3199c0201294"
        );
        // A valid curve point, or every manifest would be refused.
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&PUBLIC_KEY).is_ok());
    }

    #[test]
    fn platform_keys_match_the_release_assets() {
        assert_eq!(platform_key("macos", "aarch64"), Some("macos-aarch64"));
        assert_eq!(platform_key("windows", "x86_64"), Some("windows-x86_64"));
        assert_eq!(platform_key("linux", "x86_64"), Some("linux-x86_64"));
        // Not built: Intel Macs, ARM Windows and Linux, other systems.
        assert_eq!(platform_key("macos", "x86_64"), None);
        assert_eq!(platform_key("windows", "aarch64"), None);
        assert_eq!(platform_key("linux", "aarch64"), None);
        assert_eq!(platform_key("freebsd", "x86_64"), None);
    }

    #[test]
    fn automatic_checks_run_at_most_once_a_day() {
        let day = AUTO_CHECK_INTERVAL_SECS;
        assert!(auto_check_due(true, None, 1_000));
        assert!(!auto_check_due(false, None, 1_000));
        assert!(!auto_check_due(true, Some(1_000), 1_000 + day - 1));
        assert!(auto_check_due(true, Some(1_000), 1_000 + day));
        // The clock went back: check rather than wait for a far date.
        assert!(auto_check_due(true, Some(1_000), 500));
    }

    #[test]
    fn release_pages_point_at_the_repository() {
        assert_eq!(
            release_page("v0.2.0"),
            "https://github.com/BadryansahBangsawan/openhe/releases/tag/v0.2.0"
        );
        assert!(RELEASES_API.contains(REPO));
        assert!(RELEASES_PAGE.ends_with(&format!("{REPO}/releases")));
    }
}

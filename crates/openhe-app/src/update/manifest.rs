//! The signed update manifest (`openhe-update.json`) of a release:
//!
//! ```json
//! {"version":"0.2.0","assets":{
//!   "macos-aarch64":{"name":"OpenHE-v0.2.0-macos-arm64.zip","sha256":"<hex>","size":123},
//!   "windows-x86_64":{...},"linux-x86_64":{...}}}
//! ```
//!
//! Its signature (`openhe-update.json.sig`) is a pure Ed25519 signature of
//! the manifest's exact bytes, as `openssl pkeyutl -sign -rawin` makes it,
//! written as 128 hex characters (a trailing newline is allowed).

use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Why a manifest was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The signature file is not 128 hex characters.
    SignatureFormat,
    /// The signature does not verify against the key (or the key is not a
    /// valid point).
    BadSignature,
    /// Signed, but not a manifest we understand (JSON, version, hashes).
    Invalid(String),
}

/// One platform's file in a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestAsset {
    /// The release asset's name.
    pub name: String,
    pub sha256: [u8; 32],
    pub size: u64,
}

/// A verified manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    /// By platform key ([`super::platform_key`]).
    pub assets: BTreeMap<String, ManifestAsset>,
}

#[derive(Deserialize)]
struct RawManifest {
    version: String,
    assets: BTreeMap<String, RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    sha256: String,
    size: u64,
}

impl Manifest {
    /// Parses manifest JSON (unknown fields are ignored, for later
    /// additions). Checks the version, every hash and size.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        let raw: RawManifest = serde_json::from_slice(bytes)
            .map_err(|e| ManifestError::Invalid(format!("json: {e}")))?;
        let version = Version::parse(&raw.version)
            .map_err(|e| ManifestError::Invalid(format!("version {:?}: {e}", raw.version)))?;
        let mut assets = BTreeMap::new();
        for (key, a) in raw.assets {
            let sha256: [u8; 32] = super::from_hex(&a.sha256)
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| ManifestError::Invalid(format!("{key}: sha256 {:?}", a.sha256)))?;
            if a.name.is_empty() || a.size == 0 || a.size > super::ASSET_LIMIT {
                return Err(ManifestError::Invalid(format!(
                    "{key}: name {:?} size {}",
                    a.name, a.size
                )));
            }
            assets.insert(
                key,
                ManifestAsset {
                    name: a.name,
                    sha256,
                    size: a.size,
                },
            );
        }
        Ok(Manifest { version, assets })
    }

    /// The entry of `platform`, if the release has one.
    pub fn asset(&self, platform: &str) -> Option<&ManifestAsset> {
        self.assets.get(platform)
    }
}

/// The 64 signature bytes of a `.sig` file: 128 hex characters, surrounding
/// whitespace (a trailing newline) ignored.
pub fn parse_signature(text: &[u8]) -> Result<[u8; 64], ManifestError> {
    let text = std::str::from_utf8(text).map_err(|_| ManifestError::SignatureFormat)?;
    super::from_hex(text.trim())
        .and_then(|v| v.try_into().ok())
        .ok_or(ManifestError::SignatureFormat)
}

/// Checks `signature` (the `.sig` file) over the exact `manifest` bytes
/// with `key` (strict Ed25519: no malleable or small-order signatures),
/// then parses the manifest. Nothing of an unsigned manifest is read.
pub fn verify(
    manifest: &[u8],
    signature: &[u8],
    key: &[u8; 32],
) -> Result<Manifest, ManifestError> {
    let sig = Signature::from_bytes(&parse_signature(signature)?);
    let key = VerifyingKey::from_bytes(key).map_err(|_| ManifestError::BadSignature)?;
    key.verify_strict(manifest, &sig)
        .map_err(|_| ManifestError::BadSignature)?;
    Manifest::parse(manifest)
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Feeds a running SHA-256 and counts bytes (the download hashes as it
/// writes, so a big file is read once).
#[derive(Default)]
pub struct Hasher {
    sha: Sha256,
    len: u64,
}

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        self.sha.update(bytes);
        self.len += bytes.len() as u64;
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn finish(self) -> [u8; 32] {
        self.sha.finalize().into()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// A fixed test key pair (never the release key).
    pub(crate) fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// `manifest` signed as the release workflow does: hex and a newline.
    pub(crate) fn sign(key: &SigningKey, manifest: &[u8]) -> Vec<u8> {
        format!("{}\n", super::super::to_hex(&key.sign(manifest).to_bytes())).into_bytes()
    }

    pub(crate) fn manifest_json(version: &str) -> String {
        format!(
            r#"{{"version":"{version}","assets":{{
              "macos-aarch64":{{"name":"OpenHE-v{version}-macos-arm64.zip","sha256":"{a}","size":10}},
              "windows-x86_64":{{"name":"OpenHE-v{version}-windows-x86_64.exe","sha256":"{b}","size":20}},
              "linux-x86_64":{{"name":"OpenHE-v{version}-linux-x86_64.AppImage","sha256":"{c}","size":30}}}}}}"#,
            a = "aa".repeat(32),
            b = "bb".repeat(32),
            c = "cc".repeat(32),
        )
    }

    #[test]
    fn a_manifest_parses_with_every_platform() {
        let m = Manifest::parse(manifest_json("0.2.0").as_bytes()).unwrap();
        assert_eq!(m.version, Version::new(0, 2, 0));
        assert_eq!(m.assets.len(), 3);
        let win = m.asset("windows-x86_64").unwrap();
        assert_eq!(win.name, "OpenHE-v0.2.0-windows-x86_64.exe");
        assert_eq!(win.sha256, [0xbb; 32]);
        assert_eq!(win.size, 20);
        assert!(m.asset("linux-aarch64").is_none());
        // Fields added later are ignored.
        let extra = r#"{"version":"1.0.0","notes":"x","assets":{}}"#;
        assert!(Manifest::parse(extra.as_bytes()).unwrap().assets.is_empty());
    }

    #[test]
    fn a_broken_manifest_is_refused() {
        for bad in [
            "not json",
            r#"{"assets":{}}"#,
            r#"{"version":"v0.2.0","assets":{}}"#,
            r#"{"version":"0.2","assets":{}}"#,
            r#"{"version":"0.2.0","assets":{"linux-x86_64":{"name":"a","sha256":"abc","size":1}}}"#,
            r#"{"version":"0.2.0","assets":{"linux-x86_64":{"name":"a","sha256":"zz","size":1}}}"#,
            r#"{"version":"0.2.0","assets":{"linux-x86_64":{"name":"","sha256":"00","size":1}}}"#,
        ] {
            assert!(
                matches!(
                    Manifest::parse(bad.as_bytes()),
                    Err(ManifestError::Invalid(_))
                ),
                "{bad}"
            );
        }
        let zero = format!(
            r#"{{"version":"0.2.0","assets":{{"x":{{"name":"a","sha256":"{}","size":0}}}}}}"#,
            "00".repeat(32)
        );
        assert!(Manifest::parse(zero.as_bytes()).is_err(), "empty file");
    }

    /// The format the release workflow writes: ed25519-dalek's signature
    /// is the same pure Ed25519 as `openssl pkeyutl -sign -rawin`.
    #[test]
    fn a_signed_manifest_verifies() {
        let key = test_key();
        let json = manifest_json("0.3.0");
        let sig = sign(&key, json.as_bytes());
        assert_eq!(sig.len(), 129, "128 hex + newline");
        let m = verify(json.as_bytes(), &sig, key.verifying_key().as_bytes()).unwrap();
        assert_eq!(m.version, Version::new(0, 3, 0));
        // Without the newline, and in upper case, too.
        let upper = String::from_utf8(sig.clone())
            .unwrap()
            .trim()
            .to_uppercase();
        assert!(verify(
            json.as_bytes(),
            upper.as_bytes(),
            key.verifying_key().as_bytes()
        )
        .is_ok());
    }

    /// Made with OpenSSL 3 exactly as the release workflow does
    /// (`genpkey -algorithm ed25519`, `pkeyutl -sign -rawin`, `od` to hex):
    /// the app reads that format.
    #[test]
    fn an_openssl_made_signature_verifies() {
        let key: [u8; 32] = super::super::from_hex(
            "b0175cd8f3df7aaaad628f4b68e607deecd0d3b2ef3f608f54ecb8b303062f29",
        )
        .unwrap()
        .try_into()
        .unwrap();
        let json = br#"{"version":"0.3.0","assets":{"linux-x86_64":{"name":"OpenHE-v0.3.0-linux-x86_64.AppImage","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","size":3}}}"#;
        let sig = b"4f5f341fb1b4974f0b7e44d5ce16c4072eff4e0dbc353b202cfd42ccbd5e37fc\
cae6e9b741a6f23c9988be30982ae3879c1084d446dc353116582bd92432850b\n";
        let m = verify(json, sig, &key).unwrap();
        assert_eq!(m.version, Version::new(0, 3, 0));
        assert_eq!(m.asset("linux-x86_64").unwrap().sha256, sha256(b"abc"));
        let mut tampered = json.to_vec();
        tampered.push(b' ');
        assert_eq!(
            verify(&tampered, sig, &key),
            Err(ManifestError::BadSignature)
        );
    }

    #[test]
    fn a_tampered_manifest_or_signature_is_refused() {
        let key = test_key();
        let json = manifest_json("0.3.0");
        let sig = sign(&key, json.as_bytes());
        let pubkey = *key.verifying_key().as_bytes();
        // One byte of the manifest changed.
        let tampered = json.replacen("0.3.0", "0.4.0", 1);
        assert_eq!(
            verify(tampered.as_bytes(), &sig, &pubkey),
            Err(ManifestError::BadSignature)
        );
        // Another key.
        let other = SigningKey::from_bytes(&[9u8; 32]);
        assert_eq!(
            verify(json.as_bytes(), &sign(&other, json.as_bytes()), &pubkey),
            Err(ManifestError::BadSignature)
        );
        // The release key does not accept a test signature.
        assert_eq!(
            verify(json.as_bytes(), &sig, &super::super::PUBLIC_KEY),
            Err(ManifestError::BadSignature)
        );
        // One signature bit flipped.
        let mut flipped = sig.clone();
        flipped[0] = if flipped[0] == b'0' { b'1' } else { b'0' };
        assert_eq!(
            verify(json.as_bytes(), &flipped, &pubkey),
            Err(ManifestError::BadSignature)
        );
        // Not a signature at all.
        for bad in [&b""[..], b"abcd", &[b'g'; 128], &sig[..100]] {
            assert_eq!(
                verify(json.as_bytes(), bad, &pubkey),
                Err(ManifestError::SignatureFormat)
            );
        }
    }

    #[test]
    fn a_valid_signature_over_a_bad_manifest_is_still_refused() {
        let key = test_key();
        let json = br#"{"version":"x"}"#;
        let sig = sign(&key, json);
        assert!(matches!(
            verify(json, &sig, key.verifying_key().as_bytes()),
            Err(ManifestError::Invalid(_))
        ));
    }

    #[test]
    fn the_running_hash_matches_the_one_shot_hash() {
        let data: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
        let mut h = Hasher::default();
        assert!(h.is_empty());
        for chunk in data.chunks(777) {
            h.update(chunk);
        }
        assert_eq!(h.len(), 10_000);
        assert_eq!(h.finish(), sha256(&data));
        // The well-known SHA-256 of "abc".
        assert_eq!(
            super::super::to_hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

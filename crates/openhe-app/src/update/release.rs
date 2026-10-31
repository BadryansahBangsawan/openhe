//! The GitHub releases list, and which release (if any) to offer.

use semver::Version;
use serde::Deserialize;

/// One release as the GitHub API lists it (only the fields used).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    /// The release notes (Markdown), shown as plain text.
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

/// A file of a release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
}

impl GhRelease {
    /// The download URL of the asset named `name`.
    pub fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.as_str())
    }

    /// A pre-release: so marked on GitHub, or a `-beta.1` style version.
    pub fn is_prerelease(&self, version: &Version) -> bool {
        self.prerelease || !version.pre.is_empty()
    }
}

/// Parses the API's JSON array.
pub fn parse_releases(bytes: &[u8]) -> Result<Vec<GhRelease>, serde_json::Error> {
    serde_json::from_slice(bytes)
}

/// The version of a `vX.Y.Z` (or `vX.Y.Z-pre`) tag; `None` for any other
/// tag.
pub fn tag_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v')?).ok()
}

/// The release to offer, with its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub release: &'a GhRelease,
    pub version: Version,
    pub manifest_url: &'a str,
    pub signature_url: &'a str,
}

/// The newest release newer than `current` that carries a manifest and its
/// signature. Drafts never count; pre-releases only with `beta`. Tags that
/// are not `vX.Y.Z` are skipped.
pub fn select<'a>(
    releases: &'a [GhRelease],
    current: &Version,
    beta: bool,
) -> Option<Candidate<'a>> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| {
            let version = tag_version(&r.tag_name)?;
            if r.is_prerelease(&version) && !beta {
                return None;
            }
            if version <= *current {
                return None;
            }
            Some(Candidate {
                release: r,
                version,
                manifest_url: r.asset_url(super::MANIFEST_NAME)?,
                signature_url: r.asset_url(super::SIGNATURE_NAME)?,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release list as the API returns it (trimmed): drafts, pre-releases,
    /// one without a manifest, an odd tag.
    const FIXTURE: &str = r#"[
      {"tag_name":"v0.5.0","draft":true,"prerelease":false,"body":"draft",
       "assets":[{"name":"openhe-update.json","browser_download_url":"https://x/d.json","size":1},
                 {"name":"openhe-update.json.sig","browser_download_url":"https://x/d.sig","size":1}]},
      {"tag_name":"v0.4.0-beta.1","draft":false,"prerelease":true,"body":"beta",
       "assets":[{"name":"openhe-update.json","browser_download_url":"https://x/b.json","size":1},
                 {"name":"openhe-update.json.sig","browser_download_url":"https://x/b.sig","size":1}]},
      {"tag_name":"v0.3.1","draft":false,"prerelease":false,"body":"no manifest",
       "assets":[{"name":"OpenHE-v0.3.1-linux-x86_64.AppImage","browser_download_url":"https://x/a","size":1}]},
      {"tag_name":"nightly","draft":false,"prerelease":false,
       "assets":[{"name":"openhe-update.json","browser_download_url":"https://x/n.json","size":1},
                 {"name":"openhe-update.json.sig","browser_download_url":"https://x/n.sig","size":1}]},
      {"tag_name":"v0.3.0","draft":false,"prerelease":false,"body":"stable","html_url":"ignored",
       "assets":[{"name":"openhe-update.json","browser_download_url":"https://x/s.json","size":1},
                 {"name":"openhe-update.json.sig","browser_download_url":"https://x/s.sig","size":1}]},
      {"tag_name":"v0.1.0","draft":false,"prerelease":true,"body":null,"assets":[]}
    ]"#;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn the_api_list_parses() {
        let r = parse_releases(FIXTURE.as_bytes()).unwrap();
        assert_eq!(r.len(), 6);
        assert!(r[0].draft);
        assert_eq!(r[5].body, None);
        assert_eq!(
            r[4].asset_url("openhe-update.json"),
            Some("https://x/s.json")
        );
        assert!(parse_releases(b"{\"message\":\"Not Found\"}").is_err());
    }

    #[test]
    fn the_newest_stable_release_with_a_manifest_is_offered() {
        let r = parse_releases(FIXTURE.as_bytes()).unwrap();
        // v0.5.0 is a draft, v0.4.0-beta.1 a pre-release, v0.3.1 has no
        // manifest, "nightly" is no version.
        let c = select(&r, &v("0.2.0"), false).unwrap();
        assert_eq!(c.version, v("0.3.0"));
        assert_eq!(c.release.tag_name, "v0.3.0");
        assert_eq!(c.manifest_url, "https://x/s.json");
        assert_eq!(c.signature_url, "https://x/s.sig");
    }

    #[test]
    fn pre_releases_only_when_asked() {
        let r = parse_releases(FIXTURE.as_bytes()).unwrap();
        let c = select(&r, &v("0.2.0"), true).unwrap();
        assert_eq!(c.version, v("0.4.0-beta.1"));
        // A "-beta" tag counts as a pre-release even when not marked so.
        let mut unmarked = r.clone();
        unmarked[1].prerelease = false;
        assert_eq!(
            select(&unmarked, &v("0.2.0"), false).unwrap().version,
            v("0.3.0")
        );
    }

    #[test]
    fn nothing_older_or_equal_is_offered() {
        let r = parse_releases(FIXTURE.as_bytes()).unwrap();
        assert!(select(&r, &v("0.3.0"), false).is_none(), "equal");
        assert!(select(&r, &v("0.9.0"), true).is_none(), "older");
        // A beta of the running version is older than the release itself.
        assert!(select(&r, &v("0.4.0"), true).is_none());
        assert!(select(&[], &v("0.1.0"), true).is_none());
    }

    #[test]
    fn a_release_needs_both_manifest_files() {
        let one = GhRelease {
            tag_name: "v9.0.0".into(),
            assets: vec![GhAsset {
                name: "openhe-update.json".into(),
                browser_download_url: "https://x/m".into(),
            }],
            ..Default::default()
        };
        assert!(select(&[one], &v("0.1.0"), false).is_none());
    }

    #[test]
    fn only_v_semver_tags_are_versions() {
        assert_eq!(tag_version("v1.2.3"), Some(v("1.2.3")));
        assert_eq!(tag_version("v1.2.3-rc.1"), Some(v("1.2.3-rc.1")));
        for bad in ["1.2.3", "v1.2", "release-1", "v", "V1.2.3", "v01.2.3"] {
            assert_eq!(tag_version(bad), None, "{bad}");
        }
    }
}

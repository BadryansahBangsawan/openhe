//! Where updates come from: [`GithubSource`] over HTTPS in the app (the
//! only network code of OpenHE), [`MemorySource`] in tests.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::Duration;

/// Why a download failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// No connection, DNS, TLS, a timeout, a broken transfer.
    Network(String),
    /// The server answered with this HTTP status (403/429: rate limited).
    Status(u16),
    /// The answer was bigger than allowed; the transfer was stopped.
    TooLarge,
    /// Writing what was downloaded failed (disk full, ...).
    Write(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Network(e) => write!(f, "network: {e}"),
            FetchError::Status(s) => write!(f, "HTTP {s}"),
            FetchError::TooLarge => write!(f, "response too large"),
            FetchError::Write(e) => write!(f, "write: {e}"),
        }
    }
}

/// Fetches update files. One method, so a fake is a few lines.
pub trait UpdateSource: Send + Sync {
    /// Streams `url` into `out`, at most `limit` bytes (more is
    /// [`FetchError::TooLarge`]), calling `progress` with the bytes so far.
    fn get(
        &self,
        url: &str,
        limit: u64,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), FetchError>;

    /// [`UpdateSource::get`] into memory, for the small files.
    fn get_bytes(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        let mut buf = Vec::new();
        self.get(url, limit, &mut buf, &mut |_| {})?;
        Ok(buf)
    }
}

/// Copies `body` into `out` in chunks, enforcing `limit`.
pub fn copy_limited(
    body: &mut dyn Read,
    limit: u64,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(u64),
) -> Result<(), FetchError> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FetchError::Network(e.to_string())),
        };
        total += n as u64;
        if total > limit {
            return Err(FetchError::TooLarge);
        }
        out.write_all(&buf[..n])
            .map_err(|e| FetchError::Write(e.to_string()))?;
        progress(total);
    }
}

/// HTTPS to GitHub (api.github.com for the list, the release download
/// redirects for files). rustls with the bundled Mozilla roots; no
/// cookies, no other headers than these.
pub struct GithubSource {
    agent: ureq::Agent,
}

impl Default for GithubSource {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(format!("OpenHE/{}", super::CURRENT_VERSION))
            .https_only(true)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            // A stalled transfer fails; a slow but moving one goes on.
            .timeout_per_read(Some(Duration::from_secs(30)))
            .build();
        GithubSource {
            agent: config.into(),
        }
    }
}

impl UpdateSource for GithubSource {
    fn get(
        &self,
        url: &str,
        limit: u64,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), FetchError> {
        let accept = if url.starts_with("https://api.github.com/") {
            "application/vnd.github+json"
        } else {
            "application/octet-stream"
        };
        let mut req = self.agent.get(url).header("Accept", accept);
        if accept != "application/octet-stream" {
            req = req.header("X-GitHub-Api-Version", "2022-11-28");
        }
        let resp = req.call().map_err(|e| match e {
            ureq::Error::StatusCode(s) => FetchError::Status(s),
            e => FetchError::Network(e.to_string()),
        })?;
        let mut body = resp.into_body().into_reader();
        copy_limited(&mut body, limit, out, progress)
    }
}

/// An in-memory source: URL → bytes; every other URL is a 404. Logs the
/// URLs asked for. For tests (no network).
#[derive(Default)]
pub struct MemorySource {
    files: Mutex<HashMap<String, Vec<u8>>>,
    log: Mutex<Vec<String>>,
}

impl MemorySource {
    pub fn new() -> MemorySource {
        MemorySource::default()
    }

    /// Serves `bytes` at `url`.
    pub fn insert(&self, url: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(url.into(), bytes.into());
    }

    /// Stops serving `url`.
    pub fn remove(&self, url: &str) {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(url);
    }

    /// The URLs asked for, in order.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl UpdateSource for MemorySource {
    fn get(
        &self,
        url: &str,
        limit: u64,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), FetchError> {
        self.log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(url.to_string());
        let bytes = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(url)
            .cloned()
            .ok_or(FetchError::Status(404))?;
        copy_limited(&mut bytes.as_slice(), limit, out, progress)
    }
}

/// No updates at all (the default outside the real app: tests that do not
/// inject a source never reach the network).
pub struct NoSource;

impl UpdateSource for NoSource {
    fn get(
        &self,
        _url: &str,
        _limit: u64,
        _out: &mut dyn Write,
        _progress: &mut dyn FnMut(u64),
    ) -> Result<(), FetchError> {
        Err(FetchError::Network("updates are off in this build".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_stops_past_the_limit() {
        let data = vec![1u8; 200_000];
        let mut out = Vec::new();
        let mut seen = Vec::new();
        copy_limited(&mut data.as_slice(), 200_000, &mut out, &mut |n| {
            seen.push(n)
        })
        .unwrap();
        assert_eq!(out.len(), 200_000);
        assert_eq!(seen.last(), Some(&200_000));
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "progress only grows");
        let mut out = Vec::new();
        assert_eq!(
            copy_limited(&mut data.as_slice(), 199_999, &mut out, &mut |_| {}),
            Err(FetchError::TooLarge)
        );
    }

    #[test]
    fn the_memory_source_serves_and_logs() {
        let s = MemorySource::new();
        s.insert("https://x/a", b"hello".to_vec());
        assert_eq!(s.get_bytes("https://x/a", 10).unwrap(), b"hello");
        assert_eq!(s.get_bytes("https://x/b", 10), Err(FetchError::Status(404)));
        assert_eq!(s.get_bytes("https://x/a", 4), Err(FetchError::TooLarge));
        assert_eq!(s.requests(), ["https://x/a", "https://x/b", "https://x/a"]);
        s.remove("https://x/a");
        assert!(s.get_bytes("https://x/a", 10).is_err());
        assert!(NoSource.get_bytes("https://x/a", 10).is_err());
    }
}

//! Operating-system hooks of the Pengaturan page: the foreground-app
//! watcher for per-app profile switching, and launch at login. Both are
//! optional (off by default) and never touch the keyboard themselves.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the foreground app is checked.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// What the watcher saw last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foreground {
    /// The app in front right now (`None` while OpenHE itself is in front,
    /// or when it cannot be read).
    pub current: Option<String>,
    /// The last app other than OpenHE that was in front (for "add this
    /// app").
    pub last_other: Option<String>,
}

/// A light background thread that reads the foreground app's name every
/// second and hands it to a `tick` callback (the per-app profile switcher),
/// so switching works without UI frames. Stops when dropped.
pub struct ForegroundWatcher {
    seen: Arc<Mutex<Foreground>>,
    stop: Arc<AtomicBool>,
}

impl ForegroundWatcher {
    /// Starts watching; `changed` is called (from the watcher thread) when
    /// the foreground app changes, so the UI can wake up, and `tick` once
    /// per check with the app in front. A panicking `tick` is contained.
    pub fn spawn(
        changed: impl Fn() + Send + 'static,
        mut tick: impl FnMut(Option<&str>) + Send + 'static,
    ) -> std::io::Result<ForegroundWatcher> {
        let seen = Arc::new(Mutex::new(Foreground::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (Arc::clone(&seen), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("openhe-foreground".into())
            .spawn(move || {
                let me = std::process::id();
                while !st.load(Ordering::SeqCst) {
                    let now = foreground_app(me);
                    let moved = {
                        let mut g = s.lock().unwrap_or_else(|p| p.into_inner());
                        let moved = g.current != now;
                        if now.is_some() {
                            g.last_other.clone_from(&now);
                        }
                        g.current = now;
                        moved
                    };
                    if moved {
                        changed();
                    }
                    let current = s.lock().unwrap_or_else(|p| p.into_inner()).current.clone();
                    let _ = catch_unwind(AssertUnwindSafe(|| tick(current.as_deref())));
                    std::thread::sleep(WATCH_INTERVAL);
                }
            })?;
        Ok(ForegroundWatcher { seen, stop })
    }

    pub fn seen(&self) -> Foreground {
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The foreground app's name, unless it is this process. A failing or
/// panicking platform call reads as "unknown".
fn foreground_app(me: u32) -> Option<String> {
    let w = catch_unwind(AssertUnwindSafe(active_win_pos_rs::get_active_window))
        .ok()?
        .ok()?;
    if w.process_id == u64::from(me) {
        return None;
    }
    let name = w.app_name.trim();
    if !name.is_empty() {
        return Some(name.to_string());
    }
    w.process_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

/// Turns launch at login on or off for the running executable (a
/// LaunchAgent on macOS, the Run registry key on Windows, an XDG autostart
/// entry on Linux). The error text is technical (English).
pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe
        .to_str()
        .ok_or_else(|| "executable path is not UTF-8".to_string())?;
    let auto = auto_launch::AutoLaunchBuilder::new()
        .set_app_name(crate::i18n::common::APP_NAME)
        .set_app_path(path)
        .build()
        .map_err(|e| e.to_string())?;
    if !on && !auto.is_enabled().unwrap_or(true) {
        return Ok(()); // nothing to remove
    }
    let r = if on { auto.enable() } else { auto.disable() };
    r.map_err(|e| e.to_string())
}

//! Shared harness for the end-to-end UI tests: the real `OpenHeApp`, run
//! headless with `egui_kittest`, on a worker whose connector opens the
//! keyboard simulator ([`sim`]).
//!
//! Safety: the app saves its data file under the OS config directory, which
//! `directories` derives from `$HOME`. Before any app is built, `$HOME` is
//! pointed at a fresh temporary directory (once per test binary), so a test
//! can never touch the user's real data. Tests run one at a time (a global
//! lock), because they share that directory.

#![allow(dead_code)]

pub mod sim;

use egui_kittest::kittest::NodeT;
use egui_kittest::Harness;
use openhe_app::app::OpenHeApp;
use openhe_app::storage::AppData;
use openhe_app::worker::{
    AppTransport, ConnectionStatus, DeviceSnapshot, Worker, WorkerConfig, WorkerHandle,
};
pub use sim::{Sim, SimState};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// The simulator's serial number (its SYNC reply).
pub const SIM_SERIAL: &str = "A68M240193000000";

static HOME: OnceLock<PathBuf> = OnceLock::new();
static SERIAL: Mutex<()> = Mutex::new(());

/// Points `$HOME` at a private temp dir (once) and checks the app's data
/// directory is inside it.
pub fn private_home() -> &'static PathBuf {
    let home = HOME.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "openhe-e2e-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join(".config"));
        std::env::set_var("APPDATA", dir.join("AppData"));
        dir
    });
    let data = AppData::default_dir().expect("config dir");
    assert!(
        data.starts_with(home),
        "data dir {data:?} is not under the test home {home:?}"
    );
    home
}

/// Fast worker timings for tests (the state poll stays off unless asked).
pub fn fast_config() -> WorkerConfig {
    WorkerConfig {
        retry_interval: Duration::from_millis(20),
        idle_poll: Duration::from_millis(5),
        state_poll: Duration::from_secs(3600),
    }
}

pub struct World {
    pub sim: Sim,
    pub h: Harness<'static, OpenHeApp>,
    pub worker: WorkerHandle<AppTransport>,
    _serial: MutexGuard<'static, ()>,
}

impl World {
    /// The app on a factory keyboard, connected and loaded.
    pub fn new() -> World {
        World::with(Sim::new(), AppData::default(), fast_config())
    }

    /// The app on `sim` with `data`. The simulator's colours count as
    /// chosen (Gelap) unless `data` says otherwise, so the first-connect
    /// dialog stays out of the way; see [`World::first_connect`].
    pub fn with(sim: Sim, mut data: AppData, cfg: WorkerConfig) -> World {
        data.keyboard_skins
            .entry(SIM_SERIAL.to_string())
            .or_default();
        World::build(sim, data, cfg)
    }

    /// The app on a factory keyboard it has never seen: no colours stored,
    /// so the first-connect dialog shows.
    pub fn first_connect(data: AppData) -> World {
        World::build(Sim::new(), data, fast_config())
    }

    fn build(sim: Sim, data: AppData, cfg: WorkerConfig) -> World {
        let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        private_home();
        // Start every test from an empty data file.
        let dir = AppData::default_dir().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let worker: Worker = Worker::spawn_with(sim.connector(), cfg);
        let handle = worker.handle();
        let app = OpenHeApp::with_worker(worker, data);
        let h = Harness::builder()
            .with_size(eframe::egui::vec2(1600.0, 1500.0))
            .build_eframe(move |cc| {
                openhe_app::theme::apply(&cc.egui_ctx);
                app
            });
        let mut w = World {
            sim,
            h,
            worker: handle,
            _serial: serial,
        };
        w.wait_connected();
        w
    }

    /// Runs frames (with a short real-time pause between them, so the
    /// worker thread gets on) until `f` holds; panics after `secs`.
    pub fn until(&mut self, what: &str, secs: u64, mut f: impl FnMut(&mut World) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            self.h.step();
            if f(self) {
                // One more frame so the UI shows the new state.
                self.h.step();
                return;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for: {what}");
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    /// A few frames with pauses, for the worker to settle.
    pub fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn wait_connected(&mut self) {
        self.until("connected and loaded", 20, |w| {
            w.worker.snapshot().is_some() && !w.worker.is_busy()
        });
    }

    /// Waits until no job is queued or running (and no reconnect pending).
    pub fn idle(&mut self) {
        self.until("worker idle", 30, |w| {
            !w.worker.is_busy() && !w.worker.is_restarting() && w.worker.snapshot().is_some()
        });
        self.frames(3);
    }

    pub fn snapshot(&self) -> DeviceSnapshot {
        self.worker.snapshot().expect("connected")
    }

    pub fn sim(&self) -> MutexGuard<'_, SimState> {
        self.sim.state()
    }

    pub fn app(&self) -> &OpenHeApp {
        self.h.state()
    }

    pub fn app_mut(&mut self) -> &mut OpenHeApp {
        self.h.state_mut()
    }
}

pub(crate) fn collect(node: &egui_kittest::Node<'_>, out: &mut Vec<String>) {
    if let Some(l) = node.accesskit_node().label() {
        if !l.is_empty() {
            out.push(l);
        }
    }
    if let Some(v) = node.accesskit_node().value() {
        if !v.is_empty() {
            out.push(format!("={v}"));
        }
    }
    for c in node.children() {
        collect(&c, out);
    }
}

/// The status is connected with a loaded snapshot.
pub fn is_connected(s: &ConnectionStatus) -> bool {
    matches!(s, ConnectionStatus::Connected(_))
}
pub mod ui;

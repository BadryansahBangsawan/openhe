//! The device worker: one background thread that owns the keyboard.
//!
//! - While no keyboard is open it tries to connect every second
//!   (`HidTransport::open_first`), and fails any submitted job with
//!   `Err(Disconnected)` straight away.
//! - On connect it loads a [`DeviceSnapshot`] (handshake, system mode,
//!   profile, polling rate, matrix, and the settings of every matrix key). A
//!   keyboard in bootloader mode is refused after the handshake, with nothing
//!   written.
//! - While idle (and at least every `idle_poll` between jobs) it drains the
//!   keyboard's own reports with `Keyboard::poll_events`: a profile or Win/Mac
//!   change made on the keyboard patches the snapshot at once and schedules a
//!   (debounced) full reload, key-data changes schedule a reload, lighting
//!   changes bump [`Worker::lighting_generation`], and anything else is
//!   ignored.
//! - The keyboard sends no event when Fn+A / Fn+S switches Win/Mac, so the
//!   idle worker also re-reads the profile and the OS mode every
//!   [`WorkerConfig::state_poll`] (2 s), only when no job is queued, no
//!   reload is pending and calibration is not running. A change is handled
//!   like the matching event: snapshot patched at once, debounced reload.
//! - Pages talk to the keyboard only through [`Worker::submit`]: a closure
//!   that runs on the worker thread with `&mut Keyboard`, and a [`Pending`]
//!   handle the UI polls each frame without blocking.
//! - A job that ends in `Err(Disconnected)` drops the keyboard: the status
//!   goes back to `Searching` (before the job's result is delivered) and the
//!   worker reconnects on its own.
//! - A job (or connector, or snapshot load) that panics is caught: its
//!   result is `Err(Io("internal error: ..."))`, the keyboard is dropped and
//!   the worker reconnects. If the thread ever ends while the handle is
//!   alive, the status becomes `Error`.
//! - A background reload that fails keeps the old snapshot, raises a
//!   [`WorkerNotice`] for the shell, and is retried with backoff.
//! - Polling jobs (live depth, calibration) go through
//!   [`Worker::submit_quiet`], so they never mark the app busy.
//! - [`Worker::reconnect`] drops the keyboard on purpose (after a polling-rate
//!   change, a factory reset or a profile switch, when the keyboard
//!   re-enumerates) and connects again after a pause;
//!   [`Worker::is_restarting`] is true until that connect worked.
//! - [`Worker::set_calibration_guard`]: while set, a keyboard that is
//!   (re)connected is sent `stop_calibration` right after its handshake
//!   (never to a keyboard in bootloader mode), and one still open when the
//!   app closes is sent it too, so a lost connection never leaves the
//!   keyboard stuck in calibration mode (where it types nothing). The app
//!   treats a set guard as "calibrating": writes stay locked until a stop
//!   worked.
//!
//! The transport is a type parameter with a connector closure, so tests drive
//! the worker with `MockTransport`; the app uses the `HidTransport` default.

use crate::i18n;
use openhe_device::{DeviceEvent, Error, HidTransport, KeySettings, Keyboard, Transport};
use openhe_proto::{DeviceInfo, Matrix, Model, PollingRate, SystemMode};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Everything the UI needs to draw, read once on connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSnapshot {
    pub model: Model,
    pub info: DeviceInfo,
    pub system_mode: Option<SystemMode>,
    /// Active onboard profile, 0-based.
    pub profile: u8,
    pub polling: PollingRate,
    pub matrix: Matrix,
    /// Settings of every key in `matrix`, by default keycode. A key whose
    /// settings could not be parsed is missing (see
    /// [`WorkerNotice::KeysSkipped`]).
    pub keys: BTreeMap<u8, KeySettings>,
}

/// `Connected` carries the snapshot by value, as the shared contract says;
/// one status lives in the worker and clones are rare, so its size is fine.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// No keyboard open; retrying every second.
    Searching,
    Connected(DeviceSnapshot),
    /// The last connect attempt failed in a way the user may need to fix
    /// (permissions, the keyboard open in another program, bootloader mode,
    /// a protocol error). Still retrying.
    /// The text is Indonesian (from [`i18n::device_error`]).
    Error(String),
}

/// Something the shell should show while connected: the values on screen
/// may be stale or incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerNotice {
    /// A background reload failed; the old snapshot is kept and the reload
    /// is retried with backoff (or at once via [`Worker::request_reload`]).
    ReloadFailed(Error),
    /// Keys whose settings could not be parsed; they are left out of the
    /// snapshot so the rest of the keyboard still works.
    KeysSkipped(Vec<(u8, Error)>),
    /// Reading unsolicited reports failed; a reload is scheduled.
    EventsFailed(Error),
}

/// The notice slots behind [`Worker::notice`]. Each slot is set and cleared
/// by its own cause, so one notice never replaces another back and forth:
///
/// - `reload_failed`: cleared by the next successful load;
/// - `events_failed`: cleared by the next successful event poll;
/// - `skipped`: replaced by every successful load; the user can dismiss it,
///   and it stays dismissed until the set of skipped keys changes.
///
/// Shown in that order of priority.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notices {
    pub reload_failed: Option<Error>,
    pub events_failed: Option<Error>,
    pub skipped: Vec<(u8, Error)>,
    pub skipped_dismissed: bool,
}

impl Notices {
    pub fn current(&self) -> Option<WorkerNotice> {
        if let Some(e) = &self.reload_failed {
            return Some(WorkerNotice::ReloadFailed(e.clone()));
        }
        if let Some(e) = &self.events_failed {
            return Some(WorkerNotice::EventsFailed(e.clone()));
        }
        if !self.skipped.is_empty() && !self.skipped_dismissed {
            return Some(WorkerNotice::KeysSkipped(self.skipped.clone()));
        }
        None
    }

    /// A load succeeded with these skipped keys.
    pub fn loaded(&mut self, skipped: Vec<(u8, Error)>) {
        self.reload_failed = None;
        if skipped != self.skipped {
            self.skipped_dismissed = false;
        }
        self.skipped = skipped;
    }

    /// Hides the current notice if it is the skipped-keys one (the others
    /// clear themselves once their cause is gone).
    pub fn dismiss(&mut self) {
        if matches!(self.current(), Some(WorkerNotice::KeysSkipped(_))) {
            self.skipped_dismissed = true;
        }
    }
}

/// What a batch of keyboard events asks the worker to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventEffects {
    /// The newest profile reported by the keyboard (patched in at once).
    pub profile: Option<u8>,
    /// The newest Win/Mac mode reported by the keyboard.
    pub system_mode: Option<Option<openhe_proto::SystemMode>>,
    /// Re-read the snapshot (debounced).
    pub reload: bool,
    /// The lighting changed on the keyboard.
    pub lighting: bool,
}

/// Maps keyboard events to worker effects. `Other` (e.g. a late RM6X21
/// header, 0x92) is noise and ignored.
pub fn event_effects(events: &[DeviceEvent]) -> EventEffects {
    let mut fx = EventEffects::default();
    for e in events {
        match *e {
            DeviceEvent::ProfileChanged(p) => {
                fx.profile = Some(p);
                fx.reload = true;
            }
            DeviceEvent::SystemModeChanged(m) => {
                fx.system_mode = Some(m);
                fx.reload = true;
            }
            DeviceEvent::KeyDataChanged => fx.reload = true,
            DeviceEvent::LightingChanged => fx.lighting = true,
            DeviceEvent::Other(_) => {}
        }
    }
    fx
}

/// Collapses a burst of events into one reload: every event pushes the
/// reload `window` further out, and it fires once the keyboard has been
/// quiet that long.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    window: Duration,
    due: Option<Instant>,
}

impl Debounce {
    pub fn new(window: Duration) -> Debounce {
        Debounce { window, due: None }
    }

    pub fn bump(&mut self, now: Instant) {
        self.due = Some(now + self.window);
    }

    pub fn is_pending(&self) -> bool {
        self.due.is_some()
    }

    /// True (once) when the quiet window has passed.
    pub fn take_due(&mut self, now: Instant) -> bool {
        match self.due {
            Some(t) if now >= t => {
                self.due = None;
                true
            }
            _ => false,
        }
    }

    pub fn cancel(&mut self) {
        self.due = None;
    }
}

/// Opens a keyboard. Called on the worker thread.
pub type Connector<T> = Box<dyn FnMut() -> Result<Keyboard<T>, Error> + Send>;

#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    /// Pause between connect attempts while disconnected; also the first
    /// backoff step after a failed background reload.
    pub retry_interval: Duration,
    /// How long the idle worker waits for a job before polling for events.
    pub idle_poll: Duration,
    /// How often the idle worker re-reads the active profile and the
    /// Win/Mac mode. The keyboard sends no event when Fn+A / Fn+S switches
    /// the OS mode (hardware finding 15), so this check is how such a
    /// change is noticed.
    pub state_poll: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        WorkerConfig {
            retry_interval: Duration::from_secs(1),
            idle_poll: Duration::from_millis(50),
            state_poll: STATE_POLL,
        }
    }
}

/// Default [`WorkerConfig::state_poll`].
pub const STATE_POLL: Duration = Duration::from_secs(2);

/// Longest wait between retries of a failed background reload.
const MAX_RELOAD_BACKOFF: Duration = Duration::from_secs(30);

/// Label shown while the snapshot loads.
pub const LOAD_LABEL: &str = "load-snapshot";

/// Prefix of the `Error::Io` text a panicking job returns.
pub const INTERNAL_ERROR: &str = "internal error";

/// What a finished job hands back to the worker loop.
struct JobDone {
    disconnected: bool,
    panicked: bool,
    deliver: Box<dyn FnOnce() + Send>,
}

type Job<T> = Box<dyn FnOnce(Option<&mut Keyboard<T>>) -> JobDone + Send>;

struct Msg<T: Transport> {
    label: &'static str,
    job: Job<T>,
    /// Counted in `outstanding` and shown as the current job (not for
    /// quiet polling jobs).
    counted: bool,
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    status: ConnectionStatus,
    current: Option<&'static str>,
    notices: Notices,
    repaint: Option<Repaint>,
}

/// State shared between the handle and the thread.
struct Control {
    shared: Mutex<Shared>,
    generation: AtomicU64,
    outstanding: AtomicUsize,
    reload: AtomicBool,
    stop: AtomicBool,
    /// Successful connects (snapshot loaded) so far.
    connects: AtomicU64,
    /// Bumped on every `LightingChanged` event.
    lighting: AtomicU64,
    /// Pause in ms before reconnecting, when a reconnect was requested
    /// (`NO_RECONNECT` otherwise).
    reconnect_ms: AtomicU64,
    /// Calibration may be running: stop it on (re)connect and on shutdown.
    calibrating: AtomicBool,
    /// A reconnect was requested (a write after which the keyboard
    /// re-enumerates); cleared by the next successful connect.
    restarting: AtomicBool,
}

const NO_RECONNECT: u64 = u64::MAX;

impl Control {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        // Every write to Shared is a single assignment, so a lock poisoned
        // by a panic elsewhere is still consistent.
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn repaint(&self) {
        let r = self.lock().repaint.clone();
        if let Some(r) = r {
            r();
        }
    }

    fn set_status(&self, s: ConnectionStatus) {
        {
            let mut g = self.lock();
            if g.status == s {
                return;
            }
            g.status = s;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.repaint();
    }

    /// A background reload read `s`: the generation always moves, also
    /// when `s` equals the shown snapshot. The snapshot does not hold
    /// function parameters (DKS, MT, TGL, MPT, END), so an import or write
    /// that changed only those reads an equal snapshot, and pages caching
    /// them must still see the reload.
    fn set_status_reloaded(&self, s: ConnectionStatus) {
        let before = self.generation.load(Ordering::SeqCst);
        self.set_status(s);
        if self.generation.load(Ordering::SeqCst) == before {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.repaint();
        }
    }

    fn set_current(&self, label: Option<&'static str>) {
        self.lock().current = label;
        self.repaint();
    }

    fn request_reconnect(&self, pause: Duration) {
        self.restarting.store(true, Ordering::SeqCst);
        let ms = u64::try_from(pause.as_millis()).unwrap_or(NO_RECONNECT - 1);
        self.reconnect_ms.store(ms, Ordering::SeqCst);
    }

    /// Edits the snapshot in place (no-op while disconnected).
    fn patch_snapshot(&self, f: impl FnOnce(&mut DeviceSnapshot)) {
        let patched = {
            let mut g = self.lock();
            match &mut g.status {
                ConnectionStatus::Connected(s) => {
                    f(s);
                    true
                }
                _ => false,
            }
        };
        if patched {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.repaint();
        }
    }

    /// Edits the notice slots; repaints when the shown notice changed.
    fn notices(&self, f: impl FnOnce(&mut Notices)) {
        let changed = {
            let mut g = self.lock();
            let before = g.notices.current();
            f(&mut g.notices);
            g.notices.current() != before
        };
        if changed {
            self.repaint();
        }
    }
}

/// Marks the worker as failed if its thread ends for any reason other than
/// the handle being dropped (including a panic outside the guarded calls).
struct ExitGuard(Arc<Control>);

impl Drop for ExitGuard {
    fn drop(&mut self) {
        let ctl = &self.0;
        if ctl.stop.load(Ordering::SeqCst) {
            return;
        }
        ctl.lock().current = None;
        ctl.outstanding.store(0, Ordering::SeqCst);
        ctl.set_status(ConnectionStatus::Error(
            i18n::device::WORKER_STOPPED.to_string(),
        ));
    }
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_string()
    }
}

/// Runs `f`, turning a panic into `Error::Io("internal error: ...")`.
fn guarded<R>(f: impl FnOnce() -> Result<R, Error>) -> (Result<R, Error>, bool) {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => (r, false),
        Err(p) => (
            Err(Error::Io(format!("{INTERNAL_ERROR}: {}", panic_text(&*p)))),
            true,
        ),
    }
}

/// How long dropping the handle waits for the thread while the calibration
/// guard is set: a worst-case job in flight (an RM6X21 poll is 2 calls of
/// up to 3 × 1 s) plus the shutdown stop.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(7);

/// Handle to the worker thread. Dropping it stops the thread after its
/// current step. Normally it is not joined, so closing the window never
/// waits on a slow keyboard; while the calibration guard is set it is
/// joined for up to [`SHUTDOWN_WAIT`], so the shutdown `stop_calibration`
/// goes out before the process ends.
pub struct Worker<T: Transport + 'static = HidTransport> {
    tx: Sender<Msg<T>>,
    ctl: Arc<Control>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The result of a submitted job, polled from the UI.
pub struct Pending<T> {
    rx: Receiver<Result<T, Error>>,
    label: &'static str,
    done: bool,
}

impl<T> Pending<T> {
    /// Non-blocking. Returns the result once; `None` before it is ready and
    /// after it was taken. A job the worker dropped unrun (the worker is
    /// gone) reads as `Err(Disconnected)`.
    pub fn poll(&mut self) -> Option<Result<T, Error>> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(r) => {
                self.done = true;
                Some(r)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Some(Err(Error::Disconnected))
            }
        }
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl Worker<HidTransport> {
    /// Starts the worker on the first supported HID keyboard.
    pub fn spawn() -> Worker<HidTransport> {
        Worker::spawn_with(
            Box::new(|| HidTransport::open_first().map(|(t, model)| Keyboard::new(t, model))),
            WorkerConfig::default(),
        )
    }
}

impl<T: Transport + 'static> Worker<T> {
    /// Starts the worker with any transport; `connect` is retried every
    /// `cfg.retry_interval` while disconnected.
    pub fn spawn_with(connect: Connector<T>, cfg: WorkerConfig) -> Worker<T> {
        let (tx, rx) = mpsc::channel();
        let ctl = Arc::new(Control {
            shared: Mutex::new(Shared {
                status: ConnectionStatus::Searching,
                current: None,
                notices: Notices::default(),
                repaint: None,
            }),
            generation: AtomicU64::new(0),
            outstanding: AtomicUsize::new(0),
            reload: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            connects: AtomicU64::new(0),
            lighting: AtomicU64::new(0),
            reconnect_ms: AtomicU64::new(NO_RECONNECT),
            calibrating: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
        });
        let thread_ctl = Arc::clone(&ctl);
        let thread = std::thread::Builder::new()
            .name("openhe-worker".into())
            .spawn(move || {
                let _guard = ExitGuard(Arc::clone(&thread_ctl));
                run(connect, rx, thread_ctl, cfg);
            })
            .expect("spawn the worker thread");
        Worker {
            tx,
            ctl,
            thread: Some(thread),
        }
    }

    /// Queues `job` to run on the worker thread with the open keyboard.
    /// Jobs run one at a time, in order. While no keyboard is open the job is
    /// not run and its result is `Err(Disconnected)`. A job that panics
    /// returns `Err(Io("internal error: ..."))` and the keyboard is reopened.
    pub fn submit<R: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
    ) -> Pending<R> {
        self.enqueue(label, true, job)
    }

    /// Like [`Worker::submit`], but the job does not make the app busy
    /// ([`Worker::is_busy`], [`Worker::current_job`]): for frequent read-only
    /// polling (live depth, calibration progress), which must not disable
    /// write controls. Keep at most one quiet job in flight per page.
    pub fn submit_quiet<R: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
    ) -> Pending<R> {
        self.enqueue(label, false, job)
    }

    fn enqueue<R: Send + 'static>(
        &self,
        label: &'static str,
        counted: bool,
        job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
    ) -> Pending<R> {
        enqueue(&self.tx, &self.ctl, label, counted, job)
    }

    /// A cheap, cloneable handle for other threads (the per-app profile
    /// switcher): it submits jobs and reads the state like the worker
    /// itself, but does not keep the thread alive or stop it.
    pub fn handle(&self) -> WorkerHandle<T> {
        WorkerHandle {
            tx: self.tx.clone(),
            ctl: Arc::clone(&self.ctl),
        }
    }

    pub fn status(&self) -> ConnectionStatus {
        self.ctl.lock().status.clone()
    }

    pub fn snapshot(&self) -> Option<DeviceSnapshot> {
        match &self.ctl.lock().status {
            ConnectionStatus::Connected(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.ctl.lock().status, ConnectionStatus::Connected(_))
    }

    /// Asks the worker to re-read the whole snapshot when it is next idle
    /// (also cancels any reload backoff).
    pub fn request_reload(&self) {
        self.ctl.reload.store(true, Ordering::SeqCst);
    }

    /// Edits the current snapshot in place (no-op while disconnected), e.g.
    /// to store key settings a job just read back.
    pub fn patch_snapshot(&self, f: impl FnOnce(&mut DeviceSnapshot)) {
        self.ctl.patch_snapshot(f);
    }

    /// Bumped whenever the status or the snapshot changes, and after every
    /// background reload (requested, after keyboard events, or a retry),
    /// even when it read an equal snapshot: function parameters are not
    /// part of the snapshot, so a page caching them re-reads on any move.
    pub fn generation(&self) -> u64 {
        self.ctl.generation.load(Ordering::SeqCst)
    }

    /// True while a job is queued or running, or the snapshot is loading.
    pub fn is_busy(&self) -> bool {
        self.ctl.outstanding.load(Ordering::SeqCst) > 0 || self.ctl.lock().current.is_some()
    }

    /// Label of the job running right now ([`LOAD_LABEL`] while loading).
    pub fn current_job(&self) -> Option<&'static str> {
        self.ctl.lock().current
    }

    /// What the shell should warn about (stale or incomplete values), if
    /// anything. Cleared by the next successful reload.
    pub fn notice(&self) -> Option<WorkerNotice> {
        self.ctl.lock().notices.current()
    }

    /// Hides a [`WorkerNotice::KeysSkipped`] notice until the set of
    /// skipped keys changes. Other notices clear themselves.
    pub fn dismiss_notice(&self) {
        self.ctl.notices(Notices::dismiss);
    }

    /// Bumped whenever the keyboard reports a lighting change of its own
    /// (no reload is done for it); pages showing lighting re-read when it
    /// moves.
    pub fn lighting_generation(&self) -> u64 {
        self.ctl.lighting.load(Ordering::SeqCst)
    }

    /// Number of successful connects so far (each one loaded a fresh
    /// snapshot). A page that expects the keyboard to re-enumerate waits
    /// for this to move.
    pub fn connects(&self) -> u64 {
        self.ctl.connects.load(Ordering::SeqCst)
    }

    /// Drops the keyboard when the worker is next between jobs, reports
    /// `Searching`, and connects again after `pause` (jobs submitted
    /// meanwhile fail with `Disconnected`). For writes after which the
    /// keyboard re-enumerates on USB (polling rate, factory reset).
    pub fn reconnect(&self, pause: Duration) {
        self.ctl.request_reconnect(pause);
    }

    /// While `on`, the worker sends `stop_calibration` to any keyboard it
    /// (re)connects to, right after the handshake, and to the open keyboard when the
    /// worker shuts down. The calibration page sets it before starting and
    /// clears it once a stop succeeded. Cleared by the worker after it sent
    /// the stop on a reconnect.
    /// True from [`Worker::reconnect`] until the next successful connect:
    /// the keyboard is re-enumerating after a write (polling rate, factory
    /// reset, profile switch). Writes and automatic profile switches stay
    /// locked meanwhile.
    pub fn is_restarting(&self) -> bool {
        self.ctl.restarting.load(Ordering::SeqCst)
    }

    pub fn set_calibration_guard(&self, on: bool) {
        self.ctl.calibrating.store(on, Ordering::SeqCst);
    }

    pub fn calibration_guard(&self) -> bool {
        self.ctl.calibrating.load(Ordering::SeqCst)
    }

    /// Called from the worker thread whenever something changed, so the UI
    /// can wake up (the app passes `egui::Context::request_repaint`).
    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        self.ctl.lock().repaint = Some(Arc::new(f));
    }
}

/// A cloneable handle to the worker for other threads (see
/// [`Worker::handle`]). Jobs submitted after the worker is gone fail with
/// `Err(Disconnected)`.
pub struct WorkerHandle<T: Transport + 'static = HidTransport> {
    tx: Sender<Msg<T>>,
    ctl: Arc<Control>,
}

impl<T: Transport + 'static> Clone for WorkerHandle<T> {
    fn clone(&self) -> Self {
        WorkerHandle {
            tx: self.tx.clone(),
            ctl: Arc::clone(&self.ctl),
        }
    }
}

impl<T: Transport + 'static> WorkerHandle<T> {
    /// Like [`Worker::submit`] (counted: the app is busy meanwhile).
    pub fn submit<R: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
    ) -> Pending<R> {
        enqueue(&self.tx, &self.ctl, label, true, job)
    }

    /// The active profile while connected (no snapshot copy).
    pub fn profile(&self) -> Option<u8> {
        match &self.ctl.lock().status {
            ConnectionStatus::Connected(s) => Some(s.profile),
            _ => None,
        }
    }

    /// See [`Worker::is_busy`].
    pub fn is_busy(&self) -> bool {
        self.ctl.outstanding.load(Ordering::SeqCst) > 0 || self.ctl.lock().current.is_some()
    }

    /// See [`Worker::calibration_guard`].
    pub fn calibration_guard(&self) -> bool {
        self.ctl.calibrating.load(Ordering::SeqCst)
    }

    /// See [`Worker::is_restarting`].
    pub fn is_restarting(&self) -> bool {
        self.ctl.restarting.load(Ordering::SeqCst)
    }

    /// See [`Worker::connects`].
    pub fn connects(&self) -> u64 {
        self.ctl.connects.load(Ordering::SeqCst)
    }

    /// See [`Worker::reconnect`].
    pub fn reconnect(&self, pause: Duration) {
        self.ctl.request_reconnect(pause);
    }

    /// See [`Worker::snapshot`].
    pub fn snapshot(&self) -> Option<DeviceSnapshot> {
        match &self.ctl.lock().status {
            ConnectionStatus::Connected(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// See [`Worker::request_reload`].
    pub fn request_reload(&self) {
        self.ctl.reload.store(true, Ordering::SeqCst);
    }

    /// See [`Worker::patch_snapshot`].
    pub fn patch_snapshot(&self, f: impl FnOnce(&mut DeviceSnapshot)) {
        self.ctl.patch_snapshot(f);
    }
}

/// A [`Pending`] whose result the test sends itself (for fakes of the
/// worker).
#[cfg(test)]
pub(crate) fn pending_channel<R>(label: &'static str) -> (Sender<Result<R, Error>>, Pending<R>) {
    let (tx, rx) = mpsc::channel();
    (
        tx,
        Pending {
            rx,
            label,
            done: false,
        },
    )
}

/// Queues a job on the worker thread (shared by [`Worker`] and
/// [`WorkerHandle`]).
fn enqueue<T: Transport + 'static, R: Send + 'static>(
    tx: &Sender<Msg<T>>,
    ctl: &Control,
    label: &'static str,
    counted: bool,
    job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
) -> Pending<R> {
    let (rtx, rrx) = mpsc::channel();
    let wrapped: Job<T> = Box::new(move |kb| {
        let ran = kb.is_some();
        let (result, panicked) = match kb {
            Some(k) => guarded(|| job(k)),
            None => (Err(Error::Disconnected), false),
        };
        JobDone {
            disconnected: ran && matches!(result, Err(Error::Disconnected)),
            panicked,
            deliver: Box::new(move || {
                let _ = rtx.send(result);
            }),
        }
    });
    if counted {
        ctl.outstanding.fetch_add(1, Ordering::SeqCst);
    }
    if tx
        .send(Msg {
            label,
            job: wrapped,
            counted,
        })
        .is_err()
        && counted
    {
        // The thread is gone; dropping the job drops its sender.
        ctl.outstanding.fetch_sub(1, Ordering::SeqCst);
    }
    Pending {
        rx: rrx,
        label,
        done: false,
    }
}

#[cfg(test)]
impl<T: Transport + 'static> Worker<T> {
    /// Test hook for UI tests: counts a successful connect.
    pub(crate) fn inject_connect(&self) {
        self.ctl.connects.fetch_add(1, Ordering::SeqCst);
    }

    /// Test hook for UI tests: overrides the shown status.
    pub(crate) fn inject_status(&self, s: ConnectionStatus) {
        self.ctl.set_status(s);
    }

    /// Test hook for UI tests: overrides the notice.
    pub(crate) fn inject_notice(&self, n: Option<WorkerNotice>) {
        self.ctl.notices(|slots| {
            *slots = Notices::default();
            match n {
                Some(WorkerNotice::ReloadFailed(e)) => slots.reload_failed = Some(e),
                Some(WorkerNotice::EventsFailed(e)) => slots.events_failed = Some(e),
                Some(WorkerNotice::KeysSkipped(k)) => slots.skipped = k,
                None => {}
            }
        });
    }
}

impl<T: Transport + 'static> Drop for Worker<T> {
    fn drop(&mut self) {
        self.ctl.stop.store(true, Ordering::SeqCst);
        // Only an open keyboard can be calibrating.
        if !self.ctl.calibrating.load(Ordering::SeqCst) || !self.is_connected() {
            return;
        }
        // The keyboard may be calibrating (it types nothing then): give the
        // thread time to finish its job and send the stop.
        if let Some(h) = self.thread.take() {
            let deadline = Instant::now() + SHUTDOWN_WAIT;
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// Reads everything the UI shows. Starts with the handshake (SYNC), which
/// refuses a keyboard in bootloader mode (`Unsupported("bootloader")`)
/// before anything else is sent.
///
/// A key whose settings cannot be parsed (`Io` or `Frame` errors, e.g. an
/// unknown Mode value) is left out and listed in the second value, so one
/// odd key never blocks connecting. Timeouts and disconnects still fail the
/// whole load.
pub fn load_snapshot<T: Transport>(
    kb: &mut Keyboard<T>,
) -> Result<(DeviceSnapshot, Vec<(u8, Error)>), Error> {
    load_snapshot_then(kb, |_| {})
}

/// [`load_snapshot`], running `after_handshake` once the handshake has
/// accepted the keyboard (so never on a keyboard in bootloader mode) and
/// before anything else is read.
pub fn load_snapshot_then<T: Transport>(
    kb: &mut Keyboard<T>,
    after_handshake: impl FnOnce(&mut Keyboard<T>),
) -> Result<(DeviceSnapshot, Vec<(u8, Error)>), Error> {
    let info = kb.handshake()?;
    after_handshake(kb);
    let system_mode = kb.system_mode()?;
    let profile = kb.profile()?;
    let polling = kb.polling_rate()?;
    let matrix = kb.matrix()?;
    let mut keys = BTreeMap::new();
    let mut skipped = Vec::new();
    let codes: Vec<u8> = matrix.keys().map(|(_, _, c)| c).collect();
    for code in codes {
        match kb.key_settings(code) {
            Ok(s) => {
                keys.insert(code, s);
            }
            Err(e @ (Error::Io(_) | Error::Frame(_))) => skipped.push((code, e)),
            Err(e) => return Err(e),
        }
    }
    let snap = DeviceSnapshot {
        model: kb.model,
        info,
        system_mode,
        profile,
        polling,
        matrix,
        keys,
    };
    Ok((snap, skipped))
}

/// Event adapter: the keyboard's own reports, drained without blocking
/// (`Keyboard::poll_events`: events queued by earlier calls, then whatever
/// input is waiting).
pub fn poll_events<T: Transport>(kb: &mut Keyboard<T>) -> Result<Vec<DeviceEvent>, Error> {
    kb.poll_events()
}

fn connect_error_status(e: &Error) -> ConnectionStatus {
    match e {
        Error::Unsupported(_) | Error::Disconnected => ConnectionStatus::Searching,
        other => ConnectionStatus::Error(i18n::device_error(other)),
    }
}

fn load_error_status(e: &Error) -> ConnectionStatus {
    match e {
        Error::Disconnected => ConnectionStatus::Searching,
        other => ConnectionStatus::Error(i18n::device_error(other)),
    }
}

/// Backoff state for failed background reloads.
struct Retry {
    base: Duration,
    next_delay: Duration,
    due: Option<Instant>,
}

impl Retry {
    fn new(base: Duration) -> Retry {
        Retry {
            base,
            next_delay: base,
            due: None,
        }
    }

    fn reset(&mut self) {
        self.next_delay = self.base;
        self.due = None;
    }

    fn schedule(&mut self) {
        self.due = Some(Instant::now() + self.next_delay);
        self.next_delay = (self.next_delay * 2).min(MAX_RELOAD_BACKOFF);
    }

    fn is_due(&self) -> bool {
        self.due.is_some_and(|t| Instant::now() >= t)
    }

    /// True (once) when due; the next `schedule` keeps the longer delay.
    fn take_due(&mut self) -> bool {
        if self.is_due() {
            self.due = None;
            true
        } else {
            false
        }
    }
}

/// How long the keyboard must stay quiet before an event-driven reload.
fn debounce_window(cfg: &WorkerConfig) -> Duration {
    cfg.idle_poll * 4
}

/// The handle is gone: leave the keyboard usable (stop a calibration that
/// may still be running) before the thread ends.
fn shutdown<T: Transport>(ctl: &Control, kb: Option<&mut Keyboard<T>>) {
    if let Some(k) = kb {
        if ctl.calibrating.load(Ordering::SeqCst) {
            let _ = guarded(|| k.stop_calibration());
        }
    }
}

fn run<T: Transport>(
    mut connect: Connector<T>,
    rx: Receiver<Msg<T>>,
    ctl: Arc<Control>,
    cfg: WorkerConfig,
) {
    let mut kb: Option<Keyboard<T>> = None;
    let mut retry = Retry::new(cfg.retry_interval);
    // Reloads after failed event polls back off on their own: a reload
    // that works does not reset it, only a poll that works does.
    let mut event_retry = Retry::new(cfg.retry_interval);
    let mut debounce = Debounce::new(debounce_window(&cfg));
    let mut last_idle = Instant::now();
    // Last time profile and OS mode were read (by a load or the state poll).
    let mut last_state = Instant::now();
    // When to try to connect next (a requested reconnect pauses first).
    let mut connect_at = Instant::now();
    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            shutdown(&ctl, kb.as_mut());
            return;
        }
        let pause = ctl.reconnect_ms.swap(NO_RECONNECT, Ordering::SeqCst);
        if pause != NO_RECONNECT {
            kb = None;
            ctl.set_status(ConnectionStatus::Searching);
            connect_at = Instant::now() + Duration::from_millis(pause);
        }
        let Some(k) = kb.as_mut() else {
            if Instant::now() >= connect_at {
                match guarded(&mut connect).0 {
                    Ok(mut k) => {
                        ctl.set_current(Some(LOAD_LABEL));
                        // The guard's stop goes out right after the
                        // handshake, so a keyboard in bootloader mode is
                        // never written to. A stop that failed is tried
                        // again next connect.
                        let stop_owed = |k: &mut Keyboard<T>| {
                            if ctl.calibrating.load(Ordering::SeqCst)
                                && guarded(|| k.stop_calibration()).0.is_ok()
                            {
                                ctl.calibrating.store(false, Ordering::SeqCst);
                            }
                        };
                        let (loaded, _) = guarded(|| load_snapshot_then(&mut k, stop_owed));
                        ctl.set_current(None);
                        match loaded {
                            Ok((snap, skipped)) => {
                                ctl.reload.store(false, Ordering::SeqCst);
                                retry.reset();
                                debounce.cancel();
                                ctl.notices(|n| {
                                    n.events_failed = None;
                                    n.loaded(skipped);
                                });
                                ctl.connects.fetch_add(1, Ordering::SeqCst);
                                ctl.restarting.store(false, Ordering::SeqCst);
                                ctl.set_status(ConnectionStatus::Connected(snap));
                                kb = Some(k);
                                last_idle = Instant::now();
                                last_state = Instant::now();
                                continue;
                            }
                            Err(e) => ctl.set_status(load_error_status(&e)),
                        }
                    }
                    Err(e) => ctl.set_status(connect_error_status(&e)),
                }
                connect_at = Instant::now() + cfg.retry_interval;
            }
            // Wait for the next attempt; jobs arriving meanwhile fail at once.
            loop {
                let left = connect_at.saturating_duration_since(Instant::now());
                if left.is_zero()
                    || ctl.stop.load(Ordering::SeqCst)
                    || ctl.reconnect_ms.load(Ordering::SeqCst) != NO_RECONNECT
                {
                    break;
                }
                // Short slices, so a stop or reconnect request is seen soon.
                match rx.recv_timeout(left.min(Duration::from_millis(100))) {
                    Ok(msg) => finish(&ctl, msg.counted, (msg.job)(None)),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            continue;
        };

        match rx.recv_timeout(cfg.idle_poll) {
            Ok(msg) => {
                if msg.counted {
                    ctl.set_current(Some(msg.label));
                }
                let done = (msg.job)(Some(k));
                if msg.counted {
                    ctl.set_current(None);
                }
                let lost = done.disconnected || done.panicked;
                if lost {
                    // After a panic the keyboard state is unknown: reopen it.
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                }
                finish(&ctl, msg.counted, done);
                // Busy with jobs (e.g. live-depth polling): still look for
                // events at least every `idle_poll`.
                if lost || last_idle.elapsed() < cfg.idle_poll {
                    continue;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                shutdown(&ctl, Some(k));
                return;
            }
        }
        let Some(k) = kb.as_mut() else {
            continue;
        };
        if ctl.reconnect_ms.load(Ordering::SeqCst) != NO_RECONNECT {
            continue;
        }

        // Idle: look for events from the keyboard, then reload if asked, if
        // events settled, or if a failed reload is due for another try.
        last_idle = Instant::now();
        let (polled, panicked) = guarded(|| poll_events(k));
        match polled {
            Ok(events) => {
                event_retry.reset();
                ctl.notices(|n| n.events_failed = None);
                let fx = event_effects(&events);
                if fx.profile.is_some() || fx.system_mode.is_some() {
                    // Show the keyboard's new profile / mode at once; the
                    // debounced reload brings the rest.
                    ctl.patch_snapshot(|s| {
                        if let Some(p) = fx.profile {
                            s.profile = p;
                        }
                        if let Some(m) = fx.system_mode {
                            s.system_mode = m;
                        }
                    });
                }
                if fx.lighting {
                    ctl.lighting.fetch_add(1, Ordering::SeqCst);
                    ctl.repaint();
                }
                if fx.reload {
                    debounce.bump(Instant::now());
                }
            }
            // A panic while reading events leaves the transport in an
            // unknown state: treat it like a disconnect and reopen.
            Err(_) if panicked => {
                kb = None;
                ctl.set_status(ConnectionStatus::Searching);
                continue;
            }
            Err(Error::Disconnected) => {
                kb = None;
                ctl.set_status(ConnectionStatus::Searching);
                continue;
            }
            Err(e) => {
                ctl.notices(|n| n.events_failed = Some(e));
                if event_retry.due.is_none() {
                    event_retry.schedule();
                }
            }
        }
        // While calibration may be running, every reload (asked for, after
        // events, or a retry) waits until it stopped: the flags and timers
        // stay pending meanwhile.
        if ctl.calibrating.load(Ordering::SeqCst) {
            continue;
        }
        // The keyboard reports no event for a Win/Mac switch (Fn+A / Fn+S),
        // so re-read profile and OS mode now and then. Only when nothing
        // else is going on: no job queued, no reload asked for or pending.
        if last_state.elapsed() >= cfg.state_poll
            && ctl.outstanding.load(Ordering::SeqCst) == 0
            && !ctl.reload.load(Ordering::SeqCst)
            && !debounce.is_pending()
            && !retry.is_due()
        {
            last_state = Instant::now();
            let (state, panicked) = guarded(|| Ok((k.profile()?, k.system_mode()?)));
            match state {
                Ok((profile, mode)) => {
                    let shown = match &ctl.lock().status {
                        ConnectionStatus::Connected(s) => Some((s.profile, s.system_mode)),
                        _ => None,
                    };
                    if shown.is_some_and(|shown| shown != (profile, mode)) {
                        // Same path as a keyboard event: show it at once,
                        // then a debounced full reload.
                        ctl.patch_snapshot(|s| {
                            s.profile = profile;
                            s.system_mode = mode;
                        });
                        debounce.bump(Instant::now());
                    }
                }
                Err(_) if panicked => {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                    continue;
                }
                Err(Error::Disconnected) => {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                    continue;
                }
                // A missed check is tried again after `state_poll`.
                Err(_) => {}
            }
        }
        let asked = ctl.reload.swap(false, Ordering::SeqCst);
        let settled = debounce.take_due(Instant::now());
        let events_retry = event_retry.take_due();
        if asked || settled || events_retry || retry.is_due() {
            debounce.cancel();
            ctl.set_current(Some(LOAD_LABEL));
            let (loaded, panicked) = guarded(|| load_snapshot(k));
            ctl.set_current(None);
            match loaded {
                Ok((snap, skipped)) => {
                    retry.reset();
                    last_state = Instant::now();
                    ctl.notices(|n| n.loaded(skipped));
                    ctl.set_status_reloaded(ConnectionStatus::Connected(snap));
                }
                Err(_) if panicked => {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                }
                Err(Error::Disconnected) => {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                }
                // Keep the old snapshot and the connection, warn, retry later.
                Err(e) => {
                    ctl.notices(|n| n.reload_failed = Some(e));
                    retry.schedule();
                }
            }
        }
    }
}

fn finish(ctl: &Control, counted: bool, done: JobDone) {
    (done.deliver)();
    if counted {
        ctl.outstanding.fetch_sub(1, Ordering::SeqCst);
    }
    ctl.repaint();
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use openhe_device::MockTransport;

    fn reply(cmd: u8, body: &[(usize, &[u8])]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        for (at, bytes) in body {
            raw[4 + at..4 + at + bytes.len()].copy_from_slice(bytes);
        }
        raw
    }

    pub(crate) fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        reply(0x00, &[(1, &[ty, v, 0xFF])])
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let [lo, hi] = v.to_le_bytes();
        reply(0x23, &[(1, &[key, layout, lo, hi])])
    }

    /// Two keys, W (0x1A) and A (0x04), in matrix row 1.
    pub(crate) const KEYS: [u8; 2] = [0x1A, 0x04];

    /// Every reply `load_snapshot` needs, in order, with `profile`.
    pub(crate) fn push_snapshot(m: &mut MockTransport, profile: u8) {
        push_handshake(m);
        push_snapshot_rest(m, profile);
    }

    /// The handshake (SYNC) reply that starts every load.
    pub(crate) fn push_handshake(m: &mut MockTransport) {
        m.push_reply(reply(0x01, &[(25, &[3]), (26, b"App")]));
    }

    /// The replies of a load after its handshake.
    pub(crate) fn push_snapshot_rest(m: &mut MockTransport, profile: u8) {
        push_snapshot_rest_with_mode(m, profile, 0);
    }

    /// Every reply of a load, with `mode` as the Mode of both keys (e.g.
    /// `0x0001`: a DKS key).
    pub(crate) fn push_snapshot_with_mode(m: &mut MockTransport, profile: u8, mode: u16) {
        push_handshake(m);
        push_snapshot_rest_with_mode(m, profile, mode);
    }

    fn push_snapshot_rest_with_mode(m: &mut MockTransport, profile: u8, mode: u16) {
        push_snapshot_rest_full(m, profile, mode, 1);
    }

    /// Every reply of a load with `os` as the QueryWinMode value (1
    /// Windows, 0 Mac).
    pub(crate) fn push_snapshot_os(m: &mut MockTransport, profile: u8, os: u8) {
        push_handshake(m);
        push_snapshot_rest_full(m, profile, 0, os);
    }

    fn push_snapshot_rest_full(m: &mut MockTransport, profile: u8, mode: u16, os: u8) {
        m.push_reply(order_reply(0x21, os));
        m.push_reply(order_reply(0x70, profile));
        m.push_reply(order_reply(0x50, 0));
        m.push_reply(reply(0x2B, &[(1, &[0]), (23, &[1]), (24, &KEYS)]));
        m.push_reply(reply(0x2B, &[(1, &[2]), (23, &[3])]));
        m.push_reply(reply(0x2B, &[(1, &[4]), (23, &[5])]));
        for key in KEYS {
            for (layout, v) in [
                (8, mode),
                (4, 2000),
                (20, 20),
                (21, 20),
                (22, 200),
                (23, 200),
                (25, 0),
            ] {
                m.push_reply(key_reply(key, layout, v));
            }
        }
    }

    fn fast() -> WorkerConfig {
        WorkerConfig {
            retry_interval: Duration::from_millis(5),
            idle_poll: Duration::from_millis(2),
            state_poll: Duration::from_secs(3600),
        }
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_result<R>(p: &mut Pending<R>) -> Result<R, Error> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(r) = p.poll() {
                return r;
            }
            assert!(
                Instant::now() < deadline,
                "job {} never finished",
                p.label()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A connector that opens a fresh mock (with one snapshot queued) while
    /// `plugged` is true, and reports "not found" otherwise.
    fn pluggable(plugged: Arc<AtomicBool>, opened: Arc<AtomicUsize>) -> Connector<MockTransport> {
        Box::new(move || {
            if !plugged.load(Ordering::SeqCst) {
                return Err(Error::Unsupported("no supported keyboard was found".into()));
            }
            opened.fetch_add(1, Ordering::SeqCst);
            let mut m = MockTransport::new();
            push_snapshot(&mut m, 1);
            Ok(Keyboard::new(m, Model::Win68HeMax))
        })
    }

    #[test]
    fn connects_and_loads_the_snapshot() {
        let w = Worker::spawn_with(
            pluggable(Arc::new(AtomicBool::new(true)), Arc::default()),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let s = w.snapshot().unwrap();
        assert_eq!(s.model, Model::Win68HeMax);
        assert_eq!(s.profile, 1);
        assert_eq!(s.polling, PollingRate::Hz8000);
        assert_eq!(s.system_mode, Some(SystemMode::Windows));
        assert_eq!(s.matrix.len(), 2);
        assert_eq!(s.keys.keys().copied().collect::<Vec<_>>(), vec![0x04, 0x1A]);
        assert_eq!(s.keys[&0x1A].actuation.0, 2000);
        assert!(w.generation() > 0);
    }

    #[test]
    fn job_round_trip() {
        let w = Worker::spawn_with(
            pluggable(Arc::new(AtomicBool::new(true)), Arc::default()),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let mut p = w.submit("read-profile", |kb| {
            kb.session_mut()
                .transport_mut()
                .push_reply(order_reply(0x70, 3));
            kb.profile()
        });
        assert_eq!(p.label(), "read-profile");
        assert_eq!(wait_result(&mut p), Ok(3));
        assert_eq!(p.poll(), None, "a result is delivered once");
        wait_for("idle", || !w.is_busy());
    }

    #[test]
    fn job_errors_are_delivered_without_dropping_the_connection() {
        let opened = Arc::new(AtomicUsize::new(0));
        let w = Worker::spawn_with(
            pluggable(Arc::new(AtomicBool::new(true)), opened.clone()),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        // No reply queued: the call times out (instantly with the mock).
        let mut p = w.submit("timeout", |kb| kb.profile());
        assert_eq!(wait_result(&mut p), Err(Error::Timeout));
        assert!(w.is_connected());
        assert_eq!(opened.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unplug_mid_job_then_reconnect() {
        // Review Focus 2.
        let plugged = Arc::new(AtomicBool::new(true));
        let opened = Arc::new(AtomicUsize::new(0));
        let w = Worker::spawn_with(pluggable(plugged.clone(), opened.clone()), fast());
        wait_for("connect", || w.is_connected());

        let unplug = plugged.clone();
        let mut p = w.submit("write", move |kb| {
            unplug.store(false, Ordering::SeqCst);
            kb.session_mut()
                .transport_mut()
                .fail_with(Error::Disconnected);
            kb.profile()
        });
        assert_eq!(wait_result(&mut p), Err(Error::Disconnected));
        // The status flips before the result is delivered.
        assert_eq!(w.status(), ConnectionStatus::Searching);
        assert!(w.snapshot().is_none());

        // While unplugged, jobs fail fast instead of queueing.
        let mut q = w.submit("while-unplugged", |kb| kb.profile());
        assert_eq!(wait_result(&mut q), Err(Error::Disconnected));

        plugged.store(true, Ordering::SeqCst);
        wait_for("reconnect", || w.is_connected());
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        let mut r = w.submit("after", |kb| Ok(kb.model));
        assert_eq!(wait_result(&mut r), Ok(Model::Win68HeMax));
    }

    #[test]
    fn idle_disconnect_is_noticed() {
        let plugged = Arc::new(AtomicBool::new(true));
        let w = Worker::spawn_with(pluggable(plugged.clone(), Arc::default()), fast());
        wait_for("connect", || w.is_connected());
        // A job that breaks the transport but itself succeeds: the idle
        // event poll is what notices.
        plugged.store(false, Ordering::SeqCst);
        let mut p = w.submit("break", |kb| {
            kb.session_mut()
                .transport_mut()
                .fail_with(Error::Disconnected);
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("searching", || w.status() == ConnectionStatus::Searching);
    }

    #[test]
    fn permission_errors_are_reported_and_retried() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let w: Worker<MockTransport> = Worker::spawn_with(
            Box::new(move || {
                t.fetch_add(1, Ordering::SeqCst);
                Err(Error::Permission("denied".into()))
            }),
            fast(),
        );
        wait_for("error", || {
            w.status()
                == ConnectionStatus::Error(i18n::device_error(&Error::Permission("x".into())))
        });
        wait_for("retries", || tries.load(Ordering::SeqCst) >= 3);
    }

    /// From a requested reconnect (a write after which the keyboard
    /// re-enumerates) until the next successful connect, the worker reports
    /// restarting, so writes and automatic switches stay locked.
    #[test]
    fn a_requested_reconnect_reports_restarting_until_the_keyboard_is_back() {
        let n = Arc::new(AtomicUsize::new(0));
        let c = n.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        assert!(!w.is_restarting());
        let before = w.connects();
        w.reconnect(Duration::from_millis(30));
        assert!(w.is_restarting(), "at once, before the worker drops it");
        assert!(w.handle().is_restarting());
        wait_for("back", || w.connects() > before);
        assert!(!w.is_restarting());
    }

    #[test]
    fn a_busy_keyboard_is_reported_and_retried() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let w: Worker<MockTransport> = Worker::spawn_with(
            Box::new(move || {
                t.fetch_add(1, Ordering::SeqCst);
                Err(Error::Busy(
                    "exclusive access and device already open".into(),
                ))
            }),
            fast(),
        );
        wait_for("busy status", || {
            w.status()
                == ConnectionStatus::Error(
                    "Keyboard sedang dipakai program lain (tutup OpenHE/AULA HUB lain)".into(),
                )
        });
        wait_for("retries", || tries.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn bootloader_is_refused() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                m.push_reply(reply(0x01, &[(7, &[1])]));
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("error", || {
            w.status() == ConnectionStatus::Error(i18n::error::BOOTLOADER.into())
        });
    }

    #[test]
    fn reloads_wait_while_calibration_may_be_running() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                push_snapshot(&mut m, 2);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        w.set_calibration_guard(true);
        w.request_reload();
        // A profile event still shows at once, but nothing is re-read.
        let mut p = w.submit("event", |kb| {
            kb.session_mut()
                .transport_mut()
                .push_pending(order_reply(0x70, 3));
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("patched", || w.snapshot().is_some_and(|s| s.profile == 3));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(w.snapshot().unwrap().profile, 3, "no reload yet");
        // Calibration stopped: the deferred reload runs (snapshot: 2).
        w.set_calibration_guard(false);
        wait_for("reload", || w.snapshot().is_some_and(|s| s.profile == 2));
        assert_eq!(w.notice(), None);
    }

    #[test]
    fn event_poll_retries_keep_backing_off() {
        // The retry after a failed event poll is not reset by the reload it
        // triggers, so repeated poll errors wait 1, 2, 4, ... intervals.
        let base = Duration::from_millis(40);
        let mut r = Retry::new(base);
        let mut delays = Vec::new();
        for _ in 0..4 {
            let t0 = Instant::now();
            r.schedule();
            delays.push(r.due.unwrap() - t0);
            assert!(!r.take_due(), "not due at once");
            r.due = Some(Instant::now()); // time passes
            assert!(r.take_due());
            assert!(!r.take_due(), "taken once");
        }
        for (i, d) in delays.iter().enumerate() {
            let want = base * 2u32.pow(i as u32);
            assert!(
                *d >= want && *d < want + Duration::from_millis(20),
                "{i}: {d:?}"
            );
        }
        r.reset();
        r.schedule();
        assert!(
            r.due.unwrap() - Instant::now() <= base,
            "a good poll resets it"
        );
    }

    #[test]
    fn reload_on_request_and_on_profile_event() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                push_snapshot(&mut m, 2);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        assert_eq!(w.snapshot().unwrap().profile, 0);
        w.request_reload();
        wait_for("reload", || w.snapshot().is_some_and(|s| s.profile == 2));

        // Fn+key on the keyboard: an unsolicited profile report.
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                push_snapshot(&mut m, 3);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let mut p = w.submit("event", |kb| {
            kb.session_mut()
                .transport_mut()
                .push_pending(order_reply(0x70, 3));
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("event reload", || {
            w.snapshot().is_some_and(|s| s.profile == 3)
        });
    }

    #[test]
    fn a_requested_reload_moves_the_generation_even_when_nothing_changed() {
        // Function parameters (DKS, MT, ...) are not in the snapshot, so a
        // reload after an import that changed only them reads an equal
        // snapshot. Pages caching such parameters still have to see it.
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let g = w.generation();
        w.request_reload();
        wait_for("generation moved", || w.generation() != g);
        assert_eq!(w.notice(), None, "the reload itself worked");
    }

    #[test]
    fn patch_snapshot_edits_in_place() {
        let w = Worker::spawn_with(
            pluggable(Arc::new(AtomicBool::new(true)), Arc::default()),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let g = w.generation();
        w.patch_snapshot(|s| s.profile = 3);
        assert_eq!(w.snapshot().unwrap().profile, 3);
        assert!(w.generation() > g);
    }

    #[test]
    fn a_panicking_job_is_contained_and_the_worker_recovers() {
        let plugged = Arc::new(AtomicBool::new(true));
        let opened = Arc::new(AtomicUsize::new(0));
        let w = Worker::spawn_with(pluggable(plugged, opened.clone()), fast());
        wait_for("connect", || w.is_connected());
        let mut p = w.submit("boom", |_kb| -> Result<(), Error> { panic!("boom") });
        match wait_result(&mut p) {
            Err(Error::Io(m)) => assert_eq!(m, format!("{INTERNAL_ERROR}: boom")),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            i18n::device_error(&Error::Io(format!("{INTERNAL_ERROR}: boom"))),
            i18n::error::INTERNAL
        );
        wait_for("idle", || !w.is_busy());
        wait_for("reconnect", || {
            w.is_connected() && opened.load(Ordering::SeqCst) == 2
        });
        let mut q = w.submit("after", |kb| Ok(kb.model));
        assert_eq!(wait_result(&mut q), Ok(Model::Win68HeMax));
    }

    #[test]
    fn a_panicking_connector_reports_an_error_and_keeps_trying() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let w: Worker<MockTransport> = Worker::spawn_with(
            Box::new(move || {
                t.fetch_add(1, Ordering::SeqCst);
                panic!("connector bug")
            }),
            fast(),
        );
        wait_for("error", || {
            w.status() == ConnectionStatus::Error(i18n::error::INTERNAL.into())
        });
        wait_for("retries", || tries.load(Ordering::SeqCst) >= 3);
        assert!(!w.is_busy());
    }

    #[test]
    fn a_failed_reload_warns_keeps_the_snapshot_and_retries() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        // No replies queued: the reload times out.
        w.request_reload();
        wait_for("notice", || {
            w.notice() == Some(WorkerNotice::ReloadFailed(Error::Timeout))
        });
        assert_eq!(w.snapshot().unwrap().profile, 0, "old snapshot kept");
        assert!(w.is_connected());
        // The keyboard answers again: the backoff retry succeeds on its own.
        let mut p = w.submit("answer", |kb| {
            push_snapshot(kb.session_mut().transport_mut(), 3);
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("retry", || w.snapshot().is_some_and(|s| s.profile == 3));
        wait_for("notice cleared", || w.notice().is_none());
    }

    #[test]
    fn an_unparseable_key_is_skipped_not_fatal() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                m.push_reply(reply(0x01, &[(25, &[3]), (26, b"App")]));
                m.push_reply(order_reply(0x21, 1));
                m.push_reply(order_reply(0x70, 0));
                m.push_reply(order_reply(0x50, 0));
                m.push_reply(reply(0x2B, &[(1, &[0]), (23, &[1]), (24, &KEYS)]));
                m.push_reply(reply(0x2B, &[(1, &[2]), (23, &[3])]));
                m.push_reply(reply(0x2B, &[(1, &[4]), (23, &[5])]));
                for (layout, v) in [
                    (8, 0),
                    (4, 2000),
                    (20, 20),
                    (21, 20),
                    (22, 200),
                    (23, 200),
                    (25, 0),
                ] {
                    m.push_reply(key_reply(0x1A, layout, v));
                }
                m.push_reply(key_reply(0x04, 8, 0x07)); // not a valid Mode
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let s = w.snapshot().unwrap();
        assert_eq!(s.keys.keys().copied().collect::<Vec<_>>(), vec![0x1A]);
        match w.notice() {
            Some(WorkerNotice::KeysSkipped(k)) => {
                assert_eq!(k.len(), 1);
                assert_eq!(k[0].0, 0x04);
                assert!(matches!(k[0].1, Error::Io(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_thread_that_ends_unexpectedly_leaves_an_error_status() {
        let ctl = Arc::new(Control {
            shared: Mutex::new(Shared {
                status: ConnectionStatus::Connected(test_snapshot()),
                current: Some("job"),
                notices: Notices::default(),
                repaint: None,
            }),
            generation: AtomicU64::new(0),
            outstanding: AtomicUsize::new(2),
            reload: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            connects: AtomicU64::new(0),
            lighting: AtomicU64::new(0),
            reconnect_ms: AtomicU64::new(NO_RECONNECT),
            calibrating: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
        });
        let c = Arc::clone(&ctl);
        let _ = std::thread::spawn(move || {
            let _guard = ExitGuard(c);
            panic!("outside any guarded call");
        })
        .join();
        assert_eq!(
            ctl.lock().status,
            ConnectionStatus::Error(i18n::device::WORKER_STOPPED.into())
        );
        assert_eq!(ctl.lock().current, None);
        assert_eq!(ctl.outstanding.load(Ordering::SeqCst), 0);

        // A normal shutdown (handle dropped) leaves the status alone.
        ctl.stop.store(true, Ordering::SeqCst);
        ctl.set_status(ConnectionStatus::Searching);
        drop(ExitGuard(Arc::clone(&ctl)));
        assert_eq!(ctl.lock().status, ConnectionStatus::Searching);
    }

    fn test_snapshot() -> DeviceSnapshot {
        let mut m = MockTransport::new();
        push_snapshot(&mut m, 0);
        load_snapshot(&mut Keyboard::new(m, Model::Win68HeMax))
            .unwrap()
            .0
    }

    #[test]
    fn event_effects_map_events_to_reload_patch_and_lighting() {
        use openhe_proto::SystemMode as M;
        assert_eq!(event_effects(&[]), EventEffects::default());
        let fx = event_effects(&[
            DeviceEvent::ProfileChanged(1),
            DeviceEvent::ProfileChanged(3),
        ]);
        assert_eq!(fx.profile, Some(3), "the newest profile wins");
        assert!(fx.reload && !fx.lighting);
        let fx = event_effects(&[DeviceEvent::SystemModeChanged(Some(M::Mac))]);
        assert_eq!(fx.system_mode, Some(Some(M::Mac)));
        assert!(fx.reload);
        assert!(event_effects(&[DeviceEvent::KeyDataChanged]).reload);
        let fx = event_effects(&[DeviceEvent::LightingChanged]);
        assert!(fx.lighting && !fx.reload);
        // Late RM6X21 headers and other noise do nothing.
        assert_eq!(
            event_effects(&[DeviceEvent::Other(0x92), DeviceEvent::Other(0x99)]),
            EventEffects::default()
        );
    }

    #[test]
    fn debounce_fires_once_after_a_quiet_window() {
        let t0 = Instant::now();
        let w = Duration::from_millis(100);
        let mut d = Debounce::new(w);
        assert!(!d.take_due(t0 + w * 10));
        d.bump(t0);
        d.bump(t0 + Duration::from_millis(60)); // the burst goes on
        assert!(!d.take_due(t0 + Duration::from_millis(120)));
        assert!(d.is_pending());
        assert!(d.take_due(t0 + Duration::from_millis(160)));
        assert!(!d.take_due(t0 + Duration::from_millis(500)), "fires once");
        d.bump(t0);
        d.cancel();
        assert!(!d.take_due(t0 + w * 10));
    }

    #[test]
    fn notices_are_stable_and_skipped_keys_can_be_dismissed() {
        let skipped = vec![(0x04, Error::Io("unknown key mode".into()))];
        let mut n = Notices::default();
        n.loaded(skipped.clone());
        assert!(matches!(n.current(), Some(WorkerNotice::KeysSkipped(_))));
        n.dismiss();
        assert_eq!(n.current(), None);
        // A reload with the same skipped keys does not bring it back.
        n.loaded(skipped.clone());
        assert_eq!(n.current(), None);
        // Other keys skipped: shown again.
        n.loaded(vec![(0x1A, Error::Io("x".into()))]);
        assert!(matches!(n.current(), Some(WorkerNotice::KeysSkipped(_))));

        // Failures outrank the skipped keys and clear only by their cause.
        n.events_failed = Some(Error::Timeout);
        assert_eq!(
            n.current(),
            Some(WorkerNotice::EventsFailed(Error::Timeout))
        );
        n.dismiss(); // not dismissable
        assert_eq!(
            n.current(),
            Some(WorkerNotice::EventsFailed(Error::Timeout))
        );
        n.loaded(vec![(0x1A, Error::Io("x".into()))]);
        assert_eq!(
            n.current(),
            Some(WorkerNotice::EventsFailed(Error::Timeout)),
            "a reload does not hide a failing event poll"
        );
        n.reload_failed = Some(Error::Timeout);
        assert_eq!(
            n.current(),
            Some(WorkerNotice::ReloadFailed(Error::Timeout))
        );
        n.loaded(Vec::new());
        n.events_failed = None;
        assert_eq!(n.current(), None);
    }

    #[test]
    fn handshake_starts_the_load() {
        let mut m = MockTransport::new();
        push_snapshot(&mut m, 0);
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        load_snapshot(&mut kb).unwrap();
        let first = kb.session_mut().transport().written()[0];
        assert_eq!(
            &first[..10],
            &[0x5C, 0x06, 0x01, 0x97, 1, 2, 3, 4, 0xFF, 0xFF]
        );
    }

    #[test]
    fn bootloader_sends_nothing_after_the_handshake() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0x01, &[(7, &[1])]));
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        assert_eq!(
            load_snapshot(&mut kb).unwrap_err(),
            Error::Unsupported("bootloader".into())
        );
        assert_eq!(kb.session_mut().transport().written().len(), 1);
    }

    /// A `MockTransport` that logs every write to a shared list (readable
    /// after the keyboard is gone) and can panic once on a zero-timeout
    /// read (the event poll).
    pub(crate) struct Probe {
        pub(crate) inner: MockTransport,
        log: Arc<Mutex<Vec<[u8; 64]>>>,
        panic_once: Arc<AtomicBool>,
    }

    impl Probe {
        /// A transport that logs its writes into `log`.
        pub(crate) fn logging(inner: MockTransport, log: Arc<Mutex<Vec<[u8; 64]>>>) -> Probe {
            Probe {
                inner,
                log,
                panic_once: Arc::default(),
            }
        }
    }

    impl Transport for Probe {
        fn write(&mut self, r: &[u8; 64]) -> Result<(), Error> {
            self.log.lock().unwrap().push(*r);
            self.inner.write(r)
        }

        fn read(&mut self, t: u32) -> Result<Option<[u8; 64]>, Error> {
            if t == 0 && self.panic_once.swap(false, Ordering::SeqCst) {
                panic!("read bug");
            }
            self.inner.read(t)
        }
    }

    pub(crate) const STOP_CALIBRATION: [u8; 7] = [0x5C, 0x03, 0x00, 0x93, 0x0D, 0xFF, 0xFF];

    pub(crate) fn wrote_stop(log: &Mutex<Vec<[u8; 64]>>) -> bool {
        log.lock()
            .unwrap()
            .iter()
            .any(|w| w[..7] == STOP_CALIBRATION)
    }

    #[test]
    fn a_profile_change_on_the_keyboard_shows_at_once_then_reloads() {
        // Review Focus 3.
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let g = w.generation();
        let mut p = w.submit("fn+key", |kb| {
            let m = kb.session_mut().transport_mut();
            m.push_pending(order_reply(0x70, 2));
            push_snapshot(m, 2);
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("patched", || w.snapshot().is_some_and(|s| s.profile == 2));
        assert!(w.generation() > g);
        // The debounced reload consumed the queued snapshot without errors.
        wait_for("reloaded", || !w.is_busy());
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(w.notice(), None);
    }

    fn polling_state(every: Duration) -> WorkerConfig {
        WorkerConfig {
            state_poll: every,
            ..fast()
        }
    }

    /// Writes the transport saw so far (read on the worker thread).
    fn writes(w: &Worker<MockTransport>) -> usize {
        let mut p = w.submit_quiet("count", |kb| {
            Ok(kb.session_mut().transport().written().len())
        });
        wait_result(&mut p).unwrap()
    }

    #[test]
    fn a_win_mac_switch_on_the_keyboard_is_found_by_the_state_poll() {
        // The keyboard sends no event for Fn+A / Fn+S (hardware finding
        // 15): the idle worker re-reads profile and OS mode, patches the
        // snapshot and reloads.
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0); // Windows
                m.push_reply(order_reply(0x70, 0)); // state poll: profile
                m.push_reply(order_reply(0x21, 0)); // state poll: Mac
                push_snapshot_os(&mut m, 0, 0); // the reload
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            polling_state(Duration::from_millis(20)),
        );
        wait_for("connect", || w.is_connected());
        assert_eq!(w.snapshot().unwrap().system_mode, Some(SystemMode::Windows));
        wait_for("mode", || {
            w.snapshot()
                .is_some_and(|s| s.system_mode == Some(SystemMode::Mac))
        });
        wait_for("reload", || !w.is_busy());
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(w.notice(), None, "the reload read the queued snapshot");
        assert_eq!(w.snapshot().unwrap().system_mode, Some(SystemMode::Mac));
    }

    #[test]
    fn a_profile_switch_found_by_the_state_poll_patches_then_reloads() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                m.push_reply(order_reply(0x70, 3));
                m.push_reply(order_reply(0x21, 1));
                push_snapshot(&mut m, 3);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            polling_state(Duration::from_millis(20)),
        );
        wait_for("connect", || w.is_connected());
        let g = w.generation();
        wait_for("profile", || w.snapshot().is_some_and(|s| s.profile == 3));
        assert!(w.generation() > g);
        wait_for("reload", || !w.is_busy());
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(w.notice(), None);
    }

    #[test]
    fn an_unchanged_state_poll_does_not_reload() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                // Plenty of unchanged answers. A reload would start with a
                // handshake, find only order replies, time out and raise
                // a ReloadFailed notice.
                for _ in 0..100 {
                    m.push_reply(order_reply(0x70, 0));
                    m.push_reply(order_reply(0x21, 1));
                }
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            polling_state(Duration::from_millis(10)),
        );
        wait_for("connect", || w.is_connected());
        let g = w.generation();
        let before = writes(&w);
        wait_for("state polls", || writes(&w) >= before + 4);
        assert_eq!(w.generation(), g, "nothing changed, nothing patched");
        assert_eq!(w.notice(), None, "no reload");
        assert_eq!(w.snapshot().unwrap().profile, 0);
    }

    #[test]
    fn the_state_poll_waits_while_calibrating() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            polling_state(Duration::from_millis(5)),
        );
        wait_for("connect", || w.is_connected());
        w.set_calibration_guard(true);
        let before = writes(&w);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(writes(&w), before, "no queries while calibrating");
        let mut p = w.submit_quiet("answers", |kb| {
            let m = kb.session_mut().transport_mut();
            m.push_reply(order_reply(0x70, 0));
            m.push_reply(order_reply(0x21, 1));
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        w.set_calibration_guard(false);
        wait_for("state poll", || writes(&w) >= before + 2);
    }

    #[test]
    fn a_burst_of_key_data_events_reloads_once() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        let mut p = w.submit("burst", |kb| {
            let m = kb.session_mut().transport_mut();
            for v in [1000, 1100, 1200] {
                m.push_pending(key_reply(0x1A, 4, v));
            }
            // Exactly one snapshot: a second reload would time out and
            // raise a ReloadFailed notice.
            push_snapshot(m, 3);
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("reload", || w.snapshot().is_some_and(|s| s.profile == 3));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(w.notice(), None, "one reload for the whole burst");
    }

    #[test]
    fn lighting_events_bump_a_counter_and_noise_is_ignored() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        assert_eq!(w.lighting_generation(), 0);
        let mut p = w.submit("light", |kb| {
            let m = kb.session_mut().transport_mut();
            m.push_pending(reply(0x18, &[(1, &[1])])); // 0x98: lighting
            m.push_pending(reply(0x12, &[(1, &[2])])); // 0x92: late RM6X21
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        wait_for("lighting", || w.lighting_generation() == 1);
        // No reload was attempted (none is queued, so it would fail).
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(w.notice(), None);
        assert_eq!(w.snapshot().unwrap().profile, 0);
    }

    #[test]
    fn events_are_polled_while_quiet_jobs_keep_the_worker_busy() {
        let w = Worker::spawn_with(
            Box::new(|| {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            WorkerConfig {
                retry_interval: Duration::from_millis(5),
                idle_poll: Duration::from_millis(20),
                state_poll: Duration::from_secs(3600),
            },
        );
        wait_for("connect", || w.is_connected());
        let mut p = w.submit("event", |kb| {
            kb.session_mut()
                .transport_mut()
                .push_pending(order_reply(0x70, 1));
            Ok(())
        });
        assert_eq!(wait_result(&mut p), Ok(()));
        // A quiet job every few ms, faster than idle_poll: the worker never
        // times out waiting, yet the event is still seen.
        let deadline = Instant::now() + Duration::from_secs(5);
        while w.snapshot().is_none_or(|s| s.profile != 1) {
            assert!(Instant::now() < deadline, "event never polled");
            let mut q = w.submit_quiet("poll", |_kb| Ok(()));
            assert!(!w.is_busy(), "quiet jobs never mark the app busy");
            assert_eq!(wait_result(&mut q), Ok(()));
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    #[test]
    fn a_panic_while_polling_events_reconnects() {
        // Deferred Track B issue: treated like a disconnect.
        let opened = Arc::new(AtomicUsize::new(0));
        let panic_once = Arc::new(AtomicBool::new(false));
        let (o, flag) = (opened.clone(), panic_once.clone());
        let w = Worker::spawn_with(
            Box::new(move || {
                o.fetch_add(1, Ordering::SeqCst);
                let mut inner = MockTransport::new();
                push_snapshot(&mut inner, 1);
                let t = Probe {
                    inner,
                    log: Arc::default(),
                    panic_once: flag.clone(),
                };
                Ok(Keyboard::new(t, Model::Win68HeMax))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        panic_once.store(true, Ordering::SeqCst);
        wait_for("reconnect", || {
            opened.load(Ordering::SeqCst) == 2 && w.is_connected()
        });
        assert!(!panic_once.load(Ordering::SeqCst));
        let mut r = w.submit("after", |kb| Ok(kb.model));
        assert_eq!(wait_result(&mut r), Ok(Model::Win68HeMax));
    }

    #[test]
    fn reconnect_drops_the_keyboard_and_connects_again() {
        let opened = Arc::new(AtomicUsize::new(0));
        let w = Worker::spawn_with(
            pluggable(Arc::new(AtomicBool::new(true)), opened.clone()),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        assert_eq!(w.connects(), 1);
        w.reconnect(Duration::from_millis(30));
        wait_for("searching", || w.status() == ConnectionStatus::Searching);
        let mut p = w.submit("during-pause", |kb| Ok(kb.model));
        assert_eq!(wait_result(&mut p), Err(Error::Disconnected));
        wait_for("reconnect", || w.is_connected() && w.connects() == 2);
        assert_eq!(opened.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn calibration_guard_stops_calibration_on_reconnect() {
        let log: Arc<Mutex<Vec<[u8; 64]>>> = Arc::default();
        let l = log.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                let mut inner = MockTransport::new();
                push_handshake(&mut inner);
                inner.push_reply(order_reply(0x0D, 1)); // stop, if sent
                push_snapshot_rest(&mut inner, 0);
                Ok(Keyboard::new(
                    Probe {
                        inner,
                        log: l.clone(),
                        panic_once: Arc::default(),
                    },
                    Model::Win68HeMax,
                ))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        assert!(!wrote_stop(&log), "no guard, no stop");
        w.set_calibration_guard(true);
        w.reconnect(Duration::from_millis(1));
        wait_for("guard cleared", || !w.calibration_guard());
        wait_for("reconnect", || w.is_connected() && w.connects() == 2);
        let writes = log.lock().unwrap().clone();
        let stop_at = writes
            .iter()
            .position(|w| w[..7] == STOP_CALIBRATION)
            .expect("stop sent");
        // Sent right after the handshake on the new connection (never
        // before it: a keyboard in bootloader mode gets no write at all).
        assert_eq!(writes[stop_at - 1][..4], [0x5C, 0x06, 0x01, 0x97]);
    }

    #[test]
    fn calibration_guard_never_writes_to_a_bootloader_keyboard() {
        let log: Arc<Mutex<Vec<[u8; 64]>>> = Arc::default();
        let (l, plugged) = (log.clone(), Arc::new(AtomicBool::new(false)));
        let tries = Arc::new(AtomicUsize::new(0));
        let (p, t) = (plugged.clone(), tries.clone());
        let w = Worker::spawn_with(
            Box::new(move || {
                if !p.load(Ordering::SeqCst) {
                    return Err(Error::Unsupported("no supported keyboard was found".into()));
                }
                t.fetch_add(1, Ordering::SeqCst);
                let mut inner = MockTransport::new();
                inner.push_reply(reply(0x01, &[(7, &[1])])); // bootloader
                inner.push_reply(order_reply(0x0D, 1)); // stop, if sent
                Ok(Keyboard::new(
                    Probe::logging(inner, l.clone()),
                    Model::Win68HeMax,
                ))
            }),
            fast(),
        );
        w.set_calibration_guard(true);
        plugged.store(true, Ordering::SeqCst);
        wait_for("bootloader seen", || {
            w.status() == ConnectionStatus::Error(i18n::error::BOOTLOADER.into())
                && tries.load(Ordering::SeqCst) >= 2
        });
        assert!(
            !wrote_stop(&log),
            "no stop to a keyboard in bootloader mode"
        );
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .all(|w| w[..4] == [0x5C, 0x06, 0x01, 0x97]));
        assert!(
            w.calibration_guard(),
            "still owed once a real keyboard is back"
        );
    }

    #[test]
    fn calibration_guard_stops_calibration_when_the_app_closes() {
        let log: Arc<Mutex<Vec<[u8; 64]>>> = Arc::default();
        let l = log.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                let mut inner = MockTransport::new();
                push_snapshot(&mut inner, 0);
                inner.push_reply(order_reply(0x0D, 1));
                Ok(Keyboard::new(
                    Probe {
                        inner,
                        log: l.clone(),
                        panic_once: Arc::default(),
                    },
                    Model::Win68HeMax,
                ))
            }),
            fast(),
        );
        wait_for("connect", || w.is_connected());
        w.set_calibration_guard(true);
        // A slow job (like an RM6X21 poll timing out) is running when the
        // app closes: dropping the handle waits for it and for the stop.
        let _slow = w.submit_quiet("slow-poll", |_kb| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        });
        std::thread::sleep(Duration::from_millis(20));
        drop(w);
        assert!(wrote_stop(&log), "stop sent before drop returned");
    }
}

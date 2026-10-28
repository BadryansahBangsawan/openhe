//! The device worker: one background thread that owns the keyboard.
//!
//! - While no keyboard is open it tries to connect every second
//!   (`HidTransport::open_first`), and fails any submitted job with
//!   `Err(Disconnected)` straight away.
//! - On connect it loads a [`DeviceSnapshot`] (info, system mode, profile,
//!   polling rate, matrix, and the settings of every matrix key).
//! - Pages talk to the keyboard only through [`Worker::submit`]: a closure
//!   that runs on the worker thread with `&mut Keyboard`, and a [`Pending`]
//!   handle the UI polls each frame without blocking.
//! - A job that ends in `Err(Disconnected)` drops the keyboard: the status
//!   goes back to `Searching` (before the job's result is delivered) and the
//!   worker reconnects on its own.
//!
//! The transport is a type parameter with a connector closure, so tests drive
//! the worker with `MockTransport`; the app uses the `HidTransport` default.

use openhe_device::{Error, HidTransport, KeySettings, Keyboard, Transport};
use openhe_proto::frame::Response;
use openhe_proto::{DeviceInfo, Matrix, Model, PollingRate, SystemMode};
use std::collections::BTreeMap;
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
    /// Settings of every key in `matrix`, by default keycode.
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
    /// (permissions, bootloader mode, a protocol error). Still retrying.
    Error(String),
}

/// Opens a keyboard. Called on the worker thread.
pub type Connector<T> = Box<dyn FnMut() -> Result<Keyboard<T>, Error> + Send>;

#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    /// Pause between connect attempts while disconnected.
    pub retry_interval: Duration,
    /// How long the idle worker waits for a job before polling for events.
    pub idle_poll: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        WorkerConfig {
            retry_interval: Duration::from_secs(1),
            idle_poll: Duration::from_millis(50),
        }
    }
}

/// Label shown while the snapshot loads.
pub const LOAD_LABEL: &str = "load-snapshot";

/// What a finished job hands back to the worker loop.
struct JobDone {
    disconnected: bool,
    deliver: Box<dyn FnOnce() + Send>,
}

type Job<T> = Box<dyn FnOnce(Option<&mut Keyboard<T>>) -> JobDone + Send>;

struct Msg<T: Transport> {
    label: &'static str,
    job: Job<T>,
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    status: ConnectionStatus,
    current: Option<&'static str>,
    last_error: Option<String>,
    repaint: Option<Repaint>,
}

/// State shared between the handle and the thread.
struct Control {
    shared: Mutex<Shared>,
    generation: AtomicU64,
    outstanding: AtomicUsize,
    reload: AtomicBool,
    stop: AtomicBool,
}

impl Control {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        // A panicking job cannot leave Shared half-updated (every write is a
        // single assignment), so a poisoned lock is still usable.
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

    fn set_current(&self, label: Option<&'static str>) {
        self.lock().current = label;
        self.repaint();
    }

    fn set_last_error(&self, e: Option<String>) {
        self.lock().last_error = e;
    }
}

/// Handle to the worker thread. Dropping it stops the thread after its
/// current step (it is not joined, so closing the window never waits on a
/// slow keyboard).
pub struct Worker<T: Transport + 'static = HidTransport> {
    tx: Sender<Msg<T>>,
    ctl: Arc<Control>,
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
                last_error: None,
                repaint: None,
            }),
            generation: AtomicU64::new(0),
            outstanding: AtomicUsize::new(0),
            reload: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let thread_ctl = Arc::clone(&ctl);
        std::thread::Builder::new()
            .name("openhe-worker".into())
            .spawn(move || run(connect, rx, thread_ctl, cfg))
            .expect("spawn the worker thread");
        Worker { tx, ctl }
    }

    /// Queues `job` to run on the worker thread with the open keyboard.
    /// Jobs run one at a time, in order. While no keyboard is open the job is
    /// not run and its result is `Err(Disconnected)`.
    pub fn submit<R: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&mut Keyboard<T>) -> Result<R, Error> + Send + 'static,
    ) -> Pending<R> {
        let (rtx, rrx) = mpsc::channel();
        let wrapped: Job<T> = Box::new(move |kb| {
            let ran = kb.is_some();
            let result = match kb {
                Some(k) => job(k),
                None => Err(Error::Disconnected),
            };
            JobDone {
                disconnected: ran && matches!(result, Err(Error::Disconnected)),
                deliver: Box::new(move || {
                    let _ = rtx.send(result);
                }),
            }
        });
        self.ctl.outstanding.fetch_add(1, Ordering::SeqCst);
        if self
            .tx
            .send(Msg {
                label,
                job: wrapped,
            })
            .is_err()
        {
            // The thread is gone; dropping the job drops its sender.
            self.ctl.outstanding.fetch_sub(1, Ordering::SeqCst);
        }
        Pending {
            rx: rrx,
            label,
            done: false,
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

    /// Asks the worker to re-read the whole snapshot when it is next idle.
    pub fn request_reload(&self) {
        self.ctl.reload.store(true, Ordering::SeqCst);
    }

    /// Edits the current snapshot in place (no-op while disconnected), e.g.
    /// to store key settings a job just read back.
    pub fn patch_snapshot(&self, f: impl FnOnce(&mut DeviceSnapshot)) {
        let patched = {
            let mut g = self.ctl.lock();
            match &mut g.status {
                ConnectionStatus::Connected(s) => {
                    f(s);
                    true
                }
                _ => false,
            }
        };
        if patched {
            self.ctl.generation.fetch_add(1, Ordering::SeqCst);
            self.ctl.repaint();
        }
    }

    /// Bumped whenever the status or the snapshot changes.
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

    /// The last error of a background reload or event poll, if any.
    pub fn last_error(&self) -> Option<String> {
        self.ctl.lock().last_error.clone()
    }

    /// Called from the worker thread whenever something changed, so the UI
    /// can wake up (the app passes `egui::Context::request_repaint`).
    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        self.ctl.lock().repaint = Some(Arc::new(f));
    }
}

impl<T: Transport + 'static> Drop for Worker<T> {
    fn drop(&mut self) {
        self.ctl.stop.store(true, Ordering::SeqCst);
    }
}

/// Reads everything the UI shows. Refuses a keyboard in bootloader mode
/// before sending anything else.
pub fn load_snapshot<T: Transport>(kb: &mut Keyboard<T>) -> Result<DeviceSnapshot, Error> {
    // TODO(A3): use `handshake()` once Track A3 is merged.
    let info = kb.info()?;
    if info.in_bootloader {
        return Err(Error::Unsupported("bootloader".into()));
    }
    let system_mode = kb.system_mode()?;
    let profile = kb.profile()?;
    let polling = kb.polling_rate()?;
    let matrix = kb.matrix()?;
    let mut keys = BTreeMap::new();
    let codes: Vec<u8> = matrix.keys().map(|(_, _, c)| c).collect();
    for code in codes {
        keys.insert(code, kb.key_settings(code)?);
    }
    Ok(DeviceSnapshot {
        model: kb.model,
        info,
        system_mode,
        profile,
        polling,
        matrix,
        keys,
    })
}

/// Event adapter: drains unsolicited reports without blocking.
///
/// Track F swaps the body for `Keyboard::poll_events` (Track A3) and maps
/// `DeviceEvent`s instead of raw replies.
pub fn poll_events<T: Transport>(kb: &mut Keyboard<T>) -> Result<Vec<Response>, Error> {
    let mut fresh = Vec::new();
    while let Some(raw) = kb.session_mut().transport_mut().read(0)? {
        if let Ok(r) = Response::decode(&raw) {
            fresh.push(r);
        }
    }
    let mut events = kb.take_events();
    events.extend(fresh);
    Ok(events)
}

/// Profile (order 0x70) or system-mode (order 0x21) changes made on the
/// keyboard itself invalidate the snapshot.
pub fn needs_reload(r: &Response) -> bool {
    r.cmd == 0x00 && matches!(r.body()[1], 0x70 | 0x21)
}

fn connect_error_status(e: &Error) -> ConnectionStatus {
    match e {
        Error::Unsupported(_) | Error::Disconnected => ConnectionStatus::Searching,
        other => ConnectionStatus::Error(other.to_string()),
    }
}

fn load_error_status(e: &Error) -> ConnectionStatus {
    match e {
        Error::Disconnected => ConnectionStatus::Searching,
        other => ConnectionStatus::Error(other.to_string()),
    }
}

fn run<T: Transport>(
    mut connect: Connector<T>,
    rx: Receiver<Msg<T>>,
    ctl: Arc<Control>,
    cfg: WorkerConfig,
) {
    let mut kb: Option<Keyboard<T>> = None;
    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(k) = kb.as_mut() else {
            match connect() {
                Ok(mut k) => {
                    ctl.set_current(Some(LOAD_LABEL));
                    let loaded = load_snapshot(&mut k);
                    ctl.set_current(None);
                    match loaded {
                        Ok(snap) => {
                            ctl.reload.store(false, Ordering::SeqCst);
                            ctl.set_last_error(None);
                            ctl.set_status(ConnectionStatus::Connected(snap));
                            kb = Some(k);
                            continue;
                        }
                        Err(e) => ctl.set_status(load_error_status(&e)),
                    }
                }
                Err(e) => ctl.set_status(connect_error_status(&e)),
            }
            // Wait before retrying; jobs arriving meanwhile fail at once.
            let deadline = Instant::now() + cfg.retry_interval;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || ctl.stop.load(Ordering::SeqCst) {
                    break;
                }
                match rx.recv_timeout(left) {
                    Ok(msg) => finish(&ctl, (msg.job)(None)),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            continue;
        };

        match rx.recv_timeout(cfg.idle_poll) {
            Ok(msg) => {
                ctl.set_current(Some(msg.label));
                let done = (msg.job)(Some(k));
                ctl.set_current(None);
                if done.disconnected {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                }
                finish(&ctl, done);
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        // Idle: look for events from the keyboard, then reload if asked.
        match poll_events(k) {
            Ok(events) => {
                if events.iter().any(needs_reload) {
                    ctl.reload.store(true, Ordering::SeqCst);
                }
            }
            Err(Error::Disconnected) => {
                kb = None;
                ctl.set_status(ConnectionStatus::Searching);
                continue;
            }
            Err(e) => ctl.set_last_error(Some(e.to_string())),
        }
        if ctl.reload.swap(false, Ordering::SeqCst) {
            ctl.set_current(Some(LOAD_LABEL));
            let loaded = load_snapshot(k);
            ctl.set_current(None);
            match loaded {
                Ok(snap) => {
                    ctl.set_last_error(None);
                    ctl.set_status(ConnectionStatus::Connected(snap));
                }
                Err(Error::Disconnected) => {
                    kb = None;
                    ctl.set_status(ConnectionStatus::Searching);
                }
                // Keep the old snapshot and the connection; say what failed.
                Err(e) => ctl.set_last_error(Some(e.to_string())),
            }
        }
    }
}

fn finish(ctl: &Control, done: JobDone) {
    (done.deliver)();
    ctl.outstanding.fetch_sub(1, Ordering::SeqCst);
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
        m.push_reply(reply(0x01, &[(25, &[3]), (26, b"App")]));
        m.push_reply(order_reply(0x21, 1));
        m.push_reply(order_reply(0x70, profile));
        m.push_reply(order_reply(0x50, 0));
        m.push_reply(reply(0x2B, &[(1, &[0]), (23, &[1]), (24, &KEYS)]));
        m.push_reply(reply(0x2B, &[(1, &[2]), (23, &[3])]));
        m.push_reply(reply(0x2B, &[(1, &[4]), (23, &[5])]));
        for key in KEYS {
            for (layout, v) in [
                (8, 0),
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
            w.status() == ConnectionStatus::Error("permission denied: denied".into())
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
            w.status() == ConnectionStatus::Error("unsupported: bootloader".into())
        });
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
    fn needs_reload_matches_profile_and_mode_reports_only() {
        let d = |raw: [u8; 64]| Response::decode(&raw).unwrap();
        assert!(needs_reload(&d(order_reply(0x70, 1))));
        assert!(needs_reload(&d(order_reply(0x21, 1))));
        assert!(!needs_reload(&d(order_reply(0x50, 1))));
        assert!(!needs_reload(&d(key_reply(0x70, 4, 0))));
    }
}

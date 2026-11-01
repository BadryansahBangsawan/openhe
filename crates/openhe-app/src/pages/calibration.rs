//! Kalibrasi: re-measures every switch's travel (mockup
//! `docs/design/mockups/06-kalibrasi.png`, PROTOCOL 3.4).
//!
//! Flow: `start_calibration`, then poll live travel
//! as fast as the replies come back (one quiet job in flight, at
//! least [`MIN_POLL_GAP`] between starts). A key counts as calibrated once
//! its peak travel this session reaches `max travel - top deadzone - bottom
//! deadzone - 0.1 mm` ([`calibrated_threshold`]); the peak is held, so the
//! key stays marked after release. A press that bottoms out entirely
//! between two samples is still invisible to the app; fast polling keeps
//! that window short. "Simpan" and "Batal" both
//! end with `stop_calibration` (the protocol has only one close command).
//!
//! The keyboard types nothing while it calibrates, so calibration is
//! always stopped: on Simpan/Batal, when the user leaves the page or closes
//! the app ([`Page::leave`]), after any start or poll error, and on a lost
//! connection (the worker's calibration guard sends the stop on reconnect).
//! The RM6X21 framing is unverified on hardware; a frame that fails to parse
//! or times out ends the calibration with a clear message.
//!
//! [`Lifecycle`] is the pure state machine; [`Progress`] the per-key
//! bookkeeping. Both are unit-tested without a UI.

use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{
    legend_list, reference_matrix, KeyDecoration, KeyboardView, Label2, Selection,
};
use crate::widgets::slider_mm::mm_short;
use crate::worker::{DeviceSnapshot, Pending, Worker};
use eframe::egui::{self, Align, CornerRadius, Layout, RichText, Sense, Ui};
use openhe_device::{Error, Keyboard, Transport};
use openhe_proto::travel::TravelFrame;
use openhe_proto::Model;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

mod strings {
    pub const KEYS: &str = "tombol";
    pub const TITLE_IDLE: &str = "Kalibrasi";
    pub const TITLE_RUNNING: &str = "Kalibrasi berjalan";
    pub const NO_TYPING: &str = "Selama kalibrasi keyboard tidak bisa mengetik.";
    pub const SWITCH: &str = "Switch";
    pub const MIXED: &str = "Campuran";
    pub const MEASURED: &str = "Jarak tekan terukur";
    pub const LAST: &str = "Terakhir dikalibrasi";
    pub const LAST_NEVER: &str = "belum tercatat";
    pub const LAST_SESSION: &str = "di sesi ini";
    pub const REDO_WARNING: &str = "Ulangi kalibrasi setelah mengganti switch.";
    pub const START: &str = "Mulai kalibrasi";
    pub const AGAIN: &str = "Kalibrasi lagi";
    pub const STARTING: &str = "Memulai…";
    pub const STOPPING: &str = "Menghentikan…";
    pub const CANCEL: &str = "Batal";
    pub const SAVE: &str = "Simpan";
    pub const LEFT_SUFFIX: &str = "lagi";
    pub const PRESS: &str = "tekan";

    pub const STEP1: &str = "1. Lepas semua tombol";
    pub const STEP1_BODY: &str = "Jangan sentuh keyboard selama 2 detik.";
    pub const STEP2: &str = "2. Tekan tiap tombol sampai mentok";
    pub const STEP2_BODY: &str = "Pelan-pelan, lalu lepas.";
    pub const STEP2_LEFT: &str = "Sisa:";
    pub const STEP2_ALL: &str = "Semua tombol sudah terukur.";
    pub const STEP3: &str = "3. Simpan";
    pub const STEP3_BODY: &str = "Hasil disimpan di keyboard.";

    pub const SAVED: &str = "Kalibrasi selesai dan disimpan di keyboard.";
    pub const CANCELLED: &str =
        "Kalibrasi dihentikan. Hasil yang sudah terukur mungkin tersimpan di keyboard.";
    pub const FAILED: &str = "Kalibrasi dihentikan karena ada masalah";
    pub const FRAME_HINT: &str =
        "Data jarak tekan dari keyboard tidak terbaca. Mode kalibrasi sudah diminta berhenti.";
    pub const INTERRUPTED: &str =
        "Keyboard terputus saat kalibrasi. Mode kalibrasi dihentikan begitu keyboard tersambung lagi.";
    pub const INTERRUPTED_DONE: &str =
        "Keyboard terputus saat kalibrasi; mode kalibrasi sudah dihentikan.";
    pub const STOP_FAILED: &str =
        "Keyboard belum menjawab perintah berhenti; mungkin masih dalam mode kalibrasi (tidak bisa mengetik).";
    pub const STOP_AGAIN: &str = "Hentikan lagi";
    /// Technical detail when the worker's stop on reconnect did not work.
    pub const RECONNECT_STOP_TECH: &str =
        "stop_calibration after reconnecting failed (calibration guard still set)";
}
use strings as t;

/// Shortest time between the starts of two progress polls. A poll (four
/// RM6X21 requests) usually takes longer, so polling runs back to back:
/// the official app's fixed 60 ms (PROTOCOL 3.4) missed quick presses on
/// hardware.
pub const MIN_POLL_GAP: Duration = Duration::from_millis(15);
/// Step 1: the keyboard measures the rest position; keys pressed meanwhile
/// do not count.
pub const SETTLE: Duration = Duration::from_secs(2);
/// The 0.1 mm margin of the calibrated rule.
pub const MARGIN_UM: u16 = 100;
/// Deeper than this, an unmeasured key is shown as being pressed.
pub const PRESSED_UM: u16 = 300;
/// Used when a key's settings are unknown: the factory deadzones.
const DEFAULT_DZ_UM: u16 = 200;

// ------------------------------------------------------------ threshold

/// Travel a key must reach to count as calibrated: `max travel - top
/// deadzone - bottom deadzone - 0.1 mm` (PROTOCOL 3.4), never below 1 µm (a
/// resting key never counts).
pub fn calibrated_threshold(max_travel: u16, top_dz: u16, bottom_dz: u16) -> u16 {
    max_travel
        .saturating_sub(top_dz)
        .saturating_sub(bottom_dz)
        .saturating_sub(MARGIN_UM)
        .max(1)
}

/// `(max travel, top dz, bottom dz)` for a key, from its switch and
/// deadzones; factory values when the key is missing from the snapshot.
pub fn key_limits(model: Model, snap: &DeviceSnapshot, code: u8) -> (u16, u16, u16) {
    let switch_max = |id: u16| {
        model
            .switches()
            .iter()
            .find(|s| s.id == id)
            .or_else(|| model.switches().first())
            .map_or(3300, |s| s.max_travel.0)
    };
    match snap.keys.get(&code) {
        Some(k) => (switch_max(k.switch), k.top_dz.0, k.bottom_dz.0),
        None => (switch_max(0), DEFAULT_DZ_UM, DEFAULT_DZ_UM),
    }
}

// ------------------------------------------------------------- progress

/// One poll: live travel in matrix positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub travel: TravelFrame,
}

/// The poll job: RM6X21 (2,1)+(2,2), travel only. The raw calibration
/// values (6,1)+(6,2) are not shown anywhere; reading them too doubled the
/// round trips of every sample, so quick presses were missed more often.
pub fn poll_job<T: Transport>(kb: &mut Keyboard<T>) -> Result<Sample, Error> {
    let travel = kb.travel()?;
    Ok(Sample { travel })
}

/// Per-key calibration progress.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// Travel right now, µm.
    pub current: BTreeMap<u8, u16>,
    /// Deepest travel seen since Mulai (counting samples only), µm. A new
    /// session starts from an empty `Progress`.
    pub peak: BTreeMap<u8, u16>,
    /// Keys that reached their threshold (sticky).
    pub done: BTreeSet<u8>,
}

impl Progress {
    /// Takes in one sample. Only `count` samples (after the settle time)
    /// can mark keys as calibrated.
    ///
    /// A sample with travel where the matrix has no key (halves of the
    /// frame mixed up, see `TravelFrame::fits`) is ignored, so it can never
    /// mark the wrong keys as calibrated.
    pub fn apply(&mut self, snap: &DeviceSnapshot, s: &Sample, count: bool) {
        if !s.travel.fits(&snap.matrix) {
            return;
        }
        for (row, col, code) in snap.matrix.keys() {
            let (r, c) = (usize::from(row), usize::from(col));
            let Some(tr) = s.travel.um.get(r).and_then(|x| x.get(c)) else {
                continue;
            };
            self.current.insert(code, *tr);
            if !count {
                continue;
            }
            // Peak hold: the deepest travel of the session decides.
            let peak = self.peak.entry(code).or_insert(0);
            *peak = (*peak).max(*tr);
            let (max, top, bottom) = key_limits(snap.model, snap, code);
            if *peak >= calibrated_threshold(max, top, bottom) {
                self.done.insert(code);
            }
        }
    }

    /// Keys still to press, in matrix (reading) order.
    pub fn remaining(&self, snap: &DeviceSnapshot) -> Vec<u8> {
        snap.matrix
            .keys()
            .map(|(_, _, c)| c)
            .filter(|c| !self.done.contains(c))
            .collect()
    }

    /// Shallowest and deepest peak over the calibrated keys.
    pub fn measured_range(&self) -> Option<(u16, u16)> {
        let peaks = self.done.iter().filter_map(|c| self.peak.get(c).copied());
        peaks.fold(None, |acc, p| match acc {
            None => Some((p, p)),
            Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
        })
    }
}

// ------------------------------------------------------------ lifecycle

/// Why a calibration is being stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Saved,
    Cancelled,
    /// The user left the page or closed the app.
    Left,
    /// A start or poll error (Indonesian text, technical text).
    Failed(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// `start_calibration` sent.
    Starting,
    Running {
        since: Instant,
    },
    /// `stop_calibration` sent.
    Stopping(StopReason),
}

/// How the last calibration ended (shown until the next start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    Cancelled,
    /// Stopped after an error (Indonesian text, technical text).
    Failed(String, String),
    /// The connection was lost; the worker stops calibration on reconnect.
    Interrupted,
    /// The stop itself failed: the keyboard may still be calibrating.
    StopFailed(String),
}

/// A device command the page must send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Start,
    Poll,
    Stop,
}

/// The calibration state machine. Every way out of `Starting`/`Running`
/// except a lost connection goes through `Stop`; a lost connection leaves
/// the stop to the worker's calibration guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifecycle {
    pub phase: Phase,
    pub poll_in_flight: bool,
    last_poll: Option<Instant>,
    pub outcome: Option<Outcome>,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Lifecycle {
            phase: Phase::Idle,
            poll_in_flight: false,
            last_poll: None,
            outcome: None,
        }
    }
}

impl Lifecycle {
    /// Starting, running or stopping: the keyboard may be calibrating.
    pub fn is_active(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// Samples count once the settle time has passed.
    pub fn counting(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Running { since } if now.duration_since(since) >= SETTLE)
    }

    pub fn start(&mut self) -> Option<Cmd> {
        if self.phase != Phase::Idle {
            return None;
        }
        self.phase = Phase::Starting;
        self.outcome = None;
        self.poll_in_flight = false;
        self.last_poll = None;
        Some(Cmd::Start)
    }

    pub fn started(&mut self, r: Result<(), Error>, now: Instant) -> Option<Cmd> {
        if self.phase != Phase::Starting {
            return None; // stopped meanwhile; the stop is already queued
        }
        match r {
            Ok(()) => {
                self.phase = Phase::Running { since: now };
                None
            }
            Err(Error::Disconnected) => {
                self.lost();
                None
            }
            // The keyboard may have entered calibration anyway: stop it.
            Err(e) => self.stop_with(StopReason::Failed(
                i18n::device_error(&e),
                i18n::technical(&e),
            )),
        }
    }

    /// Asks for a poll when one is due and none is in flight.
    pub fn tick(&mut self, now: Instant) -> Option<Cmd> {
        if !self.is_running() || self.poll_in_flight {
            return None;
        }
        if self
            .last_poll
            .is_some_and(|t| now.duration_since(t) < MIN_POLL_GAP)
        {
            return None;
        }
        self.poll_in_flight = true;
        self.last_poll = Some(now);
        Some(Cmd::Poll)
    }

    /// A poll finished. Returns `Stop` after an error.
    pub fn polled<S>(&mut self, r: &Result<S, Error>) -> Option<Cmd> {
        self.poll_in_flight = false;
        if !self.is_running() {
            return None;
        }
        match r {
            Ok(_) => None,
            Err(Error::Disconnected) => {
                self.lost();
                None
            }
            Err(e) => {
                let text = match e {
                    Error::Io(_) | Error::Frame(_) => {
                        format!("{} {}", i18n::device_error(e), t::FRAME_HINT)
                    }
                    other => i18n::device_error(other),
                };
                self.stop_with(StopReason::Failed(text, i18n::technical(e)))
            }
        }
    }

    /// Simpan (`save`) or Batal.
    pub fn finish(&mut self, save: bool) -> Option<Cmd> {
        let reason = if save {
            StopReason::Saved
        } else {
            StopReason::Cancelled
        };
        self.stop_with(reason)
    }

    /// The page is left or the app closes.
    pub fn leave(&mut self) -> Option<Cmd> {
        self.stop_with(StopReason::Left)
    }

    /// After a failed stop: try again.
    pub fn retry_stop(&mut self) -> Option<Cmd> {
        if matches!(self.outcome, Some(Outcome::StopFailed(_))) {
            return self.stop_owed();
        }
        None
    }

    /// A stop the keyboard may still need although nothing runs here (the
    /// worker's calibration guard is still set, e.g. after a failed stop or
    /// a reconnect whose stop failed). Only while idle.
    pub fn stop_owed(&mut self) -> Option<Cmd> {
        if self.phase != Phase::Idle {
            return None;
        }
        self.phase = Phase::Stopping(StopReason::Cancelled);
        Some(Cmd::Stop)
    }

    fn stop_with(&mut self, reason: StopReason) -> Option<Cmd> {
        match self.phase {
            Phase::Starting | Phase::Running { .. } => {
                self.phase = Phase::Stopping(reason);
                Some(Cmd::Stop)
            }
            _ => None,
        }
    }

    /// The keyboard disconnected (or was reopened) while active.
    pub fn connection_lost(&mut self) {
        if self.is_active() {
            self.lost();
        }
    }

    fn lost(&mut self) {
        self.phase = Phase::Idle;
        self.poll_in_flight = false;
        self.outcome = Some(Outcome::Interrupted);
    }

    /// The stop finished.
    pub fn stopped(&mut self, r: Result<(), Error>) {
        let Phase::Stopping(reason) = self.phase.clone() else {
            return;
        };
        self.phase = Phase::Idle;
        self.poll_in_flight = false;
        self.outcome = match r {
            Ok(()) => match reason {
                StopReason::Saved => Some(Outcome::Saved),
                StopReason::Cancelled => Some(Outcome::Cancelled),
                StopReason::Left => None,
                StopReason::Failed(msg, tech) => Some(Outcome::Failed(msg, tech)),
            },
            Err(Error::Disconnected) => Some(Outcome::Interrupted),
            Err(e) => Some(Outcome::StopFailed(i18n::technical(&e))),
        };
    }
}

/// "28-10-2026" (day-month-year, UTC) for Unix seconds.
pub fn date_label(secs: u64) -> String {
    // Days to civil date (H. Hinnant's algorithm), for days >= 0.
    let z = secs / 86_400 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{day:02}-{month:02}-{year}")
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ----------------------------------------------------------------- page

/// How long the app waits on exit for the calibration stop: a worst-case
/// poll in flight (two RM6X21 calls of up to 3 × 1 s each when the framing
/// is wrong) plus the stop itself.
pub const EXIT_STOP_WAIT: Duration = Duration::from_secs(7);

/// Waits up to `timeout` for a stop job's own result; clears the worker's
/// calibration guard when the stop worked. True when it worked.
pub fn wait_for_stop<T: Transport + 'static>(
    worker: &Worker<T>,
    mut stop: Pending<()>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(r) = stop.poll() {
            let ok = r.is_ok();
            if ok {
                worker.set_calibration_guard(false);
            }
            return ok;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Default)]
pub struct Page {
    life: Lifecycle,
    progress: Progress,
    start: Option<Pending<()>>,
    poll: Option<Pending<Sample>>,
    stop: Option<Pending<()>>,
    /// `Worker::connects()` when calibration started: a reconnect since
    /// means the keyboard was reopened (and the worker stopped it).
    connects_at_start: u64,
    saved_this_session: bool,
    /// A calibration was saved and not yet recorded in the data file.
    saved_unrecorded: bool,
    selection: Selection,
}

impl Page {
    /// Starting, running or stopping (the top bar stays locked).
    pub fn is_active(&self) -> bool {
        self.life.is_active()
    }

    /// App exit. Whatever the page or phase (also after a failed stop,
    /// when the phase is already `Idle`), if the worker's calibration guard
    /// is set the keyboard may still be calibrating: returns a fresh stop
    /// job to wait on with [`wait_for_stop`]. It is queued behind any poll
    /// or stop already in flight.
    pub fn exit_stop<T: Transport + 'static>(&mut self, worker: &Worker<T>) -> Option<Pending<()>> {
        // The page is done either way; the stop below replaces the one
        // `leave` would send.
        let _ = self.life.leave();
        worker
            .calibration_guard()
            .then(|| worker.submit("calibration-stop-exit", |kb| kb.stop_calibration()))
    }

    /// The shell's "Hentikan kalibrasi": the worker's calibration guard is
    /// set while nothing runs here, so the keyboard may still be
    /// calibrating. Sends a stop (picked up by [`Page::pump`], which clears
    /// the guard once it worked). False when nothing was sent.
    pub fn stop_owed<T: Transport + 'static>(&mut self, worker: &Worker<T>) -> bool {
        if !worker.calibration_guard() {
            return false;
        }
        let cmd = self.life.stop_owed();
        let sent = cmd.is_some();
        self.exec(worker, cmd);
        sent
    }

    /// The connection came back while the last calibration ended as
    /// `Interrupted`: if the worker's calibration guard is still set, its
    /// stop on reconnect did not work, so the keyboard may still be
    /// calibrating. That is a failed stop (with "Hentikan lagi"), not an
    /// interruption that has been taken care of.
    fn after_reconnect(&mut self, connected: bool, connects: u64, guard: bool) {
        if self.life.outcome == Some(Outcome::Interrupted)
            && self.life.phase == Phase::Idle
            && connected
            && connects != self.connects_at_start
            && guard
        {
            self.life.outcome = Some(Outcome::StopFailed(t::RECONNECT_STOP_TECH.to_string()));
        }
    }

    /// Called when the user leaves the page: stops a running calibration
    /// (the result is picked up by [`Page::pump`]).
    pub fn leave(&mut self, worker: &Worker) {
        let cmd = self.life.leave();
        self.exec(worker, cmd);
    }

    /// Runs every frame, whichever page is shown: collects finished jobs,
    /// notices a lost connection and sends the next poll.
    pub fn pump<T: Transport + 'static>(
        &mut self,
        worker: &Worker<T>,
        snapshot: Option<&DeviceSnapshot>,
        ui: &egui::Context,
    ) {
        let now = Instant::now();
        if let Some(r) = self.start.as_mut().and_then(Pending::poll) {
            self.start = None;
            let cmd = self.life.started(r, now);
            self.exec(worker, cmd);
        }
        if let Some(r) = self.poll.as_mut().and_then(Pending::poll) {
            self.poll = None;
            let counting = self.life.counting(now);
            if let (Ok(sample), Some(snap)) = (&r, snapshot) {
                if self.life.is_running() {
                    self.progress.apply(snap, sample, counting);
                }
            }
            let cmd = self.life.polled(&r);
            self.exec(worker, cmd);
        }
        if let Some(r) = self.stop.as_mut().and_then(Pending::poll) {
            self.stop = None;
            if self.stop_finished(r) {
                worker.set_calibration_guard(false);
            }
        }
        if self.life.is_active()
            && (snapshot.is_none() || worker.connects() != self.connects_at_start)
            && !matches!(self.life.phase, Phase::Stopping(_))
        {
            self.life.connection_lost();
            self.start = None;
            self.poll = None;
        }
        self.after_reconnect(
            snapshot.is_some(),
            worker.connects(),
            worker.calibration_guard(),
        );
        let cmd = self.life.tick(now);
        self.exec(worker, cmd);
        if self.life.is_active() {
            ui.request_repaint_after(MIN_POLL_GAP);
        }
    }

    /// The stop job finished; true when it worked (the guard may go).
    fn stop_finished(&mut self, r: Result<(), Error>) -> bool {
        let ok = r.is_ok();
        self.life.stopped(r);
        if ok && self.life.outcome == Some(Outcome::Saved) {
            self.saved_this_session = true;
            self.saved_unrecorded = true;
        }
        ok
    }

    /// True once after a calibration was saved: the shell records the date
    /// in the data file ("Terakhir dikalibrasi").
    pub fn take_saved(&mut self) -> bool {
        std::mem::take(&mut self.saved_unrecorded)
    }

    /// Screenshot hook: a calibration in step 2 with `done` measured and
    /// `pressed` held down. Sends nothing.
    #[cfg(feature = "preview")]
    pub fn preview(&mut self, done: &[u8], pressed: &[u8]) {
        self.life.phase = Phase::Running {
            since: Instant::now() - SETTLE - Duration::from_secs(1),
        };
        self.life.poll_in_flight = true; // no polling from the preview
        self.progress = Progress::default();
        for &c in done {
            self.progress.done.insert(c);
            self.progress.peak.insert(c, 3380 + u16::from(c % 9) * 10);
        }
        for &c in pressed {
            self.progress.current.insert(c, 1800);
        }
    }

    fn exec<T: Transport + 'static>(&mut self, worker: &Worker<T>, cmd: Option<Cmd>) {
        match cmd {
            None => {}
            Some(Cmd::Start) => {
                // Set before the start goes out: whatever happens next, a
                // reconnect or shutdown stops calibration.
                worker.set_calibration_guard(true);
                self.connects_at_start = worker.connects();
                self.progress = Progress::default();
                self.start = Some(worker.submit("calibration-start", |kb| kb.start_calibration()));
            }
            Some(Cmd::Poll) => {
                self.poll = Some(worker.submit_quiet("calibration-poll", poll_job));
            }
            Some(Cmd::Stop) => {
                self.stop = Some(worker.submit("calibration-stop", |kb| kb.stop_calibration()));
            }
        }
    }

    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        page_with_right_panel(
            ui,
            "calibration-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    // ------------------------------------------------------------ centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let model = ctx.snapshot.map_or(Model::Win68HeMax, |s| s.model);
        let fallback;
        let matrix = match ctx.snapshot {
            Some(s) => &s.matrix,
            None => {
                fallback = reference_matrix(model);
                &fallback
            }
        };
        let total = matrix.len();
        let done = self.progress.done.len().min(total);
        let show_progress = self.life.is_active() || self.life.outcome.is_some();

        // The count only; the instructions are in the step cards below.
        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            if show_progress {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{done} / {total} {}", t::KEYS))
                            .font(font::bold(15.0))
                            .color(color::SUCCESS),
                    );
                });
            }
        });
        ui.add_space(10.0);
        progress_bar(
            ui,
            if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            },
        );
        ui.add_space(16.0);

        let now = Instant::now();
        let counting = self.life.counting(now);
        let progress = &self.progress;
        ui.vertical_centered(|ui| {
            // Selection is not used here: the view is display-only.
            KeyboardView::new(model, matrix)
                .skin(ctx.skin())
                .enabled(false)
                .id_salt("calibration-keyboard")
                .show(ui, &mut self.selection, |code| {
                    decorate(progress, code, show_progress, counting)
                });
        });
        ui.add_space(16.0);

        let step = self.active_step(now);
        let remaining = ctx
            .snapshot
            .map(|s| self.progress.remaining(s))
            .unwrap_or_default();
        let h = ui.available_height().max(150.0);
        ui.columns(3, |cols| {
            step_card(&mut cols[0], 1, step, t::STEP1, t::STEP1_BODY, h);
            let body2 = if show_progress && remaining.is_empty() {
                t::STEP2_ALL.to_string()
            } else if show_progress {
                format!(
                    "{} {} {}",
                    t::STEP2_BODY,
                    t::STEP2_LEFT,
                    legend_list(&remaining, 10)
                )
            } else {
                t::STEP2_BODY.to_string()
            };
            step_card(&mut cols[1], 2, step, t::STEP2, &body2, h);
            step_card(&mut cols[2], 3, step, t::STEP3, t::STEP3_BODY, h);
        });
    }

    /// 0: not started; 1..=3: the step in progress; 4: all done.
    fn active_step(&self, now: Instant) -> u8 {
        match &self.life.phase {
            Phase::Idle => match self.life.outcome {
                Some(Outcome::Saved) => 4,
                _ => 0,
            },
            Phase::Starting => 1,
            Phase::Running { .. } if !self.life.counting(now) => 1,
            Phase::Running { .. } => 2,
            Phase::Stopping(_) => 3,
        }
    }

    // ------------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let running = self.life.is_active();
        egui::TopBottomPanel::bottom("calibration-buttons")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                top: 12,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.buttons(ctx, ui));

        ui.label(
            RichText::new(if running {
                t::TITLE_RUNNING
            } else {
                t::TITLE_IDLE
            })
            .font(font::heading())
            .color(color::TEXT)
            .strong(),
        );
        card::hint(ui, t::NO_TYPING);
        ui.add_space(16.0);

        Card::new().padding(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            card::kv_row(ui, t::SWITCH, &switch_summary(ctx.snapshot));
            ui.add_space(6.0);
            let range = match self.progress.measured_range() {
                Some((lo, hi)) if lo == hi => format!("{} mm", mm_short(lo)),
                Some((lo, hi)) => format!("{} – {} mm", mm_short(lo), mm_short(hi)),
                None => "—".to_string(),
            };
            card::kv_row(ui, t::MEASURED, &range);
            ui.add_space(6.0);
            let last = match ctx.data.borrow().last_calibrated {
                Some(secs) => date_label(secs),
                None if self.saved_this_session => t::LAST_SESSION.to_string(),
                None => t::LAST_NEVER.to_string(),
            };
            card::kv_row(ui, t::LAST, &last);
        });
        ui.add_space(16.0);
        Card::new()
            .fill(color::ACCENT_SOFT)
            .stroke(color::ACCENT.gamma_multiply(0.5))
            .padding(14)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(t::REDO_WARNING).color(color::ACCENT)).wrap(),
                );
            });
        ui.add_space(16.0);
        self.outcome_box(ctx, ui);
    }

    fn outcome_box(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let Some(outcome) = self.life.outcome.clone() else {
            return;
        };
        match outcome {
            Outcome::Saved => {
                ui.label(RichText::new(format!("✔ {}", t::SAVED)).color(color::SUCCESS));
            }
            Outcome::Cancelled => card::hint(ui, t::CANCELLED),
            Outcome::Failed(msg, tech) => {
                card::error_box(ui, t::FAILED, &msg, None, Some(&tech));
            }
            Outcome::Interrupted => {
                let text = if ctx.worker.calibration_guard() {
                    t::INTERRUPTED
                } else {
                    t::INTERRUPTED_DONE
                };
                card::hint(ui, text);
            }
            Outcome::StopFailed(tech) => {
                // Not `can_write`: that stays false while the guard is set.
                let retry = (ctx.connected() && !ctx.busy).then_some(t::STOP_AGAIN);
                if card::error_box(ui, t::FAILED, t::STOP_FAILED, retry, Some(&tech)) {
                    let cmd = self.life.retry_stop();
                    self.exec(ctx.worker, cmd);
                }
            }
        }
    }

    fn buttons(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let total = ctx.snapshot.map_or(0, |s| s.matrix.len());
        let left = total.saturating_sub(self.progress.done.len());
        match &self.life.phase {
            Phase::Idle => {
                let label = if self.life.outcome.is_some() {
                    t::AGAIN
                } else {
                    t::START
                };
                if card::primary_button(ui, label, ctx.can_write()).clicked() {
                    let cmd = self.life.start();
                    self.exec(ctx.worker, cmd);
                }
            }
            Phase::Starting | Phase::Running { .. } => {
                let mut cancel = false;
                let mut save = false;
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    cancel = ui
                        .add_sized(
                            egui::vec2(w * 0.38, 46.0),
                            egui::Button::new(RichText::new(t::CANCEL).font(font::bold(15.0)))
                                .fill(color::BG)
                                .stroke(line(1.0, color::BORDER_STRONG))
                                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2)),
                        )
                        .clicked();
                    let label = if left == 0 {
                        t::SAVE.to_string()
                    } else {
                        format!("{} ({left} {})", t::SAVE, t::LEFT_SUFFIX)
                    };
                    let ready = self.life.is_running() && left == 0;
                    save = card::primary_button(ui, &label, ready).clicked();
                });
                if save || cancel {
                    let cmd = self.life.finish(save);
                    self.exec(ctx.worker, cmd);
                }
                if self.life.phase == Phase::Starting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        card::hint(ui, t::STARTING);
                    });
                }
            }
            Phase::Stopping(_) => {
                card::primary_button(ui, t::STOPPING, false);
            }
        }
    }
}

/// Key decoration: green with a check once calibrated, amber "tekan" while
/// an unmeasured key is held down.
fn decorate(p: &Progress, code: u8, active: bool, counting: bool) -> KeyDecoration {
    if !active {
        return KeyDecoration::default();
    }
    if p.done.contains(&code) {
        return KeyDecoration {
            label2: Some(Label2::colored("✔", color::SUCCESS)),
            fill: Some(color::SUCCESS.gamma_multiply(0.18)),
            stroke: Some(color::SUCCESS.gamma_multiply(0.6)),
            badge: None,
        };
    }
    let pressed = counting && p.current.get(&code).is_some_and(|&v| v >= PRESSED_UM);
    if pressed {
        return KeyDecoration {
            label2: Some(t::PRESS.into()),
            fill: Some(color::ACCENT.gamma_multiply(0.3)),
            stroke: Some(color::ACCENT),
            badge: None,
        };
    }
    KeyDecoration::default()
}

/// The switch shared by every key, or "Campuran".
fn switch_summary(snap: Option<&DeviceSnapshot>) -> String {
    let Some(s) = snap else {
        return "—".to_string();
    };
    let ids: BTreeSet<u16> = s.keys.values().map(|k| k.switch).collect();
    match ids.iter().next() {
        Some(&id) if ids.len() == 1 => s
            .model
            .switches()
            .iter()
            .find(|sw| sw.id == id)
            .map_or_else(|| format!("#{id}"), |sw| sw.name.to_string()),
        Some(_) => t::MIXED.to_string(),
        None => "—".to_string(),
    }
}

fn progress_bar(ui: &mut Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), color::TRACK);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    if fill.width() > 0.5 {
        p.rect_filled(fill, CornerRadius::same(4), color::SUCCESS);
    }
}

/// One of the three step cards. `active` is the step in progress (0 none,
/// 4 all done).
fn step_card(ui: &mut Ui, n: u8, active: u8, title: &str, body: &str, h: f32) {
    let done = active > n;
    let current = active == n;
    let (stroke, title_c, body_c) = if current {
        (color::ACCENT, color::TEXT, color::MUTED)
    } else if done || active == 0 {
        (color::BORDER, color::TEXT, color::MUTED)
    } else {
        (color::BORDER, color::FAINT, color::FAINT)
    };
    // `columns` gives justified layouts; plain left-aligned text here.
    let card = Card::new().stroke(stroke);
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        card.show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), (h - 42.0).max(80.0)));
            let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
            let (fill, text, c) = if done {
                (
                    color::SUCCESS.gamma_multiply(0.18),
                    "✔".to_string(),
                    color::SUCCESS,
                )
            } else if current {
                (
                    color::ACCENT.gamma_multiply(0.18),
                    n.to_string(),
                    color::ACCENT,
                )
            } else {
                (color::KEY_HOVER, n.to_string(), color::MUTED)
            };
            ui.painter().circle_filled(r.center(), 14.0, fill);
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font::bold(14.0),
                c,
            );
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(RichText::new(title).font(font::bold(15.0)).color(title_c)).wrap(),
            );
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(body).color(body_c)).wrap());
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::snapshot68;
    use openhe_device::MockTransport;

    fn frame_with(snap: &DeviceSnapshot, f: impl Fn(u8) -> u16) -> TravelFrame {
        let mut um = [[0u16; 21]; 6];
        for (r, c, code) in snap.matrix.keys() {
            um[usize::from(r)][usize::from(c)] = f(code);
        }
        TravelFrame { um }
    }

    fn sample(snap: &DeviceSnapshot, f: impl Fn(u8) -> u16) -> Sample {
        Sample {
            travel: frame_with(snap, &f),
        }
    }

    #[test]
    fn the_last_calibration_date_reads_day_month_year() {
        assert_eq!(date_label(0), "01-01-1970");
        assert_eq!(date_label(951_782_400), "29-02-2000");
        assert_eq!(date_label(1_793_145_600), "28-10-2026");
        assert_eq!(date_label(1_793_231_999), "28-10-2026");
    }

    #[test]
    fn a_saved_calibration_is_reported_once_for_the_data_file() {
        let mut page = Page::default();
        assert!(!page.take_saved());
        page.life.start();
        page.life.started(Ok(()), Instant::now());
        page.life.finish(true);
        page.stop_finished(Ok(()));
        assert!(page.take_saved());
        assert!(!page.take_saved(), "once");
        // Cancelled: nothing to record.
        page.life.start();
        page.life.started(Ok(()), Instant::now());
        page.life.finish(false);
        page.stop_finished(Ok(()));
        assert!(!page.take_saved());
    }

    #[test]
    fn threshold_is_max_minus_deadzones_minus_a_tenth() {
        // Wing Chun 3.4 mm, factory deadzones 0.2 / 0.2: 2.9 mm.
        assert_eq!(calibrated_threshold(3400, 200, 200), 2900);
        assert_eq!(calibrated_threshold(3300, 100, 0), 3100);
        // Never zero: a resting key never counts.
        assert_eq!(calibrated_threshold(100, 1000, 1000), 1);
    }

    #[test]
    fn a_key_counts_once_it_reaches_its_own_threshold() {
        let snap = snapshot68(); // switch 0 (3.4 mm), deadzones 0.2 / 0.2
        let mut p = Progress::default();
        // W at exactly the threshold, A just below it, the rest at rest.
        p.apply(
            &snap,
            &sample(&snap, |c| match c {
                0x1A => 2900,
                0x04 => 2899,
                _ => 0,
            }),
            true,
        );
        assert!(p.done.contains(&0x1A));
        assert!(!p.done.contains(&0x04));
        assert_eq!(p.done.len(), 1);
        // A frame with travel at a keyless position is ignored.
        let mut bad = sample(&snap, |_| 3400);
        bad.travel.um[0][0] = 3400; // row 0 has no keys
        let before = p.clone();
        p.apply(&snap, &bad, true);
        assert_eq!(p, before);
        assert_eq!(p.remaining(&snap).len(), snap.matrix.len() - 1);
        // Sticky: releasing W keeps it done.
        p.apply(&snap, &sample(&snap, |_| 0), true);
        assert!(p.done.contains(&0x1A));
        assert_eq!(p.peak[&0x1A], 2900);
        assert_eq!(p.measured_range(), Some((2900, 2900)));
    }

    #[test]
    fn deadzones_and_switch_move_the_threshold() {
        let mut snap = snapshot68();
        let k = snap.keys.get_mut(&0x04).unwrap();
        k.switch = 1; // Magnetic Jade Pro, 3.3 mm
        k.top_dz = openhe_proto::Micrometers(500);
        k.bottom_dz = openhe_proto::Micrometers(300);
        let mut p = Progress::default();
        p.apply(
            &snap,
            &sample(&snap, |c| if c == 0x04 { 2400 } else { 0 }),
            true,
        );
        assert!(p.done.contains(&0x04), "3300 - 500 - 300 - 100 = 2400");
    }

    #[test]
    fn samples_during_the_settle_time_do_not_count() {
        let snap = snapshot68();
        let mut p = Progress::default();
        p.apply(&snap, &sample(&snap, |_| 3400), false);
        assert!(p.done.is_empty());
        assert_eq!(p.current[&0x1A], 3400, "still shown live");
    }

    #[test]
    fn whole_keyboard_reaches_n_of_n() {
        let snap = snapshot68();
        let mut p = Progress::default();
        p.apply(&snap, &sample(&snap, |c| 3000 + u16::from(c)), true);
        assert_eq!(p.done.len(), snap.matrix.len());
        assert!(p.remaining(&snap).is_empty());
        let (lo, hi) = p.measured_range().unwrap();
        assert!(lo >= 3000 && hi > lo);
    }

    /// The peak of a press is held: a key is marked once any sample of the
    /// session reached its threshold, even if the next sample shows it
    /// released or only half pressed.
    #[test]
    fn a_quick_press_is_held_by_its_peak() {
        let snap = snapshot68();
        let mut p = Progress::default();
        for depth in [1500, 2950, 400, 0] {
            p.apply(
                &snap,
                &sample(&snap, |c| if c == 0x1A { depth } else { 0 }),
                true,
            );
        }
        assert_eq!(p.peak[&0x1A], 2950);
        assert!(p.done.contains(&0x1A));
        assert_eq!(p.current[&0x1A], 0, "released now");
        // Two half presses never add up.
        for depth in [2000, 0, 2100] {
            p.apply(
                &snap,
                &sample(&snap, |c| if c == 0x04 { depth } else { 0 }),
                true,
            );
        }
        assert!(!p.done.contains(&0x04));
    }

    #[test]
    fn mulai_resets_the_peaks() {
        use crate::app::test_support::offline_worker;
        let snap = snapshot68();
        let mut page = Page::default();
        page.progress.apply(&snap, &sample(&snap, |_| 3400), true);
        assert!(!page.progress.peak.is_empty() && !page.progress.done.is_empty());
        let w = offline_worker();
        let cmd = page.life.start();
        page.exec(&w, cmd);
        assert!(
            page.progress.peak.is_empty(),
            "peaks of the last session gone"
        );
        assert!(page.progress.done.is_empty());
        w.set_calibration_guard(false);
    }

    fn running(l: &mut Lifecycle, now: Instant) {
        assert_eq!(l.start(), Some(Cmd::Start));
        assert_eq!(l.started(Ok(()), now), None);
        assert!(l.is_running());
    }

    /// A quick press can bottom out between two samples, so polling runs
    /// as fast as the replies come back: one job in flight, the next as
    /// soon as it finished, but never sooner than 15 ms after the last one
    /// started.
    #[test]
    fn lifecycle_polls_one_at_a_time_as_fast_as_replies_allow() {
        let t0 = Instant::now();
        let mut l = Lifecycle::default();
        assert_eq!(l.tick(t0), None, "idle: no polling");
        running(&mut l, t0);
        assert_eq!(l.tick(t0), Some(Cmd::Poll));
        assert_eq!(l.tick(t0 + Duration::from_secs(1)), None, "one in flight");
        assert_eq!(l.polled::<()>(&Ok(())), None);
        assert_eq!(l.tick(t0 + Duration::from_millis(10)), None, "minimum gap");
        assert_eq!(l.tick(t0 + Duration::from_millis(15)), Some(Cmd::Poll));
        // A slow reply (40 ms): the next poll goes out right away.
        assert_eq!(l.polled::<()>(&Ok(())), None);
        assert_eq!(l.tick(t0 + Duration::from_millis(55)), Some(Cmd::Poll));
        assert!(!l.counting(t0 + Duration::from_millis(1999)));
        assert!(l.counting(t0 + SETTLE));
    }

    #[test]
    fn save_and_cancel_both_stop() {
        for save in [true, false] {
            let mut l = Lifecycle::default();
            running(&mut l, Instant::now());
            assert_eq!(l.finish(save), Some(Cmd::Stop));
            assert!(l.is_active(), "stopping until the stop returns");
            assert_eq!(l.finish(save), None, "one stop");
            l.stopped(Ok(()));
            assert!(!l.is_active());
            let want = if save {
                Outcome::Saved
            } else {
                Outcome::Cancelled
            };
            assert_eq!(l.outcome, Some(want));
        }
    }

    #[test]
    fn leaving_the_page_always_stops() {
        let t0 = Instant::now();
        // While running.
        let mut l = Lifecycle::default();
        running(&mut l, t0);
        assert_eq!(l.leave(), Some(Cmd::Stop));
        l.stopped(Ok(()));
        assert_eq!(l.outcome, None);
        // While the start is still on its way: the stop is queued behind
        // it, and the late start result changes nothing.
        let mut l = Lifecycle::default();
        l.start();
        assert_eq!(l.leave(), Some(Cmd::Stop));
        assert_eq!(l.started(Ok(()), t0), None);
        assert!(matches!(l.phase, Phase::Stopping(StopReason::Left)));
        // Idle: nothing to stop.
        let mut l = Lifecycle::default();
        assert_eq!(l.leave(), None);
    }

    #[test]
    fn errors_stop_calibration() {
        // A poll that does not parse (RM6X21 framing unverified).
        let mut l = Lifecycle::default();
        running(&mut l, Instant::now());
        l.tick(Instant::now());
        let cmd = l.polled::<Sample>(&Err(Error::Io("RM6X21 bad".into())));
        assert_eq!(cmd, Some(Cmd::Stop));
        l.stopped(Ok(()));
        match &l.outcome {
            Some(Outcome::Failed(msg, tech)) => {
                assert!(msg.contains(t::FRAME_HINT), "{msg}");
                assert!(tech.contains("RM6X21"));
            }
            other => panic!("{other:?}"),
        }
        // A timeout.
        let mut l = Lifecycle::default();
        running(&mut l, Instant::now());
        l.tick(Instant::now());
        assert_eq!(l.polled::<Sample>(&Err(Error::Timeout)), Some(Cmd::Stop));
        // A start that failed: the keyboard may be calibrating anyway.
        let mut l = Lifecycle::default();
        l.start();
        assert_eq!(
            l.started(Err(Error::Timeout), Instant::now()),
            Some(Cmd::Stop)
        );
    }

    #[test]
    fn a_lost_connection_leaves_the_stop_to_the_worker_guard() {
        let mut l = Lifecycle::default();
        running(&mut l, Instant::now());
        l.tick(Instant::now());
        assert_eq!(l.polled::<Sample>(&Err(Error::Disconnected)), None);
        assert_eq!(l.outcome, Some(Outcome::Interrupted));
        assert!(!l.is_active());
        // Or noticed by the page (reconnect between frames).
        let mut l = Lifecycle::default();
        running(&mut l, Instant::now());
        l.connection_lost();
        assert_eq!(l.outcome, Some(Outcome::Interrupted));
        assert_eq!(l.tick(Instant::now() + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_failed_stop_is_reported_and_can_be_retried() {
        let mut l = Lifecycle::default();
        running(&mut l, Instant::now());
        l.finish(false);
        l.stopped(Err(Error::Timeout));
        assert!(matches!(l.outcome, Some(Outcome::StopFailed(_))));
        assert_eq!(l.retry_stop(), Some(Cmd::Stop));
        l.stopped(Ok(()));
        assert_eq!(l.outcome, Some(Outcome::Cancelled));
        assert_eq!(l.retry_stop(), None);
    }

    /// An RM6X21 reply (3 reports) carrying `value` for every cell.
    fn rm_reply(sub: u8, value: u16) -> [[u8; 64]; 3] {
        let mut all = [0u8; 192];
        all[..4].copy_from_slice(&[0x5C, 0x3C, 0x92, 0x00]);
        all[5] = sub;
        for i in 0..63 {
            let at = 6 + 2 * i;
            all[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        let mut out = [[0u8; 64]; 3];
        for (i, rep) in out.iter_mut().enumerate() {
            rep.copy_from_slice(&all[i * 64..(i + 1) * 64]);
        }
        out
    }

    #[test]
    fn poll_job_reads_travel_only() {
        let mut m = MockTransport::new();
        for (sub, v) in [(2, 1500), (2, 1500)] {
            for r in rm_reply(sub, v) {
                m.push_reply(r);
            }
        }
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        let s = poll_job(&mut kb).unwrap();
        assert_eq!(s.travel.um[3][5], 1500);
        let w = kb.session_mut().transport().written().to_vec();
        let subs: Vec<(u8, u8)> = w.iter().map(|f| (f[4], f[5])).collect();
        assert_eq!(subs, vec![(2, 1), (2, 2)]);
    }

    #[test]
    fn a_garbled_frame_is_an_error_not_a_panic() {
        let mut m = MockTransport::new();
        let mut bad = rm_reply(3, 0); // SUB 3 is not travel
        bad[0][5] = 3;
        for r in bad {
            m.push_reply(r);
        }
        let mut kb = Keyboard::new(m, Model::Win68HeMax);
        assert!(poll_job(&mut kb).is_err());
    }

    /// A connected mock worker whose writes are logged; the keyboard
    /// answers the snapshot load and then one `stop_calibration`.
    type WriteLog = std::sync::Arc<std::sync::Mutex<Vec<[u8; 64]>>>;

    fn logged_worker() -> (Worker<crate::worker::tests::Probe>, WriteLog) {
        use crate::worker::tests::{order_reply, push_snapshot, Probe};
        use crate::worker::WorkerConfig;
        let log: WriteLog = Default::default();
        let l = log.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                let mut m = MockTransport::new();
                push_snapshot(&mut m, 0);
                m.push_reply(order_reply(0x0D, 1));
                Ok(Keyboard::new(
                    Probe::logging(m, l.clone()),
                    Model::Win68HeMax,
                ))
            }),
            WorkerConfig {
                retry_interval: Duration::from_millis(5),
                idle_poll: Duration::from_millis(2),
                state_poll: Duration::from_secs(3600),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.is_connected() {
            assert!(Instant::now() < deadline, "never connected");
            std::thread::sleep(Duration::from_millis(1));
        }
        (w, log)
    }

    #[test]
    fn app_exit_stops_calibration_behind_a_slow_poll() {
        use crate::worker::tests::wrote_stop;
        let (w, log) = logged_worker();
        let mut page = Page::default();
        page.life.start();
        page.life.started(Ok(()), Instant::now());
        w.set_calibration_guard(true);
        // A poll that takes long (RM6X21 retries) is in flight on exit.
        let _poll = w.submit_quiet("calibration-poll", |_kb| {
            std::thread::sleep(Duration::from_millis(800));
            Ok(())
        });
        let stop = page.exit_stop(&w).expect("guard set: a stop is sent");
        assert!(wait_for_stop(&w, stop, EXIT_STOP_WAIT));
        assert!(wrote_stop(&log));
        assert!(!w.calibration_guard(), "cleared once the stop worked");
    }

    #[test]
    fn app_exit_after_a_failed_stop_still_stops() {
        use crate::worker::tests::wrote_stop;
        let (w, log) = logged_worker();
        let mut page = Page::default();
        page.life.start();
        page.life.started(Ok(()), Instant::now());
        page.life.finish(false);
        page.life.stopped(Err(Error::Timeout));
        assert!(matches!(page.life.outcome, Some(Outcome::StopFailed(_))));
        assert!(!page.is_active(), "phase is Idle after a failed stop");
        w.set_calibration_guard(true); // still set after the failed stop
        let stop = page.exit_stop(&w).expect("a stop is sent anyway");
        assert!(wait_for_stop(&w, stop, EXIT_STOP_WAIT));
        assert!(wrote_stop(&log));
    }

    #[test]
    fn app_exit_without_calibration_sends_nothing() {
        let (w, log) = logged_worker();
        let mut page = Page::default();
        assert!(page.exit_stop(&w).is_none());
        assert!(!crate::worker::tests::wrote_stop(&log));
    }

    #[test]
    fn a_reconnect_whose_stop_failed_is_a_failed_stop_not_an_interruption() {
        let mut page = Page::default();
        page.life.start();
        page.connects_at_start = 1;
        page.life.started(Ok(()), Instant::now());
        page.life.connection_lost();
        assert_eq!(page.life.outcome, Some(Outcome::Interrupted));
        // Still disconnected, or reconnected with the stop done: as is.
        page.after_reconnect(false, 1, true);
        page.after_reconnect(true, 2, false);
        assert_eq!(page.life.outcome, Some(Outcome::Interrupted));
        // Reconnected, but the worker's stop did not work (guard still set).
        page.after_reconnect(true, 2, true);
        assert!(
            matches!(page.life.outcome, Some(Outcome::StopFailed(_))),
            "{:?}",
            page.life.outcome
        );
        // ... which offers "Hentikan lagi".
        assert_eq!(page.life.retry_stop(), Some(Cmd::Stop));
    }

    #[test]
    fn a_stop_owed_from_any_page_clears_the_guard() {
        use crate::app::test_support::offline_worker;
        let (w, log) = logged_worker();
        let ctx = egui::Context::default();
        let snap = snapshot68();
        let mut page = Page::default();
        // The guard is set but nothing runs (e.g. a stop that failed after
        // a reconnect): the shell's "Hentikan kalibrasi" sends a stop.
        w.set_calibration_guard(true);
        assert!(page.stop_owed(&w));
        assert!(page.is_active(), "stopping");
        assert!(!page.stop_owed(&w), "one stop at a time");
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.is_active() {
            assert!(Instant::now() < deadline);
            page.pump(&w, Some(&snap), &ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(crate::worker::tests::wrote_stop(&log));
        assert!(!w.calibration_guard(), "cleared once the stop worked");
        // Without a guard there is nothing to stop.
        let off = offline_worker();
        assert!(!Page::default().stop_owed(&off));
    }

    #[test]
    fn the_page_stops_calibration_when_left_and_on_lost_connection() {
        use crate::app::test_support::offline_worker;
        let w = offline_worker();
        let ctx = egui::Context::default();
        let snap = snapshot68();
        let mut page = Page::default();
        let cmd = page.life.start();
        page.exec(&w, cmd);
        assert!(w.calibration_guard(), "guard set before the start goes out");
        // The offline worker fails the start with Disconnected.
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.is_active() {
            assert!(Instant::now() < deadline);
            page.pump(&w, Some(&snap), &ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(page.life.outcome, Some(Outcome::Interrupted));
        assert!(w.calibration_guard(), "the worker stops it on reconnect");

        // Leaving while running sends a stop.
        let mut page = Page::default();
        page.life.start();
        page.life.started(Ok(()), Instant::now());
        page.leave(&w);
        assert!(page.stop.is_some());
        assert!(matches!(page.life.phase, Phase::Stopping(StopReason::Left)));
    }
}

//! Pengaturan (mockup `docs/design/mockups/07-pengaturan.png`): polling
//! rate, Win/Mac, the four onboard profiles (switch, local names,
//! import/export to JSON), per-app automatic profile switching, launch at
//! login, device info and factory reset.
//!
//! Also home of the profile and Win/Mac switch jobs, which the top bar
//! shares, and of the restart flow ([`Restart`], [`RestartRun`]) for writes
//! after which the keyboard re-enumerates (polling rate, factory reset, and
//! a profile switch: PROTOCOL 7.x finding 23): "Menyambung ulang…", never an
//! error, while the worker reconnects, then a check of what the reconnected
//! keyboard reports. Win/Mac is stored per profile, so after a profile
//! switch the mode comes from the reconnected keyboard's snapshot.

pub mod os;
pub mod profile_file;

use crate::app::{firmware_label, page_with_right_panel, AppCtx};
use crate::i18n;
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::legend_list;
use crate::widgets::segmented::Segmented;
use crate::widgets::toggle::toggle;
use crate::worker::{Debounce, DeviceSnapshot, Pending, WorkerHandle};
use eframe::egui::{self, Align, Align2, CornerRadius, Layout, Rect, RichText, Sense, Ui};
use openhe_device::{Error, Keyboard, Transport, WriteOutcome};
use openhe_proto::{PollingRate, SystemMode};
use os::{launch_at_login_enabled, set_launch_at_login, ForegroundWatcher};
use profile_file::{
    export_job, import_job, plan_import, Exported, FileError, ImportPlan, ImportReport, ImportStep,
    ProfileData, ProfileFile,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------- switch jobs

/// What a Win/Mac switch job did: the write result, and the mode read
/// back afterwards (also after a failed write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOutcome {
    pub write: Result<(), Error>,
    /// The mode read back; `None` when the re-read failed.
    pub system_mode: Option<Option<SystemMode>>,
}

/// Switches the onboard profile (0..=3) with one attempt. The keyboard
/// re-enumerates afterwards, so this is a restart write: run it through
/// [`RestartRun`] with [`RestartKind::Profile`], which reconnects and checks
/// the profile of the reconnected keyboard.
pub fn profile_job<T: Transport>(kb: &mut Keyboard<T>, idx: u8) -> Result<WriteOutcome, Error> {
    kb.set_profile(idx)
}

/// Switches Win/Mac mode, then reads it back.
pub fn system_mode_job<T: Transport>(
    kb: &mut Keyboard<T>,
    mode: SystemMode,
) -> Result<SwitchOutcome, Error> {
    let write = kb.set_system_mode(mode);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let system_mode = match kb.system_mode() {
        Ok(m) => Some(m),
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(_) => None,
    };
    Ok(SwitchOutcome { write, system_mode })
}

/// What the UI does with a finished Win/Mac switch job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchEffects {
    /// Patch the snapshot's Win/Mac mode with this (read back).
    pub system_mode: Option<Option<SystemMode>>,
    /// A mode switch may change the layout: re-read all.
    pub request_reload: bool,
    /// Indonesian error text.
    pub error: Option<String>,
    pub technical: Option<String>,
}

/// Turns a mode switch job result into UI effects. Every finished job
/// reloads the snapshot (a new mode may change the layout), except a
/// disconnect, after which the worker reloads anyway.
pub fn switch_effects(result: Result<SwitchOutcome, Error>, failed: &str) -> SwitchEffects {
    match result {
        Ok(out) => {
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => (
                    Some(format!("{failed} {}", i18n::device_error(e))),
                    Some(i18n::technical(e)),
                ),
            };
            SwitchEffects {
                system_mode: out.system_mode,
                request_reload: true,
                error,
                technical,
            }
        }
        Err(e) => SwitchEffects {
            system_mode: None,
            request_reload: e != Error::Disconnected,
            error: Some(format!("{failed} {}", i18n::device_error(&e))),
            technical: Some(i18n::technical(&e)),
        },
    }
}

/// Applies [`SwitchEffects`] to the worker; returns the error text.
pub fn apply_switch_effects(
    worker: &crate::worker::Worker,
    fx: SwitchEffects,
) -> Option<(String, Option<String>)> {
    if let Some(m) = fx.system_mode {
        worker.patch_snapshot(move |s| s.system_mode = m);
    }
    if fx.request_reload {
        worker.request_reload();
    }
    fx.error.map(|e| (e, fx.technical))
}

// ------------------------------------------------------ restart flows

/// Pause before reconnecting after a write that restarts the keyboard.
pub const RECONNECT_PAUSE: Duration = Duration::from_millis(1500);
/// Give up waiting for the keyboard to come back after this long.
pub const RECONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A write after which the keyboard re-enumerates on USB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartKind {
    Polling(PollingRate),
    FactoryReset,
    /// A switch to this profile (0..=3).
    Profile(u8),
}

/// The polling-rate / factory-reset / profile-switch flow (Review Focus 4): the write, then
/// "Menyambung ulang…" (never an error) while the worker reconnects, then a
/// check of what the keyboard reports.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Restart {
    #[default]
    Idle,
    /// The write is queued or running; `connects` is
    /// [`Worker::connects`] when it was sent.
    Sending {
        kind: RestartKind,
        connects: u64,
    },
    Reconnecting {
        kind: RestartKind,
        connects: u64,
        since: Instant,
    },
    Done(String),
    Failed(String, Option<String>),
}

/// What the page must do after [`Restart::job_done`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartAction {
    None,
    /// Ask the worker to drop the keyboard and reconnect after a pause.
    Reconnect,
    /// The write failed outright: re-read the state.
    Reload,
}

impl Restart {
    pub fn is_busy(&self) -> bool {
        matches!(self, Restart::Sending { .. } | Restart::Reconnecting { .. })
    }

    /// The write was sent.
    pub fn begin(kind: RestartKind, connects: u64) -> Restart {
        Restart::Sending { kind, connects }
    }

    /// The write job finished. `Done` and `Reenumerating` mean "the keyboard
    /// restarts": wait for it to come back. `Disconnected` means the frame
    /// was never written (`set_profile`, `set_polling_rate` and
    /// `factory_reset` all report a disconnect after the write as
    /// `Reenumerating`), so it is a failed write: waiting for a restart
    /// would end in a confusing "reports 8000 Hz" or "Profil 1".
    pub fn job_done(&mut self, r: Result<WriteOutcome, Error>, now: Instant) -> RestartAction {
        let Restart::Sending { kind, connects } = *self else {
            return RestartAction::None;
        };
        match r {
            Ok(_) => {
                *self = Restart::Reconnecting {
                    kind,
                    connects,
                    since: now,
                };
                RestartAction::Reconnect
            }
            // A write that never went out; the keyboard is gone and the
            // worker reloads when it is back.
            Err(Error::Disconnected) => {
                *self = Restart::Failed(
                    format!(
                        "{} {}",
                        t::WRITE_FAILED,
                        i18n::device_error(&Error::Disconnected)
                    ),
                    Some(i18n::technical(&Error::Disconnected)),
                );
                RestartAction::None
            }
            Err(e) => {
                *self = Restart::Failed(
                    format!("{} {}", t::WRITE_FAILED, i18n::device_error(&e)),
                    Some(i18n::technical(&e)),
                );
                RestartAction::Reload
            }
        }
    }

    /// Watches the worker while reconnecting: done once a new connection
    /// has loaded a snapshot; the polling rate is checked against it.
    pub fn observe(&mut self, connects: u64, snapshot: Option<&DeviceSnapshot>, now: Instant) {
        let Restart::Reconnecting {
            kind,
            connects: before,
            since,
        } = *self
        else {
            return;
        };
        match snapshot {
            Some(s) if connects > before => {
                *self = match kind {
                    RestartKind::Polling(r) if s.polling == r => {
                        Restart::Done(format!("{} {} Hz.", t::POLLING_DONE, r.hz()))
                    }
                    RestartKind::Polling(_) => Restart::Failed(
                        format!("{} {} Hz.", t::POLLING_MISMATCH, s.polling.hz()),
                        None,
                    ),
                    RestartKind::FactoryReset => Restart::Done(t::RESET_DONE.to_string()),
                    // The new connection read the profile (and the Win/Mac
                    // mode, which is stored per profile) from the keyboard.
                    RestartKind::Profile(p) if s.profile == p => {
                        Restart::Done(format!("{} {}.", t::PROFILE_DONE, p + 1))
                    }
                    RestartKind::Profile(_) => {
                        Restart::Failed(format!("{} {}.", t::PROFILE_MISMATCH, s.profile + 1), None)
                    }
                };
            }
            _ if now.duration_since(since) >= RECONNECT_TIMEOUT => {
                *self = Restart::Failed(t::NOT_BACK.to_string(), None);
            }
            _ => {}
        }
    }
}

/// What a restart flow needs from the worker: [`crate::worker::Worker`]
/// and [`WorkerHandle`] in the app. The reconnect itself is scheduled by
/// the worker thread ([`crate::worker::Worker::submit_restart`]); the flow
/// only observes it.
pub trait RestartPort {
    /// [`crate::worker::Worker::connects`].
    fn connects(&self) -> u64;
    fn request_reload(&self);
}

impl<T: Transport + 'static> RestartPort for crate::worker::Worker<T> {
    fn connects(&self) -> u64 {
        crate::worker::Worker::connects(self)
    }
    fn request_reload(&self) {
        crate::worker::Worker::request_reload(self);
    }
}

impl<T: Transport + 'static> RestartPort for WorkerHandle<T> {
    fn connects(&self) -> u64 {
        WorkerHandle::<T>::connects(self)
    }
    fn request_reload(&self) {
        WorkerHandle::request_reload(self);
    }
}

/// One restart flow with its write job: the state machine ([`Restart`])
/// plus the plumbing every caller needs (the settings page, the top bar,
/// the per-app switcher).
#[derive(Default)]
pub struct RestartRun {
    pub state: Restart,
    job: Option<Pending<WriteOutcome>>,
}

impl RestartRun {
    /// The write `job` was submitted for `kind` with
    /// [`crate::worker::Worker::submit_restart`] (which reconnects on its
    /// own); `connects` is the worker's count at that moment.
    pub fn start(&mut self, kind: RestartKind, connects: u64, job: Pending<WriteOutcome>) {
        self.state = Restart::begin(kind, connects);
        self.job = Some(job);
    }

    pub fn is_busy(&self) -> bool {
        self.state.is_busy()
    }

    /// Collects the write job (asking for a reload after a failed write;
    /// the worker already scheduled the reconnect itself) and follows the
    /// reconnect. Never calls `reconnect`: a late call could drop the
    /// fresh connection.
    pub fn pump(
        &mut self,
        port: &impl RestartPort,
        snapshot: Option<&DeviceSnapshot>,
        now: Instant,
    ) {
        if let Some(r) = self.job.as_mut().and_then(Pending::poll) {
            self.job = None;
            if self.state.job_done(r, now) == RestartAction::Reload {
                port.request_reload();
            }
        }
        self.state.observe(port.connects(), snapshot, now);
    }
}

/// Sets the polling rate (one attempt; see `Keyboard::set_polling_rate`).
pub fn polling_job<T: Transport>(
    kb: &mut Keyboard<T>,
    rate: PollingRate,
) -> Result<WriteOutcome, Error> {
    kb.set_polling_rate(rate)
}

/// Factory reset (one attempt). The board restarts, so no reply or a
/// disconnect after the write means "restarting" (see
/// `Keyboard::factory_reset`).
pub fn reset_job<T: Transport>(kb: &mut Keyboard<T>) -> Result<WriteOutcome, Error> {
    kb.factory_reset()
}

/// The typed confirmation for a factory reset: exactly "RESET" (no
/// trimming, no other case).
pub fn reset_confirmed(typed: &str) -> bool {
    typed == RESET_WORD
}

pub const RESET_WORD: &str = "RESET";

// ------------------------------------------------- per-app switching

/// The profile for `app` from `rules`: the first rule whose app name
/// matches (trimmed, case-insensitive). Empty names and profiles outside
/// 0..=3 never match.
pub fn match_rule(app: &str, rules: &[(String, u8)]) -> Option<u8> {
    let app = app.trim();
    if app.is_empty() {
        return None;
    }
    rules
        .iter()
        .find(|(name, p)| {
            *p <= 3 && !name.trim().is_empty() && name.trim().eq_ignore_ascii_case(app)
        })
        .map(|(_, p)| *p)
}

/// Decides when the foreground app should switch the profile: only when a
/// rule matches and the keyboard is on another profile, and at most once
/// per (app, profile) while that app stays in front, so a failing or
/// overridden switch is not retried in a loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoSwitch {
    attempted: Option<(String, u8)>,
}

impl AutoSwitch {
    pub fn decide(&mut self, app: Option<&str>, rules: &[(String, u8)], current: u8) -> Option<u8> {
        let Some(app) = app else {
            self.attempted = None;
            return None;
        };
        let Some(target) = match_rule(app, rules) else {
            self.attempted = None;
            return None;
        };
        let key = (app.trim().to_lowercase(), target);
        if target == current || self.attempted.as_ref() == Some(&key) {
            self.attempted = Some(key);
            return None;
        }
        self.attempted = Some(key);
        Some(target)
    }
}

/// What the per-app switcher needs from the worker. [`WorkerHandle`] in
/// the app; a fake in tests.
pub trait SwitchPort: RestartPort {
    /// The active profile while connected.
    fn profile(&self) -> Option<u8>;
    /// The snapshot while connected (read while a switch reconnects).
    fn snapshot(&self) -> Option<DeviceSnapshot>;
    /// The worker is busy (a job queued or running, or loading), restarting
    /// (a reconnect after a re-enumerating write) or may be calibrating.
    fn locked(&self) -> bool;
    /// Submits a profile switch ([`profile_job`]).
    fn switch(&self, profile: u8) -> Pending<WriteOutcome>;
}

impl<T: Transport + 'static> SwitchPort for WorkerHandle<T> {
    fn profile(&self) -> Option<u8> {
        WorkerHandle::profile(self)
    }

    fn snapshot(&self) -> Option<DeviceSnapshot> {
        WorkerHandle::<T>::snapshot(self)
    }

    fn locked(&self) -> bool {
        self.is_busy() || self.is_restarting() || self.calibration_guard()
    }

    fn switch(&self, profile: u8) -> Pending<WriteOutcome> {
        self.submit_restart("auto-profile", RECONNECT_PAUSE, move |kb| {
            profile_job(kb, profile)
        })
    }
}

/// State shared by the Pengaturan page (UI thread) and the per-app switcher
/// (watcher thread).
#[derive(Debug, Default)]
pub struct AutoShared {
    rules: Mutex<Vec<(String, u8)>>,
    /// The rules changed: forget which switch was already tried.
    rules_changed: AtomicBool,
    /// The UI holds switches back: a restart flow, an import planned or
    /// running, a profile or mode switch of the UI. (Busy jobs and the
    /// calibration guard are read from the worker directly.)
    pub ui_lock: AtomicBool,
    /// A failed automatic switch, for the shell to show.
    error: Mutex<Option<String>>,
    /// An automatic switch is running (write or reconnect).
    reconnecting: AtomicBool,
}

impl AutoShared {
    pub fn set_rules(&self, rules: Vec<(String, u8)>) {
        *self.rules.lock().unwrap_or_else(|p| p.into_inner()) = rules;
        self.rules_changed.store(true, Ordering::SeqCst);
    }

    /// A per-app switch is waiting for the keyboard to reconnect (the
    /// shell shows "Menyambung ulang…").
    pub fn reconnecting(&self) -> bool {
        self.reconnecting.load(Ordering::SeqCst)
    }

    pub fn set_ui_lock(&self, on: bool) {
        self.ui_lock.store(on, Ordering::SeqCst);
    }

    /// The last failed automatic switch (Indonesian), once.
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).take()
    }

    fn rules(&self) -> Vec<(String, u8)> {
        self.rules.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// The per-app switcher. It runs on the foreground-watcher thread, so it
/// works while the window is hidden or minimised (when the UI may not run
/// any frames): it decides, submits the switch through a [`SwitchPort`],
/// and collects and applies the result itself.
#[derive(Default)]
pub struct AutoSwitcher {
    decide: AutoSwitch,
    /// The running switch: write, reconnect, check of the new profile.
    run: RestartRun,
}

impl AutoSwitcher {
    /// One watcher tick with the app in front (`None`: OpenHE itself or
    /// unknown). Follows a running switch through the keyboard's
    /// reconnect, then starts one when a rule asks for it and nothing holds
    /// switches back (never while a switch reconnects). True when a switch
    /// finished (the UI may want to repaint).
    pub fn tick(&mut self, app: Option<&str>, port: &impl SwitchPort, shared: &AutoShared) -> bool {
        self.tick_at(app, port, shared, Instant::now())
    }

    fn tick_at(
        &mut self,
        app: Option<&str>,
        port: &impl SwitchPort,
        shared: &AutoShared,
        now: Instant,
    ) -> bool {
        if shared.rules_changed.swap(false, Ordering::SeqCst) {
            self.decide = AutoSwitch::default();
        }
        let mut finished = false;
        if self.run.is_busy() {
            let snap = port.snapshot();
            self.run.pump(port, snap.as_ref(), now);
            match std::mem::take(&mut self.run.state) {
                Restart::Done(_) => finished = true,
                Restart::Failed(msg, _) => {
                    finished = true;
                    let e = format!("{} {msg}", i18n::events::AUTO_SWITCH_FAILED);
                    *shared.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
                }
                other => self.run.state = other,
            }
        }
        shared
            .reconnecting
            .store(self.run.is_busy(), Ordering::SeqCst);
        if self.run.is_busy() || shared.ui_lock.load(Ordering::SeqCst) || port.locked() {
            return finished;
        }
        let Some(current) = port.profile() else {
            return finished;
        };
        if let Some(p) = self.decide.decide(app, &shared.rules(), current) {
            let connects = port.connects();
            self.run
                .start(RestartKind::Profile(p), connects, port.switch(p));
            shared.reconnecting.store(true, Ordering::SeqCst);
        }
        finished
    }
}

// --------------------------------------------------------------- page

mod strings {
    pub const KEYBOARD: &str = "Keyboard";
    pub const POLLING: &str = "Polling rate";
    pub const POLLING_HINT: &str = "Makin tinggi makin responsif, tapi makin berat untuk CPU";
    pub const POLLING_RESTART: &str = "Keyboard tersambung ulang sebentar setelah diganti.";
    pub const SYSTEM: &str = "Mode sistem";
    pub const SYSTEM_HINT: &str = "Menukar posisi Win/Alt dan tombol media sesuai OS";
    pub const WIN_LINUX: &str = "Windows / Linux";
    pub const MACOS: &str = "macOS";
    pub const PROFILES: &str = "Profil di keyboard";
    pub const PROFILE: &str = "Profil";
    pub const ACTIVE: &str = "aktif";
    pub const UNNAMED: &str = "Tanpa nama";
    pub const NAME_HINT: &str =
        "Nama disimpan di komputer ini. Klik kartu untuk mengaktifkan profil.";
    pub const IMPORT: &str = "Impor…";
    pub const EXPORT: &str = "Ekspor…";
    pub const EXPORTING: &str = "Membaca profil dari keyboard…";
    pub const EXPORTED: &str = "Profil diekspor ke";
    pub const EXPORTED_NO_LIGHT: &str = "Lampu tidak terbaca, jadi tidak ikut diekspor.";
    pub const EXPORT_FAILED: &str = "Gagal mengekspor profil.";
    pub const WRITE_FILE_FAILED: &str = "File tidak bisa disimpan.";
    pub const READ_FILE_FAILED: &str = "File tidak bisa dibaca.";
    pub const IMPORT_TITLE: &str = "Impor ke profil";
    pub const IMPORT_KEYS: &str = "tombol";
    pub const IMPORT_FUNCS: &str = "fungsi lanjutan";
    pub const IMPORT_LIGHT: &str = "lampu";
    pub const IMPORT_OVERWRITE: &str = "Pengaturan profil ini di keyboard akan ditimpa.";
    pub const IMPORT_CHANGED: &str =
        "Keyboard berubah sejak file dibaca, jadi rencana impor dibuat ulang. Periksa lagi, lalu tekan Terapkan.";
    pub const IMPORT_MACROS: &str = "Isi macro tidak ikut diimpor; tombol macro dilewati:";
    pub const IMPORT_APPLY: &str = "Terapkan";
    pub const CANCEL: &str = "Batal";
    pub const IMPORTING: &str = "Mengimpor ke keyboard…";
    pub const IMPORTED: &str = "Profil diimpor dan dibaca ulang dari keyboard.";
    pub const IMPORT_PARTIAL: &str = "Sebagian impor gagal; nilai sudah dibaca ulang:";
    pub const IMPORT_DISCONNECTED: &str =
        "Keyboard terputus saat impor. Sebagian pengaturan mungkin sudah tertulis; nilai dibaca ulang saat tersambung.";
    pub const PART_PERF: &str = "performa";
    pub const PART_ADV: &str = "fungsi lanjutan";
    pub const PART_LAYERS: &str = "remap";
    pub const PART_LIGHT: &str = "lampu";
    pub const PART_COLORS: &str = "warna per tombol";
    pub const FILE_NOT_JSON: &str = "File ini bukan JSON yang valid.";
    pub const FILE_NOT_PROFILE: &str = "File ini bukan profil OpenHE.";
    pub const FILE_NEWER: &str = "File ini dibuat oleh OpenHE yang lebih baru.";
    pub const FILE_MODEL: &str = "Model keyboard di file tidak dikenal.";
    pub const FILE_WRONG_MODEL: &str = "Profil ini untuk keyboard lain:";
    pub const FILE_BAD_VALUE: &str = "File berisi nilai yang tidak valid.";
    pub const FILE_TRANSPARENT_BASE: &str = "File memberi kode 0x0001 di layer Base untuk tombol";
    pub const FILE_TRANSPARENT_BASE_TAIL: &str =
        "— kode itu \"Ikut Base\" (transparan), bukan Fn, dan di Base membuat tombol mati. Profil tidak diimpor.";

    pub const APP: &str = "Aplikasi";
    pub const AUTO: &str = "Ganti profil otomatis per aplikasi";
    pub const AUTO_HINT: &str = "Contoh: buka Valorant → Profil 2. Butuh OpenHE tetap berjalan.";
    pub const AUTO_FRONT: &str = "Aplikasi di depan:";
    pub const AUTO_NONE: &str = "belum terbaca";
    pub const AUTO_ADD: &str = "Tambah";
    pub const AUTO_APP_HINT: &str = "Nama aplikasi";
    pub const AUTO_REMOVE: &str = "Hapus";
    pub const AUTO_NO_RULES: &str = "Belum ada aturan. Tambahkan aplikasi dan profilnya.";
    pub const AUTO_FAILED: &str = "Pemantau aplikasi tidak bisa dijalankan.";
    pub const AUTO_SESSION: &str =
        "Aturan dan sakelar ini disimpan di komputer ini. Pergantian hanya terjadi selama OpenHE berjalan.";
    pub const LOGIN: &str = "Jalankan saat komputer menyala";
    pub const LOGIN_FAILED: &str = "Pengaturan jalan otomatis gagal diubah.";
    pub const LANGUAGE: &str = "Bahasa";
    pub const LANGUAGE_ID: &str = "Bahasa Indonesia";
    pub const THEME: &str = "Tema";
    pub const THEME_LIGHT: &str = "Terang";
    pub const THEME_DARK: &str = "Gelap";
    pub const THEME_SYSTEM: &str = "Ikut sistem";
    pub const THEME_ONLY_DARK: &str = "Saat ini hanya tema gelap.";
    pub const SAVE_FAILED: &str = "Pengaturan lokal tidak bisa disimpan.";

    pub const DEVICE: &str = "Perangkat";
    pub const MODEL: &str = "Model";
    pub const FIRMWARE: &str = "Firmware";
    pub const SERIAL: &str = "Nomor seri";
    pub const USB: &str = "USB";
    pub const FIRMWARE_NOTE: &str =
        "Update firmware tidak tersedia di OpenHE. Gunakan updater resmi Aula di Windows.";
    pub const RESET: &str = "Reset pabrik";
    pub const RESET_BODY: &str =
        "Menghapus semua profil, remap, macro, dan lampu di keyboard. Tidak bisa dibatalkan.";
    pub const RESET_OPEN: &str = "Reset keyboard…";
    pub const RESET_TYPE: &str = "Ketik RESET untuk melanjutkan.";
    pub const RESET_NOW: &str = "Reset sekarang";

    pub const RECONNECTING: &str = "Menyambung ulang…";
    pub const RESTARTING: &str = "Keyboard memulai ulang — membaca ulang…";
    pub const POLLING_DONE: &str = "Polling rate sekarang";
    pub const POLLING_MISMATCH: &str = "Keyboard tersambung lagi, tapi melaporkan";
    pub const RESET_DONE: &str = "Keyboard sudah direset ke bawaan pabrik dan dibaca ulang.";
    pub const PROFILE_DONE: &str = "Keyboard tersambung lagi dengan Profil";
    pub const PROFILE_MISMATCH: &str = "Keyboard tersambung lagi, tapi melaporkan Profil";
    pub const NOT_BACK: &str = "Keyboard belum tersambung lagi. Cabut lalu colokkan kembali.";
    pub const WRITE_FAILED: &str = "Perintah tidak terkirim.";
    pub const MODE_FAILED: &str = "Gagal mengganti mode sistem.";
}
use strings as t;

/// Indonesian text for a profile file problem.
pub fn file_error_text(e: &FileError) -> String {
    match e {
        FileError::NotJson(_) => t::FILE_NOT_JSON.to_string(),
        FileError::NotAProfile => t::FILE_NOT_PROFILE.to_string(),
        FileError::NewerVersion(_) => t::FILE_NEWER.to_string(),
        FileError::UnknownModel(_) => t::FILE_MODEL.to_string(),
        FileError::WrongModel { file, keyboard } => {
            format!("{} {file} (keyboard ini {keyboard}).", t::FILE_WRONG_MODEL)
        }
        FileError::BadValue(_) => t::FILE_BAD_VALUE.to_string(),
        FileError::TransparentOnBase(k) => format!(
            "{} {} {}",
            t::FILE_TRANSPARENT_BASE,
            openhe_proto::keycodes::legend(*k),
            t::FILE_TRANSPARENT_BASE_TAIL
        ),
    }
}

/// The import/export flow.
enum Transfer {
    Idle,
    Exporting {
        path: PathBuf,
        pending: Pending<Exported>,
    },
    /// A file was read and planned; waiting for "Terapkan".
    Planned {
        file: String,
        /// The parsed file, re-planned against the keyboard on "Terapkan".
        data: Box<ProfileData>,
        /// The plan shown (what "Terapkan" writes if nothing changed).
        plan: Box<ImportPlan>,
        /// `(profile, worker generation)` the plan was made for.
        planned: (u8, u64),
        /// The keyboard changed since the first plan; this is a new one.
        changed: bool,
    },
    Importing(Pending<ImportReport>),
    Done(String),
    Failed(String, Option<String>),
}

impl Transfer {
    fn is_busy(&self) -> bool {
        matches!(self, Transfer::Exporting { .. } | Transfer::Importing(_))
    }
}

/// What "Terapkan" does with a planned import.
#[derive(Debug)]
pub enum Confirm {
    /// The keyboard is as it was when the plan was shown: write this plan,
    /// made again from the current snapshot.
    Write(ImportPlan),
    /// The profile or the snapshot changed since the plan was shown:
    /// nothing is written; show this new plan (or why it cannot be made)
    /// and ask again.
    Changed(Result<ImportPlan, FileError>),
}

/// "Terapkan" on a plan made for `planned` = `(profile, generation)`.
/// Always plans again against `snap`, so what gets cleared or skipped
/// matches the keyboard now; refuses to write when the profile or the
/// worker generation moved since the user saw the plan.
pub fn confirm_import(
    data: &ProfileData,
    planned: (u8, u64),
    snap: &DeviceSnapshot,
    generation: u64,
) -> Confirm {
    let plan = plan_import(data, snap);
    if planned != (snap.profile, generation) {
        return Confirm::Changed(plan);
    }
    match plan {
        Ok(p) => Confirm::Write(p),
        Err(e) => Confirm::Changed(Err(e)),
    }
}

/// Indonesian summary of a finished import.
pub fn import_result_text(r: &Result<ImportReport, Error>) -> (bool, String) {
    match r {
        Ok(rep) if rep.failed.is_empty() => (true, t::IMPORTED.to_string()),
        Ok(rep) => {
            let mut parts: Vec<&str> = Vec::new();
            for (step, _) in &rep.failed {
                let p = match step {
                    ImportStep::Performance => t::PART_PERF,
                    ImportStep::Advanced(_) => t::PART_ADV,
                    ImportStep::Layers => t::PART_LAYERS,
                    ImportStep::Lighting => t::PART_LIGHT,
                    ImportStep::KeyColors => t::PART_COLORS,
                };
                if !parts.contains(&p) {
                    parts.push(p);
                }
            }
            (
                false,
                format!("{} {}.", t::IMPORT_PARTIAL, parts.join(", ")),
            )
        }
        Err(Error::Disconnected) => (false, t::IMPORT_DISCONNECTED.to_string()),
        Err(e) => (false, i18n::device_error(e)),
    }
}

/// Profile names are saved this long after the last keystroke (and at once
/// when the field loses focus).
pub const NAME_SAVE_DELAY: Duration = Duration::from_millis(800);

pub struct Page {
    switch: Option<(Pending<SwitchOutcome>, &'static str)>,
    /// A profile name changed and is not saved yet.
    name_save: Debounce,
    switch_error: Option<(String, Option<String>)>,
    /// Polling change, factory reset or profile switch (the keyboard
    /// re-enumerates after each).
    restart: RestartRun,
    /// What the last restart flow was for (its status shows there).
    last_restart: Option<RestartKind>,
    /// Profile and polling rate when the last restart's result was first
    /// shown (see [`Page::forget_stale_restart`]).
    restart_seen: Option<(u8, PollingRate)>,
    /// Polling segment shown while a change is under way.
    polling_target: Option<PollingRate>,
    transfer: Transfer,
    auto_enabled: bool,
    /// Runs the per-app switcher (see [`AutoSwitcher`]) while enabled.
    watcher: Option<ForegroundWatcher>,
    auto: Arc<AutoShared>,
    /// The saved "per-app switching on" was looked at (once per run).
    auto_restored: bool,
    app_error: Option<String>,
    /// Launch at login as the OS has it (read once, and after a change);
    /// the outer `None` is "not read yet", the inner one "unreadable".
    login_os: Option<Option<bool>>,
    new_rule: (String, u8),
    reset_open: bool,
    reset_text: String,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            switch: None,
            name_save: Debounce::new(NAME_SAVE_DELAY),
            switch_error: None,
            restart: RestartRun::default(),
            last_restart: None,
            restart_seen: None,
            polling_target: None,
            transfer: Transfer::Idle,
            auto_enabled: false,
            watcher: None,
            auto: Arc::default(),
            auto_restored: false,
            app_error: None,
            login_os: None,
            new_rule: (String::new(), 0),
            reset_open: false,
            reset_text: String::new(),
        }
    }
}

impl Page {
    /// Runs every frame, whichever page is shown: finishes jobs, follows a
    /// keyboard restart, and tells the per-app switcher (which runs on the
    /// watcher thread) what the UI holds it back with. `locked` (e.g. while
    /// calibrating, or a top-bar switch) holds automatic switches back.
    /// Returns an error for the shell to show (a failed automatic switch).
    pub fn background(
        &mut self,
        ctx: &AppCtx,
        locked: bool,
        egui_ctx: &egui::Context,
    ) -> Option<String> {
        let now = Instant::now();
        if let Some((p, failed)) = self.switch.as_mut() {
            if let Some(r) = p.poll() {
                let failed = *failed;
                self.switch = None;
                self.switch_error = apply_switch_effects(ctx.worker, switch_effects(r, failed));
            }
        }
        self.restart.pump(ctx.worker, ctx.snapshot, now);
        self.forget_stale_restart(ctx.snapshot);
        if !self.restart.is_busy() {
            self.polling_target = None;
        } else {
            egui_ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.poll_transfer(ctx);

        let saved_on = ctx.data.borrow().auto_profile_enabled;
        if self.restore_auto_due(saved_on) {
            self.set_auto(true, ctx, egui_ctx);
        }
        if self.names_due(now) && ctx.save_data().is_err() {
            self.app_error = Some(t::SAVE_FAILED.to_string());
        }
        if self.name_save.is_pending() {
            egui_ctx.request_repaint_after(NAME_SAVE_DELAY);
        }
        // The switcher itself runs on the watcher thread; the UI only says
        // what it holds switches back with and shows failures.
        self.auto.set_ui_lock(locked || self.auto_locked());
        self.auto.take_error()
    }

    fn poll_transfer(&mut self, ctx: &AppCtx) {
        match &mut self.transfer {
            Transfer::Exporting { path, pending } => {
                let Some(r) = pending.poll() else { return };
                let path = path.clone();
                self.transfer = match r {
                    Ok(out) => {
                        let json = ProfileFile::from_data(&out.data).to_json();
                        match std::fs::write(&path, json) {
                            Ok(()) => {
                                let name = path.file_name().map_or_else(
                                    || path.display().to_string(),
                                    |f| f.to_string_lossy().into_owned(),
                                );
                                let mut msg = format!("{} {name}.", t::EXPORTED);
                                if out.data.lighting.is_none() {
                                    msg = format!("{msg} {}", t::EXPORTED_NO_LIGHT);
                                }
                                Transfer::Done(msg)
                            }
                            Err(e) => Transfer::Failed(
                                t::WRITE_FILE_FAILED.to_string(),
                                Some(e.to_string()),
                            ),
                        }
                    }
                    Err(e) => Transfer::Failed(
                        format!("{} {}", t::EXPORT_FAILED, i18n::device_error(&e)),
                        Some(i18n::technical(&e)),
                    ),
                };
            }
            Transfer::Importing(pending) => {
                let Some(r) = pending.poll() else { return };
                // Whatever happened, the keyboard is the truth: re-read it
                // (after a disconnect the worker reloads on reconnect).
                if r != Err(Error::Disconnected) {
                    ctx.worker.request_reload();
                }
                let tech = r.as_ref().err().map(i18n::technical).or_else(|| {
                    r.as_ref()
                        .ok()
                        .and_then(|rep| rep.failed.first())
                        .map(|(_, e)| i18n::technical(e))
                });
                let (ok, text) = import_result_text(&r);
                self.transfer = if ok {
                    Transfer::Done(text)
                } else {
                    Transfer::Failed(text, tech)
                };
            }
            _ => {}
        }
    }

    /// Screenshot hook: 1 = polling change reconnecting, 2 = reset
    /// confirmation typed, 3 = import planned with a rule list. Sends
    /// nothing.
    #[cfg(feature = "preview")]
    pub fn preview(&mut self, state: u8) {
        match state {
            1 => {
                let kind = RestartKind::Polling(PollingRate::Hz1000);
                self.polling_target = Some(PollingRate::Hz1000);
                self.last_restart = Some(kind);
                self.restart.state = Restart::Reconnecting {
                    kind,
                    connects: u64::MAX,
                    since: Instant::now() + Duration::from_secs(3600),
                };
            }
            2 => {
                self.reset_open = true;
                self.reset_text = RESET_WORD.into();
            }
            _ => {
                self.transfer = Transfer::Planned {
                    file: "openhe-profil-2-Gaming.json".into(),
                    data: Box::new(ProfileData {
                        model: openhe_proto::Model::Win68HeMax,
                        keys: Vec::new(),
                        lighting: None,
                        key_colors: Vec::new(),
                        keep_lighting_flags: false,
                    }),
                    plan: Box::new(ImportPlan {
                        performance: vec![((0x04..0x1E).collect(), Default::default())],
                        skipped_macros: vec![0x10],
                        ..Default::default()
                    }),
                    planned: (1, 0),
                    changed: false,
                };
                self.auto_enabled = true;
            }
        }
    }

    /// A finished restart's "✔ … Profil 3" / "… 1000 Hz" describes the
    /// keyboard as it was then: once the profile or polling rate changes
    /// elsewhere (top bar, keyboard, per-app rule) the note goes.
    fn forget_stale_restart(&mut self, snapshot: Option<&DeviceSnapshot>) {
        if !matches!(self.restart.state, Restart::Done(_) | Restart::Failed(..)) {
            self.restart_seen = None;
            return;
        }
        let Some(s) = snapshot else {
            return;
        };
        let now = (s.profile, s.polling);
        match self.restart_seen {
            None => self.restart_seen = Some(now),
            Some(then) if then != now => {
                self.restart.state = Restart::Idle;
                self.last_restart = None;
                self.restart_seen = None;
            }
            Some(_) => {}
        }
    }

    /// A per-app switch is reconnecting the keyboard (for the shell's
    /// notice).
    pub fn auto_reconnecting(&self) -> bool {
        self.auto.reconnecting()
    }

    /// Automatic profile switches wait: a switch, restart or import of this
    /// page is running, or an import is planned and waiting for
    /// "Terapkan" (a switch would change the profile it was planned for).
    fn auto_locked(&self) -> bool {
        !self.idle() || matches!(self.transfer, Transfer::Planned { .. })
    }

    /// A profile name field this frame: `changed` (typed) or `lost_focus`.
    /// True when the names should be saved now (the field was left); typing
    /// saves after a pause (see [`Page::names_due`]).
    fn name_edited(&mut self, changed: bool, lost_focus: bool, now: Instant) -> bool {
        if lost_focus {
            self.name_save.cancel();
            return true;
        }
        if changed {
            self.name_save.bump(now);
        }
        false
    }

    /// True (once) when typed names have waited [`NAME_SAVE_DELAY`].
    fn names_due(&mut self, now: Instant) -> bool {
        self.name_save.take_due(now)
    }

    /// The app is closing: true when typed names are still waiting for
    /// their save (the caller saves now; the wait is dropped).
    pub fn take_unsaved_names(&mut self) -> bool {
        let pending = self.name_save.is_pending();
        self.name_save.cancel();
        pending
    }

    /// Nothing of this page is running on the keyboard.
    fn idle(&self) -> bool {
        self.switch.is_none() && !self.restart.is_busy() && !self.transfer.is_busy()
    }

    fn can_write(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && self.idle()
    }

    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        page_with_right_panel(
            ui,
            "settings-right",
            self,
            |page, ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                right: 10,
                                ..Default::default()
                            })
                            .show(ui, |ui| {
                                page.keyboard_section(ctx, ui);
                                ui.add_space(20.0);
                                page.app_section(ctx, ui);
                            });
                    });
            },
            |page, ui| page.right(ctx, ui),
        );
    }

    // --------------------------------------------------------- keyboard

    fn keyboard_section(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let snap = ctx.snapshot;
        let can = self.can_write(ctx);
        section_card(ui, t::KEYBOARD, |ui| {
            // Polling rate.
            setting_row(ui, t::POLLING, Some(t::POLLING_HINT), true, |ui| {
                let rates = [
                    (PollingRate::Hz125, "125"),
                    (PollingRate::Hz250, "250"),
                    (PollingRate::Hz500, "500"),
                    (PollingRate::Hz1000, "1K"),
                    (PollingRate::Hz2000, "2K"),
                    (PollingRate::Hz4000, "4K"),
                    (PollingRate::Hz8000, "8K Hz"),
                ];
                let current = snap.map(|s| s.polling);
                let mut shown = self
                    .polling_target
                    .or(current)
                    .unwrap_or(PollingRate::Hz8000);
                let r = Segmented::new(&rates)
                    .enabled(snap.is_some())
                    .read_only(!can)
                    .hover_text(t::POLLING_RESTART)
                    .show(ui, &mut shown);
                if r.changed() && Some(shown) != current {
                    self.change_polling(ctx, shown);
                }
            });
            self.restart_status(ui, |k| matches!(k, RestartKind::Polling(_)));

            // Win/Mac.
            setting_row(ui, t::SYSTEM, Some(t::SYSTEM_HINT), true, |ui| {
                let modes = [
                    (Some(SystemMode::Windows), t::WIN_LINUX),
                    (Some(SystemMode::Mac), t::MACOS),
                ];
                let current = snap.and_then(|s| s.system_mode);
                let mut mode = current;
                let r = Segmented::new(&modes)
                    .enabled(snap.is_some())
                    .read_only(!can)
                    .show(ui, &mut mode);
                if let (true, Some(m)) = (r.changed() && mode != current, mode) {
                    self.switch_error = None;
                    self.switch = Some((
                        ctx.worker
                            .submit("set-system-mode", move |kb| system_mode_job(kb, m)),
                        t::MODE_FAILED,
                    ));
                }
            });

            // Profiles.
            setting_row(ui, t::PROFILES, None, false, |ui| {
                let busy = !self.idle() || !ctx.can_write();
                if ui
                    .add_enabled(snap.is_some() && !busy, egui::Button::new(t::EXPORT))
                    .clicked()
                {
                    self.start_export(ctx);
                }
                if ui
                    .add_enabled(snap.is_some() && !busy, egui::Button::new(t::IMPORT))
                    .clicked()
                {
                    self.pick_import(ctx);
                }
            });
            self.profile_cards(ctx, ui, can);
            self.restart_status(ui, |k| matches!(k, RestartKind::Profile(_)));
            ui.add_space(4.0);
            card::hint(ui, t::NAME_HINT);
            if let Some((e, tech)) = &self.switch_error {
                ui.add_space(8.0);
                card::error_box(ui, e, "", None, tech.as_deref());
            }
            self.transfer_status(ctx, ui);
        });
    }

    fn change_polling(&mut self, ctx: &AppCtx, rate: PollingRate) {
        self.polling_target = Some(rate);
        self.last_restart = Some(RestartKind::Polling(rate));
        self.restart.start(
            RestartKind::Polling(rate),
            ctx.worker.connects(),
            ctx.worker
                .submit_restart("set-polling", RECONNECT_PAUSE, move |kb| {
                    polling_job(kb, rate)
                }),
        );
    }

    /// A profile switch re-enumerates the keyboard: the restart flow
    /// reconnects, and the reconnected keyboard's profile is checked.
    fn switch_profile(&mut self, ctx: &AppCtx, idx: u8) {
        self.switch_error = None;
        let kind = RestartKind::Profile(idx);
        self.last_restart = Some(kind);
        self.restart.start(
            kind,
            ctx.worker.connects(),
            ctx.worker
                .submit_restart("set-profile", RECONNECT_PAUSE, move |kb| {
                    profile_job(kb, idx)
                }),
        );
    }

    fn start_reset(&mut self, ctx: &AppCtx) {
        self.reset_open = false;
        self.reset_text.clear();
        self.last_restart = Some(RestartKind::FactoryReset);
        self.restart.start(
            RestartKind::FactoryReset,
            ctx.worker.connects(),
            ctx.worker
                .submit_restart("factory-reset", RECONNECT_PAUSE, reset_job),
        );
    }

    /// The status line of the last restart flow, under the control that
    /// started it (`mine` picks the kind).
    fn restart_status(&self, ui: &mut Ui, mine: impl Fn(RestartKind) -> bool) {
        let Some(kind) = self.last_restart.filter(|k| mine(*k)) else {
            return;
        };
        match &self.restart.state {
            Restart::Sending { .. } | Restart::Reconnecting { .. } => {
                let text = match kind {
                    RestartKind::FactoryReset => t::RESTARTING,
                    RestartKind::Polling(_) | RestartKind::Profile(_) => t::RECONNECTING,
                };
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(text).color(color::ACCENT));
                });
            }
            Restart::Done(msg) => {
                ui.label(RichText::new(format!("✔ {msg}")).color(color::SUCCESS));
            }
            Restart::Failed(msg, tech) => {
                card::error_box(ui, msg, "", None, tech.as_deref());
            }
            Restart::Idle => {}
        }
    }

    fn profile_cards(&mut self, ctx: &AppCtx, ui: &mut Ui, can: bool) {
        let active = ctx.snapshot.map(|s| s.profile);
        let gap = 10.0;
        let w = ((ui.available_width() - 3.0 * gap) / 4.0).max(80.0);
        let mut switch_to = None;
        let mut save = false;
        // One row allocated up front; cards and name fields are placed in
        // it by hand so the text fields never move the layout cursor.
        let (row, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 72.0), Sense::hover());
        {
            for i in 0..4u8 {
                let is_active = active == Some(i);
                let rect = Rect::from_min_size(
                    row.min + egui::vec2(f32::from(i) * (w + gap), 0.0),
                    egui::vec2(w, 72.0),
                );
                let resp = ui.interact(rect, ui.id().with(("profile-card", i)), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        can,
                        is_active,
                        format!("{} {}", t::PROFILE, i + 1),
                    )
                });
                let (fill, stroke) = if is_active {
                    (color::ACCENT_SOFT, color::ACCENT)
                } else if resp.hovered() && can {
                    (color::KEY_HOVER, color::BORDER_STRONG)
                } else {
                    (color::BG, color::BORDER_STRONG)
                };
                ui.painter().rect(
                    rect,
                    CornerRadius::same(size::CONTROL_RADIUS + 2),
                    fill,
                    line(1.0, stroke),
                    egui::StrokeKind::Inside,
                );
                let label = if is_active {
                    format!("{} {} · {}", t::PROFILE, i + 1, t::ACTIVE)
                } else {
                    format!("{} {}", t::PROFILE, i + 1)
                };
                ui.painter().text(
                    rect.min + egui::vec2(16.0, 14.0),
                    Align2::LEFT_TOP,
                    label,
                    font::small(),
                    if is_active {
                        color::ACCENT
                    } else {
                        color::MUTED
                    },
                );
                let edit_rect = Rect::from_min_size(
                    rect.min + egui::vec2(12.0, 34.0),
                    egui::vec2(w - 24.0, 26.0),
                );
                let mut data = ctx.data.borrow_mut();
                let name = &mut data.profile_names[usize::from(i)];
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
                let edit = child.add(
                    egui::TextEdit::singleline(name)
                        .hint_text(t::UNNAMED)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .font(font::bold(15.0))
                        .text_color(color::TEXT)
                        .char_limit(24),
                );
                let left = edit.lost_focus() || (edit.changed() && !edit.has_focus());
                if self.name_edited(edit.changed(), left, Instant::now()) {
                    save = true;
                }
                drop(data);
                if resp.clicked() && can && !is_active && ctx.snapshot.is_some() {
                    switch_to = Some(i);
                }
            }
        }
        if save && ctx.save_data().is_err() {
            self.app_error = Some(t::SAVE_FAILED.to_string());
        }
        if let Some(i) = switch_to {
            self.switch_profile(ctx, i);
        }
    }

    // ---------------------------------------------------- import/export

    fn start_export(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else { return };
        let default_name = format!(
            "openhe-profil-{}.json",
            ctx.data
                .borrow()
                .profile_label(snap.profile)
                .replace(" · ", "-")
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        let keys: Vec<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
        self.transfer = Transfer::Exporting {
            path,
            pending: ctx
                .worker
                .submit("profile-export", move |kb| export_job(kb, &keys)),
        };
    }

    fn pick_import(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else { return };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        self.transfer = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file = path
                    .file_name()
                    .map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                let parsed = ProfileFile::from_json(&text).and_then(|f| f.to_data());
                match parsed.and_then(|d| plan_import(&d, snap).map(|p| (d, p))) {
                    Ok((data, plan)) => Transfer::Planned {
                        file,
                        data: Box::new(data),
                        plan: Box::new(plan),
                        planned: (snap.profile, ctx.worker.generation()),
                        changed: false,
                    },
                    Err(e) => Transfer::Failed(file_error_text(&e), Some(format!("{e:?}"))),
                }
            }
            Err(e) => Transfer::Failed(t::READ_FILE_FAILED.to_string(), Some(e.to_string())),
        };
    }

    fn transfer_status(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let mut next = None;
        match &self.transfer {
            Transfer::Idle => {}
            Transfer::Exporting { .. } | Transfer::Importing(_) => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    let text = if matches!(self.transfer, Transfer::Exporting { .. }) {
                        t::EXPORTING
                    } else {
                        t::IMPORTING
                    };
                    ui.label(RichText::new(text).color(color::ACCENT));
                });
            }
            Transfer::Planned {
                file,
                plan,
                planned,
                changed,
                ..
            } => {
                ui.add_space(10.0);
                let profile = planned.0 + 1;
                Card::new()
                    .fill(color::ACCENT_SOFT)
                    .stroke(color::ACCENT.gamma_multiply(0.5))
                    .padding(14)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(format!("{} {profile}: {file}", t::IMPORT_TITLE))
                                .color(color::TEXT)
                                .strong(),
                        );
                        let mut summary = format!(
                            "{} {}, {} {}",
                            plan.key_count(),
                            t::IMPORT_KEYS,
                            plan.advanced.len(),
                            t::IMPORT_FUNCS
                        );
                        if plan.lighting.is_some() {
                            summary = format!("{summary}, {}", t::IMPORT_LIGHT);
                        }
                        ui.label(
                            RichText::new(format!("{summary}. {}", t::IMPORT_OVERWRITE))
                                .color(color::MUTED),
                        );
                        if *changed {
                            card::hint(ui, t::IMPORT_CHANGED);
                        }
                        if !plan.skipped_macros.is_empty() {
                            card::hint(
                                ui,
                                format!(
                                    "{} {}",
                                    t::IMPORT_MACROS,
                                    legend_list(&plan.skipped_macros, 8)
                                ),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(ctx.can_write(), egui::Button::new(t::IMPORT_APPLY))
                                .clicked()
                            {
                                next = Some(true);
                            }
                            if ui.button(t::CANCEL).clicked() {
                                next = Some(false);
                            }
                        });
                    });
            }
            Transfer::Done(msg) => {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("✔ {msg}")).color(color::SUCCESS));
            }
            Transfer::Failed(msg, tech) => {
                ui.add_space(8.0);
                card::error_box(ui, msg, "", None, tech.as_deref());
            }
        }
        match next {
            Some(true) => {
                let Some(snap) = ctx.snapshot else { return };
                if let Transfer::Planned {
                    file,
                    data,
                    planned,
                    ..
                } = std::mem::replace(&mut self.transfer, Transfer::Idle)
                {
                    let generation = ctx.worker.generation();
                    self.transfer = match confirm_import(&data, planned, snap, generation) {
                        Confirm::Write(plan) => Transfer::Importing(
                            ctx.worker
                                .submit("profile-import", move |kb| import_job(kb, &plan)),
                        ),
                        Confirm::Changed(Ok(plan)) => Transfer::Planned {
                            file,
                            data,
                            plan: Box::new(plan),
                            planned: (snap.profile, generation),
                            changed: true,
                        },
                        Confirm::Changed(Err(e)) => {
                            Transfer::Failed(file_error_text(&e), Some(format!("{e:?}")))
                        }
                    };
                }
            }
            Some(false) => self.transfer = Transfer::Idle,
            None => {}
        }
    }

    // ------------------------------------------------------------- app

    fn app_section(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        section_card(ui, t::APP, |ui| {
            setting_row(ui, t::AUTO, Some(t::AUTO_HINT), true, |ui| {
                let mut on = self.auto_enabled;
                if toggle(ui, &mut on, true).changed() {
                    self.toggle_auto(on, ctx, ui.ctx());
                }
            });
            if self.auto_enabled {
                self.rules_editor(ctx, ui);
            }
            setting_row(ui, t::LOGIN, None, true, |ui| {
                // The OS entry is the truth (the saved flag can be stale
                // after a recovered data file); the flag is the fallback.
                let os = *self.login_os.get_or_insert_with(launch_at_login_enabled);
                let mut on = os.unwrap_or(ctx.data.borrow().launch_at_login);
                if toggle(ui, &mut on, true).changed() {
                    let r = set_launch_at_login(on);
                    self.login_os = None; // read again
                    match r {
                        Ok(()) => {
                            ctx.data.borrow_mut().launch_at_login = on;
                            if ctx.save_data().is_err() {
                                self.app_error = Some(t::SAVE_FAILED.to_string());
                            }
                        }
                        Err(e) => {
                            self.app_error = Some(format!("{} ({e})", t::LOGIN_FAILED));
                        }
                    }
                }
            });
            setting_row(ui, t::LANGUAGE, None, true, |ui| {
                ui.add_enabled(false, egui::Button::new(t::LANGUAGE_ID));
            });
            setting_row(ui, t::THEME, None, false, |ui| {
                let themes = [
                    (0u8, t::THEME_LIGHT),
                    (1, t::THEME_DARK),
                    (2, t::THEME_SYSTEM),
                ];
                let mut theme = 1u8;
                Segmented::new(&themes)
                    .disable(0)
                    .disable(2)
                    .hover_text(t::THEME_ONLY_DARK)
                    .show(ui, &mut theme);
            });
            if let Some(e) = &self.app_error {
                ui.add_space(8.0);
                card::error_box(ui, e, "", None, None);
            }
        });
    }

    /// True once per run, when per-app switching was on last time: it is
    /// started again when the app opens.
    fn restore_auto_due(&mut self, saved_on: bool) -> bool {
        let first = !std::mem::replace(&mut self.auto_restored, true);
        first && saved_on && !self.auto_enabled
    }

    /// The toggle: starts or stops switching and stores the choice.
    fn toggle_auto(&mut self, on: bool, ctx: &AppCtx, egui_ctx: &egui::Context) {
        self.auto_restored = true;
        self.set_auto(on, ctx, egui_ctx);
        let enabled = self.auto_enabled;
        ctx.data.borrow_mut().auto_profile_enabled = enabled;
        if ctx.save_data().is_err() && self.app_error.is_none() {
            self.app_error = Some(t::SAVE_FAILED.to_string());
        }
    }

    /// Starts or stops per-app switching. The watcher thread reads the
    /// foreground app every second and runs an [`AutoSwitcher`] against a
    /// [`WorkerHandle`], so switching does not depend on UI frames.
    fn set_auto(&mut self, on: bool, ctx: &AppCtx, egui_ctx: &egui::Context) {
        self.app_error = None;
        self.watcher = None;
        if !on {
            self.auto_enabled = false;
            return;
        }
        self.auto
            .set_rules(ctx.data.borrow().app_profile_rules.clone());
        let c = egui_ctx.clone();
        let repaint = c.clone();
        let handle = ctx.worker.handle();
        let shared = Arc::clone(&self.auto);
        let mut switcher = AutoSwitcher::default();
        let tick = move |app: Option<&str>| {
            if switcher.tick(app, &handle, &shared) {
                repaint.request_repaint();
            }
        };
        match ForegroundWatcher::spawn(move || c.request_repaint(), tick) {
            Ok(w) => {
                self.watcher = Some(w);
                self.auto_enabled = true;
            }
            Err(e) => self.app_error = Some(format!("{} ({e})", t::AUTO_FAILED)),
        }
    }

    fn rules_editor(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let seen = self
            .watcher
            .as_ref()
            .map(ForegroundWatcher::seen)
            .unwrap_or_default();
        let now = Instant::now();
        let mut changed = false;
        let mut typed = false;
        let mut remove = None;
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 0,
                right: 0,
                top: 4,
                bottom: 12,
            })
            .show(ui, |ui| {
                card::hint(
                    ui,
                    format!(
                        "{} {}",
                        t::AUTO_FRONT,
                        seen.last_other.as_deref().unwrap_or(t::AUTO_NONE)
                    ),
                );
                ui.add_space(6.0);
                let profiles: Vec<(u8, String)> = (0..4u8)
                    .map(|i| (i, ctx.data.borrow().profile_label(i)))
                    .collect();
                let opts: Vec<(u8, &str)> =
                    profiles.iter().map(|(i, l)| (*i, l.as_str())).collect();
                {
                    let mut data = ctx.data.borrow_mut();
                    if data.app_profile_rules.is_empty() {
                        card::hint(ui, t::AUTO_NO_RULES);
                    }
                    for (idx, (name, profile)) in data.app_profile_rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(name)
                                    .desired_width(200.0)
                                    .hint_text(t::AUTO_APP_HINT),
                            );
                            // Like profile names: the switcher gets every
                            // keystroke, the file is saved after a pause, on
                            // leaving the field, or when the app closes.
                            typed |= r.changed();
                            changed |= self.name_edited(r.changed(), r.lost_focus(), now);
                            let mut p = (*profile).min(3);
                            if Segmented::new(&opts)
                                .height(30.0)
                                .show(ui, &mut p)
                                .changed()
                            {
                                *profile = p;
                                changed = true;
                            }
                            if ui.button(t::AUTO_REMOVE).clicked() {
                                remove = Some(idx);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        data.app_profile_rules.remove(i);
                        changed = true;
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if self.new_rule.0.is_empty() {
                        if let Some(app) = &seen.last_other {
                            self.new_rule.0.clone_from(app);
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_rule.0)
                            .desired_width(200.0)
                            .hint_text(t::AUTO_APP_HINT),
                    );
                    Segmented::new(&opts)
                        .height(30.0)
                        .show(ui, &mut self.new_rule.1);
                    let ok = !self.new_rule.0.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new(t::AUTO_ADD)).clicked() {
                        let rule = (self.new_rule.0.trim().to_string(), self.new_rule.1);
                        ctx.data.borrow_mut().app_profile_rules.push(rule);
                        self.new_rule.0.clear();
                        changed = true;
                    }
                });
                card::hint(ui, t::AUTO_SESSION);
            });
        if changed || typed {
            self.auto
                .set_rules(ctx.data.borrow().app_profile_rules.clone());
        }
        if changed {
            if ctx.save_data().is_err() {
                self.app_error = Some(t::SAVE_FAILED.to_string());
            }
        }
    }

    // ---------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("settings-reset")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                top: 12,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.reset_card(ctx, ui));

        ui.label(
            RichText::new(t::DEVICE)
                .font(font::heading())
                .color(color::TEXT)
                .strong(),
        );
        ui.add_space(16.0);
        Card::new().padding(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let dash = "—".to_string();
            let (model, fw, serial, usb) = match ctx.snapshot {
                Some(s) => (
                    s.model.name().to_string(),
                    firmware_label(&s.info.firmware),
                    s.info.serial.clone(),
                    format!("{:04X}:{:04X}", openhe_proto::model::VID, s.model.pid()),
                ),
                None => (dash.clone(), dash.clone(), dash.clone(), dash),
            };
            for (k, v) in [
                (t::MODEL, model),
                (t::FIRMWARE, fw),
                (t::SERIAL, serial),
                (t::USB, usb),
            ] {
                card::kv_row(ui, k, &v);
                ui.add_space(6.0);
            }
        });
        ui.add_space(16.0);
        Card::new().padding(14).show(ui, |ui| {
            ui.set_width(ui.available_width());
            card::hint(ui, t::FIRMWARE_NOTE);
        });
    }

    fn reset_card(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        Card::new()
            .fill(color::DANGER_SOFT)
            .stroke(color::DANGER.gamma_multiply(0.45))
            .padding(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(t::RESET).color(color::DANGER).strong());
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(t::RESET_BODY).color(color::TEXT)).wrap());
                ui.add_space(10.0);
                self.restart_status(ui, |k| k == RestartKind::FactoryReset);
                let can = self.can_write(ctx);
                if !self.reset_open {
                    if danger_button(ui, t::RESET_OPEN, can).clicked() {
                        self.reset_open = true;
                        self.reset_text.clear();
                    }
                    return;
                }
                card::hint(ui, t::RESET_TYPE);
                ui.add(
                    egui::TextEdit::singleline(&mut self.reset_text)
                        .hint_text(RESET_WORD)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);
                let ok = reset_confirmed(&self.reset_text) && can;
                ui.horizontal(|ui| {
                    if ui.button(t::CANCEL).clicked() {
                        self.reset_open = false;
                        self.reset_text.clear();
                    }
                    if danger_button(ui, t::RESET_NOW, ok).clicked() {
                        self.start_reset(ctx);
                    }
                });
            });
    }
}

/// Reads and plans an import file (pure apart from the parse).
pub fn prepare_import(text: &str, snap: &DeviceSnapshot) -> Result<ImportPlan, FileError> {
    let file = ProfileFile::from_json(text)?;
    let data = file.to_data()?;
    plan_import(&data, snap)
}

/// A card with a title and rows separated by rules.
fn section_card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    Card::new().padding(20).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .font(font::bold(15.0))
                .color(color::TEXT),
        );
        ui.add_space(12.0);
        body(ui);
    });
}

/// "Title / hint ........ control", with a rule underneath when `rule`.
fn setting_row(
    ui: &mut Ui,
    title: &str,
    hint: Option<&str>,
    rule: bool,
    right: impl FnOnce(&mut Ui),
) {
    let r = ui
        .horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).color(color::TEXT));
                if let Some(h) = hint {
                    ui.label(RichText::new(h).font(font::small()).color(color::MUTED));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), right);
        })
        .response;
    if rule {
        let y = r.rect.bottom() + 6.0;
        ui.painter()
            .hline(r.rect.x_range(), y, line(1.0, color::BORDER));
        ui.add_space(12.0);
    } else {
        ui.add_space(6.0);
    }
}

fn danger_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(color::DANGER).strong())
            .fill(color::BG)
            .stroke(line(1.0, color::DANGER.gamma_multiply(0.7)))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .min_size(egui::vec2(ui.available_width(), 40.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::MockTransport;
    use openhe_proto::Model;

    pub(crate) fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x5C, 0x04, 0x80, 0x00]);
        r[4..8].copy_from_slice(&[0, ty, v, 0xFF]);
        r
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    /// A profile switch re-enumerates the keyboard (PROTOCOL 7.x finding
    /// 23): one write, no read-back on the dying connection.
    #[test]
    fn a_profile_switch_job_is_one_write_that_expects_a_restart() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 2));
        let mut k = kb(m);
        assert_eq!(profile_job(&mut k, 2), Ok(WriteOutcome::Done));
        assert_eq!(k.session_mut().transport().written().len(), 1);
        let mut k = kb(MockTransport::new());
        assert_eq!(profile_job(&mut k, 3), Ok(WriteOutcome::Reenumerating));
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    fn snap_on(profile: u8, mode: SystemMode) -> DeviceSnapshot {
        let mut s = crate::app::test_support::snapshot68();
        s.profile = profile;
        s.system_mode = Some(mode);
        s
    }

    #[test]
    fn a_profile_switch_reconnects_then_checks_the_reconnected_profile() {
        let t0 = Instant::now();
        let mut f = Restart::begin(RestartKind::Profile(2), 1);
        assert_eq!(
            f.job_done(Ok(WriteOutcome::Reenumerating), t0),
            RestartAction::Reconnect
        );
        // Still the old connection: nothing to check yet.
        f.observe(1, Some(&snap_on(0, SystemMode::Windows)), t0);
        assert!(f.is_busy());
        f.observe(1, None, t0);
        assert!(f.is_busy());
        // The reconnected keyboard reports profile 2 (with its own mode).
        f.observe(2, Some(&snap_on(2, SystemMode::Mac)), t0);
        assert_eq!(f, Restart::Done(format!("{} 3.", t::PROFILE_DONE)));

        let mut f = Restart::begin(RestartKind::Profile(2), 1);
        f.job_done(Ok(WriteOutcome::Done), t0);
        f.observe(2, Some(&snap_on(0, SystemMode::Mac)), t0);
        assert_eq!(
            f,
            Restart::Failed(format!("{} 1.", t::PROFILE_MISMATCH), None)
        );
        // `set_profile` reports Disconnected only when nothing was written
        // (after the write it is Reenumerating): the command did not go
        // out, so say so instead of a profile mismatch after the reconnect.
        let mut f = Restart::begin(RestartKind::Profile(1), 1);
        assert_eq!(
            f.job_done(Err(Error::Disconnected), t0),
            RestartAction::None
        );
        match &f {
            Restart::Failed(msg, _) => assert!(msg.starts_with(t::WRITE_FAILED), "{msg}"),
            other => panic!("{other:?}"),
        }
        // The same for polling and reset: a disconnect is reported by the
        // device API only when nothing was written.
        for kind in [
            RestartKind::Polling(PollingRate::Hz1000),
            RestartKind::FactoryReset,
        ] {
            let mut f = Restart::begin(kind, 1);
            assert_eq!(
                f.job_done(Err(Error::Disconnected), t0),
                RestartAction::None
            );
            assert!(matches!(f, Restart::Failed(..)), "{f:?}");
        }
    }

    #[test]
    fn disconnect_is_passed_through_and_does_not_reload() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let r = system_mode_job(&mut kb(m), SystemMode::Mac);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = switch_effects(r, "x");
        assert!(!fx.request_reload);
        assert!(fx.error.is_some());
    }

    #[test]
    fn mode_switch_rereads_the_mode() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x31, 1)); // accepted
        m.push_reply(order_reply(0x21, 0)); // verify: Mac
        m.push_reply(order_reply(0x21, 0)); // re-read
        let out = system_mode_job(&mut kb(m), SystemMode::Mac).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.system_mode, Some(Some(SystemMode::Mac)));
    }

    // ------------------------------------------------- restart flows

    fn snap_with(polling: PollingRate) -> DeviceSnapshot {
        let mut s = crate::app::test_support::snapshot68();
        s.polling = polling;
        s
    }

    #[test]
    fn polling_change_reconnects_without_an_error() {
        // Review Focus 4.
        let t0 = Instant::now();
        for outcome in [Ok(WriteOutcome::Done), Ok(WriteOutcome::Reenumerating)] {
            let mut f = Restart::begin(RestartKind::Polling(PollingRate::Hz1000), 1);
            assert!(f.is_busy());
            let action = f.job_done(outcome.clone(), t0);
            assert_eq!(action, RestartAction::Reconnect);
            assert!(matches!(f, Restart::Reconnecting { .. }), "{f:?}");
            // Still on the old connection (or none): keep waiting, no error.
            f.observe(1, Some(&snap_with(PollingRate::Hz8000)), t0);
            f.observe(1, None, t0 + Duration::from_secs(5));
            assert!(matches!(f, Restart::Reconnecting { .. }));
            // Back with the new rate.
            f.observe(
                2,
                Some(&snap_with(PollingRate::Hz1000)),
                t0 + Duration::from_secs(6),
            );
            assert_eq!(f, Restart::Done(format!("{} 1000 Hz.", t::POLLING_DONE)));
        }
    }

    #[test]
    fn polling_change_reports_a_rate_that_did_not_stick() {
        let t0 = Instant::now();
        let mut f = Restart::begin(RestartKind::Polling(PollingRate::Hz500), 3);
        f.job_done(Ok(WriteOutcome::Reenumerating), t0);
        f.observe(4, Some(&snap_with(PollingRate::Hz8000)), t0);
        match f {
            Restart::Failed(msg, _) => assert!(msg.contains("8000 Hz"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_keyboard_that_never_comes_back_times_out() {
        let t0 = Instant::now();
        let mut f = Restart::begin(RestartKind::Polling(PollingRate::Hz500), 0);
        f.job_done(Ok(WriteOutcome::Reenumerating), t0);
        f.observe(0, None, t0 + RECONNECT_TIMEOUT - Duration::from_millis(1));
        assert!(f.is_busy());
        f.observe(0, None, t0 + RECONNECT_TIMEOUT);
        assert_eq!(f, Restart::Failed(t::NOT_BACK.to_string(), None));
    }

    #[test]
    fn a_write_that_fails_before_restarting_is_an_error_and_reloads() {
        let mut f = Restart::begin(RestartKind::Polling(PollingRate::Hz500), 0);
        let a = f.job_done(Err(Error::Io("x".into())), Instant::now());
        assert_eq!(a, RestartAction::Reload);
        assert!(matches!(f, Restart::Failed(..)));
        assert!(!f.is_busy());
        // Late observations change nothing.
        f.observe(5, Some(&snap_with(PollingRate::Hz500)), Instant::now());
        assert!(matches!(f, Restart::Failed(..)));
    }

    #[test]
    fn factory_reset_treats_silence_as_restarting() {
        // No reply at all: exactly one write, then "restarting".
        let mut kb = kb(MockTransport::new());
        assert_eq!(reset_job(&mut kb), Ok(WriteOutcome::Reenumerating));
        assert_eq!(kb.session_mut().transport().written().len(), 1);
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x11, 0));
        assert_eq!(
            reset_job(&mut Keyboard::new(m, Model::Win68HeMax)),
            Ok(WriteOutcome::Done)
        );
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        assert_eq!(
            reset_job(&mut Keyboard::new(m, Model::Win68HeMax)),
            Err(Error::Disconnected)
        );

        let t0 = Instant::now();
        let mut f = Restart::begin(RestartKind::FactoryReset, 1);
        assert_eq!(
            f.job_done(Ok(WriteOutcome::Reenumerating), t0),
            RestartAction::Reconnect
        );
        f.observe(2, Some(&snap_with(PollingRate::Hz8000)), t0);
        assert_eq!(f, Restart::Done(t::RESET_DONE.to_string()));
    }

    #[test]
    fn polling_job_sends_one_frame() {
        let mut kb = kb(MockTransport::new());
        assert_eq!(
            polling_job(&mut kb, PollingRate::Hz1000),
            Ok(WriteOutcome::Reenumerating)
        );
        assert_eq!(kb.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn reset_needs_the_exact_word() {
        assert!(reset_confirmed("RESET"));
        assert!(!reset_confirmed("  RESET "), "no trimming");
        assert!(!reset_confirmed("reset"));
        assert!(!reset_confirmed("RESE"));
        assert!(!reset_confirmed(""));
    }

    // --------------------------------------------- per-app switching

    fn rules() -> Vec<(String, u8)> {
        vec![
            ("Valorant".into(), 1),
            ("  ".into(), 2),
            ("Figma".into(), 9), // invalid profile
            ("valorant".into(), 3),
            ("Code".into(), 0),
        ]
    }

    #[test]
    fn rules_match_by_name_ignoring_case_first_wins() {
        assert_eq!(match_rule("VALORANT", &rules()), Some(1));
        assert_eq!(match_rule(" Code ", &rules()), Some(0));
        assert_eq!(match_rule("Figma", &rules()), None);
        assert_eq!(match_rule("", &rules()), None);
        assert_eq!(match_rule("Safari", &rules()), None);
    }

    #[test]
    fn auto_switch_only_when_the_profile_differs_and_once_per_app() {
        let mut a = AutoSwitch::default();
        // Already on the rule's profile: nothing.
        assert_eq!(a.decide(Some("Valorant"), &rules(), 1), None);
        // On another profile: switch once.
        let mut a = AutoSwitch::default();
        assert_eq!(a.decide(Some("Valorant"), &rules(), 0), Some(1));
        // Still in front and the switch failed or was undone: no loop.
        assert_eq!(a.decide(Some("Valorant"), &rules(), 0), None);
        // Another app without a rule, then back: switch again.
        assert_eq!(a.decide(Some("Safari"), &rules(), 0), None);
        assert_eq!(a.decide(Some("Valorant"), &rules(), 0), Some(1));
        // OpenHE itself in front (None): nothing, and it resets.
        assert_eq!(a.decide(None, &rules(), 1), None);
        assert_eq!(a.decide(Some("Code"), &rules(), 1), Some(0));
    }

    // ------------------------------- per-app switching off the UI thread

    use crate::worker::pending_channel;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::Ordering;

    type Reply = std::sync::mpsc::Sender<Result<WriteOutcome, Error>>;

    /// A worker stand-in: its connection count and snapshot, whether it
    /// is locked, the switches asked for (their results are sent by the
    /// test), and the reloads requested. (The reconnect is the worker's
    /// own business: `switch` stands for `submit_restart`.)
    #[derive(Default)]
    struct FakePort {
        snapshot: RefCell<Option<DeviceSnapshot>>,
        connects: Cell<u64>,
        locked: Cell<bool>,
        sent: RefCell<Vec<(u8, Reply)>>,
        reloads: Cell<usize>,
    }

    impl FakePort {
        fn on(profile: u8) -> FakePort {
            let p = FakePort::default();
            p.connect(profile);
            p
        }

        /// A (re)connect that loaded a snapshot on `profile`.
        fn connect(&self, profile: u8) {
            self.connects.set(self.connects.get() + 1);
            *self.snapshot.borrow_mut() = Some(snap_on(profile, SystemMode::Windows));
        }

        fn disconnect(&self) {
            *self.snapshot.borrow_mut() = None;
        }
    }

    impl RestartPort for FakePort {
        fn connects(&self) -> u64 {
            self.connects.get()
        }
        fn request_reload(&self) {
            self.reloads.set(self.reloads.get() + 1);
        }
    }

    impl SwitchPort for FakePort {
        fn profile(&self) -> Option<u8> {
            self.snapshot.borrow().as_ref().map(|s| s.profile)
        }
        fn snapshot(&self) -> Option<DeviceSnapshot> {
            self.snapshot.borrow().clone()
        }
        fn locked(&self) -> bool {
            self.locked.get()
        }
        fn switch(&self, profile: u8) -> Pending<WriteOutcome> {
            let (tx, p) = pending_channel("auto-profile");
            self.sent.borrow_mut().push((profile, tx));
            p
        }
    }

    fn shared_with(rules: Vec<(String, u8)>) -> AutoShared {
        let s = AutoShared::default();
        s.set_rules(rules);
        s
    }

    #[test]
    fn the_switcher_decides_and_submits_without_the_ui() {
        let port = FakePort::on(0);
        let shared = shared_with(rules());
        let mut sw = AutoSwitcher::default();
        // Valorant in front, keyboard on profile 0: switch to 1.
        assert!(!sw.tick(Some("Valorant"), &port, &shared));
        assert_eq!(port.sent.borrow().len(), 1);
        assert_eq!(port.sent.borrow()[0].0, 1);
        // One switch at a time.
        sw.tick(Some("Code"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 1);
        // The write went out and the keyboard restarts: reconnect.
        port.sent.borrow()[0]
            .1
            .send(Ok(WriteOutcome::Reenumerating))
            .unwrap();
        assert!(!sw.tick(Some("Valorant"), &port, &shared));
        assert!(shared.reconnecting(), "the shell shows menyambung ulang");
        // Back on profile 1: done, no error, nothing more to do.
        port.connect(1);
        assert!(sw.tick(Some("Valorant"), &port, &shared), "finished");
        assert!(!shared.reconnecting(), "notice gone");
        assert_eq!(shared.take_error(), None);
        sw.tick(Some("Valorant"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 1, "already on 1");
    }

    /// While a switch reconnects nothing fires again, even though the
    /// snapshot still shows the old profile (or none at all).
    #[test]
    fn no_automatic_switch_fires_while_a_switch_reconnects() {
        let port = FakePort::on(0);
        let shared = shared_with(rules());
        let mut sw = AutoSwitcher::default();
        sw.tick(Some("Valorant"), &port, &shared);
        port.sent.borrow()[0]
            .1
            .send(Ok(WriteOutcome::Reenumerating))
            .unwrap();
        // Old snapshot (profile 0), then searching, for many ticks; other
        // apps with other rules come to the front meanwhile.
        for app in ["Valorant", "Code", "Valorant"] {
            sw.tick(Some(app), &port, &shared);
        }
        port.disconnect();
        for app in ["Code", "Valorant"] {
            sw.tick(Some(app), &port, &shared);
        }
        assert_eq!(port.sent.borrow().len(), 1, "no second switch");
        assert!(shared.reconnecting());
        // The keyboard comes back on another profile: reported once, and
        // not retried in a loop.
        port.connect(0);
        assert!(sw.tick(Some("Valorant"), &port, &shared));
        let e = shared.take_error().expect("mismatch reported");
        assert!(e.starts_with(i18n::events::AUTO_SWITCH_FAILED), "{e}");
        assert!(e.contains(t::PROFILE_MISMATCH), "{e}");
        sw.tick(Some("Valorant"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 1);
    }

    #[test]
    fn the_switcher_respects_every_lock() {
        let shared = shared_with(rules());
        let mut sw = AutoSwitcher::default();
        let port = FakePort::default();
        // Disconnected.
        sw.tick(Some("Valorant"), &port, &shared);
        port.connect(0);
        // The worker is busy, restarting or calibrating.
        port.locked.set(true);
        sw.tick(Some("Valorant"), &port, &shared);
        port.locked.set(false);
        // The UI holds it back (restart, import planned or running, a
        // switch from the top bar).
        shared.set_ui_lock(true);
        sw.tick(Some("Valorant"), &port, &shared);
        assert!(port.sent.borrow().is_empty());
        shared.set_ui_lock(false);
        sw.tick(Some("Valorant"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 1);
    }

    #[test]
    fn a_failed_switch_is_reported_once_and_not_retried_in_a_loop() {
        let port = FakePort::on(0);
        let shared = shared_with(rules());
        let mut sw = AutoSwitcher::default();
        sw.tick(Some("Valorant"), &port, &shared);
        port.sent.borrow()[0]
            .1
            .send(Err(Error::Io("x".into())))
            .unwrap();
        assert!(sw.tick(Some("Valorant"), &port, &shared));
        let e = shared.take_error().expect("shown by the shell");
        assert!(e.starts_with(i18n::events::AUTO_SWITCH_FAILED), "{e}");
        assert_eq!(shared.take_error(), None, "taken once");
        assert_eq!(port.reloads.get(), 1, "the state is re-read");
        assert!(!shared.reconnecting());
        sw.tick(Some("Valorant"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 1, "no retry loop");
        // Edited rules start over.
        shared.set_rules(rules());
        sw.tick(Some("Valorant"), &port, &shared);
        assert_eq!(port.sent.borrow().len(), 2);
    }

    /// End to end with a real worker on the watcher thread: the switch
    /// re-enumerates, the worker reconnects, and the new connection's
    /// profile and Win/Mac mode (stored per profile) are what the app shows.
    #[test]
    fn a_real_worker_switches_from_another_thread() {
        use crate::worker::tests::{order_reply, push_snapshot, push_snapshot_os};
        use crate::worker::{Worker, WorkerConfig};
        let opened = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let o = opened.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                let mut m = MockTransport::new();
                if o.fetch_add(1, Ordering::SeqCst) == 0 {
                    push_snapshot(&mut m, 0); // Windows
                    m.push_reply(order_reply(0x70, 2)); // the SET reply
                } else {
                    push_snapshot_os(&mut m, 2, 0); // profile 2 is Mac
                }
                Ok(Keyboard::new(m, Model::Win68HeMax))
            }),
            WorkerConfig {
                retry_interval: Duration::from_millis(5),
                idle_poll: Duration::from_millis(2),
                state_poll: Duration::from_secs(3600),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.is_connected() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let handle = w.handle();
        let shared = std::sync::Arc::new(shared_with(vec![("Game".into(), 2)]));
        let sh = shared.clone();
        // The watcher thread: no UI frame runs meanwhile.
        std::thread::spawn(move || {
            let mut sw = AutoSwitcher::default();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !sw.tick(Some("Game"), &handle, &sh) {
                assert!(Instant::now() < deadline, "never switched");
                std::thread::sleep(Duration::from_millis(2));
            }
        })
        .join()
        .unwrap();
        assert_eq!(opened.load(Ordering::SeqCst), 2, "reconnected once");
        let snap = w.snapshot().unwrap();
        assert_eq!(snap.profile, 2);
        assert_eq!(snap.system_mode, Some(SystemMode::Mac));
        assert!(!w.is_restarting());
        assert_eq!(shared.take_error(), None);
        assert!(!shared.ui_lock.load(Ordering::SeqCst));
    }

    // ----------------------------------------------------- import

    #[test]
    fn import_files_are_checked_before_anything_is_written() {
        let snap = crate::app::test_support::snapshot68();
        assert!(matches!(
            prepare_import("nope", &snap),
            Err(FileError::NotJson(_))
        ));
        let text = file_error_text(&FileError::WrongModel {
            file: "WIN 60 HE MAX".into(),
            keyboard: "WIN 68 HE MAX".into(),
        });
        assert!(text.starts_with(t::FILE_WRONG_MODEL), "{text}");
        assert!(
            !file_error_text(&FileError::NotJson("expected value".into())).contains("expected")
        );
    }

    fn sample_data() -> profile_file::ProfileData {
        let snap = crate::app::test_support::snapshot68();
        let json = ProfileFile::from_data(&profile_file::ProfileData {
            model: snap.model,
            keys: Vec::new(),
            lighting: None,
            key_colors: Vec::new(),
            keep_lighting_flags: false,
        })
        .to_json();
        ProfileFile::from_json(&json).unwrap().to_data().unwrap()
    }

    #[test]
    fn terapkan_replans_against_the_current_keyboard_and_refuses_a_stale_plan() {
        use openhe_proto::key::Advanced;
        let mut data = sample_data();
        let mut snap = crate::app::test_support::snapshot68();
        data.keys.push(profile_file::KeyData {
            code: 0x29,
            settings: snap.keys[&0x29],
            layers: [0x29, 0, 0, 0],
            advanced: openhe_device::AdvancedConfig::None,
        });
        // Planned on profile 1, generation 7.
        assert!(matches!(
            confirm_import(&data, (1, 7), &snap, 7),
            Confirm::Write(_)
        ));
        // Esc got a function since (e.g. on another page): the plan written
        // is made from the keyboard as it is now, so it clears it.
        snap.keys.get_mut(&0x29).unwrap().mode.advanced = Advanced::Dks;
        match confirm_import(&data, (1, 7), &snap, 7) {
            Confirm::Write(plan) => assert_eq!(plan.clear_advanced, vec![0x29]),
            _ => panic!("same generation: write"),
        }
        // The keyboard changed since the plan was shown (a reload, or
        // another profile): nothing is written, the new plan is shown.
        assert!(matches!(
            confirm_import(&data, (1, 7), &snap, 8),
            Confirm::Changed(Ok(_))
        ));
        snap.profile = 2;
        assert!(matches!(
            confirm_import(&data, (1, 7), &snap, 7),
            Confirm::Changed(Ok(_))
        ));
    }

    #[test]
    fn a_planned_import_holds_automatic_switches_back() {
        let mut page = Page::default();
        assert!(!page.auto_locked());
        page.transfer = Transfer::Planned {
            file: "x.json".into(),
            data: Box::new(sample_data()),
            plan: Box::default(),
            planned: (0, 0),
            changed: false,
        };
        assert!(page.auto_locked());
    }

    #[test]
    fn profile_names_are_saved_shortly_after_typing() {
        let t0 = Instant::now();
        let mut page = Page::default();
        // Typing: saved once the user pauses, not on every key.
        assert!(!page.name_edited(true, false, t0));
        assert!(!page.names_due(t0 + Duration::from_millis(100)));
        assert!(!page.name_edited(true, false, t0 + Duration::from_millis(200)));
        assert!(!page.names_due(t0 + NAME_SAVE_DELAY));
        assert!(page.names_due(t0 + Duration::from_millis(200) + NAME_SAVE_DELAY));
        assert!(!page.names_due(t0 + Duration::from_secs(10)), "saved once");
        // Leaving the field saves at once (and nothing is left pending).
        page.name_edited(true, false, t0);
        assert!(page.name_edited(false, true, t0));
        assert!(!page.names_due(t0 + Duration::from_secs(10)));
    }

    #[test]
    fn typed_rule_names_reach_the_switcher_and_are_saved_on_close() {
        use crate::app::test_support::offline_worker;
        use crate::storage::AppData;
        use egui_kittest::kittest::Queryable;
        use std::cell::RefCell;
        struct S {
            page: Page,
            w: crate::worker::Worker,
            data: RefCell<AppData>,
            theme: crate::theme::Theme,
            status: crate::worker::ConnectionStatus,
        }
        let mut d = AppData::default();
        d.app_profile_rules.push(("Valorant".into(), 1));
        d.set_save_blocked(true); // never touch the real data file
        let state = S {
            page: Page {
                auto_enabled: true, // rules editor without a watcher
                ..Page::default()
            },
            w: offline_worker(),
            data: RefCell::new(d),
            theme: crate::theme::Theme::default(),
            status: crate::worker::ConnectionStatus::Searching,
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440.0, 1400.0))
            .build_ui_state(
                |ui, s: &mut S| {
                    let ctx = AppCtx {
                        worker: &s.w,
                        snapshot: None,
                        status: &s.status,
                        busy: false,
                        theme: &s.theme,
                        data: &s.data,
                    };
                    s.page.ui(&ctx, ui);
                },
                state,
            );
        h.run_steps(2);
        macro_rules! field {
            () => {
                h.query_by(|n| {
                    n.role() == egui::accesskit::Role::TextInput
                        && n.value().is_some_and(|v| v.starts_with("Valorant"))
                })
                .expect("rule field")
            };
        }
        field!().focus();
        h.run_steps(2);
        field!().type_text("X");
        h.run_steps(2);
        let s = h.state_mut();
        assert_eq!(s.data.borrow().app_profile_rules[0].0, "ValorantX");
        // The switcher uses the name while the field still has focus...
        assert_eq!(s.page.auto.rules(), vec![("ValorantX".to_string(), 1)]);
        // ...and closing the app now saves it.
        assert!(s.page.take_unsaved_names());
    }

    #[test]
    fn per_app_switching_is_remembered() {
        use crate::app::test_support::offline_worker;
        use crate::storage::AppData;
        use std::cell::RefCell;
        // Started again once when the app opens, if it was on.
        let mut page = Page::default();
        assert!(page.restore_auto_due(true));
        assert!(!page.restore_auto_due(true), "once");
        assert!(!Page::default().restore_auto_due(false));
        // The switch itself is stored.
        let w = offline_worker();
        let theme = crate::theme::Theme::default();
        let mut d = AppData::default();
        d.auto_profile_enabled = true;
        d.set_save_blocked(true); // never touch the real data file
        let data = RefCell::new(d);
        let status = crate::worker::ConnectionStatus::Searching;
        let ctx = AppCtx {
            worker: &w,
            snapshot: None,
            status: &status,
            busy: false,
            theme: &theme,
            data: &data,
        };
        let mut page = Page::default();
        page.toggle_auto(false, &ctx, &egui::Context::default());
        assert!(!data.borrow().auto_profile_enabled);
        assert!(!page.auto_enabled);
    }

    #[test]
    fn import_results_read_in_indonesian() {
        assert_eq!(
            import_result_text(&Ok(ImportReport::default())),
            (true, t::IMPORTED.to_string())
        );
        let rep = ImportReport {
            failed: vec![
                (ImportStep::Lighting, Error::Timeout),
                (ImportStep::Advanced(4), Error::Io("verify failed".into())),
                (ImportStep::Advanced(7), Error::Timeout),
            ],
        };
        let (ok, text) = import_result_text(&Ok(rep));
        assert!(!ok);
        assert_eq!(
            text,
            format!("{} {}, {}.", t::IMPORT_PARTIAL, t::PART_LIGHT, t::PART_ADV)
        );
        let (_, text) = import_result_text(&Err(Error::Disconnected));
        assert_eq!(text, t::IMPORT_DISCONNECTED);
    }

    #[test]
    fn the_page_renders_in_every_state() {
        use crate::app::test_support::{offline_worker, run_frames, snapshot68};
        use crate::storage::AppData;
        use std::cell::RefCell;
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let w = offline_worker();
        let data = RefCell::new(AppData::default());
        data.borrow_mut()
            .app_profile_rules
            .push(("Valorant".into(), 1));
        let theme = crate::theme::Theme::default();
        let snap = snapshot68();
        let status = crate::worker::ConnectionStatus::Connected(snap.clone());
        let mut page = Page {
            reset_open: true,
            reset_text: "RESET".into(),
            last_restart: Some(RestartKind::Polling(PollingRate::Hz1000)),
            restart: RestartRun {
                state: Restart::Reconnecting {
                    kind: RestartKind::Polling(PollingRate::Hz1000),
                    connects: 0,
                    since: Instant::now(),
                },
                job: None,
            },
            transfer: Transfer::Planned {
                file: "x.json".into(),
                data: Box::new(sample_data()),
                plan: Box::default(),
                planned: (1, 0),
                changed: true,
            },
            auto_enabled: true, // rules editor without a watcher
            ..Page::default()
        };
        for snapshot in [Some(&snap), None] {
            let app_ctx = AppCtx {
                worker: &w,
                snapshot,
                status: &status,
                busy: false,
                theme: &theme,
                data: &data,
            };
            run_frames(&ctx, 2, |c| {
                egui::CentralPanel::default().show(c, |ui| page.ui(&app_ctx, ui));
            });
        }
    }
}

//! The app shell: sidebar, top bar, device card, and page routing.

use crate::i18n;
use crate::pages;
use crate::pages::settings::{
    apply_switch_effects, switch_effects, Restart, RestartKind, RestartRun, SwitchOutcome,
};
use crate::storage::{AppData, LoadNotice};
use crate::theme::{self, color, font, line, size, Theme};
use crate::widgets::card::Card;
use crate::widgets::segmented::Segmented;
use crate::worker::{ConnectionStatus, DeviceSnapshot, Pending, Worker, WorkerNotice, LOAD_LABEL};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Mesh, Pos2, Rect, RichText,
    Sense, Ui,
};
use openhe_proto::SystemMode;
use std::cell::RefCell;

/// What every page gets each frame.
///
/// Strings come from [`crate::i18n`] (plain `pub const`s, so there is no
/// runtime object to pass).
///
/// # How a page writes to the keyboard
///
/// Copy the Performa page (`pages/performance.rs`):
///
/// 1. Put the whole device conversation in a transport-generic function,
///    e.g. `pub fn apply_job<T: Transport>(kb: &mut Keyboard<T>, ..) ->
///    Result<Outcome, Error>`. It does the write, then **always** re-reads
///    the affected state (also after a failed write), and returns both the
///    write result and what it read. It returns `Err(Disconnected)` as is,
///    so the worker can reconnect.
/// 2. Submit it: `ctx.worker.submit("label", move |kb| apply_job(kb, ..))`
///    and keep the [`crate::worker::Pending`]; poll it once per frame.
/// 3. Turn the result into UI effects with a pure function (e.g.
///    `apply_effects(result) -> Effects`: what to patch into the snapshot
///    with [`Worker::patch_snapshot`], whether to
///    [`Worker::request_reload`], the Indonesian error text via
///    [`i18n::device_error`]). Never keep cached values after a failure.
/// 4. Test steps 1 and 3 with `MockTransport`, including a verify failure,
///    a failing re-read, and a failure halfway through a batch.
/// 5. Disable write controls unless [`AppCtx::can_write`] and no job of
///    the page is pending.
pub struct AppCtx<'a> {
    pub worker: &'a Worker,
    /// The device snapshot while connected.
    pub snapshot: Option<&'a DeviceSnapshot>,
    pub status: &'a ConnectionStatus,
    /// A job is queued or running, or the snapshot is loading. Write
    /// controls are disabled meanwhile.
    pub busy: bool,
    pub theme: &'a Theme,
    /// Local app data; call [`AppCtx::save_data`] after changing it.
    pub data: &'a RefCell<AppData>,
}

impl AppCtx<'_> {
    pub fn connected(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Connected, idle and not calibrating: write buttons may be enabled.
    /// While the worker's calibration guard is set the keyboard may still
    /// be in calibration mode (also after a stop that failed), so nothing
    /// is written until a stop worked.
    pub fn can_write(&self) -> bool {
        self.snapshot.is_some()
            && !self.busy
            && !self.worker.calibration_guard()
            && !self.worker.is_restarting()
    }

    pub fn save_data(&self) -> std::io::Result<()> {
        self.data.borrow().save()
    }

    /// The storage key of the connected keyboard (its serial).
    pub fn keyboard_key(&self) -> Option<String> {
        self.snapshot
            .map(|s| crate::skin::skin_key(s.model, &s.info.serial))
    }

    /// The key cap colours to draw the keyboard in (see
    /// [`AppData::skin_for`]).
    pub fn skin(&self) -> crate::skin::KeyboardSkin {
        self.data.borrow().skin_for(self.keyboard_key().as_deref())
    }
}

/// Longest the window stays open after a close request while a write job
/// runs.
pub const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// What the shell does with the window this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Nothing to do.
    Idle,
    /// Let the requested close happen.
    Allow,
    /// Cancel the requested close: a write is still running.
    Cancel,
    /// Still waiting for the write (repaint soon).
    Wait,
    /// The write finished (or the wait ran out): close the window now.
    Close,
}

/// Holds a window close while a write job runs (closing mid-write could
/// leave the keyboard half-written), for at most [`CLOSE_WAIT`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CloseGuard {
    since: Option<std::time::Instant>,
    releasing: bool,
}

impl CloseGuard {
    /// One frame: `requested` is the window's close request, `busy` the
    /// worker's [`Worker::is_busy`].
    pub fn frame(&mut self, requested: bool, busy: bool, now: std::time::Instant) -> CloseAction {
        let done = |since: std::time::Instant| !busy || now.duration_since(since) >= CLOSE_WAIT;
        if requested {
            if self.releasing {
                return CloseAction::Allow;
            }
            return match self.since {
                Some(since) if done(since) => CloseAction::Allow,
                Some(_) => CloseAction::Cancel,
                None if busy => {
                    self.since = Some(now);
                    CloseAction::Cancel
                }
                None => CloseAction::Allow,
            };
        }
        match self.since {
            Some(_) if self.releasing => CloseAction::Idle,
            Some(since) if done(since) => {
                self.releasing = true;
                CloseAction::Close
            }
            Some(_) => CloseAction::Wait,
            None => CloseAction::Idle,
        }
    }

    /// A close is being held ("Menunggu perubahan selesai…").
    pub fn waiting(&self) -> bool {
        self.since.is_some() && !self.releasing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageId {
    Performance,
    Advanced,
    Remap,
    Macros,
    Lighting,
    Calibration,
    Templates,
    Settings,
}

impl PageId {
    pub const ALL: [PageId; 8] = [
        PageId::Performance,
        PageId::Advanced,
        PageId::Remap,
        PageId::Macros,
        PageId::Lighting,
        PageId::Calibration,
        PageId::Templates,
        PageId::Settings,
    ];

    pub fn title(self) -> &'static str {
        use i18n::nav::*;
        match self {
            PageId::Performance => PERFORMANCE,
            PageId::Advanced => ADVANCED,
            PageId::Remap => REMAP,
            PageId::Macros => MACROS,
            PageId::Lighting => LIGHTING,
            PageId::Calibration => CALIBRATION,
            PageId::Templates => TEMPLATES,
            PageId::Settings => SETTINGS,
        }
    }

    pub fn subtitle(self) -> &'static str {
        use i18n::subtitle::*;
        match self {
            PageId::Performance => PERFORMANCE,
            PageId::Advanced => ADVANCED,
            PageId::Remap => REMAP,
            PageId::Macros => MACROS,
            PageId::Lighting => LIGHTING,
            PageId::Calibration => CALIBRATION,
            PageId::Templates => TEMPLATES,
            PageId::Settings => SETTINGS,
        }
    }
}

pub struct OpenHeApp {
    worker: Worker,
    theme: Theme,
    page: PageId,
    data: RefCell<AppData>,
    data_notice: Option<LoadNotice>,
    performance: pages::performance::Page,
    advanced: pages::advanced::Page,
    remap: pages::remap::Page,
    macros: pages::macros::Page,
    lighting: pages::lighting::Page,
    calibration: pages::calibration::Page,
    templates: pages::templates::Page,
    settings: pages::settings::Page,
    /// A Win/Mac switch started from the top bar.
    switch: Option<TopbarSwitch>,
    /// A profile switch started from the top bar: the keyboard
    /// re-enumerates, so it runs the restart flow (reconnect, then a check
    /// of the reconnected keyboard's profile).
    profile_switch: RestartRun,
    /// An error from the top bar, shown as a dismissable notice.
    shell_error: Option<(String, Option<String>)>,
    close: CloseGuard,
    /// Keyboards whose colour dialog was closed without a choice this
    /// session (asked again next time the app opens).
    skin_asked: std::collections::BTreeSet<String>,
    /// The highlighted card of the colour dialog.
    skin_choice: crate::skin::KeyboardSkin,
}

/// A running top-bar switch and the value it asked for (shown meanwhile).
struct TopbarSwitch {
    pending: Pending<SwitchOutcome>,
    kind: TopbarAction,
}

/// What the user picked in the top bar this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TopbarAction {
    pub profile: Option<u8>,
    pub mode: Option<SystemMode>,
}

impl OpenHeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let worker = Worker::spawn();
        let ctx = cc.egui_ctx.clone();
        worker.set_repaint(move || ctx.request_repaint());
        // A broken data file must not stop the app: it is moved aside (so
        // the next save cannot overwrite it) and the shell says so.
        let (data, notice) = AppData::load();
        let mut app = Self::with_worker(worker, data);
        app.data_notice = notice;
        app
    }

    /// Builds the app around an existing worker (tests use one whose
    /// connector never opens a device).
    pub fn with_worker(worker: Worker, data: AppData) -> Self {
        OpenHeApp {
            worker,
            theme: Theme::default(),
            page: PageId::Performance,
            data: RefCell::new(data),
            data_notice: None,
            performance: Default::default(),
            advanced: Default::default(),
            remap: Default::default(),
            macros: Default::default(),
            lighting: Default::default(),
            calibration: Default::default(),
            templates: Default::default(),
            settings: Default::default(),
            switch: None,
            profile_switch: RestartRun::default(),
            shell_error: None,
            close: CloseGuard::default(),
            skin_asked: Default::default(),
            skin_choice: crate::skin::KeyboardSkin::GreyYellow,
        }
    }
}

impl eframe::App for OpenHeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        let now = std::time::Instant::now();
        match self.close.frame(requested, self.worker.is_busy(), now) {
            CloseAction::Cancel => ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose),
            CloseAction::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            CloseAction::Wait => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            CloseAction::Allow | CloseAction::Idle => {}
        }
        self.draw(ctx);
    }

    /// Saves profile names still waiting for their save, and never leaves
    /// the keyboard in calibration mode (it types nothing there): stop it
    /// and give the stop a moment to go out. The worker's calibration guard
    /// sends it too if this does not finish in time.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Profile names typed in the last moment are saved after a pause;
        // closing must not lose them.
        if self.settings.take_unsaved_names() {
            let _ = self.data.borrow().save();
        }
        // Whenever the guard is set (any page, any phase, also after a
        // failed stop), send a stop and wait for its own result. Should it
        // still not finish, dropping the worker waits for the thread's own
        // shutdown stop.
        if let Some(stop) = self.calibration.exit_stop(&self.worker) {
            pages::calibration::wait_for_stop(
                &self.worker,
                stop,
                pages::calibration::EXIT_STOP_WAIT,
            );
        }
    }
}

impl OpenHeApp {
    pub fn page(&self) -> PageId {
        self.page
    }

    pub fn set_page(&mut self, page: PageId) {
        self.change_page(page);
    }

    /// The keyboard may be calibrating: the Kalibrasi page is running one,
    /// or the worker's calibration guard is still set (a stop that failed,
    /// a reconnect whose stop failed). Writes, the top bar and automatic
    /// profile switches stay locked meanwhile.
    pub fn calibrating(&self) -> bool {
        self.calibration.is_active() || self.worker.calibration_guard()
    }

    /// The guard is set but no calibration runs on the page: the shell
    /// shows a banner with "Hentikan kalibrasi".
    pub fn stop_owed(&self) -> bool {
        self.worker.calibration_guard() && !self.calibration.is_active()
    }

    /// Switches pages; leaving Kalibrasi always stops a running calibration.
    fn change_page(&mut self, page: PageId) {
        if self.page == PageId::Calibration && page != PageId::Calibration {
            self.calibration.leave(&self.worker);
        }
        self.page = page;
    }

    /// Screenshot hook: see `pages::calibration::Page::preview`.
    #[cfg(feature = "preview")]
    pub fn preview_calibration(&mut self, done: &[u8], pressed: &[u8]) {
        self.calibration.preview(done, pressed);
    }

    /// Screenshot hook: see `pages::performance::Page::preview_live`.
    #[cfg(feature = "preview")]
    pub fn preview_live_depth(&mut self, depths: &[(u8, u16)], select: &[u8]) {
        self.performance.preview_live(depths, select);
    }

    /// Screenshot hook: see `pages::templates::Page::preview_state`.
    #[cfg(feature = "preview")]
    pub fn preview_templates(&mut self) {
        let status = self.worker.status();
        let snapshot = match &status {
            ConnectionStatus::Connected(s) => Some(s),
            _ => None,
        };
        let ctx = AppCtx {
            worker: &self.worker,
            snapshot,
            status: &status,
            busy: false,
            theme: &self.theme,
            data: &self.data,
        };
        self.templates.preview_state(&ctx);
    }

    /// The Template page (tests read its preview and open files with it).
    pub fn templates(&self) -> &pages::templates::Page {
        &self.templates
    }

    /// Opens a template file on the Template page, as its import dialog
    /// would (tests cannot drive the native dialog).
    pub fn import_template(&mut self, path: &std::path::Path) {
        let status = self.worker.status();
        let snapshot = match &status {
            ConnectionStatus::Connected(s) => Some(s),
            _ => None,
        };
        let ctx = AppCtx {
            worker: &self.worker,
            snapshot,
            status: &status,
            busy: self.worker.is_busy() || self.worker.is_restarting(),
            theme: &self.theme,
            data: &self.data,
        };
        self.templates.import_path(&ctx, path);
    }

    /// Screenshot hook: see `pages::settings::Page::preview`.
    #[cfg(feature = "preview")]
    pub fn preview_settings(&mut self, state: u8) {
        self.settings.preview(state);
    }

    /// The profile a top-bar switch is reconnecting to, if one runs.
    fn profile_target(&self) -> Option<u8> {
        match self.profile_switch.state {
            Restart::Sending {
                kind: RestartKind::Profile(p),
                ..
            }
            | Restart::Reconnecting {
                kind: RestartKind::Profile(p),
                ..
            } => Some(p),
            _ => None,
        }
    }

    /// Polls the top-bar switch jobs and starts a new one if asked.
    fn run_topbar(&mut self, action: TopbarAction, snapshot: Option<&DeviceSnapshot>) {
        let now = std::time::Instant::now();
        self.profile_switch.pump(&self.worker, snapshot, now);
        match std::mem::take(&mut self.profile_switch.state) {
            Restart::Failed(msg, tech) => {
                self.shell_error = Some((
                    format!("{} {msg}", i18n::events::PROFILE_SWITCH_FAILED),
                    tech,
                ));
            }
            Restart::Done(_) | Restart::Idle => {}
            running => self.profile_switch.state = running,
        }
        if let Some(sw) = self.switch.as_mut() {
            if let Some(result) = sw.pending.poll() {
                self.shell_error = apply_switch_effects(
                    &self.worker,
                    switch_effects(result, i18n::events::MODE_SWITCH_FAILED),
                );
                self.switch = None;
            }
        }
        if self.switch.is_some() || self.profile_switch.is_busy() {
            return;
        }
        if let Some(idx) = action.profile {
            self.shell_error = None;
            let connects = self.worker.connects();
            self.profile_switch.start(
                RestartKind::Profile(idx),
                connects,
                self.worker.submit_restart(
                    "set-profile",
                    pages::settings::RECONNECT_PAUSE,
                    move |kb| pages::settings::profile_job(kb, idx),
                ),
            );
        } else if let Some(mode) = action.mode {
            self.shell_error = None;
            self.switch = Some(TopbarSwitch {
                pending: self.worker.submit("set-system-mode", move |kb| {
                    pages::settings::system_mode_job(kb, mode)
                }),
                kind: action,
            });
        }
    }

    /// One frame of the whole window.
    pub fn draw(&mut self, ctx: &egui::Context) {
        let status = self.worker.status();
        self.draw_with_status(ctx, status);
    }

    /// One frame drawn for `status` instead of the worker's (screenshots
    /// and tests render a connected keyboard this way).
    pub fn draw_with_status(&mut self, ctx: &egui::Context, status: ConnectionStatus) {
        theme::ensure_fonts(ctx);
        let snapshot = match &status {
            ConnectionStatus::Connected(s) => Some(s),
            _ => None,
        };
        // A reconnect after a re-enumerating write (polling, reset, profile
        // switch) locks writes like a running job.
        let busy = self.worker.is_busy() || self.worker.is_restarting();
        let loading = self.worker.current_job() == Some(LOAD_LABEL);
        // Collect finished top-bar jobs.
        self.run_topbar(TopbarAction::default(), snapshot);
        // Calibration jobs are collected whichever page is shown, so a stop
        // sent on leaving the page is always seen through.
        self.calibration.pump(&self.worker, snapshot, ctx);
        if self.calibration.take_saved() {
            self.data.borrow_mut().last_calibrated = Some(pages::calibration::unix_now());
            if self.data.borrow().save().is_err() {
                self.shell_error = Some((i18n::storage::SAVE_FAILED.to_string(), None));
            }
        }
        let calibrating = self.calibrating();
        let stop_owed = self.stop_owed();
        if busy || snapshot.is_none() {
            // Keep polling pending jobs and the connection state.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        let side = egui::SidePanel::left("sidebar")
            .exact_width(size::SIDEBAR)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(color::SIDEBAR)
                    .inner_margin(Margin::symmetric(16, 20)),
            )
            .show(ctx, |ui| {
                let mut page = self.page;
                sidebar(ui, &mut page, &status, loading);
                page
            });
        let next_page = side.inner;
        let side = side.response.rect;
        if next_page != self.page {
            self.change_page(next_page);
        }

        let top = egui::TopBottomPanel::top("topbar")
            .exact_height(size::TOPBAR)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(color::BG)
                    .inner_margin(Margin::symmetric(size::PAGE_MARGIN as i8, 0)),
            )
            .show(ctx, |ui| {
                let shown = self.switch.as_ref().map(|s| s.kind).or_else(|| {
                    self.profile_target().map(|p| TopbarAction {
                        profile: Some(p),
                        mode: None,
                    })
                });
                let lock = if calibrating {
                    Some(i18n::events::LOCKED_CALIBRATING)
                } else {
                    None
                };
                let enabled = !busy && !calibrating;
                topbar(
                    ui,
                    self.page,
                    snapshot,
                    &self.data.borrow(),
                    TopbarState {
                        enabled,
                        lock,
                        shown,
                    },
                )
            });
        let action = top.inner;
        let top = top.response.rect;
        self.run_topbar(action, snapshot);
        // Rules between sidebar and content, and under the top bar.
        let rules = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("shell-rules"),
        ));
        let screen = ctx.screen_rect();
        rules.vline(side.right(), screen.y_range(), line(1.0, color::BORDER));
        rules.hline(
            side.right()..=screen.right(),
            top.bottom(),
            line(1.0, color::BORDER),
        );

        let worker_notice = snapshot.and(self.worker.notice());
        let closing = self.close.waiting();
        let reconnecting = self.profile_switch.is_busy() || self.settings.auto_reconnecting();
        if reconnecting
            || worker_notice.is_some()
            || self.data_notice.is_some()
            || self.shell_error.is_some()
            || stop_owed
            || closing
        {
            egui::TopBottomPanel::top("notices")
                .show_separator_line(false)
                .frame(Frame::new().fill(color::BG).inner_margin(Margin {
                    left: size::PAGE_MARGIN as i8,
                    right: size::PAGE_MARGIN as i8,
                    top: 12,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
                    if closing {
                        notice_line(ui, i18n::common::CLOSING);
                    }
                    if reconnecting {
                        notice_line(ui, i18n::events::PROFILE_RECONNECTING);
                    }
                    if stop_owed
                        && notice_bar(
                            ui,
                            i18n::events::CALIBRATION_OWED,
                            i18n::events::STOP_CALIBRATION,
                            snapshot.is_some() && !busy,
                        )
                    {
                        self.calibration.stop_owed(&self.worker);
                    }
                    if let Some(n) = &worker_notice {
                        // Skipped keys stay skipped on a reload: offer to
                        // hide the notice instead (it comes back if other
                        // keys are skipped later).
                        if let WorkerNotice::KeysSkipped(_) = n {
                            if notice_bar(ui, &worker_notice_text(n), i18n::common::DISMISS, true) {
                                self.worker.dismiss_notice();
                            }
                        } else if notice_bar(
                            ui,
                            &worker_notice_text(n),
                            i18n::common::RELOAD,
                            !busy,
                        ) {
                            self.worker.request_reload();
                        }
                    }
                    if let Some(n) = &self.data_notice {
                        if notice_bar(ui, &data_notice_text(n), i18n::common::DISMISS, true) {
                            self.data_notice = None;
                        }
                    }
                    if let Some((text, _)) = &self.shell_error {
                        if notice_bar(ui, text, i18n::common::DISMISS, true) {
                            self.shell_error = None;
                        }
                    }
                });
        }

        let app_ctx = AppCtx {
            worker: &self.worker,
            snapshot,
            status: &status,
            busy,
            theme: &self.theme,
            data: &self.data,
        };
        // Pengaturan's restart flows run whichever page is shown; per-app
        // profile switching runs on its own thread and is told here what
        // the UI holds it back with (calibration, a top-bar switch).
        // The Template page's jobs run whichever page is shown; a preview
        // is made for this profile, so automatic switches wait for it.
        self.templates.pump(&app_ctx);
        let auto_locked = calibrating
            || self.switch.is_some()
            || self.profile_switch.is_busy()
            || self.templates.holds_profile();
        if let Some(e) = self.settings.background(&app_ctx, auto_locked, ctx) {
            self.shell_error = Some((e, None));
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(color::BG))
            .show(ctx, |ui| match self.page {
                PageId::Performance => self.performance.ui(&app_ctx, ui),
                PageId::Advanced => self.advanced.ui(&app_ctx, ui),
                PageId::Remap => self.remap.ui(&app_ctx, ui),
                PageId::Macros => self.macros.ui(&app_ctx, ui),
                PageId::Lighting => self.lighting.ui(&app_ctx, ui),
                PageId::Calibration => self.calibration.ui(&app_ctx, ui),
                PageId::Templates => self.templates.ui(&app_ctx, ui),
                PageId::Settings => self.settings.ui(&app_ctx, ui),
            });
        self.skin_dialog(ctx, snapshot);
    }

    /// Asks for the colours of a keyboard connected for the first time
    /// (the keyboard does not report them), and remembers the keyboard
    /// last connected so its colours show while it is unplugged.
    fn skin_dialog(&mut self, ctx: &egui::Context, snapshot: Option<&DeviceSnapshot>) {
        let Some(snap) = snapshot else { return };
        let key = crate::skin::skin_key(snap.model, &snap.info.serial);
        if self.data.borrow().skin_of(&key).is_some() {
            if self.data.borrow().last_keyboard.as_deref() != Some(key.as_str()) {
                self.data.borrow_mut().last_keyboard = Some(key);
                // Only a convenience: a failed save is not worth a notice.
                let _ = self.data.borrow().save();
            }
            return;
        }
        if self.skin_asked.contains(&key) {
            return;
        }
        let outcome = crate::widgets::skin_picker::first_connect_dialog(
            ctx,
            snap.model,
            &snap.matrix,
            &mut self.skin_choice,
        );
        use crate::widgets::skin_picker::DialogOutcome;
        let skin = match outcome {
            None => return,
            Some(DialogOutcome::Dismissed) => {
                self.skin_asked.insert(key);
                return;
            }
            Some(DialogOutcome::Later) => crate::skin::KeyboardSkin::Gelap,
            Some(DialogOutcome::Use(s)) => s,
        };
        self.data.borrow_mut().set_skin(&key, skin);
        if self.data.borrow().save().is_err() {
            self.shell_error = Some((i18n::storage::SAVE_FAILED.to_string(), None));
        }
    }
}

/// Splits a page into a centre column and the 348 pt right panel. The right
/// panel is drawn first (egui panels must precede the central area); both
/// closures get the page state.
pub fn page_with_right_panel<S>(
    ui: &mut Ui,
    id: &'static str,
    state: &mut S,
    center: impl FnOnce(&mut S, &mut Ui),
    right: impl FnOnce(&mut S, &mut Ui),
) {
    egui::SidePanel::right(id)
        .exact_width(size::RIGHT_PANEL)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(color::SIDEBAR)
                .inner_margin(Margin::same(size::PAGE_MARGIN as i8)),
        )
        .show_inside(ui, |ui| {
            let r = ui.max_rect().expand(size::PAGE_MARGIN);
            ui.painter()
                .vline(r.left(), r.y_range(), line(1.0, color::BORDER));
            right(state, ui);
        });
    egui::CentralPanel::default()
        .frame(Frame::new().inner_margin(Margin::same(size::PAGE_MARGIN as i8)))
        .show_inside(ui, |ui| center(state, ui));
}

// ------------------------------------------------------------------ notices

/// Shell text for a worker notice.
pub fn worker_notice_text(n: &WorkerNotice) -> String {
    match n {
        WorkerNotice::ReloadFailed(e) => {
            format!("{} ({})", i18n::device::STALE_RELOAD, i18n::device_error(e))
        }
        WorkerNotice::EventsFailed(e) => {
            format!("{} ({})", i18n::device::STALE_EVENTS, i18n::device_error(e))
        }
        WorkerNotice::KeysSkipped(keys) => {
            let codes: Vec<u8> = keys.iter().map(|(c, _)| *c).collect();
            format!(
                "{} {}",
                i18n::device::KEYS_SKIPPED,
                crate::widgets::keyboard::legend_list(&codes, 8)
            )
        }
    }
}

/// Shell text for a data-file notice.
pub fn data_notice_text(n: &LoadNotice) -> String {
    match n {
        LoadNotice::Recovered { moved_to } => format!(
            "{} {}",
            i18n::storage::RECOVERED,
            moved_to.file_name().map_or_else(
                || moved_to.display().to_string(),
                |f| f.to_string_lossy().into_owned()
            )
        ),
        LoadNotice::Unreadable(_) => i18n::storage::UNREADABLE.to_string(),
    }
}

/// An amber notice row without a button.
fn notice_line(ui: &mut Ui, text: &str) {
    Card::new()
        .fill(color::ACCENT_SOFT)
        .stroke(color::ACCENT.gamma_multiply(0.5))
        .padding(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add(egui::Label::new(RichText::new(text).color(color::TEXT)).wrap());
            });
        });
    ui.add_space(4.0);
}

/// An amber notice row with one button; true when the button was clicked.
fn notice_bar(ui: &mut Ui, text: &str, button: &str, enabled: bool) -> bool {
    let mut clicked = false;
    Card::new()
        .fill(color::ACCENT_SOFT)
        .stroke(color::ACCENT.gamma_multiply(0.5))
        .padding(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").color(color::ACCENT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    clicked = ui.add_enabled(enabled, egui::Button::new(button)).clicked();
                    ui.add(egui::Label::new(RichText::new(text).color(color::TEXT)).wrap());
                });
            });
        });
    ui.add_space(4.0);
    clicked
}

// ------------------------------------------------------------------ sidebar

fn sidebar(ui: &mut Ui, page: &mut PageId, status: &ConnectionStatus, loading: bool) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
        paint_logo(ui, rect, color::ACCENT);
        ui.add_space(4.0);
        ui.label(
            RichText::new(i18n::common::APP_NAME)
                .font(font::bold(21.0))
                .color(color::TEXT)
                .strong(),
        );
    });
    ui.add_space(28.0);
    for p in PageId::ALL {
        if nav_item(ui, p.title(), *page == p).clicked() {
            *page = p;
        }
        ui.add_space(4.0);
    }
    egui::TopBottomPanel::bottom("device-card")
        .show_separator_line(false)
        .frame(Frame::new())
        .show_inside(ui, |ui| device_card(ui, status, loading));
}

fn nav_item(ui: &mut Ui, text: &str, active: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
    let fill = if active {
        Color32::from_rgb(0x2A, 0x21, 0x12)
    } else if resp.hovered() {
        color::KEY_HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(size::CONTROL_RADIUS), fill);
    let text_color = if active { color::ACCENT } else { color::TEXT };
    if active {
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            3.0,
            color::ACCENT,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        if active {
            font::bold(14.0)
        } else {
            egui::FontId::proportional(14.0)
        },
        text_color,
    );
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, text)
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "App V1.1.3" → "v1.1.3"; anything else as is.
pub fn firmware_label(raw: &str) -> String {
    let s = raw.trim();
    let s = s.strip_prefix("App ").unwrap_or(s);
    match s.strip_prefix('V') {
        Some(rest) => format!("v{rest}"),
        None => s.to_string(),
    }
}

fn device_card(ui: &mut Ui, status: &ConnectionStatus, loading: bool) {
    Card::new().padding(16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        device_card_body(ui, status, loading);
    });
}

fn device_card_body(ui: &mut Ui, status: &ConnectionStatus, loading: bool) {
    {
        let (dot, text) = match status {
            ConnectionStatus::Connected(_) => (color::SUCCESS, i18n::device::CONNECTED_USB),
            ConnectionStatus::Searching if loading => (color::ACCENT, i18n::device::LOADING),
            ConnectionStatus::Searching => (color::ACCENT, i18n::device::SEARCHING),
            ConnectionStatus::Error(_) => (color::DANGER, i18n::device::ERROR),
        };
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(RichText::new(text).font(font::small()).color(dot));
        });
        match status {
            ConnectionStatus::Connected(s) => {
                ui.label(
                    RichText::new(s.model.name())
                        .font(font::heading())
                        .color(color::TEXT)
                        .strong(),
                );
                crate::widgets::card::kv_row(
                    ui,
                    i18n::device::POLLING,
                    &format!("{} {}", s.polling.hz(), i18n::device::HZ),
                );
                crate::widgets::card::kv_row(
                    ui,
                    i18n::device::FIRMWARE,
                    &firmware_label(&s.info.firmware),
                );
            }
            ConnectionStatus::Searching => {
                ui.label(
                    RichText::new(i18n::device::NO_DEVICE)
                        .font(font::heading())
                        .color(color::MUTED),
                );
            }
            ConnectionStatus::Error(e) => {
                crate::widgets::card::hint(ui, e.as_str());
            }
        }
    }
}

// ------------------------------------------------------------------ top bar

/// How the top bar's switches behave this frame.
struct TopbarState {
    /// Idle and allowed to write.
    enabled: bool,
    /// Why the switches are locked (shown on hover), if they are.
    lock: Option<&'static str>,
    /// A switch in progress: show its target instead of the snapshot.
    shown: Option<TopbarAction>,
}

/// Longest profile label in the top bar ("1 · " included).
const TOPBAR_PROFILE_CHARS: usize = 14;

/// `s` cut to `max` characters, with "…" when cut.
pub fn elide_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// The page title, then the profile and Win/Mac switches (right-aligned).
/// Returns what the user picked; the app runs it as a worker job.
fn topbar(
    ui: &mut Ui,
    page: PageId,
    snapshot: Option<&DeviceSnapshot>,
    data: &AppData,
    state: TopbarState,
) -> TopbarAction {
    let mut action = TopbarAction::default();
    ui.horizontal_centered(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let connected = snapshot.is_some();
            let interactive = connected && state.enabled;
            let shown = state.shown.unwrap_or_default();
            let current_mode = snapshot.and_then(|s| s.system_mode);
            let mut mode = shown.mode.map(Some).unwrap_or(current_mode);
            let modes = [
                (Some(SystemMode::Mac), i18n::topbar::MAC),
                (Some(SystemMode::Windows), i18n::topbar::WIN),
            ];
            let mut seg = Segmented::new(&modes)
                .enabled(connected)
                .read_only(!interactive);
            if let Some(h) = state.lock {
                seg = seg.hover_text(h);
            }
            if seg.show(ui, &mut mode).changed() && mode != current_mode {
                action.mode = mode;
            }
            ui.add_space(12.0);

            // Long profile names are shortened so the bar never runs into
            // the title (names may be 24 characters).
            let labels: Vec<String> = (0..4u8)
                .map(|i| elide_chars(&data.profile_label(i), TOPBAR_PROFILE_CHARS))
                .collect();
            let opts: Vec<(u8, &str)> = labels
                .iter()
                .enumerate()
                .map(|(i, l)| (i as u8, l.as_str()))
                .collect();
            let current = snapshot.map_or(u8::MAX, |s| s.profile);
            let mut profile = shown.profile.unwrap_or(current);
            let mut seg = Segmented::new(&opts)
                .enabled(connected)
                .read_only(!interactive);
            if let Some(h) = state.lock {
                seg = seg.hover_text(h);
            }
            if seg.show(ui, &mut profile).changed() && profile != current {
                action.profile = Some(profile);
            }
            ui.label(RichText::new(i18n::topbar::PROFILE).color(color::MUTED));
            ui.add_space(16.0);
            // The title, then the subtitle cut to the room that is left.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(page.title())
                            .font(font::title())
                            .color(color::TEXT)
                            .strong(),
                    )
                    .extend(),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(RichText::new(page.subtitle()).color(color::MUTED)).truncate(),
                );
            });
        });
    });
    action
}

// --------------------------------------------------------------------- logo

/// The concept-A "Open Magnet" mark (assets/logo/concepts/a-open-magnet.svg):
/// three annular segments of a ring centred at (128, 132), outer radius 100,
/// inner radius 54. Each entry: outer arc start/end and inner arc start/end,
/// both running clockwise (SVG coordinates, y down).
const LOGO: [[(f32, f32); 4]; 3] = [
    [(158.9, 36.9), (204.6, 67.7), (144.7, 80.6), (169.4, 97.3)],
    [(210.9, 76.1), (45.1, 76.1), (172.8, 101.8), (83.2, 101.8)],
    [(51.4, 67.7), (97.1, 36.9), (86.6, 97.3), (111.3, 80.6)],
];
const LOGO_CENTER: (f32, f32) = (128.0, 132.0);
const LOGO_R: f32 = 100.0;
const LOGO_INNER_R: f32 = 54.0;

fn arc_angles(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let ang = |p: (f32, f32)| (p.1 - LOGO_CENTER.1).atan2(p.0 - LOGO_CENTER.0);
    let start = ang(a);
    let mut end = ang(b);
    while end < start {
        end += std::f32::consts::TAU;
    }
    (start, end)
}

/// Paints the logo mark filling `rect` (the ring's outer diameter).
pub fn paint_logo(ui: &Ui, rect: Rect, c: Color32) {
    let scale = rect.width().min(rect.height()) / (2.0 * LOGO_R);
    let center = rect.center();
    let at =
        |angle: f32, r: f32| -> Pos2 { center + egui::vec2(angle.cos(), angle.sin()) * r * scale };
    let mut mesh = Mesh::default();
    const STEPS: u32 = 24;
    for seg in LOGO {
        let (o0, o1) = arc_angles(seg[0], seg[1]);
        let (i0, i1) = arc_angles(seg[2], seg[3]);
        let base = mesh.vertices.len() as u32;
        for s in 0..=STEPS {
            let t = s as f32 / STEPS as f32;
            mesh.colored_vertex(at(o0 + (o1 - o0) * t, LOGO_R), c);
            mesh.colored_vertex(at(i0 + (i1 - i0) * t, LOGO_INNER_R), c);
        }
        for s in 0..STEPS {
            let k = base + 2 * s;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Helpers for headless UI tests: a worker that never opens a device, a
/// realistic snapshot, and a frame runner.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::widgets::keyboard::reference_matrix;
    use crate::worker::WorkerConfig;
    use openhe_device::{Error, KeySettings};
    use openhe_proto::key::{Advanced, KeyMode, TouchMode};
    use openhe_proto::{DeviceInfo, Micrometers, Model, PollingRate};
    use std::time::{Duration, Instant};

    /// A worker whose connector always fails (no HID access). It reports
    /// `Error` once, then waits an hour before retrying, so a status
    /// injected afterwards stays put. Jobs fail with `Disconnected`.
    pub fn offline_worker() -> Worker {
        let w = Worker::spawn_with(
            Box::new(|| Err(Error::Permission("test".into()))),
            WorkerConfig {
                retry_interval: Duration::from_secs(3600),
                idle_poll: Duration::from_millis(5),
                state_poll: Duration::from_secs(3600),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(w.status(), ConnectionStatus::Error(_)) {
            assert!(Instant::now() < deadline, "worker never reported");
            std::thread::sleep(Duration::from_millis(1));
        }
        w
    }

    /// A WIN 68 at factory defaults, with W A S D at 0.40 mm rapid trigger.
    pub fn snapshot68() -> DeviceSnapshot {
        let matrix = reference_matrix(Model::Win68HeMax);
        let keys = matrix
            .keys()
            .map(|(_, _, code)| {
                let wasd = crate::widgets::keyboard::WASD.contains(&code);
                let touch = if wasd {
                    TouchMode::RapidTrigger
                } else {
                    TouchMode::Global
                };
                let s = KeySettings {
                    mode: KeyMode {
                        touch,
                        advanced: Advanced::None,
                        high: 0,
                    },
                    actuation: Micrometers(if wasd { 400 } else { 2000 }),
                    rt_press: Micrometers(20),
                    rt_release: Micrometers(20),
                    top_dz: Micrometers(200),
                    bottom_dz: Micrometers(200),
                    switch: 0,
                };
                (code, s)
            })
            .collect();
        DeviceSnapshot {
            model: Model::Win68HeMax,
            info: DeviceInfo {
                board_id: 0x1901_0001,
                in_bootloader: false,
                serial: "A68M240193000000".into(),
                firmware: "App V1.1.3".into(),
                build_date: String::new(),
            },
            system_mode: Some(SystemMode::Windows),
            profile: 1,
            polling: PollingRate::Hz8000,
            matrix,
            keys,
        }
    }

    /// A WIN 60 HE MAX at factory defaults (its reference matrix), Windows
    /// mode, profile 1.
    pub fn snapshot60() -> DeviceSnapshot {
        let matrix = reference_matrix(Model::Win60HeMax);
        let keys = matrix
            .keys()
            .map(|(_, _, code)| {
                let s = KeySettings {
                    mode: KeyMode {
                        touch: TouchMode::Global,
                        advanced: Advanced::None,
                        high: 0,
                    },
                    actuation: Micrometers(2000),
                    rt_press: Micrometers(20),
                    rt_release: Micrometers(20),
                    top_dz: Micrometers(200),
                    bottom_dz: Micrometers(200),
                    switch: 0,
                };
                (code, s)
            })
            .collect();
        DeviceSnapshot {
            model: Model::Win60HeMax,
            matrix,
            keys,
            ..snapshot68()
        }
    }

    /// Runs `n` frames at 1440×900.
    pub fn run_frames(ctx: &egui::Context, n: usize, mut f: impl FnMut(&egui::Context)) {
        for _ in 0..n {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, &mut f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn every_page_renders_offline_and_connected() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
        for page in PageId::ALL {
            app.set_page(page);
            run_frames(&ctx, 2, |c| app.draw(c));
        }
        app.worker
            .inject_status(ConnectionStatus::Connected(snapshot68()));
        for page in PageId::ALL {
            app.set_page(page);
            run_frames(&ctx, 2, |c| app.draw(c));
        }
        assert_eq!(app.page(), PageId::Settings);
    }

    #[test]
    fn notices_render_and_are_indonesian() {
        use openhe_device::Error;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let w = offline_worker();
        w.inject_status(ConnectionStatus::Connected(snapshot68()));
        w.inject_notice(Some(WorkerNotice::ReloadFailed(Error::Timeout)));
        let mut app = OpenHeApp::with_worker(w, AppData::default());
        app.data_notice = Some(LoadNotice::Recovered {
            moved_to: "/x/openhe.json.bad".into(),
        });
        run_frames(&ctx, 3, |c| app.draw(c));
        let t = worker_notice_text(&WorkerNotice::ReloadFailed(Error::Timeout));
        assert!(t.starts_with(i18n::device::STALE_RELOAD), "{t}");
        assert!(!t.contains("timed out"));
        let k = worker_notice_text(&WorkerNotice::KeysSkipped(vec![(
            0x1A,
            Error::Io("unknown key mode".into()),
        )]));
        assert!(k.ends_with(" W"), "{k}");
        assert!(data_notice_text(app.data_notice.as_ref().unwrap()).ends_with("openhe.json.bad"));
    }

    #[test]
    fn bold_text_without_theme_apply_does_not_panic() {
        // The whole shell without theme::apply: the first frame draws bold
        // text in the regular face and installs the fonts for the next.
        std::thread::spawn(|| {
            let ctx = egui::Context::default();
            let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
            app.worker
                .inject_status(ConnectionStatus::Connected(snapshot68()));
            run_frames(&ctx, 3, |c| app.draw(c));
        })
        .join()
        .expect("shell without theme::apply");
        // A bare page test (no shell, no apply) on a fresh thread.
        std::thread::spawn(|| {
            let ctx = egui::Context::default();
            run_frames(&ctx, 1, |c| {
                egui::CentralPanel::default().show(c, |ui| {
                    ui.label(RichText::new("x").font(font::bold(14.0)));
                });
            });
        })
        .join()
        .expect("bold without theme::apply");
    }

    /// A worker whose first connection is on profile 0 (Windows) and
    /// answers one profile SET; every later connection loads `then`
    /// (profile, QueryWinMode value).
    fn switching_worker(
        then: (u8, u8),
    ) -> (
        Worker<openhe_device::MockTransport>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use crate::worker::tests::{order_reply, push_snapshot, push_snapshot_os};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let opened = std::sync::Arc::new(AtomicUsize::new(0));
        let o = opened.clone();
        let w = Worker::spawn_with(
            Box::new(move || {
                let mut m = openhe_device::MockTransport::new();
                if o.fetch_add(1, Ordering::SeqCst) == 0 {
                    push_snapshot(&mut m, 0);
                    m.push_reply(order_reply(0x70, then.0)); // the SET reply
                } else {
                    push_snapshot_os(&mut m, then.0, then.1);
                }
                Ok(openhe_device::Keyboard::new(
                    m,
                    openhe_proto::Model::Win68HeMax,
                ))
            }),
            crate::worker::WorkerConfig {
                retry_interval: std::time::Duration::from_millis(5),
                idle_poll: std::time::Duration::from_millis(2),
                state_poll: std::time::Duration::from_secs(3600),
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !w.is_connected() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        (w, opened)
    }

    /// Drives `run_topbar` like frames do until the profile switch ends.
    fn run_profile_switch<T: openhe_device::Transport + 'static>(
        w: &Worker<T>,
        flow: &mut RestartRun,
        idx: u8,
    ) -> Vec<bool> {
        let mut restarting_seen = Vec::new();
        flow.start(
            RestartKind::Profile(idx),
            w.connects(),
            w.submit_restart("set-profile", pages::settings::RECONNECT_PAUSE, move |kb| {
                pages::settings::profile_job(kb, idx)
            }),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while flow.is_busy() {
            assert!(std::time::Instant::now() < deadline, "{:?}", flow.state);
            let snap = w.snapshot();
            flow.pump(w, snap.as_ref(), std::time::Instant::now());
            restarting_seen.push(w.is_restarting());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        restarting_seen
    }

    #[test]
    fn a_profile_switch_reconnects_and_takes_profile_and_mode_from_the_new_connection() {
        // Profile 2 is in Mac mode: Win/Mac is stored per profile.
        let (w, opened) = switching_worker((2, 0));
        assert_eq!(w.snapshot().unwrap().system_mode, Some(SystemMode::Windows));
        let mut flow = RestartRun::default();
        let seen = run_profile_switch(&w, &mut flow, 2);
        assert!(seen.contains(&true), "locked while reconnecting");
        assert!(matches!(flow.state, Restart::Done(_)), "{:?}", flow.state);
        assert_eq!(opened.load(std::sync::atomic::Ordering::SeqCst), 2);
        let snap = w.snapshot().unwrap();
        assert_eq!(snap.profile, 2);
        assert_eq!(snap.system_mode, Some(SystemMode::Mac));
        assert!(!w.is_restarting(), "unlocked once back");
    }

    #[test]
    fn the_topbar_profile_switch_runs_the_restart_flow() {
        let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
        app.run_topbar(
            TopbarAction {
                profile: Some(2),
                mode: None,
            },
            None,
        );
        assert!(app.profile_switch.is_busy());
        assert_eq!(app.profile_target(), Some(2), "shown in the top bar");
        // The offline worker never runs the job (Disconnected, nothing
        // written): "Perintah tidak terkirim", not a reconnect.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.profile_switch.is_busy() {
            assert!(std::time::Instant::now() < deadline);
            app.run_topbar(TopbarAction::default(), None);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (text, _) = app.shell_error.clone().expect("reported");
        assert!(
            text.starts_with(i18n::events::PROFILE_SWITCH_FAILED),
            "{text}"
        );
        assert!(text.contains("Perintah tidak terkirim"), "{text}");
    }

    #[test]
    fn nothing_else_starts_while_a_topbar_profile_switch_reconnects() {
        let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
        app.profile_switch.state = Restart::Reconnecting {
            kind: RestartKind::Profile(2),
            connects: u64::MAX,
            since: std::time::Instant::now(),
        };
        app.run_topbar(
            TopbarAction {
                profile: Some(3),
                mode: Some(SystemMode::Mac),
            },
            None,
        );
        assert_eq!(app.profile_target(), Some(2));
        assert!(app.switch.is_none());
        assert_eq!(app.shell_error, None);
    }

    #[test]
    fn a_topbar_profile_switch_whose_keyboard_comes_back_elsewhere_is_reported() {
        let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
        app.profile_switch.state = Restart::Reconnecting {
            kind: RestartKind::Profile(2),
            connects: 0,
            since: std::time::Instant::now(),
        };
        let mut snap = snapshot68();
        snap.profile = 1;
        // A new connection (connects 0 -> 1) loaded profile 1.
        app.worker.inject_connect();
        app.run_topbar(TopbarAction::default(), Some(&snap));
        assert!(!app.profile_switch.is_busy());
        let (text, _) = app.shell_error.clone().expect("mismatch shown");
        assert!(
            text.starts_with(i18n::events::PROFILE_SWITCH_FAILED),
            "{text}"
        );
    }

    #[test]
    fn a_set_calibration_guard_locks_writes_app_wide() {
        let w = offline_worker();
        w.inject_status(ConnectionStatus::Connected(snapshot68()));
        let app = OpenHeApp::with_worker(w, AppData::default());
        let snap = snapshot68();
        let status = app.worker.status();
        let ctx = AppCtx {
            worker: &app.worker,
            snapshot: Some(&snap),
            status: &status,
            busy: false,
            theme: &app.theme,
            data: &app.data,
        };
        assert!(ctx.can_write());
        assert!(!app.calibrating());
        assert!(!app.stop_owed());
        // A stop that failed (page idle, guard still set): the keyboard may
        // still be calibrating, so every page and the top bar stay locked
        // and the shell offers "Hentikan kalibrasi".
        app.worker.set_calibration_guard(true);
        assert!(!ctx.can_write());
        assert!(app.calibrating());
        assert!(app.stop_owed());
        let egui_ctx = egui::Context::default();
        theme::apply(&egui_ctx);
        let mut app = app;
        run_frames(&egui_ctx, 2, |c| app.draw(c));
    }

    #[test]
    fn closing_waits_for_a_running_write_at_most_ten_seconds() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut g = CloseGuard::default();
        // Idle: the window closes at once.
        assert_eq!(g.frame(true, false, t0), CloseAction::Allow);
        // A write runs: hold the close, show "Menunggu perubahan selesai…".
        let mut g = CloseGuard::default();
        assert_eq!(g.frame(true, true, t0), CloseAction::Cancel);
        assert!(g.waiting());
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(g.frame(false, true, t1), CloseAction::Wait);
        // A second click while still busy keeps waiting (same deadline).
        assert_eq!(g.frame(true, true, t1), CloseAction::Cancel);
        // The write finished: close now.
        assert_eq!(g.frame(false, false, t1), CloseAction::Close);
        // ... and the close that follows goes through.
        assert_eq!(g.frame(true, false, t1), CloseAction::Allow);
        // A keyboard that never answers: close after CLOSE_WAIT anyway.
        let mut g = CloseGuard::default();
        g.frame(true, true, t0);
        assert_eq!(g.frame(false, true, t0 + CLOSE_WAIT), CloseAction::Close);
        assert_eq!(g.frame(true, true, t0 + CLOSE_WAIT), CloseAction::Allow);
        assert!(CLOSE_WAIT <= Duration::from_secs(10));
    }

    #[test]
    fn firmware_labels() {
        assert_eq!(firmware_label("App V1.1.3"), "v1.1.3");
        assert_eq!(firmware_label("1.0.7"), "1.0.7");
    }

    #[test]
    fn logo_points_lie_on_their_rings() {
        let dist =
            |p: (f32, f32)| ((p.0 - LOGO_CENTER.0).powi(2) + (p.1 - LOGO_CENTER.1).powi(2)).sqrt();
        for seg in LOGO {
            for p in &seg[..2] {
                assert!((dist(*p) - LOGO_R).abs() < 0.6, "{p:?}");
            }
            for p in &seg[2..] {
                assert!((dist(*p) - LOGO_INNER_R).abs() < 0.6, "{p:?}");
            }
        }
        // The bottom segment is the long one (> 180 degrees).
        let (a, b) = arc_angles(LOGO[1][0], LOGO[1][1]);
        assert!(b - a > std::f32::consts::PI);
        let (a, b) = arc_angles(LOGO[0][0], LOGO[0][1]);
        assert!(b - a < 1.0);
    }
}

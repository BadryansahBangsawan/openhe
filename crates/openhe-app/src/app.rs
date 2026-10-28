//! The app shell: sidebar, top bar, device card, and page routing.

use crate::i18n;
use crate::pages;
use crate::pages::settings::{apply_switch_effects, switch_effects, SwitchOutcome};
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

    /// Connected and idle: write buttons may be enabled.
    pub fn can_write(&self) -> bool {
        self.snapshot.is_some() && !self.busy
    }

    pub fn save_data(&self) -> std::io::Result<()> {
        self.data.borrow().save()
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
    Settings,
}

impl PageId {
    pub const ALL: [PageId; 7] = [
        PageId::Performance,
        PageId::Advanced,
        PageId::Remap,
        PageId::Macros,
        PageId::Lighting,
        PageId::Calibration,
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
    settings: pages::settings::Page,
    /// A profile or Win/Mac switch started from the top bar.
    switch: Option<TopbarSwitch>,
    /// An error from the top bar, shown as a dismissable notice.
    shell_error: Option<(String, Option<String>)>,
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
            settings: Default::default(),
            switch: None,
            shell_error: None,
        }
    }
}

impl eframe::App for OpenHeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
    }

    /// Never leave the keyboard in calibration mode (it types nothing
    /// there): stop it and give the stop a moment to go out. The worker's
    /// calibration guard sends it too if this does not finish in time.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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

    /// Screenshot hook: see `pages::settings::Page::preview`.
    #[cfg(feature = "preview")]
    pub fn preview_settings(&mut self, state: u8) {
        self.settings.preview(state);
    }

    /// Polls the top-bar switch job and starts a new one if asked.
    fn run_topbar(&mut self, action: TopbarAction) {
        if let Some(sw) = self.switch.as_mut() {
            if let Some(result) = sw.pending.poll() {
                let failed = if sw.kind.profile.is_some() {
                    i18n::events::PROFILE_SWITCH_FAILED
                } else {
                    i18n::events::MODE_SWITCH_FAILED
                };
                self.shell_error =
                    apply_switch_effects(&self.worker, switch_effects(result, failed));
                self.switch = None;
            }
        }
        if self.switch.is_some() {
            return;
        }
        if let Some(idx) = action.profile {
            self.shell_error = None;
            self.switch = Some(TopbarSwitch {
                pending: self.worker.submit("set-profile", move |kb| {
                    pages::settings::profile_job(kb, idx)
                }),
                kind: action,
            });
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
        let busy = self.worker.is_busy();
        let loading = self.worker.current_job() == Some(LOAD_LABEL);
        // Collect finished top-bar jobs.
        self.run_topbar(TopbarAction::default());
        // Calibration jobs are collected whichever page is shown, so a stop
        // sent on leaving the page is always seen through.
        self.calibration.pump(&self.worker, snapshot, ctx);
        let calibrating = self.calibration.is_active();
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
                let shown = self.switch.as_ref().map(|s| s.kind);
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
        self.run_topbar(action);
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
        if worker_notice.is_some() || self.data_notice.is_some() || self.shell_error.is_some() {
            egui::TopBottomPanel::top("notices")
                .show_separator_line(false)
                .frame(Frame::new().fill(color::BG).inner_margin(Margin {
                    left: size::PAGE_MARGIN as i8,
                    right: size::PAGE_MARGIN as i8,
                    top: 12,
                    bottom: 0,
                }))
                .show(ctx, |ui| {
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
        // Per-app profile switching and Pengaturan's restart flows run
        // whichever page is shown.
        if let Some(e) = self.settings.background(&app_ctx, calibrating, ctx) {
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
                PageId::Settings => self.settings.ui(&app_ctx, ui),
            });
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
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
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
        egui::FontId::proportional(15.0),
        text_color,
    );
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
        ui.label(
            RichText::new(page.title())
                .font(font::title())
                .color(color::TEXT)
                .strong(),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(page.subtitle()).color(color::MUTED));

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

            let labels: Vec<String> = (0..4u8).map(|i| data.profile_label(i)).collect();
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

    #[test]
    fn a_topbar_profile_switch_runs_a_job_and_reports_failure() {
        let mut app = OpenHeApp::with_worker(offline_worker(), AppData::default());
        app.run_topbar(TopbarAction {
            profile: Some(2),
            mode: None,
        });
        assert_eq!(app.switch.as_ref().unwrap().kind.profile, Some(2));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.switch.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.run_topbar(TopbarAction::default());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // The offline worker fails the job with Disconnected.
        let (text, _) = app.shell_error.clone().unwrap();
        assert!(
            text.starts_with(i18n::events::PROFILE_SWITCH_FAILED),
            "{text}"
        );
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

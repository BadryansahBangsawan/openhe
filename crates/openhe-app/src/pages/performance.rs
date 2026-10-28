//! Performa: per-key actuation, rapid trigger, deadzones and switch.
//!
//! Mockup `docs/design/mockups/01-performa.png`. The keys show their
//! actuation point; the right panel edits the selected keys. Only fields the
//! user touched are sent ([`build_change`]), through one worker job that
//! calls `Keyboard::set_performance` and then re-reads those keys, whether
//! the write worked or not, so the screen never shows stale values.

use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n::{self, perf as t};
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{
    legend_list, reference_matrix, KeyDecoration, KeyboardView, Selection, WASD,
};
use crate::widgets::segmented::Segmented;
use crate::widgets::slider_mm::{mm_short, mm_value_box, SliderMm};
use crate::widgets::toggle::toggle;
use crate::worker::{DeviceSnapshot, Pending};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, Sense,
    StrokeKind, Ui,
};
use openhe_device::keyboard::performance::{PerformanceChange, MAX_DEADZONE_UM, MIN_RT_UM};
use openhe_device::{Error, KeySettings};
use openhe_device::{Keyboard, Transport};
use openhe_proto::model::MIN_ACTUATION_UM;
use openhe_proto::travel::TravelFrame;
use openhe_proto::{Micrometers, Model, TouchMode};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// Factory default actuation (PROTOCOL 7.x finding 9).
pub const DEFAULT_ACTUATION_UM: u16 = 2000;
/// Fallback when a key's switch is unknown: the shortest switch travel.
const SAFE_MAX_TRAVEL_UM: u16 = 3300;

// ------------------------------------------------------------ form logic

/// The values shown in the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    pub rt_mode: bool,
    /// The first selected key is in touch mode Global (factory state).
    pub global: bool,
    pub actuation: u16,
    pub rt_press: u16,
    pub rt_release: u16,
    pub rt_sync: bool,
    pub top_dz: u16,
    pub bottom_dz: u16,
    pub switch: u16,
    /// The selected keys do not all share these values.
    pub mixed: bool,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            rt_mode: false,
            global: true,
            actuation: DEFAULT_ACTUATION_UM,
            rt_press: 20,
            rt_release: 20,
            rt_sync: true,
            top_dz: 200,
            bottom_dz: 200,
            switch: 0,
            mixed: false,
        }
    }
}

fn same_perf(a: &KeySettings, b: &KeySettings) -> bool {
    a.mode.touch == b.mode.touch
        && a.actuation == b.actuation
        && a.rt_press == b.rt_press
        && a.rt_release == b.rt_release
        && a.top_dz == b.top_dz
        && a.bottom_dz == b.bottom_dz
        && a.switch == b.switch
}

impl Form {
    /// Values of the first of `keys` found in the snapshot.
    pub fn from_keys(keys: &[u8], snap: &DeviceSnapshot) -> Option<Form> {
        let first = keys.iter().find_map(|k| snap.keys.get(k))?;
        let mixed = keys
            .iter()
            .filter_map(|k| snap.keys.get(k))
            .any(|s| !same_perf(s, first));
        Some(Form {
            rt_mode: first.mode.touch == TouchMode::RapidTrigger,
            global: first.mode.touch == TouchMode::Global,
            actuation: first.actuation.0,
            rt_press: first.rt_press.0,
            rt_release: first.rt_release.0,
            rt_sync: first.rt_press == first.rt_release,
            top_dz: first.top_dz.0,
            bottom_dz: first.bottom_dz.0,
            switch: first.switch,
            mixed,
        })
    }
}

/// Which controls the user changed since the form was loaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Touched {
    pub mode: bool,
    pub actuation: bool,
    pub rt: bool,
    pub top_dz: bool,
    pub bottom_dz: bool,
    pub switch: bool,
}

impl Touched {
    pub fn any(&self) -> bool {
        self.mode || self.actuation || self.rt || self.top_dz || self.bottom_dz || self.switch
    }
}

/// The change to send: only touched fields. The touch mode is sent only
/// when the mode control was touched; otherwise the device keeps each key's
/// mode (a Global key becomes Single when actuation or RT is set).
pub fn build_change(form: &Form, touched: &Touched) -> PerformanceChange {
    let um = |v: u16| Some(Micrometers(v));
    let mut c = PerformanceChange::default();
    if touched.mode {
        c.touch = Some(if form.rt_mode {
            TouchMode::RapidTrigger
        } else {
            TouchMode::Single
        });
    }
    if touched.actuation {
        c.actuation = um(form.actuation);
    }
    if touched.rt {
        c.rt_press = um(form.rt_press);
        c.rt_release = um(if form.rt_sync {
            form.rt_press
        } else {
            form.rt_release
        });
    }
    if touched.top_dz {
        c.top_dz = um(form.top_dz);
    }
    if touched.bottom_dz {
        c.bottom_dz = um(form.bottom_dz);
    }
    if touched.switch {
        c.switch = Some(form.switch);
    }
    c
}

fn switch_max(model: Model, id: u16) -> u16 {
    model
        .switches()
        .iter()
        .find(|s| s.id == id)
        .map_or(SAFE_MAX_TRAVEL_UM, |s| s.max_travel.0)
}

/// Upper limit for travel values: the chosen switch when the user changed
/// it, else the shortest travel among the selected keys' switches.
pub fn max_travel(snap: &DeviceSnapshot, keys: &[u8], form: &Form, touched: &Touched) -> u16 {
    if touched.switch {
        return switch_max(snap.model, form.switch);
    }
    keys.iter()
        .filter_map(|k| snap.keys.get(k))
        .map(|s| switch_max(snap.model, s.switch))
        .min()
        .unwrap_or(SAFE_MAX_TRAVEL_UM)
}

/// One batch of an apply: the keys and the change they all get.
pub type Batch = (Vec<u8>, PerformanceChange);

/// Which of a key's own (untouched) travel values exceed `max`.
fn over(s: &KeySettings, base: &PerformanceChange, max: u16) -> [bool; 3] {
    [
        base.actuation.is_none() && s.actuation.0 > max,
        base.rt_press.is_none() && s.rt_press.0 > max,
        base.rt_release.is_none() && s.rt_release.0 > max,
    ]
}

/// The writes for an apply: [`build_change`] for every key and, when the
/// switch changes, each key's own untouched travel values beyond the new
/// switch clamped to it (`set_performance` refuses to keep them, and the
/// form only shows the first key's values). Keys needing the same clamps
/// share a batch; the key order is kept.
pub fn plan_changes(
    snap: &DeviceSnapshot,
    keys: &[u8],
    form: &Form,
    touched: &Touched,
) -> Vec<Batch> {
    let base = build_change(form, touched);
    if !touched.switch {
        return vec![(keys.to_vec(), base)];
    }
    let max = switch_max(snap.model, form.switch);
    let mut groups: Vec<([bool; 3], Vec<u8>)> = Vec::new();
    for &k in keys {
        let o = snap
            .keys
            .get(&k)
            .map_or([false; 3], |s| over(s, &base, max));
        match groups.iter_mut().find(|(g, _)| *g == o) {
            Some((_, ks)) => ks.push(k),
            None => groups.push((o, vec![k])),
        }
    }
    groups
        .into_iter()
        .map(|([a, p, r], ks)| {
            let mut c = base.clone();
            let clamp = |on: bool| on.then_some(Micrometers(max));
            c.actuation = c.actuation.or(clamp(a));
            c.rt_press = c.rt_press.or(clamp(p));
            c.rt_release = c.rt_release.or(clamp(r));
            (ks, c)
        })
        .collect()
}

/// Some selected key has its own value beyond the chosen switch's travel.
fn needs_clamp(snap: &DeviceSnapshot, keys: &[u8], form: &Form, touched: &Touched) -> bool {
    touched.switch
        && plan_changes(snap, keys, form, touched)
            .iter()
            .any(|(_, c)| *c != build_change(form, touched))
}

/// What the apply job hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub write: Result<(), Error>,
    /// Settings re-read after the write, for every key that could be read.
    pub reloaded: Vec<(u8, KeySettings)>,
    /// Set when re-reading failed; the page then asks for a full reload.
    pub reload_error: Option<Error>,
}

/// The Performa write job. Runs on the worker thread; generic over the
/// transport so tests drive it with `MockTransport`.
///
/// Writes with `set_performance`, then re-reads every key, also when the
/// write failed (a batch can fail halfway, leaving some keys written). A
/// disconnect at any point is returned as `Err(Disconnected)` so the worker
/// drops the keyboard; other read-back failures stop the re-read and are
/// reported in `reload_error`.
pub fn apply_job<T: Transport>(
    kb: &mut Keyboard<T>,
    keys: &[u8],
    change: &PerformanceChange,
) -> Result<ApplyOutcome, Error> {
    apply_plan_job(kb, &[(keys.to_vec(), change.clone())])
}

/// [`apply_job`] for a plan of several batches ([`plan_changes`]): they are
/// written in order, stopping at the first failure, then every key of the
/// plan is re-read.
pub fn apply_plan_job<T: Transport>(
    kb: &mut Keyboard<T>,
    plan: &[Batch],
) -> Result<ApplyOutcome, Error> {
    let mut write = Ok(());
    for (keys, change) in plan {
        write = kb.set_performance(keys, change);
        if write.is_err() {
            break;
        }
    }
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let keys: Vec<u8> = plan.iter().flat_map(|(k, _)| k.iter().copied()).collect();
    let mut reloaded = Vec::with_capacity(keys.len());
    let mut reload_error = None;
    for &k in &keys {
        match kb.key_settings(k) {
            Ok(s) => reloaded.push((k, s)),
            Err(Error::Disconnected) => return Err(Error::Disconnected),
            Err(e) => {
                reload_error = Some(e);
                break;
            }
        }
    }
    Ok(ApplyOutcome {
        write,
        reloaded,
        reload_error,
    })
}

/// What the page does with a finished apply job (pure, so it is tested
/// without a UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyEffects {
    /// Freshly read settings to store in the snapshot.
    pub patch: Vec<(u8, KeySettings)>,
    /// Ask the worker for a full reload (the re-read did not finish).
    pub request_reload: bool,
    /// Indonesian text for the inline error box.
    pub error: Option<String>,
    /// Raw error text, for the "Detail teknis" tooltip.
    pub technical: Option<String>,
    /// The write went through: drop the edits and reload the form.
    pub applied: bool,
}

pub fn apply_effects(result: Result<ApplyOutcome, Error>) -> ApplyEffects {
    match result {
        Ok(out) => {
            let reread = !out.reloaded.is_empty();
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => {
                    let mut msg = i18n::device_error(e);
                    if reread {
                        msg = format!("{msg} {}", t::RELOADED_HINT);
                    }
                    (Some(msg), Some(i18n::technical(e)))
                }
            };
            ApplyEffects {
                applied: out.write.is_ok(),
                patch: out.reloaded,
                request_reload: out.reload_error.is_some(),
                error,
                technical,
            }
        }
        // Mid-batch disconnect: some keys may already be written. The
        // worker reloads everything on reconnect.
        Err(Error::Disconnected) => ApplyEffects {
            patch: Vec::new(),
            request_reload: false,
            error: Some(t::PARTIAL_WRITE.to_string()),
            technical: Some(i18n::technical(&Error::Disconnected)),
            applied: false,
        },
        // E.g. an internal error: the state is unknown, read it all again.
        Err(e) => ApplyEffects {
            patch: Vec::new(),
            request_reload: true,
            error: Some(i18n::device_error(&e)),
            technical: Some(i18n::technical(&e)),
            applied: false,
        },
    }
}

// ------------------------------------------------------------------ page

/// The keyboard state a job was submitted against: its profile and the
/// worker's connection count. A result read back under another profile or
/// after a reconnect is not patched into the (newer) snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentOn {
    pub profile: u8,
    pub connects: u64,
}

impl SentOn {
    pub fn now(ctx: &AppCtx) -> Option<SentOn> {
        ctx.snapshot.map(|s| SentOn {
            profile: s.profile,
            connects: ctx.worker.connects(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Actuation,
    Live,
}

pub struct Page {
    selection: Selection,
    form: Form,
    touched: Touched,
    /// Selection and worker generation the form was loaded from.
    loaded_from: Option<(BTreeSet<u8>, u64)>,
    pending: Option<Pending<ApplyOutcome>>,
    /// Profile and connection count when the pending job was submitted.
    sent_on: Option<SentOn>,
    /// The last change sent, for "Coba lagi".
    last_sent: Option<Vec<Batch>>,
    error: Option<String>,
    error_technical: Option<String>,
    applied: bool,
    /// Values were clamped to a newly chosen switch's travel.
    clamped: bool,
    view: View,
    /// Live key depth, polled while the "Kedalaman live" view is shown.
    live: LiveDepth,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            selection: Selection::default(),
            form: Form::default(),
            touched: Touched::default(),
            loaded_from: None,
            pending: None,
            sent_on: None,
            last_sent: None,
            error: None,
            error_technical: None,
            applied: false,
            clamped: false,
            view: View::Actuation,
            live: LiveDepth::default(),
        }
    }
}

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.poll_pending(ctx);
        self.sync_form(ctx);
        self.live
            .pump(ctx, self.view == View::Live && ctx.connected(), ui.ctx());
        page_with_right_panel(
            ui,
            "performance-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    /// Screenshot hook: the live view with these depths and selection.
    /// Sends nothing.
    #[cfg(feature = "preview")]
    pub fn preview_live(&mut self, depths: &[(u8, u16)], select: &[u8]) {
        self.view = View::Live;
        self.live.depth = depths.iter().copied().collect();
        self.live.last = Some(Instant::now() + Duration::from_secs(3600));
        self.selection.set(select.iter().copied());
    }

    fn poll_pending(&mut self, ctx: &AppCtx) {
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        let Some(result) = p.poll() else {
            return;
        };
        self.pending = None;
        let fx = apply_effects(result);
        let sent_on = self.sent_on.take();
        if sent_on.is_none() || sent_on != SentOn::now(ctx) {
            // Read only now, after a profile switch or a reconnect: the
            // values belong to the keyboard state of back then and the
            // snapshot was loaded afresh since. Nothing is patched; the
            // form reloads from the current snapshot.
            self.error = fx.error;
            self.error_technical = fx.technical;
            self.applied = false;
            if fx.applied {
                self.touched = Touched::default();
                self.clamped = false;
                self.loaded_from = None;
            }
            return;
        }
        let profile = sent_on.map(|s| s.profile);
        if !fx.patch.is_empty() {
            let fresh = fx.patch.clone();
            ctx.worker.patch_snapshot(move |s| {
                if Some(s.profile) != profile {
                    return;
                }
                for (k, v) in fresh {
                    s.keys.insert(k, v);
                }
            });
        }
        if fx.request_reload {
            ctx.worker.request_reload();
        }
        self.error = fx.error;
        self.error_technical = fx.technical;
        self.applied = fx.applied;
        if fx.applied {
            self.touched = Touched::default();
            self.clamped = false;
            self.reload_form(ctx, &fx.patch);
        }
        // On failure the edits stay, so "Coba lagi" resends the same change.
    }

    /// After a write: loads the form from the values just read back.
    ///
    /// `ctx.snapshot` is this frame's copy, taken before the patch above
    /// reached the worker, while the generation already counts the patch.
    /// Reloading from that copy (as `sync_form` would) showed the values
    /// from before the write and then never reloaded, so the fresh values
    /// are applied to it first. The selection stays the same, so the
    /// "Tersimpan di keyboard" status stays too.
    fn reload_form(&mut self, ctx: &AppCtx, fresh: &[(u8, KeySettings)]) {
        let Some(snap) = ctx.snapshot else {
            self.loaded_from = None;
            return;
        };
        let mut snap = snap.clone();
        for &(k, v) in fresh {
            snap.keys.insert(k, v);
        }
        let codes = self.selection.codes_in(&snap.matrix);
        if let Some(f) = Form::from_keys(&codes, &snap) {
            self.form = f;
        }
        self.loaded_from = Some((self.selection.as_set().clone(), ctx.worker.generation()));
    }

    /// Reloads the form when the selection or the device data changed and
    /// the user has no unsent edits for this selection.
    fn sync_form(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else {
            return;
        };
        self.selection.retain_present(&snap.matrix);
        let sel = self.selection.as_set().clone();
        let generation = ctx.worker.generation();
        let same_selection = self.loaded_from.as_ref().is_some_and(|(s, _)| *s == sel);
        let same_data = self
            .loaded_from
            .as_ref()
            .is_some_and(|(_, g)| *g == generation);
        if same_selection && (same_data || self.touched.any()) {
            return;
        }
        if !same_selection {
            self.touched = Touched::default();
            self.applied = false;
            self.clamped = false;
            self.error = None;
            self.error_technical = None;
        }
        let codes = self.selection.codes_in(&snap.matrix);
        if let Some(f) = Form::from_keys(&codes, snap) {
            self.form = f;
        }
        self.loaded_from = Some((sel, generation));
    }

    /// Selected keys in reading order (W, A, S, D) while connected.
    fn ordered(&self, ctx: &AppCtx) -> Vec<u8> {
        match ctx.snapshot {
            Some(s) => self.selection.codes_in(&s.matrix),
            None => self.selection.codes(),
        }
    }

    fn submit(&mut self, ctx: &AppCtx, plan: Vec<Batch>) {
        self.error = None;
        self.applied = false;
        self.last_sent = Some(plan.clone());
        self.error_technical = None;
        self.sent_on = SentOn::now(ctx);
        self.pending = Some(
            ctx.worker
                .submit("performance-apply", move |kb| apply_plan_job(kb, &plan)),
        );
    }

    // -------------------------------------------------------------- centre

    fn center(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let before = self.selection.clone();
        self.center_body(ctx, ui);
        if self.selection != before {
            // The right panel was drawn first; show it the new selection.
            ui.ctx().request_repaint();
        }
    }

    fn center_body(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let connected = ctx.connected();
        let model = ctx.snapshot.map_or(Model::Win68HeMax, |s| s.model);
        let fallback;
        let matrix = match ctx.snapshot {
            Some(s) => &s.matrix,
            None => {
                fallback = reference_matrix(model);
                &fallback
            }
        };

        ui.horizontal(|ui| {
            ui.set_min_height(40.0);
            let wasd: BTreeSet<u8> = WASD.into_iter().collect();
            let letters = self.selection.len() == 26
                && self
                    .selection
                    .codes()
                    .iter()
                    .all(|c| (0x04..=0x1D).contains(c));
            let all = self.selection.len() == matrix.len() && !self.selection.is_empty();
            if tool_button(ui, t::SELECT_ALL, all, connected).clicked() {
                self.selection.select_all(matrix);
            }
            if tool_button(ui, t::WASD, *self.selection.as_set() == wasd, connected).clicked() {
                self.selection.select_wasd(matrix);
            }
            if tool_button(ui, t::LETTERS, letters, connected).clicked() {
                self.selection.select_letters(matrix);
            }
            let clear = egui::Button::new(RichText::new(t::CLEAR).color(color::MUTED)).frame(false);
            if ui
                .add_enabled(connected && !self.selection.is_empty(), clear)
                .clicked()
            {
                self.selection.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let views = [
                    (View::Actuation, t::VIEW_ACTUATION),
                    (View::Live, t::VIEW_LIVE),
                ];
                Segmented::new(&views).show(ui, &mut self.view);
            });
        });
        ui.add_space(12.0);

        let snap = ctx.snapshot;
        let live = (self.view == View::Live).then_some(&self.live);
        ui.vertical_centered(|ui| {
            KeyboardView::new(model, matrix).enabled(connected).show(
                ui,
                &mut self.selection,
                |code| match live {
                    Some(l) => live_decoration(snap, l, code),
                    None => decorate(snap, code),
                },
            );
        });
        if !connected {
            ui.add_space(8.0);
            waiting_banner(ui, ctx);
        }
        ui.add_space(16.0);

        let h = ui.available_height().max(180.0);
        ui.horizontal_top(|ui| {
            let legend_w = 270.0_f32.min(ui.available_width() * 0.35);
            ui.allocate_ui_with_layout(
                egui::vec2(legend_w, h),
                Layout::top_down(Align::Min),
                |ui| self.legend_card(ui, h),
            );
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), h),
                Layout::top_down(Align::Min),
                |ui| self.travel_card(ctx, ui, h),
            );
        });
    }

    fn legend_card(&self, ui: &mut Ui, h: f32) {
        Card::new().show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 42.0));
            card::title(ui, t::LEGEND);
            ui.add_space(6.0);
            legend_row(
                ui,
                Swatch::Selected,
                &format!("{} · {} tombol", t::LEGEND_SELECTED, self.selection.len()),
            );
            legend_row(ui, Swatch::Plain, t::LEGEND_VALUE);
            legend_row(ui, Swatch::Changed, t::LEGEND_CHANGED);
            legend_row(ui, Swatch::Rt, t::LEGEND_RT);
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                card::hint(ui, t::SELECT_HINT);
            });
        });
    }

    /// "Gerak tombol": the travel bar of the selected key, or, in the live
    /// view, of the deepest key right now (among the selection, if any).
    fn travel_card(&mut self, ctx: &AppCtx, ui: &mut Ui, h: f32) {
        let codes = self.ordered(ctx);
        let live = self.view == View::Live;
        // In the live view: the deepest key, its depth and its own form
        // (when nothing is selected the form is that key's).
        let focus = match (live, ctx.snapshot) {
            (true, Some(s)) => {
                let pool: Vec<u8> = if codes.is_empty() {
                    s.matrix.keys().map(|(_, _, c)| c).collect()
                } else {
                    codes.clone()
                };
                self.live.deepest(&pool).map(|(code, depth)| {
                    let form = if codes.is_empty() {
                        Form::from_keys(&[code], s).unwrap_or_default()
                    } else {
                        self.form
                    };
                    (code, depth, form)
                })
            }
            _ => None,
        };
        let mut retry = false;
        Card::new().show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 42.0));
            ui.horizontal(|ui| {
                let title_code = focus.map(|f| f.0).or(codes.first().copied());
                let title = match title_code {
                    Some(c) => format!("{} · {}", t::TRAVEL, openhe_proto::keycodes::legend(c)),
                    None => t::TRAVEL.to_string(),
                };
                card::title(ui, title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hint = match (live, codes.first()) {
                        (false, _) => live_strings::SWITCH_TO_LIVE.to_string(),
                        (true, Some(&c)) if codes.len() == 1 => format!(
                            "{} {} {}",
                            live_strings::PRESS,
                            openhe_proto::keycodes::legend(c),
                            live_strings::TO_SEE_LIVE
                        ),
                        (true, _) => live_strings::PRESS_ANY.to_string(),
                    };
                    ui.label(RichText::new(hint).font(font::small()).color(color::MUTED));
                });
            });
            ui.add_space(12.0);
            if let (true, Some((msg, tech))) = (live, &self.live.error) {
                retry = card::error_box(
                    ui,
                    live_strings::FAILED,
                    msg,
                    Some(i18n::common::RETRY),
                    Some(tech),
                );
                return;
            }
            match (ctx.snapshot, focus) {
                (Some(s), Some((code, depth, form))) => {
                    let max = max_travel(s, &[code], &form, &Touched::default());
                    let max = if codes.is_empty() {
                        max
                    } else {
                        max_travel(s, &codes, &self.form, &self.touched)
                    };
                    paint_travel(ui, &form, max, Some(depth));
                }
                (Some(s), None) if !codes.is_empty() => {
                    let max = max_travel(s, &codes, &self.form, &self.touched);
                    paint_travel(ui, &self.form, max, None);
                }
                _ => card::hint(ui, t::NOTHING_SELECTED_BODY),
            }
        });
        if retry {
            self.live.retry();
        }
    }

    // --------------------------------------------------------- right panel

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        egui::TopBottomPanel::bottom("performance-apply")
            .show_separator_line(false)
            .frame(Frame::new().inner_margin(Margin {
                top: 12,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.apply_area(ctx, ui));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Leave room for the floating scroll bar.
                Frame::new()
                    .inner_margin(Margin {
                        right: 10,
                        ..Default::default()
                    })
                    .show(ui, |ui| self.editor(ctx, ui));
            });
    }

    fn editor(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let codes = self.ordered(ctx);
        let Some(snap) = ctx.snapshot.filter(|_| !codes.is_empty()) else {
            ui.label(
                RichText::new(t::NOTHING_SELECTED)
                    .font(font::heading())
                    .color(color::TEXT)
                    .strong(),
            );
            card::hint(
                ui,
                if ctx.connected() {
                    t::NOTHING_SELECTED_BODY
                } else {
                    i18n::common::WAITING_DEVICE_BODY
                },
            );
            return;
        };
        let editable = ctx.can_write() && self.pending.is_none();
        let model = snap.model;
        let switch_name = model
            .switches()
            .iter()
            .find(|s| s.id == self.form.switch)
            .map_or("?", |s| s.name);

        ui.label(
            RichText::new(legend_list(&codes, 6))
                .font(font::heading())
                .color(color::TEXT)
                .strong(),
        );
        ui.label(
            RichText::new(format!(
                "{} {} · {} {}",
                codes.len(),
                t::KEYS_SELECTED,
                t::SWITCH_PREFIX,
                switch_name
            ))
            .font(font::small())
            .color(color::MUTED),
        );
        if self.form.mixed {
            card::hint(ui, t::MIXED_VALUES);
        }
        ui.add_space(10.0);

        // Mode
        card::section(ui, t::MODE);
        let modes = [(false, t::MODE_NORMAL), (true, t::MODE_RT)];
        if Segmented::new(&modes)
            .full_width(true)
            .height(40.0)
            .enabled(editable)
            .show(ui, &mut self.form.rt_mode)
            .changed()
        {
            self.touched.mode = true;
        }
        if self.form.global && !self.touched.mode {
            card::hint(ui, t::MODE_GLOBAL_HINT);
        }
        ui.add_space(10.0);

        let max = max_travel(snap, &codes, &self.form, &self.touched);
        self.clamp_to(max);
        if needs_clamp(snap, &codes, &self.form, &self.touched) {
            self.clamped = true;
        }
        if self.clamped {
            card::hint(ui, format!("{} {} mm.", t::CLAMPED, mm_short(max)));
        }

        // Actuation
        value_header(ui, t::ACTUATION, |ui| {
            if mm_value_box(
                ui,
                &mut self.form.actuation,
                MIN_ACTUATION_UM..=max,
                editable,
            )
            .changed()
            {
                self.touched.actuation = true;
            }
        });
        if SliderMm::new(&mut self.form.actuation, MIN_ACTUATION_UM..=max)
            .enabled(editable)
            .end_labels(
                format!("{} · {}", mm_short(MIN_ACTUATION_UM), t::SENSITIVE),
                format!("{} · {}", mm_short(max), t::DEEP),
            )
            .show(ui)
            .changed()
        {
            self.touched.actuation = true;
        }
        ui.add_space(10.0);

        // Rapid trigger
        if self.form.rt_mode {
            Card::new().padding(16).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(t::RAPID_TRIGGER)
                        .color(color::ACCENT)
                        .strong(),
                );
                ui.add_space(6.0);
                let mut changed = false;
                value_header(ui, t::RT_PRESS, |ui| {
                    changed |= mm_value_box(ui, &mut self.form.rt_press, MIN_RT_UM..=max, editable)
                        .changed();
                });
                changed |= SliderMm::new(&mut self.form.rt_press, MIN_RT_UM..=max)
                    .enabled(editable)
                    .show(ui)
                    .changed();
                ui.add_space(6.0);
                let release_enabled = editable && !self.form.rt_sync;
                if self.form.rt_sync {
                    self.form.rt_release = self.form.rt_press;
                }
                value_header(ui, t::RT_RELEASE, |ui| {
                    changed |= mm_value_box(
                        ui,
                        &mut self.form.rt_release,
                        MIN_RT_UM..=max,
                        release_enabled,
                    )
                    .changed();
                });
                changed |= SliderMm::new(&mut self.form.rt_release, MIN_RT_UM..=max)
                    .enabled(release_enabled)
                    .show(ui)
                    .changed();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t::RT_SYNC).color(color::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if toggle(ui, &mut self.form.rt_sync, editable).changed() {
                            changed = true;
                            if self.form.rt_sync {
                                self.form.rt_release = self.form.rt_press;
                            }
                        }
                    });
                });
                if changed {
                    self.touched.rt = true;
                }
            });
            ui.add_space(10.0);
        }

        // Deadzones
        card::section(ui, t::DEADZONE);
        ui.columns(2, |cols| {
            if dz_box(&mut cols[0], t::DZ_TOP, &mut self.form.top_dz, editable) {
                self.touched.top_dz = true;
            }
            if dz_box(
                &mut cols[1],
                t::DZ_BOTTOM,
                &mut self.form.bottom_dz,
                editable,
            ) {
                self.touched.bottom_dz = true;
            }
        });
        ui.add_space(10.0);

        // Switch
        card::section(ui, t::SWITCH);
        ui.add_enabled_ui(editable, |ui| {
            egui::ComboBox::from_id_salt("performance-switch")
                .width(ui.available_width() - 8.0)
                .selected_text(switch_name)
                .show_ui(ui, |ui| {
                    for s in model.switches() {
                        let label = format!("{} · {} mm", s.name, mm_short(s.max_travel.0));
                        if ui
                            .selectable_value(&mut self.form.switch, s.id, label)
                            .changed()
                        {
                            self.touched.switch = true;
                        }
                    }
                });
        });
        card::hint(ui, t::SWITCH_HINT);
    }

    /// After the user picks a switch with less travel, the form's values
    /// beyond it are clamped for display and a note says so. They are not
    /// marked touched: [`plan_changes`] clamps each selected key's own value
    /// (the form only shows the first key's), so other keys keep theirs.
    /// Without a switch change nothing is clamped: a value outside the
    /// slider's range (another selected key's switch) is shown as it is and
    /// is not sent unless edited.
    fn clamp_to(&mut self, max: u16) {
        if !self.touched.switch {
            return;
        }
        let f = &mut self.form;
        if f.actuation > max {
            f.actuation = max;
            self.clamped = true;
        }
        if f.rt_press > max || f.rt_release > max {
            f.rt_press = f.rt_press.min(max);
            f.rt_release = f.rt_release.min(max);
            self.clamped = true;
        }
    }

    fn apply_area(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let idle = ctx.can_write() && self.pending.is_none();
        if let Some(err) = self.error.clone() {
            let retry = self.last_sent.is_some() && idle;
            let clicked = card::error_box(
                ui,
                t::APPLY_FAILED,
                &err,
                retry.then_some(i18n::common::RETRY),
                self.error_technical.as_deref(),
            );
            if clicked {
                if let Some(plan) = self.last_sent.clone() {
                    self.submit(ctx, plan);
                }
            }
            ui.add_space(8.0);
        }

        let can_apply = idle && !self.selection.is_empty() && self.touched.any();
        if card::primary_button(ui, t::APPLY, can_apply).clicked() {
            let keys = self.ordered(ctx);
            let plan = match ctx.snapshot {
                Some(snap) => plan_changes(snap, &keys, &self.form, &self.touched),
                None => vec![(keys, build_change(&self.form, &self.touched))],
            };
            self.submit(ctx, plan);
        }
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(t::APPLYING)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            } else if self.applied && !self.touched.any() {
                ui.label(
                    RichText::new(t::APPLIED)
                        .font(font::small())
                        .color(color::SUCCESS),
                );
            } else if let Some(s) = ctx.snapshot {
                ui.label(
                    RichText::new(format!(
                        "{} {} · {}",
                        t::SAVED_IN_PROFILE,
                        s.profile + 1,
                        t::WORKS_WITHOUT_APP
                    ))
                    .font(font::small())
                    .color(color::MUTED),
                );
            }
        });
    }
}

/// Per-key decoration: actuation value, a stroke when it differs from the
/// factory default, and an "RT" badge for rapid-trigger keys.
fn decorate(snap: Option<&DeviceSnapshot>, code: u8) -> KeyDecoration {
    let Some(s) = snap.and_then(|s| s.keys.get(&code)) else {
        return KeyDecoration::default();
    };
    KeyDecoration {
        label2: Some(mm_short(s.actuation.0)),
        fill: None,
        stroke: (s.actuation.0 != DEFAULT_ACTUATION_UM).then_some(color::INFO.gamma_multiply(0.7)),
        badge: (s.mode.touch == TouchMode::RapidTrigger).then(|| ("RT".to_string(), color::ACCENT)),
    }
}

fn tool_button(ui: &mut Ui, text: &str, active: bool, enabled: bool) -> egui::Response {
    let (fill, stroke, text_c) = if active {
        (color::ACCENT_SOFT, color::ACCENT, color::ACCENT)
    } else {
        (color::BG, color::BORDER_STRONG, color::TEXT)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(text_c))
            .fill(fill)
            .stroke(line(1.0, stroke))
            .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
            .min_size(egui::vec2(0.0, 38.0)),
    )
}

/// "Label ............ [value]" on one row.
fn value_header(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(color::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), value);
    });
}

/// A deadzone box ("Atas  0.10"); returns true when changed.
fn dz_box(ui: &mut Ui, label: &str, value: &mut u16, enabled: bool) -> bool {
    let mut mm = f64::from(*value) / 1000.0;
    let mut edited = false;
    Frame::new()
        .fill(color::PANEL)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(color::MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    edited = ui
                        .add_enabled(
                            enabled,
                            egui::DragValue::new(&mut mm)
                                .range(0.0..=f64::from(MAX_DEADZONE_UM) / 1000.0)
                                .clamp_existing_to_range(false)
                                .speed(0.01)
                                .fixed_decimals(2),
                        )
                        .changed();
                });
            });
        });
    // Only a real edit counts: a stored 0.205 mm shown as "0.21" must not
    // be rounded and sent just because the box was drawn.
    if !edited {
        return false;
    }
    let um = dz_um(mm);
    if um != *value {
        *value = um;
        true
    } else {
        false
    }
}

/// Deadzone box value (mm) to µm, on the 0.01 mm grid, within 0..=1.0 mm.
fn dz_um(mm: f64) -> u16 {
    ((mm * 100.0).round() * 10.0).clamp(0.0, f64::from(MAX_DEADZONE_UM)) as u16
}

#[derive(Clone, Copy)]
enum Swatch {
    Selected,
    Plain,
    Changed,
    Rt,
}

fn legend_row(ui: &mut Ui, swatch: Swatch, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
        let p = ui.painter();
        let radius = CornerRadius::same(4);
        let (fill, stroke) = match swatch {
            Swatch::Selected => (color::ACCENT_SOFT, line(1.5, color::ACCENT)),
            Swatch::Plain => (color::KEY, line(1.0, color::BORDER_STRONG)),
            Swatch::Changed => (color::KEY, line(1.0, color::INFO.gamma_multiply(0.7))),
            Swatch::Rt => (
                color::ACCENT.gamma_multiply(0.18),
                line(0.0, Color32::TRANSPARENT),
            ),
        };
        p.rect(r, radius, fill, stroke, StrokeKind::Inside);
        if let Swatch::Rt = swatch {
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                "RT",
                egui::FontId::proportional(8.5),
                color::ACCENT,
            );
        }
        ui.label(RichText::new(text).color(color::TEXT));
    });
}

fn waiting_banner(ui: &mut Ui, ctx: &AppCtx) {
    let (title, body) = match ctx.status {
        crate::worker::ConnectionStatus::Error(e) => (i18n::device::ERROR, e.clone()),
        _ if ctx.busy => (
            i18n::common::LOADING_DEVICE,
            i18n::common::WAITING_DEVICE_BODY.to_string(),
        ),
        _ => (
            i18n::common::WAITING_DEVICE,
            i18n::common::WAITING_DEVICE_BODY.to_string(),
        ),
    };
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(title).color(color::ACCENT));
        ui.label(RichText::new(body).font(font::small()).color(color::MUTED));
    });
}

/// The travel bar: deadzones hatched at both ends, the actuation point
/// marked, a scale underneath. With `depth` (live view) the bar fills to the
/// key's current depth, marked with a white line and its value; otherwise it
/// fills to the actuation point.
fn paint_travel(ui: &mut Ui, form: &Form, max: u16, depth: Option<u16>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 134.0), Sense::hover());
    let p = ui.painter_at(rect.expand(2.0));
    let bar = Rect::from_min_size(rect.min + egui::vec2(0.0, 26.0), egui::vec2(width, 56.0));
    let span = f32::from(max.max(1));
    let x_of = |um: u16| bar.left() + bar.width() * (f32::from(um.min(max)) / span);

    p.rect(
        bar,
        CornerRadius::same(8),
        color::CASE,
        line(1.0, color::BORDER_STRONG),
        StrokeKind::Inside,
    );
    let ax = x_of(form.actuation);
    let fill_to = depth.map_or(ax, x_of);
    let fill = Rect::from_min_max(bar.min, egui::pos2(fill_to, bar.max.y));
    if fill.width() >= 1.0 {
        let strength = if depth.is_some_and(|d| d >= form.actuation) {
            0.55
        } else {
            0.28
        };
        p.rect_filled(
            fill,
            CornerRadius::same(8),
            color::ACCENT.gamma_multiply(strength),
        );
    }

    let hatch = |from: f32, to: f32| {
        let zone = Rect::from_min_max(egui::pos2(from, bar.top()), egui::pos2(to, bar.bottom()));
        if zone.width() < 1.0 {
            return;
        }
        let hp = p.with_clip_rect(zone);
        let mut x = zone.left() - zone.height();
        while x < zone.right() {
            hp.line_segment(
                [
                    egui::pos2(x, zone.bottom()),
                    egui::pos2(x + zone.height(), zone.top()),
                ],
                line(1.5, color::MUTED.gamma_multiply(0.45)),
            );
            x += 7.0;
        }
    };
    hatch(bar.left(), x_of(form.top_dz));
    hatch(x_of(max.saturating_sub(form.bottom_dz)), bar.right());

    p.line_segment(
        [
            egui::pos2(ax, bar.top() - 8.0),
            egui::pos2(ax, bar.bottom() + 8.0),
        ],
        line(2.0, color::ACCENT),
    );
    p.text(
        egui::pos2(ax, rect.top()),
        Align2::CENTER_TOP,
        format!("{} {}", t::ACTUATION_MARK, mm_short(form.actuation)),
        font::small(),
        color::ACCENT,
    );
    if let Some(d) = depth {
        let dx = x_of(d);
        p.line_segment(
            [egui::pos2(dx, bar.top()), egui::pos2(dx, bar.bottom())],
            line(2.5, color::TEXT),
        );
        p.text(
            egui::pos2(dx.max(bar.left() + 24.0), bar.bottom() + 4.0),
            Align2::CENTER_TOP,
            format!("{} mm", mm_short(d)),
            font::small(),
            color::TEXT,
        );
    }

    // Scale: every 0.5 mm (kept clear of the end label), plus the end;
    // pushed down a line when the live depth value sits under the bar.
    let scale_y = bar.bottom() + if depth.is_some() { 26.0 } else { 12.0 };
    let mut um = 0u16;
    while um < max && x_of(max) - x_of(um) > 64.0 {
        let align = if um == 0 {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_TOP
        };
        p.text(
            egui::pos2(x_of(um), scale_y),
            align,
            format!("{:.1}", f32::from(um) / 1000.0),
            font::tiny(),
            color::FAINT,
        );
        um += 500;
    }
    p.text(
        egui::pos2(bar.right(), scale_y),
        Align2::RIGHT_TOP,
        format!("{} mm", mm_short(max)),
        font::tiny(),
        color::FAINT,
    );

    ui.add_space(8.0);
    let mut summary = format!(
        "{} {} · {} {}",
        t::DZ_SUMMARY_TOP,
        mm_short(form.top_dz),
        t::DZ_SUMMARY_BOTTOM,
        mm_short(form.bottom_dz)
    );
    if form.rt_mode {
        let release = if form.rt_sync {
            form.rt_press
        } else {
            form.rt_release
        };
        summary.push_str(&format!(
            "   {} {} · {} {}",
            t::RT_SUMMARY,
            mm_short(release),
            t::RT_SUMMARY_PRESS,
            mm_short(form.rt_press)
        ));
    }
    ui.label(
        RichText::new(summary)
            .font(font::small())
            .color(color::MUTED),
    );
}

// ------------------------------------------------- live depth (Track F)

mod live_strings {
    pub const FAILED: &str = "Kedalaman live tidak terbaca";
    pub const PRESS: &str = "Tekan tombol";
    pub const TO_SEE_LIVE: &str = "untuk melihat secara live";
    pub const PRESS_ANY: &str = "Tekan tombol mana saja untuk melihat kedalamannya";
    pub const SWITCH_TO_LIVE: &str = "Pilih \"Kedalaman live\" untuk melihat gerak tombol";
    pub const FRAME_HINT: &str = "Data jarak tekan dari keyboard tidak dikenali.";
}

/// Poll period of the live view (the official app polls every 10 ms; one
/// RM6X21 frame is 6 reports, so this keeps the queue light).
pub const LIVE_INTERVAL: Duration = Duration::from_millis(40);

/// Live key depth: one quiet `travel()` job in flight at a time while the
/// live view is shown; stops on its own when the page or view is left. A
/// failed read pauses polling until "Coba lagi".
#[derive(Default)]
pub struct LiveDepth {
    pending: Option<Pending<TravelFrame>>,
    last: Option<Instant>,
    /// Current depth per key, µm.
    pub depth: BTreeMap<u8, u16>,
    /// Indonesian text and technical detail of the last failure.
    pub error: Option<(String, String)>,
}

impl LiveDepth {
    /// Whether a new poll should go out now.
    pub fn due(&self, active: bool, now: Instant) -> bool {
        active
            && self.pending.is_none()
            && self.error.is_none()
            && self
                .last
                .is_none_or(|t| now.duration_since(t) >= LIVE_INTERVAL)
    }

    fn pump(&mut self, ctx: &AppCtx, active: bool, egui_ctx: &egui::Context) {
        if let Some(r) = self.pending.as_mut().and_then(Pending::poll) {
            self.pending = None;
            match (r, ctx.snapshot) {
                (Ok(frame), Some(s)) => self.apply(s, &frame),
                (Ok(_), None) => {}
                (Err(e), _) => self.fail(&e),
            }
        }
        let now = Instant::now();
        if self.due(active, now) {
            self.last = Some(now);
            self.pending = Some(ctx.worker.submit_quiet("live-depth", |kb| kb.travel()));
        }
        if active {
            egui_ctx.request_repaint_after(LIVE_INTERVAL);
        }
    }

    /// Stores one frame: the depth of every matrix key at its DEFKEY
    /// position. A frame with travel where the matrix has no key (halves
    /// mixed up, see `TravelFrame::fits`) is skipped.
    pub fn apply(&mut self, snap: &DeviceSnapshot, frame: &TravelFrame) {
        if !frame.fits(&snap.matrix) {
            return;
        }
        for (row, col, code) in snap.matrix.keys() {
            if let Some(v) = frame
                .um
                .get(usize::from(row))
                .and_then(|r| r.get(usize::from(col)))
            {
                self.depth.insert(code, *v);
            }
        }
    }

    pub fn fail(&mut self, e: &Error) {
        let msg = match e {
            Error::Io(_) | Error::Frame(_) => {
                format!("{} {}", live_strings::FRAME_HINT, i18n::device_error(e))
            }
            other => i18n::device_error(other),
        };
        self.error = Some((msg, i18n::technical(e)));
        // A disconnect needs no retry click: the next connection resumes.
        if *e == Error::Disconnected {
            self.error = None;
            self.depth.clear();
        }
    }

    pub fn retry(&mut self) {
        self.error = None;
        self.last = None;
    }

    /// The deepest key among `keys` (the first one on a tie).
    pub fn deepest(&self, keys: &[u8]) -> Option<(u8, u16)> {
        keys.iter()
            .map(|&c| (c, self.depth.get(&c).copied().unwrap_or(0)))
            .fold(None, |best: Option<(u8, u16)>, (c, d)| match best {
                Some((_, bd)) if bd >= d => best,
                _ => Some((c, d)),
            })
    }
}

/// Live view decoration: the depth in mm under the legend, the key tinted by
/// how deep it is, and an accent border once it is past its actuation point.
fn live_decoration(snap: Option<&DeviceSnapshot>, live: &LiveDepth, code: u8) -> KeyDecoration {
    let Some(s) = snap else {
        return KeyDecoration::default();
    };
    let depth = live.depth.get(&code).copied().unwrap_or(0);
    let (actuation, max) = match s.keys.get(&code) {
        Some(k) => (k.actuation.0, switch_max(s.model, k.switch)),
        None => (DEFAULT_ACTUATION_UM, SAFE_MAX_TRAVEL_UM),
    };
    let frac = f32::from(depth.min(max)) / f32::from(max.max(1));
    KeyDecoration {
        label2: Some(mm_short(depth)),
        fill: (depth > 0).then(|| {
            color::KEY.lerp_to_gamma(Color32::from_rgb(0x5A, 0x40, 0x10), frac.clamp(0.0, 1.0))
        }),
        stroke: (depth > 0 && depth >= actuation).then_some(color::ACCENT),
        badge: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_proto::key::{Advanced, KeyMode};
    use openhe_proto::{DeviceInfo, Matrix, PollingRate};
    use std::collections::BTreeMap;

    fn settings(touch: TouchMode, actuation: u16, switch: u16) -> KeySettings {
        KeySettings {
            mode: KeyMode {
                touch,
                advanced: Advanced::None,
                high: 0,
            },
            actuation: Micrometers(actuation),
            rt_press: Micrometers(150),
            rt_release: Micrometers(150),
            top_dz: Micrometers(100),
            bottom_dz: Micrometers(200),
            switch,
        }
    }

    fn snap(keys: &[(u8, KeySettings)]) -> DeviceSnapshot {
        DeviceSnapshot {
            model: Model::Win68HeMax,
            info: DeviceInfo {
                board_id: 0,
                in_bootloader: false,
                serial: String::new(),
                firmware: String::new(),
                build_date: String::new(),
            },
            system_mode: None,
            profile: 0,
            polling: PollingRate::Hz1000,
            matrix: Matrix::default(),
            keys: keys.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn only_touched_fields_are_sent() {
        let form = Form {
            rt_mode: true,
            actuation: 400,
            rt_press: 150,
            rt_release: 300,
            rt_sync: false,
            ..Form::default()
        };
        assert_eq!(
            build_change(&form, &Touched::default()),
            PerformanceChange::default()
        );
        let c = build_change(
            &form,
            &Touched {
                actuation: true,
                ..Default::default()
            },
        );
        assert_eq!(
            c,
            PerformanceChange {
                actuation: Some(Micrometers(400)),
                ..Default::default()
            }
        );
        let c = build_change(
            &form,
            &Touched {
                mode: true,
                rt: true,
                switch: true,
                ..Default::default()
            },
        );
        assert_eq!(c.touch, Some(TouchMode::RapidTrigger));
        assert_eq!(c.rt_press, Some(Micrometers(150)));
        assert_eq!(c.rt_release, Some(Micrometers(300)));
        assert_eq!(c.switch, Some(0));
        assert_eq!(c.actuation, None);
    }

    #[test]
    fn synced_rt_sends_press_for_both() {
        let form = Form {
            rt_press: 120,
            rt_release: 900,
            rt_sync: true,
            ..Form::default()
        };
        let c = build_change(
            &form,
            &Touched {
                rt: true,
                ..Default::default()
            },
        );
        assert_eq!(c.rt_release, Some(Micrometers(120)));
    }

    #[test]
    fn normal_mode_sends_single() {
        let form = Form::default();
        let c = build_change(
            &form,
            &Touched {
                mode: true,
                ..Default::default()
            },
        );
        assert_eq!(c.touch, Some(TouchMode::Single));
    }

    #[test]
    fn max_travel_is_the_shortest_switch_or_the_chosen_one() {
        // Switch 0 Wing Chun 3.4 mm, switch 3 Magneto 3.3 mm (WIN 68).
        let s = snap(&[
            (0x1A, settings(TouchMode::Single, 400, 0)),
            (0x04, settings(TouchMode::Single, 400, 3)),
        ]);
        let f = Form::default();
        assert_eq!(max_travel(&s, &[0x1A], &f, &Touched::default()), 3400);
        assert_eq!(max_travel(&s, &[0x1A, 0x04], &f, &Touched::default()), 3300);
        let chosen = Form {
            switch: 0,
            ..Form::default()
        };
        let touched = Touched {
            switch: true,
            ..Default::default()
        };
        assert_eq!(max_travel(&s, &[0x04], &chosen, &touched), 3400);
    }

    #[test]
    fn form_from_the_first_key_and_mixed_flag() {
        let s = snap(&[
            (0x04, settings(TouchMode::RapidTrigger, 400, 0)),
            (0x1A, settings(TouchMode::Global, 2000, 0)),
        ]);
        let f = Form::from_keys(&[0x04], &s).unwrap();
        assert!(f.rt_mode && !f.global && !f.mixed);
        assert_eq!(f.actuation, 400);
        assert!(f.rt_sync);
        let f = Form::from_keys(&[0x1A, 0x04], &s).unwrap();
        assert!(f.global && f.mixed);
        assert_eq!(Form::from_keys(&[0x29], &s), None);
    }

    /// Drives the page headlessly: load the WASD form, apply while the
    /// worker has no keyboard, and check the inline error and retry state.
    #[test]
    fn apply_without_a_keyboard_shows_an_inline_error_and_keeps_edits() {
        use crate::app::test_support::*;
        use crate::storage::AppData;
        use crate::theme::Theme;
        use crate::worker::ConnectionStatus;
        use std::cell::RefCell;

        let worker = offline_worker();
        let snap = snapshot68();
        worker.inject_status(ConnectionStatus::Connected(snap.clone()));
        let theme = Theme::default();
        let data = RefCell::new(AppData::default());
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut page = Page::default();
        page.selection.select_wasd(&snap.matrix);

        let frame = |page: &mut Page, n: usize| {
            run_frames(&ctx, n, |c| {
                let status = worker.status();
                let app_ctx = AppCtx {
                    worker: &worker,
                    snapshot: match &status {
                        ConnectionStatus::Connected(s) => Some(s),
                        _ => None,
                    },
                    status: &status,
                    busy: worker.is_busy(),
                    theme: &theme,
                    data: &data,
                };
                egui::CentralPanel::default().show(c, |ui| page.ui(&app_ctx, ui));
            })
        };
        frame(&mut page, 2);
        assert_eq!(page.form.actuation, 400);
        assert!(page.form.rt_mode);
        assert!(!page.touched.any());

        page.form.actuation = 550;
        page.touched.actuation = true;
        let change = build_change(&page.form, &page.touched);
        {
            let status = worker.status();
            let app_ctx = AppCtx {
                worker: &worker,
                snapshot: None,
                status: &status,
                busy: false,
                theme: &theme,
                data: &data,
            };
            page.submit(&app_ctx, vec![(page.selection.codes(), change.clone())]);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while page.pending.is_some() {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut page, 1);
        }
        assert_eq!(page.error.as_deref(), Some(t::PARTIAL_WRITE));
        assert_eq!(
            page.last_sent,
            Some(vec![(vec![0x04, 0x07, 0x16, 0x1A], change)])
        );
        assert!(page.touched.actuation, "edits survive a failed apply");
        assert_eq!(page.form.actuation, 550);
        frame(&mut page, 2); // the error box renders without panicking
    }

    // ---- apply job and its effects, against MockTransport

    use openhe_device::{MockTransport, Transport};
    use openhe_proto::frame::FrameError;

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn fail_reply() -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xFF;
        raw
    }

    /// The seven replies of `key_settings`: Global, given actuation.
    fn push_settings(m: &mut MockTransport, key: u8, actuation: u16) {
        for (layout, v) in [
            (8, 0x0010),
            (4, actuation),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, 0),
        ] {
            m.push_reply(key_reply(key, layout, v));
        }
    }

    fn actuation(um: u16) -> PerformanceChange {
        PerformanceChange {
            actuation: Some(Micrometers(um)),
            ..Default::default()
        }
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn verify_failure_still_rereads_and_patches() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x1A, 8, 0x0010)); // Mode (Single)
        m.push_reply(key_reply(0x1A, 25, 0)); // Switch
        m.push_reply(key_reply(0x1A, 4, 500)); // SET echo
        m.push_reply(key_reply(0x1A, 4, 2000)); // verify: did not stick
        push_settings(&mut m, 0x1A, 2000); // re-read
        let out = apply_job(&mut kb(m), &[0x1A], &actuation(500)).unwrap();
        assert!(matches!(out.write, Err(Error::Io(ref s)) if s.starts_with("verify failed")));
        assert_eq!(out.reloaded.len(), 1);
        assert_eq!(out.reloaded[0].1.actuation, Micrometers(2000));
        assert_eq!(out.reload_error, None);

        let fx = apply_effects(Ok(out));
        assert!(!fx.applied);
        assert!(!fx.request_reload);
        assert_eq!(fx.patch.len(), 1, "the true value replaces the cache");
        assert_eq!(
            fx.error.as_deref(),
            Some(format!("{} {}", i18n::error::VERIFY, t::RELOADED_HINT).as_str())
        );
        assert!(fx.technical.unwrap().contains("verify failed"));
    }

    #[test]
    fn a_failing_reread_asks_for_a_full_reload() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x1A, 8, 0x0010));
        m.push_reply(key_reply(0x1A, 25, 0));
        m.push_reply(key_reply(0x1A, 4, 500)); // SET echo
        m.push_reply(key_reply(0x1A, 4, 500)); // verify ok
                                               // Re-read: no replies, so it times out.
        let out = apply_job(&mut kb(m), &[0x1A], &actuation(500)).unwrap();
        assert_eq!(out.write, Ok(()));
        assert!(out.reloaded.is_empty());
        assert_eq!(out.reload_error, Some(Error::Timeout));

        let fx = apply_effects(Ok(out));
        assert!(fx.applied);
        assert!(fx.request_reload);
        assert_eq!(fx.error, None);
    }

    #[test]
    fn a_batch_failing_halfway_rereads_every_key() {
        // 15 keys: two SET packets (14 + 1); the keyboard rejects the second.
        let keys: Vec<u8> = (0x04..0x04 + 15).collect();
        let mut m = MockTransport::new();
        for &k in &keys {
            m.push_reply(key_reply(k, 8, 0x0010));
            m.push_reply(key_reply(k, 25, 0));
        }
        m.push_reply(key_reply(0x04, 4, 800)); // packet 1 echo
        m.push_reply(fail_reply()); // packet 2 rejected
        for &k in &keys {
            let written = k != *keys.last().unwrap();
            push_settings(&mut m, k, if written { 800 } else { 2000 });
        }
        let out = apply_job(&mut kb(m), &keys, &actuation(800)).unwrap();
        assert_eq!(out.write, Err(Error::Frame(FrameError::DeviceFail)));
        assert_eq!(out.reloaded.len(), 15);
        assert_eq!(out.reloaded[0].1.actuation, Micrometers(800));
        assert_eq!(out.reloaded[14].1.actuation, Micrometers(2000));

        let fx = apply_effects(Ok(out));
        assert!(!fx.applied);
        assert_eq!(fx.patch.len(), 15, "the partial state is shown as it is");
        assert!(fx.error.unwrap().starts_with(i18n::error::FRAME));
    }

    /// Fails every call after `n` writes, like a keyboard pulled mid-batch.
    struct UnplugAfter {
        inner: MockTransport,
        writes_left: usize,
    }

    impl Transport for UnplugAfter {
        fn write(&mut self, r: &[u8; 64]) -> Result<(), Error> {
            if self.writes_left == 0 {
                return Err(Error::Disconnected);
            }
            self.writes_left -= 1;
            self.inner.write(r)
        }
        fn read(&mut self, t: u32) -> Result<Option<[u8; 64]>, Error> {
            self.inner.read(t)
        }
    }

    #[test]
    fn unplugged_mid_batch_is_disconnected_with_a_partial_write_message() {
        let keys: Vec<u8> = (0x04..0x04 + 15).collect();
        let mut m = MockTransport::new();
        for &k in &keys {
            m.push_reply(key_reply(k, 8, 0x0010));
            m.push_reply(key_reply(k, 25, 0));
        }
        m.push_reply(key_reply(0x04, 4, 800)); // packet 1 echo
                                               // 30 reads + 1 SET go through; the second SET fails.
        let t = UnplugAfter {
            inner: m,
            writes_left: 31,
        };
        let mut k = Keyboard::new(t, Model::Win68HeMax);
        let r = apply_job(&mut k, &keys, &actuation(800));
        assert_eq!(r, Err(Error::Disconnected));
        let fx = apply_effects(r);
        assert_eq!(fx.error.as_deref(), Some(t::PARTIAL_WRITE));
        assert!(!fx.applied && fx.patch.is_empty());
    }

    #[test]
    fn other_job_errors_reload_everything() {
        let fx = apply_effects(Err(Error::Io(format!(
            "{}: boom",
            crate::worker::INTERNAL_ERROR
        ))));
        assert!(fx.request_reload);
        assert_eq!(fx.error.as_deref(), Some(i18n::error::INTERNAL));
    }

    #[test]
    fn error_texts_have_no_english() {
        let all = [
            Error::Timeout,
            Error::Disconnected,
            Error::Permission("denied by udev".into()),
            Error::Unsupported("bootloader".into()),
            Error::Unsupported("no supported keyboard was found".into()),
            Error::Frame(FrameError::DeviceFail),
            Error::InvalidInput("key 0x1A: actuation 3400 um is outside".into()),
            Error::Io("verify failed: key 0x1A".into()),
            Error::Io("unexpected reply".into()),
        ];
        for e in all {
            let s = i18n::device_error(&e);
            for word in [
                "key 0x",
                "um is",
                "failed",
                "denied",
                "unexpected",
                "timed out",
            ] {
                assert!(!s.contains(word), "{e:?} -> {s}");
            }
        }
    }

    #[test]
    fn deadzone_grid() {
        assert_eq!(dz_um(0.205), 210);
        assert_eq!(dz_um(0.2), 200);
        assert_eq!(dz_um(1.7), 1000);
    }

    #[test]
    fn switch_change_clamps_and_sends_values_beyond_the_new_travel() {
        let mut page = Page {
            form: Form {
                actuation: 3400,
                rt_press: 3400,
                rt_release: 100,
                ..Form::default()
            },
            ..Page::default()
        };
        // Without a switch change nothing is clamped or sent.
        page.clamp_to(3300);
        assert_eq!(page.form.actuation, 3400);
        assert!(!page.touched.any() && !page.clamped);
        // After choosing a 3.3 mm switch the form shows the clamped values;
        // they are sent per key by `plan_changes`, not as edits.
        page.touched.switch = true;
        page.clamp_to(3300);
        assert_eq!((page.form.actuation, page.form.rt_press), (3300, 3300));
        assert!(page.clamped);
        assert!(!page.touched.actuation && !page.touched.rt);
    }

    #[test]
    fn a_switch_change_clamps_each_keys_own_values() {
        let mut long = settings(TouchMode::RapidTrigger, 3400, 0);
        long.rt_release = Micrometers(3400);
        let short = settings(TouchMode::Single, 1000, 0);
        let s = snap(&[(0x14, short), (0x1A, long), (0x04, short)]);
        let form = Form {
            switch: 3, // Magneto, 3.3 mm
            ..Form::from_keys(&[0x14], &s).unwrap()
        };
        // Only the switch: Q and A keep their values, W is clamped.
        let touched = Touched {
            switch: true,
            ..Touched::default()
        };
        let plan = plan_changes(&s, &[0x14, 0x1A, 0x04], &form, &touched);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].0, vec![0x14, 0x04]);
        assert_eq!(plan[0].1.actuation, None);
        assert_eq!(plan[0].1.switch, Some(3));
        assert_eq!(plan[1].0, vec![0x1A]);
        assert_eq!(plan[1].1.actuation, Some(Micrometers(3300)));
        assert_eq!(plan[1].1.rt_press, None, "0.15 mm fits");
        assert_eq!(plan[1].1.rt_release, Some(Micrometers(3300)));
        assert!(needs_clamp(&s, &[0x14, 0x1A], &form, &touched));
        assert!(!needs_clamp(&s, &[0x14, 0x04], &form, &touched));
        // An edited actuation goes to every key as it is; W's release is
        // still clamped.
        let touched = Touched {
            actuation: true,
            ..touched
        };
        let plan = plan_changes(&s, &[0x14, 0x1A], &form, &touched);
        assert_eq!(plan.len(), 2);
        assert!(plan
            .iter()
            .all(|(_, c)| c.actuation == Some(Micrometers(1000))));
        assert_eq!(plan[1].1.rt_release, Some(Micrometers(3300)));
        // Without a switch change there is one batch, nothing added.
        let plan = plan_changes(&s, &[0x14, 0x1A], &form, &Touched::default());
        assert_eq!(plan, vec![(vec![0x14, 0x1A], PerformanceChange::default())]);
    }

    #[test]
    fn decoration_marks_changed_and_rt_keys() {
        let s = snap(&[
            (0x04, settings(TouchMode::RapidTrigger, 400, 0)),
            (0x1A, settings(TouchMode::Global, 2000, 0)),
        ]);
        let a = decorate(Some(&s), 0x04);
        assert_eq!(a.label2.as_deref(), Some("0.40"));
        assert!(a.stroke.is_some());
        assert_eq!(a.badge.map(|b| b.0), Some("RT".to_string()));
        let w = decorate(Some(&s), 0x1A);
        assert_eq!(w.label2.as_deref(), Some("2.00"));
        assert!(w.stroke.is_none() && w.badge.is_none());
        assert_eq!(decorate(None, 0x04), KeyDecoration::default());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::app::test_support::snapshot68;

    fn frame(snap: &DeviceSnapshot, f: impl Fn(u8) -> u16) -> TravelFrame {
        let mut um = [[0u16; 21]; 6];
        for (r, c, code) in snap.matrix.keys() {
            um[usize::from(r)][usize::from(c)] = f(code);
        }
        TravelFrame { um }
    }

    #[test]
    fn frames_map_to_keys_by_matrix_position() {
        let snap = snapshot68();
        let mut l = LiveDepth::default();
        l.apply(&snap, &frame(&snap, |c| if c == 0x1A { 1290 } else { 0 }));
        assert_eq!(l.depth[&0x1A], 1290);
        assert_eq!(l.depth[&0x04], 0);
        assert_eq!(l.depth.len(), snap.matrix.len());
        // The deepest key, overall or among a selection.
        let all: Vec<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
        assert_eq!(l.deepest(&all), Some((0x1A, 1290)));
        assert_eq!(l.deepest(&[0x04, 0x16]), Some((0x04, 0)));
        assert_eq!(l.deepest(&[]), None);
    }

    #[test]
    fn one_poll_in_flight_paced_and_paused_after_an_error() {
        let t0 = Instant::now();
        let mut l = LiveDepth::default();
        assert!(!l.due(false, t0), "only while the live view is shown");
        assert!(l.due(true, t0));
        l.last = Some(t0);
        assert!(!l.due(true, t0 + Duration::from_millis(10)));
        assert!(l.due(true, t0 + LIVE_INTERVAL));
        // A frame that does not parse: a clear message, polling paused.
        l.fail(&Error::Io("RM6X21 answered with SUB 3".into()));
        let (msg, tech) = l.error.clone().unwrap();
        assert!(msg.starts_with(live_strings::FRAME_HINT), "{msg}");
        assert!(tech.contains("RM6X21"));
        assert!(!l.due(true, t0 + Duration::from_secs(9)));
        l.retry();
        assert!(l.due(true, t0 + Duration::from_secs(9)));
        // A disconnect needs no retry; old depths are dropped.
        l.depth.insert(0x1A, 500);
        l.fail(&Error::Disconnected);
        assert!(l.error.is_none());
        assert!(l.depth.is_empty());
    }

    #[test]
    fn keys_past_their_actuation_point_are_outlined() {
        let snap = snapshot68(); // W A S D at 0.40 mm, the rest at 2.00 mm
        let mut l = LiveDepth::default();
        l.apply(
            &snap,
            &frame(&snap, |c| match c {
                0x1A => 500,  // W, past 0.40
                0x0A => 1000, // G, short of 2.00
                _ => 0,
            }),
        );
        let w = live_decoration(Some(&snap), &l, 0x1A);
        assert_eq!(w.label2.as_deref(), Some("0.50"));
        assert_eq!(w.stroke, Some(color::ACCENT));
        assert!(w.fill.is_some());
        let g = live_decoration(Some(&snap), &l, 0x0A);
        assert_eq!(g.stroke, None);
        assert!(g.fill.is_some());
        let rest = live_decoration(Some(&snap), &l, 0x29);
        assert_eq!(rest.fill, None);
        assert_eq!(rest.label2.as_deref(), Some("0.00"));
        assert_eq!(live_decoration(None, &l, 0x1A), KeyDecoration::default());
    }
}

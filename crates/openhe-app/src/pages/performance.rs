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
use openhe_proto::model::MIN_ACTUATION_UM;
use openhe_proto::{Micrometers, Model, TouchMode};
use std::collections::BTreeSet;

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

/// What the apply job hands back.
pub struct ApplyOutcome {
    pub write: Result<(), Error>,
    /// Settings re-read after the write, for every key that could be read.
    pub reloaded: Vec<(u8, KeySettings)>,
    /// Set when re-reading failed; the page then asks for a full reload.
    pub reload_error: Option<Error>,
}

// ------------------------------------------------------------------ page

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
    /// The last change sent, for "Coba lagi".
    last_sent: Option<(Vec<u8>, PerformanceChange)>,
    error: Option<String>,
    applied: bool,
    view: View,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            selection: Selection::default(),
            form: Form::default(),
            touched: Touched::default(),
            loaded_from: None,
            pending: None,
            last_sent: None,
            error: None,
            applied: false,
            view: View::Actuation,
        }
    }
}

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        self.poll_pending(ctx);
        self.sync_form(ctx);
        page_with_right_panel(
            ui,
            "performance-right",
            self,
            |page, ui| page.center(ctx, ui),
            |page, ui| page.right(ctx, ui),
        );
    }

    fn poll_pending(&mut self, ctx: &AppCtx) {
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        let Some(result) = p.poll() else {
            return;
        };
        self.pending = None;
        match result {
            Ok(out) => {
                if !out.reloaded.is_empty() {
                    let fresh = out.reloaded;
                    ctx.worker.patch_snapshot(move |s| {
                        for (k, v) in fresh {
                            s.keys.insert(k, v);
                        }
                    });
                }
                if out.reload_error.is_some() {
                    ctx.worker.request_reload();
                }
                match out.write {
                    Ok(()) => {
                        self.error = None;
                        self.applied = true;
                        self.touched = Touched::default();
                        self.loaded_from = None; // reload the form from the device
                    }
                    Err(e) => {
                        self.error = Some(i18n::device_error(&e));
                        self.applied = false;
                    }
                }
            }
            Err(e) => {
                // Disconnected (or the worker is gone): the snapshot is
                // reloaded on reconnect; keep the edits for a retry.
                self.error = Some(i18n::device_error(&e));
                self.applied = false;
            }
        }
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
            self.error = None;
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

    fn submit(&mut self, ctx: &AppCtx, keys: Vec<u8>, change: PerformanceChange) {
        self.error = None;
        self.applied = false;
        self.last_sent = Some((keys.clone(), change.clone()));
        self.pending = Some(ctx.worker.submit("performance-apply", move |kb| {
            let write = kb.set_performance(&keys, &change);
            if write == Err(Error::Disconnected) {
                return Err(Error::Disconnected);
            }
            // Never trust the cache after a write: read the keys back.
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
        }));
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
                Segmented::new(&views)
                    .disable(View::Live)
                    .hover_text(t::LIVE_SOON)
                    .show(ui, &mut self.view);
            });
        });
        ui.add_space(12.0);

        let snap = ctx.snapshot;
        ui.vertical_centered(|ui| {
            KeyboardView::new(model, matrix).enabled(connected).show(
                ui,
                &mut self.selection,
                |code| decorate(snap, code),
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

    fn travel_card(&self, ctx: &AppCtx, ui: &mut Ui, h: f32) {
        Card::new().show(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), h - 42.0));
            let codes = self.ordered(ctx);
            ui.horizontal(|ui| {
                let title = match codes.first() {
                    Some(&c) => format!("{} · {}", t::TRAVEL, openhe_proto::keycodes::legend(c)),
                    None => t::TRAVEL.to_string(),
                };
                card::title(ui, title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(t::LIVE_SOON)
                            .font(font::small())
                            .color(color::MUTED),
                    );
                });
            });
            ui.add_space(12.0);
            let max = match ctx.snapshot {
                Some(s) if !codes.is_empty() => max_travel(s, &codes, &self.form, &self.touched),
                _ => 3400,
            };
            if ctx.snapshot.is_some() && !codes.is_empty() {
                paint_travel(ui, &self.form, max);
            } else {
                card::hint(ui, t::NOTHING_SELECTED_BODY);
            }
        });
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

    /// Keeps form values inside the limits of the current switch.
    fn clamp_to(&mut self, max: u16) {
        let f = &mut self.form;
        f.actuation = f.actuation.clamp(MIN_ACTUATION_UM, max);
        f.rt_press = f.rt_press.clamp(MIN_RT_UM, max);
        f.rt_release = f.rt_release.clamp(MIN_RT_UM, max);
        f.top_dz = f.top_dz.min(MAX_DEADZONE_UM);
        f.bottom_dz = f.bottom_dz.min(MAX_DEADZONE_UM);
    }

    fn apply_area(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let idle = ctx.can_write() && self.pending.is_none();
        if let Some(err) = self.error.clone() {
            let retry = self.last_sent.is_some() && idle;
            let clicked = card::error_box(
                ui,
                t::APPLY_FAILED,
                &format!("{err} {}", t::RELOADED_HINT),
                retry.then_some(i18n::common::RETRY),
            );
            if clicked {
                if let Some((keys, change)) = self.last_sent.clone() {
                    self.submit(ctx, keys, change);
                }
            }
            ui.add_space(8.0);
        }

        let can_apply = idle && !self.selection.is_empty() && self.touched.any();
        if card::primary_button(ui, t::APPLY, can_apply).clicked() {
            let keys = self.ordered(ctx);
            let change = build_change(&self.form, &self.touched);
            self.submit(ctx, keys, change);
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
    Frame::new()
        .fill(color::PANEL)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(color::MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled(
                        enabled,
                        egui::DragValue::new(&mut mm)
                            .range(0.0..=f64::from(MAX_DEADZONE_UM) / 1000.0)
                            .speed(0.01)
                            .fixed_decimals(2),
                    );
                });
            });
        });
    let um = ((mm * 100.0).round() * 10.0).clamp(0.0, f64::from(MAX_DEADZONE_UM)) as u16;
    if um != *value {
        *value = um;
        true
    } else {
        false
    }
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

/// A static travel bar: deadzones hatched at both ends, the actuation point
/// marked, a scale underneath. Live depth arrives with Track F.
fn paint_travel(ui: &mut Ui, form: &Form, max: u16) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 120.0), Sense::hover());
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
    let fill = Rect::from_min_max(bar.min, egui::pos2(ax, bar.max.y));
    p.rect_filled(
        fill,
        CornerRadius::same(8),
        color::ACCENT.gamma_multiply(0.28),
    );

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

    // Scale: every 0.5 mm (kept clear of the end label), plus the end.
    let mut um = 0u16;
    while um < max && x_of(max) - x_of(um) > 64.0 {
        let align = if um == 0 {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_TOP
        };
        p.text(
            egui::pos2(x_of(um), bar.bottom() + 12.0),
            align,
            format!("{:.1}", f32::from(um) / 1000.0),
            font::tiny(),
            color::FAINT,
        );
        um += 500;
    }
    p.text(
        egui::pos2(bar.right(), bar.bottom() + 12.0),
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
            page.submit(&app_ctx, page.selection.codes(), change.clone());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while page.pending.is_some() {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut page, 1);
        }
        assert_eq!(
            page.error.as_deref(),
            Some(i18n::common::DISCONNECTED_WRITE)
        );
        assert_eq!(page.last_sent, Some((vec![0x04, 0x07, 0x16, 0x1A], change)));
        assert!(page.touched.actuation, "edits survive a failed apply");
        assert_eq!(page.form.actuation, 550);
        frame(&mut page, 2); // the error box renders without panicking
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

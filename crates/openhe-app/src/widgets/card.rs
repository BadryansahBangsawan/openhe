//! Cards and small text helpers shared by pages.

use crate::theme::{color, font, line, size};
use eframe::egui::{self, Color32, CornerRadius, Frame, InnerResponse, Margin, RichText, Ui};

/// A rounded panel-coloured box with a 1 px border.
pub struct Card {
    fill: Color32,
    stroke: Color32,
    padding: i8,
}

impl Default for Card {
    fn default() -> Self {
        Card {
            fill: color::PANEL,
            stroke: color::BORDER,
            padding: 20,
        }
    }
}

impl Card {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fill(mut self, c: Color32) -> Self {
        self.fill = c;
        self
    }

    pub fn stroke(mut self, c: Color32) -> Self {
        self.stroke = c;
        self
    }

    pub fn padding(mut self, p: i8) -> Self {
        self.padding = p;
        self
    }

    pub fn frame(&self) -> Frame {
        Frame::new()
            .fill(self.fill)
            .stroke(line(1.0, self.stroke))
            .corner_radius(CornerRadius::same(size::CARD_RADIUS))
            .inner_margin(Margin::same(self.padding))
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        self.frame().show(ui, add)
    }
}

/// Bold-looking card title.
pub fn title(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text)
            .font(font::heading())
            .color(color::TEXT)
            .strong(),
    );
}

/// Section label ("Mode", "Deadzone").
pub fn section(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).font(font::body()).color(color::MUTED));
}

/// Small muted text that wraps.
pub fn hint(ui: &mut Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text).font(font::small()).color(color::MUTED)).wrap());
}

/// An inline error box with an optional retry button; returns true when
/// retry was clicked.
pub fn error_box(ui: &mut Ui, title: &str, detail: &str, retry: Option<&str>) -> bool {
    let mut clicked = false;
    Card::new()
        .fill(color::DANGER_SOFT)
        .stroke(color::DANGER.gamma_multiply(0.6))
        .padding(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).color(color::DANGER).strong());
            ui.add(
                egui::Label::new(RichText::new(detail).font(font::small()).color(color::TEXT))
                    .wrap(),
            );
            if let Some(r) = retry {
                if ui.button(r).clicked() {
                    clicked = true;
                }
            }
        });
    clicked
}

/// A full-width accent button with centred bold text ("Terapkan ke
/// keyboard"); greyed out and inert when disabled.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    let (fill, text_c) = if !enabled {
        (color::TRACK, color::FAINT)
    } else if resp.hovered() {
        (color::ACCENT.gamma_multiply(0.9), color::ON_ACCENT)
    } else {
        (color::ACCENT, color::ON_ACCENT)
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(size::CONTROL_RADIUS + 2), fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font::bold(15.0),
        text_c,
    );
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Label on the left, value on the right, in one row.
pub fn kv_row(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).color(color::MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).color(color::TEXT));
        });
    });
}

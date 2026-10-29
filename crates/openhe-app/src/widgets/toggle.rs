//! An on/off switch in the accent colour.

use crate::theme::color;
use eframe::egui::{self, Response, Sense, Ui};

/// Draws a 40×22 switch bound to `on`; `changed()` when flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let size = egui::vec2(40.0, 22.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, ""));
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track = if *on {
        color::ACCENT
    } else {
        color::BORDER_STRONG
    };
    let track = if enabled {
        track
    } else {
        track.gamma_multiply(0.5)
    };
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, track);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, color::TEXT);
    response
}

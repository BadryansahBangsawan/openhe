//! A millimetre slider (values held in micrometres) and a matching value box.

use crate::theme::{color, font, line, size};
use eframe::egui::{self, Align2, CornerRadius, Frame, Key, Margin, Response, Sense, Ui};
use std::ops::RangeInclusive;

/// "0.400" for 400 µm.
pub fn mm3(um: u16) -> String {
    format!("{:.3}", f64::from(um) / 1000.0)
}

/// "0.40" when the value is a whole hundredth, else "0.405".
pub fn mm_short(um: u16) -> String {
    if um % 10 == 0 {
        format!("{:.2}", f64::from(um) / 1000.0)
    } else {
        mm3(um)
    }
}

/// Snaps `v` to a multiple of `step`, then clamps it into `min..=max`.
pub fn snap(v: f64, step: u16, min: u16, max: u16) -> u16 {
    let step = f64::from(step.max(1));
    let snapped = (v / step).round() * step;
    snapped.clamp(f64::from(min), f64::from(max)).round() as u16
}

/// Horizontal slider over `min..=max` µm. Dragging snaps to `step` µm; the
/// arrow keys move one step when it has focus.
pub struct SliderMm<'a> {
    value: &'a mut u16,
    min: u16,
    max: u16,
    step: u16,
    enabled: bool,
    ends: Option<(String, String)>,
}

impl<'a> SliderMm<'a> {
    pub fn new(value: &'a mut u16, range: RangeInclusive<u16>) -> Self {
        SliderMm {
            value,
            min: *range.start(),
            max: (*range.end()).max(*range.start()),
            step: 10,
            enabled: true,
            ends: None,
        }
    }

    pub fn step(mut self, um: u16) -> Self {
        self.step = um.max(1);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Captions under both ends, e.g. "0.10 · sensitif" and "3.40 · dalam".
    pub fn end_labels(mut self, left: impl Into<String>, right: impl Into<String>) -> Self {
        self.ends = Some((left.into(), right.into()));
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let height = if self.ends.is_some() { 40.0 } else { 22.0 };
        let sense = if self.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, mut response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
        let track_y = rect.min.y + 11.0;
        let left = rect.min.x + 8.0;
        let right = rect.max.x - 8.0;
        let span = f64::from(self.max - self.min).max(1.0);

        if self.enabled {
            if let Some(p) = response.interact_pointer_pos() {
                if response.is_pointer_button_down_on() || response.clicked() {
                    response.request_focus();
                    let t = f64::from(((p.x - left) / (right - left)).clamp(0.0, 1.0));
                    let v = snap(
                        f64::from(self.min) + t * span,
                        self.step,
                        self.min,
                        self.max,
                    );
                    if v != *self.value {
                        *self.value = v;
                        response.mark_changed();
                    }
                }
            }
            if response.has_focus() {
                let (l, r) = ui.input(|i| {
                    (
                        i.key_pressed(Key::ArrowLeft),
                        i.key_pressed(Key::ArrowRight),
                    )
                });
                let mut v = *self.value;
                if l {
                    v = v.saturating_sub(self.step).max(self.min);
                }
                if r {
                    v = v.saturating_add(self.step).min(self.max);
                }
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
            }
        }

        let v = (*self.value).clamp(self.min, self.max);
        let t = (f64::from(v - self.min) / span) as f32;
        let thumb_x = left + t * (right - left);
        let painter = ui.painter();
        let (accent, track) = if self.enabled {
            (color::ACCENT, color::BORDER_STRONG)
        } else {
            (color::FAINT, color::BORDER)
        };
        let track_rect = egui::Rect::from_min_max(
            egui::pos2(left, track_y - 2.0),
            egui::pos2(right, track_y + 2.0),
        );
        painter.rect_filled(track_rect, CornerRadius::same(2), track);
        let fill_rect =
            egui::Rect::from_min_max(track_rect.min, egui::pos2(thumb_x, track_rect.max.y));
        painter.rect_filled(fill_rect, CornerRadius::same(2), accent);
        let c = egui::pos2(thumb_x, track_y);
        painter.circle_filled(c, 9.0, accent);
        painter.circle_filled(c, 5.0, color::TEXT);
        if response.has_focus() {
            painter.circle_stroke(c, 12.0, line(1.0, color::ACCENT_SOFT));
        }
        if let Some((l, r)) = &self.ends {
            let y = rect.max.y;
            painter.text(
                egui::pos2(left - 8.0, y),
                Align2::LEFT_BOTTOM,
                l,
                font::small(),
                color::MUTED,
            );
            painter.text(
                egui::pos2(right + 8.0, y),
                Align2::RIGHT_BOTTOM,
                r,
                font::small(),
                color::MUTED,
            );
        }
        response
    }
}

/// A framed value box ("0.400 mm") that edits µm with 0.001 mm precision.
pub fn mm_value_box(
    ui: &mut Ui,
    value: &mut u16,
    range: RangeInclusive<u16>,
    enabled: bool,
) -> Response {
    let (min, max) = (*range.start(), (*range.end()).max(*range.start()));
    let mut mm = f64::from(*value) / 1000.0;
    let inner = Frame::new()
        .fill(color::PANEL)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.add_enabled(
                enabled,
                egui::DragValue::new(&mut mm)
                    .range(f64::from(min) / 1000.0..=f64::from(max) / 1000.0)
                    // Show a value outside the range as it is (e.g. a key on
                    // another switch); only user edits are clamped.
                    .clamp_existing_to_range(false)
                    .speed(0.001)
                    .fixed_decimals(3)
                    .suffix(" mm"),
            )
        });
    let mut r = inner.inner;
    if r.changed() {
        let um = (mm * 1000.0).round().clamp(f64::from(min), f64::from(max)) as u16;
        if um != *value {
            *value = um;
        } else {
            // Dragged back to the same micrometre: nothing to send.
            r.flags.remove(egui::response::Flags::CHANGED);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(mm3(400), "0.400");
        assert_eq!(mm3(5), "0.005");
        assert_eq!(mm_short(1200), "1.20");
        assert_eq!(mm_short(1205), "1.205");
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(404.0, 10, 100, 3400), 400);
        assert_eq!(snap(406.0, 10, 100, 3400), 410);
        assert_eq!(snap(20.0, 10, 100, 3400), 100);
        assert_eq!(snap(9999.0, 10, 100, 3300), 3300);
        assert_eq!(snap(7.0, 10, 5, 3400), 10);
        assert_eq!(snap(2.0, 10, 5, 3400), 5);
    }
}

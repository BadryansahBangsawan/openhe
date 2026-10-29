//! Segmented control: a rounded track with one highlighted segment.

use crate::theme::{color, font, line, size};
use eframe::egui::{self, Align2, CornerRadius, Response, Sense, StrokeKind, Ui, WidgetText};

pub struct Segmented<'a, T: PartialEq + Copy> {
    options: &'a [(T, &'a str)],
    full_width: bool,
    enabled: bool,
    read_only: bool,
    disabled: Vec<T>,
    height: f32,
    hover: Option<&'a str>,
}

impl<'a, T: PartialEq + Copy> Segmented<'a, T> {
    pub fn new(options: &'a [(T, &'a str)]) -> Self {
        Segmented {
            options,
            full_width: false,
            enabled: true,
            read_only: false,
            disabled: Vec::new(),
            height: 36.0,
            hover: None,
        }
    }

    /// Stretch over the available width, segments sharing it equally.
    pub fn full_width(mut self, yes: bool) -> Self {
        self.full_width = yes;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Shows the value with full contrast but ignores clicks.
    pub fn read_only(mut self, yes: bool) -> Self {
        self.read_only = yes;
        self
    }

    /// Greys out one option (it cannot be picked).
    pub fn disable(mut self, option: T) -> Self {
        self.disabled.push(option);
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    pub fn hover_text(mut self, text: &'a str) -> Self {
        self.hover = Some(text);
        self
    }

    /// Draws the control; `changed()` is set when the user picked another
    /// option.
    pub fn show(self, ui: &mut Ui, current: &mut T) -> Response {
        let pad = 4.0;
        let seg_pad = 14.0;
        let galleys: Vec<_> = self
            .options
            .iter()
            .map(|(value, label)| {
                // The selected option is bold, as in the mockups.
                let font = if *value == *current {
                    font::bold(font::body().size)
                } else {
                    font::body()
                };
                WidgetText::from(*label).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::FontSelection::FontId(font),
                )
            })
            .collect();
        let natural: Vec<f32> = galleys.iter().map(|g| g.size().x + 2.0 * seg_pad).collect();
        let total_natural: f32 = natural.iter().sum::<f32>() + 2.0 * pad;
        let width = if self.full_width {
            ui.available_width().max(total_natural)
        } else {
            total_natural
        };
        let (rect, mut response) =
            ui.allocate_exact_size(egui::vec2(width, self.height), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect(
            rect,
            CornerRadius::same(size::CONTROL_RADIUS + 2),
            color::TRACK,
            line(1.0, color::BORDER),
            StrokeKind::Inside,
        );
        let inner_w = width - 2.0 * pad;
        let n = self.options.len().max(1) as f32;
        let mut x = rect.min.x + pad;
        for (i, ((value, _), galley)) in self.options.iter().zip(galleys).enumerate() {
            let w = if self.full_width {
                inner_w / n
            } else {
                natural[i]
            };
            let seg = egui::Rect::from_min_size(
                egui::pos2(x, rect.min.y + pad),
                egui::vec2(w, self.height - 2.0 * pad),
            );
            x += w;
            let option_enabled = self.enabled && !self.disabled.contains(value);
            let interactive = option_enabled && !self.read_only;
            let id = response.id.with(i);
            let seg_resp = ui.interact(
                seg,
                id,
                if interactive {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let selected = *value == *current;
            let label = self.options[i].1;
            seg_resp.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    interactive,
                    selected,
                    label,
                )
            });
            if seg_resp.clicked() && !selected {
                *current = *value;
                response.mark_changed();
            }
            if selected {
                painter.rect_filled(
                    seg,
                    CornerRadius::same(size::CONTROL_RADIUS),
                    color::SEGMENT_ON,
                );
            } else if seg_resp.hovered() && interactive {
                painter.rect_filled(
                    seg,
                    CornerRadius::same(size::CONTROL_RADIUS),
                    color::KEY_HOVER,
                );
            }
            let text_color = if !option_enabled {
                color::FAINT
            } else if selected {
                color::TEXT
            } else {
                color::MUTED
            };
            painter.galley_with_override_text_color(
                Align2::CENTER_CENTER
                    .align_size_within_rect(galley.size(), seg)
                    .min,
                galley,
                text_color,
            );
            if let Some(h) = self.hover {
                seg_resp.on_hover_text(h);
            }
        }
        response
    }
}

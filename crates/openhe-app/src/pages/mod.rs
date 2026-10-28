//! One module per sidebar page. Each exposes `pub struct Page` (built with
//! `Default`) and `pub fn ui(&mut self, ctx: &AppCtx, ui: &mut egui::Ui)`.
//! A page that wants the 348 pt right panel uses
//! [`crate::app::page_with_right_panel`].

pub mod advanced;
pub mod calibration;
pub mod lighting;
pub mod macros;
pub mod performance;
pub mod remap;
pub mod settings;
pub mod templates;

use crate::app::AppCtx;
use crate::i18n;
use crate::theme::{color, font, size};
use crate::widgets::card::Card;
use eframe::egui::{self, RichText};

/// Placeholder body for pages that are not built yet.
pub fn coming_soon(_ctx: &AppCtx, ui: &mut egui::Ui, title: &str) {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(size::PAGE_MARGIN as i8))
        .show(ui, |ui| {
            Card::new().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(title)
                        .font(font::heading())
                        .color(color::TEXT)
                        .strong(),
                );
                ui.label(
                    RichText::new(i18n::common::COMING_SOON)
                        .font(font::title())
                        .color(color::ACCENT),
                );
                ui.label(RichText::new(i18n::common::COMING_SOON_BODY).color(color::MUTED));
            });
        });
}

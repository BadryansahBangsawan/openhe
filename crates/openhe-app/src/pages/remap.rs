//! Remap page (4 layers). Track D, Wave 2.
//!
//! Stub: shows "Segera hadir" until its track lands. The app builds the page
//! with `Page::default()` and calls `ui` every frame while it is open.

use crate::app::AppCtx;
use crate::i18n;
use eframe::egui;

#[derive(Default)]
pub struct Page;

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut egui::Ui) {
        super::coming_soon(ctx, ui, i18n::nav::REMAP);
    }
}

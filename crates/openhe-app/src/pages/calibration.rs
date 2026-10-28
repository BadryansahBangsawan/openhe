//! Kalibrasi page. Track F, Wave 2.
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
        super::coming_soon(ctx, ui, i18n::nav::CALIBRATION);
    }

    /// Calibration is starting, running or stopping.
    pub fn is_active(&self) -> bool {
        false
    }

    /// Called when the user leaves the page or the app closes: stops a
    /// running calibration. Returns the stop job, if one was sent.
    pub fn leave(&mut self, _worker: &crate::worker::Worker) -> Option<crate::worker::Pending<()>> {
        None
    }
}

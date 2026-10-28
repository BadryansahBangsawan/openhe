//! OpenHE desktop app: egui shell, device worker and pages.

#![forbid(unsafe_code)]

pub mod app;
pub mod i18n;
pub mod pages;
pub mod storage;
pub mod theme;
pub mod widgets;
pub mod worker;

use theme::size;

/// Opens the main window. Blocks until it is closed.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(i18n::common::APP_NAME)
            .with_app_id("dev.openhe.OpenHE")
            .with_inner_size(size::START_WINDOW)
            .with_min_inner_size(size::MIN_WINDOW),
        ..Default::default()
    };
    eframe::run_native(
        i18n::common::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::OpenHeApp::new(cc)))),
    )
}

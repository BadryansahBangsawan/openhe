//! OpenHE desktop app: egui shell, device worker and pages.

#![forbid(unsafe_code)]

pub mod app;
pub mod i18n;
pub mod instance;
pub mod pages;
pub mod skin;
pub mod storage;
pub mod theme;
pub mod widgets;
pub mod worker;

use theme::size;

/// Opens the main window. Blocks until it is closed. A second instance
/// only says that OpenHE is already running and exits: two apps must never
/// talk to the keyboard at the same time.
pub fn run() -> eframe::Result {
    let _instance = match instance::acquire_default() {
        instance::Acquire::AlreadyRunning => {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title(i18n::common::ALREADY_RUNNING)
                .set_description(i18n::common::ALREADY_RUNNING_BODY)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            return Ok(());
        }
        // Held until the window closes.
        instance::Acquire::Acquired(lock) => Some(lock),
        // No usable lock file: run anyway rather than not at all.
        instance::Acquire::Unavailable(_) => None,
    };
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

//! OpenHE desktop app: egui shell, device worker and pages.

#![forbid(unsafe_code)]

pub mod app;
pub mod i18n;
pub mod instance;
pub mod pages;
pub mod skin;
pub mod storage;
pub mod templates;
pub mod theme;
pub mod update;
pub mod widgets;
pub mod worker;

use theme::size;

/// Opens the main window. Blocks until it is closed. A second instance
/// only says that OpenHE is already running and exits: two apps must never
/// talk to the keyboard at the same time.
///
/// Started by an update ([`update::AFTER_UPDATE_ARG`]), it first waits up
/// to [`instance::AFTER_UPDATE_WAIT`] for the old version to close.
pub fn run() -> eframe::Result {
    let after_update = std::env::args_os().any(|a| a == update::AFTER_UPDATE_ARG);
    let acquired = if after_update {
        instance::acquire_default_waiting(instance::AFTER_UPDATE_WAIT)
    } else {
        instance::acquire_default()
    };
    let _instance = match acquired {
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
    // What an earlier update left behind (the previous version, the
    // staging folder), quietly and off the UI thread.
    if let Ok(target) = update::install::current_target() {
        std::thread::spawn(move || update::install::cleanup_leftovers(&target));
    }
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(i18n::common::APP_NAME)
        .with_app_id("dev.openhe.OpenHE")
        .with_inner_size(size::START_WINDOW)
        .with_min_inner_size(size::MIN_WINDOW);
    // Without an icon of our own, eframe sets its default "e" icon at run
    // time, which on macOS also replaces the bundle's openhe.icns in the Dock.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/icons/png/openhe-512.png"
    )) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        i18n::common::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::OpenHeApp::new(cc)))),
    )
}

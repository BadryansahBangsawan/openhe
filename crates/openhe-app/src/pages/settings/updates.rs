//! Pengaturan → Pembaruan: the running version, checks (by hand and
//! automatic), the offered release with its notes, "Unduh & pasang" with
//! progress, and the restart into the new version. The work itself runs on
//! the updater's thread ([`crate::update::Updater`]); this is only the
//! card and the per-frame plumbing.

use super::{section_card, setting_row, Page};
use crate::app::AppCtx;
use crate::i18n::update as t;
use crate::theme::color;
use crate::update::install::{self, Unsupported};
use crate::update::updater::{Event, Offer};
use crate::update::{self, Phase, UpdateEnv, Updater};
use crate::widgets::card::{self, Card};
use crate::widgets::toggle::toggle;
use eframe::egui::{self, RichText, Ui};
use std::time::Duration;

/// Release notes longer than this are cut (the release page has them all).
pub const NOTES_MAX_CHARS: usize = 3000;

/// "baru saja", "5 menit lalu", "3 jam lalu", "2 hari lalu".
pub fn ago_text(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => t::JUST_NOW.to_string(),
        60..3600 => format!("{} {}", s / 60, t::MINUTES_AGO),
        3600..86_400 => format!("{} {}", s / 3600, t::HOURS_AGO),
        _ => format!("{} {}", s / 86_400, t::DAYS_AGO),
    }
}

/// Release notes as shown: no carriage returns, cut to
/// [`NOTES_MAX_CHARS`] (with "…"), or a placeholder when empty.
pub fn notes_text(notes: &str) -> String {
    let notes = notes.replace('\r', "");
    let notes = notes.trim();
    if notes.is_empty() {
        return t::NO_NOTES.to_string();
    }
    crate::app::elide_chars(notes, NOTES_MAX_CHARS)
}

/// Why this copy cannot install the update itself.
pub fn unsupported_text(u: Unsupported) -> &'static str {
    match u {
        Unsupported::NotInBundle => t::UNSUPPORTED_BUNDLE,
        Unsupported::Translocated => t::UNSUPPORTED_TRANSLOCATED,
        Unsupported::NotAppImage => t::UNSUPPORTED_APPIMAGE,
        Unsupported::Platform | Unsupported::NoExe => t::UNSUPPORTED_PLATFORM,
    }
}

/// Why "Mulai ulang sekarang" is disabled, if it is: never while the
/// keyboard is being written (closing waits for that anyway, but a restart
/// should not look stuck).
pub fn restart_blocked(writing: bool) -> Option<&'static str> {
    writing.then_some(t::RESTART_BUSY)
}

/// "Versi 0.3.0" (with "(versi uji)" for a pre-release).
fn offer_title(o: &Offer) -> String {
    let mut s = format!("{} v{}", t::AVAILABLE, o.version);
    if o.prerelease {
        s = format!("{s} ({})", t::PRERELEASE);
    }
    s
}

/// "12,3 / 40,0 MB".
fn megabytes(done: u64, total: u64) -> String {
    let mb = |b: u64| format!("{:.1}", b as f64 / 1_000_000.0).replace('.', ",");
    format!("{} / {} MB", mb(done), mb(total))
}

impl Page {
    /// Replaces the updater's environment (the app installs the real one,
    /// tests a fake).
    pub fn set_update_env(&mut self, env: UpdateEnv) {
        self.updates.set_env(env);
    }

    pub fn updater(&self) -> &Updater {
        &self.updates
    }

    /// The version for the sidebar's notice: an update was found and its
    /// notice was not dismissed (`dismissed`, from the data file).
    pub fn update_notice(&self, dismissed: Option<&str>) -> Option<String> {
        let v = self.updates.offer()?.version.to_string();
        (dismissed != Some(v.as_str())).then_some(v)
    }

    /// Every frame: the run's automatic check (once, if due), and results
    /// of the updater's thread. A finished check is stored.
    pub(super) fn pump_updates(&mut self, ctx: &AppCtx, egui_ctx: &egui::Context) {
        let prefs = ctx.data.borrow().update.clone();
        let now = update::unix_now();
        self.updates
            .auto_check(prefs.auto_check, prefs.beta, prefs.last_check, now);
        if self.updates.pump() == Some(Event::Checked) {
            ctx.data.borrow_mut().update.last_check = Some(now);
            // Only a convenience: a failed save is not worth a notice.
            let _ = ctx.save_data();
        }
        if self.updates.is_busy() {
            egui_ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(super) fn update_section(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let prefs = ctx.data.borrow().update.clone();
        let now = update::unix_now();
        section_card(ui, t::TITLE, |ui| {
            let last = match prefs.last_check {
                Some(then) => format!("{}: {}", t::LAST_CHECK, ago_text(now, then)),
                None => t::NEVER_CHECKED.to_string(),
            };
            let version = format!("{}: {}", t::VERSION, update::CURRENT_VERSION);
            let busy = self.updates.is_busy();
            let installed = self.updates.phase() == Phase::Installed;
            setting_row(ui, &version, Some(&last), true, |ui| {
                if ui
                    .add_enabled(!busy && !installed, egui::Button::new(t::CHECK_NOW))
                    .clicked()
                {
                    self.updates.check_now(prefs.beta);
                }
            });
            self.update_status(ctx, ui);

            setting_row(ui, t::AUTO, None, true, |ui| {
                let mut on = prefs.auto_check;
                if toggle(ui, &mut on, true).changed() {
                    ctx.data.borrow_mut().update.auto_check = on;
                    self.save_update_prefs(ctx);
                }
            });
            setting_row(ui, t::BETA, Some(t::BETA_HINT), true, |ui| {
                let mut on = prefs.beta;
                if toggle(ui, &mut on, true).changed() {
                    ctx.data.borrow_mut().update.beta = on;
                    self.save_update_prefs(ctx);
                }
            });
            if ui.link(t::RELEASES_LINK).clicked() {
                self.open_link(update::RELEASES_PAGE);
            }
            if let Some(e) = &self.link_error {
                ui.add_space(4.0);
                card::hint(ui, e);
            }
        });
    }

    fn save_update_prefs(&mut self, ctx: &AppCtx) {
        if ctx.save_data().is_err() {
            self.app_error = Some(super::t::SAVE_FAILED.to_string());
        }
    }

    fn open_link(&mut self, url: &str) {
        self.link_error = install::open_url(url)
            .err()
            .map(|_| format!("{} {url}", t::LINK_FAILED));
    }

    /// The check's result, the offer, the progress and failures.
    fn update_status(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let phase = self.updates.phase();
        match phase {
            Phase::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t::CHECKING).color(color::ACCENT));
                });
                ui.add_space(8.0);
            }
            Phase::Checked if self.updates.offer().is_none() => {
                ui.label(RichText::new(format!("✔ {}", t::UP_TO_DATE)).color(color::SUCCESS));
                ui.add_space(8.0);
            }
            _ => {}
        }
        if let Some((text, tech, install)) = self.updates.error() {
            let title = if install {
                t::INSTALL_FAILED
            } else {
                t::CHECK_FAILED
            };
            card::error_box(ui, title, text, None, Some(tech));
            ui.add_space(8.0);
        }
        let Some(offer) = self.updates.offer().cloned() else {
            return;
        };
        Card::new()
            .fill(color::ACCENT_SOFT)
            .stroke(color::ACCENT.gamma_multiply(0.5))
            .padding(14)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(offer_title(&offer))
                        .color(color::ACCENT)
                        .strong(),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("update-notes")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(notes_text(&offer.notes)).color(color::TEXT),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(10.0);
                self.offer_action(ctx, ui, &offer, phase);
            });
        ui.add_space(12.0);
    }

    /// Install, progress, restart, or the manual download link.
    fn offer_action(&mut self, ctx: &AppCtx, ui: &mut Ui, offer: &Offer, phase: Phase) {
        match phase {
            Phase::Installed => {
                ui.label(
                    RichText::new(format!("✔ v{} {}", offer.version, t::INSTALLED))
                        .color(color::SUCCESS),
                );
                ui.add_space(8.0);
                let writing = ctx.busy || ctx.worker.calibration_guard() || !self.idle();
                let blocked = restart_blocked(writing);
                let r = card::primary_button(ui, t::RESTART, blocked.is_none());
                if let Some(why) = blocked {
                    r.on_hover_text(why);
                } else if r.clicked() && self.updates.restart() {
                    // The new version waits for this one to let go of the
                    // keyboard and the instance lock; closing runs the
                    // usual exit (names saved, calibration stopped).
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Phase::Downloading { done, total } => {
                ui.label(RichText::new(t::DOWNLOADING).color(color::ACCENT));
                let frac = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                ui.add(
                    egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                        .fill(color::ACCENT)
                        .text(megabytes(done, total)),
                );
            }
            Phase::Installing => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t::INSTALLING).color(color::ACCENT));
                });
            }
            _ if self.updates.can_install() => {
                let enabled = phase == Phase::Checked;
                if card::primary_button(ui, t::INSTALL, enabled).clicked() {
                    self.updates.install();
                }
            }
            _ => {
                let why = match &self.updates.env().target {
                    Err(u) => unsupported_text(*u),
                    Ok(_) => t::NO_ASSET,
                };
                card::hint(ui, why);
                ui.add_space(6.0);
                if ui.button(t::DOWNLOAD_PAGE).clicked() {
                    self.open_link(&offer.page());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_check_reads_as_time_ago() {
        assert_eq!(ago_text(1_000, 1_000), "baru saja");
        assert_eq!(ago_text(1_059, 1_000), "baru saja");
        assert_eq!(ago_text(1_000 + 5 * 60, 1_000), "5 menit lalu");
        assert_eq!(ago_text(1_000 + 3 * 3600 + 59, 1_000), "3 jam lalu");
        assert_eq!(ago_text(1_000 + 2 * 86_400, 1_000), "2 hari lalu");
        // A clock that went back.
        assert_eq!(ago_text(500, 1_000), "baru saja");
    }

    #[test]
    fn release_notes_are_cut_and_never_empty() {
        assert_eq!(notes_text("  \r\n "), t::NO_NOTES);
        assert_eq!(notes_text("- a\r\n- b\r\n"), "- a\n- b");
        let long = "x".repeat(NOTES_MAX_CHARS + 50);
        let cut = notes_text(&long);
        assert_eq!(cut.chars().count(), NOTES_MAX_CHARS);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_restart_waits_for_keyboard_writes() {
        assert_eq!(restart_blocked(true), Some(t::RESTART_BUSY));
        assert_eq!(restart_blocked(false), None);
    }

    #[test]
    fn download_sizes_read_in_megabytes() {
        assert_eq!(megabytes(12_340_000, 40_000_000), "12,3 / 40,0 MB");
    }

    #[test]
    fn every_unsupported_case_explains_itself() {
        for u in [
            Unsupported::NotInBundle,
            Unsupported::Translocated,
            Unsupported::NotAppImage,
            Unsupported::Platform,
            Unsupported::NoExe,
        ] {
            assert!(!unsupported_text(u).is_empty());
        }
    }

    #[test]
    fn the_sidebar_notice_can_be_dismissed_per_version() {
        let mut page = Page::default();
        assert_eq!(page.update_notice(None), None);
        let offer = Offer {
            tag: "v0.3.0".into(),
            version: semver::Version::new(0, 3, 0),
            prerelease: false,
            notes: String::new(),
            asset: None,
        };
        page.updates.set_offer(Some(offer), Phase::Checked);
        assert_eq!(page.update_notice(None), Some("0.3.0".into()));
        assert_eq!(page.update_notice(Some("0.3.0")), None);
        // A newer one shows again.
        assert_eq!(page.update_notice(Some("0.2.5")), Some("0.3.0".into()));
    }
}

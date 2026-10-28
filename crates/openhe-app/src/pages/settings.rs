//! Pengaturan page (polling, Win/Mac, profiles, reset). Track F, Wave 2.
//!
//! Also home of the profile and Win/Mac switch jobs, which the top bar
//! shares.

use crate::app::AppCtx;
use crate::i18n;
use eframe::egui;
use openhe_device::{Error, Keyboard, Transport};
use openhe_proto::SystemMode;

// ------------------------------------------------------- switch jobs

/// What a profile or Win/Mac switch job did: the write result, and the
/// value read back afterwards (also after a failed write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOutcome {
    pub write: Result<(), Error>,
    /// The profile read back (profile switches only); `None` when the
    /// re-read failed.
    pub profile: Option<u8>,
    /// The mode read back (mode switches only); `None` when the re-read
    /// failed.
    pub system_mode: Option<Option<SystemMode>>,
}

/// Switches the onboard profile (0..=3), then reads it back. A disconnect
/// at any point is `Err(Disconnected)`, so the worker reconnects.
pub fn profile_job<T: Transport>(kb: &mut Keyboard<T>, idx: u8) -> Result<SwitchOutcome, Error> {
    let write = kb.set_profile(idx);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let profile = match kb.profile() {
        Ok(p) => Some(p),
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(_) => None,
    };
    Ok(SwitchOutcome {
        write,
        profile,
        system_mode: None,
    })
}

/// Switches Win/Mac mode, then reads it back.
pub fn system_mode_job<T: Transport>(
    kb: &mut Keyboard<T>,
    mode: SystemMode,
) -> Result<SwitchOutcome, Error> {
    let write = kb.set_system_mode(mode);
    if write == Err(Error::Disconnected) {
        return Err(Error::Disconnected);
    }
    let system_mode = match kb.system_mode() {
        Ok(m) => Some(m),
        Err(Error::Disconnected) => return Err(Error::Disconnected),
        Err(_) => None,
    };
    Ok(SwitchOutcome {
        write,
        profile: None,
        system_mode,
    })
}

/// What the UI does with a finished switch job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchEffects {
    /// Patch the snapshot's profile with this (read back).
    pub profile: Option<u8>,
    /// Patch the snapshot's Win/Mac mode with this (read back).
    pub system_mode: Option<Option<SystemMode>>,
    /// A profile or mode switch changes what the keys do: re-read all.
    pub request_reload: bool,
    /// Indonesian error text.
    pub error: Option<String>,
    pub technical: Option<String>,
}

/// Turns a switch job result into UI effects. Every finished job reloads
/// the snapshot (a new profile has new key settings; a new mode may change
/// the layout), except a disconnect, after which the worker reloads anyway.
pub fn switch_effects(result: Result<SwitchOutcome, Error>, failed: &str) -> SwitchEffects {
    match result {
        Ok(out) => {
            let (error, technical) = match &out.write {
                Ok(()) => (None, None),
                Err(e) => (
                    Some(format!("{failed} {}", i18n::device_error(e))),
                    Some(i18n::technical(e)),
                ),
            };
            SwitchEffects {
                profile: out.profile,
                system_mode: out.system_mode,
                request_reload: true,
                error,
                technical,
            }
        }
        Err(e) => SwitchEffects {
            profile: None,
            system_mode: None,
            request_reload: e != Error::Disconnected,
            error: Some(format!("{failed} {}", i18n::device_error(&e))),
            technical: Some(i18n::technical(&e)),
        },
    }
}

/// Applies [`SwitchEffects`] to the worker; returns the error text.
pub fn apply_switch_effects(
    worker: &crate::worker::Worker,
    fx: SwitchEffects,
) -> Option<(String, Option<String>)> {
    if fx.profile.is_some() || fx.system_mode.is_some() {
        let (p, m) = (fx.profile, fx.system_mode);
        worker.patch_snapshot(move |s| {
            if let Some(p) = p {
                s.profile = p;
            }
            if let Some(m) = m {
                s.system_mode = m;
            }
        });
    }
    if fx.request_reload {
        worker.request_reload();
    }
    fx.error.map(|e| (e, fx.technical))
}

// --------------------------------------------------------------- page

#[derive(Default)]
pub struct Page;

impl Page {
    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut egui::Ui) {
        super::coming_soon(ctx, ui, i18n::nav::SETTINGS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::MockTransport;
    use openhe_proto::Model;

    pub(crate) fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x5C, 0x04, 0x80, 0x00]);
        r[4..8].copy_from_slice(&[0, ty, v, 0xFF]);
        r
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn profile_switch_writes_verifies_and_rereads() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 2)); // SET
        m.push_reply(order_reply(0x70, 2)); // verify
        m.push_reply(order_reply(0x70, 2)); // re-read
        let out = profile_job(&mut kb(m), 2).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.profile, Some(2));
        let fx = switch_effects(Ok(out), "x");
        assert_eq!(fx.profile, Some(2));
        assert!(fx.request_reload);
        assert_eq!(fx.error, None);
    }

    #[test]
    fn a_failed_profile_switch_still_rereads_and_reloads() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 2)); // SET
        m.push_reply(order_reply(0x70, 1)); // verify: stayed on 1
        m.push_reply(order_reply(0x70, 1)); // re-read
        let out = profile_job(&mut kb(m), 2).unwrap();
        assert!(out.write.is_err());
        assert_eq!(out.profile, Some(1));
        let fx = switch_effects(Ok(out), "Gagal.");
        assert_eq!(
            fx.profile,
            Some(1),
            "the screen shows what the keyboard has"
        );
        assert!(fx.request_reload);
        let e = fx.error.unwrap();
        assert!(e.starts_with("Gagal. "), "{e}");
        assert!(!e.contains("verify"), "no English: {e}");
    }

    #[test]
    fn a_failing_reread_leaves_the_value_unpatched() {
        // No replies: SET times out, the re-read too.
        let out = profile_job(&mut kb(MockTransport::new()), 3).unwrap();
        assert_eq!(out.write, Err(Error::Timeout));
        assert_eq!(out.profile, None);
        let fx = switch_effects(Ok(out), "x");
        assert_eq!(fx.profile, None);
        assert!(fx.request_reload);
    }

    #[test]
    fn disconnect_is_passed_through_and_does_not_reload() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let r = system_mode_job(&mut kb(m), SystemMode::Mac);
        assert_eq!(r, Err(Error::Disconnected));
        let fx = switch_effects(r, "x");
        assert!(!fx.request_reload);
        assert!(fx.error.is_some());
    }

    #[test]
    fn mode_switch_rereads_the_mode() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x31, 1)); // accepted
        m.push_reply(order_reply(0x21, 0)); // verify: Mac
        m.push_reply(order_reply(0x21, 0)); // re-read
        let out = system_mode_job(&mut kb(m), SystemMode::Mac).unwrap();
        assert_eq!(out.write, Ok(()));
        assert_eq!(out.system_mode, Some(Some(SystemMode::Mac)));
    }
}

//! Keyboard macro (CMD 0x20 MACRO and 0x21 MACRO_MODE) operations.
//!
//! The macro library lives on the host; only the copy bound to a key goes to
//! the keyboard.
//!
//! **Unverified:** the official app writes every macro's data at address 256
//! with id 0, so it is not known whether several keys can hold *different*
//! macros at the same time (PROTOCOL open question 8). Binding a second macro
//! may overwrite the first one's actions on the keyboard. Test this on
//! hardware before building a UI that promises it.

use crate::{AdvancedConfig, Error, Keyboard, Transport};
use openhe_proto::key::{Advanced, KeyMode, KeyParam};
use openhe_proto::macros::{self, MacroAction, MacroError, PlayMode};

pub use openhe_proto::macros::MacroBinding;

/// A MACRO_MODE reply that does not parse: a problem with what the keyboard
/// sent (`Error::Io`). Macros passed in are checked with [`invalid`].
impl From<MacroError> for Error {
    fn from(e: MacroError) -> Self {
        Error::Io(format!("unexpected macro reply: {e}"))
    }
}

/// A macro from the caller that the keyboard cannot store.
fn invalid(e: MacroError) -> Error {
    Error::InvalidInput(e.to_string())
}

impl<T: Transport> Keyboard<T> {
    /// Binds a macro to `key` and verifies the playback settings.
    ///
    /// Sequence:
    /// 1. validate (1..=255 actions, delays and `interval_ms` below 2^24,
    ///    `repeat >= 1`; `Error::InvalidInput`, before any I/O);
    /// 2. read the key's Mode (a read only; an unknown Mode is an `Error::Io`
    ///    before anything is written). When the key holds another advanced
    ///    function (DKS, MT, TGL, MPT, END or SOCD), clear it first with
    ///    `set_advanced(key, AdvancedConfig::None)`, which also clears the
    ///    old partner of a SOCD pair, so no partner is left half-bound; then
    ///    read the Mode again;
    /// 3. write the MACRO data packets (at most 9 actions each, address from 256);
    /// 4. write Mode as `(touch << 4) | 6`, keeping the key's current touch mode
    ///    and high byte, verified by a read-back;
    /// 5. write MACRO_MODE;
    /// 6. read MACRO_MODE back and compare it with what was written
    ///    ("verify failed" `Error::Io` on a difference).
    ///
    /// The action data itself has no read-back command, so step 3 is confirmed
    /// by the keyboard's replies only.
    ///
    /// Whether several keys can hold different macros is **unverified**: every
    /// bind writes its data at address 256 with id 0. See the module docs.
    pub fn bind_macro(
        &mut self,
        key: u8,
        actions: &[MacroAction],
        mode: PlayMode,
        repeat: u16,
        interval_ms: u32,
    ) -> Result<(), Error> {
        macros::validate(actions, repeat, interval_ms).map_err(invalid)?;
        let packets = macros::data_packets(actions).map_err(invalid)?;
        let mode_req = macros::mode_set(key, actions.len() as u8, mode, repeat, interval_ms)
            .map_err(invalid)?;

        let mut current = self.macro_key_mode(key)?;
        if !matches!(current.advanced, Advanced::None | Advanced::Macro) {
            self.set_advanced(key, &AdvancedConfig::None)?;
            current = self.macro_key_mode(key)?;
        }
        let bound = KeyMode {
            advanced: Advanced::Macro,
            ..current
        };

        for req in &packets {
            self.session.call(req)?;
        }
        self.set_key_params(&[(key, KeyParam::Mode, bound.to_u16())])?;
        self.session.call(&mode_req)?;

        let want = MacroBinding {
            len: actions.len() as u8,
            mode,
            repeat,
            interval_ms,
        };
        let got = self.read_macro_mode(key)?;
        if got != want {
            return Err(Error::Io(format!(
                "verify failed: macro mode of key 0x{key:02X}: wrote {want:?}, read {got:?}"
            )));
        }
        Ok(())
    }

    /// The macro bound to `key`, or `None` when the key's advanced mode is not
    /// Macro (a single KEY read in that case).
    pub fn macro_binding(&mut self, key: u8) -> Result<Option<MacroBinding>, Error> {
        if self.macro_key_mode(key)?.advanced != Advanced::Macro {
            return Ok(None);
        }
        Ok(Some(self.read_macro_mode(key)?))
    }

    fn macro_key_mode(&mut self, key: u8) -> Result<KeyMode, Error> {
        let raw = self.key_param(key, KeyParam::Mode)?;
        KeyMode::from_u16(raw)
            .ok_or_else(|| Error::Io(format!("unknown key mode 0x{raw:04X} for key 0x{key:02X}")))
    }

    fn read_macro_mode(&mut self, key: u8) -> Result<MacroBinding, Error> {
        let r = self.session.call(&macros::mode_get(key))?;
        let (got_key, binding) = macros::parse_mode(&r)?;
        if got_key != key {
            return Err(Error::Io(format!(
                "unexpected MACRO_MODE reply for key 0x{key:02X}: key 0x{got_key:02X}"
            )));
        }
        Ok(binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::model::Model;

    fn payload(frame: &[u8; 64]) -> &[u8] {
        &frame[4..4 + usize::from(frame[1])]
    }

    fn act(code: u16, down: bool, delay_ms: u32) -> MacroAction {
        MacroAction {
            code,
            down,
            delay_ms,
        }
    }

    fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let [lo, hi] = v.to_le_bytes();
        reply(0x23, &[0, key, layout, lo, hi])
    }

    fn data_reply() -> [u8; 64] {
        reply(0x20, &[0, 0, 1, 0])
    }

    fn mode_reply(key: u8, len: u8, mode: PlayMode, repeat: u16, interval: u32) -> [u8; 64] {
        let p = macros::mode_set(key, len, mode, repeat, interval)
            .unwrap()
            .payload;
        reply(0x21, &p)
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    const KEY: u8 = 0x0A;

    /// Queues the replies of a successful `bind_macro` of `n` actions and
    /// `packets` data packets, for a key whose current Mode is `current`.
    fn queue_bind(m: &mut MockTransport, current: u16, packets: usize, n: u8, new_mode: u16) {
        m.push_reply(key_reply(KEY, 8, current)); // Mode read
        for _ in 0..packets {
            m.push_reply(data_reply());
        }
        m.push_reply(key_reply(KEY, 8, new_mode)); // Mode SET echo
        m.push_reply(key_reply(KEY, 8, new_mode)); // Mode read-back
        m.push_reply(mode_reply(KEY, n, PlayMode::WhileHeld, 2, 100)); // SET reply
        m.push_reply(mode_reply(KEY, n, PlayMode::WhileHeld, 2, 100)); // GET reply
    }

    #[test]
    fn bind_runs_data_then_mode_then_macro_mode_then_verifies() {
        let ten: Vec<_> = (0..10u16)
            .map(|i| act(0x04 + i, i % 2 == 0, 100 * u32::from(i)))
            .collect();
        let mut m = MockTransport::new();
        queue_bind(&mut m, 0x0000, 2, 10, 0x0006);
        let mut k = kb(m);
        assert_eq!(k.bind_macro(KEY, &ten, PlayMode::WhileHeld, 2, 100), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 7);
        assert_eq!(payload(&w[0]), [0x00, KEY, 0x08, 0x00, 0x00]); // Mode GET
        assert_eq!(
            (w[1][2], &w[1][4..8]),
            (0x20, &[0x01, 0x00, 0x01, 0x09][..])
        ); // packet 1, addr 256
        assert_eq!(
            (w[2][2], &w[2][4..8]),
            (0x20, &[0x01, 0x09, 0x01, 0x01][..])
        ); // packet 2, addr 265
        assert_eq!(payload(&w[3]), [0x01, KEY, 0x08, 0x06, 0x00]); // Mode SET
        assert_eq!(payload(&w[4]), [0x00, KEY, 0x08, 0x00, 0x00]); // Mode read-back
        assert_eq!(
            &w[5][..15],
            &[
                0x5C, 0x0B, 0x21, 0xBD, 0x01, KEY, 0x00, 0x00, 0x0A, 0x02, 0x02, 0x00, 0x64, 0x00,
                0x00
            ]
        ); // MACRO_MODE SET: len 10, WhileHeld, repeat 2, 100 ms
        assert_eq!(w[6][2], 0x21);
        assert_eq!(w[6][4], 0x00, "MACRO_MODE GET uses flag 00");
    }

    #[test]
    fn mode_write_keeps_touch_mode_and_high_byte() {
        // Current Mode: touch RapidTrigger (2), no function, high byte 0x07.
        // (A key with another function is cleared first; see below.)
        let current = KeyMode::from_u16(0x0720).unwrap().to_u16();
        assert_eq!(current, 0x0720);
        let new = 0x0726; // touch 2, advanced Macro, high 7
        let mut m = MockTransport::new();
        queue_bind(&mut m, current, 1, 1, new);
        let mut k = kb(m);
        assert_eq!(
            k.bind_macro(KEY, &[act(4, true, 0)], PlayMode::WhileHeld, 2, 100),
            Ok(())
        );
        let w = k.session_mut().transport().written();
        assert_eq!(payload(&w[2]), [0x01, KEY, 0x08, 0x26, 0x07]);
    }

    /// A SOCD pair (4, 7), as `socd_get` reports it from either key.
    fn socd_pair_reply() -> [u8; 64] {
        reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0])
    }

    #[test]
    fn binding_over_a_socd_key_clears_the_pair_first() {
        // Key 4 is in a SOCD pair with key 7 (Mode 0x0018: Single + SOCD).
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0018)); // bind: Mode read
        m.push_reply(key_reply(0x04, 8, 0x0018)); // clear: Mode read
        m.push_reply(socd_pair_reply()); // clear: its partner is 7
        m.push_reply(key_reply(0x04, 8, 0x0018)); // clear: re-read key 4
        m.push_reply(key_reply(0x07, 8, 0x0328)); // clear: re-read partner
        m.push_reply(key_reply(0x04, 8, 0x0010)); // clear: Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0010)); // clear: read-backs
        m.push_reply(key_reply(0x07, 8, 0x0320));
        m.push_reply(key_reply(0x04, 8, 0x0010)); // bind: Mode re-read
        m.push_reply(data_reply());
        m.push_reply(key_reply(0x04, 8, 0x0016)); // Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0016)); // read-back
        m.push_reply(reply(
            0x21,
            &macros::mode_set(0x04, 1, PlayMode::Once, 1, 100)
                .unwrap()
                .payload,
        ));
        m.push_reply(reply(
            0x21,
            &macros::mode_set(0x04, 1, PlayMode::Once, 1, 100)
                .unwrap()
                .payload,
        ));
        let mut k = kb(m);
        assert_eq!(
            k.bind_macro(0x04, &[act(4, true, 0)], PlayMode::Once, 1, 100),
            Ok(())
        );
        let w = k.session_mut().transport().written().to_vec();
        // The pair is cleared before any macro data goes out: key 4 and its
        // partner 7 lose the SOCD nibble (touch mode and high byte kept).
        let clear = w
            .iter()
            .position(|f| payload(f) == [0x01, 0x04, 8, 0x10, 0x00, 0x07, 8, 0x20, 0x03])
            .expect("SOCD pair cleared");
        let data = w.iter().position(|f| f[2] == 0x20).expect("macro data");
        assert!(clear < data);
        // Then key 4 gets the macro nibble on its current touch mode.
        assert!(w.iter().any(|f| payload(f) == [0x01, 0x04, 8, 0x16, 0x00]));
    }

    #[test]
    fn binding_over_another_function_clears_it_first() {
        // Key with MT (nibble 3): cleared, then bound.
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0003)); // bind: Mode read
        m.push_reply(key_reply(KEY, 8, 0x0003)); // clear: Mode read
        m.push_reply(key_reply(KEY, 8, 0x0003)); // clear: re-read
        m.push_reply(key_reply(KEY, 8, 0x0000)); // clear: SET echo
        m.push_reply(key_reply(KEY, 8, 0x0000)); // clear: read-back
        queue_bind(&mut m, 0x0000, 1, 1, 0x0006);
        let mut k = kb(m);
        assert_eq!(
            k.bind_macro(KEY, &[act(4, true, 0)], PlayMode::WhileHeld, 2, 100),
            Ok(())
        );
        let w = k.session_mut().transport().written();
        assert_eq!(payload(&w[3]), [0x01, KEY, 0x08, 0x00, 0x00]); // cleared
        assert!(w
            .iter()
            .any(|f| payload(f) == [0x01, KEY, 0x08, 0x06, 0x00]));
    }

    #[test]
    fn rebinding_a_macro_key_does_not_clear_it() {
        let mut m = MockTransport::new();
        queue_bind(&mut m, 0x0006, 1, 1, 0x0006);
        let mut k = kb(m);
        assert_eq!(
            k.bind_macro(KEY, &[act(4, true, 0)], PlayMode::WhileHeld, 2, 100),
            Ok(())
        );
        assert_eq!(k.session_mut().transport().written().len(), 6);
    }

    #[test]
    fn macro_mode_mismatch_is_a_verify_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0));
        m.push_reply(data_reply());
        m.push_reply(key_reply(KEY, 8, 6));
        m.push_reply(key_reply(KEY, 8, 6));
        m.push_reply(mode_reply(KEY, 1, PlayMode::WhileHeld, 2, 100));
        m.push_reply(mode_reply(KEY, 1, PlayMode::Once, 2, 100)); // read back differs
        let e = kb(m)
            .bind_macro(KEY, &[act(4, true, 0)], PlayMode::WhileHeld, 2, 100)
            .unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
    }

    #[test]
    fn invalid_input_is_rejected_before_any_io() {
        let mut k = kb(MockTransport::new());
        let a = [act(4, true, 0)];
        for (actions, repeat, interval) in [
            (&a[..], 0u16, 100u32),
            (&a[..], 1, 1 << 24),
            (&[act(4, true, 1 << 24)][..], 1, 100),
            (&[][..], 1, 100),
        ] {
            let e = k
                .bind_macro(KEY, actions, PlayMode::Once, repeat, interval)
                .unwrap_err();
            assert!(matches!(e, Error::InvalidInput(_)), "{e:?}");
        }
        let too_many: Vec<_> = (0..256).map(|_| act(4, true, 0)).collect();
        assert!(matches!(
            k.bind_macro(KEY, &too_many, PlayMode::Once, 1, 100),
            Err(Error::InvalidInput(_))
        ));
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn unknown_current_mode_stops_before_writing_anything() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x07)); // advanced nibble 7 is unknown
        let mut k = kb(m);
        let e = k
            .bind_macro(KEY, &[act(4, true, 0)], PlayMode::Once, 1, 100)
            .unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn macro_binding_is_none_when_the_key_has_no_macro() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0010));
        let mut k = kb(m);
        assert_eq!(k.macro_binding(KEY), Ok(None));
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn macro_binding_reads_the_playback_settings() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0026));
        m.push_reply(mode_reply(KEY, 5, PlayMode::Toggle, 3, 250));
        let mut k = kb(m);
        assert_eq!(
            k.macro_binding(KEY),
            Ok(Some(MacroBinding {
                len: 5,
                mode: PlayMode::Toggle,
                repeat: 3,
                interval_ms: 250
            }))
        );
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2);
        assert_eq!(w[1][2], 0x21);
        assert_eq!(w[1][5], KEY);
    }

    #[test]
    fn an_unknown_play_mode_in_a_reply_is_an_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0006));
        let mut body = macros::mode_set(KEY, 5, PlayMode::Once, 1, 100)
            .unwrap()
            .payload;
        body[5] = 9; // not a play mode
        m.push_reply(reply(0x21, &body));
        let e = kb(m).macro_binding(KEY).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
        assert!(matches!(Error::from(MacroError::WrongReply), Error::Io(_)));
    }

    #[test]
    fn macro_binding_rejects_a_reply_for_another_key() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0006));
        m.push_reply(mode_reply(0x0B, 5, PlayMode::Once, 1, 100));
        let e = kb(m).macro_binding(KEY).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
    }
}

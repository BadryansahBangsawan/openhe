pub mod advanced;
pub mod lighting;
pub mod macros;
pub mod performance;
pub mod system;

use crate::{Error, Session, Transport};
use openhe_proto::defkey::{self, Matrix};
use openhe_proto::key::{self, KeyMode, KeyParam};
use openhe_proto::model::{Model, PollingRate, SystemMode};
use openhe_proto::order::{self, OrderType};
use openhe_proto::sync::{self, DeviceInfo};
use openhe_proto::units::Micrometers;

/// All per-key settings, read in one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySettings {
    pub mode: KeyMode,
    pub actuation: Micrometers,
    pub rt_press: Micrometers,
    pub rt_release: Micrometers,
    pub top_dz: Micrometers,
    pub bottom_dz: Micrometers,
    pub switch: u16,
}

/// Typed access to a keyboard: reads, plus verified per-key writes.
pub struct Keyboard<T: Transport> {
    pub model: Model,
    session: Session<T>,
    /// RGBNACK was sent on this connection (see `lighting`).
    rgbnack_sent: bool,
}

impl<T: Transport> Keyboard<T> {
    pub fn new(transport: T, model: Model) -> Self {
        Keyboard {
            model,
            session: Session::new(transport),
            rgbnack_sent: false,
        }
    }

    pub fn take_events(&mut self) -> Vec<openhe_proto::frame::Response> {
        self.session.take_events()
    }

    pub fn session_mut(&mut self) -> &mut Session<T> {
        &mut self.session
    }

    pub fn info(&mut self) -> Result<DeviceInfo, Error> {
        let r = self.session.call(&sync::request())?;
        Ok(sync::parse(&r))
    }

    pub fn system_mode(&mut self) -> Result<Option<SystemMode>, Error> {
        let r = self.session.call(&order::query(OrderType::QueryWinMode))?;
        Ok(order::parse_system_mode(&r))
    }

    pub fn profile(&mut self) -> Result<u8, Error> {
        let r = self.session.call(&order::query(OrderType::Profile))?;
        order::parse_profile(&r)
            .ok_or_else(|| Error::Io(format!("unexpected profile reply: {:02X?}", &r.body()[..4])))
    }

    pub fn polling_rate(&mut self) -> Result<PollingRate, Error> {
        let r = self.session.call(&order::query(OrderType::Polling))?;
        order::parse_polling(&r)
            .ok_or_else(|| Error::Io(format!("unexpected polling reply: {:02X?}", &r.body()[..4])))
    }

    pub fn matrix(&mut self) -> Result<Matrix, Error> {
        let mut m = Matrix::default();
        for req in defkey::requests() {
            let r = self.session.call(&req)?;
            m.apply(&r)?;
        }
        Ok(m)
    }

    pub fn key_param(&mut self, key: u8, p: KeyParam) -> Result<u16, Error> {
        let r = self.session.call(&key::get(key, p))?;
        let layout = p.layout();
        match key::parse_value(&r) {
            Some((k, l, v)) if k == key && l == layout => Ok(v),
            _ => Err(Error::Io(format!(
                "unexpected reply for key 0x{key:02X} layout {layout}: {:02X?}",
                &r.body()[..5]
            ))),
        }
    }

    pub fn key_settings(&mut self, key: u8) -> Result<KeySettings, Error> {
        let mode_raw = self.key_param(key, KeyParam::Mode)?;
        let mode = KeyMode::from_u16(mode_raw).ok_or_else(|| {
            Error::Io(format!(
                "unknown key mode 0x{mode_raw:04X} for key 0x{key:02X}"
            ))
        })?;
        let actuation = Micrometers(self.key_param(key, KeyParam::Actuation)?);
        let rt_press = Micrometers(self.key_param(key, KeyParam::RtPress)?);
        let rt_release = Micrometers(self.key_param(key, KeyParam::RtRelease)?);
        let top_dz = Micrometers(self.key_param(key, KeyParam::TopDeadzone)?);
        let bottom_dz = Micrometers(self.key_param(key, KeyParam::BottomDeadzone)?);
        let switch = self.key_param(key, KeyParam::Switch)?;
        Ok(KeySettings {
            mode,
            actuation,
            rt_press,
            rt_release,
            top_dz,
            bottom_dz,
            switch,
        })
    }

    /// Writes per-key parameters, then reads each one back to confirm it stuck.
    ///
    /// Every request from `key::set` goes through the session, so each SET reply
    /// is matched by the usual rules. Afterwards every entry is read with
    /// `key_param` and compared with the value written; a difference is an
    /// `Error::Io` that starts with "verify failed". Nothing is read back after a
    /// failed SET, and an empty `entries` does no I/O. A `(key, param)` that
    /// appears twice is `Error::InvalidInput`, returned before any I/O.
    pub fn set_key_params(&mut self, entries: &[(u8, KeyParam, u16)]) -> Result<(), Error> {
        let mut seen = std::collections::HashSet::new();
        for &(key, param, _) in entries {
            if !seen.insert((key, param)) {
                return Err(Error::InvalidInput(format!(
                    "duplicate entry for key 0x{key:02X} layout {}",
                    param.layout()
                )));
            }
        }
        if entries.is_empty() {
            return Ok(());
        }
        for req in key::set(entries) {
            self.session.call(&req)?;
        }
        for &(key, param, want) in entries {
            let got = self.key_param(key, param)?;
            if got != want {
                return Err(Error::Io(format!(
                    "verify failed: key 0x{key:02X} layout {}: wrote {want}, read {got}",
                    param.layout()
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::key::{Advanced, TouchMode};

    /// The payload of a written frame (`5C LEN CMD CHK` header stripped).
    fn payload(frame: &[u8; 64]) -> &[u8] {
        &frame[4..4 + usize::from(frame[1])]
    }

    fn defkey_reply(row_a: u8, a: &[u8], row_b: u8, b: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xAB;
        raw[5] = row_a;
        raw[6..6 + a.len()].copy_from_slice(a);
        raw[27] = row_b;
        raw[28..28 + b.len()].copy_from_slice(b);
        raw
    }

    fn codes(n: usize, first: u8) -> Vec<u8> {
        (0..n).map(|i| first + i as u8).collect()
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0x80;
        raw[5] = ty;
        raw[6] = v;
        raw
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn reads_matrix_three_calls() {
        let mut m = MockTransport::new();
        m.push_reply(defkey_reply(0, &codes(15, 0x29), 1, &codes(15, 0x40)));
        m.push_reply(defkey_reply(2, &codes(14, 0x50), 3, &codes(14, 0x60)));
        m.push_reply(defkey_reply(4, &codes(10, 0x70), 5, &[]));
        let mut k = kb(m);
        let matrix = k.matrix().unwrap();
        assert_eq!(matrix.len(), 68);
        assert_eq!(k.session_mut().transport().written().len(), 3);
    }

    #[test]
    fn key_settings_reads_seven_params() {
        let mut m = MockTransport::new();
        let k = 0x0A;
        for (layout, v) in [
            (8u8, 0x20u16),
            (4, 1500),
            (20, 100),
            (21, 120),
            (22, 50),
            (23, 60),
            (25, 7),
        ] {
            m.push_reply(key_reply(k, layout, v));
        }
        let mut kbd = kb(m);
        let s = kbd.key_settings(k).unwrap();
        assert_eq!(
            s,
            KeySettings {
                mode: KeyMode {
                    touch: TouchMode::RapidTrigger,
                    advanced: Advanced::None,
                    high: 0,
                },
                actuation: Micrometers(1500),
                rt_press: Micrometers(100),
                rt_release: Micrometers(120),
                top_dz: Micrometers(50),
                bottom_dz: Micrometers(60),
                switch: 7,
            }
        );
        assert_eq!(kbd.session_mut().transport().written().len(), 7);
    }

    #[test]
    fn wrong_key_code_is_never_taken_as_the_answer() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0B, 4, 1500));
        let mut k = kb(m);
        assert_eq!(k.key_param(0x0A, KeyParam::Actuation), Err(Error::Timeout));
        let ev = k.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(key::parse_value(&ev[0]), Some((0x0B, 4, 1500)));
    }

    #[test]
    fn wrong_layout_is_never_taken_as_the_answer() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 20, 1500));
        let mut k = kb(m);
        assert_eq!(k.key_param(0x0A, KeyParam::Actuation), Err(Error::Timeout));
        assert_eq!(k.take_events().len(), 1);
    }

    #[test]
    fn unknown_mode_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 8, 0x07));
        let e = kb(m).key_settings(0x0A).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
    }

    #[test]
    fn profile_reads_index() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 2));
        assert_eq!(kb(m).profile(), Ok(2));
    }

    #[test]
    fn profile_out_of_range_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 9));
        assert!(matches!(kb(m).profile(), Err(Error::Io(_))));
    }

    #[test]
    fn system_mode_and_polling() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x21, 1));
        m.push_reply(order_reply(0x50, 3));
        let mut k = kb(m);
        assert_eq!(k.system_mode(), Ok(Some(SystemMode::Windows)));
        assert_eq!(k.polling_rate(), Ok(PollingRate::Hz1000));
    }

    #[test]
    fn set_then_verify() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 4, 1500)); // SET echo
        m.push_reply(key_reply(0x0A, 4, 1500)); // read-back GET
        let mut k = kb(m);
        assert_eq!(
            k.set_key_params(&[(0x0A, KeyParam::Actuation, 1500)]),
            Ok(())
        );
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2);
        assert_eq!(payload(&w[0]), [0x01, 0x0A, 0x04, 0xDC, 0x05]); // SET 1500 = 0x05DC
        assert_eq!(payload(&w[1]), [0x00, 0x0A, 0x04, 0x00, 0x00]); // GET
    }

    #[test]
    fn set_verify_mismatch() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 4, 1500)); // SET echo
        m.push_reply(key_reply(0x0A, 4, 2000)); // read-back differs
        let e = kb(m)
            .set_key_params(&[(0x0A, KeyParam::Actuation, 1500)])
            .unwrap_err();
        assert_eq!(
            e,
            Error::Io("verify failed: key 0x0A layout 4: wrote 1500, read 2000".to_string())
        );
    }

    #[test]
    fn unsolicited_key_event_during_set_does_not_replace_the_read_back() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 4, 1500)); // SET echo
        m.push_reply(key_reply(0x0B, 4, 1500)); // unsolicited: another key, same layout and value
        m.push_reply(key_reply(0x0A, 4, 1500)); // the real read-back
        let mut k = kb(m);
        assert_eq!(
            k.set_key_params(&[(0x0A, KeyParam::Actuation, 1500)]),
            Ok(())
        );
        let ev = k.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(key::parse_value(&ev[0]), Some((0x0B, 4, 1500)));
        assert_eq!(k.session_mut().transport().written().len(), 2);
    }

    #[test]
    fn unsolicited_key_event_during_set_cannot_fake_a_passing_verify() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 4, 1500)); // SET echo
        m.push_reply(key_reply(0x0A, 20, 1500)); // unsolicited: same key, other layout
        m.push_reply(key_reply(0x0A, 4, 2000)); // the real read-back: the write did not stick
        let e = kb(m)
            .set_key_params(&[(0x0A, KeyParam::Actuation, 1500)])
            .unwrap_err();
        assert_eq!(
            e,
            Error::Io("verify failed: key 0x0A layout 4: wrote 1500, read 2000".to_string())
        );
    }

    #[test]
    fn duplicate_entries_are_rejected_before_any_io() {
        let mut k = kb(MockTransport::new());
        let e = k
            .set_key_params(&[
                (0x0A, KeyParam::Actuation, 1500),
                (0x0B, KeyParam::Actuation, 1500),
                (0x0A, KeyParam::Actuation, 1600),
            ])
            .unwrap_err();
        assert_eq!(
            e,
            Error::InvalidInput("duplicate entry for key 0x0A layout 4".to_string())
        );
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn same_param_on_different_layers_or_keys_is_not_a_duplicate() {
        use openhe_proto::key::Layer;
        let entries = [
            (0x0A, KeyParam::Layer(Layer::Base), 0x0A),
            (0x0A, KeyParam::Layer(Layer::Fn1), 0x0B),
            (0x0B, KeyParam::Layer(Layer::Base), 0x0B),
        ];
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0A)); // SET echo (first entry)
        for (key, p, v) in entries {
            m.push_reply(key_reply(key, p.layout(), v)); // read-backs
        }
        assert_eq!(kb(m).set_key_params(&entries), Ok(()));
    }

    #[test]
    fn set_nothing_does_no_io() {
        let mut k = kb(MockTransport::new());
        assert_eq!(k.set_key_params(&[]), Ok(()));
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn failed_set_is_not_read_back() {
        let mut m = MockTransport::new();
        let mut fail = [0u8; 64];
        fail[0] = 0x5C;
        fail[2] = 0xFF;
        m.push_reply(fail); // the keyboard rejects the SET
        m.push_reply(key_reply(0x0A, 4, 1500)); // a read-back would accept this; it must stay unread
        let mut k = kb(m);
        let e = k
            .set_key_params(&[(0x0A, KeyParam::Actuation, 1500)])
            .unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 1);
        assert_eq!(payload(&w[0])[0], 0x01);
    }

    #[test]
    fn set_batches_then_verifies_each_entry_in_order() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 4, 1500)); // one SET packet carries both entries
        m.push_reply(key_reply(0x0A, 4, 1500)); // read-back of entry 1: ok
        m.push_reply(key_reply(0x0A, 20, 99)); // read-back of entry 2: differs from 100
        let mut k = kb(m);
        let e = k
            .set_key_params(&[
                (0x0A, KeyParam::Actuation, 1500),
                (0x0A, KeyParam::RtPress, 100),
            ])
            .unwrap_err();
        assert_eq!(
            e,
            Error::Io("verify failed: key 0x0A layout 20: wrote 100, read 99".to_string())
        );
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 3);
        assert_eq!(
            payload(&w[0]),
            [0x01, 0x0A, 0x04, 0xDC, 0x05, 0x0A, 0x14, 0x64, 0x00]
        );
        assert_eq!(payload(&w[1]), [0x00, 0x0A, 0x04, 0x00, 0x00]);
        assert_eq!(payload(&w[2]), [0x00, 0x0A, 0x14, 0x00, 0x00]);
    }

    #[test]
    fn set_sends_every_packet_past_fourteen_entries() {
        let entries: Vec<_> = (0..15u8)
            .map(|i| (0x10 + i, KeyParam::Actuation, 1000 + u16::from(i)))
            .collect();
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x10, 4, 1000)); // SET echo, packet 1 (14 entries)
        m.push_reply(key_reply(0x1E, 4, 1014)); // SET echo, packet 2 (1 entry)
        for &(key, _, v) in &entries {
            m.push_reply(key_reply(key, 4, v)); // read-backs
        }
        let mut k = kb(m);
        assert_eq!(k.set_key_params(&entries), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2 + 15);
        assert_eq!(payload(&w[0]).len(), 1 + 14 * 4);
        assert_eq!(payload(&w[1]), [0x01, 0x1E, 0x04, 0xF6, 0x03]);
        for (frame, &(key, _, _)) in w[2..].iter().zip(&entries) {
            assert_eq!(payload(frame), [0x00, key, 0x04, 0x00, 0x00]);
        }
    }
}

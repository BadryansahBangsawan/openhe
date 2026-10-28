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

/// Read-only typed access to a keyboard.
pub struct Keyboard<T: Transport> {
    pub model: Model,
    session: Session<T>,
}

impl<T: Transport> Keyboard<T> {
    pub fn new(transport: T, model: Model) -> Self {
        Keyboard {
            model,
            session: Session::new(transport),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::key::{Advanced, TouchMode};

    fn defkey_reply(row_a: u8, a: &[u8], row_b: u8, b: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
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
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        let mut raw = [0u8; 64];
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
                    advanced: Advanced::None
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
    fn wrong_key_code_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0B, 4, 1500));
        let e = kb(m).key_param(0x0A, KeyParam::Actuation).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
    }

    #[test]
    fn wrong_layout_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 20, 1500));
        let e = kb(m).key_param(0x0A, KeyParam::Actuation).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
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
}

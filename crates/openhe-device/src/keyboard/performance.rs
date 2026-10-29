//! Per-key performance settings: actuation, rapid trigger, deadzones, switch.
//!
//! [`Keyboard::set_performance`] changes any subset of these on a set of keys.
//! It never clobbers a key's advanced function: the Mode value (KEY LAYOUT 8)
//! is read first and only its touch nibble changes, so the advanced nibble
//! (DKS, MT, ...) and the high byte survive an actuation or RT change.

use crate::{Error, Keyboard, Transport};
use openhe_proto::key::{KeyMode, KeyParam, TouchMode};
use openhe_proto::model::{Switch, MIN_ACTUATION_UM};
use openhe_proto::units::Micrometers;

/// Smallest rapid-trigger sensitivity, in micrometers (0.005 mm).
pub const MIN_RT_UM: u16 = 5;
/// Largest deadzone, in micrometers (1.0 mm).
pub const MAX_DEADZONE_UM: u16 = 1000;

/// A change to apply to every key passed to [`Keyboard::set_performance`].
/// `None` leaves that setting as it is.
///
/// Touch mode rules (controller ruling, matching AULA HUB): setting the
/// actuation or an RT value puts the key in `Single` or `RapidTrigger`. When
/// `touch` is `None` and such a value is set, a key in `Global` becomes
/// `Single` and a key already in `Single`/`RapidTrigger` keeps its mode.
/// `touch: Some(Global)` together with an actuation or RT value is rejected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerformanceChange {
    pub touch: Option<TouchMode>,
    pub actuation: Option<Micrometers>,
    pub rt_press: Option<Micrometers>,
    pub rt_release: Option<Micrometers>,
    pub top_dz: Option<Micrometers>,
    pub bottom_dz: Option<Micrometers>,
    pub switch: Option<u16>,
}

impl PerformanceChange {
    /// True when the change carries a value that pins the touch mode to
    /// Single or RapidTrigger.
    fn sets_travel(&self) -> bool {
        self.actuation.is_some() || self.rt_press.is_some() || self.rt_release.is_some()
    }

    fn is_empty(&self) -> bool {
        self.touch.is_none()
            && !self.sets_travel()
            && self.top_dz.is_none()
            && self.bottom_dz.is_none()
            && self.switch.is_none()
    }
}

fn invalid(msg: String) -> Error {
    Error::InvalidInput(msg)
}

fn check_dz(name: &str, v: Option<Micrometers>) -> Result<(), Error> {
    match v {
        Some(v) if v.0 > MAX_DEADZONE_UM => Err(invalid(format!(
            "{name} deadzone {} um is above the maximum of {MAX_DEADZONE_UM} um",
            v.0
        ))),
        _ => Ok(()),
    }
}

/// Checks the travel values against one switch.
fn check_travel(
    key: u8,
    sw: &Switch,
    actuation: Option<Micrometers>,
    rt_press: Option<Micrometers>,
    rt_release: Option<Micrometers>,
) -> Result<(), Error> {
    let max = sw.max_travel.0;
    if let Some(a) = actuation {
        if a.0 < MIN_ACTUATION_UM || a.0 > max {
            return Err(invalid(format!(
                "key 0x{key:02X}: actuation {} um is outside {MIN_ACTUATION_UM}..={max} um ({})",
                a.0, sw.name
            )));
        }
    }
    for (name, v) in [("RT press", rt_press), ("RT release", rt_release)] {
        if let Some(v) = v {
            if v.0 < MIN_RT_UM || v.0 > max {
                return Err(invalid(format!(
                    "key 0x{key:02X}: {name} {} um is outside {MIN_RT_UM}..={max} um ({})",
                    v.0, sw.name
                )));
            }
        }
    }
    Ok(())
}

impl<T: Transport> Keyboard<T> {
    fn switch_by_id(&self, id: u16) -> Result<&'static Switch, Error> {
        self.model
            .switches()
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| invalid(format!("unknown switch id {id} for {}", self.model.name())))
    }

    /// Applies `change` to every key in `keys`, then verifies by read-back.
    ///
    /// Before any write it reads each key's Mode (and its switch, unless the
    /// change sets one) and validates every value against that key's switch:
    /// actuation 0.1 mm..=max travel, RT 0.005 mm..=max travel, deadzones
    /// 0..=1.0 mm. When the switch changes, values not being written are read
    /// and checked against the new switch's max travel too. Nothing is written
    /// unless every key passes.
    ///
    /// All parameters go out in one batched [`Keyboard::set_key_params`]
    /// (switch first, Mode last). Mode is written only where the touch mode
    /// changes, and then with the advanced nibble and high byte as read.
    /// Empty `keys` or an empty change does no I/O; a repeated key is
    /// `InvalidInput`.
    pub fn set_performance(
        &mut self,
        keys: &[u8],
        change: &PerformanceChange,
    ) -> Result<(), Error> {
        let mut seen = std::collections::HashSet::new();
        for &k in keys {
            if !seen.insert(k) {
                return Err(invalid(format!("key 0x{k:02X} is listed twice")));
            }
        }
        if change.touch == Some(TouchMode::Global) && change.sets_travel() {
            return Err(invalid(
                "setting actuation or RT needs touch mode Single or RapidTrigger".to_string(),
            ));
        }
        check_dz("top", change.top_dz)?;
        check_dz("bottom", change.bottom_dz)?;
        let new_switch = change.switch.map(|id| self.switch_by_id(id)).transpose()?;
        if keys.is_empty() || change.is_empty() {
            return Ok(());
        }

        // Read and validate everything before the first write.
        let mut modes: Vec<(u8, KeyMode)> = Vec::with_capacity(keys.len());
        for &key in keys {
            let raw = self.key_param(key, KeyParam::Mode)?;
            let mode = KeyMode::from_u16(raw).ok_or_else(|| {
                Error::Io(format!("unknown key mode 0x{raw:04X} for key 0x{key:02X}"))
            })?;
            let sw = match new_switch {
                Some(sw) => sw,
                None => {
                    let id = self.key_param(key, KeyParam::Switch)?;
                    self.switch_by_id(id).map_err(|_| {
                        Error::Io(format!("key 0x{key:02X} reports unknown switch {id}"))
                    })?
                }
            };
            check_travel(
                key,
                sw,
                change.actuation,
                change.rt_press,
                change.rt_release,
            )?;
            if new_switch.is_some() {
                // Values kept as they are must still fit the new switch.
                let mut kept = |p: KeyParam, set: Option<Micrometers>| -> Result<_, Error> {
                    Ok(match set {
                        Some(_) => None,
                        None => Some(Micrometers(self.key_param(key, p)?)),
                    })
                };
                let a = kept(KeyParam::Actuation, change.actuation)?;
                let p = kept(KeyParam::RtPress, change.rt_press)?;
                let r = kept(KeyParam::RtRelease, change.rt_release)?;
                check_travel(key, sw, a, p, r).map_err(|e| match e {
                    Error::InvalidInput(m) => {
                        invalid(format!("{m}; set it first or keep the switch"))
                    }
                    other => other,
                })?;
            }
            modes.push((key, mode));
        }

        let mut entries: Vec<(u8, KeyParam, u16)> = Vec::new();
        for &key in keys {
            if let Some(id) = change.switch {
                entries.push((key, KeyParam::Switch, id));
            }
            let values = [
                (KeyParam::Actuation, change.actuation),
                (KeyParam::RtPress, change.rt_press),
                (KeyParam::RtRelease, change.rt_release),
                (KeyParam::TopDeadzone, change.top_dz),
                (KeyParam::BottomDeadzone, change.bottom_dz),
            ];
            for (p, v) in values {
                if let Some(v) = v {
                    entries.push((key, p, v.0));
                }
            }
        }
        for &(key, old) in &modes {
            let touch = match (change.touch, old.touch) {
                (Some(t), _) => t,
                (None, TouchMode::Global) if change.sets_travel() => TouchMode::Single,
                (None, t) => t,
            };
            if touch != old.touch {
                let new = KeyMode { touch, ..old };
                entries.push((key, KeyParam::Mode, new.to_u16()));
            }
        }
        self.set_key_params(&entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::key::Advanced;
    use openhe_proto::model::Model;

    fn payload(frame: &[u8; 64]) -> &[u8] {
        &frame[4..4 + usize::from(frame[1])]
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

    /// Queues replies: the reads, one SET echo per packet, then a read-back
    /// for each written entry.
    fn script(reads: &[(u8, u8, u16)], echoes: usize, writes: &[(u8, u8, u16)]) -> MockTransport {
        let mut m = MockTransport::new();
        for &(k, l, v) in reads {
            m.push_reply(key_reply(k, l, v));
        }
        for &(k, l, v) in writes.iter().take(echoes) {
            m.push_reply(key_reply(k, l, v));
        }
        for &(k, l, v) in writes {
            m.push_reply(key_reply(k, l, v));
        }
        m
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    const W: u8 = 0x1A;
    const A: u8 = 0x04;

    #[test]
    fn actuation_change_keeps_dks_and_the_high_byte() {
        // Review Focus 1: a Global key with DKS (and an unknown high byte).
        let dks_mode = KeyMode {
            touch: TouchMode::Global,
            advanced: Advanced::Dks,
            high: 0xAB,
        };
        let want_mode = KeyMode {
            touch: TouchMode::Single,
            ..dks_mode
        };
        assert_eq!(want_mode.to_u16(), 0xAB11);
        let writes = [(W, 4, 400), (W, 8, 0xAB11)];
        let m = script(&[(W, 8, dks_mode.to_u16()), (W, 25, 0)], 1, &writes);
        let mut k = kb(m);
        let change = PerformanceChange {
            actuation: Some(Micrometers(400)),
            ..Default::default()
        };
        assert_eq!(k.set_performance(&[W], &change), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2 + 1 + 2);
        assert_eq!(payload(&w[0]), [0x00, W, 8, 0, 0]);
        assert_eq!(payload(&w[1]), [0x00, W, 25, 0, 0]);
        assert_eq!(payload(&w[2]), [0x01, W, 4, 0x90, 0x01, W, 8, 0x11, 0xAB]);
        assert_eq!(payload(&w[3]), [0x00, W, 4, 0, 0]);
        assert_eq!(payload(&w[4]), [0x00, W, 8, 0, 0]);
    }

    #[test]
    fn switching_to_rapid_trigger_keeps_mt() {
        let mt = KeyMode {
            touch: TouchMode::Single,
            advanced: Advanced::Mt,
            high: 0x02,
        };
        let writes = [(W, 20, 150), (W, 21, 150), (W, 8, 0x0223)];
        let m = script(&[(W, 8, mt.to_u16()), (W, 25, 0)], 1, &writes);
        let mut k = kb(m);
        let change = PerformanceChange {
            touch: Some(TouchMode::RapidTrigger),
            rt_press: Some(Micrometers(150)),
            rt_release: Some(Micrometers(150)),
            ..Default::default()
        };
        assert_eq!(k.set_performance(&[W], &change), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(
            payload(&w[2]),
            [0x01, W, 20, 150, 0, W, 21, 150, 0, W, 8, 0x23, 0x02]
        );
    }

    #[test]
    fn unchanged_touch_mode_is_not_rewritten() {
        let rt = KeyMode {
            touch: TouchMode::RapidTrigger,
            advanced: Advanced::Socd,
            high: 0,
        };
        let writes = [(W, 4, 1200), (A, 4, 1200)];
        let reads = [
            (W, 8, rt.to_u16()),
            (W, 25, 0),
            (A, 8, rt.to_u16()),
            (A, 25, 2),
        ];
        let mut k = kb(script(&reads, 1, &writes));
        let change = PerformanceChange {
            actuation: Some(Micrometers(1200)),
            ..Default::default()
        };
        assert_eq!(k.set_performance(&[W, A], &change), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(payload(&w[4]), [0x01, W, 4, 0xB0, 0x04, A, 4, 0xB0, 0x04]);
        assert_eq!(w.len(), 4 + 1 + 2);
    }

    #[test]
    fn deadzone_only_change_leaves_mode_alone() {
        let writes = [(W, 22, 100), (W, 23, 300)];
        let mut k = kb(script(&[(W, 8, 0x0001), (W, 25, 0)], 1, &writes));
        let change = PerformanceChange {
            top_dz: Some(Micrometers(100)),
            bottom_dz: Some(Micrometers(300)),
            ..Default::default()
        };
        assert_eq!(k.set_performance(&[W], &change), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(payload(&w[2]), [0x01, W, 22, 100, 0, W, 23, 0x2C, 0x01]);
    }

    #[test]
    fn actuation_above_the_switch_max_is_rejected_before_any_write() {
        // Switch 3 (Magneto) travels 3.3 mm.
        let mut k = kb(script(&[(W, 8, 0), (W, 25, 3)], 0, &[]));
        let change = PerformanceChange {
            actuation: Some(Micrometers(3400)),
            ..Default::default()
        };
        let e = k.set_performance(&[W], &change).unwrap_err();
        assert!(
            matches!(e, Error::InvalidInput(ref m) if m.contains("3300")),
            "{e:?}"
        );
        let w = k.session_mut().transport().written();
        assert!(w.iter().all(|f| payload(f)[0] == 0x00), "no SET was sent");
    }

    #[test]
    fn range_checks() {
        let cases = [
            PerformanceChange {
                actuation: Some(Micrometers(99)),
                ..Default::default()
            },
            PerformanceChange {
                rt_press: Some(Micrometers(4)),
                ..Default::default()
            },
            PerformanceChange {
                rt_release: Some(Micrometers(3401)),
                ..Default::default()
            },
        ];
        for c in cases {
            let mut k = kb(script(&[(W, 8, 0), (W, 25, 0)], 0, &[]));
            assert!(
                matches!(k.set_performance(&[W], &c), Err(Error::InvalidInput(_))),
                "{c:?}"
            );
        }
        // Deadzones and unknown switches fail before any I/O.
        for c in [
            PerformanceChange {
                top_dz: Some(Micrometers(1001)),
                ..Default::default()
            },
            PerformanceChange {
                switch: Some(9),
                ..Default::default()
            },
            PerformanceChange {
                touch: Some(TouchMode::Global),
                actuation: Some(Micrometers(1000)),
                ..Default::default()
            },
        ] {
            let mut k = kb(MockTransport::new());
            assert!(matches!(
                k.set_performance(&[W], &c),
                Err(Error::InvalidInput(_))
            ));
            assert!(k.session_mut().transport().written().is_empty());
        }
    }

    #[test]
    fn new_switch_sets_the_limit_and_is_written_first() {
        // Moving to Magneto (3.3 mm): 3.4 mm fails, 3.3 mm passes. The switch
        // is not read, since the change sets it; the kept RT values are.
        let change = PerformanceChange {
            switch: Some(3),
            actuation: Some(Micrometers(3300)),
            ..Default::default()
        };
        let writes = [(W, 25, 3), (W, 4, 3300), (W, 8, 0x0010)];
        let reads = [(W, 8, 0), (W, 20, 20), (W, 21, 20)];
        let mut k = kb(script(&reads, 1, &writes));
        assert_eq!(k.set_performance(&[W], &change), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(
            payload(&w[3]),
            [0x01, W, 25, 3, 0, W, 4, 0xE4, 0x0C, W, 8, 0x10, 0]
        );
    }

    #[test]
    fn new_switch_rejects_a_kept_actuation_beyond_its_travel() {
        let change = PerformanceChange {
            switch: Some(3),
            ..Default::default()
        };
        let reads = [(W, 8, 0), (W, 4, 3400), (W, 20, 20), (W, 21, 20)];
        let mut k = kb(script(&reads, 0, &[]));
        assert!(matches!(
            k.set_performance(&[W], &change),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn empty_input_and_duplicates() {
        let mut k = kb(MockTransport::new());
        let change = PerformanceChange {
            actuation: Some(Micrometers(1000)),
            ..Default::default()
        };
        assert_eq!(k.set_performance(&[], &change), Ok(()));
        assert_eq!(
            k.set_performance(&[W], &PerformanceChange::default()),
            Ok(())
        );
        assert!(matches!(
            k.set_performance(&[W, W], &change),
            Err(Error::InvalidInput(_))
        ));
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn verify_failure_is_reported() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(W, 8, 0x10));
        m.push_reply(key_reply(W, 25, 0));
        m.push_reply(key_reply(W, 4, 500)); // SET echo
        m.push_reply(key_reply(W, 4, 2000)); // read-back differs
        let mut k = kb(m);
        let change = PerformanceChange {
            actuation: Some(Micrometers(500)),
            ..Default::default()
        };
        let e = k.set_performance(&[W], &change).unwrap_err();
        assert!(
            matches!(e, Error::Io(ref m) if m.starts_with("verify failed")),
            "{e:?}"
        );
    }
}

//! CMD 0x23 KEY: per-key parameter store.

use crate::frame::{Cmd, Request, Response};

/// Entries per SET packet: 1 + 14 * 4 = 57 bytes <= MAX_PAYLOAD.
const ENTRIES_PER_PACKET: usize = 14;

/// A per-key parameter slot.
///
/// `Layer(n)`, `DksBinding(n)` and `DksTrps(n)` take `n` in `0..=3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyParam {
    Layer(u8),
    Actuation,
    DksFirst,
    Mode,
    DksBinding(u8),
    DksTrps(u8),
    MtDelay,
    RtPress,
    RtRelease,
    TopDeadzone,
    BottomDeadzone,
    Switch,
}

impl KeyParam {
    /// Layout index on the wire. `n` outside `0..=3` is a programming error:
    /// debug builds assert, release builds mask with `n & 3`.
    pub fn layout(self) -> u8 {
        match self {
            KeyParam::Layer(n) => {
                debug_assert!(n <= 3);
                n & 3
            }
            KeyParam::Actuation => 4,
            KeyParam::DksFirst => 5,
            KeyParam::Mode => 8,
            KeyParam::DksBinding(n) => {
                debug_assert!(n <= 3);
                9 + (n & 3)
            }
            KeyParam::DksTrps(n) => {
                debug_assert!(n <= 3);
                13 + (n & 3)
            }
            KeyParam::MtDelay => 19,
            KeyParam::RtPress => 20,
            KeyParam::RtRelease => 21,
            KeyParam::TopDeadzone => 22,
            KeyParam::BottomDeadzone => 23,
            KeyParam::Switch => 25,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchMode {
    Global = 0,
    Single = 1,
    RapidTrigger = 2,
}

impl TouchMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Global),
            1 => Some(Self::Single),
            2 => Some(Self::RapidTrigger),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Advanced {
    None = 0,
    Dks = 1,
    Mpt = 2,
    Mt = 3,
    Tgl = 4,
    End = 5,
    Macro = 6,
    Socd = 8,
}

impl Advanced {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Dks),
            2 => Some(Self::Mpt),
            3 => Some(Self::Mt),
            4 => Some(Self::Tgl),
            5 => Some(Self::End),
            6 => Some(Self::Macro),
            8 => Some(Self::Socd),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyMode {
    pub touch: TouchMode,
    pub advanced: Advanced,
}

impl KeyMode {
    pub fn to_u16(self) -> u16 {
        (u16::from(self.touch as u8) << 4) | u16::from(self.advanced as u8)
    }

    /// Uses only the low byte: touch = high nibble, advanced = low nibble.
    pub fn from_u16(v: u16) -> Option<KeyMode> {
        let low = (v & 0xFF) as u8;
        Some(KeyMode {
            touch: TouchMode::from_u8(low >> 4)?,
            advanced: Advanced::from_u8(low & 0xF)?,
        })
    }
}

pub fn get(key: u8, p: KeyParam) -> Request {
    Request {
        cmd: Cmd::Key,
        payload: vec![0x00, key, p.layout(), 0, 0],
    }
}

/// Batches entries into packets of at most 14; empty input yields no packets.
pub fn set(entries: &[(u8, KeyParam, u16)]) -> Vec<Request> {
    entries
        .chunks(ENTRIES_PER_PACKET)
        .map(|chunk| {
            let mut payload = Vec::with_capacity(1 + chunk.len() * 4);
            payload.push(0x01);
            for &(key, p, v) in chunk {
                let [lo, hi] = v.to_le_bytes();
                payload.extend_from_slice(&[key, p.layout(), lo, hi]);
            }
            Request {
                cmd: Cmd::Key,
                payload,
            }
        })
        .collect()
}

/// Returns `(key, layout, value)` from a KEY reply, or None for other commands.
pub fn parse_value(r: &Response) -> Option<(u8, u8, u16)> {
    if r.cmd != Cmd::Key as u8 {
        return None;
    }
    let b = r.body();
    Some((b[1], b[2], u16::from_le_bytes([b[3], b[4]])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::REPORT_LEN;

    fn resp(cmd: u8, body: &[u8]) -> Response {
        let mut raw = [0u8; REPORT_LEN];
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        Response::decode(&raw).unwrap()
    }

    #[test]
    fn get_actuation_bytes() {
        let r = get(0x1A, KeyParam::Actuation);
        assert_eq!(r.cmd, Cmd::Key);
        assert_eq!(r.payload, [0x00, 0x1A, 0x04, 0x00, 0x00]);
    }

    #[test]
    fn layouts() {
        assert_eq!(KeyParam::Layer(3).layout(), 3);
        assert_eq!(KeyParam::DksBinding(3).layout(), 12);
        assert_eq!(KeyParam::DksTrps(0).layout(), 13);
        assert_eq!(KeyParam::DksTrps(3).layout(), 16);
        assert_eq!(KeyParam::Switch.layout(), 25);
        assert_eq!(KeyParam::BottomDeadzone.layout(), 23);
    }

    #[test]
    fn set_empty() {
        assert!(set(&[]).is_empty());
    }

    #[test]
    fn set_splits_at_14() {
        let entries: Vec<_> = (0..15u8)
            .map(|i| (0x10 + i, KeyParam::Actuation, 0x0190 + u16::from(i)))
            .collect();
        let reqs = set(&entries);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].payload.len(), 1 + 14 * 4);
        assert_eq!(reqs[1].payload.len(), 1 + 4);
        assert!(reqs.iter().all(|r| r.payload[0] == 0x01));
        assert_eq!(reqs[1].payload, [0x01, 0x1E, 4, 0x9E, 0x01]);
    }

    #[test]
    fn set_payload_fits() {
        let entries = vec![(1u8, KeyParam::RtPress, 100u16); 14];
        let reqs = set(&entries);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].payload.len(), 57);
        assert!(reqs[0].encode().is_ok());
    }

    #[test]
    fn mode_roundtrip() {
        let m = KeyMode {
            touch: TouchMode::RapidTrigger,
            advanced: Advanced::None,
        };
        assert_eq!(m.to_u16(), 0x20);
        assert_eq!(KeyMode::from_u16(0x20), Some(m));
        assert_eq!(
            KeyMode::from_u16(0x13),
            Some(KeyMode {
                touch: TouchMode::Single,
                advanced: Advanced::Mt
            })
        );
        assert_eq!(KeyMode::from_u16(0x07), None);
        assert_eq!(KeyMode::from_u16(0x30), None);
        assert_eq!(
            KeyMode::from_u16(0xAB13).map(|m| m.advanced),
            Some(Advanced::Mt)
        );
    }

    #[test]
    fn parse_value_ok() {
        let r = resp(0x23, &[0, 0x1A, 4, 0x90, 0x01]);
        assert_eq!(parse_value(&r), Some((0x1A, 4, 400)));
        assert_eq!(parse_value(&resp(0x24, &[0, 0x1A, 4, 0x90, 0x01])), None);
    }
}

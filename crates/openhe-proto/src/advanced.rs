//! CMD 0x24-0x28 and 0x2C advanced keys: DKS, MPT, MT, TGL, END and SOCD.
//!
//! Byte layouts follow PROTOCOL section 3.6, using the SOCD >= 1.0.7 format
//! (the target firmware is App V1.1.3). All u16 values are little-endian.
//! Builders validate before any I/O is possible; parsers never validate
//! ranges, so a device holding out-of-range data can still be read.

use crate::frame::{Cmd, Request, Response};
use crate::units::Micrometers;
use thiserror::Error;

/// Shallowest allowed depth (0.1 mm).
pub const MIN_DEPTH_UM: u16 = 100;
/// Deepest allowed depth, and the fixed DKS bottom-out (3.4 mm).
pub const MAX_DEPTH_UM: u16 = 3400;
/// MT / TGL delay bounds in milliseconds.
pub const MIN_DELAY_MS: u16 = 1;
pub const MAX_DELAY_MS: u16 = 1000;
/// Largest SOCD delay value.
pub const MAX_SOCD_DELAY: u16 = 50;

/// DKS `trps` bits (PROTOCOL 3.6).
pub const TRPS_PRESS_P0: u8 = 0x01;
pub const TRPS_HOLD_P0_P1: u8 = 0x02;
pub const TRPS_PRESS_P1: u8 = 0x04;
pub const TRPS_HOLD_P1_P2: u8 = 0x18;
pub const TRPS_PRESS_P2: u8 = 0x20;
pub const TRPS_HOLD_P2_P3: u8 = 0x40;
pub const TRPS_PRESS_P3: u8 = 0x80;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum AdvancedError {
    #[error("{what} of {value_um} um is outside 100..=3400 um")]
    DepthRange { what: &'static str, value_um: u16 },
    #[error("MPT depths must be strictly increasing, got {0:?} um")]
    DepthOrder([u16; 3]),
    #[error("DKS bottom must be 3400 um, got {0} um")]
    DksBottom(u16),
    #[error("{what} delay of {ms} ms is outside 1..=1000 ms")]
    Delay { what: &'static str, ms: u16 },
    #[error("SOCD delay {0} is outside 0..=50")]
    SocdDelay(u16),
    #[error("not a 0x{expected:02X} reply (command 0x{got:02X})")]
    WrongReply { expected: u8, got: u8 },
    #[error("unknown SOCD mode byte {0}")]
    SocdMode(u8),
    #[error("unknown SOCD bind type byte {0}")]
    SocdType(u8),
    #[error("DKS trigger value {0} does not fit in one byte")]
    TrpsRange(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dks {
    pub bindings: [u16; 4],
    pub trps: [u8; 4],
    pub first_actuation: Micrometers,
    pub bottom: Micrometers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mt {
    pub hold: u16,
    pub tap: u16,
    pub delay_ms: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tgl {
    pub code: u16,
    pub delay_ms: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mpt {
    pub codes: [u16; 3],
    pub depths: [Micrometers; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct End {
    pub code: u16,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocdMode {
    LastInput = 0,
    Key1Wins = 1,
    Key2Wins = 2,
    Neutral = 3,
}

impl SocdMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::LastInput),
            1 => Some(Self::Key1Wins),
            2 => Some(Self::Key2Wins),
            3 => Some(Self::Neutral),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socd {
    pub pos1: u8,
    pub pos2: u8,
    pub key1: u16,
    pub key2: u16,
    /// `false` binds the two physical positions, `true` binds `key1`/`key2`.
    pub bind_values: bool,
    pub mode: SocdMode,
    pub delay: u16,
}

/// Packs the seven DKS UI flags `[press@P0, hold P0-P1, press@P1, hold P1-P2,
/// press@P2, hold P2-P3, press@P3]` into a `trps` byte.
pub fn trps_encode(flags: [bool; 7]) -> u8 {
    const BITS: [u8; 7] = [
        TRPS_PRESS_P0,
        TRPS_HOLD_P0_P1,
        TRPS_PRESS_P1,
        TRPS_HOLD_P1_P2,
        TRPS_PRESS_P2,
        TRPS_HOLD_P2_P3,
        TRPS_PRESS_P3,
    ];
    flags
        .iter()
        .zip(BITS)
        .filter(|(on, _)| **on)
        .fold(0, |acc, (_, bit)| acc | bit)
}

/// Inverse of [`trps_encode`]; either bit of the 0x18 pair is the hold P1-P2 flag.
pub fn trps_decode(trps: u8) -> [bool; 7] {
    [
        trps & TRPS_PRESS_P0 != 0,
        trps & TRPS_HOLD_P0_P1 != 0,
        trps & TRPS_PRESS_P1 != 0,
        trps & TRPS_HOLD_P1_P2 != 0,
        trps & TRPS_PRESS_P2 != 0,
        trps & TRPS_HOLD_P2_P3 != 0,
        trps & TRPS_PRESS_P3 != 0,
    ]
}

impl Dks {
    /// Assembles a DKS from the KEY layouts it is read back from (bindings
    /// 9..12, trps 13..16, first actuation 5). The bottom is fixed at 3400 um
    /// because the keyboard does not report it.
    pub fn from_key_params(
        bindings: [u16; 4],
        trps: [u16; 4],
        first_actuation: u16,
    ) -> Result<Dks, AdvancedError> {
        let mut t = [0u8; 4];
        for (dst, &src) in t.iter_mut().zip(&trps) {
            *dst = u8::try_from(src).map_err(|_| AdvancedError::TrpsRange(src))?;
        }
        Ok(Dks {
            bindings,
            trps: t,
            first_actuation: Micrometers(first_actuation),
            bottom: Micrometers(MAX_DEPTH_UM),
        })
    }
}

impl Mt {
    /// Assembles an MT from KEY layouts 9 (hold), 10 (tap) and 19 (delay in
    /// 10 ms units).
    pub fn from_key_params(hold: u16, tap: u16, delay_units: u16) -> Mt {
        Mt {
            hold,
            tap,
            delay_ms: delay_units.saturating_mul(10),
        }
    }
}

/// Milliseconds to the wire's 10 ms units: rounded to nearest, at least 1.
fn delay_units(what: &'static str, ms: u16) -> Result<u8, AdvancedError> {
    if !(MIN_DELAY_MS..=MAX_DELAY_MS).contains(&ms) {
        return Err(AdvancedError::Delay { what, ms });
    }
    // 1..=1000 ms gives at most 100, so the cast cannot truncate.
    Ok(((ms + 5) / 10).max(1) as u8)
}

fn depth(what: &'static str, d: Micrometers) -> Result<(), AdvancedError> {
    if (MIN_DEPTH_UM..=MAX_DEPTH_UM).contains(&d.0) {
        Ok(())
    } else {
        Err(AdvancedError::DepthRange {
            what,
            value_um: d.0,
        })
    }
}

fn req(cmd: Cmd, payload: Vec<u8>) -> Request {
    Request { cmd, payload }
}

/// DKS SET: `[01, key, b1..b4 u16, trps1..4, first u16, bottom u16]` (18 bytes).
pub fn dks_set(key: u8, dks: &Dks) -> Result<Request, AdvancedError> {
    depth("DKS first actuation", dks.first_actuation)?;
    if dks.bottom.0 != MAX_DEPTH_UM {
        return Err(AdvancedError::DksBottom(dks.bottom.0));
    }
    let mut p = vec![0x01, key];
    for b in dks.bindings {
        p.extend_from_slice(&b.to_le_bytes());
    }
    p.extend_from_slice(&dks.trps);
    p.extend_from_slice(&dks.first_actuation.to_le());
    p.extend_from_slice(&dks.bottom.to_le());
    Ok(req(Cmd::Dks, p))
}

/// MPT GET: `[00, key]`.
pub fn mpt_get(key: u8) -> Request {
    req(Cmd::Mpt, vec![0x00, key])
}

/// MPT SET: `[01, key, k1, k2, k3 u16, d1, d2, d3 u16 um]` (14 bytes).
pub fn mpt_set(key: u8, mpt: &Mpt) -> Result<Request, AdvancedError> {
    for d in mpt.depths {
        depth("MPT depth", d)?;
    }
    if !(mpt.depths[0] < mpt.depths[1] && mpt.depths[1] < mpt.depths[2]) {
        return Err(AdvancedError::DepthOrder(mpt.depths.map(|d| d.0)));
    }
    let mut p = vec![0x01, key];
    for c in mpt.codes {
        p.extend_from_slice(&c.to_le_bytes());
    }
    for d in mpt.depths {
        p.extend_from_slice(&d.to_le());
    }
    Ok(req(Cmd::Mpt, p))
}

/// MT SET: `[01, key, hold u16, tap u16, delay/10]` (7 bytes).
pub fn mt_set(key: u8, mt: &Mt) -> Result<Request, AdvancedError> {
    let units = delay_units("MT", mt.delay_ms)?;
    let mut p = vec![0x01, key];
    p.extend_from_slice(&mt.hold.to_le_bytes());
    p.extend_from_slice(&mt.tap.to_le_bytes());
    p.push(units);
    Ok(req(Cmd::Mt, p))
}

/// TGL GET: `[00, key, 0, 0, 0]`.
pub fn tgl_get(key: u8) -> Request {
    req(Cmd::Tgl, vec![0x00, key, 0, 0, 0])
}

/// TGL SET: `[01, key, code u16, delay/10]` (5 bytes).
pub fn tgl_set(key: u8, tgl: &Tgl) -> Result<Request, AdvancedError> {
    let units = delay_units("TGL", tgl.delay_ms)?;
    let mut p = vec![0x01, key];
    p.extend_from_slice(&tgl.code.to_le_bytes());
    p.push(units);
    Ok(req(Cmd::Tgl, p))
}

/// END GET: `[00, key, 0, 0]`.
pub fn end_get(key: u8) -> Request {
    req(Cmd::End, vec![0x00, key, 0, 0])
}

/// END SET: `[01, key, code u16]` (4 bytes).
pub fn end_set(key: u8, end: &End) -> Request {
    let [lo, hi] = end.code.to_le_bytes();
    req(Cmd::End, vec![0x01, key, lo, hi])
}

/// SOCD GET: `[00, key]`.
pub fn socd_get(key: u8) -> Request {
    req(Cmd::Socd, vec![0x00, key])
}

/// SOCD SET (firmware >= 1.0.7): `[01, pos1, pos2, key1 u16, key2 u16, type,
/// mode, delay u16]` (11 bytes).
pub fn socd_set(socd: &Socd) -> Result<Request, AdvancedError> {
    if socd.delay > MAX_SOCD_DELAY {
        return Err(AdvancedError::SocdDelay(socd.delay));
    }
    let mut p = vec![0x01, socd.pos1, socd.pos2];
    p.extend_from_slice(&socd.key1.to_le_bytes());
    p.extend_from_slice(&socd.key2.to_le_bytes());
    p.push(u8::from(socd.bind_values));
    p.push(socd.mode as u8);
    p.extend_from_slice(&socd.delay.to_le_bytes());
    Ok(req(Cmd::Socd, p))
}

fn body_of(r: &Response, cmd: Cmd) -> Result<&[u8], AdvancedError> {
    if r.cmd == cmd as u8 {
        Ok(r.body())
    } else {
        Err(AdvancedError::WrongReply {
            expected: cmd as u8,
            got: r.cmd,
        })
    }
}

fn le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

/// MPT reply: `R = [st, key, k1, k2, k3, d1, d2, d3]` (u16 each after the key).
pub fn parse_mpt(r: &Response) -> Result<Mpt, AdvancedError> {
    let b = body_of(r, Cmd::Mpt)?;
    Ok(Mpt {
        codes: [le(b, 2), le(b, 4), le(b, 6)],
        depths: [
            Micrometers(le(b, 8)),
            Micrometers(le(b, 10)),
            Micrometers(le(b, 12)),
        ],
    })
}

/// TGL reply: `R = [st, key, code u16, delay/10]`.
pub fn parse_tgl(r: &Response) -> Result<Tgl, AdvancedError> {
    let b = body_of(r, Cmd::Tgl)?;
    Ok(Tgl {
        code: le(b, 2),
        delay_ms: u16::from(b[4]) * 10,
    })
}

/// END reply: `R = [st, key, code u16, delay u16]`; the delay is ignored.
pub fn parse_end(r: &Response) -> Result<End, AdvancedError> {
    let b = body_of(r, Cmd::End)?;
    Ok(End { code: le(b, 2) })
}

/// SOCD reply (>= 1.0.7): `R = [st, pos1, pos2, k1 u16, k2 u16, type, mode, delay u16]`.
pub fn parse_socd(r: &Response) -> Result<Socd, AdvancedError> {
    let b = body_of(r, Cmd::Socd)?;
    let bind_values = match b[7] {
        0 => false,
        1 => true,
        v => return Err(AdvancedError::SocdType(v)),
    };
    let mode = SocdMode::from_u8(b[8]).ok_or(AdvancedError::SocdMode(b[8]))?;
    Ok(Socd {
        pos1: b[1],
        pos2: b[2],
        key1: le(b, 3),
        key2: le(b, 5),
        bind_values,
        mode,
        delay: le(b, 9),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::REPORT_LEN;

    /// Asserts the encoded report starts with `head` and is zero after it.
    fn check(req: &Request, head: &[u8]) {
        let out = req.encode().unwrap();
        assert_eq!(&out[..head.len()], head);
        assert!(out[head.len()..].iter().all(|&b| b == 0));
    }

    fn resp(cmd: u8, body: &[u8]) -> Response {
        let mut raw = [0u8; REPORT_LEN];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        Response::decode(&raw).unwrap()
    }

    fn um(v: u16) -> Micrometers {
        Micrometers(v)
    }

    fn dks() -> Dks {
        Dks {
            bindings: [0x04, 0x05, 0x06, 0x07],
            trps: [0x01, 0x02, 0x04, 0x18],
            first_actuation: um(1400),
            bottom: um(3400),
        }
    }

    fn mpt() -> Mpt {
        Mpt {
            codes: [0x04, 0x05, 0x06],
            depths: [um(1000), um(2000), um(3000)],
        }
    }

    fn socd() -> Socd {
        Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 0x04,
            key2: 0x07,
            bind_values: true,
            mode: SocdMode::Key2Wins,
            delay: 50,
        }
    }

    // Checksum = (0x35 + 0x5C + LEN + CMD + last payload byte) & 0xFF = 0x91 + ...

    #[test]
    fn golden_dks_set() {
        // LEN 0x12, CMD 0x26, last byte 0x0D: 0x91 + 0x12 + 0x26 + 0x0D = 0xD6.
        let r = dks_set(0x0A, &dks()).unwrap();
        assert_eq!(r.cmd, Cmd::Dks);
        check(
            &r,
            &[
                0x5C, 0x12, 0x26, 0xD6, // header
                0x01, 0x0A, // SET, key
                0x04, 0x00, 0x05, 0x00, 0x06, 0x00, 0x07, 0x00, // bindings
                0x01, 0x02, 0x04, 0x18, // trps
                0x78, 0x05, // first actuation 1400 = 0x0578
                0x48, 0x0D, // bottom 3400 = 0x0D48
            ],
        );
    }

    #[test]
    fn golden_mpt_set_and_get() {
        // LEN 0x0E, CMD 0x27, last byte 0x0B: 0x91 + 0x0E + 0x27 + 0x0B = 0xD1.
        let r = mpt_set(0x0A, &mpt()).unwrap();
        assert_eq!(r.cmd, Cmd::Mpt);
        check(
            &r,
            &[
                0x5C, 0x0E, 0x27, 0xD1, //
                0x01, 0x0A, 0x04, 0x00, 0x05, 0x00, 0x06, 0x00, //
                0xE8, 0x03, 0xD0, 0x07, 0xB8, 0x0B, // 1000, 2000, 3000
            ],
        );
        // LEN 2, last byte 0x0A: 0x91 + 2 + 0x27 + 0x0A = 0xC4.
        check(&mpt_get(0x0A), &[0x5C, 0x02, 0x27, 0xC4, 0x00, 0x0A]);
    }

    #[test]
    fn golden_mt_set() {
        // Caps (0x39): hold LCtrl 0x00E0, tap Esc 0x0029, 200 ms = 20 units.
        // LEN 7, CMD 0x24, last byte 0x14: 0x91 + 7 + 0x24 + 0x14 = 0xD0.
        let mt = Mt {
            hold: 0x00E0,
            tap: 0x0029,
            delay_ms: 200,
        };
        let r = mt_set(0x39, &mt).unwrap();
        assert_eq!(r.cmd, Cmd::Mt);
        check(
            &r,
            &[
                0x5C, 0x07, 0x24, 0xD0, //
                0x01, 0x39, 0xE0, 0x00, 0x29, 0x00, 0x14,
            ],
        );
    }

    #[test]
    fn golden_tgl_set_and_get() {
        // 300 ms = 30 units = 0x1E. LEN 5, CMD 0x25: 0x91 + 5 + 0x25 + 0x1E = 0xD9.
        let tgl = Tgl {
            code: 0x0004,
            delay_ms: 300,
        };
        let r = tgl_set(0x0A, &tgl).unwrap();
        assert_eq!(r.cmd, Cmd::Tgl);
        check(&r, &[0x5C, 0x05, 0x25, 0xD9, 0x01, 0x0A, 0x04, 0x00, 0x1E]);
        // GET: last byte 0: 0x91 + 5 + 0x25 = 0xBB.
        check(
            &tgl_get(0x0A),
            &[0x5C, 0x05, 0x25, 0xBB, 0x00, 0x0A, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn golden_end_set_and_get() {
        // LEN 4, CMD 0x28, last byte 0: 0x91 + 4 + 0x28 = 0xBD for both.
        let r = end_set(0x0A, &End { code: 0x0005 });
        assert_eq!(r.cmd, Cmd::End);
        check(&r, &[0x5C, 0x04, 0x28, 0xBD, 0x01, 0x0A, 0x05, 0x00]);
        check(
            &end_get(0x0A),
            &[0x5C, 0x04, 0x28, 0xBD, 0x00, 0x0A, 0x00, 0x00],
        );
    }

    #[test]
    fn golden_socd_set_and_get() {
        // LEN 0x0B, CMD 0x2C, last byte (delay high) 0: 0x91 + 0x0B + 0x2C = 0xC8.
        let r = socd_set(&socd()).unwrap();
        assert_eq!(r.cmd, Cmd::Socd);
        check(
            &r,
            &[
                0x5C, 0x0B, 0x2C, 0xC8, //
                0x01, 0x04, 0x07, // SET, pos1, pos2
                0x04, 0x00, 0x07, 0x00, // key1, key2
                0x01, 0x02, // type = bind values, mode = key 2 wins
                0x32, 0x00, // delay 50
            ],
        );
        // LEN 2, last byte 0x04: 0x91 + 2 + 0x2C + 4 = 0xC3.
        check(&socd_get(0x04), &[0x5C, 0x02, 0x2C, 0xC3, 0x00, 0x04]);
    }

    #[test]
    fn socd_bind_position_and_modes_on_the_wire() {
        for (mode, byte) in [
            (SocdMode::LastInput, 0u8),
            (SocdMode::Key1Wins, 1),
            (SocdMode::Key2Wins, 2),
            (SocdMode::Neutral, 3),
        ] {
            let s = Socd {
                bind_values: false,
                mode,
                ..socd()
            };
            let p = socd_set(&s).unwrap().payload;
            assert_eq!(p[7], 0, "bind position");
            assert_eq!(p[8], byte);
            assert_eq!(SocdMode::from_u8(byte), Some(mode));
        }
        assert_eq!(SocdMode::from_u8(4), None);
    }

    #[test]
    fn trps_bits() {
        assert_eq!(TRPS_PRESS_P0, 0x01);
        assert_eq!(TRPS_HOLD_P0_P1, 0x02);
        assert_eq!(TRPS_PRESS_P1, 0x04);
        assert_eq!(TRPS_HOLD_P1_P2, 0x18);
        assert_eq!(TRPS_PRESS_P2, 0x20);
        assert_eq!(TRPS_HOLD_P2_P3, 0x40);
        assert_eq!(TRPS_PRESS_P3, 0x80);
        assert_eq!(trps_encode([true; 7]), 0xFF);
        assert_eq!(trps_encode([false; 7]), 0);
        assert_eq!(
            trps_encode([true, false, true, false, false, false, true]),
            0x85
        );
        assert_eq!(
            trps_encode([false, false, false, true, false, false, false]),
            0x18
        );
        assert_eq!(
            trps_decode(0x85),
            [true, false, true, false, false, false, true]
        );
        // Either bit of the 0x18 pair counts as the hold P1->P2 flag.
        assert!(trps_decode(0x08)[3]);
        assert!(trps_decode(0x10)[3]);
        for flags in [[true, true, false, true, true, false, true], [false; 7]] {
            assert_eq!(trps_decode(trps_encode(flags)), flags);
        }
    }

    #[test]
    fn dks_from_key_params() {
        let d = Dks::from_key_params([4, 5, 6, 7], [1, 2, 4, 0x18], 1400).unwrap();
        assert_eq!(d, dks());
        assert_eq!(
            Dks::from_key_params([4, 5, 6, 7], [1, 2, 256, 0x18], 1400),
            Err(AdvancedError::TrpsRange(256))
        );
    }

    #[test]
    fn mt_from_key_params_scales_units() {
        assert_eq!(
            Mt::from_key_params(0xE0, 0x29, 20),
            Mt {
                hold: 0xE0,
                tap: 0x29,
                delay_ms: 200
            }
        );
    }

    #[test]
    fn dks_validation() {
        for bad in [99, 3401, 0] {
            let d = Dks {
                first_actuation: um(bad),
                ..dks()
            };
            assert_eq!(
                dks_set(0x0A, &d),
                Err(AdvancedError::DepthRange {
                    what: "DKS first actuation",
                    value_um: bad
                })
            );
        }
        for ok in [100, 3400] {
            let d = Dks {
                first_actuation: um(ok),
                ..dks()
            };
            assert!(dks_set(0x0A, &d).is_ok());
        }
        let d = Dks {
            bottom: um(3000),
            ..dks()
        };
        assert_eq!(dks_set(0x0A, &d), Err(AdvancedError::DksBottom(3000)));
    }

    #[test]
    fn mpt_validation() {
        let with = |d: [u16; 3]| Mpt {
            depths: [um(d[0]), um(d[1]), um(d[2])],
            ..mpt()
        };
        assert_eq!(
            mpt_set(1, &with([99, 2000, 3000])),
            Err(AdvancedError::DepthRange {
                what: "MPT depth",
                value_um: 99
            })
        );
        assert_eq!(
            mpt_set(1, &with([1000, 2000, 3401])),
            Err(AdvancedError::DepthRange {
                what: "MPT depth",
                value_um: 3401
            })
        );
        for d in [[1000, 1000, 3000], [1000, 2000, 2000], [3000, 2000, 1000]] {
            assert_eq!(mpt_set(1, &with(d)), Err(AdvancedError::DepthOrder(d)));
        }
        assert!(mpt_set(1, &with([100, 101, 3400])).is_ok());
    }

    #[test]
    fn mt_and_tgl_delay_validation() {
        for bad in [0u16, 1001, u16::MAX] {
            let mt = Mt {
                hold: 1,
                tap: 2,
                delay_ms: bad,
            };
            assert_eq!(
                mt_set(1, &mt),
                Err(AdvancedError::Delay {
                    what: "MT",
                    ms: bad
                })
            );
            let tgl = Tgl {
                code: 1,
                delay_ms: bad,
            };
            assert_eq!(
                tgl_set(1, &tgl),
                Err(AdvancedError::Delay {
                    what: "TGL",
                    ms: bad
                })
            );
        }
    }

    #[test]
    fn delays_are_rounded_to_ten_ms_units_with_a_minimum_of_one() {
        let units = |ms: u16| {
            let r = mt_set(
                1,
                &Mt {
                    hold: 0,
                    tap: 0,
                    delay_ms: ms,
                },
            )
            .unwrap();
            let t = tgl_set(
                1,
                &Tgl {
                    code: 0,
                    delay_ms: ms,
                },
            )
            .unwrap();
            assert_eq!(r.payload[6], t.payload[4]);
            r.payload[6]
        };
        assert_eq!(units(1), 1);
        assert_eq!(units(4), 1);
        assert_eq!(units(14), 1);
        assert_eq!(units(15), 2);
        assert_eq!(units(205), 21);
        assert_eq!(units(1000), 100);
    }

    #[test]
    fn socd_validation() {
        let s = Socd {
            delay: 51,
            ..socd()
        };
        assert_eq!(socd_set(&s), Err(AdvancedError::SocdDelay(51)));
        for ok in [0, 50] {
            assert!(socd_set(&Socd {
                delay: ok,
                ..socd()
            })
            .is_ok());
        }
    }

    /// A reply body for a GET: status 0, the key, then the SET payload's data.
    fn echo_body(key: u8, data: &[u8]) -> Vec<u8> {
        let mut b = vec![0, key];
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn mpt_round_trip() {
        let m = mpt();
        let set = mpt_set(0x0A, &m).unwrap();
        let r = resp(0x27, &echo_body(0x0A, &set.payload[2..]));
        assert_eq!(parse_mpt(&r), Ok(m));
    }

    #[test]
    fn mpt_parse_does_not_validate_ranges() {
        let bad = Mpt {
            codes: [1, 2, 3],
            depths: [um(3000), um(10), um(5)],
        };
        let mut data = Vec::new();
        for c in bad.codes {
            data.extend_from_slice(&c.to_le_bytes());
        }
        for d in bad.depths {
            data.extend_from_slice(&d.to_le());
        }
        assert_eq!(parse_mpt(&resp(0x27, &echo_body(1, &data))), Ok(bad));
    }

    #[test]
    fn tgl_round_trip() {
        let t = Tgl {
            code: 0x0004,
            delay_ms: 300,
        };
        let set = tgl_set(0x0A, &t).unwrap();
        let r = resp(0x25, &echo_body(0x0A, &set.payload[2..]));
        assert_eq!(parse_tgl(&r), Ok(t));
    }

    #[test]
    fn end_round_trip() {
        let e = End { code: 0x0005 };
        let set = end_set(0x0A, &e);
        let r = resp(0x28, &echo_body(0x0A, &set.payload[2..]));
        assert_eq!(parse_end(&r), Ok(e));
        // The delay field R[4..5] of a GET reply is ignored.
        let r = resp(0x28, &[0, 0x0A, 0x05, 0x00, 0x2C, 0x01]);
        assert_eq!(parse_end(&r), Ok(e));
    }

    #[test]
    fn socd_round_trip() {
        for bind_values in [false, true] {
            for mode in [
                SocdMode::LastInput,
                SocdMode::Key1Wins,
                SocdMode::Key2Wins,
                SocdMode::Neutral,
            ] {
                let s = Socd {
                    bind_values,
                    mode,
                    ..socd()
                };
                let set = socd_set(&s).unwrap();
                // Reply R = [st, pos1, pos2, k1, k2, type, mode, delay]; the
                // SET payload with its flag byte replaced by the status has
                // the same shape.
                let mut body = set.payload.clone();
                body[0] = 0;
                assert_eq!(parse_socd(&resp(0x2C, &body)), Ok(s));
            }
        }
    }

    #[test]
    fn parsers_reject_other_commands() {
        let other = resp(0x23, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let want = |expected| {
            Err::<(), _>(AdvancedError::WrongReply {
                expected,
                got: 0x23,
            })
        };
        assert_eq!(parse_mpt(&other).map(drop), want(0x27));
        assert_eq!(parse_tgl(&other).map(drop), want(0x25));
        assert_eq!(parse_end(&other).map(drop), want(0x28));
        assert_eq!(parse_socd(&other).map(drop), want(0x2C));
    }

    #[test]
    fn socd_parse_rejects_unknown_bytes() {
        let body = |ty: u8, mode: u8| [0, 4, 7, 4, 0, 7, 0, ty, mode, 10, 0];
        assert_eq!(
            parse_socd(&resp(0x2C, &body(2, 0))),
            Err(AdvancedError::SocdType(2))
        );
        assert_eq!(
            parse_socd(&resp(0x2C, &body(0, 9))),
            Err(AdvancedError::SocdMode(9))
        );
    }
}

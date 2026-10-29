//! CMD 0x20 MACRO (action data) and CMD 0x21 MACRO_MODE (playback settings).
//!
//! Binding a macro to a key is three steps (PROTOCOL 3.7): MACRO data packets,
//! KEY SET Mode `(touch<<4)|6`, then MACRO_MODE. Every bind writes its data at
//! address 256 with id 0, so whether several keys can hold different macros at
//! the same time is unverified (PROTOCOL open question 8).

use crate::frame::{Cmd, Request, Response};
use thiserror::Error;

/// Address of the first action in the macro store.
pub const BASE_ADDR: u16 = 256;
/// Actions per MACRO data packet (1 + 2 + 1 + 9 * 6 = 58 bytes <= 60).
pub const ACTIONS_PER_PACKET: usize = 9;
/// Most actions one macro may hold (`len` is a single byte).
pub const MAX_ACTIONS: usize = 255;
/// Delays and the playback interval are 24-bit.
pub const MAX_U24: u32 = 0x00FF_FFFF;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MacroError {
    #[error("a macro needs at least one action")]
    Empty,
    #[error("{0} actions is over the limit of 255")]
    TooManyActions(usize),
    #[error("action {index}: delay {delay_ms} ms does not fit in 24 bits")]
    DelayTooLong { index: usize, delay_ms: u32 },
    #[error("repeat count must be at least 1")]
    ZeroRepeat,
    #[error("interval {0} ms does not fit in 24 bits")]
    IntervalTooLong(u32),
    #[error("reply is not a MACRO_MODE reply")]
    WrongReply,
    #[error("unknown play mode {0}")]
    UnknownPlayMode(u8),
}

/// One recorded key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroAction {
    /// Key code from the keycode table (PROTOCOL 4.3).
    pub code: u16,
    /// true = key down, false = key up.
    pub down: bool,
    /// Delay in milliseconds, below 2^24.
    pub delay_ms: u32,
}

/// How a bound macro plays.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// Play once per press.
    Once = 0,
    /// Press again to stop.
    Toggle = 1,
    /// Stop as soon as the key is released.
    WhileHeld = 2,
    /// On release, finish the current cycle first.
    WhileHeldFinish = 3,
}

impl PlayMode {
    pub fn from_u8(v: u8) -> Option<PlayMode> {
        match v {
            0 => Some(PlayMode::Once),
            1 => Some(PlayMode::Toggle),
            2 => Some(PlayMode::WhileHeld),
            3 => Some(PlayMode::WhileHeldFinish),
            _ => None,
        }
    }
}

/// The playback settings stored for a key's macro (MACRO_MODE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroBinding {
    /// Number of actions.
    pub len: u8,
    pub mode: PlayMode,
    /// Loop count.
    pub repeat: u16,
    /// Interval between loops in milliseconds.
    pub interval_ms: u32,
}

fn validate_actions(actions: &[MacroAction]) -> Result<(), MacroError> {
    if actions.is_empty() {
        return Err(MacroError::Empty);
    }
    if actions.len() > MAX_ACTIONS {
        return Err(MacroError::TooManyActions(actions.len()));
    }
    for (index, a) in actions.iter().enumerate() {
        if a.delay_ms > MAX_U24 {
            return Err(MacroError::DelayTooLong {
                index,
                delay_ms: a.delay_ms,
            });
        }
    }
    Ok(())
}

fn validate_playback(repeat: u16, interval_ms: u32) -> Result<(), MacroError> {
    if repeat == 0 {
        return Err(MacroError::ZeroRepeat);
    }
    if interval_ms > MAX_U24 {
        return Err(MacroError::IntervalTooLong(interval_ms));
    }
    Ok(())
}

/// Checks the limits `data_packets` and `mode_set` enforce: 1..=255 actions,
/// every delay below 2^24, `repeat` at least 1 and `interval_ms` below 2^24.
pub fn validate(actions: &[MacroAction], repeat: u16, interval_ms: u32) -> Result<(), MacroError> {
    validate_actions(actions)?;
    validate_playback(repeat, interval_ms)
}

/// MACRO data packets: at most 9 actions each, addresses from 256 advancing by
/// the number of actions already sent.
///
/// Packet: `[01, addr u16, count, (code u16, word u32)*count]` with
/// `word = (flag << 24) | delay_ms`, flag 1 = down and 8 = up, all little-endian.
pub fn data_packets(actions: &[MacroAction]) -> Result<Vec<Request>, MacroError> {
    validate_actions(actions)?;
    let mut addr = BASE_ADDR;
    Ok(actions
        .chunks(ACTIONS_PER_PACKET)
        .map(|chunk| {
            let mut p = Vec::with_capacity(4 + chunk.len() * 6);
            p.push(0x01);
            p.extend_from_slice(&addr.to_le_bytes());
            p.push(chunk.len() as u8);
            for a in chunk {
                let flag: u32 = if a.down { 1 } else { 8 };
                let word = (flag << 24) | (a.delay_ms & MAX_U24);
                p.extend_from_slice(&a.code.to_le_bytes());
                p.extend_from_slice(&word.to_le_bytes());
            }
            addr += chunk.len() as u16;
            Request {
                cmd: Cmd::Macro,
                payload: p,
            }
        })
        .collect())
}

/// MACRO_MODE SET `[01, key, id u16 = 0, len, mode, repeat u16, interval u24]`.
pub fn mode_set(
    key: u8,
    len: u8,
    mode: PlayMode,
    repeat: u16,
    interval_ms: u32,
) -> Result<Request, MacroError> {
    validate_playback(repeat, interval_ms)?;
    let mut p = vec![0x01, key, 0x00, 0x00, len, mode as u8];
    p.extend_from_slice(&repeat.to_le_bytes());
    p.extend_from_slice(&interval_ms.to_le_bytes()[..3]);
    Ok(Request {
        cmd: Cmd::MacroMode,
        payload: p,
    })
}

/// MACRO_MODE GET: flag 00 and the key, zero-padded to the SET length.
pub fn mode_get(key: u8) -> Request {
    let mut p = vec![0u8; 11];
    p[1] = key;
    Request {
        cmd: Cmd::MacroMode,
        payload: p,
    }
}

/// `(key, binding)` from a MACRO_MODE reply (R[1] key, R[2..3] id, R[4] len,
/// R[5] mode, R[6..7] repeat, R[8..10] interval).
pub fn parse_mode(r: &Response) -> Result<(u8, MacroBinding), MacroError> {
    if r.cmd != Cmd::MacroMode as u8 {
        return Err(MacroError::WrongReply);
    }
    let b = r.body();
    let mode = PlayMode::from_u8(b[5]).ok_or(MacroError::UnknownPlayMode(b[5]))?;
    Ok((
        b[1],
        MacroBinding {
            len: b[4],
            mode,
            repeat: u16::from_le_bytes([b[6], b[7]]),
            interval_ms: u32::from_le_bytes([b[8], b[9], b[10], 0]),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::REPORT_LEN;

    fn act(code: u16, down: bool, delay_ms: u32) -> MacroAction {
        MacroAction {
            code,
            down,
            delay_ms,
        }
    }

    fn frame(head: &[u8]) -> [u8; REPORT_LEN] {
        let mut out = [0u8; REPORT_LEN];
        out[..head.len()].copy_from_slice(head);
        out
    }

    fn reply(body: &[u8]) -> Response {
        let mut raw = [0u8; REPORT_LEN];
        raw[0] = 0x5C;
        raw[2] = 0xA1;
        raw[4..4 + body.len()].copy_from_slice(body);
        Response::decode(&raw).unwrap()
    }

    // Payload: 01, addr 0x0100 (256), count 1, code 0x0004, word 00 00 00 | flag 01.
    // Checksum: 0x91 + LEN 0x0A + 0x20 + last payload byte 0x01 = 0xBC.
    #[test]
    fn one_action_golden_bytes() {
        let p = data_packets(&[act(0x04, true, 0)]).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(
            p[0].encode().unwrap(),
            frame(&[
                0x5C, 0x0A, 0x20, 0xBC, 0x01, 0x00, 0x01, 0x01, 0x04, 0x00, 0x00, 0x00, 0x00, 0x01
            ])
        );
    }

    fn ten() -> Vec<MacroAction> {
        // Code 4 + i, down on even i, delay 100 * i.
        (0..10u16)
            .map(|i| act(0x04 + i, i % 2 == 0, 100 * u32::from(i)))
            .collect()
    }

    // Packet 1: addr 256, count 9; checksum 0x91 + LEN 0x3A + 0x20 + 0x01 = 0xEC.
    // Packet 2: addr 265 (0x0109), count 1; checksum 0x91 + 0x0A + 0x20 + 0x08 = 0xC3.
    #[test]
    fn ten_actions_split_into_two_packets_with_advancing_addresses() {
        let p = data_packets(&ten()).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[0].encode().unwrap(),
            frame(&[
                0x5C, 0x3A, 0x20, 0xEC, // header, LEN 58
                0x01, 0x00, 0x01, 0x09, // set, addr 256, count 9
                0x04, 0x00, 0x00, 0x00, 0x00, 0x01, // 0: code 4 down, 0 ms
                0x05, 0x00, 0x64, 0x00, 0x00, 0x08, // 1: code 5 up, 100 ms
                0x06, 0x00, 0xC8, 0x00, 0x00, 0x01, // 2: 200 ms
                0x07, 0x00, 0x2C, 0x01, 0x00, 0x08, // 3: 300 ms
                0x08, 0x00, 0x90, 0x01, 0x00, 0x01, // 4: 400 ms
                0x09, 0x00, 0xF4, 0x01, 0x00, 0x08, // 5: 500 ms
                0x0A, 0x00, 0x58, 0x02, 0x00, 0x01, // 6: 600 ms
                0x0B, 0x00, 0xBC, 0x02, 0x00, 0x08, // 7: 700 ms
                0x0C, 0x00, 0x20, 0x03, 0x00, 0x01, // 8: 800 ms
            ])
        );
        assert_eq!(
            p[1].encode().unwrap(),
            frame(&[
                0x5C, 0x0A, 0x20, 0xC3, // header, LEN 10
                0x01, 0x09, 0x01, 0x01, // set, addr 265, count 1
                0x0D, 0x00, 0x84, 0x03, 0x00, 0x08, // 9: code 13 up, 900 ms
            ])
        );
    }

    #[test]
    fn exactly_nine_actions_is_one_packet_and_eighteen_is_two() {
        let nine: Vec<_> = (0..9).map(|_| act(4, true, 0)).collect();
        assert_eq!(data_packets(&nine).unwrap().len(), 1);
        let eighteen: Vec<_> = (0..18).map(|_| act(4, true, 0)).collect();
        let p = data_packets(&eighteen).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(&p[1].payload[..4], &[0x01, 0x09, 0x01, 0x09]);
    }

    #[test]
    fn max_actions_are_accepted() {
        let all: Vec<_> = (0..255).map(|_| act(4, true, 0)).collect();
        let p = data_packets(&all).unwrap();
        assert_eq!(p.len(), 29); // 28 * 9 + 3
        assert_eq!(p[28].payload[3], 3);
        // 256 + 28 * 9 = 508 = 0x01FC
        assert_eq!(&p[28].payload[1..3], &[0xFC, 0x01]);
    }

    #[test]
    fn delay_uses_24_bits_and_flag_goes_in_the_top_byte() {
        let p = data_packets(&[act(0x0004, false, 0x00AB_CDEF)]).unwrap();
        assert_eq!(&p[0].payload[4..], &[0x04, 0x00, 0xEF, 0xCD, 0xAB, 0x08]);
    }

    #[test]
    fn validation_errors() {
        assert_eq!(data_packets(&[]), Err(MacroError::Empty));
        let too_many: Vec<_> = (0..256).map(|_| act(4, true, 0)).collect();
        assert_eq!(
            data_packets(&too_many),
            Err(MacroError::TooManyActions(256))
        );
        assert_eq!(
            data_packets(&[act(4, true, 0), act(4, false, 1 << 24)]),
            Err(MacroError::DelayTooLong {
                index: 1,
                delay_ms: 1 << 24
            })
        );
        assert!(data_packets(&[act(4, true, (1 << 24) - 1)]).is_ok());
        assert_eq!(
            validate(&[act(4, true, 0)], 0, 100),
            Err(MacroError::ZeroRepeat)
        );
        assert_eq!(
            validate(&[act(4, true, 0)], 1, 1 << 24),
            Err(MacroError::IntervalTooLong(1 << 24))
        );
        assert_eq!(validate(&[act(4, true, 0)], 1, (1 << 24) - 1), Ok(()));
    }

    // Payload: 01, key 0A, id 0000, len 3, mode 2, repeat 0001, interval 000064.
    // Checksum: 0x91 + LEN 0x0B + 0x21 + last payload byte 0x00 = 0xBD.
    #[test]
    fn mode_set_golden_bytes() {
        let r = mode_set(0x0A, 3, PlayMode::WhileHeld, 1, 100).unwrap();
        assert_eq!(
            r.encode().unwrap(),
            frame(&[
                0x5C, 0x0B, 0x21, 0xBD, 0x01, 0x0A, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x64, 0x00,
                0x00
            ])
        );
    }

    #[test]
    fn mode_set_encodes_repeat_and_interval_little_endian() {
        let r = mode_set(0x0A, 255, PlayMode::WhileHeldFinish, 0x0102, 0x0003_0201).unwrap();
        assert_eq!(
            r.payload,
            [0x01, 0x0A, 0x00, 0x00, 0xFF, 0x03, 0x02, 0x01, 0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn mode_set_validates() {
        assert_eq!(
            mode_set(1, 1, PlayMode::Once, 0, 0),
            Err(MacroError::ZeroRepeat)
        );
        assert_eq!(
            mode_set(1, 1, PlayMode::Once, 1, 1 << 24),
            Err(MacroError::IntervalTooLong(1 << 24))
        );
    }

    // Checksum: 0x91 + LEN 11 + 0x21 + 0 = 0xBD.
    #[test]
    fn mode_get_is_zero_padded_to_the_set_length() {
        assert_eq!(
            mode_get(0x0A).encode().unwrap(),
            frame(&[0x5C, 0x0B, 0x21, 0xBD, 0x00, 0x0A])
        );
    }

    #[test]
    fn parse_mode_round_trips_the_set_payload() {
        let set = mode_set(0x0A, 7, PlayMode::Toggle, 300, 0x01_0000).unwrap();
        let (key, b) = parse_mode(&reply(&set.payload)).unwrap();
        assert_eq!(key, 0x0A);
        assert_eq!(
            b,
            MacroBinding {
                len: 7,
                mode: PlayMode::Toggle,
                repeat: 300,
                interval_ms: 0x01_0000
            }
        );
    }

    #[test]
    fn parse_mode_rejects_unknown_mode_and_other_commands() {
        let mut body = mode_set(0x0A, 1, PlayMode::Once, 1, 0).unwrap().payload;
        body[5] = 9;
        assert_eq!(
            parse_mode(&reply(&body)),
            Err(MacroError::UnknownPlayMode(9))
        );
        let mut raw = [0u8; REPORT_LEN];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        assert_eq!(
            parse_mode(&Response::decode(&raw).unwrap()),
            Err(MacroError::WrongReply)
        );
    }
}

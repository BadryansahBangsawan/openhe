//! CMD 0x00 "order" sub-commands.

use crate::frame::{Cmd, Request, Response};
use crate::model::{PollingRate, SystemMode};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    StartCalibration = 0x0C,
    CloseCalibration = 0x0D,
    FactoryReset = 0x11,
    QueryWinMode = 0x21,
    PrecisionStroke = 0x25,
    KeyboardName = 0x26,
    SetWin = 0x30,
    SetMac = 0x31,
    RgbNack = 0x47,
    Polling = 0x50,
    Profile = 0x70,
}

/// `[type, FF, FF]`
pub fn query(t: OrderType) -> Request {
    Request {
        cmd: Cmd::Order,
        payload: vec![t as u8, 0xFF, 0xFF],
    }
}

/// `[type, arg, FF, FF]`
pub fn set(t: OrderType, arg: u8) -> Request {
    Request {
        cmd: Cmd::Order,
        payload: vec![t as u8, arg, 0xFF, 0xFF],
    }
}

fn value(r: &Response) -> Option<u8> {
    if r.cmd != Cmd::Order as u8 {
        return None;
    }
    r.body().get(2).copied()
}

/// R[2]: 1 = Windows, 0 = Mac, anything else (255) = unknown.
pub fn parse_system_mode(r: &Response) -> Option<SystemMode> {
    match value(r)? {
        1 => Some(SystemMode::Windows),
        0 => Some(SystemMode::Mac),
        _ => None,
    }
}

/// R[2] when in 0..=3.
pub fn parse_profile(r: &Response) -> Option<u8> {
    value(r).filter(|&v| v <= 3)
}

pub fn parse_polling(r: &Response) -> Option<PollingRate> {
    PollingRate::from_index(value(r)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(cmd_byte: u8, body: &[u8]) -> Response {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[1] = 2;
        raw[2] = cmd_byte;
        raw[4..4 + body.len()].copy_from_slice(body);
        Response::decode(&raw).unwrap()
    }

    #[test]
    fn query_profile_bytes() {
        let out = query(OrderType::Profile).encode().unwrap();
        assert_eq!(&out[..7], &[0x5C, 0x03, 0x00, 0x93, 0x70, 0xFF, 0xFF]);
    }

    #[test]
    fn set_polling_bytes() {
        let out = set(OrderType::Polling, 3).encode().unwrap();
        assert_eq!(out[1], 4);
        assert_eq!(&out[4..8], &[0x50, 0x03, 0xFF, 0xFF]);
    }

    #[test]
    fn parse_mode() {
        assert_eq!(
            parse_system_mode(&resp(0x80, &[0, 0x21, 1])),
            Some(SystemMode::Windows)
        );
        assert_eq!(
            parse_system_mode(&resp(0x80, &[0, 0x21, 0])),
            Some(SystemMode::Mac)
        );
        assert_eq!(parse_system_mode(&resp(0x80, &[0, 0x21, 255])), None);
        assert_eq!(parse_system_mode(&resp(0x81, &[0, 0x21, 1])), None);
    }

    #[test]
    fn parse_profile_and_polling() {
        assert_eq!(parse_profile(&resp(0x80, &[0, 0x70, 3])), Some(3));
        assert_eq!(parse_profile(&resp(0x80, &[0, 0x70, 4])), None);
        assert_eq!(
            parse_polling(&resp(0x80, &[0, 0x50, 3])),
            Some(PollingRate::Hz1000)
        );
        assert_eq!(parse_polling(&resp(0x80, &[0, 0x50, 9])), None);
        assert_eq!(parse_polling(&resp(0x82, &[0, 0x50, 3])), None);
    }
}

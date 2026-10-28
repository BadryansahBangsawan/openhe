//! CMD 0x01 SYNC: device identification.

use crate::frame::{Cmd, Request, Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub board_id: u32,
    pub in_bootloader: bool,
    pub serial: String,
    pub firmware: String,
    pub build_date: String,
}

pub fn request() -> Request {
    Request {
        cmd: Cmd::Sync,
        payload: vec![1, 2, 3, 4, 0xFF, 0xFF],
    }
}

/// Bytes up to the first NUL, decoded lossily, whitespace-trimmed.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

pub fn parse(r: &Response) -> DeviceInfo {
    let b = r.body();
    DeviceInfo {
        board_id: u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
        in_bootloader: b[7] != 0,
        serial: text(&b[9..25]),
        firmware: text(&b[26..36]),
        build_date: text(&b[43..54]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Response;

    fn put(raw: &mut [u8; 64], off: usize, s: &[u8]) {
        raw[4 + off..4 + off + s.len()].copy_from_slice(s);
    }

    #[test]
    fn parse_sync() {
        let mut raw = [0u8; 64];
        raw[2] = 0x81;
        put(&mut raw, 1, &0x1901_0001u32.to_le_bytes());
        put(&mut raw, 9, b"A68M240193000000");
        put(&mut raw, 26, b"1.0.7");
        put(&mut raw, 43, b"2024-05-01 ");
        let info = parse(&Response::decode(&raw).unwrap());
        assert_eq!(info.board_id, 0x1901_0001);
        assert!(!info.in_bootloader);
        assert_eq!(info.serial, "A68M240193000000");
        assert_eq!(info.firmware, "1.0.7");
        assert_eq!(info.build_date, "2024-05-01");
        raw[4 + 7] = 1;
        assert!(parse(&Response::decode(&raw).unwrap()).in_bootloader);
    }

    #[test]
    fn request_payload() {
        let r = request();
        assert_eq!(r.cmd, crate::frame::Cmd::Sync);
        assert_eq!(r.payload, vec![1, 2, 3, 4, 0xFF, 0xFF]);
    }
}

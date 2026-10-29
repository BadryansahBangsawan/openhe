//! CMD 0x01 SYNC: device identification.

use crate::frame::{Cmd, Request, Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub board_id: u32,
    pub in_bootloader: bool,
    pub serial: String,
    pub firmware: String,
    /// On FW App V1.1.3 real hardware this is garbled (e.g. "Nov 14 50258");
    /// treat it as informational only.
    pub build_date: String,
}

pub fn request() -> Request {
    Request {
        cmd: Cmd::Sync,
        payload: vec![1, 2, 3, 4, 0xFF, 0xFF],
    }
}

/// Longest text slot.
const SLOT_MAX: usize = 16;

/// Bytes up to the first NUL (or other ASCII control byte), decoded lossily,
/// whitespace-trimmed.
fn text(b: &[u8]) -> String {
    let end = b
        .iter()
        .position(|c| c.is_ascii_control())
        .unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// The text slot whose length byte is at `b[at]` and whose bytes follow it:
/// `min(length byte, 16, rest of the body)` bytes.
fn slot(b: &[u8], at: usize) -> String {
    let start = at + 1;
    let n = usize::from(b[at])
        .min(SLOT_MAX)
        .min(b.len().saturating_sub(start));
    text(&b[start..start + n])
}

/// Body layout: R[1..5] board id, R[7] run mode, then length-prefixed text
/// slots: R[8] serial, R[25] firmware, R[42] build date.
pub fn parse(r: &Response) -> DeviceInfo {
    let b = r.body();
    DeviceInfo {
        board_id: u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
        in_bootloader: b[7] != 0,
        serial: slot(b, 8),
        firmware: slot(b, 25),
        build_date: slot(b, 42),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Response;

    fn put(raw: &mut [u8; 64], off: usize, s: &[u8]) {
        raw[4 + off..4 + off + s.len()].copy_from_slice(s);
    }

    fn sync_reply() -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0x81;
        raw
    }

    #[test]
    fn parse_sync() {
        let mut raw = sync_reply();
        put(&mut raw, 1, &0x1901_0001u32.to_le_bytes());
        put(&mut raw, 8, &[16]);
        put(&mut raw, 9, b"A68M240193000000");
        put(&mut raw, 25, &[16]);
        put(&mut raw, 26, b"1.0.7");
        put(&mut raw, 42, &[11]);
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
    fn firmware_slot_is_not_cut_at_ten_bytes() {
        let mut raw = sync_reply();
        put(&mut raw, 25, &[16]);
        put(&mut raw, 26, b"App V1.1.13\0\0\0\0\0");
        assert_eq!(
            parse(&Response::decode(&raw).unwrap()).firmware,
            "App V1.1.13"
        );
    }

    #[test]
    fn slot_length_comes_from_its_prefix() {
        let mut raw = sync_reply();
        put(&mut raw, 8, &[4]);
        put(&mut raw, 9, b"ABCDEFGH");
        put(&mut raw, 25, &[7]);
        put(&mut raw, 26, b"App V1.1.13");
        put(&mut raw, 42, &[0]);
        put(&mut raw, 43, b"Nov 14 2025");
        let info = parse(&Response::decode(&raw).unwrap());
        assert_eq!(info.serial, "ABCD");
        assert_eq!(info.firmware, "App V1.");
        assert_eq!(info.build_date, "");
    }

    #[test]
    fn slot_length_is_capped_at_16_bytes() {
        let mut raw = sync_reply();
        put(&mut raw, 8, &[0xFF]);
        put(&mut raw, 9, b"0123456789ABCDEFxyz");
        put(&mut raw, 42, &[0xFF]);
        put(&mut raw, 43, b"0123456789ABCDEF"); // runs to R[58]; R[59] is the last body byte
        raw[63] = b'!';
        let info = parse(&Response::decode(&raw).unwrap());
        assert_eq!(info.serial, "0123456789ABCDEF");
        assert_eq!(info.build_date, "0123456789ABCDEF");
    }

    #[test]
    fn request_payload() {
        let r = request();
        assert_eq!(r.cmd, crate::frame::Cmd::Sync);
        assert_eq!(r.payload, vec![1, 2, 3, 4, 0xFF, 0xFF]);
    }
}

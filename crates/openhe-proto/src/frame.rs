//! Request framing: `5C LEN CMD CHK` followed by the payload, in a 64-byte report.

use thiserror::Error;

pub const REPORT_LEN: usize = 64;
pub const MAGIC: u8 = 0x5C;
pub const MAX_PAYLOAD: usize = 60;

/// Commands the host may send. Bootloader/firmware commands 0x08-0x0E are
/// deliberately absent so they cannot be constructed.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Order = 0x00,
    Sync = 0x01,
    Rm6x21 = 0x12,
    Prgb = 0x18,
    Macro = 0x20,
    MacroMode = 0x21,
    Key = 0x23,
    Mt = 0x24,
    Tgl = 0x25,
    Dks = 0x26,
    Mpt = 0x27,
    End = 0x28,
    Krgb = 0x2A,
    DefKey = 0x2B,
    Socd = 0x2C,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    #[error("payload too long: {0} bytes (max {MAX_PAYLOAD})")]
    PayloadTooLong(usize),
    #[error("report shorter than expected")]
    ShortReport,
    #[error("device reported failure")]
    DeviceFail,
}

/// `(0x35 + 0x5C + LEN + CMD + PAYLOAD[LEN-1]) & 0xFF`, last term only when 1 <= LEN <= 252.
pub fn checksum(len: u8, cmd: u8, payload: &[u8]) -> u8 {
    let last = if (1..=252).contains(&len) {
        payload.get(len as usize - 1).copied().unwrap_or(0)
    } else {
        0
    };
    0x35u8
        .wrapping_add(MAGIC)
        .wrapping_add(len)
        .wrapping_add(cmd)
        .wrapping_add(last)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub cmd: Cmd,
    pub payload: Vec<u8>,
}

impl Request {
    pub fn encode(&self) -> Result<[u8; REPORT_LEN], FrameError> {
        let n = self.payload.len();
        if n > MAX_PAYLOAD {
            return Err(FrameError::PayloadTooLong(n));
        }
        let mut out = [0u8; REPORT_LEN];
        out[0] = MAGIC;
        out[1] = n as u8;
        out[2] = self.cmd as u8;
        out[3] = checksum(n as u8, self.cmd as u8, &self.payload);
        out[4..4 + n].copy_from_slice(&self.payload);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(req: &Request, head: &[u8]) {
        let out = req.encode().unwrap();
        assert_eq!(&out[..head.len()], head);
        assert!(out[head.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn encode_sync() {
        let r = Request {
            cmd: Cmd::Sync,
            payload: vec![1, 2, 3, 4, 0xFF, 0xFF],
        };
        check(&r, &[0x5C, 0x06, 0x01, 0x97, 1, 2, 3, 4, 0xFF, 0xFF]);
    }

    #[test]
    fn encode_factory_reset() {
        let r = Request {
            cmd: Cmd::Order,
            payload: vec![0x11, 0xFF, 0xFF],
        };
        check(&r, &[0x5C, 0x03, 0x00, 0x93, 0x11, 0xFF, 0xFF]);
    }

    #[test]
    fn encode_rm6x21() {
        let r = Request {
            cmd: Cmd::Rm6x21,
            payload: vec![2, 1, 0xFF, 0xFF],
        };
        check(&r, &[0x5C, 0x04, 0x12, 0xA6, 2, 1, 0xFF, 0xFF]);
    }

    #[test]
    fn payload_too_long() {
        let r = Request {
            cmd: Cmd::Key,
            payload: vec![0; 61],
        };
        assert_eq!(r.encode(), Err(FrameError::PayloadTooLong(61)));
    }

    #[test]
    fn checksum_empty_payload() {
        assert_eq!(checksum(0, 0x01, &[]), 0x92);
    }
}

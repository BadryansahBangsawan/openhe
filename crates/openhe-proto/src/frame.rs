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
    #[error("report is not a protocol reply")]
    NotAReply,
}

/// Expects `payload.len() >= len`; a missing last byte is treated as 0.
///
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

/// A keyboard-to-host reply: `[5C, LEN, CMD|0x80, CHK, R...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Command byte with the 0x80 reply bit cleared.
    pub cmd: u8,
    pub len: u8,
    pub raw: [u8; REPORT_LEN],
}

impl Response {
    /// A reply starts with the 0x5C magic (verified on hardware) and has the
    /// 0x80 bit set in its command byte; anything else is `NotAReply`.
    /// Input longer than a report is truncated to its first 64 bytes.
    pub fn decode(report: &[u8]) -> Result<Response, FrameError> {
        if report.len() < REPORT_LEN {
            return Err(FrameError::ShortReport);
        }
        let mut raw = [0u8; REPORT_LEN];
        raw.copy_from_slice(&report[..REPORT_LEN]);
        if raw[0] != MAGIC || raw[2] & 0x80 == 0 {
            return Err(FrameError::NotAReply);
        }
        if raw[2] == 0xFF {
            return Err(FrameError::DeviceFail);
        }
        Ok(Response {
            cmd: raw[2] & 0x7F,
            len: raw[1],
            raw,
        })
    }

    /// The reply body `R` (`raw[4..]`).
    pub fn body(&self) -> &[u8] {
        &self.raw[4..]
    }

    /// Diagnostic only; replies are never rejected on a checksum mismatch.
    /// Hardware shows the checksum covers the reply's own CMD byte
    /// (`raw[2]`, i.e. with the 0x80 bit set).
    pub fn checksum_ok(&self) -> bool {
        let body = self.body();
        let n = body.len().min(MAX_PAYLOAD);
        checksum(self.len, self.raw[2], &body[..n]) == self.raw[3]
    }
}

impl Request {
    /// Whether `resp` answers this request.
    ///
    /// The command byte must match, and for commands whose reply echoes the
    /// request so must the echoed fields:
    /// - Order: R[1] is the TYPE (`payload[0]`).
    /// - Key: R[1..3] is the key and layout (`payload[1..3]`, the same place in
    ///   GET and SET). A multi-entry SET is matched on its first entry only.
    /// - DefKey: R[1] is the first requested row (`payload[1]`).
    /// - Rm6x21: R[1] is the SUB (`payload[0]`); only the first of the reply's
    ///   3 reports carries a header, the rest are raw continuations. The
    ///   reply does not echo the PART, so a late reply to PART 1 that
    ///   arrives while PART 2 is awaited (after a retry) matches too. One
    ///   already waiting is drained before the write; for the rest the
    ///   consumers skip frames that do not fit the matrix
    ///   (`TravelFrame::fits`).
    /// - Krgb: R[1] is the key (`payload[1]`) for a SET or GET (PROTOCOL
    ///   3.8: the SET reply echoes `[st, key, R, G, B]`); the commit
    ///   (`[01, FE, ...]`) matches on the command only.
    /// - MacroMode GET: R[1] is the key (PROTOCOL 3.7; read back on
    ///   hardware, 7.x finding 24). A SET matches on the command only: its
    ///   reply layout is not documented.
    ///
    /// MPT, TGL and END replies are not matched here: their R[1] = key is
    /// inferred from the vendor parsers, not documented, so the device
    /// layer checks it after the call and reports a mismatch as `Io`.
    /// SOCD replies carry the pair (`pos1`, `pos2`), not the requested key,
    /// and PRGB replies carry no key at all.
    ///
    /// A payload too short to carry those fields never matches.
    pub fn expects(&self, resp: &Response) -> bool {
        if resp.cmd != self.cmd as u8 {
            return false;
        }
        let body = resp.body();
        let p = &self.payload;
        match self.cmd {
            Cmd::Order => p.first().is_some_and(|&t| body[1] == t),
            Cmd::Key => p.get(1..3).is_some_and(|kl| body[1..3] == *kl),
            Cmd::DefKey => p.get(1).is_some_and(|&row| body[1] == row),
            Cmd::Rm6x21 => p.first().is_some_and(|&sub| body[1] == sub),
            // The KRGB commit [01, FE, ...] is a marker, not a key: its
            // reply layout is unknown, so it matches on the command only.
            Cmd::Krgb => match p.get(..2) {
                Some(&[0x01, 0xFE]) => true,
                Some(&[_, key]) => body[1] == key,
                _ => false,
            },
            Cmd::MacroMode => match p.get(..2) {
                Some(&[0x00, key]) => body[1] == key,
                Some(_) => true,
                None => false,
            },
            _ => true,
        }
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

    #[test]
    fn encode_max_payload_last_byte() {
        let mut payload = vec![0u8; 60];
        payload[59] = 0xAB;
        let r = Request {
            cmd: Cmd::Key,
            payload,
        };
        let out = r.encode().unwrap();
        assert_eq!(out[1], 60);
        assert_eq!(out[63], 0xAB);
        assert_eq!(out[3], checksum(60, 0x23, &r.payload));
    }

    #[test]
    fn encode_empty_payload() {
        let r = Request {
            cmd: Cmd::Sync,
            payload: vec![],
        };
        check(&r, &[0x5C, 0x00, 0x01, 0x92]);
    }

    fn reply(cmd_byte: u8, body: &[u8]) -> Vec<u8> {
        let mut raw = vec![0u8; REPORT_LEN];
        raw[0] = MAGIC;
        raw[1] = 2;
        raw[2] = cmd_byte;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    /// KRGB (SET and GET) and MACRO_MODE GET replies carry the key at
    /// R[1] (PROTOCOL 3.7, 3.8; MACRO_MODE GET read back on hardware,
    /// finding 24): a reply for another key is not the answer. The KRGB
    /// commit and the MACRO_MODE SET keep matching on the command only.
    #[test]
    fn krgb_and_macro_mode_get_replies_match_on_the_key() {
        let krgb = |flag: u8, key: u8| Request {
            cmd: Cmd::Krgb,
            payload: vec![flag, key, 1, 2, 3],
        };
        let resp = |cmd: u8, key: u8| Response::decode(&reply(cmd, &[0, key, 1, 2, 3])).unwrap();
        for flag in [0, 1] {
            assert!(krgb(flag, 0x0A).expects(&resp(0xAA, 0x0A)));
            assert!(!krgb(flag, 0x0A).expects(&resp(0xAA, 0x0B)));
        }
        let commit = krgb(1, 0xFE);
        assert!(commit.expects(&resp(0xAA, 0x00)));
        let get = Request {
            cmd: Cmd::MacroMode,
            payload: {
                let mut p = vec![0u8; 11];
                p[1] = 0x0A;
                p
            },
        };
        assert!(get.expects(&resp(0xA1, 0x0A)));
        assert!(!get.expects(&resp(0xA1, 0x0B)));
        let set = Request {
            cmd: Cmd::MacroMode,
            payload: vec![1, 0x0A, 0, 0, 2, 0, 1, 0, 100, 0, 0],
        };
        assert!(set.expects(&resp(0xA1, 0x0B)));
        let short = Request {
            cmd: Cmd::Krgb,
            payload: vec![0],
        };
        assert!(!short.expects(&resp(0xAA, 0x00)));
    }

    #[test]
    fn decode_strips_high_bit() {
        let r = Response::decode(&reply(0xA3, &[])).unwrap();
        assert_eq!(r.cmd, 0x23);
        assert_eq!(r.len, 2);
        assert_eq!(r.body().len(), 60);
    }

    #[test]
    fn decode_fail() {
        assert_eq!(
            Response::decode(&reply(0xFF, &[])),
            Err(FrameError::DeviceFail)
        );
    }

    #[test]
    fn decode_short() {
        assert_eq!(Response::decode(&[0u8; 10]), Err(FrameError::ShortReport));
    }

    #[test]
    fn decode_truncates_long_input() {
        let mut raw = reply(0x81, &[]);
        raw.extend_from_slice(&[9; 8]);
        let r = Response::decode(&raw).unwrap();
        assert_eq!(r.raw.len(), REPORT_LEN);
        assert_eq!(r.cmd, 1);
    }

    #[test]
    fn checksum_ok_diagnostic() {
        let mut raw = reply(0x81, &[7, 8]);
        raw[3] = checksum(2, 0x81, &[7, 8]); // real replies checksum their own CMD|0x80
        assert!(Response::decode(&raw).unwrap().checksum_ok());
        raw[3] ^= 1;
        assert!(!Response::decode(&raw).unwrap().checksum_ok());
    }

    #[test]
    fn order_reply_matches_type() {
        let q = Request {
            cmd: Cmd::Order,
            payload: vec![0x70, 0xFF, 0xFF],
        };
        let ok = Response::decode(&reply(0x80, &[1, 0x70, 2])).unwrap();
        assert!(q.expects(&ok));
        let bad = Response::decode(&reply(0x80, &[1, 0x21, 2])).unwrap();
        assert!(!q.expects(&bad));
    }

    #[test]
    fn order_empty_payload_no_panic() {
        let q = Request {
            cmd: Cmd::Order,
            payload: vec![],
        };
        let resp = Response::decode(&reply(0x80, &[1, 0x70])).unwrap();
        assert!(!q.expects(&resp));
    }

    #[test]
    fn key_reply_matches() {
        let q = Request {
            cmd: Cmd::Key,
            payload: vec![1, 0x0A, 4, 0xDC, 0x05],
        };
        let echo = Response::decode(&reply(0xA3, &[0, 0x0A, 4, 0xDC, 0x05])).unwrap();
        assert!(q.expects(&echo));
        assert!(!q.expects(&Response::decode(&reply(0x80, &[0, 0x0A, 4])).unwrap()));
    }

    #[test]
    fn key_reply_must_carry_the_requested_key_and_layout() {
        for payload in [vec![0, 0x0A, 4, 0, 0], vec![1, 0x0A, 4, 0xDC, 0x05]] {
            let q = Request {
                cmd: Cmd::Key,
                payload,
            };
            let other_key = Response::decode(&reply(0xA3, &[0, 0x0B, 4, 0xDC, 0x05])).unwrap();
            let other_layout = Response::decode(&reply(0xA3, &[0, 0x0A, 20, 0xDC, 0x05])).unwrap();
            let same = Response::decode(&reply(0xA3, &[0, 0x0A, 4, 0xD0, 0x07])).unwrap();
            assert!(!q.expects(&other_key), "{:?}", q.payload);
            assert!(!q.expects(&other_layout), "{:?}", q.payload);
            assert!(q.expects(&same), "{:?}", q.payload);
        }
    }

    #[test]
    fn multi_entry_key_set_matches_on_its_first_entry() {
        let q = Request {
            cmd: Cmd::Key,
            payload: vec![1, 0x0A, 4, 0xDC, 0x05, 0x0B, 20, 0x64, 0x00],
        };
        let first = Response::decode(&reply(0xA3, &[0, 0x0A, 4, 0xDC, 0x05])).unwrap();
        let second = Response::decode(&reply(0xA3, &[0, 0x0B, 20, 0x64, 0x00])).unwrap();
        assert!(q.expects(&first));
        assert!(!q.expects(&second));
    }

    #[test]
    fn short_key_payload_never_matches() {
        let resp = Response::decode(&reply(0xA3, &[0, 0x0A, 4])).unwrap();
        for payload in [vec![], vec![0], vec![0, 0x0A]] {
            let q = Request {
                cmd: Cmd::Key,
                payload,
            };
            assert!(!q.expects(&resp));
        }
    }

    #[test]
    fn defkey_reply_must_carry_the_requested_first_row() {
        let q = Request {
            cmd: Cmd::DefKey,
            payload: vec![0, 2, 3],
        };
        assert!(q.expects(&Response::decode(&reply(0xAB, &[0, 2])).unwrap()));
        assert!(!q.expects(&Response::decode(&reply(0xAB, &[0, 0])).unwrap()));
        assert!(!q.expects(&Response::decode(&reply(0xAB, &[0, 4])).unwrap()));
        let empty = Request {
            cmd: Cmd::DefKey,
            payload: vec![0],
        };
        assert!(!empty.expects(&Response::decode(&reply(0xAB, &[0, 0])).unwrap()));
    }

    #[test]
    fn rm6x21_reply_must_echo_the_requested_sub() {
        let q = Request {
            cmd: Cmd::Rm6x21,
            payload: vec![2, 1, 0xFF, 0xFF],
        };
        // body = [status, SUB, ...]
        assert!(q.expects(&Response::decode(&reply(0x92, &[0, 2])).unwrap()));
        assert!(!q.expects(&Response::decode(&reply(0x92, &[0, 6])).unwrap()));
        assert!(!q.expects(&Response::decode(&reply(0x92, &[0, 3])).unwrap()));
        assert!(!q.expects(&Response::decode(&reply(0x80, &[0, 2])).unwrap()));
        let empty = Request {
            cmd: Cmd::Rm6x21,
            payload: vec![],
        };
        assert!(!empty.expects(&Response::decode(&reply(0x92, &[0, 2])).unwrap()));
    }

    #[test]
    fn decode_rejects_reports_without_the_magic_byte() {
        let mut raw = reply(0xA3, &[0, 0x0A, 4]);
        raw[0] = 0x00;
        assert_eq!(Response::decode(&raw), Err(FrameError::NotAReply));
        raw[0] = 0x35;
        assert_eq!(Response::decode(&raw), Err(FrameError::NotAReply));
    }

    #[test]
    fn decode_rejects_reports_without_the_reply_bit() {
        for cmd in [0x00, 0x01, 0x23, 0x7F] {
            assert_eq!(
                Response::decode(&reply(cmd, &[])),
                Err(FrameError::NotAReply),
                "{cmd:#04X}"
            );
        }
    }
}

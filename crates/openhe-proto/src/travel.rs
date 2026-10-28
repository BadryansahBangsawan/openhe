//! CMD 0x12 RM6X21 live key travel (6x21 matrix travel readout).
//!
//! A request `[SUB, PART, FF, FF]` is answered by 3 input reports. Only the
//! first carries the `5C LEN CMD CHK` header; the other two are raw
//! continuation data. The three reports are concatenated (192 bytes) and the
//! first 4 bytes dropped, which leaves `[status, SUB, data...]`. The data is 3
//! matrix rows of 21 little-endian u16 each (126 bytes), row-major.
//!
//! PART 1 carries rows 0..=2 and PART 2 rows 3..=5 (PROTOCOL 3.4, marked [I]:
//! the framing and the row split are inferred from the AULA HUB sources and
//! not yet confirmed on hardware, so `parse` is strict about what it accepts).
//! SUB 2 is travel in micrometers; SUB 6 is calibration values (raw u16).

use crate::frame::{Cmd, FrameError, Request, MAGIC, REPORT_LEN};
use thiserror::Error;

pub const ROWS: usize = 6;
pub const COLS: usize = 21;
/// Reports a request is answered with.
pub const REPORTS: usize = 3;
/// SUB for live key travel.
pub const SUB_TRAVEL: u8 = 2;
/// SUB for calibration values.
pub const SUB_CALIBRATION: u8 = 6;

const REPLY_CMD: u8 = Cmd::Rm6x21 as u8 | 0x80;
/// Bytes stripped from the concatenated reports.
const HEADER: usize = 4;
const HALF_BYTES: usize = 3 * COLS * 2;

/// A full 6x21 readout, row-major by matrix position (the same (row, col) as
/// DEFKEY). Travel values are micrometers; calibration values are raw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TravelFrame {
    pub um: [[u16; COLS]; ROWS],
}

/// One reply: 3 matrix rows, either rows 0..=2 (PART 1) or 3..=5 (PART 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TravelHalf {
    /// The SUB echoed by the keyboard: `SUB_TRAVEL` or `SUB_CALIBRATION`.
    pub sub: u8,
    pub um: [[u16; COLS]; 3],
}

/// The two halves of a frame were read with different SUBs (e.g. travel and
/// calibration values mixed).
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("RM6X21 halves have different SUBs: {top} and {bottom}")]
pub struct SubMismatch {
    pub top: u8,
    pub bottom: u8,
}

impl TravelFrame {
    /// Rows 0..=2 from `top` (PART 1), rows 3..=5 from `bottom` (PART 2). The
    /// halves must carry the same SUB, else `SubMismatch`.
    pub fn from_halves(top: &TravelHalf, bottom: &TravelHalf) -> Result<TravelFrame, SubMismatch> {
        if top.sub != bottom.sub {
            return Err(SubMismatch {
                top: top.sub,
                bottom: bottom.sub,
            });
        }
        let mut um = [[0u16; COLS]; ROWS];
        um[..3].copy_from_slice(&top.um);
        um[3..].copy_from_slice(&bottom.um);
        Ok(TravelFrame { um })
    }
}

impl TravelFrame {
    /// Every matrix position without a key reads 0.
    ///
    /// RM6X21 replies carry their SUB but not their PART, so a late reply
    /// to one half that arrives while the other half is awaited would put
    /// its rows in the wrong place. Such a frame usually shows travel where
    /// the matrix has no key (row 0 and the gaps are empty), so consumers
    /// skip frames that do not fit rather than credit the wrong keys.
    pub fn fits(&self, matrix: &crate::defkey::Matrix) -> bool {
        self.um
            .iter()
            .zip(matrix.codes.iter())
            .all(|(vals, codes)| {
                vals.iter()
                    .zip(codes.iter())
                    .all(|(&v, &code)| code != 0 || v == 0)
            })
    }
}

/// `[sub, part, FF, FF]`, no GET/SET flag.
pub fn request(sub: u8, part: u8) -> Request {
    Request {
        cmd: Cmd::Rm6x21,
        payload: vec![sub, part, 0xFF, 0xFF],
    }
}

/// Parses the 3 reports answering a `SUB_TRAVEL` or `SUB_CALIBRATION` request.
///
/// Strict: exactly 3 reports, the first a 0x5C-magic reply with command byte
/// 0x92, and a SUB of 2 or 6 after the status byte. Anything else is an error
/// (`ShortReport` for the wrong report count, `NotAReply` for the rest).
/// Trailing bytes after the 126 data bytes are ignored. SUB 3 (status) is not
/// supported.
pub fn parse(reports: &[[u8; REPORT_LEN]]) -> Result<TravelHalf, FrameError> {
    if reports.len() != REPORTS {
        return Err(FrameError::ShortReport);
    }
    let head = &reports[0];
    if head[0] != MAGIC || head[2] != REPLY_CMD {
        return Err(FrameError::NotAReply);
    }
    let all: Vec<u8> = reports.iter().flatten().copied().collect();
    let body = &all[HEADER..];
    let sub = body[1];
    if sub != SUB_TRAVEL && sub != SUB_CALIBRATION {
        return Err(FrameError::NotAReply);
    }
    let data = &body[2..2 + HALF_BYTES];
    let mut um = [[0u16; COLS]; 3];
    for (i, v) in data.as_chunks::<2>().0.iter().enumerate() {
        um[i / COLS][i % COLS] = u16::from_le_bytes(*v);
    }
    Ok(TravelHalf { sub, um })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic reply: header report + 2 continuations, with
    /// `value(row, col)` laid out row-major from body offset 2.
    fn capture(sub: u8, value: impl Fn(usize, usize) -> u16) -> [[u8; 64]; 3] {
        let mut all = [0u8; 192];
        all[..4].copy_from_slice(&[0x5C, 0x3C, 0x92, 0x00]);
        all[5] = sub;
        for r in 0..3 {
            for c in 0..COLS {
                let at = 4 + 2 + 2 * (r * COLS + c);
                all[at..at + 2].copy_from_slice(&value(r, c).to_le_bytes());
            }
        }
        let mut out = [[0u8; 64]; 3];
        for (i, rep) in out.iter_mut().enumerate() {
            rep.copy_from_slice(&all[i * 64..(i + 1) * 64]);
        }
        out
    }

    #[test]
    fn a_frame_fits_when_keyless_positions_read_zero() {
        let mut m = crate::defkey::Matrix::default();
        m.codes[1][0] = 0x29;
        let mut f = TravelFrame {
            um: [[0; COLS]; ROWS],
        };
        f.um[1][0] = 3400;
        assert!(f.fits(&m));
        f.um[0][0] = 1200; // row 0 has no keys
        assert!(!f.fits(&m));
    }

    fn v(r: usize, c: usize) -> u16 {
        (r * 1000 + c * 7 + 300) as u16
    }

    #[test]
    fn request_bytes() {
        for (sub, part) in [(2u8, 1u8), (2, 2), (6, 1), (6, 2)] {
            let out = request(sub, part).encode().unwrap();
            // checksum covers only the last payload byte (0xFF): 0xA6
            assert_eq!(&out[..8], &[0x5C, 0x04, 0x12, 0xA6, sub, part, 0xFF, 0xFF]);
            assert!(out[8..].iter().all(|&b| b == 0));
        }
    }

    #[test]
    fn parses_rows_across_report_boundaries() {
        let reps = capture(2, v);
        let half = parse(&reps).unwrap();
        assert_eq!(half.sub, SUB_TRAVEL);
        for r in 0..3 {
            for c in 0..COLS {
                assert_eq!(half.um[r][c], v(r, c), "row {r} col {c}");
            }
        }
        // The data really does straddle the report boundaries.
        assert_ne!(reps[0][60..], [0u8; 4]);
        assert_ne!(reps[1], [0u8; 64]);
        assert_ne!(reps[2][..4], [0u8; 4]);
    }

    #[test]
    fn little_endian_micrometers() {
        let mut reps = capture(2, |_, _| 0);
        reps[0][6] = 0xD0;
        reps[0][7] = 0x07; // row 0 col 0 = 2000 µm
        assert_eq!(parse(&reps).unwrap().um[0][0], 2000);
    }

    #[test]
    fn calibration_sub_is_accepted() {
        assert_eq!(parse(&capture(6, v)).unwrap().sub, SUB_CALIBRATION);
    }

    #[test]
    fn strict_about_report_count() {
        let reps = capture(2, v);
        assert_eq!(parse(&reps[..2]), Err(FrameError::ShortReport));
        assert_eq!(parse(&[]), Err(FrameError::ShortReport));
        let four = [reps[0], reps[1], reps[2], [0u8; 64]];
        assert_eq!(parse(&four), Err(FrameError::ShortReport));
    }

    #[test]
    fn strict_about_header_and_sub() {
        let mut reps = capture(2, v);
        reps[0][0] = 0;
        assert_eq!(parse(&reps), Err(FrameError::NotAReply));
        let mut reps = capture(2, v);
        reps[0][2] = 0x80; // an order reply, not RM6X21
        assert_eq!(parse(&reps), Err(FrameError::NotAReply));
        for sub in [0u8, 1, 3, 4, 0xFF] {
            assert_eq!(parse(&capture(sub, v)), Err(FrameError::NotAReply));
        }
    }

    #[test]
    fn halves_make_a_frame() {
        let top = parse(&capture(2, v)).unwrap();
        let bottom = parse(&capture(2, |r, c| v(r + 3, c))).unwrap();
        let f = TravelFrame::from_halves(&top, &bottom).unwrap();
        assert_eq!(f.um[0][0], v(0, 0));
        assert_eq!(f.um[2][20], v(2, 20));
        assert_eq!(f.um[3][0], v(3, 0));
        assert_eq!(f.um[5][20], v(5, 20));
        let cal = parse(&capture(6, v)).unwrap();
        assert_eq!(
            TravelFrame::from_halves(&top, &cal),
            Err(SubMismatch { top: 2, bottom: 6 })
        );
    }
}

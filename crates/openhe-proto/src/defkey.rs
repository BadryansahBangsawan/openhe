//! CMD 0x2B DEFKEY: factory-default keycode matrix.

use crate::frame::{Cmd, FrameError, Request, Response};

pub const ROWS: usize = 6;
pub const COLS: usize = 21;

pub fn requests() -> [Request; 3] {
    [[0, 0, 1], [0, 2, 3], [0, 4, 5]].map(|p| Request {
        cmd: Cmd::DefKey,
        payload: p.to_vec(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Matrix {
    pub codes: [[u8; COLS]; ROWS],
}

impl Matrix {
    /// Merges the two rows carried by one DEFKEY reply.
    pub fn apply(&mut self, r: &Response) -> Result<(), FrameError> {
        if r.cmd != Cmd::DefKey as u8 {
            return Err(FrameError::ShortReport);
        }
        let b = r.body();
        let (ra, rb) = (b[1] as usize, b[23] as usize);
        if ra >= ROWS || rb >= ROWS {
            return Err(FrameError::ShortReport);
        }
        self.codes[ra].copy_from_slice(&b[2..23]);
        self.codes[rb].copy_from_slice(&b[24..45]);
        Ok(())
    }

    /// Non-zero keys as `(row, col, code)`, row-major.
    pub fn keys(&self) -> impl Iterator<Item = (u8, u8, u8)> + '_ {
        self.codes.iter().enumerate().flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .filter(|(_, &c)| c != 0)
                .map(move |(c, &code)| (r as u8, c as u8, code))
        })
    }

    pub fn len(&self) -> usize {
        self.keys().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameError, Response};

    fn reply(row_a: u8, a: &[u8], row_b: u8, b: &[u8]) -> Response {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xAB;
        raw[4 + 1] = row_a;
        raw[4 + 2..4 + 2 + a.len()].copy_from_slice(a);
        raw[4 + 23] = row_b;
        raw[4 + 24..4 + 24 + b.len()].copy_from_slice(b);
        Response::decode(&raw).unwrap()
    }

    fn codes(n: usize, first: u8) -> Vec<u8> {
        (0..n).map(|i| first + i as u8).collect()
    }

    #[test]
    fn matrix_from_three_replies() {
        let mut m = Matrix::default();
        m.apply(&reply(0, &codes(15, 0x29), 1, &codes(15, 0x40)))
            .unwrap();
        m.apply(&reply(2, &codes(14, 0x50), 3, &codes(14, 0x60)))
            .unwrap();
        m.apply(&reply(4, &codes(10, 0x70), 5, &[])).unwrap();
        assert_eq!(m.len(), 68);
        assert!(!m.is_empty());
        assert_eq!(m.keys().next(), Some((0, 0, 0x29)));
    }

    const REAL: [&[u8]; 6] = [
        &[],
        &[
            0x29, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A,
            0x49,
        ],
        &[
            0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31,
            0x4C,
        ],
        &[
            0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x00, 0x28,
            0x4B,
        ],
        &[
            0xE1, 0x00, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xE5, 0x52,
            0x4E,
        ],
        &[
            0xE0, 0xE3, 0xE2, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0xE6, 0x01, 0xE4, 0x50, 0x51,
            0x4F,
        ],
    ];

    #[test]
    fn real_win68_matrix() {
        let mut m = Matrix::default();
        for pair in 0..3u8 {
            let (a, b) = (pair as usize * 2, pair as usize * 2 + 1);
            m.apply(&reply(a as u8, REAL[a], b as u8, REAL[b])).unwrap();
        }
        assert_eq!(m.len(), 68);
        assert!(m.codes[0].iter().all(|&c| c == 0));
        assert_eq!(m.keys().next(), Some((1, 0, 0x29)));
        assert_eq!(m.codes[3][13], 0x28); // Enter, col 12 empty
        assert_eq!(m.codes[5][10], 0x01); // Fn
        assert_eq!(m.codes[1][14], 0x49); // Insert
        assert_eq!(m.codes[4][14], 0x4E); // PgDn
    }

    #[test]
    fn requests_payloads() {
        let r = requests();
        assert_eq!(r[0].payload, vec![0, 0, 1]);
        assert_eq!(r[1].payload, vec![0, 2, 3]);
        assert_eq!(r[2].payload, vec![0, 4, 5]);
    }

    #[test]
    fn apply_rejects_wrong_cmd() {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0x81;
        let mut m = Matrix::default();
        assert_eq!(
            m.apply(&Response::decode(&raw).unwrap()),
            Err(FrameError::ShortReport)
        );
    }

    #[test]
    fn apply_rejects_bad_row() {
        let mut m = Matrix::default();
        assert_eq!(
            m.apply(&reply(6, &[1], 0, &[])),
            Err(FrameError::ShortReport)
        );
        assert_eq!(
            m.apply(&reply(0, &[1], 6, &[])),
            Err(FrameError::ShortReport)
        );
        assert!(m.is_empty());
    }
}

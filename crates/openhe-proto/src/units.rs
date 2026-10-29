//! Physical units used by the protocol.

use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    #[error("value is not a finite number")]
    NotFinite,
    #[error("value is negative")]
    Negative,
    #[error("value too large for 16 bits")]
    TooLarge,
    #[error("{value_um} um is below the minimum of {min_um} um")]
    BelowMin { value_um: u16, min_um: u16 },
    #[error("{value_um} um is above the maximum of {max_um} um")]
    AboveMax { value_um: u16, max_um: u16 },
}

/// A distance in micrometers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Micrometers(pub u16);

impl Micrometers {
    /// Converts millimeters, rounding to the nearest micrometer.
    pub fn from_mm(mm: f32) -> Result<Self, RangeError> {
        if !mm.is_finite() {
            return Err(RangeError::NotFinite);
        }
        if mm < 0.0 {
            return Err(RangeError::Negative);
        }
        // Widen to f64 first: f32 1.13 is 1.12999999523..., so 1129.99999523 rounds to 1130.
        let um = (f64::from(mm) * 1000.0).round();
        if um > f64::from(u16::MAX) {
            return Err(RangeError::TooLarge);
        }
        Ok(Micrometers(um as u16))
    }

    pub fn to_mm(self) -> f32 {
        f32::from(self.0) / 1000.0
    }

    pub fn to_le(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_not_truncates() {
        assert_eq!(Micrometers::from_mm(1.13), Ok(Micrometers(1130)));
        assert_eq!(Micrometers::from_mm(0.4), Ok(Micrometers(400)));
        assert_eq!(Micrometers::from_mm(1.0006), Ok(Micrometers(1001)));
        assert_eq!(Micrometers::from_mm(0.0005), Ok(Micrometers(1)));
        assert_eq!(Micrometers::from_mm(1.0004), Ok(Micrometers(1000)));
    }

    #[test]
    fn rejects_nan() {
        assert_eq!(Micrometers::from_mm(f32::NAN), Err(RangeError::NotFinite));
        assert_eq!(
            Micrometers::from_mm(f32::INFINITY),
            Err(RangeError::NotFinite)
        );
    }

    #[test]
    fn rejects_negative_and_too_large() {
        assert_eq!(Micrometers::from_mm(-0.5), Err(RangeError::Negative));
        assert_eq!(Micrometers::from_mm(-0.0004), Err(RangeError::Negative));
        assert_eq!(Micrometers::from_mm(65.536), Err(RangeError::TooLarge));
        assert_eq!(Micrometers::from_mm(65.535), Ok(Micrometers(65535)));
    }

    #[test]
    fn le_and_mm() {
        assert_eq!(Micrometers(3400).to_le(), [0x48, 0x0D]);
        assert!((Micrometers(1130).to_mm() - 1.13).abs() < 1e-6);
    }
}

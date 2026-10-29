//! Supported keyboards, their switches, and device settings enums.

use crate::units::{Micrometers, RangeError};

pub const VID: u16 = 0x1CA2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {
    Win68HeMax,
    Win60HeMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    pub id: u16,
    pub name: &'static str,
    pub max_travel: Micrometers,
}

const fn sw(id: u16, name: &'static str, max: u16) -> Switch {
    Switch {
        id,
        name,
        max_travel: Micrometers(max),
    }
}

static WIN68_SWITCHES: [Switch; 4] = [
    sw(0, "Wing Chun", 3400),
    sw(1, "Magnetic Jade Pro", 3300),
    sw(2, "Black Emperor", 3400),
    sw(3, "Magneto", 3300),
];

static WIN60_SWITCHES: [Switch; 4] = [
    sw(0, "Wing Chun", 3400),
    sw(1, "Black Emperor", 3400),
    sw(2, "Magneto", 3300),
    sw(3, "Magnetic Jade Pro", 3300),
];

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Model::Win68HeMax => "WIN 68 HE MAX",
            Model::Win60HeMax => "WIN 60 HE MAX",
        }
    }

    pub fn pid(self) -> u16 {
        match self {
            Model::Win68HeMax => 0x1901,
            Model::Win60HeMax => 0x1902,
        }
    }

    pub fn from_hid(vid: u16, pid: u16, product: &str) -> Option<Model> {
        if vid != VID {
            return None;
        }
        let model = match pid {
            0x1901 => Model::Win68HeMax,
            0x1902 => Model::Win60HeMax,
            _ => return None,
        };
        (product.trim() == model.name()).then_some(model)
    }

    pub fn switches(self) -> &'static [Switch] {
        match self {
            Model::Win68HeMax => &WIN68_SWITCHES,
            Model::Win60HeMax => &WIN60_SWITCHES,
        }
    }
}

/// Minimum actuation distance in micrometers.
pub const MIN_ACTUATION_UM: u16 = 100;

pub fn validate_actuation(v: Micrometers, sw: &Switch) -> Result<(), RangeError> {
    if v.0 < MIN_ACTUATION_UM {
        return Err(RangeError::BelowMin {
            value_um: v.0,
            min_um: MIN_ACTUATION_UM,
        });
    }
    if v > sw.max_travel {
        return Err(RangeError::AboveMax {
            value_um: v.0,
            max_um: sw.max_travel.0,
        });
    }
    Ok(())
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PollingRate {
    Hz8000 = 0,
    Hz4000 = 1,
    Hz2000 = 2,
    Hz1000 = 3,
    Hz500 = 4,
    Hz250 = 5,
    Hz125 = 6,
}

impl PollingRate {
    pub fn from_index(i: u8) -> Option<Self> {
        Some(match i {
            0 => Self::Hz8000,
            1 => Self::Hz4000,
            2 => Self::Hz2000,
            3 => Self::Hz1000,
            4 => Self::Hz500,
            5 => Self::Hz250,
            6 => Self::Hz125,
            _ => return None,
        })
    }

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn hz(self) -> u32 {
        match self {
            Self::Hz8000 => 8000,
            Self::Hz4000 => 4000,
            Self::Hz2000 => 2000,
            Self::Hz1000 => 1000,
            Self::Hz500 => 500,
            Self::Hz250 => 250,
            Self::Hz125 => 125,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemMode {
    Windows,
    Mac,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_by_product() {
        assert_eq!(
            Model::from_hid(0x1CA2, 0x1901, "WIN 68 HE MAX"),
            Some(Model::Win68HeMax)
        );
        assert_eq!(Model::from_hid(0x1CA2, 0x1901, "WIN 68 HE PRO"), None);
        assert_eq!(
            Model::from_hid(0x1CA2, 0x1902, "WIN 60 HE MAX"),
            Some(Model::Win60HeMax)
        );
        assert_eq!(
            Model::from_hid(0x1CA2, 0x1902, "  WIN 60 HE MAX \n"),
            Some(Model::Win60HeMax)
        );
        assert_eq!(Model::from_hid(0x1234, 0x1901, "WIN 68 HE MAX"), None);
        assert_eq!(Model::from_hid(0x1CA2, 0x1902, "WIN 68 HE MAX"), None);
    }

    #[test]
    fn switch_tables() {
        let s68 = Model::Win68HeMax.switches();
        assert_eq!(s68[1].name, "Magnetic Jade Pro");
        assert_eq!(s68[3].name, "Magneto");
        let s60 = Model::Win60HeMax.switches();
        assert_eq!(s60[1].name, "Black Emperor");
        assert_eq!(s60[3].max_travel, Micrometers(3300));
    }

    #[test]
    fn actuation_range() {
        let magneto = &Model::Win68HeMax.switches()[3];
        assert!(validate_actuation(Micrometers(50), magneto).is_err());
        assert!(validate_actuation(Micrometers(3400), magneto).is_err());
        assert_eq!(validate_actuation(Micrometers(3300), magneto), Ok(()));
        assert_eq!(validate_actuation(Micrometers(100), magneto), Ok(()));
    }

    #[test]
    fn polling_index() {
        assert_eq!(PollingRate::from_index(3), Some(PollingRate::Hz1000));
        assert_eq!(PollingRate::from_index(7), None);
        assert_eq!(PollingRate::Hz1000.index(), 3);
        assert_eq!(PollingRate::Hz125.hz(), 125);
    }
}

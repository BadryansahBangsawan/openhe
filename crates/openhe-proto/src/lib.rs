#![forbid(unsafe_code)]

pub mod advanced;
pub mod defkey;
pub mod frame;
pub mod key;
pub mod keycodes;
pub mod lighting;
pub mod macros;
pub mod model;
pub mod order;
pub mod sync;
pub mod travel;
pub mod units;

pub use defkey::Matrix;
pub use key::{Advanced, KeyMode, KeyParam, Layer, Slot, TouchMode};
pub use model::{validate_actuation, Model, PollingRate, Switch, SystemMode};
pub use order::OrderType;
pub use sync::DeviceInfo;
pub use units::{Micrometers, RangeError};

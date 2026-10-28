#![forbid(unsafe_code)]

pub mod defkey;
pub mod frame;
pub mod key;
pub mod model;
pub mod order;
pub mod sync;
pub mod units;

pub use defkey::Matrix;
pub use key::{Advanced, KeyMode, KeyParam, TouchMode};
pub use model::{validate_actuation, Model, PollingRate, Switch, SystemMode};
pub use order::OrderType;
pub use sync::DeviceInfo;
pub use units::{Micrometers, RangeError};

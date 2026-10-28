#![forbid(unsafe_code)]

pub mod frame;
pub mod model;
pub mod units;

pub use model::{validate_actuation, Model, PollingRate, Switch, SystemMode};
pub use units::{Micrometers, RangeError};

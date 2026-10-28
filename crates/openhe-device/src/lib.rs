#![forbid(unsafe_code)]

pub mod hid;
pub mod keyboard;
pub mod session;
pub mod transport;

use openhe_proto::frame::FrameError;
use thiserror::Error;

pub use hid::{list_devices, DeviceEntry, HidTransport};
pub use keyboard::advanced::{AdvancedConfig, FN_KEY};
pub use keyboard::macros::MacroBinding;
pub use keyboard::performance::PerformanceChange;
pub use keyboard::system::{DeviceEvent, WriteOutcome};
pub use keyboard::{KeySettings, Keyboard};
pub use session::Session;
pub use transport::{MockTransport, TimedTransport, Transport};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("timed out waiting for the keyboard")]
    Timeout,
    #[error("keyboard disconnected")]
    Disconnected,
    #[error("permission denied: {0}")]
    Permission(String),
    /// The keyboard is open in another program (macOS hidapi opens it
    /// exclusively: a second OpenHE, the CLI or AULA HUB).
    #[error("keyboard is in use by another program: {0}")]
    Busy(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("protocol error: {0}")]
    Frame(FrameError),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl From<FrameError> for Error {
    fn from(e: FrameError) -> Self {
        Error::Frame(e)
    }
}

#![forbid(unsafe_code)]

pub mod session;
pub mod transport;

use openhe_proto::frame::FrameError;
use thiserror::Error;

pub use session::Session;
pub use transport::{MockTransport, Transport};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("timed out waiting for the keyboard")]
    Timeout,
    #[error("keyboard disconnected")]
    Disconnected,
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("protocol error: {0}")]
    Frame(FrameError),
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<FrameError> for Error {
    fn from(e: FrameError) -> Self {
        Error::Frame(e)
    }
}

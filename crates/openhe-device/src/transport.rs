use crate::Error;
use std::collections::VecDeque;

/// A 64-byte HID report pipe. `read` blocks up to `timeout_ms` and returns
/// `Ok(None)` when nothing arrived.
pub trait Transport: Send {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error>;
    fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error>;
}

/// In-memory transport for tests: replies are served FIFO, never sleeping.
#[derive(Debug, Default)]
pub struct MockTransport {
    replies: VecDeque<[u8; 64]>,
    written: Vec<[u8; 64]>,
    failure: Option<Error>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_reply(&mut self, report: [u8; 64]) {
        self.replies.push_back(report);
    }

    /// Every subsequent `write` and `read` returns this error.
    pub fn fail_with(&mut self, e: Error) {
        self.failure = Some(e);
    }

    pub fn written(&self) -> &[[u8; 64]] {
        &self.written
    }
}

impl Transport for MockTransport {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error> {
        if let Some(e) = &self.failure {
            return Err(e.clone());
        }
        self.written.push(*report);
        Ok(())
    }

    fn read(&mut self, _timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
        if let Some(e) = &self.failure {
            return Err(e.clone());
        }
        Ok(self.replies.pop_front())
    }
}

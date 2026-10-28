use crate::Error;
use std::collections::VecDeque;

/// A 64-byte HID report pipe. `read` blocks up to `timeout_ms` and returns
/// `Ok(None)` when nothing arrived.
pub trait Transport: Send {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error>;
    fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error>;
}

/// In-memory transport for tests, never sleeping.
///
/// Two FIFO queues: `push_pending` reports are already waiting in the input
/// buffer, so any read returns them; `push_reply` reports stand for input that
/// arrives while a read waits, so only a read with `timeout_ms > 0` returns
/// them, and only once the pending queue is empty.
#[derive(Debug, Default)]
pub struct MockTransport {
    pending: VecDeque<[u8; 64]>,
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

    /// Queues a report that is readable immediately, even with `read(0)`.
    pub fn push_pending(&mut self, report: [u8; 64]) {
        self.pending.push_back(report);
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

    fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
        if let Some(e) = &self.failure {
            return Err(e.clone());
        }
        if let Some(r) = self.pending.pop_front() {
            return Ok(Some(r));
        }
        if timeout_ms == 0 {
            return Ok(None);
        }
        Ok(self.replies.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(tag: u8) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[0] = tag;
        r
    }

    #[test]
    fn zero_timeout_reads_only_pending_reports() {
        let mut m = MockTransport::new();
        m.push_reply(report(1));
        assert_eq!(m.read(0), Ok(None));
        m.push_pending(report(2));
        assert_eq!(m.read(0), Ok(Some(report(2))));
        assert_eq!(m.read(0), Ok(None));
        assert_eq!(m.read(1000), Ok(Some(report(1))));
    }

    #[test]
    fn waiting_read_serves_pending_before_replies() {
        let mut m = MockTransport::new();
        m.push_reply(report(1));
        m.push_pending(report(2));
        m.push_pending(report(3));
        assert_eq!(m.read(5), Ok(Some(report(2))));
        assert_eq!(m.read(5), Ok(Some(report(3))));
        assert_eq!(m.read(5), Ok(Some(report(1))));
        assert_eq!(m.read(5), Ok(None));
    }
}

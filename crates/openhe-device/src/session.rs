use crate::{transport::Transport, Error};
use openhe_proto::frame::{Request, Response};

use std::time::{Duration, Instant};

const ATTEMPT_MS: u64 = 1000;
const MAX_ATTEMPTS: usize = 3;

pub struct Session<T: Transport> {
    t: T,
    events: Vec<Response>,
}

impl<T: Transport> Session<T> {
    pub fn new(t: T) -> Self {
        Session {
            t,
            events: Vec::new(),
        }
    }

    pub fn transport(&self) -> &T {
        &self.t
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.t
    }

    pub fn take_events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.events)
    }

    /// Sends `req` and waits for its reply: 1000 ms per attempt, 3 attempts.
    /// Reports that do not answer `req` are queued as events.
    pub fn call(&mut self, req: &Request) -> Result<Response, Error> {
        let mut out = self.exchange(req, 1)?;
        let first = out.remove(0);
        Ok(Response::decode(&first)?)
    }

    /// Like `call`, but the matching reply is followed by `n - 1` headerless
    /// continuation reports, returned raw and in order (`n >= 1`).
    pub fn call_many(&mut self, req: &Request, n: usize) -> Result<Vec<[u8; 64]>, Error> {
        self.exchange(req, n.max(1))
    }

    fn exchange(&mut self, req: &Request, n: usize) -> Result<Vec<[u8; 64]>, Error> {
        let frame = req.encode()?;
        for _ in 0..MAX_ATTEMPTS {
            self.t.write(&frame)?;
            let deadline = Instant::now() + Duration::from_millis(ATTEMPT_MS);
            let mut got: Vec<[u8; 64]> = Vec::new();
            while got.len() < n {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let Some(raw) = self.t.read(left.as_millis().max(1) as u32)? else {
                    break;
                };
                if !got.is_empty() {
                    got.push(raw);
                    continue;
                }
                let resp = Response::decode(&raw)?;
                if req.expects(&resp) {
                    got.push(raw);
                } else {
                    self.events.push(resp);
                }
            }
            if got.len() == n {
                return Ok(got);
            }
        }
        Err(Error::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::key::{self, KeyParam};
    use openhe_proto::order::{self, OrderType};

    fn reply(cmd_byte: u8, ty: u8) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[2] = cmd_byte;
        r[5] = ty;
        r
    }

    #[test]
    fn reply_matched() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0xA3, 0));
        let mut s = Session::new(m);
        let r = s.call(&key::get(4, KeyParam::Actuation)).unwrap();
        assert_eq!(r.cmd, 0x23);
        assert_eq!(s.transport().written().len(), 1);
    }

    #[test]
    fn unsolicited_profile_event_queued() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0x80, 0x70));
        m.push_reply(reply(0x80, 0x21));
        let mut s = Session::new(m);
        let r = s.call(&order::query(OrderType::QueryWinMode)).unwrap();
        assert_eq!(r.raw[5], 0x21);
        let ev = s.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].raw[5], 0x70);
    }

    #[test]
    fn timeout_after_three_attempts() {
        let mut s = Session::new(MockTransport::new());
        let e = s.call(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Timeout);
        assert_eq!(s.transport().written().len(), 3);
    }

    #[test]
    fn disconnect_propagates() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let mut s = Session::new(m);
        let e = s.call(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Disconnected);
    }

    #[test]
    fn device_fail_is_error() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0xFF, 0));
        let mut s = Session::new(m);
        let e = s.call(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
    }

    #[test]
    fn call_many_collects_continuations() {
        let mut m = MockTransport::new();
        let mut a = reply(0x80 | 0x12, 0);
        a[10] = 1;
        let mut b = [0u8; 64];
        b[0] = 7;
        let mut c = [0u8; 64];
        c[0] = 8;
        m.push_reply(a);
        m.push_reply(b);
        m.push_reply(c);
        let mut s = Session::new(m);
        let req = Request {
            cmd: openhe_proto::frame::Cmd::Rm6x21,
            payload: vec![],
        };
        let v = s.call_many(&req, 3).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], a);
        assert_eq!(v[1][0], 7);
        assert_eq!(v[2][0], 8);
    }
}

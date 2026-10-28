use crate::{transport::Transport, Error};
use openhe_proto::frame::{Cmd, FrameError, Request, Response};
use openhe_proto::lighting;

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
    ///
    /// Before every write, input already waiting (late or unsolicited
    /// replies) is drained into the event queue, so it cannot be taken as the
    /// answer. Replies that do not answer `req` are queued as events; reports
    /// that are not protocol replies (`FrameError::NotAReply`) are dropped.
    /// Events survive a failed call.
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

    /// Like `call`, but with a single attempt: no reply within 1000 ms is
    /// `Error::Timeout`, and the request is never written twice. For commands
    /// that must not be repeated (factory reset) or that make the keyboard
    /// re-enumerate (polling rate). Draining and reply matching are the same.
    pub fn call_once(&mut self, req: &Request) -> Result<Response, Error> {
        self.call_once_tracked(req).0
    }

    /// `call_once`, also telling whether the frame reached the transport:
    /// `false` means the call failed before anything was written (encoding,
    /// the pre-write drain, or the write itself), so the keyboard never saw
    /// the request.
    pub fn call_once_tracked(&mut self, req: &Request) -> (Result<Response, Error>, bool) {
        let mut wrote = false;
        let res = self
            .exchange_tracked(req, 1, 1, &mut wrote)
            .and_then(|mut out| Ok(Response::decode(&out.remove(0))?));
        (res, wrote)
    }

    /// Non-blocking read of whatever input is waiting: protocol replies, in
    /// arrival order. Nothing is written. Reports that are not replies (noise,
    /// stale failure reports) are skipped. A read error ends the drain; use
    /// `try_poll` to see it.
    pub fn poll(&mut self) -> Vec<Response> {
        let mut out = Vec::new();
        let _ = self.drain_into(&mut out);
        out
    }

    /// `poll`, but a transport error (e.g. `Disconnected`) is returned. Replies
    /// read before the error are kept as events.
    pub fn try_poll(&mut self) -> Result<Vec<Response>, Error> {
        let mut out = Vec::new();
        match self.drain_into(&mut out) {
            Ok(()) => Ok(out),
            Err(e) => {
                self.events.append(&mut out);
                Err(e)
            }
        }
    }

    /// Waits `quiet`, then drains whatever arrived into the event queue, so
    /// a late reply to an earlier request (or an echo) cannot answer the
    /// next one. Nothing is written. For read-backs whose reply cannot be
    /// told apart from the late reply by its bytes (PRGB, where the SET
    /// reply, the GET reply and the keyboard's own lighting report all use
    /// 0x98).
    pub fn settle(&mut self, quiet: Duration) -> Result<(), Error> {
        std::thread::sleep(quiet);
        self.drain()
    }

    fn drain_into(&mut self, out: &mut Vec<Response>) -> Result<(), Error> {
        while let Some(raw) = self.t.read(0)? {
            if let Ok(resp) = Response::decode(&raw) {
                out.push(resp);
            }
        }
        Ok(())
    }

    /// Reads with a zero timeout until nothing is waiting. Replies become
    /// events; non-replies and stale failure reports are dropped, since they
    /// cannot belong to a request that has not been sent yet.
    fn drain(&mut self) -> Result<(), Error> {
        while let Some(raw) = self.t.read(0)? {
            if let Ok(resp) = Response::decode(&raw) {
                self.events.push(resp);
            }
        }
        Ok(())
    }

    fn exchange(&mut self, req: &Request, n: usize) -> Result<Vec<[u8; 64]>, Error> {
        self.exchange_tracked(req, n, MAX_ATTEMPTS, &mut false)
    }

    /// `*wrote` is set once a write has succeeded.
    fn exchange_tracked(
        &mut self,
        req: &Request,
        n: usize,
        attempts: usize,
        wrote: &mut bool,
    ) -> Result<Vec<[u8; 64]>, Error> {
        refuse_malformed_prgb_get(req)?;
        let frame = req.encode()?;
        for _ in 0..attempts {
            self.drain()?;
            self.t.write(&frame)?;
            *wrote = true;
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
                let resp = match Response::decode(&raw) {
                    Ok(resp) => resp,
                    Err(FrameError::NotAReply) => continue,
                    Err(e) => return Err(e.into()),
                };
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

/// A PRGB SET (flag 01) must be the LEN-44 layout, and every other PRGB
/// request must be exactly `lighting::get()`. The keyboard rejects any other GET (a LEN-44 GET got a
/// 0xFF DeviceFail reply) and right after that rejection it stopped sensing
/// keys until it was replugged (PROTOCOL 7.x finding 14), so nothing may
/// reach the transport that way.
fn refuse_malformed_prgb_get(req: &Request) -> Result<(), Error> {
    if req.cmd != Cmd::Prgb {
        return Ok(());
    }
    let n = req.payload.len();
    if req.payload.first() == Some(&0x01) {
        if n != lighting::PRGB_SET_LEN {
            return Err(Error::InvalidInput(format!(
                "PRGB SET must be LEN {}, not LEN {n}",
                lighting::PRGB_SET_LEN
            )));
        }
    } else if *req != lighting::get() {
        return Err(Error::InvalidInput(format!(
            "PRGB GET must be the LEN-15 form, not LEN {n}"
        )));
    }
    Ok(())
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
        r[0] = 0x5C;
        r[2] = cmd_byte;
        r[5] = ty;
        r
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut r = reply(0xA3, key);
        r[6] = layout;
        r[7..9].copy_from_slice(&v.to_le_bytes());
        r
    }

    #[test]
    fn reply_matched() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(4, 4, 0));
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
    fn unsolicited_key_event_for_another_key_is_queued_during_a_get() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0B, 4, 1500)); // the keyboard reports another key
        m.push_reply(key_reply(0x0A, 4, 2000)); // the real answer
        let mut s = Session::new(m);
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        let ev = s.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(key::parse_value(&ev[0]), Some((0x0B, 4, 1500)));
    }

    #[test]
    fn stale_input_is_drained_into_events_before_the_write() {
        let mut m = MockTransport::new();
        m.push_pending(key_reply(0x0A, 4, 1111)); // left over from an earlier exchange
        m.push_pending(reply(0x80, 0x70)); // an unsolicited profile event
        m.push_reply(key_reply(0x0A, 4, 2000)); // the answer to this request
        let mut s = Session::new(m);
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        let ev = s.take_events();
        assert_eq!(ev.len(), 2);
        assert_eq!(key::parse_value(&ev[0]), Some((0x0A, 4, 1111)));
        assert_eq!(ev[1].raw[5], 0x70);
        assert_eq!(s.transport().written().len(), 1);
    }

    /// Reports that are not protocol replies: wrong magic, or no 0x80 bit.
    fn noise() -> [[u8; 64]; 2] {
        let mut no_magic = key_reply(0x0A, 4, 2000);
        no_magic[0] = 0x00;
        let mut no_reply_bit = key_reply(0x0A, 4, 2000);
        no_reply_bit[2] = 0x23;
        [no_magic, no_reply_bit]
    }

    #[test]
    fn non_reply_reports_are_ignored() {
        let mut m = MockTransport::new();
        for n in noise() {
            m.push_pending(n); // seen by the drain
            m.push_reply(n); // seen while waiting for the reply
        }
        m.push_reply(key_reply(0x0A, 4, 2000));
        let mut s = Session::new(m);
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        assert!(s.take_events().is_empty());
        assert_eq!(s.transport().written().len(), 1);
    }

    #[test]
    fn events_queued_before_a_timeout_survive_it() {
        let mut m = MockTransport::new();
        m.push_pending(reply(0x80, 0x70)); // drained before the first write
        m.push_reply(key_reply(0x0B, 4, 1500)); // arrives while waiting; not the answer
        let mut s = Session::new(m);
        let e = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Timeout);
        let ev = s.take_events();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].raw[5], 0x70);
        assert_eq!(key::parse_value(&ev[1]), Some((0x0B, 4, 1500)));
    }

    /// Delivers reports when a frame is written, as a device would: the n-th
    /// write makes `deliveries[n]` readable. Any read pops the oldest one;
    /// nothing readable means the read timed out.
    struct Scripted {
        deliveries: std::collections::VecDeque<Vec<[u8; 64]>>,
        inbox: std::collections::VecDeque<[u8; 64]>,
        writes: usize,
    }

    impl Scripted {
        fn new(deliveries: Vec<Vec<[u8; 64]>>) -> Self {
            Scripted {
                deliveries: deliveries.into(),
                inbox: Default::default(),
                writes: 0,
            }
        }
    }

    impl Transport for Scripted {
        fn write(&mut self, _report: &[u8; 64]) -> Result<(), Error> {
            self.writes += 1;
            let now = self.deliveries.pop_front().unwrap_or_default();
            self.inbox.extend(now);
            Ok(())
        }

        fn read(&mut self, _timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
            Ok(self.inbox.pop_front())
        }
    }

    #[test]
    fn late_duplicate_reply_after_a_retry_does_not_desync_the_next_call() {
        let echo = key_reply(0x0A, 4, 1500);
        let s_set = &key::set(&[(0x0A, KeyParam::Actuation, 1500)])[0];
        let mut s = Session::new(Scripted::new(vec![
            vec![],                         // SET attempt 1: no reply in time
            vec![echo, echo],               // SET attempt 2: its echo, then attempt 1's late echo
            vec![key_reply(0x0A, 4, 2000)], // GET read-back: the stored value
        ]));
        s.call(s_set).unwrap();
        assert_eq!(s.transport().writes, 2);
        // The late echo looks like an answer to the read-back (same key and
        // layout). It must be drained, not taken as the stored value.
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        let ev = s.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(key::parse_value(&ev[0]), Some((0x0A, 4, 1500)));
    }

    #[test]
    fn call_many_collects_continuations() {
        let mut m = MockTransport::new();
        let mut a = reply(0x80 | 0x12, 2);
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
            payload: vec![2, 1, 0xFF, 0xFF],
        };
        let v = s.call_many(&req, 3).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], a);
        assert_eq!(v[1][0], 7);
        assert_eq!(v[2][0], 8);
    }

    #[test]
    fn call_once_matches_the_reply_with_one_write() {
        let mut m = MockTransport::new();
        m.push_pending(reply(0x80, 0x70)); // stale: drained into events first
        m.push_reply(key_reply(0x0B, 4, 1500)); // not the answer: queued
        m.push_reply(key_reply(0x0A, 4, 2000));
        let mut s = Session::new(m);
        let r = s.call_once(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        assert_eq!(s.transport().written().len(), 1);
        assert_eq!(s.take_events().len(), 2);
    }

    #[test]
    fn call_once_never_retries() {
        let mut s = Session::new(MockTransport::new());
        let e = s.call_once(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Timeout);
        assert_eq!(s.transport().written().len(), 1);
    }

    #[test]
    fn call_once_propagates_disconnect_and_device_fail() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let mut s = Session::new(m);
        let e = s.call_once(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Disconnected);

        let mut m = MockTransport::new();
        m.push_reply(reply(0xFF, 0));
        let mut s = Session::new(m);
        let e = s.call_once(&key::get(4, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        assert_eq!(s.transport().written().len(), 1);
    }

    #[test]
    fn call_once_tracked_says_whether_the_frame_was_written() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected); // fails the drain, before any write
        let mut s = Session::new(m);
        let (r, wrote) = s.call_once_tracked(&key::get(4, KeyParam::Actuation));
        assert_eq!(r.unwrap_err(), Error::Disconnected);
        assert!(!wrote);
        assert!(s.transport().written().is_empty());

        let mut s = Session::new(MockTransport::new());
        let (r, wrote) = s.call_once_tracked(&key::get(4, KeyParam::Actuation));
        assert_eq!(r.unwrap_err(), Error::Timeout);
        assert!(wrote);

        let mut m = MockTransport::new();
        m.push_reply(key_reply(4, 4, 7));
        let mut s = Session::new(m);
        let (r, wrote) = s.call_once_tracked(&key::get(4, KeyParam::Actuation));
        assert_eq!(key::parse_value(&r.unwrap()), Some((4, 4, 7)));
        assert!(wrote);
    }

    #[test]
    fn poll_returns_pending_reports_without_writing() {
        let mut m = MockTransport::new();
        m.push_pending(reply(0x80, 0x70));
        m.push_pending(key_reply(0x0A, 4, 1111));
        m.push_reply(reply(0x80, 0x21)); // only arrives while a read waits
        let mut s = Session::new(m);
        let got = s.poll();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].raw[5], 0x70);
        assert_eq!(key::parse_value(&got[1]), Some((0x0A, 4, 1111)));
        assert!(s.transport().written().is_empty());
        assert!(s.poll().is_empty());
        assert!(s.take_events().is_empty());
    }

    #[test]
    fn poll_skips_non_reply_reports() {
        let mut m = MockTransport::new();
        let [a, b] = noise();
        m.push_pending(a);
        m.push_pending(reply(0x80, 0x70));
        m.push_pending(b);
        let mut s = Session::new(m);
        let got = s.poll();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].raw[5], 0x70);
    }

    #[test]
    fn try_poll_reports_a_disconnect() {
        let mut s = Session::new(MockTransport::new());
        s.transport_mut().fail_with(Error::Disconnected);
        assert_eq!(s.try_poll().unwrap_err(), Error::Disconnected);
        assert!(s.poll().is_empty());
    }

    /// A PRGB SET must be the LEN-44 "1.0.9" layout (flag 01).
    #[test]
    fn a_prgb_set_of_another_length_is_never_written() {
        use openhe_proto::frame::Cmd;
        for len in [5usize, 15, 43, 45] {
            let mut payload = vec![0u8; len];
            payload[0] = 0x01;
            let req = Request {
                cmd: Cmd::Prgb,
                payload,
            };
            let mut s = Session::new(MockTransport::new());
            assert!(
                matches!(s.call_once(&req), Err(Error::InvalidInput(_))),
                "{len}"
            );
            assert!(s.transport().written().is_empty(), "{len}");
        }
    }

    /// A PRGB GET other than the LEN-15 form is refused before any write:
    /// the keyboard rejects a LEN-44 GET and then stopped sensing keys until
    /// it was replugged (PROTOCOL 7.x finding 14).
    #[test]
    fn a_malformed_prgb_get_is_never_written() {
        use openhe_proto::frame::Cmd;
        use openhe_proto::lighting;
        let bad = [
            vec![0u8; 44],
            vec![0u8; 14],
            vec![0u8; 16],
            Vec::new(),
            {
                let mut p = vec![0u8; 15];
                p[5] = 1;
                p
            },
            vec![0x02; 15],
        ];
        for payload in bad {
            let req = Request {
                cmd: Cmd::Prgb,
                payload: payload.clone(),
            };
            let mut s = Session::new(MockTransport::new());
            for r in [s.call(&req), s.call_once(&req)] {
                assert!(
                    matches!(r, Err(Error::InvalidInput(_))),
                    "{payload:?}: {r:?}"
                );
            }
            assert!(s.call_many(&req, 1).is_err());
            assert!(s.transport().written().is_empty(), "{payload:?}");
        }
        // The canonical GET and a SET (flag 01) still go out.
        let mut m = MockTransport::new();
        let mut r = [0u8; 64];
        r[0] = 0x5C;
        r[2] = 0x98;
        m.push_reply(r);
        let mut s = Session::new(m);
        assert!(s.call(&lighting::get()).is_ok());
        let set = Request {
            cmd: Cmd::Prgb,
            payload: {
                let mut p = vec![0u8; 44];
                p[0] = 1;
                p
            },
        };
        let _ = s.call_once(&set);
        assert_eq!(s.transport().written().len(), 2);
    }
}

use crate::{transport::Transport, Error};
use openhe_proto::frame::{Cmd, FrameError, Request, Response};
use openhe_proto::lighting;

use std::time::{Duration, Instant};

const ATTEMPT_MS: u64 = 1000;
const MAX_ATTEMPTS: usize = 3;

/// How long after an attempt timed out its reply is still expected (and a
/// matching report is taken for it rather than for a keyboard event).
const LATE_WINDOW: Duration = Duration::from_secs(2);
/// At most this many timed-out attempts are remembered.
const MAX_LATE: usize = 8;
/// Quiet wait after a call that needed a retry, so the reply of the
/// attempt that was not consumed can arrive and be drained.
const RETRY_SETTLE: Duration = Duration::from_millis(100);

/// At most this many KEY SET packets are remembered for their echoes.
const MAX_ECHOES: usize = 32;

/// An attempt that timed out: its reply may still arrive.
struct Late {
    req: Request,
    until: Instant,
    /// The call it belonged to (see `Session::calls`).
    call: u64,
}

/// Late replies (scan finding 4). A reply that arrives after its attempt's
/// 1 s deadline is a stray: it can answer the retry (whose own reply then
/// becomes the stray), turn up as a keyboard event, or, as a 0xFF reject,
/// abort an unrelated call. The session remembers attempts that timed out
/// for `LATE_WINDOW` and:
/// - drops a report that one of them explains (`Request::expects`) instead
///   of queueing it as an event, so a slow keyboard cannot cause a reload
///   storm through 0x80 / 0xA3 "events";
/// - absorbs one 0xFF per such attempt of an earlier call while waiting,
///   instead of failing the current call (when that leaves the last
///   attempt unanswered, the result is still `DeviceFail`);
/// - after a call that needed a retry, waits `RETRY_SETTLE` and drains, so
///   the duplicate reply cannot answer the next request.
///
/// A KEY SET packet whose per-entry echoes may still arrive (see
/// `Session::expect_echoes`).
struct Echo {
    req: Request,
    until: Instant,
}

/// Reports that no remembered attempt explains are handled as before.
///
/// Own echoes. A multi-entry KEY SET may echo
/// every entry (open question 17); only the first echo answers the call.
/// The entries of a KEY SET registered with `expect_echoes` are remembered
/// for `LATE_WINDOW`, and a KEY reply that repeats one of them exactly
/// (key, layout and value) is dropped instead of being queued as a
/// keyboard event, wherever it turns up (while a later request is awaited,
/// in `settle` or in `poll`).
///
/// Unsolicited reports inside a multi-report reply: while the headerless
/// continuations of an RM6X21 reply are awaited, a report that is clearly a
/// framed keyboard event (`framed_event`) is queued as an event instead of
/// being taken as travel data.
pub struct Session<T: Transport> {
    t: T,
    events: Vec<Response>,
    late: Vec<Late>,
    echoes: Vec<Echo>,
    calls: u64,
}

impl<T: Transport> Session<T> {
    pub fn new(t: T) -> Self {
        Session {
            t,
            events: Vec::new(),
            late: Vec::new(),
            echoes: Vec::new(),
            calls: 0,
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

    /// Remembers the entries of a KEY SET packet (flag 01) for
    /// `LATE_WINDOW`: a KEY reply that repeats one of them exactly (key,
    /// layout and value, the shape of a SET echo, PROTOCOL 7.x finding 11)
    /// is then the keyboard echoing this write, not a change made on the
    /// keyboard, and is dropped instead of being queued as an event. Every
    /// such repeat in the window is dropped: a report of a value that was
    /// just written tells nothing new. A reply that answers the request
    /// being made is still taken as its answer. Other requests are ignored.
    ///
    /// Register a packet before it is sent: its echoes may come in any
    /// order (open question 17).
    pub fn expect_echoes(&mut self, req: &Request) {
        if req.cmd != Cmd::Key || req.payload.first() != Some(&0x01) {
            return;
        }
        self.prune_echoes();
        if self.echoes.len() >= MAX_ECHOES {
            self.echoes.remove(0);
        }
        self.echoes.push(Echo {
            req: req.clone(),
            until: Instant::now() + LATE_WINDOW,
        });
    }

    fn drain_into(&mut self, out: &mut Vec<Response>) -> Result<(), Error> {
        while let Some(raw) = self.t.read(0)? {
            if let Ok(resp) = Response::decode(&raw) {
                if !self.is_own(&resp) {
                    out.push(resp);
                }
            }
        }
        Ok(())
    }

    /// Whether `resp` answers one of our own earlier writes (a late reply
    /// or an echo) rather than reporting a change on the keyboard.
    fn is_own(&mut self, resp: &Response) -> bool {
        self.take_late(resp) || self.is_echo(resp)
    }

    fn prune_echoes(&mut self) {
        let now = Instant::now();
        self.echoes.retain(|e| e.until > now);
    }

    /// Whether `resp` repeats an entry of a remembered KEY SET exactly.
    fn is_echo(&mut self, resp: &Response) -> bool {
        self.prune_echoes();
        if resp.cmd != Cmd::Key as u8 {
            return false;
        }
        let got = &resp.body()[1..5];
        self.echoes.iter().any(|e| {
            e.req.payload[1..]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|entry| entry[..] == *got)
        })
    }

    /// Reads with a zero timeout until nothing is waiting. Replies become
    /// events (unless they are late replies to our own timed-out attempts);
    /// non-replies and stale failure reports are dropped, since they cannot
    /// belong to a request that has not been sent yet.
    fn drain(&mut self) -> Result<(), Error> {
        let mut out = Vec::new();
        let r = self.drain_into(&mut out);
        self.events.append(&mut out);
        r
    }

    /// Forgets timed-out attempts whose reply is no longer expected.
    fn prune_late(&mut self) {
        let now = Instant::now();
        self.late.retain(|l| l.until > now);
    }

    /// Whether `resp` is the late reply of a remembered timed-out attempt;
    /// if so, that attempt is forgotten (its reply arrived).
    fn take_late(&mut self, resp: &Response) -> bool {
        self.prune_late();
        match self.late.iter().position(|l| l.req.expects(resp)) {
            Some(i) => {
                self.late.remove(i);
                true
            }
            None => false,
        }
    }

    /// Whether a 0xFF reject can be blamed on a timed-out attempt of an
    /// earlier call; if so, the oldest such attempt is forgotten.
    fn take_late_fail(&mut self, call: u64) -> bool {
        self.prune_late();
        match self.late.iter().position(|l| l.call != call) {
            Some(i) => {
                self.late.remove(i);
                true
            }
            None => false,
        }
    }

    fn remember_late(&mut self, req: &Request, call: u64) {
        if self.late.len() >= MAX_LATE {
            self.late.remove(0);
        }
        self.late.push(Late {
            req: req.clone(),
            until: Instant::now() + LATE_WINDOW,
            call,
        });
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
        self.calls += 1;
        let call = self.calls;
        let mut absorbed_fail = false;
        for attempt in 0..attempts {
            self.drain()?;
            self.t.write(&frame)?;
            *wrote = true;
            absorbed_fail = false;
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
                    // Headerless continuation data, unless the report is
                    // clearly a keyboard event that arrived in between.
                    match framed_event(&raw) {
                        Some(ev) => {
                            if !self.is_own(&ev) {
                                self.events.push(ev);
                            }
                        }
                        None => got.push(raw),
                    }
                    continue;
                }
                let resp = match Response::decode(&raw) {
                    Ok(resp) => resp,
                    Err(FrameError::NotAReply) => continue,
                    Err(FrameError::DeviceFail) if self.take_late_fail(call) => {
                        absorbed_fail = true;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                if req.expects(&resp) {
                    got.push(raw);
                } else if !self.is_own(&resp) {
                    self.events.push(resp);
                }
            }
            if got.len() == n {
                if attempt > 0 {
                    // An earlier attempt's reply is still on its way (or
                    // answered this one, leaving this attempt's reply to
                    // come): let it arrive and drain it now.
                    std::thread::sleep(RETRY_SETTLE);
                    let _ = self.drain();
                }
                return Ok(got);
            }
            self.remember_late(req, call);
        }
        if absorbed_fail {
            // The reject blamed on an earlier request may have been this
            // one's: a failure is reported, not a timeout.
            return Err(FrameError::DeviceFail.into());
        }
        Err(Error::Timeout)
    }
}

/// A report the keyboard pushes on its own, recognised inside the raw
/// continuation data of a multi-report (RM6X21) reply.
///
/// Continuations are raw u16 matrix values, so their first bytes can be
/// anything, 0x5C included (travel up to 3400 µm and calibration values of
/// about 2200..2400 can have 0x5C or 0x98 as a byte). Only a report with
/// all of the following is taken as an event:
/// - the 0x5C magic;
/// - a documented unsolicited command (PROTOCOL 2.3): 0x80 with the profile
///   (0x70) or Win/Mac (0x21) TYPE at R[1], or 0x98 (lighting);
/// - a valid checksum (`Response::checksum_ok`; every reply captured on
///   hardware has one, see `docs/protocol/captures`).
///
/// Random data passes all three about once in eight million reports.
fn framed_event(raw: &[u8; 64]) -> Option<Response> {
    let resp = Response::decode(raw).ok()?;
    let known = match resp.raw[2] {
        0x80 => matches!(resp.body()[1], 0x70 | 0x21),
        0x98 => true,
        _ => false,
    };
    (known && resp.checksum_ok()).then_some(resp)
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
        // layout). It must be drained, not taken as the stored value; and
        // as a reply to our own timed-out attempt it is not a device event.
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        assert!(s.take_events().is_empty());
    }

    /// Scan finding 4: attempt 1 answers after its 1 s deadline, so it
    /// satisfies attempt 2, and attempt 2's own reply arrives later. A
    /// retried call that succeeds settles briefly, so that reply cannot
    /// answer the read-back GET written next (a KEY SET echo is
    /// byte-identical to the GET reply, 7.x finding 11).
    #[test]
    fn a_retried_call_settles_before_the_next_request() {
        use crate::TimedTransport;
        let ms = Duration::from_millis;
        let echo = key_reply(0x0A, 4, 1500);
        let mut t = TimedTransport::new();
        t.on_write(vec![(ms(1050), echo)]); // attempt 1: late
        t.on_write(vec![(ms(30), echo)]); // attempt 2: its own echo
        t.on_write(vec![(ms(60), key_reply(0x0A, 4, 2000))]); // the stored value
        let mut s = Session::new(t);
        s.call(&key::set(&[(0x0A, KeyParam::Actuation, 1500)])[0])
            .unwrap();
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 2000)));
        assert_eq!(s.transport().written().len(), 3);
        assert!(s.take_events().is_empty(), "our own late reply is no event");
    }

    /// Scan finding 4: a late 0xFF reject of an earlier request that timed
    /// out (e.g. RGBNACK, sent once) must not abort the next, unrelated
    /// call. A reject with no earlier timeout to blame is still DeviceFail.
    #[test]
    fn a_late_reject_of_a_timed_out_request_does_not_abort_the_next_call() {
        let nack = order::set(OrderType::RgbNack, 1);
        let mut s = Session::new(Scripted::new(vec![
            vec![],                                      // RGBNACK: no reply in time
            vec![reply(0xFF, 0), key_reply(0x0A, 4, 7)], // its late reject, then the answer
        ]));
        assert_eq!(s.call_once(&nack).unwrap_err(), Error::Timeout);
        let r = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0A, 4, 7)));
        assert_eq!(s.transport().writes, 2);

        // The blame is used up: the next reject is this call's own.
        let mut s = Session::new(Scripted::new(vec![
            vec![],               // RGBNACK: no reply in time
            vec![reply(0xFF, 0)], // absorbed as the late reject
            vec![reply(0xFF, 0)], // the retry's own reject
        ]));
        assert_eq!(s.call_once(&nack).unwrap_err(), Error::Timeout);
        let e = s.call(&key::get(0x0A, KeyParam::Actuation)).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        assert_eq!(s.transport().writes, 3);

        // A single-attempt call reports an absorbed reject, not a timeout.
        let mut s = Session::new(Scripted::new(vec![vec![], vec![reply(0xFF, 0)]]));
        assert_eq!(s.call_once(&nack).unwrap_err(), Error::Timeout);
        let e = s
            .call_once(&key::get(0x0A, KeyParam::Actuation))
            .unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
    }

    /// Scan finding 4: a late reply to our own timed-out request is not a
    /// keyboard event (0xA3 and 0x80 events start a full reload in the
    /// app). Replies that no recent request explains still are.
    #[test]
    fn late_replies_to_timed_out_requests_are_not_events() {
        let mut s = Session::new(MockTransport::new());
        let get = key::get(0x0A, KeyParam::Actuation);
        assert_eq!(s.call(&get).unwrap_err(), Error::Timeout);
        assert_eq!(
            s.call_once(&order::query(OrderType::Profile)).unwrap_err(),
            Error::Timeout
        );
        let m = s.transport_mut();
        m.push_pending(key_reply(0x0A, 4, 2000)); // late: ours
        m.push_pending(reply(0x80, 0x70)); // late: ours
        m.push_pending(key_reply(0x0B, 4, 2000)); // another key: an event
        m.push_pending(reply(0x80, 0x21)); // Win/Mac: an event
        let got = s.poll();
        assert_eq!(got.len(), 2);
        assert_eq!(key::parse_value(&got[0]), Some((0x0B, 4, 2000)));
        assert_eq!(got[1].raw[5], 0x21);
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

    /// A report with a correct `5C LEN CMD CHK` header.
    fn framed(cmd_byte: u8, body: &[u8]) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[0] = 0x5C;
        r[1] = body.len() as u8;
        r[2] = cmd_byte;
        r[4..4 + body.len()].copy_from_slice(body);
        r[3] = openhe_proto::frame::checksum(r[1], cmd_byte, &r[4..]);
        r
    }

    fn travel_req() -> Request {
        Request {
            cmd: openhe_proto::frame::Cmd::Rm6x21,
            payload: vec![2, 1, 0xFF, 0xFF],
        }
    }

    /// A profile event (0x80 TYPE 0x70) and a lighting report (0x98) that
    /// arrive between the header of an RM6X21 reply and its continuations
    /// are queued as events, not taken as travel data; the call goes on
    /// waiting for the real continuations.
    #[test]
    fn an_event_between_travel_reports_is_queued_not_taken_as_data() {
        let profile = framed(0x80, &[0x00, 0x70, 0x02, 0xFF]);
        // The shape of a profile reply captured on hardware.
        let captured = framed(0x80, &[0x00, 0x70, 0x00, 0xFF]);
        assert_eq!(
            &captured[..8],
            &[0x5C, 0x04, 0x80, 0x14, 0x00, 0x70, 0x00, 0xFF]
        );
        let mut light_body = vec![0u8; 0x2D];
        light_body[37] = 0xC1;
        let lighting = framed(0x98, &light_body);
        let mut a = reply(0x80 | 0x12, 2);
        a[10] = 1;
        let mut b = [0u8; 64];
        b[0] = 7;
        let mut c = [0u8; 64];
        c[0] = 8;
        let mut m = MockTransport::new();
        for r in [a, lighting, b, profile, c] {
            m.push_reply(r);
        }
        let mut s = Session::new(m);
        let v = s.call_many(&travel_req(), 3).unwrap();
        assert_eq!(v, vec![a, b, c]);
        let ev = s.take_events();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].raw, lighting);
        assert_eq!(ev[1].raw, profile);
        assert_eq!(s.transport().written().len(), 1);
    }

    /// Continuation data is raw u16 values and may start with 0x5C: only
    /// a documented event command with a valid checksum is taken out.
    #[test]
    fn continuation_data_that_only_looks_framed_is_kept() {
        let mut bad_sum = framed(0x98, &[0u8; 0x2D]);
        bad_sum[3] ^= 0x01;
        let mut bad_sum_order = framed(0x80, &[0x00, 0x70, 0x02, 0xFF]);
        bad_sum_order[3] ^= 0x01;
        let other_type = framed(0x80, &[0x00, 0x50, 0x03, 0xFF]); // polling, not an event
        let other_cmd = framed(0x92, &[0x00, 0x02]); // a travel header
        let key = framed(0xA3, &[0x00, 0x0A, 0x04, 0xDC, 0x05]); // not pushed by the keyboard
        for data in [bad_sum, bad_sum_order, other_type, other_cmd, key] {
            let mut a = reply(0x80 | 0x12, 2);
            a[10] = 1;
            let mut c = [0u8; 64];
            c[0] = 8;
            let mut m = MockTransport::new();
            for r in [a, data, c] {
                m.push_reply(r);
            }
            let mut s = Session::new(m);
            let v = s.call_many(&travel_req(), 3).unwrap();
            assert_eq!(v, vec![a, data, c], "{:02X?}", &data[..8]);
            assert!(s.take_events().is_empty());
        }
    }

    /// The entries of a registered KEY SET are our own echoes: an exact
    /// repeat (key, layout, value) is no event, wherever it turns up. A
    /// KEY report with another value or key, and any other report, still
    /// is; and an identical reply still answers a read-back GET.
    #[test]
    fn echoes_of_a_registered_key_set_are_not_events() {
        let set = key::set(&[
            (0x0A, KeyParam::Actuation, 1500),
            (0x0B, KeyParam::Actuation, 1500),
        ])
        .remove(0);
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0B, 4, 1500)); // second entry first: not the answer, ours
        m.push_reply(key_reply(0x0A, 4, 1500)); // the SET reply
        let mut s = Session::new(m);
        s.expect_echoes(&set);
        s.call(&set).unwrap();
        assert!(s.take_events().is_empty());

        let m = s.transport_mut();
        m.push_pending(key_reply(0x0B, 4, 1500)); // ours
        m.push_pending(key_reply(0x0A, 4, 1500)); // ours (a duplicate)
        m.push_pending(key_reply(0x0B, 4, 1600)); // another value: an event
        m.push_pending(key_reply(0x0C, 4, 1500)); // another key: an event
        m.push_pending(key_reply(0x0B, 20, 1500)); // another layout: an event
        m.push_pending(reply(0x80, 0x70)); // a profile event
        let got = s.poll();
        let keys: Vec<_> = got.iter().filter_map(key::parse_value).collect();
        assert_eq!(
            keys,
            vec![(0x0B, 4, 1600), (0x0C, 4, 1500), (0x0B, 20, 1500)]
        );
        assert_eq!(got.len(), 4);
        assert_eq!(got[3].raw[5], 0x70);

        s.transport_mut().push_reply(key_reply(0x0B, 4, 1500));
        let r = s.call(&key::get(0x0B, KeyParam::Actuation)).unwrap();
        assert_eq!(key::parse_value(&r), Some((0x0B, 4, 1500)));

        // A GET is not a write: nothing is registered for it.
        let mut s = Session::new(MockTransport::new());
        s.expect_echoes(&key::get(0x0A, KeyParam::Actuation));
        s.transport_mut().push_pending(key_reply(0x0A, 4, 0));
        assert_eq!(s.poll().len(), 1);
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

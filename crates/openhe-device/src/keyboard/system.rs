//! Keyboard system, profile and Win/Mac mode operations, device events, and
//! live travel / calibration readouts.
//!
//! Everything here is an `impl<T: Transport> Keyboard<T>` block plus the two
//! public types it needs: [`DeviceEvent`] and [`WriteOutcome`].

use super::Keyboard;
use crate::{Error, Transport};
use openhe_proto::frame::Response;
use openhe_proto::model::{PollingRate, SystemMode};
use openhe_proto::order::{self, OrderType};
use openhe_proto::sync::{self, DeviceInfo};
use openhe_proto::travel::{self, TravelFrame, SUB_CALIBRATION, SUB_TRAVEL};

/// Something the keyboard reported on its own (a key combo, Fn+profile, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The profile was switched on the keyboard (0..=3).
    ProfileChanged(u8),
    /// Win/Mac mode changed; `None` when the keyboard reports neither.
    SystemModeChanged(Option<SystemMode>),
    /// The main lighting state changed (reply 0x98).
    LightingChanged,
    /// Key data changed (replies 0xA3 and 0xAB).
    KeyDataChanged,
    /// Anything else: the report's command byte (reply bit set).
    Other(u8),
}

impl DeviceEvent {
    /// Classifies an unsolicited reply: order 0x80 with TYPE 0x70 (profile) or
    /// 0x21 (Win/Mac), 0x98, 0xA3/0xAB, else `Other(raw command byte)`. A
    /// profile outside 0..=3 is `Other(0x80)`.
    pub fn from_response(r: &Response) -> DeviceEvent {
        let cmd = r.raw[2];
        match cmd {
            0x80 => match r.body()[1] {
                0x70 => match order::parse_profile(r) {
                    Some(p) => DeviceEvent::ProfileChanged(p),
                    None => DeviceEvent::Other(cmd),
                },
                0x21 => DeviceEvent::SystemModeChanged(order::parse_system_mode(r)),
                _ => DeviceEvent::Other(cmd),
            },
            0x98 => DeviceEvent::LightingChanged,
            0xA3 | 0xAB => DeviceEvent::KeyDataChanged,
            _ => DeviceEvent::Other(cmd),
        }
    }
}

/// How a write that can restart the keyboard ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The keyboard answered.
    Done,
    /// No answer, or the keyboard disconnected after the write: it is
    /// presumably re-enumerating on USB. Reconnect and read the setting back.
    Reenumerating,
}

impl<T: Transport> Keyboard<T> {
    /// Non-blocking: events queued by earlier calls (`take_events`), then
    /// whatever input is waiting right now. Nothing is written.
    ///
    /// The input is read first and the queue taken only afterwards, so a
    /// transport error (e.g. `Disconnected`) leaves every queued event in
    /// place for `take_events`.
    pub fn poll_events(&mut self) -> Result<Vec<DeviceEvent>, Error> {
        let polled = self.session.try_poll()?;
        let mut out: Vec<DeviceEvent> = self
            .session
            .take_events()
            .iter()
            .map(DeviceEvent::from_response)
            .collect();
        out.extend(polled.iter().map(DeviceEvent::from_response));
        Ok(out)
    }

    /// SYNC. `Error::Unsupported("bootloader")` when the keyboard is in its
    /// bootloader, since none of the other commands apply there.
    pub fn handshake(&mut self) -> Result<DeviceInfo, Error> {
        let r = self.session.call(&sync::request())?;
        let info = sync::parse(&r);
        if info.in_bootloader {
            return Err(Error::Unsupported("bootloader".into()));
        }
        Ok(info)
    }

    /// Switches the onboard profile (0..=3) with exactly one attempt.
    ///
    /// The keyboard re-enumerates on USB after a profile switch, like after
    /// a polling-rate change (PROTOCOL 7.x finding 23), so there is no
    /// read-back here: a timeout or disconnect after the write is
    /// `Reenumerating`, and the caller reconnects and reads `profile()`
    /// from the new connection. An index above 3 is `InvalidInput` before
    /// any I/O; an error before the frame was written propagates.
    pub fn set_profile(&mut self, idx: u8) -> Result<WriteOutcome, Error> {
        if idx > 3 {
            return Err(Error::InvalidInput(format!(
                "profile {idx} out of range 0..=3"
            )));
        }
        let (res, wrote) = self
            .session
            .call_once_tracked(&order::set(OrderType::Profile, idx));
        match res {
            Ok(_) => Ok(WriteOutcome::Done),
            Err(Error::Disconnected | Error::Timeout) if wrote => Ok(WriteOutcome::Reenumerating),
            Err(e) => Err(e),
        }
    }

    /// Switches Win/Mac mode: the reply must report success (R[2] == 1), and
    /// the mode is re-queried to confirm.
    pub fn set_system_mode(&mut self, m: SystemMode) -> Result<(), Error> {
        let ty = match m {
            SystemMode::Windows => OrderType::SetWin,
            SystemMode::Mac => OrderType::SetMac,
        };
        let r = self.session.call(&order::set(ty, 1))?;
        if r.body()[2] != 1 {
            return Err(Error::Io(format!(
                "keyboard refused the {m:?} mode switch: {:02X?}",
                &r.body()[..4]
            )));
        }
        let got = self.system_mode()?;
        if got != Some(m) {
            return Err(Error::Io(format!(
                "verify failed: system mode: wrote {m:?}, read {got:?}"
            )));
        }
        Ok(())
    }

    /// Sets the polling rate with exactly one attempt. The keyboard probably
    /// restarts, so a timeout or disconnect after the write is
    /// `Reenumerating`, not an error. An error before the frame was written
    /// (e.g. the keyboard was already unplugged) propagates, as does any other
    /// error.
    pub fn set_polling_rate(&mut self, r: PollingRate) -> Result<WriteOutcome, Error> {
        let (res, wrote) = self
            .session
            .call_once_tracked(&order::set(OrderType::Polling, r.index()));
        match res {
            Ok(_) => Ok(WriteOutcome::Done),
            Err(Error::Disconnected | Error::Timeout) if wrote => Ok(WriteOutcome::Reenumerating),
            Err(e) => Err(e),
        }
    }

    /// Restores factory settings. Exactly one attempt, never retried: any
    /// matching reply is `Done`.
    ///
    /// The board most likely restarts, so a timeout or disconnect **after**
    /// the write is `Reenumerating` (re-read the device state), like
    /// `set_polling_rate`. An error before the frame was written (e.g. a
    /// handle that is already dead) propagates: nothing was reset.
    pub fn factory_reset(&mut self) -> Result<WriteOutcome, Error> {
        let (res, wrote) = self
            .session
            .call_once_tracked(&order::query(OrderType::FactoryReset));
        match res {
            Ok(_) => Ok(WriteOutcome::Done),
            Err(Error::Disconnected | Error::Timeout) if wrote => Ok(WriteOutcome::Reenumerating),
            Err(e) => Err(e),
        }
    }

    pub fn start_calibration(&mut self) -> Result<(), Error> {
        self.session
            .call(&order::query(OrderType::StartCalibration))?;
        Ok(())
    }

    pub fn stop_calibration(&mut self) -> Result<(), Error> {
        self.session
            .call(&order::query(OrderType::CloseCalibration))?;
        Ok(())
    }

    /// Live key travel in micrometers: RM6X21 (2,1) for rows 0..=2 and (2,2)
    /// for rows 3..=5.
    pub fn travel(&mut self) -> Result<TravelFrame, Error> {
        self.rm6x21_frame(SUB_TRAVEL)
    }

    /// Raw calibration values: RM6X21 (6,1) and (6,2).
    pub fn calibration_values(&mut self) -> Result<TravelFrame, Error> {
        self.rm6x21_frame(SUB_CALIBRATION)
    }

    fn rm6x21_frame(&mut self, sub: u8) -> Result<TravelFrame, Error> {
        let top = self.rm6x21_half(sub, 1)?;
        let bottom = self.rm6x21_half(sub, 2)?;
        TravelFrame::from_halves(&top, &bottom).map_err(|e| Error::Io(e.to_string()))
    }

    fn rm6x21_half(&mut self, sub: u8, part: u8) -> Result<travel::TravelHalf, Error> {
        let reports = self
            .session
            .call_many(&travel::request(sub, part), travel::REPORTS)?;
        let half = travel::parse(&reports)?;
        if half.sub != sub {
            return Err(Error::Io(format!(
                "RM6X21 ({sub},{part}) answered with SUB {}",
                half.sub
            )));
        }
        Ok(half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::model::{Model, PollingRate, SystemMode};
    use openhe_proto::travel::{SUB_CALIBRATION, SUB_TRAVEL};

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    /// An order reply `[00, ty, value, FF]`.
    fn order_reply(ty: u8, v: u8) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x5C, 0x04, 0x80, 0x00]);
        r[4..8].copy_from_slice(&[0, ty, v, 0xFF]);
        r
    }

    /// A reply with the given command byte and first body bytes.
    fn reply(cmd_byte: u8, body: &[u8]) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x5C, 0x04, cmd_byte]);
        r[4..4 + body.len()].copy_from_slice(body);
        r
    }

    fn sync_reply(in_bootloader: bool) -> [u8; 64] {
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x5C, 0x3C, 0x81]);
        r[4 + 7] = in_bootloader as u8;
        r[4 + 8] = 4;
        r[4 + 9..4 + 13].copy_from_slice(b"S123");
        r[4 + 25] = 10;
        r[4 + 26..4 + 36].copy_from_slice(b"App V1.1.3");
        r
    }

    /// An RM6X21 reply of 3 reports: row-major `value` from body offset 2.
    fn rm_reply(sub: u8, base: u16) -> [[u8; 64]; 3] {
        let mut all = [0u8; 192];
        all[..4].copy_from_slice(&[0x5C, 0x3C, 0x92, 0x00]);
        all[5] = sub;
        for i in 0..63 {
            let at = 6 + 2 * i;
            all[at..at + 2].copy_from_slice(&(base + i as u16).to_le_bytes());
        }
        let mut out = [[0u8; 64]; 3];
        for (i, rep) in out.iter_mut().enumerate() {
            rep.copy_from_slice(&all[i * 64..(i + 1) * 64]);
        }
        out
    }

    fn assert_frame(frame: &[u8; 64], head: &[u8]) {
        assert_eq!(&frame[..head.len()], head);
        assert!(frame[head.len()..].iter().all(|&b| b == 0));
    }

    // Checksum = 0x35 + 0x5C + LEN + CMD + last payload byte (0xFF in all of
    // these): order query 0x93 (LEN 3), order set 0x94 (LEN 4), SYNC 0x97
    // (LEN 6, CMD 1), RM6X21 0xA6 (LEN 4, CMD 0x12).

    #[test]
    fn handshake_bytes_and_parse() {
        let mut m = MockTransport::new();
        m.push_reply(sync_reply(false));
        let mut k = kb(m);
        let info = k.handshake().unwrap();
        assert_eq!(info.serial, "S123");
        assert_eq!(info.firmware, "App V1.1.3");
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 1);
        assert_frame(&w[0], &[0x5C, 0x06, 0x01, 0x97, 1, 2, 3, 4, 0xFF, 0xFF]);
    }

    #[test]
    fn handshake_in_bootloader_is_unsupported() {
        let mut m = MockTransport::new();
        m.push_reply(sync_reply(true));
        let e = kb(m).handshake().unwrap_err();
        assert_eq!(e, Error::Unsupported("bootloader".into()));
    }

    #[test]
    fn set_profile_done_on_reply_with_one_write() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x70, 2)); // SET reply
        let mut k = kb(m);
        assert_eq!(k.set_profile(2), Ok(WriteOutcome::Done));
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 1, "no read-back: the keyboard re-enumerates");
        assert_frame(&w[0], &[0x5C, 0x04, 0x00, 0x94, 0x70, 2, 0xFF, 0xFF]);
    }

    /// A profile switch re-enumerates USB like a polling change (PROTOCOL
    /// 7.x finding 23): silence or a disconnect after the write is
    /// `Reenumerating`, and the SET is never written twice.
    #[test]
    fn set_profile_timeout_or_disconnect_after_the_write_is_reenumerating() {
        let mut k = kb(MockTransport::new());
        assert_eq!(k.set_profile(1), Ok(WriteOutcome::Reenumerating));
        assert_eq!(k.session_mut().transport().written().len(), 1);
        let mut k = Keyboard::new(
            GoneAfterWrite {
                writes: 0,
                wrote: false,
                err: Error::Disconnected,
            },
            Model::Win68HeMax,
        );
        assert_eq!(k.set_profile(3), Ok(WriteOutcome::Reenumerating));
        assert_eq!(k.session_mut().transport().writes, 1);
    }

    #[test]
    fn set_profile_errors_before_the_write_or_other_errors_propagate() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected); // gone before anything was written
        assert_eq!(kb(m).set_profile(1), Err(Error::Disconnected));
        let mut k = Keyboard::new(
            GoneAfterWrite {
                writes: 0,
                wrote: false,
                err: Error::Io("boom".into()),
            },
            Model::Win68HeMax,
        );
        assert_eq!(k.set_profile(1), Err(Error::Io("boom".into())));
    }

    #[test]
    fn set_profile_rejects_out_of_range_before_any_io() {
        let mut k = kb(MockTransport::new());
        assert!(matches!(k.set_profile(4), Err(Error::InvalidInput(_))));
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn set_system_mode_bytes_and_requery() {
        for (mode, ty, reported) in [
            (SystemMode::Windows, 0x30u8, 1u8),
            (SystemMode::Mac, 0x31, 0),
        ] {
            let mut m = MockTransport::new();
            m.push_reply(order_reply(ty, 1)); // R[2]==1: accepted
            m.push_reply(order_reply(0x21, reported)); // re-query
            let mut k = kb(m);
            k.set_system_mode(mode).unwrap();
            let w = k.session_mut().transport().written().to_vec();
            assert_eq!(w.len(), 2);
            assert_frame(&w[0], &[0x5C, 0x04, 0x00, 0x94, ty, 1, 0xFF, 0xFF]);
            assert_frame(&w[1], &[0x5C, 0x03, 0x00, 0x93, 0x21, 0xFF, 0xFF]);
        }
    }

    #[test]
    fn set_system_mode_refused_or_unconfirmed_is_an_error() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x31, 0)); // R[2] != 1
        let mut k = kb(m);
        assert!(matches!(
            k.set_system_mode(SystemMode::Mac),
            Err(Error::Io(_))
        ));
        assert_eq!(k.session_mut().transport().written().len(), 1); // no re-query

        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x31, 1));
        m.push_reply(order_reply(0x21, 1)); // still Windows
        let e = kb(m).set_system_mode(SystemMode::Mac).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
    }

    #[test]
    fn set_polling_rate_done_on_reply() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x50, 3));
        let mut k = kb(m);
        assert_eq!(
            k.set_polling_rate(PollingRate::Hz1000),
            Ok(WriteOutcome::Done)
        );
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 1);
        assert_frame(&w[0], &[0x5C, 0x04, 0x00, 0x94, 0x50, 3, 0xFF, 0xFF]);
    }

    #[test]
    fn set_polling_rate_timeout_is_reenumerating_after_one_write() {
        let mut k = kb(MockTransport::new());
        assert_eq!(
            k.set_polling_rate(PollingRate::Hz500),
            Ok(WriteOutcome::Reenumerating)
        );
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    /// Accepts writes, then the device is gone: every read is `Disconnected`.
    struct GoneAfterWrite {
        writes: usize,
        wrote: bool,
        err: Error,
    }

    impl Transport for GoneAfterWrite {
        fn write(&mut self, _r: &[u8; 64]) -> Result<(), Error> {
            self.writes += 1;
            self.wrote = true;
            Ok(())
        }

        fn read(&mut self, _t: u32) -> Result<Option<[u8; 64]>, Error> {
            if self.wrote {
                Err(self.err.clone())
            } else {
                Ok(None)
            }
        }
    }

    #[test]
    fn set_polling_rate_disconnect_after_the_write_is_reenumerating() {
        let mut k = Keyboard::new(
            GoneAfterWrite {
                writes: 0,
                wrote: false,
                err: Error::Disconnected,
            },
            Model::Win68HeMax,
        );
        assert_eq!(
            k.set_polling_rate(PollingRate::Hz250),
            Ok(WriteOutcome::Reenumerating)
        );
        assert_eq!(k.session_mut().transport().writes, 1);
    }

    #[test]
    fn set_polling_rate_other_errors_propagate() {
        let mut k = Keyboard::new(
            GoneAfterWrite {
                writes: 0,
                wrote: false,
                err: Error::Io("boom".into()),
            },
            Model::Win68HeMax,
        );
        assert_eq!(
            k.set_polling_rate(PollingRate::Hz250),
            Err(Error::Io("boom".into()))
        );
    }

    #[test]
    fn factory_reset_bytes_and_any_reply_is_ok() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x11, 1));
        let mut k = kb(m);
        assert_eq!(k.factory_reset(), Ok(WriteOutcome::Done));
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 1);
        assert_frame(&w[0], &[0x5C, 0x03, 0x00, 0x93, 0x11, 0xFF, 0xFF]);
    }

    #[test]
    fn factory_reset_is_never_retried() {
        let mut k = kb(MockTransport::new());
        assert_eq!(k.factory_reset(), Ok(WriteOutcome::Reenumerating));
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn factory_reset_disconnect_before_or_after_the_write() {
        // After the write: the board restarts.
        let mut k = Keyboard::new(
            GoneAfterWrite {
                writes: 0,
                wrote: false,
                err: Error::Disconnected,
            },
            Model::Win68HeMax,
        );
        assert_eq!(k.factory_reset(), Ok(WriteOutcome::Reenumerating));
        // Before the write: nothing was sent.
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        assert_eq!(kb(m).factory_reset(), Err(Error::Disconnected));
    }

    #[test]
    fn calibration_start_and_stop_bytes() {
        let mut m = MockTransport::new();
        m.push_reply(order_reply(0x0C, 1));
        m.push_reply(order_reply(0x0D, 1));
        let mut k = kb(m);
        k.start_calibration().unwrap();
        k.stop_calibration().unwrap();
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 2);
        assert_frame(&w[0], &[0x5C, 0x03, 0x00, 0x93, 0x0C, 0xFF, 0xFF]);
        assert_frame(&w[1], &[0x5C, 0x03, 0x00, 0x93, 0x0D, 0xFF, 0xFF]);
    }

    fn push_all(m: &mut MockTransport, reps: [[u8; 64]; 3]) {
        for r in reps {
            m.push_reply(r);
        }
    }

    #[test]
    fn travel_requests_two_parts_and_joins_rows() {
        let mut m = MockTransport::new();
        push_all(&mut m, rm_reply(SUB_TRAVEL, 1000));
        push_all(&mut m, rm_reply(SUB_TRAVEL, 5000));
        let mut k = kb(m);
        let f = k.travel().unwrap();
        let w = k.session_mut().transport().written().to_vec();
        assert_eq!(w.len(), 2);
        assert_frame(&w[0], &[0x5C, 0x04, 0x12, 0xA6, 2, 1, 0xFF, 0xFF]);
        assert_frame(&w[1], &[0x5C, 0x04, 0x12, 0xA6, 2, 2, 0xFF, 0xFF]);
        assert_eq!(f.um[0][0], 1000);
        assert_eq!(f.um[2][20], 1062);
        assert_eq!(f.um[3][0], 5000);
        assert_eq!(f.um[5][20], 5062);
    }

    /// Replies PART 1 twice (a duplicate, as after a retry) and PART 2
    /// once, each right after its request.
    struct DuplicatePart1 {
        out: std::collections::VecDeque<[u8; 64]>,
    }

    impl Transport for DuplicatePart1 {
        fn write(&mut self, r: &[u8; 64]) -> Result<(), Error> {
            let part = r[5];
            let value = if part == 1 { 1000 } else { 5000 };
            let times = if part == 1 { 2 } else { 1 };
            for _ in 0..times {
                self.out.extend(rm_reply(SUB_TRAVEL, value));
            }
            Ok(())
        }

        fn read(&mut self, _t: u32) -> Result<Option<[u8; 64]>, Error> {
            Ok(self.out.pop_front())
        }
    }

    /// RM6X21 replies are matched on SUB only (they carry no PART). A
    /// second PART-1 reply that is already waiting when PART 2 is
    /// requested is drained before the write, so it is never taken as the
    /// PART-2 answer. (One that arrives *while* PART 2 is awaited cannot be
    /// told apart; the app ignores frames that do not fit the matrix.)
    #[test]
    fn a_late_duplicate_part1_reply_is_not_taken_for_part2() {
        let mut k = Keyboard::new(
            DuplicatePart1 {
                out: Default::default(),
            },
            Model::Win68HeMax,
        );
        let f = k.travel().unwrap();
        assert_eq!(f.um[0][0], 1000);
        assert_eq!(f.um[3][0], 5000, "rows 3..5 come from the PART-2 reply");
        assert_eq!(f.um[5][20], 5062);
    }

    #[test]
    fn calibration_values_use_sub_six() {
        let mut m = MockTransport::new();
        push_all(&mut m, rm_reply(SUB_CALIBRATION, 10));
        push_all(&mut m, rm_reply(SUB_CALIBRATION, 20));
        let mut k = kb(m);
        let f = k.calibration_values().unwrap();
        let w = k.session_mut().transport().written().to_vec();
        assert_frame(&w[0], &[0x5C, 0x04, 0x12, 0xA6, 6, 1, 0xFF, 0xFF]);
        assert_frame(&w[1], &[0x5C, 0x04, 0x12, 0xA6, 6, 2, 0xFF, 0xFF]);
        assert_eq!((f.um[0][0], f.um[3][0]), (10, 20));
    }

    #[test]
    fn travel_with_the_wrong_sub_or_a_short_reply_is_an_error() {
        let mut m = MockTransport::new();
        push_all(&mut m, rm_reply(SUB_CALIBRATION, 0)); // asked for travel
        let mut k = kb(m);
        // The session does not take a reply with another SUB as the answer.
        assert_eq!(k.travel(), Err(Error::Timeout));
        assert_eq!(k.take_events()[0].raw[2], 0x92);

        // Header + only one continuation: the call times out, never guesses.
        let mut m = MockTransport::new();
        let r = rm_reply(SUB_TRAVEL, 0);
        m.push_reply(r[0]);
        m.push_reply(r[1]);
        assert_eq!(kb(m).travel(), Err(Error::Timeout));
    }

    #[test]
    fn event_mapping() {
        let ev = |cmd, body: &[u8]| {
            DeviceEvent::from_response(&Response::decode(&reply(cmd, body)).unwrap())
        };
        assert_eq!(
            ev(0x80, &[0, 0x70, 2, 0xFF]),
            DeviceEvent::ProfileChanged(2)
        );
        assert_eq!(
            ev(0x80, &[0, 0x21, 1, 0xFF]),
            DeviceEvent::SystemModeChanged(Some(SystemMode::Windows))
        );
        assert_eq!(
            ev(0x80, &[0, 0x21, 0, 0xFF]),
            DeviceEvent::SystemModeChanged(Some(SystemMode::Mac))
        );
        assert_eq!(
            ev(0x80, &[0, 0x21, 255, 0xFF]),
            DeviceEvent::SystemModeChanged(None)
        );
        assert_eq!(ev(0x98, &[0]), DeviceEvent::LightingChanged);
        assert_eq!(ev(0xA3, &[0, 4, 4]), DeviceEvent::KeyDataChanged);
        assert_eq!(ev(0xAB, &[0, 1]), DeviceEvent::KeyDataChanged);
        assert_eq!(ev(0x99, &[0]), DeviceEvent::Other(0x99));
        assert_eq!(ev(0x80, &[0, 0x50, 3, 0xFF]), DeviceEvent::Other(0x80));
        assert_eq!(ev(0x80, &[0, 0x70, 9, 0xFF]), DeviceEvent::Other(0x80));
        assert_eq!(ev(0x92, &[0]), DeviceEvent::Other(0x92));
    }

    #[test]
    fn poll_events_converts_queued_events_then_drains_waiting_input() {
        let mut m = MockTransport::new();
        // An unsolicited profile event while a call waits: queued by the session.
        m.push_reply(order_reply(0x70, 1));
        m.push_reply(order_reply(0x21, 1));
        m.push_pending(reply(0x98, &[0]));
        let mut k = kb(m);
        // (the pending report is drained into the queue before the write)
        assert_eq!(k.system_mode(), Ok(Some(SystemMode::Windows)));
        k.session_mut()
            .transport_mut()
            .push_pending(reply(0xA3, &[0, 4, 4]));
        let ev = k.poll_events().unwrap();
        assert_eq!(
            ev,
            vec![
                DeviceEvent::LightingChanged,
                DeviceEvent::ProfileChanged(1),
                DeviceEvent::KeyDataChanged,
            ]
        );
        assert!(k.poll_events().unwrap().is_empty());
        assert!(k.take_events().is_empty());
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn poll_events_surfaces_a_disconnect() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        assert_eq!(kb(m).poll_events(), Err(Error::Disconnected));
    }

    #[test]
    fn poll_events_failure_keeps_queued_events() {
        let mut m = MockTransport::new();
        m.push_pending(order_reply(0x70, 3)); // drained into the queue by the call
        m.push_reply(order_reply(0x21, 1));
        let mut k = kb(m);
        assert_eq!(k.system_mode(), Ok(Some(SystemMode::Windows)));
        // The keyboard is unplugged before the next poll.
        k.session_mut()
            .transport_mut()
            .fail_with(Error::Disconnected);
        assert_eq!(k.poll_events(), Err(Error::Disconnected));
        // The queued profile event is not lost.
        let ev = k.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(
            DeviceEvent::from_response(&ev[0]),
            DeviceEvent::ProfileChanged(3)
        );
    }

    #[test]
    fn set_polling_rate_disconnect_before_the_write_propagates() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        let mut k = kb(m);
        assert_eq!(
            k.set_polling_rate(PollingRate::Hz1000),
            Err(Error::Disconnected)
        );
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn frame_errors_are_not_swallowed() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0xFF, &[]));
        assert_eq!(kb(m).handshake(), Err(Error::Frame(FrameError::DeviceFail)));
    }
}

//! Keyboard lighting (CMD 0x18 PRGB and 0x2A KRGB) operations.

use crate::{Error, Keyboard, Transport};
use openhe_proto::lighting::{self, Lighting, LightingError, Rgb};
use openhe_proto::order::{self, OrderType};
use std::collections::HashSet;

/// A lighting reply that does not parse or holds out-of-range values: a
/// problem with what the keyboard sent (`Error::Io`). Values the caller
/// passes in are checked with [`invalid`] instead.
impl From<LightingError> for Error {
    fn from(e: LightingError) -> Self {
        Error::Io(format!("unexpected lighting reply: {e}"))
    }
}

/// A lighting value from the caller that the firmware would refuse.
fn invalid(e: LightingError) -> Error {
    Error::InvalidInput(e.to_string())
}

impl<T: Transport> Keyboard<T> {
    /// Reads the main light state.
    ///
    /// Sends RGBNACK (order 0x47, arg 1) before the first read of a
    /// connection, as the official app does before reading lighting, then
    /// a PRGB GET.
    ///
    /// RGBNACK is best effort and goes out **once per connection**, with a
    /// single attempt (`call_once`, at most one 1 s wait): FW App V1.1.3
    /// rejects it with a 0xFF reply (DeviceFail) while the GET works
    /// without it (PROTOCOL 7.x finding 22), so its answer never decides
    /// the read. A reply, a DeviceFail, a timeout or any other error marks
    /// it as tried and the GET follows; only `Disconnected` fails the read
    /// (the GET could not be sent either).
    pub fn lighting(&mut self) -> Result<Lighting, Error> {
        if !self.rgbnack_sent {
            match self.session.call_once(&order::set(OrderType::RgbNack, 1)) {
                Err(Error::Disconnected) => return Err(Error::Disconnected),
                Ok(_) | Err(_) => self.rgbnack_sent = true,
            }
        }
        let r = self.session.call(&lighting::get())?;
        Ok(lighting::parse(&r)?)
    }

    /// Writes the main light state, then reads it back with `lighting` and
    /// compares every field except `extra_flags`.
    ///
    /// `extra_flags` (flag bits other than on and reverse, meaning unknown)
    /// is written as given but not verified: the keyboard may keep or set
    /// its own bits (it reported 0xC1 where the official app writes 0x10),
    /// so a state that was not read from this keyboard, such as an older
    /// profile file, would otherwise always fail. A read value written back
    /// reads back identically on hardware.
    ///
    /// The speed of static and custom modes is sent as 0, so that is what the
    /// read-back must show. Out-of-range fields are `Error::InvalidInput`,
    /// returned before any I/O; a read-back with out-of-range values is an
    /// `Error::Io`. A difference is an `Error::Io` starting with
    /// "verify failed".
    pub fn set_lighting(&mut self, l: &Lighting) -> Result<(), Error> {
        let req = lighting::set(l).map_err(invalid)?;
        self.session.call(&req)?;
        let got = self.lighting()?;
        let want = Lighting {
            speed: l.wire_speed(),
            extra_flags: got.extra_flags,
            ..*l
        };
        if got != want {
            return Err(Error::Io(format!(
                "verify failed: lighting: wrote {want:?}, read {got:?}"
            )));
        }
        Ok(())
    }

    /// Reads the custom colour of each key (KRGB GET, one packet per key).
    pub fn key_colors(&mut self, keys: &[u8]) -> Result<Vec<(u8, Rgb)>, Error> {
        let mut out = Vec::with_capacity(keys.len());
        for &key in keys {
            let r = self.session.call(&lighting::krgb_get(key))?;
            let (got_key, c) = lighting::krgb_parse(&r)?;
            if got_key != key {
                return Err(Error::Io(format!(
                    "unexpected KRGB reply for key 0x{key:02X}: key 0x{got_key:02X}"
                )));
            }
            out.push((key, c));
        }
        Ok(out)
    }

    /// Writes custom per-key colours, then commits them with `[01, FE, FE, FE, FE]`.
    ///
    /// Each KRGB SET reply echoes `[st, key, R, G, B]`; the echo is compared
    /// with what was sent (there is no separate read-back). The commit is sent
    /// once after every batch that succeeded, never after a failure, and an
    /// empty batch does no I/O. A key listed twice, or key 0xFE (the commit
    /// marker), is `Error::InvalidInput` before any I/O.
    pub fn set_key_colors(&mut self, colors: &[(u8, Rgb)]) -> Result<(), Error> {
        let mut seen = HashSet::new();
        let mut reqs = Vec::with_capacity(colors.len());
        for &(key, c) in colors {
            if !seen.insert(key) {
                return Err(Error::InvalidInput(format!(
                    "duplicate colour for key 0x{key:02X}"
                )));
            }
            reqs.push(lighting::krgb_set(key, c).map_err(invalid)?);
        }
        if reqs.is_empty() {
            return Ok(());
        }
        for (req, &(key, want)) in reqs.iter().zip(colors) {
            let r = self.session.call(req)?;
            let (got_key, got) = lighting::krgb_parse(&r)?;
            if got_key != key || got != want {
                return Err(Error::Io(format!(
                    "verify failed: key 0x{key:02X}: wrote {want:?}, reply was key 0x{got_key:02X} {got:?}"
                )));
            }
        }
        self.session.call(&lighting::krgb_commit())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::frame::FrameError;
    use openhe_proto::model::Model;

    fn payload(frame: &[u8; 64]) -> &[u8] {
        &frame[4..4 + usize::from(frame[1])]
    }

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn sample() -> Lighting {
        Lighting {
            on: true,
            reverse: false,
            brightness: 3,
            mode: 1,
            speed: 2,
            sleep_minutes: 10,
            static_color: 7,
            dynamic_color: 3,
            colors: [
                rgb(0xFF, 0, 0),
                rgb(0, 0xFF, 0),
                rgb(0, 0, 0xFF),
                rgb(1, 2, 3),
                rgb(0x10, 0x20, 0x30),
                rgb(0xAA, 0xBB, 0xCC),
                rgb(0, 0, 0),
            ],
            extra_flags: lighting::DEFAULT_EXTRA_FLAGS,
        }
    }

    fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    fn nack_reply() -> [u8; 64] {
        reply(0x00, &[0, 0x47, 1, 0xFF])
    }

    fn prgb_reply(l: &Lighting) -> [u8; 64] {
        reply(0x18, &lighting::set(l).unwrap().payload)
    }

    fn krgb_reply(key: u8, c: Rgb) -> [u8; 64] {
        reply(0x2A, &[0, key, c.r, c.g, c.b])
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn lighting_sends_rgbnack_before_the_get() {
        let mut m = MockTransport::new();
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&sample()));
        let mut k = kb(m);
        assert_eq!(k.lighting(), Ok(sample()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2);
        // Checksum: 0x91 + LEN 4 + CMD 0 + last payload byte 0xFF = 0x194 -> 0x94.
        assert_eq!(&w[0][..4], &[0x5C, 0x04, 0x00, 0x94]);
        assert_eq!(payload(&w[0]), [0x47, 0x01, 0xFF, 0xFF]);
        // The LEN-15 GET (flag 00 + 14 zeros); the keyboard rejects LEN 44.
        // Checksum: 0x91 + 0x0F + 0x18 + 0 = 0xB8.
        let mut get = [0u8; 64];
        get[..4].copy_from_slice(&[0x5C, 0x0F, 0x18, 0xB8]);
        assert_eq!(w[1], get, "GET uses flag 00 and LEN 15");
    }

    /// FW App V1.1.3 rejects RGBNACK with a 0xFF reply (PROTOCOL 7.x
    /// finding 22); the PRGB GET after it works, so the read goes on.
    #[test]
    fn a_rejected_rgbnack_does_not_stop_the_read() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0x7F, &[])); // 0xFF: DeviceFail for RGBNACK
        m.push_reply(prgb_reply(&sample()));
        let mut k = kb(m);
        assert_eq!(k.lighting(), Ok(sample()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 2, "RGBNACK once, then the GET");
        assert_eq!(payload(&w[0]), [0x47, 0x01, 0xFF, 0xFF]);
        assert_eq!(&w[1][..5], &[0x5C, 0x0F, 0x18, 0xB8, 0x00]);
    }

    /// Answers PRGB GETs only; RGBNACK gets no reply at all.
    struct SilentNack {
        out: Vec<[u8; 64]>,
        written: Vec<[u8; 64]>,
    }

    impl Transport for SilentNack {
        fn write(&mut self, r: &[u8; 64]) -> Result<(), Error> {
            self.written.push(*r);
            if r[2] == 0x18 {
                self.out.push(prgb_reply(&sample()));
            }
            Ok(())
        }

        fn read(&mut self, _t: u32) -> Result<Option<[u8; 64]>, Error> {
            Ok(self.out.pop())
        }
    }

    /// RGBNACK is best effort: a keyboard that never answers it (Timeout)
    /// is still read, with a single RGBNACK attempt, and RGBNACK is not
    /// sent again on the next read.
    #[test]
    fn an_unanswered_rgbnack_does_not_stop_the_read() {
        let mut k = Keyboard::new(
            SilentNack {
                out: Vec::new(),
                written: Vec::new(),
            },
            Model::Win68HeMax,
        );
        assert_eq!(k.lighting(), Ok(sample()));
        assert_eq!(k.lighting(), Ok(sample()));
        let w = &k.session_mut().transport().written;
        let nacks = w.iter().filter(|f| f[2] == 0x00 && f[4] == 0x47).count();
        assert_eq!(nacks, 1, "one RGBNACK attempt, never repeated");
        assert_eq!(w.len(), 3, "RGBNACK, GET, GET");
    }

    #[test]
    fn a_disconnect_on_rgbnack_fails_the_read() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        assert_eq!(kb(m).lighting(), Err(Error::Disconnected));
    }

    #[test]
    fn a_read_lighting_with_unknown_flags_is_written_back_and_verified() {
        let mut body = lighting::set(&sample()).unwrap().payload;
        body[0] = 0;
        body[37] = 0xC1;
        let mut m = MockTransport::new();
        m.push_reply(reply(0x7F, &[])); // RGBNACK rejected
        m.push_reply(reply(0x18, &body));
        let mut k = kb(m);
        let read = k.lighting().unwrap();
        let m = k.session_mut().transport_mut();
        m.push_reply(reply(0x18, &body)); // SET reply
        m.push_reply(reply(0x18, &body)); // read-back (no RGBNACK again)
        assert_eq!(k.set_lighting(&read), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 4, "RGBNACK, GET, SET, GET");
        assert_eq!(w[2][4], 0x01, "the SET");
        assert_eq!(w[2][4 + 37], 0xC1);
        assert_eq!(w[2][5..48], body[1..44]);
    }

    /// RGBNACK goes out once per connection: FW App V1.1.3 rejects it
    /// anyway, and it is not needed for the GET (PROTOCOL 7.x finding 22).
    #[test]
    fn rgbnack_is_sent_once_per_connection() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0x7F, &[])); // RGBNACK rejected
        m.push_reply(prgb_reply(&sample()));
        m.push_reply(prgb_reply(&sample()));
        let mut k = kb(m);
        assert_eq!(k.lighting(), Ok(sample()));
        assert_eq!(k.lighting(), Ok(sample()));
        let w = k.session_mut().transport().written();
        let nacks = w.iter().filter(|f| f[2] == 0x00 && f[4] == 0x47).count();
        assert_eq!(nacks, 1);
        assert_eq!(w.len(), 3, "RGBNACK, GET, GET");
    }

    /// The flag bits other than on/reverse are not verified: an import of
    /// an older file (default 0x10) onto a keyboard reporting 0xC1 must not
    /// fail. On, reverse and every other field still are.
    #[test]
    fn set_lighting_does_not_verify_the_unknown_flag_bits() {
        let mut body = lighting::set(&sample()).unwrap().payload;
        body[0] = 0;
        body[37] = 0xC1; // the keyboard keeps its own bits 6/7, clears bit4
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&sample())); // SET reply
        m.push_reply(nack_reply());
        m.push_reply(reply(0x18, &body)); // read-back
        assert_eq!(kb(m).set_lighting(&sample()), Ok(()));
        // An on/off difference is still a verify failure.
        body[37] = 0xC0;
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&sample()));
        m.push_reply(nack_reply());
        m.push_reply(reply(0x18, &body));
        let e = kb(m).set_lighting(&sample()).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
    }

    #[test]
    fn set_lighting_writes_then_verifies_by_get() {
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&sample())); // SET reply
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&sample())); // read-back
        let mut k = kb(m);
        assert_eq!(k.set_lighting(&sample()), Ok(()));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 3);
        assert_eq!(&w[0][..4], &[0x5C, 0x2C, 0x18, 0xD8]);
        assert_eq!(w[0][4], 0x01, "SET uses flag 01");
        assert_eq!(payload(&w[1]), [0x47, 0x01, 0xFF, 0xFF]);
        assert_eq!(&w[2][..5], &[0x5C, 0x0F, 0x18, 0xB8, 0x00]);
    }

    #[test]
    fn set_lighting_mismatch_is_a_verify_error() {
        let mut other = sample();
        other.brightness = 1;
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&sample()));
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&other));
        let e = kb(m).set_lighting(&sample()).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
    }

    #[test]
    fn set_lighting_expects_speed_zero_for_static() {
        let mut l = sample();
        l.mode = 0;
        l.speed = 4;
        let mut shown = l;
        shown.speed = 0;
        let mut m = MockTransport::new();
        m.push_reply(prgb_reply(&shown));
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&shown));
        let mut k = kb(m);
        assert_eq!(k.set_lighting(&l), Ok(()));
        assert_eq!(k.session_mut().transport().written()[0][4 + 40], 0);
    }

    #[test]
    fn set_lighting_validates_before_any_io() {
        let mut l = sample();
        l.sleep_minutes = 4;
        let mut k = kb(MockTransport::new());
        let e = k.set_lighting(&l).unwrap_err();
        assert!(matches!(e, Error::InvalidInput(_)), "{e:?}");
        assert!(k.session_mut().transport().written().is_empty());
    }

    #[test]
    fn an_out_of_range_lighting_read_is_an_io_error() {
        // The keyboard reports brightness 9: a reply problem, not bad input.
        let mut body = lighting::set(&sample()).unwrap().payload;
        body[38] = 9;
        let mut m = MockTransport::new();
        m.push_reply(nack_reply());
        m.push_reply(reply(0x18, &body));
        let e = kb(m).lighting().unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.contains("brightness 9")),
            "{e:?}"
        );
    }

    #[test]
    fn lighting_reply_errors_are_io_errors() {
        let e = Error::from(LightingError::WrongReply("KRGB"));
        assert!(matches!(e, Error::Io(_)), "{e:?}");
    }

    #[test]
    fn failed_set_is_not_read_back() {
        let mut m = MockTransport::new();
        m.push_reply(reply(0x7F, &[])); // 0xFF: device failure
        m.push_reply(nack_reply());
        m.push_reply(prgb_reply(&sample()));
        let mut k = kb(m);
        let e = k.set_lighting(&sample()).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn key_colors_reads_each_key() {
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, rgb(1, 2, 3)));
        m.push_reply(krgb_reply(0x0B, rgb(4, 5, 6)));
        let mut k = kb(m);
        assert_eq!(
            k.key_colors(&[0x0A, 0x0B]),
            Ok(vec![(0x0A, rgb(1, 2, 3)), (0x0B, rgb(4, 5, 6))])
        );
        let w = k.session_mut().transport().written();
        assert_eq!(payload(&w[0]), [0x00, 0x0A]);
        assert_eq!(payload(&w[1]), [0x00, 0x0B]);
    }

    #[test]
    fn key_colors_rejects_a_reply_for_another_key() {
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0B, rgb(1, 2, 3)));
        let e = kb(m).key_colors(&[0x0A]).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
    }

    #[test]
    fn set_key_colors_commits_after_the_batch() {
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, rgb(0x11, 0x22, 0x33)));
        m.push_reply(krgb_reply(0x0B, rgb(0, 0, 0)));
        m.push_reply(krgb_reply(0xFE, rgb(0xFE, 0xFE, 0xFE)));
        let mut k = kb(m);
        assert_eq!(
            k.set_key_colors(&[(0x0A, rgb(0x11, 0x22, 0x33)), (0x0B, rgb(0, 0, 0))]),
            Ok(())
        );
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 3);
        assert_eq!(
            &w[0][..9],
            &[0x5C, 0x05, 0x2A, 0xF3, 0x01, 0x0A, 0x11, 0x22, 0x33]
        );
        assert_eq!(payload(&w[1]), [0x01, 0x0B, 0, 0, 0]);
        assert_eq!(
            &w[2][..9],
            &[0x5C, 0x05, 0x2A, 0xBE, 0x01, 0xFE, 0xFE, 0xFE, 0xFE]
        );
    }

    #[test]
    fn every_batch_gets_its_own_commit() {
        let mut m = MockTransport::new();
        for _ in 0..2 {
            m.push_reply(krgb_reply(0x0A, rgb(9, 9, 9)));
            m.push_reply(krgb_reply(0xFE, rgb(0xFE, 0xFE, 0xFE)));
        }
        let mut k = kb(m);
        k.set_key_colors(&[(0x0A, rgb(9, 9, 9))]).unwrap();
        k.set_key_colors(&[(0x0A, rgb(9, 9, 9))]).unwrap();
        let commits = k
            .session_mut()
            .transport()
            .written()
            .iter()
            .filter(|f| payload(f) == [0x01, 0xFE, 0xFE, 0xFE, 0xFE])
            .count();
        assert_eq!(commits, 2);
    }

    #[test]
    fn echo_mismatch_fails_and_skips_the_commit() {
        let mut m = MockTransport::new();
        m.push_reply(krgb_reply(0x0A, rgb(9, 9, 9))); // echoes another colour
        let mut k = kb(m);
        let e = k.set_key_colors(&[(0x0A, rgb(1, 2, 3))]).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
        assert_eq!(k.session_mut().transport().written().len(), 1);
    }

    #[test]
    fn key_colors_batch_rejects_bad_input_before_any_io() {
        let mut k = kb(MockTransport::new());
        let dup = k
            .set_key_colors(&[(0x0A, rgb(1, 2, 3)), (0x0A, rgb(4, 5, 6))])
            .unwrap_err();
        assert!(matches!(dup, Error::InvalidInput(_)), "{dup:?}");
        let reserved = k.set_key_colors(&[(0xFE, rgb(1, 2, 3))]).unwrap_err();
        assert!(matches!(reserved, Error::InvalidInput(_)), "{reserved:?}");
        assert_eq!(k.set_key_colors(&[]), Ok(()));
        assert!(k.session_mut().transport().written().is_empty());
    }
}

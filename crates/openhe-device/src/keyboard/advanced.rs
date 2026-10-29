//! Keyboard advanced-key (CMD 0x24-0x28 and 0x2C) and layer remap operations.

use super::Keyboard;
use crate::{Error, Transport};
use openhe_proto::advanced::{self, AdvancedError, Dks, End, Mpt, Mt, Socd, Tgl};
use openhe_proto::frame::{Request, Response};
use openhe_proto::key::{Advanced, KeyMode, KeyParam, Layer, Slot};

/// The advanced function bound to a key, as read from or written to the
/// keyboard.
///
/// `Macro` is read-only here: a macro is bound with `Keyboard::bind_macro`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvancedConfig {
    None,
    Dks(Dks),
    Mt(Mt),
    Tgl(Tgl),
    Mpt(Mpt),
    End(End),
    Socd(Socd),
    Macro,
}

const SLOTS: [Slot; 4] = [Slot::S1, Slot::S2, Slot::S3, Slot::S4];

/// The Fn key's own default code (its key id in every per-key command).
pub const FN_KEY: u8 = 0x01;
/// Layer code 0x0001: transparent ("follow Base") on the Fn layers.
const TRANSPARENT: u16 = 0x0001;

fn invalid(e: AdvancedError) -> Error {
    Error::InvalidInput(e.to_string())
}

fn bad_reply(e: AdvancedError) -> Error {
    Error::Io(format!("unexpected advanced-key reply: {e}"))
}

/// MPT, TGL and END replies carry the key at R[1] (PROTOCOL 3.6, inferred
/// from the vendor parsers). `Session` does not match on it, since the
/// layout is not hardware-verified; a reply for another key (stray or
/// late) is an `Io` error here instead of being read as this key's data.
fn reply_key_is(key: u8, what: &str, r: &Response) -> Result<(), Error> {
    let got = r.body()[1];
    if got == key {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "unexpected {what} reply for key 0x{key:02X}: key 0x{got:02X}"
        )))
    }
}

impl AdvancedConfig {
    /// The advanced-mode nibble of the key's Mode for this function.
    pub fn advanced(&self) -> Advanced {
        match self {
            AdvancedConfig::None => Advanced::None,
            AdvancedConfig::Dks(_) => Advanced::Dks,
            AdvancedConfig::Mt(_) => Advanced::Mt,
            AdvancedConfig::Tgl(_) => Advanced::Tgl,
            AdvancedConfig::Mpt(_) => Advanced::Mpt,
            AdvancedConfig::End(_) => Advanced::End,
            AdvancedConfig::Socd(_) => Advanced::Socd,
            AdvancedConfig::Macro => Advanced::Macro,
        }
    }
}

/// The function packet for `cfg` on `key`, validated; `None` for "no function".
fn function_packet(key: u8, cfg: &AdvancedConfig) -> Result<Option<Request>, Error> {
    let packet = match cfg {
        AdvancedConfig::None => return Ok(None),
        AdvancedConfig::Macro => {
            return Err(Error::InvalidInput(
                "macros are bound with bind_macro, not set_advanced".to_string(),
            ))
        }
        AdvancedConfig::Dks(d) => advanced::dks_set(key, d),
        AdvancedConfig::Mt(m) => advanced::mt_set(key, m),
        AdvancedConfig::Tgl(t) => advanced::tgl_set(key, t),
        AdvancedConfig::Mpt(m) => advanced::mpt_set(key, m),
        AdvancedConfig::End(e) => Ok(advanced::end_set(key, e)),
        AdvancedConfig::Socd(s) => {
            if key != s.pos1 && key != s.pos2 {
                return Err(Error::InvalidInput(format!(
                    "key 0x{key:02X} is not one of the SOCD pair (0x{:02X}, 0x{:02X})",
                    s.pos1, s.pos2
                )));
            }
            advanced::socd_set(s)
        }
    };
    packet.map(Some).map_err(invalid)
}

impl<T: Transport> Keyboard<T> {
    /// The Mode of `key`; an unknown value is an `Error::Io`.
    pub(crate) fn read_mode(&mut self, key: u8) -> Result<KeyMode, Error> {
        let raw = self.key_param(key, KeyParam::Mode)?;
        KeyMode::from_u16(raw)
            .ok_or_else(|| Error::Io(format!("unknown key mode 0x{raw:04X} for key 0x{key:02X}")))
    }

    /// Reads the parameters of function `adv` on `key`, without the Mode.
    fn read_function(&mut self, key: u8, adv: Advanced) -> Result<AdvancedConfig, Error> {
        Ok(match adv {
            Advanced::None => AdvancedConfig::None,
            Advanced::Macro => AdvancedConfig::Macro,
            Advanced::Dks => {
                let mut bindings = [0u16; 4];
                let mut trps = [0u16; 4];
                for (i, slot) in SLOTS.into_iter().enumerate() {
                    bindings[i] = self.key_param(key, KeyParam::DksBinding(slot))?;
                }
                for (i, slot) in SLOTS.into_iter().enumerate() {
                    trps[i] = self.key_param(key, KeyParam::DksTrps(slot))?;
                }
                let first = self.key_param(key, KeyParam::DksFirst)?;
                AdvancedConfig::Dks(Dks::from_key_params(bindings, trps, first).map_err(bad_reply)?)
            }
            Advanced::Mt => {
                let hold = self.key_param(key, KeyParam::DksBinding(Slot::S1))?;
                let tap = self.key_param(key, KeyParam::DksBinding(Slot::S2))?;
                let units = self.key_param(key, KeyParam::MtDelay)?;
                AdvancedConfig::Mt(Mt::from_key_params(hold, tap, units))
            }
            Advanced::Mpt => {
                let r = self.session.call(&advanced::mpt_get(key))?;
                reply_key_is(key, "MPT", &r)?;
                AdvancedConfig::Mpt(advanced::parse_mpt(&r).map_err(bad_reply)?)
            }
            Advanced::Tgl => {
                let r = self.session.call(&advanced::tgl_get(key))?;
                reply_key_is(key, "TGL", &r)?;
                AdvancedConfig::Tgl(advanced::parse_tgl(&r).map_err(bad_reply)?)
            }
            Advanced::End => {
                let r = self.session.call(&advanced::end_get(key))?;
                reply_key_is(key, "END", &r)?;
                AdvancedConfig::End(advanced::parse_end(&r).map_err(bad_reply)?)
            }
            Advanced::Socd => {
                let r = self.session.call(&advanced::socd_get(key))?;
                let s = advanced::parse_socd(&r).map_err(bad_reply)?;
                // The record names the pair, which must hold this key.
                if s.pos1 != key && s.pos2 != key {
                    return Err(Error::Io(format!(
                        "unexpected SOCD reply for key 0x{key:02X}: pair 0x{:02X}/0x{:02X}",
                        s.pos1, s.pos2
                    )));
                }
                AdvancedConfig::Socd(s)
            }
        })
    }

    /// Reads the advanced function of `key`: its Mode, then that function's
    /// parameters.
    pub fn advanced(&mut self, key: u8) -> Result<AdvancedConfig, Error> {
        let mode = self.read_mode(key)?;
        self.read_function(key, mode.advanced)
    }

    /// The other key of the SOCD pair `key` belongs to, read from the keyboard.
    fn socd_partner(&mut self, key: u8) -> Result<Option<u8>, Error> {
        let r = self.session.call(&advanced::socd_get(key))?;
        let s = advanced::parse_socd(&r).map_err(bad_reply)?;
        Ok(if s.pos1 == key {
            Some(s.pos2)
        } else if s.pos2 == key {
            Some(s.pos1)
        } else {
            None
        })
    }

    /// Binds (or, with `AdvancedConfig::None`, clears) the advanced function of
    /// `key`.
    ///
    /// The config is validated before any I/O. Steps, in order:
    ///
    /// 1. For every key whose Mode will be written, read its Mode; where it is
    ///    SOCD, read the SOCD pair and remember the partner key.
    /// 2. Send the function packet (none for `AdvancedConfig::None`).
    /// 3. Re-read the Mode of each key and change only its advanced nibble
    ///    (touch mode and high byte stay as the keyboard now has them), so a
    ///    firmware side effect of the function packet is never reverted. One
    ///    batched write, verified by `set_key_params`.
    /// 4. Read the function's parameters back and compare them with what was
    ///    sent (delays compare in their 10 ms wire units).
    ///
    /// A SOCD pair is bound to two keys: `key` must be one of the pair and both
    /// keys get their Mode written. Replacing or clearing a key that is in a
    /// SOCD pair (including moving it into a different pair) also clears the
    /// advanced nibble of its old partner, as AULA's delete does, unless the
    /// partner is written anyway; the partner keeps its touch mode and high
    /// byte and is only touched if it is still in SOCD mode. Re-setting the same
    /// pair changes nothing else. `AdvancedConfig::Macro` is rejected; use
    /// `bind_macro`.
    ///
    /// Not atomic: the function packet can succeed and the Mode write then fail,
    /// leaving the new function data stored while the Mode still shows the old
    /// function. The phases fail differently: a Mode mismatch is `Error::Io`
    /// starting "verify failed (mode)", a function mismatch "verify failed
    /// (function)"; transport and device errors are passed through unchanged.
    /// After any error re-read the key with `advanced` rather than trusting the
    /// last state.
    pub fn set_advanced(&mut self, key: u8, cfg: &AdvancedConfig) -> Result<(), Error> {
        let packet = function_packet(key, cfg)?;
        let target = cfg.advanced();
        let mode_keys = match cfg {
            AdvancedConfig::Socd(s) if s.pos1 != s.pos2 => vec![s.pos1, s.pos2],
            _ => vec![key],
        };
        let mut orphans: Vec<u8> = Vec::new();
        for &k in &mode_keys {
            if self.read_mode(k)?.advanced == Advanced::Socd {
                if let Some(p) = self.socd_partner(k)? {
                    if !mode_keys.contains(&p) && !orphans.contains(&p) {
                        orphans.push(p);
                    }
                }
            }
        }
        if let Some(p) = &packet {
            self.session.call(p)?;
        }
        let mut entries = Vec::with_capacity(mode_keys.len() + orphans.len());
        for &k in &mode_keys {
            let mode = self.read_mode(k)?;
            let new = KeyMode {
                advanced: target,
                ..mode
            };
            entries.push((k, KeyParam::Mode, new.to_u16()));
        }
        for &p in &orphans {
            let mode = self.read_mode(p)?;
            if mode.advanced == Advanced::Socd {
                let new = KeyMode {
                    advanced: Advanced::None,
                    ..mode
                };
                entries.push((p, KeyParam::Mode, new.to_u16()));
            }
        }
        self.set_key_params(&entries).map_err(|e| match e {
            Error::Io(s) => Error::Io(match s.strip_prefix("verify failed: ") {
                Some(rest) => format!("verify failed (mode): {rest}"),
                None => s,
            }),
            other => other,
        })?;
        let Some(sent) = packet else {
            return Ok(());
        };
        let back = self.read_function(key, target)?;
        let same = matches!(
            function_packet(key, &back),
            Ok(Some(ref q)) if q.payload == sent.payload
        );
        if same {
            Ok(())
        } else {
            Err(Error::Io(format!(
                "verify failed (function): key 0x{key:02X}: wrote {cfg:?}, read {back:?}"
            )))
        }
    }

    /// Reads the code on `layer` for `key`.
    pub fn layer_code(&mut self, key: u8, layer: Layer) -> Result<u16, Error> {
        self.key_param(key, KeyParam::Layer(layer))
    }

    /// Writes layer codes, then reads each back (see `set_key_params`).
    ///
    /// The Fn key's own Base-layer code must never be overwritten, so any
    /// Base-layer entry for key 0x01 is `Error::InvalidInput` and nothing at all
    /// is written. Code 0x0001 on Base is refused the same way: it is the
    /// transparent "follow Base" code of the Fn layers (hardware finding 16),
    /// not Fn, so on Base it makes a dead key. To make a key act as Fn on
    /// Base use 0xF001 (Fn1 while held), as the factory Fn key does. The Fn
    /// key may be remapped on the Fn layers. At most 14 entries go in each
    /// packet.
    pub fn set_layer_codes(&mut self, entries: &[(u8, Layer, u16)]) -> Result<(), Error> {
        for &(key, layer, code) in entries {
            if layer == Layer::Base && key == FN_KEY {
                return Err(Error::InvalidInput(format!(
                    "refusing to overwrite the Fn key's base-layer code (key 0x{key:02X})"
                )));
            }
            if layer == Layer::Base && code == TRANSPARENT {
                return Err(Error::InvalidInput(format!(
                    "code 0x0001 is transparent, not Fn: refusing it on the base layer \
                     (key 0x{key:02X}; use 0xF001 for Fn1 while held)"
                )));
            }
        }
        let params: Vec<_> = entries
            .iter()
            .map(|&(key, layer, code)| (key, KeyParam::Layer(layer), code))
            .collect();
        self.set_key_params(&params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;
    use openhe_proto::advanced::SocdMode;
    use openhe_proto::frame::FrameError;
    use openhe_proto::model::Model;
    use openhe_proto::units::Micrometers;

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    /// The payload of a written frame (`5C LEN CMD CHK` header stripped).
    fn payload(frame: &[u8; 64]) -> &[u8] {
        &frame[4..4 + usize::from(frame[1])]
    }

    fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        raw
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let [lo, hi] = v.to_le_bytes();
        reply(0x23, &[0, key, layout, lo, hi])
    }

    fn um(v: u16) -> Micrometers {
        Micrometers(v)
    }

    const KEY: u8 = 0x0A;

    fn mt() -> Mt {
        Mt {
            hold: 0x00E0,
            tap: 0x0029,
            delay_ms: 200,
        }
    }

    fn dks() -> Dks {
        Dks {
            bindings: [0x04, 0x05, 0x06, 0x07],
            trps: [0x01, 0x02, 0x04, 0x18],
            first_actuation: um(1400),
            bottom: um(3400),
        }
    }

    fn mpt() -> Mpt {
        Mpt {
            codes: [0x04, 0x05, 0x06],
            depths: [um(1000), um(2000), um(3000)],
        }
    }

    fn socd() -> Socd {
        Socd {
            pos1: 0x04,
            pos2: 0x07,
            key1: 0x04,
            key2: 0x07,
            bind_values: true,
            mode: SocdMode::Key2Wins,
            delay: 50,
        }
    }

    fn written(k: &mut Keyboard<MockTransport>) -> Vec<[u8; 64]> {
        k.session_mut().transport().written().to_vec()
    }

    // ---- reads ----

    #[test]
    fn advanced_none() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0020));
        let mut k = kb(m);
        assert_eq!(k.advanced(KEY), Ok(AdvancedConfig::None));
        assert_eq!(written(&mut k).len(), 1);
    }

    #[test]
    fn advanced_macro_is_reported_not_parsed() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0016));
        assert_eq!(kb(m).advanced(KEY), Ok(AdvancedConfig::Macro));
    }

    #[test]
    fn advanced_unknown_mode_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0019)); // RS (9): not offered
        assert!(matches!(kb(m).advanced(KEY), Err(Error::Io(_))));
    }

    #[test]
    fn advanced_reads_mt_from_key_layouts() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0xAB13));
        m.push_reply(key_reply(KEY, 9, 0x00E0));
        m.push_reply(key_reply(KEY, 10, 0x0029));
        m.push_reply(key_reply(KEY, 19, 20));
        let mut k = kb(m);
        assert_eq!(k.advanced(KEY), Ok(AdvancedConfig::Mt(mt())));
        let w = written(&mut k);
        let layouts: Vec<u8> = w.iter().map(|f| payload(f)[2]).collect();
        assert_eq!(layouts, [8, 9, 10, 19]);
    }

    #[test]
    fn advanced_reads_dks_from_key_layouts() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0011));
        for (i, b) in [4u16, 5, 6, 7].into_iter().enumerate() {
            m.push_reply(key_reply(KEY, 9 + i as u8, b));
        }
        for (i, t) in [1u16, 2, 4, 0x18].into_iter().enumerate() {
            m.push_reply(key_reply(KEY, 13 + i as u8, t));
        }
        m.push_reply(key_reply(KEY, 5, 1400));
        let mut k = kb(m);
        assert_eq!(k.advanced(KEY), Ok(AdvancedConfig::Dks(dks())));
        let layouts: Vec<u8> = written(&mut k).iter().map(|f| payload(f)[2]).collect();
        assert_eq!(layouts, [8, 9, 10, 11, 12, 13, 14, 15, 16, 5]);
    }

    #[test]
    fn advanced_dks_trps_wider_than_a_byte_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0011));
        for i in 0..4u8 {
            m.push_reply(key_reply(KEY, 9 + i, 4));
        }
        m.push_reply(key_reply(KEY, 13, 0x0100));
        for i in 1..4u8 {
            m.push_reply(key_reply(KEY, 13 + i, 0));
        }
        m.push_reply(key_reply(KEY, 5, 1400));
        assert!(matches!(kb(m).advanced(KEY), Err(Error::Io(_))));
    }

    #[test]
    fn advanced_reads_mpt_tgl_end_socd_from_their_commands() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0012));
        m.push_reply(reply(
            0x27,
            &[0, KEY, 4, 0, 5, 0, 6, 0, 0xE8, 0x03, 0xD0, 0x07, 0xB8, 0x0B],
        ));
        m.push_reply(key_reply(KEY, 8, 0x0014));
        m.push_reply(reply(0x25, &[0, KEY, 4, 0, 30]));
        m.push_reply(key_reply(KEY, 8, 0x0015));
        m.push_reply(reply(0x28, &[0, KEY, 5, 0, 0, 0]));
        m.push_reply(key_reply(0x04, 8, 0x0018));
        m.push_reply(reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0]));
        let mut k = kb(m);
        assert_eq!(k.advanced(KEY), Ok(AdvancedConfig::Mpt(mpt())));
        assert_eq!(
            k.advanced(KEY),
            Ok(AdvancedConfig::Tgl(Tgl {
                code: 4,
                delay_ms: 300
            }))
        );
        assert_eq!(k.advanced(KEY), Ok(AdvancedConfig::End(End { code: 5 })));
        assert_eq!(k.advanced(0x04), Ok(AdvancedConfig::Socd(socd())));
        let w = written(&mut k);
        assert_eq!(payload(&w[1]), [0x00, KEY]); // MPT GET
        assert_eq!(payload(&w[3]), [0x00, KEY, 0, 0, 0]); // TGL GET
        assert_eq!(payload(&w[5]), [0x00, KEY, 0, 0]); // END GET
        assert_eq!(payload(&w[7]), [0x00, 0x04]); // SOCD GET
    }

    // ---- writes ----

    /// Queues the replies of a successful single-key write: the current Mode
    /// (checked for SOCD), the function packet's echo, the Mode read again, the
    /// Mode SET echo and its read-back.
    fn script_set(m: &mut MockTransport, func_cmd: u8, before: u16, after: u16) {
        m.push_reply(key_reply(KEY, 8, before));
        if func_cmd != 0 {
            m.push_reply(reply(func_cmd, &[0, KEY]));
        }
        m.push_reply(key_reply(KEY, 8, before));
        m.push_reply(key_reply(KEY, 8, after));
        m.push_reply(key_reply(KEY, 8, after));
    }

    #[test]
    fn set_mt_writes_function_then_mode_then_verifies() {
        let mut m = MockTransport::new();
        // Touch Single, high byte 0xAB, no function yet.
        script_set(&mut m, 0x24, 0xAB10, 0xAB13);
        m.push_reply(key_reply(KEY, 9, 0x00E0));
        m.push_reply(key_reply(KEY, 10, 0x0029));
        m.push_reply(key_reply(KEY, 19, 20));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::Mt(mt())), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 8);
        assert_eq!(payload(&w[0]), [0x00, KEY, 8, 0, 0]); // current Mode
        assert_eq!(w[1][2], 0x24);
        assert_eq!(payload(&w[1]), [0x01, KEY, 0xE0, 0, 0x29, 0, 0x14]);
        assert_eq!(payload(&w[2]), [0x00, KEY, 8, 0, 0]); // Mode re-read after the packet
                                                          // Mode keeps touch = Single and the 0xAB high byte, advanced = MT.
        assert_eq!(payload(&w[3]), [0x01, KEY, 8, 0x13, 0xAB]);
        assert_eq!(payload(&w[4]), [0x00, KEY, 8, 0, 0]); // Mode read-back
        assert_eq!(payload(&w[5]), [0x00, KEY, 9, 0, 0]);
        assert_eq!(payload(&w[6]), [0x00, KEY, 10, 0, 0]);
        assert_eq!(payload(&w[7]), [0x00, KEY, 19, 0, 0]);
    }

    #[test]
    fn mode_is_re_read_after_the_function_packet() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0xAB10)); // before the packet
        m.push_reply(reply(0x24, &[0, KEY]));
        // The firmware changed the touch mode and high byte in the meantime.
        m.push_reply(key_reply(KEY, 8, 0xCD20));
        m.push_reply(key_reply(KEY, 8, 0xCD23));
        m.push_reply(key_reply(KEY, 8, 0xCD23));
        m.push_reply(key_reply(KEY, 9, 0x00E0));
        m.push_reply(key_reply(KEY, 10, 0x0029));
        m.push_reply(key_reply(KEY, 19, 20));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::Mt(mt())), Ok(()));
        let w = written(&mut k);
        // Only the advanced nibble changes, from the Mode as read after the packet.
        assert_eq!(payload(&w[3]), [0x01, KEY, 8, 0x23, 0xCD]);
    }

    #[test]
    fn mode_write_failing_after_the_function_packet_is_reported_as_mode_phase() {
        let mut m = MockTransport::new();
        script_set(&mut m, 0x24, 0x0010, 0x0010); // the Mode did not change
        let mut k = kb(m);
        let e = k.set_advanced(KEY, &AdvancedConfig::Mt(mt())).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed (mode): key 0x0A layout 8")),
            "{e:?}"
        );
        // The function packet went out; no function read-back was attempted.
        let w = written(&mut k);
        assert_eq!(w.len(), 5);
        assert_eq!(w[1][2], 0x24);
    }

    #[test]
    fn mode_set_rejected_after_the_function_packet_passes_the_error_through() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0));
        m.push_reply(reply(0x24, &[0, KEY]));
        m.push_reply(key_reply(KEY, 8, 0));
        let mut fail = [0u8; 64];
        fail[0] = 0x5C;
        fail[2] = 0xFF;
        m.push_reply(fail);
        let mut k = kb(m);
        let e = k.set_advanced(KEY, &AdvancedConfig::Mt(mt())).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        assert_eq!(written(&mut k).len(), 4);
    }

    #[test]
    fn set_mt_verify_mismatch_is_io_error() {
        let mut m = MockTransport::new();
        script_set(&mut m, 0x24, 0x0000, 0x0003);
        m.push_reply(key_reply(KEY, 9, 0x00E0));
        m.push_reply(key_reply(KEY, 10, 0x0029));
        m.push_reply(key_reply(KEY, 19, 25)); // 250 ms: the write did not stick
        let e = kb(m)
            .set_advanced(KEY, &AdvancedConfig::Mt(mt()))
            .unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed (function): key 0x0A")),
            "{e:?}"
        );
    }

    #[test]
    fn set_mt_rounded_delay_still_verifies() {
        // 205 ms goes out as 21 units and reads back as 210 ms.
        let mut m = MockTransport::new();
        script_set(&mut m, 0x24, 0, 3);
        m.push_reply(key_reply(KEY, 9, 0x00E0));
        m.push_reply(key_reply(KEY, 10, 0x0029));
        m.push_reply(key_reply(KEY, 19, 21));
        let cfg = AdvancedConfig::Mt(Mt {
            delay_ms: 205,
            ..mt()
        });
        assert_eq!(kb(m).set_advanced(KEY, &cfg), Ok(()));
    }

    #[test]
    fn set_dks_preserves_touch_and_high_byte() {
        let mut m = MockTransport::new();
        // Rapid Trigger, high byte 0x12.
        script_set(&mut m, 0x26, 0x1220, 0x1221);
        for (i, b) in [4u16, 5, 6, 7].into_iter().enumerate() {
            m.push_reply(key_reply(KEY, 9 + i as u8, b));
        }
        for (i, t) in [1u16, 2, 4, 0x18].into_iter().enumerate() {
            m.push_reply(key_reply(KEY, 13 + i as u8, t));
        }
        m.push_reply(key_reply(KEY, 5, 1400));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::Dks(dks())), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 5 + 9);
        assert_eq!(w[1][2], 0x26);
        assert_eq!(w[1][1], 18);
        assert_eq!(payload(&w[3]), [0x01, KEY, 8, 0x21, 0x12]);
    }

    #[test]
    fn set_mpt_tgl_end_succeed() {
        let mut m = MockTransport::new();
        script_set(&mut m, 0x27, 0, 2);
        m.push_reply(reply(
            0x27,
            &[0, KEY, 4, 0, 5, 0, 6, 0, 0xE8, 3, 0xD0, 7, 0xB8, 0xB],
        ));
        script_set(&mut m, 0x25, 0x0010, 0x0014);
        m.push_reply(reply(0x25, &[0, KEY, 4, 0, 30]));
        script_set(&mut m, 0x28, 0x0020, 0x0025);
        m.push_reply(reply(0x28, &[0, KEY, 5, 0, 0, 0]));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::Mpt(mpt())), Ok(()));
        let tgl = Tgl {
            code: 4,
            delay_ms: 300,
        };
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::Tgl(tgl)), Ok(()));
        assert_eq!(
            k.set_advanced(KEY, &AdvancedConfig::End(End { code: 5 })),
            Ok(())
        );
        let w = written(&mut k);
        // MPT: GET Mode, SET, GET Mode, Mode SET, Mode GET, MPT GET.
        assert_eq!(payload(&w[3]), [0x01, KEY, 8, 0x02, 0x00]);
        // TGL keeps touch Single (0x10) and writes TGL (4).
        assert_eq!(payload(&w[9]), [0x01, KEY, 8, 0x14, 0x00]);
        // END keeps touch RT (0x20) and writes END (5).
        assert_eq!(payload(&w[15]), [0x01, KEY, 8, 0x25, 0x00]);
    }

    /// Scan finding 3: the MPT, TGL and END replies carry the key at R[1].
    /// A reply for another key (a stray or late one) must not pass the
    /// set_advanced verify, which rebuilds the packet with the requested
    /// key, nor be read as this key's function.
    #[test]
    fn a_function_reply_for_another_key_never_verifies() {
        const OTHER: u8 = 0x0B;
        let mut m = MockTransport::new();
        script_set(&mut m, 0x27, 0, 2);
        m.push_reply(reply(
            0x27,
            &[0, OTHER, 4, 0, 5, 0, 6, 0, 0xE8, 3, 0xD0, 7, 0xB8, 0xB],
        ));
        script_set(&mut m, 0x25, 0x0010, 0x0014);
        m.push_reply(reply(0x25, &[0, OTHER, 4, 0, 30]));
        script_set(&mut m, 0x28, 0x0020, 0x0025);
        m.push_reply(reply(0x28, &[0, OTHER, 5, 0, 0, 0]));
        let mut k = kb(m);
        let tgl = Tgl {
            code: 4,
            delay_ms: 300,
        };
        for cfg in [
            AdvancedConfig::Mpt(mpt()),
            AdvancedConfig::Tgl(tgl),
            AdvancedConfig::End(End { code: 5 }),
        ] {
            let e = k.set_advanced(KEY, &cfg).unwrap_err();
            assert!(
                matches!(&e, Error::Io(s) if s.contains("key 0x0B")),
                "{cfg:?}: {e:?}"
            );
        }
        // Plain reads check the key too; a SOCD record must name the key.
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0014));
        m.push_reply(reply(0x25, &[0, OTHER, 4, 0, 30]));
        m.push_reply(key_reply(KEY, 8, 0x0018));
        m.push_reply(reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0]));
        let mut k = kb(m);
        assert!(matches!(k.advanced(KEY), Err(Error::Io(_))));
        assert!(matches!(k.advanced(KEY), Err(Error::Io(_))));
    }

    #[test]
    fn set_socd_writes_mode_on_both_keys_keeping_each_ones_bits() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0000)); // current Mode of pos1
        m.push_reply(key_reply(0x07, 8, 0x1200)); // pos2: Global, high byte 0x12
        m.push_reply(reply(0x2C, &[0, 4, 7]));
        m.push_reply(key_reply(0x04, 8, 0x0000)); // both re-read after the packet
        m.push_reply(key_reply(0x07, 8, 0x1200));
        m.push_reply(key_reply(0x04, 8, 0x0008)); // Mode SET echo (first entry)
        m.push_reply(key_reply(0x04, 8, 0x0008)); // read-backs
        m.push_reply(key_reply(0x07, 8, 0x1208));
        m.push_reply(reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0]));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(0x07, &AdvancedConfig::Socd(socd())), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 9);
        assert_eq!(w[2][2], 0x2C);
        assert_eq!(payload(&w[2]), [0x01, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0]);
        assert_eq!(
            payload(&w[5]),
            [0x01, 0x04, 8, 0x08, 0x00, 0x07, 8, 0x08, 0x12]
        );
        assert_eq!(payload(&w[8]), [0x00, 0x07]); // SOCD GET by the key given
    }

    /// A SOCD pair (4, 7); `socd_get` reports it from either key.
    fn socd_pair_reply() -> [u8; 64] {
        reply(0x2C, &[0, 4, 7, 4, 0, 7, 0, 1, 2, 50, 0])
    }

    #[test]
    fn clearing_one_socd_key_also_clears_its_partner() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0008)); // key 4 is SOCD
        m.push_reply(socd_pair_reply()); // its partner is 7
        m.push_reply(key_reply(0x04, 8, 0x0008)); // re-read key 4
        m.push_reply(key_reply(0x07, 8, 0x1218)); // re-read partner: Single, SOCD, high 0x12
        m.push_reply(key_reply(0x04, 8, 0x0000)); // Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0000)); // read-backs
        m.push_reply(key_reply(0x07, 8, 0x1210));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(0x04, &AdvancedConfig::None), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 7);
        assert_eq!(payload(&w[1]), [0x00, 0x04]); // SOCD GET
                                                  // The partner keeps its touch mode (0x10 nibble of 0x18) and high byte.
        assert_eq!(
            payload(&w[4]),
            [0x01, 0x04, 8, 0x00, 0x00, 0x07, 8, 0x10, 0x12]
        );
    }

    #[test]
    fn replacing_socd_with_mt_clears_the_partner() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(socd_pair_reply());
        m.push_reply(reply(0x24, &[0, 0x04])); // MT SET
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(key_reply(0x07, 8, 0x0008));
        m.push_reply(key_reply(0x04, 8, 0x0003)); // Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0003)); // read-backs
        m.push_reply(key_reply(0x07, 8, 0x0000));
        m.push_reply(key_reply(0x04, 9, 0x00E0)); // function read-back
        m.push_reply(key_reply(0x04, 10, 0x0029));
        m.push_reply(key_reply(0x04, 19, 20));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(0x04, &AdvancedConfig::Mt(mt())), Ok(()));
        let w = written(&mut k);
        assert_eq!(
            payload(&w[5]),
            [0x01, 0x04, 8, 0x03, 0x00, 0x07, 8, 0x00, 0x00]
        );
    }

    #[test]
    fn moving_a_key_into_a_new_socd_pair_clears_its_old_partner() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0008)); // key 4 is in pair (4, 7)
        m.push_reply(socd_pair_reply());
        m.push_reply(key_reply(0x09, 8, 0x0000)); // key 9 is free
        m.push_reply(reply(0x2C, &[0, 4, 9])); // SOCD SET
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(key_reply(0x09, 8, 0x0000));
        m.push_reply(key_reply(0x07, 8, 0x0008)); // old partner re-read
        m.push_reply(key_reply(0x04, 8, 0x0008)); // Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0008)); // read-backs
        m.push_reply(key_reply(0x09, 8, 0x0008));
        m.push_reply(key_reply(0x07, 8, 0x0000));
        m.push_reply(reply(0x2C, &[0, 4, 9, 4, 0, 9, 0, 1, 2, 50, 0]));
        let new_pair = Socd {
            pos2: 0x09,
            key2: 0x09,
            ..socd()
        };
        let mut k = kb(m);
        assert_eq!(
            k.set_advanced(0x04, &AdvancedConfig::Socd(new_pair)),
            Ok(())
        );
        let w = written(&mut k);
        assert_eq!(
            payload(&w[7]),
            [0x01, 0x04, 8, 0x08, 0x00, 0x09, 8, 0x08, 0x00, 0x07, 8, 0x00, 0x00]
        );
    }

    #[test]
    fn re_setting_the_same_socd_pair_writes_only_the_pair() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(socd_pair_reply());
        m.push_reply(key_reply(0x07, 8, 0x0008));
        m.push_reply(socd_pair_reply());
        m.push_reply(reply(0x2C, &[0, 4, 7])); // SOCD SET
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(key_reply(0x07, 8, 0x0008));
        m.push_reply(key_reply(0x04, 8, 0x0008)); // Mode SET echo
        m.push_reply(key_reply(0x04, 8, 0x0008)); // read-backs
        m.push_reply(key_reply(0x07, 8, 0x0008));
        m.push_reply(socd_pair_reply()); // function read-back
        let mut k = kb(m);
        assert_eq!(k.set_advanced(0x04, &AdvancedConfig::Socd(socd())), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 11);
        assert_eq!(
            payload(&w[7]),
            [0x01, 0x04, 8, 0x08, 0x00, 0x07, 8, 0x08, 0x00]
        );
    }

    #[test]
    fn a_partner_no_longer_in_socd_mode_is_left_alone() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(socd_pair_reply());
        m.push_reply(key_reply(0x04, 8, 0x0008));
        m.push_reply(key_reply(0x07, 8, 0x1200)); // already cleared elsewhere
        m.push_reply(key_reply(0x04, 8, 0x0000));
        m.push_reply(key_reply(0x04, 8, 0x0000));
        let mut k = kb(m);
        assert_eq!(k.set_advanced(0x04, &AdvancedConfig::None), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 6);
        assert_eq!(payload(&w[4]), [0x01, 0x04, 8, 0x00, 0x00]);
    }

    #[test]
    fn set_none_clears_function_keeping_touch_and_high_byte() {
        let mut m = MockTransport::new();
        script_set(&mut m, 0, 0xAB23, 0xAB20);
        let mut k = kb(m);
        assert_eq!(k.set_advanced(KEY, &AdvancedConfig::None), Ok(()));
        let w = written(&mut k);
        assert_eq!(
            w.len(),
            4,
            "two mode reads, mode write, mode read-back only"
        );
        assert_eq!(payload(&w[2]), [0x01, KEY, 8, 0x20, 0xAB]);
    }

    #[test]
    fn mode_verify_mismatch_is_io_error() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0023));
        m.push_reply(key_reply(KEY, 8, 0x0023));
        m.push_reply(key_reply(KEY, 8, 0x0020));
        m.push_reply(key_reply(KEY, 8, 0x0023)); // the clear did not stick
        let e = kb(m).set_advanced(KEY, &AdvancedConfig::None).unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
    }

    #[test]
    fn invalid_config_is_rejected_before_any_io() {
        let mut k = kb(MockTransport::new());
        let bad_mt = Mt {
            delay_ms: 0,
            ..mt()
        };
        let bad_mpt = Mpt {
            depths: [um(2000), um(1000), um(3000)],
            ..mpt()
        };
        let bad_dks = Dks {
            first_actuation: um(50),
            ..dks()
        };
        let bad_socd = Socd {
            delay: 51,
            ..socd()
        };
        let bad_tgl = Tgl {
            code: 4,
            delay_ms: 1001,
        };
        for cfg in [
            AdvancedConfig::Mt(bad_mt),
            AdvancedConfig::Mpt(bad_mpt),
            AdvancedConfig::Dks(bad_dks),
            AdvancedConfig::Socd(bad_socd),
            AdvancedConfig::Tgl(bad_tgl),
        ] {
            let e = k.set_advanced(0x04, &cfg).unwrap_err();
            assert!(matches!(e, Error::InvalidInput(_)), "{cfg:?}: {e:?}");
        }
        assert!(written(&mut k).is_empty());
    }

    #[test]
    fn macro_cannot_be_set_here() {
        let mut k = kb(MockTransport::new());
        let e = k.set_advanced(KEY, &AdvancedConfig::Macro).unwrap_err();
        assert!(matches!(e, Error::InvalidInput(_)), "{e:?}");
        assert!(written(&mut k).is_empty());
    }

    #[test]
    fn socd_key_must_be_one_of_the_pair() {
        let mut k = kb(MockTransport::new());
        let e = k
            .set_advanced(0x20, &AdvancedConfig::Socd(socd()))
            .unwrap_err();
        assert!(matches!(e, Error::InvalidInput(_)), "{e:?}");
        assert!(written(&mut k).is_empty());
    }

    #[test]
    fn failed_function_packet_leaves_mode_unwritten() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0000));
        let mut fail = [0u8; 64];
        fail[0] = 0x5C;
        fail[2] = 0xFF;
        m.push_reply(fail);
        let mut k = kb(m);
        let e = k.set_advanced(KEY, &AdvancedConfig::Mt(mt())).unwrap_err();
        assert_eq!(e, Error::Frame(FrameError::DeviceFail));
        assert_eq!(written(&mut k).len(), 2);
    }

    #[test]
    fn unknown_current_mode_blocks_the_write() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 8, 0x0019));
        let mut k = kb(m);
        let e = k.set_advanced(KEY, &AdvancedConfig::Mt(mt())).unwrap_err();
        assert!(matches!(e, Error::Io(_)), "{e:?}");
        assert_eq!(written(&mut k).len(), 1);
    }

    // ---- layers ----

    #[test]
    fn layer_code_reads_the_layer_layout() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(KEY, 2, 0x0050));
        let mut k = kb(m);
        assert_eq!(k.layer_code(KEY, Layer::Fn2), Ok(0x0050));
        assert_eq!(payload(&written(&mut k)[0]), [0x00, KEY, 2, 0, 0]);
    }

    #[test]
    fn set_layer_codes_writes_and_verifies() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0B)); // SET echo
        m.push_reply(key_reply(0x0A, 0, 0x0B));
        m.push_reply(key_reply(0x0A, 1, 0x4F));
        let mut k = kb(m);
        assert_eq!(
            k.set_layer_codes(&[(0x0A, Layer::Base, 0x0B), (0x0A, Layer::Fn1, 0x4F)]),
            Ok(())
        );
        let w = written(&mut k);
        assert_eq!(payload(&w[0]), [0x01, 0x0A, 0, 0x0B, 0, 0x0A, 1, 0x4F, 0]);
    }

    #[test]
    fn fn_key_is_never_written_on_the_base_layer() {
        let mut k = kb(MockTransport::new());
        // Overwriting the Fn key's own code, or resetting it to itself.
        for entry in [(0x01, Layer::Base, 0x04), (0x01, Layer::Base, 0x01)] {
            let e = k
                .set_layer_codes(&[(0x0B, Layer::Base, 0x0B), entry])
                .unwrap_err();
            assert!(matches!(e, Error::InvalidInput(_)), "{entry:?}: {e:?}");
        }
        assert!(
            written(&mut k).is_empty(),
            "nothing is written, not even the valid entry"
        );
    }

    #[test]
    fn fn_may_be_assigned_to_other_keys_on_the_base_layer() {
        // Caps (0x39) -> Fn1 while held (0xF001), like the factory Fn key.
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x39, 0, 0xF001));
        m.push_reply(key_reply(0x39, 0, 0xF001));
        let mut k = kb(m);
        assert_eq!(k.set_layer_codes(&[(0x39, Layer::Base, 0xF001)]), Ok(()));
        assert_eq!(payload(&written(&mut k)[0]), [0x01, 0x39, 0, 0x01, 0xF0]);
    }

    #[test]
    fn transparent_0x0001_is_refused_on_base_only() {
        // 0x0001 is "follow Base" (finding 16): a dead key on Base.
        let mut k = kb(MockTransport::new());
        let e = k
            .set_layer_codes(&[(0x04, Layer::Fn1, 0x05), (0x39, Layer::Base, 0x0001)])
            .unwrap_err();
        assert!(
            matches!(&e, Error::InvalidInput(s) if s.contains("0x0001")),
            "{e:?}"
        );
        assert!(written(&mut k).is_empty(), "nothing written");
        // On a Fn layer it is the normal transparent code.
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x39, 2, 0x0001));
        m.push_reply(key_reply(0x39, 2, 0x0001));
        assert_eq!(kb(m).set_layer_codes(&[(0x39, Layer::Fn2, 0x0001)]), Ok(()));
    }

    #[test]
    fn fn_key_may_be_remapped_on_the_fn_layers() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x01, 1, 0x04));
        m.push_reply(key_reply(0x01, 1, 0x04));
        assert_eq!(kb(m).set_layer_codes(&[(0x01, Layer::Fn1, 0x04)]), Ok(()));
    }

    #[test]
    fn set_layer_codes_splits_past_fourteen_entries() {
        let entries: Vec<_> = (0..15u8)
            .map(|i| (0x10 + i, Layer::Fn1, 0x0100 + u16::from(i)))
            .collect();
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x10, 1, 0x0100)); // SET echo, packet 1
        m.push_reply(key_reply(0x1E, 1, 0x010E)); // SET echo, packet 2
        for &(key, _, v) in &entries {
            m.push_reply(key_reply(key, 1, v));
        }
        let mut k = kb(m);
        assert_eq!(k.set_layer_codes(&entries), Ok(()));
        let w = written(&mut k);
        assert_eq!(w.len(), 2 + 15);
        assert_eq!(payload(&w[0]).len(), 1 + 14 * 4);
        assert_eq!(payload(&w[1]), [0x01, 0x1E, 1, 0x0E, 0x01]);
    }

    #[test]
    fn set_layer_codes_verify_mismatch_and_empty() {
        let mut m = MockTransport::new();
        m.push_reply(key_reply(0x0A, 0, 0x0B));
        m.push_reply(key_reply(0x0A, 0, 0x0C)); // read-back differs
        let e = kb(m)
            .set_layer_codes(&[(0x0A, Layer::Base, 0x0B)])
            .unwrap_err();
        assert!(
            matches!(&e, Error::Io(s) if s.starts_with("verify failed")),
            "{e:?}"
        );
        let mut k = kb(MockTransport::new());
        assert_eq!(k.set_layer_codes(&[]), Ok(()));
        assert!(written(&mut k).is_empty());
    }
}

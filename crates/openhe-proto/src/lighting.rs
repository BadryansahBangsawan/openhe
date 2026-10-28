//! CMD 0x18 PRGB (main light) and CMD 0x2A KRGB (per-key custom colour).
//!
//! PRGB SET uses the "1.0.9" layout of PROTOCOL 3.8 (LEN 44, always 7 colours
//! in B, G, R, 0xFF order); PRGB GET is a fixed LEN-15 packet (see `get`). KRGB sets one key per packet in R, G, B order and is
//! made permanent by a commit packet (`[01, FE, FE, FE, FE]`).

use crate::frame::{Cmd, Request, Response};
use thiserror::Error;

/// Colour slots in a PRGB packet. Always sent in full.
pub const COLOR_SLOTS: usize = 7;

/// Light sleep timeouts the firmware accepts, in minutes (0 = never).
pub const SLEEP_MINUTES: [u8; 13] = [0, 1, 2, 3, 5, 10, 15, 20, 25, 30, 45, 60, 120];

/// Wire mode of the static light.
pub const MODE_STATIC: u8 = 0;
/// Wire mode of per-key custom lighting.
pub const MODE_CUSTOM: u8 = 21;

/// KRGB key value that marks a commit packet; never a real key.
pub const KRGB_COMMIT_KEY: u8 = 0xFE;

/// Dynamic effects: `(wire mode, English name)`, wire modes 1..=20.
pub const EFFECTS: [(u8, &str); 20] = [
    (1, "Ripple"),
    (2, "Tide"),
    (3, "Ripple Light"),
    (4, "Rotating Storm"),
    (5, "Lucky Rainbow"),
    (6, "Shining Rainbow"),
    (7, "Glitter"),
    (8, "Moving Panels"),
    (9, "Waveform"),
    (10, "Shadow Shift"),
    (11, "Flowing Water"),
    (12, "Sine Wave"),
    (13, "Blooming"),
    (14, "Mosaic"),
    (15, "Rainfall"),
    (16, "Pulse"),
    (17, "Snow Trail"),
    (18, "Plum Blossom"),
    (19, "Laser"),
    (20, "Water Ripple"),
];

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum LightingError {
    #[error("brightness {0} out of range 0..=4")]
    Brightness(u8),
    #[error("mode {0} out of range 0..=21")]
    Mode(u8),
    #[error("speed {0} out of range 0..=4")]
    Speed(u8),
    #[error("sleep timeout {0} min is not one of 0, 1, 2, 3, 5, 10, 15, 20, 25, 30, 45, 60, 120")]
    Sleep(u8),
    #[error("static colour index {0} out of range 0..=7")]
    StaticColor(u8),
    #[error("dynamic colour index {0} out of range 0..=7")]
    DynamicColor(u8),
    #[error("key 0xFE is the KRGB commit marker, not a key")]
    ReservedKey,
    #[error("reply is not a {0} reply")]
    WrongReply(&'static str),
}

/// A colour in natural R, G, B order (the PRGB packet reorders it on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The main-light state of PRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lighting {
    pub on: bool,
    pub reverse: bool,
    /// 0..=4 (UI levels 1..=5).
    pub brightness: u8,
    /// 0 static, 1..=20 effects (see `EFFECTS`), 21 custom per-key.
    pub mode: u8,
    /// 0..=4. Sent as 0 for static (0) and custom (21), whatever is set here.
    pub speed: u8,
    /// One of `SLEEP_MINUTES`.
    pub sleep_minutes: u8,
    /// 0..=6 index into `colors`, 7 = colourful.
    pub static_color: u8,
    /// 0 = colourful, n = `colors[n - 1]`; up to 7.
    pub dynamic_color: u8,
    pub colors: [Rgb; COLOR_SLOTS],
    /// The flag bits other than on (bit0) and reverse (bit1), kept as the
    /// keyboard reported them so a SET of a read value writes them back
    /// unchanged (a real keyboard reported 0xC1: bits 6 and 7 set, bit4
    /// clear). [`DEFAULT_EXTRA_FLAGS`] for a state built from scratch.
    pub extra_flags: u8,
}

/// `extra_flags` of a lighting state that was not read from the keyboard:
/// bit4 ("superResponse"), which the official app always sets.
pub const DEFAULT_EXTRA_FLAGS: u8 = FLAG_ALWAYS;

impl Lighting {
    /// Checks every field against the ranges the firmware accepts.
    pub fn validate(&self) -> Result<(), LightingError> {
        if self.brightness > 4 {
            return Err(LightingError::Brightness(self.brightness));
        }
        if self.mode > MODE_CUSTOM {
            return Err(LightingError::Mode(self.mode));
        }
        if self.speed > 4 {
            return Err(LightingError::Speed(self.speed));
        }
        if !SLEEP_MINUTES.contains(&self.sleep_minutes) {
            return Err(LightingError::Sleep(self.sleep_minutes));
        }
        if self.static_color > 7 {
            return Err(LightingError::StaticColor(self.static_color));
        }
        if self.dynamic_color > 7 {
            return Err(LightingError::DynamicColor(self.dynamic_color));
        }
        Ok(())
    }

    /// The speed that goes on the wire: 0 for static and custom modes.
    pub fn wire_speed(&self) -> u8 {
        if self.mode == MODE_STATIC || self.mode == MODE_CUSTOM {
            0
        } else {
            self.speed
        }
    }
}

const FLAG_ON: u8 = 1;
const FLAG_REVERSE: u8 = 2;
/// "superResponse": the official app always sets it.
const FLAG_ALWAYS: u8 = 16;

/// Offsets inside the 44-byte PRGB payload / reply body.
const OFF_COLORS: usize = 5;
const OFF_FLAGS: usize = 37;
const OFF_BRIGHTNESS: usize = 38;
const OFF_MODE: usize = 39;
const OFF_SPEED: usize = 40;
const OFF_SLEEP: usize = 41;
const OFF_STATIC: usize = 42;
const OFF_DYNAMIC: usize = 43;
const PRGB_LEN: usize = 44;

/// PRGB SET (flag 01), LEN 44. Fails on any out-of-range field. The flags
/// byte is `extra_flags` with on (bit0) and reverse (bit1) set from `on` and
/// `reverse`.
pub fn set(l: &Lighting) -> Result<Request, LightingError> {
    l.validate()?;
    let mut p = vec![0u8; PRGB_LEN];
    p[0] = 0x01;
    for (i, c) in l.colors.iter().enumerate() {
        let o = OFF_COLORS + i * 4;
        p[o..o + 4].copy_from_slice(&[c.b, c.g, c.r, 0xFF]);
    }
    p[OFF_FLAGS] = (l.extra_flags & !(FLAG_ON | FLAG_REVERSE))
        | if l.on { FLAG_ON } else { 0 }
        | if l.reverse { FLAG_REVERSE } else { 0 };
    p[OFF_BRIGHTNESS] = l.brightness;
    p[OFF_MODE] = l.mode;
    p[OFF_SPEED] = l.wire_speed();
    p[OFF_SLEEP] = l.sleep_minutes;
    p[OFF_STATIC] = l.static_color;
    p[OFF_DYNAMIC] = l.dynamic_color;
    Ok(Request {
        cmd: Cmd::Prgb,
        payload: p,
    })
}

/// Length of the only PRGB GET the keyboard accepts.
const PRGB_GET_LEN: usize = 15;

/// PRGB GET, LEN 15: flag 00 followed by 14 zero bytes.
///
/// This is what AULA HUB sends for its first read (`getLighting` with an
/// empty state: pad(4), no colours, pad(4), flags, brightness, mode, speed,
/// sleep, static colour). The keyboard (FW App V1.1.3) rejects a LEN-44 GET
/// with a 0xFF DeviceFail reply, and right after that rejection it stopped
/// sensing keys until it was replugged, so this is the only GET OpenHE can
/// build. The reply still carries the full 44-byte layout (see `parse`).
pub fn get() -> Request {
    Request {
        cmd: Cmd::Prgb,
        payload: vec![0u8; PRGB_GET_LEN],
    }
}

/// Parses a PRGB reply (also the unsolicited 0x98 report) with the SET offsets.
///
/// Every field is checked like `Lighting::validate`: a brightness or speed
/// above 4, a mode above 21, a sleep timeout that is not one of
/// `SLEEP_MINUTES` or a colour index above 7 is an error (the same
/// `LightingError` as validation), never clamped, so a value the keyboard
/// should not report is never written back as something else. Flag bits
/// other than bit0 (on) and bit1 (reverse) are not checked; they are kept
/// in `extra_flags` (a real reply had flags 0xC1: bits 6 and 7 set, bit4
/// clear).
pub fn parse(r: &Response) -> Result<Lighting, LightingError> {
    if r.cmd != Cmd::Prgb as u8 {
        return Err(LightingError::WrongReply("PRGB"));
    }
    let b = r.body();
    let mut colors = [Rgb::default(); COLOR_SLOTS];
    for (i, c) in colors.iter_mut().enumerate() {
        let o = OFF_COLORS + i * 4;
        *c = Rgb {
            r: b[o + 2],
            g: b[o + 1],
            b: b[o],
        };
    }
    let flags = b[OFF_FLAGS];
    let l = Lighting {
        on: flags & FLAG_ON != 0,
        reverse: flags & FLAG_REVERSE != 0,
        brightness: b[OFF_BRIGHTNESS],
        mode: b[OFF_MODE],
        speed: b[OFF_SPEED],
        sleep_minutes: b[OFF_SLEEP],
        static_color: b[OFF_STATIC],
        dynamic_color: b[OFF_DYNAMIC],
        colors,
        extra_flags: flags & !(FLAG_ON | FLAG_REVERSE),
    };
    l.validate()?;
    Ok(l)
}

/// KRGB SET `[01, key, R, G, B]`.
pub fn krgb_set(key: u8, c: Rgb) -> Result<Request, LightingError> {
    if key == KRGB_COMMIT_KEY {
        return Err(LightingError::ReservedKey);
    }
    Ok(Request {
        cmd: Cmd::Krgb,
        payload: vec![0x01, key, c.r, c.g, c.b],
    })
}

/// KRGB GET `[00, key]`.
pub fn krgb_get(key: u8) -> Request {
    Request {
        cmd: Cmd::Krgb,
        payload: vec![0x00, key],
    }
}

/// KRGB commit `[01, FE, FE, FE, FE]`, sent after every batch of colours.
pub fn krgb_commit() -> Request {
    Request {
        cmd: Cmd::Krgb,
        payload: vec![0x01, KRGB_COMMIT_KEY, 0xFE, 0xFE, 0xFE],
    }
}

/// `(key, colour)` from a KRGB reply `[st, key, R, G, B]`.
pub fn krgb_parse(r: &Response) -> Result<(u8, Rgb), LightingError> {
    if r.cmd != Cmd::Krgb as u8 {
        return Err(LightingError::WrongReply("KRGB"));
    }
    let b = r.body();
    Ok((
        b[1],
        Rgb {
            r: b[2],
            g: b[3],
            b: b[4],
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::REPORT_LEN;

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
            extra_flags: DEFAULT_EXTRA_FLAGS,
        }
    }

    fn frame(head: &[u8]) -> [u8; REPORT_LEN] {
        let mut out = [0u8; REPORT_LEN];
        out[..head.len()].copy_from_slice(head);
        out
    }

    /// A reply carrying `body` after the 4-byte header, with the 0x80 bit set.
    fn reply(cmd: u8, body: &[u8]) -> Response {
        let mut raw = [0u8; REPORT_LEN];
        raw[0] = 0x5C;
        raw[2] = cmd | 0x80;
        raw[4..4 + body.len()].copy_from_slice(body);
        Response::decode(&raw).unwrap()
    }

    #[test]
    fn effects_cover_wire_modes_1_to_20() {
        for (i, &(mode, name)) in EFFECTS.iter().enumerate() {
            assert_eq!(usize::from(mode), i + 1);
            assert!(!name.is_empty());
        }
        assert_eq!(EFFECTS[0], (1, "Ripple"));
        assert_eq!(EFFECTS[2], (3, "Ripple Light"));
        assert_eq!(EFFECTS[19], (20, "Water Ripple"));
    }

    // Checksum: 0x35 + 0x5C + LEN 0x2C + CMD 0x18 + payload[43] (dynamic colour 3)
    // = 0x91 + 0x2C + 0x18 + 0x03 = 0xD8.
    #[test]
    fn set_golden_bytes() {
        let out = set(&sample()).unwrap().encode().unwrap();
        let want = frame(&[
            0x5C, 0x2C, 0x18, 0xD8, // header, LEN 44
            0x01, // flag: set
            0x00, 0x00, 0x00, 0x00, // pad
            0x00, 0x00, 0xFF, 0xFF, // red   as B G R FF
            0x00, 0xFF, 0x00, 0xFF, // green
            0xFF, 0x00, 0x00, 0xFF, // blue
            0x03, 0x02, 0x01, 0xFF, // (1, 2, 3)
            0x30, 0x20, 0x10, 0xFF, // (0x10, 0x20, 0x30)
            0xCC, 0xBB, 0xAA, 0xFF, // (0xAA, 0xBB, 0xCC)
            0x00, 0x00, 0x00, 0xFF, // black
            0x00, 0x00, 0x00, 0x00, // pad
            0x11, // flags: on (bit0) + always-1 (bit4)
            0x03, // brightness
            0x01, // mode
            0x02, // speed
            0x0A, // sleep minutes
            0x07, // static colour
            0x03, // dynamic colour
        ]);
        assert_eq!(out, want);
    }

    // Checksum: 0x91 + 0x2C + 0x18 + 0 = 0xD5.
    #[test]
    fn static_mode_forces_speed_zero_and_reverse_sets_bit1() {
        let mut l = sample();
        l.mode = MODE_STATIC;
        l.speed = 4;
        l.reverse = true;
        l.dynamic_color = 0;
        let out = set(&l).unwrap().encode().unwrap();
        assert_eq!(out[3], 0xD5);
        assert_eq!(out[4 + 37], 0x13); // on + reverse + bit4
        assert_eq!(out[4 + 39], 0);
        assert_eq!(out[4 + 40], 0, "speed forced to 0");
        assert_eq!(l.wire_speed(), 0);
    }

    #[test]
    fn custom_mode_forces_speed_zero() {
        let mut l = sample();
        l.mode = MODE_CUSTOM;
        l.speed = 3;
        let out = set(&l).unwrap().encode().unwrap();
        assert_eq!(out[4 + 39], 21);
        assert_eq!(out[4 + 40], 0);
    }

    #[test]
    fn off_clears_bit0_but_keeps_bit4() {
        let mut l = sample();
        l.on = false;
        let out = set(&l).unwrap().encode().unwrap();
        assert_eq!(out[4 + 37], 0x10);
    }

    // The keyboard rejects a LEN-44 GET (reply 0xFF, DeviceFail; hardware
    // finding 14). AULA HUB's first GET is flag 00 + 14 zero bytes:
    // pad(4), no colours, pad(4), flags, brightness, mode, speed, sleep,
    // static colour. Checksum: 0x91 + 0x0F + 0x18 + 0 = 0xB8.
    #[test]
    fn get_is_the_15_byte_form_the_keyboard_accepts() {
        let out = get().encode().unwrap();
        assert_eq!(
            out,
            frame(&[
                0x5C, 0x0F, 0x18, 0xB8, // header, LEN 15
                0x00, // flag: get
                0x00, 0x00, 0x00, 0x00, // pad
                0x00, 0x00, 0x00, 0x00, // pad (no colours)
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // flags .. static colour
            ])
        );
        assert_eq!(get().payload.len(), PRGB_GET_LEN);
    }

    #[test]
    fn parse_round_trips_set_payload() {
        let l = sample();
        let body = set(&l).unwrap().payload;
        let parsed = parse(&reply(0x18, &body)).unwrap();
        assert_eq!(parsed, l);
    }

    #[test]
    fn parse_round_trips_reverse_off_and_custom() {
        let mut l = sample();
        l.on = false;
        l.reverse = true;
        l.mode = MODE_CUSTOM;
        l.speed = 0;
        let body = set(&l).unwrap().payload;
        assert_eq!(parse(&reply(0x18, &body)).unwrap(), l);
    }

    #[test]
    fn parse_rejects_out_of_range_values() {
        // A value the keyboard should never report is an error, not
        // something silently clamped that a later write would send back.
        let cases: [(usize, u8, LightingError); 7] = [
            (38, 5, LightingError::Brightness(5)),
            (39, 22, LightingError::Mode(22)),
            (39, 99, LightingError::Mode(99)),
            (40, 5, LightingError::Speed(5)),
            (41, 4, LightingError::Sleep(4)),
            (42, 8, LightingError::StaticColor(8)),
            (43, 8, LightingError::DynamicColor(8)),
        ];
        for (at, v, want) in cases {
            let mut body = set(&sample()).unwrap().payload;
            body[at] = v;
            assert_eq!(parse(&reply(0x18, &body)), Err(want), "byte {at} = {v}");
        }
    }

    /// Flags the keyboard reported (0xC1: bits 6 and 7 set, bit4 clear) go
    /// back unchanged, so a read value can be restored exactly.
    #[test]
    fn unknown_flag_bits_survive_a_parse_and_set() {
        for flags in [0xC1u8, 0xC3, 0xC0, 0x10, 0x00, 0xFF] {
            let mut body = set(&sample()).unwrap().payload;
            body[37] = flags;
            let read = parse(&reply(0x18, &body)).unwrap();
            let again = set(&read).unwrap().payload;
            assert_eq!(again[1..], body[1..], "flags {flags:#04X}");
        }
    }

    #[test]
    fn parse_accepts_the_range_edges() {
        let mut l = sample();
        l.brightness = 4;
        l.mode = 20;
        l.speed = 4;
        l.sleep_minutes = 120;
        l.static_color = 7;
        l.dynamic_color = 7;
        let body = set(&l).unwrap().payload;
        assert_eq!(parse(&reply(0x18, &body)), Ok(l));
    }

    #[test]
    fn parse_accepts_the_unsolicited_0x98_report() {
        let body = set(&sample()).unwrap().payload;
        // reply() already sets the 0x80 bit: 0x18 | 0x80 = 0x98.
        assert_eq!(parse(&reply(0x18, &body)).unwrap(), sample());
    }

    #[test]
    fn parse_rejects_other_commands() {
        let body = set(&sample()).unwrap().payload;
        assert_eq!(
            parse(&reply(0x23, &body)),
            Err(LightingError::WrongReply("PRGB"))
        );
    }

    #[test]
    fn validation_errors() {
        let bad = |f: fn(&mut Lighting)| {
            let mut l = sample();
            f(&mut l);
            set(&l).unwrap_err()
        };
        assert_eq!(bad(|l| l.brightness = 5), LightingError::Brightness(5));
        assert_eq!(bad(|l| l.mode = 22), LightingError::Mode(22));
        assert_eq!(bad(|l| l.speed = 5), LightingError::Speed(5));
        assert_eq!(bad(|l| l.sleep_minutes = 4), LightingError::Sleep(4));
        assert_eq!(bad(|l| l.sleep_minutes = 121), LightingError::Sleep(121));
        assert_eq!(bad(|l| l.static_color = 8), LightingError::StaticColor(8));
        assert_eq!(bad(|l| l.dynamic_color = 8), LightingError::DynamicColor(8));
    }

    #[test]
    fn every_allowed_sleep_value_is_accepted() {
        for m in SLEEP_MINUTES {
            let mut l = sample();
            l.sleep_minutes = m;
            assert_eq!(l.validate(), Ok(()), "{m}");
        }
    }

    #[test]
    fn range_edges_are_accepted() {
        let mut l = sample();
        l.brightness = 4;
        l.speed = 4;
        l.mode = 21;
        l.static_color = 7;
        l.dynamic_color = 7;
        assert_eq!(l.validate(), Ok(()));
        l.brightness = 0;
        l.speed = 0;
        l.mode = 0;
        l.static_color = 0;
        l.dynamic_color = 0;
        assert_eq!(l.validate(), Ok(()));
    }

    // Checksum: 0x91 + 5 + 0x2A + 0x33 (last payload byte) = 0xF3.
    #[test]
    fn krgb_set_golden_bytes_rgb_order() {
        let out = krgb_set(0x0A, rgb(0x11, 0x22, 0x33))
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(
            out,
            frame(&[0x5C, 0x05, 0x2A, 0xF3, 0x01, 0x0A, 0x11, 0x22, 0x33])
        );
    }

    // Checksum: 0x91 + 5 + 0x2A + 0xFE = 0x1BE -> 0xBE.
    #[test]
    fn krgb_commit_golden_bytes() {
        let out = krgb_commit().encode().unwrap();
        assert_eq!(
            out,
            frame(&[0x5C, 0x05, 0x2A, 0xBE, 0x01, 0xFE, 0xFE, 0xFE, 0xFE])
        );
    }

    // Checksum: 0x91 + 2 + 0x2A + 0x0A (last payload byte = key) = 0xC7.
    #[test]
    fn krgb_get_golden_bytes() {
        let out = krgb_get(0x0A).encode().unwrap();
        assert_eq!(out, frame(&[0x5C, 0x02, 0x2A, 0xC7, 0x00, 0x0A]));
    }

    #[test]
    fn krgb_set_rejects_the_commit_marker() {
        assert_eq!(
            krgb_set(0xFE, rgb(1, 2, 3)).unwrap_err(),
            LightingError::ReservedKey
        );
    }

    #[test]
    fn krgb_parse_reads_rgb_order() {
        let r = reply(0x2A, &[0, 0x0A, 0x11, 0x22, 0x33]);
        assert_eq!(krgb_parse(&r), Ok((0x0A, rgb(0x11, 0x22, 0x33))));
        assert_eq!(
            krgb_parse(&reply(0x23, &[0, 0x0A, 1, 2, 3])),
            Err(LightingError::WrongReply("KRGB"))
        );
    }
}

//! Replies captured from a real WIN 68 HE MAX (FW App V1.1.3), see
//! docs/protocol/captures/.

use openhe_proto::frame::{Cmd, Request, Response};
use openhe_proto::key::parse_value;
use openhe_proto::model::{PollingRate, SystemMode};
use openhe_proto::order::{parse_polling, parse_profile, parse_system_mode};
use openhe_proto::sync;

fn raw(hex: &str) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (i, b) in hex.split(' ').enumerate() {
        out[i] = u8::from_str_radix(b, 16).unwrap();
    }
    out
}

const KEY_REPLY: &str = "5C 39 A3 6D 00 29 04 D0 07 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 00 00 00";
const POLLING_REPLY: &str = "5C 04 80 14 00 50 00 FF 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
const PROFILE_REPLY: &str = "5C 04 80 14 00 70 00 FF 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
const MODE_REPLY: &str = "5C 04 80 14 00 21 01 FF 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";

/// docs/protocol/captures/win68-info.txt; the serial is redacted as sixteen 0x58 ('X').
const SYNC_REPLY: &str = "5C 3C 81 4D 00 01 19 02 06 C0 01 00 10 58 58 58 58 58 58 58 58 58 58 58 58 58 58 58 58 10 41 70 70 20 56 31 2E 31 2E 33 00 00 31 36 3A 30 10 4E 6F 76 20 31 34 20 35 30 32 35 38 04 01 03 4C FF";

#[test]
fn real_sync_reply() {
    let r = Response::decode(&raw(SYNC_REPLY)).unwrap();
    assert!(r.checksum_ok());
    let info = sync::parse(&r);
    assert_eq!(info.board_id, 0x0602_1901);
    assert!(!info.in_bootloader);
    assert_eq!(info.serial, "XXXXXXXXXXXXXXXX");
    assert_eq!(info.firmware, "App V1.1.3");
    // The build-date slot is garbled on this firmware and is followed by binary
    // bytes (04 01 03 ...); the text stops at the first control byte.
    assert_eq!(info.build_date, "Nov 14 50258");
}

#[test]
fn real_key_reply_checksum_and_value() {
    let r = Response::decode(&raw(KEY_REPLY)).unwrap();
    assert!(r.checksum_ok());
    // key 0x29, layout 4 (actuation), 0x07D0 = 2000 um = 2.000 mm.
    assert_eq!(parse_value(&r), Some((0x29, 4, 2000)));
}

#[test]
fn real_order_replies_checksum_and_values() {
    let p = Response::decode(&raw(POLLING_REPLY)).unwrap();
    assert!(p.checksum_ok());
    assert_eq!(parse_polling(&p), Some(PollingRate::Hz8000));
    let m = Response::decode(&raw(MODE_REPLY)).unwrap();
    assert!(m.checksum_ok());
    assert_eq!(parse_system_mode(&m), Some(SystemMode::Windows));
    let pr = Response::decode(&raw(PROFILE_REPLY)).unwrap();
    assert!(pr.checksum_ok());
    assert_eq!(parse_profile(&pr), Some(0));
}

const TRACES: [&str; 3] = [
    include_str!("../../../docs/protocol/captures/win68-info.txt"),
    include_str!("../../../docs/protocol/captures/win68-dump-trace.txt"),
    include_str!("../../../docs/protocol/captures/win68-set-actuation.txt"),
];

/// Rebuilds the `Request` a captured host-to-device frame was encoded from.
fn request(frame: &[u8; 64]) -> Request {
    let cmd = match frame[2] {
        0x00 => Cmd::Order,
        0x01 => Cmd::Sync,
        0x23 => Cmd::Key,
        0x2B => Cmd::DefKey,
        other => panic!("unexpected command 0x{other:02X} in a capture"),
    };
    Request {
        cmd,
        payload: frame[4..4 + usize::from(frame[1])].to_vec(),
    }
}

#[test]
fn every_captured_reply_decodes_and_answers_its_request() {
    let mut pairs = 0;
    for trace in TRACES {
        let mut sent: Option<Request> = None;
        for line in trace.lines() {
            if let Some(hex) = line.strip_prefix("> ") {
                let req = request(&raw(hex));
                assert_eq!(req.encode().unwrap(), raw(hex), "re-encode {hex}");
                sent = Some(req);
            } else if let Some(hex) = line.strip_prefix("< ") {
                let req = sent.take().expect("a reply follows a request");
                let resp = Response::decode(&raw(hex)).unwrap();
                assert!(req.expects(&resp), "{req:?} does not accept {hex}");
                pairs += 1;
            }
        }
    }
    // info 4 + dump (4 + 3 + 68 * 7) + two set-actuation runs (3 + 7 + 2 each).
    assert_eq!(pairs, 4 + 483 + 2 * 12);
}

//! Replies captured from a real WIN 68 HE MAX (FW App V1.1.3), see
//! docs/protocol/captures/.

use openhe_proto::frame::Response;
use openhe_proto::key::parse_value;
use openhe_proto::model::PollingRate;
use openhe_proto::order::{parse_polling, parse_system_mode};

fn raw(hex: &str) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (i, b) in hex.split(' ').enumerate() {
        out[i] = u8::from_str_radix(b, 16).unwrap();
    }
    out
}

const KEY_REPLY: &str = "5C 39 A3 6D 00 29 04 D0 07 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 FF FF FF 00 00 00 00";
const POLLING_REPLY: &str = "5C 04 80 14 00 50 00 FF 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
const MODE_REPLY: &str = "5C 04 80 14 00 21 01 FF 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";

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
    assert!(parse_system_mode(&m).is_some());
}

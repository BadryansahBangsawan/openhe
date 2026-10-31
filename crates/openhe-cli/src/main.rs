#![forbid(unsafe_code)]
mod output;
mod trace;

use clap::{Parser, Subcommand};
use openhe_device::{list_devices, Error, HidTransport, Keyboard, Transport};
use openhe_proto::defkey::Matrix;
use openhe_proto::key::{KeyParam, TouchMode};
use openhe_proto::model::{validate_actuation, Model, SystemMode};
use openhe_proto::units::Micrometers;
use output::{matrix_rows, DumpOut, InfoOut, KeyOut};
use std::process::ExitCode;
use trace::TraceTransport;

#[derive(Parser, Debug)]
#[command(name = "openhe", version, about = "AULA HE MAX keyboard tool")]
struct Cli {
    /// Print every HID report (hex) to stderr.
    #[arg(long, global = true)]
    trace: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List connected HID devices.
    List,
    /// Show device info.
    Info,
    /// Dump info, matrix and per-key settings.
    Dump {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Set one key's actuation point. Writes to the keyboard, then reads it back.
    SetActuation {
        /// Key: a letter A-Z, a digit 0-9, or a hex HID usage code such as 0x2C.
        #[arg(value_parser = parse_key)]
        key: u8,
        /// New actuation point in millimeters, for example 1.50.
        #[arg(allow_negative_numbers = true)]
        mm: f32,
    },
}

const KEY_HELP: &str = "use a letter A-Z, a digit 0-9, or a hex HID usage code such as 0x2C";

/// Parses the key argument into its HID usage code: a letter A-Z (0x04-0x1D),
/// a digit 0-9 (0x1E-0x27), or one byte of hex such as `0x2C`.
fn parse_key(s: &str) -> Result<u8, String> {
    match s.as_bytes() {
        [c @ b'A'..=b'Z'] => Ok(c - b'A' + 0x04),
        [c @ b'a'..=b'z'] => Ok(c - b'a' + 0x04),
        [c @ b'1'..=b'9'] => Ok(c - b'1' + 0x1E),
        [b'0'] => Ok(0x27),
        _ => parse_hex_key(s).ok_or_else(|| KEY_HELP.to_string()),
    }
}

/// `0x2C` or `0X2c`: one byte, written with one or two hex digits.
fn parse_hex_key(s: &str) -> Option<u8> {
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if !(1..=2).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

/// A validated actuation change, ready to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActuationPlan {
    code: u8,
    old: Micrometers,
    new: Micrometers,
    /// The key's touch mode, which this command does not change.
    touch: TouchMode,
}

impl ActuationPlan {
    /// The only slot written: the actuation point (layout 4). The mode byte and
    /// every other parameter are left alone.
    fn entry(&self) -> (u8, KeyParam, u16) {
        (self.code, KeyParam::Actuation, self.new.0)
    }

    fn writing_line(&self) -> String {
        format!(
            "writing actuation for key 0x{:02X}: {} mm -> {} mm",
            self.code,
            output::mm(self.old),
            output::mm(self.new)
        )
    }

    /// A warning for a key in Global touch mode, where the per-key actuation
    /// may be ignored (PROTOCOL.md 7.x finding 13).
    fn note_line(&self) -> Option<String> {
        (self.touch == TouchMode::Global).then(|| {
            format!(
                "note: key 0x{:02X} is in Global touch mode; the per-key actuation may not take \
                 effect until the key uses Single/RT mode (not changed by this command)",
                self.code
            )
        })
    }

    fn done_line(&self) -> String {
        format!(
            "ok: key 0x{:02X} actuation is now {} mm (read back)",
            self.code,
            output::mm(self.new)
        )
    }
}

/// Why `set-actuation` stopped before writing anything.
#[derive(Debug, PartialEq)]
enum PlanError {
    /// The request cannot be honoured (exit code 1).
    Invalid(String),
    /// Talking to the keyboard failed.
    Device(Error),
}

impl From<Error> for PlanError {
    fn from(e: Error) -> Self {
        PlanError::Device(e)
    }
}

/// Reads what the change needs and checks it against the key's switch. Sends no SET.
/// A keyboard in bootloader mode is refused before anything else is read.
fn plan_actuation<T: Transport>(
    kb: &mut Keyboard<T>,
    code: u8,
    mm: f32,
) -> Result<ActuationPlan, PlanError> {
    if kb.info()?.in_bootloader {
        return Err(PlanError::Invalid(
            "the keyboard is in bootloader mode; refusing to write".to_string(),
        ));
    }
    let matrix = kb.matrix()?;
    if !matrix.keys().any(|(_, _, c)| c == code) {
        return Err(PlanError::Invalid(format!(
            "key 0x{code:02X} is not on this keyboard"
        )));
    }
    let current = kb.key_settings(code)?;
    let switch = kb
        .model
        .switches()
        .iter()
        .find(|s| s.id == current.switch)
        .ok_or_else(|| {
            PlanError::Invalid(format!(
                "key 0x{code:02X} reports switch id {}, which is not a known switch for {}",
                current.switch,
                kb.model.name()
            ))
        })?;
    let new = Micrometers::from_mm(mm)
        .map_err(|e| PlanError::Invalid(format!("cannot set actuation to {mm} mm: {e}")))?;
    validate_actuation(new, switch).map_err(|e| {
        PlanError::Invalid(format!(
            "cannot set actuation to {mm} mm: {e} (switch: {})",
            switch.name
        ))
    })?;
    Ok(ActuationPlan {
        code,
        old: current.actuation,
        new,
        touch: current.mode.touch,
    })
}

fn exit_code(e: &Error) -> u8 {
    match e {
        Error::Unsupported(_) | Error::NoConfigInterface(_) => 2,
        Error::Permission(_) | Error::Busy(_) => 3,
        _ => 1,
    }
}

/// Errors that mean the device is gone or silent: abort instead of continuing per key.
fn is_fatal(e: &Error) -> bool {
    matches!(
        e,
        Error::Disconnected
            | Error::Timeout
            | Error::Permission(_)
            | Error::Busy(_)
            | Error::Unsupported(_)
            | Error::NoConfigInterface(_)
    )
}

fn collect_keys<T: Transport>(kb: &mut Keyboard<T>, matrix: &Matrix) -> Result<Vec<KeyOut>, Error> {
    let mut keys = Vec::new();
    for (r, c, code) in matrix.keys() {
        match kb.key_settings(code) {
            Ok(s) => keys.push(KeyOut::ok(r, c, code, &s)),
            Err(e) if is_fatal(&e) => return Err(e),
            Err(e) => keys.push(KeyOut::err(r, c, code, e.to_string())),
        }
    }
    Ok(keys)
}

fn collect_dump<T: Transport>(kb: &mut Keyboard<T>, model: Model) -> Result<DumpOut, Error> {
    let info = collect_info(kb, model)?;
    let matrix = kb.matrix()?;
    let keys = collect_keys(kb, &matrix)?;
    Ok(DumpOut {
        info,
        matrix: matrix_rows(&matrix),
        keys,
    })
}

fn collect_info<T: Transport>(kb: &mut Keyboard<T>, model: Model) -> Result<InfoOut, Error> {
    let info = kb.info()?;
    let mode = match kb.system_mode()? {
        Some(SystemMode::Windows) => "Windows".to_string(),
        Some(SystemMode::Mac) => "Mac".to_string(),
        None => "unknown".to_string(),
    };
    Ok(InfoOut {
        model: model.name().to_string(),
        firmware: info.firmware,
        serial: info.serial,
        build_date: info.build_date,
        in_bootloader: info.in_bootloader,
        system_mode: mode,
        profile: kb.profile()?,
        polling_hz: kb.polling_rate()?.hz(),
    })
}

fn open(trace: bool) -> Result<(Keyboard<TraceTransport<HidTransport>>, Model), Error> {
    let (t, model) = HidTransport::open_first()?;
    let t = TraceTransport {
        inner: t,
        enabled: trace,
    };
    Ok((Keyboard::new(t, model), model))
}

/// Ok(true) = complete success, Ok(false) = partial read (exit 1).
fn run(cli: &Cli) -> Result<bool, Error> {
    match cli.command {
        Cmd::List => {
            let devices = list_devices()?;
            if devices.is_empty() {
                println!("no HID devices found");
            }
            for d in devices {
                let status = match d.model {
                    Some(m) => format!("supported ({})", m.name()),
                    None => "unsupported".to_string(),
                };
                println!("{}  [{}]  {}", d.product, status, d.path);
            }
        }
        Cmd::Info => {
            let (mut kb, model) = open(cli.trace)?;
            collect_info(&mut kb, model)?.print();
        }
        Cmd::Dump { json } => {
            let (mut kb, model) = open(cli.trace)?;
            let out = collect_dump(&mut kb, model)?;
            if json {
                let s = serde_json::to_string_pretty(&out).map_err(|e| Error::Io(e.to_string()))?;
                println!("{s}");
            } else {
                out.print();
            }
            let failed = out.keys.iter().filter(|k| k.error.is_some()).count();
            if failed > 0 {
                eprintln!(
                    "openhe: {failed} of {} keys could not be read",
                    out.keys.len()
                );
                return Ok(false);
            }
        }
        Cmd::SetActuation { key, mm } => {
            let (mut kb, _) = open(cli.trace)?;
            let plan = match plan_actuation(&mut kb, key, mm) {
                Ok(plan) => plan,
                Err(PlanError::Invalid(msg)) => {
                    eprintln!("openhe: {msg}");
                    return Ok(false);
                }
                Err(PlanError::Device(e)) => return Err(e),
            };
            eprintln!("{}", plan.writing_line());
            kb.set_key_params(&[plan.entry()])?;
            println!("{}", plan.done_line());
            if let Some(note) = plan.note_line() {
                eprintln!("{note}");
            }
        }
    }
    Ok(true)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("openhe: {e}");
            ExitCode::from(exit_code(&e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use openhe_device::MockTransport;
    #[test]
    fn cli_valid() {
        Cli::command().debug_assert();
    }
    #[test]
    fn dump_json() {
        let c = Cli::try_parse_from(["openhe", "dump", "--json"]).unwrap();
        assert!(matches!(c.command, Cmd::Dump { json: true }));
    }
    #[test]
    fn trace_flag() {
        let c = Cli::try_parse_from(["openhe", "--trace", "info"]).unwrap();
        assert!(c.trace);
    }

    fn key_reply(key: u8, layout: u8, v: u16) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xA3;
        raw[5] = key;
        raw[6] = layout;
        raw[7..9].copy_from_slice(&v.to_le_bytes());
        raw
    }

    fn two_key_matrix() -> Matrix {
        let mut m = Matrix::default();
        m.codes[0][0] = 0x0A;
        m.codes[0][1] = 0x0B;
        m
    }

    fn kb(m: MockTransport) -> Keyboard<MockTransport> {
        Keyboard::new(m, Model::Win68HeMax)
    }

    #[test]
    fn fatal_classification() {
        use openhe_device::Error as E;
        assert!(is_fatal(&E::Disconnected));
        assert!(is_fatal(&E::Timeout));
        assert!(is_fatal(&E::Permission("x".into())));
        assert!(is_fatal(&E::Busy("x".into())));
        assert!(is_fatal(&E::Unsupported("x".into())));
        assert!(is_fatal(&E::NoConfigInterface("x".into())));
        assert!(!is_fatal(&E::Io("x".into())));
        assert!(!is_fatal(&E::Frame(
            openhe_proto::frame::FrameError::ShortReport
        )));
    }

    #[test]
    fn invalid_input_exits_with_code_1_and_is_not_fatal() {
        let e = Error::InvalidInput("duplicate entry".into());
        assert_eq!(exit_code(&e), 1);
        assert!(!is_fatal(&e));
    }

    #[test]
    fn disconnect_aborts_dump() {
        let mut k = kb(MockTransport::new());
        k.session_mut()
            .transport_mut()
            .fail_with(Error::Disconnected);
        let e = collect_keys(&mut k, &two_key_matrix()).unwrap_err();
        assert_eq!(e, Error::Disconnected);
        assert_eq!(k.session_mut().transport().written().len(), 0);
    }

    #[test]
    fn per_key_io_error_is_recorded_and_dump_continues() {
        let mut m = MockTransport::new();
        // Key 0x0A: an unknown mode value -> Io error, and its other params are not read.
        m.push_reply(key_reply(0x0A, 8, 0x07));
        // Key 0x0B: seven good replies.
        for (layout, v) in [
            (8u8, 0x20u16),
            (4, 1500),
            (20, 1),
            (21, 2),
            (22, 3),
            (23, 4),
            (25, 7),
        ] {
            m.push_reply(key_reply(0x0B, layout, v));
        }
        let keys = collect_keys(&mut kb(m), &two_key_matrix()).unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys[0].error.is_some());
        assert!(keys[1].error.is_none());
        assert_eq!(keys[1].switch, Some(7));
    }

    #[test]
    fn parse_key_maps_letters_digits_and_hex() {
        for (s, want) in [
            ("G", 0x0A),
            ("g", 0x0A),
            ("A", 0x04),
            ("Z", 0x1D),
            ("1", 0x1E),
            ("9", 0x26),
            ("0", 0x27),
            ("0x2C", 0x2C),
            ("0X2c", 0x2C),
            ("0xA", 0x0A),
            ("0xFF", 0xFF),
        ] {
            assert_eq!(parse_key(s), Ok(want), "{s}");
        }
    }

    #[test]
    fn parse_key_covers_every_letter_and_digit() {
        for (i, c) in ('A'..='Z').enumerate() {
            let want = 0x04 + i as u8;
            assert_eq!(parse_key(&c.to_string()), Ok(want));
            assert_eq!(parse_key(&c.to_ascii_lowercase().to_string()), Ok(want));
        }
        for d in 1..=9u8 {
            assert_eq!(parse_key(&d.to_string()), Ok(0x1D + d));
        }
    }

    #[test]
    fn parse_key_rejects_names_and_malformed_hex() {
        for s in [
            "Esc", "", "ab", "10", "0x", "0x123", "0xZZ", "0x+1", " g", "g ", "é", "-", "0x 1",
            "x2C", "2C",
        ] {
            assert!(parse_key(s).is_err(), "{s:?} should be rejected");
        }
        let msg = parse_key("Esc").unwrap_err();
        assert!(msg.contains("A-Z") && msg.contains("0x"), "{msg}");
    }

    #[test]
    fn set_actuation_parses_key_and_mm() {
        let c = Cli::try_parse_from(["openhe", "set-actuation", "G", "1.50"]).unwrap();
        assert!(matches!(c.command, Cmd::SetActuation { key: 0x0A, mm } if mm == 1.5));
    }

    #[test]
    fn set_actuation_takes_trace_and_a_hex_key() {
        let c = Cli::try_parse_from(["openhe", "--trace", "set-actuation", "0x2C", "2"]).unwrap();
        assert!(c.trace);
        assert!(matches!(c.command, Cmd::SetActuation { key: 0x2C, .. }));
    }

    #[test]
    fn set_actuation_rejects_bad_arguments() {
        let cases: [&[&str]; 4] = [
            &["openhe", "set-actuation", "Esc", "1.5"],
            &["openhe", "set-actuation", "G"],
            &["openhe", "set-actuation", "G", "abc"],
            &["openhe", "set-actuation"],
        ];
        for args in cases {
            assert!(
                Cli::try_parse_from(args.iter().copied()).is_err(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn negative_mm_is_not_mistaken_for_a_flag() {
        let c = Cli::try_parse_from(["openhe", "set-actuation", "G", "-0.5"]).unwrap();
        assert!(matches!(c.command, Cmd::SetActuation { mm, .. } if mm < 0.0));
    }

    fn defkey_reply(row_a: u8, a: &[u8], row_b: u8, b: &[u8]) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0xAB;
        raw[5] = row_a;
        raw[6..6 + a.len()].copy_from_slice(a);
        raw[27] = row_b;
        raw[28..28 + b.len()].copy_from_slice(b);
        raw
    }

    /// Queues the three DEFKEY replies for a matrix whose only keys are `codes` (row 1).
    fn push_matrix(m: &mut MockTransport, codes: &[u8]) {
        m.push_reply(defkey_reply(0, &[], 1, codes));
        m.push_reply(defkey_reply(2, &[], 3, &[]));
        m.push_reply(defkey_reply(4, &[], 5, &[]));
    }

    /// Queues the SYNC reply `info` reads.
    fn push_sync(m: &mut MockTransport, in_bootloader: bool) {
        let mut raw = [0u8; 64];
        raw[0] = 0x5C;
        raw[2] = 0x81;
        raw[4 + 7] = u8::from(in_bootloader);
        m.push_reply(raw);
    }

    /// Queues the seven replies `key_settings` reads (touch Global, factory-style values).
    fn push_settings(m: &mut MockTransport, key: u8, actuation: u16, switch: u16) {
        push_settings_with_mode(m, key, 0, actuation, switch);
    }

    fn push_settings_with_mode(
        m: &mut MockTransport,
        key: u8,
        mode: u16,
        actuation: u16,
        switch: u16,
    ) {
        for (layout, v) in [
            (8u8, mode),
            (4, actuation),
            (20, 20),
            (21, 20),
            (22, 200),
            (23, 200),
            (25, switch),
        ] {
            m.push_reply(key_reply(key, layout, v));
        }
    }

    /// A KEY SET frame: cmd 0x23 with the SET flag.
    fn is_key_set(frame: &[u8; 64]) -> bool {
        frame[2] == 0x23 && frame[4] == 0x01
    }

    fn plan_on(
        code: u8,
        mm: f32,
        settings: (u16, u16),
        keys: &[u8],
    ) -> Result<ActuationPlan, PlanError> {
        let mut m = MockTransport::new();
        push_sync(&mut m, false);
        push_matrix(&mut m, keys);
        push_settings(&mut m, code, settings.0, settings.1);
        plan_actuation(&mut kb(m), code, mm)
    }

    #[test]
    fn plan_reads_then_computes_the_change_without_writing() {
        let mut m = MockTransport::new();
        push_sync(&mut m, false);
        push_matrix(&mut m, &[0x0A, 0x0B]);
        push_settings(&mut m, 0x0A, 2000, 0);
        let mut k = kb(m);
        let plan = plan_actuation(&mut k, 0x0A, 1.5).unwrap();
        assert_eq!(
            plan,
            ActuationPlan {
                code: 0x0A,
                old: Micrometers(2000),
                new: Micrometers(1500),
                touch: TouchMode::Global,
            }
        );
        assert_eq!(plan.entry(), (0x0A, KeyParam::Actuation, 1500));
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 1 + 3 + 7); // SYNC, the matrix, then the key's seven parameters
        assert_eq!(w[0][2], 0x01, "device info is read first");
        assert!(!w.iter().any(is_key_set), "planning must not write");
    }

    #[test]
    fn plan_refuses_a_keyboard_in_bootloader_mode() {
        let mut m = MockTransport::new();
        push_sync(&mut m, true);
        push_matrix(&mut m, &[0x0A]);
        push_settings(&mut m, 0x0A, 2000, 0);
        let mut k = kb(m);
        assert_eq!(
            plan_actuation(&mut k, 0x0A, 1.5),
            Err(PlanError::Invalid(
                "the keyboard is in bootloader mode; refusing to write".to_string()
            ))
        );
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 1, "nothing after SYNC");
        assert_eq!(w[0][2], 0x01);
    }

    #[test]
    fn global_touch_mode_gets_a_note_and_other_modes_do_not() {
        let plan = plan_on(0x0A, 1.5, (2000, 0), &[0x0A]).unwrap();
        assert_eq!(
            plan.note_line().as_deref(),
            Some(
                "note: key 0x0A is in Global touch mode; the per-key actuation may not take \
                 effect until the key uses Single/RT mode (not changed by this command)"
            )
        );
        for mode in [0x10u16, 0x20] {
            let mut m = MockTransport::new();
            push_sync(&mut m, false);
            push_matrix(&mut m, &[0x0A]);
            push_settings_with_mode(&mut m, 0x0A, mode, 2000, 0);
            let plan = plan_actuation(&mut kb(m), 0x0A, 1.5).unwrap();
            assert_eq!(plan.note_line(), None, "mode 0x{mode:02X}");
        }
    }

    #[test]
    fn plan_rejects_a_key_missing_from_the_matrix_before_reading_it() {
        let mut m = MockTransport::new();
        push_sync(&mut m, false);
        push_matrix(&mut m, &[0x0B]);
        let mut k = kb(m);
        assert_eq!(
            plan_actuation(&mut k, 0x0A, 1.5),
            Err(PlanError::Invalid(
                "key 0x0A is not on this keyboard".to_string()
            ))
        );
        assert_eq!(k.session_mut().transport().written().len(), 1 + 3); // SYNC and the matrix only
    }

    #[test]
    fn plan_rejects_values_the_switch_cannot_do() {
        // Wing Chun allows 0.100 mm to 3.400 mm.
        for (mm, msg) in [
            (
                0.05,
                "cannot set actuation to 0.05 mm: 50 um is below the minimum of 100 um (switch: Wing Chun)",
            ),
            (
                3.5,
                "cannot set actuation to 3.5 mm: 3500 um is above the maximum of 3400 um (switch: Wing Chun)",
            ),
        ] {
            assert_eq!(
                plan_on(0x0A, mm, (2000, 0), &[0x0A]),
                Err(PlanError::Invalid(msg.to_string()))
            );
        }
    }

    #[test]
    fn plan_accepts_both_range_limits() {
        for (mm, um) in [(0.1, 100), (3.4, 3400)] {
            let plan = plan_on(0x0A, mm, (2000, 0), &[0x0A]).unwrap();
            assert_eq!(plan.new, Micrometers(um), "{mm}");
        }
    }

    #[test]
    fn plan_rejects_non_finite_negative_and_oversized_values() {
        for (mm, msg) in [
            (
                f32::NAN,
                "cannot set actuation to NaN mm: value is not a finite number",
            ),
            (
                f32::INFINITY,
                "cannot set actuation to inf mm: value is not a finite number",
            ),
            (-0.5, "cannot set actuation to -0.5 mm: value is negative"),
            (
                70.0,
                "cannot set actuation to 70 mm: value too large for 16 bits",
            ),
        ] {
            assert_eq!(
                plan_on(0x0A, mm, (2000, 0), &[0x0A]),
                Err(PlanError::Invalid(msg.to_string()))
            );
        }
    }

    #[test]
    fn plan_rounds_mm_to_the_nearest_micrometer() {
        // 1.13 mm is 1129.99999... in f32: it must become 1130 um, not 1129.
        // 1.0006 mm must round up to 1001 um; 1.0004 mm down to 1000 um.
        for (mm, um) in [(1.13, 1130), (1.0006, 1001), (1.0004, 1000)] {
            let plan = plan_on(0x0A, mm, (2000, 0), &[0x0A]).unwrap();
            assert_eq!(plan.new, Micrometers(um), "{mm} mm");
        }
    }

    #[test]
    fn plan_rejects_an_unknown_switch_id() {
        assert_eq!(
            plan_on(0x0A, 1.5, (2000, 9), &[0x0A]),
            Err(PlanError::Invalid(
                "key 0x0A reports switch id 9, which is not a known switch for WIN 68 HE MAX"
                    .to_string()
            ))
        );
    }

    #[test]
    fn plan_uses_the_models_own_switch_table() {
        // Switch id 1 is Magnetic Jade Pro (3.3 mm) on the WIN 68 but Black Emperor (3.4 mm) on the WIN 60.
        assert_eq!(
            plan_on(0x0A, 3.4, (2000, 1), &[0x0A]),
            Err(PlanError::Invalid(
                "cannot set actuation to 3.4 mm: 3400 um is above the maximum of 3300 um (switch: Magnetic Jade Pro)"
                    .to_string()
            ))
        );
        let mut m = MockTransport::new();
        push_sync(&mut m, false);
        push_matrix(&mut m, &[0x0A]);
        push_settings(&mut m, 0x0A, 2000, 1);
        let mut k60 = Keyboard::new(m, Model::Win60HeMax);
        assert_eq!(
            plan_actuation(&mut k60, 0x0A, 3.4).map(|p| p.new),
            Ok(Micrometers(3400))
        );
    }

    #[test]
    fn plan_passes_device_errors_through() {
        let mut m = MockTransport::new();
        m.fail_with(Error::Disconnected);
        assert_eq!(
            plan_actuation(&mut kb(m), 0x0A, 1.5),
            Err(PlanError::Device(Error::Disconnected))
        );
    }

    #[test]
    fn full_flow_writes_only_the_actuation_slot_then_verifies() {
        let mut m = MockTransport::new();
        push_sync(&mut m, false);
        push_matrix(&mut m, &[0x0A]);
        push_settings(&mut m, 0x0A, 2000, 0);
        m.push_reply(key_reply(0x0A, 4, 1500)); // SET echo
        m.push_reply(key_reply(0x0A, 4, 1500)); // read-back
        let mut k = kb(m);
        let plan = plan_actuation(&mut k, 0x0A, 1.5).unwrap();
        k.set_key_params(&[plan.entry()]).unwrap();
        let w = k.session_mut().transport().written();
        assert_eq!(w.len(), 1 + 3 + 7 + 1 + 1);
        let sets: Vec<_> = w.iter().filter(|f| is_key_set(f)).collect();
        assert_eq!(sets.len(), 1, "exactly one SET");
        // Layout 4 (actuation) only: never layout 8 (the mode byte) or anything else.
        assert_eq!(sets[0][4..9], [0x01, 0x0A, 0x04, 0xDC, 0x05]);
    }

    #[test]
    fn progress_lines() {
        let plan = ActuationPlan {
            code: 0x0A,
            old: Micrometers(2000),
            new: Micrometers(1500),
            touch: TouchMode::Single,
        };
        assert_eq!(
            plan.writing_line(),
            "writing actuation for key 0x0A: 2.000 mm -> 1.500 mm"
        );
        assert_eq!(
            plan.done_line(),
            "ok: key 0x0A actuation is now 1.500 mm (read back)"
        );
    }
}

#![forbid(unsafe_code)]
mod output;
mod trace;

use clap::{Parser, Subcommand};
use openhe_device::{list_devices, Error, HidTransport, Keyboard, Transport};
use openhe_proto::defkey::Matrix;
use openhe_proto::model::{Model, SystemMode};
use output::{matrix_rows, DumpOut, InfoOut, KeyOut};
use std::process::ExitCode;
use trace::TraceTransport;

#[derive(Parser, Debug)]
#[command(
    name = "openhe",
    version,
    about = "Read-only AULA HE MAX keyboard tool"
)]
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
}

fn exit_code(e: &Error) -> u8 {
    match e {
        Error::Unsupported(_) => 2,
        Error::Permission(_) => 3,
        _ => 1,
    }
}

/// Errors that mean the device is gone or silent: abort instead of continuing per key.
fn is_fatal(e: &Error) -> bool {
    matches!(
        e,
        Error::Disconnected | Error::Timeout | Error::Permission(_) | Error::Unsupported(_)
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
        assert!(is_fatal(&E::Unsupported("x".into())));
        assert!(!is_fatal(&E::Io("x".into())));
        assert!(!is_fatal(&E::Frame(
            openhe_proto::frame::FrameError::ShortReport
        )));
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
        // Key 0x0A: wrong layout for the first param -> Io error.
        m.push_reply(key_reply(0x0A, 20, 1));
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
}

#![forbid(unsafe_code)]
mod output;
mod trace;

use clap::{Parser, Subcommand};
use openhe_device::{list_devices, Error, HidTransport, Keyboard, Transport};
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

fn run(cli: &Cli) -> Result<(), Error> {
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
            let info = collect_info(&mut kb, model)?;
            let matrix = kb.matrix()?;
            let keys = matrix
                .keys()
                .map(|(r, c, code)| match kb.key_settings(code) {
                    Ok(s) => KeyOut::ok(r, c, code, &s),
                    Err(e) => KeyOut::err(r, c, code, e.to_string()),
                })
                .collect();
            let out = DumpOut {
                info,
                matrix: matrix_rows(&matrix),
                keys,
            };
            if json {
                let s = serde_json::to_string_pretty(&out).map_err(|e| Error::Io(e.to_string()))?;
                println!("{s}");
            } else {
                out.print();
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
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
}

use hidapi::{HidApi, HidDevice};
use openhe_proto::model::Model;

use crate::{Error, Transport};

pub const VENDOR_ID: u16 = 0x1CA2;
pub const USAGE_PAGE: u16 = 0xFFA0;
/// The Linux udev rule, relative to the repository root. It must sort before
/// systemd's 73-seat-late.rules, which applies the `uaccess` tag.
pub const UDEV_RULES: &str = "packaging/linux/70-openhe.rules";

pub fn classify(vid: u16, pid: u16, usage_page: u16, product: &str) -> Option<Model> {
    if usage_page != USAGE_PAGE {
        return None;
    }
    Model::from_hid(vid, pid, product)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub model: Option<Model>,
    pub product: String,
    pub path: String,
}

fn hid_api() -> Result<HidApi, Error> {
    HidApi::new().map_err(|e| Error::Io(e.to_string()))
}

pub fn list_devices() -> Result<Vec<DeviceEntry>, Error> {
    let api = hid_api()?;
    Ok(api
        .device_list()
        .filter(|d| d.vendor_id() == VENDOR_ID && d.usage_page() == USAGE_PAGE)
        .map(|d| {
            let product = d.product_string().unwrap_or("").to_string();
            DeviceEntry {
                model: classify(d.vendor_id(), d.product_id(), d.usage_page(), &product),
                product,
                path: d.path().to_string_lossy().into_owned(),
            }
        })
        .collect())
}

pub struct HidTransport {
    device: HidDevice,
}

impl HidTransport {
    pub fn open_first() -> Result<(HidTransport, Model), Error> {
        let api = hid_api()?;
        let mut unsupported: Vec<String> = Vec::new();
        for info in api.device_list() {
            if info.vendor_id() != VENDOR_ID {
                continue;
            }
            let product = info.product_string().unwrap_or("");
            let model = classify(
                info.vendor_id(),
                info.product_id(),
                info.usage_page(),
                product,
            );
            match model {
                Some(model) if info.usage() == 1 => {
                    let device = info
                        .open_device(&api)
                        .map_err(|e| open_error(&e.to_string()))?;
                    return Ok((HidTransport { device }, model));
                }
                Some(_) => {}
                None => unsupported.push(product.to_string()),
            }
        }
        let mut msg = String::from("no supported keyboard was found");
        if !unsupported.is_empty() {
            msg.push_str(&format!(
                " (unsupported devices seen: {})",
                unsupported.join(", ")
            ));
        }
        Err(Error::Unsupported(msg))
    }
}

/// kIOReturnExclusiveAccess: macOS hidapi opens the keyboard exclusively,
/// so this is what a second program gets while one holds it.
const EXCLUSIVE_ACCESS: &str = "0xe00002c5";
const EXCLUSIVE_ACCESS_TEXT: &str = "exclusive access and device already open";

/// The error for a failed open: `Busy` when another program holds the
/// keyboard, else `Permission` (with the udev hint on Linux).
fn open_error(detail: &str) -> Error {
    let lower = detail.to_ascii_lowercase();
    if lower.contains(EXCLUSIVE_ACCESS) || lower.contains(EXCLUSIVE_ACCESS_TEXT) {
        Error::Busy(detail.to_string())
    } else {
        Error::Permission(open_failure_message(detail))
    }
}

fn open_failure_message(detail: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("{detail}; install {UDEV_RULES} to grant access to the keyboard")
    }
    #[cfg(not(target_os = "linux"))]
    {
        detail.to_string()
    }
}

impl Transport for HidTransport {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error> {
        let mut buf = [0u8; 65];
        buf[1..].copy_from_slice(report);
        self.device
            .write(&buf)
            .map(|_| ())
            .map_err(|_| Error::Disconnected)
    }

    fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
        let mut buf = [0u8; 64];
        let timeout = timeout_ms.min(i32::MAX as u32) as i32;
        match self.device.read_timeout(&mut buf, timeout) {
            Ok(0) => Ok(None),
            Ok(n) => {
                let mut out = [0u8; 64];
                out[..n.min(64)].copy_from_slice(&buf[..n.min(64)]);
                Ok(Some(out))
            }
            Err(_) => Err(Error::Disconnected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn udev_rule_exists_and_runs_before_uaccess_is_applied() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(UDEV_RULES);
        let rule = std::fs::read_to_string(&path).expect("udev rule file");
        assert!(rule.contains(r#"TAG+="uaccess""#));
        assert!(rule.contains("1901|1902"));
        let name = Path::new(UDEV_RULES).file_name().unwrap().to_str().unwrap();
        let number: u32 = name.split('-').next().unwrap().parse().unwrap();
        assert!(number < 73, "{name} sorts after 73-seat-late.rules");
        assert!(
            rule.contains(&format!("sudo cp {name} ")),
            "install hint names the file"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn open_failure_names_the_udev_rule() {
        assert!(open_failure_message("denied").contains("packaging/linux/70-openhe.rules"));
    }

    /// The text macOS hidapi gives when another program holds the keyboard
    /// (seen on hardware with a second OpenHE open).
    const MAC_BUSY: &str = "hid_open_path: failed to open IOHIDDevice from mach entry: \
        (0xE00002C5) (iokit/common) exclusive access and device already open";

    #[test]
    fn a_device_held_by_another_program_is_busy_not_a_permission_problem() {
        match open_error(MAC_BUSY) {
            Error::Busy(detail) => {
                assert!(detail.contains("0xE00002C5"));
                assert!(!detail.contains("udev"), "no Linux hint for a busy device");
            }
            other => panic!("expected Busy, got {other:?}"),
        }
        // Either half of the message is enough.
        assert!(matches!(open_error("(0xe00002c5)"), Error::Busy(_)));
        assert!(matches!(
            open_error("Exclusive access and device already open"),
            Error::Busy(_)
        ));
        assert!(matches!(open_error("access denied"), Error::Permission(_)));
    }

    #[test]
    fn model_filter() {
        assert_eq!(
            classify(0x1CA2, 0x1901, 0xFFA0, "WIN 68 HE MAX"),
            Some(Model::Win68HeMax)
        );
        assert_eq!(classify(0x1CA2, 0x1901, 0xFFA0, "WIN 68 HE PRO"), None);
        assert_eq!(classify(0x1CA2, 0x1901, 0x0001, "WIN 68 HE MAX"), None);
        assert_eq!(
            classify(0x1CA2, 0x1902, 0xFFA0, "WIN 60 HE MAX"),
            Some(Model::Win60HeMax)
        );
    }
}

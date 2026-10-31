use hidapi::{HidApi, HidDevice};
use openhe_proto::model::Model;

use crate::{Error, Transport};

pub const VENDOR_ID: u16 = 0x1CA2;
pub const USAGE_PAGE: u16 = 0xFFA0;
/// The HID usage of the configuration interface on `USAGE_PAGE`.
pub const USAGE: u16 = 1;
/// The Linux udev rule, relative to the repository root. It must sort before
/// systemd's 73-seat-late.rules, which applies the `uaccess` tag.
pub const UDEV_RULES: &str = "packaging/linux/70-openhe.rules";

pub fn classify(vid: u16, pid: u16, usage_page: u16, product: &str) -> Option<Model> {
    if usage_page != USAGE_PAGE {
        return None;
    }
    Model::from_hid(vid, pid, product)
}

/// One HID interface of the vendor, as `open_first` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Iface {
    vid: u16,
    pid: u16,
    usage_page: u16,
    usage: u16,
    product: String,
}

/// What `open_first` makes of the vendor's HID interfaces.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct OpenPlan {
    /// The configuration interfaces of supported keyboards, to try in
    /// order: (index into the interface list, model).
    candidates: Vec<(usize, Model)>,
    /// Supported keyboards present on any interface (their keyboard,
    /// consumer or other interfaces), once each.
    models: Vec<Model>,
    /// Products of the vendor that are no supported keyboard, once each.
    unsupported: Vec<String>,
}

/// Sorts the vendor's interfaces: a supported keyboard's other
/// interfaces (boot keyboard, consumer control, ...) only show that the
/// keyboard is there; they are never listed as unsupported devices.
fn plan_open(ifaces: &[Iface]) -> OpenPlan {
    let mut plan = OpenPlan::default();
    for (i, f) in ifaces.iter().enumerate() {
        if f.vid != VENDOR_ID {
            continue;
        }
        match Model::from_hid(f.vid, f.pid, &f.product) {
            Some(model) => {
                if !plan.models.contains(&model) {
                    plan.models.push(model);
                }
                if classify(f.vid, f.pid, f.usage_page, &f.product).is_some() && f.usage == USAGE {
                    plan.candidates.push((i, model));
                }
            }
            None => {
                let name = match f.product.trim() {
                    "" => format!("USB 0x{:04X}:0x{:04X}", f.vid, f.pid),
                    p => p.to_string(),
                };
                if !plan.unsupported.contains(&name) {
                    plan.unsupported.push(name);
                }
            }
        }
    }
    plan
}

/// How much an open failure tells the user: a permission problem first
/// (the fix is on this computer), then a keyboard held by another
/// program, then anything else.
fn failure_rank(e: &Error) -> u8 {
    match e {
        Error::Permission(_) => 2,
        Error::Busy(_) => 1,
        _ => 0,
    }
}

/// The error of an `open_first` that opened nothing: the most telling
/// failure of the candidates tried (`failure_rank`, the first one on a
/// tie); else, with a supported keyboard present but no configuration
/// interface, says so; else "no supported keyboard was found", naming the
/// vendor's unsupported devices.
fn not_opened(plan: &OpenPlan, failures: Vec<Error>) -> Error {
    if let Some(e) = failures
        .into_iter()
        .min_by_key(|e| std::cmp::Reverse(failure_rank(e)))
    {
        return e;
    }
    if !plan.models.is_empty() {
        let names: Vec<&str> = plan.models.iter().map(|m| m.name()).collect();
        return Error::Unsupported(format!(
            "{} found, but not its configuration interface \
             (usage page 0x{USAGE_PAGE:04X}, usage {USAGE}); reconnect the keyboard",
            names.join(", ")
        ));
    }
    let mut msg = String::from("no supported keyboard was found");
    if !plan.unsupported.is_empty() {
        msg.push_str(&format!(
            " (unsupported devices seen: {})",
            plan.unsupported.join(", ")
        ));
    }
    Error::Unsupported(msg)
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
    /// Opens the configuration interface of the first supported keyboard.
    ///
    /// Every matching interface is tried in turn; when none opens, the
    /// most telling failure is returned (see `not_opened`).
    pub fn open_first() -> Result<(HidTransport, Model), Error> {
        let api = hid_api()?;
        let infos: Vec<_> = api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR_ID)
            .collect();
        let ifaces: Vec<Iface> = infos
            .iter()
            .map(|d| Iface {
                vid: d.vendor_id(),
                pid: d.product_id(),
                usage_page: d.usage_page(),
                usage: d.usage(),
                product: d.product_string().unwrap_or("").to_string(),
            })
            .collect();
        let plan = plan_open(&ifaces);
        let mut failures = Vec::new();
        for &(i, model) in &plan.candidates {
            match infos[i].open_device(&api) {
                Ok(device) => return Ok((HidTransport { device }, model)),
                Err(e) => failures.push(open_error(&e.to_string())),
            }
        }
        Err(not_opened(&plan, failures))
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

    fn iface(pid: u16, usage_page: u16, usage: u16, product: &str) -> Iface {
        Iface {
            vid: VENDOR_ID,
            pid,
            usage_page,
            usage,
            product: product.to_string(),
        }
    }

    /// The interfaces a WIN 68 HE MAX shows besides its configuration
    /// interface (boot keyboard, consumer control, a second vendor usage).
    fn win68_other_ifaces() -> Vec<Iface> {
        vec![
            iface(0x1901, 0x0001, 0x06, "WIN 68 HE MAX"),
            iface(0x1901, 0x000C, 0x01, "WIN 68 HE MAX"),
            iface(0x1901, USAGE_PAGE, 0x02, "WIN 68 HE MAX"),
        ]
    }

    #[test]
    fn a_supported_keyboards_other_interfaces_are_not_unsupported_devices() {
        let mut ifaces = win68_other_ifaces();
        ifaces.insert(1, iface(0x1901, USAGE_PAGE, USAGE, "WIN 68 HE MAX"));
        ifaces.push(iface(0x1901, USAGE_PAGE, USAGE, "WIN 68 HE PRO"));
        ifaces.push(iface(0x1901, 0x0001, 0x06, "WIN 68 HE PRO"));
        let mut other_vendor = iface(0x1901, USAGE_PAGE, USAGE, "WIN 68 HE MAX");
        other_vendor.vid = 0x1234;
        ifaces.push(other_vendor);
        let plan = plan_open(&ifaces);
        assert_eq!(plan.candidates, vec![(1, Model::Win68HeMax)]);
        assert_eq!(plan.models, vec![Model::Win68HeMax]);
        assert_eq!(plan.unsupported, vec!["WIN 68 HE PRO".to_string()]);
    }

    #[test]
    fn a_keyboard_without_its_configuration_interface_is_named() {
        let plan = plan_open(&win68_other_ifaces());
        assert!(plan.candidates.is_empty());
        match not_opened(&plan, Vec::new()) {
            Error::Unsupported(m) => {
                assert!(
                    m.starts_with("WIN 68 HE MAX found, but not its configuration interface"),
                    "{m}"
                );
                assert!(!m.contains("unsupported devices seen"), "{m}");
            }
            e => panic!("expected Unsupported, got {e:?}"),
        }
    }

    #[test]
    fn nothing_supported_lists_the_unsupported_devices_once() {
        let ifaces = vec![
            iface(0x1999, USAGE_PAGE, USAGE, "AULA F99"),
            iface(0x1999, 0x0001, 0x06, "AULA F99"),
            iface(0x1998, 0x0001, 0x06, ""),
        ];
        let plan = plan_open(&ifaces);
        assert_eq!(
            not_opened(&plan, Vec::new()),
            Error::Unsupported(
                "no supported keyboard was found (unsupported devices seen: \
                 AULA F99, USB 0x1CA2:0x1998)"
                    .into()
            )
        );
        assert_eq!(
            not_opened(&plan_open(&[]), Vec::new()),
            Error::Unsupported("no supported keyboard was found".into())
        );
    }

    /// Every matching interface is a candidate (two keyboards, or a
    /// keyboard listed twice), in the order listed.
    #[test]
    fn every_matching_interface_is_tried_in_order() {
        let ifaces = vec![
            iface(0x1902, USAGE_PAGE, USAGE, "WIN 60 HE MAX"),
            iface(0x1901, 0x0001, 0x06, "WIN 68 HE MAX"),
            iface(0x1901, USAGE_PAGE, USAGE, "WIN 68 HE MAX"),
        ];
        let plan = plan_open(&ifaces);
        assert_eq!(
            plan.candidates,
            vec![(0, Model::Win60HeMax), (2, Model::Win68HeMax)]
        );
        assert_eq!(plan.models, vec![Model::Win60HeMax, Model::Win68HeMax]);
    }

    /// When no candidate opens, a permission problem is reported before a
    /// busy keyboard, and either before the "not found" message.
    #[test]
    fn the_most_telling_open_failure_is_returned() {
        let plan = plan_open(&[iface(0x1901, USAGE_PAGE, USAGE, "WIN 68 HE MAX")]);
        let busy = || Error::Busy("held".into());
        let denied = |n: &str| Error::Permission(n.into());
        assert_eq!(
            not_opened(&plan, vec![busy(), denied("a"), denied("b")]),
            denied("a")
        );
        assert_eq!(not_opened(&plan, vec![busy()]), busy());
        assert_eq!(
            not_opened(&plan, vec![Error::Io("x".into()), busy()]),
            busy()
        );
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

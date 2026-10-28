use hidapi::{HidApi, HidDevice};
use openhe_proto::model::Model;

use crate::{Error, Transport};

pub const VENDOR_ID: u16 = 0x1CA2;
pub const USAGE_PAGE: u16 = 0xFFA0;

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
                        .map_err(|e| Error::Permission(open_failure_message(&e.to_string())))?;
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

fn open_failure_message(detail: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("{detail}; install packaging/linux/99-openhe.rules to grant access to the keyboard")
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

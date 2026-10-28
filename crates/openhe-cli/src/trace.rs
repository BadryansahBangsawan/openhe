use openhe_device::{Error, Transport};

/// Space-separated uppercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One trace line: direction marker, space, hex.
pub fn line(dir: char, report: &[u8; 64]) -> String {
    format!("{dir} {}", hex(report))
}

/// Wraps a transport and optionally prints every report to stderr.
pub struct TraceTransport<T: Transport> {
    pub inner: T,
    pub enabled: bool,
}

impl<T: Transport> Transport for TraceTransport<T> {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error> {
        if self.enabled {
            eprintln!("{}", line('>', report));
        }
        self.inner.write(report)
    }

    fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
        let r = self.inner.read(timeout_ms)?;
        if let (true, Some(report)) = (self.enabled, &r) {
            eprintln!("{}", line('<', report));
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::MockTransport;

    #[test]
    fn hex_is_uppercase_space_separated() {
        assert_eq!(hex(&[0x00, 0x5C, 0xAB, 0x0F]), "00 5C AB 0F");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn trace_passes_through_and_formats() {
        let mut t = TraceTransport {
            inner: MockTransport::new(),
            enabled: false,
        };
        let mut r = [0u8; 64];
        r[0] = 0x35;
        t.write(&r).unwrap();
        assert_eq!(t.inner.written()[0], r);
        assert!(t.read(1).unwrap().is_none());
        assert!(line('>', &r).starts_with("> 35 00"));
        assert_eq!(line('<', &r).split(' ').count(), 65);
    }
}

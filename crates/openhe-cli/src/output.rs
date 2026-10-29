use openhe_device::KeySettings;
use openhe_proto::defkey::Matrix;
use serde::Serialize;

#[derive(Serialize)]
pub struct InfoOut {
    pub model: String,
    pub firmware: String,
    pub serial: String,
    pub build_date: String,
    pub in_bootloader: bool,
    pub system_mode: String,
    pub profile: u8,
    pub polling_hz: u32,
}

impl InfoOut {
    pub fn print(&self) {
        println!("Model:        {}", self.model);
        println!("Firmware:     {}", self.firmware);
        println!("Serial:       {}", self.serial);
        println!("Build date:   {}", self.build_date);
        println!("Bootloader:   {}", self.in_bootloader);
        println!("System mode:  {}", self.system_mode);
        println!("Profile:      {}", self.profile);
        println!("Polling rate: {} Hz", self.polling_hz);
    }
}

#[derive(Serialize, Debug)]
pub struct KeyOut {
    pub row: u8,
    pub col: u8,
    pub code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actuation_mm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rt_press_mm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rt_release_mm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_dz_mm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom_dz_mm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Millimeters with three decimals, e.g. `1.500`.
pub fn mm(m: openhe_proto::units::Micrometers) -> String {
    format!("{:.3}", m.to_mm())
}

impl KeyOut {
    pub fn ok(row: u8, col: u8, code: u8, s: &KeySettings) -> Self {
        KeyOut {
            row,
            col,
            code,
            touch: Some(format!("{:?}", s.mode.touch)),
            advanced: Some(format!("{:?}", s.mode.advanced)),
            actuation_mm: Some(mm(s.actuation)),
            rt_press_mm: Some(mm(s.rt_press)),
            rt_release_mm: Some(mm(s.rt_release)),
            top_dz_mm: Some(mm(s.top_dz)),
            bottom_dz_mm: Some(mm(s.bottom_dz)),
            switch: Some(s.switch),
            error: None,
        }
    }

    pub fn err(row: u8, col: u8, code: u8, msg: String) -> Self {
        KeyOut {
            row,
            col,
            code,
            touch: None,
            advanced: None,
            actuation_mm: None,
            rt_press_mm: None,
            rt_release_mm: None,
            top_dz_mm: None,
            bottom_dz_mm: None,
            switch: None,
            error: Some(msg),
        }
    }
}

#[derive(Serialize)]
pub struct DumpOut {
    #[serde(flatten)]
    pub info: InfoOut,
    /// 6 rows of 21 uppercase hex codes.
    pub matrix: Vec<Vec<String>>,
    pub keys: Vec<KeyOut>,
}

pub fn matrix_rows(m: &Matrix) -> Vec<Vec<String>> {
    m.codes
        .iter()
        .map(|row| row.iter().map(|c| format!("{c:02X}")).collect())
        .collect()
}

impl DumpOut {
    pub fn print(&self) {
        self.info.print();
        println!("\nMatrix:");
        for row in &self.matrix {
            println!("  {}", row.join(" "));
        }
        println!("\nKeys:");
        for k in &self.keys {
            let head = format!("  r{} c{:<2} code 0x{:02X}", k.row, k.col, k.code);
            if let Some(e) = &k.error {
                println!("{head}  error: {e}");
            } else {
                println!(
                    "{head}  {}/{}  act {} rt {}/{} dz {}/{}  switch {}",
                    k.touch.as_deref().unwrap_or("?"),
                    k.advanced.as_deref().unwrap_or("?"),
                    k.actuation_mm.as_deref().unwrap_or("?"),
                    k.rt_press_mm.as_deref().unwrap_or("?"),
                    k.rt_release_mm.as_deref().unwrap_or("?"),
                    k.top_dz_mm.as_deref().unwrap_or("?"),
                    k.bottom_dz_mm.as_deref().unwrap_or("?"),
                    k.switch.unwrap_or(0),
                );
            }
        }
    }
}

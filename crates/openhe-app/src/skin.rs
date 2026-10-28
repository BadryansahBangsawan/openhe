//! Keyboard colour skins ("Warna keyboard"): how the keyboard widget paints
//! the key caps, after the AULA WIN 68 / WIN 60 HE MAX colourways
//! (`docs/design/lunagraph/KeyboardSkin.tsx`).
//!
//! The keyboard does not report its colourway, so the user picks it once;
//! the choice is stored locally per keyboard (see
//! [`crate::storage::AppData::keyboard_skins`]).

use eframe::egui::Color32;
use openhe_proto::{keycodes, Model};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

/// A key cap colourway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyboardSkin {
    /// Abu Kuning: grey alphas, dark mods, amber accents.
    GreyYellow,
    /// Hitam Merah: black caps with a red contour-line artwork.
    BlackRed,
    /// Putih Biru: white alphas, grey mods, blue accents.
    WhiteBlue,
    /// Gelap: neutral dark caps (the look before a colour is picked).
    #[default]
    Gelap,
    /// Kustom: the top colour of each group; sides and legends follow.
    Custom {
        #[serde(with = "hex_rgb")]
        alpha: [u8; 3],
        #[serde(rename = "mod", with = "hex_rgb")]
        modifier: [u8; 3],
        #[serde(rename = "acc", with = "hex_rgb")]
        accent: [u8; 3],
    },
}

/// The custom colours offered first (mockup 08e).
pub const DEFAULT_CUSTOM: [[u8; 3]; 3] =
    [[0xD9, 0xD4, 0xC7], [0x3A, 0x3F, 0x4B], [0xE2, 0x72, 0x5B]];

/// Preset swatches of the custom editor (mockup 08e).
pub const PRESETS: [[u8; 3]; 8] = [
    [0xE2, 0x72, 0x5B],
    [0xF6, 0xB2, 0x32],
    [0x4B, 0x50, 0xC4],
    [0xC4, 0x23, 0x30],
    [0x3B, 0xA8, 0x6B],
    [0xE8, 0x8F, 0xB8],
    [0x7F, 0xB3, 0xDB],
    [0xF2, 0xEF, 0xEA],
];

impl KeyboardSkin {
    /// The four choices of the first-connect dialog, in its order.
    pub const PRESET_CHOICES: [KeyboardSkin; 4] = [
        KeyboardSkin::GreyYellow,
        KeyboardSkin::BlackRed,
        KeyboardSkin::WhiteBlue,
        KeyboardSkin::Gelap,
    ];

    pub fn custom(alpha: [u8; 3], modifier: [u8; 3], accent: [u8; 3]) -> KeyboardSkin {
        KeyboardSkin::Custom {
            alpha,
            modifier,
            accent,
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, KeyboardSkin::Custom { .. })
    }

    /// The custom colours (alpha, mod, accent), or the defaults.
    pub fn custom_colors(&self) -> [[u8; 3]; 3] {
        match *self {
            KeyboardSkin::Custom {
                alpha,
                modifier,
                accent,
            } => [alpha, modifier, accent],
            _ => DEFAULT_CUSTOM,
        }
    }

    /// One artwork across all caps (Hitam Merah).
    pub fn has_artwork(&self) -> bool {
        matches!(self, KeyboardSkin::BlackRed)
    }

    /// Case, bezel and the three cap groups.
    pub fn colors(&self) -> SkinColors {
        let cap = |top: u32, side: u32, legend: Color32| CapColors {
            top: hex(top),
            side: hex(side),
            legend,
        };
        match *self {
            KeyboardSkin::GreyYellow => SkinColors {
                case: hex(0x1B1B1D),
                bezel: hex(0x2B2B2E),
                alpha: cap(0xABAAA6, 0x74736F, hex(0xF5A21A)),
                modifier: cap(0x2E2E33, 0x1A1A1D, hex(0xECECEC)),
                accent: cap(0xF6B232, 0xC7861A, hex(0xFFFFFF)),
            },
            KeyboardSkin::WhiteBlue => SkinColors {
                case: hex(0xDCDCE1),
                bezel: hex(0xC4C4CB),
                alpha: cap(0xF2EFEA, 0xCFCBC4, hex(0x4A4FC0)),
                modifier: cap(0x8E8E93, 0x6D6D72, hex(0xFFFFFF)),
                accent: cap(0x4B50C4, 0x363A9E, hex(0xFFFFFF)),
            },
            KeyboardSkin::BlackRed => {
                // Legends are printed on the front of the caps, so they
                // show only faintly from above.
                let faint = Color32::from_white_alpha(89);
                let c = cap(0x1F2023, 0x111214, faint);
                SkinColors {
                    case: hex(0x141416),
                    bezel: hex(0x2A2A2D),
                    alpha: c,
                    modifier: c,
                    accent: c,
                }
            }
            KeyboardSkin::Gelap => {
                let c = cap(0x26272C, 0x18191C, hex(0xE8E8EA));
                SkinColors {
                    case: hex(0x0E0F11),
                    bezel: hex(0x26282C),
                    alpha: c,
                    modifier: c,
                    accent: c,
                }
            }
            KeyboardSkin::Custom {
                alpha,
                modifier,
                accent,
            } => SkinColors {
                case: hex(0x151517),
                bezel: hex(0x2A2A2D),
                alpha: custom_cap(alpha),
                modifier: custom_cap(modifier),
                accent: custom_cap(accent),
            },
        }
    }
}

/// Top face, side wall and legend colour of one cap group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapColors {
    pub top: Color32,
    pub side: Color32,
    pub legend: Color32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinColors {
    pub case: Color32,
    pub bezel: Color32,
    pub alpha: CapColors,
    pub modifier: CapColors,
    pub accent: CapColors,
}

impl SkinColors {
    pub fn cap(&self, group: KeyGroup) -> CapColors {
        match group {
            KeyGroup::Alpha => self.alpha,
            KeyGroup::Mod => self.modifier,
            KeyGroup::Acc => self.accent,
        }
    }
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// A custom cap: the side wall 0.7× the top, the legend black or white.
pub fn custom_cap(top: [u8; 3]) -> CapColors {
    let c = Color32::from_rgb(top[0], top[1], top[2]);
    CapColors {
        top: c,
        side: shade(c, 0.7),
        legend: legend_for(c),
    }
}

/// Each channel times `f`, rounded (the TSX `shade`).
pub fn shade(c: Color32, f: f32) -> Color32 {
    let m = |v: u8| (f32::from(v) * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// Black on light caps, white on dark ones (the TSX `legendFor`), with a
/// safeguard the TSX lacks: where its rule gives a contrast under 2.5
/// (saturated greens and cyans) the other one is used.
pub fn legend_for(c: Color32) -> Color32 {
    let (dark, light) = (hex(0x1A1A1C), hex(0xF4F4F5));
    let y = 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
    let (pick, other) = if y > 150.0 {
        (dark, light)
    } else {
        (light, dark)
    };
    if contrast(pick, c) < 2.5 {
        other
    } else {
        pick
    }
}

/// WCAG relative luminance of an opaque colour.
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio (1..=21) of two opaque colours.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `c` composited over the opaque `bg` (alpha taken into account).
pub fn over(c: Color32, bg: Color32) -> Color32 {
    let a = f32::from(c.a()) / 255.0;
    // Color32 is premultiplied.
    let m = |fg: u8, b: u8| {
        (f32::from(fg) + f32::from(b) * (1.0 - a))
            .round()
            .min(255.0) as u8
    };
    Color32::from_rgb(m(c.r(), bg.r()), m(c.g(), bg.g()), m(c.b(), bg.b()))
}

/// `c` (composited over `bg`), moved towards black or white, whichever
/// stands out on `bg`, until its contrast with `bg` is at least `min`.
pub fn readable(c: Color32, bg: Color32, min: f32) -> Color32 {
    let c = over(c, bg);
    if contrast(c, bg) >= min {
        return c;
    }
    let target = if luminance(bg) > 0.18 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let mut t = 0.1;
    while t < 1.0 {
        let m = c.lerp_to_gamma(target, t);
        if contrast(m, bg) >= min {
            return m;
        }
        t += 0.1;
    }
    target
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// "#D9D4C7", "d9d4c7" or "#abc" → the colour.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match s.len() {
        6 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
        }
        3 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            let d = |n: u32| ((n & 0xF) * 0x11) as u8;
            Some([d(v >> 8), d(v >> 4), d(v)])
        }
        _ => None,
    }
}

mod hex_rgb {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(c: &[u8; 3], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::to_hex(*c))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 3], D::Error> {
        let s = String::deserialize(d)?;
        super::parse_hex(&s).ok_or_else(|| serde::de::Error::custom("not a #RRGGBB colour"))
    }
}

/// Reads the stored skins leniently: a value this version does not know
/// (a newer skin, a hand edit) reads as [`KeyboardSkin::Gelap`] rather than
/// failing the whole data file; anything but a map reads as empty.
pub fn deserialize_skins<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, KeyboardSkin>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let serde_json::Value::Object(map) = v else {
        return Ok(BTreeMap::new());
    };
    Ok(map
        .into_iter()
        .map(|(k, v)| (k, serde_json::from_value(v).unwrap_or_default()))
        .collect())
}

/// The storage key of a keyboard: its serial, or the model when the
/// keyboard reports none.
pub fn skin_key(model: Model, serial: &str) -> String {
    let s = serial.trim();
    if s.is_empty() {
        format!("model:{model:?}")
    } else {
        s.to_string()
    }
}

// ------------------------------------------------------------ key groups

/// Which cap colour a key gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyGroup {
    Alpha,
    Mod,
    Acc,
}

/// Group of a key by its default (matrix) keycode: accents are Esc, Enter,
/// Space and the arrows; modifiers are Tab, Caps, Shift, Ctrl, Win, Alt,
/// Fn, Super/Menu, Backspace and the navigation column; the rest are
/// alphas.
pub fn key_group(code: u8) -> KeyGroup {
    match code {
        0x29 | 0x28 | 0x2C | 0x4F..=0x52 => KeyGroup::Acc,
        0x2B | 0x39 | 0xE0..=0xE7 | 0x01 | 0x65 | 0x2A | 0x49..=0x4E => KeyGroup::Mod,
        _ => KeyGroup::Alpha,
    }
}

/// A small symbol drawn before a modifier label (as shapes, so they show
/// without a font that has the glyph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapIcon {
    /// ← (Backspace)
    Back,
    /// ↵ (Enter)
    Return,
    /// ⇧ (Shift)
    Shift,
}

/// What is printed on a cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapLegend {
    /// One large character (letters).
    Char(&'static str),
    /// Shifted symbol above the base character (numbers, punctuation).
    TwoTier {
        shifted: &'static str,
        base: &'static str,
    },
    /// A small label, with an optional icon before it (modifiers, Esc).
    Label {
        icon: Option<CapIcon>,
        text: &'static str,
    },
    /// An arrow key (drawn as a shape), with its direction.
    Arrow(u8),
    /// Nothing (Space).
    Blank,
}

/// The shifted symbol of a number or punctuation key.
pub fn shifted(code: u8) -> Option<&'static str> {
    Some(match code {
        0x1E => "!",
        0x1F => "@",
        0x20 => "#",
        0x21 => "$",
        0x22 => "%",
        0x23 => "^",
        0x24 => "&",
        0x25 => "*",
        0x26 => "(",
        0x27 => ")",
        0x2D => "_",
        0x2E => "+",
        0x2F => "{",
        0x30 => "}",
        0x31 => "|",
        0x33 => ":",
        0x34 => "\"",
        0x35 => "~",
        0x36 => "<",
        0x37 => ">",
        0x38 => "?",
        _ => return None,
    })
}

/// The cap legend of a key, by its default keycode.
pub fn cap_legend(code: u8) -> CapLegend {
    let label = |text| CapLegend::Label { icon: None, text };
    match code {
        0x2A => CapLegend::Label {
            icon: Some(CapIcon::Back),
            text: "Backspace",
        },
        0x28 => CapLegend::Label {
            icon: Some(CapIcon::Return),
            text: "Enter",
        },
        0xE1 | 0xE5 => CapLegend::Label {
            icon: Some(CapIcon::Shift),
            text: "Shift",
        },
        0x39 => label("Caps Lock"),
        0x49 => label("Insert"),
        0x4C => label("Delete"),
        0x65 => label("Super"),
        0x2C => CapLegend::Blank,
        0x4F..=0x52 => CapLegend::Arrow(code),
        _ => {
            if let Some(s) = shifted(code) {
                return CapLegend::TwoTier {
                    shifted: s,
                    base: keycodes::legend(code),
                };
            }
            match key_group(code) {
                KeyGroup::Alpha => CapLegend::Char(keycodes::legend(code)),
                _ => label(keycodes::legend(code)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::keyboard::reference_matrix;

    fn rgb(c: Color32) -> u32 {
        (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b())
    }

    #[test]
    fn preset_colours_match_the_design() {
        let g = KeyboardSkin::GreyYellow.colors();
        assert_eq!(rgb(g.case), 0x1B1B1D);
        assert_eq!(
            (rgb(g.alpha.top), rgb(g.alpha.side), rgb(g.alpha.legend)),
            (0xABAAA6, 0x74736F, 0xF5A21A)
        );
        assert_eq!(rgb(g.modifier.top), 0x2E2E33);
        assert_eq!(
            (rgb(g.accent.top), rgb(g.accent.side)),
            (0xF6B232, 0xC7861A)
        );
        let w = KeyboardSkin::WhiteBlue.colors();
        assert_eq!(rgb(w.case), 0xDCDCE1);
        assert_eq!(rgb(w.alpha.legend), 0x4A4FC0);
        assert_eq!(rgb(w.accent.top), 0x4B50C4);
        let b = KeyboardSkin::BlackRed.colors();
        assert_eq!(b.alpha, b.accent);
        assert!(b.alpha.legend.a() < 128, "faint legends");
        assert!(KeyboardSkin::BlackRed.has_artwork());
        let d = KeyboardSkin::default();
        assert_eq!(d, KeyboardSkin::Gelap);
        assert_eq!(rgb(d.colors().alpha.top), 0x26272C);
    }

    #[test]
    fn custom_caps_shade_the_side_and_pick_a_readable_legend() {
        let s = KeyboardSkin::custom([0xD9, 0xD4, 0xC7], [0x3A, 0x3F, 0x4B], [0xE2, 0x72, 0x5B]);
        let c = s.colors();
        assert_eq!(rgb(c.alpha.top), 0xD9D4C7);
        // 0.7×, rounded: D9→98, D4→94, C7→8B.
        assert_eq!(rgb(c.alpha.side), 0x98948B);
        assert_eq!(rgb(c.modifier.side), 0x292C35);
        assert_eq!(rgb(c.alpha.legend), 0x1A1A1C, "dark legend on a light cap");
        assert_eq!(
            rgb(c.modifier.legend),
            0xF4F4F5,
            "light legend on a dark cap"
        );
        assert_eq!(rgb(c.accent.legend), 0xF4F4F5, "as in the mockup");
        // Pure green: the TSX rule alone would give white (contrast 1.3).
        assert_eq!(rgb(custom_cap([0, 0xFF, 0]).legend), 0x1A1A1C);
        assert_eq!(s.custom_colors()[2], [0xE2, 0x72, 0x5B]);
        assert_eq!(KeyboardSkin::Gelap.custom_colors(), DEFAULT_CUSTOM);
    }

    #[test]
    fn legends_contrast_with_custom_caps() {
        for v in (0..=255u16).step_by(15) {
            let v = v as u8;
            for top in [[v, v, v], [v, 0, 255 - v], [0, v, 0], [0, v, v]] {
                let cap = custom_cap(top);
                assert!(
                    contrast(cap.legend, cap.top) >= 2.5,
                    "{top:?}: {}",
                    contrast(cap.legend, cap.top)
                );
            }
        }
    }

    #[test]
    fn readable_moves_a_colour_until_it_stands_out() {
        let cream = hex(0xF2EFEA);
        let green = hex(0x4ADE80);
        assert!(contrast(green, cream) < 3.0);
        let g = readable(green, cream, 3.0);
        assert!(contrast(g, cream) >= 3.0);
        assert!(g.g() > g.r(), "still greenish: {g:?}");
        // Already readable: unchanged.
        assert_eq!(readable(green, hex(0x111214), 3.0), green);
        // A translucent colour is composited first.
        let soft = Color32::from_white_alpha(89);
        let on_dark = over(soft, hex(0x1F2023));
        assert!(on_dark.r() > 0x60 && on_dark.r() < 0x80, "{on_dark:?}");
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#D9D4C7"), Some([0xD9, 0xD4, 0xC7]));
        assert_eq!(parse_hex(" d9d4c7 "), Some([0xD9, 0xD4, 0xC7]));
        assert_eq!(parse_hex("#abc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
        assert_eq!(to_hex([0xE2, 0x72, 0x5B]), "#E2725B");
    }

    #[test]
    fn groups_follow_the_keycode() {
        use KeyGroup::*;
        for (code, g) in [
            (0x29, Acc),   // Esc
            (0x28, Acc),   // Enter
            (0x2C, Acc),   // Space
            (0x52, Acc),   // Up
            (0x50, Acc),   // Left
            (0x2B, Mod),   // Tab
            (0x39, Mod),   // Caps
            (0xE1, Mod),   // LShift
            (0xE5, Mod),   // RShift
            (0xE0, Mod),   // LCtrl
            (0xE3, Mod),   // Win
            (0xE6, Mod),   // RAlt
            (0x01, Mod),   // Fn
            (0x65, Mod),   // Super (WIN 60)
            (0x2A, Mod),   // Backspace
            (0x49, Mod),   // Ins
            (0x4C, Mod),   // Del
            (0x4B, Mod),   // PgUp
            (0x4E, Mod),   // PgDn
            (0x04, Alpha), // A
            (0x1E, Alpha), // 1
            (0x31, Alpha), // backslash
            (0x38, Alpha), // /
        ] {
            assert_eq!(key_group(code), g, "0x{code:02X}");
        }
    }

    #[test]
    fn win68_and_win60_group_counts() {
        let count = |m: Model, g: KeyGroup| {
            reference_matrix(m)
                .keys()
                .filter(|&(_, _, c)| key_group(c) == g)
                .count()
        };
        // Esc, Enter, Space + 4 arrows.
        assert_eq!(count(Model::Win68HeMax, KeyGroup::Acc), 7);
        // Tab Caps 2×Shift 2×Ctrl Win 2×Alt Fn Bksp Ins Del PgUp PgDn.
        assert_eq!(count(Model::Win68HeMax, KeyGroup::Mod), 15);
        assert_eq!(count(Model::Win68HeMax, KeyGroup::Alpha), 46);
        assert_eq!(count(Model::Win60HeMax, KeyGroup::Acc), 3);
        // Tab Caps 2×Shift 2×Ctrl Win 2×Alt Fn Super Bksp.
        assert_eq!(count(Model::Win60HeMax, KeyGroup::Mod), 12);
        assert_eq!(count(Model::Win60HeMax, KeyGroup::Alpha), 46);
    }

    #[test]
    fn legends() {
        assert_eq!(
            cap_legend(0x1E),
            CapLegend::TwoTier {
                shifted: "!",
                base: "1"
            }
        );
        assert_eq!(
            cap_legend(0x34),
            CapLegend::TwoTier {
                shifted: "\"",
                base: "'"
            }
        );
        assert_eq!(cap_legend(0x1A), CapLegend::Char("W"));
        assert_eq!(
            cap_legend(0x2A),
            CapLegend::Label {
                icon: Some(CapIcon::Back),
                text: "Backspace"
            }
        );
        assert_eq!(
            cap_legend(0x39),
            CapLegend::Label {
                icon: None,
                text: "Caps Lock"
            }
        );
        assert_eq!(
            cap_legend(0xE5),
            CapLegend::Label {
                icon: Some(CapIcon::Shift),
                text: "Shift"
            }
        );
        assert_eq!(
            cap_legend(0x49),
            CapLegend::Label {
                icon: None,
                text: "Insert"
            }
        );
        assert_eq!(
            cap_legend(0x29),
            CapLegend::Label {
                icon: None,
                text: "Esc"
            }
        );
        assert_eq!(cap_legend(0x2C), CapLegend::Blank);
        assert_eq!(cap_legend(0x51), CapLegend::Arrow(0x51));
    }

    #[test]
    fn serde_round_trip_and_tolerance() {
        let mut m = BTreeMap::new();
        m.insert("A".to_string(), KeyboardSkin::BlackRed);
        m.insert(
            "B".to_string(),
            KeyboardSkin::custom([1, 2, 3], [0xAA, 0xBB, 0xCC], [0xFF, 0, 0x10]),
        );
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r##""kind":"black_red""##), "{json}");
        assert!(json.contains(r##""mod":"#AABBCC""##), "{json}");
        let back: BTreeMap<String, KeyboardSkin> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);

        #[derive(Deserialize)]
        struct W {
            #[serde(deserialize_with = "deserialize_skins")]
            s: BTreeMap<String, KeyboardSkin>,
        }
        let w: W = serde_json::from_str(
            r##"{"s":{"x":{"kind":"neon"},"y":{"kind":"white_blue"},"z":{"kind":"custom","alpha":"nope"}}}"##,
        )
        .unwrap();
        assert_eq!(w.s["x"], KeyboardSkin::Gelap);
        assert_eq!(w.s["y"], KeyboardSkin::WhiteBlue);
        assert_eq!(w.s["z"], KeyboardSkin::Gelap);
        let w: W = serde_json::from_str(r#"{"s":5}"#).unwrap();
        assert!(w.s.is_empty());
    }

    #[test]
    fn storage_key_is_the_serial_or_the_model() {
        assert_eq!(skin_key(Model::Win68HeMax, " A68M24 "), "A68M24");
        assert_eq!(skin_key(Model::Win60HeMax, ""), "model:Win60HeMax");
    }
}

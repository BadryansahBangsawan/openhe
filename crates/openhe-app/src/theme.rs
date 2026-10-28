//! Colours, sizes and egui style for the dark theme of the mockups.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// Raw palette (mockups, Global Constraints).
pub mod color {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(0x0F, 0x10, 0x13);
    pub const PANEL: Color32 = Color32::from_rgb(0x16, 0x18, 0x1C);
    pub const SIDEBAR: Color32 = Color32::from_rgb(0x14, 0x16, 0x1A);
    pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x26, 0x2C);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x2C, 0x30, 0x38);
    pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xED, 0xEF);
    pub const MUTED: Color32 = Color32::from_rgb(0x8B, 0x90, 0x99);
    /// Dimmer than `MUTED`, for disabled text and scale labels.
    pub const FAINT: Color32 = Color32::from_rgb(0x5C, 0x61, 0x6B);
    pub const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xB2, 0x24);
    /// rgba(255,178,36,0.14), premultiplied.
    pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(36, 25, 5, 36);
    /// Text on an accent fill.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1A, 0x14, 0x05);
    /// Key cap fill and its hover.
    pub const KEY: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x22);
    pub const KEY_HOVER: Color32 = Color32::from_rgb(0x22, 0x25, 0x2B);
    /// Keyboard case behind the keys.
    pub const CASE: Color32 = Color32::from_rgb(0x0B, 0x0C, 0x0E);
    /// Segmented-control track and its selected segment.
    pub const TRACK: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x22);
    pub const SEGMENT_ON: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x34);
    pub const SUCCESS: Color32 = Color32::from_rgb(0x4A, 0xDE, 0x80);
    pub const DANGER: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
    pub const DANGER_SOFT: Color32 = Color32::from_rgba_premultiplied(35, 16, 16, 36);
    /// Values that differ from the factory default (mockup: blue-grey 1.80).
    pub const INFO: Color32 = Color32::from_rgb(0x8F, 0xA8, 0xD6);
}

/// Layout sizes in points.
pub mod size {
    pub const SIDEBAR: f32 = 232.0;
    pub const TOPBAR: f32 = 64.0;
    pub const RIGHT_PANEL: f32 = 348.0;
    pub const PAGE_MARGIN: f32 = 24.0;
    pub const CARD_RADIUS: u8 = 12;
    pub const CONTROL_RADIUS: u8 = 8;
    pub const KEY_RADIUS: u8 = 6;
    pub const MIN_WINDOW: [f32; 2] = [1280.0, 800.0];
    pub const START_WINDOW: [f32; 2] = [1440.0, 900.0];
}

/// The theme handed to pages through `AppCtx`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub bg: Color32,
    pub panel: Color32,
    pub sidebar: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub info: Color32,
}

impl Default for Theme {
    fn default() -> Self {
        use color::*;
        Theme {
            bg: BG,
            panel: PANEL,
            sidebar: SIDEBAR,
            border: BORDER,
            border_strong: BORDER_STRONG,
            text: TEXT,
            muted: MUTED,
            faint: FAINT,
            accent: ACCENT,
            accent_soft: ACCENT_SOFT,
            on_accent: ON_ACCENT,
            success: SUCCESS,
            danger: DANGER,
            info: INFO,
        }
    }
}

/// Font sizes.
pub mod font {
    use eframe::egui::{FontFamily, FontId};

    /// The bold family installed by [`super::apply`] (a system bold face
    /// when one is found, else egui's default font).
    pub fn bold_family() -> FontFamily {
        FontFamily::Name(super::BOLD_FAMILY.into())
    }
    pub fn bold(size: f32) -> FontId {
        FontId::new(size, bold_family())
    }
    pub fn title() -> FontId {
        bold(22.0)
    }
    pub fn heading() -> FontId {
        bold(17.0)
    }
    pub fn body() -> FontId {
        FontId::new(14.0, FontFamily::Proportional)
    }
    pub fn small() -> FontId {
        FontId::new(12.0, FontFamily::Proportional)
    }
    pub fn tiny() -> FontId {
        FontId::new(10.5, FontFamily::Proportional)
    }
    pub fn value() -> FontId {
        FontId::new(15.0, FontFamily::Monospace)
    }
}

/// Name of the bold font family (see [`font::bold_family`]).
pub const BOLD_FAMILY: &str = "bold";

/// System fonts used when present: regular, then bold. They are read from
/// the user's own system at startup and never shipped; egui's bundled fonts
/// stay as fallbacks for missing glyphs.
#[cfg(target_os = "macos")]
const SYSTEM_FONTS: (&[&str], &[&str]) = (
    &["/System/Library/Fonts/Supplemental/Arial.ttf"],
    &["/System/Library/Fonts/Supplemental/Arial Bold.ttf"],
);
#[cfg(target_os = "windows")]
const SYSTEM_FONTS: (&[&str], &[&str]) = (
    &["C:\\Windows\\Fonts\\segoeui.ttf"],
    &[
        "C:\\Windows\\Fonts\\seguisb.ttf",
        "C:\\Windows\\Fonts\\segoeuib.ttf",
    ],
);
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SYSTEM_FONTS: (&[&str], &[&str]) = (
    &[
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ],
    &[
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    ],
);

fn first_readable(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| std::fs::read(p).ok())
}

/// Installs fonts: a system sans as the proportional face when available,
/// and the [`BOLD_FAMILY`] family (always defined, so bold text never
/// panics even without system fonts).
pub fn install_fonts(ctx: &egui::Context) {
    use eframe::egui::{FontData, FontDefinitions};
    use std::sync::Arc;
    let mut defs = FontDefinitions::default();
    let (regular, bold) = SYSTEM_FONTS;
    if let Some(bytes) = first_readable(regular) {
        defs.font_data
            .insert("system".into(), Arc::new(FontData::from_owned(bytes)));
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system".into());
    }
    let mut bold_list = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(bytes) = first_readable(bold) {
        defs.font_data
            .insert("system-bold".into(), Arc::new(FontData::from_owned(bytes)));
        bold_list.insert(0, "system-bold".into());
    }
    defs.families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_list);
    ctx.set_fonts(defs);
}

/// Installs the fonts, dark visuals and text styles on `ctx`. Call once at
/// startup (and in UI tests) before drawing.
pub fn apply(ctx: &egui::Context) {
    use color::*;
    install_fonts(ctx);
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = PANEL;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = line(1.0, ACCENT);
    v.window_stroke = line(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(size::CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(size::CONTROL_RADIUS);
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.window_shadow = egui::epaint::Shadow::NONE;
    let r = CornerRadius::same(size::CONTROL_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = line(1.0, BORDER);
    w.noninteractive.fg_stroke = line(1.0, TEXT);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = TRACK;
    w.inactive.weak_bg_fill = TRACK;
    w.inactive.bg_stroke = line(1.0, BORDER_STRONG);
    w.inactive.fg_stroke = line(1.0, TEXT);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = KEY_HOVER;
    w.hovered.weak_bg_fill = KEY_HOVER;
    w.hovered.bg_stroke = line(1.0, MUTED);
    w.hovered.fg_stroke = line(1.0, TEXT);
    w.hovered.corner_radius = r;
    w.active.bg_fill = SEGMENT_ON;
    w.active.weak_bg_fill = SEGMENT_ON;
    w.active.bg_stroke = line(1.0, ACCENT);
    w.active.fg_stroke = line(1.0, TEXT);
    w.active.corner_radius = r;
    w.open = w.active;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, font::heading()),
            (TextStyle::Body, font::body()),
            (TextStyle::Button, font::body()),
            (TextStyle::Small, font::small()),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 30.0;
        s.interaction.selectable_labels = false;
        s.spacing.scroll = egui::style::ScrollStyle::floating();
    });
}

/// A stroke of `width` points (avoids float-literal inference fallbacks
/// with `Stroke::new`'s `impl Into<f32>`).
pub fn line(width: f32, c: Color32) -> Stroke {
    Stroke { width, color: c }
}

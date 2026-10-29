//! Choosing the keyboard's colours ("Warna keyboard"): the first-connect
//! dialog (mockup `08c-warna-dialog.png`), the chips of the Pengaturan row
//! (`08d-warna-pengaturan.png`) and the custom editor
//! (`08e-warna-kustom.png`).

use crate::skin::{parse_hex, to_hex, KeyboardSkin, DEFAULT_CUSTOM, PRESETS};
use crate::theme::{color, font, line};
use crate::widgets::keyboard::{KeyboardView, Selection};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Id, Layout, Margin, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, Ui,
};
use openhe_proto::defkey::Matrix;
use openhe_proto::Model;

pub mod strings {
    pub const GREY_YELLOW: &str = "Abu Kuning";
    pub const GREY_YELLOW_SUB: &str = "Grey Yellow";
    pub const BLACK_RED: &str = "Hitam Merah";
    pub const BLACK_RED_SUB: &str = "Black Red";
    pub const WHITE_BLUE: &str = "Putih Biru";
    pub const WHITE_BLUE_SUB: &str = "White Blue";
    pub const OTHER: &str = "Lainnya / gelap";
    pub const OTHER_SUB: &str = "Netral, untuk keycap custom";
    pub const GELAP: &str = "Gelap";
    pub const CUSTOM: &str = "Kustom…";

    pub const CONNECTED: &str = "TERSAMBUNG";
    pub const DIALOG_TITLE: &str = "Warna keyboard kamu yang mana?";
    pub const DIALOG_BODY: &str = "Keyboard tidak memberi tahu warnanya, jadi pilih sekali saja. Tampilan keyboard di app akan mengikuti. Bisa diganti kapan pun di Pengaturan.";
    pub const DIALOG_NOTE: &str = "Disimpan per keyboard (nomor seri) di komputer ini.";
    pub const LATER: &str = "Nanti saja";
    pub const USE: &str = "Pakai warna ini";

    pub const ROW_TITLE: &str = "Warna keyboard";
    pub const ROW_HINT: &str = "Tampilan keyboard di app. Keyboard tidak melaporkan warnanya, jadi dipilih manual. Disimpan untuk keyboard ini.";
    pub const ROW_OFFLINE: &str = "Sambungkan keyboard untuk memilih warnanya.";

    pub const EDITOR_PATH: &str = "PENGATURAN · WARNA KEYBOARD · KUSTOM";
    pub const EDITOR_TITLE: &str = "Keycap kustom";
    pub const EDITOR_BODY: &str = "Untuk keycap yang sudah diganti. Pilih warna atas tiga kelompok tombol; sisi keycap dan warna huruf dihitung otomatis supaya tetap terbaca.";
    pub const EDITOR_PREVIEW: &str =
        "Pratinjau langsung. Tombol yang dipilih tetap bergaris amber.";
    pub const EDITOR_COLORS: &str = "Warna";
    pub const FIELD_ALPHA: &str = "Tombol huruf & angka";
    pub const FIELD_MOD: &str = "Modifier (Tab, Shift, Ctrl…)";
    pub const FIELD_ACC: &str = "Aksen (Esc, Enter, Space, panah)";
    pub const PRESET: &str = "Warna contoh";
    pub const CANCEL: &str = "Batal";
    pub const SAVE: &str = "Simpan";
}
use strings as t;

/// Name of a skin (Settings chips, dialog cards).
pub fn skin_name(s: &KeyboardSkin) -> &'static str {
    match s {
        KeyboardSkin::GreyYellow => t::GREY_YELLOW,
        KeyboardSkin::BlackRed => t::BLACK_RED,
        KeyboardSkin::WhiteBlue => t::WHITE_BLUE,
        KeyboardSkin::Gelap => t::GELAP,
        KeyboardSkin::Custom { .. } => t::CUSTOM,
    }
}

fn model_name(m: Model) -> &'static str {
    match m {
        Model::Win68HeMax => "WIN 68 HE MAX",
        Model::Win60HeMax => "WIN 60 HE MAX",
    }
}

/// The colour dots of a chip, centred vertically at `left_mid`; returns
/// their width.
pub fn paint_dots(p: &egui::Painter, left_mid: Pos2, skin: &KeyboardSkin, r: f32) -> f32 {
    let ring = |c: Color32| {
        if crate::skin::luminance(c) < 0.03 {
            line(1.0, color::BORDER_STRONG)
        } else {
            Stroke::NONE
        }
    };
    let pair = |a: Color32, b: Color32| {
        let c1 = left_mid + egui::vec2(r, 0.0);
        let c2 = left_mid + egui::vec2(2.0 * r - 2.0, 0.0) + egui::vec2(r * 0.4, 0.0);
        p.circle(c1, r, a, ring(a));
        p.circle(c2, r, b, ring(b));
        c2.x + r - left_mid.x
    };
    let c = skin.colors();
    match skin {
        KeyboardSkin::GreyYellow => pair(c.alpha.top, c.accent.top),
        KeyboardSkin::BlackRed => pair(c.case, crate::widgets::skin_art::SWIRL_RED),
        KeyboardSkin::WhiteBlue => pair(c.alpha.top, c.accent.top),
        KeyboardSkin::Gelap => {
            p.circle(
                left_mid + egui::vec2(r, 0.0),
                r,
                c.alpha.top,
                line(1.0, color::BORDER_STRONG),
            );
            2.0 * r
        }
        KeyboardSkin::Custom { .. } => {
            // A colour wheel.
            let center = left_mid + egui::vec2(r, 0.0);
            let hues = [
                Color32::from_rgb(0xE2, 0x72, 0x5B),
                Color32::from_rgb(0xF6, 0xB2, 0x32),
                Color32::from_rgb(0x3B, 0xA8, 0x6B),
                Color32::from_rgb(0x7F, 0xB3, 0xDB),
                Color32::from_rgb(0x4B, 0x50, 0xC4),
                Color32::from_rgb(0xE8, 0x8F, 0xB8),
            ];
            let n = hues.len();
            for (i, h) in hues.iter().enumerate() {
                let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
                let mut pts = vec![center];
                for s in 0..=6 {
                    let a = a0 + (a1 - a0) * s as f32 / 6.0;
                    pts.push(center + r * egui::vec2(a.cos(), a.sin()));
                }
                p.add(egui::Shape::convex_polygon(pts, *h, Stroke::NONE));
            }
            2.0 * r
        }
    }
}

fn primary_button(ui: &mut Ui, text: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .font(font::bold(14.0))
                .color(color::ON_ACCENT),
        )
        .fill(color::ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(width, 40.0)),
    )
}

fn secondary_button(ui: &mut Ui, text: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .font(font::bold(14.0))
                .color(color::TEXT),
        )
        .fill(color::BG)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(width, 40.0)),
    )
}

fn modal_frame() -> Frame {
    Frame::new()
        .fill(color::PANEL)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(28))
}

/// A radio mark: an empty ring, or an amber disc with a check.
fn paint_radio(p: &egui::Painter, center: Pos2, on: bool) {
    let r = 11.0;
    if on {
        p.circle_filled(center, r, color::ACCENT);
        let s = line(2.0, color::ON_ACCENT);
        let a = center + egui::vec2(-4.5, 0.5);
        let b = center + egui::vec2(-1.2, 3.8);
        let c = center + egui::vec2(5.0, -3.8);
        p.line_segment([a, b], s);
        p.line_segment([b, c], s);
    } else {
        p.circle_stroke(center, r - 0.5, line(1.5, color::BORDER_STRONG));
    }
}

// ------------------------------------------------------- first connect

/// What the first-connect dialog decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogOutcome {
    /// "Pakai warna ini".
    Use(KeyboardSkin),
    /// "Nanti saja": the dark skin, and do not ask again.
    Later,
    /// Escape or a click outside: ask again next time.
    Dismissed,
}

/// The dialog shown when a keyboard connects whose colours were never
/// chosen. `choice` is the highlighted card.
pub fn first_connect_dialog(
    ctx: &egui::Context,
    model: Model,
    matrix: &Matrix,
    choice: &mut KeyboardSkin,
) -> Option<DialogOutcome> {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 120.0).clamp(560.0, 760.0);
    let mut out = None;
    let resp = egui::Modal::new(Id::new("skin-first-connect"))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.label(
                RichText::new(format!("{} {}", model_name(model), t::CONNECTED))
                    .font(font::bold(12.0))
                    .color(color::MUTED),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(t::DIALOG_TITLE)
                    .font(font::bold(22.0))
                    .color(color::TEXT),
            );
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(t::DIALOG_BODY).color(color::MUTED)).wrap());
            ui.add_space(14.0);
            let cards = [
                (KeyboardSkin::GreyYellow, t::GREY_YELLOW, t::GREY_YELLOW_SUB),
                (KeyboardSkin::BlackRed, t::BLACK_RED, t::BLACK_RED_SUB),
                (KeyboardSkin::WhiteBlue, t::WHITE_BLUE, t::WHITE_BLUE_SUB),
                (KeyboardSkin::Gelap, t::OTHER, t::OTHER_SUB),
            ];
            let gap = 12.0;
            let card_w = (width - gap) / 2.0;
            for row in cards.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (skin, name, sub) in row {
                        if option_card(
                            ui,
                            model,
                            matrix,
                            *skin,
                            name,
                            sub,
                            card_w,
                            *choice == *skin,
                        ) {
                            *choice = *skin;
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t::DIALOG_NOTE)
                        .font(font::small())
                        .color(color::MUTED),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary_button(ui, t::USE, 150.0).clicked() {
                        out = Some(DialogOutcome::Use(*choice));
                    }
                    if secondary_button(ui, t::LATER, 110.0).clicked() {
                        out = Some(DialogOutcome::Later);
                    }
                });
            });
        });
    if out.is_none() && resp.should_close() {
        out = Some(DialogOutcome::Dismissed);
    }
    out
}

/// One card of the dialog; true when clicked.
#[allow(clippy::too_many_arguments)]
fn option_card(
    ui: &mut Ui,
    model: Model,
    matrix: &Matrix,
    skin: KeyboardSkin,
    name: &str,
    sub: &str,
    width: f32,
    selected: bool,
) -> bool {
    let pad = 16.0;
    let inner_w = width - 2.0 * pad;
    let unit = (inner_w / 16.4).min(19.0);
    let thumb_h = 5.0 * unit + 2.0 * (unit * 0.26).clamp(3.0, 14.0);
    let height = pad + thumb_h + 14.0 + 42.0 + pad;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, name)
    });
    let (fill, stroke) = if selected {
        (
            crate::skin::over(color::ACCENT.gamma_multiply(0.08), color::PANEL),
            line(2.0, color::ACCENT),
        )
    } else if resp.hovered() {
        (color::KEY_HOVER, line(1.0, color::BORDER_STRONG))
    } else {
        (color::PANEL, line(1.0, color::BORDER_STRONG))
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(14),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let thumb = Rect::from_min_size(
        rect.min + egui::vec2(pad, pad),
        egui::vec2(inner_w, thumb_h),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(thumb)
            .layout(Layout::top_down(Align::Center)),
    );
    let mut sel = Selection::default();
    KeyboardView::new(model, matrix)
        .skin(skin)
        .picture()
        .min_unit(6.0)
        .max_unit(unit)
        .id_salt(match skin {
            KeyboardSkin::GreyYellow => "thumb-gy",
            KeyboardSkin::BlackRed => "thumb-br",
            KeyboardSkin::WhiteBlue => "thumb-wb",
            _ => "thumb-gelap",
        })
        .show(&mut child, &mut sel, |_| Default::default());
    let text_top = thumb.bottom() + 14.0;
    let p = ui.painter();
    p.text(
        egui::pos2(rect.min.x + pad + 2.0, text_top),
        Align2::LEFT_TOP,
        name,
        font::bold(16.0),
        color::TEXT,
    );
    p.text(
        egui::pos2(rect.min.x + pad + 2.0, text_top + 22.0),
        Align2::LEFT_TOP,
        sub,
        font::body(),
        color::MUTED,
    );
    paint_radio(
        p,
        egui::pos2(rect.max.x - pad - 12.0, text_top + 19.0),
        selected,
    );
    resp.clicked()
}

// ------------------------------------------------------------ chips

/// A chip clicked in the Settings row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipClick {
    Preset(KeyboardSkin),
    Custom,
}

/// The five chips: Abu Kuning, Hitam Merah, Putih Biru, Gelap, Kustom….
/// Labels never wrap; the chips wrap to a new line instead.
pub fn chips(ui: &mut Ui, current: Option<KeyboardSkin>, enabled: bool) -> Option<ChipClick> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        let all = [
            KeyboardSkin::GreyYellow,
            KeyboardSkin::BlackRed,
            KeyboardSkin::WhiteBlue,
            KeyboardSkin::Gelap,
            current
                .filter(KeyboardSkin::is_custom)
                .unwrap_or(KeyboardSkin::custom(
                    DEFAULT_CUSTOM[0],
                    DEFAULT_CUSTOM[1],
                    DEFAULT_CUSTOM[2],
                )),
        ];
        for skin in all {
            let on = match current {
                Some(c) if skin.is_custom() => c.is_custom(),
                Some(c) => c == skin,
                None => false,
            };
            if chip(ui, &skin, on, enabled).clicked() {
                clicked = Some(if skin.is_custom() {
                    ChipClick::Custom
                } else {
                    ChipClick::Preset(skin)
                });
            }
        }
    });
    clicked
}

fn chip(ui: &mut Ui, skin: &KeyboardSkin, on: bool, enabled: bool) -> egui::Response {
    let name = skin_name(skin);
    let font = if on {
        font::bold(14.0)
    } else {
        egui::FontId::proportional(14.0)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), font, color::TEXT);
    let dots_w = match skin {
        KeyboardSkin::Gelap | KeyboardSkin::Custom { .. } => 18.0,
        _ => 28.8,
    };
    let size = egui::vec2(14.0 + dots_w + 10.0 + galley.size().x + 16.0, 44.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, on, name)
    });
    let p = ui.painter();
    let (fill, stroke) = if on {
        (
            crate::skin::over(color::ACCENT.gamma_multiply(0.10), color::PANEL),
            line(2.0, color::ACCENT),
        )
    } else if resp.hovered() {
        (color::KEY_HOVER, line(1.0, color::MUTED))
    } else {
        (color::PANEL, line(1.0, color::BORDER_STRONG))
    };
    let radius = CornerRadius::same(10);
    if skin.is_custom() && !on {
        p.rect_filled(rect, radius, fill);
        dashed_rect(p, rect.shrink(0.5), 10.0, stroke);
    } else {
        p.rect(rect, radius, fill, stroke, StrokeKind::Inside);
    }
    let left_mid = egui::pos2(rect.min.x + 14.0, rect.center().y);
    paint_dots(p, left_mid, skin, 9.0);
    p.galley(
        egui::pos2(
            left_mid.x + dots_w + 10.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color::TEXT,
    );
    if !enabled {
        p.rect_filled(rect, radius, color::PANEL.gamma_multiply(0.5));
    }
    resp
}

fn dashed_rect(p: &egui::Painter, r: Rect, radius: f32, stroke: Stroke) {
    let sides = [
        [
            egui::pos2(r.left() + radius, r.top()),
            egui::pos2(r.right() - radius, r.top()),
        ],
        [
            egui::pos2(r.left() + radius, r.bottom()),
            egui::pos2(r.right() - radius, r.bottom()),
        ],
        [
            egui::pos2(r.left(), r.top() + radius),
            egui::pos2(r.left(), r.bottom() - radius),
        ],
        [
            egui::pos2(r.right(), r.top() + radius),
            egui::pos2(r.right(), r.bottom() - radius),
        ],
    ];
    for s in sides {
        p.extend(egui::Shape::dashed_line(&s, stroke, 5.0, 4.0));
    }
    // Corners as short arcs.
    let corners = [
        (
            egui::pos2(r.left() + radius, r.top() + radius),
            std::f32::consts::PI,
        ),
        (
            egui::pos2(r.right() - radius, r.top() + radius),
            1.5 * std::f32::consts::PI,
        ),
        (egui::pos2(r.right() - radius, r.bottom() - radius), 0.0),
        (
            egui::pos2(r.left() + radius, r.bottom() - radius),
            0.5 * std::f32::consts::PI,
        ),
    ];
    for (c, a0) in corners {
        let pts: Vec<Pos2> = (0..=6)
            .map(|i| {
                let a = a0 + std::f32::consts::FRAC_PI_2 * i as f32 / 6.0;
                c + radius * egui::vec2(a.cos(), a.sin())
            })
            .collect();
        p.add(egui::Shape::line(pts, stroke));
    }
}

// ------------------------------------------------------- custom editor

/// The custom editor's state: the three colours being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEditor {
    pub colors: [[u8; 3]; 3],
    hex: [String; 3],
    /// The field the preset swatches apply to.
    pub active: usize,
}

/// What the editor decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOutcome {
    Save(KeyboardSkin),
    Cancel,
}

impl CustomEditor {
    /// Starts from `skin`'s custom colours (or the defaults).
    pub fn new(skin: &KeyboardSkin) -> CustomEditor {
        let colors = skin.custom_colors();
        CustomEditor {
            colors,
            hex: colors.map(to_hex),
            active: 2,
        }
    }

    pub fn skin(&self) -> KeyboardSkin {
        KeyboardSkin::custom(self.colors[0], self.colors[1], self.colors[2])
    }

    /// Sets field `i` (picker or swatch); the hex text follows.
    pub fn set(&mut self, i: usize, c: [u8; 3]) {
        self.colors[i] = c;
        self.hex[i] = to_hex(c);
        self.active = i;
    }

    /// Typed hex for field `i`: applied once it parses.
    pub fn type_hex(&mut self, i: usize, text: &str) {
        self.hex[i] = text.to_string();
        if let Some(c) = parse_hex(text) {
            self.colors[i] = c;
        }
        self.active = i;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        model: Model,
        matrix: &Matrix,
    ) -> Option<EditorOutcome> {
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.0).clamp(640.0, 1160.0);
        let panel_w = 300.0;
        let mut out = None;
        let resp = egui::Modal::new(Id::new("skin-custom-editor"))
            .frame(modal_frame())
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    let left_w = width - panel_w - 28.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(left_w);
                            ui.label(
                                RichText::new(t::EDITOR_PATH)
                                    .font(font::bold(12.0))
                                    .color(color::MUTED),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(t::EDITOR_TITLE)
                                    .font(font::bold(22.0))
                                    .color(color::TEXT),
                            );
                            ui.add(
                                egui::Label::new(RichText::new(t::EDITOR_BODY).color(color::MUTED))
                                    .wrap(),
                            );
                            ui.add_space(14.0);
                            let mut sel = Selection::default();
                            KeyboardView::new(model, matrix)
                                .skin(self.skin())
                                .picture()
                                .min_unit(16.0)
                                .max_unit(50.0)
                                .id_salt("custom-preview")
                                .show(ui, &mut sel, |_| Default::default());
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(t::EDITOR_PREVIEW)
                                    .font(font::small())
                                    .color(color::MUTED),
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(panel_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            Frame::new()
                                .fill(color::BG)
                                .stroke(line(1.0, color::BORDER))
                                .corner_radius(CornerRadius::same(14))
                                .inner_margin(Margin::same(18))
                                .show(ui, |ui| {
                                    ui.set_width(panel_w - 36.0);
                                    ui.label(
                                        RichText::new(t::EDITOR_COLORS)
                                            .font(font::bold(16.0))
                                            .color(color::TEXT),
                                    );
                                    ui.add_space(8.0);
                                    for (i, label) in [t::FIELD_ALPHA, t::FIELD_MOD, t::FIELD_ACC]
                                        .iter()
                                        .enumerate()
                                    {
                                        self.field(ui, i, label);
                                        if i == self.active {
                                            self.presets(ui);
                                        }
                                        ui.add_space(6.0);
                                    }
                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        let bw = (ui.available_width() - 8.0) / 2.0;
                                        if secondary_button(ui, t::CANCEL, bw).clicked() {
                                            out = Some(EditorOutcome::Cancel);
                                        }
                                        if primary_button(ui, t::SAVE, bw).clicked() {
                                            out = Some(EditorOutcome::Save(self.skin()));
                                        }
                                    });
                                });
                        },
                    );
                });
            });
        if out.is_none() && resp.should_close() && !egui::Popup::is_any_open(ctx) {
            out = Some(EditorOutcome::Cancel);
        }
        out
    }

    /// "Label / [swatch  #RRGGBB]".
    fn field(&mut self, ui: &mut Ui, i: usize, label: &str) {
        ui.label(RichText::new(label).color(color::MUTED));
        let stroke = if i == self.active {
            line(2.0, color::ACCENT)
        } else {
            line(1.0, color::BORDER_STRONG)
        };
        let r = Frame::new()
            .fill(color::PANEL)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size = egui::vec2(28.0, 28.0);
                    let mut c = self.colors[i];
                    if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                        self.set(i, c);
                    }
                    let mut text = self.hex[i].clone();
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .id(Id::new(("skin-hex", i)))
                            .font(egui::FontId::monospace(15.0))
                            .frame(false)
                            .char_limit(7)
                            .desired_width(ui.available_width()),
                    );
                    if edit.changed() {
                        self.type_hex(i, &text);
                    }
                    if edit.gained_focus() {
                        self.active = i;
                    }
                    if edit.lost_focus() {
                        // Shown as the colour in use.
                        self.hex[i] = to_hex(self.colors[i]);
                    }
                });
            });
        if r.response.interact(Sense::click()).clicked() {
            self.active = i;
        }
    }

    /// The 8 preset swatches for the active field.
    fn presets(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let n = PRESETS.len() as f32;
            let w = ((ui.available_width() - 4.0 * (n - 1.0)) / n).floor();
            for (k, c) in PRESETS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click());
                let hex = to_hex(*c);
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("{} {}", t::PRESET, k + 1),
                    )
                });
                let col = Color32::from_rgb(c[0], c[1], c[2]);
                let on = self.colors[self.active] == *c;
                ui.painter().rect_filled(rect, CornerRadius::same(7), col);
                if on {
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(7),
                        line(2.0, color::TEXT),
                        StrokeKind::Inside,
                    );
                }
                if resp.on_hover_text(hex).clicked() {
                    let a = self.active;
                    self.set(a, *c);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_edits_and_parses_hex() {
        let mut e = CustomEditor::new(&KeyboardSkin::Gelap);
        assert_eq!(e.colors, DEFAULT_CUSTOM);
        e.type_hex(0, "#12");
        assert_eq!(e.colors[0], DEFAULT_CUSTOM[0], "incomplete hex is ignored");
        e.type_hex(0, "#123456");
        assert_eq!(e.colors[0], [0x12, 0x34, 0x56]);
        e.set(1, PRESETS[3]);
        assert_eq!(e.active, 1);
        assert_eq!(
            e.skin(),
            KeyboardSkin::custom([0x12, 0x34, 0x56], PRESETS[3], DEFAULT_CUSTOM[2])
        );
        let from = KeyboardSkin::custom([1, 2, 3], [4, 5, 6], [7, 8, 9]);
        assert_eq!(CustomEditor::new(&from).skin(), from);
    }

    #[test]
    fn names() {
        assert_eq!(skin_name(&KeyboardSkin::GreyYellow), "Abu Kuning");
        assert_eq!(skin_name(&KeyboardSkin::default()), "Gelap");
        assert_eq!(
            skin_name(&KeyboardSkin::custom([0; 3], [0; 3], [0; 3])),
            "Kustom…"
        );
    }
}

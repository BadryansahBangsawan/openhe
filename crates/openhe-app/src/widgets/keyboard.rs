//! The keyboard widget: real key geometry from the DEFKEY matrix, legends
//! from `keycodes::legend`, per-key decoration, and multi-key selection by
//! click, shift-click and drag-rectangle.
//!
//! Keys are identified by their default keycode (the `KEY` byte of every
//! per-key packet), which is what [`Selection`] stores.

use crate::i18n;
use crate::skin::{
    cap_legend, contrast, key_group, legend_for, luminance, over, readable, CapColors, CapIcon,
    CapLegend, KeyboardSkin,
};
use crate::theme::{color, font, line, size};
use crate::widgets::skin_art;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Pos2, Rect, Response, Sense,
    StrokeKind, Ui, Vec2,
};
use openhe_proto::defkey::{Matrix, COLS, ROWS};
use openhe_proto::keycodes;
use openhe_proto::Model;
use std::collections::BTreeSet;

// ---------------------------------------------------------------- geometry

/// One key cap, in key units (1u = one alpha key). Height is always 1u.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyGeom {
    pub code: u8,
    /// Matrix position (unfiltered DEFKEY row/col), for live travel later.
    pub row: u8,
    pub col: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
}

impl KeyGeom {
    fn rect_units(&self) -> Rect {
        Rect::from_min_size(egui::pos2(self.x, self.y), egui::vec2(self.w, 1.0))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyLayout {
    pub keys: Vec<KeyGeom>,
    /// Widest row, in units.
    pub width: f32,
    pub rows: usize,
    /// False for the WIN 60, whose real matrix has not been read yet.
    pub tested: bool,
}

/// Width of a key in units. The WIN 68 widths are confirmed (68% board); the
/// WIN 60 ones follow a standard 61-key ANSI board.
fn key_width(model: Model, code: u8, bottom_row: bool) -> f32 {
    match code {
        0x2A => 2.0,        // Backspace
        0x2B | 0x31 => 1.5, // Tab, backslash
        0x39 => 1.75,       // Caps
        0x28 => 2.25,       // Enter
        0xE1 => 2.25,       // LShift
        0xE5 => match model {
            Model::Win68HeMax => 1.75,
            Model::Win60HeMax => 2.75,
        },
        0x2C => 6.25,                                          // Space
        0xE0 | 0xE3 | 0xE2 => 1.25,                            // left Ctrl / Win / Alt
        _ if bottom_row && model == Model::Win60HeMax => 1.25, // right-hand mods
        _ => 1.0,
    }
}

/// Lays out the non-empty matrix rows top to bottom, keys in column order.
pub fn layout(model: Model, matrix: &Matrix) -> KeyLayout {
    let rows: Vec<usize> = (0..ROWS)
        .filter(|&r| matrix.codes[r].iter().any(|&c| c != 0))
        .collect();
    let mut keys = Vec::new();
    let mut width: f32 = 0.0;
    for (y, &r) in rows.iter().enumerate() {
        let bottom = y + 1 == rows.len();
        let mut x = 0.0;
        for col in 0..COLS {
            let code = matrix.codes[r][col];
            if code == 0 {
                continue;
            }
            let w = key_width(model, code, bottom);
            keys.push(KeyGeom {
                code,
                row: r as u8,
                col: col as u8,
                x,
                y: y as f32,
                w,
            });
            x += w;
        }
        width = width.max(x);
    }
    KeyLayout {
        keys,
        width,
        rows: rows.len(),
        tested: model == Model::Win68HeMax,
    }
}

fn matrix_from_rows(rows: &[&[u8]]) -> Matrix {
    let mut m = Matrix::default();
    for (r, row) in rows.iter().enumerate() {
        m.codes[r][..row.len()].copy_from_slice(row);
    }
    m
}

/// The matrix the keyboard is expected to report, for drawing a placeholder
/// while disconnected. WIN 68: read from hardware (PROTOCOL 4.1). WIN 60: a
/// guess (14/14/13/12/8 keys), unverified.
pub fn reference_matrix(model: Model) -> Matrix {
    match model {
        Model::Win68HeMax => matrix_from_rows(&[
            &[],
            &[
                0x29, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A,
                0x49,
            ],
            &[
                0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31,
                0x4C,
            ],
            &[
                0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x00, 0x28,
                0x4B,
            ],
            &[
                0xE1, 0x00, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xE5, 0x52,
                0x4E,
            ],
            &[
                0xE0, 0xE3, 0xE2, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0xE6, 0x01, 0xE4, 0x50, 0x51,
                0x4F,
            ],
        ]),
        Model::Win60HeMax => matrix_from_rows(&[
            &[],
            &[
                0x29, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A,
            ],
            &[
                0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31,
            ],
            &[
                0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x28,
            ],
            &[
                0xE1, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xE5,
            ],
            &[0xE0, 0xE3, 0xE2, 0x2C, 0xE6, 0x01, 0x65, 0xE4],
        ]),
    }
}

/// Codes of the keys whose caps intersect `area` (in units).
pub fn keys_in_rect(layout: &KeyLayout, area: Rect) -> Vec<u8> {
    layout
        .keys
        .iter()
        .filter(|k| k.rect_units().intersects(area))
        .map(|k| k.code)
        .collect()
}

/// The key under `p` (in units), if any.
pub fn key_at(layout: &KeyLayout, p: Pos2) -> Option<u8> {
    layout
        .keys
        .iter()
        .find(|k| k.rect_units().contains(p))
        .map(|k| k.code)
}

// --------------------------------------------------------------- selection

/// W, A, S, D.
pub const WASD: [u8; 4] = [0x1A, 0x04, 0x16, 0x07];

/// A set of selected keys, with the drag-rectangle state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    keys: BTreeSet<u8>,
    /// While dragging: what was selected before the drag (empty unless the
    /// drag is additive).
    drag_base: Option<BTreeSet<u8>>,
}

impl Selection {
    pub fn contains(&self, code: u8) -> bool {
        self.keys.contains(&code)
    }
    pub fn len(&self) -> usize {
        self.keys.len()
    }
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
    /// Selected codes in ascending order.
    pub fn codes(&self) -> Vec<u8> {
        self.keys.iter().copied().collect()
    }

    /// Selected codes in reading order of `matrix` (row by row, left to
    /// right), e.g. W, A, S, D; codes not in the matrix are left out.
    pub fn codes_in(&self, matrix: &Matrix) -> Vec<u8> {
        matrix
            .keys()
            .map(|(_, _, c)| c)
            .filter(|c| self.keys.contains(c))
            .collect()
    }
    pub fn as_set(&self) -> &BTreeSet<u8> {
        &self.keys
    }

    /// Plain click: select only `code`. Shift-click: toggle `code`.
    pub fn click(&mut self, code: u8, additive: bool) {
        if additive {
            if !self.keys.remove(&code) {
                self.keys.insert(code);
            }
        } else {
            self.keys.clear();
            self.keys.insert(code);
        }
    }

    /// Click on empty space: clears unless additive.
    pub fn click_empty(&mut self, additive: bool) {
        if !additive {
            self.keys.clear();
        }
    }

    pub fn begin_drag(&mut self, additive: bool) {
        let base = if additive {
            self.keys.clone()
        } else {
            BTreeSet::new()
        };
        self.keys = base.clone();
        self.drag_base = Some(base);
    }

    /// Selection becomes the drag base plus every key the rectangle touches.
    pub fn update_drag(&mut self, hit: impl IntoIterator<Item = u8>) {
        if let Some(base) = &self.drag_base {
            self.keys = base.iter().copied().chain(hit).collect();
        }
    }

    pub fn end_drag(&mut self) {
        self.drag_base = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_base.is_some()
    }

    /// Replaces the selection with `codes`.
    pub fn set(&mut self, codes: impl IntoIterator<Item = u8>) {
        self.keys = codes.into_iter().collect();
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// Every key of `matrix`.
    pub fn select_all(&mut self, matrix: &Matrix) {
        self.set(matrix.keys().map(|(_, _, c)| c));
    }

    /// W, A, S, D (those present in `matrix`).
    pub fn select_wasd(&mut self, matrix: &Matrix) {
        self.set(present(matrix, |c| WASD.contains(&c)));
    }

    /// A..Z (those present in `matrix`).
    pub fn select_letters(&mut self, matrix: &Matrix) {
        self.set(present(matrix, |c| (0x04..=0x1D).contains(&c)));
    }

    /// Drops keys that are not in `matrix` (after a reconnect to another board).
    pub fn retain_present(&mut self, matrix: &Matrix) {
        let all: BTreeSet<u8> = matrix.keys().map(|(_, _, c)| c).collect();
        self.keys.retain(|c| all.contains(c));
    }
}

fn present(matrix: &Matrix, f: impl Fn(u8) -> bool) -> Vec<u8> {
    matrix.keys().map(|(_, _, c)| c).filter(|&c| f(c)).collect()
}

/// Legends of `codes` joined for a heading, e.g. "W, A, S, D" or
/// "Q, W, E, R, T, Y +3".
pub fn legend_list(codes: &[u8], max: usize) -> String {
    let mut s = codes
        .iter()
        .take(max)
        .map(|&c| keycodes::legend(c))
        .collect::<Vec<_>>()
        .join(", ");
    if codes.len() > max {
        s.push_str(&format!(" +{}", codes.len() - max));
    }
    s
}

// ------------------------------------------------------------------ widget

/// Unit vector for the arrow keys (drawn as shapes, so they render even
/// when no installed font has the arrow glyphs).
fn arrow_dir(code: u8) -> Option<Vec2> {
    match code {
        0x4F => Some(egui::vec2(1.0, 0.0)),
        0x50 => Some(egui::vec2(-1.0, 0.0)),
        0x51 => Some(egui::vec2(0.0, 1.0)),
        0x52 => Some(egui::vec2(0.0, -1.0)),
        _ => None,
    }
}

fn paint_arrow(p: &egui::Painter, center: Pos2, len: f32, dir: Vec2, c: Color32) {
    let tip = center + dir * (len / 2.0);
    let tail = center - dir * (len / 2.0);
    let side = egui::vec2(-dir.y, dir.x) * (len * 0.32);
    let back = tip - dir * (len * 0.38);
    let stroke = line(1.6, c);
    p.line_segment([tail, tip], stroke);
    p.line_segment([tip, back + side], stroke);
    p.line_segment([tip, back - side], stroke);
}

/// The second line of a key cap: its text and, optionally, its own colour
/// (otherwise muted, or the accent on a selected key). Built from a string
/// with `.into()`; reads as `&str` (`label2.as_deref()`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Label2 {
    pub text: String,
    pub color: Option<Color32>,
}

impl Label2 {
    /// A second line in its own colour (e.g. a green "✔").
    pub fn colored(text: impl Into<String>, color: Color32) -> Label2 {
        Label2 {
            text: text.into(),
            color: Some(color),
        }
    }
}

impl From<&str> for Label2 {
    fn from(text: &str) -> Label2 {
        Label2 {
            text: text.to_string(),
            color: None,
        }
    }
}

impl From<String> for Label2 {
    fn from(text: String) -> Label2 {
        Label2 { text, color: None }
    }
}

impl std::ops::Deref for Label2 {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

/// One line of `text`, cut with "…" to `width`.
fn elided(text: &str, font: FontId, c: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, c, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job
}

/// Per-key extras drawn by the page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyDecoration {
    /// Second line under the legend (e.g. "1.20"), elided to the key width.
    pub label2: Option<Label2>,
    pub fill: Option<Color32>,
    pub stroke: Option<Color32>,
    /// Small pill in the top-right corner; a dot in that colour when the
    /// text is empty or the pill would touch the legend.
    pub badge: Option<(String, Color32)>,
}

pub struct KeyboardView {
    layout: KeyLayout,
    enabled: bool,
    interactive: bool,
    min_unit: f32,
    max_unit: f32,
    id_salt: &'static str,
    skin: Option<KeyboardSkin>,
}

const CASE_PAD: f32 = 14.0;

/// Amber selection outline width and its gap (in the case colour) from the
/// cap, so it shows on amber caps too.
const SELECT_W: f32 = 2.0;
const SELECT_GAP: f32 = 1.0;

impl KeyboardView {
    pub fn new(model: Model, matrix: &Matrix) -> KeyboardView {
        KeyboardView {
            layout: layout(model, matrix),
            enabled: true,
            interactive: true,
            min_unit: 28.0,
            max_unit: 64.0,
            id_salt: "keyboard",
            skin: None,
        }
    }

    /// Disabled: drawn dimmed, no selection changes.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Largest key size in points (default 64).
    pub fn max_unit(mut self, unit: f32) -> Self {
        self.max_unit = unit;
        self
    }

    /// Smallest key size in points (default 28).
    pub fn min_unit(mut self, unit: f32) -> Self {
        self.min_unit = unit;
        self
    }

    pub fn id_salt(mut self, salt: &'static str) -> Self {
        self.id_salt = salt;
        self
    }

    /// Paints the caps in `skin`'s colours (side wall, top face, printed
    /// legends). Without a skin the caps are flat (the Lampu page, whose
    /// fills are the RGB preview).
    pub fn skin(mut self, skin: KeyboardSkin) -> Self {
        self.skin = Some(skin);
        self
    }

    /// A picture only (thumbnails): no hover, no clicks, no per-key
    /// accessibility nodes, not dimmed.
    pub fn picture(mut self) -> Self {
        self.interactive = false;
        self
    }

    pub fn layout(&self) -> &KeyLayout {
        &self.layout
    }

    pub fn show(
        self,
        ui: &mut Ui,
        selection: &mut Selection,
        mut decorate: impl FnMut(u8) -> KeyDecoration,
    ) -> Response {
        let lay = &self.layout;
        let width_u = lay.width.max(1.0);
        let skin = self.skin.map(|s| (s, s.colors()));
        let case_pad = |unit: f32| {
            if skin.is_some() {
                (unit * 0.26).clamp(3.0, CASE_PAD)
            } else {
                CASE_PAD
            }
        };
        let mut unit = ((ui.available_width() - 2.0 * case_pad(self.max_unit)) / width_u)
            .clamp(self.min_unit, self.max_unit);
        if skin.is_some() {
            unit = ((ui.available_width() - 2.0 * case_pad(unit)) / width_u)
                .clamp(self.min_unit, self.max_unit);
        }
        let pad_case = case_pad(unit);
        let note_h = if lay.tested { 0.0 } else { 22.0 };
        let size = egui::vec2(
            width_u * unit + 2.0 * pad_case,
            lay.rows as f32 * unit + 2.0 * pad_case + note_h,
        );
        let interactive = self.enabled && self.interactive;
        let sense = if interactive {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let origin = rect.min + egui::vec2(pad_case, pad_case);
        let to_units = |p: Pos2| -> Pos2 { ((p - origin) / unit).to_pos2() };
        let id = Id::new(self.id_salt).with("drag-origin");

        // ---- interaction
        let mut drag_rect = None;
        if interactive {
            let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            if response.drag_started() {
                if let Some(o) = ui.input(|i| i.pointer.press_origin()) {
                    ui.data_mut(|d| d.insert_temp(id, o));
                    selection.begin_drag(additive);
                }
            }
            if response.dragged() {
                let start: Option<Pos2> = ui.data(|d| d.get_temp(id));
                if let (Some(a), Some(b)) = (start, response.interact_pointer_pos()) {
                    let area = Rect::from_two_pos(to_units(a), to_units(b));
                    selection.update_drag(keys_in_rect(lay, area));
                    drag_rect = Some(Rect::from_two_pos(a, b));
                }
            }
            if response.drag_stopped() {
                selection.end_drag();
                ui.data_mut(|d| d.remove::<Pos2>(id));
            }
            if response.clicked() {
                match response
                    .interact_pointer_pos()
                    .and_then(|p| key_at(lay, to_units(p)))
                {
                    Some(code) => selection.click(code, additive),
                    None => selection.click_empty(additive),
                }
            }
        }
        let hovered = response
            .hover_pos()
            .filter(|_| interactive)
            .and_then(|p| key_at(lay, to_units(p)));

        // ---- drawing
        let painter = ui.painter_at(rect.expand(SELECT_W + SELECT_GAP + 1.0));
        let board = Rect::from_min_size(rect.min, egui::vec2(size.x, size.y - note_h));
        match &skin {
            Some((_, sc)) => {
                let k = unit / 50.0;
                painter.rect(
                    board,
                    CornerRadius::same((10.0 * k).clamp(4.0, 16.0) as u8),
                    sc.case,
                    line(2.0 * k.clamp(0.5, 1.0), sc.bezel),
                    StrokeKind::Inside,
                );
            }
            None => {
                painter.rect(
                    rect,
                    CornerRadius::same(16),
                    color::CASE,
                    line(1.0, color::BORDER),
                    StrokeKind::Inside,
                );
            }
        }
        let gap = if skin.is_some() {
            (unit * 0.07).clamp(1.0, 4.0)
        } else {
            (unit * 0.09).clamp(3.0, 6.0)
        };
        let fs = (unit * 0.26).clamp(10.0, 14.0);
        let fs2 = (unit * 0.235).clamp(9.0, 12.5);
        let dim = |c: Color32| {
            if self.enabled {
                c
            } else {
                c.gamma_multiply(0.55)
            }
        };
        let key_rect = |k: &KeyGeom| {
            let min = origin + egui::vec2(k.x * unit + gap / 2.0, k.y * unit + gap / 2.0);
            Rect::from_min_size(min, egui::vec2(k.w * unit - gap, unit - gap))
        };
        let art = match &skin {
            Some((s, _)) if s.has_artwork() => {
                let rects: Vec<Rect> = lay.keys.iter().map(key_rect).collect();
                Some(skin_art::cached(
                    ui.ctx(),
                    Id::new(self.id_salt),
                    board,
                    &rects,
                    unit,
                ))
            }
            _ => None,
        };
        let mut selected_rects = Vec::new();
        for (ki, k) in lay.keys.iter().enumerate() {
            let r = key_rect(k);
            let deco = decorate(k.code);
            let selected = selection.contains(k.code);
            let is_hover = hovered == Some(k.code);
            // Each key is its own accessibility node (screen readers, UI
            // tests), named like its hover text; clicks go to the board.
            if self.interactive {
                ui.ctx()
                    .accesskit_node_builder(response.id.with(k.code), |n| {
                        n.set_role(egui::accesskit::Role::Button);
                        n.set_label(format!("{} · 0x{:02X}", keycodes::legend(k.code), k.code));
                        n.set_bounds(egui::accesskit::Rect {
                            x0: r.min.x.into(),
                            y0: r.min.y.into(),
                            x1: r.max.x.into(),
                            y1: r.max.y.into(),
                        });
                        if selected {
                            n.set_selected(true);
                        }
                        if !self.enabled {
                            n.set_disabled();
                        }
                    });
            }
            if let Some((_, sc)) = &skin {
                let cap = SkinnedCap {
                    rect: r,
                    unit,
                    fs2,
                    code: k.code,
                    colors: sc.cap(key_group(k.code)),
                    case: sc.case,
                    art: art.as_ref().map(|a| &a.keys[ki]),
                    selected,
                    hovered: is_hover,
                    enabled: self.enabled,
                    // Thumbnails keep the design's sizes; the board on a
                    // page keeps legends readable.
                    min_text: self.min_unit >= 20.0,
                };
                cap.paint(&painter, &deco);
                if selected {
                    selected_rects.push(r);
                }
                continue;
            }
            let fill = deco.fill.unwrap_or(if selected {
                Color32::from_rgb(0x33, 0x27, 0x10)
            } else if is_hover {
                color::KEY_HOVER
            } else {
                color::KEY
            });
            let (stroke_c, stroke_w) = if selected {
                (color::ACCENT, 1.5)
            } else if let Some(s) = deco.stroke {
                (s, 1.0)
            } else if is_hover {
                (color::MUTED, 1.0)
            } else {
                (color::BORDER_STRONG, 1.0)
            };
            painter.rect(
                r,
                CornerRadius::same(size::KEY_RADIUS),
                dim(fill),
                line(stroke_w, dim(stroke_c)),
                StrokeKind::Inside,
            );
            let pad = Vec2::new(
                (unit * 0.16).clamp(5.0, 10.0),
                (unit * 0.12).clamp(4.0, 8.0),
            );
            let text_c = if selected { color::ACCENT } else { color::TEXT };
            // Text stays inside the cap: the second line is elided to the
            // inner width, and the badge shrinks to a dot when it would
            // touch the legend.
            // Right margin smaller than the left: the text may run up to 3 px
            // from the cap's edge.
            let inner_w = (r.width() - pad.x - 3.0).max(4.0);
            let legend_right = if let Some(dir) = arrow_dir(k.code) {
                paint_arrow(
                    &painter,
                    r.min + pad + egui::vec2(fs * 0.45, fs * 0.6),
                    fs * 0.8,
                    dir,
                    dim(text_c),
                );
                r.min.x + pad.x + fs * 0.9
            } else {
                painter
                    .text(
                        r.min + pad,
                        Align2::LEFT_TOP,
                        keycodes::legend(k.code),
                        font::bold(fs),
                        dim(text_c),
                    )
                    .right()
            };
            if let Some(l2) = &deco.label2 {
                let c = match (l2.color, selected) {
                    (Some(c), _) => c,
                    (None, true) => color::ACCENT,
                    (None, false) => color::MUTED,
                };
                let galley = painter.layout_job(elided(
                    &l2.text,
                    FontId::new(fs2, FontFamily::Proportional),
                    dim(c),
                    inner_w,
                ));
                let pos = egui::pos2(r.min.x + pad.x, r.max.y - pad.y - galley.size().y);
                painter.galley(pos, galley, dim(c));
            }
            if let Some((text, c)) = &deco.badge {
                let font = FontId::new((fs2 - 2.0).max(8.0), FontFamily::Proportional);
                let galley = painter.layout_no_wrap(text.clone(), font, dim(*c));
                let bsize = galley.size() + egui::vec2(6.0, 2.0);
                let brect =
                    Rect::from_min_size(egui::pos2(r.max.x - bsize.x - 4.0, r.min.y + 4.0), bsize);
                if !text.is_empty() && brect.left() >= legend_right + 3.0 {
                    painter.rect_filled(brect, CornerRadius::same(4), dim(c.gamma_multiply(0.18)));
                    painter.galley(brect.min + egui::vec2(3.0, 1.0), galley, *c);
                } else {
                    let rad = (unit * 0.055).clamp(2.5, 3.5);
                    painter.circle_filled(
                        egui::pos2(r.max.x - rad - 5.0, r.min.y + rad + 5.0),
                        rad,
                        dim(*c),
                    );
                }
            }
        }
        // Selection outlines last, so neighbouring caps never cover them:
        // a ring in the case colour, then the amber line.
        if let Some((_, sc)) = &skin {
            let k = unit / 50.0;
            let radius = (6.0 * k).clamp(2.0, 6.0);
            for r in selected_rects {
                let ring = r.expand(SELECT_GAP);
                painter.rect_stroke(
                    ring,
                    CornerRadius::same((radius + SELECT_GAP) as u8),
                    line(SELECT_GAP, sc.case),
                    StrokeKind::Outside,
                );
                painter.rect_stroke(
                    ring.expand(SELECT_GAP),
                    CornerRadius::same((radius + 2.0 * SELECT_GAP) as u8),
                    line(SELECT_W, dim(color::ACCENT)),
                    StrokeKind::Outside,
                );
            }
        }
        if let Some(r) = drag_rect {
            painter.rect(
                r,
                CornerRadius::same(4),
                color::ACCENT_SOFT,
                line(1.0, color::ACCENT),
                StrokeKind::Inside,
            );
        }
        if !lay.tested {
            let bg = skin.as_ref().map_or(color::CASE, |(_, sc)| sc.case);
            painter.text(
                egui::pos2(rect.min.x + pad_case, rect.max.y - pad_case / 2.0 - 2.0),
                Align2::LEFT_BOTTOM,
                format!("⚠ {}", i18n::common::UNTESTED_LAYOUT),
                FontId::new(11.0, FontFamily::Proportional),
                readable(color::ACCENT, bg, 3.0),
            );
        }
        if let Some(code) = hovered {
            if !selection.is_dragging() {
                response
                    .clone()
                    .on_hover_text(format!("{} · 0x{code:02X}", keycodes::legend(code)));
            }
        }
        response
    }
}

/// One cap painted in a skin.
struct SkinnedCap<'a> {
    /// The whole cap (side wall).
    rect: Rect,
    unit: f32,
    /// Second-line font size.
    fs2: f32,
    code: u8,
    colors: CapColors,
    case: Color32,
    art: Option<&'a skin_art::KeyArt>,
    selected: bool,
    hovered: bool,
    enabled: bool,
    /// Keep legends at least ~9 pt (not for thumbnails).
    min_text: bool,
}

impl SkinnedCap<'_> {
    fn k(&self) -> f32 {
        self.unit / 50.0
    }

    /// The top face: inset 4/2/4/7 at a 50 pt unit.
    fn face(&self) -> Rect {
        let k = self.k();
        let r = self.rect;
        Rect::from_min_max(
            r.min + egui::vec2(4.0 * k, 2.0 * k),
            r.max - egui::vec2(4.0 * k, 7.0 * k),
        )
    }

    /// `base` × k, at least `min` on a page's board.
    fn text_size(&self, base: f32, min: f32) -> f32 {
        let s = base * self.k();
        if self.min_text {
            s.max(min)
        } else {
            s
        }
    }

    fn paint(&self, painter: &egui::Painter, deco: &KeyDecoration) {
        let k = self.k();
        let r = self.rect;
        let face = self.face();
        let wall_r = CornerRadius::same((6.0 * k).clamp(1.0, 6.0) as u8);
        let face_r = CornerRadius::same((5.0 * k).clamp(1.0, 5.0) as u8);
        let c = self.colors;

        // Face colour as decorations change it: a fill tints the cap.
        let mut top = c.top;
        if let Some(art) = self.art {
            // The artwork: dimmed on the wall, full on the face.
            skin_art::paint(painter, art, r);
            painter.rect_filled(r, wall_r, Color32::from_black_alpha(107));
            skin_art::paint(painter, art, face);
            top = skin_art::ART_BG;
        } else {
            painter.rect_filled(r, wall_r, c.side);
            painter.rect_filled(face, face_r, c.top);
        }
        if let Some(fill) = deco.fill {
            painter.rect_filled(face, face_r, fill);
            top = over(fill, top);
        }
        if self.hovered && !self.selected {
            let hover = if luminance(top) > 0.4 {
                Color32::from_black_alpha(18)
            } else {
                Color32::from_white_alpha(14)
            };
            painter.rect_filled(face, face_r, hover);
            top = over(hover, top);
        }
        // A thin highlight along the top edge of the face.
        if face.height() > 6.0 {
            painter.hline(
                face.x_range().shrink(3.0 * k),
                face.top() + 0.5,
                line(1.0, Color32::from_white_alpha(20)),
            );
        }
        if let Some(s) = deco.stroke {
            painter.rect_stroke(
                r,
                wall_r,
                line(1.5, readable(s, c.side, 2.0)),
                StrokeKind::Inside,
            );
        }

        // Legend colour: the printed one, unless a tint made it unreadable.
        let legend = if deco.fill.is_some() && contrast(over(c.legend, top), top) < 3.0 {
            legend_for(top)
        } else {
            c.legend
        };
        let pad = egui::vec2(6.0 * k, 4.0 * k);
        let inner = face;
        let text_w = (inner.width() - pad.x - 2.0 * k).max(4.0);
        let font = |s: f32| font::bold(s);

        // Second line (values, ✓, …), at the bottom of the face.
        let l2 = deco.label2.as_ref().map(|l2| {
            let col = match (l2.color, self.selected) {
                (Some(col), _) => readable(col, top, 3.0),
                (None, true) => readable(color::ACCENT, top, 3.0),
                (None, false) => readable(legend.gamma_multiply(0.8), top, 3.0),
            };
            let g = painter.layout_job(elided(
                &l2.text,
                FontId::new(
                    self.fs2.min(self.text_size(12.0, 8.0)),
                    FontFamily::Proportional,
                ),
                col,
                text_w,
            ));
            (g, col)
        });
        let l2_h = l2.as_ref().map_or(0.0, |(g, _)| g.size().y);
        let top_left = inner.min + pad;
        let mut legend_right = top_left.x;
        match cap_legend(self.code) {
            CapLegend::Blank => {}
            CapLegend::Char(s) => {
                let g =
                    painter.layout_job(elided(s, font(self.text_size(17.0, 12.0)), legend, text_w));
                legend_right = top_left.x + g.size().x;
                painter.galley(top_left, g, legend);
            }
            CapLegend::TwoTier { shifted, base } => {
                let size = self.text_size(13.0, 9.5);
                let line_h = size * 1.05;
                let room = inner.height() - pad.y - l2_h - 2.0 * k;
                if l2.is_none() || 2.0 * line_h <= room {
                    painter.text(top_left, Align2::LEFT_TOP, shifted, font(size), legend);
                    let r2 = painter.text(
                        top_left + egui::vec2(0.0, line_h),
                        Align2::LEFT_TOP,
                        base,
                        font(size),
                        legend,
                    );
                    legend_right = r2.right();
                } else {
                    // No room for both lines above the value: the base
                    // character alone, like a letter.
                    let r2 = painter.text(
                        top_left,
                        Align2::LEFT_TOP,
                        base,
                        font(self.text_size(17.0, 12.0)),
                        legend,
                    );
                    legend_right = r2.right();
                }
            }
            CapLegend::Label { icon, text } => {
                let mut size = self.text_size(11.0, 9.0);
                let icon_w = |s: f32| if icon.is_some() { s * 1.15 } else { 0.0 };
                // A label a little too wide ("Delete" on 1u) is set
                // slightly smaller rather than cut.
                let wide = painter
                    .layout_no_wrap(text.to_string(), font(size), legend)
                    .size()
                    .x;
                if wide > text_w - icon_w(size) {
                    size = (size * (text_w - icon_w(size)) / wide).max(size * 0.8);
                }
                let icon_w = icon_w(size);
                let g = painter.layout_job(elided(text, font(size), legend, text_w - icon_w));
                let h = g.size().y;
                // Centred vertically, or at the top above a second line.
                let y = if l2.is_some() {
                    top_left.y
                } else {
                    inner.center().y - h / 2.0
                };
                let mut x = top_left.x;
                if let Some(icon) = icon {
                    paint_icon(
                        painter,
                        icon,
                        egui::pos2(x, y + h / 2.0),
                        size * 0.72,
                        legend,
                    );
                    x += icon_w;
                }
                legend_right = x + g.size().x;
                painter.galley(egui::pos2(x, y), g, legend);
            }
            CapLegend::Arrow(code) => {
                if let Some(dir) = arrow_dir(code) {
                    let len = self.text_size(12.0, 9.0);
                    let center = if l2.is_some() {
                        egui::pos2(inner.center().x, top_left.y + len * 0.6)
                    } else {
                        inner.center()
                    };
                    paint_arrow(painter, center, len, dir, legend);
                    legend_right = center.x + len / 2.0;
                }
            }
        }
        if let Some((g, col)) = l2 {
            let pos = egui::pos2(
                inner.min.x + pad.x,
                inner.max.y - (2.0 * k).max(1.0) - g.size().y,
            );
            painter.galley(pos, g, col);
        }
        if let Some((text, col)) = &deco.badge {
            let size = self.text_size(10.0, 8.0);
            let g =
                painter.layout_no_wrap(text.clone(), FontId::proportional(size), legend_for(*col));
            let bsize = g.size() + egui::vec2(6.0, 2.0);
            let brect = Rect::from_min_size(
                egui::pos2(face.max.x - bsize.x - 2.0 * k, face.min.y + 2.0 * k),
                bsize,
            );
            if !text.is_empty() && brect.left() >= legend_right + 3.0 {
                painter.rect(
                    brect,
                    CornerRadius::same(4),
                    *col,
                    line(1.0, self.case),
                    StrokeKind::Outside,
                );
                painter.galley(brect.min + egui::vec2(3.0, 1.0), g, legend_for(*col));
            } else {
                // TSX "mark": a dot at right 6, top 4, size 6 (at 50 pt),
                // ringed in the case colour so it shows on any cap.
                let rad = if self.min_text {
                    (3.0 * k).clamp(2.5, 3.5)
                } else {
                    3.0 * k
                };
                let center = egui::pos2(r.max.x - 6.0 * k - rad, r.min.y + 4.0 * k + rad);
                painter.circle(center, rad, *col, line(1.0, self.case));
            }
        }
        if !self.enabled {
            painter.rect_filled(r.expand(0.5), wall_r, self.case.gamma_multiply(0.5));
        }
    }
}

/// The small symbols before modifier labels.
fn paint_icon(p: &egui::Painter, icon: CapIcon, left_mid: Pos2, s: f32, c: Color32) {
    let stroke = line((s * 0.14).clamp(0.6, 1.4), c);
    let x0 = left_mid.x;
    let y = left_mid.y;
    match icon {
        CapIcon::Back => {
            let tip = egui::pos2(x0, y);
            p.line_segment([tip, egui::pos2(x0 + s, y)], stroke);
            p.line_segment([tip, egui::pos2(x0 + s * 0.38, y - s * 0.34)], stroke);
            p.line_segment([tip, egui::pos2(x0 + s * 0.38, y + s * 0.34)], stroke);
        }
        CapIcon::Return => {
            let tip = egui::pos2(x0, y + s * 0.2);
            let corner = egui::pos2(x0 + s, y + s * 0.2);
            p.line_segment([tip, corner], stroke);
            p.line_segment([corner, egui::pos2(x0 + s, y - s * 0.4)], stroke);
            p.line_segment(
                [tip, egui::pos2(x0 + s * 0.32, y + s * 0.2 - s * 0.3)],
                stroke,
            );
            p.line_segment(
                [tip, egui::pos2(x0 + s * 0.32, y + s * 0.2 + s * 0.3)],
                stroke,
            );
        }
        CapIcon::Shift => {
            let w = s;
            let h = s * 1.05;
            let top = egui::pos2(x0 + w / 2.0, y - h / 2.0);
            let pts = vec![
                top,
                egui::pos2(x0 + w, y),
                egui::pos2(x0 + w * 0.72, y),
                egui::pos2(x0 + w * 0.72, y + h / 2.0),
                egui::pos2(x0 + w * 0.28, y + h / 2.0),
                egui::pos2(x0 + w * 0.28, y),
                egui::pos2(x0, y),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lines_are_elided_to_the_key_width() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});
        let font = FontId::new(10.0, FontFamily::Proportional);
        let g =
            ctx.fonts(|f| f.layout_job(elided("Kode 0xF344", font.clone(), Color32::WHITE, 30.0)));
        assert!(g.size().x <= 30.5, "{}", g.size().x);
        let last = g.rows[0].glyphs.last().map(|x| x.chr);
        assert_eq!(last, Some('…'));
        assert_eq!(g.rows.len(), 1);
        let g = ctx.fonts(|f| f.layout_job(elided("2.00", font, Color32::WHITE, 30.0)));
        let shown: String = g.rows[0].glyphs.iter().map(|x| x.chr).collect();
        assert_eq!(shown, "2.00");
    }

    #[test]
    fn a_second_line_can_carry_its_own_colour() {
        let plain: Label2 = "✔".into();
        assert_eq!((&*plain, plain.color), ("✔", None));
        let green = Label2::colored("✔", color::SUCCESS);
        assert_eq!(green.color, Some(color::SUCCESS));
        let d = KeyDecoration {
            label2: Some(green),
            ..KeyDecoration::default()
        };
        assert_eq!(d.label2.as_deref(), Some("✔"));
    }

    fn rows_of(l: &KeyLayout) -> Vec<Vec<KeyGeom>> {
        let mut rows = vec![Vec::new(); l.rows];
        for k in &l.keys {
            rows[k.y as usize].push(*k);
        }
        rows
    }

    #[test]
    fn win68_rows_are_15u_plus_the_right_column() {
        let l = layout(Model::Win68HeMax, &reference_matrix(Model::Win68HeMax));
        assert_eq!(l.keys.len(), 68);
        assert_eq!(l.rows, 5);
        assert!(l.tested);
        assert!((l.width - 16.0).abs() < 0.01);
        for (i, row) in rows_of(&l).iter().enumerate() {
            let total: f32 = row.iter().map(|k| k.w).sum();
            assert!((total - 16.0).abs() < 0.01, "row {i}: {total}u");
            let main: f32 = row.iter().filter(|k| k.x < 14.99).map(|k| k.w).sum();
            assert!((main - 15.0).abs() < 0.01, "row {i} main block: {main}u");
        }
        // Right-hand column: Ins, Del, PgUp, PgDn, then Right arrow.
        let right: Vec<u8> = l
            .keys
            .iter()
            .filter(|k| (k.x - 15.0).abs() < 0.01)
            .map(|k| k.code)
            .collect();
        assert_eq!(right, vec![0x49, 0x4C, 0x4B, 0x4E, 0x4F]);
    }

    #[test]
    fn win68_key_widths() {
        let l = layout(Model::Win68HeMax, &reference_matrix(Model::Win68HeMax));
        let w = |code: u8| l.keys.iter().find(|k| k.code == code).unwrap().w;
        for (code, want) in [
            (0x2A, 2.0),
            (0x2B, 1.5),
            (0x31, 1.5),
            (0x39, 1.75),
            (0x28, 2.25),
            (0xE1, 2.25),
            (0xE5, 1.75),
            (0xE0, 1.25),
            (0xE3, 1.25),
            (0xE2, 1.25),
            (0x2C, 6.25),
            (0x01, 1.0),
            (0x52, 1.0),
        ] {
            assert_eq!(w(code), want, "0x{code:02X}");
        }
        // Up arrow sits right of RShift, at 14u.
        let up = l.keys.iter().find(|k| k.code == 0x52).unwrap();
        assert!((up.x - 14.0).abs() < 0.01);
        // Matrix positions are kept for live travel.
        let fn_key = l.keys.iter().find(|k| k.code == 0x01).unwrap();
        assert_eq!((fn_key.row, fn_key.col), (5, 10));
    }

    #[test]
    fn keys_never_overlap_within_a_row() {
        for model in [Model::Win68HeMax, Model::Win60HeMax] {
            let l = layout(model, &reference_matrix(model));
            for row in rows_of(&l) {
                for pair in row.windows(2) {
                    assert!(pair[1].x >= pair[0].x + pair[0].w - 0.001);
                }
            }
        }
    }

    #[test]
    fn win60_best_effort_is_61_keys_of_15u_and_flagged() {
        let l = layout(Model::Win60HeMax, &reference_matrix(Model::Win60HeMax));
        assert_eq!(l.keys.len(), 61);
        assert!(!l.tested);
        let sizes: Vec<usize> = rows_of(&l).iter().map(|r| r.len()).collect();
        assert_eq!(sizes, vec![14, 14, 13, 12, 8]);
        for (i, row) in rows_of(&l).iter().enumerate() {
            let total: f32 = row.iter().map(|k| k.w).sum();
            assert!((total - 15.0).abs() < 0.01, "row {i}: {total}u");
        }
    }

    #[test]
    fn empty_matrix_gives_an_empty_layout() {
        let l = layout(Model::Win68HeMax, &Matrix::default());
        assert!(l.keys.is_empty());
        assert_eq!(l.rows, 0);
    }

    #[test]
    fn hit_testing() {
        let l = layout(Model::Win68HeMax, &reference_matrix(Model::Win68HeMax));
        assert_eq!(key_at(&l, egui::pos2(0.5, 0.5)), Some(0x29)); // Esc
        assert_eq!(key_at(&l, egui::pos2(14.9, 0.5)), Some(0x2A)); // Backspace (13..15)
        assert_eq!(key_at(&l, egui::pos2(5.0, 4.5)), Some(0x2C)); // Space
        assert_eq!(key_at(&l, egui::pos2(16.5, 0.5)), None);
        // A rectangle over the middle of Q..E and A..D.
        let area = Rect::from_two_pos(egui::pos2(2.0, 1.5), egui::pos2(3.9, 2.5));
        let mut hit = keys_in_rect(&l, area);
        hit.sort();
        // Row 2 (Tab 1.5): W 2.5..3.5, Q 1.5..2.5, E 3.5..4.5.
        // Row 3 (Caps 1.75): A 1.75..2.75, S 2.75..3.75, D 3.75..4.75.
        assert_eq!(hit, vec![0x04, 0x07, 0x08, 0x14, 0x16, 0x1A]);
    }

    #[test]
    fn click_and_shift_click() {
        let mut s = Selection::default();
        s.click(0x04, false);
        s.click(0x16, false);
        assert_eq!(s.codes(), vec![0x16]);
        s.click(0x04, true);
        assert_eq!(s.codes(), vec![0x04, 0x16]);
        s.click(0x16, true); // shift-click toggles off
        assert_eq!(s.codes(), vec![0x04]);
        s.click_empty(true);
        assert_eq!(s.len(), 1);
        s.click_empty(false);
        assert!(s.is_empty());
    }

    #[test]
    fn drag_replaces_or_adds() {
        let mut s = Selection::default();
        s.set([0x29]);
        s.begin_drag(false);
        assert!(s.is_dragging());
        s.update_drag([0x04, 0x16]);
        s.update_drag([0x04]); // rectangle shrank
        s.end_drag();
        assert_eq!(s.codes(), vec![0x04]);

        s.begin_drag(true); // shift-drag keeps what was there
        s.update_drag([0x1A, 0x07]);
        s.end_drag();
        assert_eq!(s.codes(), vec![0x04, 0x07, 0x1A]);
        assert!(!s.is_dragging());
        s.update_drag([0x29]); // ignored outside a drag
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn helpers_use_only_present_keys() {
        let m = reference_matrix(Model::Win68HeMax);
        let mut s = Selection::default();
        s.select_all(&m);
        assert_eq!(s.len(), 68);
        s.select_wasd(&m);
        assert_eq!(s.codes(), vec![0x04, 0x07, 0x16, 0x1A]);
        assert_eq!(s.codes_in(&m), WASD.to_vec(), "reading order: W, A, S, D");
        s.select_letters(&m);
        assert_eq!(s.len(), 26);
        s.clear();
        assert!(s.is_empty());

        let mut tiny = Matrix::default();
        tiny.codes[1][0] = 0x1A;
        s.select_wasd(&tiny);
        assert_eq!(s.codes(), vec![0x1A]);
        s.set([0x1A, 0x29]);
        s.retain_present(&tiny);
        assert_eq!(s.codes(), vec![0x1A]);
    }

    #[test]
    fn legend_lists() {
        assert_eq!(legend_list(&[0x1A, 0x04, 0x16, 0x07], 6), "W, A, S, D");
        assert_eq!(legend_list(&[0x04, 0x05, 0x06], 2), "A, B +1");
    }
}

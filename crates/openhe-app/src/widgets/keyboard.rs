//! The keyboard widget: real key geometry from the DEFKEY matrix, legends
//! from `keycodes::legend`, per-key decoration, and multi-key selection by
//! click, shift-click and drag-rectangle.
//!
//! Keys are identified by their default keycode (the `KEY` byte of every
//! per-key packet), which is what [`Selection`] stores.

use crate::i18n;
use crate::theme::{color, font, line, size};
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
    max_unit: f32,
    id_salt: &'static str,
}

const CASE_PAD: f32 = 14.0;

impl KeyboardView {
    pub fn new(model: Model, matrix: &Matrix) -> KeyboardView {
        KeyboardView {
            layout: layout(model, matrix),
            enabled: true,
            max_unit: 64.0,
            id_salt: "keyboard",
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

    pub fn id_salt(mut self, salt: &'static str) -> Self {
        self.id_salt = salt;
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
        let unit = ((ui.available_width() - 2.0 * CASE_PAD) / width_u).clamp(28.0, self.max_unit);
        let note_h = if lay.tested { 0.0 } else { 22.0 };
        let size = egui::vec2(
            width_u * unit + 2.0 * CASE_PAD,
            lay.rows as f32 * unit + 2.0 * CASE_PAD + note_h,
        );
        let sense = if self.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let origin = rect.min + egui::vec2(CASE_PAD, CASE_PAD);
        let to_units = |p: Pos2| -> Pos2 { ((p - origin) / unit).to_pos2() };
        let id = Id::new(self.id_salt).with("drag-origin");

        // ---- interaction
        let mut drag_rect = None;
        if self.enabled {
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
            .filter(|_| self.enabled)
            .and_then(|p| key_at(lay, to_units(p)));

        // ---- drawing
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect(
            rect,
            CornerRadius::same(16),
            color::CASE,
            line(1.0, color::BORDER),
            StrokeKind::Inside,
        );
        let gap = (unit * 0.09).clamp(3.0, 6.0);
        let fs = (unit * 0.26).clamp(10.0, 14.0);
        let fs2 = (unit * 0.235).clamp(9.0, 12.5);
        let dim = |c: Color32| {
            if self.enabled {
                c
            } else {
                c.gamma_multiply(0.55)
            }
        };
        for k in &lay.keys {
            let min = origin + egui::vec2(k.x * unit + gap / 2.0, k.y * unit + gap / 2.0);
            let r = Rect::from_min_size(min, egui::vec2(k.w * unit - gap, unit - gap));
            let deco = decorate(k.code);
            let selected = selection.contains(k.code);
            let is_hover = hovered == Some(k.code);
            // Each key is its own accessibility node (screen readers, UI
            // tests), named like its hover text; clicks go to the board.
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
            painter.text(
                egui::pos2(rect.min.x + CASE_PAD, rect.max.y - CASE_PAD / 2.0 - 2.0),
                Align2::LEFT_BOTTOM,
                format!("⚠ {}", i18n::common::UNTESTED_LAYOUT),
                FontId::new(11.0, FontFamily::Proportional),
                color::ACCENT,
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

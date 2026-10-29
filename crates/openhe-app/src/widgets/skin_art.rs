//! The Hitam Merah (Black Red) artwork: one picture across all caps, black
//! with thin red contour lines and a red swirl with dark contour lines
//! (`KeyboardSkin.tsx`, `Art`). Each cap shows its own window of it.
//!
//! egui fills only convex polygons, so the swirl is triangulated into a
//! mesh; the contour lines are ellipse polylines, cut per key into the runs
//! that cross that key. Everything is built once per board size and
//! position and kept in egui's temp data, so a frame only clones shapes.

use eframe::egui::{self, epaint::Vertex, Color32, Id, Mesh, Painter, Pos2, Rect, Shape, Stroke};
use std::sync::Arc;

/// Board background under the lines.
pub const ART_BG: Color32 = Color32::from_rgb(0x1F, 0x20, 0x23);
/// The swirl's red.
pub const SWIRL_RED: Color32 = Color32::from_rgb(0xC4, 0x23, 0x30);

/// The swirl outline, in fractions of the board (TSX `SWIRL`).
const SWIRL: [(f32, f32); 20] = [
    (0.00, 1.00),
    (0.00, 0.82),
    (0.24, 0.80),
    (0.40, 0.74),
    (0.48, 0.64),
    (0.55, 0.50),
    (0.58, 0.36),
    (0.52, 0.18),
    (0.45, 0.00),
    (0.50, 0.00),
    (0.57, 0.16),
    (0.61, 0.30),
    (0.63, 0.00),
    (0.74, 0.00),
    (0.67, 0.30),
    (0.70, 0.46),
    (0.76, 0.58),
    (0.84, 0.62),
    (1.00, 0.60),
    (1.00, 1.00),
];

/// The artwork pieces that fall on one key, in screen coordinates.
#[derive(Debug, Clone, Default)]
pub struct KeyArt {
    pub shapes: Vec<Shape>,
}

#[derive(Debug, Clone)]
pub struct Art {
    stamp: (Rect, Vec<Rect>),
    pub keys: Vec<KeyArt>,
}

/// Triangulates a simple polygon by ear clipping (indices into `pts`).
pub fn triangulate(pts: &[Pos2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let ccw = area > 0.0;
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b - a).x * (c - a).y - (b - a).y * (c - a).x;
    let convex = |a: Pos2, b: Pos2, c: Pos2| {
        let z = cross(a, b, c);
        if ccw {
            z > 0.0
        } else {
            z < 0.0
        }
    };
    let inside = |p: Pos2, a: Pos2, b: Pos2, c: Pos2| {
        let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    };
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if !convex(a, b, c) {
                continue;
            }
            let blocked = idx
                .iter()
                .filter(|&&j| j != ia && j != ib && j != ic)
                .any(|&j| inside(pts[j], a, b, c));
            if blocked {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

/// Even-odd point-in-polygon test.
pub fn point_in_polygon(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The swirl outline on `board`.
pub fn swirl_outline(board: Rect) -> Vec<Pos2> {
    SWIRL
        .iter()
        .map(|&(x, y)| board.min + egui::vec2(x * board.width(), y * board.height()))
        .collect()
}

/// Concentric ellipses (repeating radial gradient lines) centred at
/// `center`, x-radius step `step`, y/x ratio `ratio`, covering `board`.
fn rings(board: Rect, center: Pos2, ratio: f32, step: f32) -> Vec<Vec<Pos2>> {
    let far = [
        board.left_top(),
        board.right_top(),
        board.left_bottom(),
        board.right_bottom(),
    ]
    .iter()
    .map(|c| {
        let d = *c - center;
        (d.x * d.x + (d.y / ratio) * (d.y / ratio)).sqrt()
    })
    .fold(0.0_f32, f32::max);
    let mut out = Vec::new();
    let mut r = step - 0.6;
    let grown = board.expand(2.0);
    while r <= far + step {
        let ry = r * ratio;
        // Points about every 3 px along the ring (by its x-radius).
        let n = ((std::f32::consts::TAU * r) / 3.0).clamp(24.0, 4000.0) as usize;
        let mut run: Vec<Pos2> = Vec::new();
        for i in 0..=n {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            let p = center + egui::vec2(r * t.cos(), ry * t.sin());
            if grown.contains(p) {
                run.push(p);
            } else if !run.is_empty() {
                run.push(p);
                out.push(std::mem::take(&mut run));
            }
        }
        if run.len() > 1 {
            out.push(run);
        }
        r += step;
    }
    out
}

/// Adds every run of `lines` that crosses one of `keys` to that key's list.
fn cut_into_keys(
    lines: &[Vec<Pos2>],
    keys: &[Rect],
    keep: impl Fn(Pos2, Pos2) -> bool,
    out: &mut [Vec<Vec<Pos2>>],
) {
    // Keys bucketed by row (same top edge) for a quick y test.
    let mut rows: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        match rows.iter_mut().find(|r| (r.0 - k.top()).abs() < 0.5) {
            Some(r) => r.2.push(i),
            None => rows.push((k.top(), k.bottom(), vec![i])),
        }
    }
    let mut last: Vec<Option<Pos2>> = vec![None; keys.len()];
    for line in lines {
        last.iter_mut().for_each(|l| *l = None);
        for seg in line.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if !keep(a, b) {
                continue;
            }
            let bb = Rect::from_two_pos(a, b);
            for row in &rows {
                if bb.bottom() < row.0 - 1.0 || bb.top() > row.1 + 1.0 {
                    continue;
                }
                for &i in &row.2 {
                    if !keys[i].expand(1.0).intersects(bb) {
                        continue;
                    }
                    let runs = &mut out[i];
                    match last[i] {
                        Some(p) if p == a => runs.last_mut().expect("run").push(b),
                        _ => runs.push(vec![a, b]),
                    }
                    last[i] = Some(b);
                }
            }
        }
    }
}

/// Builds the artwork for a board at `board` with keys at `keys` (wall
/// rects), with lines `unit`-scaled.
pub fn build(board: Rect, keys: &[Rect], unit: f32) -> Art {
    let (w, h) = (board.width(), board.height());
    let sp = (w / 52.0).max(5.0);
    let step = sp + 1.2;
    let width = (unit / 50.0 * 1.2).clamp(0.6, 1.2);
    let red = Stroke::new(width, Color32::from_rgba_unmultiplied(196, 35, 48, 230));
    let dark = Stroke::new(width, Color32::from_rgba_unmultiplied(17, 18, 20, 217));

    let outline = swirl_outline(board);
    let mut mesh = Mesh::default();
    for p in &outline {
        mesh.vertices.push(Vertex {
            pos: *p,
            uv: egui::epaint::WHITE_UV,
            color: SWIRL_RED,
        });
    }
    for t in triangulate(&outline) {
        mesh.indices.extend(t.iter().map(|&i| i as u32));
    }
    let mesh = Arc::new(mesh);

    // Background lines: ellipse 260% x 150% at 60% 46%.
    let bg_lines = rings(
        board,
        board.min + egui::vec2(0.60 * w, 0.46 * h),
        (1.5 * h) / (2.6 * w),
        step,
    );
    // Swirl lines: ellipse 120% x 80% at 58% 40%, only inside the swirl.
    let swirl_lines = rings(
        board,
        board.min + egui::vec2(0.58 * w, 0.40 * h),
        (0.8 * h) / (1.2 * w),
        step,
    );
    let mut red_runs = vec![Vec::new(); keys.len()];
    let mut dark_runs = vec![Vec::new(); keys.len()];
    // Red lines outside the swirl only (the swirl covers them anyway).
    cut_into_keys(
        &bg_lines,
        keys,
        |a, b| !point_in_polygon(a + (b - a) * 0.5, &outline),
        &mut red_runs,
    );
    cut_into_keys(
        &swirl_lines,
        keys,
        |a, b| point_in_polygon(a + (b - a) * 0.5, &outline),
        &mut dark_runs,
    );
    let keys_art = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let mut shapes = vec![Shape::rect_filled(*k, 0.0, ART_BG)];
            shapes.extend(
                std::mem::take(&mut red_runs[i])
                    .into_iter()
                    .map(|r| Shape::line(r, red)),
            );
            let touches = (0..5).any(|i| {
                (0..5).any(|j| {
                    let p = k.min + egui::vec2(k.width() * i as f32, k.height() * j as f32) / 4.0;
                    point_in_polygon(p, &outline)
                })
            }) || outline.iter().any(|p| k.contains(*p));
            if touches {
                shapes.push(Shape::Mesh(mesh.clone()));
            }
            shapes.extend(
                std::mem::take(&mut dark_runs[i])
                    .into_iter()
                    .map(|r| Shape::line(r, dark)),
            );
            KeyArt { shapes }
        })
        .collect();
    Art {
        stamp: (board, keys.to_vec()),
        keys: keys_art,
    }
}

/// The artwork for this board, from egui's temp data when the board and
/// keys did not move.
pub fn cached(ctx: &egui::Context, id: Id, board: Rect, keys: &[Rect], unit: f32) -> Arc<Art> {
    let id = id.with("black-red-art");
    if let Some(a) = ctx.data(|d| d.get_temp::<Arc<Art>>(id)) {
        if a.stamp.0 == board && a.stamp.1 == keys {
            return a;
        }
    }
    let a = Arc::new(build(board, keys, unit));
    ctx.data_mut(|d| d.insert_temp(id, a.clone()));
    a
}

/// Paints one key's window of the artwork, clipped to `clip`.
pub fn paint(painter: &Painter, art: &KeyArt, clip: Rect) {
    let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
    p.extend(art.shapes.iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ear_clipping_covers_the_swirl() {
        let board = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 280.0));
        let pts = swirl_outline(board);
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), pts.len() - 2);
        let tri_area: f32 = tris
            .iter()
            .map(|t| {
                let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
            })
            .sum();
        let poly_area: f32 = (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                a.x * b.y - b.x * a.y
            })
            .sum::<f32>()
            .abs()
            / 2.0;
        assert!(
            (tri_area - poly_area).abs() < 1.0,
            "{tri_area} vs {poly_area}"
        );
        // Bottom-left is red, top-left is not; the arm reaches the top.
        assert!(point_in_polygon(egui::pos2(10.0, 270.0), &pts));
        assert!(!point_in_polygon(egui::pos2(10.0, 10.0), &pts));
        assert!(point_in_polygon(egui::pos2(0.68 * 800.0, 5.0), &pts));
    }

    #[test]
    fn every_key_gets_lines_and_runs_stay_near_it() {
        let board = Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(660.0, 220.0));
        let keys: Vec<Rect> = (0..5)
            .flat_map(|r| {
                (0..16).map(move |c| {
                    Rect::from_min_size(
                        egui::pos2(110.0 + c as f32 * 40.0, 60.0 + r as f32 * 40.0),
                        egui::vec2(37.0, 37.0),
                    )
                })
            })
            .collect();
        let art = build(board, &keys, 40.0);
        assert_eq!(art.keys.len(), keys.len());
        for (k, a) in keys.iter().zip(&art.keys) {
            let lines = a
                .shapes
                .iter()
                .filter(|s| matches!(s, Shape::Path(_)))
                .count();
            assert!(lines > 0, "no lines on {k:?}");
            for s in &a.shapes {
                if let Shape::Path(p) = s {
                    // Each run starts or ends within a segment of the key.
                    let near = p.points.iter().any(|q| k.expand(4.0).contains(*q));
                    assert!(near, "{k:?}");
                }
            }
        }
        // The bottom-left key shows the swirl, the top-left one does not.
        let has_mesh = |i: usize| {
            art.keys[i]
                .shapes
                .iter()
                .any(|s| matches!(s, Shape::Mesh(_)))
        };
        assert!(has_mesh(64));
        assert!(!has_mesh(0));
    }

    #[test]
    fn cache_is_reused_until_the_board_moves() {
        let ctx = egui::Context::default();
        let board = Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 100.0));
        let keys = vec![Rect::from_min_size(
            egui::pos2(5.0, 5.0),
            egui::vec2(18.0, 18.0),
        )];
        let a = cached(&ctx, Id::new("t"), board, &keys, 19.0);
        let b = cached(&ctx, Id::new("t"), board, &keys, 19.0);
        assert!(Arc::ptr_eq(&a, &b));
        let moved = board.translate(egui::vec2(0.0, 10.0));
        let c = cached(&ctx, Id::new("t"), moved, &keys, 19.0);
        assert!(!Arc::ptr_eq(&a, &c));
    }
}

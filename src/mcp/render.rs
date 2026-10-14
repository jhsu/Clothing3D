//! Off-screen PNG rendering of the pattern for agents, with point, segment
//! and seam index labels so the picture can be matched to tool arguments.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

use crate::model::{Pattern, PointKind, Vec2};
use crate::units::Units;

pub struct RenderOptions {
    /// Only this piece (and seams touching it), or everything.
    pub piece: Option<usize>,
    pub point_labels: bool,
    pub segment_labels: bool,
    pub seams: bool,
    /// Longest image side in pixels.
    pub size: u32,
    pub units: Units,
}

const MARGIN: f32 = 40.0;
const FONT_PX: f32 = 13.0;

struct Canvas {
    pixmap: Pixmap,
    font: FontRef<'static>,
    lo: Vec2,
    scale: f64,
    height: f32,
}

impl Canvas {
    fn to_px(&self, p: Vec2) -> (f32, f32) {
        let x = (p.x - self.lo.x) * self.scale;
        let y = (p.y - self.lo.y) * self.scale;
        (MARGIN + x as f32, self.height - MARGIN - y as f32) // Y up
    }

    fn path(&self, pts: &[Vec2], close: bool) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        let (x, y) = self.to_px(*pts.first()?);
        pb.move_to(x, y);
        for p in &pts[1..] {
            let (x, y) = self.to_px(*p);
            pb.line_to(x, y);
        }
        if close {
            pb.close();
        }
        pb.finish()
    }

    fn stroke(&mut self, path: &tiny_skia::Path, color: Color, width: f32, dash: bool) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let mut stroke = Stroke { width, ..Stroke::default() };
        if dash {
            stroke.dash = tiny_skia::StrokeDash::new(vec![6.0, 4.0], 0.0);
        }
        self.pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
    }

    fn fill(&mut self, path: &tiny_skia::Path, color: Color) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        self.pixmap.fill_path(path, &paint, FillRule::EvenOdd, Transform::identity(), None);
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), color: Color, width: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(path) = pb.finish() {
            self.stroke(&path, color, width, false);
        }
    }

    fn text_width(&self, text: &str, px: f32) -> f32 {
        let font = self.font.as_scaled(PxScale::from(px));
        text.chars().map(|c| font.h_advance(font.glyph_id(c))).sum()
    }

    /// Draw `text` centered on `(cx, cy)` over a translucent white backing.
    fn label(&mut self, text: &str, (cx, cy): (f32, f32), color: Color, px: f32) {
        let w = self.text_width(text, px);
        let font = self.font.as_scaled(PxScale::from(px));
        let (ascent, descent) = (font.ascent(), font.descent());
        let h = ascent - descent;
        let (x0, top) = (cx - w / 2.0, cy - h / 2.0);
        if let Some(rect) = tiny_skia::Rect::from_xywh(x0 - 2.0, top - 1.0, w + 4.0, h + 2.0) {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(255, 255, 255, 210));
            self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
        let mut x = x0;
        let baseline = top + ascent;
        for c in text.chars() {
            let id = font.glyph_id(c);
            let glyph = id.with_scale_and_position(px, ab_glyph::point(x, baseline));
            x += font.h_advance(id);
            let Some(outlined) = self.font.outline_glyph(glyph) else { continue };
            let b = outlined.px_bounds();
            let (pw, ph) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
            let pixels = self.pixmap.pixels_mut();
            outlined.draw(|gx, gy, cov| {
                let (px_x, px_y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px_x < 0 || px_y < 0 || px_x >= pw || px_y >= ph {
                    return;
                }
                let dst = &mut pixels[(px_y * pw + px_x) as usize];
                *dst = blend(*dst, color, cov.clamp(0.0, 1.0));
            });
        }
    }
}

/// Source-over blend of `color` at `coverage` onto a premultiplied pixel.
fn blend(dst: PremultipliedColorU8, color: Color, coverage: f32) -> PremultipliedColorU8 {
    let a = color.alpha() * coverage;
    let mix = |d: u8, s: f32| (s * a * 255.0 + d as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    let alpha = mix(dst.alpha(), 1.0);
    PremultipliedColorU8::from_rgba(
        mix(dst.red(), color.red()).min(alpha),
        mix(dst.green(), color.green()).min(alpha),
        mix(dst.blue(), color.blue()).min(alpha),
        alpha,
    )
    .unwrap_or(dst)
}

fn seam_color(seam: usize) -> Color {
    const PALETTE: [(u8, u8, u8); 6] =
        [(220, 50, 47), (38, 139, 210), (133, 153, 0), (211, 54, 130), (203, 75, 22), (42, 161, 152)];
    let (r, g, b) = PALETTE[seam % PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// A "nice" scale-bar length in mm, near a fifth of `span_mm`.
fn scale_bar(units: Units, span_mm: f64) -> (f64, String) {
    let target = span_mm / 5.0;
    let steps: &[f64] = match units {
        Units::Millimeters => &[1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0],
        Units::Inches => &[0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 6.0, 12.0, 24.0, 36.0, 72.0],
    };
    let unit = units.to_mm(1.0);
    let v = steps.iter().copied().rev().find(|s| s * unit <= target).unwrap_or(steps[0]);
    let label = match units {
        Units::Millimeters if v >= 10.0 => format!("{} cm", v / 10.0),
        Units::Millimeters => format!("{v} mm"),
        Units::Inches => format!("{v} in"),
    };
    (v * unit, label)
}

/// Render the pattern to PNG bytes.
pub fn render_png(pattern: &Pattern, opts: &RenderOptions) -> Result<Vec<u8>, String> {
    let shown: Vec<usize> = match opts.piece {
        Some(i) => vec![i],
        None => (0..pattern.pieces.len()).collect(),
    };
    let (lo, hi) = shown
        .iter()
        .filter_map(|&i| pattern.pieces[i].outer_bounds())
        .reduce(|(a, b), (c, d)| (Vec2::new(a.x.min(c.x), a.y.min(c.y)), Vec2::new(b.x.max(d.x), b.y.max(d.y))))
        .ok_or("nothing to render: the pattern (or piece) has no points")?;
    let span = Vec2::new((hi.x - lo.x).max(1.0), (hi.y - lo.y).max(1.0));
    let size = opts.size.clamp(256, 2048) as f64;
    let inner = size - 2.0 * MARGIN as f64;
    let scale = inner / span.x.max(span.y);
    let width = (span.x * scale + 2.0 * MARGIN as f64).ceil() as u32;
    let height = (span.y * scale + 2.0 * MARGIN as f64).ceil() as u32 + 24; // room for the scale bar
    let mut pixmap = Pixmap::new(width, height).ok_or("bad image size")?;
    pixmap.fill(Color::WHITE);
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).map_err(|e| e.to_string())?;
    let mut c = Canvas { pixmap, font, lo, scale, height: height as f32 - 24.0 };

    let ink = Color::from_rgba8(30, 30, 30, 255);
    let seg_ink = Color::from_rgba8(30, 90, 200, 255);

    for &i in &shown {
        let piece = &pattern.pieces[i];
        let cut = piece.cut_line();
        if let Some(path) = c.path(&cut, true) {
            c.fill(&path, Color::from_rgba8(246, 242, 232, 255));
            c.stroke(&path, ink, 1.0, false);
        }
        let outline = piece.flatten();
        if let Some(path) = c.path(&outline, piece.closed) {
            if piece.closed {
                c.fill(&path, Color::from_rgba8(235, 228, 210, 255));
            }
            c.stroke(&path, ink, 1.5, false);
        }
    }

    if opts.seams {
        for (si, seam) in pattern.seams.iter().enumerate() {
            let spans = seam.spans();
            if !spans.iter().any(|s| shown.contains(&s.piece)) {
                continue;
            }
            let color = seam_color(si);
            for span in spans {
                if !shown.contains(&span.piece) {
                    continue;
                }
                let pts = span.polyline(&pattern.pieces);
                if let Some(path) = c.path(&pts, false) {
                    c.stroke(&path, color, 4.0, false);
                }
                // Start notch: a short tick across the edge.
                if pts.len() >= 2 {
                    let (a, b) = (c.to_px(pts[0]), c.to_px(pts[1]));
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
                    let (nx, ny) = (-dy / len * 8.0, dx / len * 8.0);
                    c.line((a.0 - nx, a.1 - ny), (a.0 + nx, a.1 + ny), color, 2.5);
                }
                let mid = crate::sewing::point_at_fraction(&pts, 0.5);
                let (x, y) = c.to_px(mid);
                c.label(&format!("seam {si}"), (x, y - 12.0), color, FONT_PX);
            }
        }
    }

    for &i in &shown {
        let piece = &pattern.pieces[i];
        if opts.segment_labels {
            for s in 0..piece.segment_count() {
                let (x, y) = c.to_px(piece.eval_segment(s, 0.5));
                c.label(&format!("s{s}"), (x, y), seg_ink, FONT_PX - 1.0);
            }
        }
        for (j, pt) in piece.points.iter().enumerate() {
            let (x, y) = c.to_px(pt.pos);
            let mut pb = PathBuilder::new();
            match pt.kind {
                PointKind::Turn => {
                    if let Some(r) = tiny_skia::Rect::from_xywh(x - 3.0, y - 3.0, 6.0, 6.0) {
                        pb.push_rect(r);
                    }
                }
                PointKind::Curve => pb.push_circle(x, y, 3.5),
            }
            if let Some(path) = pb.finish() {
                c.fill(&path, ink);
            }
            if opts.point_labels {
                c.label(&j.to_string(), (x + 9.0, y - 9.0), ink, FONT_PX);
            }
        }
        let center = piece.bounds().map(|(a, b)| (a + b) * 0.5).unwrap_or_default();
        let (x, y) = c.to_px(center);
        c.label(&format!("{i}: {}", piece.name), (x, y), ink, FONT_PX + 3.0);
    }

    // Scale bar along the bottom.
    let (bar_mm, bar_label) = scale_bar(opts.units, span.x.max(span.y));
    let y = height as f32 - 16.0;
    let x0 = MARGIN;
    let x1 = MARGIN + (bar_mm * scale) as f32;
    c.line((x0, y), (x1, y), ink, 2.0);
    c.line((x0, y - 5.0), (x0, y + 5.0), ink, 2.0);
    c.line((x1, y - 5.0), (x1, y + 5.0), ink, 2.0);
    let w = c.text_width(&bar_label, FONT_PX);
    c.label(&bar_label, (x1 + 8.0 + w / 2.0, y), ink, FONT_PX);

    c.pixmap.encode_png().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::MM_PER_INCH;

    #[test]
    fn renders_example_png() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/sewing.json")).unwrap();
        let pattern: Pattern = serde_json::from_str(&text).unwrap();
        let opts = RenderOptions {
            piece: None,
            point_labels: true,
            segment_labels: true,
            seams: true,
            size: 800,
            units: Units::Inches,
        };
        let png = render_png(&pattern, &opts).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        if let Ok(dir) = std::env::var("CLOTHES3D_RENDER_OUT") {
            std::fs::write(std::path::Path::new(&dir).join("render.png"), png).unwrap();
        }
    }

    #[test]
    fn scale_bar_is_nice() {
        assert_eq!(scale_bar(Units::Millimeters, 600.0).1, "10 cm");
        assert_eq!(scale_bar(Units::Inches, 25.4 * 20.0).1, "4 in");
        assert!((scale_bar(Units::Inches, 25.4 * 20.0).0 - 4.0 * MM_PER_INCH).abs() < 1e-9);
    }
}

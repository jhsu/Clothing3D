//! Custom-drawn widgets from the component sheets: key caps, toggle chips,
//! section headers, the tool dock and seam list rows.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};

use crate::theme;

/// A key or mouse action drawn as a key cap, for the hint bar.
/// Drawn at its text's size, so it doesn't stretch to a taller row.
pub fn key_cap(ui: &mut egui::Ui, keys: &str) {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(keys.to_owned(), font, theme::INK);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(10.0, 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, 3.0, theme::PAPER, Stroke::new(1.0, theme::CUT_LINE), egui::StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, theme::INK);
}

/// Small caps chip for a display toggle: blue while on.
pub fn toggle_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let (fill, text) = if on {
        (theme::ACCENT.gamma_multiply(0.12), theme::ACCENT)
    } else {
        (Color32::TRANSPARENT, theme::MUTED)
    };
    let text = egui::RichText::new(label).size(10.5).strong().extra_letter_spacing(0.8).color(text);
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(6.0, 2.0);
        ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(3.0))
    })
    .inner
}

/// Small caps section title, with optional muted text on the right. Starts
/// a new section: a hairline above it unless it's the first thing in the panel.
pub fn section_header(ui: &mut egui::Ui, title: &str, trailing: Option<&str>) {
    if ui.min_rect().height() > 1.0 {
        ui.add_space(8.0);
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, theme::LINE));
        ui.add_space(8.0);
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(11.0).strong().extra_letter_spacing(1.1).color(theme::MUTED));
        if let Some(text) = trailing {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(text).size(12.0).color(theme::MUTED));
            });
        }
    });
    ui.add_space(2.0);
}

/// Glyphs for the tool dock.
#[derive(Clone, Copy, Debug)]
pub enum ToolIcon {
    Select,
    Draw,
    AddPoint,
    Curve,
    Sew,
}

/// One tool in the dock: a 40 px square with the tool's glyph and its
/// shortcut letter in the corner, blue while active.
pub fn tool_button(ui: &mut egui::Ui, icon: ToolIcon, letter: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let painter = ui.painter();
    let fill = if active {
        theme::ACCENT
    } else if resp.hovered() {
        theme::CHROME
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 8.0, fill);
    let ink = if active { theme::PAPER } else { theme::INK };
    paint_tool_icon(painter, rect.center(), icon, ink, fill);
    let letter_color = if active { theme::PAPER } else { theme::MUTED };
    painter.text(rect.right_bottom() + vec2(-4.0, -2.0), Align2::RIGHT_BOTTOM, letter, FontId::proportional(9.0), letter_color);
    resp
}

/// Draw `icon` in an 18 px box centered on `c`. `bg` fills hollow parts.
fn paint_tool_icon(painter: &egui::Painter, c: Pos2, icon: ToolIcon, ink: Color32, bg: Color32) {
    // Points in the 18 × 18 icon grid, origin at the top left.
    let p = |x: f32, y: f32| c + vec2(x - 9.0, y - 9.0);
    let stroke = Stroke::new(1.5, ink);
    match icon {
        ToolIcon::Select => {
            // The arrow isn't convex: fill its head and tail separately.
            painter.add(Shape::convex_polygon(vec![p(4.0, 2.0), p(4.0, 15.0), p(7.5, 11.5), p(14.0, 10.6)], ink, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![p(7.5, 11.5), p(10.0, 16.5), p(12.0, 15.5), p(9.6, 10.6)], ink, Stroke::NONE));
        }
        ToolIcon::Draw => {
            let pen = [(3.0, 15.0), (5.0, 9.0), (12.0, 2.0), (16.0, 6.0), (9.0, 13.0)];
            painter.add(Shape::closed_line(pen.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
            painter.line_segment([p(3.0, 15.0), p(7.0, 11.0)], stroke);
        }
        ToolIcon::AddPoint => {
            painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                [p(1.0, 14.0), p(9.0, 6.0), p(17.0, 14.0)],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
            painter.line_segment([p(12.0, 1.0), p(12.0, 7.0)], stroke);
            painter.line_segment([p(9.0, 4.0), p(15.0, 4.0)], stroke);
        }
        ToolIcon::Curve => {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [p(2.0, 15.0), p(4.0, 4.0), p(14.0, 4.0), p(16.0, 15.0)],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
            painter.circle(p(9.0, 6.8), 2.0, bg, stroke);
        }
        ToolIcon::Sew => {
            painter.line_segment([p(2.0, 4.0), p(16.0, 4.0)], stroke);
            painter.line_segment([p(2.0, 14.0), p(16.0, 14.0)], stroke);
            for x in [5.0, 9.0, 13.0] {
                painter.extend(Shape::dashed_line(&[p(x, 4.0), p(x, 14.0)], stroke, 2.0, 2.0));
            }
        }
    }
}

/// What a seam row shows.
pub struct SeamRow<'a> {
    pub number: usize,
    pub color: Color32,
    pub label: &'a str,
    /// Length difference, already formatted.
    pub delta: &'a str,
    pub over_tolerance: bool,
    pub selected: bool,
}

/// A clickable seam list row: number badge in the seam's color, label, Δ in
/// a right-aligned lane, and a status mark (green dot, or amber warning).
pub fn seam_row(ui: &mut egui::Ui, row: &SeamRow) -> egui::Response {
    const HEIGHT: f32 = 30.0;
    const DELTA_W: f32 = 64.0;
    const STATUS_W: f32 = 18.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::click());
    let painter = ui.painter();
    if row.selected {
        painter.rect_filled(rect, 0.0, theme::ACCENT.gamma_multiply(0.08));
        let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
        painter.rect_filled(bar, 0.0, theme::ACCENT);
    } else if resp.hovered() {
        painter.rect_filled(rect, 0.0, theme::LINE.gamma_multiply(0.5));
    }
    let y = rect.center().y;
    let badge = pos2(rect.left() + 18.0, y);
    painter.circle_filled(badge, 10.0, row.color);
    painter.text(badge, Align2::CENTER_CENTER, row.number.to_string(), FontId::proportional(10.0), theme::on(row.color));

    let status = pos2(rect.right() - STATUS_W / 2.0 - 4.0, y);
    let delta_right = rect.right() - STATUS_W - 10.0;
    let label_left = rect.left() + 36.0;
    let label_rect = Rect::from_x_y_ranges(label_left..=delta_right - DELTA_W - 6.0, rect.y_range());
    let galley = painter.layout_no_wrap(row.label.to_owned(), FontId::proportional(13.0), theme::INK);
    painter.with_clip_rect(label_rect).galley(pos2(label_left, y - galley.size().y / 2.0), galley, theme::INK);

    let delta_color = if row.over_tolerance { theme::WARN } else { theme::MUTED };
    painter.text(pos2(delta_right, y), Align2::RIGHT_CENTER, row.delta, FontId::monospace(12.0), delta_color);
    if row.over_tolerance {
        let tri = vec![status + vec2(0.0, -5.5), status + vec2(6.0, 4.5), status + vec2(-6.0, 4.5)];
        painter.add(Shape::convex_polygon(tri, theme::WARN, Stroke::NONE));
        painter.text(status + vec2(0.0, 1.0), Align2::CENTER_CENTER, "!", FontId::proportional(9.0), theme::PAPER);
    } else {
        painter.circle_filled(status, 3.5, theme::OK);
    }
    resp
}

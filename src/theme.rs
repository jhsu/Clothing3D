//! The "tailor's chalk" look: a pattern-paper canvas, graphite ink, and
//! chalk colors for seams. Chalk blue is reserved for the selection and for
//! things being created; seams never use it.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

/// Interface text: Inter Tight (SIL OFL 1.1, see `assets/fonts/inter-tight`).
/// The variable font is drawn at its default weight, Regular.
pub const UI_FONT: &[u8] = include_bytes!("../assets/fonts/inter-tight/InterTight[wght].ttf");
/// Measurements: IBM Plex Mono (SIL OFL 1.1, see `assets/fonts/ibm-plex-mono`).
pub const MONO_FONT: &[u8] = include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");

/// Canvas and piece fill: pattern paper.
pub const PAPER: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// Panels and bars around the canvas.
pub const CHROME: Color32 = Color32::from_rgb(0xF3, 0xF3, 0xF1);
/// Hairlines and dividers.
pub const LINE: Color32 = Color32::from_rgb(0xE2, 0xE2, 0xDE);
pub const GRID_MINOR: Color32 = Color32::from_rgb(0xEF, 0xEF, 0xEC);
pub const GRID_MAJOR: Color32 = Color32::from_rgb(0xDE, 0xDE, 0xD9);
/// Secondary text and labels.
pub const MUTED: Color32 = Color32::from_rgb(0x5F, 0x62, 0x67);
/// Graphite: text and sew lines.
pub const INK: Color32 = Color32::from_rgb(0x1C, 0x1E, 0x21);
/// Tailor's chalk blue: selection, hover, and edits in progress.
pub const ACCENT: Color32 = Color32::from_rgb(0x2D, 0x5B, 0xD8);
/// Fill of the selected piece: paper with a trace of chalk blue.
pub const SELECTED_FILL: Color32 = Color32::from_rgb(0xF2, 0xF5, 0xFD);
/// Seam allowance band between the sew line and the cut line.
pub const ALLOWANCE: Color32 = Color32::from_rgb(0xF1, 0xF3, 0xF7);
pub const CUT_LINE: Color32 = Color32::from_rgb(0xAE, 0xB4, 0xBF);
/// Within tolerance.
pub const OK: Color32 = Color32::from_rgb(0x1B, 0x7A, 0x4D);
/// Out of tolerance; dark enough to read as text.
pub const WARN: Color32 = Color32::from_rgb(0x94, 0x62, 0x00);

/// Seam identity colors, in order of creation: chalk red, ochre, teal,
/// violet, moss, cocoa. No blue: that means "selected".
pub const SEAMS: [Color32; 6] = [
    Color32::from_rgb(0xD6, 0x45, 0x3A),
    Color32::from_rgb(0xD9, 0x9A, 0x1E),
    Color32::from_rgb(0x2A, 0x9D, 0x8F),
    Color32::from_rgb(0x8B, 0x5C, 0xC4),
    Color32::from_rgb(0x5E, 0x8C, 0x3A),
    Color32::from_rgb(0x8C, 0x6A, 0x4F),
];

/// Identity color of seam `i`.
pub fn seam(i: usize) -> Color32 {
    SEAMS[i % SEAMS.len()]
}

/// Text color for labels drawn on a `bg` fill (badges, chips): ink or
/// paper, whichever contrasts more.
pub fn on(bg: Color32) -> Color32 {
    if contrast(INK, bg) >= contrast(PAPER, bg) { INK } else { PAPER }
}

/// WCAG 2 contrast ratio between two opaque colors (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Light egui visuals in the chalk palette.
pub fn visuals() -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = CHROME;
    v.window_fill = PAPER;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.faint_bg_color = Color32::from_rgb(0xEB, 0xEB, 0xE7);
    v.extreme_bg_color = PAPER;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.weak_text_color = Some(MUTED);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, PAPER);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.fg_stroke.color = INK;
    }
    w.inactive.weak_bg_fill = Color32::from_rgb(0xE7, 0xE7, 0xE3);
    w.inactive.bg_fill = Color32::from_rgb(0xE7, 0xE7, 0xE3);
    w.hovered.weak_bg_fill = Color32::from_rgb(0xDD, 0xDD, 0xD8);
    w.hovered.bg_fill = Color32::from_rgb(0xDD, 0xDD, 0xD8);
    w.hovered.bg_stroke = Stroke::new(1.0, CUT_LINE);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    // Fields and buttons 6, menus and windows 8 (see the component sheets).
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = 6.into();
        state.expansion = 0.0;
    }
    v.menu_corner_radius = 8.into();
    v.window_corner_radius = 8.into();
    let shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(30) };
    v.popup_shadow = shadow;
    v.window_shadow = shadow;
    v
}

/// Spacing and text sizes from the type scale: 13 px body and buttons,
/// 11 px small labels, roomier buttons.
fn style(style: &mut egui::Style) {
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::proportional(18.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 4.0);
    s.interact_size.y = 24.0;
    s.menu_margin = egui::Margin::same(6);
    s.window_margin = egui::Margin::same(14);
}

/// Inter Tight for text and IBM Plex Mono for numbers, ahead of egui's own
/// fonts, which stay as fallbacks for symbols and emoji.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes, family) in [
        ("Inter Tight", UI_FONT, FontFamily::Proportional),
        ("IBM Plex Mono", MONO_FONT, FontFamily::Monospace),
    ] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        fonts.families.entry(family).or_default().insert(0, name.to_owned());
    }
    fonts
}

/// Apply the chalk look to `ctx`. The pattern reads as paper, so the app
/// stays light even when the system prefers dark.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.set_visuals_of(egui::Theme::Light, visuals());
    ctx.style_mut_of(egui::Theme::Light, style);
    ctx.set_fonts(fonts());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hue in degrees.
    fn hue(c: Color32) -> f32 {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h * 60.0
    }

    #[test]
    fn seams_never_use_the_selection_blue() {
        let blue = hue(ACCENT);
        for c in SEAMS {
            // Anything near the accent's hue would read as "selected".
            assert!((hue(c) - blue).abs() > 30.0, "{c:?} is too close to the selection blue");
        }
        assert_eq!(seam(SEAMS.len()), seam(0));
    }

    #[test]
    fn text_is_readable_on_its_backgrounds() {
        // WCAG AA for small text: 4.5:1.
        let hover = Color32::from_rgb(0xE7, 0xE7, 0xE3);
        for (name, text, bg) in [
            ("ink on paper", INK, PAPER),
            ("muted on paper", MUTED, PAPER),
            ("muted on chrome", MUTED, CHROME),
            ("muted on hovered control", MUTED, hover),
            ("muted on line (hovered row)", MUTED, LINE),
            ("warn on paper", WARN, PAPER),
            ("warn on chrome", WARN, CHROME),
            ("ok on paper", OK, PAPER),
            ("ok on chrome", OK, CHROME),
            ("accent on paper", ACCENT, PAPER),
            ("accent on chrome", ACCENT, CHROME),
            ("paper on accent (selected rows, active tool)", PAPER, ACCENT),
        ] {
            let c = contrast(text, bg);
            assert!(c >= 4.5, "{name}: {c:.2}");
        }
    }

    #[test]
    fn badge_text_picks_the_readable_ink() {
        assert_eq!(on(SEAMS[1]), INK); // ochre is light
        assert_eq!(on(SEAMS[3]), PAPER); // violet is dark
        for c in SEAMS {
            assert!(contrast(on(c), c) >= 4.0, "{c:?}: {:.2}", contrast(on(c), c));
        }
    }
}

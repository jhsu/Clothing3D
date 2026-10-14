//! Viewports: windows onto the open pattern. Each has its own kind of view
//! and camera; the pattern, selection and tool are shared by all of them.

use eframe::egui::{Rect, pos2};
use serde::{Deserialize, Serialize};

use crate::view::View;

/// What a viewport shows and how.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ViewKind {
    /// The flat pattern, through a 2D camera.
    Pattern(View),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Viewport {
    pub kind: ViewKind,
    /// Frame the pattern the next time this viewport is laid out.
    #[serde(skip)]
    pub fit_requested: bool,
}

impl Viewport {
    pub fn pattern(view: View) -> Self {
        Self { kind: ViewKind::Pattern(view), fit_requested: false }
    }

    /// The 2D camera, for viewports that show the flat pattern.
    pub fn camera(&self) -> Option<View> {
        match self.kind {
            ViewKind::Pattern(view) => Some(view),
        }
    }

    pub fn camera_mut(&mut self) -> Option<&mut View> {
        match &mut self.kind {
            ViewKind::Pattern(view) => Some(view),
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::pattern(View::default())
    }
}

/// Gap between side-by-side viewports, in screen pixels.
pub const GAP: f32 = 2.0;

/// Split `area` into `n` equal columns separated by [`GAP`].
pub fn layout(area: Rect, n: usize) -> Vec<Rect> {
    let n = n.max(1);
    let width = (area.width() - GAP * (n - 1) as f32).max(0.0) / n as f32;
    (0..n)
        .map(|i| {
            let left = area.left() + i as f32 * (width + GAP);
            Rect::from_min_max(pos2(left, area.top()), pos2(left + width, area.bottom()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_splits_into_columns() {
        let area = Rect::from_min_max(pos2(10.0, 0.0), pos2(212.0, 100.0));
        let rects = layout(area, 2);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].left(), 10.0);
        assert_eq!(rects[1].right(), 212.0);
        assert_eq!(rects[1].left() - rects[0].right(), GAP);
        assert_eq!(rects[0].width(), rects[1].width());
        assert_eq!(layout(area, 1), vec![area]);
    }
}

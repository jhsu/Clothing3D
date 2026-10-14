//! 2D camera: maps pattern space (mm, Y up) to screen space (px, Y down).

use crate::model::Vec2;
use eframe::egui::{Pos2, Rect};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct View {
    /// World point shown at the center of the viewport.
    pub center: Vec2,
    /// Pixels per millimetre.
    pub zoom: f64,
}

pub const MIN_ZOOM: f64 = 0.02;
pub const MAX_ZOOM: f64 = 200.0;

impl Default for View {
    fn default() -> Self {
        Self { center: Vec2::new(250.0, 250.0), zoom: 1.0 }
    }
}

impl View {
    pub fn to_screen(self, rect: Rect, p: Vec2) -> Pos2 {
        let c = rect.center();
        Pos2::new(
            c.x + ((p.x - self.center.x) * self.zoom) as f32,
            c.y - ((p.y - self.center.y) * self.zoom) as f32,
        )
    }

    pub fn to_world(self, rect: Rect, p: Pos2) -> Vec2 {
        let c = rect.center();
        Vec2::new(
            self.center.x + f64::from(p.x - c.x) / self.zoom,
            self.center.y - f64::from(p.y - c.y) / self.zoom,
        )
    }

    /// Pan by a screen-space delta.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.center.x -= f64::from(dx) / self.zoom;
        self.center.y += f64::from(dy) / self.zoom;
    }

    /// Zoom by `factor`, keeping the world point under `anchor` fixed.
    pub fn zoom_at(&mut self, rect: Rect, anchor: Pos2, factor: f64) {
        let before = self.to_world(rect, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(rect, anchor);
        self.center = self.center + (before - after);
    }

    /// Frame the given world-space bounds with some margin.
    pub fn fit(&mut self, rect: Rect, min: Vec2, max: Vec2) {
        let size = max - min;
        self.center = min.lerp(max, 0.5);
        let zx = f64::from(rect.width()) / size.x.max(1.0);
        let zy = f64::from(rect.height()) / size.y.max(1.0);
        self.zoom = (zx.min(zy) * 0.85).clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

/// A camera placed in a screen rectangle: how one 2D viewport maps between
/// pattern and screen this frame.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    pub view: View,
    pub rect: Rect,
}

impl Projection {
    pub fn to_screen(self, p: Vec2) -> Pos2 {
        self.view.to_screen(self.rect, p)
    }

    pub fn to_world(self, p: Pos2) -> Vec2 {
        self.view.to_world(self.rect, p)
    }

    /// Pixels per millimetre.
    pub fn zoom(self) -> f64 {
        self.view.zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_zoom_anchor() {
        let rect = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(800.0, 600.0));
        let mut v = View { center: Vec2::new(10.0, 20.0), zoom: 2.0 };
        let w = Vec2::new(33.0, -7.0);
        let back = v.to_world(rect, v.to_screen(rect, w));
        assert!((back - w).length() < 1e-3);

        let anchor = Pos2::new(100.0, 450.0);
        let before = v.to_world(rect, anchor);
        v.zoom_at(rect, anchor, 3.0);
        assert!((v.to_world(rect, anchor) - before).length() < 1e-3);
    }
}

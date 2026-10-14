//! Editing Bézier handles: dragging handles (mirrored for smooth points),
//! bending a segment through a dragged point, straightening and smoothing.

use crate::model::{Piece, PointKind, Vec2};

/// Which of a point's two handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleSide {
    In,
    Out,
}

fn normalized(v: Vec2) -> Option<Vec2> {
    let len = v.length();
    (len > 1e-12).then(|| v * (1.0 / len))
}

impl Piece {
    /// Whether point `i` has a segment arriving at it.
    fn has_in_segment(&self, i: usize) -> bool {
        self.prev(i).is_some()
    }

    /// Whether point `i` has a segment leaving it.
    fn has_out_segment(&self, i: usize) -> bool {
        self.next(i).is_some()
    }

    /// Handles drawn for editing point `i`: explicit handles, plus automatic
    /// ones on curve points. Sides without a segment are `None`.
    pub fn visible_handles(&self, i: usize) -> (Option<Vec2>, Option<Vec2>) {
        let p = &self.points[i];
        let show = |explicit: Option<Vec2>, auto: Vec2, has_seg: bool| {
            has_seg
                .then(|| explicit.or((p.kind == PointKind::Curve).then_some(auto)))
                .flatten()
        };
        (
            show(p.handle_in, self.in_handle(i), self.has_in_segment(i)),
            show(p.handle_out, self.out_handle(i), self.has_out_segment(i)),
        )
    }

    /// Replace a curve point's automatic handles with explicit ones of the
    /// same shape, so later edits to neighbors don't change its curve.
    pub fn materialize_handles(&mut self, i: usize) {
        if self.points[i].kind != PointKind::Curve {
            return;
        }
        if self.points[i].handle_in.is_none() && self.has_in_segment(i) {
            self.points[i].handle_in = Some(self.in_handle(i));
        }
        if self.points[i].handle_out.is_none() && self.has_out_segment(i) {
            self.points[i].handle_out = Some(self.out_handle(i));
        }
    }

    /// Set one handle of point `i` to `offset`. On a curve (smooth) point the
    /// opposite handle mirrors it (same length, opposite direction) when
    /// `mirror`; without `mirror` the handles are broken apart and the point
    /// becomes a turn point (corner). Turn points' handles move independently.
    pub fn set_handle(&mut self, i: usize, side: HandleSide, offset: Vec2, mirror: bool) {
        self.materialize_handles(i);
        let has_other = match side {
            HandleSide::In => self.has_out_segment(i),
            HandleSide::Out => self.has_in_segment(i),
        };
        let p = &mut self.points[i];
        let smooth = p.kind == PointKind::Curve;
        match side {
            HandleSide::In => p.handle_in = Some(offset),
            HandleSide::Out => p.handle_out = Some(offset),
        }
        if smooth && !mirror {
            p.kind = PointKind::Turn;
        } else if smooth && has_other {
            let opposite = Some(offset * -1.0);
            match side {
                HandleSide::In => p.handle_out = opposite,
                HandleSide::Out => p.handle_in = opposite,
            }
        }
    }

    /// Control handles `(out of start, into end)` of segment `seg`, as offsets.
    /// Straight segments get handles a third of the way along the chord, so
    /// bending one starts from an evenly parameterized straight line.
    pub fn segment_handles(&self, seg: usize) -> (Vec2, Vec2) {
        let (a, b) = self.segment_ends(seg);
        if !self.segment_is_curved(seg) {
            let chord = self.points[b].pos - self.points[a].pos;
            return (chord * (1.0 / 3.0), chord * (-1.0 / 3.0));
        }
        (self.out_handle(a), self.in_handle(b))
    }

    /// Set the handles of segment `seg` (see [`Self::segment_handles`]). The
    /// far handle of a smooth end point is turned to stay collinear, keeping
    /// its length, so the neighboring segment stays smoothly joined.
    pub fn set_segment_handles(&mut self, seg: usize, out_a: Vec2, in_b: Vec2) {
        let (a, b) = self.segment_ends(seg);
        self.materialize_handles(a);
        self.materialize_handles(b);
        self.points[a].handle_out = Some(out_a);
        self.points[b].handle_in = Some(in_b);
        let keep_collinear = |far: Option<Vec2>, near: Vec2| -> Option<Vec2> {
            let far = far?;
            Some(normalized(near).map_or(far, |d| d * -far.length()))
        };
        if self.points[a].kind == PointKind::Curve {
            self.points[a].handle_in = keep_collinear(self.points[a].handle_in, out_a);
        }
        if self.points[b].kind == PointKind::Curve {
            self.points[b].handle_out = keep_collinear(self.points[b].handle_out, in_b);
        }
    }

    /// Bend segment `seg` so the curve point at parameter `t` moves by
    /// `delta`, starting from handles `start` (as returned by
    /// [`Self::segment_handles`] when the drag began). Both handles move by
    /// the same amount, which moves `B(t)` by exactly `delta`.
    pub fn bend_segment(&mut self, seg: usize, t: f64, start: (Vec2, Vec2), delta: Vec2) {
        let t = t.clamp(0.05, 0.95);
        let k = 1.0 / (3.0 * t * (1.0 - t));
        let d = delta * k;
        self.set_segment_handles(seg, start.0 + d, start.1 + d);
    }

    /// Make segment `seg` straight. Its end points become turn points; their
    /// handles on the neighboring segments are kept, so neighbors don't change.
    pub fn straighten_segment(&mut self, seg: usize) {
        let (a, b) = self.segment_ends(seg);
        self.materialize_handles(a);
        self.materialize_handles(b);
        self.points[a].handle_out = None;
        self.points[b].handle_in = None;
        self.points[a].kind = PointKind::Turn;
        self.points[b].kind = PointKind::Turn;
    }

    /// Change point `i`'s kind. Becoming a curve point lines up existing
    /// explicit handles so the point is actually smooth.
    pub fn set_point_kind(&mut self, i: usize, kind: PointKind) {
        let p = &mut self.points[i];
        p.kind = kind;
        if kind == PointKind::Curve
            && let (Some(hin), Some(hout)) = (p.handle_in, p.handle_out)
            && let Some(dir) = normalized(hout - hin)
        {
            p.handle_in = Some(dir * -hin.length());
            p.handle_out = Some(dir * hout.length());
        }
    }

    /// Remove point `i`'s explicit handles (curve points fall back to automatic smoothing).
    pub fn clear_handles(&mut self, i: usize) {
        self.points[i].handle_in = None;
        self.points[i].handle_out = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;

    fn square() -> Piece {
        let mut p = Piece::new("sq");
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)] {
            p.points.push(Point::turn(Vec2::new(x, y)));
        }
        p.closed = true;
        p
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn automatic_handles_reproduce_previous_curves() {
        // Explicitly materialized handles must not change the shape.
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        let before = p.flatten();
        p.materialize_handles(1);
        assert!(p.points[1].handle_in.is_some() && p.points[1].handle_out.is_some());
        for (a, b) in before.iter().zip(p.flatten()) {
            assert!(close(*a, b));
        }
    }

    #[test]
    fn bend_moves_grabbed_point_exactly() {
        let mut p = square();
        let start = p.segment_handles(0);
        let before = p.eval_segment(0, 0.3);
        p.bend_segment(0, 0.3, start, Vec2::new(0.0, -20.0));
        assert!(p.segment_is_curved(0));
        assert!(close(p.eval_segment(0, 0.3), before + Vec2::new(0.0, -20.0)));
        // End points stay put; neighbors stay straight.
        assert!(close(p.eval_segment(0, 0.0), Vec2::new(0.0, 0.0)));
        assert!(close(p.eval_segment(0, 1.0), Vec2::new(100.0, 0.0)));
        assert!(!p.segment_is_curved(1));
    }

    #[test]
    fn mirrored_and_broken_handles() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        p.set_handle(1, HandleSide::Out, Vec2::new(10.0, 30.0), true);
        assert_eq!(p.points[1].handle_in, Some(Vec2::new(-10.0, -30.0)));
        assert_eq!(p.points[1].kind, PointKind::Curve);

        p.set_handle(1, HandleSide::In, Vec2::new(-5.0, 0.0), false);
        assert_eq!(p.points[1].kind, PointKind::Turn);
        assert_eq!(p.points[1].handle_out, Some(Vec2::new(10.0, 30.0)));

        // Back to smooth: handles realigned, lengths kept.
        p.set_point_kind(1, PointKind::Curve);
        let (hin, hout) = (p.points[1].handle_in.unwrap(), p.points[1].handle_out.unwrap());
        assert!((hin.length() - 5.0).abs() < 1e-9);
        assert!((hout.length() - 10.0f64.hypot(30.0)).abs() < 1e-9);
        assert!((hin.x * hout.y - hin.y * hout.x).abs() < 1e-9); // collinear
        assert!(hin.dot(hout) < 0.0); // opposite
    }

    #[test]
    fn bending_next_to_smooth_point_keeps_it_smooth() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        let start = p.segment_handles(0);
        p.bend_segment(0, 0.5, start, Vec2::new(0.0, -30.0));
        let (hin, hout) = (p.points[1].handle_in.unwrap(), p.points[1].handle_out.unwrap());
        assert!((hin.x * hout.y - hin.y * hout.x).abs() < 1e-9);
        assert!(hin.dot(hout) < 0.0);
    }

    #[test]
    fn insert_splits_bezier_exactly() {
        let mut p = square();
        let start = p.segment_handles(0);
        p.bend_segment(0, 0.5, start, Vec2::new(10.0, -25.0));
        let samples: Vec<Vec2> = (0..=10).map(|k| p.eval_segment(0, k as f64 / 10.0)).collect();
        let idx = p.insert_on_segment(0, 0.4);
        assert_eq!(idx, 1);
        // First half covers t in [0, 0.4], second half [0.4, 1].
        for (k, s) in samples.iter().enumerate() {
            let t = k as f64 / 10.0;
            let q = if t <= 0.4 { p.eval_segment(0, t / 0.4) } else { p.eval_segment(1, (t - 0.4) / 0.6) };
            assert!((q - *s).length() < 1e-9, "t={t}");
        }
    }

    #[test]
    fn straighten_keeps_neighbors() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        p.points[2].kind = PointKind::Curve;
        let neighbor: Vec<Vec2> = (0..=8).map(|k| p.eval_segment(0, k as f64 / 8.0)).collect();
        p.straighten_segment(1);
        assert!(!p.segment_is_curved(1));
        for (k, s) in neighbor.iter().enumerate() {
            assert!((p.eval_segment(0, k as f64 / 8.0) - *s).length() < 1e-9);
        }
    }

    #[test]
    fn rotate_and_flip_carry_handles() {
        let mut p = square();
        let start = p.segment_handles(0);
        p.bend_segment(0, 0.5, start, Vec2::new(0.0, -30.0));
        let area = p.area();
        let mut r = p.clone();
        r.rotate(90.0);
        assert!((r.area() - area).abs() < 1e-6);
        let mut f = p.clone();
        let (c, bulge) = (p.center(), p.eval_segment(0, 0.5));
        f.flip(false);
        // Every curve point, not just the vertices, lands on its mirror image.
        assert!(close(f.eval_segment(0, 0.5), Vec2::new(bulge.x, 2.0 * c.y - bulge.y)));
        assert!((f.area().abs() - area.abs()).abs() < 1e-6);
    }

    #[test]
    fn old_files_without_handles_still_load() {
        let json = r#"{"pos":{"x":1.0,"y":2.0},"kind":"Curve"}"#;
        let p: Point = serde_json::from_str(json).unwrap();
        assert_eq!(p.handle_in, None);
        assert!(!serde_json::to_string(&p).unwrap().contains("handle"));
    }
}

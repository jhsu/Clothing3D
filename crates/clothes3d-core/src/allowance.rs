//! Seam allowance: the cut line around a piece's sew line.
//!
//! A piece's outline is its sew line. Each segment has an allowance width:
//! its start point's `allowance` override, else the piece's `seam_allowance`.
//! The cut line offsets every segment outward by its width and joins
//! neighbors at the intersection of their offset lines (mitered corners);
//! loops where the offset folds into itself (tight concave curves, notches
//! narrower than the allowance) are cut out.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;

use crate::model::{Piece, Vec2};

/// Corners sharper than this (miter length over allowance width) are beveled
/// instead of extended to a spike.
const MITER_LIMIT: f64 = 4.0;
/// Offset ends closer than this (mm) are merged into one vertex.
const JOIN_EPS: f64 = 1e-3;

impl Piece {
    /// Seam allowance (mm) of segment `seg`: its own, else the piece default.
    pub fn segment_allowance(&self, seg: usize) -> f64 {
        self.points[seg].allowance.unwrap_or(self.seam_allowance).max(0.0)
    }

    /// Whether the piece has a cut line: closed, with some allowance.
    pub fn has_allowance(&self) -> bool {
        self.closed && (0..self.segment_count()).any(|s| self.segment_allowance(s) > 0.0)
    }

    /// Set the allowance of every segment to the piece default by dropping
    /// their overrides.
    pub fn clear_allowance_overrides(&mut self) {
        for p in &mut self.points {
            p.allowance = None;
        }
    }

    /// Axis-aligned bounds including the seam allowance, as `(min, max)`.
    pub fn outer_bounds(&self) -> Option<(Vec2, Vec2)> {
        let cut = self.cut_line();
        if cut.is_empty() {
            return self.bounds();
        }
        Some(cut.iter().fold((cut[0], cut[0]), |(lo, hi), p| {
            (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y)))
        }))
    }

    /// The cut line: the outline offset outward by each segment's allowance,
    /// as a closed counter-clockwise polyline (first point repeated). Empty
    /// without allowance.
    pub fn cut_line(&self) -> Vec<Vec2> {
        if !self.has_allowance() {
            return Vec::new();
        }
        // Outward is to the right of travel for counter-clockwise outlines.
        let side = if self.area() < 0.0 { -1.0 } else { 1.0 };
        // (offset samples, width, sew-line corner at the segment's end)
        let offsets: Vec<(Vec<Vec2>, f64, Vec2)> = (0..self.segment_count())
            .filter_map(|seg| {
                let w = self.segment_allowance(seg);
                let pts = self.flatten_segment(seg);
                let corner = pts[pts.len() - 1];
                offset_polyline(&pts, w * side).map(|off| (off, w, corner))
            })
            .collect();
        let n = offsets.len();
        if n < 2 {
            return Vec::new();
        }

        // Each segment adds its interior samples; joins supply the corners.
        let mut poly = Vec::new();
        for (i, (pts, w, corner)) in offsets.iter().enumerate() {
            let (next, next_w, _) = &offsets[(i + 1) % n];
            poly.extend_from_slice(&pts[1..pts.len() - 1]);
            let convex = cross(pts[pts.len() - 1] - pts[pts.len() - 2], next[1] - next[0]) * side > 0.0;
            join(&mut poly, pts, *w, next, *next_w, convex.then_some(*corner));
        }
        let mut outline = self.flatten();
        outline.pop();
        let mut poly = outer_boundary(&outline, &poly);
        if let Some(&first) = poly.first() {
            poly.push(first);
        }
        poly
    }
}

fn perp(d: Vec2) -> Vec2 {
    Vec2::new(d.y, -d.x)
}

fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.length();
    (len > 1e-12).then(|| v * (1.0 / len))
}

/// Offset an open polyline by `dist` to the right of travel (negative: left),
/// mitering its interior vertices. `None` if it has no length.
fn offset_polyline(pts: &[Vec2], dist: f64) -> Option<Vec<Vec2>> {
    let mut pts = pts.to_vec();
    pts.dedup_by(|a, b| (*a - *b).length() < 1e-9);
    if pts.len() < 2 {
        return None;
    }
    let normals: Vec<Vec2> = pts.windows(2).map(|w| unit(w[1] - w[0]).map_or(Vec2::default(), perp)).collect();
    let last = normals.len() - 1;
    Some(
        (0..pts.len())
            .map(|k| {
                let (n0, n1) = (normals[k.saturating_sub(1)], normals[k.min(last)]);
                let Some(avg) = unit(n0 + n1) else { return pts[k] + n1 * dist };
                // Scale so both adjacent offset edges stay `dist` away.
                let cos = avg.dot(n1).max(0.5);
                pts[k] + avg * (dist / cos)
            })
            .collect(),
    )
}

/// Join the end of offset segment `a` (width `wa`) to the start of the next
/// offset segment `b`, pushing the corner vertices onto `poly`. At a convex
/// sew-line `corner` the offsets meet where their lines intersect, even when
/// that's short of one offset's end (very different widths, e.g. beside a
/// fold with none); other corners are joined straight and the overlap is
/// cut out later.
fn join(poly: &mut Vec<Vec2>, a: &[Vec2], wa: f64, b: &[Vec2], wb: f64, corner: Option<Vec2>) {
    let (a_end, b_start) = (a[a.len() - 1], b[0]);
    if (a_end - b_start).length() < JOIN_EPS {
        poly.push(a_end.lerp(b_start, 0.5));
        return;
    }
    let a_dir = a_end - a[a.len() - 2];
    let b_dir = b[1] - b_start;
    if let Some(corner) = corner
        && let (Some(ua), Some(ub)) = (unit(a_dir), unit(b_dir))
        && cross(ua, ub).abs() > 1e-6
    {
        // a_end + a_dir·u = b_start + b_dir·v
        let u = cross(b_start - a_end, b_dir) / cross(a_dir, b_dir);
        let x = a_end + a_dir * u;
        if (x - corner).length() <= MITER_LIMIT * wa.max(wb) {
            poly.push(x);
            return;
        }
    }
    poly.push(a_end);
    poly.push(b_start);
}

fn signed_area(poly: &[Vec2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| cross(poly[i], poly[(i + 1) % n])).sum::<f64>() * 0.5
}

/// Outer boundary of the region covered by `outline` together with the
/// outward offset `raw` (both closed, no repeated end vertex), as a
/// counter-clockwise polygon. Where the offset folds back on itself (tight
/// concave curves, notches narrower than the allowance) its loops wind the
/// other way and drop out under the positive fill rule.
fn outer_boundary(outline: &[Vec2], raw: &[Vec2]) -> Vec<Vec2> {
    let ccw = |pts: &[Vec2]| {
        let mut c: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
        if signed_area(pts) < 0.0 {
            c.reverse();
        }
        c
    };
    let contours = vec![ccw(outline), ccw(raw)];
    let shapes = contours.simplify_shape(FillRule::Positive);
    let to_vec = |c: &Vec<[f64; 2]>| c.iter().map(|p| Vec2::new(p[0], p[1])).collect::<Vec<_>>();
    shapes
        .iter()
        .filter_map(|shape| shape.first().map(to_vec))
        .max_by(|a, b| signed_area(a).abs().total_cmp(&signed_area(b).abs()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Point, PointKind};

    fn piece(pts: &[(f64, f64)]) -> Piece {
        let mut p = Piece::new("p");
        p.points = pts.iter().map(|&(x, y)| Point::turn(Vec2::new(x, y))).collect();
        p.closed = true;
        p
    }

    fn square() -> Piece {
        piece(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])
    }

    fn approx(a: Vec2, b: (f64, f64)) -> bool {
        (a - Vec2::new(b.0, b.1)).length() < 1e-6
    }

    #[test]
    fn no_allowance_no_cut_line() {
        let mut p = square();
        assert!(p.cut_line().is_empty());
        p.seam_allowance = 10.0;
        p.closed = false;
        assert!(p.cut_line().is_empty());
    }

    #[test]
    fn uniform_square_both_windings() {
        for flip in [false, true] {
            let mut p = square();
            if flip {
                p.points.reverse();
            }
            p.seam_allowance = 10.0;
            let cut = p.cut_line();
            assert_eq!(cut.len(), 5, "{cut:?}");
            assert_eq!(cut.first(), cut.last());
            for corner in [(-10.0, -10.0), (110.0, -10.0), (110.0, 110.0), (-10.0, 110.0)] {
                assert!(cut.iter().any(|&q| approx(q, corner)), "{corner:?} missing from {cut:?}");
            }
            assert!((signed_area(&cut[..4]).abs() - 120.0 * 120.0).abs() < 1e-6);
        }
    }

    #[test]
    fn per_segment_widths_and_zero_override() {
        let mut p = square();
        p.seam_allowance = 10.0;
        p.points[0].allowance = Some(30.0); // bottom: hem
        p.points[3].allowance = Some(0.0); // left: fold
        assert_eq!(p.segment_allowance(0), 30.0);
        assert_eq!(p.segment_allowance(1), 10.0);
        let cut = p.cut_line();
        for corner in [(0.0, -30.0), (110.0, -30.0), (110.0, 110.0), (0.0, 110.0)] {
            assert!(cut.iter().any(|&q| approx(q, corner)), "{corner:?} missing from {cut:?}");
        }
        p.clear_allowance_overrides();
        assert_eq!(p.segment_allowance(0), 10.0);
    }

    #[test]
    fn allowance_stops_at_fold() {
        // Left edge is a fold (no allowance); the slanted top meets it at an
        // obtuse corner, where the offset must end on the fold line.
        let mut p = piece(&[(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 100.0)]);
        p.seam_allowance = 10.0;
        p.points[3].allowance = Some(0.0);
        let cut = p.cut_line();
        assert!(cut.iter().all(|q| q.x > -1e-6), "{cut:?}");
        assert!(cut.iter().any(|&q| approx(q, (0.0, -10.0))), "{cut:?}");
        let top = cut.iter().filter(|q| q.x.abs() < 1e-6).map(|q| q.y).fold(f64::MIN, f64::max);
        assert!((top - (100.0 + 10.0 * (1.0 + 0.2f64.powi(2)).sqrt())).abs() < 1e-6, "{cut:?}");
    }

    #[test]
    fn concave_corner_is_trimmed() {
        // L-shape: the inner corner at (100, 100) is concave.
        let mut p = piece(&[(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (100.0, 100.0), (100.0, 200.0), (0.0, 200.0)]);
        p.seam_allowance = 10.0;
        let cut = p.cut_line();
        assert!(cut.iter().any(|&q| approx(q, (110.0, 110.0))), "{cut:?}");
        assert_eq!(cut.len(), 7, "{cut:?}");
        // Mitered offset of a rectilinear outline: area + perimeter·w + w²·(convex − concave corners).
        let expected = 30_000.0 + 800.0 * 10.0 + 100.0 * (5.0 - 1.0);
        assert!((signed_area(&cut[..cut.len() - 1]) - expected).abs() < 1e-6);
    }

    #[test]
    fn narrow_notch_loop_removed() {
        // A 10 mm wide slot with 10 mm allowance: its offsets cross.
        let mut p = piece(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (55.0, 100.0),
            (55.0, 40.0),
            (45.0, 40.0),
            (45.0, 100.0),
            (0.0, 100.0),
        ]);
        p.seam_allowance = 10.0;
        let cut = p.cut_line();
        // The slot fills in completely: the cut line is the plain square.
        assert_eq!(cut.len(), 5, "{cut:?}");
        assert!((signed_area(&cut[..4]) - 120.0 * 120.0).abs() < 1e-6, "{cut:?}");
    }

    #[test]
    fn curves_offset_by_allowance() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        p.points[2].kind = PointKind::Curve;
        p.seam_allowance = 15.0;
        let cut = p.cut_line();
        // Cut-line vertices sit one allowance off the sew line, mitered
        // corners a bit farther.
        let mut on_offset = 0;
        for &q in &cut {
            let d = p.closest_on_outline(q).unwrap().3;
            assert!((14.5..=15.0 * MITER_LIMIT).contains(&d), "{q:?} is {d} away");
            on_offset += usize::from((d - 15.0).abs() < 0.5);
        }
        assert!(on_offset * 10 >= cut.len() * 9, "{on_offset} of {}", cut.len());
    }

    #[test]
    fn insert_and_unfold_keep_allowances() {
        let mut p = square();
        p.points[0].allowance = Some(30.0);
        let idx = p.insert_on_segment(0, 0.5);
        assert_eq!(p.points[idx].allowance, Some(30.0));

        // Half piece: left edge is the fold, bottom (seg 0) a hem.
        let mut pattern = crate::model::Pattern { pieces: vec![square()], seams: vec![], ..Default::default() };
        let half = &mut pattern.pieces[0];
        half.points[0].allowance = Some(30.0);
        half.points[1].allowance = Some(15.0);
        half.points[2].allowance = Some(5.0);
        pattern.unfold(0, &[3]).unwrap();
        let full = &pattern.pieces[0];
        assert_eq!(full.points.len(), 6);
        // Hem along both halves of the bottom, sides 15, top 5.
        for seg in 0..full.segment_count() {
            let mid = full.eval_segment(seg, 0.5);
            let want = if mid.y.abs() < 1e-9 {
                30.0
            } else if (mid.y - 100.0).abs() < 1e-9 {
                5.0
            } else {
                15.0
            };
            assert_eq!(full.segment_allowance(seg), want, "segment {seg} at {mid:?}");
        }
    }
}

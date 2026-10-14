//! Pattern data model.
//!
//! Units are millimetres, Y up (CAD convention, matches DXF). The structure
//! intentionally mirrors what DXF-AAMA/ASTM expects so import/export can be
//! added later:
//! - a `Piece` boundary is a closed polyline (AAMA layer 1)
//! - each boundary point is either a turn point (layer 2) or a curve point
//!   (layer 3); curve points are smoothly interpolated by readers
//! - internal lines, notches, grain lines, etc. will be added as extra
//!   collections on `Piece`.
//!
//! Curved segments are cubic Béziers. A point's optional handles set the
//! control points explicitly; where a handle is absent, curve points get an
//! automatic smooth tangent (Catmull-Rom style) and turn points none. DXF-AAMA
//! has no Béziers, so export will sample them into curve points.

use serde::{Deserialize, Serialize};

use crate::sewing::Seam;
use crate::symmetry::Symmetry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn lerp(self, o: Self, t: f64) -> Self {
        self + (o - self) * t
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}
impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}
impl std::ops::Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s)
    }
}

/// A distance-preserving map `p ↦ m·p + t`: a rotation or a reflection, plus
/// a translation. `x` and `y` are the columns of `m`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Isometry {
    pub x: Vec2,
    pub y: Vec2,
    pub t: Vec2,
}

impl Default for Isometry {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Isometry {
    pub const IDENTITY: Self = Self { x: Vec2::new(1.0, 0.0), y: Vec2::new(0.0, 1.0), t: Vec2::new(0.0, 0.0) };

    pub fn translation(d: Vec2) -> Self {
        Self { t: d, ..Self::IDENTITY }
    }

    /// Counter-clockwise rotation by `degrees` about `c`.
    pub fn rotation(c: Vec2, degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self { x: Vec2::new(cos, sin), y: Vec2::new(-sin, cos), t: Vec2::default() }.fixing(c)
    }

    /// Reflection across the line through `a` and `b`.
    pub fn mirror(a: Vec2, b: Vec2) -> Self {
        let d = b - a;
        let len2 = d.dot(d);
        if len2 == 0.0 {
            return Self::IDENTITY;
        }
        // Columns of the reflection matrix: (cos 2θ, sin 2θ) and (sin 2θ, -cos 2θ).
        let (cos, sin) = ((d.x * d.x - d.y * d.y) / len2, 2.0 * d.x * d.y / len2);
        Self { x: Vec2::new(cos, sin), y: Vec2::new(sin, -cos), t: Vec2::default() }.fixing(a)
    }

    /// The same linear part, translated so that `c` maps to itself.
    fn fixing(self, c: Vec2) -> Self {
        Self { t: c - self.linear(c), ..self }
    }

    /// Apply to an offset (such as a Bézier handle): rotation/reflection only.
    pub fn linear(&self, v: Vec2) -> Vec2 {
        self.x * v.x + self.y * v.y
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        self.linear(p) + self.t
    }

    /// This map followed by `next`.
    pub fn then(&self, next: &Self) -> Self {
        Self { x: next.linear(self.x), y: next.linear(self.y), t: next.apply(self.t) }
    }

    pub fn inverse(&self) -> Self {
        // The linear part is orthogonal, so its inverse is its transpose.
        let m = Self { x: Vec2::new(self.x.x, self.y.x), y: Vec2::new(self.x.y, self.y.y), t: Vec2::default() };
        Self { t: m.linear(self.t) * -1.0, ..m }
    }

    /// The mirror line `(point, unit direction)` of a reflection, ignoring
    /// any slide along it; `None` for rotations and translations.
    pub fn mirror_line(&self) -> Option<(Vec2, Vec2)> {
        if self.x.x * self.y.y - self.y.x * self.x.y >= 0.0 {
            return None;
        }
        let angle = self.x.y.atan2(self.x.x) / 2.0;
        let dir = Vec2::new(angle.cos(), angle.sin());
        let slide = self.t.dot(dir);
        Some(((self.t - dir * slide) * 0.5, dir))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointKind {
    /// Sharp corner (AAMA "turn point").
    #[default]
    Turn,
    /// Smooth point the outline curves through (AAMA "curve point").
    Curve,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub pos: Vec2,
    pub kind: PointKind,
    /// Incoming Bézier handle, as an offset from `pos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle_in: Option<Vec2>,
    /// Outgoing Bézier handle, as an offset from `pos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle_out: Option<Vec2>,
    /// Seam allowance (mm) of the segment starting at this point, overriding
    /// the piece's default. `Some(0.0)` means no allowance on that segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<f64>,
}

impl Point {
    pub fn turn(pos: Vec2) -> Self {
        Self { pos, ..Self::default() }
    }

    pub fn has_handles(&self) -> bool {
        self.handle_in.is_some() || self.handle_out.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    pub name: String,
    pub points: Vec<Point>,
    pub closed: bool,
    /// Default seam allowance (mm) for segments without their own.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seam_allowance: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Number of samples used to flatten one curved segment.
const CURVE_SAMPLES: usize = 32;

impl Piece {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), points: Vec::new(), closed: false, seam_allowance: 0.0 }
    }

    pub fn segment_count(&self) -> usize {
        match (self.points.len(), self.closed) {
            (0 | 1, _) => 0,
            (n, true) => n,
            (n, false) => n - 1,
        }
    }

    /// Index of the point following `i`, honoring closure.
    pub fn next(&self, i: usize) -> Option<usize> {
        let n = self.points.len();
        if i + 1 < n {
            Some(i + 1)
        } else if self.closed && n > 1 {
            Some(0)
        } else {
            None
        }
    }

    pub fn prev(&self, i: usize) -> Option<usize> {
        let n = self.points.len();
        if i > 0 {
            Some(i - 1)
        } else if self.closed && n > 1 {
            Some(n - 1)
        } else {
            None
        }
    }

    /// Automatic tangent at point `i` (Catmull-Rom). Turn points have a zero
    /// tangent, so a segment between two turn points is a straight line.
    pub(crate) fn auto_tangent(&self, i: usize) -> Vec2 {
        let p = &self.points[i];
        if p.kind == PointKind::Turn {
            return Vec2::default();
        }
        let prev = self.prev(i).map_or(p.pos, |j| self.points[j].pos);
        let next = self.next(i).map_or(p.pos, |j| self.points[j].pos);
        (next - prev) * 0.5
    }

    pub fn segment_is_curved(&self, seg: usize) -> bool {
        let Some(b) = self.next(seg) else { return false };
        let (pa, pb) = (&self.points[seg], &self.points[b]);
        pa.kind == PointKind::Curve
            || pb.kind == PointKind::Curve
            || pa.handle_out.is_some()
            || pb.handle_in.is_some()
    }

    /// Effective outgoing handle of point `i`: explicit, else the automatic
    /// one for curve points (a third of the tangent, Hermite-equivalent).
    pub fn out_handle(&self, i: usize) -> Vec2 {
        self.points[i].handle_out.unwrap_or_else(|| self.auto_tangent(i) * (1.0 / 3.0))
    }

    /// Effective incoming handle of point `i` (see [`Self::out_handle`]).
    pub fn in_handle(&self, i: usize) -> Vec2 {
        self.points[i].handle_in.unwrap_or_else(|| self.auto_tangent(i) * (-1.0 / 3.0))
    }

    /// Absolute Bézier points `[p0, c0, c1, p1]` of segment `seg`.
    pub fn segment_bezier(&self, seg: usize) -> [Vec2; 4] {
        let b = self.next(seg).expect("segment index out of range");
        let (p0, p1) = (self.points[seg].pos, self.points[b].pos);
        [p0, p0 + self.out_handle(seg), p1 + self.in_handle(b), p1]
    }

    /// Evaluate segment `seg` (from point `seg` to the next) at `t` in [0, 1].
    pub fn eval_segment(&self, seg: usize, t: f64) -> Vec2 {
        let b = self.next(seg).expect("segment index out of range");
        let (p0, p1) = (self.points[seg].pos, self.points[b].pos);
        if !self.segment_is_curved(seg) {
            return p0.lerp(p1, t);
        }
        let [_, c0, c1, _] = self.segment_bezier(seg);
        let u = 1.0 - t;
        p0 * (u * u * u) + c0 * (3.0 * u * u * t) + c1 * (3.0 * u * t * t) + p1 * (t * t * t)
    }

    /// Sampled polyline for one segment, including both endpoints.
    pub fn flatten_segment(&self, seg: usize) -> Vec<Vec2> {
        if !self.segment_is_curved(seg) {
            let b = self.next(seg).unwrap();
            return vec![self.points[seg].pos, self.points[b].pos];
        }
        (0..=CURVE_SAMPLES)
            .map(|k| self.eval_segment(seg, k as f64 / CURVE_SAMPLES as f64))
            .collect()
    }

    /// Full outline as a polyline (closed outlines repeat the first point).
    pub fn flatten(&self) -> Vec<Vec2> {
        let mut out = Vec::new();
        for seg in 0..self.segment_count() {
            let pts = self.flatten_segment(seg);
            let skip = usize::from(!out.is_empty());
            out.extend(pts.into_iter().skip(skip));
        }
        if out.is_empty() {
            out.extend(self.points.iter().map(|p| p.pos));
        }
        out
    }

    /// Closest point on the outline to `p`: `(segment, t, position, distance)`.
    pub fn closest_on_outline(&self, p: Vec2) -> Option<(usize, f64, Vec2, f64)> {
        let mut best: Option<(usize, f64, Vec2, f64)> = None;
        for seg in 0..self.segment_count() {
            let pts = self.flatten_segment(seg);
            let n = (pts.len() - 1) as f64;
            for (k, w) in pts.windows(2).enumerate() {
                let (q, local_t) = closest_on_line(w[0], w[1], p);
                let d = (q - p).length();
                if best.is_none_or(|b| d < b.3) {
                    best = Some((seg, (k as f64 + local_t) / n, q, d));
                }
            }
        }
        best
    }

    /// Insert a point on segment `seg` at parameter `t`. Returns its index.
    /// A segment with handles is split exactly (de Casteljau), so its shape
    /// is unchanged; the halves keep the original parameterization.
    pub fn insert_on_segment(&mut self, seg: usize, t: f64) -> usize {
        let pos = self.eval_segment(seg, t);
        let idx = seg + 1;
        let (a, b) = self.segment_ends(seg);
        // Both halves keep the split segment's allowance.
        let allowance = self.points[a].allowance;
        if self.points[a].handle_out.is_none() && self.points[b].handle_in.is_none() {
            let kind = if self.segment_is_curved(seg) { PointKind::Curve } else { PointKind::Turn };
            self.points.insert(idx, Point { pos, kind, allowance, ..Point::default() });
            return idx;
        }
        // Freeze automatic handles at both ends: their tangents depend on the
        // neighbors, which are about to change.
        self.materialize_handles(a);
        self.materialize_handles(b);
        let [p0, c0, c1, p1] = self.segment_bezier(seg);
        let (q0, q1, q2) = (p0.lerp(c0, t), c0.lerp(c1, t), c1.lerp(p1, t));
        let (r0, r1) = (q0.lerp(q1, t), q1.lerp(q2, t));
        self.points[a].handle_out = Some(q0 - p0);
        self.points[b].handle_in = Some(q2 - p1);
        let mid = Point {
            pos,
            kind: PointKind::Curve,
            handle_in: Some(r0 - pos),
            handle_out: Some(r1 - pos),
            allowance,
        };
        self.points.insert(idx, mid);
        idx
    }

    /// Signed area (positive = counter-clockwise). Only meaningful when closed.
    pub fn area(&self) -> f64 {
        let pts = self.flatten();
        pts.windows(2).map(|w| w[0].x * w[1].y - w[1].x * w[0].y).sum::<f64>() * 0.5
    }

    /// Outline polygon (without the repeated closing point) and its fill
    /// triangles as index triples. Empty unless the piece is closed.
    pub fn fill_triangles(&self) -> (Vec<Vec2>, Vec<[usize; 3]>) {
        if !self.closed {
            return (Vec::new(), Vec::new());
        }
        let mut poly = self.flatten();
        if poly.len() > 1 && poly.first() == poly.last() {
            poly.pop();
        }
        let tris = triangulate(&poly);
        (poly, tris)
    }

    /// Whether `p` lies inside the closed outline (even-odd rule).
    pub fn contains(&self, p: Vec2) -> bool {
        if !self.closed {
            return false;
        }
        let pts = self.flatten();
        let mut inside = false;
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }

    /// Axis-aligned bounds of the outline as `(min, max)`.
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let mut pts = self.flatten().into_iter();
        let first = pts.next()?;
        Some(pts.fold((first, first), |(lo, hi), p| {
            (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y)))
        }))
    }

    /// Center of the bounding box (the pivot for rotating and flipping).
    pub fn center(&self) -> Vec2 {
        self.bounds().map_or_else(Vec2::default, |(lo, hi)| lo.lerp(hi, 0.5))
    }

    /// Move every point (and turn its handles) by `iso`.
    pub fn transform(&mut self, iso: &Isometry) {
        for p in &mut self.points {
            p.pos = iso.apply(p.pos);
            p.handle_in = p.handle_in.map(|h| iso.linear(h));
            p.handle_out = p.handle_out.map(|h| iso.linear(h));
        }
    }

    /// Counter-clockwise rotation by `degrees` about the bounding-box center.
    pub fn rotation(&self, degrees: f64) -> Isometry {
        Isometry::rotation(self.center(), degrees)
    }

    /// Left-right (`horizontal`) or top-bottom mirror about the bounding-box center.
    pub fn flipping(&self, horizontal: bool) -> Isometry {
        let c = self.center();
        let along = if horizontal { Vec2::new(0.0, 1.0) } else { Vec2::new(1.0, 0.0) };
        Isometry::mirror(c, c + along)
    }

    /// Rotate counter-clockwise by `degrees` about the bounding-box center.
    pub fn rotate(&mut self, degrees: f64) {
        self.transform(&self.rotation(degrees));
    }

    /// Mirror left-right (`horizontal`) or top-bottom about the bounding-box center.
    pub fn flip(&mut self, horizontal: bool) {
        self.transform(&self.flipping(horizontal));
    }

    /// End point indices `(start, end)` of segment `seg`.
    pub fn segment_ends(&self, seg: usize) -> (usize, usize) {
        (seg, self.next(seg).expect("segment index out of range"))
    }

    pub fn segment_length(&self, seg: usize) -> f64 {
        self.flatten_segment(seg).windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }

    /// Change the length of segment `seg` by sliding one end point along the
    /// line between the ends; the end point moves unless `move_start`. Curved
    /// segments are solved for their arc length. Returns `false` if the
    /// segment is degenerate and has no direction.
    pub fn set_segment_length(&mut self, seg: usize, target: f64, move_start: bool) -> bool {
        let (a, b) = self.segment_ends(seg);
        let (fixed, moving) = if move_start { (b, a) } else { (a, b) };
        let origin = self.points[fixed].pos;
        let chord = self.points[moving].pos - origin;
        let chord_len = chord.length();
        if chord_len < 1e-9 || target <= 0.0 {
            return false;
        }
        let dir = chord * (1.0 / chord_len);
        let place = |p: &mut Self, s: f64| {
            p.points[moving].pos = origin + dir * s;
            p.segment_length(seg)
        };
        if !self.segment_is_curved(seg) {
            place(self, target);
            return true;
        }
        // Secant iteration on the chord length; arc length grows with it.
        let (mut s0, mut f0) = (chord_len, self.segment_length(seg) - target);
        let mut s1 = chord_len * target / (f0 + target);
        for _ in 0..50 {
            let f1 = place(self, s1) - target;
            if f1.abs() < 1e-6 || (f1 - f0).abs() < 1e-12 {
                break;
            }
            let next = (s1 - f1 * (s1 - s0) / (f1 - f0)).max(1e-6);
            (s0, f0, s1) = (s1, f1, next);
        }
        true
    }

    pub fn translate(&mut self, d: Vec2) {
        for p in &mut self.points {
            p.pos = p.pos + d;
        }
    }

    pub fn perimeter(&self) -> f64 {
        self.flatten().windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }
}

/// Mirror `p` across the infinite line through `a` and `b`.
pub fn reflect(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    if len2 == 0.0 {
        return p;
    }
    let foot = a + ab * ((p - a).dot(ab) / len2);
    foot * 2.0 - p
}

/// Closest point on segment `a`–`b` to `p`, with its parameter along the segment.
pub fn closest_on_line(a: Vec2, b: Vec2, p: Vec2) -> (Vec2, f64) {
    let ab = b - a;
    let len2 = ab.dot(ab);
    if len2 == 0.0 {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (a.lerp(b, t), t)
}

fn cross(o: Vec2, a: Vec2, b: Vec2) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// Triangulate a simple polygon (either winding, concave allowed) by ear
/// clipping. Self-intersecting input still yields triangles covering
/// roughly the right area rather than looping forever.
pub fn triangulate(poly: &[Vec2]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let signed_area: f64 =
        (0..n).map(|i| cross(Vec2::default(), poly[i], poly[(i + 1) % n])).sum();
    let mut idx: Vec<usize> = (0..n).collect();
    if signed_area < 0.0 {
        idx.reverse();
    }

    let mut tris = Vec::with_capacity(n - 2);
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (pa, pb, pc) = (poly[a], poly[b], poly[c]);
            if cross(pa, pb, pc) <= 0.0 {
                return false; // reflex or degenerate corner
            }
            idx.iter().all(|&j| {
                j == a || j == b || j == c || {
                    let p = poly[j];
                    // Outside (or on the edge of) triangle abc.
                    cross(pa, pb, p) < 0.0 || cross(pb, pc, p) < 0.0 || cross(pc, pa, p) < 0.0
                }
            })
        });
        // No ear means degenerate/self-intersecting input: clip anyway.
        let i = ear.unwrap_or(0);
        tris.push([idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]]);
        idx.remove(i);
    }
    tris.push([idx[0], idx[1], idx[2]]);
    tris
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub pieces: Vec<Piece>,
    #[serde(default)]
    pub seams: Vec<Seam>,
    /// Pieces and halves of pieces edited as mirror images of each other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symmetries: Vec<Symmetry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Piece {
        let mut p = Piece::new("sq");
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)] {
            p.points.push(Point::turn(Vec2::new(x, y)));
        }
        p.closed = true;
        p
    }

    #[test]
    fn square_area_and_perimeter() {
        let p = square();
        assert_eq!(p.segment_count(), 4);
        assert!((p.area() - 10_000.0).abs() < 1e-9);
        assert!((p.perimeter() - 400.0).abs() < 1e-9);
    }

    #[test]
    fn insert_on_closing_segment() {
        let mut p = square();
        let (seg, t, pos, d) = p.closest_on_outline(Vec2::new(-5.0, 50.0)).unwrap();
        assert_eq!(seg, 3);
        assert!((d - 5.0).abs() < 1e-9);
        assert_eq!(pos, Vec2::new(0.0, 50.0));
        let idx = p.insert_on_segment(seg, t);
        assert_eq!(idx, 4);
        assert_eq!(p.points[4].pos, Vec2::new(0.0, 50.0));
        assert!((p.area() - 10_000.0).abs() < 1e-9);
    }

    #[test]
    fn insert_on_straight_segment_is_linear() {
        let mut p = square();
        let idx = p.insert_on_segment(0, 0.2);
        assert_eq!(p.points[idx].pos, Vec2::new(20.0, 0.0));
    }

    #[test]
    fn curve_points_pass_through_endpoints() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        assert!(p.segment_is_curved(0));
        assert_eq!(p.eval_segment(0, 0.0), p.points[0].pos);
        assert_eq!(p.eval_segment(0, 1.0), p.points[1].pos);
        assert!(!p.segment_is_curved(2));
    }

    fn tri_area(poly: &[Vec2], tris: &[[usize; 3]]) -> f64 {
        tris.iter().map(|t| cross(poly[t[0]], poly[t[1]], poly[t[2]]).abs() * 0.5).sum()
    }

    #[test]
    fn triangulate_concave_both_windings() {
        // L-shape, area 3 * 100 * 100.
        let mut poly: Vec<Vec2> = [(0, 0), (200, 0), (200, 100), (100, 100), (100, 200), (0, 200)]
            .iter()
            .map(|&(x, y)| Vec2::new(x as f64, y as f64))
            .collect();
        for _ in 0..2 {
            let tris = triangulate(&poly);
            assert_eq!(tris.len(), poly.len() - 2);
            assert!((tri_area(&poly, &tris) - 30_000.0).abs() < 1e-6);
            poly.reverse();
        }
    }

    #[test]
    fn closed_piece_fill_matches_area() {
        let mut p = square();
        p.points[1].kind = PointKind::Curve;
        let (poly, tris) = p.fill_triangles();
        assert!((tri_area(&poly, &tris) - p.area().abs()).abs() < 1e-6);
        p.closed = false;
        assert!(p.fill_triangles().1.is_empty());
    }

    #[test]
    fn contains_and_translate() {
        let mut p = square();
        assert!(p.contains(Vec2::new(50.0, 50.0)));
        assert!(!p.contains(Vec2::new(150.0, 50.0)));
        p.translate(Vec2::new(100.0, 0.0));
        assert!(p.contains(Vec2::new(150.0, 50.0)));
        p.closed = false;
        assert!(!p.contains(Vec2::new(150.0, 50.0)));
    }

    #[test]
    fn rotate_and_flip_about_center() {
        let mut p = square();
        p.points[2].pos = Vec2::new(100.0, 200.0); // make it asymmetric
        let c = p.center();
        p.rotate(90.0);
        assert!((p.center() - c).length() < 1e-9);
        // (0,0) rotated 90° CCW about (50,100) lands at (150,50).
        assert!((p.points[0].pos - Vec2::new(150.0, 50.0)).length() < 1e-9);
        let mut q = square();
        q.flip(true);
        assert_eq!(q.points[0].pos, Vec2::new(100.0, 0.0));
        q.flip(false);
        assert_eq!(q.points[0].pos, Vec2::new(100.0, 100.0));
    }

    #[test]
    fn set_straight_and_curved_segment_length() {
        let mut p = square();
        assert!(p.set_segment_length(0, 150.0, false));
        assert_eq!(p.points[1].pos, Vec2::new(150.0, 0.0));
        assert!(p.set_segment_length(0, 50.0, true));
        assert_eq!(p.points[0].pos, Vec2::new(100.0, 0.0));

        let mut c = square();
        c.points[1].kind = PointKind::Curve;
        for target in [80.0, 140.0, 200.0] {
            assert!(c.set_segment_length(0, target, false));
            assert!((c.segment_length(0) - target).abs() < 1e-4, "{}", c.segment_length(0));
        }
    }

    #[test]
    fn reflect_across_line() {
        let r = reflect(Vec2::new(1.0, 2.0), Vec2::new(0.0, 0.0), Vec2::new(0.0, 5.0));
        assert_eq!(r, Vec2::new(-1.0, 2.0));
    }

    #[test]
    fn open_piece_segments() {
        let mut p = square();
        p.closed = false;
        assert_eq!(p.segment_count(), 3);
        assert_eq!(p.flatten().len(), 4);
    }
}

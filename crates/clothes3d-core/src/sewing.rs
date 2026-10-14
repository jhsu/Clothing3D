//! Free sewing: pairs of outline spans that are sewn together.
//!
//! A span is stored as two positions on a piece outline (segment index plus
//! parameter), so it follows the outline when points or pieces move. Spans
//! are directional: the start of span `a` is sewn to the start of span `b`.

use serde::{Deserialize, Serialize};

use crate::model::{Pattern, Piece, Point, Vec2, reflect};

/// Samples per curved segment when measuring spans. Finer than display
/// sampling so lengths agree between the length readout and length matching.
const SPAN_SAMPLES: usize = 64;

/// A location on a piece outline: segment `seg` at parameter `t` in [0, 1].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutlinePos {
    pub seg: usize,
    pub t: f64,
}

impl OutlinePos {
    pub const fn new(seg: usize, t: f64) -> Self {
        Self { seg, t }
    }

    fn param(self) -> f64 {
        self.seg as f64 + self.t
    }
}

/// A directional stretch of one piece's outline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeSpan {
    pub piece: usize,
    pub start: OutlinePos,
    pub end: OutlinePos,
    /// Walk the outline in point order (`true`) or against it.
    pub forward: bool,
}

impl EdgeSpan {
    /// Span from `start` to `end` taking the shorter way around the outline
    /// (open outlines have only one way).
    pub fn shortest(pieces: &[Piece], piece: usize, start: OutlinePos, end: OutlinePos) -> Self {
        let p = &pieces[piece];
        let forward = if p.closed {
            polyline_length(&p.walk(start, end)) <= polyline_length(&p.walk(end, start))
        } else {
            end.param() >= start.param()
        };
        Self { piece, start, end, forward }
    }

    pub fn reversed(self) -> Self {
        Self { start: self.end, end: self.start, forward: !self.forward, ..self }
    }

    pub fn polyline(&self, pieces: &[Piece]) -> Vec<Vec2> {
        let p = &pieces[self.piece];
        if self.forward {
            p.walk(self.start, self.end)
        } else {
            let mut pts = p.walk(self.end, self.start);
            pts.reverse();
            pts
        }
    }

    pub fn length(&self, pieces: &[Piece]) -> f64 {
        polyline_length(&self.polyline(pieces))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seam {
    pub a: EdgeSpan,
    pub b: EdgeSpan,
}

impl Seam {
    pub fn spans(&self) -> [EdgeSpan; 2] {
        [self.a, self.b]
    }

    fn spans_mut(&mut self) -> [&mut EdgeSpan; 2] {
        [&mut self.a, &mut self.b]
    }
}

impl Piece {
    pub fn eval_pos(&self, pos: OutlinePos) -> Vec2 {
        self.eval_segment(pos.seg, pos.t)
    }

    /// Outline position of point `i` (the last point of an open outline is
    /// the end of the final segment).
    pub fn point_pos(&self, i: usize) -> OutlinePos {
        if i >= self.segment_count() {
            OutlinePos::new(self.segment_count().saturating_sub(1), 1.0)
        } else {
            OutlinePos::new(i, 0.0)
        }
    }

    /// Samples of segment `seg` between parameters `t0` and `t1`.
    fn sub_segment(&self, seg: usize, t0: f64, t1: f64) -> Vec<Vec2> {
        let steps = if self.segment_is_curved(seg) {
            ((SPAN_SAMPLES as f64 * (t1 - t0)).ceil() as usize).max(1)
        } else {
            1
        };
        (0..=steps)
            .map(|k| self.eval_segment(seg, t0 + (t1 - t0) * k as f64 / steps as f64))
            .collect()
    }

    /// Walk the outline in point order from `s` to `e`, wrapping around a
    /// closed outline if needed.
    pub fn walk(&self, s: OutlinePos, e: OutlinePos) -> Vec<Vec2> {
        let n = self.segment_count();
        let mut out: Vec<Vec2> = Vec::new();
        let (mut seg, mut t) = (s.seg, s.t);
        for _ in 0..=n {
            let done = seg == e.seg && e.t >= t;
            let part = self.sub_segment(seg, t, if done { e.t } else { 1.0 });
            let skip = usize::from(!out.is_empty());
            out.extend(part.into_iter().skip(skip));
            if done || n == 0 {
                break;
            }
            seg = (seg + 1) % n;
            t = 0.0;
        }
        out
    }
}

impl Piece {
    /// Position `dist` mm along the outline from `start`, walking in point
    /// order (`forward`) or against it. `None` if an open outline ends first
    /// or `dist` exceeds a full lap.
    pub fn pos_at_distance(&self, start: OutlinePos, dist: f64, forward: bool) -> Option<OutlinePos> {
        let n = self.segment_count();
        let mut remaining = dist;
        let (mut seg, mut t) = (start.seg, start.t);
        for lap in 0..=n {
            // The final pass ends back at the start: one full lap at most.
            let t_end = if lap == n { start.t } else if forward { 1.0 } else { 0.0 };
            let steps = if self.segment_is_curved(seg) { SPAN_SAMPLES } else { 1 };
            let (mut prev_t, mut prev) = (t, self.eval_segment(seg, t));
            for k in 1..=steps {
                let nt = t + (t_end - t) * k as f64 / steps as f64;
                let p = self.eval_segment(seg, nt);
                let len = (p - prev).length();
                if len >= remaining && len > 0.0 {
                    return Some(OutlinePos::new(seg, prev_t + (nt - prev_t) * remaining / len));
                }
                remaining -= len;
                (prev_t, prev) = (nt, p);
            }
            if forward {
                seg = if seg + 1 < n { seg + 1 } else if self.closed { 0 } else { return None };
                t = 0.0;
            } else {
                seg = if seg > 0 { seg - 1 } else if self.closed { n - 1 } else { return None };
                t = 1.0;
            }
        }
        None
    }
}

pub fn polyline_length(pts: &[Vec2]) -> f64 {
    pts.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

/// Point at fraction `f` of the polyline's length.
pub fn point_at_fraction(pts: &[Vec2], f: f64) -> Vec2 {
    let mut remaining = polyline_length(pts) * f.clamp(0.0, 1.0);
    for w in pts.windows(2) {
        let len = (w[1] - w[0]).length();
        if remaining <= len && len > 0.0 {
            return w[0].lerp(w[1], remaining / len);
        }
        remaining -= len;
    }
    pts.last().copied().unwrap_or_default()
}

// ------------------------------------------------------------------------
// Pattern edits that keep seams attached to their outlines.

impl Pattern {
    fn remap_positions(&mut self, piece: usize, f: impl Fn(OutlinePos) -> OutlinePos) {
        for seam in &mut self.seams {
            for span in seam.spans_mut() {
                if span.piece == piece {
                    span.start = f(span.start);
                    span.end = f(span.end);
                }
            }
        }
    }

    /// Insert a point on an outline segment, ignoring symmetry links.
    /// Returns the new point's index.
    pub(crate) fn insert_point_raw(&mut self, piece: usize, seg: usize, t0: f64) -> usize {
        let idx = self.pieces[piece].insert_on_segment(seg, t0);
        self.remap_positions(piece, |p| {
            if p.seg > seg {
                OutlinePos::new(p.seg + 1, p.t)
            } else if p.seg < seg {
                p
            } else if p.t < t0 {
                OutlinePos::new(seg, p.t / t0)
            } else if t0 < 1.0 {
                OutlinePos::new(seg + 1, (p.t - t0) / (1.0 - t0))
            } else {
                OutlinePos::new(seg + 1, 0.0)
            }
        });
        idx
    }

    /// Remove point `i`, ignoring symmetry links, opening the outline below 3
    /// points and removing the piece once empty. Returns `true` if the whole
    /// piece was removed.
    pub(crate) fn remove_point_raw(&mut self, piece: usize, i: usize) -> bool {
        let p = &mut self.pieces[piece];
        let n = p.points.len();
        let was_closed = p.closed;
        p.points.remove(i);
        if p.points.len() < 3 {
            p.closed = false;
        }
        if p.points.is_empty() {
            self.remove_piece(piece);
            return true;
        }

        if was_closed && !self.pieces[piece].closed {
            // Closing segment vanished; seams on this piece can't be kept.
            self.seams.retain(|s| s.spans().iter().all(|sp| sp.piece != piece));
        } else if i == 0 && !was_closed {
            self.remap_positions(piece, |p| {
                if p.seg == 0 { OutlinePos::new(0, 0.0) } else { OutlinePos::new(p.seg - 1, p.t) }
            });
        } else if i == n - 1 && !was_closed {
            let last = n.saturating_sub(3);
            self.remap_positions(piece, |p| {
                if p.seg >= n - 2 { OutlinePos::new(last, 1.0) } else { p }
            });
        } else {
            // Segments before and after the point merge into one.
            let prev = (i + n - 1) % n;
            let merged = if i > 0 { i - 1 } else { n - 2 };
            self.remap_positions(piece, |p| {
                if p.seg == prev {
                    OutlinePos::new(merged, p.t * 0.5)
                } else if p.seg == i {
                    OutlinePos::new(merged, 0.5 + p.t * 0.5)
                } else if p.seg > i {
                    OutlinePos::new(p.seg - 1, p.t)
                } else {
                    p
                }
            });
        }
        self.validate_seams();
        false
    }

    /// Copy a piece (without its seams), placed to the right of the original.
    /// Returns the new piece's index.
    pub fn duplicate_piece(&mut self, piece: usize) -> usize {
        let mut copy = self.pieces[piece].clone();
        copy.name = format!("{} copy", copy.name);
        copy.translate(self.duplicate_offset(piece));
        self.pieces.push(copy);
        self.pieces.len() - 1
    }

    /// How far [`Self::duplicate_piece`] moves the copy: just clear to the right.
    pub(crate) fn duplicate_offset(&self, piece: usize) -> Vec2 {
        self.pieces[piece].bounds().map_or_else(Vec2::default, |(lo, hi)| Vec2::new(hi.x - lo.x + 20.0, 0.0))
    }

    /// Unfold a closed piece across a fold line made of the straight,
    /// contiguous, collinear segments `segs`: the piece is mirrored over that
    /// line and both halves merge into one symmetric outline. Points inside
    /// the fold line are dropped. Seams on the original half are kept; seams
    /// on the fold are dropped, and so are symmetry links on the piece.
    pub fn unfold(&mut self, piece: usize, segs: &[usize]) -> Result<(), &'static str> {
        self.unfold_along(piece, segs).map(|_| ())
    }

    /// [`Self::unfold`], returning the fold line's end points. In the
    /// unfolded outline point `i` mirrors point `(n - i) % n`.
    pub(crate) fn unfold_along(&mut self, piece: usize, segs: &[usize]) -> Result<(Vec2, Vec2), &'static str> {
        let p = &self.pieces[piece];
        let n = p.points.len();
        if !p.closed || n < 3 {
            return Err("Unfold needs a closed piece");
        }
        if segs.is_empty() || segs.iter().any(|&s| s >= n) {
            return Err("Select the fold edge first");
        }
        if segs.iter().any(|&s| p.segment_is_curved(s)) {
            return Err("The fold edge must be straight (no curve points)");
        }
        let in_fold = |s: usize| segs.contains(&s);
        let m = (0..n).filter(|&s| in_fold(s)).count();
        // The chain starts at the only selected segment whose predecessor isn't selected.
        let starts: Vec<usize> = (0..n).filter(|&s| in_fold(s) && !in_fold((s + n - 1) % n)).collect();
        let [k] = starts[..] else {
            return Err("The selected edges must be connected end to end");
        };
        if m >= n - 1 {
            return Err("The fold can't use (almost) the whole outline");
        }
        let (fa, fb) = (p.points[k].pos, p.points[(k + m) % n].pos);
        let len = (fb - fa).length();
        if len < 1e-9 {
            return Err("The fold edge has zero length");
        }
        let off_line = (1..m).any(|i| {
            let q = p.points[(k + i) % n].pos;
            let d = fb - fa;
            ((q.x - fa.x) * d.y - (q.y - fa.y) * d.x).abs() / len > 0.5
        });
        if off_line {
            return Err("The selected edges aren't on one straight line");
        }

        // Walk from the fold's end point around to its start point, then
        // back through the mirrored interior points.
        let b = (k + m) % n;
        let mut half: Vec<Point> = (0..=n - m).map(|j| p.points[(b + j) % n]).collect();
        // Handles are offsets, so they reflect across the fold direction alone.
        let dir = fb - fa;
        let reflect_offset = |v: Vec2| reflect(v, Vec2::default(), dir);
        // The mirrored half runs in reverse, so incoming and outgoing swap.
        // A mirrored segment runs backwards, so it starts at the mirror of the
        // original segment's end point and takes that segment's allowance.
        let mirrored = (1..half.len() - 1).rev().map(|i| Point {
            pos: reflect(half[i].pos, fa, fb),
            kind: half[i].kind,
            handle_in: half[i].handle_out.map(reflect_offset),
            handle_out: half[i].handle_in.map(reflect_offset),
            allowance: half[i - 1].allowance,
        });
        let mirrored: Vec<Point> = mirrored.collect();
        // The fold ends now join their own mirror images.
        let last = half.len() - 1;
        half[last].allowance = half[last - 1].allowance;
        half[0].handle_in = half[0].handle_out.map(reflect_offset);
        half[last].handle_out = half[last].handle_in.map(reflect_offset);
        let points: Vec<Point> = half.into_iter().chain(mirrored).collect();

        self.seams.retain(|s| {
            s.spans()
                .iter()
                .all(|sp| sp.piece != piece || (!in_fold(sp.start.seg) && !in_fold(sp.end.seg)))
        });
        // Original segment j now starts at index (j - b) mod n.
        self.remap_positions(piece, |pos| OutlinePos::new((pos.seg + n - b) % n, pos.t));
        self.pieces[piece].points = points;
        self.validate_symmetries();
        Ok((fa, fb))
    }

    pub fn remove_piece(&mut self, piece: usize) {
        self.pieces.remove(piece);
        self.seams.retain(|s| s.spans().iter().all(|sp| sp.piece != piece));
        for seam in &mut self.seams {
            for span in seam.spans_mut() {
                if span.piece > piece {
                    span.piece -= 1;
                }
            }
        }
        self.symmetries.retain(|s| s.a != piece && s.b != piece);
        for s in &mut self.symmetries {
            s.a -= usize::from(s.a > piece);
            s.b -= usize::from(s.b > piece);
        }
    }

    /// Drop seams whose spans no longer fit their outlines.
    fn validate_seams(&mut self) {
        let pieces = &self.pieces;
        self.seams.retain(|s| {
            s.spans().iter().all(|sp| {
                pieces.get(sp.piece).is_some_and(|p| {
                    let n = p.segment_count();
                    sp.start.seg < n && sp.end.seg < n
                })
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Point, PointKind};

    fn square(x: f64) -> Piece {
        let mut p = Piece::new("sq");
        for (px, py) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)] {
            p.points.push(Point::turn(Vec2::new(x + px, py)));
        }
        p.closed = true;
        p
    }

    #[test]
    fn shortest_span_wraps_around_closing_segment() {
        let pieces = vec![square(0.0)];
        // From the middle of the left edge (closing segment) to the middle of the bottom.
        let span = EdgeSpan::shortest(&pieces, 0, OutlinePos::new(3, 0.5), OutlinePos::new(0, 0.5));
        assert!(span.forward);
        assert!((span.length(&pieces) - 100.0).abs() < 1e-9);
        let pts = span.polyline(&pieces);
        assert_eq!(pts.first(), Some(&Vec2::new(0.0, 50.0)));
        assert_eq!(pts.last(), Some(&Vec2::new(50.0, 0.0)));

        // Reverse clicks: shortest way is now backwards.
        let back = EdgeSpan::shortest(&pieces, 0, OutlinePos::new(0, 0.5), OutlinePos::new(3, 0.5));
        assert!(!back.forward);
        assert!((back.length(&pieces) - 100.0).abs() < 1e-9);
        assert_eq!(back.polyline(&pieces).first(), Some(&Vec2::new(50.0, 0.0)));
    }

    #[test]
    fn same_segment_reverse_goes_around() {
        let pieces = vec![square(0.0)];
        let mut span =
            EdgeSpan::shortest(&pieces, 0, OutlinePos::new(0, 0.2), OutlinePos::new(0, 0.8));
        assert!((span.length(&pieces) - 60.0).abs() < 1e-9);
        span.forward = !span.forward;
        assert!((span.length(&pieces) - 340.0).abs() < 1e-9);
    }

    fn pattern_with_seam() -> Pattern {
        let pieces = vec![square(0.0), square(200.0)];
        let a = EdgeSpan::shortest(&pieces, 0, OutlinePos::new(1, 0.0), OutlinePos::new(1, 1.0));
        let b = EdgeSpan::shortest(&pieces, 1, OutlinePos::new(3, 1.0), OutlinePos::new(3, 0.0));
        Pattern { pieces, seams: vec![Seam { a, b }], ..Pattern::default() }
    }

    #[test]
    fn insert_point_keeps_seam_geometry() {
        let mut pat = pattern_with_seam();
        let before = pat.seams[0].a.polyline(&pat.pieces);
        pat.insert_point(0, 1, 0.25);
        pat.insert_point(0, 0, 0.5); // before the span: shifts indices
        let after = pat.seams[0].a.polyline(&pat.pieces);
        assert_eq!(before.first(), after.first());
        assert_eq!(before.last(), after.last());
        assert!((polyline_length(&after) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn remove_point_and_piece_update_seams() {
        let mut pat = pattern_with_seam();
        pat.insert_point(0, 0, 0.5);
        assert!(!pat.remove_point(0, 1));
        assert_eq!(pat.seams[0].a.start.seg, 1);
        assert!((pat.seams[0].a.length(&pat.pieces) - 100.0).abs() < 1e-9);

        pat.pieces.insert(0, square(-200.0));
        for s in &mut pat.seams {
            s.a.piece += 1;
            s.b.piece += 1;
        }
        pat.remove_piece(0);
        assert_eq!((pat.seams[0].a.piece, pat.seams[0].b.piece), (0, 1));
        pat.remove_piece(1);
        assert!(pat.seams.is_empty());
    }

    #[test]
    fn distance_along_outline() {
        let mut p = square(0.0);
        let start = OutlinePos::new(0, 0.5); // (50, 0)
        let fwd = p.pos_at_distance(start, 120.0, true).unwrap();
        assert_eq!(p.eval_pos(fwd), Vec2::new(100.0, 70.0));
        // Backwards wraps through point 0 onto the closing segment.
        let back = p.pos_at_distance(start, 120.0, false).unwrap();
        assert_eq!(back.seg, 3);
        assert!((p.eval_pos(back) - Vec2::new(0.0, 70.0)).length() < 1e-9);
        assert!(p.pos_at_distance(start, 401.0, true).is_none());

        p.closed = false;
        assert!(p.pos_at_distance(start, 120.0, false).is_none());
        assert!(p.pos_at_distance(start, 250.0, true).is_some());
        assert!(p.pos_at_distance(start, 251.0, true).is_none());
    }

    #[test]
    fn matched_span_length_on_curve() {
        let mut p = square(0.0);
        p.points[1].kind = PointKind::Curve;
        p.points[2].kind = PointKind::Curve;
        let pieces = vec![p];
        let start = OutlinePos::new(0, 0.3);
        for (dist, forward) in [(37.0, true), (180.0, true), (95.5, false)] {
            let end = pieces[0].pos_at_distance(start, dist, forward).unwrap();
            let span = EdgeSpan { piece: 0, start, end, forward };
            assert!((span.length(&pieces) - dist).abs() < 0.01, "{dist}: {}", span.length(&pieces));
        }
    }

    #[test]
    fn unfold_mirrors_and_keeps_seams() {
        // Half of a 200 x 100 rectangle, fold along the right edge x = 100.
        let mut pat = pattern_with_seam();
        pat.seams[0].a.start = OutlinePos::new(0, 0.0); // bottom edge of piece 0
        pat.seams[0].a.end = OutlinePos::new(0, 1.0);
        let before = pat.seams[0].a.polyline(&pat.pieces);
        let area = pat.pieces[0].area().abs();

        assert_eq!(pat.unfold(0, &[1]), Ok(()));
        let p = &pat.pieces[0];
        assert_eq!(p.points.len(), 6);
        assert!((p.area().abs() - 2.0 * area).abs() < 1e-9);
        let (lo, hi) = p.bounds().unwrap();
        assert_eq!((lo, hi), (Vec2::new(0.0, 0.0), Vec2::new(200.0, 100.0)));
        assert_eq!(pat.seams[0].a.polyline(&pat.pieces), before);

        // A seam on the fold segment itself is dropped.
        let mut pat = pattern_with_seam();
        assert_eq!(pat.unfold(0, &[1]), Ok(()));
        assert!(pat.seams.is_empty());
    }

    #[test]
    fn unfold_along_multi_segment_edge() {
        let mut pat = pattern_with_seam();
        pat.seams.clear();
        // Split the right edge into three collinear segments: 1, 2, 3.
        pat.insert_point(0, 1, 0.25);
        pat.insert_point(0, 2, 0.5);
        assert_eq!(pat.pieces[0].points.len(), 6);
        // Selection order doesn't matter.
        assert_eq!(pat.unfold(0, &[3, 1, 2]), Ok(()));
        let p = &pat.pieces[0];
        assert_eq!(p.points.len(), 6); // interior fold points are dropped
        assert!((p.area().abs() - 20_000.0).abs() < 1e-9);
        assert_eq!(p.bounds(), Some((Vec2::new(0.0, 0.0), Vec2::new(200.0, 100.0))));
    }

    #[test]
    fn unfold_mirrors_handles() {
        let mut pat = pattern_with_seam();
        pat.seams.clear();
        // Bend the top edge (segment 2) upward, then unfold on the right edge.
        let start = pat.pieces[0].segment_handles(2);
        pat.pieces[0].bend_segment(2, 0.5, start, Vec2::new(0.0, 30.0));
        let half_area = pat.pieces[0].area().abs();
        assert_eq!(pat.unfold(0, &[1]), Ok(()));
        let p = &pat.pieces[0];
        assert!((p.area().abs() - 2.0 * half_area).abs() < 1e-6);
        let (lo, hi) = p.bounds().unwrap();
        // Symmetric about x = 100, with the bulge on both halves.
        assert!(((lo.x + hi.x) / 2.0 - 100.0).abs() < 1e-6);
        assert!(hi.y > 120.0);
    }

    #[test]
    fn unfold_rejects_bad_folds() {
        let mut pat = pattern_with_seam();
        pat.insert_point(0, 1, 0.5);
        // Not connected: segments 0 and 2 (bottom edge and upper right half).
        assert!(pat.unfold(0, &[0, 2]).is_err());
        // Connected but not collinear: bottom edge + lower right half.
        assert!(pat.unfold(0, &[0, 1]).is_err());
        // Nudge the middle point off the line: collinear check fails.
        pat.pieces[0].points[2].pos.x += 5.0;
        assert!(pat.unfold(0, &[1, 2]).is_err());
        assert_eq!(pat.pieces[0].points.len(), 5); // unchanged on error
    }

    #[test]
    fn duplicate_offsets_copy() {
        let mut pat = pattern_with_seam();
        let i = pat.duplicate_piece(0);
        assert_eq!(pat.pieces[i].name, "sq copy");
        assert_eq!(pat.pieces[i].points[0].pos, Vec2::new(120.0, 0.0));
        assert_eq!(pat.seams.len(), 1);
    }

    #[test]
    fn fraction_along_polyline() {
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        assert_eq!(point_at_fraction(&pts, 0.75), Vec2::new(10.0, 5.0));
        assert_eq!(point_at_fraction(&pts, 1.0), Vec2::new(10.0, 10.0));
    }
}

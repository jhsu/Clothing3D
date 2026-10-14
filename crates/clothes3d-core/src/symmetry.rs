//! Symmetric editing: two pieces, or the two halves of one piece, linked
//! across a mirror line so that an edit on one side is repeated, mirrored,
//! on the other.
//!
//! A link maps piece `a` onto piece `b` (the same piece for two halves) with
//! an isometry and pairs every point of `a` with its image on `b`. The two
//! halves of a piece run in opposite directions around the outline, so a
//! point's incoming handle mirrors its partner's outgoing one; a mirrored
//! copy of a whole piece keeps the point order.
//!
//! Shape edits are made to one side and then copied across by
//! [`Pattern::mirror_changes`]. Adding and removing points
//! ([`Pattern::insert_point`], [`Pattern::remove_points`]) changes both sides
//! at once, and moving a whole piece ([`Pattern::transform_piece`]) carries
//! its links along instead of moving its partner.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::{Isometry, Pattern, Piece, Point, PointKind, Vec2};

/// Coordinates closer than this (mm) count as equal.
const EPS: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symmetry {
    pub a: usize,
    /// The mirror-image piece; the same as `a` for the two halves of one piece.
    pub b: usize,
    /// Maps piece `a` onto piece `b`.
    pub map: Isometry,
    /// `pairs[i]` is the point of `b` that mirrors point `i` of `a`.
    pub pairs: Vec<usize>,
    /// Whether `b`'s outline runs the opposite way to `a`'s (always for halves).
    pub reversed: bool,
}

impl Symmetry {
    pub fn is_halves(&self) -> bool {
        self.a == self.b
    }

    pub fn touches(&self, piece: usize) -> bool {
        self.a == piece || self.b == piece
    }

    /// The mirror line `(point, unit direction)`; `None` once one of two
    /// linked pieces was flipped, making them plain (unmirrored) copies.
    pub fn axis(&self) -> Option<(Vec2, Vec2)> {
        self.map.mirror_line()
    }

    fn is_valid(&self, pieces: &[Piece]) -> bool {
        let (Some(a), Some(b)) = (pieces.get(self.a), pieces.get(self.b)) else { return false };
        let n = self.pairs.len();
        let mut seen = vec![false; n];
        a.points.len() == n
            && b.points.len() == n
            && self.pairs.iter().all(|&j| j < n && !std::mem::replace(&mut seen[j], true))
            && (!self.is_halves() || (a.closed && self.reversed))
    }

    /// Where point `i` of piece `from` is mirrored to: `(piece, point, map)`.
    fn partner(&self, from: usize, i: usize) -> Option<(usize, usize, Isometry)> {
        if from == self.a {
            Some((self.b, *self.pairs.get(i)?, self.map))
        } else if from == self.b {
            Some((self.a, self.pairs.iter().position(|&j| j == i)?, self.map.inverse()))
        } else {
            None
        }
    }

    /// The segment `(piece, segment)` mirroring segment `seg` of piece
    /// `from`. With `reversed` it runs the other way, so parameter `t` on
    /// one is `1 - t` on the other.
    fn partner_segment(&self, pieces: &[Piece], from: usize, seg: usize) -> Option<(usize, usize)> {
        let (i, j) = pieces[from].segment_ends(seg);
        let (to, pi, _) = self.partner(from, i)?;
        let (_, pj, _) = self.partner(from, j)?;
        let (start, end) = if self.reversed { (pj, pi) } else { (pi, pj) };
        (pieces[to].next(start) == Some(end)).then_some((to, start))
    }
}

fn same_vec(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < EPS
}

fn same_handle(a: Option<Vec2>, b: Option<Vec2>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_vec(a, b),
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// Same position, kind and handles (allowance aside).
fn same_shape(p: &Point, q: &Point) -> bool {
    same_vec(p.pos, q.pos)
        && p.kind == q.kind
        && same_handle(p.handle_in, q.handle_in)
        && same_handle(p.handle_out, q.handle_out)
}

/// `target` with its shape (not its allowance) replaced by the mirror image of `p`.
fn mirrored(p: &Point, map: &Isometry, reversed: bool, target: Point) -> Point {
    let (hin, hout) = (p.handle_in.map(|h| map.linear(h)), p.handle_out.map(|h| map.linear(h)));
    let (handle_in, handle_out) = if reversed { (hout, hin) } else { (hin, hout) };
    Point { pos: map.apply(p.pos), kind: p.kind, handle_in, handle_out, ..target }
}

impl Pattern {
    /// Drop links that no longer fit their pieces, e.g. after points were
    /// added to or removed from one side only.
    pub fn validate_symmetries(&mut self) {
        let pieces = &self.pieces;
        self.symmetries.retain(|s| s.is_valid(pieces));
    }

    pub fn is_linked(&self, piece: usize) -> bool {
        self.symmetries.iter().any(|s| s.touches(piece))
    }

    /// Remove every symmetry link of `piece`; both sides become independent.
    pub fn unlink(&mut self, piece: usize) {
        self.symmetries.retain(|s| !s.touches(piece));
    }

    /// Every point linked to point `i` of `piece`, directly or through other
    /// links, not counting the point itself.
    pub fn mirror_images(&self, piece: usize, i: usize) -> Vec<(usize, usize)> {
        let mut found = vec![(piece, i)];
        let mut k = 0;
        while k < found.len() {
            let (pi, pt) = found[k];
            for s in &self.symmetries {
                if let Some((to, j, _)) = s.partner(pi, pt)
                    && self.pieces.get(to).is_some_and(|p| j < p.points.len())
                    && !found.contains(&(to, j))
                {
                    found.push((to, j));
                }
            }
            k += 1;
        }
        found.remove(0);
        found
    }

    // ------------------------------------------------------------ linking

    /// Unfold `piece` like [`Pattern::unfold`] and link the two halves, so
    /// that later edits to one half are mirrored onto the other.
    pub fn unfold_symmetric(&mut self, piece: usize, segs: &[usize]) -> Result<(), &'static str> {
        let (fa, fb) = self.unfold_along(piece, segs)?;
        let n = self.pieces[piece].points.len();
        self.symmetries.push(Symmetry {
            a: piece,
            b: piece,
            map: Isometry::mirror(fa, fb),
            pairs: (0..n).map(|i| (n - i) % n).collect(),
            reversed: true,
        });
        Ok(())
    }

    /// Copy `piece` next to the original, flip the copy left-right and link
    /// the two, so that later edits to either are mirrored onto the other.
    /// Returns the copy's index.
    pub fn duplicate_mirrored(&mut self, piece: usize) -> usize {
        let shift = Isometry::translation(self.duplicate_offset(piece));
        let copy = self.duplicate_piece(piece);
        let flip = self.pieces[copy].flipping(true);
        self.pieces[copy].transform(&flip);
        self.pieces[copy].name = format!("{} mirrored", self.pieces[piece].name);
        let n = self.pieces[piece].points.len();
        self.symmetries.push(Symmetry {
            a: piece,
            b: copy,
            map: shift.then(&flip),
            pairs: (0..n).collect(),
            reversed: false,
        });
        copy
    }

    // ------------------------------------------------------ whole pieces

    /// Move, rotate or flip a whole piece by `iso`. Its links move with it:
    /// a linked partner piece stays where it is, and the mirror line between
    /// the halves of the piece moves along.
    pub fn transform_piece(&mut self, piece: usize, iso: &Isometry) {
        self.pieces[piece].transform(iso);
        let inv = iso.inverse();
        for s in &mut self.symmetries {
            if s.a == piece {
                s.map = inv.then(&s.map);
            }
            if s.b == piece {
                s.map = s.map.then(iso);
            }
        }
    }

    // -------------------------------------------------- mirroring edits

    /// Repeat on the linked side every shape change made since `before`: a
    /// changed point's partner moves to its mirror image and takes its kind
    /// and mirrored handles, and segment and piece allowances follow. Where
    /// both sides of a link changed, the side of `focus` (the point being
    /// edited) wins. Points on a piece's own mirror line are kept on it.
    ///
    /// Pieces whose number of points changed are skipped: adding and
    /// removing points mirrors itself. Returns whether anything changed.
    pub fn mirror_changes(&mut self, before: &Pattern, focus: Option<(usize, usize)>) -> bool {
        self.validate_symmetries();
        if self.symmetries.is_empty() || before.pieces.len() != self.pieces.len() {
            return false;
        }
        let mut points = Vec::new();
        let mut segments = Vec::new();
        for (pi, (now, old)) in self.pieces.iter().zip(&before.pieces).enumerate() {
            if now.points.len() != old.points.len() || !self.is_linked(pi) {
                continue;
            }
            for (i, (p, q)) in now.points.iter().zip(&old.points).enumerate() {
                if !same_shape(p, q) {
                    points.push(((pi, i), *q));
                }
                if p.allowance != q.allowance && i < now.segment_count() {
                    segments.push((pi, i));
                }
            }
        }
        // The focus point first, then the rest of its side, then the others.
        points.sort_by_key(|&((pi, i), _)| match focus {
            Some(f) if f == (pi, i) => 0,
            Some(f) if f.0 == pi && self.same_side(before, pi, i, f.1) => 1,
            Some(_) => 2,
            None => 0,
        });
        let mut changed = false;
        for (src, old) in points {
            changed |= self.spread_shape(src, old);
        }
        for src in segments {
            changed |= self.spread_allowance(src);
        }
        changed | self.spread_piece_allowance(before)
    }

    /// Whether points `i` and `j` of `piece` were (in `before`) on the same
    /// side of the mirror line between the piece's halves.
    fn same_side(&self, before: &Pattern, piece: usize, i: usize, j: usize) -> bool {
        let pts = &before.pieces[piece].points;
        let halves = self.symmetries.iter().filter(|s| s.a == piece && s.is_halves());
        halves.filter_map(Symmetry::axis).all(|(o, d)| {
            let side = |p: Vec2| {
                let c = d.x * (p.y - o.y) - d.y * (p.x - o.x);
                if c.abs() < 1e-6 { 0.0 } else { c.signum() }
            };
            side(pts[i].pos) * side(pts[j].pos) >= 0.0
        })
    }

    /// Copy the shape of point `src` across its links, and on from there.
    /// `old` is its value before the edit.
    fn spread_shape(&mut self, src: (usize, usize), old: Point) -> bool {
        let mut changed = false;
        let mut queue = VecDeque::from([(src, old)]);
        // Mirroring is exact, so values settle after one pass; the budget
        // only guards against links that contradict each other.
        let mut budget = 4 * self.pieces.iter().map(|p| p.points.len()).sum::<usize>();
        while let Some(((pi, i), old)) = queue.pop_front() {
            changed |= self.settle_on_axis(pi, i, &old);
            let p = self.pieces[pi].points[i];
            for s in &self.symmetries {
                let Some((to, j, map)) = s.partner(pi, i) else { continue };
                if (to, j) == (pi, i) {
                    continue;
                }
                let target = &mut self.pieces[to].points[j];
                let next = mirrored(&p, &map, s.reversed, *target);
                if !same_shape(&next, target) {
                    let prev = std::mem::replace(target, next);
                    changed = true;
                    if budget > 0 {
                        budget -= 1;
                        queue.push_back(((to, j), prev));
                    }
                }
            }
        }
        changed
    }

    /// Keep point `i` of `piece` on the mirror line of a halves link that
    /// pairs it with itself: it slides along the line, and one handle is
    /// mirrored from the other, the one that was edited (`old` is the
    /// point's previous value). A smooth point crosses the line square.
    fn settle_on_axis(&mut self, piece: usize, i: usize, old: &Point) -> bool {
        let mut changed = false;
        for s in &self.symmetries {
            if !(s.is_halves() && s.a == piece && s.pairs[i] == i) {
                continue;
            }
            let map = s.map;
            let p = &mut self.pieces[piece].points[i];
            let lead_in = !same_handle(p.handle_in, old.handle_in) && same_handle(p.handle_out, old.handle_out);
            let lead = if lead_in { p.handle_in } else { p.handle_out };
            let lead = match p.kind {
                PointKind::Curve => lead.map(|h| (h - map.linear(h)) * 0.5),
                PointKind::Turn => lead,
            };
            let other = lead.map(|h| map.linear(h));
            let (handle_in, handle_out) = if lead_in { (lead, other) } else { (other, lead) };
            let next = Point { pos: p.pos.lerp(map.apply(p.pos), 0.5), handle_in, handle_out, ..*p };
            if !same_shape(&next, p) {
                *p = next;
                changed = true;
            }
        }
        changed
    }

    /// Copy the seam allowance of segment `src` across its links.
    fn spread_allowance(&mut self, src: (usize, usize)) -> bool {
        let mut changed = false;
        let mut queue = VecDeque::from([src]);
        while let Some((pi, seg)) = queue.pop_front() {
            let width = self.pieces[pi].points[seg].allowance;
            for s in &self.symmetries {
                let Some((to, t)) = s.partner_segment(&self.pieces, pi, seg) else { continue };
                let target = &mut self.pieces[to].points[t].allowance;
                if *target != width {
                    *target = width;
                    changed = true;
                    queue.push_back((to, t));
                }
            }
        }
        changed
    }

    /// Give linked pieces the same default seam allowance, taking the one
    /// that changed since `before`.
    fn spread_piece_allowance(&mut self, before: &Pattern) -> bool {
        let mut changed = false;
        for _ in 0..self.symmetries.len() {
            let mut pass = false;
            for s in self.symmetries.iter().filter(|s| !s.is_halves()) {
                let (wa, wb) = (self.pieces[s.a].seam_allowance, self.pieces[s.b].seam_allowance);
                if wa == wb {
                    continue;
                }
                if wa != before.pieces[s.a].seam_allowance {
                    self.pieces[s.b].seam_allowance = wa;
                } else {
                    self.pieces[s.a].seam_allowance = wb;
                }
                pass = true;
            }
            changed |= pass;
            if !pass {
                break;
            }
        }
        changed
    }

    // --------------------------------------------- adding and removing points

    /// Insert a point on segment `seg` of `piece` at parameter `t`, and its
    /// mirror images on linked pieces and halves; seams stay in place.
    /// Returns the new point's index.
    pub fn insert_point(&mut self, piece: usize, seg: usize, t: f64) -> usize {
        self.validate_symmetries();
        // Outline positions `(piece, seg, t)` to insert at, found by
        // following the links, and the pairs they form: `(link, on a, on b)`.
        let mut plan = vec![(piece, seg, t)];
        let mut pairs = Vec::new();
        let mut k = 0;
        while k < plan.len() {
            let (pi, s, t) = plan[k];
            for (li, link) in self.symmetries.iter().enumerate() {
                let Some((to, s2)) = link.partner_segment(&self.pieces, pi, s) else { continue };
                let t2 = if link.reversed { 1.0 - t } else { t };
                let node = plan.iter().position(|&(p, q, u)| p == to && q == s2 && (u - t2).abs() < EPS);
                let node = node.unwrap_or_else(|| {
                    plan.push((to, s2, t2));
                    plan.len() - 1
                });
                pairs.push(if pi == link.a { (li, k, node) } else { (li, node, k) });
            }
            k += 1;
        }

        // In outline order, the j-th insertion on a piece lands at index seg + j + 1.
        let mut order: Vec<usize> = (0..plan.len()).collect();
        order.sort_by(|&x, &y| {
            let ((px, sx, tx), (py, sy, ty)) = (plan[x], plan[y]);
            (px, sx).cmp(&(py, sy)).then(tx.total_cmp(&ty))
        });
        let mut new_index = vec![0; plan.len()];
        let mut nth = 0;
        for (n, &x) in order.iter().enumerate() {
            nth = if n > 0 && plan[order[n - 1]].0 == plan[x].0 { nth + 1 } else { 0 };
            new_index[x] = plan[x].1 + nth + 1;
        }
        let shifted = |piece: usize, i: usize| i + plan.iter().filter(|&&(p, s, _)| p == piece && s < i).count();
        for (li, link) in self.symmetries.iter_mut().enumerate() {
            if !plan.iter().any(|&(p, ..)| link.touches(p)) {
                continue;
            }
            let added = plan.iter().filter(|&&(p, ..)| p == link.a).count();
            let mut next = vec![usize::MAX; link.pairs.len() + added];
            for (i, &j) in link.pairs.iter().enumerate() {
                next[shifted(link.a, i)] = shifted(link.b, j);
            }
            for &(_, x, y) in pairs.iter().filter(|p| p.0 == li) {
                next[new_index[x]] = new_index[y];
                if link.is_halves() {
                    next[new_index[y]] = new_index[x];
                }
            }
            link.pairs = next;
        }

        // Insert from the end of each outline so earlier positions keep their
        // indices; a later insertion on the same segment lands in its first part.
        let mut last: Option<(usize, usize, f64)> = None;
        for &x in order.iter().rev() {
            let (p, s, t) = plan[x];
            let t_part = match last {
                Some((lp, ls, lt)) if (lp, ls) == (p, s) && lt > 0.0 => t / lt,
                _ => t,
            };
            self.insert_point_raw(p, s, t_part);
            last = Some((p, s, t));
        }
        self.validate_symmetries();
        // Make the mirror images exact (curves through automatic smoothing
        // shift slightly when two points are added to one segment).
        let idx = new_index[0];
        let p = self.pieces[piece].points[idx];
        self.spread_shape((piece, idx), p);
        idx
    }

    /// Remove point `i` of `piece` and its mirror images (see
    /// [`Self::remove_points`]). Returns `true` if `piece` itself was removed.
    pub fn remove_point(&mut self, piece: usize, i: usize) -> bool {
        self.remove_points(&[(piece, i)]).contains(&piece)
    }

    /// Remove points `(piece, point)` and their mirror images on linked pieces
    /// and halves. Outlines left with fewer than 3 points open, and pieces
    /// left with none are removed. Returns the (original) indices of the
    /// removed pieces.
    pub fn remove_points(&mut self, points: &[(usize, usize)]) -> Vec<usize> {
        self.validate_symmetries();
        let mut doomed: Vec<(usize, usize)> = Vec::new();
        for &(pi, i) in points {
            if !self.pieces.get(pi).is_some_and(|p| i < p.points.len()) {
                continue;
            }
            for r in std::iter::once((pi, i)).chain(self.mirror_images(pi, i)) {
                if !doomed.contains(&r) {
                    doomed.push(r);
                }
            }
        }
        let shifted = |piece: usize, i: usize| i - doomed.iter().filter(|&&(p, j)| p == piece && j < i).count();
        for link in &mut self.symmetries {
            if !doomed.iter().any(|&(p, _)| link.touches(p)) {
                continue;
            }
            link.pairs = (link.pairs.iter().enumerate())
                .filter(|&(i, _)| !doomed.contains(&(link.a, i)))
                .map(|(_, &j)| if doomed.contains(&(link.b, j)) { usize::MAX } else { shifted(link.b, j) })
                .collect();
        }
        // Highest piece and point first, so earlier removals don't shift later ones.
        doomed.sort_by(|x, y| y.cmp(x));
        let mut removed = Vec::new();
        for (pi, i) in doomed {
            if self.remove_point_raw(pi, i) {
                removed.push(pi);
            }
        }
        self.validate_symmetries();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-6
    }

    /// Left half of a 200 x 100 rectangle, folded along x = 100 (segment 1).
    fn half() -> Pattern {
        let mut p = Piece::new("front");
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)] {
            p.points.push(Point::turn(Vec2::new(x, y)));
        }
        p.closed = true;
        Pattern { pieces: vec![p], ..Pattern::default() }
    }

    /// Assert piece `pi` is its own mirror image across x = `axis`.
    fn assert_symmetric(pat: &Pattern, pi: usize, axis: f64) {
        let piece = &pat.pieces[pi];
        let mirror = |v: Vec2| Vec2::new(2.0 * axis - v.x, v.y);
        for k in 0..=40 {
            let q = piece.eval_segment(0, k as f64 / 40.0);
            let (_, _, _, d) = piece.closest_on_outline(mirror(q)).unwrap();
            assert!(d < 0.05, "{q:?} has no mirror image (off by {d})");
        }
        for seg in 0..piece.segment_count() {
            let (_, _, _, d) = piece.closest_on_outline(mirror(piece.eval_segment(seg, 0.37))).unwrap();
            assert!(d < 0.05, "segment {seg} has no mirror image (off by {d})");
        }
    }

    #[test]
    fn isometry_basics() {
        let m = Isometry::mirror(Vec2::new(10.0, 0.0), Vec2::new(10.0, 5.0));
        assert!(close(m.apply(Vec2::new(0.0, 3.0)), Vec2::new(20.0, 3.0)));
        assert!(close(m.linear(Vec2::new(1.0, 1.0)), Vec2::new(-1.0, 1.0)));
        let (o, d) = m.mirror_line().unwrap();
        assert!((o.x - 10.0).abs() < 1e-9 && d.x.abs() < 1e-9);

        let r = Isometry::rotation(Vec2::new(5.0, 5.0), 90.0);
        assert!(r.mirror_line().is_none());
        assert!(close(r.apply(Vec2::new(10.0, 5.0)), Vec2::new(5.0, 10.0)));
        let p = Vec2::new(3.0, -7.0);
        assert!(close(r.inverse().apply(r.apply(p)), p));
        assert!(close(m.then(&r).apply(p), r.apply(m.apply(p))));
    }

    #[test]
    fn unfold_symmetric_mirrors_point_moves() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        assert_eq!(pat.symmetries.len(), 1);
        assert_symmetric(&pat, 0, 100.0);

        // Move a corner: its partner on the other half follows.
        let before = pat.clone();
        let i = pat.pieces[0].points.iter().position(|p| p.pos == Vec2::new(0.0, 100.0)).unwrap();
        pat.pieces[0].points[i].pos = Vec2::new(-20.0, 130.0);
        assert!(pat.mirror_changes(&before, Some((0, i))));
        assert!(pat.pieces[0].points.iter().any(|p| close(p.pos, Vec2::new(220.0, 130.0))));
        assert_symmetric(&pat, 0, 100.0);

        // A point on the fold line only slides along it.
        let before = pat.clone();
        pat.pieces[0].points[0].pos = Vec2::new(130.0, 105.0);
        pat.mirror_changes(&before, Some((0, 0)));
        assert!(close(pat.pieces[0].points[0].pos, Vec2::new(100.0, 105.0)));
        // Already symmetric: nothing more to do.
        assert!(!pat.mirror_changes(&before, None));
    }

    #[test]
    fn handles_and_bends_are_mirrored() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        // Bend the top edge next to the fold, then a handle on a smooth axis point.
        let top = (0..4).find(|&s| {
            let (a, b) = pat.pieces[0].segment_ends(s);
            pat.pieces[0].points[a].pos == Vec2::new(100.0, 100.0) && pat.pieces[0].points[b].pos.x < 100.0
        });
        let seg = top.unwrap();
        let before = pat.clone();
        let start = pat.pieces[0].segment_handles(seg);
        pat.pieces[0].bend_segment(seg, 0.5, start, Vec2::new(0.0, 25.0));
        pat.mirror_changes(&before, None);
        assert_symmetric(&pat, 0, 100.0);

        let (axis_pt, _) = pat.pieces[0].segment_ends(seg);
        let before = pat.clone();
        pat.pieces[0].set_point_kind(axis_pt, PointKind::Curve);
        pat.pieces[0].set_handle(axis_pt, crate::curves::HandleSide::Out, Vec2::new(-30.0, 10.0), true);
        pat.mirror_changes(&before, Some((0, axis_pt)));
        let p = pat.pieces[0].points[axis_pt];
        // Smooth on the axis: the handles cross it square.
        assert!(close(p.handle_out.unwrap(), Vec2::new(-30.0, 0.0)));
        assert!(close(p.handle_in.unwrap(), Vec2::new(30.0, 0.0)));
        assert_symmetric(&pat, 0, 100.0);
    }

    #[test]
    fn focus_side_wins_when_both_halves_changed() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        let before = pat.clone();
        // Move every point right by 10, as a drag of all selected points would.
        for p in &mut pat.pieces[0].points {
            p.pos.x += 10.0;
        }
        let left = pat.pieces[0].points.iter().position(|p| p.pos == Vec2::new(10.0, 0.0)).unwrap();
        pat.mirror_changes(&before, Some((0, left)));
        // The left half moved toward the fixed fold line; the right half mirrors it.
        let mut xs: Vec<f64> = pat.pieces[0].points.iter().map(|p| p.pos.x).collect();
        xs.sort_by(f64::total_cmp);
        assert!((xs[0] - 10.0).abs() < 1e-9 && (xs[xs.len() - 1] - 190.0).abs() < 1e-9);
    }

    #[test]
    fn insert_and_remove_points_on_both_halves() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        let n = pat.pieces[0].points.len();
        // Insert on the bottom left edge (segment from (0,0) toward the fold).
        let seg = (0..n).find(|&s| pat.pieces[0].points[s].pos == Vec2::new(0.0, 0.0)).unwrap();
        let idx = pat.insert_point(0, seg, 0.25);
        assert_eq!(pat.pieces[0].points.len(), n + 2);
        assert_eq!(pat.symmetries.len(), 1);
        let images = pat.mirror_images(0, idx);
        assert_eq!(images.len(), 1);
        let mirror = pat.pieces[0].points[images[0].1].pos;
        assert!(close(mirror, Vec2::new(200.0 - pat.pieces[0].points[idx].pos.x, 0.0)));

        // Moving the new point still mirrors.
        let before = pat.clone();
        pat.pieces[0].points[idx].pos.y = -15.0;
        pat.mirror_changes(&before, Some((0, idx)));
        assert!((pat.pieces[0].points[images[0].1].pos.y + 15.0).abs() < 1e-9);

        assert!(!pat.remove_point(0, idx));
        assert_eq!(pat.pieces[0].points.len(), n);
        assert_eq!(pat.symmetries.len(), 1);
        assert_symmetric(&pat, 0, 100.0);
    }

    #[test]
    fn insert_on_segment_crossing_the_fold() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        // Remove the bottom fold point: the bottom edge now crosses the fold.
        let fold = (0..6).find(|&i| pat.pieces[0].points[i].pos == Vec2::new(100.0, 0.0)).unwrap();
        pat.remove_point(0, fold);
        assert_eq!(pat.pieces[0].points.len(), 5);
        let seg = (0..5).find(|&s| pat.pieces[0].eval_segment(s, 0.5) == Vec2::new(100.0, 0.0)).unwrap();
        // Off-center: two points. Centered: one, on the fold.
        pat.insert_point(0, seg, 0.25);
        assert_eq!(pat.pieces[0].points.len(), 7);
        let seg = (0..7).find(|&s| close(pat.pieces[0].eval_segment(s, 0.5), Vec2::new(100.0, 0.0))).unwrap();
        let mid = pat.insert_point(0, seg, 0.5);
        assert_eq!(pat.pieces[0].points.len(), 8);
        assert!(pat.mirror_images(0, mid).is_empty()); // its own mirror image
        assert_symmetric(&pat, 0, 100.0);
    }

    #[test]
    fn mirrored_copy_follows_edits_both_ways() {
        let mut pat = half();
        let copy = pat.duplicate_mirrored(0);
        assert_eq!(pat.pieces[copy].name, "front mirrored");
        let axis = |pat: &Pattern| pat.symmetries[0].axis().unwrap();
        // Copied 120 to the right and flipped: mirror line at x = 110.
        assert!((axis(&pat).0.x - 110.0).abs() < 1e-9);

        let before = pat.clone();
        pat.pieces[0].points[2].pos = Vec2::new(90.0, 120.0);
        pat.mirror_changes(&before, Some((0, 2)));
        assert!(close(pat.pieces[copy].points[2].pos, Vec2::new(130.0, 120.0)));

        let before = pat.clone();
        pat.pieces[copy].points[0].pos.y = -10.0;
        pat.mirror_changes(&before, Some((copy, 0)));
        assert!(close(pat.pieces[0].points[0].pos, Vec2::new(0.0, -10.0)));

        // Moving the copy away keeps the original in place, and edits still mirror.
        pat.transform_piece(copy, &Isometry::translation(Vec2::new(50.0, 0.0)));
        assert!(close(pat.pieces[0].points[0].pos, Vec2::new(0.0, -10.0)));
        assert!((axis(&pat).0.x - 135.0).abs() < 1e-9);
        let before = pat.clone();
        pat.pieces[0].points[1].pos.x = 80.0;
        pat.mirror_changes(&before, Some((0, 1)));
        assert!(close(pat.pieces[copy].points[1].pos, Vec2::new(190.0, 0.0)));

        // Seam allowance follows too.
        let before = pat.clone();
        pat.pieces[copy].seam_allowance = 10.0;
        pat.pieces[copy].points[3].allowance = Some(0.0);
        pat.mirror_changes(&before, None);
        assert_eq!(pat.pieces[0].seam_allowance, 10.0);
        assert_eq!(pat.pieces[0].points[3].allowance, Some(0.0));
    }

    #[test]
    fn rotating_a_symmetric_piece_carries_its_fold() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        let rot = pat.pieces[0].rotation(90.0);
        pat.transform_piece(0, &rot);
        let before = pat.clone();
        pat.pieces[0].points[1].pos = pat.pieces[0].points[1].pos + Vec2::new(5.0, 5.0);
        pat.mirror_changes(&before, Some((0, 1)));
        // Undo the rotation: the piece is still symmetric about x = 100.
        pat.transform_piece(0, &rot.inverse());
        assert_symmetric(&pat, 0, 100.0);
    }

    #[test]
    fn mirror_allowance_on_halves() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        let before = pat.clone();
        pat.pieces[0].points[1].allowance = Some(25.0);
        pat.mirror_changes(&before, None);
        let set: Vec<usize> = (0..6).filter(|&i| pat.pieces[0].points[i].allowance == Some(25.0)).collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn one_sided_changes_drop_the_link() {
        let mut pat = half();
        let copy = pat.duplicate_mirrored(0);
        pat.pieces[copy].points.pop();
        pat.validate_symmetries();
        assert!(pat.symmetries.is_empty());

        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        pat.remove_piece(0);
        assert!(pat.symmetries.is_empty());
    }

    #[test]
    fn links_survive_a_save() {
        let mut pat = half();
        pat.unfold_symmetric(0, &[1]).unwrap();
        let json = serde_json::to_string(&pat).unwrap();
        assert_eq!(serde_json::from_str::<Pattern>(&json).unwrap(), pat);
        assert!(!serde_json::to_string(&half()).unwrap().contains("symmetries"));
    }
}

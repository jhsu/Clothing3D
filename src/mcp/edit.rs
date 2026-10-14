//! Pattern edits requested over MCP. Each edit is plain data so a `batch`
//! can apply several as one undo step; lengths are in the user's display
//! units unless a value names its own unit.

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use crate::curves::HandleSide;
use crate::model::{Pattern, Piece, Point, PointKind, Vec2};
use crate::sewing::{EdgeSpan, OutlinePos, Seam};
use crate::units::Units;

/// A length or coordinate: a number in the user's current units, or a string
/// that may name its unit (`"90mm"`, `"9 cm"`, `"3 1/2 in"`, `"5/8\""`).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Length {
    Number(f64),
    Text(String),
}

impl Length {
    pub fn mm(&self, units: Units) -> Result<f64, String> {
        match self {
            Length::Number(v) if v.is_finite() => Ok(units.to_mm(*v)),
            Length::Number(v) => Err(format!("{v} is not a finite number")),
            Length::Text(s) => parse_length(s, units).ok_or_else(|| {
                format!("can't read length {s:?}: use a number, a fraction like \"3 1/2\", or add mm/cm/in")
            }),
        }
    }
}

fn parse_length(s: &str, units: Units) -> Option<f64> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('-') {
        return parse_length(rest, units).map(|v| -v);
    }
    let lower = s.to_ascii_lowercase();
    const SUFFIXES: [(&str, Units, f64); 7] = [
        ("inches", Units::Inches, 1.0),
        ("inch", Units::Inches, 1.0),
        ("in", Units::Inches, 1.0),
        ("\"", Units::Inches, 1.0),
        ("mm", Units::Millimeters, 1.0),
        ("cm", Units::Millimeters, 10.0),
        ("m", Units::Millimeters, 1000.0),
    ];
    for (suffix, unit, scale) in SUFFIXES {
        if let Some(num) = lower.strip_suffix(suffix) {
            return unit.parse(num).map(|v| v * scale);
        }
    }
    units.parse(s)
}

/// A position or offset `{x, y}`; Y points up.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Xy {
    pub x: Length,
    pub y: Length,
}

impl Xy {
    fn mm(&self, units: Units) -> Result<Vec2, String> {
        Ok(Vec2::new(self.x.mm(units)?, self.y.mm(units)?))
    }
}

/// A piece by index or by exact name.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PieceRef {
    Index(usize),
    Name(String),
}

impl PieceRef {
    pub fn resolve(&self, pattern: &Pattern) -> Result<usize, String> {
        match self {
            PieceRef::Index(i) if *i < pattern.pieces.len() => Ok(*i),
            PieceRef::Index(i) => Err(format!("piece {i} doesn't exist (the pattern has {} pieces)", pattern.pieces.len())),
            PieceRef::Name(name) => {
                let hits: Vec<usize> =
                    pattern.pieces.iter().enumerate().filter(|(_, p)| &p.name == name).map(|(i, _)| i).collect();
                match hits[..] {
                    [i] => Ok(i),
                    [] => Err(format!("no piece named {name:?}")),
                    _ => Err(format!("several pieces are named {name:?} (indices {hits:?}); use an index")),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Sharp corner.
    Turn,
    /// Smooth point the outline curves through.
    Curve,
}

impl From<Kind> for PointKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Turn => PointKind::Turn,
            Kind::Curve => PointKind::Curve,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct PointSpec {
    pub x: Length,
    pub y: Length,
    /// Defaults to `turn`.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Incoming Bézier handle as an offset from the point.
    #[serde(default)]
    pub handle_in: Option<Xy>,
    /// Outgoing Bézier handle as an offset from the point.
    #[serde(default)]
    pub handle_out: Option<Xy>,
}

impl PointSpec {
    fn point(&self, units: Units) -> Result<Point, String> {
        Ok(Point {
            pos: Vec2::new(self.x.mm(units)?, self.y.mm(units)?),
            kind: self.kind.map_or(PointKind::Turn, Into::into),
            handle_in: self.handle_in.as_ref().map(|h| h.mm(units)).transpose()?,
            handle_out: self.handle_out.as_ref().map(|h| h.mm(units)).transpose()?,
        })
    }
}

fn points(specs: &[PointSpec], units: Units) -> Result<Vec<Point>, String> {
    specs.iter().map(|p| p.point(units)).collect()
}

fn default_true() -> bool {
    true
}

fn default_half() -> f64 {
    0.5
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Flip {
    /// Mirror left ↔ right.
    Horizontal,
    /// Mirror top ↔ bottom.
    Vertical,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Start,
    End,
}

/// A position on an outline: at point `point`, or at parameter `t` (0..1)
/// along segment `segment`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct OutlineSpec {
    #[serde(default)]
    pub point: Option<usize>,
    #[serde(default)]
    pub segment: Option<usize>,
    #[serde(default)]
    pub t: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Way {
    /// The shorter way around the outline.
    #[default]
    Shortest,
    /// In point order.
    Forward,
    /// Against point order.
    Backward,
}

/// One edge of a seam: a whole segment (`segment`), or a stretch of outline
/// from `start` to `end`. Seams sew the two edges' starts together.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SpanSpec {
    pub piece: PieceRef,
    /// Shorthand for the whole segment, walked in point order.
    #[serde(default)]
    pub segment: Option<usize>,
    #[serde(default)]
    pub start: Option<OutlineSpec>,
    #[serde(default)]
    pub end: Option<OutlineSpec>,
    /// Which way to walk from `start` to `end` (default: shortest).
    #[serde(default)]
    pub way: Way,
    /// Swap the edge's direction after building it.
    #[serde(default)]
    pub reverse: bool,
}

// ------------------------------------------------------------------ edits

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct AddPiece {
    pub name: String,
    /// Outline points in order.
    pub points: Vec<PointSpec>,
    /// Closed outline (default true).
    #[serde(default = "default_true")]
    pub closed: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetPiecePoints {
    pub piece: PieceRef,
    pub points: Vec<PointSpec>,
    /// Keep the current setting when omitted.
    #[serde(default)]
    pub closed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct PieceOnly {
    pub piece: PieceRef,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RenamePiece {
    pub piece: PieceRef,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct TransformPiece {
    pub piece: PieceRef,
    /// Applied first, about the piece's bounding-box center.
    #[serde(default)]
    pub flip: Option<Flip>,
    /// Degrees counter-clockwise about the piece's center, applied second.
    #[serde(default)]
    pub rotate: Option<f64>,
    /// Offset applied last.
    #[serde(default)]
    pub translate: Option<Xy>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct UnfoldPiece {
    pub piece: PieceRef,
    /// Straight, contiguous, collinear segments forming the fold line.
    pub segments: Vec<usize>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct MovePoint {
    pub piece: PieceRef,
    pub point: usize,
    /// New absolute position.
    #[serde(default)]
    pub to: Option<Xy>,
    /// Relative move (applied after `to`).
    #[serde(default)]
    pub by: Option<Xy>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct InsertPoint {
    pub piece: PieceRef,
    pub segment: usize,
    /// Position along the segment, 0..1 (default 0.5).
    #[serde(default = "default_half")]
    pub t: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct PointOnly {
    pub piece: PieceRef,
    pub point: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetPointKind {
    pub piece: PieceRef,
    pub point: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetHandles {
    pub piece: PieceRef,
    pub point: usize,
    /// Incoming handle offset from the point.
    #[serde(default)]
    pub handle_in: Option<Xy>,
    /// Outgoing handle offset from the point.
    #[serde(default)]
    pub handle_out: Option<Xy>,
    /// On a curve point, setting one handle mirrors the other to keep the
    /// point smooth. Without it the point becomes a corner.
    #[serde(default = "default_true")]
    pub mirror: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetSegmentLength {
    pub piece: PieceRef,
    pub segment: usize,
    /// Target length (arc length for curved segments).
    pub length: Length,
    /// Which end point moves (default: the user's setting).
    #[serde(default)]
    pub moving: Option<End>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetSegmentHandles {
    pub piece: PieceRef,
    pub segment: usize,
    /// Handle leaving the segment's start point, as an offset from it.
    pub start_handle: Xy,
    /// Handle arriving at the segment's end point, as an offset from it.
    pub end_handle: Xy,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct BendSegment {
    pub piece: PieceRef,
    pub segment: usize,
    /// Which spot on the segment to pull, 0..1 (default 0.5).
    #[serde(default = "default_half")]
    pub t: f64,
    /// How far to move that spot.
    pub by: Xy,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SegmentOnly {
    pub piece: PieceRef,
    pub segment: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct AddSeam {
    pub a: SpanSpec,
    pub b: SpanSpec,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SeamOnly {
    pub seam: usize,
}

/// One pattern edit, tagged by `op`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    AddPiece(AddPiece),
    SetPiecePoints(SetPiecePoints),
    RemovePiece(PieceOnly),
    DuplicatePiece(PieceOnly),
    RenamePiece(RenamePiece),
    TransformPiece(TransformPiece),
    UnfoldPiece(UnfoldPiece),
    MovePoint(MovePoint),
    InsertPoint(InsertPoint),
    RemovePoint(PointOnly),
    SetPointKind(SetPointKind),
    SetHandles(SetHandles),
    ClearHandles(PointOnly),
    SetSegmentLength(SetSegmentLength),
    SetSegmentHandles(SetSegmentHandles),
    BendSegment(BendSegment),
    StraightenSegment(SegmentOnly),
    AddSeam(AddSeam),
    RemoveSeam(SeamOnly),
    FlipSeam(SeamOnly),
}

/// What an edit did, for the reply.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Pieces whose geometry changed and still exist.
    pub touched: Vec<usize>,
    /// New piece / point / seam index, if the edit made one.
    pub created: Option<(&'static str, usize)>,
    pub notes: Vec<String>,
}

impl Outcome {
    fn piece(i: usize) -> Self {
        Self { touched: vec![i], ..Self::default() }
    }

    fn merge(&mut self, other: Outcome) {
        for i in other.touched {
            if !self.touched.contains(&i) {
                self.touched.push(i);
            }
        }
        if other.created.is_some() {
            self.created = other.created;
        }
        self.notes.extend(other.notes);
    }
}

pub struct EditEnv {
    pub units: Units,
    pub length_moves_start: bool,
}

fn check_point(piece: &Piece, i: usize) -> Result<(), String> {
    if i < piece.points.len() {
        Ok(())
    } else {
        Err(format!("point {i} doesn't exist ({:?} has {} points)", piece.name, piece.points.len()))
    }
}

fn check_segment(piece: &Piece, seg: usize) -> Result<(), String> {
    if seg < piece.segment_count() {
        Ok(())
    } else {
        Err(format!("segment {seg} doesn't exist ({:?} has {} segments)", piece.name, piece.segment_count()))
    }
}

fn check_seam(pattern: &Pattern, i: usize) -> Result<(), String> {
    if i < pattern.seams.len() {
        Ok(())
    } else {
        Err(format!("seam {i} doesn't exist (the pattern has {} seams)", pattern.seams.len()))
    }
}

fn check_t(t: f64) -> Result<f64, String> {
    if (0.0..=1.0).contains(&t) { Ok(t) } else { Err(format!("t must be between 0 and 1, got {t}")) }
}

/// Drop seams on `piece` (its outline was replaced) and report how many.
fn drop_seams_on(pattern: &mut Pattern, piece: usize, out: &mut Outcome) {
    let before = pattern.seams.len();
    pattern.seams.retain(|s| s.spans().iter().all(|sp| sp.piece != piece));
    let dropped = before - pattern.seams.len();
    if dropped > 0 {
        out.notes.push(format!("removed {dropped} seam(s) attached to the replaced outline"));
    }
}

/// Remove pieces, shifting later piece indices; reports what moved.
fn note_reindex(out: &mut Outcome, removed: usize, count_before: usize) {
    if removed + 1 < count_before {
        out.notes.push(format!("pieces after {removed} moved down one index"));
    }
}

fn outline_pos(piece: &Piece, spec: &OutlineSpec) -> Result<OutlinePos, String> {
    match (spec.point, spec.segment) {
        (Some(i), None) => {
            check_point(piece, i)?;
            let n = piece.segment_count();
            if n == 0 {
                return Err(format!("{:?} has no segments", piece.name));
            }
            // The last point of an open outline is the end of the last segment.
            Ok(if i < n { OutlinePos::new(i, 0.0) } else { OutlinePos::new(n - 1, 1.0) })
        }
        (None, Some(seg)) => {
            check_segment(piece, seg)?;
            Ok(OutlinePos::new(seg, check_t(spec.t.unwrap_or(0.0))?))
        }
        _ => Err("an outline position needs either `point` or `segment` (+ optional `t`)".into()),
    }
}

fn span(pattern: &Pattern, spec: &SpanSpec) -> Result<EdgeSpan, String> {
    let pi = spec.piece.resolve(pattern)?;
    let piece = &pattern.pieces[pi];
    let span = match (spec.segment, &spec.start, &spec.end) {
        (Some(seg), None, None) => {
            check_segment(piece, seg)?;
            EdgeSpan { piece: pi, start: OutlinePos::new(seg, 0.0), end: OutlinePos::new(seg, 1.0), forward: true }
        }
        (None, Some(s), Some(e)) => {
            let (start, end) = (outline_pos(piece, s)?, outline_pos(piece, e)?);
            match spec.way {
                Way::Shortest => EdgeSpan::shortest(&pattern.pieces, pi, start, end),
                Way::Forward | Way::Backward if !piece.closed => {
                    return Err("`way` only applies to closed outlines".into());
                }
                Way::Forward => EdgeSpan { piece: pi, start, end, forward: true },
                Way::Backward => EdgeSpan { piece: pi, start, end, forward: false },
            }
        }
        _ => return Err("a seam edge needs either `segment` or both `start` and `end`".into()),
    };
    Ok(if spec.reverse { span.reversed() } else { span })
}

impl Edit {
    pub fn apply(&self, pattern: &mut Pattern, env: &EditEnv) -> Result<Outcome, String> {
        let u = env.units;
        match self {
            Edit::AddPiece(e) => {
                let mut piece = Piece::new(e.name.clone());
                piece.points = points(&e.points, u)?;
                piece.closed = e.closed && piece.points.len() >= 3;
                if e.closed && !piece.closed {
                    return Err("a closed piece needs at least 3 points".into());
                }
                pattern.pieces.push(piece);
                let i = pattern.pieces.len() - 1;
                Ok(Outcome { created: Some(("piece", i)), ..Outcome::piece(i) })
            }
            Edit::SetPiecePoints(e) => {
                let i = e.piece.resolve(pattern)?;
                let pts = points(&e.points, u)?;
                let closed = e.closed.unwrap_or(pattern.pieces[i].closed);
                if closed && pts.len() < 3 {
                    return Err("a closed piece needs at least 3 points".into());
                }
                let mut out = Outcome::piece(i);
                drop_seams_on(pattern, i, &mut out);
                let piece = &mut pattern.pieces[i];
                piece.points = pts;
                piece.closed = closed;
                Ok(out)
            }
            Edit::RemovePiece(e) => {
                let i = e.piece.resolve(pattern)?;
                let n = pattern.pieces.len();
                pattern.remove_piece(i);
                let mut out = Outcome::default();
                note_reindex(&mut out, i, n);
                Ok(out)
            }
            Edit::DuplicatePiece(e) => {
                let i = e.piece.resolve(pattern)?;
                let copy = pattern.duplicate_piece(i);
                Ok(Outcome { created: Some(("piece", copy)), ..Outcome::piece(copy) })
            }
            Edit::RenamePiece(e) => {
                let i = e.piece.resolve(pattern)?;
                pattern.pieces[i].name = e.name.clone();
                Ok(Outcome::piece(i))
            }
            Edit::TransformPiece(e) => {
                let i = e.piece.resolve(pattern)?;
                let delta = e.translate.as_ref().map(|d| d.mm(u)).transpose()?;
                if e.rotate.is_some_and(|r| !r.is_finite()) {
                    return Err("rotate must be a finite angle in degrees".into());
                }
                let piece = &mut pattern.pieces[i];
                if let Some(flip) = e.flip {
                    piece.flip(matches!(flip, Flip::Horizontal));
                }
                if let Some(deg) = e.rotate {
                    piece.rotate(deg);
                }
                if let Some(d) = delta {
                    piece.translate(d);
                }
                Ok(Outcome::piece(i))
            }
            Edit::UnfoldPiece(e) => {
                let i = e.piece.resolve(pattern)?;
                pattern.unfold(i, &e.segments)?;
                Ok(Outcome::piece(i))
            }
            Edit::MovePoint(e) => {
                let i = e.piece.resolve(pattern)?;
                check_point(&pattern.pieces[i], e.point)?;
                if e.to.is_none() && e.by.is_none() {
                    return Err("give `to` (absolute) and/or `by` (relative)".into());
                }
                let to = e.to.as_ref().map(|p| p.mm(u)).transpose()?;
                let by = e.by.as_ref().map(|p| p.mm(u)).transpose()?;
                let pos = &mut pattern.pieces[i].points[e.point].pos;
                if let Some(to) = to {
                    *pos = to;
                }
                if let Some(by) = by {
                    *pos = *pos + by;
                }
                Ok(Outcome::piece(i))
            }
            Edit::InsertPoint(e) => {
                let i = e.piece.resolve(pattern)?;
                check_segment(&pattern.pieces[i], e.segment)?;
                let t = check_t(e.t)?;
                let new = pattern.insert_point(i, e.segment, t);
                let mut out = Outcome { created: Some(("point", new)), ..Outcome::piece(i) };
                out.notes.push(format!("points from {new} on moved up one index"));
                Ok(out)
            }
            Edit::RemovePoint(e) => {
                let i = e.piece.resolve(pattern)?;
                check_point(&pattern.pieces[i], e.point)?;
                let n = pattern.pieces.len();
                if pattern.remove_point(i, e.point) {
                    let mut out = Outcome::default();
                    out.notes.push("that was the last point; the piece was removed".into());
                    note_reindex(&mut out, i, n);
                    Ok(out)
                } else {
                    Ok(Outcome::piece(i))
                }
            }
            Edit::SetPointKind(e) => {
                let i = e.piece.resolve(pattern)?;
                check_point(&pattern.pieces[i], e.point)?;
                pattern.pieces[i].set_point_kind(e.point, e.kind.into());
                Ok(Outcome::piece(i))
            }
            Edit::SetHandles(e) => {
                let i = e.piece.resolve(pattern)?;
                let piece = &mut pattern.pieces[i];
                check_point(piece, e.point)?;
                if e.handle_in.is_none() && e.handle_out.is_none() {
                    return Err("give `handle_in` and/or `handle_out`".into());
                }
                let (has_in, has_out) = (piece.prev(e.point).is_some(), piece.next(e.point).is_some());
                for (side, h, exists) in [(HandleSide::In, &e.handle_in, has_in), (HandleSide::Out, &e.handle_out, has_out)] {
                    let Some(h) = h else { continue };
                    if !exists {
                        return Err(format!("point {} has no {side:?} segment to put a handle on", e.point));
                    }
                    // Setting both sides explicitly mustn't mirror one over the other.
                    let mirror = e.mirror && (e.handle_in.is_none() || e.handle_out.is_none());
                    piece.set_handle(e.point, side, h.mm(u)?, mirror);
                }
                Ok(Outcome::piece(i))
            }
            Edit::ClearHandles(e) => {
                let i = e.piece.resolve(pattern)?;
                check_point(&pattern.pieces[i], e.point)?;
                pattern.pieces[i].clear_handles(e.point);
                Ok(Outcome::piece(i))
            }
            Edit::SetSegmentLength(e) => {
                let i = e.piece.resolve(pattern)?;
                check_segment(&pattern.pieces[i], e.segment)?;
                let len = e.length.mm(u)?;
                if len <= 0.0 {
                    return Err("length must be positive".into());
                }
                let move_start = match e.moving {
                    Some(End::Start) => true,
                    Some(End::End) => false,
                    None => env.length_moves_start,
                };
                if !pattern.pieces[i].set_segment_length(e.segment, len, move_start) {
                    return Err("the segment has zero length, so it has no direction to stretch along".into());
                }
                Ok(Outcome::piece(i))
            }
            Edit::SetSegmentHandles(e) => {
                let i = e.piece.resolve(pattern)?;
                check_segment(&pattern.pieces[i], e.segment)?;
                let (a, b) = (e.start_handle.mm(u)?, e.end_handle.mm(u)?);
                pattern.pieces[i].set_segment_handles(e.segment, a, b);
                Ok(Outcome::piece(i))
            }
            Edit::BendSegment(e) => {
                let i = e.piece.resolve(pattern)?;
                let piece = &mut pattern.pieces[i];
                check_segment(piece, e.segment)?;
                let t = check_t(e.t)?;
                if t == 0.0 || t == 1.0 {
                    return Err("t must be strictly between 0 and 1 to bend a segment".into());
                }
                let start = piece.segment_handles(e.segment);
                piece.bend_segment(e.segment, t, start, e.by.mm(u)?);
                Ok(Outcome::piece(i))
            }
            Edit::StraightenSegment(e) => {
                let i = e.piece.resolve(pattern)?;
                check_segment(&pattern.pieces[i], e.segment)?;
                pattern.pieces[i].straighten_segment(e.segment);
                Ok(Outcome::piece(i))
            }
            Edit::AddSeam(e) => {
                let (a, b) = (span(pattern, &e.a)?, span(pattern, &e.b)?);
                pattern.seams.push(Seam { a, b });
                let mut out = Outcome { created: Some(("seam", pattern.seams.len() - 1)), ..Outcome::default() };
                out.touched = if a.piece == b.piece { vec![a.piece] } else { vec![a.piece, b.piece] };
                Ok(out)
            }
            Edit::RemoveSeam(e) => {
                check_seam(pattern, e.seam)?;
                pattern.seams.remove(e.seam);
                Ok(Outcome::default())
            }
            Edit::FlipSeam(e) => {
                check_seam(pattern, e.seam)?;
                let seam = &mut pattern.seams[e.seam];
                seam.b = seam.b.reversed();
                Ok(Outcome::default())
            }
        }
    }
}

/// Apply `edits` in order; stops at the first failure (the caller discards
/// the partly edited pattern).
pub fn apply_all(edits: &[Edit], pattern: &mut Pattern, env: &EditEnv) -> Result<Outcome, String> {
    let mut out = Outcome::default();
    for (n, edit) in edits.iter().enumerate() {
        let step = edit.apply(pattern, env).map_err(|e| {
            if edits.len() > 1 { format!("edit {n} failed, nothing was changed: {e}") } else { e }
        })?;
        // Removing pieces invalidates earlier touched indices.
        out.touched.retain(|&i| i < pattern.pieces.len());
        out.merge(step);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(units: Units) -> EditEnv {
        EditEnv { units, length_moves_start: false }
    }

    fn json<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    fn square_pattern() -> Pattern {
        let mut p = Pattern::default();
        let e: Edit = json(serde_json::json!({
            "op": "add_piece", "name": "sq",
            "points": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}]
        }));
        e.apply(&mut p, &env(Units::Millimeters)).unwrap();
        p
    }

    #[test]
    fn lengths_use_current_units_unless_named() {
        let mm = Units::Millimeters;
        let inch = Units::Inches;
        assert_eq!(Length::Number(2.0).mm(inch), Ok(50.8));
        assert_eq!(Length::Text("90mm".into()).mm(inch), Ok(90.0));
        assert_eq!(Length::Text("9 cm".into()).mm(inch), Ok(90.0));
        assert_eq!(Length::Text("3 1/2 in".into()).mm(mm), Ok(3.5 * 25.4));
        assert_eq!(Length::Text("5/8\"".into()).mm(mm), Ok(0.625 * 25.4));
        assert_eq!(Length::Text("-1 1/2".into()).mm(inch), Ok(-1.5 * 25.4));
        assert_eq!(Length::Text("12".into()).mm(mm), Ok(12.0));
        assert!(Length::Text("wide".into()).mm(mm).is_err());
    }

    #[test]
    fn pieces_resolve_by_name_or_index() {
        let p = square_pattern();
        assert_eq!(PieceRef::Name("sq".into()).resolve(&p), Ok(0));
        assert!(PieceRef::Index(1).resolve(&p).is_err());
        assert!(PieceRef::Name("nope".into()).resolve(&p).is_err());
    }

    #[test]
    fn out_of_range_indices_are_errors_not_panics() {
        let mut p = square_pattern();
        let e = env(Units::Millimeters);
        for v in [
            serde_json::json!({"op": "move_point", "piece": 0, "point": 9, "to": {"x": 1, "y": 1}}),
            serde_json::json!({"op": "insert_point", "piece": 0, "segment": 4}),
            serde_json::json!({"op": "set_segment_length", "piece": 0, "segment": 7, "length": 5}),
            serde_json::json!({"op": "remove_seam", "seam": 0}),
            serde_json::json!({"op": "add_seam", "a": {"piece": 0, "segment": 0}, "b": {"piece": 0, "segment": 9}}),
        ] {
            assert!(json::<Edit>(v).apply(&mut p, &e).is_err());
        }
    }

    #[test]
    fn segment_length_in_inches() {
        let mut p = square_pattern();
        let e: Edit = json(serde_json::json!({"op": "set_segment_length", "piece": "sq", "segment": 0, "length": 5}));
        e.apply(&mut p, &env(Units::Inches)).unwrap();
        assert!((p.pieces[0].segment_length(0) - 127.0).abs() < 1e-9);
    }

    #[test]
    fn seam_from_segments_and_points() {
        let mut p = square_pattern();
        let e: Edit = json(serde_json::json!({"op": "add_seam",
            "a": {"piece": 0, "segment": 0},
            "b": {"piece": 0, "start": {"point": 3}, "end": {"point": 2}}}));
        let out = e.apply(&mut p, &env(Units::Millimeters)).unwrap();
        assert_eq!(out.created, Some(("seam", 0)));
        let [a, b] = p.seams[0].spans().map(|s| s.length(&p.pieces));
        assert!((a - 100.0).abs() < 1e-6 && (b - 100.0).abs() < 1e-6);
    }

    #[test]
    fn batch_failure_reports_step() {
        let mut p = square_pattern();
        let edits: Vec<Edit> = json(serde_json::json!([
            {"op": "rename_piece", "piece": 0, "name": "Front"},
            {"op": "remove_point", "piece": 0, "point": 42},
        ]));
        let err = apply_all(&edits, &mut p, &env(Units::Millimeters)).unwrap_err();
        assert!(err.starts_with("edit 1 failed"), "{err}");
    }
}

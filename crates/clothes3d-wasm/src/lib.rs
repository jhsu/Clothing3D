//! WebAssembly bindings for `clothes3d-core`.
//!
//! `PatternDoc` owns a `Pattern` and exposes its editing and geometry
//! operations to JavaScript. Geometry crosses the boundary as flat typed
//! arrays (`[x0, y0, x1, y1, ...]`, millimetres, Y up); structured data
//! (seams, spans, the whole pattern) as plain JS objects shaped like the
//! JSON file format.

use clothes3d_core::curves::HandleSide;
use clothes3d_core::model::{Pattern, Piece, Point, PointKind, Vec2};
use clothes3d_core::sewing::{EdgeSpan, OutlinePos, Seam};
use wasm_bindgen::prelude::*;

fn flat(pts: &[Vec2]) -> Vec<f64> {
    pts.iter().flat_map(|p| [p.x, p.y]).collect()
}

fn to_js<T: serde::Serialize>(v: &T) -> Result<JsValue, JsError> {
    Ok(serde_wasm_bindgen::to_value(v)?)
}

/// Triangulated fill of one piece.
#[wasm_bindgen]
pub struct Mesh {
    vertices: Vec<f64>,
    indices: Vec<u32>,
}

#[wasm_bindgen]
impl Mesh {
    /// `[x0, y0, x1, y1, ...]`
    #[wasm_bindgen(getter)]
    pub fn vertices(&self) -> Vec<f64> {
        self.vertices.clone()
    }

    /// Three vertex indices per triangle.
    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
}

#[wasm_bindgen]
#[derive(Default)]
pub struct PatternDoc {
    pattern: Pattern,
}

impl PatternDoc {
    fn piece(&self, i: usize) -> Result<&Piece, JsError> {
        self.pattern.pieces.get(i).ok_or_else(|| JsError::new("piece index out of range"))
    }

    fn piece_mut(&mut self, i: usize) -> Result<&mut Piece, JsError> {
        self.pattern.pieces.get_mut(i).ok_or_else(|| JsError::new("piece index out of range"))
    }

    fn point(&self, piece: usize, point: usize) -> Result<&Piece, JsError> {
        let p = self.piece(piece)?;
        if point >= p.points.len() {
            return Err(JsError::new("point index out of range"));
        }
        Ok(p)
    }

    fn segment(&self, piece: usize, seg: usize) -> Result<&Piece, JsError> {
        let p = self.piece(piece)?;
        if seg >= p.segment_count() {
            return Err(JsError::new("segment index out of range"));
        }
        Ok(p)
    }
}

#[wasm_bindgen]
impl PatternDoc {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a pattern in the desktop app's JSON file format.
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(json: &str) -> Result<PatternDoc, JsError> {
        Ok(Self { pattern: serde_json::from_str(json)? })
    }

    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> Result<String, JsError> {
        Ok(serde_json::to_string_pretty(&self.pattern)?)
    }

    /// The whole pattern as a plain JS object.
    #[wasm_bindgen(js_name = toObject)]
    pub fn to_object(&self) -> Result<JsValue, JsError> {
        to_js(&self.pattern)
    }

    #[wasm_bindgen(js_name = fromObject)]
    pub fn from_object(value: JsValue) -> Result<PatternDoc, JsError> {
        Ok(Self { pattern: serde_wasm_bindgen::from_value(value)? })
    }

    /// Bounds of all pieces as `[minX, minY, maxX, maxY]`, or `undefined`
    /// for an empty pattern.
    pub fn bounds(&self) -> Option<Vec<f64>> {
        self.pattern.pieces.iter().filter_map(Piece::bounds).reduce(|(alo, ahi), (blo, bhi)| {
            (Vec2::new(alo.x.min(blo.x), alo.y.min(blo.y)), Vec2::new(ahi.x.max(bhi.x), ahi.y.max(bhi.y)))
        })
        .map(|(lo, hi)| vec![lo.x, lo.y, hi.x, hi.y])
    }

    // ---- pieces -----------------------------------------------------------

    #[wasm_bindgen(js_name = pieceCount)]
    pub fn piece_count(&self) -> usize {
        self.pattern.pieces.len()
    }

    #[wasm_bindgen(js_name = pieceName)]
    pub fn piece_name(&self, piece: usize) -> Result<String, JsError> {
        Ok(self.piece(piece)?.name.clone())
    }

    #[wasm_bindgen(js_name = setPieceName)]
    pub fn set_piece_name(&mut self, piece: usize, name: String) -> Result<(), JsError> {
        self.piece_mut(piece)?.name = name;
        Ok(())
    }

    #[wasm_bindgen(js_name = isClosed)]
    pub fn is_closed(&self, piece: usize) -> Result<bool, JsError> {
        Ok(self.piece(piece)?.closed)
    }

    /// Add a piece from `[x0, y0, x1, y1, ...]` turn points. Returns its index.
    #[wasm_bindgen(js_name = addPiece)]
    pub fn add_piece(&mut self, name: String, coords: &[f64], closed: bool) -> usize {
        let mut piece = Piece::new(name);
        piece.points = coords.as_chunks::<2>().0.iter().map(|&[x, y]| Point::turn(Vec2::new(x, y))).collect();
        piece.closed = closed;
        self.pattern.pieces.push(piece);
        self.pattern.pieces.len() - 1
    }

    /// Remove a piece and any seams touching it.
    #[wasm_bindgen(js_name = removePiece)]
    pub fn remove_piece(&mut self, piece: usize) -> Result<(), JsError> {
        self.piece(piece)?;
        self.pattern.remove_piece(piece);
        Ok(())
    }

    /// Copy a piece (without seams) next to the original. Returns its index.
    #[wasm_bindgen(js_name = duplicatePiece)]
    pub fn duplicate_piece(&mut self, piece: usize) -> Result<usize, JsError> {
        self.piece(piece)?;
        Ok(self.pattern.duplicate_piece(piece))
    }

    /// Mirror a closed piece across the straight, contiguous fold segments
    /// `segs` and merge both halves.
    pub fn unfold(&mut self, piece: usize, segs: &[usize]) -> Result<(), JsError> {
        self.piece(piece)?;
        self.pattern.unfold(piece, segs).map_err(JsError::new)
    }

    #[wasm_bindgen(js_name = translatePiece)]
    pub fn translate_piece(&mut self, piece: usize, dx: f64, dy: f64) -> Result<(), JsError> {
        self.piece_mut(piece)?.translate(Vec2::new(dx, dy));
        Ok(())
    }

    /// Rotate counter-clockwise about the piece's center.
    #[wasm_bindgen(js_name = rotatePiece)]
    pub fn rotate_piece(&mut self, piece: usize, degrees: f64) -> Result<(), JsError> {
        self.piece_mut(piece)?.rotate(degrees);
        Ok(())
    }

    #[wasm_bindgen(js_name = flipPiece)]
    pub fn flip_piece(&mut self, piece: usize, horizontal: bool) -> Result<(), JsError> {
        self.piece_mut(piece)?.flip(horizontal);
        Ok(())
    }

    pub fn area(&self, piece: usize) -> Result<f64, JsError> {
        Ok(self.piece(piece)?.area())
    }

    pub fn perimeter(&self, piece: usize) -> Result<f64, JsError> {
        Ok(self.piece(piece)?.perimeter())
    }

    pub fn contains(&self, piece: usize, x: f64, y: f64) -> Result<bool, JsError> {
        Ok(self.piece(piece)?.contains(Vec2::new(x, y)))
    }

    /// Smooth outline polyline for drawing, `[x0, y0, ...]`.
    pub fn flatten(&self, piece: usize) -> Result<Vec<f64>, JsError> {
        Ok(flat(&self.piece(piece)?.flatten()))
    }

    #[wasm_bindgen(js_name = fillMesh)]
    pub fn fill_mesh(&self, piece: usize) -> Result<Mesh, JsError> {
        let (verts, tris) = self.piece(piece)?.fill_triangles();
        Ok(Mesh { vertices: flat(&verts), indices: tris.iter().flatten().map(|&i| i as u32).collect() })
    }

    /// Nearest outline position to `(x, y)` as `[seg, t, x, y, distance]`,
    /// or `undefined` if the piece has no segments.
    #[wasm_bindgen(js_name = closestOnOutline)]
    pub fn closest_on_outline(&self, piece: usize, x: f64, y: f64) -> Result<Option<Vec<f64>>, JsError> {
        Ok(self.piece(piece)?.closest_on_outline(Vec2::new(x, y)).map(|(seg, t, q, d)| vec![seg as f64, t, q.x, q.y, d]))
    }

    /// Point on the outline at `{seg, t}`, as `[x, y]`.
    #[wasm_bindgen(js_name = evalPos)]
    pub fn eval_pos(&self, piece: usize, seg: usize, t: f64) -> Result<Vec<f64>, JsError> {
        let p = self.segment(piece, seg)?.eval_pos(OutlinePos::new(seg, t));
        Ok(vec![p.x, p.y])
    }

    /// Outline position `dist` mm from `{seg, t}`, as `[seg, t]`.
    #[wasm_bindgen(js_name = posAtDistance)]
    pub fn pos_at_distance(&self, piece: usize, seg: usize, t: f64, dist: f64, forward: bool) -> Result<Option<Vec<f64>>, JsError> {
        let p = self.segment(piece, seg)?;
        Ok(p.pos_at_distance(OutlinePos::new(seg, t), dist, forward).map(|o| vec![o.seg as f64, o.t]))
    }

    // ---- points -----------------------------------------------------------

    /// Control points as `[x0, y0, ...]`.
    pub fn points(&self, piece: usize) -> Result<Vec<f64>, JsError> {
        Ok(self.piece(piece)?.points.iter().flat_map(|p| [p.pos.x, p.pos.y]).collect())
    }

    /// Per-point kind: 0 = turn (corner), 1 = curve.
    #[wasm_bindgen(js_name = pointKinds)]
    pub fn point_kinds(&self, piece: usize) -> Result<Vec<u8>, JsError> {
        Ok(self.piece(piece)?.points.iter().map(|p| (p.kind == PointKind::Curve) as u8).collect())
    }

    #[wasm_bindgen(js_name = movePoint)]
    pub fn move_point(&mut self, piece: usize, point: usize, x: f64, y: f64) -> Result<(), JsError> {
        let p = self.piece_mut(piece)?.points.get_mut(point).ok_or_else(|| JsError::new("point index out of range"))?;
        p.pos = Vec2::new(x, y);
        Ok(())
    }

    #[wasm_bindgen(js_name = setCurvePoint)]
    pub fn set_curve_point(&mut self, piece: usize, point: usize, curve: bool) -> Result<(), JsError> {
        self.point(piece, point)?;
        let kind = if curve { PointKind::Curve } else { PointKind::Turn };
        self.pattern.pieces[piece].set_point_kind(point, kind);
        Ok(())
    }

    // ---- curves -----------------------------------------------------------

    /// Handles to draw for editing a point, as offsets from it:
    /// `[inX, inY, outX, outY]`. Explicit handles, plus automatic ones on curve
    /// points; a missing handle is `NaN, NaN`.
    #[wasm_bindgen(js_name = visibleHandles)]
    pub fn visible_handles(&self, piece: usize, point: usize) -> Result<Vec<f64>, JsError> {
        let (hin, hout) = self.point(piece, point)?.visible_handles(point);
        let xy = |h: Option<Vec2>| h.map_or([f64::NAN, f64::NAN], |v| [v.x, v.y]);
        Ok([xy(hin), xy(hout)].concat())
    }

    /// Set a point's incoming (`out = false`) or outgoing handle to offset
    /// `(dx, dy)`. On a smooth (curve) point the opposite handle mirrors it
    /// when `mirror`; otherwise the handles break apart into a corner.
    #[wasm_bindgen(js_name = setHandle)]
    pub fn set_handle(&mut self, piece: usize, point: usize, out: bool, dx: f64, dy: f64, mirror: bool) -> Result<(), JsError> {
        self.point(piece, point)?;
        let side = if out { HandleSide::Out } else { HandleSide::In };
        self.pattern.pieces[piece].set_handle(point, side, Vec2::new(dx, dy), mirror);
        Ok(())
    }

    /// Remove a point's explicit handles.
    #[wasm_bindgen(js_name = clearHandles)]
    pub fn clear_handles(&mut self, piece: usize, point: usize) -> Result<(), JsError> {
        self.point(piece, point)?;
        self.pattern.pieces[piece].clear_handles(point);
        Ok(())
    }

    /// Cubic Bézier of a segment, `[p0x, p0y, c0x, c0y, c1x, c1y, p1x, p1y]`.
    #[wasm_bindgen(js_name = segmentBezier)]
    pub fn segment_bezier(&self, piece: usize, seg: usize) -> Result<Vec<f64>, JsError> {
        Ok(flat(&self.segment(piece, seg)?.segment_bezier(seg)))
    }

    /// Bend a segment so its point at parameter `t` moves by `(dx, dy)`.
    /// For an interactive drag, call with the total offset on a copy taken at
    /// drag start, or with per-frame deltas (the result is the same).
    #[wasm_bindgen(js_name = bendSegment)]
    pub fn bend_segment(&mut self, piece: usize, seg: usize, t: f64, dx: f64, dy: f64) -> Result<(), JsError> {
        let p = self.segment(piece, seg)?;
        let start = p.segment_handles(seg);
        self.pattern.pieces[piece].bend_segment(seg, t, start, Vec2::new(dx, dy));
        Ok(())
    }

    /// Make a segment straight; neighboring segments keep their shape.
    #[wasm_bindgen(js_name = straightenSegment)]
    pub fn straighten_segment(&mut self, piece: usize, seg: usize) -> Result<(), JsError> {
        self.segment(piece, seg)?;
        self.pattern.pieces[piece].straighten_segment(seg);
        Ok(())
    }

    /// Insert a point on segment `seg` at parameter `t`, keeping seams in
    /// place. Returns the new point's index.
    #[wasm_bindgen(js_name = insertPoint)]
    pub fn insert_point(&mut self, piece: usize, seg: usize, t: f64) -> Result<usize, JsError> {
        self.segment(piece, seg)?;
        Ok(self.pattern.insert_point(piece, seg, t))
    }

    /// Remove a point, keeping seams in place. Returns `false` if refused.
    #[wasm_bindgen(js_name = removePoint)]
    pub fn remove_point(&mut self, piece: usize, point: usize) -> Result<bool, JsError> {
        if point >= self.piece(piece)?.points.len() {
            return Err(JsError::new("point index out of range"));
        }
        Ok(self.pattern.remove_point(piece, point))
    }

    // ---- segments ---------------------------------------------------------

    #[wasm_bindgen(js_name = segmentCount)]
    pub fn segment_count(&self, piece: usize) -> Result<usize, JsError> {
        Ok(self.piece(piece)?.segment_count())
    }

    #[wasm_bindgen(js_name = segmentLength)]
    pub fn segment_length(&self, piece: usize, seg: usize) -> Result<f64, JsError> {
        Ok(self.segment(piece, seg)?.segment_length(seg))
    }

    #[wasm_bindgen(js_name = segmentIsCurved)]
    pub fn segment_is_curved(&self, piece: usize, seg: usize) -> Result<bool, JsError> {
        Ok(self.segment(piece, seg)?.segment_is_curved(seg))
    }

    /// Resize a segment by moving its end (or start) point; curved segments
    /// are solved for arc length. Returns `false` if it can't be reached.
    #[wasm_bindgen(js_name = setSegmentLength)]
    pub fn set_segment_length(&mut self, piece: usize, seg: usize, length: f64, move_start: bool) -> Result<bool, JsError> {
        self.segment(piece, seg)?;
        Ok(self.pattern.pieces[piece].set_segment_length(seg, length, move_start))
    }

    // ---- seams ------------------------------------------------------------

    #[wasm_bindgen(js_name = seamCount)]
    pub fn seam_count(&self) -> usize {
        self.pattern.seams.len()
    }

    /// Seam `{a, b}` where each side is an `EdgeSpan`
    /// `{piece, start: {seg, t}, end: {seg, t}, forward}`.
    pub fn seam(&self, seam: usize) -> Result<JsValue, JsError> {
        to_js(self.pattern.seams.get(seam).ok_or_else(|| JsError::new("seam index out of range"))?)
    }

    /// The shorter span between two outline positions on one piece.
    #[wasm_bindgen(js_name = shortestSpan)]
    pub fn shortest_span(&self, piece: usize, start_seg: usize, start_t: f64, end_seg: usize, end_t: f64) -> Result<JsValue, JsError> {
        self.segment(piece, start_seg)?;
        self.segment(piece, end_seg)?;
        let span = EdgeSpan::shortest(&self.pattern.pieces, piece, OutlinePos::new(start_seg, start_t), OutlinePos::new(end_seg, end_t));
        to_js(&span)
    }

    /// Sew span `a` to span `b` (starts are sewn together). Returns the
    /// seam's index.
    #[wasm_bindgen(js_name = addSeam)]
    pub fn add_seam(&mut self, a: JsValue, b: JsValue) -> Result<usize, JsError> {
        let seam = Seam { a: serde_wasm_bindgen::from_value(a)?, b: serde_wasm_bindgen::from_value(b)? };
        for span in seam.spans() {
            let n = self.piece(span.piece)?.segment_count();
            if span.start.seg >= n || span.end.seg >= n {
                return Err(JsError::new("span segment out of range"));
            }
        }
        self.pattern.seams.push(seam);
        Ok(self.pattern.seams.len() - 1)
    }

    #[wasm_bindgen(js_name = removeSeam)]
    pub fn remove_seam(&mut self, seam: usize) -> Result<(), JsError> {
        if seam >= self.pattern.seams.len() {
            return Err(JsError::new("seam index out of range"));
        }
        self.pattern.seams.remove(seam);
        Ok(())
    }

    /// Lengths of both sides of a seam, `[a, b]`.
    #[wasm_bindgen(js_name = seamLengths)]
    pub fn seam_lengths(&self, seam: usize) -> Result<Vec<f64>, JsError> {
        let s = self.pattern.seams.get(seam).ok_or_else(|| JsError::new("seam index out of range"))?;
        Ok(s.spans().iter().map(|sp| sp.length(&self.pattern.pieces)).collect())
    }

    /// Polylines of both sides of a seam, `[aCoords, bCoords]`.
    #[wasm_bindgen(js_name = seamPolylines)]
    pub fn seam_polylines(&self, seam: usize) -> Result<Vec<js_sys::Float64Array>, JsError> {
        let s = self.pattern.seams.get(seam).ok_or_else(|| JsError::new("seam index out of range"))?;
        Ok(s.spans().iter().map(|sp| js_sys::Float64Array::from(&flat(&sp.polyline(&self.pattern.pieces))[..])).collect())
    }
}

#[wasm_bindgen]
impl PatternDoc {
    /// Export closed pieces as DXF-AAMA/ASTM. Returns `{text, skipped}`,
    /// where `skipped` lists the names of open pieces left out.
    #[wasm_bindgen(js_name = toAamaDxf)]
    pub fn to_aama_dxf(&self, style_name: &str) -> Result<JsValue, JsError> {
        to_js(&clothes3d_core::dxf::export_aama(&self.pattern, style_name))
    }
}

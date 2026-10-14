//! JSON descriptions of the pattern for MCP replies, in display units.

use serde_json::{Value, json};

use crate::model::{Pattern, Piece, PointKind, Vec2};
use crate::sewing::EdgeSpan;
use crate::units::Units;

/// Converts mm to the reply's units, rounded to keep replies short.
#[derive(Clone, Copy)]
pub struct Fmt {
    pub units: Units,
}

impl Fmt {
    pub fn len(self, mm: f64) -> f64 {
        let v = self.units.display_value(mm);
        let scale = match self.units {
            Units::Millimeters => 1e3,
            Units::Inches => 1e4,
        };
        let r = (v * scale).round() / scale;
        if r == 0.0 { 0.0 } else { r } // no "-0"
    }

    pub fn area(self, mm2: f64) -> f64 {
        let s = self.units.display_value(1.0);
        self.len(mm2 * s) // one factor here, one in `len`
    }

    pub fn xy(self, v: Vec2) -> Value {
        json!({ "x": self.len(v.x), "y": self.len(v.y) })
    }

    pub fn name(self) -> &'static str {
        match self.units {
            Units::Millimeters => "mm",
            Units::Inches => "in",
        }
    }
}

fn bounds(f: Fmt, piece: &Piece) -> Value {
    piece.bounds().map_or(Value::Null, |(lo, hi)| json!({ "min": f.xy(lo), "max": f.xy(hi) }))
}

fn piece_seams(pattern: &Pattern, piece: usize) -> Vec<usize> {
    (0..pattern.seams.len())
        .filter(|&s| pattern.seams[s].spans().iter().any(|sp| sp.piece == piece))
        .collect()
}

/// One line per piece, for the pattern overview.
pub fn piece_summary(f: Fmt, pattern: &Pattern, i: usize) -> Value {
    let p = &pattern.pieces[i];
    json!({
        "index": i,
        "name": p.name,
        "closed": p.closed,
        "points": p.points.len(),
        "segments": p.segment_count(),
        "perimeter": f.len(p.perimeter()),
        "area": if p.closed { f.area(p.area().abs()) } else { 0.0 },
        "bounds": bounds(f, p),
    })
}

/// Everything about one piece: points, handles and segment lengths.
pub fn piece_detail(f: Fmt, pattern: &Pattern, i: usize) -> Value {
    let p = &pattern.pieces[i];
    let points: Vec<Value> = p
        .points
        .iter()
        .enumerate()
        .map(|(j, pt)| {
            let mut v = json!({
                "index": j,
                "x": f.len(pt.pos.x),
                "y": f.len(pt.pos.y),
                "kind": match pt.kind { PointKind::Turn => "turn", PointKind::Curve => "curve" },
            });
            if let Some(h) = pt.handle_in {
                v["handle_in"] = f.xy(h);
            }
            if let Some(h) = pt.handle_out {
                v["handle_out"] = f.xy(h);
            }
            v
        })
        .collect();
    let segments: Vec<Value> = (0..p.segment_count())
        .map(|s| {
            let (a, b) = p.segment_ends(s);
            json!({
                "index": s,
                "from": a,
                "to": b,
                "length": f.len(p.segment_length(s)),
                "curved": p.segment_is_curved(s),
            })
        })
        .collect();
    let mut v = piece_summary(f, pattern, i);
    v["points"] = points.into();
    v["segments"] = segments.into();
    v["seams"] = piece_seams(pattern, i).into();
    v
}

fn span(f: Fmt, pattern: &Pattern, s: &EdgeSpan) -> Value {
    let pos = |p: crate::sewing::OutlinePos| json!({ "segment": p.seg, "t": (p.t * 1e4).round() / 1e4 });
    json!({
        "piece": s.piece,
        "piece_name": pattern.pieces[s.piece].name,
        "start": pos(s.start),
        "end": pos(s.end),
        "forward": s.forward,
        "length": f.len(s.length(&pattern.pieces)),
    })
}

/// Seam `i` with both edge lengths and whether they match within `tolerance` (mm).
pub fn seam(f: Fmt, pattern: &Pattern, i: usize, tolerance: f64) -> Value {
    let s = &pattern.seams[i];
    let [la, lb] = s.spans().map(|sp| sp.length(&pattern.pieces));
    json!({
        "index": i,
        "a": span(f, pattern, &s.a),
        "b": span(f, pattern, &s.b),
        "difference": f.len(la - lb),
        "within_tolerance": (la - lb).abs() <= tolerance,
    })
}

/// Seams on any of `pieces` whose edges differ by more than `tolerance` mm.
pub fn seam_warnings(f: Fmt, pattern: &Pattern, pieces: &[usize], tolerance: f64) -> Vec<Value> {
    (0..pattern.seams.len())
        .filter(|&i| {
            let s = &pattern.seams[i];
            let [la, lb] = s.spans().map(|sp| sp.length(&pattern.pieces));
            (la - lb).abs() > tolerance && s.spans().iter().any(|sp| pieces.contains(&sp.piece))
        })
        .map(|i| seam(f, pattern, i, tolerance))
        .collect()
}

pub fn overview(f: Fmt, pattern: &Pattern, tolerance: f64) -> Value {
    json!({
        "units": f.name(),
        "seam_tolerance": f.len(tolerance),
        "pieces": (0..pattern.pieces.len()).map(|i| piece_summary(f, pattern, i)).collect::<Vec<_>>(),
        "seams": (0..pattern.seams.len()).map(|i| seam(f, pattern, i, tolerance)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_and_area() {
        let mm = Fmt { units: Units::Millimeters };
        let inch = Fmt { units: Units::Inches };
        assert_eq!(mm.len(12.345_678), 12.346);
        assert_eq!(inch.len(25.4), 1.0);
        assert_eq!(inch.area(25.4 * 25.4 * 2.0), 2.0);
        assert_eq!(mm.len(-1e-9), 0.0);
    }
}

//! DXF-AAMA/ASTM export (ASTM D6673 style, ASCII DXF R12).
//!
//! Each closed piece becomes a BLOCK, placed with an INSERT at the origin so
//! the layout is preserved:
//! - layer 1: boundary POLYLINE, plus "Piece Name" / "Quantity" TEXT
//! - layer 2: turn points (POINT)
//! - layer 3: curve points (POINT)
//!
//! Bézier segments can't be expressed directly, so curved segments are
//! densified: their sampled interior vertices go on the boundary polyline
//! without a layer-2/3 marker. Coordinates are written as-is (millimetres,
//! Y up) and declared with `Units: METRIC`.
//!
//! Not yet exported: seams (AAMA has no seam concept), notches, grain lines,
//! internal lines, seam allowance, grading.

use std::collections::HashSet;
use std::fmt::{Display, Write};

use serde::Serialize;

use crate::model::{Pattern, Piece, PointKind, Vec2};

/// Height of annotation text, in mm.
const TEXT_HEIGHT: f64 = 5.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DxfExport {
    /// The DXF file contents.
    pub text: String,
    /// Names of pieces left out because their outline isn't closed.
    pub skipped: Vec<String>,
}

/// Export the closed pieces of `pattern` as a DXF-AAMA/ASTM file.
/// `style_name` is written to the file's "Style Name" annotation.
pub fn export_aama(pattern: &Pattern, style_name: &str) -> DxfExport {
    let mut w = DxfWriter::default();
    let mut skipped = Vec::new();
    let mut used_names = HashSet::new();
    let mut blocks = Vec::new();
    for piece in &pattern.pieces {
        if piece.closed && piece.points.len() >= 3 {
            blocks.push((piece, unique_block_name(&piece.name, &mut used_names)));
        } else {
            skipped.push(piece.name.clone());
        }
    }

    w.section("HEADER");
    w.pair(9, "$ACADVER");
    w.pair(1, "AC1009");
    w.end_section();

    w.section("TABLES");
    w.pair(0, "TABLE");
    w.pair(2, "LAYER");
    w.pair(70, 3);
    for layer in 1..=3 {
        w.pair(0, "LAYER");
        w.pair(2, layer);
        w.pair(70, 0);
        w.pair(62, 7);
        w.pair(6, "CONTINUOUS");
    }
    w.pair(0, "ENDTAB");
    w.end_section();

    w.section("BLOCKS");
    for (piece, name) in &blocks {
        w.piece_block(piece, name);
    }
    w.end_section();

    w.section("ENTITIES");
    let header = [
        format!("Style Name: {style_name}"),
        "Units: METRIC".to_string(),
        "Sample Size:".to_string(),
        "Grading Rule Table:".to_string(),
    ];
    for (i, line) in header.iter().enumerate() {
        w.text(1, Vec2::new(0.0, -2.0 * TEXT_HEIGHT * (i + 1) as f64), line);
    }
    for (_, name) in &blocks {
        w.pair(0, "INSERT");
        w.pair(8, 1);
        w.pair(2, name);
        w.point(10, Vec2::default());
    }
    w.end_section();
    w.pair(0, "EOF");

    DxfExport { text: w.out, skipped }
}

/// A DXF-safe block name derived from `name`, unique within `used`.
fn unique_block_name(name: &str, used: &mut HashSet<String>) -> String {
    let base: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_".contains(c) { c } else { '_' })
        .collect();
    let base = if base.is_empty() { "PIECE".to_string() } else { base };
    let mut candidate = base.clone();
    let mut n = 2;
    while !used.insert(candidate.to_ascii_uppercase()) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

/// Escape text for R12: non-ASCII characters become `\U+XXXX`.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() && !c.is_ascii_control() {
            out.push(c);
        } else if c.is_ascii_control() {
            out.push(' ');
        } else {
            let mut units = [0u16; 2];
            for u in c.encode_utf16(&mut units) {
                let _ = write!(out, "\\U+{u:04X}");
            }
        }
    }
    out
}

/// Boundary vertices of a closed piece (no repeated closing vertex), with
/// the control point each one came from, if any.
fn boundary(piece: &Piece) -> Vec<(Vec2, Option<PointKind>)> {
    let mut out = Vec::new();
    for seg in 0..piece.segment_count() {
        let pts = piece.flatten_segment(seg);
        out.push((pts[0], Some(piece.points[seg].kind)));
        // Interior samples; the end is the next segment's start.
        out.extend(pts[1..pts.len() - 1].iter().map(|&p| (p, None)));
    }
    out
}

#[derive(Default)]
struct DxfWriter {
    out: String,
}

impl DxfWriter {
    fn pair(&mut self, code: u16, value: impl Display) {
        let _ = write!(self.out, "{code:>3}\r\n{value}\r\n");
    }

    fn num(&mut self, code: u16, v: f64) {
        // Trim to 4 decimals (0.1 µm); avoid "-0".
        let v = if v.abs() < 5e-5 { 0.0 } else { v };
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        self.pair(code, s);
    }

    /// Coordinates with group codes `code`, `code + 10`, `code + 20`.
    fn point(&mut self, code: u16, p: Vec2) {
        self.num(code, p.x);
        self.num(code + 10, p.y);
        self.num(code + 20, 0.0);
    }

    fn section(&mut self, name: &str) {
        self.pair(0, "SECTION");
        self.pair(2, name);
    }

    fn end_section(&mut self) {
        self.pair(0, "ENDSEC");
    }

    fn text(&mut self, layer: u8, at: Vec2, s: &str) {
        self.pair(0, "TEXT");
        self.pair(8, layer);
        self.point(10, at);
        self.num(40, TEXT_HEIGHT);
        self.pair(1, escape_text(s));
    }

    fn piece_block(&mut self, piece: &Piece, name: &str) {
        self.pair(0, "BLOCK");
        self.pair(8, 1);
        self.pair(2, name);
        self.pair(70, 0);
        self.point(10, Vec2::default());
        self.pair(3, name);

        let verts = boundary(piece);
        self.pair(0, "POLYLINE");
        self.pair(8, 1);
        self.pair(66, 1);
        self.point(10, Vec2::default());
        self.pair(70, 1); // closed
        for &(p, _) in &verts {
            self.pair(0, "VERTEX");
            self.pair(8, 1);
            self.point(10, p);
        }
        self.pair(0, "SEQEND");
        self.pair(8, 1);

        for &(p, kind) in &verts {
            let layer = match kind {
                Some(PointKind::Turn) => 2,
                Some(PointKind::Curve) => 3,
                None => continue,
            };
            self.pair(0, "POINT");
            self.pair(8, layer);
            self.point(10, p);
        }

        let c = piece.center();
        self.text(1, c, &format!("Piece Name: {}", piece.name));
        self.text(1, c - Vec2::new(0.0, 2.0 * TEXT_HEIGHT), "Quantity: 1");

        self.pair(0, "ENDBLK");
        self.pair(8, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;

    fn square(name: &str, x: f64) -> Piece {
        let mut p = Piece::new(name);
        p.points = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
            .map(|(px, py)| Point::turn(Vec2::new(x + px, py)))
            .to_vec();
        p.closed = true;
        p
    }

    /// Parse into (code, value) pairs, checking the line structure.
    fn pairs(text: &str) -> Vec<(u16, String)> {
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len() % 2, 0, "odd number of lines");
        lines.chunks(2).map(|c| (c[0].trim().parse().expect("group code"), c[1].to_string())).collect()
    }

    fn entities<'a>(pairs: &'a [(u16, String)], kind: &str) -> Vec<&'a [(u16, String)]> {
        let starts: Vec<usize> = (0..pairs.len()).filter(|&i| pairs[i].0 == 0).collect();
        starts
            .iter()
            .enumerate()
            .filter(|&(_, &s)| pairs[s].1 == kind)
            .map(|(k, &s)| &pairs[s..starts.get(k + 1).copied().unwrap_or(pairs.len())])
            .collect()
    }

    fn value(entity: &[(u16, String)], code: u16) -> &str {
        &entity.iter().find(|(c, _)| *c == code).unwrap().1
    }

    #[test]
    fn square_structure() {
        let pattern = Pattern { pieces: vec![square("Front", 0.0)], seams: vec![] };
        let out = export_aama(&pattern, "Test");
        assert!(out.skipped.is_empty());
        assert!(out.text.contains("\r\n"));
        let p = pairs(&out.text);
        assert_eq!(p.last().unwrap(), &(0, "EOF".to_string()));
        assert_eq!(entities(&p, "SECTION").len(), 4);
        assert_eq!(entities(&p, "ENDSEC").len(), 4);
        assert_eq!(entities(&p, "BLOCK").len(), 1);
        assert_eq!(value(entities(&p, "INSERT")[0], 2), "Front");
        assert_eq!(value(entities(&p, "POLYLINE")[0], 70), "1");
        let verts = entities(&p, "VERTEX");
        assert_eq!(verts.len(), 4);
        assert_eq!((value(verts[2], 10), value(verts[2], 20)), ("100", "100"));
        let points = entities(&p, "POINT");
        assert_eq!(points.len(), 4);
        assert!(points.iter().all(|e| value(e, 8) == "2"));
        let texts: Vec<&str> = entities(&p, "TEXT").iter().map(|e| value(e, 1)).collect();
        assert!(texts.contains(&"Piece Name: Front"));
        assert!(texts.contains(&"Units: METRIC"));
        assert!(texts.contains(&"Style Name: Test"));
    }

    #[test]
    fn curves_are_densified_and_marked() {
        let mut piece = square("Curvy", 0.0);
        piece.points[1].kind = PointKind::Curve;
        let pattern = Pattern { pieces: vec![piece.clone()], seams: vec![] };
        let p = pairs(&export_aama(&pattern, "S").text);
        let verts = entities(&p, "VERTEX");
        // Segments 0 and 1 touch the curve point and are sampled.
        assert!(verts.len() > 4);
        let points = entities(&p, "POINT");
        assert_eq!(points.iter().filter(|e| value(e, 8) == "3").count(), 1);
        assert_eq!(points.iter().filter(|e| value(e, 8) == "2").count(), 3);
        // Every vertex lies on the true outline.
        for v in verts {
            let q = Vec2::new(value(v, 10).parse().unwrap(), value(v, 20).parse().unwrap());
            assert!(piece.closest_on_outline(q).unwrap().3 < 1e-3);
        }
    }

    #[test]
    fn open_pieces_skipped_and_names_unique() {
        let mut open = square("Open", 0.0);
        open.closed = false;
        let pattern = Pattern {
            pieces: vec![square("Sleeve", 0.0), square("sleeve", 200.0), square("Back/Yoke ✂", 400.0), open],
            seams: vec![],
        };
        let out = export_aama(&pattern, "S");
        assert_eq!(out.skipped, ["Open"]);
        let p = pairs(&out.text);
        let names: Vec<&str> = entities(&p, "BLOCK").iter().map(|e| value(e, 2)).collect();
        assert_eq!(names, ["Sleeve", "sleeve_2", "Back_Yoke__"]);
        let texts: Vec<&str> = entities(&p, "TEXT").iter().map(|e| value(e, 1)).collect();
        assert!(texts.contains(&"Piece Name: Back/Yoke \\U+2702"));
    }
}

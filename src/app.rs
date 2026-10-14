use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke,
};

use serde::{Deserialize, Serialize};

use crate::model::{Pattern, Piece, Point, PointKind, Vec2};
use crate::sewing::{EdgeSpan, OutlinePos, Seam, point_at_fraction};
use crate::view::View;

/// Pick radius for points and lines, in screen pixels.
const PICK_PX: f32 = 8.0;
const UNDO_LIMIT: usize = 200;
/// Storage key for the persisted working session.
const SESSION_KEY: &str = "session";
/// Screen distance within which a sewing end snaps to the matching-length point.
const MATCH_SNAP_PX: f32 = 12.0;
const OK_COLOR: Color32 = Color32::from_rgb(0, 140, 60);
const WARN_COLOR: Color32 = Color32::from_rgb(220, 130, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Select,
    Draw,
    AddPoint,
    Sew,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select (V)",
            Tool::Draw => "Draw piece (P)",
            Tool::AddPoint => "Add point (A)",
            Tool::Sew => "Free sewing (S)",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tool::Select => {
                "Click a point or edge to select (Shift+click for several). Type a length + Enter \
                 to resize one segment. Drag to move (arrows nudge). Double-click an edge to add \
                 a point. R rotate, H flip, Ctrl+D duplicate, U unfold along selected edges"
            }
            Tool::Draw => {
                "Click to place points, or type a length + Enter to place the next point toward the \
                 cursor. Click the first point or press Enter to close, Esc to stop"
            }
            Tool::AddPoint => "Click on an outline to insert a point",
            Tool::Sew => "Click a start point on an edge",
        }
    }
}

/// An in-progress drag with the Select tool.
enum Drag {
    /// Selected points, moved together by the grabbed point's (snapped) offset.
    Points { grabbed: PointRef, original: Vec<(PointRef, Vec2)> },
    /// Whole piece; positions are offset from the originals so grid snapping
    /// applies to the total displacement rather than accumulating error.
    Piece { piece: usize, start: Vec2, original: Vec<Vec2> },
}

/// Progress through the four clicks of the free sewing tool.
#[derive(Clone, Copy, Debug)]
enum SewState {
    Idle,
    /// First span's start chosen.
    FirstStart(usize, OutlinePos),
    /// First span complete.
    First(EdgeSpan),
    /// First span complete, second span's start chosen.
    SecondStart(EdgeSpan, usize, OutlinePos),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PointRef {
    piece: usize,
    point: usize,
}

/// Working state restored on the next launch.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Session {
    pattern: Pattern,
    file: Option<PathBuf>,
    view: View,
    snap_grid: bool,
    snap_step: f64,
    snap_points: bool,
    snap_match: bool,
    seam_tolerance: f64,
}

impl Default for Session {
    fn default() -> Self {
        App::default().session()
    }
}

pub struct App {
    pattern: Pattern,
    view: View,
    tool: Tool,
    selected_piece: Option<usize>,
    /// Selected points; Shift+click adds or removes.
    selected_points: Vec<PointRef>,
    selected_seam: Option<usize>,
    /// Selected outline segments `(piece, seg)`; Shift+click adds or removes.
    selected_segments: Vec<(usize, usize)>,
    /// Digits typed for a precise length (draw tool or selected segment).
    length_input: String,
    /// Segment length edits move the segment's start point instead of its end.
    length_moves_start: bool,
    /// Angle (degrees, counter-clockwise) for the "Rotate by" command.
    rotate_angle: f64,
    sew: SewState,
    /// Piece currently being drawn with the Draw tool.
    drawing: Option<usize>,
    /// Point or piece being dragged with the Select tool.
    dragging: Option<Drag>,
    undo: Vec<Pattern>,
    redo: Vec<Pattern>,
    snap_grid: bool,
    snap_step: f64,
    snap_points: bool,
    /// Snap the end of the second sewing edge to where its length matches the first.
    snap_match: bool,
    /// Seam length difference (mm) above which a seam is flagged.
    seam_tolerance: f64,
    file: Option<PathBuf>,
    status: String,
    cursor_world: Option<Vec2>,
    fit_requested: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            pattern: Pattern::default(),
            view: View::default(),
            tool: Tool::Draw,
            selected_piece: None,
            selected_points: Vec::new(),
            selected_seam: None,
            selected_segments: Vec::new(),
            length_input: String::new(),
            length_moves_start: false,
            rotate_angle: 15.0,
            sew: SewState::Idle,
            drawing: None,
            dragging: None,
            undo: Vec::new(),
            redo: Vec::new(),
            snap_grid: true,
            snap_step: 10.0,
            snap_points: true,
            snap_match: true,
            seam_tolerance: 1.0,
            file: None,
            status: String::new(),
            cursor_world: None,
            fit_requested: false,
        }
    }
}

impl App {
    /// Create the app, restoring the previous session from `storage`. A
    /// pattern file given on the command line takes precedence.
    pub fn new(storage: Option<&dyn eframe::Storage>, path: Option<PathBuf>) -> Self {
        let mut app = Self::default();
        if let Some(session) = storage.and_then(|s| eframe::get_value::<Session>(s, SESSION_KEY)) {
            app.restore(session);
        }
        if let Some(path) = path {
            app.load(path);
            // Loading pushes an undo step; a fresh launch shouldn't start with one.
            app.undo.clear();
        }
        app
    }

    fn session(&self) -> Session {
        Session {
            pattern: self.pattern.clone(),
            file: self.file.clone(),
            view: self.view,
            snap_grid: self.snap_grid,
            snap_step: self.snap_step,
            snap_points: self.snap_points,
            snap_match: self.snap_match,
            seam_tolerance: self.seam_tolerance,
        }
    }

    fn restore(&mut self, s: Session) {
        self.pattern = s.pattern;
        self.file = s.file;
        self.view = s.view;
        self.snap_grid = s.snap_grid;
        self.snap_step = s.snap_step;
        self.snap_points = s.snap_points;
        self.snap_match = s.snap_match;
        self.seam_tolerance = s.seam_tolerance;
        // A stale or hand-edited session must not leave dangling indices.
        self.validate_selection();
    }

    // ---------------------------------------------------------------- history

    /// Snapshot the pattern before a modification.
    fn checkpoint(&mut self) {
        self.undo.push(self.pattern.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.pattern, prev));
            self.after_history_change();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.pattern, next));
            self.after_history_change();
        }
    }

    fn after_history_change(&mut self) {
        self.drawing = None;
        self.dragging = None;
        self.sew = SewState::Idle;
        self.validate_selection();
    }

    fn validate_selection(&mut self) {
        let n = self.pattern.pieces.len();
        if self.selected_piece.is_some_and(|i| i >= n) {
            self.selected_piece = None;
        }
        let pieces = &self.pattern.pieces;
        self.selected_points.retain(|r| pieces.get(r.piece).is_some_and(|p| r.point < p.points.len()));
        self.selected_segments
            .retain(|&(pi, seg)| pieces.get(pi).is_some_and(|p| seg < p.segment_count()));
        if self.selected_seam.is_some_and(|i| i >= self.pattern.seams.len()) {
            self.selected_seam = None;
        }
    }

    // ----------------------------------------------------------------- files

    fn new_pattern(&mut self) {
        self.checkpoint();
        self.pattern = Pattern::default();
        self.file = None;
        self.after_history_change();
        self.clear_selection();
    }

    fn open(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Pattern", &["json"]).pick_file() {
            self.load(path);
        }
    }

    fn load(&mut self, path: PathBuf) {
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Pattern>(&s).map_err(|e| e.to_string()));
        match result {
            Ok(p) => {
                self.checkpoint();
                self.pattern = p;
                self.after_history_change();
                self.status = format!("Opened {}", path.display());
                self.file = Some(path);
                self.fit_requested = true;
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    fn save(&mut self, save_as: bool) {
        let path = match (&self.file, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let Some(p) = rfd::FileDialog::new()
                    .add_filter("Pattern", &["json"])
                    .set_file_name("pattern.json")
                    .save_file()
                else {
                    return;
                };
                p
            }
        };
        let result = serde_json::to_string_pretty(&self.pattern)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    // --------------------------------------------------------------- editing

    fn set_tool(&mut self, tool: Tool) {
        if self.tool == Tool::Draw && tool != Tool::Draw {
            self.finish_drawing(false);
        }
        self.tool = tool;
        self.sew = SewState::Idle;
    }

    /// End the piece being drawn. Closes it if requested and it has 3+ points;
    /// discards it if it has fewer than 2 points.
    fn finish_drawing(&mut self, close: bool) {
        let Some(idx) = self.drawing.take() else { return };
        let piece = &mut self.pattern.pieces[idx];
        if piece.points.len() < 2 {
            self.pattern.remove_piece(idx);
            self.clear_selection();
            return;
        }
        if close && piece.points.len() >= 3 {
            piece.closed = true;
        }
    }

    fn delete_selection(&mut self) {
        if let Some(i) = self.selected_seam.take() {
            self.checkpoint();
            self.pattern.seams.remove(i);
        } else if !self.selected_points.is_empty() {
            self.checkpoint();
            // Highest piece, then highest point first, so earlier removals
            // don't shift the indices of later ones.
            let mut points = std::mem::take(&mut self.selected_points);
            points.sort_by_key(|r| std::cmp::Reverse((r.piece, r.point)));
            points.dedup();
            for r in points {
                if r.piece < self.pattern.pieces.len()
                    && r.point < self.pattern.pieces[r.piece].points.len()
                    && self.pattern.remove_point(r.piece, r.point)
                    && self.drawing == Some(r.piece)
                {
                    self.drawing = None;
                }
            }
            self.selected_segments.clear();
        } else if let Some(i) = self.selected_piece.take() {
            self.checkpoint();
            self.pattern.remove_piece(i);
            self.drawing = None;
            self.clear_selection();
        }
        self.validate_selection();
    }

    /// Toggle the selected points between turn and curve (all follow the first).
    fn toggle_curve(&mut self) {
        let Some(first) = self.selected_points.first() else { return };
        let kind = match self.pattern.pieces[first.piece].points[first.point].kind {
            PointKind::Turn => PointKind::Curve,
            PointKind::Curve => PointKind::Turn,
        };
        self.set_point_kind(kind);
    }

    fn set_point_kind(&mut self, kind: PointKind) {
        self.checkpoint();
        for r in &self.selected_points {
            self.pattern.pieces[r.piece].points[r.point].kind = kind;
        }
    }

    // ------------------------------------------------------------- selection

    fn clear_selection(&mut self) {
        self.selected_piece = None;
        self.selected_points.clear();
        self.selected_segments.clear();
        self.selected_seam = None;
        self.length_input.clear();
    }

    /// Select a point; with `additive` (Shift) toggle it in the selection instead.
    fn select_point(&mut self, r: PointRef, additive: bool) {
        if !additive {
            self.clear_selection();
        }
        self.length_input.clear();
        self.selected_seam = None;
        toggle_in(&mut self.selected_points, r, additive);
        self.selected_piece = Some(r.piece);
    }

    /// Select a segment; with `additive` (Shift) toggle it in the selection instead.
    fn select_segment(&mut self, pi: usize, seg: usize, additive: bool) {
        if !additive {
            self.clear_selection();
        }
        self.length_input.clear();
        self.selected_seam = None;
        toggle_in(&mut self.selected_segments, (pi, seg), additive);
        self.selected_piece = Some(pi);
    }

    fn single_point(&self) -> Option<PointRef> {
        match self.selected_points[..] {
            [r] => Some(r),
            _ => None,
        }
    }

    /// The one selected segment, if exactly one segment and no points are selected.
    fn single_segment(&self) -> Option<(usize, usize)> {
        match self.selected_segments[..] {
            [s] if self.selected_points.is_empty() => Some(s),
            _ => None,
        }
    }

    /// Whether typed digits currently feed `length_input`.
    fn length_input_active(&self) -> bool {
        match self.tool {
            Tool::Draw => self.drawing.is_some(),
            Tool::Select => self.single_segment().is_some(),
            _ => false,
        }
    }

    fn set_segment_length(&mut self, pi: usize, seg: usize, len: f64) {
        self.checkpoint();
        if !self.pattern.pieces[pi].set_segment_length(seg, len, self.length_moves_start) {
            self.undo.pop();
            self.status = "Can't set the length of a zero-length segment".into();
        }
    }

    /// Apply the typed length: place the next point while drawing, or resize
    /// the selected segment.
    fn apply_length_input(&mut self) {
        let Ok(len) = self.length_input.parse::<f64>() else {
            self.status = format!("Not a length: {}", self.length_input);
            self.length_input.clear();
            return;
        };
        self.length_input.clear();
        if len <= 0.0 {
            return;
        }
        if self.tool == Tool::Select {
            if let Some((pi, seg)) = self.single_segment() {
                self.set_segment_length(pi, seg, len);
            }
            return;
        }
        let Some(idx) = self.drawing else { return };
        let pts = &self.pattern.pieces[idx].points;
        let Some(last) = pts.last().map(|p| p.pos) else { return };
        // Head toward the cursor; fall back to continuing the previous segment, then +X.
        let toward = self.cursor_world.map(|c| c - last).filter(|d| d.length() > 1e-9);
        let previous = (pts.len() >= 2).then(|| last - pts[pts.len() - 2].pos);
        let dir = toward.or(previous).filter(|d| d.length() > 1e-9).unwrap_or(Vec2::new(1.0, 0.0));
        let pos = last + dir * (len / dir.length());
        self.checkpoint();
        let piece = &mut self.pattern.pieces[idx];
        piece.points.push(Point::turn(pos));
        self.selected_points = vec![PointRef { piece: idx, point: piece.points.len() - 1 }];
    }

    // ------------------------------------------------------ piece transforms

    fn rotate_selected(&mut self, degrees: f64) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        self.pattern.pieces[pi].rotate(degrees);
    }

    fn flip_selected(&mut self, horizontal: bool) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        self.pattern.pieces[pi].flip(horizontal);
    }

    fn duplicate_selected(&mut self) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        let copy = self.pattern.duplicate_piece(pi);
        self.clear_selection();
        self.selected_piece = Some(copy);
    }

    /// Piece and segments of the current segment selection, if all on one piece.
    fn fold_selection(&self) -> Option<(usize, Vec<usize>)> {
        let (pi, _) = *self.selected_segments.first()?;
        self.selected_segments
            .iter()
            .all(|&(p, _)| p == pi)
            .then(|| (pi, self.selected_segments.iter().map(|&(_, s)| s).collect()))
    }

    fn unfold_selected(&mut self) {
        let Some((pi, segs)) = self.fold_selection() else {
            self.status = "Select the straight edge(s) of one closed piece to unfold along \
                           (Shift+click for several)"
                .into();
            return;
        };
        self.checkpoint();
        match self.pattern.unfold(pi, &segs) {
            Ok(()) => {
                self.clear_selection();
                self.selected_piece = Some(pi);
                self.status = "Piece unfolded".into();
            }
            Err(e) => {
                self.undo.pop();
                self.status = e.into();
            }
        }
    }

    /// Cheap pre-check for enabling the Unfold command; `unfold` does the
    /// full validation (connected, collinear) and reports why it fails.
    fn can_unfold(&self) -> bool {
        self.fold_selection().is_some_and(|(pi, segs)| {
            let p = &self.pattern.pieces[pi];
            p.closed && segs.iter().all(|&s| !p.segment_is_curved(s))
        })
    }

    fn fit_view(&mut self, rect: Rect) {
        let Some((min, max)) = self.pattern.pieces.iter().filter_map(|p| p.bounds()).reduce(
            |(lo, hi), (a, b)| {
                (Vec2::new(lo.x.min(a.x), lo.y.min(a.y)), Vec2::new(hi.x.max(b.x), hi.y.max(b.y)))
            },
        ) else {
            self.view = View::default();
            return;
        };
        self.view.fit(rect, min, max);
    }

    // --------------------------------------------------------------- picking

    fn pick_point(&self, rect: Rect, screen: Pos2) -> Option<PointRef> {
        let mut best: Option<(PointRef, f32)> = None;
        for (pi, piece) in self.pattern.pieces.iter().enumerate() {
            for (i, p) in piece.points.iter().enumerate() {
                let d = self.view.to_screen(rect, p.pos).distance(screen);
                if d <= PICK_PX && best.is_none_or(|b| d < b.1) {
                    best = Some((PointRef { piece: pi, point: i }, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Closest outline location within pick range: `(piece, segment, t, pos)`.
    fn pick_outline(&self, rect: Rect, screen: Pos2) -> Option<(usize, usize, f64, Vec2)> {
        let world = self.view.to_world(rect, screen);
        let max = f64::from(PICK_PX) / self.view.zoom;
        self.pattern
            .pieces
            .iter()
            .enumerate()
            .filter_map(|(pi, p)| p.closest_on_outline(world).map(|c| (pi, c)))
            .filter(|(_, c)| c.3 <= max)
            .min_by(|a, b| a.1.3.total_cmp(&b.1.3))
            .map(|(pi, (seg, t, pos, _))| (pi, seg, t, pos))
    }

    /// Outline location for sewing: snaps to nearby points, else the closest
    /// outline position within pick range.
    fn pick_outline_pos(&self, rect: Rect, screen: Pos2) -> Option<(usize, OutlinePos)> {
        if let Some(r) = self.pick_point(rect, screen) {
            let piece = &self.pattern.pieces[r.piece];
            if piece.segment_count() > 0 {
                return Some((r.piece, piece.point_pos(r.point)));
            }
        }
        self.pick_outline(rect, screen).map(|(pi, seg, t, _)| (pi, OutlinePos::new(seg, t)))
    }

    /// Topmost piece whose outline is near `screen` or whose fill contains it.
    fn pick_piece(&self, rect: Rect, screen: Pos2) -> Option<usize> {
        if let Some((pi, ..)) = self.pick_outline(rect, screen) {
            return Some(pi);
        }
        let world = self.view.to_world(rect, screen);
        self.pattern.pieces.iter().rposition(|p| p.contains(world))
    }

    /// Round a displacement to the grid step when grid snapping is on.
    fn snap_offset(&self, d: Vec2) -> Vec2 {
        if self.snap_grid && self.snap_step > 0.0 {
            let s = self.snap_step;
            Vec2::new((d.x / s).round() * s, (d.y / s).round() * s)
        } else {
            d
        }
    }

    /// Move the selected piece by `d` (one undo step per call).
    /// Move the selected points by `d`, or the whole selected piece if no
    /// points are selected (one undo step per call).
    fn nudge_selected(&mut self, d: Vec2) {
        if !self.selected_points.is_empty() {
            self.checkpoint();
            for r in &self.selected_points {
                let p = &mut self.pattern.pieces[r.piece].points[r.point];
                p.pos = p.pos + d;
            }
            return;
        }
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        self.pattern.pieces[pi].translate(d);
    }

    /// Snap a world position to existing points (excluding `skip`) or the grid.
    fn snap(&self, rect: Rect, screen: Pos2, skip: Option<PointRef>) -> Vec2 {
        if self.snap_points
            && let Some(r) = self.pick_point(rect, screen)
            && Some(r) != skip
        {
            return self.pattern.pieces[r.piece].points[r.point].pos;
        }
        let w = self.view.to_world(rect, screen);
        if self.snap_grid && self.snap_step > 0.0 {
            let s = self.snap_step;
            Vec2::new((w.x / s).round() * s, (w.y / s).round() * s)
        } else {
            w
        }
    }

    // -------------------------------------------------------------- viewport

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;

        if std::mem::take(&mut self.fit_requested) {
            self.fit_view(rect);
        }

        // Navigation: scroll/pinch zooms at cursor, middle/right drag pans.
        if let Some(hover) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = f64::from(pinch) * (f64::from(scroll) * 0.002).exp();
            if factor != 1.0 {
                self.view.zoom_at(rect, hover, factor);
            }
        }
        if response.dragged_by(PointerButton::Middle) || response.dragged_by(PointerButton::Secondary)
        {
            let d = response.drag_delta();
            self.view.pan(d.x, d.y);
        }

        let hover = response.hover_pos();
        self.cursor_world = hover.map(|h| self.view.to_world(rect, h));

        match self.tool {
            Tool::Select => self.select_tool(&response, rect),
            Tool::Draw => self.draw_tool(&response, rect),
            Tool::AddPoint => self.add_point_tool(&response, rect),
            Tool::Sew => self.sew_tool(&response, rect),
        }

        self.paint(&painter, rect, hover);
    }

    fn select_tool(&mut self, response: &egui::Response, rect: Rect) {
        let shift = response.ctx.input(|i| i.modifiers.shift);
        if response.drag_started_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            if let Some(r) = self.pick_point(rect, pos) {
                // Dragging an unselected point selects it (Shift adds it) first.
                if !self.selected_points.contains(&r) {
                    if shift {
                        self.selected_points.push(r);
                        self.selected_piece = Some(r.piece);
                    } else {
                        self.select_point(r, false);
                    }
                }
                self.checkpoint();
                let pieces = &self.pattern.pieces;
                let original =
                    self.selected_points.iter().map(|&q| (q, pieces[q.piece].points[q.point].pos)).collect();
                self.dragging = Some(Drag::Points { grabbed: r, original });
            } else if let Some(pi) = self.pick_piece(rect, pos) {
                self.checkpoint();
                let original = self.pattern.pieces[pi].points.iter().map(|p| p.pos).collect();
                let start = self.view.to_world(rect, pos);
                self.dragging = Some(Drag::Piece { piece: pi, start, original });
                if self.selected_piece != Some(pi) {
                    self.clear_selection();
                    self.selected_piece = Some(pi);
                }
            }
        }
        if response.dragged_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            match &self.dragging {
                Some(Drag::Points { grabbed, original }) => {
                    let start = original.iter().find(|(q, _)| q == grabbed).map(|(_, p)| *p);
                    if let Some(start) = start {
                        let d = self.snap(rect, pos, Some(*grabbed)) - start;
                        for &(q, o) in original {
                            self.pattern.pieces[q.piece].points[q.point].pos = o + d;
                        }
                    }
                }
                Some(Drag::Piece { piece, start, original }) => {
                    let d = self.snap_offset(self.view.to_world(rect, pos) - *start);
                    for (p, o) in self.pattern.pieces[*piece].points.iter_mut().zip(original) {
                        p.pos = *o + d;
                    }
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }

        if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((pi, seg, t, _)) = self.pick_outline(rect, pos)
        {
            self.checkpoint();
            let idx = self.pattern.insert_point(pi, seg, t);
            self.select_point(PointRef { piece: pi, point: idx }, false);
        } else if response.clicked_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            if let Some(r) = self.pick_point(rect, pos) {
                self.select_point(r, shift);
            } else if let Some((pi, seg, ..)) = self.pick_outline(rect, pos) {
                self.select_segment(pi, seg, shift);
            } else if let Some(pi) = self.pick_piece(rect, pos) {
                if !shift {
                    self.clear_selection();
                }
                self.selected_piece = Some(pi);
            } else if !shift {
                self.clear_selection();
            }
        }
    }

    fn draw_tool(&mut self, response: &egui::Response, rect: Rect) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else { return };

        // Clicking the first point of the piece being drawn closes it.
        if let Some(idx) = self.drawing {
            let piece = &self.pattern.pieces[idx];
            let first = self.view.to_screen(rect, piece.points[0].pos);
            if piece.points.len() >= 3 && first.distance(pos) <= PICK_PX {
                self.checkpoint();
                self.finish_drawing(true);
                self.status = "Piece closed".into();
                return;
            }
        }

        let w = self.snap(rect, pos, None);
        self.checkpoint();
        let idx = *self.drawing.get_or_insert_with(|| {
            let n = self.pattern.pieces.len() + 1;
            self.pattern.pieces.push(Piece::new(format!("Piece {n}")));
            self.pattern.pieces.len() - 1
        });
        let piece = &mut self.pattern.pieces[idx];
        piece.points.push(Point::turn(w));
        let r = PointRef { piece: idx, point: piece.points.len() - 1 };
        self.select_point(r, false);
    }

    fn add_point_tool(&mut self, response: &egui::Response, rect: Rect) {
        if response.clicked_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((pi, seg, t, _)) = self.pick_outline(rect, pos)
        {
            self.checkpoint();
            let idx = self.pattern.insert_point(pi, seg, t);
            self.select_point(PointRef { piece: pi, point: idx }, false);
        }
    }

    fn sew_tool(&mut self, response: &egui::Response, rect: Rect) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else { return };
        let no_snap = response.ctx.input(|i| i.modifiers.alt);
        match self.sew {
            SewState::Idle | SewState::First(_) => {
                let Some((pi, at)) = self.pick_outline_pos(rect, pos) else { return };
                self.sew = match self.sew {
                    SewState::First(a) => SewState::SecondStart(a, pi, at),
                    _ => SewState::FirstStart(pi, at),
                };
            }
            SewState::FirstStart(start_piece, _) | SewState::SecondStart(_, start_piece, _) => {
                let Some((span, _)) = self.pending_span(rect, pos, no_snap) else {
                    if self.pick_outline_pos(rect, pos).is_some_and(|(pi, _)| pi != start_piece) {
                        self.status = "The end point must be on the same piece as the start".into();
                    }
                    return;
                };
                if let SewState::SecondStart(a, ..) = self.sew {
                    self.checkpoint();
                    self.pattern.seams.push(Seam { a, b: span });
                    self.selected_seam = Some(self.pattern.seams.len() - 1);
                    let [la, lb] = [a, span].map(|s| s.length(&self.pattern.pieces));
                    self.status = format!("Seam created: {la:.1} mm ↔ {lb:.1} mm (Δ {:.1} mm)", la - lb);
                    self.sew = SewState::Idle;
                } else {
                    self.sew = SewState::First(span);
                }
            }
        }
    }

    /// Ends on piece `piece` where an edge from `start` would be exactly as
    /// long as `a`, one per walking direction.
    fn matching_ends(&self, a: &EdgeSpan, piece: usize, start: OutlinePos) -> Vec<EdgeSpan> {
        let len = a.length(&self.pattern.pieces);
        let p = &self.pattern.pieces[piece];
        [true, false]
            .into_iter()
            .filter_map(|forward| {
                p.pos_at_distance(start, len, forward)
                    .map(|end| EdgeSpan { piece, start, end, forward })
            })
            .collect()
    }

    /// The edge span that clicking at `screen` would complete while sewing,
    /// and whether it snapped to the matching length.
    fn pending_span(&self, rect: Rect, screen: Pos2, no_snap: bool) -> Option<(EdgeSpan, bool)> {
        let (start_piece, start, first) = match self.sew {
            SewState::FirstStart(pi, s) => (pi, s, None),
            SewState::SecondStart(a, pi, s) => (pi, s, Some(a)),
            _ => return None,
        };
        let piece = &self.pattern.pieces[start_piece];
        if let Some(a) = first
            && self.snap_match
            && !no_snap
            && let Some((span, _)) = self
                .matching_ends(&a, start_piece, start)
                .into_iter()
                .map(|sp| (sp, self.view.to_screen(rect, piece.eval_pos(sp.end)).distance(screen)))
                .filter(|(_, d)| *d <= MATCH_SNAP_PX)
                .min_by(|x, y| x.1.total_cmp(&y.1))
        {
            return Some((span, true));
        }
        let (pi, at) = self.pick_outline_pos(rect, screen)?;
        if pi != start_piece || (piece.eval_pos(at) - piece.eval_pos(start)).length() < 1e-6 {
            return None;
        }
        Some((EdgeSpan::shortest(&self.pattern.pieces, pi, start, at), false))
    }

    fn sew_hint(&self) -> &'static str {
        match self.sew {
            SewState::Idle => Tool::Sew.hint(),
            SewState::FirstStart(..) => "Click the end of the first edge segment (Esc cancels)",
            SewState::First(_) => "Click the start of the matching edge segment",
            SewState::SecondStart(..) => {
                "Click the end of the matching edge segment. Diamonds mark the matching length \
                 (hold Alt to place freely)"
            }
        }
    }

    // -------------------------------------------------------------- painting

    fn paint(&self, painter: &egui::Painter, rect: Rect, hover: Option<Pos2>) {
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(250));
        self.paint_grid(&painter, rect);

        let to_screen = |p: Vec2| self.view.to_screen(rect, p);

        for (pi, piece) in self.pattern.pieces.iter().enumerate() {
            let selected = self.selected_piece == Some(pi);
            let color = if selected {
                Color32::from_rgb(20, 110, 220)
            } else {
                Color32::from_rgb(40, 40, 40)
            };
            let (poly, tris) = piece.fill_triangles();
            if !tris.is_empty() {
                let fill = fill_color(pi, selected);
                let mut mesh = egui::Mesh::default();
                for p in poly {
                    mesh.colored_vertex(to_screen(p), fill);
                }
                for [a, b, c] in tris {
                    mesh.add_triangle(a as u32, b as u32, c as u32);
                }
                painter.add(mesh);
            }

            let line: Vec<Pos2> = piece.flatten().into_iter().map(to_screen).collect();
            if line.len() >= 2 {
                painter.line(line, Stroke::new(if selected { 2.0 } else { 1.5 }, color));
            }

            for (i, p) in piece.points.iter().enumerate() {
                let s = to_screen(p.pos);
                let is_sel = self.selected_points.contains(&PointRef { piece: pi, point: i });
                let fill = if is_sel { Color32::from_rgb(240, 120, 0) } else { Color32::WHITE };
                match p.kind {
                    PointKind::Turn => {
                        let r = Rect::from_center_size(s, egui::vec2(7.0, 7.0));
                        painter.rect_filled(r, 0.0, fill);
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Middle);
                    }
                    PointKind::Curve => {
                        painter.circle_filled(s, 3.5, fill);
                        painter.circle_stroke(s, 3.5, Stroke::new(1.0, color));
                    }
                }
            }

            if let (Some(first), true) = (piece.points.first(), piece.closed || selected) {
                painter.text(
                    to_screen(first.pos) + egui::vec2(6.0, -6.0),
                    Align2::LEFT_BOTTOM,
                    &piece.name,
                    FontId::proportional(12.0),
                    color,
                );
            }
        }

        let orange = Color32::from_rgb(240, 120, 0);
        for &(pi, seg) in &self.selected_segments {
            let pts: Vec<Pos2> =
                self.pattern.pieces[pi].flatten_segment(seg).into_iter().map(to_screen).collect();
            painter.line(pts, Stroke::new(4.0, orange));
        }
        if let Some(&(pi, seg)) = self.selected_segments.last() {
            let piece = &self.pattern.pieces[pi];
            let mid = to_screen(piece.eval_segment(seg, 0.5));
            let text = if self.selected_segments.len() > 1 {
                let total: f64 = self
                    .selected_segments
                    .iter()
                    .map(|&(p, s)| self.pattern.pieces[p].segment_length(s))
                    .sum();
                format!("{} segments · {total:.1} mm", self.selected_segments.len())
            } else if self.length_input.is_empty() {
                format!("{:.1} mm", piece.segment_length(seg))
            } else {
                format!("{}_ mm  ⏎ to apply", self.length_input)
            };
            paint_label(&painter, mid + egui::vec2(12.0, -14.0), &text, Color32::from_rgb(180, 80, 0));
        }

        for (i, seam) in self.pattern.seams.iter().enumerate() {
            let selected = self.selected_seam == Some(i);
            self.paint_seam(&painter, rect, seam, seam_color(i), selected, true);
        }
        self.paint_sewing_preview(&painter, rect, hover);

        let Some(hover) = hover else { return };
        match self.tool {
            Tool::Sew => {}
            Tool::Draw => {
                let mut w = self.snap(rect, hover, None);
                let typed = self.length_input.parse::<f64>().ok().filter(|l| *l > 0.0);
                if let (Some(len), Some(idx)) = (typed, self.drawing)
                    && let Some(last) = self.pattern.pieces[idx].points.last()
                {
                    let cursor = self.view.to_world(rect, hover) - last.pos;
                    if cursor.length() > 1e-9 {
                        w = last.pos + cursor * (len / cursor.length());
                    }
                }
                let s = to_screen(w);
                if let Some(idx) = self.drawing
                    && let Some(last) = self.pattern.pieces[idx].points.last()
                {
                    let dashed = egui::Shape::dashed_line(
                        &[to_screen(last.pos), s],
                        Stroke::new(1.0, Color32::from_rgb(20, 110, 220)),
                        6.0,
                        4.0,
                    );
                    painter.extend(dashed);
                    let text = if self.length_input.is_empty() {
                        format!("{:.1} mm", (w - last.pos).length())
                    } else {
                        format!("{}_ mm  ⏎ to place", self.length_input)
                    };
                    paint_label(&painter, s + egui::vec2(12.0, 14.0), &text, Color32::DARK_GRAY);
                }
                painter.circle_stroke(s, 4.0, Stroke::new(1.0, Color32::from_rgb(240, 120, 0)));
            }
            Tool::AddPoint | Tool::Select => {
                if self.tool == Tool::Select && self.pick_point(rect, hover).is_some() {
                    return;
                }
                if let Some((_, _, _, pos)) = self.pick_outline(rect, hover) {
                    painter.circle_stroke(
                        to_screen(pos),
                        4.0,
                        Stroke::new(1.5, Color32::from_rgb(240, 120, 0)),
                    );
                }
            }
        }
    }

    /// Draw a span along its outline with a perpendicular notch at its start.
    /// Returns the span's screen polyline.
    fn paint_span(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        span: &EdgeSpan,
        color: Color32,
        width: f32,
    ) -> Vec<Pos2> {
        let pts: Vec<Pos2> = span
            .polyline(&self.pattern.pieces)
            .into_iter()
            .map(|p| self.view.to_screen(rect, p))
            .collect();
        if pts.len() < 2 {
            return pts;
        }
        painter.line(pts.clone(), Stroke::new(width, color));
        // Notch: perpendicular tick through the start, along the first non-degenerate direction.
        if let Some(dir) =
            pts.windows(2).map(|w| w[1] - w[0]).find(|d| d.length() > 0.5).map(|d| d.normalized())
        {
            let n = egui::vec2(-dir.y, dir.x) * 12.0;
            painter.line_segment([pts[0] - n, pts[0] + n], Stroke::new(width, color));
        }
        pts
    }

    /// Draw a seam: both spans, dotted connectors, and a warning badge when
    /// the lengths differ by more than the tolerance (if `badge`).
    fn paint_seam(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        seam: &Seam,
        color: Color32,
        selected: bool,
        badge: bool,
    ) {
        let width = if selected { 4.0 } else { 2.5 };
        let a = self.paint_span(painter, rect, &seam.a, color, width);
        let b = self.paint_span(painter, rect, &seam.b, color, width);
        if a.len() < 2 || b.len() < 2 {
            return;
        }
        // Screen-space polylines as Vec2 so `point_at_fraction` can be reused.
        let as_vec2 = |pts: &[Pos2]| -> Vec<Vec2> {
            pts.iter().map(|p| Vec2::new(f64::from(p.x), f64::from(p.y))).collect()
        };
        let (wa, wb) = (as_vec2(&a), as_vec2(&b));
        let to_pos = |v: Vec2| Pos2::new(v.x as f32, v.y as f32);
        // Dotted connectors between corresponding points: start, middle and end.
        for f in [0.0, 0.5, 1.0] {
            let line = [to_pos(point_at_fraction(&wa, f)), to_pos(point_at_fraction(&wb, f))];
            painter.extend(egui::Shape::dotted_line(&line, color, 6.0, 1.3));
        }

        let [la, lb] = seam.spans().map(|s| s.length(&self.pattern.pieces));
        if badge && (la - lb).abs() > self.seam_tolerance {
            let mid = to_pos(point_at_fraction(&wa, 0.5).lerp(point_at_fraction(&wb, 0.5), 0.5));
            painter.circle_filled(mid, 8.0, WARN_COLOR);
            painter.text(mid, Align2::CENTER_CENTER, "!", FontId::proportional(12.0), Color32::WHITE);
            paint_label(painter, mid + egui::vec2(12.0, 0.0), &format!("Δ {:.1} mm", la - lb), WARN_COLOR);
        }
    }

    fn paint_sewing_preview(&self, painter: &egui::Painter, rect: Rect, hover: Option<Pos2>) {
        if self.tool != Tool::Sew {
            return;
        }
        let pieces = &self.pattern.pieces;
        let color = seam_color(self.pattern.seams.len());
        let no_snap = painter.ctx().input(|i| i.modifiers.alt);
        let pending = hover.and_then(|h| self.pending_span(rect, h, no_snap));
        let marker = |piece: usize, at: OutlinePos| {
            let s = self.view.to_screen(rect, pieces[piece].eval_pos(at));
            painter.circle_filled(s, 4.0, color);
        };
        // Length readout next to the cursor.
        let mut readout: Option<(String, Color32)> = None;

        match self.sew {
            SewState::Idle => {}
            SewState::FirstStart(pi, start) => {
                marker(pi, start);
                if let Some((span, _)) = pending {
                    self.paint_span(painter, rect, &span, color.gamma_multiply(0.6), 2.5);
                    readout = Some((format!("A {:.1} mm", span.length(pieces)), Color32::DARK_GRAY));
                }
            }
            SewState::First(a) => {
                self.paint_span(painter, rect, &a, color, 2.5);
                readout = Some((format!("A {:.1} mm", a.length(pieces)), Color32::DARK_GRAY));
            }
            SewState::SecondStart(a, pi, start) => {
                self.paint_span(painter, rect, &a, color, 2.5);
                marker(pi, start);
                if self.snap_match && !no_snap {
                    for end in self.matching_ends(&a, pi, start) {
                        let s = self.view.to_screen(rect, pieces[pi].eval_pos(end.end));
                        let diamond = [(0.0, -7.0), (7.0, 0.0), (0.0, 7.0), (-7.0, 0.0)]
                            .map(|(x, y)| s + egui::vec2(x, y))
                            .to_vec();
                        painter.add(egui::Shape::convex_polygon(
                            diamond,
                            Color32::WHITE,
                            Stroke::new(2.0, OK_COLOR),
                        ));
                    }
                }
                let la = a.length(pieces);
                readout = Some((format!("A {la:.1} mm"), Color32::DARK_GRAY));
                if let Some((b, snapped)) = pending {
                    let seam = Seam { a, b };
                    self.paint_seam(painter, rect, &seam, color.gamma_multiply(0.6), false, false);
                    let lb = b.length(pieces);
                    let ok = (la - lb).abs() <= self.seam_tolerance;
                    readout = Some((
                        format!(
                            "A {la:.1} mm · B {lb:.1} mm · Δ {:.1}{}",
                            la - lb,
                            if snapped { "  (matched)" } else { "" }
                        ),
                        if ok { OK_COLOR } else { WARN_COLOR },
                    ));
                }
            }
        }

        let hovered = match pending {
            Some((span, _)) => Some((span.piece, span.end)),
            None => hover.and_then(|h| self.pick_outline_pos(rect, h)),
        };
        if let Some((pi, at)) = hovered {
            let s = self.view.to_screen(rect, pieces[pi].eval_pos(at));
            painter.circle_stroke(s, 5.0, Stroke::new(1.5, color));
        }
        if let (Some(h), Some((text, c))) = (hover, readout) {
            paint_label(painter, h + egui::vec2(14.0, 14.0), &text, c);
        }
    }

    fn paint_grid(&self, painter: &egui::Painter, rect: Rect) {
        let minor = self.view.grid_step(8.0);
        let major = minor * 10.0;
        let lo = self.view.to_world(rect, rect.left_bottom());
        let hi = self.view.to_world(rect, rect.right_top());

        for (step, color) in [(minor, Color32::from_gray(235)), (major, Color32::from_gray(210))] {
            let stroke = Stroke::new(1.0, color);
            let mut x = (lo.x / step).floor() * step;
            while x <= hi.x {
                let sx = self.view.to_screen(rect, Vec2::new(x, 0.0)).x;
                painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], stroke);
                x += step;
            }
            let mut y = (lo.y / step).floor() * step;
            while y <= hi.y {
                let sy = self.view.to_screen(rect, Vec2::new(0.0, y)).y;
                painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], stroke);
                y += step;
            }
        }

        let origin = self.view.to_screen(rect, Vec2::default());
        painter.line_segment(
            [Pos2::new(rect.left(), origin.y), Pos2::new(rect.right(), origin.y)],
            Stroke::new(1.0, Color32::from_rgb(230, 150, 150)),
        );
        painter.line_segment(
            [Pos2::new(origin.x, rect.top()), Pos2::new(origin.x, rect.bottom())],
            Stroke::new(1.0, Color32::from_rgb(150, 200, 150)),
        );
    }

    // ------------------------------------------------------------------- UI

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Don't steal keys from focused text fields.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.length_input_active() {
            self.handle_length_keys(ctx);
        }
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let (redo_shift, undo, redo_y, save, open, tool, del, curve, enter, esc, fit, snap) =
            ctx.input_mut(|i| {
                let tool = if i.key_pressed(Key::V) {
                    Some(Tool::Select)
                } else if i.key_pressed(Key::P) {
                    Some(Tool::Draw)
                } else if i.key_pressed(Key::A) {
                    Some(Tool::AddPoint)
                } else if i.key_pressed(Key::S) && !i.modifiers.command {
                    Some(Tool::Sew)
                } else {
                    None
                };
                // Ctrl+Shift+Z must be consumed before Ctrl+Z.
                (
                    i.consume_key(cmd_shift, Key::Z),
                    i.consume_key(cmd, Key::Z),
                    i.consume_key(cmd, Key::Y),
                    i.consume_key(cmd, Key::S),
                    i.consume_key(cmd, Key::O),
                    tool,
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::C),
                    i.key_pressed(Key::Enter),
                    i.key_pressed(Key::Escape),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::G),
                )
            });
        if undo {
            self.undo();
        }
        if redo_shift || redo_y {
            self.redo();
        }
        if save {
            self.save(false);
        }
        if open {
            self.open();
        }
        if let Some(t) = tool {
            self.set_tool(t);
        }
        if del {
            self.delete_selection();
        }
        if curve {
            self.toggle_curve();
        }
        if enter && self.drawing.is_some() {
            self.checkpoint();
            self.finish_drawing(true);
        }
        if esc {
            if !matches!(self.sew, SewState::Idle) {
                self.sew = SewState::Idle;
            } else if self.drawing.is_some() {
                self.finish_drawing(false);
            } else {
                self.clear_selection();
            }
        }
        if fit {
            self.fit_requested = true;
        }
        if self.tool == Tool::Select && self.dragging.is_none() {
            let (nudge, fine) = ctx.input(|i| {
                let mut d = Vec2::default();
                for (key, dx, dy) in [
                    (Key::ArrowLeft, -1.0, 0.0),
                    (Key::ArrowRight, 1.0, 0.0),
                    (Key::ArrowUp, 0.0, 1.0),
                    (Key::ArrowDown, 0.0, -1.0),
                ] {
                    if i.key_pressed(key) {
                        d = d + Vec2::new(dx, dy);
                    }
                }
                (d, i.modifiers.shift)
            });
            if nudge != Vec2::default() {
                // Arrows move one grid step (or 10 mm without snapping); Shift moves 1 mm.
                let step = if fine {
                    1.0
                } else if self.snap_grid {
                    self.snap_step
                } else {
                    10.0
                };
                self.nudge_selected(nudge * step);
            }
        }
        if snap {
            self.snap_grid = !self.snap_grid;
        }

        let (rotate, flip, unfold, duplicate) = ctx.input_mut(|i| {
            let plain = !i.modifiers.command && !i.modifiers.alt;
            (
                (plain && i.key_pressed(Key::R)).then_some(if i.modifiers.shift { 90.0 } else { -90.0 }),
                (plain && i.key_pressed(Key::H)).then_some(!i.modifiers.shift),
                plain && i.key_pressed(Key::U),
                i.consume_key(Modifiers::COMMAND, Key::D),
            )
        });
        if let Some(deg) = rotate {
            self.rotate_selected(deg);
        }
        if let Some(horizontal) = flip {
            self.flip_selected(horizontal);
        }
        if unfold {
            self.unfold_selected();
        }
        if duplicate {
            self.duplicate_selected();
        }
    }

    /// Typed digits build a length; Backspace edits it, Enter applies it and
    /// Esc discards it. Those keys are consumed only while a length is being
    /// typed, so they keep their usual meaning otherwise.
    fn handle_length_keys(&mut self, ctx: &egui::Context) {
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .flat_map(str::chars)
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect()
        });
        for c in typed.chars() {
            if c != '.' || !self.length_input.contains('.') {
                self.length_input.push(c);
            }
        }
        if self.length_input.is_empty() {
            return;
        }
        let none = Modifiers::NONE;
        let (back, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(none, Key::Backspace),
                i.consume_key(none, Key::Enter),
                i.consume_key(none, Key::Escape),
            )
        });
        if back {
            self.length_input.pop();
        }
        if esc {
            self.length_input.clear();
        }
        if enter {
            self.apply_length_input();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.new_pattern();
                    ui.close();
                }
                if ui.button("Open…  (Ctrl+O)").clicked() {
                    self.open();
                    ui.close();
                }
                if ui.button("Save  (Ctrl+S)").clicked() {
                    self.save(false);
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    self.save(true);
                    ui.close();
                }
                ui.separator();
                ui.add_enabled(false, egui::Button::new("Import DXF (AAMA/ASTM) — coming soon"));
                ui.add_enabled(false, egui::Button::new("Export DXF (AAMA/ASTM) — coming soon"));
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo  (Ctrl+Z)")).clicked() {
                    self.undo();
                    ui.close();
                }
                if ui
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo  (Ctrl+Shift+Z)"))
                    .clicked()
                {
                    self.redo();
                    ui.close();
                }
                ui.separator();
                if ui.button("Delete selection  (Del)").clicked() {
                    self.delete_selection();
                    ui.close();
                }
            });
            ui.menu_button("Piece", |ui| {
                let has = self.selected_piece.is_some();
                let item = |ui: &mut egui::Ui, enabled: bool, label: &str| {
                    let clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
                    if clicked {
                        ui.close();
                    }
                    clicked
                };
                if item(ui, has, "Rotate 90° clockwise  (R)") {
                    self.rotate_selected(-90.0);
                }
                if item(ui, has, "Rotate 90° counter-clockwise  (Shift+R)") {
                    self.rotate_selected(90.0);
                }
                if item(ui, has, "Flip horizontal  (H)") {
                    self.flip_selected(true);
                }
                if item(ui, has, "Flip vertical  (Shift+H)") {
                    self.flip_selected(false);
                }
                ui.separator();
                if item(ui, has, "Duplicate  (Ctrl+D)") {
                    self.duplicate_selected();
                }
                if item(ui, self.can_unfold(), "Unfold along selected edge  (U)") {
                    self.unfold_selected();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Fit to pattern  (F)").clicked() {
                    self.fit_requested = true;
                    ui.close();
                }
            });
        });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tools");
        for tool in [Tool::Select, Tool::Draw, Tool::AddPoint, Tool::Sew] {
            if ui.selectable_label(self.tool == tool, tool.label()).clicked() {
                self.set_tool(tool);
            }
        }

        ui.separator();
        ui.heading("Snapping");
        ui.checkbox(&mut self.snap_points, "Snap to points");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.snap_grid, "Grid (G)");
            ui.add_enabled(
                self.snap_grid,
                egui::DragValue::new(&mut self.snap_step).range(0.1..=1000.0).suffix(" mm"),
            );
        });

        ui.separator();
        ui.heading("Sewing");
        ui.checkbox(&mut self.snap_match, "Snap to matching length")
            .on_hover_text("While sewing the second edge, snap its end to where its length equals the first edge. Hold Alt to bypass.");
        ui.horizontal(|ui| {
            ui.label("Tolerance");
            ui.add(egui::DragValue::new(&mut self.seam_tolerance).range(0.0..=50.0).speed(0.1).suffix(" mm"))
                .on_hover_text("Seams whose edge lengths differ by more than this are flagged");
        });

        ui.separator();
        ui.heading("Pieces");
        let mut clicked = None;
        for (i, piece) in self.pattern.pieces.iter().enumerate() {
            let label = format!(
                "{}{}  ({} pts)",
                piece.name,
                if piece.closed { "" } else { " [open]" },
                piece.points.len()
            );
            if ui.selectable_label(self.selected_piece == Some(i), label).clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.clear_selection();
            self.selected_piece = Some(i);
        }

        self.seams_panel(ui);

        if let Some(pi) = self.selected_piece {
            ui.separator();
            let mut name = self.pattern.pieces[pi].name.clone();
            ui.horizontal(|ui| {
                ui.label("Name");
                let r = ui.text_edit_singleline(&mut name);
                if r.gained_focus() {
                    self.checkpoint();
                }
            });
            let piece = &mut self.pattern.pieces[pi];
            piece.name = name;
            if piece.closed {
                ui.label(format!("Area: {:.1} cm²", piece.area().abs() / 100.0));
            }
            ui.label(format!("Perimeter: {:.1} mm", piece.perimeter()));
            self.transform_panel(ui);
            self.segments_panel(ui, pi);
        }

        if self.selected_points.len() > 1 {
            ui.separator();
            ui.heading(format!("{} points", self.selected_points.len()));
            ui.horizontal(|ui| {
                ui.label("Type");
                if ui.button("Turn").clicked() {
                    self.set_point_kind(PointKind::Turn);
                }
                if ui.button("Curve").clicked() {
                    self.set_point_kind(PointKind::Curve);
                }
            });
        }

        if let Some(r) = self.single_point() {
            ui.separator();
            ui.heading(format!("Point {}", r.point + 1));
            let mut p = self.pattern.pieces[r.piece].points[r.point];
            let mut begin_edit = false;
            egui::Grid::new("point").num_columns(2).show(ui, |ui| {
                for (label, v) in [("X", &mut p.pos.x), ("Y", &mut p.pos.y)] {
                    ui.label(label);
                    let resp = ui.add(egui::DragValue::new(v).speed(0.5).suffix(" mm"));
                    begin_edit |= resp.drag_started() || resp.gained_focus();
                    ui.end_row();
                }
                ui.label("Type");
                ui.horizontal(|ui| {
                    begin_edit |= ui.radio_value(&mut p.kind, PointKind::Turn, "Turn").clicked();
                    begin_edit |= ui.radio_value(&mut p.kind, PointKind::Curve, "Curve").clicked();
                });
                ui.end_row();
            });
            if begin_edit {
                self.checkpoint();
            }
            self.pattern.pieces[r.piece].points[r.point] = p;
        }
    }

    fn transform_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("CW 90°").on_hover_text("Rotate 90° clockwise (R)").clicked() {
                self.rotate_selected(-90.0);
            }
            if ui.button("CCW 90°").on_hover_text("Rotate 90° counter-clockwise (Shift+R)").clicked() {
                self.rotate_selected(90.0);
            }
            if ui.button("Flip H").on_hover_text("Flip horizontal (H)").clicked() {
                self.flip_selected(true);
            }
            if ui.button("Flip V").on_hover_text("Flip vertical (Shift+H)").clicked() {
                self.flip_selected(false);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Rotate by").clicked() {
                self.rotate_selected(self.rotate_angle);
            }
            ui.add(egui::DragValue::new(&mut self.rotate_angle).range(-360.0..=360.0).suffix("°"))
                .on_hover_text("Counter-clockwise; negative is clockwise");
        });
        ui.horizontal(|ui| {
            if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                self.duplicate_selected();
            }
            if ui
                .add_enabled(self.can_unfold(), egui::Button::new("Unfold"))
                .on_hover_text("Mirror the piece across the selected straight edge and merge the halves (U)")
                .on_disabled_hover_text("Select a straight edge of a closed piece first")
                .clicked()
            {
                self.unfold_selected();
            }
        });
    }

    fn segments_panel(&mut self, ui: &mut egui::Ui, pi: usize) {
        let piece = &self.pattern.pieces[pi];
        let rows: Vec<(usize, usize, f64)> = (0..piece.segment_count())
            .map(|seg| {
                let (a, b) = piece.segment_ends(seg);
                (a, b, piece.segment_length(seg))
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        let additive = ui.input(|i| i.modifiers.shift);
        let mut select = None;
        let mut begin_edit = false;
        let mut change = None;
        egui::CollapsingHeader::new(format!("Segments ({})", rows.len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Edits move");
                    ui.radio_value(&mut self.length_moves_start, false, "end point");
                    ui.radio_value(&mut self.length_moves_start, true, "start point");
                });
                egui::Grid::new("segments").num_columns(2).striped(true).show(ui, |ui| {
                    for (seg, &(a, b, len)) in rows.iter().enumerate() {
                        let label = format!("{} → {}", a + 1, b + 1);
                        let selected = self.selected_segments.contains(&(pi, seg));
                        if ui.selectable_label(selected, label).clicked() {
                            select = Some(seg);
                        }
                        let mut v = len;
                        let resp = ui.add(
                            egui::DragValue::new(&mut v)
                                .speed(0.5)
                                .range(0.1..=100_000.0)
                                .max_decimals(1)
                                .suffix(" mm"),
                        );
                        begin_edit |= resp.drag_started() || resp.gained_focus();
                        if resp.changed() {
                            change = Some((seg, v));
                        }
                        ui.end_row();
                    }
                });
            });
        if let Some(seg) = select {
            self.select_segment(pi, seg, additive);
        }
        if begin_edit {
            self.checkpoint();
        }
        if let Some((seg, len)) = change {
            self.pattern.pieces[pi].set_segment_length(seg, len, self.length_moves_start);
        }
    }

    fn seams_panel(&mut self, ui: &mut egui::Ui) {
        if self.pattern.seams.is_empty() {
            return;
        }
        ui.separator();
        ui.heading("Seams");
        let pieces = &self.pattern.pieces;
        let mut clicked = None;
        for (i, seam) in self.pattern.seams.iter().enumerate() {
            let [la, lb] = seam.spans().map(|s| s.length(pieces));
            let mismatch = (la - lb).abs() > self.seam_tolerance;
            let text = egui::RichText::new(format!(
                "Seam {}: {} ↔ {}  Δ {:.1}{}",
                i + 1,
                pieces[seam.a.piece].name,
                pieces[seam.b.piece].name,
                la - lb,
                if mismatch { "  ⚠" } else { "" }
            ))
            .color(seam_color(i));
            let resp = ui.selectable_label(self.selected_seam == Some(i), text);
            if resp.clicked() {
                clicked = Some(i);
            }
            if mismatch {
                resp.on_hover_text(format!(
                    "Edge lengths differ by more than the {:.1} mm tolerance",
                    self.seam_tolerance
                ));
            }
        }
        if let Some(i) = clicked {
            self.clear_selection();
            self.selected_seam = Some(i);
        }

        let Some(i) = self.selected_seam else { return };
        let seam = self.pattern.seams[i];
        let [la, lb] = seam.spans().map(|s| s.length(&self.pattern.pieces));
        ui.label(format!("Edge A: {la:.1} mm   Edge B: {lb:.1} mm"));
        ui.horizontal(|ui| {
            if ui.button("Flip B").on_hover_text("Reverse direction of edge B").clicked() {
                self.checkpoint();
                self.pattern.seams[i].b = seam.b.reversed();
            }
            let b_closed = self.pattern.pieces[seam.b.piece].closed;
            if ui
                .add_enabled(b_closed, egui::Button::new("Other way"))
                .on_hover_text("Take the other way around piece B")
                .clicked()
            {
                self.checkpoint();
                self.pattern.seams[i].b.forward = !seam.b.forward;
            }
            if ui.button("Delete").clicked() {
                self.checkpoint();
                self.pattern.seams.remove(i);
                self.selected_seam = None;
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.cursor_world {
                Some(c) => ui.monospace(format!("x {:>8.1}  y {:>8.1} mm", c.x, c.y)),
                None => ui.monospace(format!("{:>27}", "")),
            };
            ui.separator();
            ui.monospace(format!("zoom {:.0}%", self.view.zoom * 100.0));
            ui.separator();
            if self.status.is_empty() {
                ui.label(if self.tool == Tool::Sew { self.sew_hint() } else { self.tool.hint() });
            } else {
                ui.label(&self.status);
            }
        });
    }
}

/// Solid fill for a closed piece; each piece gets its own pastel hue,
/// slightly darker when selected.
fn fill_color(piece: usize, selected: bool) -> Color32 {
    const PALETTE: [(u8, u8, u8); 6] = [
        (150, 190, 235),
        (240, 180, 150),
        (160, 215, 170),
        (225, 175, 220),
        (240, 215, 140),
        (170, 210, 215),
    ];
    let (r, g, b) = PALETTE[piece % PALETTE.len()];
    let c = Color32::from_rgb(r, g, b);
    if selected { c.gamma_multiply(0.85).to_opaque() } else { c }
}

/// Add `item` to `list`; with `toggle`, remove it instead if already present.
fn toggle_in<T: PartialEq>(list: &mut Vec<T>, item: T, toggle: bool) {
    match list.iter().position(|x| *x == item) {
        Some(i) if toggle => {
            list.remove(i);
        }
        Some(_) => {}
        None => list.push(item),
    }
}

/// Text on a translucent white plate, readable over the grid and fills.
fn paint_label(painter: &egui::Painter, pos: Pos2, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(12.0), color);
    let rect = Align2::LEFT_CENTER.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand(3.0), 3.0, Color32::from_white_alpha(220));
    painter.galley(rect.min, galley, color);
}

/// Distinct, saturated color per seam so pairs are easy to tell apart.
fn seam_color(seam: usize) -> Color32 {
    const PALETTE: [(u8, u8, u8); 6] = [
        (220, 40, 120),
        (0, 150, 110),
        (230, 110, 0),
        (110, 60, 210),
        (0, 120, 200),
        (170, 140, 0),
    ];
    let (r, g, b) = PALETTE[seam % PALETTE.len()];
    Color32::from_rgb(r, g, b)
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SESSION_KEY, &self.session());
    }

    /// Save often so a crash loses little work (the default is 30 s).
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.pointer.any_click()) {
            self.status.clear();
        }
        self.handle_shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("side").default_size(230.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui));
        });
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
    }
}

use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke,
};

use serde::{Deserialize, Serialize};

use crate::curves::HandleSide;
use crate::mcp::{self, McpServer};
use crate::dxf;
use crate::palette::{Entry, Palette};
use crate::settings::Settings;
use crate::theme;
use crate::units::{InchFormat, Lengths, Units};
use crate::model::{Isometry, Pattern, Piece, Point, PointKind, Vec2, triangulate};
use crate::sewing::{EdgeSpan, OutlinePos, Seam, point_at_fraction};
use crate::view::{Projection, View};
use crate::viewport::{self, Viewport};

/// Pick radius for points and lines, in screen pixels.
const PICK_PX: f32 = 8.0;
const UNDO_LIMIT: usize = 200;
/// Storage key for the persisted working session.
const SESSION_KEY: &str = "session";
/// Screen distance within which a sewing end snaps to the matching-length point.
const MATCH_SNAP_PX: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Select,
    Draw,
    AddPoint,
    Curve,
    Sew,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select (V)",
            Tool::Draw => "Draw piece (P)",
            Tool::AddPoint => "Add point (A)",
            Tool::Curve => "Edit curve (E)",
            Tool::Sew => "Free sewing (S)",
        }
    }

    /// Longer help, shown on the tool's button.
    fn help(self) -> &'static str {
        match self {
            Tool::Select => {
                "Click a point or edge to select (Shift+click for several), or drag a box on empty \
                 canvas (Ctrl+drag over pieces) to select points. Type a length + Enter \
                 to resize one segment. Drag to move (arrows nudge), drag handles to shape curves. \
                 Double-click an edge to add a point. R rotate, H flip, Ctrl+D duplicate, U unfold. \
                 Shift+U unfolds symmetric and Ctrl+Shift+D makes a mirrored copy: edits to one side \
                 are mirrored onto the other"
            }
            Tool::Draw => {
                "Click to place points, or type a length + Enter to place the next point toward the \
                 cursor. Click the first point or press Enter to close, Esc to stop"
            }
            Tool::AddPoint => "Click on an outline to insert a point",
            Tool::Curve => {
                "Drag an edge to bend it; double-click it to remove the curvature. Select points or \
                 edges to show handles; drag a handle to shape the curve (Alt breaks a smooth point)"
            }
            Tool::Sew => "Click a start point on an edge",
        }
    }
}

/// Everything runnable from the command palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Tool(Tool),
    ClosePiece,
    Undo,
    Redo,
    New,
    Open,
    Save,
    SaveAs,
    ExportDxf,
    Fit,
    SplitView,
    CloseView,
    ToggleGrid,
    ToggleSnapPoints,
    ToggleMatchSnap,
    ToggleLengths,
    ToggleAllowance,
    ToggleUnits,
    Settings,
    Delete,
    ToggleCurve,
    ClearHandles,
    Straighten,
    RotateCw,
    RotateCcw,
    FlipH,
    FlipV,
    Duplicate,
    DuplicateMirrored,
    Unfold,
    UnfoldSymmetric,
    Unlink,
    AddAllowance,
    RemoveAllowance,
}

/// Palette rows: command, label, shortcut. Tools first, since activating a
/// tool is the most common use.
const COMMANDS: &[(Command, &str, &str)] = &[
    (Command::Tool(Tool::Select), "Select tool", "V"),
    (Command::Tool(Tool::Draw), "Draw piece tool", "P"),
    (Command::Tool(Tool::AddPoint), "Add point tool", "A"),
    (Command::Tool(Tool::Curve), "Edit curve tool", "E"),
    (Command::Tool(Tool::Sew), "Free sewing tool", "S"),
    (Command::ClosePiece, "Close piece being drawn", "Enter"),
    (Command::Undo, "Undo", "Ctrl+Z"),
    (Command::Redo, "Redo", "Ctrl+Shift+Z"),
    (Command::New, "New pattern", ""),
    (Command::Open, "Open pattern…", "Ctrl+O"),
    (Command::Save, "Save pattern", "Ctrl+S"),
    (Command::SaveAs, "Save pattern as…", ""),
    (Command::ExportDxf, "Export DXF (AAMA/ASTM)…", ""),
    (Command::Fit, "Fit view to pattern", "F"),
    (Command::SplitView, "Split view", ""),
    (Command::CloseView, "Close view", ""),
    (Command::ToggleGrid, "Toggle grid snapping", "G"),
    (Command::ToggleSnapPoints, "Toggle snap to points", ""),
    (Command::ToggleMatchSnap, "Toggle snap to matching seam length", ""),
    (Command::ToggleLengths, "Toggle segment length labels", "L"),
    (Command::ToggleAllowance, "Toggle seam allowance display", ""),
    (Command::ToggleUnits, "Toggle units (mm / inches)", ""),
    (Command::Settings, "Settings…", ""),
    (Command::Delete, "Delete selection", "Del"),
    (Command::ToggleCurve, "Toggle turn / curve point", "C"),
    (Command::ClearHandles, "Clear point handles", ""),
    (Command::Straighten, "Remove curvature from selected segments", "Shift+C"),
    (Command::RotateCw, "Rotate piece 90° clockwise", "R"),
    (Command::RotateCcw, "Rotate piece 90° counter-clockwise", "Shift+R"),
    (Command::FlipH, "Flip piece horizontally", "H"),
    (Command::FlipV, "Flip piece vertically", "Shift+H"),
    (Command::Duplicate, "Duplicate piece", "Ctrl+D"),
    (Command::DuplicateMirrored, "Duplicate piece mirrored (linked symmetric)", "Ctrl+Shift+D"),
    (Command::Unfold, "Unfold piece along selected edges", "U"),
    (Command::UnfoldSymmetric, "Unfold piece symmetric (linked halves)", "Shift+U"),
    (Command::Unlink, "Unlink symmetric piece", ""),
    (Command::AddAllowance, "Add seam allowance around piece", ""),
    (Command::RemoveAllowance, "Remove seam allowance from piece", ""),
];

/// An in-progress drag with the Select tool.
enum Drag {
    /// Selected points, moved together by the grabbed point's (snapped) offset.
    Points { grabbed: PointRef, original: Vec<(PointRef, Vec2)> },
    /// Whole piece, `moved` so far from where the drag started; grid snapping
    /// applies to the total displacement rather than accumulating error.
    Piece { piece: usize, start: Vec2, moved: Vec2 },
    /// One Bézier handle of a point.
    Handle { point: PointRef, side: HandleSide },
    /// Bending a segment: the curve point at `t` follows the cursor, starting
    /// from the segment's handles when the drag began.
    Bend { piece: usize, seg: usize, t: f64, grab: Vec2, start: (Vec2, Vec2) },
    /// Rectangle selection between two screen positions of `viewport`;
    /// `additive` (Shift) adds to the current selection instead of replacing it.
    Marquee { viewport: usize, start: Pos2, end: Pos2, additive: bool },
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
    viewports: Vec<Viewport>,
    active_viewport: usize,
    /// Camera of sessions saved before there were several viewports.
    #[serde(skip_serializing, deserialize_with = "legacy_view")]
    view: Option<View>,
}

/// Old sessions stored the camera as a plain `view: (...)`, not an `Option`.
fn legacy_view<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<View>, D::Error> {
    View::deserialize(d).map(Some)
}

impl Default for Session {
    fn default() -> Self {
        App::default().session()
    }
}

pub struct App {
    pattern: Pattern,
    /// Windows onto the pattern, laid out side by side; never empty.
    viewports: Vec<Viewport>,
    /// The viewport last hovered: keyboard view commands apply to it.
    active_viewport: usize,
    tool: Tool,
    selected_piece: Option<usize>,
    /// Selected points; Shift+click adds or removes.
    selected_points: Vec<PointRef>,
    selected_seam: Option<usize>,
    /// Selected outline segments `(piece, seg)`; Shift+click adds or removes.
    selected_segments: Vec<(usize, usize)>,
    /// Digits typed for a precise length (draw tool or selected segment).
    length_input: String,
    /// Angle (degrees, counter-clockwise) for the "Rotate by" command.
    rotate_angle: f64,
    sew: SewState,
    /// Piece currently being drawn with the Draw tool.
    drawing: Option<usize>,
    /// Point or piece being dragged with the Select tool.
    dragging: Option<Drag>,
    undo: Vec<Pattern>,
    redo: Vec<Pattern>,
    /// Application preferences, saved apart from the session.
    settings: Settings,
    /// Settings as last written to disk, to save only on change.
    saved_settings: Settings,
    show_settings: bool,
    /// Edge under the last right-click `(piece, seg, t)`, for the context menu.
    context_target: Option<(usize, usize, f64)>,
    file: Option<PathBuf>,
    status: String,
    cursor_world: Option<Vec2>,
    palette: Palette,
    /// MCP server for AI agents, while enabled in the settings.
    mcp: Option<McpServer>,
    /// Why the MCP server isn't running, if starting it failed.
    mcp_error: Option<String>,
    /// `(enabled, port)` the MCP server was last started or stopped for.
    mcp_applied: Option<(bool, u16)>,
    /// For starting the MCP server, which wakes the UI on requests.
    egui_ctx: Option<egui::Context>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            pattern: Pattern::default(),
            viewports: vec![Viewport::default()],
            active_viewport: 0,
            tool: Tool::Draw,
            selected_piece: None,
            selected_points: Vec::new(),
            selected_seam: None,
            selected_segments: Vec::new(),
            length_input: String::new(),
            rotate_angle: 15.0,
            sew: SewState::Idle,
            drawing: None,
            dragging: None,
            undo: Vec::new(),
            redo: Vec::new(),
            settings: Settings::default(),
            saved_settings: Settings::default(),
            show_settings: false,
            context_target: None,
            file: None,
            status: String::new(),
            cursor_world: None,
            palette: Palette::default(),
            mcp: None,
            mcp_error: None,
            mcp_applied: None,
            egui_ctx: None,
        }
    }
}

impl App {
    /// Create the app, restoring the previous session from `storage`. A
    /// pattern file given on the command line takes precedence.
    pub fn new(ctx: &egui::Context, storage: Option<&dyn eframe::Storage>, path: Option<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self { egui_ctx: Some(ctx.clone()), ..Self::default() };
        let (settings, error) = Settings::load();
        app.saved_settings = settings.clone();
        app.settings = settings;
        if let Some(e) = error {
            app.status = e;
        }
        if let Some(session) = storage.and_then(|s| eframe::get_value::<Session>(s, SESSION_KEY)) {
            app.restore(session);
        }
        if let Some(path) = path {
            app.load(path);
            // Loading pushes an undo step; a fresh launch shouldn't start with one.
            app.undo.clear();
        }
        app.sync_mcp_server();
        app
    }

    fn session(&self) -> Session {
        Session {
            pattern: self.pattern.clone(),
            file: self.file.clone(),
            viewports: self.viewports.clone(),
            active_viewport: self.active_viewport,
            view: None,
        }
    }

    fn restore(&mut self, s: Session) {
        self.pattern = s.pattern;
        self.file = s.file;
        // Only old sessions have `view`; their `viewports` is just the default.
        self.viewports = match (s.view, s.viewports) {
            (Some(view), _) => vec![Viewport::pattern(view)],
            (None, v) if !v.is_empty() => v,
            _ => vec![Viewport::default()],
        };
        self.active_viewport = s.active_viewport.min(self.viewports.len() - 1);
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
                self.fit_all();
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

    fn export_dxf(&mut self) {
        let stem = self.file.as_ref().and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().into_owned());
        let style = stem.as_deref().unwrap_or("pattern");
        let Some(path) = rfd::FileDialog::new()
            .add_filter("DXF (AAMA/ASTM)", &["dxf"])
            .set_file_name(format!("{style}.dxf"))
            .save_file()
        else {
            return;
        };
        let export = dxf::export_aama(&self.pattern, style);
        let exported = self.pattern.pieces.len() - export.skipped.len();
        self.status = match std::fs::write(&path, export.text) {
            Ok(()) if export.skipped.is_empty() => {
                format!("Exported {exported} piece(s) to {}", path.display())
            }
            Ok(()) => format!(
                "Exported {exported} piece(s) to {}; skipped open piece(s): {}",
                path.display(),
                export.skipped.join(", ")
            ),
            Err(e) => format!("Export failed: {e}"),
        };
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
            // Mirror images on linked symmetric pieces go too.
            let points: Vec<(usize, usize)> = self.selected_points.drain(..).map(|r| (r.piece, r.point)).collect();
            let removed = self.pattern.remove_points(&points);
            // Removing a piece shifts the indices after it.
            if self.drawing.is_some_and(|d| removed.iter().any(|&p| p <= d)) {
                self.drawing = None;
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
            self.pattern.pieces[r.piece].set_point_kind(r.point, kind);
        }
    }

    fn clear_selected_handles(&mut self) {
        self.checkpoint();
        for r in &self.selected_points {
            self.pattern.pieces[r.piece].clear_handles(r.point);
        }
    }

    /// Remove the curvature of the selected segments.
    fn straighten_selected(&mut self) {
        if self.selected_segments.is_empty() {
            self.status = "Select the segment(s) to remove curvature from".into();
            return;
        }
        self.checkpoint();
        for &(pi, seg) in &self.selected_segments {
            self.pattern.pieces[pi].straighten_segment(seg);
        }
    }

    /// Points whose handles are shown and draggable: selected points and the
    /// end points of selected segments.
    fn handle_points(&self) -> Vec<PointRef> {
        let mut out = self.selected_points.clone();
        for &(pi, seg) in &self.selected_segments {
            let (a, b) = self.pattern.pieces[pi].segment_ends(seg);
            for point in [a, b] {
                let r = PointRef { piece: pi, point };
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        // Mirror images on linked symmetric pieces show their handles too.
        for r in out.clone() {
            for (piece, point) in self.pattern.mirror_images(r.piece, r.point) {
                let m = PointRef { piece, point };
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Visible handle under `screen`, if any.
    fn pick_handle(&self, proj: Projection, screen: Pos2) -> Option<(PointRef, HandleSide)> {
        self.handle_points().into_iter().find_map(|r| {
            let piece = &self.pattern.pieces[r.piece];
            let pos = piece.points[r.point].pos;
            let (hin, hout) = piece.visible_handles(r.point);
            [(hout, HandleSide::Out), (hin, HandleSide::In)].into_iter().find_map(|(h, side)| {
                let h = h?;
                (proj.to_screen(pos + h).distance(screen) <= PICK_PX).then_some((r, side))
            })
        })
    }

    /// The point an edit in progress centers on. Where both sides of a
    /// symmetry link changed, its side wins.
    fn mirror_focus(&self) -> Option<(usize, usize)> {
        let r = match &self.dragging {
            Some(Drag::Points { grabbed, .. }) => Some(*grabbed),
            Some(Drag::Handle { point, .. }) => Some(*point),
            _ => self.selected_points.first().copied(),
        }?;
        Some((r.piece, r.point))
    }

    /// Repeat the edits made since `before` on linked symmetric pieces and
    /// halves. Edits are made to one side only and mirrored here, once per
    /// frame before painting, so the other side follows live while dragging.
    /// Returns whether anything changed.
    fn mirror_edits(&mut self, before: &Pattern) -> bool {
        let focus = self.mirror_focus();
        self.pattern.mirror_changes(before, focus)
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

    /// Select the points inside screen rectangle `area` (added to the
    /// selection when `additive`).
    fn select_in_rect(&mut self, proj: Projection, area: Rect, additive: bool) {
        if !additive {
            self.clear_selection();
        }
        self.length_input.clear();
        self.selected_seam = None;
        for (pi, piece) in self.pattern.pieces.iter().enumerate() {
            for (i, p) in piece.points.iter().enumerate() {
                let r = PointRef { piece: pi, point: i };
                if area.contains(proj.to_screen(p.pos)) && !self.selected_points.contains(&r) {
                    self.selected_points.push(r);
                }
            }
        }
        if let Some(r) = self.selected_points.last() {
            self.selected_piece = Some(r.piece);
        }
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
        if !self.pattern.pieces[pi].set_segment_length(seg, len, self.settings.length_moves_start) {
            self.undo.pop();
            self.status = "Can't set the length of a zero-length segment".into();
        }
    }

    /// Apply the typed length: place the next point while drawing, or resize
    /// the selected segment.
    fn apply_length_input(&mut self) {
        let Some(len) = self.settings.units.parse(&self.length_input) else {
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

    // Whole-piece moves go through the pattern so symmetry links move along.

    fn rotate_selected(&mut self, degrees: f64) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        let iso = self.pattern.pieces[pi].rotation(degrees);
        self.pattern.transform_piece(pi, &iso);
    }

    fn flip_selected(&mut self, horizontal: bool) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        let iso = self.pattern.pieces[pi].flipping(horizontal);
        self.pattern.transform_piece(pi, &iso);
    }

    /// Copy the selected piece; `mirrored` flips the copy and links the two
    /// so edits to either are mirrored onto the other.
    fn duplicate_selected(&mut self, mirrored: bool) {
        let Some(pi) = self.selected_piece else { return };
        self.checkpoint();
        let copy = if mirrored { self.pattern.duplicate_mirrored(pi) } else { self.pattern.duplicate_piece(pi) };
        self.clear_selection();
        self.selected_piece = Some(copy);
        if mirrored {
            self.status = "Mirrored copy linked: edits to either piece are mirrored onto the other".into();
        }
    }

    fn unlink_selected(&mut self) {
        let Some(pi) = self.selected_piece.filter(|&pi| self.pattern.is_linked(pi)) else { return };
        self.checkpoint();
        self.pattern.unlink(pi);
        self.status = "Symmetry unlinked: the sides are now edited independently".into();
    }

    /// Piece and segments of the current segment selection, if all on one piece.
    fn fold_selection(&self) -> Option<(usize, Vec<usize>)> {
        let (pi, _) = *self.selected_segments.first()?;
        self.selected_segments
            .iter()
            .all(|&(p, _)| p == pi)
            .then(|| (pi, self.selected_segments.iter().map(|&(_, s)| s).collect()))
    }

    /// Unfold along the selected edges; `symmetric` keeps the halves linked.
    fn unfold_selected(&mut self, symmetric: bool) {
        let Some((pi, segs)) = self.fold_selection() else {
            self.status = "Select the straight edge(s) of one closed piece to unfold along \
                           (Shift+click for several)"
                .into();
            return;
        };
        self.checkpoint();
        let result =
            if symmetric { self.pattern.unfold_symmetric(pi, &segs) } else { self.pattern.unfold(pi, &segs) };
        match result {
            Ok(()) => {
                self.clear_selection();
                self.selected_piece = Some(pi);
                self.status = if symmetric {
                    "Piece unfolded symmetric: edits to one half are mirrored onto the other".into()
                } else {
                    "Piece unfolded".into()
                };
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

    /// Frame the whole pattern in viewport `vi`, laid out in `rect`.
    fn fit_view(&mut self, vi: usize, rect: Rect) {
        let Some((min, max)) = self.pattern.pieces.iter().filter_map(|p| p.outer_bounds()).reduce(
            |(lo, hi), (a, b)| {
                (Vec2::new(lo.x.min(a.x), lo.y.min(a.y)), Vec2::new(hi.x.max(b.x), hi.y.max(b.y)))
            },
        ) else {
            if let Some(view) = self.viewports[vi].camera_mut() {
                *view = View::default();
            }
            return;
        };
        if let Some(view) = self.viewports[vi].camera_mut() {
            view.fit(rect, min, max);
        }
    }

    fn fit_active(&mut self) {
        self.viewports[self.active_viewport].fit_requested = true;
    }

    fn fit_all(&mut self) {
        for v in &mut self.viewports {
            v.fit_requested = true;
        }
    }

    /// Open a second window onto the pattern beside the active one, starting
    /// from the same camera.
    fn split_view(&mut self) {
        let copy = self.viewports[self.active_viewport].clone();
        self.viewports.insert(self.active_viewport + 1, copy);
        self.active_viewport += 1;
    }

    /// Close the active viewport, keeping at least one.
    fn close_view(&mut self) {
        if self.viewports.len() <= 1 {
            return;
        }
        let vi = self.active_viewport;
        self.viewports.remove(vi);
        if let Some(Drag::Marquee { viewport, .. }) = &mut self.dragging {
            if *viewport == vi {
                self.dragging = None;
            } else if *viewport > vi {
                *viewport -= 1;
            }
        }
        self.active_viewport = vi.min(self.viewports.len() - 1);
    }

    // --------------------------------------------------------------- picking

    fn pick_point(&self, proj: Projection, screen: Pos2) -> Option<PointRef> {
        let mut best: Option<(PointRef, f32)> = None;
        for (pi, piece) in self.pattern.pieces.iter().enumerate() {
            for (i, p) in piece.points.iter().enumerate() {
                let d = proj.to_screen(p.pos).distance(screen);
                if d <= PICK_PX && best.is_none_or(|b| d < b.1) {
                    best = Some((PointRef { piece: pi, point: i }, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Closest outline location within pick range: `(piece, segment, t, pos)`.
    fn pick_outline(&self, proj: Projection, screen: Pos2) -> Option<(usize, usize, f64, Vec2)> {
        let world = proj.to_world(screen);
        let max = f64::from(PICK_PX) / proj.zoom();
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
    fn pick_outline_pos(&self, proj: Projection, screen: Pos2) -> Option<(usize, OutlinePos)> {
        if let Some(r) = self.pick_point(proj, screen) {
            let piece = &self.pattern.pieces[r.piece];
            if piece.segment_count() > 0 {
                return Some((r.piece, piece.point_pos(r.point)));
            }
        }
        self.pick_outline(proj, screen).map(|(pi, seg, t, _)| (pi, OutlinePos::new(seg, t)))
    }

    /// Topmost piece whose outline is near `screen` or whose fill contains it.
    fn pick_piece(&self, proj: Projection, screen: Pos2) -> Option<usize> {
        if let Some((pi, ..)) = self.pick_outline(proj, screen) {
            return Some(pi);
        }
        let world = proj.to_world(screen);
        self.pattern.pieces.iter().rposition(|p| p.contains(world))
    }

    /// Round a displacement to the grid step when grid snapping is on.
    fn snap_offset(&self, d: Vec2) -> Vec2 {
        if self.settings.snap_grid && self.settings.snap_step > 0.0 {
            let s = self.settings.snap_step;
            Vec2::new((d.x / s).round() * s, (d.y / s).round() * s)
        } else {
            d
        }
    }

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
        self.pattern.transform_piece(pi, &Isometry::translation(d));
    }

    /// Snap a world position to existing points (excluding `skip`) or the grid.
    fn snap(&self, proj: Projection, screen: Pos2, skip: Option<PointRef>) -> Vec2 {
        if self.settings.snap_points
            && let Some(r) = self.pick_point(proj, screen)
            && Some(r) != skip
        {
            return self.pattern.pieces[r.piece].points[r.point].pos;
        }
        let w = proj.to_world(screen);
        if self.settings.snap_grid && self.settings.snap_step > 0.0 {
            let s = self.settings.snap_step;
            Vec2::new((w.x / s).round() * s, (w.y / s).round() * s)
        } else {
            w
        }
    }

    // -------------------------------------------------------------- viewport

    /// The canvas area: lays the viewports out side by side, lets each
    /// handle input, then paints them all. `frame_start` is the pattern at
    /// the start of the frame, for mirroring this frame's edits before painting.
    fn viewports(&mut self, ui: &mut egui::Ui, frame_start: &Pattern) {
        let area = ui.available_rect_before_wrap();
        let rects = viewport::layout(area, self.viewports.len());
        self.cursor_world = None;
        let mut frames = Vec::new();
        for (vi, rect) in rects.into_iter().enumerate() {
            let response = ui.interact(rect, ui.id().with(("viewport", vi)), Sense::click_and_drag());
            if let Some(proj) = self.viewport_input(vi, &response) {
                frames.push((vi, proj, response.hover_pos()));
            }
        }
        self.mirror_edits(frame_start);
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::CUT_LINE);
        for (vi, proj, hover) in frames {
            self.paint(vi, &painter, proj, hover);
        }
    }

    /// Navigation and tool input for viewport `vi`. Returns its projection
    /// for painting, if it shows the flat pattern.
    fn viewport_input(&mut self, vi: usize, response: &egui::Response) -> Option<Projection> {
        let rect = response.rect;
        if response.hovered() {
            self.active_viewport = vi;
        }
        if std::mem::take(&mut self.viewports[vi].fit_requested) {
            self.fit_view(vi, rect);
        }
        let view = self.viewports[vi].camera_mut()?;

        // Navigation: scroll/pinch zooms at cursor, middle/right drag pans.
        if let Some(hover) = response.hover_pos() {
            let (scroll, pinch) = response.ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = f64::from(pinch) * (f64::from(scroll) * 0.002).exp();
            if factor != 1.0 {
                view.zoom_at(rect, hover, factor);
            }
        }
        if response.dragged_by(PointerButton::Middle) || response.dragged_by(PointerButton::Secondary)
        {
            let d = response.drag_delta();
            view.pan(d.x, d.y);
        }
        let proj = Projection { view: *view, rect };

        if let Some(hover) = response.hover_pos() {
            self.cursor_world = Some(proj.to_world(hover));
        }

        match self.tool {
            Tool::Select | Tool::Curve => self.select_tool(vi, response, proj),
            Tool::Draw => self.draw_tool(response, proj),
            Tool::AddPoint => self.add_point_tool(response, proj),
            Tool::Sew => self.sew_tool(response, proj),
        }

        if response.secondary_clicked() {
            self.context_target = response
                .interact_pointer_pos()
                .and_then(|pos| self.pick_outline(proj, pos))
                .map(|(pi, seg, t, _)| (pi, seg, t));
        }
        response.context_menu(|ui| self.viewport_context_menu(ui));
        Some(proj)
    }

    /// Right-click menu: segment actions when an edge was clicked.
    fn viewport_context_menu(&mut self, ui: &mut egui::Ui) {
        if let Some((pi, seg, t)) = self.context_target
            && pi < self.pattern.pieces.len()
            && seg < self.pattern.pieces[pi].segment_count()
        {
            let curved = self.pattern.pieces[pi].segment_is_curved(seg);
            if ui.add_enabled(curved, egui::Button::new("Remove curvature")).clicked() {
                self.checkpoint();
                self.pattern.pieces[pi].straighten_segment(seg);
                self.select_segment(pi, seg, false);
                ui.close();
            }
            if ui.button("Insert point here").clicked() {
                self.checkpoint();
                let idx = self.pattern.insert_point(pi, seg, t);
                self.select_point(PointRef { piece: pi, point: idx }, false);
                ui.close();
            }
            if ui.button("Select segment").clicked() {
                self.select_segment(pi, seg, false);
                ui.close();
            }
            ui.separator();
        }
        if ui.button("Fit to pattern").clicked() {
            self.fit_active();
            ui.close();
        }
        if ui.button("Command palette…").clicked() {
            self.palette.open();
            ui.close();
        }
    }

    fn select_tool(&mut self, vi: usize, response: &egui::Response, proj: Projection) {
        let (shift, alt, ctrl) =
            response.ctx.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        if response.drag_started_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            // Ctrl forces a selection rectangle even when starting over a piece.
            if ctrl {
                self.dragging = Some(Drag::Marquee { viewport: vi, start: pos, end: pos, additive: shift });
            } else if let Some((point, side)) = self.pick_handle(proj, pos) {
                self.checkpoint();
                self.dragging = Some(Drag::Handle { point, side });
            } else if let Some(r) = self.pick_point(proj, pos) {
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
            } else if self.tool == Tool::Curve
                && let Some((pi, seg, t, _)) = self.pick_outline(proj, pos)
            {
                self.checkpoint();
                self.select_segment(pi, seg, false);
                let start = self.pattern.pieces[pi].segment_handles(seg);
                let grab = proj.to_world(pos);
                self.dragging = Some(Drag::Bend { piece: pi, seg, t, grab, start });
            } else if self.tool == Tool::Select
                && let Some(pi) = self.pick_piece(proj, pos)
            {
                self.checkpoint();
                let start = proj.to_world(pos);
                self.dragging = Some(Drag::Piece { piece: pi, start, moved: Vec2::default() });
                if self.selected_piece != Some(pi) {
                    self.clear_selection();
                    self.selected_piece = Some(pi);
                }
            } else if self.pick_outline(proj, pos).is_none() {
                // Empty canvas (or a piece's fill in the curve tool): rectangle selection.
                self.dragging = Some(Drag::Marquee { viewport: vi, start: pos, end: pos, additive: shift });
            }
        }
        if response.dragged_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            if let Some(Drag::Marquee { end, .. }) = &mut self.dragging {
                *end = pos;
            }
            match &self.dragging {
                Some(Drag::Marquee { .. }) => {}
                Some(Drag::Points { grabbed, original }) => {
                    let start = original.iter().find(|(q, _)| q == grabbed).map(|(_, p)| *p);
                    if let Some(start) = start {
                        let d = self.snap(proj, pos, Some(*grabbed)) - start;
                        for &(q, o) in original {
                            self.pattern.pieces[q.piece].points[q.point].pos = o + d;
                        }
                    }
                }
                &Some(Drag::Piece { piece, start, moved }) => {
                    let d = self.snap_offset(proj.to_world(pos) - start);
                    self.pattern.transform_piece(piece, &Isometry::translation(d - moved));
                    self.dragging = Some(Drag::Piece { piece, start, moved: d });
                }
                &Some(Drag::Handle { point, side }) => {
                    let piece = &mut self.pattern.pieces[point.piece];
                    let offset = proj.to_world(pos) - piece.points[point.point].pos;
                    piece.set_handle(point.point, side, offset, !alt);
                }
                &Some(Drag::Bend { piece, seg, t, grab, start }) => {
                    let delta = proj.to_world(pos) - grab;
                    self.pattern.pieces[piece].bend_segment(seg, t, start, delta);
                }
                None => {}
            }
        }
        if response.drag_stopped()
            && let Some(Drag::Marquee { start, end, additive, .. }) = self.dragging.take()
        {
            self.select_in_rect(proj, Rect::from_two_pos(start, end), additive);
        }
        if response.drag_stopped() {
            self.dragging = None;
        }

        if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((pi, seg, t, _)) = self.pick_outline(proj, pos)
        {
            self.checkpoint();
            if self.tool == Tool::Curve {
                // Double-clicking an edge in the curve tool removes its curvature.
                self.pattern.pieces[pi].straighten_segment(seg);
                self.select_segment(pi, seg, false);
            } else {
                let idx = self.pattern.insert_point(pi, seg, t);
                self.select_point(PointRef { piece: pi, point: idx }, false);
            }
        } else if response.clicked_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            if let Some(r) = self.pick_point(proj, pos) {
                self.select_point(r, shift);
            } else if let Some((pi, seg, ..)) = self.pick_outline(proj, pos) {
                self.select_segment(pi, seg, shift);
            } else if let Some(pi) = self.pick_piece(proj, pos) {
                if !shift {
                    self.clear_selection();
                }
                self.selected_piece = Some(pi);
            } else if !shift {
                self.clear_selection();
            }
        }
    }

    fn draw_tool(&mut self, response: &egui::Response, proj: Projection) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else { return };

        // Clicking the first point of the piece being drawn closes it.
        if let Some(idx) = self.drawing {
            let piece = &self.pattern.pieces[idx];
            let first = proj.to_screen(piece.points[0].pos);
            if piece.points.len() >= 3 && first.distance(pos) <= PICK_PX {
                self.checkpoint();
                self.finish_drawing(true);
                self.status = "Piece closed".into();
                return;
            }
        }

        let w = self.snap(proj, pos, None);
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

    fn add_point_tool(&mut self, response: &egui::Response, proj: Projection) {
        if response.clicked_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((pi, seg, t, _)) = self.pick_outline(proj, pos)
        {
            self.checkpoint();
            let idx = self.pattern.insert_point(pi, seg, t);
            self.select_point(PointRef { piece: pi, point: idx }, false);
        }
    }

    fn sew_tool(&mut self, response: &egui::Response, proj: Projection) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else { return };
        let no_snap = response.ctx.input(|i| i.modifiers.alt);
        match self.sew {
            SewState::Idle | SewState::First(_) => {
                let Some((pi, at)) = self.pick_outline_pos(proj, pos) else { return };
                self.sew = match self.sew {
                    SewState::First(a) => SewState::SecondStart(a, pi, at),
                    _ => SewState::FirstStart(pi, at),
                };
            }
            SewState::FirstStart(start_piece, _) | SewState::SecondStart(_, start_piece, _) => {
                let Some((span, _)) = self.pending_span(proj, pos, no_snap) else {
                    if self.pick_outline_pos(proj, pos).is_some_and(|(pi, _)| pi != start_piece) {
                        self.status = "The end point must be on the same piece as the start".into();
                    }
                    return;
                };
                if let SewState::SecondStart(a, ..) = self.sew {
                    self.checkpoint();
                    self.pattern.seams.push(Seam { a, b: span });
                    self.selected_seam = Some(self.pattern.seams.len() - 1);
                    let [la, lb] = [a, span].map(|s| s.length(&self.pattern.pieces));
                    let u = self.settings.lengths();
                    self.status =
                        format!("Seam created: {} ↔ {} (Δ {})", u.format(la), u.format(lb), u.format(la - lb));
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
    fn pending_span(&self, proj: Projection, screen: Pos2, no_snap: bool) -> Option<(EdgeSpan, bool)> {
        let (start_piece, start, first) = match self.sew {
            SewState::FirstStart(pi, s) => (pi, s, None),
            SewState::SecondStart(a, pi, s) => (pi, s, Some(a)),
            _ => return None,
        };
        let piece = &self.pattern.pieces[start_piece];
        if let Some(a) = first
            && self.settings.snap_match
            && !no_snap
            && let Some((span, _)) = self
                .matching_ends(&a, start_piece, start)
                .into_iter()
                .map(|sp| (sp, proj.to_screen(piece.eval_pos(sp.end)).distance(screen)))
                .filter(|(_, d)| *d <= MATCH_SNAP_PX)
                .min_by(|x, y| x.1.total_cmp(&y.1))
        {
            return Some((span, true));
        }
        let (pi, at) = self.pick_outline_pos(proj, screen)?;
        if pi != start_piece || (piece.eval_pos(at) - piece.eval_pos(start)).length() < 1e-6 {
            return None;
        }
        Some((EdgeSpan::shortest(&self.pattern.pieces, pi, start, at), false))
    }

    // -------------------------------------------------------------- painting

    fn paint(&self, vi: usize, painter: &egui::Painter, proj: Projection, hover: Option<Pos2>) {
        let rect = proj.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, theme::PAPER);
        self.paint_grid(&painter, proj);

        let to_screen = |p: Vec2| proj.to_screen(p);

        for (pi, piece) in self.pattern.pieces.iter().enumerate() {
            let selected = self.selected_piece == Some(pi);
            let color = theme::INK;
            if self.settings.show_allowance {
                let mut cut = piece.cut_line();
                cut.pop(); // closing vertex
                if cut.len() >= 3 {
                    let fill = theme::ALLOWANCE;
                    let mut mesh = egui::Mesh::default();
                    for &p in &cut {
                        mesh.colored_vertex(to_screen(p), fill);
                    }
                    for [a, b, c] in triangulate(&cut) {
                        mesh.add_triangle(a as u32, b as u32, c as u32);
                    }
                    painter.add(mesh);
                    let line: Vec<Pos2> = cut.into_iter().map(to_screen).collect();
                    painter.add(egui::Shape::closed_line(line, Stroke::new(1.0, theme::CUT_LINE)));
                }
            }
            let (poly, tris) = piece.fill_triangles();
            if !tris.is_empty() {
                let fill = if selected { theme::SELECTED_FILL } else { theme::PAPER };
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

            // Diamonds are corners, circles are smooth points. Points on the
            // selected piece are drawn in the selection blue, filled when selected.
            let (stroke, size) = if selected { (theme::ACCENT, 4.5) } else { (theme::MUTED, 3.5) };
            for (i, p) in piece.points.iter().enumerate() {
                let is_sel = self.selected_points.contains(&PointRef { piece: pi, point: i });
                let fill = if is_sel { theme::ACCENT } else { theme::PAPER };
                paint_point(&painter, to_screen(p.pos), p.kind, size, fill, Stroke::new(1.5, stroke));
            }

            // Name in the middle of the piece, clear of seam badges at its corners.
            if let (Some((lo, hi)), true) = (piece.bounds(), piece.closed || selected) {
                painter.text(
                    to_screen(lo.lerp(hi, 0.5)),
                    Align2::CENTER_CENTER,
                    &piece.name,
                    FontId::proportional(13.0),
                    color,
                );
            }
        }

        for &(pi, seg) in &self.selected_segments {
            let pts: Vec<Pos2> =
                self.pattern.pieces[pi].flatten_segment(seg).into_iter().map(to_screen).collect();
            painter.line(pts.clone(), Stroke::new(10.0, theme::ACCENT.gamma_multiply(0.16)));
            painter.line(pts, Stroke::new(2.5, theme::ACCENT));
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
                format!("{} segments · {}", self.selected_segments.len(), self.settings.lengths().format(total))
            } else if self.length_input.is_empty() {
                self.settings.lengths().format(piece.segment_length(seg))
            } else {
                format!("{}_{}  ⏎ to apply", self.length_input, self.settings.units.suffix())
            };
            paint_label(&painter, mid + egui::vec2(12.0, -14.0), &text, theme::ACCENT);
        }

        self.paint_symmetry(&painter, proj);
        self.paint_handles(&painter, proj);

        for (i, seam) in self.pattern.seams.iter().enumerate() {
            let selected = self.selected_seam == Some(i);
            self.paint_seam(&painter, proj, seam, theme::seam(i), selected, Some(i + 1));
        }
        // After the seams, so their dotted connectors don't cross the labels.
        if self.settings.show_lengths {
            self.paint_segment_lengths(&painter, proj);
        }
        self.paint_sewing_preview(&painter, proj, hover);
        if let Some(Drag::Marquee { viewport, start, end, .. }) = self.dragging
            && viewport == vi
        {
            let area = Rect::from_two_pos(start, end);
            painter.rect_filled(area, 0.0, theme::ACCENT.gamma_multiply(0.10));
            painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Middle);
        }

        let Some(hover) = hover else { return };
        match self.tool {
            Tool::Sew => {}
            Tool::Draw => {
                let mut w = self.snap(proj, hover, None);
                let typed = self.settings.units.parse(&self.length_input).filter(|l| *l > 0.0);
                if let (Some(len), Some(idx)) = (typed, self.drawing)
                    && let Some(last) = self.pattern.pieces[idx].points.last()
                {
                    let cursor = proj.to_world(hover) - last.pos;
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
                        Stroke::new(1.0, theme::ACCENT),
                        6.0,
                        4.0,
                    );
                    painter.extend(dashed);
                    let text = if self.length_input.is_empty() {
                        self.settings.lengths().format((w - last.pos).length())
                    } else {
                        format!("{}_{}  ⏎ to place", self.length_input, self.settings.units.suffix())
                    };
                    paint_label(&painter, s + egui::vec2(12.0, 14.0), &text, theme::INK);
                }
                painter.circle_stroke(s, 4.0, Stroke::new(1.5, theme::ACCENT));
            }
            Tool::AddPoint | Tool::Select | Tool::Curve => {
                if self.tool != Tool::AddPoint
                    && (self.pick_point(proj, hover).is_some() || self.pick_handle(proj, hover).is_some())
                {
                    return;
                }
                if let Some((_, _, _, pos)) = self.pick_outline(proj, hover) {
                    painter.circle_stroke(
                        to_screen(pos),
                        4.0,
                        Stroke::new(1.5, theme::ACCENT),
                    );
                }
            }
        }
    }

    /// Length label beside every segment long enough on screen to fit one,
    /// on the outside of closed pieces.
    fn paint_segment_lengths(&self, painter: &egui::Painter, proj: Projection) {
        let units = self.settings.lengths();
        let color = theme::MUTED;
        for piece in &self.pattern.pieces {
            // Outward is to the right of travel for counter-clockwise outlines.
            let side = if piece.closed && piece.area() < 0.0 { -1.0 } else { 1.0 };
            for seg in 0..piece.segment_count() {
                let (a, b) = piece.segment_ends(seg);
                let (pa, pb) = (piece.points[a].pos, piece.points[b].pos);
                if proj.to_screen(pa).distance(proj.to_screen(pb)) < 50.0 {
                    continue;
                }
                let mid = piece.eval_segment(seg, 0.5);
                // Direction of travel at the midpoint, from a nearby sample.
                let dir = piece.eval_segment(seg, 0.51) - piece.eval_segment(seg, 0.49);
                let len = dir.length();
                if len < 1e-12 {
                    continue;
                }
                // World Y is up and screen Y down, so the right-hand normal flips in y.
                let n = egui::vec2((dir.y / len * side) as f32, (dir.x / len * side) as f32);
                let pos = proj.to_screen(mid) + n * 8.0;
                // Anchor on the side facing the edge so the label never covers it.
                let anchor = if n.x > 0.5 {
                    Align2::LEFT_CENTER
                } else if n.x < -0.5 {
                    Align2::RIGHT_CENTER
                } else if n.y > 0.0 {
                    Align2::CENTER_TOP
                } else {
                    Align2::CENTER_BOTTOM
                };
                let text = units.format(piece.segment_length(seg));
                paint_label_at(painter, pos, anchor, 11.0, &text, color);
            }
        }
    }

    /// The dashed mirror line of every symmetry link, spanning its pieces,
    /// and a ring on the mirror image of each selected point.
    fn paint_symmetry(&self, painter: &egui::Painter, proj: Projection) {
        let pieces = &self.pattern.pieces;
        for s in &self.pattern.symmetries {
            let Some((o, d)) = s.axis() else { continue };
            let along = [s.a, s.b].into_iter().flat_map(|pi| pieces[pi].flatten()).map(|p| (p - o).dot(d));
            let (lo, hi) = along.fold((f64::MAX, f64::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)));
            if lo > hi {
                continue;
            }
            let margin = 24.0 / proj.zoom();
            let ends = [o + d * (lo - margin), o + d * (hi + margin)].map(|p| proj.to_screen(p));
            let selected = self.selected_piece.is_some_and(|pi| s.touches(pi));
            let color = theme::MUTED;
            let stroke = if selected { Stroke::new(1.5, color) } else { Stroke::new(1.0, color.gamma_multiply(0.6)) };
            painter.extend(egui::Shape::dashed_line(&ends, stroke, 10.0, 6.0));
        }
        for r in &self.selected_points {
            for (piece, point) in self.pattern.mirror_images(r.piece, r.point) {
                if !self.selected_points.contains(&PointRef { piece, point }) {
                    let s = proj.to_screen(pieces[piece].points[point].pos);
                    painter.circle_stroke(s, 7.0, Stroke::new(1.5, theme::ACCENT));
                }
            }
        }
    }

    /// Handles of the selected points / segment ends. Explicit handles are
    /// solid; automatic ones (curve points without handles) are dashed.
    fn paint_handles(&self, painter: &egui::Painter, proj: Projection) {
        let color = theme::ACCENT;
        for r in self.handle_points() {
            let piece = &self.pattern.pieces[r.piece];
            let point = &piece.points[r.point];
            let base = proj.to_screen(point.pos);
            let (hin, hout) = piece.visible_handles(r.point);
            for (h, explicit) in [(hin, point.handle_in.is_some()), (hout, point.handle_out.is_some())] {
                let Some(h) = h else { continue };
                let tip = proj.to_screen(point.pos + h);
                if explicit {
                    painter.line_segment([base, tip], Stroke::new(1.0, color));
                } else {
                    painter.extend(egui::Shape::dashed_line(&[base, tip], Stroke::new(1.0, color), 4.0, 3.0));
                }
                painter.circle_filled(tip, 4.0, if explicit { color } else { theme::PAPER });
                painter.circle_stroke(tip, 4.0, Stroke::new(1.2, color));
            }
        }
    }

    /// Draw a span along its outline with a perpendicular notch at its start,
    /// and `badge` text in a disc beside the notch. Returns the span's screen
    /// polyline.
    fn paint_span(
        &self,
        painter: &egui::Painter,
        proj: Projection,
        span: &EdgeSpan,
        color: Color32,
        width: f32,
        badge: Option<&str>,
    ) -> Vec<Pos2> {
        let pts: Vec<Pos2> = span
            .polyline(&self.pattern.pieces)
            .into_iter()
            .map(|p| proj.to_screen(p))
            .collect();
        if pts.len() < 2 {
            return pts;
        }
        painter.line(pts.clone(), Stroke::new(width, color));
        // Notch: perpendicular tick through the start, along the first non-degenerate direction.
        if let Some(dir) =
            pts.windows(2).map(|w| w[1] - w[0]).find(|d| d.length() > 0.5).map(|d| d.normalized())
        {
            let n = egui::vec2(-dir.y, dir.x);
            painter.line_segment([pts[0] - n * 12.0, pts[0] + n * 12.0], Stroke::new(width, color));
            if let Some(text) = badge {
                let at = self.badge_pos(proj, span.piece, &pts);
                painter.circle_filled(at, 9.0, color);
                painter.text(at, Align2::CENTER_CENTER, text, FontId::proportional(9.5), theme::PAPER);
            }
        }
        pts
    }

    /// Where to put a span's badge: a little way along it from the start,
    /// clear of the corner point and notch, on the inside of its piece.
    fn badge_pos(&self, proj: Projection, piece: usize, pts: &[Pos2]) -> Pos2 {
        const ALONG: f32 = 22.0;
        const ACROSS: f32 = 12.0;
        let total: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
        let mut left = ALONG.min(total * 0.5);
        let (mut at, mut dir) = (pts[0], egui::Vec2::X);
        for w in pts.windows(2) {
            let d = w[0].distance(w[1]);
            if d < 1e-3 {
                continue;
            }
            dir = (w[1] - w[0]) / d;
            if d >= left {
                at = w[0] + dir * left;
                break;
            }
            left -= d;
            at = w[1];
        }
        let n = egui::vec2(-dir.y, dir.x);
        let piece = &self.pattern.pieces[piece];
        [at + n * ACROSS, at - n * ACROSS]
            .into_iter()
            .find(|&c| piece.closed && piece.contains(proj.to_world(c)))
            .unwrap_or(at + n * ACROSS)
    }

    /// Draw a seam: both spans, dotted connectors, and for a created seam
    /// (`number`, counting from 1) `nA`/`nB` badges at the span starts and a
    /// warning badge when the lengths differ by more than the tolerance.
    fn paint_seam(
        &self,
        painter: &egui::Painter,
        proj: Projection,
        seam: &Seam,
        color: Color32,
        selected: bool,
        number: Option<usize>,
    ) {
        let width = if selected { 4.0 } else { 2.5 };
        let [ba, bb] = ["A", "B"].map(|side| number.map(|n| format!("{n}{side}")));
        let a = self.paint_span(painter, proj, &seam.a, color, width, ba.as_deref());
        let b = self.paint_span(painter, proj, &seam.b, color, width, bb.as_deref());
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
        if number.is_some() && (la - lb).abs() > self.settings.seam_tolerance {
            let mid = to_pos(point_at_fraction(&wa, 0.5).lerp(point_at_fraction(&wb, 0.5), 0.5));
            painter.circle_filled(mid, 8.0, theme::WARN);
            painter.text(mid, Align2::CENTER_CENTER, "!", FontId::proportional(12.0), theme::PAPER);
            paint_label(painter, mid + egui::vec2(12.0, 0.0), &format!("Δ {}", self.settings.lengths().format(la - lb)), theme::WARN);
        }
    }

    fn paint_sewing_preview(&self, painter: &egui::Painter, proj: Projection, hover: Option<Pos2>) {
        if self.tool != Tool::Sew {
            return;
        }
        let pieces = &self.pattern.pieces;
        let u = self.settings.lengths();
        // A seam being made is in the selection blue; it takes its own color once created.
        let color = theme::ACCENT;
        let no_snap = painter.ctx().input(|i| i.modifiers.alt);
        let pending = hover.and_then(|h| self.pending_span(proj, h, no_snap));
        let marker = |piece: usize, at: OutlinePos| {
            let s = proj.to_screen(pieces[piece].eval_pos(at));
            painter.circle_filled(s, 4.0, color);
        };
        // Length readout next to the cursor.
        let mut readout: Option<(String, Color32)> = None;

        match self.sew {
            SewState::Idle => {}
            SewState::FirstStart(pi, start) => {
                marker(pi, start);
                if let Some((span, _)) = pending {
                    self.paint_span(painter, proj, &span, color.gamma_multiply(0.6), 2.5, None);
                    readout = Some((format!("A {}", u.format(span.length(pieces))), theme::INK));
                }
            }
            SewState::First(a) => {
                self.paint_span(painter, proj, &a, color, 2.5, None);
                readout = Some((format!("A {}", u.format(a.length(pieces))), theme::INK));
            }
            SewState::SecondStart(a, pi, start) => {
                self.paint_span(painter, proj, &a, color, 2.5, None);
                marker(pi, start);
                if self.settings.snap_match && !no_snap {
                    for end in self.matching_ends(&a, pi, start) {
                        let s = proj.to_screen(pieces[pi].eval_pos(end.end));
                        let diamond = [(0.0, -7.0), (7.0, 0.0), (0.0, 7.0), (-7.0, 0.0)]
                            .map(|(x, y)| s + egui::vec2(x, y))
                            .to_vec();
                        painter.add(egui::Shape::convex_polygon(
                            diamond,
                            theme::PAPER,
                            Stroke::new(2.0, theme::OK),
                        ));
                    }
                }
                let la = a.length(pieces);
                readout = Some((format!("A {}", u.format(la)), theme::INK));
                if let Some((b, snapped)) = pending {
                    let seam = Seam { a, b };
                    self.paint_seam(painter, proj, &seam, color.gamma_multiply(0.6), false, None);
                    let lb = b.length(pieces);
                    let ok = (la - lb).abs() <= self.settings.seam_tolerance;
                    readout = Some((
                        format!(
                            "A {} · B {} · Δ {}{}",
                            u.format(la),
                            u.format(lb),
                            u.number(la - lb),
                            if snapped { "  (matched)" } else { "" }
                        ),
                        if ok { theme::OK } else { theme::WARN },
                    ));
                }
            }
        }

        let hovered = match pending {
            Some((span, _)) => Some((span.piece, span.end)),
            None => hover.and_then(|h| self.pick_outline_pos(proj, h)),
        };
        if let Some((pi, at)) = hovered {
            let s = proj.to_screen(pieces[pi].eval_pos(at));
            painter.circle_stroke(s, 5.0, Stroke::new(1.5, color));
        }
        if let (Some(h), Some((text, c))) = (hover, readout) {
            paint_label(painter, h + egui::vec2(14.0, 14.0), &text, c);
        }
    }

    fn paint_grid(&self, painter: &egui::Painter, proj: Projection) {
        let rect = proj.rect;
        let (minor, major) = self.settings.units.grid_steps(proj.zoom(), 8.0);
        let lo = proj.to_world(rect.left_bottom());
        let hi = proj.to_world(rect.right_top());

        for (step, color) in [(minor, theme::GRID_MINOR), (major, theme::GRID_MAJOR)] {
            let stroke = Stroke::new(1.0, color);
            let mut x = (lo.x / step).floor() * step;
            while x <= hi.x {
                let sx = proj.to_screen(Vec2::new(x, 0.0)).x;
                painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], stroke);
                x += step;
            }
            let mut y = (lo.y / step).floor() * step;
            while y <= hi.y {
                let sy = proj.to_screen(Vec2::new(0.0, y)).y;
                painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], stroke);
                y += step;
            }
        }

        let origin = proj.to_screen(Vec2::default());
        painter.line_segment(
            [Pos2::new(rect.left(), origin.y), Pos2::new(rect.right(), origin.y)],
            Stroke::new(1.0, theme::SEAMS[0].gamma_multiply(0.45)),
        );
        painter.line_segment(
            [Pos2::new(origin.x, rect.top()), Pos2::new(origin.x, rect.bottom())],
            Stroke::new(1.0, theme::OK.gamma_multiply(0.45)),
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
                } else if i.key_pressed(Key::E) {
                    Some(Tool::Curve)
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
                    i.key_pressed(Key::C).then_some(i.modifiers.shift),
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
        match curve {
            Some(true) => self.straighten_selected(),
            Some(false) => self.toggle_curve(),
            None => {}
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
            self.fit_active();
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
                // Arrows move one grid step (or the unit's nudge step without
                // snapping); Shift moves the fine step.
                let (normal, fine_step) = self.settings.units.nudge_steps();
                let step = if fine {
                    fine_step
                } else if self.settings.snap_grid {
                    self.settings.snap_step
                } else {
                    normal
                };
                self.nudge_selected(nudge * step);
            }
        }
        if snap {
            self.settings.snap_grid = !self.settings.snap_grid;
        }

        let (rotate, flip, unfold, mirrored, duplicate, lengths) = ctx.input_mut(|i| {
            let plain = !i.modifiers.command && !i.modifiers.alt;
            (
                (plain && i.key_pressed(Key::R)).then_some(if i.modifiers.shift { 90.0 } else { -90.0 }),
                (plain && i.key_pressed(Key::H)).then_some(!i.modifiers.shift),
                (plain && i.key_pressed(Key::U)).then_some(i.modifiers.shift),
                // Ctrl+Shift+D must be consumed before Ctrl+D.
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::D),
                i.consume_key(Modifiers::COMMAND, Key::D),
                plain && i.key_pressed(Key::L),
            )
        });
        if lengths {
            self.settings.show_lengths = !self.settings.show_lengths;
        }
        if let Some(deg) = rotate {
            self.rotate_selected(deg);
        }
        if let Some(horizontal) = flip {
            self.flip_selected(horizontal);
        }
        if let Some(symmetric) = unfold {
            self.unfold_selected(symmetric);
        }
        if mirrored || duplicate {
            self.duplicate_selected(mirrored);
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
                .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '/' | ' '))
                .collect()
        });
        for c in typed.chars() {
            // Space and '/' only make sense after a number (`3 1/2`, `3/8`).
            if c.is_ascii_digit() || (!self.length_input.is_empty() && !self.length_input.ends_with(' ')) {
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

    // -------------------------------------------------------- command palette

    /// Whether a command can run right now (others are listed greyed out).
    fn command_enabled(&self, cmd: Command) -> bool {
        let piece = self.selected_piece.is_some();
        match cmd {
            Command::ClosePiece => self.drawing.is_some(),
            Command::Undo => !self.undo.is_empty(),
            Command::CloseView => self.viewports.len() > 1,
            Command::Redo => !self.redo.is_empty(),
            Command::Delete => piece || self.selected_seam.is_some(),
            Command::ToggleCurve | Command::ClearHandles => !self.selected_points.is_empty(),
            Command::Straighten => !self.selected_segments.is_empty(),
            Command::RotateCw
            | Command::RotateCcw
            | Command::FlipH
            | Command::FlipV
            | Command::Duplicate
            | Command::DuplicateMirrored => piece,
            Command::Unfold | Command::UnfoldSymmetric => self.can_unfold(),
            Command::Unlink => self.selected_piece.is_some_and(|pi| self.pattern.is_linked(pi)),
            Command::AddAllowance => self.selected_piece.is_some_and(|pi| self.pattern.pieces[pi].closed),
            Command::RemoveAllowance => self.selected_piece.is_some_and(|pi| self.pattern.pieces[pi].has_allowance()),
            _ => true,
        }
    }

    fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::Tool(tool) => self.set_tool(tool),
            Command::ClosePiece => {
                self.checkpoint();
                self.finish_drawing(true);
            }
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::New => self.new_pattern(),
            Command::Open => self.open(),
            Command::Save => self.save(false),
            Command::SaveAs => self.save(true),
            Command::ExportDxf => self.export_dxf(),
            Command::Fit => self.fit_active(),
            Command::SplitView => self.split_view(),
            Command::CloseView => self.close_view(),
            Command::ToggleGrid => self.settings.snap_grid = !self.settings.snap_grid,
            Command::ToggleSnapPoints => self.settings.snap_points = !self.settings.snap_points,
            Command::ToggleMatchSnap => self.settings.snap_match = !self.settings.snap_match,
            Command::ToggleLengths => self.settings.show_lengths = !self.settings.show_lengths,
            Command::ToggleAllowance => self.settings.show_allowance = !self.settings.show_allowance,
            Command::ToggleUnits => self.settings.toggle_units(),
            Command::Settings => self.show_settings = true,
            Command::Delete => self.delete_selection(),
            Command::ToggleCurve => self.toggle_curve(),
            Command::ClearHandles => self.clear_selected_handles(),
            Command::Straighten => self.straighten_selected(),
            Command::RotateCw => self.rotate_selected(-90.0),
            Command::RotateCcw => self.rotate_selected(90.0),
            Command::FlipH => self.flip_selected(true),
            Command::FlipV => self.flip_selected(false),
            Command::Duplicate => self.duplicate_selected(false),
            Command::DuplicateMirrored => self.duplicate_selected(true),
            Command::Unfold => self.unfold_selected(false),
            Command::UnfoldSymmetric => self.unfold_selected(true),
            Command::Unlink => self.unlink_selected(),
            Command::AddAllowance => {
                if let Some(pi) = self.selected_piece {
                    self.set_piece_allowance(pi, self.settings.seam_allowance);
                }
            }
            Command::RemoveAllowance => {
                if let Some(pi) = self.selected_piece {
                    self.set_piece_allowance(pi, 0.0);
                }
            }
        }
    }

    fn show_palette(&mut self, ctx: &egui::Context) {
        let entries: Vec<Entry> = COMMANDS
            .iter()
            .map(|&(cmd, label, shortcut)| Entry { label, shortcut, enabled: self.command_enabled(cmd) })
            .collect();
        if let Some(i) = self.palette.show(ctx, &entries) {
            self.run_command(COMMANDS[i].0);
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
                if ui.button("Export DXF (AAMA/ASTM)…").clicked() {
                    self.export_dxf();
                    ui.close();
                }
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
                let curved = self.selected_segments.iter().any(|&(p, s)| self.pattern.pieces[p].segment_is_curved(s));
                if ui.add_enabled(curved, egui::Button::new("Remove curvature  (Shift+C)")).clicked() {
                    self.straighten_selected();
                    ui.close();
                }
                ui.separator();
                if ui.button("Settings…").clicked() {
                    self.show_settings = true;
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
                    self.duplicate_selected(false);
                }
                if item(ui, self.can_unfold(), "Unfold along selected edge  (U)") {
                    self.unfold_selected(false);
                }
                ui.separator();
                if item(ui, has, "Duplicate mirrored, linked  (Ctrl+Shift+D)") {
                    self.duplicate_selected(true);
                }
                if item(ui, self.can_unfold(), "Unfold symmetric, linked  (Shift+U)") {
                    self.unfold_selected(true);
                }
                let linked = self.selected_piece.is_some_and(|pi| self.pattern.is_linked(pi));
                if item(ui, linked, "Unlink symmetry") {
                    self.unlink_selected();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Command palette  (Ctrl+K)").clicked() {
                    self.palette.open();
                    ui.close();
                }
                ui.checkbox(&mut self.settings.show_lengths, "Segment lengths  (L)");
                ui.checkbox(&mut self.settings.show_allowance, "Seam allowance");
                ui.menu_button("Units", |ui| {
                    for units in [Units::Millimeters, Units::Inches] {
                        if ui.radio(self.settings.units == units, units.name()).clicked()
                            && self.settings.units != units
                        {
                            self.settings.toggle_units();
                        }
                    }
                });
                ui.separator();
                if ui.button("Split view").clicked() {
                    self.split_view();
                    ui.close();
                }
                if ui.add_enabled(self.viewports.len() > 1, egui::Button::new("Close view")).clicked() {
                    self.close_view();
                    ui.close();
                }
                if ui.button("Fit to pattern  (F)").clicked() {
                    self.fit_active();
                    ui.close();
                }
            });
        });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tools");
        for tool in [Tool::Select, Tool::Draw, Tool::AddPoint, Tool::Curve, Tool::Sew] {
            if ui.selectable_label(self.tool == tool, tool.label()).on_hover_text(tool.help()).clicked() {
                self.set_tool(tool);
            }
        }

        ui.separator();
        ui.heading("Snapping");
        ui.checkbox(&mut self.settings.snap_points, "Snap to points");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.settings.snap_grid, "Grid (G)");
            let units = self.settings.lengths();
            ui.add_enabled_ui(self.settings.snap_grid, |ui| {
                length_value(ui, &mut self.settings.snap_step, units, 0.1..=1000.0);
            });
        });

        ui.separator();
        ui.heading("Sewing");
        ui.checkbox(&mut self.settings.snap_match, "Snap to matching length")
            .on_hover_text("While sewing the second edge, snap its end to where its length equals the first edge. Hold Alt to bypass.");
        ui.horizontal(|ui| {
            ui.label("Tolerance");
            let units = self.settings.lengths();
            length_value(ui, &mut self.settings.seam_tolerance, units, 0.0..=50.0)
                .on_hover_text("Seams whose edge lengths differ by more than this are flagged");
        });

        ui.separator();
        ui.heading("Pieces");
        let mut clicked = None;
        for (i, piece) in self.pattern.pieces.iter().enumerate() {
            let label = format!(
                "{}{}{}  ({} pts)",
                piece.name,
                if piece.closed { "" } else { " [open]" },
                if self.pattern.is_linked(i) { " ↔" } else { "" },
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
                ui.label(format!("Area: {}", self.settings.units.format_area(piece.area().abs())));
            }
            ui.label(format!("Perimeter: {}", self.settings.lengths().format(piece.perimeter())));
            self.symmetry_panel(ui, pi);
            if self.pattern.pieces[pi].closed {
                self.allowance_panel(ui, pi);
            }
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
                if ui.button("Clear handles").clicked() {
                    self.clear_selected_handles();
                }
            });
        }

        if let Some(r) = self.single_point() {
            ui.separator();
            ui.heading(format!("Point {}", r.point + 1));
            let mut p = self.pattern.pieces[r.piece].points[r.point];
            let kind_before = p.kind;
            let mut begin_edit = false;
            let mut clear_handles = false;
            let units = self.settings.lengths();
            egui::Grid::new("point").num_columns(2).show(ui, |ui| {
                for (label, v) in [("X", &mut p.pos.x), ("Y", &mut p.pos.y)] {
                    ui.label(label);
                    let resp = length_value(ui, v, units, f64::MIN..=f64::MAX);
                    begin_edit |= resp.drag_started() || resp.gained_focus();
                    ui.end_row();
                }
                ui.label("Type");
                ui.horizontal(|ui| {
                    begin_edit |= ui.radio_value(&mut p.kind, PointKind::Turn, "Turn").clicked();
                    begin_edit |= ui.radio_value(&mut p.kind, PointKind::Curve, "Curve").clicked();
                });
                ui.end_row();
                if p.has_handles() {
                    ui.label("Handles");
                    clear_handles = ui
                        .button("Clear")
                        .on_hover_text("Remove explicit handles (curve points go back to automatic smoothing)")
                        .clicked();
                    ui.end_row();
                }
            });
            if begin_edit || clear_handles {
                self.checkpoint();
            }
            let piece = &mut self.pattern.pieces[r.piece];
            piece.points[r.point] = p;
            if p.kind != kind_before {
                piece.set_point_kind(r.point, p.kind);
            }
            if clear_handles {
                piece.clear_handles(r.point);
            }
        }
    }

    /// What piece `pi` is symmetrically linked with, and a button to unlink it.
    fn symmetry_panel(&mut self, ui: &mut egui::Ui, pi: usize) {
        let pieces = &self.pattern.pieces;
        let links: Vec<String> = (self.pattern.symmetries.iter().filter(|s| s.touches(pi)))
            .map(|s| {
                let other = &pieces[if s.a == pi { s.b } else { s.a }].name;
                match (s.is_halves(), s.axis().is_some()) {
                    (true, _) => "Symmetric halves".to_owned(),
                    (false, true) => format!("Mirrors {other}"),
                    (false, false) => format!("Linked copy of {other}"),
                }
            })
            .collect();
        if links.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("↔ {}", links.join(", ")))
                .on_hover_text("Edits to one side are mirrored onto the other");
            if ui.small_button("Unlink").on_hover_text("Edit the sides independently from now on").clicked() {
                self.unlink_selected();
            }
        });
    }

    /// Whole-piece seam allowance: add, edit or remove it, and reset
    /// per-segment widths back to it.
    fn allowance_panel(&mut self, ui: &mut egui::Ui, pi: usize) {
        let units = self.settings.lengths();
        let piece = &self.pattern.pieces[pi];
        let overrides = piece.points.iter().filter(|p| p.allowance.is_some()).count();
        let mut width = piece.seam_allowance;
        let mut begin_edit = false;
        let mut changed = false;
        let mut set = None;
        let mut reset = false;
        ui.horizontal(|ui| {
            ui.label("Seam allowance");
            if !piece.has_allowance() {
                let label = format!("Add {}", units.format(self.settings.seam_allowance));
                if ui.button(label).on_hover_text("Add seam allowance around the whole piece").clicked() {
                    set = Some(self.settings.seam_allowance);
                }
                return;
            }
            let resp = length_value(ui, &mut width, units, 0.0..=500.0)
                .on_hover_text("Allowance of every segment that doesn't have its own");
            begin_edit |= resp.drag_started() || resp.gained_focus();
            changed |= resp.changed();
            if ui.button("Remove").on_hover_text("Remove the seam allowance from every segment").clicked() {
                set = Some(0.0);
            }
        });
        if overrides > 0 {
            ui.horizontal(|ui| {
                ui.label(format!("{overrides} segment(s) have their own width"));
                reset = ui
                    .button("Reset")
                    .on_hover_text("Give every segment the piece's seam allowance")
                    .clicked();
            });
        }
        if begin_edit {
            self.checkpoint();
        }
        if changed {
            self.pattern.pieces[pi].seam_allowance = width;
        }
        if let Some(w) = set {
            self.set_piece_allowance(pi, w);
        }
        if reset {
            self.checkpoint();
            self.pattern.pieces[pi].clear_allowance_overrides();
        }
    }

    /// Give the whole piece seam allowance `width` (0 removes it), dropping
    /// per-segment widths.
    fn set_piece_allowance(&mut self, pi: usize, width: f64) {
        self.checkpoint();
        let piece = &mut self.pattern.pieces[pi];
        piece.seam_allowance = width;
        piece.clear_allowance_overrides();
    }

    /// Set (or with `None`, reset to the piece default) the seam allowance
    /// of every selected segment.
    fn set_selected_allowance(&mut self, width: Option<f64>) {
        for &(pi, seg) in &self.selected_segments {
            self.pattern.pieces[pi].points[seg].allowance = width;
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
                self.duplicate_selected(false);
            }
            if ui
                .button("Mirrored copy")
                .on_hover_text(
                    "Duplicate, flip horizontally and link the two pieces: edits to either are mirrored \
                     onto the other (Ctrl+Shift+D)",
                )
                .clicked()
            {
                self.duplicate_selected(true);
            }
        });
        ui.horizontal(|ui| {
            let can = self.can_unfold();
            let disabled = "Select a straight edge of a closed piece first";
            if ui
                .add_enabled(can, egui::Button::new("Unfold"))
                .on_hover_text("Mirror the piece across the selected straight edge and merge the halves (U)")
                .on_disabled_hover_text(disabled)
                .clicked()
            {
                self.unfold_selected(false);
            }
            if ui
                .add_enabled(can, egui::Button::new("Unfold symmetric"))
                .on_hover_text(
                    "Unfold and keep the halves linked across the fold: edits to one half are mirrored \
                     onto the other (Shift+U)",
                )
                .on_disabled_hover_text(disabled)
                .clicked()
            {
                self.unfold_selected(true);
            }
        });
    }

    fn segments_panel(&mut self, ui: &mut egui::Ui, pi: usize) {
        let piece = &self.pattern.pieces[pi];
        // (start, end, length, allowance, has its own allowance)
        let rows: Vec<(usize, usize, f64, f64, bool)> = (0..piece.segment_count())
            .map(|seg| {
                let (a, b) = piece.segment_ends(seg);
                let own = piece.points[seg].allowance.is_some();
                (a, b, piece.segment_length(seg), piece.segment_allowance(seg), own)
            })
            .collect();
        let allowances = piece.closed;
        if rows.is_empty() {
            return;
        }
        let additive = ui.input(|i| i.modifiers.shift);
        let units = self.settings.lengths();
        let mut select = None;
        let mut begin_edit = false;
        let mut change = None;
        let mut allowance_change = None;
        egui::CollapsingHeader::new(format!("Segments ({})", rows.len()))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Edits move");
                    ui.radio_value(&mut self.settings.length_moves_start, false, "end point");
                    ui.radio_value(&mut self.settings.length_moves_start, true, "start point");
                });
                egui::Grid::new("segments").num_columns(4).striped(true).show(ui, |ui| {
                    if allowances {
                        ui.label("");
                        ui.label("Length");
                        ui.label("Allowance");
                        ui.end_row();
                    }
                    for (seg, &(a, b, len, width, own)) in rows.iter().enumerate() {
                        let label = format!("{} → {}", a + 1, b + 1);
                        let selected = self.selected_segments.contains(&(pi, seg));
                        if ui.selectable_label(selected, label).clicked() {
                            select = Some(seg);
                        }
                        let mut v = len;
                        let resp = length_value(ui, &mut v, units, 0.1..=100_000.0);
                        begin_edit |= resp.drag_started() || resp.gained_focus();
                        if resp.changed() {
                            change = Some((seg, v));
                        }
                        if allowances {
                            let mut w = width;
                            let resp = length_value(ui, &mut w, units, 0.0..=500.0).on_hover_text(if own {
                                "This segment's own seam allowance"
                            } else {
                                "The piece's seam allowance; edit to give this segment its own"
                            });
                            begin_edit |= resp.drag_started() || resp.gained_focus();
                            if resp.changed() {
                                allowance_change = Some((seg, Some(w)));
                            }
                            if own
                                && ui.small_button("↺").on_hover_text("Use the piece's seam allowance").clicked()
                            {
                                begin_edit = true;
                                allowance_change = Some((seg, None));
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        let any_curved = self.selected_segments.iter().any(|&(p, s)| self.pattern.pieces[p].segment_is_curved(s));
        if !self.selected_segments.is_empty()
            && ui
                .add_enabled(any_curved, egui::Button::new("Remove curvature"))
                .on_hover_text("Make the selected segments straight; neighbors keep their shape (Shift+C)")
                .on_disabled_hover_text("The selected segments are already straight")
                .clicked()
        {
            self.straighten_selected();
        }
        if let Some(seg) = select {
            self.select_segment(pi, seg, additive);
        }
        if begin_edit {
            self.checkpoint();
        }
        if let Some((seg, len)) = change {
            self.pattern.pieces[pi].set_segment_length(seg, len, self.settings.length_moves_start);
        }
        if let Some((seg, width)) = allowance_change {
            self.pattern.pieces[pi].points[seg].allowance = width;
        }
        if allowances {
            self.selected_allowance_row(ui);
        }
    }

    /// Seam allowance for all selected segments at once.
    fn selected_allowance_row(&mut self, ui: &mut egui::Ui) {
        let Some(&(p0, s0)) = self.selected_segments.first() else { return };
        if !self.selected_segments.iter().all(|&(p, _)| self.pattern.pieces[p].closed) {
            return;
        }
        let units = self.settings.lengths();
        let mut width = self.pattern.pieces[p0].segment_allowance(s0);
        let any_own = self.selected_segments.iter().any(|&(p, s)| self.pattern.pieces[p].points[s].allowance.is_some());
        let mut begin_edit = false;
        let mut set = None;
        ui.horizontal(|ui| {
            ui.label(format!("Allowance of {} selected", self.selected_segments.len()));
            let resp = length_value(ui, &mut width, units, 0.0..=500.0)
                .on_hover_text("Type or drag to set the seam allowance of every selected segment; 0 for none");
            begin_edit |= resp.drag_started() || resp.gained_focus();
            if resp.changed() {
                set = Some(Some(width));
            }
            if ui
                .add_enabled(any_own, egui::Button::new("Piece default"))
                .on_hover_text("Use the piece's seam allowance on the selected segments")
                .clicked()
            {
                begin_edit = true;
                set = Some(None);
            }
        });
        if begin_edit {
            self.checkpoint();
        }
        if let Some(width) = set {
            self.set_selected_allowance(width);
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
            let mismatch = (la - lb).abs() > self.settings.seam_tolerance;
            let text = egui::RichText::new(format!(
                "Seam {}: {} ↔ {}  Δ {}{}",
                i + 1,
                pieces[seam.a.piece].name,
                pieces[seam.b.piece].name,
                self.settings.lengths().number(la - lb),
                if mismatch { "  ⚠" } else { "" }
            ))
            .color(theme::seam(i));
            let resp = ui.selectable_label(self.selected_seam == Some(i), text);
            if resp.clicked() {
                clicked = Some(i);
            }
            if mismatch {
                resp.on_hover_text(format!(
                    "Edge lengths differ by more than the {} tolerance",
                    self.settings.lengths().format(self.settings.seam_tolerance)
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
        let u = self.settings.lengths();
        ui.label(format!("Edge A: {}   Edge B: {}", u.format(la), u.format(lb)));
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

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("Settings").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            let s = &mut self.settings;
            egui::Grid::new("settings").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label("Units");
                ui.horizontal(|ui| {
                    for units in [Units::Millimeters, Units::Inches] {
                        if ui.radio(s.units == units, units.name()).clicked() && s.units != units {
                            s.toggle_units();
                        }
                    }
                });
                ui.end_row();

                ui.label("Inch lengths");
                egui::ComboBox::from_id_salt("inch_format")
                    .selected_text(s.inch_format.name())
                    .show_ui(ui, |ui| {
                        for f in InchFormat::ALL {
                            ui.selectable_value(&mut s.inch_format, f, f.name());
                        }
                    })
                    .response
                    .on_hover_text("How lengths are written when units are inches");
                ui.end_row();

                ui.label("Segment lengths");
                ui.checkbox(&mut s.show_lengths, "Label every segment (L)");
                ui.end_row();

                ui.label("Seam allowance");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.show_allowance, "Show cut line");
                    ui.horizontal(|ui| {
                        ui.label("Default width");
                        let units = s.lengths();
                        length_value(ui, &mut s.seam_allowance, units, 0.0..=500.0)
                            .on_hover_text("Width given to a piece by \"Add seam allowance\"");
                    });
                });
                ui.end_row();

                ui.label("Snapping");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.snap_points, "Snap to points");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.snap_grid, "Grid step");
                        let units = s.lengths();
                        length_value(ui, &mut s.snap_step, units, 0.1..=1000.0);
                    });
                });
                ui.end_row();

                ui.label("Sewing");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.snap_match, "Snap to matching length");
                    ui.horizontal(|ui| {
                        ui.label("Length tolerance");
                        let units = s.lengths();
                        length_value(ui, &mut s.seam_tolerance, units, 0.0..=50.0);
                    });
                });
                ui.end_row();

                ui.label("Segment length edits");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.length_moves_start, false, "move end point");
                    ui.radio_value(&mut s.length_moves_start, true, "move start point");
                });
                ui.end_row();

                ui.label("AI agents (MCP)");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.mcp_enabled, "Serve on port");
                        ui.add(egui::DragValue::new(&mut s.mcp_port).range(1024..=65535));
                    });
                    match (&self.mcp, &self.mcp_error) {
                        (Some(server), _) => {
                            ui.horizontal(|ui| {
                                ui.monospace(server.url());
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(server.url());
                                }
                            });
                        }
                        (None, Some(e)) => {
                            ui.colored_label(ui.visuals().warn_fg_color, e);
                        }
                        (None, None) => {}
                    }
                });
                ui.end_row();
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset to defaults").clicked() {
                    *s = Settings::default();
                }
                if let Some(path) = Settings::path() {
                    ui.weak(format!("Saved to {}", path.display()));
                }
            });
        });
        self.show_settings = open;
    }

    /// Write settings to disk if they changed since the last save.
    fn save_settings(&mut self) {
        if self.settings == self.saved_settings {
            return;
        }
        match self.settings.save() {
            Ok(()) => self.saved_settings = self.settings.clone(),
            Err(e) => self.status = format!("Couldn't save settings: {e}"),
        }
    }

    /// What clicks and keys do right now, for the hint bar: `(keys, action)`.
    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.length_input_active() && !self.length_input.is_empty() {
            let enter = if self.tool == Tool::Draw { "Place point" } else { "Set length" };
            return vec![("Enter", enter), ("Backspace", "Delete digit"), ("Esc", "Cancel")];
        }
        match self.dragging {
            Some(Drag::Handle { .. }) => return vec![("Alt", "Break smooth point into a corner")],
            Some(Drag::Marquee { .. }) => return vec![("Release", "Select points inside")],
            Some(Drag::Points { .. } | Drag::Piece { .. }) => return vec![("G", "Grid snap on/off")],
            Some(Drag::Bend { .. }) | None => {}
        }
        let mut hints = match self.tool {
            Tool::Select => {
                let mut h = vec![("Click", "Select"), ("Shift", "Add to selection"), ("Drag", "Move")];
                if self.single_segment().is_some() {
                    h.insert(0, ("0–9", "Type length"));
                } else {
                    h.push(("Ctrl drag", "Box select"));
                }
                if self.selected_piece.is_some() {
                    h.extend([("Arrows", "Nudge"), ("R", "Rotate"), ("H", "Flip")]);
                }
                h.push(("Double-click", "Insert point"));
                h
            }
            Tool::Draw if self.drawing.is_some() => vec![
                ("Click", "Place point"),
                ("0–9", "Type length"),
                ("Enter", "Close piece"),
                ("Esc", "Stop"),
            ],
            Tool::Draw => vec![("Click", "Place first point")],
            Tool::AddPoint => vec![("Click outline", "Insert point")],
            Tool::Curve => vec![
                ("Drag edge", "Bend"),
                ("Double-click", "Straighten"),
                ("Drag handle", "Shape"),
                ("Alt", "Break smooth point"),
            ],
            Tool::Sew => match self.sew {
                SewState::Idle => vec![("Click edge", "Start of first edge")],
                SewState::FirstStart(..) => vec![("Click", "End of first edge"), ("Esc", "Cancel")],
                SewState::First(_) => vec![("Click edge", "Start of matching edge"), ("Esc", "Cancel")],
                SewState::SecondStart(..) => {
                    let mut h = vec![("Click", "End of matching edge")];
                    if self.settings.snap_match {
                        h.push(("Alt", "Place freely"));
                    }
                    h.push(("Esc", "Cancel"));
                    h
                }
            },
        };
        hints.push(("Ctrl K", "Commands"));
        hints
    }

    /// Bottom bar: hints (or the latest status message) on the left; display
    /// toggles, units, cursor position and zoom on the right.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(view) = self.viewports[self.active_viewport].camera() {
                    ui.label(egui::RichText::new(format!("{:.0}%", view.zoom * 100.0)).monospace().color(theme::MUTED))
                        .on_hover_text("Zoom");
                }
                if let Some(c) = self.cursor_world {
                    let u = self.settings.lengths();
                    let text = format!("x {}  y {}", u.number(c.x), u.number(c.y));
                    ui.label(egui::RichText::new(text).monospace().color(theme::MUTED));
                }
                let units = match self.settings.units {
                    Units::Millimeters => "mm",
                    Units::Inches => "in",
                };
                if toggle_chip(ui, units, false).on_hover_text("Units: click to switch").clicked() {
                    self.settings.toggle_units();
                }
                ui.separator();
                let s = &mut self.settings;
                for (label, on, tip) in [
                    ("SA", &mut s.show_allowance, "Show seam allowance"),
                    ("LENGTHS", &mut s.show_lengths, "Label segment lengths (L)"),
                    ("SNAP", &mut s.snap_points, "Snap to points"),
                    ("GRID", &mut s.snap_grid, "Snap to grid (G)"),
                ] {
                    if toggle_chip(ui, label, *on).on_hover_text(tip).clicked() {
                        *on = !*on;
                    }
                }
                if let Some(server) = &self.mcp {
                    ui.separator();
                    ui.label(egui::RichText::new("MCP").color(theme::MUTED))
                        .on_hover_text(format!("AI agents can connect at {}", server.url()));
                } else if let Some(e) = &self.mcp_error {
                    ui.separator();
                    ui.colored_label(theme::WARN, "MCP off").on_hover_text(e);
                }
                ui.separator();
                // The rest of the bar, left to right.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if !self.status.is_empty() {
                        ui.label(&self.status);
                        return;
                    }
                    for (keys, action) in self.hints() {
                        key_cap(ui, keys);
                        ui.label(action);
                        ui.add_space(6.0);
                    }
                });
            });
        });
    }
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
/// A key or mouse action drawn as a key cap, for the hint bar.
fn key_cap(ui: &mut egui::Ui, keys: &str) {
    egui::Frame::new()
        .fill(theme::PAPER)
        .stroke(Stroke::new(1.0, theme::CUT_LINE))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(keys).size(11.0).strong().color(theme::INK));
        });
}

/// Small caps chip for a display toggle: blue while on.
fn toggle_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let (fill, text) = if on {
        (theme::ACCENT.gamma_multiply(0.12), theme::ACCENT)
    } else {
        (Color32::TRANSPARENT, theme::MUTED)
    };
    let text = egui::RichText::new(label).size(10.5).strong().extra_letter_spacing(0.8).color(text);
    ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(3.0))
}

/// A point marker: a diamond for a corner, a circle for a smooth point.
fn paint_point(painter: &egui::Painter, at: Pos2, kind: PointKind, size: f32, fill: Color32, stroke: Stroke) {
    match kind {
        PointKind::Turn => {
            let r = size * 1.25;
            let diamond = [(0.0, -r), (r, 0.0), (0.0, r), (-r, 0.0)].map(|(x, y)| at + egui::vec2(x, y)).to_vec();
            painter.add(egui::Shape::convex_polygon(diamond, fill, stroke));
        }
        PointKind::Curve => {
            painter.circle_filled(at, size, fill);
            painter.circle_stroke(at, size, stroke);
        }
    }
}

fn paint_label(painter: &egui::Painter, pos: Pos2, text: &str, color: Color32) {
    paint_label_at(painter, pos, Align2::LEFT_CENTER, 12.0, text, color);
}

fn paint_label_at(painter: &egui::Painter, pos: Pos2, anchor: Align2, size: f32, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(size), color);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand(2.0), 3.0, theme::PAPER);
    painter.galley(rect.min, galley, color);
}

/// Drag field for a length stored in mm, shown (as a fraction for inches,
/// if so set) and edited in `lengths`' units. Typed fractions are accepted.
fn length_value(
    ui: &mut egui::Ui,
    mm: &mut f64,
    lengths: Lengths,
    range_mm: std::ops::RangeInclusive<f64>,
) -> egui::Response {
    let units = lengths.units;
    let mut v = units.display_value(*mm);
    let resp = ui.add(
        egui::DragValue::new(&mut v)
            .speed(units.drag_speed())
            .custom_formatter(|v, _| lengths.number_display(v))
            .custom_parser(|s| lengths.parse(s).map(|mm| units.display_value(mm)))
            .suffix(units.suffix()),
    );
    if resp.changed() {
        *mm = units.to_mm(v).clamp(*range_mm.start(), *range_mm.end());
    }
    resp
}

// -------------------------------------------------------------------- MCP

impl App {
    /// Start, stop or move the MCP server when its settings change. A failed
    /// start isn't retried until they change again.
    fn sync_mcp_server(&mut self) {
        let want = (self.settings.mcp_enabled, self.settings.mcp_port);
        if self.mcp_applied == Some(want) {
            return;
        }
        self.mcp_applied = Some(want);
        self.mcp = None;
        self.mcp_error = None;
        let (Some(ctx), (true, port)) = (self.egui_ctx.clone(), want) else { return };
        match McpServer::start(port, ctx) {
            Ok(server) => self.mcp = Some(server),
            Err(e) => self.mcp_error = Some(e),
        }
    }

    fn run_mcp_jobs(&mut self) {
        let jobs = self.mcp.as_ref().map(McpServer::pending).unwrap_or_default();
        for job in jobs {
            job(self);
        }
    }

    pub(crate) fn pattern(&self) -> &Pattern {
        &self.pattern
    }

    pub(crate) fn mcp_units(&self) -> Units {
        self.settings.units
    }

    fn mcp_fmt(&self) -> mcp::Fmt {
        mcp::Fmt { units: self.settings.units }
    }

    /// Apply agent edits to a copy of the pattern; on success it replaces the
    /// pattern as one undo step, on failure nothing changes.
    pub(crate) fn mcp_edit(&mut self, label: &str, edits: &[mcp::Edit]) -> Result<mcp::Outcome, String> {
        let env = mcp::EditEnv { units: self.settings.units, length_moves_start: self.settings.length_moves_start };
        let mut next = self.pattern.clone();
        // Agent input reaches core code that indexes freely; never take the app down with it.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mcp::apply_edits(edits, &mut next, &env)
        }))
        .unwrap_or_else(|_| Err("internal error while editing; nothing was changed".into()))?;
        self.checkpoint();
        self.pattern = next;
        self.after_history_change();
        self.status = format!("AI agent: {label}");
        Ok(outcome)
    }

    pub(crate) fn mcp_outcome(&self, out: &mcp::Outcome) -> serde_json::Value {
        mcp::outcome_json(self.mcp_fmt(), &self.pattern, out, self.settings.seam_tolerance)
    }

    pub(crate) fn mcp_overview(&self) -> serde_json::Value {
        mcp::overview_json(self.mcp_fmt(), &self.pattern, self.settings.seam_tolerance)
    }

    pub(crate) fn mcp_piece(&self, piece: &mcp::PieceRef) -> Result<serde_json::Value, String> {
        mcp::piece_json(self.mcp_fmt(), &self.pattern, piece)
    }

    pub(crate) fn mcp_selection(&self) -> serde_json::Value {
        serde_json::json!({
            "piece": self.selected_piece,
            "points": self.selected_points.iter().map(|r| serde_json::json!({"piece": r.piece, "point": r.point})).collect::<Vec<_>>(),
            "segments": self.selected_segments.iter().map(|&(piece, segment)| serde_json::json!({"piece": piece, "segment": segment})).collect::<Vec<_>>(),
            "seam": self.selected_seam,
        })
    }

    /// Undo (or redo); `false` if there was nothing to do.
    pub(crate) fn mcp_undo(&mut self, redo: bool) -> bool {
        let stack = if redo { &self.redo } else { &self.undo };
        if stack.is_empty() {
            return false;
        }
        if redo { self.redo() } else { self.undo() }
        self.status = format!("AI agent: {}", if redo { "redo" } else { "undo" });
        true
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SESSION_KEY, &self.session());
        self.save_settings();
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
        self.run_mcp_jobs();
        let frame_start = self.pattern.clone();
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            if self.palette.is_open() {
                self.palette.close();
            } else {
                self.palette.open();
            }
        }
        // While the palette is open, keys belong to it.
        if !self.palette.is_open() {
            self.handle_shortcuts(&ctx);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("side").default_size(230.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui));
        });
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewports(ui, &frame_start));
        self.settings_window(&ctx);
        self.sync_mcp_server();
        self.show_palette(&ctx);
        // Edits made after the canvas was painted (e.g. from the palette).
        if self.mirror_edits(&frame_start) {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_from_before_viewports_keeps_its_camera() {
        let old = "(pattern:(pieces:[],seams:[]),file:None,view:(center:(x:12.0,y:34.0),zoom:2.5))";
        let s: Session = ron::from_str(old).unwrap();
        let mut app = App::default();
        app.restore(s);
        assert_eq!(app.viewports.len(), 1);
        let view = app.viewports[0].camera().unwrap();
        assert_eq!((view.center.x, view.center.y, view.zoom), (12.0, 34.0, 2.5));
    }

    #[test]
    fn session_round_trips_viewports() {
        let mut app = App::default();
        app.split_view();
        let text = ron::to_string(&app.session()).unwrap();
        let mut restored = App::default();
        restored.restore(ron::from_str(&text).unwrap());
        assert_eq!(restored.viewports.len(), 2);
        assert_eq!(restored.active_viewport, 1);
    }

    fn keys(app: &App) -> Vec<&'static str> {
        app.hints().into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn hints_follow_the_sewing_steps() {
        let mut app = App { tool: Tool::Sew, ..App::default() };
        assert_eq!(app.hints()[0].1, "Start of first edge");
        let at = OutlinePos::new(0, 0.0);
        app.sew = SewState::FirstStart(0, at);
        assert_eq!(keys(&app), ["Click", "Esc", "Ctrl K"]);
        let a = EdgeSpan { piece: 0, start: at, end: OutlinePos::new(1, 0.0), forward: true };
        app.sew = SewState::SecondStart(a, 1, at);
        assert_eq!(keys(&app), ["Click", "Alt", "Esc", "Ctrl K"]);
        // Alt only bypasses the length snap, so it isn't offered without it.
        app.settings.snap_match = false;
        assert_eq!(keys(&app), ["Click", "Esc", "Ctrl K"]);
    }

    #[test]
    fn hints_while_typing_a_length() {
        let mut app = App { tool: Tool::Select, ..App::default() };
        app.selected_segments = vec![(0, 0)];
        assert_eq!(keys(&app)[0], "0–9");
        app.length_input = "3 1/2".into();
        assert_eq!(app.hints(), [("Enter", "Set length"), ("Backspace", "Delete digit"), ("Esc", "Cancel")]);
    }

    #[test]
    fn close_view_keeps_one() {
        let mut app = App::default();
        app.split_view();
        app.close_view();
        app.close_view();
        assert_eq!(app.viewports.len(), 1);
        assert_eq!(app.active_viewport, 0);
    }
}

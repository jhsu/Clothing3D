//! MCP server built into the desktop app, so AI agents can edit the open
//! pattern live. Serves Streamable HTTP on `127.0.0.1:<port>/mcp`; each tool
//! call is handed to the UI thread, applied as one undo step and repainted.

mod edit;
mod render;
mod report;

use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use eframe::egui;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::{self, JsonSchema};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::app::App;
pub use edit::{Edit, EditEnv, Outcome, PieceRef, apply_all as apply_edits};
pub use report::Fmt;

/// Work for the UI thread.
pub type Job = Box<dyn FnOnce(&mut App) + Send>;

/// How long a tool call waits for the UI thread. Frames stop while the
/// window is hidden on some compositors, so don't wait forever.
const UI_TIMEOUT: Duration = Duration::from_secs(10);

/// A running server; dropping it shuts the server down.
pub struct McpServer {
    pub port: u16,
    jobs: mpsc::Receiver<Job>,
    cancel: CancellationToken,
}

impl McpServer {
    pub fn start(port: u16, ctx: egui::Context) -> Result<Self, String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("MCP server couldn't listen on port {port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let (tx, jobs) = mpsc::channel();
        let cancel = CancellationToken::new();
        let tools = Tools { tool_router: Tools::tool_router(), jobs: tx, ctx };
        let config = StreamableHttpServerConfig::default()
            .enforce_origin_validation() // no browser pages
            .with_cancellation_token(cancel.clone());
        let service: StreamableHttpService<Tools, LocalSessionManager> =
            StreamableHttpService::new(move || Ok(tools.clone()), Default::default(), config);
        let shutdown = cancel.clone();
        std::thread::Builder::new()
            .name("mcp".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => return eprintln!("MCP runtime failed: {e}"),
                };
                rt.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(l) => l,
                        Err(e) => return eprintln!("MCP listener failed: {e}"),
                    };
                    let router = axum::Router::new().nest_service("/mcp", service);
                    let _ = axum::serve(listener, router)
                        .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
                        .await;
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { port, jobs, cancel })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// Requests waiting for the UI thread.
    pub fn pending(&self) -> Vec<Job> {
        self.jobs.try_iter().collect()
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

// ------------------------------------------------------------------ tools

#[derive(Clone)]
struct Tools {
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
    jobs: mpsc::Sender<Job>,
    ctx: egui::Context,
}

fn fail(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

fn ok_json(v: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(v.to_string())])
}

impl Tools {
    /// Run `f` on the UI thread with the app and wait for its result.
    async fn on_ui<R: Send + 'static>(&self, f: impl FnOnce(&mut App) -> R + Send + 'static) -> Result<R, ErrorData> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.jobs
            .send(Box::new(move |app| {
                let _ = tx.send(f(app));
            }))
            .map_err(|_| ErrorData::internal_error("clothes3d is shutting down", None))?;
        self.ctx.request_repaint();
        match tokio::time::timeout(UI_TIMEOUT, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(ErrorData::internal_error("clothes3d dropped the request", None)),
            Err(_) => Err(ErrorData::internal_error(
                "clothes3d didn't respond; its window may be minimized or hidden",
                None,
            )),
        }
    }

    async fn edit(&self, label: &'static str, edits: Vec<Edit>) -> Result<CallToolResult, ErrorData> {
        self.on_ui(move |app| match app.mcp_edit(label, &edits) {
            Ok(out) => ok_json(&app.mcp_outcome(&out)),
            Err(e) => fail(e),
        })
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetPiece {
    /// Piece index or exact name.
    piece: PieceRef,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Render {
    /// Only this piece (index or name); default: the whole pattern.
    #[serde(default)]
    piece: Option<PieceRef>,
    /// Label point indices (default true).
    #[serde(default = "yes")]
    point_labels: bool,
    /// Label segment indices as s0, s1, … (default true).
    #[serde(default = "yes")]
    segment_labels: bool,
    /// Draw seams with their index and start notch (default true).
    #[serde(default = "yes")]
    seams: bool,
    /// Longest image side in pixels, 256–2048 (default 1024).
    #[serde(default)]
    size: Option<u32>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Batch {
    /// Edits applied in order, each shaped like the matching tool's
    /// arguments plus `"op": "<tool name>"`. All succeed or none apply.
    edits: Vec<Edit>,
}

#[tool_router]
impl Tools {
    /// Overview of the open pattern: the units every length is in, each piece
    /// (index, name, closed, point/segment counts, perimeter, area, bounds)
    /// and every seam with both edge lengths and whether they match within
    /// the seam tolerance. Start here.
    #[tool(annotations(read_only_hint = true))]
    async fn get_pattern(&self) -> Result<CallToolResult, ErrorData> {
        self.on_ui(|app| ok_json(&app.mcp_overview())).await
    }

    /// One piece in full: points (x, y, kind turn/curve, Bézier handle
    /// offsets), segments (from/to point, length, curved) and the seams on it.
    /// Segment i runs from point i to point i+1 (the last closes the outline).
    #[tool(annotations(read_only_hint = true))]
    async fn get_piece(&self, Parameters(p): Parameters<GetPiece>) -> Result<CallToolResult, ErrorData> {
        self.on_ui(move |app| match app.mcp_piece(&p.piece) {
            Ok(v) => ok_json(&v),
            Err(e) => fail(e),
        })
        .await
    }

    /// What the user has selected in the editor (piece, points, segments,
    /// seam), so requests like "lengthen this edge" can be resolved.
    #[tool(annotations(read_only_hint = true))]
    async fn get_selection(&self) -> Result<CallToolResult, ErrorData> {
        self.on_ui(|app| ok_json(&app.mcp_selection())).await
    }

    /// PNG picture of the pattern (or one piece), Y up, with point indices,
    /// segment indices (s0, s1, …), piece names, seams (colored, labeled,
    /// with a tick at each edge's start) and a scale bar. Use it to check
    /// shapes after editing.
    #[tool(annotations(read_only_hint = true))]
    async fn render(&self, Parameters(p): Parameters<Render>) -> Result<CallToolResult, ErrorData> {
        let snapshot = self
            .on_ui(move |app| {
                let piece = p.piece.as_ref().map(|r| r.resolve(app.pattern())).transpose()?;
                Ok::<_, String>((
                    app.pattern().clone(),
                    render::RenderOptions {
                        piece,
                        point_labels: p.point_labels,
                        segment_labels: p.segment_labels,
                        seams: p.seams,
                        size: p.size.unwrap_or(1024),
                        units: app.mcp_units(),
                    },
                ))
            })
            .await?;
        let (pattern, opts) = match snapshot {
            Ok(s) => s,
            Err(e) => return Ok(fail(e)),
        };
        let png = tokio::task::spawn_blocking(move || render::render_png(&pattern, &opts))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(match png {
            Ok(png) => CallToolResult::success(vec![ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(png),
                "image/png",
            )]),
            Err(e) => fail(e),
        })
    }

    /// Add a piece from its outline points, in order. Returns its index.
    #[tool]
    async fn add_piece(&self, Parameters(p): Parameters<edit::AddPiece>) -> Result<CallToolResult, ErrorData> {
        self.edit("add piece", vec![Edit::AddPiece(p)]).await
    }

    /// Replace a piece's whole outline. Seams on the piece are removed.
    #[tool]
    async fn set_piece_points(
        &self,
        Parameters(p): Parameters<edit::SetPiecePoints>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit("replace outline", vec![Edit::SetPiecePoints(p)]).await
    }

    /// Delete a piece and its seams. Later pieces move down one index.
    #[tool(annotations(destructive_hint = true))]
    async fn remove_piece(&self, Parameters(p): Parameters<edit::PieceOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("remove piece", vec![Edit::RemovePiece(p)]).await
    }

    /// Copy a piece (without seams), placed to the right of the original.
    #[tool]
    async fn duplicate_piece(&self, Parameters(p): Parameters<edit::PieceOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("duplicate piece", vec![Edit::DuplicatePiece(p)]).await
    }

    #[tool(description = "Rename a piece.")]
    async fn rename_piece(&self, Parameters(p): Parameters<edit::RenamePiece>) -> Result<CallToolResult, ErrorData> {
        self.edit("rename piece", vec![Edit::RenamePiece(p)]).await
    }

    /// Flip, rotate (degrees counter-clockwise, about the piece center) and/or
    /// translate a piece, in that order. Seams follow.
    #[tool]
    async fn transform_piece(
        &self,
        Parameters(p): Parameters<edit::TransformPiece>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit("transform piece", vec![Edit::TransformPiece(p)]).await
    }

    /// Mirror a closed piece across a fold line made of straight, contiguous,
    /// collinear segments, merging both halves into one symmetric piece.
    #[tool]
    async fn unfold_piece(&self, Parameters(p): Parameters<edit::UnfoldPiece>) -> Result<CallToolResult, ErrorData> {
        self.edit("unfold piece", vec![Edit::UnfoldPiece(p)]).await
    }

    /// Move a point to an absolute position (`to`) and/or by an offset (`by`).
    #[tool]
    async fn move_point(&self, Parameters(p): Parameters<edit::MovePoint>) -> Result<CallToolResult, ErrorData> {
        self.edit("move point", vec![Edit::MovePoint(p)]).await
    }

    /// Insert a point on a segment at parameter t (0..1) without changing the
    /// outline's shape. Later points shift up one index; seams stay attached.
    #[tool]
    async fn insert_point(&self, Parameters(p): Parameters<edit::InsertPoint>) -> Result<CallToolResult, ErrorData> {
        self.edit("insert point", vec![Edit::InsertPoint(p)]).await
    }

    /// Delete a point; its two segments merge. Later points shift down one index.
    #[tool(annotations(destructive_hint = true))]
    async fn remove_point(&self, Parameters(p): Parameters<edit::PointOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("remove point", vec![Edit::RemovePoint(p)]).await
    }

    /// Make a point a sharp corner (`turn`) or a smooth curve point (`curve`).
    #[tool]
    async fn set_point_kind(&self, Parameters(p): Parameters<edit::SetPointKind>) -> Result<CallToolResult, ErrorData> {
        self.edit("set point kind", vec![Edit::SetPointKind(p)]).await
    }

    /// Set a point's Bézier handles (offsets from the point) to shape the
    /// curves on either side of it.
    #[tool]
    async fn set_handles(&self, Parameters(p): Parameters<edit::SetHandles>) -> Result<CallToolResult, ErrorData> {
        self.edit("set handles", vec![Edit::SetHandles(p)]).await
    }

    /// Remove a point's explicit handles; curve points go back to automatic smoothing.
    #[tool]
    async fn clear_handles(&self, Parameters(p): Parameters<edit::PointOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("clear handles", vec![Edit::ClearHandles(p)]).await
    }

    /// Set a segment's length by sliding one end point along the chord.
    /// Curved segments are solved for arc length.
    #[tool]
    async fn set_segment_length(
        &self,
        Parameters(p): Parameters<edit::SetSegmentLength>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit("set segment length", vec![Edit::SetSegmentLength(p)]).await
    }

    /// Set both control handles of a segment (cubic Bézier), as offsets from
    /// its start and end points. Smooth neighbors stay smoothly joined.
    #[tool]
    async fn set_segment_handles(
        &self,
        Parameters(p): Parameters<edit::SetSegmentHandles>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit("set segment handles", vec![Edit::SetSegmentHandles(p)]).await
    }

    /// Bend a segment so the spot at parameter t moves by `by`, like dragging
    /// the edge in the editor. Straight segments become curves.
    #[tool]
    async fn bend_segment(&self, Parameters(p): Parameters<edit::BendSegment>) -> Result<CallToolResult, ErrorData> {
        self.edit("bend segment", vec![Edit::BendSegment(p)]).await
    }

    #[tool(description = "Make a segment straight.")]
    async fn straighten_segment(
        &self,
        Parameters(p): Parameters<edit::SegmentOnly>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit("straighten segment", vec![Edit::StraightenSegment(p)]).await
    }

    /// Sew two edges together. Each edge is a whole segment, or a stretch of
    /// outline from `start` to `end` (each a point, or a segment + t). The
    /// starts of the two edges are sewn to each other. Returns the seam and
    /// both lengths; compare them to check the edges match.
    #[tool]
    async fn add_seam(&self, Parameters(p): Parameters<edit::AddSeam>) -> Result<CallToolResult, ErrorData> {
        self.edit("add seam", vec![Edit::AddSeam(p)]).await
    }

    /// Delete a seam. Later seams move down one index.
    #[tool(annotations(destructive_hint = true))]
    async fn remove_seam(&self, Parameters(p): Parameters<edit::SeamOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("remove seam", vec![Edit::RemoveSeam(p)]).await
    }

    /// Reverse a seam's second edge, swapping which ends are sewn together.
    #[tool]
    async fn flip_seam(&self, Parameters(p): Parameters<edit::SeamOnly>) -> Result<CallToolResult, ErrorData> {
        self.edit("flip seam", vec![Edit::FlipSeam(p)]).await
    }

    /// Apply several edits as one undo step; if any fails, none apply.
    /// Indices in later edits see the effect of earlier ones.
    #[tool]
    async fn batch(&self, Parameters(p): Parameters<Batch>) -> Result<CallToolResult, ErrorData> {
        self.edit("batch edit", p.edits).await
    }

    /// Undo the last change (the agent's or the user's).
    #[tool]
    async fn undo(&self) -> Result<CallToolResult, ErrorData> {
        self.on_ui(|app| if app.mcp_undo(false) { ok_json(&app.mcp_overview()) } else { fail("nothing to undo") })
            .await
    }

    /// Redo the last undone change.
    #[tool]
    async fn redo(&self) -> Result<CallToolResult, ErrorData> {
        self.on_ui(|app| if app.mcp_undo(true) { ok_json(&app.mcp_overview()) } else { fail("nothing to redo") })
            .await
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "clothes3d",
    instructions = "Live control of the clothes3d sewing-pattern editor; the user sees every change \
immediately and can undo it. Lengths and coordinates use the user's display units (named in every \
reply as \"units\"); bare numbers are in those units, and strings may name a unit: \"90mm\", \"9 cm\", \
\"3 1/2 in\". Y points up. Pieces are outlines of turn (corner) and curve (smooth) points; segment i \
runs from point i to point i+1. Call get_pattern first, render to look at shapes, and batch for \
multi-step changes. Indices shift after inserting or removing points, pieces and seams; every edit \
reply lists the updated pieces."
)]
impl rmcp::ServerHandler for Tools {}

/// The reply for a successful edit: what was created, the updated pieces in
/// full, and seams on them that no longer match.
pub fn outcome_json(app_units: Fmt, pattern: &crate::model::Pattern, out: &Outcome, tolerance: f64) -> Value {
    let mut v = json!({ "units": app_units.name() });
    if let Some((kind, index)) = out.created {
        v["created"] = json!({ kind: index });
    }
    if !out.notes.is_empty() {
        v["notes"] = out.notes.clone().into();
    }
    v["pieces"] =
        out.touched.iter().map(|&i| report::piece_detail(app_units, pattern, i)).collect::<Vec<_>>().into();
    if let Some(("seam", i)) = out.created {
        v["seam"] = report::seam(app_units, pattern, i, tolerance);
    }
    let warnings = report::seam_warnings(app_units, pattern, &out.touched, tolerance);
    if !warnings.is_empty() {
        v["mismatched_seams"] = warnings.into();
    }
    v
}

pub fn overview_json(f: Fmt, pattern: &crate::model::Pattern, tolerance: f64) -> Value {
    report::overview(f, pattern, tolerance)
}

pub fn piece_json(f: Fmt, pattern: &crate::model::Pattern, piece: &PieceRef) -> Result<Value, String> {
    let i = piece.resolve(pattern)?;
    let mut v = report::piece_detail(f, pattern, i);
    v["units"] = f.name().into();
    Ok(v)
}


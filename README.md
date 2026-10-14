# clothes3d

A Rust pattern editor for drafting and sewing garment patterns.
Phase 1: a 2D pattern viewport for drafting pieces with points.

## Run

    cargo run --release -- [pattern.json]

Try `cargo run -- examples/bodice.json` or `examples/sewing.json`.

The repo is a Cargo workspace; the desktop app is the root package, so plain
`cargo run` / `cargo build` target it. `cargo test --workspace` runs every
crate's tests.

The working session (pattern, seams, view, current file) is saved to
`~/.local/share/clothes3d/app.ron` on exit and every 5 seconds, and restored on
the next launch. A file passed on the command line takes precedence.

Application settings (units, inch fractions or decimals, length labels, snapping, seam tolerance, segment
edit behavior) are separate from the pattern: edit them in Edit → Settings…;
they're saved to `$XDG_CONFIG_HOME/clothes3d/settings.json`
(default `~/.config/clothes3d/settings.json`).

## Controls

| Action | Input |
|---|---|
| Hint bar | The bottom bar shows what clicks and keys do right now for the current tool and step (e.g. while sewing or typing a length); status messages replace it until the next click. On the right, GRID / SNAP / LENGTHS / SA toggle grid snap, point snap, length labels and seam allowance, and the units chip switches mm / in |
| Command palette | Ctrl+K — fuzzy-find any tool or command (↑/↓ to pick, Enter to run, Esc to close). Commands that need a selection are shown greyed out |
| Segment length labels | L (or View → Segment lengths) — label every segment with its length |
| Units | View → Units, Settings, or the palette: millimetres or inches. Typed lengths use the current unit and accept fractions (`3 1/2`, `3/8`); the grid switches to inch steps. Inch lengths show as fractions to 1/16 by default; Settings → Inch lengths picks 1/8, 1/32 or decimals |
| Remove curvature | Shift+C on selected segments, double-click an edge with the Edit curve tool, right-click an edge → Remove curvature, or the Segments list button |
| Right-click menu | right-click an edge: Remove curvature, Insert point here, Select segment |
| Zoom at cursor | scroll wheel / pinch |
| Pan | middle- or right-drag |
| Fit to pattern | F |
| Split view | View → Split view (or the palette) opens another window onto the pattern beside the active one, with its own pan and zoom; selection and tools are shared. View → Close view removes the active one. F and the zoom readout apply to the viewport under the cursor |
| Select tool | V — drag a point to move it, drag a piece (fill or outline) to move the piece, double-click outline to insert point |
| Nudge selected piece | arrow keys (one grid step), Shift+arrow (1 mm) |
| Select a segment | click an edge with the Select tool (or pick it in the side panel's Segments list) |
| Box select | with the Select (or Edit curve) tool, drag on empty canvas to select the points inside the rectangle; Ctrl+drag starts a box even over a piece; Shift adds to the selection |
| Multi-select | Shift+click points or edges (also in the Segments list) to add/remove them. Dragging any selected point moves them all; arrows nudge them; C toggles turn/curve on all; Del deletes them |
| Set segment length | with a segment selected, type a length + Enter, or edit it in the Segments list. Choose whether edits move the end or start point. Curved segments are solved for arc length |
| Place point at a length | while drawing, type a length + Enter: the next point goes that far from the last one, toward the cursor |
| Rotate piece | R (90° clockwise), Shift+R (90° counter-clockwise), or "Rotate by" any angle in the side panel |
| Flip piece | H (horizontal), Shift+H (vertical) |
| Duplicate piece | Ctrl+D |
| Unfold (mirror) piece | select a straight edge of a closed piece — Shift+click to select several connected, collinear segments — and press U: the piece is mirrored across that line and merged into one symmetric piece (points inside the fold line are dropped) |
| Draw tool | P — click to place points, click first point / Enter to close, Esc to stop |
| Add point tool | A — click an outline to insert a point (on curves too) |
| Free sewing tool | S — click start + end on one edge, then start + end on the matching edge. Starts are marked with a perpendicular notch and are sewn to each other; dotted lines connect the pair. Esc cancels. Flip/delete seams from the side panel |
| Seam length matching | While placing the second edge, a readout shows A/B lengths and Δ (green within tolerance, amber outside). Green diamonds mark where B's length equals A's; the end click snaps to them (hold Alt to place freely). Seams outside the tolerance (Sewing → Tolerance, default 1 mm) get a ⚠ badge on the canvas and in the seam list |
| Edit curve tool | E — drag any edge (straight or curved) to bend it through the cursor |
| Bézier handles | select points or edges (Select or Edit curve tool) to show their handles; drag a handle to shape the curve. Smooth (curve) points mirror their handles; hold Alt to break them into a corner. Dashed handles are automatic and become explicit when dragged. "Clear handles" (point panel) and "Straighten selected" (Segments list) undo curvature |
| Toggle turn/curve point | C |
| Delete point (or piece) | Del / Backspace |
| Toggle grid snap | G |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z, Ctrl+Y |
| Open / Save | Ctrl+O / Ctrl+S (JSON) |
| Seam allowance | Outlines are sew lines; allowance adds a cut line outside them. Select a closed piece and click **Add** next to "Seam allowance" (default width in Settings), then type or drag to change it for the whole piece. Give a segment its own width in the Segments list's Allowance column (0 for none, e.g. on a fold; ↺ returns it to the piece's), or select several segments and set them together. View → Seam allowance hides the cut line |
| Export DXF (AAMA/ASTM) | File → Export DXF… (or the command palette) — pieces with seam allowance export the cut line on layer 1 and the sew line on layer 14 |

## AI agents (MCP)

While the app runs it serves an [MCP](https://modelcontextprotocol.io) server
at `http://127.0.0.1:7333/mcp` (Streamable HTTP), so agents such as Codex,
opencode, Claude Code or t3code can edit the open pattern. Changes show up
live, each tool call is one undo step, and the status bar names the agent's
last edit. Turn it off or change the port in Edit → Settings… → AI agents.
Only local connections are accepted, and requests from web pages are refused.

Codex (`~/.codex/config.toml`):

```toml
[mcp_servers.clothes3d]
url = "http://127.0.0.1:7333/mcp"
```

opencode (`opencode.json`):

```json
{ "mcp": { "clothes3d": { "type": "remote", "url": "http://127.0.0.1:7333/mcp" } } }
```

Claude Code: `claude mcp add --transport http clothes3d http://127.0.0.1:7333/mcp`.
t3code uses the MCP servers configured for the agent it runs.

Lengths use your display units (mm or inches), and every reply names them.
Agents can also spell out a unit: `"90mm"`, `"9 cm"`, `"3 1/2 in"`.

| Tool | Does |
|---|---|
| `get_pattern` | Pieces (size, perimeter, area, bounds) and seams with both edge lengths and tolerance check |
| `get_piece` | One piece's points, handles and segment lengths |
| `get_selection` | What you've selected, so "make this edge longer" works |
| `render` | PNG of the pattern or a piece, labeled with point / segment / seam indices and a scale bar |
| `add_piece`, `set_piece_points`, `remove_piece`, `duplicate_piece`, `rename_piece` | Pieces |
| `transform_piece`, `unfold_piece` | Flip / rotate / move; mirror across a fold |
| `move_point`, `insert_point`, `remove_point`, `set_point_kind` | Points |
| `set_handles`, `clear_handles`, `set_segment_handles`, `bend_segment`, `straighten_segment` | Curves |
| `set_segment_length` | Exact lengths (arc length on curves) |
| `add_seam`, `remove_seam`, `flip_seam` | Sewing |
| `batch` | Several edits as one undo step; all or nothing |
| `undo`, `redo` | History |

Pieces can be named by index or name. Every edit replies with the updated
pieces and any seams on them that no longer match. The window has to be
open (not minimized) for tool calls to be answered.

## Model

Units are millimetres, Y up (matches DXF). Each piece is a polyline of
**turn points** (corners) and **curve points** (smoothly interpolated), which maps
directly onto DXF-AAMA/ASTM boundary layers (1 = boundary, 2 = turn points,
3 = curve points).

### DXF-AAMA/ASTM export

File → Export DXF writes an ASCII DXF R12 file in the AAMA/ASTM D6673 layout:
one BLOCK per closed piece (inserted at the origin, so the layout is kept)
with the boundary POLYLINE and "Piece Name" / "Quantity" text on layer 1,
turn points on layer 2 and curve points on layer 3. Header text carries the
style name (the file name) and `Units: METRIC` (coordinates in mm).

Curved (Bézier) segments are densified: the sampled in-between vertices sit
on the boundary without a turn/curve marker. Open pieces are skipped and
listed in the status bar.

For a piece with seam allowance, the boundary on layer 1 is the cut line,
with its corners (turns over 10°) as turn points, and the outline goes on
layer 14 as the sew line.

#### Known shortcomings

Not yet validated in commercial pattern CAD (Gerber AccuMark, Optitex,
Lectra, Valentina, …); output has only been checked with the `ezdxf` reader.

Geometry
- Curves are sampled at a fixed 32 points per curved segment rather than to
  a tolerance, so files are larger than needed and very long curves may be
  coarse.
- Bézier handles are lost: the outline shape is kept, but re-importing would
  give sampled points, not editable curves. Readers that re-smooth unmarked
  vertices may shift curves slightly.
- A turn point with handles (smooth-looking but marked as a corner) is
  exported as a turn point.
- Outline winding direction is not normalized, and self-intersecting
  outlines are not detected.
- The outline is exported as-is on layer 1 (cut line); there is no seam
  allowance and no separate sew line (layer 14).

Units and header
- `Units: METRIC` with coordinates in millimetres is assumed to be what
  readers expect; this is unverified. There is no `ENGLISH` (inch) option,
  and the app's mm/inch display setting is ignored.
- No separate AAMA vs ASTM flavor: one layout is written, without version,
  author, creation date/time or other ASTM-specific header text.
  "Sample Size" and "Grading Rule Table" are written empty.

Piece data
- Quantity is always 1; no size, category, material or fabric annotations.
- Text height is a fixed 5 mm and labels sit at the bounding-box center,
  which may fall outside concave pieces.
- Block names are reduced to ASCII letters, digits, `-` and `_` (and made
  unique); the original name is kept in the "Piece Name" text, with
  non-ASCII characters written as `\U+XXXX` escapes.

Not exported
- Seams / sewing information (AAMA has no equivalent).
- Notches (layer 4), mirror/fold lines (6), grain lines (7), internal lines
  (8), drill holes (13), sew lines (14).
- Open pieces (could become internal lines) and pieces with fewer than three
  points.
- Grading / multiple sizes.

DXF import is not implemented.

Seams store each edge as positions along a piece outline (segment + parameter),
so they follow the outline as points and pieces move.

## Web / WebAssembly

The pattern model and sewing logic live in `crates/clothes3d-core`, which has
no UI or I/O dependencies. `crates/clothes3d-wasm` wraps it with
`wasm-bindgen` as a `PatternDoc` class for use in the browser:

    rustup target add wasm32-unknown-unknown
    wasm-pack build crates/clothes3d-wasm --target web   # → crates/clothes3d-wasm/pkg

```js
import init, { PatternDoc } from "./pkg/clothes3d_wasm.js";
await init();
const doc = PatternDoc.fromJson(await (await fetch("sewing.json")).text());
const outline = doc.flatten(0);   // Float64Array [x0, y0, x1, y1, ...] in mm, Y up
const mesh = doc.fillMesh(0);     // { vertices: Float64Array, indices: Uint32Array }
doc.insertPoint(0, 2, 0.5);       // seams stay attached
const json = doc.toJson();        // same format as the desktop app's files
const { text, skipped } = doc.toAamaDxf("style"); // DXF-AAMA/ASTM file contents
```

Use `--target bundler` for webpack/Vite or `--target nodejs` for Node. Index
arguments are validated and throw on out-of-range values.

## Layout

- `src/` — desktop app (root package)
  - `main.rs` — entry point
  - `app.rs` — egui app: tools, viewport painting, panels, undo, file dialogs
  - `view.rs` — 2D camera (world mm ↔ screen px)
  - `viewport.rs` — viewports: each window onto the pattern and its camera
  - `theme.rs` — colors, egui visuals and fonts
  - `widgets.rs` — custom-drawn widgets: tool dock, section headers, seam rows, key caps
  - `mcp/` — MCP server: tools (`mod.rs`), edits (`edit.rs`), replies
    (`report.rs`), PNG rendering (`render.rs`)
- `crates/clothes3d-core/` — platform-independent core
  - `model.rs` — pattern / piece / point data, curve evaluation, hit-testing
  - `sewing.rs` — free sewing: seam spans, outline walking, seam-preserving edits
  - `dxf.rs` — DXF-AAMA/ASTM export
- `crates/clothes3d-wasm/` — `wasm-bindgen` bindings over the core
- `assets/fonts/` — bundled fonts, each with its license

## License

Clothes3D is released under the [MIT License](LICENSE).

The bundled fonts are not covered by the MIT License. They are unmodified
official releases, distributed under the
[SIL Open Font License 1.1](https://openfontlicense.org):

| Font | Used for | Copyright | License |
|---|---|---|---|
| [Inter Tight](https://github.com/rsms/inter) | interface text | © 2022 The Inter Project Authors | [OFL](assets/fonts/inter-tight/OFL.txt) |
| [IBM Plex Mono](https://github.com/IBM/plex) | measurements | © 2017 IBM Corp., Reserved Font Name "Plex" | [OFL](assets/fonts/ibm-plex-mono/OFL.txt) |

They are compiled into the app binary; each font file also carries its
copyright and license in its own metadata.

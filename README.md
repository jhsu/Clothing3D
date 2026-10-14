# clothes3d

A Rust desktop pattern editor, aiming toward basic CLO3D-style functionality.
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

Application settings (units, length labels, snapping, seam tolerance, segment
edit behavior) are separate from the pattern: edit them in Edit → Settings…;
they're saved to `$XDG_CONFIG_HOME/clothes3d/settings.json`
(default `~/.config/clothes3d/settings.json`).

## Controls

| Action | Input |
|---|---|
| Command palette | Ctrl+K — fuzzy-find any tool or command (↑/↓ to pick, Enter to run, Esc to close). Commands that need a selection are shown greyed out |
| Segment length labels | L (or View → Segment lengths) — label every segment with its length |
| Units | View → Units, Settings, or the palette: millimetres or inches. Typed lengths use the current unit and accept fractions (`3 1/2`, `3/8`); the grid switches to inch steps |
| Remove curvature | Shift+C on selected segments, double-click an edge with the Edit curve tool, right-click an edge → Remove curvature, or the Segments list button |
| Right-click menu | right-click an edge: Remove curvature, Insert point here, Select segment |
| Zoom at cursor | scroll wheel / pinch |
| Pan | middle- or right-drag |
| Fit to pattern | F |
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

## Model

Units are millimetres, Y up (matches DXF). Each piece is a polyline of
**turn points** (corners) and **curve points** (smoothly interpolated), which maps
directly onto DXF-AAMA/ASTM boundary layers (1 = boundary, 2 = turn points,
3 = curve points). DXF import/export is planned next.

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
```

Use `--target bundler` for webpack/Vite or `--target nodejs` for Node. Index
arguments are validated and throw on out-of-range values.

## Layout

- `src/` — desktop app (root package)
  - `main.rs` — entry point
  - `app.rs` — egui app: tools, viewport painting, panels, undo, file dialogs
  - `view.rs` — 2D camera (world mm ↔ screen px)
- `crates/clothes3d-core/` — platform-independent core
  - `model.rs` — pattern / piece / point data, curve evaluation, hit-testing
  - `sewing.rs` — free sewing: seam spans, outline walking, seam-preserving edits
- `crates/clothes3d-wasm/` — `wasm-bindgen` bindings over the core

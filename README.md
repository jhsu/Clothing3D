# clothes3d

A Rust desktop pattern editor, aiming toward basic CLO3D-style functionality.
Phase 1: a 2D pattern viewport for drafting pieces with points.

## Run

    cargo run --release -- [pattern.json]

Try `cargo run -- examples/bodice.json` or `examples/sewing.json`.

The working session (pattern, seams, view, snap/sewing settings, current file)
is saved to `~/.local/share/clothes3d/app.ron` on exit and every 5 seconds, and
restored on the next launch. A file passed on the command line takes precedence.

## Controls

| Action | Input |
|---|---|
| Zoom at cursor | scroll wheel / pinch |
| Pan | middle- or right-drag |
| Fit to pattern | F |
| Select tool | V — drag a point to move it, drag a piece (fill or outline) to move the piece, double-click outline to insert point |
| Nudge selected piece | arrow keys (one grid step), Shift+arrow (1 mm) |
| Select a segment | click an edge with the Select tool (or pick it in the side panel's Segments list) |
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

## Layout

- `src/model.rs` — pattern / piece / point data, curve evaluation, hit-testing
- `src/sewing.rs` — free sewing: seam spans, outline walking, seam-preserving edits
- `src/view.rs` — 2D camera (world mm ↔ screen px)
- `src/app.rs` — egui app: tools, viewport painting, panels

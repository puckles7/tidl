# Notes free-form grid layout + auto-layout

Replace the Notes grid's auto-reflowing masonry/linear order (#42/#43) with
an explicit per-note `(row, col)` grid position: removing a note leaves a
gap instead of everything shifting up, notes can be dragged into any empty
cell regardless of what's nearby, and a new "Auto-layout" button repacks
everything on demand.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — extends the `notes` table and grid UI
  introduced there.
- [42 - Note ordering — persisted manual order + pinned-group
  invariant](42-note-order-pinned-invariant.md) — this replaces its
  `sort_order` model entirely, but keeps its pinned/non-pinned group
  invariant.
- [43 - Drag-and-drop note reordering](43-note-drag-reorder.md) — this
  replaces its reflow-based drag entirely (see that file's closing note).

## Functional Requirements

- Each note has an explicit `(grid_row, grid_col)` position, independent
  per pinned/non-pinned zone. Removing a note leaves its cell empty — no
  automatic reflow to close the gap.
- Dragging a note onto an empty cell just moves it there.
- Dragging a note onto an occupied cell displaces the occupant to the
  nearest free cell (searching outward through occupied neighbors),
  cascading further if that neighbor is also occupied — minimizing the
  total number of notes moved.
- A note can never move across the pinned/non-pinned boundary via drag
  (existing #42 invariant, enforced server-side).
- An "Auto-layout" button repacks each zone tightly (no gaps), pinned zone
  still rendered above non-pinned.
- Fixed column count per zone (not viewport-responsive) — narrow viewports
  scroll horizontally rather than reflowing column count, so stored
  positions stay meaningful regardless of window size.

## Open Questions

- None — column count, displacement algorithm, and auto-layout behavior
  were confirmed with the user before implementation; see the resolution
  for the final numbers/algorithm chosen.

## Resolution

Implemented as specified. Both zones (pinned and non-pinned) got their own
independent grid, each `NOTES_GRID_COLS` (4, `crates/tidl-web/src/types.rs`)
columns wide.

- **Schema**: migration `20260715000000_note_grid_position.sql` adds
  `notes.grid_row`/`grid_col` (`INTEGER NOT NULL DEFAULT 0`), backfills by
  packing every note into a 4-column grid in its prior `sort_order`, then
  drops `sort_order` entirely along with every helper/server function
  built on it (`top_of_group_sort_order`, `reorder_sort_order`,
  `needs_renumber`, `renumber_group`, `neighbor_sort_order`,
  `move_to_top_of_group`, `reorder_note` — replaced by `first_gap`,
  `place_in_first_gap`, `apply_positions`, `move_notes`,
  `auto_layout_notes` in `api/notes.rs`).
- **Deviation from the approved plan**: dropped the plan's
  `UNIQUE(pinned, grid_row, grid_col)` index. Caught during
  implementation: collections are scoped per owner, and two unrelated
  users' notes (different owners, no shared collection) would routinely
  land on the same coordinate — everyone naturally starts near `(0,0)` —
  without ever being displayed together, so a table-wide constraint would
  turn a harmless overlap into a hard write failure for a bystander user.
  Collision-avoidance instead happens in application code
  (`first_gap`/`resolve_drop`/`move_notes`), scoped to whatever a single
  viewer can actually see via the same access-filtered query `list_notes`
  already uses — the only scope where a collision would ever be visible.
- **Displacement**: `resolve_drop` in `notes/mod.rs` — BFS outward from
  the drop target through occupied cells (left/right checked before
  up/down, so an equally-short sideways displacement is preferred over
  bumping something to a different row), stopping at the first free
  cell; every note on that shortest path shifts one step toward the
  target. Live preview computed the same way client-side
  (`drag_overlay`), applied via one `move_notes` batch on drop.
- **Rendering**: both grids switched from masonry/auto-fill to real CSS
  Grid with explicit per-note `grid-row`/`grid-column` (reusing the
  `align-items: start` technique from #45's second follow-up, so a short
  card next to a tall row-mate still keeps its own height, at the cost of
  visible empty space below it — accepted along with plain empty cells as
  part of moving off dense packing). The "+ Take a note…" tile moved to
  render *outside* the grid (a note's own position can legitimately be
  `(0,0)`, which would otherwise collide with an implicitly-placed first
  child) — a minor visual deviation from the original inline-tile layout,
  not called out to the user in advance.
- **Narrow viewports**: grids scroll horizontally (`overflow-x: auto`)
  rather than reflowing column count, per the confirmed fixed-column
  decision.
- **A real bug found via the Playwright verification below, not
  anticipated in the plan**: dragging into empty space far from where the
  drag started (a common case now — the whole point of "move notes to
  spaces without surrounding notes") could carry the pointer outside
  every element with a `pointermove`/`pointerup` listener, since pointer
  capture was never set — the drag would silently freeze with no
  `pointerup` ever received. Fixed by calling `setPointerCapture` on the
  card at pickup (`pointer_down_geometry`), which keeps `pointermove`/
  `pointerup` bubbling to `.notes-main`'s listeners regardless of where
  the cursor physically is.
- Verified with an isolated Playwright script (own `TidlServer`, own temp
  DB/port, `DISABLE_SERVER_FN_HASH=1` build in a scratch copy of the repo
  — cleaned up afterward, never touched the developer's live
  `cargo leptos watch` or its `target/`): drop on an empty cell moves only
  the dragged note; drop on an occupied cell displaces exactly the
  occupant with no collisions; Auto-layout repacks tightly into the first
  N cells of the 4-column grid with pinned still rendered first. Also
  `cargo test --workspace` (328 tests across the workspace, including new
  unit tests for `cell_at`, `resolve_drop`, and `first_gap`), `cargo
  clippy --workspace -- -D warnings`, `cargo fmt`, and
  `cargo build --target wasm32-unknown-unknown -p tidl-web --features
  hydrate` — all clean.

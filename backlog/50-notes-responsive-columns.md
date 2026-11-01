# Notes grid — responsive column count (no horizontal scrollbar)

The Notes grid (`crates/tidl-web/src/components/notes/mod.rs`) currently renders a
fixed `NOTES_GRID_COLS` (4) columns at a fixed 260px width — documented as
intentionally not viewport-responsive (`crates/tidl-web/src/types.rs:225-231`,
`assets/style.css:3918-3932`), so narrow viewports scroll horizontally instead of
reflowing. In practice this means small/narrow windows get a horizontal
scrollbar and notes can end up requiring scrolling to reach. This wish makes the
number of *rendered* columns responsive to available width, while keeping every
note's persisted position exactly as precise as it is today.

## Dependencies

- [47 - Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md) — the drag/drop cascade and free-form (row, col) position model this builds on
- [48 - Notes grid — variable-height masonry via row-span units](48-notes-grid-row-span-units.md) — the row-unit/span mechanics reused for per-column offset stacking

## Context / Background

Each note's `grid_row`/`grid_col` (`NoteInfo`/`NotePosition` in
`crates/tidl-web/src/types.rs`) is a real, persisted, exact position — not just a
CSS hint. Drag-and-drop (`resolve_drop`/`stack_column`/`column_at_cursor` in
`components/notes/mod.rs`) computes and persists exact cascades within a single
real column. Any responsive design has to preserve this: resizing the window
must never silently rewrite anyone's stored position, and dragging at a narrow
width must resolve to the same precise, gap-preserving semantics dragging has
today at full width — not a lossy reflow.

Two rejected approaches, for the record:
- **Live "current column count is the stored truth", repack-on-resize** — every
  resize crossing a breakpoint would repack and persist all notes into the new
  column count. Rejected: resizing alone must never rewrite anything.
- **Masonry reflow into N virtual columns (bin-pack by shortest column) for
  narrow rendering, with a whole-zone repack-and-persist on drop** — display is
  lossless (never persisted merely by resizing), but a drop while narrow would
  discard existing gaps/exact ordering in favor of a tight repack of the whole
  zone. Rejected in favor of the column-grouping approach below, which keeps
  full fidelity — deferred as a "smarter later" idea only if grouping proves
  insufficient.

## Functional Requirements

- Compute a live **effective column count** `cols` (`1..=NOTES_GRID_COLS`) from
  the measured width of the notes grid's container (not viewport width — the
  sidebar collapses at 767px and changes available width independently), via a
  `NodeRef` + window resize listener. Formula: `floor((width + GAP) / (CARD_WIDTH
  + GAP))`, clamped to `[1, NOTES_GRID_COLS]`.
- `.notes-pinned-row` / `.notes-grid`'s `grid-template-columns` becomes
  `repeat({cols}, 260px)`, driven by this live value instead of the current
  hardcoded `repeat(4, 260px)`. Column *width* stays fixed at 260px always —
  only the *count* changes. `overflow-x: auto` is replaced with `overflow-x:
  hidden` as a defensive guard (should never trigger given correct `cols` math).
- **Stored positions are never rewritten by resizing alone.** `NOTES_GRID_COLS`
  stays a constant; `api/notes.rs` (`place_in_first_gap`, `auto_layout_notes`,
  `create_note`, `update_note`, `toggle_note_pin`) require **no changes**.
- **Rendering below `NOTES_GRID_COLS` columns** ("compact mode") is a pure,
  ephemeral, client-only computation, redone every render, never persisted:
  - Group the `NOTES_GRID_COLS` real columns into `cols` virtual columns via
    `v = (real_col * cols) / NOTES_GRID_COLS` (integer division) — a
    deterministic, contiguous, order-preserving grouping (e.g. `cols=2` →
    real columns `{0,1}` render as virtual column 0, `{2,3}` as virtual column
    1; `cols=1` → all four merge into one).
  - Within a virtual column, each real column in its group is stacked directly
    beneath the previous one: real column `c`'s notes get a rendering offset
    equal to the summed occupied height (`max(row + span)`) of every earlier
    real column in the same group. This reuses the existing row-unit/span
    model (backlog #48) — no new packing algorithm, just an additive offset.
    The result is visually continuous (no gap or overlap at the boundary
    between grouped columns) — indistinguishable from a native single column,
    because the offset math is the same running-total masonry math a column
    already uses internally between its own notes. The only real consequence
    is *ordering*: all of real column 0's notes render before any of real
    column 1's, even if a column-1 note is newer — grouped by column identity
    first, then row within it.
  - The pinned zone's add-tile reservation (`NOTES_ADD_TILE_ROW_SPAN`,
    `reserved_floor`) is unaffected: real column 0 always maps to virtual
    column 0 (the formula gives `v=0` for `real_col=0` at any `cols`), so the
    tile keeps rendering there exactly as today.
- **Dragging in compact mode retains full free-form fidelity** — same
  precision as full-width dragging, not a reflow-and-repack:
  - Pointer X picks the virtual column (via the existing `column_at_cursor`,
    parameterized by `cols` instead of the `NOTES_GRID_COLS` constant) → which
    group of real columns.
  - Pointer Y, adjusted for each grouped real column's rendering offset, picks
    the specific real column and the local row within it.
  - The existing per-column cascade (`stack_column`/`resolve_drop`) then runs
    completely unchanged, scoped to that one real column.
  - Persistence is the existing `move_notes([NotePosition])` call, unchanged in
    shape — drops always resolve to an exact real `(row, col)` in true
    `NOTES_GRID_COLS`-column space.
  - Dragging across the boundary between two real columns grouped into the
    same virtual column (pointer Y crosses the offset boundary) or across
    virtual columns (pointer X crosses a virtual column boundary) both retarget
    the real column the same way a full-width drag retargets on crossing a
    column boundary today.
- At `cols == NOTES_GRID_COLS` (the common desktop case), rendering and
  dragging are **byte-for-byte unchanged** from today — the grouping formula
  degenerates to the identity mapping (`v = real_col`), and no offset is ever
  applied.

## Testing

- Unit tests (no wasm/DOM needed) for the pure functions: the `cols` formula
  (width → column count, clamped), the column-grouping formula (`real_col,
  cols` → virtual column, boundary cases at each `cols` value 1..4), and the
  per-column offset computation given a set of `(row, span)` entries.
- Manual verification (per CLAUDE.md's UI testing rule): resize the Notes tab
  through each column-count breakpoint and confirm no horizontal scrollbar
  ever appears; drag a note at each width and confirm the resulting persisted
  position is exact (survives a page reload) both at that width and after
  resizing back up to full width.

## Open Questions

- None outstanding — design discussed and confirmed. A smarter mechanism that
  avoids even the column-identity-based render ordering constraint (so a
  column-1 note could interleave with column-0 notes by recency rather than
  always rendering after) was explicitly deferred as a possible future
  refinement, not required for this wish.

## Resolution

Implemented exactly as specified: `cols` (1..=4) is measured from
`.notes-main`'s own width via a `NodeRef` + window resize listener
(`crates/tidl-web/src/components/notes/mod.rs`), driving
`grid-template-columns` inline on both grid zones. Stored `(grid_row,
grid_col)` stay in true 4-column space always — `api/notes.rs` required zero
changes. Rendering and drag targeting both go through
`group_of`/`column_offsets`/`compact_cell`/`expand_cell`, a set of pure,
unit-tested functions (20 new tests) that group the 4 real columns into
`cols` contiguous virtual columns and stack same-group real columns
vertically with no visible seam — confirmed both mathematically (all
functions degenerate to the identity mapping at `cols == NOTES_GRID_COLS`)
and visually in a real browser.

**One deviation from the spec, found during manual verification and fixed
along the way:** `.notes-layout`'s `align-items: flex-start` (there to keep
the sidebar at its own height in desktop row mode) also governs *width* once
the layout flips to a column flex direction under the mobile breakpoint —
without an override, `.notes-main` was sizing to fit its own last-rendered
content instead of the actually-available width, a measurement feedback loop
that got stuck at 2 columns and overflowed the page below ~536px. Fixed with
`align-self: stretch` on `.notes-main` (`assets/style.css`) — not anticipated
in the original design, caught by browser-testing the actual resize behavior
rather than trusting the code in isolation.

**Verified in a real browser** (isolated `TidlServer` instance, own port/data
dir, Playwright): column count steps 4→3→2→1 correctly as the container
narrows, with zero grid overflow at any width from full desktop down to
390px. Dragging at 2 columns resolved to an exact real `(grid_row,
grid_col)`, persisted across a reload, and was byte-for-byte unchanged after
expanding back to full width — confirming both the "resize alone never
rewrites anything" and "drag keeps full free-form fidelity" requirements.
Below ~350px of available width even a single fixed-260px card can't fit
(an inherent, accepted consequence of keeping card width fixed, not a
regression); at that same extreme a separate, pre-existing toolbar
limitation was found and filed as
[51](51-notes-toolbar-narrow-overflow.md) rather than fixed here (out of
this wish's scope).

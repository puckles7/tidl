# Notes grid — variable-height masonry via row-span units

Replace `#47`'s one-note-per-grid-row model with a fine-grained row *unit*
so a note's real rendered height determines how many grid rows it spans.
This lets two short notes stack directly above each other and together
match one tall neighbor's height in an adjacent column — true masonry
packing — while keeping `#47`'s stable, addressable `(row, col)` positions
(gap-preserving deletes, drag-to-any-cell, Auto-layout).

## Dependencies

- [47 - Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md)
  — this replaces its one-row-per-note coordinate semantics with row
  *units* + a per-note row-span, but keeps its explicit-position model,
  displacement-on-drop concept, and Auto-layout entirely.

## Context / Background

`#47` gives every note a stable `(grid_row, grid_col)`, but each grid row
is exactly one note tall — a CSS grid row's track height still follows its
tallest occupant regardless of column (inherent to CSS grid), so a short
note sharing a row with a tall one leaves visible dead space below it, and
there's no way for two short notes to stack and together match a tall
neighbor's height. This is the same masonry-vs-stable-position tension
`#45` fought through several iterations before `#47` traded masonry away
for stable positions — this wish gets both by making "row" fine-grained
instead of "one note tall".

## Functional Requirements

- **Row unit**: 8px. `.notes-pinned-row`/`.notes-grid` use
  `grid-auto-rows: 8px`; each note is placed via
  `grid-row: {row+1} / span {row_span};` instead of a single row index.
  `grid_col` is unchanged (still 0–3, `NOTES_GRID_COLS`).
- **Schema**: `notes.grid_row` now indexes row *units*, not notes. New
  column `notes.grid_row_span` (`INTEGER NOT NULL DEFAULT 1`) holds each
  note's real height in units (`ceil(rendered_height_px / 8)`).
- **Migration**: add `grid_row_span`, backfill `grid_row = 0`,
  `grid_row_span = 1` for every existing note. No attempt to compute real
  positions server-side — the client-side self-correction pass (below)
  untangles the resulting overlaps on first load after migration.
- **Height measurement & self-correction**: after every render, measure
  each visible note's real height client-side, convert to units, and
  compare to its stored `grid_row_span`. On mismatch (new note, edited
  note, or a freshly-migrated one): persist the corrected span, then
  cascade-push any now-overlapping notes further down the same column
  (same mechanism as drag displacement, below). Steady state (spans
  already correct) costs a cheap measure-and-compare pass with no writes.
- **New note placement**: starts with a placeholder span of 1 unit, placed
  via shortest-column (below); corrected by the measurement pass on its
  first real render.
- **Placement algorithms (server)**: `first_gap`/`place_in_first_gap` and
  `auto_layout_notes` both become shortest-column-first bin packing: walk
  notes in reading order, each goes into whichever column currently has
  the smallest total height (in units), which then grows by that note's
  span. The pinned zone's column 0 starts with a floor equal to the
  add-tile's reserved span (`ceil(96 / 8) = 12` units) instead of 0.
- **Drag & drop**: dropping a note onto an occupied interval pushes every
  overlapping note further down the *same* column, cascading as needed —
  no sideways displacement across columns (removed from `#47`'s BFS
  displacement). `cell_at`'s row resolution no longer needs DOM-measured
  per-cell geometry — row-unit height is a fixed constant, so it's cursor
  Y ÷ 8px, same as column already is cursor X ÷ column width.
- **Add-tile reservation**: still reserved at the pinned zone's `(0, 0)`,
  now as a 12-unit-tall span (`grid-row: 1 / span 12;`) instead of a
  single cell — no note's span may ever overlap that interval in column 0.

## Open Questions

- None — row unit size, new-note placement strategy, migration approach,
  drag displacement rules, and auto-layout packing strategy were all
  confirmed with the user during design (see conversation this was
  brainstormed in) before implementation.

## Resolution

Implemented as specified — see the implementation plan
(`docs/superpowers/plans/2026-07-15-notes-grid-row-span-units.md`) for the
full task breakdown. `pack_shortest_column`, `stack_column`, and
`correct_spans` are the three pure functions the whole feature is built
from (unit tested directly, 15 new tests total); server-side placement and
the client's drag displacement and height self-correction are thin
wrappers around them.

**Bug caught during manual verification, not anticipated in the spec**:
`stack_column`'s stable sort meant that when the just-dropped/just-measured
note tied on `row_start` with an incumbent (the common case — dropping
directly onto row 0, or a freshly-migrated note still parked at row 0),
the *incumbent* won the tie and the mover got pushed instead — backwards
from what a drop or a correction visually means. Fixed by always ordering
the mover (or the phantom add-tile reservation, in `correct_spans`) first
in the list handed to `stack_column`, so it wins ties and displaces
incumbents rather than the reverse. Caught by the unit tests themselves
(`drop_overlapping_an_existing_note_pushes_it_down`,
`cascade_pushes_the_whole_chain`), not by manual testing.

**A second, more fundamental bug only surfaced in a real browser**: with
CSS Grid's default `align-items: stretch`, a note-card's *rendered* height
gets forcibly clamped to exactly its current `grid-row-span` regardless of
content — so the height self-correction pass's `getBoundingClientRect()`
measurement was circular, always reading back whatever (possibly stale)
span the card already had rather than its true content height. This is
the same bug `#45` hit and fixed once already (see that file's "Follow-up
2"), reintroduced here by assuming fine-grained 8px units would make
`stretch` close enough to correct sizing to not matter — wrong, because
the correction mechanism needs to *see* a wrong size to fix it in the
first place. Fixed by restoring `align-items: start` (removed at the start
of this work on the assumption it was no longer needed).

Verified against an isolated scratch build (own `TidlServer`, own temp
DB/port, `DISABLE_SERVER_FN_HASH=1` build in a scratch copy of the repo —
never touched the developer's live `cargo leptos watch` or its `target/`):
two short pinned notes now stack directly on top of each other in one
column with zero dead space between them, alongside a long note occupying
a full column of its own — the originally reported bug. `cargo test
--workspace` (202 tests), `cargo clippy --workspace -- -D warnings`, and
`cargo build --target wasm32-unknown-unknown -p tidl-web --features
hydrate` all clean.

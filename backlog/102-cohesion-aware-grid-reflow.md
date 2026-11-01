# Cohesion-aware grid reflow (gaggle-preserving resize)

The Notes and Contacts masonry grids store an explicit `(grid_row, grid_col,
grid_row_span)` per card, but narrower viewports don't recompute layout —
they mechanically group several real columns into one visual column and
stack them vertically (`grid::group_of`/`column_offsets`). Two cards placed
deliberately side by side at a wide viewport can end up stacked far apart
once the window narrows, purely by luck of which real column each landed
in. This replaces that mechanical regrouping with a genuine 2D reflow that
detects visually-clustered groups of cards ("gaggles") from their stored
positions and keeps each gaggle together — reflowing its own internal
shape (taller, narrower) rather than letting resize scatter its members.

Also formalizes the Auto-layout/manual-placement relationship: two
explicit modes (manual 2D placement vs. alphabetical auto-pack), with an
explicit "Keep this layout" commit path between them — subsuming #99's
narrower "commit auto-layout as the new manual layout" scope, folded into
this bigger reflow design rather than built as a separate small feature.

## Dependencies

- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md)
- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)
- [47 - Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md)
- [97 - Auto-layout should restore manual placement when toggled back off](97-autolayout-restore-manual-on-toggle-off.md) — this item's snapshot/restore mechanism is superseded here: Contacts' Auto-layout was reworked (2026-09-24) into a non-persisting preview mode, so there's nothing to restore anymore; toggling off just stops previewing. Notes still needs the equivalent update.
- [99 - Commit auto-layout result as the new manual layout](99-commit-auto-layout-to-manual.md) — its scope (a "keep this" commit button) is absorbed into this item's Auto→Manual transition, under the abstract-space design below rather than built standalone.

## Context / Background

Diagnosed 2026-09-24 while investigating why a newly-created Contacts
collection's Auto-layout didn't behave as expected after hiding another
collection in the sidebar filter. `grid::group_of` is purely mechanical —
`(real_col * cols) / CARD_GRID_COLS` — so it has no notion of which cards
were placed near each other; resize just reslices the same 8 real columns
into fewer visual ones and stacks whatever landed on each side of the
integer-division boundary, without regard for actual visual proximity.

Full design discussion (approaches considered and rejected, the
idempotence requirement, and why the abstract space stays 8 columns wide)
is in the design spec this item points to once written.

## Functional Requirements

- **Manual mode** (Auto-layout off): cards keep an abstract 2D placement
  (reusing the existing `grid_row`/`grid_col`/`grid_row_span` columns
  unchanged — no migration for this part). Resize/hide-collection-filter
  changes reflow live, client-side, preserving which cards are "close" to
  each other (their gaggle) and each gaggle's position relative to other
  gaggles — nothing is written to the database just from a resize.
- **Auto mode** (Auto-layout on): cards pack tightly by order (alphabetical
  by title/display-name, same as today), ignoring gaggle structure
  entirely. Dragging in this mode only reorders (via a new persisted
  per-card order-override field), never touches the 2D placement fields.
- **"Keep this layout"**: while in Auto mode, converts the currently
  displayed auto-packed positions into new Manual-mode abstract positions
  for every card, and switches the mode to Manual.
- The reflow function must be idempotent at a fixed column count: feeding
  it positions that are already one of its own outputs at that column
  count reproduces them exactly (the practical form of a bijection, given
  a fixed image size — see design spec).
- Applies to both Notes and Contacts (shared `grid.rs` math), bringing
  Notes up to parity with whatever preview/commit UI ships for Contacts.

## Open Questions

- None outstanding at backlog-entry time — resolved during design
  (2026-09-24 brainstorming): geometric-only cohesion signal, keep the
  existing lane-based cascade drag mechanic (coarser distance signal
  accepted as the tradeoff), abstract space stays 8 columns wide (reusing
  the existing narrow-view `cell_at`/`expand_cell` expansion machinery),
  new order-override field is a genuinely new column rather than
  repurposing existing ones.

## Resolution

Built across four plans (design spec: `docs/superpowers/specs/2026-09-24-cohesion-aware-grid-reflow-design.md`):

- **Plan A** — pure grid math in `grid.rs`: `detect_clusters` (union-find
  gaggle detection over `(row_center/CLUSTER_ROW_THRESHOLD, col)`
  buckets), `pack_clusters_for_view` (cluster-preserving reflow at any
  `view_cols`, verified idempotent via a 200-trial fuzz test), and
  `nearest_neighbor_translate` (the drag-resolution primitive).
- **Plan B** — the `manual_order` column (nullable `REAL`, both
  `contacts`/`notes`) and `reorder_contact`/`reorder_note` server fns for
  Auto-mode's persisted rank override, with whole-zone backfill on first
  use so later inserts always interpolate between real numbers.
- **Plan C** — wired the reflow into rendering: Manual mode renders
  through `pack_clusters_for_view` via a `render_position` prop
  (`ContactCard`/`NoteCard`) that overrides the card's stored position for
  display only; Auto mode became a non-persisting client preview with an
  explicit "Keep this layout" commit button, replacing the old
  snapshot/restore mechanism (superseding #97's narrower scope).
- **Plan D** — cluster-aware drag *resolution*: dragging in Manual mode
  now translates the pointer's cluster-space cell back to abstract
  storage space via `nearest_neighbor_translate` (falling back to the
  pre-existing `expand_cell`/`compact_cell` grouping scheme for a
  single-card zone, which has no neighbor to translate from); dragging in
  Auto mode now persists a `reorder_contact`/`reorder_note` rank instead
  of meaningless raw grid coordinates. Along the way, fixed a real
  Manhattan-distance scale mismatch (`col_weight`, added to
  `nearest_neighbor_translate`) between column and row-unit pixel scale.

Deviations from the original design conversation: none functionally —
the abstract space stayed 8 columns wide, the drag mechanic stayed
lane-based/cascading (never free continuous placement), and the
Auto↔Manual transition matches the 3-state design agreed during
brainstorming exactly. One accepted, explicitly scoped-out gap: a card
that is *displaced* (but not itself dragged) by another card's live
cascade during an active Manual-mode drag may render at a raw,
non-cluster-translated position for the duration of that drag —
self-corrects the instant the drag ends. The actual drop position and
the drop-shadow preview are both fully cluster-aware; only this one
incidental neighbor's momentary overlay is not, since fixing it would
require restructuring `card_style`'s signature for a purely cosmetic,
self-correcting glitch.

No live browser interactive verification was performed for Plan D — the
drag interaction was verified via `grid.rs`'s pure-function unit tests
(covering every algorithmic path: cluster resolution, the empty-neighbor
fallback, `col_weight` rank-swapping, Auto-mode neighbor-finding) plus a
clean `cargo build`/`cargo build --target wasm32-unknown-unknown`/
`cargo clippy`/`cargo test --workspace` at every step, per this
project's established convention of testing library logic directly
rather than DOM-coupled component wiring (CLAUDE.md §16).

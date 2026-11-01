# Regression-test and fix the cards-grid drag/resize/visibility invariants

A real (assertion-based, persisted) Playwright regression suite for the
Contacts/Notes card grid, covering eight concrete invariants the previous
interactive-only testing pass (backlog #102) never actually checked:
grid-wide drag displacement, resize reflow preservation, Auto-layout's
non-persisting preview, hidden-collection isolation, hidden-card collision
relocation, resize round-trip idempotence, no-overlap, and "Keep this
layout" fidelity. Investigating each invariant against the real code
surfaced four genuine bugs beyond missing tests.

## Dependencies

- [102 - Cohesion-aware grid reflow (gaggle-preserving resize)](102-cohesion-aware-grid-reflow.md) — this item's regression suite covers that item's shared `grid.rs`/`drag_grid.rs` machinery and fixes follow-on bugs found in it.

## Context / Background

The parallel session that built #102 tested its work via interactive
Playwright MCP calls (`browser_evaluate`/`browser_run_code_unsafe`) that
dumped card positions for a human to eyeball — no coded assertions, nothing
saved as a regression test. Bugs kept surviving into manual testing as a
result. This item built a real pytest+Playwright suite
(`tests/test_cards_layout.py`, following the project's existing
`tests/test_hydration.py`/`tests/fixtures.py` pattern) against the exact
grid engine (`crates/tidl-web/src/grid.rs`, `components/drag_grid.rs`,
`components/contacts/mod.rs`, `components/notes/mod.rs`) that #102 built.

## Functional Requirements

- Dragging a card must relocate any card it displaces to the nearest still-
  free cell close to that card's own original position, anywhere in the
  zone — not the old column-only cascade.
- Resizing narrower must preserve each gaggle's shape, using more vertical
  space as columns shrink; resizing back to the original width must
  restore the exact original layout.
- Auto-layout is a non-persisting preview; toggling it off must restore
  the exact manual positions. Dragging while Auto-layout is on must only
  reorder — the zone stays tightly packed, never leaving a gap.
- Cards must never visually overlap, across any resize or drag.
- Toggling a collection's visibility must never move any other card.
- Dragging onto a hidden card's stored cell must relocate that hidden card
  ("underwater" — recalculated but not rendered until shown again) instead
  of silently colliding with it in storage.
- "Keep this layout" must persist exactly the positions the Auto-layout
  preview showed, including correctly relocating any hidden card it would
  otherwise collide with.

## Resolution

Built in an isolated worktree (`fix/cards-layout-regression`) off the
committed `main` HEAD, since a parallel session had uncommitted WIP in the
same files in the main worktree.

**New primitives in `grid.rs`:**
- `place_with_relocation` — Manual-mode-only grid-wide relocation: places
  the dragged card at its target cell and relocates any directly-displaced
  card to the nearest still-free cell to its *own* original position
  (weighted the same way `nearest_neighbor_translate` already was),
  terminating in at most one relocation pass per displaced card. Verified
  with a 200-trial property test asserting zero pairwise overlaps in the
  output.
- `relocate_if_colliding` — checks a card's stored cell against a claimed
  set and returns a relocation if it collides; the shared primitive behind
  both the hidden-card-drag fix and the `Keep this layout` fix below.
- `resolve_drop` (the pre-existing column-scoped cascade) was **kept**, not
  replaced: Auto-mode dragging must only reorder within an already tightly-
  packed zone, which `place_with_relocation`'s free-anywhere placement
  would violate (it can legitimately move a card to a different column,
  corrupting Auto-mode's reading-order rank and reintroducing gaps).
  `drag_grid.rs`'s `pointer_move` now branches on the same signal that
  already distinguishes Manual mode (has a usable cluster anchor) from
  Auto mode/no-anchor, picking the matching algorithm.

**Four real bugs fixed**, found via the regression suite (not just missing
tests):
1. **Hidden-collection isolation**: `pack_clusters_for_view`'s render-
   position computation in both components was fed the *visibility-
   filtered* card list, so hiding a collection could re-rank and shift
   visible cards' rendered positions — this was the exact bug #102's design
   spec was originally diagnosed from, resurfaced. Fixed by sourcing the
   clustering input from the unfiltered (by visibility) resource in both
   `contacts::ContactsView` and `notes::NotesView`.
2. **Hidden-card collision on drop**: a drop could silently land exactly on
   a hidden card's stored cell with zero collision check, since hidden
   cards are excluded from every layout structure. Fixed by running
   `relocate_if_colliding` against every hidden card before persisting a
   drag commit or a "Keep this layout" commit, in both components.
3. **"Keep this layout" divergence**: the button routed through a server-
   side `auto_layout_contacts`/`auto_layout_notes` recompute that repacked
   *every* accessible card alphabetically — diverging from the client
   preview (which only packs the *visible* subset) whenever a collection
   was hidden during the preview. Fixed by persisting the client preview's
   own positions directly via `move_contacts`/`move_notes` (mirroring a
   drag commit) plus the hidden-collision pass above. `auto_layout_notes`
   (now with zero callers) was removed along with its test;
   `auto_layout_contacts` is kept — it's still the only path for the post-
   VCF-import rebalance.
4. **Height self-correction staleness + duplicate persists**: the RAF-
   scheduled span-correction pass captured its position *baseline* at
   effect-fire time but only read live DOM heights at (possibly much
   later, if the tab was backgrounded/throttled) RAF-execution time, and
   nothing cancelled a stale pending RAF when a newer one was scheduled —
   several rapid actions could each schedule their own correction, and all
   of them eventually fired, with no ordering guarantee against a more
   authoritative action's own persist. Fixed by re-reading the baseline
   fresh inside the RAF callback (`get_untracked`) and tracking/cancelling
   any still-pending RAF before scheduling a new one, in both components.
   A `committing_layout` guard was also added mirroring the existing
   in-progress-drag guard, since "Keep this layout" flipping
   `auto_layout_enabled` immediately after persisting is itself one of the
   card-list-building effect's own dependencies.

**One bug found and fixed in this item's own new code**: the initial
`place_with_relocation` call site passed the effective *view* column count
(e.g. 3 at a narrow viewport) as the function's real-column-space bound,
silently clamping any resolved real column beyond it back down into the
visible range. Fixed by always passing `CARD_GRID_COLS` (the true 8-column
storage space) regardless of how many columns are currently rendered.

**Two more bugs surfaced by live manual testing after the above landed**
(the automated suite's fixed spans — chosen to sidestep an unrelated race —
meant it never exercised either path):

6. **Cluster-packing render positions went stale after a successful height
   self-correction**: fixing bug #1 above made both components' cluster-
   aware render-position map source its entries from the raw server
   resource (`contacts_res`/`notes_res`) instead of the locally self-
   corrected display list, so it could pick up hidden-collection members —
   but a *successful* height self-correction persist never bumps `refresh`
   to refetch that resource, so any visible card's corrected
   `grid_row_span` stayed permanently stale in the clustering input,
   silently under-booking its real height and packing its neighbors too
   close — cards rendered visibly overlapping ("collapsed into each
   other"), most visible on a fresh account (or `seed-demo` data, which
   seeds every note at a deliberately-wrong placeholder `grid_row_span=1`
   for this exact pass to correct). Fixed by merging each *visible* card's
   fresh, locally self-corrected position over the raw resource data
   (hidden cards still come from the resource, since they're never
   measured/corrected) before feeding `pack_clusters_for_view`.
7. **Auto-mode dragging could strand a sibling, leaving a gap**: a drop's
   local patch only ever touched the ids its own column-scoped cascade
   moved (the *target* column) — a card left behind in the *source*
   column (one with other members) kept its stale pre-drop row forever,
   since nothing pulled it up to close the gap, and a successful
   `reorder_contact`/`reorder_note` never bumps `refresh` either. Fixed by
   expressing the reorder as an explicit new whole-zone reading-order list
   (moving the dragged id next to whichever neighbor
   `auto_mode_reorder_neighbors` found) and immediately re-packing the
   *entire* zone from that order via `pack_for_view` — the same primitive
   the Auto-layout preview itself already uses — instead of only patching
   the cascade's own few ids.

New test file: `tests/test_cards_layout.py`, 12 tests (contacts + notes),
each against its own throwaway `tidl-server` instance (mirrors
`tests/fixtures.py`'s `TidlServer`, not the shared session-scoped one) so
seeded positions from one test can never influence another's cluster-
detection math. Ground truth is `list_contacts`/`list_notes` for anything
meant to persist; DOM `data-grid-row`/`data-grid-col`/rect reads for
client-only preview state. Two tests (`test_notes_first_load_does_not_-
collapse_with_placeholder_spans`, `test_auto_layout_drag_leaves_no_gap_in_-
the_source_column`) were added after live manual testing surfaced bugs #6
and #7 above, specifically to cover the gap the fixed-span seeding had left.

Deviations from the original plan: the "drag onto a hidden card's slot"
test was redesigned mid-implementation — the first version tried to target
an *invisible* card's exact pixel slot with no other visible card to anchor
the pointer-resolution math against, which is a pre-existing, already-
documented degenerate case (`nearest_neighbor_translate`'s "no neighbors to
translate from" fallback), not what this assertion is about. Redesigned to
target a *visible* anchor card's own position instead (realistic usage),
with the hidden card seeded at a colliding starting position — this
directly exercises the commit-time relocation logic without depending on
ambiguous single-card-zone pixel math.

Full verification: `cargo test --workspace` (783 tests across all crates),
`cargo clippy --workspace -- -D warnings`, `cargo build --target
wasm32-unknown-unknown -p tidl-web`, and the new
`pytest tests/test_cards_layout.py -v` (12/12) all pass. `cargo fmt --check`
shows only pre-existing baseline drift unrelated to this change (verified
line-by-line against the diffs already present before this item started).
Bugs #6 and #7 were also confirmed fixed live via a manual `gts` session
(Playwright-driven, not just the automated suite) against real seed-demo
data, reproducing both exactly as reported before the fix and confirming
their absence after.

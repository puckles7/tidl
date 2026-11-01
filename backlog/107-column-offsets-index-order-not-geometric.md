# `column_offsets` orders colliding real columns by raw index, not by content — breaks the "purely geometric" mental model

When a narrow viewport forces two real columns to share a view column
(`grid::view_col_of`), `column_offsets` decides which one renders "on top"
purely by ascending real-column index — real column 0 always renders before
real column 7 when they collide, regardless of either column's actual row
values. This produces a consistently asymmetric squeeze (low-index columns
are structurally "sticky" and never move; whichever column happens to be
higher-index absorbs the entire pressure) rather than the "everything feels
proportional pressure, gaggles interleave by vertical position" behavior a
purely geometric compaction would suggest.

## Dependencies

- [106 - Manual-mode cluster cache confused real (storage) columns with render (view) columns](106-cluster-cache-real-vs-view-column-confusion.md) — introduced `view_col_of`/`column_offsets` as the current compaction mechanism

## Context / Background

Reported live (2026-09-28) after backlog #106 shipped: with three gaggles
arranged top-left, middle-right, and bottom-left, narrowing the window only
ever squeezes middle-right — it gets shoved to a much larger vertical
distance from its own original spacing than expected, and can end up
rendered *below* bottom-left even though its own rows were never anywhere
near that. Bottom-left (and top-left) never move at all, no matter how
narrow the window gets, right up until they themselves collide with
something.

Root cause, precisely: `column_offsets` loops real columns `0..CARD_GRID_COLS`
in that fixed order. For each real column `c`, it looks up
`column_height[view_col_of(c, cols)]` (how tall that view column's stack is
so far), assigns `c` that offset, then adds `c`'s own occupied height to the
running total. Because the loop always visits column 0 before column 7,
whichever colliding column has the lower raw index *always* gets offset 0
(renders first) and the higher-index one is *always* pushed below the
other's entire occupied height — this is a fixed tie-break by column
identity, not an emergent property of geometry or of either column's actual
vertical content. A "purely geometric" model — where two colliding gaggles
interleave based on where their cards actually sit vertically — would need
the tie-break to be based on content (e.g. each colliding column's lowest
occupied row), not on which one happens to have the smaller stored column
number.

This is a real trade-off, not a one-line bug: switching to a content-based
tie-break (order colliding columns by, say, lowest occupied row) stays a
pure function of current occupancy and `cols_now`, so it doesn't threaten
the exact-resize-round-trip guarantee `compact_cell`/`expand_cell` were
built for (backlog #106) — but it does introduce a new hazard: a small edit
to either colliding gaggle (e.g. dragging one card one row earlier) could
cross the ordering threshold and *flip* which whole gaggle renders on top,
a more jarring discontinuity than today's smooth-but-lopsided "one column's
block grows, so the other one shifts a bit further down." It also wouldn't
by itself close the "far more distance than the gaggle's own spacing" gap
— that's inherent to concatenating whole blocks rather than interleaving
individual cards row-by-row, which is a materially bigger change (and
raises the same "can `expand_cell` still invert this exactly" question that
sank the original masonry-repack approach in backlog #106).

Parked for now — revisit only if the asymmetric-squeeze behavior becomes a
real complaint rather than a curiosity, since the fix isn't free.

## Functional Requirements

- Not yet scoped. Candidate approaches to evaluate when this is picked up:
  1. Order colliding real columns within `column_offsets` by lowest
     occupied row instead of raw column index (bounded change, doesn't
     touch `view_col_of`/the grouping itself, keeps exact round-trip
     stability, but can produce order-flip discontinuities on unrelated
     edits within the colliding group).
  2. A genuine row-level interleave across colliding columns (closes the
     "excess distance" gap too, but needs its own design pass on whether
     `expand_cell` can still invert it exactly).

## Open Questions

- Is the "order flip" hazard from approach 1 actually worse in practice
  than today's "one column always eats it" behavior, or just different?
- Is approach 2's added complexity (and its own invertibility risk)
  justified, or is approach 1 alone "good enough"?

## Resolution

Implemented **approach 1** (2026-10-01), confined to
`crates/tidl-web/src/grid.rs`:

- `column_offsets` now partitions real columns into view-column groups as
  before, but renders each group's members in *content order*: sorted by
  their topmost occupied row (`min(row)`), ties keeping real-column order.
  Members with no occupied rows (`height == 0`) sort last, and a real
  column-0 reservation is treated as content starting at row 0 so it always
  stays on top. Offsets are still the running sum of the preceding members'
  occupied heights, so the exact masonry-style round trip is unchanged.
- `expand_cell` was rewritten accordingly: among the members sharing a view
  column it now picks the one with the *greatest offset still `<=`
  `virtual_row`* (rather than the `deepest real index` whose offset has
  started), which is exactly the block containing the row for any render
  order, and stays the exact inverse of `compact_cell`. Ties (only possible
  among the trailing empty members) are broken by largest real index.

This directly fixes the reported symptom: a "middle" gaggle colliding with a
"bottom" one now renders above it (smaller topmost row wins) instead of
always being shoved below by virtue of its stored column number, and
low-index columns are no longer structurally sticky.

**Not done (deliberately):** approach 2's row-level interleave. The excess
distance from concatenating whole blocks (rather than interleaving
individual cards row-by-row) remains, as documented above; it is a
materially larger change and would need its own design pass on
`expand_cell` invertibility. Approach 1 was deemed "good enough" for the
reported complaint.

**Tests:** three new `column_offsets` tests (content ordering beats raw
index; equal topmost rows keep real-column order with empties last;
reserved column 0 stays on top) and one new `compact_expand_cell` test
asserting exact `expand_cell`/`compact_cell` inversion over a
content-ordered compaction. `cargo test --workspace` and
`cargo clippy --workspace -- -D warnings` pass. The Playwright e2e suite
(`tests/test_cards_layout.py`) could not be run in this sandbox
(browsers don't launch); the change does not alter which real columns share
a view column, so its no-overlap / gaggle-cohesion assertions are
unaffected in principle, but the collision-order-related cases were not
re-executed.

## Follow-up (2026-10-04): the collision-order e2e cases, once runnable

The browser-launch blocker was fixed (`tests/conftest.py` strips the host
`LD_LIBRARY_PATH`), so the previously-unrun Playwright cases now execute —
and exactly the collision-order ones surfaced, as the "order flip" hazard
above predicted:

- `test_drag_relocates_displaced_card_near_its_own_original_spot` — a card
  displaced to the capped join gap (backlog #120) one row under the member
  above was remapped by `expand_cell` into an *empty* real column that
  content-sorts **above** it, swapping the two cards' rows. Fixed in
  `grid::expand_cell`: while the group has any occupied member at or above
  `virtual_row`, prefer the occupied member (greatest rendered start `<=`
  row); a targeted empty member is only reached for a view column with no
  occupied member at all. The exact occupied-cell inverse is unchanged.
- `test_dropped_card_lands_exactly_where_its_shadow_showed` and
  `test_drop_never_flings_a_displaced_note_far_away` — these encoded the
  pre-#107 guarantee that a drop (and any displaced card) lands at its
  exact shadow row and moves only a short distance. Content ordering can
  flip which whole block renders on top once an edit changes a column's
  topmost row, so a local drop may reflow a block vertically. Per the
  documented trade-off, the tests were updated to assert the invariants
  that still hold — the drop lands in the same *column* as the shadow, no
  card changes column, and nothing ever overlaps — rather than the exact
  row.

`cargo test --workspace` (602), `cargo clippy -p tidl-web --features ssr
-- -D warnings`, and the full `pytest tests/` suite (110) all pass.


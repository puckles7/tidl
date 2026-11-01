# Manual-mode drag shadow snaps to the wrong column mid-drag

While dragging a card in Manual mode, the dashed drop-target shadow could
visibly jump to a completely different (and unrelated) column partway
through the drag, even while the pointer stayed within the same real
column the whole time — most obvious when dragging a card through
wide-open empty space toward a distant, lone card in its own column.
Releasing while the shadow was showing the wrong column landed the drag
there, displacing an unrelated card.

## Dependencies

- [102 - Cohesion-aware grid reflow (gaggle-preserving resize)](102-cohesion-aware-grid-reflow.md) — the cluster-aware drag resolution (`nearest_neighbor_translate`, `drop_shadow_view`) this item fixes two bugs in.
- [103 - Regression-test and fix the cards-grid drag/resize/visibility invariants](103-cards-layout-regression-suite.md) — the prior follow-up pass on the same machinery; per its own Resolution note, Plan D's live drag interaction had never actually been verified in a browser.

## Context / Background

Reported live (2026-09-26) against the Contacts tab's default synthetic
data: dragging Grace Hopper (leftmost card, second row) straight north
into the large empty area above — directly over Ada Lovelace's column,
with no other card anywhere nearby in that column — made the dashed
destination box immediately jump to sit under Alan Turing (the second
column) and start pushing that card down, before finally snapping back to
the correct column only once the pointer got close to Ada. Releasing
early (while the shadow was still in the wrong column) displaced Alan
Turing to a different cell instead of leaving Grace in column one as
expected.

Root-caused via systematic debugging (reproduced headlessly through
Playwright against a `gts` test server, with temporary `leptos::logging`
instrumentation to trace both directions of the translation):

1. **Forward direction** (`pointer_move`'s pointer → abstract-space
   resolution): `nearest_neighbor_translate` picked whichever existing
   card was nearest by a *weighted* Manhattan distance across every
   column, not preferring an exact match on the pointer's own
   already-unambiguous render column — so a row-close card in a different
   column could out-rank an exact column match that was merely far away
   in row-distance.
2. **Reverse direction** (`drop_shadow_view`'s abstract-space →
   render-space translation for display): even after fixing (1), this
   *second*, independent `nearest_neighbor_translate` call degenerated
   whenever the zone's *abstract* (stored) columns were themselves
   uninformative — e.g. every card parked at abstract column 0 (true of
   the seed/demo data, and generally of any card never individually
   dragged before, since only the cluster-reflow packing ever assigns it
   a distinct *render* column) — in which case every candidate trivially
   "matched" the target abstract column and the search fell back to
   picking whichever card was closest in raw abstract row, regardless of
   which render column that card actually occupied.

## Functional Requirements

- The drop-shadow must track the pointer's real column continuously
  during a Manual-mode drag through empty space — no visible jump to an
  unrelated column while the pointer's own screen position hasn't crossed
  into a different real column.
- Releasing the drag must always place the card in the column the shadow
  was last showing.

## Resolution

Two-part fix, both in `crates/tidl-web/src`:

- `grid::nearest_neighbor_translate` — an entry sharing the target's own
  `from`-column is now always preferred over every other entry,
  regardless of `col_weight` or row distance, before falling back to the
  existing weighted cross-column search. Fixes the forward direction (1).
- `drag_grid::DragItemState` gained `last_render_cell: Option<(i32,
  i32)>` — the pointer's own render-space cell from `cell_at`, tracked
  directly alongside the existing abstract-space `last_cell` whenever
  Manual mode has a usable cluster anchor. `drop_shadow_view` now prefers
  this tracked value over re-deriving the render position by reversing
  `last_cell` through a second `nearest_neighbor_translate` call — sound
  because `place_with_relocation` never moves the *dragged* card away
  from its resolved cell (only displaced neighbors relocate), so the
  render cell the pointer already landed on via plain pixel geometry is
  always correct, with no reverse-translation ambiguity to fall into.
  Fixes the reverse direction (2), which (1) alone did not.

Verified via two new `grid.rs` unit tests (an exact-column match beating
a row-closer different-column entry; the weighted fallback still applies
when no exact match exists) plus a full live-browser reproduction
(`gts` + Playwright) before and after each fix — the shadow now holds the
correct column for the whole drag in both the full-range and the
release-mid-drag cases that previously misplaced a card.

`card_style`'s own cluster-space rendering of an *other*, cascade-displaced
card (not the dragged one) still goes through the same degenerate-prone
reverse translation — left as is, matching #102's own accepted, explicitly
scoped-out gap ("may render at a raw, non-cluster-translated position for
the duration of that drag — self-corrects the instant the drag ends").

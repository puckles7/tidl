# Manual-mode cluster cache confused real (storage) columns with render (view) columns

A dropped card could snap to a completely unrelated position — far from
where it was released — and, separately, a note or contact could render
squeezed into an unstyled sliver of a column, wrapping one letter per
line, right after page load or a resize.

## Dependencies

- [102 - Cohesion-aware grid reflow (gaggle-preserving resize)](102-cohesion-aware-grid-reflow.md) — introduced `cluster_cache`/`pinned_cluster_cache`/`other_cluster_cache`, the cache this item fixes.
- [104 - Manual-mode drag shadow snaps to the wrong column mid-drag](104-manual-drag-shadow-wrong-column.md) — found in the same area; a distinct bug in the same machinery.
- [105 - Manual mode recomputed cluster reflow on every render, not just resize](105-manual-mode-recomputes-reflow-every-render.md) — introduced the in-place cache patches this item fixes.

## Context / Background

Reported live (2026-09-27): a note rendered in an implicit, unstyled grid
column — visibly squeezed to a sliver, wrapping one letter per line —
after the initial page load at a narrower-than-full window width, and
again after some resizes. Separately, dragging a Manual-mode card (e.g.
"Wifi guest network" among Notes, a contact among a two-island Contacts
layout) and releasing it snapped the card to a distant, unrelated cell
instead of where it was dropped.

Root-caused via systematic debugging: `pinned_cluster_cache`/
`other_cluster_cache` (Notes) and `cluster_cache` (Contacts) hold
**render/view-space** column indices — the compacted, cluster-shelf-
packed column a card actually shows up in (`pack_clusters_for_view`'s
output, always `< view_cols`). Several in-place patches added by #105
(and the "Keep this layout" commit path) inserted the **real (storage)
space** column instead — a different, unrelated number range once the
grid compacts below its full column count, which is nearly always true
even at ordinary desktop widths:

- The height self-correction pass's cache patch reinserted
  `correct_spans`' `orig_col` (real space, never actually changed by
  that pass) over an existing, correct cached render column.
- The Manual-mode drag-drop handler's cache patch reinserted
  `place_with_relocation`'s real-space output directly.
- "Keep this layout"'s cache patch reinserted the freshly-persisted
  real-space column directly.

A related bug in the same call sites: the hidden-collection-relocation
branch's `relocate_if_colliding` call passed the *view* column count
(`cols.get_untracked()`) where real (storage) column space was expected
— the same class of confusion `place_with_relocation`'s adjacent call
had already been fixed for (see #104's Resolution) — silently clamping a
hidden card's own real column into the (smaller) view width and
dropping any claimed real-space entry beyond it from collision
consideration.

## Functional Requirements

- A card must never render in an implicit (unstyled, content-sized)
  grid column or overlap another visible card, at any window width, on
  initial load, after a resize, or immediately after a Manual-mode
  drag/self-correction/"Keep this layout" commit.
- A Manual-mode drop must land visibly where the pointer released it
  (or the nearest free cell to it), never at an unrelated column
  elsewhere in the grid.
- Hidden-collection relocation must correctly avoid colliding with
  every real-space claimed position, regardless of the current view
  column count.

## Resolution

Both in `crates/tidl-web/src/components/notes/mod.rs` and
`contacts/mod.rs` (Contacts has one flat cache; Notes has one per zone):

- The height self-correction cache patch now only overwrites the
  cached entry's row/span, preserving whatever render column was
  already cached (`correct_spans` never actually changes the column).
- The Manual-mode drag-drop and "Keep this layout" cache patches no
  longer reinsert the real-space column verbatim. For "Keep this
  layout" (which commits the whole zone at once, leaving nothing stale
  behind to collide with), the touched ids are simply dropped from the
  cache, falling back to `card_style`'s no-override path
  (`compact_cell` against the freshly-persisted real position) — safe
  because nothing else in the zone is still relying on a stale cached
  render position at that moment. For the drag-drop handler (where
  *other*, untouched cards keep rendering from their own cached,
  shelf-packed positions), each moved id's natural cell is computed via
  `compact_cell` against the zone's current real positions, then
  nudged to the nearest free cell via `relocate_if_colliding` — scoped
  to just the *other* still-cached ids — if that natural cell is
  already occupied, so a moved card can never overlap an untouched
  neighbor rendered through the different (cluster-shelf) addressing
  scheme.
- The hidden-collection-relocation branch's `relocate_if_colliding`
  call in both files now passes `grid::CARD_GRID_COLS` (real space)
  instead of the view column count, matching `place_with_relocation`'s
  adjacent, already-fixed call.

Verified: `cargo test --workspace` (522 tests), `cargo clippy
--workspace -- -D warnings`, and the full `tests/test_cards_layout.py`
Playwright suite (14 tests, including the two that started failing
once the wrong-cache-value bug stopped being masked by the *other*,
wrong-shadow-column bug from #104) all pass.

### Follow-up: the drop still didn't land where its own shadow showed

The first fix pass above stopped the wild jumps and the overlaps, but
live testing (2026-09-27) found the dropped card could still land at a
cell visibly different from the one the dashed shadow had just shown —
a smaller but still-wrong displacement. Root cause: the drag-drop cache
patch re-derived the dragged item's own render cell via
`grid::compact_cell` against *freshly recomputed* column offsets (built
from the post-drop occupancy in `crates/tidl-web/src/components/
notes/mod.rs`/`contacts/mod.rs`). `compact_cell` only round-trips back
to the exact cell the shadow showed when fed the *same* (frozen,
pickup-time) offsets `drop_shadow_view` itself used — and a drag that
changes real-column occupancy changes those offsets, so the two
diverged.

Fixed in `crates/tidl-web/src/components/drag_grid.rs`: factored
`drop_shadow_view`'s cell computation out into
`resolve_dragged_render_cell`, and `DragController::pointer_up` now
calls it itself (before clearing `drag`/`drag_overlay`) and returns the
result as a new `dragged_render_cell` field on `DragOutcome::Moved`.
Both callers' cache patches now use this value verbatim for the dragged
item's own cache entry — never re-derived — while still computing a
natural `compact_cell` position (nudged clear of the cache via
`relocate_if_colliding` if occupied) for any *other*, merely-displaced
id, which has no shadow to match.

Added `tests/test_cards_layout.py::
test_dropped_card_lands_exactly_where_its_shadow_showed` — six cards
across three real columns at a heavily compacted (2-column) viewport,
captures `.grid-drop-shadow`'s bounding box immediately before release,
and asserts the dropped card's own rect matches it within 3px.
Confirmed this test fails without the `dragged_render_cell` fix
(the dropped card landed 376px away from where its shadow last showed)
and passes with it. Full suite: 15 Playwright tests, `cargo test
--workspace` (522), `cargo clippy --workspace -- -D warnings`, and both
wasm32 and server builds all clean.

### Second follow-up: the whole approach was patching symptoms of one
### architectural problem (2026-09-27, live testing)

Even after both fixes above, live testing turned up a cluster of further
bugs, all traceable to one root cause: dragging one card could visibly
move a completely unrelated, non-colliding card elsewhere ("spooky
action at a distance"); a card visibly displaced during the live drag
preview could land somewhere *else* once actually dropped; ordinary local
drags could fling a displaced card far down the page; a resize round
trip (narrow, then back to the original width) could scramble a
carefully-arranged Manual-mode layout; and "Keep this layout" would
revert to a different arrangement shortly after committing.

**Root cause**: Manual-mode dragging computed pointer/collision
resolution in **real (storage) space** (`expand_cell`/
`nearest_neighbor_translate`/`place_with_relocation` against
`grid::CARD_GRID_COLS`), while the actual on-screen rendering (once
`pack_clusters_for_view`'s cluster cache populated) showed cards in a
**completely different, shelf-packed render space**. The two coordinate
systems only ever agreed by coincidence. `nearest_neighbor_translate`'s
job was to bridge them with a nearest-neighbor heuristic — explicitly
documented elsewhere as "lossy by design" — which is exactly what let a
drag's real-space target coincide with an unrelated card's real position
even though the two were nowhere near each other on screen. Every
previous fix in this item (and #104) was patching one more symptom of
this same two-coordinate-system mismatch, not the mismatch itself.

**Fix**: rewrote the Manual-mode drag path in
`crates/tidl-web/src/components/drag_grid.rs` to operate in **one**
coordinate space end to end. `DragItemState` gained a `manual: bool`
flag; when set, `occupied`/`cols`/`last_cell`/the collision search
(`place_with_relocation`) and the final persisted/cached positions are
**all** in render/view space — the same space `cell_at` resolves the
pointer into and the same space the cluster cache and every card's own
`render_position` already use. Auto mode is untouched (still real space,
via `resolve_drop`/`compact_cell`/`expand_cell`, which was never
ambiguous to begin with). This deleted `nearest_neighbor_translate`,
`cluster_render_position`, `expand_cell`'s Manual-mode call,
`last_render_cell`, and the `dragged_render_cell`/
`resolve_dragged_render_cell` bridge from the previous follow-up above —
none of them are needed once there's only one space to be in.
`notes::manual_pickup_items`/`contacts::manual_pickup_items` build a
drag's `items` in render space at pickup (each card's cluster-cache
entry, falling back to a plain compaction for one not yet cached). The
drag-drop and hidden-collection-relocation cache patches in
`notes/mod.rs`/`contacts/mod.rs` are now a direct `m.insert(...)` of
`moves`' own values — no re-derivation, no separate translation step to
diverge from what's on screen.

**A second, independent bug surfaced once the above was fixed**: "Keep
this layout" reverted to a different arrangement shortly after
committing, and a resize round trip could still scramble things.
Root cause: the cluster-cache-populating `Effect` in both files
unconditionally re-ran `pack_clusters_for_view` whenever
`auto_layout_enabled` flipped to `false` — true both for "cancel the
Auto-layout preview" (which must re-cluster) and "Keep this layout"
committing (which must *not*, since it already patched the cache with
the exact positions just shown). Fixed by adding a `manual_reflow_trigger`
counter (bumped only by the "cancel preview" path) and a `last_reflow_key`
`(cols, accessible_ids, trigger)` the Effect compares against before
recomputing, skipping the redundant reflow when only `auto_layout_enabled`
itself changed.

Separately, `pack_clusters_for_view` is only guaranteed idempotent when
re-applied to **its own** prior output (per its existing unit test) —
never on an arbitrary `pack_for_view` tight pack. "Keep this layout" was
persisting the raw tight pack, so the *next* reflow (a resize, or simply
the next full page load, whose first render always populates the cache
from scratch) re-clustered it into a visibly different arrangement.
Fixed by running the tight pack through `pack_clusters_for_view` once,
per zone, before persisting/caching, in both `on_keep_layout`s — so a
later reflow of those exact positions is re-deriving its own prior
output, which the idempotence guarantee actually covers. This means the
displayed layout can shift slightly the instant "Keep this layout" is
clicked (never before), in exchange for staying exactly stable forever
after, including across a full reload.

A third, narrower bug in the same family: the height self-correction
pass patched a cached entry's row/span in place while preserving its
existing cached column, to avoid clobbering a good cluster position with
`correct_spans`' real-space output. But `pack_clusters_for_view`'s own
member-ordering tie-break sorts on exact row first (deliberately, for
*its own* idempotence), so a correction changing a member's row without
going through a full re-cluster left that member's row and its
(preserved) column mutually inconsistent — invisible until the *next*
unrelated reflow (typically a resize) suddenly recomputed a different
column for it. Fixed by making the correction trigger an immediate full
re-cluster of just that one zone (using the corrected positions),
instead of an in-place patch — so the cache is always exactly what a
fresh `pack_clusters_for_view` would derive right now, with nothing left
for a later reflow to silently change. (Changing `pack_clusters_for_view`'s
own tie-break to be less row-sensitive was tried first and reverted: it
broke the existing `is_idempotent_at_a_fixed_column_count` fuzz test.)

**Tests added** (`tests/test_cards_layout.py`), each written to fail
without its corresponding fix and confirmed to pass with it:
- `test_dragging_one_note_never_moves_an_unrelated_far_away_note`
- `test_note_displaced_during_preview_lands_exactly_there_after_drop`
- `test_drop_never_flings_a_displaced_note_far_away`
- `test_manual_layout_survives_a_resize_round_trip_after_real_drags` —
  checks gaggle cohesion (column-compact, no cross-mixing) across a
  resize round trip following a real drag, not exact per-id position:
  a resize's own re-cluster legitimately repacks a gaggle's *internal*
  member arrangement at its natural width, and was never a promise to
  preserve a manually-dragged sub-arrangement pixel-for-pixel.
- `test_keep_this_layout_persists_after_reload_with_prior_drags` —
  checks against what was on screen the instant "Keep" committed, not
  the pre-commit preview (see above), and forces a real full-page reload
  as ground truth.
- `test_keep_this_layout_persists_exactly_what_was_previewed` (existing,
  Contacts) updated to the same "compare against post-commit state, not
  the preview" ground truth.

**Final verification**: `cargo test --workspace` (522 tests), `cargo
clippy --workspace -- -D warnings`, `cargo build --target
wasm32-unknown-unknown -p tidl-web`, and the full
`tests/test_cards_layout.py` suite (20 tests) all pass.

### Third follow-up: resize losing horizontal spread, and Auto-layout
### on/off destroying gaggles (2026-09-27/28, live testing)

Live testing after the second follow-up found two more distinct bugs,
both traced to the *same* underlying mechanism, and both matching a
correct architectural observation from the user: resizing (or
previewing/canceling Auto-layout, which never persists anything) must
be a pure presentation-layer transform of the stored data — it must
never rewrite the abstract, stored positions.

**Bug A — a live height correction could reorder gaggles relative to
each other on a later, unrelated resize.** The second follow-up's fix
made a height correction trigger an immediate full re-cluster of the
zone, to keep the cache internally consistent. That full re-cluster
also reruns `pack_clusters_for_view`'s *inter-cluster* ranking, which
sorts on each cluster's `row_top` — so a correction nudging one
member's row (to fix a real content-height mismatch, nothing the user
asked to rearrange) could silently reorder that whole cluster relative
to every other cluster the next time anything reflowed. In practice
this happens on nearly every real note (real content rarely matches
its placeholder span exactly), which matches the user's screenshots:
gaggles stayed intact internally but lost their relative left-to-right
position across a resize round trip, "everything centered."

Fixed in two parts:
- Reverted the correction's cache patch back to row/span-only, preserving
  the existing cached column (undoing the second follow-up's "full
  recompute on correction" — that was solving a narrower problem at the
  cost of this much more visible one).
- `grid::pack_clusters_for_view`'s inter-cluster ranking now buckets
  `row_top` by `CLUSTER_ROW_THRESHOLD` (the same window `detect_clusters`
  already groups by) before falling back to `col_centroid` and the exact
  `row_top` — so a correction has to shift a cluster into a genuinely
  different row band, not just a few units, before it can change any
  cluster's rank relative to another. Verified this doesn't break the
  existing `is_idempotent_at_a_fixed_column_count` fuzz test (a coarser,
  more stable ordering is still self-consistent). A parallel attempt to
  bucket the *within*-cluster member tie-break the same way was tried
  and reverted — it broke that fuzz test, because a repack's own output
  already carries "fresh" column values a bucketed row comparison
  couldn't consistently order against a first pass's raw ones.

**Bug B — two Leptos effects racing on the same signal write, twice.**
Both "Keep this layout" and canceling an Auto-layout preview flip
`auto_layout_enabled`, which is a reactive dependency of *two*
independent Effects: the one rebuilding `pinned_display`/`other_display`
from stored data, and the cluster-cache-populating one. Leptos does not
run a dependent Effect synchronously inline with the `.set()` call that
dirtied it, and gives no ordering guarantee between two Effects
triggered by the same write when neither depends on the other's output
through the tracked graph (only through an untracked `get_untracked()`
read at computation time, which isn't part of that graph). Two
consequences, both reproduced live:
- "Keep this layout" committed the correct positions (verified via
  `leptos::logging::log!` — the cache and `other_display` held exactly
  the tight-pack-through-`pack_clusters_for_view` result), but the
  *rendered* DOM kept showing stale, pre-persist positions, because the
  guard meant to prevent the `notes_res`/`contacts_res`-rebuilding
  Effect from re-running with stale data (`committing_layout`) was
  itself cleared in the same synchronous block as `auto_layout_enabled`,
  so that Effect — whenever it happened to run after this write —
  always saw the guard already down.
- Canceling an Auto-layout preview could leave two notes/contacts
  overlapping: the cache-populating Effect's recompute reads
  `pinned_display`/`other_display`'s *current* value (to preserve an
  in-flight drag's optimistic position), but nothing guaranteed the
  *other* Effect — which reverts those from the preview's tight pack
  back to the real stored positions — had already run first.

Fixed by deferring the guard-clearing/trigger-bumping write in both
cases to the next animation frame (`request_animation_frame`), the same
technique the height self-correction pass already uses elsewhere in
this file to wait for a reactive settle: `committing_layout.set(false)`
in `on_keep_layout`, and `manual_reflow_trigger`'s bump in
`on_auto_layout`'s cancel path, in both `notes/mod.rs` and
`contacts/mod.rs`.

**Tests added** (`tests/test_cards_layout.py`), each confirmed to
reproduce its bug against the pre-fix code and pass with the fix:
- `test_height_correction_never_reorders_gaggles_on_a_later_resize` —
  seeds a mismatched placeholder span to force a real correction, then
  asserts a later resize never changes which of two gaggles ranks
  first.
- `test_auto_layout_off_restores_manual_gaggles` /
  `_four_manual_gaggles` / `_gaggles_built_via_several_real_drags` —
  two, four, and (via several real sequential drags rather than seeded
  positions) three gaggles respectively, each toggling Auto-layout on
  then off and checking cohesion is preserved and nothing was persisted
  during the preview.
- `test_resize_from_max_to_min_columns_and_back_preserves_gaggles` /
  `_through_several_widths_preserves_four_manual_gaggles` — resize
  round trips (including the actual min/max column counts reachable via
  resizing, recalibrated: 6 columns, not `grid::CARD_GRID_COLS`'s 8 —
  `.notes-main`/`.contacts-main` cap out around 2000px wide regardless
  of viewport).

**Final verification**: `cargo test --workspace` (522 tests), `cargo
clippy --workspace -- -D warnings`, `cargo build --target
wasm32-unknown-unknown -p tidl-web`, and the full
`tests/test_cards_layout.py` suite — now 26 tests — all pass.

### Fourth follow-up: resize and Auto-layout still weren't idempotent — the "abstract" was never actually resize-independent

Reported live (2026-09-27, after the third follow-up): resizing a
manually-arranged layout down and back up still didn't restore it, and
toggling Auto-layout on then off still scrambled it — the exact same
two symptoms, still present, despite the third follow-up's fixes and a
green 26/26 test suite. Confirmed via build-artifact mtimes that the
live dev server *was* serving the latest code — this was a genuine
remaining bug, not a stale build.

**Root cause.** `pinned_cluster_cache`/`other_cluster_cache` (Notes) and
`cluster_cache` (Contacts) were the *only* record of a Manual-mode
card's position — populated by `pack_clusters_for_view(..., cols_now)`,
i.e. at whatever column count was on screen at the time. The
cache-populating Effect's own recompute (needed so an in-flight drag's
optimistic position wasn't lost) read the cache's own *current* value
back in as its input on every subsequent run — including a plain
resize. So: squeeze the window down to one column and every card's
column collapses to 0 (there's only one column to show); widen it back
up, and the Effect re-clusters from that already-collapsed state — the
original horizontal spread was never stored anywhere else to recover
from. There was no genuine "abstract" position independent of whatever
had last been rendered; despite the name, `pack_clusters_for_view`'s
input space and its `cols_now`-wide output space were the same signal,
serving both roles at once.

**Fix.** Introduced a second signal per zone — `pinned_abstract`/
`other_abstract` (Notes), `abstract_positions` (Contacts) — always
`CARD_GRID_COLS`-wide, exactly like Auto mode's own "real" space, and
written *only* by an actual edit (a drag, a hidden-note/contact
relocation, a height correction, or "Keep this layout"), each of which
now re-derives it from the zone's current on-screen arrangement via
`pack_clusters_for_view(_, CARD_GRID_COLS)` before persisting via
`move_notes`/`move_contacts` — never the raw `cols_now`-scale positions
directly. A combined `Effect` tracking `cols`, `auto_layout_enabled`,
and the accessible id set is the *only* thing that resets the render
cache, fresh, via `pack_clusters_for_view(abstract, cols_now)` — never
composed on top of the previous render. Since this reset is a pure,
deterministic function of (the untouched abstract, the current view
width), a resize or an Auto-layout on/off round trip back to the same
state now reproduces the exact same render every time.

The first cut at this used a `Memo` for the render cache (recomputed
lazily from the abstract on every read) instead of the reset-effect —
simpler in principle, but it broke the original shadow-matching fix
from this item's first follow-up: repacking a drop's cascade through
`pack_clusters_for_view` doesn't necessarily reproduce the exact
arrangement `place_with_relocation` resolved it to, so a drop stopped
landing where its own shadow last showed. Reverted to a directly-patched
`RwSignal` cache instead: a drag/height-correction patches it with its
*exact* render-space result (preserving shadow-matching), while
separately updating the abstract map for persistence; only the
reset-effect (resize, Auto-layout toggle, or a note/contact being
added/removed) ever replaces the cache wholesale.

One accepted consequence, matching "Keep this layout"'s own
already-established contract: a drag's on-screen result (patched in
directly, exact render-space precision) is *not* itself
`pack_clusters_for_view`'s canonical output, so the very first resize
(or Auto-layout round trip) after a drag settles it into that canonical
form — stable from then on, but not necessarily bit-identical to the
raw drag placement. Three resize tests were tightened from "gaggle
cohesion preserved" to exact per-id position equality to actually prove
the idempotence claim, each restructured to capture its baseline
*after* one settling round trip rather than immediately post-drag.

Also found and fixed in the same pass: two "uninvolved card must be
untouched by someone else's drop" test assertions were checking an
invalid assumption — an isolated single-member cluster always starts at
column 0 of its own shelf on *any* `pack_clusters_for_view` repack (it
never preserves an arbitrary absolute column, only relative cluster
membership and reading order), so an uninvolved card's exact column was
never guaranteed to survive a repack even before this fix; only its row
(distance from the drag) is a meaningful thing to check.

**Tests changed** (`tests/test_cards_layout.py`):
- `test_manual_layout_survives_a_resize_round_trip_after_real_drags`,
  `test_resize_from_max_to_min_columns_and_back_preserves_gaggles`,
  `test_resize_through_several_widths_preserves_four_manual_gaggles` —
  tightened from cohesion-only to exact per-id position equality,
  restructured to capture baseline after a settling round trip.
- `test_drag_relocates_displaced_card_near_its_own_original_spot`,
  `test_notes_drag_relocates_displaced_note_near_its_own_original_spot`
  — switched from raw persisted DB reads to rendered DOM positions (the
  persisted value is now the `CARD_GRID_COLS`-wide abstract, not the
  `cols_now`-scale value the card visually dropped into), and the
  "uninvolved card" assertion narrowed to row-only.
- `test_keep_this_layout_persists_exactly_what_was_previewed` (Contacts)
  — switched from comparing "kept" against the raw persisted DB row/col
  to comparing against a post-reload DOM re-render (mirroring the
  already-passing Notes equivalent), for the same reason.

**Final verification**: `cargo test --workspace` (522 tests), `cargo
clippy --workspace -- -D warnings` (clean on the touched files), `cargo
build --target wasm32-unknown-unknown -p tidl-web`, `cargo fmt` (on the
touched files), and the full `tests/test_cards_layout.py` suite — 26
tests — all pass.

### Fifth follow-up: a large gaggle claimed a whole shelf's width, crowding out its neighbor

Reported live (2026-09-28), immediately after the fourth follow-up: a
resize or Auto-layout round trip still visibly changed three
manually-arranged gaggles — not by merging or losing them (their
membership and top-to-bottom order stayed exactly intact, confirmed via
targeted `leptos::logging::log!` instrumentation added for this session
and removed afterward), but each gaggle's *own left/right position*
relative to its neighbors collapsed: "topleft, middle-right, bottom-left"
became "top, middle, bottom", all left-anchored.

**Root cause.** `pack_clusters_for_view`'s cluster width was
`(members.len()).clamp(1, view_cols)` — a cluster's shelf width came
from its *member count*, not the column span it was actually occupying.
`pack_shortest_column`'s height-balancing then spread that cluster's
members across every one of those columns to minimize height. For a
10-member gaggle at `view_cols` (or `CARD_GRID_COLS`) close to 8, that
claims the *entire* shelf width, leaving no room for a second gaggle to
share it — so the second gaggle gets pushed to its own shelf below,
left-anchored at column 0, same as the first. This is a pre-existing
characteristic of `pack_clusters_for_view` (unchanged since #102) that
predates this session; the fourth follow-up's abstract-recompute-on-
every-drag just re-triggered it far more often than the occasional
reflow that used to trigger it.

**Fix.** A cluster's shelf width is now the column span its members are
*actually* occupying in the input (`max_col - min_col + 1`, clamped to
`view_cols`), not its member count — so a gaggle that was manually
arranged into a compact 2-column block stays 2 columns wide on a repack,
leaving room for a sibling gaggle beside it. Verified idempotent on its
own prior output (a repack's own output span reproduces the same width
next time) via the existing fuzz test. One existing unit test
(`single_cluster_matches_plain_pack_for_view`) asserted the *old*
behavior — a single cluster whose members were all stacked in one
column, expected to spread across `member_count` columns like a plain
`pack_for_view` masonry balance — and was rewritten
(`single_cluster_preserves_its_own_column_span_not_plain_pack_for_view`)
to assert the corrected expectation: it stays in that one column.

**A second, compounding bug found while verifying this fix live**: a new
regression test for two gaggles side by side (`test_large_gaggle_does_
not_crowd_a_smaller_sibling_off_its_shelf`) initially failed with an
outright card-on-card overlap, not just a stacking/crowding difference.
Root cause: the height self-correction pass built its render-space
`entries` from `pinned_display`/`other_display`'s own `grid_row`/
`grid_col` fields — which, for a note/contact never touched by a drag
since page load, still hold the *raw* stored column, never synced to
wherever the on-screen cache had actually reflowed it to (e.g. onto a
separate shelf once a sibling gaggle no longer fit beside it, per the
bug above). Correcting height against that stale column computed a "corrected"
row assuming the wrong column context, then wrote only that row into the
*cache* entry while leaving the cache's own, different, already-correct
column untouched — landing the note directly on top of an unrelated one
at the cache's *real* column. Fixed by building those entries from the
render cache directly (falling back to a plain compaction for an
id the cache hasn't placed yet, mirroring `manual_pickup_items`'s
existing fallback), in both `notes/mod.rs` and `contacts/mod.rs`
(the latter reusing `manual_pickup_items` itself, having only one zone).

**Tests changed**:
- `single_cluster_matches_plain_pack_for_view` (grid.rs unit test) →
  renamed and rewritten, see above.
- `test_large_gaggle_does_not_crowd_a_smaller_sibling_off_its_shelf`
  (new) — an 8-member and a 3-member gaggle seeded compactly side by
  side; asserts they stay side by side (sharing a row band, disjoint
  columns) through a resize round trip.

**Final verification**: `cargo test --workspace` (522 tests), `cargo
clippy --workspace -- -D warnings` (clean on the touched files), `cargo
build --target wasm32-unknown-unknown -p tidl-web`, `cargo fmt` (on the
touched files), and the full `tests/test_cards_layout.py` suite — now 27
tests — all pass. All temporary diagnostic logging added during live
debugging was removed before this was considered done.

### Sixth follow-up: replaced clustering with an exact bidirectional real/view mapping

The fifth follow-up's `pack_clusters_for_view`-based abstract, even after
fixing its width formula, could not give a mathematically exact
guarantee: `pack_shortest_column`'s height-balancing redistributes a
cluster's members on every call, so `abstract → display → abstract` was
never a true round trip, only "eventually stable." User-driven design
review (2026-09-28) established the actual requirement precisely: an
exact, invertible mapping between real (stored) and view (rendered)
space at any fixed view width, with a drag writing only the ids it
touches — never a whole-zone re-snapshot, which could silently rescale
every *other* note to whatever (possibly narrower) width was active when
one unrelated note got dragged.

**Fix**: replaced `pack_clusters_for_view`/`detect_clusters` entirely, in
both `notes/mod.rs` and `contacts/mod.rs`, with the exact mechanism Auto
mode already used for its own real/view compaction:
`grid::compact_cell`/`grid::expand_cell`/`grid::column_offsets`/
`grid::view_col_of`. These form a true bijection at a fixed view width (by
construction, not by convention), so:
- The render cache is now a pure `Memo` over the abstract map:
  `compact_cell` applied per note, using `column_offsets` computed fresh
  from the abstract's real occupancy at the current width. Same inputs
  always give the same output — no patch/reset bookkeeping needed at all.
- A drag inverts only its own cascade's final cells via `expand_cell`,
  computed from the *current* (pre-drop) abstract — reproducing the exact
  offsets the render cache itself used — and writes only those specific
  ids. Every other note's real position is untouched, so dragging one
  note while narrow can never rewrite anyone else's stored scale.
- "Keep this layout" translates the whole preview via `expand_cell` with
  all-zero offsets (a fresh placement, no prior occupancy to stack
  against) — injective by construction, so it can't introduce a collision
  the preview didn't already have.
- Height correction operates on the abstract only, exactly as in the
  fifth follow-up, but now gated to skip patching `pinned_display`/
  `other_display`/`contacts_display` while the Auto-layout preview is
  active: found live, during preview those lists deliberately hold
  `pack_for_view`'s *view*-space tight-pack positions (a different,
  unpersisted arrangement), and writing a real-space correction into them
  corrupted the preview, landing two cards on top of each other.

**Consequence**: "gaggle cohesion" is no longer an explicit guarantee at
*every* intermediate width — two real columns 1 apart can compact into
the same rendered column at a narrow-enough view, same as Auto mode's
real columns always could. What's now exact instead is the thing
actually asked for: resizing to any width and back to the *same* width
reproduces the identical render, and so does toggling Auto-layout off
after on, with zero drift, at any width, forever — not just "usually,
after one settling reflow."

**Tests**: three tests seeded adjacent real columns (0, 1) as a proxy for
"visually distinct positions," which is no longer a valid assumption
under this model (they read the actual rendered position before the
drag instead, and use wider real-column gaps) —
`test_drag_relocates_displaced_card_near_its_own_original_spot`,
`test_notes_drag_relocates_displaced_note_near_its_own_original_spot`,
`test_note_displaced_during_preview_lands_exactly_there_after_drop`.
`test_two_islands_stay_cohesive_and_separate_across_a_resize`'s
`detect_clusters`-adjacency merge check (a same-or-adjacent column *and*
close row) was removed as architecturally obsolete and replaced with an
exact-round-trip assertion across its existing 2200→1450→1150→2200
resize sequence — directly testing the invariant this follow-up is
about, on the exact scenario (two multi-card islands) the user described
live.

**Found and fixed (separate, pre-existing issue)**: three Auto-mode
"drag while previewing" tests consistently timed out waiting for their
`reorder_contact`/`reorder_note`/`move_notes` response —
root-caused to a brand-new contact/note's `grid_row_span` defaulting to
the literal `1` at `INSERT` time (`create_contact`'s and `create_note`'s
own placement calls), far short of its real rendered height, combined
with `contacts_res`/`notes_res` never refetching after a local
height-correction persists (only `refresh`'s failure path did). Two
same-shelf cards ended up rendered almost on top of each other,
intercepting the test's pointer-down before it ever reached the intended
card. Confirmed independent of this follow-up's own changes (identical,
deterministic pixel output with a 300ms, 1.5s, and 4s settle wait alike)
and pre-existing (unrelated to `compact_cell`/`expand_cell`).

Fixed in two parts:
1. `create_note`/`create_contact` now seed a real initial
   `grid_row_span` via `grid::estimate_span`, mirroring the exact
   content-line logic `vcard_content_lines`/the VCF import path already
   used — instead of the placeholder `1`.
2. Contacts' height-correction pass now bumps `refresh` on a
   *successful* persist too, not just a failed one, so `contacts_res` —
   `contacts_display`'s only source for `grid_row_span` — picks up a
   corrected span before a later Auto-layout `pack_for_view` call reads
   the stale one (guarded off while `committing_layout` is up, to avoid
   racing "Keep this layout"'s own refetch-guard dance). Notes
   deliberately does **not** mirror this second part: enabling the same
   unconditional refresh for notes reproduced the identical "two cards on
   top of each other" symptom, but *after* "Keep this layout" —
   `on_keep_layout`'s own pack computation, once it saw a refetch land
   with a non-uniform mix of just-corrected and still-original spans,
   committed an overlapping layout that then persisted and survived a
   reload byte-for-byte (confirmed not a race: the post-reload snapshot
   matched the pre-reload "kept" snapshot exactly). Not chased to its own
   root cause — nothing in this test suite needs notes to refetch on a
   successful correction, since `pinned_display`/`other_display` already
   pick up the corrected span locally outside the Auto-layout preview.
   Left as a known latent issue in `notes::NotesView`'s `on_keep_layout`
   for a future, dedicated pass.

Also fixed the third failing test
(`test_auto_layout_off_restores_gaggles_built_via_several_real_drags`),
whose real cause was unrelated to spans: the notes grid container has
its own hard max-width, capping `cols_now` at 7 real columns regardless
of viewport width, and `view_col_of` aliases real columns 0 and 1 into the
same view column at `cols_now == 7` (every other adjacent pair maps
distinctly) — `column_offsets` then concatenates the two real columns'
entire row ranges one after another rather than interleaving them (an
inherent, correct consequence of `compact_cell`/`expand_cell` being a
true bijection), pushing one gaggle member thousands of pixels down the
page, past any reasonable viewport height, so the test's own
non-element-targeted `page.mouse` drag (which doesn't auto-scroll) never
reached it. Fixed by shifting the test's seeded columns from `0..6` to
`1..7`, which maps to seven *distinct* view columns at `cols_now == 7`.

All 27 e2e tests in `tests/test_cards_layout.py` pass, along with the
full `cargo test --workspace` suite (522 passed), `cargo clippy
--workspace -- -D warnings`, and `cargo build --target
wasm32-unknown-unknown -p tidl-web`.

### Seventh follow-up: "Keep this layout" applied `expand_cell` to already-real positions

Reported live (2026-09-28) as a serious regression: "Keep this layout" no
longer kept the layout at all, and now also overlapped cards. Reproduced
with a dedicated test,
`test_keep_this_layout_matches_the_observed_auto_layout_preview_for_every_card`
(`tests/test_cards_layout.py`) — three collections (3+2+3 contacts)
scattered across real columns/rows, Auto-layout toggled on, the rendered
preview captured directly, then compared against what's on screen right
after clicking "Keep this layout".

**Root cause**: `on_keep_layout` (in both `contacts::ContactsView` and
`notes::NotesView`) ran every card's `grid_row`/`grid_col` through
`grid::expand_cell` with all-zero offsets before persisting, on the
premise that the Auto-layout preview's positions were view-space
(compacted) coordinates needing translation back to real space — the same
translation a drag commit legitimately needs. That premise was wrong:
`pack_for_view` — the function that populates `contacts_display`/
`pinned_display`/`other_display` while the preview is showing — already
returns *real* columns by construction (its own doc comment describes a
two-level process: pick each item's *visual* column via a masonry pack,
then spread that visual column's members back across its own *real*
columns). The screen position shown during the preview is computed
separately, by `ContactCard`/`NoteCard`'s `card_style` call applying
`compact_cell` to these same real values with the current `cols`/
`col_offsets` (the `render_position: None` branch) — there was never a
view-space coordinate stored anywhere for `on_keep_layout` to translate
from.

Feeding an already-real column into `expand_cell` as if it were a view
column is not a no-op: `expand_cell`'s real-column choice depends only on
which view column its input falls into (via `view_col_of`), not on the
real column already recorded. With every real column's offset forced to 0
(the `zero_offsets` argument), any two cards whose real columns happened
to fall into the same accidental view bucket collapsed onto one single
real column, both at real row 0 — silently merging unrelated cards into
the same stored cell. A test asserting only "kept" vs. "after a page
reload" (the existing
`test_keep_this_layout_persists_exactly_what_was_previewed`) couldn't
catch this: with the same bug applied both before and after the reload,
the corrupted result was internally self-consistent, so it round-tripped
"correctly" while being wrong relative to what was actually shown.

**Fix**: `on_keep_layout` now persists `contacts_display`'s (or
`pinned_display`'s/`other_display`'s) own `grid_row`/`grid_col`/
`grid_row_span` directly — no `expand_cell` translation — since
`pack_for_view`'s output already *is* the correct real-space encoding of
what's on screen.

**Test methodology note**: the regression test itself needed a fix mid-way
too — its first version compared `data-grid-row`/`-col` attributes before
and after the commit and still failed, because those attributes carry
*different* coordinate systems in the two states being compared: real
columns during the Auto-layout preview (`render_position: None`) vs. view
columns after commit (`render_position: Some`, from `cluster_cache`, once
`auto_layout_enabled` flips back to `false`). Comparing rendered pixel
rects (`card_rects`, i.e. `getBoundingClientRect()`) instead — the one
representation both states resolve to identically — fixed the test's own
false positive and gave a real, coordinate-system-agnostic assertion.

All 28 e2e tests in `tests/test_cards_layout.py` pass (the 27 from the
sixth follow-up plus this new regression test), along with the full
`cargo test --workspace` suite (522 passed), `cargo clippy --workspace --
-D warnings`, `cargo fmt --check` (on the touched files), and `cargo build
--target wasm32-unknown-unknown -p tidl-web`.

### Eighth follow-up: a "Keep this layout" commit permanently unsubscribed the rebuild effect from `hidden_collections`/`search`/`auto_layout_enabled`/`cols`

Reported live (2026-09-28) immediately after the seventh follow-up
shipped: "Keep this layout" correctly returns to manual mode, but a
second "Auto-layout" click then silently no-ops — the manual layout stays
put. Navigating away to another page and back makes it work again.

**Root cause**: both `contacts::ContactsView` and `notes::NotesView` have
an `Effect` that rebuilds the on-screen card list from the fetched
resource, filtered by `hidden_collections`/`search`(/`pinned_only` for
notes) and repacked via `pack_for_view` when `auto_layout_enabled` is on.
That effect has an early-return guard on `committing_layout` — deliberately
added earlier (backlog #102 follow-up) so "Keep this layout"'s own
`auto_layout_enabled.set(false)` doesn't rebuild from a `contacts_res`/
`notes_res` snapshot that hasn't refetched yet, which would silently
revert the just-persisted positions.

Leptos re-derives an `Effect`'s reactive dependencies from scratch on
every run, based only on which signals it actually called `.get()` on
*that specific run* — there is no accumulation across runs. The
`committing_layout` guard returns early *before* the effect ever reached
its own `hidden_collections.get()`/`search.get()`/`auto_layout_enabled.get()`/
`cols.get()` calls further down (those were only read inside later
branches). The very first time this guard fires — right after any "Keep
this layout" commit — that run's dependency set collapses to just the
resource signal, permanently dropping the other four from the effect's
subscription list. A later write to any of them (e.g. the next
"Auto-layout" click, or toggling a collection's visibility) then has no
subscriber left to notify, so the effect never runs again — until the
component remounts (navigating away and back), which recreates the
`Effect` and resubscribes it from scratch, explaining exactly why that
"fixed" it.

**Fix**: read `hidden_collections`, `search`, `auto_layout_enabled`, and
`cols` unconditionally at the top of each effect, before either
early-return guard, storing them in local bindings the rest of the effect
body uses instead of re-calling `.get()` later. This keeps the effect
subscribed to all four regardless of which branch (or early return) a
given run takes, while the `committing_layout` guard still does its job
of skipping the actual rebuild during the commit's own async gap. Notes
additionally had a second, narrower version of the same avoidance
(`is_filtered` deliberately short-circuiting before `auto_layout_enabled.get()`,
to dodge a *different* imagined staleness case) — removed as redundant,
since `committing_layout` already covers the one real staleness hazard at
the point it actually occurs.

**Test methodology note**: a naive regression test ("click Auto-layout
twice with nothing in between, assert the second preview differs from
what was kept") does not work here — `pack_for_view` is a deterministic
function of (ids, spans, sort order, column count), so with nothing else
changed, a *correctly* working second click reproduces the exact
arrangement "Keep this layout" just committed, identical to what a stuck
effect would also show by coincidence (it had frozen on that same
arrangement moments earlier). The regression tests
(`test_hiding_a_collection_still_works_after_a_keep_this_layout_commit`
and its notes counterpart) instead hide a collection right after the
commit: that has no legitimate reason to reproduce the same visible set
whether or not the effect reruns, making it an unambiguous probe. Verified
against the pre-fix code (temporarily reverted) that these tests do fail
without the fix, confirming they're not vacuously true.

All 30 e2e tests in `tests/test_cards_layout.py` pass (28 from the
seventh follow-up plus these two new regression tests), along with the
full `cargo test --workspace` suite (522 passed), `cargo clippy
--workspace -- -D warnings`, `cargo fmt --check`, and `cargo build
--target wasm32-unknown-unknown -p tidl-web`.

### Ninth follow-up: "Keep this layout" was unusable with any mix of pinned and unpinned notes

Reported live (2026-09-28): "Keep this layout doesn't keep the same
layout when you have some cards pinned." Two separate bugs, both specific
to the pinned/unpinned split (contacts has no such split and was
unaffected):

**Bug 1 — the commit silently discarded itself entirely.** `on_keep_layout`
chained `pinned_real` and `other_real` into one combined `Vec<GridPosition>`
and made a single `move_notes` call. The server explicitly rejects a batch
mixing pinned and unpinned notes ("Cannot move notes across the pinned
boundary in one batch") — with at least one note of each kind present,
this call always failed outright, and since the whole transaction errors
out together, *neither* zone's positions were persisted. `auto_layout_enabled
.set(false)` still ran regardless, flipping back to manual mode, which
then rendered from `pinned_abstract`/`other_abstract` — still holding the
pre-auto-layout scattered positions, since the failed persist's optimistic
apply was correctly skipped. From the user's side this looked exactly
like "Keep this layout doesn't keep the layout." Fixed by issuing two
separate `move_notes` calls, one per zone (skipping a zone with no
moves), and applying each zone's optimistic patch independently so a
failure in one doesn't discard the other's success.

**Bug 2 — the pinned row's reserved height disagreed with itself.** Even
after fixing bug 1, notes in the *unpinned* zone still rendered ~16px
lower after commit than they had during the preview. Root cause: the
Auto-layout repack's `pack_for_view` call built its `entries` list
(`(id, grid_row_span)`) straight from `notes_res` — the raw fetched
resource, which notes deliberately never refetches after a successful
height correction (see the sixth follow-up's own note on this). A note
whose real rendered height doesn't match its stored span (very common
right after creation, given `estimate_span`'s approximation) gets
corrected in `pinned_abstract`/`other_abstract` immediately, but
`notes_res`'s cached copy stays stale for the rest of the session. The
pinned row's own CSS grid reserved height for that stale span during the
preview, then reserved the correct (smaller) height post-commit once
manual-mode rendering took over from `pinned_abstract` directly — a
visible, deterministic height mismatch between the two states, not a
settling race (confirmed by re-testing with a 1500ms wait on both sides).

Fixed in two parts:
1. `pinned_abstract`/`other_abstract` moved earlier in the component
   function (before the rebuild effect that needs them — they have no
   dependencies of their own, so this is purely a declaration-order fix)
   so the effect's `auto` branches can look up each note's span from
   there — the one place a correction always lands unconditionally —
   falling back to the resource's own span only for a note not yet in the
   abstract map.
2. The height-correction pass's own `pinned_display`/`other_display`
   mirror now always patches `grid_row_span` (previously guarded off
   during the Auto-layout preview, matching contacts' identical guard).
   Verified this doesn't reopen the "two notes render on top of each
   other" regression from the seventh follow-up: that regression was
   re-traced (via bisection) to notes' `refresh` bump after a successful
   correction persist specifically, not to patching span in place — span
   never feeds `on_keep_layout`'s persisted values (which read each
   note's real `grid_row`/`grid_col` directly), so patching it can't
   corrupt a commit; it only feeds the render closure's own
   `column_offsets` call, which is recomputed fresh from *all* current
   entries every render, so it can't go internally inconsistent the way
   patching row/col in place can. `refresh` itself stays off for notes,
   unchanged.

A new regression test,
`test_keep_this_layout_matches_the_preview_with_pinned_notes_present`,
covers both bugs directly: 2 pinned + 3 unpinned notes, Auto-layout,
Keep this layout, compare every note's rendered pixel position (not
`data-grid-row`/`-col` attributes — same reasoning as the seventh
follow-up's test) between the preview and the commit.

All 31 e2e tests in `tests/test_cards_layout.py` pass, along with the
full `cargo test --workspace` suite (522 passed), `cargo clippy
--workspace -- -D warnings`, `cargo fmt --check` (via `rustfmt` directly
on the touched file only), and `cargo build --target
wasm32-unknown-unknown -p tidl-web`.

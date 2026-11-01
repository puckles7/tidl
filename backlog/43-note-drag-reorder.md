# Drag-and-drop note reordering

Let users rearrange notes in the notes grid by clicking and dragging a note
card to a new position, instead of only ever landing in
pinned/updated-at order.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — reorders the `NoteCard` grid it
  introduced.
- [42 - Note ordering — persisted manual order + pinned-group
  invariant](42-note-order-pinned-invariant.md) — provides the order field
  and the pinned/non-pinned boundary rule this feature must respect.

## Functional Requirements

- Clicking and holding a note card, then moving the pointer, picks it up for
  reordering; other cards live-reflow to make room at the dragged card's
  current hover position (not just a static insertion-line placeholder) —
  the layout should look like it's already landed there.
- The new position is only committed (persisted via #42's order field) on
  mouse release. Releasing over an invalid target snaps the card back to its
  original position with no change.
- Must respect the pinned-group invariant from #42: a drag can reorder a
  pinned note only within the pinned block, and a non-pinned note only
  within the non-pinned block. Attempting to drag across the boundary (a
  non-pinned note up into the pinned block, or vice versa) is disallowed —
  show a "no-drop" affordance at the boundary and snap back on release,
  rather than silently toggling the note's pinned state as a side effect.
- Works within the currently-visible/filtered set of notes (respects
  collection visibility, the "★ Pinned" filter, and search) — dragging
  reorders among what's on screen; the persisted order still applies
  globally once the filter is cleared.

## Open Questions

- Touch/pointer-drag support (mobile) vs. mouse-only for v1 — leave to
  implementation; at minimum this must work with mouse `mousedown`/
  `mousemove`/`mouseup`, matching the "clicking and dragging" ask.

## Resolution

Implemented as specified.

- **Backend**: a new `reorder_note(id, before_id, after_id)` server function
  (`crates/tidl-web/src/api/notes.rs`) sets `id`'s `sort_order` to the
  midpoint of its two new neighbors' `sort_order` values (or 1000 past
  whichever single neighbor exists at a group boundary), reusing #42's
  gap-based scheme. If the float gap between neighbors is exhausted by
  repeated drops at the same spot, the whole pinned group is renumbered
  first. Both neighbors must share the moved note's `pinned` state — the
  server-side half of the pinned-group invariant, independent of whatever
  the client UI allows.
- **Live reflow**: `NotesView` (`crates/tidl-web/src/components/notes/mod.rs`)
  replaced its `Suspend`-driven note list with two plain
  `RwSignal<Vec<NoteInfo>>` (one per pinned group), populated by an `Effect`
  off the existing resources (the leptos-suspense skill's "Effect +
  RwSignal workaround" — needed because drag reflow requires synchronous
  local mutation on every `pointermove`). Dragging does DOM hit-testing
  (bounding rects of `.note-card[data-note-id]` elements) against the two
  group containers to find the live insertion point and splices the
  dragged note into the display vec directly, so the grid visually
  reflows in place — matching the spec's "look like it's already landed
  there" rather than a floating ghost that follows the cursor.
- **Pointer/click disambiguation**: `NoteCard` now starts a drag candidate
  on `pointerdown`; the 5px travel-distance threshold (matching
  `week_grid.rs`'s existing `DragState` convention) decides click-to-open
  vs. drag on release, mirroring the calendar's drag-to-move event pattern.
- **Boundary enforcement (client)**: while dragging, hovering the pointer
  over the *other* pinned group's container freezes the live reorder and
  applies a `cursor: not-allowed` + dashed-outline affordance on that
  group; releasing there reverts the dragged note's group to its
  pre-drag order (snap-back with no server call), matching the spec.
- **Deviation**: touch support was left out, as the Open Question
  permitted — implemented with `pointerdown`/`pointermove`/`pointerup`
  (which also covers touch to the extent the browser maps touch to
  pointer events), but the drag mechanics were only exercised with mouse
  input during manual browser verification (headless Chromium via
  Playwright, not a committed test).
- Verified manually against a real `cargo leptos build` + browser session:
  live reflow within a group, order persisting across reload, cross-
  boundary no-drop snap-back, plain click still opening the note editor,
  and the ★ pin toggle still working without triggering a drag.

### Follow-up: "pick up and carry" feedback (2026-07-14)

Initial ship only reflowed neighbors when the pointer crossed into a new
slot — the dragged card itself stayed put (just dimmed) until that
threshold, so nothing visibly moved until a swap occurred. Added a live
`transform: translate3d()` on the dragged `.note-card` (`NoteCard`'s
`drag_offset` prop, driven by a `RwSignal<(f64, f64)>` in `NotesView`
kept separate from the main `drag` signal so per-pixel pointermoves don't
retrigger the full grid rebuild) so the card now visibly detaches and
tracks the cursor in real time between reflows, the way "picking it up"
implies. The offset baseline resets to zero on every reflow splice (so it
doesn't accumulate drift against the card's newly-reflowed layout box),
and a CSS transition on `.note-card` (suppressed via `transition: none`
on `.note-card--dragging` while actively held) animates it settling back
to identity transform on release. Dropped the old `opacity: 0.55` dimming
in favor of a stronger drop shadow + elevated `z-index`, matching a
"lifted" card rather than a "disabled" one.

### Follow-up: text-selection bleed during drag, and horizontal (masonry
column) reordering (2026-07-14)

Two more issues surfaced after the above:

- **Native text selection**: sweeping the pointer across other cards'
  text while dragging triggered the browser's native text-selection
  highlight. Fixed with a `notes-main--dragging` class (applied whenever
  `drag.get().is_some()`, not just once the 5px drag threshold is
  crossed) setting `user-select: none` on the drag container.
- **Horizontal reordering was effectively broken**: dragging within
  `.notes-grid` (a CSS multi-column masonry, `columns: 3 260px`,
  column-major fill) toward a different column would keep re-targeting a
  card in the *current* column, making the other column undroppable —
  reported as "dragging from the center column to the right, I never
  seem to be able to replace it; it jumps to the bottom of the centre
  column." Root-caused via a live-browser repro with temporary
  `leptos::logging::log!` tracing (removed before commit) to two distinct
  bugs, both extracted into pure, unit-tested functions in
  `crates/tidl-web/src/components/notes/mod.rs`:
  - `hit_test_target` — the original hit-test picked whichever candidate
    card was nearest by raw 2D distance, which ignores which
    column/row the cursor is actually over: a short second column would
    keep losing to a physically-closer card in the first column. Fixed
    by bucketing candidates by the primary axis first (column-left for
    the column-major masonry, row-top for `.notes-pinned-row`'s
    row-major CSS grid) before breaking ties on the secondary axis.
  - `reorder_insert_index` — even with the right target card chosen, two
    distinct array-splice issues could still land the drop in the wrong
    place: (1) CSS multi-column masonry balances columns by *count*
    across the whole array, not a fixed split point, so "insert
    immediately before the first card of the next column" is, by
    definition of column-major fill, actually *the end of the previous
    column* — correct only when the dragged item is already later in the
    array than the target column, wrong (and silently absorbed back into
    the current column) when it's earlier; (2) separately, and not
    masonry-specific, "insert X immediately before its own current
    array-successor" is *always* a no-op (remove-then-reinsert-adjacent
    trivially reproduces the original order) — this can freeze a drag
    dead the moment the nearest candidate happens to be the item already
    right next to the dragged one. Both cases nudge/flip the computed
    insertion index so a hit-test result that legitimately differs from
    the last one always produces a visible reorder.
  - Also reverted an intermediate attempt at preserving the exact
    "grab point" (pointer-to-card-corner offset) across column-crossing
    splices via re-querying the dragged card's fresh DOM position after
    each reflow — this proved fragile (the enclosing view block tears
    down and rebuilds the whole grid subtree on every splice, so the
    re-query could hit a stale or not-yet-relocated node and render the
    card hundreds of px off) in favor of the simpler, DOM-query-free
    "reset the follow-offset to zero on every splice" scheme, which
    can't observe stale state by construction. Trade-off: the card
    visually snaps to its new layout box on each splice rather than
    smoothly gliding, most noticeable when several splices fire in quick
    succession (e.g. dragging fast across a densely-packed column) —
    accepted as a minor cosmetic cost against the alternative's
    occasional wildly-wrong position.
- Verified via `cargo test` (11 new unit tests directly covering the
  masonry column-boundary and no-op-insertion scenarios, reproducing the
  exact layouts hit in the live-browser repro) and a real
  `cargo leptos build` + Playwright session: dragging left-to-right and
  right-to-left across masonry columns lands and persists correctly
  across reload; no native text selection while sweeping across other
  cards during a drag.

### Follow-up: jitter/flicker and the card jumping ahead of the pointer

Live use surfaced two more problems with the previous follow-up's design,
both reported directly against real dragging (not synthesized from a
repro): "jittery" placement flickering between two positions, and the
card sometimes jumping far more than the pointer had moved.

- **Root cause of the jump**: the previous fix's accepted trade-off —
  resetting the pointer-follow offset to `(0,0)` on every live-reorder
  splice, so the card's `transform` was always relative to its *current*
  (possibly just-changed) layout box — meant that whenever a splice moved
  the card's underlying layout slot (which a masonry-column reflow can do
  by a large distance), the card's rendered position snapped to that new
  slot with zero visual continuity, independent of where the pointer
  actually was. This was supposed to be "barely noticeable"; in practice
  it was the dominant visible glitch.
- **Fix**: decoupled the dragged card from the live-reordered layout
  entirely. On pickup, the card's `getBoundingClientRect()` and the
  pointer's offset into it (`grab_dx`/`grab_dy`) are captured once
  (`pointer_down_geometry`). While dragging, the card renders with
  `position: fixed` at `pointer - grab_offset` with an explicit
  width/height held steady from pickup (`NoteCard::card_style`), so its
  on-screen position is a pure function of the raw pointer position —
  never of the card's current array index or DOM slot. The underlying
  list still live-reorders (other cards reflow to open/close the gap),
  but the dragged card itself is now immune to how far or how often that
  reflow moves its slot.
- **Root cause of the jitter**: `hit_test_target` picked whichever
  candidate was nearest with no dead zone, so a pointer sitting near the
  boundary between two cards (or near a single card's own before/after
  midpoint) could flip the "nearest" pick on every pixel of jitter — and
  because a pick change immediately triggers a reflow that itself moves
  the very boundary being compared against, that flip-on-jitter became a
  visible flicker between two positions.
- **Fix**: `hit_test_target` now takes a `sticky: Option<(i64, bool)>` —
  the caller's current target (`NoteDragState::last_target`) — and
  requires a candidate to beat it by `DRAG_HYSTERESIS_MARGIN` (24px)
  before switching away from it, both for which card is picked and,
  once a card is picked, whether the cursor has moved decisively enough
  past its center to flip before/after. Covered by three new unit tests
  (`hysteresis_keeps_sticky_target_when_candidate_barely_closer`,
  `hysteresis_switches_once_candidate_clearly_closer`,
  `hysteresis_keeps_before_after_edge_until_clearly_crossed`).
- Verified via `cargo test` (workspace: 204 passed), `cargo clippy -D
  warnings` (clean), and `cargo build --target wasm32-unknown-unknown -p
  tidl-web --features hydrate` (clean). Not independently re-verified in
  a live browser session by the assistant this round — the user reported
  the bugs from live use and should confirm the fix the same way.

### Follow-up: drop-shadow placeholder, and a flicker regression it caused

Added a dashed "drop-shadow" placeholder (`.note-card-drop-shadow`,
styled like `.notes-add-tile`) rendered at the dragged note's live
array position, so the landing slot is visible while the actual card
floats separately as the `position: fixed` overlay. Sized from
`drag_size` to match the dragged card, so nothing else shifts when it
takes the real card's place in the flow.

This reintroduced flicker (dragged note flipping between its old and
new slot) that the earlier hysteresis fix had resolved. Root cause:
before the placeholder, the dragged card contributed *zero* footprint
to the masonry/grid layout wherever it sat in the array (a
`position: fixed` element takes no flow space), so reordering it within
the array never moved any *other* card's rect — the hit-test geometry
was inherently stable between drag decisions. The placeholder changed
that: it *does* occupy space, so moving it to a new array index (a
committed reorder) can shift other cards' rects, sometimes by a whole
masonry column. `on_notes_pointer_move` re-queried `.note-card` rects
from the DOM on every single pointermove, so a commit's own layout
side effect was immediately fed back into the very next hit-test
decision within the same handler tick — sometimes flipping the target
straight back, before the user had moved the pointer again.

Fixed by caching the queried rects (`own_card_rects:
RwSignal<Option<Vec<CardRect>>>`) and only refreshing them right after
a commit actually changes the layout (or at pickup) — not on every
pointermove. Since nothing in the layout can change between commits
except through this handler itself, reusing the last snapshot until
the next real pointer movement is always correct, and it breaks the
same-tick feedback loop: a hit-test decision is now always made
against geometry from *before* that decision, never geometry the
decision itself just produced.
- Verified via `cargo test` (workspace: 204 passed), `cargo clippy -D
  warnings` (clean), `cargo fmt` (clean), and
  `cargo build --target wasm32-unknown-unknown -p tidl-web --features
  hydrate` (clean). Live-browser re-verification is again on the user,
  per the same sandbox limitation noted above.

### Follow-up: the caching fix above still flickered — a deeper redesign

The rect-cache-with-invalidation-on-commit fix above did not actually
fix it. Reproduced with an isolated Playwright script (own TidlServer,
own temp DB/port, `DISABLE_SERVER_FN_HASH=1` build in a scratch copy of
the repo — never touching the dev `cargo leptos watch` instance or its
`target/`) driving a slow, monotonic drag of the bottom card up through
a vertical stack of notes, polling `.notes-grid .note-card` DOM order
on every step. With the caching fix, dragging past just one boundary
was clean, but dragging further — past a second, third card — reliably
reverted the array to an earlier state mid-drag despite the pointer
moving in one steady direction the whole time, exactly matching "the
shadow card and the remaining note continually trade places."

**Root cause**, found by manually tracing the exact recorded step
sequence against `reorder_insert_index`: swapping two adjacently
stacked same-size cards moves the displaced card's center by roughly
its own height (~126px in the repro) — far more than any reasonable
hysteresis margin (24px) can absorb. Re-querying live post-commit
geometry (even only once per commit, as the caching fix did) feeds
that shift straight back into the next hit-test, which reads it as the
cursor now being decisively on the *other* side of the just-moved
card and swaps right back.

**Fix, part 1 — stop depending on live geometry at all.**
`own_card_rects` is now a `Vec<CardRect>` snapshotted *once*, at
pickup (`snapshot_card_rects`), and never refreshed for the rest of
that drag — replacing the query-on-every-pointermove logic in
`on_notes_pointer_move` entirely. Since `reorder_insert_index` only
ever moves the dragged note (every other note keeps its relative
order), the *identity* of "which slot a target id represents" stays
meaningful all drag long even though array positions shift; freezing
the geometry just means hit-testing always answers "which of the
original slots is the cursor over," a question that stays well-defined
no matter how many times the array has been spliced so far — and,
crucially, is now completely decoupled from any layout side effect the
drag's own reflow (or the drop-shadow placeholder) causes.

**Fix, part 2 — the old "no-op nudge" was actively fighting the new
hysteresis.** `reorder_insert_index` used to nudge the dragged note one
slot further whenever the naive computed index equalled its own
current one, on the theory that a hit-test result different from the
last one should always produce a visible move. That's no longer true:
`hit_test_target`'s hysteresis band around a *single* target's own
before/after midpoint can legitimately flip `before` for the *same*
`target_id` as the cursor drifts within that band — and once the
dragged note has already landed adjacent to that target from an
earlier commit, the flip's naive insertion point is exactly the
dragged note's own current position. Nudging away from it anyway
undid the placement hysteresis had only just correctly settled on.
Fixed by making `to == from` a genuine no-op (`None`, no splice)
instead of a forced nudge — verified this doesn't reintroduce the
original "drag gets stuck" concern the nudge existed for, since a
*new* target (reached as the cursor keeps moving) is never a
same-position no-op. Two unit tests, formerly
`insert_before_immediate_successor_is_not_a_silent_noop` and
`insert_after_immediate_predecessor_is_not_a_silent_noop`, were
renamed to `insert_before_immediate_successor_is_a_real_noop` /
`insert_after_immediate_predecessor_is_a_real_noop` and rewritten to
assert `None`.
Re-verified the whole `hit_test_target`/`reorder_insert_index` pairing
against the exact recorded repro sequence by hand before touching the
code, then again with the Playwright script after: a 6-card drag from
bottom to top now produces exactly 5 order-change events — one per
card boundary crossed, strictly monotonic, zero reversions — including
20 rounds of ±2px jitter dwelling at the final boundary (the "slow
drag" condition the user specifically called out) with no further
change, and a correct `reorder_note` persist (HTTP 200) on drop.
- Verified via the Playwright repro above (isolated build/DB/port, own
  scratch copy of the repo — cleaned up afterward), `cargo test`
  (workspace: 204 passed — same count, two tests renamed not added),
  `cargo clippy -D warnings` (clean), `cargo fmt` (clean), and
  `cargo build --target wasm32-unknown-unknown -p tidl-web --features
  hydrate` (clean). The developer's live `cargo leptos watch` instance
  and its `target/` were never touched by any of this round's builds.

### Superseded by backlog #47

The reflow/hysteresis model this file documents — a single linear
`sort_order`, other cards live-splicing to fill the gap around the
dragged one — was replaced entirely by
[47 - Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md):
notes now hold an explicit `(grid_row, grid_col)` and don't auto-reflow
when one moves. `hit_test_target`, `reorder_insert_index`, and the
hysteresis/frozen-snapshot machinery described above no longer exist in
the codebase — replaced by #47's `cell_at`/`resolve_drop`. This file is
kept as-is (not rewritten) as a record of what was tried, and why, across
several rounds of live-observed bugs — several of #47's own design
choices (frozen-at-pickup geometry snapshots, avoiding same-tick
feedback loops, verifying via an isolated Playwright build) came directly
from lessons documented here.

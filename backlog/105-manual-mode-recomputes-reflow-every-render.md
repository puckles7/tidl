# Manual mode re-derived cluster layout on every render, not just on resize

Manual mode's cluster-aware rendering (backlog #102) called
`grid::pack_clusters_for_view` fresh inside the render closure on *every*
re-render — not only when the column count actually changed. Since that
function re-detects every gaggle and re-ranks/re-shelves all of them from
scratch, an ordinary drag (or even just the height self-correction pass)
could silently reshuffle every *other* gaggle on screen too, because the
dragged card's own new position changes its cluster's ranking key among
all the others. In practice this meant a manually-dragged arrangement
never actually stuck, even at a perfectly stable column count — reflow
should only be a resize-time (or accessible-id-set-change) event, not
something that happens on every render.

## Dependencies

- [102 - Cohesion-aware grid reflow (gaggle-preserving resize)](102-cohesion-aware-grid-reflow.md) — the rendering path this item changes from "always recompute" to "cache, patched in place".
- [104 - Manual-mode drag shadow snaps to the wrong column mid-drag](104-manual-drag-shadow-wrong-column.md) — found investigating the same live-testing session; a distinct bug in the same machinery.

## Context / Background

Surfaced 2026-09-27 while building a regression test for two independent
4-card collections dragged into 2x2 "islands": dragging one card visibly
shifted the *other*, untouched collection's cards around on screen, even
though nothing about their own stored position had changed and the
column count hadn't either.

Root-caused by comparing against pre-existing, uncommitted work from a
parallel session (recovered from `git fsck`'s dangling-commit list after
being prematurely dropped as a stash) that had already designed and
partially built the right fix — a `cluster_cache` signal, refreshed only
by an `Effect` gated on the accessible id set and column count, patched
directly for just the ids a drag/self-correction/"Keep this layout"
commit actually touched — but against an older, pre-#103 version of
`contacts::ContactsView`/`notes::NotesView`. This item ports that design
onto the current (post-#103) code.

## Functional Requirements

- Dragging a card, or the height self-correction pass adjusting a span,
  must never change any *other* card's rendered position.
- A real reflow trigger — the column count changing, or the accessible
  id set changing (a card created/deleted, or a note pinned/unpinned
  moving it between zones) — must still fully re-cluster and re-rank, so
  resize-driven cohesion (backlog #102) keeps working exactly as before.
- Applies to both Contacts (one cache) and Notes (one cache per zone,
  pinned/other).

## Resolution

Added `cluster_cache` (Contacts) / `pinned_cluster_cache` +
`other_cluster_cache` (Notes) — `RwSignal<HashMap<id, (row, col, span)>>`
holding the last full reflow. A companion `Effect`, gated on a `Memo` of
the accessible id set (from the unfiltered server resource, matching
#103's hidden-collection-isolation fix — Notes' memo tracks `(id,
pinned)` pairs so a pin/unpin toggle also triggers a refresh) plus the
column count, repopulates it via `pack_clusters_for_view` from scratch.
Everywhere else that used to call `pack_clusters_for_view` inline inside
the render closure now just reads the cache directly.

Four call sites patch the cache in place instead of waiting for the
gated `Effect` to notice (none of them change the id set or column
count): the Manual-mode drag-drop handler, the height self-correction
pass, and `"Keep this layout"` — the last of these mirrors
`contacts_display`'s own existing `committing_layout` race guard exactly
(same hazard: the post-persist `auto_layout_enabled.set(false)` write
would otherwise retrigger the cache's `Effect` against the still-stale
pre-persist resource snapshot).

Verified live (`gts` + a scripted browser session, not just the new
automated tests below): dragging one card no longer moves any other
gaggle; two 4-card collections dragged into 2x2 blocks each stay their
own cohesive, non-mixing island across a resize crossing the "columns <
4" reshape threshold, elongating vertically as expected. Two new
Playwright tests in `tests/test_cards_layout.py`
(`test_manual_drag_shadow_tracks_the_pointers_real_column` and
`test_two_islands_stay_cohesive_and_separate_across_a_resize`) cover
this plus backlog #104's shadow-tracking fix — the latter is the first
test in the suite that samples `.grid-drop-shadow` mid-drag rather than
only asserting on the final dropped position.

`card_style`'s own cluster-space rendering of an *other*, cascade-
displaced card (not the dragged one, not the cached value) is unrelated
to this cache and still goes through the same reverse-translation path
backlog #104 didn't touch there — see that item's Resolution for why
that specific gap is accepted as-is.

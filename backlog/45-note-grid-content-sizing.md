# Notes grid — size cards by their own content, not row-mates

Google-Keep-style masonry layout for the Notes grid: each card's height
should follow its own content, instead of every card in a CSS grid row being
stretched to match the tallest card in that row.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — introduced `.notes-grid` / `NoteCard`.

## Context / Background

`.notes-grid` (`assets/style.css`) is a plain CSS grid
(`grid-template-columns: repeat(3, 1fr)`). CSS grid sizes each row to its
tallest cell, so a short note that happens to land in the same row as a long
one gets visually padded out to match — card height currently depends on
row-mates, not on the note's own text length.

True masonry (items packed by height, independent per column) isn't
solvable with plain CSS grid `auto-rows`. CSS's native `masonry` value for
`grid-template-rows` is still experimental/behind flags in most browsers, so
this likely needs either:
- CSS multi-column (`columns: 3; column-gap: ...`) with `break-inside:
  avoid` on `.note-card` — simplest, but items flow top-to-bottom
  column-by-column rather than left-to-right, which reorders cards visually
  relative to their list order (may or may not be acceptable — TBD).
- A small client-side bin-packing pass (JS/WASM) that measures rendered
  card heights and assigns explicit `grid-row`/absolute positioning per
  card — matches Keep/Pinterest exactly but adds layout logic and a resize
  listener.

## Functional Requirements

- To be worked out further. At minimum: a note card's height must be driven
  by its own text/badge content, not by the tallest card sharing its grid
  row.
- Should degrade sensibly to a single column on narrow viewports (existing
  `.notes-grid` likely already has a responsive breakpoint — verify and
  preserve).

## Open Questions

- Multi-column (reflow order) vs. JS bin-packing (preserves list order,
  more code) — pick an approach before implementing.
- Does visual reordering (multi-column approach) matter given notes are
  otherwise ordered by pinned/updated_at, or manual order per #42/#43?
- Interaction with pinned-group ordering invariant from #42 — masonry
  packing must not silently violate "pinned notes stay together" once #42
  lands.

## Resolution

Went with the CSS multi-column approach (`columns: 3 260px; column-gap:
16px;` on `.notes-grid`, `break-inside: avoid; margin-bottom: 16px;` on
`.note-card` and `.notes-add-tile`), not JS bin-packing — no new dependency,
no resize-listener/layout logic needed, and it's a one-file CSS change.
`.notes-empty`'s `grid-column: 1 / -1` was dropped since it's meaningless
outside a grid container; it now just flows as a normal block, which is an
acceptable no-op since it only appears alongside the empty state.

`columns: 3 260px` (count + min width, not a fixed count) also gives
responsive column-count for free — narrower viewports automatically collapse
toward one column without a media query, addressing the "degrade to single
column" requirement without extra rules.

Deviation from the open questions: did not resolve the reflow-order vs.
list-order question with the user before implementing — cards now flow
top-to-bottom per column (column-major) rather than left-to-right
(row-major), so visual order no longer strictly matches list order
(pinned-first / most-recently-updated-first). Flagged as a known tradeoff,
not verified against user expectations. The #42 pinned-group interaction is
still open and unresolved — #42 isn't implemented yet, so there was nothing
concrete to reconcile against.

**Follow-up**: the column-major tradeoff above turned out to be a real
problem in practice — pinned notes (sorted first in the list) all piled up
in the left column instead of appearing across the top, since CSS
multi-column fills one column completely before starting the next. Fixed by
splitting the notes list into two containers: pinned notes render in a new
`.notes-pinned-row` (plain CSS grid, `repeat(auto-fill, minmax(260px,
1fr))`, row-major — order-preserving since pinned notes are typically few
and equal-height stretching there is a non-issue), and everything else
renders below in the `.notes-grid` masonry section. The "+ Take a note…"
tile moved into the pinned row (it's always first regardless of pin state).
`margin-bottom`/`break-inside: avoid` on `.note-card` is now scoped to
`.notes-grid > .note-card` only, so it doesn't double up with the pinned
row's `gap`. This also happens to match Google Keep's own pinned/others
split, so it's a reasonable permanent shape rather than a stopgap.

**Follow-up 2**: `.notes-pinned-row` being `display: grid` brought back the
original row-stretch bug, scoped to the pinned section — grid's default
`align-items: stretch` was pulling every card in a row up to the height of
its tallest row-mate. Fixed with `align-items: start` on `.notes-pinned-row`,
so cards keep their own height and sit at the top of their grid cell; row
placement and row-track height (driven by the tallest item) are unaffected.

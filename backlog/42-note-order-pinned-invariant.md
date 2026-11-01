# Note ordering — persisted manual order + pinned-group invariant

Today notes have no user-controlled manual order at all. The list query
(`get_notes` in `api/notes.rs:408`) sorts purely by `n.pinned DESC,
n.updated_at DESC` — pinning already places pinned notes above all
non-pinned ones unconditionally, but there is no way for a user to choose the
relative order of notes within either group. This item adds that ordering
and the invariant that governs it, as groundwork for
[43 - Drag-and-drop note reordering](43-note-drag-reorder.md).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — extends the `notes` table and
  `get_notes`/`toggle_note_pin` introduced there.

## Functional Requirements

- Add a persisted manual order to `notes` (e.g. a `sort_order` column —
  fractional/gap-based indexing recommended so reordering one note doesn't
  require rewriting every row) so a chosen order survives reloads and is
  shared across sessions/devices.
- **Invariant**: pinned notes always form a contiguous block above all
  non-pinned notes. Manual order only controls relative position *within*
  each group — a non-pinned note can never end up between two pinned notes,
  and a pinned note can never end up between two non-pinned notes. Enforce
  this server-side on write (not just client-side), since two collaborators
  on a shared collection could reorder concurrently.
- Toggling a note's pinned state (existing `toggle_note_pin`) moves it to the
  correct end of its new group (top of pinned, or top/bottom of non-pinned —
  pick one and document it) rather than leaving it at a now-invalid position.
- **"★ Pinned" toolbar toggle stays a declutter filter, unchanged in
  behavior.** Since pinning already sorts to the top regardless of the
  toggle, the toggle's only remaining job is hiding non-pinned notes when the
  user wants to focus on pinned ones — it does not control position. This was
  ambiguous in the original #37 spec ("filter toggle" vs. sort); resolving it
  here so it isn't re-litigated: no code change to the toggle itself, just
  this documented rationale.

## Open Questions

- Exact storage scheme for manual order (integer with periodic renumbering
  vs. fractional/lexicographic keys) — leave to implementation, but prefer
  whichever avoids a full-table rewrite on every single-note move.

## Resolution

Implemented as specified, with both open decisions resolved via user
sign-off before the schema change:

- **Storage**: `notes.sort_order REAL NOT NULL DEFAULT 0` (migration
  `20260714000000_note_sort_order.sql`), gap-based with a 1000-wide step.
  Backfilled from the prior implicit order (`pinned DESC, updated_at DESC`)
  using `UPDATE ... FROM` with a precomputed `ROW_NUMBER() OVER (PARTITION BY
  pinned ORDER BY updated_at DESC)` window — note this had to be a plain
  (non-correlated) subquery joined via `UPDATE...FROM`; a correlated
  subquery referencing the same table being updated silently produced 0 for
  every row in SQLite instead of the window function's rank.
- **Query**: `list_notes` now orders by `n.pinned DESC, n.sort_order ASC`
  instead of `updated_at DESC`. The invariant (pinned notes always a
  contiguous block above non-pinned) falls directly out of sorting on
  `pinned` first — `sort_order` only ever determines relative position
  *within* a group, so no group-boundary values can leak across, even under
  concurrent writes from different collaborators.
- **Placement on write**: new notes and notes whose `pinned` state changes
  (via the dedicated pin toggle *or* the edit-modal's pin checkbox — both
  paths were updated) are placed at the top of their group, via a shared
  `move_to_top_of_group` helper that computes `MIN(sort_order)` for the
  target group inside the same transaction as the write, so the new
  position is never based on stale client-supplied data.
- **Toolbar toggle**: left untouched, as specified — it remains a pure
  declutter filter.
- No `sort_order` field was added to `NoteInfo`/the client DTO — ordering is
  fully server-side via the `list_notes` query, and #43 (drag-and-drop) will
  add whatever client-facing reorder endpoint/fields it needs.
- One deviation from the open question's framing: the "group" that
  `sort_order` groups by is scoped to the whole `notes` table (`WHERE pinned
  = ?`, no collection/visibility filter), not per-viewer. This was a
  deliberate choice — `list_notes` already flattens all of a user's visible
  collections into one masonry grid, and scoping top-of-group placement to
  a single viewer's visible set would make placement inconsistent between
  two collaborators with different collection memberships. A global
  minimum keeps the invariant correct for every viewer.

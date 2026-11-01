# Note to-do completion write-back + audit log

When a note's mirrored VTODO is marked complete, reflect that back onto the
originating note in the Notes UI, mirroring the existing write-back pattern
already used for markdown-sourced todos.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — this extends its mirroring/completion
  handling.

## Context / Background

`complete_todo_item` (`api/events.rs`) already dispatches on the `events`
table's `source` column and, for `source == "markdown"`, writes the
completion back to the originating markdown file. Extending this with a
`source == "note"` branch to flip some "done" state visible in the Notes UI
is architecturally cheap and consistent with that existing pattern.

Note: `complete_todo_item` is only reachable from the tidl web UI's own
Todos/Focus checkbox (it's a `#[server]` fn scoped to the logged-in user's
own rows). While investigating this, we confirmed calendar sharing (#13) does
**not** currently expose any calendar to raw CalDAV clients for a different
user's Basic Auth identity — see
[41 - Expose calendar shares over raw CalDAV](41-caldav-share-exposure.md) —
so a share recipient completing/deleting a note's mirrored to-do via their
own external CalDAV client isn't a live scenario today. This item is purely
about reflecting completion state back into the Notes UI for the note's
owner (and, once #41 exists, for anyone with a read-write calendar share).

An audit trail (who completed it, and when) was suggested as a nice-to-have
alongside this, but needs a new column/table and richer UI, so it's captured
here as a stretch goal rather than baseline scope.

## Functional Requirements

- Extend `complete_todo_item`'s dispatch to handle `source == "note"`:
  update the originating `notes` row so the Notes UI reflects the completed
  state (exact representation — a boolean, or clearing `is_todo` — left to
  implementation).
- Stretch goal: record who completed/reopened the mirrored to-do and when,
  and surface it somewhere in the Notes UI (e.g. on hover or in the note
  editor). Needs a new column (or small side table) — not required for the
  baseline write-back to ship.

## Open Questions

- Should completing the mirrored to-do also unpin/archive the note, or just
  mark it done while leaving it visible in the grid? Needs a product
  decision, not just an engineering one.
  **Resolved**: mark done, leave the note visible as a plain note (see
  Resolution below).

## Resolution

- `complete_todo_item` (`api/events.rs`) now branches on `source == "note"`
  in addition to the existing `"markdown"` branch. When such a to-do is
  completed, it clears the originating note's `is_todo`/`tickler_date`/
  `mirrored_event_uid` columns (matching what `update_note` does when a
  note is manually un-flagged) and deletes the mirror's `events` row
  immediately.
- Immediate deletion (rather than leaving the row with `status =
  COMPLETED` for a later sync pass, as markdown does) is necessary because
  nothing else ever re-syncs a note's mirror away — markdown todos get
  cleaned up on the next file sync, but notes have no equivalent background
  job.
- Looked up the note by `mirrored_event_uid = uid` only (not scoped to the
  resolved event owner) since a shared note collection's mirror can be
  last written by any member with access, so `events.username` doesn't
  reliably match `notes.author_username`.
- No client-side changes were needed: `Todos`/`Focus`/`Tickler` already
  refetch their resource after calling `complete_todo_item`, so the item
  disappearing from those views falls out for free.
- Stretch goal (audit trail: who completed/reopened, and when) **not**
  implemented — still needs a new column/table and UI, left for a future
  wish if wanted.

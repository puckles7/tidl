# Go-to-source for a note task should flash the task pill, not the note card

Focus's "↗ Go to source" for a note-origin task opens `NoteGotoModal`, which
navigates to the note's collection and pulses the whole **note card**. The
markdown equivalent instead flashes the exact rendered **pill** (backlog #74).
Make the note case match: land on the specific task/await chip inside the note,
not the card.

**Status:** done.

## Dependencies

- [62 - Note items need a "go to source" arrow](62-note-goto-source-arrow.md) —
  the arrow this refines.
- [125 - Central done-item tracking](125-central-done-tracking.md) — the Done
  screen reuses the same note go-to path.

## Context

- A note token already carries `data-note-token` (`note:{id}:{line}:{ordinal}`),
  so the exact chip is addressable.
- Affects Focus and the Done screen, which share this path.

## Functional Requirements

- Thread line/ordinal into the note target, then scroll to and flash the
  matching `note-token` chip on the Notes tab.
- Fall back to the existing "open the note" behaviour when the chip can no
  longer be located.

## Resolution

Threaded the token location through the note go-to-source path. The
`note_target` signal (Focus + Done) now carries a `NoteTarget { note_id, token:
Option<(u32, u32)> }` instead of a bare `i64`; the `(line, ordinal)` half is
parsed from the item's `note:{id}:{line}:{ordinal}` uid by a new
`note_token_locator` helper (unit-tested). `NoteGotoModal`'s "Show in
collection" now appends `&line=&ordinal=` to the `/notes?highlight=` deep link,
and `NotesView` reads those params: it scrolls to and pulses the exact
`[data-note-token=…]` chip (own urgency colour via the new
`.note-token--highlight` CSS, cleaned up on `animationend`), falling back to the
whole-card `note-card--highlight` (backlog #63) when the token can no longer be
located (edited away) or the location was never encoded. Covers both Focus and
the Done screen, which share the path.

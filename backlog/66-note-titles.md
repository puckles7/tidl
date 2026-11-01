# Note titles

Notes have no title today — the Notes-tab card shows the raw note text in
full, and the Tasks/Focus/Await views label a note-backed task/await card
with an auto-truncated first-120-chars summary derived from the note body.
Add an optional title to notes: shown bold above the content on the
Notes-tab card, and used verbatim as the card label in Tasks/Focus/Await
(via the mirrored VTODO's SUMMARY) when set.

## Context / Background

- See design spec: `docs/superpowers/specs/2026-07-24-note-titles-design.md`.
- `notes` table (migration `20260710000000_notes.sql`) has no title column.
- `sync_mirror` (`crates/tidl-web/src/api/notes.rs`) derives the mirrored
  VTODO's `SUMMARY` via `mirror_summary(&input.text)` — a 120-char
  truncate-and-collapse of the note body.
- `NoteCard` (`components/notes/note_card.rs`) renders `note.text` in full
  in `.note-card-text`; no title row today.

## Functional Requirements

- New nullable migration: `ALTER TABLE notes ADD COLUMN title TEXT`.
- `NoteInfo`/`NewNote` (`types.rs`) gain `title: Option<String>`.
  `list_notes`/`create_note`/`update_note` read/write it.
- `sync_mirror`'s `SUMMARY` uses the trimmed title verbatim when non-empty,
  else falls back to today's `mirror_summary(&input.text)`.
- `NoteEditorModal` gets an optional single-line "Title" text input above
  the content textarea.
- `NoteCard` renders `note.title` bold above `.note-card-text` when set;
  unchanged when absent.
- No title requirement, no character-limit enforcement, no backfill of
  existing notes, no change to plain (untracked) notes' Tasks/Focus/Await
  visibility.

## Open Questions

- None — confirmed during brainstorming (title optional, single-line input
  above textarea, title used verbatim for the mirrored summary).

## Resolution

Built per spec, no deviations:

- Migration `20260724000000_note_title.sql` adds nullable `notes.title`.
- `NoteInfo`/`NewNote` gained `title: Option<String>`; `list_notes` selects
  it, `create_note`/`update_note` bind a trimmed, empty-filtered value.
- `sync_mirror` (`api/notes.rs`) now derives the VTODO `SUMMARY` from the
  trimmed title when non-empty, falling back to the existing
  `mirror_summary(&input.text)` otherwise. Covered by three new
  `sqlx::test`s: title verbatim, untitled fallback, and blank-title-treated-
  as-untitled.
- `NoteEditorModal` gained an optional single-line "Title" `<input>` above
  the content textarea, wired into `build_input`.
- `NoteCard` renders `note.title` bold (new `.note-card-title` CSS rule)
  above `.note-card-text` when set; unchanged when absent.
- README's Notes tab bullet updated to describe title-driven `SUMMARY` vs.
  the always-full-text `DESCRIPTION`.

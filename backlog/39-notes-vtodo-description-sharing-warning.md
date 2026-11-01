# Mirrored VTODO description + note/calendar sharing-mismatch warning

Two small follow-ups to [37 - Notes tab](37-notes-tab.md), surfaced while reviewing
how a note's mirrored to-do interacts with the pre-existing calendar-sharing
feature ([13 - Calendar sharing](13-calendar-sharing.md)).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — this modifies the mirroring logic and
  editor component it introduced.

## Context / Background

Today `sync_mirror` (in `api/notes.rs`) only sets the mirrored VTODO's
`SUMMARY` (a truncated, collapsed copy of the note text via `mirror_summary`).
Anyone who can see the calendar the to-do lands in — including someone the
calendar is shared with, per #13, who has no relationship to the note's own
collection — sees only that truncated summary with no way to get the rest of
the note's content unless they also have Notes-UI access to its collection.

Separately, the "Calendar" chip row in `NoteEditorModal` (shown once "Make
this a to-do" is checked) looks like a plain categorization choice, but it
actually determines which CalDAV calendar — and therefore which audience,
via #13 calendar sharing — the mirrored to-do becomes visible to. That
coupling isn't visible in the UI today.

## Functional Requirements

- Include the full note text in the mirrored VTODO's `DESCRIPTION` property
  (RFC 5545 line-folded/escaped), not just the truncated `SUMMARY`. No schema
  change needed.
- Add a short caption under the "Calendar" chip row in `NoteEditorModal`
  clarifying that the selected calendar determines CalDAV sync/visibility of
  the resulting to-do.
- Soft warning (not enforcement): when the note's collection is shared with a
  different set of users than the selected calendar's shares, show a small
  inline notice ("this calendar is shared with people outside this note's
  collection" or similar). Do not block saving — there are legitimate cases
  where the audiences differ on purpose (e.g. a private note collection whose
  resulting to-dos are deliberately mirrored into an already-shared
  family/work calendar).

## Open Questions

- Exact wording/placement of the mismatch warning — leave to implementation,
  should match the visual language of existing dialog hints
  (`.dialog-hint`/`.form-check-label` conventions already used in
  `NoteEditorModal`/`CollectionEditorModal`).

## Resolution

- Added `escape_ical_text` and `fold_ical_line` to
  `tidl-caldav::icalendar_utils` (RFC 5545 §3.3.11 TEXT escaping and §3.1
  75-octet line folding, unit-tested including a fold/unfold round trip and
  a multi-byte char-boundary case). `sync_mirror` (`api/notes.rs`) now emits
  a `DESCRIPTION` property built from the full, escaped/folded note text,
  alongside the existing truncated `SUMMARY`.
- `NoteEditorModal`'s Calendar chip row gained a `.dialog-hint` caption
  explaining that the selected calendar drives CalDAV visibility.
- Added a soft sharing-mismatch warning below the chip row: a
  `LocalResource` fetches `get_calendar_shares` for the selected calendar
  (refetches on chip click) and compares its grantee set against the note's
  collection members (from the `collections` prop already in memory). A
  mismatch renders a `.note-sharing-warning` hint; save is never blocked.
  Wrapped in a local `<Transition fallback=|| ()>` (per `/leptos-suspense`)
  so the per-click refetch doesn't blank the modal or bubble into the root
  boundary.
- Deviation from spec wording: the warning text is direction-neutral
  ("isn't shared with the same people... may see one but not the other")
  rather than the one-directional example in the spec, since the mismatch
  can go either way (calendar more shared than the collection, or vice
  versa).
- Added an e2e assertion in `tests/test_notes.py` checking the mirrored
  VTODO's `ical_data` contains the full-text `DESCRIPTION`.

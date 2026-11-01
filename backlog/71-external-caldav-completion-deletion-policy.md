# External CalDAV completion/deletion policy for note-linked tasks

Decide and implement what should happen when an external CalDAV client (DAVx5,
Thunderbird, Apple Calendar/Reminders, …) — not the tidl web UI — completes or
deletes a VTODO, for both plain tasks and note-mirrored ones. Surfaced from
buglist item 1 ("How to handle a task delete or completion from a different
caldav client for both normal tasks, and tasks originating from a note").

## Context / Background

Investigated 2026-07-30 alongside buglist item 4 (backlog work, same
session). Current actual behaviour, traced through the code:

- **External completion is not wired up at all.** `tidl_caldav::extract_event_fields`
  never parses a VTODO's `STATUS` property, and `Storage::store_event` (the
  function backing `caldav_event_put`) never writes the `events.status`
  column. So today, an external client setting `STATUS:COMPLETED` on a PUT
  has *zero effect* — the task keeps showing as incomplete in Focus/Tasks/
  Await regardless of source (plain or note-derived). This is true for both
  cases named in the bug, not just note-linked ones.
- **External deletion of a note-linked task doesn't sever the note's link.**
  `caldav_event_delete_or_subcollection` just deletes the `events` row. The
  owning note's `mirrored_event_uid` is left pointing at a uid that no
  longer exists, and its `kind` (todo/await flag) is untouched. The task
  immediately disappears from Focus/Tasks/Await (the query joins from
  `events`, so a missing row means no result), but the Notes view still
  shows the note flagged as a to-do/await with a dead mirror. The dangling
  mirror self-heals the *next* time the user edits/saves that note in the
  app — `sync_mirror` (`api/notes.rs`) unconditionally deletes-then-recreates
  the mirror on every save — but until then the note looks like a live
  to-do that quietly vanished from the task views.
- Buglist item 4 (see 2026-07-30 fix, `Storage::store_event`) already fixed
  a related but distinct bug: external edits used to reset `source`/`kind`/
  `status` to NULL/default via `INSERT OR REPLACE`'s delete+insert semantics.
  That fix (switched to `ON CONFLICT ... DO UPDATE`) means *completion state
  is now preserved* across an external edit — it just still can't be *set*
  by one.

## Decisions (resolved 2026-07-30)

- **External `STATUS:COMPLETED` is honored** — it's standard CalDAV client
  behavior and silently ignoring it is surprising.
- **Note-linked tasks: completion mirrors the app's own checkbox exactly**
  (backlog #40) — clear the note's `kind`/`due_date`/`mirrored_event_uid`
  and delete the mirrored VTODO, regardless of whether the completion came
  from the web UI or an external client. One consistent completion model.
- **`STATUS:NEEDS-ACTION` is honored symmetrically** for plain tasks (flips
  `completed` back to `false`). Moot for note-linked tasks — completing one
  already deletes the mirror row, so there's nothing left to un-complete.
- **External DELETE of a note-linked task's VTODO also clears the note's
  link immediately** (not just on completion) — same `kind`/`due_date`/
  `mirrored_event_uid` clear as completion, no event-row work needed since
  the row's already gone. The note itself is never deleted, only reverted
  to a plain (non-todo) note.
- **Recurring VTODOs are a non-issue**: `Task` has no `rrule` field at all —
  recurrence only exists for `Event`/VEVENT in this app's model, so there's
  no recurring-VTODO case to scope around.

## Functional Requirements

- `tidl_caldav::EventFields` gains a `status: Option<String>` field, parsed
  from the VTODO's `STATUS` property (raw RFC 5545 value — `NEEDS-ACTION`,
  `COMPLETED`, `IN-PROCESS`, `CANCELLED`, or absent).
- `Storage` gains two methods:
  - `set_event_completed(username, calendar, uid, completed: bool)` — sets
    `status`/`completed_date` directly.
  - `complete_note_mirror_if_any(uid)` — clears the owning note's `kind`/
    `due_date`/`mirrored_event_uid` if `uid` is a mirror; returns whether it
    matched. A no-op (`false`) for a plain task.
- `caldav_event_put`, after the existing priority reverse-sync and only for
  `component_type == "VTODO"`: if the body carries a `STATUS`,
  - `COMPLETED` → try `complete_note_mirror_if_any`; if it matched, also
    delete the just-stored event row. If it didn't match, `set_event_completed(true)`.
  - anything else present (`NEEDS-ACTION`, `IN-PROCESS`, `CANCELLED`) →
    `set_event_completed(false)`.
  - `STATUS` absent from the body → untouched (same as today).
- `caldav_event_delete_or_subcollection`, after deleting the event row:
  unconditionally call `complete_note_mirror_if_any` too.

## Resolution

Implemented 2026-07-30 exactly as specified above:

- `tidl_caldav::EventFields` gained `status: Option<String>`, parsed
  (case-normalised) from a VTODO's `STATUS` property. 3 unit tests.
- `Storage` gained `set_event_completed` and `complete_note_mirror_if_any`,
  both on the `CalendarStore` trait with the usual inherent-method +
  delegating-trait-impl pair. 4 unit tests (`storage.rs`).
- `caldav_event_put`'s completion handling was extracted into a standalone
  `sync_vtodo_completion(storage, username, calendar, uid, fields)`
  function (mirroring how the existing priority reverse-sync already
  worked) rather than left inline, specifically so it could be unit-tested
  directly against a real `Storage`/in-memory SQLite without needing to
  construct the Axum `BasicAuth`/`AppState` extractors — no existing test
  in `caldav_handlers.rs` does that, all operate at this same level. 5 unit
  tests cover: VEVENT ignored, missing `STATUS` leaves the column alone,
  plain-task complete/un-complete, and note-mirror completion severing the
  link and deleting the row.
- `caldav_event_delete_or_subcollection` now also calls
  `complete_note_mirror_if_any` after deleting the event row.

No deviations from the plan. `cargo test --workspace` (333 tidl-web + 101
tidl-server + 67 tidl-caldav + 22 tidl-markdown, all passing), clippy, and
the wasm32 build for `tidl-web` all clean.

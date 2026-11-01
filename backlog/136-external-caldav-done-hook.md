# Index external CalDAV completions/deletions immediately

The `done_items` index reconciles on every Done-view load, so a task completed
or deleted by an external CalDAV client (DAVx⁵, Thunderbird, …) only appears (or
archives) the next time the Done screen opens. There is no immediate hook, so an
external *delete* of an item that was never observed as completed is missed.

**Status:** done.

## Resolution (2026-10-09)

External CalDAV completions and deletions now hit the `done_items` index
immediately instead of waiting for the next Done-view reconcile:

- `tidl_web::ssr_types::sync_done_item_from_caldav` dispatches to
  `api::done::upsert_caldav_done_item` (completed) / `archive_caldav_done_item`
  (un-completed or deleted) — the same cross-crate pattern as
  `sync_note_priority_from_caldav`. The upsert reads the event row and builds
  the exact live row `reconcile_caldav` would (shared `caldav_scanned` mapping).
- `CalendarStore::sync_done_item_from_caldav` (inherent + delegating trait impl)
  wraps the helper so the CalDAV handlers can reach the pool.
- `sync_vtodo_completion` upserts on `COMPLETED` (plain task) and archives on
  any other `STATUS`; `caldav_event_delete_or_subcollection` archives after
  deleting the event. Note-mirror completion deletes the mirror, so no CalDAV
  row is indexed (matching reconcile's end state).

Deviation: an external *un-completion* archives the live row (keeps history)
rather than deleting it — the reconcile-consistent behaviour, though the app's
own `complete_todo_item(uid, false)` still uses `forget_done_item`. A plain-task
completion is indexed even if the task is deleted before the next Done load
(the "never-observed-completed delete" case).

## Dependencies

- [124 - Clean-up for completed tasks](124-done-task-cleanup.md) — the index and
  its reconciliation.
- [71 - External CalDAV completion/deletion policy for note-linked tasks](71-external-caldav-completion-deletion-policy.md)
  — the same external entry points.
- [125 - Central done-item tracking](125-central-done-tracking.md).

## Context

- `sync_vtodo_completion` / `caldav_event_delete_or_subcollection`
  (`crates/tidl-server/src/caldav_handlers.rs`) are the external entry points.
- Reconcile-on-load already reaches the correct end state; this is purely about
  immediacy and the never-observed-completed delete case.

## Functional Requirements

- Call a `tidl_web::ssr_types` helper from those handlers to upsert/archive the
  matching `done_items` row — the same cross-crate pattern as
  `sync_note_priority_from_caldav`.

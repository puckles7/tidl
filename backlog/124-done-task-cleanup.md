# Clean-up for completed tasks

Completed markdown tasks are rewritten to `#done/<slug>` and stay in the source
file forever; completed ad-hoc/note tasks and awaits likewise accumulate. There
is no way to actually remove done items once they are finished, so sources grow
unbounded and previews keep rendering old done badges. This adds a clean-up
action for completed items.

**Status:** done. Design:
[spec](../docs/superpowers/specs/2026-10-08-done-item-lifecycle-design.md) ·
[plan](../docs/superpowers/plans/2026-10-08-done-item-lifecycle.md).

## Resolution (2026-10-08)

Shipped the removal engine and clean-up UI described in the design/plan:
migration `20261008000000_done_items.sql`, `tidl_markdown` done-tag strip
helpers, `note_tokens` removal/reopen helpers, and `api/done.rs` (reconcile
engine, `preview_done_purge`/`purge_done_items` archive-not-delete, reopen,
per-scope + archive retention, lazy archive pruning). The `/done` screen drives
bulk "Clean up all matching" with a retention/archive **Clean up…** dialog.
Merged to `main` as `feat/central-done-tracking` (3638c5d).

Deviations: the remaining polish work was split into its own items — per-item
removal ([#133](133-done-remove-single-item.md)), note go-to flashing the task
pill ([#134](134-note-goto-flash-pill.md)), auto-remove-on-complete
([#135](135-auto-remove-completed-per-source.md)), and an explicit
external-CalDAV completion/deletion hook
([#136](136-external-caldav-done-hook.md)).

## Progress (2026-10-08)

Implemented on `feat/central-done-tracking`:

- Migration `20261008000000_done_items.sql` (`done_items`, `done_scopes`,
  `done_settings`).
- `tidl_markdown`: `scan_done_tags`, `remove_tag_at`, `remove_done_tags`
  (strip-only, drop emptied lines, LF/CRLF preserved).
- `note_tokens`: `remove_token_at`, `remove_tokens`, `reopen_token_at`.
- `api/done.rs`: reconcile engine (upsert live rows, preserve tombstones,
  generation keys), `preview_done_purge` / `purge_done_items`
  (archive-not-delete), `reopen_done_item`, per-scope + archive settings,
  `clear_archived_done_items`, lazy archive pruning.
- `/done` screen: search, origin filter, Show-removed, bulk "Clean up all
  matching", per-item Re-open, Clean up… dialog (retention + archive).
- Reopen paths (`reopen_markdown_task`, `reopen_note_token`,
  `complete_todo_item` reopen) forget their index row.

Remaining, split into their own items: per-item removal
([#133](133-done-remove-single-item.md)), note go-to flashing the task pill
([#134](134-note-goto-flash-pill.md)), auto-remove-on-complete
([#135](135-auto-remove-completed-per-source.md)), and an explicit
external-CalDAV completion/deletion hook
([#136](136-external-caldav-done-hook.md)) — reconcile-on-load currently covers
the last one's outcome.

## Dependencies

- [112 - Rendered markdown task checkboxes write `#done` back](112-rendered-task-checkbox.md) — the `#done/<slug>` rewrite this cleans up.
- [117 - Markdown checked-off tasks keep their data and can be re-opened](117-markdown-done-roundtrip.md) — done items are deliberately retained today.
- [40 - Note to-do completion write-back + audit log](40-note-todo-completion-writeback.md) — completion/audit behaviour for note-linked items.

## Functional Requirements

- A clean-up action that removes completed items from their origin:
  - markdown: delete the `#done/<slug>` line(s) from the source file(s);
  - note-linked / ad-hoc: remove the completed task/await (and its mirror).
- Scope and preview: per source / per collection / per list view, with a count
  of what will be removed and a confirmation step.
- Preserve file line-ending style (LF/CRLF); push git-backed sources and trigger
  a re-sync, mirroring #112/#117's writers.
- Optional retention rule (e.g. keep the most recent N, or only remove older
  than a date) rather than all-or-nothing.

## Open Questions

- Where does the UI live — Settings → Sources, a per-file action in the preview,
  or a button in Focus/Tasks/Tickler?
- Per-item vs. bulk; is there an undo?
- Should clean-up be automatic (a setting) or always manual?
- Relationship to [125 - Central done-item tracking](125-central-done-tracking.md)
  — same removal engine, different entry point?

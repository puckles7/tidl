# Central done-item tracking with optional source removal

Completed items are scattered: markdown `#done/<slug>` lines live in files, done
note-linked/CalDAV tasks live in the calendar store, and none of them surfaces
in a list view once complete. There is no central archive to see what was done
across all sources, and no way to remove an item from its source from one place.

**Status:** done. Design:
[spec](../docs/superpowers/specs/2026-10-08-done-item-lifecycle-design.md) ·
[plan](../docs/superpowers/plans/2026-10-08-done-item-lifecycle.md).

## Resolution (2026-10-08)

Shipped the `/done` screen (navbar tab after Contacts): completed items across
markdown, note, and CalDAV origins with search, origin filter, "Show removed"
(archived history with removed badge), per-item **Re-open**, and bulk **Clean up
all matching**, backed by `api/done.rs` (`list_done_items`, `list_done_scopes`,
`get/set_done_settings`, `set_done_scope`). Merged to `main` as
`feat/central-done-tracking` (3638c5d).

Deviations: per-item removal ([#133](133-done-remove-single-item.md)), note
go-to flashing the task pill ([#134](134-note-goto-flash-pill.md)), and the
navbar live count badge ([#137](137-done-tab-count-badge.md)) were split into
their own items. The go-to-source arrow itself ships with the same behaviour as
Focus.

## Progress (2026-10-08)

The `/done` screen (navbar tab after Contacts) lists completed items across all
three origins, with search, an origin filter, a "Show removed" toggle (archived
history rendered with a removed badge), per-item **Re-open** on live rows, and a
bulk **Clean up all matching**. A **Clean up…** dialog holds per-source
retention (keep-newest-N / older-than-days), the archive-retention window, and
**Clear archived items**. Backed by `api/done.rs` (`list_done_items`,
`list_done_scopes`, `get/set_done_settings`, `set_done_scope`).

Remaining, split into their own items: per-item removal
([#133](133-done-remove-single-item.md)), note go-to flashing the task pill
([#134](134-note-goto-flash-pill.md)), and a live count badge in the navbar
([#137](137-done-tab-count-badge.md)). The go-to-source arrow itself now ships
(same behaviour as Focus).

## Dependencies

- [124 - Clean-up for completed tasks](124-done-task-cleanup.md) — the removal engine this view drives.
- [112 - Rendered markdown task checkboxes write `#done` back](112-rendered-task-checkbox.md) — the markdown done representation.
- [117 - Markdown checked-off tasks keep their data and can be re-opened](117-markdown-done-roundtrip.md) — done items can be reopened.
- [40 - Note to-do completion write-back + audit log](40-note-todo-completion-writeback.md) — completion metadata for note-linked items.
- [71 - External CalDAV completion/deletion policy for note-linked tasks](71-external-caldav-completion-deletion-policy.md) — deletion policy already decided for note-linked items.

## Functional Requirements

- A central "Done" view listing completed items across calendars, markdown
  sources, and notes, each with its kind, source (file/collection/calendar), and
  when it was completed.
- Filter by source and by time window; search by title.
- Per-item and bulk "remove from source" — markdown line removal, CalDAV VTODO
  deletion, note token/note removal — with a confirmation that names the source.
- Optional per-source "remove from source automatically when completed" setting.
- Reuse of the existing completion/audit trail where present.

## Open Questions

- A full route/tab, or a panel inside Settings?
- Is this a single view over all sources or per-source sections?
- Retention/undo: soft-delete vs. hard removal; can a removed item be restored?
- How much of #124's clean-up UI is just this view filtered by source?

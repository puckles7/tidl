# Manage ad-hoc tasks and awaits (edit/delete, split +Task/+Await)

Ad-hoc items created from Focus ("+ Task") have no in-app source to go back to:
no markdown file and no note (they are plain VTODOs in the selected calendar,
which may itself be shared or wired to a remote CalDAV source). That made them
undeletable once the standalone Tasks view (which owned the inline delete) was
folded into Focus (#115), and the single create dialog blurred the task/await
line by always writing `kind='task'` even when a due date was supplied.

This gives these items a "↗" arrow that opens the same dialog used to create
them — now in edit mode, with Save and a one-click Delete — and splits creation
into `+ Task` (1-100 priority) and `+ Await` (date), mirroring the Notes
editor's Task/Await chips. The edit dialog keeps the same calendar picker as
creation, so an item can also be moved between calendars.

## Dependencies

- [115 - Merge Tasks and Await into Focus with an All/Calendar/Tasks/Await filter](115-focus-type-filter-pill.md) — removed the old inline task-delete UI
- [49 - TASK/AWAIT kind unification](49-task-await-unification.md) — the `events.kind` discriminator this reuses

## Functional Requirements

- `+ Task` creates a `kind='task'` VTODO with a priority (no due date);
  `+ Await` creates a `kind='await'` VTODO with a date (no priority).
- Every CalDAV-origin item (no markdown/note backing) shows the "↗" arrow.
- The arrow opens the create dialog in edit mode: prefilled title, the
  kind-appropriate field, the same calendar picker as creation, a Save button,
  and a one-click Delete button.
- Wording makes clear these are plain VTODOs created in tidl (not derived from
  a markdown file or a note), while not claiming they lack calendar backing —
  they live in the selected calendar and may sync out.
- `update_todo_item` rewrites the stored iCal so CalDAV clients see edits; the
  in-place property patcher preserves unknown/other properties and folded
  lines.
- Editing a task leaves any legacy due date alone (and vice versa), so an old
  task that carried a due date doesn't silently lose it.

## Resolution

Built as specified.

- `api/events.rs`: `create_todo_item` gained a `kind` argument and now writes
  task vs. await correctly; new `update_todo_item` (restricted to `source` NULL
  or `'caldav'`) edits title, the kind-appropriate field, and the calendar
  (bumping/notifying both the old and new calendar when it moves), patching
  `ical_data` via `set_vtodo_prop`/`fold_ical_line`; `delete_todo_item`'s
  predicate changed to `COALESCE(source, 'caldav') != 'markdown'` so externally
  created tasks (source NULL) are deletable too. `AwaitItem` gained a
  `calendar` field so the edit dialog can prefill the picker.
- `components/focus.rs`: `TodoItemModal` replaces `AddTodoModal` and handles
  Create(task/await) and Edit(item) with Save + one-click Delete and a shared
  calendar picker; header shows
  `+ Task` / `+ Await`; the card's arrow opens the edit dialog for ad-hoc items
  and keeps preview/note behaviour for sourced ones.
- Unit tests cover the iCal property patcher (replace, parameterised property,
  removal, folded-line removal, folding).

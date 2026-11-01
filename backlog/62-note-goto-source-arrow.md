# Note items need a "go to source" arrow in Focus, Tasks, and Await lists

Calendar events and markdown-origin tasks/awaits already have a "↗ Go to source" hover action in the Focus, Tasks, and Await lists. Note-origin items (tasks/awaits mirrored from a note, backlog #37/#38) did not: the arrow either silently no-op'd (Focus), was replaced by a delete button (Tasks), or was unconditionally disabled for everyone (Await — a stub left over from #20).

## Functional Requirements

- Note-origin tasks/awaits in Focus, Tasks, and Await get a working "↗" that opens the note's own editor.
- Note-origin tasks in the Tasks view lose the "Delete task" button (matching markdown items, which never show delete) rather than showing both — deleting a note-task's mirrored VTODO directly via the old delete button orphaned the note (dangling `mirrored_event_uid`) instead of going through the Notes UI's proper delete path.
- As a side effect of wiring this up in the Await view, markdown-origin await items also get a working "go to source" (finishing the stub noted in #20).

## Resolution

- `types.rs`: added `note_id: Option<i64>` to `Task` and `AwaitItem`.
- `api/events.rs`: `get_tasks`/`get_awaits` now select `n.id AS note_id` from the existing `notes` join.
- `components/focus.rs`: added a `NoteGotoModal` component (exported for reuse) that fetches `list_notes()` + `list_note_collections()` and renders the existing `NoteEditorModal` directly — reused as-is rather than building a separate read-only preview, since notes (unlike markdown files) are natively editable in-app. Shows "Note not found" if the note was deleted concurrently. `on_goto` now routes note-origin items to a `note_target` signal instead of falling through to the markdown branch.
- `components/todo_list.rs`: `FocusCard`'s hover-actions condition changed from `if is_markdown` to `if is_markdown || is_note`; note tasks no longer show the delete button.
- `components/tickler.rs`: added the `preview`/`note_target` signals and `PreviewOverlay`/`NoteGotoModal` rendering this view never had; replaced the permanently-`disabled=true` button with a working one for both markdown and note-origin items.
- `backlog/20-tickler-view.md`: updated its "Deviation" note now that the stub is resolved.

Verified with a real headless-browser run (Playwright against a freshly built, isolated `tidl-server`): note-origin task/await items on all three views open `NoteEditorModal` with the correct note content; plain caldav tasks still show their delete button; markdown tasks still show only the arrow — no regressions.

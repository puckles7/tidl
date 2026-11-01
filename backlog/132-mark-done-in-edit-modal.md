# "Mark as done" in the edit task modal

Add a **Mark as done** action to the edit-task/await modal (the `TodoItemModal`
opened from a Focus card's ↗ arrow), so an item can be completed without first
closing the dialog and using the card's checkbox.

**Status:** done.

## Resolution (2026-10-09)

Added a **Mark as done** button to the shared `TodoItemModal`, left of **Delete**
(it only appears when editing an existing, not-yet-done item — hidden on the
create form and for done items). It reuses `complete_todo_item`, refreshes the
list, and closes the dialog, exactly like the card checkbox.

Source-backed chips work too: `TodoOrigin::event_uid()` (new, unit-tested)
derives the canonical `events.uid` for a note token
(`note:{id}:{line}:{ord}`) or a markdown tag
(`md:{source_id}:{rel_path}:{line}:{ord}`), so completing a chip rewrites its
token / source line through the same server path — no special-casing. A
`#done` chip never loads as `TodoChip`, and ad-hoc done items carry
`AdHocEdit::completed`, so the button is hidden once an item is done.

Deviations: Open Question 1 (show **Reopen** instead on a done item) was not
implemented — done items simply hide the action; their existing reopen control
(the card checkbox / chip checkbox) is unchanged. Placement went with
Open Question 2's "alongside Delete on the left".

## Context

`TodoItemModal` (`crates/tidl-web/src/components/focus.rs`) currently shows, for
an existing ad-hoc item, a **Delete** button and **Save**. Completion is only
reachable from the card's checkbox (`focus.rs:1412`, calling
`complete_todo_item`). Completion already has server support
(`complete_todo_item`, `crates/tidl-web/src/api/events.rs:1050`) and is
reversible for note/markdown-backed items, so this is mostly a UI affordance.

## Functional Requirements

- A **Mark as done** button in the edit modal for an existing task/await.
- Pressing it completes the item (reusing `complete_todo_item`) and closes the
  modal, with the list refreshing like the card checkbox does.
- On a source-backed chip (note / markdown origin), completing rewrites the
  token / source line as today — no special-casing.
- Only show it when editing an existing item (not on the create form), and hide
  or disable it when the item is already done.

## Open Questions

- Should an already-done item instead show **Reopen** in the modal, matching the
  round-trip behaviour elsewhere (#117)?
- Placement: alongside Delete on the left, or as a distinct primary action?

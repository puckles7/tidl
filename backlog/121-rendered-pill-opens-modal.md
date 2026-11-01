# Clicking a rendered task/await pill opens its task/await modal

Today a rendered task or await chip only reacts to its checkbox (mark done /
reopen). Clicking the pill itself does nothing, so from a note card or a
rendered markdown preview there is no way to inspect or edit the item without
going back to a list view. Clicking the pill body should open the same
task/await modal used elsewhere, for both notes and markdown sources.

**Status:** done.

**Design:** [2026-10-08-rendered-pill-opens-modal-design.md](../../docs/superpowers/specs/2026-10-08-rendered-pill-opens-modal-design.md)
**Plan:** [2026-10-08-rendered-pill-opens-modal.md](../../docs/superpowers/plans/2026-10-08-rendered-pill-opens-modal.md)

## Dependencies

- [112 - Rendered markdown task checkboxes write `#done` back](112-rendered-task-checkbox.md) — the rendered badge + checkbox this adds a click target to.
- [116 - Task/await chip UI](116-task-await-chip-ui.md) — the `note-token` chip markup shared by notes and markdown.
- [118 - Manage ad-hoc tasks/awaits](118-adhoc-task-await-management.md) — the edit modal this should reuse.
- [113 - Inline tasks and awaits in notes](113-note-inline-tasks.md) — the note-side token model the click must resolve against.

## Background

Markdown render (`tidl_markdown::render_tag_line`) emits
`<span class="note-token note-token--task" data-md-line data-md-ordinal>` with an
`md-task-checkbox` input and the label; note cards render the same chip shape on
a `data-mention`/token delegation handler in `note_card.rs`. The preview's
delegated click handler already distinguishes the checkbox target; the task/
await edit modal is `TodoModalState::Edit(AdHocEdit)` in `focus.rs`, currently
only wired for ad-hoc (CalDAV) items.

## Resolution

Shipped 2026-10-08 (backlog #121). Decisions: **full editable** modal for
source-backed items, **done pills do nothing** on body click, **one shared
modal signal in context** at the app root.

- **Library helpers**: `tidl_markdown::edit_tag_at` (rewrites the ord-th active
  tag, preserving `#task/pN`, dated `#task/<date>`, or `#await/<date>` form) and
  `note_tokens::edit_token_at` (same for note bodies, preserving kind) — both
  unit-tested.
- **Server fns** (`api/events.rs`): `load_todo_chip(TodoOrigin)` returns the
  item's kind/title/priority/date for the modal and `None` for a `#done` record
  (so a done body click opens nothing); `update_todo_chip` writes the new title
  (slugified) + priority/date back — notes rewrite the token and re-sync in one
  transaction, markdown rewrites the line via `markdown_line_transform`.
- **Modal**: `TodoModalState::EditChip(TodoChip)` reuses `TodoItemModal`, now
  source-aware (priority slider vs date field, no calendar picker / Delete for
  source-backed items). `TodoModalCtx` is provided in `AuthenticatedApp` and the
  modal rendered at the app root, so every route shares it.
- **Click wiring**: Focus markdown preview, note cards (chip span now carries
  `data-note-token`), and the note editor's chip row all resolve the chip and
  open the shared modal; checkbox clicks stop propagation and never open it.

Deviations from the original sketch: deletion of source-backed chips is not
offered (ad-hoc only), and task↔await kind switching is not an edit — both noted
as non-goals in the design.

## Open Questions (resolved)

- Editable vs read-only → editable, writing back to origin.
- Done pill body → nothing (checkbox stays the reopen control).
- Modal placement → shared context signal at the app root.

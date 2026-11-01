# Mark a task done from the markdown editor's edit dialog

Clicking a `#task`/`#await` tag in the in-app markdown editor opens its edit
dialog, but the only way to complete the item is to leave the editor and use
the rendered preview's checkbox or Focus. Add a "Mark as done" checkbox to the
dialog so a task can be completed without abandoning the unsaved draft.

## Dependencies

- [129 - Edit (and create) task/await tokens in the note and markdown editors](129-editor-token-click-edit.md)
  — introduced the markdown editor's local task/await edit dialog.

## Functional Requirements

- The edit dialog (editing an existing tag, not creating one) shows a
  "Mark as done" checkbox.
- Saving with it checked rewrites the tag in the in-memory document to the
  same rich `#done` record the rest of the app writes, using today's local date
  as the completion date and preserving the original kind and parameters:
  - `#task/p<N>/<slug>` → `#done/<today>/p<N>/<slug>`
  - `#task/<date>/<slug>` → `#done/<today>/t<date>/<slug>`
  - `#await/<date>/<slug>` → `#done/<today>/<date>/<slug>`
- The token preview in the dialog reflects the `#done` form while checked.
- The file is still written only on the editor's own Save.

## Resolution

Added `markdown_tokens::format_done_token` (mirroring
`tidl_markdown::parser::mark_tag_done`) plus a "Mark as done" checkbox in the
markdown editor's edit dialog. Checking it and saving replaces the tag with the
rich `#done` record dated with the browser's local date; the dialog's token
preview updates live. New-tag dialogs do not show the checkbox, and the
checkbox resets whenever the dialog opens or closes.

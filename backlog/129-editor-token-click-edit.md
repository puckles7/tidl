# Clicking a task/await token in an editor opens (and creates) it there

While editing a note, typing `#task` (or `#await`) opens a caret popup that
launches the task/await authoring dialog. But a fully-specified tag that already
exists — `#task/p001/some-task`, `#await/2026-01-01/some-task` — still triggers
that same "create" popup whenever the caret is placed inside its `#task`/`#await`
keyword (`#`, `#t`, `#ta`, `#tas`, `#task`). Creating a task from there *removes*
the typed partial and appends a fresh token, silently invalidating the existing
one.

Clicking anywhere inside an existing tag should instead open that item in its
edit dialog — the same affordance the rendered pill already has (backlog #121).
And the editors should become a first-class place to author new items: the
markdown editor should offer the same `#` menu the note editor has.

**Status:** done.

**Design:** [2026-10-08-editor-token-click-edit-design.md](../../docs/superpowers/specs/2026-10-08-editor-token-click-edit-design.md)
**Plan:** [2026-10-08-editor-token-click-edit.md](../../docs/superpowers/plans/2026-10-08-editor-token-click-edit.md)

## Dependencies

- [113 - Inline tasks and awaits in notes](113-note-inline-tasks.md) — the `#`
  chooser popup and inline authoring dialog that misfire.
- [121 - Clicking a rendered task/await pill opens its modal](121-rendered-pill-opens-modal.md)
  — the rendered-pill edit affordance this mirrors in the editors.
- [91 - Markdown editor contact mention](91-markdown-editor-contact-mention.md)
  — the CodeMirror autocomplete plumbing the `#` menu extends.

## Background

- The note editor (`note_editor_modal.rs`) is a plain `<textarea>`. Its
  `update_hash_state` calls `util::find_active_hash`, which fires whenever the
  text after the last `#` is a prefix of `task`/`await`/`done`/`todo`/`pending`.
  Inside `#task/p1/foo` the caret after `#task` still satisfies that, so the
  chooser opens; `open_task_modal` then deletes `#task` (`state.start..end`) and
  `submit_task` appends a new token — the corruption reported here.
- The markdown editor (`editor.rs` + `assets/editor.js`) is CodeMirror 6. It has
  only an `@mention` autocomplete source and *no* task/await authoring at all.
- `#done` tags are completion records: like a done pill, they have no edit modal
  and must simply not trigger the chooser.

## Resolution

Shipped 2026-10-08. Both editors get a **local** edit dialog (the note editor
already had a local *create* dialog; the markdown editor gains one) rather than
the shared, server-writing `TodoItemModal`: an editor has unsaved text, so a
server-side write followed by the editor's own Save would clobber it. The local
dialog rewrites the editor's in-memory text and looks/behaves like the rendered
pill's dialog (same fields).

- **Detection**: a new wasm-safe `markdown_tokens` scanner mirrors
  `tidl_markdown`'s tag grammar (priority task, dated task, await, done) and
  resolves the tag under a caret; `note_tokens::token_at_caret` does the same for
  note bodies.
- **Note editor**: the `#` chooser is suppressed while the caret sits inside an
  existing tag; clicking an active one prefills and opens the local dialog,
  whose submit rewrites the token via `note_tokens::edit_token_at`. A `#done`
  token just suppresses the chooser.
- **Markdown editor**: CM6 grows a `#` completion menu (Task… / Await…), a
  mousedown handler that resolves the tag under the click, and `insert`/`replace`
  JS helpers. The Rust overlay renders the same local dialog; a new item is
  inserted at the caret, an existing tag rewritten in place (form preserved).

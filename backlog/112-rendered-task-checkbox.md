# Rendered markdown task checkboxes write `#done` back to the source

**Status: done.**

A task/await line in a rendered markdown file previews as a badge with a
checkbox; ticking it rewrites that line's tag in the source file to
`#done/<slug>`, so the item is checked off and archived in place instead of
silently vanishing from the file.

## Dependencies

- [18 - Markdown source files for todos and pending items](18-markdown-sources.md) — the `#task`/`#await` tag syntax and the parse/sync pipeline this extends.
- [23 - Open in editor — Markdown preview modal](23-open-in-editor.md) — the rendered preview whose badges gain the checkbox.

## Background

Tasks and awaits are tags in markdown source files (`#task/p0/slug`,
`#await/2025-01-15/slug`). The rendered preview (`tidl_markdown::render_to_html`)
turned a leading tag into a coloured badge, but offered no way to act on it, and
completing from the Tasks/Tickler/Focus list checkboxes **stripped** the tag
from the file entirely — so the record of what was done (and its slug/title)
was lost.

Decision (agreed during design): keep a record. Completing rewrites the tag to
`#done/<slug>` — the slug is preserved as the archived title, the
priority/due-date segments are dropped (a done item has neither) — rendered as
a checked, non-interactive badge. `#done` is deliberately not one of the
"active" patterns, so a done line drops out of `parse_tasks`/`parse_awaits`
(and therefore out of every list) while still rendering in a preview. Both the
new preview checkbox and the existing list-view checkboxes write `#done`, so
the two paths agree.

## Functional Requirements

- Render a checkbox on every active `#task`/`#todo`/`#await`/`#pending` badge,
  and a checked `disabled` one on a `#done/<slug>` badge.
- Ticking a checkbox rewrites that exact source line's tag to `#done/<slug>`,
  atomically, pushes git sources, and triggers an immediate re-sync.
- The rewrite must not need a pre-existing `events` row (a previewed file can
  contain several tasks, and the clicked one need not be the uid's own line).
- Preserve the file's line-ending style (LF/CRLF), matching the old strip path.

## Open Questions

- Re-opening a completed item would need the dropped priority/date restored, so
  completing is one-way for now (the done checkbox is `disabled`).
- Should a `#done` line's badge be collapsible/hidden in previews? Left visible
  for now.

## Resolution

- **`tidl-markdown/src/parser.rs`**: added `DONE_RE`, an `ANY_TASK_TAG_RE`, and
  a public pure `mark_tag_done(line)` that rewrites the first active tag to
  `#done/<slug>`. `render_tag_line` now recognizes `#done/<slug>`; every badge
  emits an `<input type="checkbox" class="md-task-checkbox">` (checked +
  `disabled` for done), and `done_badge_html` renders a muted, struck-through
  "Done · <title>" badge. Done lines are not parsed as tasks/awaits.
- **`tidl-web/src/api/events.rs`**: replaced `strip_todo_tag` /
  `strip_tag_from_content` with `mark_done_at_line` and a shared
  `markdown_line_done` writer (atomic write + git push + `MarkdownSyncTrigger`,
  mirroring `save_markdown_file`). `complete_todo_item`'s markdown branch now
  calls it, so list-view completion writes `#done` too. New `#[server]`
  `complete_markdown_task(uid, line)` resolves the source purely from the uid
  and rewrites the given line — the preview checkbox's entry point.
- **`tidl-web/src/components/focus.rs`**: the `.fp-rendered` delegated click
  handler now detects a checkbox target, walks up to the badge's
  `data-md-line`, disables the box optimistically, and calls
  `complete_markdown_task`, then refetches both the rendered and source views.
- **`assets/style.css`**: `.md-task-checkbox` and `.note-badge--done`.
- Tests: 6 new `tidl-markdown` parser tests (checkbox presence, done badge,
  done-line exclusion from parsing, `mark_tag_done` forms) and 5 rewritten
  `mark_done_at_line` tests (LF/CRLF/newline-less, out-of-range, already-done).

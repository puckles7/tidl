# Markdown checked-off tasks keep their data and can be re-opened

Markdown's rendered checkbox currently writes a lossy `#done/<slug>` (priority
and due date dropped) and is one-way — the done checkbox is `disabled`. Notes
already do better (`#done/<completed>/p<N>/<slug>` /
`#done/<completed>/<date>/<slug>`, re-openable). Make markdown match.

**Status:** done. Full design:
[spec](../../docs/superpowers/specs/2026-10-02-markdown-done-roundtrip-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-02-markdown-done-roundtrip.md).

## Dependencies

- [112 - Rendered markdown task checkboxes](112-rendered-task-checkbox.md) —
  the `#done` write-back this extends.
- [113 - Inline tasks and awaits in notes](113-note-inline-tasks.md) — the
  `#done/...` round-trip grammar this mirrors.
- [18 - Markdown source files](18-markdown-sources.md) — the tag parser/sync.

## Requirements

- `#task/p<N>/<slug>` completed → `#done/<completed-date>/p<N>/<slug>`.
- `#await/<due>/<slug>` completed → `#done/<completed-date>/<due>/<slug>`.
- Dated task `#task/<due>/<slug>` completed → keep its date too (see spec).
- The rendered done chip's checkbox is **interactive**; un-checking restores
  the original tag exactly.
- Legacy `#done/<slug>` files stay renderable (but not re-openable).

## Resolution

- **`crates/tidl-markdown/src/parser.rs`**: `mark_tag_done(line, completed)`
  now writes `#done/<c>/p<N>/<slug>` (task), `#done/<c>/t<d>/<slug>` (dated
  task) or `#done/<c>/<d>/<slug>` (await) — first tag only, via one
  alternation regex. New `reopen_tag(line) -> Option<String>` reverses each
  form; `DoneOriginal` enum; `DONE_TASK_RE`/`DONE_DATED_RE`/`DONE_AWAIT_RE`/
  `DONE_LEGACY_RE`. Rich done chips render checked + interactive with
  `data-md-reopen`; legacy `#done/<slug>` stays checked + `disabled`.
- **`crates/tidl-web/src/api/events.rs`**: `markdown_line_done` generalised to
  `markdown_line_transform` (+ `transform_line_at`, LF/CRLF preserving);
  `complete_markdown_task` and `complete_todo_item`'s markdown branch pass the
  completion date; new `#[server] reopen_markdown_task(uid, line)`.
- **`crates/tidl-web/src/components/focus.rs`**: the preview checkbox handler
  reads `data-md-reopen` and calls `reopen_markdown_task` instead of
  `complete_markdown_task` for a done chip.
- Tests: parser done/reopen/first-only cases; `transform_line_at` done/reopen
  LF+CRLF; 593 workspace tests pass; WASM build compiles.
- README grammar + feature bullets updated.

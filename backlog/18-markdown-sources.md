# Markdown source files for todos and pending items

Allow users to connect local markdown files as sources for todos
(`#todo/p<1|2|3>/<topic>`) and pending/tickler items
(`#pending/<YYYY-MM-DD>/<topic>`). This is one of two todo sources in the
app — the other is CalDAV VTODO (#19). Both coexist; markdown is the
read-only-from-the-app authoring model, CalDAV VTODO is the ad-hoc
in-app creation model.

## Parsing contract

Source files contain structured list items:

```
- Finalize pricing proposal
- Update team OKRs doc
- Send contract to client
- Return borrowed projector
```

Regex:
- Todo:    `/^- #todo\/p([1-3])\/(\w+)\s+(.+)$/`    → `{ prio, topic, title }`
- Pending: `/^- #pending\/(\d{4}-\d{2}-\d{2})\/(\w+)\s+(.+)$/` → `{ date, topic, title }`

Every parsed item carries back the **source file path** and **line number**
so "Go to source" can deep-link to the file preview overlay.

**Parser gap:** the existing `tidl-markdown` `TodoItem` captures `subject`
(the topic slug) and `context` (the raw line) but not a separate `title`
field (the human-readable text after the tag). The parser needs a small fix
to extract the title from the remainder of the line.

## Backend

The infrastructure is already largely in place:
- `events` table has `source TEXT` (`'caldav'` | `'markdown'`) and
  `source_file TEXT` columns.
- `markdown_sources` config table and `sync_state` table exist.
- `tidl-markdown` crate has `MarkdownParser`, `MarkdownWatcher`, and
  `FilesystemSource`.

What remains:
- Wire the `MarkdownWatcher` into the server startup so parsed items are
  persisted to the `events` table as VTODOs with `source = 'markdown'`.
- CRUD endpoints for users to add/remove source file paths.
- `get_todos()` server function returns both `source = 'markdown'` and
  `source = 'caldav'` rows (they are already in the same table); callers
  use the `source` field to differentiate.

## Frontend

- Sources sidebar section lists connected `.md` files.
- Sources tab (or Settings) exposes add/remove for source file paths.
- `get_todos()` response shape gains `source`, `source_file`, `line`,
  and `topic` fields for both markdown and CalDAV items.

## Open questions

- File access model: server-side path (simplest, implemented) vs. user
  upload vs. remote URL. Start with server-side path.
- Watch vs. poll: the `MarkdownWatcher` already does hash-based change
  detection; wire it on a poll interval rather than inotify for simplicity.

## Resolution

Parser fix: updated `TODO_RE` / `PENDING_RE` in `tidl-markdown` to capture a
trailing text group; `TodoItem` and `PendingItem` gain a `title` field
(literal text after the tag, falling back to `slug_to_title(subject)`).

Migration `20240117000000_markdown_sync.sql` adds `source_line INTEGER` and
`topic TEXT` to the `events` table.

`markdown_sync.rs` in `tidl-server` is a new polling loop (60 s interval) that
loads enabled `markdown_sources` rows, walks the configured filesystem path with
SHA-256 hash-based change detection via `sync_state`, and upserts VTODOs into
`events` with `source = 'markdown'`, `calendar = '__markdown__'`, `source_file`,
`source_line`, and `topic`. Broadcasts the owning username on change for SSE
refetch. Spawned at server startup alongside the remote CalDAV loop.

Three new server functions in `api.rs`: `list_markdown_sources`,
`add_markdown_source`, `delete_markdown_source` (mirrors the remote CalDAV
sources CRUD). Config stored as `{"path": "…", "recursive": false}` JSON.

`get_todos()` now selects `source`, `source_file`, `source_line`, `topic`;
`Todo` struct gains those fields with `#[serde(default)]`.

UI: `SourcesView` replaced with a real add/remove form. `TodoItem` component
shows a `"md · filename:line"` badge for markdown-sourced todos.

**2026-07-27 addendum — trailing-title capture reversed:** the "capture
text after the tag as `title`" behavior above turned out to be the wrong
design. It caused markdown appearing after a tag (e.g. a link,
`#pending/2026-12-31/assess-implementation [Assess implementation](https://x.example)`)
to be absorbed into the tag's own rendered badge instead of rendering as
independent content. Reversed: `title` is now unconditionally
`slug_to_title(subject)`; the tag regexes no longer have a trailing-text
capture group at all. Any text following a tag on the same line is no
longer part of the tag/item in any way — it's ordinary markdown, rendered
immediately after the tag's badge by the normal CommonMark pass (so a
trailing link still renders and works, just decoupled from the badge).

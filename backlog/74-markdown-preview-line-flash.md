# Flash the task's rendered pill when opening a markdown task/await preview

When a note-origin item is opened via "Show in collection" / a `?highlight=` deep link, the corresponding note card scrolls into view and pulses with a highlight animation (backlog #63). Markdown-origin tasks and awaits have an equivalent preview (the `MarkdownPreviewInner` modal, backlog #23) that renders the raw markdown with tasks shown as colored pills (priority-colored, e.g. "High · 1 / Decloudify Roomba"), but opening it does not draw the reader's eye to which pill corresponds to the task that was clicked. It should get the same pulse treatment — applied to just the pill, not the whole source line.

## Dependencies

- [23 - Open in editor — Markdown preview modal](23-open-in-editor.md) — this is the modal being enhanced
- [62 - Note items need a "go to source" arrow](62-note-goto-source-arrow.md) — established the "go to source" pattern this extends to markdown items

## Context / Background

`MarkdownPreviewInner` (`crates/tidl-web/src/components/focus.rs`) already has `source_file` and `source_line` for the task, and renders the file's markdown content with tasks shown as pills. The note-highlight precedent (`crates/tidl-web/src/components/notes/mod.rs`) scrolls the target element into view, sets a `--note-highlight-color` CSS variable, and toggles a `note-card--highlight` class that drives a CSS pulse animation on that specific card, then clears the highlight state after firing (one-shot).

## Functional Requirements

- When the markdown preview modal opens, scroll to `source_line` and apply a brief pulse/flash highlight to the task's rendered pill specifically — not the full line/row it sits on, and not any surrounding markdown text.
- Reuse the same animation approach as the note highlight (pulse via a CSS class + custom property) rather than inventing a new one.
- The flash fires once per open, not repeated while the modal stays open.
- Works for both tasks and awaits, and for both filesystem and git-backed markdown sources (backlog #34).

## Open Questions

- Does the pill already have a stable DOM id/ref keyed to the task (so it can be targeted directly), or does the preview renderer need a small change to make the specific pill addressable?

## Resolution

The renderer needed the small change: `tidl_markdown::render_to_html` now stamps a
`data-md-line="N"` attribute directly on the `note-badge` span for every
`#task`/`#await` pill (`priority_badge_html`/`due_badge_html` in
`crates/tidl-markdown/src/parser.rs`), where `N` is the pill's 1-indexed line
number in the *original* file content — including any YAML frontmatter, so it
lines up with `TaskItem`/`AwaitItem::line` and with `source_line` on the focus
card. `render_to_html` threads a `body_line_offset` (computed in
`extract_frontmatter`) through `process_task_tags` to make that line number
correct even when frontmatter is present.

`MarkdownPreviewInner` (`crates/tidl-web/src/components/focus.rs`) adds a
one-shot `pill_flashed` guard: the first time the Rendered tab's HTML for the
root item (not a wikilink-navigated one) is mounted, it queries
`[data-md-line='{source_line}']`, scrolls it into view, copies the pill's own
inline `color` (present on priority pills; absent on await pills) into a new
`--md-pill-highlight-color` CSS variable, and adds a `md-pill--highlight`
class. `assets/style.css` defines `.md-pill--highlight` /
`@keyframes md-pill-highlight-pulse` as a direct copy of the existing
`.note-card--highlight` pulse (backlog #63) — `animation-iteration-count: 3`
so it self-terminates without any JS cleanup — just retargeted at the pill's
own `box-shadow` instead of a whole card, and defaulting to the same
note-highlight purple when no per-instance color is set (the await-pill
case).

Deviation from the open question's framing: no DOM *id* was added (ids must
be unique per page and the same file can render more than one pill), a
`data-md-line` attribute was used instead — same pattern as `data-note-id` in
the notes grid, just keyed by line number rather than a database id.

# Tickler view — hifi redesign

Redesign the existing `TicklerView` component to match the hifi specification
from the Focus-tab design handoff. Data comes from markdown source files (#18).

## Layout

Same outer shell as the Tasks view. Title "Tickler", subtitle "Date-tagged
pending items from markdown sources".

## Card design

Each pending item is a `focus-card`:
- Left stripe: `#8b5cf6` (purple) for future/today items; `#d93025` (red) for
  overdue items.
- Checkbox circle (same as Tasks / Focus cards).
- Title + meta row:
  - Date pill: purple (`#f0eafe` / `#6b3fc5`) for future/today, red
    (`#fce8e6` / `#c5221f`) with "Nd overdue" for overdue.
  - Topic pill: `#f1f3f4` / `#666`.
  - Source pill: grey dot + filename.
- Hover: "Go to source" action opens markdown source overlay (same as Tasks
  view but `#pending/...` tags coloured `#6b3fc5` in the code block).

## Sorting

Chronological ascending (earliest first). Overdue items float to the top.
Done items stay visible but sink to the bottom.

## Dependencies

- [#18 — Markdown source files](18-markdown-sources.md)

## Resolution

Implemented as specified. Changes made:

- **`types.rs`** — Extended `TicklerItem` with `topic`, `source_file`, `completed` fields.
- **`api.rs`** — Updated `get_tickler_items` SQL to also select `topic`, `source_file`, `status`; maps `status = "COMPLETED"` to `completed: bool`.
- **`tickler.rs`** — Full rewrite: same outer shell as Tasks view (`tasks-view` / `filter-layout`), `focus-card` pattern with inline-coloured stripe (purple `#8b5cf6` / red `#d93025`), functional checkbox, date pill (`pill-date-future` / `pill-date-overdue`), topic pill, and source pill. Filters hidden markdown sources via `HiddenMdSourcesCtx`. Sorts overdue-incomplete → future-incomplete → completed, then by date ascending.
- **`style.css`** — Added `.pill-date-future` and `.pill-date-overdue` classes with dark-mode variants.

Deviation (resolved): the "Go to source" button was initially shipped disabled (same stub pattern as Tasks view for markdown items). It was wired up later (alongside note-origin "go to source" support in Focus/Tasks/Await, see backlog work on 2026-07-23) — it now opens the markdown source overlay for markdown-origin items and the note editor for note-origin items, matching Tasks and Focus.

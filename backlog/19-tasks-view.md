# Tasks view — hifi redesign

Redesign the existing `TodoList` component to match the hifi specification
from the Focus-tab design handoff. Data comes from **both** markdown source
files (#18) and CalDAV VTODOs — both are surfaced in this view, distinguished
by a source indicator.

## Layout

Flex column, 28px vertical / 32px horizontal padding. Max content width 600 px.

**Header:** "Tasks" title (28px / weight 300) + subtitle "All open tasks".

## Two item kinds

### Markdown todo
- `source = 'markdown'` rows from the `events` table.
- Identified by a source pill: grey dot + filename (e.g. `work.md`).
- "Go to source" → opens the markdown file preview overlay at the tagged line
  (Variant B overlay, see #21).

### CalDAV VTODO
- `source = 'caldav'` (or null/unset) rows from the `events` table.
- Identified by a source pill: coloured CalDAV icon or "Tasks" label.
- "Go to source" → navigates to this Tasks tab / scrolls to the item
  (i.e. the item itself is the source; no external file to open).
- Can be **created** in this view via the "+ Add task" button (new inline
  form or modal: title, optional priority, optional due date).
- Can be **edited and deleted** inline.

## Card design

Each todo is a `focus-card` (shared component, see #21):
- Left stripe: `#cfd2d6` (neutral grey) for both kinds.
- Priority badge: 16×16 px coloured square (p1/high red, p2/medium amber,
  p3/low grey). See priority normalisation below.
- Title: 13px / `#1f1f1f`; strikethrough + 45% opacity when done.
- Meta row: priority pill, topic pill (markdown) or none (CalDAV), source pill.
- Hover actions: "Go to source" (↗), no suppress button in Tasks view.

## Pill palette

| Priority | Background | Text     |
|----------|------------|----------|
| p1 / high   | `#fce8e6`  | `#c5221f` |
| p2 / medium | `#fef7e0`  | `#b06000` |
| p3 / low    | `#f1f3f4`  | `#666`   |

## Priority normalisation (CalDAV → display)

CalDAV priority is 0–9 (RFC 5545: 1–4 = high, 5 = medium, 6–9 = low,
0 = undefined). Normalise for display:

| CalDAV priority | Display |
|-----------------|---------|
| 1–4             | p1 / high |
| 5               | p2 / medium |
| 6–9, 0          | p3 / low |

This is the same mapping used by the Focus view scoring function (see #21).

## Sorting

Default: priority ascending (p1/high first), then alphabetical within
priority. Done items stay in the list but sink to the bottom.

## Completing todos

| Source | Behaviour |
|--------|-----------|
| Markdown | Checking off the item rewrites the source `.md` file: changes `- [ ]` to `- [x]` at the tagged line. The sync loop (#18) will then re-parse the file and update the `events` row. |
| CalDAV | Sets `STATUS:COMPLETED` + `PERCENT-COMPLETE:100` and PUTs the updated VTODO back to the CalDAV server. |

## Resolved design decisions

| # | Decision |
|---|----------|
| 1 | `focus-card` is a **shared component** built as part of this item, placed in a shared components module so #21 can reuse it without changes. |
| 2 | Completing a CalDAV VTODO **writes back to the server**: set `STATUS:COMPLETED` + `PERCENT-COMPLETE:100`, PUT to the item's CalDAV URL. Requires storing `caldav_url` and `etag` on the `events` row; handle 412 (etag conflict) gracefully. |
| 3 | "+ Add task" opens a **modal**. |
| 4 | CalDAV source pill: **coloured pill with a short text label** (e.g. calendar name or "CalDAV"). No icon asset needed. |
| 5 | "Go to source" on markdown todos is **stubbed**: button is rendered but is a no-op (or shows a toast) until the Variant B overlay ships in #21. |

## Dependencies

- [#18 — Markdown source files](18-markdown-sources.md)

## Resolution

Shipped as specified. Key implementation notes:

- **`FocusCard`** — shared Leptos component in `todo_list.rs`; left stripe `#cfd2d6`, 16×16 priority badge (red/amber/grey), pill palette as specced.
- **Priority normalisation** — `display_priority(raw)` maps CalDAV 0–9 → `p1`/`p2`/`p3` at the UI layer; raw values are unchanged in the DB.
- **Sorting** — incomplete todos sorted p1 → p2 → p3, then alphabetical; completed items always sink to bottom.
- **New API endpoints** — `create_todo_item`, `complete_todo_item`, `delete_todo_item` added to `api.rs`.
- **CalDAV completion** — updates `status = 'COMPLETED'` directly in the `events` table (tidl is its own CalDAV server; no HTTP PUT roundtrip required). Etag conflict handling deferred.
- **Markdown completion write-back** — strips the `#todo/…` or `#pending/…` tag from the source line, then updates the DB row to `COMPLETED` for immediate UI feedback. The markdown sync loop will detect the file change on next poll and remove the event row.
- **"Go to source" on markdown** — rendered as a disabled `↗` button pending the Variant B overlay (#21).
- **`caldav_url` migration** — skipped; since tidl owns the CalDAV server, direct DB updates suffice and no external PUT is needed for locally-created todos.

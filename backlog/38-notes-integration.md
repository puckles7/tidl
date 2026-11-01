# Todos/Focus/Tickler note integration

Surface to-do-flagged and tickler-flagged notes (from
[37 - Notes tab](37-notes-tab.md)) in the existing Todos, Focus, and Tickler
views, and give those views a "Note collections" sidebar block so users can
show/hide note-collections the same way they already show/hide calendars and
markdown sources. This mirrors the existing dependency shape of
[18 - Markdown sources](18-markdown-sources.md) →
[19 - Tasks view](19-tasks-view.md) / [20 - Tickler view](20-tickler-view.md) /
[21 - Focus view](21-focus-view.md): a new todo *origin* that plugs into
already-existing rendering/ranking pipelines rather than duplicating them.

Design reference: same handoff package as #37 — the "Todos integration
preview" and "Focus integration preview" screens in `Notes Tab.dc.html` are
explicitly **illustrative only** (hardcoded sample data, light-theme-only
mockups) — treat them as a spec for what these tabs need to gain, not literal
components to port.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — provides the `notes`/`note_collections`
  tables and the mirrored-VTODO upsert (`events.source = 'note'`) this item
  reads.
- [19 - Tasks view](19-tasks-view.md), [20 - Tickler view](20-tickler-view.md),
  [21 - Focus view](21-focus-view.md) — existing views being extended.

## Functional Requirements

**Sidebar (`crates/tidl-web/src/components/filter_sidebar.rs`)**
- Add a third collapsible panel, "Note collections", below the existing
  Markdown-sources panel (sidebar order: Calendars → Markdown → Note
  collections), structurally identical to the existing panels: bordered rows,
  colored dot + name, hover-reveal edit pencil, create ("+") / edit / delete
  via the collection editor modal from #37.
- New `HiddenNoteCollectionsCtx` (`RwSignal<HashSet<String>>` of collection
  IDs), provided in `AuthenticatedApp` alongside `HiddenCalsCtx` /
  `HiddenMdSourcesCtx`. Toggling a collection off filters its to-do/tickler
  notes out of Todos, Tickler, and Focus — the same show/hide mechanism
  already used for calendars and markdown sources.

**Todos view (`todo_list.rs`)**
- `get_todos()` already returns any `source` value generically (`COALESCE
  (source, 'caldav')`) — no query change needed.
- When rendering a row with `source == "note"`, show a colored
  `"● <Collection name> note"` pill in the tag slot currently used for the
  markdown filename tag, colored via the note's collection color. Regular
  priority pill (High/Medium/Low) still applies, unmodified — the mirrored
  VTODO's priority is already a representative CalDAV value (2/5/8) that
  `display_priority()` buckets correctly with no code changes (see #37's
  "Priority scale" decision).
- Filter out rows whose originating collection is in
  `HiddenNoteCollectionsCtx`.

**Tickler view (`tickler.rs`)**
- `get_tickler_items()` currently hard-filters `source = 'markdown'` — extend
  to `source IN ('markdown', 'note')` so mirrored tickler-note VTODOs appear.
- Same collection-pill treatment as Todos; same `HiddenNoteCollectionsCtx`
  filtering.

**Focus view (`focus.rs`)**
- To-do/tickler notes flow through the same underlying `events` rows once
  mirrored, so existing urgency bucketing (SOON/UPCOMING/LATER) applies
  unmodified — no ranking-logic changes needed, only the collection-pill
  rendering addition.

## Non-goals

- Any change to the mirroring/upsert mechanism itself (owned by #37).
- CalDAV export changes beyond what already happens for any `VTODO` row
  (mirrored note-todos are already CalDAV-visible for free, same as markdown
  todos are today).

## Open Questions

- None outstanding.

## Resolution

Built as specced, with the collection lookup done server-side rather than
client-side:

- `Todo` and `TicklerItem` gained `collection_id` / `collection_name` /
  `collection_color` fields. `get_todos()` and `get_tickler_items()`
  (`api/events.rs`) now `LEFT JOIN notes n ON n.mirrored_event_uid = e.uid
  LEFT JOIN note_collections nc ON nc.id = n.collection_id`, so the pill's
  label and color ride along on the same row instead of requiring a second
  `list_note_collections()` fetch and a client-side join. `get_tickler_items()`'s
  source filter was extended to `e.source IN ('markdown', 'note')`.
- `HiddenNoteCollectionsCtx` (`RwSignal<HashSet<String>>`, collection ids as
  strings) was added to `filter_sidebar.rs` alongside the existing
  `HiddenCalsCtx` / `HiddenMdSourcesCtx`, provided in `app.rs`. The sidebar
  gained a third "Note collections" panel, structurally identical to the
  Markdown panel, reusing the existing generic `CalendarPill` component and
  `#37`'s `CollectionEditorModal` for create/edit/delete.
- Todos, Tickler, and Focus each filter out note-origin rows whose
  `collection_id` is in `HiddenNoteCollectionsCtx`, and render a
  "● <Collection name> note" pill tinted with the collection's own color via
  a new `note_pill_style()` helper (`focus.rs`) — a `color-mix()` background
  tint plus solid-color text, since the existing source pills
  (`pill-source-markdown` / `-caldav`) are static CSS classes and
  user-defined collection colors need per-instance styling. A matching
  `.pill-source-note` CSS class provides a neutral-grey fallback.
- `todo_stripe_color()` and the Focus view's `stripe_color_for()` /
  `item_meta()` gained a `source == "note"` branch using
  `collection_color` / `collection_name`, matching the existing
  markdown/caldav branches.
- Verified end-to-end against a real `cargo leptos build` server and
  committed the check as a standing Playwright regression test —
  `tests/test_hydration.py::test_note_collection_pill_and_visibility_toggle`:
  creates a note collection, adds a to-do+tickler-flagged note to it,
  confirms the pill appears in Todos/Tickler/Focus, and confirms toggling
  the collection off in the sidebar removes it from Todos. Run via
  `test-hydration <email> <password>` alongside the rest of the suite.

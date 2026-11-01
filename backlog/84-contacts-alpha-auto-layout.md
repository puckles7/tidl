# Contacts — auto-layout should order cards alphabetically

Contacts' auto-layout currently places cards using the same shortest-column masonry placement as Notes (`crate::grid`), which orders by creation/insertion order. For Contacts specifically, alphabetical order (by name) makes more sense as the auto-layout default than insertion order — contacts are looked up by name, not recency, so an alphabetically-sorted grid is easier to scan.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — auto-layout (`auto_layout_contacts`) and the shared `crate::grid` placement math were introduced here.
- [82 - Contacts — drag-to-reorder](82-contacts-drag-reorder.md) — manual drag reordering already exists; this item is about what auto-layout falls back to, not about removing manual reorder.

## Context / Background

- `auto_layout_notes`/`auto_layout_contacts` (server-side, `api/notes.rs`/`api/contacts.rs`) currently share the same placement logic via `crate::grid`, which is order-preserving (places items in whatever order it receives them, not alphabetically).
- Notes should keep its current (non-alphabetical, e.g. creation-order/manual) behavior — this change is Contacts-only.
- Likely needs `auto_layout_contacts` to sort contacts by display name (or last-name/first-name, TBD) before handing them to the shared placement function, rather than a change to `crate::grid` itself.

## Functional Requirements

- When Contacts auto-layout runs (e.g. "Auto-arrange" action, or whatever currently triggers `auto_layout_contacts`), cards are placed in alphabetical order by contact name rather than creation order.
- Manual drag-to-reorder (#82) still works and isn't forced back into alphabetical order on every render — only an explicit auto-layout action re-sorts.
- Notes' auto-layout behavior is unchanged.

## Open Questions

- Sort key: full display name, or last-name-first? Check how contact names are stored/displayed elsewhere in the Contacts tab for consistency.
- Should there be a UI affordance to trigger "re-sort alphabetically" distinct from whatever "auto-layout" already means today, or is auto-layout the only place this applies?

## Resolution

Changed `auto_layout_contacts`'s SELECT (`api/contacts.rs`) to `ORDER BY display_name COLLATE NOCASE ASC` instead of `grid_row ASC, grid_col ASC`, then feeds the same `grid::pack_for_view` placement used before — the sort key resolves the first open question: `display_name` is already the single field used for search/sort/display everywhere else in Contacts, so it was the obvious choice over a last-name-first split that doesn't otherwise exist in the data model. No new UI affordance was added — the existing "Auto-layout" toggle button is already the only trigger for `auto_layout_contacts`, resolving the second open question as "no, auto-layout is the only place this applies." The core logic was factored into a testable `auto_layout_contacts_for` (mirroring the existing `clear_contacts_for`/`clear_contacts` split) with a new `sqlx::test` unit test asserting alphabetical, case-insensitive placement. Notes' `auto_layout_notes` is untouched.

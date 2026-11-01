# Redesign sidebar pills (CalendarPill) to match new mockup

Restyle the row used for every toggleable entry in the app's left-hand
sidebars — calendars, markdown sources, note collections — to match the
redesign mockup at `~/Downloads/tidl-notes.html`, and make sure the change
lands in the one shared component all of these already use, not as
per-screen copies.

## Dependencies

- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md) — note-collection rows in the sidebar are one of the pill's call sites.
- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — contextually related sidebar work, though Contacts has no sidebar pill list of its own today.
- [46 - Restyle filter sidebar to match Notes tab sidebar](46-filter-sidebar-notes-style-parity.md) — prior pass that aligned the sidebar *container* chrome; this item is about the row/pill content inside it.

## Context / Background

`CalendarPill` (`crates/tidl-web/src/components/calendar_pill.rs`) is
already the one shared component behind every sidebar toggle row: a flex
row with a toggle button (colour dot + name) on the left and, only when the
caller supplies the matching callback, hover-reveal edit/sync/share buttons
on the right. It's used by `calendar_list_panel.rs` (calendars),
`filter_sidebar.rs` (markdown sources), and `notes/mod.rs` (note
collections).

The mockup at `~/Downloads/tidl-notes.html` (see its `.collection-row` /
`.row-icon-btn` / `.toggle-wrap` styles, used in the "NOTE COLLECTIONS"
sidebar section) shows a different visual treatment for this same row:

- A small colour dot, then the name in bold, coloured text, filling the
  row.
- A refresh icon button and an edit icon button, each its own small square
  icon button (not hover-reveal — visible whenever applicable), shown only
  when that action actually applies to the entry (e.g. a refresh icon only
  makes sense for a syncable/remote source).
- A proper on/off toggle switch (pill-shaped track + sliding thumb, colour
  matching the entry's own colour when on) at the right end of the row,
  replacing however visibility-toggling currently reads.
- The whole row gets a subtle hover background and rounded corners.

## Functional Requirements

- Restyle `CalendarPill` to match the mockup's row layout: dot, name,
  conditional action icon buttons, toggle switch.
- Icon buttons (refresh/edit, and share if it still applies) only render
  when the corresponding callback/capability is present for that entry —
  same "only shown when applicable" rule the component already follows,
  just restyled to the mockup's always-visible-when-present icon-button
  look instead of hover-reveal.
- The restyled component must be the *only* place this markup/CSS lives —
  calendars, markdown sources, and note collections all pick up the new
  look automatically by using the same `CalendarPill`, with no per-screen
  duplicate styles.
- Preserve existing behavior: click-to-toggle visibility, and whatever each
  icon button already does (edit opens its editor, sync/refresh triggers a
  resync, share opens the share dialog) — this is a visual restyle, not a
  behavior change.
- Consider renaming the component/its file if "CalendarPill" no longer
  reads accurately given it's shared by non-calendar entries too (optional,
  not required for the visual change itself).

## Open Questions

- Exact color/spacing values — pull them from the mockup file directly
  during implementation rather than guessing here.
- Whether the "share" action (calendars/note collections) gets its own
  always-visible icon button matching refresh/edit, or stays hover-reveal
  — the mockup doesn't show a share icon, so this needs a decision before
  implementing.

## Resolution

`CalendarPill` was renamed to `SidebarPill` (`crates/tidl-web/src/components/sidebar_pill.rs`)
and restyled to match the mockup: colour dot, bold coloured name, then
sync/share/edit icon buttons (each its own SVG, shown only when the
matching callback is present), then a real track/thumb toggle switch whose
on-colour is the entry's own colour (via a `--accent` custom property).
All three call sites (`calendar_list_panel.rs`, `filter_sidebar.rs` ×2,
`notes/mod.rs`) were updated to the new name with no per-screen styling.

Deviations / decisions made during implementation and review:

- **Share got its own always-visible icon button** (a share-2-style glyph),
  matching refresh/edit's "shown only when applicable" rule, rather than
  staying hover-reveal — since hover-reveal would have been the only
  remaining inconsistent affordance in an otherwise always-visible design.
- **Icon order is sync → share → edit → toggle**, with edit anchored
  immediately before the toggle rather than icons occupying fixed reserved
  columns. Every entry has edit but not every entry has sync/share, so
  anchoring edit to the toggle (both always present) keeps edit visually
  stable without reserving empty space on rows that lack sync/share —
  found during review that a fixed-column approach wasted too much width
  and truncated even short names.
- Icon buttons + the toggle are grouped in their own tightly-packed
  `.sidebar-pill-controls` cluster (2px internal gap) separate from the
  row's own gap, and `.fs-section` padding was trimmed from 18px to 14px,
  both to give the name column more room — surfaced during manual
  browser review of the three-icon (sync+share+edit) case.
- The row's hover background and hairline `border-bottom` divider between
  rows (mirroring the mockup's `.collection-row` border) live on
  `.fs-list-item`/`.sidebar-pill` — `.fs-list`'s old `gap: 8px` was removed
  in favour of that divider, matching the mockup's tighter row spacing.
- Kept `CalendarPill`'s "always render `on_edit`/`on_sync`/`on_share`'s
  wrapper regardless of `Option`" invariant (documented in the original
  doc comment) — an absent icon is simply omitted from the DOM, not swapped
  for a different layout.

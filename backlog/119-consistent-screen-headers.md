# Consistent Calendar/Focus headers (date, centred pills, right-aligned search)

The Calendar and Focus headers had drifted: Calendar put its view-mode pill on
the right with no search, while Focus had its date/clock, filter pill, horizon
pill, and add buttons bunched against the right edge. This aligns both to one
top-row shape and gives each a working search box.

Notes and Contacts keep their existing toolbars (buttons left, search right).

## Dependencies

- [115 - Merge Tasks and Await into Focus](115-focus-type-filter-pill.md)
- [118 - Manage ad-hoc tasks/awaits](118-adhoc-task-await-management.md)

## Functional Requirements

- Calendar top (only) row: date heading left, Day/Work week/Week/Month pill
  centred, search box right; search filters the visible grid events by
  title/description.
- Focus top row: `+ Task` / `+ Await` left, and the filter pill, horizon pill,
  and search box grouped on the right (wider gap between the right-hand groups
  than between the two add buttons); search filters the ranked list by title
  and source label. The date/clock heading is dropped from Focus.
- Focus second row: a single normal-font (13px) line
  `"{All|Calendar|Task|Await} items · Ranked by urgency · N remaining"`,
  replacing the old all-caps label + count badge.
- Notes and Contacts headers unchanged.

## Resolution

Built as specified.

- `view_header.rs`: removed the date/clock `ViewHeader`; `SectionHeader` is now
  the one-line summary. `format_today_heading` is reused by Calendar instead of
  its hand-rolled heading.
- `focus.rs`: new `.screen-toolbar` row (add buttons, filter pill, horizon
  pill, search); client-side search retains items by title/source; empty-state
  shows "No matches for …" while searching.
- `calendar/mod.rs`: added a search signal feeding `events_memo` (title +
  description filter) and a search box in `.cal-pill-row`, which became a
  3-column grid so the mode pill is truly centred.
- `assets/style.css`: `.screen-toolbar` / `.focus-add-btns` /
  `.focus-result-line`; `.cal-pill-row` switched to a centred grid; removed the
  now-unused `.focus-header`/`.focus-controls`/`.focus-section-*` styles and
  `WEEKDAY_LONG`.

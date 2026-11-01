# #24 — Calendar view toggle pill

## Status
in progress

## Summary

Add a segmented control (pill) to the Calendar view header that lets the user switch between calendar display modes, analogous to the "Today / 3 days / 7 days" horizon pill in the Focus view.

## Options

| Label     | Description                              |
|-----------|------------------------------------------|
| Day       | Single-day column view                   |
| Week      | 7-day week grid (current default)        |
| Work week | Mon–Fri 5-day grid                       |
| Month     | Month grid overview                      |

## Design notes

- Use the same `.focus-horizon-ctrl` / `.horizon-btn` / `.horizon-btn--active` CSS classes (or a renamed equivalent) so the pill renders identically to the Focus horizon selector.
- The selected mode should be persisted in local component state (signal), not in the URL, so it resets on reload (same as the Focus horizon).
- The "Work week" option hides Saturday and Sunday columns from the week grid without removing events.
- The "Month" option requires a new grid layout component.

## Depends on

—

## Resolution

Implemented the four-mode segmented control pill in the calendar view header using the existing `.focus-horizon-ctrl` / `.horizon-btn` / `.horizon-btn--active` CSS classes. State lives in a `CalMode` signal (`Day | WorkWeek | Week | Month`) that resets on reload.

- **Day**: single-column view (reuses `mobile_day` signal for the focused date).
- **Work week**: 5-column Mon–Fri view by slicing `week_monday..+5d`.
- **Week**: 7-column grid (default).
- **Month**: new `MonthGrid` component — 6-week grid with multiday/all-day event bars (using the existing `compute_allday_layout`) and single-day timed events shown as `● HH:MM Summary` lines beneath.

The `do_prev`/`do_next` navigation adapts per mode (month: ±1 month, day: ±1 day, others: ±7 days). The pill is hidden on mobile via `@media (max-width: 767px)` since mobile already uses single-day view. The month event rendering matches Google Calendar style: colored dot + start time + description on one line; multiday events rendered as spanning bars above them.

## Known gaps / remaining work

- **Month: no event creation by click** — clicking an empty day cell in Month view does not open the new-event dialog. Week/Day views already do this; Month should too (click on cell → pre-fill date).
- **Month: no overflow cap** — if a day has many events, they all render and expand the row height. A "+N more" truncation with an expand affordance would keep the grid height predictable.
- **Month: context menu** — right-clicking an event in Month view does nothing. Should show the same Edit / Delete context menu as the week grid.
- **Day mode: mini-calendar click** — clicking a date in the sidebar mini-calendar updates `week_monday` and `mobile_day`, which works for Day mode, but `mini_month` may drift from the displayed day. Needs a review pass.

# All-day and multi-day event display

Improve the calendar view to handle all-day and multi-day events correctly. Currently these events are not rendered well. They should appear in a dedicated horizontal banner area at the top of the day columns, spanning across as many columns as the event covers.

## Background

Calendar apps (Google Calendar, Apple Calendar) separate timed events from all-day events visually: a fixed-height "all-day row" sits above the time grid, and multi-day events stretch horizontally across the days they span. This is also how CalDAV clients expect events with `DATE` (not `DATE-TIME`) values or with a duration crossing midnight to be presented.

iCalendar distinguishes:
- **All-day events** — `DTSTART` has a `DATE` value (no time component), duration is one or more full days
- **Multi-day timed events** — `DTSTART` is a `DATE-TIME` but `DTEND` falls on a later calendar date

Both should be treated the same way in the view.

## Functional Requirements

### All-day / multi-day banner row
- Reserve a horizontal strip above the time grid for all-day and multi-day events
- Each event renders as a horizontal pill/bar spanning the columns for its start through end date
- The strip grows in height if multiple all-day events overlap on the same day (stacked rows, similar to Google Calendar)
- The time grid below adjusts its top offset so it is never obscured by the banner

### Event spanning
- A multi-day event bar starts at the left edge of its start-day column and ends at the right edge of its end-day column
- If the view window clips the event (e.g. week view and the event started last week), the bar starts at the leftmost visible column with a visual indicator (arrow / fade) showing it continues beyond the view
- Same treatment for events that extend past the last visible day

### Interaction
- Clicking a bar opens the event detail / edit view, same as timed events
- Dragging to resize or move (if drag-and-drop is supported in the future) should be scoped to a separate story

### Data / CalDAV correctness
- All-day events must be stored and serialised with `DATE` values (no time, no timezone) in iCalendar
- Multi-day events with `DATE-TIME` values are stored as-is; only the view layer decides they belong in the banner
- Round-trip through CalDAV must not silently convert `DATE` to `DATE-TIME`

## Open Questions

- How many stacked all-day rows do we show before adding a "+N more" overflow indicator?
- Should the banner be collapsible for days with no all-day events?
- Month view: all-day/multi-day events already render horizontally there — ensure the fix is consistent across views.

## Resolution

Implemented the all-day banner in the week view. Key decisions:

- A `week-allday-section` grid row sits above the scrollable time grid, pinned so it stays visible while the time grid scrolls.
- All-day events and multi-day timed events are both rendered as horizontal pills via `compute_allday_layout`, which uses a greedy row-packing algorithm. The banner grows in height to accommodate stacked rows (22 px per row).
- Multi-day timed events receive fractional column edges based on their actual start/end times (e.g. a 12:00 start is at column_index + 0.5), while all-day events span full integer column extents.
- Clip indicators (`‹ ›`) are shown when an event extends beyond the visible week.
- Clicking a pill opens the existing edit dialog. Unit tests cover the fractional positioning and clip logic in `compute_allday_layout`.

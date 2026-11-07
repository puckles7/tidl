# Focus event preview should open the edit-event dialog (with Show in calendar)

Clicking a calendar event on the Focus screen currently opens a read-only
preview dialog (`EventPreviewInner`) whose only action is a link to the calendar
root. Replace it with the real edit-event dialog (`EventFormDialog`), so an
event can be inspected and edited without leaving Focus. That dialog gains a
"Show in calendar" button which navigates to the calendar and briefly flashes
the event — mirroring how "Show in collection" flashes a note.

**Status:** done.

## Dependencies

- [63 - "Show in collection" button on the note-goto modal](63-note-goto-show-in-collection.md) —
  the navigation + flash pattern to mirror.
- [115 - Merge Tasks and Await into Focus with an All/Calendar/Tasks/Await filter](115-focus-type-filter-pill.md) —
  established the current Focus event preview.
- [35 - Recurring event authoring (RRULE UI)](35-recurring-event-authoring.md) —
  the edit dialog's recurrence-scope behaviour must keep working from this new
  entry point.

## Context

- `EventPreviewInner` (`crates/tidl-web/src/components/focus.rs`) is a
  read-only preview; editing is only reachable from the Calendar tab's
  `EventFormDialog` (`crates/tidl-web/src/components/calendar/event_form.rs`).
- Notes already solve the symmetric problem: a deep link to `/notes?highlight=…`
  scrolls to and pulses the target (backlog #63/#134). The calendar has no
  equivalent flash yet.
- The Focus preview is also used for focus-only items; only the calendar-event
  variant changes here.

## Functional Requirements

- Opening a calendar event from Focus shows `EventFormDialog` for that event
  instead of `EventPreviewInner`.
- The edit dialog gains a "Show in calendar" button that navigates to the
  Calendar tab and flashes the event.
- The flash should be a brief, one-shot highlight (same feel as the note card /
  pill flash), located by event uid, and degrade gracefully (navigate without
  flashing) when the occurrence isn't visible in the current calendar view.
- Existing Focus preview behaviour for non-event items is unchanged.

## Open Questions

- How is the flash target encoded in the URL — `?highlight=<uid>` (matching
  notes) — and how does it interact with the calendar's current view/date so
  the event is actually on screen?
- Should "Show in calendar" close the dialog before navigating, and should the
  unsaved-changes guard apply?

## Resolution

- Clicking a calendar event in Focus now opens `EventFormDialog` (the real
  edit dialog) instead of the removed `EventPreviewInner`; saving closes it and
  refreshes the Focus list. Non-event previews are unchanged.
- `EventFormDialog` gained an optional `on_show_in_calendar` prop, rendered as
  a "Show in calendar" button only in Focus (the Calendar tab leaves it off).
- The button client-side navigates to
  `/?highlight=<percent-encoded uid>&date=<YYYY-MM-DD>`. `date` seeds the
  calendar's week/day/month so the event is in the fetched range even when it
  isn't this week; the uid is percent-encoded because CalDAV uids are arbitrary
  strings. (Originally a full-page navigation; switched to client-side routing
  in #165 so the already-loaded WASM/session/cached events survive.)
- CalendarView adds `data-event-uid` to timed blocks, all-day pills, and month
  lines/bars, then pulses the matching element once (`cal-event--highlight`,
  stripped on `animationend`). If the event stays hidden (deselected calendar,
  search filter, or out of range) the navigation still works without a flash.
- No unsaved-changes guard: "Show in calendar" is an explicit navigation away
  from the dialog, and it simply navigates (unmounting the dialog discards
  edits, same as any navigation).

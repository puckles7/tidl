# Recurring event authoring (RRULE UI)

The event editor currently has no controls for creating or editing recurrence rules.
Events with an RRULE imported via CalDAV sync or ICS file are expanded and displayed
correctly (#26), but users cannot author recurring events from within the app.

## Dependencies

- [26 - Recurring event expansion (RRULE)](26-recurring-events.md) — server-side RRULE expansion must already work

## Background

The `rrule` crate is already a workspace dependency (SSR-only). The editor modal
(`cal_edit_modal.rs`) and event editor component (`editor.rs`) would need a new
"Recurrence" section. The RRULE string is part of `ical_data` stored as raw iCalendar
text, so the create/update server functions need to accept a recurrence descriptor and
emit the correct `RRULE:` line in the VEVENT.

## Functional Requirements

- Add a "Recurrence" section to the event editor (collapsed/hidden when not in use).
- Frequency options: Does not repeat / Daily / Weekly / Monthly / Yearly.
- For weekly: allow selection of one or more weekdays (MO–SU).
- End condition: Never / After N occurrences / On date.
- Editing a recurring event: ask whether to edit "this occurrence only", "this and
  future occurrences", or "all occurrences" (standard calendar UX).
- The RRULE is serialised into the stored `ical_data`; no schema change required.
- Deleting a single occurrence of a recurring event creates an `EXDATE` rather than
  removing the whole series.

## Open Questions

- How complex should the initial frequency UI be? A minimal MVP (daily/weekly/monthly/yearly
  with no custom interval) may be enough for a first iteration.
- "This and future" edits require splitting the series (setting `UNTIL` on the original
  and creating a new VEVENT with `DTSTART` at the split point) — worth deferring to v2?

## Resolution

Implemented 2026-06-26.

- Added `rrule`, `occurrence_start`, `is_exception` to `Event`; `rrule` to `NewEvent`.
- Fixed phantom duplicate occurrences: `expand_rrule` now receives an exception-dates `HashSet` (built cheaply from existing UIDs in `get_events`) and skips occurrences covered by an exception row.
- `build_event_ical` now emits `RRULE:` when `NewEvent.rrule` is Some.
- New server fns: `update_event_occurrence` (creates exception VEVENT with `RECURRENCE-ID`) and `delete_event_occurrence` (adds `EXDATE` to master, removes exception row).
- UI: `RecurrenceEditor` component with frequency, interval, weekday picker (weekly only), and end-condition controls. Edit-scope dialog ("This event only" / "All events") shown for recurring master events. Exception rows skip the dialog and always edit that occurrence.
- "This and future" series split deferred to v2 (as agreed in spec).

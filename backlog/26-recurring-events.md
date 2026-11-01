# Recurring event expansion (RRULE)

Calendar events with a recurrence rule (`RRULE`) were only shown on their
original `DTSTART` date. All future occurrences were invisible, making
recurring personal events (weekly lessons, monthly meetings, etc.) effectively
missing from the calendar view.

## Functional Requirements

- Recurring VEVENTs (those containing an `RRULE` property) must be expanded
  into individual `Event` instances, one per occurrence.
- Expansion must respect `EXDATE` entries (cancelled occurrences).
- Occurrences are generated from `DTSTART`, bounded to the date range the
  caller requests (see Resolution — originally this was a fixed two-year
  cutoff from "now", which was later replaced with range-based expansion).
- When the `RRULE` cannot be parsed (unsupported syntax), the event falls back
  to displaying only its original occurrence — same behaviour as before.
- All-day and timed events are both supported.

## Resolution

Implemented at query time in `get_events` (`crates/tidl-web/src/api.rs`):

- Added `rrule = "0.11"` workspace dependency (SSR-only; not linked into WASM).
- Added `expand_rrule()` helper that extracts `DTSTART`, `RRULE`, and `EXDATE`
  lines from stored `ical_data`, feeds them to the `rrule` crate's `RRuleSet`
  parser, and maps each occurrence to an `Event` with the correct start/end
  times (preserving original event duration).
- The `get_events` loop checks for `RRULE:` in `ical_data`; on success the
  expanded occurrences replace the single-event fallback; on parse failure the
  original single event is emitted unchanged.
- No DB schema changes required.

### Follow-up: range-based expansion (fixes indefinite-RRULE cutoff bug)

The original implementation bounded expansion with a fixed
`now + 2 years` wall (`.take_while(dt <= cutoff)`), unrelated to what the
user was actually viewing. Indefinitely-recurring events (no `UNTIL`/
`COUNT`) silently stopped appearing once the calendar was navigated past
that horizon — surfaced by a Google ICS import ("Alice Work").

Fixed by making expansion range-reactive instead of horizon-based:

- `expand_rrule()` (`crates/tidl-web/src/api/mod.rs`) now takes
  `range_start`/`range_end: DateTime<Utc>` and bounds occurrences by overlap
  (`skip_while` on `start + duration < range_start`, `take_while` on
  `start <= range_end`) rather than a fixed cutoff — this also fixes
  multi-day/all-day occurrences that start just before the visible range.
- `get_events(range_start: String, range_end: String)` now requires the
  caller to specify the window ("YYYY-MM-DD"), clamped server-side to a
  5-year max span as a defensive backstop (`clamp_range` in
  `crates/tidl-web/src/api/events.rs`).
- The Calendar view (`crates/tidl-web/src/components/calendar/mod.rs`)
  computes its requested range from the currently visible mini-calendar +
  main grid span (`date_helpers::visible_range`), so navigating the
  calendar — no matter how far forward or back — naturally refetches the
  right window. No more fixed horizon.
- Focus view (`crates/tidl-web/src/components/focus.rs`), which has no
  navigable range concept, preserves its original effective behaviour by
  requesting a fixed `today - 1 day .. today + 730 days` window itself.
- Trade-off: Calendar navigation now triggers a network refetch instead of
  relying on a fully preloaded 2-year dataset. The existing `cached_events`
  stale-while-revalidate pattern softens this.

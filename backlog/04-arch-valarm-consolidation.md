# Consolidate Duplicated VALARM Parser

The VALARM parsing and serialisation logic exists in three separate places with near-identical
implementations, creating a maintenance hazard where a bug fix in one copy won't be applied
to the others.

## Locations of duplication

- `tidl-server/src/alarm/mod.rs` — `parse_simple_duration`, `unfold_ical`, `prop_value`,
  `prop_value_with_params`, `serialise_alarm`
- `tidl-web/src/alarm_parse.rs` — same five functions under slightly different names
- `tidl-web/src/api.rs` — a third independent inline duration serialiser inside
  `build_alarms_ical`

## Fix

Move the shared primitive utilities (`parse_simple_duration`, `unfold_ical`, `prop_value`,
`prop_value_with_params`, `ical_duration_str`) to `tidl-caldav::icalendar_utils`, which is
the natural home for iCalendar utilities. Add `tidl-caldav` as an SSR-only optional dependency
of `tidl-web` so both alarm parsers can import from the shared location.

## Resolution

Implemented. All four utility functions live in `tidl-caldav::icalendar_utils`. Both
`alarm_parse.rs` and `alarm/mod.rs` import from there. The inline duration serialiser in
`api.rs::build_alarms_ical` uses `ical_duration_str` from `tidl-caldav`.

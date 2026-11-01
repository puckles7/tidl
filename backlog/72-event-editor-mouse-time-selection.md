# Event editor — mouse-based time selection

Let the user set an event's start/end time in the event dialog with a
click-to-pick affordance instead of only free-typing into a text field.
Surfaced from buglist item 2 ("Time selection is clunky.. should allow
selection with mouse somehow").

## Context / Background

The other half of buglist item 2 — end date auto-bumping when the start
date changes, and never allowing an end date before the start date — was
implemented directly (2026-07-30, `EventFormDialog`/`DateTimeInput` in
`crates/tidl-web/src/components/calendar/event_form.rs`).

This item turned out to be scoped to the event dialog's own time field, not
the calendar grid: the dialog previously fell back to a bare
`<input type="text">` for 24h-format time entry (and native
`<input type="time">` for 12h/zulu) specifically because native time-input
chrome didn't render consistently across Firefox on Linux and Safari on
iOS — mouse selection was dropped entirely as a result. The fix needed to
restore mouse selection *and* stay reliable across both browsers, without
reintroducing native `<input type="time">`/`<select>`-only widgets.

## Functional Requirements

- `DateTimeInput`'s time field is a single free-text input (always
  canonical `"HH:MM"`, parsed via `date_helpers::parse_hhmm`) paired with a
  custom-drawn click-to-pick popup — plain buttons/divs, not a native
  `<select>`/`<datalist>`/`<input type="time">`, so its rendering doesn't
  depend on browser picker chrome. Applies uniformly to all three time
  formats (12h/24h/zulu); labels are formatted per-format via
  `date_helpers::format_hm`.
- The popup lists every 15-minute mark (96 options) and scrolls to the one
  nearest the field's current value each time it opens.
- Typing a value directly into the field always works alongside the popup
  (same field, not a separate control) — commits on blur/Enter, parsed via
  `parse_hhmm`.
- Changing Start's time (via typing or the popup, forward or backward, even
  across midnight) shifts End by the same delta, preserving the event's
  duration — `date_helpers::bump_end_time`, the timed counterpart of the
  existing `bump_end_date`. One-directional: editing End never moves Start.
- End cannot be set at or before Start when they fall on the same calendar
  day — enforced in both the popup (matching options filtered out) and
  manual typing (rejected on commit), via the `min` prop now carrying
  Start's full datetime rather than just its date.

## Resolution

Implemented in `crates/tidl-web/src/components/calendar/event_form.rs`
(`DateTimeInput`) and `crates/tidl-web/src/components/calendar/date_helpers.rs`
(`parse_hhmm`, `bump_end_time`), 2026-07-30. All functional requirements above
are met: custom-drawn click-to-pick popup (96 15-minute marks, auto-scrolled
to current value) alongside free-text `"HH:MM"` entry, applied uniformly
across 12h/24h/zulu formats; Start-time edits bump End by the same delta via
`bump_end_time`; End is prevented from landing at/before Start on the same
day in both the popup and typed entry via the `min` prop carrying Start's
full datetime.

No deviations from the spec. Along the way, fixed two unrelated pre-existing
CSS bugs surfaced while testing this dialog: a later `.form-row` rule (tuned
for narrow+wide field pairs elsewhere) was silently overriding the event
dialog's Start/End split to 1fr/2fr instead of 1fr/1fr (`.form-row--even`
modifier added to disambiguate), and `.dialog-actions` had its own
horizontal padding stacked on top of `.dialog-form`'s, double-insetting
every dialog's action buttons versus the rest of their content (affects ~12
dialogs, not just this one).

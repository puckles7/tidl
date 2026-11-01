# Focus view — proximity-based calendar event urgency

Calendar events in the Focus view currently score urgency from a fixed
hour-of-day bucket tied to the event's own start time (e.g. "before 9am"
always scores 85), not from how close the event actually is to *now*. As a
result, an event minutes away can score lower than a high-priority todo or
overdue pending item. This wish makes event urgency climb continuously as
the event approaches, trumping todo/pending scores once it's imminent or
in progress, and makes the Focus list re-rank live as time passes.

## Dependencies

- [21 - Focus view — urgency-ranked unified list](21-focus-view.md) — this
  refines the scoring function and ranked-list component introduced there.

## Functional Requirements

- `event_urgency` gains a `now_local: &str` parameter (already computed in
  `FocusView` for horizon filtering, just not previously threaded into
  scoring).
- For today's **timed** events, replace the flat hour-of-day bucket with
  proximity tiers, checked in this order:
  1. In progress (`start <= now <= end`) → **99**
  2. Starting within 15 min → **99**
  3. Starting within 1h → **95**
  4. Starting within 3h → **90**
  5. Beyond 3h (or already ended) → existing hour-of-day bucket
     (85 / 80 / 75 / 70), unchanged
- All-day events today stay flat at **65** — proximity doesn't apply to a
  day-granularity event.
- Days other than "today" (tomorrow / ≤3 days / later) are unchanged.
- The Focus list re-ranks live: a `now_tick` signal increments every 60s
  (hydrate-only `set_interval`, same pattern as the live clock in
  `crates/tidl-web/src/components/calendar/mod.rs`) and is read inside the
  existing reactive closure in `FocusView` so the ranked list recomputes
  every minute without re-fetching data.
- `include_item` / horizon filtering is unchanged — this only affects the
  score, not inclusion.

## Open Questions

- None — resolved during brainstorming (see chat history for the tier
  values and rationale).

## Resolution

Implemented as designed:

- `event_urgency()` in `crates/tidl-web/src/components/focus.rs` now takes
  `now_local: &str` and, for today's timed events, checks in-progress /
  within-15min / within-1h / within-3h tiers (99 / 99 / 95 / 90) before
  falling back to the existing hour-of-day bucket. A same-calendar-day
  guard prevents a stale tick from misfiring proximity scoring across a
  midnight boundary.
- Added a `now_tick: RwSignal<u32>` incrementing every 60s via a
  hydrate-only `set_interval` (mirrors the live clock pattern in
  `calendar/mod.rs`), read inside `FocusView`'s existing reactive closure so
  the ranked list recomputes and re-sorts live.
- Added test coverage for in-progress, all three proximity tiers, the
  already-ended fallback, and the day-boundary guard; existing hour-bucket
  and determinism tests updated for the new function signature.
- No deviations from the design as brainstormed.

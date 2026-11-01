# Calendar event priority attenuation

Focus currently scores calendar events purely by how soon they start (the
`calendar_time_left` curve). That makes a genuinely unimportant event — a
recurring optional stand-up, say — outrank an urgent task once it is
imminent. This wish lets an event carry a priority, reusing the priority
scale and CalDAV `PRIORITY` property the rest of the app already uses, and
caps the event's time-based urgency by that priority so an unimportant
event can never trump an urgent non-calendar item.

## Context / Background

Full design: `docs/superpowers/specs/2026-10-29-event-priority-attenuation-design.md`.
Implementation plan: `docs/superpowers/plans/2026-10-29-event-priority-attenuation.md`.

Related: #21 (Focus view), #36 (proximity urgency), #68 (priority curves),
#70 (canonical 1-100 priority scale).

## Functional Requirements

- Per-event priority, stored on the existing `events.priority` column
  (canonical 1-100, 1 = highest) and projected onto RFC 5545's 1-9
  `PRIORITY` at the iCal boundary. Unset means "no attenuation".
- Legacy rows with a `NULL` column fall back to the `PRIORITY` property in
  `ical_data`, so imported and iMIP events retain priority without a
  backfill.
- Event urgency becomes
  `min(calendar_time_left_score, eval_curve(calendar_event_priority, p))`
  when a priority is set. The new curve is a fourth per-user, tunable
  Priority Curves entry; its default caps normal priority at 80 and the
  lowest priority at 60, so a prioritised event can never reach URGENT.
- Event editor: a "Set priority" toggle plus the existing 1-100 slider;
  the slider previews the attenuation cap rather than the task-priority
  label.
- Recurring events inherit the master's priority on every occurrence.
  Editing "this occurrence only" gives the exception its own copy; editing
  "all events" updates the master, leaving already-created exceptions
  alone (standard CalDAV semantics).
- Shared calendars work unchanged: the shared event query selects the same
  priority column, and external CalDAV clients see the written `PRIORITY`
  in the shared calendar's iCal data.

## Open Questions

None — resolved during brainstorming (ceiling semantics, fourth curve,
existing slider + toggle, standard recurrence exception behavior).

## Resolution

Implemented as designed (see design and plan links above). Event priority is
read from `events.priority` (canonical 1-100) with an `ical_data` `PRIORITY`
fallback for legacy imported rows; it caps `event_urgency` through the new
`calendar_event_priority` curve, default `(1,100)->(100,60)`. The event
editor exposes an opt-in toggle + the shared 1-100 slider, and the iCal
`PRIORITY` property keeps recurring occurrences and external CalDAV clients
in sync. Verified: `cargo test --workspace` (697 passed), `cargo clippy
--workspace --all-targets -- -D warnings`, the wasm32 hydrate build, a
hash-free `cargo leptos build`, and the full Python e2e suite (122 passed).

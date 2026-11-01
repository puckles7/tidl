# Show why a Focus item has its current urgency

Focus ranks every item by a single 0–100 urgency score, surfaced only as a
label (`LATER` / `UPCOMING` / `SOON` / `URGENT`) and a bar. The score is opaque:
two items can show the same label for completely different reasons (a high
priority with no deadline vs. a low-priority task that is due tomorrow vs. a
calendar event about to start). Add a lightweight, optional explanation — e.g.
on hover over the urgency cluster, or a tap on mobile — that breaks the score
down into the inputs that produced it, including the user's configured
priority curves (backlog #68).

**Status:** done.

Design: [2026-10-30 Focus urgency explanation](../../docs/superpowers/specs/2026-10-30-focus-urgency-explanation-design.md).

## Dependencies

- [68 - Configurable priority curves](68-configurable-priority-curves.md) —
  urgency is curve-driven, so the explanation must reflect the user's curves
  rather than hard-coded thresholds.
- [21 - Focus view — urgency-ranked unified list](21-focus-view.md) — the list
  and urgency cluster this annotates.
- [36 - Focus view — proximity-based calendar event urgency](36-focus-proximity-urgency.md) —
  event urgency's time-left curve.
- [115 - Merge Tasks and Await into Focus with an All/Calendar/Tasks/Await filter](115-focus-type-filter-pill.md) —
  the three item kinds each compute urgency differently.

## Context

- `event_urgency` uses the `calendar_time_left` curve keyed on hours until
  start, floored to `0` while the event is in progress.
- `task_urgency` is `max(priority_score, pending_urgency(due))` — a TASK with
  both a priority and a due date is ranked by whichever contributes more.
- `pending_urgency` uses the `due_date_time_left` curve keyed on days until
  due; AWAIT items use only that (they have no priority).
- `task_priority_score` maps the canonical 1–100 priority through the
  `source_priority` curve.
- Every card already renders an `fsc-urgency` cluster with label + bar, which
  is the natural anchor for the explanation.

## Functional Requirements

- On hover (desktop) — and by tap/focus for keyboard and touch users — show
  which input(s) determined the score, e.g. "Priority 80 → 72 · due in 2 days
  → 65 (using 72)" or "Starts in 45 min → 81".
- For events, indicate the in-progress floor when it applies.
- Show the labels/values actually used, not the raw curve internals; keep it
  visual and compact (a tooltip/popover consistent with the existing
  `fsc-urgency` styling).
- Must not change ranking, scores, or default (non-hover) rendering.
- Keep the explanation reactive to the current time tick (the Focus list
  already re-evaluates urgency every 60s).

## Open Questions

- Tooltip vs. popover vs. an expandable row? Needs to work on narrow/mobile
  screens where hover doesn't exist.
- Should the UI show the raw curve name/threshold for power users, or only the
  human-readable contributing factors?
- Do suppressed/completed items get an explanation (e.g. "suppressed"), or
  only active ones?

## Resolution

Implemented as a hover/focus/tap popover anchored to the existing
`fsc-urgency` cluster (design:
[2026-10-30 Focus urgency explanation](../../docs/superpowers/specs/2026-10-30-focus-urgency-explanation-design.md)).
Each contributing input is shown with the value the user's Priority Curves
produced — task priority, due-date proximity, event start proximity (with the
in-progress 0h floor), and an event's priority cap — and the row(s) that
determined the final score are highlighted. The closed cluster renders exactly
as before; completed items keep the plain `DONE` cluster and suppressed items
keep the cluster hidden, both without a breakdown.

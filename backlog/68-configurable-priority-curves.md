# Configurable priority curves

Focus's urgency ranking maps each item's urgency input (calendar time-to-
event, task/await time-to-due, task's own priority) to a 0-100 score via
hardcoded step functions in `components/focus.rs`. The last five commits on
that file were all manual retuning of those thresholds, showing the fixed
mapping doesn't fit everyone and needs to be user-configurable instead of
re-tuned in code every time. This wish lets each user reshape three
independent draggable-point curves (one per source kind: calendar events,
due dates, note/task priority) in a new Settings tab.

## Context / Background

Full design: `docs/superpowers/specs/2026-07-29-priority-curves-design.md`.
Implementation plan: `docs/superpowers/plans/2026-07-29-priority-curves.md`.

## Functional Requirements

- Three independently-configurable curves, one per user, one per source
  *kind* (not per individual calendar/markdown source/note collection):
  calendar time-left, due-date time-left, source priority.
- New "Priority Curves" tab in Settings with a drag-only SVG graph editor
  per curve (click line to add a point, drag to move, drag-off/double-click
  to remove, minimum 2 points).
- Shipped defaults reproduce today's hardcoded thresholds so no existing
  user's Focus ranking changes until they edit a curve.
- Curves persist server-side (new `priority_curve_points` table) and apply
  wherever urgency scoring currently happens: Focus, Tasks, Tickler, and the
  Notes-tab priority slider/badges.

## Open Questions

None — resolved during brainstorming (see design doc's Non-goals section
for explicitly deferred scope: per-individual-source curves, a live-preview
readout, numeric point-entry fields).

## Resolution

Implemented as designed — see the design spec and implementation plan
linked above. `PriorityCurvesCtx` wired at the app root; `event_urgency`
rewritten to a single continuous "hours until start" curve (dropping the
old day-bucket + hour-of-day-bucket + intraday-proximity-override
structure entirely, per the approved design); `task_priority_score`,
`pending_urgency`, `task_urgency`, and the Notes-tab priority slider/badges
all take an explicit `curves: &PriorityCurves` parameter rather than
reading it via `use_context` internally (deviation from an earlier draft of
the spec's "Client wiring" section, corrected during planning — every
existing caller of these functions reads context/signals before entering a
`Suspend::new` async block, never inside, and ~40 pre-existing unit tests
call these functions with no Leptos runtime at all). The `CurveEditor` drag
component reads `ev.current_target()` rather than a typed SVG `NodeRef`,
since this codebase had no prior typed-SVG-element usage to build on.
`profile.rs`'s `USERNAME_COLUMNS` list gained a `priority_curve_points`
entry so the username-rename cascade covers the new table (caught by its
own exhaustiveness test). Verified: `cargo test --workspace` (325 passed),
`cargo clippy --workspace -- -D warnings` (clean), `cargo build --target
wasm32-unknown-unknown -p tidl-web` with and without `--features hydrate`
(clean), a full `cargo leptos build` + server smoke-start confirming the
migration applies and the server starts without error. No real browser was
available in this environment to click through the drag interaction itself
— that manual check is still open for whoever reviews this.

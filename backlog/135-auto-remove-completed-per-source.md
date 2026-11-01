# Auto-archive completed items per source

The Done settings tab stores a per-source retention age and applies it on save
("Save & apply"). Nothing runs automatically, so a completed item only leaves
its source when the user re-saves the settings or runs "Clean up all matching".

**Status:** done.

## Resolution (2026-10-09)

Shipped an automatic retention sweep. `tidl-server` gained a `done_sweep`
background loop (hourly, alongside the markdown/remote/webcal sync loops) that
calls a new SSR entry point `tidl_web::api::sweep_due_done_items`: for every
`done_scopes` row carrying an `older_than_days` rule it reconciles the owner's
index once, then strips + archives the matching items via the same
`purge_done_items` engine (`Retention::OlderThan`). The `purge_done_items`
server fn and the sweep now share one `purge_scope` helper, so the manual
"Save & apply" / "Clean up all matching" paths and the automatic sweep can never
diverge. A scope whose reconcile fails is skipped rather than purged against a
stale index.

Cadence decision: a background loop, not a lazy Done-view GET — the latter would
perform markdown writes + git pushes during a read, which the design
deliberately avoids. The per-scope `auto_archive` (archive the instant an item is
completed) column stays scaffolded-but-unwired; this item shipped the retention
sweep only.

## Dependencies

- [124 - Clean-up for completed tasks](124-done-task-cleanup.md) — the archive
  engine this would drive on a schedule.
- [125 - Central done-item tracking](125-central-done-tracking.md) — where the
  per-source settings live.

## Functional Requirements

- A per-source (or global) automatic sweep: completed items older than the
  configured age are stripped from their source and archived without manual
  action.
- Reuse the strip+archive engine (`purge_done_items`).
- Decide cadence: a background/scheduled job, or lazily on Done-view load —
  noting that the latter performs source-file writes during a GET, which the
  current implementation deliberately avoids.

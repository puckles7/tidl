# Done — archive a single item from the list

The Done screen only offers bulk clean-up ("Clean up all matching"), driven by
the active filter/search. There is no way to archive one specific done item from
its source without first narrowing the filter to just it.

**Status:** done.

## Dependencies

- [124 - Clean-up for completed tasks](124-done-task-cleanup.md) — the archive
  engine this extends.
- [125 - Central done-item tracking](125-central-done-tracking.md) — the list
  the action lives on.

## Functional Requirements

- A per-item "archive" hover action on each live done row — stripping just that
  item's `#done` tag/token (or deleting its VTODO) and archiving its index row.
- Confirm before archiving, naming the source (reuse the Done confirm dialog).
- Server: accept an explicit id set (the current `purge_done_items` takes a
  filter + retention only), e.g. `purge_done_ids`.

## Resolution

Built on `feat/done-remove-single-item`.

- **Server** (`crates/tidl-web/src/api/done.rs`): extracted the origin-strip +
  archive loop out of `purge_done_items` into a shared `apply_done_archive`
  helper, then added `purge_done_ids(ids: Vec<i64>) -> PurgeResult`, backed by
  `targets_by_ids` (live rows only, scoped to the caller). The bulk purge now
  just computes targets then calls the same helper — no logic duplicated.
- **UI** (`crates/tidl-web/src/components/done.rs`): each live row's hover
  actions gain a red `✕` **Archive** button (archived rows are
  read-only and show no button). It opens the existing `ConfirmModal`,
  naming the source label, then calls `purge_done_ids` and refreshes.
- **CSS**: `.fsc-action-btn--danger` hover treatment (light + dark).
- **Tests**: `targets_by_ids_only_returns_live_own_rows` covers live-only,
  own-user scoping and unknown ids.
- **README**: Done feature bullet updated.

No deviations from the spec; no open design choices remained.

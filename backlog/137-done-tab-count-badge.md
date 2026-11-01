# Done tab: live count badge in the navbar

The Done tab gives no indication of how many live (non-archived) completed items
it holds. A small count badge on the navbar entry would surface it, matching the
open question left in backlog #125.

**Status:** done.

## Dependencies

- [125 - Central done-item tracking](125-central-done-tracking.md) — the tab this
  annotates.

## Functional Requirements

- Show a live done count next to the "Done" nav link, updating as the index
  changes.
- Count live items only (archived history excluded).

## Resolution

- New `count_live_done_items` server fn (`api/done.rs`): reconciles the done
  index, then `COUNT(*)`s live (`archived_at IS NULL`) rows. A shared
  `DoneRefresh` context signal (a newtype, per the `TypeId`-collision rule)
  drives a `LocalResource` in `AuthenticatedApp`; the navbar link renders a
  `.nav-count-badge` pill whenever the count is > 0.
- The reconcile is deliberate — `done_items` is a cache only refreshed by
  reconcile, so a just-completed/re-opened item wouldn't show otherwise. To
  avoid a reconcile storm it is *not* wired to the SSE-driven `EventsRefresh`;
  instead `DoneRefresh` is bumped at every in-app done mutation (Focus card
  checkboxes, the edit-modal "mark as done", the markdown preview checkbox, the
  note-card token checkbox, the Done view's re-open/purge, and Settings' Done
  clean-up).
- Deviation: external CalDAV/markdown completions don't move the badge until the
  next in-app change — that's the separate #136. `clear_archived_done_items`
  doesn't bump either, since it only touches archived rows.
- Test: `count_live_rows_excludes_archived` (`api/done.rs`).

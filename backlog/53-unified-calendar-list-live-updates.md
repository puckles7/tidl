# Unify calendar-list sidebar + live share/calendar updates

Merge the Calendar page's independent "calendar list with visibility toggle" implementation with the one used by Focus/Todos/Tickler, and finish the SSE push wiring so shared-calendar and calendar-metadata changes (share granted, share revoked, calendar deleted) appear in any already-open browser tab without a manual refresh.

## Dependencies

- [41 - Make calendar shares actually usable (web UI + raw CalDAV)](41-caldav-share-exposure.md) — this closes the last gaps that item's Follow-ups 1 and 4 left behind (the same shared-calendar bug had to be fixed twice because these two implementations were never actually unified, only visually restyled to match)

## Context / Background

While fixing #41 Follow-up 4 (shared calendars missing from the Calendar page specifically), it became clear the Calendar page and the Focus/Todos/Tickler pages never shared code for their "Calendars" sidebar section — only CSS classes, from an earlier visual-unification pass (`9e39f22`, 2026-05-17) that never touched the underlying data model. Calendar page: `CalendarsCtx`/`SharedCalendarsCtx` (`Vec<(T, bool)>`, Effect-populated, persisted at the App level, also driving the event-grid's visibility filter). Focus/Todos/Tickler: `HiddenCalsCtx` (a bare hidden-slug `HashSet<String>`), each page independently fetching its own copy of `get_calendars()` for color lookups.

Separately, sharing-related state changes didn't notify the affected browser tab at all — `revoke_calendar_share` only notified the acting owner, and accepting an invitation sent no notification to anyone. A user with a tab already open wouldn't see a newly-shared (or newly-revoked) calendar until they manually reloaded.

Full design: `docs/superpowers/specs/2026-07-20-unified-calendar-list-design.md`.

## Functional Requirements

- One `AllCalendarsCtx` (`RwSignal<Vec<(CalendarEntry, bool)>>`) replaces `CalendarsCtx`, `SharedCalendarsCtx`, and `HiddenCalsCtx`, keyed by `(owner_username, calendar_name)` rather than bare name.
- One `CalendarListPanel` component replaces both `calendar/calendar_list.rs::CalendarList` and `FilterSidebar`'s inline Calendars block.
- `revoke_calendar_share`, the share-accept flow, and `delete_calendar_api` all notify every affected account over the existing SSE broadcast (`change_tx`) so open tabs update live.

## Resolution

Implemented largely as designed, with two corrections found during implementation that the design doc got wrong:

**Data model & component** (`types.rs`, `app.rs`, `calendar_list_panel.rs`): added `CalendarEntry` (+ `merge_calendar_entries`, unit-tested: owned-only, shared-only, read-only-permission mapping, owned/shared same-name non-collision, visibility preserved across refetch, new entries default visible) and `AllCalendarsCtx`. `CalendarListPanel` owns the fetch (`get_calendars()` + `get_shared_calendars()`), merges via `merge_calendar_entries`, and renders one `CalendarPill` per entry — owned entries get edit/sync buttons (via `CalendarEntry::to_calendar_info()`), shared entries get neither. `CalendarPill.on_edit` changed from `#[prop(optional)]` to `#[prop(default = None)]` (matching `on_sync`'s existing pattern) since it now needs to be conditionally `Some`/`None` per entry rather than always supplied — the four pre-existing call sites (Calendar, Markdown sources, Note collections `×2` via `filter_sidebar.rs`) were updated to wrap in `Some(...)`.

`calendar/calendar_list.rs` deleted. `calendar/mod.rs`'s `events_memo` now filters by `(owner_username, calendar_name)` compound key — matched against `Event.owner_username`/`Event.calendar`, which already carried the right values (no server change needed) — closing the same-name owned/shared collision bug noted as a known limitation in #41 Follow-up 1. `calendar_loaded`'s gating simplified to depend only on `events_res` (previously also waited on `cals_res`, which no longer lives in this component). `focus.rs` and `todo_list.rs` dropped their independent `get_calendars()` fetches, reading `AllCalendarsCtx` instead (Task/AwaitItem don't carry `owner_username`, so their hidden-check/color-lookup stay bare-name-keyed — only Event items get the compound-key treatment, since only events currently support cross-owner sharing).

**Corrections vs. the design doc:** the design predicted `event_form.rs`'s calendar-creation picker offering read-write shared calendars would be "a small, direct consequence of the refactor." That's wrong — `create_event` and `create_todo_item` always write under `INSERT ... (username, ...)` bound to the *current* user with no owner-resolution at all (unlike edits to an existing shared item, which already resolve cross-owner via `resolve_event_write_owner`). Offering a shared calendar as a creation target today would silently create a same-named phantom calendar under the wrong user instead of writing to the actual shared one. Both pickers were kept owned-only, matching current (unchanged) behavior — real cross-owner creation support would be its own item, not a side effect of this one.

**Live-update wiring**: `revoke_calendar_share` now notifies both the owner and the grantee (`ChangeKind::All`, since the grantee loses both the calendar and its events). `accept_share_invitation` (`tidl-server/storage.rs`) now returns the owner's username on success (`Result<()>` → `Result<String>`, trait + forwarding impl + tests updated) so `accept_handler` can notify both the accepting session's username and the owner via `state.change_tx` directly — no need to route through `tidl-web`'s `notify_change` helper, since `tidl-server` already holds the same broadcast sender. `delete_calendar_api` now looks up grantees before dropping the `calendar_shares` rows (closing a previously-untracked orphaned-row bug — those rows had no cleanup path at all) and notifies each one after commit. No new client-side plumbing needed for any of this: `CalendarListPanel`'s fetch already depends on the `cals_refresh` signal, which the existing SSE handler in `app.rs` already bumps on a `"cals"`/`"all"` message.

Not covered (explicit scope cuts, matching the design doc): Markdown sources and Note collections panels are untouched, sharing only `CalendarPill`'s visual style as before. Cross-owner event/task *creation* on a shared calendar remains unimplemented (see correction above). `delete_calendar_api`'s new share-cleanup queries have no dedicated unit test — this file has no existing test precedent to extend (confirmed via grep: zero `#[cfg(test)]` in `api/calendars.rs` before this change), so this follows the file's existing convention rather than introducing a new one; recommend E2E coverage if this file gains a test suite later.

Verified: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo build --target wasm32-unknown-unknown -p tidl-web` all clean.

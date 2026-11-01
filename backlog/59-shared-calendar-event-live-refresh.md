# Shared calendar events don't push live to grantees

Remote CalDAV source sync refreshes a user's own view automatically as soon
as its interval elapses, but a calendar shared *with* a user by another
local account doesn't: editing an event in a shared calendar only updates
the editor's own tab, and every grantee has to manually refresh to see it.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — the sharing mechanism this bug affects
- [41 - Make calendar shares actually usable](41-caldav-share-exposure.md) — introduced cross-owner shared-calendar writes

## Context / Background

Root cause (found via systematic debugging): the SSE change-notification
system (`crates/tidl-server/src/main.rs`'s `sse_changes` handler) filters
the broadcast channel per connected browser by comparing the *acting*
username embedded in the message (`"<kind>:<username>"`,
`ChangeKind::to_msg`) against that browser's own session username. Every
event-mutating `#[server]` function in `crates/tidl-web/src/api/events.rs`
(`create_event`, `update_event_times`, `update_event`, `delete_event`,
`update_event_occurrence`, `delete_event_occurrence`) called
`notify_change(&username, ChangeKind::Events)` with only the *acting* user
— the calendar's owner when they edit their own calendar, or the grantee
themselves when writing to someone else's shared calendar via
`resolve_event_write_owner`. Nobody else watching that calendar (the owner,
when a grantee edits; every other grantee, always) ever got an SSE push.

This is exactly the pattern already fixed for calendar-*level* changes:
`delete_calendar_api` and `clear_shared_calendar`
(`crates/tidl-web/src/api/calendars.rs`) already look up every row in
`calendar_shares` for the affected `(owner_username, calendar_name)` and
call `notify_change` for the owner plus each grantee — that's why grant
creation/revocation and calendar deletion already "just work" live, while
event-level edits within an already-shared calendar didn't.

## Functional Requirements

- Every event mutation in `crates/tidl-web/src/api/events.rs` notifies the
  calendar's real owner and every one of its grantees, not just whoever
  made the change.
- `update_event` additionally notifies the *old* calendar's watchers when
  an edit moves an event to a different calendar (its own change already
  bumps that calendar's CalDAV sync token separately).

## Resolution

Added two private (now `pub(super)`) helpers in `crates/tidl-web/src/api/events.rs`:
`calendar_grantees(pool, owner_username, calendar_name) -> Vec<String>`
(the same `calendar_shares` query already used ad hoc in
`api/calendars.rs`) and `notify_calendar_watchers(pool, owner_username,
calendar_name, kind)`, which calls `notify_change` for the owner then every
grantee. Replaced the plain `notify_change(&username, ChangeKind::Events)`
call in all six event-mutation functions with this helper, using each
function's already-resolved real owner (`effective_username` where
`resolve_event_write_owner` applies, `username` where the function only
ever touches the caller's own calendar). `update_event` calls it twice when
the event's calendar changed, once for the old calendar and once for the
new. Also switched the two other narrow `notify_change` call sites in
`api/calendars.rs` that mutate a calendar's events (`refresh_webcal_calendar_api`,
`import_ics`) to the same helper.

**Follow-up (same day):** manual live verification surfaced a second copy of
the same bug living entirely inside `tidl-server`, outside the reach of the
`tidl-web` fix above — background sync loops and direct CalDAV writes
`change_tx.send` the SSE message themselves and can't call into
`tidl-web`'s private helper (the compilation boundary, CLAUDE.md §8) or even
its own crate's private fn. Added `crates/tidl-server/src/notify.rs` with an
equivalent `notify_calendar_watchers(storage, change_tx, owner_username,
calendar_name, kind)` built on `CalendarStore::list_shares_for_calendar`
(already on the trait), and wired it into every mutation path that
previously notified only the acting/owning user:
- `remote_sync.rs` — both the periodic tick loop and the manual
  "sync now" trigger path. This was the specific case a manual repro caught:
  a remote CalDAV pull (3061 `shared` → 3062 `New shared`) updated alice's
  own view live but never reached bob, who has `New shared` shared with her
  on the 3062 instance.
- `webcal_sync.rs` — the hourly feed-refresh loop.
- `caldav_handlers.rs` — direct CalDAV `POST`/`PUT`/`DELETE` from external
  clients (iOS Calendar, DAVx⁵, Thunderbird) writing to a shared calendar.
- `smtp_listener.rs` — inbound iMIP email import into a shared calendar
  (kept as a local raw-query notify rather than threading the `CalendarStore`
  trait through the whole SMTP listener, since it already had `pool` and
  this is the only mutation site there).

`markdown_sync.rs` was deliberately left alone — markdown todos live under
the synthetic `__markdown__` calendar label, which has no backing
`calendars` row and can't be shared via `calendar_shares`.

Verified live end-to-end (not just unit tests): ran two local instances
(ports 3061/3062, per the existing `run-tidl` dev script) matching the
reporter's exact setup, subscribed to `/api/stream` as the grantee (`bob`)
with a cookie-authenticated `curl -N`, then `PUT` an event change directly
via CalDAV as the owner (`alice`) on the 3061 instance. Confirmed via the
grantee's raw SSE stream (`data: events`) and server debug logs
(`list_shares_for_calendar` returning the grantee) that the push now
reaches her without a manual refresh.

Added `calendar_grantees_tests` (`returns_every_grantee_regardless_of_permission`,
`returns_empty_for_an_unshared_calendar`) covering the query itself; the
SSE dispatch path isn't unit-testable in isolation since `notify_change`
requires a live Axum request context, matching the existing test coverage
style in this file. `cargo test --workspace`, `cargo clippy --workspace -- -D
warnings`, and `cargo build --target wasm32-unknown-unknown -p tidl-web` all
pass.

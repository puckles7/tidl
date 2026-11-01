# Make calendar shares actually usable (web UI + raw CalDAV)

Let a calendar shared via [13 - Calendar sharing](13-calendar-sharing.md)
actually show up and be usable for the grantee — both inside the tidl web
UI's own Calendar view, and as an additional calendar inside their own raw
CalDAV client (iOS Calendar, DAVx⁵, Thunderbird). Originally scoped as
CalDAV-only; expanded after discovering neither surface actually consumes
share grants today (see #13's "Known Gap" section).

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — this consumes its
  existing `calendar_shares` grants; no new sharing/invite mechanism is
  introduced here.

## Context / Background

Tracing how a `calendar_shares` grant is actually used today turned up two
separate, previously-unnoticed gaps:

**Web UI:** `get_shared_calendars()` (`api/calendars.rs`) — the only
function that reads "what's shared with me" — is used exclusively by the
Settings → Sharing tab (`components/settings/sharing.rs`) to render an
informational list. `get_events()`, which powers the actual Calendar view,
is hard-scoped to `WHERE e.username = ?` with no join against
`calendar_shares` — a shared calendar's events never render for the grantee
anywhere. `update_event` / `delete_event` / `complete_todo_item` are
similarly scoped to `username = current_user` with no cross-user path at
all, regardless of the grant's `permission` value. So even logged into the
tidl web app as the grantee, a shared calendar is invisible and unusable
today — sharing only ever produced a row in `calendar_shares` and a listing
of that fact.

**Raw CalDAV:** every handler in `caldav_handlers.rs` (GET, PUT, PROPFIND,
REPORT, DELETE, MOVE — all of them) calls
`require_same_user(&auth, &username)`, which rejects any Basic Auth identity
that doesn't exactly match the calendar-owner username in the URL path, with
no exception for a `calendar_shares` grant. So a native CalDAV client (iOS
Calendar, DAVx⁵, Thunderbird) logged in as the grantee has no way to
discover or access a calendar shared with them either.

**Existing workaround (works today, zero new code, but coarse):** hand the
grantee an app token for the *owner's* account and have them add it as a
second, separate CalDAV account entry in their client. This exposes **all**
of the owner's calendars, not just the one meant to be shared —
`AppTokenInfo` has no per-calendar scope (only an account-wide `read_only`
flag) — and it shows up as a distinct account, not one more calendar folded
into the grantee's own existing tidl account (web or CalDAV).

**Target UX:** the shared calendar appears as one more calendar inside the
grantee's *own* existing view/account, in both the web UI and their own
CalDAV client — no second account, no all-or-nothing token. CalDAV itself
(RFC 4791) has no notion of sharing; two real-world precedents exist for how
servers bolt it on:

- **WebDAV ACL (RFC 3744):** a resource carries a `DAV:acl` granting specific
  privileges to non-owning principals *at that same URL*. Under this model,
  "same username as the URL" is the wrong authorization primitive to begin
  with — checks should be resource-plus-principal, not path-plus-username.
- **Apple CalendarServer-style sharing** (used by Sabre/DAViCal/iCloud): a
  shared calendar is exposed as its own resource under the *sharee's* own
  calendar-home-set (e.g. a synthetic child of `/caldav/{grantee}/`),
  discovered via the sharee's own principal PROPFIND, each with its own ACL.
  This is the shape that actually produces "one more calendar shows up"
  without a second login.

tidl doesn't implement WebDAV ACL, and building general-purpose ACL support
is a much bigger project than this needs. The CalendarServer-style approach
fits tidl's existing shape better: synthesize an aliased resource under the
grantee's own namespace that transparently proxies to the owner's real
calendar storage. The web UI side is simpler — it's an in-process join, no
protocol-level aliasing needed.

## Functional Requirements

### Web UI (Calendar view)

- `get_events()` must additionally return events from calendars the current
  user has a `calendar_shares` grant for (any permission level — read
  includes read), tagged with enough owner/calendar info for the frontend to
  visually distinguish "mine" vs "shared by {owner}" (e.g. a distinct
  color/badge, similar to how markdown-sourced todos are already
  distinguished from CalDAV-native ones).
- `update_event` / `delete_event` / `complete_todo_item` (and any other
  event-mutating `#[server]` fn) must accept operating on a shared
  calendar's events when the current user holds a `read-write` grant for
  that specific `(owner, calendar)` pair — reject with a clear error for
  `read`-only grants attempting a write.
- Calendar-selection UI (event creation, the calendar chip rows used
  elsewhere in the app) should distinguish shared calendars the user only
  has `read` access to (not offered as a creation target) from `read-write`
  ones (offered, clearly labeled as shared).

### Raw CalDAV

- **Alias naming, decided:** key the synthetic shared-calendar resource off
  `calendar_shares.id` (a stable `INTEGER PRIMARY KEY AUTOINCREMENT`
  surrogate already in the schema), not the owner's calendar display name —
  e.g. `/caldav/{grantee}/shared-{share_id}/`. This survives the owner
  renaming their calendar and keeps the grantee's client's cached
  href/ctag/sync-token valid across renames. Clients don't care about the
  literal path shape — CalDAV discovery is fully dynamic (PROPFIND on the
  principal for `calendar-home-set`, then a `Depth: 1` PROPFIND on that
  collection to enumerate children) — so this naming choice is purely an
  internal implementation detail, invisible to iOS/DAVx⁵/Thunderbird.
- The calendar-home-set PROPFIND for `/caldav/{grantee}/` must additionally
  enumerate `calendar_shares` rows where `grantee_username = {grantee}` and
  synthesize an extra `<cal_href>` per shared calendar using the scheme
  above.
- Every handler currently doing `require_same_user(&auth, &username)` needs a
  second branch: if `auth.username != username`, check whether the calendar
  path segment matches the `shared-{share_id}` alias pattern; if so, look up
  that `calendar_shares` row, confirm `grantee_username == auth.username`,
  and — if the verb is permitted by `permission` (read-only grant →
  GET/PROPFIND/REPORT only; read-write → PUT/DELETE/MOVE too) — operate on
  the *owner's* underlying storage (the row's `owner_username` +
  `calendar_name`) instead of rejecting or using the grantee's own storage.
- `require_calendar_writable` needs the equivalent share-aware branch for the
  write path specifically.
- ETag/sync-token behavior for the aliased resource should be identical to
  the owner's own view of that calendar (same underlying rows), so two
  clients (owner's, grantee's web UI, grantee's CalDAV client) all syncing
  the same calendar via different paths stay consistent.

### Cross-cutting

- This is authorization/session-adjacent logic touching every CalDAV handler
  plus several core event `#[server]` fns — per CLAUDE.md §19 this needs
  explicit human sign-off before merging to `main`, same as a schema
  migration would.
- Both surfaces (web UI, CalDAV) should enforce the exact same permission
  semantics off the same `calendar_shares` row — no separate/divergent
  permission logic between them.

## Open Questions

- Exact visual treatment for shared events in the Calendar view — color
  override, a small owner-name badge, both? Leave to implementation, should
  match existing conventions (calendar color dots, source pills already used
  elsewhere). Not yet implemented — `Event` carries `owner_username`/
  `shared_permission` but no component reads them for display yet.
- Scope: is per-calendar granularity sufficient, or do we eventually want
  per-collection-of-calendars sharing (mirroring how Notes' collections
  group multiple notes under one shared unit)?

## Resolution

Implemented on both fronts as designed.

**Web UI:** `get_events()` now unions in events from any calendar the
current user holds a `calendar_shares` grant for (via a new
`list_shared_calendar_events` helper), tagging each with `owner_username`/
`shared_permission` on the `Event` type. `update_event`, `delete_event`,
and `complete_todo_item` resolve a `read-write` grant to the real owner's
username (via a new `resolve_event_write_owner` helper) before scoping
their reads/writes — a `read`-only grant or no grant falls through to the
existing same-user-only behavior unchanged. Verified end-to-end over real
HTTP (not just unit tests): logged in as a grantee with no events of their
own, `get_events` returned the owner's shared event tagged correctly, and
`delete_event` successfully removed it from the owner's data.

**Raw CalDAV:** shared calendars appear as a `shared-{share_id}` alias in
the grantee's own calendar-home-set PROPFIND response, keyed by the stable
`calendar_shares.id` (not the owner's calendar name, so a rename doesn't
break a client's cached sync state). Every CalDAV read/write handler
(GET/PROPFIND/REPORT/PUT/DELETE) resolves that alias back to the owner's
real storage via `resolve_calendar_identity`, gated by the grant's
`permission`; `caldav_report`'s delegation to `caldav_sync_collection` for
sync-collection REPORTs needed splitting into separate "which storage to
query" vs. "which href to advertise to the client" identities, since a
grantee's client must keep seeing its own alias path in hrefs even though
the underlying storage calls resolve to the owner. Verified with a
dedicated end-to-end test suite (`tests/test_caldav_sharing.py`): discovery
via PROPFIND, a full read-write PUT/GET/DELETE round-trip, a read-only
grantee's write correctly rejected with 403, and a third user attempting
another grantee's share-id directly also rejected with 403.

Calendar-collection-level management operations (MKCOL, whole-calendar
DELETE/COPY/MOVE, PROPPATCH, LOCK/UNLOCK) remain owner-only by deliberate
scope cut, per this item's Global Constraints — only discovery and
individual-event CRUD are share-aware.

While investigating the existing (pre-this-item) write-permission
enforcement for shared calendars, confirmed that neither the web UI nor
CalDAV had any cross-user write path at all before this change — so the
question of whether it previously checked `read` vs `read-write` correctly
turned out to be moot; there was no path to check.

## Follow-up (2026-07-20)

The "Open Questions" gap around visual treatment turned out to have a more
basic hole underneath it: the shared calendar never appeared as its own
entry in the web UI's left sidebar at all — `get_shared_calendars()` was
still only ever called from Settings → Sharing (informational list),
matching the exact pattern this item already diagnosed for `get_events()`
before it was fixed. A user shared a calendar read/write and, correctly,
found no new item in their sidebar to toggle or recognise it by, even
though the shared events themselves were merging into `get_events()` as
designed.

Fixed in `filter_sidebar.rs`: the "Calendars" panel now also fetches
`get_shared_calendars()` and renders each as an additional `CalendarPill`
labeled `"{display_name} (from {owner_username})"`, with no edit button
(grantees don't own the calendar's metadata). The toggle keys off
`calendar_name` unprefixed — the same field `Event::calendar` already
carries for shared events (see `list_shared_calendar_events`) — so hiding
the pill also hides that calendar's events wherever `HiddenCalsCtx` is
already consulted (e.g. the Focus view), with no new plumbing needed.

Known limitation carried over, not fixed here: `HiddenCalsCtx` keys purely
by calendar name with no owner scoping, so a shared calendar and one of the
grantee's own calendars that happen to share the same name will toggle
together. Pre-existing behavior of the underlying event-tagging scheme, not
introduced by this fix.

## Follow-up 2 (2026-07-20): owner-side revocation was effectively hidden

`revoke_calendar_share` and `get_calendar_shares` already existed and
worked correctly, but the only place that called them (Settings → Sharing's
"Current access" section) only rendered once the owner picked a calendar in
the *same* dropdown used to send a brand-new invite — a single-calendar
owner (the common case) had no reason to interact with that dropdown except
to send an invite, so the existing grants (and the only Revoke button) were
never visible in practice. Reported as "the sharer cannot revoke the
sharing."

Fixed in `components/settings/sharing.rs`: replaced the dropdown-gated
"Current access" section with an always-visible "Your shared calendars"
section that aggregates `get_calendar_shares()` across every calendar the
owner has (loaded via an `Effect` off the existing `cals_res` resource, one
call per owned calendar), each row showing the calendar name, the grantee,
the permission, and a Revoke button — independent of whatever's selected in
the invite form above it. No new `#[server]` functions were added; this
only changes how the existing ones are called from the component.

## Follow-up 3 (2026-07-20): invitation acceptance had no identity checks

While investigating a real self-share row found in `data-3061/tidl.db`
(`owner_username == grantee_username`, both `alice@example.com`),
tracing `accept_handler` (`tidl-server/src/sharing.rs`) turned up two
missing checks on `accept_share_invitation`:

- Nothing rejected the calendar owner accepting their own invitation —
  self-accepting duplicates every one of that calendar's events in
  `get_events()` (its owned-rows query and its shared-rows query both
  resolve to the same `(username, calendar)`, and the union has no dedup),
  and duplicates the calendar itself in the sidebar (the pill added in
  Follow-up 1 renders alongside the real owned pill). Not DB corruption —
  `PRAGMA integrity_check` was clean, the row satisfies every constraint —
  just wrong application-level data.
- More importantly, `accept_handler` only checked that *some* valid session
  existed — it never checked that the session's account is the one the
  invitation named. `invitee_email` was recorded on `share_invitations` but
  never read back at accept time. Any authenticated user who obtained or
  guessed the token could accept an invitation meant for someone else and
  end up holding the grant themselves.

Fixed in `storage::accept_share_invitation` (now takes `grantee_email` in
addition to `grantee_username`): rejects when `owner_username ==
grantee_username` (self-share), and rejects when `grantee_email` doesn't
case-insensitively match the invitation's `invitee_email`. Either rejection
leaves the invitation `pending` (not consumed), so a legitimate accept can
still happen afterward. `accept_handler` now resolves the session's
username to its account email via `state.users.find_active_by_username`
before calling accept, matching the `username`/`email` split already
established by the `users` table (CalDAV paths use `username`; the invite
was addressed to an `email`). Covered by four new tests in
`storage::tests`: normal accept, case-insensitive email match, self-share
rejection, and wrong-email rejection. The stray self-share row found during
investigation was independently revoked (via the Follow-up 2 UI) before
this fix landed, so no manual DB cleanup was needed here.

## Follow-up 4 (2026-07-20): the Calendar page's *own* sidebar/filter never got the fix

With a legitimate grant now in place (alice → bob, calendar `shared`,
read-write, confirmed valid in `data-3061/tidl.db`), the pill was still
missing from the sidebar **and** the shared calendar's events were missing
from the Calendar grid — even after a hard refresh, which ruled out the
stale-WASM-bundle theory from Follow-up 1's verification.

Root cause: Follow-up 1 only edited `components/filter_sidebar.rs`, which
is the sidebar for the Focus/Todos/Tickler pages. The actual **Calendar**
page (`components/calendar/mod.rs`) has always had its own, entirely
separate calendar-list mechanism — `CalendarsCtx` (populated only from
`get_calendars()`, owned calendars only) driving both `CalendarList`
(`components/calendar/calendar_list.rs`, the real Calendar-page sidebar)
*and* `events_memo`'s visibility filter. That filter is an include-list:
`visible = names of calendars in CalendarsCtx marked visible`, and events
are dropped unless `visible.contains(&e.calendar)`. Since a shared
calendar never entered `CalendarsCtx`, its events were fetched correctly by
`get_events()` (unchanged, pre-existing, still correct) and then silently
filtered back out client-side before ever reaching the grid. Editing
`filter_sidebar.rs` alone could never have fixed either symptom — wrong
component entirely.

(Focus/Todos/Tickler were unaffected by this specific filter because
`focus.rs` uses an exclude-list keyed off `HiddenCalsCtx`, which defaults
to empty/nothing-hidden — a calendar's events show through by default
whether or not that calendar has a pill.)

Fixed by adding a `SharedCalendarsCtx = RwSignal<Vec<(SharedCalendar,
bool)>>` (`app.rs`), populated in `calendar/mod.rs` the same way
`CalendarsCtx` already is (an Effect off a `get_shared_calendars()`
resource, preserving each entry's visibility flag across refetches, keyed
on `(owner_username, calendar_name)` since shared calendars have no
`CalendarInfo.id`). `events_memo`'s `visible` set now also includes visible
shared-calendar names. `CalendarList` takes a new `shared_calendars` prop
and renders a second, unstyled-differently list of pills labeled `"{name}
(from {owner})"` — no edit button, matching the read-only-metadata pattern
from Follow-up 1's `filter_sidebar.rs` pills. Kept as a separate context
type rather than merging into `CalendarsCtx` as synthetic `CalendarInfo`
rows, since `CalendarList`'s existing `on_edit` would otherwise offer to
edit a calendar the grantee doesn't own.

`filter_sidebar.rs` (Follow-up 1) is unaffected and still correct for its
own pages — this just closes the parallel gap in the Calendar page's
independent implementation.

## Follow-up 5 (2026-07-20): the event-creation FR (line 89-92) is still open

While unifying the calendar-list sidebar (see
[53](53-unified-calendar-list-live-updates.md)), the calendar-creation
pickers in `event_form.rs` and `todo_list.rs`'s `AddTodoModal` finally had
read/read-write permission data available to implement "offer read-write
shared calendars as event-creation targets, exclude read-only ones" — but
turned out to still be blocked on something this item didn't check at the
time: `create_event`/`create_todo_item` always `INSERT` under the *current
session's own* username with no owner-resolution at all (unlike edits to an
*existing* shared event, which already resolve cross-owner via
`resolve_event_write_owner`). Offering a shared calendar as a creation
target today would silently create a same-named phantom calendar under the
wrong user rather than write to the actual shared one. Both pickers were
kept owned-only (unchanged from before #53) rather than ship a broken
option. This FR needs real cross-owner support added to `create_event` and
`create_todo_item` first — a bigger, auth-adjacent change in its own right
(same CLAUDE.md §19 category as the rest of this item) — before it can
close. Not done as part of #53.

# Let a grantee leave a shared calendar

A user who has accepted a share invite has no way to remove that calendar
from their own view. The owner can revoke access (Settings → Sharing →
"Your shared calendars" → Revoke), but the grantee's own "Shared with me"
list is read-only — surfaced while fixing an unrelated bug in the remote
CalDAV source edit flow.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — this removes a grant
  created by that mechanism; no new sharing concept is introduced.
- [41 - Make calendar shares actually usable](41-caldav-share-exposure.md) —
  a shared calendar is now a real, usable calendar for the grantee (events,
  writes), which is what makes "I don't want this synced to me anymore"
  worth acting on.

## Context / Background

`revoke_calendar_share` (`api/calendars.rs`) deletes a `calendar_shares` row
scoped to `WHERE owner_username = <current user>` — only the owner can call
it. `SharingPanel` (`components/settings/sharing.rs`) renders "Shared with
me" (`get_shared_calendars`) as a plain list with no action button at all.
There is no code path, UI or server-fn, for the grantee side.

## Functional Requirements

- A new server fn deletes the `calendar_shares` row scoped to
  `WHERE grantee_username = <current user> AND owner_username = ? AND
  calendar_name = ?` — the mirror image of `revoke_calendar_share`'s
  owner-scoped delete.
- "Shared with me" rows in `SharingPanel` get a button (e.g. "Remove") that
  calls it and refreshes the list.
- Notify both sides over SSE on removal, same as `revoke_calendar_share`
  does today (owner's "Your shared calendars" list and the grantee's own
  view both need to drop it live).

## Resolution

Added `leave_shared_calendar(owner_username, calendar_name)` server fn in
`api/calendars.rs`, scoped by `WHERE grantee_username = <current user>` —
the mirror of `revoke_calendar_share`. Wired a "Remove" button into
`SharingPanel`'s "Shared with me" rows that calls it and refreshes
`shared_with_me`. Reused the existing `notify_change` calls for both the
grantee and owner usernames so both sides' lists update live, matching
`revoke_calendar_share`'s notification pattern.

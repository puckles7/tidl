# Let a grantee customize and (write-)clear a shared calendar

A calendar shared with the current user had no edit affordance at all in the
sidebar — surfaced right after adding [55 - Let a grantee leave a shared
calendar](55-grantee-leave-shared-calendar.md), in the same conversation.
The grantee should be able to set their own display color for it, and —
if their grant is `read-write` — clear its events, with a clear warning that
this deletes the owner's real events, not just their own view.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — operates on grants
  created by that mechanism.
- [41 - Make calendar shares actually usable](41-caldav-share-exposure.md) —
  a shared calendar is a real, usable calendar for the grantee (events,
  writes) by this point, which is what "clear its events" acts on.

## Context / Background

`CalendarListPanel` only wired an edit button for owned entries
(`entry.to_calendar_info()` returns `None` for shared ones). `CalendarInfo`,
and the modal built around it, assume a calendar the current user owns —
name, webcal URL, and deletion all mutate the owner's actual `calendars`
row, which a grantee has no business touching. Reusing that modal as-is for
shared calendars wasn't an option.

## Functional Requirements

- Shared calendar color is a **per-viewer override**, not a change to the
  owner's calendar — `calendar_shares.grantee_color` (new nullable column,
  migration `20260721000002_shared_calendar_grantee_color.sql`).
  `get_shared_calendars()` returns `COALESCE(grantee_color, calendars.color,
  fallback)`. New `set_shared_calendar_color(owner_username, calendar_name,
  color)` server fn, scoped to `grantee_username = current user`.
- "Clear all events" is only offered when the grant's `permission` is
  `read-write`. New `clear_shared_calendar(owner_username, calendar_name)`
  re-checks that permission server-side (never trust the client-side gate
  alone), then deletes every event in that calendar — the same
  tombstone/sync-token-bump dance as `clear_personal_calendar` /
  `delete_calendar_api`, keyed to the *owner's* username so CalDAV clients
  (including the owner's own) see the deletion. Every grantee of that
  calendar is notified, not just the one who triggered it — mirrors
  `delete_calendar_api`'s notification of all grantees.
- A confirmation dialog spells out that this deletes events for the owner
  and every other grantee, not just the current user's own view.
- Reuses `CalendarEditModal` rather than a bespoke modal: a new `shared:
  Option<SharedCalendarEditInfo>` prop, mutually exclusive with the existing
  `cal: Option<CalendarInfo>`. When `shared` is set, the Name field is shown
  but disabled with a "Shared calendars can only be renamed by the owner"
  hint, Share is shown but disabled (reshare is #57's problem), the webcal
  URL field stays hidden (same `is_edit && !is_webcal` check already hid it
  for non-webcal owned edits), and the danger-button slot shows "Clear all
  events" only under a `read-write` grant.

## Open Questions

- Reshare (letting a grantee re-extend the calendar to someone else) is
  explicitly out of scope here — tracked separately as [57 - Reshare a
  calendar received via a share](57-reshare-shared-calendar.md), since the
  permission-cascade questions (can a read-only grantee reshare at all? does
  a reshared grant chain back to the original owner or create an
  independent one? does the owner get any visibility into it?) need their
  own design pass.

## Resolution

Implemented as scoped above: `grantee_color` column + `COALESCE` in
`get_shared_calendars()`, `set_shared_calendar_color` and
`clear_shared_calendar` server fns in `api/calendars.rs` (the latter's core
logic factored into a testable `clear_shared_calendar_tx`, covering the
permission gate and the tombstone/token-bump transaction). Deviated from the
initial pass, which added a separate `SharedCalendarEditModal`: that was
folded back into `CalendarEditModal` itself (a `shared` prop alongside
`cal`) so owned and shared calendars share one dialog implementation.
`CalendarListPanel` now tracks a single `EditTarget` enum
(`Create`/`Owned`/`Shared`) and renders one `<CalendarEditModal>` call site
regardless of which kind of entry was clicked.

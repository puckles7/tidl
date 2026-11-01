# Calendar sharing

Allow users to share their calendars with other users on the same server. Sharing is initiated by sending an email invitation to the invitee's address. This story also introduces the email client infrastructure (SMTP) that other features (e.g. notifications) will depend on.

## Background

CalDAV defines calendar sharing via the `caldav-sharing` extension (draft-pot-caldav-sharing) and the `DAV:share-resource` POST action. Access control builds on WebDAV ACLs (RFC 3744). On the UI side the user picks which calendars to share and with whom; on the server side the invited user gets a new principal-accessible calendar collection.

## Dependencies

- [14 - Settings dialog tabs](14-settings-tabs.md) — SMTP settings need their own tab; tabs should land first so the settings dialog has room

## Functional Requirements

### Sharing flow
- Owner can share any of their calendars with one or more other users
- Invitation is sent by email to the invitee's address
- Invitee can accept or decline (at minimum via a link in the email; ideally also from the UI)
- Accepted share appears as a read-only (or read-write, depending on permission granted) calendar in the invitee's calendar list
- Owner can revoke access at any time from the calendar's settings

### Permissions
- At minimum: `read` and `read-write` access levels
- Owner always retains full control

### Email client integration (shared infrastructure)
- Integrate an SMTP client (e.g. `lettre`) for sending outbound email
- Configuration exposed in the Settings dialog (dedicated tab, see #14):
  - SMTP host and port
  - TLS mode: none / STARTTLS / TLS
  - Username and password
  - From address / display name
  - Optional: test-send button
- Configuration stored securely (password not logged or exposed in API responses)
- Email client abstracted behind a `Mailer` trait so transport (SMTP, stub for tests, future SendGrid etc.) is swappable

### CalDAV alignment
- Model sharing state using `caldav-sharing` semantics where practical
- Store share grants in the database; reflect them as WebDAV ACL entries so CalDAV clients that understand ACLs see the correct permissions

## Open Questions

- Do we support sharing with users outside the server (external email addresses without a local account)?
- Should the invitee need an existing account, or does accepting an invitation create one (ties into user management)?
- How do we handle revocation when the invitee's CalDAV client has already synced the shared calendar?
- Store SMTP password encrypted at rest or rely on OS/deployment-level secrets management?

## Known Gap (found 2026-07-13, while investigating #41; closed 2026-07-20, see #41's Follow-up)

Despite being marked `done`, this item only shipped the invite/accept/decline
flow and the `calendar_shares` grant table — the actual "accepted share
appears as a calendar in the invitee's calendar list" requirement (line 19
above) was never implemented anywhere. Confirmed by tracing the code:

- `get_shared_calendars()` (`api/calendars.rs`) — the only function that
  reads "what's shared with me" — is used exclusively by the Settings →
  Sharing tab (`components/settings/sharing.rs`) to render an informational
  list. It's never called from the Calendar view.
- `get_events()` (`api/events.rs`), which powers the entire Calendar view, is
  hard-scoped to `WHERE e.username = ?` with no join against
  `calendar_shares` — a shared calendar's events never render for the
  grantee anywhere in the web UI.
- `update_event` / `delete_event` / `complete_todo_item` are similarly
  scoped to `username = current_user` with no cross-user path at all,
  regardless of the grant's `permission` value — write access for a
  grantee doesn't exist yet, not even in the web UI.
- The CalDAV-protocol side has the same gap:
  `require_same_user` in `caldav_handlers.rs` rejects any identity that
  doesn't match the calendar-owner, with no exception for a share grant —
  so raw CalDAV clients (iOS Calendar, DAVx⁵, Thunderbird) can't see a
  shared calendar either.

In short: sharing today grants a row in `calendar_shares` and lets you see
*that* it exists, but nothing actually consumes that grant to show or edit
the owner's calendar anywhere. [41 - Expose calendar shares over raw
CalDAV](41-caldav-share-exposure.md) closes this gap on both fronts (web UI
Calendar view + CalDAV protocol layer).

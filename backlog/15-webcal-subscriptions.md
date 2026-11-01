# Webcal URL subscriptions

Allow users to subscribe to external calendar feeds via `webcal://` URLs. The
server fetches the remote `.ics` feed periodically and surfaces the events as a
read-only calendar. This is the standard mechanism used by school portals,
sports leagues, public holiday providers, and similar services.

## Background

`webcal://` is a URI scheme alias for `http://` / `https://` that signals "this
is an iCalendar feed". CalDAV clients and calendar apps treat it as a
subscribe-and-sync operation: fetch the URL, parse the iCalendar data, and
refresh on a schedule. The server should own the fetch-and-cache cycle so that
all CalDAV clients connected to tidl see the same external events without each
client polling independently.

Example feed URL:
```
webcal://api.socialschools.eu/api/v1/icalfeed/?schoolId=2344&roleTypeId=1&userId=7403d31b-7523-4e19-908a-70033ec4e044&hash=MilRVoWKQlGWK14cxuP5KZI9U27x0JYJbw3egbaRgY
```

## Design

A webcal subscription is **a property of a calendar**, not a separate concept.
A calendar can optionally have a sync URL attached to it; when it does, the
server refreshes events from that URL periodically and the calendar becomes
read-only.

### UI

The **edit-calendar dialog** gains an optional "Sync from URL" field:
- If a URL is entered, the calendar becomes a subscribed (read-only) calendar.
- If the URL is cleared, the calendar reverts to a normal local calendar
  (keeping cached events, or optionally clearing them).
- No separate Sources tab or section is needed — this supersedes #11.

### Read-only indication

Subscribed calendars are read-only, the same as calendars shared with read
permission by another user. Both cases should be indicated in the calendar
sidebar (e.g. a lock icon or muted rename/delete controls). This is currently
not implemented for shared calendars either — both gaps should be addressed
together.

## Dependencies

- [14 - Settings dialog tabs](14-settings-tabs.md) ✓ — edit-calendar dialog
  already exists; this extends it.

*(#11 is no longer a dependency — the Sources tab approach was dropped in
favour of attaching the webcal URL directly to the calendar.)*

## Functional Requirements

### Adding a subscription
- User opens the edit-calendar dialog and pastes a `webcal://` (or `http://` /
  `https://`) URL into the "Sync from URL" field.
- Server validates the URL is reachable and returns parseable iCalendar data
  before saving.
- Once a URL is saved the calendar is marked read-only.

### Periodic sync
- Server fetches each subscribed URL on a configurable interval (default: every hour).
- Respects `ETag` and `Last-Modified` response headers to avoid redundant re-parsing.
- Failed fetches are retried with backoff; persistent failures surface a warning in the UI.
- Events from the feed are stored read-only — local edits are not allowed.

### CalDAV exposure
- The subscribed calendar appears as a standard calendar collection to CalDAV clients.
- Marked read-only (no `write` privilege in WebDAV ACLs).
- Event ETags derived from the remote feed so CalDAV clients sync correctly when the feed changes.

### URL handling
- Rewrite `webcal://` → `https://` (fallback to `http://`) before fetching.
- Support feeds behind redirects (follow up to N hops).
- Handle feeds with self-signed or missing TLS certs — configurable strict/lenient mode.

## Decisions

- **Sync interval**: global setting, not per-subscription.
- **Manual refresh**: yes — expose a "refresh now" action in the UI (e.g. in the edit-calendar dialog or calendar context menu).
- **Feed sync strategy**: full replace on each successful fetch (wipe + re-insert by UID). ETag/Last-Modified at the HTTP layer already avoids redundant re-parsing, so delta diffing is not needed.
- **Sharing scope**: subscriptions are owned by the user who created them but are shareable via the existing calendar-sharing mechanism (#13) — read-only only, capped at read permission since the calendar itself is read-only. No separate concept needed.
- **URL detach**: when a sync URL is removed, delete all cached events from that feed.

## Resolution

Implemented in full. Key files:

- **Migration**: `migrations/20240118000000_webcal_sync.sql` — adds `sync_url`, `sync_etag`, `sync_last_modified`, `last_synced_at`, `sync_error` columns to `calendars`.
- **Storage**: `Calendar` struct extended; `set_calendar_sync_url` and `update_webcal_sync_status` added to `CalendarStore` trait and `Storage` impl.
- **Background sync**: `crates/tidl-server/src/webcal_sync.rs` — hourly polling loop; full-replace strategy with HTTP ETag/Last-Modified conditional fetching; webcal:// URL rewriting; up to 10 redirect hops.
- **API**: `set_calendar_sync_url_api` (validates URL before saving, deletes events on detach) and `refresh_webcal_calendar_api` (manual one-shot sync) in `tidl-web/src/api.rs`.
- **UI**: `CalendarList` in `calendar.rs` — ⟲ subscribe button; subscribe form; 🔒 lock indicator for read-only feeds; webcal edit dialog with sync URL, last-synced status, sync error display, and "Refresh now" button; "Unsubscribe" replaces delete for webcal calendars.

No deviations from spec.

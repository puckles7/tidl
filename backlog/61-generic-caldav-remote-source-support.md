# Evaluate generic CalDAV server support for remote sync (beyond tidl-to-tidl)

The "Remote CalDAV source" sync feature (Settings → Sharing) is currently documented and implemented as tidl-to-tidl only: the UI hint says "Sync with another tidl instance," and the client hardcodes tidl's own URL convention. Evaluate whether/how to extend it to work against arbitrary standards-compliant CalDAV servers (Nextcloud, Radicale, Fastmail, iCloud, Google Calendar via CalDAV), since the wire protocol operations already used are standard CalDAV (HTTP Basic auth, PROPFIND, RFC 6578 `sync-collection` REPORT, GET/PUT/DELETE with ETag concurrency).

## Context / Background

Investigated 2026-07-23 while reviewing the sync UI copy in `crates/tidl-web/src/components/settings/remote_sources.rs:314`. Findings:

- `crates/tidl-server/src/caldav_client.rs:65-71` hardcodes calendar/event URLs as `{base_url}/caldav/{username}/{calendar}/...` — tidl's own routing convention, not derived from server discovery.
- `crates/tidl-web/src/api/calendars.rs:1674-1725` (`discover_remote_calendars`) PROPFINDs that same hardcoded `/caldav/{username}/` path rather than following the standard CalDAV discovery chain (`.well-known/caldav` → `current-user-principal` → `calendar-home-set`).
- No other tidl-specific behavior was found beyond the URL layout — auth is plain HTTP Basic with `username:app_token`, and XML parsing (`parse_calendar_slugs_from_propfind`) is generic enough to tolerate `D:` or unprefixed namespaces.

So the gap is specifically the missing discovery chain, not the wire protocol itself.

## Functional Requirements

- Implement the standard discovery chain: `.well-known/caldav` redirect, `current-user-principal` PROPFIND, `calendar-home-set` PROPFIND, then enumerate calendar collections under the home set.
- Stop assuming the remote path contains `{username}` — treat discovered calendar hrefs as opaque URLs.
- Confirm RFC 6578 `sync-collection` REPORT behavior and ETag/If-Match semantics against at least one non-tidl server (e.g. Radicale or Nextcloud) — some servers have quirks here (partial sync-token support, different conflict responses).

## Open Questions

- Auth: several major providers (Google, iCloud) don't accept a bare Basic `username:app_password` for CalDAV the same way tidl does — iCloud/Google require provider-specific app-password setup outside tidl's control. Is supporting those in scope, or is this limited to self-hosted CalDAV servers (Nextcloud, Radicale, Baïkal)?
- Should the UI hint/copy change to "Sync with a CalDAV server" once discovery lands, or stay tidl-specific if only self-hosted servers are realistically supported?
- Does the existing "Discover" button UX (single dropdown of calendar slugs) still make sense once hrefs are opaque URLs instead of readable slugs?

## Why dropped

Not worth the effort right now — the discovery-chain implementation and per-provider auth quirks (especially Google/iCloud) are more work than the payoff justifies at this time. Revisit if demand for non-tidl remote sources shows up.

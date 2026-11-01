# Work Calendar Integration

Integrate a corporate work calendar into tidl so all calendars are visible in one place, from any device, without depending on corporate infrastructure directly.

## Dependencies

- [03 - Multi-Calendar Support](03-multi-calendar.md) — the work calendar is synced into a dedicated tidl calendar owned by the user

## Context

tidl runs self-hosted. The work machine is a Windows laptop behind a corporate firewall with draconian inbound restrictions, but outbound HTTPS (port 443) is allowed. A pre-compiled binary can be run on the work machine. The work machine is **not always on**.

Work calendar is almost certainly Exchange / Office 365 (EWS or Microsoft Graph API), or occasionally Google Workspace.

## Core Problem

Corporate firewalls typically:
- Block all **inbound** connections to work machines
- Allow **outbound** HTTPS (port 443) — this is the key
- May do TLS inspection, but rarely block standard HTTPS to arbitrary hosts

A secondary concern is data governance: **work calendar data should not be pulled from the work environment by an external system**. Instead, the work machine should decide what to push outward. This rules out architectures where the personal server holds credentials to poll a work calendar source.

## Proposed Architecture

Two tidl instances: one on the work machine (local, not always on), one on the personal server (always on, internet-facing). The work machine initiates all sync in both directions over outbound HTTPS.

```
                Work machine (Windows laptop)
                ┌──────────────────────────────┐
                │  tidl (local instance)       │
                │  ├── personal calendar       │◄─── synced FROM personal server
                │  └── work calendar           │◄─── Outlook COM (local read)
                └──────────────────────────────┘
                             │
                             │ outbound HTTPS/443
                             │ work machine initiates both directions
                             ▼
                Personal server (always-on, internet-facing)
                ┌──────────────────────────────┐
                │  tidl (primary instance)     │
                │  ├── personal calendar       │◄─── phone, personal laptop
                │  └── work calendar mirror   │  (stale when laptop is off)
                └──────────────────────────────┘
```

Sync flows (work machine initiates all connections — both directions are outbound HTTPS from the work machine):
- **Work → personal server**: work machine CalDAV-pushes work events to personal server's work-mirror calendar (PUT/DELETE).
- **Personal → work machine**: work machine CalDAV-pulls personal events from personal server (REPORT/GET). Personal server never connects to work machine.
- **When work machine is off**: personal server holds a stale-but-present copy of work events; personal calendar is unaffected.

The local tidl instance on the work machine runs on a local port (e.g. `localhost:5000`) and is accessed via browser on the work machine itself.

## Calendar Source Options

| Source | How | Pros | Cons |
|--------|-----|------|------|
| **Outlook COM** | Talk to the local Outlook process | No extra auth, works offline, no API registration | Outlook must be running |
| **Microsoft Graph API** | OAuth2 to `graph.microsoft.com` | Clean, doesn't need Outlook open | Requires app registration in tenant (IT involvement possible) |
| **EWS (Exchange Web Services)** | SOAP to Exchange server | Works on-prem, no app registration | Legacy, on-prem only, verbose |

**Chosen approach: Outlook COM.** Graph API access is not permitted by IT policy. Outlook COM requires no app registration and reads whatever Outlook has already synced locally.

## What Lives Where

- Personal server: primary instance, always on, holds personal calendar (authoritative) and a work calendar mirror (read-only, stale when laptop off)
- Work machine: local instance, holds personal calendar (synced copy) and work calendar (authoritative, read from Outlook)
- Phone and personal laptop connect only to the personal server
- Work machine browser connects to `localhost`

## Requirements for tidl

- Must run as a standalone executable on Windows (no installer, no admin rights required — ideally a single binary)
- Must be able to bind a local port (verify via PowerShell `[System.Net.HttpListener]` test before assuming this works)
- Personal server must run behind a reverse proxy (Caddy/nginx) serving HTTPS on port 443
- The work calendar mirror on the personal server should be write-locked: only the work machine credential can write to it; other clients are read-only
- tidl needs a **CalDAV client** capability: the work machine instance acts as a CalDAV client to the personal server for both sync directions (push work events, pull personal events). The personal server needs no new code — it just sees authenticated CalDAV clients.
- Sync interval: 5–15 minutes is sufficient

## What the Work Machine tidl Instance Needs to Do

1. Read work calendar events from Outlook COM for a rolling window (e.g. −30 days to +180 days)
2. Diff against its local work calendar; apply changes locally
3. Push work calendar changes to the personal server's work-mirror calendar via CalDAV
4. Pull personal calendar changes from the personal server into its local personal calendar

Steps 3 and 4 use CalDAV (the work machine instance acts as a CalDAV client). The personal server needs no new code — it sees the work machine as two authenticated clients: one with write access to the work-mirror calendar, one with read access to the personal calendar. tidl needs a CalDAV client implementation to enable this.


## Risk

If the corporate network does deep packet inspection and blocks connections to unknown hosts, traffic may need to route through a trusted front (e.g. Cloudflare). Rare outside finance/government — worth trying plain HTTPS first.

## Resolution

Implemented the server-side CalDAV sync infrastructure that enables this architecture:

- `crates/tidl-server/src/caldav_client.rs` — CalDAV client with `sync-collection` REPORT (RFC 6578), GET, PUT, and DELETE; HTTP Basic auth with app tokens.
- `crates/tidl-server/src/remote_sync.rs` — background sync loop (5-minute interval) with pull (delta sync via sync token) and push (local changes to remote); notifies connected browsers via SSE after a pull brings in changes.
- `migrations/20240115000000_remote_caldav_sources.sql` — per-user table of remote CalDAV sources with encrypted token storage, sync direction, and sync token tracking.
- `calendars.read_only` + `sync_user` columns (migration 20240104) enforced in CalDAV handlers — write-lock any calendar so only the designated sync user can write to it; other users (including shared access) are blocked.
- API server functions: `list_remote_sources`, `add_remote_source`, `delete_remote_source`.
- Settings UI: "Remote Sources" tab with full add/list/remove flow.

**Deviations from spec:** Outlook COM integration (reading the local Outlook process on Windows) is out of scope for the tidl server binary; it would require a separate Windows-only tool or script that pushes events into the local tidl CalDAV endpoint. The server-side sync infrastructure is complete and works for any CalDAV source, including tidl-to-tidl sync.

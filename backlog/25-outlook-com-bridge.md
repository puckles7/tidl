# Outlook COM Bridge

A lightweight script that runs on the Windows work machine, reads the local Outlook calendar via COM automation, and pushes events to the local tidl CalDAV endpoint — completing the work calendar sync loop defined in backlog #1.

## Dependencies

- [01 - Work Calendar Integration](01-work-calendar-integration.md) — server-side CalDAV sync infrastructure must be in place; the bridge acts as a CalDAV client to `localhost`

## Background

Backlog #1 built all the server-side plumbing: a CalDAV client, background sync loop, remote sources table, and write-lock enforcement. What it intentionally left out was the Windows-side piece: reading the corporate Outlook calendar from the local Outlook COM object and feeding it into a local tidl instance.

The work machine environment is:
- Windows laptop behind a corporate firewall
- Outbound HTTPS on 443 allowed; no inbound connections
- IT policy prevents Microsoft Graph API app registration
- Outlook is the mail/calendar client and keeps a local cache
- No admin rights assumed; no installer acceptable

The bridge must run without installation and without admin rights.

## Architecture

```
Outlook process (local)
      │  COM automation
      ▼
  bridge script
      │  CalDAV PUT/DELETE (HTTP to localhost)
      ▼
tidl (localhost:5000)
      │  remote_sync.rs (every 5 min, outbound HTTPS)
      ▼
personal server (tidl primary, port 443)
```

The bridge is a standalone script scheduled via Windows Task Scheduler to run every 5–15 minutes. It:
1. Connects to the running Outlook COM object
2. Reads calendar items in a rolling window (−30 days to +180 days)
3. Diffs against the local tidl CalDAV endpoint
4. PUTs new/changed events, DELETEs removed ones

## Tech Options

| Option      | COM access                    | HTTP            | Install needed | Single file | Notes |
|-------------|-------------------------------|-----------------|----------------|-------------|-------|
| PowerShell  | `New-Object -ComObject Outlook.Application` | `Invoke-RestMethod` | None (built-in) | Yes (`.ps1`) | Scheduled with Task Scheduler; `-ExecutionPolicy Bypass` flag avoids system policy |
| Python      | `win32com.client`             | `requests`      | Python + pywin32 | No          | Heavier footprint |
| Rust binary | `windows-rs` COM interop      | `reqwest`       | None (single `.exe`) | Yes (`.exe`) | Verbose COM interop; harder to debug; cross-compile from Linux is tricky |

**Chosen approach: PowerShell.** Zero install footprint, native COM support, HTTP via `Invoke-RestMethod`, trivially scheduled. Script can be signed or run with `-ExecutionPolicy Bypass` at the Task Scheduler level without changing system policy.

## Functional Requirements

- Read Outlook `CalendarItems` for the rolling window using `Items.Restrict` with a time filter
- Map Outlook fields to iCalendar (VEVENT): `Subject`, `Start`, `End`, `Location`, `Body`, `GlobalAppointmentID` (as UID), `LastModificationTime` (as DTSTAMP/LAST-MODIFIED)
- Recurring events: expand occurrences within the window (Outlook's `IncludeRecurrences` flag + `Restrict`)
- Build a minimal but valid `.ics` payload per event
- PUT to `http://localhost:{port}/dav/{user}/{calendar-slug}/{uid}.ics` with Basic auth (app token)
- Fetch existing UIDs from the local tidl CalDAV endpoint (PROPFIND) and DELETE any that are no longer in Outlook
- Skip gracefully if Outlook is not running (catch COM error, exit 0)
- Log to a local file (`%APPDATA%\tidl-bridge\bridge.log`) with timestamps, capped at ~1 MB (rotate on startup)

## Configuration

A sidecar config file (`bridge.conf`) in the same directory as the script:

```
TIDL_URL=http://localhost:5000
TIDL_USER=alice
TIDL_TOKEN=tidl_<hex>
TIDL_CALENDAR=work
OUTLOOK_CALENDAR=Calendar
WINDOW_PAST_DAYS=30
WINDOW_FUTURE_DAYS=180
```

## Scheduling

Task Scheduler action:
```
Program: powershell.exe
Arguments: -ExecutionPolicy Bypass -NonInteractive -WindowStyle Hidden -File "C:\Users\<user>\tidl-bridge\bridge.ps1"
Trigger: every 10 minutes, starting at logon
```

## Resolution

Implemented as `tools/outlook-bridge/bridge.ps1` — a standalone PowerShell 5.1 script with no install footprint. Ships alongside `bridge.conf.example` and `install.md`.

Key implementation details:
- Attaches to a running Outlook process via `GetActiveObject`; exits cleanly if Outlook is not running
- Uses `Items.IncludeRecurrences = $true` + `Items.Restrict` for correct occurrence expansion
- Appends `_YYYYMMDDTHHmmss` to `GlobalAppointmentID` for unique UIDs on recurring occurrences and exceptions
- Full RFC 5545 iCalendar output: TEXT escaping, line folding at 75 octets, all-day `VALUE=DATE` handling
- CalDAV sync via PROPFIND (fetch existing UIDs+ETags), PUT with `If-None-Match`/`If-Match`, DELETE with `If-Match`
- Log rotation at ~1 MB; graceful error handling per event (one failure does not abort the run)

## Open Questions — Resolved

- **`Content-Type`**: tidl PUT handlers require `text/calendar; charset=utf-8`. Used exactly that.
- **All-day events**: tidl stores `DTSTART;VALUE=DATE` / `DTEND;VALUE=DATE` for all-day. Outlook's COM `AllDayEvent` flag is reliable; `Start`/`End` are midnight boundaries. `DTEND` (exclusive) maps directly to Outlook's `End` — no `+1` needed.
- **`GlobalAppointmentID` and recurrences**: stable per series (same for all occurrences). For `olApptOccurrence` (3) and `olApptException` (4), the bridge appends `_YYYYMMDDTHHMMSS` to produce a unique UID per occurrence.

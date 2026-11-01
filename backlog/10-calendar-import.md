# Calendar Import

Allow users to import events from exported calendar files into tidl. The primary
target is Google Calendar exports, but since Google (and virtually every other
calendar app) exports standard `.ics` files, good iCalendar support covers the
whole ecosystem.

## Format support

| App | Export format |
|-----|--------------|
| Google Calendar | `.ics` (iCalendar, RFC 5545) |
| Apple Calendar | `.ics` |
| Microsoft Outlook | `.ics` (also `.csv`, lower priority) |
| Thunderbird | `.ics` |
| Fastmail / Proton Calendar | `.ics` |

**Conclusion:** implement solid `.ics` import first; `.csv` can be added later if
there is demand (Outlook `.csv` is non-standard and lossy).

## Functional Requirements

- Upload an `.ics` file via the web UI (file picker or drag-and-drop).
- Parse all `VEVENT` components in the file, including:
  - Summary, description, location
  - DTSTART / DTEND (date-only all-day events and datetime events)
  - Timezone handling (`TZID`, floating, and UTC)
  - Recurrence rules (`RRULE`, `EXDATE`) — at minimum store the raw rule
  - `VALARM` components
  - Attendees / organiser (store, but no invite sending needed initially)
- Let the user choose which target calendar to import into.
- Show a preview (count of events found) before committing.
- Skip duplicates: if an event `UID` already exists in the target calendar,
  offer to skip or overwrite.
- Report how many events were imported / skipped / failed.

## Technical Decisions

- **Server-side import** via `POST /api/import` — the server parses the `.ics`
  and writes events directly to the store. Avoids sending hundreds of individual
  API calls from the browser and keeps the parsing logic in Rust where the
  `icalendar` crate already lives.
- **Size cap** — enforce a reasonable maximum file size on the upload endpoint
  (e.g. 10 MB) to keep memory usage bounded. Google Calendar exports for a
  typical user are well under 1 MB; 10 MB covers years of heavy usage.

## UI Entry Point

The import function should be accessible from the **Settings modal** (gear icon),
as a dedicated "Import" section. This keeps the main navigation clean and groups
it with other account/data management actions.

## Out of scope (for now)

- Recurring event expansion (store the rule, don't explode instances)
- `.csv` import (Outlook)
- Streaming / chunked import for very large files

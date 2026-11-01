# Move Sources config into Settings

The current "Sources" nav link leads to a separate page (`/sources`) that is
mostly a placeholder. Source configuration belongs alongside other
account/data settings rather than as a top-level navigation item. Move it into
the Settings modal as a dedicated "Sources" tab.

## Scope

Sources here means **knowledge base / zettelkasten input locations** — the
various protocols and paths from which tidl ingests notes, documents, or other
structured content. Examples of source types to be defined:

- Local filesystem paths (Markdown directories, etc.)
- Remote URLs (git repositories, DAV endpoints, custom protocols TBD)
- Any other ingestion protocol defined in future

This is distinct from calendar subscriptions (webcal URLs), which are a
property of an individual calendar and configured via the edit-calendar dialog
(see #15).

## Functional Requirements

- Remove the "Sources" entry from the top navigation bar.
- Add a "Sources" tab inside the Settings modal where users can:
  - View, add, and remove source locations
  - Configure per-source options (protocol, path/URL, refresh interval, etc.)
- The set of supported source protocols is to be defined; the tab provides the
  UI shell that protocol-specific forms will slot into.

## Motivation

- Keeps the nav bar minimal (Calendar, Todos, Tickler are the core views).
- Sources is a one-time setup concern, not a frequently visited destination.
- Consistent with where Import (#10) lives.

## Resolution

- Removed `<li><A href="/sources">` nav link and the `/sources` route from `app.rs`.
- Deleted the standalone `SourcesView` component from `app.rs`.
- Added a **"Markdown Sources"** tab to the Settings modal (`settings.rs`) with the
  same add/remove/list UI, using the settings-native form classes (`form-group`,
  `form-input`, `sharing-row`, `btn-danger-small`) for visual consistency.
- Renamed the existing **"Remote Sources"** tab to **"CalDAV Sync"** (internal key
  `"remote"` unchanged) to free the word "Sources" for its knowledge-base meaning.
- Tab label chosen as "Markdown Sources" because all supported transports (filesystem,
  git, remote URL) ultimately provide markdown files — the label describes the content
  type, not the transport.

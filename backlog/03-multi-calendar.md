# Multi-Calendar Support

Allow each user to create and manage multiple named calendars, with colour coding, per-calendar visibility toggles, and sharing with other users.

## Dependencies

- [02 - User Management](02-user-management.md) — calendars are owned by a user

## Motivation

A single flat calendar is too limiting. Users want to separate concerns (personal, work, family, project-specific) and optionally share individual calendars with others — without exposing everything.

## Calendars

### Creation & Properties

- A user can create any number of calendars
- Each calendar has:
  - **Name** — free text
  - **Colour** — chosen from a palette or via colour picker; a sensible default is assigned on creation
  - **Owner** — the user who created it

### Default Calendar

Every new user gets a default calendar (`Personal`) created automatically on first login.

## UI

### Sidebar (left panel)

- Lists all calendars the user owns or has been shared with them
- Each entry shows a **colour swatch**, the **calendar name**, and a **checkbox**
- Checkbox controls visibility in the main calendar view — checked by default
- Clicking the colour swatch opens a colour picker
- Overflow menu (⋯) per calendar for rename, share, delete

### Calendar View

- Events are rendered in their calendar's colour
- Hidden calendars (unchecked) are not rendered at all — but not deleted

## Sharing

A calendar can be shared with another tidl user by their email address.

### Invite Flow

1. Owner opens the share dialog for a calendar
2. Enters the recipient's email address
3. Chooses permission level: **read-only** or **read-write**
4. Tidl sends an email invite to the recipient
5. Recipient accepts → calendar appears in their sidebar (with the owner's chosen colour as default, overridable)

### Permission Levels

| Level | Can view events | Can add/edit/delete events |
|-------|----------------|---------------------------|
| Read-only | yes | no |
| Read-write | yes | yes |

Ownership (rename, delete, reshare) always stays with the original owner.

### Revoking Access

Owner can remove a shared user at any time. The calendar disappears from the recipient's sidebar immediately.

## CalDAV Representation

- Each calendar maps to a CalDAV collection: `/caldav/<user-slug>/<calendar-slug>/`
- Shared calendars appear under the *recipient's* principal as a delegated collection
- Colour stored as `calendar-color` (Apple extension, widely supported by clients)

## Open Questions

- Should read-write shared users be able to see *who* created an event (i.e. show creator name)?
- Maximum number of calendars per user? (Probably no hard limit for self-hosted.)
- Should sharing be visible to CalDAV clients via the delegation/sharing extensions (RFC 6638)?

## Resolution

Implemented core multi-calendar support (sharing deferred — requires email infrastructure):

- **Database**: existing `calendars` table (with `color`, `display_name`, `read_only`, `sync_user` columns) used as-is; `ensure_personal_calendar()` lazily creates the default "Personal" calendar on first `get_calendars()` call
- **Server functions** (`api.rs`): `get_calendars`, `create_calendar_api`, `rename_calendar_api`, `update_calendar_color_api`, `delete_calendar_api`; `get_events` JOINs the calendars table to attach `calendar_color`; `create_event` routes to the correct per-user calendar directory
- **Types** (`types.rs`): added `CalendarInfo`; extended `Event` with `calendar`/`calendar_color` fields; extended `NewEvent` with `calendar` field
- **Frontend** (`calendar.rs`): `CalendarsCtx = RwSignal<Vec<(CalendarInfo, bool)>>` shared via context; sidebar `CalendarList` component with color swatch (native color picker), visibility checkbox, rename inline input, delete with confirm dialog; new-event dialog gains a calendar `<select>` when >1 calendar exists; events rendered in their calendar color via inline `background-color`
- **CSS** (`style.css`): ~170 lines for the sidebar calendar list UI
- **Deviations**: sharing (invite flow, permission levels, CalDAV delegation) deferred; CalDAV `calendar-color` PROPFIND response not yet updated (Apple extension)

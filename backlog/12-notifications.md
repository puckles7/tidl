# Notifications

Add notification support for calendar events. Users should be able to configure per-event alerts (type and timing) as well as default notification preferences in the settings dialog. The implementation should align closely with VALARM (RFC 5545) so that CalDAV round-trips preserve notification data faithfully.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — introduces the SMTP/`Mailer` infrastructure that email notifications depend on

## Background

VALARM is the iCalendar component for attaching alarms to events. It supports actions like `EMAIL`, `DISPLAY`, and `AUDIO`, a trigger (absolute or relative to event start/end), and optional repeat/duration fields. Designing the internal model to mirror VALARM makes CalDAV sync lossless and future-proofs the abstraction.

## Functional Requirements

### Per-event notification settings
- Allow one or more notifications per event (matching VALARM's 1-to-many relationship)
- Each notification has:
  - **Action** — initially `email` and `display` (push); keep action an enum/trait so new types slot in without touching existing code
  - **Trigger** — relative offset before/after event start or end (e.g. −15 min), or an absolute datetime
- Expose this in the event create/edit view

### General notification defaults (Settings dialog)
- Default action type (e.g. email)
- Default lead time (e.g. 15 minutes before start)
- Applied automatically when creating a new event (user can override per-event)

### Notification types
- **Email** — send an email reminder to the user
- **Push** (stretch) — browser/device push notification
- Keep the action type behind a clean abstraction (e.g. a `NotificationAction` trait or enum) so additional types (SMS, webhook) can be added without touching notification dispatch logic

### VALARM integration
- Map the internal notification model 1-to-1 with VALARM fields (`ACTION`, `TRIGGER`, `DESCRIPTION`, `SUMMARY`, `ATTENDEE` for email)
- When parsing incoming CalDAV events, preserve all VALARM components; when serialising, emit valid VALARM blocks
- If a VALARM action type is not yet supported, store it as-is (round-trip safe) and surface it read-only in the UI

## Open Questions

- Do we need a background job/scheduler to actually dispatch email notifications, or defer that to a separate story?
- Should push notifications use the Web Push API (browser) or a native mobile push service?
- How do we handle timezone-aware absolute VALARM triggers across CalDAV clients?
- Should default notification settings be per-calendar or global?

## Resolution

Implemented as part of ongoing CalDAV/event work. The full VALARM model is in place:

- **Data model** — `Alarm` struct (`tidl-web/src/types.rs`) covers action, amount+unit trigger, repeat count and delay, and a raw-iCal passthrough for lossless round-tripping of unsupported trigger forms.
- **Server-side parsing/serialisation** — `tidl-server/src/alarm/` implements RFC 5545 VALARM parsing with an `AlarmAction` enum (Display, Audio, Email, Other) and `AlarmTrigger` enum; unsupported actions are preserved verbatim.
- **Browser-side scheduler** — `tidl-web/src/alarm_scheduler.rs` polls every 30 seconds, fires browser notifications (Web Notifications API) for `display` alarms, synthesises a bell tone via Web Audio API for `audio` alarms, and handles `REPEAT`/`DURATION` with `setTimeout`.
- **UI** — `AlarmEditor` component inside the event create and edit dialogs: action selector, amount + unit dropdowns, repeat fields, and a read-only "raw alarm" badge for unsupported trigger forms.

Deviations from spec: email dispatch (server-side background job) and global default notification settings were not implemented; those remain as potential follow-up items.

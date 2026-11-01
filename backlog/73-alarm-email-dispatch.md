# Server-side EMAIL alarm dispatch (background job)

Actually send reminder emails for events whose alarm action is `EMAIL`. Today `tidl-server/src/alarm/email.rs` is a doc-only stub — the VALARM model round-trips `ACTION:EMAIL` faithfully, but nothing ever sends the email. This was a known, explicitly-flagged gap left open in [#12 - Notifications](12-notifications.md)'s resolution.

## Dependencies

- [12 - Notifications](12-notifications.md) — VALARM model, parsing/serialisation already in place
- [13 - Calendar sharing](13-calendar-sharing.md) — introduced the SMTP/`Mailer` infrastructure this would reuse

## Background

Found while investigating a user report of "no alerts from the calendar" (2026-07-30): browser DISPLAY/AUDIO alarms do work via `tidl-web/src/alarm_scheduler.rs` (separately diagnosed as blocked by the browser's insecure-origin Notification policy, not a code bug), but EMAIL alarms have no dispatch path at all — `tidl-server/src/alarm/mod.rs` is explicitly marked `#![allow(dead_code)] // module is future infrastructure for server-side alarm dispatch`.

## Functional Requirements

- A background job (interval poll or scheduled task) that finds events with an `EMAIL` alarm whose trigger time has arrived and hasn't yet been sent
- Track "already sent" state per (event, alarm) so restarts/polling don't double-send — mirrors the browser scheduler's `FIRED` dedup set, but needs to be durable (survives server restart), so likely a DB table rather than in-memory
- Reuse the existing `Mailer`/SMTP config (`tidl-server/src/mailer.rs`) for delivery
- Compose the email using the alarm's `SUMMARY`/`DESCRIPTION` (RFC 5545 requires these for `ACTION:EMAIL`) and the event's own summary/time as fallback
- Handle recurring events (RRULE expansion) — an alarm on a recurring event should fire once per occurrence, not once total
- Handle `REPEAT`/`DURATION` (repeated reminders) the same way the browser scheduler does

## Decisions

- **Poll mechanism** — a dedicated `run_alarm_dispatch_loop`, `tokio::spawn`ed
  in `main.rs` alongside the existing `remote_sync`/`markdown_sync`/`webcal_sync`
  loops (same established pattern), own interval (~60s), independent of the
  other jobs.
- **Sent-state storage** — new table `sent_alarm_notifications(event_uid,
  alarm_index, occurrence_start, sent_at)` with a uniqueness constraint on
  `(event_uid, alarm_index, occurrence_start)`. `occurrence_start` makes
  recurring-event dedup natural; a `REPEAT`/`DURATION` repeat is its own
  logical trigger and gets its own row.
- **SMTP not configured** — log (`tracing::warn!`, once per skipped
  event+alarm) and skip; no new UI surface. EMAIL alarms are opt-in
  per-event and SMTP setup is a one-time step in Settings.
- **Dispatch scope** — single instance-wide scan per tick, not per-user.
  SMTP config is already a single instance-wide row (`smtp_config WHERE id
  = 1`), so there's no per-user SMTP identity to dispatch against — every
  user's email goes out through the same shared `SmtpMailer`.

## Design notes (for implementation)

- Email composition: subject/body from the alarm's `SUMMARY`/`DESCRIPTION`,
  falling back to the event's own summary/time if absent.
- Error handling: SMTP send failures are logged (`tracing::error!`) and the
  row is *not* marked sent, so a transient failure retries next tick. A
  permanently-failing send retries every tick — acceptable for now, matches
  the retry posture of the other sync loops.
- Testing: library-level unit tests for the "which alarms are due" query and
  dedup-insert races (`sqlx::test`); a `tokio::test` integration test with
  `StubMailer` asserting a due alarm sends exactly once across two
  consecutive loop ticks.

## Resolution

Implemented 2026-08-31 as `crates/tidl-server/src/alarm/dispatch.rs`
(`run_alarm_dispatch_loop`), spawned in `main.rs` alongside the other
background loops.

- **Recurring-event expansion** reuses `tidl_web::api::event_row_to_web_events`
  (bumped from private to `pub`) — the exact RRULE/exception-row logic the
  Calendar view's `get_events` already relies on — instead of re-implementing
  occurrence expansion in `tidl-server`. Each tick scans all VEVENTs
  instance-wide (`ical_data LIKE '%ACTION:EMAIL%'` prefilter) with a rolling
  window: alarms are only considered "due" if their trigger time falls in
  `[now - 24h, now]` (bounds the flood of stale reminders after downtime),
  and occurrences are expanded up to 60 days ahead (bounds how far in advance
  a very long alarm offset can be detected — a documented, accepted
  limitation for pathologically large offsets).
- **REPEAT/DURATION** repeats are each their own firing/dedup row, matching
  the browser scheduler's semantics — the `occurrence_start` dedup key got a
  `#<repeat_index>` suffix (0 = base firing) beyond what was originally
  scoped, to disambiguate them; noted in the migration's doc comment.
- **Recipient resolution** (not explicitly scoped above, but necessary to
  implement "compose the email"): prefers an explicit `ATTENDEE:mailto:...`
  line on the alarm (e.g. from an externally-authored VALARM); the web UI has
  no field for one yet, so UI-created EMAIL alarms fall back to the event
  owner's account email (`users.email`).
- `sent_alarm_notifications.username` was added to `profile.rs`'s
  `USERNAME_COLUMNS` so a username rename cascades to it (caught by that
  module's exhaustiveness test).
- Migration: `migrations/20260831000000_sent_alarm_notifications.sql`.

No deviations from the four Decisions above. Tests: 6 pure unit tests for
`due_email_alarms`/`extract_ical_prop` plus 2 `sqlx::test` integration tests
(exactly-once-across-two-ticks, and no-send-when-nothing-due) using an
in-memory recording `Mailer`.

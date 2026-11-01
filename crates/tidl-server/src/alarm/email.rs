//! EMAIL alarm — sends an email when the alarm triggers.
//!
//! CalDAV clients handle delivery.  The iCal for an EMAIL alarm requires:
//!
//!   BEGIN:VALARM
//!   ACTION:EMAIL
//!   TRIGGER:-PT15M
//!   SUMMARY:Reminder: <event title>
//!   DESCRIPTION:This is a reminder for your event.
//!   ATTENDEE:mailto:user@example.com
//!   END:VALARM
//!
//! SUMMARY and DESCRIPTION are REQUIRED by RFC 5545 for EMAIL alarms, and at
//! least one ATTENDEE is required.  These are preserved verbatim through the
//! raw_ical round-trip mechanism if they came from an external client.
//!
//! Blocks created via the web UI carry no ATTENDEE (the UI has no field for
//! one yet); `dispatch` falls back to the event owner's account email in
//! that case.
//!
//! Server-side delivery (for alarms tidl itself owns, as opposed to ones an
//! external CalDAV client attached) is `dispatch`'s job — see
//! `crate::alarm::dispatch::run_alarm_dispatch_loop`.

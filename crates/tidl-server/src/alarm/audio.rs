//! AUDIO alarm — plays a sound in the calendar client.
//!
//! CalDAV clients handle this autonomously.  The web UI plays a synthesised
//! bell via the Web Audio API (see calendar.rs in tidl-web) when the alarm
//! time arrives while the browser tab is open.
//!
//! The iCal for an AUDIO alarm without an explicit ATTACH:
//!
//!   BEGIN:VALARM
//!   ACTION:AUDIO
//!   TRIGGER:-PT15M
//!   END:VALARM
//!
//! An optional ATTACH pointing to an audio file is preserved verbatim through
//! the raw_ical round-trip mechanism.  We never generate one ourselves.
//!
//! Nothing to execute server-side.

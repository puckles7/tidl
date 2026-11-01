//! iCalendar utilities and helpers
//!
//! Thin wrappers around the `icalendar` crate that keep the rest of the
//! codebase insulated from API details.

use chrono::{DateTime, Utc};
use icalendar::{Calendar, CalendarComponent, Component, Event, EventLike, Todo};

use crate::error::CalDavError;

type Result<T> = std::result::Result<T, CalDavError>;

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Parse a raw iCalendar string.
pub fn parse_icalendar(ical_str: &str) -> Result<Calendar> {
    ical_str.parse::<Calendar>().map_err(CalDavError::IcalParse)
}

/// Validate a raw iCalendar string.
///
/// The `icalendar` crate's parser is lenient, so we also require the structural
/// markers that RFC 5545 mandates: the data must begin with `BEGIN:VCALENDAR`
/// and end with `END:VCALENDAR`, and the parse must succeed.
pub fn validate_icalendar(ical_str: &str) -> Result<()> {
    let trimmed = ical_str.trim();
    if !trimmed.starts_with("BEGIN:VCALENDAR") {
        return Err(CalDavError::InvalidRequest(
            "iCalendar data must begin with BEGIN:VCALENDAR".to_string(),
        ));
    }
    if !trimmed.ends_with("END:VCALENDAR") {
        return Err(CalDavError::InvalidRequest(
            "iCalendar data must end with END:VCALENDAR".to_string(),
        ));
    }
    parse_icalendar(ical_str)?;
    Ok(())
}

// ── Extraction helpers ────────────────────────────────────────────────────────

/// Return the UID of the first component that has one.
pub fn extract_uid(ical_str: &str) -> Result<String> {
    let calendar = parse_icalendar(ical_str)?;
    for component in &calendar.components {
        let uid = match component {
            CalendarComponent::Event(e) => e.get_uid(),
            CalendarComponent::Todo(t) => t.get_uid(),
            CalendarComponent::Venue(v) => v.property_value("UID"),
            _ => None,
        };
        if let Some(uid) = uid {
            return Ok(uid.to_string());
        }
    }
    Err(CalDavError::MissingUid)
}

/// Return `"VEVENT"`, `"VTODO"`, or `"VVENUE"` for the first component.
pub fn extract_component_type(ical_str: &str) -> Result<String> {
    let calendar = parse_icalendar(ical_str)?;
    let kind = match calendar.components.first() {
        Some(CalendarComponent::Event(_)) => "VEVENT",
        Some(CalendarComponent::Todo(_)) => "VTODO",
        Some(CalendarComponent::Venue(_)) => "VVENUE",
        Some(_) => "UNKNOWN",
        None => return Err(CalDavError::MissingComponent),
    };
    Ok(kind.to_string())
}

// ── PARTSTAT helpers ──────────────────────────────────────────────────────────

/// Extract the PARTSTAT value from the first ATTENDEE line in iCalendar data.
///
/// Handles RFC 5545 line folding (CRLF/LF followed by whitespace).
/// Returns values like `"ACCEPTED"`, `"TENTATIVE"`, `"DECLINED"`, etc.
pub fn extract_partstat(ical_str: &str) -> Option<String> {
    let unfolded = unfold_ical(ical_str);

    for line in unfolded.lines() {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("ATTENDEE") {
            if let Some(pos) = upper.find("PARTSTAT=") {
                let rest = &line[pos + 9..];
                let end = rest.find([';', ':']).unwrap_or(rest.len());
                return Some(rest[..end].to_ascii_uppercase());
            }
        }
    }
    None
}

/// Inject or update PARTSTAT on the first ATTENDEE line in iCalendar data.
///
/// If no ATTENDEE line exists, one is inserted before `END:VEVENT`.
/// `mailto` is only used when creating a new ATTENDEE line.
///
/// Note: does not handle folded (multi-physical-line) ATTENDEE properties;
/// for those, the caller should unfold first or build ICS with PARTSTAT
/// included from the start.
pub fn inject_partstat(ical_str: &str, partstat: &str, mailto: Option<&str>) -> String {
    let crlf = ical_str.contains("\r\n");
    let line_ending = if crlf { "\r\n" } else { "\n" };

    let lines: Vec<&str> = if crlf {
        ical_str.split("\r\n").collect()
    } else {
        ical_str.split('\n').collect()
    };

    let mut output: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut found_attendee = false;

    for line in lines {
        let trimmed = line.trim_start();
        let upper = trimmed.to_ascii_uppercase();

        if upper.starts_with("ATTENDEE") && !found_attendee {
            found_attendee = true;
            if let Some(pos) = upper.find("PARTSTAT=") {
                // Replace existing PARTSTAT value
                let rest = &trimmed[pos + 9..];
                let end_offset = rest.find([';', ':']).unwrap_or(rest.len());
                output.push(format!(
                    "{}PARTSTAT={}{}",
                    &trimmed[..pos],
                    partstat,
                    &trimmed[pos + 9 + end_offset..]
                ));
            } else if let Some(colon) = trimmed.find(':') {
                // Insert PARTSTAT before the value colon
                output.push(format!(
                    "{};PARTSTAT={}{}",
                    &trimmed[..colon],
                    partstat,
                    &trimmed[colon..]
                ));
            } else {
                output.push(line.to_string());
            }
        } else if upper == "END:VEVENT" && !found_attendee {
            // No ATTENDEE found — insert a minimal one before END:VEVENT
            let attendee = match mailto {
                Some(addr) => format!("ATTENDEE;PARTSTAT={}:mailto:{}", partstat, addr),
                None => format!("ATTENDEE;PARTSTAT={}:", partstat),
            };
            output.push(attendee);
            output.push(line.to_string());
            found_attendee = true;
        } else {
            output.push(line.to_string());
        }
    }

    output.join(line_ending)
}

// ── Field extraction for DB storage ──────────────────────────────────────────

/// Fields extracted from a raw iCalendar string, ready for database storage.
#[derive(Debug, Default)]
pub struct EventFields {
    /// VEVENT or VTODO (uppercase).
    pub component_type: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    /// `start_date` in SQLite `DATETIME` format (`YYYY-MM-DD HH:MM:SS`).
    pub start_date: Option<String>,
    /// `end_date` / `DTEND` in SQLite `DATETIME` format.
    pub end_date: Option<String>,
    /// `due_date` / `DUE` in SQLite `DATETIME` format (VTODO only).
    pub due_date: Option<String>,
    /// `PRIORITY` (VEVENT or VTODO). RFC 5545 §3.8.1.9: 1 = highest,
    /// 9 = lowest, 0/absent = unset.
    pub priority: Option<i64>,
    /// `STATUS` (VTODO only), raw RFC 5545 §3.8.1.11 value — `NEEDS-ACTION`,
    /// `COMPLETED`, `IN-PROCESS`, or `CANCELLED`. `None` if the property is
    /// absent from the body.
    pub status: Option<String>,
}

/// Extract the displayable fields from a raw iCalendar string.
///
/// Handles RFC 5545 line folding.  Timezone-aware datetimes are stored in
/// whatever their local representation is (no tz conversion); the web UI
/// treats all stored datetimes as local time.
pub fn extract_event_fields(ical_str: &str) -> EventFields {
    let unfolded = unfold_ical(ical_str);
    let mut fields = EventFields::default();
    let mut in_component = false; // inside VEVENT or VTODO

    for line in unfolded.lines() {
        let upper = line.to_ascii_uppercase();

        if upper == "BEGIN:VEVENT" {
            in_component = true;
            fields.component_type = "VEVENT".to_string();
            continue;
        }
        if upper == "BEGIN:VTODO" {
            in_component = true;
            fields.component_type = "VTODO".to_string();
            continue;
        }
        if upper == "END:VEVENT" || upper == "END:VTODO" {
            // Stop after the first component.
            break;
        }

        if !in_component {
            continue;
        }

        // SUMMARY
        if fields.summary.is_none() {
            if let Some(v) = prop_value_with_params(line, "SUMMARY") {
                let s = unescape_ical_text(v.trim());
                if !s.is_empty() {
                    fields.summary = Some(s);
                }
            }
        }

        // DESCRIPTION
        if fields.description.is_none() {
            if let Some(v) = prop_value_with_params(line, "DESCRIPTION") {
                let s = unescape_ical_text(v.trim());
                if !s.is_empty() {
                    fields.description = Some(s);
                }
            }
        }

        // LOCATION
        if fields.location.is_none() {
            if let Some(v) = prop_value_with_params(line, "LOCATION") {
                let s = unescape_ical_text(v.trim());
                if !s.is_empty() {
                    fields.location = Some(s);
                }
            }
        }

        // DTSTART — may include TZID param: "DTSTART;TZID=America/New_York:20230601T120000"
        if fields.start_date.is_none() {
            if let Some(v) = prop_value_with_params(line, "DTSTART") {
                fields.start_date = ical_dt_to_sqlite(v.trim());
            }
        }

        // DTEND
        if fields.end_date.is_none() {
            if let Some(v) = prop_value_with_params(line, "DTEND") {
                fields.end_date = ical_dt_to_sqlite(v.trim());
            }
        }

        // DUE (VTODO)
        if fields.due_date.is_none() {
            if let Some(v) = prop_value_with_params(line, "DUE") {
                fields.due_date = ical_dt_to_sqlite(v.trim());
            }
        }

        // PRIORITY (VTODO)
        if fields.priority.is_none() {
            if let Some(v) = prop_value_with_params(line, "PRIORITY") {
                fields.priority = v.trim().parse::<i64>().ok();
            }
        }

        // STATUS (VTODO)
        if fields.status.is_none() {
            if let Some(v) = prop_value_with_params(line, "STATUS") {
                let s = v.trim();
                if !s.is_empty() {
                    fields.status = Some(s.to_ascii_uppercase());
                }
            }
        }
    }

    fields
}

/// Convert an iCalendar date or date-time value to SQLite DATETIME format.
///
/// Handles:
/// - `VALUE=DATE` dates: `20230101` → `"2023-01-01 00:00:00"`
/// - UTC date-times: `20230101T120000Z` → `"2023-01-01 12:00:00"`
/// - Floating date-times: `20230101T120000` → `"2023-01-01 12:00:00"`
///
/// Returns `None` for anything that doesn't parse.
pub fn ical_dt_to_sqlite(value: &str) -> Option<String> {
    // Strip trailing 'Z' (UTC marker) — we store as-is.
    let v = value.trim_end_matches('Z');

    if v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()) {
        // DATE-only: YYYYMMDD
        Some(format!("{}-{}-{} 00:00:00", &v[0..4], &v[4..6], &v[6..8]))
    } else if v.len() == 15 && v.as_bytes()[8] == b'T' {
        // DATETIME: YYYYMMDD'T'HHMMSS
        Some(format!(
            "{}-{}-{} {}:{}:{}",
            &v[0..4],
            &v[4..6],
            &v[6..8],
            &v[9..11],
            &v[11..13],
            &v[13..15]
        ))
    } else {
        None
    }
}

/// Unescape iCalendar TEXT value escapes: `\n` → newline, `\,` → `,`, `\\` → `\`.
fn unescape_ical_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some('t') | Some('T') => out.push('\t'),
                Some(x) => out.push(x),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ── Low-level iCal line utilities ─────────────────────────────────────────────
//
// These are shared by both the server-side VALARM parser (`tidl-server/alarm`)
// and the web-side alarm parser (`tidl-web/alarm_parse`) so they live here
// rather than being duplicated in each consumer.

/// Unfold iCalendar content lines (RFC 5545 §3.1: CRLF + single WSP = continuation).
///
/// Single-pass byte scan rather than four chained `String::replace` calls
/// (each of which allocates and rescans the whole string) — this is called
/// per event on every `get_events` request, so on a calendar with a few
/// thousand events the old four-pass version was a measured hotspot.
/// Matching on bytes (not `char`s) is safe here: CR/LF/SPACE/TAB are all
/// ASCII, and a UTF-8 continuation byte is always `>= 0x80`, so it can never
/// be mistaken for one of these markers or split mid-character.
pub fn unfold_ical(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r'
            && bytes.get(i + 1) == Some(&b'\n')
            && matches!(bytes.get(i + 2), Some(&b' ') | Some(&b'\t'))
        {
            i += 3;
            continue;
        }
        if bytes[i] == b'\n' && matches!(bytes.get(i + 1), Some(&b' ') | Some(&b'\t')) {
            i += 2;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() && bytes[i] != b'\r' && bytes[i] != b'\n' {
            i += 1;
        }
        out.push_str(&s[start..i]);
    }
    out
}

/// Escape iCalendar TEXT value special characters (RFC 5545 §3.3.11):
/// backslash, semicolon, comma, and newline. Inverse of `unescape_ical_text`.
/// CRLF and lone CR are both normalized to the single `\n` escape so a
/// round trip through `unescape_ical_text` reproduces plain `\n` newlines.
pub fn escape_ical_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Fold a single iCalendar content line to RFC 5545 §3.1's 75-octet limit,
/// inserting `CRLF` + a single continuation space before each following
/// segment. Splits only on UTF-8 char boundaries, so a segment may be a few
/// octets short of 75 when it ends on a multi-byte character. Pairs with
/// `unfold_ical` on the read side.
pub fn fold_ical_line(line: &str) -> String {
    const LIMIT: usize = 75;
    if line.len() <= LIMIT {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + line.len() / LIMIT * 3);
    let mut start = 0;
    let mut chunk_limit = LIMIT;
    while line.len() - start > LIMIT {
        let mut end = start + chunk_limit;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&line[start..end]);
        out.push_str("\r\n ");
        start = end;
        chunk_limit = LIMIT - 1; // continuation lines spend 1 octet on the leading space
    }
    out.push_str(&line[start..]);
    out
}

/// Parse the simple duration forms used in TRIGGER/DURATION properties.
///
/// Handles `-PTnM`, `-PTnH`, `-PnD`, `-PnW` (and positive variants).
/// Returns total minutes, or `None` for anything more complex (combined
/// date+time, seconds, etc.).
pub fn parse_simple_duration(s: &str) -> Option<i32> {
    let s = s.trim();
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let rest = rest.strip_prefix('P')?;

    // Week: PnW
    if let Some(r) = rest.strip_suffix('W') {
        if rest.starts_with('T') {
            return None; // PTnW is not valid
        }
        let n: i32 = r.parse().ok()?;
        let m = n * 7 * 24 * 60;
        return Some(if neg { -m } else { m });
    }
    // Day: PnD (no time component)
    if !rest.contains('T') {
        if let Some(r) = rest.strip_suffix('D') {
            let n: i32 = r.parse().ok()?;
            let m = n * 24 * 60;
            return Some(if neg { -m } else { m });
        }
        return None;
    }
    // Pure time: PTnH or PTnM (not combined PTnHnM)
    let time = rest.strip_prefix('T')?;
    if let Some(r) = time.strip_suffix('H') {
        if r.contains('M') || r.contains('S') {
            return None;
        }
        let n: i32 = r.parse().ok()?;
        let m = n * 60;
        return Some(if neg { -m } else { m });
    }
    if let Some(r) = time.strip_suffix('M') {
        if r.contains('H') || r.contains('S') {
            return None;
        }
        let n: i32 = r.parse().ok()?;
        return Some(if neg { -n } else { n });
    }
    None
}

/// Extract the value of a property line (after the first `:`), case-insensitively.
///
/// Matches `NAME:value` forms. Returns `None` if the line does not start with
/// `{name}:` (case-insensitive).
pub fn prop_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let upper = line.to_uppercase();
    let prefix = format!("{}:", name.to_uppercase());
    if upper.starts_with(&prefix) {
        Some(&line[name.len() + 1..])
    } else {
        None
    }
}

/// Like `prop_value` but also matches `NAME;params:value`.
pub fn prop_value_with_params<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let upper = line.to_uppercase();
    let name_upper = name.to_uppercase();
    if upper.starts_with(&format!("{}:", name_upper)) {
        return Some(&line[name.len() + 1..]);
    }
    if upper.starts_with(&format!("{};", name_upper)) {
        return line.find(':').map(|pos| &line[pos + 1..]);
    }
    None
}

/// Serialise `total_mins` as an iCal duration string.
///
/// Negative values produce `"-P…"` (before-start) forms; positive produce `"P…"`.
/// Chooses the most compact unit (weeks → days → hours → minutes).
pub fn ical_duration_str(total_mins: i32) -> String {
    let m = total_mins.abs();
    let sign = if total_mins < 0 { "-" } else { "" };
    if m == 0 {
        return "PT0S".to_string();
    }
    if m % (7 * 24 * 60) == 0 {
        format!("{}P{}W", sign, m / (7 * 24 * 60))
    } else if m % (24 * 60) == 0 {
        format!("{}P{}D", sign, m / (24 * 60))
    } else if m % 60 == 0 {
        format!("{}PT{}H", sign, m / 60)
    } else {
        format!("{}PT{}M", sign, m)
    }
}

// ── UID sanitization ──────────────────────────────────────────────────────────

/// Detect and normalise Google "continuation series" UIDs.
///
/// Google Calendar sometimes ends a recurring series and starts a new one by
/// creating a fresh VEVENT whose UID encodes the old series' recurrence-ID:
///
///   `{base}_R{YYYYMMDDTHHmmss}[Z]@{domain}`
///
/// iOS Calendar pattern-matches this form and treats the VEVENT as a
/// recurrence override of `{base}@{domain}` rather than as an independent
/// series master.  When the parent series has already ended, iOS silently
/// discards the event, so no RRULE occurrences are ever shown.
///
/// This function rewrites such UIDs to `{base}-{YYYYMMDDTHHmmss}@tidl.local`,
/// which is unique, stable, and does not trigger iOS's pattern matching.
///
/// Apply this to BOTH the series master and every RECURRENCE-ID override that
/// carries the same base UID — the transformation is deterministic so both
/// sides always agree.
pub fn sanitize_google_continuation_uid(uid: &str) -> String {
    let Some(r_pos) = find_google_continuation_marker(uid) else {
        return uid.to_string();
    };

    let base = &uid[..r_pos]; // everything before `_R`
    let after_r = &uid[r_pos + 2..]; // skip `_R`

    // after_r = "{timestamp}[Z]@{domain}"
    let at_pos = after_r
        .find('@')
        .expect("checked by find_google_continuation_marker");
    let timestamp = &after_r[..at_pos]; // e.g. "20231118T090000" or "20231118T090000Z"
    let timestamp = timestamp.trim_end_matches('Z');

    format!("{base}-{timestamp}@tidl.local")
}

/// Returns the byte offset of `_R` within `uid` if the UID matches the Google
/// continuation pattern `*_R{8-digit-date}T{6-digit-time}[Z]@*`.
fn find_google_continuation_marker(uid: &str) -> Option<usize> {
    let bytes = uid.as_bytes();
    let len = uid.len();
    // Need at least: "_R" + 8 digits + "T" + 6 digits + "@" = 18 chars from marker
    for i in 0..len.saturating_sub(17) {
        if bytes[i] != b'_' || bytes.get(i + 1) != Some(&b'R') {
            continue;
        }
        let rest = &uid[i + 2..];
        // 8 decimal digits
        if rest.len() < 16 {
            continue;
        }
        if !rest[..8].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // 'T'
        if rest.as_bytes()[8] != b'T' {
            continue;
        }
        // 6 decimal digits
        if !rest[9..15].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // optional 'Z', then '@'
        let after_dt = if rest.as_bytes().get(15) == Some(&b'Z') {
            &rest[16..]
        } else {
            &rest[15..]
        };
        if after_dt.starts_with('@') {
            return Some(i);
        }
    }
    None
}

/// Rewrite the `UID:` property inside an iCal string.
///
/// Handles folded lines (RFC 5545 line continuation).  The resulting string
/// uses the same line endings as the input.
pub fn rewrite_uid_in_ical(ical_str: &str, old_uid: &str, new_uid: &str) -> String {
    let unfolded = unfold_ical(ical_str);
    let eol = if ical_str.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    unfolded
        .lines()
        .map(|line| {
            let up = line.to_ascii_uppercase();
            if up.starts_with("UID:") && &line[4..] == old_uid {
                format!("UID:{new_uid}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(eol)
}

// ── Builders ──────────────────────────────────────────────────────────────────

/// Build a minimal VEVENT iCalendar string.
pub fn create_event(uid: &str, summary: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let mut event = Event::new();
    event.uid(uid).summary(summary).starts(start).ends(end);
    Calendar::new().push(event.done()).done().to_string()
}

/// Build a minimal VTODO iCalendar string.
///
/// `priority` follows RFC 5545 §3.8.1.9 (1 = highest, 9 = lowest; 0 = unset).
pub fn create_todo(
    uid: &str,
    summary: &str,
    priority: Option<u32>,
    due: Option<DateTime<Utc>>,
) -> String {
    let mut todo = Todo::new();
    todo.uid(uid).summary(summary);
    if let Some(p) = priority {
        todo.priority(p);
    }
    if let Some(d) = due {
        todo.due(d);
    }
    Calendar::new().push(todo.done()).done().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_VEVENT: &str = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\n\
UID:test-uid@example.com\r\n\
SUMMARY:Test Event\r\n\
DTSTART:20230601T120000Z\r\n\
DTEND:20230601T130000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR";

    const SIMPLE_VTODO: &str = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VTODO\r\n\
UID:todo-uid@example.com\r\n\
SUMMARY:Test Todo\r\n\
DUE:20230615T090000Z\r\n\
END:VTODO\r\n\
END:VCALENDAR";

    // ── validate_icalendar ──────────────────────────────────────────────────

    #[test]
    fn validate_valid_vevent() {
        assert!(validate_icalendar(SIMPLE_VEVENT).is_ok());
    }

    #[test]
    fn validate_missing_begin_vcalendar() {
        let bad = "BEGIN:VEVENT\r\nUID:x\r\nEND:VEVENT\r\nEND:VCALENDAR";
        assert!(validate_icalendar(bad).is_err());
    }

    #[test]
    fn validate_missing_end_vcalendar() {
        let bad = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nEND:VEVENT";
        assert!(validate_icalendar(bad).is_err());
    }

    // ── extract_uid ─────────────────────────────────────────────────────────

    #[test]
    fn extract_uid_from_vevent() {
        assert_eq!(extract_uid(SIMPLE_VEVENT).unwrap(), "test-uid@example.com");
    }

    #[test]
    fn extract_uid_from_vtodo() {
        assert_eq!(extract_uid(SIMPLE_VTODO).unwrap(), "todo-uid@example.com");
    }

    #[test]
    fn extract_uid_missing_returns_error() {
        let no_uid =
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:No UID\r\nEND:VEVENT\r\nEND:VCALENDAR";
        assert!(extract_uid(no_uid).is_err());
    }

    // ── extract_component_type ───────────────────────────────────────────────

    #[test]
    fn component_type_vevent() {
        assert_eq!(extract_component_type(SIMPLE_VEVENT).unwrap(), "VEVENT");
    }

    #[test]
    fn component_type_vtodo() {
        assert_eq!(extract_component_type(SIMPLE_VTODO).unwrap(), "VTODO");
    }

    // ── ical_dt_to_sqlite ────────────────────────────────────────────────────

    #[test]
    fn dt_date_only() {
        assert_eq!(
            ical_dt_to_sqlite("20230601"),
            Some("2023-06-01 00:00:00".to_string())
        );
    }

    #[test]
    fn dt_utc_datetime() {
        assert_eq!(
            ical_dt_to_sqlite("20230601T120000Z"),
            Some("2023-06-01 12:00:00".to_string())
        );
    }

    #[test]
    fn dt_floating_datetime() {
        assert_eq!(
            ical_dt_to_sqlite("20230601T120000"),
            Some("2023-06-01 12:00:00".to_string())
        );
    }

    #[test]
    fn dt_invalid_returns_none() {
        assert_eq!(ical_dt_to_sqlite("not-a-date"), None);
        assert_eq!(ical_dt_to_sqlite(""), None);
    }

    // ── unfold_ical ──────────────────────────────────────────────────────────

    #[test]
    fn unfold_crlf_space() {
        assert_eq!(unfold_ical("SUMMARY:Hello\r\n World"), "SUMMARY:HelloWorld");
    }

    #[test]
    fn unfold_lf_space() {
        assert_eq!(unfold_ical("SUMMARY:Hello\n World"), "SUMMARY:HelloWorld");
    }

    #[test]
    fn unfold_lf_tab() {
        assert_eq!(unfold_ical("SUMMARY:Hello\n\tWorld"), "SUMMARY:HelloWorld");
    }

    #[test]
    fn unfold_no_folding_unchanged() {
        let s = "SUMMARY:Hello World";
        assert_eq!(unfold_ical(s), s);
    }

    // ── escape_ical_text ─────────────────────────────────────────────────────

    #[test]
    fn escape_backslash_semicolon_comma() {
        assert_eq!(escape_ical_text("a\\b;c,d"), "a\\\\b\\;c\\,d");
    }

    #[test]
    fn escape_newline_becomes_backslash_n() {
        assert_eq!(escape_ical_text("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn escape_crlf_becomes_single_backslash_n() {
        assert_eq!(escape_ical_text("line1\r\nline2"), "line1\\nline2");
    }

    #[test]
    fn escape_plain_text_unchanged() {
        assert_eq!(escape_ical_text("Hello World"), "Hello World");
    }

    #[test]
    fn escape_then_unescape_round_trips() {
        let original = "Notes, with; punctuation\\and\nnewlines";
        let escaped = escape_ical_text(original);
        let unescaped = unescape_ical_text(&escaped);
        assert_eq!(unescaped, original);
    }

    // ── fold_ical_line ───────────────────────────────────────────────────────

    #[test]
    fn fold_short_line_unchanged() {
        let line = "DESCRIPTION:short";
        assert_eq!(fold_ical_line(line), line);
    }

    #[test]
    fn fold_line_at_75_octets() {
        let value = "x".repeat(100);
        let line = format!("DESCRIPTION:{value}");
        let folded = fold_ical_line(&line);
        for physical_line in folded.split("\r\n") {
            assert!(
                physical_line.len() <= 75,
                "line too long: {physical_line:?}"
            );
        }
    }

    #[test]
    fn fold_then_unfold_round_trips() {
        let value = "y".repeat(200);
        let line = format!("DESCRIPTION:{value}");
        let folded = fold_ical_line(&line);
        assert_eq!(unfold_ical(&folded), line);
    }

    #[test]
    fn fold_does_not_split_multibyte_char_boundary() {
        let value = "é".repeat(60); // 2 bytes each — crosses the 75-octet mark mid-char
        let line = format!("DESCRIPTION:{value}");
        let folded = fold_ical_line(&line);
        for physical_line in folded.split("\r\n") {
            assert!(physical_line.is_char_boundary(physical_line.len()));
        }
        assert_eq!(unfold_ical(&folded), line);
    }

    // ── parse_simple_duration ────────────────────────────────────────────────

    #[test]
    fn duration_negative_minutes() {
        assert_eq!(parse_simple_duration("-PT15M"), Some(-15));
    }

    #[test]
    fn duration_negative_hours() {
        assert_eq!(parse_simple_duration("-PT2H"), Some(-120));
    }

    #[test]
    fn duration_negative_days() {
        assert_eq!(parse_simple_duration("-P1D"), Some(-1440));
    }

    #[test]
    fn duration_positive_weeks() {
        assert_eq!(parse_simple_duration("P1W"), Some(7 * 24 * 60));
    }

    #[test]
    fn duration_positive_minutes() {
        assert_eq!(parse_simple_duration("PT30M"), Some(30));
    }

    #[test]
    fn duration_combined_hours_minutes_rejected() {
        assert_eq!(parse_simple_duration("PT1H30M"), None);
    }

    #[test]
    fn duration_invalid_returns_none() {
        assert_eq!(parse_simple_duration("garbage"), None);
        assert_eq!(parse_simple_duration(""), None);
    }

    // ── ical_duration_str ────────────────────────────────────────────────────

    #[test]
    fn duration_str_zero() {
        assert_eq!(ical_duration_str(0), "PT0S");
    }

    #[test]
    fn duration_str_negative_minutes() {
        assert_eq!(ical_duration_str(-15), "-PT15M");
    }

    #[test]
    fn duration_str_negative_hours() {
        assert_eq!(ical_duration_str(-120), "-PT2H");
    }

    #[test]
    fn duration_str_negative_days() {
        assert_eq!(ical_duration_str(-1440), "-P1D");
    }

    #[test]
    fn duration_str_positive_weeks() {
        assert_eq!(ical_duration_str(7 * 24 * 60), "P1W");
    }

    // ── prop_value ───────────────────────────────────────────────────────────

    #[test]
    fn prop_value_match() {
        assert_eq!(
            prop_value("SUMMARY:Hello World", "SUMMARY"),
            Some("Hello World")
        );
    }

    #[test]
    fn prop_value_no_match() {
        assert_eq!(prop_value("LOCATION:Office", "SUMMARY"), None);
    }

    #[test]
    fn prop_value_case_insensitive() {
        assert_eq!(prop_value("summary:Hello", "SUMMARY"), Some("Hello"));
    }

    // ── prop_value_with_params ───────────────────────────────────────────────

    #[test]
    fn prop_value_with_params_simple() {
        assert_eq!(
            prop_value_with_params("DTSTART:20230601T120000Z", "DTSTART"),
            Some("20230601T120000Z")
        );
    }

    #[test]
    fn prop_value_with_params_tzid() {
        assert_eq!(
            prop_value_with_params("DTSTART;TZID=America/New_York:20230601T120000", "DTSTART"),
            Some("20230601T120000")
        );
    }

    #[test]
    fn prop_value_with_params_no_match() {
        assert_eq!(prop_value_with_params("SUMMARY:Test", "DTSTART"), None);
    }

    // ── extract_partstat ─────────────────────────────────────────────────────

    #[test]
    fn partstat_accepted() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nATTENDEE;PARTSTAT=ACCEPTED:mailto:a@b.com\nEND:VEVENT\nEND:VCALENDAR";
        assert_eq!(extract_partstat(ical), Some("ACCEPTED".to_string()));
    }

    #[test]
    fn partstat_declined() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nATTENDEE;CN=Bob;PARTSTAT=DECLINED:mailto:b@c.com\nEND:VEVENT\nEND:VCALENDAR";
        assert_eq!(extract_partstat(ical), Some("DECLINED".to_string()));
    }

    #[test]
    fn partstat_missing_returns_none() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:No attendee\nEND:VEVENT\nEND:VCALENDAR";
        assert_eq!(extract_partstat(ical), None);
    }

    // ── inject_partstat ──────────────────────────────────────────────────────

    #[test]
    fn inject_partstat_updates_existing() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:a@b.com\nEND:VEVENT\nEND:VCALENDAR\n";
        let result = inject_partstat(ical, "ACCEPTED", None);
        assert!(result.contains("PARTSTAT=ACCEPTED"), "result: {result}");
        assert!(!result.contains("PARTSTAT=NEEDS-ACTION"));
    }

    #[test]
    fn inject_partstat_inserts_new_attendee() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Test\nEND:VEVENT\nEND:VCALENDAR\n";
        let result = inject_partstat(ical, "ACCEPTED", Some("user@example.com"));
        assert!(result.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:user@example.com"));
    }

    // ── extract_event_fields ─────────────────────────────────────────────────

    #[test]
    fn extract_fields_vevent_full() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
SUMMARY:Meeting\n\
DESCRIPTION:Discuss plans\n\
LOCATION:Room 101\n\
DTSTART:20230601T120000Z\n\
DTEND:20230601T130000Z\n\
END:VEVENT\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.component_type, "VEVENT");
        assert_eq!(f.summary.as_deref(), Some("Meeting"));
        assert_eq!(f.description.as_deref(), Some("Discuss plans"));
        assert_eq!(f.location.as_deref(), Some("Room 101"));
        assert_eq!(f.start_date.as_deref(), Some("2023-06-01 12:00:00"));
        assert_eq!(f.end_date.as_deref(), Some("2023-06-01 13:00:00"));
    }

    #[test]
    fn extract_fields_vtodo_with_due() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VTODO\n\
SUMMARY:Task\n\
DUE:20230615T090000Z\n\
END:VTODO\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.component_type, "VTODO");
        assert_eq!(f.summary.as_deref(), Some("Task"));
        assert_eq!(f.due_date.as_deref(), Some("2023-06-15 09:00:00"));
    }

    #[test]
    fn extract_fields_dtstart_with_tzid() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
DTSTART;TZID=America/New_York:20230601T120000\n\
END:VEVENT\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.start_date.as_deref(), Some("2023-06-01 12:00:00"));
    }

    #[test]
    fn extract_fields_vtodo_status_completed() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VTODO\n\
SUMMARY:Task\n\
STATUS:COMPLETED\n\
END:VTODO\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.status.as_deref(), Some("COMPLETED"));
    }

    #[test]
    fn extract_fields_vtodo_status_lowercase_is_normalised() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VTODO\n\
STATUS:needs-action\n\
END:VTODO\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.status.as_deref(), Some("NEEDS-ACTION"));
    }

    #[test]
    fn extract_fields_vtodo_status_absent_is_none() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VTODO\n\
SUMMARY:Task\n\
END:VTODO\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.status, None);
    }

    #[test]
    fn extract_fields_unescapes_backslash_n() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
DESCRIPTION:Line1\\nLine2\n\
END:VEVENT\n\
END:VCALENDAR";
        let f = extract_event_fields(ical);
        assert_eq!(f.description.as_deref(), Some("Line1\nLine2"));
    }

    // ── sanitize_google_continuation_uid / rewrite_uid_in_ical ───────────────

    #[test]
    fn sanitize_google_continuation_uid_rewrites() {
        let uid = "meeting-abc123_R20231118T090000@google.com";
        let got = sanitize_google_continuation_uid(uid);
        assert_eq!(
            got,
            "meeting-abc123-20231118T090000@tidl.local",
        );
    }

    #[test]
    fn sanitize_google_continuation_uid_utc_z() {
        let uid = "base_R20231118T090000Z@google.com";
        let got = sanitize_google_continuation_uid(uid);
        // trailing Z stripped from timestamp
        assert_eq!(got, "base-20231118T090000@tidl.local");
    }

    #[test]
    fn sanitize_google_continuation_uid_plain_uid_unchanged() {
        let uid = "plain-event@google.com";
        assert_eq!(sanitize_google_continuation_uid(uid), uid);
    }

    #[test]
    fn sanitize_google_continuation_uid_regular_recurrence_unchanged() {
        // A uid that contains "_R" but without the timestamp+@ pattern
        let uid = "something_Rextra@example.com";
        assert_eq!(sanitize_google_continuation_uid(uid), uid);
    }

    #[test]
    fn rewrite_uid_in_ical_replaces_uid() {
        let ical = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:old@example.com\r\nSUMMARY:Test\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let result = rewrite_uid_in_ical(ical, "old@example.com", "new@tidl.local");
        assert!(result.contains("UID:new@tidl.local"), "got: {result}");
        assert!(!result.contains("UID:old@example.com"), "got: {result}");
    }

    #[test]
    fn rewrite_uid_in_ical_handles_folded_uid() {
        // UID folded across two lines as in a real-world ICS
        let ical = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:meeting-abc123_R20231118T09\r\n 0000@google.com\r\nSUMMARY:Test\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let old_uid = "meeting-abc123_R20231118T090000@google.com";
        let new_uid =
            "meeting-abc123-20231118T090000@tidl.local";
        let result = rewrite_uid_in_ical(ical, old_uid, new_uid);
        assert!(result.contains(&format!("UID:{new_uid}")), "got: {result}");
        assert!(
            !result.contains("_R20231118T09"),
            "old folded UID still present; got: {result}"
        );
    }
}

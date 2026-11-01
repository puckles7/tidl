//! VALARM parsing helpers — SSR-only, no web-sys deps.
//!
//! These functions are used by `get_events` to populate `Event::alarms` so
//! the edit dialog can pre-fill alarm fields from existing events.

use crate::types::Alarm;
use tidl_caldav::icalendar_utils::{
    ical_duration_str, parse_simple_duration, prop_value, prop_value_with_params, unfold_ical,
};

// ── VALARM block parser ───────────────────────────────────────────────────────

/// Parse the VALARM blocks in a VCALENDAR string into `Alarm` values.
///
/// Scoped to the VEVENT/VTODO component (falling back to the whole string if
/// neither is found) rather than unfolding the full blob. Google Calendar
/// exports carry an embedded VTIMEZONE ahead of the event, often several
/// times the byte size of the event itself — unfolding it just to find no
/// VALARM lines was a measured hotspot in `get_events` (every event, every
/// request, unconditionally).
pub fn parse_alarms_from_ical(ical: &str) -> Vec<Alarm> {
    let component = component_block(ical);
    let unfolded = unfold_ical(component);
    let mut alarms = Vec::new();
    let mut in_alarm = false;
    let mut lines: Vec<String> = Vec::new();

    for raw_line in unfolded.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.eq_ignore_ascii_case("BEGIN:VALARM") {
            in_alarm = true;
            lines.clear();
            continue;
        }
        if line.eq_ignore_ascii_case("END:VALARM") {
            in_alarm = false;
            if let Some(a) = build_alarm_from_lines(&lines) {
                alarms.push(a);
            }
            lines.clear();
            continue;
        }
        if in_alarm {
            lines.push(line.to_string());
        }
    }
    alarms
}

/// Return the `BEGIN:VEVENT`/`BEGIN:VTODO` component slice of `ical` (up to
/// but excluding its matching `END:` line), or the whole string if neither
/// component is found.
fn component_block(ical: &str) -> &str {
    for kind in ["VEVENT", "VTODO"] {
        let begin = format!("BEGIN:{kind}");
        if let Some(start) = ical.find(&begin) {
            let end = format!("END:{kind}");
            if let Some(rel_end) = ical[start..].find(&end) {
                return &ical[start..start + rel_end];
            }
        }
    }
    ical
}

/// `pub(crate)`: also called from `api::extract_event_ical_meta`, which
/// folds VALARM extraction into the same single unfolded-VEVENT scan used
/// for the all-day/RRULE checks rather than re-parsing via
/// `parse_alarms_from_ical` (see that function's call site in `events.rs`).
pub(crate) fn build_alarm_from_lines(lines: &[String]) -> Option<Alarm> {
    let mut action = String::from("display");
    let mut trigger_str = String::new();
    let mut trigger_line = String::new();
    let mut repeat: Option<u32> = None;
    let mut duration_m: Option<u32> = None;

    for line in lines {
        if let Some(v) = prop_value(line, "ACTION") {
            action = v.trim().to_lowercase();
        } else if let Some(v) = prop_value_with_params(line, "TRIGGER") {
            trigger_str = v.trim().to_string();
            trigger_line = line.to_ascii_uppercase();
        } else if let Some(v) = prop_value(line, "REPEAT") {
            repeat = v.trim().parse().ok();
        } else if let Some(v) = prop_value(line, "DURATION") {
            duration_m = parse_simple_duration(v.trim()).map(|m| m.unsigned_abs());
        }
    }

    if trigger_str.is_empty() {
        return None;
    }

    // Check if this is a trigger form we can represent in the UI.
    // RELATED=END and VALUE=DATE-TIME appear in the property parameters (before
    // the ':'), so we check the full TRIGGER line, not just the value part.
    let is_raw = trigger_line.contains("RELATED=END") || trigger_line.contains("VALUE=DATE-TIME");

    let (amount, unit, is_raw) = if is_raw {
        (0, "minutes".to_string(), true)
    } else {
        // Extract dur-value from trigger (may have params: TRIGGER;VALUE=DURATION:…)
        let dur = if let Some(pos) = trigger_str.rfind(':') {
            &trigger_str[pos + 1..]
        } else {
            &trigger_str
        };
        match parse_simple_duration(dur) {
            Some(mins) => {
                let m = mins.unsigned_abs();
                // Use ical_duration_str to pick the best unit for display.
                let repr = ical_duration_str(-(m as i32)); // negative = before start
                let (unit, divisor) = if repr.ends_with('W') {
                    ("weeks", 7 * 24 * 60)
                } else if repr.ends_with('D') {
                    ("days", 24 * 60)
                } else if repr.ends_with('H') {
                    ("hours", 60)
                } else {
                    ("minutes", 1)
                };
                (m / divisor, unit.to_string(), false)
            }
            None => (0, "minutes".to_string(), true),
        }
    };

    let repeat_count = repeat;
    let repeat_delay_m = if repeat.is_some() { duration_m } else { None };

    // Build a compact raw_ical string for round-tripping.
    let raw_ical: String = lines.iter().map(|l| format!("{}\n", l)).collect();

    Some(Alarm {
        action,
        amount,
        unit,
        repeat_count,
        repeat_delay_m,
        raw_ical: Some(raw_ical),
        is_raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_alarms_returns_empty() {
        let ical = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Test\nEND:VEVENT\nEND:VCALENDAR";
        assert!(parse_alarms_from_ical(ical).is_empty());
    }

    #[test]
    fn single_display_alarm_minutes() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
SUMMARY:Meeting\n\
BEGIN:VALARM\n\
ACTION:DISPLAY\n\
TRIGGER:-PT15M\n\
DESCRIPTION:Reminder\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        let alarms = parse_alarms_from_ical(ical);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].action, "display");
        assert_eq!(alarms[0].amount, 15);
        assert_eq!(alarms[0].unit, "minutes");
        assert!(!alarms[0].is_raw);
    }

    #[test]
    fn alarm_hours_unit() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
BEGIN:VALARM\n\
ACTION:DISPLAY\n\
TRIGGER:-PT2H\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        let alarms = parse_alarms_from_ical(ical);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].amount, 2);
        assert_eq!(alarms[0].unit, "hours");
    }

    #[test]
    fn alarm_with_repeat_and_duration() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
BEGIN:VALARM\n\
ACTION:AUDIO\n\
TRIGGER:-PT10M\n\
REPEAT:3\n\
DURATION:PT5M\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        let alarms = parse_alarms_from_ical(ical);
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].action, "audio");
        assert_eq!(alarms[0].repeat_count, Some(3));
        assert_eq!(alarms[0].repeat_delay_m, Some(5));
    }

    #[test]
    fn multiple_alarms() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
BEGIN:VALARM\n\
ACTION:DISPLAY\n\
TRIGGER:-PT30M\n\
END:VALARM\n\
BEGIN:VALARM\n\
ACTION:AUDIO\n\
TRIGGER:-PT5M\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        let alarms = parse_alarms_from_ical(ical);
        assert_eq!(alarms.len(), 2);
        assert_eq!(alarms[0].amount, 30);
        assert_eq!(alarms[1].amount, 5);
    }

    #[test]
    fn alarm_related_end_is_raw() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
BEGIN:VALARM\n\
ACTION:DISPLAY\n\
TRIGGER;RELATED=END:-PT15M\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        let alarms = parse_alarms_from_ical(ical);
        assert_eq!(alarms.len(), 1);
        assert!(alarms[0].is_raw);
    }

    #[test]
    fn alarm_missing_trigger_is_skipped() {
        let ical = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
BEGIN:VALARM\n\
ACTION:DISPLAY\n\
END:VALARM\n\
END:VEVENT\n\
END:VCALENDAR";
        assert!(parse_alarms_from_ical(ical).is_empty());
    }
}

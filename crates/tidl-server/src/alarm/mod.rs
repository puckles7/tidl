//! VALARM parsing, serialisation, and sub-action modules.
#![allow(dead_code)]
// ParsedAlarm/AlarmAction/AlarmTrigger are unused pending further integration; `dispatch` (below) is live
//!
//! Architecture
//! ────────────
//! `tidl_web::api::Alarm`  is the *shared* (WASM + SSR) type that the UI
//! and server functions exchange over the wire.
//!
//! `ParsedAlarm` in this module is the *server-only* richer representation
//! that carries everything the RFC 5545 VALARM component can express.
//!
//! Round-trip strategy
//! ───────────────────
//! • If the VALARM can be mapped losslessly to/from `Alarm`, we do that.
//! • Otherwise we store the raw lines verbatim in `Alarm::raw_ical` and
//!   emit them unchanged when rebuilding the iCal.

pub mod audio;
pub mod dispatch;
pub mod display;
pub mod email;

use tidl_caldav::icalendar_utils::{
    ical_duration_str, parse_simple_duration, prop_value, prop_value_with_params, unfold_ical,
};
use tidl_web::api::Alarm;

// ── ParsedAlarm ───────────────────────────────────────────────────────────────

/// Full server-side representation of a VALARM component.
#[derive(Debug, Clone)]
pub struct ParsedAlarm {
    pub action: AlarmAction,
    pub trigger: AlarmTrigger,
    /// Optional repetition: (repeat_count, delay_between_repetitions_in_minutes)
    pub repeat: Option<(u32, u32)>,
    /// Raw lines of the VALARM block (everything between BEGIN:VALARM and
    /// END:VALARM, exclusive).  Used for faithful round-tripping of fields we
    /// don't model (e.g. ATTACH, ATTENDEE, custom X- properties).
    pub raw_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlarmAction {
    Display,
    Audio,
    Email,
    /// Any other action value — pass through verbatim.
    Other(String),
}

impl AlarmAction {
    pub fn as_ical_str(&self) -> &str {
        match self {
            Self::Display => "DISPLAY",
            Self::Audio => "AUDIO",
            Self::Email => "EMAIL",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_uppercase().as_str() {
            "DISPLAY" => Self::Display,
            "AUDIO" => Self::Audio,
            "EMAIL" => Self::Email,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Which TRIGGER forms we fully understand.
#[derive(Debug, Clone)]
pub enum AlarmTrigger {
    /// `-PTnM`, `-PTnH`, `-PnD`, `-PnW` relative to DTSTART (most common).
    RelativeStart { minutes: i32 },
    /// Anything else we preserve verbatim (RELATED=END, absolute, combined).
    Raw(String),
}

impl AlarmTrigger {
    /// Parse a TRIGGER property value string (after the colon).
    /// Also accepts the full line `TRIGGER:…` or `TRIGGER;…:…`.
    pub fn parse(value: &str) -> Self {
        let v = value.trim();

        // Strip RELATED=START parameter (implicit default) so we treat it
        // the same as a bare relative trigger.
        // Reject RELATED=END and VALUE=DATE-TIME — fall back to Raw.
        if v.to_uppercase().contains("RELATED=END") || v.to_uppercase().contains("VALUE=DATE-TIME")
        {
            return Self::Raw(v.to_string());
        }

        // Extract the actual dur-value part (after any parameters).
        let dur = if let Some(pos) = v.rfind(':') {
            &v[pos + 1..]
        } else {
            v
        };

        if let Some(mins) = parse_simple_duration(dur) {
            Self::RelativeStart { minutes: mins }
        } else {
            Self::Raw(v.to_string())
        }
    }

    pub fn to_ical_value(&self) -> String {
        match self {
            Self::RelativeStart { minutes } => ical_duration_str(*minutes),
            Self::Raw(s) => s.clone(),
        }
    }
}

// ── Duration serialiser helper ────────────────────────────────────────────────

/// Convert UI `amount` + `unit` into total minutes (always negative = before start).
pub fn ui_to_minutes(amount: u32, unit: &str) -> i32 {
    let a = amount as i32;
    -(match unit {
        "hours" => a * 60,
        "days" => a * 24 * 60,
        "weeks" => a * 7 * 24 * 60,
        _ => a, // "minutes" default
    })
}

// ── VALARM → iCal string ──────────────────────────────────────────────────────

/// Serialise a `ParsedAlarm` back into a `BEGIN:VALARM … END:VALARM` block.
pub fn serialise_alarm(alarm: &ParsedAlarm) -> String {
    let mut out = String::from("BEGIN:VALARM\r\n");
    out.push_str(&format!("ACTION:{}\r\n", alarm.action.as_ical_str()));
    out.push_str(&format!("TRIGGER:{}\r\n", alarm.trigger.to_ical_value()));

    // For DISPLAY we always need DESCRIPTION.
    if alarm.action == AlarmAction::Display {
        // Preserve original DESCRIPTION if present in raw_lines, else default.
        let has_desc = alarm.raw_lines.iter().any(|l| l.starts_with("DESCRIPTION"));
        if !has_desc {
            out.push_str("DESCRIPTION:Reminder\r\n");
        }
    }

    // Emit REPEAT + DURATION pair if present.
    if let Some((count, delay_m)) = alarm.repeat {
        out.push_str(&format!("REPEAT:{}\r\n", count));
        out.push_str(&format!("DURATION:PT{}M\r\n", delay_m));
    }

    // Re-emit any raw lines we preserved (excluding lines we handle above).
    let skip_prefixes = [
        "ACTION:",
        "TRIGGER:",
        "REPEAT:",
        "DURATION:",
        "DESCRIPTION:",
    ];
    for line in &alarm.raw_lines {
        if !skip_prefixes.iter().any(|p| line.starts_with(p)) {
            out.push_str(line);
            out.push_str("\r\n");
        } else if line.starts_with("DESCRIPTION:") {
            // Re-emit the original DESCRIPTION (it was preserved).
            out.push_str(line);
            out.push_str("\r\n");
        }
    }

    out.push_str("END:VALARM\r\n");
    out
}

// ── iCal → ParsedAlarm ────────────────────────────────────────────────────────

/// Extract all VALARM blocks from a VCALENDAR string.
pub fn parse_valarms(ical: &str) -> Vec<ParsedAlarm> {
    let mut alarms = Vec::new();
    let mut in_alarm = false;
    let mut current_lines: Vec<String> = Vec::new();

    let unfolded = unfold_ical(ical);

    for line in unfolded.lines() {
        let line = line.trim_end_matches('\r');
        if line.eq_ignore_ascii_case("BEGIN:VALARM") {
            in_alarm = true;
            current_lines.clear();
            continue;
        }
        if line.eq_ignore_ascii_case("END:VALARM") {
            in_alarm = false;
            if let Some(alarm) = build_alarm(&current_lines) {
                alarms.push(alarm);
            }
            current_lines.clear();
            continue;
        }
        if in_alarm {
            current_lines.push(line.to_string());
        }
    }
    alarms
}

fn build_alarm(lines: &[String]) -> Option<ParsedAlarm> {
    let mut action_str = String::from("DISPLAY");
    let mut trigger_str = String::new();
    let mut repeat: Option<u32> = None;
    let mut duration_m: Option<u32> = None;

    for line in lines {
        if let Some(v) = prop_value(line, "ACTION") {
            action_str = v.to_string();
        } else if let Some(v) = prop_value_with_params(line, "TRIGGER") {
            trigger_str = v.to_string();
        } else if let Some(v) = prop_value(line, "REPEAT") {
            repeat = v.parse().ok();
        } else if let Some(v) = prop_value(line, "DURATION") {
            duration_m = parse_simple_duration(v).map(|m| m.unsigned_abs());
        }
    }

    if trigger_str.is_empty() {
        return None; // TRIGGER is required
    }

    let repeat_pair = match (repeat, duration_m) {
        (Some(r), Some(d)) => Some((r, d)),
        _ => None,
    };

    Some(ParsedAlarm {
        action: AlarmAction::from_str(&action_str),
        trigger: AlarmTrigger::parse(&trigger_str),
        repeat: repeat_pair,
        raw_lines: lines.to_vec(),
    })
}

// ── Alarm ↔ ParsedAlarm conversion ───────────────────────────────────────────

/// Convert a UI `Alarm` into a `ParsedAlarm` ready for serialisation.
pub fn alarm_from_ui(a: &Alarm) -> ParsedAlarm {
    if let Some(raw) = &a.raw_ical {
        // The alarm came from an external CalDAV client — re-parse so we can
        // merge any UI edits on top.
        let lines: Vec<String> = raw
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| {
                !l.is_empty()
                    && !l.eq_ignore_ascii_case("BEGIN:VALARM")
                    && !l.eq_ignore_ascii_case("END:VALARM")
            })
            .collect();
        if let Some(mut parsed) = build_alarm(&lines) {
            // Override with any UI edits.
            parsed.action = AlarmAction::from_str(&a.action);
            parsed.trigger = AlarmTrigger::RelativeStart {
                minutes: ui_to_minutes(a.amount, &a.unit),
            };
            if let (Some(rc), Some(rd)) = (a.repeat_count, a.repeat_delay_m) {
                parsed.repeat = Some((rc, rd));
            } else {
                parsed.repeat = None;
            }
            return parsed;
        }
    }

    ParsedAlarm {
        action: AlarmAction::from_str(&a.action),
        trigger: AlarmTrigger::RelativeStart {
            minutes: ui_to_minutes(a.amount, &a.unit),
        },
        repeat: if let (Some(rc), Some(rd)) = (a.repeat_count, a.repeat_delay_m) {
            Some((rc, rd))
        } else {
            None
        },
        raw_lines: Vec::new(),
    }
}

/// Convert a `ParsedAlarm` to a UI `Alarm`.
pub fn alarm_to_ui(parsed: &ParsedAlarm) -> Alarm {
    let (amount, unit) = match &parsed.trigger {
        AlarmTrigger::RelativeStart { minutes } => {
            let m = minutes.abs();
            let repr = ical_duration_str(-m); // negative = "before start" form
            let (unit, divisor) = if repr.ends_with('W') {
                ("weeks", 7 * 24 * 60)
            } else if repr.ends_with('D') {
                ("days", 24 * 60)
            } else if repr.ends_with('H') {
                ("hours", 60)
            } else {
                ("minutes", 1)
            };
            if m == 0 {
                (0u32, "minutes".to_string())
            } else {
                (m as u32 / divisor, unit.to_string())
            }
        }
        AlarmTrigger::Raw(_) => (0, "minutes".to_string()),
    };

    // Rebuild raw_ical for round-tripping.
    let raw_ical = serialise_alarm(parsed);
    // Strip BEGIN/END wrappers for the compact representation.
    let inner: String = raw_ical
        .lines()
        .filter(|l| {
            !l.trim().eq_ignore_ascii_case("BEGIN:VALARM")
                && !l.trim().eq_ignore_ascii_case("END:VALARM")
        })
        .map(|l| format!("{}\n", l.trim_end_matches('\r')))
        .collect();

    Alarm {
        action: parsed.action.as_ical_str().to_lowercase(),
        amount,
        unit,
        repeat_count: parsed.repeat.map(|(r, _)| r),
        repeat_delay_m: parsed.repeat.map(|(_, d)| d),
        raw_ical: Some(inner),
        is_raw: matches!(parsed.trigger, AlarmTrigger::Raw(_)),
    }
}

//! Server functions for the web UI.
//!
//! The `#[server]` macro generates:
//! - A client-side stub (WASM) that POSTs to `/api/<FnName>`
//! - A server-side handler (SSR) that runs the body directly.
//!
//! The SSR body extracts the `sqlx::SqlitePool` from the Axum `Extension`
//! layer registered in `tidl-server/src/main.rs`.
//!
//! Shared data types live in `crate::types` and are re-exported here so
//! existing `use crate::api::TypeName` imports continue to work.

mod admin;
mod auth;
mod calendars;
mod consolidate;
mod contacts;
pub(crate) mod done;
mod events;
mod files;
mod focus;
mod keep;
mod notes;
mod oidc;
mod profile;
mod settings;
mod sharing;

pub use admin::*;
pub use auth::*;
pub use calendars::*;
pub use consolidate::*;
pub use contacts::*;
pub use done::*;
pub use events::*;
pub use files::*;
pub use focus::*;
pub use keep::*;
pub use notes::*;
pub use oidc::get_oidc_config;
pub use profile::*;
pub use settings::*;

#[cfg(feature = "ssr")]
use leptos::prelude::ServerFnError;

// Re-export all shared types so callers using `crate::api::TypeName` continue
// to work without change.
pub use crate::types::{
    Alarm, AppTokenInfo, AwaitItem, CalendarInfo, ContactAddress, ContactCollectionInfo,
    ContactDto, ContactMethod, CurvePoint, DirEntryInfo, DirListing, DoneFilter, DoneItem,
    DoneScope, DoneSettings, Event, GridPosition, ImportResult, InboundEmailTokenInfo,
    KeepImportResult, MarkdownSourceInfo, NewContact, NewContactCollection, NewEvent, NewGitSource,
    NewMarkdownSource, NewNote, NewNoteCollection, NewNoteImage, NewRemoteCalDavSource,
    NoteCollectionInfo, NoteImageInfo, NoteInfo, NoteLabelInfo, PriorityCurves, ProfileInfo,
    PurgeGroup, PurgePreview, PurgeResult, RemoteCalDavSource, Retention, SessionInfo, ShareInfo,
    SharedCalendar, SmtpConfig, Task, TodoChip, TodoOrigin, TrustedSenderInfo,
    UpdateRemoteCalDavSource, UserAdminInfo, UserDeletionSummary, UserPrefs, VcfImportPreview,
    MIN_SYNC_INTERVAL_SECS,
};

#[cfg(feature = "ssr")]
pub(crate) use tidl_caldav::change::ChangeKind;

// ── Session helpers (SSR-only) ────────────────────────────────────────────────

/// Extract the authenticated username from the session cookie.
/// Returns an error if the session is missing or invalid.
#[cfg(feature = "ssr")]
pub(super) async fn current_user(
    pool: &sqlx::SqlitePool,
    headers: &axum::http::HeaderMap,
) -> Result<String, ServerFnError> {
    use axum::http::header::COOKIE;
    use axum::Extension;
    use sqlx::Row;

    let Extension(cookie_name): Extension<crate::ssr_types::SessionCookieName> =
        leptos_axum::extract().await?;

    let cookies = headers
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = cookie_name
        .extract(cookies)
        .ok_or_else(|| ServerFnError::new("Not authenticated"))?;

    let row = sqlx::query(
        "SELECT username FROM sessions WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(&token)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    row.and_then(|r| r.try_get::<String, _>("username").ok())
        .ok_or_else(|| ServerFnError::new("Session invalid or expired"))
}

/// Like [`current_user`] but additionally verifies `is_admin = 1`.
/// Returns the username on success, or an error if the user is not an admin.
#[cfg(feature = "ssr")]
pub(super) async fn require_admin(
    pool: &sqlx::SqlitePool,
    headers: &axum::http::HeaderMap,
) -> Result<String, ServerFnError> {
    use sqlx::Row;
    let username = current_user(pool, headers).await?;
    let row = sqlx::query("SELECT is_admin FROM users WHERE username = ? AND is_active = 1")
        .bind(&username)
        .fetch_optional(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let is_admin: i64 = row
        .and_then(|r| r.try_get::<i64, _>("is_admin").ok())
        .unwrap_or(0);
    if is_admin == 0 {
        return Err(ServerFnError::new("Admin access required"));
    }
    Ok(username)
}

/// Create a new session for `username`, INSERT it into the DB, and append the
/// `Set-Cookie` header to the current Leptos response.
///
/// Extracted from the three "log the user in immediately" blocks that appeared
/// in `do_login`, `do_verify_email`, and `do_reset_password`.
#[cfg(feature = "ssr")]
pub(super) async fn create_and_set_session(
    username: &str,
    pool: &sqlx::SqlitePool,
    opts: &leptos_axum::ResponseOptions,
) -> Result<(), ServerFnError> {
    use axum::http::{header, HeaderValue};
    use axum::Extension;

    let Extension(cookie_name): Extension<crate::ssr_types::SessionCookieName> =
        leptos_axum::extract().await?;

    let token = uuid::Uuid::new_v4().to_string();
    const TTL: i64 = 86_400;
    let expires_modifier = format!("+{TTL} seconds");
    sqlx::query(
        "INSERT INTO sessions (token, username, expires_at) \
         VALUES (?, ?, datetime('now', ?))",
    )
    .bind(&token)
    .bind(username)
    .bind(&expires_modifier)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let cookie = cookie_name.set_header_value(&token, TTL);
    opts.append_header(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|e| ServerFnError::new(e.to_string()))?,
    );
    Ok(())
}

/// Extract the authenticated user's pool and username from the current request context.
///
/// Combines the three boilerplate lines that appear at the top of every
/// authenticated server function: pool extraction, header extraction, and
/// session validation.  Returns `(pool, username)`.
#[cfg(feature = "ssr")]
pub(super) async fn auth() -> Result<(sqlx::SqlitePool, String), ServerFnError> {
    use axum::http::HeaderMap;
    use axum::Extension;
    use leptos_axum::extract;
    use sqlx::SqlitePool;
    let Extension(pool): Extension<SqlitePool> = extract().await?;
    let headers: HeaderMap = extract().await?;
    let username = current_user(&pool, &headers).await?;
    Ok((pool, username))
}

/// Like [`auth`] but also returns the caller's own session token, for
/// operations that need to distinguish "this session" from "other sessions
/// for this user" (e.g. revoking every other session on password change).
#[cfg(feature = "ssr")]
pub(super) async fn auth_with_token() -> Result<(sqlx::SqlitePool, String, String), ServerFnError> {
    use axum::http::{header::COOKIE, HeaderMap};
    use axum::Extension;
    use leptos_axum::extract;
    use sqlx::SqlitePool;

    let Extension(pool): Extension<SqlitePool> = extract().await?;
    let headers: HeaderMap = extract().await?;
    let username = current_user(&pool, &headers).await?;

    let Extension(cookie_name): Extension<crate::ssr_types::SessionCookieName> = extract().await?;
    let token = cookie_name
        .extract(
            headers
                .get(COOKIE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
        .ok_or_else(|| ServerFnError::new("Not authenticated"))?;

    Ok((pool, username, token))
}

/// Like [`auth`] but additionally verifies the user has `is_admin = 1`.
#[cfg(feature = "ssr")]
pub(super) async fn auth_admin() -> Result<(sqlx::SqlitePool, String), ServerFnError> {
    use axum::http::HeaderMap;
    use axum::Extension;
    use leptos_axum::extract;
    use sqlx::SqlitePool;
    let Extension(pool): Extension<SqlitePool> = extract().await?;
    let headers: HeaderMap = extract().await?;
    let username = require_admin(&pool, &headers).await?;
    Ok((pool, username))
}

/// Fire a change notification so connected browsers refetch data for `username`.
/// Non-fatal: if no SSE subscribers are connected the broadcast receiver count
/// is zero and `send` returns an error, which we silently discard.
#[cfg(feature = "ssr")]
pub(super) async fn notify_change(username: &str, kind: tidl_caldav::change::ChangeKind) {
    use axum::Extension;
    use leptos_axum::extract;
    use tokio::sync::broadcast;
    if let Ok(Extension(tx)) = extract::<Extension<broadcast::Sender<String>>>().await {
        let _ = tx.send(kind.to_msg(username));
    }
}

// ── Private iCal helpers ────────────────────────────────────────────────────

/// Extract the value of the `RECURRENCE-ID` property from a wrapped
/// VCALENDAR/VEVENT string, returning `None` when absent.
///
/// RFC 5545: a VEVENT that carries `RECURRENCE-ID` is an override for a single
/// occurrence within a recurring series, not the master event itself.
#[cfg(feature = "ssr")]
pub(super) fn extract_recurrence_id(ical_str: &str) -> Option<String> {
    use tidl_caldav::icalendar_utils::{prop_value_with_params, unfold_ical};
    let unfolded = unfold_ical(ical_str);
    let mut in_vevent = false;
    for line in unfolded.lines() {
        let up = line.to_ascii_uppercase();
        if up == "BEGIN:VEVENT" {
            in_vevent = true;
            continue;
        }
        if up == "END:VEVENT" {
            break;
        }
        if !in_vevent {
            continue;
        }
        if let Some(v) = prop_value_with_params(line, "RECURRENCE-ID") {
            let rid = v.trim().trim_end_matches('Z').to_string();
            if !rid.is_empty() {
                return Some(rid);
            }
        }
    }
    None
}

/// Parse a SQLite datetime string (`"YYYY-MM-DD HH:MM[:SS]"`) or a bare date
/// (`"YYYY-MM-DD"`, treated as midnight) into a tz-aware `DateTime<Utc>` using
/// an IANA timezone name. Returns `None` on any parse/conversion failure.
#[cfg(feature = "ssr")]
pub(super) fn tz_aware_datetime_utc(
    s: &str,
    tz_name: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDateTime, TimeZone};
    use chrono_tz::Tz;
    use std::str::FromStr;
    let s = s.trim();
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M"))
        .or_else(|_| NaiveDateTime::parse_from_str(&format!("{s} 00:00:00"), "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    let tz = Tz::from_str(tz_name).unwrap_or(chrono_tz::UTC);
    tz.from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Convert a SQLite datetime string to UTC epoch milliseconds using an IANA timezone.
/// Returns 0 on any parse/conversion failure.
#[cfg(feature = "ssr")]
pub(super) fn tz_aware_epoch_ms(s: &str, tz_name: &str) -> i64 {
    tz_aware_datetime_utc(s, tz_name)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

/// Expand a recurring VEVENT into one `Event` per occurrence.
///
/// Only occurrences overlapping `[range_start, range_end]` are returned —
/// an occurrence is included when its start is on or before `range_end` and
/// its end (start + duration) is on or after `range_start`, so multi-day/
/// all-day events that begin just before the range still appear.
///
/// Returns an empty `Vec` when the event has no RRULE, when the RRULE string
/// cannot be parsed, or when no occurrences overlap the range.
/// The caller should fall back to a single-event representation in that case.
///
/// `exception_dates` is the set of recurrence-IDs (compact format, e.g.
/// `"20260504T180000"`) for which an exception row already exists in the DB.
/// Occurrences whose key is in this set are skipped so the exception row's
/// (possibly rescheduled) event takes precedence and no phantom duplicate appears.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)] // one caller; params are distinct SQL row fields, not a bundle
pub(super) fn expand_rrule(
    uid: &str,
    summary: &str,
    calendar: &str,
    calendar_color: &Option<String>,
    description: &Option<String>,
    alarms: &[Alarm],
    all_day: bool,
    raw_start: &str,
    raw_end: &str,
    ical_data: &str,
    exception_dates: &std::collections::HashSet<String>,
    timezone: &str,
    range_start: chrono::DateTime<chrono::Utc>,
    range_end: chrono::DateTime<chrono::Utc>,
) -> Vec<Event> {
    use chrono::NaiveDateTime;
    use rrule::RRuleSet;
    use tidl_caldav::icalendar_utils::unfold_ical;

    let unfolded = unfold_ical(ical_data);

    let mut dtstart_line: Option<String> = None;
    let mut rrule_lines: Vec<String> = Vec::new();
    let mut exdate_lines: Vec<String> = Vec::new();

    let mut in_vevent = false;
    for line in unfolded.lines() {
        let up = line.to_ascii_uppercase();
        if up == "BEGIN:VEVENT" {
            in_vevent = true;
            continue;
        }
        if up == "END:VEVENT" {
            break;
        }
        if !in_vevent {
            continue;
        }

        if up.starts_with("DTSTART") && dtstart_line.is_none() {
            dtstart_line = Some(line.to_string());
        } else if up.starts_with("RRULE:") {
            rrule_lines.push(line.to_string());
        } else if up.starts_with("EXDATE") {
            exdate_lines.push(line.to_string());
        }
    }

    let Some(dtstart_line) = dtstart_line else {
        return vec![];
    };
    if rrule_lines.is_empty() {
        return vec![];
    }

    // Extract the RRULE value (without "RRULE:" prefix) for the Event.rrule field.
    let rrule_value: Option<String> = rrule_lines.first().and_then(|l| {
        let up = l.to_ascii_uppercase();
        if up.starts_with("RRULE:") {
            Some(l[6..].to_string())
        } else {
            None
        }
    });

    // Build the string the rrule crate expects.
    let mut rrule_input = dtstart_line;
    for l in &rrule_lines {
        rrule_input.push('\n');
        rrule_input.push_str(l);
    }
    for l in &exdate_lines {
        rrule_input.push('\n');
        rrule_input.push_str(l);
    }

    let rrule_set: RRuleSet = match rrule_input.parse() {
        Ok(rs) => rs,
        Err(_) => return vec![],
    };

    // Compute event duration from the stored start/end (SQLite "YYYY-MM-DD HH:MM:SS").
    let fmt = "%Y-%m-%d %H:%M:%S";
    let duration_mins: i64 = {
        let parse = |s: &str| {
            NaiveDateTime::parse_from_str(s, fmt)
                .or_else(|_| {
                    let ds = if s.len() >= 10 { &s[..10] } else { s };
                    NaiveDateTime::parse_from_str(&format!("{} 00:00:00", ds), fmt)
                })
                .ok()
        };
        match (parse(raw_start), parse(raw_end)) {
            (Some(s), Some(e)) => (e - s).num_minutes().max(0),
            _ => 60,
        }
    };

    // Bound expansion to occurrences overlapping [range_start, range_end].
    // skip_while walks occurrences one at a time from DTSTART — cheap even
    // across decades for realistic (daily-or-coarser) frequencies.
    rrule_set
        .into_iter()
        .skip_while(|dt| {
            dt.with_timezone(&chrono::Utc) + chrono::Duration::minutes(duration_mins) < range_start
        })
        .take_while(|dt| dt.with_timezone(&chrono::Utc) <= range_end)
        .filter_map(|dt| {
            // Compact occurrence key — matches format from extract_recurrence_id.
            let occurrence_start = if all_day {
                dt.format("%Y%m%d").to_string()
            } else {
                dt.format("%Y%m%dT%H%M%S").to_string()
            };

            // Skip occurrences covered by an exception row in the DB.
            if exception_dates.contains(&occurrence_start) {
                return None;
            }

            let (start, end) = if all_day {
                let s = dt.format("%Y-%m-%d").to_string();
                let end_naive = dt.naive_local() + chrono::Duration::minutes(duration_mins);
                let e = date_str_minus_one(&end_naive.format("%Y-%m-%d").to_string());
                (s, e)
            } else {
                let s = dt.format("%Y-%m-%d %H:%M").to_string();
                let e = (dt + chrono::Duration::minutes(duration_mins))
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
                (s, e)
            };
            let start_epoch_ms = tz_aware_epoch_ms(&start, timezone);

            Some(Event {
                id: uid.to_string(),
                summary: summary.to_string(),
                start,
                end,
                all_day,
                description: description.clone(),
                priority: None,
                alarms: alarms.to_vec(),
                calendar: calendar.to_string(),
                calendar_color: calendar_color.clone(),
                rrule: rrule_value.clone(),
                occurrence_start: Some(occurrence_start),
                is_exception: false,
                start_epoch_ms,
                owner_username: String::new(),
                shared_permission: None,
            })
        })
        .collect()
}

/// "YYYY-MM-DD HH:MM" → "YYYYMMDDTHHMMSS" (iCal datetime format).
#[cfg(feature = "ssr")]
pub(super) fn fmt_ical_dt(s: &str) -> String {
    let s = s.trim();
    if s.len() < 16 {
        return String::new();
    }
    format!(
        "{}{}{}T{}{}00",
        &s[0..4],
        &s[5..7],
        &s[8..10],
        &s[11..13],
        &s[14..16]
    )
}

/// Return the `BEGIN:VEVENT`/`END:VEVENT` slice of `ical`, or `None` if the
/// event has no VEVENT component.
///
/// Scoping to the VEVENT matters for every helper below: an embedded
/// VTIMEZONE (present in nearly every Google Calendar export, to describe
/// DST transition rules) is typically several times larger than the VEVENT
/// it precedes, and carries its own DTSTART/RRULE lines that read as false
/// positives if a helper scans the whole `ical` blob instead of just the
/// event's own component. Measured impact: scanning the VTIMEZONE preamble
/// for no reason was driving ~190ms of ~230ms per `get_events` call.
#[cfg(feature = "ssr")]
fn vevent_block(ical: &str) -> Option<&str> {
    let start = ical.find("BEGIN:VEVENT")?;
    let rel_end = ical[start..].find("END:VEVENT")?;
    Some(&ical[start..start + rel_end])
}

/// All-day detection (DTSTART is a DATE value — `DTSTART;VALUE=DATE:20260426`
/// or `DTSTART:20260426`) plus VALARM extraction, from a single unfolded
/// scan of `ical`'s VEVENT block.
///
/// Combines what used to be separate `is_all_day_ical` + a
/// `parse_alarms_from_ical` call, each of which independently located the
/// VEVENT block and unfolded it from scratch. `event_row_to_web_events`
/// calls this only for rows that survive its range check (or are
/// recurring), so on a calendar with a few thousand events this now runs
/// for a small fraction of rows instead of every one.
#[cfg(feature = "ssr")]
pub(super) fn extract_all_day_and_alarms(ical: &str) -> (bool, Vec<Alarm>) {
    use tidl_caldav::icalendar_utils::unfold_ical;

    let Some(block) = vevent_block(ical) else {
        return (false, Vec::new());
    };

    let mut all_day = false;
    let mut dtstart_seen = false;
    let mut alarms = Vec::new();
    let mut in_alarm = false;
    let mut alarm_lines: Vec<String> = Vec::new();

    for raw_line in unfold_ical(block).lines() {
        let line = raw_line.trim_end_matches('\r');

        if line.eq_ignore_ascii_case("BEGIN:VALARM") {
            in_alarm = true;
            alarm_lines.clear();
            continue;
        }
        if line.eq_ignore_ascii_case("END:VALARM") {
            in_alarm = false;
            if let Some(a) = crate::alarm_parse::build_alarm_from_lines(&alarm_lines) {
                alarms.push(a);
            }
            alarm_lines.clear();
            continue;
        }
        if in_alarm {
            alarm_lines.push(line.to_string());
            continue;
        }

        if !dtstart_seen && line.to_ascii_uppercase().starts_with("DTSTART") {
            dtstart_seen = true;
            if let Some(colon) = line.rfind(':') {
                let val = line[colon + 1..].trim();
                // DATE-only value: exactly 8 ASCII digits, no 'T'.
                all_day = val.len() == 8 && val.bytes().all(|b| b.is_ascii_digit());
            }
        }
    }

    (all_day, alarms)
}

/// Return true if the VEVENT component of `ical` itself contains an RRULE
/// line — as opposed to `ical` merely containing the substring "RRULE:"
/// anywhere (see `vevent_block` for why that distinction matters).
#[cfg(feature = "ssr")]
pub(super) fn vevent_has_rrule(ical: &str) -> bool {
    use tidl_caldav::icalendar_utils::unfold_ical;
    let Some(block) = vevent_block(ical) else {
        return false;
    };
    unfold_ical(block)
        .lines()
        .any(|line| line.to_ascii_uppercase().starts_with("RRULE:"))
}

/// Add one calendar day to a `"YYYY-MM-DD"` date string.
///
/// Used to convert an inclusive UI end date to the exclusive DTEND required by
/// iCalendar (e.g. "2026-04-26" → "2026-04-27").
#[cfg(feature = "ssr")]
pub(super) fn date_str_plus_one(s: &str) -> String {
    if s.len() < 10 {
        return s.to_string();
    }
    let y: i32 = match s[0..4].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let m: u32 = match s[5..7].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let d: u32 = match s[8..10].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let last = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    };
    let (ny, nm, nd) = if d < last {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    };
    format!("{:04}-{:02}-{:02}", ny, nm, nd)
}

/// Build the VCALENDAR/VEVENT iCal string for a `NewEvent`.
///
/// Separated from both `create_event` and `update_event` because the two
/// callers share identical iCal construction logic — they only differ in how
/// they produce `uid_line` and `dtstamp_line`.
///
/// Returns `(ical, start_dt, end_dt)` where `start_dt`/`end_dt` are the
/// datetime strings suitable for storing in the `events` table.
#[cfg(feature = "ssr")]
// The Leptos #[server] macro rewrites call sites; rustc cannot see them.
#[allow(dead_code)]
pub(super) fn build_event_ical(
    event: &NewEvent,
    uid_line: &str,
    dtstamp_line: &str,
    alarm_ical: &str,
) -> (String, String, String) {
    let (ical_dtstart, ical_dtend, start_dt, end_dt) = if event.all_day {
        let fmt_d = |s: &str| -> String {
            let s = s.trim();
            if s.len() < 10 {
                return String::new();
            }
            format!("{}{}{}", &s[0..4], &s[5..7], &s[8..10])
        };
        let ds = fmt_d(&event.start);
        let de = fmt_d(&date_str_plus_one(&event.end)); // exclusive DTEND
        (
            format!("DTSTART;VALUE=DATE:{ds}"),
            format!("DTEND;VALUE=DATE:{de}"),
            format!("{} 00:00:00", event.start.trim()),
            format!("{} 00:00:00", date_str_plus_one(&event.end)),
        )
    } else {
        (
            format!("DTSTART:{}", fmt_ical_dt(&event.start)),
            format!("DTEND:{}", fmt_ical_dt(&event.end)),
            format!("{}:00", event.start.trim()),
            format!("{}:00", event.end.trim()),
        )
    };

    let mut ical = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
         BEGIN:VEVENT\r\n{uid_line}\r\n{dtstamp_line}\r\n\
         {dtstart}\r\n{dtend}\r\nSUMMARY:{summary}\r\n",
        dtstart = ical_dtstart,
        dtend = ical_dtend,
        summary = event.summary.replace('\n', "\\n"),
    );
    if let Some(priority) = event.priority {
        ical.push_str(&format!(
            "PRIORITY:{}\r\n",
            crate::types::source_priority_to_caldav(priority)
        ));
    }
    if let Some(ref rrule) = event.rrule {
        if !rrule.trim().is_empty() {
            ical.push_str(&format!("RRULE:{}\r\n", rrule.trim()));
        }
    }
    if let Some(ref loc) = event.location {
        if !loc.is_empty() {
            ical.push_str(&format!("LOCATION:{}\r\n", loc.replace('\n', "\\n")));
        }
    }
    if let Some(ref desc) = event.description {
        if !desc.is_empty() {
            ical.push_str(&format!("DESCRIPTION:{}\r\n", desc.replace('\n', "\\n")));
        }
    }
    ical.push_str(alarm_ical);
    ical.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");

    (ical, start_dt, end_dt)
}

/// Insert an `EXDATE` line into a master VEVENT iCal blob, immediately before
/// `END:VEVENT`.  `occurrence_start` must be in compact iCal format
/// (`"YYYYMMDD"` for all-day or `"YYYYMMDDTHHmmSS"` for timed).
#[cfg(feature = "ssr")]
pub(super) fn add_exdate_to_master(ical: &str, occurrence_start: &str) -> String {
    let exdate_line = if occurrence_start.len() == 8 {
        // All-day: 8-digit date value.
        format!("EXDATE;VALUE=DATE:{}\r\n", occurrence_start)
    } else {
        // Timed: YYYYMMDDTHHmmSS
        format!("EXDATE:{}\r\n", occurrence_start)
    };
    ical.replacen("END:VEVENT", &format!("{}END:VEVENT", exdate_line), 1)
}

/// Subtract one calendar day from a `"YYYY-MM-DD"` date string.
///
/// Used to convert an iCalendar exclusive DTEND to an inclusive end date for
/// the UI (e.g. "2026-04-27" → "2026-04-26").
#[cfg(feature = "ssr")]
pub(super) fn date_str_minus_one(s: &str) -> String {
    if s.len() < 10 {
        return s.to_string();
    }
    let y: i32 = match s[0..4].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let m: u32 = match s[5..7].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let d: u32 = match s[8..10].parse() {
        Ok(v) => v,
        _ => return s.to_string(),
    };
    let (ny, nm, nd) = if d > 1 {
        (y, m, d - 1)
    } else {
        let (py, pm) = if m == 1 { (y - 1, 12u32) } else { (y, m - 1) };
        let last = match pm {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if (py % 4 == 0 && py % 100 != 0) || py % 400 == 0 {
                    29
                } else {
                    28
                }
            }
            _ => 30,
        };
        (py, pm, last)
    };
    format!("{:04}-{:02}-{:02}", ny, nm, nd)
}

/// Serialise a slice of `Alarm` values into concatenated VALARM iCal blocks.
///
/// If the alarm carries a `raw_ical` inner block, that is re-emitted verbatim
/// (with `ACTION:` and `TRIGGER:` replaced by any UI edits) for lossless
/// round-tripping of external CalDAV client alarms.
#[cfg(feature = "ssr")]
pub(super) fn build_alarms_ical(alarms: &[Alarm]) -> String {
    use tidl_caldav::icalendar_utils::ical_duration_str;

    let mut out = String::new();

    for alarm in alarms {
        let total_mins: i32 = {
            let m = alarm.amount as i32;
            match alarm.unit.as_str() {
                "hours" => m * 60,
                "days" => m * 24 * 60,
                "weeks" => m * 7 * 24 * 60,
                _ => m, // "minutes"
            }
        };
        // Always emit as "before start" (negative).
        let trigger = ical_duration_str(-total_mins);
        let action = alarm.action.to_uppercase();

        let managed_prefixes = ["ACTION:", "TRIGGER:", "REPEAT:", "DURATION:"];

        out.push_str("BEGIN:VALARM\r\n");
        out.push_str(&format!("ACTION:{action}\r\n"));
        out.push_str(&format!("TRIGGER:{trigger}\r\n"));

        if let (Some(rc), Some(rd)) = (alarm.repeat_count, alarm.repeat_delay_m) {
            out.push_str(&format!("REPEAT:{rc}\r\n"));
            out.push_str(&format!("DURATION:PT{rd}M\r\n"));
        }

        let mut has_description = false;
        if let Some(raw) = &alarm.raw_ical {
            for line in raw.lines() {
                let l = line.trim_end_matches('\r');
                if l.is_empty()
                    || l.eq_ignore_ascii_case("BEGIN:VALARM")
                    || l.eq_ignore_ascii_case("END:VALARM")
                    || managed_prefixes
                        .iter()
                        .any(|p| l.to_uppercase().starts_with(p))
                {
                    continue;
                }
                if l.to_uppercase().starts_with("DESCRIPTION:") {
                    has_description = true;
                }
                out.push_str(l);
                out.push_str("\r\n");
            }
        }

        if action == "DISPLAY" && !has_description {
            out.push_str("DESCRIPTION:Reminder\r\n");
        }

        out.push_str("END:VALARM\r\n");
    }

    out
}

#[cfg(all(test, feature = "ssr"))]
mod rrule_tests {
    use super::expand_rrule;
    use chrono::{DateTime, TimeZone, Utc};

    /// A range wide enough (1970–2100) to cover every fixed test date below
    /// without the range bound itself affecting which occurrences appear.
    fn wide_range() -> (DateTime<Utc>, DateTime<Utc>) {
        (
            Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
        )
    }

    #[test]
    fn build_event_ical_emits_rrule() {
        use super::{build_event_ical, NewEvent};

        let event = NewEvent {
            summary: "Weekly".to_string(),
            start: "2026-06-01 09:00".to_string(),
            end: "2026-06-01 10:00".to_string(),
            all_day: false,
            location: None,
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".to_string(),
            rrule: Some("FREQ=WEEKLY;BYDAY=MO".to_string()),
            calendar_owner: None,
        };
        let (ical, _, _) = build_event_ical(&event, "UID:test-uid", "DTSTAMP:20260601T090000", "");
        assert!(
            ical.contains("RRULE:FREQ=WEEKLY;BYDAY=MO\r\n"),
            "RRULE line missing in output:\n{}",
            ical
        );
    }

    #[test]
    fn build_event_ical_no_rrule_when_none() {
        use super::{build_event_ical, NewEvent};

        let event = NewEvent {
            summary: "Single".to_string(),
            start: "2026-06-01 09:00".to_string(),
            end: "2026-06-01 10:00".to_string(),
            all_day: false,
            location: None,
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".to_string(),
            rrule: None,
            calendar_owner: None,
        };
        let (ical, _, _) = build_event_ical(&event, "UID:test-uid", "DTSTAMP:20260601T090000", "");
        assert!(
            !ical.contains("RRULE:"),
            "RRULE must not appear when rrule is None:\n{}",
            ical
        );
    }

    #[test]
    fn build_event_ical_emits_priority_when_set() {
        use super::{build_event_ical, NewEvent};

        let event = NewEvent {
            summary: "Prioritised".to_string(),
            start: "2026-06-01 09:00".to_string(),
            end: "2026-06-01 10:00".to_string(),
            all_day: false,
            location: None,
            description: None,
            priority: Some(100),
            alarms: vec![],
            calendar: "personal".to_string(),
            rrule: None,
            calendar_owner: None,
        };
        let (ical, _, _) = build_event_ical(&event, "UID:test-uid", "DTSTAMP:20260601T090000", "");
        assert!(
            ical.contains("PRIORITY:9\r\n"),
            "lowest canonical priority must project to RFC 5545 PRIORITY:9:\n{ical}"
        );
    }

    #[test]
    fn build_event_ical_omits_priority_when_unset() {
        use super::{build_event_ical, NewEvent};

        let event = NewEvent {
            summary: "Unprioritised".to_string(),
            start: "2026-06-01 09:00".to_string(),
            end: "2026-06-01 10:00".to_string(),
            all_day: false,
            location: None,
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".to_string(),
            rrule: None,
            calendar_owner: None,
        };
        let (ical, _, _) = build_event_ical(&event, "UID:test-uid", "DTSTAMP:20260601T090000", "");
        assert!(
            !ical.contains("PRIORITY:"),
            "PRIORITY must be absent:\n{ical}"
        );
    }

    #[test]
    fn add_exdate_to_master_timed() {
        use super::add_exdate_to_master;

        let ical = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nRRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let result = add_exdate_to_master(ical, "20260504T180000");
        assert!(
            result.contains("EXDATE:20260504T180000\r\n"),
            "EXDATE line missing:\n{}",
            result
        );
        assert!(
            result.contains("EXDATE:20260504T180000\r\nEND:VEVENT"),
            "EXDATE must appear immediately before END:VEVENT:\n{}",
            result
        );
    }

    #[test]
    fn add_exdate_to_master_allday() {
        use super::add_exdate_to_master;

        let ical = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let result = add_exdate_to_master(ical, "20260405");
        assert!(
            result.contains("EXDATE;VALUE=DATE:20260405\r\n"),
            "All-day EXDATE line missing:\n{}",
            result
        );
    }

    #[test]
    fn update_event_occurrence_builds_exception_ical() {
        // Verify that build_event_ical + RECURRENCE-ID insertion produces a valid exception VEVENT.
        use super::{build_event_ical, NewEvent};

        let event = NewEvent {
            summary: "Exception".to_string(),
            start: "2026-05-04 18:00".to_string(),
            end: "2026-05-04 19:00".to_string(),
            all_day: false,
            location: None,
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".to_string(),
            rrule: None, // exceptions never carry RRULE
            calendar_owner: None,
        };
        let (mut ical, _, _) = build_event_ical(
            &event,
            "UID:base-uid_R20260504T180000",
            "DTSTAMP:20260504T180000",
            "",
        );
        // Insert RECURRENCE-ID before END:VEVENT
        ical = ical.replacen(
            "END:VEVENT",
            "RECURRENCE-ID:20260504T180000\r\nEND:VEVENT",
            1,
        );

        assert!(
            ical.contains("RECURRENCE-ID:20260504T180000\r\n"),
            "RECURRENCE-ID missing:\n{}",
            ical
        );
        assert!(
            !ical.contains("RRULE:"),
            "exception VEVENT must not contain RRULE:\n{}",
            ical
        );
    }

    #[test]
    fn weekly_occurrence_may18_present() {
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
EXDATE;TZID=Europe/Berlin:20260427T180000\r\n\
EXDATE;TZID=Europe/Berlin:20260504T180000\r\n\
SUMMARY:Weekly Class\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = expand_rrule(
            "uid-test",
            "Weekly Class",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        assert!(!events.is_empty(), "expand_rrule returned nothing");
        let may18 = events.iter().find(|e| e.start.starts_with("2026-05-18"));
        assert!(
            may18.is_some(),
            "no occurrence on 2026-05-18; got starts: {:?}",
            events.iter().map(|e| &e.start).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exdate_occurrences_excluded() {
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
EXDATE;TZID=Europe/Berlin:20260427T180000\r\n\
EXDATE;TZID=Europe/Berlin:20260504T180000\r\n\
SUMMARY:Weekly Class\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = expand_rrule(
            "uid-test",
            "Weekly Class",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        let apr27 = events.iter().find(|e| e.start.starts_with("2026-04-27"));
        let may04 = events.iter().find(|e| e.start.starts_with("2026-05-04"));
        assert!(apr27.is_none(), "2026-04-27 should be excluded");
        assert!(may04.is_none(), "2026-05-04 should be excluded");
    }

    #[test]
    fn allday_value_date_expands_yearly() {
        // Birthday-style event: DTSTART;VALUE=DATE with RRULE:FREQ=YEARLY, no timezone.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:20110405\r\n\
DTEND;VALUE=DATE:20110406\r\n\
RRULE:FREQ=YEARLY\r\n\
SUMMARY:Birthday Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = expand_rrule(
            "birthday-uid",
            "Birthday Event",
            "Birthdays",
            &None,
            &None,
            &[],
            true,
            "2011-04-05 00:00:00",
            "2011-04-06 00:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        assert!(
            !events.is_empty(),
            "expand_rrule returned nothing for VALUE=DATE event"
        );
        let y2026 = events.iter().find(|e| e.start.starts_with("2026-04-05"));
        assert!(
            y2026.is_some(),
            "no occurrence on 2026-04-05; starts: {:?}",
            events.iter().map(|e| &e.start).collect::<Vec<_>>()
        );
    }

    #[test]
    fn wkst_tu_weekly_expands_june2026() {
        // Reproduces a "FREQ=WEEKLY;WKST=TU" series
        // starting Sat 2023-11-18. Verifies the June 27, 2026 occurrence is present.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20231118T100000\r\n\
DTEND;TZID=Europe/Berlin:20231118T120000\r\n\
RRULE:FREQ=WEEKLY;WKST=TU\r\n\
EXDATE;TZID=Europe/Berlin:20231230T100000\r\n\
EXDATE;TZID=Europe/Berlin:20240106T100000\r\n\
SUMMARY:Scouting Group\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = expand_rrule(
            "weekly-uid",
            "Scouting Group",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2023-11-18 10:00:00",
            "2023-11-18 12:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        assert!(
            !events.is_empty(),
            "expand_rrule returned nothing for WKST=TU event"
        );
        let jun27 = events.iter().find(|e| e.start.starts_with("2026-06-27"));
        assert!(
            jun27.is_some(),
            "no occurrence on 2026-06-27; last 5 starts: {:?}",
            events
                .iter()
                .rev()
                .take(5)
                .map(|e| &e.start)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn expand_rrule_skips_exception_dates() {
        // Weekly event; occurrence on 2026-05-04 is in the exception set.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
SUMMARY:Test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        // Simulate an exception row stored as uid_R20260504T180000.
        let mut exceptions = std::collections::HashSet::new();
        exceptions.insert("20260504T180000".to_string());

        let events = expand_rrule(
            "uid-test",
            "Test",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &exceptions,
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        let may04 = events.iter().find(|e| e.start.starts_with("2026-05-04"));
        assert!(
            may04.is_none(),
            "2026-05-04 should be excluded (exception date)"
        );
        // Other occurrences should still be present
        let may11 = events.iter().find(|e| e.start.starts_with("2026-05-11"));
        assert!(may11.is_some(), "2026-05-11 should still be present");
    }

    #[test]
    fn expand_rrule_populates_occurrence_start() {
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
SUMMARY:Test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let exceptions = std::collections::HashSet::new();
        let events = expand_rrule(
            "uid-test",
            "Test",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &exceptions,
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        assert!(!events.is_empty());
        // Every occurrence must have occurrence_start set
        for ev in &events {
            assert!(
                ev.occurrence_start.is_some(),
                "occurrence_start missing on {}",
                ev.start
            );
        }
        // First occurrence: 2026-01-05T18:00:00 → "20260105T180000"
        let first = events
            .iter()
            .find(|e| e.start.starts_with("2026-01-05"))
            .unwrap();
        assert_eq!(first.occurrence_start.as_deref(), Some("20260105T180000"));
    }

    #[test]
    fn expand_rrule_populates_rrule_field() {
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO\r\n\
SUMMARY:Test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let exceptions = std::collections::HashSet::new();
        let events = expand_rrule(
            "uid-test",
            "Test",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &exceptions,
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        assert!(!events.is_empty());
        for ev in &events {
            assert_eq!(
                ev.rrule.as_deref(),
                Some("FREQ=WEEKLY;BYDAY=MO"),
                "rrule field incorrect on {}",
                ev.start
            );
        }
    }

    #[test]
    fn phantom_duplicate_excluded_by_exception_map() {
        // Weekly event; occurrence on 2026-05-04 has an override exception row.
        // expand_rrule receives the exception set; that occurrence must vanish from expansion.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
SUMMARY:Weekly\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let mut exceptions = std::collections::HashSet::new();
        exceptions.insert("20260504T180000".to_string());

        let events = expand_rrule(
            "uid-base",
            "Weekly",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &exceptions,
            "UTC",
            wide_range().0,
            wide_range().1,
        );

        let may04_count = events
            .iter()
            .filter(|e| e.start.starts_with("2026-05-04"))
            .count();
        assert_eq!(
            may04_count, 0,
            "phantom occurrence on 2026-05-04 must be excluded"
        );
    }

    #[test]
    fn expand_rrule_far_future_range_still_expands_indefinite_rrule() {
        // Indefinitely-recurring RRULE (no UNTIL/COUNT), like the "Alice Work"
        // Google import that regressed under the old `now + 2 years` cutoff.
        // Requesting a range far in the future (2035) must still produce
        // occurrences — the expansion is bounded by the requested range, not
        // by a fixed horizon from "now".
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20230103T090000\r\n\
DTEND;TZID=Europe/Berlin:20230103T170000\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\r\n\
SUMMARY:Alice Work\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let range_start = Utc.with_ymd_and_hms(2035, 6, 1, 0, 0, 0).unwrap();
        let range_end = Utc.with_ymd_and_hms(2035, 6, 30, 23, 59, 59).unwrap();

        let events = expand_rrule(
            "kantoor-uid",
            "Alice Work",
            "work",
            &None,
            &None,
            &[],
            false,
            "2023-01-03 09:00:00",
            "2023-01-03 17:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "Europe/Berlin",
            range_start,
            range_end,
        );

        assert!(
            !events.is_empty(),
            "expand_rrule returned nothing for an indefinite RRULE requested far in the future (2035); \
             a fixed 'now + 2 years' cutoff would incorrectly suppress these occurrences"
        );
        assert!(
            events.iter().all(|e| e.start.starts_with("2035-06")),
            "all occurrences should fall within the requested June 2035 range; got: {:?}",
            events.iter().map(|e| &e.start).collect::<Vec<_>>()
        );
    }

    #[test]
    fn expand_rrule_excludes_occurrences_outside_range() {
        // Weekly event spanning many years; a narrow range should only return
        // the occurrences that actually overlap it.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=WEEKLY\r\n\
SUMMARY:Test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let range_start = Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();
        let range_end = Utc.with_ymd_and_hms(2026, 5, 31, 23, 59, 59).unwrap();

        let events = expand_rrule(
            "uid-test",
            "Test",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-05 18:00:00",
            "2026-01-05 19:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            range_start,
            range_end,
        );

        assert!(!events.is_empty(), "expected occurrences within May 2026");
        assert!(
            events.iter().all(|e| e.start.starts_with("2026-05")),
            "occurrences outside the requested range leaked through: {:?}",
            events.iter().map(|e| &e.start).collect::<Vec<_>>()
        );
        // Sanity: an occurrence well outside the range (e.g. January) must not appear.
        assert!(events.iter().all(|e| !e.start.starts_with("2026-01")));
    }

    #[test]
    fn expand_rrule_includes_multiday_event_starting_before_range() {
        // A multi-day (3-hour-long, but conceptually "spans the boundary")
        // occurrence whose start is just before range_start but whose
        // start+duration overlaps range_start must still be included —
        // this is the skip_while overlap check, not a plain start-time filter.
        let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=UTC:20260101T230000\r\n\
DTEND;TZID=UTC:20260102T020000\r\n\
RRULE:FREQ=WEEKLY\r\n\
SUMMARY:Overnight\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        // Occurrence starts 2026-01-08 23:00 UTC, ends 2026-01-09 02:00 UTC
        // (3-hour duration). range_start is set to 00:30 on 2026-01-09 —
        // after the occurrence's start but before its end.
        let range_start = Utc.with_ymd_and_hms(2026, 1, 9, 0, 30, 0).unwrap();
        let range_end = Utc.with_ymd_and_hms(2026, 1, 9, 23, 59, 59).unwrap();

        let events = expand_rrule(
            "overnight-uid",
            "Overnight",
            "personal",
            &None,
            &None,
            &[],
            false,
            "2026-01-01 23:00:00",
            "2026-01-02 02:00:00",
            ical_data,
            &std::collections::HashSet::new(),
            "UTC",
            range_start,
            range_end,
        );

        assert!(
            events.iter().any(|e| e.start.starts_with("2026-01-08")),
            "occurrence starting before range_start but overlapping it was dropped; got: {:?}",
            events.iter().map(|e| &e.start).collect::<Vec<_>>()
        );
    }
}

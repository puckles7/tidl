//! Event, task, and await item server functions.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::{
    add_exdate_to_master, auth, build_alarms_ical, build_event_ical, current_user,
    date_str_minus_one, expand_rrule, extract_all_day_and_alarms, extract_recurrence_id,
    fmt_ical_dt, notify_change, tz_aware_datetime_utc, tz_aware_epoch_ms, vevent_has_rrule,
    ChangeKind,
};

use super::{AwaitItem, Event, NewEvent, Task};
#[cfg(feature = "ssr")]
use crate::types::{MarkdownUid, NoteLabelInfo};
use crate::types::{TodoChip, TodoOrigin};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResolvedWikilink {
    pub uid: String,
    pub source_file: String,
}

/// Convert the raw fields from one event SQL row into zero or more `Event` values.
///
/// Returns multiple events when the row has an RRULE (recurring event expansion).
/// Falls back to a single non-recurring event when expansion fails or is not needed.
/// `raw_start` / `raw_end` are SQLite datetime strings (`"YYYY-MM-DD HH:MM:SS"`),
/// used by `expand_rrule` to compute event duration.
///
/// `exception_dates` is the set of recurrence-IDs for which an override row
/// already exists in the DB (derived from sibling UIDs of the form
/// `{series_uid}_R{recurrence_id}`). These occurrences are skipped during RRULE
/// expansion so only the exception row is emitted, not a phantom duplicate.
///
/// `range_start_dt` / `range_end_dt` bound RRULE expansion to the date range
/// the UI is currently displaying — see `expand_rrule`. Callers get these
/// from `range_bounds`, called *once* per request (not per row): both are
/// loop-invariant across every row `get_events` processes, and `range_bounds`
/// parses `timezone` via a chrono-tz lookup + DST-aware conversion, which is
/// too costly to redo per row on a calendar with a few thousand events.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)] // params are distinct SQL row fields, not a bundle
pub fn event_row_to_web_events(
    uid: String,
    summary: String,
    calendar: String,
    calendar_color: Option<String>,
    description: Option<String>,
    priority: Option<u32>,
    raw_start: String,
    raw_end: String,
    ical: &str,
    exception_dates: &std::collections::HashSet<String>,
    timezone: &str,
    range_start_dt: chrono::DateTime<chrono::Utc>,
    range_end_dt: chrono::DateTime<chrono::Utc>,
) -> Vec<Event> {
    let is_exception = uid.contains("_R");

    if !is_exception && vevent_has_rrule(ical) {
        let (all_day, alarms) = extract_all_day_and_alarms(ical);
        let expanded = expand_rrule(
            &uid,
            &summary,
            &calendar,
            &calendar_color,
            &description,
            &alarms,
            all_day,
            &raw_start,
            &raw_end,
            ical,
            exception_dates,
            timezone,
            range_start_dt,
            range_end_dt,
        );
        if !expanded.is_empty() {
            return expanded
                .into_iter()
                .map(|e| Event { priority, ..e })
                .collect();
        }
        // Expansion failed — fall through to single-event handling below,
        // which recomputes all_day/alarms. Rare path (malformed RRULE or
        // unparseable DTSTART), not worth threading the values through.
    }

    // Non-recurring (or failed expansion): check the cheap range-overlap
    // condition — using only `raw_start`/`raw_end` straight from the SQL
    // row, no iCal parsing at all — *before* paying for `is_all_day`/VALARM
    // extraction below. `get_events` previously ran that extraction for
    // every row in a user's entire history on every request, discarding
    // the ones outside the range only afterward; for an account with a few
    // thousand events, only a handful ever overlap a given request's
    // window, so checking first turns "parse ~100% of rows" into "parse
    // ~1%". `raw_end` is the exclusive end instant either way (a real
    // DTEND for timed events, or the CalDAV-spec "day after the last day"
    // for all-day events), so `e >= range_start_dt` is the correct
    // half-open-interval overlap check for both.
    if let (Some(s), Some(e)) = (
        tz_aware_datetime_utc(&raw_start, timezone),
        tz_aware_datetime_utc(&raw_end, timezone),
    ) {
        if e < range_start_dt || s > range_end_dt {
            return vec![];
        }
    }
    // Unparseable start/end: fall through and include unconditionally rather
    // than silently hiding an event with malformed dates.

    let (all_day, alarms) = extract_all_day_and_alarms(ical);
    let start_epoch_ms = tz_aware_epoch_ms(&raw_start, timezone);

    // All-day: "YYYY-MM-DD HH:MM:SS" → "YYYY-MM-DD" (inclusive end = DTEND−1d).
    // Timed:   "YYYY-MM-DD HH:MM:SS" → "YYYY-MM-DD HH:MM".
    let (start, end) = if all_day {
        let s = if raw_start.len() >= 10 {
            raw_start[..10].to_string()
        } else {
            raw_start
        };
        let e = date_str_minus_one(if raw_end.len() >= 10 {
            &raw_end[..10]
        } else {
            &raw_end
        });
        (s, e)
    } else {
        let s = if raw_start.len() >= 16 {
            raw_start[..16].to_string()
        } else {
            raw_start
        };
        let e = if raw_end.len() >= 16 {
            raw_end[..16].to_string()
        } else {
            raw_end
        };
        (s, e)
    };

    // For exception rows, extract RECURRENCE-ID from iCal so the UI
    // can route re-edits to update_event_occurrence with the correct key.
    let occurrence_start = if is_exception {
        extract_recurrence_id(ical)
    } else {
        None
    };

    vec![Event {
        id: uid,
        summary,
        start,
        end,
        all_day,
        description,
        priority,
        alarms,
        calendar,
        calendar_color,
        rrule: None,
        occurrence_start,
        is_exception,
        start_epoch_ms,
        owner_username: String::new(),
        shared_permission: None,
    }]
}

/// Maximum span (in days) accepted for `range_start`..`range_end`. This is a
/// defensive backstop, not the primary mechanism — the UI only ever requests
/// small (day/week/month) windows via `visible_range`, but `#[server]`
/// functions are real HTTP endpoints reachable independent of the UI.
#[cfg(feature = "ssr")]
const MAX_RANGE_DAYS: i64 = 366 * 5;

/// Parse `range_start`/`range_end` (`"YYYY-MM-DD"`) into inclusive UTC
/// instant bounds — `range_end` resolves to the *end* of that day (23:59:59),
/// not midnight. `expand_rrule`'s `take_while(dt <= range_end)` is an
/// inclusive point-in-time compare, so a midnight bound would silently
/// exclude any occurrence later that same day; the day-wide subtraction
/// keeps this correct even across a DST-shifted 23/25-hour local day.
///
/// Falls back to a `MAX_RANGE_DAYS`-wide, finite window around whichever
/// side parsed (or around "now" if neither did) instead of `MIN_UTC`/
/// `MAX_UTC` — `expand_rrule`'s `take_while` never terminates for an
/// indefinitely-recurring RRULE against `MAX_UTC`, which would hang the
/// request thread forever.
#[cfg(feature = "ssr")]
fn range_bounds(
    range_start: &str,
    range_end: &str,
    timezone: &str,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let start_dt = tz_aware_datetime_utc(range_start, timezone);
    let end_dt = tz_aware_datetime_utc(range_end, timezone)
        .map(|d| d + chrono::Duration::days(1) - chrono::Duration::seconds(1));
    match (start_dt, end_dt) {
        (Some(s), Some(e)) => (s, e),
        (Some(s), None) => (s, s + chrono::Duration::days(MAX_RANGE_DAYS)),
        (None, Some(e)) => (e - chrono::Duration::days(MAX_RANGE_DAYS), e),
        (None, None) => {
            let now = chrono::Utc::now();
            (
                now - chrono::Duration::days(MAX_RANGE_DAYS),
                now + chrono::Duration::days(MAX_RANGE_DAYS),
            )
        }
    }
}

/// Resolve one VEVENT row's canonical 1-100 priority (backlog #156).
///
/// The DB column is authoritative whenever it has a value — app-written
/// events keep full 1-100 precision there even though the iCal wire form is
/// quantised to RFC 5545's 1-9. Legacy rows created before the column was
/// populated (`import_ics`, inbound iMIP) fall back to the `PRIORITY`
/// property in `ical_data`; `0` and absent both mean "no priority".
#[cfg(feature = "ssr")]
fn effective_event_priority(db: Option<i64>, ical: &str) -> Option<u32> {
    if let Some(p) = db {
        return (p > 0).then(|| (p as u32).clamp(1, 100));
    }
    tidl_caldav::extract_event_fields(ical)
        .priority
        .filter(|p| *p != 0)
        .map(crate::types::caldav_priority_to_source_priority)
}

/// A single raw `events` row plus the sharing context needed to tag the
/// `Event` values built from it (backlog #41). `owner_username` is the
/// requesting user's own username for their own events, or the actual
/// owner's username for events pulled in from a calendar shared with them;
/// `shared_permission` mirrors `calendar_shares.permission` and is `None`
/// for the caller's own events.
#[cfg(feature = "ssr")]
struct EventRow {
    uid: String,
    summary: String,
    calendar: String,
    calendar_color: Option<String>,
    description: Option<String>,
    priority: Option<i64>,
    raw_start: String,
    raw_end: String,
    ical: String,
    owner_username: String,
    shared_permission: Option<String>,
}

/// SQL fragment appended to both `get_events`' own-calendar query and
/// `list_shared_calendar_events`'s query — a *safe superset* row prefilter
/// evaluated by SQLite before rows ever cross into Rust, so the app never
/// deserializes/parses the ~98% of a large history that a given request's
/// visible window has no use for. A row survives if any of:
///  - its own `start_date`/`end_date` overlaps `[?, ?]` (bound to
///    `sql_prefilter_bounds`'s padded output — see there for why it's
///    padded and string-compared rather than exact),
///  - its *own* `VEVENT` block contains an RRULE (mirrors `vevent_has_rrule`'s
///    VEVENT-scoping in SQL via nested `instr`/`substr`, rather than a bare
///    `ical_data LIKE '%RRULE%'` — Google Calendar exports embed an RRULE
///    inside their `VTIMEZONE` preamble for DST transitions, so an unscoped
///    check would match nearly every row and defeat the filter),
///  - it's a recurrence-exception row (`uid` contains `_R`) — these must
///    always be fetched regardless of their own date, because
///    `get_events`'s `exception_map` (built from exactly these rows) is what
///    stops RRULE expansion from re-emitting a phantom duplicate at the
///    *original* occurrence date, which can fall inside the visible window
///    even when the exception's relocated date does not,
///  - its `start_date`/`end_date` is `NULL` — `event_row_to_web_events`
///    deliberately fails *open* on an unparseable date ("include
///    unconditionally rather than silently hiding an event with malformed
///    dates"), so the SQL side must too. `NULL >= ?` evaluates to `NULL`
///    (not true) in SQLite, not "unbounded", so without this branch a
///    ~50-event chunk of this real account's data (Google-imported events
///    with no DTEND) silently vanished — caught by the diagnostic diff
///    against the unfiltered query while developing this filter, not by
///    any prior unit test.
///
/// One known, accepted gap versus the unfiltered query: `raw_start`/
/// `raw_end` can also fail to parse in Rust for a reason SQL has no way to
/// replicate — a local timestamp that lands in a DST fall-back's repeated
/// hour (e.g. `2018-10-28 02:00:00` in `Europe/Berlin`) is *ambiguous*,
/// so `chrono_tz`'s `.single()` returns `None` even though the string is
/// well-formed. `event_row_to_web_events` treats that the same as a
/// malformed date and includes the event unconditionally — which, absent
/// this prefilter, meant a handful of such events (in practice: one, in
/// this account's real data) showed up on *every* calendar view regardless
/// of the requested range. This prefilter scopes them back to requests
/// whose padded window covers their actual date, same as any other event —
/// arguably a bug fix (no more permanently-stuck-visible event) rather than
/// a loss, but worth stating plainly since it is a real behavior change.
#[cfg(feature = "ssr")]
const SQL_RANGE_PREFILTER: &str = "
    AND (
        (e.end_date >= ? AND e.start_date <= ?)
        OR e.end_date IS NULL OR e.start_date IS NULL
        OR (
            instr(e.ical_data, 'BEGIN:VEVENT') > 0
            AND instr(substr(e.ical_data, instr(e.ical_data, 'BEGIN:VEVENT')), 'RRULE:') > 0
        )
        OR instr(e.uid, '_R') > 0
    )";

/// Padded, string-comparable SQL bounds for `SQL_RANGE_PREFILTER`. Widened
/// by one calendar day past `range_start`/`range_end` on each side: unlike
/// the exact tz-aware check in `event_row_to_web_events` (which converts
/// `start_date`/`end_date` — stored in the *user's local timezone* — to UTC
/// instants before comparing), this is a plain string compare done inside
/// SQLite. A day of padding safely absorbs any timezone offset (up to
/// UTC+14) without risking excluding a row the precise check downstream
/// would have kept; that downstream check still applies afterward and is
/// what actually decides inclusion.
#[cfg(feature = "ssr")]
fn sql_prefilter_bounds(range_start: &str, range_end: &str) -> (String, String) {
    use chrono::NaiveDate;
    let pad = chrono::Duration::days(1);
    let lo = NaiveDate::parse_from_str(range_start.trim(), "%Y-%m-%d")
        .map(|d| (d - pad).format("%Y-%m-%d 00:00:00").to_string())
        .unwrap_or_else(|_| "0000-01-01 00:00:00".to_string());
    let hi = NaiveDate::parse_from_str(range_end.trim(), "%Y-%m-%d")
        .map(|d| (d + pad).format("%Y-%m-%d 23:59:59").to_string())
        .unwrap_or_else(|_| "9999-12-31 23:59:59".to_string());
    (lo, hi)
}

/// Events from calendars shared with `grantee`, tagged with their real
/// owner and the grant's permission. Split out from `get_events` so it's
/// unit-testable without a live Leptos/Axum session context.
///
/// `sql_lo`/`sql_hi` are `sql_prefilter_bounds`'s output — see
/// `SQL_RANGE_PREFILTER`.
#[cfg(feature = "ssr")]
async fn list_shared_calendar_events(
    pool: &sqlx::SqlitePool,
    grantee: &str,
    sql_lo: &str,
    sql_hi: &str,
) -> Result<Vec<EventRow>, sqlx::Error> {
    use sqlx::Row;

    let query = format!(
        "SELECT e.uid, e.summary, e.calendar,
                CAST(e.start_date AS TEXT) AS start_date,
                CAST(e.end_date   AS TEXT) AS end_date,
                e.description, e.priority, e.ical_data,
                c.color AS calendar_color,
                s.owner_username, s.permission
         FROM calendar_shares s
         JOIN events e ON e.username = s.owner_username AND e.calendar = s.calendar_name
         LEFT JOIN calendars c ON c.username = e.username AND c.name = e.calendar
         WHERE s.grantee_username = ? AND e.type = 'VEVENT'
         {SQL_RANGE_PREFILTER}
         ORDER BY e.start_date ASC"
    );
    let rows = sqlx::query(&query)
        .bind(grantee)
        .bind(sql_lo)
        .bind(sql_hi)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| EventRow {
            uid: r.try_get("uid").unwrap_or_default(),
            summary: r
                .try_get::<Option<String>, _>("summary")
                .unwrap_or_default()
                .unwrap_or_default(),
            calendar: r
                .try_get::<Option<String>, _>("calendar")
                .unwrap_or_default()
                .unwrap_or_else(|| "personal".to_string()),
            calendar_color: r.try_get("calendar_color").unwrap_or_default(),
            description: r.try_get("description").unwrap_or_default(),
            priority: r.try_get("priority").unwrap_or_default(),
            raw_start: r
                .try_get::<Option<String>, _>("start_date")
                .unwrap_or_default()
                .unwrap_or_default(),
            raw_end: r
                .try_get::<Option<String>, _>("end_date")
                .unwrap_or_default()
                .unwrap_or_default(),
            ical: r.try_get("ical_data").unwrap_or_default(),
            owner_username: r.try_get("owner_username").unwrap_or_default(),
            shared_permission: r.try_get("permission").ok(),
        })
        .collect())
}

/// True when `caller` holds a live read-write share of `calendar_name`
/// owned by `owner` — used by `create_event` to authorize writing to a
/// calendar picked from the "shared with me" dropdown before trusting the
/// client-supplied `calendar_owner`. Mirrors `resolve_event_write_owner`'s
/// read-write gating, but keyed by calendar identity rather than an
/// existing event uid, since a brand-new event has no uid yet.
#[cfg(feature = "ssr")]
async fn caller_has_read_write_share(
    pool: &sqlx::SqlitePool,
    owner: &str,
    calendar_name: &str,
    caller: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT 1 FROM calendar_shares \
         WHERE owner_username = ? AND calendar_name = ? \
         AND grantee_username = ? AND permission = 'read-write'",
    )
    .bind(owner)
    .bind(calendar_name)
    .bind(caller)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// If `uid` belongs to the caller's own events, returns `None` (caller
/// should use their own username as before, unchanged). If it belongs to
/// someone else's calendar that's been shared with `caller` at
/// `read-write` permission, returns `Some(owner_username)` — the caller
/// may operate on that event using the owner's username instead. Returns
/// `None` (not an error) for a read-only share or no share at all; callers
/// distinguish "use my own username" from "no access at all" by then
/// checking their own `WHERE uid = ? AND username = ?` lookup normally —
/// this helper only ever *adds* permitted cross-owner access, it never
/// removes the existing same-user path.
#[cfg(feature = "ssr")]
async fn resolve_event_write_owner(
    pool: &sqlx::SqlitePool,
    caller: &str,
    uid: &str,
) -> Result<Option<String>, sqlx::Error> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT s.owner_username FROM calendar_shares s
         JOIN events e ON e.username = s.owner_username AND e.calendar = s.calendar_name
         WHERE e.uid = ? AND s.grantee_username = ? AND s.permission = 'read-write'",
    )
    .bind(uid)
    .bind(caller)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| r.try_get::<String, _>("owner_username").unwrap_or_default()))
}

/// Every username `calendar_name` (owned by `owner_username`) is shared
/// with, regardless of permission level — a `read`-only grantee still needs
/// to see the calendar's events change live, same as a `read-write` one.
#[cfg(feature = "ssr")]
pub(super) async fn calendar_grantees(
    pool: &sqlx::SqlitePool,
    owner_username: &str,
    calendar_name: &str,
) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT grantee_username FROM calendar_shares WHERE owner_username = ? AND calendar_name = ?",
    )
    .bind(owner_username)
    .bind(calendar_name)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Notify a calendar's owner and every one of its grantees that `kind`
/// changed. Event mutations used to `notify_change` only the acting user —
/// fine for the owner editing their own calendar, but a grantee viewing that
/// same calendar never got pushed an update and needed a manual refresh
/// (backlog #59). Mirrors the owner+grantees notification pattern already
/// used by calendar-level mutations (e.g. `delete_calendar_api`,
/// `clear_shared_calendar` in `api/calendars.rs`).
#[cfg(feature = "ssr")]
pub(super) async fn notify_calendar_watchers(
    pool: &sqlx::SqlitePool,
    owner_username: &str,
    calendar_name: &str,
    kind: ChangeKind,
) {
    notify_change(owner_username, kind).await;
    for grantee in &calendar_grantees(pool, owner_username, calendar_name).await {
        notify_change(grantee, kind).await;
    }
}

/// Return all VEVENT entries whose occurrences overlap `[range_start, range_end]`
/// ("YYYY-MM-DD"), sorted by start date.
///
/// Non-recurring events are always returned in full regardless of range (the
/// UI only renders what's relevant); only RRULE expansion is range-bound.
/// The requested span is clamped server-side to `MAX_RANGE_DAYS` as a safety
/// net against pathological requests.
#[server]
pub async fn get_events(
    range_start: String,
    range_end: String,
) -> Result<Vec<Event>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;
        use std::collections::{HashMap, HashSet};

        let (pool, username) = auth().await?;

        let (range_start, range_end) = clamp_range(&range_start, &range_end);
        let (sql_lo, sql_hi) = sql_prefilter_bounds(&range_start, &range_end);

        let timezone: String = sqlx::query_scalar(
            "SELECT COALESCE(timezone, 'UTC') FROM user_preferences WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "UTC".to_string());

        let query = format!(
            "SELECT e.uid, e.summary, e.calendar,
                    CAST(e.start_date AS TEXT) AS start_date,
                    CAST(e.end_date   AS TEXT) AS end_date,
                    e.description, e.priority, e.ical_data,
                    c.color AS calendar_color
             FROM events e
             LEFT JOIN calendars c ON c.username = e.username AND c.name = e.calendar
             WHERE e.type = 'VEVENT' AND e.username = ?
             {SQL_RANGE_PREFILTER}
             ORDER BY e.start_date ASC"
        );
        let rows = sqlx::query(&query)
            .bind(&username)
            .bind(&sql_lo)
            .bind(&sql_hi)
            .fetch_all(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut all_rows: Vec<EventRow> = rows
            .into_iter()
            .map(|r| EventRow {
                uid: r.try_get("uid").unwrap_or_default(),
                summary: r
                    .try_get::<Option<String>, _>("summary")
                    .unwrap_or_default()
                    .unwrap_or_default(),
                calendar: r
                    .try_get::<Option<String>, _>("calendar")
                    .unwrap_or_default()
                    .unwrap_or_else(|| "personal".to_string()),
                calendar_color: r.try_get("calendar_color").unwrap_or_default(),
                description: r.try_get("description").unwrap_or_default(),
                priority: r.try_get("priority").unwrap_or_default(),
                raw_start: r
                    .try_get::<Option<String>, _>("start_date")
                    .unwrap_or_default()
                    .unwrap_or_default(),
                raw_end: r
                    .try_get::<Option<String>, _>("end_date")
                    .unwrap_or_default()
                    .unwrap_or_default(),
                ical: r.try_get("ical_data").unwrap_or_default(),
                owner_username: username.clone(),
                shared_permission: None,
            })
            .collect();

        all_rows.extend(
            list_shared_calendar_events(&pool, &username, &sql_lo, &sql_hi)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?,
        );

        // Build series_uid → set-of-recurrence-ids from exception row UIDs.
        // Exception UIDs have the form "{series_uid}_R{recurrence_id}". Built
        // over the combined (own + shared) rows so a shared calendar's own
        // recurring-event exceptions are detected too.
        let mut exception_map: HashMap<String, HashSet<String>> = HashMap::new();
        for r in &all_rows {
            if let Some(pos) = r.uid.rfind("_R") {
                let series = r.uid[..pos].to_string();
                let rid = r.uid[pos + 2..].to_string();
                exception_map.entry(series).or_default().insert(rid);
            }
        }

        let empty_set: HashSet<String> = HashSet::new();

        // Computed once per request, not per row: both bounds and the
        // `timezone` chrono-tz lookup are identical for every row processed
        // below (see `event_row_to_web_events`'s doc comment).
        let (range_start_dt, range_end_dt) = range_bounds(&range_start, &range_end, &timezone);

        let mut result: Vec<Event> = Vec::new();
        for r in all_rows {
            // Look up exceptions using the series uid (strip _R suffix if present).
            let series_uid = if let Some(pos) = r.uid.rfind("_R") {
                r.uid[..pos].to_string()
            } else {
                r.uid.clone()
            };
            let exceptions = exception_map.get(&series_uid).unwrap_or(&empty_set);
            let priority = effective_event_priority(r.priority, &r.ical);

            for mut event in event_row_to_web_events(
                r.uid,
                r.summary,
                r.calendar,
                r.calendar_color,
                r.description,
                priority,
                r.raw_start,
                r.raw_end,
                &r.ical,
                exceptions,
                &timezone,
                range_start_dt,
                range_end_dt,
            ) {
                event.owner_username = r.owner_username.clone();
                event.shared_permission = r.shared_permission.clone();
                result.push(event);
            }
        }
        return Ok(result);
    }

    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Clamp a requested `["YYYY-MM-DD", "YYYY-MM-DD"]` range to at most
/// `MAX_RANGE_DAYS` days, and swap the bounds if they arrive reversed.
/// Unparseable bounds are left as-is (this function only clamps when both
/// sides parse as `NaiveDate`) and passed through to `event_row_to_web_events`,
/// which applies its own bounded fallback (relative to `MAX_RANGE_DAYS`) when
/// `tz_aware_datetime_utc` can't parse a side — so a malformed value still
/// degrades gracefully, with a finite window, rather than silently hiding
/// events or iterating unboundedly.
#[cfg(feature = "ssr")]
fn clamp_range(range_start: &str, range_end: &str) -> (String, String) {
    use chrono::NaiveDate;

    let start = NaiveDate::parse_from_str(range_start.trim(), "%Y-%m-%d").ok();
    let end = NaiveDate::parse_from_str(range_end.trim(), "%Y-%m-%d").ok();

    match (start, end) {
        (Some(s), Some(e)) => {
            let (s, e) = if s <= e { (s, e) } else { (e, s) };
            let clamped_e = if (e - s).num_days() > MAX_RANGE_DAYS {
                s + chrono::Duration::days(MAX_RANGE_DAYS)
            } else {
                e
            };
            (
                s.format("%Y-%m-%d").to_string(),
                clamped_e.format("%Y-%m-%d").to_string(),
            )
        }
        _ => (range_start.to_string(), range_end.to_string()),
    }
}

/// Parse a `GROUP_CONCAT` of `id␟name␟color` records (records joined by
/// `\u{1e}`, fields by `\u{1f}`) into `NoteLabelInfo`s (backlog #114). The
/// ASCII separators are used instead of commas/colons so label names that
/// themselves contain those characters survive intact. Anything malformed is
/// dropped rather than aborting the whole list.
#[cfg(feature = "ssr")]
fn parse_note_labels(concat: &str) -> Vec<NoteLabelInfo> {
    concat
        .split('\u{1e}')
        .filter(|rec| !rec.is_empty())
        .filter_map(|rec| {
            let mut fields = rec.splitn(3, '\u{1f}');
            let id = fields.next()?.parse().ok()?;
            let name = fields.next()?.to_string();
            let color = fields.next().unwrap_or_default().to_string();
            Some(NoteLabelInfo { id, name, color })
        })
        .collect()
}

/// Return all TASK-kind VTODO entries, sorted by priority then due date.
#[server]
pub async fn get_tasks() -> Result<Vec<Task>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT e.uid, e.summary, e.priority, CAST(e.due_date AS TEXT) AS due_date, e.status,
                    COALESCE(e.source, 'caldav') AS source, e.calendar,
                    e.source_file, e.source_line, e.topic,
                    nc.id AS collection_id, nc.name AS collection_name, nc.color AS collection_color,
                    n.id AS note_id, n.priority AS note_priority,
                    (SELECT GROUP_CONCAT(l.id || CHAR(31) || l.name || CHAR(31) || l.color, CHAR(30))
                     FROM note_label_links lnk
                     JOIN note_labels l ON l.id = lnk.label_id
                     WHERE lnk.note_id = n.id) AS note_labels
             FROM events e
             LEFT JOIN notes n ON n.id = e.note_id
             LEFT JOIN note_collections nc ON nc.id = n.collection_id
             WHERE e.type = 'VTODO' AND e.username = ? AND e.kind = 'task'
             ORDER BY e.priority ASC NULLS LAST, e.due_date ASC NULLS LAST",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| Task {
                id: r.try_get("uid").unwrap_or_default(),
                summary: r
                    .try_get::<Option<String>, _>("summary")
                    .unwrap_or_default()
                    .unwrap_or_default(),
                priority: r
                    .try_get::<Option<i64>, _>("priority")
                    .unwrap_or_default()
                    .unwrap_or(1000) as u32,
                due: r
                    .try_get::<Option<String>, _>("due_date")
                    .unwrap_or_default(),
                completed: r
                    .try_get::<Option<String>, _>("status")
                    .unwrap_or_default()
                    .as_deref()
                    == Some("COMPLETED"),
                source: r
                    .try_get::<Option<String>, _>("source")
                    .unwrap_or_default()
                    .unwrap_or_else(|| "caldav".into()),
                calendar: r
                    .try_get::<Option<String>, _>("calendar")
                    .unwrap_or_default()
                    .unwrap_or_default(),
                source_file: r
                    .try_get::<Option<String>, _>("source_file")
                    .unwrap_or_default(),
                source_line: r
                    .try_get::<Option<i64>, _>("source_line")
                    .unwrap_or_default()
                    .map(|v| v as u32),
                topic: r.try_get::<Option<String>, _>("topic").unwrap_or_default(),
                collection_id: r
                    .try_get::<Option<i64>, _>("collection_id")
                    .unwrap_or_default()
                    .map(|v| v.to_string()),
                collection_name: r
                    .try_get::<Option<String>, _>("collection_name")
                    .unwrap_or_default(),
                collection_color: r
                    .try_get::<Option<String>, _>("collection_color")
                    .unwrap_or_default(),
                note_id: r.try_get::<Option<i64>, _>("note_id").unwrap_or_default(),
                note_priority: r
                    .try_get::<Option<i64>, _>("note_priority")
                    .unwrap_or_default()
                    .map(|v| v as u32),
                note_labels: r
                    .try_get::<Option<String>, _>("note_labels")
                    .unwrap_or_default()
                    .map(|s| parse_note_labels(&s))
                    .unwrap_or_default(),
            })
            .collect());
    }

    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Create a new ad-hoc CalDAV VTODO for the current user.
///
/// `kind` is `"task"` (priority-driven) or `"await"` (date-driven) — the same
/// TASK/AWAIT discriminator the Notes editor uses (backlog #49), so a manual
/// creation lands in exactly one list instead of always `task` with a due
/// date. A task carries a 1-100 `priority` and no due date; an await carries a
/// `due_date` and no priority.
#[server(input = server_fn::codec::Json)]
pub async fn create_todo_item(
    summary: String,
    kind: String,
    priority: Option<u32>,
    due_date: Option<String>,
    calendar: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;
        use uuid::Uuid;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;

        let uid = Uuid::new_v4().to_string();
        let etag = format!("etag-{}", Uuid::new_v4().simple());
        let kind = if kind == "await" { "await" } else { "task" }.to_string();

        // `priority` is the canonical 1-100 scale stored in the DB column; the
        // ical PRIORITY line is the one place it gets projected onto RFC 5545's
        // native 1-9 range (backlog #70). Awaits have no PRIORITY at all.
        let (priority_col, ical_body) = if kind == "await" {
            let d = due_date
                .as_deref()
                .ok_or_else(|| ServerFnError::new("An await needs a date"))?;
            (
                None,
                format!(
                    "SUMMARY:{summary}\r\nDUE;VALUE=DATE:{}\r\nSTATUS:NEEDS-ACTION",
                    d.replace('-', "")
                ),
            )
        } else {
            let p = priority.unwrap_or(50).clamp(1, 100);
            let caldav_priority = crate::types::source_priority_to_caldav(p);
            (
                Some(p as i64),
                format!("SUMMARY:{summary}\r\nPRIORITY:{caldav_priority}\r\nSTATUS:NEEDS-ACTION"),
            )
        };

        let ical_data = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
             BEGIN:VTODO\r\nUID:{uid}\r\n{ical_body}\r\n\
             END:VTODO\r\nEND:VCALENDAR\r\n"
        );

        sqlx::query(
            "INSERT INTO events
             (username, calendar, uid, type, summary, priority, status,
              due_date, source, etag, ical_data, kind)
             VALUES (?, ?, ?, 'VTODO', ?, ?, 'NEEDS-ACTION',
                     ?, 'caldav', ?, ?, ?)",
        )
        .bind(&username)
        .bind(&calendar)
        .bind(&uid)
        .bind(&summary)
        .bind(priority_col)
        .bind(due_date.as_deref())
        .bind(&etag)
        .bind(&ical_data)
        .bind(&kind)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_caldav_sync_token(&pool, &username, &calendar, &uid, "modified").await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Update an ad-hoc CalDAV VTODO's title, its kind-appropriate field (a task's
/// priority 1-100 or an await's due date), and/or the calendar it lives in.
///
/// Only items with no markdown/note backing (`source` NULL or `'caldav'`) are
/// editable here; markdown- and note-backed items are rewritten through their
/// own source. The kind's other field is left untouched so editing a legacy
/// task that happens to carry a due date doesn't silently drop it.
#[server(input = server_fn::codec::Json)]
pub async fn update_todo_item(
    uid: String,
    summary: String,
    priority: Option<u32>,
    due_date: Option<String>,
    calendar: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;
        use tidl_caldav::etag::generate_etag;

        let (pool, username) = auth().await?;
        let effective_username = match resolve_event_write_owner(&pool, &username, &uid).await {
            Ok(Some(owner)) => owner,
            _ => username.clone(),
        };

        let row = sqlx::query(
            "SELECT ical_data, calendar, kind FROM events
             WHERE uid = ? AND username = ? AND type = 'VTODO'
               AND COALESCE(source, 'caldav') = 'caldav'",
        )
        .bind(&uid)
        .bind(&effective_username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Task not found or not editable"))?;

        let ical: String = row.try_get("ical_data").unwrap_or_default();
        let old_calendar: String = row
            .try_get::<Option<String>, _>("calendar")
            .unwrap_or_default()
            .unwrap_or_default();
        let kind: String = row.try_get("kind").unwrap_or_else(|_| "task".to_string());

        // Empty selection keeps the current calendar (defensive; the dialog
        // always sends a value, prefilled from the item).
        let new_calendar = if calendar.trim().is_empty() {
            old_calendar.clone()
        } else {
            calendar.clone()
        };

        let ical = set_vtodo_prop(&ical, "SUMMARY", Some(&format!("SUMMARY:{summary}")));

        if kind == "await" {
            let d = due_date
                .as_deref()
                .ok_or_else(|| ServerFnError::new("An await needs a date"))?;
            let ical = set_vtodo_prop(
                &ical,
                "DUE",
                Some(&format!("DUE;VALUE=DATE:{}", d.replace('-', ""))),
            );
            let etag = generate_etag(&ical);
            sqlx::query(
                "UPDATE events SET summary = ?, due_date = ?, calendar = ?, ical_data = ?,
                        etag = ?, updated_at = datetime('now')
                 WHERE uid = ? AND username = ? AND type = 'VTODO'",
            )
            .bind(&summary)
            .bind(d)
            .bind(&new_calendar)
            .bind(&ical)
            .bind(&etag)
            .bind(&uid)
            .bind(&effective_username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        } else {
            let p = priority.unwrap_or(50).clamp(1, 100);
            let caldav_priority = crate::types::source_priority_to_caldav(p);
            let ical = set_vtodo_prop(
                &ical,
                "PRIORITY",
                Some(&format!("PRIORITY:{caldav_priority}")),
            );
            let etag = generate_etag(&ical);
            sqlx::query(
                "UPDATE events SET summary = ?, priority = ?, calendar = ?, ical_data = ?,
                        etag = ?, updated_at = datetime('now')
                 WHERE uid = ? AND username = ? AND type = 'VTODO'",
            )
            .bind(&summary)
            .bind(p as i64)
            .bind(&new_calendar)
            .bind(&ical)
            .bind(&etag)
            .bind(&uid)
            .bind(&effective_username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }

        // Moving calendars: tell the old one the item left, then announce it in
        // the new one (same two-sided bump `update_event` does, backlog #59).
        if !old_calendar.is_empty() && old_calendar != new_calendar {
            bump_caldav_sync_token(&pool, &effective_username, &old_calendar, &uid, "deleted")
                .await;
            notify_calendar_watchers(
                &pool,
                &effective_username,
                &old_calendar,
                ChangeKind::Events,
            )
            .await;
        }
        if !new_calendar.is_empty() {
            bump_caldav_sync_token(&pool, &effective_username, &new_calendar, &uid, "modified")
                .await;
            notify_calendar_watchers(
                &pool,
                &effective_username,
                &new_calendar,
                ChangeKind::Events,
            )
            .await;
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Replace (or insert) a top-level `key` property within the first VTODO of an
/// iCal string, preserving every other line verbatim — including folded
/// continuation lines of the property being replaced. `line` is the complete
/// replacement property line (e.g. `"DUE;VALUE=DATE:20260801"`), or `None` to
/// remove the property. Values over 75 octets are folded per RFC 5545.
#[cfg(feature = "ssr")]
fn set_vtodo_prop(ical: &str, key: &str, line: Option<&str>) -> String {
    let crlf = ical.contains("\r\n");
    let lines: Vec<&str> = ical
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let prefix_colon = format!("{key}:");
    let prefix_semi = format!("{key};");
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    let mut in_vtodo = false;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l == "BEGIN:VTODO" {
            in_vtodo = true;
        } else if l == "END:VTODO" {
            if let Some(line) = line {
                out.push(fold_ical_line(line));
            }
            in_vtodo = false;
        } else if in_vtodo && (l.starts_with(&prefix_colon) || l.starts_with(&prefix_semi)) {
            i += 1;
            while i < lines.len() && (lines[i].starts_with(' ') || lines[i].starts_with('\t')) {
                i += 1;
            }
            continue;
        }
        out.push(l.to_string());
        i += 1;
    }
    let sep = if crlf { "\r\n" } else { "\n" };
    out.join(sep)
}

/// Fold a single iCal content line at 75 octets (continuation lines begin with
/// a space), never splitting a UTF-8 code point.
#[cfg(feature = "ssr")]
fn fold_ical_line(line: &str) -> String {
    if line.len() <= 75 {
        return line.to_string();
    }
    let mut out = String::new();
    let mut start = 0;
    let mut first = true;
    while start < line.len() {
        let limit = start + if first { 75 } else { 74 };
        let mut end = limit.min(line.len());
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }
        if !first {
            out.push_str("\r\n ");
        }
        out.push_str(&line[start..end]);
        start = end;
        first = false;
    }
    out
}

/// Mark a VTODO as completed (or reopen it).
///
/// For markdown-sourced todos, the tag is stripped from the source file so the
/// next sync removes the event row entirely. For note-sourced todos, the
/// inline `#task`/`#await` token is rewritten to its `#done/...` record
/// (retaining the original priority/date so it stays reopenable) and the event
/// is deleted — the note body is the record (backlog #113).
#[server(input = server_fn::codec::Json)]
pub async fn complete_todo_item(uid: String, completed: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;
        let effective_username = match resolve_event_write_owner(&pool, &username, &uid).await {
            Ok(Some(owner)) => owner,
            _ => username.clone(),
        };

        // Fetch the todo to check its source (and, for a note origin, the
        // fields needed to build its `#done` record).
        let row = sqlx::query(
            "SELECT source, source_file, source_line, kind, priority, topic, summary,
                    calendar, note_id, CAST(due_date AS TEXT) AS due_date
             FROM events
             WHERE uid = ? AND username = ? AND type = 'VTODO'",
        )
        .bind(&uid)
        .bind(&effective_username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let row = row.ok_or_else(|| ServerFnError::new("Todo not found"))?;
        let source: String = row
            .try_get::<Option<String>, _>("source")
            .unwrap_or_default()
            .unwrap_or_else(|| "caldav".into());
        let source_file: Option<String> = row.try_get("source_file").unwrap_or_default();
        let source_line: Option<i64> = row.try_get("source_line").unwrap_or_default();

        // For markdown todos being completed: rewrite the tag to `#done/<slug>`
        // so the line stays in the file as a record but drops out of the task
        // lists on the next sync (backlog #112; previously the tag was stripped
        // entirely).
        if source == "markdown" && completed {
            if let Some(rel_path) = source_file {
                // Uid format: "md:{source_id}:{rel_path}:{line}:{ordinal}".
                let parsed = MarkdownUid::parse(&uid);
                let source_id: Option<i64> = parsed.as_ref().and_then(|u| u.source_id_i64());
                // Which tag on the line: several can share one.
                let ordinal = parsed.as_ref().map(|u| u.ordinal_u32()).unwrap_or(0) as usize;

                if let Some(sid) = source_id {
                    let row = sqlx::query(
                        "SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?",
                    )
                    .bind(sid)
                    .bind(&effective_username)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

                    if let Some(row) = row {
                        let source_type: String = row.try_get("type").unwrap_or_default();
                        let config_json: String = row.try_get("config").unwrap_or_default();
                        let v: serde_json::Value =
                            serde_json::from_str(&config_json).unwrap_or_default();
                        let idx = source_line.map(|n| (n - 1) as usize).unwrap_or(usize::MAX);
                        let completed = chrono::Utc::now().date_naive();
                        markdown_line_transform(
                            &data_dir,
                            &git_pusher,
                            &md_trigger,
                            &source_type,
                            &v,
                            sid,
                            &rel_path,
                            idx,
                            |l| tidl_markdown::mark_tag_done_at(l, completed, ordinal),
                        )
                        .await?;
                    }
                }
            }
        }

        // For note-sourced todos being completed: rewrite the inline token to
        // its `#done/...` form in the note body, then drop the event. The body
        // is the record; re-opening is the reverse text rewrite, done from the
        // note editor, so no event is retained (backlog #113).
        if source == "note" && completed {
            use crate::note_tokens::{format_done, scan_note_tokens, slugify, DoneOriginal};

            let note_id: Option<i64> = row.try_get("note_id").unwrap_or_default();
            let note_id =
                note_id.ok_or_else(|| ServerFnError::new("Note task has no owning note"))?;

            let slug = row
                .try_get::<Option<String>, _>("topic")
                .unwrap_or_default()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    slugify(
                        &row.try_get::<Option<String>, _>("summary")
                            .unwrap_or_default()
                            .unwrap_or_default(),
                    )
                });
            if slug.is_empty() {
                return Err(ServerFnError::new("Cannot complete a task with no name"));
            }

            let completed_date = chrono::Utc::now().date_naive();
            let kind: String = row.try_get("kind").unwrap_or_default();
            let done_token = if kind == "await" {
                let due: Option<String> = row.try_get("due_date").unwrap_or_default();
                let date = due
                    .as_deref()
                    .and_then(|d| {
                        chrono::NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d").ok()
                    })
                    .ok_or_else(|| ServerFnError::new("Await task has no date"))?;
                format_done(completed_date, &DoneOriginal::Await { date }, &slug)
            } else {
                let priority = row
                    .try_get::<Option<i64>, _>("priority")
                    .unwrap_or_default()
                    .unwrap_or(50)
                    .clamp(1, 100) as u32;
                format_done(completed_date, &DoneOriginal::Task { priority }, &slug)
            };

            // `note:<id>:<line>:<ordinal>` (backlog #113).
            let parts: Vec<&str> = uid.split(':').collect();
            let line: Option<usize> = parts.get(2).and_then(|s| s.parse::<usize>().ok());
            let ordinal: usize = parts
                .get(3)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);

            let text: Option<String> = sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            let text = text.ok_or_else(|| ServerFnError::new("Note not found"))?;

            // Find the nth token on the recorded line and swap it in place.
            let target = line.and_then(|line_no| {
                scan_note_tokens(&text)
                    .into_iter()
                    .filter(|t| t.line == line_no)
                    .nth(ordinal)
            });
            if let Some(target) = target {
                let mut new_text = String::with_capacity(text.len() + done_token.len());
                new_text.push_str(&text[..target.start]);
                new_text.push_str(&done_token);
                new_text.push_str(&text[target.end..]);

                sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(&new_text)
                    .bind(note_id)
                    .execute(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }

            sqlx::query("DELETE FROM events WHERE uid = ? AND username = ?")
                .bind(&uid)
                .bind(&effective_username)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            if let Some(cal) = row
                .try_get::<Option<String>, _>("calendar")
                .unwrap_or_default()
            {
                bump_caldav_sync_token(&pool, &effective_username, &cal, &uid, "deleted").await;
            }

            return Ok(());
        }

        // Update status in the DB for immediate UI feedback.
        let new_status = if completed {
            "COMPLETED"
        } else {
            "NEEDS-ACTION"
        };
        sqlx::query("UPDATE events SET status = ? WHERE uid = ? AND username = ?")
            .bind(new_status)
            .bind(&uid)
            .bind(&effective_username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if !completed {
            let _ = crate::api::done::forget_done_item(&pool, &username, "caldav", &uid).await;
        }

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a CalDAV VTODO permanently.  Markdown-sourced todos cannot be
/// deleted via this endpoint (they are managed by the sync loop).
#[server(input = server_fn::codec::Json)]
pub async fn delete_todo_item(uid: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let row = sqlx::query(
            "SELECT calendar, source, note_id, source_line FROM events
             WHERE uid = ? AND username = ? AND type = 'VTODO'
               AND COALESCE(source, 'caldav') != 'markdown'",
        )
        .bind(&uid)
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let (calendar, source, note_id, source_line) = row
            .map(|r| {
                (
                    r.try_get::<Option<String>, _>("calendar").ok().flatten(),
                    r.try_get::<Option<String>, _>("source").ok().flatten(),
                    r.try_get::<Option<i64>, _>("note_id").ok().flatten(),
                    r.try_get::<Option<i64>, _>("source_line").ok().flatten(),
                )
            })
            .unwrap_or_default();

        sqlx::query(
            "DELETE FROM events
             WHERE uid = ? AND username = ? AND type = 'VTODO'
               AND COALESCE(source, 'caldav') != 'markdown'",
        )
        .bind(&uid)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Note-origin: also drop the inline token, otherwise the next note
        // save's re-sync would resurrect the deleted event (backlog #113).
        if source.as_deref() == Some("note") {
            if let (Some(note_id), Some(line)) = (note_id, source_line) {
                let ordinal = uid
                    .split(':')
                    .nth(3)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                remove_note_token(&pool, note_id, line.max(1) as usize, ordinal).await?;
            }
        }

        if let Some(cal) = calendar {
            bump_caldav_sync_token(&pool, &username, &cal, &uid, "deleted").await;
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Removes the `ordinal`th (0-based) inline token on 1-based `line` from a
/// note's body — used when deleting a note-origin task/await event (backlog
/// #112). No-op if the body no longer has that token.
#[cfg(feature = "ssr")]
async fn remove_note_token(
    pool: &sqlx::SqlitePool,
    note_id: i64,
    line: usize,
    ordinal: usize,
) -> Result<(), ServerFnError> {
    use crate::note_tokens::scan_note_tokens;

    let text: Option<String> = sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_optional(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let Some(text) = text else {
        return Ok(());
    };

    let target = scan_note_tokens(&text)
        .into_iter()
        .filter(|t| t.line == line)
        .nth(ordinal);
    let Some(target) = target else {
        return Ok(());
    };

    let mut new_text = String::with_capacity(text.len());
    new_text.push_str(&text[..target.start]);
    new_text.push_str(&text[target.end..]);

    sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(new_text.trim())
        .bind(note_id)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Rewrite the task/await tag on `rel_path`'s zero-based `line_idx` using
/// `transform` — the shared write-back for the list-view checkbox
/// (`complete_todo_item`) and the rendered-markdown checkboxes
/// (`complete_markdown_task` / `reopen_markdown_task`, backlog #112/#117).
/// Atomic write, git push for git sources, and an immediate re-sync trigger,
/// mirroring `save_markdown_file`. A line `transform` leaves unchanged, or an
/// out-of-range index, is a no-op.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn markdown_line_transform<F>(
    data_dir: &std::path::Path,
    git_pusher: &crate::ssr_types::GitCommitPusher,
    md_trigger: &crate::ssr_types::MarkdownSyncTrigger,
    source_type: &str,
    config: &serde_json::Value,
    source_id: i64,
    rel_path: &str,
    line_idx: usize,
    transform: F,
) -> Result<(), ServerFnError>
where
    F: Fn(&str) -> String,
{
    let base_dir =
        super::files::resolve_markdown_source_dir(source_type, config, source_id, data_dir)
            .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
    let abs_path = base_dir.join(rel_path);
    let content = tokio::fs::read_to_string(&abs_path)
        .await
        .map_err(|e| ServerFnError::new(format!("Cannot read file: {e}")))?;
    let new_content = transform_line_at(&content, line_idx, transform);
    if new_content == content {
        return Ok(());
    }
    markdown_write_content(
        data_dir,
        git_pusher,
        md_trigger,
        source_type,
        config,
        source_id,
        rel_path,
        new_content,
    )
    .await
}

/// Atomically write `new_content` to a markdown source file, push git sources
/// and trigger a re-sync — the shared write tail for the line transform above
/// and the whole-file Done clean-up rewrite (backlog #124).
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn markdown_write_content(
    data_dir: &std::path::Path,
    git_pusher: &crate::ssr_types::GitCommitPusher,
    md_trigger: &crate::ssr_types::MarkdownSyncTrigger,
    source_type: &str,
    config: &serde_json::Value,
    source_id: i64,
    rel_path: &str,
    new_content: String,
) -> Result<(), ServerFnError> {
    let base_dir =
        super::files::resolve_markdown_source_dir(source_type, config, source_id, data_dir)
            .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
    let abs_path = base_dir.join(rel_path);
    // Atomic write: hidden .tmp next to the target, then rename — same as
    // `save_markdown_file`.
    let tmp_path = {
        let mut p = abs_path.clone();
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        p.set_file_name(format!(".{name}.tmp"));
        p
    };
    tokio::fs::write(&tmp_path, new_content.as_bytes())
        .await
        .map_err(|e| ServerFnError::new(format!("Write failed: {e}")))?;
    tokio::fs::rename(&tmp_path, &abs_path)
        .await
        .map_err(|e| ServerFnError::new(format!("Rename failed: {e}")))?;

    // Mirrors save_markdown_file's data-safety stance: the local rewrite
    // already happened, so surface push failures rather than silently leaving
    // the remote out of sync with what the UI shows.
    if source_type == "git" {
        git_pusher.0(source_id, vec![rel_path.to_string()])
            .await
            .map_err(|e| ServerFnError::new(format!("Git push failed: {e}")))?;
    }
    let _ = md_trigger.0.send(source_id);
    Ok(())
}

/// Apply `transform` to `content`'s zero-indexed `line_idx` line, preserving
/// the file's original line-ending style. `str::lines()` treats `\r\n` and
/// `\n` identically, discarding the `\r` — rejoining with a hardcoded `"\n"`
/// would silently downgrade an entire CRLF file to LF just from editing one
/// tagged line in it. Detecting the file's separator once and rejoining with
/// that keeps every untouched line byte-for-byte identical.
#[cfg(feature = "ssr")]
fn transform_line_at(content: &str, line_idx: usize, transform: impl Fn(&str) -> String) -> String {
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if let Some(line) = lines.get_mut(line_idx) {
        *line = transform(line);
    }
    let mut new_content = lines.join(newline);
    if content.ends_with('\n') {
        new_content.push_str(newline);
    }
    new_content
}

/// Complete the markdown task/await on `line` (1-based, in the file `uid`
/// points at) by rewriting its tag to `#done/...` — the rendered-preview
/// checkbox (backlog #112). `ordinal` is the 0-based index of the tag within
/// that line (see `data-md-ordinal` in tidl-markdown), so a line holding
/// several tags completes exactly the clicked one. Unlike `complete_todo_item`,
/// this needs no pre-existing `events` row: a previewed file can contain
/// several tasks, and the clicked one need not be the uid's own line, so the
/// source is resolved purely from the uid.
#[server(input = server_fn::codec::Json)]
pub async fn complete_markdown_task(
    uid: String,
    line: u32,
    ordinal: u32,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        // Uid format: "md:{source_id}:{rel_path}:{line}:{ordinal}" — same as
        // get_markdown_rendered (rel_path may contain ':').
        let parsed = MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("Invalid todo uid format"))?;
        let rel_path = parsed.rel_path;

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();

        let line_idx = line.saturating_sub(1) as usize;
        let completed = chrono::Utc::now().date_naive();
        markdown_line_transform(
            &data_dir,
            &git_pusher,
            &md_trigger,
            &source_type,
            &v,
            source_id,
            rel_path,
            line_idx,
            |l| tidl_markdown::mark_tag_done_at(l, completed, ordinal as usize),
        )
        .await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Re-open the completed markdown task/await on `line` (1-based, in the file
/// `uid` points at) by rewriting its rich `#done` record back to the original
/// active tag — the rendered-preview done checkbox (backlog #117). `ordinal`
/// is the 0-based index of the rich `#done` tag within that line (see
/// `data-md-ordinal` in tidl-markdown), matching `complete_markdown_task`.
/// Mirrors `complete_markdown_task`: resolves the source purely from the uid
/// and needs no `events` row. A legacy `#done/<slug>` (nothing to restore) is
/// a no-op.
#[server(input = server_fn::codec::Json)]
pub async fn reopen_markdown_task(
    uid: String,
    line: u32,
    ordinal: u32,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        // Uid format: "md:{source_id}:{rel_path}:{line}:{ordinal}" — same as
        // get_markdown_rendered (rel_path may contain ':').
        let parsed = MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("Invalid todo uid format"))?;
        let rel_path = parsed.rel_path;

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();

        let line_idx = line.saturating_sub(1) as usize;
        markdown_line_transform(
            &data_dir,
            &git_pusher,
            &md_trigger,
            &source_type,
            &v,
            source_id,
            rel_path,
            line_idx,
            |l| tidl_markdown::reopen_tag_at(l, ordinal as usize).unwrap_or_else(|| l.to_string()),
        )
        .await?;
        let _ = crate::api::done::forget_done_item(&pool, &username, "markdown", &uid).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Remove the active task/await tag on `line` (1-based) from the markdown file
/// `uid` points at — the edit modal's "Remove await tag" action. `ordinal` is
/// the 0-based index of the tag within that line (see `data-md-ordinal` in
/// tidl-markdown). Strips only the tag itself, keeping any surrounding prose.
/// The matching `events` row is dropped immediately so lists update without
/// waiting for the source re-sync (which the write also triggers).
#[server(input = server_fn::codec::Json)]
pub async fn remove_markdown_task(
    uid: String,
    line: u32,
    ordinal: u32,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        // Uid format: "md:{source_id}:{rel_path}:{line}:{ordinal}" — same as
        // get_markdown_rendered (rel_path may contain ':').
        let parsed = MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("Invalid todo uid format"))?;
        let rel_path = parsed.rel_path;

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();

        let line_idx = line.saturating_sub(1) as usize;
        markdown_line_transform(
            &data_dir,
            &git_pusher,
            &md_trigger,
            &source_type,
            &v,
            source_id,
            rel_path,
            line_idx,
            |l| {
                tidl_markdown::remove_tag_at(l, ordinal as usize, false)
                    .unwrap_or_else(|| l.to_string())
            },
        )
        .await?;

        // Drop the row now instead of waiting for the re-sync so the Focus /
        // Tickler lists stop showing the removed await immediately.
        let event_uid = format!("md:{source_id}:{rel_path}:{line}:{ordinal}");
        sqlx::query("DELETE FROM events WHERE uid = ? AND username = ?")
            .bind(&event_uid)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Load a source-backed task/await chip for the edit modal (backlog #121).
/// Returns `None` when the addressed spot holds no active task/await — a
/// `#done` record, an empty position, or a line that no longer parses (so a
/// body click on a done pill opens nothing).
#[server(input = server_fn::codec::Json)]
pub async fn load_todo_chip(origin: TodoOrigin) -> Result<Option<TodoChip>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use super::notes::has_collection_access;
        use crate::note_tokens::{scan_note_tokens, slug_to_title, ScannedTokenKind};
        use sqlx::Row;

        let (pool, username) = auth().await?;

        match origin.clone() {
            TodoOrigin::Note {
                note_id,
                line,
                ordinal,
            } => {
                let collection_id: Option<i64> =
                    sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
                        .bind(note_id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                let collection_id =
                    collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;
                if !has_collection_access(&pool, &username, collection_id).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }
                let text: Option<String> =
                    sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
                        .bind(note_id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                let text = text.ok_or_else(|| ServerFnError::new("Note not found"))?;

                let target = scan_note_tokens(&text)
                    .into_iter()
                    .filter(|t| t.line == line as usize)
                    .nth(ordinal as usize);
                let Some(target) = target else {
                    return Ok(None);
                };
                let chip = match &target.kind {
                    ScannedTokenKind::Task { priority, slug } => TodoChip {
                        origin,
                        kind: "task".into(),
                        title: slug_to_title(slug),
                        priority: Some(*priority),
                        date: None,
                        dated: false,
                    },
                    ScannedTokenKind::Await { date, slug } => TodoChip {
                        origin,
                        kind: "await".into(),
                        title: slug_to_title(slug),
                        priority: None,
                        date: Some(date.format("%Y-%m-%d").to_string()),
                        dated: true,
                    },
                    ScannedTokenKind::Done { .. } => return Ok(None),
                };
                return Ok(Some(chip));
            }
            TodoOrigin::Markdown { uid, line, ordinal } => {
                use crate::ssr_types::DataDir;
                use axum::Extension;
                use leptos_axum::extract;

                let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

                let parsed = MarkdownUid::parse(&uid)
                    .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
                let source_id = parsed
                    .source_id_i64()
                    .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
                let rel_path = parsed.rel_path.to_string();

                let row = sqlx::query(
                    "SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?",
                )
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;
                let source_type: String = row.try_get("type").unwrap_or_default();
                let config_json: String = row.try_get("config").unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
                let base_dir = super::files::resolve_markdown_source_dir(
                    &source_type,
                    &v,
                    source_id,
                    &data_dir,
                )
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
                let content = tokio::fs::read_to_string(base_dir.join(&rel_path))
                    .await
                    .map_err(|e| ServerFnError::new(format!("Cannot read file: {e}")))?;

                let (tasks, awaits) = tidl_markdown::MarkdownParser::new().parse_all(&content);
                if let Some(t) = tasks
                    .into_iter()
                    .find(|i| i.line == line as usize && i.ordinal == ordinal as usize)
                {
                    return Ok(Some(TodoChip {
                        origin,
                        kind: "task".into(),
                        title: t.title,
                        priority: t.priority,
                        date: t.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
                        dated: t.priority.is_none(),
                    }));
                }
                if let Some(a) = awaits
                    .into_iter()
                    .find(|i| i.line == line as usize && i.ordinal == ordinal as usize)
                {
                    return Ok(Some(TodoChip {
                        origin,
                        kind: "await".into(),
                        title: a.title,
                        priority: None,
                        date: Some(a.date.format("%Y-%m-%d").to_string()),
                        dated: true,
                    }));
                }
                return Ok(None);
            }
        }
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Rewrite a source-backed task/await chip (backlog #121). Writes the new
/// title (as a slug) plus, for a priority task, the new priority, or, for an
/// await / dated task, the new date, back to the item's origin.
#[server(input = server_fn::codec::Json)]
pub async fn update_todo_chip(
    origin: TodoOrigin,
    title: String,
    priority: Option<u32>,
    date: Option<String>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use super::notes::{has_collection_access, sync_note_tokens};
        use crate::note_tokens::slugify;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let slug = slugify(title.trim());
        if slug.is_empty() {
            return Err(ServerFnError::new("A title is required."));
        }
        let parsed_date = match date.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => Some(
                chrono::NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d")
                    .map_err(|_| ServerFnError::new("Invalid date"))?,
            ),
            None => None,
        };

        match origin.clone() {
            TodoOrigin::Note {
                note_id,
                line,
                ordinal,
            } => {
                let collection_id: Option<i64> =
                    sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
                        .bind(note_id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                let collection_id =
                    collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;
                if !has_collection_access(&pool, &username, collection_id).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }
                let row = sqlx::query("SELECT text, calendar_name FROM notes WHERE id = ?")
                    .bind(note_id)
                    .fetch_one(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                let text: String = row.try_get("text").unwrap_or_default();
                let calendar: Option<String> = row.try_get("calendar_name").unwrap_or_default();

                let new_text = crate::note_tokens::edit_token_at(
                    &text,
                    line as usize,
                    ordinal as usize,
                    &slug,
                    priority,
                    parsed_date,
                )
                .ok_or_else(|| ServerFnError::new("Task/await not found in note"))?;

                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(&new_text)
                    .bind(note_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                sync_note_tokens(&mut tx, &username, note_id, calendar.as_deref(), &new_text)
                    .await?;
                tx.commit()
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                return Ok(());
            }
            TodoOrigin::Markdown { uid, line, ordinal } => {
                use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
                use axum::Extension;
                use leptos_axum::extract;

                let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
                let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
                let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

                let parsed = MarkdownUid::parse(&uid)
                    .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
                let source_id = parsed
                    .source_id_i64()
                    .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
                let rel_path = parsed.rel_path.to_string();

                let row = sqlx::query(
                    "SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?",
                )
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;
                let source_type: String = row.try_get("type").unwrap_or_default();
                let config_json: String = row.try_get("config").unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();

                let line_idx = (line as usize).saturating_sub(1);
                markdown_line_transform(
                    &data_dir,
                    &git_pusher,
                    &md_trigger,
                    &source_type,
                    &v,
                    source_id,
                    &rel_path,
                    line_idx,
                    |l| {
                        tidl_markdown::edit_tag_at(
                            l,
                            ordinal as usize,
                            &slug,
                            priority,
                            parsed_date,
                        )
                    },
                )
                .await?;
                return Ok(());
            }
        }
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Return AWAIT items: date-gated VTODOs from markdown sources and notes.
#[server]
pub async fn get_awaits() -> Result<Vec<AwaitItem>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT e.uid, e.summary, CAST(e.due_date AS TEXT) AS due_date, e.description,
                    e.topic, e.source_file, e.source_line, e.status, e.calendar,
                    nc.id AS collection_id, nc.name AS collection_name, nc.color AS collection_color,
                    n.id AS note_id,
                    (SELECT GROUP_CONCAT(l.id || CHAR(31) || l.name || CHAR(31) || l.color, CHAR(30))
                     FROM note_label_links lnk
                     JOIN note_labels l ON l.id = lnk.label_id
                     WHERE lnk.note_id = n.id) AS note_labels
             FROM events e
             LEFT JOIN notes n ON n.id = e.note_id
             LEFT JOIN note_collections nc ON nc.id = n.collection_id
             WHERE e.type = 'VTODO' AND e.username = ? AND e.kind = 'await'
             ORDER BY e.due_date ASC",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| {
                let status: Option<String> = r.try_get("status").unwrap_or_default();
                AwaitItem {
                    id: r.try_get("uid").unwrap_or_default(),
                    summary: r
                        .try_get::<Option<String>, _>("summary")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                    date: r
                        .try_get::<Option<String>, _>("due_date")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                    description: r.try_get("description").unwrap_or_default(),
                    topic: r.try_get("topic").unwrap_or_default(),
                    source_file: r.try_get("source_file").unwrap_or_default(),
                    source_line: r
                        .try_get::<Option<i64>, _>("source_line")
                        .unwrap_or_default()
                        .map(|v| v as u32),
                    completed: status.as_deref() == Some("COMPLETED"),
                    collection_id: r
                        .try_get::<Option<i64>, _>("collection_id")
                        .unwrap_or_default()
                        .map(|v| v.to_string()),
                    collection_name: r
                        .try_get::<Option<String>, _>("collection_name")
                        .unwrap_or_default(),
                    collection_color: r
                        .try_get::<Option<String>, _>("collection_color")
                        .unwrap_or_default(),
                    note_id: r.try_get::<Option<i64>, _>("note_id").unwrap_or_default(),
                    note_labels: r
                        .try_get::<Option<String>, _>("note_labels")
                        .unwrap_or_default()
                        .map(|s| parse_note_labels(&s))
                        .unwrap_or_default(),
                    calendar: r
                        .try_get::<Option<String>, _>("calendar")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                }
            })
            .collect());
    }

    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Return the content of a markdown source file for the Focus view Variant B preview.
///
/// Looks up the todo by uid (verifying ownership), resolves the absolute file path
/// from the registered markdown source config, and returns the filename, the
/// 1-based highlighted line, and all file lines.
#[server(input = server_fn::codec::Json)]
pub async fn get_markdown_file_content(
    uid: String,
) -> Result<(String, u32, Vec<String>), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        // Parse uid directly: "md:{source_id}:{rel_path}:{line}:{ordinal}"
        // This avoids a DB lookup that would fail if the event was already
        // deleted by the sync loop (e.g. after completing a markdown todo the
        // TODO tag is stripped, the file changes, and the sync re-processes it,
        // deleting and re-inserting events — completed items are not re-inserted).
        let parsed = MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("Invalid todo uid format"))?;
        let rel_path = parsed.rel_path.to_string();
        let source_line: i64 = parsed.line.parse().unwrap_or(1);

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &v, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;

        let abs_path = base_dir.join(&rel_path);
        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read file: {}", e)))?;

        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let filename = std::path::Path::new(&rel_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&rel_path)
            .to_string();

        return Ok((filename, source_line as u32, lines));
    }
    #[allow(unreachable_code)]
    Ok(("".to_string(), 1, vec![]))
}

/// The current user's own contact display names, for resolving `@mention`
/// syntax in a rendered markdown file (backlog #91) — mirrors
/// `contacts.rs`'s `list_contacts` scoping (`owner_username = ?`) but only
/// needs the name, not the full `ContactDto`.
#[cfg(feature = "ssr")]
async fn fetch_contact_names_for_mentions(
    pool: &sqlx::SqlitePool,
    username: &str,
) -> Result<Vec<String>, ServerFnError> {
    sqlx::query_scalar("SELECT display_name FROM contacts WHERE owner_username = ?")
        .bind(username)
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))
}

/// Return the rendered HTML of a markdown source file for the given todo uid.
///
/// Parses the uid (`"md:{source_id}:{rel_path}:{line}:{ordinal}"`), resolves the file
/// via the registered markdown source config, reads the content, and renders
/// it to HTML using `tidl_markdown::render_to_html`.
#[server(input = server_fn::codec::Json)]
pub async fn get_markdown_rendered(uid: String) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let contact_names = fetch_contact_names_for_mentions(&pool, &username).await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        // Parse uid: "md:{source_id}:{rel_path}:{line}:{ordinal}"
        // This avoids a DB lookup that would fail if the event was already
        // deleted by the sync loop — the uid itself carries all routing info.
        let parsed = MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("Invalid todo uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("Invalid todo uid format"))?;
        let rel_path = parsed.rel_path;

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &v, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;

        let abs_path = base_dir.join(rel_path);
        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read file: {}", e)))?;

        // Rewrite local relative image links to the authenticated
        // `/api/md-images/...` serving route so they render in the preview
        // (backlog #122) — otherwise a browser resolves them against the page
        // URL and 404s. Paths are relative to the file's own folder.
        let file_dir_rel = rel_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let content =
            super::consolidate::rewrite_markdown_image_dests(&content, source_id, file_dir_rel);

        // Colour task/await chips from the user's priority curve so a rendered
        // markdown chip matches what Notes/Focus show for the same priority
        // (backlog #116).
        let curves = super::settings::fetch_priority_curves(&pool, &username).await?;
        return Ok(tidl_markdown::render_to_html_with_urgency(
            &content,
            &contact_names,
            &|priority| crate::types::note_priority_urgency_color(priority, &curves),
        ));
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Create a new calendar event. Returns the UID of the created event.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn create_event(event: NewEvent) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;
        use tidl_caldav::etag::generate_etag;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;

        let uid = uuid::Uuid::new_v4().to_string();
        let alarm_ical = build_alarms_ical(&event.alarms);

        let uid_line = format!("UID:{uid}");
        let dtstamp_line = format!(
            "DTSTAMP:{}",
            fmt_ical_dt(if event.all_day {
                "2000-01-01 00:00"
            } else {
                &event.start
            }),
        );
        let (ical, start_dt, end_dt) =
            build_event_ical(&event, &uid_line, &dtstamp_line, &alarm_ical);
        let etag = generate_etag(&ical);

        let cal_name = if event.calendar.trim().is_empty() {
            "personal".to_string()
        } else {
            event.calendar.clone()
        };

        // A non-owner `calendar_owner` means the client picked a calendar
        // shared with the caller — re-verify the share is live and
        // read-write server-side (the dropdown filter is a UI convenience,
        // not a trust boundary) before writing under someone else's username.
        let effective_username = match &event.calendar_owner {
            Some(owner) if owner != &username => {
                let shared = caller_has_read_write_share(&pool, owner, &cal_name, &username)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                if !shared {
                    return Err(ServerFnError::new(
                        "Not authorized to create events on this calendar",
                    ));
                }
                owner.clone()
            }
            _ => username.clone(),
        };

        // Ensure the target calendar exists (create personal if missing).
        sqlx::query(
            "INSERT OR IGNORE INTO calendars (username, name, display_name, color) VALUES (?, ?, ?, ?)",
        )
        .bind(&effective_username).bind(&cal_name)
        .bind(if cal_name == "personal" { "Personal" } else { &cal_name })
        .bind("#3b82f6")
        .execute(&pool).await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            r#"INSERT OR REPLACE INTO events
               (username, calendar, uid, type, summary, description, location, priority,
                start_date, end_date, etag, ical_data, source, updated_at)
               VALUES (?, ?, ?, 'VEVENT', ?, ?, ?, ?,
                       datetime(?), datetime(?), ?, ?, 'web', datetime('now'))"#,
        )
        .bind(&effective_username)
        .bind(&cal_name)
        .bind(&uid)
        .bind(&event.summary)
        .bind(&event.description)
        .bind(&event.location)
        .bind(event.priority.map(|p| p.clamp(1, 100) as i64))
        .bind(&start_dt)
        .bind(&end_dt)
        .bind(&etag)
        .bind(&ical)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_caldav_sync_token(&pool, &effective_username, &cal_name, &uid, "modified").await;
        notify_calendar_watchers(&pool, &effective_username, &cal_name, ChangeKind::Events).await;
        return Ok(uid);
    }

    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Bump the CalDAV sync token for `calendar` and append a `sync_log` entry.
///
/// Fire-and-forget: errors are silently ignored — the event data is already
/// persisted, so a token bump failure is non-fatal.
#[cfg(feature = "ssr")]
pub(crate) async fn bump_caldav_sync_token(
    pool: &sqlx::SqlitePool,
    username: &str,
    calendar: &str,
    uid: &str,
    operation: &str,
) {
    use sqlx::Row;
    let _ = sqlx::query(
        "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
    )
    .bind(username)
    .bind(calendar)
    .execute(pool)
    .await;

    if let Ok(Some(row)) =
        sqlx::query("SELECT sync_token FROM calendars WHERE username = ? AND name = ?")
            .bind(username)
            .bind(calendar)
            .fetch_optional(pool)
            .await
    {
        let token: i64 = row.try_get("sync_token").unwrap_or(0);
        let _ = sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(username)
        .bind(calendar)
        .bind(uid)
        .bind(operation)
        .bind(token)
        .execute(pool)
        .await;
    }
}

/// Update only the start and end times of an existing event (used by drag-to-move/resize).
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn update_event_times(
    uid: String,
    /// "YYYY-MM-DD HH:MM"
    start: String,
    /// "YYYY-MM-DD HH:MM"
    end: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;
        use tidl_caldav::etag::generate_etag;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;
        let effective_username = match resolve_event_write_owner(&pool, &username, &uid).await {
            Ok(Some(owner)) => owner,
            _ => username.clone(),
        };

        let start_dt = format!("{}:00", start.trim());
        let end_dt = format!("{}:00", end.trim());

        let fmt_dt = |s: &str| -> String {
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
        };
        let ical_start = fmt_dt(&start);
        let ical_end = fmt_dt(&end);

        let row =
            sqlx::query("SELECT ical_data, calendar FROM events WHERE uid = ? AND username = ?")
                .bind(&uid)
                .bind(&effective_username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut updated_calendar: Option<String> = None;

        if let Some(row) = row {
            use sqlx::Row;
            let ical_data: String = row
                .try_get("ical_data")
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            let calendar: String = row.try_get("calendar").unwrap_or_default();

            let patched: String = ical_data
                .lines()
                .map(|line| {
                    if line.starts_with("DTSTART") {
                        format!("DTSTART:{}", ical_start)
                    } else if line.starts_with("DTEND") {
                        format!("DTEND:{}", ical_end)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\r\n");
            let patched = if patched.ends_with("\r\n") {
                patched
            } else {
                patched + "\r\n"
            };
            let new_etag = generate_etag(&patched);

            sqlx::query(
                "UPDATE events SET start_date = datetime(?), end_date = datetime(?), \
                 ical_data = ?, etag = ?, updated_at = datetime('now') WHERE uid = ? AND username = ?"
            )
            .bind(&start_dt)
            .bind(&end_dt)
            .bind(&patched)
            .bind(&new_etag)
            .bind(&uid)
            .bind(&effective_username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            // Bump CalDAV sync token so external clients (and remote sync) see the change.
            if !calendar.is_empty() {
                bump_caldav_sync_token(&pool, &effective_username, &calendar, &uid, "modified")
                    .await;
                updated_calendar = Some(calendar);
            }
        }

        match updated_calendar {
            Some(calendar) => {
                notify_calendar_watchers(&pool, &effective_username, &calendar, ChangeKind::Events)
                    .await
            }
            None => notify_change(&username, ChangeKind::Events).await,
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Update all editable fields of an existing event.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn update_event(uid: String, event: NewEvent) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};
        use tidl_caldav::etag::generate_etag;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;
        let effective_username = match resolve_event_write_owner(&pool, &username, &uid).await {
            Ok(Some(owner)) => owner,
            _ => username.clone(),
        };

        let alarm_ical = build_alarms_ical(&event.alarms);

        let row =
            sqlx::query("SELECT ical_data, calendar FROM events WHERE uid = ? AND username = ?")
                .bind(&uid)
                .bind(&effective_username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let old_calendar: String = row
            .as_ref()
            .and_then(|r| r.try_get("calendar").ok())
            .unwrap_or_default();
        let base_ical: String = row
            .and_then(|r| r.try_get("ical_data").ok())
            .unwrap_or_default();

        let uid_line = base_ical
            .lines()
            .find(|l| l.starts_with("UID:"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("UID:{}", uid));
        let dtstamp_line = base_ical
            .lines()
            .find(|l| l.starts_with("DTSTAMP:"))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "DTSTAMP:{}",
                    fmt_ical_dt(if event.all_day {
                        "2000-01-01 00:00"
                    } else {
                        &event.start
                    })
                )
            });

        let (ical, start_dt, end_dt) =
            build_event_ical(&event, &uid_line, &dtstamp_line, &alarm_ical);
        let etag = generate_etag(&ical);

        sqlx::query(
            "UPDATE events SET summary=?, description=?, location=?, priority=?, \
             start_date=datetime(?), end_date=datetime(?), \
             calendar=?, ical_data=?, etag=?, updated_at=datetime('now') WHERE uid=? AND username=?"
        )
        .bind(&event.summary).bind(&event.description).bind(&event.location)
        .bind(event.priority.map(|p| p.clamp(1, 100) as i64))
        .bind(&start_dt).bind(&end_dt).bind(&event.calendar).bind(&ical).bind(&etag).bind(&uid).bind(&effective_username)
        .execute(&pool).await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Bump CalDAV sync tokens so external clients see the change. Keyed to
        // effective_username (the calendar's real owner — same as `username`
        // for the caller's own events, or the share grantor when writing to a
        // shared calendar) since that's whose (username, calendar) sync token
        // actually needs to advance.
        if !old_calendar.is_empty() && old_calendar != event.calendar {
            bump_caldav_sync_token(&pool, &effective_username, &old_calendar, &uid, "deleted")
                .await;
            // The event left this calendar — its watchers need to know too,
            // not just the destination calendar's (backlog #59).
            notify_calendar_watchers(
                &pool,
                &effective_username,
                &old_calendar,
                ChangeKind::Events,
            )
            .await;
        }
        bump_caldav_sync_token(
            &pool,
            &effective_username,
            &event.calendar,
            &uid,
            "modified",
        )
        .await;

        notify_calendar_watchers(
            &pool,
            &effective_username,
            &event.calendar,
            ChangeKind::Events,
        )
        .await;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Delete an existing event owned by the current user.
#[allow(unused_variables)]
#[server]
pub async fn delete_event(uid: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let effective_username = match resolve_event_write_owner(&pool, &username, &uid).await {
            Ok(Some(owner)) => owner,
            _ => username.clone(),
        };

        let row = sqlx::query("SELECT calendar FROM events WHERE uid = ? AND username = ?")
            .bind(&uid)
            .bind(&effective_username)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let calendar: Option<String> = row.and_then(|r| r.try_get("calendar").ok());

        // Delete master and all exception occurrence rows (uid LIKE '{uid}_R%').
        sqlx::query("DELETE FROM events WHERE username = ? AND (uid = ? OR uid LIKE ? || '_R%')")
            .bind(&effective_username)
            .bind(&uid)
            .bind(&uid)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if let Some(cal) = calendar {
            bump_caldav_sync_token(&pool, &effective_username, &cal, &uid, "deleted").await;
            notify_calendar_watchers(&pool, &effective_username, &cal, ChangeKind::Events).await;
        } else {
            notify_change(&username, ChangeKind::Events).await;
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Resolve a wikilink name to a file UID for navigation.
///
/// Walks the markdown source directory of the file identified by `current_uid`
/// and returns the UID + relative path of the first `.md` file whose stem
/// matches `name` (case-insensitive).  Returns `None` when no match is found.
#[server(input = server_fn::codec::Json)]
pub async fn resolve_wikilink(
    current_uid: String,
    name: String,
) -> Result<Option<ResolvedWikilink>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        // Extract source_id from current_uid: "md:{source_id}:{rel_path}:{line}:{ordinal}"
        let source_id: i64 = MarkdownUid::parse(&current_uid)
            .and_then(|u| u.source_id_i64())
            .ok_or_else(|| ServerFnError::new("Invalid uid format"))?;

        let row = sqlx::query(
            "SELECT type, config FROM markdown_sources WHERE id = ? AND username = ? AND enabled = 1",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_str: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &v, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
        let base_path = base_dir.to_string_lossy().to_string();

        let name_lower = name.to_lowercase();
        let rel_path = walk_for_stem(std::path::Path::new(&base_path), &name_lower, &base_path);

        return Ok(rel_path.map(|rel| ResolvedWikilink {
            uid: format!("md:{}:{}:1:0", source_id, rel),
            source_file: rel,
        }));
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Return true when a file stem matches a wikilink name, treating hyphens and
/// spaces as equivalent (e.g. `it-alerting-and-monitoring` matches `IT Alerting And Monitoring`).
#[cfg(feature = "ssr")]
fn stem_matches(stem: &str, name_lower: &str) -> bool {
    stem.to_lowercase().replace('-', " ") == name_lower.replace('-', " ")
}

/// Walk dir looking for `{name_lower}.md`; returns relative path from base on first match.
#[cfg(feature = "ssr")]
fn walk_for_stem(dir: &std::path::Path, name_lower: &str, base: &str) -> Option<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(r) = walk_for_stem(&path, name_lower, base) {
                return Some(r);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem_matches(stem, name_lower) {
                    if let Some(rel) = path.strip_prefix(base).ok().and_then(|p| p.to_str()) {
                        return Some(rel.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Create or replace a single-occurrence exception VEVENT for a recurring series.
///
/// The exception VEVENT is stored under uid `{series_uid}_R{occurrence_start}`.
/// It carries a `RECURRENCE-ID` property so CalDAV clients recognise it as an
/// override, not a standalone event.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn update_event_occurrence(
    series_uid: String,
    occurrence_start: String,
    event: NewEvent,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};
        use tidl_caldav::etag::generate_etag;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;
        let effective_username =
            match resolve_event_write_owner(&pool, &username, &series_uid).await {
                Ok(Some(owner)) => owner,
                _ => username.clone(),
            };

        // Fetch master to verify ownership and extract stable UID/DTSTAMP.
        let master_row = sqlx::query(
            "SELECT ical_data, calendar FROM events
             WHERE uid = ? AND username = ? AND type = 'VEVENT'",
        )
        .bind(&series_uid)
        .bind(&effective_username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Event not found"))?;

        let master_ical: String = master_row.try_get("ical_data").unwrap_or_default();
        let calendar: String = master_row.try_get("calendar").unwrap_or_default();

        let dtstamp_line = master_ical
            .lines()
            .find(|l| l.starts_with("DTSTAMP:"))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "DTSTAMP:{}",
                    fmt_ical_dt(if event.all_day {
                        "2000-01-01 00:00"
                    } else {
                        &event.start
                    }),
                )
            });

        // Build exception VEVENT: same as build_event_ical but without RRULE,
        // plus a RECURRENCE-ID line.
        let alarm_ical = build_alarms_ical(&event.alarms);
        let exception_uid_line = format!("UID:{series_uid}_R{occurrence_start}");
        let event_for_ical = NewEvent {
            rrule: None,
            ..event
        };
        let (mut ical, start_dt, end_dt) = build_event_ical(
            &event_for_ical,
            &exception_uid_line,
            &dtstamp_line,
            &alarm_ical,
        );
        // Insert RECURRENCE-ID before END:VEVENT.
        // All-day occurrences use VALUE=DATE (8-digit YYYYMMDD); timed ones use
        // the default datetime form.
        let recurrence_id_line = if occurrence_start.len() == 8 {
            format!("RECURRENCE-ID;VALUE=DATE:{}", occurrence_start)
        } else {
            format!("RECURRENCE-ID:{}", occurrence_start)
        };
        ical = ical.replacen(
            "END:VEVENT",
            &format!("{}\r\nEND:VEVENT", recurrence_id_line),
            1,
        );
        let exception_uid = format!("{series_uid}_R{occurrence_start}");
        let etag = generate_etag(&ical);

        sqlx::query(
            r#"INSERT OR REPLACE INTO events
               (username, calendar, uid, type, summary, description, location, priority,
                start_date, end_date, etag, ical_data, source, updated_at)
               VALUES (?, ?, ?, 'VEVENT', ?, ?, ?, ?,
                       datetime(?), datetime(?), ?, ?, 'web', datetime('now'))"#,
        )
        .bind(&effective_username)
        .bind(&calendar)
        .bind(&exception_uid)
        .bind(&event_for_ical.summary)
        .bind(&event_for_ical.description)
        .bind(&event_for_ical.location)
        .bind(event_for_ical.priority.map(|p| p.clamp(1, 100) as i64))
        .bind(&start_dt)
        .bind(&end_dt)
        .bind(&etag)
        .bind(&ical)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_caldav_sync_token(
            &pool,
            &effective_username,
            &calendar,
            &exception_uid,
            "modified",
        )
        .await;
        notify_calendar_watchers(&pool, &effective_username, &calendar, ChangeKind::Events).await;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a single occurrence from a recurring series.
///
/// Appends an `EXDATE` to the master VEVENT's iCal data (so the rrule
/// crate will skip that date on future expansions) and removes any existing
/// exception row for that occurrence.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn delete_event_occurrence(
    series_uid: String,
    occurrence_start: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};
        use tidl_caldav::etag::generate_etag;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;
        let effective_username =
            match resolve_event_write_owner(&pool, &username, &series_uid).await {
                Ok(Some(owner)) => owner,
                _ => username.clone(),
            };

        // Fetch master and verify ownership.
        let master_row = sqlx::query(
            "SELECT ical_data, calendar FROM events
             WHERE uid = ? AND username = ? AND type = 'VEVENT'",
        )
        .bind(&series_uid)
        .bind(&effective_username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Event not found"))?;

        let master_ical: String = master_row.try_get("ical_data").unwrap_or_default();
        let calendar: String = master_row.try_get("calendar").unwrap_or_default();

        // Add EXDATE to master.
        let patched_ical = add_exdate_to_master(&master_ical, &occurrence_start);
        let new_etag = generate_etag(&patched_ical);

        sqlx::query(
            "UPDATE events SET ical_data = ?, etag = ?, updated_at = datetime('now')
             WHERE uid = ? AND username = ?",
        )
        .bind(&patched_ical)
        .bind(&new_etag)
        .bind(&series_uid)
        .bind(&effective_username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Remove any existing exception row for this occurrence.
        let exception_uid = format!("{series_uid}_R{occurrence_start}");
        sqlx::query("DELETE FROM events WHERE uid = ? AND username = ?")
            .bind(&exception_uid)
            .bind(&effective_username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_caldav_sync_token(
            &pool,
            &effective_username,
            &calendar,
            &series_uid,
            "modified",
        )
        .await;
        notify_calendar_watchers(&pool, &effective_username, &calendar, ChangeKind::Events).await;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::stem_matches;

    mod set_vtodo_prop_tests {
        use super::super::{fold_ical_line, set_vtodo_prop};

        fn ical() -> String {
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:abc\r\nSUMMARY:Old\r\n\
             PRIORITY:5\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
                .to_string()
        }

        #[test]
        fn replaces_summary_and_preserves_other_lines() {
            let out = set_vtodo_prop(&ical(), "SUMMARY", Some("SUMMARY:New title"));
            assert!(out.contains("SUMMARY:New title\r\n"));
            assert!(!out.contains("SUMMARY:Old"));
            assert!(out.contains("PRIORITY:5\r\n"));
            assert!(out.contains("UID:abc\r\n"));
        }

        #[test]
        fn replaces_property_with_a_parameter() {
            let out = set_vtodo_prop(&ical(), "DUE", Some("DUE;VALUE=DATE:20260801"));
            assert!(out.contains("DUE;VALUE=DATE:20260801\r\nEND:VTODO"));
        }

        #[test]
        fn removes_property_when_none() {
            let out = set_vtodo_prop(&ical(), "PRIORITY", None);
            assert!(!out.contains("PRIORITY"));
        }

        #[test]
        fn drops_folded_continuations_of_replaced_property() {
            let src = "BEGIN:VTODO\r\nSUMMARY:line one\r\n continuation\r\n\
                       STATUS:NEEDS-ACTION\r\nEND:VTODO\r\n";
            let out = set_vtodo_prop(src, "SUMMARY", Some("SUMMARY:new"));
            assert!(!out.contains("continuation"));
            assert!(out.contains("SUMMARY:new"));
            assert!(out.contains("STATUS:NEEDS-ACTION"));
        }

        #[test]
        fn folds_long_values() {
            let long = format!("SUMMARY:{}", "x".repeat(200));
            let folded = fold_ical_line(&long);
            for line in folded.split("\r\n") {
                assert!(line.len() <= 75, "line too long: {}", line.len());
            }
        }
    }

    mod fetch_contact_names_for_mentions_tests {
        use super::super::fetch_contact_names_for_mentions;

        #[sqlx::test(migrations = "../../migrations")]
        async fn returns_only_the_caller_own_contacts(pool: sqlx::SqlitePool) {
            sqlx::query(
                "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
            ).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('bob', 'bob@example.com', 'bob', 'x', 0)"
            ).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO contacts (owner_username, uid, display_name, etag, grid_row, grid_col, grid_row_span)
                 VALUES ('alice', 'u1', 'Jane Doe', 'e1', 0, 0, 1)",
            ).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO contacts (owner_username, uid, display_name, etag, grid_row, grid_col, grid_row_span)
                 VALUES ('bob', 'u2', 'Someone Else', 'e2', 0, 0, 1)",
            ).execute(&pool).await.unwrap();

            let names = fetch_contact_names_for_mentions(&pool, "alice")
                .await
                .unwrap();
            assert_eq!(names, vec!["Jane Doe".to_string()]);
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn returns_empty_when_the_caller_has_no_contacts(pool: sqlx::SqlitePool) {
            sqlx::query(
                "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
            ).execute(&pool).await.unwrap();

            let names = fetch_contact_names_for_mentions(&pool, "alice")
                .await
                .unwrap();
            assert!(names.is_empty());
        }
    }

    #[test]
    fn test_stem_matches_exact() {
        assert!(stem_matches("backup", "backup"));
        assert!(stem_matches("Backup", "backup"));
    }

    #[test]
    fn test_stem_matches_hyphen_vs_space() {
        // slug file, spaced link
        assert!(stem_matches(
            "it-alerting-and-monitoring",
            "it alerting and monitoring"
        ));
        // spaced file, slug link
        assert!(stem_matches(
            "IT Alerting And Monitoring",
            "it-alerting-and-monitoring"
        ));
    }

    #[test]
    fn test_stem_matches_no_false_positives() {
        assert!(!stem_matches("backup", "other"));
        assert!(!stem_matches("it-alerting", "it alerting and monitoring"));
    }

    // ── clamp_range ───────────────────────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod clamp_range_tests {
        use super::super::clamp_range;

        #[test]
        fn within_max_span_is_unchanged() {
            assert_eq!(
                clamp_range("2026-05-01", "2026-05-31"),
                ("2026-05-01".to_string(), "2026-05-31".to_string())
            );
        }

        #[test]
        fn reversed_bounds_are_swapped() {
            assert_eq!(
                clamp_range("2026-05-31", "2026-05-01"),
                ("2026-05-01".to_string(), "2026-05-31".to_string())
            );
        }

        #[test]
        fn span_beyond_max_is_clamped() {
            let (start, end) = clamp_range("2020-01-01", "2040-01-01");
            assert_eq!(start, "2020-01-01");
            // Clamped to MAX_RANGE_DAYS (366*5) from start, not the requested end.
            assert_ne!(end, "2040-01-01");
            let s = chrono::NaiveDate::parse_from_str(&start, "%Y-%m-%d").unwrap();
            let e = chrono::NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap();
            assert_eq!((e - s).num_days(), 366 * 5);
        }

        #[test]
        fn unparseable_bounds_pass_through_unchanged() {
            assert_eq!(
                clamp_range("not-a-date", "2026-05-31"),
                ("not-a-date".to_string(), "2026-05-31".to_string())
            );
        }
    }

    // ── effective_event_priority ─────────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod effective_event_priority_tests {
        use super::super::effective_event_priority;

        const ICAL_PRIORITY_9: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n\
             UID:x\r\nSUMMARY:X\r\nPRIORITY:9\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        const ICAL_PRIORITY_0: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n\
             UID:x\r\nSUMMARY:X\r\nPRIORITY:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

        #[test]
        fn db_value_wins_over_ical() {
            assert_eq!(
                effective_event_priority(Some(50), ICAL_PRIORITY_9),
                Some(50)
            );
        }

        #[test]
        fn falls_back_to_ical_when_column_is_null() {
            assert_eq!(effective_event_priority(None, ICAL_PRIORITY_9), Some(100));
        }

        #[test]
        fn zero_and_absent_mean_no_priority() {
            assert_eq!(effective_event_priority(None, ICAL_PRIORITY_0), None);
            assert_eq!(effective_event_priority(Some(0), ICAL_PRIORITY_9), None);
            assert_eq!(
                effective_event_priority(None, "BEGIN:VEVENT\r\nEND:VEVENT\r\n"),
                None
            );
        }
    }

    // ── sql_prefilter_bounds ─────────────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod sql_prefilter_bounds_tests {
        use super::super::sql_prefilter_bounds;

        #[test]
        fn pads_one_day_on_each_side() {
            assert_eq!(
                sql_prefilter_bounds("2026-07-01", "2026-07-31"),
                (
                    "2026-06-30 00:00:00".to_string(),
                    "2026-08-01 23:59:59".to_string()
                )
            );
        }

        #[test]
        fn unparseable_bounds_fall_back_to_wide_open() {
            assert_eq!(
                sql_prefilter_bounds("not-a-date", "2026-07-31"),
                (
                    "0000-01-01 00:00:00".to_string(),
                    "2026-08-01 23:59:59".to_string()
                )
            );
            assert_eq!(
                sql_prefilter_bounds("2026-07-01", "not-a-date"),
                (
                    "2026-06-30 00:00:00".to_string(),
                    "9999-12-31 23:59:59".to_string()
                )
            );
        }
    }

    // ── event_row_to_web_events ──────────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod event_row_to_web_events_tests {
        use super::super::{event_row_to_web_events, range_bounds};

        /// Regression test for a code-review finding: an indefinitely-recurring
        /// RRULE (no UNTIL/COUNT) combined with an unparseable `range_end` used
        /// to fall back to `chrono::DateTime::<Utc>::MAX_UTC`. `expand_rrule`
        /// bounds its occurrence iterator with `take_while(dt <= range_end)`,
        /// which never becomes false against MAX_UTC — the underlying
        /// `RRuleSet` iterator for a DAILY rule with no end has no natural
        /// stopping point either, so the old behavior would iterate forever
        /// and hang the request thread. Under the fix, an unparseable
        /// `range_end` falls back to `range_start_dt + MAX_RANGE_DAYS`, so
        /// expansion is bounded and this test completes quickly with a small,
        /// finite result instead of hanging.
        #[test]
        fn garbage_range_end_bounds_indefinite_rrule_expansion() {
            let ical_data = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Berlin:20260105T180000\r\n\
DTEND;TZID=Europe/Berlin:20260105T190000\r\n\
RRULE:FREQ=DAILY\r\n\
SUMMARY:Indefinite Daily\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

            let (range_start_dt, range_end_dt) =
                range_bounds("2026-01-05", "not-a-date", "Europe/Berlin");
            let events = event_row_to_web_events(
                "indefinite-uid".to_string(),
                "Indefinite Daily".to_string(),
                "personal".to_string(),
                None,
                None,
                None,
                "2026-01-05 18:00:00".to_string(),
                "2026-01-05 19:00:00".to_string(),
                ical_data,
                &std::collections::HashSet::new(),
                "Europe/Berlin",
                range_start_dt,
                range_end_dt,
            );

            // Bounded fallback is MAX_RANGE_DAYS (366*5) wide, so a DAILY rule
            // produces at most MAX_RANGE_DAYS + 1 occurrences — thousands, not
            // an unbounded/hung computation.
            assert!(
                !events.is_empty(),
                "expected at least one occurrence for an indefinite DAILY RRULE"
            );
            assert!(
                events.len() as i64 <= super::super::MAX_RANGE_DAYS + 1,
                "expected a bounded number of occurrences (<= {} + 1), got {}; \
                 an unbounded fallback range would never have completed this test",
                super::super::MAX_RANGE_DAYS,
                events.len()
            );
        }

        fn plain_ical(start: &str, end: &str) -> String {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:{start}\r\nDTEND:{end}\r\n\
                 SUMMARY:Plain\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        }

        /// `get_events` used to return every non-recurring event a user had
        /// ever created on every request — the non-recurring fallback path
        /// never checked `range_start`/`range_end` at all. An event with a
        /// fixed date far outside the requested window must now be excluded.
        #[test]
        fn plain_event_outside_range_is_excluded() {
            let ical = plain_ical("20200601T100000Z", "20200601T110000Z");
            let (range_start_dt, range_end_dt) = range_bounds("2026-07-01", "2026-07-31", "UTC");
            let events = event_row_to_web_events(
                "uid".to_string(),
                "Plain".to_string(),
                "personal".to_string(),
                None,
                None,
                None,
                "2020-06-01 10:00:00".to_string(),
                "2020-06-01 11:00:00".to_string(),
                &ical,
                &std::collections::HashSet::new(),
                "UTC",
                range_start_dt,
                range_end_dt,
            );
            assert!(
                events.is_empty(),
                "event in 2020 should not appear in a 2026-07 window"
            );
        }

        #[test]
        fn plain_event_inside_range_is_included() {
            let ical = plain_ical("20260715T100000Z", "20260715T110000Z");
            let (range_start_dt, range_end_dt) = range_bounds("2026-07-01", "2026-07-31", "UTC");
            let events = event_row_to_web_events(
                "uid".to_string(),
                "Plain".to_string(),
                "personal".to_string(),
                None,
                None,
                None,
                "2026-07-15 10:00:00".to_string(),
                "2026-07-15 11:00:00".to_string(),
                &ical,
                &std::collections::HashSet::new(),
                "UTC",
                range_start_dt,
                range_end_dt,
            );
            assert_eq!(events.len(), 1);
        }

        #[test]
        fn plain_event_carries_its_priority() {
            let ical = plain_ical("20260715T100000Z", "20260715T110000Z");
            let (range_start_dt, range_end_dt) = range_bounds("2026-07-01", "2026-07-31", "UTC");
            let events = event_row_to_web_events(
                "uid".to_string(),
                "Plain".to_string(),
                "personal".to_string(),
                None,
                None,
                Some(50),
                "2026-07-15 10:00:00".to_string(),
                "2026-07-15 11:00:00".to_string(),
                &ical,
                &std::collections::HashSet::new(),
                "UTC",
                range_start_dt,
                range_end_dt,
            );
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].priority, Some(50));
        }

        /// Regression test for a related bug found while fixing the above:
        /// `range_end` was resolved to *midnight* of the last day instead of
        /// its end, so anything later that day was wrongly excluded.
        #[test]
        fn plain_event_late_on_the_last_day_of_range_is_included() {
            let ical = plain_ical("20260731T220000Z", "20260731T230000Z");
            let (range_start_dt, range_end_dt) = range_bounds("2026-07-01", "2026-07-31", "UTC");
            let events = event_row_to_web_events(
                "uid".to_string(),
                "Plain".to_string(),
                "personal".to_string(),
                None,
                None,
                None,
                "2026-07-31 22:00:00".to_string(),
                "2026-07-31 23:00:00".to_string(),
                &ical,
                &std::collections::HashSet::new(),
                "UTC",
                range_start_dt,
                range_end_dt,
            );
            assert_eq!(
                events.len(),
                1,
                "an event at 22:00 on the last day of the range must not be excluded"
            );
        }

        #[test]
        fn plain_multiday_event_overlapping_range_start_is_included() {
            // Starts before the range, ends inside it — must still appear.
            let ical = plain_ical("20260625T100000Z", "20260705T100000Z");
            let (range_start_dt, range_end_dt) = range_bounds("2026-07-01", "2026-07-31", "UTC");
            let events = event_row_to_web_events(
                "uid".to_string(),
                "Plain".to_string(),
                "personal".to_string(),
                None,
                None,
                None,
                "2026-06-25 10:00:00".to_string(),
                "2026-07-05 10:00:00".to_string(),
                &ical,
                &std::collections::HashSet::new(),
                "UTC",
                range_start_dt,
                range_end_dt,
            );
            assert_eq!(events.len(), 1);
        }
    }

    // ── list_shared_calendar_events (backlog #41) ──────────────────────────────

    #[cfg(feature = "ssr")]
    mod list_shared_calendar_events_tests {
        use super::super::list_shared_calendar_events;
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::SqlitePool;
        use std::str::FromStr;

        async fn test_pool() -> SqlitePool {
            let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
            pool
        }

        async fn seed_user(pool: &SqlitePool, username: &str) {
            sqlx::query("INSERT INTO users (username, email, password_hash) VALUES (?, ?, 'x')")
                .bind(username)
                .bind(format!("{username}@test"))
                .execute(pool)
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn returns_events_from_a_shared_calendar() {
            let pool = test_pool().await;
            seed_user(&pool, "alice").await;
            seed_user(&pool, "bob").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES ('alice', 'family', 'bob', 'read')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'evt1', 'VEVENT', 'Shared Event', \
                 '2026-08-01 10:00:00', '2026-08-01 11:00:00', 'BEGIN:VCALENDAR...', 'etag1')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "0000-01-01 00:00:00",
                "9999-12-31 23:59:59",
            )
            .await
            .unwrap();
            assert_eq!(shared.len(), 1);
            assert_eq!(shared[0].summary, "Shared Event");
            assert_eq!(shared[0].owner_username, "alice");
            assert_eq!(shared[0].shared_permission, Some("read".to_string()));
        }

        #[tokio::test]
        async fn returns_nothing_for_a_user_with_no_shares() {
            let pool = test_pool().await;
            seed_user(&pool, "bob").await;

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "0000-01-01 00:00:00",
                "9999-12-31 23:59:59",
            )
            .await
            .unwrap();
            assert!(shared.is_empty());
        }

        // ── SQL_RANGE_PREFILTER ─────────────────────────────────────────────

        async fn seed_family_share(pool: &SqlitePool) {
            seed_user(pool, "alice").await;
            seed_user(pool, "bob").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES ('alice', 'family', 'bob', 'read')",
            )
            .execute(pool)
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn prefilter_excludes_a_plain_event_outside_the_requested_range() {
            let pool = test_pool().await;
            seed_family_share(&pool).await;
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'evt1', 'VEVENT', 'Old Event', \
                 '2010-01-01 10:00:00', '2010-01-01 11:00:00', \
                 'BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20100101T100000Z\nEND:VEVENT\nEND:VCALENDAR', 'etag1')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "2026-06-30 00:00:00",
                "2026-08-01 23:59:59",
            )
            .await
            .unwrap();
            assert!(
                shared.is_empty(),
                "an event from 2010 must not survive a 2026-07 prefilter"
            );
        }

        /// Regression guard: a recurrence-exception row must always survive
        /// the prefilter regardless of its own (relocated) date, because
        /// `get_events`'s `exception_map` — built from exactly these rows —
        /// is what stops RRULE expansion from re-emitting a phantom
        /// duplicate at the occurrence's *original* date, which can fall
        /// inside the visible window even when the exception's relocated
        /// date does not (see `phantom_duplicate_excluded_by_exception_map`
        /// in `api::rrule_tests` for the expansion-side half of this).
        #[tokio::test]
        async fn prefilter_always_includes_exception_rows() {
            let pool = test_pool().await;
            seed_family_share(&pool).await;
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'series1_R20100101T100000', 'VEVENT', 'Moved Occurrence', \
                 '2010-02-01 10:00:00', '2010-02-01 11:00:00', \
                 'BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20100201T100000Z\nEND:VEVENT\nEND:VCALENDAR', 'etag1')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "2026-06-30 00:00:00",
                "2026-08-01 23:59:59",
            )
            .await
            .unwrap();
            assert_eq!(
                shared.len(), 1,
                "an exception row must survive the prefilter even when its own date is out of range"
            );
        }

        /// A recurring master event must always survive the prefilter
        /// regardless of its own (first-occurrence) date, since later
        /// occurrences of an ongoing series can fall inside the visible
        /// window even when the series' DTSTART does not.
        #[tokio::test]
        async fn prefilter_always_includes_recurring_rows() {
            let pool = test_pool().await;
            seed_family_share(&pool).await;
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'series2', 'VEVENT', 'Weekly Standup', \
                 '2010-03-01 10:00:00', '2010-03-01 11:00:00', \
                 'BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20100301T100000Z\nRRULE:FREQ=WEEKLY\nEND:VEVENT\nEND:VCALENDAR', 'etag1')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "2026-06-30 00:00:00",
                "2026-08-01 23:59:59",
            )
            .await
            .unwrap();
            assert_eq!(
                shared.len(), 1,
                "an indefinitely-recurring series must survive the prefilter even when its DTSTART is out of range"
            );
        }

        /// Regression test: `NULL >= ?` evaluates to `NULL` (not true) in
        /// SQLite, so a plain string comparison silently drops rows with no
        /// `end_date` — found via a diagnostic diff against real account
        /// data (Google-imported events with no DTEND) while developing
        /// this filter. `event_row_to_web_events` fails *open* on an
        /// unparseable date rather than hiding the event, so the prefilter
        /// must too.
        #[tokio::test]
        async fn prefilter_always_includes_rows_with_a_null_end_date() {
            let pool = test_pool().await;
            seed_family_share(&pool).await;
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'no-dtend', 'VEVENT', 'No End Date', \
                 '2010-01-01 10:00:00', NULL, \
                 'BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20100101T100000Z\nEND:VEVENT\nEND:VCALENDAR', 'etag1')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let shared = list_shared_calendar_events(
                &pool,
                "bob",
                "2026-06-30 00:00:00",
                "2026-08-01 23:59:59",
            )
            .await
            .unwrap();
            assert_eq!(
                shared.len(), 1,
                "a row with a NULL end_date must survive the prefilter even when start_date is out of range"
            );
        }
    }

    // ── caller_has_read_write_share (shared-calendar new-event dropdown) ──────

    #[cfg(feature = "ssr")]
    mod caller_has_read_write_share_tests {
        use super::super::caller_has_read_write_share;
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::SqlitePool;
        use std::str::FromStr;

        async fn test_pool() -> SqlitePool {
            let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
            pool
        }

        async fn seed_user(pool: &SqlitePool, username: &str) {
            sqlx::query("INSERT INTO users (username, email, password_hash) VALUES (?, ?, 'x')")
                .bind(username)
                .bind(format!("{username}@test"))
                .execute(pool)
                .await
                .unwrap();
        }

        async fn seed_share(pool: &SqlitePool, permission: &str) {
            seed_user(pool, "alice").await;
            seed_user(pool, "bob").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES ('alice', 'family', 'bob', ?)",
            )
            .bind(permission)
            .execute(pool)
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn allows_read_write_share() {
            let pool = test_pool().await;
            seed_share(&pool, "read-write").await;

            let ok = caller_has_read_write_share(&pool, "alice", "family", "bob")
                .await
                .unwrap();
            assert!(ok);
        }

        #[tokio::test]
        async fn rejects_read_only_share() {
            let pool = test_pool().await;
            seed_share(&pool, "read").await;

            let ok = caller_has_read_write_share(&pool, "alice", "family", "bob")
                .await
                .unwrap();
            assert!(
                !ok,
                "read-only grantee must not be authorized to create events"
            );
        }

        #[tokio::test]
        async fn rejects_when_no_share_exists() {
            let pool = test_pool().await;
            seed_user(&pool, "alice").await;
            seed_user(&pool, "bob").await;

            let ok = caller_has_read_write_share(&pool, "alice", "family", "bob")
                .await
                .unwrap();
            assert!(!ok, "no share at all must not be authorized");
        }
    }

    // ── resolve_event_write_owner (backlog #41) ────────────────────────────────

    #[cfg(feature = "ssr")]
    mod resolve_event_write_owner_tests {
        use super::super::resolve_event_write_owner;
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::SqlitePool;
        use std::str::FromStr;

        async fn test_pool() -> SqlitePool {
            let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
            pool
        }

        async fn seed_user(pool: &SqlitePool, username: &str) {
            sqlx::query("INSERT INTO users (username, email, password_hash) VALUES (?, ?, 'x')")
                .bind(username)
                .bind(format!("{username}@test"))
                .execute(pool)
                .await
                .unwrap();
        }

        async fn seed_shared_event(pool: &SqlitePool, permission: &str) {
            seed_user(pool, "alice").await;
            seed_user(pool, "bob").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES ('alice', 'family', 'bob', ?)",
            )
            .bind(permission)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, ical_data, etag) \
                 VALUES ('alice', 'family', 'evt1', 'VEVENT', 'Original', \
                 '2026-08-01 10:00:00', '2026-08-01 11:00:00', 'BEGIN:VCALENDAR...', 'etag1')",
            )
            .execute(pool)
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn rejects_read_only_share() {
            let pool = test_pool().await;
            seed_shared_event(&pool, "read").await;

            let owner = resolve_event_write_owner(&pool, "bob", "evt1")
                .await
                .unwrap();
            assert!(
                owner.is_none(),
                "read-only grantee must not resolve a writable owner"
            );
        }

        #[tokio::test]
        async fn allows_read_write_share() {
            let pool = test_pool().await;
            seed_shared_event(&pool, "read-write").await;

            let owner = resolve_event_write_owner(&pool, "bob", "evt1")
                .await
                .unwrap();
            assert_eq!(owner, Some("alice".to_string()));
        }

        #[tokio::test]
        async fn returns_none_for_unrelated_uid() {
            let pool = test_pool().await;
            seed_shared_event(&pool, "read-write").await;

            let owner = resolve_event_write_owner(&pool, "bob", "no-such-uid")
                .await
                .unwrap();
            assert!(owner.is_none());
        }
    }

    // ── calendar_grantees (backlog #59) ─────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod calendar_grantees_tests {
        use super::super::calendar_grantees;
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::SqlitePool;
        use std::str::FromStr;

        async fn test_pool() -> SqlitePool {
            let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
            pool
        }

        async fn seed_user(pool: &SqlitePool, username: &str) {
            sqlx::query("INSERT INTO users (username, email, password_hash) VALUES (?, ?, 'x')")
                .bind(username)
                .bind(format!("{username}@test"))
                .execute(pool)
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn returns_every_grantee_regardless_of_permission() {
            let pool = test_pool().await;
            seed_user(&pool, "alice").await;
            seed_user(&pool, "bob").await;
            seed_user(&pool, "carol").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES ('alice', 'family', 'bob', 'read'), ('alice', 'family', 'carol', 'read-write')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let mut grantees = calendar_grantees(&pool, "alice", "family").await;
            grantees.sort();
            assert_eq!(grantees, vec!["bob".to_string(), "carol".to_string()]);
        }

        #[tokio::test]
        async fn returns_empty_for_an_unshared_calendar() {
            let pool = test_pool().await;
            seed_user(&pool, "alice").await;
            sqlx::query(
                "INSERT INTO calendars (username, name, display_name) VALUES ('alice', 'family', 'Family')",
            )
            .execute(&pool)
            .await
            .unwrap();

            let grantees = calendar_grantees(&pool, "alice", "family").await;
            assert!(grantees.is_empty());
        }
    }

    // ── mark_done_at_line ─────────────────────────────────────────────────────

    #[cfg(feature = "ssr")]
    mod mark_done_at_line_tests {
        use super::super::transform_line_at;

        fn c() -> chrono::NaiveDate {
            chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
        }

        fn mark_done(content: &str, idx: usize) -> String {
            transform_line_at(content, idx, |line| tidl_markdown::mark_tag_done(line, c()))
        }

        fn reopen_done(content: &str, idx: usize) -> String {
            transform_line_at(content, idx, |line| {
                tidl_markdown::reopen_tag(line).unwrap_or_else(|| line.to_string())
            })
        }

        #[test]
        fn lf_file_stays_lf() {
            let content = "one\n- #task/p0/x Fix it\nthree\n";
            assert_eq!(
                mark_done(content, 1),
                "one\n- #done/2026-10-02/p0/x Fix it\nthree\n"
            );
        }

        #[test]
        fn crlf_file_stays_crlf() {
            // Regression test: rejoining with a hardcoded "\n" used to
            // silently downgrade every untouched line in a CRLF file to LF.
            let content = "one\r\n- #task/p0/x Fix it\r\nthree\r\n";
            assert_eq!(
                mark_done(content, 1),
                "one\r\n- #done/2026-10-02/p0/x Fix it\r\nthree\r\n"
            );
        }

        #[test]
        fn crlf_file_without_trailing_newline() {
            let content = "one\r\n- #await/2025-01-15/x Fix it\r\nthree";
            assert_eq!(
                mark_done(content, 1),
                "one\r\n- #done/2026-10-02/2025-01-15/x Fix it\r\nthree"
            );
        }

        #[test]
        fn out_of_range_index_leaves_content_unchanged() {
            let content = "one\r\ntwo\r\n";
            assert_eq!(mark_done(content, 99), content);
        }

        #[test]
        fn already_done_line_is_unchanged() {
            let content = "one\n- #done/x\n";
            assert_eq!(mark_done(content, 1), content);
        }

        #[test]
        fn reopen_restores_the_original_tag() {
            let content = "one\n- #done/2026-10-02/p0/x Fix it\nthree\n";
            assert_eq!(reopen_done(content, 1), "one\n- #task/p0/x Fix it\nthree\n");
        }

        #[test]
        fn reopen_legacy_done_is_unchanged() {
            let content = "one\n- #done/x\n";
            assert_eq!(reopen_done(content, 1), content);
        }
    }

    // ── get_events perf diagnostics ─────────────────────────────────────────────
    //
    // Not part of the normal suite: depends on the real, local data/tidl.db
    // rather than a synthetic fixture, so it's #[ignore]d and only runs when
    // asked for explicitly. Opens the DB read-only — never writes.
    //
    //   cargo test -p tidl-web --lib --features ssr -- --ignored --nocapture bench_get_events_real_data

    #[cfg(feature = "ssr")]
    mod perf_bench {
        use super::super::{
            event_row_to_web_events, expand_rrule, range_bounds, sql_prefilter_bounds, EventRow,
            SQL_RANGE_PREFILTER,
        };
        use crate::api::extract_all_day_and_alarms;
        use crate::api::vevent_has_rrule;
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::{Row, SqlitePool};
        use std::collections::{HashMap, HashSet};
        use std::path::Path;
        use std::str::FromStr;
        use std::time::Instant;

        #[tokio::test]
        #[ignore]
        async fn bench_get_events_real_data() {
            let db_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/tidl.db");
            let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
                .unwrap()
                .read_only(true);
            let pool = SqlitePool::connect_with(opts)
                .await
                .expect("open real data/tidl.db read-only (run from a checkout with real data)");

            // Point this at a real user via TIDL_BENCH_USER when running the
            // benchmark against a populated data/tidl.db.
            let username = std::env::var("TIDL_BENCH_USER").unwrap_or_else(|_| "admin".to_string());
            let timezone = "Europe/Berlin";
            let (range_start, range_end) = ("2026-07-01".to_string(), "2026-07-31".to_string());
            let (sql_lo, sql_hi) = sql_prefilter_bounds(&range_start, &range_end);

            let t_query = Instant::now();
            let query = format!(
                "SELECT e.uid, e.summary, e.calendar,
                        CAST(e.start_date AS TEXT) AS start_date,
                        CAST(e.end_date   AS TEXT) AS end_date,
                        e.description, e.ical_data,
                        c.color AS calendar_color
                 FROM events e
                 LEFT JOIN calendars c ON c.username = e.username AND c.name = e.calendar
                 WHERE e.type = 'VEVENT' AND e.username = ?
                 {SQL_RANGE_PREFILTER}
                 ORDER BY e.start_date ASC"
            );
            let rows = sqlx::query(&query)
                .bind(&username)
                .bind(&sql_lo)
                .bind(&sql_hi)
                .fetch_all(&pool)
                .await
                .unwrap();
            let query_elapsed = t_query.elapsed();

            let t_map = Instant::now();
            let all_rows: Vec<EventRow> = rows
                .into_iter()
                .map(|r| EventRow {
                    uid: r.try_get("uid").unwrap_or_default(),
                    summary: r
                        .try_get::<Option<String>, _>("summary")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                    calendar: r
                        .try_get::<Option<String>, _>("calendar")
                        .unwrap_or_default()
                        .unwrap_or_else(|| "personal".to_string()),
                    calendar_color: r.try_get("calendar_color").unwrap_or_default(),
                    description: r.try_get("description").unwrap_or_default(),
                    priority: r.try_get("priority").unwrap_or_default(),
                    raw_start: r
                        .try_get::<Option<String>, _>("start_date")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                    raw_end: r
                        .try_get::<Option<String>, _>("end_date")
                        .unwrap_or_default()
                        .unwrap_or_default(),
                    ical: r.try_get("ical_data").unwrap_or_default(),
                    owner_username: username.clone(),
                    shared_permission: None,
                })
                .collect();
            let map_elapsed = t_map.elapsed();
            let row_count = all_rows.len();

            let mut exception_map: HashMap<String, HashSet<String>> = HashMap::new();
            for r in &all_rows {
                if let Some(pos) = r.uid.rfind("_R") {
                    let series = r.uid[..pos].to_string();
                    let rid = r.uid[pos + 2..].to_string();
                    exception_map.entry(series).or_default().insert(rid);
                }
            }
            let empty_set: HashSet<String> = HashSet::new();

            // Computed once, like get_events now does — not per row.
            let (range_start_dt, range_end_dt) = range_bounds(&range_start, &range_end, timezone);

            // Isolate expand_rrule cost on the truly-recurring subset, borrowing
            // all_rows so the consuming loop below can still move out of it.
            let t_rrule = Instant::now();
            let mut recurring_count = 0usize;
            for r in &all_rows {
                let is_exception = r.uid.contains("_R");
                if is_exception || !vevent_has_rrule(&r.ical) {
                    continue;
                }
                recurring_count += 1;
                let (all_day, alarms) = extract_all_day_and_alarms(&r.ical);
                let exceptions = exception_map.get(&r.uid).unwrap_or(&empty_set);
                expand_rrule(
                    &r.uid,
                    &r.summary,
                    &r.calendar,
                    &r.calendar_color,
                    &r.description,
                    &alarms,
                    all_day,
                    &r.raw_start,
                    &r.raw_end,
                    &r.ical,
                    exceptions,
                    timezone,
                    range_start_dt,
                    range_end_dt,
                );
            }
            let rrule_elapsed = t_rrule.elapsed();

            // Mirrors get_events exactly: consumes all_rows by value, one
            // event_row_to_web_events call per row, aggregate-timed.
            let t_loop = Instant::now();
            let mut result_count = 0usize;
            for r in all_rows {
                let series_uid = if let Some(pos) = r.uid.rfind("_R") {
                    r.uid[..pos].to_string()
                } else {
                    r.uid.clone()
                };
                let exceptions = exception_map.get(&series_uid).unwrap_or(&empty_set);
                result_count += event_row_to_web_events(
                    r.uid,
                    r.summary,
                    r.calendar,
                    r.calendar_color,
                    r.description,
                    None,
                    r.raw_start,
                    r.raw_end,
                    &r.ical,
                    exceptions,
                    timezone,
                    range_start_dt,
                    range_end_dt,
                )
                .len();
            }
            let loop_elapsed = t_loop.elapsed();

            eprintln!(
                "\n--- get_events perf breakdown (username={username}, range={range_start}..{range_end}) ---\n\
                 rows fetched:                            {row_count}\n\
                 recurring rows (RRULE in own VEVENT):    {recurring_count}\n\
                 events returned:                         {result_count}\n\
                 DB query (sqlx fetch_all):               {query_elapsed:?}\n\
                 row -> EventRow mapping:                 {map_elapsed:?}\n\
                 expand_rrule only (recurring subset):    {rrule_elapsed:?}\n\
                 event_row_to_web_events loop (all rows): {loop_elapsed:?}\n\
                 total (query + map + loop):              {:?}\n",
                query_elapsed + map_elapsed + loop_elapsed,
            );
        }
    }
}

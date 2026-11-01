//! Background job that sends reminder emails for `ACTION:EMAIL` alarms
//! (backlog #73). Spawned once at server startup.
//!
//! Every `TICK_SECS` it scans every user's `VEVENT`s for EMAIL alarms whose
//! trigger time has arrived, expanding recurring events via
//! [`event_row_to_web_events`] — the same RRULE/exception logic the Calendar
//! view uses — rather than re-implementing occurrence expansion here.
//! "Already sent" state lives in `sent_alarm_notifications` (durable, so
//! restarts/polling don't double-send).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration as StdDuration,
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::{Row, SqlitePool};
use tracing::{error, warn};

use tidl_caldav::icalendar_utils::prop_value_with_params;
use tidl_web::api::{event_row_to_web_events, Event};

use crate::alarm::ui_to_minutes;
use crate::mailer::Mailer;

/// How often the loop wakes to check for due alarms.
const TICK_SECS: u64 = 60;

/// How far into the future to expand recurring events looking for due
/// triggers. An alarm set further ahead of its occurrence than this won't
/// be detected until the occurrence enters this window on a later tick.
const LOOKAHEAD_DAYS: i64 = 60;

/// Don't fire alarms whose trigger time is older than this — bounds the
/// flood of stale reminders after extended downtime.
const STALE_CUTOFF_HOURS: i64 = 24;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_alarm_dispatch_loop(pool: SqlitePool, mailer: Arc<dyn Mailer + Send + Sync>) {
    loop {
        if let Err(e) = dispatch_due(&pool, mailer.as_ref()).await {
            error!("alarm_dispatch: tick failed: {e}");
        }
        tokio::time::sleep(StdDuration::from_secs(TICK_SECS)).await;
    }
}

async fn dispatch_due(
    pool: &SqlitePool,
    mailer: &(dyn Mailer + Send + Sync),
) -> anyhow::Result<usize> {
    dispatch_due_at(pool, mailer, Utc::now()).await
}

// ── Row loading ───────────────────────────────────────────────────────────────

struct EmailAlarmRow {
    username: String,
    uid: String,
    summary: String,
    calendar: String,
    description: Option<String>,
    raw_start: String,
    raw_end: String,
    ical: String,
    timezone: String,
    owner_email: String,
}

async fn load_email_alarm_rows(pool: &SqlitePool) -> anyhow::Result<Vec<EmailAlarmRow>> {
    let rows = sqlx::query(
        "SELECT e.username, e.uid, e.summary, e.calendar, e.description,
                CAST(e.start_date AS TEXT) AS start_date,
                CAST(e.end_date   AS TEXT) AS end_date,
                e.ical_data,
                COALESCE(up.timezone, 'UTC') AS timezone,
                u.email AS owner_email
         FROM events e
         JOIN users u ON u.username = e.username
         LEFT JOIN user_preferences up ON up.username = e.username
         WHERE e.type = 'VEVENT' AND e.ical_data LIKE '%ACTION:EMAIL%'",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| EmailAlarmRow {
            username: r.try_get("username").unwrap_or_default(),
            uid: r.try_get("uid").unwrap_or_default(),
            summary: r
                .try_get::<Option<String>, _>("summary")
                .unwrap_or_default()
                .unwrap_or_default(),
            calendar: r.try_get("calendar").unwrap_or_default(),
            description: r.try_get("description").unwrap_or_default(),
            raw_start: r
                .try_get::<Option<String>, _>("start_date")
                .unwrap_or_default()
                .unwrap_or_default(),
            raw_end: r
                .try_get::<Option<String>, _>("end_date")
                .unwrap_or_default()
                .unwrap_or_default(),
            ical: r.try_get("ical_data").unwrap_or_default(),
            timezone: r.try_get("timezone").unwrap_or_default(),
            owner_email: r.try_get("owner_email").unwrap_or_default(),
        })
        .collect())
}

// ── Core tick ─────────────────────────────────────────────────────────────────

async fn dispatch_due_at(
    pool: &SqlitePool,
    mailer: &(dyn Mailer + Send + Sync),
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let rows = load_email_alarm_rows(pool).await?;

    // (username, series_uid) -> {recurrence-id}, from exception-row uids
    // (form "{series_uid}_R{recurrence_id}"). Mirrors `get_events`'s
    // exception_map, but scoped per-user since `uid` is only unique per
    // (username, calendar), not globally.
    let mut exception_map: HashMap<(String, String), HashSet<String>> = HashMap::new();
    for r in &rows {
        if let Some(pos) = r.uid.rfind("_R") {
            let series = r.uid[..pos].to_string();
            let rid = r.uid[pos + 2..].to_string();
            exception_map
                .entry((r.username.clone(), series))
                .or_default()
                .insert(rid);
        }
    }
    let empty_set: HashSet<String> = HashSet::new();

    let range_start = now - Duration::hours(STALE_CUTOFF_HOURS);
    let range_end = now + Duration::days(LOOKAHEAD_DAYS);

    let mut sent_count = 0usize;
    for r in rows {
        let series_uid = match r.uid.rfind("_R") {
            Some(pos) => r.uid[..pos].to_string(),
            None => r.uid.clone(),
        };
        let exceptions = exception_map
            .get(&(r.username.clone(), series_uid))
            .unwrap_or(&empty_set);

        let events = event_row_to_web_events(
            r.uid,
            r.summary,
            r.calendar,
            None,
            r.description,
            None, // priority is irrelevant to alarm dispatch
            r.raw_start,
            r.raw_end,
            &r.ical,
            exceptions,
            &r.timezone,
            range_start,
            range_end,
        );

        for event in &events {
            for due in due_email_alarms(event, now, range_start) {
                match send_due_alarm(pool, mailer, &r.username, &r.owner_email, event, &due).await {
                    Ok(true) => sent_count += 1,
                    Ok(false) => {}
                    Err(e) => error!(
                        "alarm_dispatch: failed dispatching {}/{} alarm {}: {e}",
                        r.username, event.id, due.alarm_index
                    ),
                }
            }
        }
    }

    Ok(sent_count)
}

// ── Due-alarm computation (pure — unit tested without a DB) ───────────────────

struct DueAlarm {
    alarm_index: usize,
    /// Dedup key: the occurrence's compact DTSTART (or `start_epoch_ms` for
    /// non-recurring events) plus a "#<repeat_index>" suffix (0 = base
    /// firing) so REPEAT/DURATION repeats each get their own row.
    occurrence_key: String,
}

fn due_email_alarms(
    event: &Event,
    now: DateTime<Utc>,
    stale_cutoff: DateTime<Utc>,
) -> Vec<DueAlarm> {
    let base_key = event
        .occurrence_start
        .clone()
        .unwrap_or_else(|| event.start_epoch_ms.to_string());

    let mut due = Vec::new();
    for (idx, alarm) in event.alarms.iter().enumerate() {
        if alarm.action != "email" || alarm.is_raw {
            continue;
        }

        let offset_mins = ui_to_minutes(alarm.amount, &alarm.unit) as i64; // negative = before start
        let base_trigger_ms = event.start_epoch_ms + offset_mins * 60_000;

        // REPEAT/DURATION: each repeat is its own firing, `repeat_delay_m`
        // apart — same semantics as the browser scheduler (`alarm_scheduler.rs`).
        let (repeats, delay_m) = match (alarm.repeat_count, alarm.repeat_delay_m) {
            (Some(count), Some(delay_m)) => (count, delay_m as i64),
            _ => (0, 0),
        };

        for rep in 0..=repeats {
            let trigger_ms = base_trigger_ms + (rep as i64) * delay_m * 60_000;
            let Some(trigger_at) = Utc.timestamp_millis_opt(trigger_ms).single() else {
                continue;
            };
            if trigger_at > now || trigger_at < stale_cutoff {
                continue;
            }
            due.push(DueAlarm {
                alarm_index: idx,
                occurrence_key: format!("{base_key}#{rep}"),
            });
        }
    }
    due
}

// ── Sending ───────────────────────────────────────────────────────────────────

fn extract_ical_prop(raw: &str, name: &str) -> Option<String> {
    raw.lines()
        .find_map(|l| prop_value_with_params(l, name).map(str::to_string))
}

/// Returns `Ok(true)` if an email was actually sent (and recorded).
async fn send_due_alarm(
    pool: &SqlitePool,
    mailer: &(dyn Mailer + Send + Sync),
    username: &str,
    owner_email: &str,
    event: &Event,
    due: &DueAlarm,
) -> anyhow::Result<bool> {
    // Belt-and-suspenders: the UNIQUE constraint on insert is the real
    // guard, but checking first avoids composing/sending an email we'd
    // then discard.
    let alarm_index = due.alarm_index as i64;
    let already_sent: Option<i64> = sqlx::query_scalar!(
        "SELECT 1 as \"x!: i64\" FROM sent_alarm_notifications
         WHERE username = ? AND event_uid = ? AND alarm_index = ? AND occurrence_start = ?",
        username,
        event.id,
        alarm_index,
        due.occurrence_key,
    )
    .fetch_optional(pool)
    .await?;
    if already_sent.is_some() {
        return Ok(false);
    }

    let alarm = &event.alarms[due.alarm_index];
    let raw = alarm.raw_ical.as_deref().unwrap_or("");

    // RFC 5545 requires SUMMARY/DESCRIPTION on the VALARM itself for
    // ACTION:EMAIL; fall back to the event's own summary/time when a
    // UI-authored alarm doesn't carry them.
    let subject =
        extract_ical_prop(raw, "SUMMARY").unwrap_or_else(|| format!("Reminder: {}", event.summary));
    let body = extract_ical_prop(raw, "DESCRIPTION")
        .unwrap_or_else(|| format!("{}  ·  starts at {}", event.summary, event.start));

    // Prefer an explicit ATTENDEE (e.g. from an externally-authored alarm);
    // the web UI has no field for one yet, so its alarms fall back to the
    // event owner's account email.
    let (to_address, to_name) = match extract_ical_prop(raw, "ATTENDEE")
        .and_then(|a| a.strip_prefix("mailto:").map(str::to_string))
    {
        Some(addr) => (addr, String::new()),
        None => (owner_email.to_string(), username.to_string()),
    };
    if to_address.is_empty() {
        warn!(
            "alarm_dispatch: no recipient email for {username}/{} alarm {}, skipping",
            event.id, due.alarm_index
        );
        return Ok(false);
    }

    if let Err(e) = mailer
        .send(&to_address, &to_name, &subject, &body, &body)
        .await
    {
        warn!(
            "alarm_dispatch: send failed for {username}/{} alarm {}: {e}",
            event.id, due.alarm_index
        );
        return Ok(false);
    }

    sqlx::query!(
        "INSERT INTO sent_alarm_notifications (username, event_uid, alarm_index, occurrence_start)
         VALUES (?, ?, ?, ?)",
        username,
        event.id,
        alarm_index,
        due.occurrence_key,
    )
    .execute(pool)
    .await?;

    Ok(true)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use tidl_web::api::Alarm;

    fn base_event(alarms: Vec<Alarm>) -> Event {
        Event {
            id: "evt-1".to_string(),
            summary: "Standup".to_string(),
            start: "2026-08-31 10:00".to_string(),
            end: "2026-08-31 10:30".to_string(),
            description: None,
            priority: None,
            alarms,
            calendar: "personal".to_string(),
            calendar_color: None,
            all_day: false,
            rrule: None,
            occurrence_start: None,
            is_exception: false,
            start_epoch_ms: 1_756_634_400_000, // 2026-08-31T10:00:00Z
            owner_username: String::new(),
            shared_permission: None,
        }
    }

    fn email_alarm(amount: u32, unit: &str) -> Alarm {
        Alarm {
            action: "email".to_string(),
            amount,
            unit: unit.to_string(),
            repeat_count: None,
            repeat_delay_m: None,
            raw_ical: None,
            is_raw: false,
        }
    }

    #[test]
    fn due_email_alarms_fires_when_trigger_has_passed() {
        let event = base_event(vec![email_alarm(15, "minutes")]);
        let now = Utc
            .timestamp_millis_opt(1_756_634_400_000 - 14 * 60_000)
            .single()
            .unwrap(); // 1 min after trigger
        let stale_cutoff = now - Duration::hours(24);
        let due = due_email_alarms(&event, now, stale_cutoff);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].alarm_index, 0);
    }

    #[test]
    fn due_email_alarms_skips_when_trigger_is_in_the_future() {
        let event = base_event(vec![email_alarm(15, "minutes")]);
        let now = Utc
            .timestamp_millis_opt(1_756_634_400_000 - 20 * 60_000)
            .single()
            .unwrap(); // 5 min before trigger
        let stale_cutoff = now - Duration::hours(24);
        assert!(due_email_alarms(&event, now, stale_cutoff).is_empty());
    }

    #[test]
    fn due_email_alarms_skips_stale_triggers() {
        let event = base_event(vec![email_alarm(15, "minutes")]);
        let now = Utc
            .timestamp_millis_opt(1_756_634_400_000 + 48 * 60 * 60_000)
            .single()
            .unwrap(); // 2 days later
        let stale_cutoff = now - Duration::hours(24);
        assert!(due_email_alarms(&event, now, stale_cutoff).is_empty());
    }

    #[test]
    fn due_email_alarms_ignores_non_email_and_raw_actions() {
        let mut display = email_alarm(15, "minutes");
        display.action = "display".to_string();
        let mut raw = email_alarm(15, "minutes");
        raw.is_raw = true;
        let event = base_event(vec![display, raw]);
        let now = Utc
            .timestamp_millis_opt(1_756_634_400_000)
            .single()
            .unwrap();
        let stale_cutoff = now - Duration::hours(24);
        assert!(due_email_alarms(&event, now, stale_cutoff).is_empty());
    }

    #[test]
    fn due_email_alarms_expands_repeats_as_separate_firings() {
        let mut alarm = email_alarm(15, "minutes");
        alarm.repeat_count = Some(2);
        alarm.repeat_delay_m = Some(5);
        let event = base_event(vec![alarm]);
        // 11 minutes after the base trigger: base + 2 repeats (at +0m, +5m, +10m) all due.
        let now = Utc
            .timestamp_millis_opt(1_756_634_400_000 - 15 * 60_000 + 11 * 60_000)
            .single()
            .unwrap();
        let stale_cutoff = now - Duration::hours(24);
        let due = due_email_alarms(&event, now, stale_cutoff);
        assert_eq!(due.len(), 3);
        let mut keys: Vec<&str> = due.iter().map(|d| d.occurrence_key.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["1756634400000#0", "1756634400000#1", "1756634400000#2"]
        );
    }

    #[test]
    fn extract_ical_prop_reads_attendee_mailto() {
        let raw = "ACTION:EMAIL\nTRIGGER:-PT15M\nATTENDEE:mailto:someone@example.com\n";
        let attendee = extract_ical_prop(raw, "ATTENDEE").unwrap();
        assert_eq!(
            attendee.strip_prefix("mailto:").unwrap(),
            "someone@example.com"
        );
    }

    // ── Recording mailer for integration-style assertions ──────────────────

    struct RecordingMailer {
        sent: Mutex<Vec<(String, String)>>, // (to_address, subject)
    }

    impl RecordingMailer {
        fn new() -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Mailer for RecordingMailer {
        async fn send(
            &self,
            to_address: &str,
            _to_name: &str,
            subject: &str,
            _body_text: &str,
            _body_html: &str,
        ) -> anyhow::Result<()> {
            self.sent
                .lock()
                .unwrap()
                .push((to_address.to_string(), subject.to_string()));
            Ok(())
        }
    }

    async fn seed_user(pool: &SqlitePool, username: &str, email: &str) {
        sqlx::query("INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES (?, ?, ?, 'x', 0)")
            .bind(username)
            .bind(email)
            .bind(username)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_email_alarm_event(
        pool: &SqlitePool,
        username: &str,
        uid: &str,
        start: DateTime<Utc>,
    ) {
        let start_sql = start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_sql = (start + Duration::minutes(30))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let ical = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Standup\r\nDTSTART:{dt}\r\nDTEND:{dt}\r\n\
             BEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT15M\r\nSUMMARY:Reminder\r\nDESCRIPTION:Standup soon\r\nEND:VALARM\r\n\
             END:VEVENT\r\nEND:VCALENDAR\r\n",
            dt = start.format("%Y%m%dT%H%M%SZ"),
        );
        sqlx::query(
            "INSERT INTO events (username, calendar, uid, type, summary, start_date, end_date, etag, ical_data)
             VALUES (?, 'personal', ?, 'VEVENT', 'Standup', ?, ?, 'etag1', ?)",
        )
        .bind(username)
        .bind(uid)
        .bind(&start_sql)
        .bind(&end_sql)
        .bind(&ical)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_due_at_sends_exactly_once_across_two_ticks(pool: SqlitePool) {
        seed_user(&pool, "alice", "alice@example.com").await;
        let start = Utc::now() - Duration::minutes(10); // trigger (start - 15m) is 5 min in the past
        seed_email_alarm_event(&pool, "alice", "evt-1", start).await;

        let mailer = RecordingMailer::new();
        let now = Utc::now();

        let first = dispatch_due_at(&pool, &mailer, now).await.unwrap();
        assert_eq!(first, 1);

        let second = dispatch_due_at(&pool, &mailer, now).await.unwrap();
        assert_eq!(
            second, 0,
            "second tick must not re-send an already-recorded alarm"
        );

        let sent = mailer.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "alice@example.com");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_due_at_skips_events_with_no_due_alarm(pool: SqlitePool) {
        seed_user(&pool, "alice", "alice@example.com").await;
        let start = Utc::now() + Duration::days(10); // trigger is far in the future
        seed_email_alarm_event(&pool, "alice", "evt-1", start).await;

        let mailer = RecordingMailer::new();
        let count = dispatch_due_at(&pool, &mailer, Utc::now()).await.unwrap();
        assert_eq!(count, 0);
        assert!(mailer.sent.lock().unwrap().is_empty());
    }
}

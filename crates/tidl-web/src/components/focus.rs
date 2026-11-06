//! Focus view — urgency-ranked unified list (backlog #21)

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, Utc};
use leptos::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
use crate::api::{complete_markdown_task, load_todo_chip, reopen_markdown_task};
use crate::api::{
    complete_todo_item, create_todo_item, delete_todo_item, get_awaits, get_events,
    get_markdown_file_content, get_markdown_rendered, get_tasks, list_markdown_sources,
    list_note_collections, list_notes, open_in_editor, remove_markdown_task, suppress_focus_item,
    trigger_markdown_sync, unsuppress_focus_item, update_todo_chip, update_todo_item, AwaitItem,
    ContactDto, CurvePoint, Event, NoteLabelInfo, PriorityCurves, Task, TodoChip, TodoOrigin,
};
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
use crate::api::{resolve_contact_mention, resolve_wikilink};
use crate::app::{use_events_refresh, use_sidebar_refresh, AllCalendarsCtx, PriorityCurvesCtx};
use crate::components::calendar::date_helpers::{add_days, input_dt_from_server, today_ymd};
use crate::components::calendar::EventFormDialog;
use crate::components::editor::MarkdownFileEditor;
use crate::components::filter_context::{
    HiddenMdSourcesCtx, HiddenNoteCollectionsCtx, HiddenNoteLabelsCtx,
};
use crate::components::focus_sidebar::FocusSidebar;
use crate::components::mention_popover::MentionPopover;
use crate::components::notes::{NoteEditorModal, PullOutNoteModal};
use crate::components::priority_slider::PrioritySlider;
use crate::components::settings::OpenSettingsCtx;
use crate::components::view_header::SectionHeader;
use crate::local_editor::{
    build_editor_uri, editor_label, normalize_repo_key, resolve_target, save_prefs, LocalEditorCtx,
    OpenTarget,
};
use crate::types::MarkdownUid;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

// ── Date utilities ─────────────────────────────────────────────────────────────

fn parse_date_str(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok()
}

/// Percent-encode a value for use inside a URL query string. Event uids are
/// arbitrary CalDAV strings (often containing `@`, `:`, or spaces), so they
/// can't be dropped into `?highlight=` verbatim.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn encode_query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Returns (item_date - today).num_days().  Returns i64::MAX when unparseable.
fn day_diff(date_str: &str, today: NaiveDate) -> i64 {
    parse_date_str(date_str)
        .map(|d| (d - today).num_days())
        .unwrap_or(i64::MAX)
}

/// Returns the current local time as "YYYY-MM-DD HH:MM".
///
/// In WASM, reads the browser's clock so the timezone matches the user's
/// locale — this is critical because event start/end strings are also in
/// local time.  In SSR, falls back to the server's local time; the value
/// is only used for filtering inside `LocalResource` (client-side only),
/// so the server path is never exercised for actual filtering.
fn local_now_str() -> String {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new_0();
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            d.get_full_year(),
            d.get_month() + 1, // get_month() is 0-indexed
            d.get_date(),
            d.get_hours(),
            d.get_minutes(),
        );
    }
    #[cfg(not(feature = "hydrate"))]
    {
        chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
    }
}

/// The current instant as UTC epoch milliseconds — the same basis
/// `Event::start_epoch_ms` uses (see its doc comment), so subtracting the
/// two gives a timezone-agnostic "hours until start" independent of the
/// user's locale. Companion to `local_now_str`, which is for display/
/// same-day string comparisons instead.
pub(crate) fn local_now_epoch_ms() -> i64 {
    #[cfg(feature = "hydrate")]
    {
        return js_sys::Date::now() as i64;
    }
    #[cfg(not(feature = "hydrate"))]
    {
        Utc::now().timestamp_millis()
    }
}

// ── Urgency scoring ────────────────────────────────────────────────────────────

/// Parses the `HH:MM` portion of a `"YYYY-MM-DD HH:MM"` string into minutes
/// since midnight. Returns `None` if the string is too short to contain a
/// time component (e.g. an all-day `"YYYY-MM-DD"` date).
fn minutes_since_midnight(datetime_str: &str) -> Option<i64> {
    if datetime_str.len() < 16 {
        return None;
    }
    let hh: i64 = datetime_str[11..13].parse().ok()?;
    let mm: i64 = datetime_str[14..16].parse().ok()?;
    Some(hh * 60 + mm)
}

/// Linear interpolation between the two points of `points` bracketing `x`;
/// flat extrapolation past the first/last point; result clamped to
/// `[0, 100]`. `points` must be sorted by `x` and have at least 2 entries —
/// every caller gets its curve from a `PriorityCurves`, whose fields are
/// only ever built by `PriorityCurves::default()` or `save_priority_curve`
/// (which validates both invariants), so this never sees fewer than 2
/// points in practice; falls back to the last point rather than panicking
/// if that invariant is ever violated.
pub(crate) fn eval_curve(points: &[CurvePoint], x: f64) -> u8 {
    let first = &points[0];
    if x <= first.x {
        return first.y.round().clamp(0.0, 100.0) as u8;
    }
    let last = &points[points.len() - 1];
    if x >= last.x {
        return last.y.round().clamp(0.0, 100.0) as u8;
    }
    for w in points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if x >= a.x && x <= b.x {
            let t = if b.x > a.x {
                (x - a.x) / (b.x - a.x)
            } else {
                0.0
            };
            let y = a.y + t * (b.y - a.y);
            return y.round().clamp(0.0, 100.0) as u8;
        }
    }
    last.y.round().clamp(0.0, 100.0) as u8
}

/// `now_local`/`now_epoch_ms` are passed in rather than read from the wall
/// clock here, so this function remains a pure, deterministic mapping of
/// its arguments — the same care taken elsewhere (see #21) to avoid
/// SSR/WASM hydration mismatches. The caller re-derives both every
/// reactive run (see `FocusView`'s 60s tick), so proximity naturally
/// updates over time without this function ever consulting the clock
/// itself.
///
/// Scores via the user's `calendar_time_left` curve (backlog #68), keyed on
/// a single continuous "hours until start" value — replacing the old
/// day-bucket + hour-of-day-bucket + intraday-proximity-override structure
/// with one curve lookup. An event whose calendar day has already passed
/// naturally lands on the curve's flat past tail (a very negative
/// hours-until-start) with no special-casing needed. The one exception is
/// an event currently in progress (`now` between `start` and `end`, using
/// the same same-day local-time string comparison the old implementation
/// used — `Event` has no `end_epoch_ms` to do this via pure epoch math):
/// its curve input is floored at `0.0` ("needs attention right now"),
/// covering both "about to start" and "already running" the same way.
///
/// An event with an explicit priority (backlog #156) has its time score
/// capped by `calendar_event_priority` — a ceiling, never a boost. So a
/// low-priority event can still outrank genuinely low tasks but can never
/// reach URGENT while a high-priority non-calendar item is pending.
fn event_in_progress(event: &Event, now_local: &str) -> bool {
    now_local.len() >= 16
        && event.start.len() >= 10
        && now_local[..10] == event.start[..10]
        && minutes_since_midnight(&event.start)
            .zip(minutes_since_midnight(now_local))
            .zip(minutes_since_midnight(&event.end))
            .is_some_and(|((start_min, now_min), end_min)| {
                start_min <= now_min && now_min <= end_min
            })
}

/// One contributing input to a Focus item's urgency score (backlog #158).
///
/// `label` is already human-readable — a raw priority, or the due/start
/// distance in everyday units — never a curve name or threshold, and `score`
/// is the value that curve lookup produced for it. `used` marks the factor(s)
/// that actually determined the final score (`max` for tasks, `min` with a
/// priority cap for events).
#[derive(Clone, PartialEq)]
pub(crate) struct UrgencyFactor {
    pub label: String,
    pub score: u8,
    pub used: bool,
}

/// The inputs behind a Focus item's urgency score, shown in the optional
/// hover/tap explanation attached to the urgency cluster (backlog #158).
#[derive(Clone, PartialEq)]
pub(crate) struct UrgencyExplanation {
    pub score: u8,
    pub factors: Vec<UrgencyFactor>,
    /// Short combining rule, e.g. "Higher of the two inputs".
    pub note: Option<&'static str>,
}

/// Human-readable distance until an event starts (or since it started),
/// derived from the same epoch-ms difference the curve is evaluated on.
fn event_time_label(hours: f64) -> String {
    let minutes = (hours * 60.0).round() as i64;
    if minutes >= 0 {
        if minutes < 60 {
            format!("Starts in {minutes} min")
        } else if minutes < 36 * 60 {
            format!("Starts in {} h", (minutes as f64 / 60.0).round() as i64)
        } else {
            let days = (minutes as f64 / 1440.0).round() as i64;
            if days == 1 {
                "Starts tomorrow".to_string()
            } else {
                format!("Starts in {days} days")
            }
        }
    } else {
        let ago = -minutes;
        if ago < 60 {
            format!("Started {ago} min ago")
        } else if ago < 36 * 60 {
            format!("Started {} h ago", (ago as f64 / 60.0).round() as i64)
        } else {
            let days = (ago as f64 / 1440.0).round() as i64;
            format!("Started {days} days ago")
        }
    }
}

/// Human-readable distance until a due date (or how long it is overdue).
fn due_time_label(days: i64) -> String {
    if days == i64::MAX {
        // `day_diff`'s marker for an unparseable date string; never present
        // in practice (dates are validated on write), but don't overflow.
        return "Due date unknown".to_string();
    }
    match days {
        0 => "Due today".to_string(),
        1 => "Due tomorrow".to_string(),
        d if d > 1 => format!("Due in {d} days"),
        -1 => "Overdue by 1 day".to_string(),
        d => format!("Overdue by {} days", -d),
    }
}

fn event_urgency_explanation(
    event: &Event,
    now_local: &str,
    now_epoch_ms: i64,
    curves: &PriorityCurves,
) -> UrgencyExplanation {
    let in_progress = event_in_progress(event, now_local);
    let x = if in_progress {
        0.0
    } else {
        (event.start_epoch_ms - now_epoch_ms) as f64 / 3_600_000.0
    };
    let time_score = eval_curve(&curves.calendar_time_left, x);
    let mut factors = vec![UrgencyFactor {
        label: if in_progress {
            "In progress (0h floor)".to_string()
        } else {
            event_time_label(x)
        },
        score: time_score,
        used: true,
    }];

    let (score, note) = match event.priority {
        Some(priority) => {
            let cap = eval_curve(&curves.calendar_event_priority, priority as f64);
            let capped = cap < time_score;
            factors[0].used = !capped;
            factors.push(UrgencyFactor {
                label: format!("Priority cap {priority}"),
                score: cap,
                used: capped,
            });
            let note = if capped {
                "Capped by event priority"
            } else if cap == time_score {
                "Priority cap ties with time"
            } else {
                "Priority cap not binding"
            };
            (time_score.min(cap), Some(note))
        }
        None => (time_score, None),
    };

    UrgencyExplanation {
        score,
        factors,
        note,
    }
}

/// Test-only score view of `event_urgency_explanation` — the tests below
/// assert the score math, while the card consumes the full explanation.
#[cfg(test)]
fn event_urgency(event: &Event, now_local: &str, now_epoch_ms: i64, curves: &PriorityCurves) -> u8 {
    event_urgency_explanation(event, now_local, now_epoch_ms, curves).score
}

/// Urgency score for a "source priority" value on the curve's 1-100 domain
/// (backlog #68) — the canonical task-priority scale shared by notes,
/// markdown, and CalDAV tasks alike (backlog #70 / 2026-07-30
/// investigation). `curves` is the single source of truth both
/// `types::note_priority_urgency_label`/`_color` (a note's own priority) and
/// `task_urgency` below (`task.priority`, clamped to 1-100) key off, so a
/// note's slider preview always agrees with what Focus later shows for the
/// same note.
pub(crate) fn task_priority_score(source_priority: u32, curves: &PriorityCurves) -> u8 {
    eval_curve(&curves.source_priority, source_priority as f64)
}

/// TASK urgency: the more urgent of its priority tier and — when it has one
/// — its due date's day-proximity via `pending_urgency`. A TASK's priority
/// and due date are independent (backlog #49: TASK may have "priority
/// and/or due date", unlike AWAIT), so a dated TASK with no/low priority
/// (e.g. the `#task/<date>/<slug>` markdown form) must still rank by how
/// soon it's due rather than always falling back to the low-priority tier.
pub(crate) fn task_urgency_explanation(
    task: &Task,
    today: NaiveDate,
    curves: &PriorityCurves,
) -> UrgencyExplanation {
    // `task.priority` is the canonical 1-100 scale for every task source
    // (notes, markdown, CalDAV) — see backlog #70 / 2026-07-30
    // investigation. `note_priority` (the note's own priority, fetched
    // directly rather than via its mirrored VTODO) is kept as a defensive
    // fallback rather than relied on for correctness, since the two now
    // agree by construction.
    let source_priority = task.note_priority.unwrap_or(task.priority).clamp(1, 100);
    let priority_score = task_priority_score(source_priority, curves);
    let mut factors = vec![UrgencyFactor {
        label: format!("Priority {source_priority}"),
        score: priority_score,
        used: false,
    }];

    let (score, note) = match &task.due {
        Some(due) => {
            let due_days = day_diff(due, today);
            factors.push(UrgencyFactor {
                label: due_time_label(due_days),
                score: pending_urgency(due, today, curves),
                used: false,
            });
            let score = priority_score.max(factors[1].score);
            for factor in &mut factors {
                factor.used = factor.score == score;
            }
            (score, Some("Higher of the two inputs"))
        }
        None => {
            factors[0].used = true;
            (priority_score, Some("No due date"))
        }
    };

    UrgencyExplanation {
        score,
        factors,
        note,
    }
}

/// Test-only score view of `task_urgency_explanation` — see `event_urgency`.
#[cfg(test)]
fn task_urgency(task: &Task, today: NaiveDate, curves: &PriorityCurves) -> u8 {
    task_urgency_explanation(task, today, curves).score
}

/// Due-date proximity score via the user's `due_date_time_left` curve
/// (backlog #68).
pub(crate) fn pending_urgency(due: &str, today: NaiveDate, curves: &PriorityCurves) -> u8 {
    let d = day_diff(due, today);
    eval_curve(&curves.due_date_time_left, d as f64)
}

/// AWAIT urgency: due-date proximity only — awaits have no priority input
/// (backlog #49), so the explanation has exactly one factor.
pub(crate) fn await_urgency_explanation(
    date: &str,
    today: NaiveDate,
    curves: &PriorityCurves,
) -> UrgencyExplanation {
    let score = pending_urgency(date, today, curves);
    UrgencyExplanation {
        score,
        factors: vec![UrgencyFactor {
            label: due_time_label(day_diff(date, today)),
            score,
            used: true,
        }],
        note: None,
    }
}

pub(crate) fn urgency_label(score: u8) -> &'static str {
    if score >= 85 {
        "URGENT"
    } else if score >= 65 {
        "SOON"
    } else if score >= 40 {
        "UPCOMING"
    } else {
        "LATER"
    }
}

/// CSS `width` value (0–40px) for the urgency-track fill bar. Shared by
/// Focus's own card and the Tasks/Await cards, which reuse the identical
/// `fsc-urgency` cluster markup so the bar means the same thing everywhere.
pub(crate) fn urgency_bar_fill_px(score: u8) -> String {
    format!("{}px", (score as u32 * 40 / 100).min(40))
}

pub(crate) fn urgency_color(score: u8) -> &'static str {
    if score >= 85 {
        "#d93025"
    } else if score >= 65 {
        "#f09300"
    } else if score >= 40 {
        "#1a73e8"
    } else {
        "#bbb"
    }
}

// ── Item model ─────────────────────────────────────────────────────────────────

/// Extract the markdown source ID from a markdown todo UID.
///
/// UID format: `"md:{source_id}:{rel_path}:{line}:{ordinal}"`.
/// Returns the `source_id` portion as a `&str`, or `""` on malformed input.
pub(super) fn md_source_id(uid: &str) -> &str {
    MarkdownUid::parse(uid).map(|u| u.source_id).unwrap_or("")
}

/// Extract the rel_path from a markdown todo UID.
///
/// UID format: `"md:{source_id}:{rel_path}:{line}:{ordinal}"`.
/// Returns the `rel_path` portion, or `""` on malformed input.
fn md_rel_path(uid: &str) -> &str {
    MarkdownUid::parse(uid).map(|u| u.rel_path).unwrap_or("")
}

/// Parse the source_id from a markdown todo UID as i64.
fn md_source_id_i64(uid: &str) -> Option<i64> {
    MarkdownUid::parse(uid).and_then(|u| u.source_id_i64())
}

/// Wait for a markdown source sync to finish (observed through `last_scan`
/// changing). Returns the source's `last_error` when the sync ran but failed,
/// `Ok(None)` on success, and `Err` when it did not complete in time.
#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "hydrate")),
    allow(dead_code)
)]
async fn wait_for_source_sync(
    source_id: i64,
    old_last_scan: Option<String>,
    attempts: u8,
) -> Result<Option<String>, String> {
    for _ in 0..attempts {
        crate::util::sleep_ms(500).await;
        if let Ok(list) = list_markdown_sources().await {
            if let Some(src) = list.iter().find(|s| s.id == source_id) {
                if src.last_scan != old_last_scan {
                    return Ok(src.last_error.clone());
                }
            }
        }
    }
    Err("Sync did not complete — check the source status in Settings.".to_string())
}

/// Trigger a markdown source sync and wait for its result.
#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "hydrate")),
    allow(dead_code)
)]
async fn sync_source_now(
    source_id: i64,
    old_last_scan: Option<String>,
    attempts: u8,
) -> Result<Option<String>, String> {
    trigger_markdown_sync(source_id)
        .await
        .map_err(|e| format_server_fn_error(&e.to_string()))?;
    wait_for_source_sync(source_id, old_last_scan, attempts).await
}

/// The 0-based line-local ordinal of the tag a markdown todo UID points at —
/// which tag on its line, so the preview pulses the right chip.
fn md_ordinal(uid: &str) -> u32 {
    MarkdownUid::parse(uid)
        .map(|u| u.ordinal_u32())
        .unwrap_or(0)
}

// ── Suppression key helpers ─────────────────────────────────────────────────────

/// The suppression key for "suppress the whole series" — just the event's
/// own id (the master UID, shared by every plain-recurring occurrence).
fn series_key(uid: &str) -> String {
    uid.to_string()
}

/// The suppression key for "suppress just this occurrence" of a recurring
/// event — every plain-recurring occurrence otherwise shares one id, so the
/// occurrence date has to be folded in to disambiguate.
fn occurrence_key(uid: &str, occurrence_start: &str) -> String {
    format!("{uid}|{occurrence_start}")
}

/// Optimistically mark `key` suppressed, then persist it; on failure, revert.
fn suppress_key(suppressed: RwSignal<HashMap<String, bool>>, key: String) {
    suppressed.update(|m| {
        m.insert(key.clone(), true);
    });
    leptos::task::spawn_local(async move {
        if suppress_focus_item(key.clone()).await.is_err() {
            suppressed.update(|m| {
                m.remove(&key);
            });
        }
    });
}

/// Optimistically clear `key`'s suppression, then persist it; on failure, revert.
fn unsuppress_key(suppressed: RwSignal<HashMap<String, bool>>, key: String) {
    suppressed.update(|m| {
        m.remove(&key);
    });
    leptos::task::spawn_local(async move {
        if unsuppress_focus_item(key.clone()).await.is_err() {
            suppressed.update(|m| {
                m.insert(key, true);
            });
        }
    });
}

#[derive(Clone, PartialEq)]
enum ItemKind {
    Event(Event),
    Task(Task),
    Await(AwaitItem),
}

/// Content-type filter for the Focus list — the `All/Calendar/Tasks/Await`
/// pill. Replaces the three separate Focus/Tasks/Await screens (backlog #115):
/// each mode shows exactly the corresponding `ItemKind`, with `All` showing
/// everything ranked together as before.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemFilter {
    All,
    Calendar,
    Tasks,
    Await,
}

impl ItemFilter {
    fn includes(self, kind: &ItemKind) -> bool {
        match self {
            ItemFilter::All => true,
            ItemFilter::Calendar => matches!(kind, ItemKind::Event(_)),
            ItemFilter::Tasks => matches!(kind, ItemKind::Task(_)),
            ItemFilter::Await => matches!(kind, ItemKind::Await(_)),
        }
    }

    /// Section-header noun phrase for this filter (composed into the one-line
    /// summary by `SectionHeader`).
    fn section_label(self) -> &'static str {
        match self {
            ItemFilter::All => "All items",
            ItemFilter::Calendar => "Calendar items",
            ItemFilter::Tasks => "Task items",
            ItemFilter::Await => "Await items",
        }
    }

    /// Empty-state message shown when the filter matches nothing.
    fn empty_message(self) -> &'static str {
        match self {
            ItemFilter::All => "Nothing in this horizon — adjust the selector above.",
            ItemFilter::Calendar => "No calendar events in this horizon.",
            ItemFilter::Tasks => "No tasks in this horizon.",
            ItemFilter::Await => "No await items in this horizon.",
        }
    }
}

/// Whether an ad-hoc item is priority-driven (task) or date-driven (await) —
/// the same TASK/AWAIT split the Notes editor uses (backlog #49).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TodoKind {
    Task,
    Await,
}

impl TodoKind {
    fn as_str(self) -> &'static str {
        match self {
            TodoKind::Task => "task",
            TodoKind::Await => "await",
        }
    }
}

/// A CalDAV-origin task/await opened in the edit dialog (backlog #118).
#[derive(Clone, PartialEq)]
pub struct AdHocEdit {
    pub uid: String,
    pub kind: TodoKind,
    pub summary: String,
    /// Task priority (1-100). Ignored for awaits.
    pub priority: u32,
    /// Await due date (`YYYY-MM-DD`). Ignored for tasks.
    pub date: String,
    /// Calendar slug the item currently lives in, so the picker prefills and
    /// the item can be moved.
    pub calendar: String,
    /// Whether the item is already completed — hides the modal's "Mark as
    /// done" action (backlog #132).
    pub completed: bool,
}

/// State of the ad-hoc task/await dialog: creating a new item, editing an
/// existing ad-hoc (CalDAV) one, or editing a source-backed chip (backlog
/// #121).
#[derive(Clone, PartialEq)]
pub enum TodoModalState {
    Create(TodoKind),
    Edit(AdHocEdit),
    EditChip(TodoChip),
}

/// "Go to source" target for a note-origin item (backlog #134): the note, plus
/// — when the item's uid identifies a specific inline token — the 1-based line
/// and 0-based ordinal of that token, so the Notes tab can scroll to and flash
/// the exact chip rather than only the note card. `token: None` means the
/// location is unknown; the card-level highlight (backlog #63) is used instead.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NoteTarget {
    pub note_id: i64,
    pub token: Option<(u32, u32)>,
}

/// The `(line, ordinal)` half of a `note:{note_id}:{line}:{ordinal}` uid, for
/// the note go-to-source chip flash (backlog #134). `None` when the uid isn't
/// that shape, which makes the jump fall back to the whole-card highlight.
pub(crate) fn note_token_locator(uid: &str) -> Option<(u32, u32)> {
    let mut parts = uid.strip_prefix("note:")?.split(':');
    let _note_id = parts.next()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let ordinal = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    Some((line, ordinal))
}

#[derive(Clone)]
struct RankedItem {
    id: String,
    score: u8,
    /// The inputs behind `score`, for the urgency hover/tap explanation
    /// (backlog #158).
    why: UrgencyExplanation,
    kind: ItemKind,
    stripe_color: String,
    /// Precomputed here (rather than in `FocusScoredCard`) because it needs
    /// `src_names`, which is only in scope alongside `cal_colors`/`src_colors`
    /// where the item list is built.
    source_label: String,
}

fn item_is_done(kind: &ItemKind, event_done: &HashSet<String>) -> bool {
    match kind {
        ItemKind::Event(e) => event_done.contains(&e.id),
        ItemKind::Task(t) => t.completed,
        ItemKind::Await(a) => a.completed,
    }
}

fn item_summary(kind: &ItemKind) -> &str {
    match kind {
        ItemKind::Event(e) => &e.summary,
        ItemKind::Task(t) => &t.summary,
        ItemKind::Await(a) => &a.summary,
    }
}

/// Resolve the stripe color for a task (caldav or markdown) from the source
/// color maps.  Falls back to grey when the source is not found.
pub(crate) fn task_stripe_color(
    t: &Task,
    cal_colors: &HashMap<String, String>,
    src_colors: &HashMap<String, String>,
) -> String {
    if t.source == "markdown" {
        let src_id = md_source_id(&t.id);
        src_colors
            .get(src_id)
            .cloned()
            .unwrap_or_else(|| "#cfd2d6".to_string())
    } else if t.source == "note" {
        t.collection_color
            .clone()
            .unwrap_or_else(|| "#cfd2d6".to_string())
    } else {
        let cal = if t.calendar.is_empty() {
            "personal"
        } else {
            &t.calendar
        };
        cal_colors
            .get(cal)
            .cloned()
            .unwrap_or_else(|| "#cfd2d6".to_string())
    }
}

/// Inline `style` value for a source pill, tinting its background and text
/// with that source's actual configured color — calendar, markdown source,
/// or note collection alike. The `.pill` CSS classes only give each *kind*
/// a fixed fallback tone, which is why the same source used to render in
/// different colors across Focus/Tasks/Await and why a task on a specific
/// calendar never showed that calendar's own color — this is always called
/// with the item's real per-source color instead (see `stripe_color_for`),
/// so the pill and the card's left stripe always agree.
pub(crate) fn source_pill_style(color: &str) -> String {
    format!(
        "background: color-mix(in srgb, {c} 20%, transparent); color: {c};",
        c = color
    )
}

/// Resolve the stripe color for an await item: note-collection color if
/// note-origin, else the markdown source color, else grey.
pub(crate) fn await_stripe_color(a: &AwaitItem, src_colors: &HashMap<String, String>) -> String {
    if let Some(color) = &a.collection_color {
        return color.clone();
    }
    src_colors
        .get(md_source_id(&a.id))
        .cloned()
        .unwrap_or_else(|| "#cfd2d6".to_string())
}

/// Source pill label for a task — calendar name, note collection name, or
/// (for markdown) `"{source name} · {filename}"`. Mirrors `task_stripe_color`
/// so the Tasks view's pill label and color always agree with Focus's.
pub(crate) fn task_source_label(t: &Task, src_names: &HashMap<String, String>) -> String {
    if t.source == "markdown" {
        md_source_label(&t.id, t.source_file.as_ref(), src_names)
    } else if t.source == "note" {
        t.collection_name
            .clone()
            .unwrap_or_else(|| "Notes".to_string())
    } else if t.calendar.is_empty() {
        "Tasks".to_string()
    } else {
        t.calendar.clone()
    }
}

/// Source pill label for an await item — note collection name if
/// note-origin, else `"{source name} · {filename}"`. Mirrors
/// `await_stripe_color`.
pub(crate) fn await_source_label(a: &AwaitItem, src_names: &HashMap<String, String>) -> String {
    if let Some(name) = &a.collection_name {
        name.clone()
    } else {
        md_source_label(&a.id, a.source_file.as_ref(), src_names)
    }
}

/// CSS modifier class for the due-date pill of a pending item.
/// Returns `None` when the due date is more than a week away (neutral).
fn pending_due_class(due: &str, today: NaiveDate) -> Option<&'static str> {
    let d = day_diff(due, today);
    if d < 0 {
        Some("fsc-when-pill--overdue")
    } else if d == 0 {
        Some("fsc-when-pill--today")
    } else if d < 7 {
        Some("fsc-when-pill--soon")
    } else {
        None
    }
}

/// Left stripe color.  For events the value is the calendar hex color; for
/// todos and pending items it is the calendar/source color.
fn stripe_color_for(
    kind: &ItemKind,
    cal_colors: &HashMap<String, String>,
    src_colors: &HashMap<String, String>,
) -> String {
    match kind {
        ItemKind::Event(e) => e
            .calendar_color
            .clone()
            .unwrap_or_else(|| "#3b82f6".to_string()),
        ItemKind::Task(t) => task_stripe_color(t, cal_colors, src_colors),
        ItemKind::Await(a) => await_stripe_color(a, src_colors),
    }
}

/// Basename of a markdown-sourced item's file, e.g. `"path/to/file.md"` → `"file.md"`.
fn md_file_basename(source_file: Option<&String>) -> Option<String> {
    source_file.map(|f| f.rsplit('/').next().unwrap_or(f).to_string())
}

/// Source pill label for a markdown-origin item: `"{source name} · {filename}"`,
/// falling back gracefully if either half is unknown. `src_names` maps a
/// markdown source's id (as a string, matching `md_source_id`) to its
/// configured display name — built alongside `src_colors` wherever this is
/// called.
fn md_source_label(
    uid: &str,
    source_file: Option<&String>,
    src_names: &HashMap<String, String>,
) -> String {
    let name = src_names.get(md_source_id(uid)).cloned();
    let file = md_file_basename(source_file);
    match (name, file) {
        (Some(n), Some(f)) => format!("{n} · {f}"),
        (Some(n), None) => n,
        (None, Some(f)) => f,
        (None, None) => "Markdown".to_string(),
    }
}

/// Source pill label for the meta row — the item's calendar, markdown
/// source (as `"{name} · {filename}"`, dropping the raw topic-slug pill
/// that used to stand in for it — see buglist item 5), or note collection.
fn item_source_label(kind: &ItemKind, src_names: &HashMap<String, String>) -> String {
    match kind {
        ItemKind::Event(e) => {
            if e.calendar.is_empty() {
                "Calendar".to_string()
            } else {
                e.calendar.clone()
            }
        }
        ItemKind::Task(t) => task_source_label(t, src_names),
        ItemKind::Await(a) => await_source_label(a, src_names),
    }
}

/// Labels on a note-origin item's source note (backlog #114/#127), rendered as
/// chips in the card's meta row so the Focus list shows what a task/await is
/// tagged with, not just which collection it came from. Empty for calendar-
/// and markdown-origin items (there is no note to tag).
fn item_note_labels(kind: &ItemKind) -> Vec<NoteLabelInfo> {
    match kind {
        ItemKind::Task(t) => t.note_labels.clone(),
        ItemKind::Await(a) => a.note_labels.clone(),
        ItemKind::Event(_) => Vec::new(),
    }
}

/// Whether a note-origin item is hidden by the label toggles. An item is
/// hidden only when it carries at least one label and *every* one of them is
/// toggled off. A note tagged with one hidden and one visible label still
/// shows — the visible label wins. Unlabelled items are never label-filtered,
/// so toggling labels never hides them.
fn note_labels_hidden(labels: &[NoteLabelInfo], hidden: &HashSet<String>) -> bool {
    !labels.is_empty() && labels.iter().all(|l| hidden.contains(&l.id.to_string()))
}

/// A short "when" description for the meta row (due date / event time).
fn item_when_label(kind: &ItemKind) -> Option<String> {
    match kind {
        ItemKind::Event(e) => {
            let when = if e.all_day || e.start.len() < 11 {
                e.start[..e.start.len().min(10)].to_string()
            } else {
                format!("{} {}", &e.start[..10], &e.start[11..e.start.len().min(16)])
            };
            Some(when)
        }
        ItemKind::Task(t) => t
            .due
            .as_ref()
            .map(|d| format!("Due {}", &d[..d.len().min(10)])),
        ItemKind::Await(a) => Some(format!("Due {}", &a.date[..a.date.len().min(10)])),
    }
}

// ── Horizon filtering ──────────────────────────────────────────────────────────

/// Widest horizon the "Today" / "3 days" / "7 days" selector offers — see
/// the `horizon-btn` buttons in `FocusView`. `include_item`'s `Event` arm
/// never shows an event past `day_diff < horizon`, so `events_res` below
/// only ever needs to fetch this far ahead, not further.
const MAX_HORIZON_DAYS: i32 = 7;

fn include_item(kind: &ItemKind, horizon: u32, today: NaiveDate, now_local: &str) -> bool {
    let h = horizon as i64;
    match kind {
        ItemKind::Event(e) => {
            let d = day_diff(&e.start, today);
            if d < 0 {
                return false;
            }
            // For timed events happening today, exclude those whose end time has
            // already passed.  Event end strings are in local time ("YYYY-MM-DD
            // HH:MM"), so a plain lexicographic comparison against now_local
            // (also local time, same format) gives the correct ordering.
            if d == 0 && e.end.len() >= 16 && e.end.as_str() <= now_local {
                return false;
            }
            d < h
        }
        ItemKind::Task(t) => {
            if t.completed {
                return true;
            }
            match horizon {
                1 => matches!(t.priority, 0..=4),
                3 => matches!(t.priority, 0..=5),
                _ => true,
            }
        }
        ItemKind::Await(a) => {
            let d = day_diff(&a.date, today);
            d < 0 || d < h
        }
    }
}

// ── Suppressed-item state ───────────────────────────────────────────────────────

/// Item UIDs the user has suppressed for "this session", mapped to a dummy
/// `true` (a set, keyed like `HiddenMdSourcesCtx`). Provided in
/// `AuthenticatedApp` — see that file for why: without it, this signal lived
/// inside `FocusView` itself, so navigating away from `/focus` and back
/// unmounted/remounted the component and silently cleared every suppression,
/// even though the button read "Suppress for this session".
#[derive(Clone, Copy)]
pub struct SuppressedFocusItemsCtx(pub RwSignal<HashMap<String, bool>>);

// ── Preview state ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub(crate) enum PreviewState {
    Event(Box<Event>),
    Markdown {
        uid: String,
        summary: String,
        source_file: String,
        source_line: u32,
        /// 0-based index of the tag within its line — which chip to pulse when
        /// several tasks share a source line.
        source_ordinal: u32,
    },
}

// ── Focus view (top-level component) ──────────────────────────────────────────

#[component]
pub fn FocusView() -> impl IntoView {
    let sidebar_refresh = use_sidebar_refresh();
    let events_refresh = use_events_refresh();

    // Session state
    let horizon: RwSignal<u32> = RwSignal::new(7);
    let filter: RwSignal<ItemFilter> = RwSignal::new(ItemFilter::All);
    let suppressed = use_context::<SuppressedFocusItemsCtx>()
        .unwrap_or_else(|| SuppressedFocusItemsCtx(RwSignal::new(HashMap::new())))
        .0;
    let show_suppressed: RwSignal<bool> = RwSignal::new(false);
    let preview: RwSignal<Option<PreviewState>> = RwSignal::new(None);
    let note_target: RwSignal<Option<NoteTarget>> = RwSignal::new(None);
    let event_done: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());
    // Open ad-hoc / source-backed task-await dialog: shared app-level signal
    // (backlog #121) — None = closed, Create = new, Edit/EditChip = existing.
    let todo_modal: RwSignal<Option<TodoModalState>> = crate::app::use_todo_modal();

    // Ticks every 60s so today's proximity-based event urgency stays live
    // (re-ranking the list as events approach) without needing any other
    // reactive trigger. Stays at 0 on SSR — harmless, since the ranking
    // Suspend block never actually runs during SSR (LocalResource always
    // defers to the client; see the comment on `events_res` below).
    let now_tick: RwSignal<u32> = RwSignal::new(0);
    #[cfg(feature = "hydrate")]
    {
        use leptos::wasm_bindgen::prelude::*;

        let tick = Closure::<dyn FnMut()>::new(move || {
            now_tick.update(|n| *n = n.wrapping_add(1));
        });
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                60_000,
            );
        }
        tick.forget();
    }

    // Calendar and markdown source visibility — shared with sidebar via context.
    let all_calendars_ctx =
        use_context::<AllCalendarsCtx>().unwrap_or_else(|| RwSignal::new(vec![]));
    let hidden_md = use_context::<HiddenMdSourcesCtx>()
        .unwrap_or_else(|| HiddenMdSourcesCtx(RwSignal::new(HashSet::new())))
        .0;
    let hidden_notes = use_context::<HiddenNoteCollectionsCtx>()
        .unwrap_or_else(|| HiddenNoteCollectionsCtx(RwSignal::new(HashSet::new())))
        .0;
    let hidden_labels = use_context::<HiddenNoteLabelsCtx>()
        .unwrap_or_else(|| HiddenNoteLabelsCtx(RwSignal::new(HashSet::new())))
        .0;
    let priority_curves_ctx = use_context::<PriorityCurvesCtx>()
        .unwrap_or_else(|| PriorityCurvesCtx(RwSignal::new(PriorityCurves::default())));

    // LocalResource (not Resource) so these do not participate in SSR
    // serialisation.  Both the server and the initial WASM hydration pass
    // render the empty fallback ("Loading…"), giving tachys identical DOM
    // structure on both sides.  Data loads client-side after hydration,
    // exactly the same pattern used by FocusSidebar.
    let events_res = LocalResource::new(move || {
        let _ = events_refresh.get();
        // Focus has no navigable "visible range" — it's an urgency-ranked
        // "what's coming up" list. It used to request a fixed 2-year window
        // (today - 1 day .. today + 730 days), a leftover approximation of
        // "everything" from before get_events did server-side range
        // filtering — but include_item's Event arm never shows anything
        // past `day_diff < horizon`, and horizon tops out at
        // MAX_HORIZON_DAYS (the "7 days" button), so nothing beyond that
        // was ever reachable. Fetching only that far (440KB → a few KB on
        // this account's real data) doesn't change what's displayed, only
        // how much the server has to filter/expand and transfer to get
        // there.
        let (ty, tm, td) = today_ymd();
        let (sy, sm, sd) = add_days(ty, tm, td, -1);
        let (ey, em, ed) = add_days(ty, tm, td, MAX_HORIZON_DAYS);
        let range_start = format!("{:04}-{:02}-{:02}", sy, sm, sd);
        let range_end = format!("{:04}-{:02}-{:02}", ey, em, ed);
        async move { get_events(range_start, range_end).await.unwrap_or_default() }
    });
    let tasks_res = LocalResource::new(move || {
        let _ = events_refresh.get();
        // Markdown source add/edit/delete/sync goes through sidebar_refresh,
        // and deleting a source cascades its tasks away server-side — refetch
        // so they clear from the Focus list without waiting for an unrelated
        // events refresh.
        let _ = sidebar_refresh.get();
        async move { get_tasks().await.unwrap_or_default() }
    });
    let awaits_res = LocalResource::new(move || {
        let _ = events_refresh.get();
        let _ = sidebar_refresh.get();
        async move { get_awaits().await.unwrap_or_default() }
    });
    let md_sources_res = LocalResource::new(move || {
        let _ = sidebar_refresh.get();
        async move { list_markdown_sources().await.unwrap_or_default() }
    });

    let today = Utc::now().date_naive();

    // Free-text filter over the ranked list (title / source label).
    let search: RwSignal<String> = RwSignal::new(String::new());

    // Remaining count is set inside Suspense and displayed outside it so the
    // section-header can live in the grid alongside the sidebar.
    let remaining_sig: RwSignal<usize> = RwSignal::new(0);

    view! {
        <div class="focus-view">
            // Flex row so the sidebar's top edge sits level with the
            // toolbar row, matching how .notes-sidebar lines up with
            // the Starred toggle on the Notes tab — a grid row above the
            // sidebar would push it down by that row's own height instead.
            <div class="sidebar-layout">
                <FocusSidebar/>
                <div class="filter-main">
                    // Top row (backlog #115/#119): add buttons on the left, then
                    // the filter + horizon pills grouped on the right next to
                    // the search box. No date/clock here (it moved off Focus).
                    <div class="screen-toolbar">
                        <div class="focus-add-btns">
                            {move || matches!(filter.get(), ItemFilter::All | ItemFilter::Tasks).then(|| view! {
                                <button
                                    class="tasks-add-btn"
                                    on:click=move |_| todo_modal.set(Some(TodoModalState::Create(TodoKind::Task)))
                                >
                                    "+ Task"
                                </button>
                            })}
                            {move || matches!(filter.get(), ItemFilter::All | ItemFilter::Await).then(|| view! {
                                <button
                                    class="tasks-add-btn"
                                    on:click=move |_| todo_modal.set(Some(TodoModalState::Create(TodoKind::Await)))
                                >
                                    "+ Await"
                                </button>
                            })}
                        </div>
                        <div class="focus-right-controls">
                        <div class="focus-horizon-ctrl focus-filter-ctrl">
                            {[
                                (ItemFilter::All, "All"),
                                (ItemFilter::Calendar, "Calendar"),
                                (ItemFilter::Tasks, "Tasks"),
                                (ItemFilter::Await, "Await"),
                            ]
                            .into_iter()
                            .map(|(value, label)| view! {
                                <button
                                    class=move || if filter.get() == value {
                                        "horizon-btn horizon-btn--active"
                                    } else {
                                        "horizon-btn"
                                    }
                                    on:click=move |_| filter.set(value)
                                >
                                    {label}
                                </button>
                            })
                            .collect_view()}
                        </div>
                        <div class="focus-horizon-ctrl">
                            <button
                                class=move || if horizon.get() == 1 {
                                    "horizon-btn horizon-btn--active"
                                } else {
                                    "horizon-btn"
                                }
                                on:click=move |_| horizon.set(1)
                            >
                                "Today"
                            </button>
                            <button
                                class=move || if horizon.get() == 3 {
                                    "horizon-btn horizon-btn--active"
                                } else {
                                    "horizon-btn"
                                }
                                on:click=move |_| horizon.set(3)
                            >
                                "3 days"
                            </button>
                            <button
                                class=move || if horizon.get() == MAX_HORIZON_DAYS as u32 {
                                    "horizon-btn horizon-btn--active"
                                } else {
                                    "horizon-btn"
                                }
                                on:click=move |_| horizon.set(MAX_HORIZON_DAYS as u32)
                            >
                                "7 days"
                            </button>
                        </div>
                        <input
                            class="form-input notes-search"
                            type="text"
                            placeholder="Search items…"
                            prop:value=move || search.get()
                            on:input=move |ev| search.set(event_target_value(&ev))
                        />
                        </div>
                    </div>

                    <SectionHeader
                        label=Signal::derive(move || filter.get().section_label().to_string())
                        count=Signal::derive(move || remaining_sig.get())
                    />
            <Transition fallback=|| view! { <p class="focus-loading">"Loading…"</p> }>
                {move || {
                    // Read reactive signals before entering async block so
                    // changes re-trigger this closure without re-fetching.
                    let h             = horizon.get();
                    let f             = filter.get();
                    let query         = search.get().trim().to_lowercase();
                    let supp          = suppressed.get();
                    let done_events   = event_done.get();
                    let calendars = all_calendars_ctx.get();
                    // Keyed by (owner_username, calendar_name) — see
                    // CalendarEntry's doc comment for why not bare name.
                    let hidden: HashSet<(String, String)> = calendars
                        .iter()
                        .filter(|(_, v)| !*v)
                        .map(|(c, _)| (c.owner_username.clone(), c.calendar_name.clone()))
                        .collect();
                    let cal_colors: HashMap<String, String> = calendars
                        .iter()
                        .map(|(c, _)| (c.calendar_name.clone(), c.color.clone()))
                        .collect();
                    let hidden_md_set = hidden_md.get();
                    let hidden_coll   = hidden_notes.get();
                    let hidden_lbl    = hidden_labels.get();
                    // Capture `today` from the enclosing component scope rather
                    // than calling Utc::now() again inside the async block.
                    // Both SSR and WASM will then use the *same* date value that
                    // was fixed when the component first rendered, preventing
                    // hydration mismatches at midnight boundaries.
                    let today         = today;
                    // Track `now_tick` so this closure reruns every 60s even if
                    // nothing else changed, keeping today's proximity-based
                    // event scores (and the list order) live.
                    let _ = now_tick.get();
                    // `now_local` is re-evaluated each reactive run (cheap) and
                    // uses the browser's local timezone via js_sys::Date so that
                    // comparisons against event start/end strings (also local
                    // time) are correct.
                    let now_local     = local_now_str();
                    let now_epoch_ms  = local_now_epoch_ms();
                    let curves        = priority_curves_ctx.0.get();

                    Suspend::new(async move {
                        let evts = events_res.await;
                        let tsks = tasks_res.await;
                        let awts = awaits_res.await;
                        let md_sources = md_sources_res.await;

                        // Color lookup for stripe coloring built above from
                        // AllCalendarsCtx (cal_colors), shared with the sidebar.
                        let src_colors: HashMap<String, String> = md_sources
                            .iter()
                            .map(|s| (s.id.to_string(), s.color.clone()))
                            .collect();
                        let src_names: HashMap<String, String> = md_sources
                            .iter()
                            .map(|s| (s.id.to_string(), s.name.clone()))
                            .collect();

                        // Build ranked items from all three source types. `today`
                        // is captured from the component scope above — no
                        // wall-clock call here, keeping SSR and WASM in sync.
                        let mut items: Vec<RankedItem> = Vec::new();

                        for e in &evts {
                            // Skip events whose calendar is hidden. Empty calendar
                            // field means the personal calendar; owner_username is
                            // always populated (get_events sets it to the viewer's
                            // own username for owned rows, the real owner for shared
                            // ones) so this matches the same (owner, name) key
                            // CalendarEntry/AllCalendarsCtx uses.
                            let cal_key = if e.calendar.is_empty() { "personal".to_string() } else { e.calendar.clone() };
                            if hidden.contains(&(e.owner_username.clone(), cal_key)) { continue; }
                            let kind = ItemKind::Event(e.clone());
                            if f.includes(&kind) && include_item(&kind, h, today, &now_local) {
                                let why = event_urgency_explanation(e, &now_local, now_epoch_ms, &curves);
                                let score = why.score;
                                let stripe_color = stripe_color_for(&kind, &cal_colors, &src_colors);
                                let source_label = item_source_label(&kind, &src_names);
                                items.push(RankedItem { id: e.id.clone(), score, why, kind, stripe_color, source_label });
                            }
                        }

                        for t in &tsks {
                            // Skip tasks from hidden markdown sources.
                            if t.source == "markdown"
                                && hidden_md_set.contains(md_source_id(&t.id)) { continue; }
                            // Skip note-origin tasks whose collection is hidden.
                            if let Some(cid) = &t.collection_id {
                                if hidden_coll.contains(cid) { continue; }
                            }
                            // Skip note-origin tasks only when every label on
                            // their note is toggled off — a task carrying one
                            // hidden and one visible label still shows.
                            if note_labels_hidden(&t.note_labels, &hidden_lbl) {
                                continue;
                            }

                            let kind = ItemKind::Task(t.clone());
                            if f.includes(&kind) && include_item(&kind, h, today, &now_local) {
                                let why = task_urgency_explanation(t, today, &curves);
                                let score = why.score;
                                let stripe_color = stripe_color_for(&kind, &cal_colors, &src_colors);
                                let source_label = item_source_label(&kind, &src_names);
                                items.push(RankedItem { id: t.id.clone(), score, why, kind, stripe_color, source_label });
                            }
                        }

                        for a in &awts {
                            // Skip awaits from hidden markdown sources.
                            if hidden_md_set.contains(md_source_id(&a.id)) { continue; }
                            // Skip note-origin awaits whose collection is hidden.
                            if let Some(cid) = &a.collection_id {
                                if hidden_coll.contains(cid) { continue; }
                            }
                            // Skip note-origin awaits only when every label on
                            // their note is toggled off — same visible-label-
                            // wins rule as tasks above.
                            if note_labels_hidden(&a.note_labels, &hidden_lbl) {
                                continue;
                            }

                            let kind = ItemKind::Await(a.clone());
                            if f.includes(&kind) && include_item(&kind, h, today, &now_local) {
                                let why = await_urgency_explanation(&a.date, today, &curves);
                                let score = why.score;
                                let stripe_color = stripe_color_for(&kind, &cal_colors, &src_colors);
                                let source_label = item_source_label(&kind, &src_names);
                                items.push(RankedItem { id: a.id.clone(), score, why, kind, stripe_color, source_label });
                            }
                        }

                        // Free-text search over the title and source label.
                        if !query.is_empty() {
                            items.retain(|it| {
                                item_summary(&it.kind).to_lowercase().contains(&query)
                                    || it.source_label.to_lowercase().contains(&query)
                            });
                        }

                        // Sort: completed to bottom, then by urgency desc.
                        items.sort_by(|a, b| {
                            let a_done = item_is_done(&a.kind, &done_events);
                            let b_done = item_is_done(&b.kind, &done_events);
                            match (a_done, b_done) {
                                (true, false) => std::cmp::Ordering::Greater,
                                (false, true) => std::cmp::Ordering::Less,
                                _ => b.score.cmp(&a.score),
                            }
                        });

                        // Partition into active and suppressed.
                        let mut active: Vec<(usize, RankedItem)> = Vec::new();
                        let mut supp_items: Vec<RankedItem>      = Vec::new();
                        let mut active_rank = 0usize;
                        for item in items {
                            // A recurring event's `id` is the bare series uid
                            // (see `series_key`'s doc comment) — a suppressed
                            // *single occurrence* is stored under the composite
                            // `occurrence_key`, so it must be checked too, or
                            // "just this occurrence" would never actually hide
                            // anything.
                            let occurrence_suppressed = match &item.kind {
                                ItemKind::Event(e) => e
                                    .occurrence_start
                                    .as_ref()
                                    .map(|occ| supp.contains_key(&occurrence_key(&item.id, occ)))
                                    .unwrap_or(false),
                                _ => false,
                            };
                            if supp.contains_key(&item.id) || occurrence_suppressed {
                                supp_items.push(item);
                            } else {
                                active_rank += 1;
                                active.push((active_rank, item));
                            }
                        }

                        let remaining = active.iter()
                            .filter(|(_, it)| !item_is_done(&it.kind, &done_events))
                            .count();
                        let supp_count = supp_items.len();

                        // Push count into the signal so the header (outside
                        // Suspense) stays in sync.
                        remaining_sig.set(remaining);

                        view! {
                            {if active.is_empty() {
                                view! {
                                    <p class="focus-empty">
                                        {if query.is_empty() {
                                            f.empty_message().to_string()
                                        } else {
                                            format!("No matches for “{}”.", query)
                                        }}
                                    </p>
                                }.into_any()
                            } else {
                                view! {
                                    <ul class="focus-cards">
                                        {active.into_iter()
                                            .map(|(rank, item)| view! {
                                                <FocusScoredCard
                                                    item
                                                    rank=rank as u32
                                                    is_suppressed=false
                                                    refresh=events_refresh
                                                    suppressed
                                                    event_done
                                                    preview
                                                    note_target
                                                    todo_modal
                                                />
                                            })
                                            .collect_view()}
                                    </ul>
                                }.into_any()
                            }}

                            {(supp_count > 0).then(|| view! {
                                <div class="focus-suppressed-section">
                                    <button
                                        class="focus-suppressed-toggle"
                                        on:click=move |_| show_suppressed.update(|b| *b = !*b)
                                    >
                                        <span class=move || if show_suppressed.get() {
                                            "focus-chevron focus-chevron--open"
                                        } else {
                                            "focus-chevron"
                                        }>
                                            "›"
                                        </span>
                                        " Suppressed ("{supp_count}")"
                                    </button>
                                    {move || show_suppressed.get().then(|| view! {
                                        <ul class="focus-cards focus-cards--suppressed">
                                            {supp_items.iter()
                                                .map(|item| view! {
                                                    <FocusScoredCard
                                                        item=item.clone()
                                                        rank=0
                                                        is_suppressed=true
                                                        refresh=events_refresh
                                                        suppressed
                                                        event_done
                                                        preview
                                                        note_target
                                                        todo_modal
                                                    />
                                                })
                                                .collect_view()}
                                        </ul>
                                    })}
                                </div>
                            })}
                        }
                    })
                }}
            </Transition>

                    // Preview overlay (Variant A or B depending on item kind)
                    {move || preview.get().map(|ps| view! {
                        <PreviewOverlay state=ps on_close=move || preview.set(None)/>
                    })}
                    {move || note_target.get().map(|t| view! {
                        <NoteGotoModal
                            note_id=t.note_id
                            token=t.token
                            on_close=move || note_target.set(None)
                            on_refresh=Callback::new(move |_| events_refresh.update(|n| *n += 1))
                        />
                    })}
                </div>  // filter-main
            </div>  // sidebar-layout
        </div>
    }
}

// ── FocusScoredCard ────────────────────────────────────────────────────────────

#[component]
fn FocusScoredCard(
    item: RankedItem,
    rank: u32, // 1-based; 0 means suppressed (rank hidden)
    is_suppressed: bool,
    refresh: RwSignal<u32>,
    suppressed: RwSignal<HashMap<String, bool>>,
    event_done: RwSignal<HashSet<String>>,
    preview: RwSignal<Option<PreviewState>>,
    note_target: RwSignal<Option<NoteTarget>>,
    todo_modal: RwSignal<Option<TodoModalState>>,
) -> impl IntoView {
    let today = Utc::now().date_naive();
    let id = item.id.clone();
    let score = item.score;
    let why = item.why.clone();
    let kind = item.kind.clone();
    let done_refresh = crate::app::use_done_refresh();
    // Ad-hoc items (no markdown/note backing) open the edit/delete dialog from
    // the arrow; everything else opens its actual source.
    let goto_title = match &kind {
        ItemKind::Task(t) if t.source == "caldav" => "Edit or delete",
        ItemKind::Await(a) if a.note_id.is_none() && a.source_file.is_none() => "Edit or delete",
        _ => "Go to source",
    };

    let stripe = item.stripe_color.clone();
    let summary = item_summary(&kind).to_string();
    let source_label = item.source_label.clone();
    let when_label = item_when_label(&kind);
    let note_labels = item_note_labels(&kind);
    // Always the item's real per-source color (same one the left stripe
    // uses) — see `source_pill_style`'s doc comment for why this used to
    // only apply to note-collection items.
    let source_pill_style = source_pill_style(&stripe);
    let dot_style = format!("background:{};", stripe);
    let when_class = match &kind {
        ItemKind::Await(a) => pending_due_class(&a.date, today)
            .map(|modifier| format!("pill fsc-when-pill {}", modifier))
            .unwrap_or_else(|| "pill fsc-when-pill".to_string()),
        _ => "pill fsc-when-pill".to_string(),
    };
    let done = item_is_done(&kind, &event_done.get_untracked());
    // Only tasks carry a completion checkbox — see the checkbox markup below.
    let is_task = matches!(kind, ItemKind::Task(_));

    let card_class = if is_suppressed {
        "card-tile focus-scored-card focus-scored-card--suppressed"
    } else if done {
        "card-tile focus-scored-card focus-scored-card--done"
    } else {
        "card-tile focus-scored-card"
    };

    // ── Done toggle ────────────────────────────────────────────────────────────
    let kind_for_toggle = kind.clone();
    let id_for_toggle = id.clone();
    let on_toggle = move |_: leptos::ev::Event| match &kind_for_toggle {
        ItemKind::Event(_) => {
            let eid = id_for_toggle.clone();
            let was_done = event_done.with_untracked(|s| s.contains(&eid));
            event_done.update(|s| {
                if was_done {
                    s.remove(&eid);
                } else {
                    s.insert(eid);
                }
            });
        }
        ItemKind::Task(t) => {
            let uid = t.id.clone();
            let target = !t.completed;
            leptos::task::spawn_local(async move {
                let _ = complete_todo_item(uid, target).await;
                refresh.update(|n| *n += 1);
                done_refresh.update(|n| *n += 1);
            });
        }
        ItemKind::Await(a) => {
            let uid = a.id.clone();
            let target = !a.completed;
            leptos::task::spawn_local(async move {
                let _ = complete_todo_item(uid, target).await;
                refresh.update(|n| *n += 1);
                done_refresh.update(|n| *n += 1);
            });
        }
    };

    // ── Suppress / unsuppress ──────────────────────────────────────────────────
    // A recurring event's `id` is shared by every plain occurrence (see
    // `series_key`'s doc comment), so suppressing it needs the user to pick
    // series-wide vs. just-this-occurrence. Anything else (tasks, awaits,
    // non-recurring events, and already-split recurring exceptions, which
    // get a distinct id per occurrence) suppresses immediately, one click.
    let recurring_uid_and_occurrence: Option<(String, String)> = match &kind {
        ItemKind::Event(e) => e
            .rrule
            .as_ref()
            .and_then(|_| e.occurrence_start.clone())
            .map(|occ| (e.id.clone(), occ)),
        _ => None,
    };
    let show_suppress_choice = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || show_suppress_choice.set(false));

    // Cloned separately for `on_suppress` below — `recurring_uid_and_occurrence`
    // itself (non-`Copy`) is moved into the dialog-rendering closure further
    // down, so `on_suppress` needs its own copy rather than sharing the move.
    let is_recurring = recurring_uid_and_occurrence.is_some();
    let id_for_supp = id.clone();
    let on_suppress = move |_: leptos::ev::MouseEvent| {
        if is_recurring {
            show_suppress_choice.set(true);
        } else {
            suppress_key(suppressed, id_for_supp.clone());
        }
    };
    // Restoring must target whichever key form actually suppressed this item
    // (bare id for a plain item or a whole-series suppression, composite
    // `occurrence_key` for a single-occurrence suppression) — the map is
    // checked directly rather than trusting one fixed key shape.
    let id_for_unsupp = id.clone();
    let recurring_for_unsupp = recurring_uid_and_occurrence.clone();
    let on_unsuppress = move |_: leptos::ev::MouseEvent| {
        let key = match &recurring_for_unsupp {
            Some((uid, occ))
                if suppressed.with_untracked(|m| m.contains_key(&occurrence_key(uid, occ))) =>
            {
                occurrence_key(uid, occ)
            }
            _ => id_for_unsupp.clone(),
        };
        unsuppress_key(suppressed, key);
    };

    // ── Go to source ───────────────────────────────────────────────────────────
    let kind_for_goto = kind.clone();
    let on_goto = move |_: leptos::ev::MouseEvent| {
        match &kind_for_goto {
            ItemKind::Event(e) => {
                preview.set(Some(PreviewState::Event(Box::new(e.clone()))));
            }
            ItemKind::Task(t) if t.source == "caldav" => {
                // Ad-hoc task: no markdown/note source, so the arrow opens the
                // same dialog used to create it, in edit/delete mode.
                todo_modal.set(Some(TodoModalState::Edit(AdHocEdit {
                    uid: t.id.clone(),
                    kind: TodoKind::Task,
                    summary: t.summary.clone(),
                    priority: t.priority,
                    date: String::new(),
                    calendar: t.calendar.clone(),
                    completed: t.completed,
                })));
            }
            ItemKind::Task(t) if t.source == "note" => {
                if let Some(nid) = t.note_id {
                    note_target.set(Some(NoteTarget {
                        note_id: nid,
                        token: note_token_locator(&t.id),
                    }));
                }
            }
            ItemKind::Task(t) => {
                if let (Some(sf), Some(sl)) = (&t.source_file, t.source_line) {
                    preview.set(Some(PreviewState::Markdown {
                        uid: t.id.clone(),
                        summary: t.summary.clone(),
                        source_file: sf.clone(),
                        source_line: sl,
                        source_ordinal: md_ordinal(&t.id),
                    }));
                }
            }
            ItemKind::Await(a) if a.note_id.is_some() => {
                if let Some(nid) = a.note_id {
                    note_target.set(Some(NoteTarget {
                        note_id: nid,
                        token: note_token_locator(&a.id),
                    }));
                }
            }
            ItemKind::Await(a) => {
                if let (Some(sf), Some(sl)) = (&a.source_file, a.source_line) {
                    preview.set(Some(PreviewState::Markdown {
                        uid: a.id.clone(),
                        summary: a.summary.clone(),
                        source_file: sf.clone(),
                        source_line: sl,
                        source_ordinal: md_ordinal(&a.id),
                    }));
                } else {
                    // Ad-hoc await: edit/delete via the same dialog.
                    todo_modal.set(Some(TodoModalState::Edit(AdHocEdit {
                        uid: a.id.clone(),
                        kind: TodoKind::Await,
                        summary: a.summary.clone(),
                        priority: 50,
                        date: a.date[..a.date.len().min(10)].to_string(),
                        calendar: a.calendar.clone(),
                        completed: a.completed,
                    })));
                }
            }
        }
    };

    // Urgency bar fill width (0–100 → 0–40px)
    let bar_fill_px = urgency_bar_fill_px(score);
    let urgency_col = urgency_color(score);

    view! {
        <li class=card_class>
            // Left stripe
            <div
                class="card-tile-stripe"
                style=format!("background:{};", stripe)
            ></div>

            // Rank number
            <div class="fsc-rank">
                {if rank == 0 || is_suppressed || done {
                    "·".to_string()
                } else {
                    rank.to_string()
                }}
            </div>

            // Checkbox — tasks only. Calendar events and awaits aren't
            // completable items; an await just passes when its date rolls
            // over, an event when it elapses.
            {is_task.then(|| view! {
                <input
                    type="checkbox"
                    class="fsc-check"
                    prop:checked=done
                    on:change=on_toggle
                />
            })}

            // Body
            <div class="fsc-body">
                <div class=if done { "fsc-title fsc-title--done" } else { "fsc-title" }>
                    {summary}
                </div>
                <div class="fsc-meta">
                    <span class="pill fsc-source-pill" style=source_pill_style>
                        <span class="pill-dot" style=dot_style></span>
                        {source_label}
                    </span>
                    {when_label.map(|w| view! {
                        <span class=when_class.clone()>{w}</span>
                    })}
                    {note_labels.into_iter().map(|l| {
                        let color = l.color.clone();
                        let name = l.name.clone();
                        let title = format!("Label: {name}");
                        view! {
                            <span class="fsc-label-chip" title=title>
                                <span
                                    class="note-label-dot"
                                    style=format!("background:{color};")
                                ></span>
                                {name}
                            </span>
                        }
                    }).collect_view()}
                </div>
            </div>

            // Urgency cluster (hidden when suppressed). Completed items keep
            // the plain "DONE" cluster without an explanation — there is no
            // active score left to explain (backlog #158).
            {(!is_suppressed).then(|| {
                if done {
                    view! {
                        <div class="fsc-urgency">
                            <span
                                class="fsc-urgency-label"
                                style=format!("color:{};", urgency_col)
                            >
                                "DONE"
                            </span>
                            <div class="fsc-urgency-track">
                                <div
                                    class="fsc-urgency-fill"
                                    style=format!(
                                        "width:{};background:{};",
                                        bar_fill_px, urgency_col
                                    )
                                ></div>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <UrgencyBadge why=why.clone() />
                    }.into_any()
                }
            })}

            // Hover actions
            <div class="fsc-actions">
                <button
                    class="fsc-action-btn"
                    title=goto_title
                    on:click=on_goto
                >
                    "↗"
                </button>
                {if is_suppressed {
                    view! {
                        <button
                            class="fsc-action-btn"
                            title="Restore to list"
                            on:click=on_unsuppress
                        >
                            "↺"
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button
                            class="fsc-action-btn"
                            title="Suppress"
                            on:click=on_suppress
                        >
                            "👁"
                        </button>
                    }.into_any()
                }}
            </div>
        </li>

        {move || {
            recurring_uid_and_occurrence
                .clone()
                .filter(|_| show_suppress_choice.get())
                .map(|(uid, occ)| {
                let uid_series = uid.clone();
                let backdrop_mousedown = RwSignal::new(false);
                view! {
                    <div
                        class="dialog-backdrop"
                        data-esc-id=esc_id.to_string()
                        on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                        on:click=move |_| if backdrop_mousedown.get() { show_suppress_choice.set(false) }
                    >
                        <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                            <div class="dialog-header">
                                <h3 class="dialog-title">"Suppress recurring event"</h3>
                                <button
                                    class="dialog-close"
                                    on:click=move |_| show_suppress_choice.set(false)
                                >
                                    "✕"
                                </button>
                            </div>
                            <div class="dialog-form">
                                <p>"This event repeats. Suppress just this occurrence, or the whole series?"</p>
                            </div>
                            <div class="dialog-actions">
                                <button
                                    class="btn-secondary"
                                    on:click=move |_| {
                                        // Close the dialog *before* suppressing: suppressing
                                        // moves this item out of the active list, which
                                        // disposes this whole card (and `show_suppress_choice`
                                        // with it) — touching the signal afterward panics.
                                        show_suppress_choice.set(false);
                                        suppress_key(suppressed, occurrence_key(&uid, &occ));
                                    }
                                >
                                    "Just this occurrence"
                                </button>
                                <button
                                    class="btn-primary"
                                    on:click=move |_| {
                                        show_suppress_choice.set(false);
                                        suppress_key(suppressed, series_key(&uid_series));
                                    }
                                >
                                    "The whole series"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })
        }}
    }
}

// ── Urgency explanation badge (backlog #158) ──────────────────────────────────

/// The Focus card's urgency cluster, plus the optional "why this score?"
/// explanation. Hover (desktop), keyboard focus, and tap all open the same
/// small popover; a transparent backdrop dismisses it on an outside tap, the
/// same house pattern as `info_hint.rs` / `mention_popover.rs`. The default
/// (closed) rendering is byte-for-byte the old label + bar cluster.
#[component]
fn UrgencyBadge(why: UrgencyExplanation) -> impl IntoView {
    let score = why.score;
    let urgency_col = urgency_color(score);
    let u_label = urgency_label(score);
    let bar_fill_px = urgency_bar_fill_px(score);
    let why = StoredValue::new(why);

    let open = RwSignal::new(false);
    // A tap pins the popover open so it survives the pointer leaving the
    // trigger on touch screens; a second tap (or an outside tap) closes it.
    let pinned = RwSignal::new(false);
    // (right offset, top-or-bottom offset, anchored-to-bottom?)
    let pos = RwSignal::new((8.0_f64, 8.0_f64, false));
    let btn_ref: NodeRef<leptos::html::Button> = NodeRef::new();

    // Anchor below the cluster, right-aligned to it (the cluster sits at the
    // card's right edge), flipping above when it would run off-screen.
    let place = move || {
        #[cfg(feature = "hydrate")]
        if let Some(btn) = btn_ref.get_untracked() {
            let r = btn.get_bounding_client_rect();
            if let Some(win) = web_sys::window() {
                let vw = win
                    .inner_width()
                    .ok()
                    .and_then(|w| w.as_f64())
                    .unwrap_or(0.0);
                let vh = win
                    .inner_height()
                    .ok()
                    .and_then(|h| h.as_f64())
                    .unwrap_or(0.0);
                let right = (vw - r.right()).max(8.0);
                let below = r.bottom() + 6.0;
                // ~180px is a safe upper bound for the popover's height.
                if below + 180.0 <= vh {
                    pos.set((right, below, false));
                } else {
                    pos.set((right, (vh - r.top() + 6.0).max(8.0), true));
                }
            }
        }
    };

    let on_enter = move |_| {
        place();
        open.set(true);
    };
    let on_leave = move |_| {
        if !pinned.get_untracked() {
            open.set(false);
        }
    };
    let on_click = move |ev: leptos::web_sys::MouseEvent| {
        ev.stop_propagation();
        if pinned.get_untracked() {
            pinned.set(false);
            open.set(false);
        } else {
            place();
            pinned.set(true);
            open.set(true);
        }
    };
    let on_focus_in = move |_| {
        place();
        open.set(true);
    };
    let on_focus_out = move |_| {
        if !pinned.get_untracked() {
            open.set(false);
        }
    };
    let on_key_down = move |ev: leptos::web_sys::KeyboardEvent| {
        if ev.key() == "Escape" && open.get_untracked() {
            pinned.set(false);
            open.set(false);
            ev.stop_propagation();
        }
    };

    let backdrop_mousedown = RwSignal::new(false);

    view! {
        <div
            class="fsc-urgency-wrap"
            on:mouseenter=on_enter
            on:mouseleave=on_leave
            on:focusin=on_focus_in
            on:focusout=on_focus_out
            on:keydown=on_key_down
        >
            <button
                type="button"
                class="fsc-urgency"
                aria-label="Why this urgency score?"
                aria-haspopup="dialog"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                node_ref=btn_ref
                on:click=on_click
            >
                <span
                    class="fsc-urgency-label"
                    style=format!("color:{};", urgency_col)
                >
                    {u_label}
                </span>
                <span class="fsc-urgency-track">
                    <span
                        class="fsc-urgency-fill"
                        style=format!(
                            "width:{};background:{};",
                            bar_fill_px, urgency_col
                        )
                    ></span>
                </span>
            </button>
            {move || open.get().then(|| {
                let (right, offset, above) = pos.get();
                let vertical = if above {
                    format!("bottom:{offset}px;")
                } else {
                    format!("top:{offset}px;")
                };
                let why = why.get_value();
                let factors = why
                    .factors
                    .into_iter()
                    .map(|factor| {
                        let row_class = if factor.used {
                            "fsc-why-row fsc-why-row--used"
                        } else {
                            "fsc-why-row"
                        };
                        let fill = format!(
                            "width:{}px;background:{};",
                            (factor.score as u32 * 54 / 100).min(54),
                            urgency_color(factor.score)
                        );
                        view! {
                            <div class=row_class>
                                <span class="fsc-why-factor">{factor.label}</span>
                                <span class="fsc-why-track">
                                    <span class="fsc-why-fill" style=fill></span>
                                </span>
                                <span class="fsc-why-score">{factor.score}</span>
                            </div>
                        }
                    })
                    .collect_view();
                let popover: AnyView = view! {
                    <div
                        class="fsc-why-popover"
                        style=format!("right:{right}px;{vertical}")
                        role="dialog"
                        aria-label="Urgency score breakdown"
                        on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <div class="fsc-why-header">
                            <span class="fsc-why-title">"Urgency " {why.score}</span>
                            <span
                                class="fsc-why-label"
                                style=format!("color:{};", urgency_col)
                            >
                                {u_label}
                            </span>
                        </div>
                        {factors}
                        {why.note.map(|note| view! {
                            <div class="fsc-why-note">{note}</div>
                        })}
                    </div>
                }.into_any();
                // The click-catcher is only needed once the popover is
                // *pinned* (tap/click). While it is merely hover-open, an
                // always-on fixed backdrop would both block the rest of the
                // card and keep the wrapper hovered, so the popover could
                // never dismiss on mouse-leave.
                if pinned.get() {
                    view! {
                        <div
                            class="fsc-why-backdrop"
                            on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                                backdrop_mousedown.set(mouse_event_targets_self(&ev))
                            }
                            on:click=move |_| {
                                if backdrop_mousedown.get() {
                                    pinned.set(false);
                                    open.set(false);
                                }
                            }
                        >
                            {popover}
                        </div>
                    }.into_any()
                } else {
                    popover
                }
            })}
        </div>
    }
}

// ── Ad-hoc task/await modal ────────────────────────────────────────────────────

/// Dialog for ad-hoc (CalDAV-only) tasks and awaits, used both to create a new
/// one and — via the card's "↗ Go to source" arrow — to edit or delete an
/// existing one (backlog #118). A task exposes the 1-100 priority slider; an
/// await exposes a date, mirroring the Notes editor's Task/Await chips.
#[component]
pub(crate) fn TodoItemModal(
    state: TodoModalState,
    refresh: RwSignal<u32>,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    use leptos::prelude::StoredValue;
    use std::sync::Arc;

    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    // Source-backed chip being edited (backlog #121); `None` for create and
    // ad-hoc (CalDAV) edits.
    let chip: Option<TodoChip> = match &state {
        TodoModalState::EditChip(c) => Some(c.clone()),
        _ => None,
    };
    let is_ad_hoc = chip.is_none();

    let kind = match &state {
        TodoModalState::Create(k) => *k,
        TodoModalState::Edit(e) => e.kind,
        TodoModalState::EditChip(c) => {
            if c.kind == "await" {
                TodoKind::Await
            } else {
                TodoKind::Task
            }
        }
    };
    // The parameter field is date-driven for awaits and (markdown's) dated
    // tasks, priority-driven otherwise.
    let dated = match &state {
        TodoModalState::EditChip(c) => c.dated,
        TodoModalState::Create(k) => *k == TodoKind::Await,
        TodoModalState::Edit(e) => e.kind == TodoKind::Await,
    };
    let editing = !matches!(state, TodoModalState::Create(_));
    let edit_uid = match &state {
        TodoModalState::Edit(e) => Some(e.uid.clone()),
        _ => None,
    };
    // The `events.uid` this modal can complete — the ad-hoc item's own uid, or
    // a source-backed chip's derived uid (backlog #132). `None` on the create
    // form, so the action only appears when editing an existing item.
    let complete_uid: Option<String> = match &state {
        TodoModalState::Edit(e) => Some(e.uid.clone()),
        TodoModalState::EditChip(c) => c.origin.event_uid(),
        TodoModalState::Create(_) => None,
    };
    // A source-backed chip never loads once done (`load_todo_chip` returns
    // `None` for a `#done` record), so only ad-hoc edits can already be done.
    let already_done = match &state {
        TodoModalState::Edit(e) => e.completed,
        _ => false,
    };

    let summary = RwSignal::new(match &state {
        TodoModalState::Create(_) => String::new(),
        TodoModalState::Edit(e) => e.summary.clone(),
        TodoModalState::EditChip(c) => c.title.clone(),
    });
    // 1-100, 1 = highest — the canonical task-priority scale (backlog #70).
    let priority = RwSignal::new(match &state {
        TodoModalState::Edit(e) => e.priority,
        TodoModalState::EditChip(c) => c.priority.unwrap_or(50),
        TodoModalState::Create(_) => 50u32,
    });
    let due_date = RwSignal::new(match &state {
        TodoModalState::Edit(e) => e.date.clone(),
        TodoModalState::EditChip(c) => c.date.clone().unwrap_or_default(),
        TodoModalState::Create(_) => String::new(),
    });
    let calendar = RwSignal::new(match &state {
        TodoModalState::Edit(e) if !e.calendar.is_empty() => e.calendar.clone(),
        _ => "personal".to_string(),
    });
    let is_saving = RwSignal::new(false);
    // Pending checkbox actions, applied on Save (like the markdown editor's
    // dialog) rather than the instant they're ticked.
    let mark_done_checked = RwSignal::new(false);
    let remove_checked = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let notes_refresh = crate::app::use_notes_refresh();
    let done_refresh = crate::app::use_done_refresh();

    let all_calendars_ctx =
        use_context::<AllCalendarsCtx>().unwrap_or_else(|| RwSignal::new(vec![]));

    let title = match (editing, kind) {
        (false, TodoKind::Task) => "New task",
        (false, TodoKind::Await) => "New await",
        (true, TodoKind::Task) => "Edit task",
        (true, TodoKind::Await) => "Edit await",
    };
    let hint = if let Some(c) = &chip {
        match &c.origin {
            TodoOrigin::Note { .. } => {
                "Backed by a note — saving rewrites the note's text.".to_string()
            }
            TodoOrigin::Markdown { .. } => {
                "Backed by a markdown file — saving rewrites the source line.".to_string()
            }
        }
    } else if editing {
        format!(
            "Ad-hoc {} — not backed by a markdown file or note. Stored in the selected calendar, which can sync over CalDAV.",
            kind.as_str()
        )
    } else {
        format!(
            "Add an ad-hoc {} — not backed by a markdown file or note. Stored in the selected calendar, which can sync over CalDAV.",
            kind.as_str()
        )
    };

    let edit_uid_submit = edit_uid.clone();
    let chip_for_submit = chip.clone();
    let complete_uid_submit = complete_uid.clone();
    // A source-backed await can have its tag stripped; an ad-hoc one has no tag
    // (its Delete button already covers removing the item itself).
    let remove_available = chip.is_some() && kind == TodoKind::Await;
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();

        // "Remove await tag" — strip the source tag instead of updating it.
        if remove_checked.get_untracked() {
            is_saving.set(true);
            error.set(None);
            let on_close = on_close;
            let chip = chip_for_submit.clone();
            let complete_uid = complete_uid_submit.clone();
            leptos::task::spawn_local(async move {
                let result = match chip {
                    Some(c) => match c.origin {
                        TodoOrigin::Markdown { uid, line, ordinal } => {
                            remove_markdown_task(uid, line, ordinal).await
                        }
                        TodoOrigin::Note { .. } => match complete_uid {
                            Some(uid) => delete_todo_item(uid).await,
                            None => Ok(()),
                        },
                    },
                    None => match complete_uid {
                        Some(uid) => delete_todo_item(uid).await,
                        None => Ok(()),
                    },
                };
                match result {
                    Ok(()) => {
                        refresh.update(|n| *n += 1);
                        notes_refresh.update(|n| *n += 1);
                        done_refresh.update(|n| *n += 1);
                        on_close.with_value(|f| f());
                    }
                    Err(e) => {
                        is_saving.set(false);
                        remove_checked.set(false);
                        error.set(Some(format_server_fn_error(&e.to_string())));
                    }
                }
            });
            return;
        }

        let s = summary.get();
        if s.trim().is_empty() {
            error.set(Some("A title is required.".into()));
            return;
        }
        let d = due_date.get();
        if dated && d.is_empty() {
            error.set(Some("This item needs a date.".into()));
            return;
        }
        let p = (!dated).then(|| priority.get());
        let due = dated.then(|| d.clone());
        let marking_done = mark_done_checked.get_untracked();
        is_saving.set(true);
        error.set(None);
        let on_close = on_close;
        if let Some(c) = chip_for_submit.clone() {
            // Source-backed chip: write the new title/parameter back to its
            // note or markdown origin (backlog #121), then — if "Mark as done"
            // was ticked — complete it like the card checkbox does.
            let complete_uid = complete_uid_submit.clone();
            leptos::task::spawn_local(async move {
                let mut result = update_todo_chip(c.origin, s, p, due).await;
                if result.is_ok() && marking_done {
                    if let Some(uid) = complete_uid {
                        result = complete_todo_item(uid, true).await;
                    }
                }
                match result {
                    Ok(()) => {
                        refresh.update(|n| *n += 1);
                        notes_refresh.update(|n| *n += 1);
                        if marking_done {
                            done_refresh.update(|n| *n += 1);
                        }
                        on_close.with_value(|f| f());
                    }
                    Err(e) => {
                        is_saving.set(false);
                        mark_done_checked.set(false);
                        error.set(Some(format_server_fn_error(&e.to_string())));
                    }
                }
            });
        } else if let Some(uid) = edit_uid_submit.clone() {
            let c = calendar.get();
            leptos::task::spawn_local(async move {
                let mut result = update_todo_item(uid.clone(), s, p, due, c).await;
                if result.is_ok() && marking_done {
                    result = complete_todo_item(uid, true).await;
                }
                match result {
                    Ok(()) => {
                        refresh.update(|n| *n += 1);
                        if marking_done {
                            done_refresh.update(|n| *n += 1);
                        }
                        on_close.with_value(|f| f());
                    }
                    Err(e) => {
                        is_saving.set(false);
                        mark_done_checked.set(false);
                        error.set(Some(format_server_fn_error(&e.to_string())));
                    }
                }
            });
        } else {
            let c = calendar.get();
            let k = kind.as_str().to_string();
            leptos::task::spawn_local(async move {
                match create_todo_item(s, k, p, due, c).await {
                    Ok(()) => {
                        refresh.update(|n| *n += 1);
                        on_close.with_value(|f| f());
                    }
                    Err(e) => {
                        is_saving.set(false);
                        error.set(Some(format_server_fn_error(&e.to_string())));
                    }
                }
            });
        }
    };

    let edit_uid_delete = edit_uid.clone();
    let on_delete = move |_| {
        let Some(uid) = edit_uid_delete.clone() else {
            return;
        };
        is_saving.set(true);
        error.set(None);
        let on_close = on_close;
        leptos::task::spawn_local(async move {
            match delete_todo_item(uid).await {
                Ok(()) => {
                    refresh.update(|n| *n += 1);
                    on_close.with_value(|f| f());
                }
                Err(e) => {
                    is_saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));

    view! {
        <div
            class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div
                class="dialog add-todo-dialog"
                on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
            >
                <div class="dialog-header">
                    <h3>{title}</h3>
                    <button
                        class="dialog-close"
                        on:click=move |_| on_close.with_value(|f| f())
                    >
                        "✕"
                    </button>
                </div>

                <form class="add-todo-form" on:submit=on_submit>
                    <p class="form-hint">{hint}</p>

                    // Title
                    <div class="form-group">
                        <label class="form-label">"Title"</label>
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Title"
                            autofocus=true
                            prop:value=summary
                            on:input=move |ev| summary.set(event_target_value(&ev))
                        />
                    </div>

                    // Priority-driven task → priority slider; await / dated task
                    // → date.
                    {if dated {
                        view! {
                            <div class="form-group">
                                <label class="form-label">"Date"</label>
                                <input
                                    type="date"
                                    class="form-input"
                                    prop:value=due_date
                                    on:input=move |ev| due_date.set(event_target_value(&ev))
                                />
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <PrioritySlider priority=priority disabled=Signal::derive(move || is_saving.get()) />
                        }.into_any()
                    }}

                    // Calendar — ad-hoc items only; a source-backed chip belongs
                    // to its note/markdown calendar.
                    {is_ad_hoc.then(|| view! {
                        <div class="form-group">
                            <label class="form-label">"Calendar"</label>
                            // Owned calendars only: create_todo_item/update_todo_item
                            // write under the current user's own username with no
                            // cross-owner support, same reasoning as the event-creation
                            // picker in event_form.rs.
                            {move || {
                                let cals: Vec<_> = all_calendars_ctx.get().into_iter()
                                    .filter(|(c, _)| c.is_owned() && !c.read_only)
                                    .collect();
                                view! {
                                    <select
                                        class="form-select"
                                        on:change=move |ev| calendar.set(event_target_value(&ev))
                                    >
                                        {cals.into_iter()
                                            .map(|(c, _)| {
                                                let name = c.calendar_name.clone();
                                                let label = c.display_name.clone();
                                                view! {
                                                    <option
                                                        value=name.clone()
                                                        selected=move || calendar.get() == name
                                                    >
                                                        {label}
                                                    </option>
                                                }
                                            })
                                            .collect_view()}
                                    </select>
                                }
                            }}
                        </div>
                    })}

                    // "Mark as done" checkbox — same affordance as the in-app
                    // markdown editor's edit dialog (backlog #162). Like that
                    // dialog, it only records the intent; Save applies it.
                    // Tasks only: an await has no completion.
                    {(editing && !already_done && kind == TodoKind::Task && complete_uid.is_some()).then(|| view! {
                        <label class="form-check-label">
                            <input
                                type="checkbox"
                                prop:checked=move || mark_done_checked.get()
                                disabled=move || is_saving.get()
                                on:change=move |ev| mark_done_checked.set(event_target_checked(&ev))
                            />
                            "Mark as done"
                        </label>
                    })}

                    // Awaits have no completion, but a source-backed one can
                    // have its tag removed. Save applies it, like the task box.
                    {(editing && remove_available).then(|| view! {
                        <label class="form-check-label">
                            <input
                                type="checkbox"
                                prop:checked=move || remove_checked.get()
                                disabled=move || is_saving.get()
                                on:change=move |ev| remove_checked.set(event_target_checked(&ev))
                            />
                            "Remove await tag"
                        </label>
                    })}

                    {move || error.get().map(|e| view! {
                        <p class="form-error">{e}</p>
                    })}

                    <div class="form-actions form-actions--split">
                        <div>
                            {(editing && is_ad_hoc).then(|| view! {
                                <button
                                    type="button"
                                    class="btn-danger"
                                    disabled=is_saving
                                    on:click=move |_| on_delete(())
                                >
                                    "Delete"
                                </button>
                            })}
                        </div>
                        <div class="form-actions-right">
                            <button
                                type="button"
                                class="btn-secondary"
                                on:click=move |_| on_close.with_value(|f| f())
                            >
                                "Cancel"
                            </button>
                            <button
                                type="submit"
                                class="btn-primary"
                                disabled=is_saving
                            >
                                {move || if is_saving.get() {
                                    "Saving…".to_string()
                                } else if editing {
                                    "Save".to_string()
                                } else {
                                    format!("Add {}", kind.as_str())
                                }}
                            </button>
                        </div>
                    </div>
                </form>
            </div>
        </div>
    }
}

// ── Preview overlays ───────────────────────────────────────────────────────────

#[allow(private_interfaces)]
#[component]
pub(crate) fn PreviewOverlay(
    state: PreviewState,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    use leptos::prelude::StoredValue;
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    match state {
        PreviewState::Event(e) => {
            // Calendar events open the real edit dialog (backlog #157) instead
            // of the old read-only preview, so they can be inspected and
            // edited without leaving Focus.
            let ev = *e;
            let start = if ev.all_day {
                ev.start.clone()
            } else {
                input_dt_from_server(&ev.start)
            };
            let end = if ev.all_day {
                ev.end.clone()
            } else {
                input_dt_from_server(&ev.end)
            };
            let events_refresh = use_events_refresh();
            let ev_for_nav = ev.clone();
            // Full-page navigation (not client-side routing) so CalendarView
            // can pick up `?highlight=`/`?date=` on mount, jump to the event's
            // week, and pulse it — same technique as Notes' "Show in
            // collection" (backlog #63).
            let show_in_calendar = Callback::new(move |_: ()| {
                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(win) = web_sys::window() {
                        let date = ev_for_nav.start.get(..10).unwrap_or(&ev_for_nav.start);
                        let url = format!(
                            "/?highlight={}&date={}",
                            encode_query_value(&ev_for_nav.id),
                            date
                        );
                        let _ = win.location().set_href(&url);
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                let _ = &ev_for_nav;
            });
            view! {
                <EventFormDialog
                    event=Some(ev)
                    start=start
                    end=end
                    on_close=move || on_close.with_value(|f| f())
                    on_saved=move || {
                        on_close.with_value(|f| f());
                        events_refresh.update(|n| *n += 1);
                    }
                    on_show_in_calendar=show_in_calendar
                />
            }
            .into_any()
        }
        PreviewState::Markdown {
            uid,
            summary,
            source_file,
            source_line,
            source_ordinal,
        } => {
            let show_rendered = RwSignal::new(true);
            let backdrop_mousedown = RwSignal::new(false);
            let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));
            view! {
                <div
                    class="dialog-backdrop"
                    data-esc-id=esc_id.to_string()
                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                    on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
                >
                    <div
                        class="dialog focus-preview-dialog focus-preview-dialog--wide"
                        on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <MarkdownPreviewInner
                            uid
                            summary
                            source_file
                            source_line
                            source_ordinal
                            show_rendered
                            on_close=move || on_close.with_value(|f| f())
                        />
                    </div>
                </div>
            }
            .into_any()
        }
    }
}

// ── Variant B: Markdown file preview ──────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct NavEntry {
    uid: String,
    display_name: String,
    source_file: String,
    source_line: u32,
    source_ordinal: u32,
    summary: String,
}

#[component]
fn MarkdownPreviewInner(
    uid: String,
    summary: String,
    source_file: String,
    source_line: u32,
    source_ordinal: u32,
    show_rendered: RwSignal<bool>,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    use leptos::prelude::StoredValue;
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let local_editor_ctx = use_context::<LocalEditorCtx>();
    let open_settings = use_context::<OpenSettingsCtx>();
    let open_error = RwSignal::new(Option::<String>::None);
    let pull_error = RwSignal::new(Option::<String>::None);
    let auto_sync_busy = RwSignal::new(false);
    #[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
    let _ = auto_sync_busy;
    let copy_hint = RwSignal::new(Option::<String>::None);

    // Inline "where is your clone?" prompt for a git source with no mapping
    // (backlog #164). Scoped to the source that opened this preview instead of
    // dead-ending into the Settings tab.
    let show_clone_setup = RwSignal::new(false);
    let clone_path = RwSignal::new(String::new());
    let clone_error = RwSignal::new(Option::<String>::None);

    // Signal to show/hide the in-app editor.
    let show_editor: RwSignal<bool> = RwSignal::new(false);
    // "Pull out into Note" (backlog #122).
    let show_pull_out: RwSignal<bool> = RwSignal::new(false);

    let root = NavEntry {
        uid: uid.clone(),
        display_name: source_file
            .rsplit('/')
            .next()
            .unwrap_or(&source_file)
            .to_string(),
        source_file: source_file.clone(),
        source_line,
        source_ordinal,
        summary: summary.clone(),
    };
    let nav_stack: RwSignal<Vec<NavEntry>> = RwSignal::new(vec![root]);
    let wikilink_error: RwSignal<Option<String>> = RwSignal::new(None);
    // Bumped after a rendered checkbox completes a task (backlog #112) so the
    // Tasks/Tickler/Focus list this preview was opened from drops the item —
    // same app-level signal those views' own checkboxes bump.
    let events_refresh = use_events_refresh();
    #[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
    let _ = events_refresh;
    let sidebar_refresh = use_sidebar_refresh();
    let done_refresh = crate::app::use_done_refresh();
    #[cfg(not(feature = "hydrate"))]
    let _ = done_refresh;
    // Click-to-open contact-detail popover for a resolved `@mention` in the
    // rendered markdown (backlog #91) — same shape as note_card.rs's own.
    let mention_popover: RwSignal<Option<(ContactDto, f64, f64)>> = RwSignal::new(None);
    // One-shot flash of the pill for `source_line` (backlog #74) — guarded so
    // it only fires for the root item that opened this modal (not one
    // reached via a wikilink) and only once even if the Rendered tab is
    // revisited or `render_res` refetches (e.g. after an in-app edit save).
    let pill_flashed: RwSignal<bool> = RwSignal::new(false);
    #[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
    let _ = pill_flashed;

    let current_uid =
        move || nav_stack.with(|s| s.last().map(|e| e.uid.clone()).unwrap_or_default());
    let current_sf =
        move || nav_stack.with(|s| s.last().map(|e| e.source_file.clone()).unwrap_or_default());
    let current_line = move || nav_stack.with(|s| s.last().map(|e| e.source_line).unwrap_or(1));
    let current_name =
        move || nav_stack.with(|s| s.last().map(|e| e.display_name.clone()).unwrap_or_default());
    let current_summary =
        move || nav_stack.with(|s| s.last().map(|e| e.summary.clone()).unwrap_or_default());
    let can_go_back = move || nav_stack.with(|s| s.len() > 1);

    let md_sources_res: LocalResource<Vec<crate::api::MarkdownSourceInfo>> =
        LocalResource::new(move || {
            let _ = sidebar_refresh.get();
            async move { list_markdown_sources().await.unwrap_or_default() }
        });

    let current_source = move || {
        let source_id = md_source_id_i64(&current_uid());
        md_sources_res
            .get()
            .unwrap_or_default()
            .into_iter()
            .find(|s| Some(s.id) == source_id)
    };

    let current_target = move || {
        let uid = current_uid();
        let rel_path = md_rel_path(&uid).to_string();
        let prefs = local_editor_ctx.map(|c| c.prefs.get()).unwrap_or_default();
        let detected = local_editor_ctx
            .map(|c| c.detected_on_server.get())
            .unwrap_or(false);
        let on_server = prefs.on_server.unwrap_or(detected);
        resolve_target(current_source().as_ref(), &rel_path, &prefs, on_server)
    };

    let local_abs_path = move || match current_target() {
        OpenTarget::LocalClone { abs_path } => Some(abs_path),
        _ => None,
    };

    let file_res = LocalResource::new(move || {
        // A task completed from another surface (the shared edit modal, a note
        // chip, an SSE update) bumps `events_refresh`; keep the raw source in
        // step so the File tab and editor don't show a stale line.
        let _ = events_refresh.get();
        let u = current_uid();
        async move { get_markdown_file_content(u).await }
    });

    // Stable key signal — update this on navigation instead of replacing the resource.
    // Storing LocalResource inside RwSignal disposes the resource mid-flight and panics
    // any in-flight Suspend::new future that still holds a handle to the old value.
    //
    // Seeded from `uid` directly rather than `current_uid()` (backlog #76):
    // `current_uid()` reads `nav_stack` via `.with()`, and calling it here —
    // eagerly, before any reactive scope (Effect/view closure) exists to
    // subscribe — tripped Leptos's "access outside a reactive tracking
    // context" warning on every modal open. It was harmless only because
    // `nav_stack` had just been built from the same `uid` a few lines above
    // and hadn't changed yet; reading `uid` itself sidesteps the warning
    // instead of relying on that coincidence.
    let render_key: RwSignal<String> = RwSignal::new(uid.clone());
    let render_res = LocalResource::new(move || {
        // Refetch when a task is completed/reopened from another surface — the
        // shared edit modal or a note chip bumps `events_refresh` — so its
        // rendered pill flips without a manual refresh.
        let _ = events_refresh.get();
        let u = render_key.get();
        async move { get_markdown_rendered(u).await }
    });

    // An external editor (e.g. VSCodium opening the server copy) writes the
    // file behind our back. When the user comes back to the browser, re-read
    // both views so the Rendered tab doesn't keep showing pre-edit HTML while
    // Source already shows the new content.
    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
    {
        let handle = window_event_listener(leptos::ev::focus, move |_| {
            file_res.refetch();
            render_res.refetch();

            // Returning from an external editor may also have changed the
            // parsed task/await index. Trigger a sync so Focus catches up, not
            // just the Source/Rendered preview.
            if show_editor.get_untracked() || auto_sync_busy.get_untracked() {
                return;
            }
            let Some(source_id) = md_source_id_i64(&current_uid()) else {
                return;
            };
            let old_last_scan = current_source().and_then(|s| s.last_scan);
            auto_sync_busy.set(true);
            leptos::task::spawn_local(async move {
                if let Ok(err) = sync_source_now(source_id, old_last_scan, 60).await {
                    pull_error.set(err);
                    events_refresh.update(|n| *n += 1);
                }
                md_sources_res.refetch();
                auto_sync_busy.set(false);
            });
        });
        on_cleanup(move || handle.remove());
    }

    view! {
        // When the in-app editor is open, hide the preview shell and show the editor
        // as a direct flex child of the dialog (filling its remaining height).
        {move || {
            let uid_str = current_uid();
            let source_id = md_source_id_i64(&uid_str).unwrap_or(0);
            let rel_path_str = md_rel_path(&uid_str).to_string();
            if show_editor.get() && source_id != 0 && !rel_path_str.is_empty() {
                view! {
                    <MarkdownFileEditor
                        source_id=source_id
                        rel_path=rel_path_str
                        on_close=Callback::new(move |_: ()| show_editor.set(false))
                        on_saved=Callback::new(move |_: ()| {
                            file_res.refetch();
                            render_res.refetch();
                            // The editor writes straight to disk; the background
                            // markdown sync that turns `#task`/`#await` tags into
                            // `events` rows runs separately. Bump the app-level
                            // signal so the Focus list behind this preview drops
                            // a removed tag (and picks up an added one) as soon
                            // as the save returns, rather than relying solely on
                            // that sync's SSE broadcast.
                            events_refresh.update(|n| *n += 1);
                        })
                    />
                }.into_any()
            } else {
                let _: () = view! { <></> };
                ().into_any()
            }
        }}

        // Preview shell — hidden while the editor is active.
        <div class="fp-preview-shell" style=move || if show_editor.get() { "display:none" } else { "" }>

        <div class="fp-header">
            {move || can_go_back().then(|| view! {
                <button
                    class="btn-secondary-small"
                    on:click=move |_| {
                        wikilink_error.set(None);
                        nav_stack.update(|s| { s.pop(); });
                        render_key.set(current_uid());
                    }
                >"← Back"</button>
            })}
            <div class="fp-icon fp-icon--md">
                "📄"
            </div>
            <div class="fp-header-text">
                <div class="fp-title">{move || current_name()}</div>
                <div class="fp-subtitle">
                    "Line "{move || current_line()}" · "{move || current_summary()}
                </div>
            </div>
            <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
        </div>

        <div class="fp-body fp-body--path">
            <div class="fp-row">
                <span class="fp-key">"Path"</span>
                <span class="fp-val fp-path">{move || current_sf()}</span>
            </div>
            <div class="fp-row fp-row--target">
                <span class="fp-key">"Target"</span>
                <span class="fp-val fp-path">
                    {move || match current_target() {
                        OpenTarget::LocalClone { abs_path } => format!("This device · {abs_path}"),
                        OpenTarget::HostCopy => "On the tidl host".to_string(),
                        OpenTarget::ConfigureClone => "No local clone on this device".to_string(),
                        OpenTarget::Unavailable => {
                            "On the tidl host (not reachable from this browser)".to_string()
                        }
                    }}
                </span>
                {move || local_abs_path().map(|path| view! {
                    <button
                        class="btn-secondary-small"
                        on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                let path = path.clone();
                                leptos::task::spawn_local(async move {
                                    let ok = crate::util::copy_to_clipboard(&path).await;
                                    copy_hint.set(Some(if ok {
                                        "Path copied".to_string()
                                    } else {
                                        "Could not copy path".to_string()
                                    }));
                                });
                            }
                            #[cfg(not(feature = "hydrate"))]
                            let _ = &path;
                        }
                    >"Copy path"</button>
                })}
            </div>
            {move || {
                (show_clone_setup.get()
                    && matches!(current_target(), OpenTarget::ConfigureClone))
                    .then(|| {
                        let source = current_source();
                        let url = source.as_ref().and_then(|s| s.url.clone()).unwrap_or_default();
                        let key = normalize_repo_key(&url);
                        let name = source.map(|s| s.name).unwrap_or_default();
                        let key_for_save = key.clone();
                        let ctx = local_editor_ctx;
                        view! {
                            <div class="fp-clone-setup">
                                <label class="form-label">
                                    {format!("Where is your clone of “{name}”?")}
                                </label>
                                <div class="fp-clone-setup-row">
                                    <input
                                        type="text"
                                        class="form-input"
                                        placeholder="/absolute/path/to/your/clone"
                                        prop:value=move || clone_path.get()
                                        on:input=move |ev| clone_path.set(event_target_value(&ev))
                                    />
                                    <button
                                        class="btn-primary"
                                        on:click=move |_| {
                                            clone_error.set(None);
                                            let path = clone_path.get_untracked().trim().to_string();
                                            if path.is_empty() {
                                                clone_error.set(Some(
                                                    "Enter the clone folder path.".to_string(),
                                                ));
                                                return;
                                            }
                                            if !crate::local_editor::is_absolute_path(&path) {
                                                clone_error.set(Some(
                                                    "Use an absolute path (starting with / or a drive letter)."
                                                        .to_string(),
                                                ));
                                                return;
                                            }
                                            let Some(ctx) = ctx else {
                                                clone_error.set(Some(
                                                    "Local editor settings are not available."
                                                        .to_string(),
                                                ));
                                                return;
                                            };
                                            let mut prefs = ctx.prefs.get_untracked();
                                            prefs.git_roots.insert(key_for_save.clone(), path);
                                            match save_prefs(&prefs) {
                                                Ok(()) => {
                                                    ctx.prefs.set(prefs);
                                                    show_clone_setup.set(false);
                                                }
                                                Err(e) => clone_error.set(Some(e)),
                                            }
                                        }
                                    >"Use this folder"</button>
                                    <button
                                        class="btn-cancel"
                                        on:click=move |_| show_clone_setup.set(false)
                                    >"Cancel"</button>
                                </div>
                                <p class="form-hint">
                                    "Use your own clone, not tidl's managed copy. Commit and push \
                                     from there to bring changes into tidl."
                                </p>
                                {move || clone_error.get().map(|e| view! {
                                    <p class="form-error">{e}</p>
                                })}
                            </div>
                        }
                    })
            }}
            {move || pull_error
                .get()
                .or_else(|| current_source().and_then(|s| s.last_error))
                .map(|e| view! { <p class="form-error">{e}</p> })}
            {move || copy_hint.get().map(|h| view! { <p class="form-hint">{h}</p> })}
        </div>

        <div class="fp-tab-bar">
            <button
                class=move || if show_rendered.get() { "fp-tab fp-tab--active" } else { "fp-tab" }
                on:click=move |_| {
                    show_rendered.set(true);
                    wikilink_error.set(None);
                    // The file may have been edited outside tidl (e.g. opened on
                    // the server with a desktop editor) since this resource was
                    // last awaited. Re-read it when the tab is selected.
                    render_res.refetch();
                }
            >
                "Rendered"
            </button>
            <button
                class=move || if !show_rendered.get() { "fp-tab fp-tab--active" } else { "fp-tab" }
                on:click=move |_| {
                    show_rendered.set(false);
                    file_res.refetch();
                }
            >
                "Source"
            </button>
        </div>

        {move || wikilink_error.get().map(|e| view! { <p class="form-error">{e}</p> })}

        {move || if !show_rendered.get() {
            view! {
                <div class="fp-body fp-body--code">
                    <Suspense fallback=|| view! { <p class="fp-loading">"Loading file…"</p> }>
                        {move || Suspend::new(async move {
                            match file_res.await {
                                Ok((_fname, highlighted_line, lines)) => {
                                    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
                                    {
                                        use wasm_bindgen::prelude::*;
                                        let closure = Closure::once(|| {
                                            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                                                if let Some(el) = doc.get_element_by_id("fp-hl-line") {
                                                    let opts = web_sys::ScrollIntoViewOptions::new();
                                                    opts.set_block(web_sys::ScrollLogicalPosition::Center);
                                                    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                                                    el.scroll_into_view_with_scroll_into_view_options(&opts);
                                                }
                                            }
                                        });
                                        if let Some(win) = web_sys::window() {
                                            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                                                closure.as_ref().unchecked_ref(),
                                                50,
                                            );
                                        }
                                        closure.forget();
                                    }

                                    view! {
                                        <pre class="fp-code-block">
                                            {lines.iter().enumerate().map(|(i, line)| {
                                                let ln = (i + 1) as u32;
                                                let is_hl = ln == highlighted_line;
                                                let text = line.clone();
                                                view! {
                                                    <div
                                                        id=if is_hl { "fp-hl-line" } else { "" }
                                                        class=if is_hl {
                                                            "fp-code-line fp-code-line--hl"
                                                        } else {
                                                            "fp-code-line"
                                                        }
                                                    >
                                                        <span class="fp-ln">{ln}</span>
                                                        <span class="fp-code-text">{text}</span>
                                                    </div>
                                                }
                                            }).collect_view()}
                                        </pre>
                                    }.into_any()
                                }
                                Err(e) => view! {
                                    <p class="focus-empty">
                                        "Could not load file: "{e.to_string()}
                                    </p>
                                }.into_any(),
                            }
                        })}
                    </Suspense>
                </div>
            }.into_any()
        } else {
            view! {
                <div class="fp-body--rendered">
                    <Suspense fallback=|| view! { <p class="fp-loading">"Rendering…"</p> }>
                        {move || Suspend::new(async move {
                            match render_res.await {
                                Ok(html) => {
                                    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
                                    if !pill_flashed.get_untracked()
                                        && nav_stack.with_untracked(|s| s.len() == 1)
                                    {
                                        use wasm_bindgen::prelude::*;
                                        let target_line = source_line;
                                        let target_ordinal = source_ordinal;
                                        let closure = Closure::once(move || {
                                            let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return; };
                                            // Pin the exact chip: a line can hold
                                            // several tags, so `data-md-line` alone
                                            // would pulse the first one instead of
                                            // the task that opened this preview.
                                            let selector = format!("[data-md-line='{target_line}'][data-md-ordinal='{target_ordinal}']");
                                            let Some(el) = doc.query_selector(&selector).ok().flatten() else { return; };
                                            let opts = web_sys::ScrollIntoViewOptions::new();
                                            opts.set_block(web_sys::ScrollLogicalPosition::Center);
                                            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                                            el.scroll_into_view_with_scroll_into_view_options(&opts);
                                            let html_el: web_sys::HtmlElement = el.unchecked_into();
                                            // Task chips carry their urgency colour inline as
                                            // `--chip-color` (see `priority_chip_html` in
                                            // tidl-markdown); reuse it for the pulse. Await/done
                                            // chips have none, so the CSS default
                                            // (note-highlight purple) applies instead.
                                            let inline_color = html_el.style().get_property_value("--chip-color").unwrap_or_default();
                                            if !inline_color.is_empty() {
                                                let _ = html_el.style().set_property("--md-pill-highlight-color", &inline_color);
                                            }
                                            let _ = html_el.class_list().add_1("md-pill--highlight");
                                            // Strip the class once the (self-terminating,
                                            // iteration-count: 3) animation finishes — "Edit in
                                            // app" toggles this preview's wrapper to `display:none`
                                            // and back, and a display:none element is removed from
                                            // the render tree, which resets any CSS animation still
                                            // applied to it; leaving the class in place would replay
                                            // the pulse on every close/cancel even though
                                            // `pill_flashed` already fired.
                                            let cleanup_el = html_el.clone();
                                            let end_closure = Closure::once(move || {
                                                let _ = cleanup_el.class_list().remove_1("md-pill--highlight");
                                            });
                                            let _ = html_el.add_event_listener_with_callback(
                                                "animationend",
                                                end_closure.as_ref().unchecked_ref(),
                                            );
                                            end_closure.forget();
                                        });
                                        if let Some(win) = web_sys::window() {
                                            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                                                closure.as_ref().unchecked_ref(),
                                                50,
                                            );
                                        }
                                        closure.forget();
                                        pill_flashed.set(true);
                                    }
                                    view! {
                                    <div
                                        class="fp-rendered"
                                        inner_html=html
                                        on:click=move |_ev: leptos::ev::MouseEvent| {
                                            #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
                                            {
                                                use leptos::wasm_bindgen::JsCast as _;
                                                let ev = _ev;
                                                let Some(target) = ev.target() else { return; };

                                                // Task/await checkbox (backlog
                                                // #112/#117): the checkbox is the
                                                // click target; walk up to its
                                                // badge span for the source line.
                                                // A rich `#done` chip carries
                                                // `data-md-reopen`, so the same
                                                // click re-opens it; otherwise it
                                                // completes. Disabled optimistically
                                                // so a double-click can't fire twice.
                                                if let Ok(input) =
                                                    target.clone().dyn_into::<web_sys::HtmlInputElement>()
                                                {
                                                    if input.type_() == "checkbox" {
                                                        let reopen =
                                                            input.get_attribute("data-md-reopen").is_some();
                                                        let mut node = input.parent_element();
                                                        let mut line: Option<u32> = None;
                                                        // 0-based index of the tag within its
                                                        // line (see `data-md-ordinal` in
                                                        // tidl-markdown) — a line may hold
                                                        // several tags, so this picks the one
                                                        // whose checkbox was clicked.
                                                        let mut ordinal: u32 = 0;
                                                        while let Some(n) = node {
                                                            if let Some(v) = n.get_attribute("data-md-line") {
                                                                line = v.parse().ok();
                                                                ordinal = n
                                                                    .get_attribute("data-md-ordinal")
                                                                    .and_then(|v| v.parse().ok())
                                                                    .unwrap_or(0);
                                                                break;
                                                            }
                                                            node = n.parent_element();
                                                        }
                                                        if let Some(line) = line {
                                                            input.set_disabled(true);
                                                            let u = current_uid();
                                                            leptos::task::spawn_local(async move {
                                                                let result = if reopen {
                                                                    reopen_markdown_task(u, line, ordinal).await
                                                                } else {
                                                                    complete_markdown_task(u, line, ordinal).await
                                                                };
                                                                match result {
                                                                    Ok(()) => {
                                                                        wikilink_error.set(None);
                                                                        events_refresh.update(|n| *n += 1);
                                                                        done_refresh.update(|n| *n += 1);
                                                                        render_res.refetch();
                                                                        file_res.refetch();
                                                                    }
                                                                    Err(e) => wikilink_error.set(Some(
                                                                        format_server_fn_error(&e.to_string()),
                                                                    )),
                                                                }
                                                            });
                                                        }
                                                        return;
                                                    }
                                                }

                                                let Ok(el) = target.dyn_into::<web_sys::Element>() else { return; };

                                                // Task/await chip body (backlog #121): a click on
                                                // the pill — not its checkbox, handled above —
                                                // opens the shared edit modal, resolved from the
                                                // chip's source line + ordinal. A `#done` chip
                                                // loads as `None` and so opens nothing.
                                                {
                                                    let mut node = Some(el.clone());
                                                    let mut chip: Option<(u32, u32)> = None;
                                                    while let Some(n) = node {
                                                        if let Some(v) = n.get_attribute("data-md-line") {
                                                            chip = Some((
                                                                v.parse().unwrap_or(0),
                                                                n.get_attribute("data-md-ordinal")
                                                                    .and_then(|v| v.parse().ok())
                                                                    .unwrap_or(0),
                                                            ));
                                                            break;
                                                        }
                                                        node = n.parent_element();
                                                    }
                                                    if let Some((line, ordinal)) = chip {
                                                        ev.stop_propagation();
                                                        let u = current_uid();
                                                        let todo_modal = crate::app::use_todo_modal();
                                                        leptos::task::spawn_local(async move {
                                                            let origin = TodoOrigin::Markdown { uid: u, line, ordinal };
                                                            if let Ok(Some(chip)) = load_todo_chip(origin).await {
                                                                todo_modal.set(Some(TodoModalState::EditChip(chip)));
                                                            }
                                                        });
                                                        return;
                                                    }
                                                }

                                                enum MdClickHit {
                                                    Wikilink(String),
                                                    Mention(String, web_sys::DomRect),
                                                }
                                                let hit = {
                                                    let mut node = Some(el);
                                                    loop {
                                                        let Some(n) = node else { break None; };
                                                        if let Some(val) = n.get_attribute("data-wikilink") {
                                                            break Some(MdClickHit::Wikilink(val));
                                                        }
                                                        if let Some(val) = n.get_attribute("data-mention") {
                                                            break Some(MdClickHit::Mention(val, n.get_bounding_client_rect()));
                                                        }
                                                        node = n.parent_element();
                                                    }
                                                };

                                                match hit {
                                                    Some(MdClickHit::Wikilink(name)) => {
                                                        let uid_for_resolve = current_uid();
                                                        leptos::task::spawn_local(async move {
                                                            match resolve_wikilink(uid_for_resolve, name.clone()).await {
                                                                Ok(Some(resolved)) => {
                                                                    wikilink_error.set(None);
                                                                    let stem = resolved.source_file
                                                                        .rsplit('/')
                                                                        .next()
                                                                        .unwrap_or(&resolved.source_file)
                                                                        .to_string();
                                                                    let new_uid = resolved.uid.clone();
                                                                    let entry = NavEntry {
                                                                        uid: resolved.uid,
                                                                        display_name: stem.clone(),
                                                                        source_file: resolved.source_file,
                                                                        source_line: 1,
                                                                        source_ordinal: 0,
                                                                        summary: stem,
                                                                    };
                                                                    nav_stack.update(|s| s.push(entry));
                                                                    render_key.set(new_uid);
                                                                }
                                                                Ok(None) => {
                                                                    wikilink_error.set(Some(format!("[[{}]]: file not found", name)));
                                                                }
                                                                Err(e) => {
                                                                    wikilink_error.set(Some(format_server_fn_error(&e.to_string())));
                                                                }
                                                            }
                                                        });
                                                    }
                                                    Some(MdClickHit::Mention(name, rect)) => {
                                                        let x = rect.left();
                                                        let y = rect.bottom() + 4.0;
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(Some(contact)) = resolve_contact_mention(name).await {
                                                                mention_popover.set(Some((contact, x, y)));
                                                            }
                                                        });
                                                    }
                                                    None => {}
                                                }
                                            }
                                        }
                                    />
                                    }.into_any()
                                }
                                Err(e) => view! { <p class="focus-empty">"Could not render: "{e.to_string()}</p> }.into_any(),
                            }
                        })}
                    </Suspense>
                </div>
            }.into_any()
        }}

        {move || mention_popover.get().map(|(contact, x, y)| view! {
            <MentionPopover
                contact=contact
                x=x
                y=y
                on_close=move || mention_popover.set(None)
            />
        })}

        // "Pull out into Note" (backlog #122).
        {move || {
            if !show_pull_out.get() {
                return ().into_any();
            }
            let uid_str = current_uid();
            let source_id = md_source_id_i64(&uid_str).unwrap_or(0);
            let rel_path = md_rel_path(&uid_str).to_string();
            if source_id == 0 || rel_path.is_empty() {
                return ().into_any();
            }
            let on_close_inner = on_close;
            view! {
                <PullOutNoteModal
                    source_id=source_id
                    rel_path=rel_path
                    on_close=move || show_pull_out.set(false)
                    on_refresh=Callback::new(move |_note_id: i64| {
                        events_refresh.update(|n| *n += 1);
                        show_pull_out.set(false);
                        on_close_inner.with_value(|f| f());
                    })
                />
            }.into_any()
        }}

        <div class="fp-footer">
            {move || open_error.get().map(|e| view! {
                <p class="form-error fp-footer-error">{e}</p>
            })}
            <div class="fp-footer-actions">
            <button class="btn-secondary" on:click=move |_| on_close.with_value(|f| f())>"Close"</button>
            <button
                class="btn-secondary"
                on:click=move |_| show_pull_out.set(true)
            >
                "Pull out into Note"
            </button>
            <button
                class="btn-secondary"
                title="Edit tidl's copy directly in the browser"
                on:click=move |_| show_editor.set(true)
            >
                "Edit in browser"
            </button>
            {move || {
                let uid_e = current_uid();
                match current_target() {
                    OpenTarget::LocalClone { abs_path } => {
                        let prefs = local_editor_ctx
                            .map(|c| c.prefs.get())
                            .unwrap_or_default();
                        let has_template = prefs.editor_id != "custom"
                            || !prefs.template.trim().is_empty();
                        if !has_template {
                            return view! {
                                <button
                                    class="btn-secondary"
                                    disabled=true
                                    title="This editor has no link template. Set one under Settings → Editors → Advanced, or use Edit in browser (the path can also be copied above)."
                                >
                                    "Open in editor"
                                </button>
                                <button
                                    class="btn-secondary-small"
                                    on:click=move |_| {
                                        on_close.with_value(|f| f());
                                        if let Some(settings) = open_settings {
                                            settings.0.set(Some("editors"));
                                        }
                                    }
                                >
                                    "Configure editor…"
                                </button>
                            }.into_any();
                        }
                        let uri = build_editor_uri(
                            &prefs.editor_id,
                            &prefs.template,
                            &abs_path,
                            current_line(),
                        );
                        let label = editor_label(&prefs.editor_id);
                        let title =
                            format!("Open {abs_path} in {label} on this device ({uri})");
                        view! {
                            // A real anchor, not `window.location = …`: Firefox/
                            // LibreWolf only hand external URL schemes to the OS
                            // from a link navigation reliably.
                            <a class="btn-secondary" href=uri title=title>
                                {format!("Open in {label} →")}
                            </a>
                        }.into_any()
                    }
                    OpenTarget::HostCopy => {
                        let prefs = local_editor_ctx
                            .map(|c| c.prefs.get_untracked())
                            .unwrap_or_default();
                        view! {
                            <button
                                class="btn-secondary"
                                title="Open this folder in your editor on the tidl host"
                                on:click=move |_| {
                                    open_error.set(None);
                                    let uid_e = uid_e.clone();
                                    let editor_id = prefs.editor_id.clone();
                                    let server_cmd = prefs.server_cmd.clone();
                                    leptos::task::spawn_local(async move {
                                        if let Err(e) =
                                            open_in_editor(uid_e, editor_id, server_cmd).await
                                        {
                                            open_error.set(Some(format_server_fn_error(&e.to_string())));
                                        }
                                    });
                                }
                            >
                                "Open on tidl host →"
                            </button>
                        }.into_any()
                    }
                    OpenTarget::ConfigureClone => view! {
                        <button
                            class="btn-secondary"
                            on:click=move |_| {
                                let existing = current_source()
                                    .and_then(|s| s.url)
                                    .map(|u| normalize_repo_key(&u))
                                    .and_then(|k| {
                                        local_editor_ctx.map(|c| {
                                            c.prefs
                                                .get_untracked()
                                                .git_roots
                                                .get(&k)
                                                .cloned()
                                                .unwrap_or_default()
                                        })
                                    })
                                    .unwrap_or_default();
                                clone_path.set(existing);
                                clone_error.set(None);
                                show_clone_setup.set(true);
                            }
                        >
                            "Set up clone…"
                        </button>
                    }.into_any(),
                    OpenTarget::Unavailable => view! {
                        <button
                            class="btn-secondary"
                            disabled=true
                            title="This folder lives on the tidl host. Browse tidl from that machine to open it in an editor, or use Edit in browser."
                        >
                            "Open on tidl host"
                        </button>
                    }.into_any(),
                }
            }}
            </div>
        </div>

        </div>  // end preview shell (display:contents / display:none toggle)
    }
}

// ── Variant C: Note "go to source" ────────────────────────────────────────────

/// "Go to source" target for note-origin Task/Await items in Focus, Tasks,
/// and Await lists. Reuses the Notes tab's own `NoteEditorModal` instead of
/// building a separate read-only preview — notes (unlike markdown files)
/// are natively editable in-app, so opening the real editor is both simpler
/// and more useful than a preview-only variant would be.
#[component]
pub(crate) fn NoteGotoModal(
    note_id: i64,
    /// The 1-based line and 0-based ordinal of the inline token to scroll to
    /// and flash on the Notes tab (backlog #134). `None` flashes the note card.
    token: Option<(u32, u32)>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    use leptos::prelude::StoredValue;
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    // Used only by the "note not found" fallback backdrop below; the resolved
    // `NoteEditorModal` registers its own id.
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.get_value()());

    // Full-page navigation (not client-side routing) so NotesView can pick up
    // `?highlight=` on mount to unhide the collection, scroll to the note, and
    // pulse its outline (backlog #63). When the originating item addressed a
    // specific token, `line`/`ordinal` ride along so NotesView pulses that chip
    // instead of the whole card (backlog #134).
    let show_in_collection = Callback::new(move |_: ()| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(win) = web_sys::window() {
                let mut url = format!("/notes?highlight={note_id}");
                if let Some((line, ordinal)) = token {
                    url.push_str(&format!("&line={line}&ordinal={ordinal}"));
                }
                let _ = win.location().set_href(&url);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = token;
    });

    view! {
        <Transition fallback=|| ().into_any()>
            {move || Suspend::new(async move {
                let notes = list_notes().await.unwrap_or_default();
                let collections = list_note_collections().await.unwrap_or_default();
                match notes.into_iter().find(|n| n.id == note_id) {
                    Some(note) => view! {
                        <NoteEditorModal
                            note=Some(note)
                            collections=collections
                            default_collection_id=None
                            on_close=move || on_close.with_value(|f| f())
                            on_refresh=on_refresh
                            on_show_in_collection=show_in_collection
                        />
                    }.into_any(),
                    None => {
                        let backdrop_mousedown = RwSignal::new(false);
                        view! {
                        <div class="dialog-backdrop"
                            data-esc-id=esc_id.to_string()
                            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
                        >
                            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                                <div class="dialog-header">
                                    <h2 class="dialog-title">"Note not found"</h2>
                                    <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                                </div>
                                <p class="dialog-hint">"This note may have been deleted or you no longer have access to it."</p>
                            </div>
                        </div>
                    }}.into_any(),
                }
            })}
        </Transition>
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CurvePoint, Event, PriorityCurves, Task};

    // ── encode_query_value ─────────────────────────────────────────────────────

    #[test]
    fn encode_query_value_escapes_uid_chars() {
        assert_eq!(
            encode_query_value("event 1@example.com"),
            "event%201%40example.com"
        );
        assert_eq!(encode_query_value("safe-._~"), "safe-._~");
        assert_eq!(encode_query_value("a/b:c"), "a%2Fb%3Ac");
    }

    // ── eval_curve ─────────────────────────────────────────────────────────────

    #[test]
    fn eval_curve_interpolates_linearly() {
        let pts = [
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 10.0, y: 100.0 },
        ];
        assert_eq!(eval_curve(&pts, 5.0), 50);
        assert_eq!(eval_curve(&pts, 2.5), 25);
    }

    #[test]
    fn eval_curve_flat_extrapolates_past_ends() {
        let pts = [
            CurvePoint { x: 0.0, y: 20.0 },
            CurvePoint { x: 10.0, y: 80.0 },
        ];
        assert_eq!(eval_curve(&pts, -50.0), 20);
        assert_eq!(eval_curve(&pts, 50.0), 80);
    }

    #[test]
    fn eval_curve_clamps_out_of_range_y() {
        let pts = [
            CurvePoint { x: 0.0, y: -10.0 },
            CurvePoint { x: 10.0, y: 150.0 },
        ];
        assert_eq!(eval_curve(&pts, 0.0), 0);
        assert_eq!(eval_curve(&pts, 10.0), 100);
    }

    #[test]
    fn eval_curve_handles_more_than_two_points() {
        let pts = [
            CurvePoint { x: 0.0, y: 90.0 },
            CurvePoint { x: 5.0, y: 50.0 },
            CurvePoint { x: 10.0, y: 20.0 },
        ];
        assert_eq!(eval_curve(&pts, 0.0), 90);
        assert_eq!(eval_curve(&pts, 5.0), 50);
        assert_eq!(eval_curve(&pts, 7.5), 35);
        assert_eq!(eval_curve(&pts, 10.0), 20);
    }

    // ── Suppression key helpers ──────────────────────────────────────────────

    #[test]
    fn series_key_is_the_bare_uid() {
        assert_eq!(series_key("evt-123"), "evt-123");
    }

    #[test]
    fn occurrence_key_joins_uid_and_occurrence_start() {
        assert_eq!(
            occurrence_key("evt-123", "20260615T090000"),
            "evt-123|20260615T090000"
        );
    }

    // Helper: build a minimal Event with a given start string.
    fn ev(start: &str, all_day: bool) -> Event {
        Event {
            id: "uid".into(),
            summary: "Test".into(),
            start: start.into(),
            end: start.into(),
            description: None,
            priority: None,
            alarms: vec![],
            calendar: String::new(),
            calendar_color: None,
            all_day,
            rrule: None,
            occurrence_start: None,
            is_exception: false,
            start_epoch_ms: 0,
            owner_username: String::new(),
            shared_permission: None,
        }
    }

    // Helper: build a minimal Task.
    fn tk(priority: u32, due: Option<&str>, source: &str, id: &str) -> Task {
        Task {
            id: id.into(),
            summary: "Test".into(),
            priority,
            due: due.map(|s| s.into()),
            completed: false,
            source: source.into(),
            calendar: String::new(),
            source_file: None,
            source_line: None,
            topic: None,
            collection_id: None,
            collection_name: None,
            collection_color: None,
            note_id: None,
            note_priority: None,
            note_labels: vec![],
        }
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A stable, hardcoded curve fixture for tests that assert exact
    /// interpolated scores — deliberately independent of
    /// `PriorityCurves::default()`, which is now a user's own live-tuned
    /// curve (backlog #68) rather than a fixed constant. Coupling
    /// score-math tests to `default()` made them break every time a
    /// default curve was retuned even though the interpolation/
    /// extrapolation logic under test hadn't changed — see the 2026-07-30
    /// investigation. These are the old hardcoded tier values, kept only
    /// as a fixed point for exercising `eval_curve`'s math.
    fn default_curves() -> PriorityCurves {
        PriorityCurves {
            calendar_time_left: vec![
                CurvePoint { x: -1.0, y: 0.0 },
                CurvePoint { x: 0.0, y: 99.0 },
                CurvePoint { x: 0.25, y: 99.0 },
                CurvePoint { x: 1.0, y: 95.0 },
                CurvePoint { x: 3.0, y: 90.0 },
                CurvePoint { x: 9.0, y: 80.0 },
                CurvePoint { x: 24.0, y: 50.0 },
                CurvePoint { x: 72.0, y: 38.0 },
                CurvePoint { x: 168.0, y: 25.0 },
            ],
            due_date_time_left: vec![
                CurvePoint { x: -1.0, y: 88.0 },
                CurvePoint { x: 0.0, y: 72.0 },
                CurvePoint { x: 1.0, y: 52.0 },
                CurvePoint { x: 2.0, y: 35.0 },
                CurvePoint { x: 3.0, y: 35.0 },
                CurvePoint { x: 4.0, y: 18.0 },
            ],
            source_priority: vec![
                CurvePoint { x: 0.0, y: 90.0 },
                CurvePoint { x: 24.0, y: 90.0 },
                CurvePoint { x: 25.0, y: 70.0 },
                CurvePoint { x: 49.0, y: 70.0 },
                CurvePoint { x: 50.0, y: 50.0 },
                CurvePoint { x: 74.0, y: 50.0 },
                CurvePoint { x: 75.0, y: 20.0 },
                CurvePoint { x: 100.0, y: 20.0 },
            ],
            calendar_event_priority: vec![
                CurvePoint { x: 1.0, y: 100.0 },
                CurvePoint { x: 100.0, y: 20.0 },
            ],
        }
    }

    // ── event_urgency ──────────────────────────────────────────────────────────

    /// Builds an epoch-ms timestamp for a `"YYYY-MM-DD HH:MM"` or
    /// `"YYYY-MM-DD"` string, treating it as UTC — self-consistent for
    /// tests since only relative differences between fixture timestamps
    /// matter, never real wall-clock time.
    fn epoch_ms(s: &str) -> i64 {
        use chrono::NaiveDateTime;
        let dt = if s.len() > 10 {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
        } else {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        };
        dt.and_utc().timestamp_millis()
    }

    fn ev_at(start: &str, start_epoch_ms: i64) -> Event {
        let mut e = ev(start, start.len() <= 10);
        e.start_epoch_ms = start_epoch_ms;
        e
    }

    #[test]
    fn event_urgency_far_past_is_zero() {
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 00:00");
        let e = ev_at("2026-05-01 10:00", epoch_ms("2026-05-01 10:00"));
        assert_eq!(event_urgency(&e, "2026-05-16 00:00", now, &curves), 0);
    }

    #[test]
    fn event_priority_unset_leaves_time_score_unchanged() {
        let curves = default_curves();
        let start = epoch_ms("2026-05-16 09:00");
        let e = ev_at("2026-05-16 09:00", start);
        assert_eq!(e.priority, None);
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", start, &curves), 99);
    }

    #[test]
    fn event_priority_highest_does_not_attenuate() {
        let curves = default_curves();
        let start = epoch_ms("2026-05-16 09:00");
        let mut e = ev_at("2026-05-16 09:00", start);
        e.priority = Some(1);
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", start, &curves), 99);
    }

    #[test]
    fn event_priority_caps_imminent_event() {
        let curves = default_curves();
        let start = epoch_ms("2026-05-16 09:00");
        let mut e = ev_at("2026-05-16 09:00", start);
        // Fixed test curve: p=50 interpolates to 60, p=100 to 20.
        e.priority = Some(50);
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", start, &curves), 60);
        e.priority = Some(100);
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", start, &curves), 20);
    }

    #[test]
    fn event_priority_caps_in_progress_event() {
        let curves = default_curves();
        let mut e = ev_at("2026-05-16 14:00", epoch_ms("2026-05-16 14:00"));
        e.end = "2026-05-16 15:00".into();
        e.priority = Some(100);
        let now = epoch_ms("2026-05-16 14:30");
        assert_eq!(event_urgency(&e, "2026-05-16 14:30", now, &curves), 20);
    }

    #[test]
    fn event_priority_ceiling_never_raises_low_time_score() {
        let curves = default_curves();
        // ~24h out scores 50; a p=50 cap of 60 must leave it at 50.
        let start = epoch_ms("2026-05-17 09:00");
        let mut e = ev_at("2026-05-17 09:00", start);
        e.priority = Some(50);
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 09:00",
                epoch_ms("2026-05-16 09:00"),
                &curves
            ),
            50
        );
    }

    #[test]
    fn event_priority_uses_custom_curve() {
        let mut curves = default_curves();
        curves.calendar_event_priority = vec![
            CurvePoint { x: 1.0, y: 40.0 },
            CurvePoint { x: 100.0, y: 40.0 },
        ];
        let start = epoch_ms("2026-05-16 09:00");
        let mut e = ev_at("2026-05-16 09:00", start);
        e.priority = Some(1);
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", start, &curves), 40);
    }

    #[test]
    fn event_urgency_starting_now_is_max() {
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 09:00");
        let e = ev_at("2026-05-16 09:00", epoch_ms("2026-05-16 09:00"));
        assert_eq!(event_urgency(&e, "2026-05-16 09:00", now, &curves), 99);
    }

    #[test]
    fn event_urgency_is_deterministic() {
        // Calling the function twice with identical inputs must yield the
        // same result — guards against accidentally reading the wall clock
        // internally (which caused the SSR/WASM hydration panics in #21).
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 09:00");
        let e = ev_at("2026-05-16 12:00", epoch_ms("2026-05-16 12:00"));
        assert_eq!(
            event_urgency(&e, "2026-05-16 09:00", now, &curves),
            event_urgency(&e, "2026-05-16 09:00", now, &curves)
        );
    }

    #[test]
    fn event_urgency_in_progress_is_max() {
        let curves = default_curves();
        let mut e = ev_at("2026-05-16 14:00", epoch_ms("2026-05-16 14:00"));
        e.end = "2026-05-16 15:00".into();
        // now = 14:30, well after start_epoch_ms — without the in-progress
        // floor this would be a negative (past) hours-until-start.
        let now = epoch_ms("2026-05-16 14:30");
        assert_eq!(event_urgency(&e, "2026-05-16 14:30", now, &curves), 99);
        // Boundaries: exactly at start and exactly at end both count.
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 14:00",
                epoch_ms("2026-05-16 14:00"),
                &curves
            ),
            99
        );
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 15:00",
                epoch_ms("2026-05-16 15:00"),
                &curves
            ),
            99
        );
    }

    #[test]
    fn event_urgency_proximity_tiers_match_default_curve() {
        let curves = default_curves();
        let start = epoch_ms("2026-05-16 14:00");
        let e = ev_at("2026-05-16 14:00", start);
        // 15 min out → default curve's (0.25h, 99) point.
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 13:45",
                epoch_ms("2026-05-16 13:45"),
                &curves
            ),
            99
        );
        // 1h out → (1h, 95).
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 13:00",
                epoch_ms("2026-05-16 13:00"),
                &curves
            ),
            95
        );
        // 3h out → (3h, 90).
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 11:00",
                epoch_ms("2026-05-16 11:00"),
                &curves
            ),
            90
        );
        // 9h out → (9h, 80) — "later today", the approximated hour-of-day bucket.
        assert_eq!(
            event_urgency(
                &e,
                "2026-05-16 05:00",
                epoch_ms("2026-05-16 05:00"),
                &curves
            ),
            80
        );
    }

    #[test]
    fn event_urgency_already_ended_falls_to_past_tail() {
        // Once the event's end time has passed, hours-until-start is
        // negative and there's no more in-progress floor — it falls onto
        // the curve's flat past tail (0), same as any past event.
        let curves = default_curves();
        let mut e = ev_at("2026-05-16 08:00", epoch_ms("2026-05-16 08:00"));
        e.end = "2026-05-16 08:30".into();
        let now = epoch_ms("2026-05-16 20:00");
        assert_eq!(event_urgency(&e, "2026-05-16 20:00", now, &curves), 0);
    }

    #[test]
    fn event_urgency_tomorrow() {
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 10:00");
        let e = ev_at("2026-05-17 10:00", epoch_ms("2026-05-17 10:00"));
        // Exactly 24h out → default curve's (24h, 50) point.
        assert_eq!(event_urgency(&e, "2026-05-16 10:00", now, &curves), 50);
    }

    #[test]
    fn event_urgency_three_days() {
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 10:00");
        let e = ev_at("2026-05-19 10:00", epoch_ms("2026-05-19 10:00"));
        // Exactly 72h out → default curve's (72h, 38) point.
        assert_eq!(event_urgency(&e, "2026-05-16 10:00", now, &curves), 38);
    }

    #[test]
    fn event_urgency_far_future() {
        let curves = default_curves();
        let now = epoch_ms("2026-05-16 10:00");
        // 14 days out — beyond the curve's last point (168h/7d, 25) → flat-extrapolates to 25.
        let e = ev_at("2026-05-30 10:00", epoch_ms("2026-05-30 10:00"));
        assert_eq!(event_urgency(&e, "2026-05-16 10:00", now, &curves), 25);
    }

    // ── task_urgency ───────────────────────────────────────────────────────────

    #[test]
    fn task_urgency_urgent_priority() {
        let today = date("2026-05-16");
        let curves = default_curves();
        // `task.priority` is now the canonical 1-100 scale (1 = highest,
        // see backlog #70 / 2026-07-30 investigation) fed straight into
        // the curve with no intermediate scaling — 0 (a stray legacy/
        // out-of-range value) clamps to 1. Priorities 1-24 all land within
        // this fixture's first plateau [0,24].
        for p in [0u32, 1, 10, 24] {
            assert_eq!(
                task_urgency(&tk(p, None, "caldav", "u"), today, &curves),
                90,
                "priority {p}"
            );
        }
    }

    #[test]
    fn task_urgency_max_priority_shows_as_urgent_label() {
        // Regression: a #task/p1 (or note-priority-1) task with no due date
        // must actually render the URGENT label/color, not just a high
        // number that happens to fall short of the 85 threshold.
        let today = date("2026-05-16");
        let curves = default_curves();
        let score = task_urgency(&tk(1, None, "caldav", "u"), today, &curves);
        assert_eq!(urgency_label(score), "URGENT");
        assert_eq!(urgency_color(score), "#d93025");
    }

    #[test]
    fn task_urgency_soon_priority() {
        let today = date("2026-05-16");
        let curves = default_curves();
        for p in [25u32, 35, 49] {
            assert_eq!(
                task_urgency(&tk(p, None, "caldav", "u"), today, &curves),
                70,
                "priority {p}"
            );
        }
    }

    #[test]
    fn task_urgency_priority_tiers_land_in_distinct_labels() {
        // Regression: each of the four priority tiers must produce a
        // distinct urgency_label -- previously only three buckets existed
        // for four labels, so fixing one gap always opened another.
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(
            urgency_label(task_urgency(&tk(10, None, "caldav", "u"), today, &curves)),
            "URGENT"
        );
        assert_eq!(
            urgency_label(task_urgency(&tk(30, None, "caldav", "u"), today, &curves)),
            "SOON"
        );
        assert_eq!(
            urgency_label(task_urgency(&tk(60, None, "caldav", "u"), today, &curves)),
            "UPCOMING"
        );
        assert_eq!(
            urgency_label(task_urgency(&tk(90, None, "caldav", "u"), today, &curves)),
            "LATER"
        );
    }

    #[test]
    fn task_urgency_upcoming_priority() {
        let today = date("2026-05-16");
        let curves = default_curves();
        for p in [50u32, 60, 74] {
            assert_eq!(
                task_urgency(&tk(p, None, "caldav", "u"), today, &curves),
                50,
                "priority {p}"
            );
        }
    }

    #[test]
    fn task_urgency_later_priority() {
        let today = date("2026-05-16");
        let curves = default_curves();
        for p in [75u32, 90, 100, 1000] {
            assert_eq!(
                task_urgency(&tk(p, None, "caldav", "u"), today, &curves),
                20,
                "priority {p}"
            );
        }
    }

    #[test]
    fn task_urgency_due_date_boosts_low_priority() {
        // A #task/<date>/<slug> tag (no priority) due tomorrow must outrank
        // its own bare low-priority score of 20, matching AWAIT's tomorrow score.
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(
            task_urgency(
                &tk(1000, Some("2026-05-17"), "markdown", "u"),
                today,
                &curves
            ),
            52
        );
    }

    #[test]
    fn task_urgency_overdue_due_date_is_max() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(
            task_urgency(
                &tk(1000, Some("2026-01-01"), "markdown", "u"),
                today,
                &curves
            ),
            88
        );
    }

    #[test]
    fn task_urgency_high_priority_beats_far_due_date() {
        // Priority still wins when the due date is distant.
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(
            task_urgency(&tk(1, Some("2027-01-01"), "markdown", "u"), today, &curves),
            90
        );
    }

    // ── urgency explanations (backlog #158) ───────────────────────────────────

    #[test]
    fn task_explanation_priority_only() {
        let today = date("2026-05-16");
        let curves = default_curves();
        let why = task_urgency_explanation(&tk(1, None, "caldav", "u"), today, &curves);
        assert_eq!(why.score, 90);
        assert_eq!(why.factors.len(), 1);
        assert_eq!(why.factors[0].label, "Priority 1");
        assert_eq!(why.factors[0].score, 90);
        assert!(why.factors[0].used);
        assert_eq!(why.note, Some("No due date"));
    }

    #[test]
    fn task_explanation_due_date_wins() {
        let today = date("2026-05-16");
        let curves = default_curves();
        let why = task_urgency_explanation(
            &tk(1000, Some("2026-05-17"), "markdown", "u"),
            today,
            &curves,
        );
        assert_eq!(why.score, 52);
        assert_eq!(why.factors.len(), 2);
        // Priority clamps to the canonical 1-100 scale.
        assert_eq!(why.factors[0].label, "Priority 100");
        assert_eq!(why.factors[0].score, 20);
        assert!(!why.factors[0].used);
        assert_eq!(why.factors[1].label, "Due tomorrow");
        assert_eq!(why.factors[1].score, 52);
        assert!(why.factors[1].used);
        assert_eq!(why.note, Some("Higher of the two inputs"));
    }

    #[test]
    fn task_explanation_priority_wins() {
        let today = date("2026-05-16");
        let curves = default_curves();
        let why =
            task_urgency_explanation(&tk(1, Some("2027-01-01"), "markdown", "u"), today, &curves);
        assert_eq!(why.score, 90);
        assert!(why.factors[0].used);
        assert!(!why.factors[1].used);
        assert!(why.factors[1].label.starts_with("Due in "));
    }

    #[test]
    fn await_explanation_has_one_factor() {
        let today = date("2026-05-16");
        let curves = default_curves();
        let why = await_urgency_explanation("2026-05-18", today, &curves);
        assert_eq!(why.score, 35);
        assert_eq!(why.factors.len(), 1);
        assert_eq!(why.factors[0].label, "Due in 2 days");
        assert!(why.factors[0].used);
        assert_eq!(why.note, None);
    }

    #[test]
    fn event_explanation_reports_in_progress_floor() {
        let curves = default_curves();
        let now = "2026-05-16 09:30";
        let now_ms = epoch_ms(now);
        let mut e = ev_at("2026-05-16 09:00", epoch_ms("2026-05-16 09:00"));
        e.end = "2026-05-16 10:00".into();
        let why = event_urgency_explanation(&e, now, now_ms, &curves);
        assert_eq!(why.score, 99);
        assert_eq!(why.factors[0].label, "In progress (0h floor)");
        assert!(why.factors[0].used);
    }

    #[test]
    fn event_explanation_formats_upcoming_time() {
        let curves = default_curves();
        let now = "2026-05-16 09:00";
        let now_ms = epoch_ms(now);
        let e = ev_at("2026-05-16 09:45", epoch_ms("2026-05-16 09:45"));
        let why = event_urgency_explanation(&e, now, now_ms, &curves);
        assert_eq!(why.factors[0].label, "Starts in 45 min");
    }

    #[test]
    fn event_explanation_priority_cap_binds() {
        let curves = default_curves();
        let now = "2026-05-16 09:00";
        let now_ms = epoch_ms(now);
        let mut e = ev_at("2026-05-16 10:00", epoch_ms("2026-05-16 10:00"));
        e.priority = Some(100);
        let why = event_urgency_explanation(&e, now, now_ms, &curves);
        assert_eq!(why.score, 20);
        assert_eq!(why.factors.len(), 2);
        assert!(!why.factors[0].used);
        assert!(why.factors[1].used);
        assert_eq!(why.factors[1].label, "Priority cap 100");
        assert_eq!(why.note, Some("Capped by event priority"));
    }

    #[test]
    fn event_explanation_priority_cap_not_binding() {
        let curves = default_curves();
        let now = "2026-05-16 09:00";
        let now_ms = epoch_ms(now);
        let mut e = ev_at("2026-05-16 10:00", epoch_ms("2026-05-16 10:00"));
        e.priority = Some(1);
        let why = event_urgency_explanation(&e, now, now_ms, &curves);
        assert_eq!(why.score, 95);
        assert!(why.factors[0].used);
        assert!(!why.factors[1].used);
        assert_eq!(why.note, Some("Priority cap not binding"));
    }

    // ── pending_urgency ────────────────────────────────────────────────────────

    #[test]
    fn pending_urgency_overdue() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(pending_urgency("2026-05-15", today, &curves), 88);
        assert_eq!(pending_urgency("2026-01-01", today, &curves), 88);
    }

    #[test]
    fn pending_urgency_due_today() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(pending_urgency("2026-05-16", today, &curves), 72);
    }

    #[test]
    fn pending_urgency_due_tomorrow() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(pending_urgency("2026-05-17", today, &curves), 52);
    }

    #[test]
    fn pending_urgency_due_soon() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(pending_urgency("2026-05-18", today, &curves), 35);
        assert_eq!(pending_urgency("2026-05-19", today, &curves), 35);
    }

    #[test]
    fn pending_urgency_later() {
        let today = date("2026-05-16");
        let curves = default_curves();
        assert_eq!(pending_urgency("2026-05-23", today, &curves), 18);
    }

    // ── urgency_label ──────────────────────────────────────────────────────────

    #[test]
    fn urgency_label_thresholds() {
        assert_eq!(urgency_label(85), "URGENT");
        assert_eq!(urgency_label(88), "URGENT");
        assert_eq!(urgency_label(84), "SOON");
        assert_eq!(urgency_label(65), "SOON");
        assert_eq!(urgency_label(64), "UPCOMING");
        assert_eq!(urgency_label(40), "UPCOMING");
        assert_eq!(urgency_label(39), "LATER");
        assert_eq!(urgency_label(0), "LATER");
    }

    // ── include_item ───────────────────────────────────────────────────────────

    #[test]
    fn include_item_event_within_horizon() {
        let today = date("2026-05-16");
        let e = ev("2026-05-17 10:00", false);
        assert!(include_item(&ItemKind::Event(e.clone()), 7, today, ""));
        assert!(include_item(&ItemKind::Event(e), 3, today, ""));
    }

    #[test]
    fn include_item_event_past_excluded() {
        let today = date("2026-05-16");
        let e = ev("2026-05-15 10:00", false);
        assert!(!include_item(&ItemKind::Event(e), 7, today, ""));
    }

    #[test]
    fn include_item_event_beyond_horizon_excluded() {
        let today = date("2026-05-16");
        let e = ev("2026-05-24 10:00", false); // 8 days out
        assert!(!include_item(&ItemKind::Event(e), 7, today, ""));
    }

    #[test]
    fn include_item_task_horizon_filtering() {
        let today = date("2026-05-16");
        // priority 0 = highest (e.g. `#task/p0/<slug>`) must be treated the
        // same as 1 — both belong to the "high" tier.
        let highest = tk(0, None, "caldav", "u");
        let high = tk(1, None, "caldav", "u"); // priority 1 = high
        let med = tk(5, None, "caldav", "u"); // priority 5 = medium
        let low = tk(9, None, "caldav", "u"); // priority 9 = low

        // horizon=1: only high-priority tasks
        assert!(include_item(&ItemKind::Task(highest.clone()), 1, today, ""));
        assert!(include_item(&ItemKind::Task(high.clone()), 1, today, ""));
        assert!(!include_item(&ItemKind::Task(med.clone()), 1, today, ""));
        assert!(!include_item(&ItemKind::Task(low.clone()), 1, today, ""));

        // horizon=3: high and medium
        assert!(include_item(&ItemKind::Task(highest), 3, today, ""));
        assert!(include_item(&ItemKind::Task(high.clone()), 3, today, ""));
        assert!(include_item(&ItemKind::Task(med.clone()), 3, today, ""));
        assert!(!include_item(&ItemKind::Task(low.clone()), 3, today, ""));

        // horizon=7: everything
        assert!(include_item(&ItemKind::Task(high), 7, today, ""));
        assert!(include_item(&ItemKind::Task(med), 7, today, ""));
        assert!(include_item(&ItemKind::Task(low), 7, today, ""));
    }

    // ── md_source_id ───────────────────────────────────────────────────────────

    #[test]
    fn md_source_id_normal() {
        assert_eq!(md_source_id("md:42:notes/todo.md:15:0"), "42");
        assert_eq!(md_source_id("md:1:path/to/file.md:100:2"), "1");
    }

    #[test]
    fn md_source_id_no_prefix_returns_empty() {
        assert_eq!(md_source_id("caldav:uid-1234"), "");
        assert_eq!(md_source_id(""), "");
    }

    #[test]
    fn md_source_id_just_prefix_returns_empty() {
        assert_eq!(md_source_id("md:"), "");
    }

    #[test]
    fn md_ordinal_reads_the_line_local_index() {
        assert_eq!(md_ordinal("md:1:path/to/file.md:100:2"), 2);
        assert_eq!(md_ordinal("md:1:path/to/file.md:100:0"), 0);
        // Malformed / old-format uid without an ordinal defaults to 0.
        assert_eq!(md_ordinal("caldav:uid-1234"), 0);
        assert_eq!(md_ordinal("md:1:file.md:3"), 0);
    }

    #[test]
    fn note_token_locator_reads_line_and_ordinal() {
        assert_eq!(note_token_locator("note:42:7:1"), Some((7, 1)));
        assert_eq!(note_token_locator("note:42:7:0"), Some((7, 0)));
        // Missing ordinal defaults to 0 (the first token on the line).
        assert_eq!(note_token_locator("note:42:7"), Some((7, 0)));
        // Not a note uid, or no usable line → no location, card fallback.
        assert_eq!(note_token_locator("caldav:uid-1234"), None);
        assert_eq!(note_token_locator("md:1:file.md:3:0"), None);
        assert_eq!(note_token_locator("note:42:x:0"), None);
    }

    // ── note_labels_hidden ─────────────────────────────────────────────────────

    fn lbl(id: i64) -> NoteLabelInfo {
        NoteLabelInfo {
            id,
            name: format!("l{id}"),
            color: "c".into(),
        }
    }

    #[test]
    fn note_labels_hidden_only_when_every_label_is_off() {
        let labels = vec![lbl(1), lbl(2)];
        // One label toggled off, the other on → item still shows.
        let hidden = HashSet::from(["1".to_string()]);
        assert!(!note_labels_hidden(&labels, &hidden));
        // Neither toggled off → shows.
        assert!(!note_labels_hidden(&labels, &HashSet::new()));
        // Every label toggled off → hidden.
        let hidden = HashSet::from(["1".to_string(), "2".to_string()]);
        assert!(note_labels_hidden(&labels, &hidden));
    }

    #[test]
    fn note_labels_hidden_ignores_unlabelled_items() {
        // An item with no labels is never filtered by the label toggles.
        let hidden = HashSet::from(["1".to_string()]);
        assert!(!note_labels_hidden(&[], &hidden));
    }
}

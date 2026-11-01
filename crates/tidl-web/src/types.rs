//! Shared data types exchanged between the WASM frontend and the SSR server.
//!
//! These types are serialised over the Leptos server-function boundary (JSON).
//! They are defined here — separate from the server functions in `api.rs` —
//! so that the WASM build can import them without dragging in any SSR code.

use serde::{Deserialize, Serialize};

// ── Calendars ─────────────────────────────────────────────────────────────────

/// A user's calendar as returned by `get_calendars()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CalendarInfo {
    /// Internal database row ID — zero when the value was not fetched.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub color: String,
    /// True for webcal subscriptions (read-only feed).
    #[serde(default)]
    pub read_only: bool,
    /// Webcal subscription URL if this is a subscribed feed, otherwise `None`.
    #[serde(default)]
    pub sync_url: Option<String>,
    /// Error message from the last failed sync, if any.
    #[serde(default)]
    pub sync_error: Option<String>,
    /// ISO-8601 timestamp of the last successful sync, if any.
    #[serde(default)]
    pub last_synced_at: Option<String>,
    /// True if this calendar has at least one active grantee (backlog #65).
    #[serde(default)]
    pub shared: bool,
}

// ── Calendar events ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub summary: String,
    /// Timed events: "YYYY-MM-DD HH:MM".  All-day events: "YYYY-MM-DD".
    pub start: String,
    /// Timed events: "YYYY-MM-DD HH:MM".  All-day events: "YYYY-MM-DD" (inclusive last day).
    pub end: String,
    pub description: Option<String>,
    /// Canonical 1-100 priority (1 = highest; backlog #156), or `None` for
    /// "no priority". Only attenuates Focus urgency — it never boosts it.
    /// Persisted in `events.priority` and projected onto RFC 5545's 1-9
    /// `PRIORITY` in `ical_data` for external CalDAV clients.
    #[serde(default)]
    pub priority: Option<u32>,
    /// Parsed alarms — populated server-side from `ical_data`.
    #[serde(default)]
    pub alarms: Vec<Alarm>,
    /// Slug name of the calendar this event belongs to (e.g. "personal").
    #[serde(default)]
    pub calendar: String,
    /// Hex color of the calendar (e.g. "#3b82f6"), for rendering.
    #[serde(default)]
    pub calendar_color: Option<String>,
    /// True when the event is an all-day or multi-day DATE event (no time component).
    #[serde(default)]
    pub all_day: bool,
    /// RRULE string from the master VEVENT (e.g. "FREQ=WEEKLY;BYDAY=MO,WE").
    /// Present on every occurrence emitted by expand_rrule; None for
    /// non-recurring events and for exception rows.
    #[serde(default)]
    pub rrule: Option<String>,
    /// Canonical DTSTART of this occurrence slot in the series, in iCal
    /// compact format: "YYYYMMDD" (all-day) or "YYYYMMDDTHHmmSS" (timed).
    /// Used as RECURRENCE-ID when saving a single-occurrence exception.
    #[serde(default)]
    pub occurrence_start: Option<String>,
    /// True when this row is an overriding exception VEVENT (uid contains "_R").
    #[serde(default)]
    pub is_exception: bool,
    /// UTC epoch milliseconds for the event start — computed server-side using
    /// the server's local timezone.  Used by the alarm scheduler to avoid
    /// browser timezone spoofing (e.g. LibreWolf's resistFingerprinting).
    /// Zero when absent (old cached data or test fixtures).
    #[serde(default)]
    pub start_epoch_ms: i64,
    /// Username of the calendar's owner. Equal to the requesting user's own
    /// username for their own events; the actual owner's username for
    /// events pulled in from a calendar shared with them (backlog #41).
    #[serde(default)]
    pub owner_username: String,
    /// `None` for the caller's own events. `Some("read")` / `Some("read-write")`
    /// for events sourced from a calendar shared with the caller — mirrors
    /// `calendar_shares.permission` and gates whether the UI offers editing.
    #[serde(default)]
    pub shared_permission: Option<String>,
}

/// A single alarm/notification on an event.
///
/// This struct is shared between the WASM UI and the SSR server.  It carries
/// enough information for the UI to display and edit, plus a `raw_ical` field
/// that preserves any content we cannot fully represent (enabling lossless
/// round-tripping of alarms created by external CalDAV clients).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Alarm {
    /// ACTION value: "display" | "audio" | "email"
    #[serde(default = "default_action")]
    pub action: String,

    /// Duration amount — only meaningful when `is_raw` is false.
    pub amount: u32,

    /// Duration unit: "minutes" | "hours" | "days" | "weeks"
    pub unit: String,

    /// Optional repetition count (REPEAT property).  Must be Some together
    /// with `repeat_delay_m`.
    pub repeat_count: Option<u32>,

    /// Delay between repetitions in minutes (DURATION property on the alarm).
    pub repeat_delay_m: Option<u32>,

    /// Raw VALARM inner lines (without BEGIN:/END:) for lossless round-tripping
    /// of alarms we cannot fully parse (RELATED=END, absolute triggers, …).
    /// When `Some`, this takes precedence over the structured fields on the
    /// server when rebuilding iCal.
    pub raw_ical: Option<String>,

    /// True when the trigger form is not one we can represent in the UI
    /// (e.g. RELATED=END, absolute DATE-TIME).  The UI should show the alarm
    /// as read-only / "custom" in that case.
    #[serde(default)]
    pub is_raw: bool,
}

fn default_action() -> String {
    "display".to_string()
}

impl Alarm {
    /// Whether firing this alarm should play an audible sound.
    ///
    /// Only `ACTION:AUDIO` alarms make noise (RFC 5545 §3.8.6.1); `DISPLAY`
    /// and `EMAIL` alarms are silent — `DISPLAY` surfaces a notification only,
    /// `EMAIL` is dispatched server-side and has no client-side effect at all.
    pub fn plays_sound(&self) -> bool {
        self.action.eq_ignore_ascii_case("audio")
    }
}

/// Input for creating or updating a calendar event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewEvent {
    pub summary: String,
    /// Timed: "YYYY-MM-DD HH:MM".  All-day: "YYYY-MM-DD".
    pub start: String,
    /// Timed: "YYYY-MM-DD HH:MM".  All-day: "YYYY-MM-DD" (inclusive last day).
    pub end: String,
    #[serde(default)]
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Canonical 1-100 event priority (1 = highest); `None` omits the
    /// `PRIORITY` property and leaves Focus scoring untouched (backlog #156).
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub alarms: Vec<Alarm>,
    /// Calendar slug to store the event in (e.g. "personal"). Defaults to "personal".
    #[serde(default = "default_calendar")]
    pub calendar: String,
    /// RRULE string for recurring events (e.g. "FREQ=WEEKLY;BYDAY=MO,WE,FR").
    /// None means the event does not repeat.
    #[serde(default)]
    pub rrule: Option<String>,
    /// Owner of `calendar`, when creating an event directly on a calendar
    /// shared with the caller at read-write permission. `None` means "my own
    /// calendar" — `create_event` verifies a live read-write share before
    /// honoring any other value. Ignored by `update_event`, which resolves
    /// the write owner from the existing event's uid instead.
    #[serde(default)]
    pub calendar_owner: Option<String>,
}

fn default_calendar() -> String {
    "personal".to_string()
}

// ── Todos ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub summary: String,
    pub priority: u32,
    pub due: Option<String>,
    pub completed: bool,
    /// `"caldav"` or `"markdown"`.  Defaults to `"caldav"` when absent.
    #[serde(default = "default_todo_source")]
    pub source: String,
    /// Calendar slug this task belongs to (e.g. `"personal"`).
    #[serde(default)]
    pub calendar: String,
    /// For markdown tasks: the source file path (relative within the source).
    #[serde(default)]
    pub source_file: Option<String>,
    /// For markdown tasks: the line number (1-indexed).
    #[serde(default)]
    pub source_line: Option<u32>,
    /// For markdown tasks: the topic slug (e.g. `"sales"`).
    #[serde(default)]
    pub topic: Option<String>,
    /// For `source == "note"` tasks: the originating note collection's id
    /// (as a string, to match `HiddenNoteCollectionsCtx`'s key type).
    #[serde(default)]
    pub collection_id: Option<String>,
    /// For `source == "note"` tasks: the originating note collection's name.
    #[serde(default)]
    pub collection_name: Option<String>,
    /// For `source == "note"` tasks: the originating note collection's color.
    #[serde(default)]
    pub collection_color: Option<String>,
    /// For `source == "note"` tasks: the originating note's own id, so
    /// "go to source" can open that exact note rather than just its
    /// collection.
    #[serde(default)]
    pub note_id: Option<i64>,
    /// For `source == "note"` tasks: the originating note's own 0-100
    /// priority (already on the `source_priority` curve's domain), so
    /// urgency scoring can use it directly instead of round-tripping
    /// through the lossy small-int `priority` field above — see
    /// `components::focus::task_urgency`.
    #[serde(default)]
    pub note_priority: Option<u32>,
    /// For `source == "note"` tasks: the originating note's labels (name +
    /// color, backlog #114/#127), so Focus can render label chips on the card
    /// and hide tasks whose note carries a hidden label. Empty for
    /// non-note-origin tasks.
    #[serde(default)]
    pub note_labels: Vec<NoteLabelInfo>,
}

fn default_todo_source() -> String {
    "caldav".to_string()
}

/// A parsed markdown-sourced todo/await UID.
///
/// Format: `md:{source_id}:{rel_path}:{line}:{ordinal}`. `rel_path` may itself
/// contain `:`, so the source id is split off the left and the line/ordinal off
/// the right; everything between is the path. `ordinal` is the 0-based index of
/// the tag among its line's active tags — several tags can share a line, and
/// the ordinal is what makes their uids unique and lets the rendered preview
/// target the right chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownUid<'a> {
    pub source_id: &'a str,
    pub rel_path: &'a str,
    pub line: &'a str,
    pub ordinal: &'a str,
}

impl<'a> MarkdownUid<'a> {
    /// Parse `md:{source_id}:{rel_path}:{line}:{ordinal}`, or `None` when the
    /// uid isn't an `md:` uid / has too few segments.
    pub fn parse(uid: &'a str) -> Option<Self> {
        let rest = uid.strip_prefix("md:")?;
        let (source_id, rest) = rest.split_once(':')?;
        let (rest, ordinal) = rest.rsplit_once(':')?;
        let (rel_path, line) = rest.rsplit_once(':')?;
        Some(Self {
            source_id,
            rel_path,
            line,
            ordinal,
        })
    }

    /// The source id as an integer, when numeric.
    pub fn source_id_i64(&self) -> Option<i64> {
        self.source_id.parse().ok()
    }

    /// The 1-based line as an integer, defaulting to 1 on malformed input.
    pub fn line_u32(&self) -> u32 {
        self.line.parse().unwrap_or(1)
    }

    /// The 0-based ordinal as an integer, defaulting to 0 on malformed input.
    pub fn ordinal_u32(&self) -> u32 {
        self.ordinal.parse().unwrap_or(0)
    }
}

#[cfg(test)]
mod markdown_uid_tests {
    use super::MarkdownUid;

    #[test]
    fn parses_rel_path_containing_colons() {
        let uid = MarkdownUid::parse("md:7:dir:sub/note.md:12:3").unwrap();
        assert_eq!(uid.source_id, "7");
        assert_eq!(uid.rel_path, "dir:sub/note.md");
        assert_eq!(uid.line, "12");
        assert_eq!(uid.ordinal, "3");
        assert_eq!(uid.source_id_i64(), Some(7));
        assert_eq!(uid.line_u32(), 12);
        assert_eq!(uid.ordinal_u32(), 3);
    }

    #[test]
    fn rejects_foreign_and_short_uids() {
        assert!(MarkdownUid::parse("caldav-uid").is_none());
        assert!(MarkdownUid::parse("md:1:file.md:3").is_none());
        assert!(MarkdownUid::parse("md:1:file.md").is_none());
    }
}

#[cfg(test)]
mod todo_origin_tests {
    use super::TodoOrigin;

    #[test]
    fn note_origin_builds_the_token_uid() {
        let origin = TodoOrigin::Note {
            note_id: 42,
            line: 7,
            ordinal: 1,
        };
        assert_eq!(origin.event_uid().as_deref(), Some("note:42:7:1"));
    }

    #[test]
    fn markdown_origin_rebuilds_the_tag_uid_from_the_enclosing_file() {
        // The chip's origin carries the enclosing file's uid plus its own line
        // and ordinal; only source id and rel path come from the file uid.
        let origin = TodoOrigin::Markdown {
            uid: "md:7:dir:sub/note.md:999:4".to_string(),
            line: 12,
            ordinal: 3,
        };
        assert_eq!(
            origin.event_uid().as_deref(),
            Some("md:7:dir:sub/note.md:12:3")
        );
    }

    #[test]
    fn markdown_origin_with_a_foreign_uid_has_no_event_uid() {
        let origin = TodoOrigin::Markdown {
            uid: "not-a-md-uid".to_string(),
            line: 1,
            ordinal: 0,
        };
        assert!(origin.event_uid().is_none());
    }
}

// ── Note collections ──────────────────────────────────────────────────────────

/// A note collection as returned by `list_note_collections()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NoteCollectionInfo {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub shared: bool,
    /// True when the current user owns this collection (vs. being a member).
    pub is_owner: bool,
    /// Usernames with read/write access, excluding the owner. Empty unless `shared`.
    #[serde(default)]
    pub members: Vec<String>,
    /// The owner's username — needed to label a "Collections shared with me"
    /// row (`is_owner` alone can't tell a member *whose* collection this is).
    #[serde(default)]
    pub owner_username: String,
}

/// Input for creating or updating a note collection.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewNoteCollection {
    pub name: String,
    pub color: String,
    pub shared: bool,
    #[serde(default)]
    pub members: Vec<String>,
}

// ── Notes ──────────────────────────────────────────────────────────────────────

/// A first-class note label (backlog #114) — a per-user entity linked to any
/// number of notes. `color` is a palette value: hash-assigned on implicit
/// creation, or chosen in the label editor (backlog #128).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct NoteLabelInfo {
    pub id: i64,
    pub name: String,
    pub color: String,
}

/// A note as returned by `list_notes()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NoteInfo {
    pub id: i64,
    pub collection_id: i64,
    /// Optional short label, shown bold above `text` on the Notes-tab card.
    pub title: Option<String>,
    pub text: String,
    pub pinned: bool,
    /// Calendar slug the note's inline task/await events resolve to (backlog
    /// #112). `None` → `personal`. Not a per-token choice — one note, one
    /// calendar, matching the pre-#112 behaviour.
    pub calendar_name: Option<String>,
    /// ISO-ish timestamp string as stored by SQLite, for relative-time display.
    pub updated_at: String,
    /// Explicit grid position (backlog #47), 0-indexed, scoped independently
    /// per `pinned` zone — replaces #42's linear `sort_order`.
    pub grid_row: i32,
    pub grid_col: i32,
    /// Height in row units (backlog #48) —
    /// `ceil(rendered_px / crate::grid::CARD_GRID_ROW_UNIT_PX)`.
    /// Client-corrected after every render; starts at 1 for a brand-new note.
    pub grid_row_span: i32,
    /// Auto-mode manual reorder override (backlog #102), scoped
    /// independently per `pinned` zone like `grid_row`/`grid_col` are —
    /// see `ContactDto::manual_order`'s doc comment for the semantics.
    pub manual_order: Option<f64>,
    /// Inline images attached to this note (backlog #110), in display order.
    /// Metadata only — bytes are served by
    /// `GET /api/notes/{note_id}/images/{id}`, never embedded in this payload.
    #[serde(default)]
    pub images: Vec<NoteImageInfo>,
    /// First-class labels on this note (backlog #114), in link order.
    #[serde(default)]
    pub labels: Vec<NoteLabelInfo>,
}

/// An image attached to a note (backlog #110) — metadata only; the bytes are
/// fetched separately through the plain Axum image route.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct NoteImageInfo {
    pub id: i64,
    pub content_type: String,
    /// Original file name, when the upload supplied one. Used for
    /// download/export labelling, not for rendering.
    pub filename: Option<String>,
    /// Insertion order (0-based).
    pub position: i32,
}

/// A newly picked note image staged client-side (backlog #110). `temp_key` is
/// the short key used in a `@img:t<key>` placeholder that [`save_note`]
/// rewrites to the real `@img:<id>` token once the row is inserted — needed
/// because a brand-new note has no image ids to reference yet.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewNoteImage {
    pub temp_key: String,
    pub base64_data: String,
    pub content_type: String,
    pub filename: Option<String>,
}

/// Prefix of the inline note-image token stored in `notes.text` (backlog
/// #110). Chosen to mirror the `@Name` contact-mention form rather than
/// markdown `![]()` — the plain-text editor shows it literally.
pub const NOTE_IMAGE_TOKEN_PREFIX: &str = "@img:";

/// The stored token for image `id`, e.g. `@img:12`.
pub fn note_image_token(id: i64) -> String {
    format!("{NOTE_IMAGE_TOKEN_PREFIX}{id}")
}

/// Scans for the token beginning at `chars[i]` (which must be an `@`).
/// Returns `(end_index, body)` where `body` is everything after `@img:` up to
/// the first character that is neither a digit nor a `t` (the temp-key
/// marker). `None` when no valid token starts here.
pub(crate) fn note_image_token_at(chars: &[char], i: usize) -> Option<(usize, String)> {
    let prefix: Vec<char> = NOTE_IMAGE_TOKEN_PREFIX.chars().collect();
    if chars.len() < i + prefix.len() || chars[i..i + prefix.len()] != prefix[..] {
        return None;
    }
    let mut j = i + prefix.len();
    let mut body = String::new();
    while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == 't') {
        body.push(chars[j]);
        j += 1;
    }
    if body.is_empty() {
        None
    } else {
        Some((j, body))
    }
}

/// Removes every stored `@img:<id>` token whose id is in `ids` from `text`
/// (used when an existing image is detached; the trailing/leading whitespace
/// is left as-is). Temp tokens are never touched.
pub fn strip_note_image_tokens(text: &str, ids: &[i64]) -> String {
    if ids.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some((end, body)) = note_image_token_at(&chars, i) {
                if !body.starts_with('t') {
                    if let Ok(id) = body.parse::<i64>() {
                        if ids.contains(&id) {
                            i = end;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Rewrites staged `@img:t<key>` placeholders to real `@img:<id>` tokens for
/// every `(temp_key, id)` pair — called by [`save_note`] after inserting the
/// image rows. `temp_key` values are the keys *without* the leading `t`.
pub fn rewrite_note_image_temp_tokens(text: &str, map: &[(String, i64)]) -> String {
    if map.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some((end, body)) = note_image_token_at(&chars, i) {
                if let Some(key) = body.strip_prefix('t') {
                    if let Some((_, id)) = map.iter().find(|(k, _)| k == key) {
                        out.push_str(&note_image_token(*id));
                        i = end;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Every stored-image id (`@img:<digits>`, temp tokens excluded) referenced in
/// `text`. Used by [`save_note`] to prune `note_images` rows whose token the
/// user removed from the body.
pub fn note_image_ids_in_text(text: &str) -> Vec<i64> {
    let chars: Vec<char> = text.chars().collect();
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some((end, body)) = note_image_token_at(&chars, i) {
                if !body.starts_with('t') {
                    if let Ok(id) = body.parse::<i64>() {
                        ids.push(id);
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    ids
}

/// One card's target grid cell in a `crate::grid`-backed masonry grid — the
/// payload unit for `move_notes`/`move_contacts`'s bulk drag-drop persist
/// (the dragged card plus everyone `grid::resolve_drop` displaced to make
/// room for it, applied together in one transaction). Shared between Notes
/// and Contacts (backlog #79) — both grids use the exact same position
/// shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridPosition {
    pub id: i64,
    pub grid_row: i32,
    pub grid_col: i32,
    pub grid_row_span: i32,
}

/// Input for creating or updating a note.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewNote {
    pub collection_id: i64,
    /// Optional short label shown above `text` on the card.
    pub title: Option<String>,
    pub text: String,
    pub pinned: bool,
    /// Calendar slug the note's inline task/await events resolve to (backlog
    /// #112). `None` → `personal`.
    pub calendar_name: Option<String>,
    /// Label names to attach to this note (backlog #114). Each is resolved to
    /// (or created as) a per-user label on save; the note's link set becomes
    /// exactly this list. Blank/duplicate names are dropped.
    #[serde(default)]
    pub labels: Vec<String>,
}

// ── Unified 1-100 task priority ↔ CalDAV RFC 5545 PRIORITY (1-9) ────────────
//
// One canonical scale backs every task priority in the app: notes' slider,
// the Tasks-view creation slider, and markdown's `#task/p<N>` tag all speak
// 1-100 (1 = highest), which is also exactly what `events.priority` stores
// for every source. RFC 5545's native `PRIORITY` field only has 9 levels, so
// it's a lossy wire-format projection applied *only* at the CalDAV boundary
// (building/reading the `PRIORITY:` line in `ical_data`) — never stored as
// an intermediate representation. See backlog #70 / 2026-07-30 investigation
// for why an earlier 0-15 "representative priority" quantizer was scrapped:
// it wasn't actually shared with anything (markdown's own range is
// independently clamped to 1-100 in `tidl-markdown`) and its top tier (15)
// produced an out-of-spec `PRIORITY:15`.

/// Projects the canonical 1-100 task priority (1 = highest) onto RFC 5545's
/// native `PRIORITY` range (1 = highest, 9 = lowest) for the CalDAV wire
/// format. Lossy — 9 levels can't preserve a 100-point scale — see
/// [`caldav_priority_to_source_priority`], its inverse.
pub fn source_priority_to_caldav(priority: u32) -> i64 {
    let clamped = priority.clamp(1, 100) as i64;
    ((clamped - 1) * 8 / 99) + 1
}

/// Inverse of [`source_priority_to_caldav`]: projects an RFC 5545
/// `PRIORITY` value from a CalDAV client onto the canonical 1-100 scale.
/// Defensively clamps to 1-9 first — a client's raw value could be RFC's
/// "0 = undefined", negative, or (rare, non-conformant) above 9 — landing
/// each on the representative point that would itself round-trip back to
/// that same CalDAV value.
pub fn caldav_priority_to_source_priority(caldav: i64) -> u32 {
    let clamped = caldav.clamp(1, 9) as u32;
    ((clamped - 1) * 99 / 8) + 1
}

/// The urgency label (URGENT/SOON/UPCOMING/LATER) a note's own priority
/// would show in Focus — delegates to the exact scoring Focus itself uses
/// so the two can't drift apart. A note's priority is already on the
/// canonical 1-100 scale every task priority shares (backlog #70), so it's
/// passed straight through with no conversion, giving the slider's live
/// preview the full precision of wherever the user actually drags it.
pub fn note_priority_urgency_label(priority: u32, curves: &PriorityCurves) -> &'static str {
    crate::components::focus::urgency_label(crate::components::focus::task_priority_score(
        priority, curves,
    ))
}

/// The urgency color a note's own priority would show in Focus — see
/// `note_priority_urgency_label`.
pub fn note_priority_urgency_color(priority: u32, curves: &PriorityCurves) -> &'static str {
    crate::components::focus::urgency_color(crate::components::focus::task_priority_score(
        priority, curves,
    ))
}

// ── Markdown sources ──────────────────────────────────────────────────────────

/// A configured markdown source as returned by `list_markdown_sources()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MarkdownSourceInfo {
    pub id: i64,
    pub name: String,
    /// Absolute filesystem path to the directory or file (empty for git sources).
    pub path: String,
    pub enabled: bool,
    pub last_scan: Option<String>,
    /// Display colour (hex, e.g. "#8b5cf6").  Defaults to purple.
    #[serde(default = "default_md_color")]
    pub color: String,
    /// "filesystem" or "git".
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Git remote URL (git sources only).
    #[serde(default)]
    pub url: Option<String>,
    /// Git branch (git sources only).
    #[serde(default)]
    pub branch: Option<String>,
    /// true = provisioned via NixOS/flake config; read-only in the UI.
    #[serde(default)]
    pub config_managed: bool,
    /// How often (in seconds) this source is rescanned/pulled. Defaults to 60.
    #[serde(default = "default_md_sync_interval_secs")]
    pub sync_interval_secs: u32,
    /// Last pull/clone/scan error, shown in Settings and the markdown preview.
    #[serde(default)]
    pub last_error: Option<String>,
}

/// One markdown file found while walking an enabled markdown source, as
/// returned by `list_all_markdown_files()` for the file finder (backlog #75).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MarkdownFileEntry {
    pub source_id: i64,
    pub source_name: String,
    /// Display colour (hex or CSS var), same as `MarkdownSourceInfo::color`.
    pub source_color: String,
    /// Path relative to the source root, e.g. `"projects/tidl.md"`.
    pub rel_path: String,
}

/// One subdirectory entry returned by `list_server_directories()` for the
/// markdown-source folder browser.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DirEntryInfo {
    /// Directory name, e.g. `"notes"`.
    pub name: String,
    /// Absolute server-side path, e.g. `"/home/user/notes"`.
    pub path: String,
}

/// A directory listing for the markdown-source folder browser.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DirListing {
    /// Absolute path of the directory that was listed.
    pub path: String,
    /// Parent directory, or `None` at the filesystem root.
    pub parent: Option<String>,
    /// Subdirectories only, sorted case-insensitively by name.
    pub dirs: Vec<DirEntryInfo>,
}

/// Input for adding a new markdown source.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewMarkdownSource {
    pub name: String,
    /// Absolute path on the server filesystem.
    pub path: String,
    /// Display colour (hex).  Defaults to purple when empty.
    #[serde(default = "default_md_color")]
    pub color: String,
    /// How often (in seconds) this source is rescanned. Defaults to 60.
    #[serde(default = "default_md_sync_interval_secs")]
    pub sync_interval_secs: u32,
}

/// Input for adding a new git-backed markdown source.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewGitSource {
    pub name: String,
    pub url: String,
    pub branch: String,
    pub pat: String,
    pub color: String,
    /// How often (in seconds) this source is pulled. Defaults to 60.
    #[serde(default = "default_md_sync_interval_secs")]
    pub sync_interval_secs: u32,
}

fn default_md_color() -> String {
    "var(--pal-md-1)".to_string()
}

fn default_source_type() -> String {
    "filesystem".to_string()
}

/// Matches the markdown-sync loop's default poll cadence prior to per-source
/// intervals (`tidl-server/src/markdown_sync.rs`), so existing sources keep
/// their current behaviour.
fn default_md_sync_interval_secs() -> u32 {
    60
}

// ── Await items ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AwaitItem {
    pub id: String,
    pub summary: String,
    pub date: String,
    pub description: Option<String>,
    pub topic: Option<String>,
    pub source_file: Option<String>,
    /// For markdown-origin items: the 1-indexed line number, so Focus's
    /// "go to source" preview can jump to it (mirrors `Task::source_line`).
    #[serde(default)]
    pub source_line: Option<u32>,
    pub completed: bool,
    /// Set when this item originates from a note (backlog #37's mirrored
    /// await-note VTODOs): the originating collection's id (as a string,
    /// to match `HiddenNoteCollectionsCtx`'s key type). `None` for
    /// markdown-origin items.
    #[serde(default)]
    pub collection_id: Option<String>,
    /// For note-origin items: the originating note collection's name.
    #[serde(default)]
    pub collection_name: Option<String>,
    /// For note-origin items: the originating note collection's color.
    #[serde(default)]
    pub collection_color: Option<String>,
    /// For note-origin items: the originating note's own id, so "go to
    /// source" can open that exact note rather than just its collection.
    #[serde(default)]
    pub note_id: Option<i64>,
    /// For note-origin items: the originating note's labels (name + color,
    /// backlog #114/#127), so Focus can render label chips on the card and
    /// hide awaits whose note carries a hidden label. Empty for
    /// markdown-origin items.
    #[serde(default)]
    pub note_labels: Vec<NoteLabelInfo>,
    /// Calendar slug this await belongs to. Lets the edit dialog prefill the
    /// calendar picker so an ad-hoc await can be moved (backlog #118).
    #[serde(default)]
    pub calendar: String,
}

// ── Rendered task/await chip edit (backlog #121) ───────────────────────────────

/// Where a source-backed task/await chip's canonical item lives. Ad-hoc CalDAV
/// items aren't addressed this way — they already carry their uid and calendar.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TodoOrigin {
    /// An inline token in a note body: `note_id` plus the token's 1-based line
    /// and 0-based ordinal within that line.
    Note {
        note_id: i64,
        line: u32,
        ordinal: u32,
    },
    /// A markdown tag: the enclosing file's uid (`md:…`) plus the chip's
    /// 1-based line and 0-based ordinal within that line.
    Markdown {
        uid: String,
        line: u32,
        ordinal: u32,
    },
}

impl TodoOrigin {
    /// The canonical `events.uid` this chip addresses, when derivable. Note
    /// tokens are `note:{note_id}:{line}:{ordinal}`; markdown tags rebuild the
    /// same `md:{source_id}:{rel_path}:{line}:{ordinal}` the sync writes. Used
    /// to complete a chip via the same `complete_todo_item` path the card's
    /// checkbox uses (backlog #132).
    pub fn event_uid(&self) -> Option<String> {
        match self {
            TodoOrigin::Note {
                note_id,
                line,
                ordinal,
            } => Some(format!("note:{note_id}:{line}:{ordinal}")),
            TodoOrigin::Markdown { uid, line, ordinal } => {
                let parsed = MarkdownUid::parse(uid)?;
                Some(format!(
                    "md:{}:{}:{}:{}",
                    parsed.source_id, parsed.rel_path, line, ordinal
                ))
            }
        }
    }
}

/// A source-backed task/await item loaded for the edit modal (backlog #121).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TodoChip {
    pub origin: TodoOrigin,
    /// `"task"` or `"await"`.
    pub kind: String,
    pub title: String,
    /// Priority-form task only (1-100).
    #[serde(default)]
    pub priority: Option<u32>,
    /// Await date, or a dated markdown task's date (`YYYY-MM-DD`).
    #[serde(default)]
    pub date: Option<String>,
    /// True when this item's parameter is a date rather than a priority — an
    /// await, or markdown's `#task/<date>/<slug>` form.
    #[serde(default)]
    pub dated: bool,
}

// ── Done-item lifecycle (backlog #124, #125) ───────────────────────────────────

/// A completed item as shown on the central Done screen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DoneItem {
    pub id: i64,
    /// `"markdown"` | `"note"` | `"caldav"`.
    pub origin: String,
    pub scope_id: String,
    /// The origin's locator — a markdown/note uid (`md:…` / `note:…`) or a
    /// CalDAV uid — enough for the go-to-source jump to flash the exact tag.
    pub origin_key: String,
    /// `"task"` | `"await"`.
    pub kind: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub priority: Option<u32>,
    /// Original date (`YYYY-MM-DD`) for an await or a dated task.
    #[serde(default)]
    pub due_date: Option<String>,
    /// Completion date (`YYYY-MM-DD`), from the tag / event.
    pub completed_at: String,
    /// Set once archived; `None` while live.
    #[serde(default)]
    pub archived_at: Option<String>,
    /// File / collection / calendar display name.
    pub source_label: String,
    /// Source colour (markdown source / note collection / calendar), for the
    /// card's left stripe and source pill — mirrors Focus.
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub note_id: Option<i64>,
    #[serde(default)]
    pub calendar: Option<String>,
}

/// Which subset of done items to list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DoneFilter {
    /// `"markdown"` | `"note"` | `"caldav"`; `None` = all origins.
    #[serde(default)]
    pub origin: Option<String>,
    /// Restrict to one source / collection / calendar id.
    #[serde(default)]
    pub scope_id: Option<String>,
    /// Inclusive completion-date window (`YYYY-MM-DD`).
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    /// Case-insensitive title / slug search.
    #[serde(default)]
    pub query: Option<String>,
    /// Include archived rows; default false (live only).
    #[serde(default)]
    pub include_archived: bool,
}

/// A retention rule for live done items, ordered by completion date. The two
/// bounds are mutually exclusive in the UI; when both are set, `KeepNewest` wins.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum Retention {
    #[default]
    None,
    KeepNewest(u32),
    OlderThan(u32),
}

/// A clean-up scope (markdown source / note collection / calendar) with its
/// current settings and done-item counts, for the filter + Clean up… dialog.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DoneScope {
    pub origin: String,
    pub scope_id: String,
    pub label: String,
    pub live_count: u32,
    pub archived_count: u32,
    #[serde(default)]
    pub auto_archive: bool,
    #[serde(default)]
    pub keep_newest: Option<u32>,
    #[serde(default)]
    pub older_than_days: Option<u32>,
}

/// Per-user archive settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DoneSettings {
    /// `None` = keep archived items indefinitely.
    #[serde(default)]
    pub archive_retention_days: Option<u32>,
}

/// One source's share of a purge preview.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PurgeGroup {
    pub origin: String,
    pub scope_id: String,
    pub label: String,
    pub count: u32,
}

/// What a purge would archive (or did archive), grouped by scope.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PurgePreview {
    pub total: u32,
    pub groups: Vec<PurgeGroup>,
}

/// Result of an applied purge.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PurgeResult {
    pub archived: u32,
}

// ── User preferences ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserPrefs {
    /// IANA timezone identifier, e.g. "America/New_York" or "UTC".
    pub timezone: String,
    /// "12h" | "24h" | "zulu"
    pub time_format: String,
    /// Whether the user has enabled dark mode.
    #[serde(default)]
    pub dark_mode: bool,
    /// Optional shell command to open a note in an external editor.
    #[serde(default)]
    pub open_in_editor_cmd: Option<String>,
    /// Whether to show the small "click to show/hide" hint text above the
    /// collection/calendar/source lists in every sidebar.
    #[serde(default = "default_true")]
    pub show_sidebar_hints: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            timezone: "UTC".into(),
            time_format: "12h".into(),
            dark_mode: false,
            open_in_editor_cmd: None,
            show_sidebar_hints: true,
        }
    }
}

// ── Priority curves (backlog #68) ─────────────────────────────────────────────

/// One control point on a priority curve: `x` is the curve's input (hours
/// until a calendar event starts, days until a due date, or a task's own
/// priority — depending which curve this point belongs to); `y` is the
/// output urgency score, 0-100.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

/// A user's four configurable urgency curves. Each `Vec<CurvePoint>` must
/// be sorted by `x` with at least 2 entries — enforced server-side by
/// `save_priority_curve`'s validation; `Default::default()` below already
/// satisfies both.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PriorityCurves {
    /// x = hours until a calendar event starts (negative = already started
    /// or past). See `components::focus::event_urgency`.
    pub calendar_time_left: Vec<CurvePoint>,
    /// x = days until a task/await's due date (negative = overdue). See
    /// `components::focus::pending_urgency`.
    pub due_date_time_left: Vec<CurvePoint>,
    /// x = a task's own priority, 0 = highest (the existing small-int scale
    /// shared by caldav/markdown/note-mirrored tasks). See
    /// `components::focus::task_priority_score`.
    pub source_priority: Vec<CurvePoint>,
    /// x = a calendar event's own priority (canonical 1-100, 1 = highest);
    /// y = the maximum urgency that event may reach, i.e. an attenuation
    /// ceiling (backlog #156). Only consulted when the event has a priority.
    pub calendar_event_priority: Vec<CurvePoint>,
}

impl Default for PriorityCurves {
    /// Hand-tuned by dragging points in the Priority Curves settings tab
    /// (backlog #68) rather than derived from the old hardcoded thresholds —
    /// these are the exact points saved from a live-tuned curve set,
    /// rounded to 2 decimals (drag-pixel precision carries no more signal
    /// than that).
    fn default() -> Self {
        Self {
            calendar_time_left: vec![
                CurvePoint { x: -27.28, y: 0.0 },
                CurvePoint { x: -1.0, y: 0.0 },
                CurvePoint { x: 0.0, y: 99.0 },
                CurvePoint { x: 0.25, y: 99.0 },
                CurvePoint { x: 5.22, y: 90.81 },
                CurvePoint { x: 11.28, y: 68.59 },
                CurvePoint { x: 26.44, y: 49.15 },
                CurvePoint { x: 57.77, y: 26.37 },
                CurvePoint {
                    x: 100.72,
                    y: 11.37,
                },
                CurvePoint { x: 179.03, y: 0.26 },
            ],
            due_date_time_left: vec![
                CurvePoint { x: -8.17, y: 100.0 },
                CurvePoint { x: -0.1, y: 100.0 },
                CurvePoint { x: 1.11, y: 74.61 },
                CurvePoint { x: 4.15, y: 39.06 },
                CurvePoint { x: 9.4, y: 17.94 },
                CurvePoint { x: 15.1, y: 0.0 },
            ],
            source_priority: vec![
                CurvePoint { x: -0.83, y: 100.0 },
                CurvePoint { x: 4.69, y: 96.9 },
                CurvePoint { x: 11.01, y: 73.01 },
                CurvePoint { x: 20.48, y: 46.34 },
                CurvePoint { x: 44.69, y: 9.68 },
                CurvePoint { x: 69.17, y: 5.79 },
                CurvePoint { x: 105.22, y: 1.9 },
            ],
            // Highest priority = no cap; normal (~50) caps at 80, lowest at
            // 60 ("UPCOMING") so a prioritised event can never reach URGENT.
            calendar_event_priority: vec![
                CurvePoint { x: 1.0, y: 100.0 },
                CurvePoint { x: 100.0, y: 60.0 },
            ],
        }
    }
}

// ── Import result ─────────────────────────────────────────────────────────────

/// Outcome of an import operation — shared by `import_ics` (.ics calendar
/// import, backlog #10) and `import_vcf` (.vcf contacts import, backlog
/// #87), whose "imported/skipped/failed" shape is identical.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// Preview of a `.vcf` import before committing (backlog #87): how many
/// `VCARD`s were found and how many look like duplicates of an existing
/// contact, using the same heuristic `import_vcf` applies when overwrite is
/// declined. Returned by `preview_vcf_import`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VcfImportPreview {
    pub total: u32,
    pub duplicates: u32,
}

/// Outcome of a Google Keep Takeout import (backlog #109). A richer shape
/// than [`ImportResult`] because it also reports how many Keep attachments
/// were re-attached as note images, which the summary line surfaces.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct KeepImportResult {
    pub imported: u32,
    /// Trashed notes (always) plus archived notes when the caller declined
    /// them — kept notes are never counted as "skipped" for another reason.
    pub skipped: u32,
    pub failed: u32,
    /// Re-imports of a Keep note already present (matched by its persisted
    /// `createdTimestampUsec`). Always dropped, never overwritten.
    pub duplicates: u32,
    /// Attachments successfully stored as note images across all imported
    /// notes. Attachments that were missing from the zip, unsupported types,
    /// or over the size cap are silently dropped (and not counted here).
    pub images: u32,
    /// Label links created across all imported notes (backlog #114). Labels
    /// are the caller's own per-user entities, resolved or created as needed.
    pub labels: u32,
}

// ── Calendar sharing ──────────────────────────────────────────────────────────

/// An accepted share grant as returned by `get_calendar_shares`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShareInfo {
    pub grantee_username: String,
    pub permission: String,
}

/// A calendar that has been shared *with* the current user by someone else.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SharedCalendar {
    pub owner_username: String,
    pub calendar_name: String,
    pub display_name: String,
    pub color: String,
    pub permission: String,
}

// ── Unified calendar list ───────────────────────────────────────────────────────

/// A single entry in the unified "calendars I can see" list — either one of
/// the current user's own calendars, or one shared with them by someone
/// else. Keyed by `(owner_username, calendar_name)` rather than bare name so
/// an owned and a shared calendar that happen to share a name never alias
/// each other's visibility state (a slug collision the previous per-page,
/// name-only models were prone to).
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarEntry {
    pub owner_username: String,
    pub calendar_name: String,
    pub display_name: String,
    pub color: String,
    /// `None` = owned by the current user. `Some(permission)` = shared with
    /// them at that permission level ("read" or "read-write").
    pub shared_permission: Option<String>,
    /// Owned-only: database row ID, needed by `CalendarEditModal`. `None` for
    /// shared entries, which the grantee can't edit.
    pub id: Option<i64>,
    pub sync_url: Option<String>,
    pub sync_error: Option<String>,
    pub last_synced_at: Option<String>,
    pub read_only: bool,
    /// Owned-only: true if this calendar has at least one active grantee
    /// (backlog #65). Always `false` for shared-with-me entries — their own
    /// share state is already carried by `shared_permission`.
    pub shared: bool,
}

impl CalendarEntry {
    pub fn is_owned(&self) -> bool {
        self.shared_permission.is_none()
    }

    /// Reconstruct the `CalendarInfo` `CalendarEditModal` expects. `None` for
    /// shared entries (no `id`) — the grantee never opens this modal for them.
    pub fn to_calendar_info(&self) -> Option<CalendarInfo> {
        Some(CalendarInfo {
            id: self.id?,
            name: self.calendar_name.clone(),
            display_name: self.display_name.clone(),
            color: self.color.clone(),
            read_only: self.read_only,
            sync_url: self.sync_url.clone(),
            sync_error: self.sync_error.clone(),
            last_synced_at: self.last_synced_at.clone(),
            shared: self.shared,
        })
    }
}

/// Merge the current user's own calendars with calendars shared with them
/// into one list of `(CalendarEntry, visible)` pairs, carrying over each
/// entry's visibility flag from `existing` by `(owner_username,
/// calendar_name)` — entries not present in `existing` (newly appeared)
/// default to visible.
pub fn merge_calendar_entries(
    my_username: &str,
    owned: Vec<CalendarInfo>,
    shared: Vec<SharedCalendar>,
    existing: &[(CalendarEntry, bool)],
) -> Vec<(CalendarEntry, bool)> {
    let visible_of = |owner: &str, name: &str| -> bool {
        existing
            .iter()
            .find(|(e, _)| e.owner_username == owner && e.calendar_name == name)
            .map(|(_, v)| *v)
            .unwrap_or(true)
    };

    let mut result: Vec<(CalendarEntry, bool)> = owned
        .into_iter()
        .map(|c| {
            let visible = visible_of(my_username, &c.name);
            let entry = CalendarEntry {
                owner_username: my_username.to_string(),
                calendar_name: c.name,
                display_name: c.display_name,
                color: c.color,
                shared_permission: None,
                id: Some(c.id),
                sync_url: c.sync_url,
                sync_error: c.sync_error,
                last_synced_at: c.last_synced_at,
                read_only: c.read_only,
                shared: c.shared,
            };
            (entry, visible)
        })
        .collect();

    result.extend(shared.into_iter().map(|s| {
        let visible = visible_of(&s.owner_username, &s.calendar_name);
        let read_only = s.permission == "read";
        let entry = CalendarEntry {
            owner_username: s.owner_username,
            calendar_name: s.calendar_name,
            display_name: s.display_name,
            color: s.color,
            shared_permission: Some(s.permission),
            id: None,
            sync_url: None,
            sync_error: None,
            last_synced_at: None,
            read_only,
            shared: false,
        };
        (entry, visible)
    }));

    result
}

// ── Session info ──────────────────────────────────────────────────────────────

/// Basic information about the authenticated user, returned by `get_session_info`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub username: String,
    pub is_admin: bool,
}

/// The signed-in user's editable profile fields, returned by `get_profile`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProfileInfo {
    pub username: String,
    pub display_name: String,
    pub email: String,
}

// ── SMTP configuration ─────────────────────────────────────────────────────────

/// SMTP settings as returned by the server (password omitted).
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// "none" | "starttls" | "tls"
    pub tls_mode: String,
    pub username: String,
    /// Password is never sent to the client; this field is always empty in responses.
    pub password: String,
    pub from_address: String,
    pub from_name: String,
}

// ── App tokens ────────────────────────────────────────────────────────────────

/// A named app token as listed in Settings (the raw token is never included).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppTokenInfo {
    pub id: String,
    pub name: String,
    pub read_only: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

// ── User administration ────────────────────────────────────────

/// A user account as listed in the admin "Users" settings tab.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserAdminInfo {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
    pub is_active: bool,
}

/// Counts of data a user still owns, shown before a hard delete.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserDeletionSummary {
    pub calendars: i64,
    pub notes: i64,
    pub collections: i64,
    pub shares_owned: i64,
    pub app_tokens: i64,
}

// ── Inbound email tokens ──────────────────────────────────────────────────────

/// An inbound email token as listed in Settings (the raw token is never included).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundEmailTokenInfo {
    pub id: String,
    pub label: String,
    pub calendar_id: i64,
    pub calendar_name: String,
    pub address: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

// ── Trusted senders ───────────────────────────────────────────────────────────

/// A trusted sender address as listed in Settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedSenderInfo {
    pub id: String,
    pub email: String,
    pub created_at: String,
}

// ── Remote CalDAV sources ─────────────────────────────────────────────────────

/// A remote CalDAV source as listed in Settings (token is never included).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct RemoteCalDavSource {
    pub id: String,
    pub name: String,
    pub remote_base_url: String,
    pub remote_username: String,
    pub remote_calendar: String,
    pub local_calendar: String,
    /// "pull" | "push" | "both"
    pub sync_direction: String,
    pub enabled: bool,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    /// How often (in seconds) this source is synced. Defaults to 300 (5 minutes).
    #[serde(default = "default_sync_interval_secs")]
    pub sync_interval_secs: u32,
}

/// Input for creating a new remote CalDAV source (includes the token).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NewRemoteCalDavSource {
    pub name: String,
    pub remote_base_url: String,
    pub remote_username: String,
    /// The raw app token — encrypted before storage.
    pub remote_token: String,
    pub remote_calendar: String,
    pub local_calendar: String,
    /// "pull" | "push" | "both"
    pub sync_direction: String,
    /// How often (in seconds) this source is synced. Defaults to 300 (5 minutes).
    #[serde(default = "default_sync_interval_secs")]
    pub sync_interval_secs: u32,
}

impl Default for NewRemoteCalDavSource {
    fn default() -> Self {
        Self {
            name: String::new(),
            remote_base_url: String::new(),
            remote_username: String::new(),
            remote_token: String::new(),
            remote_calendar: String::new(),
            local_calendar: String::new(),
            sync_direction: String::new(),
            sync_interval_secs: 300,
        }
    }
}

/// Input for updating an existing remote CalDAV source.
///
/// Changing `local_calendar` (or `remote_base_url`/`remote_username`/
/// `remote_calendar`) resets the stored sync cursors so the next sync
/// treats it as attaching fresh: if the newly-mapped calendar already has
/// events, they get wiped first (same as attaching to an existing calendar
/// at creation time — the UI warns about this before saving), and only
/// changes from that point on are pulled/pushed. See
/// `remote_sync::wipe_calendar_events`/`seed_last_push_token_if_unset` and
/// `update_remote_source_tx` in `tidl-server`/`tidl-web`. Events left behind
/// in the *old* local calendar (if it changed) are untouched — they just
/// stop being synced.
///
/// `remote_token` is `None` (or blank) to keep the existing encrypted token
/// unchanged — the raw token is never sent back to the client after
/// creation, so there is nothing to pre-fill an edit form with.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UpdateRemoteCalDavSource {
    pub name: String,
    pub remote_base_url: String,
    pub remote_username: String,
    pub remote_token: Option<String>,
    pub remote_calendar: String,
    pub local_calendar: String,
    /// "pull" | "push" | "both"
    pub sync_direction: String,
    pub sync_interval_secs: u32,
}

fn default_sync_interval_secs() -> u32 {
    300
}

/// The remote CalDAV and markdown sync schedulers both only wake every
/// `TICK_SECS` (10s, see `tidl-server/src/remote_sync.rs` and
/// `tidl-server/src/markdown_sync.rs`) to check which sources are due, so no
/// source can ever sync faster than that regardless of its configured
/// interval. Kept in sync with `TICK_SECS` by comment reference since
/// `tidl-web` cannot depend on `tidl-server`.
pub const MIN_SYNC_INTERVAL_SECS: u32 = 10;

// ── Contacts (backlog #79) ───────────────────────────────────────────────────

/// One email address or phone number on a contact. Used for both
/// `ContactDto::emails`/`phones` and `NewContact`/`ContactUpdate`'s
/// corresponding fields — the DB row shape (`contact_emails`/
/// `contact_phones`) is identical for both, only the column they live in
/// differs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ContactMethod {
    pub value: String,
    /// "home" | "work" | "mobile" | "other" — matches the `type` CHECK
    /// constraint on `contact_emails`/`contact_phones`/`contact_urls`/
    /// `contact_impps`.
    pub kind: String,
    pub is_primary: bool,
}

/// One postal address. vCard's `ADR` is repeatable (home + work are common),
/// unlike the rest of a contact's fields — found via a real bug (backlog
/// #29's iOS testing) where a single flat set of address columns silently
/// overwrote itself on a second `ADR` line.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ContactAddress {
    /// "home" | "work" | "other" — no "mobile", doesn't apply to an address.
    pub kind: String,
    pub is_primary: bool,
    pub street: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// A contact as returned by `list_contacts()`/`get_contact()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ContactDto {
    pub id: i64,
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    /// `yyyy-mm-dd`.
    pub birthday: Option<String>,
    /// `yyyy-mm-dd`.
    pub anniversary: Option<String>,
    pub notes: Option<String>,
    pub categories: Vec<String>,
    pub emails: Vec<ContactMethod>,
    pub phones: Vec<ContactMethod>,
    pub urls: Vec<ContactMethod>,
    pub impps: Vec<ContactMethod>,
    pub addresses: Vec<ContactAddress>,
    /// Whether this contact has a photo — the bytes themselves aren't in
    /// this DTO (it backs the whole grid list, and photos are too large to
    /// embed in every row); fetch them from `GET /api/contacts/{id}/photo`.
    pub has_photo: bool,
    /// ISO-ish timestamp string as stored by SQLite, for relative-time display.
    pub updated_at: String,
    /// Explicit grid position (backlog #47/#48, shared with Notes — see
    /// `crate::grid`), 0-indexed. Contacts have a single flat zone (no
    /// pinning, no reserved column-0 floor).
    pub grid_row: i32,
    pub grid_col: i32,
    /// Height in row units — see `NoteInfo::grid_row_span`.
    pub grid_row_span: i32,
    /// Which collection this contact belongs to (backlog #81).
    pub collection_id: i64,
    /// Auto-mode manual reorder override (backlog #102) — `None` means
    /// "not yet placed," sorting alphabetically; `Some(v)` sorts by `v`
    /// among other explicitly-ordered contacts, before every `None` one.
    pub manual_order: Option<f64>,
}

/// Input for creating or updating a contact.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NewContact {
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    pub birthday: Option<String>,
    pub anniversary: Option<String>,
    pub notes: Option<String>,
    pub categories: Vec<String>,
    pub emails: Vec<ContactMethod>,
    pub phones: Vec<ContactMethod>,
    pub urls: Vec<ContactMethod>,
    pub impps: Vec<ContactMethod>,
    pub addresses: Vec<ContactAddress>,
    /// Target collection (backlog #81). `None` on create falls back to the
    /// creator's default collection; ignored on update (a contact's
    /// collection doesn't change via the general edit form in this plan —
    /// no cross-collection move UI yet).
    pub collection_id: Option<i64>,
}

/// A contact collection as returned by `list_contact_collections()` —
/// mirrors `NoteCollectionInfo` exactly (backlog #81).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContactCollectionInfo {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub shared: bool,
    /// True when the current user owns this collection (vs. being a member).
    pub is_owner: bool,
    /// Usernames with access, excluding the owner. Empty unless `shared`.
    #[serde(default)]
    pub members: Vec<String>,
    /// The owner's username — needed to label a "Collections shared with
    /// me" row (`is_owner` alone can't tell a member *whose* collection
    /// this is).
    #[serde(default)]
    pub owner_username: String,
}

/// Input for creating/updating a contact collection — mirrors `NewNoteCollection`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewContactCollection {
    pub name: String,
    pub color: String,
    pub shared: bool,
    #[serde(default)]
    pub members: Vec<String>,
}

/// User-resolved scalar fields for the surviving contact after a merge
/// (backlog #88) — mirrors `NewContact`'s scalar subset exactly. Repeatable
/// fields (emails/phones/urls/impps/addresses/categories) are *not* here:
/// they're unioned automatically server-side across every selected
/// contact (see `union_methods` et al in `api/contacts.rs`), never
/// resolved by the user, since a union can't silently discard data the way
/// picking one scalar value over another can.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MergedScalarFields {
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    pub birthday: Option<String>,
    pub anniversary: Option<String>,
    pub notes: Option<String>,
}

/// Input for `merge_contacts` (backlog #88). `primary_id` survives;
/// every id in `merged_ids` is deleted after merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeContactsInput {
    pub primary_id: i64,
    pub merged_ids: Vec<i64>,
    pub resolved: MergedScalarFields,
    /// Copy this contact's photo onto `primary_id` instead of keeping
    /// `primary_id`'s own. `None` leaves the primary's existing photo
    /// (if any) untouched. Must be `primary_id` itself or one of
    /// `merged_ids`.
    pub keep_photo_from: Option<i64>,
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CalendarInfo ──────────────────────────────────────────────────────────

    fn cal(name: &str, display_name: &str, color: &str) -> CalendarInfo {
        CalendarInfo {
            id: 0,
            name: name.into(),
            display_name: display_name.into(),
            color: color.into(),
            read_only: false,
            sync_url: None,
            sync_error: None,
            last_synced_at: None,
            shared: false,
        }
    }

    #[test]
    fn calendar_info_serde_roundtrip() {
        let info = cal("work", "Work", "#ff0000");
        let json = serde_json::to_string(&info).unwrap();
        let decoded: CalendarInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, decoded);
    }

    #[test]
    fn calendar_info_equality() {
        let a = cal("a", "A", "#fff");
        let b = cal("a", "A", "#fff");
        assert_eq!(a, b);

        let c = cal("b", "B", "#000");
        assert_ne!(a, c);
    }

    // ── Alarm ────────────────────────────────────────────────────────────────

    fn alarm_with_action(action: &str) -> Alarm {
        Alarm {
            action: action.into(),
            amount: 15,
            unit: "minutes".into(),
            repeat_count: None,
            repeat_delay_m: None,
            raw_ical: None,
            is_raw: false,
        }
    }

    #[test]
    fn only_audio_alarms_play_sound() {
        assert!(alarm_with_action("audio").plays_sound());
        assert!(alarm_with_action("AUDIO").plays_sound());
        assert!(!alarm_with_action("display").plays_sound());
        assert!(!alarm_with_action("email").plays_sound());
    }

    // ── CalendarEntry / merge_calendar_entries ──────────────────────────────────

    fn shared_cal(
        owner: &str,
        name: &str,
        display_name: &str,
        color: &str,
        perm: &str,
    ) -> SharedCalendar {
        SharedCalendar {
            owner_username: owner.into(),
            calendar_name: name.into(),
            display_name: display_name.into(),
            color: color.into(),
            permission: perm.into(),
        }
    }

    #[test]
    fn merge_owned_only() {
        let merged = merge_calendar_entries(
            "alice",
            vec![cal("personal", "Personal", "#3b82f6")],
            vec![],
            &[],
        );
        assert_eq!(merged.len(), 1);
        let (entry, visible) = &merged[0];
        assert_eq!(entry.owner_username, "alice");
        assert_eq!(entry.calendar_name, "personal");
        assert!(entry.is_owned());
        assert!(visible);
    }

    #[test]
    fn merge_owned_carries_shared_flag() {
        let mut c = cal("personal", "Personal", "#3b82f6");
        c.shared = true;
        let merged = merge_calendar_entries("alice", vec![c], vec![], &[]);
        assert!(merged[0].0.shared);
    }

    #[test]
    fn merge_shared_with_me_entry_has_shared_false() {
        // `shared` describes the owner's grantee count, which is meaningless
        // (and unavailable) for a shared-with-me entry — always false there.
        let merged = merge_calendar_entries(
            "bob",
            vec![],
            vec![shared_cal(
                "alice",
                "family",
                "Family",
                "#f00",
                "read-write",
            )],
            &[],
        );
        assert!(!merged[0].0.shared);
    }

    #[test]
    fn merge_shared_only() {
        let merged = merge_calendar_entries(
            "bob",
            vec![],
            vec![shared_cal(
                "alice",
                "family",
                "Family",
                "#f00",
                "read-write",
            )],
            &[],
        );
        assert_eq!(merged.len(), 1);
        let (entry, visible) = &merged[0];
        assert_eq!(entry.owner_username, "alice");
        assert_eq!(entry.calendar_name, "family");
        assert!(!entry.is_owned());
        assert_eq!(entry.shared_permission.as_deref(), Some("read-write"));
        assert!(!entry.read_only);
        assert!(visible);
    }

    #[test]
    fn merge_shared_read_only_permission_sets_read_only_flag() {
        let merged = merge_calendar_entries(
            "bob",
            vec![],
            vec![shared_cal("alice", "family", "Family", "#f00", "read")],
            &[],
        );
        assert!(merged[0].0.read_only);
    }

    #[test]
    fn merge_owned_and_shared_same_name_do_not_collide() {
        // bob owns a "personal" calendar AND has alice's "personal" calendar
        // shared with him. Before the (owner, name) keying these would alias.
        let merged = merge_calendar_entries(
            "bob",
            vec![cal("personal", "Personal", "#3b82f6")],
            vec![shared_cal(
                "alice",
                "personal",
                "Alice's Personal",
                "#f00",
                "read",
            )],
            &[],
        );
        assert_eq!(merged.len(), 2);
        let mine = merged
            .iter()
            .find(|(e, _)| e.owner_username == "bob")
            .unwrap();
        let theirs = merged
            .iter()
            .find(|(e, _)| e.owner_username == "alice")
            .unwrap();
        assert!(mine.0.is_owned());
        assert!(!theirs.0.is_owned());

        // Hiding bob's own "personal" must not affect alice's shared "personal".
        let existing = vec![(mine.0.clone(), false), (theirs.0.clone(), true)];
        let remerged = merge_calendar_entries(
            "bob",
            vec![cal("personal", "Personal", "#3b82f6")],
            vec![shared_cal(
                "alice",
                "personal",
                "Alice's Personal",
                "#f00",
                "read",
            )],
            &existing,
        );
        let mine2 = remerged
            .iter()
            .find(|(e, _)| e.owner_username == "bob")
            .unwrap();
        let theirs2 = remerged
            .iter()
            .find(|(e, _)| e.owner_username == "alice")
            .unwrap();
        assert!(!mine2.1);
        assert!(theirs2.1);
    }

    #[test]
    fn merge_preserves_visibility_across_refetch() {
        let existing =
            merge_calendar_entries("alice", vec![cal("work", "Work", "#000")], vec![], &[]);
        let (entry, _) = &existing[0];
        let hidden = vec![(entry.clone(), false)];

        let remerged =
            merge_calendar_entries("alice", vec![cal("work", "Work", "#000")], vec![], &hidden);
        assert!(
            !remerged[0].1,
            "visibility must carry over from the existing list"
        );
    }

    #[test]
    fn merge_new_entry_defaults_to_visible() {
        let existing = vec![]; // nothing known yet
        let merged =
            merge_calendar_entries("alice", vec![cal("new", "New", "#000")], vec![], &existing);
        assert!(merged[0].1);
    }

    // ── Event ─────────────────────────────────────────────────────────────────

    #[test]
    fn event_calendar_field_defaults_empty() {
        // When the JSON payload omits "calendar", it should default to "".
        let json =
            r#"{"id":"uid1","summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert_eq!(event.calendar, "");
    }

    #[test]
    fn event_calendar_color_defaults_none() {
        let json =
            r#"{"id":"uid1","summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert_eq!(event.calendar_color, None);
    }

    #[test]
    fn event_with_calendar_fields_roundtrips() {
        let event = Event {
            id: "uid1".to_string(),
            summary: "Team standup".to_string(),
            start: "2024-03-01 09:00".to_string(),
            end: "2024-03-01 09:30".to_string(),
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "work".to_string(),
            calendar_color: Some("#e11d48".to_string()),
            all_day: false,
            rrule: None,
            occurrence_start: None,
            is_exception: false,
            start_epoch_ms: 0,
            owner_username: String::new(),
            shared_permission: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.calendar, "work");
        assert_eq!(decoded.calendar_color, Some("#e11d48".to_string()));
    }

    #[test]
    fn event_explicit_calendar_preserved() {
        let color = "#16a34a";
        let json = format!(
            r#"{{"id":"uid2","summary":"Sync","start":"2024-02-01 14:00","end":"2024-02-01 15:00","calendar":"family","calendar_color":"{}"}}"#,
            color
        );
        let event: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event.calendar, "family");
        assert_eq!(event.calendar_color, Some(color.to_string()));
    }

    // ── NewEvent ──────────────────────────────────────────────────────────────

    #[test]
    fn new_event_calendar_defaults_to_personal() {
        // When "calendar" is absent from JSON, the serde default fires.
        let json = r#"{"summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: NewEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.calendar, "personal");
    }

    #[test]
    fn new_event_calendar_explicit() {
        let json = r#"{"summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00","calendar":"work"}"#;
        let event: NewEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.calendar, "work");
    }

    // ── Alarm defaults ────────────────────────────────────────────────────────

    #[test]
    fn alarm_action_defaults_to_display() {
        // "action" is omitted — default_action() should fill it in.
        let json = r#"{"amount":15,"unit":"minutes"}"#;
        let alarm: Alarm = serde_json::from_str(json).unwrap();
        assert_eq!(alarm.action, "display");
    }

    #[test]
    fn alarm_action_explicit() {
        let json = r#"{"action":"email","amount":30,"unit":"minutes"}"#;
        let alarm: Alarm = serde_json::from_str(json).unwrap();
        assert_eq!(alarm.action, "email");
    }

    // ── Event new fields (rrule, occurrence_start, is_exception) ──────────────

    #[test]
    fn event_new_fields_default_when_absent() {
        let json =
            r#"{"id":"uid1","summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert_eq!(event.rrule, None);
        assert_eq!(event.occurrence_start, None);
        assert!(!event.is_exception);
    }

    #[test]
    fn event_new_fields_roundtrip() {
        let event = Event {
            id: "uid1".to_string(),
            summary: "Test".to_string(),
            start: "2024-01-01 10:00".to_string(),
            end: "2024-01-01 11:00".to_string(),
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".to_string(),
            calendar_color: None,
            all_day: false,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE".to_string()),
            occurrence_start: Some("20240101T100000".to_string()),
            is_exception: false,
            start_epoch_ms: 0,
            owner_username: String::new(),
            shared_permission: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.rrule, Some("FREQ=WEEKLY;BYDAY=MO,WE".to_string()));
        assert_eq!(
            decoded.occurrence_start,
            Some("20240101T100000".to_string())
        );
        assert!(!decoded.is_exception);
    }

    // ── Event shared-calendar fields (backlog #41) ─────────────────────────────

    #[test]
    fn event_shared_fields_default_when_absent() {
        let json =
            r#"{"id":"uid1","summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert_eq!(event.owner_username, "");
        assert_eq!(event.shared_permission, None);
    }

    #[test]
    fn event_shared_fields_roundtrip() {
        let event = Event {
            id: "uid1".to_string(),
            summary: "Test".to_string(),
            start: "2024-01-01 10:00".to_string(),
            end: "2024-01-01 11:00".to_string(),
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "family".to_string(),
            calendar_color: None,
            all_day: false,
            rrule: None,
            occurrence_start: None,
            is_exception: false,
            start_epoch_ms: 0,
            owner_username: "alice".to_string(),
            shared_permission: Some("read-write".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.owner_username, "alice");
        assert_eq!(decoded.shared_permission, Some("read-write".to_string()));
    }

    #[test]
    fn new_event_rrule_defaults_none() {
        let json = r#"{"summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00"}"#;
        let event: NewEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.rrule, None);
    }

    #[test]
    fn new_event_rrule_explicit() {
        let json = r#"{"summary":"Test","start":"2024-01-01 10:00","end":"2024-01-01 11:00","rrule":"FREQ=DAILY"}"#;
        let event: NewEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.rrule, Some("FREQ=DAILY".to_string()));
    }
}

#[cfg(test)]
mod notes_priority_tests {
    use super::*;

    #[test]
    fn source_priority_to_caldav_endpoints_and_clamping() {
        assert_eq!(source_priority_to_caldav(1), 1);
        assert_eq!(source_priority_to_caldav(100), 9);
        // Below/above the valid domain clamps rather than panicking or
        // producing an out-of-spec CalDAV value.
        assert_eq!(source_priority_to_caldav(0), 1);
        assert_eq!(source_priority_to_caldav(1000), 9);
    }

    #[test]
    fn source_priority_to_caldav_is_monotonic() {
        let values: Vec<i64> = (1..=100).map(source_priority_to_caldav).collect();
        for w in values.windows(2) {
            assert!(w[0] <= w[1], "not monotonic: {values:?}");
        }
        assert_eq!(*values.first().unwrap(), 1);
        assert_eq!(*values.last().unwrap(), 9);
    }

    #[test]
    fn caldav_priority_to_source_priority_endpoints_and_clamping() {
        assert_eq!(caldav_priority_to_source_priority(1), 1);
        assert_eq!(caldav_priority_to_source_priority(9), 100);
        // RFC 5545 "0 = undefined" and any negative/out-of-range client
        // value clamp to the highest tier rather than panicking.
        assert_eq!(caldav_priority_to_source_priority(0), 1);
        assert_eq!(caldav_priority_to_source_priority(-5), 1);
        assert_eq!(caldav_priority_to_source_priority(99), 100);
    }

    #[test]
    fn caldav_priority_to_source_priority_is_monotonic() {
        let values: Vec<u32> = (1..=9).map(caldav_priority_to_source_priority).collect();
        for w in values.windows(2) {
            assert!(w[0] < w[1], "not strictly increasing: {values:?}");
        }
    }

    #[test]
    fn urgency_label_and_color_match_focus_exactly() {
        // Each quartile must land in a distinct Focus urgency label/color --
        // this is the whole point of delegating to Focus's own scoring
        // instead of keeping a separately-maintained High/Medium/Low scale.
        // Uses a fixed fixture rather than `PriorityCurves::default()`,
        // which is now a user's own live-tuned curve (backlog #68) and has
        // no obligation to keep every quartile distinct — see the
        // 2026-07-30 investigation.
        let curves = PriorityCurves {
            // Irrelevant to this test — note_priority_urgency_label/_color
            // only ever reads `source_priority`.
            calendar_time_left: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.0 }],
            due_date_time_left: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.0 }],
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
                CurvePoint { x: 100.0, y: 60.0 },
            ],
        };
        assert_eq!(note_priority_urgency_label(0, &curves), "URGENT");
        assert_eq!(note_priority_urgency_color(0, &curves), "#d93025");
        assert_eq!(note_priority_urgency_label(25, &curves), "SOON");
        assert_eq!(note_priority_urgency_color(25, &curves), "#f09300");
        assert_eq!(note_priority_urgency_label(50, &curves), "UPCOMING");
        assert_eq!(note_priority_urgency_color(50, &curves), "#1a73e8");
        assert_eq!(note_priority_urgency_label(100, &curves), "LATER");
        assert_eq!(note_priority_urgency_color(100, &curves), "#bbb");
    }
}

#[cfg(test)]
mod note_image_token_tests {
    use super::{
        note_image_ids_in_text, note_image_token, rewrite_note_image_temp_tokens,
        strip_note_image_tokens,
    };

    #[test]
    fn token_format() {
        assert_eq!(note_image_token(12), "@img:12");
    }

    #[test]
    fn rewrite_temp_tokens_to_real_ids() {
        let map = vec![("1".to_string(), 42i64), ("2".to_string(), 43i64)];
        assert_eq!(
            rewrite_note_image_temp_tokens("look @img:t1 then @img:t2 end", &map),
            "look @img:42 then @img:43 end"
        );
    }

    #[test]
    fn rewrite_leaves_unknown_temp_token_alone() {
        let map = vec![("1".to_string(), 42i64)];
        assert_eq!(
            rewrite_note_image_temp_tokens("@img:t1 and @img:t9", &map),
            "@img:42 and @img:t9"
        );
    }

    #[test]
    fn rewrite_does_not_touch_real_tokens() {
        let map = vec![("1".to_string(), 42i64)];
        assert_eq!(
            rewrite_note_image_temp_tokens("keep @img:7", &map),
            "keep @img:7"
        );
    }

    #[test]
    fn strip_removes_only_listed_ids() {
        let text = "a @img:1 b @img:2 c @img:t3 d";
        assert_eq!(
            strip_note_image_tokens(text, &[2]),
            "a @img:1 b  c @img:t3 d"
        );
    }

    #[test]
    fn strip_no_ids_is_a_no_op() {
        assert_eq!(strip_note_image_tokens("a @img:1", &[]), "a @img:1");
    }

    #[test]
    fn ids_in_text_collects_numeric_tokens_only() {
        assert_eq!(
            note_image_ids_in_text("@img:5 x @img:t9 y @img:6"),
            vec![5, 6]
        );
    }

    #[test]
    fn ids_in_text_empty_when_no_tokens() {
        assert!(note_image_ids_in_text("plain @ mention").is_empty());
    }

    #[test]
    fn partial_or_malformed_tokens_are_ignored() {
        assert!(note_image_ids_in_text("@img: @img").is_empty());
        // Non-boundary occurrence is still matched (documented looseness).
        assert_eq!(note_image_ids_in_text("@img:3x"), vec![3]);
    }
}

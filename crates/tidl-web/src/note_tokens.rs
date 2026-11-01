//! Inline note-body task/await/done tokens (backlog #113).
//!
//! The grammar is markdown's task/await convention (`#task/p<N>/<slug>`,
//! `#await/<date>/<slug>`, plus the legacy `#todo`/`#pending` aliases) extended
//! with an inert `#done/...` completion record that keeps the original
//! priority/date so re-opening is a pure text rewrite.
//!
//! This lives in `tidl-web` rather than `tidl-markdown` because it must compile
//! to wasm32 — `tidl-markdown` as a whole does not (its watcher/sync modules
//! pull in `tokio`/`notify`). Both the client (card rendering, editor) and the
//! server fns (`api/notes.rs`, `api/events.rs`, `api/keep.rs`) use this one
//! module, so what renders and what syncs can't drift.

use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

// These match only the tag itself (`#tag/param/slug`), with no trailing-text
// capture — a token's title always comes from `slug_to_title(slug)`. A
// `#done` record's second segment is `p<digits>` (a task) or a date (an
// await); the two are mutually exclusive, so match order doesn't matter.
static TASK_PRIORITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?:task|todo)/p(\d+)/([a-zA-Z0-9-]+)").unwrap());
static AWAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#(?:await|pending)/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});
static DONE_TASK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#done/(\d{4}-\d{2}-\d{2})/p(\d+)/([a-zA-Z0-9-]+)").unwrap());
static DONE_AWAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#done/(\d{4}-\d{2}-\d{2})/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});

/// What a `#done/...` token was before it was completed — enough to reopen it
/// by rewriting the token back to its original form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoneOriginal {
    /// `#done/<completed>/p<N>/<slug>` — reopen to `#task/p<N>/<slug>`.
    Task { priority: u32 },
    /// `#done/<completed>/<date>/<slug>` — reopen to `#await/<date>/<slug>`.
    Await { date: NaiveDate },
}

/// One inline task/await/done token found in a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedToken {
    /// Byte offset of the leading `#`.
    pub start: usize,
    /// Byte offset one past the token.
    pub end: usize,
    /// 1-based line number of `start`.
    pub line: usize,
    pub kind: ScannedTokenKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannedTokenKind {
    Task {
        priority: u32,
        slug: String,
    },
    Await {
        date: NaiveDate,
        slug: String,
    },
    Done {
        completed: NaiveDate,
        original: DoneOriginal,
        slug: String,
    },
}

impl ScannedTokenKind {
    pub fn slug(&self) -> &str {
        match self {
            ScannedTokenKind::Task { slug, .. }
            | ScannedTokenKind::Await { slug, .. }
            | ScannedTokenKind::Done { slug, .. } => slug,
        }
    }
}

impl ScannedToken {
    /// Human-readable title (de-hyphenated slug).
    pub fn title(&self) -> String {
        slug_to_title(self.kind.slug())
    }

    /// The token exactly as it appears in the body.
    pub fn raw<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

/// Scans a note body for inline tokens. A token must sit at a boundary (start
/// of text or after whitespace / an opening bracket or quote) and end at one
/// (end of text or a non-word, non-`-`, non-`/` character), matching the
/// bare-URL autolinking rule so it never fires mid-word. Results are ordered by
/// position. Only the notes grammar is recognized — *not* markdown's
/// dated-TASK form (`#task/<date>/<slug>`).
pub fn scan_note_tokens(text: &str) -> Vec<ScannedToken> {
    let bytes = text.as_bytes();
    let mut found: Vec<ScannedToken> = Vec::new();

    let mut push = |start: usize, end: usize, kind: ScannedTokenKind| {
        if !boundary_before(bytes, start) || !boundary_after(bytes, end) {
            return;
        }
        let line = 1 + text[..start].bytes().filter(|b| *b == b'\n').count();
        found.push(ScannedToken {
            start,
            end,
            line,
            kind,
        });
    };

    for cap in TASK_PRIORITY_RE.captures_iter(text) {
        let Some(m) = cap.get(0) else { continue };
        let (Some(p), Some(slug)) = (cap.get(1), cap.get(2)) else {
            continue;
        };
        let Ok(raw) = p.as_str().parse::<u32>() else {
            continue;
        };
        push(
            m.start(),
            m.end(),
            ScannedTokenKind::Task {
                priority: clamp_priority(raw),
                slug: slug.as_str().to_string(),
            },
        );
    }
    for cap in AWAIT_RE.captures_iter(text) {
        let Some(m) = cap.get(0) else { continue };
        let (Some(d), Some(slug)) = (cap.get(1), cap.get(2)) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(d.as_str(), "%Y-%m-%d") else {
            continue;
        };
        push(
            m.start(),
            m.end(),
            ScannedTokenKind::Await {
                date,
                slug: slug.as_str().to_string(),
            },
        );
    }
    for cap in DONE_TASK_RE.captures_iter(text) {
        let Some(m) = cap.get(0) else { continue };
        let (Some(c), Some(p), Some(slug)) = (cap.get(1), cap.get(2), cap.get(3)) else {
            continue;
        };
        let (Ok(completed), Ok(raw)) = (
            NaiveDate::parse_from_str(c.as_str(), "%Y-%m-%d"),
            p.as_str().parse::<u32>(),
        ) else {
            continue;
        };
        push(
            m.start(),
            m.end(),
            ScannedTokenKind::Done {
                completed,
                original: DoneOriginal::Task {
                    priority: clamp_priority(raw),
                },
                slug: slug.as_str().to_string(),
            },
        );
    }
    for cap in DONE_AWAIT_RE.captures_iter(text) {
        let Some(m) = cap.get(0) else { continue };
        let (Some(c), Some(d), Some(slug)) = (cap.get(1), cap.get(2), cap.get(3)) else {
            continue;
        };
        let (Ok(completed), Ok(date)) = (
            NaiveDate::parse_from_str(c.as_str(), "%Y-%m-%d"),
            NaiveDate::parse_from_str(d.as_str(), "%Y-%m-%d"),
        ) else {
            continue;
        };
        push(
            m.start(),
            m.end(),
            ScannedTokenKind::Done {
                completed,
                original: DoneOriginal::Await { date },
                slug: slug.as_str().to_string(),
            },
        );
    }

    found.sort_by_key(|t| t.start);
    found
}

fn boundary_before(bytes: &[u8], start: usize) -> bool {
    if start == 0 {
        return true;
    }
    matches!(
        bytes[start - 1],
        b' ' | b'\t' | b'\n' | b'\r' | b'(' | b'[' | b'{' | b'"' | b'\''
    )
}

fn boundary_after(bytes: &[u8], end: usize) -> bool {
    match bytes.get(end) {
        None => true,
        Some(b) => !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'/'),
    }
}

/// The token whose byte span strictly contains `caret_byte`
/// (`start < caret_byte < end`), so a caret placed just after the leading `#`
/// through just before the final character counts as "inside the tag" — but a
/// caret to the right of the tag's last character does not, so continuing to
/// type after a finished token never pops the edit dialog. Used by the note
/// editor to open an edit dialog instead of its `#` create chooser when the
/// caret lands in an existing token. Returns `None` at offset 0, before the
/// `#`, or at/after the end of the tag.
pub fn token_at_caret(text: &str, caret_byte: usize) -> Option<ScannedToken> {
    token_at_caret_ordinal(text, caret_byte).map(|(t, _)| t)
}

/// Like [`token_at_caret`], but also returns the token's 0-based ordinal among
/// its line's tokens — the coordinate `edit_token_at` and the chip row address
/// tokens by.
pub fn token_at_caret_ordinal(text: &str, caret_byte: usize) -> Option<(ScannedToken, usize)> {
    let tokens = scan_note_tokens(text);
    let target = tokens
        .iter()
        .find(|t| t.start < caret_byte && caret_byte < t.end)?;
    let ordinal = tokens
        .iter()
        .filter(|t| t.line == target.line)
        .position(|t| t.start == target.start)?;
    Some((target.clone(), ordinal))
}

/// Clamps a raw `#task/p<N>` priority into the app's canonical 1-100 domain
/// (1 = highest). `0` clamps to `1` (still highest); anything above 100 to
/// `100` (lowest) — matching `tidl_markdown`'s clamp so a token behaves the
/// same wherever it is parsed.
pub fn clamp_priority(raw: u32) -> u32 {
    raw.clamp(1, 100)
}

/// Convert a subject slug to human-readable format.
pub fn slug_to_title(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a free-text name into a token slug (`[a-z0-9-]+`): lowercase, every
/// non-alphanumeric run collapsed to a single `-`, trimmed. Empty when the
/// input has no alphanumeric characters — callers decide the fallback.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_dash = false;
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out
}

/// `#task/p<N>/<slug>`.
pub fn format_task_token(priority: u32, slug: &str) -> String {
    format!("#task/p{priority}/{slug}")
}

/// `#await/<YYYY-MM-DD>/<slug>`.
pub fn format_await_token(date: NaiveDate, slug: &str) -> String {
    format!("#await/{}/{slug}", date.format("%Y-%m-%d"))
}

/// `#done/<completed>/<original-spec>/<slug>`, retaining the original priority
/// (task) or date (await) so [`reopen_token`] can restore it exactly.
pub fn format_done(completed: NaiveDate, original: &DoneOriginal, slug: &str) -> String {
    let completed = completed.format("%Y-%m-%d");
    match original {
        DoneOriginal::Task { priority } => format!("#done/{completed}/p{priority}/{slug}"),
        DoneOriginal::Await { date } => {
            format!("#done/{completed}/{}/{slug}", date.format("%Y-%m-%d"))
        }
    }
}

/// The original token a [`DoneOriginal`] was completed from.
pub fn reopen_token(original: &DoneOriginal, slug: &str) -> String {
    match original {
        DoneOriginal::Task { priority } => format_task_token(*priority, slug),
        DoneOriginal::Await { date } => format_await_token(*date, slug),
    }
}

/// Strips a leading markdown unordered-list marker (`- `, `* ` or `+ `) from a
/// line that has already had its indentation removed, or `None` when the line
/// does not start with one.
fn strip_list_marker(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
}

/// The priority a [`bullets_to_checklist`] task gets — the middle of the 1-100
/// domain, matching the Keep importer and the editor's "+ Task" default.
pub const DEFAULT_TASK_PRIORITY: u32 = 50;

/// Converts every maximal run of consecutive "token-only" lines in `text` into
/// a plain markdown bullet list (backlog #130), discarding the task/await state
/// and keeping only the human-readable title: `#task/p50/buy-milk` — or a
/// `- #task/…` bullet, or a `#done/…` record — becomes `- Buy Milk`. This is
/// how an imported Google Keep checklist (whose items all came in as tokens,
/// #109/#113) is turned back into an ordinary list so it stops cluttering
/// Tasks/Focus/CalDAV.
///
/// A line is "token-only" when, ignoring leading indentation and an optional
/// list marker, its entire trimmed content is exactly one inline token. A line
/// with prose around a token (a task mentioned inside a sentence) is left
/// untouched, as is any blank or ordinary line. Indentation is preserved so a
/// nested checklist keeps its shape. Returns `None` when nothing converted.
pub fn checklist_to_bullets(text: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let indent_len = line.len() - line.trim_start_matches(&[' ', '\t'][..]).len();
        let (indent, rest) = line.split_at(indent_len);
        let rest = strip_list_marker(rest).unwrap_or(rest);
        let trimmed = rest.trim_end();
        let tokens = scan_note_tokens(trimmed);
        if tokens.len() == 1 && tokens[0].start == 0 && tokens[0].end == trimmed.len() {
            out.push_str(indent);
            out.push_str("- ");
            out.push_str(&tokens[0].title());
            changed = true;
        } else {
            out.push_str(line);
        }
    }
    changed.then_some(out)
}

/// Converts every maximal run of markdown unordered-list lines (`- `, `* ` or
/// `+ `, with indentation) in `text` into inline task tokens at
/// [`DEFAULT_TASK_PRIORITY`] (backlog #130) — the inverse of
/// [`checklist_to_bullets`]: `- Buy Milk` becomes `#task/p50/buy-milk`, so a
/// plain list written in the editor becomes real tasks in Tasks/Focus/CalDAV.
///
/// Only a bullet whose item text is plain prose converts: a bullet that already
/// contains a token (`- #task/p50/x`), or whose text slugifies to nothing (e.g.
/// `- ???`), is left untouched, as is any blank or ordinary line. The item text
/// is slugified into the task title, so inline markdown within it is dropped.
/// Indentation is preserved so a nested list keeps its shape. Returns `None`
/// when nothing converted.
pub fn bullets_to_checklist(text: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let indent_len = line.len() - line.trim_start_matches(&[' ', '\t'][..]).len();
        let (indent, rest) = line.split_at(indent_len);
        let Some(item) = strip_list_marker(rest) else {
            out.push_str(line);
            continue;
        };
        let trimmed = item.trim();
        let slug = slugify(trimmed);
        if slug.is_empty() || !scan_note_tokens(trimmed).is_empty() {
            out.push_str(line);
            continue;
        }
        out.push_str(indent);
        out.push_str(&format_task_token(DEFAULT_TASK_PRIORITY, &slug));
        changed = true;
    }
    changed.then_some(out)
}

/// Rewrite the token at `(line, ordinal)` in `text` with a new slug and
/// parameter, preserving the token's kind (backlog #121). Returns the new body
/// text, or `None` when no active task/await token sits there — a `#done`
/// record, a position with no token, or an empty `new_slug`. `new_priority` /
/// `new_date` replace the existing parameter when `Some`; a task ignores
/// `new_date` and an await ignores `new_priority`.
pub fn edit_token_at(
    text: &str,
    line: usize,
    ordinal: usize,
    new_slug: &str,
    new_priority: Option<u32>,
    new_date: Option<NaiveDate>,
) -> Option<String> {
    if new_slug.is_empty() {
        return None;
    }
    let target = scan_note_tokens(text)
        .into_iter()
        .filter(|t| t.line == line)
        .nth(ordinal)?;
    let replacement = match &target.kind {
        ScannedTokenKind::Task { priority, .. } => {
            format_task_token(new_priority.unwrap_or(*priority).clamp(1, 100), new_slug)
        }
        ScannedTokenKind::Await { date, .. } => {
            format_await_token(new_date.unwrap_or(*date), new_slug)
        }
        ScannedTokenKind::Done { .. } => return None,
    };
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..target.start]);
    out.push_str(&replacement);
    out.push_str(&text[target.end..]);
    Some(out)
}

/// Remove the token at (`line` 1-based, `ordinal` 0-based within the line) from
/// `text`, dropping the whole line when only whitespace / a list marker is left.
/// Returns the new body text, or `None` when no token sits there. Strips only
/// the token itself; surrounding prose is preserved (a doubled space at the seam
/// is collapsed). Backs the Done clean-up engine (backlog #124).
pub fn remove_token_at(text: &str, line: usize, ordinal: usize) -> Option<String> {
    let target = scan_note_tokens(text)
        .into_iter()
        .filter(|t| t.line == line)
        .nth(ordinal)?;

    let line_start = text[..target.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[target.start..]
        .find('\n')
        .map(|i| target.start + i)
        .unwrap_or(text.len());
    let before = &text[line_start..target.start];
    let mut after = &text[target.end..line_end];
    if (before.is_empty() || before.ends_with(' ')) && after.starts_with(' ') {
        after = &after[1..];
    }
    let before = if after.is_empty() {
        before.trim_end()
    } else {
        before
    };
    let mut body = String::with_capacity(before.len() + after.len());
    body.push_str(before);
    body.push_str(after);

    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..line_start]);
    if effectively_empty(&body) {
        // Drop the line and its trailing newline (when present).
        if line_end < text.len() {
            out.push_str(&text[line_end + 1..]);
        }
    } else {
        out.push_str(&body);
        out.push_str(&text[line_end..]);
    }
    Some(out)
}

/// Rewrite the `#done` token at (`line` 1-based, `ordinal` 0-based within the
/// line) back to the active token it was completed from. Returns `None` when no
/// `#done` token sits there. The mirror of [`remove_token_at`] for re-opening
/// from the Done view (backlog #124, #125).
pub fn reopen_token_at(text: &str, line: usize, ordinal: usize) -> Option<String> {
    let target = scan_note_tokens(text)
        .into_iter()
        .filter(|t| t.line == line)
        .nth(ordinal)?;
    let ScannedTokenKind::Done { original, slug, .. } = &target.kind else {
        return None;
    };
    let replacement = reopen_token(original, slug);
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..target.start]);
    out.push_str(&replacement);
    out.push_str(&text[target.end..]);
    Some(out)
}

/// Bulk [`remove_token_at`]: remove every token whose `(line, ordinal)` is in
/// `targets`. Applied highest-coordinate-first so the remaining coordinates stay
/// valid as earlier removals shrink the text.
pub fn remove_tokens(text: &str, targets: &[(usize, usize)]) -> String {
    let mut sorted: Vec<(usize, usize)> = targets.to_vec();
    sorted.sort_by(|a, b| b.cmp(a));
    let mut current = text.to_string();
    for (line, ordinal) in sorted {
        if let Some(next) = remove_token_at(&current, line, ordinal) {
            current = next;
        }
    }
    current
}

/// True when `line` carries no content once whitespace and a bare list marker
/// (optionally with a `[ ]`/`[x]` checkbox) are stripped — such a line is
/// dropped when its token is removed rather than left as an orphan bullet.
fn effectively_empty(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return true;
    }
    let after_marker = match t.as_bytes()[0] {
        b'-' | b'*' | b'+' => t[1..].trim_start(),
        _ => return false,
    };
    let after_checkbox = if after_marker.starts_with('[') {
        match after_marker.find(']') {
            Some(i) if i <= 2 => after_marker[i + 1..].trim_start(),
            _ => after_marker,
        }
    } else {
        after_marker
    };
    after_checkbox.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn slugify_examples() {
        assert_eq!(slugify("Check production now"), "check-production-now");
        assert_eq!(slugify("  Buy   MILK!! "), "buy-milk");
        assert_eq!(
            slugify("Call Bob re: invoice @ 3pm"),
            "call-bob-re-invoice-3pm"
        );
        assert_eq!(slugify("--already-slugged--"), "already-slugged");
        assert_eq!(slugify("café"), "caf");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn formats_tokens() {
        assert_eq!(format_task_token(1, "check-now"), "#task/p1/check-now");
        assert_eq!(
            format_await_token(d(2026, 12, 31), "renew"),
            "#await/2026-12-31/renew"
        );
    }

    #[test]
    fn done_round_trips() {
        let completed = d(2026, 10, 1);
        let task = DoneOriginal::Task { priority: 7 };
        let task_token = format_done(completed, &task, "buy-milk");
        assert_eq!(task_token, "#done/2026-10-01/p7/buy-milk");
        assert_eq!(reopen_token(&task, "buy-milk"), "#task/p7/buy-milk");

        let await_ = DoneOriginal::Await {
            date: d(2026, 12, 31),
        };
        let await_token = format_done(completed, &await_, "renew");
        assert_eq!(await_token, "#done/2026-10-01/2026-12-31/renew");
        assert_eq!(reopen_token(&await_, "renew"), "#await/2026-12-31/renew");
    }

    #[test]
    fn edit_token_at_rewrites_preserving_kind() {
        let text = "First\n#task/p10/buy-milk and #await/2026-12-31/renew\nlast";
        let edited = edit_token_at(text, 2, 0, "get-milk", Some(3), None).unwrap();
        assert_eq!(
            edited,
            "First\n#task/p3/get-milk and #await/2026-12-31/renew\nlast"
        );
        let edited = edit_token_at(&edited, 2, 1, "renew-pass", None, Some(d(2027, 1, 1))).unwrap();
        assert_eq!(
            edited,
            "First\n#task/p3/get-milk and #await/2027-01-01/renew-pass\nlast"
        );
    }

    #[test]
    fn edit_token_at_falls_back_and_rejects() {
        let text = "#task/p10/buy-milk";
        assert_eq!(
            edit_token_at(text, 1, 0, "x", None, None).unwrap(),
            "#task/p10/x"
        );
        assert_eq!(edit_token_at(text, 1, 0, "", Some(1), None), None);
        assert_eq!(edit_token_at(text, 1, 5, "x", Some(1), None), None);
        assert_eq!(
            edit_token_at("#done/2026-10-01/p3/buy-milk", 1, 0, "x", Some(1), None),
            None
        );
    }

    #[test]
    fn scan_kinds_and_lines() {
        let text = "First line\n#task/p10/buy-milk and #await/2026-12-31/renew\nlast";
        let toks = scan_note_tokens(text);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].line, 2);
        assert_eq!(toks[0].title(), "Buy Milk");
        assert_eq!(
            toks[0].kind,
            ScannedTokenKind::Task {
                priority: 10,
                slug: "buy-milk".into()
            }
        );
        assert!(matches!(toks[1].kind, ScannedTokenKind::Await { .. }));
    }

    #[test]
    fn scan_requires_boundaries() {
        assert!(scan_note_tokens("see#task/p1/foo").is_empty());
        assert_eq!(scan_note_tokens("#task/p1/foo").len(), 1);
        assert_eq!(scan_note_tokens("x #task/p1/foo").len(), 1);
        assert_eq!(scan_note_tokens("(#task/p1/foo)").len(), 1);
        assert!(scan_note_tokens("#task/p1/foo/bar").is_empty());
    }

    #[test]
    fn scan_ignores_dated_task_form() {
        assert!(scan_note_tokens("#task/2026-01-01/foo").is_empty());
    }

    #[test]
    fn token_at_caret_covers_the_whole_tag() {
        let text = "do #task/p001/some-task now";
        let start = text.find('#').unwrap();
        let end = start + "#task/p001/some-task".len();
        // Any caret strictly inside the tag — including right after `#` and
        // `#task` — resolves to it.
        for caret in (start + 1)..end {
            let hit = token_at_caret(text, caret).unwrap_or_else(|| panic!("caret {caret}"));
            assert_eq!(hit.start, start);
            assert_eq!(hit.end, end);
        }
        // Before the `#`, exactly at its end, or after the tag does not.
        assert!(token_at_caret(text, start).is_none());
        assert!(
            token_at_caret(text, end).is_none(),
            "a caret right after the tag must not open it"
        );
        assert!(token_at_caret(text, end + 1).is_none());
        assert!(token_at_caret(text, 0).is_none());
    }

    #[test]
    fn token_at_caret_resolves_done_and_await() {
        let text = "#done/2026-10-01/p3/old #await/2026-12-31/renew";
        let done = token_at_caret(text, 3).unwrap();
        assert!(matches!(done.kind, ScannedTokenKind::Done { .. }));
        let await_start = text.find("#await").unwrap();
        let await_tok = token_at_caret(text, await_start + 7).unwrap();
        assert!(matches!(await_tok.kind, ScannedTokenKind::Await { .. }));
    }

    #[test]
    fn scan_done_records() {
        let toks = scan_note_tokens("#done/2026-10-01/p3/foo\n#done/2026-10-02/2026-12-31/bar");
        assert_eq!(toks.len(), 2);
        assert!(matches!(
            &toks[0].kind,
            ScannedTokenKind::Done { completed, original: DoneOriginal::Task { priority: 3 }, slug }
                if *completed == d(2026, 10, 1) && slug == "foo"
        ));
        assert!(matches!(
            &toks[1].kind,
            ScannedTokenKind::Done { original: DoneOriginal::Await { date }, .. }
                if *date == d(2026, 12, 31)
        ));
    }

    #[test]
    fn scan_legacy_aliases() {
        let toks = scan_note_tokens("#todo/p1/foo #pending/2026-12-31/bar");
        assert_eq!(toks.len(), 2);
        assert!(matches!(toks[0].kind, ScannedTokenKind::Task { .. }));
        assert!(matches!(toks[1].kind, ScannedTokenKind::Await { .. }));
    }

    #[test]
    fn scan_clamps_priority() {
        let toks = scan_note_tokens("#task/p9999/foo");
        assert_eq!(
            toks[0].kind,
            ScannedTokenKind::Task {
                priority: 100,
                slug: "foo".into()
            }
        );
    }

    #[test]
    fn remove_token_at_strips_and_keeps_prose() {
        let text = "First\n- #done/2026-10-01/p3/buy-milk keep this\nlast";
        assert_eq!(
            remove_token_at(text, 2, 0).as_deref(),
            Some("First\n- keep this\nlast")
        );
    }

    #[test]
    fn checklist_converts_token_only_lines_and_preserves_prose() {
        let text = "Packing list:\n#task/p50/socks\n#done/2026-10-01/p50/passport\n\nback home #task/p1/milk now";
        assert_eq!(
            checklist_to_bullets(text).unwrap(),
            "Packing list:\n- Socks\n- Passport\n\nback home #task/p1/milk now"
        );
    }

    #[test]
    fn remove_token_at_drops_emptied_line() {
        let text = "First\n- #done/2026-10-01/p3/buy-milk\nlast";
        assert_eq!(remove_token_at(text, 2, 0).as_deref(), Some("First\nlast"));
    }

    #[test]
    fn remove_token_at_targets_the_nth_on_a_line() {
        let text = "#task/p1/one and #await/2026-12-31/two";
        assert_eq!(
            remove_token_at(text, 1, 1).as_deref(),
            Some("#task/p1/one and")
        );
        assert_eq!(remove_token_at(text, 1, 5), None);
    }

    #[test]
    fn remove_tokens_splices_multiple_lines() {
        let text = "a\n#task/p1/one\n#done/2026-10-01/p2/two b\nc";
        assert_eq!(remove_tokens(text, &[(2, 0), (3, 0)]), "a\nb\nc");
    }

    #[test]
    fn reopen_token_at_restores_the_original() {
        assert_eq!(
            reopen_token_at("#done/2026-10-01/p3/buy-milk", 1, 0).as_deref(),
            Some("#task/p3/buy-milk")
        );
        assert_eq!(
            reopen_token_at("#done/2026-10-01/2026-12-31/renew", 1, 0).as_deref(),
            Some("#await/2026-12-31/renew")
        );
        assert_eq!(reopen_token_at("#task/p1/active", 1, 0), None);
    }

    #[test]
    fn checklist_keeps_indentation_and_existing_markers() {
        let text = "  - #task/p50/socks\n\t* #await/2026-12-31/renew";
        assert_eq!(checklist_to_bullets(text).unwrap(), "  - Socks\n\t- Renew");
    }

    #[test]
    fn checklist_returns_none_when_nothing_to_convert() {
        assert_eq!(checklist_to_bullets("just prose"), None);
        assert_eq!(checklist_to_bullets("inline #task/p1/foo here"), None);
        assert_eq!(checklist_to_bullets("#task/p1/two #task/p2/in-one"), None);
        assert_eq!(checklist_to_bullets(""), None);
    }

    #[test]
    fn checklist_preserves_trailing_newline() {
        assert_eq!(
            checklist_to_bullets("#task/p50/milk\n").unwrap(),
            "- Milk\n"
        );
    }

    #[test]
    fn bullets_convert_each_list_line_to_a_default_priority_task() {
        let text = "Trip:\n- Buy milk\n* Call Bob\n+ Pack bags";
        assert_eq!(
            bullets_to_checklist(text).unwrap(),
            "Trip:\n#task/p50/buy-milk\n#task/p50/call-bob\n#task/p50/pack-bags"
        );
    }

    #[test]
    fn bullets_keep_indentation_and_skip_token_or_empty_items() {
        let text = "  - Socks\n\t* #task/p1/already\n- ???\nplain line";
        assert_eq!(
            bullets_to_checklist(text).unwrap(),
            "  #task/p50/socks\n\t* #task/p1/already\n- ???\nplain line"
        );
    }

    #[test]
    fn bullets_returns_none_when_nothing_to_convert() {
        assert_eq!(bullets_to_checklist("just prose"), None);
        assert_eq!(bullets_to_checklist("- #task/p1/foo"), None);
        assert_eq!(bullets_to_checklist("- ???"), None);
        assert_eq!(bullets_to_checklist(""), None);
    }

    #[test]
    fn bullets_preserves_trailing_newline() {
        assert_eq!(bullets_to_checklist("- Milk\n").unwrap(), "#task/p50/milk\n");
    }

    #[test]
    fn bullets_round_trips_a_bulletized_checklist() {
        let text = "#task/p50/buy-milk\n#await/2026-12-31/renew";
        let bulletized = checklist_to_bullets(text).unwrap();
        assert_eq!(
            bullets_to_checklist(&bulletized).unwrap(),
            "#task/p50/buy-milk\n#task/p50/renew"
        );
    }
}

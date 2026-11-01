//! Markdown tag parser for todo and pending items

use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};

// Compiled once at first use; the literals are valid so the unwrap is infallible.
// These match only the tag itself (`#tag/param/slug`) — no trailing-text
// capture. An item's title always comes from `slug_to_title(subject)`; any
// text after the tag on the same line is not part of the tag in any way and
// is left for the normal CommonMark pass to render on its own.
// TASK_PRIORITY_RE matches the priority-only form (#task/p<N>/<slug>);
// TASK_DATED_RE matches the dated form (#task/<date>/<slug>) with no
// explicit priority — checked second so a priority-form tag (whose middle
// segment starts with 'p') never partially matches the dated pattern.
// `#todo`/`#pending` are accepted as aliases of `#task`/`#await` so markdown
// sources written before the backlog #49 rename keep parsing unchanged.
static TASK_PRIORITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?:task|todo)/p(\d+)/([a-zA-Z0-9-]+)").unwrap());
static TASK_DATED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?:task|todo)/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap());
static AWAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#(?:await|pending)/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});
// A checked-off line — written by completing a task/await (see
// `mark_tag_done`) and rendered as a checked chip. Deliberately NOT one of
// the "active" patterns above, so a done line drops out of
// `parse_tasks`/`parse_awaits` and therefore out of every list that consumes
// them, while still rendering in a file preview.
//
// Rich done records keep the original parameters + completion date (backlog
// #117), mirroring notes' `#done` grammar; the segment after the completion
// date disambiguates on re-open: `p<N>` = priority task, `t<date>` = dated
// task, bare `<date>` = await. `DONE_LEGACY_RE` is the old slug-only record,
// kept renderable but not re-openable.
static DONE_TASK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#done/(\d{4}-\d{2}-\d{2})/p(\d+)/([a-zA-Z0-9-]+)").unwrap());
static DONE_DATED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#done/(\d{4}-\d{2}-\d{2})/t(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});
static DONE_AWAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#done/(\d{4}-\d{2}-\d{2})/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});
static DONE_LEGACY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#done/([a-zA-Z0-9-]+)").unwrap());
// Any active task/await tag, capturing which form it is so `mark_tag_done`
// can preserve the parameters into the done record: group 1 = `#task/p<N>`,
// group 2 = dated `#task/<date>`, group 3 = `#await/<date>`, group 4 = slug.
static ANY_ACTIVE_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#(?:(?:task|todo)/p(\d+)|(?:task|todo)/(\d{4}-\d{2}-\d{2})|(?:await|pending)/(\d{4}-\d{2}-\d{2}))/([a-zA-Z0-9-]+)",
    )
    .unwrap()
});
// Any **rich** `#done` record (backlog #117), capturing which active form it
// was completed from so `reopen_tag_at` can restore it: group 1 = completion
// date, 2 = priority task, 3 = dated task, 4 = await, 5 = slug. The legacy
// slug-only `#done/<slug>` record is deliberately excluded — it carries no
// original parameters and so is not re-openable.
static ANY_RICH_DONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#done/(\d{4}-\d{2}-\d{2})/(?:p(\d+)|t(\d{4}-\d{2}-\d{2})|(\d{4}-\d{2}-\d{2}))/([a-zA-Z0-9-]+)",
    )
    .unwrap()
});

// Triple-bracket must be matched before double-bracket to avoid partial replacement.
static TRIPLE_WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[\[([^\]]+)\]\]\]").unwrap());
static WIKILINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    /// `None` for the dated-only tag form (`#task/<date>/<slug>`).
    pub priority: Option<u32>,
    /// `Some` only when this came from the dated tag form.
    pub due_date: Option<NaiveDate>,
    /// Topic slug, e.g. `"sales"`.
    pub subject: String,
    /// Human-readable title, always derived from `slug_to_title(subject)`.
    /// Any text after the tag on the same line is not part of the tag in
    /// any way — it's ordinary markdown content and never affects this.
    pub title: String,
    pub line: usize,
    /// 0-based index of this tag among the line's *active* tags (task, dated
    /// task, and await together, in source order). Several tags can share a
    /// line; this is what makes their `events` uids unique and lets the
    /// rendered preview target the right chip. Matches the `data-md-ordinal`
    /// `process_task_tags` stamps onto the rendered chip and the ordinal
    /// [`mark_tag_done_at`] counts.
    pub ordinal: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwaitItem {
    pub date: NaiveDate,
    /// Topic slug, e.g. `"sales"`.
    pub subject: String,
    /// Human-readable title, always derived from `slug_to_title(subject)`.
    /// Any text after the tag on the same line is not part of the tag in
    /// any way — it's ordinary markdown content and never affects this.
    pub title: String,
    pub line: usize,
    /// See [`TaskItem::ordinal`] — the same line-wide active-tag index.
    pub ordinal: usize,
    pub context: String,
}

/// Clamps a raw `#task/p<N>` priority into the app's canonical 1-100 task
/// priority domain (1 = highest, shared by notes' slider and CalDAV's
/// RFC 5545 PRIORITY mapping — backlog #70 / 2026-07-30 investigation).
/// `0` clamps to `1` (still highest) rather than being rejected, so
/// pre-existing `#task/p0/<slug>` tags keep behaving exactly like
/// `#task/p1/<slug>`. Logs a warning on any out-of-range input so a typo'd
/// tag (e.g. a stray extra digit) doesn't silently produce a different
/// priority than the author intended.
fn clamp_markdown_priority(raw: u32, context: &str) -> u32 {
    if raw == 0 {
        tracing::warn!(
            raw,
            context,
            "markdown task priority 0 clamped to 1 (highest)"
        );
        1
    } else if raw > 100 {
        tracing::warn!(
            raw,
            context,
            "markdown task priority above 100 clamped to 100 (lowest)"
        );
        100
    } else {
        raw
    }
}

#[derive(Debug, Clone, Default)]
pub struct MarkdownParser;

impl MarkdownParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse TASK items (both `#task/p<N>/<slug>` and `#task/<date>/<slug>`) from markdown content.
    pub fn parse_tasks(&self, content: &str) -> Vec<TaskItem> {
        self.scan_all(content).0
    }

    /// Parse AWAIT items from markdown content
    pub fn parse_awaits(&self, content: &str) -> Vec<AwaitItem> {
        self.scan_all(content).1
    }

    /// Parse both TASK and AWAIT items
    pub fn parse_all(&self, content: &str) -> (Vec<TaskItem>, Vec<AwaitItem>) {
        self.scan_all(content)
    }

    /// The one pass behind [`parse_tasks`]/[`parse_awaits`]/[`parse_all`].
    /// Scans each line with the same source-order tag scanner the renderer
    /// uses, so a tag's `ordinal` here is exactly the `data-md-ordinal` its
    /// rendered chip carries.
    fn scan_all(&self, content: &str) -> (Vec<TaskItem>, Vec<AwaitItem>) {
        let mut tasks = Vec::new();
        let mut awaits = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            for tag in scan_line_tags(line) {
                let line_number = line_num + 1;
                match tag.kind {
                    LineTagKind::Priority { priority, slug } => tasks.push(TaskItem {
                        priority: Some(priority),
                        due_date: None,
                        title: slug_to_title(&slug),
                        subject: slug,
                        line: line_number,
                        ordinal: tag.active_ord.unwrap_or(0),
                        context: line.to_string(),
                    }),
                    LineTagKind::Dated { date, slug } => {
                        if let Ok(due_date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
                            tasks.push(TaskItem {
                                priority: None,
                                due_date: Some(due_date),
                                title: slug_to_title(&slug),
                                subject: slug,
                                line: line_number,
                                ordinal: tag.active_ord.unwrap_or(0),
                                context: line.to_string(),
                            });
                        }
                    }
                    LineTagKind::Await { date, slug } => {
                        if let Ok(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
                            awaits.push(AwaitItem {
                                date,
                                title: slug_to_title(&slug),
                                subject: slug,
                                line: line_number,
                                ordinal: tag.active_ord.unwrap_or(0),
                                context: line.to_string(),
                            });
                        }
                    }
                    LineTagKind::DoneTask { .. }
                    | LineTagKind::DoneDated { .. }
                    | LineTagKind::DoneAwait { .. }
                    | LineTagKind::DoneLegacy { .. } => {}
                }
            }
        }

        (tasks, awaits)
    }
}

/// Rewrites the **first** active task/await tag in `line` to a rich `#done`
/// record that keeps the original parameters plus the completion date
/// (backlog #117), mirroring notes' `#done` grammar:
///
/// - `#task/p<N>/<slug>`      → `#done/<c>/p<N>/<slug>`
/// - `#task/<date>/<slug>`    → `#done/<c>/t<date>/<slug>`
/// - `#await/<date>/<slug>`   → `#done/<c>/<date>/<slug>`
///
/// A line with no active tag (or an already-`#done` one) is returned
/// unchanged. Pure string transform; the caller owns file I/O.
pub fn mark_tag_done(line: &str, completed: NaiveDate) -> String {
    mark_tag_done_at(line, completed, 0)
}

/// Like [`mark_tag_done`], but rewrites the `ordinal`-th active tag on the
/// line (0-based, in source order) — so a line holding several tags completes
/// exactly the one whose checkbox was clicked. Matches the ordinals
/// `process_task_tags` stamps onto each chip as `data-md-ordinal`.
pub fn mark_tag_done_at(line: &str, completed: NaiveDate, ordinal: usize) -> String {
    let completed = completed.format("%Y-%m-%d").to_string();
    let mut out = String::with_capacity(line.len());
    let mut last = 0usize;
    for (seen, caps) in ANY_ACTIVE_TAG_RE.captures_iter(line).enumerate() {
        if seen != ordinal {
            continue;
        }
        let Some(m) = caps.get(0) else { continue };
        out.push_str(&line[last..m.start()]);
        let slug = &caps[4];
        let replacement = if let Some(p) = caps.get(1) {
            format!("#done/{completed}/p{}/{slug}", p.as_str())
        } else if let Some(d) = caps.get(2) {
            format!("#done/{completed}/t{}/{slug}", d.as_str())
        } else {
            format!("#done/{completed}/{}/{slug}", &caps[3])
        };
        out.push_str(&replacement);
        last = m.end();
        break;
    }
    out.push_str(&line[last..]);
    out
}

/// Reverses [`mark_tag_done`]: rewrites the first **rich** `#done` record in
/// `line` back to the active tag it was completed from (backlog #117).
/// Returns `None` when there is nothing re-openable — a line with no done tag,
/// or only a legacy `#done/<slug>` record (whose original parameters were
/// lost). Pure string transform; the caller owns file I/O.
pub fn reopen_tag(line: &str) -> Option<String> {
    reopen_tag_at(line, 0)
}

/// Like [`reopen_tag`], but rewrites the `ordinal`-th **rich** `#done` record
/// on the line (0-based, in source order) — the mirror of
/// [`mark_tag_done_at`] for a line holding several done chips.
pub fn reopen_tag_at(line: &str, ordinal: usize) -> Option<String> {
    for (seen, caps) in ANY_RICH_DONE_RE.captures_iter(line).enumerate() {
        if seen != ordinal {
            continue;
        }
        let m = caps.get(0)?;
        let slug = caps.get(5)?.as_str();
        let active = if let Some(p) = caps.get(2) {
            format!("#task/p{}/{slug}", p.as_str())
        } else if let Some(d) = caps.get(3) {
            format!("#task/{}/{slug}", d.as_str())
        } else {
            format!("#await/{}/{slug}", caps.get(4)?.as_str())
        };
        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..m.start()]);
        out.push_str(&active);
        out.push_str(&line[m.end()..]);
        return Some(out);
    }
    None
}

/// Rewrites the `ordinal`-th active task/await tag in `line` in place (backlog
/// #121), preserving the tag's textual form: a priority task keeps its
/// `#task/p<N>/` shape with the new priority, a dated task keeps its
/// `#task/<date>/` shape with the new date, and an await keeps `#await/<date>/`.
//
// `new_slug` is the new topic slug; `new_priority`/`new_date` replace the
// tag's parameter (falling back to the existing one when `None`). An
// out-of-range ordinal, a line with no active tag at that ordinal, or an empty
// `new_slug` returns `line` unchanged. Pure string transform; the caller owns
// file I/O.
pub fn edit_tag_at(
    line: &str,
    ordinal: usize,
    new_slug: &str,
    new_priority: Option<u32>,
    new_date: Option<NaiveDate>,
) -> String {
    if new_slug.is_empty() {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + new_slug.len());
    let mut last = 0usize;
    for (seen, caps) in ANY_ACTIVE_TAG_RE.captures_iter(line).enumerate() {
        if seen != ordinal {
            continue;
        }
        let Some(m) = caps.get(0) else { continue };
        out.push_str(&line[last..m.start()]);
        let replacement = if let Some(p) = caps.get(1) {
            let priority = new_priority
                .map(|p| p.clamp(1, 100).to_string())
                .unwrap_or_else(|| p.as_str().to_string());
            format!("#task/p{priority}/{new_slug}")
        } else if let Some(d) = caps.get(2) {
            let date = new_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| d.as_str().to_string());
            format!("#task/{date}/{new_slug}")
        } else if let Some(d) = caps.get(3) {
            let date = new_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| d.as_str().to_string());
            format!("#await/{date}/{new_slug}")
        } else {
            caps.get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        };
        out.push_str(&replacement);
        last = m.end();
        break;
    }
    out.push_str(&line[last..]);
    out
}

// ── Done-tag enumeration + removal (backlog #124) ──────────────────────────────

/// Which active form a rich `#done` record was completed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneKind {
    /// `#done/<c>/p<N>/<slug>`.
    Task,
    /// `#done/<c>/t<date>/<slug>` — a dated task.
    DatedTask,
    /// `#done/<c>/<date>/<slug>`.
    Await,
}

/// One rich `#done` record found in a markdown source (backlog #124).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneTag {
    /// 1-based line number.
    pub line: usize,
    /// 0-based index among the line's rich `#done` records — the same ordinal
    /// [`reopen_tag_at`] / [`mark_tag_done_at`] count.
    pub ordinal: usize,
    pub kind: DoneKind,
    pub slug: String,
    pub completed: NaiveDate,
    /// Original priority (priority task only).
    pub priority: Option<u32>,
    /// Original date (dated task or await).
    pub due_date: Option<NaiveDate>,
}

/// Enumerate every rich `#done` record in `content`. Legacy `#done/<slug>`
/// records are skipped — they carry no original parameters, so they are neither
/// re-openable nor represented in the Done index.
pub fn scan_done_tags(content: &str) -> Vec<DoneTag> {
    let mut out = Vec::new();
    for (i, line) in content.split('\n').enumerate() {
        for (ordinal, caps) in ANY_RICH_DONE_RE.captures_iter(line).enumerate() {
            let (Some(cap_completed), Some(slug)) = (caps.get(1), caps.get(5)) else {
                continue;
            };
            let Ok(completed) = NaiveDate::parse_from_str(cap_completed.as_str(), "%Y-%m-%d")
            else {
                continue;
            };
            let (kind, priority, due_date) = if let Some(p) = caps.get(2) {
                (
                    DoneKind::Task,
                    p.as_str().parse::<u32>().ok().map(|v| v.clamp(1, 100)),
                    None,
                )
            } else if let Some(d) = caps.get(3) {
                (
                    DoneKind::DatedTask,
                    None,
                    NaiveDate::parse_from_str(d.as_str(), "%Y-%m-%d").ok(),
                )
            } else if let Some(d) = caps.get(4) {
                (
                    DoneKind::Await,
                    None,
                    NaiveDate::parse_from_str(d.as_str(), "%Y-%m-%d").ok(),
                )
            } else {
                continue;
            };
            out.push(DoneTag {
                line: i + 1,
                ordinal,
                kind,
                slug: slug.as_str().to_string(),
                completed,
                priority,
                due_date,
            });
        }
    }
    out
}

/// Remove the `ordinal`-th tag from `line` — the `ordinal`-th *active* tag when
/// `done` is `false`, or the `ordinal`-th *rich done* record when `true`.
/// Returns `None` when no such tag sits there. Strips only the tag itself; any
/// surrounding prose on the line is left intact (a doubled space where the tag
/// sat is collapsed).
pub fn remove_tag_at(line: &str, ordinal: usize, done: bool) -> Option<String> {
    let tags = scan_line_tags(line);
    let target = tags.iter().find(|t| {
        if done {
            t.done_ord == Some(ordinal)
        } else {
            t.active_ord == Some(ordinal)
        }
    })?;
    Some(remove_span(line, target.start, target.end))
}

/// Remove the given rich done tags — each a `(1-based line, 0-based done
/// ordinal)` — from `content`, dropping any line left effectively empty (only
/// whitespace or an empty list marker). Returns the new content and the number
/// of tags removed. Line-ending style (LF/CRLF) is preserved for kept lines.
pub fn remove_done_tags(content: &str, targets: &[(usize, usize)]) -> (String, usize) {
    use std::collections::HashSet;
    let set: HashSet<(usize, usize)> = targets.iter().copied().collect();
    let mut out = String::with_capacity(content.len());
    let mut removed = 0usize;
    for (i, segment) in content.split_inclusive('\n').enumerate() {
        let line_no = i + 1;
        let (body, ending) = match segment.strip_suffix('\n') {
            Some(rest) => match rest.strip_suffix('\r') {
                Some(r) => (r, "\r\n"),
                None => (rest, "\n"),
            },
            None => (segment, ""),
        };
        let spans: Vec<(usize, usize)> = scan_line_tags(body)
            .into_iter()
            .filter_map(|t| t.done_ord.map(|o| (o, t.start, t.end)))
            .filter(|(o, _, _)| set.contains(&(line_no, *o)))
            .map(|(_, s, e)| (s, e))
            .collect();
        if spans.is_empty() {
            out.push_str(segment);
            continue;
        }
        removed += spans.len();
        let new_body = remove_spans(body, &spans);
        if !effectively_empty(&new_body) {
            out.push_str(&new_body);
            out.push_str(ending);
        }
    }
    (out, removed)
}

/// Remove `spans` (ascending, non-overlapping) from `line`, collapsing a single
/// space at each seam so removing a tag never leaves a doubled space.
fn remove_spans(line: &str, spans: &[(usize, usize)]) -> String {
    let mut sorted: Vec<(usize, usize)> = spans.to_vec();
    sorted.sort_unstable();
    let mut out = String::with_capacity(line.len());
    let mut last = 0usize;
    for (s, e) in sorted {
        out.push_str(&line[last..s]);
        last = e;
        if (out.is_empty() || out.ends_with(' ')) && line[last..].starts_with(' ') {
            last += 1;
        }
    }
    let tail = &line[last..];
    if tail.is_empty() && out.ends_with(' ') {
        out.pop();
    }
    out.push_str(tail);
    out
}

/// Remove a single `[start, end)` span from `line`, collapsing a doubled space
/// at the seam.
fn remove_span(line: &str, start: usize, end: usize) -> String {
    let before = &line[..start];
    let mut after = &line[end..];
    if (before.is_empty() || before.ends_with(' ')) && after.starts_with(' ') {
        after = &after[1..];
    }
    let before = if after.is_empty() {
        before.trim_end()
    } else {
        before
    };
    let mut out = String::with_capacity(line.len());
    out.push_str(before);
    out.push_str(after);
    out
}

/// True when `line` carries no content once whitespace and a bare list marker
/// (optionally with a `[ ]`/`[x]` checkbox) are stripped — such a line is
/// dropped when its tag is removed rather than left as an orphan bullet.
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

/// Render a markdown string to an HTML string.
///
/// Handles YAML frontmatter (rendered as a meta block), `[[wiki links]]` and
/// `[[[wiki links]]]` (rendered as styled spans), `@mention` syntax against
/// `contact_names` (backlog #91 — pass `&[]` when the caller has no contact
/// list, e.g. a preview with no authenticated user), GFM extensions, and
/// standard CommonMark. The caller is responsible for sanitising the output
/// if the content is untrusted.
pub fn render_to_html(content: &str, contact_names: &[String]) -> String {
    render_to_html_with_urgency(content, contact_names, &default_urgency_color)
}

/// Like [`render_to_html`], but resolves each task/await chip's urgency colour
/// through `urgency_color` (canonical 1-100 priority → CSS colour). The web
/// layer passes the user's configured priority curve here so a rendered
/// markdown chip matches the colour Notes/Focus show for the same priority;
/// callers without curves can use [`render_to_html`].
pub fn render_to_html_with_urgency(
    content: &str,
    contact_names: &[String],
    urgency_color: &dyn Fn(u32) -> &'static str,
) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let (frontmatter_html, body, body_line_offset) = extract_frontmatter(content);
    // Mentions run *before* wikilinks: render_mentions_in_text escapes every
    // character of its input (matching the plain-text-in, HTML-out contract
    // Notes needs from it), which would double-escape the raw <button>
    // markup process_wiki_links already emitted if it ran second. Safe in
    // this order since the two syntaxes use disjoint trigger characters
    // ("@" vs "[[") and neither's generated HTML can contain the other's
    // trigger.
    let body = process_mentions(body, contact_names);
    let body = process_wiki_links(&body);
    let body = process_task_tags(&body, body_line_offset, urgency_color);
    let parser = Parser::new_ext(&body, Options::all());
    let mut body_html = String::new();
    html::push_html(&mut body_html, parser);
    format!("{}{}", frontmatter_html, body_html)
}

/// Renders every `#task`/`#todo`/`#pending`/`#await`/`#done` tag on a line as
/// the shared `note-token` chip (backlog #116, the same look a note-body token
/// uses) — each carrying a checkbox (backlog #112), urgency shown by a round
/// dot and outline coloured via `urgency_color`, and `#done` as a checked-off
/// one — instead of raw hashtag text.
///
/// A line whose first non-space, non-list-marker content is a tag keeps the
/// block-level `<div>` wrapper: without it, consecutive tag-only lines with no
/// blank line between them are one CommonMark paragraph and would get glued
/// onto a single rendered line. Any tag that shares its line with other text
/// (before it, or after the leading one) is emitted as an inline chip instead,
/// so surrounding prose — and a list marker — flows normally around it.
///
/// Tags inside fenced or inline code are left literal, so a note documenting
/// the syntax (`` `#task/p1/foo` ``) doesn't render a live chip.
///
/// `line_offset` is the number of lines consumed by frontmatter before
/// `content` starts (0 if there was none), so the `data-md-line` attribute
/// stamped on each chip (backlog #74) lines up with `TaskItem`/`AwaitItem`'s
/// `line` field, which is always 1-indexed against the *original*,
/// frontmatter-included file content.
fn process_task_tags(
    content: &str,
    line_offset: usize,
    urgency_color: &dyn Fn(u32) -> &'static str,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut fence: Option<char> = None;
    for (i, line) in content.lines().enumerate() {
        let line_num = line_offset + i + 1;

        // Inside (or on the boundary of) a fenced code block, copy verbatim.
        let trimmed = line.trim_start();
        let fence_char = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = fence_char {
            let run = trimmed.chars().take_while(|ch| *ch == c).count();
            if run >= 3 {
                // A backtick fence's info string may not contain a backtick,
                // so a run closed later on the same line is an inline code
                // span, not a fence opening (`` ```cmd``` ``). Treating it as
                // an opening would leave the tracker open and swallow every
                // following line as code.
                let inline_span = fence.is_none() && c == '`' && trimmed[run..].contains('`');
                if !inline_span {
                    match fence {
                        Some(open) if open == c => fence = None,
                        None => fence = Some(c),
                        _ => {}
                    }
                    out.push_str(line);
                    out.push('\n');
                    continue;
                }
            }
        }
        if fence.is_some() {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let tags = scan_line_tags(line);

        // The line's first tag is "leading" when it is also the first
        // non-space, non-list-marker content — i.e. `line` reduced by its
        // leading whitespace and marker starts with the tag — and it is a
        // real tag (not one inside inline code or a malformed `…/foo/bar`).
        let leading_start = line.len() - strip_list_marker(line.trim_start()).len();
        match tags.iter().find(|t| {
            t.start == leading_start
                && tag_boundary_after(line, t.end)
                && !inside_inline_code(line, t.start)
        }) {
            Some(first) => {
                out.push('\n');
                out.push_str("<div class=\"md-tag-line\">");
                out.push_str(&tag_chip_html(first, line_num, urgency_color));
                out.push_str("</div>\n\n");
                let rest = render_tags_in(line, &tags, first.end, line_num, urgency_color);
                let rest = rest.trim();
                if !rest.is_empty() {
                    out.push_str(rest);
                    out.push('\n');
                }
            }
            None => {
                out.push_str(&render_tags_in(line, &tags, 0, line_num, urgency_color));
                out.push('\n');
            }
        }
    }
    out
}

/// One task/await/done tag found in a rendered line, with its byte span and
/// the ordinal the preview checkbox uses to target exactly this tag when a
/// line holds several — the same 0-based "which one on this line" scheme notes
/// use for their inline tokens.
struct LineTag {
    start: usize,
    end: usize,
    kind: LineTagKind,
    /// 0-based index among the line's *active* tags (completion target).
    active_ord: Option<usize>,
    /// 0-based index among the line's *rich* `#done` tags (reopen target).
    done_ord: Option<usize>,
}

enum LineTagKind {
    Priority { priority: u32, slug: String },
    Dated { date: String, slug: String },
    Await { date: String, slug: String },
    DoneTask { completed: String, slug: String },
    DoneDated { completed: String, slug: String },
    DoneAwait { completed: String, slug: String },
    DoneLegacy { slug: String },
}

fn push_candidate(candidates: &mut Vec<LineTag>, cap: &regex::Captures, kind: LineTagKind) {
    if let Some(m) = cap.get(0) {
        candidates.push(LineTag {
            start: m.start(),
            end: m.end(),
            kind,
            active_ord: None,
            done_ord: None,
        });
    }
}

/// Find every tag on `line` in source order. Candidates from the individual
/// regexes are sorted by position (longest first at a shared start, so a rich
/// `#done` record beats the legacy `#done/<slug>` prefix it contains) and
/// overlapping ones dropped. Active and rich-done tags then get their
/// per-line ordinals, matching what [`mark_tag_done_at`]/[`reopen_tag_at`]
/// count so a chip's `data-md-ordinal` selects exactly that tag.
fn scan_line_tags(line: &str) -> Vec<LineTag> {
    let mut candidates: Vec<LineTag> = Vec::new();

    for cap in TASK_PRIORITY_RE.captures_iter(line) {
        if let (Some(p), Some(slug)) = (cap.get(1), cap.get(2)) {
            if let Ok(raw) = p.as_str().parse::<u32>() {
                push_candidate(
                    &mut candidates,
                    &cap,
                    LineTagKind::Priority {
                        priority: clamp_markdown_priority(raw, line),
                        slug: slug.as_str().to_string(),
                    },
                );
            }
        }
    }
    for cap in TASK_DATED_RE.captures_iter(line) {
        if let (Some(d), Some(slug)) = (cap.get(1), cap.get(2)) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::Dated {
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }
    for cap in AWAIT_RE.captures_iter(line) {
        if let (Some(d), Some(slug)) = (cap.get(1), cap.get(2)) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::Await {
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }
    for cap in DONE_TASK_RE.captures_iter(line) {
        if let (Some(c), Some(slug)) = (cap.get(1), cap.get(3)) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::DoneTask {
                    completed: c.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }
    for cap in DONE_DATED_RE.captures_iter(line) {
        if let (Some(c), Some(slug)) = (cap.get(1), cap.get(3)) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::DoneDated {
                    completed: c.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }
    for cap in DONE_AWAIT_RE.captures_iter(line) {
        if let (Some(c), Some(slug)) = (cap.get(1), cap.get(3)) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::DoneAwait {
                    completed: c.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }
    for cap in DONE_LEGACY_RE.captures_iter(line) {
        if let Some(slug) = cap.get(1) {
            push_candidate(
                &mut candidates,
                &cap,
                LineTagKind::DoneLegacy {
                    slug: slug.as_str().to_string(),
                },
            );
        }
    }

    candidates.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));

    let mut tags: Vec<LineTag> = Vec::new();
    let mut last_end = 0usize;
    for t in candidates {
        if t.start >= last_end {
            last_end = t.end;
            tags.push(t);
        }
    }

    let mut active = 0usize;
    let mut done = 0usize;
    for t in &mut tags {
        match t.kind {
            LineTagKind::Priority { .. }
            | LineTagKind::Dated { .. }
            | LineTagKind::Await { .. } => {
                t.active_ord = Some(active);
                active += 1;
            }
            LineTagKind::DoneTask { .. }
            | LineTagKind::DoneDated { .. }
            | LineTagKind::DoneAwait { .. } => {
                t.done_ord = Some(done);
                done += 1;
            }
            LineTagKind::DoneLegacy { .. } => {}
        }
    }
    tags
}

/// Render every boundary-respecting tag in `line` as an inline chip, starting
/// at byte offset `from`, leaving the text between them untouched. Tags that
/// fail the word-boundary test (e.g. `see#task/p1/foo`) or sit inside inline
/// code (`` `#task/p1/foo` ``) stay literal, matching the notes scanner — but
/// they still occupy an ordinal, so a rendered tag's `data-md-ordinal` keeps
/// pointing at the same tag the server counts.
fn render_tags_in(
    line: &str,
    tags: &[LineTag],
    from: usize,
    line_num: usize,
    urgency_color: &dyn Fn(u32) -> &'static str,
) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last = from;
    for t in tags {
        if t.end <= from {
            continue;
        }
        if !tag_boundary_before(line, t.start)
            || !tag_boundary_after(line, t.end)
            || inside_inline_code(line, t.start)
        {
            continue;
        }
        out.push_str(&line[last..t.start]);
        out.push_str(&tag_chip_html(t, line_num, urgency_color));
        last = t.end;
    }
    out.push_str(&line[last..]);
    out
}

/// Whether byte offset `pos` falls inside a backtick-delimited inline code
/// span on `line`. Runs of backticks open/close by matching length; a tag
/// inside the span is left as literal code.
fn inside_inline_code(line: &str, pos: usize) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut open: Option<usize> = None;
    while i < pos {
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let len = i - start;
            match open {
                Some(l) if l == len => open = None,
                None => open = Some(len),
                Some(_) => {}
            }
        } else {
            i += 1;
        }
    }
    open.is_some()
}

/// A tag must start the text or follow whitespace / an opening bracket or
/// quote, mirroring the notes scanner so a token never fires mid-word.
fn tag_boundary_before(line: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    matches!(
        line.as_bytes()[start - 1],
        b' ' | b'\t' | b'\n' | b'\r' | b'(' | b'[' | b'{' | b'"' | b'\''
    )
}

/// A tag must end the text or be followed by a non-word, non-`-`, non-`/`
/// character, so `#task/p1/foo/bar` is not treated as a `#task/p1/foo` tag.
fn tag_boundary_after(line: &str, end: usize) -> bool {
    match line.as_bytes().get(end) {
        None => true,
        Some(b) => !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'/'),
    }
}

fn strip_list_marker(line: &str) -> &str {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest;
        }
    }
    line
}

// `data-md-line` (backlog #74) lets the preview modal find and pulse-highlight
// this specific pill via `[data-md-line='N']` — scoped to just the chip span,
// so the flash never bleeds onto the rest of the rendered line.
// `data-md-ordinal` (0-based, in source order) disambiguates when a line holds
// several tags, so completing/reopening targets exactly the clicked one.
//
// The checkbox (backlog #112) is the click target the preview's delegated
// handler looks for; it walks up to the enclosing span's `data-md-line` /
// `data-md-ordinal` to know which source tag to rewrite. It deliberately has
// no `data-md-line` of its own — the pulse query above would otherwise match
// two elements per line.
fn priority_chip_html(
    priority: u32,
    subject: &str,
    line_num: usize,
    color: &str,
    ordinal: usize,
) -> String {
    let title = escape_html(&slug_to_title(subject));
    // Same shared chip skeleton as a note-body token (backlog #116): urgency
    // is the round dot + outline (`--chip-color`), the text stays the normal
    // readable colour, and the tier word is gone. The exact priority lives in
    // the tooltip. `data-md-line` and `md-task-checkbox` keep the preview's
    // pulse/complete hooks.
    format!(
        "<span class=\"note-token note-token--task\" data-md-line=\"{line_num}\" data-md-ordinal=\"{ordinal}\" style=\"--chip-color:{color};\" title=\"{title} (priority {priority})\"><input type=\"checkbox\" class=\"md-task-checkbox\" /><span class=\"note-token-dot\"></span><span class=\"note-token-label\">{title}</span></span>"
    )
}

fn due_chip_html(date: &str, subject: &str, line_num: usize, ordinal: usize) -> String {
    let title = escape_html(&slug_to_title(subject));
    format!(
        "<span class=\"note-token note-token--await\" data-md-line=\"{line_num}\" data-md-ordinal=\"{ordinal}\" title=\"{title} (due {date})\"><input type=\"checkbox\" class=\"md-task-checkbox\" /><span class=\"note-token-dot\"></span><span class=\"note-token-label\">{title}</span></span>"
    )
}

/// An `#await` chip: like `due_chip_html` but with **no checkbox** — an await
/// simply passes when its date rolls over, it is never "completed" — and the
/// date shown inline after the label.
fn await_chip_html(date: &str, subject: &str, line_num: usize, ordinal: usize) -> String {
    let title = escape_html(&slug_to_title(subject));
    format!(
        "<span class=\"note-token note-token--await\" data-md-line=\"{line_num}\" data-md-ordinal=\"{ordinal}\" title=\"{title} (due {date})\"><span class=\"note-token-dot\"></span><span class=\"note-token-label\">{title}</span><span class=\"note-token-date\">{date}</span></span>"
    )
}

/// A checked-off `#done` chip — a checked box plus a muted, struck-through
/// label. A **rich** record (`completed` is `Some`, backlog #117) keeps a live
/// checkbox carrying `data-md-reopen`, which the preview's delegated handler
/// uses to restore the original tag; a **legacy** `#done/<slug>` record (no
/// original parameters to restore) keeps the box `disabled`, so the browser
/// never dispatches a click for it (and so needs no ordinal).
fn done_chip_html(slug: &str, line_num: usize, completed: Option<&str>, ordinal: usize) -> String {
    let title = escape_html(&slug_to_title(slug));
    match completed {
        Some(c) => format!(
            "<span class=\"note-token note-token--done\" data-md-line=\"{line_num}\" data-md-ordinal=\"{ordinal}\" title=\"Done {c}\"><input type=\"checkbox\" class=\"md-task-checkbox\" checked data-md-reopen=\"1\" /><span class=\"note-token-label\">{title}</span></span>"
        ),
        None => format!(
            "<span class=\"note-token note-token--done\" data-md-line=\"{line_num}\" title=\"Done\"><input type=\"checkbox\" class=\"md-task-checkbox\" checked disabled /><span class=\"note-token-label\">{title}</span></span>"
        ),
    }
}

/// Render one scanned tag as its chip span.
fn tag_chip_html(
    t: &LineTag,
    line_num: usize,
    urgency_color: &dyn Fn(u32) -> &'static str,
) -> String {
    match &t.kind {
        LineTagKind::Priority { priority, slug } => {
            let color = urgency_color(*priority);
            priority_chip_html(*priority, slug, line_num, color, t.active_ord.unwrap_or(0))
        }
        LineTagKind::Dated { date, slug } => {
            due_chip_html(date, slug, line_num, t.active_ord.unwrap_or(0))
        }
        LineTagKind::Await { date, slug } => {
            await_chip_html(date, slug, line_num, t.active_ord.unwrap_or(0))
        }
        LineTagKind::DoneTask { completed, slug }
        | LineTagKind::DoneDated { completed, slug }
        | LineTagKind::DoneAwait { completed, slug } => {
            done_chip_html(slug, line_num, Some(completed), t.done_ord.unwrap_or(0))
        }
        LineTagKind::DoneLegacy { slug } => done_chip_html(slug, line_num, None, 0),
    }
}

/// The urgency colour for a canonical 1-100 priority (1 = highest) using a
/// copy of the **default** `source_priority` curve. Mirrors
/// `tidl_web::types::PriorityCurves::default()` +
/// `components::focus::{task_priority_score, urgency_color}`; duplicated
/// rather than shared because tidl-markdown must not depend on tidl-web
/// (CLAUDE.md §6 dependency direction). The web layer overrides this with the
/// user's real curve via [`render_to_html_with_urgency`]. KEEP IN SYNC with
/// `types.rs`'s default curve and `focus::urgency_color`'s thresholds.
pub fn default_urgency_color(priority: u32) -> &'static str {
    let score = eval_curve(&DEFAULT_SOURCE_PRIORITY, priority.clamp(1, 100) as f64);
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

/// `PriorityCurves::default().source_priority` — see `default_urgency_color`.
const DEFAULT_SOURCE_PRIORITY: [(f64, f64); 7] = [
    (-0.83, 100.0),
    (4.69, 96.9),
    (11.01, 73.01),
    (20.48, 46.34),
    (44.69, 9.68),
    (69.17, 5.79),
    (105.22, 1.9),
];

/// Linear interpolation between the two bracketing points; flat
/// extrapolation past either end; result clamped to 0-100. Mirrors
/// `focus::eval_curve` for the copy above. `points` is sorted by `x` with at
/// least 2 entries (a private constant, so the invariant holds).
fn eval_curve(points: &[(f64, f64)], x: f64) -> u8 {
    let first = points[0];
    if x <= first.0 {
        return first.1.round().clamp(0.0, 100.0) as u8;
    }
    let last = points[points.len() - 1];
    if x >= last.0 {
        return last.1.round().clamp(0.0, 100.0) as u8;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x >= a.0 && x <= b.0 {
            let t = if b.0 > a.0 {
                (x - a.0) / (b.0 - a.0)
            } else {
                0.0
            };
            let y = a.1 + t * (b.1 - a.1);
            return y.round().clamp(0.0, 100.0) as u8;
        }
    }
    last.1.round().clamp(0.0, 100.0) as u8
}

/// Split optional YAML frontmatter off the start of `content`.
/// Returns `(fm_text, body, line_offset)` where `line_offset` is the number
/// of lines consumed before `body` starts — needed so line numbers computed
/// against `body` (e.g. for `data-md-line`, backlog #74) can be translated
/// back to line numbers in the original, frontmatter-included file content.
/// `None` when there is no frontmatter.
fn split_frontmatter(content: &str) -> Option<(&str, &str, usize)> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;

    // Locate the closing `---` on its own line.
    let (fm_end, body_start) = if let Some(pos) = rest.find("\n---\n") {
        (pos, pos + 5)
    } else if let Some(pos) = rest.find("\n---\r\n") {
        (pos, pos + 6)
    } else if let Some(pos) = rest.find("\n---") {
        // closing `---` at end of file without trailing newline
        let after = pos + 4;
        if after >= rest.len() {
            (pos, rest.len())
        } else {
            return None;
        }
    } else {
        return None;
    };

    let fm_text = &rest[..fm_end];
    let body = &rest[body_start..];
    let consumed = &content[..content.len() - body.len()];
    let line_offset = consumed.matches('\n').count();
    Some((fm_text, body, line_offset))
}

/// Strip and render YAML frontmatter delimited by `---` at the start of the file.
/// Returns `(frontmatter_html, remaining_body, body_line_offset)`. If no
/// frontmatter is present, `frontmatter_html` is empty, the full content is
/// returned as the body, and the offset is 0.
fn extract_frontmatter(content: &str) -> (String, &str, usize) {
    match split_frontmatter(content) {
        Some((fm_text, body, line_offset)) => (render_frontmatter(fm_text), body, line_offset),
        None => (String::new(), content, 0),
    }
}

/// The `tags:` values declared in a markdown file's YAML frontmatter (backlog
/// #114). Supports both the block-list form:
///
/// ```text
/// tags:
///   - gaming
///   - dcs
/// ```
///
/// and the inline-array form `tags: [gaming, dcs]`. The `tags` key is matched
/// case-insensitively; returns `[]` when there is no frontmatter or no `tags`
/// entry. This is the canonical label convention for markdown sources.
pub fn frontmatter_tags(content: &str) -> Vec<String> {
    let Some((fm_text, _, _)) = split_frontmatter(content) else {
        return Vec::new();
    };
    parse_frontmatter_entries(fm_text)
        .into_iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("tags"))
        .map(|(_, value)| match value {
            FmValue::List(items) => items,
            FmValue::Scalar(s) => vec![s],
        })
        .unwrap_or_default()
}

/// A parsed frontmatter value: a bare scalar or a `- ` / `[a, b]` list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

/// Strips one layer of matching single/double quotes from a frontmatter token.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        if (bytes[0] == b'"' && bytes[s.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[s.len() - 1] == b'\'')
        {
            return s[1..s.len() - 1].trim().to_string();
        }
    }
    s.to_string()
}

/// Parse YAML frontmatter into ordered `(key, value)` entries. Handles scalar
/// values, indented `- ` block lists, and inline `[a, b]` arrays. Unknown or
/// malformed lines are skipped, matching the previous renderer's leniency.
fn parse_frontmatter_entries(fm_text: &str) -> Vec<(String, FmValue)> {
    let mut entries: Vec<(String, FmValue)> = Vec::new();
    let mut lines = fm_text.lines().peekable();

    while let Some(line) = lines.next() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        if key.is_empty() {
            continue;
        }

        if val.is_empty() {
            // Collect list items indented below this key.
            let mut items: Vec<String> = Vec::new();
            while let Some(next) = lines.peek() {
                let trimmed = next.trim_start();
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let item = unquote(item);
                    if !item.is_empty() {
                        items.push(item);
                    }
                    lines.next();
                } else if trimmed.is_empty() {
                    lines.next();
                } else {
                    break;
                }
            }
            entries.push((key.to_string(), FmValue::List(items)));
        } else if let Some(inner) = val.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            let items = inner
                .split(',')
                .map(unquote)
                .filter(|s| !s.is_empty())
                .collect();
            entries.push((key.to_string(), FmValue::List(items)));
        } else {
            entries.push((key.to_string(), FmValue::Scalar(unquote(val))));
        }
    }

    entries
}

/// Render parsed YAML frontmatter lines into an HTML meta block.
fn render_frontmatter(fm_text: &str) -> String {
    let mut html = String::from("<div class=\"md-meta\">");

    for (key, value) in parse_frontmatter_entries(fm_text) {
        html.push_str("<div class=\"md-meta-row\">");
        html.push_str(&format!(
            "<span class=\"md-meta-key\">{}</span>",
            escape_html(&key)
        ));
        match value {
            FmValue::Scalar(v) => {
                html.push_str(&format!(
                    "<span class=\"md-meta-val\">{}</span>",
                    escape_html(&v)
                ));
            }
            FmValue::List(items) => {
                html.push_str("<span class=\"md-meta-val\">");
                for item in &items {
                    html.push_str(&format!(
                        "<span class=\"md-tag\">{}</span>",
                        escape_html(item)
                    ));
                }
                html.push_str("</span>");
            }
        }
        html.push_str("</div>");
    }

    html.push_str("</div>");
    html
}

/// Detects `@Name` mentions in plain text that case-insensitively match
/// one of `contact_names`, rendering them as clickable `<button
/// data-mention>` spans — everything else, including an `@` that doesn't
/// resolve (e.g. `foo@example.com`), is HTML-escaped plain text.
///
/// The canonical implementation for both surfaces that render `@mention`
/// syntax (backlog #80's Notes cards via `tidl-web`'s
/// `render_note_mentions`, and backlog #91's markdown preview via
/// `render_to_html`'s `process_mentions`) — this crate has no DB access,
/// so callers must supply the live contact-name list themselves.
///
/// A mention only triggers at an `@` that starts the text or follows
/// whitespace (so `foo@example.com` never matches), and only when the
/// character right after the matched name (if any) isn't alphanumeric (so
/// a contact named "Jane" never swallows into "Janet"). Longest name wins
/// when one name is a prefix of another (e.g. "Jane" vs "Jane Doe"), and
/// matching is Unicode-case-insensitive (not just ASCII).
pub fn render_mentions_in_text(text: &str, contact_names: &[String]) -> String {
    let mut candidates: Vec<Vec<char>> = contact_names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().collect())
        .collect();
    candidates.sort_by_key(|chars| std::cmp::Reverse(chars.len()));

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    let mut at_boundary = true;

    while i < chars.len() {
        let c = chars[i];
        if c == '@' && at_boundary {
            let matched = candidates.iter().find(|name_chars| {
                let end = i + 1 + name_chars.len();
                end <= chars.len()
                    && chars[i + 1..end]
                        .iter()
                        .zip(name_chars.iter())
                        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                    && chars.get(end).map(|c| !c.is_alphanumeric()).unwrap_or(true)
            });
            if let Some(name_chars) = matched {
                let name: String = name_chars.iter().collect();
                let escaped = escape_html(&name);
                out.push_str(&format!(
                    r#"<button class="md-mention" data-mention="{escaped}">@{escaped}</button>"#
                ));
                i += 1 + name_chars.len();
                at_boundary = false;
                continue;
            }
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
        at_boundary = c.is_whitespace();
        i += 1;
    }
    out
}

/// Replace `[[[name]]]` and `[[name]]` with inline HTML buttons so they survive
/// the pulldown-cmark pass and can be intercepted for navigation. Triple-bracket is handled first.
fn process_wiki_links(content: &str) -> String {
    let s = TRIPLE_WIKILINK_RE.replace_all(content, |caps: &regex::Captures| {
        let name = escape_html(&caps[1]);
        format!(
            "<button class=\"md-wikilink\" data-wikilink=\"{}\">{}</button>",
            name, name
        )
    });
    WIKILINK_RE
        .replace_all(s.as_ref(), |caps: &regex::Captures| {
            let name = escape_html(&caps[1]);
            format!(
                "<button class=\"md-wikilink\" data-wikilink=\"{}\">{}</button>",
                name, name
            )
        })
        .into_owned()
}

/// Thin adapter between `render_to_html`'s pipeline (which threads plain
/// `&str`/`String` through each pass) and `render_mentions_in_text` (the
/// reusable algorithm Notes calls too, via `tidl-web`). A no-op pass when
/// `contact_names` is empty.
fn process_mentions(content: &str, contact_names: &[String]) -> String {
    if contact_names.is_empty() {
        return content.to_string();
    }
    render_mentions_in_text(content, contact_names)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Convert a subject slug to human-readable format
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tasks_priority_only() {
        let parser = MarkdownParser::new();
        // Trailing text after the tag is included in the source to verify
        // it's ignored for the title in every way — title always comes from
        // the slug (backlog #18's "capture trailing text as title" design
        // was reversed: text after the tag is not part of the tag at all).
        // Priority is clamped to the app's canonical 1-100 domain (backlog
        // #70): 0 clamps to 1 (still highest), and anything above 100
        // clamps to 100 (lowest).
        let content = r#"
# My Notes

- #task/p0/critical-bug Fix the critical bug
- #task/p100/implement-feature Implement new feature
- #task/p999999/someday-maybe
"#;

        let tasks = parser.parse_tasks(content);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].priority, Some(1));
        assert_eq!(tasks[0].subject, "critical-bug");
        assert_eq!(tasks[0].title, "Critical Bug");
        assert_eq!(tasks[1].priority, Some(100));
        assert_eq!(tasks[1].title, "Implement Feature");
        assert_eq!(tasks[2].priority, Some(100));
        assert_eq!(tasks[2].title, "Someday Maybe");
    }

    #[test]
    fn test_parse_tasks_priority_clamps_within_valid_range() {
        let parser = MarkdownParser::new();
        let tasks = parser.parse_tasks("- #task/p1/a\n- #task/p50/b\n- #task/p100/c\n");
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].priority, Some(1));
        assert_eq!(tasks[1].priority, Some(50));
        assert_eq!(tasks[2].priority, Some(100));
    }

    #[test]
    fn test_parse_tasks_dated() {
        let parser = MarkdownParser::new();
        let content = "- #task/2025-06-01/renew-passport Renew passport\n";

        let tasks = parser.parse_tasks(content);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, None);
        assert_eq!(tasks[0].subject, "renew-passport");
        assert_eq!(tasks[0].title, "Renew Passport");
    }

    #[test]
    fn test_parse_tasks_todo_alias() {
        let parser = MarkdownParser::new();
        let content = "- #todo/p0/critical-bug Fix the critical bug\n";

        let tasks = parser.parse_tasks(content);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, Some(1));
        assert_eq!(tasks[0].subject, "critical-bug");
        assert_eq!(tasks[0].title, "Critical Bug");
    }

    #[test]
    fn test_parse_awaits_pending_alias() {
        let parser = MarkdownParser::new();
        let content = "- #pending/2025-01-15/call-dentist Call the dentist\n";

        let awaits = parser.parse_awaits(content);
        assert_eq!(awaits.len(), 1);
        assert_eq!(awaits[0].subject, "call-dentist");
        assert_eq!(awaits[0].title, "Call Dentist");
    }

    #[test]
    fn test_parse_awaits() {
        let parser = MarkdownParser::new();
        let content = r#"
# Awaits

- #await/2025-01-15/call-dentist Call the dentist
- #await/2025-12-31/review-year Year end review
"#;

        let awaits = parser.parse_awaits(content);
        assert_eq!(awaits.len(), 2);
        assert_eq!(awaits[0].subject, "call-dentist");
        assert_eq!(awaits[0].title, "Call Dentist");
        assert_eq!(awaits[1].subject, "review-year");
        assert_eq!(awaits[1].title, "Review Year");
    }

    #[test]
    fn test_parse_all_assigns_line_wide_ordinals() {
        let parser = MarkdownParser::new();
        // Two tasks and an await on one line: the ordinal counts all *active*
        // tags in source order (task, await, task), across both lists, so each
        // tag's uid is unique.
        let content = "Do #task/p1/first and #await/2026-01-01/second then #task/p2/third\n";
        let (tasks, awaits) = parser.parse_all(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(awaits.len(), 1);
        assert_eq!(tasks[0].subject, "first");
        assert_eq!(tasks[0].ordinal, 0);
        assert_eq!(awaits[0].subject, "second");
        assert_eq!(awaits[0].ordinal, 1);
        assert_eq!(tasks[1].subject, "third");
        assert_eq!(tasks[1].ordinal, 2);
    }

    #[test]
    fn test_parse_ordinal_resets_per_line() {
        let parser = MarkdownParser::new();
        let content = "#task/p1/one\n#task/p2/two\n";
        let tasks = parser.parse_tasks(content);
        assert_eq!(tasks[0].ordinal, 0);
        assert_eq!(tasks[1].ordinal, 0);
    }

    #[test]
    fn test_slug_to_title() {
        assert_eq!(slug_to_title("fix-critical-bug"), "Fix Critical Bug");
        assert_eq!(slug_to_title("call-dentist"), "Call Dentist");
    }

    #[test]
    fn test_render_to_html_basic() {
        let html = render_to_html("# Hello\n\nThis is a **paragraph**.", &[]);
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<strong>paragraph</strong>"));
    }

    #[test]
    fn test_render_to_html_gfm_table() {
        // Tables are a GFM extension; Options::all() must be set for this to work.
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let html = render_to_html(md, &[]);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_render_to_html_empty_input() {
        assert_eq!(render_to_html("", &[]), "");
    }

    #[test]
    fn test_render_to_html_task_priority_tag_becomes_badge() {
        // Trailing text is not part of the tag in any way: the badge title
        // comes from the slug, and the trailing text renders separately as
        // its own ordinary (non-badge) content right after the badge.
        let html = render_to_html("- #task/p0/critical-bug Fix the critical bug\n", &[]);
        assert!(!html.contains("#task"), "raw tag text should not survive");
        assert!(html.contains("note-token--task"));
        assert!(
            html.contains("note-token-dot"),
            "urgency is the round dot: {html}"
        );
        assert!(!html.contains("High"), "the tier word is gone: {html}");
        assert!(!html.contains("Medium"), "the tier word is gone: {html}");
        assert!(!html.contains("Low"), "the tier word is gone: {html}");
        assert!(
            html.contains("Critical Bug"),
            "badge title comes from the slug: {html}"
        );
        assert!(
            html.contains("Fix the critical bug"),
            "trailing text still renders, just outside the badge: {html}"
        );
        let badge_start = html.find("note-token--task").unwrap();
        let badge_end = html[badge_start..].find("</div>").unwrap() + badge_start;
        assert!(
            !html[badge_start..badge_end].contains("Fix the critical bug"),
            "trailing text must not be folded into the badge itself: {html}"
        );
    }

    #[test]
    fn test_render_to_html_pending_tag_becomes_due_badge() {
        let html = render_to_html("#pending/2026-07-17/call-dentist Call the dentist\n", &[]);
        assert!(
            !html.contains("#pending"),
            "raw tag text should not survive"
        );
        assert!(html.contains("note-token--await"));
        assert!(html.contains("2026-07-17"));
        assert!(
            html.contains("Call Dentist"),
            "badge title comes from the slug: {html}"
        );
    }

    #[test]
    fn test_render_to_html_pending_tag_trailing_markdown_link_renders_separately() {
        // Reversal of backlog #18: text after the tag is not part of the
        // tag/item in any way. The badge title is always the humanized
        // slug, and any trailing markdown (like a link here) renders
        // completely independently, right after the badge.
        let html = render_to_html(
            "#pending/2026-12-31/assess-implementation [Assess implementation](https://somelink.com)\n",
            &[],
        );
        assert!(
            !html.contains("#pending"),
            "raw tag text should not survive"
        );
        assert!(
            html.contains("Assess Implementation"),
            "badge title comes from the slug, not the trailing text: {html}"
        );
        assert!(
            html.contains("<a href=\"https://somelink.com\">Assess implementation</a>"),
            "trailing markdown link should render as a real link, independent of the badge: {html}"
        );
    }

    #[test]
    fn test_render_to_html_todo_dated_tag_becomes_due_badge() {
        let html = render_to_html("#todo/2026-07-17/todo-due-date-item\n", &[]);
        assert!(!html.contains("#todo"), "raw tag text should not survive");
        assert!(html.contains("note-token--await"));
        assert!(html.contains("2026-07-17"));
        assert!(
            html.contains("Todo Due Date Item"),
            "title always comes from the humanized slug: {html}"
        );
    }

    #[test]
    fn test_render_to_html_priority_tag_title_comes_from_slug() {
        let html = render_to_html("- #task/p1/car-oil-change\n", &[]);
        assert!(html.contains("note-token--task"));
        assert!(html.contains("note-token-dot"));
        assert!(!html.contains("High"), "the tier word is gone: {html}");
        assert!(
            html.contains("Car Oil Change"),
            "title always comes from the humanized slug: {html}"
        );
    }

    #[test]
    fn test_render_to_html_uses_injected_urgency_color() {
        // backlog #116: the web layer injects the user's priority-curve colour,
        // so a markdown chip matches the colour Notes/Focus show.
        let html = render_to_html_with_urgency("- #task/p42/car-oil-change\n", &[], &|_| "#123456");
        assert!(html.contains("--chip-color:#123456;"), "{html}");
    }

    #[test]
    fn test_default_urgency_color_matches_focus_thresholds() {
        // The default curve copy must yield the same palette
        // `focus::urgency_color` does for a given score.
        assert_eq!(default_urgency_color(1), "#d93025");
        assert_eq!(default_urgency_color(100), "#bbb");
    }

    #[test]
    fn test_render_to_html_consecutive_tag_lines_stay_separate() {
        // Consecutive lines with no blank line between them would normally
        // merge into a single CommonMark paragraph; tag lines must not.
        let md = "#pending/2026-07-17/pending-item\n#pending/2026-07-17/awaiting-item\n";
        let html = render_to_html(md, &[]);
        assert_eq!(
            html.matches("md-tag-line").count(),
            2,
            "each tag line should render as its own block: {html}"
        );
    }

    #[test]
    fn test_render_to_html_inline_tag_after_text_becomes_chip() {
        // A tag that shares its line with prose (here, text before it) still
        // renders as a chip — only the chip is special; the surrounding text
        // flows normally through CommonMark.
        let html = render_to_html("See #task/p0/foo for details inline.\n", &[]);
        assert!(!html.contains("#task/p0/foo"), "raw tag is gone: {html}");
        assert!(html.contains("note-token--task"), "{html}");
        assert!(html.contains("See "), "leading prose survives: {html}");
        assert!(
            html.contains("for details inline."),
            "trailing prose survives: {html}"
        );
    }

    #[test]
    fn test_render_to_html_inline_tag_in_list_item_becomes_chip() {
        // The common real-world shape: a bullet whose tag trails the item text.
        // The list marker is preserved (unlike a leading-tag line, which takes
        // the block-badge path) so it renders as a real list item.
        let html = render_to_html("- Buy milk #task/p1/buy-milk\n", &[]);
        assert!(html.contains("<li>"), "list marker preserved: {html}");
        assert!(
            !html.contains("#task/p1/buy-milk"),
            "raw tag is gone: {html}"
        );
        assert!(html.contains("note-token--task"), "{html}");
        assert!(html.contains("Buy milk"), "item text survives: {html}");
        assert!(html.contains("Buy Milk"), "chip title from slug: {html}");
    }

    #[test]
    fn test_render_to_html_inline_tags_get_per_line_ordinals() {
        // Two tags on one line are numbered in source order so the preview can
        // complete/reopen exactly the clicked one.
        let html = render_to_html("Do #task/p0/first then #await/2026-01-01/second.\n", &[]);
        assert!(html.contains("data-md-ordinal=\"0\""), "{html}");
        assert!(html.contains("data-md-ordinal=\"1\""), "{html}");
        let task = html.find("note-token--task").unwrap();
        let await_ = html.find("note-token--await").unwrap();
        assert!(task < await_, "tags keep source order: {html}");
    }

    #[test]
    fn test_render_to_html_leading_tag_keeps_block_and_renders_following_tag() {
        // A line whose first tag leads keeps the block-level badge (so
        // consecutive tag lines stay separate), but a *second* tag later on
        // the same line is still rendered as an inline chip.
        let html = render_to_html("- #task/p0/one then #task/p1/two\n", &[]);
        assert_eq!(
            html.matches("md-tag-line").count(),
            1,
            "only the leading tag is block-wrapped: {html}"
        );
        assert_eq!(
            html.matches("note-token--task").count(),
            2,
            "both tags render: {html}"
        );
        assert!(html.contains("data-md-ordinal=\"1\""), "{html}");
    }

    #[test]
    fn test_render_to_html_midword_tag_stays_literal() {
        // No word boundary before the `#`, matching the notes scanner — a tag
        // must not fire mid-word.
        let html = render_to_html("see#task/p0/foo\n", &[]);
        assert!(html.contains("see#task/p0/foo"), "{html}");
        assert!(!html.contains("note-token"), "{html}");
    }

    #[test]
    fn test_render_to_html_tag_in_inline_code_stays_literal() {
        // Documenting the syntax shouldn't render a live chip.
        let html = render_to_html("Use `#task/p1/foo` to add a task.\n", &[]);
        assert!(!html.contains("note-token"), "{html}");
        assert!(html.contains("<code>#task/p1/foo</code>"), "{html}");
    }

    #[test]
    fn test_render_to_html_leading_code_span_does_not_become_block() {
        // The code-span tag leads the line, but it's still code: only the real
        // tag after it renders, inline, and its ordinal accounts for the code
        // one the server will also count.
        let html = render_to_html("`#task/p1/foo` then #task/p2/bar\n", &[]);
        assert!(!html.contains("md-tag-line"), "{html}");
        assert!(html.contains("<code>#task/p1/foo</code>"), "{html}");
        assert!(html.contains("note-token--task"), "{html}");
        assert!(html.contains("data-md-ordinal=\"1\""), "{html}");
    }

    #[test]
    fn test_render_to_html_malformed_tag_stays_literal() {
        // `#task/p1/foo/bar` has no valid word boundary after the slug, so it
        // is not a tag (matching the notes scanner).
        let html = render_to_html("#task/p1/foo/bar\n", &[]);
        assert!(!html.contains("note-token"), "{html}");
        assert!(html.contains("#task/p1/foo/bar"), "{html}");
    }

    #[test]
    fn test_render_to_html_tag_in_fenced_code_stays_literal() {
        let html = render_to_html("```\n#task/p1/foo\n```\n", &[]);
        assert!(!html.contains("note-token"), "{html}");
        assert!(html.contains("#task/p1/foo"), "{html}");
    }

    #[test]
    fn test_render_to_html_tag_after_single_line_backtick_span() {
        // Regression: a "fence" opened and closed on one line (here an inline
        // code span, because a backtick fence's info string may not contain a
        // backtick) must not leave the fence tracker open and swallow the rest
        // of the document.
        let content = "- Insert stick.\n    ```systemctl start cbs```\n- Check journal output.\n- #todo/p003/schedule-periodic-usb-update-reminder\n";
        let html = render_to_html(content, &[]);
        assert!(
            html.contains("note-token--task"),
            "todo must render: {html}"
        );
        assert!(!html.contains("#todo/p003"), "{html}");
    }

    #[test]
    fn test_render_to_html_badge_carries_data_md_line() {
        // backlog #74: the preview modal targets a pill by line number to
        // flash it — the attribute must land on the badge span itself, not
        // the wrapping `md-tag-line` div, and match the file's real
        // (1-indexed) line number.
        let html = render_to_html("# Notes\n\n- #task/p0/critical-bug\n", &[]);
        assert!(
            html.contains("data-md-line=\"3\""),
            "tag is on line 3 of the file: {html}"
        );
        let attr_pos = html.find("data-md-line=\"3\"").unwrap();
        let span_start = html[..attr_pos].rfind("<span").unwrap();
        assert!(
            html[span_start..attr_pos].contains("note-token"),
            "data-md-line must be on the note-token span: {html}"
        );
    }

    #[test]
    fn test_render_to_html_badge_data_md_line_accounts_for_frontmatter() {
        // Task/await `line` fields are 1-indexed against the full file
        // including frontmatter (see TaskItem::line docs) — the badge's
        // data-md-line must match that, not a body-relative line count.
        let md = "---\ndate: 2026-01-01\n---\n#pending/2026-07-17/call-dentist\n";
        let html = render_to_html(md, &[]);
        assert!(
            html.contains("data-md-line=\"4\""),
            "tag is on line 4 counting the frontmatter block: {html}"
        );
    }

    #[test]
    fn test_frontmatter_string_field() {
        let md = "---\ndate: 2020-09-16T22:35\n---\n# Body\n";
        let html = render_to_html(md, &[]);
        assert!(html.contains("md-meta"), "should have meta block");
        assert!(html.contains("date"), "key should appear");
        assert!(html.contains("2020-09-16T22:35"), "value should appear");
        assert!(html.contains("<h1>Body</h1>"), "body should be rendered");
    }

    #[test]
    fn test_frontmatter_tag_list() {
        let md = "---\ntags:\n    - it-home\n    - rust\n---\ncontent\n";
        let html = render_to_html(md, &[]);
        assert!(html.contains("md-tag"), "tags should be rendered as chips");
        assert!(html.contains("it-home"), "tag value should appear");
        assert!(html.contains("rust"), "second tag should appear");
    }

    #[test]
    fn frontmatter_tags_reads_block_and_inline_forms() {
        let block = "---\ndate: 2020-10-07T15:22\ntags:\n    - gaming\n    - dcs\n    - flightsim\n---\nbody\n";
        assert_eq!(
            frontmatter_tags(block),
            vec![
                "gaming".to_string(),
                "dcs".to_string(),
                "flightsim".to_string()
            ]
        );

        let inline = "---\ntags: [alpha, beta]\n---\nbody\n";
        assert_eq!(
            frontmatter_tags(inline),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn frontmatter_tags_trims_quotes_blanks_and_casing_key() {
        let md = "---\nTags:\n  - \"two words\"\n  - 'square'\n  -\n---\nbody\n";
        assert_eq!(
            frontmatter_tags(md),
            vec!["two words".to_string(), "square".to_string()]
        );
    }

    #[test]
    fn frontmatter_tags_empty_without_frontmatter_or_tags() {
        assert!(frontmatter_tags("# Heading\n\ntext\n").is_empty());
        assert!(frontmatter_tags("---\ndate: 2020-01-01\n---\nbody\n").is_empty());
    }

    #[test]
    fn inline_array_frontmatter_renders_as_chips() {
        let html = render_to_html("---\ntags: [a, b]\n---\nbody\n", &[]);
        assert!(html.contains("md-tag"), "inline array tags render as chips");
        assert!(html.contains(">a<"));
        assert!(html.contains(">b<"));
    }

    #[test]
    fn test_wikilink_double_bracket() {
        let html = render_to_html("See [[other zettel]] for details.", &[]);
        assert!(html.contains("md-wikilink"), "wiki link should be styled");
        assert!(html.contains("other zettel"), "link text should appear");
        assert!(!html.contains("[["), "raw brackets should be gone");
        assert!(html.contains("<button"), "should emit button element");
        assert!(
            html.contains("data-wikilink="),
            "should have data-wikilink attribute"
        );
    }

    #[test]
    fn test_wikilink_triple_bracket_treated_same() {
        let html = render_to_html("See [[[other zettel]]] too.", &[]);
        assert!(html.contains("md-wikilink"));
        assert!(html.contains("other zettel"));
        assert!(!html.contains("[[["), "raw triple brackets should be gone");
        assert!(html.contains("<button"), "should emit button element");
        assert!(
            html.contains("data-wikilink="),
            "should have data-wikilink attribute"
        );
    }

    #[test]
    fn test_render_to_html_mention_resolves_against_supplied_contacts() {
        let html = render_to_html("Ping @Jane Doe about this.", &["Jane Doe".to_string()]);
        assert!(html.contains(r#"data-mention="Jane Doe""#));
    }

    #[test]
    fn test_render_to_html_unresolved_mention_stays_plain_text() {
        let html = render_to_html("Ping @Bob about this.", &["Jane Doe".to_string()]);
        assert!(!html.contains("md-mention"));
        assert!(html.contains("@Bob"));
    }

    #[test]
    fn test_render_to_html_mention_and_wikilink_coexist_on_one_line() {
        let html = render_to_html(
            "@Jane Doe mentioned [[some file]] in passing.",
            &["Jane Doe".to_string()],
        );
        assert!(html.contains(r#"data-mention="Jane Doe""#));
        assert!(html.contains(r#"data-wikilink="some file""#));
    }

    #[test]
    fn test_no_frontmatter_unchanged() {
        let md = "# Just a heading\n\nSome text.\n";
        let html = render_to_html(md, &[]);
        assert!(
            !html.contains("md-meta"),
            "no meta block without frontmatter"
        );
        assert!(html.contains("<h1>Just a heading</h1>"));
    }

    #[test]
    fn test_active_task_badge_carries_a_checkbox() {
        let html = render_to_html("- #task/p0/critical-bug\n", &[]);
        assert!(
            html.contains("md-task-checkbox"),
            "active task needs a checkbox: {html}"
        );
        assert!(
            !html.contains("checked"),
            "active task must start unchecked: {html}"
        );
        assert!(
            !html.contains("disabled"),
            "active task checkbox must be interactive: {html}"
        );
    }

    #[test]
    fn test_active_await_badge_has_no_checkbox_and_shows_date() {
        let html = render_to_html("#await/2025-01-15/call-dentist\n", &[]);
        assert!(
            !html.contains("md-task-checkbox"),
            "an await is never completed, so it carries no checkbox: {html}"
        );
        assert!(html.contains("note-token--await"));
        assert!(
            html.contains("2025-01-15"),
            "await chip shows its date inline: {html}"
        );
    }

    #[test]
    fn test_legacy_done_tag_renders_a_checked_disabled_badge() {
        let html = render_to_html("- #done/critical-bug Fix the critical bug\n", &[]);
        assert!(!html.contains("#done"), "raw tag text should not survive");
        assert!(html.contains("note-token--done"));
        assert!(
            html.contains("checked"),
            "done badge checkbox is checked: {html}"
        );
        assert!(
            html.contains("disabled"),
            "legacy done can't be re-toggled: {html}"
        );
        assert!(
            html.contains("Critical Bug"),
            "title comes from the slug: {html}"
        );
    }

    #[test]
    fn test_rich_done_tag_renders_an_interactive_reopenable_badge() {
        for tag in [
            "#done/2026-10-02/p3/critical-bug",
            "#done/2026-10-02/t2025-06-01/renew-passport",
            "#done/2026-10-02/2025-01-15/call-dentist",
        ] {
            let html = render_to_html(&format!("- {tag}\n"), &[]);
            assert!(
                !html.contains("#done"),
                "raw tag text should not survive: {html}"
            );
            assert!(html.contains("note-token--done"), "{html}");
            assert!(html.contains("checked"), "done chip is checked: {html}");
            assert!(
                !html.contains("disabled"),
                "rich done chip must be re-openable, not disabled: {html}"
            );
            assert!(
                html.contains("data-md-reopen"),
                "re-open hook present: {html}"
            );
        }
    }

    #[test]
    fn test_done_line_is_not_parsed_as_a_task_or_await() {
        let parser = MarkdownParser::new();
        let content = "- #done/critical-bug\n- #done/2026-10-02/p3/critical-bug\n\
                       - #done/2026-10-02/t2025-06-01/renew-passport\n\
                       - #done/2026-10-02/2025-01-15/call-dentist\n";
        assert!(parser.parse_tasks(content).is_empty());
        assert!(parser.parse_awaits(content).is_empty());
    }

    #[test]
    fn test_mark_tag_done_keeps_priority_date_and_completion() {
        let c = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(
            mark_tag_done("#task/p3/critical-bug", c),
            "#done/2026-10-02/p3/critical-bug"
        );
        assert_eq!(
            mark_tag_done("#task/p100/someday-maybe", c),
            "#done/2026-10-02/p100/someday-maybe"
        );
        assert_eq!(
            mark_tag_done("- #task/2025-06-01/renew-passport Renew passport", c),
            "- #done/2026-10-02/t2025-06-01/renew-passport Renew passport"
        );
        assert_eq!(
            mark_tag_done("#await/2025-01-15/call-dentist Call the dentist", c),
            "#done/2026-10-02/2025-01-15/call-dentist Call the dentist"
        );
        assert_eq!(
            mark_tag_done("- #pending/2025-01-15/call-dentist", c),
            "- #done/2026-10-02/2025-01-15/call-dentist"
        );
        assert_eq!(
            mark_tag_done("- #todo/p0/critical-bug Fix it", c),
            "- #done/2026-10-02/p0/critical-bug Fix it"
        );
    }

    #[test]
    fn test_mark_tag_done_replaces_only_the_first_tag() {
        let c = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(
            mark_tag_done("#task/p0/one #task/p1/two", c),
            "#done/2026-10-02/p0/one #task/p1/two"
        );
    }

    #[test]
    fn test_mark_tag_done_at_targets_the_nth_tag() {
        let c = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(
            mark_tag_done_at("#task/p0/one #task/p1/two", c, 0),
            "#done/2026-10-02/p0/one #task/p1/two"
        );
        assert_eq!(
            mark_tag_done_at("#task/p0/one #task/p1/two", c, 1),
            "#task/p0/one #done/2026-10-02/p1/two"
        );
        assert_eq!(
            mark_tag_done_at("#task/p0/one #task/p1/two", c, 9),
            "#task/p0/one #task/p1/two",
            "an out-of-range ordinal is a no-op"
        );
    }

    #[test]
    fn test_mark_tag_done_leaves_untagged_and_already_done_lines_unchanged() {
        let c = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(
            mark_tag_done("Just prose with no tag", c),
            "Just prose with no tag"
        );
        assert_eq!(
            mark_tag_done("- #done/critical-bug", c),
            "- #done/critical-bug"
        );
        assert_eq!(
            mark_tag_done("- #done/2026-10-02/p3/x", c),
            "- #done/2026-10-02/p3/x"
        );
        assert_eq!(
            mark_tag_done("See #task/p0/foo inline", c),
            "See #done/2026-10-02/p0/foo inline"
        );
    }

    #[test]
    fn test_reopen_tag_restores_each_form() {
        assert_eq!(
            reopen_tag("#done/2026-10-02/p3/critical-bug").as_deref(),
            Some("#task/p3/critical-bug")
        );
        assert_eq!(
            reopen_tag("- #done/2026-10-02/t2025-06-01/renew-passport").as_deref(),
            Some("- #task/2025-06-01/renew-passport")
        );
        assert_eq!(
            reopen_tag("#done/2026-10-02/2025-01-15/call-dentist doit").as_deref(),
            Some("#await/2025-01-15/call-dentist doit")
        );
    }

    #[test]
    fn test_reopen_tag_returns_none_for_legacy_or_plain() {
        assert_eq!(reopen_tag("- #done/critical-bug"), None);
        assert_eq!(reopen_tag("just prose"), None);
        assert_eq!(reopen_tag("- #task/p0/x"), None);
    }

    #[test]
    fn test_reopen_tag_at_targets_the_nth_rich_done_record() {
        let line = "#done/2026-10-02/p0/one #done/2026-10-02/2025-01-15/two";
        assert_eq!(
            reopen_tag_at(line, 0).as_deref(),
            Some("#task/p0/one #done/2026-10-02/2025-01-15/two")
        );
        assert_eq!(
            reopen_tag_at(line, 1).as_deref(),
            Some("#done/2026-10-02/p0/one #await/2025-01-15/two")
        );
        assert_eq!(reopen_tag_at(line, 2), None);
    }

    #[test]
    fn scan_done_tags_reports_forms_and_ordinals() {
        let content = "line one\n- #done/2026-10-02/p3/buy-milk Buy milk\n#done/2026-10-03/t2025-06-01/renew and #done/2026-10-03/2025-01-15/call\n";
        let tags = scan_done_tags(content);
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].line, 2);
        assert_eq!(tags[0].ordinal, 0);
        assert_eq!(tags[0].kind, DoneKind::Task);
        assert_eq!(tags[0].slug, "buy-milk");
        assert_eq!(tags[0].priority, Some(3));
        assert_eq!(
            tags[0].completed,
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
        );
        assert_eq!(tags[1].line, 3);
        assert_eq!(tags[1].ordinal, 0);
        assert_eq!(tags[1].kind, DoneKind::DatedTask);
        assert_eq!(tags[1].due_date, NaiveDate::from_ymd_opt(2025, 6, 1));
        assert_eq!(tags[2].line, 3);
        assert_eq!(tags[2].ordinal, 1);
        assert_eq!(tags[2].kind, DoneKind::Await);
        assert_eq!(tags[2].due_date, NaiveDate::from_ymd_opt(2025, 1, 15));
    }

    #[test]
    fn scan_done_tags_skips_legacy() {
        assert!(scan_done_tags("- #done/old-slug\n").is_empty());
        assert!(scan_done_tags("- #task/p1/active\n").is_empty());
    }

    #[test]
    fn remove_tag_at_strips_only_the_tag() {
        assert_eq!(
            remove_tag_at("- #task/p3/alpha keep this", 0, false).as_deref(),
            Some("- keep this")
        );
        assert_eq!(
            remove_tag_at("- #done/2026-10-02/p3/alpha keep this", 0, true).as_deref(),
            Some("- keep this")
        );
        assert_eq!(remove_tag_at("- #done/2026-10-02/p3/alpha", 0, false), None);
        assert_eq!(remove_tag_at("- #task/p3/alpha", 0, true), None);
    }

    #[test]
    fn remove_done_tags_strips_and_drops_emptied_lines() {
        let content = "intro\n- #done/2026-10-02/p3/buy-milk\n- #done/2026-10-02/p4/keep-me write report\noutro\n";
        let (out, removed) = remove_done_tags(content, &[(2, 0), (3, 0)]);
        assert_eq!(removed, 2);
        assert_eq!(out, "intro\n- write report\noutro\n");
    }

    #[test]
    fn remove_done_tags_preserves_crlf() {
        let content = "a\r\n- #done/2026-10-02/p3/x\r\nb\r\n";
        let (out, removed) = remove_done_tags(content, &[(2, 0)]);
        assert_eq!(removed, 1);
        assert_eq!(out, "a\r\nb\r\n");
    }

    #[test]
    fn remove_done_tags_handles_multiple_on_one_line() {
        let content = "#done/2026-10-02/p1/one #done/2026-10-02/p2/two\n";
        let (out, removed) = remove_done_tags(content, &[(1, 0), (1, 1)]);
        assert_eq!(removed, 2);
        assert_eq!(out, "");
    }

    #[test]
    fn test_edit_tag_at_preserves_each_form() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(
            edit_tag_at("#task/p3/critical-bug", 0, "renamed", Some(10), None),
            "#task/p10/renamed"
        );
        assert_eq!(
            edit_tag_at(
                "- #task/2025-06-01/renew-passport go",
                0,
                "new-pass",
                None,
                Some(d(2026, 1, 2))
            ),
            "- #task/2026-01-02/new-pass go"
        );
        assert_eq!(
            edit_tag_at(
                "#await/2025-01-15/call-dentist",
                0,
                "book-dentist",
                None,
                Some(d(2026, 3, 4))
            ),
            "#await/2026-03-04/book-dentist"
        );
    }

    #[test]
    fn test_edit_tag_at_targets_the_nth_tag_and_keeps_others() {
        let line = "#task/p0/one #await/2025-01-15/two";
        assert_eq!(
            edit_tag_at(line, 0, "uno", Some(7), None),
            "#task/p7/uno #await/2025-01-15/two"
        );
        assert_eq!(
            edit_tag_at(
                line,
                1,
                "dos",
                None,
                Some(NaiveDate::from_ymd_opt(2025, 2, 2).unwrap())
            ),
            "#task/p0/one #await/2025-02-02/dos"
        );
    }

    #[test]
    fn test_edit_tag_at_falls_back_when_param_missing_or_out_of_range() {
        assert_eq!(
            edit_tag_at("#task/p3/x", 9, "y", Some(1), None),
            "#task/p3/x"
        );
        assert_eq!(
            edit_tag_at("#task/p3/x", 0, "", Some(1), None),
            "#task/p3/x"
        );
        assert_eq!(
            edit_tag_at("#await/2025-01-15/x", 0, "y", None, None),
            "#await/2025-01-15/y"
        );
    }
}

#[cfg(test)]
mod mention_tests {
    use super::render_mentions_in_text;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_exact_case_insensitive_match() {
        let html = render_mentions_in_text("ping @jane doe about this", &names(&["Jane Doe"]));
        assert!(html.contains(r#"data-mention="Jane Doe""#));
        assert!(html.contains(">@Jane Doe<"));
    }

    #[test]
    fn leaves_unresolved_mention_as_plain_text() {
        let html = render_mentions_in_text("ping @Bob about this", &names(&["Jane Doe"]));
        assert!(!html.contains("md-mention"));
        assert!(html.contains("@Bob"));
    }

    #[test]
    fn does_not_match_inside_an_email_address() {
        let html = render_mentions_in_text("contact foo@example.com", &names(&["Example"]));
        assert!(
            !html.contains("md-mention"),
            "@ must follow whitespace or start-of-text to trigger"
        );
    }

    #[test]
    fn longest_name_wins_when_one_is_a_prefix_of_another() {
        let html = render_mentions_in_text("@Jane Doe is here", &names(&["Jane", "Jane Doe"]));
        assert!(html.contains(r#"data-mention="Jane Doe""#));
        assert!(!html.contains(r#"data-mention="Jane""#));
    }

    #[test]
    fn does_not_swallow_a_longer_word_sharing_a_prefix() {
        let html = render_mentions_in_text("@Janet was here", &names(&["Jane"]));
        assert!(
            !html.contains("md-mention"),
            "\"Jane\" must not match inside \"Janet\""
        );
        assert!(html.contains("@Janet"));
    }

    #[test]
    fn escapes_html_in_both_plain_text_and_matched_names() {
        let html = render_mentions_in_text("<script> @Bob & friends", &names(&["Bob"]));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(r#"data-mention="Bob""#));
        assert!(html.contains("&amp; friends"));
    }

    #[test]
    fn mention_at_start_of_text_matches() {
        let html = render_mentions_in_text("@Jane hi", &names(&["Jane"]));
        assert!(html.contains(r#"data-mention="Jane""#));
    }

    #[test]
    fn empty_contact_name_is_never_matched() {
        let html = render_mentions_in_text("@ nobody", &names(&[""]));
        assert!(!html.contains("md-mention"));
    }
}

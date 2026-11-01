//! Wasm-safe scanner for markdown task/await tags (backlog #129).
//!
//! The markdown editor is CodeMirror, but the tag grammar lives in
//! `tidl_markdown::parser`. That crate as a whole does not compile to wasm32
//! (its watcher/sync modules pull in `tokio`/`notify`), so — exactly like
//! [`crate::note_tokens`] — this mirrors only the tag grammar here. The
//! server fns (`load_todo_chip`, `update_todo_chip`) still re-resolve against
//! the real parser, so the client scanner only needs to agree on *which* tag
//! a click addresses: its 1-based line and 0-based active ordinal.
//!
//! Grammar is identical to `tidl_markdown::parser`'s `scan_line_tags`:
//! `#task/p<N>/<slug>`, dated `#task/<date>/<slug>`, `#await/<date>/<slug>`,
//! the legacy `#todo`/`#pending` aliases, and the `#done/...` completion
//! records. Ordinals count every active tag on the line (matching
//! `MarkdownParser::parse_all`), while a *hit* additionally requires the
//! word-boundary and inline-code rules the renderer applies.

use std::sync::LazyLock;

use regex::Regex;

static TASK_PRIORITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?:task|todo)/p(\d+)/([a-zA-Z0-9-]+)").unwrap());
static TASK_DATED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?:task|todo)/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap());
static AWAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#(?:await|pending)/(\d{4}-\d{2}-\d{2})/([a-zA-Z0-9-]+)").unwrap()
});
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

/// The textual form of an *active* tag, which [`format_md_token`] preserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdTagForm {
    PriorityTask,
    DatedTask,
    Await,
}

/// An active tag's parsed payload, for prefilling the editor's dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdTagKind {
    Priority { priority: u32, slug: String },
    Dated { date: String, slug: String },
    Await { date: String, slug: String },
}

impl MdTagKind {
    pub fn form(&self) -> MdTagForm {
        match self {
            MdTagKind::Priority { .. } => MdTagForm::PriorityTask,
            MdTagKind::Dated { .. } => MdTagForm::DatedTask,
            MdTagKind::Await { .. } => MdTagForm::Await,
        }
    }
}

/// A rich `#done/...` record's original tag plus its completion date, for
/// prefilling the editor's dialog so unchecking "Mark as done" restores the
/// active tag. A legacy `#done/<slug>` record has no parameters to restore and
/// is not represented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdDoneKind {
    Task {
        completed: String,
        priority: u32,
        slug: String,
    },
    DatedTask {
        completed: String,
        date: String,
        slug: String,
    },
    Await {
        completed: String,
        date: String,
        slug: String,
    },
}

impl MdDoneKind {
    pub fn form(&self) -> MdTagForm {
        match self {
            MdDoneKind::Task { .. } => MdTagForm::PriorityTask,
            MdDoneKind::DatedTask { .. } => MdTagForm::DatedTask,
            MdDoneKind::Await { .. } => MdTagForm::Await,
        }
    }

    pub fn slug(&self) -> &str {
        match self {
            MdDoneKind::Task { slug, .. }
            | MdDoneKind::DatedTask { slug, .. }
            | MdDoneKind::Await { slug, .. } => slug,
        }
    }

    pub fn completed(&self) -> &str {
        match self {
            MdDoneKind::Task { completed, .. }
            | MdDoneKind::DatedTask { completed, .. }
            | MdDoneKind::Await { completed, .. } => completed,
        }
    }

    /// The original due/original date; `None` for a priority task.
    pub fn date(&self) -> Option<&str> {
        match self {
            MdDoneKind::Task { .. } => None,
            MdDoneKind::DatedTask { date, .. } | MdDoneKind::Await { date, .. } => Some(date),
        }
    }

    pub fn priority(&self) -> u32 {
        match self {
            MdDoneKind::Task { priority, .. } => *priority,
            MdDoneKind::DatedTask { .. } | MdDoneKind::Await { .. } => 50,
        }
    }
}

/// A tag resolved under a caret. `kind` is `Some` for an active task/await;
/// `done` is `Some` for a rich `#done` record (both `None` for a legacy
/// `#done/<slug>`, which has nothing to edit or restore).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdTagAt {
    /// 1-based line number.
    pub line: usize,
    /// 0-based active-tag ordinal within the line.
    pub ordinal: usize,
    /// Byte offset of the leading `#` in the whole document.
    pub start: usize,
    /// Byte offset one past the tag in the whole document.
    pub end: usize,
    pub kind: Option<MdTagKind>,
    pub done: Option<MdDoneKind>,
}

impl MdTagAt {
    pub fn is_active(&self) -> bool {
        self.kind.is_some()
    }
}

/// The markdown tag whose byte span strictly contains `caret_byte`
/// (`start < caret_byte < end`), applying the renderer's word-boundary and
/// inline-code rules so a mid-word `see#task/p1/foo` or a backticked code span
/// never resolves. A caret at or after the tag's last character does not count,
/// so typing on after a finished tag never opens its dialog. Returns `None` off
/// any tag.
pub fn tag_at_caret(doc: &str, caret_byte: usize) -> Option<MdTagAt> {
    let caret = caret_byte.min(doc.len());
    let line_start = doc[..caret].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = doc[caret..]
        .find('\n')
        .map(|i| caret + i)
        .unwrap_or(doc.len());
    let line = &doc[line_start..line_end];
    let line_no = 1 + doc[..line_start].bytes().filter(|b| *b == b'\n').count();
    let col = caret - line_start;

    let tag = scan_line_tags(line).into_iter().find(|t| {
        t.start < col
            && col < t.end
            && boundary_before(line, t.start)
            && boundary_after(line, t.end)
            && !inside_inline_code(line, t.start)
    })?;

    Some(MdTagAt {
        line: line_no,
        ordinal: tag.active_ord.unwrap_or(0),
        start: line_start + tag.start,
        end: line_start + tag.end,
        kind: tag.kind,
        done: tag.done,
    })
}

/// Builds a token in the given textual form.
pub fn format_md_token(
    form: MdTagForm,
    slug: &str,
    priority: Option<u32>,
    date: Option<&str>,
) -> String {
    match form {
        MdTagForm::PriorityTask => {
            format!("#task/p{}/{}", priority.unwrap_or(50).clamp(1, 100), slug)
        }
        MdTagForm::DatedTask => format!("#task/{}/{}", date.unwrap_or_default(), slug),
        MdTagForm::Await => format!("#await/{}/{}", date.unwrap_or_default(), slug),
    }
}

/// Builds the `#done/...` record a tag becomes when completed on
/// `completed` (a `YYYY-MM-DD` date), preserving its original kind and
/// parameters exactly like `tidl_markdown::parser::mark_tag_done`:
///
/// - `#task/p<N>/<slug>`   → `#done/<c>/p<N>/<slug>`
/// - `#task/<date>/<slug>` → `#done/<c>/t<date>/<slug>`
/// - `#await/<date>/<slug>` → `#done/<c>/<date>/<slug>`
pub fn format_done_token(
    form: MdTagForm,
    slug: &str,
    priority: Option<u32>,
    date: Option<&str>,
    completed: &str,
) -> String {
    match form {
        MdTagForm::PriorityTask => format!(
            "#done/{completed}/p{}/{slug}",
            priority.unwrap_or(50).clamp(1, 100)
        ),
        MdTagForm::DatedTask => {
            format!("#done/{completed}/t{}/{slug}", date.unwrap_or_default())
        }
        MdTagForm::Await => format!("#done/{completed}/{}/{slug}", date.unwrap_or_default()),
    }
}

#[derive(Debug, Clone)]
struct LineTag {
    start: usize,
    end: usize,
    kind: Option<MdTagKind>,
    done: Option<MdDoneKind>,
    /// 0-based index among the line's *active* tags.
    active_ord: Option<usize>,
}

/// Mirrors `tidl_markdown::parser::scan_line_tags` (see module docs).
fn scan_line_tags(line: &str) -> Vec<LineTag> {
    let mut candidates: Vec<(usize, usize, Option<MdTagKind>, Option<MdDoneKind>)> = Vec::new();

    let mut push = |cap: &regex::Captures, kind: Option<MdTagKind>, done: Option<MdDoneKind>| {
        if let Some(m) = cap.get(0) {
            candidates.push((m.start(), m.end(), kind, done));
        }
    };

    for cap in TASK_PRIORITY_RE.captures_iter(line) {
        if let (Some(p), Some(slug)) = (cap.get(1), cap.get(2)) {
            if let Ok(priority) = p.as_str().parse::<u32>() {
                push(
                    &cap,
                    Some(MdTagKind::Priority {
                        priority: priority.clamp(1, 100),
                        slug: slug.as_str().to_string(),
                    }),
                    None,
                );
            }
        }
    }
    for cap in TASK_DATED_RE.captures_iter(line) {
        if let (Some(d), Some(slug)) = (cap.get(1), cap.get(2)) {
            push(
                &cap,
                Some(MdTagKind::Dated {
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                }),
                None,
            );
        }
    }
    for cap in AWAIT_RE.captures_iter(line) {
        if let (Some(d), Some(slug)) = (cap.get(1), cap.get(2)) {
            push(
                &cap,
                Some(MdTagKind::Await {
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                }),
                None,
            );
        }
    }
    // Rich done records keep their original form/parameters so the editor can
    // restore the active tag when "Mark as done" is unchecked. Legacy
    // `#done/<slug>` records carry no such data (both fields stay `None`).
    for cap in DONE_TASK_RE.captures_iter(line) {
        if let (Some(c), Some(p), Some(slug)) = (cap.get(1), cap.get(2), cap.get(3)) {
            if let Ok(priority) = p.as_str().parse::<u32>() {
                push(
                    &cap,
                    None,
                    Some(MdDoneKind::Task {
                        completed: c.as_str().to_string(),
                        priority: priority.clamp(1, 100),
                        slug: slug.as_str().to_string(),
                    }),
                );
            }
        }
    }
    for cap in DONE_DATED_RE.captures_iter(line) {
        if let (Some(c), Some(d), Some(slug)) = (cap.get(1), cap.get(2), cap.get(3)) {
            push(
                &cap,
                None,
                Some(MdDoneKind::DatedTask {
                    completed: c.as_str().to_string(),
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                }),
            );
        }
    }
    for cap in DONE_AWAIT_RE.captures_iter(line) {
        if let (Some(c), Some(d), Some(slug)) = (cap.get(1), cap.get(2), cap.get(3)) {
            push(
                &cap,
                None,
                Some(MdDoneKind::Await {
                    completed: c.as_str().to_string(),
                    date: d.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                }),
            );
        }
    }
    for cap in DONE_LEGACY_RE.captures_iter(line) {
        push(&cap, None, None);
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut tags: Vec<LineTag> = Vec::new();
    let mut last_end = 0usize;
    for (start, end, kind, done) in candidates {
        if start >= last_end {
            last_end = end;
            tags.push(LineTag {
                start,
                end,
                kind,
                done,
                active_ord: None,
            });
        }
    }

    let mut active = 0usize;
    for t in &mut tags {
        if t.kind.is_some() {
            t.active_ord = Some(active);
            active += 1;
        }
    }
    tags
}

fn boundary_before(line: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    matches!(
        line.as_bytes()[start - 1],
        b' ' | b'\t' | b'\n' | b'\r' | b'(' | b'[' | b'{' | b'"' | b'\''
    )
}

fn boundary_after(line: &str, end: usize) -> bool {
    match line.as_bytes().get(end) {
        None => true,
        Some(b) => !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'/'),
    }
}

fn inside_inline_code(line: &str, pos: usize) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut open: Option<usize> = None;
    while i < pos && i < bytes.len() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_priority_task_across_the_whole_tag() {
        let doc = "intro\ndo #task/p001/some-task now\nlast";
        let start = doc.find('#').unwrap();
        let end = start + "#task/p001/some-task".len();
        for caret in (start + 1)..end {
            let hit = tag_at_caret(doc, caret).unwrap_or_else(|| panic!("caret {caret}"));
            assert_eq!(hit.line, 2, "caret {caret}");
            assert_eq!(hit.ordinal, 0);
            assert_eq!(
                hit.kind,
                Some(MdTagKind::Priority {
                    priority: 1,
                    slug: "some-task".into()
                })
            );
        }
        assert!(tag_at_caret(doc, start).is_none());
        assert!(
            tag_at_caret(doc, end).is_none(),
            "a caret right after the tag must not open it"
        );
        assert!(tag_at_caret(doc, end + 1).is_none());
    }

    #[test]
    fn resolves_dated_task_and_await() {
        let doc = "#task/2026-01-01/dated and #await/2026-12-31/renew";
        let dated = tag_at_caret(doc, 5).unwrap();
        assert_eq!(dated.kind.unwrap().form(), MdTagForm::DatedTask);
        let await_start = doc.find("#await").unwrap();
        let await_hit = tag_at_caret(doc, await_start + 3).unwrap();
        assert_eq!(await_hit.kind.unwrap().form(), MdTagForm::Await);
        assert_eq!(await_hit.ordinal, 1);
    }

    #[test]
    fn done_records_resolve_with_their_original_form() {
        let doc = "#done/2026-10-01/p3/old";
        let hit = tag_at_caret(doc, 3).unwrap();
        assert!(!hit.is_active());
        assert!(hit.kind.is_none());
        assert_eq!(
            hit.done,
            Some(MdDoneKind::Task {
                completed: "2026-10-01".into(),
                priority: 3,
                slug: "old".into(),
            })
        );

        let await_doc = "#done/2026-10-02/2026-12-31/renew";
        let await_hit = tag_at_caret(await_doc, 4).unwrap();
        assert_eq!(
            await_hit.done,
            Some(MdDoneKind::Await {
                completed: "2026-10-02".into(),
                date: "2026-12-31".into(),
                slug: "renew".into(),
            })
        );

        let legacy = "#done/just-a-slug";
        let legacy_hit = tag_at_caret(legacy, 4).unwrap();
        assert!(legacy_hit.kind.is_none());
        assert!(legacy_hit.done.is_none());
    }

    #[test]
    fn respects_boundaries_and_inline_code() {
        assert!(tag_at_caret("see#task/p1/foo", 6).is_none());
        assert!(tag_at_caret("`#task/p1/foo`", 5).is_none());
        assert!(
            !tag_at_caret("#task/p1/foo/bar", 5)
                .map(|h| h.is_active())
                .unwrap_or(false),
            "an over-long tag is rejected by the trailing boundary"
        );
    }

    #[test]
    fn ordinals_count_all_active_tags_even_unrenderable_ones() {
        // The mid-word tag still consumes ordinal 0, so the real tag is 1.
        let doc = "see#task/p1/x then #await/2026-01-01/y";
        let await_start = doc.find("#await").unwrap();
        let hit = tag_at_caret(doc, await_start + 2).unwrap();
        assert_eq!(hit.ordinal, 1);
    }

    #[test]
    fn formats_tokens() {
        assert_eq!(
            format_md_token(MdTagForm::PriorityTask, "buy-milk", Some(3), None),
            "#task/p3/buy-milk"
        );
        assert_eq!(
            format_md_token(MdTagForm::DatedTask, "buy-milk", None, Some("2026-01-01")),
            "#task/2026-01-01/buy-milk"
        );
        assert_eq!(
            format_md_token(MdTagForm::Await, "renew", None, Some("2026-12-31")),
            "#await/2026-12-31/renew"
        );
    }

    #[test]
    fn formats_done_tokens_preserving_original_kind() {
        assert_eq!(
            format_done_token(
                MdTagForm::PriorityTask,
                "buy-milk",
                Some(3),
                None,
                "2026-10-31"
            ),
            "#done/2026-10-31/p3/buy-milk"
        );
        assert_eq!(
            format_done_token(
                MdTagForm::DatedTask,
                "buy-milk",
                None,
                Some("2026-01-01"),
                "2026-10-31"
            ),
            "#done/2026-10-31/t2026-01-01/buy-milk"
        );
        assert_eq!(
            format_done_token(
                MdTagForm::Await,
                "renew",
                None,
                Some("2026-12-31"),
                "2026-10-31"
            ),
            "#done/2026-10-31/2026-12-31/renew"
        );
    }
}

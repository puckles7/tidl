//! Shared UI utilities.

use crate::api::ContactDto;

/// True when a mouse event's target is the exact element the listener is
/// bound to, not a descendant.
///
/// Used on modal backdrops to tell an intentional backdrop click apart from
/// the mouseup of a text-selection drag that started inside the dialog: the
/// browser resolves that drag's `click` target to the nearest common
/// ancestor of the mousedown/mouseup targets, which is the backdrop itself —
/// bypassing the dialog's `stop_propagation` entirely. Checking this on
/// `mousedown` (recorded before the drag) catches that case; checking it on
/// `click` alone would not, since both scenarios report the backdrop as the
/// click target.
pub fn mouse_event_targets_self(ev: &leptos::web_sys::MouseEvent) -> bool {
    ev.target() == ev.current_target()
}

/// Format a server-function error into a user-friendly string.
///
/// Handles two layers of noise:
/// - The Leptos `"error running server function: "` wrapper prefix.
/// - HTML gateway responses (502 / 503 / 504) that appear when the backend
///   is unreachable and a reverse-proxy returns an HTML error page instead of
///   a valid server-function response.
pub fn format_server_fn_error(msg: &str) -> String {
    // HTML body in the response → the backend is down or the proxy is routing
    // to nowhere.  Give a human-readable message instead of raw HTML noise.
    if msg.contains("<html>")
        || msg.contains("502 Bad Gateway")
        || msg.contains("503 Service Unavailable")
        || msg.contains("504 Gateway Timeout")
    {
        return "Server is unreachable — please try again later.".to_string();
    }

    let stripped = msg
        .strip_prefix("error running server function: ")
        .unwrap_or(msg)
        .trim();

    if stripped.is_empty() {
        "Something went wrong — please try again.".to_string()
    } else {
        stripped.to_string()
    }
}

/// Sleep for `ms` milliseconds in the browser. No-op on the server.
#[cfg(feature = "hydrate")]
pub async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(win) = web_sys::window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

#[cfg(not(feature = "hydrate"))]
pub async fn sleep_ms(_ms: i32) {}

/// Copy `text` to the clipboard.
///
/// Tries the modern `navigator.clipboard.writeText` first, since it's the
/// only way to copy without moving focus. That API only exists in secure
/// contexts (HTTPS or `localhost`) and its `Promise` can reject even when
/// present — e.g. the document isn't focused, or the permission was denied
/// (found 2026-07-21: an earlier version fired the promise and forgot it,
/// reporting success unconditionally — this silently broke the app-token
/// "Copy" button, leading to a stale/empty paste and 401s on CalDAV remote
/// sources set up with the bad token). So on any failure — API missing or
/// promise rejected — falls back to the legacy `document.execCommand("copy")`
/// path via a hidden, off-screen `<textarea>`: no secure-context or async
/// permission prompt required, since it runs synchronously inside the same
/// click handler that carries the user gesture.
#[cfg(feature = "hydrate")]
pub async fn copy_to_clipboard(text: &str) -> bool {
    if copy_via_clipboard_api(text).await {
        return true;
    }
    copy_via_exec_command(text)
}

#[cfg(feature = "hydrate")]
async fn copy_via_clipboard_api(text: &str) -> bool {
    let Some(win) = web_sys::window() else {
        return false;
    };
    let nav = win.navigator();
    let clipboard = nav.clipboard();
    let clipboard_value: &web_sys::wasm_bindgen::JsValue = clipboard.as_ref();
    if clipboard_value.is_undefined() {
        return false;
    }
    wasm_bindgen_futures::JsFuture::from(clipboard.write_text(text))
        .await
        .is_ok()
}

#[cfg(feature = "hydrate")]
fn copy_via_exec_command(text: &str) -> bool {
    use wasm_bindgen::JsCast;

    let Some(win) = web_sys::window() else {
        return false;
    };
    let Some(doc) = win.document() else {
        return false;
    };
    let Some(body) = doc.body() else {
        return false;
    };
    let Ok(el) = doc.create_element("textarea") else {
        return false;
    };
    let Ok(textarea) = el.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return false;
    };

    textarea.set_value(text);
    // Keep it out of the visible viewport (still needs to be a real,
    // focusable element in the document for `select()`/`execCommand` to
    // find a selection to copy).
    let style = textarea.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("top", "-1000px");
    let _ = style.set_property("left", "-1000px");

    if body.append_child(&textarea).is_err() {
        return false;
    }
    let _ = textarea.focus();
    textarea.select();
    let ok = doc
        .dyn_ref::<web_sys::HtmlDocument>()
        .and_then(|d| d.exec_command("copy").ok())
        .unwrap_or(false);
    let _ = body.remove_child(&textarea);
    ok
}

/// Escape a string for safe inclusion in HTML text/attribute content.
/// Duplicated from `tidl_markdown::parser`'s private helper of the same
/// name rather than shared — that crate is pure/no-DB and can't see the
/// live contact list [`render_note_mentions`] needs, so this can't live
/// there (see that function's doc comment).
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Detects `@Name` mentions in note text that case-insensitively match one
/// of `contact_names`, rendering them as clickable `<button data-mention>`
/// spans (backlog #80) — everything else, including an `@` that doesn't
/// resolve (e.g. `foo@example.com`), is HTML-escaped plain text. Delegate to
/// [`render_note_content`], so it also linkifies bare `http(s)://`/`www.`
/// URLs (backlog #111). Mirrors
/// `tidl_markdown`'s `[[wikilink]]` click-delegation approach (a `<button
/// data-*>`, since the output is injected via `inner_html` and can't carry
/// Leptos `on:click` handlers directly) — kept in `tidl-web` rather than
/// `tidl-markdown` because it needs the current user's live contact list,
/// which that crate (pure, no DB, and not even wasm32-safe as a whole —
/// its `sync_engine`'s `tokio`/`notify` deps don't compile to wasm32) can't
/// provide; this runs entirely client-side against an already-fetched
/// `list_contacts()` result instead.
///
/// A mention only triggers at an `@` that starts the text or follows
/// whitespace (so `foo@example.com` never matches), and only when the
/// character right after the matched name (if any) isn't alphanumeric (so
/// a contact named "Jane" never swallows into "Janet"). Longest name wins
/// when one name is a prefix of another (e.g. "Jane" vs "Jane Doe"), and
/// matching is Unicode-case-insensitive (not just ASCII).
pub fn render_note_mentions(text: &str, contact_names: &[String]) -> String {
    render_note_content(text, contact_names, &[], 0, &Default::default())
}

/// Whether `chars[start..]` begins with the ASCII `needle` compared
/// case-insensitively (URL schemes are case-insensitive).
fn starts_with_ascii_ci(chars: &[char], start: usize, needle: &str) -> bool {
    let mut it = chars[start..].iter();
    needle
        .chars()
        .all(|n| it.next().is_some_and(|c| c.to_ascii_lowercase() == n))
}

/// The end index (exclusive) of a bare URL starting at `start` in `chars`,
/// or `None` when no URL starts there. Recognizes `http://`, `https://`
/// (case-insensitive), and `www.` (linkified to `https://…`).
///
/// The URL runs to the next whitespace, `<`, `>`, or `"`, then trailing
/// sentence punctuation (`. , ; : ! ? '`) is trimmed so "see https://x.com."
/// doesn't link the final period. A trailing closing bracket is trimmed only
/// when it's unbalanced *within* the URL, so a Wikipedia-style
/// `.../Foo_(bar)` keeps its own `)` while `(https://x.com)` drops the
/// wrapper's. At least one character past the prefix is required, so a bare
/// `https://` or `www.` stays plain text.
fn url_span(chars: &[char], start: usize) -> Option<usize> {
    let scheme_len = if starts_with_ascii_ci(chars, start, "https://") {
        8
    } else if starts_with_ascii_ci(chars, start, "http://") {
        7
    } else if starts_with_ascii_ci(chars, start, "www.") {
        4
    } else {
        return None;
    };

    let body_start = start + scheme_len;
    let mut end = body_start;
    while end < chars.len() {
        let c = chars[end];
        if c.is_whitespace() || matches!(c, '<' | '>' | '"') {
            break;
        }
        end += 1;
    }
    if end == body_start {
        return None;
    }

    while end > body_start {
        let c = chars[end - 1];
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'') {
            end -= 1;
            continue;
        }
        if let Some(open) = match c {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        } {
            let opens = chars[start..end].iter().filter(|&&x| x == open).count();
            let closes = chars[start..end].iter().filter(|&&x| x == c).count();
            if closes > opens {
                end -= 1;
                continue;
            }
        }
        break;
    }

    (end > body_start).then_some(end)
}

/// Renders note body text to HTML: `@img:<id>` inline image tokens (backlog
/// #110) become `<img>` tags, bare `http(s)://`/`www.` URLs become clickable
/// links, inline `#task`/`#await`/`#done` tokens (backlog #113) become chips,
/// then `@Name` mentions are rendered exactly as [`render_note_mentions`] does.
/// Extracted so the card can render all of these in one pass rather than
/// chaining separate string transforms (which would escape the generated
/// markup a second time).
///
/// An image token only renders when its `id` is present in `image_ids` (the
/// note's own attached images) — a stale token for a deleted image stays as
/// plain text, as does a staged `@img:t…` placeholder that never got
/// rewritten. `note_id` is only used to build the image route URL. `curves`
/// colours a task chip by its priority (the same scale the Tasks/Focus views
/// use).
pub fn render_note_content(
    text: &str,
    contact_names: &[String],
    image_ids: &[i64],
    note_id: i64,
    curves: &crate::types::PriorityCurves,
) -> String {
    let mut candidates: Vec<Vec<char>> = contact_names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().collect())
        .collect();
    candidates.sort_by_key(|chars| std::cmp::Reverse(chars.len()));

    let chars: Vec<char> = text.chars().collect();

    // Inline `#task`/`#await`/`#done` tokens (backlog #113), keyed by the char
    // index of their leading `#` — built from the scanner's byte offsets so it
    // lines up with the char-indexed loop below. The scanner already enforces
    // a word boundary, so no extra check is needed here. The `uid` mirrors
    // `sync_note_tokens`'s scheme (`note:<id>:<line>:<ordinal>`, the ordinal
    // counting every token on the line) so the card checkbox can complete the
    // right event.
    let mut byte_to_char = vec![0usize; text.len() + 1];
    {
        let mut b = 0usize;
        for (ci, c) in chars.iter().enumerate() {
            for _ in 0..c.len_utf8() {
                byte_to_char[b] = ci;
                b += 1;
            }
        }
        byte_to_char[text.len()] = chars.len();
    }
    let mut line_ord: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let token_map: std::collections::HashMap<usize, RenderToken> =
        crate::note_tokens::scan_note_tokens(text)
            .into_iter()
            .map(|t| {
                let ord = {
                    let e = line_ord.entry(t.line).or_insert(0);
                    let this = *e;
                    *e += 1;
                    this
                };
                let token = RenderToken {
                    end: byte_to_char[t.end],
                    kind: t.kind,
                    uid: format!("note:{note_id}:{}:{ord}", t.line),
                };
                (byte_to_char[t.start], token)
            })
            .collect();

    // Renders the inline span `[start, end)` (absolute char indices into
    // `chars`) to `out` — the pre-#130 whole-text pass, now invoked once per
    // source line so that a bullet item's text gets exactly the same
    // token/mention/URL/image treatment as ordinary prose.
    let render_inline = |out: &mut String, start: usize, end: usize| {
        let mut i = start;
        let mut at_boundary = true;

        while i < end {
            let c = chars[i];
            if let Some(token) = token_map.get(&i) {
                out.push_str(&render_note_token_html(
                    &token.kind,
                    curves,
                    &token.uid,
                    note_id > 0,
                ));
                i = token.end;
                at_boundary = false;
                continue;
            }
            if c == '@' && at_boundary {
                // Inline image token first — its body ("12" or "t1") can't be a
                // contact name, so the two token kinds never conflict.
                if let Some((img_end, body)) = crate::types::note_image_token_at(&chars, i) {
                    if let Ok(image_id) = body.parse::<i64>() {
                        if image_ids.contains(&image_id) {
                            let src = format!("/api/notes/{note_id}/images/{image_id}");
                            // Wrapped in a span carrying the zoomable src (the
                            // card's click delegation opens the lightbox) plus
                            // an expand badge that CSS reveals on hover, when
                            // the image dims.
                            out.push_str(&format!(
                                r#"<span class="note-inline-image-wrap" data-note-image="{src}"><img class="note-inline-image" src="{src}" loading="lazy" alt="" /><span class="note-inline-image-open" aria-hidden="true"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 3 21 3 21 9"></polyline><polyline points="9 21 3 21 3 15"></polyline><line x1="21" y1="3" x2="14" y2="10"></line><line x1="3" y1="21" x2="10" y2="14"></line></svg></span></span>"#
                            ));
                            i = img_end;
                            at_boundary = false;
                            continue;
                        }
                    }
                }

                let matched = candidates.iter().find(|name_chars| {
                    let m_end = i + 1 + name_chars.len();
                    m_end <= end
                        && chars[i + 1..m_end]
                            .iter()
                            .zip(name_chars.iter())
                            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        && chars
                            .get(m_end)
                            .map(|c| !c.is_alphanumeric())
                            .unwrap_or(true)
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

            // Bare URL, at a boundary (start of the span, whitespace, or an
            // opening bracket/quote) so it isn't detected mid-word. Runs after
            // the `@` token checks above, which is what keeps a URL's own `@` —
            // e.g. a `https://x.com/@user` path — from being mistaken for a
            // mention once the whole URL has been consumed in one step.
            let url_at_boundary = i == start
                || chars[i - 1].is_whitespace()
                || matches!(chars[i - 1], '(' | '[' | '{' | '"');
            if url_at_boundary {
                if let Some(url_end) = url_span(&chars, i) {
                    let raw: String = chars[i..url_end].iter().collect();
                    let href = if starts_with_ascii_ci(&chars, i, "www.") {
                        format!("https://{raw}")
                    } else {
                        raw.clone()
                    };
                    out.push_str(&format!(
                        r#"<a class="note-link" href="{}" target="_blank" rel="noopener noreferrer">{}</a>"#,
                        escape_html(&href),
                        escape_html(&raw),
                    ));
                    i = url_end;
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
    };

    // Split into line spans (the `\n` separators excluded) so `- item` lines
    // can be grouped into `<ul>`/`<li>` blocks (backlog #130) while every other
    // line keeps its literal newline for the card's `white-space: pre-wrap`.
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut line_start = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            lines.push((line_start, i));
            line_start = i + 1;
        }
    }
    lines.push((line_start, chars.len()));
    let last = lines.len().saturating_sub(1);

    let mut out = String::with_capacity(text.len() + 32);
    let mut in_list = false;
    // Index of the innermost open `<ul>` (0 = the outermost list).
    let mut list_level = 0usize;

    for (li, &(s, e)) in lines.iter().enumerate() {
        if let Some((indent, content)) = list_item_offsets(&chars[s..e]) {
            let level = indent / 2;
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
                list_level = 0;
                while list_level < level {
                    out.push_str("<ul>");
                    list_level += 1;
                }
            } else if level > list_level {
                while list_level < level {
                    out.push_str("<ul>");
                    list_level += 1;
                }
            } else {
                while level < list_level {
                    out.push_str("</ul>");
                    list_level -= 1;
                }
            }
            out.push_str("<li>");
            render_inline(&mut out, s + content, e);
            out.push_str("</li>");
        } else {
            if in_list {
                loop {
                    out.push_str("</ul>");
                    if list_level == 0 {
                        break;
                    }
                    list_level -= 1;
                }
                in_list = false;
            }
            render_inline(&mut out, s, e);
            // Keep the blank line between two ordinary lines; a following list
            // is separated by the `<ul>`'s block display instead.
            let next_is_list = lines
                .get(li + 1)
                .map(|&(ns, ne)| list_item_offsets(&chars[ns..ne]).is_some())
                .unwrap_or(false);
            if li != last && !next_is_list {
                out.push('\n');
            }
        }
    }
    if in_list {
        loop {
            out.push_str("</ul>");
            if list_level == 0 {
                break;
            }
            list_level -= 1;
        }
    }
    out
}

/// For a source line's char slice, returns `(indent, content_offset)` when the
/// line is a markdown unordered-list item (`- `, `* ` or `+ `, after any
/// leading spaces/tabs). `indent` is the leading-whitespace width (used to
/// derive the nesting level, two spaces per level) and `content_offset` is the
/// index within the slice where the item's content begins. Returns `None` for a
/// non-list line.
fn list_item_offsets(line: &[char]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i < line.len() && (line[i] == ' ' || line[i] == '\t') {
        i += 1;
    }
    if i + 1 < line.len() && matches!(line[i], '-' | '*' | '+') && line[i + 1] == ' ' {
        Some((i, i + 2))
    } else {
        None
    }
}

/// One inline token resolved for rendering: its char end index, parsed kind,
/// and the `events` uid its completion targets (`note:<id>:<line>:<ordinal>`).
struct RenderToken {
    end: usize,
    kind: crate::note_tokens::ScannedTokenKind,
    uid: String,
}

/// Renders one inline note token as a chip (backlog #113/#116) — a shared look
/// whose urgency is carried by a round dot + outline in the priority-curve
/// colour (`--chip-color`), with the text always in the normal readable colour
/// and the exact priority/date in the `title` tooltip. Await chips signal an
/// already-passed date by turning the dot/outline red; done records are
/// struck-through. When `interactive` (a real note card, not a markdown
/// preview), a task chip gets a `data-note-uid` checkbox the card's click
/// delegation completes, and a done chip a `data-note-reopen` checkbox that
/// re-opens it. Await chips carry neither — an await has no completion, it just
/// shows its date and passes when that date rolls over.
fn render_note_token_html(
    kind: &crate::note_tokens::ScannedTokenKind,
    curves: &crate::types::PriorityCurves,
    uid: &str,
    interactive: bool,
) -> String {
    use crate::note_tokens::{slug_to_title, DoneOriginal, ScannedTokenKind};

    match kind {
        ScannedTokenKind::Task { priority, slug } => {
            let color = crate::types::note_priority_urgency_color(*priority, curves);
            let title = escape_html(&slug_to_title(slug));
            let check = if interactive {
                format!(
                    r#"<span class="note-token-check" data-note-uid="{uid}" role="checkbox" aria-checked="false" title="Mark done">☐</span>"#
                )
            } else {
                String::new()
            };
            // One shared chip look (backlog #116): urgency is the round dot +
            // outline (`--chip-color`), the text always stays the normal
            // readable colour. The exact priority lives in the tooltip. The
            // chip span itself carries the token uid (`data-note-token`) so a
            // click on its body can open the edit modal (backlog #121).
            let token_attr = if interactive {
                format!(r#" data-note-token="{uid}""#)
            } else {
                String::new()
            };
            format!(
                r#"<span class="note-token note-token--task"{token_attr} style="--chip-color:{color};" title="{title} (priority {priority})">{check}<span class="note-token-dot"></span><span class="note-token-label">{title}</span></span>"#
            )
        }
        ScannedTokenKind::Await { date, slug } => {
            let expired = *date < chrono::Utc::now().date_naive();
            let title = escape_html(&slug_to_title(slug));
            let class = if expired {
                "note-token note-token--await note-token--expired"
            } else {
                "note-token note-token--await"
            };
            let token_attr = if interactive {
                format!(r#" data-note-token="{uid}""#)
            } else {
                String::new()
            };
            // No checkbox: an await just passes when its date rolls over.
            format!(
                r#"<span class="{class}"{token_attr} title="{title} (due {date})"><span class="note-token-dot"></span><span class="note-token-label">{title}</span><span class="note-token-date">{date}</span></span>"#,
                date = date.format("%Y-%m-%d")
            )
        }
        ScannedTokenKind::Done {
            completed,
            original,
            slug,
        } => {
            let spec = match original {
                DoneOriginal::Task { priority } => format!("p{priority}"),
                DoneOriginal::Await { date } => date.format("%Y-%m-%d").to_string(),
            };
            let title = escape_html(&slug_to_title(slug));
            let check = if interactive {
                format!(
                    r#"<span class="note-token-check note-token-check--done" data-note-reopen="{uid}" role="checkbox" aria-checked="true" title="Reopen">☑</span>"#
                )
            } else {
                r#"<span class="note-token-check note-token-check--done">☑</span>"#.to_string()
            };
            // Body-click target too (backlog #121): the handler resolves the
            // uid and loads it; a done record loads as `None`, so a body click
            // opens nothing (the checkbox stays the reopen affordance).
            let token_attr = if interactive {
                format!(r#" data-note-token="{uid}""#)
            } else {
                String::new()
            };
            format!(
                r#"<span class="note-token note-token--done"{token_attr} title="Done {} ({spec})">{check}<span class="note-token-label">{title}</span></span>"#,
                completed.format("%Y-%m-%d")
            )
        }
    }
}

/// Rewrites `@OldName` mentions in raw note text to `@NewName` for every
/// `(old, new)` pair in `renames` — used by contact merge (backlog #88) to
/// re-point mentions at the surviving contact's final name. Same boundary
/// rule as `render_note_mentions` (an `@` starting the text or following
/// whitespace, the exact name, then a non-alphanumeric or end-of-text
/// character), scanned in a *single* pass so a rename to a short name
/// (e.g. "Jane") can never accidentally re-match text that a longer rename
/// in the same call (e.g. "Jane Doe") already substituted — sequential
/// find-and-replace per pair would risk exactly that. Unlike
/// `render_note_mentions`, this returns the raw stored form, not HTML — no
/// escaping, since the result is written straight back to `notes.text`.
#[cfg(feature = "ssr")]
pub(crate) fn rewrite_mentions(text: &str, renames: &[(String, String)]) -> String {
    if renames.is_empty() {
        return text.to_string();
    }
    let mut candidates: Vec<(Vec<char>, &str)> = renames
        .iter()
        .filter(|(old, _)| !old.is_empty())
        .map(|(old, new)| (old.chars().collect::<Vec<char>>(), new.as_str()))
        .collect();
    candidates.sort_by_key(|(chars, _)| std::cmp::Reverse(chars.len()));

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    let mut at_boundary = true;

    while i < chars.len() {
        let c = chars[i];
        if c == '@' && at_boundary {
            let matched = candidates.iter().find(|(old_chars, _)| {
                let end = i + 1 + old_chars.len();
                end <= chars.len()
                    && chars[i + 1..end]
                        .iter()
                        .zip(old_chars.iter())
                        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                    && chars.get(end).map(|c| !c.is_alphanumeric()).unwrap_or(true)
            });
            if let Some((old_chars, new_name)) = matched {
                out.push('@');
                out.push_str(new_name);
                i += 1 + old_chars.len();
                at_boundary = false;
                continue;
            }
        }
        out.push(c);
        at_boundary = c.is_whitespace();
        i += 1;
    }
    out
}

/// Filters and orders `contacts` for the `@mention` typeahead popup
/// (backlog #80, shared with backlog #91's markdown-editor popup): prefix
/// matches against `display_name` first, then substring matches,
/// alphabetical within each group, capped at 8 results. Matching is
/// case-insensitive; an empty `query` matches everything (so opening the
/// popup with a bare `@` shows the whole list, alphabetically, up to the
/// cap).
pub fn filter_contacts_for_mention(contacts: &[ContactDto], query: &str) -> Vec<ContactDto> {
    let query = query.to_lowercase();
    let mut list: Vec<ContactDto> = contacts
        .iter()
        .filter(|c| query.is_empty() || c.display_name.to_lowercase().contains(&query))
        .cloned()
        .collect();
    list.sort_by(|a, b| {
        let a_pref = !a.display_name.to_lowercase().starts_with(&query);
        let b_pref = !b.display_name.to_lowercase().starts_with(&query);
        a_pref
            .cmp(&b_pref)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    list.truncate(8);
    list
}

/// Case-insensitive search predicate for the Contacts-tab filter (backlog
/// #101): true when `query` appears as a substring in any text field a
/// contact carries, so the filter matches everything the card (and the
/// editor modal) can show rather than just name/org/emails/phones. Covers
/// the display name and its separate name parts, nickname, org, title,
/// notes, categories, and every email/phone/URL/IMPP value, plus each
/// postal-address line. `query` is lowercased internally; an empty `query`
/// matches every contact.
pub fn contact_matches_search(c: &ContactDto, query: &str) -> bool {
    let q = query.to_lowercase();
    if q.is_empty() {
        return true;
    }
    let has = |s: &str| s.to_lowercase().contains(&q);
    let opt = |s: &Option<String>| s.as_deref().map(has).unwrap_or(false);
    has(&c.display_name)
        || opt(&c.nickname)
        || opt(&c.given_name)
        || opt(&c.family_name)
        || opt(&c.middle_name)
        || opt(&c.name_prefix)
        || opt(&c.name_suffix)
        || opt(&c.org)
        || opt(&c.title)
        || opt(&c.notes)
        || c.categories.iter().any(|cat| has(cat))
        || c.emails.iter().any(|m| has(&m.value))
        || c.phones.iter().any(|m| has(&m.value))
        || c.urls.iter().any(|m| has(&m.value))
        || c.impps.iter().any(|m| has(&m.value))
        || c.addresses.iter().any(|a| {
            opt(&a.street)
                || opt(&a.city)
                || opt(&a.region)
                || opt(&a.postal_code)
                || opt(&a.country)
        })
}

/// Finds an in-progress `@mention` trigger at the end of `text_before_caret`
/// (backlog #80, the note editor's typeahead): the query text after the
/// last `@`, when that `@` starts the string or immediately follows
/// whitespace, and the query itself contains no whitespace (so it stops
/// triggering once the user types past the name into more words). Returns
/// `(byte offset of the '@', query)`. Same trigger discipline as
/// [`render_note_mentions`]'s boundary rule, so what starts the popup and
/// what ends up styled agree with each other.
pub fn find_active_mention(text_before_caret: &str) -> Option<(usize, &str)> {
    let at_idx = text_before_caret.rfind('@')?;
    let query = &text_before_caret[at_idx + '@'.len_utf8()..];
    if query.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    let preceded_ok = at_idx == 0
        || text_before_caret[..at_idx]
            .chars()
            .next_back()
            .map(|c| c.is_whitespace())
            .unwrap_or(true);
    if !preceded_ok {
        return None;
    }
    Some((at_idx, query))
}

/// Typeahead trigger for the note editor's `#` token chooser (backlog #113):
/// the partial word after the last `#` when that `#` starts the string or
/// follows whitespace/an opening bracket, and the partial is still a prefix of
/// a token keyword (`task`/`await`/`done`, or a legacy `todo`/`pending`). Once
/// a `/` or whitespace is typed the popup dismisses and hand-typing takes over.
/// Returns `(byte offset of the '#', partial)`.
pub fn find_active_hash(text_before_caret: &str) -> Option<(usize, &str)> {
    let idx = text_before_caret.rfind('#')?;
    let partial = &text_before_caret[idx + 1..];
    if partial.chars().any(|c| c == '/' || c.is_whitespace()) {
        return None;
    }
    let preceded_ok = idx == 0
        || text_before_caret[..idx]
            .chars()
            .next_back()
            .map(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\''))
            .unwrap_or(true);
    if !preceded_ok {
        return None;
    }
    const KEYS: [&str; 5] = ["task", "await", "done", "todo", "pending"];
    let is_prefix = KEYS.iter().any(|k| k.starts_with(partial));
    is_prefix.then_some((idx, partial))
}

/// True when `caret_byte` sits inside or immediately after an inline
/// note-image token (`@img:<id>` / `@img:t<key>`) in `text`. The mention
/// typeahead consults this so placing the caret on an image token never
/// opens the contact popup — an `@img:` token is not a contact mention.
pub fn caret_in_note_image_token(text: &str, caret_byte: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    let mut byte_off = 0usize;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some((end, _)) = crate::types::note_image_token_at(&chars, i) {
                let token_end_byte =
                    byte_off + chars[i..end].iter().map(|c| c.len_utf8()).sum::<usize>();
                if caret_byte > byte_off && caret_byte <= token_end_byte {
                    return true;
                }
                byte_off = token_end_byte;
                i = end;
                continue;
            }
        }
        byte_off += chars[i].len_utf8();
        i += 1;
    }
    false
}

#[cfg(test)]
mod caret_in_note_image_token_tests {
    use super::caret_in_note_image_token;

    #[test]
    fn end_of_token_is_inside() {
        assert!(caret_in_note_image_token("see @img:12 here", 11));
    }

    #[test]
    fn middle_of_token_is_inside() {
        assert!(caret_in_note_image_token("see @img:12 here", 8));
    }

    #[test]
    fn just_after_at_is_inside() {
        assert!(caret_in_note_image_token("see @img:12 here", 5));
    }

    #[test]
    fn before_the_at_is_outside() {
        assert!(!caret_in_note_image_token("see @img:12 here", 3));
    }

    #[test]
    fn after_the_token_is_outside() {
        assert!(!caret_in_note_image_token("see @img:12 here", 12));
    }

    #[test]
    fn plain_mention_is_outside() {
        assert!(!caret_in_note_image_token("see @jane here", 9));
    }

    #[test]
    fn temp_token_is_inside() {
        assert!(caret_in_note_image_token("see @img:t3 here", 11));
    }
}

#[cfg(test)]
mod find_active_mention_tests {
    use super::find_active_mention;

    #[test]
    fn empty_query_right_after_at() {
        assert_eq!(find_active_mention("hi @"), Some((3, "")));
    }

    #[test]
    fn partial_query_after_at() {
        assert_eq!(find_active_mention("hi @jan"), Some((3, "jan")));
    }

    #[test]
    fn at_at_start_of_text() {
        assert_eq!(find_active_mention("@jan"), Some((0, "jan")));
    }

    #[test]
    fn no_at_at_all() {
        assert_eq!(find_active_mention("just some text"), None);
    }

    #[test]
    fn query_with_whitespace_is_not_active() {
        assert_eq!(find_active_mention("hi @jane doe"), None);
    }

    #[test]
    fn at_not_preceded_by_whitespace_is_not_active() {
        assert_eq!(find_active_mention("foo@bar"), None);
    }

    #[test]
    fn uses_the_last_at_when_several_are_present() {
        assert_eq!(find_active_mention("hi @jane and @bo"), Some((13, "bo")));
    }
}

#[cfg(test)]
mod mention_tests {
    use super::{render_note_mentions, rewrite_mentions};

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_exact_case_insensitive_match() {
        let html = render_note_mentions("ping @jane doe about this", &names(&["Jane Doe"]));
        assert!(html.contains(r#"data-mention="Jane Doe""#));
        assert!(html.contains(">@Jane Doe<"));
    }

    #[test]
    fn leaves_unresolved_mention_as_plain_text() {
        let html = render_note_mentions("ping @Bob about this", &names(&["Jane Doe"]));
        assert!(!html.contains("md-mention"));
        assert!(html.contains("@Bob"));
    }

    #[test]
    fn does_not_match_inside_an_email_address() {
        let html = render_note_mentions("contact foo@example.com", &names(&["Example"]));
        assert!(
            !html.contains("md-mention"),
            "@ must follow whitespace or start-of-text to trigger"
        );
    }

    #[test]
    fn longest_name_wins_when_one_is_a_prefix_of_another() {
        let html = render_note_mentions("@Jane Doe is here", &names(&["Jane", "Jane Doe"]));
        assert!(html.contains(r#"data-mention="Jane Doe""#));
        assert!(!html.contains(r#"data-mention="Jane""#));
    }

    #[test]
    fn does_not_swallow_a_longer_word_sharing_a_prefix() {
        let html = render_note_mentions("@Janet was here", &names(&["Jane"]));
        assert!(
            !html.contains("md-mention"),
            "\"Jane\" must not match inside \"Janet\""
        );
        assert!(html.contains("@Janet"));
    }

    #[test]
    fn escapes_html_in_both_plain_text_and_matched_names() {
        let html = render_note_mentions("<script> @Bob & friends", &names(&["Bob"]));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(r#"data-mention="Bob""#));
        assert!(html.contains("&amp; friends"));
    }

    #[test]
    fn mention_at_start_of_text_matches() {
        let html = render_note_mentions("@Jane hi", &names(&["Jane"]));
        assert!(html.contains(r#"data-mention="Jane""#));
    }

    #[test]
    fn empty_contact_name_is_never_matched() {
        let html = render_note_mentions("@ nobody", &names(&[""]));
        assert!(!html.contains("md-mention"));
    }

    #[test]
    fn rewrite_mentions_replaces_simple_case() {
        let renames = vec![("Jane Doe".to_string(), "Jane Smith".to_string())];
        assert_eq!(
            rewrite_mentions("Ping @Jane Doe about the invoice", &renames),
            "Ping @Jane Smith about the invoice"
        );
    }

    #[test]
    fn rewrite_mentions_respects_boundary_rule() {
        let renames = vec![("example".to_string(), "renamed".to_string())];
        // "foo@example.com" never triggers — same rule as render_note_mentions.
        assert_eq!(
            rewrite_mentions("contact me at foo@example.com", &renames),
            "contact me at foo@example.com"
        );
    }

    #[test]
    fn rewrite_mentions_longest_wins_in_one_pass() {
        let renames = vec![
            ("Jane".to_string(), "J. Public".to_string()),
            ("Jane Doe".to_string(), "Jane Smith".to_string()),
        ];
        // Must not partially match "Jane" inside "Jane Doe" first.
        assert_eq!(
            rewrite_mentions("@Jane Doe and @Jane both here", &renames),
            "@Jane Smith and @J. Public both here"
        );
    }

    #[test]
    fn rewrite_mentions_rewrites_every_occurrence() {
        let renames = vec![("Bob".to_string(), "Robert".to_string())];
        assert_eq!(
            rewrite_mentions("@Bob then @Bob again", &renames),
            "@Robert then @Robert again"
        );
    }

    #[test]
    fn rewrite_mentions_no_op_when_name_absent() {
        let renames = vec![("Nobody Here".to_string(), "New Name".to_string())];
        assert_eq!(
            rewrite_mentions("Just some text @Someone Else", &renames),
            "Just some text @Someone Else"
        );
    }

    #[test]
    fn rewrite_mentions_empty_renames_returns_text_unchanged() {
        assert_eq!(rewrite_mentions("@Jane Doe hello", &[]), "@Jane Doe hello");
    }
}

#[cfg(test)]
mod note_image_render_tests {
    use super::render_note_content;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn renders_a_known_image_token() {
        let html = render_note_content("look @img:12 here", &[], &[12], 7, &Default::default());
        assert!(html.contains(r#"class="note-inline-image""#));
        assert!(html.contains(r#"src="/api/notes/7/images/12""#));
        assert!(!html.contains("@img:12"));
    }

    #[test]
    fn image_token_is_wrapped_for_the_lightbox() {
        let html = render_note_content("look @img:12 here", &[], &[12], 7, &Default::default());
        assert!(html.contains(r#"class="note-inline-image-wrap""#), "{html}");
        assert!(
            html.contains(r#"data-note-image="/api/notes/7/images/12""#),
            "{html}"
        );
        assert!(html.contains(r#"class="note-inline-image-open""#), "{html}");
    }

    #[test]
    fn leaves_an_unknown_image_token_as_text() {
        let html = render_note_content("look @img:99 here", &[], &[12], 7, &Default::default());
        assert!(!html.contains("<img"));
        assert!(html.contains("@img:99"));
    }

    #[test]
    fn leaves_a_temp_token_as_text() {
        let html = render_note_content("look @img:t1 here", &[], &[12], 7, &Default::default());
        assert!(!html.contains("<img"));
        assert!(html.contains("@img:t1"));
    }

    #[test]
    fn image_and_mention_tokens_coexist() {
        let html = render_note_content(
            "@Jane hi @img:3",
            &names(&["Jane"]),
            &[3],
            5,
            &Default::default(),
        );
        assert!(html.contains(r#"data-mention="Jane""#));
        assert!(html.contains(r#"src="/api/notes/5/images/3""#));
    }

    #[test]
    fn escapes_html_around_an_image() {
        let html = render_note_content("<b> @img:1", &[], &[1], 2, &Default::default());
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("<img"));
    }

    #[test]
    fn renders_inline_task_await_and_done_tokens_as_chips() {
        let html = render_note_content(
            "#task/p10/buy-milk #await/2026-12-31/renew #done/2026-10-01/p1/old-thing",
            &[],
            &[],
            1,
            &Default::default(),
        );
        assert!(html.contains("note-token--task"), "{html}");
        assert!(html.contains("Buy Milk"), "{html}");
        assert!(html.contains("note-token--await"), "{html}");
        assert!(html.contains("2026-12-31"), "{html}");
        assert!(html.contains("note-token--done"), "{html}");
        assert!(
            !html.contains("#task/"),
            "raw token text must not survive: {html}"
        );
        assert!(
            !html.contains("#await/"),
            "raw token text must not survive: {html}"
        );
        assert!(
            !html.contains("#done/"),
            "raw token text must not survive: {html}"
        );
    }

    #[test]
    fn task_chip_uses_dot_urgency_and_a_clean_label() {
        let html = render_note_content("#task/p10/buy-milk", &[], &[], 1, &Default::default());
        // Urgency is a round dot + outline driven by `--chip-color`; the text
        // label is the plain title, not a tinted tier word (backlog #116).
        assert!(html.contains("note-token-dot"), "{html}");
        assert!(html.contains("--chip-color:"), "{html}");
        assert!(!html.contains("URGENT"), "no tier label inline: {html}");
        assert!(!html.contains("SOON"), "no tier label inline: {html}");
        assert!(!html.contains("LATER"), "no tier label inline: {html}");
        // Full text (title + priority) is available on hover.
        assert!(html.contains("title=\"Buy Milk (priority 10)\""), "{html}");
    }

    #[test]
    fn does_not_render_a_token_mid_word() {
        let html = render_note_content("see#task/p1/foo", &[], &[], 1, &Default::default());
        assert!(html.contains("see#task/p1/foo"), "{html}");
        assert!(!html.contains("note-token"), "{html}");
    }
}

#[cfg(test)]
mod note_link_render_tests {
    use super::{render_note_content, url_span};

    fn urls_in(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let boundary = i == 0
                || chars[i - 1].is_whitespace()
                || matches!(chars[i - 1], '(' | '[' | '{' | '"');
            if boundary {
                if let Some(end) = url_span(&chars, i) {
                    out.push(chars[i..end].iter().collect());
                    i = end;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    fn render(text: &str) -> String {
        render_note_content(text, &[], &[], 0, &Default::default())
    }

    #[test]
    fn linkifies_https_url() {
        let html = render("see https://example.com now");
        assert!(html.contains(r#"href="https://example.com""#));
        assert!(html.contains(r#">https://example.com</a>"#));
    }

    #[test]
    fn linkifies_http_and_case_insensitive_scheme() {
        let html = render("HTTP://example.com and http://plain.test");
        assert!(html.contains(r#"href="HTTP://example.com""#));
        assert!(html.contains(r#"href="http://plain.test""#));
    }

    #[test]
    fn www_url_gets_https_href_but_keeps_display_text() {
        let html = render("go to www.example.com");
        assert!(html.contains(r#"href="https://www.example.com""#));
        assert!(html.contains(r#">www.example.com</a>"#));
    }

    #[test]
    fn opens_links_in_a_new_tab() {
        let html = render("https://example.com");
        assert!(html.contains(r#"target="_blank""#));
        assert!(html.contains(r#"rel="noopener noreferrer""#));
    }

    #[test]
    fn trims_trailing_sentence_punctuation() {
        assert_eq!(urls_in("see https://example.com."), ["https://example.com"]);
        assert_eq!(
            urls_in("really? https://example.com!"),
            ["https://example.com"]
        );
        assert_eq!(urls_in("https://example.com,"), ["https://example.com"]);
    }

    #[test]
    fn trims_an_unbalanced_closing_paren_but_keeps_a_balanced_one() {
        assert_eq!(urls_in("(https://example.com)"), ["https://example.com"]);
        assert_eq!(
            urls_in("https://en.wikipedia.org/wiki/Foo_(bar)"),
            ["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
    }

    #[test]
    fn keeps_query_strings_and_fragments() {
        assert_eq!(
            urls_in("https://example.com/a?b=1&c=2#frag"),
            ["https://example.com/a?b=1&c=2#frag"]
        );
    }

    #[test]
    fn does_not_linkify_a_bare_scheme() {
        assert!(urls_in("https:// alone").is_empty());
        assert!(urls_in("www. nothing").is_empty());
    }

    #[test]
    fn does_not_linkify_a_url_mid_word() {
        assert!(urls_in("xhttps://example.com").is_empty());
    }

    #[test]
    fn escapes_ampersands_in_the_href_and_display_text() {
        let html = render("https://example.com/?a=1&b=2");
        assert!(html.contains(r#"href="https://example.com/?a=1&amp;b=2""#));
        assert!(html.contains(r#">https://example.com/?a=1&amp;b=2</a>"#));
        assert!(!html.contains("a=1&b=2"));
    }

    #[test]
    fn a_url_is_consumed_whole_so_its_path_is_never_a_mention() {
        let html = render_note_content(
            "https://example.com/@Jane",
            &["Jane".to_string()],
            &[],
            0,
            &Default::default(),
        );
        assert!(html.contains(r#"href="https://example.com/@Jane""#));
        assert!(!html.contains("md-mention"));
    }

    #[test]
    fn links_and_mentions_coexist() {
        let html = render_note_content(
            "@Jane see https://example.com",
            &["Jane".to_string()],
            &[],
            0,
            &Default::default(),
        );
        assert!(html.contains(r#"data-mention="Jane""#));
        assert!(html.contains(r#"href="https://example.com""#));
    }
}

#[cfg(test)]
mod note_bullet_render_tests {
    use super::render_note_content;

    fn render(text: &str) -> String {
        render_note_content(text, &[], &[], 0, &Default::default())
    }

    #[test]
    fn renders_a_bullet_list_as_ul_li() {
        let html = render("- milk\n- eggs");
        assert!(
            html.contains("<ul><li>milk</li><li>eggs</li></ul>"),
            "{html}"
        );
    }

    #[test]
    fn renders_star_and_plus_markers() {
        assert_eq!(render("* milk"), "<ul><li>milk</li></ul>");
        assert_eq!(render("+ milk"), "<ul><li>milk</li></ul>");
    }

    #[test]
    fn nests_by_indentation() {
        let html = render("- a\n  - b\n- c");
        assert_eq!(html, "<ul><li>a</li><ul><li>b</li></ul><li>c</li></ul>");
    }

    #[test]
    fn a_bulleted_token_still_renders_as_a_chip() {
        let html = render("- #task/p10/buy-milk");
        assert!(html.contains("<ul><li>"), "{html}");
        assert!(html.contains("note-token--task"), "{html}");
        assert!(!html.contains("#task/"), "{html}");
    }

    #[test]
    fn prose_around_a_list_is_preserved() {
        let html = render("Shopping\n- milk\nDone.");
        assert!(html.contains("Shopping"), "{html}");
        assert!(html.contains("<ul><li>milk</li></ul>"), "{html}");
        assert!(html.contains("Done."), "{html}");
    }

    #[test]
    fn a_hyphen_without_a_space_is_not_a_list() {
        let html = render("well-known fact");
        assert!(!html.contains("<ul>"), "{html}");
    }
}

/// Stub for the plain `wasm32-unknown-unknown` compile check (`cargo build
/// --target wasm32-unknown-unknown -p tidl-web` in CLAUDE.md's pre-commit
/// checklist runs with no features, so `wasm-bindgen-futures`/`js-sys` —
/// both gated behind `hydrate` — aren't available). Never called at
/// runtime: the real client build always enables `hydrate`.
#[cfg(all(target_arch = "wasm32", not(feature = "hydrate")))]
pub async fn copy_to_clipboard(_text: &str) -> bool {
    false
}

/// Case-insensitive subsequence fuzzy match (telescope/`fzf`-style, backlog
/// #75): every character of `query` must appear in `candidate` in order, but
/// not necessarily contiguously. Returns `None` when `query` doesn't match at
/// all — a filter, not just a ranker. Otherwise returns a score where higher
/// is a better match: consecutive matched characters score far more than
/// ones separated by gaps, so `"cfg"` ranks an exact run in
/// `"src/cfg/mod.rs"` above a scattered hit in `"crates/foo/gen.rs"`, and an
/// empty query matches everything with a score of `0`.
pub fn fuzzy_match_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let candidate_lower = candidate.to_lowercase();
    let cand_chars: Vec<char> = candidate_lower.chars().collect();

    let mut score = 0i32;
    let mut search_from = 0usize;
    let mut prev_match_idx: Option<usize> = None;

    for qc in query.to_lowercase().chars() {
        let idx = cand_chars[search_from..]
            .iter()
            .position(|&c| c == qc)
            .map(|i| i + search_from)?;

        score += if prev_match_idx == Some(idx.wrapping_sub(1)) {
            15
        } else {
            1
        };
        prev_match_idx = Some(idx);
        search_from = idx + 1;
    }

    Some(score)
}

#[cfg(test)]
mod fuzzy_match_score_tests {
    use super::fuzzy_match_score;

    #[test]
    fn empty_query_matches_everything_with_zero_score() {
        assert_eq!(fuzzy_match_score("", "anything.md"), Some(0));
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert_eq!(fuzzy_match_score("xyz", "abc.md"), None);
    }

    #[test]
    fn characters_must_appear_in_order() {
        // "abc" contains c, a, b — but not in the order "cab" requires.
        assert_eq!(fuzzy_match_score("cab", "abc"), None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(fuzzy_match_score("ABC", "abc.md").is_some());
        assert!(fuzzy_match_score("abc", "ABC.MD").is_some());
    }

    #[test]
    fn contiguous_match_scores_higher_than_scattered_match() {
        let contiguous = fuzzy_match_score("cfg", "src/cfg/mod.rs").unwrap();
        let scattered = fuzzy_match_score("cfg", "crates/foo/gen.rs").unwrap();
        assert!(
            contiguous > scattered,
            "contiguous={contiguous} scattered={scattered}"
        );
    }

    #[test]
    fn matches_across_path_separators() {
        assert!(fuzzy_match_score("notesmd", "notes/README.md").is_some());
    }
}

#[cfg(test)]
mod filter_contacts_for_mention_tests {
    use super::filter_contacts_for_mention;
    use crate::api::ContactDto;

    fn contact(display_name: &str) -> ContactDto {
        ContactDto {
            display_name: display_name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_query_returns_all_up_to_the_cap_alphabetically() {
        let contacts = vec![contact("Zoe"), contact("Amy"), contact("Mo")];
        let result = filter_contacts_for_mention(&contacts, "");
        let names: Vec<&str> = result.iter().map(|c| c.display_name.as_str()).collect();
        assert_eq!(names, vec!["Amy", "Mo", "Zoe"]);
    }

    #[test]
    fn prefix_matches_rank_before_substring_matches() {
        let contacts = vec![contact("Bo Jane"), contact("Jane Doe")];
        let result = filter_contacts_for_mention(&contacts, "jane");
        let names: Vec<&str> = result.iter().map(|c| c.display_name.as_str()).collect();
        assert_eq!(names, vec!["Jane Doe", "Bo Jane"]);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let contacts = vec![contact("Jane Doe")];
        let result = filter_contacts_for_mention(&contacts, "JANE");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn non_matching_query_excludes_the_contact() {
        let contacts = vec![contact("Jane Doe")];
        let result = filter_contacts_for_mention(&contacts, "bob");
        assert!(result.is_empty());
    }

    #[test]
    fn result_is_capped_at_eight() {
        let contacts: Vec<ContactDto> = (0..20).map(|i| contact(&format!("Contact {i}"))).collect();
        let result = filter_contacts_for_mention(&contacts, "");
        assert_eq!(result.len(), 8);
    }
}

#[cfg(test)]
mod contact_matches_search_tests {
    use super::contact_matches_search;
    use crate::api::{ContactAddress, ContactDto, ContactMethod};

    fn method(value: &str) -> ContactMethod {
        ContactMethod {
            value: value.to_string(),
            ..Default::default()
        }
    }

    fn base() -> ContactDto {
        ContactDto {
            display_name: "Jane Doe".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(contact_matches_search(&base(), ""));
    }

    #[test]
    fn matches_display_name_case_insensitively() {
        assert!(contact_matches_search(&base(), "jane"));
        assert!(contact_matches_search(&base(), "DOE"));
    }

    #[test]
    fn non_matching_query_is_excluded() {
        assert!(!contact_matches_search(&base(), "bob"));
    }

    #[test]
    fn matches_other_scalar_text_fields() {
        let mut c = base();
        c.nickname = Some("JD".to_string());
        c.title = Some("Engineer".to_string());
        c.org = Some("Acme".to_string());
        c.notes = Some("met at conference".to_string());
        c.categories = vec!["friends".to_string()];
        assert!(contact_matches_search(&c, "jd"));
        assert!(contact_matches_search(&c, "engineer"));
        assert!(contact_matches_search(&c, "acme"));
        assert!(contact_matches_search(&c, "conference"));
        assert!(contact_matches_search(&c, "friends"));
    }

    #[test]
    fn matches_name_parts_not_present_in_display_name() {
        let mut c = base();
        c.given_name = Some("Janet".to_string());
        c.family_name = Some("Smith".to_string());
        assert!(contact_matches_search(&c, "janet"));
        assert!(contact_matches_search(&c, "smith"));
    }

    #[test]
    fn matches_every_method_list_including_urls_and_impps() {
        let mut c = base();
        c.emails = vec![method("jane@acme.test")];
        c.phones = vec![method("555-0100")];
        c.urls = vec![method("https://acme.test/jane")];
        c.impps = vec![method("jane:matrix")];
        assert!(contact_matches_search(&c, "jane@acme"));
        assert!(contact_matches_search(&c, "555-0100"));
        assert!(contact_matches_search(&c, "acme.test/jane"));
        assert!(contact_matches_search(&c, "matrix"));
    }

    #[test]
    fn matches_postal_address_lines() {
        let mut c = base();
        c.addresses = vec![ContactAddress {
            street: Some("1 Main St".to_string()),
            city: Some("Springfield".to_string()),
            region: Some("Illinois".to_string()),
            postal_code: Some("62701".to_string()),
            country: Some("USA".to_string()),
            ..Default::default()
        }];
        assert!(contact_matches_search(&c, "main st"));
        assert!(contact_matches_search(&c, "springfield"));
        assert!(contact_matches_search(&c, "illinois"));
        assert!(contact_matches_search(&c, "62701"));
        assert!(contact_matches_search(&c, "usa"));
    }
}

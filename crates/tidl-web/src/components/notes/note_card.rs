//! A single note card in the Notes-tab grid.

use leptos::prelude::*;

use crate::api::{delete_note, toggle_note_pin, ContactDto, NoteCollectionInfo, NoteInfo};
use crate::app::PriorityCurvesCtx;
use crate::components::delete_confirm_button::{DeleteConfirmModal, DeleteTriggerButton};
use crate::components::drag_grid::{card_style, DragController};
use crate::components::mention_popover::MentionPopover;
use crate::util::render_note_content;

/// Formats an SQLite `updated_at` timestamp ("YYYY-MM-DD HH:MM:SS") as a
/// short relative-time label. Falls back to the raw date portion if parsing fails.
fn relative_time(updated_at: &str) -> String {
    use chrono::NaiveDateTime;
    let Ok(then) = NaiveDateTime::parse_from_str(updated_at, "%Y-%m-%d %H:%M:%S") else {
        return updated_at.get(..10).unwrap_or(updated_at).to_string();
    };
    relative_time_between(then, chrono::Utc::now().naive_utc())
}

/// Pure core of [`relative_time`]: how long `then` is before `now`, rolled up
/// into progressively coarser units so old notes don't read as `3170d ago`
/// (backlog #138). Months/years use fixed 30/365-day approximations — exact
/// enough for a corner-of-the-card label.
fn relative_time_between(then: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> String {
    let mins = (now - then).num_minutes();
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else if mins < 60 * 24 {
        format!("{}h ago", mins / 60)
    } else {
        let days = mins / (60 * 24);
        if days < 7 {
            format!("{days}d ago")
        } else if days < 35 {
            format!("{}w ago", days / 7)
        } else if days < 365 {
            format!("{}mo ago", days / 30)
        } else {
            let years = days / 365;
            let months = (days % 365) / 30;
            if months == 0 {
                format!("{years}y ago")
            } else {
                format!("{years}y {months}mo ago")
            }
        }
    }
}

#[component]
pub fn NoteCard(
    note: NoteInfo,
    collection: NoteCollectionInfo,
    on_pick_up: Callback<leptos::web_sys::PointerEvent>,
    /// `true` only once the pointer has traveled past the click threshold
    /// for *this* card — see `DragController::dragging_id`.
    dragging: Signal<bool>,
    /// Owns the live drag state shared by every zone in this grid — see
    /// `crate::components::drag_grid`'s doc comment for why it isn't
    /// created internally (a shared controller must span zones).
    controller: DragController,
    refresh: RwSignal<u32>,
    /// Effective column count (backlog #50) — 1..=NOTES_GRID_COLS, measured
    /// from the notes-main container's width. Plain, not a signal:
    /// `NotesView`'s render closure already reconstructs every `NoteCard`
    /// whenever this changes (it reads the `cols` signal itself).
    cols: i32,
    /// This zone's per-real-column rendering offset at the current `cols`
    /// (backlog #50, `super::column_offsets`) — index is the real column
    /// (`0..NOTES_GRID_COLS`).
    col_offsets: Vec<i32>,
    /// Manual mode's cluster-aware render position (backlog #102) --
    /// mirrors `contacts::ContactCard`'s identical prop exactly (see its
    /// doc comment). `Some((row, col, span))` overrides
    /// `note.grid_row`/`grid_col`/`grid_row_span` for display only; the
    /// note's own true stored abstract position is untouched (drag
    /// pickup in `NotesView` still reads `note.grid_row`/etc directly).
    /// `None` in Auto mode.
    render_position: Option<(i32, i32, i32)>,
    /// Current user's contact display names, for `@mention` rendering
    /// (backlog #80) — see `NotesView`'s doc comment on the signal this is
    /// fed from.
    contact_names: RwSignal<Vec<String>>,
) -> impl IntoView {
    let pinned = RwSignal::new(note.pinned);
    let done_refresh = crate::app::use_done_refresh();
    #[cfg(not(feature = "hydrate"))]
    let _ = done_refresh;
    let curves = use_context::<PriorityCurvesCtx>()
        .map(|c| c.0.get_untracked())
        .unwrap_or_default();
    let note_id = note.id;
    let (grid_row, grid_col, grid_row_span) =
        render_position.unwrap_or((note.grid_row, note.grid_col, note.grid_row_span));

    let time_label = relative_time(&note.updated_at);
    let title = note.title.clone().filter(|t| !t.is_empty());
    let text = note.text.clone();
    let labels = note.labels.clone();
    // Inline image ids present on this note (backlog #110) — passed to the
    // renderer so `@img:<id>` tokens resolve to `<img>` tags.
    let image_ids: Vec<i64> = note.images.iter().map(|i| i.id).collect();
    let coll_color = collection.color.clone();
    let coll_color2 = coll_color.clone();
    let coll_name = collection.name.clone();

    // A cluster-aware position is already the final render coordinate
    // (Plan A's design) -- CARD_GRID_COLS plus all-zero offsets makes
    // `compact_cell` (inside `card_style`) a no-op pass-through instead
    // of re-compacting an already-compacted position.
    let (style_cols, style_offsets) = match render_position {
        Some(_) => (
            crate::grid::CARD_GRID_COLS,
            vec![0; crate::grid::CARD_GRID_COLS as usize],
        ),
        None => (cols, col_offsets),
    };
    let style = card_style(
        controller,
        dragging,
        note_id,
        grid_row,
        grid_col,
        grid_row_span,
        style_cols,
        style_offsets,
    );

    let on_pin_click = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        let new_val = !pinned.get_untracked();
        pinned.set(new_val);
        leptos::task::spawn_local(async move {
            let _ = toggle_note_pin(note_id, new_val).await;
            refresh.update(|n| *n += 1);
        });
    };

    // Delete without opening the note (backlog #78) — confirmation is a
    // modal (`DeleteConfirmModal`), not a native `confirm()` dialog.
    let delete_armed = RwSignal::new(false);
    let on_delete_confirm = Callback::new(move |_: ()| {
        leptos::task::spawn_local(async move {
            let _ = delete_note(note_id).await;
            refresh.update(|n| *n += 1);
        });
    });

    // `@mention` rendering + click-to-open popover (backlog #80). The
    // rendered text is injected via `inner_html` (mirrors
    // `tidl_markdown`'s `[[wikilink]]` click-delegation in
    // `MarkdownPreviewInner`), so a resolved mention can't carry a Leptos
    // `on:click` of its own — instead one delegated listener on the
    // container walks up from the click target looking for
    // `data-mention`.
    let mention_popover: RwSignal<Option<(ContactDto, f64, f64)>> = RwSignal::new(None);

    // Stops a mention click's — or an inline task/await checkbox's (backlog
    // #112) — `pointerdown` from reaching the card's own drag/pick-up listener
    // (bound on the outer wrapper below), same reasoning/technique as the pin
    // star's `on:pointerdown` handler a few lines down. A linkified URL
    // (backlog #111) is the same story: without this, pressing the link would
    // start a drag, and the pointerup would resolve to a `Click` that opens
    // the note editor — racing (and usually beating) the anchor's own
    // navigation. Clicking anywhere else in the text still bubbles up normally
    // and opens the note editor, unchanged.
    let on_text_pointerdown = move |ev: leptos::web_sys::PointerEvent| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast as _;
            if let Some(target) = ev.target() {
                if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                    let mut node = Some(el);
                    while let Some(n) = node {
                        if n.get_attribute("data-mention").is_some()
                            || n.tag_name().eq_ignore_ascii_case("a")
                            || n.get_attribute("data-note-uid").is_some()
                            || n.get_attribute("data-note-reopen").is_some()
                            || n.get_attribute("data-note-token").is_some()
                            || n.get_attribute("data-note-image").is_some()
                        {
                            ev.stop_propagation();
                            break;
                        }
                        node = n.parent_element();
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    let on_text_click = move |ev: leptos::web_sys::MouseEvent| {
        #[cfg(feature = "hydrate")]
        {
            use crate::api::{complete_todo_item, reopen_note_token, resolve_contact_mention};
            use wasm_bindgen::JsCast as _;
            let Some(target) = ev.target() else {
                return;
            };
            let Ok(el) = target.dyn_into::<web_sys::Element>() else {
                return;
            };

            // Inline token checkbox (backlog #113): complete a task/await, or
            // re-open a done record, straight from the card — the server
            // rewrites the token (and drops/recreates the event), then the note
            // list refetches.
            let mut complete_uid: Option<String> = None;
            let mut reopen_uid: Option<String> = None;
            let mut scan = Some(el.clone());
            while let Some(n) = scan {
                if let Some(u) = n.get_attribute("data-note-uid") {
                    complete_uid = Some(u);
                    break;
                }
                if let Some(u) = n.get_attribute("data-note-reopen") {
                    reopen_uid = Some(u);
                    break;
                }
                scan = n.parent_element();
            }
            if let Some(uid) = complete_uid {
                ev.stop_propagation();
                leptos::task::spawn_local(async move {
                    if complete_todo_item(uid, true).await.is_ok() {
                        refresh.update(|n| *n += 1);
                        done_refresh.update(|n| *n += 1);
                    }
                });
                return;
            }
            if let Some(uid) = reopen_uid {
                ev.stop_propagation();
                leptos::task::spawn_local(async move {
                    if reopen_note_token(uid).await.is_ok() {
                        refresh.update(|n| *n += 1);
                        done_refresh.update(|n| *n += 1);
                    }
                });
                return;
            }

            // Chip body click (backlog #121): open the shared task/await edit
            // modal for this token. A `#done` chip loads as `None`, so a body
            // click opens nothing (the checkbox above is its reopen control).
            let mut token_uid: Option<String> = None;
            let mut scan_tok = Some(el.clone());
            while let Some(n) = scan_tok {
                if let Some(u) = n.get_attribute("data-note-token") {
                    token_uid = Some(u);
                    break;
                }
                scan_tok = n.parent_element();
            }
            if let Some(uid) = token_uid {
                ev.stop_propagation();
                let parts: Vec<&str> = uid.split(':').collect();
                let note_id = parts.get(1).and_then(|s| s.parse::<i64>().ok());
                let line = parts.get(2).and_then(|s| s.parse::<u32>().ok());
                let ordinal = parts
                    .get(3)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                if let (Some(note_id), Some(line)) = (note_id, line) {
                    let todo_modal = crate::app::use_todo_modal();
                    leptos::task::spawn_local(async move {
                        let origin = crate::types::TodoOrigin::Note {
                            note_id,
                            line,
                            ordinal,
                        };
                        if let Ok(Some(chip)) = crate::api::load_todo_chip(origin).await {
                            todo_modal.set(Some(
                                crate::components::focus::TodoModalState::EditChip(chip),
                            ));
                        }
                    });
                }
                return;
            }

            // Inline image click (backlog #140): open the fullscreen
            // lightbox on the image's own URL.
            let mut image_src: Option<String> = None;
            let mut scan_img = Some(el.clone());
            while let Some(n) = scan_img {
                if let Some(src) = n.get_attribute("data-note-image") {
                    image_src = Some(src);
                    break;
                }
                scan_img = n.parent_element();
            }
            if let Some(src) = image_src {
                ev.stop_propagation();
                crate::app::use_image_lightbox().set(Some(src));
                return;
            }

            let mut node = Some(el);
            let mut found: Option<(String, web_sys::DomRect)> = None;
            while let Some(n) = node {
                if let Some(name) = n.get_attribute("data-mention") {
                    found = Some((name, n.get_bounding_client_rect()));
                    break;
                }
                node = n.parent_element();
            }
            let Some((name, rect)) = found else {
                return;
            };
            ev.stop_propagation();
            let x = rect.left();
            let y = rect.bottom() + 4.0;
            leptos::task::spawn_local(async move {
                if let Ok(Some(contact)) = resolve_contact_mention(name).await {
                    mention_popover.set(Some((contact, x, y)));
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    view! {
        <div
            class=move || if dragging.get() { "card-tile note-card card-tile--dragging" } else { "card-tile note-card" }
            style=style
            data-note-id=note_id.to_string()
            data-grid-row=grid_row.to_string()
            data-grid-col=grid_col.to_string()
            data-grid-row-span=grid_row_span.to_string()
            on:pointerdown=move |ev| on_pick_up.run(ev)
        >
            <div class="card-tile-stripe" style=format!("background:{coll_color};")></div>

            <div class="note-card-body">
                <div class="note-card-top">
                    // The actions float right and come first in the DOM so the
                    // title/body text flows *around* the star/trash at the
                    // top-right, then reclaims the full card width below them
                    // (backlog #116 note-card fill). A flex column here instead
                    // narrowed every line for the card's whole height.
                    <div class="note-card-actions">
                        <span
                            class=move || if pinned.get() { "note-pin note-pin--active" } else { "note-pin" }
                            title=move || if pinned.get() { "Unstar note" } else { "Star note" }
                            on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
                            on:click=on_pin_click
                        >"★"</span>
                        <DeleteTriggerButton armed=delete_armed />
                    </div>
                    <div class="note-card-top-main">
                        {title.map(|t| view! {
                            <div class="note-card-title">{t}</div>
                        })}
                        <div
                            class="note-card-text"
                            inner_html=move || render_note_content(&text, &contact_names.get(), &image_ids, note_id, &curves)
                            on:pointerdown=on_text_pointerdown
                            on:click=on_text_click
                        ></div>
                    </div>
                </div>

                {(!labels.is_empty()).then(|| view! {
                    <div class="note-card-labels">
                        {labels.into_iter().map(|label| {
                            let color = label.color.clone();
                            let name = label.name.clone();
                            let name_title = name.clone();
                            view! {
                                <span class="note-label-chip" title=name_title>
                                    <span
                                        class="note-label-dot"
                                        style=format!("background:{color};")
                                    ></span>
                                    {name}
                                </span>
                            }
                        }).collect_view()}
                    </div>
                })}

                <div class="note-card-bottom">
                    <span class="note-card-collection" style=format!("color:{};", coll_color2)>
                        <span class="note-card-collection-dot" style=format!("background:{};", collection.color)></span>
                        {coll_name}
                    </span>
                    <span class="note-card-time">{time_label}</span>
                </div>
            </div>
        </div>

        <DeleteConfirmModal armed=delete_armed noun="note" on_confirm=on_delete_confirm />

        {move || mention_popover.get().map(|(contact, x, y)| view! {
            <MentionPopover
                contact=contact
                x=x
                y=y
                on_close=move || mention_popover.set(None)
            />
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDateTime};

    fn now() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-01-01 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn ago(d: Duration) -> String {
        relative_time_between(now() - d, now())
    }

    #[test]
    fn rolls_up_through_coarser_units() {
        assert_eq!(ago(Duration::seconds(30)), "just now");
        assert_eq!(ago(Duration::minutes(5)), "5m ago");
        assert_eq!(ago(Duration::hours(3)), "3h ago");
        assert_eq!(ago(Duration::days(3)), "3d ago");
        assert_eq!(ago(Duration::days(14)), "2w ago");
        assert_eq!(ago(Duration::days(60)), "2mo ago");
        assert_eq!(ago(Duration::days(365)), "1y ago");
        assert_eq!(ago(Duration::days(400)), "1y 1mo ago");
        assert_eq!(ago(Duration::days(3170)), "8y 8mo ago");
    }

    #[test]
    fn unparseable_falls_back_to_date_portion() {
        assert_eq!(relative_time("garbage"), "garbage");
        assert_eq!(relative_time("2020-06-15 99:99:99"), "2020-06-15");
    }
}

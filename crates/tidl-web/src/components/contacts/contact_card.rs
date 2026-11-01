//! A single contact card in the Contacts-tab grid — sized to its own
//! content (backlog #79), positioned and drag-reordered via the same
//! `crate::components::drag_grid` machinery as `notes::NoteCard` (backlog
//! #82) — Contacts is just a single-zone grid, so it's the simplest
//! possible caller of that shared code (no pinned/non-pinned split, no
//! cross-zone no-drop check).

use leptos::prelude::*;

use crate::api::{delete_contact, ContactCollectionInfo, ContactDto};
use crate::components::delete_confirm_button::{DeleteConfirmModal, DeleteTriggerButton};
use crate::components::drag_grid::{card_style, DragController};

#[component]
pub fn ContactCard(
    contact: ContactDto,
    /// The collection this contact belongs to (backlog #81) — for the
    /// colored stripe/label, mirroring `NoteCard`'s identical prop.
    collection: ContactCollectionInfo,
    on_pick_up: Callback<leptos::web_sys::PointerEvent>,
    /// `true` only once the pointer has traveled past the click threshold
    /// for *this* card — see `DragController::dragging_id`.
    dragging: Signal<bool>,
    controller: DragController,
    refresh: RwSignal<u32>,
    /// Effective column count (backlog #50) — see `NoteCard`'s identical prop.
    cols: i32,
    /// This zone's `grid::column_offsets` at the current `cols` (backlog #50).
    col_offsets: Vec<i32>,
    /// Manual mode's cluster-aware render position (backlog #102) --
    /// `Some((row, col, span))` overrides `contact.grid_row`/`grid_col`/
    /// `grid_row_span` for *display only*; the contact's own true stored
    /// abstract position is untouched (`ContactsView`'s drag pickup
    /// still reads `contact.grid_row`/etc directly, not this override).
    /// `None` in Auto mode, which already produces its own real-space
    /// positions via the unchanged `column_offsets`/`compact_cell` path.
    render_position: Option<(i32, i32, i32)>,
    /// Deep-link target from a `@mention` popover's "Go to contact" button
    /// (backlog #80) — `Some(this card's id)` for a few seconds right
    /// after navigating in, driving a reactive pulse. See `ContactsView`'s
    /// doc comment on the effect that sets/clears this for why the pulse
    /// is a reactive class binding rather than an imperative DOM mutation.
    highlight_id: RwSignal<Option<i64>>,
    /// Merge-select mode (backlog #88) — while `true`, clicking the card
    /// toggles selection instead of opening the editor.
    merge_mode: Signal<bool>,
    /// Whether this card is currently selected for merge.
    selected: Signal<bool>,
    on_toggle_select: Callback<i64>,
) -> impl IntoView {
    let id = contact.id;
    let (grid_row, grid_col, grid_row_span) =
        render_position.unwrap_or((contact.grid_row, contact.grid_col, contact.grid_row_span));
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
        id,
        grid_row,
        grid_col,
        grid_row_span,
        style_cols,
        style_offsets,
    );

    let subtitle = match (&contact.title, &contact.org) {
        (Some(t), Some(o)) if !t.is_empty() && !o.is_empty() => Some(format!("{t} · {o}")),
        (Some(t), _) if !t.is_empty() => Some(t.clone()),
        (_, Some(o)) if !o.is_empty() => Some(o.clone()),
        _ => None,
    };
    let notes = contact.notes.clone().filter(|n| !n.is_empty());
    let display_name = contact.display_name.clone();
    let nickname = contact.nickname.clone().filter(|n| !n.is_empty());
    let has_photo = contact.has_photo;
    // First letter of the display name — the placeholder shown when there's
    // no photo, same idea as any initials-avatar.
    let initial = display_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let emails = contact.emails.clone();
    let phones = contact.phones.clone();
    // Primary URL only, not the full list — the card stays light, full
    // detail lives in the editor modal (same split as emails/phones vs.
    // the modal's `ContactMethodRows` already establishes).
    let primary_url = contact
        .urls
        .iter()
        .find(|u| u.is_primary)
        .or_else(|| contact.urls.first())
        .map(|u| u.value.clone());

    let delete_armed = RwSignal::new(false);
    let on_delete_confirm = Callback::new(move |_: ()| {
        leptos::task::spawn_local(async move {
            let _ = delete_contact(id).await;
            refresh.update(|n| *n += 1);
        });
    });

    view! {
        <div
            class=move || if dragging.get() { "card-tile contact-card card-tile--dragging" } else { "card-tile contact-card" }
            class=("contact-card--highlight", move || highlight_id.get() == Some(id))
            class=("contact-card--merge-selected", move || selected.get())
            style=style
            data-contact-id=id.to_string()
            data-grid-row=grid_row.to_string()
            data-grid-col=grid_col.to_string()
            data-grid-row-span=grid_row_span.to_string()
            on:pointerdown=move |ev| on_pick_up.run(ev)
        >
            {move || merge_mode.get().then(|| view! {
                <input
                    type="checkbox"
                    class="contact-card-merge-checkbox"
                    checked=move || selected.get()
                    on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
                    on:click=move |ev: leptos::ev::MouseEvent| {
                        ev.stop_propagation();
                        on_toggle_select.run(id);
                    }
                />
            })}
            <div class="card-tile-stripe" style=format!("background:{coll_color};")></div>

            <div class="contact-card-body">
                <div class="contact-card-top">
                    <div class="contact-card-top-main">
                        {if has_photo {
                            view! { <img class="contact-card-avatar" src=format!("/api/contacts/{id}/photo") /> }.into_any()
                        } else {
                            view! { <div class="contact-card-avatar-placeholder">{initial}</div> }.into_any()
                        }}
                        <div class="contact-card-name">
                            {display_name}
                            {nickname.map(|n| view! { <span class="contact-card-nickname">" (" {n} ")"</span> })}
                        </div>
                    </div>
                    <DeleteTriggerButton armed=delete_armed />
                </div>

                {subtitle.map(|s| view! { <div class="contact-card-subtitle">{s}</div> })}

                {primary_url.map(|u| {
                    let href = u.clone();
                    view! {
                        // Swallow pointerdown here so it never reaches the
                        // card's own handler (wired for click-to-edit /
                        // drag-to-reorder, backlog #92) — lets the anchor's
                        // native click-to-navigate behavior go through
                        // unobstructed instead of opening the editor modal.
                        <a
                            class="contact-card-url"
                            href=href
                            target="_blank"
                            rel="noopener noreferrer"
                            on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
                        >{u}</a>
                    }
                })}

                {(!emails.is_empty()).then(|| view! {
                    <div class="contact-card-methods">
                        {emails.into_iter().map(|m| view! {
                            <div class="contact-card-method">
                                <span class="contact-card-method-kind">{m.kind}</span>
                                <span class="contact-card-method-value">{m.value}</span>
                            </div>
                        }).collect_view()}
                    </div>
                })}

                {(!phones.is_empty()).then(|| view! {
                    <div class="contact-card-methods">
                        {phones.into_iter().map(|m| view! {
                            <div class="contact-card-method">
                                <span class="contact-card-method-kind">{m.kind}</span>
                                <span class="contact-card-method-value">{m.value}</span>
                            </div>
                        }).collect_view()}
                    </div>
                })}

                {notes.map(|n| view! { <div class="contact-card-notes">{n}</div> })}

                <div class="note-card-bottom">
                    <span class="note-card-collection" style=format!("color:{};", coll_color2)>
                        <span class="note-card-collection-dot" style=format!("background:{};", collection.color)></span>
                        {coll_name}
                    </span>
                </div>
            </div>
        </div>

        <DeleteConfirmModal armed=delete_armed noun="contact" on_confirm=on_delete_confirm />
    }
}

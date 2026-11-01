//! Contacts tab — per-user address book, CRUD (backlog #79), organized
//! into shareable collections (backlog #81), with drag-to-reorder
//! (backlog #82).
//!
//! The grid (responsive column count, masonry placement, content-based
//! row-span self-correction, and — as of #82 — pointer drag-and-drop) is
//! entirely shared with the Notes tab, via `crate::grid` (placement math)
//! and `crate::components::drag_grid` (the interaction layer). One thing
//! stays Contacts-specific: there's no pinned/non-pinned zone split — a
//! single flat masonry grid across every collection the user can access
//! (visibility is a pure filter via `HiddenContactCollectionsCtx`, not a
//! grid partition — same relationship Notes' collections have to its own
//! flat non-pinned zone) — which makes this the *simplest* possible
//! caller of `drag_grid`: one zone (`ZONE`, always `0`), an empty
//! `other_zone_refs` list, so the shared no-drop check trivially never
//! blocks a drop.

mod clear_contacts_modal;
mod contact_card;
pub mod contact_collection_editor_modal;
mod contact_editor_modal;
mod contact_merge_modal;
use clear_contacts_modal::ClearContactsModal;
use contact_card::ContactCard;
use contact_collection_editor_modal::ContactCollectionEditorModal;
pub use contact_editor_modal::ContactEditorModal;
use contact_merge_modal::ContactMergeModal;

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
#[allow(unused_imports)] // only read from the wasm32 half of the height self-correction effect
use std::collections::{HashMap, HashSet};

use crate::api::{
    find_possible_duplicates, list_contact_collections, list_contacts, move_contacts,
    reorder_contact, ContactCollectionInfo, ContactDto,
};
use crate::components::drag_grid::{drop_shadow_view, DragController, DragOutcome};
use crate::components::icons::{GridIcon, MergeIcon, PlusIcon, TrashIcon};
use crate::components::settings::OpenSettingsCtx;
use crate::components::sidebar_pill::SidebarPill;
use crate::grid;
use crate::types::GridPosition;
use crate::util::contact_matches_search;

/// Contact collection IDs (as strings) hidden by the user. Empty = show
/// all. Newtype, not a bare `RwSignal<HashSet<String>>` alias — see
/// `HiddenNoteCollectionsCtx`'s doc comment in `filter_context.rs` for why
/// (a bare alias collides with any other context sharing the same inner
/// type via `TypeId`). Provided once in `AuthenticatedApp` (`app.rs`),
/// same lifetime rationale as `HiddenNoteCollectionsCtx` — a per-route
/// local signal resets on navigation away from `/contacts`.
#[derive(Clone, Copy)]
pub struct HiddenContactCollectionsCtx(pub RwSignal<std::collections::HashSet<String>>);

/// Last collection the user saved a contact to (backlog #81, mirrors
/// `notes::note_editor_modal::LastUsedCollectionCtx`) — lets
/// `ContactEditorModal` default a new contact's collection picker to
/// wherever the user most recently saved one, instead of always the
/// oldest/default collection. Provided by `ContactsView` itself (scoped
/// to this route's mount, unlike `HiddenContactCollectionsCtx` above)
/// since it only needs to survive within one Contacts-tab session.
#[derive(Clone, Copy)]
pub struct LastUsedContactCollectionCtx(pub RwSignal<Option<i64>>);

/// This grid's only zone — see this module's doc comment.
const ZONE: usize = 0;

/// Card width (px) — see `notes::NOTE_CARD_WIDTH`'s doc comment; the gap
/// and max column count are shared via `crate::grid`.
const CONTACT_CARD_WIDTH: f64 = 280.0;

/// Every contact in `contacts`, as `(id, row, col, span)` in Manual mode's
/// own render/view space — mirrors `notes::manual_pickup_items` exactly
/// (see its doc comment).
fn manual_pickup_items(
    contacts: &[ContactDto],
    render_positions: &HashMap<i64, (i32, i32, i32)>,
) -> Vec<(i64, i32, i32, i32)> {
    contacts
        .iter()
        .map(|c| {
            let (r, col, s) = render_positions.get(&c.id).copied().unwrap_or((
                c.grid_row,
                c.grid_col,
                c.grid_row_span,
            ));
            (c.id, r, col, s)
        })
        .collect()
}

#[component]
pub fn ContactsView() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    // Contacts imported from Settings' Import tab (#87) bump this shared
    // signal instead — see `crate::app::ContactsRefresh`'s doc comment.
    let external_refresh = crate::app::use_contacts_refresh();

    provide_context(LastUsedContactCollectionCtx(RwSignal::new(None)));

    let contacts_res = LocalResource::new(move || {
        let _ = refresh.get();
        let _ = external_refresh.get();
        async move { list_contacts().await.unwrap_or_default() }
    });

    // ── Collection sidebar (backlog #81) ──────────────────────────────────────
    let hidden_collections = use_context::<HiddenContactCollectionsCtx>()
        .expect("HiddenContactCollectionsCtx must be provided by AuthenticatedApp")
        .0;
    let open_settings = use_context::<OpenSettingsCtx>()
        .expect("OpenSettingsCtx must be provided by AuthenticatedApp");
    let sidebar_open = RwSignal::new(false);
    let editing_collection: RwSignal<Option<Option<ContactCollectionInfo>>> = RwSignal::new(None);
    let collections_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { list_contact_collections().await.unwrap_or_default() }
    });

    // Possible-duplicates scan (backlog #88) — re-runs on every load/
    // refresh, same LocalResource pattern as `contacts_res`. No
    // persistence for dismissed suggestions (see this plan's Global
    // Constraints) — `dismissed_clusters` below is purely session-local.
    let duplicates_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { find_possible_duplicates().await.unwrap_or_default() }
    });
    let dismissed_clusters: RwSignal<std::collections::HashSet<Vec<i64>>> =
        RwSignal::new(std::collections::HashSet::new());
    let reviewing_cluster: RwSignal<Option<Vec<i64>>> = RwSignal::new(None);

    let search = RwSignal::new(String::new());
    let editing_contact: RwSignal<Option<Option<ContactDto>>> = RwSignal::new(None);
    let on_refresh = Callback::new(move |_| refresh.update(|n| *n += 1));

    // Deep-link target from a `@mention` popover's "Go to contact" button
    // (backlog #80) — `?highlight=<contact_id>` on a fresh full-page
    // navigation, same mechanism as `notes::NotesView`'s `?highlight=` for
    // the note-goto modal (backlog #63).
    let highlight_id: RwSignal<Option<i64>> = RwSignal::new(
        use_query_map()
            .get_untracked()
            .get("highlight")
            .and_then(|s| s.parse().ok()),
    );

    let contacts_main_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let cols: RwSignal<i32> = RwSignal::new(grid::CARD_GRID_COLS);

    // Drag-and-drop (backlog #82) — see this module's doc comment for why
    // this is the simplest possible `DragController` caller (one zone, no
    // cross-zone no-drop check).
    let controller = DragController::new();
    let dragging_id = controller.dragging_id();

    let remeasure_cols = move || {
        if let Some(width) = grid::measure_container_width(contacts_main_ref) {
            let next = grid::effective_cols(
                width,
                CONTACT_CARD_WIDTH,
                grid::CARD_GRID_GAP,
                grid::CARD_GRID_COLS,
            );
            if cols.get_untracked() != next {
                cols.set(next);
            }
        }
    };

    Effect::new(move |_| {
        let _ = contacts_main_ref.get();
        remeasure_cols();
    });

    // A `.vcf` import (Settings modal, #87) places new contacts across the
    // full `grid::CARD_GRID_COLS` real-column space (see `import_vcf`'s doc
    // comment) using each card's *estimated* span (`grid::estimate_span`) —
    // real contacts vary in rendered height in ways the estimate doesn't
    // capture (wrapping names/emails, avatars, …), so a rebalance run
    // immediately against those still-estimated spans just reproduces the
    // same unevenness the estimate already had. `contacts_needs_relayout`
    // flags that a rebalance is owed; it's consumed below, inside the
    // height self-correction pass, so the rebalance always runs against
    // *real* measured spans (whether that pass just corrected some, or
    // found nothing to correct) rather than racing it.
    let contacts_needs_relayout = crate::app::use_contacts_needs_relayout();

    #[cfg(target_arch = "wasm32")]
    {
        // See `notes::NotesView`'s identical listener for why this must be
        // tied to `on_cleanup` — left unregistered, it outlives this
        // component's disposal and panics (crashing the whole WASM module)
        // the next time anything resizes after navigating away from
        // `/contacts`.
        let handle = window_event_listener(leptos::ev::resize, move |_| remeasure_cols());
        on_cleanup(move || handle.remove());
    }

    // "Auto-layout" as a preview-only mode, not a one-shot repack: turning
    // it on never writes anything — it's a client-only repack computed
    // below (`contacts_display`'s effect), same mechanism the search box
    // already uses. Turning it off (without hitting "Keep this layout")
    // just stops rendering the preview; nothing was ever persisted, so
    // there's nothing to restore. `on_auto_layout` below owns the toggle
    // itself; "Keep this layout" is the only path that writes, by
    // persisting the preview's own positions via `move_contacts` (backlog
    // #102 follow-up).
    let auto_layout_enabled = RwSignal::new(false);

    // Guards `contacts_display`'s effect (below) against its own
    // `auto_layout_enabled.set(false)` write during "Keep this layout" --
    // see that effect's doc comment for the exact hazard. Set for the
    // whole round trip, cleared only once the optimistic update this
    // guards has already landed.
    let committing_layout = RwSignal::new(false);

    // Auto-layout follows the search box's empty ⇄ non-empty transition:
    // preserving a filtered-out card's old position once the filter clears
    // isn't useful, so searching turns auto-layout on and clearing the
    // search turns it back off. Edge-triggered on `was_searching` (not a
    // continuous binding to `search`) so a manual toggle mid-search isn't
    // immediately overridden by the next keystroke.
    let was_searching = RwSignal::new(false);
    Effect::new(move |_| {
        let is_searching = !search.get().trim().is_empty();
        if is_searching != was_searching.get_untracked() {
            was_searching.set(is_searching);
            auto_layout_enabled.set(is_searching);
        }
    });

    // Plain signal fed by an Effect off the resource (leptos-suspense
    // skill's "Effect + RwSignal workaround") — the height self-correction
    // pass below needs synchronous local mutation after measuring the DOM,
    // which a Suspend-driven render can't provide. Mirrors
    // `notes::NotesView`'s `pinned_display`/`other_display`.
    let contacts_display: RwSignal<Vec<ContactDto>> = RwSignal::new(Vec::new());

    Effect::new(move |_| {
        let Some(mut contacts) = contacts_res.get() else {
            return;
        };
        // Read unconditionally, before any of the early-return guards
        // below, even though some branches may not end up using their
        // values (found live, 2026-09-28: after a "Keep this layout"
        // commit, hiding a collection stopped hiding anything, and a
        // second "Auto-layout" click silently no-op'd; navigating away and
        // back fixed both). Leptos re-derives an `Effect`'s reactive
        // dependencies fresh on every run, from whichever signals it
        // actually called `.get()` on *that* run -- the `committing_layout`
        // guard below returns early right after a commit, before this
        // effect ever reached `hidden_collections`/`search`/
        // `auto_layout_enabled`/`cols`'s own `.get()` calls further down,
        // which silently dropped all four from this effect's subscription
        // list; a later write to any of them then had nothing left to
        // notify. Reading them here, ahead of every early return, keeps
        // this effect subscribed to all four regardless of which branch
        // (or early return) a given run takes.
        let hidden = hidden_collections.get();
        let query = search.get().to_lowercase();
        let auto = auto_layout_enabled.get();
        let cols_now = cols.get();
        // A background refetch (e.g. another tab's edit) must not clobber
        // an in-flight drag — the drop handler bumps `refresh` itself once
        // it's done (or on failure, to resync). Mirrors `notes::NotesView`.
        if controller.drag.get_untracked().is_some() {
            return;
        }
        // Same hazard, different trigger (backlog #102 follow-up, found via
        // this fix's own regression suite): "Keep this layout" flips
        // `auto_layout_enabled` to `false` right after persisting, which is
        // itself one of this effect's dependencies — without this guard,
        // that flip reruns this effect immediately, rebuilding from
        // `contacts_res`'s still-stale pre-persist snapshot (the refetch is
        // async and hasn't landed yet) and silently reverting the just-
        // persisted positions in the display; a subsequent height self-
        // correction pass then persists *that* stale revert. `get_untracked`
        // deliberately, so clearing this guard doesn't itself trigger a
        // second, redundant rerun.
        if committing_layout.get_untracked() {
            return;
        }
        if !hidden.is_empty() {
            contacts.retain(|c| !hidden.contains(&c.collection_id.to_string()));
        }

        if !query.is_empty() {
            contacts.retain(|c| contact_matches_search(c, &query));

            // Auto-layout is forced on while searching (see `was_searching`
            // above) — repack just this filtered subset tightly, entirely
            // client-side and unpersisted, since the stored positions
            // cover the whole address book and would otherwise leave gaps
            // where a filtered-out neighbor's slot used to sit. Reverts to
            // the stored positions automatically once the search clears,
            // since this branch (and the retain above) only ever runs on
            // a non-empty query.
            if auto {
                let entries: Vec<(i64, i32)> =
                    contacts.iter().map(|c| (c.id, c.grid_row_span)).collect();
                let packed = grid::pack_for_view(
                    &entries,
                    cols_now,
                    &vec![0; grid::CARD_GRID_COLS as usize],
                );
                for c in contacts.iter_mut() {
                    if let Some(&(_, row, col, span)) = packed.iter().find(|&&(id, ..)| id == c.id)
                    {
                        c.grid_row = row;
                        c.grid_col = col;
                        c.grid_row_span = span;
                    }
                }
            }
        } else if auto {
            // The manual "Auto-layout" toggle, not searching: same
            // client-only, unpersisted repack as the search branch above,
            // over whatever's currently visible (i.e. respecting the
            // sidebar's hidden-collection filter) — sorted alphabetically
            // first so the preview matches what "Keep this layout" would
            // actually persist (`auto_layout_contacts_for` repacks the
            // *whole* address book alphabetically, backlog #84); packing
            // only the visible subset here would otherwise assign it
            // positions that could collide with a hidden collection's real
            // ones once unhidden.
            contacts.sort_by(|a, b| {
                grid::manual_order_then(
                    a.manual_order,
                    b.manual_order,
                    a.display_name
                        .to_lowercase()
                        .cmp(&b.display_name.to_lowercase()),
                )
            });
            let entries: Vec<(i64, i32)> =
                contacts.iter().map(|c| (c.id, c.grid_row_span)).collect();
            let packed =
                grid::pack_for_view(&entries, cols_now, &vec![0; grid::CARD_GRID_COLS as usize]);
            for c in contacts.iter_mut() {
                if let Some(&(_, row, col, span)) = packed.iter().find(|&&(id, ..)| id == c.id) {
                    c.grid_row = row;
                    c.grid_col = col;
                    c.grid_row_span = span;
                }
            }
        }
        contacts_display.set(contacts);
    });

    // Manual mode's canonical, resize-independent positions (backlog #106
    // fourth follow-up) -- always `CARD_GRID_COLS`-wide, exactly like Auto
    // mode's own "real" space. Only an actual edit -- a drag, a
    // hidden-contact relocation, a height correction, or "Keep this
    // layout" -- ever writes here; a resize (or toggling the Auto-layout
    // preview on and back off) only ever *reads* it, through the combined
    // effect below, to reset the on-screen cache -- never composed on top
    // of whatever was last displayed. Found live: the previous design
    // patched a *view-space* cache directly (at whatever column count was
    // active when each edit happened), then fed that same cache back in
    // as the "real" input on every later reflow — so squeezing down to
    // one column collapsed every contact's column to 0, and nothing was
    // left to recover the original spread from on the way back up.
    // Mirrors `notes::NotesView`'s identical `pinned_abstract`/
    // `other_abstract` (contacts has only one zone).
    let abstract_positions: RwSignal<HashMap<i64, (i32, i32, i32)>> = RwSignal::new(HashMap::new());

    // Every *accessible* contact's id — deliberately from `contacts_res`
    // (every contact this user can access, matching the render block's
    // own "every accessible contact" input for the hidden-collection-
    // isolation fix), not `contacts_display` (which is visibility/search-
    // filtered) — and deliberately not each contact's own `(row, col,
    // span)`, so a drag (which changes an existing id's position but
    // never this set) leaves this Memo's output unchanged.
    let accessible_ids = Memo::new(move |_| {
        let mut ids: Vec<i64> = contacts_res
            .get()
            .unwrap_or_default()
            .iter()
            .map(|c| c.id)
            .collect();
        ids.sort_unstable();
        ids
    });
    // Keeps `abstract_positions` in sync with *which* contacts exist,
    // surgically (backlog #106 fifth follow-up): a newly-accessible
    // contact gets its raw stored position inserted (a fresh contact's own
    // `grid_row`/`grid_col` is already real-space, same as everything else
    // `contacts_res` returns), a no-longer-accessible one gets removed,
    // and every *already-placed* contact's existing entry is left
    // completely untouched -- this never repacks or re-derives anything
    // from scratch, so adding one contact can't perturb any other
    // contact's stored position.
    Effect::new(move |_| {
        let ids: HashSet<i64> = accessible_ids.get().into_iter().collect();
        let raw_by_id: HashMap<i64, ContactDto> = contacts_res
            .get_untracked()
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        abstract_positions.update(|m| {
            m.retain(|id, _| ids.contains(id));
            for &id in &ids {
                m.entry(id).or_insert_with(|| {
                    raw_by_id
                        .get(&id)
                        .map(|c| (c.grid_row, c.grid_col, c.grid_row_span))
                        .unwrap_or((0, 0, 1))
                });
            }
        });
    });

    // On-screen render positions for Manual mode -- a pure Memo over
    // `abstract_positions` via `grid::compact_cell` (backlog #106 fifth
    // follow-up), empty in Auto mode (where `ContactCard`'s
    // `render_position: None` falls back to the unchanged
    // `column_offsets`/`compact_cell` path it already uses). Recomputed
    // fresh from scratch on every read -- there is no separate "patch this
    // in place" step anymore, because `compact_cell` is a true, exact
    // function of (real position, view width): the same inputs always
    // give the same output, so there's nothing for a stale patch to drift
    // out of sync with.
    let cluster_cache: Memo<HashMap<i64, (i32, i32, i32)>> = Memo::new(move |_| {
        if auto_layout_enabled.get() {
            return HashMap::new();
        }
        let cols_now = cols.get();
        let abstract_now = abstract_positions.get();
        let triples: Vec<(i32, i32, i32)> = abstract_now.values().copied().collect();
        let offsets = grid::column_offsets(&triples, cols_now, 0);
        abstract_now
            .iter()
            .map(|(&id, &(r, c, s))| {
                let (vr, vc) = grid::compact_cell(r, c, cols_now, &offsets);
                (id, (vr, vc, s))
            })
            .collect()
    });

    // Scroll to (centered, when possible) and pulse-highlight a contact
    // deep-linked via `?highlight=` (backlog #80), once it actually shows
    // up in the rendered grid. Mirrors `notes::NotesView`'s identical
    // `?highlight=` effect (backlog #63) for the scroll, but the pulse
    // itself is a *reactive* `class=("contact-card--highlight", ...)`
    // binding on `ContactCard` (keyed on `highlight_id`, passed down
    // below) rather than an imperative `classList.add` here — the height
    // self-correction pass right after this effect can remount the
    // (unkeyed) card list in response to the very same `contacts_display`
    // change that makes the target contact visible for the first time,
    // which would silently wipe a class applied by direct DOM mutation
    // before the user ever saw it. `highlight_id` is cleared after the
    // pulse's real duration (3 x 0.8s) so that reactive binding — and
    // this effect — are genuinely one-shot rather than re-firing on every
    // later, unrelated grid re-render.
    Effect::new(move |_| {
        let Some(id) = highlight_id.get() else {
            return;
        };
        if !contacts_display.get().iter().any(|c| c.id == id) {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::prelude::Closure;
            use leptos::wasm_bindgen::JsCast;

            // Deferred to the next animation frame (matches the
            // self-correction pass below) — a synchronous query here can
            // run before Leptos finishes patching the DOM for a card that
            // just became visible.
            let scroll = Closure::once(move || {
                if let Some(el) = leptos::web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| {
                        d.query_selector(&format!("[data-contact-id='{id}']"))
                            .ok()
                            .flatten()
                    })
                {
                    let el: leptos::web_sys::Element = el.unchecked_into();
                    let opts = leptos::web_sys::ScrollIntoViewOptions::new();
                    opts.set_block(leptos::web_sys::ScrollLogicalPosition::Center);
                    opts.set_behavior(leptos::web_sys::ScrollBehavior::Smooth);
                    el.scroll_into_view_with_scroll_into_view_options(&opts);
                }
            });
            if let Some(win) = leptos::web_sys::window() {
                let _ = win.request_animation_frame(scroll.as_ref().unchecked_ref());
            }
            scroll.forget();

            let clear = Closure::once(move || highlight_id.set(None));
            if let Some(win) = leptos::web_sys::window() {
                let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    clear.as_ref().unchecked_ref(),
                    2500,
                );
            }
            clear.forget();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            highlight_id.set(None);
        }
    });

    // Height self-correction (mirrors `notes::NotesView`'s identical pass —
    // see its doc comment): after every render, measure each visible
    // card's real height, correct any mismatch against its stored
    // `grid_row_span`, and persist the result via `move_contacts`. No
    // reserved column-0 floor (single flat zone, no add-tile chrome).
    //
    // `pending_correction_raf` (backlog #102 follow-up, found live: without
    // this, several rapid actions in a row -- hide a collection, toggle
    // Auto-layout, click Keep this layout -- each schedule their own
    // `requestAnimationFrame`, and *all* of them eventually fire, not just
    // the latest; the last one to actually run wins, silently persisting
    // whatever it captured over anything a later action already did) holds
    // the most recently scheduled RAF's id so a new one scheduled before
    // the last one fired can cancel it — at most one measurement pass is
    // ever actually pending at a time.
    #[allow(unused_variables)] // only read on the wasm32 half of this effect
    let pending_correction_raf: std::rc::Rc<std::cell::Cell<Option<i32>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    Effect::new(move |_| {
        // Tracked only to know *that* the card list changed enough to
        // warrant a fresh measurement pass -- `measure_and_correct` below
        // re-reads the actual baseline itself at RAF-execution time (see
        // its own doc comment), never this snapshot.
        let _contacts_changed = contacts_display.get();
        if controller.drag.get_untracked().is_some() {
            return; // don't fight an in-progress drag
        }

        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::prelude::Closure;
            use leptos::wasm_bindgen::JsCast;

            let measure_and_correct = move || {
                let Some(window) = leptos::web_sys::window() else {
                    return;
                };
                let Some(document) = window.document() else {
                    return;
                };
                let Ok(list) = document.query_selector_all(".contact-card") else {
                    return;
                };
                let mut measured: HashMap<i64, i32> = HashMap::new();
                for i in 0..list.length() {
                    let Some(node) = list.item(i) else { continue };
                    let el: web_sys::Element = node.unchecked_into();
                    let Some(id_str) = el.get_attribute("data-contact-id") else {
                        continue;
                    };
                    let Ok(id) = id_str.parse::<i64>() else {
                        continue;
                    };
                    let height = el.get_bounding_client_rect().height();
                    let span = ((height + grid::CARD_GRID_GAP) / grid::CARD_GRID_ROW_UNIT_PX)
                        .ceil()
                        .max(1.0) as i32;
                    measured.insert(id, span);
                }

                // Re-read `contacts_display` fresh here (`get_untracked`),
                // rather than reusing the effect's own outer `contacts`
                // snapshot: a headless/backgrounded tab can throttle
                // `requestAnimationFrame` by seconds, and this closure isn't
                // cancelled if a *later* effect run schedules a newer one
                // (backlog #102 follow-up, found live: this stale baseline
                // let a page-load-time correction persist minutes later,
                // silently reverting a "Keep this layout" commit that had
                // already landed in between). The live DOM measurement
                // below was already always read fresh at RAF time; only the
                // *baseline* it diffs against was stale.
                // Correct the abstract (real-space) positions directly
                // (backlog #106 fifth follow-up) -- a measured height is a
                // property of the contact's rendered content, not of which
                // space it's placed in, and the abstract is now the single
                // source of truth Manual mode's on-screen cache is purely
                // derived from, so there's no separate render-space
                // correction to keep in sync anymore.
                let abstract_now = abstract_positions.get_untracked();
                let abstract_entries: Vec<(i64, i32, i32, i32)> = abstract_now
                    .iter()
                    .map(|(&id, &(r, c, s))| (id, r, c, s))
                    .collect();
                let zone_measured: HashMap<i64, i32> = abstract_entries
                    .iter()
                    .filter_map(|&(id, _, _, stored)| {
                        let real = *measured.get(&id)?;
                        (real != stored).then_some((id, real))
                    })
                    .collect();
                // Only row/span, preserving each id's existing column
                // (`correct_spans` never changes it) -- NOT a full
                // re-cluster (backlog #106, reverted after live testing,
                // before the fifth follow-up replaced clustering with
                // `compact_cell`/`expand_cell` entirely): a height
                // correction changing just one member's row (to fix a real
                // content-height mismatch, nothing the user asked to
                // rearrange) must never reorder or reshuffle anything
                // else.
                let abstract_moves = if zone_measured.is_empty() {
                    Vec::new()
                } else {
                    grid::correct_spans(&abstract_entries, &zone_measured, 0)
                };
                if !abstract_moves.is_empty() {
                    abstract_positions.update(|m| {
                        for mv in &abstract_moves {
                            if let Some(entry) = m.get_mut(&mv.id) {
                                entry.0 = mv.grid_row;
                                entry.2 = mv.grid_row_span;
                            }
                        }
                    });
                    // `contacts_display`'s own `grid_row`/`grid_row_span`
                    // mirror the correction too, but *only* outside the
                    // Auto-layout preview: while previewing, `contacts_display`
                    // deliberately holds `pack_for_view`'s tight-packed
                    // *view*-space positions -- every card's row there is a
                    // function of *every* card's span (each column is a
                    // running cumulative offset), so patching just one
                    // card's span in place without redoing the whole pack
                    // leaves it inconsistent with the *other* cards' rows,
                    // which were computed on the assumption of its old span
                    // (found live: this is what caused two Auto-layout-
                    // preview cards to render exactly on top of each other,
                    // right after "Keep this layout" -- a regression from an
                    // earlier attempt at this same fix that patched span
                    // unconditionally, reasoning it was "space-independent"
                    // and therefore safe; span *is* space-independent in
                    // isolation, but `pack_for_view`'s row assignment for
                    // every other card in the column is not). Real
                    // (non-preview) mode reads these fields directly as
                    // real-space positions, and `manual_pickup_items` falls
                    // back to them for a contact the cache hasn't placed
                    // yet, so both need the corrected, real-space values
                    // then. The Auto-layout preview instead gets the
                    // corrected span the moment `contacts_res` refetches
                    // (see the unconditional `refresh` bump below), which
                    // reruns the whole `pack_for_view` call fresh.
                    if !auto_layout_enabled.get_untracked() {
                        contacts_display.update(|list| {
                            for c in list.iter_mut() {
                                if let Some(m) = abstract_moves.iter().find(|m| m.id == c.id) {
                                    c.grid_row = m.grid_row;
                                    c.grid_row_span = m.grid_row_span;
                                }
                            }
                        });
                    }
                }

                // Never persist — or run the post-import rebalance below —
                // while a search filter is narrowing what's rendered:
                // `moves` (and `contacts` itself) only reflect the
                // *visible* subset (and, while auto-layout is
                // search-driven, ephemeral tight-packed positions computed
                // just for that subset), so writing either to the server
                // would silently overwrite the real, full-address-book
                // layout with search-scoped positions the moment they
                // diverge even slightly from a fresh height measurement.
                // The local `contacts_display` patch above still keeps the
                // on-screen packing visually correct during the search.
                // `contacts_needs_relayout` is left set if skipped here, so
                // it fires later once this effect re-runs against the
                // unfiltered view instead of being silently dropped.
                if !search.get_untracked().trim().is_empty() {
                    return;
                }

                // A pending post-import rebalance (see
                // `contacts_needs_relayout`'s doc comment above) is always
                // consumed from here, never on its own — this is the first
                // point real (rendered) spans are known, whether this pass
                // just corrected some or found nothing to correct.
                let needs_relayout = contacts_needs_relayout.get_untracked();
                if abstract_moves.is_empty() && !needs_relayout {
                    return;
                }
                if needs_relayout {
                    contacts_needs_relayout.set(false);
                }
                let view_cols = cols.get_untracked();
                let persist: Vec<GridPosition> = abstract_moves
                    .into_iter()
                    .map(|m| GridPosition {
                        id: m.id,
                        grid_row: m.grid_row,
                        grid_col: m.grid_col,
                        grid_row_span: m.grid_row_span,
                    })
                    .collect();
                leptos::task::spawn_local(async move {
                    // Bumped on a *successful* persist too, not just a
                    // failed one (backlog #106 sixth follow-up, found live):
                    // `contacts_res` is `contacts_display`'s only source for
                    // `grid_row_span` -- the rebuild effect above re-derives
                    // `contacts_display` from it fresh on every
                    // `auto_layout_enabled`/`hidden_collections`/`search`/
                    // `cols` change, discarding any in-memory-only
                    // correction patch -- so a span correction that isn't
                    // reflected back into the resource is invisible the
                    // moment any of those fire (e.g. clicking Auto-layout
                    // right after a card is created with an estimated,
                    // not yet measured, span): `pack_for_view` then packs
                    // using the stale estimate and overlaps two cards.
                    //
                    // Skipped while `committing_layout` is up (found live:
                    // unconditionally bumping here raced with "Keep this
                    // layout"'s own careful RAF-deferred refetch-guard dance
                    // -- see that handler's doc comment -- reintroducing the
                    // exact stale-revert hazard that guard exists to
                    // prevent, deterministically overlapping two cards right
                    // after a commit). The commit's own success path bumps
                    // `refresh` itself once it's safe to.
                    let mut needs_refresh =
                        !persist.is_empty() && !committing_layout.get_untracked();
                    if !persist.is_empty() && move_contacts(persist).await.is_err() {
                        needs_refresh = true;
                    }
                    if needs_relayout {
                        // Fully-qualified, not a top-level `use`: this whole
                        // effect only compiles on the wasm32 target (see the
                        // surrounding `#[cfg(target_arch = "wasm32")]`
                        // block), so a plain top-level import would show as
                        // unused under every native `cargo clippy` run --
                        // this is the only remaining caller now that "Keep
                        // this layout" persists the preview directly via
                        // `move_contacts` instead (backlog #102 follow-up).
                        let _ = crate::api::auto_layout_contacts(view_cols).await;
                        needs_refresh = true;
                    }
                    if needs_refresh {
                        refresh.update(|n| *n += 1);
                    }
                });
            };

            let closure = Closure::once(measure_and_correct);
            if let Some(win) = leptos::web_sys::window() {
                if let Some(stale_id) = pending_correction_raf.get() {
                    let _ = win.cancel_animation_frame(stale_id);
                }
                if let Ok(new_id) = win.request_animation_frame(closure.as_ref().unchecked_ref()) {
                    pending_correction_raf.set(Some(new_id));
                }
            }
            closure.forget();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (_contacts_changed, contacts_needs_relayout);
        }
    });

    // While searching, auto-layout is forced on (see the `was_searching`
    // effect above) and must stay that way — toggling it off mid-search
    // leaves manual dragging broken, since filtered-out cards still occupy
    // their real (stored) grid positions that the drag/placement math has
    // to account for (backlog #93). The button below is `disabled` for
    // this too; the guard here is defense in depth.
    let is_searching = Signal::derive(move || !search.get().trim().is_empty());
    let on_auto_layout = move |_: leptos::ev::MouseEvent| {
        if is_searching.get_untracked() {
            return;
        }
        // Nothing was ever persisted while the preview was on, so exiting
        // it (without "Keep this layout") is just: stop previewing.
        // `auto_layout_enabled` is one of the combined abstract/cache
        // effect's own reactive deps (backlog #106 fourth follow-up), so
        // flipping it back to `false` here makes that effect reset
        // `cluster_cache` fresh from `abstract_positions` on its very next
        // run -- no separate trigger/ordering bookkeeping needed, since
        // the abstract map was never touched by entering or leaving the
        // preview in the first place.
        auto_layout_enabled.set(!auto_layout_enabled.get_untracked());
    };

    // Commits the current preview (backlog #102 follow-up, assertion
    // "Keep this layout must persist exactly what was previewed"):
    // persists `contacts_display` itself — the exact visible-subset
    // alphabetical pack already on screen — via `move_contacts`, the same
    // primitive a drag commit uses, rather than a second server-side
    // repack (`auto_layout_contacts_for` used to always repack the *whole*
    // address book alphabetically, which diverged from this preview
    // whenever a collection was hidden, since the preview only ever packs
    // the visible subset). A previously-hidden collection still can't
    // collide with these freshly persisted positions: the same
    // `relocate_if_colliding` pass a drag commit runs handles that here
    // too, rather than sidestepping the problem by repacking everything
    // together.
    let on_keep_layout = move |_: leptos::ev::MouseEvent| {
        let col_weight = ((CONTACT_CARD_WIDTH + grid::CARD_GRID_GAP) / grid::CARD_GRID_ROW_UNIT_PX)
            .round() as i32;
        let contacts_now = contacts_display.get_untracked();

        // `contacts_now`'s own `grid_row`/`grid_col` are *already* real
        // (storage-space) positions, not view-space ones needing
        // translation: `pack_for_view` -- the function that populated them
        // while the Auto-layout preview was showing -- returns real
        // columns by construction (see its own doc comment's "level 2"
        // step, which spreads each visual column's members back across
        // that visual column's *own* real columns). What's actually shown
        // on screen during the preview is computed separately, by
        // `card_style` applying `compact_cell` to these same real values
        // using the current `cols`/`col_offsets` (see `ContactCard`'s
        // `render_position: None` branch) -- the preview never stores a
        // view-space coordinate anywhere to begin with.
        //
        // Previously this ran every entry through `expand_cell` with
        // all-zero offsets, on the mistaken premise that these were
        // view-space positions being "expanded" back to real space (found
        // live, 2026-09-28: "Keep this layout" silently collapsed most
        // cards onto the same handful of real cells -- `expand_cell`'s
        // real-column choice depends only on which *view* column its
        // input falls into, not on the real column already recorded, so
        // feeding it an already-real column as if it were a view column
        // remapped every contact sharing that (accidental) view bucket
        // onto one single real column, all at real row 0). The fix is to
        // persist these values directly -- no translation needed, since
        // there was never a view-space coordinate to translate from.
        let mut real_positions: HashMap<i64, (i32, i32, i32)> = contacts_now
            .iter()
            .map(|c| (c.id, (c.grid_row, c.grid_col, c.grid_row_span)))
            .collect();

        // Checked against the freshly-translated real positions above,
        // matching a hidden contact's own stored space directly.
        let hidden = hidden_collections.get_untracked();
        if !hidden.is_empty() {
            let claimed: Vec<(i64, i32, i32, i32)> = real_positions
                .iter()
                .map(|(&id, &(r, c, s))| (id, r, c, s))
                .collect();
            for c in contacts_res.get_untracked().unwrap_or_default() {
                if !hidden.contains(&c.collection_id.to_string()) {
                    continue;
                }
                if let Some((row, col)) = grid::relocate_if_colliding(
                    &claimed,
                    c.id,
                    c.grid_row,
                    c.grid_col,
                    c.grid_row_span,
                    grid::CARD_GRID_COLS,
                    0,
                    col_weight,
                ) {
                    real_positions.insert(c.id, (row, col, c.grid_row_span));
                }
            }
        }

        let moves: Vec<GridPosition> = real_positions
            .iter()
            .map(|(&id, &(grid_row, grid_col, grid_row_span))| GridPosition {
                id,
                grid_row,
                grid_col,
                grid_row_span,
            })
            .collect();

        committing_layout.set(true);
        leptos::task::spawn_local(async move {
            let ok = move_contacts(moves.clone()).await.is_ok();
            if ok {
                // Apply exactly what was just persisted directly, the same
                // way a drag commit does -- never by leaning on
                // `contacts_res`'s own refetch, which is what raced against
                // `auto_layout_enabled` flipping below in the first place
                // (see `committing_layout`'s doc comment).
                contacts_display.update(|contacts| {
                    for c in contacts.iter_mut() {
                        if let Some(gp) = moves.iter().find(|m| m.id == c.id) {
                            c.grid_row = gp.grid_row;
                            c.grid_col = gp.grid_col;
                        }
                    }
                });
                // `abstract_positions` must reflect this commit
                // immediately (backlog #106 fifth follow-up) -- this is
                // what a later resize, or this same page reloaded fresh,
                // re-derives the display from via `compact_cell`, so it
                // has to be the real-space encoding, not the view-space
                // preview. `cluster_cache` is a pure Memo over this map,
                // so it reflects the commit the moment the very next read
                // happens -- no separate cache write needed.
                abstract_positions.set(real_positions);
            }
            auto_layout_enabled.set(false);
            // Deferred to the next animation frame, not cleared here
            // immediately (backlog #106, found live: "Keep this layout"
            // committed the right positions, but the *rendered* DOM kept
            // showing stale, pre-persist ones). `auto_layout_enabled.set(false)`
            // is one of the `contacts_display`-building Effect's own
            // reactive deps, and that Effect's own
            // `committing_layout.get_untracked()` guard exists precisely
            // to skip the stale `contacts_res` rerun this triggers -- but
            // Leptos doesn't run a dependent Effect synchronously inline
            // with the `.set()` call that dirtied it; the guard's
            // `.set(false)` landing in this same synchronous block, right
            // after, meant that Effect always saw the guard already
            // cleared by the time it actually ran, overwriting this
            // commit's optimistic positions with `contacts_res`'s still-
            // stale (pre-refetch) snapshot. Waiting a frame lets that
            // Effect's guarded rerun (and every other reactive
            // consequence of `auto_layout_enabled` flipping) actually
            // happen first, while the guard is still up.
            #[cfg(target_arch = "wasm32")]
            {
                use leptos::wasm_bindgen::prelude::Closure;
                use leptos::wasm_bindgen::JsCast;
                let closure = Closure::once(move || {
                    committing_layout.set(false);
                });
                if let Some(win) = leptos::web_sys::window() {
                    let _ = win.request_animation_frame(closure.as_ref().unchecked_ref());
                }
                closure.forget();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                committing_layout.set(false);
            }
            if !ok {
                refresh.update(|n| *n += 1);
            }
        });
    };

    // Merge-select mode (backlog #88) — a session-local toggle, mirroring
    // `auto_layout_enabled` above. While active, clicking a card toggles
    // its membership in `merge_selected` instead of opening the editor.
    let merge_mode = RwSignal::new(false);
    let merge_selected: RwSignal<std::collections::HashSet<i64>> =
        RwSignal::new(std::collections::HashSet::new());
    let on_toggle_merge_mode = move |_: leptos::ev::MouseEvent| {
        merge_mode.update(|m| *m = !*m);
        merge_selected.update(|s| s.clear());
    };
    let on_toggle_select = Callback::new(move |id: i64| {
        merge_selected.update(|s| {
            if !s.remove(&id) {
                s.insert(id);
            }
        });
    });
    let show_merge_modal = RwSignal::new(false);

    // "Clear all" (backlog #90) — a modal with a collection picker and an
    // optional "also delete the collection(s)" checkbox, mirroring Notes'
    // `ClearNotesModal` now that contacts live in collections too.
    let confirm_clear_all = RwSignal::new(false);

    let on_contacts_pointer_move = move |ev: leptos::web_sys::PointerEvent| {
        // Single zone, empty `other_zone_refs` — the no-drop check this
        // passes to `pointer_move` trivially never fires (see this
        // module's doc comment).
        controller.pointer_move(&ev, CONTACT_CARD_WIDTH, &[(ZONE, contacts_main_ref)], |_| 0);
    };

    let on_contacts_pointer_up = move |_: leptos::web_sys::PointerEvent| {
        match controller.pointer_up() {
            Some(DragOutcome::Click { item_id }) => {
                if merge_mode.get_untracked() {
                    on_toggle_select.run(item_id);
                } else {
                    let c = contacts_display
                        .get_untracked()
                        .into_iter()
                        .find(|c| c.id == item_id);
                    if let Some(c) = c {
                        editing_contact.set(Some(Some(c)));
                    }
                }
            }
            Some(DragOutcome::Moved {
                item_id, positions, ..
            }) => {
                if auto_layout_enabled.get_untracked() {
                    // Auto mode: the cascade's raw (row, col) positions
                    // have no persisted meaning (see the design spec's
                    // Drag Interaction section) -- used only to derive the
                    // dragged contact's new rank from its neighbors in the
                    // resulting reading order, never applied directly to
                    // `contacts_display` (backlog #102 follow-up, found
                    // live: a contact left behind in the *source* column --
                    // one this drop's own column-scoped cascade never
                    // touches -- stayed at its stale pre-drop row forever,
                    // since nothing ever pulled it up to close the gap,
                    // and a successful `reorder_contact` never bumps
                    // `refresh` to refetch a corrected order either).
                    let cascade_items: Vec<(i64, i32, i32)> = positions
                        .iter()
                        .map(|(&id, &(row, col))| (id, row, col))
                        .collect();
                    let current_order: Vec<i64> = contacts_display
                        .get_untracked()
                        .iter()
                        .map(|c| c.id)
                        .collect();
                    let (predecessor_id, successor_id) =
                        grid::auto_mode_reorder_neighbors(&cascade_items, &current_order, item_id);

                    // Always packed, by construction: reorder is expressed
                    // as an explicit new reading-order list (moving
                    // `item_id` next to whichever neighbor
                    // `auto_mode_reorder_neighbors` found), then the whole
                    // list is fully repacked via `pack_for_view` from that
                    // order -- the exact same primitive the Auto-layout
                    // preview itself uses -- rather than patching just the
                    // few ids this drop's local cascade happened to touch.
                    let mut new_order: Vec<i64> = current_order
                        .into_iter()
                        .filter(|&id| id != item_id)
                        .collect();
                    let insert_at = match (predecessor_id, successor_id) {
                        (Some(pred), _) => new_order
                            .iter()
                            .position(|&id| id == pred)
                            .map(|i| i + 1)
                            .unwrap_or(new_order.len()),
                        (None, Some(succ)) => {
                            new_order.iter().position(|&id| id == succ).unwrap_or(0)
                        }
                        (None, None) => 0,
                    };
                    new_order.insert(insert_at.min(new_order.len()), item_id);

                    let span_by_id: HashMap<i64, i32> = contacts_display
                        .get_untracked()
                        .iter()
                        .map(|c| (c.id, c.grid_row_span))
                        .collect();
                    let entries: Vec<(i64, i32)> = new_order
                        .iter()
                        .map(|&id| (id, span_by_id.get(&id).copied().unwrap_or(1)))
                        .collect();
                    let packed = grid::pack_for_view(
                        &entries,
                        cols.get_untracked(),
                        &vec![0; grid::CARD_GRID_COLS as usize],
                    );
                    let packed_by_id: HashMap<i64, (i32, i32)> = packed
                        .into_iter()
                        .map(|(id, row, col, _span)| (id, (row, col)))
                        .collect();
                    contacts_display.update(|contacts| {
                        for c in contacts.iter_mut() {
                            if let Some(&(row, col)) = packed_by_id.get(&c.id) {
                                c.grid_row = row;
                                c.grid_col = col;
                            }
                        }
                    });

                    leptos::task::spawn_local(async move {
                        if reorder_contact(item_id, predecessor_id, successor_id)
                            .await
                            .is_err()
                        {
                            refresh.update(|n| *n += 1);
                        }
                    });
                } else {
                    // Manual mode: apply optimistically right away -- the
                    // server call below is fire-and-forget except on
                    // failure, when it resyncs from scratch via `refresh`.
                    let span_by_id: HashMap<i64, i32> = contacts_display
                        .get_untracked()
                        .iter()
                        .map(|c| (c.id, c.grid_row_span))
                        .collect();
                    let moves: Vec<GridPosition> = positions
                        .into_iter()
                        .map(|(id, (grid_row, grid_col))| GridPosition {
                            id,
                            grid_row,
                            grid_col,
                            grid_row_span: span_by_id.get(&id).copied().unwrap_or(1),
                        })
                        .collect();

                    // Translate each touched (display-space) position back
                    // to real space via `expand_cell` -- the exact inverse
                    // of `compact_cell` (backlog #106 fifth follow-up).
                    // Computed from the *current* (pre-this-drop) abstract,
                    // which is exactly what the render cache -- and so
                    // this drag's own collision search -- was itself
                    // derived from, so this reproduces the same offsets
                    // `place_with_relocation` resolved this cascade
                    // against. Only the ids this drop actually touched get
                    // written below; every other contact's real position
                    // is left completely untouched, so dragging one card
                    // while narrow can never silently rewrite anyone
                    // else's stored position to a different width's scale.
                    let col_weight = ((CONTACT_CARD_WIDTH + grid::CARD_GRID_GAP)
                        / grid::CARD_GRID_ROW_UNIT_PX)
                        .round() as i32;
                    let cols_now = cols.get_untracked();
                    let abstract_before = abstract_positions.get_untracked();
                    let triples: Vec<(i32, i32, i32)> = abstract_before.values().copied().collect();
                    let offsets = grid::column_offsets(&triples, cols_now, 0);
                    let mut touched: HashMap<i64, (i32, i32, i32)> = moves
                        .iter()
                        .map(|m| {
                            let (real_row, real_col) = grid::expand_cell(
                                m.grid_row, m.grid_col, cols_now, &offsets, &triples,
                            );
                            (m.id, (real_row, real_col, m.grid_row_span))
                        })
                        .collect();

                    // A drop can land exactly on a hidden-collection
                    // member's stored cell — it was never rendered, so
                    // nothing above could have displaced it (backlog #102
                    // follow-up, "dragging onto a hidden card's slot").
                    // Checked against the full real-space occupancy
                    // (existing entries plus this drop's own new ones),
                    // matching a hidden contact's own stored space
                    // directly.
                    let hidden = hidden_collections.get_untracked();
                    if !hidden.is_empty() {
                        let mut claimed_map = abstract_before.clone();
                        for (&id, &pos) in &touched {
                            claimed_map.insert(id, pos);
                        }
                        let claimed: Vec<(i64, i32, i32, i32)> = claimed_map
                            .iter()
                            .map(|(&id, &(r, c, s))| (id, r, c, s))
                            .collect();
                        for c in contacts_res.get_untracked().unwrap_or_default() {
                            if !hidden.contains(&c.collection_id.to_string())
                                || touched.contains_key(&c.id)
                            {
                                continue;
                            }
                            if let Some((row, col)) = grid::relocate_if_colliding(
                                &claimed,
                                c.id,
                                c.grid_row,
                                c.grid_col,
                                c.grid_row_span,
                                grid::CARD_GRID_COLS,
                                0,
                                col_weight,
                            ) {
                                touched.insert(c.id, (row, col, c.grid_row_span));
                            }
                        }
                    }

                    abstract_positions.update(|m| {
                        for (&id, &pos) in &touched {
                            m.insert(id, pos);
                        }
                    });
                    // `contacts_display`'s own `grid_row`/`grid_col`
                    // mirror the same real-space values just written above
                    // -- Auto mode reads these fields directly as real
                    // positions, and `manual_pickup_items` falls back to
                    // them for a contact the cache hasn't placed yet, so
                    // both need the real, not display-space, coordinates.
                    contacts_display.update(|contacts| {
                        for c in contacts.iter_mut() {
                            if let Some(&(row, col, _)) = touched.get(&c.id) {
                                c.grid_row = row;
                                c.grid_col = col;
                            }
                        }
                    });
                    let persist_moves: Vec<GridPosition> = touched
                        .into_iter()
                        .map(|(id, (grid_row, grid_col, grid_row_span))| GridPosition {
                            id,
                            grid_row,
                            grid_col,
                            grid_row_span,
                        })
                        .collect();

                    leptos::task::spawn_local(async move {
                        if move_contacts(persist_moves).await.is_err() {
                            refresh.update(|n| *n += 1);
                        }
                    });
                }
            }
            Some(DragOutcome::Cancelled) | None => {}
        }
    };

    view! {
        <div class="contacts-view">
            <div class="contacts-layout">
                <aside class="notes-sidebar fs-panel">
                    <button
                        class="fs-collapse-btn"
                        on:click=move |_| sidebar_open.update(|b| *b = !*b)
                    >
                        "Contact collections"
                        <span class=move || if sidebar_open.get() {
                            "fs-chevron fs-chevron--open"
                        } else {
                            "fs-chevron"
                        }>"›"</span>
                    </button>
                    <div class=move || if sidebar_open.get() {
                        "fs-content"
                    } else {
                        "fs-content fs-content--collapsed"
                    }>
                        <div class="notes-sidebar-header">
                            <span class="notes-sidebar-title">"CONTACT COLLECTIONS"</span>
                            <button
                                class="notes-sidebar-add"
                                title="New collection"
                                on:click=move |_| editing_collection.set(Some(None))
                            >"+"</button>
                        </div>
                        {move || crate::components::settings::show_sidebar_hints().then(|| view! {
                            <p class="notes-sidebar-hint">"Click a collection to show/hide its contacts."</p>
                        })}

                        <Transition fallback=|| view! { <p class="dialog-hint">"Loading…"</p> }>
                            {move || {
                                let hidden = hidden_collections.get();
                                Suspend::new(async move {
                                    let collections = collections_res.await;
                                    view! {
                                        <ul class="fs-list">
                                            {collections.into_iter().map(|c| {
                                                let id = c.id.to_string();
                                                let id_for_toggle = id.clone();
                                                let c_for_edit = c.clone();
                                                let visible = !hidden.contains(&id);
                                                let is_shared_either_way = !c.is_owner || c.shared;
                                                let share_title = if !c.is_owner {
                                                    format!("Shared by {}", c.owner_username)
                                                } else {
                                                    "Manage sharing".to_string()
                                                };
                                                let on_share: Option<Callback<()>> = is_shared_either_way.then(|| Callback::new(move |_| {
                                                    open_settings.0.set(Some("sharing"));
                                                }));
                                                view! {
                                                    <li class="fs-list-item">
                                                        <SidebarPill
                                                            display_name=c.name.clone()
                                                            color=c.color.clone()
                                                            visible=visible
                                                            on_toggle=Callback::new(move |_| hidden_collections.update(|s| {
                                                                if !s.insert(id_for_toggle.clone()) { s.remove(&id_for_toggle); }
                                                            }))
                                                            on_edit=Some(Callback::new(move |_| {
                                                                editing_collection.set(Some(Some(c_for_edit.clone())));
                                                            }))
                                                            on_share=on_share
                                                            share_title=share_title
                                                        />
                                                    </li>
                                                }
                                            }).collect_view()}
                                        </ul>
                                    }
                                })
                            }}
                        </Transition>
                    </div>
                </aside>

            <div
                class=move || if controller.drag.get().is_some() { "contacts-main contacts-main--dragging" } else { "contacts-main" }
                node_ref=contacts_main_ref
                on:pointermove=on_contacts_pointer_move
                on:pointerup=on_contacts_pointer_up
                on:pointercancel=move |_: leptos::web_sys::PointerEvent| { controller.cancel(); }
            >
                <div class="notes-toolbar">
                    <button class="notes-pin-filter" on:click=move |_| editing_contact.set(Some(None))>
                        <PlusIcon />" New contact"
                    </button>
                    <button
                        class=move || if auto_layout_enabled.get() { "notes-pin-filter notes-pin-filter--active" } else { "notes-pin-filter" }
                        title=move || if is_searching.get() {
                            "Auto-layout stays on while searching"
                        } else {
                            "Preview a tight repack of what's currently visible; click \"Keep this layout\" to save it, or click again (or drag a card) to cancel the preview"
                        }
                        disabled=move || is_searching.get()
                        on:click=on_auto_layout
                    >
                        <GridIcon />" Auto-layout"
                    </button>
                    {move || (auto_layout_enabled.get() && !is_searching.get()).then(|| view! {
                        <button
                            class="notes-pin-filter notes-pin-filter--active"
                            title="Repacks your whole address book alphabetically (not just what's currently visible) so a hidden collection never ends up colliding with these new positions"
                            on:click=on_keep_layout
                        >
                            "Keep this layout"
                        </button>
                    })}
                    <button
                        class=move || if merge_mode.get() { "notes-pin-filter notes-pin-filter--active" } else { "notes-pin-filter" }
                        title="Select contacts to merge into one"
                        on:click=on_toggle_merge_mode
                    >
                        <MergeIcon />" Merge"
                    </button>
                    {move || (merge_mode.get() && merge_selected.get().len() >= 2).then(|| {
                        let count = merge_selected.get().len();
                        view! {
                            <button class="notes-pin-filter notes-pin-filter--active" on:click=move |_| show_merge_modal.set(true)>
                                {format!("Merge ({count})")}
                            </button>
                        }
                    })}
                    // Reuses `merge_mode`'s selection checkboxes (backlog #88) as the
                    // "a selection of contacts" picker for export (backlog #89) rather
                    // than a second, parallel selection UI — a single contact selected
                    // covers #89's "export a single contact" case too, so no separate
                    // per-card export action is needed.
                    {move || (merge_mode.get() && !merge_selected.get().is_empty()).then(|| {
                        let ids: Vec<i64> = merge_selected.get().into_iter().collect();
                        let count = ids.len();
                        let ids_param = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
                        let href = format!("/export/contacts?ids={ids_param}");
                        view! {
                            <a class="notes-pin-filter notes-pin-filter--active" href=href download="contacts.vcf">
                                {format!("⬇ Export ({count})")}
                            </a>
                        }
                    })}
                    {move || (contacts_res.get().map(|c| c.len()).unwrap_or(0) > 0).then(|| view! {
                        <button class="notes-pin-filter" on:click=move |_| confirm_clear_all.set(true)>
                            <TrashIcon />" Clear all"
                        </button>
                    })}
                    <input class="form-input notes-search" type="text" placeholder="Search contacts…"
                        prop:value=move || search.get()
                        on:input=move |ev| search.set(event_target_value(&ev)) />
                </div>

                {move || {
                    let clusters: Vec<Vec<i64>> = duplicates_res.get()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|mut c| { c.sort_unstable(); c })
                        .filter(|c| !dismissed_clusters.get().contains(c))
                        .collect();
                    (!clusters.is_empty()).then(|| view! {
                        <div class="contact-duplicates-banner">
                            <span>{format!("🔍 {} possible duplicate group{} found", clusters.len(), if clusters.len() == 1 { "" } else { "s" })}</span>
                            <div class="contact-duplicates-banner-list">
                                {clusters.clone().into_iter().map(|cluster| {
                                    let names: Vec<String> = contacts_display.get_untracked()
                                        .iter()
                                        .filter(|c| cluster.contains(&c.id))
                                        .map(|c| c.display_name.clone())
                                        .collect();
                                    let cluster_for_review = cluster.clone();
                                    let cluster_for_dismiss = cluster.clone();
                                    view! {
                                        <div class="contact-duplicates-banner-row">
                                            <span>{names.join(" · ")}</span>
                                            <button class="btn-secondary" on:click=move |_| reviewing_cluster.set(Some(cluster_for_review.clone()))>
                                                "Review"
                                            </button>
                                            <button class="btn-secondary" on:click=move |_| dismissed_clusters.update(|s| { s.insert(cluster_for_dismiss.clone()); })>
                                                "Not duplicates"
                                            </button>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        </div>
                    })
                }}

                {move || {
                    if contacts_res.get().is_none() {
                        return view! { <p class="dialog-hint">"Loading contacts…"</p> }.into_any();
                    }
                    let contacts = contacts_display.get();
                    let count = contacts.len();
                    let cols_now = cols.get();
                    let triples: Vec<(i32, i32, i32)> = contacts
                        .iter()
                        .map(|c| (c.grid_row, c.grid_col, c.grid_row_span))
                        .collect();
                    let offsets = grid::column_offsets(&triples, cols_now, 0);
                    // Manual mode's cluster-aware render positions
                    // (backlog #102) -- read straight from `cluster_cache`
                    // rather than recomputed here, so an ordinary drag
                    // (this closure's own most common trigger, via
                    // `contacts_display`) doesn't force a full re-cluster
                    // of the whole grid and silently reshuffle every other
                    // gaggle on screen (backlog #104 follow-up — see
                    // `cluster_cache`'s doc comment). Empty in Auto mode,
                    // where `ContactCard`'s `render_position: None` falls
                    // back to the unchanged `column_offsets`/`compact_cell`
                    // path above.
                    let render_positions: std::sync::Arc<HashMap<i64, (i32, i32, i32)>> =
                        std::sync::Arc::new(cluster_cache.get());
                    let collections_by_id: HashMap<i64, ContactCollectionInfo> = collections_res
                        .get()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| (c.id, c))
                        .collect();

                    view! {
                        <p class="notes-result-count">{count}" contact"{if count == 1 { "" } else { "s" }}</p>

                        {if count == 0 {
                            view! { <p class="notes-empty">"No contacts yet."</p> }.into_any()
                        } else {
                            view! {
                                <div class="notes-grid contacts-grid"
                                    style:grid-template-columns=move || format!("repeat({}, {}px)", cols.get(), CONTACT_CARD_WIDTH as i32)
                                >
                                    {drop_shadow_view(controller, ZONE)}
                                    {contacts.into_iter().map(|c| {
                                        let cid = c.id;
                                        let cspan = c.grid_row_span;
                                        let card_collection = collections_by_id.get(&c.collection_id).cloned().unwrap_or_else(|| ContactCollectionInfo {
                                            id: c.collection_id,
                                            name: "Unknown".to_string(),
                                            color: "#999999".to_string(),
                                            shared: false,
                                            is_owner: false,
                                            members: vec![],
                                            owner_username: String::new(),
                                        });
                                        let render_positions_for_pickup = std::sync::Arc::clone(&render_positions);
                                        let on_pick_up = Callback::new(move |ev: leptos::web_sys::PointerEvent| {
                                            let all_contacts = contacts_display.get_untracked();
                                            let manual = !auto_layout_enabled.get_untracked();
                                            let items: Vec<(i64, i32, i32, i32)> = if manual {
                                                manual_pickup_items(
                                                    &all_contacts,
                                                    &render_positions_for_pickup,
                                                )
                                            } else {
                                                all_contacts
                                                    .iter()
                                                    .map(|x| (x.id, x.grid_row, x.grid_col, x.grid_row_span))
                                                    .collect()
                                            };
                                            controller.pick_up(
                                                &ev, cid, ZONE, cspan, &items,
                                                contacts_main_ref, cols_now, 0,
                                                manual,
                                            );
                                        });
                                        let dragging = Signal::derive(move || dragging_id.get() == Some(cid));
                                        let render_position = render_positions.get(&cid).copied();
                                        view! {
                                            <ContactCard
                                                contact=c
                                                collection=card_collection
                                                on_pick_up=on_pick_up
                                                dragging=dragging
                                                controller=controller
                                                refresh=refresh
                                                cols=cols_now
                                                col_offsets=offsets.clone()
                                                render_position=render_position
                                                highlight_id=highlight_id
                                                merge_mode=Signal::derive(move || merge_mode.get())
                                                selected=Signal::derive(move || merge_selected.get().contains(&cid))
                                                on_toggle_select=on_toggle_select
                                            />
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }}
                    }.into_any()
                }}
            </div>
            </div>
        </div>

        {move || editing_collection.get().map(|coll_opt| {
            view! {
                <ContactCollectionEditorModal
                    collection=coll_opt
                    on_close=move || editing_collection.set(None)
                    on_refresh=on_refresh
                />
            }
        })}

        {move || editing_contact.get().map(|c| view! {
            <ContactEditorModal
                contact=c
                on_close=move || editing_contact.set(None)
                on_refresh=on_refresh
            />
        })}

        <ClearContactsModal
            armed=confirm_clear_all
            collections=Signal::derive(move || collections_res.get().unwrap_or_default())
            on_refresh=on_refresh
        />

        {move || show_merge_modal.get().then(|| {
            let selected_ids = merge_selected.get_untracked();
            let selected_contacts: Vec<ContactDto> = contacts_display
                .get_untracked()
                .into_iter()
                .filter(|c| selected_ids.contains(&c.id))
                .collect();
            view! {
                <ContactMergeModal
                    contacts=selected_contacts
                    on_close=move || {
                        show_merge_modal.set(false);
                        merge_mode.set(false);
                        merge_selected.update(|s| s.clear());
                    }
                    on_refresh=Callback::new(move |_| {
                        refresh.update(|n| *n += 1);
                        merge_mode.set(false);
                        merge_selected.update(|s| s.clear());
                    })
                />
            }
        })}

        {move || reviewing_cluster.get().map(|cluster| {
            let selected_contacts: Vec<ContactDto> = contacts_display
                .get_untracked()
                .into_iter()
                .filter(|c| cluster.contains(&c.id))
                .collect();
            view! {
                <ContactMergeModal
                    contacts=selected_contacts
                    on_close=move || reviewing_cluster.set(None)
                    on_refresh=Callback::new(move |_| {
                        refresh.update(|n| *n += 1);
                        reviewing_cluster.set(None);
                    })
                />
            }
        })}
    }
}

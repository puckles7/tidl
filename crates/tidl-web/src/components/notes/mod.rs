//! Notes tab — collections + ad-hoc notes (backlog #37).

pub mod collection_editor_modal;

pub mod note_label_editor_modal;
pub use note_label_editor_modal::NoteLabelEditorModal;

mod note_editor_modal;
use note_editor_modal::LastUsedCollectionCtx;
pub use note_editor_modal::NoteEditorModal;

mod consolidate_modal;
pub use consolidate_modal::ConsolidateNoteModal;

mod pull_out_modal;
pub use pull_out_modal::PullOutNoteModal;

mod note_card;
use note_card::NoteCard;

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use std::collections::{HashMap, HashSet};

use crate::api::{
    list_contacts, list_note_collections, list_notes, move_notes, reorder_note, NoteCollectionInfo,
    NoteInfo,
};
use crate::app::use_notes_refresh;
use crate::components::drag_grid::{drop_shadow_view, DragController, DragOutcome};
use crate::components::filter_context::{HiddenNoteCollectionsCtx, HiddenNoteLabelsCtx};
use crate::components::icons::{GridIcon, PlusIcon, StarIcon, TrashIcon};
use crate::components::note_collections_panel::NoteCollectionsPanel;
use crate::grid;
use crate::types::GridPosition;

mod clear_notes_modal;
use clear_notes_modal::ClearNotesModal;

/// Card width (px) for both grid zones — the inter-cell gap
/// (`grid::CARD_GRID_GAP`) and max column count (`grid::CARD_GRID_COLS`)
/// are shared with Contacts (backlog #79); card width stays per-feature
/// since it's driven by each card's own content layout.
const NOTE_CARD_WIDTH: f64 = 260.0;

/// Zone identities for the shared `DragController` (backlog #82) — Notes
/// is the two-zone case (pinned always sits above non-pinned); Contacts
/// (`contacts::mod`) has only zone 0.
const ZONE_PINNED: usize = 0;
const ZONE_OTHER: usize = 1;

/// Every note in `notes`, as `(id, row, col, span)` in Manual mode's own
/// render/view space — read straight from the render cache (backlog #106
/// fifth follow-up): the cache always has an entry for every accessible
/// note, since it's a pure function of the abstract map, which is kept in
/// sync with the accessible id set independently of any drag. Fed to
/// `DragController::pick_up` so a Manual-mode drag's pointer resolution
/// and collision search happen in the space the drag is actually rendered
/// in. The fallback is defensive only, for the theoretical gap between a
/// brand-new note appearing and the abstract-populating effect's next run.
fn manual_pickup_items(
    notes: &[NoteInfo],
    render_positions: &HashMap<i64, (i32, i32, i32)>,
) -> Vec<(i64, i32, i32, i32)> {
    notes
        .iter()
        .map(|n| {
            let (r, c, s) = render_positions.get(&n.id).copied().unwrap_or((
                n.grid_row,
                n.grid_col,
                n.grid_row_span,
            ));
            (n.id, r, c, s)
        })
        .collect()
}

#[component]
pub fn NotesView() -> impl IntoView {
    let refresh = RwSignal::new(0u32);

    // Tracks the last collection the user saved a note to (backlog #58), so
    // `NoteEditorModal` can re-use it as the default on the next "+ Take a
    // note…" — mirrors `LastUsedCalendarCtx` in `CalendarView`.
    provide_context(LastUsedCollectionCtx(RwSignal::new(None)));

    // Lets Settings' Keep import (#109) tell an already-mounted Notes tab to
    // refetch, mirroring `ContactsRefresh` for the `.vcf` import.
    let notes_refresh = use_notes_refresh();

    let collections_res = LocalResource::new(move || {
        let _ = refresh.get();
        let _ = notes_refresh.get();
        async move { list_note_collections().await.unwrap_or_default() }
    });
    let notes_res = LocalResource::new(move || {
        let _ = refresh.get();
        let _ = notes_refresh.get();
        async move { list_notes().await.unwrap_or_default() }
    });

    // Contact display names for `@mention` rendering (backlog #80) — a
    // plain signal, not read via `Suspend::new`, so a slow contacts fetch
    // never suspends the notes grid itself; cards just render without
    // mention styling until this resolves. Fed by an `Effect` (the
    // leptos-suspense skill's "Effect + RwSignal workaround") rather than
    // read directly in each `NoteCard`'s render closure, so a contact
    // rename/delete updates every card's mentions independently of the
    // notes list's own refresh cycle.
    let contacts_res =
        LocalResource::new(|| async move { list_contacts().await.unwrap_or_default() });
    let contact_names: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    Effect::new(move |_| {
        if let Some(contacts) = contacts_res.get() {
            contact_names.set(contacts.into_iter().map(|c| c.display_name).collect());
        }
    });

    // Per-viewer visibility toggle. Shares `HiddenNoteCollectionsCtx` with
    // Focus's sidebar rather than owning a local signal —
    // a local `RwSignal` here was reset every time `NotesView` unmounted
    // (i.e. every navigation away from `/notes`), so hiding a collection
    // silently "forgot itself" the moment the user switched tabs and came
    // back. The shared context is provided once in `AuthenticatedApp` and
    // outlives any single route's mount.
    let hidden_collections = use_context::<HiddenNoteCollectionsCtx>()
        .unwrap_or_else(|| HiddenNoteCollectionsCtx(RwSignal::new(HashSet::new())))
        .0;
    let hidden_labels = use_context::<HiddenNoteLabelsCtx>()
        .unwrap_or_else(|| HiddenNoteLabelsCtx(RwSignal::new(HashSet::new())))
        .0;
    let pinned_only = RwSignal::new(false);
    let search = RwSignal::new(String::new());

    // Mobile: sidebar collapses behind a toggle, matching FocusSidebar's panels.
    let sidebar_open = RwSignal::new(false);

    // Modal state: None = closed; Some(None) = create; Some(Some(x)) = edit.
    // (Collection/label editing lives inside the shared `NoteCollectionsPanel`.)
    let editing_note: RwSignal<Option<Option<NoteInfo>>> = RwSignal::new(None);

    let on_refresh = Callback::new(move |_| refresh.update(|n| *n += 1));

    // The shared collections/labels panel refetches off this; folding in
    // `notes_refresh` means a Keep import (which can create labels) also
    // refreshes the panel without remounting NotesView.
    let collections_refresh: Signal<u32> =
        Signal::derive(move || refresh.get() + notes_refresh.get());

    // Deep-link target from the note-goto modal's "Show in collection"
    // button (backlog #63): `?highlight=<note_id>` on a fresh full-page
    // navigation here. `hidden_collections` above always starts empty on
    // a fresh mount (it's session-only, never persisted), so there's
    // nothing to unhide by the time this runs — the note is already going
    // to render.
    let highlight_id: RwSignal<Option<i64>> = RwSignal::new(
        use_query_map()
            .get_untracked()
            .get("highlight")
            .and_then(|s| s.parse().ok()),
    );

    // When the deep-link came from a note task/await's "go to source", the
    // item's `note:{id}:{line}:{ordinal}` uid rides along as `?line=&ordinal=`
    // (backlog #134) so the matching inline chip is flashed instead of the whole
    // card. Absent (a plain "Show in collection", backlog #63) → card highlight.
    let highlight_token: RwSignal<Option<(u32, u32)>> = RwSignal::new({
        let q = use_query_map();
        let line = q.get_untracked().get("line").and_then(|s| s.parse().ok());
        let ordinal = q
            .get_untracked()
            .get("ordinal")
            .and_then(|s| s.parse().ok());
        match (line, ordinal) {
            (Some(line), Some(ordinal)) => Some((line, ordinal)),
            _ => None,
        }
    });

    // ── Drag-and-drop grid placement state (backlog #47, generalized to
    // Contacts too in #82 — see `crate::components::drag_grid`) ────────────
    let controller = DragController::new();
    let dragging_id: Memo<Option<i64>> = controller.dragging_id();

    // The card-list closure below (the big `view!` block that renders every
    // `NoteCard`) must not subscribe to the whole `drag` signal: field writes
    // that churn on most pointermoves (`drag_px`, `last_cell` — see
    // `DragController::pointer_move`) would re-run that block and destroy
    // every card's `<img>` DOM node mid-drag, re-loading each note image —
    // the visible flicker (backlog #110). The block only ever needs two
    // drag-derived facts: whether a cross-zone no-drop is active, and which
    // zone owns the drag. Project just those through a `Memo`: it re-runs on
    // every `drag` write but, unlike the signal, notifies subscribers only
    // when the projected tuple actually changes. The `drag_px >= 5.0` filter
    // keeps a mere pointerdown (below the click/drag threshold) from changing
    // the projection at all. Same narrowing technique as `card_style`'s
    // `overlay_pos` and `DragController::dragging_id`.
    let drag_zone_state: Memo<(bool, Option<usize>)> = Memo::new(move |_| {
        controller
            .drag
            .get()
            .filter(|d| d.drag_px >= 5.0)
            .map_or((false, None), |d| (d.no_drop, Some(d.zone)))
    });

    let pinned_row_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let other_grid_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Effective column count (backlog #50) — measured from `.notes-main`'s
    // own rendered width, not the viewport, so it already reflects the
    // sidebar's 767px collapse breakpoint (assets/style.css) without a
    // second, separate breakpoint to keep in sync. Starts at
    // `grid::CARD_GRID_COLS` (matches this being a CSR app: the first real
    // render already has a DOM to measure, so this corrects itself
    // immediately rather than waiting on an SSR/hydrate mismatch).
    let notes_main_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let cols: RwSignal<i32> = RwSignal::new(grid::CARD_GRID_COLS);

    let remeasure_cols = move || {
        if let Some(width) = grid::measure_container_width(notes_main_ref) {
            let next = grid::effective_cols(
                width,
                NOTE_CARD_WIDTH,
                grid::CARD_GRID_GAP,
                grid::CARD_GRID_COLS,
            );
            if cols.get_untracked() != next {
                cols.set(next);
            }
        }
    };

    // Reactive on `notes_main_ref` becoming available — runs once the div
    // mounts, correcting the initial `grid::CARD_GRID_COLS` guess before the
    // first paint the user actually sees.
    Effect::new(move |_| {
        let _ = notes_main_ref.get();
        remeasure_cols();
    });

    #[cfg(target_arch = "wasm32")]
    {
        // `window_event_listener` attaches to `window` itself, entirely
        // outside this component's own DOM subtree — routing away from
        // `/notes` disposes `notes_main_ref`/`cols` (and every other signal
        // above) but does NOT detach this listener on its own (see its own
        // doc example): left unregistered, it keeps firing on every future
        // resize, anywhere in the app, forever — and each one calls
        // `remeasure_cols`, which touches the now-disposed `notes_main_ref`
        // and panics (a WASM panic here traps the whole module, breaking
        // the app until reload). `on_cleanup` ties its removal to this
        // component's own disposal, same as leptos_dom's own example.
        let handle = window_event_listener(leptos::ev::resize, move |_| remeasure_cols());
        on_cleanup(move || handle.remove());
    }

    // "Auto-layout" as a preview-only mode, not a one-shot repack: turning
    // it on never writes anything -- it's a client-only repack computed
    // below (the notes_display effect), same mechanism the search box
    // already uses. Turning it off (without hitting "Keep this layout")
    // just stops rendering the preview; nothing was ever persisted, so
    // there's nothing to restore. `on_auto_layout` below owns the toggle
    // itself; "Keep this layout" is the only path that writes, by
    // persisting the preview's own positions via `move_notes` (backlog
    // #102 follow-up). Mirrors `contacts::ContactsView`'s identical
    // rework.
    let auto_layout_enabled = RwSignal::new(false);

    // Guards the effect below against its own `auto_layout_enabled.set
    // (false)` write during "Keep this layout" -- mirrors
    // `contacts::ContactsView`'s identical `committing_layout` (see its
    // doc comment for the exact hazard).
    let committing_layout = RwSignal::new(false);

    // Auto-layout follows the search box's empty ⇄ non-empty transition —
    // mirrors `contacts::ContactsView`'s identical effect (see its doc
    // comment for why: preserving a filtered-out card's old position once
    // the filter clears isn't useful). Edge-triggered on `was_searching`
    // (not a continuous binding to `search`) so a manual toggle mid-search
    // isn't immediately overridden by the next keystroke.
    let was_searching = RwSignal::new(false);
    Effect::new(move |_| {
        let is_searching = !search.get().trim().is_empty();
        if is_searching != was_searching.get_untracked() {
            was_searching.set(is_searching);
            auto_layout_enabled.set(is_searching);
        }
    });

    // Plain signals fed by an Effect off the resources below (leptos-suspense
    // skill's "Effect + RwSignal workaround") — drag preview needs synchronous
    // local mutation on every pointermove, which a Suspend-driven render
    // can't provide.
    let coll_map: RwSignal<HashMap<i64, NoteCollectionInfo>> = RwSignal::new(HashMap::new());
    let pinned_display: RwSignal<Vec<NoteInfo>> = RwSignal::new(Vec::new());
    let other_display: RwSignal<Vec<NoteInfo>> = RwSignal::new(Vec::new());

    // Manual mode's canonical, resize-independent positions, one map per
    // zone (backlog #106 fifth follow-up) -- real, `CARD_GRID_COLS`-wide
    // space, exactly Auto mode's own "real" space (the same numbers
    // `notes_res` itself stores). Written *only* by an actual edit -- a
    // drag, a hidden-note relocation, a height correction, or "Keep this
    // layout" -- and even then, only the specific ids that edit touched:
    // never a whole-zone re-snapshot. Declared here, ahead of the rebuild
    // effect below, specifically so that effect's own Auto-layout repack
    // can read each note's *corrected* span from here (see its own doc
    // comment) rather than from `notes_res`'s raw, uncorrected copy.
    let pinned_abstract: RwSignal<HashMap<i64, (i32, i32, i32)>> = RwSignal::new(HashMap::new());
    let other_abstract: RwSignal<HashMap<i64, (i32, i32, i32)>> = RwSignal::new(HashMap::new());

    Effect::new(move |_| {
        let Some(collections) = collections_res.get() else {
            return;
        };
        let Some(mut notes) = notes_res.get() else {
            return;
        };
        // Read unconditionally, before either early-return guard below,
        // even though the branches further down that actually use
        // `auto`/`view_cols` may not run on every pass (found live,
        // 2026-09-28: a second "Auto-layout" click after a "Keep this
        // layout" commit silently no-op'd for notes, mirroring the
        // identical bug in `contacts::ContactsView` -- see its matching
        // effect's doc comment for the full mechanism). Leptos re-derives
        // an `Effect`'s reactive dependencies fresh on every run, from
        // whichever signals it actually called `.get()` on *that* run --
        // the `committing_layout` guard below returns early right after a
        // commit, before this effect used to reach `auto_layout_enabled`'s
        // `.get()` further down (previously read only inside the
        // `is_filtered`/`else if` branches), which silently dropped it
        // from this effect's subscription list; a later
        // `auto_layout_enabled.set(true)` (the next "Auto-layout" click)
        // then had nothing left to notify. `committing_layout` itself
        // already fully covers the staleness hazard this file's own
        // "must short-circuit before `auto_layout_enabled.get()`" comment
        // used to describe (a `notes_res`/`collections_res` refetch not
        // having landed yet when `auto_layout_enabled` flips) -- any run
        // that would have hit that hazard already returns early at the
        // `committing_layout` guard below, so tracking `auto_layout_enabled`
        // unconditionally here doesn't reopen it.
        let hidden = hidden_collections.get();
        let hidden_labels_set = hidden_labels.get();
        let only_pinned = pinned_only.get();
        let query = search.get().to_lowercase();
        let auto = auto_layout_enabled.get();
        let view_cols = cols.get();
        // Also read unconditionally here, for the same reason: used only
        // inside the `auto` branches below (see their own doc comment for
        // *why* they're read at all), but tracking them only there would
        // drop them from this effect's subscription list the moment a
        // `committing_layout`-guarded run returns early, same as the four
        // signals above.
        let pinned_abs = pinned_abstract.get();
        let other_abs = other_abstract.get();
        // A background refetch (e.g. another tab's edit) must not clobber
        // an in-flight drag — the drop handler bumps `refresh` itself once
        // it's done (or on failure, to resync).
        if controller.drag.get_untracked().is_some() {
            return;
        }
        // See `committing_layout`'s doc comment.
        if committing_layout.get_untracked() {
            return;
        }

        coll_map.set(collections.iter().cloned().map(|c| (c.id, c)).collect());

        notes.retain(|n| {
            !hidden.contains(&n.collection_id.to_string())
                && !n
                    .labels
                    .iter()
                    .any(|l| hidden_labels_set.contains(&l.id.to_string()))
                && (!only_pinned || n.pinned)
                && (query.is_empty()
                    || n.text.to_lowercase().contains(&query)
                    || n.title
                        .as_deref()
                        .is_some_and(|t| t.to_lowercase().contains(&query)))
        });

        let (mut pinned, mut other): (Vec<NoteInfo>, Vec<NoteInfo>) =
            notes.into_iter().partition(|n| n.pinned);

        // Auto-layout is forced on while searching (see `was_searching`
        // above) — repack each zone's filtered subset tightly, entirely
        // client-side and unpersisted (mirrors `contacts::ContactsView`'s
        // identical repack — see its doc comment for why). Reverts to the
        // stored positions automatically once the filter clears, since
        // this only ever runs while a filter is actually narrowing what's
        // visible. `hidden`/`only_pinned` count as filters here too (not
        // just `query`) — toggling a collection off or "pinned only" on
        // removes cards from the grid exactly like a search does, and
        // without repacking here the remaining cards keep their
        // full-set positions, leaving gaps where the filtered-out notes
        // used to sit.
        //
        // A note's span here should reflect its *corrected* height, not
        // `notes_res`'s raw, possibly-stale copy (found live, 2026-09-28:
        // notes deliberately never bumps `refresh` after a successful
        // height correction -- see the correction pass's own doc comment
        // for why -- so a repack built straight from `n.grid_row_span`
        // kept reserving room for the pre-correction estimate, most
        // visibly as extra empty space below the pinned row whenever a
        // pinned note's real height didn't match its stored span).
        // `pinned_abstract`/`other_abstract` (already read above) are the
        // one place that's always kept current (the correction pass
        // updates them unconditionally, with no such guard) — falling
        // back to the resource's own span only for a note that hasn't
        // reached the abstract map yet.
        let span_of = |abs: &HashMap<i64, (i32, i32, i32)>, n: &NoteInfo| -> i32 {
            abs.get(&n.id)
                .map(|&(_, _, s)| s)
                .unwrap_or(n.grid_row_span)
        };
        let is_filtered =
            !query.is_empty() || !hidden.is_empty() || !hidden_labels_set.is_empty() || only_pinned;
        if is_filtered && auto {
            for (zone, abs) in [(&mut pinned, &pinned_abs), (&mut other, &other_abs)] {
                let entries: Vec<(i64, i32)> =
                    zone.iter().map(|n| (n.id, span_of(abs, n))).collect();
                let packed = grid::pack_for_view(
                    &entries,
                    view_cols,
                    &vec![0; grid::CARD_GRID_COLS as usize],
                );
                for n in zone.iter_mut() {
                    if let Some(&(_, row, col, span)) = packed.iter().find(|&&(id, ..)| id == n.id)
                    {
                        n.grid_row = row;
                        n.grid_col = col;
                        n.grid_row_span = span;
                    }
                }
            }
        } else if auto {
            // The manual "Auto-layout" toggle, not searching: same
            // client-only, unpersisted repack as the search branch above,
            // over whatever's currently visible in each zone (respecting
            // the sidebar's hidden-collection filter) -- sorted by
            // manual_order first, falling back to *reading order* (not
            // alphabetical -- Notes has never sorted alphabetically,
            // unlike Contacts; see Plan B's own ledger for why that
            // matters) so the preview matches exactly what "Keep this
            // layout" now persists (it just saves this same computed
            // preview via `move_notes`, backlog #102 follow-up).
            for (zone, abs) in [(&mut pinned, &pinned_abs), (&mut other, &other_abs)] {
                zone.sort_by(|a, b| {
                    grid::manual_order_then(
                        a.manual_order,
                        b.manual_order,
                        (a.grid_row, a.grid_col).cmp(&(b.grid_row, b.grid_col)),
                    )
                });
                let entries: Vec<(i64, i32)> =
                    zone.iter().map(|n| (n.id, span_of(abs, n))).collect();
                let packed = grid::pack_for_view(
                    &entries,
                    view_cols,
                    &vec![0; grid::CARD_GRID_COLS as usize],
                );
                for n in zone.iter_mut() {
                    if let Some(&(_, row, col, span)) = packed.iter().find(|&&(id, ..)| id == n.id)
                    {
                        n.grid_row = row;
                        n.grid_col = col;
                        n.grid_row_span = span;
                    }
                }
            }
        }

        pinned_display.set(pinned);
        other_display.set(other);
    });

    // The on-screen cache below (`pinned_cluster_cache`/`other_cluster_cache`)
    // is a pure, deterministic function of (`pinned_abstract`/
    // `other_abstract` above, the current view width) via
    // `grid::compact_cell`/`column_offsets` -- Auto mode's own exact,
    // invertible real<->view mapping (`grid::expand_cell` is its inverse),
    // reused here instead of a lossy masonry repack. Because it's a true
    // bijection at a fixed view width, a resize or an Auto-layout on/off
    // round trip that returns to the same width reproduces the exact same
    // render every time, and a drag only ever needs to invert its own
    // cascade's final cells -- never touching any other note's stored
    // position, so nothing gets silently rewritten to a different width's
    // scale just because *something else* was dragged while narrow.

    // Every *accessible* note's `(id, pinned)` -- deliberately from
    // `notes_res` (every note this user can access, matching the render
    // block's own "every accessible note" input for the hidden-collection
    // isolation fix), not `pinned_display`/`other_display` (which are
    // visibility/search-filtered) -- and deliberately not each note's own
    // `(row, col, span)`, so a drag (which changes an existing id's
    // position but never this set) leaves this Memo's output unchanged.
    // `pinned` is part of the tracked value (not just `id`) so pinning or
    // unpinning a note -- which moves it between zones without changing
    // the overall id set -- still triggers a sync of both zones' maps.
    let accessible_ids = Memo::new(move |_| {
        let mut ids: Vec<(i64, bool)> = notes_res
            .get()
            .unwrap_or_default()
            .iter()
            .map(|n| (n.id, n.pinned))
            .collect();
        ids.sort_unstable();
        ids
    });
    // Keeps `pinned_abstract`/`other_abstract` in sync with *which* notes
    // exist, surgically: a newly-accessible note gets its raw stored
    // position inserted (a fresh note's own `grid_row`/`grid_col` is
    // already real-space, same as everything else `notes_res` returns),
    // a no-longer-accessible one gets removed, and every *already-placed*
    // note's existing entry is left completely untouched -- this never
    // repacks or re-derives anything from scratch, so adding one note
    // can't perturb any other note's stored position (backlog #106 fifth
    // follow-up).
    Effect::new(move |_| {
        let ids = accessible_ids.get();
        let raw_by_id: HashMap<i64, NoteInfo> = notes_res
            .get_untracked()
            .unwrap_or_default()
            .into_iter()
            .map(|n| (n.id, n))
            .collect();
        for (target, want_pinned) in [(pinned_abstract, true), (other_abstract, false)] {
            let want_ids: HashSet<i64> = ids
                .iter()
                .filter(|&&(_, p)| p == want_pinned)
                .map(|&(id, _)| id)
                .collect();
            target.update(|m| {
                m.retain(|id, _| want_ids.contains(id));
                for &id in &want_ids {
                    m.entry(id).or_insert_with(|| {
                        raw_by_id
                            .get(&id)
                            .map(|n| (n.grid_row, n.grid_col, n.grid_row_span))
                            .unwrap_or((0, 0, 1))
                    });
                }
            });
        }
    });

    // On-screen render positions for Manual mode, one per zone -- a pure
    // Memo over the abstract map above via `grid::compact_cell` (backlog
    // #106 fifth follow-up), empty in Auto mode (where `NoteCard`'s
    // `render_position: None` falls back to the unchanged
    // `column_offsets`/`compact_cell` path it already uses). Recomputed
    // fresh from scratch on every read -- there is no separate "patch this
    // in place" step anymore, because `compact_cell` is a true, exact
    // function of (real position, view width): the same inputs always
    // give the same output, so there's nothing for a stale patch to drift
    // out of sync with.
    let pinned_cluster_cache: Memo<HashMap<i64, (i32, i32, i32)>> = Memo::new(move |_| {
        if auto_layout_enabled.get() {
            return HashMap::new();
        }
        let cols_now = cols.get();
        let abstract_now = pinned_abstract.get();
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
    let other_cluster_cache: Memo<HashMap<i64, (i32, i32, i32)>> = Memo::new(move |_| {
        if auto_layout_enabled.get() {
            return HashMap::new();
        }
        let cols_now = cols.get();
        let abstract_now = other_abstract.get();
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

    // Scroll to and pulse-highlight a note deep-linked via `?highlight=`
    // (backlog #63), once it actually shows up in the rendered grid. When a
    // token was addressed too (backlog #134), the exact inline chip is scrolled
    // to and flashed instead; the whole card is the fallback. One-shot: clearing
    // both signals after firing means a later, unrelated grid re-render (e.g.
    // another tab's edit) can't re-trigger the scroll/pulse.
    Effect::new(move |_| {
        let Some(id) = highlight_id.get() else {
            return;
        };
        let token = highlight_token.get();
        let Some(note) = pinned_display
            .get()
            .into_iter()
            .chain(other_display.get())
            .find(|n| n.id == id)
        else {
            return;
        };
        let color = coll_map
            .get_untracked()
            .get(&note.collection_id)
            .map(|c| c.color.clone())
            .unwrap_or_else(|| "#6b3fc5".to_string());

        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::JsCast;
            if let Some(doc) = leptos::web_sys::window().and_then(|w| w.document()) {
                // Prefer the exact inline-token chip this jump addressed; fall
                // back to the whole card when the token no longer exists (it was
                // edited away) or the location was never known.
                let chip = token.and_then(|(line, ordinal)| {
                    doc.query_selector(&format!("[data-note-token='note:{id}:{line}:{ordinal}']"))
                        .ok()
                        .flatten()
                });
                if let Some(chip_el) = chip {
                    let el: leptos::web_sys::HtmlElement = chip_el.unchecked_into();
                    let opts = leptos::web_sys::ScrollIntoViewOptions::new();
                    opts.set_block(leptos::web_sys::ScrollLogicalPosition::Center);
                    opts.set_behavior(leptos::web_sys::ScrollBehavior::Smooth);
                    el.scroll_into_view_with_scroll_into_view_options(&opts);
                    // The chip's own `--chip-color` (task urgency colour) drives
                    // the pulse via `.note-token--highlight`; awaits/done fall
                    // back to the note-highlight purple.
                    let _ = el.class_list().add_1("note-token--highlight");
                    // Self-terminating animation: strip the class on `animationend`
                    // so a later grid re-render can't replay the pulse.
                    let cleanup_el = el.clone();
                    let end_closure = leptos::wasm_bindgen::prelude::Closure::once(move || {
                        let _ = cleanup_el.class_list().remove_1("note-token--highlight");
                    });
                    let _ = el.add_event_listener_with_callback(
                        "animationend",
                        end_closure.as_ref().unchecked_ref(),
                    );
                    end_closure.forget();
                } else if let Some(card_el) = doc
                    .query_selector(&format!("[data-note-id='{id}']"))
                    .ok()
                    .flatten()
                {
                    let el: leptos::web_sys::HtmlElement = card_el.unchecked_into();
                    el.scroll_into_view_with_bool(true);
                    let _ = el.style().set_property("--note-highlight-color", &color);
                    let _ = el.class_list().add_1("note-card--highlight");
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (color, token);

        highlight_id.set(None);
        highlight_token.set(None);
    });

    // Height self-correction (backlog #48): after every render, measure
    // each visible note's real height and compare it to its stored
    // `grid_row_span`. Mismatches — a brand-new note (placeholder span 1),
    // an edited note whose text length changed its height, or a note
    // freshly migrated from the pre-#48 model (parked at row 0 with
    // span 1) — get corrected and any resulting overlap cascaded away via
    // `correct_spans`. Self-terminating: once spans are correct, a
    // remeasure finds no mismatch and writes nothing, so this converges
    // after at most one extra pass rather than looping.
    //
    // `pending_correction_raf`: mirrors `contacts::ContactsView`'s
    // identical guard -- see its doc comment for the exact hazard (several
    // rapid actions each schedule their own `requestAnimationFrame`, and
    // without cancellation *all* of them eventually fire, not just the
    // latest).
    #[allow(unused_variables)] // only read on the wasm32 half of this effect
    let pending_correction_raf: std::rc::Rc<std::cell::Cell<Option<i32>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    Effect::new(move |_| {
        // Tracked only to know *that* the card list changed enough to
        // warrant a fresh measurement pass -- `measure_and_correct` below
        // re-reads the actual baseline itself at RAF-execution time (see
        // its own doc comment), never this snapshot.
        let _pinned_changed = pinned_display.get();
        let _other_changed = other_display.get();
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
                let Ok(list) = document.query_selector_all(".note-card") else {
                    return;
                };
                let mut measured: HashMap<i64, i32> = HashMap::new();
                for i in 0..list.length() {
                    let Some(node) = list.item(i) else { continue };
                    let el: web_sys::Element = node.unchecked_into();
                    let Some(id_str) = el.get_attribute("data-note-id") else {
                        continue;
                    };
                    let Ok(id) = id_str.parse::<i64>() else {
                        continue;
                    };
                    let height = el.get_bounding_client_rect().height();
                    // Row-gap is 0 in CSS (see .notes-grid) because a nonzero
                    // row-gap would slice into a *single* note's own span —
                    // grid gutters fall between every 8px auto-row, including
                    // the ones inside one tall card. So the vertical gap
                    // between stacked notes is baked into each note's own
                    // reserved span instead, matching grid::CARD_GRID_GAP (the
                    // horizontal cell gap) so vertical and horizontal spacing
                    // read the same.
                    let span = ((height + grid::CARD_GRID_GAP) / grid::CARD_GRID_ROW_UNIT_PX)
                        .ceil()
                        .max(1.0) as i32;
                    measured.insert(id, span);
                }

                // Re-read both zones fresh here (`get_untracked`), rather
                // than reusing the effect's own outer snapshot -- a
                // headless/backgrounded tab can throttle
                // `requestAnimationFrame` by seconds, and this closure
                // isn't cancelled if a *later* effect run schedules a
                // newer one (backlog #102 follow-up, mirrors
                // `contacts::ContactsView`'s identical fix -- see its doc
                // comment for the concrete failure this caused). The live
                // DOM measurement above was already always read fresh at
                // RAF time; only the *baseline* it diffs against was stale.
                for pinned in [true, false] {
                    // Correct the abstract (real-space) positions directly
                    // (backlog #106 fifth follow-up) -- a measured height
                    // is a property of the card's rendered content, not of
                    // which space it's placed in, and the abstract is now
                    // the single source of truth Manual mode's on-screen
                    // cache is purely derived from, so there's no separate
                    // render-space correction to keep in sync anymore.
                    let abstract_target = if pinned {
                        pinned_abstract
                    } else {
                        other_abstract
                    };
                    let abstract_now = abstract_target.get_untracked();
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
                    if zone_measured.is_empty() {
                        continue;
                    }
                    // Only row/span, preserving each id's existing column
                    // (`correct_spans` never changes it) -- NOT a full
                    // re-cluster (backlog #106, reverted after live
                    // testing, before the fifth follow-up replaced
                    // clustering with `compact_cell`/`expand_cell`
                    // entirely): a height correction changing just one
                    // member's row (to fix a real content-height mismatch,
                    // nothing the user asked to rearrange) must never
                    // reorder or reshuffle anything else.
                    let abstract_moves = grid::correct_spans(&abstract_entries, &zone_measured, 0);
                    if abstract_moves.is_empty() {
                        continue;
                    }
                    abstract_target.update(|m| {
                        for mv in &abstract_moves {
                            if let Some(entry) = m.get_mut(&mv.id) {
                                entry.0 = mv.grid_row;
                                entry.2 = mv.grid_row_span;
                            }
                        }
                    });
                    // `own_group`'s own `grid_row` mirrors the correction
                    // too, but *only* outside the Auto-layout preview: while
                    // previewing, `own_group` (`pinned_display`/
                    // `other_display`) deliberately holds `pack_for_view`'s
                    // tight-packed *real*-space row/col assignment -- every
                    // note's row there is a function of *every* note's span
                    // (each column is a running cumulative offset), so
                    // patching just one note's row in place without redoing
                    // the whole pack leaves it inconsistent with the
                    // *other* notes' rows, which were computed on the
                    // assumption of its old span. Real (non-preview) mode
                    // reads this field directly as a real-space position,
                    // and Manual mode's `manual_pickup_items` falls back to
                    // it for a note the cache hasn't placed yet, so it
                    // needs the corrected, real-space value then.
                    //
                    // `grid_row_span`, unlike `grid_row`, is always mirrored
                    // here, even mid-preview: it never feeds `on_keep_layout`
                    // (which reads each note's real `grid_row`/`grid_col`
                    // directly, not through this render pass's own
                    // `column_offsets` call), so patching it can't corrupt
                    // what a commit persists -- only the render closure's
                    // per-frame `pinned_offsets`/`other_offsets` computation
                    // (used solely to place cards on screen during preview,
                    // recomputed fresh from *all* of `own_group`'s current
                    // entries on every render, so it's always internally
                    // consistent with whatever span each entry currently
                    // carries). Left stale before this fix (found live,
                    // 2026-09-28): the pinned row's own CSS grid reserved
                    // height for a stale, pre-correction span, visibly
                    // pushing the notes grid below it down by the
                    // difference -- exactly the "Keep this layout doesn't
                    // keep the same layout with pinned notes" report, since
                    // the preview and the post-commit render disagreed on
                    // the pinned row's height. (A truly identical-looking
                    // bug -- two Auto-layout-preview notes rendering exactly
                    // on top of each other -- was previously traced to
                    // notes' `refresh` bump after a successful correction
                    // persist, not to patching span here; that bump stays
                    // off, see the doc comment below.)
                    let own_group = if pinned {
                        pinned_display
                    } else {
                        other_display
                    };
                    own_group.update(|notes| {
                        for n in notes.iter_mut() {
                            if let Some(m) = abstract_moves.iter().find(|m| m.id == n.id) {
                                n.grid_row_span = m.grid_row_span;
                                if !auto_layout_enabled.get_untracked() {
                                    n.grid_row = m.grid_row;
                                }
                            }
                        }
                    });
                    // Never persist while any filter (search, a hidden
                    // collection, "pinned only") is narrowing what's
                    // rendered -- `zone_notes`'s own visibility already
                    // reflects this, and while auto-layout is
                    // search-driven, `zone_notes` holds ephemeral
                    // tight-packed positions computed just for that
                    // subset. `abstract_moves` itself is always derived
                    // from the *whole* zone's abstract map regardless of
                    // filtering, but skip persisting anyway to match the
                    // existing filtered guard's intent: a filtered view is
                    // a bad moment to trust a height measurement, since
                    // filtered-out cards aren't even in the DOM to
                    // measure.
                    let unfiltered = search.get_untracked().trim().is_empty()
                        && hidden_collections.get_untracked().is_empty()
                        && hidden_labels.get_untracked().is_empty()
                        && !pinned_only.get_untracked();
                    if unfiltered {
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
                            // Deliberately *not* mirroring
                            // `contacts::ContactsView`'s identical pass here
                            // (which bumps `refresh` on a successful persist
                            // too, not just a failed one -- see its doc
                            // comment): doing the same for notes reproduces,
                            // deterministically, the exact "two cards render
                            // on top of each other" symptom this whole
                            // backlog item is about, but *after* "Keep this
                            // layout" -- `on_keep_layout`'s own pack
                            // computation, once it sees a refetch land with
                            // a *non-uniform* mix of just-corrected and
                            // still-original spans, produces an overlapping
                            // commit that then persists and survives a
                            // reload byte-for-byte (found live: not a race —
                            // `after_reload` matched `kept` exactly, so the
                            // bad layout was already the one committed, not
                            // something a later refetch corrupted). Not
                            // chased to its own root cause here since
                            // nothing in this backlog item's test suite
                            // needs notes to refetch on a successful
                            // correction -- `pinned_display`/`other_display`
                            // already pick up the corrected span locally,
                            // unconditionally, from the block above (span
                            // alone is safe to mirror even mid-preview; see
                            // that block's own doc comment for why).
                            if move_notes(persist).await.is_err() {
                                refresh.update(|n| *n += 1);
                            }
                        });
                    }
                }
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
            let _ = (_pinned_changed, _other_changed);
        }
    });

    let on_notes_pointer_move = move |ev: leptos::web_sys::PointerEvent| {
        controller.pointer_move(
            &ev,
            NOTE_CARD_WIDTH,
            &[(ZONE_PINNED, pinned_row_ref), (ZONE_OTHER, other_grid_ref)],
            |_zone| 0,
        );
    };

    let on_notes_pointer_up = move |_: leptos::web_sys::PointerEvent| {
        match controller.pointer_up() {
            Some(DragOutcome::Click { item_id }) => {
                let note = pinned_display
                    .get_untracked()
                    .into_iter()
                    .chain(other_display.get_untracked())
                    .find(|n| n.id == item_id);
                if let Some(n) = note {
                    editing_note.set(Some(Some(n)));
                }
            }
            Some(DragOutcome::Moved {
                zone,
                item_id,
                positions,
            }) => {
                let own_group = if zone == ZONE_PINNED {
                    pinned_display
                } else {
                    other_display
                };
                if auto_layout_enabled.get_untracked() {
                    // Auto mode: mirrors contacts::ContactsView's identical
                    // branch -- see its comment for the reasoning. Scoped
                    // to this drag's own zone, matching reorder_note's own
                    // pinned-zone scoping (Plan B).
                    //
                    // The cascade's own (row, col) is trustworthy insertion
                    // order but the full zone list's raw grid position is
                    // not -- used only to derive `predecessor_id`/
                    // `successor_id` below, never applied directly to
                    // `own_group` (backlog #102 follow-up, found live: a
                    // card left behind in the *source* column -- one this
                    // drop's own column-scoped cascade never touches --
                    // stayed at its stale pre-drop row forever, since
                    // nothing ever pulled it up to close the gap, and a
                    // successful `reorder_note` never bumps `refresh` to
                    // refetch a corrected order either).
                    let cascade_items: Vec<(i64, i32, i32)> = positions
                        .iter()
                        .map(|(&id, &(row, col))| (id, row, col))
                        .collect();
                    let current_order: Vec<i64> =
                        own_group.get_untracked().iter().map(|n| n.id).collect();
                    let (predecessor_id, successor_id) =
                        grid::auto_mode_reorder_neighbors(&cascade_items, &current_order, item_id);

                    // Always packed, by construction: reorder is expressed
                    // as an explicit new reading-order list (moving
                    // `item_id` next to whichever neighbor
                    // `auto_mode_reorder_neighbors` found), then the whole
                    // zone is fully repacked via `pack_for_view` from that
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

                    let span_by_id: HashMap<i64, i32> = own_group
                        .get_untracked()
                        .iter()
                        .map(|n| (n.id, n.grid_row_span))
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
                    own_group.update(|notes| {
                        for n in notes.iter_mut() {
                            if let Some(&(row, col)) = packed_by_id.get(&n.id) {
                                n.grid_row = row;
                                n.grid_col = col;
                            }
                        }
                    });

                    leptos::task::spawn_local(async move {
                        if reorder_note(item_id, predecessor_id, successor_id)
                            .await
                            .is_err()
                        {
                            refresh.update(|n| *n += 1);
                        }
                    });
                } else {
                    // Manual mode: apply optimistically to the local
                    // display list right away -- the server call below is
                    // fire-and-forget except on failure, when it resyncs
                    // from scratch via `refresh`.
                    let span_by_id: HashMap<i64, i32> = own_group
                        .get_untracked()
                        .iter()
                        .map(|n| (n.id, n.grid_row_span))
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
                    // written below; every other note's real position is
                    // left completely untouched, so dragging one card
                    // while narrow can never silently rewrite anyone
                    // else's stored position to a different width's scale.
                    let col_weight = ((NOTE_CARD_WIDTH + grid::CARD_GRID_GAP)
                        / grid::CARD_GRID_ROW_UNIT_PX)
                        .round() as i32;
                    let cols_now = cols.get_untracked();
                    let want_pinned = zone == ZONE_PINNED;
                    let abstract_target = if zone == ZONE_PINNED {
                        pinned_abstract
                    } else {
                        other_abstract
                    };
                    let abstract_before = abstract_target.get_untracked();
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

                    // Mirrors contacts::ContactsView's identical fix
                    // (backlog #102 follow-up, "dragging onto a hidden
                    // card's slot") -- a drop can land exactly on a
                    // hidden-collection member's stored cell, which
                    // nothing above could have displaced since it was
                    // never rendered. Checked against the full real-space
                    // occupancy (existing entries plus this drop's own new
                    // ones), matching a hidden note's own stored space
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
                        for n in notes_res.get_untracked().unwrap_or_default() {
                            if n.pinned != want_pinned
                                || !hidden.contains(&n.collection_id.to_string())
                                || touched.contains_key(&n.id)
                            {
                                continue;
                            }
                            if let Some((row, col)) = grid::relocate_if_colliding(
                                &claimed,
                                n.id,
                                n.grid_row,
                                n.grid_col,
                                n.grid_row_span,
                                grid::CARD_GRID_COLS,
                                0,
                                col_weight,
                            ) {
                                touched.insert(n.id, (row, col, n.grid_row_span));
                            }
                        }
                    }

                    abstract_target.update(|m| {
                        for (&id, &pos) in &touched {
                            m.insert(id, pos);
                        }
                    });
                    // `own_group`'s own `grid_row`/`grid_col` mirror the
                    // same real-space values just written above -- Auto
                    // mode reads these fields directly as real positions,
                    // and `manual_pickup_items` falls back to them for a
                    // note the cache hasn't placed yet, so both need the
                    // real, not display-space, coordinates.
                    own_group.update(|notes| {
                        for n in notes.iter_mut() {
                            if let Some(&(row, col, _)) = touched.get(&n.id) {
                                n.grid_row = row;
                                n.grid_col = col;
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
                        if move_notes(persist_moves).await.is_err() {
                            refresh.update(|n| *n += 1);
                        }
                    });
                }
            }
            Some(DragOutcome::Cancelled) | None => {}
        }
    };

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
        // `pinned_cluster_cache`/`other_cluster_cache` fresh from
        // `pinned_abstract`/`other_abstract` on its very next run -- no
        // separate trigger/ordering bookkeeping needed, since the
        // abstract maps were never touched by entering or leaving the
        // preview in the first place.
        auto_layout_enabled.set(!auto_layout_enabled.get_untracked());
    };

    // Commits the current preview by persisting `pinned_display`/
    // `other_display` themselves via `move_notes` — mirrors
    // `contacts::ContactsView::on_keep_layout` exactly (see its comment for
    // the full reasoning: this used to always repack every accessible note
    // via the server, which diverged from the preview whenever a
    // collection was hidden).
    let on_keep_layout = move |_: leptos::ev::MouseEvent| {
        let col_weight =
            ((NOTE_CARD_WIDTH + grid::CARD_GRID_GAP) / grid::CARD_GRID_ROW_UNIT_PX).round() as i32;
        let pinned_now = pinned_display.get_untracked();
        let other_now = other_display.get_untracked();

        // `pinned_now`/`other_now`'s own `grid_row`/`grid_col` are
        // *already* real (storage-space) positions, not view-space ones
        // needing translation: `pack_for_view` -- the function that
        // populated them while the Auto-layout preview was showing --
        // returns real columns by construction (see its own doc comment's
        // "level 2" step, which spreads each visual column's members back
        // across that visual column's *own* real columns). What's
        // actually shown on screen during the preview is computed
        // separately, by `card_style` applying `compact_cell` to these
        // same real values using the current `cols`/`col_offsets` (see
        // `NoteCard`'s `render_position: None` branch) -- the preview
        // never stores a view-space coordinate anywhere to begin with.
        //
        // Previously this ran every entry through `expand_cell` with
        // all-zero offsets, on the mistaken premise that these were
        // view-space positions being "expanded" back to real space (found
        // live, 2026-09-28, mirroring the identical bug in
        // `contacts::ContactsView::on_keep_layout`: "Keep this layout"
        // silently collapsed most notes onto the same handful of real
        // cells -- `expand_cell`'s real-column choice depends only on
        // which *view* column its input falls into, not on the real
        // column already recorded, so feeding it an already-real column
        // as if it were a view column remapped every note sharing that
        // (accidental) view bucket onto one single real column, all at
        // real row 0). The fix is to persist these values directly -- no
        // translation needed, since there was never a view-space
        // coordinate to translate from.
        let to_real = |zone_now: &[NoteInfo]| -> HashMap<i64, (i32, i32, i32)> {
            zone_now
                .iter()
                .map(|n| (n.id, (n.grid_row, n.grid_col, n.grid_row_span)))
                .collect()
        };
        let mut pinned_real = to_real(&pinned_now);
        let mut other_real = to_real(&other_now);

        // Row/col numbering is zone-local (pinned and other render as two
        // separate grid containers), so a hidden note's collision check
        // must stay scoped to its own zone's claimed positions — never
        // mixed across zones, unlike a drag's own single-zone drop.
        // Checked against the freshly-translated real positions above,
        // matching a hidden note's own stored space directly.
        let hidden = hidden_collections.get_untracked();
        if !hidden.is_empty() {
            for (target, want_pinned) in [(&mut pinned_real, true), (&mut other_real, false)] {
                let claimed: Vec<(i64, i32, i32, i32)> = target
                    .iter()
                    .map(|(&id, &(r, c, s))| (id, r, c, s))
                    .collect();
                for n in notes_res.get_untracked().unwrap_or_default() {
                    if n.pinned != want_pinned || !hidden.contains(&n.collection_id.to_string()) {
                        continue;
                    }
                    if let Some((row, col)) = grid::relocate_if_colliding(
                        &claimed,
                        n.id,
                        n.grid_row,
                        n.grid_col,
                        n.grid_row_span,
                        grid::CARD_GRID_COLS,
                        0,
                        col_weight,
                    ) {
                        target.insert(n.id, (row, col, n.grid_row_span));
                    }
                }
            }
        }

        let to_moves = |real: &HashMap<i64, (i32, i32, i32)>| -> Vec<GridPosition> {
            real.iter()
                .map(|(&id, &(grid_row, grid_col, grid_row_span))| GridPosition {
                    id,
                    grid_row,
                    grid_col,
                    grid_row_span,
                })
                .collect()
        };
        let pinned_moves = to_moves(&pinned_real);
        let other_moves = to_moves(&other_real);

        committing_layout.set(true);
        leptos::task::spawn_local(async move {
            // Two separate persists, not one combined batch (found live,
            // 2026-09-28): `move_notes` rejects a batch mixing pinned and
            // unpinned notes ("Cannot move notes across the pinned
            // boundary in one batch") -- with at least one note in each
            // zone, a single combined call always failed outright,
            // silently discarding the whole commit (both zones, not just
            // the mismatched one) and reverting the display to the
            // pre-auto-layout manual positions the moment
            // `auto_layout_enabled.set(false)` below flipped back to
            // manual mode.
            let pinned_ok =
                pinned_moves.is_empty() || move_notes(pinned_moves.clone()).await.is_ok();
            let other_ok = other_moves.is_empty() || move_notes(other_moves.clone()).await.is_ok();
            // Mirrors `contacts::ContactsView::on_keep_layout`'s optimistic
            // apply -- see `committing_layout`'s doc comment for why this
            // must never lean on `notes_res`'s own (still-stale) refetch.
            // Applied per zone, independently, so a failure in one zone
            // doesn't also discard the other zone's successful commit.
            if pinned_ok {
                pinned_display.update(|notes| {
                    for n in notes.iter_mut() {
                        if let Some(gp) = pinned_moves.iter().find(|m| m.id == n.id) {
                            n.grid_row = gp.grid_row;
                            n.grid_col = gp.grid_col;
                        }
                    }
                });
                // The abstract map must reflect this commit immediately
                // (backlog #106 fifth follow-up) -- this is what a later
                // resize, or this same page reloaded fresh, re-derives the
                // display from via `compact_cell`, so it has to be the
                // real-space encoding, not the view-space preview. The
                // render cache is a pure Memo over this map, so it
                // reflects this the moment the very next read happens --
                // no separate cache write needed.
                pinned_abstract.set(pinned_real);
            }
            if other_ok {
                other_display.update(|notes| {
                    for n in notes.iter_mut() {
                        if let Some(gp) = other_moves.iter().find(|m| m.id == n.id) {
                            n.grid_row = gp.grid_row;
                            n.grid_col = gp.grid_col;
                        }
                    }
                });
                other_abstract.set(other_real);
            }
            let ok = pinned_ok && other_ok;
            auto_layout_enabled.set(false);
            // Deferred to the next animation frame, not cleared here
            // immediately (backlog #106, found live: "Keep this layout"
            // committed the right positions, but the *rendered* DOM kept
            // showing stale, pre-persist ones). `auto_layout_enabled.set(false)`
            // is one of the `pinned_display`/`other_display`-building
            // Effect's own reactive deps, and that Effect's own
            // `committing_layout.get_untracked()` guard exists precisely
            // to skip the stale `notes_res` rerun this triggers -- but
            // Leptos doesn't run a dependent Effect synchronously inline
            // with the `.set()` call that dirtied it; the guard's
            // `.set(false)` landing in this same synchronous block, right
            // after, meant that Effect always saw the guard already
            // cleared by the time it actually ran, overwriting this
            // commit's optimistic positions with `notes_res`'s still-
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

    // "Clear all" (backlog #90) — a modal with a collection picker, rather
    // than the plain confirm Contacts uses, since a note's scope (its own
    // collection vs. every collection you can see) actually needs to be
    // chosen, not just confirmed.
    let clearing_notes = RwSignal::new(false);

    view! {
        <div class="notes-view">
            <div class="notes-layout">
                <aside class="notes-sidebar fs-panel">
                    <button
                        class="fs-collapse-btn"
                        on:click=move |_| sidebar_open.update(|b| *b = !*b)
                    >
                        "Note collections"
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
                        // Collections + labels in one box, shared verbatim with
                        // the Focus sidebar (backlog #114 follow-up).
                        <NoteCollectionsPanel refresh=collections_refresh on_refresh=on_refresh />
                    </div>
                </aside>

                <div
                    class=move || {
                        let d = controller.drag.get();
                        // `user-select: none` (via `--dragging`) stops the browser's
                        // native text-selection drag from kicking in as the pointer
                        // sweeps across other cards' text while reordering.
                        match (d.is_some(), d.is_some_and(|d| d.no_drop)) {
                            (true, true) => "notes-main notes-main--dragging notes-main--no-drop",
                            (true, false) => "notes-main notes-main--dragging",
                            (false, _) => "notes-main",
                        }
                    }
                    node_ref=notes_main_ref
                    on:pointermove=on_notes_pointer_move
                    on:pointerup=on_notes_pointer_up
                    on:pointercancel=move |_: leptos::web_sys::PointerEvent| { controller.cancel(); }
                >
                    <div class="notes-toolbar">
                        <button class="notes-pin-filter" on:click=move |_| editing_note.set(Some(None))>
                            <PlusIcon />" New note"
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
                        ><GridIcon />" Auto-layout"</button>
                        {move || (auto_layout_enabled.get() && !is_searching.get()).then(|| view! {
                            <button
                                class="notes-pin-filter notes-pin-filter--active"
                                title="Repacks each zone's whole set (not just what's currently visible) so a hidden collection never ends up colliding with these new positions"
                                on:click=on_keep_layout
                            >
                                "Keep this layout"
                            </button>
                        })}
                        {move || (notes_res.get().map(|n| !n.is_empty()).unwrap_or(false)).then(|| view! {
                            <button class="notes-pin-filter" on:click=move |_| clearing_notes.set(true)>
                                <TrashIcon />" Clear all"
                            </button>
                        })}
                        <button
                            class=move || if pinned_only.get() { "notes-pin-filter notes-pin-filter--active" } else { "notes-pin-filter" }
                            on:click=move |_| pinned_only.update(|v| *v = !*v)
                        ><StarIcon />" Starred"</button>
                        <input
                            class="form-input notes-search"
                            type="text"
                            placeholder="Search notes…"
                            prop:value=move || search.get()
                            on:input=move |ev| search.set(event_target_value(&ev))
                        />
                    </div>

                    {move || {
                        if collections_res.get().is_none() || notes_res.get().is_none() {
                            return view! { <p class="dialog-hint">"Loading notes…"</p> }.into_any();
                        }

                        let pinned_notes = pinned_display.get();
                        let other_notes = other_display.get();
                        let map = coll_map.get();
                        let count = pinned_notes.len() + other_notes.len();

                        // Per-zone compaction offsets (backlog #50) —
                        // recomputed whenever this closure reruns (any note
                        // list change, or `cols` itself changing on
                        // resize), from *every* note currently in the zone
                        // (including one mid-drag — see column_offsets's
                        // doc comment).
                        let cols_now = cols.get();
                        let pinned_triples: Vec<(i32, i32, i32)> = pinned_notes
                            .iter()
                            .map(|n| (n.grid_row, n.grid_col, n.grid_row_span))
                            .collect();
                        let pinned_offsets = grid::column_offsets(&pinned_triples, cols_now, 0);
                        let other_triples: Vec<(i32, i32, i32)> = other_notes
                            .iter()
                            .map(|n| (n.grid_row, n.grid_col, n.grid_row_span))
                            .collect();
                        let other_offsets = grid::column_offsets(&other_triples, cols_now, 0);

                        // Manual mode's cluster-aware render positions
                        // (backlog #102) -- read straight from
                        // `pinned_cluster_cache`/`other_cluster_cache`
                        // rather than recomputed here, so an ordinary drag
                        // (this closure's own most common trigger, via
                        // `pinned_display`/`other_display`) doesn't force
                        // a full re-cluster of the whole zone and silently
                        // reshuffle every other gaggle on screen (backlog
                        // #104 follow-up -- see those signals' doc
                        // comment). Empty in Auto mode, where `NoteCard`'s
                        // `render_position: None` falls back to the
                        // unchanged `column_offsets`/`compact_cell` path
                        // above. Mirrors `contacts::ContactsView`'s
                        // identical wiring.
                        let pinned_render_positions: std::sync::Arc<HashMap<i64, (i32, i32, i32)>> =
                            std::sync::Arc::new(pinned_cluster_cache.get());
                        let other_render_positions: std::sync::Arc<HashMap<i64, (i32, i32, i32)>> =
                            std::sync::Arc::new(other_cluster_cache.get());

                        let lookup = move |cid: i64, map: &HashMap<i64, NoteCollectionInfo>| {
                            map.get(&cid).cloned().unwrap_or_else(|| NoteCollectionInfo {
                                id: cid,
                                name: "Unknown".to_string(),
                                color: "#8b96ad".to_string(),
                                shared: false,
                                is_owner: false,
                                members: vec![],
                                owner_username: String::new(),
                            })
                        };

                        view! {
                            <p class="notes-result-count">{count}" note"{if count == 1 { "" } else { "s" }}</p>

                            // Pinned notes get their own explicit-placement grid so they
                            // stay above everything else. Both grids use the same fixed
                            // `grid::CARD_GRID_COLS` columns and row-span placement (backlog
                            // #48), starting at row 0 in every column — the "+ Take a
                            // note…" tile that used to reserve column 0's top rows moved to
                            // a toolbar button (backlog #90).
                            <div
                                class=move || {
                                    let (no_drop, zone) = drag_zone_state.get();
                                    if no_drop && zone != Some(ZONE_PINNED) {
                                        "notes-pinned-row notes-group--blocked"
                                    } else {
                                        "notes-pinned-row"
                                    }
                                }
                                style:grid-template-columns=move || format!("repeat({}, {}px)", cols.get(), NOTE_CARD_WIDTH as i32)
                                node_ref=pinned_row_ref
                            >
                                {drop_shadow_view(controller, ZONE_PINNED)}
                                {pinned_notes.into_iter().map(|n| {
                                    let collection = lookup(n.collection_id, &map);
                                    let nid = n.id;
                                    let nspan = n.grid_row_span;
                                    let pinned_render_positions_for_pickup =
                                        std::sync::Arc::clone(&pinned_render_positions);
                                    let on_pick_up = Callback::new(move |ev: leptos::web_sys::PointerEvent| {
                                        let all_notes = pinned_display.get_untracked();
                                        let manual = !auto_layout_enabled.get_untracked();
                                        // Includes this drag's own note (see column_offsets's/
                                        // manual_pickup_items' doc comments) — its original slot
                                        // must keep counting toward its own column/cell for the
                                        // whole drag.
                                        let items: Vec<(i64, i32, i32, i32)> = if manual {
                                            manual_pickup_items(
                                                &all_notes,
                                                &pinned_render_positions_for_pickup,
                                            )
                                        } else {
                                            all_notes
                                                .iter()
                                                .map(|x| (x.id, x.grid_row, x.grid_col, x.grid_row_span))
                                                .collect()
                                        };
                                        controller.pick_up(
                                            &ev, nid, ZONE_PINNED, nspan, &items,
                                            pinned_row_ref, cols_now, 0,
                                            manual,
                                        );
                                    });
                                    let dragging = Signal::derive(move || dragging_id.get() == Some(nid));
                                    let render_position = pinned_render_positions.get(&nid).copied();
                                    view! {
                                        <NoteCard
                                            note=n
                                            collection=collection
                                            on_pick_up=on_pick_up
                                            dragging=dragging
                                            controller=controller
                                            refresh=refresh
                                            cols=cols_now
                                            col_offsets=pinned_offsets.clone()
                                            render_position=render_position
                                            contact_names=contact_names
                                        />
                                    }
                                }).collect_view()}
                            </div>

                            {if count == 0 {
                                view! { <p class="notes-empty">"No notes yet."</p> }.into_any()
                            } else if other_notes.is_empty() {
                                ().into_any()
                            } else {
                                view! {
                                    <div
                                        class=move || {
                                            let (no_drop, zone) = drag_zone_state.get();
                                            if no_drop && zone == Some(ZONE_PINNED) {
                                                "notes-grid notes-group--blocked"
                                            } else {
                                                "notes-grid"
                                            }
                                        }
                                        style:grid-template-columns=move || format!("repeat({}, {}px)", cols.get(), NOTE_CARD_WIDTH as i32)
                                        node_ref=other_grid_ref
                                    >
                                        {drop_shadow_view(controller, ZONE_OTHER)}
                                        {other_notes.into_iter().map(|n| {
                                            let collection = lookup(n.collection_id, &map);
                                            let nid = n.id;
                                            let nspan = n.grid_row_span;
                                            let other_render_positions_for_pickup =
                                                std::sync::Arc::clone(&other_render_positions);
                                            let on_pick_up = Callback::new(move |ev: leptos::web_sys::PointerEvent| {
                                                let all_notes = other_display.get_untracked();
                                                let manual = !auto_layout_enabled.get_untracked();
                                                let items: Vec<(i64, i32, i32, i32)> = if manual {
                                                    manual_pickup_items(
                                                        &all_notes,
                                                        &other_render_positions_for_pickup,
                                                    )
                                                } else {
                                                    all_notes
                                                        .iter()
                                                        .map(|x| (x.id, x.grid_row, x.grid_col, x.grid_row_span))
                                                        .collect()
                                                };
                                                controller.pick_up(
                                                    &ev, nid, ZONE_OTHER, nspan, &items,
                                                    other_grid_ref, cols_now, 0,
                                                    manual,
                                                );
                                            });
                                            let dragging = Signal::derive(move || dragging_id.get() == Some(nid));
                                            let render_position = other_render_positions.get(&nid).copied();
                                            view! {
                                                <NoteCard
                                                    note=n
                                                    collection=collection
                                                    on_pick_up=on_pick_up
                                                    dragging=dragging
                                                    controller=controller
                                                    refresh=refresh
                                                    cols=cols_now
                                                    col_offsets=other_offsets.clone()
                                                    render_position=render_position
                                                    contact_names=contact_names
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

        {move || editing_note.get().map(|n| {
            view! {
                <Transition fallback=|| ().into_any()>
                    {move || {
                        let n = n.clone();
                        Suspend::new(async move {
                            let collections = list_note_collections().await.unwrap_or_default();
                            view! {
                                <NoteEditorModal
                                    note=n.clone()
                                    collections=collections
                                    default_collection_id=None
                                    on_close=move || editing_note.set(None)
                                    on_refresh=on_refresh
                                />
                            }
                        })
                    }}
                </Transition>
            }
        })}

        <ClearNotesModal
            armed=clearing_notes
            collections=Signal::derive(move || coll_map.get().into_values().collect::<Vec<_>>())
            on_refresh=on_refresh
        />
    }
}

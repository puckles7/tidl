//! Shared masonry/responsive-grid math for card grids that persist an
//! explicit `(grid_row, grid_col, grid_row_span)` per item — used by the
//! Notes tab (`components/notes/mod.rs`, backlog #47/#48/#50) and the
//! Contacts tab (`components/contacts/mod.rs`, backlog #79), so the two
//! features' grids can never silently drift apart.
//!
//! Everything here is pure (no Leptos reactivity, no DOM, no sqlx) so it
//! compiles unchanged on both the `ssr` and `hydrate`/`wasm32` targets —
//! `api/notes.rs`/`api/contacts.rs` call the placement/packing functions
//! server-side, `components/notes/mod.rs`/`components/contacts/mod.rs` call
//! the compaction/drag functions client-side.

use std::collections::HashMap;

use crate::types::GridPosition;

/// Maximum number of grid columns a stored `(grid_row, grid_col)` may use —
/// shared between server-side placement/auto-layout and the client's CSS
/// grid + drag math, so they can never silently drift out of sync. Stored
/// positions always live in this many columns, regardless of window size —
/// but *rendering* below this many columns is a live, purely client-side
/// compaction (backlog #50, `view_col_of`/`column_offsets`/`compact_cell`
/// below), so a narrow viewport reflows down to fewer rendered columns
/// instead of scrolling horizontally.
pub const CARD_GRID_COLS: i32 = 8;

/// Row-unit height (px) for a card grid's CSS `grid-auto-rows` (backlog
/// #48) — each item's `grid_row`/`grid_row_span` are counts of this unit,
/// not one-item-per-row.
pub const CARD_GRID_ROW_UNIT_PX: f64 = 8.0;

/// Inter-cell gap (px) for a card grid — must match the `gap` used in the
/// corresponding CSS rule (`.notes-grid`, `.contacts-grid`, …).
pub const CARD_GRID_GAP: f64 = 16.0;

/// First row unit a real item may occupy in column 0 of a zone that
/// reserves part of that column for its own chrome — 0 for a zone with no
/// such reservation (every zone today: Notes' pinned-tile reservation was
/// removed in backlog #90, leaving this mechanism unused but kept for any
/// future zone that needs it).
pub fn reserved_floor(col: i32, reserved_col0_floor: i32) -> i32 {
    if col == 0 {
        reserved_col0_floor
    } else {
        0
    }
}

// ── Drag/drop pixel geometry ────────────────────────────────────────────────

/// Pixel geometry of one zone's grid container, snapshotted once at pickup:
/// its own top-left corner. Row-unit height and column width are fixed
/// constants, so no DOM measurement beyond the container's own corner is
/// needed at all.
#[derive(Clone, Debug, Default)]
pub struct GridGeometry {
    #[allow(dead_code)] // only read from the wasm32 half of pointer-move handlers
    pub container_left: f64,
    #[allow(dead_code)]
    pub container_top: f64,
}

/// Grab-point offset (px, pointer minus the card's own top-left) and the
/// card's on-screen size at pickup, read from `ev.current_target()`'s
/// `getBoundingClientRect()`. Captured once so the dragged card's
/// `position: fixed` overlay can hold a steady size and track the pointer
/// with the exact offset it was grabbed at for the whole drag.
///
/// Also sets pointer capture on the card, so `pointermove`/`pointerup` keep
/// bubbling from it even once the cursor leaves the card, the grid, or its
/// container — without this, a fast or far drag can carry the pointer
/// outside every element with a `pointermove` listener, silently freezing
/// the drag mid-air with no `pointerup` ever received to end it.
#[allow(unused_variables)]
pub fn pointer_down_geometry(
    ev: &leptos::web_sys::PointerEvent,
    cx: f64,
    cy: f64,
) -> (f64, f64, f64, f64) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(target) = ev.current_target() {
            return pointer_down_geometry_for(&target, ev.pointer_id(), cx, cy);
        }
    }
    (0.0, 0.0, 260.0, 96.0)
}

/// Same geometry `pointer_down_geometry` computes, but for a target
/// element captured ahead of time instead of read from a live event's
/// `current_target()` — needed by touch's long-press-to-drag confirm
/// path (mobile drag support): a `PointerEvent`'s `current_target()`
/// resets to `null` once the event finishes dispatching, so it can't be
/// read again ~200ms later from inside a deferred timer callback the
/// way `pointer_down_geometry` reads it synchronously. The pointer
/// itself is still physically down at that point (the user is still
/// holding), so `set_pointer_capture` is just as valid to call here as
/// it would have been immediately at `pointerdown`.
#[allow(unused_variables)]
pub fn pointer_down_geometry_for(
    target: &leptos::web_sys::EventTarget,
    pointer_id: i32,
    cx: f64,
    cy: f64,
) -> (f64, f64, f64, f64) {
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::wasm_bindgen::JsCast;
        let el: &web_sys::Element = target.unchecked_ref();
        let _ = el.set_pointer_capture(pointer_id);
        let r = el.get_bounding_client_rect();
        (cx - r.left(), cy - r.top(), r.width(), r.height())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (target, pointer_id, cx, cy);
        (0.0, 0.0, 260.0, 96.0)
    }
}

/// Snapshots `container_ref`'s top-left corner (see `GridGeometry`).
#[allow(unused_variables)]
pub fn snapshot_grid_geometry(
    container_ref: leptos::prelude::NodeRef<leptos::html::Div>,
) -> GridGeometry {
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::prelude::Get;
        use leptos::wasm_bindgen::JsCast;
        if let Some(container) = container_ref.get() {
            let container_el: &web_sys::Element = container.unchecked_ref();
            let r = container_el.get_bounding_client_rect();
            return GridGeometry {
                container_left: r.left(),
                container_top: r.top(),
            };
        }
    }
    GridGeometry::default()
}

/// Measures `container_ref`'s current rendered width (backlog #50) — used
/// to derive `effective_cols` without a separate breakpoint. `None` before
/// the element exists yet, on server/native builds (no DOM), or if
/// `container_ref`'s owning component has since been disposed (a caller
/// invoked from a `window`-level event listener can outlive the component
/// that registered it — see the `on_cleanup` callers in `notes::NotesView`/
/// `contacts::ContactsView`, which exist to prevent exactly that, but a
/// `try_get` here means a still-leaked call degrades to a no-op measurement
/// instead of a `.get()` panic that traps the whole WASM module).
#[allow(unused_variables)]
pub fn measure_container_width(
    container_ref: leptos::prelude::NodeRef<leptos::html::Div>,
) -> Option<f64> {
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::prelude::Get;
        use leptos::wasm_bindgen::JsCast;
        if let Some(el) = container_ref.try_get().flatten() {
            let el: &web_sys::Element = el.unchecked_ref();
            return Some(el.get_bounding_client_rect().width());
        }
    }
    None
}

/// Maps a position to a `(row, col)` grid cell. Both axes are pure
/// functions of fixed geometry (column width/gap, row-unit height) — no
/// dependency on existing cards, so always well-defined even over empty
/// space.
///
/// Callers must pass the dragged card's own top-left corner (`pointer -
/// grab_dx/dy`), not the raw pointer position: the row unit is much finer
/// than the column width, so resolving off the pointer instead of the
/// card's edge visibly misplaces the drop target whenever the card was
/// grabbed anywhere but its very top.
#[allow(clippy::too_many_arguments)] // params are distinct fixed geometry constants, not a bundle
pub fn cell_at(
    cx: f64,
    cy: f64,
    container_left: f64,
    container_top: f64,
    col_width: f64,
    gap: f64,
    row_unit: f64,
    cols: i32,
) -> (i32, i32) {
    let col = (((cx - container_left) / (col_width + gap)).floor() as i32).clamp(0, cols - 1);
    let row = (((cy - container_top) / row_unit).floor() as i32).max(0);
    (row, col)
}

// ── Column stacking / drop resolution ───────────────────────────────────────

/// Sorts `entries` (each `(id, row_start, span)`) by `row_start` and
/// cascades any overlaps: if an entry starts before its predecessor ends,
/// it's pushed down to start exactly where the predecessor ends. Used both
/// for drag displacement (`resolve_drop`) and the height self-correction
/// pass (`correct_spans`) — the same "push later items down just enough to
/// clear the one before them" rule applies whether the overlap was caused
/// by a drop or by an item's measured span changing.
pub fn stack_column(mut entries: Vec<(i64, i32, i32)>) -> Vec<(i64, i32, i32)> {
    entries.sort_by_key(|&(_, row_start, _)| row_start);
    for i in 1..entries.len() {
        let (_, prev_row, prev_span) = entries[i - 1];
        let prev_end = prev_row + prev_span;
        let (id, row, span) = entries[i];
        if row < prev_end {
            entries[i] = (id, prev_end, span);
        }
    }
    entries
}

/// Computes the minimum set of position changes to place `moving_id` (with
/// its own `moving_span`, unchanged by the move) at `target_row` in its
/// column, given every *other* item already in that column as `(id,
/// row_start, span)`. Displacement is column-scoped only: dropping onto an
/// occupied interval pushes every overlapping item further down the same
/// column, cascading via `stack_column` — no sideways displacement into a
/// neighboring column. `reserved_end` is the first row unit real items may
/// occupy here (`reserved_floor`); `target_row` is clamped up to it.
///
/// `moving_id` is placed *before* `column_items` in the list handed to
/// `stack_column`: its sort is stable, so when the dropped item and an
/// incumbent tie on `row_start` (the common case — dropping directly onto
/// an occupied row), the just-dropped item wins that row and the incumbent
/// is the one pushed down, matching what a drop visually means.
///
/// Deliberately kept alongside `place_with_relocation` (backlog #102
/// follow-up) rather than replaced by it: Auto mode's drag must only ever
/// *reorder* — the zone stays tightly packed by `pack_for_view` immediately
/// after, and `auto_mode_reorder_neighbors` reads this cascade's row order
/// as the new reading-order rank — so it must stay column-scoped and
/// gapless-by-construction. `place_with_relocation`'s grid-wide relocation
/// (which can legitimately move a card to a different column) is only
/// correct for Manual mode, where positions persist as-is with no
/// immediately-following repack. See `drag_grid.rs`'s `pointer_move` for
/// the mode branch that picks between the two.
pub fn resolve_drop(
    column_items: &[(i64, i32, i32)],
    moving_id: i64,
    moving_span: i32,
    target_row: i32,
    reserved_end: i32,
) -> Vec<(i64, i32, i32)> {
    let target_row = target_row.max(reserved_end);
    let mut entries: Vec<(i64, i32, i32)> = vec![(moving_id, target_row, moving_span)];
    entries.extend_from_slice(column_items);
    stack_column(entries)
}

/// Places `moving_id` (with its own `moving_span`, unchanged by the move)
/// at `(target_row, target_col)`, and relocates any card that drop directly
/// overlaps to the nearest still-free cell *to that card's own original
/// position* — never a same-column cascade the way `resolve_drop` works
/// (backlog #102 follow-up: displacement should read as "give way and
/// settle back down as close as you started", not "get shoved further down
/// your own column"). Manual mode only — see `resolve_drop`'s doc comment
/// for why Auto mode must keep using that one instead. `occupied` is every
/// *other* card
/// currently in the zone as `(id, row, col, span)` — the dragged item's own
/// entry must already be excluded (its old cell is vacated by this move).
///
/// Only directly-displaced cards ever move: since every relocation lands on
/// a cell verified free against the occupancy *as updated by the moves
/// already made* in this same call, a relocated card can never itself
/// collide with anything else, so there is no further transitive cascade to
/// resolve — this always terminates in at most `occupied.len()` placements.
/// Displaced ids are processed in ascending-id order for determinism (ties
/// only matter when two displaced cards would otherwise contend for the
/// same nearest free cell — the lower id wins it, matching this file's
/// existing tie-break convention).
///
/// `reserved_col0_floor` is the zone's column-0 reservation (`reserved_floor`
/// applies it per candidate column, including the target). `col_weight`
/// scales the column term of every distance comparison exactly like
/// `nearest_neighbor_translate`'s, so "nearest" means the same physical
/// distance in both places.
///
/// Returns the moving card plus every id that had to relocate — never the
/// untouched rest of `occupied` (callers use this as a sparse overlay, not
/// a full snapshot).
#[allow(clippy::too_many_arguments)] // params are distinct fixed geometry/id values, not a bundle
pub fn place_with_relocation(
    occupied: &[(i64, i32, i32, i32)],
    moving_id: i64,
    moving_span: i32,
    target_row: i32,
    target_col: i32,
    cols: i32,
    reserved_col0_floor: i32,
    col_weight: i32,
) -> Vec<(i64, i32, i32, i32)> {
    let cols = cols.max(1);
    let target_col = target_col.clamp(0, cols - 1);
    let target_row = target_row.max(reserved_floor(target_col, reserved_col0_floor));

    let mut by_col: Vec<Vec<(i64, i32, i32)>> = vec![Vec::new(); cols as usize];
    for &(id, row, col, span) in occupied {
        if id != moving_id && col >= 0 && (col as usize) < by_col.len() {
            by_col[col as usize].push((id, row, span));
        }
    }
    for col_items in by_col.iter_mut() {
        col_items.sort_by_key(|&(_, row, _)| row);
    }

    let target_end = target_row + moving_span;
    let mut displaced: Vec<(i64, i32, i32)> = Vec::new(); // (id, orig_row, span)
    by_col[target_col as usize].retain(|&(id, row, span)| {
        if row < target_end && target_row < row + span {
            displaced.push((id, row, span));
            false
        } else {
            true
        }
    });
    displaced.sort_by_key(|&(id, ..)| id);

    by_col[target_col as usize].push((moving_id, target_row, moving_span));
    by_col[target_col as usize].sort_by_key(|&(_, row, _)| row);

    let mut result = vec![(moving_id, target_row, target_col, moving_span)];
    for (id, orig_row, span) in displaced {
        let (row, col) = nearest_free_slot(
            &by_col,
            orig_row,
            target_col,
            span,
            reserved_col0_floor,
            col_weight,
        );
        by_col[col as usize].push((id, row, span));
        by_col[col as usize].sort_by_key(|&(_, r, _)| r);
        result.push((id, row, col, span));
    }
    result
}

/// True if a card at `(row, col, span)` overlaps anything in `occupied` —
/// and if so, the nearest still-free cell it should relocate to instead.
/// The mirror image of `place_with_relocation`'s own displacement step:
/// there, a moving card claims its target cell and *other* cards yield;
/// here, `id` itself is the one that must yield because something else
/// (typically a just-completed visible drag) already claimed its stored
/// cell out from under it while it wasn't being rendered — backlog #102
/// follow-up, "dragging onto a hidden card's slot" — so its own cell is
/// never "reserved" against `occupied`, unlike a genuine drop target.
/// `occupied` is every card that must *not* move (the ones that just took
/// priority) — `id`'s own current entry must not be included in it.
/// Returns `None` (nothing to do) when `(row, col, span)` was already free.
#[allow(clippy::too_many_arguments)] // params are distinct fixed geometry/id values, not a bundle
pub fn relocate_if_colliding(
    occupied: &[(i64, i32, i32, i32)],
    id: i64,
    row: i32,
    col: i32,
    span: i32,
    cols: i32,
    reserved_col0_floor: i32,
    col_weight: i32,
) -> Option<(i32, i32)> {
    let cols = cols.max(1);
    let col = col.clamp(0, cols - 1);

    let mut by_col: Vec<Vec<(i64, i32, i32)>> = vec![Vec::new(); cols as usize];
    for &(oid, orow, ocol, ospan) in occupied {
        if oid != id && ocol >= 0 && (ocol as usize) < by_col.len() {
            by_col[ocol as usize].push((oid, orow, ospan));
        }
    }
    for col_items in by_col.iter_mut() {
        col_items.sort_by_key(|&(_, r, _)| r);
    }

    let end = row + span;
    let overlaps = by_col[col as usize]
        .iter()
        .any(|&(_, r, s)| r < end && row < r + s);
    if !overlaps {
        return None;
    }
    Some(nearest_free_slot(
        &by_col,
        row,
        col,
        span,
        reserved_col0_floor,
        col_weight,
    ))
}

#[cfg(test)]
mod relocate_if_colliding_tests {
    use super::*;

    #[test]
    fn a_free_cell_needs_no_relocation() {
        let occupied = [(2, 0, 0, 4)];
        assert_eq!(relocate_if_colliding(&occupied, 1, 10, 0, 2, 8, 0, 1), None);
    }

    #[test]
    fn a_colliding_cell_relocates_to_the_nearest_free_slot() {
        // id 1's stored cell (0,0,span 2) now collides with id 2 (which
        // just claimed rows [0,4) in column 0) -- 1 must yield, landing
        // right after 2 in the same column (distance 4) rather than
        // hopping to another column (distance >= col_weight=10).
        let occupied = [(2, 0, 0, 4)];
        assert_eq!(
            relocate_if_colliding(&occupied, 1, 0, 0, 2, 8, 0, 10),
            Some((4, 0))
        );
    }

    #[test]
    fn the_colliding_ids_own_entry_is_never_treated_as_an_obstacle() {
        // Passing id 1 itself inside `occupied` must not make it seem to
        // collide with its own historical position.
        let occupied = [(1, 0, 0, 2)];
        assert_eq!(relocate_if_colliding(&occupied, 1, 0, 0, 2, 8, 0, 1), None);
    }
}

/// Finds the nearest free `(row, col)` cell able to hold a `span`-tall card,
/// searching every column in `by_col` and weighting the column term of the
/// distance by `col_weight` (see `place_with_relocation`'s doc comment).
/// Always returns a usable cell: `nearest_free_row_in_column`'s open-ended
/// tail gap guarantees a match in every column, so this only has to pick
/// the best one across `by_col.len()` (always >= 1) candidates.
fn nearest_free_slot(
    by_col: &[Vec<(i64, i32, i32)>],
    origin_row: i32,
    origin_col: i32,
    span: i32,
    reserved_col0_floor: i32,
    col_weight: i32,
) -> (i32, i32) {
    let mut best: Option<(i32, i32, i32)> = None; // (distance, row, col)
    for (col_idx, items) in by_col.iter().enumerate() {
        let col = col_idx as i32;
        let floor = reserved_floor(col, reserved_col0_floor);
        let row = nearest_free_row_in_column(items, origin_row, span, floor);
        let dist = (row - origin_row).abs() + (col - origin_col).abs() * col_weight;
        if best.is_none_or(|(best_dist, ..)| dist < best_dist) {
            best = Some((dist, row, col));
        }
    }
    let (_, row, col) = best.expect("by_col always has at least one column");
    (row, col)
}

/// Finds the row nearest `origin_row` in a single column, sorted by row,
/// where a `span`-tall card fits without overlapping any existing interval,
/// never below `floor`. Scans the gaps between consecutive items (plus
/// before the first and the open-ended tail after the last) rather than
/// every individual row, since a card can only ever start in a gap.
fn nearest_free_row_in_column(
    items: &[(i64, i32, i32)],
    origin_row: i32,
    span: i32,
    floor: i32,
) -> i32 {
    let mut best: Option<(i32, i32)> = None; // (distance, row)
    let mut cursor = floor;
    let consider = |start: i32, end: i32, best: &mut Option<(i32, i32)>| {
        if end != i32::MAX && end - start < span {
            return;
        }
        let candidate = if origin_row < start {
            start
        } else if end != i32::MAX && origin_row > end - span {
            end - span
        } else {
            origin_row
        };
        let dist = (candidate - origin_row).abs();
        if best.is_none_or(|(best_dist, _)| dist < best_dist) {
            *best = Some((dist, candidate));
        }
    };
    for &(_, row, item_span) in items {
        let row = row.max(floor);
        if row > cursor {
            consider(cursor, row, &mut best);
        }
        cursor = cursor.max(row + item_span);
    }
    consider(cursor, i32::MAX, &mut best);
    best.expect("the open-ended tail gap always yields a candidate")
        .1
}

// ── Responsive column compaction (backlog #50) ──────────────────────────────

/// Maps a real column index (`0..CARD_GRID_COLS`) to the view column it
/// renders in when the grid is compacted below `CARD_GRID_COLS` columns.
/// Deterministic, contiguous, and order-preserving: for any `cols`, real
/// column 0 always maps to view column 0, and increasing `real_col`
/// never decreases the result. Degenerates to the identity mapping at `cols
/// == CARD_GRID_COLS`.
pub fn view_col_of(real_col: i32, cols: i32) -> i32 {
    (real_col * cols) / CARD_GRID_COLS
}

/// Largest vertical gap (row units) that responsive compaction preserves
/// between the last card of one real column and the first card of the next
/// real column stacked into the same view column (backlog #120). The gap is
/// measured *relative* to the previous member's last card, so cards that
/// overlapped vertically (a side-by-side gaggle) pack at gap 0; only a card
/// genuinely below the previous one keeps spacing, capped here. Without the
/// cap a member placed far down left a full-size hole once its column
/// collapsed. `CLUSTER_ROW_THRESHOLD` (24) was itself chosen to roughly match
/// one card's typical height, so that is a sensible "medium card" starting
/// cap; tune against real data if the join still feels too airy or too tight.
pub const COMPACT_GAP_MAX: i32 = 24;

/// For each real column, the vertical offset (row units) it renders at
/// within its view column — the summed occupied height of every real
/// column that renders *before* it in the same view column. Real columns
/// sharing a view column stack directly on top of one another, so each
/// item's own exact row is preserved; only a whole column's vertical
/// starting point shifts. This is why there's no visible seam at the join:
/// the offset is the same running-total masonry math a column already uses
/// internally between its own items.
///
/// The *join gap* between two stacked members is the vertical distance
/// between the previous member's last card and this member's first card
/// (`top - prev_bottom`), clamped to `[0, COMPACT_GAP_MAX]` row units
/// (backlog #120). Cards that overlapped vertically in the original layout —
/// the same gaggle, or two neighbours placed side by side — therefore pack
/// tightly (gap 0), while genuinely-separated islands keep their spacing, up
/// to the cap. Measuring the *relative* distance (not this member's absolute
/// `top`) matters: uncapped, a member placed far down re-added its whole
/// vertical position as a meaningless hole once its column collapsed. Items
/// *within* one member keep their exact rows. With one member per view column
/// (full width) every offset is 0, so wide layouts are unchanged.
///
/// Which member of a view column renders on top is decided by *content*,
/// not by raw real-column index (backlog #107): the member whose topmost
/// occupied row is smallest renders first, so two colliding gaggles stack
/// by where their cards actually sit vertically rather than whichever
/// happens to have the lower stored column number. Members with no
/// occupied row (`height == 0`) sort last, and a real column-0 reservation
/// counts as content starting at row 0, so it always stays on top. Ties
/// keep real-column order.
///
/// The ordering is a pure function of the current occupancy and `cols`, so
/// the exact real<->view round-trip `compact_cell`/`expand_cell` guarantee
/// (backlog #106) is preserved. The known trade-off is that a large enough
/// edit to one member can cross the other's topmost row and flip which
/// whole block renders on top — see backlog #107.
///
/// `occupied` is every item currently in the zone as `(row, col, span)` —
/// deliberately *not* filtered to exclude an in-progress drag's own item:
/// while dragging, the dragged card renders via a `position: fixed`
/// overlay, genuinely vacating its origin cell, but every *other* card's
/// compaction must stay stable for the whole drag, so this always reads
/// from the frozen pickup-time snapshot including the dragged item's
/// original slot. `reserved_col0_floor` is real column 0's own reserved
/// floor (0 for a zone with no reservation).
pub fn column_offsets(
    occupied: &[(i32, i32, i32)],
    cols: i32,
    reserved_col0_floor: i32,
) -> Vec<i32> {
    let n = CARD_GRID_COLS as usize;
    // Per real column: occupied height (`max(row + span)`) and topmost
    // occupied row (`min(row)`); an empty column keeps `top == i32::MAX` so
    // it sorts after every column with content.
    let mut height = vec![0i32; n];
    let mut top = vec![i32::MAX; n];
    for &(row, col, span) in occupied {
        if !(0..CARD_GRID_COLS).contains(&col) {
            continue;
        }
        let idx = col as usize;
        height[idx] = height[idx].max(row + span);
        top[idx] = top[idx].min(row);
    }
    if reserved_col0_floor > 0 {
        height[0] = height[0].max(reserved_col0_floor);
        top[0] = 0;
    }

    let mut groups: Vec<Vec<i32>> = vec![Vec::new(); cols.max(1) as usize];
    for c in 0..CARD_GRID_COLS {
        groups[view_col_of(c, cols) as usize].push(c);
    }

    let mut offsets = vec![0i32; n];
    for group in groups.iter_mut() {
        group.sort_by_key(|&c| (top[c as usize], c));
        // Render members top-to-bottom. The join gap is the *relative*
        // distance between the previous member's last card and this member's
        // first card — `top_c - prev_bottom` — clamped to `[0, MAX]`
        // (backlog #120): cards that vertically overlapped in the original
        // layout (same gaggle, e.g. side by side) pack tightly (gap 0),
        // genuinely-separated islands keep their spacing up to the cap, and a
        // member placed far below no longer re-adds its whole absolute row
        // position as a hole.
        let mut prev_offset = 0i32;
        let mut prev_bottom = 0i32;
        let mut placed_any = false;
        for &c in group.iter() {
            let idx = c as usize;
            // An empty real column has no cards: keep the pass-through offset
            // (its rendered bottom) and let it contribute no height.
            if top[idx] == i32::MAX || height[idx] == 0 {
                offsets[idx] = prev_offset + prev_bottom;
                continue;
            }
            let offset = if placed_any {
                let gap = (top[idx] - prev_bottom).clamp(0, COMPACT_GAP_MAX);
                prev_offset + prev_bottom + gap - top[idx]
            } else {
                0
            };
            offsets[idx] = offset;
            prev_offset = offset;
            prev_bottom = height[idx];
            placed_any = true;
        }
    }
    offsets
}

/// Real `(row, col)` -> view `(row, col)` for rendering, given
/// `column_offsets`' output for the zone's current items and `cols`.
pub fn compact_cell(real_row: i32, real_col: i32, cols: i32, offsets: &[i32]) -> (i32, i32) {
    let v = view_col_of(real_col, cols);
    (real_row + offsets[real_col as usize], v)
}

/// Inverse of `compact_cell` — maps a view (pointer-resolved) `(row, col)`
/// back to the real `(row, col)` it targets, so dragging in compact mode
/// resolves to an exact real grid cell instead of a lossy reflow. Among the
/// real columns sharing `virtual_col`, picks the member whose rendered
/// *occupied* span contains `virtual_row`. Capping the join gap (backlog
/// #120) can make a member's `offset` smaller than the previous member's, so
/// the key is not the offset itself but the member's rendered occupied start
/// `offset + top` — that is always strictly increasing (each member starts at
/// the previous member's rendered end plus a non-negative capped gap), and
/// occupied spans stay disjoint and ordered. So the chosen member is the one
/// with the greatest `offset + top <= virtual_row`; for any actually-occupied
/// cell this is exact. Gaps between members resolve to the member above.
///
/// Content-ordering (backlog #107) puts every *empty* member's notional start
/// at `content_end`, the last occupied member's rendered bottom. A drop in the
/// capped join gap — or one just below a displaced card's new home, one row
/// under the member above — can therefore fall at or past `content_end`, where
/// both an occupied member and an empty one could claim the row. Resolving it
/// to the empty member is wrong: the drop then lands in a *different* real
/// column that content-sorts above the member it was meant to sit under, so
/// the dragged card visibly swaps places with the card it just displaced.
/// So whenever the group has *any* occupied member at or above `virtual_row`,
/// prefer the occupied member (greatest rendered start <= row); targeted empty
/// members are only reached for a view column with no occupied member at all.
///
/// `occupied` is the same `(row, col, span)` triples `column_offsets` took.
/// Falls back to the group's first real column at row 0 when `virtual_row`
/// precedes every member's start.
pub fn expand_cell(
    virtual_row: i32,
    virtual_col: i32,
    cols: i32,
    offsets: &[i32],
    occupied: &[(i32, i32, i32)],
) -> (i32, i32) {
    let n = CARD_GRID_COLS as usize;
    let mut top = vec![i32::MAX; n];
    for &(row, col, _span) in occupied {
        if !(0..CARD_GRID_COLS).contains(&col) {
            continue;
        }
        let idx = col as usize;
        top[idx] = top[idx].min(row);
    }

    let mut real_col = 0;
    let mut best_start = i32::MIN;
    let mut found = false;
    let consider =
        |c: i32, start: i32, best_start: &mut i32, real_col: &mut i32, found: &mut bool| {
            if !*found || start > *best_start || (start == *best_start && c > *real_col) {
                *real_col = c;
                *best_start = start;
                *found = true;
            }
        };

    // Prefer the occupied member above (greatest rendered start <= row).
    for c in 0..CARD_GRID_COLS {
        let idx = c as usize;
        if view_col_of(c, cols) != virtual_col || top[idx] == i32::MAX {
            continue;
        }
        let start = offsets[idx] + top[idx];
        if start <= virtual_row {
            consider(c, start, &mut best_start, &mut real_col, &mut found);
        }
    }
    if !found {
        // `virtual_row` precedes the first occupied card: target the topmost
        // occupied member (smallest start, then smallest col), clamping
        // `real_row` to >= 0 below.
        let mut best = i32::MAX;
        let mut col = 0;
        let mut any = false;
        for c in 0..CARD_GRID_COLS {
            let idx = c as usize;
            if view_col_of(c, cols) != virtual_col || top[idx] == i32::MAX {
                continue;
            }
            let start = offsets[idx] + top[idx];
            if !any || start < best || (start == best && c < col) {
                best = start;
                col = c;
                any = true;
            }
        }
        if any {
            real_col = col;
            found = true;
        }
    }

    if !found {
        // The whole view column is empty: target an empty member — greatest
        // notional start <= row, highest real column on a tie.
        for c in 0..CARD_GRID_COLS {
            let idx = c as usize;
            if view_col_of(c, cols) != virtual_col || top[idx] != i32::MAX {
                continue;
            }
            if offsets[idx] <= virtual_row {
                consider(c, offsets[idx], &mut best_start, &mut real_col, &mut found);
            }
        }
    }

    if !found {
        // `virtual_row` precedes every member's rendered start; fall back to
        // the group's first real column at row 0.
        real_col = (0..CARD_GRID_COLS)
            .find(|&c| view_col_of(c, cols) == virtual_col)
            .unwrap_or(0);
    }
    let real_row = (virtual_row - offsets[real_col as usize]).max(0);
    (real_row, real_col)
}

/// How many `card_width`-wide columns (at `gap` spacing) fit in `width` px
/// without overflowing, clamped to `[1, max_cols]` — the container never
/// needs a horizontal scrollbar because the rendered column count always
/// fits what's actually available.
pub fn effective_cols(width: f64, card_width: f64, gap: f64, max_cols: i32) -> i32 {
    (((width + gap) / (card_width + gap)).floor() as i32).clamp(1, max_cols)
}

// ── Height self-correction (backlog #48) ────────────────────────────────────

/// Given a zone's current items as `(id, row, col, stored_span)` and a map
/// of freshly `measured` spans (only entries that differ from `stored_span`
/// need to be present — callers should already have filtered), returns
/// every item whose position or span needs to change: each measured item
/// gets its corrected span at its *existing* row, then each column is
/// re-stacked (`stack_column`) to cascade away any resulting overlap. Items
/// with no entry in `measured` and no overlap forced onto them by a
/// neighbor keep their exact current position, and are omitted from the
/// result — an empty return means nothing to persist. `reserved_col0_floor`
/// keeps column 0 from ever being pushed to start before its zone's own
/// reservation (0 for a zone with no reservation).
pub fn correct_spans(
    zone_items: &[(i64, i32, i32, i32)],
    measured: &HashMap<i64, i32>,
    reserved_col0_floor: i32,
) -> Vec<GridPosition> {
    if measured.is_empty() {
        return vec![];
    }
    let cols = zone_items.iter().map(|&(_, _, c, _)| c).max().unwrap_or(0) + 1;
    let mut result = vec![];
    for col in 0..cols {
        // The phantom reservation goes first: `stack_column`'s sort is
        // stable, so when it ties on row_start with a real item (the
        // common post-migration case — everything parked at row 0), it
        // must win that tie and push the real item down, not the other way
        // around. Same reasoning as `resolve_drop`'s ordering.
        let mut entries: Vec<(i64, i32, i32)> = vec![];
        if col == 0 && reserved_col0_floor > 0 {
            entries.push((-1, 0, reserved_col0_floor)); // phantom reservation
        }
        entries.extend(
            zone_items
                .iter()
                .filter(|&&(_, _, c, _)| c == col)
                .map(|&(id, row, _, span)| (id, row, measured.get(&id).copied().unwrap_or(span))),
        );
        let stacked = stack_column(entries);
        for (id, row, span) in stacked {
            if id == -1 {
                continue; // phantom reservation, never a real item
            }
            let original = zone_items.iter().find(|&&(oid, _, _, _)| oid == id);
            let Some(&(_, orig_row, orig_col, orig_span)) = original else {
                continue;
            };
            if row != orig_row || span != orig_span {
                result.push(GridPosition {
                    id,
                    grid_row: row,
                    grid_col: orig_col,
                    grid_row_span: span,
                });
            }
        }
    }
    result
}

// ── Server-side masonry placement (auto-layout, new-item placement) ────────

/// Given every existing item's `(row, col, span)` in a zone, the `(row,
/// col)` for a new item of `new_span` that lands in the currently-shortest
/// column, honoring `col0_floor` (a reserved starting height for column 0 —
/// 0 if none). `new_span` is unused today (which column is shortest doesn't
/// depend on the placement's own span, only on how much it grows the
/// chosen column by afterward) but kept in the signature so callers always
/// have it on hand for a future column-capacity heuristic.
pub fn shortest_column(
    rows: &[(i32, i32, i32)],
    cols: i32,
    col0_floor: i32,
    new_span: i32,
) -> (i32, i32) {
    let mut heights = column_heights(rows, cols, col0_floor);
    place_in_shortest(&mut heights, new_span)
}

/// The per-column heights `shortest_column` scans `rows` to build, exposed
/// on its own so a bulk-placement loop (e.g. `.vcf` import, backlog #89) can
/// compute it once up front and then place many items via
/// `place_in_shortest` — re-running `shortest_column` (a full rescan) for
/// every single item turns an O(n) import into O(n²).
pub fn column_heights(rows: &[(i32, i32, i32)], cols: i32, col0_floor: i32) -> Vec<i32> {
    let mut heights = vec![0i32; cols.max(1) as usize];
    if let Some(h) = heights.get_mut(0) {
        *h = col0_floor;
    }
    for &(row, col, span) in rows {
        if let Some(h) = heights.get_mut(col as usize) {
            *h = (*h).max(row + span);
        }
    }
    heights
}

/// Picks the currently-shortest column from `heights` and reserves
/// `new_span` rows in it (mutating `heights` in place), so consecutive calls
/// place items one after another the same way repeated `shortest_column`
/// calls would — without rescanning already-placed rows each time.
pub fn place_in_shortest(heights: &mut [i32], new_span: i32) -> (i32, i32) {
    let col = heights
        .iter()
        .enumerate()
        .min_by_key(|&(_, h)| *h)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let row = heights.get(col).copied().unwrap_or(0);
    if let Some(h) = heights.get_mut(col) {
        *h = row + new_span;
    }
    (row, col as i32)
}

/// Baseline row-span "chrome" (px) common to every card — padding plus its
/// header row — before any of its optional content lines are counted. See
/// `estimate_span`.
const CARD_ESTIMATE_BASE_PX: f64 = 68.0;

/// Rough per-content-line height (px) used only to seed placement
/// bookkeeping for a card whose real rendered height isn't known yet. See
/// `estimate_span`.
const CARD_ESTIMATE_LINE_PX: f64 = 20.0;

/// Rough row-span estimate for a card that hasn't been rendered yet, sized
/// off how many optional content lines it will show (subtitle, URL, one
/// line per email/phone, notes body, …) rather than a flat guess. Meant
/// only to seed a *bulk*-placement loop's in-memory column-height
/// bookkeeping (`place_in_shortest`) across many consecutive calls — a
/// single placement backed by a fresh DB scan doesn't need this, since
/// `shortest_column` picks the column before adding the new item's own
/// span at all.
///
/// Without this, a loop that always books every new item's growth as `1`
/// (regardless of real content) makes every column look equally short after
/// each step, which degenerates into plain round-robin cycling through
/// columns — so several genuinely tall cards landing `cols` calls apart
/// pile onto the very same column while others, which just happened to
/// receive short cards, stay short. `estimate_span` fixes this by giving
/// the bookkeeping a size proportional to the card's real content, so the
/// "shortest column" picked at each step tracks real, not assumed, height.
/// The client's height self-correction pass (`correct_spans`) reconciles
/// this estimate against the real rendered height on first paint, same as
/// it already does for any placeholder span.
pub fn estimate_span(content_lines: i32) -> i32 {
    let px =
        CARD_ESTIMATE_BASE_PX + CARD_GRID_GAP + content_lines.max(0) as f64 * CARD_ESTIMATE_LINE_PX;
    ((px / CARD_GRID_ROW_UNIT_PX).ceil() as i32).max(1)
}

/// Packs `(id, span)` entries, in order, into `cols` columns — each entry
/// goes into whichever column currently has the smallest total height
/// (masonry packing). `col_floors` seeds each column's starting height
/// (used to reserve a zone's column 0 for chrome like Notes' add-tile).
/// Returns `(id, row, col, span)` in the same order as the input — span is
/// passed through unchanged, just carried along so callers building
/// `GridPosition` don't need a second lookup.
pub fn pack_shortest_column(
    entries: &[(i64, i32)],
    cols: i32,
    col_floors: &[i32],
) -> Vec<(i64, i32, i32, i32)> {
    let mut heights: Vec<i32> = col_floors.to_vec();
    heights.resize(cols as usize, 0);
    entries
        .iter()
        .map(|&(id, span)| {
            let col = heights
                .iter()
                .enumerate()
                .min_by_key(|&(_, h)| *h)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let row = heights[col];
            heights[col] += span;
            (id, row, col as i32, span)
        })
        .collect()
}

/// Packs `entries` (each `(id, span)`, in current reading order) so the
/// result looks even in a grid currently *rendered* at `view_cols` columns
/// (backlog #50's client-side compaction — see `view_col_of`/`column_offsets`)
/// rather than in the underlying `CARD_GRID_COLS`-wide real storage.
///
/// `column_offsets` renders a visual column's member real columns as
/// sequential stacked blocks, so a visual column's total rendered height is
/// exactly the *sum* of its real columns' occupied heights. Plain
/// `pack_shortest_column(entries, CARD_GRID_COLS, ..)` balances all
/// `CARD_GRID_COLS` real columns to equal height — fine when `view_cols ==
/// CARD_GRID_COLS`, but whenever `CARD_GRID_COLS` doesn't divide evenly by
/// `view_cols` (e.g. 8 real columns compacted to 5 visible ones: group
/// sizes `[2, 2, 1, 2, 1]`), a visual column made of two equal-height real
/// columns then renders twice as tall as one made of a single real column —
/// "auto-layout fills one column and leaves others nearly empty", worse the
/// less evenly `view_cols` divides `CARD_GRID_COLS`.
///
/// Fixes this in two levels: first decide which of the `view_cols` *visual*
/// columns each item belongs to (a plain masonry pack at `view_cols` width,
/// so the visual result is balanced for the shape the caller is actually
/// looking at); then, within each visual column, spread its own items
/// across that column's real columns (another masonry pack, scoped to just
/// those real columns) so the result stays reasonable at other viewport
/// widths too — though, per the note above, *which* real column within a
/// group holds an item doesn't affect that group's own rendered height,
/// only the level-1 assignment does.
pub fn pack_for_view(
    entries: &[(i64, i32)],
    view_cols: i32,
    col_floors: &[i32],
) -> Vec<(i64, i32, i32, i32)> {
    let view_cols = view_cols.clamp(1, CARD_GRID_COLS);

    // Real columns belonging to each visual column, and that visual
    // column's own floor (the sum of its real columns' floors).
    let mut members: Vec<Vec<i32>> = vec![Vec::new(); view_cols as usize];
    let mut visual_floors = vec![0i32; view_cols as usize];
    for c in 0..CARD_GRID_COLS {
        let v = view_col_of(c, view_cols) as usize;
        members[v].push(c);
        visual_floors[v] += col_floors.get(c as usize).copied().unwrap_or(0);
    }

    let by_visual = pack_shortest_column(entries, view_cols, &visual_floors);

    let mut result = Vec::with_capacity(entries.len());
    for (v, group) in members.iter().enumerate() {
        let group_entries: Vec<(i64, i32)> = by_visual
            .iter()
            .filter(|&&(_, _, col, _)| col == v as i32)
            .map(|&(id, _, _, span)| (id, span))
            .collect();
        let group_floors: Vec<i32> = group
            .iter()
            .map(|&c| col_floors.get(c as usize).copied().unwrap_or(0))
            .collect();
        let placed = pack_shortest_column(&group_entries, group.len() as i32, &group_floors);
        for (id, row, local_col, span) in placed {
            let real_col = group[local_col as usize];
            result.push((id, row, real_col, span));
        }
    }
    result
}

#[cfg(test)]
mod view_col_of_tests {
    use super::*;

    #[test]
    fn identity_at_full_columns() {
        for c in 0..CARD_GRID_COLS {
            assert_eq!(view_col_of(c, CARD_GRID_COLS), c);
        }
    }

    #[test]
    fn real_column_zero_always_maps_to_view_zero() {
        for cols in 1..=CARD_GRID_COLS {
            assert_eq!(view_col_of(0, cols), 0);
        }
    }

    // These two tests assume CARD_GRID_COLS == 8 (the real column space
    // these view-column mappings are computed over).
    #[test]
    fn two_columns_groups_contiguous_halves() {
        assert_eq!(view_col_of(0, 2), 0);
        assert_eq!(view_col_of(3, 2), 0);
        assert_eq!(view_col_of(4, 2), 1);
        assert_eq!(view_col_of(7, 2), 1);
    }

    #[test]
    fn three_columns_groups_are_balanced() {
        assert_eq!(view_col_of(0, 3), 0);
        assert_eq!(view_col_of(2, 3), 0);
        assert_eq!(view_col_of(3, 3), 1);
        assert_eq!(view_col_of(5, 3), 1);
        assert_eq!(view_col_of(6, 3), 2);
        assert_eq!(view_col_of(7, 3), 2);
    }

    #[test]
    fn one_column_merges_everything() {
        for c in 0..CARD_GRID_COLS {
            assert_eq!(view_col_of(c, 1), 0);
        }
    }

    #[test]
    fn monotonic_non_decreasing_in_real_col() {
        for cols in 1..=CARD_GRID_COLS {
            let mut prev = 0;
            for c in 0..CARD_GRID_COLS {
                let v = view_col_of(c, cols);
                assert!(v >= prev);
                prev = v;
            }
        }
    }
}

#[cfg(test)]
mod column_offsets_tests {
    use super::*;

    // These tests assume CARD_GRID_COLS == 8.
    #[test]
    fn full_columns_gives_all_zero_offsets() {
        let occupied = vec![
            (0, 0, 10),
            (0, 1, 20),
            (0, 2, 5),
            (0, 3, 1),
            (0, 4, 7),
            (0, 5, 2),
            (0, 6, 9),
            (0, 7, 3),
        ];
        let offsets = column_offsets(&occupied, CARD_GRID_COLS, 0);
        assert_eq!(offsets, vec![0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn two_columns_stacks_the_second_half_below_the_first() {
        let occupied = vec![
            (0, 0, 10),
            (0, 1, 4),
            (0, 2, 6),
            (0, 3, 3),
            (0, 4, 5),
            (0, 5, 7),
            (0, 6, 2),
            (0, 7, 8),
        ];
        let offsets = column_offsets(&occupied, 2, 0);
        assert_eq!(offsets, vec![0, 10, 14, 20, 0, 5, 12, 14]);
    }

    #[test]
    fn one_column_stacks_all_eight_in_order() {
        let occupied = vec![
            (0, 0, 5),
            (0, 1, 3),
            (0, 2, 4),
            (0, 3, 2),
            (0, 4, 6),
            (0, 5, 1),
            (0, 6, 3),
            (0, 7, 4),
        ];
        let offsets = column_offsets(&occupied, 1, 0);
        assert_eq!(offsets, vec![0, 5, 8, 12, 14, 20, 21, 24]);
    }

    #[test]
    fn empty_real_column_contributes_zero_height() {
        let occupied = vec![(0, 0, 6)];
        let offsets = column_offsets(&occupied, 2, 0);
        assert_eq!(offsets, vec![0, 6, 6, 6, 0, 0, 0, 0]);
    }

    #[test]
    fn reserved_col0_floor_counts_toward_column_zeros_height() {
        let offsets = column_offsets(&[], 2, 11);
        assert_eq!(offsets, vec![0, 11, 11, 11, 0, 0, 0, 0]);
    }

    #[test]
    fn colliding_columns_order_by_topmost_row_not_real_index() {
        // Real column 3 holds content at row 0; real column 0 holds content
        // much lower, at row 10. Because 3's topmost row is smaller it must
        // render *first* (offset 0), pushing column 0 below it — the exact
        // opposite of the old raw-index ordering (backlog #107). Column 0's
        // card sits 5 rows below column 3's last card, so it packs to a
        // 5-row gap (offset 0), and the empty columns trail at the rendered
        // bottom (15).
        let occupied = vec![(10, 0, 5), (0, 3, 5)];
        let offsets = column_offsets(&occupied, 2, 0);
        assert_eq!(offsets, vec![0, 15, 15, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn equal_topmost_rows_keep_real_column_order_and_empties_last() {
        // Columns 1 and 2 share topmost row 0, so they keep real-column
        // order; empty column 0 (and 3) sort after every column with
        // content, regardless of index.
        let occupied = vec![(0, 1, 3), (0, 2, 4)];
        let offsets = column_offsets(&occupied, 2, 0);
        assert_eq!(offsets, vec![7, 0, 3, 7, 0, 0, 0, 0]);
    }

    #[test]
    fn reserved_column_zero_always_renders_on_top() {
        // A col-0 reservation is content starting at row 0, so it stays
        // first even though it has no occupied rows at all.
        let occupied = vec![(0, 2, 4)];
        let offsets = column_offsets(&occupied, 2, 11);
        assert_eq!(offsets, vec![0, 15, 11, 15, 0, 0, 0, 0]);
    }

    #[test]
    fn collapsed_join_gap_is_capped_at_compact_gap_max() {
        // Real column 1's card sits far below the top (row 100). Stacked
        // under column 0 (bottom 5) its leading whitespace would otherwise
        // inject a 100-row hole; the join gap is capped (backlog #120).
        let occupied = vec![(0, 0, 5), (100, 1, 3)];
        let offsets = column_offsets(&occupied, 1, 0);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 5 - (100 - COMPACT_GAP_MAX));
        let rendered_gap = (offsets[1] + 100) - (offsets[0] + 5);
        assert_eq!(rendered_gap, COMPACT_GAP_MAX);
    }

    #[test]
    fn collapsed_join_gap_within_the_cap_is_preserved_exactly() {
        // Column 0's last card ends at row 5, column 1's first card is at row
        // 10 -> a genuine 5-row gap, preserved exactly (under the cap).
        let occupied = vec![(0, 0, 5), (10, 1, 3)];
        let offsets = column_offsets(&occupied, 1, 0);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 0);
        let rendered_gap = (offsets[1] + 10) - (offsets[0] + 5);
        assert_eq!(rendered_gap, 5);
    }

    #[test]
    fn adjacent_collapsing_columns_pack_tightly() {
        // The two cards overlapped vertically (both start at row 40) in the
        // original layout — the same gaggle, side by side — so once their
        // columns collapse they must pack with zero gap (backlog #120).
        let occupied = vec![(40, 0, 5), (40, 1, 5)];
        let offsets = column_offsets(&occupied, 1, 0);
        assert_eq!(offsets[0], 0);
        let rendered_gap = (offsets[1] + 40) - (offsets[0] + 45);
        assert_eq!(rendered_gap, 0);
    }
}

#[cfg(test)]
mod compact_expand_cell_tests {
    use super::*;

    fn occ_top0_all() -> Vec<(i32, i32, i32)> {
        (0..CARD_GRID_COLS).map(|c| (0, c, 1)).collect()
    }

    // These tests assume CARD_GRID_COLS == 8.
    #[test]
    fn compact_cell_is_identity_at_full_columns() {
        let offsets = vec![0; 8];
        for real_col in 0..CARD_GRID_COLS {
            assert_eq!(
                compact_cell(7, real_col, CARD_GRID_COLS, &offsets),
                (7, real_col)
            );
        }
    }

    #[test]
    fn compact_cell_applies_the_group_offset() {
        let offsets = vec![0, 10, 14, 20, 0, 5, 12, 14];
        assert_eq!(compact_cell(0, 1, 2, &offsets), (10, 0));
        assert_eq!(compact_cell(3, 5, 2, &offsets), (8, 1));
    }

    #[test]
    fn expand_cell_is_identity_at_full_columns() {
        let offsets = vec![0; 8];
        let occ = occ_top0_all();
        for col in 0..CARD_GRID_COLS {
            assert_eq!(
                expand_cell(7, col, CARD_GRID_COLS, &offsets, &occ),
                (7, col)
            );
        }
    }

    #[test]
    fn expand_cell_inverts_compact_cell() {
        let offsets = vec![0, 10, 14, 20, 0, 5, 12, 14];
        let occ = occ_top0_all();
        assert_eq!(expand_cell(10, 0, 2, &offsets, &occ), (0, 1));
        assert_eq!(expand_cell(5, 0, 2, &offsets, &occ), (5, 0));
        assert_eq!(expand_cell(10, 1, 2, &offsets, &occ), (5, 5));
    }

    #[test]
    fn expand_cell_defaults_to_the_groups_first_column() {
        let offsets = vec![0, 5, 8, 12, 14, 20, 21, 24];
        let occ = occ_top0_all();
        assert_eq!(expand_cell(0, 0, 1, &offsets, &occ), (0, 0));
    }

    #[test]
    fn expand_cell_inverts_a_content_ordered_compaction() {
        // column 3 renders first (offset 0, card at row 0), column 0 second
        // (offset 5, card at row 10), then empty columns 1 and 2 (offset 20).
        let offsets = vec![5, 20, 20, 0, 0, 0, 0, 0];
        let occ = vec![(0, 3, 1), (10, 0, 1)];
        // Exact inverse for each occupied item.
        assert_eq!(
            expand_cell(compact_cell(0, 3, 2, &offsets).0, 0, 2, &offsets, &occ),
            (0, 3)
        );
        assert_eq!(
            expand_cell(compact_cell(10, 0, 2, &offsets).0, 0, 2, &offsets, &occ),
            (10, 0)
        );
        // A view row in the gap above column 0's first card resolves to
        // column 3 (the member above); a row past every occupied start keeps
        // resolving to the occupied member above rather than to an empty one
        // in the same view column (backlog #107 regression: a card displaced
        // just below the dragged one must stay in its own column, not be
        // remapped into an empty member that content-sorts above it).
        assert_eq!(expand_cell(4, 0, 2, &offsets, &occ), (4, 3));
        assert_eq!(expand_cell(5, 0, 2, &offsets, &occ), (5, 3));
        assert_eq!(expand_cell(20, 0, 2, &offsets, &occ), (15, 0));
    }

    #[test]
    fn expand_cell_prefers_the_occupied_member_at_the_capped_join_gap() {
        // Real column 3 holds a card at row 0; real column 2 is empty and
        // content-sorts below it. A card displaced to view row 15 (exactly
        // column 3's rendered bottom) must map back to column 3, not the
        // empty column 2 -- otherwise it re-sorts *above* the card it was
        // displaced from and the two visibly swap (backlog #107 regression).
        let occupied = vec![(0, 3, 15)];
        let offsets = column_offsets(&occupied, 5, 0);
        assert_eq!(expand_cell(15, 1, 5, &offsets, &occupied), (15, 3));
    }

    #[test]
    fn expand_cell_inverts_a_capped_compaction() {
        // With a capped (non-monotonic) offset, occupied cells must still
        // round-trip exactly (backlog #120).
        let occupied = vec![(0, 0, 5), (100, 1, 3)];
        let offsets = column_offsets(&occupied, 1, 0);
        for &(row, col, _span) in &occupied {
            let vr = compact_cell(row, col, 1, &offsets).0;
            assert_eq!(expand_cell(vr, 0, 1, &offsets, &occupied), (row, col));
        }
    }
}

#[cfg(test)]
mod effective_cols_tests {
    use super::*;

    const CARD_WIDTH: f64 = 260.0;

    #[test]
    fn fits_four_columns_at_full_desktop_width() {
        // 4*260 + 3*16 = 1088
        assert_eq!(
            effective_cols(1088.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            4
        );
    }

    #[test]
    fn caps_at_max_cols_on_very_wide_screens() {
        assert_eq!(
            effective_cols(10_000.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            CARD_GRID_COLS
        );
    }

    #[test]
    fn drops_to_three_below_four_columns_width() {
        assert_eq!(
            effective_cols(1087.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            3
        );
    }

    #[test]
    fn drops_to_two_below_three_columns_width() {
        // 3*260 + 2*16 = 812
        assert_eq!(
            effective_cols(811.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            2
        );
    }

    #[test]
    fn drops_to_one_below_two_columns_width() {
        // 2*260 + 1*16 = 536
        assert_eq!(
            effective_cols(535.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            1
        );
    }

    #[test]
    fn never_goes_below_one() {
        assert_eq!(
            effective_cols(0.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            1
        );
        assert_eq!(
            effective_cols(-100.0, CARD_WIDTH, CARD_GRID_GAP, CARD_GRID_COLS),
            1
        );
    }
}

#[cfg(test)]
mod cell_at_tests {
    use super::*;

    #[test]
    fn column_is_a_fixed_division_of_cursor_x() {
        assert_eq!(cell_at(0.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4), (0, 0));
        assert_eq!(cell_at(276.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4), (0, 1));
        assert_eq!(cell_at(552.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4), (0, 2));
    }

    #[test]
    fn column_clamps_to_grid_bounds() {
        assert_eq!(cell_at(-500.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4), (0, 0));
        assert_eq!(cell_at(5000.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4), (0, 3));
    }

    #[test]
    fn row_is_a_fixed_division_of_cursor_y() {
        assert_eq!(cell_at(0.0, 0.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4).0, 0);
        assert_eq!(cell_at(0.0, 7.9, 0.0, 0.0, 260.0, 16.0, 8.0, 4).0, 0);
        assert_eq!(cell_at(0.0, 8.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4).0, 1);
        assert_eq!(cell_at(0.0, 800.0, 0.0, 0.0, 260.0, 16.0, 8.0, 4).0, 100);
    }

    #[test]
    fn container_offset_shifts_both_axes() {
        assert_eq!(
            cell_at(300.0, 108.0, 40.0, 100.0, 260.0, 16.0, 8.0, 4),
            (1, 0)
        );
    }
}

#[cfg(test)]
mod stack_column_tests {
    use super::*;

    #[test]
    fn non_overlapping_entries_are_unchanged() {
        let entries = vec![(1, 0, 5), (2, 10, 3)];
        assert_eq!(stack_column(entries.clone()), entries);
    }

    #[test]
    fn overlapping_entry_is_pushed_to_clear_its_predecessor() {
        let entries = vec![(1, 0, 10), (2, 3, 4)];
        assert_eq!(stack_column(entries), vec![(1, 0, 10), (2, 10, 4)]);
    }

    #[test]
    fn cascade_pushes_every_downstream_overlap() {
        let entries = vec![(1, 0, 5), (2, 0, 5), (3, 0, 5)];
        assert_eq!(
            stack_column(entries),
            vec![(1, 0, 5), (2, 5, 5), (3, 10, 5)]
        );
    }

    #[test]
    fn sorts_by_row_start_before_stacking() {
        let entries = vec![(2, 10, 3), (1, 0, 5)];
        assert_eq!(stack_column(entries), vec![(1, 0, 5), (2, 10, 3)]);
    }
}

#[cfg(test)]
mod resolve_drop_tests {
    use super::*;

    #[test]
    fn drop_on_empty_column_just_places_the_item() {
        let moves = resolve_drop(&[], 1, 3, 5, 0);
        assert_eq!(moves, vec![(1, 5, 3)]);
    }

    #[test]
    fn drop_overlapping_an_existing_item_pushes_it_down() {
        let moves = resolve_drop(&[(2, 0, 4)], 1, 2, 0, 0);
        assert_eq!(moves, vec![(1, 0, 2), (2, 2, 4)]);
    }

    #[test]
    fn target_row_is_clamped_to_the_reserved_floor() {
        let moves = resolve_drop(&[], 1, 3, 0, 12);
        assert_eq!(moves, vec![(1, 12, 3)]);
    }

    #[test]
    fn cascade_pushes_the_whole_chain() {
        let moves = resolve_drop(&[(2, 0, 3), (3, 3, 3)], 1, 3, 0, 0);
        assert_eq!(moves, vec![(1, 0, 3), (2, 3, 3), (3, 6, 3)]);
    }
}

#[cfg(test)]
mod place_with_relocation_tests {
    use super::*;

    #[test]
    fn drop_on_an_empty_grid_just_places_the_item() {
        let moves = place_with_relocation(&[], 1, 3, 5, 2, 8, 0, 1);
        assert_eq!(moves, vec![(1, 5, 2, 3)]);
    }

    #[test]
    fn drop_onto_a_free_cell_leaves_everyone_else_untouched() {
        let occupied = [(2, 0, 0, 4), (3, 100, 5, 2)];
        let moves = place_with_relocation(&occupied, 1, 3, 10, 0, 8, 0, 1);
        assert_eq!(moves, vec![(1, 10, 0, 3)]);
    }

    #[test]
    fn target_row_is_clamped_to_the_reserved_floor() {
        let moves = place_with_relocation(&[], 1, 3, 0, 0, 8, 12, 1);
        assert_eq!(moves, vec![(1, 12, 0, 3)]);
    }

    #[test]
    fn a_directly_overlapping_card_relocates_to_the_nearest_free_column_at_its_own_row() {
        // Column 0 has one card at row 0..4. Dropping a span-2 card at
        // (0, 0) must displace it -- with column 1 wide open, it settles
        // right next to its own original row rather than being pushed
        // further down column 0 the way the old cascade did.
        let occupied = [(2, 0, 0, 4)];
        let moves = place_with_relocation(&occupied, 1, 2, 0, 0, 8, 0, 1);
        assert_eq!(moves, vec![(1, 0, 0, 2), (2, 0, 1, 4)]);
    }

    #[test]
    fn relocation_prefers_a_free_row_in_the_same_column_over_a_free_column_elsewhere() {
        // Column 0: card 2 at rows [0,4), the drop target [0,2) overlaps
        // it. Once card 2 vacates, column 0 has a free run starting right
        // after the moving card at row 2 (distance 2 from card 2's own
        // original row 0) -- closer than any cell in a different column
        // (distance >= col_weight). With col_weight=10 the same-column
        // slot must win over relocating to column 1.
        let occupied = [(2, 0, 0, 4)];
        let moves = place_with_relocation(&occupied, 1, 2, 0, 0, 8, 0, 10);
        assert_eq!(moves, vec![(1, 0, 0, 2), (2, 2, 0, 4)]);
    }

    #[test]
    fn two_displaced_cards_are_resolved_in_ascending_id_order_without_colliding() {
        // Both existing cards occupy the exact target interval in column 0;
        // dropping there displaces both. Lower id (2) claims the nearer
        // free slot first; id 3 must search further out and lands
        // somewhere that doesn't collide with either 1 or 2.
        let occupied = [(3, 0, 0, 2), (2, 0, 0, 2)];
        let moves = place_with_relocation(&occupied, 1, 2, 0, 0, 8, 0, 1);
        assert_eq!(moves.len(), 3);
        let mut seen: Vec<(i32, i32)> = Vec::new();
        for &(id, row, col, span) in &moves {
            for &(orow, ocol) in &seen {
                let same_col = col == ocol;
                assert!(
                    !(same_col && row < orow + span && orow < row + span),
                    "id {id} at ({row},{col}) overlaps a previously placed card"
                );
            }
            seen.push((row, col));
        }
    }

    #[test]
    fn an_uninvolved_card_in_another_column_is_never_returned() {
        let occupied = [(2, 0, 0, 4), (3, 50, 5, 3)];
        let moves = place_with_relocation(&occupied, 1, 4, 0, 0, 8, 0, 1);
        assert!(
            !moves.iter().any(|&(id, ..)| id == 3),
            "card 3 was never touched by this drop and must not appear in the result"
        );
    }

    #[test]
    fn no_two_returned_cards_ever_overlap_across_a_range_of_random_layouts() {
        // Property test (backlog #102 follow-up, assertion "cards must
        // never overlap"): for a spread of small pseudo-random layouts and
        // drop targets, the result must never place two cards in the same
        // column with overlapping row ranges.
        let mut seed: u64 = 0x2026_0926;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 33) as i32
        };
        for trial in 0..200 {
            let cols = 4;
            let n = 1 + (next().rem_euclid(6));
            // Stack each card at its column's running cursor so the
            // *starting* layout is always a valid (non-overlapping) one --
            // this test is only meaningful about what
            // `place_with_relocation` does to a valid input, not about
            // whether random rows happen to collide with each other before
            // the function under test ever runs.
            let mut cursor = vec![0i32; cols as usize];
            let mut occupied: Vec<(i64, i32, i32, i32)> = Vec::new();
            for i in 0..n {
                let col = next().rem_euclid(cols);
                let span = 1 + next().rem_euclid(4);
                let row = cursor[col as usize];
                cursor[col as usize] += span;
                occupied.push((i as i64 + 100, row, col, span));
            }
            let moving_span = 1 + next().rem_euclid(4);
            let target_row = next().rem_euclid(20);
            let target_col = next().rem_euclid(cols);
            let moves = place_with_relocation(
                &occupied,
                999,
                moving_span,
                target_row,
                target_col,
                cols,
                0,
                1,
            );

            // Reconstruct the full post-drop layout: untouched occupied
            // cards plus everything `moves` returned (which may override
            // some of them).
            let moved_ids: std::collections::HashSet<i64> =
                moves.iter().map(|&(id, ..)| id).collect();
            let mut full: Vec<(i64, i32, i32, i32)> = occupied
                .iter()
                .copied()
                .filter(|&(id, ..)| !moved_ids.contains(&id))
                .collect();
            full.extend(moves.iter().copied());

            for i in 0..full.len() {
                for j in (i + 1)..full.len() {
                    let (id_a, row_a, col_a, span_a) = full[i];
                    let (id_b, row_b, col_b, span_b) = full[j];
                    if col_a != col_b {
                        continue;
                    }
                    let overlap = row_a < row_b + span_b && row_b < row_a + span_a;
                    assert!(
                        !overlap,
                        "trial {trial}: cards {id_a} and {id_b} overlap in column {col_a} \
                         after dropping at ({target_row},{target_col})"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod correct_spans_tests {
    use super::*;

    #[test]
    fn no_measurements_means_nothing_to_correct() {
        let items = vec![(1, 0, 0, 5)];
        assert_eq!(correct_spans(&items, &HashMap::new(), 0), vec![]);
    }

    #[test]
    fn a_span_that_grew_pushes_the_next_item_down() {
        let items = vec![(1, 0, 0, 3), (2, 3, 0, 4)];
        let measured = HashMap::from([(1, 6)]);
        let moves = correct_spans(&items, &measured, 0);
        assert_eq!(moves.len(), 2);
        assert!(moves
            .iter()
            .any(|m| m.id == 1 && m.grid_row == 0 && m.grid_row_span == 6));
        assert!(moves
            .iter()
            .any(|m| m.id == 2 && m.grid_row == 6 && m.grid_row_span == 4));
    }

    #[test]
    fn a_span_that_shrank_does_not_pull_the_next_item_up() {
        let items = vec![(1, 0, 0, 6), (2, 6, 0, 4)];
        let measured = HashMap::from([(1, 3)]);
        let moves = correct_spans(&items, &measured, 0);
        assert_eq!(
            moves,
            vec![GridPosition {
                id: 1,
                grid_row: 0,
                grid_col: 0,
                grid_row_span: 3
            }]
        );
    }

    #[test]
    fn migrated_items_all_parked_at_row_zero_get_stacked_apart() {
        let items = vec![(1, 0, 0, 1), (2, 0, 0, 1), (3, 0, 0, 1)];
        let measured = HashMap::from([(1, 5), (2, 3), (3, 4)]);
        let moves = correct_spans(&items, &measured, 0);
        assert_eq!(moves.len(), 3);
        let by_id: HashMap<i64, &GridPosition> = moves.iter().map(|m| (m.id, m)).collect();
        assert_eq!((by_id[&1].grid_row, by_id[&1].grid_row_span), (0, 5));
        assert_eq!((by_id[&2].grid_row, by_id[&2].grid_row_span), (5, 3));
        assert_eq!((by_id[&3].grid_row, by_id[&3].grid_row_span), (8, 4));
    }

    #[test]
    fn reserved_col0_floor_keeps_items_below_the_reservation() {
        let items = vec![(1, 0, 0, 1)];
        let measured = HashMap::from([(1, 5)]);
        let moves = correct_spans(&items, &measured, 12);
        assert_eq!(
            moves,
            vec![GridPosition {
                id: 1,
                grid_row: 12,
                grid_col: 0,
                grid_row_span: 5
            }]
        );
    }
}

#[cfg(test)]
mod shortest_column_tests {
    use super::*;

    #[test]
    fn empty_zone_places_at_column_zero() {
        assert_eq!(shortest_column(&[], 4, 0, 1), (0, 0));
    }

    #[test]
    fn picks_the_shortest_column() {
        let rows = vec![(0, 0, 10), (0, 1, 2)];
        assert_eq!(shortest_column(&rows, 2, 0, 1), (2, 1));
    }

    #[test]
    fn col0_floor_is_honored_when_column_zero_is_otherwise_empty() {
        assert_eq!(shortest_column(&[], 2, 11, 1), (0, 1));
    }
}

#[cfg(test)]
mod estimate_span_tests {
    use super::*;

    #[test]
    fn more_content_lines_never_decreases_the_estimate() {
        let mut prev = estimate_span(0);
        for lines in 1..20 {
            let span = estimate_span(lines);
            assert!(
                span >= prev,
                "estimate_span({lines}) = {span} < previous {prev}"
            );
            prev = span;
        }
    }

    #[test]
    fn negative_lines_are_clamped_to_the_zero_line_baseline() {
        assert_eq!(estimate_span(-5), estimate_span(0));
    }

    /// Regression test for the bug reported against real address books
    /// (backlog contacts import): a bulk-placement loop that books every
    /// new item's growth as a flat `1`, regardless of its real content
    /// size, degenerates into round-robin column cycling — so a handful of
    /// information-heavy cards landing `cols` calls apart all pile onto the
    /// same column while the rest, which happened to be short, stay short.
    /// Booking `estimate_span(content_lines)` instead keeps the "shortest
    /// column" bookkeeping proportional to real height, which keeps the
    /// resulting columns close to even.
    #[test]
    fn bulk_placement_must_book_real_span_not_a_flat_placeholder() {
        const COLS: usize = 5;
        // 25 cards, one every `cols` slots is content-heavy (10 lines);
        // under round-robin cycling these all land in the same column.
        let content_lines: Vec<i32> = (0..25).map(|i| if i % 5 == 2 { 10 } else { 1 }).collect();
        let spans: Vec<i32> = content_lines.iter().map(|&l| estimate_span(l)).collect();

        // The bug: bookkeeping always grows the chosen column by 1, no
        // matter the item's real span.
        let mut tracked_flat = vec![0i32; COLS];
        let mut real_heights_flat = vec![0i32; COLS];
        for &span in &spans {
            let (_, col) = place_in_shortest(&mut tracked_flat, 1);
            real_heights_flat[col as usize] += span;
        }
        let spread_flat =
            real_heights_flat.iter().max().unwrap() - real_heights_flat.iter().min().unwrap();

        // The fix: bookkeeping grows the chosen column by the item's own
        // estimated span, so the tracked heights already are the real ones.
        let mut tracked_estimated = vec![0i32; COLS];
        for &span in &spans {
            place_in_shortest(&mut tracked_estimated, span);
        }
        let spread_estimated =
            tracked_estimated.iter().max().unwrap() - tracked_estimated.iter().min().unwrap();

        assert!(
            spread_flat > 4 * spread_estimated,
            "flat-placeholder bookkeeping should be dramatically less balanced \
             than real-span bookkeeping: flat={real_heights_flat:?} (spread {spread_flat}), \
             estimated={tracked_estimated:?} (spread {spread_estimated})"
        );
    }
}

#[cfg(test)]
mod pack_shortest_column_tests {
    use super::*;

    #[test]
    fn fills_the_lowest_column_first() {
        let placed = pack_shortest_column(&[(1, 1), (2, 1), (3, 1)], 2, &[0, 0]);
        assert_eq!(placed, vec![(1, 0, 0, 1), (2, 0, 1, 1), (3, 1, 0, 1)]);
    }

    #[test]
    fn respects_a_tall_entry() {
        let placed = pack_shortest_column(&[(1, 10), (2, 1)], 2, &[0, 0]);
        assert_eq!(placed, vec![(1, 0, 0, 10), (2, 0, 1, 1)]);
    }

    #[test]
    fn honors_col_floors() {
        let placed = pack_shortest_column(&[(1, 1)], 2, &[12, 0]);
        assert_eq!(placed, vec![(1, 0, 1, 1)]);
    }

    #[test]
    fn span_passes_through_unchanged() {
        let placed = pack_shortest_column(&[(1, 7)], 1, &[0]);
        assert_eq!(placed, vec![(1, 0, 0, 7)]);
    }
}

#[cfg(test)]
mod pack_for_view_tests {
    use super::*;

    /// Each real column's own occupied height (`max(row + span)`) — the
    /// same quantity `column_offsets` sums per visual group in production,
    /// used here to check what an auto-layout result actually *renders
    /// as* once compacted for display, not just its raw real assignment.
    fn real_column_heights(placed: &[(i64, i32, i32, i32)]) -> Vec<i32> {
        let mut heights = vec![0i32; CARD_GRID_COLS as usize];
        for &(_, row, col, span) in placed {
            let h = &mut heights[col as usize];
            *h = (*h).max(row + span);
        }
        heights
    }

    fn visual_heights_from_real(real_heights: &[i32], view_cols: i32) -> Vec<i32> {
        let mut heights = vec![0i32; view_cols as usize];
        for (c, &h) in real_heights.iter().enumerate() {
            let v = view_col_of(c as i32, view_cols) as usize;
            heights[v] += h;
        }
        heights
    }

    #[test]
    fn identity_at_full_view_columns() {
        let entries: Vec<(i64, i32)> = (0..8).map(|i| (i as i64, 5)).collect();
        let floors = vec![0; CARD_GRID_COLS as usize];
        let placed = pack_for_view(&entries, CARD_GRID_COLS, &floors);
        let plain = pack_shortest_column(&entries, CARD_GRID_COLS, &floors);
        let mut placed_sorted = placed;
        let mut plain_sorted = plain;
        placed_sorted.sort_by_key(|&(id, ..)| id);
        plain_sorted.sort_by_key(|&(id, ..)| id);
        assert_eq!(placed_sorted, plain_sorted);
    }

    /// Regression test for the reported bug: repacking 8 real columns
    /// evenly and then compacting to a `view_cols` that doesn't evenly
    /// divide `CARD_GRID_COLS` (5, here) renders some visual columns much
    /// taller than others, purely from how many real columns got merged
    /// into them — `pack_for_view` should keep the *visual* columns close
    /// to even instead.
    #[test]
    fn balances_the_rendered_view_not_just_the_real_columns() {
        let floors = vec![0; CARD_GRID_COLS as usize];
        let entries: Vec<(i64, i32)> = (0..24).map(|i| (i as i64, 5)).collect();
        const VIEW_COLS: i32 = 5;

        let old = pack_shortest_column(&entries, CARD_GRID_COLS, &floors);
        let old_visual = visual_heights_from_real(&real_column_heights(&old), VIEW_COLS);
        let old_spread = old_visual.iter().max().unwrap() - old_visual.iter().min().unwrap();

        let new = pack_for_view(&entries, VIEW_COLS, &floors);
        let new_visual = visual_heights_from_real(&real_column_heights(&new), VIEW_COLS);
        let new_spread = new_visual.iter().max().unwrap() - new_visual.iter().min().unwrap();

        assert!(
            new_spread * 2 < old_spread,
            "plain repack should render far less evenly at view_cols={VIEW_COLS} than pack_for_view: \
             plain={old_visual:?} (spread {old_spread}), pack_for_view={new_visual:?} (spread {new_spread})"
        );
    }

    #[test]
    fn every_item_still_gets_a_unique_real_position() {
        let floors = vec![0; CARD_GRID_COLS as usize];
        let entries: Vec<(i64, i32)> = (0..17).map(|i| (i as i64, (i % 4) + 1)).collect();
        let placed = pack_for_view(&entries, 5, &floors);
        assert_eq!(placed.len(), entries.len());
        let mut ids: Vec<i64> = placed.iter().map(|&(id, ..)| id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "every entry placed exactly once");
    }
}

// ── Cohesion-aware clustering (backlog #102) ────────────────────────────────

/// Row-center-and-column adjacency threshold for Manual-mode clustering
/// (backlog #102) — two cards are in the same cluster if their columns
/// are the same or adjacent AND their row-center distance is at most
/// this many row units. Starting value chosen to roughly match one
/// card's own typical height, so cards touching or nearly touching join
/// a cluster while cards separated by a visible gap don't — tune
/// against real data if gaggles feel too eager or too reluctant to
/// merge (see the design spec's Open Questions).
pub const CLUSTER_ROW_THRESHOLD: i32 = 24;

/// One card's id and stored abstract position — the input shape both
/// `detect_clusters` and `pack_clusters_for_view` take.
pub type PlacedEntry = (i64, i32, i32, i32); // (id, grid_row, grid_col, grid_row_span)

/// Groups `entries` into clusters ("gaggles") of visually-close cards —
/// see the design spec's Manual-Mode Reflow Algorithm section for the
/// full rationale. Two cards are adjacent (same cluster) if their
/// columns are the same or differ by 1, AND their row-center distance
/// (`grid_row + grid_row_span / 2`) is at most `CLUSTER_ROW_THRESHOLD`
/// — connected-components over that adjacency graph, so a chain of near
/// cards transitively joins one cluster even if its two ends are
/// individually far apart.
///
/// Buckets entries by `(row_center / CLUSTER_ROW_THRESHOLD, col)` first
/// and only compares within the same or an adjacent row-bucket, instead
/// of an all-pairs O(n²) scan — this is sound because bucket size
/// equals the threshold itself: two centers two or more buckets apart
/// can never be within threshold of each other, so checking only
/// adjacent buckets never misses a real match. Matters at real
/// address-book scale (~1000 cards).
///
/// Returns one `Vec<i64>` (member ids) per cluster; the clusters and
/// each cluster's own ids are in no particular order — callers that
/// need a specific order (rendering) sort afterward.
pub fn detect_clusters(entries: &[PlacedEntry]) -> Vec<Vec<i64>> {
    if entries.is_empty() {
        return vec![];
    }

    // Deterministic id -> index ordering so the union-find below is
    // reproducible regardless of `entries`' own input order.
    let mut sorted: Vec<&PlacedEntry> = entries.iter().collect();
    sorted.sort_by_key(|&&(id, ..)| id);
    let n = sorted.len();

    let centers: Vec<(i32, i32)> = sorted
        .iter()
        .map(|&&(_, row, col, span)| (row + span / 2, col))
        .collect();

    let threshold = CLUSTER_ROW_THRESHOLD.max(1);
    let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, &(row_center, col)) in centers.iter().enumerate() {
        let bucket_row = row_center.div_euclid(threshold);
        buckets.entry((bucket_row, col)).or_default().push(i);
    }

    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }

    for (i, &(row_center, col)) in centers.iter().enumerate() {
        let bucket_row = row_center.div_euclid(threshold);
        for d_row in -1..=1 {
            for d_col in -1..=1 {
                let Some(indices) = buckets.get(&(bucket_row + d_row, col + d_col)) else {
                    continue;
                };
                for &j in indices {
                    if j <= i {
                        continue; // each unordered pair considered once
                    }
                    let (rj, cj) = centers[j];
                    if (col - cj).abs() <= 1 && (row_center - rj).abs() <= CLUSTER_ROW_THRESHOLD {
                        union(&mut parent, i, j);
                    }
                }
            }
        }
    }

    let mut clusters: HashMap<usize, Vec<i64>> = HashMap::new();
    for (i, entry) in sorted.iter().enumerate() {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(entry.0);
    }
    clusters.into_values().collect()
}

#[cfg(test)]
mod detect_clusters_tests {
    use super::*;

    #[test]
    fn empty_input_produces_no_clusters() {
        assert_eq!(detect_clusters(&[]), Vec::<Vec<i64>>::new());
    }

    #[test]
    fn a_single_card_forms_its_own_cluster() {
        let clusters = detect_clusters(&[(1, 0, 0, 5)]);
        assert_eq!(clusters, vec![vec![1]]);
    }

    #[test]
    fn two_near_cards_in_the_same_column_form_one_cluster() {
        // centers: 0+5/2=2, 6+5/2=6+2=8 -> distance 6, well within threshold
        let entries = vec![(1, 0, 0, 5), (2, 6, 0, 5)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 1);
        let mut ids = clusters[0].clone();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn two_far_apart_cards_in_the_same_column_form_separate_clusters() {
        let entries = vec![(1, 0, 0, 5), (2, 1000, 0, 5)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn adjacent_columns_within_row_threshold_join_one_cluster() {
        let entries = vec![(1, 0, 0, 5), (2, 0, 1, 5)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 1);
    }

    #[test]
    fn non_adjacent_columns_never_join_even_at_the_same_row() {
        let entries = vec![(1, 0, 0, 5), (2, 0, 2, 5)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn a_chain_of_near_cards_transitively_joins_even_though_the_ends_are_far_apart() {
        // Consecutive gaps of 20 (within CLUSTER_ROW_THRESHOLD=24) chain
        // all four together, even though card 1 and card 4 are 60 apart
        // on their own -- a single-pairwise-distance check would wrongly
        // split this into two clusters.
        let entries = vec![(1, 0, 0, 1), (2, 20, 0, 1), (3, 40, 0, 1), (4, 60, 0, 1)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 1);
    }

    #[test]
    fn cards_straddling_a_bucket_boundary_still_join_when_within_threshold() {
        // CLUSTER_ROW_THRESHOLD=24 buckets rows into [0,24), [24,48), ...
        // -- these two land in different buckets (0 and 1) but are only
        // 6 apart, well within threshold; the bucket scan must still
        // find them.
        let entries = vec![(1, 20, 0, 1), (2, 26, 0, 1)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 1);
    }

    #[test]
    fn cards_at_exactly_the_threshold_distance_join() {
        let entries = vec![(1, 0, 0, 1), (2, CLUSTER_ROW_THRESHOLD, 0, 1)];
        let clusters = detect_clusters(&entries);
        assert_eq!(
            clusters.len(),
            1,
            "distance exactly CLUSTER_ROW_THRESHOLD must be inclusive (<=), matching the spec's idempotence requirement for a consistent boundary"
        );
    }

    #[test]
    fn cards_one_past_the_threshold_distance_do_not_join() {
        let entries = vec![(1, 0, 0, 1), (2, CLUSTER_ROW_THRESHOLD + 1, 0, 1)];
        let clusters = detect_clusters(&entries);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn clustering_is_independent_of_input_order() {
        let a = vec![(1, 0, 0, 5), (2, 6, 0, 5), (3, 1000, 0, 5)];
        let mut b = a.clone();
        b.reverse();
        let normalize = |v: Vec<Vec<i64>>| {
            let mut v: Vec<Vec<i64>> = v
                .into_iter()
                .map(|mut c| {
                    c.sort_unstable();
                    c
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            normalize(detect_clusters(&a)),
            normalize(detect_clusters(&b))
        );
    }
}

/// Packs every card for a given view width, preserving gaggle structure
/// under resize (backlog #102): ranks `detect_clusters`' output by
/// centroid reading order (row-center first, then column, ties broken
/// by lowest member id), then stacks each cluster as its own floor via
/// `pack_shortest_column`, packed directly into `view_cols` columns
/// (this is the card's actual render position, with no later real-to-
/// virtual compaction step — unlike `pack_for_view`, which exists
/// specifically to balance a wider real column space for such a later
/// step, and would happily spread one cluster's own members several
/// real columns apart, silently breaking `detect_clusters`' column-
/// adjacency rule on a repack).
///
/// Every real column gets the same starting floor: the previous
/// cluster's tallest point, plus `CLUSTER_ROW_THRESHOLD + 1`. That gap
/// is required for idempotence — a zero-gap floor chain would place one
/// cluster's bottom row directly against the next cluster's top row,
/// and re-running `detect_clusters` on the packed output would then
/// find them within threshold and merge two clusters that were
/// deliberately kept separate.
///
/// A cluster's own packing width is always `view_cols` (same as the
/// whole zone) — there's no separate "cluster width" cap, since a
/// cluster naturally can't render wider than the columns available.
pub fn pack_clusters_for_view(entries: &[PlacedEntry], view_cols: i32) -> Vec<GridPosition> {
    if entries.is_empty() {
        return vec![];
    }
    // Every sibling packing function in this file defends against a
    // non-positive `cols` (`column_offsets`, `shortest_column`,
    // `column_heights`); this one must too, since `pack_shortest_column`
    // below resizes its internal heights vec to exactly `view_cols` and
    // panics indexing into it at `view_cols <= 0`.
    let view_cols = view_cols.max(1);

    let by_id: HashMap<i64, &PlacedEntry> = entries.iter().map(|e| (e.0, e)).collect();
    let clusters = detect_clusters(entries);

    // Rank clusters by reading order: top row first, leftmost column to
    // break a tie. Deliberately each cluster's *minimum* row across its
    // own members, not a span-weighted centroid -- every cluster this
    // function places starts its own `pack_shortest_column` call from
    // exactly the same floor (its shelf's row, see below), so on any
    // cluster that shares a shelf with another, the minimum row is
    // always exactly that shared floor, identically on a repack. A
    // centroid isn't: two shelf-mates starting at the *same* floor but
    // with different spans get *different* centroids purely from that
    // span difference, and that relative order can flip once it's fed
    // back in as the next pack's input (found via the idempotence fuzz
    // test below -- two single-member clusters that happened to land
    // on one shelf swapped columns on a repack, because the one with
    // the larger span centroid-outranked the other on the first pass
    // but not the second).
    let mut ranked: Vec<(i32, f64, i64, Vec<i64>)> = clusters
        .into_iter()
        .map(|ids| {
            let members: Vec<&PlacedEntry> =
                ids.iter().filter_map(|id| by_id.get(id).copied()).collect();
            let n = members.len().max(1) as f64;
            let row_top = members
                .iter()
                .map(|&&(_, row, _, _)| row)
                .min()
                .unwrap_or(0);
            let col_centroid: f64 = members
                .iter()
                .map(|&&(_, _, col, _)| col as f64)
                .sum::<f64>()
                / n;
            let min_id = ids.iter().copied().min().unwrap_or(0);
            (row_top, col_centroid, min_id, ids)
        })
        .collect();
    // Bucketed `row_top` first (same `CLUSTER_ROW_THRESHOLD` window
    // `detect_clusters` groups by), falling back to exact `row_top` only
    // after `col_centroid` (backlog #106, found live): a live height
    // self-correction can nudge one cluster member's own row by a few
    // units to fix a real content-height mismatch, shifting that
    // cluster's `row_top` without the user asking for it to move
    // relative to any *other* cluster -- sorting on the exact value
    // first let that alone reorder two clusters (and everything between
    // their old and new shelf) the next time anything reflows (most
    // commonly a resize), even though neither actually moved on screen.
    // Bucketing means a correction has to shift a cluster into a
    // genuinely different row band before it can change inter-cluster
    // order at all.
    let row_bucket = |row_top: i32| row_top.div_euclid(CLUSTER_ROW_THRESHOLD.max(1));
    ranked.sort_by(|a, b| {
        row_bucket(a.0)
            .cmp(&row_bucket(b.0))
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.2.cmp(&b.2))
    });

    let mut result = Vec::with_capacity(entries.len());
    // A "shelf" is a row band shared by however many clusters fit
    // side by side across `view_cols` before it fills up (backlog
    // #102, resolved 2026-09-25 -- see the design spec's "Resolved"
    // note on this function). Replaces an earlier design that gave
    // every cluster its own floor across *all* of `view_cols` -- which
    // meant clusters only ever stacked strictly on top of one another
    // and never spread sideways, no matter how wide the view was --
    // and a windowed-placement design after that, which spread
    // clusters sideways but could coincidentally land two *different*
    // clusters close enough (adjacent columns, similar rows) that
    // `detect_clusters` merged them into one on a repack, breaking
    // idempotence (caught by the fuzz test below).
    //
    // Each cluster on one shelf gets a >=1-column buffer from its
    // shelf-mates -- `detect_clusters`' `col_diff <= 1` rule can then
    // never merge two shelf-mates regardless of row, since they're
    // never column-adjacent. Advancing to a new shelf applies the same
    // `CLUSTER_ROW_THRESHOLD + 1` gap the original single-floor design
    // used between every pair of clusters, uniformly to the *whole*
    // shelf -- so two clusters on different shelves are never
    // row-close enough to merge either, regardless of column. Together
    // these mean this design carries over the original's already-
    // proven idempotence (each cluster is still packed via
    // `pack_shortest_column` from one uniform floor, exactly as
    // before) while letting more than one cluster share that floor.
    let mut shelf_row = 0i32;
    // `None` = nothing placed on the current shelf yet -- distinct from
    // `Some(0)` (the shelf's first cluster ended up at column 0, a
    // width-1 cluster is common), which must NOT be mistaken for empty
    // or the next cluster would overlap it at column 0 too.
    let mut shelf_last_col: Option<i32> = None;
    let mut shelf_bottom = 0i32;

    for (_, _, _, ids) in ranked {
        let mut members: Vec<&PlacedEntry> =
            ids.iter().filter_map(|id| by_id.get(id).copied()).collect();
        // Reading order within the cluster's own floor -- same
        // (row-center, col, id) tie-break as the inter-cluster ranking
        // above, applied per-member instead of per-cluster centroid.
        // Deliberately raw `row`, not the row-center used for clustering
        // and inter-cluster ranking above -- two cards can legitimately
        // land at the same starting row (different real columns) after
        // packing, and if their spans differ, sorting by center instead
        // of raw row would compute a *different* relative order for them
        // on a repack than the one that actually produced that layout,
        // silently reversing their reading order and feeding pack_for_view
        // a different input sequence -- breaking idempotence. Tying on
        // raw row and falling back to `col` anchors the tie-break to
        // whatever column the previous pack actually assigned, which is
        // exactly the reading order that produced it.
        members.sort_by_key(|&&(id, row, col, _span)| (row, col, id));

        let member_entries: Vec<(i64, i32)> = members
            .iter()
            .map(|&&(id, _, _, span)| (id, span))
            .collect();

        // This cluster's own natural width: the column span it's
        // *actually* occupying in the input, not its member count
        // (backlog #106, found live: a cluster with more members than
        // columns used to get stretched across up to `view_cols` columns
        // regardless of how tightly its members were actually packed --
        // `pack_shortest_column`'s height-balancing spreads a new member
        // into whichever column is currently shortest, which for a
        // cluster of, say, ten members and a width of eight uses every
        // one of those eight columns. That claims the *entire* shelf
        // width for one gaggle, leaving no room for a second one to sit
        // beside it -- exactly the "gaggle loses its left/right position
        // relative to its neighbor" bug this was meant to prevent).
        // Never more than `view_cols` allows.
        let min_col = members
            .iter()
            .map(|&&(_, _, col, _)| col)
            .min()
            .unwrap_or(0);
        let max_col = members
            .iter()
            .map(|&&(_, _, col, _)| col)
            .max()
            .unwrap_or(0);
        let width = (max_col - min_col + 1).clamp(1, view_cols);

        // Does this cluster fit on the current shelf, with its
        // required buffer column from whatever's already on it? If
        // not, close this shelf and open a new one below it.
        let start = match shelf_last_col {
            None => 0,
            Some(last) => last + 2,
        };
        let start = if start + width > view_cols {
            shelf_row = shelf_bottom + CLUSTER_ROW_THRESHOLD + 1;
            shelf_bottom = shelf_row;
            0
        } else {
            start
        };

        // Deliberately `pack_shortest_column`, not `pack_for_view`:
        // `pack_for_view` exists specifically to balance a *real*
        // CARD_GRID_COLS-wide space for a later, separate real->virtual
        // compaction step (the model Auto mode's existing flat pack
        // still uses) -- it can legitimately spread two entries several
        // real columns apart. This output has no such downstream step;
        // it *is* the render position. Worse, spreading a cluster's own
        // members several real columns apart breaks idempotence outright:
        // `detect_clusters`' column-adjacency-1 rule would no longer
        // recognize them as one cluster on a repack, silently splitting
        // it. Packing directly into this cluster's own `width`-wide
        // slot keeps its members within `width` of each other, which
        // stays within that adjacency rule for any real view width.
        let floors = vec![shelf_row; width as usize];
        let placed = pack_shortest_column(&member_entries, width, &floors);

        let cluster_bottom = placed
            .iter()
            .map(|&(_, row, _, span)| row + span)
            .max()
            .unwrap_or(shelf_row);

        for (id, row, local_col, span) in placed {
            result.push(GridPosition {
                id,
                grid_row: row,
                grid_col: start + local_col,
                grid_row_span: span,
            });
        }

        shelf_last_col = Some(start + width - 1);
        shelf_bottom = shelf_bottom.max(cluster_bottom);
    }
    result
}

#[cfg(test)]
mod pack_clusters_for_view_tests {
    use super::*;

    #[test]
    fn empty_input_produces_no_output() {
        assert_eq!(pack_clusters_for_view(&[], CARD_GRID_COLS), vec![]);
    }

    /// Found in final review: `view_cols.max(1)` sized the floors vec
    /// but the raw (unclamped) `view_cols` was still passed as
    /// `pack_shortest_column`'s own `cols` argument, which resizes its
    /// internal heights vec to exactly that length -- at `view_cols=0`
    /// this truncates it to empty, and indexing into it on the first
    /// entry panics. `effective_cols` (the real caller) never actually
    /// produces 0, but every sibling function in this file (`column_offsets`,
    /// `shortest_column`, `column_heights`) defends against a non-positive
    /// `cols` anyway -- this function should too, on its own, not by
    /// relying on callers always doing the right thing.
    #[test]
    fn zero_view_cols_does_not_panic() {
        let entries: Vec<PlacedEntry> = vec![(1, 0, 0, 5)];
        let placed = pack_clusters_for_view(&entries, 0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].grid_col, 0);
    }

    #[test]
    fn single_cluster_preserves_its_own_column_span_not_plain_pack_for_view() {
        // All three stacked in the same column -> one cluster whose own
        // natural width is 1 (backlog #106 fourth follow-up) -- NOT
        // spread across `member_count` columns the way a plain
        // `pack_for_view` masonry balance would (that was this test's
        // original assertion, before a cluster's width came from its own
        // column span rather than its member count: found live, a
        // cluster's member count used to always dictate its shelf width
        // regardless of how tightly its members were actually packed,
        // which could claim an entire shelf's width for one gaggle and
        // leave no room for a sibling gaggle to sit beside it). A cluster
        // that was actually packed into one column stays in that one
        // column, stacked by row -- it never gets *wider* than the input
        // it was given.
        let entries: Vec<PlacedEntry> = vec![(1, 0, 0, 5), (2, 4, 0, 5), (3, 8, 0, 5)];
        let placed = pack_clusters_for_view(&entries, CARD_GRID_COLS);

        let mut placed_sorted = placed;
        placed_sorted.sort_by_key(|p| p.id);
        assert_eq!(
            placed_sorted,
            vec![
                GridPosition {
                    id: 1,
                    grid_row: 0,
                    grid_col: 0,
                    grid_row_span: 5
                },
                GridPosition {
                    id: 2,
                    grid_row: 5,
                    grid_col: 0,
                    grid_row_span: 5
                },
                GridPosition {
                    id: 3,
                    grid_row: 10,
                    grid_col: 0,
                    grid_row_span: 5
                },
            ]
        );
    }

    /// Two small, unrelated clusters at a view wide enough to fit both
    /// side by side must actually render side by side (backlog #102) --
    /// not stack, regardless of how far apart their *original* stored
    /// rows happened to be. A resize/reflow re-derives position purely
    /// from the current view width; an old row value from a much wider
    /// or narrower layout carries no claim on the new one.
    #[test]
    fn two_small_clusters_share_a_wide_shelf_side_by_side() {
        let entries: Vec<PlacedEntry> = vec![
            (1, 0, 0, 10),
            (2, 0, 1, 10), // cluster A
            (3, 1000, 0, 5),
            (4, 1000, 1, 5), // cluster B -- stored far below, but plenty of width here
        ];
        let placed = pack_clusters_for_view(&entries, CARD_GRID_COLS);
        let by_id: HashMap<i64, &GridPosition> = placed.iter().map(|p| (p.id, p)).collect();

        assert_eq!(
            by_id[&1].grid_row, by_id[&3].grid_row,
            "both clusters share one shelf"
        );
        let cluster_a_cols: Vec<i32> = [1, 2].iter().map(|id| by_id[id].grid_col).collect();
        let cluster_b_cols: Vec<i32> = [3, 4].iter().map(|id| by_id[id].grid_col).collect();
        let a_max = *cluster_a_cols.iter().max().unwrap();
        let b_min = *cluster_b_cols.iter().min().unwrap();
        assert!(
            b_min - a_max >= 2,
            "cluster B (cols {cluster_b_cols:?}) must leave at least a 1-column gap after \
             cluster A (cols {cluster_a_cols:?}), or detect_clusters would merge them on a repack"
        );
    }

    /// The narrow-view counterpart: when there genuinely isn't room for
    /// a second cluster to share the current shelf, it must advance to
    /// a new one below, with the same idempotence-preserving gap the
    /// original single-shelf design already required.
    #[test]
    fn clusters_that_dont_fit_the_shelf_advance_with_a_gap() {
        let entries: Vec<PlacedEntry> = vec![
            (1, 0, 0, 10),
            (2, 0, 1, 10), // cluster A -- 2 columns wide
            (3, 0, 5, 5),
            (4, 0, 6, 5), // cluster B -- also 2 columns wide
        ];
        // view_cols=3: cluster A (width 2) plus its required buffer
        // column leaves no room for cluster B's own width-2 footprint.
        let placed = pack_clusters_for_view(&entries, 3);
        let by_id: HashMap<i64, &GridPosition> = placed.iter().map(|p| (p.id, p)).collect();

        let cluster_a_bottom = [1, 2]
            .iter()
            .map(|id| by_id[id].grid_row + by_id[id].grid_row_span)
            .max()
            .unwrap();
        let required_floor = cluster_a_bottom + CLUSTER_ROW_THRESHOLD + 1;
        for id in [3, 4] {
            assert!(
                by_id[&id].grid_row >= required_floor,
                "cluster B's card {id} at row {} must start at or below {required_floor} \
                 (cluster A's bottom {cluster_a_bottom} plus the idempotence gap)",
                by_id[&id].grid_row
            );
        }
    }

    #[test]
    fn clusters_render_in_reading_order_regardless_of_input_order() {
        // Cluster B (further down, originally) listed FIRST in the
        // input -- both fit on the same wide shelf, so reading order
        // must show up as column order (A left of B), not row order.
        let entries: Vec<PlacedEntry> = vec![
            (3, 1000, 0, 5),
            (4, 1000, 1, 5), // cluster B
            (1, 0, 0, 10),
            (2, 0, 1, 10), // cluster A
        ];
        let placed = pack_clusters_for_view(&entries, CARD_GRID_COLS);
        let by_id: HashMap<i64, &GridPosition> = placed.iter().map(|p| (p.id, p)).collect();
        assert!(
            by_id[&1].grid_col < by_id[&3].grid_col,
            "cluster A (originally at row 0) must rank -- and so render further left -- \
             than cluster B (row 1000), regardless of input list order"
        );
    }

    /// However many clusters land on however many shelves, no two
    /// placed cards may ever occupy the same cell -- the general
    /// collision-freedom property the earlier row-sorted check assumed
    /// held by construction. It doesn't once clusters can share a
    /// shelf: cards from *different* clusters can legitimately share a
    /// row (side by side), so "no overlap when sorted by row" no longer
    /// distinguishes a real collision from an intentional side-by-side
    /// placement -- an actual per-column range check is required.
    #[test]
    fn no_two_placed_cards_ever_overlap() {
        let entries: Vec<PlacedEntry> = vec![
            (1, 0, 0, 5),
            (2, 1000, 0, 5),
            (3, 2000, 0, 5),
            (4, 3000, 0, 5),
        ];
        let placed = pack_clusters_for_view(&entries, CARD_GRID_COLS);
        for a in &placed {
            for b in &placed {
                if a.id >= b.id {
                    continue;
                }
                if a.grid_col != b.grid_col {
                    continue;
                }
                let a_range = a.grid_row..(a.grid_row + a.grid_row_span);
                let b_range = b.grid_row..(b.grid_row + b.grid_row_span);
                assert!(
                    a_range.start >= b_range.end || b_range.start >= a_range.end,
                    "card {} ({:?}) overlaps card {} ({:?}) in column {}",
                    a.id,
                    a_range,
                    b.id,
                    b_range,
                    a.grid_col
                );
            }
        }
    }

    /// Direct regression test for the reported symptom (found via live
    /// testing, 2026-09-25): four independent gaggles (three singletons,
    /// one pair), each originally stored far apart so none of them
    /// cluster together, at a view wide enough to comfortably fit all
    /// four side by side. Every real render column should end up
    /// occupied by *something* -- not everything crammed into the
    /// first one or two, leaving the rest of the width empty and, as a
    /// direct consequence, leaving Manual-mode dragging with no
    /// cluster-position reference points anywhere except that first
    /// column to translate a drop against.
    #[test]
    fn independent_clusters_spread_across_a_wide_view_not_just_the_first_columns() {
        let entries: Vec<PlacedEntry> = vec![
            (1, 0, 0, 8),    // singleton
            (2, 500, 2, 8),  // singleton
            (3, 1000, 4, 8), // singleton
            (4, 1500, 6, 8), // pair
            (5, 1500, 7, 8), // pair
        ];
        let placed = pack_clusters_for_view(&entries, 6);
        let mut distinct_cols: Vec<i32> = placed.iter().map(|p| p.grid_col).collect();
        distinct_cols.sort_unstable();
        distinct_cols.dedup();
        assert!(
            distinct_cols.len() >= 4,
            "4 independent gaggles at 6 available columns must occupy at least 4 distinct \
             render columns between them, not collapse onto 1-2 -- got columns {distinct_cols:?}"
        );
    }

    #[test]
    fn every_item_still_gets_placed_exactly_once() {
        let entries: Vec<PlacedEntry> = (0..20).map(|i| (i as i64, i * 3, i % 8, 2)).collect();
        let placed = pack_clusters_for_view(&entries, 5);
        assert_eq!(placed.len(), entries.len());
        let mut ids: Vec<i64> = placed.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "every entry placed exactly once");
    }

    /// The design spec's central correctness property: repacking
    /// `pack_clusters_for_view`'s own output, at the same `view_cols`,
    /// must reproduce it exactly. A modest hand-rolled fuzz (fixed seed,
    /// so it's fully deterministic and reproducible) rather than a
    /// handful of hand-picked cases, since this property must hold for
    /// arbitrary inputs, not just convenient ones.
    #[test]
    fn is_idempotent_at_a_fixed_column_count() {
        let mut seed: u64 = 0x5EED_1234;
        let mut next_i32 = |bound: i32| -> i32 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((seed >> 33) as i32).rem_euclid(bound.max(1))
        };

        for view_cols in 1..=CARD_GRID_COLS {
            for trial in 0..25 {
                let n = 5 + (trial % 15);
                let entries: Vec<PlacedEntry> = (0..n)
                    .map(|i| {
                        let row = next_i32(300);
                        let col = next_i32(CARD_GRID_COLS);
                        let span = 1 + next_i32(10);
                        (i as i64, row, col, span)
                    })
                    .collect();

                let first = pack_clusters_for_view(&entries, view_cols);
                let first_as_entries: Vec<PlacedEntry> = first
                    .iter()
                    .map(|p| (p.id, p.grid_row, p.grid_col, p.grid_row_span))
                    .collect();
                let second = pack_clusters_for_view(&first_as_entries, view_cols);

                let mut first_sorted = first;
                first_sorted.sort_by_key(|p| p.id);
                let mut second_sorted = second;
                second_sorted.sort_by_key(|p| p.id);

                assert_eq!(
                    first_sorted, second_sorted,
                    "view_cols={view_cols} trial={trial}: repacking pack_clusters_for_view's \
                     own output at the same width must reproduce it exactly (entries: {entries:?})"
                );
            }
        }
    }
}

/// Interpolates a target coordinate in one coordinate space (`to`-space)
/// from a known coordinate in the other (`from`-space), using whichever
/// existing card's own `from`-space position is nearest the target as a
/// reference point, then applying that same delta in `to`-space. Falls
/// back to `(0, 0)` when `rendered` is empty. Ties on distance broken
/// by lowest `id`, matching `detect_clusters`' own determinism rule.
///
/// This is a heuristic approximation, not an exact inverse — appropriate
/// because `pack_clusters_for_view`'s reflow has no exact inverse to
/// begin with (see the design spec's Idempotence section: it's lossy by
/// design, many different abstract layouts can render identically).
///
/// Used in both directions by Manual-mode drag (Plan D): resolving a
/// drop (`from` = virtual/pointer-resolved cell, `to` = real/storage
/// cell) and previewing the drop shadow (`from` = real/post-cascade
/// cell, `to` = virtual/display cell) — the same trick, with the tuple
/// order swapped per direction. `rendered` is `(id, from_row, from_col,
/// to_row, to_col)` for every other card currently in the zone.
/// `col_weight` scales the column term of the distance comparison only —
/// the returned delta stays in unscaled column-index units, since that's
/// what the `to` side's values mean (see the design spec's "Resolved for
/// Plan D" note on this).
///
/// An entry sitting in the target's own `from`-column is always preferred
/// over every other entry, regardless of `col_weight` or row distance —
/// found via live testing (2026-09-26): dragging a card straight through
/// wide-open empty space, directly above another card in the very same
/// column, made the drop-shadow snap sideways to an unrelated card in a
/// different column simply because that other card happened to be closer
/// in row-distance. Unlike the row/column *trade-off* `col_weight` exists
/// to approximate (there's no way to know how visually "close" a card in
/// a different column really is), the target's own column here isn't a
/// guess at all — the caller already resolved it exactly from the
/// pointer's real screen position — so a same-column entry's row distance
/// is the only genuinely relevant signal, however large. Only when no
/// entry shares that column does the weighted cross-column search below
/// run at all.
pub fn nearest_neighbor_translate(
    rendered: &[(i64, i32, i32, i32, i32)],
    from_target_row: i32,
    from_target_col: i32,
    col_weight: i32,
) -> (i32, i32) {
    let same_column: Vec<&(i64, i32, i32, i32, i32)> = rendered
        .iter()
        .filter(|&&(_, _, fc, ..)| fc == from_target_col)
        .collect();
    let pool: Vec<&(i64, i32, i32, i32, i32)> = if same_column.is_empty() {
        rendered.iter().collect()
    } else {
        same_column
    };
    let nearest = pool.into_iter().min_by_key(|&&(id, fr, fc, ..)| {
        let dist = (fr - from_target_row).abs() + (fc - from_target_col).abs() * col_weight;
        (dist, id)
    });
    let Some(&(_, fr, fc, tr, tc)) = nearest else {
        return (0, 0);
    };
    let delta_row = from_target_row - fr;
    let delta_col = from_target_col - fc;
    ((tr + delta_row).max(0), tc + delta_col)
}

#[cfg(test)]
mod nearest_neighbor_translate_tests {
    use super::*;

    #[test]
    fn empty_rendered_falls_back_to_origin() {
        assert_eq!(nearest_neighbor_translate(&[], 5, 3, 1), (0, 0));
    }

    #[test]
    fn a_single_entry_translates_by_its_own_delta() {
        // from=(10,2), to=(100,20) -- a target 2 away from `from` in
        // row-space should land 2 away from `to` in the same direction.
        let rendered = vec![(1, 10, 2, 100, 20)];
        assert_eq!(nearest_neighbor_translate(&rendered, 12, 2, 1), (102, 20));
    }

    #[test]
    fn picks_whichever_entry_is_nearest_the_target() {
        let rendered = vec![
            (1, 0, 0, 1000, 0),  // far from target
            (2, 10, 0, 2000, 0), // close to target
        ];
        // target (11, 0) is 1 away from entry 2's from-position (10,0),
        // and 11 away from entry 1's (0,0) -- entry 2 must win.
        assert_eq!(nearest_neighbor_translate(&rendered, 11, 0, 1), (2001, 0));
    }

    #[test]
    fn ties_on_distance_are_broken_by_lowest_id() {
        // Both entries are exactly 5 away from the target in row-space.
        let rendered = vec![(5, 5, 0, 500, 0), (2, 15, 0, 200, 0)];
        // target (10, 0): |10-5|=5, |10-15|=5 -- exact tie, id 2 must win.
        // id 2's delta is 10-15=-5, so the result is 200 + (-5) = 195.
        assert_eq!(nearest_neighbor_translate(&rendered, 10, 0, 1), (195, 0));
    }

    #[test]
    fn the_zero_delta_case_lands_exactly_on_the_neighbors_to_position() {
        let rendered = vec![(1, 7, 3, 70, 30)];
        assert_eq!(nearest_neighbor_translate(&rendered, 7, 3, 1), (70, 30));
    }

    #[test]
    fn composing_forward_then_backward_is_idempotent_for_a_coinciding_point() {
        // Translating a point that already coincides with an existing
        // entry's `from` position, then translating the result back
        // using the reversed tuple order, must return exactly where you
        // started -- the two directions are meant to be exact inverses
        // of each other at any point that lands exactly on a neighbor.
        let rendered = vec![(1, 4, 1, 40, 10), (2, 9, 1, 90, 10)];
        let forward = nearest_neighbor_translate(&rendered, 4, 1, 1);
        let reversed: Vec<(i64, i32, i32, i32, i32)> = rendered
            .iter()
            .map(|&(id, fr, fc, tr, tc)| (id, tr, tc, fr, fc))
            .collect();
        let back = nearest_neighbor_translate(&reversed, forward.0, forward.1, 1);
        assert_eq!(back, (4, 1));
    }

    #[test]
    fn col_weight_changes_which_neighbor_is_nearest_when_neither_matches_the_target_column() {
        // Neither entry sits in the target's own column (2) -- entry 1 is
        // 2 columns off, entry 2 is 1 column off -- so this exercises the
        // weighted-fallback path, not the exact-column-match preference.
        let rendered = vec![
            (1, 0, 4, 100, 40), // far in column (2), close in row (0)
            (2, 5, 1, 200, 10), // close in column (1), far in row (5)
        ];
        // Unweighted (col_weight=1): entry 1's distance is 0+2=2, entry
        // 2's is 5+1=6 -- entry 1 wins. Its delta from target (0,2) is
        // row 0-0=0, col 2-4=-2, landing at (100+0, 40-2).
        assert_eq!(nearest_neighbor_translate(&rendered, 0, 2, 1), (100, 38));
        // Column-weighted (col_weight=10): entry 1's distance is
        // 0+2*10=20, entry 2's is 5+1*10=15 -- entry 2 wins instead. Its
        // delta from target (0,2) is row 0-5=-5, col 2-1=1, landing at
        // (200-5, 10+1).
        assert_eq!(nearest_neighbor_translate(&rendered, 0, 2, 10), (195, 11));
    }

    /// Regression test for the reported bug (found via live testing,
    /// 2026-09-26): dragging a card straight north through wide-open
    /// empty space, directly above another card in the very same column,
    /// made the drop-shadow snap sideways to an unrelated card in a
    /// different column simply because that other card happened to be
    /// closer in row-distance -- even though the target's own column was
    /// completely unambiguous from the pointer's real screen position.
    /// An entry sharing the target's exact column must always be
    /// preferred as the anchor over a row-closer entry in a different
    /// column, regardless of `col_weight` -- matching a column exactly is
    /// not something a few rows of extra closeness elsewhere should ever
    /// outweigh, since the target's column is already known precisely
    /// (not something this function is approximating).
    #[test]
    fn an_exact_column_match_is_preferred_over_a_row_closer_different_column_entry() {
        let rendered = vec![
            (1, 100, 0, 1000, 0), // same column as target (0), far in row
            (2, 5, 1, 200, 1),    // different column, very close in row
        ];
        assert_eq!(nearest_neighbor_translate(&rendered, 10, 0, 1), (910, 0));
    }

    /// Companion to the above: with *no* candidate in the target's own
    /// column at all, the weighted fallback across every column is still
    /// the right (only available) approximation.
    #[test]
    fn falls_back_to_the_weighted_search_when_no_candidate_shares_the_target_column() {
        let rendered = vec![
            (1, 100, 3, 1000, 3), // far in row, far in column
            (2, 5, 4, 200, 4),    // close in row, closer in column
        ];
        assert_eq!(nearest_neighbor_translate(&rendered, 10, 0, 1), (205, 0));
    }
}

/// Translates an abstract/storage-space cell to cluster/render space
/// (backlog #102) — the reverse of the drop-resolution direction, used
/// for anything that needs to *display* an abstract-space position
/// during a Manual-mode drag: the drop-shadow preview and a live-
/// cascade-displaced (but not itself dragged) card's own overlay
/// position. Unweighted (`col_weight`=1): both sides of this lookup are
/// already abstract-space column indices, not pixel-derived ones (see
/// `nearest_neighbor_translate`'s doc comment on `col_weight`).
pub fn cluster_render_position(
    occupied: &HashMap<i64, (i32, i32, i32)>,
    cluster_positions: &HashMap<i64, (i32, i32)>,
    row: i32,
    col: i32,
) -> (i32, i32) {
    let rendered: Vec<(i64, i32, i32, i32, i32)> = cluster_positions
        .iter()
        .filter_map(|(&id, &(cr, cc))| occupied.get(&id).map(|&(ar, ac, _)| (id, ar, ac, cr, cc)))
        .collect();
    nearest_neighbor_translate(&rendered, row, col, 1)
}

#[cfg(test)]
mod cluster_render_position_tests {
    use super::*;

    #[test]
    fn translates_abstract_position_to_render_space_via_a_neighbor() {
        let occupied: HashMap<i64, (i32, i32, i32)> = [(9, (0, 6, 8))].into_iter().collect();
        let cluster_positions: HashMap<i64, (i32, i32)> = [(9, (0, 0))].into_iter().collect();
        // Target abstract cell (8, 6) is 8 rows below neighbor 9's own
        // abstract cell (0, 6); applying that same delta to neighbor 9's
        // render cell (0, 0) lands at (8, 0).
        assert_eq!(
            cluster_render_position(&occupied, &cluster_positions, 8, 6),
            (8, 0)
        );
    }

    #[test]
    fn falls_back_to_zero_with_no_neighbors() {
        assert_eq!(
            cluster_render_position(&HashMap::new(), &HashMap::new(), 5, 3),
            (0, 0)
        );
    }
}

/// Combines a `manual_order` comparison with a fallback ordering for
/// when either side (or both) is un-ordered (backlog #102) — mirrors
/// the server's own `manual_order IS NULL, manual_order, <fallback>`
/// sort exactly, so a grid's live client-side Auto-mode preview
/// matches what "Keep this layout" would actually persist. `fallback`
/// is the caller's own comparison for the case where `manual_order`
/// doesn't decide it (both `None`) — e.g. alphabetical for Contacts,
/// reading order for Notes (the two grids' fallbacks differ; see the
/// design spec's `manual_order` section for why Notes' is reading
/// order, not alphabetical).
pub fn manual_order_then(
    a: Option<f64>,
    b: Option<f64>,
    fallback: std::cmp::Ordering,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => fallback,
    }
}

#[cfg(test)]
mod manual_order_then_tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn both_ordered_compares_by_value() {
        assert_eq!(
            manual_order_then(Some(1.0), Some(2.0), Ordering::Equal),
            Ordering::Less
        );
        assert_eq!(
            manual_order_then(Some(2.0), Some(1.0), Ordering::Equal),
            Ordering::Greater
        );
        assert_eq!(
            manual_order_then(Some(1.0), Some(1.0), Ordering::Equal),
            Ordering::Equal
        );
    }

    #[test]
    fn an_ordered_value_always_sorts_before_an_unordered_one() {
        assert_eq!(
            manual_order_then(Some(1000.0), None, Ordering::Greater),
            Ordering::Less
        );
        assert_eq!(
            manual_order_then(None, Some(1000.0), Ordering::Less),
            Ordering::Greater
        );
    }

    #[test]
    fn both_unordered_falls_back_to_the_given_ordering() {
        assert_eq!(
            manual_order_then(None, None, Ordering::Less),
            Ordering::Less
        );
        assert_eq!(
            manual_order_then(None, None, Ordering::Greater),
            Ordering::Greater
        );
        assert_eq!(
            manual_order_then(None, None, Ordering::Equal),
            Ordering::Equal
        );
    }
}

/// Finds `dragged_id`'s immediate neighbors in `(row, col, id)` reading
/// order among `items` (backlog #102's Auto-mode drag-to-reorder) — the
/// two ids `reorder_contact`/`reorder_note` need as `predecessor_id`/
/// `successor_id`. `items` should already reflect the drop that just
/// happened (i.e. every displaced card's post-cascade position, not a
/// pre-drag snapshot). Ties on `(row, col)` break by id ascending,
/// matching `detect_clusters`' own tie-break convention — real cascades
/// never produce a tie (`resolve_drop` never stacks two items on one
/// cell), so this only matters for a caller passing synthetic input.
pub fn neighbors_in_reading_order(
    items: &[(i64, i32, i32)],
    dragged_id: i64,
) -> (Option<i64>, Option<i64>) {
    let mut sorted: Vec<&(i64, i32, i32)> = items.iter().collect();
    sorted.sort_by_key(|&&(id, row, col)| (row, col, id));
    let Some(idx) = sorted.iter().position(|&&(id, ..)| id == dragged_id) else {
        return (None, None);
    };
    let predecessor = idx
        .checked_sub(1)
        .and_then(|i| sorted.get(i))
        .map(|&&(id, ..)| id);
    let successor = sorted.get(idx + 1).map(|&&(id, ..)| id);
    (predecessor, successor)
}

#[cfg(test)]
mod neighbors_in_reading_order_tests {
    use super::*;

    #[test]
    fn empty_items_returns_no_neighbors() {
        assert_eq!(neighbors_in_reading_order(&[], 1), (None, None));
    }

    #[test]
    fn dragged_id_not_present_returns_no_neighbors() {
        let items = vec![(1, 0, 0), (2, 1, 0)];
        assert_eq!(neighbors_in_reading_order(&items, 99), (None, None));
    }

    #[test]
    fn a_single_item_has_no_neighbors() {
        assert_eq!(neighbors_in_reading_order(&[(1, 0, 0)], 1), (None, None));
    }

    #[test]
    fn finds_neighbors_around_a_middle_item() {
        // Deliberately out of id order and out of insertion order, to
        // prove the function sorts by (row, col) itself rather than
        // trusting the caller's ordering.
        let items = vec![(30, 10, 0), (10, 0, 0), (20, 5, 0)];
        assert_eq!(neighbors_in_reading_order(&items, 20), (Some(10), Some(30)));
    }

    #[test]
    fn first_item_has_no_predecessor() {
        let items = vec![(10, 0, 0), (20, 5, 0), (30, 10, 0)];
        assert_eq!(neighbors_in_reading_order(&items, 10), (None, Some(20)));
    }

    #[test]
    fn last_item_has_no_successor() {
        let items = vec![(10, 0, 0), (20, 5, 0), (30, 10, 0)];
        assert_eq!(neighbors_in_reading_order(&items, 30), (Some(20), None));
    }

    #[test]
    fn ties_on_row_and_col_break_by_id() {
        let items = vec![(30, 0, 0), (10, 0, 0), (20, 0, 0)];
        // All three share (row=0, col=0) -- reading order becomes id
        // order: 10, 20, 30.
        assert_eq!(neighbors_in_reading_order(&items, 20), (Some(10), Some(30)));
    }
}

/// Picks the dragged item's predecessor/successor for Auto-mode
/// drag-to-reorder (backlog #102). Prefers `cascade_items` (every id
/// `grid::resolve_drop`'s cascade touched, including the dragged item
/// itself, as `(id, row, col)`) — since a real column is only ever
/// appended to, never reordered, the row order of items sharing that
/// column is trustworthy insertion order, matching `pack_for_view`'s
/// own alphabetical/`manual_order` input order.
///
/// Falls back to the dragged item's neighbors in `fallback_order` (the
/// already-correct alphabetical/`manual_order` id sequence, e.g. the
/// order `NotesView`'s or `ContactsView`'s own Auto-mode preview is
/// already sorted in) when the cascade alone has no same-column
/// neighbor to anchor against — common with a small list, since
/// `pack_for_view` spreads it across up to `CARD_GRID_COLS` real
/// columns and most end up with a single occupant. This matters
/// because `reorder_contact`/`reorder_note`'s own handling of
/// `(None, None)` assigns a single fixed constant
/// (`MANUAL_ORDER_SPACING`) — every drop that fell through to it would
/// otherwise collide on the exact same `manual_order` value.
pub fn auto_mode_reorder_neighbors(
    cascade_items: &[(i64, i32, i32)],
    fallback_order: &[i64],
    dragged_id: i64,
) -> (Option<i64>, Option<i64>) {
    let (predecessor, successor) = neighbors_in_reading_order(cascade_items, dragged_id);
    if predecessor.is_some() || successor.is_some() {
        return (predecessor, successor);
    }
    let fallback_items: Vec<(i64, i32, i32)> = fallback_order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i as i32, 0))
        .collect();
    neighbors_in_reading_order(&fallback_items, dragged_id)
}

#[cfg(test)]
mod auto_mode_reorder_neighbors_tests {
    use super::*;

    #[test]
    fn prefers_cascade_neighbors_when_present() {
        let cascade = vec![(10, 0, 0), (20, 5, 0), (30, 10, 0)];
        let fallback = vec![99, 10, 20, 30, 98]; // deliberately different order
        assert_eq!(
            auto_mode_reorder_neighbors(&cascade, &fallback, 20),
            (Some(10), Some(30))
        );
    }

    #[test]
    fn falls_back_to_array_order_when_the_dragged_item_has_no_cascade_neighbor() {
        // The dragged item (20) is alone in its cascade column -- no
        // same-column neighbor to anchor against.
        let cascade = vec![(20, 0, 0)];
        let fallback = vec![10, 20, 30];
        assert_eq!(
            auto_mode_reorder_neighbors(&cascade, &fallback, 20),
            (Some(10), Some(30))
        );
    }

    #[test]
    fn falls_back_when_the_dragged_item_is_missing_from_the_cascade() {
        let cascade = vec![];
        let fallback = vec![10, 20, 30];
        assert_eq!(
            auto_mode_reorder_neighbors(&cascade, &fallback, 20),
            (Some(10), Some(30))
        );
    }

    #[test]
    fn no_neighbors_anywhere_returns_none() {
        let cascade = vec![(20, 0, 0)];
        let fallback = vec![20];
        assert_eq!(
            auto_mode_reorder_neighbors(&cascade, &fallback, 20),
            (None, None)
        );
    }
}

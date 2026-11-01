//! Shared pointer drag-and-drop for card grids (Notes backlog #47/#48/#50,
//! generalized to Contacts in backlog #82) — layered on top of the pure
//! placement math in `crate::grid`.
//!
//! `DragController` owns the live drag state (shared across every zone in
//! one grid — Notes has two, pinned/non-pinned; Contacts has one) and
//! exposes the three pointer-lifecycle operations as plain methods rather
//! than a Leptos component, since the pointer listeners themselves must be
//! attached by the caller to whatever element is a common DOM ancestor of
//! every zone: pointer capture, set at pickup (`grid::pointer_down_geometry`),
//! keeps delivering `pointermove`/`pointerup` to the dragged card's own
//! ancestor chain regardless of where the pointer physically is, so with
//! more than one zone a single "outer wrapper" listener is required — a
//! listener on each zone's own container individually would stop receiving
//! events the moment the pointer left that zone's rendered area.
//!
//! `card_style`/`drop_shadow_view` are the other two pieces every card grid
//! needs; both are plain functions (not components) for the same reason
//! `note_editor_modal.rs`'s `event_child` pattern exists — a closure this
//! shaped, embedded directly as a `view!` prop or child, trips the macro
//! parser if written inline.

use std::collections::HashMap;

use leptos::prelude::*;

use crate::grid::{self, GridGeometry};

/// How long a touch pointer must hold still before a drag is confirmed
/// (mobile drag support) — long enough to clearly distinguish a
/// deliberate hold from a fast scroll swipe, short enough not to feel
/// laggy. Matches common mobile long-press conventions.
#[allow(dead_code)] // only read from the wasm32 half of begin_touch_hold
const TOUCH_HOLD_MS: i32 = 200;

/// Maximum finger travel (px) allowed during the hold before it's
/// treated as a scroll attempt instead and cancelled.
const TOUCH_HOLD_WIGGLE_PX: f64 = 10.0;

/// A touch pointerdown's not-yet-confirmed long-press (mobile drag
/// support) — see `DragController::pick_up`'s doc comment for why touch
/// doesn't pick up immediately the way mouse/pen do.
#[derive(Clone, Copy, PartialEq)]
struct PendingTouch {
    item_id: i64,
    origin_x: f64,
    origin_y: f64,
    /// Distinguishes this hold from any other (earlier, already
    /// resolved, or a later one that superseded it) — the 200ms timer
    /// callback checks this against the controller's current pending
    /// state before acting, so a stale timer is a no-op instead of
    /// reviving a hold that was already cancelled, dropped, or
    /// superseded by a new pointerdown.
    generation: u64,
}

/// Live drag-and-drop state for one card, held while a pointer button is
/// down over it. `zone` replaces what was a Notes-only `pinned: bool` —
/// Contacts has a single zone (always `0`); Notes uses `0`/`1` for
/// pinned/non-pinned.
///
/// Backlog #106: this used to mix two incompatible coordinate systems for
/// Manual mode — the pointer/collision math worked in *real*
/// (`CARD_GRID_COLS`-wide, storage) space via `expand_cell`/
/// `nearest_neighbor_translate`, while the actual on-screen rendering (once
/// `pack_clusters_for_view`'s cluster cache populated) used a completely
/// different shelf-packed *render* space — the two only ever agreed by
/// coincidence. Every field below now lives in ONE space, `manual`-
/// dependent: real (storage) space for Auto mode, unchanged; render/view
/// space for Manual mode, matching exactly what `render_position`/the
/// cluster cache already show on screen. This is what let a drag displace
/// a card nowhere near the pointer, strand a displaced card somewhere the
/// live preview never showed, or fling it far away — none of those are
/// approximation bugs to patch individually; they were guaranteed by two
/// different coordinate systems being compared as if they were one.
#[derive(Clone, Debug)]
pub struct DragItemState {
    pub item_id: i64,
    pub zone: usize,
    origin_x: f64,
    origin_y: f64,
    /// Offset (px) from the dragged card's own top-left to the pointer,
    /// captured once at pickup. The card's fixed-position overlay is
    /// placed at `pointer - grab_offset` on every move, so it tracks the
    /// pointer 1:1 for the whole drag.
    grab_dx: f64,
    grab_dy: f64,
    /// Max pointer travel distance (px) since pickup — 5px is the
    /// click-vs-drag threshold (matches `week_grid.rs`'s `DragState`).
    pub drag_px: f64,
    /// True while the pointer is hovering a zone other than this drag's
    /// own — the live preview is frozen and a "no-drop" affordance shows.
    pub no_drop: bool,
    /// The last grid cell pointer resolution landed on — real
    /// (`CARD_GRID_COLS`-wide) space for Auto mode, render/view
    /// (`cols`-wide) space for Manual mode. Used both to skip
    /// recomputing the drop preview when the pointer hasn't crossed into
    /// a new cell, and directly as the drop-shadow's own displayed cell
    /// (no second, independent re-derivation — see this struct's doc
    /// comment on why a second heuristic translation is exactly what
    /// caused these bugs).
    pub last_cell: Option<(i32, i32)>,
    /// This item's own row-span, unchanged for the whole drag — dragging
    /// repositions an item, it doesn't resize it.
    moving_span: i32,
    /// Every *other visible* item in this zone as `(row_start, col,
    /// span)`, keyed by id, in this drag's own space (`manual` below) —
    /// snapshotted at pickup and held frozen for the whole drag, so a
    /// hover resolves against the *true* pre-drag layout, never a
    /// compounding view of earlier hover positions this same drag already
    /// previewed and abandoned.
    #[allow(dead_code)] // only read from the wasm32 half of pointer_move
    occupied: HashMap<i64, (i32, i32, i32)>,
    /// Pixel geometry needed to map the pointer to a grid cell — also
    /// frozen at pickup, same reasoning as `occupied`.
    #[allow(dead_code)]
    geometry: GridGeometry,
    /// Effective column count at pickup (backlog #50) — frozen for the
    /// whole drag so a resize mid-drag can't retroactively change what a
    /// hover position means. Real-space width (`grid::CARD_GRID_COLS`)
    /// for Auto mode; render/view width for Manual mode.
    #[allow(dead_code)] // only read from the wasm32 half of pointer_move
    cols: i32,
    /// Auto mode only: this zone's `grid::column_offsets` at pickup, used
    /// to compact the real-space resolution above into the same
    /// render/view cells every Auto-mode card's own resting position
    /// already uses. Manual mode needs no such translation — its own
    /// `occupied`/`cols`/results already live in render/view space
    /// directly — so this is simply empty there.
    #[allow(dead_code)] // only read from the wasm32 half of pointer_move
    col_offsets: Vec<i32>,
    /// `true` for a Manual-mode zone (backlog #102/#106): switches
    /// `pointer_move`/`card_style`/`drop_shadow_view` onto the grid-wide
    /// `place_with_relocation` path, operating and persisting directly in
    /// render/view space. `false` for Auto mode, which keeps the original
    /// column-scoped `resolve_drop` cascade in real (`CARD_GRID_COLS`-
    /// wide) space, compacted for display exactly as a resting card is.
    manual: bool,
}

/// What a completed pointer-up resolved to — the caller decides what to do
/// with each (persist + optimistic local update on `Moved`, open an editor
/// on `Click`), since both are caller-specific (which local signal to
/// write, which editor to open).
pub enum DragOutcome {
    /// Pointer never traveled past the 5px threshold — a plain click.
    Click { item_id: i64 },
    /// Dropped somewhere valid: every id's new `(row, col)`, keyed by id —
    /// includes the dragged item itself plus everyone `grid::resolve_drop`/
    /// `grid::place_with_relocation` displaced to make room for it, all in
    /// this drag's own space (real for Auto mode, render/view for Manual
    /// mode — see `DragItemState`'s doc comment). `item_id` names which one
    /// was actually dragged, so a caller doing Auto-mode drag-to-reorder
    /// (backlog #102) can tell it apart from the ids it merely displaced.
    Moved {
        zone: usize,
        item_id: i64,
        positions: HashMap<i64, (i32, i32)>,
    },
    /// Dropped on a blocked zone, or resolved to no actual change.
    Cancelled,
}

/// Owns one grid's live drag state — shared across every zone belonging to
/// that grid. Cheap to pass around (four `RwSignal`s, all `Copy`).
#[derive(Clone, Copy)]
pub struct DragController {
    pub drag: RwSignal<Option<DragItemState>>,
    /// Live `(left, top)` in fixed viewport px the dragged card should be
    /// drawn at — kept separate from `drag` so per-pixel pointermoves
    /// don't retrigger a full grid rebuild (which reads `drag.get()`);
    /// only the dragged card's own style closure subscribes to this.
    pub drag_offset: RwSignal<(f64, f64)>,
    /// The dragged card's on-screen `(width, height)` at pickup, held
    /// steady for the whole drag so the `position: fixed` overlay doesn't
    /// reflow its own content.
    pub drag_size: RwSignal<(f64, f64)>,
    /// Live preview positions for every item the current hover would
    /// displace (`grid::resolve_drop`'s output), keyed by item id — read
    /// by every card in the zone being dragged in. Cleared on pointer up
    /// (moved into the caller's own display list on a successful drop).
    pub drag_overlay: RwSignal<HashMap<i64, (i32, i32)>>,
    /// A touch pointerdown's not-yet-confirmed long-press, if any
    /// (mobile drag support) — mutually exclusive with `drag`: this is
    /// `Some` only during the `TOUCH_HOLD_MS` window before a touch
    /// drag is confirmed (or cancelled/released early), at which point
    /// it's cleared and `drag` takes over exactly as it already does
    /// for mouse/pen.
    pending_touch: RwSignal<Option<PendingTouch>>,
    /// Monotonic counter backing `PendingTouch::generation` — shared
    /// across every hold (not per-pickup), so each new touch
    /// pointerdown gets a value no earlier pending hold's timer could
    /// still match.
    #[allow(dead_code)] // only read from the wasm32 half of begin_touch_hold
    touch_generation: RwSignal<u64>,
}

impl Default for DragController {
    fn default() -> Self {
        Self::new()
    }
}

impl DragController {
    pub fn new() -> Self {
        Self {
            drag: RwSignal::new(None),
            drag_offset: RwSignal::new((0.0, 0.0)),
            drag_size: RwSignal::new((260.0, 96.0)),
            drag_overlay: RwSignal::new(HashMap::new()),
            pending_touch: RwSignal::new(None),
            touch_generation: RwSignal::new(0),
        }
    }

    /// `Some(id)` only once the pointer has actually traveled past the
    /// click threshold — matches the pre-existing `dragging_id` Memo each
    /// view built by hand. Create this once per view, not per card.
    pub fn dragging_id(&self) -> Memo<Option<i64>> {
        let drag = self.drag;
        Memo::new(move |_| {
            drag.get()
                .and_then(|d| (d.drag_px >= 5.0).then_some(d.item_id))
        })
    }

    /// Call from a card's `on:pointerdown`. `items` is every *other visible*
    /// item currently in `zone` as `(id, row, col, span)`, plus this one's
    /// own current slot (its own slot must keep counting toward its
    /// column's height for `grid::column_offsets` in Auto mode — see that
    /// function's doc comment) — all in this drag's own space: real
    /// (storage) `(grid_row, grid_col)` for Auto mode, or each item's
    /// current render/view position (its cluster-cache entry, falling
    /// back to a plain real-space compaction for one not yet cached) for
    /// Manual mode. `manual` picks which — see `DragItemState`'s doc
    /// comment for why the two must never be mixed.
    ///
    /// Mouse and pen pick up immediately, exactly as always. Touch is
    /// different (mobile drag support): picking up immediately would
    /// mean *every* finger-drag starting on a card — including a normal
    /// swipe meant to scroll the page — gets intercepted, since we'd
    /// have to set `touch-action: none` up front to keep the browser
    /// from stealing the gesture for native scrolling. Instead, a touch
    /// pointerdown starts a `TOUCH_HOLD_MS` hold via `begin_touch_hold`:
    /// if the finger moves more than `TOUCH_HOLD_WIGGLE_PX` before the
    /// hold elapses, that's treated as a scroll attempt and the pending
    /// pickup is simply dropped — `touch-action` was never touched, so
    /// native scrolling was never interrupted. Only once the hold
    /// elapses without that movement does this actually pick up, via
    /// the same `pick_up_confirmed` mouse/pen already use.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_up(
        &self,
        ev: &leptos::web_sys::PointerEvent,
        item_id: i64,
        zone: usize,
        moving_span: i32,
        items: &[(i64, i32, i32, i32)],
        container_ref: NodeRef<leptos::html::Div>,
        cols: i32,
        reserved_col0_floor: i32,
        manual: bool,
    ) {
        let cx = ev.client_x() as f64;
        let cy = ev.client_y() as f64;

        if ev.pointer_type() != "touch" {
            let (grab_dx, grab_dy, width, height) = grid::pointer_down_geometry(ev, cx, cy);
            self.pick_up_confirmed(
                item_id,
                zone,
                moving_span,
                items,
                container_ref,
                cols,
                reserved_col0_floor,
                manual,
                cx,
                cy,
                grab_dx,
                grab_dy,
                width,
                height,
                false,
            );
            return;
        }

        self.begin_touch_hold(
            ev,
            item_id,
            zone,
            moving_span,
            items,
            container_ref,
            cols,
            reserved_col0_floor,
            manual,
            cx,
            cy,
        );
    }

    /// Starts a touch pointerdown's `TOUCH_HOLD_MS` hold (mobile drag
    /// support) — see `pick_up`'s doc comment for the reasoning. A no-op
    /// on non-wasm builds (there's no real timer or DOM to hold against,
    /// and this is only ever reached from real touch input).
    #[allow(clippy::too_many_arguments)]
    fn begin_touch_hold(
        &self,
        ev: &leptos::web_sys::PointerEvent,
        item_id: i64,
        zone: usize,
        moving_span: i32,
        items: &[(i64, i32, i32, i32)],
        container_ref: NodeRef<leptos::html::Div>,
        cols: i32,
        reserved_col0_floor: i32,
        manual: bool,
        cx: f64,
        cy: f64,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::prelude::Closure;
            use leptos::wasm_bindgen::JsCast;

            // Captured as an `EventTarget` (not the event itself) and as
            // owned data, not borrows -- `ev.current_target()` resets to
            // `null` once this event finishes dispatching, and `items`
            // can't outlive this function call as a borrow, but the timer
            // callback below runs `TOUCH_HOLD_MS` later, well after it
            // would be gone.
            let Some(target) = ev.current_target() else {
                return;
            };
            let pointer_id = ev.pointer_id();
            let items = items.to_vec();

            let generation = self.touch_generation.get_untracked() + 1;
            self.touch_generation.set(generation);
            self.pending_touch.set(Some(PendingTouch {
                item_id,
                origin_x: cx,
                origin_y: cy,
                generation,
            }));

            let controller = *self;
            let pending_touch = self.pending_touch;
            let closure = Closure::once(move || {
                if pending_touch.get_untracked().map(|p| p.generation) != Some(generation) {
                    // Cancelled (moved too far), released early (a tap),
                    // or superseded by a newer pointerdown -- either way
                    // this hold is no longer live, do nothing.
                    return;
                }
                pending_touch.set(None);
                let (grab_dx, grab_dy, width, height) =
                    grid::pointer_down_geometry_for(&target, pointer_id, cx, cy);
                controller.pick_up_confirmed(
                    item_id,
                    zone,
                    moving_span,
                    &items,
                    container_ref,
                    cols,
                    reserved_col0_floor,
                    manual,
                    cx,
                    cy,
                    grab_dx,
                    grab_dy,
                    width,
                    height,
                    true,
                );
            });
            if let Some(win) = leptos::web_sys::window() {
                let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    TOUCH_HOLD_MS,
                );
            }
            closure.forget();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (
                ev,
                item_id,
                zone,
                moving_span,
                items,
                container_ref,
                cols,
                reserved_col0_floor,
                manual,
                cx,
                cy,
            );
        }
    }

    /// The actual pickup, common to mouse/pen (immediate) and a
    /// confirmed touch hold (~`TOUCH_HOLD_MS` after pointerdown). `cx`/
    /// `cy` are the pointer's client coordinates at the moment pickup
    /// actually starts (unchanged from pointerdown even for a deferred
    /// touch confirm, since the whole point of the hold is that the
    /// finger hasn't meaningfully moved from there). `via_long_press`
    /// only affects the seeded `drag_px`: a touch hold has already
    /// disambiguated intent by the time it confirms, so there's no
    /// reason to *also* make the user move `5.0` px before the card
    /// visually lifts, the way mouse/pen's own click-vs-drag threshold
    /// requires -- seeding past that threshold makes `dragging_id()`/
    /// `card_style` treat it as already-dragging from the very first
    /// frame, giving the immediate "picked up" feedback a long press is
    /// meant to confirm.
    #[allow(clippy::too_many_arguments)]
    fn pick_up_confirmed(
        &self,
        item_id: i64,
        zone: usize,
        moving_span: i32,
        items: &[(i64, i32, i32, i32)],
        container_ref: NodeRef<leptos::html::Div>,
        cols: i32,
        reserved_col0_floor: i32,
        manual: bool,
        cx: f64,
        cy: f64,
        grab_dx: f64,
        grab_dy: f64,
        width: f64,
        height: f64,
        via_long_press: bool,
    ) {
        let occupied: HashMap<i64, (i32, i32, i32)> = items
            .iter()
            .filter(|&&(id, ..)| id != item_id)
            .map(|&(id, row, col, span)| (id, (row, col, span)))
            .collect();
        let geometry = grid::snapshot_grid_geometry(container_ref);
        // Auto mode only: Manual mode's `items` already live in render/view
        // space (see `pick_up`'s doc comment), so there's nothing left to
        // compact into fewer columns for display -- `col_offsets` stays
        // empty and unused for the whole drag.
        let col_offsets = if manual {
            Vec::new()
        } else {
            let triples: Vec<(i32, i32, i32)> = items
                .iter()
                .map(|&(_, row, col, span)| (row, col, span))
                .collect();
            grid::column_offsets(&triples, cols, reserved_col0_floor)
        };
        let drag_px = if via_long_press { 5.0 } else { 0.0 };
        self.drag.set(Some(DragItemState {
            item_id,
            zone,
            origin_x: cx,
            origin_y: cy,
            grab_dx,
            grab_dy,
            drag_px,
            no_drop: false,
            last_cell: None,
            moving_span,
            occupied,
            geometry,
            cols,
            col_offsets,
            manual,
        }));
        self.drag_size.set((width, height));
        self.drag_offset.set((cx - grab_dx, cy - grab_dy));
    }

    /// Wire as `on:pointermove` on the outer element common to every zone.
    /// `zones` should list every zone sharing this controller (including
    /// the dragged item's own — the no-drop check skips same-zone
    /// automatically), each as `(zone_id, container_ref)`.
    /// `reserved_floor_for(zone) -> i32` supplies each zone's own
    /// column-0 reservation (e.g. Notes' pinned-zone add-tile).
    pub fn pointer_move(
        &self,
        ev: &leptos::web_sys::PointerEvent,
        card_width: f64,
        zones: &[(usize, NodeRef<leptos::html::Div>)],
        reserved_floor_for: impl Fn(usize) -> i32,
    ) {
        let cx = ev.client_x() as f64;
        let cy = ev.client_y() as f64;

        if let Some(pending) = self.pending_touch.get_untracked() {
            let dist = ((cx - pending.origin_x).powi(2) + (cy - pending.origin_y).powi(2)).sqrt();
            if dist > TOUCH_HOLD_WIGGLE_PX {
                // Moved too far before the hold elapsed -- a scroll
                // attempt, not a hold. `touch-action` was never touched
                // for this pointer, so the browser's own native scroll
                // handling continues completely unaffected; there's
                // nothing else to undo here.
                self.pending_touch.set(None);
            }
            return;
        }

        let Some(ds) = self.drag.get() else { return };
        let dist = ((cx - ds.origin_x).powi(2) + (cy - ds.origin_y).powi(2)).sqrt();
        let new_drag_px = ds.drag_px.max(dist);
        if new_drag_px != ds.drag_px {
            self.drag.update(|opt| {
                if let Some(d) = opt {
                    d.drag_px = new_drag_px;
                }
            });
        }
        if new_drag_px < 5.0 {
            return;
        }

        // The card follows the pointer 1:1 at all times — never dependent
        // on anyone's grid cell, so a preview below can never make it
        // visibly jump.
        self.drag_offset.set((cx - ds.grab_dx, cy - ds.grab_dy));

        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::JsCast;

            let invalid = zones.iter().any(|&(zid, container_ref)| {
                zid != ds.zone
                    && container_ref.get().is_some_and(|el| {
                        let el: &web_sys::Element = el.unchecked_ref();
                        let r = el.get_bounding_client_rect();
                        cx >= r.left() && cx <= r.right() && cy >= r.top() && cy <= r.bottom()
                    })
            });
            if invalid != ds.no_drop {
                self.drag.update(|opt| {
                    if let Some(d) = opt {
                        d.no_drop = invalid;
                    }
                });
            }
            if invalid {
                if !self.drag_overlay.get_untracked().is_empty() {
                    self.drag_overlay.set(HashMap::new());
                }
                return;
            }

            // `cell_at` resolves the dragged card's own top-left corner
            // (pointer minus grab offset — the same point `drag_offset`
            // renders the card at, so the shadow lines up with where the
            // card is actually drawn instead of trailing below it) directly
            // to a cell in this drag's own space (`ds.cols`-wide): real
            // (`CARD_GRID_COLS`-wide) for Auto mode, render/view space for
            // Manual mode -- no second translation into or out of a
            // different space needed either way (backlog #106; see
            // `DragItemState`'s doc comment for why an extra translation
            // step here was the actual cause of a whole family of
            // "displaced/dropped somewhere the pointer never was" bugs).
            let cell = grid::cell_at(
                cx - ds.grab_dx,
                cy - ds.grab_dy,
                ds.geometry.container_left,
                ds.geometry.container_top,
                card_width,
                grid::CARD_GRID_GAP,
                grid::CARD_GRID_ROW_UNIT_PX,
                ds.cols,
            );
            // `col_weight` makes every distance comparison below respect the
            // real physical scale difference between a column (`card_width`
            // -ish px) and a row-unit (`CARD_GRID_ROW_UNIT_PX` px) -- shared
            // by the grid-wide relocation search further down, so "nearest"
            // means the same thing in both places.
            let col_weight =
                ((card_width + grid::CARD_GRID_GAP) / grid::CARD_GRID_ROW_UNIT_PX).round() as i32;
            if ds.last_cell != Some(cell) {
                self.drag.update(|opt| {
                    if let Some(d) = opt {
                        d.last_cell = Some(cell);
                    }
                });
                // Always resolved against the frozen pickup-time `occupied`
                // snapshot, never against a previous hover's preview — see
                // `DragItemState::occupied`'s doc comment.
                let (target_row, target_col) = cell;
                let occupied: Vec<(i64, i32, i32, i32)> = ds
                    .occupied
                    .iter()
                    .map(|(&id, &(row, col, span))| (id, row, col, span))
                    .collect();
                let moves: HashMap<i64, (i32, i32)> = if ds.manual {
                    // Grid-wide (not column-scoped): any card the drop
                    // displaces relocates to the nearest still-free cell to
                    // its *own* original position, anywhere in the zone --
                    // `occupied`/`target_col`/`ds.cols` are all already in
                    // the SAME render/view space `place_with_relocation`'s
                    // own result gets persisted and cached in directly, no
                    // translation in either direction.
                    grid::place_with_relocation(
                        &occupied,
                        ds.item_id,
                        ds.moving_span,
                        target_row,
                        target_col,
                        ds.cols,
                        reserved_floor_for(ds.zone),
                        col_weight,
                    )
                    .into_iter()
                    .map(|(id, row, col, _span)| (id, (row, col)))
                    .collect()
                } else {
                    // Auto mode: column-scoped cascade only -- Auto mode's
                    // drag must only ever *reorder*
                    // (`auto_mode_reorder_neighbors` reads this cascade's
                    // row order as the new rank, and the zone gets tightly
                    // repacked by `pack_for_view` right after), so it keeps
                    // the column-scoped `resolve_drop` cascade, which is
                    // gapless by construction and never scatters a card
                    // into an unrelated column.
                    let column_items: Vec<(i64, i32, i32)> = ds
                        .occupied
                        .iter()
                        .filter(|&(_, &(_, col, _))| col == target_col)
                        .map(|(&id, &(row, _, span))| (id, row, span))
                        .collect();
                    let reserved_end =
                        grid::reserved_floor(target_col, reserved_floor_for(ds.zone));
                    grid::resolve_drop(
                        &column_items,
                        ds.item_id,
                        ds.moving_span,
                        target_row,
                        reserved_end,
                    )
                    .into_iter()
                    .map(|(id, row, _span)| (id, (row, target_col)))
                    .collect()
                };
                self.drag_overlay.set(moves);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (cx, cy, card_width, zones, reserved_floor_for);
        }
    }

    /// Wire as `on:pointerup` on the same outer element `pointer_move` is
    /// wired to. `None` if there was no drag in progress (e.g. a stray
    /// pointerup with nothing picked up).
    pub fn pointer_up(&self) -> Option<DragOutcome> {
        if let Some(pending) = self.pending_touch.get_untracked() {
            // Released before the hold elapsed -- a plain tap, same as
            // a quick mouse click.
            self.pending_touch.set(None);
            return Some(DragOutcome::Click {
                item_id: pending.item_id,
            });
        }

        let ds = self.drag.get()?;
        self.drag.set(None);
        self.drag_offset.set((0.0, 0.0));

        if ds.drag_px < 5.0 {
            self.drag_overlay.set(HashMap::new());
            return Some(DragOutcome::Click {
                item_id: ds.item_id,
            });
        }

        let overlay = self.drag_overlay.get_untracked();
        self.drag_overlay.set(HashMap::new());

        if ds.no_drop || overlay.is_empty() {
            return Some(DragOutcome::Cancelled);
        }

        Some(DragOutcome::Moved {
            zone: ds.zone,
            item_id: ds.item_id,
            positions: overlay,
        })
    }

    /// Wire as `on:pointercancel` on the same outer element.
    pub fn cancel(&self) {
        self.pending_touch.set(None);
        self.drag.set(None);
        self.drag_offset.set((0.0, 0.0));
        self.drag_overlay.set(HashMap::new());
    }
}

/// A card's positioning style: `position: fixed` tracking the pointer
/// while it's the one being dragged, or a plain grid-cell placement
/// (applying any live displacement preview from `drag_overlay`)
/// otherwise. Shared by `NoteCard` and `ContactCard`.
#[allow(clippy::too_many_arguments)]
pub fn card_style(
    controller: DragController,
    dragging: Signal<bool>,
    item_id: i64,
    row: i32,
    col: i32,
    span: i32,
    cols: i32,
    col_offsets: Vec<i32>,
) -> impl Fn() -> String + 'static {
    // Narrows this card's dependency down to just its own overlay entry.
    // `drag_overlay` is one shared map covering every card in the zone, so
    // reading it directly inside the closure below would re-run *every*
    // card's style (and repaint its DOM style attribute) on every grid cell
    // the pointer crosses while dragging — imperceptible with a handful of
    // cards, but a visible stutter with hundreds, since almost every card's
    // own entry never actually changes yet still gets recomputed. A `Memo`
    // only notifies subscribers when its own output changes (`PartialEq`):
    // a card that isn't currently displaced sees `None` before and after
    // the map updates, so it never re-renders, leaving the real per-move
    // cost proportional to however many cards `resolve_drop` displaced —
    // not the zone's total card count.
    let overlay_pos = Memo::new(move |_| controller.drag_overlay.get().get(&item_id).copied());
    move || {
        if dragging.get() {
            let (left, top) = controller.drag_offset.get();
            let (width, height) = controller.drag_size.get();
            // `touch-action:none` matters for a touch-confirmed drag
            // (mobile drag support): once a long-press has confirmed and
            // the card is actively tracking the finger, the browser must
            // not *also* try to scroll/pan/zoom based on that same
            // movement. Harmless for mouse/pen, which have no
            // `touch-action` concept to begin with.
            format!(
                "position:fixed;left:{left}px;top:{top}px;width:{width}px;height:{height}px;margin:0;touch-action:none;"
            )
        } else {
            let overlay = overlay_pos.get();
            let (vrow, vcol) = match overlay {
                Some((r, c)) => {
                    // This card is being live-displaced by another card's
                    // active drag cascade (backlog #102). `drag_overlay`'s
                    // values already live in that drag's own final space --
                    // real (`CARD_GRID_COLS`-wide) for Auto mode, matching
                    // this card's own resting compaction below; render/view
                    // space for Manual mode, already final, no compaction
                    // needed (mirrors the resting position's own
                    // `CARD_GRID_COLS`/zero-offset no-op for a cached card
                    // -- see `NoteCard`'s doc comment). `get_untracked`
                    // since `overlay_pos`'s own Memo already re-triggers
                    // this closure on every cascade change; subscribing to
                    // the whole `DragItemState` too would also rerun it on
                    // unrelated per-move field churn (`last_cell`, etc).
                    if controller.drag.get_untracked().is_some_and(|d| d.manual) {
                        (r, c)
                    } else {
                        grid::compact_cell(r, c, cols, &col_offsets)
                    }
                }
                None => {
                    // Real (row, col) -> virtual (row, col) (backlog #50) —
                    // a no-op at cols == grid::CARD_GRID_COLS, so full-width
                    // rendering is byte-for-byte unchanged.
                    grid::compact_cell(row, col, cols, &col_offsets)
                }
            };
            format!(
                "grid-row:{} / span {};grid-column:{};",
                vrow + 1,
                span,
                vcol + 1
            )
        }
    }
}

/// The dashed drop-target shadow shown at the live-hovered cell while
/// dragging within `zone` — `None` whenever this zone isn't the one being
/// dragged in, or the drag hasn't crossed the click threshold yet. Shared
/// by `notes::NotesView` (called once per zone) and `contacts::ContactsView`.
pub fn drop_shadow_view(
    controller: DragController,
    zone: usize,
) -> impl Fn() -> Option<AnyView> + 'static {
    move || {
        let d = controller.drag.get()?;
        if d.zone != zone || d.drag_px < 5.0 {
            return None;
        }
        let (row, col) = d.last_cell?;
        // `last_cell` is the raw pointer-resolved cell, which can sit
        // *inside* a hovered item's occupied span — displaying it as-is
        // would draw the shadow overlapping both that item and whatever
        // comes after it. `drag_overlay` holds the cascade's outcome
        // (keyed by every affected item's id, including this drag's own),
        // so preferring that entry snaps the shadow straight to its
        // actual landing slot — before or after the hovered item, never
        // mid-overlap. Both are already in this drag's own final space
        // (real for Auto mode, render/view for Manual mode — see
        // `DragItemState`'s doc comment), so, unlike before backlog #106,
        // there is no second translation step left to diverge from what
        // actually gets persisted.
        let (vrow, vcol) = controller
            .drag_overlay
            .get()
            .get(&d.item_id)
            .copied()
            .unwrap_or((row, col));
        let span = d.moving_span;
        let drag_size = controller.drag_size;
        Some(
            view! {
                <div
                    class="grid-drop-shadow"
                    style=move || format!(
                        "grid-row:{} / span {};grid-column:{};height:{}px;",
                        vrow + 1, span, vcol + 1, drag_size.get().1,
                    )
                ></div>
            }
            .into_any(),
        )
    }
}

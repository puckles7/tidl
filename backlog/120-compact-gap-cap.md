# Cap the join gap when narrow columns compact

When the viewport narrows, real columns collapse into fewer view columns and
each whole real column's cards are stacked under the previous one. The vertical
gap at that join is exactly the injected column's **leading whitespace**
(`top`), which can be large for a gaggle that was deliberately placed low —
producing a big empty hole in the resulting single/narrow column. Preserve the
gap (so separate islands still read as separate) but cap it at roughly a
medium card's height.

**Status:** done. Full design:
[spec](../../docs/superpowers/specs/2026-10-02-compact-gap-cap-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-02-compact-gap-cap.md).

## Dependencies

- [102 - Cohesion-aware grid reflow](102-cohesion-aware-grid-reflow.md) — the
  abstract-space/drag machinery this shares.
- [106 - `column_offsets` real↔view exactness](106-cluster-cache-real-vs-view-column-confusion.md),
  [107 - content-ordered compaction](107-column-offsets-index-order-not-geometric.md).

## Requirements

- Capping only when a view column holds more than one real column (i.e. at a
  collapsed width); full-width and single-member layouts are byte-for-byte
  unchanged.
- The gap is *reduced*, never removed — separate islands stay distinct.
- `compact_cell`/`expand_cell` stay an exact inverse on all occupied cells, so
  drag-while-narrow never rewrites anyone else's stored position.

## Resolution

- **`crates/tidl-web/src/grid.rs`**: new `COMPACT_GAP_MAX` (24). `column_offsets`
  now stacks each member at the previous one's rendered bottom plus a
  **relative** gap `clamp(top - prev_bottom, 0, MAX)` — overlapping (same-gaggle)
  cards pack tight, separated islands keep spacing up to the cap. Empty members
  keep the pass-through offset. `expand_cell` gained an `occupied` param and
  keys on `offset + top` (strictly increasing even when capped offsets aren't),
  keeping exact occupied-cell inversion; empty members still targetable at the
  tail.
- **Call sites**: `notes/mod.rs` and `contacts/mod.rs` drag resolution pass
  their existing `triples`.
- Tests: capped/uncapped join-gap cases, capped-offset `compact`↔`expand`
  inversion, updated `expand_cell` tests; 596 workspace tests pass; WASM build
  compiles.
- README Notes/Contacts layout descriptions updated.

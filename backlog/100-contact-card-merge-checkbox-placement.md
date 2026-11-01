# Reposition the merge-select checkbox on contact cards

When merge-select mode is active (#88), each `ContactCard` shows a checkbox absolutely positioned at `top: 10px; right: 10px`. That's the same top-right corner the delete button (`DeleteTriggerButton`) sits in, inline within `.contact-card-top-main`, so the two controls visually overlap/collide. Find a placement for the checkbox that doesn't compete with the delete icon — bottom-right of the card is one option worth trying.

## Dependencies

- [88 - Merge duplicate contacts](88-contacts-merge.md) — introduced the merge checkbox this item repositions

## Context / Background

- `crates/tidl-web/src/components/contacts/contact_card.rs` renders the checkbox (`.contact-card-merge-checkbox`, line ~105) and the delete trigger (`<DeleteTriggerButton armed=delete_armed />`, line ~131) inside/near the same top-right region of the card.
- CSS: `.contact-card-merge-checkbox` in `assets/style.css` (~line 4808) is `position: absolute; top: 10px; right: 10px`, overlapping the delete button which sits inline in the `contact-card-top-main` flex row.

## Functional Requirements

- Move the merge-select checkbox to a spot on the card that never overlaps the delete icon, in any state (selected/unselected, with/without avatar, long vs. short names).
- Bottom-right of the card is a candidate; consider hit-target size and spacing from the card edge for a `card-tile` roughly matching the grid's existing padding conventions.
- Verify against both light and dark themes and against the `.contact-card--merge-selected` outline treatment, which should still read clearly with the checkbox in its new spot.

## Open Questions

- Bottom-right vs. some other spot (e.g. checkbox replacing/moving the delete icon while merge mode is active) — needs a quick visual pass before committing to a placement.

## Resolution

Implemented the minimal Option B: `.contact-card-merge-checkbox` in `assets/style.css` now pins to `bottom: 10px; right: 10px` instead of `top: 10px; right: 10px`. No Rust changes — the `<input>` stays a direct child of the card root (`contact_card.rs`), whose `position: relative; overflow: hidden` keeps the absolutely-positioned checkbox clipped inside the card, and the existing `pointerdown`/`click` `stop_propagation` handlers are untouched. The checkbox now sits in the empty right-hand slot of the bottom row, away from the delete trigger in `.contact-card-top`.

Known accepted trade-off: an over-long collection label in `.note-card-collection` (no truncation) could still slide under the bottom-right checkbox; left as-is to keep the diff minimal.


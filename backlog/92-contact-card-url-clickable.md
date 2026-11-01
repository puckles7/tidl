# Contact card URL should be clickable, not open the edit modal

On a Contacts-tab card, clicking a contact's URL currently opens the
contact editor modal (the card's own click-to-edit behavior) instead of
navigating to the URL. The URL text should behave like a normal link.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — introduced the contact card and its primary-URL display.

## Context / Background

`ContactCard` (`crates/tidl-web/src/components/contacts/contact_card.rs`)
already renders the primary URL as a real `<a>` element
(`class="contact-card-url"`, `target="_blank"`, `rel="noopener noreferrer"`),
but the card's own `on:pointerdown` (wired up in
`crates/tidl-web/src/components/contacts/mod.rs` via `DragController` for
click-to-edit / drag-to-reorder) intercepts the pointer event before the
anchor's default navigation can happen, so clicking the URL text opens the
edit modal instead of following the link.

## Functional Requirements

- Clicking a contact card's URL opens the link (in a new tab, matching the
  existing `target="_blank"`) instead of opening the contact editor modal.
- Clicking anywhere else on the card still opens the editor as today.
- Dragging the card by its URL text must still work as a drag (or, if that
  turns out to be awkward given the anchor's native drag behavior, dragging
  from elsewhere on the card is an acceptable fallback) — don't silently
  break reordering.

## Open Questions

- Whether to stop propagation/pointerdown default only on the `<a>` itself
  (simplest, scoped fix) or reconsider how `DragController`'s pick-up
  handler is attached to the whole card — resolve during implementation by
  checking how `contact-card-url`'s `<a>` participates in the existing
  `on:pointerdown=move |ev| on_pick_up.run(ev)` on the card's outer `<div>`.

## Resolution

Took the scoped fix: added `on:pointerdown=|ev| ev.stop_propagation()` directly on the `contact-card-url` `<a>` in `ContactCard`. This stops the pointerdown from ever reaching the card's outer `on:pick_up`/`DragController` handler, so `DragController.drag` never gets set for a press that starts on the URL — the browser's native anchor click-to-navigate then proceeds unobstructed instead of the editor modal opening. As anticipated in the requirements, this makes dragging by the URL text itself a no-op (pointerdown swallowed before reaching the drag controller) — dragging from elsewhere on the card is unaffected and still works normally, which the spec called out as an acceptable fallback.

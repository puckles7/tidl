# Inline note images — hover dim + fullscreen lightbox

Inline images on a note card (#110) render as plain, inert `<img>`s: nothing
signals they can be opened, and there's no way to view one larger. A hover
affordance (dim + expand icon) that opens a fullscreen lightbox makes the
attached image actually inspectable.

**Status:** done.

## Dependencies

- [110 - Note image / attachment support](110-note-image-attachments.md) — the
  inline images this annotates.

## Functional Requirements

- Hovering an inline note image dims it and reveals an "expand" icon.
- Clicking it opens a large, screen-filling modal showing the image.
- The modal closes on backdrop click, its ✕ button, and Escape.

## Resolution

- `render_note_content` (`util.rs`) now wraps each emitted image in a
  `.note-inline-image-wrap` span carrying `data-note-image="<src>"` plus an
  inline-SVG expand badge; CSS dims the image and fades the badge in on hover.
- A root-level `ImageLightbox` component (`components/image_lightbox.rs`) shows
  the image in a viewport-bounded overlay. It reuses `.dialog-backdrop`, so the
  shared Escape-close registry (#123) and backdrop-click-away work for free.
- Shared `ImageLightboxCtx` (a newtype, per the `TypeId`-collision rule) is
  provided in `AuthenticatedApp` and rendered at the app root, so a card's
  transformed drag wrapper can't clip the fixed overlay.
- `NoteCard`'s click delegation opens it, and its `pointerdown` guard stops an
  image press from starting a card drag.

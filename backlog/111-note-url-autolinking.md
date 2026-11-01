# Clickable URLs in notes

**Status: done.**

Paste a URL into a note and it renders as a real link on the card, opening in
a new tab, instead of non-clickable literal text.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — the note card / `render_note_content`
  rendering path this extends.
- [80 - @mention contact linking in notes](80-note-contact-mention.md) — the
  same single-pass, `inner_html` + click-delegation rendering that this now
  shares a pass with.

## Background

Notes render through `util.rs::render_note_content`, which (before this item)
only recognized `@img:<id>` inline-image tokens and `@Name` contact mentions;
every other character was HTML-escaped plain text. Bare URLs therefore stayed
literal. The Notes tab deliberately has no markdown renderer (see #80's scope
clarification), and even the Focus/Tasks/Await markdown preview can't help
here: it uses pulldown-cmark 0.9, which has no GFM/bare-URL autolinking at all.

## Functional Requirements

- Linkify bare `http://`, `https://` (scheme case-insensitive), and `www.`
  URLs in a note's body. `www.` display text stays `www.example.com` but its
  `href` is `https://www.example.com`.
- Anchors open in a new tab (`target="_blank" rel="noopener noreferrer"`),
  matching the Contacts card URL (#92).
- Only trigger at a boundary (start of text, whitespace, or an opening
  bracket/quote) so a URL is never detected mid-word.
- Trim trailing sentence punctuation (`. , ; : ! ? '`), and an unbalanced
  closing bracket only — so `(https://x.com)` drops the wrapper's `)` while
  `.../Foo_(bar)` keeps its own balanced one.
- Consume the whole URL in one step, so a URL's own `@` in a path
  (`https://x.com/@user`) is never mistaken for a contact mention.
- Clicking the link must not start the card's drag, nor open the note editor;
  clicking elsewhere in the note body still opens the editor.

## Open Questions

- `mailto:` and bare dotless schemes (e.g. `localhost:3000`) are deliberately
  not linkified; revisit if they come up.
- Should the markdown preview also autolink? Out of scope here — would mean
  bumping pulldown-cmark to a version with `ENABLE_GFM` or a custom pass.

## Resolution

Implemented entirely in `render_note_content` (single pass, so generated
`<img>`/`<a>`/`<button>` markup is never re-escaped):

- Added `url_span` + `starts_with_ascii_ci` helpers in
  `crates/tidl-web/src/util.rs`; the URL check runs after the `@` token checks
  and emits `<a class="note-link" target="_blank" rel="noopener noreferrer">`.
  Both `href` and display text go through `escape_html`.
- `note_card.rs`'s delegated `on_text_pointerdown` now also stops propagation
  when the target is inside an `<a>`, so the anchor's native navigation wins
  over the card's drag / click-to-edit (same technique as #92's contact URL).
- `.note-link` styling added to `assets/style.css`.
- 12 new unit tests in `util.rs` (`note_link_render_tests`) covering schemes,
  `www.` href rewriting, punctuation/bracket trimming, query strings, escaped
  ampersands, mid-word rejection, and URL/mention coexistence.

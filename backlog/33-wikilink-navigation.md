# Wikilink navigation — Roam/markdown-memo style

Navigation between markdown files via `[[zettel]]` wikilinks, similar to Roam Research and markdown-memo. Clicking a wikilink in the rendered preview should open the linked file (`zettel.md`, with heading `# zettel`).

## Current state

`[[link]]` and `[[[link]]]` already render as `<span class="md-wikilink">link</span>` in `tidl-markdown/src/parser.rs`, but the spans carry no click behaviour. The rendered HTML is injected via `inner_html` in `MarkdownPreviewInner` (`focus.rs`).

## Functional Requirements

- Clicking a `[[zettel]]` span in the Rendered tab opens the file `zettel.md`.
- The link text is the file stem: `[[zettel]]` → look for `zettel.md` (case-insensitive match, same source first, then other sources).
- Links to non-existent files should be visually distinct (e.g. dimmed / dashed underline) and clicking them is a no-op (or shows a "not found" message).

## Open Questions

- **Navigation target**: does clicking a wikilink replace the current dialog content (in-place navigation with a back stack), or open a new overlay on top (stack of cards)? — *not yet decided*.
- **File lookup scope**: search the same markdown source only, or all sources the user has configured?
- **Anchor heading**: should the renderer optionally scroll to the heading matching the link text when the file is opened?
- **Server function**: resolve `[[name]]` → file UID on the server (new `#[server]` fn) or pass `(source_id, name)` to the existing `get_markdown_rendered` with a path-by-name variant?

## Notes

- The rendered HTML is a string injected with `inner_html`, so click delegation needs a JS event listener on the container — the spans cannot have Leptos `on:click` handlers directly.
- `process_wiki_links` in `parser.rs` would need to emit `<a>` or `<button>` elements (or `data-*` attributes) instead of bare `<span>` so the browser has something clickable.

## Resolution

Implemented in-place navigation with a back stack:

- **Parser**: `process_wiki_links` now emits `<button class="md-wikilink" data-wikilink="name">` instead of `<span>`
- **Server function**: `resolve_wikilink(current_uid, name)` walks the same markdown source filesystem for `{name}.md` (case-insensitive stem match)
- **Component**: `MarkdownPreviewInner` uses an internal `nav_stack: RwSignal<Vec<NavEntry>>` for in-place navigation; click delegation on the `.fp-rendered` container captures `data-wikilink` via bubbling; Back button appears when depth > 1
- **CSS**: `.md-wikilink` reset for `<button>` defaults, `cursor: pointer`

**Decisions vs spec:**
- Navigation: in-place with back stack (decided in favour of simplicity over stacked overlays)
- Lookup scope: same source only (user decision — no cross-source fallback)
- Broken links: show inline "file not found" on click (no pre-resolution)
- Anchor heading scroll: not implemented (punted)

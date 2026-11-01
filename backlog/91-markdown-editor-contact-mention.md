# @mention contact linking in the markdown editor

Bring the `@Contact Name` mention syntax from #80 (Notes) to the in-app markdown file editor (`MarkdownFileEditor`, `components/editor.rs`), and render resolved mentions consistently with Notes wherever a markdown-sourced file is displayed as HTML (`MarkdownPreviewInner`, `components/focus.rs`).

## Dependencies

- [79 - Contacts data model + Contacts tab](79-contacts-data-model-tab.md) — the contacts this feature links to.
- [80 - @mention contact linking in notes](80-note-contact-mention.md) — the syntax, storage convention (`resolve_contact_mention`, plain `@Name` text, first-match-wins), and popover UX this extends to a second editor/render surface.

## Context / Background

Requested as a follow-on to #80, once the user noticed the note editor's `@mention` support has no counterpart in the markdown-file side of the app (markdown sources synced from disk, edited/previewed via Focus/Tasks/Await — `markdown_sync` background job, `tidl-markdown` crate).

Two surfaces are in scope, and they are architecturally quite different from #80's note editor and from each other:

- **Editing**: `MarkdownFileEditor` (`components/editor.rs`) is *not* a plain `<textarea>` like `note_editor_modal.rs` — it's a JS-bridged CodeMirror 6 instance (`assets/editor.js`, loaded dynamically, driven through a small `wasm_bindgen` `extern "C"` bridge: `tidl_editor_init`/`tidl_editor_get_content`/`tidl_editor_destroy`). #80's caret-mirror-div technique (measuring an offscreen div to find a `<textarea>`'s caret pixel position) does not apply here at all — CM6 has its own extension/autocomplete API (`@codemirror/autocomplete`) that would need to be wired into `assets/editor.js` and bridged back to Rust for the contact list/selection, a meaningfully different (and likely larger) implementation surface than #80's.
- **Rendering**: markdown files render via `get_markdown_rendered(uid)` (a `#[server]` fn backed by `tidl-markdown`, `ssr`-only per #80's Resolution notes), producing HTML server-side that's injected client-side via `inner_html` (`MarkdownPreviewInner`, `focus.rs`). `[[wikilink]]` already renders in this pipeline as a `data-wikilink`-carrying element resolved by a click handler (`resolve_wikilink`) — `@mention` would need the same treatment (either inside `tidl-markdown`'s own renderer, or as a post-process step alongside it), *not* reuse of `render_note_mentions` (`tidl-web/src/util.rs`), which is Notes-specific plain-text substitution, not part of the markdown-to-HTML pipeline.

"Render it consistently... as in notes" should mean the same resolved-mention *look and click behavior* (styled span, opens `MentionPopover` with the contact's detail and a "Go to contact →" link) — not literally the same code path, since the note and markdown-file rendering pipelines don't share one today.

## Functional Requirements

- Typing `@` in `MarkdownFileEditor` opens a filterable popup of the caller's own contacts (same matching/ordering rules as #80: prefix matches first, then substring, alphabetical, capped list), anchored sensibly near the caret.
- Selecting a contact inserts `@{display_name}` into the document at the caret.
- In the rendered markdown view (`MarkdownPreviewInner`), an `@Name` that resolves to an existing contact (case-insensitive, owner-scoped, first-match-wins — matching #80 exactly) renders as a styled, clickable span.
- Clicking a resolved mention opens the same `MentionPopover` component #80 built (full contact detail + "Go to contact →"), not a new/different popover.
- An unresolved `@Name` (or an `@` inside e.g. an email address) renders as plain text — same false-positive avoidance rules as #80.

## Open Questions

- Where should mention resolution live in the render pipeline — inside `tidl-markdown` itself (which is `ssr`-only in practice per #80's discovery, so this is *not* blocked by the crate's wasm32 claims the way client-side rendering would be), or as a post-process over its output in `tidl-web` (mirroring how wikilinks are currently handled)? Left to implementation.
- CM6 autocomplete integration approach (a custom `autocompletion()` source in `assets/editor.js` vs. a simpler hand-rolled popup driven from Rust) is an implementation decision, not a spec-blocking one — whichever keeps the JS bridge surface smallest.

## Resolution

Implemented largely as designed, with one design correction found during implementation and one deliberate divergence from #80's editing mechanism.

**Editing** uses CM6's native `autocompletion()` extension (`@codemirror/autocomplete@6.20.3`, already vendored as a transitive dependency of the packages `editor.js` already imported — no new dependency needed). This is a deliberate divergence from #80: CM6 has a real `coordsAtPos` API and its own popup/keyboard-nav/text-insertion machinery, so none of #80's mirror-div caret-tracking or manual `MentionState`/keydown handling was ported — only the *filtering* logic is shared (`util::filter_contacts_for_mention`, extracted from `note_editor_modal.rs`'s inline derived signal and now used by both editors). A new `#[wasm_bindgen]` export, `filter_mentions_json` (`components/editor.rs`), is called synchronously from the CM6 completion source per keystroke; contact-list delivery into the CM6 instance is decoupled from CM6 initialization itself (a separate `Effect` pushes whenever either `editor_id` or the contacts fetch resolves, in either order) to avoid a timing bug where a late-resolving contacts fetch would be silently dropped.

**Rendering**: `tidl_markdown::render_mentions_in_text` (the boundary/longest-match/escaping algorithm) now lives in `tidl-markdown`, called by `render_to_html` (signature grew a `contact_names: &[String]` parameter — its one production call site, `get_markdown_rendered`, was updated; ~17 test call sites got a mechanical `, &[]`). Ordering matters: mentions are processed *before* `[[wikilink]]` substitution, not after — running mention-processing second would re-escape the `<button data-wikilink>` HTML wikilinks already emitted (caught by a failing test during implementation, not anticipated in the design). `get_markdown_rendered` fetches the caller's own contact names via a new small `fetch_contact_names_for_mentions(pool, username)` helper (mirroring `contacts.rs`'s `clear_contacts`/`clear_contacts_for` split, since `#[server]` fns can't be unit-tested directly — only their `cfg(ssr)`-only helpers can).

**Design correction**: the design doc originally called for `tidl-web::util::render_note_mentions` (Notes' own mention renderer) to also delegate to `tidl_markdown::render_mentions_in_text`, eliminating that duplication entirely. This does not compile: `tidl-web` only depends on `tidl-markdown` under its `ssr` feature, and `tidl-markdown` doesn't compile to `wasm32` at all regardless (its `watcher`/`sources` modules pull in `notify`/`tokio` crate-wide, per CLAUDE.md §8) — `render_note_mentions` runs client-side and can never link `tidl-markdown`. Confirmed by attempting the change and hitting an unresolved-crate build error. `render_note_mentions` was reverted to its original, separate implementation; only the markdown-preview path shares the new function.

**Shared as designed**: `resolve_contact_mention` and `MentionPopover` (moved from `components/notes/` to `components/mention_popover.rs`, since it's no longer Notes-exclusive) are reused verbatim, unchanged.

Verified end-to-end in a real browser (system Chromium via Playwright, in an isolated worktree instance with a seeded markdown source and contact — the Claude-in-Chrome extension and the Nix-store Playwright Chromium were both unavailable in this sandbox): typing `@Jan` in the in-app editor opens the popup filtered to "Jane Doe"; selecting it inserts `@Jane Doe ` via CM6's native text replacement; saving and viewing the Rendered tab shows a styled, clickable pill; clicking it opens the contact-detail popover with a "Go to contact →" link; both an unresolved `@NoSuchPerson` and a plain `foo@example.com` correctly render as plain text.

`cargo test --workspace` (392 tests in `tidl-web`, plus `tidl-markdown`'s own suite, all passing), `cargo clippy --workspace -- -D warnings` (clean), and `cargo build --target wasm32-unknown-unknown -p tidl-web` (clean, both with and without the `hydrate` feature) all pass.

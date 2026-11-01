# Note bullet lists + convert imported Keep checklists to bullets

Google Keep checklists currently import as inline `#task`/`#done` tokens
(#109 / #113), which is right for genuine to-dos but wrong for lists that are
just lists — a travel packing list, a shopping list, a set of options. Those
come in as tasks and clutter Tasks / Focus / CalDAV. We need (1) real bullet
lists in note bodies, kept to markdown syntax, and (2) an easy way to turn an
imported (or hand-written) checklist's task tokens back into plain bullets.

## Context

- The Keep importer maps **every** `listContent[]` item to a token —
  `build_note_text` in `crates/tidl-web/src/api/keep.rs:221` emits
  `#task/p50/<slug>` / `#done/...` per item — so a non-actionable list becomes a
  pile of tasks.
- `render_note_content` (`crates/tidl-web/src/util.rs:250`) only escapes HTML
  and expands inline tokens / `@mentions` / URLs / `@img`; it has no block-level
  handling, so a literal `- item` line renders as the text `- item`. Line breaks
  come from CSS (`white-space`), not from a markdown pass.
- Markdown **source** files already have a parser (#18, `tidl-markdown` /
  `markdown_tokens.rs`) — check whether its list handling can be reused rather
  than adding a second grammar.

## Functional Requirements

- Render markdown unordered bullet lists (`- `, and ideally `* ` / `+ `) in the
  note card and the note editor preview, nesting by indentation. Stick to
  markdown syntax (`- item`) as requested, not a new token type.
- A list item may still contain inline `#task`/`#await` tokens, `@mentions`,
  URLs, and `@img` — the bullet prefix must not break any of them.
- Give a one-step way to convert a checklist's task/await tokens into bullets:
  either a Keep-import option to import a given list as bullets (per note or by
  heuristic), or an in-note action that turns selected tokens / a whole token
  block into a `- ` list — or both. Decide during design.

## Open Questions

- Syntax: plain `- item` only, or also markdown task-list syntax `- [ ]` /
  `- [x]` so bullets and tasks share one markdown grammar (and possibly align
  notes with the markdown-source parser)?
- Convert at import time (a Keep-import choice or heuristic for "this looks like
  a packing list") vs. as a post-import edit action vs. both?
- Should the done/checked state survive the conversion (a checked Keep item has
  no meaning as a plain bullet — drop it, or keep a `- [x]`)?

## Resolution

Implemented (branch `feat/note-bullet-lists`).

- **Rendering** — `render_note_content` (`crates/tidl-web/src/util.rs`) is now
  block-aware: it splits the body into lines and groups runs of markdown
  unordered-list lines (`- `, `* `, `+ `, leading spaces/tabs) into
  `<ul>`/`<li>` blocks, nesting by indentation (two spaces per level). The old
  inline pass was extracted into a `render_inline(start, end)` closure and is
  run per line, so a bullet item's text still expands inline `#task`/`#await`/
  `#done` chips, `@mentions`, URLs, and `@img` exactly as before. Non-list lines
  keep their literal newlines for the card's `white-space: pre-wrap`. CSS
  (`assets/style.css`) restores list markers (Tailwind preflight strips them)
  and indents nested lists.
- **Conversion** — `note_tokens::checklist_to_bullets` turns every maximal run
  of *token-only* lines (a line whose entire trimmed content, after an optional
  list marker and indentation, is one inline token) into `- <Title>`, preserving
  indentation. Lines with prose around a token are left untouched. It returns
  `None` when nothing converts. The inverse, `note_tokens::bullets_to_checklist`,
  turns every markdown list line whose item text is plain prose into a
  default-priority (`DEFAULT_TASK_PRIORITY` = `p50`) task token — `- Buy Milk`
  becomes `#task/p50/buy-milk` — preserving indentation and skipping bullets that
  already hold a token or slugify to nothing. It likewise returns `None` when
  nothing converts.
- **Action** — the note editor gained a **Bulletize / Taskify** toggle (enabled
  only when one of the two directions applies) that rewrites the draft's text via
  the pure functions; saving then re-syncs tokens (`save_note` deletes and
  rebuilds the note's events), so bulletized tasks/awaits disappear and taskified
  bullets appear. The label names the direction that will run (`Bulletize` when a
  convertible token run exists, otherwise `Taskify`), so clicking it again after
  one pass flips the note back.

Decisions (per the open questions): plain `- ` bullets only (no `- [ ]` task-list
syntax — real tasks stay on the `#task` token grammar); a post-import in-note
action (no import-time toggle/heuristic); checked/done state is dropped, only the
item's text/title survives the conversion. The markdown-source preview already
handled bullets via `tidl-markdown`'s pulldown-cmark pass, so no change was
needed there.


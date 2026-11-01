# Task/await chip UI — subtle dot urgency in notes and markdown

Unify the look of inline task/await chips between **note bodies** (backlog
#113) and **rendered markdown** (backlog #18/#112), and fix the note card's
text layout. Today the two render differently and the note chip's tiered
coloring reads as "disabled" for `LATER` items.

**Status:** done. Full design:
[spec](../../docs/superpowers/specs/2026-10-02-task-await-chip-ui-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-02-task-await-chip-ui.md).

## Dependencies

- [113 - Inline tasks and awaits in notes](113-note-inline-tasks.md) — the
  note-body token chips (`render_note_token_html`) this restyles.
- [112 - Rendered markdown task checkboxes](112-rendered-task-checkbox.md) —
  the rendered markdown badge (`priority_badge_html`) this restyles.
- [68 - Configurable priority curves](68-configurable-priority-curves.md) —
  the number → urgency → color mapping both renders must read from.

## Requirements (from the user)

- Note chips: subtler box/background, **text always black**, urgency shown by
  a **colored round dot**; fix `LATER` reading as greyed-out/disabled.
- Markdown badges: **drop the High/Medium/Low text label**, use a **round**
  dot, colors taken from the **priority-curve** mapping (not the old fixed
  tiers), outline matches the dot; match the note rendering, reusing code.
- Note chips: **single line**, `…`-truncate, full text in a hover tooltip.
- Note cards: make the body **use the full horizontal width**, keeping the
  star/trash icons clear of the text, with the **same right margin as left**;
  justify the text.

## Resolution

- **One shared chip** (`.note-token`) now backs both note-body tokens and
  rendered markdown tags: `[checkbox] ● Title`, a round `.note-token-dot`, a
  1px outline + subtle background derived from a single `--chip-color`, and
  text in the normal readable colour (`var(--text)`) — never tinted by
  urgency. `assets/style.css` reshapes `.note-token` to a single-line
  `inline-flex` row with an ellipsis-truncating `.note-token-label`; the old
  square `.note-badge*` rules (`.note-badge`, `--await`, `--done`,
  `.note-badge-dot`) are removed.
- **`crates/tidl-web/src/util.rs`**: `render_note_token_html` drops the inline
  tier word (`URGENT · 25 / …`) and inline `color:`, emits the dot, and moves
  the full title + priority/date into the token's `title` tooltip. Await
  expiry turns `--chip-color` red; done stays struck-through.
- **`crates/tidl-markdown/src/parser.rs`**: `priority_badge_html` emits the
  same `.note-token` skeleton (no `High/Medium/Low`), and the fixed
  `priority_tier`/`priority_label`/`priority_color` helpers are deleted. New
  `render_to_html_with_urgency(content, contacts, &dyn Fn(u32)->&'static str)`
  takes a colour provider; `render_to_html` keeps the old 2-arg signature and
  uses `default_urgency_color` (a copy of `PriorityCurves::default()`'s
  `source_priority` curve + Focus's 85/65/40 thresholds, KEEP IN SYNC).
- **`crates/tidl-web`**: `get_markdown_rendered` loads the user's curves via
  `fetch_priority_curves` (now `pub(crate)`) and passes
  `note_priority_urgency_color` as the provider, so a markdown chip matches
  Notes/Focus. The preview's line-flash now reads `--chip-color`.
- **Note card layout**: `.note-card-text` is justified; the text block fills
  the width left of `.note-card-actions` (star/trash), body padding is
  symmetric.
- Tests: 43 `tidl-markdown` parser tests green (incl. injected-colour and
  default-palette cases), new note-chip test, 591 workspace tests pass, and
  the WASM/hydrate build compiles.

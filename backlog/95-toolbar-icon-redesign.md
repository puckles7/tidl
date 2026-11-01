# Redesign Notes/Contacts toolbar buttons and unify the trash icon

Restyle the action-button row above the Notes and Contacts grids to match
the mockup at `~/Downloads/tidl-notes.html`: the buttons cluster tightly on
the left with only the search box pushed to the right, and every toolbar
glyph — Auto-layout, Starred, Merge, and every delete/clear-all affordance
in the app — switches from its current mixed Unicode-glyph/emoji look to a
consistent set of outlined SVG icons, with the trash icon in particular
shared by every delete/clear-all button in the app.

## Dependencies

- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md) — introduced `.notes-toolbar` / "Clear all", which Contacts also reuses.
- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — Contacts' toolbar shares the same `.notes-toolbar` markup/class.
- [90 - Clear all buttons for Contacts and Notes](90-contacts-notes-clear-all.md) — added the 🗑 "Clear all" buttons this item restyles.

## Context / Background

Both `NotesView` and `ContactsView`
(`crates/tidl-web/src/components/notes/mod.rs`,
`crates/tidl-web/src/components/contacts/mod.rs`) render their toolbar as
`<div class="notes-toolbar">` — the same shared class and CSS
(`assets/style.css`, `.notes-toolbar`), so a fix here covers both screens
at once, matching how `.notes-pin-filter` (the button style) and
`.notes-search` are already shared.

Two concrete gaps versus the mockup's toolbar section
(`.toolbtn` buttons + the `margin-left: auto` search `<label>`):

- **Alignment.** `.notes-toolbar` is `display: flex; justify-content:
  space-between;` — this spreads every child evenly across the row instead
  of clustering the action buttons together on the left with just the
  search pushed to the far right. The mockup's buttons sit in a tight
  left-hand group; only the search input is pinned right.
- **Icons.** The mockup's toolbar buttons (New note, Auto-layout, Clear
  all, Starred) each use a proper outlined SVG icon. The real app mixes
  Unicode glyphs instead:
  - `+` for New note/New contact (plain character, not the mockup's plus-SVG).
  - `⊞` for Auto-layout (`todo_list.rs`, `notes/mod.rs`, `contacts/mod.rs`).
  - `★` for Starred (`notes/mod.rs`).
  - `🔗` for Merge (`contacts/mod.rs` — Contacts-only, no mockup reference
    for this one specifically; match the other icons' outlined-stroke style).
  - 🗑 for every delete/clear affordance — the one this item originally
    called out specifically:
    - `notes/mod.rs`'s and `contacts/mod.rs`'s "Clear all" buttons
      (`<span class="icon-emoji">"🗑"</span>" Clear all"`).
    - `delete_confirm_button.rs`'s `DeleteTriggerButton` — the per-card
      trash icon used on every Note and Contact card.

  The 🗑 usages already lean on a `.icon-emoji` CSS workaround
  (`assets/style.css`) specifically because 🗑 defaults to
  `Emoji_Presentation`, whose fallback font's taller ascent/descent
  otherwise pushes the button's text line up relative to its non-emoji
  siblings — switching to an SVG icon (as the mockup does) removes the
  need for that workaround entirely, not just the visual mismatch.

## Functional Requirements

- `.notes-toolbar`'s action buttons (New note/New contact, Auto-layout,
  Merge, Clear all, Starred where present) sit in one tight left-aligned
  group; the search input is the only element pushed to the right (e.g.
  `margin-left: auto` on `.notes-search`, matching the mockup).
- Every toolbar glyph switches to an outlined SVG icon, consistent in
  style/weight with the mockup's: New note's `+`, Auto-layout's `⊞`,
  Starred's `★`, and Merge's `🔗` (no direct mockup reference for Merge —
  match the others' stroke style).
- Introduce one shared trash-icon SVG (a small component or an inlined
  `<svg>` snippet reused everywhere) and use it in place of every 🗑 emoji:
  `notes/mod.rs`'s "Clear all", `contacts/mod.rs`'s "Clear all", and
  `DeleteTriggerButton` in `delete_confirm_button.rs`.
- Remove `.icon-emoji`'s vertical-alignment workaround once nothing uses
  the emoji anymore (confirm no other 🗑 usage was missed first).
- No behavior change — same click targets, same confirmation flow, just
  restyled.

## Open Questions

- Exact spacing/gap values between the left-grouped buttons, and the exact
  SVGs for Merge (no mockup reference) — pull/design during implementation
  rather than guessing here.

## Resolution

Implemented in `crates/tidl-web/src/components/icons.rs`: `PlusIcon`,
`GridIcon`, `TrashIcon`, `StarIcon`, `SearchIcon`, and `MergeIcon` — a
shared, consistent outlined-stroke SVG set (`stroke-width="2"`, round
caps/joins). `TrashIcon` is now the single shared trash icon used by
`notes/mod.rs`'s and `contacts/mod.rs`'s "Clear all" buttons and by
`DeleteTriggerButton` in `delete_confirm_button.rs`.

`.notes-toolbar` switched from `justify-content: space-between` to a plain
left-aligned `display: flex` with `gap`, and `.notes-search` picked up
`margin-left: auto` to push only the search box to the right, matching the
mockup. The `.icon-emoji` vertical-alignment workaround was removed from
`assets/style.css` now that no button uses the 🗑 emoji.

Merge borrows Feather/Lucide's `git-merge` glyph (no mockup reference was
given for it) to stay in the same stroke-icon family as the rest.

No deviations from the spec; no behavior change.

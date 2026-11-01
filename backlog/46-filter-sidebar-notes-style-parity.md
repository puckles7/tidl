# Restyle filter sidebar to match Notes tab sidebar

Visually align the left sidebar used on Calendar/Focus/Todos/Tickler
(`FilterSidebar`, `crates/tidl-web/src/components/filter_sidebar.rs`,
`.fs-*` classes in `assets/style.css`) with the Notes tab's collection
sidebar (`.notes-sidebar`), which uses a noticeably different visual
language for what is functionally the same kind of element (a list panel
with an "add new" affordance).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — introduced the `.notes-sidebar` style
  this item aligns the filter sidebar toward.

## Context / Background

Concrete differences observed between `.notes-sidebar` (Notes tab) and
`.filter-sidebar`/`.fs-*` (Calendar, Focus, Todos, Tickler — all share
`FilterSidebar` or its `.fs-*` classes):

- **The "+" add button is different.** `.notes-sidebar-add` is a bordered
  square chip (`border: 1px solid var(--border); border-radius: 6px; width:
  22px; height: 22px;`). `.fs-add-btn` is a bare glyph — no border, no
  background, just a colored `+` character (`font-size: 1rem; padding: 0
  0.15rem;`).
- **Sizing is different.** `.notes-sidebar` is a fixed `290px` with `18px`
  padding and `10px` border-radius, all in px. `.filter-layout` allocates
  the sidebar column at `200px` (`grid-template-columns: 200px 1fr`), and
  `.fs-section` uses `0.75rem` padding / `0.5rem` border-radius — different
  scale and unit system (rem vs. px) from the Notes styles.
- **Positioning/structure is jarringly different.** Notes sidebar is a
  single bordered card containing a header (title + add button) and a flat
  list. The filter sidebar is a stack of independently-collapsible
  `.fs-panel`/`.fs-section` cards (Calendars, Markdown Sources, etc.), each
  with its own header, smaller padding, and a collapse chevron — a
  different structural pattern, not just different spacing values.

Affects every screen that renders `<FilterSidebar/>` or reuses `.fs-*`
classes directly (Calendar's `aside.filter-sidebar.calendar-sidebar` in
`crates/tidl-web/src/components/calendar/mod.rs` included) — fixing the
shared `.fs-*` styles covers all of them at once.

- **Small-screen behavior is missing on Notes, not just styled
  differently.** `.filter-sidebar` has a `@media (max-width: 767px)` block
  (`assets/style.css` ~line 3263) that turns each `.fs-panel` into a
  collapsed-by-default toggle row (`.fs-collapse-btn`) on narrow viewports.
  `.notes-layout`/`.notes-sidebar` have no responsive rules at all — the
  290px sidebar stays side-by-side with the note grid at any viewport width,
  unlike every other screen. This direction is the reverse of the rest of
  this item: the *behavior* should flow from filter sidebar → Notes, even
  though the *visual chrome* is meant to flow from Notes → filter sidebar.

## Functional Requirements

- Restyle `.fs-add-btn` to match `.notes-sidebar-add`'s bordered-chip look
  (or introduce one shared "add" button style used by both).
- Bring `.fs-section`/`.fs-panel` sizing (padding, border-radius, width) in
  line with `.notes-sidebar`'s scale — resolve the rem/px inconsistency
  rather than just copying numbers across.
- Reconcile the structural difference (collapsible multi-panel stack vs.
  single flat card) — decide whether the filter sidebar's per-panel
  collapse behavior (needed since it hosts multiple independent sections:
  Calendars, Markdown Sources, etc.) should be preserved with Notes-style
  chrome, or whether some other resolution is more appropriate.
- Do not regress per-section collapse/expand behavior already present in
  `FilterSidebar` (Calendars panel, Markdown Sources panel).
- Give `.notes-sidebar` the same small-screen collapse behavior the other
  screens already have: collapsed/toggleable on narrow viewports instead of
  permanently occupying 290px next to the note grid.

## Open Questions

- Exact target padding/width/radius values — pick concrete numbers during
  implementation rather than guessing here.
- Whether Notes' single-flat-card structure should be adopted wholesale
  (dropping collapsibility) or whether collapsible sections should just be
  restyled to look like stacked Notes-style cards — resolve before
  implementing, since it changes interaction, not just appearance.

## Resolution

Kept the filter sidebar's collapsible multi-panel structure (dropping it
would have regressed the required per-section collapse/expand behaviour)
and restyled its chrome — plus the calendar page's parallel
`.cal-list-section` styles, which turned out to be an undocumented
duplicate of `.fs-section` — to match `.notes-sidebar`'s visual language:

- Unified on **rem** everywhere this element appears (`.fs-section`,
  `.cal-list-section`, `.notes-sidebar`, the `.filter-layout` sidebar
  column), resolving the px/rem split called out in Context. Chosen
  values: `18rem` width, `0.625rem` radius, `1.125rem` padding — close to
  Notes' original 290px/10px/18px, rounded to a clean rem scale.
- Introduced one shared bordered-chip "add" button style
  (`.fs-add-btn`, `.notes-sidebar-add`, `.cal-list-add-btn` all point at
  the same rule) instead of matching Notes' look with parallel copies.
- Gave `.notes-sidebar` the same mobile collapse behavior as the other
  filter-sidebar panels by reusing the existing `.fs-collapse-btn` /
  `.fs-content` / `.fs-content--collapsed` classes and adding a
  `sidebar_open` signal in `NotesView`, rather than inventing new CSS.

Deviation from the open question: added a *new*, separate
`@media (max-width: 767px)` block at the end of the stylesheet (after the
Notes tab section) for the two notes-sidebar mobile overrides, instead of
extending the shared mobile block near the top of the file. The shared
block sits earlier in the file than `.notes-sidebar`'s base rule (added
later, in the backlog #37 section), so an override placed there was
being silently beaten by the later, unconditional base rule at equal
specificity — caught by rendering the real markup/CSS in a headless
browser at both breakpoints during verification, not by the type checker
or test suite.

# Per-entity color palettes (no cross-type overlap)

Give each colored entity type its **own** palette so a color never means two
things at once. Today the palettes overlap heavily across types, so a blue dot
in the sidebar could be a calendar, a markdown source, a note collection, or a
label — the color carries no type identity.

**Status:** done.

## Background

Current assignment:

| Entity            | Palette                                             | Source                                             |
|-------------------|-----------------------------------------------------|----------------------------------------------------|
| Calendars         | 12 muted tones (`var(--pal-1)`…`var(--pal-12)`)     | `components/cal_edit_modal.rs` `PALETTE`           |
| Markdown sources  | *same* 12 `var(--pal-N)`                            | `cal_edit_modal.rs` `PALETTE`                      |
| Note collections  | 6 curated hex colors                                | `notes/collection_editor_modal.rs` `NOTE_PALETTE`  |
| Contact collections | *same* 6 curated hex colors                       | `contacts/contact_collection_editor_modal.rs`      |
| Note labels       | *same* 6 colors, auto-assigned by name hash         | `api/notes.rs` `resolve_label`                     |

Two overlaps make colors ambiguous: calendars vs markdown sources share the
12-tone set, and note collections vs labels (vs contact collections) share the
6-tone set. There is also an inconsistency in representation: the 12-tone
palette uses CSS variables (theme-aware), the 6-tone palette uses raw hex
(theme-blind).

## Functional Requirements

- Each of the four *semantic* types — **calendar**, **markdown source**,
  **note collection**, **label** — draws from a distinct palette whose hues do
  not collide with the other types' palettes (contact collections can stay
  paired with note collections, or get their own — decide).
- Calendars and markdown sources must stop sharing one palette so a source
  color reads differently from a calendar color.
- Labels must stop sharing the note-collection palette.
- Extend the theme-aware CSS-variable approach (`--pal-N` under `:root` /
  `html.dark`) to every palette, replacing raw hex so all entity colors adapt
  to light/dark consistently.
- Keep existing color-assignment logic (name hash / default index) intact —
  only the palette each type draws from changes.
- Apply consistently everywhere colors render: sidebar pills/dots, calendar
  grid, Focus/Tasks/Await stripes, note cards, label chips, note editor.

## Open Questions

- How to partition the hue wheel: four disjoint hue bands (e.g. each type gets
  a fixed arc), or per-type tuned fixed lists? Bands scale to more entities
  per type; fixed lists are easier to tune for harmony.
- Do contact collections belong to the note-collection palette (both are
  "collections") or need a fifth palette?
- Migration for existing stored colors: leave as-is (like backlog #32 did) or
  remap on read so old values still land in the right type's range?
- How many distinct colors does each type realistically need? (labels and
  collections can grow unbounded via hash assignment, so bands may be better
  than fixed lists for those.)

## Resolution

Implemented as **one well-spread base palette hue-rotated per entity type**,
extending the theme-aware CSS-variable approach from #32
(`assets/style.css`). A first pass used four narrow *disjoint hue bands*, but
that gave each panel six near-identical colours (e.g. all calendars blue);
the palette was reworked on request so each panel has real internal contrast:

| Entity | Variables | Hue rotation |
|---|---|---|
| Calendars | `--pal-cal-1..6` | +0° |
| Markdown sources | `--pal-md-1..6` | +15° |
| Note collections | `--pal-coll-1..6` | +30° |
| Labels | `--pal-label-1..6` | +45° |

The base set is 6 hues 60° apart, so within a panel colours are 60° apart
(strong contrast); rotating by 15° per type keeps every type's hues disjoint
from the others (nearest cross-type pair is 15° apart). Light values use the
#32 recipe (L≈41%, S≈42%); `html.dark` overrides at L≈65%, S≈48%.

- `cal_edit_modal.rs`: the single `PALETTE` split into `CALENDAR_PALETTE` and
  `MARKDOWN_PALETTE`; `MdSourceEditModal`'s stray raw-hex default (`#8b5cf6`)
  fixed to `MARKDOWN_PALETTE[0]`.
- `notes/collection_editor_modal.rs` `NOTE_PALETTE` and
  `contacts/contact_collection_editor_modal.rs` now reference `--pal-coll-N`.
- `notes/note_label_editor_modal.rs`: new `LABEL_PALETTE` (`--pal-label-N`);
  `api/notes.rs::resolve_label` hash-assigns from it instead of `NOTE_PALETTE`.
- Server defaults: markdown-source fallbacks in `api/calendars.rs` and
  `api/files.rs`, plus `types.rs::default_md_color()`, now `var(--pal-md-1)`.
- `tidl-server/remote_sync.rs::pick_unused_color` switched to
  `CALENDAR_PALETTE`.

**Decisions:** contact collections share the note-collection palette
(rotated +30°; both are "collections"), kept as separate constants so they can
diverge later.

**Deviation:** existing stored colours are *not* migrated (matching #32) — the
legacy `--pal-1..12` variables remain defined so old rows still render, but an
old markdown source or calendar may still show a value outside its type's
rotated set until the user re-picks.


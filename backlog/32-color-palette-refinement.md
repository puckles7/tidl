# Color Palette Refinement

The current auto-generated colors for calendars and markdown sources are too saturated and visually jarring, particularly in dark mode. Colors appear harsh in isolation and clash with each other when multiple sources are displayed side-by-side — in the left-hand sidebar, the calendar grid, the Focus view cards, and the Todo/Tickler stripe accents.

## Background

Colors are assigned to calendars and markdown sources on creation (or first display) and used consistently across all views as visual identifiers. The goal is not to remove color differentiation but to make the palette more harmonious and easier on the eyes — lower chroma, muted tones that still read as distinct hues without creating visual noise.

## Functional Requirements

- Reduce saturation/chroma of the palette used for calendar and markdown source colors — aim for muted, desaturated tones rather than fully saturated primaries.
- Ensure colors remain visually distinct from each other when several are shown together (sufficient hue spacing, not just saturation reduction).
- Colors should look good in **dark mode first**, but must also remain legible in light mode.
- Maintain enough contrast between the color swatch/stripe and its background so the color identity is still readable at a glance.
- Apply consistently across all surfaces that use calendar/source colors:
  - Left-hand sidebar calendar list (color dots / swatches)
  - Calendar grid (event chips)
  - Focus view cards (stripe accents and priority indicators)
  - Todo/Tickler view cards (stripe accents)
- Ideally keep the existing color-assignment logic intact; only change the palette the colors are drawn from.

## Open Questions

- Should the palette be a fixed list of pre-tuned hex values, or a generative approach (e.g. fixed hue steps with capped saturation/lightness in HSL)?
- Do existing user-saved calendar colors need a migration path, or is a one-time reset acceptable?
- Should dark mode and light mode use different lightness values for the same hue (i.e. CSS custom properties that flip with the theme), or a single fixed value that works for both?

## Resolution

Implemented the "Sage & Steel" palette — 12 muted, cool-toned colors at ~30° hue spacing.

**Approach chosen:** CSS custom properties (`--pal-1` … `--pal-12`) defined in `:root` (L≈40%, readable on white) and overridden in `html.dark` (L≈65%, readable on dark surfaces). The PALETTE constant in `cal_edit_modal.rs` now holds `var(--pal-N)` references so colors adapt to the active theme automatically without any per-component logic.

**Files changed:**
- `assets/style.css` — added 12 `--pal-N` variables to `:root` and `html.dark`
- `crates/tidl-web/src/components/cal_edit_modal.rs` — PALETTE expanded from 8 raw hex values to 12 CSS variable references
- `crates/tidl-web/src/types.rs` — `default_md_color()` changed from `#8b5cf6` to `var(--pal-10)` (violet)
- `crates/tidl-web/src/components/calendar_pill.rs` — replaced `{c}18` hex-opacity suffix with `color-mix(in srgb, {c} 10%, transparent)` so CSS variables resolve correctly
- `crates/tidl-web/src/components/focus.rs` — same fix for the `{}22`/`{}44` opacity patterns on event icons

**Deviations:** No DB migration — existing stored hex colors continue to render as-is; users re-pick from the new palette at their convenience.

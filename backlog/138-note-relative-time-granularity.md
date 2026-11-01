# Note age — human-friendly units (weeks/months/years)

Note cards show their age in the bottom-right corner as a relative timestamp.
`NoteCard::relative_time` (`crates/tidl-web/src/components/notes/note_card.rs:14`)
currently only goes up to days, so very old notes render as e.g. `3170d ago`,
which reads oddly for anything more than a few weeks old. Large day counts
should roll up into weeks, months, and years.

**Status:** done. Implemented in `relative_time_between` (note_card.rs) with
compact labels: `3w ago`, `2mo ago`, `1y ago`, `8y 8mo ago`. Months/years use
fixed 30/365-day approximations. Only the note card shows an age, so no shared
helper was needed; the pure core is unit-tested.

## Functional Requirements

- Extend `relative_time` (or extract a shared helper) so the label uses
  progressively coarser units:
  - `< 1 min` → `just now`
  - `< 60 min` → `Nm ago`
  - `< 24 h`   → `Nh ago`
  - `< 7 d`    → `Nd ago`
  - `< ~5 weeks` → `Nw ago`
  - `< ~12 months` → `Nmo ago`
  - else → `Ny ago` (or `Ny Nmo ago` if that reads better)
- Keep the existing fallback to the raw date portion when the timestamp can't
  be parsed.
- Keep labels short — they sit in a corner of the card, not a full sentence.

## Open Questions

- Exact thresholds and wording: `3w ago` vs `3 weeks ago`; `2mo ago` vs
  `2 months ago`. Pick whichever fits the card's compact style.
- Whether months should be calendar-based or a fixed 30-day approximation
  (approximation is almost certainly fine here).
- Should the same helper be shared with contacts or any other card type that
  shows an age, so they stay consistent?

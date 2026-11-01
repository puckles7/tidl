# Focus: permanent item suppression

Focus's "suppress for this session" button hides an item from the ranked
list, but only for the current browser session (in-memory client state) —
a reload or a different device loses it. This wish makes suppression
permanent and server-persisted, while keeping suppressed items visible in
a collapsed "Suppressed (N)" section at the bottom of the list so they can
always be unsuppressed. For a recurring event, the user picks at
suppress-time whether to suppress just the current occurrence or the
whole series.

## Context / Background

Full design: `docs/superpowers/specs/2026-07-29-focus-permanent-suppression-design.md`.
Implementation plan: `docs/superpowers/plans/2026-07-29-focus-permanent-suppression.md`.

## Functional Requirements

- Suppression persists in a new `focus_suppressions` table (per-user,
  keyed by item_key), not client-side state.
- Works uniformly across events, tasks, and awaits.
- Recurring events: a small popup lets the user choose "just this
  occurrence" vs. "the whole series" at suppress-time.
- Suppressed items stay visible in the existing collapsed-by-default
  "Suppressed (N)" section — never fully hidden.
- No permanent orphans: a suppression for a since-deleted item is cleaned
  up via reconciliation the next time Focus loads (see design doc for why
  this isn't done eagerly at delete-time).

## Open Questions

None — resolved during brainstorming (see design doc for the rejected
alternatives: client-side localStorage, eager per-deletion-site cleanup,
and reusing Focus's bounded event fetch for reconciliation).

## Resolution

Implemented as designed: new `focus_suppressions` table, `api/focus.rs`
module (list/suppress/unsuppress + self-healing reconciliation), and
`FocusScoredCard` now persists suppression server-side with a
series-vs-occurrence choice dialog for recurring events. One addition not
called out in the design doc: `focus_suppressions.username` had to be
added to `profile.rs`'s `USERNAME_COLUMNS` list so a username rename
cascades to it too — caught immediately by that file's own
`rename_username_column_list_is_exhaustive` safety-net test.

Manual browser verification caught two real bugs neither `cargo test` nor
`cargo clippy` could have found, both fixed before merging:

1. Choosing a suppress option in the recurring-event dialog caused a WASM
   panic ("reactive value ... already disposed"). Calling `suppress_key`
   moves the item out of the active list, disposing the very
   `FocusScoredCard` instance the click handler is running in; the
   subsequent `show_suppress_choice.set(false)` then touched an
   already-torn-down signal. Fixed by closing the dialog *before* calling
   `suppress_key`, not after.
2. The active/suppressed partition check only tested `supp.contains_key(&item.id)`,
   but a single-occurrence suppression is stored under the composite
   `"{uid}|{occurrence_start}"` key — so "just this occurrence" silently
   suppressed nothing. Fixed by also checking the composite key when the
   item is a recurring event occurrence. The "Restore to list" button had
   the mirror-image bug (always unsuppressing by bare id) and was fixed the
   same way — determine which key form actually matched before deleting it.

# Commit auto-layout result as the new manual layout (Notes & Contacts)

Right now "Auto-layout" is purely a reversible preview: turning it on repacks the grid, turning it off restores whatever manual layout existed beforehand (#97). There's no way to say "actually, keep this" — if your manual layout is a mess, the only way to benefit from a clean auto-layout is to leave the toggle permanently on, which then fights any future manual drag (a drag turns the toggle off and restores the old mess, per #97's snapshot/restore behavior). Add a way to take the current auto-layout result and adopt it as the new baseline manual layout, discarding the pre-auto-layout snapshot instead of restoring it.

## Dependencies

- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md)
- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)
- [47 - Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md)
- [97 - Auto-layout should restore manual placement when toggled back off](97-autolayout-restore-manual-on-toggle-off.md)

## Context / Background

- `ContactsView`/`NotesView` (`crates/tidl-web/src/components/contacts/mod.rs`, `crates/tidl-web/src/components/notes/mod.rs`) hold a `pre_auto_layout: RwSignal<Option<Vec<GridPosition>>>` snapshot captured when auto-layout is switched on (#97). Switching it off replays that snapshot and clears it; a manual drag while auto-layout is on also clears it without restoring.
- There is currently no third option: accept the auto-layout result as-is and clear the snapshot without reverting to it. The user has to either keep auto-layout on forever, or drag every card back into the auto-layout positions by hand to "convert" it to manual.

## Functional Requirements

- Add a control (e.g. a button, shown only while auto-layout is on) that commits the current auto-layout positions as the new manual layout: clears `pre_auto_layout` without restoring it, and turns the auto-layout toggle off, leaving the cards exactly where auto-layout put them.
- After committing, the grid behaves exactly as if the user had manually dragged every card to its current position — a future drag or auto-layout toggle should treat this as the baseline, not something to revert.
- Applies to both the Contacts tab and the Notes tab, matching the toggle's existing scope.

## Open Questions

- Placement/label for the control — inline next to the Auto-layout toggle, or a separate confirm affordance? Needs a design pass before implementation.

## Why dropped

Absorbed into [102 - Cohesion-aware grid reflow](102-cohesion-aware-grid-reflow.md), which redesigned the Auto-layout/manual-placement relationship from scratch (Auto-layout became a non-persisting preview rather than a snapshot/restore pair, making `pre_auto_layout` obsolete) and implemented this exact "Keep this layout" commit button as part of that larger design, for both Notes and Contacts.

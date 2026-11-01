# Google Keep import — make shared-note collections optional

The Google Keep (Takeout) importer currently sorts every note shared in Keep
into an on-demand `Shared with <email(s)>` collection. This is helpful when a
user wants that structure, but there's no way to opt out — some users would
rather have everything land in the chosen target collection, keeping their
collection list unchanged.

Add a checkbox to the Keep import section of the Settings → Import tab that
controls whether these shared-note collections are created. It should be
**checked (enabled) by default**, preserving today's behaviour.

## Relevant code

- `import_keep` server fn — `crates/tidl-web/src/api/keep.rs:515`
- `KeepImportSection` import UI — `crates/tidl-web/src/components/settings/import.rs:424`

## Functional Requirements

- New checkbox in `KeepImportSection`, near the existing "Include archived
  notes" option, e.g. "Create separate collections for notes shared in Keep".
- Defaults to checked/enabled (current behaviour).
- When unchecked, shared notes are imported into the target collection (or the
  new-collection name) just like unshared notes — no `Shared with …`
  collections are created.
- Thread a new boolean parameter (e.g. `split_shared`) through `import_keep`
  and gate the per-sharee collection logic on it.
- Update the helper text under the checkbox to describe the new option in both
  states.

## Open Questions

- Should the existing `Shared with …` grouping helper text be rewritten to
  only apply when the box is checked?
- Any interaction with re-imports (a user importing once with the box checked,
  once unchecked) — the current duplicate-skipping should already handle it,
  but worth a test.

## Resolution

Implemented on `feat/keep-shared-collections-optional`.

- Added a **"Create separate collections for notes shared in Keep"** checkbox to
  `KeepImportSection` (checked by default), with state-dependent helper text;
  the `Shared with …` grouping explanation now only shows when it's on.
- Threaded `split_shared: bool` through `import_keep`, extracting the per-note
  collection choice into `resolve_note_collection`, which falls through to the
  target/new collection when `split_shared` is false.
- Re-imports are unaffected: `split_shared` only changes which collection a note
  lands in, and the duplicate check runs before it.
- Test `split_shared_gates_the_shared_collection` covers off → fallback with no
  `Shared with …` created, on → collection created, and unshared → fallback
  either way.

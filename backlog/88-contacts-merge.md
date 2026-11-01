# Merge duplicate contacts

Allow a user to merge two or more contact records into one, so duplicates created by separate manual entry, CardDAV sync, or vCard import can be consolidated without losing data.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — merge operates on the existing contact model and CRUD UI

## Context / Background

Contacts can end up duplicated through several paths already in the app: manual creation, CardDAV two-way sync (#29), and vCard import (#87). There's currently no way to combine two contact records into one.

## Functional Requirements

- User can select two (or more) contacts and choose to merge them.
- User picks which contact is the "primary" (surviving) record, or the system proposes a sensible default (e.g. most fields filled in, or most recently updated).
- Field-level conflicts (e.g. different phone numbers) are surfaced so the user can choose which value(s) to keep — merging should not silently discard data the user might want.
- References to the merged-away contact (e.g. @mentions in notes, see #80) are re-pointed to the surviving contact.
- The merged-away contact is deleted after merge; the operation should be atomic (no partial state if it fails partway).
- Merge is reflected correctly on the next CardDAV sync (no resurrecting the deleted contact from a stale remote copy).

## Open Questions

- Should merge history/undo be supported, or is merge a one-way, confirmed-destructive action?
- How to handle merging contacts that belong to different sharing scopes (relevant once #81 lands)?

## Resolution

Implemented via a new `merge_contacts` server function (`api/contacts.rs`) plus a `ContactMergeModal` reachable from a new "🔗 Merge" toggle in the Contacts toolbar (select 2+ cards, then "Merge (N)").

- Scalar fields (name, org, birthday, etc.) are resolved by the user only where selected contacts actually disagree; the primary defaults to whichever selected contact has the most fields filled in.
- Repeatable fields (emails, phones, URLs, IM handles, addresses, categories) are unioned automatically server-side — nothing is silently discarded, no picker needed.
- `@mention`s in the merging user's own notes are re-pointed from every merged-away contact's old display name to the survivor's final name in the same transaction as the merge, reusing the note-editing code path so a mentioned TASK/AWAIT note's mirrored calendar entry stays in sync too.
- The whole merge (field writes, mention rewrite, merged-away deletion) is one SQL transaction; CardDAV sync-log entries (one "modified", one "deleted" per merged-away contact) are written right after commit, the same non-transactional pattern `create_contact`/`update_contact`/`delete_contact` already use — so the next `sync-collection` REPORT reflects the merge correctly.
- Duplicates are also surfaced automatically (iOS Contacts-style): a `find_possible_duplicates` scan clusters existing contacts whose name matches and whose email/phone either overlaps or is absent on both sides — the same `looks_like_duplicate` rule `import_vcf`'s duplicate check already used, now shared rather than reimplemented, so the two can't drift apart. A dismissible banner on the Contacts tab lists each cluster with "Review" (opens the same merge modal, pre-selected) and "Not duplicates" (session-local dismiss, not persisted).

Deviations from the spec's open questions:
- No merge undo/history — one-way, confirmed-destructive, matching every other destructive action in the app (delete, clear-all).
- Sharing-scope conflicts are unhandled — contacts have no sharing model yet (#81 is still `idea`), so this was out of scope by construction (the ownership check only ever sees contacts the merging user solely owns).

Not in the original spec but added during implementation: automatic duplicate detection (the spec only asked for manual selection). While building it, found and fixed a real data-loss bug in the *existing* import-time duplicate matcher (`find_duplicate_contact`): its "no email/phone → trust the name" fallback only ever checked the incoming vCard's side, so a same-named vCard with real data (e.g. a URL) but no email/phone could be silently treated as a duplicate of an existing contact and discarded on import skip. The fallback now requires *both* sides to be genuinely free of any identifying data, not just email/phone. "Dismissed" duplicate suggestions aren't persisted — they reappear after a reload — accepted as a v1 rough edge rather than adding a new DB table for it.

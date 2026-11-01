# Contacts sharing

Allow a user's address book (#79) to be shared with other tidl users, the way calendars (#13) and note collections (#44) already are.

## Dependencies

- [79 - Contacts data model + Contacts tab](79-contacts-data-model-tab.md) — the address book being shared.

## Context / Background

Explicitly deferred out of #79's initial scope: contacts are private per-user for the first version. Flagged as a placeholder to revisit once the Contacts tab and CardDAV sync (#29) are in use, following the existing share infrastructure/UX patterns (grant/revoke, permission levels) established for calendars and note collections.

## Functional Requirements (not yet scoped)

- Reuse the existing sharing model (see #13, #44) rather than inventing a new one.
- Open question left for when this is picked up: does a grantee get read-only or read-write access to a shared address book, and does CardDAV (#29) need to expose shared address books to the grantee's own clients, or is sharing web-UI-only initially?

## Resolution

Built in two plans.

**Plan 1 — data model + web UI.** Contacts moved from one flat per-user address book to named, colour-coded `contact_collections`, each with an owner and a nullable `contacts.collection_id`; existing contacts were backfilled into a "My Contacts" default collection per user. Sharing reuses the generalized `share_invitations` table (already shared by calendars and note collections) with a new `resource_type = 'contact_collection'`. Unlike calendar sharing's `read`/`read-write` split, contact-collection sharing uses a **single `'member'` tier** — full read-write access for anyone on the collection, no granular permission checks — since contacts don't have the same "someone else's calendar, read-only" use case calendars do. The Contacts tab gained a "Contact collections" sidebar panel (same primitives as the Notes tab's collection sidebar) and Settings → Sharing gained a third section alongside Calendars and Note collections.

**Plan 2 — full CardDAV multi-address-book exposure.** Originally deferred, then pulled into scope at the user's explicit request. Each contact collection — owned or shared — is now its own CardDAV address book. `address_books`/`contact_sync_log` were rekeyed from `owner_username` to `collection_id` (a clean-break migration: tables dropped and recreated, sync tokens restart at 0 — **no data-preserving path was built**, since sync tokens are opaque cursors clients re-baseline from empty). CardDAV routes changed from a fixed `/carddav/{username}/contacts` segment to a `/carddav/{username}/{book}` path parameter, with `book` aliased as `book-{collection_id}` — the same uniform scheme for owned and shared collections (deliberately simpler than CalDAV's two-scheme `shared-{share_id}` vs. real-name split, per backlog #41's precedent). `addressbook-home-set` enumerates every collection a user owns or is a member of.

**Explicit decision: this breaks backward compatibility.** No legacy alias was kept for the old fixed `/carddav/username/contacts` path — existing CardDAV client configurations must re-discover the server (delete and re-add the account) to pick up the new per-collection `book-{id}` paths. The user chose this over preserving a legacy alias when asked directly.

**Bug found and fixed during its own end-to-end test:** the initial `contact_collection_access` resolver returned `Option<String>`, collapsing "no such collection" and "exists but not accessible to this caller" into the same `None` — so `resolve_addressbook_identity` mapped both to 403 Forbidden. Replaced with a three-way `ContactCollectionAccess` enum (`NotFound` / `Forbidden` / `Allowed`) so an unknown `book-{id}` alias correctly 404s instead of 403ing.

Verified with a new `tests/test_carddav_sharing.py` (mirroring `tests/test_caldav_sharing.py`'s shape) covering address-book-home-set enumeration, PUT/GET/DELETE round-trips as owner and as member, non-member rejection, the unknown-book 404 case, and `sync-collection` change reporting — plus the full Rust workspace suite (730 tests), clippy, and the `wasm32-unknown-unknown` build.

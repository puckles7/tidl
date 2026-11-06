# CardDAV interoperability verification — iOS Contacts, DAVx5

Verify tidl's CardDAV implementation (#29) against real clients beyond Thunderbird — at minimum iOS Contacts and Android DAVx5, the two other clients tidl's CalDAV support already advertises compatibility with.

## Dependencies

- [29 - CardDAV protocol — read-write contact sync](29-carddav-protocol.md) — the implementation this verifies.

## Context / Background

#29 shipped CardDAV read-write sync and was verified bidirectionally against Thunderbird (2026-09-15). That verification pass was not a formality: it surfaced and fixed three real bugs that no unit/integration test caught —

1. contacts created via the web UI never bumped the CardDAV sync token, so they were invisible to `sync-collection` clients,
2. the `sync-collection` REPORT response had a malformed extra `<D:response>` entry not matching RFC 6578,
3. a `getctag` property used an XML namespace prefix (`CS:`) that was never declared in the envelope — invalid XML that Thunderbird silently discarded rather than erroring on.

All three were invisible without a real client attempting a real sync — self-consistent protocol-level tests (round-tripping tidl's own PROPFIND/REPORT/PUT/GET/DELETE against itself) didn't and structurally couldn't catch them. There's no reason to assume iOS's or DAVx5's CardDAV stacks are lenient about the same classes of mistake, or that they don't have their own quirks Thunderbird's stack happens not to exercise (see the original CalDAV interop caution in #61, and #31's dropped DAVx5-specific CalDAV verification item).

## Functional Requirements

- Connect iOS Contacts (Settings → Contacts → Accounts → Add Account → Other → CardDAV) to `/carddav/{username}/contacts`; confirm discovery succeeds and existing tidl contacts appear.
- Create, edit, and delete a contact on iOS; confirm it propagates to tidl (web UI and DB).
- Create, edit, and delete a contact in tidl's web UI; confirm it propagates to iOS.
- Repeat both directions against DAVx5 (Android).
- Note and fix any client-specific quirks found, the same way the Thunderbird pass did — this item isn't done until at least one full bidirectional round-trip is confirmed working per client, not just "it connected."

## Open Questions

- Whether macOS Contacts.app (same underlying CardDAV stack as iOS, but worth a separate quick check) belongs in this pass too — include if convenient, not blocking.
- Whether to also re-verify CalDAV against these same clients while set up — subsequently done for DAVx⁵; see #31.

## Resolution

Verified bidirectionally against iOS Contacts and Thunderbird (2026-09-17), and
against DAVx⁵ on the Jolla Phone 2026 (Android AppSupport, 2026-11-06) — all
connected, and a full create/edit/delete round-trip in each direction was
confirmed working. The DAVx⁵ pass also covered CalDAV (see #31), and native
Sailfish OS Contacts on the Jolla Phone 2026 was verified bidirectionally too.
macOS Contacts.app was not checked separately.

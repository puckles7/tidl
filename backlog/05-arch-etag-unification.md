# Unify ETag Algorithm

Two code paths through the system produce ETags via incompatible hash algorithms:

- **CalDAV PUT** → `tidl-caldav::etag::generate_etag` (SHA-256)
- **Web UI create/update** → `fnv64` (inline FNV-1a 64-bit, in `api.rs`)

A CalDAV client that fetches an event created via the web UI and then does a conditional PUT
with the stored ETag will fail the precondition check because the algorithms produce different
values for the same content.

## Fix

Replace the inline `fnv64` function in `api.rs` with calls to `tidl_caldav::etag::generate_etag`.
Both code paths then use SHA-256.

## Resolution

Implemented. `fnv64` removed from `api.rs`; all ETag generation goes through
`tidl_caldav::etag::generate_etag`.

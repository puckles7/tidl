# `move_notes`/`move_contacts` can hit "database is locked" under near-simultaneous writes

Dragging a note (or contact) can occasionally fail server-side with a 500 and `ServerError|error returned from database: (code: 5) database is locked`, even though the DB pool is configured with WAL journal mode and a 30s `busy_timeout` (`tidl-server/src/storage.rs`). The error returns essentially instantly (latency=0ms in the server's own trace log), not after waiting out the busy timeout, which suggests the failure isn't simple lock contention being retried and exhausting the timeout — something about how the two writes overlap is producing an immediate `SQLITE_BUSY` instead.

## Dependencies

None — pre-existing behavior, not scoped to any single feature. Found while verifying [82 - Contacts drag-to-reorder](82-contacts-drag-reorder.md), but confirmed (2026-09-15) to reproduce identically against the `3e6f98c` commit (i.e. before #82's changes) by stashing #82's work and rerunning the same repro — so this is not something #82 introduced, and #82's implementation should not be blocked on it.

## Context / Background

Repro: in the Notes tab, create a few notes in quick succession, then drag one to a new position via a scripted (Playwright, non-human-paced) pointer drag. Two requests to `/api/move_notes` were observed to both 500 with "database is locked" — most likely the drag's own `pointer_up` persist call and the height self-correction `Effect`'s own `move_notes` call (in `components/notes/mod.rs`) landing close enough together to race, since the self-correction effect is itself triggered by the drag's optimistic local update to `pinned_display`/`other_display`.

This likely also applies to Contacts' `move_contacts` (backlog #82 gave it the identical call shape — a drop persist plus a height self-correction pass that can also call the same server fn), though it wasn't separately isolated.

A real human dragging one card at a time is far less likely to hit this than the tight, scripted back-to-back writes used in verification — which may be why it hadn't surfaced before. Still worth fixing, since it means a real (if rarer) failure mode: an occasional drag that silently doesn't persist (client already shows the optimistic new position; the user has no reason to believe the write failed unless they inspect the network tab or reload and see it revert).

## Functional Requirements

- Root-cause why the write fails immediately rather than after the configured 30s `busy_timeout` — worth checking whether `SqliteConnectOptions::busy_timeout` is actually being honored by every connection the pool hands out under concurrent access, or whether sqlx's own pool-level acquisition (rather than SQLite's busy handler) is what's failing fast.
- Either prevent the two writes from overlapping in the first place (e.g., client-side: don't let the height self-correction effect fire a persist while a drop's own persist is still in flight — a simple "operation pending" guard) or make the server-side write resilient to the race (retry on `SQLITE_BUSY`/database-is-locked once or twice before surfacing an error).
- On failure, `move_notes`/`move_contacts` callers already resync via `refresh` — confirm that path actually recovers the correct state (the client's optimistic update must not stick around silently wrong if the server write really did fail).

## Open Questions

- Is this specific to SQLite (would move away with a different DB), or a real transaction-design issue (`apply_positions`'s two-pass "park in a temp position, then move to final" pattern, done as many individual `UPDATE`s inside one transaction) worth simplifying regardless?

## Resolution

Root cause (2026-09-19): `move_notes`/`move_contacts` opened their transaction
with a plain `pool.begin()` — SQLite's default *deferred* `BEGIN`, which only
takes its read-snapshot at the transaction's first read, not at `BEGIN`
itself. Both functions read first (the per-note/per-contact access-check
loop — partly via `&pool` on a *second* connection, not the transaction's
own) and only write afterward, in `apply_positions`. If a different
connection committed a write in that window, the transaction's snapshot was
now stale, and WAL mode returned `SQLITE_BUSY_SNAPSHOT` the moment the first
`UPDATE` ran. Unlike ordinary lock contention, SQLite does not retry
`SQLITE_BUSY_SNAPSHOT` through the busy-handler installed by
`busy_timeout` — retrying would just see the same stale snapshot — so it
fails instantly instead of waiting out the configured 30s, exactly matching
the reported 0ms latency.

Fix: both functions now open their transaction with `BEGIN IMMEDIATE`
(`pool.begin_with("BEGIN IMMEDIATE")`) instead of the default deferred
`BEGIN`. This grabs the write lock up front, before any reads, so two
overlapping move transactions become ordinary lock contention — which
`busy_timeout` *does* retry — rather than a stale-snapshot error that
bypasses it.

Also fixed the third functional requirement while investigating it: the
height self-correction `Effect` in both `components/notes/mod.rs` and
`components/contacts/mod.rs` (the *other* side of the original race) called
`move_notes`/`move_contacts` and silently discarded a failure (`let _ =
...`), unlike the drag-drop and auto-layout call sites in the same files,
which already resync via `refresh` on error. Both now bump `refresh` on
failure too, matching the sibling call sites — so if a write ever does fail
(e.g. genuine multi-second lock contention beyond `busy_timeout`), the
client no longer keeps showing a silently-wrong optimistic position.

Added two regression tests in `crates/tidl-web/src/api/notes.rs` (mechanism
is identical for both files, so not duplicated in `contacts.rs`):
`deferred_begin_fails_instantly_when_a_concurrent_writer_advances_the_wal_snapshot`
pins the root cause (reproduces the exact `(code: 5) database is locked`
error, near-instantly, under a real on-disk WAL pool), and
`immediate_begin_serializes_concurrent_note_moves_instead_of_erroring`
validates the fix (two overlapping `BEGIN IMMEDIATE` movers both succeed
via ordinary lock-wait instead of racing). Confirmed the first test fails
with the pre-fix `pool.begin()` and both pass with the fix.

No deviations from the spec's requirements; the open question (SQLite- vs.
architecture-specific) is answered by the fix — the two-pass parking pattern
in `apply_positions` was never the problem, and doesn't need to change.

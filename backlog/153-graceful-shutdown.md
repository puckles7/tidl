# Graceful shutdown — checkpoint the SQLite WAL on stop

`systemctl stop` (SIGTERM) currently terminates the server immediately: `main`
serves through a bare `axum::serve(...)` with no graceful-shutdown future
(`crates/tidl-server/src/main.rs:863`). SQLite is opened in WAL mode
(`crates/tidl-server/src/storage.rs:674`), so because the process never closes
its connections, the WAL is never checkpointed — `tidl.db-wal` (and `-shm`) are
left behind holding every commit since the last auto-checkpoint.

The database itself is crash-safe (the WAL is replayed on the next open, so no
data is lost), but a stopped server is not self-contained: any tool that copies
`tidl.db` alone — a pre-upgrade backup, most obviously — silently misses those
commits. That is exactly the failure mode that makes a "just copy the DB" backup
unreliable, and it bit a real deploy.

## Functional Requirements

- SIGTERM (systemd `stop` / `restart`) and SIGINT (Ctrl-C) trigger a graceful
  shutdown: stop accepting connections, drain in-flight requests, then close the
  SQLite pool.
- Closing the pool must checkpoint the WAL into `tidl.db` and remove
  `tidl.db-wal` / `-shm`, so a stopped server leaves a self-contained database.
- Long-lived SSE streams (`GET /api/stream`) must be told to end on shutdown.
  Otherwise axum's graceful shutdown waits them out, systemd's `TimeoutStopSec`
  fires SIGKILL, and the checkpoint is skipped — defeating the whole point.
- A new SSE connection request arriving during shutdown is refused rather than
  opened-and-immediately-torn-down.

## Resolution

- `main` now builds a `tokio::sync::watch` shutdown channel and serves with
  `axum::serve(...).with_graceful_shutdown(shutdown)`, where `shutdown` resolves
  on SIGTERM (Unix) or SIGINT, then flips the watch flag.
- `sse_handler` (`/api/stream`) takes the watch receiver as an `Extension`, sells
  out immediately with `503` if shutdown has already begun, and races its
  change-forwarding loop against `shutdown_rx.changed()` so the stream closes
  (and its HTTP connection drains) the moment the signal fires.
- After `axum::serve` returns, `shutdown_pool.close().await` closes the pool. The
  final SQLite connection close checkpoints the WAL into `tidl.db` and removes
  the `-wal`/`-shm` files.
- No schema or migration changes; no requirement on the NixOS/systemd unit
  (SIGTERM is systemd's default stop signal, and the drain is fast).

Verified by `cargo test --workspace` (681 passed) and
`cargo clippy -p tidl-server -- -D warnings` (clean). The end-to-end
`systemctl stop` → no `-wal` check is a manual verification on a real
deployment.

Note: the `.layer(axum::Extension(shutdown_rx))` addition touches the Axum
middleware stack (CLAUDE.md §19 sign-off) — it only adds an inert `Extension`,
alongside the existing ones, and does not reorder any existing middleware.

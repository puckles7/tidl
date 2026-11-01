# Bump rustls past RUSTSEC-2026-0346…0349 (and drop the yanked spin)

`cargo deny check advisories` currently fails on four `rustls 0.23.45`
advisories that are unrelated to any recent dependency work — they appeared
when the RustSec advisory database advanced:

- RUSTSEC-2026-0346 — derived `Debug` impls on `Tls12/13ClientSessionValue`
  can expose TLS secrets (only affects custom `ClientSessionStore` impls that
  log them).
- RUSTSEC-2026-0347 — buffered application data is sent to the outer server
  after an ECH rejection (only with `EchMode::Enable`, GREASE unaffected).
- RUSTSEC-2026-0348 — Brotli certificate decompression can allocate up to
  1 GiB before authentication (only with the non-default `brotli` feature).
- RUSTSEC-2026-0349 — an unauthenticated ServerHello can pin a classical key
  exchange for later connections (loss of post-quantum confidentiality; needs
  resumption enabled).

All four are fixed by the same patch bump: rustls `>= 0.23.46` (i.e.
`cargo update -p rustls`). Surfaced while resolving #142; kept out of that
branch to keep it a single-purpose quick-xml migration.

`cargo deny check` also warns that `spin 0.9.8` is yanked (pulled in through
`flume`/`sqlx` and `rrule`); worth checking whether a newer `spin` is
available when touching the lockfile.

## Functional Requirements

- `cargo update -p rustls` to `>= 0.23.46` (a transitive dep via
  `hyper-rustls`, `lettre`, `reqwest`, `tokio-rustls`), then confirm the
  whole graph still builds and tests pass.
- `cargo deny check` should report `advisories ok` (no `ignore` entries
  needed for these four).
- Resolve the yanked-`spin` warning if a non-yanked version is reachable
  without other churn; otherwise note why it's pinned.

## Open Questions

- None — mechanical patch bump; the only question is whether any of the
  advisories actually bite tidl (in practice most require non-default
  rustls features/settings), but the fix is cheap enough to just take.

## Resolution

- `cargo update -p rustls` bumped `rustls 0.23.45 → 0.23.46`; all four
  advisories are resolved.
- `cargo update -p spin` bumped the yanked `spin 0.9.8 → 0.9.9`; no other
  lockfile churn (4 lines changed).
- `cargo deny check` now reports `advisories ok, bans ok, licenses ok,
  sources ok`.
- `cargo test --workspace` passes (681 passed, 0 failed).

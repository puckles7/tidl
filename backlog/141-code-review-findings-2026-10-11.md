# Code review findings (2026-10-11 full-project pass)

A full-codebase review (all 5 crates, ~90k LOC) surfaced one critical security
gap, a systemic library-crate architecture violation, and a handful of
smaller hardening/tooling gaps. No correctness bugs in day-to-day feature code
were found — the project's own forbidden-pattern discipline (unwrap/expect,
`cfg(ssr)` leakage, bare context-type aliasing, raw-string SQL) holds up well
in practice; the issues below are the exceptions.

## Findings, prioritized

### Critical

1. **Default `server.secret` silently used as an encryption key, with no
   startup warning.** ~~`Config::default_secret()`
   (`crates/tidl-server/src/config.rs`) returns the literal string
   `"change-me-in-production"` if `TIDL_SECRET`/config isn't set.~~ That value
   was fed straight into key derivation (SHA-256 + a fixed label) to produce
   the AES-256 key used to encrypt **SMTP passwords** and **remote CalDAV
   personal access tokens** at rest (`main.rs`, `mailer.rs`, `ssr_types.rs`).
   Since the default string is public (it's in this repo), anyone with read
   access to a deployment's SQLite file could decrypt every stored secret
   unless the operator remembered to set `TIDL_SECRET`.

   **Status: done (2026-10-11).** `server.secret` is now `Option<String>`
   with no default; `Config::require_secret()` returns an error naming how to
   fix it (`tidl-server generate-key` + `TIDL_SECRET`/`TIDL_SECRET_FILE`/
   `[server] secret`), and `main.rs`'s server-start path logs it via
   `tracing::error!` and refuses to start (`cmd_create_admin`/`cmd_seed_demo`/
   `hash-password`/`generate-key` are unaffected — they never touch the
   secret). Also added `TIDL_SECRET_FILE` support (file takes precedence over
   `TIDL_SECRET`, matching the existing SMTP/OIDC pattern) and a NixOS module
   option `services.tidl.secretFile` (required `types.path`, bound read-only
   into the systemd unit, wired to `TIDL_SECRET_FILE` — mirrors
   `encryptionKeyFile`/`clientSecretFile`/`passwordFile`). Updated
   `config.toml.example` and the README's config/NixOS examples accordingly.

   Deployment note: rotating an existing deployment's secret away from the
   default breaks decryption of anything already encrypted under it — a
   remote CalDAV source's stored PAT needs re-entering after the restart that
   picks up a real secret. A declaratively-configured SMTP password
   self-heals (re-upserted from `config.toml`'s plaintext `[smtp]` table on
   every startup); one stored only via Settings → Email does not and would
   also need re-entering.

### High

2. **Library crates return `anyhow::Error` instead of typed errors — violates
   CLAUDE.md §11.** `tidl-caldav`, `tidl-carddav`, and `tidl-markdown` are
   supposed to be pure libraries ("Domain errors: define typed error enums
   (thiserror). Never return `anyhow::Error` from library functions"). In
   practice all three use `anyhow::{anyhow, Result}` throughout:
   `tidl-caldav/src/{caldav,webdav,icalendar_utils}.rs`,
   `tidl-carddav/src/carddav.rs`, `tidl-markdown/src/{source,sources/filesystem,watcher}.rs`.
   This is systemic, not a one-off — every public fallible function in these
   crates needs a `thiserror` enum instead (or the rule in CLAUDE.md §11 needs
   to be relaxed/removed if the project has decided `anyhow` is fine for these
   crates after all).

   **Status: done (2026-10-11).** Added a `thiserror` enum per crate
   (`tidl_caldav::CalDavError`, `tidl_carddav::CardDavError`,
   `tidl_markdown::MarkdownError`) and swapped every `anyhow::Result`/
   `anyhow!(...)` call site in the five files named above for it. Every
   caller outside these crates already treated the error opaquely — via
   `Display` (`tracing`'s `%e`, an outer `anyhow!("...: {e}")` in
   `tidl-server`) or by discarding it (`.map_err(|_| StatusCode::...)`,
   `.unwrap_or_else(|_| ...)`) — so this was a drop-in replacement with no
   call-site behavior change. One wrinkle found along the way:
   `tidl-markdown`'s `MarkdownSource`/`watcher`/`sources::filesystem` (the
   files this finding named for that crate) are dead code from the app's
   perspective — `markdown_sync.rs` reads files directly and never goes
   through that trait — so this part of the fix has no observable effect,
   it's still correct per CLAUDE.md §11.

3. **SSRF surface: webcal feed subscriptions and remote CalDAV sources let any
   authenticated non-admin user make the server fetch an arbitrary URL.**
   `crates/tidl-server/src/webcal_sync.rs` (calendar `sync_url`, any user can
   set one via the calendar settings UI, no admin check) and
   `crates/tidl-server/src/remote_sync.rs` / `caldav_client.rs` (remote CalDAV
   source `remote_base_url`) both build a `reqwest::Client` and fetch a
   user-supplied URL server-side with no validation against
   loopback/link-local/private address ranges. In a multi-user deployment this
   lets any regular user use the server as a probe into its own network (and,
   via sync results/error messages, get some signal back about what's there).
   Fix: resolve and reject loopback/private/link-local targets before fetching
   (or document this as an accepted risk for trusted-user deployments only).

   **Status: done (2026-10-11).** Added `tidl-server/src/ssrf_guard.rs`: a
   `reqwest::dns::Resolve` that resolves hostnames normally but drops any
   resolved address that's loopback/private/link-local/unspecified/
   multicast/broadcast (including IPv4-mapped forms inside an IPv6
   address), installed as both clients' DNS resolver rather than a
   one-time check on the saved URL — so it also covers redirect targets
   and DNS rebinding (a name resolving differently on a later sync).

### Medium

4. **Session cookie is missing the `Secure` attribute.**
   `tidl-web/src/ssr_types.rs::SessionCookieName::set_header_value`/
   `clear_header_value` set `HttpOnly; SameSite=Strict; Path=/` but never
   `Secure`. `SameSite=Strict` covers most CSRF risk, but without `Secure` the
   session token can be sent over a plaintext HTTP connection if a deployment
   is ever reached without TLS enforced in front of it (e.g. a misconfigured
   reverse proxy, or direct LAN access). Should be conditional on whether the
   server is known to be behind TLS (config flag), defaulting to on.

   **Status: done (2026-10-11).** Added `server.secure_cookie` (default
   `true`, `TIDL_SECURE_COOKIE` env override, NixOS
   `services.tidl.secureCookie` option), threaded into `SessionCookieName`
   as a second field.

5. **`cargo deny check` is currently broken.** `deny.toml`'s
   `unmaintained = "warn"` is not a value the installed `cargo-deny 0.19.0`
   accepts (`error[unexpected-value]: expected ["all", "workspace",
   "transitive", "none"]`) — the whole check fails to even run, so dependency
   advisory/license scanning that's documented as available isn't actually
   happening. Needs updating to the current `cargo-deny` config schema.

   **Status: partially done (2026-10-11).** Schema fixed; `unmaintained` is
   now `"all"` (a graph-scope selector in the new schema, not a lint-level).
   Running the check for the first time surfaced real findings: `rustls` got
   a safe patch bump (0.23.41 → 0.23.45, fixes RUSTSEC-2026-0285); `paste`,
   `proc-macro-error2`, `rustls-pemfile`, and `rsa` are unmaintained/
   vulnerable with no safe upgrade upstream (transitively pulled via leptos
   macros / openidconnect / sqlx-core) and are now documented `ignore`
   entries; `0BSD`/`BSL-1.0`/`CDLA-Permissive-2.0` were added to the license
   allow-list as uncontroversially permissive.

   **Update (2026-10-11, later the same day):** `viaspf`/`viaspf-record`
   (`GPL-3.0-or-later`, licenses FAILED) replaced with `mailrs-spf` + its
   `hickory`-feature DNS backend (`Apache-2.0 OR MIT`) in
   `smtp_listener.rs` — see `git log --grep smtp` for the commit. This
   also removed `idna` 0.3.0 from the dependency graph entirely
   (`viaspf` was its only path in), resolving RUSTSEC-2024-0421 as a
   side effect. `licenses ok` now.

   One thing deliberately left failing: `quick-xml` 0.31's DoS
   vulnerabilities (RUSTSEC-2026-0194/0195, fix needs a major version bump
   touching `tidl-caldav`/`tidl-carddav`'s XML parsing — real implementation
   work, not a mechanical config fix). Split out to
   [142 - Upgrade quick-xml past its DoS vulnerabilities](142-quick-xml-dos-vulnerability.md)
   rather than keep this tracking item open for it.

6. **CLAUDE.md §15's "no raw `sqlx::query()` with string SQL" rule is stale
   as written.** ~15 call sites (`api/contacts.rs`, `api/profile.rs`,
   `api/done.rs`, `api/events.rs`, `api/focus.rs`) build the SQL string with
   `format!()` to interpolate either a fixed table/column-name constant or a
   variable-length `?` placeholder list for an `IN (...)` clause — every
   actual value is still bind-parameterized, so this is not a SQL-injection
   bug, and a couple of sites even have a comment explaining why. But the rule
   table doesn't document this as an accepted exception, so it reads as
   "forbidden" when it's actually the established, reviewed pattern for
   dynamic-identifier/variable-arity queries. Worth either carving out the
   exception explicitly in CLAUDE.md or wrapping the pattern in a small helper
   so future dynamic-SQL call sites follow the same shape.

   **Status: done (2026-10-11).** Documented the exception directly in
   CLAUDE.md §15's Forbidden Patterns table (no code changes — the pattern
   itself was already correct, just undocumented).

### Low

7. **`cargo clippy --workspace --all-targets -- -D warnings` fails; the
   project's own documented command (`--workspace` only, §16/§19) does not.**
   Two trivial lints live only in test code:
   `needless_range_loop` at `tidl-web/src/components/calendar/layout.rs:541`
   and `unnecessary_cast` at `tidl-web/src/grid.rs:2556`. Harmless, but any CI
   or agent that runs clippy with `--all-targets` (a common stricter default)
   will see spurious failures.

   **Status: done (2026-10-11).** Fixed both lints in place (iterate the
   slice instead of indexing by range in `layout.rs`; dropped the
   already-redundant `as i32` cast in `grid.rs`).

8. **No CI configuration exists** (no `.github/workflows` or equivalent) — the
   pre-commit checks mandated by CLAUDE.md §16 (`cargo test`, `clippy --
   -D warnings`, `fmt --check`, wasm32 build) are enforced only by convention/
   by whoever runs them locally, not automatically on every push/PR.

   **Status: done (2026-10-11).** This finding was already stale when
   written — `.forgejo/workflows/ci.yml` (this project's host is Forgejo,
   not GitHub, hence no `.github/workflows`) had landed four days before
   the review ran, but only gated on `cargo fmt --check`. Added sibling
   `test`/`clippy`/`wasm` jobs covering the rest of §16/§19's checklist.

## What held up well (no action needed)

- Password hashing (argon2id, with a bcrypt legacy-migration bridge),
  high-entropy random app/inbound-email tokens hashed with SHA-256 at rest,
  and the OIDC Authorization Code + PKCE flow (state, nonce, and PKCE verifier
  all correctly validated, pending rows time-boxed and swept) are all sound.
- `unwrap()`/`expect()` in production code paths is rare and, where present,
  confined to statically-known-valid regex literals — not data-dependent
  panics on user input.
- The `pub type FooCtx = RwSignal<T>` context-aliasing footgun called out in
  CLAUDE.md §15 does not recur anywhere in the current codebase, including in
  the in-progress #140 work.
- Table/export/share endpoints that take a bare token in the URL
  (`export.rs`, `sharing.rs`) correctly re-check session ownership or treat
  the token itself as the sole credential by design, as appropriate per
  endpoint.

## Functional Requirements

- Address finding 1 (critical) and ideally 2–3 (high) before the next
  release; 4–6 are reasonable to batch into the same pass since they touch
  adjacent files. 7–8 are cheap, do whenever.
- Each fix should land as its own commit/PR given how unrelated the areas
  are; this item is a tracking/umbrella entry, not a single patch.

## Status: done (2026-10-11)

All eight findings are addressed, each in its own commit: 1–4 and 6–8
done outright; finding 5's `cargo deny check` schema is fixed, and the
`viaspf` GPL license question it surfaced was resolved by replacing it
with `mailrs-spf` (Apache-2.0 OR MIT), which also incidentally fixed
the `idna` vulnerability. The one remaining `cargo deny` failure —
`quick-xml`'s DoS vulnerabilities, whose fix needs a major version
bump touching `tidl-caldav`/`tidl-carddav`'s XML parsing — is real
implementation work, not a mechanical fix, so it's split out to
[142 - Upgrade quick-xml past its DoS vulnerabilities](142-quick-xml-dos-vulnerability.md)
rather than keeping this tracking item open for it.

Two issues unrelated to any finding here turned up incidentally while
verifying the fixes (`cargo build --target wasm32-unknown-unknown -p
tidl-web` and `cargo fmt --all -- --check`) and were left alone as
out of scope:

- `tidl-web/src/modal_esc.rs` doesn't compile on `wasm32-unknown-unknown`
  (`wasm_bindgen::JsCast` not in scope) — pre-existing, confirmed
  unmodified by any commit in this item's history, so the wasm32 build
  was already broken before this review.
- `cargo fmt --all -- --check` also flags pre-existing drift (unrelated to
  this item) in `tidl-server/src/storage.rs`, `tidl-web/src/app.rs`,
  `tidl-web/src/components/editor.rs`, and `tidl-web/src/note_tokens.rs`.

# Versioning scheme and changelog

tidl has no release or versioning story. The workspace `Cargo.toml` carries a
hard-coded `version = "0.1.0"` that is never bumped, no crate or binary exposes
it at runtime (`tidl-server --version` prints nothing useful), there are no
version tags or releases, and there is no `CHANGELOG.md`. Anyone running a
built binary cannot tell which revision they are on, and there is no
human-readable record of what changed between builds — which matters both for
self-hosters upgrading by hand and for the (planned) prebuilt artifacts in
[143 - Alternative packaging & distribution strategies](143-packaging-distribution-strategy.md).

This item introduces a single, documented versioning scheme: one semantic
version for the whole product (all workspace crates move in lockstep), surfaced
at build time and in the UI, tagged on every release, and accompanied by a
`CHANGELOG.md` kept up to date as work lands.

## Context / Background

- All five crates already inherit `version.workspace = true`, so the root
  `Cargo.toml` is the natural single source of truth — it just needs to be
  bumped and read back at build time.
- The frontend is a separate WASM artifact; the version must be injected into
  the `tidl-web` build too (e.g. via a `build.rs`-provided env var or a
  generated constant), not only the server binary, so the UI can display it.
- Nix (`flake.nix`), the Windows cross-compile, and any future packaging
  channels all need to agree on the same version and artifact hashes (see
  #143's "single source of truth" requirement).

## Functional Requirements

- Adopt **SemVer** for the product as a whole; while pre-1.0, note explicitly
  that a breaking change bumps the *minor* version.
- Keep the version in the root `Cargo.toml` as the single source of truth and
  derive everything else from it (no duplicated version strings).
- Expose the version at runtime: a `--version` flag on `tidl-server` that
  prints the semantic version plus the git SHA and build date, and a small
  "About"/footer readout in the web UI.
- Add a root `CHANGELOG.md` in [Keep a Changelog](https://keepachangelog.com/)
  format (Added / Changed / Deprecated / Removed / Fixed / Security), with an
  `## [Unreleased]` section that accumulating work appends to.
- Tag releases `vX.Y.Z` and publish a GitHub Release whose notes are the
  matching changelog section (so packaging channels, #143, can point at a
  concrete versioned artifact).
- Document the release procedure (bump version → finalise changelog section →
  tag → build/publish → verify) in-repo, and wire version/hash reporting into
  the Nix package metadata.

## Open Questions

- **Manual vs. generated changelog.** Hand-curated entries (accurate, but easy
  to forget) versus a conventional-commits + tool (e.g. `git-cliff`) pipeline
  that generates entries from commit history (automatic, but only as good as
  commit discipline). The project does not currently follow Conventional
  Commits, which argues for a lightweight manual convention at first.
- **Where the running version is displayed / how it is injected into WASM** —
  build script env var, a generated module, or a `#[server]`-supplied string.
- **How releases are cut** — purely by hand, or a CI workflow triggered by a
  tag; and whether pre-1.0 releases get their own `0.x` cadence.
- Whether the version should also be embedded in the SQLite schema / migration
  history to help with upgrade diagnostics.

## Resolution

Implemented on `feat/distribution` (alongside #148/#149/#152, which depend on it).

- **Single source of truth** — the version lives only in the root `Cargo.toml`
  `[workspace.package]`; the Nix flake now reads it back
  (`builtins.fromTOML … ./Cargo.toml`) instead of hardcoding `0.1.0`, so the
  package, image, and tarballs cannot drift.
- **Runtime reporting** — `crates/tidl-server/build.rs` injects `TIDL_GIT_SHA`
  and `TIDL_BUILD_DATE`; `tidl-server --version` prints
  `X.Y.Z (<sha>, <date>)` (`-V` prints the bare version). The Nix package passes
  `self.shortRev` so reproducible builds still report a revision.
- **Web UI** — `crates/tidl-web/build.rs` injects the same values; a footer in
  `app.rs` shows `tidl <version> · <sha>`.
- **CHANGELOG.md** — root file in Keep a Changelog format with an
  `## [Unreleased]` section (initial entries for #147/#148/#149/#152).
- **Release procedure** — [`docs/releasing.md`](../docs/releasing.md): bump
  version → finalise changelog → tag `vX.Y.Z` → push → CI publishes artifacts +
  image + a GitHub Release whose notes are the matching changelog section.
- **CI** — `release.yml` extracts the `## [X.Y.Z]` section from `CHANGELOG.md`
  as the release body; `docker.yml` tags the image from the same tag.

**Decisions on the open questions**

- **Manual changelog** (not generated): the project doesn't follow Conventional
  Commits, so a lightweight hand-curated convention is the starting point.
- **Injection**: a `build.rs` env var in both `tidl-server` and `tidl-web`
  (`env!` at compile time works for native and wasm alike) — not a `#[server]`
  fn.
- **Releases are cut by CI on a `v*` tag** (see `docs/releasing.md`); pre-1.0
  uses a `0.x` cadence with breaking changes bumping the minor version.
- The version is **not** embedded in the SQLite schema/migrations (append-only
  migrations + `--version` are enough for diagnostics).

**Note**: the version bump to `0.2.0` and the changelog `[Unreleased]` →
`[0.2.0]` finalisation are deliberately left to the first real release, per the
procedure.

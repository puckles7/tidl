# Releasing tidl

tidl follows [Semantic Versioning](https://semver.org/) for the product as a
whole: every workspace crate moves in lockstep, and the version lives in exactly
one place — `[workspace.package] version` in the root `Cargo.toml`. Everything
else derives from it:

- `cargo` stamps it into every crate (`version.workspace = true`).
- `tidl-server --version` prints it, plus the git revision and build date.
- The web UI footer shows it (`env!("CARGO_PKG_VERSION")`).
- The Nix flake reads it back (`builtins.fromTOML … ./Cargo.toml`), so the Nix
  package, the Docker image, and the release tarballs all agree.

While tidl is pre-1.0 (`0.x`), a **breaking change bumps the minor version**
(`0.MINOR.0`), not the patch. The changelog is hand-curated in
[Keep a Changelog](https://keepachangelog.com/) format (see
[`CHANGELOG.md`](../CHANGELOG.md)).

## Cutting a release

1. **Green build.** `cargo test --workspace` passes (and `pytest tests/` for a
   release you actually intend to publish).
2. **Finalise the changelog.** In `CHANGELOG.md`, move the contents of
   `## [Unreleased]` into a new section for the version, e.g.:

   ```markdown
   ## [0.2.0] - 2026-11-01

   ### Added
   ...
   ```

   Leave an empty `## [Unreleased]` section at the top.
3. **Bump the version.** Update `version` in the root `Cargo.toml`
   `[workspace.package]` section to match. This is the only version string to
   change.
4. **Commit** the changelog + version bump.
5. **Tag and push.**

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

6. **CI does the rest.** On the `v*` tag:
   - [`.github/workflows/release.yml`](../.github/workflows/release.yml) builds
     the Linux tarballs (`x86_64`, `aarch64`) and the Windows zip, then creates a
     GitHub Release whose notes are the matching `CHANGELOG.md` section.
   - [`.github/workflows/docker.yml`](../.github/workflows/docker.yml) builds the
     multi-arch image and pushes `ghcr.io/<owner>/tidl:<tag>` and `:latest`.
7. **Verify.** Check the GitHub Release has all three artifacts, and that
   `docker pull ghcr.io/<owner>/tidl:<tag>` works on both architectures.

## Notes

- The version is **not** embedded in the SQLite schema or migration history
  (the open question in #147); migrations are append-only and the binary reports
  its own version via `--version`, which is enough for upgrade diagnostics.
- If the source is not a clean git checkout (e.g. a Nix build from a tarball),
  the revision reported by `--version` is `unknown`; the Nix package passes
  `self.shortRev` when it can.

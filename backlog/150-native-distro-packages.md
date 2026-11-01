# Native distro packages (`.deb` / `.rpm` / AUR + systemd)

Ship tidl as native packages for common Linux distributions, each with a systemd
unit. **Dropped**: one packaging recipe + unit per distro (`.deb`, `.rpm`, AUR),
each needing ongoing drift maintenance, for an audience that the release tarball
+ a documented systemd unit largely already covers.

## Dependencies

- [148 - CI build & publish release artifacts](148-ci-release-artifacts.md) — the tarballs are the natural input for a package recipe.
- [147 - Versioning scheme and changelog](147-versioning-and-changelog.md)

## Design

- [Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md)

## Background

The NixOS module shows the shape of a good native deployment (systemd unit,
`WorkingDirectory` at the artifact root, `TIDL_*` env vars, `*_FILE` secrets),
but assumes NixOS. A distro package would replicate that for Debian/Fedora/Arch.

## Functional Requirements

- Produce `.deb` and `.rpm` (and/or an AUR `PKGBUILD`) from the release
  artifacts, with a systemd unit that runs the server with `WorkingDirectory`
  set so the cwd-relative asset paths resolve.
- Package the two-artifact layout (binary + `target/site/pkg` + `assets`).
- Document the data dir, secret, and port configuration.

## Why dropped

High per-distro maintenance burden relative to reach, with no demonstrated
demand. The release tarball, Docker image, and documented systemd unit already
cover the deployment story, so this is not worth carrying.

## Open Questions

- None — dropped.

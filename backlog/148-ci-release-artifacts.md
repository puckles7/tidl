# CI build & publish release artifacts

Build tidl once per release and publish prebuilt, versioned artifacts on GitHub
Releases, so a non-Nix user on a plain Linux or Windows box can download, unpack,
and run tidl with no toolchain. Recommended as the **second** distribution
channel in the #143 spike — the no-container-runtime / Windows fallback — after
the OCI image (#149), whose CI/release pipeline it extends.

## Dependencies

- [147 - Versioning scheme and changelog](147-versioning-and-changelog.md) — release artifacts need a version to be named after and a tag to trigger the release.
- [149 - OCI / Docker image](149-oci-docker-image.md) — delivered second; extends the CI/release pipeline #149 establishes.

## Design

- [Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md)

## Background

`cargo leptos build` produces a coupled two-artifact layout: the `tidl-server`
binary plus `target/site/` (WASM + CSS), which the server serves from
**hardcoded cwd-relative paths** (`target/site/pkg` at `/pkg/`, `assets/` at
`/assets/`, `assets/index.html` as SPA fallback). Windows already packages this
correctly via the `dist-windows` dev command (`dist/tidl-windows-<version>.zip`,
with `run.bat`/`run.sh` that `cd` into the dist dir). There is no Linux
equivalent and no CI to build or publish anything.

## Functional Requirements

- Add a `dist-linux` dev command mirroring `dist-windows`: same fixed layout
  (binary at the artifact root, `target/site/pkg/`, `assets/`,
  `config.toml.example`, `README.md`, a launcher that `cd`s to the artifact
  root), packaged as `tidl-linux-<arch>-<version>.tar.gz`.
- Cover targets `x86_64-linux`, `aarch64-linux`, and `x86_64-windows`.
- Add a CI workflow that, on a version tag, builds all artifacts and attaches
  them to a GitHub Release. Build through the Nix flake (the reproducible path
  already exists) rather than a hand-rolled toolchain install.
- Embed the version from the single source of truth (#147) in every artifact
  name — no channel hardcodes its own version.

## Acceptance Criteria

- A user on a plain Debian/Ubuntu box can download the tarball, unpack it, run
  the launcher, and reach the web UI — no Rust, no Nix.
- The Windows zip and the Linux tarballs are produced by the same CI run and
  share the version tag.
- The launcher/`WORKDIR` guarantee the cwd-relative asset paths resolve.

## Open Questions

- GitHub-hosted runners are amd64-only; build `aarch64-linux` via a Nix
  cross-build, native ARM runners, or QEMU emulation?

## Resolution

Implemented on `feat/distribution` (with #149 and #152).

- **Single layout definition** — the `tidlLinuxDist` derivation in `flake.nix`
  packages `tidl` + `target/site/pkg` + `assets/` + `config.toml.example` +
  `README.md` + a `run.sh` launcher that `cd`s to the artifact root, as
  `tidl-linux-<arch>-<version>.tar.gz`. Exposed as `nix build .#linuxDist`. Both
  the dev command and CI consume it, so the layout is never duplicated.
- **`dist-linux` dev command** — mirrors `dist-windows`; `nix build .#linuxDist`
  then copies the tarball into `dist/`. Added to the shell banner (dashes
  realigned).
- **CI** — [`.github/workflows/release.yml`](../.github/workflows/release.yml):
  on `v*` tags, builds the Linux tarball on native amd64 + arm64 runners
  (`nix build .#linuxDist`) and the Windows zip via a minimal
  `devShells.dist` (`nix develop .#dist -c dist-windows`, no Wine/Playwright
  closure), then attaches all artefacts to a GitHub Release.
- **README** — Quick Start now documents `dist-windows`/`dist-linux` and the
  published release artefacts.

**Deviations / notes**

- **aarch64** is built on GitHub's native arm64 runner (`ubuntu-24.04-arm`),
  not a Nix cross-build or QEMU — the open question above is resolved in favour
  of native runners.
- **Dependency #147 (versioning) is not yet implemented.** The workflow triggers
  on `v*` tags and the artifact version comes from `tidlPackage.version`
  (`Cargo.toml`), which is a provisional scheme; a proper versioning/changelog
  policy remains #147.
- CI is committed but unexercised here (first run on the next `v*` tag). The
  `linuxDist` and `docker` derivations evaluate cleanly under `nix eval`.

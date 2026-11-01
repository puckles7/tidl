# Alternative packaging & distribution strategies

tidl can currently only be obtained by building it yourself (`cargo run` /
`cargo leptos build`) or by running the NixOS module. The NixOS module is
polished and reproducible, but it assumes the operator runs NixOS — which is a
small slice of the self-hosting audience. There is no way for someone on a
plain Debian/Ubuntu/Fedora box, a Synology NAS, a Raspberry Pi, or a desktop to
install tidl as a package, and no container image for people who orchestrate
everything with Docker/Compose/Swarm/Kubernetes.

This item is a **spike**: figure out which distribution channels are worth
supporting, in what order, and what each one would cost us to build and keep
working. It should produce a decision (and possibly follow-up backlog items per
channel) rather than a full implementation in one pass.

## Context

- The build is not a plain `cargo build`: the frontend is a cargo-leptos WASM
  bundle plus the server binary, and `target/site` must ship alongside the
  executable. Any packaging must account for the two-artifact layout and the
  fact that `target/site` is resolved relative to the process cwd.
- SQLite storage (`DATABASE_URL`) and a data dir are the only persistent
  state — that keeps containerisation simple, but migrations run at startup.
- Several runtime-config pieces are env/file driven (`TIDL_SECRET`/
  `TIDL_SECRET_FILE`, SMTP config, declarative config) — a package's
  configuration story (flags vs. env vs. mounted config file) matters as much
  as the artifact itself.
- Windows already cross-compiles (`build-windows`, Wine smoke tests); that
  channel may just need a documented release-artifact story (zip with
  `.exe` + `site/`) rather than new infrastructure.

## Candidate channels to evaluate

- **Docker / OCI image** — single image (`ghcr.io/...`) that bundles the
  binary + `target/site`, exposes the port, and documents volume mounts for
  the data dir and secret. Multi-arch (`amd64`/`arm64`) for NAS / Pi.
  docker-compose example.
- **GitHub Release artifacts** — prebuilt `tidl-server` + `site/` tarballs per
  target (`x86_64-linux`, `aarch64-linux`, `x86_64-windows`) so non-Nix users
  can grab a binary without a toolchain. Possibly the lowest-effort, broadest
  win.
- **Flatpak** — desktop-installable, sandboxed; brings its own challenges
  (a server that needs to bind a port, filesystem access for the data dir,
  background service vs. app).
- **Native distro packages** — `.deb` / `.rpm` / AUR, and a systemd unit.
  Higher maintenance (one per distro), best reserved for when there's actual
  demand.
- **macOS** — Homebrew formula / tap, or a signed `.app`? Lower priority given
  the server focus.
- **Alternative runner stories** — `podman`, rootless, unRAID/TrueNAS
  templates, etc. (mostly downstream of whatever OCI image we publish).

## Functional Requirements

- Benchmark the candidates above on: effort to build, ongoing maintenance
  burden, and reach (how many would-be self-hosters it unlocks).
- Recommend a target set and an order of delivery, and split each accepted
  channel into its own backlog item with concrete acceptance criteria.
- For the top candidate(s), sketch the concrete shape: build/publish pipeline
  (CI), artifact layout, config surface, and a "getting started" doc.
- Ensure whatever we ship keeps a single source of truth for versioning and
  hashes with the existing Nix package so channels don't drift.

## Open Questions

- Is Docker the obvious first step, or do plain release tarballs give more
  reach for less effort?
- Does Flatpak make sense for a long-running server, or is it a poor fit
  (sandbox + port binding + daemon)?
- Do we want an official image (GitHub Container Registry) maintained by the
  project, or just a documented Dockerfile users build themselves?
- How do we handle the `target/site` + binary coupled-artifact problem across
  all channels without duplicating the packaging logic?

## Resolution

Delivered as a **spike decision document** (no code), as the item specified:
[Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md).

**Decision — two accepted channels, in order:**

1. **OCI / Docker image** (official multi-arch `ghcr.io` image, compose example,
   deploy docs) → [#149](149-oci-docker-image.md) — the default quick-start path
   and the highest reach.
2. **GitHub Release artifacts** (prebuilt, versioned tarballs/zip per target,
   built and published by CI) → [#148](148-ci-release-artifacts.md) — the
   no-container-runtime / Windows fallback, extending #149's CI pipeline.

**Deferred** (captured as `idea` items so the analysis is preserved): native
distro packages ([#150](150-native-distro-packages.md)), macOS
([#151](151-macos-distribution.md)), and NAS/alternative-runner templates
([#152](152-nas-runner-templates.md)). Flatpak was judged a poor fit for a
long-running server and folded into the deferred distro-packages item.

All open questions were answered in the spec: **Docker first** (the standard
quick-start path; it hides the coupled-artifact layout behind `WORKDIR`, and its
CI pipeline is reused for the tarballs), then tarballs as the
no-container-runtime / Windows fallback; Flatpak is a poor fit; publish an
**official** image from CI *and* keep an in-repo build recipe; and resolve the
coupled-artifact problem once by having every channel consume the same Nix build
output (single source of truth for version and hashes). #147 (versioning) is a
hard prerequisite for both accepted channels.

No deviations from the spec — the spike's expected deliverable was a decision
plus per-channel follow-ups, which is exactly what shipped.

# OCI / Docker image

Publish an official multi-arch OCI image for tidl on GitHub Container Registry,
with a `docker-compose` example and deployment docs. Recommended as the
**first** distribution channel in the #143 spike: the highest-reach path for
self-hosters (Docker/Compose/Swarm/Kubernetes, plus NAS devices), the default
"quick start", and a natural fit for tidl's SQLite + data-dir state model.

## Dependencies

- [147 - Versioning scheme and changelog](147-versioning-and-changelog.md) — images must be tagged from the single version source.

## Design

- [Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md)

## Decisions

- **Registry: `ghcr.io`.** The source is planned to be hosted on GitHub, so
  GitHub Container Registry is the natural home — free for public images,
  anonymous pulls allowed, and CI pushes with the built-in `GITHUB_TOKEN` (no
  extra account or secret). The image itself is registry-agnostic; a Docker Hub
  mirror can be added later if discoverability there is wanted.

## Background

SQLite (`DATABASE_URL`) in `TIDL_DATA_DIR` is tidl's only persistent state, and
migrations run at startup — so the container story is simple. The one wrinkle is
the coupled two-artifact layout (binary + `target/site/` + `assets/`) served from
cwd-relative paths, which the image must preserve by setting `WORKDIR` to the
artifact root. The Nix flake already produces a package with exactly this layout
(`$out/bin/tidl`, `$out/share/tidl/target/site/pkg`, `$out/share/tidl/assets`).

## Functional Requirements

- Publish `ghcr.io/<org>/tidl` from CI, multi-arch: `linux/amd64` and
  `linux/arm64`.
- Build the image with Nix's `pkgs.dockerTools.buildLayeredImage` (or
  `buildImage`) **from the same `tidlPackage`** the NixOS module uses, so the
  image, release artifacts, and Nix package share one closure and cannot drift.
  `WORKDIR` = artifact root.
- Configuration entirely via `TIDL_*` env vars; document a volume mount for
  `TIDL_DATA_DIR` and a secret mount for `TIDL_SECRET_FILE`, and the exposed
  port.
- Ship a `docker-compose.yml` example and a "Deploy with Docker" README section.

## Acceptance Criteria

- `docker compose up` with a data volume and a secret starts tidl, runs
  migrations, and serves the UI on the mapped port.
- The same image tag runs on amd64 and arm64.
- The image version matches the release tag and the Nix package version.

## Open Questions

- Run as a non-root user in the image, and if so with what uid/gid defaults for
  the data volume?
- Publish a documented `Dockerfile` in-repo in addition to the Nix-built image,
  for users who want to build without Nix? (Spike recommendation: yes.)

## Resolution

Implemented on `feat/distribution` (with #148 and #152).

- **Nix image** — `tidlDockerImage` in `flake.nix`
  (`pkgs.dockerTools.buildLayeredImage`), built from the same `tidlPackage` the
  NixOS module uses. `WorkingDir` = the package's `share/tidl` (so the
  cwd-relative `target/site/pkg` + `assets/` resolve), `TIDL_DATA_DIR=/data`
  with `/data` as a volume, port 3060 exposed, `TIDL_HOST=0.0.0.0`, and the
  binary on `PATH`. Exposed as `nix build .#docker` (a loadable `.tar.gz`).
- **Compose** — [`docker-compose.yml`](../docker-compose.yml): data volume,
  file-based secret (`TIDL_SECRET_FILE`), `restart: unless-stopped`, journald
  logging.
- **Dockerfile** — [`Dockerfile`](../Dockerfile) + `.dockerignore`: multi-stage
  non-Nix build (rust builder → `debian:bookworm-slim` runtime), compiling
  against the committed SQLx offline cache.
- **CI** — [`.github/workflows/docker.yml`](../.github/workflows/docker.yml):
  builds `.#docker` on native amd64 (`ubuntu-latest`) and arm64
  (`ubuntu-24.04-arm`) runners on `v*` tags, pushes per-arch images to
  `ghcr.io/<owner>/tidl`, then creates the multi-arch manifest (`latest` + tag).
- **Docs** — README "Deploy with Docker"; NAS/Podman templates in
  [`deploy/`](../deploy/) (#152).

**Deviations / notes**

- Registry decided as `ghcr.io` (see Decisions). Docker Hub mirroring left as a
  future option.
- The non-root uid/gid question is still open: the image runs as root by default
  for simplicity; a non-root variant is future work.
- CI cannot be exercised from this environment — the workflow file is committed
  but its first real run is on the next `v*` tag. `nix build .#docker` evaluates
  cleanly; a full image build was not run here.
- The Nix `dockerTools` image is only defined for Linux systems
  (`packages` is guarded with `pkgs.stdenv.isLinux`).

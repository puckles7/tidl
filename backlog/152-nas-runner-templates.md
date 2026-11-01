# NAS / alternative-runner templates (unRAID, TrueNAS, podman)

Ready-made deployment templates for NAS and alternative container runners —
unRAID and TrueNAS app templates, plus podman/rootless guidance. **Deferred** by
the #143 spike: this is mostly documentation and is downstream of the OCI image
(#149), so it should be folded into that work where possible.

## Dependencies

- [149 - OCI / Docker image](149-oci-docker-image.md) — the templates wrap the published image.

## Design

- [Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md)

## Background

Once a multi-arch `ghcr.io` image exists, most NAS/runner stories reduce to a
template that points at it and maps the data volume, secret, and port. unRAID and
TrueNAS have their own template formats; podman/rootless mainly needs a short
"How to run without Docker" note.

## Functional Requirements

- unRAID and TrueNAS app templates referencing the published image.
- A podman/rootless run example (quadlet or `podman run`) in the docs.
- Document volume/secret/port mapping consistently with #149.

## Why deferred

Downstream of #149 and largely documentation; no new build infrastructure.

## Open Questions

- Host the templates in-repo or in the respective community app catalogs?

## Resolution

Implemented on `feat/distribution` (downstream of #149). New `deploy/` directory:

- [`deploy/unraid/tidl.xml`](../deploy/unraid/tidl.xml) — unRAID Community
  Applications–style container template (port, `/data` appdata path, secret
  file, `TIDL_DOMAIN`).
- [`deploy/truenas/compose.yaml`](../deploy/truenas/compose.yaml) — TrueNAS
  SCALE "Custom App" compose file.
- [`deploy/podman/tidl.container`](../deploy/podman/tidl.container) — Podman
  Quadlet unit (systemd-native, rootless-friendly, journald logs), plus a plain
  `podman run` example.
- [`deploy/README.md`](../deploy/README.md) — index and per-target install
  instructions.

**Notes**

- All templates wrap the same `ghcr.io` image and config surface from #149.
- Hosting decision: templates live **in-repo** for now; submitting to the unRAID
  CA / TrueNAS catalogs is left as a later, optional step.
- Templates use the `puckles7` GitHub org for the image, consistent with
  the README.

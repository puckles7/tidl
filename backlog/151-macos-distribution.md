# macOS distribution (Homebrew tap / signed app)

Make tidl installable on macOS, via a Homebrew formula/tap or a signed,
notarised `.app`. **Dropped**: the project is server-focused, so a Homebrew tap
or signed app is a lot of ceremony for a small audience.

## Dependencies

- [147 - Versioning scheme and changelog](147-versioning-and-changelog.md)
- [148 - CI build & publish release artifacts](148-ci-release-artifacts.md)

## Design

- [Spec: Alternative packaging & distribution strategies](../docs/superpowers/specs/2026-10-15-packaging-distribution-strategy-design.md)

## Background

macOS is a Unix, so the Linux release artifacts are close to usable — but a
"proper" macOS story means either a Homebrew formula (a service that manages the
data dir and a launchd plist) or a signed + notarised app bundle (an Apple
Developer account and signing infrastructure).

## Functional Requirements

- A Homebrew tap with a formula that installs the binary + `target/site` +
  `assets` and offers a `brew services` launchd unit, **or** a signed `.app`.
- Preserve the coupled-artifact layout (cwd = artifact root).

## Why dropped

Low reach relative to effort for a server-first tool; signing/notarisation adds
account and CI cost, and there is no demonstrated demand.

## Open Questions

- None — dropped.

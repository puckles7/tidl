# Cache the wasm-side dependency build in the crane Nix package too

`flake.nix`'s `packages.default` was migrated to crane so that source-only
commits (the common case — CSS/JS/single-component edits, no `Cargo.lock`
change) don't force a full ~600-crate recompile on every `nixos-rebuild`.
That migration only caches the *native* (`tidl-server`, `ssr` feature)
dependency tree — the wasm/frontend (`tidl-web`, `hydrate`/`client-debug`
feature) side still recompiles its own ~200-crate dependency set from
scratch every build.

## Context / Background

`cargo leptos build --release` runs two separate cargo invocations
internally, with different target/profile/features/target-dir:

- Native: default target, default `target-dir`, `--features ssr`, `release`
  profile.
- Wasm: `--target wasm32-unknown-unknown`, `--target-dir=target/front`,
  `--features client-debug`, the custom `wasm-release` profile (lto=true,
  codegen-units=1).

`craneLib.buildDepsOnly` bakes in a single cargo invocation per call. The
current `tidlCommonArgs.cargoExtraArgs` (`--package tidl-server
--no-default-features --features ssr`) matches only the native side, which
is why that half is now cached and the wasm half isn't.

Caching the wasm side too needs a *second* `buildDepsOnly` call configured
to mirror cargo-leptos's wasm invocation exactly (`CARGO_BUILD_TARGET`,
`CARGO_TARGET_DIR=target/front`, matching features/profile) — and crane's
`buildPackage` only accepts one `cargoArtifacts`, so there's no built-in way
to feed it two separate cached dependency sets. Merging both into one build
sandbox before the real build phase runs would need custom derivation glue
crane doesn't provide out of the box.

Deprioritized for now: the wasm side is the cheaper half already (~200
crates, ~2.5 minutes even fully cold), so the effort-to-reward ratio is
worse than what the native-side migration already delivered (verified:
209 crates / 272s for a source-only rebuild, vs. 609 crates recompiled
from scratch before the migration).

## Functional Requirements

- A second `craneLib.buildDepsOnly` derivation whose cargo invocation
  (target, target-dir, profile, features) matches cargo-leptos's own wasm
  build exactly, so the pre-built dependency artifacts are actually found
  and reused rather than silently recompiled (the same
  fingerprint-matching pitfall hit twice during the native-side migration
  — verify empirically with a source-only rebuild + crate-count diff, not
  just by evaluating the derivation).
- A way to make both cached dependency sets (native + wasm) available in
  the same `buildPackage` build sandbox before `cargo leptos build
  --release` runs.
- Re-verify the full source-only-rebuild timing after this lands, and
  update the comment on `tidlCargoArtifacts` in `flake.nix` (currently
  documents this as a known limitation).

## Open Questions

- Does crane (or a newer version of it) have first-class support for
  multi-target dependency caching in a single workspace build, avoiding
  the need for custom merge glue? Worth checking upstream crane docs/issues
  before hand-rolling this.

## Why dropped

Parked, not abandoned: the wasm side's ~2.5 minute cold-build cost is
tolerable for now and the crane multi-cargoArtifacts glue needed to fix it
is nontrivial (see above). Revisit if wasm-side rebuild time on
source-only commits becomes actually painful in practice.

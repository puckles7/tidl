# Editors tab and local/remote open flow — implementation plan

**Design:** [2026-11-06-editors-tab-design.md](../specs/2026-11-06-editors-tab-design.md) ·
**Backlog:** [#164](../../../backlog/164-open-in-editor-local-remote-consolidation.md)

> `cargo test --workspace` after each phase; build WASM after `tidl-web` changes
> (`cargo build --target wasm32-unknown-unknown -p tidl-web --no-default-features
> --features hydrate`).

## Phase 1 — shared model (`local_editor.rs`)

- [x] Add `server_cmd` to `EditorPreset` + `server_cmd_for`; derive a default
      CLI from the editor choice, with `resolve_server_cmd` honoring a
      per-browser override.
- [x] Canonicalize clone keys to `host/owner/repo`, reconciling SSH/SCP/HTTPS;
      re-key legacy `localStorage` entries on load.
- [x] Add `is_absolute_path` for UI validation.
- [x] Collapse `OpenTarget` to `LocalClone` / `HostCopy` / `ConfigureClone` /
      `Unavailable`; git ignores location and never falls back to tidl's clone.
- [x] Unit tests for keys, paths, presets, and all resolver states.

## Phase 2 — server fn (`api/settings.rs`)

- [x] `open_in_editor(uid, editor_id, server_cmd)` prefers a non-empty
      per-browser override, then the legacy stored command, then the preset's
      command.
- [x] Custom editors / empty overrides / missing launcher binaries fall back to
      the OS default opener.

## Phase 3 — Settings → Editors (`components/settings/`)

- [x] Rename the tab and module from `This Device` / `this_device` to
      `Editors` / `editors`.
- [x] One editor picker; URI template, command override, and location override
      under `Advanced`; info badges for the long explanations.
- [x] Local clone rows: absolute-path warning and a `Test` link.
- [x] Remove `Server editor command` from General.

## Phase 4 — preview (`components/focus.rs`)

- [x] New target labels and a disabled `Open on tidl host` with a hover hint.
- [x] A mapped clone with no usable link template shows a disabled
      `Open in editor` plus `Configure editor…` instead of misreporting a
      missing clone.
- [x] Inline, source-scoped `Set up clone…` prompt.
- [x] Remove server-clone pull/commit UI and dead code.
- [x] Send `editor_id` to the server fn for filesystem sources.

## Phase 5 — docs + verification

- [x] Backlog #164 / BACKLOG.md, CHANGELOG, README.
- [x] `cargo test --workspace`.
- [x] WASM hydrate build.
- [ ] `rat` e2e — run from an isolated worktree if the main checkout's dev
      server owns the test ports.

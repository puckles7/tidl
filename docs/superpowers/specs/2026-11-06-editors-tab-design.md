# Editors tab: one editor, two kinds of source — design

Backlog: [#164](../../../backlog/164-open-in-editor-local-remote-consolidation.md)

## Problem

"Open in editor" was configured on two surfaces with two vocabularies: a
server-wide `Server editor command` in General and a per-browser `Local editor`
(link template, clone roots, and a browser-location override) under
`This Device`. The UI never said that the two settings run on different
machines, so they read as duplicates. The runtime leaked four `OpenTarget`
states into the preview footer, and git sources could be opened from tidl's
managed clone — a copy users should never edit by hand. "Set up local editor"
also dead-ended into a generic tab instead of asking for the one thing it
needed: a clone path for the source in front of the user.

## Decision

Git and filesystem sources have different rules, and only those rules are
exposed:

| source type | browser on the tidl host | browser elsewhere |
|---|---|---|
| **git** | your own clone → open on this device | your own clone → open on this device |
| **filesystem** | open on the tidl host | external open disabled; edit in browser |
| any | edit tidl's copy in the browser | edit tidl's copy in the browser |

Consequences:

- **Git never opens tidl's managed clone.** Colocation is irrelevant. The
  user must map a clone once; after that the path is identical everywhere.
  The old "Commit & push server clone" / "Pull server clone" recovery UI is
  removed because nothing external dirties tidl's clone any more.
- **Filesystem sources only open externally when the browser is on the tidl
  host.** Otherwise the action is disabled with a hover explanation and the
  in-app editor remains available.
- **One editor identity.** Each preset supplies a browser URI template and a
  default server CLI command. The user picks "VS Code" once. The CLI command is
  only a default — launcher names vary across Windows, Linux distributions,
  Flatpak/Snap, and GUI-only installs — so Advanced exposes an optional
  "Command on the tidl host" override. Custom editors provide a URI template
  and fall back to the host's default opener when no override is set.

## Resolver

`local_editor::resolve_target` collapses to three outcomes plus one
configuration state:

- `LocalClone { abs_path }` — git source with a usable clone mapping.
- `HostCopy` — filesystem source, browser on the tidl host.
- `ConfigureClone` — git source without a mapping (or without a usable editor).
- `Unavailable` — filesystem source, browser remote.

`on_server` only influences the filesystem branch. The loopback heuristic stays
(auto-detect), with a manual override kept under Advanced.

## Settings → Editors

- **Editor** — one preset picker. The URL template is only editable under an
  `Advanced` disclosure; editing it selects `Custom`.
- **Local clones** — always shown for git sources, since git always uses the
  user's clone. Rows accept an absolute path, warn on relative paths, and offer
  a `Test` link that opens the clone root with the chosen editor.
- **Advanced** — editor link template, browser-location override, and an
  optional "Command on the tidl host" override. The preset's command appears as
  the placeholder; leaving the field empty keeps the preset/default behaviour.

All of this still lives in `localStorage`, untouched by account sync.

### Resolving the tidl-host command

For a filesystem source, the command is chosen in this order:

1. the browser's explicit Advanced override;
2. the legacy per-user `user_preferences.open_in_editor_cmd`, so existing
   installs keep their configured launcher;
3. the selected preset's built-in command;
4. the host's default file opener.

If the chosen launcher binary does not exist (common with `code`, Flatpak/Snap
wrappers, or GUI-only installs), the server falls back to the host's default
opener once instead of returning a bare `ENOENT`.

## Preview flow

The preview shows one concise target line and one primary action:

- local clone: `This device · /path` → `Open in VS Code →`
- local clone with no usable link template: `This device · /path` →
  disabled `Open in editor` + `Configure editor…`; `Copy path` and
  `Edit in browser` remain available
- filesystem on host: `On the tidl host` → `Open on tidl host →`
- git without a mapping: `No local clone on this device` → `Set up clone…`
- filesystem remote: `On the tidl host (not reachable from this browser)` →
  disabled `Open on tidl host` with a hover hint

The link template is only used for the first two cases. Files on the tidl host
use the command; git clones use the URL scheme. A browser cannot execute an
arbitrary local command, so an editor without a registered URL scheme cannot be
launched for a local clone — `Edit in browser` and `Copy path` are the
fallbacks.

`Set up clone…` expands an inline prompt scoped to that source
("Where is your clone of «source»?"), saves the mapping, and lets the primary
action become `Open in <editor>` without leaving the preview. `Edit in browser`
is always available.

## Clone-key reconciliation and validation

SSH and HTTPS spellings of the same remote canonicalize to one key
(`host/owner/repo`), stripping scheme, user info, port, leading/trailing
slashes, and `.git`. Existing `localStorage` keys are re-canonicalized on load,
so moving between transports does not require re-entering paths.

The browser cannot stat the user's filesystem, so validation is limited to
syntactic checks (non-empty, absolute path) plus the `Test` link. "Path exists /
is the expected repository" is explicitly out of reach for a client-only path.

## Testing

- Unit tests cover canonical keys (SSH, SCP-like, HTTPS, ports), absolute-path
  detection, derived server commands, and every resolver outcome.
- `cargo test --workspace` and the WASM hydrate build are required after the
  change.

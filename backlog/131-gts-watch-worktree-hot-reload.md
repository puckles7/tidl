# gts cargo leptos watch hot-reload broken in a worktree

`gts <port>` (`flake.nix`) runs `cargo leptos build` then `cargo leptos watch`
for hot-reload. When run from a linked worktree
(`<repo>/.worktrees/<slug>`), source edits stop being picked up on reload —
Ctrl-F5 keeps serving the old build.

**Status:** done.

## Symptoms

- Editing a watched source file does not trigger a rebuild: the served
  `target/site/pkg/tidl.wasm` keeps its old mtime indefinitely (observed >150s
  with no rebuild).
- The `tidl_bg.wasm` symlink (a workaround for cargo-leptos 0.3 renaming the
  wasm output, so `tidl.js` still points at `tidl_bg.wasm`) can be missing, so
  `/pkg/tidl_bg.wasm` 404s and the browser keeps the previously-loaded wasm.
- A symlink-keeper background loop added to the gts script did **not** fix it,
  so the symlink is not (the only) cause — the watcher itself stops reacting.

## Suspicions

- Worktree-specific: `cargo leptos watch` may resolve/watch paths relative to
  the main checkout's `.git` (worktrees share a git-common-dir), confusing its
  file watcher or reload websocket.
- An inotify watch limit interacting with the worktree layout / a large
  `target/`.
- The reload websocket port (`LEPTOS_RELOAD_PORT`) colliding with another
  running `cargo leptos watch`.

## To investigate

- Reproduce in a fresh worktree with no other cargo/watch process running.
- Compare `cargo leptos watch` behaviour in the main checkout vs a worktree.
- Inspect cargo-leptos git integration / reload-websocket logs; determine
  whether the watcher dies or the symlink is wiped (or both).
- Decide whether the fix belongs in `gts` (`flake.nix`) or is an upstream
  cargo-leptos bug.

## Resolution

Two independent bugs, both fixed.

**1. The watcher registered zero inotify watches in a worktree (the real
"no rebuild" cause).** cargo-leptos 0.3.2 (`src/service/notify.rs`,
`GitAwareWatcher::ignore_paths`) tests *every ancestor directory* of a candidate
path against `.gitignore` — including ancestors **above** the `.gitignore`
root. `.gitignore` had an **unanchored** `.worktrees/` rule, and a linked
worktree lives at `<repo>/.worktrees/<slug>`, so the ancestor `<repo>/.worktrees`
matched; every candidate path was therefore filtered out and the watcher
registered **0** inotify watches (`/proc/<pid>/fdinfo` confirmed zero, vs a
control instance which showed watches). With no watches, no filesystem events
ever reached the rebuild loop.

Fix: **anchor** the rule as `/.worktrees/` in `.gitignore`. Git still ignores
the directory, but the `ignore` crate no longer matches the ancestor above the
root — verified by toggling the rule: 0 watches → 44 watches, and a source edit
then triggered a rebuild that updated `target/site/pkg/tidl.wasm`.

This is arguably an upstream cargo-leptos bug (matching ancestors above the
gitignore root is meaningless); anchoring is the repo-side fix that keeps the
worktree layout.

**2. The `tidl_bg.wasm` symlink `gts` created was corrupting the wasm.** The
`tidl.js` glue's default wasm URL is `tidl_bg.wasm`, but `assets/index.html`
loads it by absolute path — `init({ module_or_path: '/pkg/tidl.wasm' })` — so
the alias was never needed. Worse, cargo-leptos renames `tidl_bg.wasm` onto
`tidl.wasm` on every rebuild; with `tidl_bg.wasm` a symlink → `tidl.wasm`, that
rename made `tidl.wasm` a **self-referential symlink** (ELOOP), so
`/pkg/tidl.wasm` returned 500 and the app broke after the first hot-reload.

Fix: removed the symlink block from `gts` **and** `run-tidl` (`flake.nix`),
with a comment explaining why no alias is needed.

Verified in a fresh worktree: `cargo leptos watch` registers watches, a source
edit rebuilds, `target/site/pkg/tidl.wasm` stays a regular file and is served
with HTTP 200 and a valid `\0asm` header. Fixes #131.

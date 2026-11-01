# AGENTS.md

opencode-facing mirror of this project's Claude Code memory
(`~/.claude/projects/<project-slug>/memory/`, where `<project-slug>` is the
checkout path with `/` replaced by `-`). Read the full index at
`~/.claude/projects/<project-slug>/memory/MEMORY.md` when a task touches
architecture, work-calendar/auth, or anything below is ambiguous.

## Shorthands

Treat these as literal commands when the user types them (standalone or at the
end of a request):

| Token          | Meaning                                                                             |
|----------------|-------------------------------------------------------------------------------------|
| `cap`          | Commit and push the current task's changes. Only when explicit — never auto-commit. |
| `capauc`/`cpc` | Commit and push **all** outstanding changes in the working tree.                    |
| `rat`          | Run all tests: `cargo test --workspace` then `pytest tests/`.                       |
| `lob`          | List open backlog items (status `idea`, `planned`, or `in progress`).               |
| `mcc`          | Merge worktree branch to `main` locally, clean up worktree+branch, then push (`cap`). Takes the repo-wide merge lock first (see below). |
| `gts`          | Generate a test server (project `gts` command: server + synthetic data). May not be merged yet — check `flake.nix` / `which gts` first, else fall back to the manual pattern below. |
| `cwi <NN>`       | Create a worktree for backlog item NN: `.worktrees/<slug>` on branch `feat/<slug>` off `main`, `direnv allow`, mark it `in progress`, then **implement the item without pausing** — stop only for open design choices that must be decided first. |

There is no `mmc` shorthand. If the user types `mmc`, the correct term is
`mcc` — correct them.

### `mcc` merge lock

`mcc` takes a repo-wide merge lock before touching `main` or any worktree, so
two concurrent `mcc` runs can never interleave their merges/pushes. The flag
lives at `<git-common-dir>/mcc.lock.d`, so the main checkout and **every**
linked worktree see the same lock; it is never tracked. Use the helper:

1. **Acquire** — `scripts/mcc-lock.sh acquire`. It atomically claims the flag
   with `mkdir(2)`. If a merge is already running it prints the holder and
   polls every 15s until the flag is free, then claims it; it returns only once
   *this* run owns the flag.
2. Do the `mcc` work (merge → test → remove worktree/branch → push).
3. **Release** — `scripts/mcc-lock.sh release`, even if the merge fails.

`scripts/mcc-lock.sh status` shows who holds the flag; `--force` steals a flag
left behind by a crashed run (check the holder first — the script warns once it
is older than an hour).

**The flag is the only coordination.** `mcc` must not actively inspect other
sessions or their in-progress git merges — don't probe holder PIDs/liveness,
don't read conflict/`MERGE_HEAD` state left in `main`, don't resolve or "rescue"
another worktree's merge, and don't ask the user to adjudicate a foreign merge.
If the flag is held, poll and wait; if the flag never clears, you wait forever.
(`--force` remains a deliberate, manual last resort, not something an `mcc` run
does on its own.)

**Why `mkdir`:** the flag is a directory claimed with atomic `mkdir`, so when
several waiters wake at the instant a holder releases, exactly one succeeds and
the rest keep polling. Never `test -e` then create — that check-then-act is
racy (TOCTOU) and lets two runs claim the flag at once. `release` also renames
the flag dir aside before deleting it, freeing the lock path in one atomic step
so a mid-delete window can't hand the flag to two waiters. A `flock(2)` on a
lock file would be an equally valid primitive (kernel-queued, auto-released on
crash), but the visible, cross-process flag is easier to reason about and
survives across separate `mcc`-driven shell steps.

## Project rules

- **No subagents/forks at all** — not for execution, and not for research or
  exploration. Do everything inline with your own Grep/Read/Bash calls.
- **Run `cargo test` after every code change**, not just when closing a backlog
  item. (`rat` additionally runs the Python e2e suite.)
- **Run the full `rat` suite after every major feature merge** — a redesign or
  cross-cutting UI change can leave Python e2e selectors stale even when
  `cargo test` is green (the Focus redesign, #115, did exactly this). The e2e
  suite needs a hash-free server+site build in an **isolated worktree**
  (`<repo>/.worktrees/…`): `DISABLE_SERVER_FN_HASH=1 cargo leptos build`, then
  run `pytest` from that worktree root. Free the test ports first (3060 default;
  3001/3002 sync; 3061/3062/3070 notes/cards) or the session fixture silently
  hits a stale server.
- **Update `README.md`** whenever a significant feature is introduced or changed
  — don't wait to be asked.
- **Maintain `CHANGELOG.md`** — add every user-visible change to `## [Unreleased]`
  (Keep a Changelog; Added/Changed/Deprecated/Removed/Fixed/Security) in the same
  session it lands, citing the backlog item (`(#NN)`). Skip refactors/deps/tests.
  Never rewrite released sections. Versioning scheme is [backlog #147](backlog/147-versioning-and-changelog.md);
  full procedure in [`docs/releasing.md`](docs/releasing.md).
- **Specs**: full design → `docs/superpowers/specs/`, full plan →
  `docs/superpowers/plans/`; `backlog/NN-*.md` stays a thin index that links to
  both. (Supersedes any older "backlog is the spec" note.)
- **New backlog number**: before assigning one, check the main checkout *and*
  every worktree (`git worktree list`, plus their uncommitted `backlog/`) for
  collisions — a fresh worktree doesn't see uncommitted numbers elsewhere.
- **Worktree path desync**: when working inside a worktree, confirm every
  Edit/Write path starts with the worktree root, not the main repo. Before
  finishing a branch, run `git status --short` in the **main** repo too.
- **`flake.nix` shellHook echo**: after adding/renaming a command, realign all
  dashes to the longest command string.
- **`#[cfg(target_arch = "wasm32")]` / `#[cfg(feature = "hydrate")]` changes**:
  also build the WASM target, e.g.
  `cargo build --target wasm32-unknown-unknown -p tidl-web --no-default-features --features hydrate`.
- **Local skills** (`backlog`, `documentation`, `leptos-suspense`) live in
  `.claude/skills/*.md`; read those files directly for the canonical instructions.
- **"Search your history"** = grep the session transcripts
  `~/.claude/projects/*/*.jsonl` first, before git/codebase/infra archaeology.
- **Never** sign up `@example.com` (or other throwaway) accounts against any
  tidl instance that has live SMTP configured — it causes real bounce emails.

## Manual verification gotchas

- Run verification scripts with cwd = repo root; `TidlServer.start()` inherits
  the Python process cwd and resolves `target/site` relative to it (else 404s /
  blank page mistaken for a hydration bug).
- `tidl-server create-admin` ignores `DATABASE_URL`; pass `TIDL_DATA_DIR`
  explicitly or it writes to the real `./data` DB.
- `CARGO_TARGET_DIR` does **not** isolate `cargo leptos build`'s `target/site` —
  it clobbers the user's live `cargo leptos watch` assets. Only a separate
  checkout (worktree) truly isolates it.
- Playwright **works** in this sandbox — `tests/conftest.py` strips the host
  `LD_LIBRARY_PATH` that used to crash bundled Chromium. If an e2e test can't
  find an element, suspect a stale selector (the UI moved), not the browser.
- `#[server]` fn routes are not reachable via direct `curl` in the dev build.
  Test CardDAV/CalDAV and plain Axum routes over HTTP instead; for plain routes,
  insert a `sessions` row manually and send `Cookie: session_<port>=<token>`.

## Detail files

The 20+ memory files (incl. `project_architecture.md`, `project_work_calendar.md`,
`project_server_fn_curl_verification.md`, `project_manual_browser_verification.md`,
`reference_leptos_suspense_skill.md`) live at
`~/.claude/projects/<project-slug>/memory/`.

# Git-backed markdown sources with in-app editor

Extend tidl's markdown source system so users can point tidl at a git repository URL (rather than a local directory). Tidl owns the full lifecycle — clone, pull, edit, commit, push — and an embedded CodeMirror 6 editor lets users edit markdown files directly in the browser.

## Context / Background

The existing filesystem-based markdown source is read-only from the app's perspective. Users must edit files in a separate editor and commit/push manually. For note repositories hosted on Forgejo, GitHub, or similar, this round-trip is friction.

The new design adds a `'git'` source type. Tidl clones the repository to a managed local directory, polls via `git pull` on the same interval schedule as CalDAV remotes, and — when the user saves an edit — writes the file, commits, and pushes. Existing filesystem sources are unchanged.

## Functional Requirements

### Git source type

- A git source is added via the Settings UI with: remote URL, branch (default `main`), display name, colour, and a PAT credential (see Secrets model below).
- On first sync, tidl clones the repository to `<data_dir>/repos/<source_id>/`.
- On each scheduled sync, tidl runs `git pull --ff-only` (or equivalent) before walking the files. Pull errors (auth failure, merge conflict) are stored and surfaced in the source's status row in the UI.
- File parsing, hash-based change detection, and DB upsert are identical to the filesystem source.
- Flake-configured sources appear in the Settings UI as read-only rows (URL, branch, last-sync status visible; no edit for the secret fields). The user may toggle enabled/disabled and change display name + colour.
- When a source is removed from the NixOS config it is NOT auto-deleted from the DB (data safety). The UI marks it as "config-managed / no longer present in config" and allows manual deletion.

### In-app editor

- A task item sourced from any markdown source (filesystem or git) shows an "Edit in app" button alongside the existing "Open in editor →" button in the preview modal.
- Clicking "Edit in app" opens a full CodeMirror 6 editor showing the entire source file, scrolled to the task's line.
- The editor uses the markdown language pack (syntax highlighting).
- Saving from the editor:
  - Writes the file atomically (write to `.tmp`, rename).
  - For git sources: `git add <file>` → `git commit -m "tidl: edit <rel_path>"` → `git push`.
  - For filesystem sources: write only (user's own git workflow handles commit/push).
  - Triggers an immediate markdown re-sync for the source (re-parses the changed file).
- If push fails, the editor shows a clear inline error with the git error message. The local file write is NOT rolled back (data safety — the next pull would conflict, which the user can resolve in their terminal).
- "Cancel" discards unsaved edits (with a confirmation if dirty).

### Secrets model

**Flake-configured sources (NixOS):**

The tidl NixOS module exposes:

```nix
services.tidl = {
  encryptionKeyFile = config.sops.secrets."tidl-enc-key".path;
  gitSources = [
    {
      name   = "notes";
      url    = "https://forgejo.example.com/user/notes.git";
      branch = "main";
      patFile = config.sops.secrets."tidl-notes-pat".path;
    }
  ];
};
```

- `encryptionKeyFile`: path to a 32-byte raw key file (managed by sops-nix). Used to encrypt ad-hoc PATs.
- `gitSources`: declarative list of git sources. `patFile` points to a sops-decrypted file containing the PAT. The PAT is read from disk on each git operation — never stored in the database.
- The module writes a `git-sources.json` config file (no secrets) and passes `TIDL_GIT_SOURCES_FILE` + `TIDL_ENCRYPTION_KEY_FILE` env vars to the service.
- On startup, tidl upserts `git-sources.json` entries into `markdown_sources` with `config_managed = true`.

**Ad-hoc UI sources:**

- The user enters a PAT in a password field in the Settings UI.
- Tidl encrypts it immediately with AES-256-GCM (using the app encryption key) and stores `base64(nonce || ciphertext)` in `markdown_sources.pat_encrypted`.
- The PAT is decrypted on demand for each git operation; it is never returned to the client.
- If `TIDL_ENCRYPTION_KEY_FILE` is not configured, the PAT field is replaced with an informative message directing the user to the NixOS module docs. Ad-hoc git sources are disabled in this case.
- If the encryption key file is lost, stored PATs are unrecoverable — user must re-enter them.

### Crate additions (`tidl-server` only)

- `git2` — libgit2 Rust bindings for clone/pull/commit/push. Requires `libgit2` in `flake.nix` `buildInputs`.
- `aes-gcm` — RustCrypto AES-256-GCM (pure Rust). No C dependency.

Neither crate is added to `tidl-web` or library crates.

## Database changes

New migration adds two nullable columns to `markdown_sources`:

```sql
ALTER TABLE markdown_sources ADD COLUMN pat_encrypted   TEXT;    -- null for filesystem + flake sources
ALTER TABLE markdown_sources ADD COLUMN config_managed  INTEGER NOT NULL DEFAULT 0; -- 1 = from git-sources.json
```

The `type` column (already `TEXT`) accepts `'git'` as a new valid value alongside `'filesystem'`. The git source config JSON: `{"url": "...", "branch": "main"}` — no secrets.

## Open Questions

*(none — all design decisions made)*

## Resolution

Implemented across 8 commits on `feat/git-backed-markdown-sources`:

**DB** — migration `20260621000001_git_sources.sql` adds `pat_encrypted TEXT` and `config_managed INTEGER` to `markdown_sources`.

**Backend** — `crypto.rs` (AES-256-GCM encrypt/decrypt, `base64(nonce||ct)` format); `git_sync.rs` (clone/pull/commit_and_push via `git2` with vendored libgit2); startup loads `TIDL_ENCRYPTION_KEY_FILE` into `AppState.enc_key` and upserts `TIDL_GIT_SOURCES_FILE` entries as config-managed sources; markdown sync loop git-pulls before file-walking for `type='git'` sources.

**Server functions** — `get_source_file_content` and `save_markdown_file` in `api/files.rs`; `add_git_source` and `is_enc_key_configured` in `api/calendars.rs`; git commit+push injected via `GitCommitPusher` closure (keeps `git2` out of the WASM target).

**NixOS module** — `services.tidl.encryptionKeyFile` + `services.tidl.gitSources` options; generates `git-sources.json` in Nix store; injects env vars; `BindReadOnlyPaths` for sops-decrypted secret files.

**Editor** — CodeMirror 6 via ESM CDN (`esm.sh`), thin `assets/editor.js` bridge, `MarkdownFileEditor` Leptos component with `wasm_bindgen(inline_js)` JS bridge, atomic save + git push.

**Settings UI** — source type toggle (Local directory / Git repository) in `MarkdownSourcesPanel`; PAT field shown only when enc key is configured; config-managed sources shown as read-only.

**Preview modal** — "Edit in app" button in `MarkdownPreviewInner` opens `MarkdownFileEditor` as full-screen overlay; uses current nav-stack UID so wikilink navigation is respected.

**Deviations from spec:** The server function was named `get_source_file_content` (not `get_markdown_file_content`) due to a name collision with an existing function. The CodeMirror CDN uses individual `@codemirror/*` ESM packages rather than the convenience `codemirror` bundle.

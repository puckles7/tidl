# Open in Editor — Markdown preview modal

When viewing a markdown file preview in the Focus view, the "Open in editor →" button is currently disabled (stubbed as "Coming soon"). It should open the source file at the relevant line in the user's local editor.

## Background

The `MarkdownPreviewInner` component already has all the data needed:

- `source_file: String` — full path to the markdown file on disk
- `source_line: u32` — the line number where the task/item lives

The challenge is that the UI runs in the browser (Leptos/WASM), so it cannot directly invoke a system editor. Two viable approaches exist:

## Approaches

### Option A — Custom URI scheme (client-side only)
Construct a URI such as `vscode://file/<path>:<line>` or `idea://open?file=<path>&line=<line>` and navigate to it. Works without any server round-trip. Downside: editor-specific and only works if the browser/OS has the URI scheme registered.

### Option B — Server API endpoint (editor-agnostic)
Add a POST `/api/open-in-editor` endpoint that shells out to `$EDITOR` or a configurable command (e.g. `code --goto <file>:<line>`). The button triggers a fetch; the server opens the file. Works for any editor configured server-side.

## Functional Requirements

- The "Open in editor →" button becomes active (not disabled).
- Clicking it opens `source_file` at `source_line` in the user's configured editor.
- Failure (editor not configured, file not found) is surfaced gracefully — e.g. a brief error message near the button.
- No page navigation or modal close as a side effect.

## Open Questions

- Which approach (A or B) to go with? Option B is more flexible but requires a server endpoint and editor configuration.
- If Option B: how is the editor command configured? Environment variable (`$EDITOR`, `$VISUAL`), a settings field, or a hardcoded default?
- Should this feature be guarded so it only appears when the server is running locally (not in a hosted/remote context)?

## Resolution

Implemented Option B (server `#[server]` function) with a settings-page field for the editor command.

**Design decisions:**
- `open_in_editor_cmd` stored as a nullable `TEXT` column in `user_preferences` (migration `20240122000000_open_in_editor_cmd.sql`)
- Command uses `{file}` and `{line}` placeholders, e.g. `code --goto {file}:{line}`; split on whitespace and substituted per-token via `tokio::process::Command` (no shell, no injection risk)
- Falls back to `xdg-open {file}` when no command is configured (opens without line number)
- Inline error displayed near the button on failure; modal stays open
- **Remote support**: Settings → This Device stores a per-browser editor link (VS Code/Cursor/Zed/Sublime/JetBrains or custom) plus a local clone path per git source. A mapped local clone takes precedence; otherwise a browser on the tidl host opens the server copy (zero config, falling back to the host's default opener).
- `Pull latest` in the preview forces an immediate pull and surfaces the source's `last_error`; the background sync still polls at the configured interval.
- Editing the server copy with an external editor leaves the clone dirty. The preview detects uncommitted paths and offers `Commit & push (N)`, which commits and pushes the server clone, then waits for the markdown re-sync.
- Loopback hostname detection is a heuristic; This Device offers an Auto/On/Remote override for LAN names and tunnels.

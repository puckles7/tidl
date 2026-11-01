# Markdown file browser / fuzzy file search

Add a file browser or fuzzy file-finder (nvim telescope-style) for navigating the files backing markdown sources, so users can jump straight to a file without going through a specific task/await link.

## Dependencies

- [18 - Markdown source files for todos and pending items](18-markdown-sources.md) — defines the markdown source model this browses
- [34 - Git-backed markdown sources with in-app editor](34-git-backed-markdown-sources.md) — the in-app CodeMirror editor a picked file should open into

## Context / Background

Today the only way to open a markdown source file in the app is indirectly, via a task/await's "Open in editor" / "Edit in app" action (backlog #23, #34), which requires the file to already contain a recognized task. There's no way to browse or search the file tree of a markdown source directly, e.g. to open a plain note file with no tasks in it, or to jump to a known file by (partial) name.

## Functional Requirements

- A command/shortcut opens a fuzzy file finder listing files across configured markdown sources (filesystem and git-backed).
- Typing filters the list by fuzzy match on path/filename, similar to nvim telescope or a VS Code "Go to File".
- Selecting a file opens it in the in-app editor (backlog #34) at the top of the file.
- Indicate which source each file belongs to when multiple sources are configured (e.g. a small source-colored tag, consistent with existing source pills).

## Open Questions

- Keyboard-only (telescope-style modal with a shortcut to invoke) or also reachable via a visible button/nav entry?
- Should this also expose a full file-tree/directory browser view as an alternative to the flat fuzzy list, or is fuzzy search sufficient on its own?
- Does this need to respect any per-source include/exclude globs, or show every file tidl already indexes?

## Resolution

Built as a modal mounted at the app root (`AuthenticatedApp` in `app.rs`), triggered by both `Ctrl/Cmd+K` (global keydown listener) and a 🔎 navbar button — resolving the first open question in favor of both, for discoverability and mobile use.

- **Server**: `list_all_markdown_files()` (`crates/tidl-web/src/api/files.rs`) walks every *enabled* markdown source's resolved directory (reusing `resolve_markdown_source_dir`) and returns `.md`/`.markdown` relative paths — never file content, unlike `tidl_markdown::MarkdownSource::list_files`. The walk is a small hand-rolled async recursive helper (`walk_markdown_files`) rather than a new `walkdir`/`async-recursion` dependency, boxing its own recursive future with just `std`.
- **Fuzzy match**: `fuzzy_match_score` (`crates/tidl-web/src/util.rs`) is a pure, dependency-free case-insensitive subsequence matcher (telescope/`fzf`-style), scoring contiguous runs higher than scattered hits. Unit-tested directly.
- **Modal**: `MarkdownFileFinder` (`crates/tidl-web/src/components/markdown_file_finder.rs`) — a flat list (no directory-tree view; fuzzy search alone was sufficient) with a search box *below* it per the request, arrow-key/Enter/Escape navigation, and a source-colored tag per row (reusing `source_pill_style`).
- **Opening a file**: reuses the existing rendered markdown preview exactly — synthesizes the same `"md:{source_id}:{rel_path}:1"` uid the Focus/Tasks/Await "go to source" arrows already build, and hands it to `PreviewOverlay`/`PreviewState::Markdown` (`components/focus.rs`), which already includes the in-app "Edit" toggle. No new viewer code.
- **Scope**: every markdown file across enabled sources, filesystem or git-backed; no include/exclude globs (none exist elsewhere in tidl either), answering the third open question.

Deviation: none from the agreed design. Not verified in a live browser this session — the Claude-in-Chrome extension wasn't connected and Playwright is known broken in this sandbox (see project memory); verified instead via `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, and both the `ssr` and `wasm32-unknown-unknown` builds, plus a manually seeded `gts` test server confirmed booting cleanly with a real markdown source attached.

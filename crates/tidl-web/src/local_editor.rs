//! Per-browser "open in editor" preferences.
//!
//! The editor choice, git-clone paths, and the "is this browser on the tidl
//! host?" override are properties of the device/browser the user is sitting
//! at. They are kept in `localStorage` and never sent to the server.
//!
//! The same editor choice drives both opening paths: a browser URI scheme for
//! files on this device, and a derived CLI command for files that only exist on
//! the tidl host (backlog #164).

use std::collections::BTreeMap;

use leptos::prelude::RwSignal;
use serde::{Deserialize, Serialize};

use crate::types::MarkdownSourceInfo;

pub const LOCAL_EDITOR_LS_KEY: &str = "tidl.local_editor";

#[derive(Clone, Copy)]
pub struct LocalEditorCtx {
    pub prefs: RwSignal<LocalEditorPrefs>,
    /// False until the post-hydration effect has loaded `localStorage` and
    /// worked out the browser's likely location. Keeps SSR and the first
    /// hydration pass identical.
    pub hydrated: RwSignal<bool>,
    /// Whether the URL hostname looks like the machine tidl is running on.
    pub detected_on_server: RwSignal<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalEditorPrefs {
    /// Preset id (`vscode`, `cursor`, …) or `custom`.
    #[serde(default = "default_editor_id")]
    pub editor_id: String,
    /// Template used when `editor_id == "custom"`.
    #[serde(default = "default_template")]
    pub template: String,
    /// Normalized git remote URL -> absolute local clone root.
    #[serde(default)]
    pub git_roots: BTreeMap<String, String>,
    /// `None` = auto-detect; `Some(true/false)` = explicit override.
    #[serde(default)]
    pub on_server: Option<bool>,
    /// Optional command used to open files on the tidl host, overriding the
    /// preset's built-in launcher. Empty/`None` means "use the preset's
    /// command, falling back to the host's default opener".
    #[serde(default)]
    pub server_cmd: Option<String>,
}

impl Default for LocalEditorPrefs {
    fn default() -> Self {
        Self {
            editor_id: default_editor_id(),
            template: default_template(),
            git_roots: BTreeMap::new(),
            on_server: None,
            server_cmd: None,
        }
    }
}

fn default_editor_id() -> String {
    "vscode".to_string()
}

fn default_template() -> String {
    "vscode://file/{file}:{line}".to_string()
}

pub struct EditorPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub template: &'static str,
    /// Command used when the file lives on the tidl host and this browser is
    /// running there too. `None` falls back to the host's default opener.
    pub server_cmd: Option<&'static str>,
}

pub const EDITOR_PRESETS: &[EditorPreset] = &[
    EditorPreset {
        id: "vscode",
        label: "VS Code",
        template: "vscode://file/{file}:{line}",
        server_cmd: Some("code --goto {file}:{line}"),
    },
    EditorPreset {
        id: "vscodium",
        label: "VSCodium",
        template: "vscodium://file/{file}:{line}",
        server_cmd: Some("codium --goto {file}:{line}"),
    },
    EditorPreset {
        id: "cursor",
        label: "Cursor",
        template: "cursor://file/{file}:{line}",
        server_cmd: Some("cursor --goto {file}:{line}"),
    },
    EditorPreset {
        id: "zed",
        label: "Zed",
        template: "zed://file/{file}:{line}",
        server_cmd: Some("zed {file}:{line}"),
    },
    EditorPreset {
        id: "sublime",
        label: "Sublime Text",
        template: "subl://open?url=file://{file}&line={line}",
        server_cmd: Some("subl {file}:{line}"),
    },
    EditorPreset {
        id: "jetbrains",
        label: "JetBrains",
        template: "jetbrains://idea/navigate/reference?path={file}&line={line}",
        server_cmd: Some("idea --line {line} {file}"),
    },
    EditorPreset {
        id: "custom",
        label: "Custom",
        template: "",
        server_cmd: None,
    },
];

pub fn preset_template(id: &str) -> Option<&'static str> {
    EDITOR_PRESETS
        .iter()
        .find(|p| p.id == id && p.id != "custom")
        .map(|p| p.template)
}

pub fn editor_label(id: &str) -> &'static str {
    EDITOR_PRESETS
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.label)
        .unwrap_or("Custom")
}

/// The CLI command used to open a file on the tidl host for this editor.
/// Custom editors only carry a browser URI template, so they fall back to the
/// host's default file opener.
pub fn server_cmd_for(editor_id: &str) -> Option<&'static str> {
    EDITOR_PRESETS
        .iter()
        .find(|p| p.id == editor_id)
        .and_then(|p| p.server_cmd)
}

/// Effective command for opening a file on the tidl host.
///
/// A non-empty browser override always wins, then the legacy per-user stored
/// command, then the preset's built-in launcher. A custom preset has no
/// launcher, so the caller falls back to the host's default opener. Preset
/// commands are only defaults — launcher names vary across Windows, distros,
/// Flatpak/Snap, and GUI-only installs, which is why the override exists.
pub fn resolve_server_cmd(
    editor_id: &str,
    override_cmd: Option<&str>,
    stored_cmd: Option<&str>,
) -> Option<String> {
    let clean = |cmd: Option<&str>| {
        cmd.map(str::trim)
            .filter(|cmd| !cmd.is_empty())
            .map(str::to_string)
    };
    clean(override_cmd)
        .or_else(|| clean(stored_cmd))
        .or_else(|| server_cmd_for(editor_id).map(str::to_string))
}

/// Normalize a git remote URL into the key used for local-clone mappings.
///
/// SSH and HTTPS spellings of the same remote canonicalize to the same
/// `host/owner/repo` key, so a clone mapped once matches every transport:
///
/// - `git@github.com:owner/repo.git`
/// - `ssh://git@github.com/owner/repo`
/// - `https://github.com/owner/repo.git/`
///
/// all become `github.com/owner/repo`. The host is lowercased (DNS is
/// case-insensitive); the path is left untouched (repository paths are not).
/// User info, schemes, ports, and the `.git` suffix are dropped.
pub fn normalize_repo_key(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    // scheme://[user@]host[:port]/path
    let (authority, path) = if let Some((_, rest)) = trimmed.split_once("://") {
        match rest.split_once('/') {
            Some((auth, path)) => (auth, path),
            None => (rest, ""),
        }
    } else if let Some((auth, path)) = trimmed.split_once(':') {
        // scp-like `[user@]host:path`; only when the left side can't be a
        // local path (no `/`).
        if auth.contains('/') {
            ("", trimmed)
        } else {
            (auth, path)
        }
    } else {
        ("", trimmed)
    };

    if authority.is_empty() {
        return trimmed.to_string();
    }

    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let path = path.trim_matches('/');
    if path.is_empty() {
        host
    } else {
        format!("{host}/{path}")
    }
}

/// True when `path` looks like an absolute filesystem path on this device.
///
/// Used only for UI feedback: the browser cannot stat the user's filesystem,
/// but it can at least catch relative paths, which no editor URI can resolve.
pub fn is_absolute_path(path: &str) -> bool {
    let path = path.trim();
    if path.starts_with('/') {
        return true;
    }
    let bytes = path.as_bytes();
    // Windows drive path (`C:\...` / `C:/...`) or UNC (`\\server\share`).
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || path.starts_with("\\\\")
}

/// Join a local clone root and the source-relative path using `/` separators.
pub fn join_local_path(root: &str, rel_path: &str) -> String {
    let root = root.trim_end_matches(['/', '\\']);
    let rel_path = rel_path.trim_start_matches('/');
    if root.is_empty() {
        rel_path.to_string()
    } else if rel_path.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{rel_path}")
    }
}

/// Build the editor URI for a preset or custom template.
pub fn build_editor_uri(
    editor_id: &str,
    custom_template: &str,
    abs_path: &str,
    line: u32,
) -> String {
    let normalized = abs_path.replace('\\', "/");
    let path_encoded = percent_encode_path(&normalized);
    let component_encoded = percent_encode_component(&normalized);
    let line = line.max(1);

    match editor_id {
        "vscode" => format!(
            "vscode://file/{}:{}",
            path_encoded.trim_start_matches('/'),
            line
        ),
        "vscodium" => format!(
            "vscodium://file/{}:{}",
            path_encoded.trim_start_matches('/'),
            line
        ),
        "cursor" => format!(
            "cursor://file/{}:{}",
            path_encoded.trim_start_matches('/'),
            line
        ),
        "zed" => format!(
            "zed://file/{}:{}",
            path_encoded.trim_start_matches('/'),
            line
        ),
        "sublime" => format!("subl://open?url=file://{}&line={}", path_encoded, line),
        "jetbrains" => format!(
            "jetbrains://idea/navigate/reference?path={}&line={}",
            component_encoded, line
        ),
        _ => custom_template
            .replace("{file}", &normalized)
            .replace("{file_encoded}", &component_encoded)
            .replace("{line}", &line.to_string()),
    }
}

/// Build an editor URI for a directory (no line/column), used by the Editors
/// tab's "Test" link to open a configured clone root.
pub fn build_editor_dir_uri(editor_id: &str, custom_template: &str, abs_path: &str) -> String {
    let normalized = abs_path.replace('\\', "/");
    let path_encoded = percent_encode_path(&normalized);
    let component_encoded = percent_encode_component(&normalized);

    match editor_id {
        "vscode" => format!("vscode://file/{}", path_encoded.trim_start_matches('/')),
        "vscodium" => format!("vscodium://file/{}", path_encoded.trim_start_matches('/')),
        "cursor" => format!("cursor://file/{}", path_encoded.trim_start_matches('/')),
        "zed" => format!("zed://file/{}", path_encoded.trim_start_matches('/')),
        "sublime" => format!("subl://open?url=file://{}", path_encoded),
        "jetbrains" => format!(
            "jetbrains://idea/navigate/reference?path={}",
            component_encoded
        ),
        _ => custom_template
            .replace("{file}", &normalized)
            .replace("{file_encoded}", &component_encoded)
            .replace("{line}", "1"),
    }
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if is_unreserved(b) || b == b'/' || b == b':' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// True when the browser's URL hostname is a loopback name. This is only a
/// heuristic; see Settings → Editors for the manual override.
pub fn host_is_loopback(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpenTarget {
    /// A git source with a user-managed clone on this device. Opened through
    /// the browser's editor URI scheme.
    LocalClone { abs_path: String },
    /// A filesystem source when this browser is on the tidl host. Opened by
    /// the server-side editor command.
    HostCopy,
    /// A git source with no usable clone mapping on this device.
    ConfigureClone,
    /// A filesystem source the browser cannot reach (remote browser).
    Unavailable,
}

/// Resolve how a markdown source can be opened (backlog #164).
///
/// Git sources always use the user's own clone, never tidl's managed clone,
/// regardless of whether this browser is on the tidl host. Filesystem sources
/// exist only on the tidl host, so they can be opened externally only when the
/// browser is on that host; otherwise the in-app editor is the only option.
pub fn resolve_target(
    source: Option<&MarkdownSourceInfo>,
    rel_path: &str,
    prefs: &LocalEditorPrefs,
    on_server: bool,
) -> OpenTarget {
    let Some(source) = source else {
        return OpenTarget::ConfigureClone;
    };

    if source.source_type == "git" {
        if let Some(url) = source.url.as_deref() {
            let root = prefs.git_roots.get(&normalize_repo_key(url));
            if let Some(root) = root.filter(|r| !r.trim().is_empty()) {
                return OpenTarget::LocalClone {
                    abs_path: join_local_path(root, rel_path),
                };
            }
        }
        OpenTarget::ConfigureClone
    } else if on_server {
        OpenTarget::HostCopy
    } else {
        OpenTarget::Unavailable
    }
}

#[cfg(feature = "hydrate")]
pub fn load_prefs() -> LocalEditorPrefs {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return LocalEditorPrefs::default();
    };
    let mut prefs = storage
        .get_item(LOCAL_EDITOR_LS_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<LocalEditorPrefs>(&raw).ok())
        .unwrap_or_default();
    // Older releases stored raw remote URLs as keys. Re-key them on load so
    // SSH and HTTPS spellings reconcile without asking the user to re-enter
    // paths (backlog #164).
    let old = std::mem::take(&mut prefs.git_roots);
    prefs.git_roots = old
        .into_iter()
        .map(|(key, path)| (normalize_repo_key(&key), path))
        .collect();
    prefs
}

#[cfg(not(feature = "hydrate"))]
pub fn load_prefs() -> LocalEditorPrefs {
    LocalEditorPrefs::default()
}

#[cfg(feature = "hydrate")]
pub fn save_prefs(prefs: &LocalEditorPrefs) -> Result<(), String> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| "This browser does not expose local storage.".to_string())?;
    let raw = serde_json::to_string(prefs)
        .map_err(|e| format!("Could not serialize local editor settings: {e}"))?;
    storage
        .set_item(LOCAL_EDITOR_LS_KEY, &raw)
        .map_err(|_| "Could not save local editor settings in this browser.".to_string())
}

#[cfg(not(feature = "hydrate"))]
pub fn save_prefs(_prefs: &LocalEditorPrefs) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(source_type: &str, url: Option<&str>) -> MarkdownSourceInfo {
        MarkdownSourceInfo {
            id: 7,
            name: "notes".into(),
            path: String::new(),
            enabled: true,
            last_scan: None,
            color: String::new(),
            source_type: source_type.into(),
            url: url.map(str::to_string),
            branch: Some("main".into()),
            config_managed: false,
            sync_interval_secs: 60,
            last_error: None,
        }
    }

    #[test]
    fn joins_paths_without_doubling_separators() {
        assert_eq!(
            join_local_path("/home/alice/notes/", "/dir/note.md"),
            "/home/alice/notes/dir/note.md"
        );
        assert_eq!(
            join_local_path("/home/alice/notes", "note.md"),
            "/home/alice/notes/note.md"
        );
        assert_eq!(join_local_path("", "note.md"), "note.md");
    }

    #[test]
    fn normalizes_git_urls() {
        assert_eq!(
            normalize_repo_key("https://forgejo.example/me/notes.git/"),
            "forgejo.example/me/notes"
        );
        assert_eq!(
            normalize_repo_key("  git@example:me/notes.git  "),
            "example/me/notes"
        );
    }

    #[test]
    fn builds_directory_uris_without_a_line() {
        assert_eq!(
            build_editor_dir_uri("vscode", "", "/home/alice/my notes"),
            "vscode://file/home/alice/my%20notes"
        );
        assert_eq!(
            build_editor_dir_uri("sublime", "", "/home/alice/notes"),
            "subl://open?url=file:///home/alice/notes"
        );
        assert_eq!(
            build_editor_dir_uri(
                "custom",
                "myeditor://open?file={file_encoded}&line={line}",
                "/a b"
            ),
            "myeditor://open?file=%2Fa%20b&line=1"
        );
    }

    #[test]
    fn reconciles_ssh_and_https_spellings() {
        let https = normalize_repo_key("https://github.com/owner/repo.git");
        let ssh_scp = normalize_repo_key("git@github.com:owner/repo.git");
        let ssh_url = normalize_repo_key("ssh://git@github.com/owner/repo");
        let https_port = normalize_repo_key("https://github.com:8443/owner/repo/");
        assert_eq!(https, "github.com/owner/repo");
        assert_eq!(ssh_scp, https);
        assert_eq!(ssh_url, https);
        assert_eq!(https_port, https);
    }

    #[test]
    fn recognizes_absolute_paths() {
        assert!(is_absolute_path("/home/alice/notes"));
        assert!(is_absolute_path("C:\\Users\\alice\\notes"));
        assert!(is_absolute_path("C:/Users/alice/notes"));
        assert!(is_absolute_path("\\\\server\\share\\notes"));
        assert!(!is_absolute_path(""));
        assert!(!is_absolute_path("notes"));
        assert!(!is_absolute_path("./notes"));
        assert!(!is_absolute_path("~/notes"));
    }

    #[test]
    fn derives_server_commands_from_presets() {
        assert_eq!(server_cmd_for("vscode"), Some("code --goto {file}:{line}"));
        assert_eq!(server_cmd_for("zed"), Some("zed {file}:{line}"));
        assert_eq!(server_cmd_for("custom"), None);
        assert_eq!(server_cmd_for("nope"), None);
    }

    #[test]
    fn server_command_resolution_order() {
        assert_eq!(
            resolve_server_cmd("vscode", Some("code.cmd --goto {file}:{line}"), None).as_deref(),
            Some("code.cmd --goto {file}:{line}")
        );
        assert_eq!(
            resolve_server_cmd("vscode", None, Some("ghostty -e nvim -c {line} {file}")).as_deref(),
            Some("ghostty -e nvim -c {line} {file}")
        );
        assert_eq!(
            resolve_server_cmd("vscode", Some("   "), Some("  ")).as_deref(),
            Some("code --goto {file}:{line}")
        );
        assert_eq!(resolve_server_cmd("custom", None, None), None);
        assert_eq!(
            resolve_server_cmd("custom", Some("my-editor {file}"), None).as_deref(),
            Some("my-editor {file}")
        );
    }

    #[test]
    fn builds_editor_uris_and_escapes_spaces() {
        assert_eq!(
            build_editor_uri("vscode", "", "/home/alice/my notes/a.md", 12),
            "vscode://file/home/alice/my%20notes/a.md:12"
        );
        assert_eq!(
            build_editor_uri("vscodium", "", "/home/alice/a.md", 7),
            "vscodium://file/home/alice/a.md:7"
        );
        assert_eq!(
            build_editor_uri("cursor", "", "/home/alice/a.md", 0),
            "cursor://file/home/alice/a.md:1"
        );
        assert_eq!(
            build_editor_uri("sublime", "", "/home/alice/a.md", 3),
            "subl://open?url=file:///home/alice/a.md&line=3"
        );
        assert_eq!(
            build_editor_uri("jetbrains", "", "/home/alice/a b.md", 3),
            "jetbrains://idea/navigate/reference?path=%2Fhome%2Falice%2Fa%20b.md&line=3"
        );
        assert_eq!(
            build_editor_uri(
                "custom",
                "myeditor://open?file={file_encoded}&line={line}",
                "/home/alice/a b.md",
                4
            ),
            "myeditor://open?file=%2Fhome%2Falice%2Fa%20b.md&line=4"
        );
    }

    #[test]
    fn local_clone_is_used_regardless_of_location() {
        let mut prefs = LocalEditorPrefs::default();
        prefs.git_roots.insert(
            normalize_repo_key("https://forgejo.example/me/notes.git"),
            "/home/alice/notes".into(),
        );
        for on_server in [true, false] {
            assert_eq!(
                resolve_target(
                    Some(&source("git", Some("git@forgejo.example:me/notes.git"))),
                    "dir/note.md",
                    &prefs,
                    on_server
                ),
                OpenTarget::LocalClone {
                    abs_path: "/home/alice/notes/dir/note.md".into()
                }
            );
        }
    }

    #[test]
    fn resolves_remaining_targets() {
        let prefs = LocalEditorPrefs::default();
        let git = source("git", Some("https://example.com/r.git"));
        assert_eq!(
            resolve_target(Some(&git), "a.md", &prefs, true),
            OpenTarget::ConfigureClone,
            "git never falls back to tidl's managed clone"
        );
        assert_eq!(
            resolve_target(Some(&git), "a.md", &prefs, false),
            OpenTarget::ConfigureClone
        );
        let fs = source("filesystem", None);
        assert_eq!(
            resolve_target(Some(&fs), "a.md", &prefs, true),
            OpenTarget::HostCopy
        );
        assert_eq!(
            resolve_target(Some(&fs), "a.md", &prefs, false),
            OpenTarget::Unavailable
        );
    }

    #[test]
    fn clone_mapping_is_independent_of_the_editor_template() {
        let mut prefs = LocalEditorPrefs::default();
        prefs.editor_id = "custom".into();
        prefs.template = String::new();
        prefs.git_roots.insert(
            normalize_repo_key("https://example.com/r.git"),
            "/home/alice/r".into(),
        );
        // The clone exists even though the editor has no usable link template.
        // The preview shows the path but disables the launch button.
        assert_eq!(
            resolve_target(
                Some(&source("git", Some("https://example.com/r.git"))),
                "a.md",
                &prefs,
                false
            ),
            OpenTarget::LocalClone {
                abs_path: "/home/alice/r/a.md".into()
            }
        );
    }

    #[test]
    fn detects_loopback_names() {
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("[::1]"));
        assert!(!host_is_loopback("tidl.lan"));
        assert!(!host_is_loopback("192.168.1.20"));
    }
}

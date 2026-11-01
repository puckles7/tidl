//! Per-browser "open in editor" preferences.
//!
//! Unlike the server-side editor command in `user_preferences`, the editor
//! link, git-clone paths, and the "is this browser on the tidl host?" override
//! are properties of the device/browser the user is sitting at. They are kept
//! in `localStorage` and never sent to the server.

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
}

impl Default for LocalEditorPrefs {
    fn default() -> Self {
        Self {
            editor_id: default_editor_id(),
            template: default_template(),
            git_roots: BTreeMap::new(),
            on_server: None,
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
}

pub const EDITOR_PRESETS: &[EditorPreset] = &[
    EditorPreset {
        id: "vscode",
        label: "VS Code",
        template: "vscode://file/{file}:{line}",
    },
    EditorPreset {
        id: "vscodium",
        label: "VSCodium",
        template: "vscodium://file/{file}:{line}",
    },
    EditorPreset {
        id: "cursor",
        label: "Cursor",
        template: "cursor://file/{file}:{line}",
    },
    EditorPreset {
        id: "zed",
        label: "Zed",
        template: "zed://file/{file}:{line}",
    },
    EditorPreset {
        id: "sublime",
        label: "Sublime Text",
        template: "subl://open?url=file://{file}&line={line}",
    },
    EditorPreset {
        id: "jetbrains",
        label: "JetBrains",
        template: "jetbrains://idea/navigate/reference?path={file}&line={line}",
    },
    EditorPreset {
        id: "custom",
        label: "Custom",
        template: "",
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

/// Normalize a git remote URL into the key used for local-clone mappings.
/// Only strips the common `.git` suffix and trailing whitespace/slashes; it
/// deliberately does not try to reconcile SSH and HTTPS spellings of the same
/// repository.
pub fn normalize_repo_key(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    trimmed.strip_suffix(".git").unwrap_or(trimmed).to_string()
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
/// heuristic; see "This Device" for the manual override.
pub fn host_is_loopback(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpenTarget {
    LocalClone {
        abs_path: String,
    },
    ServerCopy,
    /// Remote, and this source has no local clone mapping.
    NotConfigured,
    /// Remote filesystem source: the files only exist on the server.
    ServerOnly,
}

pub fn resolve_target(
    source: Option<&MarkdownSourceInfo>,
    rel_path: &str,
    prefs: &LocalEditorPrefs,
    on_server: bool,
) -> OpenTarget {
    let Some(source) = source else {
        return OpenTarget::NotConfigured;
    };

    if source.source_type == "git" {
        if let Some(url) = source.url.as_deref() {
            let root = prefs.git_roots.get(&normalize_repo_key(url));
            let has_editor = prefs.editor_id != "custom" || !prefs.template.trim().is_empty();
            if let Some(root) = root.filter(|r| !r.trim().is_empty()).filter(|_| has_editor) {
                return OpenTarget::LocalClone {
                    abs_path: join_local_path(root, rel_path),
                };
            }
        }
        if on_server {
            OpenTarget::ServerCopy
        } else {
            OpenTarget::NotConfigured
        }
    } else if on_server {
        OpenTarget::ServerCopy
    } else {
        OpenTarget::ServerOnly
    }
}

#[cfg(feature = "hydrate")]
pub fn load_prefs() -> LocalEditorPrefs {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return LocalEditorPrefs::default();
    };
    storage
        .get_item(LOCAL_EDITOR_LS_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
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
            "https://forgejo.example/me/notes"
        );
        assert_eq!(
            normalize_repo_key("  git@example:me/notes.git  "),
            "git@example:me/notes"
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
    fn local_clone_wins_over_server_copy() {
        let mut prefs = LocalEditorPrefs::default();
        prefs.git_roots.insert(
            normalize_repo_key("https://forgejo.example/me/notes.git"),
            "/home/alice/notes".into(),
        );
        assert_eq!(
            resolve_target(
                Some(&source("git", Some("https://forgejo.example/me/notes.git"))),
                "dir/note.md",
                &prefs,
                true
            ),
            OpenTarget::LocalClone {
                abs_path: "/home/alice/notes/dir/note.md".into()
            }
        );
    }

    #[test]
    fn resolves_server_copy_and_remote_states() {
        let prefs = LocalEditorPrefs::default();
        let git = source("git", Some("https://example.com/r.git"));
        assert_eq!(
            resolve_target(Some(&git), "a.md", &prefs, true),
            OpenTarget::ServerCopy
        );
        assert_eq!(
            resolve_target(Some(&git), "a.md", &prefs, false),
            OpenTarget::NotConfigured
        );
        let fs = source("filesystem", None);
        assert_eq!(
            resolve_target(Some(&fs), "a.md", &prefs, false),
            OpenTarget::ServerOnly
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

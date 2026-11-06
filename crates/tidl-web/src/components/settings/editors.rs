//! "Editors" settings tab — per-browser editor choice and local git clone
//! paths. Nothing here is sent to the server; it all lives in `localStorage`.

use leptos::prelude::*;

use super::UserPrefsCtx;
use crate::api::{list_markdown_sources, MarkdownSourceInfo};
use crate::local_editor::{
    build_editor_dir_uri, editor_label, is_absolute_path, normalize_repo_key, preset_template,
    save_prefs, server_cmd_for, LocalEditorCtx, LocalEditorPrefs, EDITOR_PRESETS,
};
use crate::util::format_server_fn_error;

/// Small `(i)` affordance with an immediate, CSS-only tooltip. Native `title`
/// tooltips have an OS-controlled delay, so the hint lives in `data-tip` and is
/// revealed by `:hover`/`:focus-visible` in `style.css`.
#[component]
fn InfoBadge(tip: &'static str) -> impl IntoView {
    view! {
        <span
            class="info-badge"
            data-tip=tip
            aria-label=tip
            tabindex="0"
            role="note"
        >"i"</span>
    }
}

#[component]
pub(super) fn EditorsPanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let ctx = use_context::<LocalEditorCtx>();
    let initial = ctx.map(|c| c.prefs.get_untracked()).unwrap_or_default();
    // Older releases stored the tidl-host command in `user_preferences`
    // instead of this browser. Surface it as the initial override so existing
    // installs keep working and users can see what will be launched.
    let stored_server_cmd = use_context::<UserPrefsCtx>()
        .map(|s| s.get_untracked().open_in_editor_cmd)
        .flatten();

    let editor_id = RwSignal::new(initial.editor_id.clone());
    let template = RwSignal::new(initial.template.clone());
    let server_cmd = RwSignal::new(
        initial
            .server_cmd
            .clone()
            .or(stored_server_cmd)
            .unwrap_or_default(),
    );
    let roots = RwSignal::new(initial.git_roots.clone());
    let on_server = RwSignal::new(match initial.on_server {
        None => "auto".to_string(),
        Some(true) => "on".to_string(),
        Some(false) => "off".to_string(),
    });
    let loaded = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    // The context is populated after hydration; if this tab mounted before
    // that happened, pull the values in once they arrive.
    if let Some(ctx) = ctx {
        Effect::new(move |_| {
            if ctx.hydrated.get() && !loaded.get_untracked() {
                let prefs = ctx.prefs.get_untracked();
                editor_id.set(prefs.editor_id);
                template.set(prefs.template);
                server_cmd.set(prefs.server_cmd.unwrap_or_default());
                roots.set(prefs.git_roots);
                on_server.set(match prefs.on_server {
                    None => "auto".to_string(),
                    Some(true) => "on".to_string(),
                    Some(false) => "off".to_string(),
                });
                loaded.set(true);
            }
        });
    }

    let sources_res: LocalResource<Vec<MarkdownSourceInfo>> =
        LocalResource::new(|| async move { list_markdown_sources().await.unwrap_or_default() });

    let on_save = move |_: leptos::ev::MouseEvent| {
        error.set(None);
        let cmd = server_cmd.get_untracked();
        let prefs = LocalEditorPrefs {
            editor_id: editor_id.get_untracked(),
            template: template.get_untracked(),
            git_roots: roots.get_untracked(),
            on_server: match on_server.get_untracked().as_str() {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            },
            server_cmd: if cmd.trim().is_empty() {
                None
            } else {
                Some(cmd)
            },
        };
        match save_prefs(&prefs) {
            Ok(()) => {
                if let Some(ctx) = ctx {
                    ctx.prefs.set(prefs);
                }
                on_close.with_value(|f| f());
            }
            Err(e) => error.set(Some(format_server_fn_error(&e))),
        }
    };

    view! {
        <p class="dialog-hint">
            "These settings relate to your local device, so they are stored in this browser — \
             not in your tidl account."
        </p>

        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <div class="form-group">
            <div class="form-label-row form-label-row--info">
                <label class="form-label">"Editor on this device"</label>
                <InfoBadge tip="The editor installed where this browser runs, launched through its registered URL scheme. Git sources always open in your own clone on this device. Local directory sources on the tidl host use the 'Command on the tidl host' setting under Advanced instead. Editors without a URL scheme can't be launched from the browser — use Edit in browser." />
            </div>
            <select
                class="form-input"
                prop:value=move || editor_id.get()
                on:change=move |ev| {
                    let id = event_target_value(&ev);
                    if let Some(t) = preset_template(&id) {
                        template.set(t.to_string());
                    }
                    editor_id.set(id);
                }
            >
                {EDITOR_PRESETS.iter().map(|p| {
                    let id = p.id;
                    let label = p.label;
                    view! { <option value=id>{label}</option> }
                }).collect_view()}
            </select>
            <p class="form-hint">
                "Used for git sources: opens your own clone on this device."
            </p>
            {move || (editor_id.get() == "custom").then(|| view! {
                <p class="form-hint">
                    "Set this editor's URL template under Advanced."
                </p>
            })}
        </div>

        <details class="form-advanced">
            <summary>"Advanced"</summary>

            <div class="form-group">
                <div class="form-label-row form-label-row--info">
                    <label class="form-label">"Link template (this device)"</label>
                    <InfoBadge tip="Used only to open your own git clone on this device. Include {file} (or {file_encoded}) and {line}. The editor must register this URL scheme with your operating system. Files on the tidl host use the command under Advanced." />
                </div>
                <input
                    type="text"
                    class="form-input"
                    placeholder="vscode://file/{file}:{line}"
                    prop:value=move || template.get()
                    on:input=move |ev| {
                        editor_id.set("custom".to_string());
                        template.set(event_target_value(&ev));
                    }
                />
                <p class="form-hint">
                    "Used to open your own git clone on this device."
                </p>
                {move || {
                    let t = template.get();
                    if editor_id.get() != "custom" || t.trim().is_empty() {
                        return ().into_any();
                    }
                    let missing_file = !t.contains("{file}") && !t.contains("{file_encoded}");
                    let missing_line = !t.contains("{line}");
                    if missing_file {
                        view! {
                            <p class="form-hint local-clone-warning">
                                "This template has no {file} placeholder, so the browser will not \
                                 receive the file path."
                            </p>
                        }.into_any()
                    } else if missing_line {
                        view! {
                            <p class="form-hint local-clone-warning">
                                "This template has no {line} placeholder; the file will open \
                                 without a line number."
                            </p>
                        }.into_any()
                    } else if !t.contains("://") {
                        view! {
                            <p class="form-hint local-clone-warning">
                                "This does not look like a URL scheme template; the browser can \
                                 only launch editors that register one."
                            </p>
                        }.into_any()
                    } else {
                        ().into_any()
                    }
                }}
            </div>

            <div class="form-group">
                <div class="form-label-row form-label-row--info">
                    <label class="form-label">"Command on the tidl host"</label>
                    <InfoBadge tip="Only used for Local directory sources when this browser runs on the tidl host; git sources never use it. Use {file} and {line}. Leave empty to use the preset's launcher or the host's default opener. Set the exact command if the default name is wrong — e.g. code.cmd on Windows, or a Flatpak/Snap wrapper." />
                </div>
                <input
                    type="text"
                    class="form-input"
                    placeholder=move || {
                        server_cmd_for(&editor_id.get())
                            .unwrap_or("(host's default opener)")
                            .to_string()
                    }
                    prop:value=move || server_cmd.get()
                    on:input=move |ev| server_cmd.set(event_target_value(&ev))
                />
                <p class="form-hint">
                    "Used only for Local directory sources when this browser is on the tidl host."
                </p>
            </div>

            <div class="form-group">
                <div class="form-label-row form-label-row--info">
                    <label class="form-label">"Browser location"</label>
                    <InfoBadge tip="Only affects Local directory sources. Git repositories always open in your own clone on this device, wherever the browser runs." />
                </div>
                <select
                    class="form-input"
                    prop:value=move || on_server.get()
                    on:change=move |ev| on_server.set(event_target_value(&ev))
                >
                    <option value="auto">"Auto-detect"</option>
                    <option value="on">"This browser is on the tidl host"</option>
                    <option value="off">"This browser is remote"</option>
                </select>
                <p class="form-hint">
                    {move || {
                        let detected = ctx
                            .map(|c| c.detected_on_server.get())
                            .unwrap_or(false);
                        if detected {
                            "Auto-detected: on the tidl host (localhost URL). Choose 'This browser \
                             is remote' if this localhost address is really an SSH tunnel or proxy."
                        } else {
                            "Auto-detected: remote. Choose 'This browser is on the tidl host' if the \
                             browser and tidl share a machine but you reach it through a LAN/Tailscale \
                             hostname."
                        }
                    }}
                </p>
            </div>
        </details>

        <div class="settings-section-header settings-section-header--info">
            <h3 class="settings-section-heading">"Local clones"</h3>
            <InfoBadge tip="Git repositories always open in your own clone, never in tidl's managed copy. Commit and push from your clone to bring changes into tidl; tidl pulls them in automatically. 'Edit in browser' edits tidl's copy directly." />
        </div>

        <p class="dialog-hint">
            "Point each git source at the clone on this device. The folder must be an absolute path."
        </p>

        {move || {
            let list = sources_res.get();
            let Some(list) = list else {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            };
            let git_sources: Vec<MarkdownSourceInfo> = list
                .into_iter()
                .filter(|s| s.source_type == "git" && s.url.is_some())
                .collect();
            if git_sources.is_empty() {
                return view! {
                    <p class="sharing-empty">
                        "No git-backed Markdown sources configured."
                    </p>
                }.into_any();
            }

            git_sources.into_iter().map(|src| {
                let url = src.url.unwrap_or_default();
                let key = normalize_repo_key(&url);
                let key_for_value = key.clone();
                let key_for_input = key.clone();
                let key_for_test = key.clone();
                let name = src.name.clone();
                let url_for_display = url.clone();
                view! {
                    <div class="sharing-row local-clone-row">
                        <div class="sharing-md-info">
                            <span class="sharing-grantee">{name}</span>
                            <span class="sharing-md-path">{url_for_display}</span>
                        </div>
                        <div class="local-clone-input-wrap">
                            <input
                                type="text"
                                class="form-input local-clone-input"
                                placeholder="/absolute/path/to/your/clone"
                                prop:value=move || {
                                    roots.with(|m| m.get(&key_for_value).cloned().unwrap_or_default())
                                }
                                on:input=move |ev| {
                                    let value = event_target_value(&ev);
                                    roots.update(|m| {
                                        if value.trim().is_empty() {
                                            m.remove(&key_for_input);
                                        } else {
                                            m.insert(key_for_input.clone(), value);
                                        }
                                    });
                                }
                            />
                            {move || {
                                let root = roots.with(|m| m.get(&key_for_test).cloned().unwrap_or_default());
                                let root = root.trim().to_string();
                                if root.is_empty() {
                                    return ().into_any();
                                }
                                if !is_absolute_path(&root) {
                                    return view! {
                                        <p class="form-hint local-clone-warning">
                                            "Use an absolute path (starting with / or a drive letter)."
                                        </p>
                                    }.into_any();
                                }
                                let uri = build_editor_dir_uri(
                                    &editor_id.get(),
                                    &template.get(),
                                    &root,
                                );
                                if uri.is_empty() {
                                    return view! {
                                        <p class="form-hint local-clone-warning">
                                            "Choose an editor (or set a link template under Advanced) to test this path."
                                        </p>
                                    }.into_any();
                                }
                                let label = editor_label(&editor_id.get());
                                view! {
                                    <a
                                        class="btn-secondary-small"
                                        href=uri
                                        title=format!("Try opening {root} in {label}")
                                    >"Test"</a>
                                }.into_any()
                            }}
                        </div>
                    </div>
                }
            }).collect_view().into_any()
        }}

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Close"</button>
            <button class="btn-primary" on:click=on_save>"Save"</button>
        </div>
    }
}

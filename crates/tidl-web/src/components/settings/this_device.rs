//! "This Device" settings tab — per-browser editor link and local git clone
//! paths. Nothing here is sent to the server; it all lives in `localStorage`.

use leptos::prelude::*;

use crate::api::{list_markdown_sources, MarkdownSourceInfo};
use crate::local_editor::{
    normalize_repo_key, preset_template, save_prefs, LocalEditorCtx, LocalEditorPrefs,
    EDITOR_PRESETS,
};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn ThisDevicePanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let ctx = use_context::<LocalEditorCtx>();
    let initial = ctx.map(|c| c.prefs.get_untracked()).unwrap_or_default();

    let editor_id = RwSignal::new(initial.editor_id.clone());
    let template = RwSignal::new(initial.template.clone());
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
        let prefs = LocalEditorPrefs {
            editor_id: editor_id.get_untracked(),
            template: template.get_untracked(),
            git_roots: roots.get_untracked(),
            on_server: match on_server.get_untracked().as_str() {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
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
            "These settings are stored in this browser, not on the server."
        </p>

        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <h3 class="settings-section-heading">"Local editor"</h3>

        <div class="form-group">
            <label class="form-label">"Editor"</label>
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
        </div>

        <div class="form-group">
            <label class="form-label">"Link template"</label>
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
                "Use {file} and {line}. {file_encoded} is also available for custom templates. \
                 Pick the editor installed on this machine — its URL scheme must be registered \
                 (VSCodium uses vscodium://, Cursor uses cursor://, VS Code uses vscode://)."
            </p>
        </div>

        <div class="form-group">
            <label class="form-label">"Browser location"</label>
            <select
                class="form-input"
                prop:value=move || on_server.get()
                on:change=move |ev| on_server.set(event_target_value(&ev))
            >
                <option value="auto">"Auto-detect"</option>
                <option value="on">"On the tidl host"</option>
                <option value="off">"Remote"</option>
            </select>
            <p class="form-hint">
                {move || {
                    let detected = ctx
                        .map(|c| c.detected_on_server.get())
                        .unwrap_or(false);
                    if detected {
                        "Auto-detected: on the tidl host (localhost URL). Choose Remote if this \
                         localhost address is actually an SSH tunnel or proxy to another machine."
                    } else {
                        "Auto-detected: remote. Choose On the tidl host if this browser and tidl \
                         share a machine but you reach it through a LAN/Tailscale hostname."
                    }
                }}
            </p>
        </div>

        <h3 class="settings-section-heading">"Local clones"</h3>

        <p class="dialog-hint">
            "Tidl keeps its own clone. Changes you make in a local clone do not reach Tidl \
             until you commit and push them, then use \"Pull latest\" in the preview (or wait \
             for the next automatic pull). \"Edit on server (in app)\" edits Tidl's clone directly."
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
                let name = src.name.clone();
                let url_for_display = url.clone();
                view! {
                    <div class="sharing-row local-clone-row">
                        <div class="sharing-md-info">
                            <span class="sharing-grantee">{name}</span>
                            <span class="sharing-md-path">{url_for_display}</span>
                        </div>
                        <input
                            type="text"
                            class="form-input local-clone-input"
                            placeholder="/path/to/local/clone"
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

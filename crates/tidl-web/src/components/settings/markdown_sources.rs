use leptos::prelude::*;

use crate::api::{list_markdown_sources, trigger_markdown_sync, MarkdownSourceInfo};
use crate::app::use_sidebar_refresh;
use crate::components::cal_edit_modal::MdSourceEditModal;

#[component]
pub(super) fn MarkdownSourcesPanel() -> impl IntoView {
    let md_list = RwSignal::new(Vec::<MarkdownSourceInfo>::new());
    let md_loading = RwSignal::new(true);
    let md_syncing_id: RwSignal<Option<i64>> = RwSignal::new(None);

    // `None` = modal closed; `Some(None)` = create; `Some(Some(src))` = edit.
    let editing_src: RwSignal<Option<Option<MarkdownSourceInfo>>> = RwSignal::new(None);

    leptos::task::spawn_local(async move {
        if let Ok(list) = list_markdown_sources().await {
            md_list.set(list);
        }
        md_loading.set(false);
    });

    let reload_md_sources = move || {
        md_loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_markdown_sources().await {
                md_list.set(list);
            }
            md_loading.set(false);
        });
    };
    // Bumping the global refresh signal (not just this panel's own list) is
    // what makes the sidebar's source list pick up adds/edits/deletes made
    // here — its `LocalResource` depends on the same signal (`focus_sidebar.rs`).
    let sidebar_refresh = use_sidebar_refresh();
    let on_refresh = Callback::new(move |_| {
        reload_md_sources();
        sidebar_refresh.update(|n| *n += 1);
    });

    // Poll every 3 s until `last_scan` changes for the triggered source.
    #[cfg(not(feature = "hydrate"))]
    let start_md_sync_poll = |_: i64| {};
    #[cfg(feature = "hydrate")]
    let start_md_sync_poll = move |poll_id: i64| {
        leptos::task::spawn_local(async move {
            let old_ts = md_list
                .get_untracked()
                .into_iter()
                .find(|s| s.id == poll_id)
                .and_then(|s| s.last_scan.clone());

            for _ in 0..10u8 {
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    let _ = web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3_000);
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;

                if let Ok(list) = list_markdown_sources().await {
                    let new_ts = list
                        .iter()
                        .find(|s| s.id == poll_id)
                        .and_then(|s| s.last_scan.clone());
                    md_list.set(list);
                    md_loading.set(false);
                    if new_ts != old_ts {
                        break;
                    }
                } else {
                    break;
                }
            }
            md_syncing_id.set(None);
        });
    };

    view! {
        <p class="sharing-empty">
            "Connect server-side Markdown directories or git repositories as a read-only source for tasks and await items."
        </p>

        <div class="settings-section-header">
            <h3 class="settings-section-heading">"Configured sources"</h3>
            <button
                class="btn-secondary-small"
                on:click=move |_| editing_src.set(Some(None))
            >"+ Add source"</button>
        </div>

        {move || {
            if md_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            let list = md_list.get();
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No Markdown sources configured."</p> }.into_any();
            }
            list.into_iter().map(|src| {
                let src_id         = src.id;
                let is_git         = src.source_type == "git";
                let config_managed = src.config_managed;
                let src_edit       = src.clone();
                // Display path or URL depending on source type.
                let location: String = if is_git {
                    src.url.unwrap_or_default()
                } else {
                    src.path.clone()
                };
                view! {
                    <div class="sharing-row">
                        <div class="sharing-md-info">
                            <span class="sharing-grantee">{src.name}</span>
                            {config_managed.then(|| view! {
                                <span class="sharing-md-badge">"(config managed)"</span>
                            })}
                            <span class="sharing-md-path">{location}</span>
                            <span class="sharing-md-path">"every " {src.sync_interval_secs} "s"</span>
                            {src.last_scan.map(|t| view! {
                                <span class="sharing-md-scan">"Last scan: " {t}</span>
                            })}
                            {src.last_error.clone().map(|e| view! {
                                <span class="sharing-md-error">{e}</span>
                            })}
                            {move || (md_syncing_id.get() == Some(src_id)).then(|| view! {
                                <span class="sync-badge">
                                    <span class="sync-spinner"></span>
                                    "Syncing…"
                                </span>
                            })}
                        </div>
                        <button
                            class="btn-secondary-small"
                            disabled=move || md_syncing_id.get() == Some(src_id)
                            on:click=move |_| {
                                md_syncing_id.set(Some(src_id));
                                leptos::task::spawn_local(async move {
                                    let _ = trigger_markdown_sync(src_id).await;
                                    start_md_sync_poll(src_id);
                                });
                            }
                        >
                            {move || {
                                if md_syncing_id.get() == Some(src_id) {
                                    "Syncing…"
                                } else {
                                    "Sync now"
                                }
                            }}
                        </button>
                        <button
                            class="btn-secondary-small"
                            on:click=move |_| editing_src.set(Some(Some(src_edit.clone())))
                        >"Edit"</button>
                    </div>
                }
            }).collect_view().into_any()
        }}

        {move || editing_src.get().map(|src_opt| {
            view! {
                <MdSourceEditModal
                    src=src_opt
                    on_close=move || editing_src.set(None)
                    on_refresh=on_refresh
                />
            }
        })}
    }
}

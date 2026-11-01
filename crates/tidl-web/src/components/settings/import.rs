use leptos::prelude::*;

use super::CalsResource;
use crate::api::{
    import_ics, import_keep, import_vcf, list_note_collections, ImportResult, KeepImportResult,
    VcfImportPreview,
};
use crate::app::{
    use_contacts_needs_relayout, use_contacts_refresh, use_events_refresh, use_notes_refresh,
    use_sidebar_refresh,
};
use crate::util::format_server_fn_error;

/// Yields to the browser's event loop so a pending DOM update — e.g. a button
/// label flipping to "Importing…" — actually paints before the following,
/// possibly heavy, step runs. `spawn_local` resumes in a microtask, and the
/// browser drains all microtasks *before* rendering, so awaiting a
/// `setTimeout(0)` promise (a fresh macrotask) is the first point the
/// compositor gets to paint.
#[cfg(feature = "hydrate")]
async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window().and_then(|w| {
            w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
                .ok()
        });
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// SSR stub: nothing to yield to when there is no browser.
#[cfg(not(feature = "hydrate"))]
async fn yield_to_browser() {}

#[component]
pub(super) fn ImportPanel(cals_res: CalsResource) -> impl IntoView {
    view! {
        <IcsImportSection cals_res=cals_res />
        <h3 class="settings-section-heading">"Contacts (.vcf)"</h3>
        <VcfImportSection />
        <h3 class="settings-section-heading">"Google Keep (.zip)"</h3>
        <KeepImportSection />
    }
}

#[component]
fn IcsImportSection(cals_res: CalsResource) -> impl IntoView {
    let file_content = RwSignal::new(Option::<String>::None);
    let file_name = RwSignal::new(Option::<String>::None);
    let event_count = RwSignal::new(0u32);
    let import_cal = RwSignal::new(String::new());
    let import_cal_new = RwSignal::new(String::new());
    let import_overwrite = RwSignal::new(false);
    let import_result = RwSignal::new(Option::<ImportResult>::None);
    let import_error = RwSignal::new(Option::<String>::None);
    let is_importing = RwSignal::new(false);

    let sidebar_refresh = use_sidebar_refresh();
    let events_refresh = use_events_refresh();

    let on_file_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast as _;
            let input = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            if let Some(input) = input {
                if let Some(files) = input.files() {
                    if let Some(file) = files.get(0) {
                        let name = file.name();
                        file_name.set(Some(name));
                        file_content.set(None);
                        event_count.set(0);
                        import_result.set(None);
                        import_error.set(None);
                        leptos::task::spawn_local(async move {
                            let promise = file.text();
                            match wasm_bindgen_futures::JsFuture::from(promise).await {
                                Ok(js_str) => {
                                    let content = js_str.as_string().unwrap_or_default();
                                    let count = content.matches("BEGIN:VEVENT").count() as u32;
                                    event_count.set(count);
                                    file_content.set(Some(content));
                                }
                                Err(_) => {
                                    file_name.set(None);
                                }
                            }
                        });
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    let on_import = move |_: leptos::ev::MouseEvent| {
        let content = match file_content.get_untracked() {
            Some(c) => c,
            None => return,
        };
        if is_importing.get_untracked() {
            return;
        }
        is_importing.set(true);
        import_result.set(None);
        import_error.set(None);
        let pick = import_cal.get_untracked();
        let is_new_cal = pick == "__new__";
        let cal = if is_new_cal {
            import_cal_new.get_untracked()
        } else {
            pick
        };
        let over = import_overwrite.get_untracked();
        leptos::task::spawn_local(async move {
            match import_ics(content, cal, over).await {
                Ok(result) => {
                    import_result.set(Some(result));
                    file_content.set(None);
                    file_name.set(None);
                    event_count.set(0);
                    if is_new_cal {
                        sidebar_refresh.update(|n| *n += 1);
                    }
                    events_refresh.update(|n| *n += 1);
                }
                Err(e) => {
                    import_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            is_importing.set(false);
        });
    };

    view! {
        <h3 class="settings-section-heading">"Calendar (.ics)"</h3>
        <div class="form-group">
            <label class="form-label">"Calendar file (.ics)"</label>
            <input
                type="file"
                accept=".ics"
                class="form-input"
                on:change=on_file_change
            />
        </div>

        {move || {
            file_name.get().map(|name| {
                let count = event_count.get();
                let msg = if count == 1 {
                    format!("\"{}\" — 1 event found", name)
                } else {
                    format!("\"{}\" — {} events found", name, count)
                };
                view! { <p class="import-preview">{msg}</p> }
            })
        }}

        <div class="form-group">
            <label class="form-label">"Import into"</label>
            <select
                class="form-input"
                prop:value=move || import_cal.get()
                on:change=move |ev| import_cal.set(event_target_value(&ev))
            >
                <option value="" disabled=true>"— pick a calendar —"</option>
                {move || {
                    cals_res.get()
                        .and_then(Result::ok)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|cal| {
                            let name    = cal.name.clone();
                            let display = cal.display_name.clone();
                            view! { <option value=name>{display}</option> }
                        })
                        .collect_view()
                }}
                <option value="__new__">"New calendar…"</option>
            </select>
        </div>
        <Show when=move || import_cal.get() == "__new__">
            <div class="form-group">
                <label class="form-label">"New calendar slug"</label>
                <input
                    type="text"
                    class="form-input"
                    placeholder="e.g. personal"
                    prop:value=move || import_cal_new.get()
                    on:input=move |ev| import_cal_new.set(event_target_value(&ev))
                />
            </div>
        </Show>

        <div class="form-group">
            <label class="form-label">"Duplicates"</label>
            <div class="settings-radio-group">
                {[
                    (false, "Skip existing"),
                    (true,  "Overwrite existing"),
                ].into_iter().map(|(val, label)| view! {
                    <label class="settings-radio-label">
                        <input
                            type="radio"
                            name="import_overwrite"
                            prop:checked=move || import_overwrite.get() == val
                            on:change=move |_| import_overwrite.set(val)
                        />
                        {label}
                    </label>
                }).collect_view()}
            </div>
        </div>

        {move || import_error.get().map(|e| view! {
            <div class="form-error">{e}</div>
        })}
        {move || import_result.get().map(|r| {
            let mut parts = vec![format!("{} imported", r.imported)];
            if r.skipped > 0 { parts.push(format!("{} skipped", r.skipped)); }
            if r.failed  > 0 { parts.push(format!("{} failed",  r.failed));  }
            view! { <p class="import-done">{parts.join(", ")}</p> }
        })}

        <div class="dialog-actions">
            <button
                class="btn-primary"
                on:click=on_import
                disabled=move || {
                    let pick = import_cal.get();
                    is_importing.get()
                        || file_content.get().is_none()
                        || pick.trim().is_empty()
                        || (pick == "__new__" && import_cal_new.get().trim().is_empty())
                }
            >
                {move || if is_importing.get() { "Importing…" } else { "Import" }}
            </button>
        </div>
    }
}

/// `.vcf` contacts import (backlog #87) — same file-read/preview/commit
/// shape as `IcsImportSection`, but the "preview" step also needs a
/// server round trip (`preview_vcf_import`) to report a duplicate count,
/// since duplicate-matching reads the current address book.
#[component]
fn VcfImportSection() -> impl IntoView {
    let file_content = RwSignal::new(Option::<String>::None);
    let file_name = RwSignal::new(Option::<String>::None);
    let preview = RwSignal::new(Option::<VcfImportPreview>::None);
    let is_previewing = RwSignal::new(false);
    let overwrite = RwSignal::new(false);
    let import_result = RwSignal::new(Option::<ImportResult>::None);
    let import_error = RwSignal::new(Option::<String>::None);
    let is_importing = RwSignal::new(false);

    let contacts_refresh = use_contacts_refresh();
    let contacts_needs_relayout = use_contacts_needs_relayout();

    let on_file_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use crate::api::preview_vcf_import;
            use wasm_bindgen::JsCast as _;
            let input = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            if let Some(input) = input {
                if let Some(files) = input.files() {
                    if let Some(file) = files.get(0) {
                        let name = file.name();
                        file_name.set(Some(name));
                        file_content.set(None);
                        preview.set(None);
                        import_result.set(None);
                        import_error.set(None);
                        leptos::task::spawn_local(async move {
                            let promise = file.text();
                            match wasm_bindgen_futures::JsFuture::from(promise).await {
                                Ok(js_str) => {
                                    let content = js_str.as_string().unwrap_or_default();
                                    file_content.set(Some(content.clone()));
                                    is_previewing.set(true);
                                    match preview_vcf_import(content).await {
                                        Ok(p) => preview.set(Some(p)),
                                        Err(e) => {
                                            import_error
                                                .set(Some(format_server_fn_error(&e.to_string())));
                                            file_content.set(None);
                                            file_name.set(None);
                                        }
                                    }
                                    is_previewing.set(false);
                                }
                                Err(_) => {
                                    file_name.set(None);
                                }
                            }
                        });
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    let on_import = move |_: leptos::ev::MouseEvent| {
        let content = match file_content.get_untracked() {
            Some(c) => c,
            None => return,
        };
        if is_importing.get_untracked() {
            return;
        }
        is_importing.set(true);
        import_result.set(None);
        import_error.set(None);
        let over = overwrite.get_untracked();
        leptos::task::spawn_local(async move {
            // `None`: import lands in the user's default collection
            // (backlog #81) — no per-import collection picker yet.
            match import_vcf(content, over, None).await {
                Ok(result) => {
                    if result.imported > 0 {
                        contacts_needs_relayout.set(true);
                    }
                    import_result.set(Some(result));
                    file_content.set(None);
                    file_name.set(None);
                    preview.set(None);
                    contacts_refresh.update(|n| *n += 1);
                }
                Err(e) => {
                    import_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            is_importing.set(false);
        });
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Contacts file (.vcf)"</label>
            <input
                type="file"
                accept=".vcf"
                class="form-input"
                on:change=on_file_change
            />
        </div>

        {move || {
            file_name.get().map(|name| {
                if is_previewing.get() {
                    view! { <p class="import-preview">{format!("\"{}\" — checking for duplicates…", name)}</p> }.into_any()
                } else if let Some(p) = preview.get() {
                    let contact_word = if p.total == 1 { "contact" } else { "contacts" };
                    let msg = if p.duplicates > 0 {
                        format!("\"{}\" — {} {} found, {} look like duplicates", name, p.total, contact_word, p.duplicates)
                    } else {
                        format!("\"{}\" — {} {} found", name, p.total, contact_word)
                    };
                    view! { <p class="import-preview">{msg}</p> }.into_any()
                } else {
                    ().into_any()
                }
            })
        }}

        <div class="form-group">
            <label class="form-label">"Duplicates"</label>
            <div class="settings-radio-group">
                {[
                    (false, "Skip existing"),
                    (true,  "Overwrite existing"),
                ].into_iter().map(|(val, label)| view! {
                    <label class="settings-radio-label">
                        <input
                            type="radio"
                            name="vcf_import_overwrite"
                            prop:checked=move || overwrite.get() == val
                            on:change=move |_| overwrite.set(val)
                        />
                        {label}
                    </label>
                }).collect_view()}
            </div>
        </div>

        {move || import_error.get().map(|e| view! {
            <div class="form-error">{e}</div>
        })}
        {move || import_result.get().map(|r| {
            let mut parts = vec![format!("{} imported", r.imported)];
            if r.skipped > 0 { parts.push(format!("{} skipped", r.skipped)); }
            if r.failed  > 0 { parts.push(format!("{} failed",  r.failed));  }
            view! { <p class="import-done">{parts.join(", ")}</p> }
        })}

        <div class="dialog-actions">
            <button
                class="btn-primary"
                on:click=on_import
                disabled=move || {
                    is_importing.get() || is_previewing.get() || preview.get().is_none()
                }
            >
                {move || if is_importing.get() { "Importing…" } else { "Import" }}
            </button>
        </div>
    }
}

/// Google Keep Takeout `.zip` import (backlog #109). Unlike the `.ics`/`.vcf`
/// sections the archive is read (and base64-encoded) client-side and parsed
/// server-side, so there's no client-visible preview of the note count — the
/// summary appears after the import instead.
#[component]
fn KeepImportSection() -> impl IntoView {
    /// Sentinel `<option>` value for "create a new collection", mirroring
    /// `IcsImportSection`'s `"__new__"` calendar entry.
    const NEW_COLLECTION: &str = "__new__";

    /// Mirrors `api::keep`'s `MAX_KEEP_ZIP_BYTES`, checked here so an
    /// oversized file is rejected before it's read into wasm memory.
    #[cfg(feature = "hydrate")]
    const MAX_KEEP_ZIP_CLIENT_BYTES: f64 = 100.0 * 1024.0 * 1024.0;

    let file_b64 = RwSignal::new(Option::<String>::None);
    let file_name = RwSignal::new(Option::<String>::None);
    let target = RwSignal::new(String::new());
    let new_coll_name = RwSignal::new(String::new());
    let include_arch = RwSignal::new(false);
    let split_shared = RwSignal::new(true);
    let import_result = RwSignal::new(Option::<KeepImportResult>::None);
    let import_error = RwSignal::new(Option::<String>::None);
    let is_importing = RwSignal::new(false);
    let colls_refresh = RwSignal::new(0u32);

    let collections_res = LocalResource::new(move || {
        let _ = colls_refresh.get();
        async move { list_note_collections().await.unwrap_or_default() }
    });

    // Default the target once the list resolves: the first collection, or the
    // "Add new collection…" entry when the user has none yet — so there is
    // always a way to create the collection an import needs, and the Import
    // button is never left disabled without explanation. Only ever fills an
    // empty selection, so a user's own choice is never overwritten.
    Effect::new(move |_| {
        if target.get_untracked().is_empty() {
            if let Some(colls) = collections_res.get() {
                match colls.into_iter().next() {
                    Some(first) => target.set(first.id.to_string()),
                    None => target.set(NEW_COLLECTION.to_string()),
                }
            }
        }
    });

    let notes_refresh = use_notes_refresh();
    let sidebar_refresh = use_sidebar_refresh();

    let on_file_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast as _;
            let input = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            let Some(file) = input.and_then(|i| i.files()).and_then(|f| f.get(0)) else {
                return;
            };
            let name = file.name();
            file_b64.set(None);
            import_result.set(None);
            import_error.set(None);
            if file.size() > MAX_KEEP_ZIP_CLIENT_BYTES {
                file_name.set(None);
                import_error.set(Some("File exceeds the 100 MB limit".to_string()));
                return;
            }
            file_name.set(Some(name));
            leptos::task::spawn_local(async move {
                let promise = file.array_buffer();
                match wasm_bindgen_futures::JsFuture::from(promise).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        file_b64.set(Some(b64));
                    }
                    Err(_) => {
                        file_name.set(None);
                        import_error.set(Some("Could not read the archive.".to_string()));
                    }
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    let on_import = move |_: leptos::ev::MouseEvent| {
        if is_importing.get_untracked() {
            return;
        }

        let pick = target.get_untracked();
        let (target_id, new_name) = if pick == NEW_COLLECTION {
            let name = new_coll_name.get_untracked().trim().to_string();
            if name.is_empty() {
                import_error.set(Some("Enter a name for the new collection.".to_string()));
                return;
            }
            (None, Some(name))
        } else {
            match pick.parse::<i64>() {
                Ok(id) => (Some(id), None),
                Err(_) => {
                    import_error.set(Some(
                        "Choose a collection to import into first.".to_string(),
                    ));
                    return;
                }
            }
        };

        if file_b64.with_untracked(|f| f.is_none()) {
            return;
        }

        is_importing.set(true);
        import_result.set(None);
        import_error.set(None);
        let archived = include_arch.get_untracked();
        let split = split_shared.get_untracked();
        leptos::task::spawn_local(async move {
            // Paint "Importing…" before reading (cloning) and base64-shipping
            // the archive — otherwise the click handler's task blocks the
            // browser and the label only appears once the request is already
            // in flight.
            yield_to_browser().await;
            let Some(b64) = file_b64.get_untracked() else {
                is_importing.set(false);
                return;
            };
            match import_keep(b64, target_id, archived, split, new_name).await {
                Ok(result) => {
                    if result.imported > 0 {
                        notes_refresh.update(|n| *n += 1);
                        // New "Shared with …" collections can appear, and the
                        // Notes sidebar lists them.
                        sidebar_refresh.update(|n| *n += 1);
                    }
                    import_result.set(Some(result));
                    file_b64.set(None);
                    file_name.set(None);
                    // Re-default the picker: the refreshed list now includes
                    // any collection an import just created.
                    target.set(String::new());
                    new_coll_name.set(String::new());
                    colls_refresh.update(|n| *n += 1);
                }
                Err(e) => {
                    import_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            is_importing.set(false);
        });
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Google Keep Takeout archive (.zip)"</label>
            <input
                type="file"
                accept=".zip,application/zip"
                class="form-input"
                on:change=on_file_change
            />
            <p class="import-preview">
                "Export Keep via Google Takeout, then upload the .zip as downloaded — no need to extract it first."
            </p>
        </div>

        <div class="form-group">
            <label class="form-label">"Import into"</label>
            <select
                class="form-input"
                prop:value=move || target.get()
                on:change=move |ev| target.set(event_target_value(&ev))
            >
                <option value="" disabled=true>"— pick a collection —"</option>
                {move || {
                    collections_res.get()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|coll| {
                            let id = coll.id.to_string();
                            view! { <option value=id>{coll.name}</option> }
                        })
                        .collect_view()
                }}
                <option value=NEW_COLLECTION>"Add new collection…"</option>
            </select>
            <Show when=move || target.get() == NEW_COLLECTION>
                <div class="form-group">
                    <label class="form-label">"New collection name"</label>
                    <input
                        type="text"
                        class="form-input"
                        placeholder="e.g. Imported from Keep"
                        prop:value=move || new_coll_name.get()
                        on:input=move |ev| new_coll_name.set(event_target_value(&ev))
                    />
                </div>
            </Show>
        </div>

        <div class="form-group">
            <label class="settings-radio-label">
                <input
                    type="checkbox"
                    prop:checked=move || include_arch.get()
                    on:change=move |ev| include_arch.set(event_target_checked(&ev))
                />
                "Include archived notes (trashed notes are always skipped)"
            </label>
        </div>

        <div class="form-group">
            <label class="settings-radio-label">
                <input
                    type="checkbox"
                    prop:checked=move || split_shared.get()
                    on:change=move |ev| split_shared.set(event_target_checked(&ev))
                />
                "Create separate collections for notes shared in Keep"
            </label>
            <p class="import-preview">
                {move || if split_shared.get() {
                    "Notes shared in Keep are grouped into \"Shared with <email>\" collections; everything else goes to the collection above."
                } else {
                    "Notes shared in Keep go to the collection above, just like unshared notes."
                }}
                " Re-importing the same archive skips notes already imported."
            </p>
        </div>

        {move || import_error.get().map(|e| view! {
            <div class="form-error">{e}</div>
        })}
        {move || import_result.get().map(|r| {
            let mut parts = vec![format!("{} imported", r.imported)];
            if r.images > 0 { parts.push(format!("{} image{}", r.images, if r.images == 1 { "" } else { "s" })); }
            if r.skipped > 0 { parts.push(format!("{} skipped", r.skipped)); }
            if r.duplicates > 0 { parts.push(format!("{} already imported", r.duplicates)); }
            if r.failed  > 0 { parts.push(format!("{} failed",  r.failed));  }
            view! { <p class="import-done">{parts.join(", ")}</p> }
        })}

        <div class="dialog-actions">
            <button
                class="btn-primary"
                on:click=on_import
                title=move || {
                    if file_b64.get().is_none() {
                        "Choose a Keep .zip first"
                    } else if target.get() == NEW_COLLECTION
                        && new_coll_name.get().trim().is_empty()
                    {
                        "Enter a name for the new collection"
                    } else {
                        "Import the selected Keep archive"
                    }
                }
                disabled=move || is_importing.get() || file_b64.get().is_none()
            >
                {move || if is_importing.get() { "Importing…" } else { "Import" }}
            </button>
        </div>
    }
}

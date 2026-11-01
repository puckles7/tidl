use super::CalsResource;
use leptos::prelude::*;

#[component]
pub(super) fn ExportPanel(cals_res: CalsResource) -> impl IntoView {
    view! {
        <h3 class="settings-section-heading">"Calendar (.ics)"</h3>
        <CalendarExportSection cals_res=cals_res />
        <h3 class="settings-section-heading">"Contacts (.vcf)"</h3>
        <ContactsExportSection />
    }
}

#[component]
fn CalendarExportSection(cals_res: CalsResource) -> impl IntoView {
    let selected = RwSignal::new(String::new());

    view! {
        <div class="form-group">
            <label class="form-label">"Calendar"</label>
            <select
                class="form-input"
                prop:value=move || selected.get()
                on:change=move |ev| selected.set(event_target_value(&ev))
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
            </select>
        </div>

        <div class="dialog-actions">
            {move || {
                let slug = selected.get();
                if slug.is_empty() {
                    view! {
                        <button class="btn-primary" disabled=true>
                            "Export"
                        </button>
                    }.into_any()
                } else {
                    let href = format!("/export/{}", slug);
                    let filename = format!("{}.ics", slug);
                    view! {
                        <a class="btn-primary" href=href download=filename>
                            "Export"
                        </a>
                    }.into_any()
                }
            }}
        </div>
    }
}

/// All contacts as one concatenated `.vcf` (backlog #89) — unlike calendar
/// export, there's exactly one address book per user, so no picker is
/// needed; the link is always live.
#[component]
fn ContactsExportSection() -> impl IntoView {
    view! {
        <p class="dialog-hint">"Download your whole address book as a single .vcf file."</p>
        <div class="dialog-actions">
            <a class="btn-primary" href="/export/contacts" download="contacts.vcf">
                "Export"
            </a>
        </div>
    }
}

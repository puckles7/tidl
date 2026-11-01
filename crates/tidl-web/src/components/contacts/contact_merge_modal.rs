//! Contact-merge conflict-resolution modal (backlog #88) — shown once 2+
//! contacts are selected in `ContactsView`'s merge-select mode. Only
//! scalar fields where the selected contacts actually disagree get a
//! picker; repeatable fields (emails/phones/urls/impps/addresses/
//! categories) are unioned automatically server-side (see
//! `api::contacts::union_methods` et al) and just get a one-line notice
//! here — see `MergedScalarFields`'s doc comment for why.

use leptos::prelude::*;

use crate::api::merge_contacts;
use crate::types::{ContactDto, MergeContactsInput, MergedScalarFields};
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// One scalar field slot: a label, an accessor into `ContactDto`, and a
/// setter into the in-progress `MergedScalarFields`. Declared as a table
/// (rather than one hand-written `<div class="form-group">` per field) so
/// the "only show fields that actually differ" logic in `conflicts` below
/// lives in one place instead of twelve near-identical copies.
struct FieldSlot {
    label: &'static str,
    get: fn(&ContactDto) -> Option<String>,
    set: fn(&mut MergedScalarFields, Option<String>),
}

fn field_slots() -> Vec<FieldSlot> {
    vec![
        FieldSlot {
            label: "Name",
            get: |c| Some(c.display_name.clone()).filter(|s| !s.is_empty()),
            set: |m, v| m.display_name = v.unwrap_or_default(),
        },
        FieldSlot {
            label: "Given name",
            get: |c| c.given_name.clone(),
            set: |m, v| m.given_name = v,
        },
        FieldSlot {
            label: "Family name",
            get: |c| c.family_name.clone(),
            set: |m, v| m.family_name = v,
        },
        FieldSlot {
            label: "Middle name",
            get: |c| c.middle_name.clone(),
            set: |m, v| m.middle_name = v,
        },
        FieldSlot {
            label: "Prefix",
            get: |c| c.name_prefix.clone(),
            set: |m, v| m.name_prefix = v,
        },
        FieldSlot {
            label: "Suffix",
            get: |c| c.name_suffix.clone(),
            set: |m, v| m.name_suffix = v,
        },
        FieldSlot {
            label: "Nickname",
            get: |c| c.nickname.clone(),
            set: |m, v| m.nickname = v,
        },
        FieldSlot {
            label: "Organisation",
            get: |c| c.org.clone(),
            set: |m, v| m.org = v,
        },
        FieldSlot {
            label: "Title",
            get: |c| c.title.clone(),
            set: |m, v| m.title = v,
        },
        FieldSlot {
            label: "Birthday",
            get: |c| c.birthday.clone(),
            set: |m, v| m.birthday = v,
        },
        FieldSlot {
            label: "Anniversary",
            get: |c| c.anniversary.clone(),
            set: |m, v| m.anniversary = v,
        },
        FieldSlot {
            label: "Notes",
            get: |c| c.notes.clone(),
            set: |m, v| m.notes = v,
        },
    ]
}

/// The contact with the most non-empty scalar fields wins the default
/// primary pick (ties broken by most-recently-updated) — a "sensible
/// default" per the backlog #88 spec, saving the common case (one contact
/// is obviously the fuller record) an extra click.
fn default_primary(contacts: &[ContactDto]) -> i64 {
    let slots = field_slots();
    contacts
        .iter()
        .max_by_key(|c| {
            let filled = slots.iter().filter(|s| (s.get)(c).is_some()).count();
            (filled, c.updated_at.clone())
        })
        .map(|c| c.id)
        .unwrap_or_default()
}

#[component]
pub fn ContactMergeModal(
    contacts: Vec<ContactDto>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let contacts = StoredValue::new(contacts);

    let primary_id = RwSignal::new(default_primary(&contacts.get_value()));
    let has_photo = contacts.get_value().iter().any(|c| c.has_photo);
    let keep_photo_from: RwSignal<Option<i64>> = RwSignal::new(
        contacts
            .get_value()
            .iter()
            .find(|c| c.id == primary_id.get_untracked() && c.has_photo)
            .map(|c| c.id)
            .or_else(|| {
                contacts
                    .get_value()
                    .iter()
                    .find(|c| c.has_photo)
                    .map(|c| c.id)
            }),
    );
    let saving = RwSignal::new(false);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);

    // One resolved value per field slot, keyed by slot index (stable —
    // `field_slots()` is a fixed compile-time list) — defaults to the
    // current primary's value, re-seeded whenever `primary_id` changes.
    let resolved: RwSignal<Vec<Option<String>>> = RwSignal::new(Vec::new());
    Effect::new(move |_| {
        let pid = primary_id.get();
        let all = contacts.get_value();
        let primary = all.iter().find(|c| c.id == pid);
        let slots = field_slots();
        let seeded = slots
            .iter()
            .map(|s| {
                primary
                    .and_then(|c| (s.get)(c))
                    .or_else(|| all.iter().find_map(|c| (s.get)(c)))
            })
            .collect();
        resolved.set(seeded);
    });

    let do_merge = move |_: leptos::ev::MouseEvent| {
        saving.set(true);
        error_msg.set(None);
        let all = contacts.get_value();
        let pid = primary_id.get_untracked();
        let merged_ids: Vec<i64> = all.iter().map(|c| c.id).filter(|id| *id != pid).collect();
        let slots = field_slots();
        let values = resolved.get_untracked();
        let mut fields = MergedScalarFields::default();
        for (slot, value) in slots.iter().zip(values.into_iter()) {
            (slot.set)(&mut fields, value);
        }
        let input = MergeContactsInput {
            primary_id: pid,
            merged_ids,
            resolved: fields,
            keep_photo_from: keep_photo_from.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            match merge_contacts(input).await {
                Ok(_) => {
                    saving.set(false);
                    on_refresh.run(());
                    on_close.get_value()();
                }
                Err(e) => {
                    saving.set(false);
                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.get_value()());

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                ev.stop_propagation();
                backdrop_mousedown.set(mouse_event_targets_self(&ev));
            }
            on:click=move |ev: leptos::ev::MouseEvent| {
                ev.stop_propagation();
                if backdrop_mousedown.get() { on_close.get_value()(); }
            }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{format!("Merge {} contacts", contacts.get_value().len())}</h2>
                </div>
                <div class="dialog-form">
                    <p class="dialog-hint">"This can't be undone. Pick which contact survives, and resolve any fields below where they disagree — emails, phones, URLs, addresses, and categories from every contact are kept automatically."</p>

                    <div class="form-group">
                        <label class="form-label">"Surviving contact"</label>
                        <div class="contact-merge-primary-picker">
                            {contacts.get_value().into_iter().map(|c| {
                                let cid = c.id;
                                let name = c.display_name.clone();
                                view! {
                                    <label class="contact-merge-radio-row">
                                        <input type="radio" name="merge-primary"
                                            checked=move || primary_id.get() == cid
                                            on:change=move |_| primary_id.set(cid) />
                                        {name}
                                    </label>
                                }
                            }).collect_view()}
                        </div>
                    </div>

                    {move || {
                        let all = contacts.get_value();
                        let slots = field_slots();
                        slots.iter().enumerate().filter_map(|(i, slot)| {
                            let mut distinct: Vec<String> = all.iter().filter_map(|c| (slot.get)(c)).collect();
                            distinct.sort();
                            distinct.dedup();
                            if distinct.len() < 2 {
                                return None; // no conflict — nothing to ask
                            }
                            let label = slot.label;
                            Some(view! {
                                <div class="form-group">
                                    <label class="form-label">{label}</label>
                                    <div class="contact-merge-radio-row-group">
                                        {distinct.into_iter().map(|value| {
                                            let value_for_radio = value.clone();
                                            let value_for_click = value.clone();
                                            view! {
                                                <label class="contact-merge-radio-row">
                                                    <input type="radio" name=format!("merge-field-{i}")
                                                        checked=move || resolved.get().get(i).cloned().flatten().as_deref() == Some(value_for_radio.as_str())
                                                        on:change=move |_| resolved.update(|r| {
                                                            if let Some(slot_val) = r.get_mut(i) { *slot_val = Some(value_for_click.clone()); }
                                                        }) />
                                                    {value.clone()}
                                                </label>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>
                            })
                        }).collect_view()
                    }}

                    {has_photo.then(|| view! {
                        <div class="form-group">
                            <label class="form-label">"Photo"</label>
                            <div class="contact-merge-radio-row-group">
                                {contacts.get_value().into_iter().filter(|c| c.has_photo).map(|c| {
                                    let cid = c.id;
                                    view! {
                                        <label class="contact-merge-radio-row">
                                            <input type="radio" name="merge-photo"
                                                checked=move || keep_photo_from.get() == Some(cid)
                                                on:change=move |_| keep_photo_from.set(Some(cid)) />
                                            {c.display_name.clone()}"'s photo"
                                        </label>
                                    }
                                }).collect_view()}
                            </div>
                        </div>
                    })}

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>
                <div class="dialog-actions">
                    <button class="btn-danger" disabled=move || saving.get() on:click=do_merge>
                        {move || if saving.get() { "Merging…" } else { "Merge" }}
                    </button>
                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>
    }
}

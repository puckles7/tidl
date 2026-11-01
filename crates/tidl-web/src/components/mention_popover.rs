//! Small contact-detail popover opened by clicking a resolved `@mention` in
//! a rendered note (backlog #80) or a rendered markdown file (backlog
//! #91). A genuine "detail" glance — broader than
//! the Contacts-tab card (which also shows nickname/URL/emails/phones/notes
//! but, being a compact grid tile, leaves out birthday/anniversary/
//! categories/addresses) — plus a "Go to contact →" link into the real
//! card. Deliberately not the full `ContactCard` component, which also
//! carries drag/delete affordances that don't belong in a read-only glance.

use leptos::prelude::*;

use crate::api::{ContactAddress, ContactDto};
use crate::util::mouse_event_targets_self;

/// One postal address as a single readable line: "kind: street, city,
/// region postal_code, country" — empty components dropped, no line at all
/// if every component is empty (a stray all-blank `ADR` row some CardDAV
/// clients emit).
fn format_address(a: &ContactAddress) -> Option<String> {
    let region_postal = [a.region.as_deref(), a.postal_code.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let parts: Vec<String> = [a.street.as_deref(), a.city.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .chain((!region_postal.is_empty()).then_some(region_postal))
        .chain(
            a.country
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        )
        .collect();
    (!parts.is_empty()).then(|| format!("{}: {}", a.kind, parts.join(", ")))
}

#[component]
pub fn MentionPopover(
    contact: ContactDto,
    /// Viewport coordinates (from the clicked mention's
    /// `getBoundingClientRect()`) — `position: fixed`, so these map directly.
    x: f64,
    y: f64,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let backdrop_mousedown = RwSignal::new(false);

    let subtitle = match (&contact.title, &contact.org) {
        (Some(t), Some(o)) if !t.is_empty() && !o.is_empty() => Some(format!("{t} · {o}")),
        (Some(t), _) if !t.is_empty() => Some(t.clone()),
        (_, Some(o)) if !o.is_empty() => Some(o.clone()),
        _ => None,
    };
    let nickname = contact.nickname.clone().filter(|n| !n.is_empty());
    let dates: Vec<String> = [
        contact.birthday.as_deref().map(|d| format!("🎂 {d}")),
        contact.anniversary.as_deref().map(|d| format!("💍 {d}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let notes = contact.notes.clone().filter(|n| !n.is_empty());
    let display_name = contact.display_name.clone();
    let has_photo = contact.has_photo;
    let id = contact.id;
    let initial = display_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let emails = contact.emails.clone();
    let phones = contact.phones.clone();
    let urls = contact.urls.clone();
    let impps = contact.impps.clone();
    let addresses: Vec<String> = contact
        .addresses
        .iter()
        .filter_map(format_address)
        .collect();
    let categories = contact.categories.clone();

    view! {
        <div
            class="mention-popover-backdrop"
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div
                class="mention-popover"
                style=format!("left:{x}px;top:{y}px;")
                on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
            >
                <div class="mention-popover-body">
                    <div class="mention-popover-top">
                        {if has_photo {
                            view! { <img class="contact-card-avatar" src=format!("/api/contacts/{id}/photo") /> }.into_any()
                        } else {
                            view! { <div class="contact-card-avatar-placeholder">{initial}</div> }.into_any()
                        }}
                        <div class="contact-card-name">
                            {display_name}
                            {nickname.map(|n| view! { <span class="contact-card-nickname">" (" {n} ")"</span> })}
                        </div>
                        <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                    </div>

                    {subtitle.map(|s| view! { <div class="contact-card-subtitle">{s}</div> })}

                    {(!dates.is_empty()).then(|| view! {
                        <div class="contact-card-subtitle">{dates.join("   ")}</div>
                    })}

                    {(!categories.is_empty()).then(|| view! {
                        <div class="mention-popover-categories">
                            {categories.into_iter().map(|c| view! {
                                <span class="mention-popover-category">{c}</span>
                            }).collect_view()}
                        </div>
                    })}

                    {(!emails.is_empty()).then(|| view! {
                        <div class="contact-card-methods">
                            {emails.into_iter().map(|m| view! {
                                <div class="contact-card-method">
                                    <span class="contact-card-method-kind">{m.kind}</span>
                                    <span class="contact-card-method-value">{m.value}</span>
                                </div>
                            }).collect_view()}
                        </div>
                    })}

                    {(!phones.is_empty()).then(|| view! {
                        <div class="contact-card-methods">
                            {phones.into_iter().map(|m| view! {
                                <div class="contact-card-method">
                                    <span class="contact-card-method-kind">{m.kind}</span>
                                    <span class="contact-card-method-value">{m.value}</span>
                                </div>
                            }).collect_view()}
                        </div>
                    })}

                    {(!urls.is_empty()).then(|| view! {
                        <div class="contact-card-methods">
                            {urls.into_iter().map(|m| {
                                let href = m.value.clone();
                                view! {
                                    <div class="contact-card-method">
                                        <span class="contact-card-method-kind">{m.kind}</span>
                                        <a class="contact-card-method-value" href=href target="_blank" rel="noopener noreferrer">{m.value}</a>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}

                    {(!impps.is_empty()).then(|| view! {
                        <div class="contact-card-methods">
                            {impps.into_iter().map(|m| view! {
                                <div class="contact-card-method">
                                    <span class="contact-card-method-kind">{m.kind}</span>
                                    <span class="contact-card-method-value">{m.value}</span>
                                </div>
                            }).collect_view()}
                        </div>
                    })}

                    {(!addresses.is_empty()).then(|| view! {
                        <div class="contact-card-notes">
                            {addresses.into_iter().map(|a| view! { <div>{a}</div> }).collect_view()}
                        </div>
                    })}

                    {notes.map(|n| view! { <div class="contact-card-notes">{n}</div> })}
                </div>

                <div class="mention-popover-footer">
                    <a class="btn-secondary fp-open-btn" href=format!("/contacts?highlight={id}")>
                        "Go to contact →"
                    </a>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::format_address;
    use crate::api::ContactAddress;

    fn addr(
        street: Option<&str>,
        city: Option<&str>,
        region: Option<&str>,
        postal: Option<&str>,
        country: Option<&str>,
    ) -> ContactAddress {
        ContactAddress {
            kind: "home".to_string(),
            is_primary: false,
            street: street.map(str::to_string),
            city: city.map(str::to_string),
            region: region.map(str::to_string),
            postal_code: postal.map(str::to_string),
            country: country.map(str::to_string),
        }
    }

    #[test]
    fn formats_a_full_address() {
        let a = addr(
            Some("1 Engine Row"),
            Some("London"),
            None,
            Some("EC1"),
            Some("UK"),
        );
        assert_eq!(
            format_address(&a).as_deref(),
            Some("home: 1 Engine Row, London, EC1, UK")
        );
    }

    #[test]
    fn drops_empty_components() {
        let a = addr(Some("1 Engine Row"), None, None, None, None);
        assert_eq!(format_address(&a).as_deref(), Some("home: 1 Engine Row"));
    }

    #[test]
    fn region_and_postal_code_join_with_a_space_not_a_comma() {
        let a = addr(
            None,
            Some("London"),
            Some("Greater London"),
            Some("EC1"),
            None,
        );
        assert_eq!(
            format_address(&a).as_deref(),
            Some("home: London, Greater London EC1")
        );
    }

    #[test]
    fn all_blank_address_yields_no_line() {
        let a = addr(None, None, None, None, None);
        assert_eq!(format_address(&a), None);
    }
}

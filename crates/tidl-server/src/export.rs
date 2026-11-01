//! GET /export/{slug} — download all events in a calendar as a single .ics file.
//! GET /export/contacts — download all (or a selection of) contacts as one .vcf file.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use tidl_carddav::generate_vcard;
use tidl_web::ssr_types::SessionCookieName;

use crate::{
    carddav_handlers::{record_to_vcard, VCF_CONTENT_TYPE},
    session::session_token_from_headers,
    storage::ContactRecord,
    AppState,
};

pub async fn export_calendar_ics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);
    let token = match session_token_from_headers(&headers, &cookie_name) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response(),
    };
    let username = match state.sessions.get_session_user(&token).await {
        Ok(Some(u)) => u,
        _ => return (StatusCode::UNAUTHORIZED, "Session invalid or expired").into_response(),
    };

    // Verify the calendar exists and belongs to this user.
    match state.storage.get_calendar(&username, &slug).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "Calendar not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to look up calendar for export");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    }

    let events = match state.storage.list_events(&username, &slug).await {
        Ok(evs) => evs,
        Err(e) => {
            tracing::error!(error = %e, "failed to list events for export");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    let mut all_tzones: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut all_events = String::new();
    for ev in &events {
        let (tzones, event_content) = extract_inner_ical(&ev.ical_data);
        for (tzid, block) in tzones {
            all_tzones.entry(tzid).or_insert(block);
        }
        all_events.push_str(&event_content);
    }

    let mut body = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n");
    for block in all_tzones.values() {
        body.push_str(block);
    }
    body.push_str(&all_events);
    body.push_str("END:VCALENDAR\r\n");

    // Strip characters that would break the quoted filename parameter.
    let safe_slug: String = slug
        .chars()
        .filter(|&c| c != '"' && c != '\r' && c != '\n')
        .collect();
    let disposition = format!("attachment; filename=\"{}.ics\"", safe_slug);
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(body))
    {
        Ok(r) => r,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ExportContactsQuery {
    /// Comma-separated contact ids to export. Omitted/empty exports every
    /// owned contact — backlog #89's "single contact, a selection, or all"
    /// are all the same request, just with a different (or absent) `ids`.
    ids: Option<String>,
}

/// Download the current user's contacts as one `.vcf` file, one `VCARD` per
/// contact concatenated in the same file (confirmed with the user over the
/// per-contact-file alternative — matches how Apple Contacts/Thunderbird/
/// Google export a multi-contact selection, and needs no zipping or
/// multi-download dance in the browser).
pub async fn export_contacts_vcf(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ExportContactsQuery>,
) -> Response {
    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);
    let token = match session_token_from_headers(&headers, &cookie_name) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response(),
    };
    let username = match state.sessions.get_session_user(&token).await {
        Ok(Some(u)) => u,
        _ => return (StatusCode::UNAUTHORIZED, "Session invalid or expired").into_response(),
    };

    let contacts = match state.contacts.list_accessible_contacts(&username).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to list contacts for export");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    let selected = select_contacts_for_export(contacts, query.ids.as_deref());
    let body: String = selected
        .iter()
        .map(|c| generate_vcard(&record_to_vcard(c)))
        .collect();

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, VCF_CONTENT_TYPE)
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"contacts.vcf\"",
        )
        .body(Body::from(body))
    {
        Ok(r) => r,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Filters `contacts` down to just the ids named in the comma-separated
/// `ids` query param. `None`/empty (the "export all" case) returns every
/// contact unfiltered. An id that doesn't parse or doesn't match any owned
/// contact (e.g. one deleted between page load and click) is silently
/// dropped rather than failing the whole export.
fn select_contacts_for_export(
    contacts: Vec<ContactRecord>,
    ids: Option<&str>,
) -> Vec<ContactRecord> {
    let ids = match ids {
        Some(s) if !s.trim().is_empty() => s,
        _ => return contacts,
    };
    let wanted: std::collections::HashSet<i64> = ids
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    contacts
        .into_iter()
        .filter(|c| wanted.contains(&c.id))
        .collect()
}

/// Strip the outer VCALENDAR wrapper, split content into timezone blocks
/// and event blocks (VEVENT, VTODO, etc.).
///
/// Returns `(vtimezones, event_content)` where `vtimezones` maps TZID →
/// full VTIMEZONE block text, and `event_content` is the inner components
/// minus the outer VCALENDAR headers.
fn extract_inner_ical(ical_data: &str) -> (std::collections::BTreeMap<String, String>, String) {
    let mut tzones: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut events = String::new();

    let mut in_vcal = false;
    let mut in_vtz = false;
    let mut tz_block = String::new();
    let mut tz_id = String::new();

    for line in ical_data.lines() {
        let trimmed = line.trim_end();
        let upper = trimmed.to_ascii_uppercase();

        if upper == "BEGIN:VCALENDAR" {
            in_vcal = true;
            continue;
        }
        if upper == "END:VCALENDAR" {
            in_vcal = false;
            continue;
        }
        if !in_vcal {
            continue;
        }
        if upper.starts_with("VERSION:") || upper.starts_with("PRODID:") {
            continue;
        }

        if upper == "BEGIN:VTIMEZONE" {
            in_vtz = true;
            tz_block.clear();
            tz_id.clear();
            tz_block.push_str(trimmed);
            tz_block.push_str("\r\n");
            continue;
        }
        if in_vtz {
            if upper.starts_with("TZID:") {
                tz_id = trimmed["TZID:".len()..].to_string();
            }
            if upper == "END:VTIMEZONE" {
                tz_block.push_str(trimmed);
                tz_block.push_str("\r\n");
                if !tz_id.is_empty() {
                    tzones
                        .entry(tz_id.clone())
                        .or_insert_with(|| tz_block.clone());
                }
                in_vtz = false;
            } else {
                tz_block.push_str(trimmed);
                tz_block.push_str("\r\n");
            }
            continue;
        }

        events.push_str(trimmed);
        events.push_str("\r\n");
    }

    (tzones, events)
}

#[cfg(test)]
mod tests {
    use super::{extract_inner_ical, select_contacts_for_export};
    use crate::storage::ContactRecord;

    fn sample_record(id: i64, uid: &str) -> ContactRecord {
        ContactRecord {
            id,
            uid: uid.into(),
            display_name: format!("Contact {id}"),
            given_name: None,
            family_name: None,
            middle_name: None,
            name_prefix: None,
            name_suffix: None,
            nickname: None,
            org: None,
            title: None,
            birthday: None,
            anniversary: None,
            notes: None,
            etag: "\"etag\"".into(),
            categories: vec![],
            emails: vec![],
            phones: vec![],
            urls: vec![],
            impps: vec![],
            addresses: vec![],
            photo: None,
            photo_content_type: None,
        }
    }

    #[test]
    fn select_contacts_for_export_returns_all_when_ids_is_none() {
        let contacts = vec![sample_record(1, "a"), sample_record(2, "b")];
        let selected = select_contacts_for_export(contacts, None);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn select_contacts_for_export_returns_all_when_ids_is_empty() {
        let contacts = vec![sample_record(1, "a"), sample_record(2, "b")];
        let selected = select_contacts_for_export(contacts, Some(""));
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn select_contacts_for_export_filters_to_named_ids() {
        let contacts = vec![
            sample_record(1, "a"),
            sample_record(2, "b"),
            sample_record(3, "c"),
        ];
        let selected = select_contacts_for_export(contacts, Some("1,3"));
        let ids: Vec<i64> = selected.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn select_contacts_for_export_supports_a_single_id() {
        let contacts = vec![sample_record(1, "a"), sample_record(2, "b")];
        let selected = select_contacts_for_export(contacts, Some("2"));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, 2);
    }

    #[test]
    fn select_contacts_for_export_ignores_unmatched_and_malformed_ids() {
        let contacts = vec![sample_record(1, "a")];
        let selected = select_contacts_for_export(contacts, Some("1, 99, nope"));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, 1);
    }

    #[test]
    fn extracts_vevent_strips_wrapper() {
        let input = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\nBEGIN:VEVENT\r\nUID:abc@tidl\r\nSUMMARY:Test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let (_tzones, events) = extract_inner_ical(input);
        assert!(events.contains("BEGIN:VEVENT"), "missing BEGIN:VEVENT");
        assert!(events.contains("UID:abc@tidl"), "missing UID");
        assert!(events.contains("END:VEVENT"), "missing END:VEVENT");
        assert!(!events.contains("BEGIN:VCALENDAR"), "outer wrapper leaked");
        assert!(!events.contains("VERSION:"), "VERSION leaked");
        assert!(!events.contains("PRODID:"), "PRODID leaked");
    }

    #[test]
    fn preserves_vtimezone() {
        let input = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:tz@tidl\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let (tzones, events) = extract_inner_ical(input);
        assert!(
            tzones.contains_key("Europe/Berlin"),
            "VTIMEZONE not keyed by TZID"
        );
        let tz_block = &tzones["Europe/Berlin"];
        assert!(
            tz_block.contains("BEGIN:VTIMEZONE"),
            "VTIMEZONE block missing BEGIN"
        );
        assert!(tz_block.contains("TZID:Europe/Berlin"), "TZID dropped");
        assert!(events.contains("BEGIN:VEVENT"), "VEVENT missing");
    }

    #[test]
    fn empty_calendar_returns_empty() {
        let (_tzones, events) = extract_inner_ical(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\nEND:VCALENDAR\r\n",
        );
        assert!(events.is_empty());
    }

    #[test]
    fn vtimezone_deduplicated_across_events() {
        let event = |uid: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
             BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nEND:VTIMEZONE\r\n\
             BEGIN:VEVENT\r\nUID:{uid}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        let mut all_tzones: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut all_events = String::new();
        for uid in ["a", "b", "c"] {
            let (tzones, evts) = extract_inner_ical(&event(uid));
            for (tzid, block) in tzones {
                all_tzones.entry(tzid).or_insert(block);
            }
            all_events.push_str(&evts);
        }
        // Should have exactly one VTIMEZONE regardless of event count
        assert_eq!(
            all_tzones.len(),
            1,
            "expected 1 VTIMEZONE, got {}",
            all_tzones.len()
        );
        assert!(all_events.contains("UID:a"));
        assert!(all_events.contains("UID:b"));
        assert!(all_events.contains("UID:c"));
    }
}

//! CardDAV HTTP route handlers (RFC 6352, backlog #29/#81)
//!
//! Multiple address books per user (backlog #81): each `contact_collections`
//! row — owned or shared with the current user — is exposed as its own
//! address book at `/carddav/{username}/book-{collection_id}/`, uniformly
//! for both cases (unlike `caldav_handlers.rs`'s `shared-{share_id}` alias,
//! which only applies to shared calendars — see the design spec's
//! Architecture Overview for why contacts use one scheme instead of two).
//! No MKCOL/DELETE/COPY/MOVE on the collection itself (nothing to create or
//! move — collection management stays web-UI-only) and no WebDAV locking
//! (RFC 6352 sync clients don't need it).
//!
//! * `carddav_discovery`  — redirect `.well-known/carddav` to the CardDAV root
//! * `carddav_root`       — service-level PROPFIND (lists principals)
//! * `carddav_principal`  — user-level PROPFIND (`addressbook-home-set`)
//! * `carddav_home`       — home collection, enumerates every accessible `book-{id}`
//! * `carddav_propfind`   — PROPFIND for one address-book collection
//! * `carddav_report`     — REPORT (addressbook-query / addressbook-multiget / sync-collection)
//! * `carddav_contact_*`  — GET/PUT/DELETE/PROPFIND on a single vCard
//! * `resolve_addressbook_identity` — resolves a `book-{id}` segment to
//!   `(collection_owner_username, collection_id)`, gated on the
//!   authenticated user actually being the owner or a member — mirrors
//!   `caldav_handlers.rs::resolve_calendar_identity`

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use tidl_caldav::webdav::properties;
use tidl_carddav::{
    carddav::{
        generate_report_response, parse_addressbook_multiget, parse_addressbook_query,
        AddressObject,
    },
    generate_etag, generate_propfind_response, generate_vcard, parse_propfind, parse_vcard, Depth,
    PropFindType, PropStat, Property, Response,
};
use tracing::debug;

use crate::{
    auth::BasicAuth,
    caldav_handlers::{require_same_user, username_to_resource_id, xml_response},
    storage::ContactRecord,
    AppState,
};

const MULTISTATUS: StatusCode = StatusCode::MULTI_STATUS;
pub(crate) const VCF_CONTENT_TYPE: &str = "text/vcard; charset=utf-8";

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Redirect `.well-known/carddav` to the CardDAV root.
pub async fn carddav_discovery() -> impl IntoResponse {
    (StatusCode::MOVED_PERMANENTLY, [("Location", "/carddav/")])
}

/// Catch-all for CardDAV paths deeper than the supported hierarchy.
pub async fn carddav_deep_any(
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
) -> impl IntoResponse {
    match method.as_str() {
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Service root ──────────────────────────────────────────────────────────────

pub async fn carddav_root_any(
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => carddav_root(headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// CardDAV service root — returns `current-user-principal` for discovery.
pub async fn carddav_root(
    headers: HeaderMap,
    State(_state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let principal_href = format!("/carddav/principals/{}/", auth.username);
    let response = Response {
        href: "/carddav/".to_string(),
        propstats: build_root_propstats(&propfind.kind, &principal_href),
    };

    let xml = generate_propfind_response(vec![response])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml))
}

// ── Principal collection ──────────────────────────────────────────────────────

pub async fn carddav_principals_collection_any(
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
) -> impl IntoResponse {
    match method.as_str() {
        "OPTIONS" => carddav_options().into_response(),
        "PROPFIND" | "GET" | "HEAD" => {
            let response = Response {
                href: "/carddav/principals/".to_string(),
                propstats: vec![PropStat {
                    properties: vec![Property {
                        namespace: "D".into(),
                        name: properties::RESOURCETYPE.into(),
                        value: Some("<D:collection/>".into()),
                    }],
                    status: "HTTP/1.1 200 OK".into(),
                }],
            };
            match generate_propfind_response(vec![response]) {
                Ok(xml) => xml_response(MULTISTATUS, xml).into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

// ── Principal ─────────────────────────────────────────────────────────────────

pub async fn carddav_principal_any(
    path: Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => carddav_principal(path, headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Principal resource — returns principal metadata and address-book home location.
pub async fn carddav_principal(
    Path(username): Path<String>,
    headers: HeaderMap,
    State(_state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let principal_href = format!("/carddav/principals/{}/", username);
    let home_href = format!("/carddav/{}/", username);
    let propstats =
        build_principal_propstats(&username, &propfind.kind, &principal_href, &home_href);

    let xml = generate_propfind_response(vec![Response {
        href: principal_href,
        propstats,
    }])
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml))
}

// ── Address-book home collection ──────────────────────────────────────────────

pub async fn carddav_home_any(
    path: Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => carddav_home(path, headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Home collection — lists the one address book at Depth:1. Lives at
/// `/carddav/{username}/`; not a principal resource, mirrors
/// `caldav_home`'s calendar-home-set role but with exactly one fixed child
/// (`contacts/`) rather than an enumerable list.
pub async fn carddav_home(
    Path(username): Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let home_href = format!("/carddav/{}/", username);
    let principal_href = format!("/carddav/principals/{}/", username);
    let mut responses = vec![Response {
        href: home_href.clone(),
        propstats: vec![PropStat {
            properties: vec![
                Property {
                    namespace: "D".into(),
                    name: properties::DISPLAYNAME.into(),
                    value: Some(username.clone()),
                },
                Property {
                    namespace: "D".into(),
                    name: properties::RESOURCETYPE.into(),
                    value: Some("<D:collection/>".into()),
                },
                Property {
                    namespace: "D".into(),
                    name: properties::CURRENT_USER_PRINCIPAL.into(),
                    value: Some(format!("<D:href>{}</D:href>", principal_href)),
                },
                Property {
                    namespace: "CARD".into(),
                    name: tidl_carddav::xml::properties::ADDRESSBOOK_HOME_SET.into(),
                    value: Some(format!("<D:href>{}</D:href>", home_href)),
                },
            ],
            status: "HTTP/1.1 200 OK".into(),
        }],
    }];

    if matches!(propfind.depth, Depth::One | Depth::Infinity) {
        // Guarantees at least one address book on a brand-new user's very
        // first discovery, even if they've never created a contact through
        // the web UI yet (which is otherwise the only thing that lazily
        // provisions "My Contacts" — see Plan 1's Task 7).
        state
            .contacts
            .ensure_default_contact_collection(&username)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let collections = state
            .contacts
            .list_contact_collections_for(&username)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for coll in collections {
            let alias = format!("book-{}", coll.id);
            let book_href = format!("/carddav/{}/{}/", username, alias);
            let sync_token = state
                .contacts
                .get_address_book_sync_token(coll.id)
                .await
                .unwrap_or(0);
            let display_name = if coll.owner_username == username {
                coll.name.clone()
            } else {
                format!("{} ({})", coll.name, coll.owner_username)
            };
            responses.push(Response {
                href: book_href,
                propstats: build_addressbook_propstats(&display_name, &propfind.kind, sync_token),
            });
        }
    }

    let xml =
        generate_propfind_response(responses).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(xml_response(MULTISTATUS, xml))
}

// ── Address-book collection ───────────────────────────────────────────────────

pub async fn carddav_collection_any(
    Path((username, book)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    match method.as_str() {
        "PROPFIND" => carddav_propfind(Path((username, book)), State(state), auth, headers, body)
            .await
            .into_response(),
        "REPORT" => carddav_report(Path((username, book)), State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// PROPFIND on the address-book collection — Depth:1 also lists every vCard.
pub async fn carddav_propfind(
    Path((username, book)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let coll_href = format!("/carddav/{}/{}/", username, book);
    let sync_token = state
        .contacts
        .get_address_book_sync_token(collection_id)
        .await
        .unwrap_or(0);
    let mut responses = vec![Response {
        href: coll_href.clone(),
        propstats: build_addressbook_propstats(&book, &propfind.kind, sync_token),
    }];

    if matches!(propfind.depth, Depth::One | Depth::Infinity) {
        let contacts = state
            .contacts
            .list_contacts_in_collection(collection_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        for c in contacts {
            let href = format!("{}{}.vcf", coll_href, c.uid);
            responses.push(Response {
                href,
                propstats: vec![PropStat {
                    properties: vec![
                        Property {
                            namespace: "D".into(),
                            name: properties::GETETAG.into(),
                            value: Some(c.etag.clone()),
                        },
                        Property {
                            namespace: "D".into(),
                            name: properties::GETCONTENTTYPE.into(),
                            value: Some(VCF_CONTENT_TYPE.into()),
                        },
                        Property {
                            namespace: "D".into(),
                            name: properties::RESOURCETYPE.into(),
                            value: None,
                        },
                    ],
                    status: "HTTP/1.1 200 OK".into(),
                }],
            });
        }
    }

    let xml =
        generate_propfind_response(responses).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(xml_response(MULTISTATUS, xml))
}

/// REPORT — dispatches to `addressbook-multiget`, `sync-collection`, or
/// `addressbook-query` (parsed for compliance but, like `caldav_report`'s own
/// `calendar-query` branch, not applied as a filter — real sync clients
/// drive off multiget/sync-collection; see this file's module doc).
pub async fn carddav_report(
    Path((username, book)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<axum::response::Response, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;

    if body.contains("sync-collection") {
        return carddav_sync_collection(&username, &book, collection_id, &state, &body)
            .await
            .map(IntoResponse::into_response);
    }

    let is_multiget = body.contains("addressbook-multiget");
    let coll_href = format!("/carddav/{}/{}/", username, book);

    let objects: Vec<AddressObject> = if is_multiget {
        let multiget = parse_addressbook_multiget(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
        let mut result = vec![];
        for href in &multiget.hrefs {
            let raw_uid = href
                .split('/')
                .next_back()
                .map(strip_vcf_extension)
                .unwrap_or(href.as_str());
            let uid = percent_encoding::percent_decode_str(raw_uid)
                .decode_utf8()
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| raw_uid.to_string());
            if let Ok(Some(record)) = state.contacts.get_contact_by_uid(collection_id, &uid).await {
                result.push(AddressObject {
                    href: href.clone(),
                    etag: record.etag.clone(),
                    address_data: if multiget.include_address_data {
                        Some(generate_vcard(&record_to_vcard(&record)))
                    } else {
                        None
                    },
                });
            }
        }
        result
    } else {
        let query = parse_addressbook_query(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
        let contacts = state
            .contacts
            .list_contacts_in_collection(collection_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        contacts
            .into_iter()
            .map(|c| {
                let href = format!("{}{}.vcf", coll_href, c.uid);
                let data = if query.include_address_data {
                    Some(generate_vcard(&record_to_vcard(&c)))
                } else {
                    None
                };
                AddressObject {
                    href,
                    etag: c.etag,
                    address_data: data,
                }
            })
            .collect()
    };

    let xml = generate_report_response(objects).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(xml_response(MULTISTATUS, xml).into_response())
}

/// Extract the `since` sync-token value from a `sync-collection` REPORT
/// body, if present. Searches for the token URI directly rather than the
/// XML tag, because different clients use different namespace prefixes for
/// the DAV namespace (e.g. iOS uses `A:`, not `D:`).
fn parse_sync_token_from_body(body: &str) -> Option<i64> {
    let pos = body.find("urn:tidl:sync:")?;
    body[pos + "urn:tidl:sync:".len()..]
        .split('<')
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Assemble the final `207 Multi-Status` `sync-collection` response body
/// from already-built per-resource `<D:response>` entries plus the new
/// sync token. RFC 6578 §3.6: `sync-token` is a **direct child of
/// `multistatus`**, not a property on some extra response entry for the
/// collection itself — an earlier version of this function wrongly added
/// one, which real clients (verified: Thunderbird) silently choke on,
/// reporting zero contacts despite the per-contact entries being present.
fn format_sync_collection_multistatus(response_entries: &str, sync_token: i64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav">
{response_entries}  <D:sync-token>urn:tidl:sync:{sync_token}</D:sync-token>
</D:multistatus>"#,
    )
}

/// `sync-collection` REPORT (RFC 6578) — mirrors `caldav_sync_collection`.
async fn carddav_sync_collection(
    username: &str,
    book: &str,
    collection_id: i64,
    state: &AppState,
    body: &str,
) -> Result<impl IntoResponse, StatusCode> {
    let since_token = parse_sync_token_from_body(body);

    let current_token = state
        .contacts
        .get_address_book_sync_token(collection_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let coll_href = format!("/carddav/{}/{}/", username, book);

    let response_xml: String = match since_token {
        None => {
            let contacts = state
                .contacts
                .list_contacts_in_collection(collection_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let mut entries = String::new();
            for c in contacts {
                let href = tidl_caldav::escape_xml(&format!("{}{}.vcf", coll_href, c.uid));
                entries.push_str(&format!(
                    "  <D:response>\n    <D:href>{href}</D:href>\n\
                         <D:propstat><D:prop><D:getetag>{etag}</D:getetag></D:prop>\
                     <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\n  </D:response>\n",
                    href = href,
                    etag = c.etag,
                ));
            }
            entries
        }
        Some(since) => {
            let changes = state
                .contacts
                .get_contact_sync_changes(collection_id, since, None)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let mut entries = String::new();
            for change in changes {
                let href = tidl_caldav::escape_xml(&format!("{}{}.vcf", coll_href, change.uid));
                // The logged operation is only a hint: the contacts table is
                // authoritative. Report 200-with-etag when the contact exists,
                // 404 only when it's genuinely gone — mirrors
                // `caldav_sync_collection`.
                match state
                    .contacts
                    .get_contact_by_uid(collection_id, &change.uid)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                {
                    Some(record) => {
                        entries.push_str(&format!(
                            "  <D:response>\n    <D:href>{href}</D:href>\n\
                             <D:propstat><D:prop><D:getetag>{etag}</D:getetag></D:prop>\
                             <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\n  </D:response>\n",
                            href = href,
                            etag = record.etag,
                        ));
                    }
                    None => {
                        entries.push_str(&format!(
                            "  <D:response>\n    <D:href>{href}</D:href>\n\
                             <D:status>HTTP/1.1 404 Not Found</D:status>\n  </D:response>\n",
                            href = href,
                        ));
                    }
                }
            }
            entries
        }
    };

    let xml = format_sync_collection_multistatus(&response_xml, current_token);

    Ok(xml_response(MULTISTATUS, xml))
}

// ── vCard resources ───────────────────────────────────────────────────────────

pub async fn carddav_contact_any(
    Path((username, book, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    let u = username.clone();
    let b = book.clone();
    let id = uid.clone();
    match method.as_str() {
        "GET" | "HEAD" => carddav_contact_get(Path((u, b, id)), State(state), auth)
            .await
            .into_response(),
        "PUT" => carddav_contact_put(Path((u, b, id)), State(state), auth, headers, body)
            .await
            .into_response(),
        "DELETE" => carddav_contact_delete(Path((u, b, id)), State(state), auth)
            .await
            .into_response(),
        "PROPFIND" => carddav_contact_propfind(Path((u, b, id)), State(state), auth, headers, body)
            .await
            .into_response(),
        "OPTIONS" => carddav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

pub async fn carddav_contact_get(
    Path((username, book, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;
    let uid = strip_vcf_extension(&uid).to_string();

    match state
        .contacts
        .get_contact_by_uid(collection_id, &uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(record) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                "ETag",
                HeaderValue::from_str(&record.etag)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
            headers.insert("Content-Type", HeaderValue::from_static(VCF_CONTENT_TYPE));
            let vcard = generate_vcard(&record_to_vcard(&record));
            Ok((StatusCode::OK, headers, vcard))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// PUT a vCard resource (create or update). The resource `uid` always comes
/// from the URL, matching `caldav_event_put`'s own treatment of `.ics` PUTs
/// — the vCard body's own optional `UID:` property, if present, is parsed
/// but not used as storage key, avoiding any URL/body mismatch cases.
pub async fn carddav_contact_put(
    Path((username, book, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;
    let uid = strip_vcf_extension(&uid).to_string();

    debug!(username = %username, collection_id, uid = %uid, content_length = body.len(), "carddav_contact_put");

    let fields = parse_vcard(&body).map_err(|e| {
        debug!(error = %e, "carddav_contact_put: vCard validation failed");
        StatusCode::UNPROCESSABLE_ENTITY
    })?;

    let etag = generate_etag(&body);
    let existing = state
        .contacts
        .get_contact_by_uid(collection_id, &uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(inm) = headers.get("If-None-Match").and_then(|v| v.to_str().ok()) {
        if inm == "*" && existing.is_some() {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
    }
    if let Some(if_match) = headers.get("If-Match").and_then(|v| v.to_str().ok()) {
        match &existing {
            Some(record) => {
                if record.etag != if_match && if_match != "*" {
                    return Err(StatusCode::PRECONDITION_FAILED);
                }
            }
            None if if_match != "*" => return Err(StatusCode::PRECONDITION_FAILED),
            None => {}
        }
    }

    if auth.token_read_only {
        return Err(StatusCode::FORBIDDEN);
    }

    // `owner` here is the *authenticated caller* (contact creator
    // attribution, backlog #81's Plan 1) — not necessarily the collection's
    // owner, which is what `resolve_addressbook_identity` returned as
    // `_owner` above and is intentionally unused for this.
    match state
        .contacts
        .store_contact_vcard(&username, collection_id, &uid, &fields, &etag)
        .await
    {
        Ok(()) => {}
        Err(e)
            if e.downcast_ref::<crate::storage::UidOwnershipConflict>()
                .is_some() =>
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    }

    state
        .contacts
        .bump_address_book_sync_token(collection_id, &uid, "modified")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        "ETag",
        etag.parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    let status = if existing.is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };

    Ok((status, resp_headers))
}

pub async fn carddav_contact_delete(
    Path((username, book, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;
    if auth.token_read_only {
        return Err(StatusCode::FORBIDDEN);
    }
    let uid = strip_vcf_extension(&uid).to_string();

    let deleted = state
        .contacts
        .delete_contact_by_uid(collection_id, &uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .contacts
        .bump_address_book_sync_token(collection_id, &uid, "deleted")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// PROPFIND on a single vCard resource — returns live properties only (no
/// dead-property store for contacts, unlike CalDAV's `properties` table).
pub async fn carddav_contact_propfind(
    Path((username, book, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (_owner, collection_id) = resolve_addressbook_identity(&state, &username, &book).await?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let _propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let uid_clean = strip_vcf_extension(&uid).to_string();
    let resource_path = format!("/carddav/{}/{}/{}", username, book, uid);

    let record = state
        .contacts
        .get_contact_by_uid(collection_id, &uid_clean)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some(record) = record else {
        return Err(StatusCode::NOT_FOUND);
    };

    let response = Response {
        href: resource_path,
        propstats: vec![PropStat {
            properties: vec![
                Property {
                    namespace: "D".into(),
                    name: properties::GETETAG.into(),
                    value: Some(record.etag),
                },
                Property {
                    namespace: "D".into(),
                    name: properties::GETCONTENTTYPE.into(),
                    value: Some(VCF_CONTENT_TYPE.into()),
                },
            ],
            status: "HTTP/1.1 200 OK".into(),
        }],
    };

    let xml = generate_propfind_response(vec![response])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(xml_response(MULTISTATUS, xml))
}

// ── OPTIONS ───────────────────────────────────────────────────────────────────

pub fn carddav_options() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            // RFC 6352 §10.1: the compliance-class token is literally
            // "addressbook" — unlike CalDAV's RFC 4791 "calendar-access",
            // there is no "-access" suffix. A client verifying CardDAV
            // support checks the DAV: header for this exact token; sending
            // the wrong one (found 2026-09-17, chasing an iOS setup hang
            // where every request up through this OPTIONS succeeded but
            // the client never proceeded past it) makes a strict client
            // conclude the server isn't really CardDAV-compliant, with no
            // visible error beyond a generic connection-failed dialog.
            ("DAV", "1, addressbook"),
            ("Allow", "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, REPORT"),
        ],
    )
}

// ── Address-book identity resolution (backlog #81) ─────────────────────────────

/// Extract the collection id from a `book-{id}` address-book-path segment.
/// Returns `None` for anything that isn't exactly that shape.
fn parse_book_alias(book: &str) -> Option<i64> {
    book.strip_prefix("book-")?.parse().ok()
}

/// Resolve a `book-{id}` path segment to the real `(owner_username,
/// collection_id)` it addresses, confirming `username` (the authenticated
/// caller, already proven to match the URL's own `{username}` segment by
/// `require_same_user`) actually has access — owner or member. Every
/// CardDAV handler below calls this before touching storage; by the time a
/// `collection_id` reaches a `ContactStore` method it's already been
/// access-checked, so those methods don't re-check it themselves (same
/// trust boundary `resolve_calendar_identity` establishes for calendars).
async fn resolve_addressbook_identity(
    state: &AppState,
    username: &str,
    book: &str,
) -> Result<(String, i64), StatusCode> {
    let collection_id = parse_book_alias(book).ok_or(StatusCode::NOT_FOUND)?;
    match state
        .contacts
        .contact_collection_access(username, collection_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        crate::storage::ContactCollectionAccess::Allowed { owner_username } => {
            Ok((owner_username, collection_id))
        }
        crate::storage::ContactCollectionAccess::Forbidden => Err(StatusCode::FORBIDDEN),
        crate::storage::ContactCollectionAccess::NotFound => Err(StatusCode::NOT_FOUND),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn strip_vcf_extension(uid: &str) -> &str {
    uid.strip_suffix(".vcf").unwrap_or(uid)
}

fn record_method_to_vcard(m: &crate::storage::ContactMethodRecord) -> tidl_carddav::VCardMethod {
    tidl_carddav::VCardMethod {
        value: m.value.clone(),
        kind: m.kind.clone(),
        is_primary: m.is_primary,
    }
}

pub(crate) fn record_to_vcard(record: &ContactRecord) -> tidl_carddav::VCardFields {
    tidl_carddav::VCardFields {
        uid: record.uid.clone(),
        display_name: record.display_name.clone(),
        given_name: record.given_name.clone(),
        family_name: record.family_name.clone(),
        middle_name: record.middle_name.clone(),
        name_prefix: record.name_prefix.clone(),
        name_suffix: record.name_suffix.clone(),
        nickname: record.nickname.clone(),
        org: record.org.clone(),
        title: record.title.clone(),
        birthday: record.birthday.clone(),
        anniversary: record.anniversary.clone(),
        notes: record.notes.clone(),
        categories: record.categories.clone(),
        emails: record.emails.iter().map(record_method_to_vcard).collect(),
        phones: record.phones.iter().map(record_method_to_vcard).collect(),
        urls: record.urls.iter().map(record_method_to_vcard).collect(),
        impps: record.impps.iter().map(record_method_to_vcard).collect(),
        addresses: record
            .addresses
            .iter()
            .map(|a| tidl_carddav::VCardAddress {
                kind: a.kind.clone(),
                is_primary: a.is_primary,
                street: a.street.clone(),
                city: a.city.clone(),
                region: a.region.clone(),
                postal_code: a.postal_code.clone(),
                country: a.country.clone(),
            })
            .collect(),
        photo: match (&record.photo, &record.photo_content_type) {
            (Some(bytes), Some(content_type)) => Some(tidl_carddav::VCardPhoto {
                bytes: bytes.clone(),
                content_type: content_type.clone(),
            }),
            _ => None,
        },
    }
}

/// Split `supported` against what the client actually asked for (RFC 4918
/// §9.1): `AllProp` returns everything; `Prop(requested)` must account for
/// *every* requested name, splitting into a `200` propstat for ones we have
/// and a `404` propstat for ones we don't — silently dropping an
/// unrecognised requested property (as `carddav_root` used to) leaves some
/// clients unable to complete discovery, since they never learn the
/// property isn't coming.
fn split_found_propstats(kind: &PropFindType, supported: Vec<Property>) -> Vec<PropStat> {
    match kind {
        PropFindType::PropName => vec![],
        PropFindType::AllProp => vec![PropStat {
            properties: supported,
            status: "HTTP/1.1 200 OK".into(),
        }],
        PropFindType::Prop(requested) => {
            let supported_map: std::collections::HashMap<String, Property> = supported
                .into_iter()
                .map(|p| (p.name.to_lowercase(), p))
                .collect();
            let mut found: Vec<Property> = vec![];
            let mut not_found: Vec<Property> = vec![];
            for name in requested {
                // Compare case-insensitively, but echo back `name` verbatim
                // (its original case) in the not-found case — see
                // `local_name_preserving_case`'s doc comment in tidl-caldav.
                match supported_map.get(&name.to_lowercase()) {
                    Some(prop) => found.push(prop.clone()),
                    None => not_found.push(Property {
                        namespace: "D".into(),
                        name: name.clone(),
                        value: None,
                    }),
                }
            }
            let mut propstats = vec![];
            if !found.is_empty() {
                propstats.push(PropStat {
                    properties: found,
                    status: "HTTP/1.1 200 OK".into(),
                });
            }
            if !not_found.is_empty() {
                propstats.push(PropStat {
                    properties: not_found,
                    status: "HTTP/1.1 404 Not Found".into(),
                });
            }
            propstats
        }
    }
}

/// Build the `Vec<PropStat>` for the CardDAV service root (`/carddav/`).
fn build_root_propstats(kind: &PropFindType, principal_href: &str) -> Vec<PropStat> {
    split_found_propstats(
        kind,
        vec![
            Property {
                namespace: "D".into(),
                name: properties::RESOURCETYPE.into(),
                value: Some("<D:collection/>".into()),
            },
            Property {
                namespace: "D".into(),
                name: properties::CURRENT_USER_PRINCIPAL.into(),
                value: Some(format!("<D:href>{}</D:href>", principal_href)),
            },
        ],
    )
}

/// Build the `Vec<PropStat>` for a principal (user) resource.
fn build_principal_propstats(
    username: &str,
    kind: &PropFindType,
    principal_href: &str,
    home_href: &str,
) -> Vec<PropStat> {
    let supported: Vec<Property> = vec![
        Property { namespace: "D".into(), name: properties::DISPLAYNAME.into(), value: Some(username.to_string()) },
        Property { namespace: "D".into(), name: properties::RESOURCETYPE.into(), value: Some("<D:principal/>".into()) },
        Property {
            namespace: "D".into(),
            name: properties::CURRENT_USER_PRINCIPAL.into(),
            value: Some(format!("<D:href>{}</D:href>", principal_href)),
        },
        Property {
            namespace: "D".into(),
            name: properties::PRINCIPAL_URL.into(),
            value: Some(format!("<D:href>{}</D:href>", principal_href)),
        },
        Property {
            namespace: "D".into(),
            name: properties::PRINCIPAL_COLLECTION_SET.into(),
            value: Some("<D:href>/carddav/principals/</D:href>".into()),
        },
        Property {
            namespace: "D".into(),
            name: properties::RESOURCE_ID.into(),
            value: Some(format!("<D:href>{}</D:href>", username_to_resource_id(username))),
        },
        Property {
            namespace: "D".into(),
            name: properties::SUPPORTED_REPORT_SET.into(),
            value: Some(
                "<D:supported-report><D:report><CARD:addressbook-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><CARD:addressbook-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .into(),
            ),
        },
        Property {
            namespace: "CARD".into(),
            name: tidl_carddav::xml::properties::ADDRESSBOOK_HOME_SET.into(),
            value: Some(format!("<D:href>{}</D:href>", home_href)),
        },
    ];

    split_found_propstats(kind, supported)
}

/// Build a `Vec<PropStat>` for the address-book collection PROPFIND response.
fn build_addressbook_propstats(
    display_name: &str,
    kind: &PropFindType,
    sync_token: i64,
) -> Vec<PropStat> {
    let sync_token_uri = format!("urn:tidl:sync:{}", sync_token);
    let ctag = format!("\"{}\"", sync_token);

    let supported: Vec<Property> = vec![
        Property { namespace: "D".into(), name: properties::DISPLAYNAME.into(), value: Some(display_name.to_string()) },
        Property { namespace: "D".into(), name: properties::RESOURCETYPE.into(), value: Some("<D:collection/><CARD:addressbook/>".into()) },
        Property { namespace: "D".into(), name: properties::GETCONTENTTYPE.into(), value: Some(VCF_CONTENT_TYPE.into()) },
        Property { namespace: "D".into(), name: properties::GETETAG.into(), value: Some(ctag.clone()) },
        Property { namespace: "D".into(), name: properties::SYNC_TOKEN.into(), value: Some(sync_token_uri) },
        Property { namespace: "CS".into(), name: properties::GETCTAG.into(), value: Some(ctag) },
        Property {
            namespace: "CARD".into(),
            name: tidl_carddav::xml::properties::SUPPORTED_ADDRESS_DATA.into(),
            value: Some(r#"<CARD:address-data-type content-type="text/vcard" version="3.0"/>"#.into()),
        },
        Property {
            namespace: "D".into(),
            name: "current-user-privilege-set".into(),
            value: Some(
                "<D:privilege><D:all/></D:privilege>\
                 <D:privilege><D:read/></D:privilege>\
                 <D:privilege><D:write/></D:privilege>\
                 <D:privilege><D:write-properties/></D:privilege>\
                 <D:privilege><D:write-content/></D:privilege>"
                    .into(),
            ),
        },
        Property {
            namespace: "D".into(),
            name: properties::SUPPORTED_REPORT_SET.into(),
            value: Some(
                "<D:supported-report><D:report><CARD:addressbook-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><CARD:addressbook-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .into(),
            ),
        },
    ];

    split_found_propstats(kind, supported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{ContactMethodRecord, ContactRecord};

    #[test]
    fn carddav_options_advertises_the_rfc6352_addressbook_compliance_token() {
        // Regression (found 2026-09-17 chasing an iOS Contacts setup hang):
        // this used to send "addressbook-access" (copying CalDAV's
        // "calendar-access" by analogy), but RFC 6352 §10.1 requires the
        // exact literal token "addressbook" with no "-access" suffix. A
        // client checking for that exact token to confirm CardDAV support
        // never finds it in "addressbook-access" — `.contains("addressbook")`
        // would wrongly pass on the buggy value too, so this checks for an
        // exact comma-separated token match instead.
        let response = carddav_options().into_response();
        let dav_header = response
            .headers()
            .get("DAV")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let tokens: Vec<&str> = dav_header.split(',').map(|s| s.trim()).collect();
        assert!(
            tokens.contains(&"addressbook"),
            "DAV header must include the exact token \"addressbook\" (got: {dav_header:?})"
        );
    }

    #[test]
    fn parse_book_alias_extracts_the_collection_id() {
        assert_eq!(parse_book_alias("book-42"), Some(42));
        assert_eq!(parse_book_alias("book-1"), Some(1));
    }

    #[test]
    fn parse_book_alias_rejects_anything_else() {
        assert_eq!(parse_book_alias("book-"), None);
        assert_eq!(parse_book_alias("book-abc"), None);
        assert_eq!(parse_book_alias("contacts"), None);
        assert_eq!(parse_book_alias("book-42-extra"), None);
    }

    #[test]
    fn parse_sync_token_from_body_extracts_the_numeric_token() {
        let body = r#"<D:sync-collection xmlns:D="DAV:"><D:sync-token>urn:tidl:sync:7</D:sync-token></D:sync-collection>"#;
        assert_eq!(parse_sync_token_from_body(body), Some(7));
    }

    #[test]
    fn parse_sync_token_from_body_tolerates_a_non_d_namespace_prefix() {
        // iOS uses "A:" (or another arbitrary prefix) for the DAV namespace —
        // matching on the token URI rather than the tag name must still work.
        let body = r#"<A:sync-collection xmlns:A="DAV:"><A:sync-token>urn:tidl:sync:3</A:sync-token></A:sync-collection>"#;
        assert_eq!(parse_sync_token_from_body(body), Some(3));
    }

    #[test]
    fn parse_sync_token_from_body_returns_none_when_absent_or_empty() {
        let body = r#"<D:sync-collection xmlns:D="DAV:"><D:sync-token></D:sync-token></D:sync-collection>"#;
        assert_eq!(parse_sync_token_from_body(body), None);
        assert_eq!(parse_sync_token_from_body("<D:sync-collection/>"), None);
    }

    #[test]
    fn format_sync_collection_multistatus_puts_sync_token_as_a_direct_multistatus_child() {
        // Regression (found via real-client testing, 2026-09-15): an earlier
        // version wrapped an extra <D:response> for the collection itself
        // around the sync-token, which real clients (Thunderbird) silently
        // discard the whole sync over — RFC 6578 §3.6 requires it as a
        // direct child of <D:multistatus>, not a property inside a propstat.
        let entry = "  <D:response>\n    <D:href>/carddav/alice/contacts/u1.vcf</D:href>\n\
                      <D:propstat><D:prop><D:getetag>\"e1\"</D:getetag></D:prop>\
                      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\n  </D:response>\n";
        let xml = format_sync_collection_multistatus(entry, 5);

        assert!(xml.contains("<D:sync-token>urn:tidl:sync:5</D:sync-token>"));
        assert!(
            !xml.contains("<D:propstat><D:prop><D:sync-token>"),
            "sync-token must never appear nested inside a propstat"
        );
        // Exactly one <D:response> — only the real contact entry, no
        // synthetic entry for the collection itself.
        assert_eq!(xml.matches("<D:response>").count(), 1);
        assert_eq!(
            xml.matches("</D:multistatus>").count(),
            1,
            "the document must still be a single well-formed multistatus"
        );
    }

    #[test]
    fn format_sync_collection_multistatus_is_well_formed_with_no_entries() {
        let xml = format_sync_collection_multistatus("", 0);
        assert!(xml.contains("<D:sync-token>urn:tidl:sync:0</D:sync-token>"));
        assert_eq!(xml.matches("<D:response>").count(), 0);
    }

    #[test]
    fn strip_vcf_extension_strips_only_the_suffix() {
        assert_eq!(strip_vcf_extension("abc.vcf"), "abc");
        assert_eq!(strip_vcf_extension("abc"), "abc");
        assert_eq!(strip_vcf_extension("a.vcf.vcf"), "a.vcf");
    }

    #[test]
    fn record_to_vcard_carries_every_field_and_method_through() {
        let record = ContactRecord {
            id: 1,
            uid: "uid-1".into(),
            display_name: "Ada Lovelace".into(),
            given_name: Some("Ada".into()),
            family_name: Some("Lovelace".into()),
            middle_name: Some("Augusta".into()),
            name_prefix: None,
            name_suffix: None,
            nickname: Some("Lady A".into()),
            org: Some("Analytical Engines".into()),
            title: Some("Mathematician".into()),
            birthday: Some("1815-12-10".into()),
            anniversary: None,
            notes: Some("First programmer".into()),
            etag: "\"etag-1\"".into(),
            categories: vec!["VIP".into()],
            emails: vec![ContactMethodRecord {
                value: "ada@example.com".into(),
                kind: "work".into(),
                is_primary: true,
            }],
            phones: vec![ContactMethodRecord {
                value: "+44 20 1234".into(),
                kind: "mobile".into(),
                is_primary: false,
            }],
            urls: vec![ContactMethodRecord {
                value: "https://ada.example".into(),
                kind: "home".into(),
                is_primary: false,
            }],
            impps: vec![],
            addresses: vec![crate::storage::ContactAddressRecord {
                kind: "home".into(),
                is_primary: true,
                street: None,
                city: Some("London".into()),
                region: None,
                postal_code: None,
                country: Some("UK".into()),
            }],
            photo: Some(vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]),
            photo_content_type: Some("image/jpeg".into()),
        };

        let vcard = record_to_vcard(&record);
        assert_eq!(vcard.uid, "uid-1");
        assert_eq!(vcard.display_name, "Ada Lovelace");
        assert_eq!(vcard.addresses.len(), 1);
        assert_eq!(vcard.addresses[0].city.as_deref(), Some("London"));
        assert_eq!(vcard.nickname.as_deref(), Some("Lady A"));
        assert_eq!(vcard.categories, vec!["VIP".to_string()]);
        assert_eq!(vcard.emails.len(), 1);
        assert_eq!(vcard.emails[0].value, "ada@example.com");
        assert!(vcard.emails[0].is_primary);
        assert_eq!(vcard.phones.len(), 1);
        assert_eq!(vcard.phones[0].kind, "mobile");
        assert_eq!(vcard.urls.len(), 1);
        let photo = vcard.photo.as_ref().expect("photo present");
        assert_eq!(photo.content_type, "image/jpeg");
        assert_eq!(photo.bytes, vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);

        // The vCard generated from a record must itself parse back cleanly —
        // exercises the same round-trip a GET response relies on.
        let text = generate_vcard(&vcard);
        assert!(text.contains("PHOTO;ENCODING=b;TYPE=JPEG:"));
        let reparsed = parse_vcard(&text).unwrap();
        assert_eq!(reparsed.display_name, "Ada Lovelace");
        assert_eq!(reparsed.emails.len(), 1);
        assert_eq!(
            reparsed.photo.as_ref().map(|p| p.bytes.clone()),
            Some(vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10])
        );
    }

    #[test]
    fn build_addressbook_propstats_all_prop_includes_sync_token_and_ctag() {
        let propstats = build_addressbook_propstats("alice", &PropFindType::AllProp, 5);
        assert_eq!(propstats.len(), 1);
        let names: Vec<&str> = propstats[0]
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(names.contains(&properties::SYNC_TOKEN));
        assert!(names.contains(&properties::GETCTAG));
        let sync_prop = propstats[0]
            .properties
            .iter()
            .find(|p| p.name == properties::SYNC_TOKEN)
            .unwrap();
        assert_eq!(sync_prop.value.as_deref(), Some("urn:tidl:sync:5"));
    }

    #[test]
    fn build_addressbook_propstats_prop_splits_found_and_not_found() {
        let requested = PropFindType::Prop(vec![
            "displayname".to_string(),
            "nonexistent-prop".to_string(),
        ]);
        let propstats = build_addressbook_propstats("alice", &requested, 0);
        assert_eq!(propstats.len(), 2);
        assert_eq!(propstats[0].status, "HTTP/1.1 200 OK");
        assert_eq!(propstats[1].status, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn build_principal_propstats_includes_addressbook_home_set() {
        let propstats = build_principal_propstats(
            "alice",
            &PropFindType::AllProp,
            "/carddav/principals/alice/",
            "/carddav/alice/",
        );
        let found = propstats[0]
            .properties
            .iter()
            .find(|p| p.name == tidl_carddav::xml::properties::ADDRESSBOOK_HOME_SET)
            .unwrap();
        assert_eq!(
            found.value.as_deref(),
            Some("<D:href>/carddav/alice/</D:href>")
        );
    }

    #[test]
    fn split_found_propstats_accounts_for_every_requested_name() {
        // Regression (found via a real iOS sync attempt, 2026-09-17):
        // carddav_root used to return a fixed property set regardless of
        // what was requested, silently dropping anything it didn't already
        // hardcode (e.g. principal-URL) instead of returning a 404 propstat
        // for it — some clients abandon discovery when a requested property
        // just never shows up in the response at all.
        let requested = PropFindType::Prop(vec![
            "current-user-principal".to_string(),
            "principal-URL".to_string(),
            "resourcetype".to_string(),
        ]);
        let supported = vec![
            Property {
                namespace: "D".into(),
                name: "resourcetype".into(),
                value: Some("<D:collection/>".into()),
            },
            Property {
                namespace: "D".into(),
                name: "current-user-principal".into(),
                value: Some("x".into()),
            },
        ];

        let propstats = split_found_propstats(&requested, supported);

        let all_names: Vec<&str> = propstats
            .iter()
            .flat_map(|ps| ps.properties.iter().map(|p| p.name.as_str()))
            .collect();
        for name in ["current-user-principal", "principal-URL", "resourcetype"] {
            assert!(
                all_names.contains(&name),
                "{name} missing from response entirely"
            );
        }
        let not_found = propstats
            .iter()
            .find(|ps| ps.status.contains("404"))
            .unwrap();
        assert_eq!(not_found.properties.len(), 1);
        assert_eq!(not_found.properties[0].name, "principal-URL");
    }

    #[test]
    fn carddav_root_propfind_end_to_end_preserves_requested_case() {
        // Unlike the test above (which hand-builds `PropFindType::Prop` and
        // so never exercises the parser), this goes through `parse_propfind`
        // for real — the exact request body a real iOS client sent
        // (2026-09-17) — to close the coverage gap that let the case bug
        // ship in the first place: `split_found_propstats` alone was never
        // wrong, only the case-destroying lowercase in `parse_propfind` was.
        let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<A:propfind xmlns:A="DAV:">
  <A:prop>
    <A:current-user-principal/>
    <A:principal-URL/>
    <A:resourcetype/>
  </A:prop>
</A:propfind>"#;
        let propfind = parse_propfind(body, Some("0")).unwrap();

        let propstats = build_root_propstats(&propfind.kind, "/carddav/principals/alice/");

        let not_found = propstats
            .iter()
            .find(|ps| ps.status.contains("404"))
            .unwrap();
        assert_eq!(
            not_found.properties[0].name, "principal-URL",
            "must echo the client's exact requested casing, not a lowercased mismatch"
        );
    }

    #[test]
    fn build_root_propstats_all_prop_includes_current_user_principal() {
        let propstats = build_root_propstats(&PropFindType::AllProp, "/carddav/principals/alice/");
        assert_eq!(propstats.len(), 1);
        let found = propstats[0]
            .properties
            .iter()
            .find(|p| p.name == properties::CURRENT_USER_PRINCIPAL)
            .unwrap();
        assert_eq!(
            found.value.as_deref(),
            Some("<D:href>/carddav/principals/alice/</D:href>")
        );
    }
}

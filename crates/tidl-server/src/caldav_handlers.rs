//! CalDAV HTTP route handlers
//!
//! Each handler has a single, clear responsibility:
//!
//! * `caldav_discovery`   — redirect .well-known to the CalDAV root
//! * `caldav_root`        — service-level PROPFIND (lists principals)
//! * `caldav_principal`   — user-level PROPFIND (lists calendars)
//! * `caldav_calendar`    — calendar collection GET
//! * `caldav_calendar_post` — create a new event via POST
//! * `caldav_event_get`   — fetch a single .ics object
//! * `caldav_event_put`   — create or update a .ics object
//! * `caldav_event_delete_or_subcollection` — delete a .ics object or sub-collection
//! * `caldav_propfind`    — PROPFIND for a calendar collection
//! * `caldav_report`      — REPORT (calendar-query / calendar-multiget)

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use tidl_caldav::{
    caldav::{
        generate_report_response, parse_calendar_multiget, parse_calendar_query, CalendarObject,
    },
    change::ChangeKind,
    etag::generate_etag,
    webdav::{
        generate_propfind_response, generate_proppatch_response, parse_propfind, parse_proppatch,
        properties, Depth, PropFindType, PropPatchOp, PropStat, Property, Response,
    },
};
use tracing::debug;

use crate::{
    auth::BasicAuth,
    if_header::{
        check_if_header, lock_token_in_if_header, overwrite_status, parse_destination,
        parse_destination_event,
    },
    storage::LockInfo,
    AppState,
};

const MULTISTATUS: StatusCode = StatusCode::MULTI_STATUS;
const XML_CONTENT_TYPE: &str = "application/xml; charset=utf-8";
const ICS_CONTENT_TYPE: &str = "text/calendar; charset=utf-8";

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Redirect `.well-known/caldav` to the CalDAV root. `.well-known/carddav`
/// is handled separately by `carddav_handlers::carddav_discovery` (backlog #29).
pub async fn caldav_discovery() -> impl IntoResponse {
    (StatusCode::MOVED_PERMANENTLY, [("Location", "/caldav/")])
}

/// Catch-all for CalDAV paths deeper than the supported hierarchy.
///
/// MKCOL/PUT with missing intermediate collections → 409 Conflict (RFC 4918).
/// Everything else → 404 Not Found.
pub async fn caldav_deep_any(
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
) -> impl IntoResponse {
    match method.as_str() {
        "MKCOL" | "MKCALENDAR" | "PUT" => StatusCode::CONFLICT.into_response(),
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Service root ──────────────────────────────────────────────────────────────

/// Dispatch wrapper for the CalDAV service root (`/caldav/`).
pub async fn caldav_root_any(
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    debug!(method = %method, body = %body, "caldav_root request");
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => caldav_root(headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// CalDAV service root — returns `current-user-principal` for discovery.
pub async fn caldav_root(
    headers: HeaderMap,
    State(_state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let _propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let principal_href = format!("/caldav/principals/{}/", auth.username);
    let response = Response {
        href: "/caldav/".to_string(),
        propstats: vec![PropStat {
            properties: vec![
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
            status: "HTTP/1.1 200 OK".into(),
        }],
    };

    let xml = generate_propfind_response(vec![response])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml))
}

// ── Principal collection ──────────────────────────────────────────────────────

/// Dispatch wrapper for `/caldav/principals/` — the principal collection.
pub async fn caldav_principals_collection_any(
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
) -> impl IntoResponse {
    match method.as_str() {
        "OPTIONS" => caldav_options().into_response(),
        "PROPFIND" | "GET" | "HEAD" => {
            // Return a minimal collection response.
            let response = Response {
                href: "/caldav/principals/".to_string(),
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

/// Dispatch wrapper for `/caldav/principals/{username}/`.
pub async fn caldav_principal_any(
    path: Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    debug!(method = %method, path = %path.0, body = %body, "caldav_principal request");
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => caldav_principal(path, headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Principal resource — returns principal metadata and calendar home location.
///
/// Lives at `/caldav/principals/{username}/`.  Does NOT enumerate calendars;
/// clients follow `calendar-home-set` to discover those.
pub async fn caldav_principal(
    Path(username): Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let email = state
        .users
        .find_active_by_username(&username)
        .await
        .ok()
        .flatten()
        .map(|u| u.email)
        .unwrap_or_default();

    let principal_href = format!("/caldav/principals/{}/", username);
    let home_href = format!("/caldav/{}/", username);
    let propstats = build_principal_propstats(
        &username,
        &email,
        &propfind.kind,
        &principal_href,
        &home_href,
    );

    let xml = generate_propfind_response(vec![Response {
        href: principal_href,
        propstats,
    }])
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml))
}

// ── Calendar home collection ──────────────────────────────────────────────────

/// Dispatch wrapper for `/caldav/{username}/` — the calendar home collection.
pub async fn caldav_home_any(
    path: Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    match method.as_str() {
        "PROPFIND" | "GET" | "HEAD" => caldav_home(path, headers, State(state), auth, body)
            .await
            .into_response(),
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Calendar home collection — lists calendars at Depth:1.
///
/// Lives at `/caldav/{username}/`.  Not a principal resource; it is a plain
/// WebDAV collection that acts as the root of the user's calendar hierarchy.
pub async fn caldav_home(
    Path(username): Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let home_href = format!("/caldav/{}/", username);
    let principal_href = format!("/caldav/principals/{}/", username);
    let mut responses = vec![];

    // Home collection self-resource.
    responses.push(Response {
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
                    namespace: "C".into(),
                    name: properties::CALENDAR_HOME_SET.into(),
                    value: Some(format!("<D:href>{}</D:href>", home_href)),
                },
            ],
            status: "HTTP/1.1 200 OK".into(),
        }],
    });

    // Include child calendars if Depth: 1.
    if matches!(propfind.depth, Depth::One | Depth::Infinity) {
        let calendars = state
            .storage
            .list_calendars(&username)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for cal in calendars {
            let cal_href = format!("/caldav/{}/{}/", username, cal.name);
            let resource_path = format!("/caldav/{}/{}", username, cal.name);
            let sync_token = state
                .storage
                .get_sync_token(&username, &cal.name)
                .await
                .unwrap_or(1);
            let dead_props = state
                .storage
                .get_properties(&username, &resource_path)
                .await
                .unwrap_or_default();
            let cal_propstats = build_calendar_propstats(
                &cal.name,
                &cal.display_name,
                &propfind.kind,
                sync_token,
                &dead_props,
                &username,
                false,
            );
            responses.push(Response {
                href: cal_href,
                propstats: cal_propstats,
            });
        }

        // Calendars shared with this user by another owner — exposed as a
        // synthetic "shared-{share_id}" resource under this user's own
        // home-set (see backlog #41). Keyed by share-grant ID, not the
        // owner's calendar name, so it survives the owner renaming their
        // calendar.
        let shares = state
            .storage
            .list_shared_with_user(&username)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for share in shares {
            let alias = format!("shared-{}", share.id);
            let cal_href = format!("/caldav/{}/{}/", username, alias);
            let sync_token = state
                .storage
                .get_sync_token(&share.owner_username, &share.calendar_name)
                .await
                .unwrap_or(1);
            let owner_resource_path =
                format!("/caldav/{}/{}", share.owner_username, share.calendar_name,);
            let dead_props = state
                .storage
                .get_properties(&share.owner_username, &owner_resource_path)
                .await
                .unwrap_or_default();
            let shared_display_name = Some(format!(
                "{} ({})",
                share
                    .calendar_display_name
                    .unwrap_or_else(|| share.calendar_name.clone()),
                share.owner_username,
            ));
            let cal_propstats = build_calendar_propstats(
                &alias,
                &shared_display_name,
                &propfind.kind,
                sync_token,
                &dead_props,
                &username,
                share.permission != "read-write",
            );
            responses.push(Response {
                href: cal_href,
                propstats: cal_propstats,
            });
        }
    }

    let xml =
        generate_propfind_response(responses).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml))
}

// ── Calendar collection ───────────────────────────────────────────────────────

/// Unified handler for all methods on a calendar collection path.
///
/// axum 0.8's `MethodFilter` does not support custom verbs like PROPFIND or
/// REPORT, so we dispatch on the method string ourselves.
pub async fn caldav_collection_any(
    path: Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    let method_str = method.as_str();
    match method_str {
        "PROPFIND" => caldav_propfind(path, State(state), auth, headers, body)
            .await
            .into_response(),
        "REPORT" => caldav_report(path, State(state), auth, body)
            .await
            .into_response(),
        "POST" => caldav_calendar_post(path, State(state), auth, body)
            .await
            .into_response(),
        "GET" | "HEAD" => caldav_calendar(path, State(state), auth)
            .await
            .into_response(),
        "MKCOL" | "MKCALENDAR" => caldav_mkcol(path, State(state), auth, body)
            .await
            .into_response(),
        "DELETE" => caldav_collection_delete(path, State(state), auth)
            .await
            .into_response(),
        "COPY" => caldav_collection_copy(path, State(state), auth, headers)
            .await
            .into_response(),
        "MOVE" => caldav_collection_move(path, State(state), auth, headers)
            .await
            .into_response(),
        "PROPPATCH" => {
            let Path((ref u, ref c)) = path;
            let resource_path = format!("/caldav/{}/{}", u, c);
            let username = u.clone();
            caldav_collection_proppatch(resource_path, username, State(state), auth, body)
                .await
                .into_response()
        }
        "LOCK" => {
            let Path((ref u, ref c)) = path;
            let resource_path = format!("/caldav/{}/{}", u, c);
            caldav_lock(resource_path, u.clone(), State(state), auth, headers, body)
                .await
                .into_response()
        }
        "UNLOCK" => {
            let Path((ref u, ref c)) = path;
            let resource_path = format!("/caldav/{}/{}", u, c);
            caldav_unlock(resource_path, u.clone(), State(state), auth, headers)
                .await
                .into_response()
        }
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// GET on a calendar collection — auto-creates and returns a description.
pub async fn caldav_calendar(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (username, calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Read,
    )
    .await?;

    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        state
            .storage
            .create_calendar(&username, &calendar, None)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok((StatusCode::OK, format!("{}/{}", username, calendar)))
}

/// MKCOL / MKCALENDAR — create a new calendar collection.
///
/// Returns `201 Created` on success, `405 Method Not Allowed` if the
/// collection already exists (RFC 4918 §9.3), `415 Unsupported Media Type`
/// for an unrecognised MKCOL body (MKCALENDAR XML bodies are accepted),
/// or `403 Forbidden` if the authenticated user does not own the target path.
pub async fn caldav_mkcol(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    // RFC 4918 §9.3: MKCOL with an unrecognised request body MUST fail with 415.
    // RFC 4791 §5.3.1: MKCALENDAR may carry an XML body with calendar properties —
    // we accept and ignore those properties (just extract displayname if present).
    let display_name: Option<String> = if !body.is_empty() {
        if body.trim_start().starts_with('<') {
            // XML body (MKCALENDAR or extended MKCOL) — extract displayname if present.
            extract_displayname_from_mkcalendar(&body)
        } else {
            // Non-XML, non-empty body on MKCOL → 415.
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
        }
    } else {
        None
    };

    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        // RFC 4918 §9.3: MKCOL on an existing resource MUST fail with 405.
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    state
        .storage
        .create_calendar(&username, &calendar, display_name.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::Cals,
    )
    .await;

    Ok(StatusCode::CREATED)
}

/// Extract the `<displayname>` value from a MKCALENDAR or extended-MKCOL XML body.
///
/// Handles any namespace prefix (e.g. `<D:displayname>`, `<DAV:displayname>`,
/// `<displayname>`).  Returns `None` if not found or empty.
fn extract_displayname_from_mkcalendar(body: &str) -> Option<String> {
    // Find any tag whose local name is "displayname" — look for ":displayname>" or
    // "<displayname>" (case-insensitive), then read the text content up to the next `<`.
    let lower = body.to_lowercase();
    let marker = "displayname>";
    let pos = lower.find(marker)?;
    // Make sure the char before "displayname>" is either `<` (no prefix) or `:` (prefix).
    let before = if pos == 0 {
        b'\0'
    } else {
        lower.as_bytes()[pos - 1]
    };
    if before != b'<' && before != b':' {
        return None;
    }
    let after_tag = &body[pos + marker.len()..];
    let end = after_tag.find('<')?;
    let name = after_tag[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// DELETE a calendar collection and all its events.
///
/// Returns `204 No Content` on success, `404 Not Found` if the collection
/// does not exist, or `403 Forbidden` if the user does not own it.
pub async fn caldav_collection_delete(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .storage
        .delete_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::Cals,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// COPY a calendar collection (RFC 4918 §9.8).
pub async fn caldav_collection_copy(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let (dst_username, dst_calendar, overwrite) =
        parse_destination(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
    // A user may only copy into their own namespace.
    require_same_user(&auth, &dst_username)?;

    // Source must exist.
    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let dst_cal = state
        .storage
        .get_calendar(&dst_username, &dst_calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let dst_exists = dst_cal.is_some();

    if let Some(cal) = &dst_cal {
        if cal.read_only {
            match &cal.sync_user {
                Some(su) if su == &auth.username => {}
                _ => return Err(StatusCode::FORBIDDEN),
            }
        }
    }

    let status = overwrite_status(dst_exists, overwrite)?;
    if dst_exists {
        state
            .storage
            .delete_calendar(&dst_username, &dst_calendar)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Depth: 0 = shallow (collection only), anything else = deep (RFC 4918 §9.8).
    let deep = headers
        .get("Depth")
        .and_then(|v| v.to_str().ok())
        .map(|v| v != "0")
        .unwrap_or(true);

    state
        .storage
        .copy_calendar(&username, &calendar, &dst_username, &dst_calendar, deep)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &dst_username,
        &dst_calendar,
        ChangeKind::All,
    )
    .await;

    Ok(status)
}

/// MOVE a calendar collection (RFC 4918 §9.9).
pub async fn caldav_collection_move(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let (dst_username, dst_calendar, overwrite) =
        parse_destination(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
    require_same_user(&auth, &dst_username)?;

    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let dst_exists = state
        .storage
        .get_calendar(&dst_username, &dst_calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    let status = overwrite_status(dst_exists, overwrite)?;
    if dst_exists {
        state
            .storage
            .delete_calendar(&dst_username, &dst_calendar)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // MOVE always moves everything (RFC 4918 §9.9).
    state
        .storage
        .copy_calendar(&username, &calendar, &dst_username, &dst_calendar, true)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .storage
        .delete_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::All,
    )
    .await;
    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &dst_username,
        &dst_calendar,
        ChangeKind::All,
    )
    .await;

    Ok(status)
}

/// OPTIONS — advertise WebDAV and CalDAV capabilities.
pub fn caldav_options() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            ("DAV", "1, 2, calendar-access"),
            (
                "Allow",
                "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, REPORT, MKCOL, MKCALENDAR, COPY, MOVE, LOCK, UNLOCK",
            ),
        ],
    )
}

// ── Locking ───────────────────────────────────────────────────────────────────

/// LOCK — acquire or refresh an exclusive/shared write lock (RFC 4918 §9.10).
///
/// - If `If:` header contains a lock token → refresh that lock.
/// - Otherwise create a new lock.
///
/// Returns `200 OK` for existing resource or refresh, `201 Created` for unmapped URL.
pub async fn caldav_lock(
    resource_path: String,
    username: String,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // Only allow locking resources in the authenticated user's own namespace.
    if auth.username != username {
        return StatusCode::FORBIDDEN.into_response();
    }

    let timeout_secs: i64 = headers
        .get("Timeout")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Second-"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);

    // RFC 4918 §9.10.2: If: header with a lock token → lock refresh.
    if let Some(if_hdr) = headers.get("If").and_then(|v| v.to_str().ok()) {
        let token_opt = if_hdr
            .find("opaquelocktoken:")
            .map(|i| &if_hdr[i..])
            .and_then(|s| s.find('>').map(|j| s[..j].to_string()));

        if let Some(token) = token_opt {
            match state.storage.refresh_lock(&token, timeout_secs).await {
                Ok(true) => {
                    let lock = match state.storage.get_lock(&token).await {
                        Ok(Some(l)) => l,
                        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    };
                    let xml = generate_lock_response(&lock, timeout_secs);
                    let mut resp_headers = HeaderMap::new();
                    let lock_token_val = match format!("<{}>", token).parse() {
                        Ok(v) => v,
                        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    };
                    resp_headers.insert("Lock-Token", lock_token_val);
                    resp_headers.insert("Content-Type", HeaderValue::from_static(XML_CONTENT_TYPE));
                    return (StatusCode::OK, resp_headers, xml).into_response();
                }
                Ok(false) => return StatusCode::PRECONDITION_FAILED.into_response(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    }

    // New lock request.
    let scope = if body.contains("<shared") {
        "shared"
    } else {
        "exclusive"
    };
    let depth = headers
        .get("Depth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0");

    // Check for conflicting locks.
    let existing = match state.storage.get_locks_for_resource(&resource_path).await {
        Ok(v) => v,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for lock in &existing {
        // Any existing exclusive lock conflicts, as does adding exclusive over shared.
        if lock.scope == "exclusive" || scope == "exclusive" {
            return StatusCode::LOCKED.into_response();
        }
    }

    let token = match state
        .storage
        .create_lock(
            &username,
            &resource_path,
            scope,
            depth,
            Some(&body),
            timeout_secs,
        )
        .await
    {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let lock_info = LockInfo {
        token: token.clone(),
        username,
        resource_path: resource_path.clone(),
        scope: scope.to_string(),
        depth: depth.to_string(),
        owner_info: None,
        timeout_secs,
    };
    let xml = generate_lock_response(&lock_info, timeout_secs);

    // 200 if resource already existed, 201 if new (unmapped) URL.
    let resource_exists = {
        let parts: Vec<&str> = resource_path
            .trim_start_matches('/')
            .splitn(5, '/')
            .collect();
        if parts.len() >= 4 && parts[0] == "caldav" {
            let (u, c, uid) = (parts[1], parts[2], parts[3]);
            let uid_clean = uid.trim_end_matches(".ics");
            let event_exists = match state.storage.get_event(u, c, uid_clean).await {
                Ok(r) => r.is_some(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            let cal_exists = match state.storage.get_calendar(u, c).await {
                Ok(r) => r.is_some(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            event_exists || cal_exists
        } else {
            false
        }
    };

    let status = if resource_exists || !existing.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    let mut resp_headers = HeaderMap::new();
    let lock_token_val = match format!("<{}>", token).parse() {
        Ok(v) => v,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    resp_headers.insert("Lock-Token", lock_token_val);
    resp_headers.insert("Content-Type", HeaderValue::from_static(XML_CONTENT_TYPE));
    (status, resp_headers, xml).into_response()
}

/// UNLOCK — release a lock (RFC 4918 §9.11).
pub async fn caldav_unlock(
    resource_path: String,
    username: String,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Only allow unlocking resources in the authenticated user's own namespace.
    if auth.username != username {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Extract token from Lock-Token header: <opaquelocktoken:...>
    let token = match headers
        .get("Lock-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_start_matches('<').trim_end_matches('>').to_string())
    {
        Some(t) => t,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };

    // Verify the lock is on this resource and belongs to the authenticated user.
    match state.storage.get_lock(&token).await {
        Ok(Some(lock))
            if lock.resource_path.trim_end_matches('/') == resource_path.trim_end_matches('/')
                && lock.username == auth.username => {}
        Ok(Some(lock))
            if lock.resource_path.trim_end_matches('/') == resource_path.trim_end_matches('/') =>
        {
            // Resource matches but lock belongs to a different user.
            return StatusCode::FORBIDDEN.into_response();
        }
        Ok(Some(_)) => return StatusCode::CONFLICT.into_response(),
        Ok(None) => return StatusCode::CONFLICT.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }

    match state.storage.delete_lock(&token).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Generate the `<D:prop><D:lockdiscovery>…</D:lockdiscovery></D:prop>` XML
/// response body for a LOCK response.
fn generate_lock_response(lock: &LockInfo, timeout_secs: i64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:prop xmlns:D="DAV:">
  <D:lockdiscovery>
    <D:activelock>
      <D:locktype><D:write/></D:locktype>
      <D:lockscope><D:{scope}/></D:lockscope>
      <D:depth>{depth}</D:depth>
      <D:timeout>Second-{timeout}</D:timeout>
      <D:locktoken><D:href>{token}</D:href></D:locktoken>
      <D:lockroot><D:href>{resource}</D:href></D:lockroot>
    </D:activelock>
  </D:lockdiscovery>
</D:prop>"#,
        scope = lock.scope,
        depth = lock.depth,
        timeout = timeout_secs,
        token = lock.token,
        resource = lock.resource_path,
    )
}

/// PROPPATCH on a calendar collection — store dead properties and handle
/// well-known Apple properties (`calendar-color`, `calendar-order`, `displayname`).
///
/// iOS sends this to set color and sort-order immediately after account setup.
/// Returning 207 Multi-Status (success) is required; 405 causes a permanent red ✗.
async fn caldav_collection_proppatch(
    resource_path: String,
    username: String,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    if auth.username != username {
        return Err(StatusCode::FORBIDDEN);
    }

    // Extract the calendar name from the resource path: /caldav/{user}/{calendar}
    let calendar = resource_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();

    let ops = parse_proppatch(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut processed: Vec<(String, String)> = Vec::new();
    // Set only when a well-known property actually got mirrored into the
    // calendars table below — a client (iOS/Thunderbird) recoloring or
    // renaming a calendar this way used to leave the web UI's sidebar
    // showing the old color/name until a manual refresh.
    let mut calendar_meta_changed = false;

    for op in ops {
        match &op {
            PropPatchOp::Set {
                namespace,
                name,
                value,
            } => {
                // Mirror well-known Apple properties into the calendars table.
                match name.as_str() {
                    "calendar-color" if namespace == "http://apple.com/ns/ical/" => {
                        // Strip any XML attributes from value (e.g. symbolic-color="custom")
                        // — the stored value should be the bare hex colour.
                        let color = value.trim();
                        let _ = state
                            .storage
                            .update_calendar_color(&username, &calendar, color)
                            .await;
                        calendar_meta_changed = true;
                    }
                    "displayname" => {
                        let _ = state
                            .storage
                            .update_calendar_display_name(&username, &calendar, value.trim())
                            .await;
                        calendar_meta_changed = true;
                    }
                    _ => {}
                }
                // Always store as dead property too so PROPFIND can echo it back.
                state
                    .storage
                    .set_property(&username, &resource_path, namespace, name, value)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                processed.push((namespace.clone(), name.clone()));
            }
            PropPatchOp::Remove { namespace, name } => {
                state
                    .storage
                    .delete_property(&username, &resource_path, namespace, name)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                processed.push((namespace.clone(), name.clone()));
            }
        }
    }

    if calendar_meta_changed {
        crate::notify::notify_calendar_watchers(
            &state.storage,
            &state.change_tx,
            &username,
            &calendar,
            ChangeKind::Cals,
        )
        .await;
    }

    let xml = generate_proppatch_response(&resource_path, &processed)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(StatusCode::MULTI_STATUS, xml))
}

/// PROPPATCH — store dead properties on a resource (RFC 4918 §9.2).
///
/// `resource_path` is the canonical URI path (e.g. `/caldav/admin/litmus/prop`).
/// `username` is the authenticated user (for storage scoping).
pub async fn caldav_proppatch(
    resource_path: String,
    username: String,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    // Basic auth sanity — resource must be under the user's namespace.
    if !resource_path.contains(&format!("/caldav/{}/", auth.username)) {
        return Err(StatusCode::FORBIDDEN);
    }

    let ops = parse_proppatch(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut processed: Vec<(String, String)> = Vec::new();

    for op in ops {
        match op {
            PropPatchOp::Set {
                namespace,
                name,
                value,
            } => {
                state
                    .storage
                    .set_property(&username, &resource_path, &namespace, &name, &value)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                processed.push((namespace, name));
            }
            PropPatchOp::Remove { namespace, name } => {
                state
                    .storage
                    .delete_property(&username, &resource_path, &namespace, &name)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                processed.push((namespace, name));
            }
        }
    }

    let xml = generate_proppatch_response(&resource_path, &processed)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(StatusCode::MULTI_STATUS, xml))
}

/// PROPFIND on a plain resource (event or WebDAV file) — returns stored dead
/// properties plus standard live properties (etag, getlastmodified, etc.).
pub async fn caldav_resource_propfind(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let _propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
    let uid_clean = strip_ics_extension(&uid).to_string();

    // Fetch dead properties.
    let dead_props = state
        .storage
        .get_properties(&username, &resource_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fetch event for live properties (etag).
    let event = state
        .storage
        .get_event(&username, &calendar, &uid_clean)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Build prop XML manually so dead-property namespaces are correct.
    // Each unique namespace gets a prefix ns0, ns1, ...
    let mut ns_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut ns_counter: usize = 0;

    // Assign prefixes for all dead-property namespaces.
    for (ns, _, _) in &dead_props {
        if !ns.is_empty() && !ns_map.contains_key(ns) {
            ns_map.insert(ns.clone(), format!("ns{}", ns_counter));
            ns_counter += 1;
        }
    }

    // Build the xmlns declarations for the multistatus element.
    let ns_decls: String = ns_map
        .iter()
        .map(|(uri, prefix)| format!(" xmlns:{}=\"{}\"", prefix, uri))
        .collect::<Vec<_>>()
        .join("");

    // Build inner prop elements.
    let mut prop_elems = String::new();

    if let Some((_ical, etag)) = event {
        prop_elems.push_str(&format!(
            "        <D:getetag>{}</D:getetag>\n        <D:getcontenttype>{}</D:getcontenttype>\n",
            etag, ICS_CONTENT_TYPE
        ));
    }

    for (ns, name, value) in &dead_props {
        if ns.is_empty() {
            if value.is_empty() {
                prop_elems.push_str(&format!("        <{}/>\n", name));
            } else {
                prop_elems.push_str(&format!("        <{0}>{1}</{0}>\n", name, value));
            }
        } else {
            let prefix = ns_map.get(ns).map(|s| s.as_str()).unwrap_or("x");
            if value.is_empty() {
                prop_elems.push_str(&format!("        <{}:{}/>\n", prefix, name));
            } else {
                prop_elems.push_str(&format!(
                    "        <{0}:{1}>{2}</{0}:{1}>\n",
                    prefix, name, value
                ));
            }
        }
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"{ns_decls}>
  <D:response>
    <D:href>{href}</D:href>
    <D:propstat>
      <D:prop>
{props}      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#,
        ns_decls = ns_decls,
        href = resource_path,
        props = prop_elems,
    );

    Ok(xml_response(MULTISTATUS, xml))
}

/// POST to a calendar collection — create a new event from the request body.
pub async fn caldav_calendar_post(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    require_calendar_writable(state.storage.as_ref(), &username, &calendar, &auth).await?;

    let uid = tidl_caldav::extract_uid(&body).unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let etag = generate_etag(&body);

    state
        .storage
        .store_event(&username, &calendar, &uid, &body, &etag)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::Events,
    )
    .await;

    let location = format!("/caldav/{}/{}/{}.ics", username, calendar, uid);
    let mut headers = HeaderMap::new();
    headers.insert(
        "Location",
        location
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    headers.insert(
        "ETag",
        etag.parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );

    Ok((StatusCode::CREATED, headers))
}

// ── Event resources ───────────────────────────────────────────────────────────

/// Dispatch wrapper for event resources, accepting any HTTP method.
pub async fn caldav_event_any(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    let u = username.clone();
    let c = calendar.clone();
    let id = uid.clone();
    match method.as_str() {
        "GET" | "HEAD" => caldav_event_get(Path((u, c, id)), State(state), auth)
            .await
            .into_response(),
        "PUT" => caldav_event_put(Path((u, c, id)), State(state), auth, headers, body)
            .await
            .into_response(),
        "DELETE" => {
            let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, true).await {
                return status.into_response();
            }
            caldav_event_delete_or_subcollection(Path((u, c, id)), State(state), auth)
                .await
                .into_response()
        }
        "MKCOL" => caldav_subcollection_mkcol(Path((u, c, id)), State(state), auth, body)
            .await
            .into_response(),
        "COPY" => caldav_event_copy(Path((u, c, id)), State(state), auth, headers)
            .await
            .into_response(),
        "MOVE" => {
            let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, false).await {
                return status.into_response();
            }
            caldav_event_move(Path((u, c, id)), State(state), auth, headers)
                .await
                .into_response()
        }
        "PROPPATCH" => {
            let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, false).await {
                return status.into_response();
            }
            caldav_proppatch(resource_path, username, State(state), auth, body)
                .await
                .into_response()
        }
        "PROPFIND" => caldav_resource_propfind(Path((u, c, id)), State(state), auth, headers, body)
            .await
            .into_response(),
        "LOCK" => {
            let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
            caldav_lock(resource_path, username, State(state), auth, headers, body)
                .await
                .into_response()
        }
        "UNLOCK" => {
            let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
            caldav_unlock(resource_path, username, State(state), auth, headers)
                .await
                .into_response()
        }
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Sub-collection event dispatcher: paths like /{username}/{calendar}/{subcoll}/{uid}.
/// Re-uses the same handlers with a compound calendar key "{calendar}/{subcoll}".
pub async fn caldav_subcol_event_any(
    Path((username, calendar, subcoll, uid)): Path<(String, String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    axum::http::request::Parts { method, .. }: axum::http::request::Parts,
    body: String,
) -> impl IntoResponse {
    // Map to a compound calendar name so all existing handlers work unchanged.
    let compound_cal = format!("{}/{}", calendar, subcoll);
    // Pre-clone for arms that need multiple copies.
    let u = username.clone();
    let c = compound_cal.clone();
    let id = uid.clone();

    match method.as_str() {
        "GET" | "HEAD" => caldav_event_get(Path((u, c, id)), State(state), auth)
            .await
            .into_response(),
        "PUT" => caldav_event_put(Path((u, c, id)), State(state), auth, headers, body)
            .await
            .into_response(),
        "DELETE" => {
            let resource_path = format!("/caldav/{}/{}/{}/{}", username, calendar, subcoll, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, true).await {
                return status.into_response();
            }
            caldav_event_delete_or_subcollection(Path((u, c, id)), State(state), auth)
                .await
                .into_response()
        }
        "COPY" => caldav_event_copy(Path((u, c, id)), State(state), auth, headers)
            .await
            .into_response(),
        "MOVE" => {
            let resource_path = format!("/caldav/{}/{}/{}/{}", username, calendar, subcoll, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, true).await {
                return status.into_response();
            }
            caldav_event_move(Path((u, c, id)), State(state), auth, headers)
                .await
                .into_response()
        }
        // MKCOL: create a sub-sub-collection if the parent sub-collection exists,
        // otherwise 409 (missing intermediate, RFC 4918 §9.3).
        "MKCOL" | "MKCALENDAR" => caldav_subcol_mkcol(Path((u, c, id)), State(state), auth, body)
            .await
            .into_response(),
        "PROPPATCH" => {
            let resource_path = format!("/caldav/{}/{}/{}/{}", username, calendar, subcoll, uid);
            if let Some(status) = check_write_lock(&resource_path, &headers, &state, true).await {
                return status.into_response();
            }
            caldav_proppatch(resource_path, username, State(state), auth, body)
                .await
                .into_response()
        }
        "LOCK" => {
            let resource_path = format!("/caldav/{}/{}/{}/{}", username, calendar, subcoll, uid);
            caldav_lock(resource_path, username, State(state), auth, headers, body)
                .await
                .into_response()
        }
        "UNLOCK" => {
            let resource_path = format!("/caldav/{}/{}/{}/{}", username, calendar, subcoll, uid);
            caldav_unlock(resource_path, username, State(state), auth, headers)
                .await
                .into_response()
        }
        "OPTIONS" => caldav_options().into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// MKCOL for a sub-sub-collection: reuses caldav_subcollection_mkcol logic
/// with the compound parent calendar already resolved.
async fn caldav_subcol_mkcol(
    Path((username, parent_cal, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    // RFC 4918 §9.3: body → 415.
    if !body.is_empty() {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    // Parent must exist → 409 if not.
    if state
        .storage
        .get_calendar(&username, &parent_cal)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::CONFLICT);
    }
    let sub_name = format!("{}/{}", parent_cal, name);
    // Already exists → 405.
    let uid = strip_ics_extension(&name);
    if state
        .storage
        .get_event(&username, &parent_cal, uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    if state
        .storage
        .get_calendar(&username, &sub_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    state
        .storage
        .create_calendar(&username, &sub_name, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::CREATED)
}

/// MKCOL on a sub-collection path (`/{username}/{calendar}/{name}`).
///
/// RFC 4918 §9.3: if the request body is non-empty and not understood,
/// return 415. If parent collection doesn't exist, return 409. Otherwise 201.
pub async fn caldav_subcollection_mkcol(
    Path((username, calendar, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    // RFC 4918 §9.3: MKCOL with an unrecognised request body MUST fail with 415.
    if !body.is_empty() {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // RFC 4918 §9.3: if the parent collection doesn't exist, return 409.
    if state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::CONFLICT);
    }

    // Use a slash-joined key to store the sub-collection.
    let sub_name = format!("{}/{}", calendar, name);

    // RFC 4918 §9.3: MKCOL on an existing resource MUST fail with 405.
    // Check both events (plain resources) and sub-collections.
    let uid = strip_ics_extension(&name);
    if state
        .storage
        .get_event(&username, &calendar, uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    if state
        .storage
        .get_calendar(&username, &sub_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    state
        .storage
        .create_calendar(&username, &sub_name, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::CREATED)
}

/// DELETE at the event/sub-collection level: handles both `.ics` events and
/// sub-collections created via MKCOL.
pub async fn caldav_event_delete_or_subcollection(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let uid_clean = strip_ics_extension(&uid).to_string();

    // Check if it's an event first — this branch is share-alias-aware.
    let (event_username, event_calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Write,
    )
    .await?;
    require_calendar_writable(
        state.storage.as_ref(),
        &event_username,
        &event_calendar,
        &auth,
    )
    .await?;
    if state
        .storage
        .get_event(&event_username, &event_calendar, &uid_clean)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        state
            .storage
            .delete_event(&event_username, &event_calendar, &uid_clean)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        // If this was a note-mirrored task, clear the note's to-do flag too
        // (backlog #71) — otherwise an external DELETE leaves the note
        // thinking it's still a live to-do, pointing at a mirror that no
        // longer exists, until the note is next edited/saved.
        let _ = state.storage.complete_note_mirror_if_any(&uid_clean).await;
        // Archive the matching done row immediately rather than waiting for the
        // next Done-view reconcile (backlog #136). Best-effort: the DELETE has
        // already succeeded.
        let _ = state
            .storage
            .sync_done_item_from_caldav(&event_username, &event_calendar, &uid_clean, false)
            .await;
        crate::notify::notify_calendar_watchers(
            &state.storage,
            &state.change_tx,
            &event_username,
            &event_calendar,
            ChangeKind::Events,
        )
        .await;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Sub-collection deletion remains owner-only — deliberately does NOT
    // consult resolve_calendar_identity (see plan Global Constraints).
    let sub_name = format!("{}/{}", calendar, uid);
    if state
        .storage
        .get_calendar(&username, &sub_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        state
            .storage
            .delete_calendar(&username, &sub_name)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    Err(StatusCode::NOT_FOUND)
}

/// GET a single `.ics` resource.
pub async fn caldav_event_get(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (username, calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Read,
    )
    .await?;

    let uid = strip_ics_extension(&uid).to_string();

    match state
        .storage
        .get_event(&username, &calendar, &uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some((ical_data, etag)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                "ETag",
                HeaderValue::from_str(&etag).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
            headers.insert("Content-Type", HeaderValue::from_static(ICS_CONTENT_TYPE));
            Ok((StatusCode::OK, headers, ical_data))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// PUT a `.ics` resource (create or update).
pub async fn caldav_event_put(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let (username, calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Write,
    )
    .await?;

    let uid = strip_ics_extension(&uid).to_string();

    debug!(
        username = %username,
        calendar = %calendar,
        uid = %uid,
        content_length = body.len(),
        if_match = ?headers.get("If-Match").and_then(|v| v.to_str().ok()),
        if_none_match = ?headers.get("If-None-Match").and_then(|v| v.to_str().ok()),
        "caldav_event_put"
    );

    // RFC 4918 §9.7.1: parent collection must exist before any content validation.
    // Checking this first ensures a missing calendar returns 409, not 422.
    let cal = state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::CONFLICT)?;

    // Validate iCalendar data before storing
    if let Err(e) = tidl_caldav::validate_icalendar(&body) {
        debug!(error = %e, "caldav_event_put: iCalendar validation failed");
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let etag = generate_etag(&body);

    // Check whether the event already exists (needed for If-None-Match and response code).
    let existing = state
        .storage
        .get_event(&username, &calendar, &uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // RFC 4918 §10.4: If-None-Match: * → only succeed if resource does NOT exist.
    if let Some(inm) = headers.get("If-None-Match").and_then(|v| v.to_str().ok()) {
        if inm == "*" && existing.is_some() {
            debug!("caldav_event_put: If-None-Match: * failed (event exists)");
            return Err(StatusCode::PRECONDITION_FAILED);
        }
    }

    // Optimistic concurrency: honour If-Match
    if let Some(if_match) = headers.get("If-Match").and_then(|v| v.to_str().ok()) {
        match &existing {
            Some((_, stored_etag)) => {
                if stored_etag != if_match && if_match != "*" {
                    debug!(stored = %stored_etag, requested = %if_match, "caldav_event_put: If-Match failed");
                    return Err(StatusCode::PRECONDITION_FAILED);
                }
            }
            None => {
                // If-Match on a non-existent resource always fails.
                if if_match != "*" {
                    return Err(StatusCode::PRECONDITION_FAILED);
                }
            }
        }
    }

    // RFC 4918 §10.4 + §7: check locking.
    {
        let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
        let locks = state
            .storage
            .get_effective_locks(&resource_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !locks.is_empty() {
            // Resource is locked — require the valid lock token in If: header,
            // AND the full If: condition must evaluate to true (including etags).
            match headers.get("If").and_then(|v| v.to_str().ok()) {
                Some(if_hdr)
                    if lock_token_in_if_header(if_hdr, &resource_path, &state).await
                        && check_if_header(if_hdr, &resource_path, &state).await =>
                {
                    // Valid lock token provided and conditions pass — allow.
                }
                Some(_) => return Err(StatusCode::PRECONDITION_FAILED),
                None => return Err(StatusCode::LOCKED),
            }
        } else if let Some(if_hdr) = headers.get("If").and_then(|v| v.to_str().ok()) {
            // If: header present but no lock on resource — validate the conditional.
            if !check_if_header(if_hdr, &resource_path, &state).await {
                return Err(StatusCode::PRECONDITION_FAILED);
            }
        }
    }

    if auth.token_read_only {
        return Err(StatusCode::FORBIDDEN);
    }
    if cal.read_only {
        match &cal.sync_user {
            Some(su) if su == &auth.username => {}
            _ => return Err(StatusCode::FORBIDDEN),
        }
    }

    state
        .storage
        .store_event(&username, &calendar, &uid, &body, &etag)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let fields = tidl_caldav::extract_event_fields(&body);

    // If this VTODO is a note mirror, propagate a client-side priority edit
    // back onto the note's own priority (2026-07-30) — a no-op for any
    // ordinary (non-note) task. Best-effort: never fail the PUT itself over
    // this, since the primary write above already succeeded.
    if let Some(priority) = fields.priority {
        if let Err(e) = state
            .storage
            .sync_note_priority_from_mirror(&uid, priority)
            .await
        {
            debug!(error = %e, uid = %uid, "caldav_event_put: note priority reverse-sync failed");
        }
    }

    // Reverse-sync a client's STATUS edit (backlog #71). Best-effort, same
    // reasoning as the priority sync above.
    if let Err(e) =
        sync_vtodo_completion(state.storage.as_ref(), &username, &calendar, &uid, &fields).await
    {
        debug!(error = %e, uid = %uid, "caldav_event_put: completion reverse-sync failed");
    }

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::Events,
    )
    .await;

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        "ETag",
        etag.parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );

    // RFC 4918 §9.7.1: 201 Created for new resources, 204 No Content for updates.
    let status = if existing.is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    debug!(status = %status, "caldav_event_put: stored event");

    Ok((status, resp_headers))
}

/// Reverse-sync a CalDAV client's `STATUS` edit on a VTODO onto the app's
/// own completion model (backlog #71).
///
/// A raw CalDAV client has no way to know whether the task it's completing
/// is note-mirrored, so this decides on its behalf: completing a
/// note-mirrored task mirrors backlog #40's own completion write-back
/// exactly (clear the note's `kind`/`due_date`/`mirrored_event_uid`, delete
/// the mirror row) rather than just flipping `status` — one consistent
/// completion model regardless of which client triggered it. A plain task
/// just gets `status`/`completed_date` set directly. No-op for anything
/// that isn't a VTODO, or that has no `STATUS` property at all (so a client
/// PUT that doesn't touch completion never clobbers the existing value —
/// see `Storage::store_event`'s doc comment).
async fn sync_vtodo_completion(
    storage: &dyn crate::storage::CalendarStore,
    username: &str,
    calendar: &str,
    uid: &str,
    fields: &tidl_caldav::EventFields,
) -> anyhow::Result<()> {
    if fields.component_type != "VTODO" {
        return Ok(());
    }
    let Some(status) = &fields.status else {
        return Ok(());
    };
    if status == "COMPLETED" {
        if storage.complete_note_mirror_if_any(uid).await? {
            storage.delete_event(username, calendar, uid).await?;
            // The mirror row is gone, so there's no CalDAV origin left to index;
            // archive any stale live row just in case (backlog #136).
            storage
                .sync_done_item_from_caldav(username, calendar, uid, false)
                .await?;
        } else {
            storage
                .set_event_completed(username, calendar, uid, true)
                .await?;
            // Index the completion immediately rather than waiting for the next
            // Done-view reconcile (backlog #136).
            storage
                .sync_done_item_from_caldav(username, calendar, uid, true)
                .await?;
        }
    } else {
        storage
            .set_event_completed(username, calendar, uid, false)
            .await?;
        storage
            .sync_done_item_from_caldav(username, calendar, uid, false)
            .await?;
    }
    Ok(())
}

/// COPY an event resource or sub-collection (RFC 4918 §9.8).
///
/// When the source "uid" segment is actually a sub-collection (stored as
/// `{calendar}/{uid}` in the calendars table) this delegates to collection
/// copy logic using the `Destination` header parsed as a collection path.
pub async fn caldav_event_copy(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let uid_clean = strip_ics_extension(&uid).to_string();
    let sub_name = format!("{}/{}", calendar, uid_clean);

    // Check if this is a sub-collection rather than an event.
    if state
        .storage
        .get_calendar(&username, &sub_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        // Treat as collection COPY: destination is also a sub-collection.
        // Use parse_destination_event to get the destination name as the "uid" field.
        let (dst_username, _dst_parent, dst_name_raw, overwrite) =
            parse_destination_event(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
        // A user may only copy into their own namespace.
        require_same_user(&auth, &dst_username)?;
        let dst_sub = format!("{}/{}", calendar, strip_ics_extension(&dst_name_raw));

        let dst_exists = state
            .storage
            .get_calendar(&dst_username, &dst_sub)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        let status = overwrite_status(dst_exists, overwrite)?;
        if dst_exists {
            state
                .storage
                .delete_calendar(&dst_username, &dst_sub)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        let deep = headers
            .get("Depth")
            .and_then(|v| v.to_str().ok())
            .map(|v| v != "0")
            .unwrap_or(true);
        state
            .storage
            .copy_calendar(&username, &sub_name, &dst_username, &dst_sub, deep)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(status);
    }

    let (dst_username, dst_calendar, dst_uid_raw, overwrite) =
        parse_destination_event(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
    require_same_user(&auth, &dst_username)?;
    let dst_uid = strip_ics_extension(&dst_uid_raw).to_string();

    // Source event must exist.
    if state
        .storage
        .get_event(&username, &calendar, &uid_clean)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    // Destination calendar must exist (RFC 4918 §9.8.5).
    let dst_cal = state
        .storage
        .get_calendar(&dst_username, &dst_calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::CONFLICT)?;

    if dst_cal.read_only {
        match &dst_cal.sync_user {
            Some(su) if su == &auth.username => {}
            _ => return Err(StatusCode::FORBIDDEN),
        }
    }

    let dst_exists = state
        .storage
        .get_event(&dst_username, &dst_calendar, &dst_uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    let status = overwrite_status(dst_exists, overwrite)?;

    // RFC 4918 §9.8.5: if destination is locked, COPY must fail unless lock token provided.
    if dst_exists {
        let dst_path = format!("/caldav/{}/{}/{}", dst_username, dst_calendar, dst_uid);
        let dst_locks = state
            .storage
            .get_effective_locks(&dst_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !dst_locks.is_empty() {
            let allowed = match headers.get("If").and_then(|v| v.to_str().ok()) {
                Some(if_hdr) => check_if_header(if_hdr, &dst_path, &state).await,
                None => false,
            };
            if !allowed {
                return Err(StatusCode::LOCKED);
            }
        }
    }

    state
        .storage
        .copy_event(
            &username,
            &calendar,
            &uid_clean,
            &dst_username,
            &dst_calendar,
            &dst_uid,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &dst_username,
        &dst_calendar,
        ChangeKind::Events,
    )
    .await;

    Ok(status)
}

/// MOVE an event resource or sub-collection (RFC 4918 §9.9).
pub async fn caldav_event_move(
    Path((username, calendar, uid)): Path<(String, String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;

    let uid_clean = strip_ics_extension(&uid).to_string();
    let sub_name = format!("{}/{}", calendar, uid_clean);

    // Check if this is a sub-collection.
    if state
        .storage
        .get_calendar(&username, &sub_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some()
    {
        let (dst_username, _dst_parent, dst_name_raw, overwrite) =
            parse_destination_event(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
        require_same_user(&auth, &dst_username)?;
        let dst_sub = format!("{}/{}", calendar, strip_ics_extension(&dst_name_raw));

        let dst_exists = state
            .storage
            .get_calendar(&dst_username, &dst_sub)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some();

        let status = overwrite_status(dst_exists, overwrite)?;
        if dst_exists {
            state
                .storage
                .delete_calendar(&dst_username, &dst_sub)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        // MOVE always moves everything.
        state
            .storage
            .copy_calendar(&username, &sub_name, &dst_username, &dst_sub, true)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state
            .storage
            .delete_calendar(&username, &sub_name)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(status);
    }

    let (dst_username, dst_calendar, dst_uid_raw, overwrite) =
        parse_destination_event(&headers, &username).ok_or(StatusCode::BAD_REQUEST)?;
    require_same_user(&auth, &dst_username)?;
    let dst_uid = strip_ics_extension(&dst_uid_raw).to_string();

    if state
        .storage
        .get_event(&username, &calendar, &uid_clean)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let dst_cal = state
        .storage
        .get_calendar(&dst_username, &dst_calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::CONFLICT)?;

    if dst_cal.read_only {
        match &dst_cal.sync_user {
            Some(su) if su == &auth.username => {}
            _ => return Err(StatusCode::FORBIDDEN),
        }
    }

    let dst_exists = state
        .storage
        .get_event(&dst_username, &dst_calendar, &dst_uid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    let status = overwrite_status(dst_exists, overwrite)?;

    state
        .storage
        .move_event(
            &username,
            &calendar,
            &uid_clean,
            &dst_username,
            &dst_calendar,
            &dst_uid,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    crate::notify::notify_calendar_watchers(
        &state.storage,
        &state.change_tx,
        &username,
        &calendar,
        ChangeKind::Events,
    )
    .await;
    if dst_calendar != calendar {
        crate::notify::notify_calendar_watchers(
            &state.storage,
            &state.change_tx,
            &dst_username,
            &dst_calendar,
            ChangeKind::Events,
        )
        .await;
    }

    Ok(status)
}

// ── PROPFIND ──────────────────────────────────────────────────────────────────

/// `PROPFIND` on a calendar collection.
///
/// Returns properties of the collection itself; at `Depth: 1` also includes
/// all contained event resources.
pub async fn caldav_propfind(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, StatusCode> {
    require_same_user(&auth, &username)?;
    let requested_calendar = calendar.clone();
    let (username, calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Read,
    )
    .await?;
    let is_shared_read_only = parse_shared_alias(&requested_calendar).is_some()
        && resolve_calendar_identity(
            state.storage.as_ref(),
            &auth.username,
            &requested_calendar,
            Permission::Write,
        )
        .await
        .is_err();

    let depth_header = headers.get("Depth").and_then(|v| v.to_str().ok());
    let propfind = parse_propfind(&body, depth_header).map_err(|_| StatusCode::BAD_REQUEST)?;

    let cal_href = format!("/caldav/{}/{}/", auth.username, requested_calendar);
    let mut responses = vec![];

    let sync_token = state
        .storage
        .get_sync_token(&username, &calendar)
        .await
        .unwrap_or(1);

    // Calendar collection resource
    let cal = state
        .storage
        .get_calendar(&username, &calendar)
        .await
        .ok()
        .flatten();
    let display_name = cal.as_ref().and_then(|c| c.display_name.clone());
    let resource_path = format!("/caldav/{}/{}", username, calendar);
    let dead_props = state
        .storage
        .get_properties(&username, &resource_path)
        .await
        .unwrap_or_default();
    let cal_propstats = build_calendar_propstats(
        &requested_calendar,
        &display_name,
        &propfind.kind,
        sync_token,
        &dead_props,
        &auth.username,
        is_shared_read_only,
    );
    responses.push(Response {
        href: cal_href.clone(),
        propstats: cal_propstats,
    });

    // Event resources at Depth: 1
    if matches!(propfind.depth, Depth::One | Depth::Infinity) {
        let events = state
            .storage
            .list_events(&username, &calendar)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        for event in events {
            let event_href = format!("{}{}.ics", cal_href, event.uid);
            responses.push(Response {
                href: event_href,
                propstats: vec![PropStat {
                    properties: vec![
                        Property {
                            namespace: "D".into(),
                            name: properties::GETETAG.into(),
                            value: Some(event.etag.clone()),
                        },
                        Property {
                            namespace: "D".into(),
                            name: properties::GETCONTENTTYPE.into(),
                            value: Some(ICS_CONTENT_TYPE.into()),
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

// ── REPORT ────────────────────────────────────────────────────────────────────

/// `REPORT` — dispatches to `calendar-query`, `calendar-multiget`, or `sync-collection`.
pub async fn caldav_report(
    Path((username, calendar)): Path<(String, String)>,
    State(state): State<AppState>,
    auth: BasicAuth,
    body: String,
) -> Result<axum::response::Response, StatusCode> {
    require_same_user(&auth, &username)?;
    let requested_calendar = calendar.clone();
    let (username, calendar) = resolve_calendar_identity(
        state.storage.as_ref(),
        &username,
        &calendar,
        Permission::Read,
    )
    .await?;

    if body.contains("sync-collection") {
        return caldav_sync_collection(
            &username,
            &calendar,
            &auth.username,
            &requested_calendar,
            &state,
            &body,
        )
        .await
        .map(IntoResponse::into_response);
    }

    let is_multiget = body.contains("calendar-multiget");

    let objects: Vec<CalendarObject> = if is_multiget {
        let multiget = parse_calendar_multiget(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

        let mut result = vec![];
        for href in &multiget.hrefs {
            let raw_uid = href
                .split('/')
                .next_back()
                .map(strip_ics_extension)
                .unwrap_or(href.as_str());
            // URL-decode in case the client percent-encoded characters such as
            // '@' (common in UIDs from Apple/Google calendar exports).
            let uid = percent_encoding::percent_decode_str(raw_uid)
                .decode_utf8()
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| raw_uid.to_string());

            if let Ok(Some((ical_data, etag))) =
                state.storage.get_event(&username, &calendar, &uid).await
            {
                result.push(CalendarObject {
                    href: href.clone(),
                    etag,
                    calendar_data: if multiget.include_calendar_data {
                        Some(ical_data)
                    } else {
                        None
                    },
                });
            }
        }
        result
    } else {
        let query = parse_calendar_query(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

        let events = state
            .storage
            .list_events(&username, &calendar)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        events
            .into_iter()
            .map(|e| CalendarObject {
                href: format!(
                    "/caldav/{}/{}/{}.ics",
                    auth.username, requested_calendar, e.uid
                ),
                etag: e.etag,
                calendar_data: if query.include_calendar_data {
                    Some(e.ical_data)
                } else {
                    None
                },
            })
            .collect()
    };

    let xml = generate_report_response(objects).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(xml_response(MULTISTATUS, xml).into_response())
}

/// `sync-collection` REPORT (RFC 6578).
///
/// - Empty or unrecognised token → full sync (all current events returned as
///   200 responses) plus the current token.
/// - Known token → incremental: only changed/deleted events since that token.
async fn caldav_sync_collection(
    username: &str,
    calendar: &str,
    requested_username: &str,
    requested_calendar: &str,
    state: &AppState,
    body: &str,
) -> Result<impl IntoResponse, StatusCode> {
    let since_token = parse_sync_token_from_body(body);

    let current_token = state
        .storage
        .get_sync_token(username, calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::debug!(
        username,
        calendar,
        since_token = ?since_token,
        current_token,
        "sync_collection: token state"
    );

    let response_xml: String = match since_token {
        // ── Full sync (no token or unrecognised token) ────────────────────
        None => {
            let events = state
                .storage
                .list_events(username, calendar)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            let mut entries = String::new();
            for ev in events {
                let href = format!(
                    "/caldav/{}/{}/{}.ics",
                    requested_username, requested_calendar, ev.uid
                );
                entries.push_str(&sync_response_entry(&href, Some(&ev.etag)));
            }
            entries
        }
        // ── Incremental sync ──────────────────────────────────────────────
        Some(since) => {
            let changes = state
                .storage
                .get_sync_changes(username, calendar, since, None)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            tracing::debug!(
                since,
                change_count = changes.len(),
                "sync_collection: incremental changes"
            );

            let mut entries = String::new();
            for change in changes {
                let href = format!(
                    "/caldav/{}/{}/{}.ics",
                    requested_username, requested_calendar, change.uid
                );
                // The logged operation is only a hint: the `events` table is
                // authoritative. A log entry can be stale in either direction
                // (a `deleted` tombstone for an event that was later re-added,
                // or a `modified` for one since removed), so always look up the
                // current row and report 200-with-etag when it exists, 404 only
                // when it's genuinely gone. This is what repairs clients that
                // already replayed a stale tombstone.
                let etag = state
                    .storage
                    .get_event(username, calendar, &change.uid)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .map(|(_ical, etag)| etag);
                entries.push_str(&sync_response_entry(&href, etag.as_deref()));
            }
            entries
        }
    };

    let xml = format_sync_multistatus(&response_xml, current_token);

    Ok(xml_response(MULTISTATUS, xml))
}

/// Extract the `since` sync-token from a `sync-collection` REPORT body, if
/// present. Searches for the token URI directly rather than the XML tag,
/// because clients use different namespace prefixes (e.g. iOS uses `A:` for
/// the DAV namespace, not `D:`), so matching `<D:sync-token>` fails.
fn parse_sync_token_from_body(body: &str) -> Option<i64> {
    let pos = body.find("urn:tidl:sync:")?;
    body[pos + "urn:tidl:sync:".len()..]
        .split('<')
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Build one `<D:response>` entry for a `.ics` resource: `200 OK` with the
/// given etag when the event exists, or `404 Not Found` when it doesn't.
/// `href` is escaped for XML here so callers can pass the raw path.
fn sync_response_entry(href: &str, etag: Option<&str>) -> String {
    let href = tidl_caldav::escape_xml(href);
    match etag {
        Some(etag) => format!(
            "  <D:response>\n    <D:href>{href}</D:href>\n\
             <D:propstat><D:prop><D:getetag>{etag}</D:getetag></D:prop>\
             <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\n  </D:response>\n",
            href = href,
            etag = etag,
        ),
        None => format!(
            "  <D:response>\n    <D:href>{href}</D:href>\n\
             <D:status>HTTP/1.1 404 Not Found</D:status>\n  </D:response>\n",
            href = href,
        ),
    }
}

/// Wrap already-built per-resource `<D:response>` entries plus the current
/// sync token in a `207 Multi-Status` document. RFC 6578 §3.6: `sync-token`
/// is a direct child of `multistatus`.
fn format_sync_multistatus(entries: &str, token: i64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/">
{responses}  <D:sync-token>urn:tidl:sync:{token}</D:sync-token>
</D:multistatus>"#,
        responses = entries,
        token = token,
    )
}

/// Returns `Some(StatusCode::LOCKED)` if the resource is actively locked and the
/// request does not supply a valid token in the `If:` header; `None` otherwise.
/// `effective` selects depth-aware locking (true) or exact-path only (false).
async fn check_write_lock(
    resource_path: &str,
    headers: &HeaderMap,
    state: &AppState,
    effective: bool,
) -> Option<StatusCode> {
    let locks = if effective {
        state.storage.get_effective_locks(resource_path).await
    } else {
        state.storage.get_locks_for_resource(resource_path).await
    };
    match locks {
        Ok(locks) if !locks.is_empty() => {
            let allowed = match headers.get("If").and_then(|v| v.to_str().ok()) {
                Some(if_hdr) => lock_token_in_if_header(if_hdr, resource_path, state).await,
                None => false,
            };
            if !allowed {
                Some(StatusCode::LOCKED)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether a CalDAV operation is read-only or requires write access.
/// Used to gate shared-calendar-alias resolution against the grant's
/// `permission` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Permission {
    Read,
    Write,
}

/// Extract the share-grant ID from a `shared-{id}` calendar-path segment.
/// Returns `None` for anything that isn't exactly that shape (including a
/// real calendar literally named e.g. "shared-notes", which is why this is
/// only ever consulted as a *fallback* after a real calendar lookup would
/// have already succeeded via `resolve_calendar_identity`'s own-calendar
/// fast path).
fn parse_shared_alias(calendar: &str) -> Option<i64> {
    calendar.strip_prefix("shared-")?.parse().ok()
}

/// Resolve the effective `(owner_username, calendar_name)` a CalDAV request
/// should operate on.
///
/// For a normal request (`calendar` is the caller's own calendar), this is
/// a no-op passthrough. For a `shared-{id}` alias, looks up the
/// corresponding `calendar_shares` grant, verifies the caller is really the
/// grantee, and checks the grant's `permission` covers the requested
/// `required` level (a `read` grant only ever satisfies `Permission::Read`).
///
/// Callers must still run `require_same_user(&auth, &username)` first, as
/// always — this function assumes that has already passed, i.e. `username`
/// here is already known to be the authenticated caller's own username.
async fn resolve_calendar_identity(
    storage: &dyn crate::storage::CalendarStore,
    username: &str,
    calendar: &str,
    required: Permission,
) -> Result<(String, String), StatusCode> {
    let Some(share_id) = parse_shared_alias(calendar) else {
        // Not an alias — operate on the caller's own calendar, unchanged.
        return Ok((username.to_string(), calendar.to_string()));
    };

    let grant = storage
        .get_share_grant_by_id(share_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::FORBIDDEN)?;

    if grant.grantee_username != username {
        return Err(StatusCode::FORBIDDEN);
    }

    if required == Permission::Write && grant.permission != "read-write" {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok((grant.owner_username, grant.calendar_name))
}

pub(crate) fn require_same_user(auth: &BasicAuth, username: &str) -> Result<(), StatusCode> {
    if auth.username != username {
        Err(StatusCode::FORBIDDEN)
    } else {
        Ok(())
    }
}

/// Returns `403 Forbidden` if the authenticated token is read-only, or if
/// `calendar` is a read-only sync mirror and the authenticated user is not
/// the designated sync writer.
async fn require_calendar_writable(
    storage: &dyn crate::storage::CalendarStore,
    username: &str,
    calendar: &str,
    auth: &BasicAuth,
) -> Result<(), StatusCode> {
    // Read-only app token: no writes allowed at all.
    if auth.token_read_only {
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(cal) = storage
        .get_calendar(username, calendar)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        if cal.read_only {
            match &cal.sync_user {
                Some(su) if *su == auth.username => {}
                _ => return Err(StatusCode::FORBIDDEN),
            }
        }
    }
    Ok(())
}

fn strip_ics_extension(uid: &str) -> &str {
    uid.strip_suffix(".ics").unwrap_or(uid)
}

pub(crate) fn xml_response(status: StatusCode, body: String) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static(XML_CONTENT_TYPE));
    (status, headers, body)
}

/// Derive a stable pseudo-UUID from the username for use as `resource-id`.
pub(crate) fn username_to_resource_id(username: &str) -> String {
    // Reuse the existing SHA-256 etag machinery; strip quotes and reformat as UUID.
    let raw = tidl_caldav::etag::generate_etag(&format!("principal:{}", username));
    let hex = raw.trim_matches('"');
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Build the `Vec<PropStat>` for a principal (user) resource.
///
/// Returns a 200 propstat for every property we support plus, when the client
/// requested specific names (`Prop` variant), a 404 propstat listing everything
/// it asked for that we do not recognise.
fn build_principal_propstats(
    username: &str,
    email: &str,
    kind: &PropFindType,
    principal_href: &str,
    home_href: &str,
) -> Vec<PropStat> {
    if matches!(kind, PropFindType::PropName) {
        return vec![];
    }

    // All properties this server knows how to return for a principal.
    let supported: Vec<Property> = vec![
        Property {
            namespace: "D".into(),
            name: properties::DISPLAYNAME.into(),
            value: Some(username.to_string()),
        },
        Property {
            namespace: "D".into(),
            name: properties::RESOURCETYPE.into(),
            value: Some("<D:principal/>".into()),
        },
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
            value: Some("<D:href>/caldav/principals/</D:href>".into()),
        },
        Property {
            namespace: "D".into(),
            name: properties::RESOURCE_ID.into(),
            value: Some(format!(
                "<D:href>{}</D:href>",
                username_to_resource_id(username)
            )),
        },
        Property {
            namespace: "D".into(),
            name: properties::SUPPORTED_REPORT_SET.into(),
            value: Some(
                "<D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .into(),
            ),
        },
        Property {
            namespace: "C".into(),
            name: properties::CALENDAR_HOME_SET.into(),
            value: Some(format!("<D:href>{}</D:href>", home_href)),
        },
        Property {
            namespace: "C".into(),
            name: properties::CALENDAR_USER_ADDRESS_SET.into(),
            value: Some(format!("<D:href>mailto:{}</D:href>", email)),
        },
        Property {
            namespace: "C".into(),
            name: properties::SCHEDULE_INBOX_URL.into(),
            value: Some(format!("<D:href>{}inbox/</D:href>", home_href)),
        },
        Property {
            namespace: "C".into(),
            name: properties::SCHEDULE_OUTBOX_URL.into(),
            value: Some(format!("<D:href>{}outbox/</D:href>", home_href)),
        },
    ];

    match kind {
        PropFindType::AllProp => {
            vec![PropStat {
                properties: supported,
                status: "HTTP/1.1 200 OK".into(),
            }]
        }
        PropFindType::Prop(requested) => {
            // Build a lookup by lowercase name — `requested` preserves the
            // client's original case (see `local_name_preserving_case` in
            // tidl-caldav), so comparison must lowercase explicitly here
            // rather than assume it already is.
            let supported_map: std::collections::HashMap<String, Property> = supported
                .into_iter()
                .map(|p| (p.name.to_lowercase(), p))
                .collect();

            let mut found: Vec<Property> = vec![];
            let mut not_found: Vec<Property> = vec![];

            for name in requested {
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
        PropFindType::PropName => vec![], // already handled above
    }
}

/// Map a namespace URI to the prefix declared in our multistatus envelope.
/// Returns `None` for namespaces we can't represent without a new declaration.
fn ns_uri_to_prefix(uri: &str) -> Option<&'static str> {
    match uri {
        "DAV:" => Some("D"),
        "urn:ietf:params:xml:ns:caldav" => Some("C"),
        "http://calendarserver.org/ns/" => Some("CS"),
        "http://apple.com/ns/ical/" => Some("ICAL"),
        _ => None,
    }
}

/// Build a `Vec<PropStat>` for a calendar collection PROPFIND response.
///
/// `dead_props` is the list of stored dead properties `(namespace_uri, name, value)`.
/// Returns a 200 propstat for all known/found properties and, when the client
/// requested a specific list, a 404 propstat for everything it asked for that
/// we cannot satisfy.
fn build_calendar_propstats(
    name: &str,
    display_name: &Option<String>,
    kind: &PropFindType,
    sync_token: i64,
    dead_props: &[(String, String, String)],
    username: &str,
    read_only: bool,
) -> Vec<PropStat> {
    if matches!(kind, PropFindType::PropName) {
        return vec![];
    }

    let sync_token_uri = format!("urn:tidl:sync:{}", sync_token);
    let ctag = format!("\"{}\"", sync_token);

    // ── Build the full map of properties we can return ────────────────────────
    // Key: lowercase property name  (parse_propfind lowercases everything)
    let mut known: std::collections::HashMap<String, Property> = std::collections::HashMap::new();

    let static_props: Vec<Property> = vec![
        Property {
            namespace: "D".into(),
            name: properties::DISPLAYNAME.into(),
            value: Some(tidl_caldav::escape_xml(
                &display_name.clone().unwrap_or_else(|| name.to_string()),
            )),
        },
        Property {
            namespace: "D".into(),
            name: properties::RESOURCETYPE.into(),
            value: Some("<D:collection/><C:calendar/>".into()),
        },
        Property {
            namespace: "D".into(),
            name: properties::GETCONTENTTYPE.into(),
            value: Some(ICS_CONTENT_TYPE.into()),
        },
        Property {
            namespace: "D".into(),
            name: properties::GETETAG.into(),
            value: Some(ctag.clone()),
        },
        Property {
            namespace: "D".into(),
            name: properties::SYNC_TOKEN.into(),
            value: Some(sync_token_uri),
        },
        Property {
            namespace: "CS".into(),
            name: properties::GETCTAG.into(),
            value: Some(ctag),
        },
        Property {
            namespace: "C".into(),
            name: properties::SUPPORTED_CALENDAR_COMPONENT_SET.into(),
            value: Some("<C:comp name=\"VEVENT\"/><C:comp name=\"VTODO\"/>".into()),
        },
        Property {
            namespace: "D".into(),
            name: "current-user-privilege-set".into(),
            value: Some(if read_only {
                "<D:privilege><D:read/></D:privilege>".into()
            } else {
                "<D:privilege><D:all/></D:privilege>\
                 <D:privilege><D:read/></D:privilege>\
                 <D:privilege><D:write/></D:privilege>\
                 <D:privilege><D:write-properties/></D:privilege>\
                 <D:privilege><D:write-content/></D:privilege>"
                    .into()
            }),
        },
        // RFC 3744: collection owner (principal URL).
        Property {
            namespace: "D".into(),
            name: "owner".into(),
            value: Some(format!(
                "<D:href>/caldav/principals/{}/</D:href>",
                username
            )),
        },
        // RFC 4791: reports supported on this calendar collection.
        Property {
            namespace: "D".into(),
            name: properties::SUPPORTED_REPORT_SET.into(),
            value: Some(
                "<D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                 <D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                 <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                    .into(),
            ),
        },
        // RFC 4791 §7.3: free/busy transparency — opaque (default, events block time).
        Property {
            namespace: "C".into(),
            name: "schedule-calendar-transp".into(),
            value: Some("<C:opaque/>".into()),
        },
        // Apple extension: hint to iOS/macOS Calendar how often to poll for changes.
        Property {
            namespace: "ICAL".into(),
            name: "refreshrate".into(),
            value: Some("PT1M".into()),
        },
    ];

    for p in static_props {
        known.insert(p.name.to_lowercase(), p);
    }

    // Overlay dead properties (e.g. calendar-color, calendar-order set by iOS).
    for (ns_uri, prop_name, value) in dead_props {
        if let Some(prefix) = ns_uri_to_prefix(ns_uri) {
            known.insert(
                prop_name.to_lowercase(),
                Property {
                    namespace: prefix.to_string(),
                    name: prop_name.clone(),
                    value: if value.is_empty() {
                        None
                    } else {
                        Some(tidl_caldav::escape_xml(value))
                    },
                },
            );
        }
    }

    match kind {
        PropFindType::AllProp => {
            vec![PropStat {
                properties: known.into_values().collect(),
                status: "HTTP/1.1 200 OK".into(),
            }]
        }
        PropFindType::Prop(requested) => {
            let mut found: Vec<Property> = vec![];
            let mut not_found: Vec<Property> = vec![];

            for req_name in requested {
                let lower = req_name.to_lowercase();
                match known.get(&lower) {
                    Some(prop) => found.push(prop.clone()),
                    None => not_found.push(Property {
                        namespace: "D".into(),
                        name: req_name.clone(),
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
        PropFindType::PropName => vec![], // handled above
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_storage(pool: SqlitePool) -> Storage {
        let dir = std::env::temp_dir().join(format!("tidl-caldav-test-{}", uuid::Uuid::new_v4()));
        Storage {
            pool,
            data_dir: dir.to_str().unwrap().to_string(),
        }
    }

    // ── sync_vtodo_completion (backlog #71) ───────────────────────────────────

    async fn seed_note_mirror(pool: &SqlitePool, uid: &str) -> i64 {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin)
             VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)",
        )
        .execute(pool)
        .await
        .unwrap();
        let coll_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Personal', '#000000') RETURNING id",
        ).fetch_one(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO notes (collection_id, author_username, text, kind, mirrored_event_uid)
             VALUES (?, 'alice', 'Buy milk', 'task', ?)",
        )
        .bind(coll_id)
        .bind(uid)
        .execute(pool)
        .await
        .unwrap();
        coll_id
    }

    fn vtodo_fields(status: Option<&str>) -> tidl_caldav::EventFields {
        tidl_caldav::EventFields {
            component_type: "VTODO".to_string(),
            status: status.map(str::to_string),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn sync_vtodo_completion_ignores_vevent() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "etag-0",
            )
            .await
            .unwrap();

        let fields = tidl_caldav::EventFields {
            component_type: "VEVENT".to_string(),
            status: Some("CANCELLED".to_string()),
            ..Default::default()
        };
        sync_vtodo_completion(&storage, "alice", "personal", "e1", &fields)
            .await
            .unwrap();

        // status column untouched — still whatever store_event left it as (NULL).
        use sqlx::Row;
        let row = sqlx::query("SELECT status FROM events WHERE uid = 'e1'")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<Option<String>, _>("status").unwrap(), None);
    }

    #[tokio::test]
    async fn sync_vtodo_completion_ignores_missing_status() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "t1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "etag-0",
            )
            .await
            .unwrap();

        sync_vtodo_completion(&storage, "alice", "personal", "t1", &vtodo_fields(None))
            .await
            .unwrap();

        use sqlx::Row;
        let row = sqlx::query("SELECT status FROM events WHERE uid = 't1'")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(
            row.try_get::<Option<String>, _>("status").unwrap(),
            None,
            "no STATUS in the body must leave the column alone"
        );
    }

    #[tokio::test]
    async fn sync_vtodo_completion_completes_a_plain_task() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "t1",
                "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Buy milk\r\nEND:VTODO\r\nEND:VCALENDAR",
                "etag-0",
            )
            .await
            .unwrap();

        sync_vtodo_completion(
            &storage,
            "alice",
            "personal",
            "t1",
            &vtodo_fields(Some("COMPLETED")),
        )
        .await
        .unwrap();

        use sqlx::Row;
        let row = sqlx::query("SELECT status FROM events WHERE uid = 't1'")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("status").unwrap(), "COMPLETED");
        // The row must still exist — only a note mirror gets deleted on completion.
        assert!(storage
            .get_event("alice", "personal", "t1")
            .await
            .unwrap()
            .is_some());
        // The completion is indexed immediately, not just on the next reconcile
        // (backlog #136).
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items \
             WHERE username = 'alice' AND origin = 'caldav' AND origin_key = 't1' \
               AND archived_at IS NULL",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(live, 1);
    }

    #[tokio::test]
    async fn sync_vtodo_completion_uncompletes_a_plain_task() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "t1",
                "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Buy milk\r\nEND:VTODO\r\nEND:VCALENDAR",
                "etag-0",
            )
            .await
            .unwrap();
        sync_vtodo_completion(
            &storage,
            "alice",
            "personal",
            "t1",
            &vtodo_fields(Some("COMPLETED")),
        )
        .await
        .unwrap();

        sync_vtodo_completion(
            &storage,
            "alice",
            "personal",
            "t1",
            &vtodo_fields(Some("NEEDS-ACTION")),
        )
        .await
        .unwrap();

        use sqlx::Row;
        let row = sqlx::query("SELECT status, completed_date FROM events WHERE uid = 't1'")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("status").unwrap(), "NEEDS-ACTION");
        assert_eq!(
            row.try_get::<Option<String>, _>("completed_date").unwrap(),
            None
        );
        // Un-completing archives the live index row rather than leaving it live
        // (backlog #136).
        let archived: Option<String> = sqlx::query_scalar(
            "SELECT CAST(archived_at AS TEXT) FROM done_items \
             WHERE username = 'alice' AND origin = 'caldav' AND origin_key = 't1'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert!(archived.is_some());
    }

    #[tokio::test]
    async fn sync_vtodo_completion_of_a_note_mirror_severs_the_link_and_deletes_the_row() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "note-1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "etag-0",
            )
            .await
            .unwrap();
        let coll_id = seed_note_mirror(&storage.pool, "note-1").await;

        sync_vtodo_completion(
            &storage,
            "alice",
            "personal",
            "note-1",
            &vtodo_fields(Some("COMPLETED")),
        )
        .await
        .unwrap();

        // Mirror row gone, exactly like the app's own completion checkbox (backlog #40).
        assert!(storage
            .get_event("alice", "personal", "note-1")
            .await
            .unwrap()
            .is_none());
        use sqlx::Row;
        let note =
            sqlx::query("SELECT kind, mirrored_event_uid FROM notes WHERE collection_id = ?")
                .bind(coll_id)
                .fetch_one(&storage.pool)
                .await
                .unwrap();
        assert_eq!(note.try_get::<Option<String>, _>("kind").unwrap(), None);
        assert_eq!(
            note.try_get::<Option<String>, _>("mirrored_event_uid")
                .unwrap(),
            None
        );
        // The mirror row (the CalDAV origin) is gone, so no CalDAV done row is
        // indexed (backlog #136).
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND origin = 'caldav'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(live, 0);
    }

    #[test]
    fn parse_shared_alias_extracts_id() {
        assert_eq!(parse_shared_alias("shared-42"), Some(42));
        assert_eq!(parse_shared_alias("shared-1"), Some(1));
    }

    #[test]
    fn parse_shared_alias_rejects_non_alias_names() {
        assert_eq!(parse_shared_alias("personal"), None);
        assert_eq!(parse_shared_alias("shared-"), None);
        assert_eq!(parse_shared_alias("shared-abc"), None);
        assert_eq!(parse_shared_alias("shared-42-extra"), None);
    }

    #[tokio::test]
    async fn resolve_calendar_identity_passes_through_own_calendar() {
        let pool = test_pool().await;
        let storage = test_storage(pool);

        let (owner, calendar) =
            resolve_calendar_identity(&storage, "alice", "personal", Permission::Read)
                .await
                .unwrap();

        assert_eq!(owner, "alice");
        assert_eq!(calendar, "personal");
    }

    #[tokio::test]
    async fn resolve_calendar_identity_resolves_valid_read_share() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        sqlx::query!(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission)
             VALUES ('alice', 'family', 'bob', 'read')"
        ).execute(&storage.pool).await.unwrap();
        let share_id: i64 =
            sqlx::query_scalar!("SELECT id FROM calendar_shares WHERE grantee_username = 'bob'")
                .fetch_one(&storage.pool)
                .await
                .unwrap()
                .unwrap_or(0);

        let alias = format!("shared-{}", share_id);
        let (owner, calendar) =
            resolve_calendar_identity(&storage, "bob", &alias, Permission::Read)
                .await
                .unwrap();

        assert_eq!(owner, "alice");
        assert_eq!(calendar, "family");
    }

    #[tokio::test]
    async fn resolve_calendar_identity_rejects_write_on_read_only_share() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        sqlx::query!(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission)
             VALUES ('alice', 'family', 'bob', 'read')"
        ).execute(&storage.pool).await.unwrap();
        let share_id: i64 =
            sqlx::query_scalar!("SELECT id FROM calendar_shares WHERE grantee_username = 'bob'")
                .fetch_one(&storage.pool)
                .await
                .unwrap()
                .unwrap_or(0);

        let alias = format!("shared-{}", share_id);
        let result = resolve_calendar_identity(&storage, "bob", &alias, Permission::Write).await;

        assert_eq!(result, Err(StatusCode::FORBIDDEN));
    }

    #[tokio::test]
    async fn resolve_calendar_identity_rejects_wrong_grantee() {
        let pool = test_pool().await;
        let storage = test_storage(pool);
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        sqlx::query!(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission)
             VALUES ('alice', 'family', 'bob', 'read-write')"
        ).execute(&storage.pool).await.unwrap();
        let share_id: i64 =
            sqlx::query_scalar!("SELECT id FROM calendar_shares WHERE grantee_username = 'bob'")
                .fetch_one(&storage.pool)
                .await
                .unwrap()
                .unwrap_or(0);

        // "eve" tries to use bob's share-alias ID directly.
        let alias = format!("shared-{}", share_id);
        let result = resolve_calendar_identity(&storage, "eve", &alias, Permission::Read).await;

        assert_eq!(result, Err(StatusCode::FORBIDDEN));
    }

    #[tokio::test]
    async fn resolve_calendar_identity_rejects_unknown_share_id() {
        let pool = test_pool().await;
        let storage = test_storage(pool);

        let result =
            resolve_calendar_identity(&storage, "bob", "shared-999999", Permission::Read).await;

        assert_eq!(result, Err(StatusCode::FORBIDDEN));
    }

    // ── sync-collection response helpers ─────────────────────────────────────

    #[test]
    fn parse_sync_token_from_body_extracts_the_numeric_token() {
        let body = r#"<D:sync-collection xmlns:D="DAV:"><D:sync-token>urn:tidl:sync:7</D:sync-token></D:sync-collection>"#;
        assert_eq!(parse_sync_token_from_body(body), Some(7));
    }

    #[test]
    fn parse_sync_token_from_body_tolerates_a_non_d_namespace_prefix() {
        let body = r#"<A:sync-collection xmlns:A="DAV:"><A:sync-token>urn:tidl:sync:3</A:sync-token></A:sync-collection>"#;
        assert_eq!(parse_sync_token_from_body(body), Some(3));
    }

    #[test]
    fn parse_sync_token_from_body_returns_none_when_absent_or_empty() {
        assert_eq!(parse_sync_token_from_body("<D:sync-collection/>"), None);
        assert_eq!(
            parse_sync_token_from_body("<D:sync-token>urn:tidl:sync:</D:sync-token>"),
            None
        );
        assert_eq!(
            parse_sync_token_from_body("<D:sync-token>garbage</D:sync-token>"),
            None
        );
    }

    #[test]
    fn sync_response_entry_is_200_with_etag_when_present() {
        let entry = sync_response_entry("/caldav/alice/family/e1.ics", Some("\"etag-1\""));
        assert!(entry.contains("<D:href>/caldav/alice/family/e1.ics</D:href>"));
        assert!(entry.contains("<D:getetag>\"etag-1\"</D:getetag>"));
        assert!(entry.contains("HTTP/1.1 200 OK"));
        assert!(!entry.contains("404"));
    }

    #[test]
    fn sync_response_entry_is_404_when_absent() {
        let entry = sync_response_entry("/caldav/alice/family/gone.ics", None);
        assert!(entry.contains("<D:href>/caldav/alice/family/gone.ics</D:href>"));
        assert!(entry.contains("HTTP/1.1 404 Not Found"));
        assert!(!entry.contains("getetag"));
    }

    #[test]
    fn sync_response_entry_escapes_the_href() {
        let entry = sync_response_entry("/caldav/alice/Alice & Bob/e1.ics", Some("\"x\""));
        assert!(entry.contains("Alice &amp; Bob"));
        assert!(!entry.contains("Alice & Bob"));
    }

    #[test]
    fn format_sync_multistatus_puts_sync_token_as_a_direct_child() {
        let entry = sync_response_entry("/caldav/alice/family/e1.ics", Some("\"x\""));
        let xml = format_sync_multistatus(&entry, 7);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<D:multistatus"));
        assert!(xml.contains(&entry));
        assert!(xml.contains("<D:sync-token>urn:tidl:sync:7</D:sync-token>"));

        // RFC 6578 §3.6: `sync-token` is a direct child of `multistatus`, not
        // nested inside a response's propstat.
        let entry_end = xml.find("</D:response>").unwrap() + "</D:response>".len();
        let token_pos = xml.find("<D:sync-token>").unwrap();
        let close_pos = xml.find("</D:multistatus>").unwrap();
        assert!(entry_end < token_pos && token_pos < close_pos);
    }

    #[test]
    fn format_sync_multistatus_is_well_formed_with_no_entries() {
        let xml = format_sync_multistatus("", 0);
        assert!(xml.contains("<D:sync-token>urn:tidl:sync:0</D:sync-token>"));
        assert!(xml.contains("</D:multistatus>"));
    }
}

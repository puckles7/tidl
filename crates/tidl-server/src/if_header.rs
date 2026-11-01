//! RFC 4918 §10.4 `If:` header evaluation and WebDAV destination parsing.
//!
//! This module is intentionally free of HTTP handler logic. All functions
//! that inspect lock state access it through `&AppState` so the module
//! stays testable and keeps `caldav_handlers.rs` focused on request dispatch.

use axum::http::{HeaderMap, StatusCode};

use crate::AppState;

// ── Destination / Overwrite helpers ──────────────────────────────────────────

/// Parse the `Destination` header for a collection COPY/MOVE.
///
/// Returns `(dst_username, dst_calendar, overwrite)`.
/// Accepts both absolute URLs (`http://host/caldav/user/cal`) and absolute
/// paths (`/caldav/user/cal`).
pub fn parse_destination(
    headers: &HeaderMap,
    _src_username: &str,
) -> Option<(String, String, bool)> {
    let dest = headers.get("Destination").and_then(|v| v.to_str().ok())?;

    let path = if dest.starts_with("http://") || dest.starts_with("https://") {
        let without_scheme = dest.splitn(3, '/').nth(2)?; // "host/path"
        without_scheme
            .split_once('/')
            .map(|x| x.1)
            .map(|p| format!("/{}", p))?
    } else {
        dest.to_string()
    };

    let path = path.trim_end_matches('/');
    if !path.starts_with("/caldav/") {
        return None;
    }
    let parts: Vec<&str> = path.trim_start_matches('/').splitn(4, '/').collect();
    if parts.len() < 3 || parts[0] != "caldav" {
        return None;
    }
    let dst_username = parts[1].to_string();
    let dst_calendar = parts[2].to_string();

    let overwrite = overwrite_flag(headers);

    Some((dst_username, dst_calendar, overwrite))
}

/// Parse the `Destination` header for an event COPY/MOVE.
///
/// Returns `(dst_username, dst_calendar, dst_uid, overwrite)`.
pub fn parse_destination_event(
    headers: &HeaderMap,
    _src_username: &str,
) -> Option<(String, String, String, bool)> {
    let dest = headers.get("Destination").and_then(|v| v.to_str().ok())?;

    let path = if dest.starts_with("http://") || dest.starts_with("https://") {
        let without_scheme = dest.splitn(3, '/').nth(2)?;
        without_scheme
            .split_once('/')
            .map(|x| x.1)
            .map(|p| format!("/{}", p))?
    } else {
        dest.to_string()
    };

    // Expect /caldav/{username}/{calendar}/{uid}[/]
    // or     /caldav/{username}/{calendar}/{subcol}/{uid}[/]
    let path = path.trim_end_matches('/');
    if !path.starts_with("/caldav/") {
        return None;
    }
    let parts: Vec<&str> = path.trim_start_matches('/').splitn(6, '/').collect();
    if parts.len() < 4 || parts[0] != "caldav" {
        return None;
    }
    let dst_username = parts[1].to_string();
    // 5-segment path: /caldav/{user}/{cal}/{subcol}/{uid}
    let (dst_calendar, dst_uid) = if parts.len() >= 5 {
        (format!("{}/{}", parts[2], parts[3]), parts[4].to_string())
    } else {
        (parts[2].to_string(), parts[3].to_string())
    };

    let overwrite = overwrite_flag(headers);

    Some((dst_username, dst_calendar, dst_uid, overwrite))
}

/// Read the `Overwrite` header. Returns `true` (overwrite allowed) unless the
/// header is explicitly set to `"F"`. Defaults to `true` per RFC 4918 §10.6.
fn overwrite_flag(headers: &HeaderMap) -> bool {
    headers
        .get("Overwrite")
        .and_then(|v| v.to_str().ok())
        .map(|v| v != "F")
        .unwrap_or(true)
}

/// Map the outcome of a COPY/MOVE onto the correct success status code.
///
/// - Destination did not exist → `201 Created`
/// - Destination existed and was overwritten → `204 No Content`
/// - Destination exists but `Overwrite: F` was set → `412 Precondition Failed`
///   (caller should propagate the `Err`).
pub fn overwrite_status(dst_existed: bool, overwrite: bool) -> Result<StatusCode, StatusCode> {
    if dst_existed && !overwrite {
        Err(StatusCode::PRECONDITION_FAILED)
    } else if dst_existed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::CREATED)
    }
}

// ── RFC 4918 §10.4 If: header evaluation ─────────────────────────────────────

/// Evaluate a WebDAV `If:` header against a resource path (RFC 4918 §10.4).
///
/// Handles both No-tag-list (`(conditions)`) and Tagged-list (`<URL> (conditions)`) forms.
/// Lists are ORed; within each list conditions are ANDed.
///
/// Returns `true` if any list evaluates to true.
pub async fn check_if_header(if_hdr: &str, resource_path: &str, state: &AppState) -> bool {
    let mut pos = 0;
    let mut any_group_found = false;
    // Current tag URL (for tagged lists).
    let mut tag_url: Option<String> = None;

    while pos < if_hdr.len() {
        let rest = if_hdr[pos..].trim_start();
        let skip = if_hdr.len() - pos - rest.len();
        pos += skip;

        if rest.is_empty() {
            break;
        }

        if rest.starts_with('<') {
            // URL tag for tagged list or state token in a group.
            if let Some(end) = rest.find('>') {
                let url_content = &rest[1..end];
                // If it's an absolute URL (tagged list), set the tag.
                if url_content.starts_with("http://") || url_content.starts_with("https://") {
                    // Extract path from URL: strip scheme + host.
                    let without_scheme = url_content.splitn(3, '/').nth(2).unwrap_or("");
                    let url_path = without_scheme
                        .split_once('/')
                        .map(|x| x.1)
                        .map(|p| format!("/{}", p))
                        .unwrap_or_else(|| format!("/{}", without_scheme));
                    tag_url = Some(url_path);
                }
                pos += end + 1;
            } else {
                break;
            }
        } else if rest.starts_with('(') {
            // Condition group.
            if let Some(close) = rest.find(')') {
                let group = &rest[1..close];
                any_group_found = true;
                // Use tag URL as the effective resource path if present.
                let effective_path = tag_url.as_deref().unwrap_or(resource_path);
                if evaluate_if_group(group, effective_path, state).await {
                    return true; // OR: one true group → overall true.
                }
                pos += close + 1;
                // Tagged list: tag applies only to following groups until next tag.
                tag_url = None;
            } else {
                break;
            }
        } else {
            pos += 1;
        }
    }

    // If no groups found (empty If: header), default pass.
    !any_group_found
}

/// Evaluate one `(...)` group from an `If:` header.
/// Returns true if all conditions in the group pass (AND logic within group).
async fn evaluate_if_group(group: &str, resource_path: &str, state: &AppState) -> bool {
    let mut result = true;
    let mut has_condition = false;
    let mut scan = group;

    loop {
        scan = scan.trim_start();
        if scan.is_empty() {
            break;
        }

        let negated = scan.starts_with("Not");
        if negated {
            scan = scan["Not".len()..].trim_start();
        }

        if scan.starts_with('<') {
            if let Some(end) = scan.find('>') {
                let token = &scan[1..end];
                scan = &scan[end + 1..];
                has_condition = true;

                let condition_result = if token.starts_with("opaquelocktoken:") {
                    match state.storage.get_lock(token).await {
                        Ok(Some(lock)) => {
                            lock.resource_path.trim_end_matches('/')
                                == resource_path.trim_end_matches('/')
                        }
                        _ => false,
                    }
                } else if token.starts_with("DAV:") {
                    false
                } else if token.starts_with("http://") || token.starts_with("https://") {
                    // Tagged list URL — not a condition.
                    continue;
                } else {
                    false
                };

                let effective = if negated {
                    !condition_result
                } else {
                    condition_result
                };
                if !effective {
                    result = false;
                }
            } else {
                break;
            }
        } else if scan.starts_with('[') {
            // Etag condition ["..."] — validate against current resource etag.
            if let Some(end) = scan.find(']') {
                let etag_expr = &scan[1..end];
                scan = &scan[end + 1..];
                has_condition = true;

                let etag_val = etag_expr.trim_matches('"');
                let condition_result = if etag_val == "*" {
                    true
                } else {
                    let parts: Vec<&str> = resource_path
                        .trim_start_matches('/')
                        .splitn(5, '/')
                        .collect();
                    if parts.len() >= 4 && parts[0] == "caldav" {
                        let (u, c, uid) = (parts[1], parts[2], parts[3]);
                        let uid_clean = uid.trim_end_matches(".ics");
                        match state.storage.get_event(u, c, uid_clean).await {
                            Ok(Some((_, stored_etag))) => {
                                stored_etag == etag_val
                                    || stored_etag == format!("\"{}\"", etag_val)
                                    || format!("\"{}\"", stored_etag) == etag_val
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                };

                let effective = if negated {
                    !condition_result
                } else {
                    condition_result
                };
                if !effective {
                    result = false;
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if !has_condition {
        true
    } else {
        result
    }
}

/// Check that the `If:` header contains a valid lock token for the resource.
///
/// Used when a resource IS locked — the actual lock token must be present, not
/// just any passing condition.
///
/// Also accepts a collection lock token if the collection depth covers the
/// resource (e.g. a `Depth: infinity` lock on `/lockcoll/` covers `/lockcoll/member.ics`).
pub async fn lock_token_in_if_header(if_hdr: &str, resource_path: &str, state: &AppState) -> bool {
    let rp = resource_path.trim_end_matches('/');
    let mut search = if_hdr;
    while let Some(start) = search.find("opaquelocktoken:") {
        let rest = &search[start..];
        if let Some(end) = rest.find('>') {
            let token = &rest[..end];
            if let Ok(Some(lock)) = state.storage.get_lock(token).await {
                let lrp = lock.resource_path.trim_end_matches('/');
                if lrp == rp || (lock.depth == "infinity" && rp.starts_with(&format!("{}/", lrp))) {
                    return true;
                }
            }
            search = &rest[end + 1..];
        } else {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    // ── parse_destination ───────────────────────────────────────────────────

    #[test]
    fn destination_absolute_path() {
        let h = headers(&[("Destination", "/caldav/alice/work")]);
        assert_eq!(
            parse_destination(&h, "alice"),
            Some(("alice".to_string(), "work".to_string(), true))
        );
    }

    #[test]
    fn destination_absolute_url() {
        let h = headers(&[("Destination", "http://localhost/caldav/alice/work")]);
        assert_eq!(
            parse_destination(&h, "alice"),
            Some(("alice".to_string(), "work".to_string(), true))
        );
    }

    #[test]
    fn destination_trailing_slash_stripped() {
        let h = headers(&[("Destination", "/caldav/alice/work/")]);
        assert_eq!(
            parse_destination(&h, "alice"),
            Some(("alice".to_string(), "work".to_string(), true))
        );
    }

    #[test]
    fn destination_overwrite_false() {
        let h = headers(&[("Destination", "/caldav/alice/work"), ("Overwrite", "F")]);
        assert_eq!(
            parse_destination(&h, "alice"),
            Some(("alice".to_string(), "work".to_string(), false))
        );
    }

    #[test]
    fn destination_no_header_returns_none() {
        assert_eq!(parse_destination(&HeaderMap::new(), "alice"), None);
    }

    #[test]
    fn destination_non_caldav_path_returns_none() {
        let h = headers(&[("Destination", "/other/path")]);
        assert_eq!(parse_destination(&h, "alice"), None);
    }

    // ── parse_destination_event ─────────────────────────────────────────────

    #[test]
    fn destination_event_4_segments() {
        let h = headers(&[("Destination", "/caldav/alice/work/event123.ics")]);
        assert_eq!(
            parse_destination_event(&h, "alice"),
            Some((
                "alice".to_string(),
                "work".to_string(),
                "event123.ics".to_string(),
                true
            ))
        );
    }

    #[test]
    fn destination_event_absolute_url() {
        let h = headers(&[("Destination", "http://localhost/caldav/bob/personal/ev.ics")]);
        assert_eq!(
            parse_destination_event(&h, "bob"),
            Some((
                "bob".to_string(),
                "personal".to_string(),
                "ev.ics".to_string(),
                true
            ))
        );
    }

    #[test]
    fn destination_event_5_segments_joins_calendar() {
        let h = headers(&[("Destination", "/caldav/alice/shared/sub/event.ics")]);
        assert_eq!(
            parse_destination_event(&h, "alice"),
            Some((
                "alice".to_string(),
                "shared/sub".to_string(),
                "event.ics".to_string(),
                true
            ))
        );
    }

    #[test]
    fn destination_event_overwrite_false() {
        let h = headers(&[
            ("Destination", "/caldav/alice/work/ev.ics"),
            ("Overwrite", "F"),
        ]);
        let (_, _, _, overwrite) = parse_destination_event(&h, "alice").unwrap();
        assert!(!overwrite);
    }

    // ── overwrite_status ────────────────────────────────────────────────────

    #[test]
    fn overwrite_status_new_resource() {
        assert_eq!(overwrite_status(false, true), Ok(StatusCode::CREATED));
    }

    #[test]
    fn overwrite_status_overwrite_existing() {
        assert_eq!(overwrite_status(true, true), Ok(StatusCode::NO_CONTENT));
    }

    #[test]
    fn overwrite_status_no_overwrite_conflict() {
        assert_eq!(
            overwrite_status(true, false),
            Err(StatusCode::PRECONDITION_FAILED)
        );
    }

    #[test]
    fn overwrite_status_no_overwrite_new_resource() {
        // Destination doesn't exist, Overwrite: F — creation still succeeds.
        assert_eq!(overwrite_status(false, false), Ok(StatusCode::CREATED));
    }
}

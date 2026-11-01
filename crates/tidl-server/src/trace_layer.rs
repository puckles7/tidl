//! Axum middleware that logs every CalDAV/CardDAV request and its response status.
//!
//! Enable with `RUST_LOG=tidl_server=trace` (or `=debug` for bodies).
//! Only active for paths starting with `/caldav`, `/carddav`, or `/.well-known`.

use axum::{
    body::Body,
    http::{Request, Response},
    middleware::Next,
};
use tracing::{debug, enabled, Level};

/// Log the full request (headers + body) and response status for CalDAV/CardDAV paths.
pub async fn caldav_trace(req: Request<Body>, next: Next) -> Response<Body> {
    let path = req.uri().path().to_string();
    // Named `is_caldav` for historical reasons (this layer predates CardDAV,
    // backlog #29) — covers both DAV flavors sharing this middleware.
    let is_caldav = path.starts_with("/caldav")
        || path.starts_with("/carddav")
        || path.starts_with("/.well-known");

    if !is_caldav {
        return next.run(req).await;
    }

    let method = req.method().clone();
    let uri = req.uri().clone();

    // Capture notable headers.
    let depth = header_str(req.headers(), "depth");
    let if_match = header_str(req.headers(), "if-match");
    let if_none_match = header_str(req.headers(), "if-none-match");
    let if_hdr = header_str(req.headers(), "if");
    let content_type = header_str(req.headers(), "content-type");
    let destination = header_str(req.headers(), "destination");

    // Buffer request body if debug logging is on.
    // Read the full body so it can be forwarded; only log up to 32 KB.
    let (req, body_text) = if enabled!(Level::DEBUG) {
        let (parts, body) = req.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .unwrap_or_default();
        let text = if bytes.len() > 32 * 1024 {
            format!("[{} bytes — truncated for logging]", bytes.len())
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let req = Request::from_parts(parts, Body::from(bytes));
        (req, text)
    } else {
        (req, String::new())
    };

    debug!(
        method  = %method,
        path    = %uri,
        depth   = %depth,
        if_match = %if_match,
        if_none_match = %if_none_match,
        if_hdr  = %if_hdr,
        content_type = %content_type,
        destination = %destination,
        body    = %body_text,
        "→ CalDAV request"
    );

    let response = next.run(req).await;

    let status = response.status();
    let etag = header_str(response.headers(), "etag");
    let location = header_str(response.headers(), "location");

    // Buffer response body for error responses and REPORT responses (to debug CalDAV sync).
    // Read the FULL body (no limit) so the response is never silently truncated;
    // only truncate the *log* message to 32 KB.
    let is_report = method.as_str() == "REPORT";
    let (response, resp_body_text) = if enabled!(Level::DEBUG)
        && (!status.is_success() && status.as_u16() != 207 || is_report)
    {
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .unwrap_or_default();
        let text = if bytes.len() > 32 * 1024 {
            format!("[{} bytes — truncated for logging]", bytes.len())
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let response = Response::from_parts(parts, Body::from(bytes));
        (response, text)
    } else {
        (response, String::new())
    };

    debug!(
        status  = %status,
        etag    = %etag,
        location = %location,
        resp_body = %resp_body_text,
        "← CalDAV response"
    );

    response
}

fn header_str(headers: &axum::http::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

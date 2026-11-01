//! Cookie-based session authentication for the web UI.

use axum::http::{header, HeaderMap};

use tidl_web::ssr_types::SessionCookieName;

/// Extract the raw session token from a `Cookie` header, if present.
pub fn session_token_from_headers(
    headers: &HeaderMap,
    cookie_name: &SessionCookieName,
) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie_name.extract(cookies)
}

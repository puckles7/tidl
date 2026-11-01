//! Minimal CalDAV HTTP client for syncing with a remote tidl instance.
//!
//! Supports:
//! - `sync_collection` REPORT (RFC 6578) for efficient delta sync
//! - GET to fetch individual events
//! - PUT to create/update events (with ETag concurrency)
//! - DELETE to remove events
//!
//! Authentication is HTTP Basic with `username:app_token`.

use anyhow::{bail, Context, Result};
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, IF_MATCH},
    Client, Method, StatusCode,
};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A change entry returned by `sync_collection`.
#[derive(Debug, Clone)]
pub struct RemoteSyncChange {
    pub href: String,
    pub status: Option<String>, // None = deleted
    #[allow(dead_code)] // available for conditional-fetch optimisation
    pub etag: Option<String>,
}

/// Result of a `sync_collection` REPORT.
#[derive(Debug)]
pub struct SyncResult {
    pub changes: Vec<RemoteSyncChange>,
    pub new_sync_token: Option<String>,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct CalDavClient {
    base_url: String,
    auth_header: HeaderValue,
    http: Client,
}

impl CalDavClient {
    pub fn new(base_url: &str, username: &str, token: &str) -> Result<Self> {
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", username, token),
        );
        let auth_header = HeaderValue::from_str(&format!("Basic {}", credentials))
            .context("building auth header")?;

        let http = crate::ssrf_guard::guarded_client_builder()
            .build()
            .context("building HTTP client")?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_header,
            http,
        })
    }

    // ── Low-level helpers ─────────────────────────────────────────────────────

    fn calendar_url(&self, username: &str, calendar: &str) -> String {
        format!("{}/caldav/{}/{}/", self.base_url, username, calendar)
    }

    fn event_url(&self, username: &str, calendar: &str, uid: &str) -> String {
        let uid = uid.strip_suffix(".ics").unwrap_or(uid);
        format!(
            "{}/caldav/{}/{}/{}.ics",
            self.base_url, username, calendar, uid
        )
    }

    async fn request(
        &self,
        method: Method,
        url: &str,
        body: Option<&str>,
        extra_headers: Option<HeaderMap>,
        content_type: Option<&str>,
    ) -> Result<reqwest::Response> {
        let mut req = self
            .http
            .request(method, url)
            .header(AUTHORIZATION, self.auth_header.clone());

        if let Some(ct) = content_type {
            req = req.header(CONTENT_TYPE, ct);
        }
        if let Some(hdrs) = extra_headers {
            for (k, v) in &hdrs {
                req = req.header(k, v);
            }
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        req.send().await.context("HTTP request failed")
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Fetch changes since `sync_token` using RFC 6578 `sync-collection`.
    /// Pass `None` for a full sync (server returns all events + a sync token).
    pub async fn sync_collection(
        &self,
        username: &str,
        calendar: &str,
        sync_token: Option<&str>,
    ) -> Result<SyncResult> {
        let url = self.calendar_url(username, calendar);
        let token_el = match sync_token {
            Some(t) => format!("<D:sync-token>{}</D:sync-token>", t),
            None => "<D:sync-token/>".to_string(),
        };

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:">
  {token_el}
  <D:sync-level>1</D:sync-level>
  <D:prop>
    <D:getetag/>
  </D:prop>
</D:sync-collection>"#
        );

        let method = Method::from_bytes(b"REPORT").unwrap();
        let resp = self
            .request(method, &url, Some(&body), None, Some("application/xml"))
            .await?;

        if resp.status() != StatusCode::MULTI_STATUS {
            bail!("sync-collection failed: {}", resp.status());
        }

        let text = resp
            .text()
            .await
            .context("reading sync-collection response")?;
        parse_sync_result(&text)
    }

    /// GET a single `.ics` resource. Returns `(ical_data, etag)`.
    pub async fn get_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
    ) -> Result<(String, String)> {
        let url = self.event_url(username, calendar, uid);
        let resp = self.request(Method::GET, &url, None, None, None).await?;

        if resp.status() != StatusCode::OK {
            bail!("GET {} failed: {}", url, resp.status());
        }

        let etag = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp.text().await.context("reading event body")?;
        Ok((body, etag))
    }

    /// PUT a `.ics` resource.  `etag` is the current ETag for If-Match (use
    /// `None` to create a new resource, `Some("*")` to force-overwrite).
    /// Returns the new ETag.
    pub async fn put_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        ical: &str,
        etag: Option<&str>,
    ) -> Result<String> {
        let url = self.event_url(username, calendar, uid);
        let mut extra = HeaderMap::new();
        match etag {
            Some(e) => {
                extra.insert(IF_MATCH, HeaderValue::from_str(e).context("bad etag")?);
            }
            None => {
                extra.insert("If-None-Match", HeaderValue::from_static("*"));
            }
        }

        let resp = self
            .request(
                Method::PUT,
                &url,
                Some(ical),
                Some(extra),
                Some("text/calendar; charset=utf-8"),
            )
            .await?;

        match resp.status() {
            StatusCode::CREATED | StatusCode::NO_CONTENT => {
                let new_etag = resp
                    .headers()
                    .get("ETag")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                Ok(new_etag)
            }
            StatusCode::PRECONDITION_FAILED => {
                bail!("PUT {}: ETag mismatch (precondition failed)", url)
            }
            s => bail!("PUT {} failed: {}", url, s),
        }
    }

    /// DELETE a `.ics` resource. `etag` is passed in If-Match if provided.
    pub async fn delete_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        etag: Option<&str>,
    ) -> Result<()> {
        let url = self.event_url(username, calendar, uid);
        let mut extra = HeaderMap::new();
        if let Some(e) = etag {
            extra.insert(IF_MATCH, HeaderValue::from_str(e).context("bad etag")?);
        }

        let resp = self
            .request(Method::DELETE, &url, None, Some(extra), None)
            .await?;

        match resp.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Ok(()), // already gone — treat as success
            s => bail!("DELETE {} failed: {}", url, s),
        }
    }
}

// ── XML parsing helpers ───────────────────────────────────────────────────────

fn parse_sync_result(xml: &str) -> Result<SyncResult> {
    // Extract sync-token from the response.
    let new_sync_token = extract_between(xml, "<D:sync-token>", "</D:sync-token>")
        .or_else(|| extract_between(xml, "<sync-token>", "</sync-token>"))
        .map(|s| s.trim().to_string());

    let changes = parse_responses_from_multistatus(xml)?
        .into_iter()
        .map(|(href, status, etag)| RemoteSyncChange {
            href,
            status: Some(status),
            etag,
        })
        .collect();

    Ok(SyncResult {
        changes,
        new_sync_token,
    })
}

/// Extract `(href, status_line, etag?)` from each `<D:response>` block.
fn parse_responses_from_multistatus(xml: &str) -> Result<Vec<(String, String, Option<String>)>> {
    let mut results = Vec::new();
    let mut rest = xml;

    while let Some(start) = rest
        .find("<D:response>")
        .or_else(|| rest.find("<response>"))
    {
        let tag_end = if rest[start..].starts_with("<D:") {
            "</D:response>"
        } else {
            "</response>"
        };
        let block_start = start;
        let block_end = rest[block_start..]
            .find(tag_end)
            .map(|i| block_start + i + tag_end.len())
            .unwrap_or(rest.len());

        let block = &rest[block_start..block_end];

        let href = extract_between(block, "<D:href>", "</D:href>")
            .or_else(|| extract_between(block, "<href>", "</href>"))
            .unwrap_or("")
            .trim()
            .to_string();

        let status = extract_between(block, "<D:status>", "</D:status>")
            .or_else(|| extract_between(block, "<status>", "</status>"))
            .unwrap_or("HTTP/1.1 200 OK")
            .trim()
            .to_string();

        let etag = extract_between(block, "<D:getetag>", "</D:getetag>")
            .or_else(|| extract_between(block, "<getetag>", "</getetag>"))
            .map(|s| s.trim().trim_matches('"').to_string());

        if !href.is_empty() {
            results.push((href, status, etag));
        }

        rest = &rest[block_end..];
    }

    Ok(results)
}

fn extract_between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open).map(|i| i + open.len())?;
    let end = s[start..].find(close).map(|i| i + start)?;
    Some(&s[start..end])
}

//! Hand-rolled async SMTP server for inbound iMIP email processing.
//!
//! Accepts email with STARTTLS, validates the inbound token, checks SPF and
//! trusted-sender lists, enforces rate limits, then imports the iCalendar
//! payload into the target calendar.

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use mail_parser::MimeHeaders;

use crate::config::InboundSmtpConfig;
use crate::mailer::Mailer;
use crate::user_store::UserStore;

// ── Public entry point ───────────────────────────────────────────────────────

pub async fn run_smtp_listener(
    config: InboundSmtpConfig,
    pool: sqlx::SqlitePool,
    users: UserStore,
    change_tx: tokio::sync::broadcast::Sender<String>,
    inbound_rate_limiter: Arc<DashMap<String, VecDeque<Instant>>>,
    _mailer: Arc<dyn Mailer + Send + Sync>,
) {
    let tls_config = match load_tls_config(&config.cert_path, &config.key_path).await {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::error!("SMTP listener: failed to load TLS config: {e}");
            return;
        }
    };

    let acceptor = TlsAcceptor::from(tls_config);

    let bind_addr: SocketAddr = ([0, 0, 0, 0], config.port).into();
    let listener = match TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("SMTP listener: failed to bind on port {}: {e}", config.port);
            return;
        }
    };

    tracing::info!("SMTP listener running on port {}", config.port);

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("SMTP listener: accept error: {e}");
                continue;
            }
        };

        let config = config.clone();
        let acceptor = acceptor.clone();
        let pool = pool.clone();
        let users = users.clone();
        let change_tx = change_tx.clone();
        let rate_limiter = Arc::clone(&inbound_rate_limiter);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(
                stream,
                peer_addr,
                &config,
                acceptor,
                &pool,
                &users,
                &change_tx,
                &rate_limiter,
            )
            .await
            {
                tracing::warn!("SMTP connection from {peer_addr} ended with error: {e}");
            }
        });
    }
}

// ── TLS setup ────────────────────────────────────────────────────────────────

async fn load_tls_config(cert_path: &str, key_path: &str) -> Result<rustls::ServerConfig> {
    let cert_pem = tokio::fs::read(cert_path).await?;
    let key_pem = tokio::fs::read(key_path).await?;

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut &cert_pem[..]).collect::<std::result::Result<Vec<_>, _>>()?;

    let key = rustls_pemfile::private_key(&mut &key_pem[..])?
        .ok_or_else(|| anyhow!("no private key found in {key_path}"))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(config)
}

// ── Per-connection handler ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)] // one caller; params are distinct connection-scoped resources, not a bundle
async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    config: &InboundSmtpConfig,
    acceptor: TlsAcceptor,
    pool: &sqlx::SqlitePool,
    users: &UserStore,
    change_tx: &tokio::sync::broadcast::Sender<String>,
    rate_limiter: &DashMap<String, VecDeque<Instant>>,
) -> Result<()> {
    let peer_ip = peer_addr.ip();

    // Phase 1: plaintext — greeting + EHLO + STARTTLS negotiation
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // Send greeting
    send_line(&mut writer, &format!("220 {} ESMTP tidl", config.domain)).await?;

    // Read EHLO
    let line = read_line(&mut reader).await?;
    if !line.to_ascii_uppercase().starts_with("EHLO") {
        send_line(&mut writer, "503 5.5.1 Expected EHLO").await?;
        return Ok(());
    }

    // Send pre-TLS capabilities
    send_line(&mut writer, &format!("250-{}", config.domain)).await?;
    send_line(&mut writer, "250-STARTTLS").await?;
    send_line(
        &mut writer,
        &format!("250-SIZE {}", config.max_message_bytes),
    )
    .await?;
    send_line(&mut writer, "250 8BITMIME").await?;

    // Wait for STARTTLS (reject MAIL FROM before TLS)
    loop {
        let line = read_line(&mut reader).await?;
        let upper = line.to_ascii_uppercase();

        if upper.starts_with("STARTTLS") {
            send_line(&mut writer, "220 Go ahead").await?;
            break;
        } else if upper.starts_with("MAIL FROM") {
            send_line(&mut writer, "530 5.7.0 Must issue a STARTTLS command first").await?;
            continue;
        } else if upper.starts_with("QUIT") {
            send_line(&mut writer, "221 Bye").await?;
            return Ok(());
        } else {
            send_line(&mut writer, "503 5.5.1 Expected STARTTLS").await?;
            continue;
        }
    }

    // Reassemble the TcpStream from the split halves, then do TLS handshake.
    let tcp_stream = reader.into_inner().unsplit(writer);
    let tls_stream = acceptor.accept(tcp_stream).await?;
    let (tls_reader, mut tls_writer) = tokio::io::split(tls_stream);
    let mut tls_reader = BufReader::new(tls_reader);

    // Phase 2: post-TLS — require new EHLO per RFC 3207
    let line = read_line(&mut tls_reader).await?;
    if !line.to_ascii_uppercase().starts_with("EHLO") {
        send_line(&mut tls_writer, "503 5.5.1 Expected EHLO after STARTTLS").await?;
        return Ok(());
    }
    let helo_domain = line
        .split_whitespace()
        .nth(1)
        .unwrap_or(&config.domain)
        .to_string();

    // Post-TLS capabilities (no STARTTLS)
    send_line(&mut tls_writer, &format!("250-{}", config.domain)).await?;
    send_line(
        &mut tls_writer,
        &format!("250-SIZE {}", config.max_message_bytes),
    )
    .await?;
    send_line(&mut tls_writer, "250 8BITMIME").await?;

    // Phase 3: envelope
    let envelope_from;
    let raw_token_hex: String;

    // Read MAIL FROM
    loop {
        let line = read_line(&mut tls_reader).await?;
        let upper = line.to_ascii_uppercase();

        if upper.starts_with("MAIL FROM") {
            envelope_from = extract_address(&line);
            send_line(&mut tls_writer, "250 OK").await?;
            break;
        } else if upper.starts_with("QUIT") {
            send_line(&mut tls_writer, "221 Bye").await?;
            return Ok(());
        } else {
            send_line(&mut tls_writer, "503 5.5.1 Expected MAIL FROM").await?;
            continue;
        }
    }

    // Read RCPT TO
    loop {
        let line = read_line(&mut tls_reader).await?;
        let upper = line.to_ascii_uppercase();

        if upper.starts_with("RCPT TO") {
            let rcpt_addr = extract_address(&line);
            let local_part = rcpt_addr.split('@').next().unwrap_or("");

            if !local_part.starts_with("tidl_") {
                send_line(&mut tls_writer, "550 5.1.1 User unknown").await?;
                return Ok(());
            }

            raw_token_hex = local_part.strip_prefix("tidl_").unwrap_or("").to_string();
            send_line(&mut tls_writer, "250 OK").await?;
            break;
        } else if upper.starts_with("QUIT") {
            send_line(&mut tls_writer, "221 Bye").await?;
            return Ok(());
        } else {
            send_line(&mut tls_writer, "503 5.5.1 Expected RCPT TO").await?;
            continue;
        }
    }

    // Read DATA
    loop {
        let line = read_line(&mut tls_reader).await?;
        let upper = line.to_ascii_uppercase();

        if upper.starts_with("DATA") {
            send_line(&mut tls_writer, "354 End data with <CR><LF>.<CR><LF>").await?;
            break;
        } else if upper.starts_with("QUIT") {
            send_line(&mut tls_writer, "221 Bye").await?;
            return Ok(());
        } else {
            send_line(&mut tls_writer, "503 5.5.1 Expected DATA").await?;
            continue;
        }
    }

    // Read message body until lone dot
    let mut body = Vec::new();
    loop {
        let line = read_line(&mut tls_reader).await?;
        if line == "." {
            break;
        }
        // Dot-stuffing: line starting with ".." means literal "."
        let data_line = if let Some(stripped) = line.strip_prefix('.') {
            stripped
        } else {
            &line
        };
        body.extend_from_slice(data_line.as_bytes());
        body.extend_from_slice(b"\r\n");

        if body.len() > config.max_message_bytes {
            send_line(&mut tls_writer, "552 5.3.4 Message too large").await?;
            return Ok(());
        }
    }

    // ── Post-DATA checks ─────────────────────────────────────────────────────

    // (a) Validate token
    let full_token = format!("tidl_{}", raw_token_hex);
    let (username, calendar_id) = match users.validate_inbound_email_token(&full_token).await {
        Ok(Some((u, cid))) => (u, cid),
        Ok(None) => {
            send_line(&mut tls_writer, "550 5.1.1 Invalid token").await?;
            return Ok(());
        }
        Err(e) => {
            tracing::error!("SMTP: token validation error: {e}");
            send_line(&mut tls_writer, "451 4.3.0 Internal error").await?;
            return Ok(());
        }
    };

    // (b) Rate limit
    {
        let key = full_token.clone();
        let now = Instant::now();
        let one_hour = Duration::from_secs(3600);

        let mut entry = rate_limiter.entry(key).or_default();
        // Prune entries older than 1 hour
        while entry
            .front()
            .is_some_and(|t| now.duration_since(*t) > one_hour)
        {
            entry.pop_front();
        }

        if entry.len() >= config.rate_limit_per_hour as usize {
            send_line(&mut tls_writer, "452 4.5.3 Rate limit exceeded").await?;
            return Ok(());
        }
        entry.push_back(now);
    }

    // (c) SPF check
    if !check_spf(peer_ip, &helo_domain, &envelope_from).await {
        tracing::warn!(
            "SMTP: SPF check failed for {} from {}",
            envelope_from,
            peer_ip
        );
        send_line(&mut tls_writer, "550 5.7.23 SPF validation failed").await?;
        return Ok(());
    }

    // (d) Trusted sender check
    match users.is_trusted_sender(&username, &envelope_from).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(
                "SMTP: sender {} not trusted for user {}",
                envelope_from,
                username
            );
            send_line(&mut tls_writer, "550 5.7.1 Sender not in trusted list").await?;
            return Ok(());
        }
        Err(e) => {
            tracing::error!("SMTP: trusted sender check error: {e}");
            send_line(&mut tls_writer, "451 4.3.0 Internal error").await?;
            return Ok(());
        }
    }

    // All checks passed
    send_line(&mut tls_writer, "250 2.0.0 OK").await?;

    // Handle QUIT
    if let Ok(line) = read_line(&mut tls_reader).await {
        if line.to_ascii_uppercase().starts_with("QUIT") {
            let _ = send_line(&mut tls_writer, "221 Bye").await;
        }
    }

    // Spawn async processing of the email body
    let pool = pool.clone();
    let change_tx = change_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = process_email_body(&pool, calendar_id, &username, &body, &change_tx).await {
            tracing::error!("SMTP: failed to process email body: {e}");
        }
    });

    Ok(())
}

// ── I/O helpers ──────────────────────────────────────────────────────────────

async fn send_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\r\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn read_line<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Result<String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err(anyhow!("connection closed"));
    }
    // Strip trailing \r\n or \n
    if line.ends_with('\n') {
        line.pop();
    }
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

/// Extract the email address from a `MAIL FROM:<addr>` or `RCPT TO:<addr>` line.
fn extract_address(line: &str) -> String {
    // Try angle-bracket form first
    if let Some(start) = line.find('<') {
        if let Some(end) = line.find('>') {
            return line[start + 1..end].to_string();
        }
    }
    // Fall back to bare address after the colon
    if let Some(colon) = line.find(':') {
        return line[colon + 1..].trim().to_string();
    }
    line.to_string()
}

// ── SPF check ────────────────────────────────────────────────────────────────

/// Returns `true` if SPF passes, `false` for every other RFC 7208 outcome
/// (`Fail`/`SoftFail`/`Neutral`/`None`/`PermError`/`TempError` are all
/// treated as "don't accept this message").
async fn check_spf(peer_ip: IpAddr, helo_domain: &str, envelope_from: &str) -> bool {
    use hickory_resolver::TokioResolver;
    use mailrs_spf::{verify, HickoryResolver, SpfResult, VerifyInput};

    let resolver = match TokioResolver::builder_tokio().and_then(|b| b.build()) {
        Ok(resolver) => HickoryResolver::new(resolver),
        Err(e) => {
            tracing::warn!("SMTP: failed to build DNS resolver for SPF check: {e}");
            return false;
        }
    };

    let input = VerifyInput {
        ip: peer_ip,
        helo: helo_domain.to_string(),
        mail_from: envelope_from.to_string(),
    };

    verify(&resolver, &input).await == SpfResult::Pass
}

// ── Email body processing ────────────────────────────────────────────────────

async fn process_email_body(
    pool: &sqlx::SqlitePool,
    calendar_id: i64,
    username: &str,
    body: &[u8],
    change_tx: &tokio::sync::broadcast::Sender<String>,
) -> Result<()> {
    let message = mail_parser::MessageParser::default()
        .parse(body)
        .ok_or_else(|| anyhow!("failed to parse email"))?;

    // Find the first text/calendar part
    let ical_content =
        find_ical_part(&message).ok_or_else(|| anyhow!("no text/calendar part found in email"))?;

    // Detect METHOD
    let method = extract_ical_method(&ical_content);

    match method.as_deref() {
        Some("CANCEL") => {
            cancel_ical_event(pool, calendar_id, &ical_content).await?;
            tracing::info!(
                "SMTP: cancelled event for user {} in calendar {}",
                username,
                calendar_id
            );
        }
        _ => {
            // REQUEST or any other method → import
            import_ical_event(pool, calendar_id, &ical_content, change_tx).await?;
            tracing::info!(
                "SMTP: imported event for user {} in calendar {}",
                username,
                calendar_id
            );
        }
    }

    Ok(())
}

fn find_ical_part(message: &mail_parser::Message<'_>) -> Option<String> {
    // Check each MIME part for text/calendar content type
    for part in message.parts.iter() {
        if part.is_content_type("text", "calendar") {
            if let Some(text) = part.text_contents() {
                return Some(text.to_string());
            }
        }
    }
    // Also check attachments
    for attachment in message.attachments() {
        if attachment.is_content_type("text", "calendar") {
            return Some(String::from_utf8_lossy(attachment.contents()).to_string());
        }
    }
    None
}

fn extract_ical_method(ical: &str) -> Option<String> {
    for line in ical.lines() {
        let trimmed = line.trim();
        if let Some(method) = trimmed.strip_prefix("METHOD:") {
            return Some(method.trim().to_ascii_uppercase());
        }
    }
    None
}

// ── iCalendar import/cancel ──────────────────────────────────────────────────

async fn import_ical_event(
    pool: &sqlx::SqlitePool,
    calendar_id: i64,
    ical: &str,
    change_tx: &tokio::sync::broadcast::Sender<String>,
) -> Result<()> {
    use tidl_caldav::etag::generate_etag;
    use tidl_caldav::icalendar_utils::{extract_event_fields, extract_uid};

    let uid = extract_uid(ical).map_err(|e| anyhow!("failed to extract UID: {e}"))?;
    let fields = extract_event_fields(ical);
    let etag = generate_etag(ical);
    let event_type = if fields.component_type.is_empty() {
        "VEVENT"
    } else {
        &fields.component_type
    };

    // Look up the calendar's username and name from calendar_id
    let cal_row = sqlx::query("SELECT username, name FROM calendars WHERE id = ?")
        .bind(calendar_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("calendar {} not found", calendar_id))?;

    let cal_username: String = sqlx::Row::get(&cal_row, "username");
    let cal_name: String = sqlx::Row::get(&cal_row, "name");

    sqlx::query(
        r#"INSERT OR REPLACE INTO events
           (username, calendar, uid, type, summary, description, location, priority,
            start_date, end_date, etag, ical_data, source, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?,
                   datetime(?), datetime(?), ?, ?, 'imip', datetime('now'))"#,
    )
    .bind(&cal_username)
    .bind(&cal_name)
    .bind(&uid)
    .bind(event_type)
    .bind(&fields.summary)
    .bind(&fields.description)
    .bind(&fields.location)
    .bind(
        fields
            .priority
            .filter(|p| *p != 0)
            .map(tidl_web::types::caldav_priority_to_source_priority)
            .map(i64::from),
    )
    .bind(&fields.start_date)
    .bind(&fields.end_date)
    .bind(&etag)
    .bind(ical)
    .execute(pool)
    .await?;

    let _ = change_tx.send(tidl_caldav::change::ChangeKind::Events.to_msg(&cal_username));
    let grantees: Vec<String> = sqlx::query_scalar(
        "SELECT grantee_username FROM calendar_shares WHERE owner_username = ? AND calendar_name = ?",
    )
    .bind(&cal_username)
    .bind(&cal_name)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for grantee in grantees {
        let _ = change_tx.send(tidl_caldav::change::ChangeKind::Events.to_msg(&grantee));
    }

    Ok(())
}

async fn cancel_ical_event(pool: &sqlx::SqlitePool, calendar_id: i64, ical: &str) -> Result<()> {
    use tidl_caldav::icalendar_utils::extract_uid;

    let uid = extract_uid(ical).map_err(|e| anyhow!("failed to extract UID: {e}"))?;

    // Look up the calendar's username and name
    let cal_row = sqlx::query("SELECT username, name FROM calendars WHERE id = ?")
        .bind(calendar_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("calendar {} not found", calendar_id))?;

    let cal_username: String = sqlx::Row::get(&cal_row, "username");
    let cal_name: String = sqlx::Row::get(&cal_row, "name");

    sqlx::query("DELETE FROM events WHERE uid = ? AND username = ? AND calendar = ?")
        .bind(&uid)
        .bind(&cal_username)
        .bind(&cal_name)
        .execute(pool)
        .await?;

    Ok(())
}

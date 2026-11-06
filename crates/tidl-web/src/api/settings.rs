//! Preferences, SMTP configuration, and app token server functions.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::{auth, auth_admin, current_user};
#[cfg(feature = "ssr")]
use crate::ssr_types::{SmtpConfigRow, SMTP_CONFIG_QUERY};

use super::{
    AppTokenInfo, CurvePoint, InboundEmailTokenInfo, PriorityCurves, SmtpConfig, TrustedSenderInfo,
    UserPrefs,
};

// ── User preferences ──────────────────────────────────────────────────────────

/// Return the display preferences for the current user.
#[server]
pub async fn get_preferences() -> Result<UserPrefs, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let row = sqlx::query(
            "SELECT timezone, time_format, dark_mode, open_in_editor_cmd, show_sidebar_hints FROM user_preferences WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(row
            .map(|r| UserPrefs {
                timezone: r.get("timezone"),
                time_format: r.get("time_format"),
                dark_mode: r.get::<i64, _>("dark_mode") != 0,
                open_in_editor_cmd: r.try_get("open_in_editor_cmd").ok().flatten(),
                show_sidebar_hints: r
                    .try_get::<i64, _>("show_sidebar_hints")
                    .map(|v| v != 0)
                    .unwrap_or(true),
            })
            .unwrap_or_default());
    }
    #[allow(unreachable_code)]
    Ok(UserPrefs::default())
}

/// Persist display preferences for the current user.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn save_preferences(
    timezone: String,
    time_format: String,
    dark_mode: bool,
    open_in_editor_cmd: Option<String>,
    show_sidebar_hints: bool,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "INSERT INTO user_preferences (username, timezone, time_format, dark_mode, open_in_editor_cmd, show_sidebar_hints) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(username) DO UPDATE SET \
             timezone            = excluded.timezone, \
             time_format         = excluded.time_format, \
             dark_mode           = excluded.dark_mode, \
             open_in_editor_cmd  = excluded.open_in_editor_cmd, \
             show_sidebar_hints  = excluded.show_sidebar_hints, \
             updated_at          = CURRENT_TIMESTAMP",
        )
        .bind(&username)
        .bind(&timezone)
        .bind(&time_format)
        .bind(dark_mode as i64)
        .bind(open_in_editor_cmd)
        .bind(show_sidebar_hints as i64)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Priority curves (backlog #68) ─────────────────────────────────────────────

#[cfg(feature = "ssr")]
const PRIORITY_CURVE_KINDS: [&str; 4] = [
    "calendar_time_left",
    "due_date_time_left",
    "source_priority",
    "calendar_event_priority",
];

/// Reads a user's saved control points for every curve kind, falling back to
/// `PriorityCurves::default()`'s points for any kind with no saved rows.
/// `pub(crate)` so `api::events::get_markdown_rendered` can colour rendered
/// markdown chips from the same curves Notes/Focus use (backlog #116).
#[cfg(feature = "ssr")]
pub(crate) async fn fetch_priority_curves(
    pool: &sqlx::SqlitePool,
    username: &str,
) -> Result<PriorityCurves, ServerFnError> {
    let mut curves = PriorityCurves::default();
    for kind in PRIORITY_CURVE_KINDS {
        let rows: Vec<(f64, f64)> = sqlx::query_as(
            "SELECT x, y FROM priority_curve_points WHERE username = ? AND curve_kind = ? ORDER BY x",
        )
        .bind(username)
        .bind(kind)
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if rows.is_empty() {
            continue;
        }
        let points: Vec<CurvePoint> = rows.into_iter().map(|(x, y)| CurvePoint { x, y }).collect();
        match kind {
            "calendar_time_left" => curves.calendar_time_left = points,
            "due_date_time_left" => curves.due_date_time_left = points,
            "source_priority" => curves.source_priority = points,
            "calendar_event_priority" => curves.calendar_event_priority = points,
            _ => unreachable!("PRIORITY_CURVE_KINDS is the only source of `kind`"),
        }
    }
    Ok(curves)
}

/// Validates and persists one curve's points, replacing whatever was saved
/// for that `curve_kind` before. Validation is authoritative here even
/// though the UI enforces the same invariants while dragging — never trust
/// client-only validation for a `#[server]` fn.
#[cfg(feature = "ssr")]
async fn store_priority_curve(
    pool: &sqlx::SqlitePool,
    username: &str,
    curve_kind: &str,
    points: &[CurvePoint],
) -> Result<(), ServerFnError> {
    if !PRIORITY_CURVE_KINDS.contains(&curve_kind) {
        return Err(ServerFnError::new(format!(
            "unknown curve_kind: {curve_kind}"
        )));
    }
    if points.len() < 2 {
        return Err(ServerFnError::new("a curve needs at least 2 points"));
    }
    let mut seen_x = std::collections::HashSet::new();
    for p in points {
        if !p.x.is_finite() {
            return Err(ServerFnError::new("x must be a finite number"));
        }
        if !p.y.is_finite() || !(0.0..=100.0).contains(&p.y) {
            return Err(ServerFnError::new(
                "y must be a finite number between 0 and 100",
            ));
        }
        if !seen_x.insert(p.x.to_bits()) {
            return Err(ServerFnError::new("duplicate x value"));
        }
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    sqlx::query("DELETE FROM priority_curve_points WHERE username = ? AND curve_kind = ?")
        .bind(username)
        .bind(curve_kind)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    for p in points {
        sqlx::query(
            "INSERT INTO priority_curve_points (username, curve_kind, x, y) VALUES (?, ?, ?, ?)",
        )
        .bind(username)
        .bind(curve_kind)
        .bind(p.x)
        .bind(p.y)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Return the current user's priority curves (defaults for any kind they
/// haven't customized).
#[server]
pub async fn get_priority_curves() -> Result<PriorityCurves, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return fetch_priority_curves(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(PriorityCurves::default())
}

/// Persist one curve (`curve_kind` is one of `"calendar_time_left"`,
/// `"due_date_time_left"`, `"source_priority"`, `"calendar_event_priority"`)
/// for the current user.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn save_priority_curve(
    curve_kind: String,
    points: Vec<CurvePoint>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return store_priority_curve(&pool, &username, &curve_kind, &points).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── SMTP configuration ────────────────────────────────────────────────────────

/// Save SMTP configuration.  Pass an empty `password` to keep the existing one.
#[server(input = server_fn::codec::Json)]
pub async fn save_smtp_config(config: SmtpConfig) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{encrypt_password, ServerSecret};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, _username) = auth_admin().await?;
        let server_secret: String = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        // If no new password, keep the existing encrypted one.
        let password_encrypted = if config.password.is_empty() {
            sqlx::query("SELECT password_encrypted FROM smtp_config WHERE id = 1")
                .fetch_optional(&pool)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
                .and_then(|r| r.try_get::<String, _>("password_encrypted").ok())
                .unwrap_or_default()
        } else {
            encrypt_password(&config.password, &server_secret).map_err(ServerFnError::new)?
        };

        sqlx::query(
            "INSERT INTO smtp_config (id, host, port, tls_mode, username, password_encrypted, from_address, from_name, updated_at) \
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET \
               host=excluded.host, port=excluded.port, tls_mode=excluded.tls_mode, \
               username=excluded.username, password_encrypted=excluded.password_encrypted, \
               from_address=excluded.from_address, from_name=excluded.from_name, updated_at=excluded.updated_at"
        )
        .bind(config.host.clone()).bind(config.port as i64).bind(config.tls_mode.clone())
        .bind(config.username.clone()).bind(password_encrypted)
        .bind(config.from_address.clone()).bind(config.from_name.clone())
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Return current SMTP configuration — password is always empty in the response.
#[server]
pub async fn get_smtp_config() -> Result<Option<SmtpConfig>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, _username) = auth_admin().await?;

        let row = sqlx::query(
            "SELECT host, port, tls_mode, username, from_address, from_name FROM smtp_config WHERE id = 1"
        )
        .fetch_optional(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(row.map(|r| SmtpConfig {
            host: r.try_get("host").unwrap_or_default(),
            port: r.try_get::<i64, _>("port").unwrap_or(587) as u16,
            tls_mode: r
                .try_get("tls_mode")
                .unwrap_or_else(|_| "starttls".to_string()),
            username: r.try_get("username").unwrap_or_default(),
            password: String::new(), // never sent to client
            from_address: r.try_get("from_address").unwrap_or_default(),
            from_name: r.try_get("from_name").unwrap_or_default(),
        }));
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Send a test email to verify SMTP configuration.
/// The actual send is delegated to the server-side `SmtpMailer`.
#[server]
pub async fn test_smtp_send(to_address: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{decrypt_password, ServerSecret};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, _username) = auth_admin().await?;
        let server_secret: String = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        // Read SMTP config and send directly via lettre so we don't need the
        // MailSender extension (which has higher-ranked lifetime issues in server fns).
        let cfg_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
            .fetch_optional(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| ServerFnError::new("SMTP not configured"))?;

        let password = decrypt_password(&cfg_row.password_encrypted, &server_secret)
            .map_err(ServerFnError::new)?;
        let from = cfg_row.header_from();

        use lettre::message::{header::ContentType, SinglePart};
        use lettre::Message;

        let msg = Message::builder()
            .from(
                from.parse().map_err(|e: lettre::address::AddressError| {
                    ServerFnError::new(e.to_string())
                })?,
            )
            .to(to_address
                .parse()
                .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?)
            .subject("tidl test email")
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body("This is a test message from tidl.".to_string()),
            )
            .map_err(|e| ServerFnError::new(e.to_string()))?;

        crate::ssr_types::smtp_send(
            &cfg_row.host,
            cfg_row.port as u16,
            &cfg_row.tls_mode,
            &cfg_row.username,
            &password,
            msg,
        )
        .await
        .map_err(ServerFnError::new)?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── App tokens ────────────────────────────────────────────────────────────────

/// List all app tokens for the authenticated user.
#[server]
pub async fn list_app_tokens() -> Result<Vec<AppTokenInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;
        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT id, name, read_only, created_at, last_used_at
             FROM app_tokens WHERE username = ? ORDER BY created_at DESC",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| {
                let read_only: i64 = r.get("read_only");
                AppTokenInfo {
                    id: r.get("id"),
                    name: r.get("name"),
                    read_only: read_only != 0,
                    created_at: r.get("created_at"),
                    last_used_at: r.try_get("last_used_at").ok().flatten(),
                }
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Create a new app token.  Returns the raw token string (shown once, never stored).
#[server(input = server_fn::codec::Json)]
pub async fn create_app_token(name: String, read_only: bool) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use rand::RngCore;
        use sha2::{Digest, Sha256};
        use sqlx::SqlitePool;
        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;

        // Generate: tidl_ + 32 random bytes as hex (64 chars).
        let mut raw_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw_bytes);
        let token = format!("tidl_{}", hex::encode(raw_bytes));

        // Hash for storage (SHA-256, hex-encoded).
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO app_tokens (id, username, name, token_hash, read_only)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&username)
        .bind(&name)
        .bind(&hash)
        .bind(read_only as i64)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(token);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

// ── Editor integration ───────────────────────────────────────────────────────

/// Open a file at a specific line in the user's configured editor.
///
/// The uid must be in `md:{source_id}:{rel_path}:{line}:{ordinal}` format.  The relative
/// path is resolved against the markdown source's base directory so the editor
/// always receives an absolute path.
///
/// The command template is derived from the browser's chosen `editor_id` (see
/// `crate::local_editor::server_cmd_for`). Tokens `{file}` and `{line}` are
/// substituted with the resolved values. Custom editors have no server
/// command, so this falls back to the OS default file-opener: `xdg-open` on
/// Linux, `open` on macOS, `cmd /c start` on Windows.
///
/// The child process is spawned fire-and-forget; this function returns as soon
/// as the process is launched, without waiting for it to finish.
///
/// This is the **server-local** path, used only for filesystem sources when
/// the browser is on the tidl host. Git sources are always opened from the
/// user's own clone via a browser URI, never from tidl's managed clone.
#[server(input = server_fn::codec::Json)]
pub async fn open_in_editor(
    uid: String,
    editor_id: String,
    server_cmd: Option<String>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;
        use tokio::process::Command;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        // Uid format: "md:{source_id}:{rel_path}:{line}:{ordinal}".
        let parsed = crate::types::MarkdownUid::parse(&uid)
            .ok_or_else(|| ServerFnError::new("unsupported uid format"))?;
        let source_id: i64 = parsed
            .source_id
            .parse()
            .map_err(|_| ServerFnError::new("invalid uid"))?;
        let rel_path = parsed.rel_path;
        let line: u32 = parsed.line_u32();

        // Resolve the relative path to an absolute one using the source's base dir.
        // Git sources live under `<data_dir>/repos/<id>/` and have no `path` in
        // their config, so this must use the shared resolver rather than reading
        // `config.path` directly (backlog #23, #34).
        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("markdown source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        let base_dir = super::files::resolve_markdown_source_dir(
            &source_type,
            &v,
            source_id,
            &data_dir,
        )
        .ok_or_else(|| {
            ServerFnError::new(format!(
                "cannot resolve a directory for source {source_id} (type {source_type}, config {config_json})"
            ))
        })?;

        let abs_path = base_dir.join(rel_path).to_string_lossy().into_owned();

        // The editor choice lives in the browser; its explicit command
        // override wins. For older installs the per-user stored command is
        // still honoured before falling back to the preset's launcher, so
        // upgrading does not silently switch to a binary that may not exist on
        // this host (backlog #164).
        let stored_cmd: Option<String> =
            sqlx::query("SELECT open_in_editor_cmd FROM user_preferences WHERE username = ?")
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .and_then(|r| {
                    r.try_get::<Option<String>, _>("open_in_editor_cmd")
                        .ok()
                        .flatten()
                })
                .filter(|cmd| !cmd.trim().is_empty());

        let cmd_opt = crate::local_editor::resolve_server_cmd(
            &editor_id,
            server_cmd.as_deref(),
            stored_cmd.as_deref(),
        );

        // A remote browser can reach this function too. If the fallback is
        // the only path and the server has no GUI session, fail clearly
        // instead of spawning an opener that cannot display anything.
        if cmd_opt.is_none()
            && cfg!(target_os = "linux")
            && std::env::var_os("DISPLAY").is_none()
            && std::env::var_os("WAYLAND_DISPLAY").is_none()
        {
            return Err(ServerFnError::new(
                "No graphical session on the tidl host — configure a local editor on this device.",
            ));
        }

        let used_fallback = cmd_opt.is_none();
        let mut argv: Vec<String> = match cmd_opt {
            Some(cmd) => {
                let line_str = line.to_string();
                split_command_template(&cmd)
                    .into_iter()
                    .map(|tok| {
                        tok.replace("{file}", &abs_path)
                            .replace("{line}", &line_str)
                    })
                    .collect()
            }
            // Platform-aware fallback: open the file without a specific line.
            _ => default_opener_argv(&abs_path),
        };

        let spawn_err = match Command::new(&argv[0]).args(&argv[1..]).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => e,
        };

        // Launcher names are not portable (e.g. `code` vs `code.cmd`, Flatpak
        // wrappers, GUI-only installs). If the configured/default command is
        // simply absent, fall back to the host's default file opener once
        // rather than surfacing a bare ENOENT (backlog #164).
        if !used_fallback && spawn_err.kind() == std::io::ErrorKind::NotFound {
            argv = default_opener_argv(&abs_path);
            return Command::new(&argv[0])
                .args(&argv[1..])
                .spawn()
                .map(|_| ())
                .map_err(|e| {
                    ServerFnError::new(format!(
                        "Could not open the file on the tidl host with '{}': {e}. Set the exact \
                         command under Settings → Editors → Advanced.",
                        argv[0]
                    ))
                });
        }

        return Err(ServerFnError::new(format!(
            "Could not open the file on the tidl host with '{}': {spawn_err}. Set the exact \
             command under Settings → Editors → Advanced.",
            argv[0]
        )));
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Platform default file-opener argv for a path, used when no editor command
/// is configured or the configured launcher is not installed.
#[cfg(feature = "ssr")]
fn default_opener_argv(abs_path: &str) -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            "cmd".to_string(),
            "/c".to_string(),
            "start".to_string(),
            String::new(),
            abs_path.to_string(),
        ]
    } else if cfg!(target_os = "macos") {
        vec!["open".to_string(), abs_path.to_string()]
    } else {
        vec!["xdg-open".to_string(), abs_path.to_string()]
    }
}

/// Split a command template into argv, honoring double-quoted segments.
///
/// `Command::new` bypasses the shell entirely (deliberately -- see backlog
/// #23, "no shell, no injection risk"), so nothing ever strips quote
/// characters for us. A naive `split_whitespace()` shreds an executable
/// path containing spaces (e.g. Windows' default
/// `C:\Users\Jane Doe\...\Code.exe`) into bogus tokens, and wrapping it in
/// `"..."` makes it worse -- the quote characters end up embedded literally
/// in the tokens instead of being stripped. This honors `"..."` quoting so
/// `"C:\Program Files\Code\Code.exe" --goto {file}:{line}` tokenizes to
/// `["C:\Program Files\Code\Code.exe", "--goto", "{file}:{line}"]`.
#[cfg(feature = "ssr")]
fn split_command_template(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;

    for c in cmd.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

/// Revoke an app token by ID.
#[server(input = server_fn::codec::Json)]
pub async fn revoke_app_token(token_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let affected = sqlx::query("DELETE FROM app_tokens WHERE id = ? AND username = ?")
            .bind(&token_id)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("token not found"));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Inbound email tokens ──────────────────────────────────────────────────────

/// Create a new inbound email token for the given calendar.
///
/// Returns the token info including the full email address — the raw token
/// embedded in the address is shown once and never stored in plaintext.
#[server(input = server_fn::codec::Json)]
pub async fn create_inbound_email_token(
    calendar_id: i64,
    label: String,
) -> Result<InboundEmailTokenInfo, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::InboundDomain;
        use axum::Extension;
        use leptos_axum::extract;
        use rand::RngCore;
        use sha2::{Digest, Sha256};

        let (pool, username) = auth().await?;
        let domain = extract::<Extension<InboundDomain>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        // Generate token: tidl_ + 32 random bytes as hex (64 chars).
        let mut raw_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw_bytes);
        let token = format!("tidl_{}", hex::encode(raw_bytes));

        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        let id = uuid::Uuid::new_v4().to_string();
        let address = format!("{}@{}", token, domain);

        // Look up the calendar name for the response.
        let calendar_name: String = sqlx::query_scalar("SELECT name FROM calendars WHERE id = ?")
            .bind(calendar_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .unwrap_or_default();

        let created_at: String = sqlx::query_scalar(
            "INSERT INTO inbound_email_tokens (id, username, calendar_id, label, token_hash, address) \
             VALUES (?, ?, ?, ?, ?, ?) \
             RETURNING created_at",
        )
        .bind(&id)
        .bind(&username)
        .bind(calendar_id)
        .bind(&label)
        .bind(&token_hash)
        .bind(&address)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(InboundEmailTokenInfo {
            id,
            label,
            calendar_id,
            calendar_name,
            address,
            created_at,
            last_used_at: None,
        });
    }
    #[allow(unreachable_code)]
    Ok(InboundEmailTokenInfo {
        id: String::new(),
        label: String::new(),
        calendar_id: 0,
        calendar_name: String::new(),
        address: String::new(),
        created_at: String::new(),
        last_used_at: None,
    })
}

/// List all inbound email tokens for the authenticated user.
#[server]
pub async fn list_inbound_email_tokens() -> Result<Vec<InboundEmailTokenInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let rows = sqlx::query(
            "SELECT t.id, t.label, t.calendar_id, c.name AS calendar_name, \
                    t.address, t.created_at, t.last_used_at \
             FROM inbound_email_tokens t \
             JOIN calendars c ON c.id = t.calendar_id \
             WHERE t.username = ? \
             ORDER BY t.created_at DESC",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| InboundEmailTokenInfo {
                id: r.get("id"),
                label: r.get("label"),
                calendar_id: r.get("calendar_id"),
                calendar_name: r.get("calendar_name"),
                address: r.get("address"),
                created_at: r.get("created_at"),
                last_used_at: r.try_get("last_used_at").ok().flatten(),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Revoke an inbound email token by ID.
#[server(input = server_fn::codec::Json)]
pub async fn revoke_inbound_email_token(id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let affected =
            sqlx::query("DELETE FROM inbound_email_tokens WHERE id = ? AND username = ?")
                .bind(&id)
                .bind(&username)
                .execute(&pool)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
                .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("inbound email token not found"));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Trusted senders ───────────────────────────────────────────────────────────

/// Add an email address to the trusted sender list.
#[server(input = server_fn::codec::Json)]
pub async fn add_trusted_sender(email: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT OR IGNORE INTO trusted_sender_addresses (id, username, email) \
             VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(&username)
        .bind(&email)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Remove a trusted sender address by ID.
#[server(input = server_fn::codec::Json)]
pub async fn remove_trusted_sender(id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let affected =
            sqlx::query("DELETE FROM trusted_sender_addresses WHERE id = ? AND username = ?")
                .bind(&id)
                .bind(&username)
                .execute(&pool)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
                .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("trusted sender not found"));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// List all trusted sender addresses for the authenticated user.
#[server]
pub async fn list_trusted_senders() -> Result<Vec<TrustedSenderInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let rows = sqlx::query(
            "SELECT id, email, created_at \
             FROM trusted_sender_addresses \
             WHERE username = ? \
             ORDER BY created_at ASC",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| TrustedSenderInfo {
                id: r.get("id"),
                email: r.get("email"),
                created_at: r.get("created_at"),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{fetch_priority_curves, split_command_template, store_priority_curve};
    use crate::types::{CurvePoint, PriorityCurves};

    #[test]
    fn split_command_template_unquoted_simple_command() {
        assert_eq!(
            split_command_template("code --goto {file}:{line}"),
            vec!["code", "--goto", "{file}:{line}"]
        );
    }

    #[test]
    fn split_command_template_quoted_windows_path_with_spaces() {
        assert_eq!(
            split_command_template(r#""C:\Program Files\Code\Code.exe" --goto {file}:{line}"#),
            vec![r"C:\Program Files\Code\Code.exe", "--goto", "{file}:{line}",]
        );
    }

    #[test]
    fn split_command_template_quotes_around_placeholder() {
        // A file path resolved from {file} may itself contain spaces, so
        // quoting the placeholder must work too, not just the executable.
        assert_eq!(
            split_command_template(r#"code --goto "{file}:{line}""#),
            vec!["code", "--goto", "{file}:{line}"]
        );
    }

    #[test]
    fn split_command_template_multiple_quoted_segments() {
        assert_eq!(
            split_command_template(r#""C:\My Editor\ed.exe" "--file={file}" --line={line}"#),
            vec![r"C:\My Editor\ed.exe", "--file={file}", "--line={line}",]
        );
    }

    #[test]
    fn split_command_template_empty_string() {
        assert_eq!(split_command_template(""), Vec::<String>::new());
    }

    #[test]
    fn split_command_template_collapses_extra_whitespace() {
        assert_eq!(
            split_command_template("code   --goto   {file}:{line}"),
            vec!["code", "--goto", "{file}:{line}"]
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curves_default_when_no_rows_exist(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let curves = fetch_priority_curves(&pool, "alice").await.unwrap();
        assert_eq!(curves, PriorityCurves::default());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curve_round_trips_through_store_and_fetch(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let points = vec![
            CurvePoint { x: 0.0, y: 10.0 },
            CurvePoint { x: 5.0, y: 90.0 },
        ];
        store_priority_curve(&pool, "alice", "source_priority", &points)
            .await
            .unwrap();

        let curves = fetch_priority_curves(&pool, "alice").await.unwrap();
        assert_eq!(curves.source_priority, points);
        // The other curves weren't touched — still defaults.
        assert_eq!(
            curves.calendar_time_left,
            PriorityCurves::default().calendar_time_left
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn calendar_event_priority_curve_round_trips(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let points = vec![
            CurvePoint { x: 1.0, y: 100.0 },
            CurvePoint { x: 100.0, y: 40.0 },
        ];
        store_priority_curve(&pool, "alice", "calendar_event_priority", &points)
            .await
            .unwrap();

        let curves = fetch_priority_curves(&pool, "alice").await.unwrap();
        assert_eq!(curves.calendar_event_priority, points);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curve_save_replaces_previous_points(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        store_priority_curve(
            &pool,
            "alice",
            "source_priority",
            &[
                CurvePoint { x: 0.0, y: 10.0 },
                CurvePoint { x: 5.0, y: 90.0 },
            ],
        )
        .await
        .unwrap();
        store_priority_curve(
            &pool,
            "alice",
            "source_priority",
            &[
                CurvePoint { x: 0.0, y: 20.0 },
                CurvePoint { x: 3.0, y: 60.0 },
                CurvePoint { x: 8.0, y: 80.0 },
            ],
        )
        .await
        .unwrap();

        let curves = fetch_priority_curves(&pool, "alice").await.unwrap();
        assert_eq!(curves.source_priority.len(), 3);
        assert_eq!(curves.source_priority[0], CurvePoint { x: 0.0, y: 20.0 });
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curve_rejects_fewer_than_two_points(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let result = store_priority_curve(
            &pool,
            "alice",
            "source_priority",
            &[CurvePoint { x: 0.0, y: 10.0 }],
        )
        .await;
        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curve_rejects_y_out_of_range(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let result = store_priority_curve(
            &pool,
            "alice",
            "source_priority",
            &[
                CurvePoint { x: 0.0, y: -5.0 },
                CurvePoint { x: 5.0, y: 50.0 },
            ],
        )
        .await;
        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn priority_curve_rejects_unknown_curve_kind(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let result = store_priority_curve(
            &pool,
            "alice",
            "not_a_real_kind",
            &[
                CurvePoint { x: 0.0, y: 10.0 },
                CurvePoint { x: 5.0, y: 50.0 },
            ],
        )
        .await;
        assert!(result.is_err());
    }
}

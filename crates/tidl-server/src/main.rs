#![recursion_limit = "512"]

use anyhow::Result;
use axum::{
    routing::{any, get},
    Router,
};
use clap::{Parser, Subcommand};
use std::convert::Infallible;
use std::net::SocketAddr;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "tidl-server",
    about = "tidl CalDAV server",
    version = env!("CARGO_PKG_VERSION"),
    // `--version` reports the revision and build date too (backlog #147);
    // `-V` keeps the bare semantic version.
    long_version = concat!(
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("TIDL_GIT_SHA"),
        ", ",
        env!("TIDL_BUILD_DATE"),
        ")"
    ),
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the CalDAV server (default)
    Serve,
    /// Create the first admin user (bootstrap).
    /// Opens the database, runs migrations, then inserts the new user.
    CreateAdmin {
        #[arg(long)]
        email: String,
        #[arg(long)]
        username: String,
        #[arg(long, default_value = "")]
        display_name: String,
        /// Plaintext password (will be hashed). Mutually exclusive with --password-hash.
        #[arg(
            long,
            conflicts_with = "password_hash",
            required_unless_present = "password_hash"
        )]
        password: Option<String>,
        /// Pre-hashed argon2id password (from `tidl hash-password`). Mutually exclusive with --password.
        #[arg(
            long,
            conflicts_with = "password",
            required_unless_present = "password"
        )]
        password_hash: Option<String>,
    },
    /// Print an argon2id password hash suitable for database import
    HashPassword {
        /// The plaintext password to hash
        password: String,
    },
    /// Generate a random AES-256-GCM key and print it as base64.
    /// Redirect output into a file or paste into sops secrets.yaml.
    GenerateKey,
    /// Generate the deployment secret and write it straight to a file
    /// (the `TIDL_SECRET_FILE` convenience wrapper around `generate-key`).
    GenerateSecret {
        /// File to write the secret to.
        #[arg(long, default_value = "tidl-secret")]
        output: std::path::PathBuf,
        /// Overwrite an existing file. Rotating the secret invalidates all
        /// stored SMTP passwords and CalDAV tokens, so this is opt-in.
        #[arg(long)]
        force: bool,
    },
    /// Seed synthetic demo data (calendar events, contacts, and Notes tasks/
    /// awaits/mentions) for an existing user. Dev tooling for the `gts`
    /// ("generate test server") shell alias — never run against real data.
    SeedDemo {
        #[arg(long)]
        username: String,
    },
}

mod alarm;
mod auth;
mod caldav_client;
mod caldav_handlers;
mod carddav_handlers;
mod config;
mod contact_photo;
mod crypto;
mod done_sweep;
mod export;
mod git_sync;
mod if_header;
mod mailer;
mod markdown_sync;
mod md_image;
mod note_image;
mod notify;
mod oidc;
mod remote_sync;
mod seed_demo;
mod session;
mod session_store;
mod sharing;
mod smtp_listener;
mod ssrf_guard;
mod storage;
mod trace_layer;
mod user_store;
mod webcal_sync;

use config::Config;
use dashmap::DashMap;
use mailer::{Mailer, SmtpMailer};
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    ClientId, ClientSecret, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, RedirectUrl,
};
use session_store::SessionStore;
use std::collections::VecDeque;
use std::sync::Arc;
use storage::{CalendarStore, ContactStore, NoteStore, Storage};
use tidl_caldav::change::ChangeKind;
use tokio::sync::broadcast;
use user_store::UserStore;

/// Holds the initialised OIDC client and display name.
/// `None` in AppState when OIDC is not configured or discovery failed at startup.
#[derive(Clone)]
pub struct OidcState {
    pub client: CoreClient<
        EndpointSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointNotSet,
        EndpointMaybeSet,
        EndpointMaybeSet,
    >,
    pub provider_name: String,
    pub http_client: reqwest::Client,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub storage: Arc<dyn CalendarStore + Send + Sync>,
    pub contacts: Arc<dyn ContactStore + Send + Sync>,
    pub notes: Arc<dyn NoteStore + Send + Sync>,
    pub users: UserStore,
    pub sessions: SessionStore,
    /// Broadcast channel: each message is the username whose data changed.
    /// Connected browsers subscribe and trigger a data refetch on receive.
    pub change_tx: broadcast::Sender<String>,
    pub mailer: Arc<dyn Mailer + Send + Sync>,
    pub inbound_rate_limiter: Arc<DashMap<String, VecDeque<std::time::Instant>>>,
    pub oidc: Option<OidcState>,
    /// 32-byte AES-256-GCM key loaded from TIDL_ENCRYPTION_KEY_FILE at startup.
    /// None if the env var is not set (ad-hoc git sources with PATs will be disabled).
    pub enc_key: Option<Arc<Vec<u8>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::HashPassword { password }) => {
            let hash = UserStore::hash_password(&password)?;
            println!("{}", hash);
            return Ok(());
        }
        Some(Command::GenerateKey) => {
            use base64::Engine as _;
            use rand::RngCore as _;
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            println!("{}", base64::engine::general_purpose::STANDARD.encode(key));
            return Ok(());
        }
        Some(Command::GenerateSecret { output, force }) => {
            use base64::Engine as _;
            use rand::RngCore as _;
            if output.exists() && !force {
                anyhow::bail!(
                    "{} already exists; refusing to overwrite (pass --force to \
                     rotate, which invalidates stored SMTP passwords and CalDAV tokens)",
                    output.display()
                );
            }
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            let encoded = base64::engine::general_purpose::STANDARD.encode(key);
            std::fs::write(&output, format!("{encoded}\n"))?;
            eprintln!("Wrote secret to {}", output.display());
            return Ok(());
        }
        Some(Command::CreateAdmin {
            email,
            username,
            display_name,
            password,
            password_hash,
        }) => {
            return cmd_create_admin(email, username, display_name, password, password_hash).await;
        }
        Some(Command::SeedDemo { username }) => {
            return cmd_seed_demo(username).await;
        }
        _ => {}
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "tidl_server=debug,tidl_web=debug,tidl_caldav=debug,tower_http=debug".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting tidl CalDAV server");

    let config = Config::load("config.toml").unwrap_or_else(|e| {
        tracing::warn!("No config.toml found ({}), using defaults", e);
        let mut cfg = Config::default();
        cfg.apply_env();
        cfg
    });

    let secret = match config.require_secret() {
        Ok(s) => s.to_string(),
        Err(e) => {
            tracing::error!("{e}");
            return Err(e);
        }
    };

    let storage = Storage::new(&config).await?;
    let pool = storage.pool().clone();
    let sessions = SessionStore::new(pool.clone());
    let users = UserStore::new(pool.clone());
    let contacts: Arc<dyn ContactStore + Send + Sync> = Arc::new(storage.clone());
    let notes: Arc<dyn NoteStore + Send + Sync> = Arc::new(storage.clone());
    let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);

    let oidc = if let Some(oidc_cfg) = &config.oidc {
        match IssuerUrl::new(oidc_cfg.issuer_url.clone()) {
            Err(e) => {
                tracing::error!(
                    "Invalid OIDC issuer URL in config, falling back to local auth: {}",
                    e
                );
                None
            }
            Ok(issuer) => {
                let http_client = openidconnect::reqwest::ClientBuilder::new()
                    .build()
                    .expect("failed to build OIDC HTTP client");
                match CoreProviderMetadata::discover_async(issuer, &http_client).await {
                    Ok(metadata) => {
                        match RedirectUrl::new(format!(
                            "https://{}/auth/oidc/callback",
                            config.server.domain
                        )) {
                            Err(e) => {
                                tracing::error!(
                                    "Invalid OIDC redirect URL, falling back to local auth: {}",
                                    e
                                );
                                None
                            }
                            Ok(redirect_url) => {
                                let client = CoreClient::from_provider_metadata(
                                    metadata,
                                    ClientId::new(oidc_cfg.client_id.clone()),
                                    Some(ClientSecret::new(oidc_cfg.client_secret.clone())),
                                )
                                .set_redirect_uri(redirect_url);
                                let provider_name = oidc_cfg
                                    .provider_name
                                    .clone()
                                    .unwrap_or_else(|| "SSO".to_string());
                                tracing::info!("OIDC enabled with provider: {}", provider_name);
                                Some(OidcState {
                                    client,
                                    provider_name,
                                    http_client,
                                })
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("OIDC discovery failed, falling back to local auth: {}", e);
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    let mailer: Arc<dyn Mailer + Send + Sync> = Arc::new(SmtpMailer {
        storage: Arc::clone(&storage),
        server_secret: secret.clone(),
    });

    let mail_sender: tidl_web::ssr_types::MailSender = {
        let m = Arc::clone(&mailer);
        Arc::new(move |to, name, subj, text, html| {
            let m = Arc::clone(&m);
            Box::pin(async move {
                m.send(&to, &name, &subj, &text, &html)
                    .await
                    .map_err(|e| e.to_string())
            })
        })
    };

    let allow_registration =
        tidl_web::ssr_types::AllowRegistration(config.server.allow_registration);
    let admin_email = tidl_web::ssr_types::AdminEmail(config.server.admin_email.clone());
    let server_secret = tidl_web::ssr_types::ServerSecret(secret.clone());
    let base_url = tidl_web::ssr_types::ServerBaseUrl(format!("https://{}", config.server.domain));
    let inbound_domain = tidl_web::ssr_types::InboundDomain(
        config
            .inbound_smtp
            .as_ref()
            .map(|c| c.domain.clone())
            .unwrap_or_else(|| config.server.domain.clone()),
    );
    let session_cookie_name = tidl_web::ssr_types::SessionCookieName::new(
        config.server.port,
        config.server.secure_cookie,
    );

    let addr = SocketAddr::from((
        config.server.host.parse::<std::net::IpAddr>()?,
        config.server.port,
    ));

    // Broadcast channel: each message is a username whose data changed.
    // Capacity 64 — fast writers can burst; slow/offline receivers simply lag.
    let (change_tx, _) = broadcast::channel::<String>(64);

    // Shutdown notification. Fired once on SIGTERM/SIGINT so long-lived SSE
    // streams (`/api/stream`) end promptly; without it axum's graceful
    // shutdown would wait out systemd's stop timeout on an open stream and get
    // SIGKILLed, skipping the WAL checkpoint on the final database close.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Grab clones for the sync loops and SMTP listener before ownership moves into AppState.
    let sync_storage = Arc::clone(&storage);
    let webcal_storage = Arc::clone(&storage);
    let sync_secret = secret.clone();
    let smtp_config = config.inbound_smtp.clone();

    // Load the AES-256-GCM encryption key from file if configured.
    // Used to decrypt PATs stored in the DB for ad-hoc git-backed markdown sources.
    // Config takes priority; TIDL_ENCRYPTION_KEY_FILE env var is the fallback.
    let enc_key_path = config.server.encryption_key_file.clone().or_else(|| {
        std::env::var("TIDL_ENCRYPTION_KEY_FILE")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let enc_key: Option<Arc<Vec<u8>>> = match enc_key_path {
        Some(path) => {
            use base64::Engine as _;
            match tokio::fs::read(&path).await {
                Ok(bytes) => {
                    match base64::engine::general_purpose::STANDARD.decode(bytes.trim_ascii()) {
                        Ok(decoded) if decoded.len() == 32 => {
                            tracing::info!("Encryption key loaded from {path}");
                            Some(Arc::new(decoded))
                        }
                        Ok(decoded) => {
                            tracing::error!(
                                "Encryption key at {path} decoded to {} bytes, expected 32; \
                                 ad-hoc git PAT encryption is disabled",
                                decoded.len()
                            );
                            None
                        }
                        Err(e) => {
                            tracing::error!(
                                "Encryption key at {path} is not valid base64: {e}; \
                                 ad-hoc git PAT encryption is disabled"
                            );
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to read encryption key file at {path}: {e}; \
                         ad-hoc git PAT encryption is disabled"
                    );
                    None
                }
            }
        }
        None => None,
    };

    // Clone for the markdown sync loop and Extension injection before enc_key moves into AppState.
    let md_enc_key = enc_key.clone();
    let enc_key_ext = enc_key.clone();

    // Upsert flake-configured git sources from TIDL_GIT_SOURCES_FILE.
    if let Ok(path) = std::env::var("TIDL_GIT_SOURCES_FILE") {
        if !path.is_empty() {
            match tokio::fs::read_to_string(&path).await {
                Ok(json) => match serde_json::from_str::<Vec<GitSourceEntry>>(&json) {
                    Ok(entries) => {
                        upsert_git_sources(&pool, &entries).await;
                    }
                    Err(e) => tracing::error!("Failed to parse {path}: {e}"),
                },
                Err(e) => tracing::error!("Failed to read TIDL_GIT_SOURCES_FILE at {path}: {e}"),
            }
        }
    }

    // Upsert declarative SMTP config from TIDL_SMTP_* env vars (or config.toml's
    // [smtp] table). Overwrites the smtp_config DB row on every startup, so
    // changes made via Settings → Email are reset on restart when this is set.
    if let Some(smtp) = &config.smtp {
        match mailer::encrypt_password(&smtp.password, &secret) {
            Ok(password_encrypted) => {
                let row = storage::SmtpConfigRow {
                    host: smtp.host.clone(),
                    port: smtp.port as i64,
                    tls_mode: smtp.tls_mode.clone(),
                    username: smtp.username.clone(),
                    password_encrypted,
                    from_address: smtp.from_address.clone(),
                    from_name: smtp.from_name.clone(),
                };
                match storage.save_smtp_config(&row).await {
                    Ok(()) => {
                        tracing::info!("Applied declarative SMTP config for host {}", smtp.host)
                    }
                    Err(e) => tracing::error!("Failed to save declarative SMTP config: {e}"),
                }
            }
            Err(e) => tracing::error!("Failed to encrypt declarative SMTP password: {e}"),
        }
    }

    // Create declaratively configured users from config.toml's [[users]] list.
    // Existing accounts (matched by email or username) are left untouched, so
    // password/admin changes made in the UI survive restarts.
    for u in &config.users {
        let email = u.email.trim();
        let username = u.username.trim();
        if email.is_empty() || username.is_empty() || u.password_hash.trim().is_empty() {
            tracing::error!(
                "Skipping declarative user {username:?}: email, username and password_hash are required"
            );
            continue;
        }
        match users
            .ensure_user(
                username,
                email,
                &u.display_name,
                u.password_hash.trim(),
                u.is_admin,
            )
            .await
        {
            Ok(true) => tracing::info!("Created declarative user '{username}' <{email}>"),
            Ok(false) => {
                tracing::info!("Declarative user '{username}' already exists; leaving unchanged")
            }
            Err(e) => tracing::error!("Failed to create declarative user '{username}': {e}"),
        }
    }

    let app_state = AppState {
        config,
        storage,
        contacts,
        notes,
        users: users.clone(),
        sessions,
        change_tx: change_tx.clone(),
        mailer: mailer.clone(),
        inbound_rate_limiter: Arc::new(DashMap::new()),
        oidc,
        enc_key,
    };

    let oidc_provider_name = tidl_web::ssr_types::OidcProviderName(
        app_state.oidc.as_ref().map(|o| o.provider_name.clone()),
    );

    // Grab clones for the SMTP listener before ownership moves into the routers.
    let smtp_users = users.clone();
    let smtp_change_tx = change_tx.clone();
    let smtp_rate_limiter = Arc::clone(&app_state.inbound_rate_limiter);
    let smtp_mailer = mailer.clone();
    // data_dir for the markdown sync loop — must be cloned before app_state moves.
    let md_data_dir = std::path::PathBuf::from(&app_state.config.paths.data_dir);

    // CalDAV routes — no CORS layer so our OPTIONS handler is not intercepted.
    let caldav_router = Router::new()
        .route(
            "/.well-known/caldav",
            any(caldav_handlers::caldav_discovery),
        )
        .route("/caldav", any(caldav_handlers::caldav_root_any))
        .route("/caldav/", any(caldav_handlers::caldav_root_any))
        // Principal collection and individual principals live under /caldav/principals/.
        // These routes must be declared before /caldav/{username} so that axum's
        // static-segment precedence resolves them correctly.
        .route(
            "/caldav/principals",
            any(caldav_handlers::caldav_principals_collection_any),
        )
        .route(
            "/caldav/principals/",
            any(caldav_handlers::caldav_principals_collection_any),
        )
        .route(
            "/caldav/principals/{username}",
            any(caldav_handlers::caldav_principal_any),
        )
        .route(
            "/caldav/principals/{username}/",
            any(caldav_handlers::caldav_principal_any),
        )
        // Calendar home collection for each user.
        .route("/caldav/{username}", any(caldav_handlers::caldav_home_any))
        .route("/caldav/{username}/", any(caldav_handlers::caldav_home_any))
        // PROPFIND and REPORT are not in axum's standard MethodFilter, so we
        // use `any` and dispatch on the method string inside the handler.
        .route(
            "/caldav/{username}/{calendar}",
            any(caldav_handlers::caldav_collection_any),
        )
        .route(
            "/caldav/{username}/{calendar}/",
            any(caldav_handlers::caldav_collection_any),
        )
        .route(
            "/caldav/{username}/{calendar}/{uid}",
            any(caldav_handlers::caldav_event_any),
        )
        .route(
            "/caldav/{username}/{calendar}/{uid}/",
            any(caldav_handlers::caldav_event_any),
        )
        // Sub-collection event paths: /{username}/{calendar}/{subcoll}/{uid}
        .route(
            "/caldav/{username}/{calendar}/{subcoll}/{uid}",
            any(caldav_handlers::caldav_subcol_event_any),
        )
        .route(
            "/caldav/{username}/{calendar}/{subcoll}/{uid}/",
            any(caldav_handlers::caldav_subcol_event_any),
        )
        // Deeper paths (5+ segments): MKCOL/PUT with missing intermediate → 409.
        .route(
            "/caldav/{username}/{calendar}/{a}/{b}/{*rest}",
            any(caldav_handlers::caldav_deep_any),
        )
        .route("/health", get(health_check))
        .layer(axum::middleware::from_fn(trace_layer::caldav_trace))
        .with_state(app_state.clone());

    // CardDAV routes (backlog #29/#81) — mirrors caldav_router's shape above.
    // Multiple address books per user (backlog #81): {book} resolves to a
    // `book-{collection_id}` alias, uniformly for owned and shared contact
    // collections — see carddav_handlers.rs's module doc for why. Still no
    // MKCOL/COPY/MOVE/LOCK on a collection itself (nothing to create or move
    // — collection management stays web-UI-only).
    let carddav_router = Router::new()
        .route(
            "/.well-known/carddav",
            any(carddav_handlers::carddav_discovery),
        )
        .route("/carddav", any(carddav_handlers::carddav_root_any))
        .route("/carddav/", any(carddav_handlers::carddav_root_any))
        .route(
            "/carddav/principals",
            any(carddav_handlers::carddav_principals_collection_any),
        )
        .route(
            "/carddav/principals/",
            any(carddav_handlers::carddav_principals_collection_any),
        )
        .route(
            "/carddav/principals/{username}",
            any(carddav_handlers::carddav_principal_any),
        )
        .route(
            "/carddav/principals/{username}/",
            any(carddav_handlers::carddav_principal_any),
        )
        .route(
            "/carddav/{username}",
            any(carddav_handlers::carddav_home_any),
        )
        .route(
            "/carddav/{username}/",
            any(carddav_handlers::carddav_home_any),
        )
        .route(
            "/carddav/{username}/{book}",
            any(carddav_handlers::carddav_collection_any),
        )
        .route(
            "/carddav/{username}/{book}/",
            any(carddav_handlers::carddav_collection_any),
        )
        .route(
            "/carddav/{username}/{book}/{uid}",
            any(carddav_handlers::carddav_contact_any),
        )
        .route(
            "/carddav/{username}/{book}/{uid}/",
            any(carddav_handlers::carddav_contact_any),
        )
        // No sub-collections exist under an address book (unlike CalDAV) —
        // anything deeper than a single vCard resource is simply 404, not a
        // 409-worthy "missing intermediate collection" case.
        .route(
            "/carddav/{username}/{book}/{uid}/{*rest}",
            any(carddav_handlers::carddav_deep_any),
        )
        .layer(axum::middleware::from_fn(trace_layer::caldav_trace))
        .with_state(app_state.clone());

    // Web UI — pure CSR: WASM is served from /pkg, server functions from /api/*.
    // All unmatched paths fall back to index.html so client-side routing works
    // on direct navigation (/, /focus, /todos, /tickler, etc.).
    let web_router = Router::new()
        .route("/api/stream", get(sse_handler))
        .route("/debug-log", axum::routing::post(client_debug_log))
        .route(
            "/api/contacts/{id}/photo",
            get(contact_photo::get_contact_photo),
        )
        .route(
            "/api/notes/{note_id}/images/{image_id}",
            get(note_image::get_note_image),
        )
        .route(
            "/api/md-images/{source_id}/{*rel_path}",
            get(md_image::get_md_source_image),
        )
        // Server functions registered by #[server] macros in tidl-web
        .route(
            "/api/{*fn_name}",
            axum::routing::any(leptos_axum::handle_server_fns),
        )
        .route("/auth/oidc/start", get(oidc::start_handler))
        .route("/auth/oidc/callback", get(oidc::callback_handler))
        .route(
            "/auth/oidc/link",
            axum::routing::post(oidc::link_handler)
                .get_service(ServeFile::new("assets/index.html")),
        )
        .route("/share/accept/{token}", get(sharing::accept_handler))
        .route("/share/decline/{token}", get(sharing::decline_handler))
        .route("/export/{slug}", get(export::export_calendar_ics))
        .route("/export/contacts", get(export::export_contacts_vcf))
        .nest_service(
            "/pkg",
            tower::ServiceBuilder::new()
                .layer(axum::middleware::from_fn(cache_pkg_assets))
                .service(
                    ServeDir::new("target/site/pkg")
                        .precompressed_gzip()
                        .precompressed_br(),
                ),
        )
        .nest_service("/assets", ServeDir::new("assets"))
        .fallback_service(ServeFile::new("assets/index.html"))
        .with_state(app_state)
        .layer(CorsLayer::new())
        // Compress responses that don't already have a precompressed sibling.
        // Release builds ship `.br`/`.gz` files for `/pkg` (cargo-leptos
        // `--precompress`) and ServeDir serves those directly; this layer
        // covers `/assets`, dev builds, and every server-fn response, and never
        // double-compresses (it skips responses that already carry
        // `Content-Encoding`). tower-http's default predicate also skips SSE,
        // so `/api/stream` keeps streaming uncompressed.
        .layer(CompressionLayer::new());

    // pool is moved into the Extension layer below — clone it first.
    let sync_pool = pool.clone();
    let smtp_pool = pool.clone();
    let git_pusher_pool = pool.clone();
    let alarm_pool = pool.clone();
    let done_sweep_pool = pool.clone();
    // Retained past the router build so the pool can be closed cleanly once the
    // server has drained (see the `pool.close()` after `axum::serve`).
    let shutdown_pool = pool.clone();

    // Channel for triggering an immediate remote-sync when a new source is added.
    let (sync_trigger_tx, sync_trigger_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let sync_trigger = tidl_web::ssr_types::SyncTrigger(sync_trigger_tx);

    // Channel for triggering an immediate markdown sync when the user clicks "Sync now".
    let (md_trigger_tx, md_trigger_rx) = tokio::sync::mpsc::unbounded_channel::<i64>();
    let md_trigger = tidl_web::ssr_types::MarkdownSyncTrigger(md_trigger_tx);

    // Build the git commit+push closure. Captures the pool, data_dir, and enc_key.
    // Runs commit_and_push via spawn_blocking so git2 (blocking I/O) doesn't stall the runtime.
    let git_pusher_data_dir = md_data_dir.clone();
    // Clone so enc_key_ext is still available for the EncryptionKey Extension below.
    let git_pusher_enc_key = enc_key_ext.clone();
    let git_commit_pusher = tidl_web::ssr_types::GitCommitPusher(std::sync::Arc::new(
        move |source_id: i64, rel_paths: Vec<String>| {
            use sqlx::Row;
            let pool = git_pusher_pool.clone();
            let data_dir = git_pusher_data_dir.clone();
            let enc_key = git_pusher_enc_key.clone();
            Box::pin(async move {
                let row = sqlx::query(
                    "SELECT type, config, pat_encrypted FROM markdown_sources WHERE id = ?",
                )
                .bind(source_id)
                .fetch_optional(&pool)
                .await
                .map_err(|e| format!("DB error: {e}"))?
                .ok_or_else(|| format!("Source {source_id} not found"))?;

                let source_type: String = row.try_get("type").unwrap_or_default();
                if source_type != "git" {
                    // Filesystem sources don't need a git commit.
                    return Ok(());
                }

                let config: String = row.try_get("config").unwrap_or_default();
                let pat_encrypted: Option<String> = row.try_get("pat_encrypted").ok().flatten();
                let v: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();

                let url = v
                    .get("url")
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| "Source has no URL".to_string())?
                    .to_string();
                let branch = v
                    .get("branch")
                    .and_then(|b| b.as_str())
                    .unwrap_or("main")
                    .to_string();
                let pat_file = v
                    .get("pat_file")
                    .and_then(|p| p.as_str())
                    .map(str::to_string);

                let git_cfg = crate::git_sync::GitSourceConfig {
                    url,
                    branch: branch.clone(),
                    pat_file,
                    pat_encrypted,
                    enc_key: enc_key.as_ref().map(|k| k.to_vec()),
                };
                let pat = git_cfg.resolve_pat().map_err(|e| e.to_string())?;

                let clone_path = data_dir.join("repos").join(source_id.to_string());
                tokio::task::spawn_blocking(move || {
                    crate::git_sync::commit_and_push_paths(&clone_path, &rel_paths, &branch, &pat)
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| format!("Git task panicked: {e}"))?
            })
        },
    ));

    // Reads the server clone's uncommitted paths for the "Commit & push
    // server copy" action. Runs git2 status via spawn_blocking like the pusher.
    let git_status_pool = pool.clone();
    let git_status_data_dir = md_data_dir.clone();
    let git_status_reader =
        tidl_web::ssr_types::GitStatusReader(std::sync::Arc::new(move |source_id: i64| {
            use sqlx::Row;
            let pool = git_status_pool.clone();
            let data_dir = git_status_data_dir.clone();
            Box::pin(async move {
                let row = sqlx::query("SELECT type FROM markdown_sources WHERE id = ?")
                    .bind(source_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| format!("DB error: {e}"))?
                    .ok_or_else(|| format!("Source {source_id} not found"))?;
                let source_type: String = row.try_get("type").unwrap_or_default();
                if source_type != "git" {
                    return Ok(Vec::new());
                }
                let clone_path = data_dir.join("repos").join(source_id.to_string());
                tokio::task::spawn_blocking(move || {
                    crate::git_sync::dirty_paths(&clone_path).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| format!("Git task panicked: {e}"))?
            })
        }));

    // Runs detect_default_branch via spawn_blocking (git2 network I/O is blocking).
    let git_branch_detector = tidl_web::ssr_types::GitBranchDetector(std::sync::Arc::new(
        move |url: String, pat: String| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    crate::git_sync::detect_default_branch(&url, &pat).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| format!("Git task panicked: {e}"))?
            })
        },
    ));

    // Manual "refresh now" path for webcal feeds: delegate to the background
    // loop's own `sync_calendar_now` so the two never diverge (backlog #108).
    let webcal_syncer_pool = pool.clone();
    let webcal_syncer_storage = Arc::clone(&webcal_storage);
    let webcal_syncer = tidl_web::ssr_types::WebcalSyncer(std::sync::Arc::new(
        move |username: String, calendar: String| {
            let pool = webcal_syncer_pool.clone();
            let storage = webcal_syncer_storage.clone();
            Box::pin(async move {
                let feed = webcal_sync::load_webcal_feed(&pool, &username, &calendar)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "No webcal feed configured for this calendar.".to_string())?;
                match webcal_sync::sync_calendar_now(&pool, &storage, &feed).await {
                    Ok(changed) => Ok(changed),
                    Err(e) => {
                        // Mirror the background loop: record the failure so the
                        // UI can surface it, then propagate.
                        let _ = storage
                            .update_webcal_sync_status(
                                &username,
                                &calendar,
                                None,
                                None,
                                Some(&e.to_string()),
                            )
                            .await;
                        Err(e.to_string())
                    }
                }
            })
        },
    ));

    // Clone the git pusher + markdown-sync trigger for the done-retention sweep
    // loop (backlog #135), which rewrites markdown sources and wakes the sync
    // loop just like a manual purge. Both are consumed by the Extension layer.
    let done_sweep_git_pusher = git_commit_pusher.clone();
    let done_sweep_md_trigger = md_trigger.clone();
    let done_sweep_data_dir = md_data_dir.clone();

    let app = caldav_router
        .merge(carddav_router)
        .merge(web_router)
        .layer(axum::Extension(pool))
        .layer(axum::Extension(change_tx.clone()))
        .layer(axum::Extension(mail_sender))
        .layer(axum::Extension(server_secret))
        .layer(axum::Extension(base_url))
        .layer(axum::Extension(allow_registration))
        .layer(axum::Extension(admin_email))
        .layer(axum::Extension(sync_trigger))
        .layer(axum::Extension(md_trigger))
        .layer(axum::Extension(inbound_domain))
        .layer(axum::Extension(oidc_provider_name))
        .layer(axum::Extension(session_cookie_name.clone()))
        .layer(axum::Extension(tidl_web::ssr_types::EncryptionKey(
            enc_key_ext,
        )))
        .layer(axum::Extension(tidl_web::ssr_types::DataDir(
            md_data_dir.clone(),
        )))
        .layer(axum::Extension(git_commit_pusher))
        .layer(axum::Extension(git_status_reader))
        .layer(axum::Extension(git_branch_detector))
        .layer(axum::Extension(webcal_syncer))
        .layer(axum::Extension(shutdown_rx))
        .layer(TraceLayer::new_for_http());

    tracing::info!("Listening on http://{}", addr);
    tracing::info!("Web UI:  http://{}", addr);
    // Point CalDAV clients (Thunderbird, etc.) at the bare server address so their
    // auto-discovery (.well-known -> principal -> calendar-home-set) can run; the
    // /caldav path itself is the service root, not a calendar collection, so clients
    // that are given it directly (skipping discovery) will reject it.
    tracing::info!(
        "CalDAV:  http://{} (auto-discovery; service root is /caldav)",
        addr
    );
    // Same reasoning as the CalDAV line above — point clients (Thunderbird,
    // iOS Contacts, DAVx5) at the bare address so .well-known/carddav
    // discovery runs, rather than at /carddav directly.
    tracing::info!(
        "CardDAV: http://{} (auto-discovery; service root is /carddav)",
        addr
    );

    // Spawn the remote CalDAV sync background loop.
    tokio::spawn(remote_sync::run_sync_loop(
        sync_pool.clone(),
        sync_storage,
        sync_secret,
        change_tx.clone(),
        sync_trigger_rx,
    ));

    // Spawn the markdown sources polling loop.
    tokio::spawn(markdown_sync::run_markdown_sync_loop(
        sync_pool.clone(),
        change_tx.clone(),
        md_trigger_rx,
        md_data_dir,
        md_enc_key,
    ));

    // Spawn the done-item retention sweep loop (backlog #135).
    tokio::spawn(done_sweep::run_done_sweep_loop(
        done_sweep_pool,
        done_sweep_data_dir,
        done_sweep_git_pusher,
        done_sweep_md_trigger,
    ));

    // Spawn the webcal subscription sync loop.
    tokio::spawn(webcal_sync::run_webcal_sync_loop(
        sync_pool,
        webcal_storage,
        change_tx.clone(),
    ));

    // Spawn the EMAIL alarm dispatch loop.
    tokio::spawn(alarm::dispatch::run_alarm_dispatch_loop(
        alarm_pool,
        Arc::clone(&mailer),
    ));

    // Spawn the SMTP listener for inbound email (iMIP).
    if let Some(cfg) = smtp_config {
        tokio::spawn(async move {
            smtp_listener::run_smtp_listener(
                cfg,
                smtp_pool,
                smtp_users,
                smtp_change_tx,
                smtp_rate_limiter,
                smtp_mailer,
            )
            .await;
        });
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Wake the SSE streams once a stop signal arrives, so the server can drain
    // instead of waiting out systemd's `TimeoutStopSec` on an open stream.
    let shutdown = async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    };

    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown)
        .await?;

    // The server has drained; close the pool so the last SQLite connection
    // shuts down cleanly. That final close checkpoints the WAL back into
    // tidl.db and removes the -wal file, so `systemctl stop` leaves a
    // self-contained database rather than a main file plus a pending WAL.
    shutdown_pool.close().await;
    tracing::info!("Database connection pool closed; WAL checkpointed");

    Ok(())
}

/// Resolves on SIGTERM (systemd `stop`) or SIGINT (Ctrl-C), after which the
/// server drains in-flight requests and closes the database pool cleanly.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }

    tracing::info!("Shutdown signal received; draining in-flight requests");
}

async fn health_check() -> &'static str {
    "OK"
}

/// Long-cache successful `/pkg/*` responses only. Applying `Cache-Control` to
/// error responses too would let the browser cache a transient 404 (e.g. hit
/// mid-rebuild while `cargo-leptos` swaps the WASM bundle) for the full
/// max-age, breaking WASM loading until the cache entry expires or a hard
/// refresh clears it.
///
/// `Vary: accept-encoding` is added here because `ServeDir`'s precompressed
/// variants set `Content-Encoding` but not `Vary`, and `CompressionLayer` only
/// adds it when *it* compresses (skipped for a precompressed response). Without
/// it, a shared cache could hand a brotli response to a gzip-only client.
async fn cache_pkg_assets(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut res = next.run(req).await;
    if res.status().is_success() {
        let headers = res.headers_mut();
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("public, max-age=3600, must-revalidate"),
        );
        if !headers.contains_key(axum::http::header::VARY) {
            headers.insert(
                axum::http::header::VARY,
                axum::http::HeaderValue::from_static("accept-encoding"),
            );
        }
    }
    res
}

/// SSE endpoint: streams `change` events to the browser whenever the
/// authenticated user's calendar data is mutated (via CalDAV or Web UI).
///
/// Uses only `change_tx` (already an Extension) and `pool` — no full AppState needed.
async fn sse_handler(
    axum::Extension(change_tx): axum::Extension<broadcast::Sender<String>>,
    axum::Extension(pool): axum::Extension<sqlx::SqlitePool>,
    axum::Extension(cookie_name): axum::Extension<tidl_web::ssr_types::SessionCookieName>,
    axum::Extension(mut shutdown_rx): axum::Extension<tokio::sync::watch::Receiver<bool>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse as _;
    use sqlx::Row;
    use tokio_stream::StreamExt as _;

    // Don't open a new stream once shutdown has begun — it would only be
    // torn down again immediately, delaying the drain.
    if *shutdown_rx.borrow() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    // --- Authenticate via session cookie ---
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let Some(token) = cookie_name.extract(cookies) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let row = match sqlx::query(
        "SELECT username FROM sessions WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(&token)
    .fetch_optional(&pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let Some(username) = row.and_then(|r| r.try_get::<String, _>("username").ok()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // --- Subscribe and filter the broadcast channel for this user ---
    // Use a spawned task to forward only this user's change notifications
    // through an mpsc channel, avoiding filter_map async-closure complexity.
    // Forward the ChangeKind prefix ("events", "cals", "all") so the client
    // can increment only the relevant refresh signal.  All senders now use
    // ChangeKind::to_msg() so every message is "<kind>:<username>" format.
    let (fwd_tx, fwd_rx) = tokio::sync::mpsc::channel::<String>(8);
    let colon_user = format!(":{}", username);
    let mut bcast_rx = change_tx.subscribe();
    tokio::spawn(async move {
        loop {
            // Race the change stream against the shutdown signal so the stream
            // closes (and its HTTP connection drains) the moment the server is
            // told to stop, rather than hanging on until systemd's kill timeout.
            let changed = tokio::select! {
                _ = shutdown_rx.changed() => break,
                recv = bcast_rx.recv() => match recv {
                    Ok(changed) => changed,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Missed some messages — trigger a full refresh to be safe.
                        if fwd_tx
                            .send(ChangeKind::All.as_str().to_string())
                            .await
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                },
            };

            if changed == username || changed.ends_with(&colon_user) {
                let (kind, _user) = ChangeKind::parse_msg(&changed);
                if fwd_tx.send(kind.as_str().to_string()).await.is_err() {
                    break; // receiver dropped (client disconnected)
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(fwd_rx)
        .map(|kind| Ok::<_, Infallible>(axum::response::sse::Event::default().data(kind)));

    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text("ping"),
        )
        .into_response()
}

/// Receives client-side debug log entries from the WASM `client-debug` feature.
/// Always compiled; harmless when the client feature is not active.
#[derive(serde::Deserialize)]
struct ClientLogEntry {
    level: String,
    message: String,
    ts: u64,
}

async fn client_debug_log(axum::Json(entry): axum::Json<ClientLogEntry>) -> axum::http::StatusCode {
    tracing::info!(
        "[CLIENT] level={} ts={} message={}",
        entry.level,
        entry.ts,
        entry.message,
    );
    axum::http::StatusCode::NO_CONTENT
}

/// Entry from TIDL_GIT_SOURCES_FILE — one per git-backed markdown source.
#[derive(serde::Deserialize)]
struct GitSourceEntry {
    username: String,
    name: String,
    url: String,
    branch: String,
    pat_file: String,
}

/// Upsert git-backed markdown sources from a JSON config file into the DB.
/// Idempotent: keyed on (username, url) with config_managed = 1.
async fn upsert_git_sources(pool: &sqlx::SqlitePool, entries: &[GitSourceEntry]) {
    for entry in entries {
        let config = serde_json::json!({
            "url": entry.url,
            "branch": entry.branch,
            "pat_file": entry.pat_file,
        })
        .to_string();

        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM markdown_sources
             WHERE username = ? AND type = 'git' AND config_managed = 1
               AND json_extract(config, '$.url') = ?",
        )
        .bind(&entry.username)
        .bind(&entry.url)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some(id) = existing {
            let _ = sqlx::query("UPDATE markdown_sources SET name = ?, config = ? WHERE id = ?")
                .bind(&entry.name)
                .bind(&config)
                .bind(id)
                .execute(pool)
                .await
                .map_err(|e| tracing::error!("upsert_git_sources update failed: {e}"));
        } else {
            let _ = sqlx::query(
                "INSERT INTO markdown_sources
                 (username, name, type, config, enabled, config_managed, sync_interval_secs)
                 VALUES (?, ?, 'git', ?, 1, 1, 300)",
            )
            .bind(&entry.username)
            .bind(&entry.name)
            .bind(&config)
            .execute(pool)
            .await
            .map_err(|e| tracing::error!("upsert_git_sources insert failed: {e}"));
        }
    }
}

/// Bootstrap: open the DB, run migrations, insert the first admin user.
async fn cmd_create_admin(
    email: String,
    username: String,
    display_name: String,
    password: Option<String>,
    password_hash: Option<String>,
) -> Result<()> {
    let config = Config::load("config.toml").unwrap_or_else(|_| {
        let mut cfg = Config::default();
        cfg.apply_env();
        cfg
    });
    let storage = Storage::new(&config).await?;
    let users = UserStore::new(storage.pool().clone());

    let hash = match password_hash {
        Some(h) => h,
        None => UserStore::hash_password(&password.unwrap())?,
    };
    let user = users
        .create_user(&username, &email, &display_name, &hash, true)
        .await?;

    println!("Created admin user '{}' <{}>", user.username, user.email);
    Ok(())
}

/// Seed synthetic demo data for an existing user (see `seed_demo`'s module
/// doc comment). Dev tooling for the `gts` shell alias — assumes the user
/// (and its personal calendar/database) already exist.
async fn cmd_seed_demo(username: String) -> Result<()> {
    let config = Config::load("config.toml").unwrap_or_else(|_| {
        let mut cfg = Config::default();
        cfg.apply_env();
        cfg
    });
    let storage = Storage::new(&config).await?;
    let pool = storage.pool().clone();
    let contacts: Arc<dyn ContactStore + Send + Sync> = Arc::new(storage.clone());
    let calendars: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);

    seed_demo::seed(&pool, &calendars, &contacts, &username).await?;

    println!("Seeded synthetic demo data for '{}'", username);
    Ok(())
}

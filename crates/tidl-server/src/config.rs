use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Display name for the "Sign in with …" button. Defaults to "SSO" if absent.
    #[serde(default)]
    pub provider_name: Option<String>,
}

/// Declarative SMTP configuration, sourced from `TIDL_SMTP_*` env vars.
/// Applied to the `smtp_config` DB row on startup (see `main.rs`) so the
/// existing DB-backed `SmtpMailer` picks it up without further plumbing.
/// Values saved this way are still editable afterwards via Settings → Email;
/// on the next restart the env vars simply overwrite them again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpEnvConfig {
    pub host: String,
    pub port: u16,
    /// "none" | "starttls" | "tls"
    pub tls_mode: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub from_address: String,
    #[serde(default)]
    pub from_name: String,
}

/// Server configuration loaded from `config.toml`.
///
/// User accounts are stored in the `users` SQLite table (see
/// `migrations/20240107000000_users.sql`).  Use `tidl-server create-admin` to
/// create the first admin account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub paths: PathsConfig,
    pub caldav: CalDAVConfig,
    pub sync: SyncConfig,
    #[serde(default)]
    pub inbound_smtp: Option<InboundSmtpConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub smtp: Option<SmtpEnvConfig>,
    /// Users created at startup (config.toml `[[users]]`). Accounts are
    /// matched by email or username; existing ones are left untouched.
    #[serde(default)]
    pub users: Vec<DeclarativeUser>,
}

/// A user provisioned declaratively from `[[users]]` in `config.toml`.
///
/// The account is created only when no user with the same email or username
/// exists; an existing account is never modified, so password and admin-flag
/// changes made in the web UI survive restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeUser {
    /// Login identifier (case-insensitive).
    pub email: String,
    /// Short slug used in CalDAV/CardDAV URLs.
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    /// Argon2id PHC string from `tidl-server hash-password <password>`
    /// (legacy bcrypt `$2b$`/`$2y$` hashes are also accepted).
    pub password_hash: String,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub domain: String,
    /// Secret used to derive the AES key for encrypting stored credentials
    /// (SMTP password, remote CalDAV personal access tokens). No default —
    /// `Config::require_secret` refuses to start the server when this is
    /// unset, rather than silently encrypting those credentials with a
    /// guessable key. Set via `TIDL_SECRET` or this field; generate one with
    /// `tidl-server generate-key`.
    #[serde(default)]
    pub secret: Option<String>,
    /// Allow new users to self-register via the web UI.
    /// Defaults to `true` for self-hosted deployments.
    /// When `false`, the sign-up form is hidden and `POST /auth/signup` returns 403;
    /// invitation-driven sign-up still works (the token serves as authorisation).
    #[serde(default = "default_allow_registration")]
    pub allow_registration: bool,
    /// Optional email address to notify when a new account is created.
    /// Leave unset to disable admin notifications.
    #[serde(default)]
    pub admin_email: Option<String>,
    /// Path to a file containing exactly 32 random bytes used as the AES-256-GCM key
    /// for encrypting ad-hoc git source PATs. If unset, falls back to TIDL_ENCRYPTION_KEY_FILE.
    #[serde(default)]
    pub encryption_key_file: Option<String>,
    /// Whether the session cookie gets the `Secure` attribute (sent only over
    /// HTTPS). Defaults to `true`, matching the documented deployment (behind
    /// a TLS-terminating reverse proxy). Set `false` only for a deployment
    /// deliberately served over plain HTTP (e.g. direct LAN access with no
    /// TLS in front) — otherwise the browser silently drops the cookie.
    #[serde(default = "default_secure_cookie")]
    pub secure_cookie: bool,
}

fn default_allow_registration() -> bool {
    true
}

fn default_secure_cookie() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    pub data_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalDAVConfig {
    pub default_calendar: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub watch_interval_secs: u64,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub r#type: String,
    pub name: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundSmtpConfig {
    /// TCP port the SMTP listener binds to (default 2525)
    pub port: u16,
    /// Path to TLS certificate PEM file
    pub cert_path: String,
    /// Path to TLS private key PEM file
    pub key_path: String,
    /// SMTP greeting domain, e.g. "calendar.example.com"
    pub domain: String,
    /// Maximum accepted message size in bytes (default 1 MB)
    pub max_message_bytes: usize,
    /// Maximum messages per token per rolling hour (default 20)
    pub rate_limit_per_hour: u32,
}

impl Default for InboundSmtpConfig {
    fn default() -> Self {
        Self {
            port: 2525,
            cert_path: String::new(),
            key_path: String::new(),
            domain: String::new(),
            max_message_bytes: 1_048_576,
            rate_limit_per_hour: 20,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3060,
                domain: "tidl.dev".to_string(),
                secret: None,
                allow_registration: default_allow_registration(),
                admin_email: None,
                encryption_key_file: None,
                secure_cookie: default_secure_cookie(),
            },
            paths: PathsConfig {
                data_dir: "./data".to_string(),
            },
            caldav: CalDAVConfig {
                default_calendar: "personal".to_string(),
            },
            sync: SyncConfig {
                enabled: false,
                watch_interval_secs: 5,
                sources: vec![],
            },
            inbound_smtp: None,
            oidc: None,
            smtp: None,
            users: vec![],
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let contents = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&contents)?;
        config.apply_env();
        Ok(config)
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        fs::write(path, contents)?;
        Ok(())
    }

    /// Override config fields from environment variables.
    ///
    /// | Variable                       | Field                        |
    /// |--------------------------------|------------------------------|
    /// | `TIDL_HOST`                    | `server.host`                |
    /// | `TIDL_PORT`                    | `server.port`                |
    /// | `TIDL_DOMAIN`                  | `server.domain`              |
    /// | `TIDL_DATA_DIR`                | `paths.data_dir`             |
    /// | `TIDL_SECRET`                  | `server.secret`              |
    /// | `TIDL_SECRET_FILE`             | `server.secret` (file)       |
    /// | `TIDL_OIDC_ISSUER_URL`         | `oidc.issuer_url`            |
    /// | `TIDL_OIDC_CLIENT_ID`          | `oidc.client_id`             |
    /// | `TIDL_OIDC_CLIENT_SECRET`      | `oidc.client_secret`         |
    /// | `TIDL_OIDC_CLIENT_SECRET_FILE` | `oidc.client_secret` (file)  |
    /// | `TIDL_OIDC_PROVIDER_NAME`      | `oidc.provider_name`         |
    /// | `TIDL_ADMIN_EMAIL`             | `server.admin_email`         |
    /// | `TIDL_ALLOW_REGISTRATION`      | `server.allow_registration`  |
    /// | `TIDL_SECURE_COOKIE`           | `server.secure_cookie`       |
    /// | `TIDL_SMTP_HOST`               | `smtp.host`                  |
    /// | `TIDL_SMTP_PORT`               | `smtp.port` (default 587)    |
    /// | `TIDL_SMTP_TLS_MODE`           | `smtp.tls_mode` (default "starttls") |
    /// | `TIDL_SMTP_USERNAME`           | `smtp.username`              |
    /// | `TIDL_SMTP_PASSWORD`           | `smtp.password`              |
    /// | `TIDL_SMTP_PASSWORD_FILE`      | `smtp.password` (file)       |
    /// | `TIDL_SMTP_FROM_ADDRESS`       | `smtp.from_address`          |
    /// | `TIDL_SMTP_FROM_NAME`          | `smtp.from_name`             |
    ///
    /// `TIDL_OIDC_CLIENT_SECRET_FILE` takes precedence over `TIDL_OIDC_CLIENT_SECRET`.
    /// OIDC is only activated when issuer URL, client ID, and client secret are all present.
    /// `TIDL_SECRET_FILE` takes precedence over `TIDL_SECRET`.
    /// `TIDL_SMTP_PASSWORD_FILE` takes precedence over `TIDL_SMTP_PASSWORD`.
    /// SMTP is only activated when host and from-address are both present; this
    /// config is upserted into the `smtp_config` DB row on startup (see `main.rs`),
    /// the same row edited by Settings → Email — env vars simply overwrite it on
    /// every restart.
    /// `TIDL_ALLOW_REGISTRATION` and `TIDL_SECURE_COOKIE` accept `true`/`1` or `false`/`0`.
    /// Variables that are absent or empty are silently ignored.
    ///
    /// `cargo leptos watch` sets `LEPTOS_SITE_ADDR` (from `site-addr` in
    /// `Cargo.toml`) on the server process; it is honoured below as a fallback
    /// for `server.host`/`server.port` so the dev server binds where
    /// cargo-leptos expects. `TIDL_HOST`/`TIDL_PORT` take precedence over it.
    pub fn apply_env(&mut self) {
        use std::env;
        if let Ok(v) = env::var("LEPTOS_SITE_ADDR") {
            let v = v.trim();
            if !v.is_empty() {
                if let Some((host, port)) = v.rsplit_once(':') {
                    let host = host.trim_start_matches('[').trim_end_matches(']');
                    if !host.is_empty() {
                        self.server.host = host.to_string();
                    }
                    if let Ok(p) = port.parse::<u16>() {
                        self.server.port = p;
                    }
                }
            }
        }
        if let Ok(v) = env::var("TIDL_HOST") {
            if !v.is_empty() {
                self.server.host = v;
            }
        }
        if let Ok(v) = env::var("TIDL_PORT") {
            if let Ok(p) = v.parse() {
                self.server.port = p;
            }
        }
        if let Ok(v) = env::var("TIDL_DOMAIN") {
            if !v.is_empty() {
                self.server.domain = v;
            }
        }
        if let Ok(v) = env::var("TIDL_DATA_DIR") {
            if !v.is_empty() {
                self.paths.data_dir = v;
            }
        }
        let secret = env::var("TIDL_SECRET_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|f| fs::read_to_string(&f).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| env::var("TIDL_SECRET").ok().filter(|s| !s.is_empty()));
        if let Some(s) = secret {
            self.server.secret = Some(s);
        }
        if let Ok(v) = env::var("TIDL_ADMIN_EMAIL") {
            if !v.is_empty() {
                self.server.admin_email = Some(v);
            }
        }
        if let Ok(v) = env::var("TIDL_ALLOW_REGISTRATION") {
            match v.as_str() {
                "true" | "1" => self.server.allow_registration = true,
                "false" | "0" => self.server.allow_registration = false,
                _ => {}
            }
        }
        if let Ok(v) = env::var("TIDL_SECURE_COOKIE") {
            match v.as_str() {
                "true" | "1" => self.server.secure_cookie = true,
                "false" | "0" => self.server.secure_cookie = false,
                _ => {}
            }
        }

        let issuer = env::var("TIDL_OIDC_ISSUER_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let client_id = env::var("TIDL_OIDC_CLIENT_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let secret = env::var("TIDL_OIDC_CLIENT_SECRET_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|f| fs::read_to_string(&f).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                env::var("TIDL_OIDC_CLIENT_SECRET")
                    .ok()
                    .filter(|s| !s.is_empty())
            });
        let provider_name = env::var("TIDL_OIDC_PROVIDER_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        if let (Some(issuer_url), Some(client_id), Some(client_secret)) =
            (issuer, client_id, secret)
        {
            self.oidc = Some(OidcConfig {
                issuer_url,
                client_id,
                client_secret,
                provider_name,
            });
        }

        let smtp_host = env::var("TIDL_SMTP_HOST").ok().filter(|s| !s.is_empty());
        let smtp_from_address = env::var("TIDL_SMTP_FROM_ADDRESS")
            .ok()
            .filter(|s| !s.is_empty());
        let smtp_port = env::var("TIDL_SMTP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(587);
        let smtp_tls_mode = env::var("TIDL_SMTP_TLS_MODE")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "starttls".to_string());
        let smtp_username = env::var("TIDL_SMTP_USERNAME").ok().unwrap_or_default();
        let smtp_password = env::var("TIDL_SMTP_PASSWORD_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|f| fs::read_to_string(&f).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                env::var("TIDL_SMTP_PASSWORD")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        let smtp_from_name = env::var("TIDL_SMTP_FROM_NAME").ok().unwrap_or_default();

        if let (Some(host), Some(from_address)) = (smtp_host, smtp_from_address) {
            self.smtp = Some(SmtpEnvConfig {
                host,
                port: smtp_port,
                tls_mode: smtp_tls_mode,
                username: smtp_username,
                password: smtp_password,
                from_address,
                from_name: smtp_from_name,
            });
        }
    }

    /// Returns the configured secret, or an error naming how to set one.
    ///
    /// Unlike `encryption_key_file` (whose absence just disables one
    /// feature), there is no safe default here: this key protects SMTP
    /// passwords and remote CalDAV tokens that are already sitting
    /// encrypted in the database, so a guessable fallback would silently
    /// defeat that encryption rather than visibly disable anything. Callers
    /// on the server-start path must treat an error from this as fatal.
    pub fn require_secret(&self) -> Result<&str> {
        match self.server.secret.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => Ok(s),
            _ => Err(anyhow::anyhow!(
                "server.secret is not set. This key protects stored SMTP passwords \
                 and remote CalDAV personal access tokens at rest, so there is no \
                 default. Generate one with `tidl-server generate-key`, then set it \
                 via the TIDL_SECRET environment variable or `secret` under [server] \
                 in config.toml, and restart."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var tests must not run concurrently — use this lock to serialise them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_values() {
        let c = Config::default();
        assert_eq!(c.server.host, "127.0.0.1");
        assert_eq!(c.server.port, 3060);
        assert_eq!(c.server.domain, "tidl.dev");
        assert!(c.server.allow_registration);
        assert!(c.server.secret.is_none());
        assert!(c.server.secure_cookie);
        assert_eq!(c.caldav.default_calendar, "personal");
        assert!(!c.sync.enabled);
        assert_eq!(c.sync.watch_interval_secs, 5);
    }

    #[test]
    fn apply_env_can_disable_secure_cookie() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_SECURE_COOKIE", "false");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SECURE_COOKIE");
        }
        assert!(!c.server.secure_cookie);
    }

    #[test]
    fn apply_env_overrides_fields() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_HOST", "0.0.0.0");
            std::env::set_var("TIDL_PORT", "8080");
            std::env::set_var("TIDL_DOMAIN", "example.com");
            std::env::set_var("TIDL_DATA_DIR", "/tmp/tidl-data");
            std::env::set_var("TIDL_SECRET", "supersecret");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_HOST");
            std::env::remove_var("TIDL_PORT");
            std::env::remove_var("TIDL_DOMAIN");
            std::env::remove_var("TIDL_DATA_DIR");
            std::env::remove_var("TIDL_SECRET");
        }

        assert_eq!(c.server.host, "0.0.0.0");
        assert_eq!(c.server.port, 8080);
        assert_eq!(c.server.domain, "example.com");
        assert_eq!(c.paths.data_dir, "/tmp/tidl-data");
        assert_eq!(c.server.secret, Some("supersecret".to_string()));
    }

    #[test]
    fn apply_env_empty_string_ignored() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_HOST", "");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_HOST");
        }
        assert_eq!(c.server.host, "127.0.0.1");
    }

    #[test]
    fn leptos_site_addr_used_as_fallback() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("LEPTOS_SITE_ADDR", "0.0.0.0:3060");
            std::env::remove_var("TIDL_HOST");
            std::env::remove_var("TIDL_PORT");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("LEPTOS_SITE_ADDR");
        }
        assert_eq!(c.server.host, "0.0.0.0");
        assert_eq!(c.server.port, 3060);
    }

    #[test]
    fn tidl_host_overrides_leptos_site_addr() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("LEPTOS_SITE_ADDR", "0.0.0.0:3060");
            std::env::set_var("TIDL_HOST", "127.0.0.1");
            std::env::set_var("TIDL_PORT", "8080");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("LEPTOS_SITE_ADDR");
            std::env::remove_var("TIDL_HOST");
            std::env::remove_var("TIDL_PORT");
        }
        assert_eq!(c.server.host, "127.0.0.1");
        assert_eq!(c.server.port, 8080);
    }

    #[test]
    fn oidc_config_parsed() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 3060
domain = "tidl.dev"
log_level = "info"

[paths]
data_dir = "./data"

[caldav]
default_calendar = "personal"

[sync]
enabled = false
watch_interval_secs = 5

[oidc]
issuer_url = "https://kanidm.example.com/oauth2/openid/tidl"
client_id = "tidl"
client_secret = "s3cr3t"
provider_name = "Kanidm"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let oidc = config.oidc.expect("oidc config should be present");
        assert_eq!(
            oidc.issuer_url,
            "https://kanidm.example.com/oauth2/openid/tidl"
        );
        assert_eq!(oidc.client_id, "tidl");
        assert_eq!(oidc.client_secret, "s3cr3t");
        assert_eq!(oidc.provider_name, Some("Kanidm".to_string()));
    }

    #[test]
    fn oidc_config_absent_defaults_to_none() {
        let c = Config::default();
        assert!(c.oidc.is_none());
    }

    #[test]
    fn smtp_env_config_absent_defaults_to_none() {
        let c = Config::default();
        assert!(c.smtp.is_none());
    }

    #[test]
    fn smtp_env_config_parsed_with_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_SMTP_HOST", "smtp.example.com");
            std::env::set_var("TIDL_SMTP_FROM_ADDRESS", "tidl@example.com");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SMTP_HOST");
            std::env::remove_var("TIDL_SMTP_FROM_ADDRESS");
        }

        let smtp = c.smtp.expect("smtp config should be present");
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.from_address, "tidl@example.com");
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.tls_mode, "starttls");
        assert_eq!(smtp.username, "");
        assert_eq!(smtp.password, "");
        assert_eq!(smtp.from_name, "");
    }

    #[test]
    fn smtp_env_config_parsed_fully() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_SMTP_HOST", "smtp.example.com");
            std::env::set_var("TIDL_SMTP_PORT", "465");
            std::env::set_var("TIDL_SMTP_TLS_MODE", "tls");
            std::env::set_var("TIDL_SMTP_USERNAME", "tidl-bot");
            std::env::set_var("TIDL_SMTP_PASSWORD", "hunter2");
            std::env::set_var("TIDL_SMTP_FROM_ADDRESS", "tidl@example.com");
            std::env::set_var("TIDL_SMTP_FROM_NAME", "tidl");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SMTP_HOST");
            std::env::remove_var("TIDL_SMTP_PORT");
            std::env::remove_var("TIDL_SMTP_TLS_MODE");
            std::env::remove_var("TIDL_SMTP_USERNAME");
            std::env::remove_var("TIDL_SMTP_PASSWORD");
            std::env::remove_var("TIDL_SMTP_FROM_ADDRESS");
            std::env::remove_var("TIDL_SMTP_FROM_NAME");
        }

        let smtp = c.smtp.expect("smtp config should be present");
        assert_eq!(smtp.port, 465);
        assert_eq!(smtp.tls_mode, "tls");
        assert_eq!(smtp.username, "tidl-bot");
        assert_eq!(smtp.password, "hunter2");
        assert_eq!(smtp.from_name, "tidl");
    }

    #[test]
    fn smtp_env_config_password_file_takes_precedence() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        let file = std::env::temp_dir().join("tidl-smtp-password-test.txt");
        std::fs::write(&file, "from-file-secret\n").unwrap();
        unsafe {
            std::env::set_var("TIDL_SMTP_HOST", "smtp.example.com");
            std::env::set_var("TIDL_SMTP_FROM_ADDRESS", "tidl@example.com");
            std::env::set_var("TIDL_SMTP_PASSWORD", "from-env");
            std::env::set_var("TIDL_SMTP_PASSWORD_FILE", file.to_str().unwrap());
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SMTP_HOST");
            std::env::remove_var("TIDL_SMTP_FROM_ADDRESS");
            std::env::remove_var("TIDL_SMTP_PASSWORD");
            std::env::remove_var("TIDL_SMTP_PASSWORD_FILE");
        }
        std::fs::remove_file(&file).unwrap();

        assert_eq!(c.smtp.unwrap().password, "from-file-secret");
    }

    #[test]
    fn secret_file_takes_precedence_over_secret_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        let file = std::env::temp_dir().join("tidl-secret-file-test.txt");
        std::fs::write(&file, "from-file-secret\n").unwrap();
        unsafe {
            std::env::set_var("TIDL_SECRET", "from-env");
            std::env::set_var("TIDL_SECRET_FILE", file.to_str().unwrap());
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SECRET");
            std::env::remove_var("TIDL_SECRET_FILE");
        }
        std::fs::remove_file(&file).unwrap();

        assert_eq!(c.server.secret, Some("from-file-secret".to_string()));
    }

    #[test]
    fn require_secret_errors_when_unset() {
        let c = Config::default();
        assert!(c.require_secret().is_err());
    }

    #[test]
    fn require_secret_errors_when_blank() {
        let mut c = Config::default();
        c.server.secret = Some("   ".to_string());
        assert!(c.require_secret().is_err());
    }

    #[test]
    fn require_secret_returns_trimmed_value_when_set() {
        let mut c = Config::default();
        c.server.secret = Some("  my-secret  ".to_string());
        assert_eq!(c.require_secret().unwrap(), "my-secret");
    }

    #[test]
    fn smtp_env_config_requires_host_and_from_address() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut c = Config::default();
        unsafe {
            std::env::set_var("TIDL_SMTP_HOST", "smtp.example.com");
        }
        c.apply_env();
        unsafe {
            std::env::remove_var("TIDL_SMTP_HOST");
        }
        assert!(c.smtp.is_none());
    }

    #[test]
    fn declarative_users_default_empty() {
        let c = Config::default();
        assert!(c.users.is_empty());
    }

    #[test]
    fn declarative_users_parsed_with_defaults() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 3060
domain = "tidl.dev"

[paths]
data_dir = "./data"

[caldav]
default_calendar = "personal"

[sync]
enabled = false
watch_interval_secs = 5

[[users]]
email = "admin@example.com"
username = "admin"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$abc$def"

[[users]]
email = "alice@example.com"
username = "alice"
display_name = "Alice"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$abc$def"
is_admin = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.users.len(), 2);

        let admin = &config.users[0];
        assert_eq!(admin.email, "admin@example.com");
        assert_eq!(admin.username, "admin");
        assert_eq!(admin.display_name, "");
        assert!(!admin.is_admin);

        let alice = &config.users[1];
        assert_eq!(alice.display_name, "Alice");
        assert!(alice.is_admin);
    }
}

//! Email sending abstraction.
//!
//! [`Mailer`] is the trait; [`SmtpMailer`] is the production implementation that
//! reads its configuration from the database on each send (so SMTP settings
//! saved via the UI take effect immediately without a server restart).
//! [`StubMailer`] logs the would-be message and is useful in tests.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use lettre::{
    message::{header::ContentType, MultiPart, SinglePart},
    Message,
};
use std::sync::Arc;

use crate::storage::CalendarStore;

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(
        &self,
        to_address: &str,
        to_name: &str,
        subject: &str,
        body_text: &str,
        body_html: &str,
    ) -> Result<()>;
}

// ── SMTP implementation ───────────────────────────────────────────────────────

/// Reads SMTP config from storage on every send so changes take effect
/// without restarting the server.
pub struct SmtpMailer {
    pub storage: Arc<dyn CalendarStore + Send + Sync>,
    pub server_secret: String,
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(
        &self,
        to_address: &str,
        to_name: &str,
        subject: &str,
        body_text: &str,
        body_html: &str,
    ) -> Result<()> {
        let cfg = self.storage.get_smtp_config().await?.ok_or_else(|| {
            anyhow!("SMTP is not configured — go to Settings → Email to set it up")
        })?;

        let password = if cfg.password_encrypted.is_empty() {
            String::new()
        } else {
            decrypt_password(&cfg.password_encrypted, &self.server_secret)
                .context("failed to decrypt SMTP password")?
        };

        let from = if cfg.from_name.is_empty() {
            cfg.from_address.clone()
        } else {
            format!("{} <{}>", cfg.from_name, cfg.from_address)
        };
        let to = if to_name.is_empty() {
            to_address.to_string()
        } else {
            format!("{} <{}>", to_name, to_address)
        };

        let email = Message::builder()
            .from(from.parse().context("invalid From address")?)
            .to(to.parse().context("invalid To address")?)
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(body_text.to_string()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(body_html.to_string()),
                    ),
            )?;

        tidl_web::ssr_types::smtp_send(
            &cfg.host,
            cfg.port as u16,
            &cfg.tls_mode,
            &cfg.username,
            &password,
            email,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        Ok(())
    }
}

// ── Stub (for tests / no-config mode) ────────────────────────────────────────

#[allow(dead_code)]
pub struct StubMailer;

#[async_trait]
impl Mailer for StubMailer {
    async fn send(
        &self,
        to_address: &str,
        _to_name: &str,
        subject: &str,
        body_text: &str,
        _body_html: &str,
    ) -> Result<()> {
        tracing::info!(
            to = to_address,
            subject,
            body = body_text,
            "StubMailer: email not sent (no SMTP configured)"
        );
        Ok(())
    }
}

// ── Password encryption helpers ───────────────────────────────────────────────

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha2::{Digest, Sha256};

fn derive_key(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b"tidl-smtp-v1");
    hasher.finalize().into()
}

/// Encrypt a plaintext password for storage in `smtp_config.password_encrypted`.
/// Mirrors `ssr_types::encrypt_password` (used by the Settings → Email UI) —
/// this copy exists so declarative env-var SMTP config (`config.rs`, applied
/// in `main.rs`) can be upserted without pulling the `tidl-web` ssr feature
/// into the startup path.
pub fn encrypt_password(plaintext: &str, secret: &str) -> Result<String> {
    use aes_gcm::aead::{rand_core::RngCore, OsRng};

    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new(key.as_ref().into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("encrypt failed: {e}"))?;
    Ok(format!(
        "{}:{}",
        BASE64.encode(nonce_bytes),
        BASE64.encode(ciphertext)
    ))
}

/// Decrypt a value produced by `encrypt_password` (see `ssr_types::encrypt_password`).
pub fn decrypt_password(encrypted: &str, secret: &str) -> Result<String> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }
    let (nonce_b64, ct_b64) = encrypted
        .split_once(':')
        .ok_or_else(|| anyhow!("malformed encrypted password"))?;
    let nonce_bytes = BASE64.decode(nonce_b64).context("bad nonce base64")?;
    let ct = BASE64.decode(ct_b64).context("bad ciphertext base64")?;
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new(key.as_ref().into());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|e| anyhow!("decrypt failed: {e}"))?;
    String::from_utf8(plaintext).context("decrypted password is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let encrypted = encrypt_password("hunter2", "server-secret").unwrap();
        assert_ne!(encrypted, "hunter2");
        assert_eq!(
            decrypt_password(&encrypted, "server-secret").unwrap(),
            "hunter2"
        );
    }

    #[test]
    fn encrypt_empty_password_is_empty() {
        assert_eq!(encrypt_password("", "server-secret").unwrap(), "");
    }

    #[test]
    fn decrypt_wrong_secret_fails() {
        let encrypted = encrypt_password("hunter2", "server-secret").unwrap();
        assert!(decrypt_password(&encrypted, "wrong-secret").is_err());
    }

    #[test]
    fn encrypt_is_nondeterministic() {
        let a = encrypt_password("hunter2", "server-secret").unwrap();
        let b = encrypt_password("hunter2", "server-secret").unwrap();
        assert_ne!(a, b, "random nonce should make repeated encryptions differ");
    }
}

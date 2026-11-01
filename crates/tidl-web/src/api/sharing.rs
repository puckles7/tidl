//! Shared helpers for the email-invite share flow, used by both
//! `calendars::share_calendar` and `notes::share_note_collection`.

#[cfg(feature = "ssr")]
use leptos::prelude::ServerFnError;

/// Minimal HTML entity escaping for email bodies.
#[cfg(feature = "ssr")]
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build and send a share-invitation email (accept/decline links). Non-fatal
/// no-op if SMTP isn't configured, matching the original inline behaviour in
/// `share_calendar` — a self-hosted instance without SMTP still creates the
/// invitation row for the owner to relay the link manually.
#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
pub(super) async fn send_share_invitation_email(
    pool: &sqlx::SqlitePool,
    server_secret: &str,
    base_url: &str,
    owner: &str,
    resource_noun: &str,
    resource_display: &str,
    perm_label: Option<&str>,
    token: &str,
    invitee_email: &str,
) -> Result<(), ServerFnError> {
    use crate::ssr_types::{decrypt_password, SmtpConfigRow, SMTP_CONFIG_QUERY};
    use lettre::{
        message::{header::ContentType, MultiPart, SinglePart},
        Message,
    };

    let accept_url = format!("{}/share/accept/{}", base_url, token);
    let decline_url = format!("{}/share/decline/{}", base_url, token);
    let perm_clause = perm_label.map(|p| format!(" ({p})")).unwrap_or_default();
    let perm_html = perm_label.map(|p| format!(" ({p})")).unwrap_or_default();

    let subject = format!("{owner} shared a {resource_noun} with you");
    let body_text = format!(
        "{owner} has invited you to access their \"{res}\" {noun}{perm}.\n\n\
         Accept:  {accept}\n\
         Decline: {decline}\n\n\
         This link expires in 7 days.",
        owner = owner,
        res = resource_display,
        noun = resource_noun,
        perm = perm_clause,
        accept = accept_url,
        decline = decline_url,
    );
    let body_html = format!(
        "<p><strong>{owner}</strong> has invited you to access their \
         <strong>{res}</strong> {noun}{perm}.</p>\
         <p>\
           <a href=\"{accept}\" style=\"background:#3b82f6;color:#fff;padding:8px 16px;\
              border-radius:4px;text-decoration:none;display:inline-block\">Accept invitation</a>\
           &nbsp;&nbsp;\
           <a href=\"{decline}\" style=\"color:#6b7280\">Decline</a>\
         </p>\
         <p style=\"color:#6b7280;font-size:0.85em\">This invitation expires in 7 days.</p>",
        owner = html_escape(owner),
        res = html_escape(resource_display),
        noun = resource_noun,
        perm = perm_html,
        accept = accept_url,
        decline = decline_url,
    );

    let cfg_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
        .fetch_optional(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if let Some(cfg) = cfg_row {
        let password =
            decrypt_password(&cfg.password_encrypted, server_secret).map_err(ServerFnError::new)?;
        let from = cfg.header_from();
        let to = invitee_email.to_string();

        let msg =
            Message::builder()
                .from(from.parse().map_err(|e: lettre::address::AddressError| {
                    ServerFnError::new(e.to_string())
                })?)
                .to(to.parse().map_err(|e: lettre::address::AddressError| {
                    ServerFnError::new(e.to_string())
                })?)
                .subject(subject)
                .multipart(
                    MultiPart::alternative()
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(body_text),
                        )
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(body_html),
                        ),
                )
                .map_err(|e| ServerFnError::new(e.to_string()))?;

        crate::ssr_types::smtp_send(
            &cfg.host,
            cfg.port as u16,
            &cfg.tls_mode,
            &cfg.username,
            &password,
            msg,
        )
        .await
        .map_err(ServerFnError::new)?;
    }

    Ok(())
}

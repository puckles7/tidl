//! OIDC-related server functions.

use leptos::prelude::*;

/// Returns the OIDC provider display name if OIDC is configured, or `None`.
/// Used by LoginPage to decide whether to show the "Sign in with …" button.
#[server]
pub async fn get_oidc_config() -> Result<Option<String>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::OidcProviderName;
        use axum::Extension;
        use leptos_axum::extract;

        let Extension(OidcProviderName(name)): Extension<OidcProviderName> = extract().await?;
        return Ok(name);
    }
    #[allow(unreachable_code)]
    Ok(None)
}

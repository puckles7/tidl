//! SSRF guard for server-side fetches of user-supplied URLs.
//!
//! Webcal feed subscriptions (`calendars.sync_url`) and remote CalDAV
//! sources (`remote_sources.remote_base_url`) are both URLs any
//! authenticated user can set, which the server then fetches from
//! (`webcal_sync.rs`, `caldav_client.rs`). Without this guard that lets any
//! regular user turn the server into a probe into its own network — see
//! backlog #141 finding 3.
//!
//! The guard is installed as a custom [`reqwest::dns::Resolve`] rather than
//! a one-time check on the configured URL, so it covers every connection a
//! `reqwest::Client` built with it makes — including redirect targets, and
//! a DNS name that resolves differently on a later sync than it did when
//! the URL was first validated (DNS rebinding).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Returns `true` if `ip` must not be reachable from a server-side fetch of
/// a user-supplied URL: loopback, private (RFC 1918), link-local,
/// unspecified, multicast, or broadcast — including IPv4-mapped/compatible
/// forms embedded in an IPv6 address, which would otherwise smuggle a
/// blocked IPv4 address past an IPv6-only check.
pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(mapped);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_multicast()
        || v4.is_broadcast()
}

/// Environment variable that, when set to `true`/`1`, makes
/// [`guarded_client_builder`] permit internal addresses.
///
/// **Test/dev escape hatch only.** Setting this in production disables the
/// SSRF protection entirely. It exists because the Python e2e suite runs two
/// `tidl-server` instances that sync to each other over `localhost` — an
/// address this guard exists to block.
pub const ALLOW_INTERNAL_ENV: &str = "TIDL_SSRF_ALLOW_INTERNAL";

fn allow_internal_from_env() -> bool {
    matches!(
        std::env::var(ALLOW_INTERNAL_ENV).as_deref(),
        Ok("true") | Ok("1")
    )
}

/// A [`Resolve`]r that resolves hostnames normally via the system resolver,
/// then drops any resolved address for which [`is_blocked`] is true. A name
/// that resolves to nothing but blocked addresses fails to connect with a
/// "no addresses allowed" error rather than silently falling through to the
/// system resolver's result.
#[derive(Debug, Clone, Copy)]
pub struct SsrfGuardedResolver {
    /// When true, internal addresses are permitted (see [`ALLOW_INTERNAL_ENV`]).
    allow_internal: bool,
}

impl SsrfGuardedResolver {
    /// A resolver that blocks internal addresses — the production default.
    pub fn new() -> Self {
        Self {
            allow_internal: false,
        }
    }

    /// A resolver that honours [`ALLOW_INTERNAL_ENV`]. Logs a loud warning
    /// when the override is active so it can't be enabled silently.
    pub fn from_env() -> Self {
        let allow_internal = allow_internal_from_env();
        if allow_internal {
            tracing::warn!(
                "{ALLOW_INTERNAL_ENV} is set — SSRF protection is DISABLED for \
                 server-side URL fetches. This must only be used in tests/dev."
            );
        }
        Self { allow_internal }
    }

    /// Whether this resolver permits internal addresses.
    #[cfg(test)]
    pub fn allows_internal(&self) -> bool {
        self.allow_internal
    }
}

impl Default for SsrfGuardedResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolve for SsrfGuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_internal = self.allow_internal;
        Box::pin(async move {
            let host = name.as_str().to_string();
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| allow_internal || !is_blocked(addr.ip()))
                .collect();
            if resolved.is_empty() {
                return Err(format!("{host}: no non-internal addresses resolved").into());
            }
            Ok(Box::new(resolved.into_iter()) as Addrs)
        })
    }
}

/// A [`reqwest::ClientBuilder`] pre-configured with [`SsrfGuardedResolver`],
/// for building clients that fetch a user-supplied URL.
pub fn guarded_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().dns_resolver(Arc::new(SsrfGuardedResolver::from_env()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_v4_and_v6() {
        assert!(is_blocked("127.0.0.1".parse().unwrap()));
        assert!(is_blocked("::1".parse().unwrap()));
    }

    #[test]
    fn blocks_private_ranges() {
        assert!(is_blocked("10.0.0.1".parse().unwrap()));
        assert!(is_blocked("172.16.0.1".parse().unwrap()));
        assert!(is_blocked("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn blocks_link_local_including_cloud_metadata() {
        // 169.254.169.254 is the AWS/GCP/Azure instance-metadata address —
        // the single highest-value SSRF target this guard needs to stop.
        assert!(is_blocked("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv6_unique_local_and_unicast_link_local() {
        assert!(is_blocked("fc00::1".parse().unwrap()));
        assert!(is_blocked("fe80::1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv4_mapped_private_address() {
        assert!(is_blocked("::ffff:10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn allows_public_addresses() {
        assert!(!is_blocked("1.1.1.1".parse().unwrap()));
        assert!(!is_blocked("2606:4700:4700::1111".parse().unwrap()));
    }

    /// End-to-end check that the resolver is actually plumbed into the
    /// client, not just that `is_blocked` is correct in isolation: a
    /// guarded client must fail to resolve `localhost`, before ever
    /// attempting to connect. Uses [`SsrfGuardedResolver::new`] directly so it
    /// is independent of the (test-only) `TIDL_SSRF_ALLOW_INTERNAL` override.
    #[tokio::test]
    async fn guarded_client_refuses_to_resolve_localhost() {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(SsrfGuardedResolver::new()))
            .build()
            .unwrap();
        let err = client
            .get("http://localhost:1/")
            .send()
            .await
            .expect_err("request to localhost must fail to resolve, not connect");
        assert!(
            err.is_connect(),
            "expected a resolver/connect-stage error, got: {err}"
        );
    }

    #[test]
    fn default_resolver_blocks_internal() {
        assert!(!SsrfGuardedResolver::new().allows_internal());
        assert!(!SsrfGuardedResolver::default().allows_internal());
    }

    #[test]
    fn allow_internal_env_parsing() {
        use std::sync::Mutex;
        // Serialise against any other env-var test in this binary.
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _lock = ENV_LOCK.lock().unwrap();

        let prev = std::env::var(ALLOW_INTERNAL_ENV).ok();
        unsafe {
            std::env::set_var(ALLOW_INTERNAL_ENV, "1");
        }
        assert!(allow_internal_from_env());
        assert!(SsrfGuardedResolver::from_env().allows_internal());
        unsafe {
            std::env::set_var(ALLOW_INTERNAL_ENV, "true");
        }
        assert!(allow_internal_from_env());
        unsafe {
            std::env::set_var(ALLOW_INTERNAL_ENV, "false");
        }
        assert!(!allow_internal_from_env());
        assert!(!SsrfGuardedResolver::from_env().allows_internal());
        unsafe {
            std::env::remove_var(ALLOW_INTERNAL_ENV);
        }
        assert!(!allow_internal_from_env());
        if let Some(v) = prev {
            unsafe {
                std::env::set_var(ALLOW_INTERNAL_ENV, v);
            }
        }
    }
}

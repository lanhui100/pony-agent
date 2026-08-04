//! Web access policy and pinned connector (PA-076 design Decision 8, tasks 6.1-6.2).
//!
//! This module is a pure, hermetic decision surface. It performs **no** network I/O and
//! owns no HTTP client. It answers one question: given a URL (initial or redirect) and an
//! injected [`WebResolver`], may the agent open a connection, and if so, to which addresses
//! with which authority/SNI must the connection be pinned?
//!
//! Security posture (fail closed):
//! - Only `http`/`https` schemes are allowed.
//! - URLs carrying userinfo (credentials) are rejected.
//! - Hosts that are literal IPs in a forbidden range are rejected without consulting DNS.
//! - Hostnames resolve through an injected [`WebResolver`]; **every** returned A/AAAA record
//!   must pass. If any record is forbidden, the whole URL is denied.
//! - Forbidden address classes: loopback, private (RFC1918), IPv6 unique-local, link-local,
//!   unspecified, multicast, reserved, IPv4-mapped IPv6, and IPv4-compatible IPv6.
//! - `localhost` (and `*.localhost`) hostnames are denied by name, independent of DNS.
//! - Restricted ports (0 plus a small deny-list) are rejected.
//! - Redirects are never followed automatically. The policy validates each hop explicitly
//!   and enforces a bounded redirect budget (`max_redirects`, default 5). Every redirect is
//!   re-parsed, re-resolved, and re-validated against the same rules.
//! - There is **no ambient proxy**: this module never reads proxy environment variables and
//!   never builds a proxy-aware client. A trusted proxy, if ever introduced, must be injected
//!   explicitly and carry the same target-validation responsibility.
//!
//! [`PinnedConnector`] combines a policy with a resolver and returns the allowed target
//! addresses plus the authority/SNI to preserve for each validated URL. A caller that then
//! opens a real socket **must** verify the actual peer IP belongs to the returned address set
//! and, for TLS, that the certificate matches `sni_host`.

use crate::agent::tool_runtime::WebResolver;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Host;
use url::Url;

/// Default maximum number of redirects the policy will validate (design Decision 8: at most 5).
pub const DEFAULT_MAX_REDIRECTS: usize = 5;

/// Small default deny-list of infrastructure/metadata ports that are never legitimate HTTP(S)
/// targets for the agent. Intentionally conservative: generic dev/web ports (e.g. 3000, 8080)
/// are **not** listed so legitimate public servers on common web ports keep working; internal
/// services are already unreachable via the address deny rules. Callers may supply their own
/// list via [`WebAccessPolicy::with_restricted_ports`].
pub const DEFAULT_RESTRICTED_PORTS: &[u16] = &[
    0, 7, 20, 21, 22, 23, 25, 53, 69, 110, 111, 135, 139, 143, 161, 389, 445, 873, 1080,
    1433, 1521, 2049, 2181, 2375, 2376, 3128, 3306, 3389, 4369, 5432, 5900, 5985, 5986,
    6379, 7001, 9092, 9200, 9300, 11211, 15672, 27017, 50000,
];

/// The access decision for one URL or a whole redirect chain.
///
/// `Allow` carries everything a pinned connector needs: the validated addresses to connect to,
/// the authority to preserve (Host header), the hostname to present as SNI, the effective port,
/// and whether TLS is required. `Deny` carries a structured reason plus the hop at which the
/// URL was denied (0 = initial URL, N = Nth redirect).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision", content = "detail")]
pub enum WebAccessDecision {
    Allow {
        resolved_addrs: Vec<IpAddr>,
        authority: String,
        sni_host: String,
        port: u16,
        is_tls: bool,
    },
    Deny {
        reason: WebAccessDenyReason,
        hop: usize,
    },
}

impl WebAccessDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, WebAccessDecision::Allow { .. })
    }

    pub fn resolved_addrs(&self) -> &[IpAddr] {
        match self {
            WebAccessDecision::Allow { resolved_addrs, .. } => resolved_addrs,
            WebAccessDecision::Deny { .. } => &[],
        }
    }
}

/// Structured reason a URL was denied. Serialized as `{ "code": "...", "detail": ... }` so
/// callers and telemetry can surface machine-readable evidence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum WebAccessDenyReason {
    /// The URL failed standard parsing (malformed, bad port, bad percent-encoding, ...).
    InvalidUrl(String),
    /// Scheme is not `http` or `https`.
    UnsupportedScheme { scheme: String },
    /// No host present.
    MissingHost,
    /// URL carries a username/password.
    CredentialsNotAllowed,
    /// Hostname is `localhost` / `*.localhost`, denied by name.
    HostForbidden { host: String, detail: String },
    /// The effective port is 0 or on the restricted deny-list.
    RestrictedPort { port: u16 },
    /// The host is a literal IP that falls in a forbidden address class.
    ForbiddenLiteralAddress { address: String, detail: String },
    /// At least one resolved address falls in a forbidden address class.
    ResolvesToForbiddenAddress { host: String, address: String, detail: String },
    /// DNS returned no usable records.
    NoAddresses { host: String },
    /// The injected resolver failed for the host.
    ResolutionFailed { host: String, error: String },
    /// The redirect budget was exceeded.
    TooManyRedirects { limit: usize },
}

/// Web access policy: scheme, credentials, host/literal-IP, DNS, port, and redirect rules.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebAccessPolicy {
    /// Maximum number of redirect hops that may be validated (default [`DEFAULT_MAX_REDIRECTS`]).
    pub max_redirects: usize,
    /// Ports that are denied even for public hosts.
    pub restricted_ports: Vec<u16>,
}

impl Default for WebAccessPolicy {
    fn default() -> Self {
        Self {
            max_redirects: DEFAULT_MAX_REDIRECTS,
            restricted_ports: DEFAULT_RESTRICTED_PORTS.to_vec(),
        }
    }
}

impl WebAccessPolicy {
    /// Validate an initial URL (hop 0).
    pub fn validate(&self, url: &str, resolver: &dyn WebResolver) -> WebAccessDecision {
        self.validate_at(url, resolver, 0)
    }

    /// Validate one more redirect hop. `hops_so_far` is the number of redirects already
    /// consumed (0 before the first redirect). The redirect budget is enforced here, so a
    /// redirect that would exceed `max_redirects` is denied before parsing.
    pub fn validate_redirect(
        &self,
        url: &str,
        resolver: &dyn WebResolver,
        hops_so_far: usize,
    ) -> WebAccessDecision {
        if hops_so_far >= self.max_redirects {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::TooManyRedirects {
                    limit: self.max_redirects,
                },
                hop: hops_so_far + 1,
            };
        }
        self.validate_at(url, resolver, hops_so_far + 1)
    }

    /// Validate a full chain at once: `urls[0]` is the initial URL and every following entry
    /// is a redirect target. Enforces the same per-hop rules plus the redirect budget, and
    /// returns the decision for the final hop when every hop is allowed.
    pub fn validate_chain(&self, urls: &[&str], resolver: &dyn WebResolver) -> WebAccessDecision {
        if urls.is_empty() {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::InvalidUrl("empty URL chain".to_string()),
                hop: 0,
            };
        }
        let mut last_allow = None;
        for (index, url) in urls.iter().enumerate() {
            if index > self.max_redirects {
                return WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::TooManyRedirects {
                        limit: self.max_redirects,
                    },
                    hop: index,
                };
            }
            match self.validate_at(url, resolver, index) {
                allow @ WebAccessDecision::Allow { .. } => last_allow = Some(allow),
                deny => return deny,
            }
        }
        last_allow.unwrap_or_else(|| WebAccessDecision::Deny {
            reason: WebAccessDenyReason::InvalidUrl("empty URL chain".to_string()),
            hop: 0,
        })
    }

    /// Resolve a (possibly relative) redirect `Location` target against `base_url`, without
    /// validating or connecting. The result must still be passed through
    /// [`Self::validate_redirect`] before any connection.
    pub fn resolve_redirect_target(&self, base_url: &str, target: &str) -> Result<String, String> {
        let base = Url::parse(base_url).map_err(|error| format!("invalid base URL: {error}"))?;
        base.join(target)
            .map(|joined| joined.to_string())
            .map_err(|error| format!("cannot resolve redirect target: {error}"))
    }

    /// Replace the restricted-port deny-list for this policy.
    pub fn with_restricted_ports(mut self, ports: Vec<u16>) -> Self {
        self.restricted_ports = ports;
        self
    }

    /// Replace the redirect budget for this policy.
    pub fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    fn validate_at(
        &self,
        url: &str,
        resolver: &dyn WebResolver,
        hop: usize,
    ) -> WebAccessDecision {
        let parsed = match Url::parse(url) {
            Ok(parsed) => parsed,
            Err(error) => {
                return WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::InvalidUrl(error.to_string()),
                    hop,
                }
            }
        };

        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::UnsupportedScheme {
                    scheme: scheme.to_string(),
                },
                hop,
            };
        }

        if !parsed.username().is_empty() || parsed.password().is_some() {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::CredentialsNotAllowed,
                hop,
            };
        }

        let host = parsed.host_str().unwrap_or("").to_string();
        if host.is_empty() {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::MissingHost,
                hop,
            };
        }

        // `localhost` and `*.localhost` are denied by name regardless of what DNS would say.
        let host_lower = host.to_ascii_lowercase();
        if host_lower == "localhost" || host_lower.ends_with(".localhost") {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::HostForbidden {
                    host: host.clone(),
                    detail: "localhost".to_string(),
                },
                hop,
            };
        }

        // Literal IP hosts are validated directly, without consulting DNS.
        let literal_ip: Option<IpAddr> = match parsed.host() {
            Some(Host::Ipv4(address)) => Some(IpAddr::V4(address)),
            Some(Host::Ipv6(address)) => Some(IpAddr::V6(address)),
            _ => host.parse::<IpAddr>().ok(),
        };

        let effective_port = parsed.port_or_known_default().unwrap_or(0);
        if effective_port == 0 || self.restricted_ports.contains(&effective_port) {
            return WebAccessDecision::Deny {
                reason: WebAccessDenyReason::RestrictedPort { port: effective_port },
                hop,
            };
        }

        let resolved_addrs = if let Some(ip) = literal_ip {
            if let Some(detail) = classify_ip(ip) {
                return WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::ForbiddenLiteralAddress {
                        address: ip.to_string(),
                        detail: detail.to_string(),
                    },
                    hop,
                };
            }
            vec![ip]
        } else {
            match resolver.resolve(&host) {
                Ok(addresses) => {
                    if addresses.is_empty() {
                        return WebAccessDecision::Deny {
                            reason: WebAccessDenyReason::NoAddresses { host: host.clone() },
                            hop,
                        };
                    }
                    for address in &addresses {
                        if let Some(detail) = classify_ip(*address) {
                            return WebAccessDecision::Deny {
                                reason: WebAccessDenyReason::ResolvesToForbiddenAddress {
                                    host: host.clone(),
                                    address: address.to_string(),
                                    detail: detail.to_string(),
                                },
                                hop,
                            };
                        }
                    }
                    addresses
                }
                Err(error) => {
                    return WebAccessDecision::Deny {
                        reason: WebAccessDenyReason::ResolutionFailed {
                            host: host.clone(),
                            error,
                        },
                        hop,
                    };
                }
            }
        };

        let is_tls = scheme == "https";
        // `url.host_str()` returns a bracketed string for IPv6 literals, so derive the
        // unbracketed SNI host from the typed host and re-bracket it only for the authority.
        let sni_host = match parsed.host() {
            Some(Host::Ipv4(address)) => address.to_string(),
            Some(Host::Ipv6(address)) => address.to_string(),
            Some(Host::Domain(domain)) => domain.to_string(),
            _ => host.clone(),
        };
        let is_ipv6_literal = matches!(parsed.host(), Some(Host::Ipv6(_)));
        let host_for_authority = if is_ipv6_literal {
            format!("[{sni_host}]")
        } else {
            sni_host.clone()
        };
        let authority = format!("{host_for_authority}:{effective_port}");

        WebAccessDecision::Allow {
            resolved_addrs,
            authority,
            sni_host,
            port: effective_port,
            is_tls,
        }
    }
}

/// Injected resolver + pinned connector: given a policy, a resolver, and a target, returns the
/// allowed target addresses to connect to and the authority/SNI to preserve. Pure decision
/// module — performs no network I/O and never follows redirects automatically.
#[derive(Clone, Copy)]
pub struct PinnedConnector<'a> {
    policy: &'a WebAccessPolicy,
    resolver: &'a dyn WebResolver,
}

impl<'a> PinnedConnector<'a> {
    pub fn new(policy: &'a WebAccessPolicy, resolver: &'a dyn WebResolver) -> Self {
        Self { policy, resolver }
    }

    /// Validate the initial URL and return the pinned connection decision.
    pub fn validate_initial(&self, url: &str) -> WebAccessDecision {
        self.policy.validate(url, self.resolver)
    }

    /// Validate one redirect hop (see [`WebAccessPolicy::validate_redirect`]).
    pub fn validate_redirect(&self, url: &str, hops_so_far: usize) -> WebAccessDecision {
        self.policy.validate_redirect(url, self.resolver, hops_so_far)
    }

    /// Validate a whole chain (see [`WebAccessPolicy::validate_chain`]).
    pub fn validate_chain(&self, urls: &[&str]) -> WebAccessDecision {
        self.policy.validate_chain(urls, self.resolver)
    }

    /// Resolve a redirect `Location` against the current URL (see
    /// [`WebAccessPolicy::resolve_redirect_target`]).
    pub fn resolve_redirect_target(&self, base_url: &str, target: &str) -> Result<String, String> {
        self.policy.resolve_redirect_target(base_url, target)
    }

    pub fn policy(&self) -> &WebAccessPolicy {
        self.policy
    }

    pub fn resolver(&self) -> &dyn WebResolver {
        self.resolver
    }
}

/// Returns a human-readable class label for a forbidden address, or `None` when the address is
/// allowed. Every class here is denied for both literal hosts and resolved records.
fn classify_ip(address: IpAddr) -> Option<&'static str> {
    match address {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(address: Ipv4Addr) -> Option<&'static str> {
    let octets = address.octets();
    match octets[0] {
        0 => Some("unspecified"),
        10 => Some("private (RFC1918)"),
        127 => Some("loopback"),
        169 if octets[1] == 254 => Some("link-local"),
        172 if (16..=31).contains(&octets[1]) => Some("private (RFC1918)"),
        192 if octets[1] == 168 => Some("private (RFC1918)"),
        224..=239 => Some("multicast"),
        240..=255 => Some("reserved"),
        _ => None,
    }
}

fn classify_v6(address: Ipv6Addr) -> Option<&'static str> {
    if address.is_unspecified() {
        return Some("unspecified");
    }
    if address.is_loopback() {
        return Some("loopback");
    }
    if address.to_ipv4_mapped().is_some() {
        return Some("ipv4-mapped-ipv6");
    }
    if address.to_ipv4().is_some() {
        return Some("ipv4-compatible-ipv6");
    }
    let segments = address.segments();
    if segments[0] & 0xfe00 == 0xfc00 {
        return Some("unique-local (private)");
    }
    if segments[0] & 0xffc0 == 0xfe80 {
        return Some("link-local");
    }
    if segments[0] & 0xffc0 == 0xfec0 {
        return Some("site-local (deprecated)");
    }
    if address.is_multicast() {
        return Some("multicast");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::FakeResolver;

    fn public_resolver() -> FakeResolver {
        let resolver = FakeResolver::default();
        resolver.set_addresses("public.example", vec!["203.0.113.10".parse().unwrap()]);
        resolver
    }

    fn assert_denied(decision: &WebAccessDecision, code: &str, hop: usize) {
        match decision {
            WebAccessDecision::Deny { reason, hop: actual } => {
                let reason_code = match reason {
                    WebAccessDenyReason::InvalidUrl(_) => "invalid_url",
                    WebAccessDenyReason::UnsupportedScheme { .. } => "unsupported_scheme",
                    WebAccessDenyReason::MissingHost => "missing_host",
                    WebAccessDenyReason::CredentialsNotAllowed => "credentials_not_allowed",
                    WebAccessDenyReason::HostForbidden { .. } => "host_forbidden",
                    WebAccessDenyReason::RestrictedPort { .. } => "restricted_port",
                    WebAccessDenyReason::ForbiddenLiteralAddress { .. } => "forbidden_literal_address",
                    WebAccessDenyReason::ResolvesToForbiddenAddress { .. } => {
                        "resolves_to_forbidden_address"
                    }
                    WebAccessDenyReason::NoAddresses { .. } => "no_addresses",
                    WebAccessDenyReason::ResolutionFailed { .. } => "resolution_failed",
                    WebAccessDenyReason::TooManyRedirects { .. } => "too_many_redirects",
                };
                assert_eq!(reason_code, code, "unexpected deny code for {reason:?}");
                assert_eq!(*actual, hop, "deny should report hop {hop}");
            }
            WebAccessDecision::Allow { .. } => {
                panic!("expected deny `{code}` but got allow: {decision:?}")
            }
        }
    }

    #[test]
    fn public_only_host_is_allowed_with_pinned_addresses() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://public.example/");

        match decision {
            WebAccessDecision::Allow {
                resolved_addrs,
                authority,
                sni_host,
                port,
                is_tls,
            } => {
                assert_eq!(resolved_addrs, vec!["203.0.113.10".parse::<IpAddr>().unwrap()]);
                assert_eq!(authority, "public.example:80");
                assert_eq!(sni_host, "public.example");
                assert_eq!(port, 80);
                assert!(!is_tls);
            }
            WebAccessDecision::Deny { .. } => panic!("public host should be allowed: {decision:?}"),
        }
    }

    #[test]
    fn https_public_host_reports_tls_and_default_port() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("https://public.example/");
        match decision {
            WebAccessDecision::Allow {
                resolved_addrs,
                authority,
                port,
                is_tls,
                ..
            } => {
                assert_eq!(resolved_addrs, vec!["203.0.113.10".parse::<IpAddr>().unwrap()]);
                assert_eq!(authority, "public.example:443");
                assert_eq!(port, 443);
                assert!(is_tls);
            }
            WebAccessDecision::Deny { .. } => panic!("https public host should be allowed"),
        }
    }

    #[test]
    fn literal_loopback_is_rejected_without_dns() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://127.0.0.1:8080/");
        assert_denied(&decision, "forbidden_literal_address", 0);
        let WebAccessDecision::Deny { reason, .. } = decision else {
            unreachable!()
        };
        assert_eq!(
            reason,
            WebAccessDenyReason::ForbiddenLiteralAddress {
                address: "127.0.0.1".to_string(),
                detail: "loopback".to_string()
            }
        );
    }

    #[test]
    fn localhost_hostname_is_rejected_by_name_without_dns() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        assert_denied(&connector.validate_initial("http://localhost/"), "host_forbidden", 0);
        assert_denied(
            &connector.validate_initial("http://sub.localhost:8080/"),
            "host_forbidden",
            0,
        );
    }

    #[test]
    fn literal_private_link_local_and_unspecified_are_rejected_without_dns() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        // Cloud metadata endpoint (link-local).
        assert_denied(
            &connector.validate_initial("http://169.254.169.254/latest/meta-data/"),
            "forbidden_literal_address",
            0,
        );
        // RFC1918.
        assert_denied(&connector.validate_initial("http://10.0.0.1/"), "forbidden_literal_address", 0);
        assert_denied(
            &connector.validate_initial("http://192.168.1.10/"),
            "forbidden_literal_address",
            0,
        );
        // Unspecified.
        assert_denied(&connector.validate_initial("http://0.0.0.0/"), "forbidden_literal_address", 0);
        // IPv6 loopback.
        assert_denied(&connector.validate_initial("http://[::1]/"), "forbidden_literal_address", 0);
    }

    #[test]
    fn private_dns_record_is_rejected() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("internal.example", vec!["10.0.0.5".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        let decision = connector.validate_initial("http://internal.example/");
        assert_denied(&decision, "resolves_to_forbidden_address", 0);
        let WebAccessDecision::Deny { reason, .. } = decision else {
            unreachable!()
        };
        assert_eq!(
            reason,
            WebAccessDenyReason::ResolvesToForbiddenAddress {
                host: "internal.example".to_string(),
                address: "10.0.0.5".to_string(),
                detail: "private (RFC1918)".to_string()
            }
        );
    }

    #[test]
    fn multiple_a_records_rejected_if_any_is_forbidden() {
        let resolver = FakeResolver::default();
        resolver.set_addresses(
            "mixed.example",
            vec![
                "203.0.113.10".parse().unwrap(),
                "192.168.1.1".parse().unwrap(),
                "8.8.8.8".parse().unwrap(),
            ],
        );
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        assert_denied(
            &connector.validate_initial("http://mixed.example/"),
            "resolves_to_forbidden_address",
            0,
        );

        let public_resolver = FakeResolver::default();
        public_resolver.set_addresses(
            "cdn.example",
            vec!["203.0.113.20".parse().unwrap(), "8.8.4.4".parse().unwrap()],
        );
        let connector = PinnedConnector::new(&policy, &public_resolver);
        let decision = connector.validate_initial("http://cdn.example/");
        assert!(decision.is_allowed(), "public-only records must be allowed: {decision:?}");
        assert_eq!(decision.resolved_addrs().len(), 2);
    }

    #[test]
    fn credentials_in_url_are_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        assert_denied(
            &connector.validate_initial("http://user:pass@public.example/"),
            "credentials_not_allowed",
            0,
        );
        assert_denied(
            &connector.validate_initial("http://user@public.example/"),
            "credentials_not_allowed",
            0,
        );
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        assert_denied(&connector.validate_initial("ftp://public.example/file"), "unsupported_scheme", 0);
        assert_denied(&connector.validate_initial("file:///etc/passwd"), "unsupported_scheme", 0);
        assert_denied(&connector.validate_initial("data:text/plain,hello"), "unsupported_scheme", 0);
    }

    #[test]
    fn ipv4_mapped_ipv6_is_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        // The embedded IPv4 (198.51.100.7) is public; the class itself is still forbidden.
        let decision = connector.validate_initial("http://[::ffff:198.51.100.7]/");
        assert_denied(&decision, "forbidden_literal_address", 0);
        let WebAccessDecision::Deny { reason, .. } = decision else {
            unreachable!()
        };
        assert_eq!(
            reason,
            WebAccessDenyReason::ForbiddenLiteralAddress {
                address: "::ffff:198.51.100.7".to_string(),
                detail: "ipv4-mapped-ipv6".to_string()
            }
        );
        // Mapped loopback is also rejected.
        assert_denied(
            &connector.validate_initial("http://[::ffff:127.0.0.1]/"),
            "forbidden_literal_address",
            0,
        );
    }

    #[test]
    fn public_ipv6_literal_is_allowed_and_bracketed_in_authority() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://[2001:4860:4860::8888]:8080/");
        match decision {
            WebAccessDecision::Allow {
                resolved_addrs,
                authority,
                sni_host,
                port,
                ..
            } => {
                assert_eq!(
                    resolved_addrs,
                    vec!["2001:4860:4860::8888".parse::<IpAddr>().unwrap()]
                );
                assert_eq!(authority, "[2001:4860:4860::8888]:8080");
                assert_eq!(sni_host, "2001:4860:4860::8888");
                assert_eq!(port, 8080);
            }
            WebAccessDecision::Deny { .. } => panic!("public IPv6 should be allowed"),
        }
    }

    #[test]
    fn restricted_port_is_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://public.example:22/");
        assert_denied(&decision, "restricted_port", 0);
        let WebAccessDecision::Deny { reason, .. } = decision else {
            unreachable!()
        };
        assert_eq!(
            reason,
            WebAccessDenyReason::RestrictedPort { port: 22 }
        );
    }

    #[test]
    fn port_zero_is_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://public.example:0/");
        assert_denied(&decision, "restricted_port", 0);
    }

    #[test]
    fn custom_port_deny_list_is_applied() {
        let policy = WebAccessPolicy::default().with_restricted_ports(vec![8080]);
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://public.example:8080/");
        assert_denied(&decision, "restricted_port", 0);
    }

    #[test]
    fn unknown_host_resolver_error_is_denied() {
        let resolver = FakeResolver::default();
        resolver.set_error("broken.example", "NXDOMAIN (simulated)");
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://broken.example/");
        assert_denied(&decision, "resolution_failed", 0);
        let WebAccessDecision::Deny { reason, .. } = decision else {
            unreachable!()
        };
        assert_eq!(
            reason,
            WebAccessDenyReason::ResolutionFailed {
                host: "broken.example".to_string(),
                error: "NXDOMAIN (simulated)".to_string()
            }
        );
    }

    #[test]
    fn empty_authority_urls_are_rejected() {
        // The `url` crate rejects empty hosts for http(s) with `EmptyHost`, so these fail
        // closed as invalid URLs (the `MissingHost` branch remains as defense-in-depth in
        // case a future parser version ever yields an empty host).
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        assert_denied(&connector.validate_initial("http://"), "invalid_url", 0);
        assert_denied(&connector.validate_initial("http://:8080/"), "invalid_url", 0);
    }

    #[test]
    fn malformed_url_is_rejected() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://exa mple.com/");
        assert_denied(&decision, "invalid_url", 0);
    }

    #[test]
    fn five_hop_redirect_chain_is_allowed_but_sixth_is_rejected() {
        let resolver = FakeResolver::default();
        for index in 0..7 {
            resolver.set_addresses(
                format!("hop{index}.example"),
                vec!["203.0.113.50".parse().unwrap()],
            );
        }
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        let chain = [
            "http://hop0.example/",
            "http://hop1.example/",
            "http://hop2.example/",
            "http://hop3.example/",
            "http://hop4.example/",
            "http://hop5.example/",
        ];
        let decision = connector.validate_chain(&chain);
        assert!(decision.is_allowed(), "5-hop chain must be allowed: {decision:?}");

        let too_long = [
            "http://hop0.example/",
            "http://hop1.example/",
            "http://hop2.example/",
            "http://hop3.example/",
            "http://hop4.example/",
            "http://hop5.example/",
            "http://hop6.example/",
        ];
        let decision = connector.validate_chain(&too_long);
        assert_denied(&decision, "too_many_redirects", 6);
    }

    #[test]
    fn validate_redirect_enforces_budget_per_hop() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("public.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        assert!(connector.validate_redirect("http://public.example/", 4).is_allowed());
        assert_denied(&connector.validate_redirect("http://public.example/", 5), "too_many_redirects", 6);
    }

    #[test]
    fn redirect_to_private_is_rejected_at_its_hop() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("public.example", vec!["203.0.113.10".parse().unwrap()]);
        resolver.set_addresses("internal.example", vec!["192.168.1.99".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        let chain = ["http://public.example/", "http://internal.example/"];
        let decision = connector.validate_chain(&chain);
        assert_denied(&decision, "resolves_to_forbidden_address", 1);
    }

    #[test]
    fn relative_redirect_target_is_resolved_against_base() {
        let policy = WebAccessPolicy::default();
        let resolved = policy
            .resolve_redirect_target("https://public.example/a/b/c", "../d")
            .expect("relative join should succeed");
        // RFC 3986: `..` removes the last path segment of the base (`/a/b/c` -> `/a`).
        assert_eq!(resolved, "https://public.example/a/d");

        let absolute = policy
            .resolve_redirect_target("https://public.example/a", "https://other.example/x")
            .expect("absolute join should succeed");
        assert_eq!(absolute, "https://other.example/x");
    }

    #[test]
    fn deny_decisions_serialize_with_code_and_detail() {
        let policy = WebAccessPolicy::default();
        let resolver = public_resolver();
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://user:pass@public.example/");
        let value = serde_json::to_value(&decision).expect("deny should serialize");
        assert_eq!(value["decision"], "deny");
        assert_eq!(value["detail"]["reason"]["code"], "credentials_not_allowed");
    }
}

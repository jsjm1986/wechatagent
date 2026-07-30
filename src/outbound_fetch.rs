//! Security boundary for administrator-configured outbound HTTP fetches.
//!
//! The ingest worker is intentionally the first consumer. Every request hop is
//! parsed, resolved, checked for public-only addresses, and then connected with
//! reqwest DNS overrides pinned to those checked addresses. Automatic redirects
//! and environment/system proxies are disabled so neither can bypass the check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use futures::StreamExt;
use reqwest::{redirect::Policy, StatusCode};
use thiserror::Error;
use tokio::net::lookup_host;
use url::{Host, Url};

pub(crate) const MAX_INGEST_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub(crate) enum OutboundFetchError {
    #[error("URL must be an absolute http or https URL")]
    InvalidUrl,
    #[error("URL credentials are not allowed")]
    CredentialsNotAllowed,
    #[error("URL host is not allowed")]
    HostNotAllowed,
    #[error("URL host did not resolve")]
    DnsNoAddresses,
    #[error("URL host resolution failed")]
    DnsFailed,
    #[error("URL resolves to a non-public network address")]
    NonPublicAddress,
    #[error("outbound HTTP client setup failed")]
    ClientSetup,
    #[error("outbound HTTP request failed")]
    RequestFailed,
    #[error("outbound connection address could not be verified")]
    MissingRemoteAddress,
    #[error("redirect response is missing a valid Location header")]
    InvalidRedirect,
    #[error("redirect limit exceeded")]
    TooManyRedirects,
    #[error("response content type is missing or not allowed for {0}")]
    ContentTypeNotAllowed(String),
    #[error("response body exceeds {MAX_INGEST_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
}

pub(crate) struct SafeFetchResponse {
    pub status: StatusCode,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

struct ResolvedTarget {
    url: Url,
    domain: Option<String>,
    addresses: Vec<SocketAddr>,
}

/// Validate and normalize a configured URL before persistence. The worker calls
/// the same resolver again immediately before every request, so a later DNS
/// change cannot rely on this save-time decision.
pub(crate) async fn validate_public_http_url(raw: &str) -> Result<Url, OutboundFetchError> {
    Ok(resolve_public_target(parse_public_http_url(raw)?)
        .await?
        .url)
}

/// Fetch an ingest source with public-only DNS pinning, manual redirect checks,
/// no proxies, a strict content type, and a streaming body limit.
pub(crate) async fn fetch_ingest_url(
    raw: &str,
    etag: Option<&str>,
    source_kind: &str,
) -> Result<SafeFetchResponse, OutboundFetchError> {
    let mut current = parse_public_http_url(raw)?;

    for redirect_count in 0..=MAX_REDIRECTS {
        let target = resolve_public_target(current).await?;
        let mut builder = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .referer(false)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent("wechatagent-ingest/1.0");
        if let Some(domain) = target.domain.as_deref() {
            builder = builder.resolve_to_addrs(domain, &target.addresses);
        }
        let client = builder
            .build()
            .map_err(|_| OutboundFetchError::ClientSetup)?;
        let mut request = client.get(target.url.clone());
        // Do not forward a conditional token to a redirect target. Content hash
        // dedupe still prevents duplicate ingestion after a stable redirect.
        if redirect_count == 0 {
            if let Some(etag) = etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
        }
        let response = request
            .send()
            .await
            .map_err(|_| OutboundFetchError::RequestFailed)?;
        let remote = response
            .remote_addr()
            .ok_or(OutboundFetchError::MissingRemoteAddress)?;
        if !is_public_ip(remote.ip())
            || !target
                .addresses
                .iter()
                .any(|resolved| resolved.ip() == remote.ip())
        {
            return Err(OutboundFetchError::NonPublicAddress);
        }

        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(SafeFetchResponse {
                status,
                etag: None,
                body: Vec::new(),
            });
        }
        if is_follow_redirect(status) {
            if redirect_count == MAX_REDIRECTS {
                return Err(OutboundFetchError::TooManyRedirects);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(OutboundFetchError::InvalidRedirect)?;
            current = resolve_redirect_url(&target.url, location)?;
            continue;
        }

        let response_etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if !status.is_success() {
            return Ok(SafeFetchResponse {
                status,
                etag: response_etag,
                body: Vec::new(),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        if !content_type_allowed(source_kind, content_type) {
            return Err(OutboundFetchError::ContentTypeNotAllowed(
                source_kind.to_string(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_INGEST_RESPONSE_BYTES as u64)
        {
            return Err(OutboundFetchError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| OutboundFetchError::RequestFailed)?;
            append_limited(&mut body, &chunk, MAX_INGEST_RESPONSE_BYTES)?;
        }
        return Ok(SafeFetchResponse {
            status,
            etag: response_etag,
            body,
        });
    }

    Err(OutboundFetchError::TooManyRedirects)
}

fn parse_public_http_url(raw: &str) -> Result<Url, OutboundFetchError> {
    let mut url = Url::parse(raw.trim()).map_err(|_| OutboundFetchError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(OutboundFetchError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OutboundFetchError::CredentialsNotAllowed);
    }
    if url.port() == Some(0) {
        return Err(OutboundFetchError::HostNotAllowed);
    }
    if let Some(Host::Domain(domain)) = url.host() {
        let normalized = domain.trim_end_matches('.').to_ascii_lowercase();
        if normalized.is_empty() || normalized == "localhost" || normalized.ends_with(".localhost")
        {
            return Err(OutboundFetchError::HostNotAllowed);
        }
        url.set_host(Some(&normalized))
            .map_err(|_| OutboundFetchError::HostNotAllowed)?;
    }
    url.set_fragment(None);
    Ok(url)
}

async fn resolve_public_target(url: Url) -> Result<ResolvedTarget, OutboundFetchError> {
    let port = url
        .port_or_known_default()
        .ok_or(OutboundFetchError::InvalidUrl)?;
    let (domain, mut addresses) = match url.host().ok_or(OutboundFetchError::InvalidUrl)? {
        Host::Ipv4(address) => (None, vec![SocketAddr::new(IpAddr::V4(address), port)]),
        Host::Ipv6(address) => (None, vec![SocketAddr::new(IpAddr::V6(address), port)]),
        Host::Domain(domain) => {
            let resolved = tokio::time::timeout(DNS_TIMEOUT, lookup_host((domain, port)))
                .await
                .map_err(|_| OutboundFetchError::DnsFailed)?
                .map_err(|_| OutboundFetchError::DnsFailed)?
                .collect::<Vec<_>>();
            (Some(domain.to_string()), resolved)
        }
    };
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(OutboundFetchError::DnsNoAddresses);
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(OutboundFetchError::NonPublicAddress);
    }
    Ok(ResolvedTarget {
        url,
        domain,
        addresses,
    })
}

fn resolve_redirect_url(base: &Url, location: &str) -> Result<Url, OutboundFetchError> {
    let joined = base
        .join(location)
        .map_err(|_| OutboundFetchError::InvalidRedirect)?;
    parse_public_http_url(joined.as_str()).map_err(|_| OutboundFetchError::InvalidRedirect)
}

fn is_follow_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn content_type_allowed(source_kind: &str, content_type: Option<&str>) -> bool {
    let Some(media_type) = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    match source_kind {
        "rss" => matches!(
            media_type.as_str(),
            "application/rss+xml"
                | "application/atom+xml"
                | "application/xml"
                | "text/xml"
                | "text/plain"
        ),
        "html" => matches!(
            media_type.as_str(),
            "text/html" | "application/xhtml+xml" | "text/plain"
        ),
        _ => false,
    }
}

fn append_limited(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
) -> Result<(), OutboundFetchError> {
    if chunk.len() > limit.saturating_sub(buffer.len()) {
        return Err(OutboundFetchError::ResponseTooLarge);
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    ![
        ("0.0.0.0", 8),
        ("10.0.0.0", 8),
        ("100.64.0.0", 10),
        ("127.0.0.0", 8),
        ("169.254.0.0", 16),
        ("172.16.0.0", 12),
        ("192.0.0.0", 24),
        ("192.0.2.0", 24),
        ("192.88.99.0", 24),
        ("192.168.0.0", 16),
        ("198.18.0.0", 15),
        ("198.51.100.0", 24),
        ("203.0.113.0", 24),
        ("224.0.0.0", 4),
        ("240.0.0.0", 4),
    ]
    .into_iter()
    .any(|(base, prefix)| {
        ipv4_in_prefix(value, u32::from(base.parse::<Ipv4Addr>().unwrap()), prefix)
    })
}

fn ipv4_in_prefix(value: u32, base: u32, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    value & mask == base & mask
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4() {
        return is_public_ipv4(mapped);
    }
    let value = u128::from(address);
    // Fail closed to global unicast 2000::/3 and remove special-purpose ranges.
    // Ingest has no reason to depend on protocol transition/anycast endpoints.
    ipv6_in_prefix(value, u128::from("2000::".parse::<Ipv6Addr>().unwrap()), 3)
        && [
            ("2001::", 23),     // IETF protocol assignments
            ("2001:db8::", 32), // documentation
            ("2002::", 16),     // 6to4 can encode private IPv4
            ("3ffe::", 16),     // retired 6bone space
        ]
        .into_iter()
        .all(|(base, prefix)| {
            !ipv6_in_prefix(value, u128::from(base.parse::<Ipv6Addr>().unwrap()), prefix)
        })
}

fn ipv6_in_prefix(value: u128, base: u128, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    value & mask == base & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_special_and_encoded_ipv4_hosts() {
        for raw in [
            "http://127.0.0.1/x",
            "http://2130706433/x",
            "http://0x7f000001/x",
            "http://0177.0.0.1/x",
            "http://10.0.0.1/x",
            "http://169.254.169.254/latest/meta-data",
            "http://192.168.1.2/x",
            "http://[::1]/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://[fc00::1]/x",
            "http://[fe80::1]/x",
        ] {
            let parsed = parse_public_http_url(raw).expect("URL syntax should parse");
            let host = parsed.host().expect("host");
            let address = match host {
                Host::Ipv4(value) => IpAddr::V4(value),
                Host::Ipv6(value) => IpAddr::V6(value),
                Host::Domain(value) => panic!("encoded IP remained a domain: {value}"),
            };
            assert!(
                !is_public_ip(address),
                "must reject {raw} parsed as {address}"
            );
        }
    }

    #[test]
    fn accepts_public_unicast_addresses() {
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(
                is_public_ip(address.parse().unwrap()),
                "must allow {address}"
            );
        }
    }

    #[test]
    fn rejects_non_http_credentials_and_zero_port() {
        for raw in [
            "file:///etc/passwd",
            "ftp://example.com/a",
            "http://user:secret@example.com/a",
            "http://example.com:0/a",
            "http://localhost/a",
            "http://service.localhost/a",
        ] {
            assert!(parse_public_http_url(raw).is_err(), "must reject {raw}");
        }
    }

    #[test]
    fn normalizes_domain_before_dns_pinning() {
        let parsed = parse_public_http_url("HTTPS://ExAmPlE.CoM./feed#fragment").unwrap();
        assert_eq!(parsed.as_str(), "https://example.com/feed");
    }

    #[test]
    fn redirect_resolution_rechecks_url_shape() {
        let base = parse_public_http_url("https://example.com/feed").unwrap();
        assert_eq!(
            resolve_redirect_url(&base, "/next").unwrap().as_str(),
            "https://example.com/next"
        );
        assert!(resolve_redirect_url(&base, "file:///etc/passwd").is_err());
        assert!(resolve_redirect_url(&base, "http://u:p@example.com/").is_err());
    }

    #[test]
    fn follows_only_standard_location_redirects() {
        for status in [301, 302, 303, 307, 308] {
            assert!(is_follow_redirect(StatusCode::from_u16(status).unwrap()));
        }
        for status in [300, 304, 305, 306] {
            assert!(!is_follow_redirect(StatusCode::from_u16(status).unwrap()));
        }
    }

    #[test]
    fn content_types_are_kind_specific() {
        assert!(content_type_allowed(
            "rss",
            Some("application/rss+xml; charset=utf-8")
        ));
        assert!(content_type_allowed("rss", Some("text/xml")));
        assert!(!content_type_allowed("rss", Some("text/html")));
        assert!(content_type_allowed(
            "html",
            Some("text/html; charset=utf-8")
        ));
        assert!(!content_type_allowed("html", Some("application/json")));
        assert!(!content_type_allowed("html", None));
    }

    #[test]
    fn body_limit_is_enforced_incrementally() {
        let mut body = vec![1, 2];
        append_limited(&mut body, &[3, 4], 4).unwrap();
        assert_eq!(body, vec![1, 2, 3, 4]);
        assert!(matches!(
            append_limited(&mut body, &[5], 4),
            Err(OutboundFetchError::ResponseTooLarge)
        ));
    }
}

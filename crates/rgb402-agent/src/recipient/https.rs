use super::*;

pub(super) struct HttpsTransport;
#[async_trait]
impl Transport for HttpsTransport {
    async fn get(&self, url: Url) -> Result<Response, DiscoveryError> {
        exchange(url, None, "application/jrd+json").await
    }
}

// Caller supplies the single total deadline, including decoding when applicable.
pub(super) async fn exchange(
    url: Url,
    payload: Option<&Value>,
    media_type: &str,
) -> Result<Response, DiscoveryError> {
    let pinned = PinnedDestination::resolve(url, &SystemDns).await?;
    let http = pinned.client()?;
    let url = pinned.url;
    let request = match payload {
        Some(payload) => http.post(url).json(payload),
        None => http.get(url),
    };
    let mut response = request
        .header(reqwest::header::ACCEPT, media_type)
        .send()
        .await
        .map_err(http_error)?;
    let status = response.status().as_u16();
    validate_status(status)?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if !content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case(media_type)
    {
        return Err(DiscoveryError::UnsupportedMediaType);
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(DiscoveryError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(http_error)? {
        if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
            return Err(DiscoveryError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response {
        status,
        content_type,
        body,
    })
}

#[async_trait]
trait Dns: Send + Sync {
    async fn resolve(&self, domain: &str) -> Result<Vec<SocketAddr>, DiscoveryError>;
}
struct SystemDns;
#[async_trait]
impl Dns for SystemDns {
    async fn resolve(&self, domain: &str) -> Result<Vec<SocketAddr>, DiscoveryError> {
        tokio::net::lookup_host((domain, 443))
            .await
            .map(|answers| answers.collect())
            .map_err(|_| DiscoveryError::DnsUnavailable)
    }
}
struct PinnedDestination {
    url: Url,
    addresses: Vec<SocketAddr>,
}
impl PinnedDestination {
    async fn resolve(url: Url, dns: &dyn Dns) -> Result<Self, DiscoveryError> {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
            || url.fragment().is_some()
        {
            return Err(DiscoveryError::DisallowedOrigin);
        }
        let domain = url.host_str().ok_or(DiscoveryError::DisallowedOrigin)?;
        let addresses = dns.resolve(domain).await?;
        validate_addresses(&addresses)?;
        Ok(Self { url, addresses })
    }
    fn client(&self) -> Result<reqwest::Client, DiscoveryError> {
        // The validated set is the exact set supplied to reqwest's DNS override.
        reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(TIMEOUT)
            .resolve_to_addrs(self.url.host_str().unwrap(), &self.addresses)
            .build()
            .map_err(|_| DiscoveryError::HttpsFailure)
    }
}

fn http_error(error: reqwest::Error) -> DiscoveryError {
    if error.is_timeout() {
        DiscoveryError::Timeout
    } else {
        DiscoveryError::HttpsFailure
    }
}
pub(super) fn validate_addresses(addresses: &[SocketAddr]) -> Result<(), DiscoveryError> {
    if addresses.is_empty() {
        return Err(DiscoveryError::DnsUnavailable);
    }
    if addresses.iter().any(|addr| !public_ip(addr.ip())) {
        return Err(DiscoveryError::DisallowedOrigin);
    }
    Ok(())
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let n = u32::from(ip);
            // Conservative exclusions for private, special-use, documentation and multicast.
            ![
                (0x00000000, 8),
                (0x0a000000, 8),
                (0x64400000, 10),
                (0x7f000000, 8),
                (0xa9fe0000, 16),
                (0xac100000, 12),
                (0xc0000000, 24),
                (0xc0000200, 24),
                (0xc0586300, 24),
                (0xc0a80000, 16),
                (0xc6120000, 15),
                (0xc6336400, 24),
                (0xcb007100, 24),
                (0xe0000000, 3),
            ]
            .iter()
            .any(|(base, bits)| n >> (32 - bits) == base >> (32 - bits))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only native global unicast; exclude transition/special-use/documentation.
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct ChangingDns(Mutex<Vec<Vec<SocketAddr>>>);
    #[async_trait]
    impl Dns for ChangingDns {
        async fn resolve(&self, domain: &str) -> Result<Vec<SocketAddr>, DiscoveryError> {
            assert_eq!(domain, "example.com");
            Ok(self.0.lock().unwrap().remove(0))
        }
    }
    #[tokio::test]
    async fn acquisition_re_resolves_and_pins_only_validated_answers() {
        let public = vec!["8.8.8.8:443".parse().unwrap()];
        let private = vec!["127.0.0.1:443".parse().unwrap()];
        let dns = ChangingDns(Mutex::new(vec![public.clone(), private]));
        let url = Url::parse("https://example.com/invoice").unwrap();
        let first = PinnedDestination::resolve(url.clone(), &dns).await.unwrap();
        assert_eq!(first.addresses, public);
        first.client().unwrap(); // Builds with the checked DNS override; no second lookup.
        assert_eq!(dns.0.lock().unwrap().len(), 1);
        assert!(matches!(
            PinnedDestination::resolve(url, &dns).await,
            Err(DiscoveryError::DisallowedOrigin)
        ));
        assert_eq!(first.addresses, public); // Later DNS change cannot mutate the pinned set.
    }
    #[tokio::test]
    async fn mixed_dns_answers_and_non_https_fail_closed() {
        let dns = ChangingDns(Mutex::new(vec![vec![
            "8.8.8.8:443".parse().unwrap(),
            "10.0.0.1:443".parse().unwrap(),
        ]]));
        assert!(matches!(
            PinnedDestination::resolve(Url::parse("https://example.com/invoice").unwrap(), &dns)
                .await,
            Err(DiscoveryError::DisallowedOrigin)
        ));
        for url in [
            "http://example.com/invoice",
            "https://user:secret@example.com/invoice",
            "https://example.com:8080/invoice",
        ] {
            assert!(matches!(
                PinnedDestination::resolve(Url::parse(url).unwrap(), &dns).await,
                Err(DiscoveryError::DisallowedOrigin)
            ));
        }
    }
}

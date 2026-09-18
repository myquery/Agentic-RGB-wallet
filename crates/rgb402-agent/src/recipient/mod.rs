//! Non-economic WebFinger discovery. No wallet, node, journal or model dependencies.
pub mod acquisition;
pub mod bridge;
pub mod btc;
mod https;
use async_trait::async_trait;
#[cfg(test)]
use https::validate_addresses;
use https::HttpsTransport;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

/// Experimental application vocabulary; not a registered standard or fetched URL.
pub const BTC_INVOICE_REL: &str = "https://rgb402.example/relations/btc-invoice";
pub const RGB_INVOICE_REL: &str = "https://rgb402.example/relations/rgb-invoice";
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;
pub const MAX_OBSERVATION_BYTES: usize = 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryError {
    MalformedIdentifier,
    MalformedJrd,
    UnknownAccount,
    MissingRelation,
    AmbiguousRelation,
    SubjectMismatch,
    DisallowedOrigin,
    RedirectRejected,
    ResponseTooLarge,
    UnsupportedMediaType,
    Timeout,
    DnsUnavailable,
    HttpsFailure,
    ServiceUnavailable,
}
impl DiscoveryError {
    fn status(&self) -> &'static str {
        match self {
            Self::MalformedIdentifier | Self::MalformedJrd => "malformed",
            Self::UnknownAccount => "not_found",
            Self::MissingRelation | Self::UnsupportedMediaType => "unsupported",
            Self::Timeout | Self::DnsUnavailable | Self::ServiceUnavailable => "unavailable",
            _ => "security_rejected",
        }
    }
}

/// Produced only after validation; intentionally not deserializable as authority.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RecipientDescriptor {
    identifier: String,
    subject: String,
    authoritative_domain: String,
    service: InvoiceService,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InvoiceService {
    #[serde(rename = "type")]
    kind: &'static str,
    url: String,
}
impl RecipientDescriptor {
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    pub fn authoritative_domain(&self) -> &str {
        &self.authoritative_domain
    }
    pub fn service_url(&self) -> &str {
        &self.service.url
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DiscoveryResult {
    Resolved {
        recipient: RecipientDescriptor,
    },
    Failed {
        category: &'static str,
        code: DiscoveryError,
    },
}
impl DiscoveryResult {
    /// No raw JRD, endpoint path, response text, DNS data or authorization is exposed.
    pub fn model_observation(&self) -> Value {
        match self {
            Self::Resolved { recipient } => json!({
                "status":"resolved", "identifier":recipient.identifier,
                "service_kind":"rgb_invoice", "service_origin":recipient.authoritative_domain,
                "next_allowed_actions":[], "future_step":"request_rgb_invoice",
                "payment_authorized":false
            }),
            Self::Failed { category, code } => json!({
                "status":category, "code":code, "next_allowed_actions":[]
            }),
        }
    }
}

struct Account {
    identifier: String,
    domain: String,
}
impl Account {
    fn parse(input: &str) -> Result<Self, DiscoveryError> {
        let (name, domain) = input
            .split_once('@')
            .ok_or(DiscoveryError::MalformedIdentifier)?;
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
            || name.starts_with('.')
            || name.ends_with('.')
            || name.contains("..")
            || domain.len() > 253
            || !domain.contains('.')
            || domain.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err(DiscoveryError::MalformedIdentifier);
        }
        let domain = domain.to_ascii_lowercase();
        if domain.parse::<IpAddr>().is_ok()
            || domain
                .rsplit('.')
                .next()
                .unwrap()
                .bytes()
                .all(|c| c.is_ascii_digit())
            || ["localhost", "local", "internal", "home", "lan", "onion"]
                .iter()
                .any(|suffix| domain == *suffix || domain.ends_with(&format!(".{suffix}")))
        {
            return Err(DiscoveryError::DisallowedOrigin);
        }
        Ok(Self {
            identifier: format!("{name}@{domain}"),
            domain,
        })
    }
    fn subject(&self) -> String {
        format!("acct:{}", self.identifier)
    }
    fn url(&self) -> Result<Url, DiscoveryError> {
        let mut url = Url::parse(&format!("https://{}/.well-known/webfinger", self.domain))
            .map_err(|_| DiscoveryError::MalformedIdentifier)?;
        if url.host_str() != Some(self.domain.as_str()) {
            return Err(DiscoveryError::MalformedIdentifier);
        }
        url.query_pairs_mut()
            .append_pair("resource", &self.subject());
        Ok(url)
    }
}

/// Production entry point: fixed public-network policy, ten-second total deadline.
/// Discovery does not contact the advertised invoice endpoint.
pub async fn resolve_recipient(identifier: &str) -> DiscoveryResult {
    resolve_with(identifier, &HttpsTransport, TIMEOUT).await
}

async fn resolve_with(
    identifier: &str,
    transport: &dyn Transport,
    deadline: Duration,
) -> DiscoveryResult {
    resolve_relation_with(identifier, transport, deadline, RGB_INVOICE_REL).await
}
async fn resolve_relation_with(
    identifier: &str,
    transport: &dyn Transport,
    deadline: Duration,
    relation: &str,
) -> DiscoveryResult {
    let result = async {
        let account = Account::parse(identifier)?;
        let response = transport.get(account.url()?).await?;
        validate_relation_response(&account, response, relation)
    };
    match tokio::time::timeout(deadline, result).await {
        Ok(Ok(recipient)) => DiscoveryResult::Resolved { recipient },
        result => {
            let code = match result {
                Ok(Err(code)) => code,
                _ => DiscoveryError::Timeout,
            };
            DiscoveryResult::Failed {
                category: code.status(),
                code,
            }
        }
    }
}

struct Response {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}
#[async_trait]
trait Transport: Send + Sync {
    async fn get(&self, url: Url) -> Result<Response, DiscoveryError>;
}
fn validate_status(status: u16) -> Result<(), DiscoveryError> {
    match status {
        200 => Ok(()),
        404 => Err(DiscoveryError::UnknownAccount),
        300..=399 => Err(DiscoveryError::RedirectRejected),
        _ => Err(DiscoveryError::ServiceUnavailable),
    }
}
fn validate_media_type(value: &str) -> Result<(), DiscoveryError> {
    if value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/jrd+json")
    {
        Ok(())
    } else {
        Err(DiscoveryError::UnsupportedMediaType)
    }
}
#[derive(Deserialize)]
struct Jrd {
    subject: String,
    #[serde(default)]
    links: Vec<Link>,
}
#[derive(Deserialize)]
struct Link {
    rel: String,
    href: Option<String>,
}
#[cfg(test)]
fn validate_response(
    account: &Account,
    response: Response,
) -> Result<RecipientDescriptor, DiscoveryError> {
    validate_relation_response(account, response, RGB_INVOICE_REL)
}
fn validate_relation_response(
    account: &Account,
    response: Response,
    relation: &str,
) -> Result<RecipientDescriptor, DiscoveryError> {
    validate_status(response.status)?;
    validate_media_type(&response.content_type)?;
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(DiscoveryError::ResponseTooLarge);
    }
    let jrd: Jrd =
        serde_json::from_slice(&response.body).map_err(|_| DiscoveryError::MalformedJrd)?;
    if jrd.subject != account.subject() {
        return Err(DiscoveryError::SubjectMismatch);
    }
    let mut supported = jrd.links.iter().filter(|link| link.rel == relation);
    let link = supported.next().ok_or(DiscoveryError::MissingRelation)?;
    if supported.next().is_some() {
        return Err(DiscoveryError::AmbiguousRelation);
    }
    let href = link.href.as_deref().ok_or(DiscoveryError::MalformedJrd)?;
    // Reject URL parser repairs and credential-bearing query/fragment syntax.
    if href.len() > 2048
        || !href.is_ascii()
        || href
            .bytes()
            .any(|c| c.is_ascii_control() || c == b' ' || c == b'\\')
    {
        return Err(DiscoveryError::DisallowedOrigin);
    }
    let endpoint = Url::parse(href).map_err(|_| DiscoveryError::DisallowedOrigin)?;
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some(&account.domain)
        || endpoint.port_or_known_default() != Some(443)
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !href.starts_with("https://")
        || href
            .split('/')
            .nth(2)
            .is_some_and(|authority| authority.contains('@'))
    {
        return Err(DiscoveryError::DisallowedOrigin);
    }
    Ok(RecipientDescriptor {
        identifier: account.identifier.clone(),
        subject: jrd.subject,
        authoritative_domain: account.domain.clone(),
        service: InvoiceService {
            kind: if relation == BTC_INVOICE_REL {
                "btc_invoice"
            } else {
                "rgb_invoice"
            },
            url: endpoint.to_string(),
        },
    })
}

#[cfg(test)]
mod tests;

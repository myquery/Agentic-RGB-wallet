//! Experimental invoice acquisition; deliberately no wallet or node capability.
use super::{https, DiscoveryError, RecipientDescriptor, Response, MAX_RESPONSE_BYTES, TIMEOUT};
use async_trait::async_trait;
use lightning_invoice::{Bolt11Invoice, Currency, TaggedField};
use rgb402_core::{wallet::PaymentRequest, AssetId, PaymentId, UnixTimestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const MAX_OBSERVATION_BYTES: usize = 2048;
const MAX_INVOICE_BYTES: usize = 12 * 1024;

/// Immutable, application-created intent. No Deserialize implementation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RecipientInvoiceContract {
    recipient: RecipientDescriptor,
    requested_asset: AssetId,
    requested_amount: u64,
    max_carrier_msat: u64,
    network: &'static str,
    contract_digest: String,
}
impl RecipientInvoiceContract {
    pub fn new(
        recipient: RecipientDescriptor,
        requested_asset: AssetId,
        requested_amount: u64,
        max_carrier_msat: u64,
    ) -> Result<Self, AcquisitionError> {
        let asset = requested_asset.as_str();
        if requested_amount == 0
            || max_carrier_msat == 0
            || asset.len() > 256
            || !asset.starts_with("rgb:")
            || !asset
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b":-_~".contains(&c))
        {
            return Err(AcquisitionError::InvalidContract);
        }
        // Fixed-order, versioned JSON tuple, never concatenated ambiguous strings.
        let encoded = serde_json::to_vec(&(
            "rgb402-recipient-invoice-v1",
            &recipient,
            &requested_asset,
            requested_amount,
            max_carrier_msat,
            "Regtest",
        ))
        .map_err(|_| AcquisitionError::InvalidContract)?;
        Ok(Self {
            recipient,
            requested_asset,
            requested_amount,
            max_carrier_msat,
            network: "Regtest",
            contract_digest: digest(&encoded),
        })
    }
    pub fn digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn recipient(&self) -> &RecipientDescriptor {
        &self.recipient
    }
    pub fn requested_asset(&self) -> &AssetId {
        &self.requested_asset
    }
    pub fn requested_amount(&self) -> u64 {
        self.requested_amount
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionError {
    InvalidContract,
    Unavailable,
    SecurityRejected,
    MalformedResponse,
    MalformedInvoice,
    AssetMismatch,
    AmountMismatch,
    UnsupportedInvoice,
    ExpiredInvoice,
    FutureTimestamp,
    NetworkMismatch,
    CarrierLimit,
    HttpsFailure,
    ResponseTooLarge,
    Timeout,
    WrongMediaType,
}
impl AcquisitionError {
    pub fn model_observation(self) -> Value {
        let category = match self {
            Self::Unavailable | Self::Timeout => "unavailable",
            Self::SecurityRejected | Self::HttpsFailure | Self::ResponseTooLarge => {
                "security_rejected"
            }
            Self::MalformedResponse | Self::MalformedInvoice | Self::InvalidContract => "malformed",
            Self::UnsupportedInvoice | Self::WrongMediaType => "unsupported",
            _ => "contract_rejected",
        };
        json!({"status":category,"code":self,"payment_authorized":false,"next_allowed_actions":[]})
    }
}
impl From<DiscoveryError> for AcquisitionError {
    fn from(error: DiscoveryError) -> Self {
        match error {
            DiscoveryError::Timeout => Self::Timeout,
            DiscoveryError::HttpsFailure => Self::HttpsFailure,
            DiscoveryError::ResponseTooLarge => Self::ResponseTooLarge,
            DiscoveryError::UnsupportedMediaType => Self::WrongMediaType,
            DiscoveryError::DnsUnavailable
            | DiscoveryError::ServiceUnavailable
            | DiscoveryError::UnknownAccount => Self::Unavailable,
            _ => Self::SecurityRejected,
        }
    }
}

/// Raw invoice occurs once, inside the decoded request. No Deserialize or Debug.
/// This is validation evidence, not an executable plan or approval token.
pub struct ValidatedRecipientInvoice {
    contract: RecipientInvoiceContract,
    request: PaymentRequest,
    invoice_id: String,
    issued_at: u64,
    validated_at: u64,
}
impl ValidatedRecipientInvoice {
    pub fn contract(&self) -> &RecipientInvoiceContract {
        &self.contract
    }
    pub fn request(&self) -> &PaymentRequest {
        &self.request
    }
    pub fn invoice_id(&self) -> &str {
        &self.invoice_id
    }
    pub fn issued_at(&self) -> u64 {
        self.issued_at
    }
    pub fn validated_at(&self) -> u64 {
        self.validated_at
    }
    pub fn matches_contract(&self, contract: &RecipientInvoiceContract) -> bool {
        self.contract == *contract
    }
    pub fn model_observation(&self) -> Value {
        json!({"status":"invoice_validated", "identifier":self.contract.recipient.identifier(),
            "service_origin":self.contract.recipient.authoritative_domain(),
            "asset":self.request.asset_id, "amount":self.request.amount.to_string(),
            "invoice_id":self.invoice_id, "contract_digest":self.contract.digest(),
            "contract_match":true, "payment_authorized":false, "next_allowed_actions":[],
            "freshness":"unexpired_at_validation_only", "future_step":"submit_to_payment_harness"})
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Performs one HTTPS acquisition followed by local RGB-aware BOLT11 validation.
pub async fn acquire_recipient_invoice(
    contract: &RecipientInvoiceContract,
) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
    acquire_with(
        contract,
        &PublicService,
        || UnixTimestamp::now().seconds(),
        TIMEOUT,
    )
    .await
}
#[async_trait]
trait InvoiceService: Send + Sync {
    async fn request(
        &self,
        contract: &RecipientInvoiceContract,
        payload: &Value,
    ) -> Result<Response, DiscoveryError>;
}
struct PublicService;
#[async_trait]
impl InvoiceService for PublicService {
    async fn request(
        &self,
        contract: &RecipientInvoiceContract,
        payload: &Value,
    ) -> Result<Response, DiscoveryError> {
        let url = reqwest::Url::parse(contract.recipient.service_url())
            .map_err(|_| DiscoveryError::DisallowedOrigin)?;
        https::exchange(url, Some(payload), "application/json").await
    }
}
async fn acquire_with(
    contract: &RecipientInvoiceContract,
    service: &dyn InvoiceService,
    now: impl Fn() -> u64,
    deadline: Duration,
) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
    let started = tokio::time::Instant::now();
    tokio::time::timeout(deadline, async {
        let payload = json!({"subject":contract.recipient.subject(), "asset_id":contract.requested_asset, "asset_amount":contract.requested_amount});
        let response = service.request(contract, &payload).await?;
        super::validate_status(response.status)?;
        if !response.content_type.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json") { return Err(AcquisitionError::WrongMediaType); }
        if response.body.len() > MAX_RESPONSE_BYTES { return Err(AcquisitionError::ResponseTooLarge); }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Candidate { invoice: String }
        let candidate: Candidate = serde_json::from_slice(&response.body).map_err(|_| AcquisitionError::MalformedResponse)?;
        let result = validate_candidate(contract, candidate.invoice, now())?;
        // Synchronous cryptographic decoding cannot yield to Tokio's timeout.
        if started.elapsed() >= deadline { return Err(AcquisitionError::Timeout); }
        Ok(result)
    }).await.map_err(|_| AcquisitionError::Timeout)?
}
fn validate_candidate(
    contract: &RecipientInvoiceContract,
    invoice: String,
    now: u64,
) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
    if invoice.len() > MAX_INVOICE_BYTES {
        return Err(AcquisitionError::ResponseTooLarge);
    }
    let decoded: Bolt11Invoice = invoice
        .parse()
        .map_err(|_| AcquisitionError::MalformedInvoice)?;
    // The upstream parser selects the first RGB field; explicitly reject ambiguity.
    if decoded
        .tagged_fields()
        .filter(|f| matches!(f, TaggedField::RgbContractId(_)))
        .count()
        != 1
        || decoded
            .tagged_fields()
            .filter(|f| matches!(f, TaggedField::RgbAmount(_)))
            .count()
            != 1
        || decoded
            .tagged_fields()
            .filter(|f| matches!(f, TaggedField::ExpiryTime(_)))
            .count()
            > 1
    {
        return Err(AcquisitionError::UnsupportedInvoice);
    }
    if decoded.currency() != Currency::Regtest {
        return Err(AcquisitionError::NetworkMismatch);
    }
    let asset_id = AssetId::new(
        decoded
            .rgb_contract_id()
            .ok_or(AcquisitionError::UnsupportedInvoice)?
            .to_string(),
    )
    .map_err(|_| AcquisitionError::MalformedInvoice)?;
    let amount = decoded
        .rgb_amount()
        .filter(|amount| *amount > 0)
        .ok_or(AcquisitionError::UnsupportedInvoice)?;
    let carrier_msat = decoded
        .amount_milli_satoshis()
        .filter(|amount| *amount > 0)
        .ok_or(AcquisitionError::UnsupportedInvoice)?;
    if asset_id != contract.requested_asset {
        return Err(AcquisitionError::AssetMismatch);
    }
    if amount != contract.requested_amount {
        return Err(AcquisitionError::AmountMismatch);
    }
    if carrier_msat > contract.max_carrier_msat {
        return Err(AcquisitionError::CarrierLimit);
    }
    let issued_at = decoded.duration_since_epoch().as_secs();
    let expires_at = issued_at
        .checked_add(decoded.expiry_time().as_secs())
        .ok_or(AcquisitionError::MalformedInvoice)?;
    if expires_at <= now {
        return Err(AcquisitionError::ExpiredInvoice);
    }
    if issued_at > now {
        return Err(AcquisitionError::FutureTimestamp);
    }
    let payment_hash = PaymentId::new(decoded.payment_hash().to_string())
        .map_err(|_| AcquisitionError::MalformedInvoice)?;
    let invoice_id = digest(invoice.as_bytes());
    Ok(ValidatedRecipientInvoice {
        contract: contract.clone(),
        request: PaymentRequest {
            asset_id,
            amount,
            invoice,
            payment_hash,
            expires_at,
            network: "Regtest".into(),
            carrier_msat,
        },
        invoice_id,
        issued_at,
        validated_at: now,
    })
}

#[cfg(test)]
mod tests;

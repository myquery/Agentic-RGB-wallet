use serde::{Deserialize, Serialize};
pub mod merchant;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const RGB402_SCHEME: &str = "RGB402";
pub const RGB402_PAYMENT_ID_HEADER: &str = "x-rgb402-payment-id";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("identifier cannot be empty")]
    EmptyIdentifier,
    #[error("amount precision {0} is too large")]
    UnsupportedPrecision(u8),
    #[error("minor units {minor_units} do not fit precision {precision}")]
    MinorUnitsOutOfRange { minor_units: u64, precision: u8 },
    #[error("amount precision mismatch: {left} vs {right}")]
    AmountPrecisionMismatch { left: u8, right: u8 },
    #[error("amount overflow")]
    AmountOverflow,
    #[error("unsupported payment scheme: {0}")]
    UnsupportedScheme(String),
}

fn validate_id(value: impl Into<String>) -> Result<String, DomainError> {
    let value = value.into();
    if value.trim().is_empty() {
        return Err(DomainError::EmptyIdentifier);
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(String);

impl AssetId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        Ok(Self(validate_id(value)?))
    }

    pub fn boss() -> Self {
        Self("rgb:boss".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentRequestId(String);

impl PaymentRequestId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        Ok(Self(validate_id(value)?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PaymentRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentId(String);

impl PaymentId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        Ok(Self(validate_id(value)?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnixTimestamp {
    seconds: u64,
}

impl UnixTimestamp {
    pub const fn new(seconds: u64) -> Self {
        Self { seconds }
    }

    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        Self { seconds }
    }

    pub fn seconds(self) -> u64 {
        self.seconds
    }

    pub fn saturating_add_seconds(self, seconds: u64) -> Self {
        Self {
            seconds: self.seconds.saturating_add(seconds),
        }
    }

    pub fn has_expired(self, now: Self) -> bool {
        now.seconds >= self.seconds
    }
}

impl fmt::Display for UnixTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.seconds)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentAmount {
    minor_units: u64,
    precision: u8,
}

impl PaymentAmount {
    pub fn new(minor_units: u64, precision: u8) -> Result<Self, DomainError> {
        validate_precision(precision)?;
        Ok(Self {
            minor_units,
            precision,
        })
    }

    pub fn from_major_minor(
        major_units: u64,
        minor_units: u64,
        precision: u8,
    ) -> Result<Self, DomainError> {
        validate_precision(precision)?;
        let scale = scale_for_precision(precision)?;
        if minor_units >= scale {
            return Err(DomainError::MinorUnitsOutOfRange {
                minor_units,
                precision,
            });
        }
        let major = major_units
            .checked_mul(scale)
            .ok_or(DomainError::AmountOverflow)?;
        let total = major
            .checked_add(minor_units)
            .ok_or(DomainError::AmountOverflow)?;
        Self::new(total, precision)
    }

    pub fn zero(precision: u8) -> Result<Self, DomainError> {
        Self::new(0, precision)
    }

    pub fn minor_units(self) -> u64 {
        self.minor_units
    }

    pub fn precision(self) -> u8 {
        self.precision
    }

    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        self.ensure_same_precision(other)?;
        let minor_units = self
            .minor_units
            .checked_add(other.minor_units)
            .ok_or(DomainError::AmountOverflow)?;
        Self::new(minor_units, self.precision)
    }

    pub fn exceeds(self, other: Self) -> Result<bool, DomainError> {
        self.ensure_same_precision(other)?;
        Ok(self.minor_units > other.minor_units)
    }

    pub fn ensure_same_precision(self, other: Self) -> Result<(), DomainError> {
        if self.precision != other.precision {
            return Err(DomainError::AmountPrecisionMismatch {
                left: self.precision,
                right: other.precision,
            });
        }
        Ok(())
    }
}

impl fmt::Display for PaymentAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = 10_u64.pow(self.precision as u32);
        if self.precision == 0 {
            return write!(f, "{}", self.minor_units);
        }
        let major = self.minor_units / scale;
        let minor = self.minor_units % scale;
        write!(
            f,
            "{major}.{minor:0width$}",
            width = self.precision as usize
        )
    }
}

fn validate_precision(precision: u8) -> Result<(), DomainError> {
    if precision > 18 {
        return Err(DomainError::UnsupportedPrecision(precision));
    }
    Ok(())
}

fn scale_for_precision(precision: u8) -> Result<u64, DomainError> {
    validate_precision(precision)?;
    Ok(10_u64.pow(precision as u32))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequest {
    pub payment_request_id: PaymentRequestId,
    pub asset_id: AssetId,
    pub amount: PaymentAmount,
    pub invoice: String,
    pub expires_at: UnixTimestamp,
    pub resource: String,
}

impl PaymentRequest {
    pub fn challenge(&self) -> PaymentChallenge {
        PaymentChallenge {
            scheme: RGB402_SCHEME.to_owned(),
            payment_request_id: self.payment_request_id.clone(),
            asset_id: self.asset_id.clone(),
            amount: self.amount,
            invoice: self.invoice.clone(),
            expires_at: self.expires_at,
            resource: self.resource.clone(),
        }
    }
}

impl TryFrom<PaymentChallenge> for PaymentRequest {
    type Error = DomainError;

    fn try_from(value: PaymentChallenge) -> Result<Self, Self::Error> {
        if value.scheme != RGB402_SCHEME {
            return Err(DomainError::UnsupportedScheme(value.scheme));
        }
        Ok(Self {
            payment_request_id: value.payment_request_id,
            asset_id: value.asset_id,
            amount: value.amount,
            invoice: value.invoice,
            expires_at: value.expires_at,
            resource: value.resource,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    Pending,
    Settled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentReceipt {
    pub payment_id: PaymentId,
    pub payment_request_id: PaymentRequestId,
    pub asset_id: AssetId,
    pub amount: PaymentAmount,
    pub settled_at: UnixTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentChallenge {
    pub scheme: String,
    pub payment_request_id: PaymentRequestId,
    pub asset_id: AssetId,
    pub amount: PaymentAmount,
    pub invoice: String,
    pub expires_at: UnixTimestamp,
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingPolicy {
    pub allowed_asset_ids: Vec<AssetId>,
    pub max_payment: PaymentAmount,
    pub session_budget: PaymentAmount,
}

impl SpendingPolicy {
    pub fn new(
        allowed_asset_ids: Vec<AssetId>,
        max_payment: PaymentAmount,
        session_budget: PaymentAmount,
    ) -> Self {
        Self {
            allowed_asset_ids,
            max_payment,
            session_budget,
        }
    }

    pub fn evaluate(
        &self,
        challenge: &PaymentChallenge,
        current_spend: PaymentAmount,
        now: UnixTimestamp,
    ) -> PolicyDecision {
        if challenge.scheme != RGB402_SCHEME {
            return PolicyDecision::rejected(PolicyRejectionReason::UnsupportedScheme);
        }

        if challenge.expires_at.has_expired(now) {
            return PolicyDecision::rejected(PolicyRejectionReason::ChallengeExpired);
        }

        if !self
            .allowed_asset_ids
            .iter()
            .any(|allowed| allowed == &challenge.asset_id)
        {
            return PolicyDecision::rejected(PolicyRejectionReason::AssetNotAllowed);
        }

        if let Err(error) = challenge.amount.ensure_same_precision(self.max_payment) {
            return PolicyDecision::rejected(PolicyRejectionReason::AmountPrecisionMismatch {
                detail: error.to_string(),
            });
        }

        if let Ok(true) = challenge.amount.exceeds(self.max_payment) {
            return PolicyDecision::rejected(PolicyRejectionReason::ExceedsMaxPayment);
        }

        if let Err(error) = current_spend.ensure_same_precision(self.session_budget) {
            return PolicyDecision::rejected(PolicyRejectionReason::AmountPrecisionMismatch {
                detail: error.to_string(),
            });
        }

        let projected_spend = match current_spend.checked_add(challenge.amount) {
            Ok(amount) => amount,
            Err(error) => {
                return PolicyDecision::rejected(PolicyRejectionReason::AmountPrecisionMismatch {
                    detail: error.to_string(),
                })
            }
        };

        if let Ok(true) = projected_spend.exceeds(self.session_budget) {
            return PolicyDecision::rejected(PolicyRejectionReason::ExceedsSessionBudget);
        }

        PolicyDecision::Approved { projected_spend }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Approved { projected_spend: PaymentAmount },
    Rejected { reason: PolicyRejectionReason },
}

impl PolicyDecision {
    pub fn rejected(reason: PolicyRejectionReason) -> Self {
        Self::Rejected { reason }
    }

    pub fn is_approved(&self) -> bool {
        matches!(self, Self::Approved { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyRejectionReason {
    UnsupportedScheme,
    AssetNotAllowed,
    ExceedsMaxPayment,
    ExceedsSessionBudget,
    ChallengeExpired,
    AmountPrecisionMismatch { detail: String },
}

#[derive(Clone, Debug)]
pub struct SpendingSession {
    policy: SpendingPolicy,
    spent: PaymentAmount,
}

impl SpendingSession {
    pub fn new(policy: SpendingPolicy) -> Result<Self, DomainError> {
        let spent = PaymentAmount::zero(policy.session_budget.precision())?;
        Ok(Self { policy, spent })
    }

    pub fn evaluate_challenge(
        &self,
        challenge: &PaymentChallenge,
        now: UnixTimestamp,
    ) -> PolicyDecision {
        self.policy.evaluate(challenge, self.spent, now)
    }

    pub fn record_receipt(&mut self, receipt: &PaymentReceipt) -> Result<(), DomainError> {
        self.spent = self.spent.checked_add(receipt.amount)?;
        Ok(())
    }

    pub fn spent(&self) -> PaymentAmount {
        self.spent
    }

    pub fn policy(&self) -> &SpendingPolicy {
        &self.policy
    }
}

pub mod wallet;

pub mod machine;

pub mod btc;
